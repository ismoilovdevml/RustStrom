//! Backends, pools and their health state.

pub mod health;
pub mod strategy;

use crate::{matcher::Matcher, metrics::Metrics, middleware::Middlewares};
use arc_swap::{ArcSwap, ArcSwapOption};
use pingora_core::{protocols::tls::ALPN, upstreams::peer::HttpPeer};
use prometheus::{Histogram, IntCounter, IntGauge};
use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use strategy::{Ring, Strategy};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scheme {
    Http,
    Https,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Healthy,
    Slow,
    Down,
}

impl Health {
    fn from_u8(value: u8) -> Health {
        match value {
            0 => Health::Healthy,
            1 => Health::Slow,
            _ => Health::Down,
        }
    }

    fn as_u8(self) -> u8 {
        match self {
            Health::Healthy => 0,
            Health::Slow => 1,
            Health::Down => 2,
        }
    }

    fn gauge_value(self) -> i64 {
        match self {
            Health::Healthy => 2,
            Health::Slow => 1,
            Health::Down => 0,
        }
    }
}

/// Health of one backend, updated by the health checker, read lock-free by the data plane.
#[derive(Debug, Default)]
pub struct HealthState {
    status: AtomicU8,
    ok_streak: AtomicU32,
    fail_streak: AtomicU32,
    last_response_ms: AtomicU64,
}

impl HealthState {
    pub fn get(&self) -> Health {
        Health::from_u8(self.status.load(Ordering::Acquire))
    }

    fn set(&self, health: Health) {
        self.status.store(health.as_u8(), Ordering::Release);
    }

    fn copy_from(&self, other: &HealthState) {
        self.status
            .store(other.status.load(Ordering::Acquire), Ordering::Release);
        self.ok_streak
            .store(other.ok_streak.load(Ordering::Relaxed), Ordering::Relaxed);
        self.fail_streak
            .store(other.fail_streak.load(Ordering::Relaxed), Ordering::Relaxed);
        self.last_response_ms.store(
            other.last_response_ms.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
    }
}

#[derive(Debug)]
pub struct Backend {
    /// The address as configured, `host:port`.
    pub address: String,
    /// Opaque sticky cookie id.
    pub id: String,
    /// Resolved socket address; hostnames are resolved by the health checker.
    pub resolved: ArcSwapOption<SocketAddr>,
    pub health: HealthState,
    inflight: Arc<AtomicUsize>,
    pub response_time: Histogram,
    pub failures: IntCounter,
    pub health_gauge: IntGauge,
}

impl Backend {
    pub fn new(address: &str, inflight: Arc<AtomicUsize>, metrics: &Metrics) -> Backend {
        let resolved = address.parse::<SocketAddr>().ok().map(Arc::new);
        let health_gauge = metrics.backend_health.with_label_values(&[address]);
        health_gauge.set(Health::Healthy.gauge_value());
        Backend {
            address: address.to_string(),
            id: strategy::backend_id(address),
            resolved: ArcSwapOption::new(resolved),
            health: HealthState::default(),
            inflight,
            response_time: metrics.backend_response_time.with_label_values(&[address]),
            failures: metrics.backend_failures.with_label_values(&[address]),
            health_gauge,
        }
    }

    pub fn inflight(&self) -> usize {
        self.inflight.load(Ordering::Relaxed)
    }

    pub fn inflight_counter(&self) -> Arc<AtomicUsize> {
        self.inflight.clone()
    }

    /// Counts a request on this backend until the guard is dropped.
    pub fn track_inflight(&self) -> CounterGuard {
        CounterGuard::acquire(self.inflight.clone())
    }

    pub fn socket_addr(&self) -> Option<SocketAddr> {
        self.resolved.load().as_deref().copied()
    }

    /// Takes over health and resolved address of the same backend from a previous config.
    pub fn carry_over(&self, previous: &Backend) {
        self.health.copy_from(&previous.health);
        self.health_gauge.set(self.health.get().gauge_value());
        if self.resolved.load().is_none() {
            self.resolved.store(previous.resolved.load_full());
        }
    }

    pub fn set_health(&self, health: Health) {
        self.health.set(health);
        self.health_gauge.set(health.gauge_value());
    }
}

/// RAII counter, used for in-flight requests per backend, per pool and per process.
#[derive(Debug)]
pub struct CounterGuard(Arc<AtomicUsize>);

impl CounterGuard {
    pub fn acquire(counter: Arc<AtomicUsize>) -> CounterGuard {
        counter.fetch_add(1, Ordering::Relaxed);
        CounterGuard(counter)
    }

