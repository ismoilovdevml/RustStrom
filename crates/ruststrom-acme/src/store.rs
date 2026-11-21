//! On-disk layout and atomic persistence.
//!
//! ```text
//! <persist_dir>/<ca>/account.json      ACME account credentials (0600)
//! <persist_dir>/<ca>/certs/<domain>.pem private key + certificate chain (0600)
//! ```
//!
//! `<ca>` is derived from the directory URL so that staging and production accounts and
//! certificates never mix. The certificate file is a single PEM bundle (key first, then the
//! chain, the format HAProxy reads), written with one atomic rename so a crash can never leave
//! a key that does not match its certificate.

use std::{
    fs, io,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rcgen::{KeyPair, PublicKeyData};
use rustls_pki_types::{pem::PemObject, CertificateDer};
use x509_parser::extensions::GeneralName;

use crate::{AcmeError, CertPem};

/// Directory for one CA, e.g. `acme-v02.api.letsencrypt.org_directory`.
pub(crate) fn ca_dir_name(directory_url: &str) -> String {
    let without_scheme = directory_url
        .split_once("://")
        .map_or(directory_url, |(_, rest)| rest);
    let name: String = without_scheme
        .trim_end_matches('/')
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    name.trim_start_matches('.').to_owned()
}

/// Creates `dir` and its parents, private to the owner on Unix.
pub(crate) fn create_private_dir(dir: &Path) -> Result<(), AcmeError> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(dir)
        .map_err(|e| AcmeError::io("failed to create directory", dir, e))
}

/// Reads a file, mapping "not found" to `None`.
pub(crate) async fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, AcmeError> {
    match tokio::fs::read(path).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(AcmeError::io("failed to read", path, e)),
    }
}

/// Atomically replaces `path` with `contents`, readable by the owner only.
pub(crate) async fn write_private(path: PathBuf, contents: Vec<u8>) -> Result<(), AcmeError> {
    let target = path.clone();
    tokio::task::spawn_blocking(move || write_private_blocking(&path, &contents))
        .await
        .map_err(|e| AcmeError::io("failed to write", target, io::Error::other(e)))?
}

fn write_private_blocking(path: &Path, contents: &[u8]) -> Result<(), AcmeError> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().unwrap_or(Path::new("."));
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(
        ".{file_name}.tmp.{}.{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    let result = write_and_rename(&tmp, path, contents);
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result?;

    // Persist the rename itself. Not every filesystem supports fsync on a directory.
    #[cfg(unix)]
    if let Err(e) = fs::File::open(dir).and_then(|d| d.sync_all()) {
        log::debug!("fsync of {} failed: {e}", dir.display());
    }
    Ok(())
}

fn write_and_rename(tmp: &Path, path: &Path, contents: &[u8]) -> Result<(), AcmeError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(tmp)
        .map_err(|e| AcmeError::io("failed to create", tmp, e))?;
    file.write_all(contents)
        .and_then(|()| file.sync_all())
        .map_err(|e| AcmeError::io("failed to write", tmp, e))?;
    drop(file);
    fs::rename(tmp, path).map_err(|e| AcmeError::io("failed to rename into", path, e))
}

/// Serializes a certificate as the on-disk bundle.
pub(crate) fn bundle(cert: &CertPem) -> String {
    let mut out = String::with_capacity(cert.key_pem.len() + cert.chain_pem.len() + 1);
    out.push_str(cert.key_pem.trim_end());
    out.push('\n');
    out.push_str(&cert.chain_pem);
    out
}

/// Splits a PEM bundle into (key, chain) text.
pub(crate) fn split_bundle(pem: &str) -> Result<(String, String), String> {
    let mut key = String::new();
    let mut chain = String::new();
    let mut current: Option<(String, String)> = None;
    for line in pem.lines() {
        let line = line.trim_end();
        if let Some(label) = line
            .strip_prefix("-----BEGIN ")
            .and_then(|l| l.strip_suffix("-----"))
        {
            current = Some((label.to_owned(), format!("{line}\n")));
        } else if let Some(label) = line
            .strip_prefix("-----END ")
            .and_then(|l| l.strip_suffix("-----"))
        {
            let Some((begin_label, mut block)) = current.take() else {
                return Err("PEM END line without BEGIN".into());
            };
            if begin_label != label {
                return Err(format!("PEM block {begin_label} ends with {label}"));
            }
            block.push_str(line);
            block.push('\n');
            if label == "CERTIFICATE" {
                chain.push_str(&block);
            } else if label.ends_with("PRIVATE KEY") {
                if !key.is_empty() {
                    return Err("more than one private key".into());
                }
                key = block;
            }
        } else if let Some((_, block)) = current.as_mut() {
            block.push_str(line);
            block.push('\n');
        }
    }
    if current.is_some() {
        return Err("unterminated PEM block".into());
    }
    if key.is_empty() {
        return Err("no private key".into());
    }
    if chain.is_empty() {
        return Err("no certificate".into());
    }
    Ok((key, chain))
}

