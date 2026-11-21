//! Async ACME (RFC 8555) certificate manager for RustStrom, using HTTP-01 validation.
//!
//! Replaces v1's blocking `acme-lib` integration. Built on [`instant_acme`] with a rustls/ring
//! HTTP client (no OpenSSL, static musl friendly), ECDSA P-256 certificate keys from `rcgen`,
//! atomic `0600` persistence and a renewal loop.
//!
//! # Integration
//!
//! ```no_run
//! use std::sync::Arc;
//! use ruststrom_acme::{AcmeConfig, AcmeManager, ChallengeStore, LETS_ENCRYPT_PRODUCTION};
//!
//! # async fn run() -> Result<(), ruststrom_acme::AcmeError> {
//! let challenges = ChallengeStore::default();
//! // The port 80 listener answers GET /.well-known/acme-challenge/<token> with
//! // `challenges.response_for_path(path)` (404 when `None`).
//! let cfg = AcmeConfig::new(LETS_ENCRYPT_PRODUCTION, "ops@example.com", "/var/lib/ruststrom/acme");
//! let manager = Arc::new(AcmeManager::new(cfg, challenges.clone())?);
//! let cert = manager.ensure_certificate("example.com").await?;
//! // install cert.chain_pem / cert.key_pem in the SNI store, then keep it fresh:
//! tokio::spawn(manager.run_renewal_loop(vec!["example.com".into()], |domain, cert| {
//!     // swap the certificate for `domain` in the SNI store
//!     let _ = (domain, cert);
//! }));
//! # Ok(())
//! # }
//! ```

mod challenge;
mod error;
mod http;
mod manager;
mod store;

use std::{fmt, path::PathBuf, time::Duration, time::SystemTime};

pub use challenge::{ChallengeStore, CHALLENGE_PATH_PREFIX};
pub use error::AcmeError;
pub use manager::AcmeManager;

/// Let's Encrypt production directory.
pub const LETS_ENCRYPT_PRODUCTION: &str = "https://acme-v02.api.letsencrypt.org/directory";
/// Let's Encrypt staging directory (untrusted certificates, generous rate limits).
pub const LETS_ENCRYPT_STAGING: &str = "https://acme-staging-v02.api.letsencrypt.org/directory";

/// Configuration of an [`AcmeManager`]. Build it with [`AcmeConfig::new`] and override fields
/// as needed.
#[derive(Clone, Debug)]
pub struct AcmeConfig {
    /// ACME directory URL: [`LETS_ENCRYPT_PRODUCTION`], [`LETS_ENCRYPT_STAGING`] or a custom
    /// CA. Must be `https://`.
    pub directory_url: String,
    /// Contact address registered with the account. May be empty (no contact).
    pub email: String,
    /// Directory for the account key and certificates. Created with mode `0700`.
    pub persist_dir: PathBuf,
    /// Renew when the certificate expires in less than this. Default: 30 days.
    pub renew_before: Duration,
    /// How often [`AcmeManager::run_renewal_loop`] checks each domain. Default: 12 hours.
    pub check_interval: Duration,
    /// Upper bound for one complete order (account, challenge, finalize, download).
    /// Default: 5 minutes.
    pub order_timeout: Duration,
    /// Trust only the CA certificate(s) in this PEM file when talking to the ACME server,
    /// instead of the bundled Mozilla roots. For private CAs and tests (Pebble). Default: `None`.
    pub ca_root_pem: Option<PathBuf>,
}

impl AcmeConfig {
    /// Config with default timings and the Mozilla root store.
    pub fn new(
        directory_url: impl Into<String>,
        email: impl Into<String>,
        persist_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            directory_url: directory_url.into(),
            email: email.into(),
            persist_dir: persist_dir.into(),
            renew_before: Duration::from_secs(30 * 24 * 3600),
            check_interval: Duration::from_secs(12 * 3600),
            order_timeout: Duration::from_secs(5 * 60),
            ca_root_pem: None,
        }
    }
}

/// A certificate with its private key, both PEM encoded.
#[derive(Clone, PartialEq, Eq)]
pub struct CertPem {
    /// Leaf certificate followed by the intermediates, as returned by the CA.
    pub chain_pem: String,
    /// PKCS#8 private key of the leaf certificate (ECDSA P-256 for issued certificates).
    pub key_pem: String,
    /// Expiry of the leaf certificate.
    pub not_after: SystemTime,
}

impl fmt::Debug for CertPem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CertPem")
            .field("chain_pem", &self.chain_pem)
            .field("key_pem", &"<redacted>")
            .field("not_after", &self.not_after)
            .finish()
    }
}
