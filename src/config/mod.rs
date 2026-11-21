//! Configuration: the v1.1 TOML format plus optional v2 keys (see docs/v2/DESIGN.md).

pub mod watcher;

use crate::{
    matcher::Matcher,
    metrics::Metrics,
    middleware::Middlewares,
    router::Router,
    tls::CertKey,
    upstream::{
        strategy::{StickyCookie, Strategy},
        Backend, HealthCheck, HealthKind, Pool, PoolParts, UpstreamSettings,
    },
};
use log::warn;
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{atomic::AtomicUsize, Arc},
    time::Duration,
};
use toml::value::Table;

/// Everything the data plane needs, swapped atomically on reload.
pub struct RuntimeConfig {
    pub http_address: SocketAddr,
    pub https_address: SocketAddr,
    /// Read once at startup, changing it requires a restart.
    pub metrics_address: SocketAddr,
    pub pools: Vec<Arc<Pool>>,
    pub router: Router,
    /// Certificates by lower-case SNI name (`*.example.com` for wildcards).
    pub certificates: HashMap<String, Arc<CertKey>>,
    pub acme: Vec<AcmeCertificate>,
    pub health_interval: Duration,
    /// Maximum in-flight requests of the whole process (0 = unlimited).
    pub max_connections: usize,
    /// Settings that only take effect at startup.
    pub process: ProcessSettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSettings {
    pub threads: usize,
    pub grace_period_secs: u64,
    pub upstream_keepalive_pool_size: usize,
    pub upgrade_sock: String,
    pub pid_file: String,
    pub max_retries: usize,
}

#[derive(Debug, Clone)]
pub struct AcmeCertificate {
    pub domain: String,
    pub staging: bool,
    pub email: String,
    pub persist_dir: PathBuf,
}

/// Reads and validates the configuration. `previous` is the running configuration on reloads:
/// health state, resolved addresses and in-flight counters of unchanged backends carry over.
pub fn load(
    path: &Path,
    metrics: &Metrics,
    previous: Option<&RuntimeConfig>,
) -> Result<RuntimeConfig, String> {
    let toml_str = fs::read_to_string(path).map_err(|e| {
        format!(
            "Error occurred when reading configuration file {}: {}",
            path.display(),
            e
        )
    })?;
    let config: TomlConfig = toml::from_str(&toml_str).map_err(|e| {
        format!(
            "Error occurred when parsing configuration file {}: TOML parse error: {}",
            path.display(),
            e
        )
    })?;
    config.print_warnings();
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("Could not resolve '{}': {}", path.display(), e))?;
    let config_dir = canonical
        .parent()
        .ok_or_else(|| "Config path does not have a parent".to_string())?;
    build(config, config_dir, metrics, previous)
        .map_err(|e| format!("Could not load configuration due to: {}", e))
}

