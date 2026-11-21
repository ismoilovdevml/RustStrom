//! Active health checks with rise/fall hysteresis and the v1.1 `slow_threshold` semantics: a
//! backend that answers correctly but slower than the threshold is `Slow` and only receives
//! traffic when no backend of its pool is `Healthy`.

use super::{Backend, Health, HealthCheck, HealthKind, Pool, UpstreamSettings};
use crate::state::State;
use log::{debug, info, warn};
use pingora_core::{server::ShutdownWatch, tls::ssl};
use std::{
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    task::JoinSet,
    time::{sleep, timeout},
};

/// Result of one probe: the response time or why it failed.
type Outcome = Result<u64, String>;

pub async fn run(state: Arc<State>, mut shutdown: ShutdownWatch) {
    loop {
        if *shutdown.borrow() {
            return;
        }
        let config = state.config.load_full();
        let interval = config.health_interval;
        let started = Instant::now();
        if interval.is_zero() {
            revive_passively_failed(&config.pools);
        } else {
            check_all(&config.pools).await;
        }
        let wait = if interval.is_zero() {
            Duration::from_secs(1)
        } else {
            interval.saturating_sub(started.elapsed())
        };
        tokio::select! {
            _ = sleep(wait) => {}
            _ = shutdown.changed() => return,
        }
    }
}

pub async fn check_all(pools: &[Arc<Pool>]) {
    let mut checks = JoinSet::new();
    for (pool_index, pool) in pools.iter().enumerate() {
        for backend in &pool.backends {
            let backend = backend.clone();
            let check = pool.health_check.clone();
            let upstream = pool.upstream.clone();
            checks.spawn(async move {
                let outcome = probe(&backend, &check, &upstream).await;
                (pool_index, backend, check, outcome)
            });
        }
    }
    let mut changed = vec![false; pools.len()];
    while let Some(result) = checks.join_next().await {
        match result {
            Ok((pool_index, backend, check, outcome)) => {
                if apply(&backend, &check, outcome) {
                    if let Some(flag) = changed.get_mut(pool_index) {
                        *flag = true;
                    }
                }
            }
            Err(e) => warn!("Health check task failed: {}", e),
        }
    }
    for (pool, changed) in pools.iter().zip(changed) {
        if changed {
            pool.rebuild_snapshot();
        }
    }
}

/// A failed connect seen by the data plane counts like a failed check (passive health). Only
/// failures are observed passively; recovery (`rise`) needs active checks.
pub fn passive_failure(pool: &Pool, backend: &Backend, reason: &str) {
    if apply(
        backend,
        &pool.health_check,
        Err(format!("passive: {}", reason)),
    ) {
        pool.rebuild_snapshot();
    }
}

/// Feeds one probe result into the backend state machine. Returns whether the state changed.
pub fn apply(backend: &Backend, check: &HealthCheck, outcome: Outcome) -> bool {
    use std::sync::atomic::Ordering::Relaxed;
    let state = &backend.health;
    let before = state.get();
    let after = match &outcome {
        Ok(response_ms) => {
            state.fail_streak.store(0, Relaxed);
            let ok_streak = state.ok_streak.fetch_add(1, Relaxed).saturating_add(1);
            state.last_response_ms.store(*response_ms, Relaxed);
            let slow = i64::try_from(*response_ms).unwrap_or(i64::MAX) > check.slow_threshold_ms;
            let observed = if slow { Health::Slow } else { Health::Healthy };
            if before == Health::Down && ok_streak < check.rise {
                Health::Down
            } else {
                observed
            }
        }
        Err(_) => {
            state.ok_streak.store(0, Relaxed);
            let fail_streak = state.fail_streak.fetch_add(1, Relaxed).saturating_add(1);
            if fail_streak >= check.fall {
                Health::Down
            } else {
                before
            }
        }
    };
    // compare-and-swap: with concurrent passive failures only one caller wins the transition
    // (and rebuilds the snapshot)
    if after == before || !backend.transition_health(before, after) {
        if let Err(reason) = &outcome {
            debug!("health check of {} failed: {}", backend.address, reason);
        }
        return false;
    }
    match (&outcome, after) {
        (Ok(ms), Health::Slow) => info!("new healthiness for {}: Slow {}", backend.address, ms),
        (Err(reason), _) => info!(
            "new healthiness for {}: Unresponsive ({})",
            backend.address, reason
        ),
        (_, health) => info!("new healthiness for {}: {:?}", backend.address, health),
    }
    true
}

/// Without active checks nothing would bring a passively failed backend back, so it gets
/// traffic again after this long (the next failed connect takes it out again).
pub const PASSIVE_RETRY_AFTER: Duration = Duration::from_secs(10);

