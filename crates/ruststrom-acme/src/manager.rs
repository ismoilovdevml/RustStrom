use std::{
    collections::HashMap,
    net::IpAddr,
    panic::{catch_unwind, AssertUnwindSafe},
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex, PoisonError},
    time::{Duration, SystemTime},
};

use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, ChallengeType, Identifier, NewAccount,
    NewOrder, Order, OrderStatus, RetryPolicy,
};
use log::{debug, error, info, warn};
use rcgen::{CertificateParams, DistinguishedName, KeyPair, PKCS_ECDSA_P256_SHA256};
use tokio::{sync::Mutex, time::Instant};

use crate::{
    challenge::ChallengeGuard, http::AcmeHttpClient, store, AcmeConfig, AcmeError, CertPem,
    ChallengeStore,
};

const ACCOUNT_DOES_NOT_EXIST: &str = "urn:ietf:params:acme:error:accountDoesNotExist";
/// First retry delay of the renewal loop after a failure (capped by `check_interval`).
const RETRY_BASE: Duration = Duration::from_secs(60);

/// Obtains, persists and renews certificates from an ACME CA using HTTP-01.
///
/// One manager serves any number of domains with a single account. Concurrent calls for the
/// same domain are serialized (the second caller gets the certificate the first one ordered);
/// calls for different domains run in parallel.
pub struct AcmeManager {
    cfg: AcmeConfig,
    challenges: ChallengeStore,
    http: AcmeHttpClient,
    ca_dir: PathBuf,
    account: Mutex<Option<Account>>,
    domain_locks: StdMutex<HashMap<String, Arc<Mutex<()>>>>,
}

/// Outcome of checking one domain.
enum Checked {
    /// A certificate that is not due for renewal (loaded from disk or just issued).
    Current(CertPem),
    /// Ordering failed. `fallback` is the unexpired certificate on disk, if there is one.
    Failed {
        error: AcmeError,
        fallback: Option<CertPem>,
    },
}

impl AcmeManager {
    /// Validates the config, creates the persist directory and prepares the HTTP client.
    /// Does not contact the CA; the account is loaded or registered on the first order.
    pub fn new(cfg: AcmeConfig, challenges: ChallengeStore) -> Result<Self, AcmeError> {
        let uri: http::Uri = cfg.directory_url.parse().map_err(|e| {
            AcmeError::Config(format!("directory_url {:?}: {e}", cfg.directory_url))
        })?;
        if uri.scheme_str() != Some("https") || uri.host().is_none() {
            return Err(AcmeError::Config(format!(
                "directory_url {:?} must be an https:// URL",
                cfg.directory_url
            )));
        }
        if cfg.check_interval.is_zero() || cfg.order_timeout.is_zero() {
            return Err(AcmeError::Config(
                "check_interval and order_timeout must be greater than zero".into(),
            ));
        }
        if cfg.email.contains(char::is_whitespace) || cfg.email.starts_with("mailto:") {
            return Err(AcmeError::Config(format!(
                "email {:?} must be a plain address without spaces or mailto:",
                cfg.email
            )));
        }

        let http = AcmeHttpClient::new(cfg.ca_root_pem.as_deref())?;
        let ca_dir = cfg.persist_dir.join(store::ca_dir_name(&cfg.directory_url));
        store::create_private_dir(&ca_dir.join("certs"))?;

        Ok(Self {
            cfg,
            challenges,
            http,
            ca_dir,
            account: Mutex::new(None),
            domain_locks: StdMutex::new(HashMap::new()),
        })
    }

    /// Returns a certificate for `domain`.
    ///
    /// Loads it from `persist_dir` if it is valid and not due for renewal. Otherwise orders a
    /// new one (HTTP-01 through the [`ChallengeStore`]), persists it atomically (`0600`) and
    /// returns it. If ordering fails but an unexpired certificate is on disk, that certificate
    /// is returned and the failure is logged, so a CA outage never takes a working site down.
    pub async fn ensure_certificate(&self, domain: &str) -> Result<CertPem, AcmeError> {
        match self.check(domain).await {
            Checked::Current(cert) => Ok(cert),
            Checked::Failed {
                error,
                fallback: Some(cert),
            } => {
                warn!(
                    "ACME renewal of {domain} failed, keeping the current certificate valid for \
                     {:?}: {error}",
                    store::remaining(cert.not_after)
                );
                Ok(cert)
            }
            Checked::Failed {
                error,
                fallback: None,
            } => Err(error),
        }
    }