fn build(
    config: TomlConfig,
    config_dir: &Path,
    metrics: &Metrics,
    previous: Option<&RuntimeConfig>,
) -> Result<RuntimeConfig, String> {
    let http_address = parse_address("http_address", &config.http_address)?;
    let https_address = parse_address("https_address", &config.https_address)?;
    let metrics_address = parse_address(
        "prometheus_metrics_address",
        &config.prometheus_metrics_address,
    )?;

    let mut old_backends: HashMap<&str, &Arc<Backend>> = HashMap::new();
    if let Some(previous) = previous {
        for backend in previous.pools.iter().flat_map(|p| p.backends.iter()) {
            old_backends
                .entry(backend.address.as_str())
                .or_insert(backend);
        }
    }
    // One in-flight counter per backend address, shared by pools and kept across reloads
    let mut inflight: HashMap<String, Arc<AtomicUsize>> = HashMap::new();

    let mut pools = Vec::with_capacity(config.backend_pools.len());
    for (index, pool) in config.backend_pools.into_iter().enumerate() {
        let pool = build_pool(index, pool, metrics, &old_backends, &mut inflight)?;
        pools.push(Arc::new(pool));
    }
    let max_retries = pools
        .iter()
        .map(|p| p.upstream.retries)
        .max()
        .unwrap_or(0)
        .saturating_add(1);

    let mut certificates = HashMap::new();
    let mut acme = Vec::new();
    for (sni_name, certificate) in config.certificates {
        let name = sni_name.trim().to_ascii_lowercase();
        if name.is_empty() {
            return Err("certificate name must not be empty".into());
        }
        match certificate {
            CertificateConfig::Local {
                certificate_path,
                private_key_path,
            } => {
                let key = CertKey::load(
                    &config_dir.join(certificate_path),
                    &config_dir.join(private_key_path),
                )?;
                certificates.insert(name, Arc::new(key));
            }
            CertificateConfig::ACME {
                staging,
                email,
                persist_dir,
            } => {
                if name.parse::<std::net::IpAddr>().is_ok() {
                    return Err(format!("ACME does not support IP addresses ('{}')", name));
                }
                acme.push(AcmeCertificate {
                    domain: name,
                    staging,
                    email,
                    persist_dir: config_dir.join(persist_dir),
                });
            }
        }
    }

    let router = Router::new(&pools);
    Ok(RuntimeConfig {
        http_address,
        https_address,
        metrics_address,
        pools,
        router,
        certificates,
        acme,
        health_interval: Duration::from_secs(config.health_interval.check_every),
        max_connections: config.max_connections,
        process: ProcessSettings {
            threads: config.threads.filter(|t| *t > 0).unwrap_or_else(|| {
                std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(1)
            }),
            grace_period_secs: config.grace_period_secs,
            upstream_keepalive_pool_size: config.upstream_keepalive_pool_size.unwrap_or(128),
            upgrade_sock: config
                .upgrade_sock
                .unwrap_or_else(|| "/tmp/ruststrom_upgrade.sock".into()),
            pid_file: config
                .pid_file
                .unwrap_or_else(|| "/tmp/ruststrom.pid".into()),
            max_retries,
        },
    })
}

fn build_pool(
    index: usize,
    config: BackendPoolConfig,
    metrics: &Metrics,
    old_backends: &HashMap<&str, &Arc<Backend>>,
    inflight: &mut HashMap<String, Arc<AtomicUsize>>,
) -> Result<Pool, String> {
    let matcher = Matcher::try_from(config.matcher.as_str())?;
    let backends = config
        .addresses
        .iter()
        .map(|address| {
            let counter = inflight
                .entry(address.clone())
                .or_insert_with(|| match old_backends.get(address.as_str()) {
                    Some(old) => old.inflight_counter(),
                    None => Arc::new(AtomicUsize::new(0)),
                })
                .clone();
            let backend = Backend::new(address, counter, metrics);
            if let Some(old) = old_backends.get(address.as_str()) {
                backend.carry_over(old);
            }
            if !looks_like_host_port(address) {
                warn!(
                    "backend pool at index {}: address '{}' is not host:port, requests to it will fail",
                    index, address
                );
            }
            Arc::new(backend)
        })
        .collect();

    let health = config.health_config;
    let expected_status = match health.expected_status.as_slice() {
        [low, high] if (100..=599).contains(low) && (100..=599).contains(high) && low <= high => {
            (*low, *high)
        }
        other => {
            return Err(format!(
                "backend pool at index {}: expected_status must be [low, high] within 100-599, got {:?}",
                index, other
            ))
        }
    };
    if health.rise == 0 || health.fall == 0 {
        return Err(format!(
            "backend pool at index {}: rise and fall must be at least 1",
            index
        ));
    }
    let health_check = HealthCheck {
        slow_threshold_ms: health.slow_threshold,
        timeout: Duration::from_millis(health.timeout.max(1)),
        path: health.path,
        rise: health.rise,
        fall: health.fall,
        kind: health.kind,
        expected_status,
    };

    let upstream = upstream_settings(index, config.client, config.upstream)?;
    let middlewares = Middlewares::from_table(config.middlewares);

    Ok(Pool::new(PoolParts {
        name: config.matcher,
        matcher,
        http: config.schemes.contains(&SchemeConfig::HTTP),
        https: config.schemes.contains(&SchemeConfig::HTTPS),
        backends,
        strategy: config.strategy.into(),
        health_check,
        upstream,
        middlewares,
    }))
}

