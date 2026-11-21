//! Process wide state shared by the proxy, the TLS callback and the background services.

use crate::{acme::ChallengeStore, config::RuntimeConfig, metrics::Metrics, tls::CertKey};
use arc_swap::ArcSwap;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{atomic::AtomicUsize, Arc},
};
use tokio::sync::Notify;

pub struct State {
    pub config: ArcSwap<RuntimeConfig>,
    pub metrics: Arc<Metrics>,
    pub config_path: PathBuf,
    /// HTTP-01 challenge tokens answered on `/.well-known/acme-challenge/<token>`.
    pub challenges: ChallengeStore,
    /// Certificates installed at runtime (ACME); they survive config reloads.
    pub dynamic_certificates: ArcSwap<HashMap<String, Arc<CertKey>>>,
    /// In-flight requests of the whole process (for `max_connections` and the drain on exit).
    pub active_requests: Arc<AtomicUsize>,
    /// Signalled after a reload so that the ACME service picks up changed certificates.
    pub acme_changed: Notify,
}

impl State {
    pub fn new(config: RuntimeConfig, metrics: Arc<Metrics>, config_path: PathBuf) -> State {
        State {
            config: ArcSwap::from_pointee(config),
            metrics,
            config_path,
            challenges: ChallengeStore::default(),
            dynamic_certificates: ArcSwap::from_pointee(HashMap::new()),
            active_requests: Arc::new(AtomicUsize::new(0)),
            acme_changed: Notify::new(),
        }
    }

    /// Installs or replaces a runtime certificate (used by the ACME renewal loop).
    pub fn install_certificate(&self, domain: &str, key: CertKey) {
        let mut certificates = HashMap::clone(&self.dynamic_certificates.load());
        certificates.insert(domain.to_ascii_lowercase(), Arc::new(key));
        self.dynamic_certificates.store(Arc::new(certificates));
    }

    /// Drops runtime certificates whose domain no longer passes `keep`.
    pub fn retain_certificates(&self, keep: impl Fn(&str) -> bool) {
        let current = self.dynamic_certificates.load();
        if current.keys().all(|domain| keep(domain)) {
            return;
        }
        let mut certificates = HashMap::clone(&current);
        certificates.retain(|domain, _| keep(domain));
        self.dynamic_certificates.store(Arc::new(certificates));
    }

    /// Called after a configuration reload was swapped in.
    pub fn on_config_change(&self) {
        self.acme_changed.notify_one();
    }
}
