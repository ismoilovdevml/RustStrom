use crate::{
    acme::AcmeHandler,
    algorithms::{
        ip_hash::IPHash, least_connection::LeastConnection, random::Random,
        round_robin::RoundRobin, sticky_cookie::StickyCookie, LoadBalancingStrategy,
    },
    backend_pool_matcher::BackendPoolMatcher,
    health::{HealthConfig, Healthiness},
    middleware::{
        authentication::Authentication, compression::Compression,
        custom_error_pages::CustomErrorPages, https_redirector::HttpsRedirector,
        maxbodysize::MaxBodySize, rate_limiter::RateLimiter, Middleware, MiddlewareChain,
    },
    server::{BackendPool, BackendPoolBuilder, Scheme, SharedData},
    tls::{certified_key_from_acme_certificate, load_certified_key},
};
use arc_swap::ArcSwap;
use log::{info, trace, warn};
use notify::{watcher, DebouncedEvent, RecursiveMode, Watcher};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    convert::{TryFrom, TryInto},
    error::Error,
    fmt::Debug,
    fs, io,
    net::SocketAddr,
    path::Path,
    sync::{mpsc::channel, Arc},
    thread::spawn,
    time::Duration,
};
use tokio::sync::watch;
use tokio_rustls::rustls::{sign::CertifiedKey, ServerName};
use toml::{value::Table, Value};

pub async fn read_initial_config<P: AsRef<Path>>(
    path: P,
) -> Result<Arc<ArcSwap<RuntimeConfig>>, io::Error> {
    let acme_handler = Arc::new(AcmeHandler::new());
    // Don't initialize ACME certificates on startup, because the HTTP listener is not running yet
    let init_acme = false;
    let config = read_runtime_config(&path, acme_handler, init_acme)
        .await
        .map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("Could not load configuration due to: {}", e),
            )
        })?;
    Ok(Arc::new(ArcSwap::from_pointee(config)))
}

pub async fn watch_config<P>(path: P, config: Arc<ArcSwap<RuntimeConfig>>) -> Result<(), io::Error>
where
    P: AsRef<Path> + Send + 'static,
{
    let config_path = path.as_ref().to_path_buf();
    let mut receiver = start_config_watcher(path)?;
    // The initial value of the channel is not a change, the config was just loaded
    receiver.borrow_and_update();
    loop {
        receiver.changed().await.map_err(broken_pipe)?;
        let old_config = config.load();
        let acme_handler = old_config.shared_data.acme_handler.clone();
        match read_runtime_config(&config_path, acme_handler, true).await {
            Ok(new_config) => {
                warn_about_ineffectual_config_changes(&old_config, &new_config);
                carry_over_health(&old_config, &new_config);
                config.store(Arc::new(new_config));
                info!("Reloaded configuration");
            }
            Err(e) => {
                warn!("Could not reload configuration due to: {}", e);
                warn!("Keeping old configuration")
            }
        }
    }
}

/// Keeps the last known health of backends that exist in both configurations,
/// so a reload does not send traffic to backends that are known to be down.
fn carry_over_health(old: &RuntimeConfig, new: &RuntimeConfig) {
    let known: HashMap<&str, Arc<Healthiness>> = old
        .shared_data
        .backend_pools
        .iter()
        .flat_map(|pool| pool.addresses.iter())
        .map(|(address, healthiness)| (address.as_str(), healthiness.load_full()))
        .collect();
    for pool in &new.shared_data.backend_pools {
        for (address, healthiness) in &pool.addresses {
            if let Some(previous) = known.get(address.as_str()) {
                healthiness.store(previous.clone());
            }
        }
    }
}

fn warn_about_ineffectual_config_changes(old: &RuntimeConfig, new: &RuntimeConfig) {
    if old.http_address != new.http_address {
        warn!(
            "A restart is required for the new http_address '{}' to take effect",
            new.http_address
        );
    }
    if old.metrics_address != new.metrics_address {
        warn!(
            "A restart is required for the new prometheus_metrics_address '{}' to take effect",
            new.metrics_address
        );
    }
    if old.https_address != new.https_address {
        warn!(
            "A restart is required for the new https_address '{}' to take effect",
            new.https_address
        );
    }
}

/// Watches the directory of the config file instead of the file itself: editors
/// (vim, `sed -i`), `mv` and Kubernetes ConfigMaps replace the file atomically,
/// which silently ends a watch on the original inode.
fn start_config_watcher<P>(path: P) -> Result<watch::Receiver<u64>, io::Error>
where
    P: AsRef<Path> + Send + 'static,
{
    let config_path = path.as_ref().canonicalize()?;
    let directory = config_path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Config path has no parent"))?
        .to_path_buf();
    let file_name = config_path.file_name().map(|it| it.to_os_string());
    let (sender, receiver) = watch::channel(0u64);
    spawn(move || {
        if let Err(e) = watch_config_blocking(&directory, file_name, sender) {
            warn!("Configuration hot reload stopped: {}", e);
        }
    });
    Ok(receiver)
}

