use arc_swap::access::Access;
use rustls_pemfile::{certs, pkcs8_private_keys, rsa_private_keys};
use std::{
    collections::HashMap,
    fs::File,
    io::{self, BufReader, ErrorKind::InvalidData, Seek},
    path::Path,
    sync::Arc,
};
use tokio_rustls::rustls::{
    server::{ClientHello, ResolvesServerCert},
    sign::{any_supported_type, CertifiedKey},
    Certificate, PrivateKey, ServerName,
};

pub fn certified_key_from_acme_certificate(
    certificate: acme_lib::Certificate,
) -> Result<CertifiedKey, io::Error> {
    let cert_der_vec = certs(&mut certificate.certificate().as_bytes())
        .map_err(|_| io::Error::new(InvalidData, "Invalid certificate"))?;
    let cert_chain: Vec<Certificate> = cert_der_vec.into_iter().map(Certificate).collect();
    let private_key = PrivateKey(certificate.private_key_der());
    let signing_key = any_supported_type(&private_key)
        .map_err(|_| io::Error::new(InvalidData, "Invalid private key"))?;
    Ok(CertifiedKey::new(cert_chain, signing_key))
}

pub fn load_certified_key<P1, P2>(
    certificate_path: P1,
    private_key_path: P2,
) -> Result<CertifiedKey, io::Error>
where
    P1: AsRef<Path>,
    P2: AsRef<Path>,
{
    let certificates = load_certs(certificate_path)?;
    let private_key = load_key(&private_key_path)?;
    let signing_key = any_supported_type(&private_key).map_err(|_| {
        io::Error::new(
            InvalidData,
            format!(
                "Invalid private key in '{}'",
                private_key_path.as_ref().display()
            ),
        )
    })?;
    Ok(CertifiedKey::new(certificates, signing_key))
}

fn load_certs<P>(path: P) -> io::Result<Vec<Certificate>>
where
    P: AsRef<Path>,
{
    let file = File::open(&path).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("Could not open '{}' due to: {}", path.as_ref().display(), e),
        )
    })?;
    let mut reader = BufReader::new(file);
    let cert_der_vec = certs(&mut reader).map_err(|_| {
        io::Error::new(
            InvalidData,
            format!("Invalid certificate in '{}'", path.as_ref().display()),
        )
    })?;
    Ok(cert_der_vec.into_iter().map(Certificate).collect())
}

fn load_key<P>(path: P) -> io::Result<PrivateKey>
where
    P: AsRef<Path>,
{
    let mut keys = load_keys(&path)?;
    if keys.is_empty() {
        return Err(io::Error::new(
            InvalidData,
            format!("No private keys found in '{}'", path.as_ref().display()),
        ));
    }
    Ok(keys.remove(0))
}

fn load_keys<P>(path: P) -> io::Result<Vec<PrivateKey>>
where
    P: AsRef<Path>,
{
    let file = File::open(&path)?;
    let mut reader = BufReader::new(file);

    // Try RSA keys first
    let rsa_keys = rsa_private_keys(&mut reader)
        .map(|keys| keys.into_iter().map(PrivateKey).collect::<Vec<_>>())
        .unwrap_or_default();

    if !rsa_keys.is_empty() {
        return Ok(rsa_keys);
    }

    // If no RSA keys found, try PKCS8 format (for ECDSA, Ed25519, etc.)
    reader.rewind()?;
    let pkcs8_keys = pkcs8_private_keys(&mut reader)
        .map(|keys| keys.into_iter().map(PrivateKey).collect::<Vec<_>>())
        .unwrap_or_default();

    if !pkcs8_keys.is_empty() {
        return Ok(pkcs8_keys);
    }

    Err(io::Error::new(
        InvalidData,
        format!("No valid private keys (RSA or PKCS8) found in '{}'", path.as_ref().display()),
    ))
}

pub struct ReconfigurableCertificateResolver<A>
where
    A: Access<HashMap<ServerName, CertifiedKey>>,
{
    certificates: A,
}

impl<A> ReconfigurableCertificateResolver<A>
where
    A: Access<HashMap<ServerName, CertifiedKey>>,
{
    pub fn new(certificates: A) -> ReconfigurableCertificateResolver<A> {
        ReconfigurableCertificateResolver { certificates }
    }
}

impl<A> ResolvesServerCert for ReconfigurableCertificateResolver<A>
where
    A: Access<HashMap<ServerName, CertifiedKey>> + Send + Sync,
{
    fn resolve(&self, client_hello: ClientHello) -> Option<Arc<CertifiedKey>> {
        if let Some(name) = client_hello.server_name() {
            let certificates = self.certificates.load();
            // Convert &str to ServerName to look up in our HashMap
            if let Ok(server_name) = name.try_into() {
                certificates
                    .get(&server_name)
                    .map(|ck| Arc::new(ck.clone()))
            } else {
                None
            }
        } else {
            // This kind of resolver requires SNI
            None
        }
    }
}