    /// Keeps the certificates of `domains` fresh. Never returns; abort the task to stop it.
    ///
    /// Every `check_interval` (and exactly when a certificate enters its renewal window) each
    /// domain is checked; certificates due for renewal are re-ordered. Failures are retried
    /// with exponential backoff starting at one minute, capped at `check_interval`.
    ///
    /// `on_update(domain, cert)` is called once per domain when the loop first has a usable
    /// certificate for it, and again every time the certificate changes. Installing the same
    /// certificate twice must therefore be harmless. A panic in `on_update` is caught and
    /// logged. Invalid domains are logged and skipped.
    pub async fn run_renewal_loop(
        self: Arc<Self>,
        domains: Vec<String>,
        on_update: impl Fn(&str, CertPem) + Send + Sync + 'static,
    ) {
        struct DomainState {
            domain: String,
            next_check: Instant,
            failures: u32,
            delivered: Option<CertPem>,
        }

        let now = Instant::now();
        let mut states: Vec<DomainState> = domains
            .into_iter()
            .filter(|d| match normalize_domain(d) {
                Ok(_) => true,
                Err(e) => {
                    error!("ACME renewal loop skips {d:?}: {e}");
                    false
                }
            })
            .map(|domain| DomainState {
                domain,
                next_check: now,
                failures: 0,
                delivered: None,
            })
            .collect();
        if states.is_empty() {
            warn!("ACME renewal loop has no domains to manage");
        }

        loop {
            for state in states.iter_mut() {
                if state.next_check > Instant::now() {
                    continue;
                }
                let (cert, error) = match self.check(&state.domain).await {
                    Checked::Current(cert) => (Some(cert), None),
                    Checked::Failed { error, fallback } => (fallback, Some(error)),
                };

                if let Some(cert) = &cert {
                    if state.delivered.as_ref() != Some(cert) {
                        let domain = state.domain.as_str();
                        let delivered = catch_unwind(AssertUnwindSafe(|| {
                            on_update(domain, cert.clone());
                        }));
                        match delivered {
                            Ok(()) => state.delivered = Some(cert.clone()),
                            Err(_) => error!("ACME on_update callback panicked for {domain}"),
                        }
                    }
                }

                let delay = match error {
                    None => {
                        state.failures = 0;
                        let until_due = cert
                            .as_ref()
                            .map(|c| self.until_due(c))
                            .unwrap_or(Duration::ZERO);
                        if until_due.is_zero() {
                            self.cfg.check_interval
                        } else {
                            until_due.min(self.cfg.check_interval)
                        }
                    }
                    Some(e) => {
                        state.failures = state.failures.saturating_add(1);
                        let delay = self.retry_delay(state.failures);
                        error!(
                            "ACME renewal of {} failed (attempt {}), retrying in {delay:?}: {e}",
                            state.domain, state.failures
                        );
                        delay
                    }
                };
                state.next_check = Instant::now() + delay;
            }

            match states.iter().map(|s| s.next_check).min() {
                Some(next) => tokio::time::sleep_until(next).await,
                None => std::future::pending::<()>().await,
            }
        }
    }