fn upstream_settings(
    index: usize,
    client: Option<UpstreamToml>,
    upstream: Option<UpstreamToml>,
) -> Result<UpstreamSettings, String> {
    let client = client.unwrap_or_default();
    let upstream = upstream.unwrap_or_default();
    let mut settings = UpstreamSettings::default();
    // `[backend_pools.upstream]` wins over `[backend_pools.client]`
    let pick = |a: Option<u64>, b: Option<u64>| a.or(b);
    let millis = |name: &str, value: Option<u64>, target: &mut Duration| -> Result<(), String> {
        if let Some(value) = value {
            if value == 0 {
                return Err(format!(
                    "backend pool at index {}: {} must be greater than 0",
                    index, name
                ));
            }
            *target = Duration::from_millis(value);
        }
        Ok(())
    };
    millis(
        "connect_timeout_ms",
        pick(upstream.connect_timeout_ms, client.connect_timeout_ms),
        &mut settings.connect_timeout,
    )?;
    millis(
        "read_timeout_ms",
        pick(upstream.read_timeout_ms, client.read_timeout_ms),
        &mut settings.read_timeout,
    )?;
    millis(
        "write_timeout_ms",
        pick(upstream.write_timeout_ms, client.write_timeout_ms),
        &mut settings.write_timeout,
    )?;
    let idle = pick(upstream.idle_timeout_ms, client.idle_timeout_ms).or_else(|| {
        // v1.1 `pool_idle_timeout`
        upstream
            .pool_idle_timeout
            .or(client.pool_idle_timeout)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX).max(1))
    });
    millis("idle_timeout_ms", idle, &mut settings.idle_timeout)?;
    if upstream
        .pool_max_idle_per_host
        .or(client.pool_max_idle_per_host)
        .is_some()
    {
        warn!(
            "backend pool at index {}: pool_max_idle_per_host is ignored in v2, use the global upstream_keepalive_pool_size",
            index
        );
    }
    if let Some(retries) = upstream.retries.or(client.retries) {
        settings.retries = retries;
    }
    if let Some(tls) = upstream.tls.or(client.tls) {
        settings.tls = tls;
    }
    if let Some(sni) = upstream.sni.or(client.sni) {
        settings.sni = sni;
    }
    if let Some(verify) = upstream.verify_cert.or(client.verify_cert) {
        settings.verify_cert = verify;
    }
    if let Some(h2) = upstream.h2.or(client.h2) {
        settings.h2 = h2;
    }
    if let Some(max) = upstream.max_connections.or(client.max_connections) {
        settings.max_connections = max;
    }
    if settings.h2 && !settings.tls {
        return Err(format!(
            "backend pool at index {}: h2 = true requires tls = true",
            index
        ));
    }
    Ok(settings)
}

fn parse_address(name: &str, value: &str) -> Result<SocketAddr, String> {
    value
        .parse()
        .map_err(|e| format!("invalid {} '{}': {}", name, value, e))
}

fn looks_like_host_port(address: &str) -> bool {
    match address.rsplit_once(':') {
        Some((host, port)) => {
            !host.is_empty() && port.parse::<u16>().is_ok() && !host.contains('/')
        }
        None => false,
    }
}