fn watch_config_blocking(
    directory: &Path,
    file_name: Option<std::ffi::OsString>,
    async_sender: watch::Sender<u64>,
) -> Result<(), io::Error> {
    let (sender, receiver) = channel();
    let mut watcher = watcher(sender, Duration::from_secs(1)).map_err(map_notify_error)?;
    watcher
        .watch(directory, RecursiveMode::NonRecursive)
        .map_err(map_notify_error)?;
    let mut generation = 0u64;
    loop {
        let event = receiver.recv().map_err(broken_pipe)?;
        let touched = match &event {
            DebouncedEvent::Create(p) | DebouncedEvent::Write(p) | DebouncedEvent::Chmod(p) => {
                vec![p.clone()]
            }
            DebouncedEvent::Rename(from, to) => vec![from.clone(), to.clone()],
            DebouncedEvent::Remove(p) => {
                warn!("'{}' was deleted", p.display());
                vec![]
            }
            DebouncedEvent::Rescan => vec![directory.to_path_buf()],
            other => {
                trace!("{:?}", other);
                vec![]
            }
        };
        let relevant = touched.iter().any(|p| {
            p.as_path() == directory
                || p.file_name() == file_name.as_deref()
                // Kubernetes swaps the `..data` symlink when a ConfigMap changes
                || p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(".."))
        });
        if relevant {
            generation = generation.wrapping_add(1);
            async_sender.send(generation).map_err(broken_pipe)?;
        }
    }
}

async fn read_runtime_config<P>(
    path: P,
    acme_handler: Arc<AcmeHandler>,
    init_acme: bool,
) -> Result<RuntimeConfig, io::Error>
where
    P: AsRef<Path>,
{
    let config = TomlConfig::read(&path)?;
    let canonical_path = path.as_ref().canonicalize()?;
    let config_dir = canonical_path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Config path does not have a parrent",
        )
    })?;
    runtime_config_from_toml_config(config_dir, config, acme_handler, init_acme).await
}

async fn runtime_config_from_toml_config<P: AsRef<Path>>(
    config_dir: P,
    other: TomlConfig,
    acme_handler: Arc<AcmeHandler>,
    init_acme: bool,
) -> Result<RuntimeConfig, io::Error> {
    let http_address = other.http_address.parse().map_err(invalid_data)?;
    let https_address = other.https_address.parse().map_err(invalid_data)?;

    let backend_pools = other
        .backend_pools
        .into_iter()
        .map(|it| BackendPool::try_from(it).map(Arc::new))
        .collect::<Result<Vec<_>, _>>()
        .map_err(invalid_data)?;
    let metrics_address = other
        .prometheus_metrics_address
        .parse()
        .map_err(invalid_data)?;

    let mut certificates = HashMap::new();
    for (sni_name, certificate_config) in other.certificates {
        let server_name: ServerName = sni_name.as_str().try_into().map_err(invalid_data)?;
        if init_acme || !matches!(certificate_config, CertificateConfig::ACME { .. }) {
            let certificate =
                create_certified_key(&config_dir, certificate_config, &server_name, &acme_handler)
                    .await?;
            certificates.insert(server_name, Arc::new(certificate));
        }
    }

    let health_interval_config: HealthIntervalConfig = other.health_interval;
    let health_interval = Duration::from_secs(health_interval_config.check_every);

    Ok(RuntimeConfig {
        http_address,
        https_address,
        shared_data: SharedData {
            backend_pools,
            acme_handler,
        },
        certificates,
        health_interval,
        metrics_address,
    })
}

async fn create_certified_key<P: AsRef<Path>>(
    config_dir: P,
    config: CertificateConfig,
    sni_name: &ServerName,
    acme_handler: &AcmeHandler,
) -> Result<CertifiedKey, io::Error> {
    let certified_key = match config {
        CertificateConfig::Local {
            certificate_path,
            private_key_path,
        } => {
            let certificate_path = config_dir.as_ref().join(certificate_path);
            let private_key_path = config_dir.as_ref().join(private_key_path);
            load_certified_key(certificate_path, private_key_path)?
        }
        CertificateConfig::ACME {
            staging,
            email,
            persist_dir,
        } => {
            let persist_dir = config_dir.as_ref().join(persist_dir);
            // TODO refresh certificates once they expire?
            let sni_str = match sni_name {
                ServerName::DnsName(d) => d.as_ref(),
                ServerName::IpAddress(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "ACME does not support IP addresses",
                    ));
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Unsupported server name type",
                    ))
                }
            };
            let certificate = acme_handler
                .initiate_challenge(staging, &persist_dir, &email, sni_str)
                .await
                .map_err(other)?;

            certified_key_from_acme_certificate(certificate).map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!(
                        "Could not load ACME certificate for '{}' due to: {}",
                        sni_str, e
                    ),
                )
            })?
        }
    };
    Ok(certified_key)
}