    /// Loads or renews the certificate of one domain, under the per-domain lock.
    async fn check(&self, domain: &str) -> Checked {
        let domain = match normalize_domain(domain) {
            Ok(d) => d,
            Err(error) => {
                return Checked::Failed {
                    error,
                    fallback: None,
                }
            }
        };
        let lock = self.domain_lock(&domain);
        let _guard = lock.lock().await;

        let path = self.cert_path(&domain);
        let existing = match store::load_cert(&path, &domain).await {
            Ok(cert) => cert,
            Err(e) => {
                warn!("ignoring persisted certificate for {domain}: {e}");
                None
            }
        };
        if let Some(cert) = existing.as_ref() {
            if !self.until_due(cert).is_zero() {
                debug!("using persisted certificate for {domain}");
                return Checked::Current(cert.clone());
            }
            info!(
                "certificate for {domain} expires in {:?}, renewing",
                store::remaining(cert.not_after)
            );
        }

        let error = match tokio::time::timeout(self.cfg.order_timeout, self.order(&domain)).await {
            Ok(Ok(cert)) => return Checked::Current(cert),
            Ok(Err(e)) => e,
            Err(_elapsed) => AcmeError::Timeout {
                domain,
                after: self.cfg.order_timeout,
            },
        };
        Checked::Failed {
            error,
            fallback: existing.filter(|c| c.not_after > SystemTime::now()),
        }
    }