#[derive(Debug, Deserialize)]
struct TomlConfig {
    #[serde(default = "default_http_address")]
    http_address: String,
    #[serde(default = "default_https_address")]
    https_address: String,
    #[serde(default)]
    backend_pools: Vec<BackendPoolConfig>,
    #[serde(default)]
    certificates: HashMap<String, CertificateConfig>,
    #[serde(default = "default_health_interval_config")]
    health_interval: HealthIntervalConfig,
    #[serde(default = "default_metrics_address")]
    prometheus_metrics_address: String,
    // New in v2, all optional
    #[serde(default)]
    max_connections: usize,
    threads: Option<usize>,
    #[serde(default = "default_grace_period")]
    grace_period_secs: u64,
    upstream_keepalive_pool_size: Option<usize>,
    upgrade_sock: Option<String>,
    pid_file: Option<String>,
}

impl TomlConfig {
    fn print_warnings(&self) {
        if self.backend_pools.is_empty() {
            warn!("No backend pool found.");
        }
        for (index, pool) in self.backend_pools.iter().enumerate() {
            if pool.schemes.is_empty() {
                warn!("backend pool at index {} is unreachable, since no schemes are registered. Consider adding `HTTP` or `HTTPS` to the schemes array.", index);
            }
            if pool.addresses.is_empty() {
                warn!(
                    "backend pool at index {} does not contain any addresses. It will always result in 503 errors.",
                    index
                );
            }
        }
    }
}

/// Metrics are unauthenticated, so they are only reachable locally unless
/// `prometheus_metrics_address` says otherwise.
fn default_metrics_address() -> String {
    "127.0.0.1:9090".to_string()
}

// Dual Stack if /proc/sys/net/ipv6/bindv6only has default value 0
fn default_http_address() -> String {
    "[::]:80".to_string()
}

fn default_https_address() -> String {
    "[::]:443".to_string()
}

fn default_health_interval_config() -> HealthIntervalConfig {
    HealthIntervalConfig { check_every: 10 }
}

fn default_grace_period() -> u64 {
    30
}