fn map_notify_error(error: notify::Error) -> io::Error {
    // notify 6.x has different error structure
    io::Error::other(format!("File watch error: {:?}", error))
}

fn broken_pipe<E>(error: E) -> io::Error
where
    E: Into<Box<dyn Error + Send + Sync>>,
{
    io::Error::new(io::ErrorKind::BrokenPipe, error)
}

fn invalid_data<E>(error: E) -> io::Error
where
    E: Into<Box<dyn Error + Send + Sync>>,
{
    io::Error::new(io::ErrorKind::InvalidData, error)
}

#[allow(dead_code)]
fn not_found<E>(error: E) -> io::Error
where
    E: Into<Box<dyn Error + Send + Sync>>,
{
    io::Error::new(io::ErrorKind::NotFound, error)
}

fn other<E>(error: E) -> io::Error
where
    E: Into<Box<dyn Error + Send + Sync>>,
{
    io::Error::other(error)
}

pub struct RuntimeConfig {
    pub http_address: SocketAddr,
    pub https_address: SocketAddr,
    pub shared_data: SharedData,
    pub certificates: HashMap<ServerName, Arc<CertifiedKey>>,
    pub health_interval: Duration,
    /// Read once at startup, changing it requires a restart.
    pub metrics_address: SocketAddr,
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
}

/// Metrics are unauthenticated, so they are only reachable locally unless
/// `prometheus_metrics_address` says otherwise.
fn default_metrics_address() -> String {
    "127.0.0.1:9090".to_string()
}

// Dual Stack if /proc/sys/net/ipv6/bindv6only has default value 0
// rf https://man7.org/linux/man-pages/man7/ipv6.7.html
fn default_http_address() -> String {
    "[::]:80".to_string()
}

fn default_https_address() -> String {
    "[::]:443".to_string()
}

fn default_health_interval_config() -> HealthIntervalConfig {
    HealthIntervalConfig { check_every: 10 }
}

impl TomlConfig {
    fn read<P: AsRef<Path>>(toml_path: P) -> io::Result<TomlConfig> {
        let toml_str = fs::read_to_string(&toml_path).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "Error occurred when reading configuration file {}: {}",
                    toml_path.as_ref().display(),
                    e
                ),
            )
        })?;
        let config: TomlConfig = toml::from_str(&toml_str).map_err(|e| {
            let e = io::Error::other(format!("TOML parse error: {}", e));
            io::Error::new(
                e.kind(),
                format!(
                    "Error occurred when parsing configuration file {}: {}",
                    toml_path.as_ref().display(),
                    e
                ),
            )
        })?;
        config.print_warnings();
        Ok(config)
    }

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
          "backend pool at index {} does not contain any addresses. It will always result in bad gateway errors.",
          index
        );
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct BackendPoolConfig {
    matcher: String,
    addresses: Vec<String>,
    schemes: HashSet<Scheme>,
    client: Option<ClientConfig>,
    #[serde(default = "default_health_config")]
    health_config: HealthTomlConfig,
    strategy: LoadBalancingStrategyConfig,
    #[serde(default)]
    middlewares: Table,
}

fn default_health_config() -> HealthTomlConfig {
    HealthTomlConfig {
        slow_threshold: default_slow_threshold(),
        timeout: default_timeout(),
        path: default_path(),
    }
}

impl TryFrom<BackendPoolConfig> for BackendPool {
    type Error = String;

    fn try_from(other: BackendPoolConfig) -> Result<Self, Self::Error> {
        let matcher = BackendPoolMatcher::try_from(other.matcher)?;
        let addresses = other
            .addresses
            .into_iter()
            .map(|address| (address, ArcSwap::from_pointee(Healthiness::Healthy)))
            .collect();
        let health_toml_config = other.health_config;
        let strategy = other.strategy.into();
        let chain = other.middlewares.into();
        let schemes = other.schemes;

        let health_config = HealthConfig {
            slow_threshold: health_toml_config.slow_threshold,
            timeout: health_toml_config.timeout,
            path: health_toml_config.path,
        };

        let mut builder =
            BackendPoolBuilder::new(matcher, addresses, health_config, strategy, chain, schemes);
        if let Some(client) = other.client {
            if let Some(pool_idle_timeout) = client.pool_idle_timeout {
                builder.pool_idle_timeout(pool_idle_timeout);
            }

            if let Some(pool_max_idle_per_host) = client.pool_max_idle_per_host {
                builder.pool_max_idle_per_host(pool_max_idle_per_host);
            }
        }

        Ok(builder.build())
    }
}