/// Used when active checks are disabled (`check_every = 0`).
fn revive_passively_failed(pools: &[Arc<Pool>]) {
    for pool in pools {
        let mut changed = false;
        for backend in &pool.backends {
            if backend.health.down_for() >= PASSIVE_RETRY_AFTER
                && backend.transition_health(Health::Down, Health::Healthy)
            {
                info!(
                    "{} is retried after {:?} (active health checks are disabled)",
                    backend.address, PASSIVE_RETRY_AFTER
                );
                changed = true;
            }
        }
        if changed {
            pool.rebuild_snapshot();
        }
    }
}

async fn probe(backend: &Backend, check: &HealthCheck, upstream: &UpstreamSettings) -> Outcome {
    let address = resolve(backend, check.timeout).await?;
    // Bound the whole exchange, not only the individual operations (same as v1.1)
    let total = check
        .timeout
        .saturating_mul(2)
        .max(Duration::from_millis(1));
    let started = Instant::now();
    match timeout(total, exchange(backend, address, check, upstream)).await {
        Ok(Ok(())) => Ok(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(format!("timed out after {:?}", total)),
    }
}

/// Resolves hostnames on every round so DNS changes (containers, k8s services) are picked up.
async fn resolve(backend: &Backend, limit: Duration) -> Result<SocketAddr, String> {
    if let Ok(address) = backend.address.parse::<SocketAddr>() {
        return Ok(address);
    }
    let lookup = tokio::net::lookup_host(backend.address.as_str());
    match timeout(limit.max(Duration::from_millis(500)), lookup).await {
        Ok(Ok(mut addresses)) => match addresses.next() {
            Some(address) => {
                if backend.socket_addr() != Some(address) {
                    backend.resolved.store(Some(Arc::new(address)));
                }
                Ok(address)
            }
            None => Err("DNS returned no address".into()),
        },
        Ok(Err(e)) => Err(format!("DNS lookup failed: {}", e)),
        Err(_) => Err("DNS lookup timed out".into()),
    }
}

async fn exchange(
    backend: &Backend,
    address: SocketAddr,
    check: &HealthCheck,
    upstream: &UpstreamSettings,
) -> Result<(), String> {
    let tcp = timeout(check.timeout, TcpStream::connect(address))
        .await
        .map_err(|_| "connect timed out".to_string())?
        .map_err(|e| format!("connect failed: {}", e))?;
    let _ = tcp.set_nodelay(true);
    let host = if upstream.tls && !upstream.sni.is_empty() {
        upstream.sni.clone()
    } else {
        backend.address.clone()
    };
    if upstream.tls {
        let sni = if upstream.sni.is_empty() {
            super::host_part(&backend.address).to_string()
        } else {
            upstream.sni.clone()
        };
        let stream = tls_connect(tcp, &sni, upstream.verify_cert).await?;
        if check.kind == HealthKind::Tcp {
            return Ok(());
        }
        http_check(stream, &host, check).await
    } else {
        if check.kind == HealthKind::Tcp {
            return Ok(());
        }
        http_check(tcp, &host, check).await
    }
}

async fn tls_connect(
    tcp: TcpStream,
    sni: &str,
    verify: bool,
) -> Result<pingora_core::tls::tokio_ssl::SslStream<TcpStream>, String> {
    let connector = tls_connector(verify)?;
    let mut config = connector.configure().map_err(|e| e.to_string())?;
    config.set_verify_hostname(verify);
    if !verify {
        config.set_verify(ssl::SslVerifyMode::NONE);
    }
    let ssl = config.into_ssl(sni).map_err(|e| e.to_string())?;
    let mut stream =
        pingora_core::tls::tokio_ssl::SslStream::new(ssl, tcp).map_err(|e| e.to_string())?;
    Pin::new(&mut stream)
        .connect()
        .await
        .map_err(|e| format!("TLS handshake failed: {}", e))?;
    Ok(stream)
}

fn tls_connector(verify: bool) -> Result<&'static ssl::SslConnector, String> {
    static VERIFYING: OnceLock<Option<ssl::SslConnector>> = OnceLock::new();
    static NON_VERIFYING: OnceLock<Option<ssl::SslConnector>> = OnceLock::new();
    let cell = if verify { &VERIFYING } else { &NON_VERIFYING };
    cell.get_or_init(|| {
        let mut builder = ssl::SslConnector::builder(ssl::SslMethod::tls_client()).ok()?;
        if !verify {
            builder.set_verify(ssl::SslVerifyMode::NONE);
        }
        Some(builder.build())
    })
    .as_ref()
    .ok_or_else(|| "could not create TLS connector".to_string())
}

