//! The data plane: RustStrom's [`ProxyHttp`] implementation.

use crate::{
    acme::CHALLENGE_PATH_PREFIX,
    config::BODY_MIN_RATE_GRACE,
    middleware::{authentication::AuthOutcome, compression, https_redirector, RequestStep},
    state::State,
    upstream::{health, Backend, CounterGuard, Pool, Scheme},
};
use async_trait::async_trait;
use bytes::Bytes;
use gethostname::gethostname;
use http::{
    header::{
        CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, HOST, LOCATION, SET_COOKIE,
        TRANSFER_ENCODING, VARY, WWW_AUTHENTICATE,
    },
    HeaderName, HeaderValue, Method,
};
use log::{debug, warn};
use pingora_core::{
    upstreams::peer::HttpPeer, Error, ErrorSource, ErrorType, ErrorType::HTTPStatus, Result,
};
use pingora_http::{RequestHeader, ResponseHeader};
use pingora_proxy::{FailToProxy, ProxyHttp, Session};
use prometheus::IntGauge;
use std::{
    net::IpAddr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, OnceLock,
    },
    time::{Duration, Instant},
};

/// How long an idle HTTP/1.1 client connection is kept open between requests.
const DOWNSTREAM_KEEPALIVE_SECS: u64 = 60;

static X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");
static X_REAL_IP: HeaderName = HeaderName::from_static("x-real-ip");
static X_FORWARDED_PORT: HeaderName = HeaderName::from_static("x-forwarded-port");
static X_FORWARDED_PROTO: HeaderName = HeaderName::from_static("x-forwarded-proto");
static X_FORWARDED_SERVER: HeaderName = HeaderName::from_static("x-forwarded-server");

pub struct Proxy {
    state: Arc<State>,
}

impl Proxy {
    pub fn new(state: Arc<State>) -> Proxy {
        Proxy { state }
    }
}

/// Counts a request in `active_http_connections` and the process in-flight counter.
struct ActiveRequest {
    gauge: IntGauge,
    counter: Arc<AtomicUsize>,
}