    /// Orders, validates, downloads and persists a certificate.
    async fn order(&self, domain: &str) -> Result<CertPem, AcmeError> {
        let account = self.account(false).await?;
        let cert = match self.run_order(&account, domain).await {
            Err(AcmeError::Protocol {
                source: instant_acme::Error::Api(problem),
                ..
            }) if problem.r#type.as_deref() == Some(ACCOUNT_DOES_NOT_EXIST) => {
                warn!("the CA no longer knows the stored ACME account, registering a new one");
                let account = self.account(true).await?;
                self.run_order(&account, domain).await?
            }
            result => result?,
        };
        store::write_private(self.cert_path(domain), store::bundle(&cert).into_bytes()).await?;
        info!(
            "issued certificate for {domain}, valid for {:?}",
            store::remaining(cert.not_after)
        );
        Ok(cert)
    }

    async fn run_order(&self, account: &Account, domain: &str) -> Result<CertPem, AcmeError> {
        let identifiers = [Identifier::Dns(domain.to_owned())];
        let mut order = account
            .new_order(&NewOrder::new(&identifiers))
            .await
            .map_err(AcmeError::protocol("creating the order"))?;

        // Challenge responses stay published until the order is ready (or this future is
        // dropped), then the guards remove them from the store.
        let mut published: Vec<ChallengeGuard> = Vec::new();
        let mut authorizations = order.authorizations();
        while let Some(authz) = authorizations.next().await {
            let mut authz = authz.map_err(AcmeError::protocol("fetching the authorization"))?;
            match authz.status {
                AuthorizationStatus::Valid => continue,
                AuthorizationStatus::Pending => {}
                status => {
                    return Err(AcmeError::ValidationFailed {
                        domain: domain.to_owned(),
                        detail: format!("authorization is {status:?}"),
                    })
                }
            }
            let mut challenge = authz.challenge(ChallengeType::Http01).ok_or_else(|| {
                AcmeError::NoHttp01Challenge {
                    domain: domain.to_owned(),
                }
            })?;
            let key_authorization = challenge.key_authorization().as_str().to_owned();
            published.push(
                self.challenges
                    .publish(challenge.token.clone(), key_authorization),
            );
            debug!("published http-01 token {} for {domain}", challenge.token);
            challenge
                .set_ready()
                .await
                .map_err(AcmeError::protocol("marking the challenge ready"))?;
        }

        let retry = RetryPolicy::new()
            .initial_delay(Duration::from_millis(500))
            .backoff(1.5)
            .timeout(self.cfg.order_timeout);
        let status = match order.poll_ready(&retry).await {
            Ok(status) => status,
            Err(instant_acme::Error::Api(problem)) => {
                return Err(AcmeError::ValidationFailed {
                    domain: domain.to_owned(),
                    detail: with_challenge_errors(problem.to_string(), &mut order).await,
                })
            }
            Err(e) => return Err(AcmeError::protocol("waiting for validation")(e)),
        };
        drop(published);
        if status != OrderStatus::Ready {
            return Err(AcmeError::ValidationFailed {
                domain: domain.to_owned(),
                detail: with_challenge_errors(format!("order is {status:?}"), &mut order).await,
            });
        }

        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
        let mut params = CertificateParams::new(vec![domain.to_owned()])?;
        params.distinguished_name = DistinguishedName::new();
        let csr = params.serialize_request(&key)?;
        order
            .finalize_csr(csr.der())
            .await
            .map_err(AcmeError::protocol("finalizing the order"))?;
        let chain_pem = order
            .poll_certificate(&retry)
            .await
            .map_err(AcmeError::protocol("downloading the certificate"))?;

        store::validate(domain, &chain_pem, &key.serialize_pem()).map_err(|reason| {
            AcmeError::BadCertificate {
                domain: domain.to_owned(),
                reason,
            }
        })
    }

    /// Returns the account, loading it from disk or registering it on first use.
    /// `force_new` registers a fresh account and overwrites the stored credentials.
    async fn account(&self, force_new: bool) -> Result<Account, AcmeError> {
        let mut slot = self.account.lock().await;
        if !force_new {
            if let Some(account) = slot.as_ref() {
                return Ok(account.clone());
            }
        }

        let path = self.ca_dir.join("account.json");
        let stored = if force_new {
            None
        } else {
            store::read_optional(&path).await?
        };
        let account = match stored {
            Some(bytes) => {
                let credentials: AccountCredentials =
                    serde_json::from_slice(&bytes).map_err(|e| {
                        AcmeError::Config(format!(
                            "corrupt ACME account file {}: {e}",
                            path.display()
                        ))
                    })?;
                let account = Account::builder_with_http(Box::new(self.http.clone()))
                    .from_credentials(credentials)
                    .await
                    .map_err(AcmeError::protocol("loading the account"))?;
                debug!("loaded ACME account {}", account.id());
                account
            }
            None => {
                let contact = format!("mailto:{}", self.cfg.email);
                let contacts: Vec<&str> = if self.cfg.email.is_empty() {
                    Vec::new()
                } else {
                    vec![contact.as_str()]
                };
                let (account, credentials) =
                    Account::builder_with_http(Box::new(self.http.clone()))
                        .create(
                            &NewAccount {
                                contact: &contacts,
                                terms_of_service_agreed: true,
                                only_return_existing: false,
                            },
                            self.cfg.directory_url.clone(),
                            None,
                        )
                        .await
                        .map_err(AcmeError::protocol("registering the account"))?;
                let json = serde_json::to_vec_pretty(&credentials).map_err(|e| {
                    AcmeError::io("failed to serialize", &path, std::io::Error::other(e))
                })?;
                store::write_private(path, json).await?;
                info!("registered ACME account {}", account.id());
                account
            }
        };
        *slot = Some(account.clone());
        Ok(account)
    }

    fn domain_lock(&self, domain: &str) -> Arc<Mutex<()>> {
        self.domain_locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(domain.to_owned())
            .or_default()
            .clone()
    }

    fn cert_path(&self, domain: &str) -> PathBuf {
        self.ca_dir.join("certs").join(format!("{domain}.pem"))
    }

    /// Time until `cert` enters its renewal window; zero if it already has.
    fn until_due(&self, cert: &CertPem) -> Duration {
        store::remaining(cert.not_after).saturating_sub(self.cfg.renew_before)
    }

    fn retry_delay(&self, failures: u32) -> Duration {
        let base = RETRY_BASE.min(self.cfg.check_interval);
        let factor = 1u32 << failures.saturating_sub(1).min(16);
        base.saturating_mul(factor).min(self.cfg.check_interval)
    }
}

/// Appends the CA's per-challenge error details to `summary`.
async fn with_challenge_errors(summary: String, order: &mut Order) -> String {
    let mut details = vec![summary];
    let mut authorizations = order.authorizations();
    while let Some(Ok(mut authz)) = authorizations.next().await {
        if let Ok(state) = authz.refresh().await {
            details.extend(
                state
                    .challenges
                    .iter()
                    .filter_map(|c| c.error.as_ref().map(|p| p.to_string())),
            );
        }
    }
    details.join("; ")
}