    /// Increments the counter unless it already reached `limit` (0 = unlimited).
    pub fn try_acquire(counter: &Arc<AtomicUsize>, limit: usize) -> Option<CounterGuard> {
        if limit == 0 {
            return Some(CounterGuard::acquire(counter.clone()));
        }
        let previous = counter.fetch_add(1, Ordering::AcqRel);
        if previous >= limit {
            counter.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(CounterGuard(counter.clone()))
    }
}

impl Drop for CounterGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// The backends a pool sends traffic to: the healthy ones, or the slow ones when none is
/// healthy (v1.1 semantics). Rebuilt only when health changes.
#[derive(Debug)]
pub struct Snapshot {
    pub backends: Vec<Arc<Backend>>,
    pub ring: Option<Ring>,
}

impl Snapshot {
    pub fn new(backends: Vec<Arc<Backend>>, with_ring: bool) -> Snapshot {
        let ring = with_ring.then(|| Ring::new(&backends));
        Snapshot { backends, ring }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthKind {
    Http,
    Tcp,
}

#[derive(Debug, Clone)]
pub struct HealthCheck {
    pub slow_threshold_ms: i64,
    pub timeout: Duration,
    pub path: String,
    pub rise: u32,
    pub fall: u32,
    pub kind: HealthKind,
    pub expected_status: (u16, u16),
}

#[derive(Debug, Clone)]
pub struct UpstreamSettings {
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub write_timeout: Duration,
    pub idle_timeout: Duration,
    pub retries: usize,
    pub tls: bool,
    pub sni: String,
    pub verify_cert: bool,
    pub h2: bool,
    pub max_connections: usize,
}

impl Default for UpstreamSettings {
    fn default() -> Self {
        UpstreamSettings {
            connect_timeout: Duration::from_millis(3000),
            read_timeout: Duration::from_millis(30000),
            write_timeout: Duration::from_millis(30000),
            idle_timeout: Duration::from_millis(60000),
            retries: 1,
            tls: false,
            sni: String::new(),
            verify_cert: true,
            h2: false,
            max_connections: 0,
        }
    }
}

#[derive(Debug)]
pub struct Pool {
    /// The matcher as written in the config, for logs.
    pub name: String,
    pub matcher: Matcher,
    pub http: bool,
    pub https: bool,
    pub backends: Vec<Arc<Backend>>,
    working: ArcSwap<Snapshot>,
    pub strategy: Strategy,
    pub health_check: HealthCheck,
    pub upstream: UpstreamSettings,
    pub middlewares: Middlewares,
    pub inflight: Arc<AtomicUsize>,
}

pub struct PoolParts {
    pub name: String,
    pub matcher: Matcher,
    pub http: bool,
    pub https: bool,
    pub backends: Vec<Arc<Backend>>,
    pub strategy: Strategy,
    pub health_check: HealthCheck,
    pub upstream: UpstreamSettings,
    pub middlewares: Middlewares,
}

impl Pool {
    pub fn new(parts: PoolParts) -> Pool {
        let pool = Pool {
            name: parts.name,
            matcher: parts.matcher,
            http: parts.http,
            https: parts.https,
            backends: parts.backends,
            working: ArcSwap::from_pointee(Snapshot::new(Vec::new(), false)),
            strategy: parts.strategy,
            health_check: parts.health_check,
            upstream: parts.upstream,
            middlewares: parts.middlewares,
            inflight: Arc::new(AtomicUsize::new(0)),
        };
        pool.rebuild_snapshot();
        pool
    }

    pub fn supports(&self, scheme: Scheme) -> bool {
        match scheme {
            Scheme::Http => self.http,
            Scheme::Https => self.https,
        }
    }

    pub fn snapshot(&self) -> arc_swap::Guard<Arc<Snapshot>> {
        self.working.load()
    }

    /// Recomputes the working set from the current health of the backends.
    pub fn rebuild_snapshot(&self) {
        let healthy: Vec<_> = self
            .backends
            .iter()
            .filter(|b| b.health.get() == Health::Healthy)
            .cloned()
            .collect();
        let working = if healthy.is_empty() {
            self.backends
                .iter()
                .filter(|b| b.health.get() == Health::Slow)
                .cloned()
                .collect()
        } else {
            healthy
        };
        self.working
            .store(Arc::new(Snapshot::new(working, self.strategy.needs_ring())));
    }

    /// The upstream peer for one attempt.
    pub fn peer(&self, backend: &Backend, address: SocketAddr) -> Box<HttpPeer> {
        let settings = &self.upstream;
        let sni = if settings.tls && settings.sni.is_empty() {
            host_part(&backend.address).to_string()
        } else {
            settings.sni.clone()
        };
        let mut peer = Box::new(HttpPeer::new(address, settings.tls, sni));
        let options = &mut peer.options;
        options.connection_timeout = Some(settings.connect_timeout);
        options.read_timeout = Some(settings.read_timeout);
        options.write_timeout = Some(settings.write_timeout);
        options.idle_timeout = Some(settings.idle_timeout);
        options.verify_cert = settings.verify_cert;
        options.verify_hostname = settings.verify_cert;
        options.alpn = if settings.h2 { ALPN::H2H1 } else { ALPN::H1 };
        peer
    }
}

/// `host` of `host:port`, without IPv6 brackets.
pub fn host_part(address: &str) -> &str {
    let host = match address.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) => host,
        _ => address,
    };
    host.trim_start_matches('[').trim_end_matches(']')
}

#[cfg(test)]
pub fn test_backend(address: &str) -> Arc<Backend> {
    let metrics = Metrics::new().unwrap();
    Arc::new(Backend::new(
        address,
        Arc::new(AtomicUsize::new(0)),
        &metrics,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_part_strips_port_and_brackets() {
        assert_eq!(host_part("example.com:443"), "example.com");
        assert_eq!(host_part("[::1]:8080"), "::1");
        assert_eq!(host_part("example.com"), "example.com");
    }

    #[test]
    fn counter_guard_limits() {
        let counter = Arc::new(AtomicUsize::new(0));
        let a = CounterGuard::try_acquire(&counter, 2).unwrap();
        let _b = CounterGuard::try_acquire(&counter, 2).unwrap();
        assert!(CounterGuard::try_acquire(&counter, 2).is_none());
        drop(a);
        assert!(CounterGuard::try_acquire(&counter, 2).is_some());
        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn literal_addresses_resolve_without_dns() {
        assert!(test_backend("127.0.0.1:80").socket_addr().is_some());
        assert!(test_backend("backend1:8080").socket_addr().is_none());
    }
}