impl Drop for ActiveRequest {
    fn drop(&mut self) {
        self.gauge.dec();
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Per-request state.
pub struct Ctx {
    _active: ActiveRequest,
    start: Instant,
    scheme: Scheme,
    pool: Option<Arc<Pool>>,
    _pool_slot: Option<CounterGuard>,
    backend: Option<Arc<Backend>>,
    _backend_slot: Option<CounterGuard>,
    tried: Vec<Arc<Backend>>,
    tries: usize,
    upstream_start: Option<Instant>,
    sticky_cookie: Option<String>,
    body_limit: Option<u64>,
    body_received: u64,
    body_started: Option<Instant>,
    encoding: Option<compression::Encoding>,
    replacement_body: Option<Bytes>,
    replacement_sent: bool,
    status: u16,
    timed_out: bool,
}

impl Ctx {
    fn pool(&self) -> Result<Arc<Pool>> {
        self.pool
            .clone()
            .ok_or_else(|| Error::explain(ErrorType::InternalError, "request without a pool"))
    }
}

fn client_ip(session: &Session) -> Option<IpAddr> {
    session
        .client_addr()
        .and_then(|address| address.as_inet())
        .map(|address| address.ip().to_canonical())
}

fn scheme_of(session: &Session) -> Scheme {
    match session.digest().and_then(|d| d.ssl_digest.as_ref()) {
        Some(_) => Scheme::Https,
        None => Scheme::Http,
    }
}

/// The hostname never changes at runtime, so look it up only once.
fn forwarded_server_header() -> Option<&'static HeaderValue> {
    static HOSTNAME: OnceLock<Option<HeaderValue>> = OnceLock::new();
    HOSTNAME
        .get_or_init(|| {
            gethostname()
                .into_string()
                .ok()
                .and_then(|hostname| HeaderValue::from_str(&hostname).ok())
        })
        .as_ref()
}

/// X-Forwarded-For: client supplied values (all header lines) followed by the client IP.
fn forwarded_for(request: &RequestHeader, client_ip: &str) -> String {
    let existing: Vec<&str> = request
        .headers
        .get_all(&X_FORWARDED_FOR)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect();
    if existing.is_empty() {
        client_ip.to_owned()
    } else {
        format!("{}, {}", existing.join(", "), client_ip)
    }
}

/// Sets the forwarding headers exactly like v1.1: X-Forwarded-For is appended to, everything
/// else is overwritten so that clients can not spoof it.
pub fn set_forwarding_headers(
    request: &mut RequestHeader,
    client_ip: Option<IpAddr>,
    scheme: Scheme,
    local_port: Option<u16>,
) -> Result<()> {
    if let Some(ip) = client_ip {
        let ip = ip.to_string();
        let forwarded = forwarded_for(request, &ip);
        request.insert_header(X_FORWARDED_FOR.clone(), forwarded)?;
        request.insert_header(X_REAL_IP.clone(), ip)?;
    } else {
        request.remove_header(&X_REAL_IP);
    }
    let (default_port, proto) = match scheme {
        Scheme::Http => (80, "http"),
        Scheme::Https => (443, "https"),
    };
    // The port the client connected to (v1.1 always sent 80/443)
    let port = local_port.unwrap_or(default_port);
    request.insert_header(X_FORWARDED_PORT.clone(), port.to_string())?;
    request.insert_header(X_FORWARDED_PROTO.clone(), proto)?;
    match forwarded_server_header() {
        Some(hostname) => request.insert_header(X_FORWARDED_SERVER.clone(), hostname.clone())?,
        None => {
            request.remove_header(&X_FORWARDED_SERVER);
        }
    }
    // Hop-by-hop headers (RFC 9110 section 7.6.1). Pingora already removes them together with
    // the ones listed in `Connection`; Connection/Upgrade/Transfer-Encoding are left to Pingora
    // because it needs them for WebSocket upgrades and body framing.
    for name in [
        "keep-alive",
        "proxy-connection",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
    ] {
        request.remove_header(name);
    }
    Ok(())
}

/// Hop-by-hop headers of a backend response (RFC 9110 section 7.6.1) describe the backend
/// connection, not the client one: Pingora decides keep-alive and framing downstream (it adds
/// `Transfer-Encoding: chunked` itself when a body has no length). `101 Switching Protocols`
/// keeps `Connection`/`Upgrade`.
pub fn strip_response_hop_by_hop(response: &mut ResponseHeader) {
    let upgrade = response.status.as_u16() == 101;
    if !upgrade {
        let listed: Vec<HeaderName> = response
            .headers
            .get_all(http::header::CONNECTION)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(','))
            .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
            .collect();
        for name in &listed {
            // Never let a backend remove the framing of its own response this way
            if name != CONTENT_LENGTH {
                response.remove_header(name);
            }
        }
        response.remove_header(&http::header::CONNECTION);
        response.remove_header(&http::header::UPGRADE);
    }
    for name in [
        "keep-alive",
        "proxy-connection",
        "te",
        "trailer",
        "transfer-encoding",
    ] {
        response.remove_header(name);
    }
}

impl Proxy {
    /// Answers the request from the proxy itself (404, 429, 503, ...). Error pages of the pool
    /// replace the body when configured.
    async fn respond(
        &self,
        session: &mut Session,
        ctx: &mut Ctx,
        status: u16,
        body: &'static str,
        extra: Option<(HeaderName, String)>,
    ) -> Result<()> {
        let page = ctx
            .pool
            .as_ref()
            .and_then(|pool| pool.middlewares.error_pages.as_ref())
            .and_then(|pages| pages.get(status))
            .cloned();
        let (body, content_type) = match page {
            Some(page) => (page, Some("text/html; charset=utf-8")),
            None if body.is_empty() => (Bytes::new(), None),
            None => (Bytes::from_static(body.as_bytes()), Some("text/plain")),
        };
        let mut response = ResponseHeader::build(status, Some(4))?;
        response.insert_header(CONTENT_LENGTH, body.len().to_string())?;
        if let Some(content_type) = content_type {
            response.insert_header(CONTENT_TYPE, content_type)?;
        }
        if let Some((name, value)) = extra {
            response.insert_header(name, value)?;
        }
        // Do not read (drain) an unwanted request body just to keep the connection alive
        if !session.is_body_empty() && !session.is_body_done() {
            session.set_keepalive(None);
        }
        let head_only = session.req_header().method == Method::HEAD || body.is_empty();
        ctx.status = status;
        session
            .write_response_header(Box::new(response), head_only)
            .await?;
        if !head_only {
            session.write_response_body(Some(body), true).await?;
        }
        Ok(())
    }