async fn http_check<S>(mut stream: S, host: &str, check: &HealthCheck) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if !check.path.starts_with('/')
        || check
            .path
            .bytes()
            .any(|b| b.is_ascii_control() || b == b' ')
    {
        return Err(format!("invalid health check path '{}'", check.path));
    }
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: RustStrom-HealthCheck\r\nAccept: */*\r\nConnection: close\r\n\r\n",
        check.path, host
    );
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| format!("write failed: {}", e))?;
    let mut buffer = [0u8; 256];
    let mut filled = 0;
    while filled < buffer.len() {
        let target = buffer.get_mut(filled..).ok_or("buffer")?;
        let read = stream
            .read(target)
            .await
            .map_err(|e| format!("read failed: {}", e))?;
        if read == 0 {
            break;
        }
        filled += read;
        if buffer
            .get(..filled)
            .is_some_and(|b| b.windows(2).any(|w| w == b"\r\n"))
        {
            break;
        }
    }
    let status = parse_status(buffer.get(..filled).unwrap_or_default())
        .ok_or_else(|| "no valid HTTP status line".to_string())?;
    let (low, high) = check.expected_status;
    if (low..=high).contains(&status) {
        Ok(())
    } else {
        Err(format!("status {}", status))
    }
}

/// Status code of `HTTP/1.x NNN ...`.
fn parse_status(head: &[u8]) -> Option<u16> {
    let line_end = head
        .windows(2)
        .position(|w| w == b"\r\n")
        .unwrap_or(head.len());
    let line = std::str::from_utf8(head.get(..line_end)?).ok()?;
    let mut parts = line.split(' ');
    let version = parts.next()?;
    if !version.starts_with("HTTP/1.") {
        return None;
    }
    let code = parts.next()?;
    if code.len() != 3 {
        return None;
    }
    code.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upstream::test_backend;

    fn check() -> HealthCheck {
        HealthCheck {
            slow_threshold_ms: 300,
            timeout: Duration::from_millis(500),
            path: "/".into(),
            rise: 2,
            fall: 3,
            kind: HealthKind::Http,
            expected_status: (200, 399),
        }
    }

    #[test]
    fn status_line_parsing() {
        assert_eq!(parse_status(b"HTTP/1.1 200 OK\r\n"), Some(200));
        assert_eq!(parse_status(b"HTTP/1.0 503 x\r\nfoo"), Some(503));
        assert_eq!(parse_status(b"HTTP/1.1 20"), None);
        assert_eq!(parse_status(b"SSH-2.0-OpenSSH\r\n"), None);
        assert_eq!(parse_status(b""), None);
    }

    #[test]
    fn fall_then_rise() {
        let backend = test_backend("127.0.0.1:1");
        let check = check();
        assert_eq!(backend.health.get(), Health::Healthy);
        assert!(!apply(&backend, &check, Err("x".into())));
        assert!(!apply(&backend, &check, Err("x".into())));
        assert!(apply(&backend, &check, Err("x".into())));
        assert_eq!(backend.health.get(), Health::Down);
        assert!(!apply(&backend, &check, Ok(10)));
        assert!(apply(&backend, &check, Ok(10)));
        assert_eq!(backend.health.get(), Health::Healthy);
    }

    #[test]
    fn concurrent_failures_transition_once() {
        let backend = test_backend("127.0.0.1:1");
        let mut check = check();
        check.fall = 1;
        let wins: usize = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..16)
                .map(|_| scope.spawn(|| apply(&backend, &check, Err("x".into())) as usize))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).sum()
        });
        assert_eq!(wins, 1);
        assert_eq!(backend.health.get(), Health::Down);
    }

    #[test]
    fn passive_down_is_revived_without_active_checks() {
        let backend = test_backend("127.0.0.1:1");
        backend.set_health(Health::Down);
        assert!(backend.health.down_for() < PASSIVE_RETRY_AFTER);
        assert!(backend.transition_health(Health::Down, Health::Healthy));
        assert!(!backend.transition_health(Health::Down, Health::Healthy));
        assert_eq!(backend.health.down_for(), Duration::ZERO);
    }

    #[test]
    fn slow_threshold() {
        let backend = test_backend("127.0.0.1:1");
        let check = check();
        assert!(apply(&backend, &check, Ok(301)));
        assert_eq!(backend.health.get(), Health::Slow);
        assert!(apply(&backend, &check, Ok(5)));
        assert_eq!(backend.health.get(), Health::Healthy);
        // an intermittent failure below `fall` keeps the state
        assert!(!apply(&backend, &check, Err("x".into())));
        assert!(!apply(&backend, &check, Ok(5)));
        assert_eq!(backend.health.get(), Health::Healthy);
    }

    #[tokio::test]
    async fn probes_a_real_listener() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 512];
                let _ = socket.read(&mut buf).await;
                let _ = socket
                    .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                    .await;
            }
        });
        let backend = test_backend(&address);
        let outcome = probe(&backend, &check(), &UpstreamSettings::default()).await;
        assert!(outcome.is_ok(), "{:?}", outcome);

        let mut wrong_status = check();
        wrong_status.expected_status = (200, 200);
        let outcome = probe(&backend, &wrong_status, &UpstreamSettings::default()).await;
        assert_eq!(outcome, Err("status 204".to_string()));

        let closed = test_backend("127.0.0.1:1");
        assert!(probe(&closed, &check(), &UpstreamSettings::default())
            .await
            .is_err());
    }
}