#[derive(Debug, Deserialize)]
struct BackendPoolConfig {
    matcher: String,
    addresses: Vec<String>,
    schemes: HashSet<SchemeConfig>,
    client: Option<UpstreamToml>,
    upstream: Option<UpstreamToml>,
    #[serde(default)]
    health_config: HealthToml,
    strategy: StrategyConfig,
    #[serde(default)]
    middlewares: Table,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[allow(clippy::upper_case_acronyms)]
enum SchemeConfig {
    HTTP,
    HTTPS,
}

/// `[backend_pools.client]` (v1.1 keys plus the v2 ones) and `[backend_pools.upstream]`.
#[derive(Debug, Default, Deserialize)]
struct UpstreamToml {
    pool_idle_timeout: Option<Duration>,
    pool_max_idle_per_host: Option<usize>,
    connect_timeout_ms: Option<u64>,
    read_timeout_ms: Option<u64>,
    write_timeout_ms: Option<u64>,
    idle_timeout_ms: Option<u64>,
    retries: Option<usize>,
    tls: Option<bool>,
    sni: Option<String>,
    verify_cert: Option<bool>,
    h2: Option<bool>,
    max_connections: Option<usize>,
}

#[derive(Debug, Deserialize, PartialEq)]
enum StrategyConfig {
    StickyCookie {
        cookie_name: String,
        http_only: bool,
        secure: bool,
        same_site: SameSiteConfig,
        inner: Box<StrategyConfig>,
    },
    Random,
    IPHash,
    LeastConnection,
    RoundRobin,
}

impl From<StrategyConfig> for Strategy {
    fn from(other: StrategyConfig) -> Self {
        match other {
            StrategyConfig::StickyCookie {
                cookie_name,
                http_only,
                secure,
                same_site,
                inner,
            } => Strategy::StickyCookie(Box::new(StickyCookie {
                cookie_name,
                http_only,
                secure,
                same_site: same_site.into(),
                inner: (*inner).into(),
            })),
            StrategyConfig::Random => Strategy::Random,
            StrategyConfig::IPHash => Strategy::IPHash,
            StrategyConfig::RoundRobin => Strategy::RoundRobin(AtomicUsize::new(0)),
            StrategyConfig::LeastConnection => Strategy::LeastConnection,
        }
    }
}

#[derive(Debug, Deserialize, PartialEq)]
enum SameSiteConfig {
    Strict,
    Lax,
    None,
}

impl From<SameSiteConfig> for cookie::SameSite {
    fn from(other: SameSiteConfig) -> Self {
        match other {
            SameSiteConfig::Strict => cookie::SameSite::Strict,
            SameSiteConfig::Lax => cookie::SameSite::Lax,
            SameSiteConfig::None => cookie::SameSite::None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum CertificateConfig {
    Local {
        certificate_path: String,
        private_key_path: String,
    },
    #[allow(clippy::upper_case_acronyms)]
    ACME {
        staging: bool,
        email: String,
        persist_dir: String,
    },
}

#[derive(Debug, Deserialize)]
struct HealthIntervalConfig {
    #[serde(default = "default_check_every")]
    check_every: u64,
}

fn default_check_every() -> u64 {
    10
}

#[derive(Debug, Deserialize)]
struct HealthToml {
    #[serde(default = "default_slow_threshold")]
    slow_threshold: i64,
    #[serde(default = "default_timeout")]
    timeout: u64,
    #[serde(default = "default_path")]
    path: String,
    #[serde(default = "default_rise")]
    rise: u32,
    #[serde(default = "default_fall")]
    fall: u32,
    #[serde(default = "default_kind")]
    kind: HealthKind,
    #[serde(default = "default_expected_status")]
    expected_status: Vec<u16>,
}

impl Default for HealthToml {
    fn default() -> Self {
        HealthToml {
            slow_threshold: default_slow_threshold(),
            timeout: default_timeout(),
            path: default_path(),
            rise: default_rise(),
            fall: default_fall(),
            kind: default_kind(),
            expected_status: default_expected_status(),
        }
    }
}

impl<'de> Deserialize<'de> for HealthKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        match value.to_ascii_lowercase().as_str() {
            "http" => Ok(HealthKind::Http),
            "tcp" => Ok(HealthKind::Tcp),
            other => Err(serde::de::Error::custom(format!(
                "unknown health check kind '{}', expected \"http\" or \"tcp\"",
                other
            ))),
        }
    }
}

fn default_slow_threshold() -> i64 {
    300
}

fn default_timeout() -> u64 {
    500
}

fn default_path() -> String {
    "/".to_string()
}

/// Without `rise`/`fall` a single check changes the state, exactly like v1.1, so existing
/// configurations keep their failover timing. `rise = 2, fall = 3` (HAProxy's defaults) is
/// the recommended setting against flapping.
fn default_rise() -> u32 {
    1
}

fn default_fall() -> u32 {
    1
}

fn default_kind() -> HealthKind {
    HealthKind::Http
}

fn default_expected_status() -> Vec<u16> {
    vec![200, 399]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn load_str(content: &str) -> Result<RuntimeConfig, String> {
        let dir = std::env::temp_dir().join(format!(
            "ruststrom-config-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::File::create(&path)
            .unwrap()
            .write_all(content.as_bytes())
            .unwrap();
        let metrics = Metrics::new().unwrap();
        let result = load(&path, &metrics, None);
        let _ = fs::remove_dir_all(&dir);
        result
    }

    #[test]
    fn shipped_configs_load() {
        for file in ["configs/config.toml", "docker-config.toml"] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
            let metrics = Metrics::new().unwrap();
            let config = load(&path, &metrics, None);
            assert!(config.is_ok(), "{}: {:?}", file, config.err());
        }
    }

    #[test]
    fn readme_style_config_loads_with_defaults() {
        let config = load_str(
            r#"
            [[backend_pools]]
            matcher = "Host('example.com') && PathRegexp('^/api/')"
            addresses = ["127.0.0.1:8080"]
            schemes = ["HTTP", "HTTPS"]
            strategy = { StickyCookie = { cookie_name = "S", http_only = true, secure = true, same_site = "Strict", inner = { RoundRobin = {} } } }
            [backend_pools.client]
            pool_idle_timeout = { secs = 90, nanos = 0 }
            pool_max_idle_per_host = 32
            [backend_pools.middlewares.RateLimiter]
            limit = 100
            window_sec = 60
            [backend_pools.middlewares.Unknown]
            "#,
        )
        .unwrap();
        assert_eq!(config.http_address.to_string(), "[::]:80");
        assert_eq!(config.metrics_address.to_string(), "127.0.0.1:9090");
        assert_eq!(config.health_interval, Duration::from_secs(10));
        let pool = &config.pools[0];
        assert!(pool.http && pool.https);
        assert_eq!(pool.upstream.idle_timeout, Duration::from_secs(90));
        assert_eq!(pool.upstream.retries, 1);
        assert_eq!(pool.health_check.expected_status, (200, 399));
        assert_eq!(config.process.grace_period_secs, 30);
    }

    #[test]
    fn new_keys_parse_and_validate() {
        let config = load_str(
            r#"
            max_connections = 1000
            threads = 2
            [[backend_pools]]
            matcher = "PathRegexp('^/')"
            addresses = ["backend1:8080"]
            schemes = ["HTTP"]
            strategy = { LeastConnection = {} }
            [backend_pools.upstream]
            connect_timeout_ms = 100
            retries = 3
            tls = true
            sni = "internal"
            verify_cert = false
            h2 = true
            max_connections = 5
            [backend_pools.health_config]
            kind = "tcp"
            rise = 1
            fall = 1
            expected_status = [200, 204]
            "#,
        )
        .unwrap();
        assert_eq!(config.max_connections, 1000);
        assert_eq!(config.process.threads, 2);
        assert_eq!(config.process.max_retries, 4);
        let pool = &config.pools[0];
        assert_eq!(pool.upstream.connect_timeout, Duration::from_millis(100));
        assert!(pool.upstream.h2 && pool.upstream.tls && !pool.upstream.verify_cert);
        assert_eq!(pool.health_check.kind, HealthKind::Tcp);

        let h2_without_tls = load_str(
            r#"
            [[backend_pools]]
            matcher = "PathRegexp('^/')"
            addresses = ["127.0.0.1:1"]
            schemes = ["HTTP"]
            strategy = { Random = {} }
            [backend_pools.upstream]
            h2 = true
            "#,
        );
        assert!(h2_without_tls.is_err());
    }

    #[test]
    fn invalid_configs_are_errors() {
        assert!(load_str("http_address = \"nonsense\"").is_err());
        assert!(load_str("this is not toml").is_err());
        assert!(load_str(
            r#"
            [[backend_pools]]
            matcher = "Host('broken'"
            addresses = []
            schemes = ["HTTP"]
            strategy = { Random = {} }
            "#
        )
        .is_err());
    }

    #[test]
    fn reload_carries_health_over() {
        let content = r#"
            [[backend_pools]]
            matcher = "PathRegexp('^/')"
            addresses = ["127.0.0.1:1", "127.0.0.1:2"]
            schemes = ["HTTP"]
            strategy = { RoundRobin = {} }
        "#;
        let first = load_str(content).unwrap();
        first.pools[0].backends[0].set_health(crate::upstream::Health::Down);
        let _guard = first.pools[0].backends[1].track_inflight();
        let dir = std::env::temp_dir().join(format!("ruststrom-reload-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::write(&path, content).unwrap();
        let metrics = Metrics::new().unwrap();
        let second = load(&path, &metrics, Some(&first)).unwrap();
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(
            second.pools[0].backends[0].health.get(),
            crate::upstream::Health::Down
        );
        assert_eq!(second.pools[0].backends[1].inflight(), 1);
        assert_eq!(second.pools[0].snapshot().backends.len(), 1);
    }
}