    /// Request-phase middlewares in configuration order. Returns true if a response was sent.
    async fn run_request_steps(
        &self,
        session: &mut Session,
        ctx: &mut Ctx,
        pool: &Pool,
    ) -> Result<bool> {
        for step in &pool.middlewares.request_steps {
            match step {
                RequestStep::HttpsRedirector => {
                    if ctx.scheme == Scheme::Http {
                        match https_redirector::location(session.req_header()) {
                            Ok(location) => {
                                debug!("Redirecting to {}", location);
                                self.respond(session, ctx, 301, "", Some((LOCATION, location)))
                                    .await?;
                            }
                            Err(rejection) => {
                                self.respond(
                                    session,
                                    ctx,
                                    rejection.status,
                                    rejection.message,
                                    None,
                                )
                                .await?;
                            }
                        }
                        return Ok(true);
                    }
                }
                RequestStep::RateLimiter(limiter) => {
                    if let Some(ip) = client_ip(session) {
                        if !limiter.register_request(ip, Instant::now()) {
                            self.respond(session, ctx, 429, "429 - Too Many Requests", None)
                                .await?;
                            return Ok(true);
                        }
                    }
                }
                RequestStep::Authentication(authentication) => {
                    match authentication.authenticate(session.req_header()).await {
                        AuthOutcome::Allowed => {}
                        AuthOutcome::Denied => {
                            let challenge =
                                (WWW_AUTHENTICATE, "Basic realm=\"RustStrom\"".to_string());
                            self.respond(session, ctx, 401, "", Some(challenge)).await?;
                            return Ok(true);
                        }
                        AuthOutcome::Busy => {
                            let retry = (http::header::RETRY_AFTER, "1".to_string());
                            self.respond(session, ctx, 503, default_body(503), Some(retry))
                                .await?;
                            return Ok(true);
                        }
                    }
                }
                RequestStep::MaxBodySize(limit) => {
                    let declared = session
                        .req_header()
                        .headers
                        .get(CONTENT_LENGTH)
                        .map(|value| {
                            value
                                .to_str()
                                .ok()
                                .and_then(|s| s.trim().parse::<u64>().ok())
                        });
                    match declared {
                        Some(Some(length)) if length > *limit => {
                            self.respond(session, ctx, 413, "", None).await?;
                            return Ok(true);
                        }
                        Some(Some(_)) => {}
                        Some(None) => {
                            self.respond(session, ctx, 400, "Invalid Content-Length", None)
                                .await?;
                            return Ok(true);
                        }
                        None => {
                            // Chunked HTTP/1.1 and HTTP/2 bodies: counted while streaming
                            ctx.body_limit = Some(ctx.body_limit.map_or(*limit, |l| l.min(*limit)));
                        }
                    }
                }
            }
        }
        Ok(false)
    }
}

fn status_for_error(error: &Error) -> u16 {
    use ErrorType::*;
    match error.etype() {
        HTTPStatus(code) => *code,
        ConnectTimedout | ConnectRefused | ConnectNoRoute | ConnectError | TLSHandshakeFailure
        | TLSHandshakeTimedout | InvalidCert | HandshakeError | ConnectProxyFailure
        | SocketError | BindError => 502,
        ReadTimedout | WriteTimedout if *error.esource() == ErrorSource::Upstream => 504,
        ReadTimedout if *error.esource() == ErrorSource::Downstream => 408,
        _ => match error.esource() {
            ErrorSource::Upstream => 502,
            ErrorSource::Downstream => match error.etype() {
                // the client is gone, nothing to answer
                WriteError | ReadError | ConnectionClosed => 0,
                _ => 400,
            },
            ErrorSource::Internal | ErrorSource::Unset => 500,
        },
    }
}

