//! Downstream TLS: certificates per SNI name, chosen during the handshake.

use crate::state::State;
use async_trait::async_trait;
use log::{debug, warn};
use pingora_core::{
    listeners::TlsAccept,
    protocols::tls::TlsRef,
    tls::{
        ext,
        pkey::{PKey, Private},
        ssl::NameType,
        x509::X509,
    },
};
use std::{path::Path, sync::Arc};

/// A certificate chain (leaf first) with its private key.
pub struct CertKey {
    pub leaf: X509,
    pub chain: Vec<X509>,
    pub key: PKey<Private>,
}

impl CertKey {
    /// Loads PEM files. RSA (PKCS#1), PKCS#8 and EC keys are accepted.
    pub fn load(certificate_path: &Path, private_key_path: &Path) -> Result<CertKey, String> {
        let certificate = std::fs::read(certificate_path).map_err(|e| {
            format!(
                "Could not open '{}' due to: {}",
                certificate_path.display(),
                e
            )
        })?;
        let key = std::fs::read(private_key_path).map_err(|e| {
            format!(
                "Could not open '{}' due to: {}",
                private_key_path.display(),
                e
            )
        })?;
        CertKey::from_pem(&certificate, &key).map_err(|e| {
            format!(
                "Invalid certificate '{}' or key '{}': {}",
                certificate_path.display(),
                private_key_path.display(),
                e
            )
        })
    }

    pub fn from_pem(certificate_pem: &[u8], key_pem: &[u8]) -> Result<CertKey, String> {
        let mut certificates = X509::stack_from_pem(certificate_pem)
            .map_err(|e| format!("invalid certificate PEM: {}", e))?
            .into_iter();
        let leaf = certificates
            .next()
            .ok_or_else(|| "no certificate found".to_string())?;
        let chain = certificates.collect();
        let key = PKey::private_key_from_pem(key_pem)
            .map_err(|e| format!("invalid private key PEM: {}", e))?;
        let public = leaf
            .public_key()
            .map_err(|e| format!("invalid certificate key: {}", e))?;
        if !public.public_eq(&key) {
            return Err("the private key does not belong to the certificate".into());
        }
        Ok(CertKey { leaf, chain, key })
    }
}

/// Picks the certificate for the SNI name of a TLS handshake: exact name first, then the
/// wildcard of the parent domain. Clients without SNI get no certificate (like v1.1).
pub struct SniResolver {
    state: Arc<State>,
}

impl SniResolver {
    pub fn new(state: Arc<State>) -> SniResolver {
        SniResolver { state }
    }

    pub fn find(&self, server_name: &str) -> Option<Arc<CertKey>> {
        let name = server_name.trim_end_matches('.').to_ascii_lowercase();
        let config = self.state.config.load();
        let dynamic = self.state.dynamic_certificates.load();
        let exact = config
            .certificates
            .get(&name)
            .or_else(|| dynamic.get(&name))
            .cloned();
        exact.or_else(|| {
            let (_, parent) = name.split_once('.')?;
            let wildcard = format!("*.{}", parent);
            config
                .certificates
                .get(&wildcard)
                .or_else(|| dynamic.get(&wildcard))
                .cloned()
        })
    }
}

#[async_trait]
impl TlsAccept for SniResolver {
    async fn certificate_callback(&self, ssl: &mut TlsRef) -> () {
        let Some(server_name) = ssl.servername(NameType::HOST_NAME).map(str::to_string) else {
            debug!("TLS client without SNI, no certificate");
            return;
        };
        let Some(cert) = self.find(&server_name) else {
            debug!("No certificate for SNI '{}'", server_name);
            return;
        };
        if let Err(e) = ext::ssl_use_certificate(ssl, &cert.leaf) {
            warn!("Could not use certificate for '{}': {}", server_name, e);
            return;
        }
        if let Err(e) = ext::ssl_use_private_key(ssl, &cert.key) {
            warn!("Could not use private key for '{}': {}", server_name, e);
            return;
        }
        for intermediate in &cert.chain {
            if let Err(e) = ext::ssl_add_chain_cert(ssl, intermediate) {
                warn!(
                    "Could not add chain certificate for '{}': {}",
                    server_name, e
                );
            }
        }
    }
}