#[derive(Debug, Deserialize)]
struct ClientConfig {
    pool_idle_timeout: Option<Duration>,
    pool_max_idle_per_host: Option<usize>,
}

#[derive(Debug, Deserialize, PartialEq)]
enum LoadBalancingStrategyConfig {
    StickyCookie {
        cookie_name: String,
        http_only: bool,
        secure: bool,
        same_site: StickyCookieSameSite,
        inner: Box<LoadBalancingStrategyConfig>,
    },
    Random,
    IPHash,
    LeastConnection,
    RoundRobin,
}

impl From<LoadBalancingStrategyConfig> for Box<dyn LoadBalancingStrategy> {
    fn from(other: LoadBalancingStrategyConfig) -> Self {
        match other {
            LoadBalancingStrategyConfig::StickyCookie {
                cookie_name,
                http_only,
                secure,
                same_site,
                inner,
            } => {
                let inner = (*inner).into();
                Box::new(StickyCookie::new(
                    cookie_name,
                    inner,
                    http_only,
                    secure,
                    same_site.into(),
                ))
            }
            LoadBalancingStrategyConfig::Random => Box::new(Random::new()),
            LoadBalancingStrategyConfig::IPHash => Box::new(IPHash::new()),
            LoadBalancingStrategyConfig::RoundRobin => Box::new(RoundRobin::new()),
            LoadBalancingStrategyConfig::LeastConnection => Box::new(LeastConnection::new()),
        }
    }
}

#[derive(Debug, Deserialize, PartialEq)]
pub enum StickyCookieSameSite {
    Strict,
    Lax,
    None,
}

impl From<StickyCookieSameSite> for cookie::SameSite {
    fn from(other: StickyCookieSameSite) -> Self {
        match other {
            StickyCookieSameSite::Strict => cookie::SameSite::Strict,
            StickyCookieSameSite::Lax => cookie::SameSite::Lax,
            StickyCookieSameSite::None => cookie::SameSite::None,
        }
    }
}

impl From<Table> for MiddlewareChain {
    fn from(other: Table) -> Self {
        let mut chain = MiddlewareChain::Empty;
        for (name, payload) in other.into_iter().rev() {
            match Box::<dyn Middleware>::try_from((name.clone(), payload)) {
                Ok(middleware) => {
                    chain = MiddlewareChain::Entry {
                        middleware,
                        chain: Box::new(chain),
                    };
                }
                Err(()) => warn!("Ignoring invalid or unknown middleware '{}'", name),
            }
        }
        chain
    }
}

impl TryFrom<(String, Value)> for Box<dyn Middleware> {
    type Error = ();

    fn try_from((name, payload): (String, Value)) -> Result<Self, Self::Error> {
        match (name.as_str(), payload) {
            ("RateLimiter", Value::Table(t)) => Ok(Box::new(RateLimiter::new(
                t.get("limit")
                    .and_then(Value::as_integer)
                    .and_then(|it| it.try_into().ok())
                    .ok_or(())?,
                t.get("window_sec")
                    .and_then(Value::as_integer)
                    .and_then(|it| it.try_into().ok())
                    .ok_or(())?,
            ))),
            ("Authentication", Value::Table(t)) => Ok(Box::new(Authentication {
                ldap_address: t
                    .get("ldap_address")
                    .and_then(Value::as_str)
                    .ok_or(())?
                    .to_string(),
                user_directory: t
                    .get("user_directory")
                    .and_then(Value::as_str)
                    .ok_or(())?
                    .to_string(),
                rdn_identifier: t
                    .get("rdn_identifier")
                    .and_then(Value::as_str)
                    .ok_or(())?
                    .to_string(),
                recursive: t.get("recursive").and_then(Value::as_bool).ok_or(())?,
            })),
            ("Compression", _) => Ok(Box::new(Compression)),
            ("HttpsRedirector", _) => Ok(Box::new(HttpsRedirector)),
            ("MaxBodySize", Value::Table(t)) => Ok(Box::new(MaxBodySize {
                limit: t.get("limit").and_then(Value::as_integer).ok_or(())?,
            })),
            ("CustomErrorPages", Value::Table(t)) => Ok(Box::new(CustomErrorPages::try_from(t)?)),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum CertificateConfig {
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

#[derive(Debug, Deserialize, Default)]
pub struct HealthIntervalConfig {
    pub check_every: u64,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Default)]
pub struct HealthTomlConfig {
    #[serde(default = "default_slow_threshold")]
    pub slow_threshold: i64,
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    #[serde(default = "default_path")]
    pub path: String,
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