/// Whether a request body that delivered `received` bytes in `elapsed` is below `min_rate`
/// bytes/s once the grace period is over. `min_rate = 0` disables the check.
fn body_too_slow(received: u64, elapsed: Duration, min_rate: u64) -> bool {
    if min_rate == 0 || elapsed < BODY_MIN_RATE_GRACE {
        return false;
    }
    let required = u128::from(min_rate).saturating_mul(elapsed.as_millis()) / 1000;
    u128::from(received) < required
}

fn default_body(status: u16) -> &'static str {
    match status {
        503 => "503 - Service Unavailable",
        504 => "504 - Gateway Timeout",
        _ => "",
    }
}

#[async_trait]
impl ProxyHttp for Proxy {
    type CTX = Ctx;

    fn new_ctx(&self) -> Ctx {
        let metrics = &self.state.metrics;
        metrics.http_requests_total.inc();
        metrics.active_http_connections.inc();
        self.state.active_requests.fetch_add(1, Ordering::Relaxed);
        Ctx {
            _active: ActiveRequest {
                gauge: metrics.active_http_connections.clone(),
                counter: self.state.active_requests.clone(),
            },
            start: Instant::now(),
            scheme: Scheme::Http,
            pool: None,
            _pool_slot: None,
            backend: None,
            _backend_slot: None,
            tried: Vec::new(),
            tries: 0,
            upstream_start: None,
            sticky_cookie: None,
            body_limit: None,
            body_received: 0,
            body_started: None,
            encoding: None,
            replacement_body: None,
            replacement_sent: false,
            status: 0,
            timed_out: false,
        }
    }

    async fn early_request_filter(&self, session: &mut Session, _ctx: &mut Ctx) -> Result<()> {
        // Pingora keeps idle HTTP/1.1 connections open forever by default
        if session.get_keepalive() == Some(0) {
            session.set_keepalive(Some(DOWNSTREAM_KEEPALIVE_SECS));
        }
        // Maximum pause between two reads of the request body (HTTP/1)
        let body_idle = self.state.config.load().client.body_idle_timeout;
        session.set_read_timeout(Some(body_idle));
        Ok(())
    }

    async fn request_filter(&self, session: &mut Session, ctx: &mut Ctx) -> Result<bool> {
        let config = self.state.config.load_full();
        ctx.scheme = scheme_of(session);

        if config.max_connections > 0
            && self.state.active_requests.load(Ordering::Relaxed) > config.max_connections
        {
            self.respond(session, ctx, 503, default_body(503), None)
                .await?;
            return Ok(true);
        }

        // RFC 9112 section 3.2: more than one Host is a 400. Routing and the backend must never
        // see different hosts.
        if session.req_header().headers.get_all(HOST).iter().count() > 1 {
            self.respond(session, ctx, 400, "Duplicate Host header", None)
                .await?;
            return Ok(true);
        }

        // ACME HTTP-01, before routing and before any middleware. Without ACME certificates in
        // the config the path is proxied, so backends can run their own ACME client.
        let path = session.req_header().uri.path();
        if path.starts_with(CHALLENGE_PATH_PREFIX) {
            if let Some(proof) = self.state.challenges.response_for_path(path) {
                let mut response = ResponseHeader::build(200, Some(2))?;
                response.insert_header(CONTENT_TYPE, "text/plain")?;
                response.insert_header(CONTENT_LENGTH, proof.len().to_string())?;
                ctx.status = 200;
                session
                    .write_response_header(Box::new(response), false)
                    .await?;
                session
                    .write_response_body(Some(Bytes::from(proof)), true)
                    .await?;
                return Ok(true);
            }
            if !config.acme.is_empty() {
                self.respond(session, ctx, 404, "404 - page not found", None)
                    .await?;
                return Ok(true);
            }
        }

        let Some(pool) = config
            .router
            .route(&config.pools, session.req_header(), ctx.scheme)
            .cloned()
        else {
            self.respond(session, ctx, 404, "404 - page not found", None)
                .await?;
            return Ok(true);
        };
        ctx.pool = Some(pool.clone());

        match CounterGuard::try_acquire(&pool.inflight, pool.upstream.max_connections) {
            Some(slot) => ctx._pool_slot = Some(slot),
            None => {
                self.respond(session, ctx, 503, default_body(503), None)
                    .await?;
                return Ok(true);
            }
        }

        if self.run_request_steps(session, ctx, &pool).await? {
            return Ok(true);
        }

        if pool.middlewares.compression && session.req_header().method != Method::HEAD {
            ctx.encoding = compression::preferred_encoding(&session.req_header().headers);
        }

        if pool.snapshot().backends.is_empty() {
            // No healthy (or slow) backend: 502 like v1.1, and every backend of the pool counts
            // the failed request (v1.1 counted nothing here)
            for backend in &pool.backends {
                backend.failures.inc();
            }
            self.respond(session, ctx, 502, default_body(502), None)
                .await?;
            return Ok(true);
        }
        Ok(false)
    }

