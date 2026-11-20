//! ACME (HTTP-01) integration of `crates/ruststrom-acme`.
//!
//! * The proxy answers `/.well-known/acme-challenge/<token>` from the shared
//!   [`ChallengeStore`] in `request_filter`, before routing and middlewares.
//! * [`AcmeService`] runs one renewal loop per (directory, email, persist_dir) group of
//!   `type = "ACME"` certificates and installs every issued certificate in the SNI store
//!   ([`State::install_certificate`]). Managers survive reloads; a loop is restarted only when
//!   its domain list changes.

use crate::{config::AcmeCertificate, state::State, tls::CertKey};
use async_trait::async_trait;
use log::{error, info, warn};
use pingora_core::{server::ShutdownWatch, services::background::BackgroundService};
use ruststrom_acme::{
    AcmeConfig, AcmeManager, CertPem, LETS_ENCRYPT_PRODUCTION, LETS_ENCRYPT_STAGING,
};
pub use ruststrom_acme::{ChallengeStore, CHALLENGE_PATH_PREFIX};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::task::JoinHandle;

/// Identifies an ACME account: certificates of one group share a manager and a renewal loop.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Group {
    directory_url: String,
    email: String,
    persist_dir: PathBuf,
}

fn groups(certificates: &[AcmeCertificate]) -> BTreeMap<Group, BTreeSet<String>> {
    let mut groups: BTreeMap<Group, BTreeSet<String>> = BTreeMap::new();
    for certificate in certificates {
        let directory_url = if certificate.staging {
            LETS_ENCRYPT_STAGING
        } else {
            LETS_ENCRYPT_PRODUCTION
        };
        groups
            .entry(Group {
                directory_url: directory_url.to_string(),
                email: certificate.email.clone(),
                persist_dir: certificate.persist_dir.clone(),
            })
            .or_default()
            .insert(certificate.domain.clone());
    }
    groups
}

struct Running {
    manager: Arc<AcmeManager>,
    domains: BTreeSet<String>,
    task: Option<JoinHandle<()>>,
}

/// Background service that keeps ACME certificates issued and renewed.
pub struct AcmeService {
    pub state: Arc<State>,
}

#[async_trait]
impl BackgroundService for AcmeService {
    async fn start(&self, mut shutdown: ShutdownWatch) {
        // The renewal loop may order right away; the HTTP listener must be accepting by then
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            _ = shutdown.changed() => return,
        }
        let mut running: HashMap<Group, Running> = HashMap::new();
        loop {
            let wanted = groups(&self.state.config.load().acme);
            self.reconcile(&mut running, wanted);
            tokio::select! {
                _ = self.state.acme_changed.notified() => {}
                _ = shutdown.changed() => break,
            }
        }
        for (_, group) in running.drain() {
            if let Some(task) = group.task {
                task.abort();
            }
        }
    }
}

impl AcmeService {
    fn reconcile(
        &self,
        running: &mut HashMap<Group, Running>,
        wanted: BTreeMap<Group, BTreeSet<String>>,
    ) {
        // Stop loops of groups that are gone or whose domains changed
        running.retain(|group, current| {
            let keep = wanted.get(group) == Some(&current.domains);
            if !keep {
                if let Some(task) = current.task.take() {
                    task.abort();
                }
            }
            // Keep the manager (account, rate limits) if the group still exists
            wanted.contains_key(group)
        });
        for (group, domains) in wanted {
            let restart = running
                .get(&group)
                .is_none_or(|current| current.task.is_none() || current.domains != domains);
            if !restart {
                continue;
            }
            let manager = match running.get(&group) {
                Some(current) => current.manager.clone(),
                None => {
                    let config = AcmeConfig::new(
                        group.directory_url.clone(),
                        group.email.clone(),
                        group.persist_dir.clone(),
                    );
                    match AcmeManager::new(config, self.state.challenges.clone()) {
                        Ok(manager) => Arc::new(manager),
                        Err(e) => {
                            error!(
                                "ACME for {:?} disabled: {}",
                                domains.iter().collect::<Vec<_>>(),
                                e
                            );
                            continue;
                        }
                    }
                }
            };
            info!(
                "ACME: managing certificates for {:?} ({})",
                domains.iter().collect::<Vec<_>>(),
                group.directory_url
            );
            let state = self.state.clone();
            let task = tokio::spawn(manager.clone().run_renewal_loop(
                domains.iter().cloned().collect(),
                move |domain: &str, cert: CertPem| install(&state, domain, &cert),
            ));
            running.insert(
                group,
                Running {
                    manager,
                    domains,
                    task: Some(task),
                },
            );
        }
        // Certificates of domains no longer configured for ACME are not served anymore
        let configured: BTreeSet<String> = running
            .values()
            .flat_map(|r| r.domains.iter().cloned())
            .collect();
        self.state
            .retain_certificates(|domain| configured.contains(domain));
    }
}

/// Installs an issued or renewed certificate. Idempotent.
fn install(state: &State, domain: &str, cert: &CertPem) {
    match CertKey::from_pem(cert.chain_pem.as_bytes(), cert.key_pem.as_bytes()) {
        Ok(key) => {
            state.install_certificate(domain, key);
            info!("ACME: installed certificate for {}", domain);
        }
        Err(e) => warn!("ACME: certificate for {} is not usable: {}", domain, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn certificate(domain: &str, staging: bool, email: &str) -> AcmeCertificate {
        AcmeCertificate {
            domain: domain.into(),
            staging,
            email: email.into(),
            persist_dir: PathBuf::from("/tmp/acme"),
        }
    }

    #[test]
    fn groups_by_account() {
        let groups = groups(&[
            certificate("a.test", false, "ops@x"),
            certificate("b.test", false, "ops@x"),
            certificate("c.test", true, "ops@x"),
        ]);
        assert_eq!(groups.len(), 2);
        let production = groups
            .iter()
            .find(|(g, _)| g.directory_url == LETS_ENCRYPT_PRODUCTION)
            .unwrap();
        assert_eq!(production.1.len(), 2);
    }

    #[test]
    fn challenge_paths() {
        let store = ChallengeStore::default();
        assert_eq!(
            store.response_for_path("/.well-known/acme-challenge/x"),
            None
        );
        assert!(CHALLENGE_PATH_PREFIX.starts_with("/.well-known/"));
    }
}
