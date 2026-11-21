use std::{io, path::PathBuf, time::Duration};

/// Everything that can go wrong while obtaining or loading a certificate.
#[derive(Debug, thiserror::Error)]
pub enum AcmeError {
    /// The [`AcmeConfig`](crate::AcmeConfig) is unusable (bad URL, unreadable CA root, ...).
    #[error("invalid ACME configuration: {0}")]
    Config(String),

    /// The domain cannot be validated with HTTP-01 (wildcard, IP address, bad characters).
    #[error("invalid domain {domain:?}: {reason}")]
    InvalidDomain {
        /// The rejected domain.
        domain: String,
        /// Why it was rejected.
        reason: &'static str,
    },

    /// Reading or writing a file in the persist directory failed.
    #[error("{action} {}: {source}", path.display())]
    Io {
        /// What was being done, e.g. "failed to write".
        action: &'static str,
        /// The file or directory involved.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: io::Error,
    },

    /// A request to the ACME CA failed (network, TLS, or a problem document from the CA).
    #[error("ACME request failed while {step}: {source}")]
    Protocol {
        /// The step of the ACME flow that failed, e.g. "creating the order".
        step: &'static str,
        /// The error reported by the ACME client.
        #[source]
        source: instant_acme::Error,
    },

    /// The CA did not offer an HTTP-01 challenge for the domain.
    #[error("the CA offered no http-01 challenge for {domain}")]
    NoHttp01Challenge {
        /// The domain being validated.
        domain: String,
    },

    /// The CA tried to validate the HTTP-01 challenge and failed, or the order became invalid.
    #[error("validation of {domain} failed: {detail}")]
    ValidationFailed {
        /// The domain being validated.
        domain: String,
        /// The CA's explanation, if it gave one.
        detail: String,
    },

    /// The whole order (account, challenge, finalize, download) took longer than
    /// [`AcmeConfig::order_timeout`](crate::AcmeConfig::order_timeout).
    #[error("ordering a certificate for {domain} timed out after {after:?}")]
    Timeout {
        /// The domain being ordered.
        domain: String,
        /// The configured order timeout.
        after: Duration,
    },

    /// Key or CSR generation failed.
    #[error("failed to generate key or CSR: {0}")]
    KeyGeneration(#[from] rcgen::Error),

    /// The CA returned a certificate chain that cannot be used.
    #[error("unusable certificate from the CA for {domain}: {reason}")]
    BadCertificate {
        /// The domain being ordered.
        domain: String,
        /// What is wrong with it.
        reason: String,
    },
}

impl AcmeError {
    pub(crate) fn io(action: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            action,
            path: path.into(),
            source,
        }
    }

    pub(crate) fn protocol(step: &'static str) -> impl FnOnce(instant_acme::Error) -> Self {
        move |source| Self::Protocol { step, source }
    }
}