    async fn request_body_filter(
        &self,
        _session: &mut Session,
        body: &mut Option<Bytes>,
        _end_of_stream: bool,
        ctx: &mut Ctx,
    ) -> Result<()> {
        let Some(chunk) = body.as_ref() else {
            return Ok(());
        };
        let started = *ctx.body_started.get_or_insert_with(Instant::now);
        ctx.body_received = ctx.body_received.saturating_add(chunk.len() as u64);
        if let Some(limit) = ctx.body_limit {
            if ctx.body_received > limit {
                return Err(Error::explain(
                    HTTPStatus(413),
                    "request body exceeds MaxBodySize",
                ));
            }
        }
        // Slow body (trickling one byte at a time defeats an idle timeout): after a grace period
        // the average rate must reach client_body_min_rate
        let min_rate = self.state.config.load().client.body_min_rate;
        if body_too_slow(ctx.body_received, started.elapsed(), min_rate) {
            return Err(Error::explain(
                HTTPStatus(408),
                "request body slower than client_body_min_rate",
            ));
        }
        Ok(())
    }

    async fn upstream_peer(&self, session: &mut Session, ctx: &mut Ctx) -> Result<Box<HttpPeer>> {
        let pool = ctx.pool()?;
        if let Some(previous) = ctx.backend.take() {
            ctx.tried.push(previous);
            self.state.metrics.upstream_retries_total.inc();
        }
        ctx._backend_slot = None;
        ctx.tries += 1;

        let selection = {
            let snapshot = pool.snapshot();
            pool.strategy.select(
                &snapshot,
                session.req_header(),
                client_ip(session),
                &ctx.tried,
            )
        };
        let Some(selection) = selection else {
            return Err(Error::explain(HTTPStatus(502), "no healthy backend"));
        };
        let backend = selection.backend;
        ctx.sticky_cookie = match (selection.set_sticky_cookie, pool.strategy.sticky()) {
            (true, Some(sticky)) => Some(sticky.set_cookie_header(&backend)),
            _ => None,
        };
        ctx._backend_slot = Some(backend.track_inflight());
        ctx.backend = Some(backend.clone());
        ctx.upstream_start = Some(Instant::now());

        let address = match backend.socket_addr() {
            Some(address) => address,
            None => {
                // Not resolved yet (hostname, health checks did not run yet)
                let lookup = tokio::net::lookup_host(backend.address.as_str());
                match tokio::time::timeout(pool.upstream.connect_timeout, lookup).await {
                    Ok(Ok(mut addresses)) => match addresses.next() {
                        Some(address) => {
                            backend.resolved.store(Some(Arc::new(address)));
                            address
                        }
                        None => return Err(self.unresolved(&backend, &pool, ctx)),
                    },
                    _ => return Err(self.unresolved(&backend, &pool, ctx)),
                }
            }
        };
        Ok(pool.peer(&backend, address))
    }

    async fn upstream_request_filter(
        &self,
        session: &mut Session,
        upstream_request: &mut RequestHeader,
        ctx: &mut Ctx,
    ) -> Result<()> {
        let local_port = session
            .server_addr()
            .and_then(|address| address.as_inet())
            .map(|address| address.port());
        set_forwarding_headers(upstream_request, client_ip(session), ctx.scheme, local_port)
    }