/// Lowercases `domain` and checks it is a name HTTP-01 can validate (and a safe file name).
fn normalize_domain(domain: &str) -> Result<String, AcmeError> {
    let invalid = |reason| AcmeError::InvalidDomain {
        domain: domain.to_owned(),
        reason,
    };
    let name = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    if name.starts_with("*.") {
        return Err(invalid(
            "wildcard certificates need DNS-01; HTTP-01 cannot issue them",
        ));
    }
    if name.parse::<IpAddr>().is_ok() {
        return Err(invalid("IP addresses are not supported"));
    }
    if name.is_empty() || name.len() > 253 {
        return Err(invalid("must be 1 to 253 characters long"));
    }
    if !name.contains('.') {
        return Err(invalid("must be a fully qualified domain name"));
    }
    let label_ok = |label: &str| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    };
    if !name.split('.').all(label_ok) {
        return Err(invalid(
            "labels must be 1-63 characters of a-z, 0-9 and '-' (use punycode for IDNs)",
        ));
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_validation() {
        assert_eq!(normalize_domain("Example.COM.").unwrap(), "example.com");
        for bad in [
            "",
            "*.example.com",
            "127.0.0.1",
            "::1",
            "localhost",
            "a..b",
            "-a.com",
            "a/b.com",
            "../x.com",
            "ex ample.com",
        ] {
            assert!(
                matches!(normalize_domain(bad), Err(AcmeError::InvalidDomain { .. })),
                "{bad:?} should be rejected"
            );
        }
    }

    fn manager(dir: &std::path::Path) -> AcmeManager {
        let mut cfg = AcmeConfig::new("https://127.0.0.1:23999/dir", "", dir);
        cfg.check_interval = Duration::from_secs(600);
        AcmeManager::new(cfg, ChallengeStore::default()).unwrap()
    }

    #[test]
    fn config_validation() {
        let dir = tempfile::tempdir().unwrap();
        let bad = |cfg: AcmeConfig| AcmeManager::new(cfg, ChallengeStore::default()).is_err();
        assert!(bad(AcmeConfig::new(
            "http://ca.example/dir",
            "",
            dir.path()
        )));
        assert!(bad(AcmeConfig::new("not a url", "", dir.path())));
        assert!(bad(AcmeConfig::new(
            "https://ca.example/dir",
            "mailto:a@b.c",
            dir.path()
        )));
        let mut cfg = AcmeConfig::new("https://ca.example/dir", "", dir.path());
        cfg.order_timeout = Duration::ZERO;
        assert!(bad(cfg));
        let mut cfg = AcmeConfig::new("https://ca.example/dir", "", dir.path());
        cfg.ca_root_pem = Some(dir.path().join("missing.pem"));
        assert!(matches!(
            AcmeManager::new(cfg, ChallengeStore::default()),
            Err(AcmeError::Io { .. })
        ));
        AcmeManager::new(
            AcmeConfig::new("https://ca.example/dir", "ops@example.com", dir.path()),
            ChallengeStore::default(),
        )
        .unwrap();
        let certs = dir.path().join("ca.example_dir/certs");
        assert!(certs.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&certs).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
    }

    #[test]
    fn retry_backoff_is_capped() {
        let dir = tempfile::tempdir().unwrap();
        let m = manager(dir.path());
        assert_eq!(m.retry_delay(1), Duration::from_secs(60));
        assert_eq!(m.retry_delay(2), Duration::from_secs(120));
        assert_eq!(m.retry_delay(4), Duration::from_secs(480));
        assert_eq!(m.retry_delay(5), Duration::from_secs(600));
        assert_eq!(m.retry_delay(u32::MAX), Duration::from_secs(600));
    }

    #[tokio::test]
    async fn unreachable_ca_is_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = manager(dir.path())
            .ensure_certificate("a.example.com")
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                AcmeError::Protocol {
                    step: "registering the account",
                    ..
                }
            ),
            "{err}"
        );
    }
}
