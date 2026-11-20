//! Integration point for ACME (HTTP-01).
//!
//! The ACME client itself lives in `crates/ruststrom-acme` (delivered separately). This module
//! owns what the data plane needs:
//!
//! * [`ChallengeStore`]: tokens answered by the proxy on
//!   `/.well-known/acme-challenge/<token>` (in `request_filter`, before routing);
//! * [`AcmeIntegration`]: the small trait the ACME manager implements. Issued certificates are
//!   handed to [`crate::state::State::install_certificate`], which the SNI resolver serves
//!   without a reload.

use crate::config::AcmeCertificate;
use std::{collections::HashMap, sync::RwLock};

pub const CHALLENGE_PREFIX: &str = "/.well-known/acme-challenge/";

/// HTTP-01 key authorizations by token.
#[derive(Debug, Default)]
pub struct ChallengeStore {
    tokens: RwLock<HashMap<String, String>>,
}

impl ChallengeStore {
    pub fn get(&self, token: &str) -> Option<String> {
        self.tokens
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(token)
            .cloned()
    }

    pub fn insert(&self, token: String, key_authorization: String) {
        self.tokens
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(token, key_authorization);
    }

    pub fn remove(&self, token: &str) {
        self.tokens
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(token);
    }

    pub fn is_empty(&self) -> bool {
        self.tokens
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }
}

/// Implemented by the ACME manager to follow configuration changes. Certificates it obtains are
/// installed with `State::install_certificate(domain, CertKey::from_pem(chain, key))`.
pub trait AcmeIntegration: Send + Sync {
    /// The set of `type = "ACME"` certificates after a (re)load.
    fn config_changed(&self, certificates: &[AcmeCertificate]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_store_roundtrip() {
        let store = ChallengeStore::default();
        assert!(store.is_empty());
        store.insert("t".into(), "t.key".into());
        assert_eq!(store.get("t").as_deref(), Some("t.key"));
        store.remove("t");
        assert_eq!(store.get("t"), None);
    }
}