    async fn upstream_response_filter(
        &self,
        _session: &mut Session,
        upstream_response: &mut ResponseHeader,
        ctx: &mut Ctx,
    ) -> Result<()> {
        if upstream_response.status.is_informational() && upstream_response.status.as_u16() != 101 {
            return Ok(());
        }
        if let (Some(backend), Some(start)) = (&ctx.backend, ctx.upstream_start.take()) {
            backend.response_time.observe(start.elapsed().as_secs_f64());
        }
        strip_response_hop_by_hop(upstream_response);
        Ok(())
    }

    async fn response_filter(
        &self,
        session: &mut Session,
        response: &mut ResponseHeader,
        ctx: &mut Ctx,
    ) -> Result<()> {
        if response.status.is_informational() && response.status.as_u16() != 101 {
            return Ok(());
        }
        let status = response.status.as_u16();
        ctx.status = status;
        if let Some(cookie) = ctx.sticky_cookie.take() {
            response.append_header(SET_COOKIE, cookie)?;
        }
        let Some(pool) = ctx.pool.clone() else {
            return Ok(());
        };

        if let Some(page) = pool
            .middlewares
            .error_pages
            .as_ref()
            .and_then(|pages| pages.get(status))
        {
            let declared_empty = response
                .headers
                .get(CONTENT_LENGTH)
                .is_some_and(|value| value.as_bytes() == b"0");
            let has_body = !declared_empty
                && status != 204
                && status != 304
                && status != 101
                && session.req_header().method != Method::HEAD;
            // A response announced without body can not get one; it keeps its own (empty) body
            if has_body {
                response.remove_header(&CONTENT_ENCODING);
                response.remove_header(&TRANSFER_ENCODING);
                response.insert_header(CONTENT_LENGTH, page.len().to_string())?;
                response.insert_header(CONTENT_TYPE, "text/html; charset=utf-8")?;
                ctx.replacement_body = Some(page.clone());
            }
        }

        if pool.middlewares.compression && status != 101 {
            // The response varies by Accept-Encoding whether or not we compress it
            let already = response.headers.get_all(VARY).iter().any(|value| {
                value.to_str().is_ok_and(|v| {
                    v.split(',').any(|t| {
                        t.trim().eq_ignore_ascii_case("accept-encoding") || t.trim() == "*"
                    })
                })
            });
            if !already {
                response.append_header(VARY, "accept-encoding")?;
            }
            if let (Some(encoding), None) = (ctx.encoding, &ctx.replacement_body) {
                compression::enable_for_response(
                    &mut session.downstream_modules_ctx,
                    encoding,
                    response,
                );
            }
        }
        Ok(())
    }

    fn response_body_filter(
        &self,
        _session: &mut Session,
        body: &mut Option<Bytes>,
        _end_of_stream: bool,
        ctx: &mut Ctx,
    ) -> Result<Option<Duration>> {
        if let Some(page) = &ctx.replacement_body {
            if ctx.replacement_sent {
                *body = None;
            } else {
                *body = Some(page.clone());
                ctx.replacement_sent = true;
            }
        }
        Ok(None)
    }

    fn fail_to_connect(
        &self,
        _session: &mut Session,
        _peer: &HttpPeer,
        ctx: &mut Ctx,
        mut e: Box<Error>,
    ) -> Box<Error> {
        if let (Some(backend), Some(pool)) = (&ctx.backend, &ctx.pool) {
            backend.failures.inc();
            // Passive health: a refused or timed out connect counts like a failed check, so a
            // dead backend leaves the rotation before the next active check (without active
            // checks it is retried after health::PASSIVE_RETRY_AFTER)
            health::passive_failure(pool, backend, &e.to_string());
        }
        // Nothing was sent yet, so any method may go to another backend
        let budget = ctx
            .pool
            .as_ref()
            .is_some_and(|pool| ctx.tries <= pool.upstream.retries);
        e.set_retry(budget);
        e
    }