/// Checks that `chain_pem` is a certificate for `domain` whose public key belongs to
/// `key_pem`, and returns it with its expiry.
pub(crate) fn validate(domain: &str, chain_pem: &str, key_pem: &str) -> Result<CertPem, String> {
    let leaf = CertificateDer::pem_slice_iter(chain_pem.as_bytes())
        .next()
        .ok_or("no certificate in chain")?
        .map_err(|e| format!("invalid certificate PEM: {e}"))?;
    let (_, cert) = x509_parser::parse_x509_certificate(&leaf)
        .map_err(|e| format!("invalid leaf certificate: {e}"))?;

    let names = cert
        .subject_alternative_name()
        .map_err(|e| format!("invalid subjectAltName: {e}"))?
        .map(|san| {
            san.value
                .general_names
                .iter()
                .filter_map(|n| match n {
                    GeneralName::DNSName(name) => Some(name.to_ascii_lowercase()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !names.iter().any(|n| n == domain) {
        return Err(format!("certificate is for {names:?}, not {domain}"));
    }

    let key = KeyPair::from_pem(key_pem).map_err(|e| format!("invalid private key: {e}"))?;
    if key.subject_public_key_info() != cert.public_key().raw {
        return Err("private key does not match the certificate".into());
    }

    let not_after = u64::try_from(cert.validity().not_after.timestamp())
        .map(|secs| UNIX_EPOCH + Duration::from_secs(secs))
        .map_err(|_| "certificate expires before 1970".to_owned())?;

    Ok(CertPem {
        chain_pem: chain_pem.to_owned(),
        key_pem: key_pem.to_owned(),
        not_after,
    })
}

/// Loads and validates a persisted certificate. Missing files yield `Ok(None)`; unreadable or
/// invalid files yield an error describing why, so the caller can log and re-order.
pub(crate) async fn load_cert(path: &Path, domain: &str) -> Result<Option<CertPem>, AcmeError> {
    let Some(bytes) = read_optional(path).await? else {
        return Ok(None);
    };
    let invalid = |reason: String| {
        AcmeError::io(
            "invalid certificate file",
            path,
            io::Error::new(io::ErrorKind::InvalidData, reason),
        )
    };
    let text = String::from_utf8(bytes).map_err(|_| invalid("not UTF-8".into()))?;
    let (key, chain) = split_bundle(&text).map_err(invalid)?;
    validate(domain, &chain, &key).map(Some).map_err(invalid)
}

/// Time left until `not_after`, or zero if already expired.
pub(crate) fn remaining(not_after: SystemTime) -> Duration {
    not_after
        .duration_since(SystemTime::now())
        .unwrap_or(Duration::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{CertificateParams, PKCS_ECDSA_P256_SHA256};

    fn self_signed(domain: &str) -> (String, String) {
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
        let cert = CertificateParams::new(vec![domain.to_owned()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        (cert.pem(), key.serialize_pem())
    }

    #[test]
    fn ca_dir_names() {
        assert_eq!(
            ca_dir_name("https://acme-v02.api.letsencrypt.org/directory"),
            "acme-v02.api.letsencrypt.org_directory"
        );
        assert_eq!(
            ca_dir_name("https://localhost:23400/dir"),
            "localhost_23400_dir"
        );
        assert_eq!(ca_dir_name("https://../../etc"), "_.._etc");
    }

    #[test]
    fn bundle_roundtrip_and_validation() {
        let (chain, key) = self_signed("a.example.com");
        let cert = validate("a.example.com", &chain, &key).unwrap();
        let (k2, c2) = split_bundle(&bundle(&cert)).unwrap();
        assert_eq!(k2, key);
        assert_eq!(c2, chain);
        assert!(remaining(cert.not_after) > Duration::from_secs(3600));
    }

    #[test]
    fn rejects_wrong_domain_and_wrong_key() {
        let (chain, key) = self_signed("a.example.com");
        let (_, other_key) = self_signed("a.example.com");
        assert!(validate("b.example.com", &chain, &key)
            .unwrap_err()
            .contains("not b.example.com"));
        assert!(validate("a.example.com", &chain, &other_key)
            .unwrap_err()
            .contains("does not match"));
    }

    #[test]
    fn rejects_broken_bundles() {
        assert!(split_bundle("").is_err());
        assert!(split_bundle("-----BEGIN CERTIFICATE-----\nAAAA\n").is_err());
        let (chain, _) = self_signed("a.example.com");
        assert_eq!(split_bundle(&chain).unwrap_err(), "no private key");
    }

    #[tokio::test]
    async fn atomic_write_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.pem");
        write_private(path.clone(), b"one".to_vec()).await.unwrap();
        write_private(path.clone(), b"two".to_vec()).await.unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let leftovers: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(leftovers.len(), 1, "temporary files must not remain");
    }

    #[tokio::test]
    async fn load_cert_reports_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.example.com.pem");
        assert!(load_cert(&path, "a.example.com").await.unwrap().is_none());
        fs::write(&path, "garbage").unwrap();
        let err = load_cert(&path, "a.example.com").await.unwrap_err();
        assert!(
            err.to_string().contains("invalid certificate file"),
            "{err}"
        );
    }
}