    fn error_while_proxy(
        &self,
        peer: &HttpPeer,
        session: &mut Session,
        e: Box<Error>,
        ctx: &mut Ctx,
        _client_reused: bool,
    ) -> Box<Error> {
        let mut e = e.more_context(format!("Peer: {}", peer));
        let upstream_failure = *e.esource() == ErrorSource::Upstream;
        if upstream_failure {
            if let Some(backend) = &ctx.backend {
                backend.failures.inc();
            }
        }
        let budget = ctx
            .pool
            .as_ref()
            .is_some_and(|pool| ctx.tries <= pool.upstream.retries);
        // After the request was (partly) sent only idempotent requests without body may be
        // repeated, and never once the client saw a response or after a timeout.
        let safe = upstream_failure
            && session.req_header().method.is_idempotent()
            && session.is_body_empty()
            && session.response_written().is_none()
            && !matches!(
                e.etype(),
                ErrorType::ReadTimedout | ErrorType::WriteTimedout | ErrorType::HTTPStatus(_)
            );
        e.set_retry(budget && safe);
        e
    }

    async fn fail_to_proxy(&self, session: &mut Session, e: &Error, ctx: &mut Ctx) -> FailToProxy {
        let code = status_for_error(e);
        if code == 504 {
            ctx.timed_out = true;
        }
        if session.response_written().is_none() && code > 0 {
            if let Err(write_error) = self
                .respond(session, ctx, code, default_body(code), None)
                .await
            {
                debug!(
                    "failed to send error response to downstream: {}",
                    write_error
                );
            }
        }
        FailToProxy {
            error_code: code,
            can_reuse_downstream: false,
        }
    }

    async fn logging(&self, session: &mut Session, e: Option<&Error>, ctx: &mut Ctx) {
        let metrics = &self.state.metrics;
        let status = session
            .response_written()
            .map(|r| r.status.as_u16())
            .unwrap_or(ctx.status);
        metrics
            .http_request_duration
            .observe(ctx.start.elapsed().as_secs_f64());
        if status > 0 {
            if let Some(counter) = metrics.status_code(status) {
                counter.inc();
            }
            if status >= 500 {
                metrics.http_errors_total.inc();
            }
        }
        if ctx.timed_out {
            metrics.http_timeouts_total.inc();
        }
        debug!(
            "{} {} {} -> {} pool={} backend={} tries={} {:?}{}",
            client_ip(session)
                .map(|ip| ip.to_string())
                .unwrap_or_default(),
            session.req_header().method,
            session.req_header().uri,
            status,
            ctx.pool.as_ref().map(|p| p.name.as_str()).unwrap_or("-"),
            ctx.backend
                .as_ref()
                .map(|b| b.address.as_str())
                .unwrap_or("-"),
            ctx.tries,
            ctx.start.elapsed(),
            e.map(|e| format!(" error={}", e)).unwrap_or_default(),
        );
    }
}

impl Proxy {
    fn unresolved(&self, backend: &Backend, pool: &Pool, ctx: &Ctx) -> Box<Error> {
        warn!("Could not resolve backend address '{}'", backend.address);
        backend.failures.inc();
        let mut e = Error::explain(
            ErrorType::ConnectError,
            format!("could not resolve '{}'", backend.address),
        );
        e.set_retry(ctx.tries <= pool.upstream.retries);
        e.into_up()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(headers: &[(&str, &str)]) -> RequestHeader {
        let mut req = RequestHeader::build("GET", b"/path?q=1", None).unwrap();
        for (name, value) in headers {
            req.append_header(name.to_string(), *value).unwrap();
        }
        req
    }

    #[test]
    fn spoofed_forwarding_headers_are_overwritten() {
        let mut req = request(&[
            ("host", "example.com"),
            ("x-real-ip", "127.0.0.1"),
            ("x-forwarded-proto", "https"),
            ("x-forwarded-port", "443"),
            ("x-forwarded-for", "1.2.3.4"),
        ]);
        set_forwarding_headers(
            &mut req,
            Some("203.0.113.7".parse().unwrap()),
            Scheme::Http,
            None,
        )
        .unwrap();
        let headers = &req.headers;
        assert_eq!(headers.get_all("x-real-ip").iter().count(), 1);
        assert_eq!(headers["x-real-ip"], "203.0.113.7");
        assert_eq!(headers.get_all("x-forwarded-proto").iter().count(), 1);
        assert_eq!(headers["x-forwarded-proto"], "http");
        assert_eq!(headers["x-forwarded-port"], "80");
        assert_eq!(headers["x-forwarded-for"], "1.2.3.4, 203.0.113.7");
        assert_eq!(headers["host"], "example.com");
        assert!(headers.contains_key("x-forwarded-server"));
    }

    #[test]
    fn forwarded_for_joins_multiple_lines_and_https() {
        let mut req = request(&[
            ("x-forwarded-for", "1.1.1.1"),
            ("x-forwarded-for", "2.2.2.2"),
        ]);
        set_forwarding_headers(
            &mut req,
            Some("::ffff:10.0.0.1".parse::<IpAddr>().unwrap().to_canonical()),
            Scheme::Https,
            Some(8443),
        )
        .unwrap();
        assert_eq!(req.headers["x-forwarded-for"], "1.1.1.1, 2.2.2.2, 10.0.0.1");
        assert_eq!(req.headers["x-forwarded-proto"], "https");
        assert_eq!(req.headers["x-forwarded-port"], "8443");
    }

    #[test]
    fn hop_by_hop_headers_are_removed() {
        let mut req = request(&[
            ("keep-alive", "timeout=5"),
            ("te", "trailers"),
            ("proxy-authorization", "Basic x"),
            ("x-kept", "1"),
        ]);
        set_forwarding_headers(&mut req, None, Scheme::Http, None).unwrap();
        for name in ["keep-alive", "te", "proxy-authorization", "x-real-ip"] {
            assert!(!req.headers.contains_key(name), "{} was forwarded", name);
        }
        assert!(req.headers.contains_key("x-kept"));
    }

    #[test]
    fn response_hop_by_hop_headers_are_removed() {
        let mut response = ResponseHeader::build(200, None).unwrap();
        for (name, value) in [
            ("connection", "keep-alive, x-internal"),
            ("keep-alive", "timeout=5"),
            ("x-internal", "1"),
            ("transfer-encoding", "chunked"),
            ("upgrade", "h2c"),
            ("x-kept", "1"),
        ] {
            response.insert_header(name.to_string(), value).unwrap();
        }
        strip_response_hop_by_hop(&mut response);
        for name in [
            "connection",
            "keep-alive",
            "x-internal",
            "transfer-encoding",
            "upgrade",
        ] {
            assert!(!response.headers.contains_key(name), "{} kept", name);
        }
        assert!(response.headers.contains_key("x-kept"));

        let mut switching = ResponseHeader::build(101, None).unwrap();
        switching.insert_header("connection", "Upgrade").unwrap();
        switching.insert_header("upgrade", "websocket").unwrap();
        strip_response_hop_by_hop(&mut switching);
        assert_eq!(switching.headers["upgrade"], "websocket");
        assert_eq!(switching.headers["connection"], "Upgrade");
    }

    #[test]
    fn slow_body_detection() {
        let s = Duration::from_secs;
        // grace period: anything goes
        assert!(!body_too_slow(0, s(9), 500));
        // 12 bytes in 12 s is a trickle
        assert!(body_too_slow(12, s(12), 500));
        // 1 MB in 20 s is fine
        assert!(!body_too_slow(1_000_000, s(20), 500));
        // disabled
        assert!(!body_too_slow(0, s(3600), 0));
    }

    #[test]
    fn error_mapping() {
        let mut connect = Error::new(ErrorType::ConnectRefused);
        connect.as_up();
        assert_eq!(status_for_error(&connect), 502);
        let mut timeout = Error::new(ErrorType::ReadTimedout);
        timeout.as_up();
        assert_eq!(status_for_error(&timeout), 504);
        assert_eq!(status_for_error(&Error::new(HTTPStatus(413))), 413);
        let mut gone = Error::new(ErrorType::ConnectionClosed);
        gone.as_down();
        assert_eq!(status_for_error(&gone), 0);
    }
}
