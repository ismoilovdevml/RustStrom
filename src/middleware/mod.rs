use crate::{
    error_response::handle_bad_gateway, http_client::StrategyNotifyHttpConnector, server::Scheme,
    utils::unwrap_result,
};
use async_trait::async_trait;
use gethostname::gethostname;
use hyper::{
    header::{
        HeaderName, HeaderValue, CONNECTION, HOST, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION, TE,
        TRAILER, TRANSFER_ENCODING, UPGRADE,
    },
    Body, Client, HeaderMap, Request, Response, Uri, Version,
};
use std::{net::SocketAddr, sync::OnceLock};

pub mod authentication;
pub mod compression;
pub mod custom_error_pages;
pub mod https_redirector;
pub mod maxbodysize;
pub mod rate_limiter;

/// A trait for implementing middlewares, see
/// [`forward_request`](Middleware::forward_request) for more details.
#[async_trait]
pub trait Middleware: Send + Sync + std::fmt::Debug {
    /// Forward the `request` via [`MiddlewareChain::forward_request`],
    /// potentially modifying the request or response along the way. The request
    /// can also be answered early (from cache or in cases like missing
    /// authentication) in which case the supplied `chain` is not used.
    ///
    /// The default implementation calls [`modify_request`][], forwards the
    /// modified request via [`MiddlewareChain::forward_request`] and then calls
    /// [`modify_response`][] on the received response. Simple implementations
    /// should prefer to just implement [`modify_request`][] or
    /// [`modify_response`][] if possible.
    ///
    /// [`modify_request`]: Middleware::modify_request
    /// [`modify_response`]: Middleware::modify_response
    async fn forward_request(
        &self,
        request: Request<Body>,
        chain: &MiddlewareChain,
        context: &Context<'_>,
    ) -> Response<Body> {
        match self.modify_request(request, context).await {
            Ok(request) => {
                let response = chain.forward_request(request, context).await;
                self.modify_response(response, context).await
            }
            Err(response) => response,
        }
    }

    /// Transforms the `request` before it is forwarded to later middlewares and
    /// then the backend server or returns an early response (from cache or in
    /// cases like missing authentication).
    ///
    /// If an early response is returned then
    /// [`modify_response`](Middleware::modify_response) of this middleware is not
    /// called.
    ///
    /// This method serves only as a convenience to implement simple middlewares.
    /// If [`forward_request`](Middleware::forward_request) is implemented, this
    /// function is never called.
    ///
    /// The default implementation just returns the original `request`.
    async fn modify_request(
        &self,
        request: Request<Body>,
        _context: &Context<'_>,
    ) -> Result<Request<Body>, Response<Body>> {
        Ok(request)
    }

    /// Transforms the response before it is returned to the calling client.
    ///
    /// This function is not called for early responses of the same middleware,
    /// but it is called for early responses of middlewares further down the
    /// [`MiddlewareChain`].
    ///
    /// This method serves only as a convenience to implement simple middlewares.
    /// If [`forward_request`](Middleware::forward_request) is implemented, this
    /// function is never called.
    ///
    /// The default implementation just returns the original `response`.
    async fn modify_response(
        &self,
        response: Response<Body>,
        _context: &Context<'_>,
    ) -> Response<Body> {
        response
    }
}

pub struct Context<'l> {
    pub client_scheme: &'l Scheme,
    /// Port of the listener the client connected to.
    pub listener_port: u16,
    pub client_address: &'l SocketAddr,
    pub backend_uri: Uri,
    pub client: &'l Client<StrategyNotifyHttpConnector, Body>,
}

/// A singly linked list of [`Middleware`]s.
///
/// This list is used to call all middlewares of a
/// [`BackendPool`](crate::server::BackendPool) in order and forward the request
/// to the backend server once the chain is empty. This is implemented in
/// [`forward_request`](MiddlewareChain::forward_request).
#[derive(Debug)]
pub enum MiddlewareChain {
    Empty,
    Entry {
        middleware: Box<dyn Middleware>,
        chain: Box<MiddlewareChain>,
    },
}

impl MiddlewareChain {
    /// If this chain is not empty this function calls
    /// [`forward_request`](Middleware::forward_request) on the first middleware,
    /// passing it the tail of this chain as an argument to be called recursively.
    ///
    /// Once this chain is empty this function does the final request
    /// transformation, setting all appropriate forwarding headers (like
    /// `x-forwarded-for`) and sends it to the backend server, returning the
    /// response.
    pub async fn forward_request(
        &self,
        request: Request<Body>,
        context: &Context<'_>,
    ) -> Response<Body> {
        match self {
            MiddlewareChain::Entry { middleware, chain } => {
                middleware.forward_request(request, chain, context).await
            }
            MiddlewareChain::Empty => {
                let backend_request = backend_request(request, context);
                unwrap_result(
                    context
                        .client
                        .request(backend_request)
                        .await
                        .map_err(handle_bad_gateway),
                )
            }
        }
    }
}

/// Hop-by-hop headers (RFC 9110, section 7.6.1) must not be forwarded.
const HOP_BY_HOP_HEADERS: [HeaderName; 8] = [
    CONNECTION,
    HeaderName::from_static("keep-alive"),
    HeaderName::from_static("proxy-connection"),
    PROXY_AUTHENTICATE,
    PROXY_AUTHORIZATION,
    TE,
    TRAILER,
    TRANSFER_ENCODING,
];

static X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");
static X_REAL_IP: HeaderName = HeaderName::from_static("x-real-ip");
static X_FORWARDED_PORT: HeaderName = HeaderName::from_static("x-forwarded-port");
static X_FORWARDED_PROTO: HeaderName = HeaderName::from_static("x-forwarded-proto");
static X_FORWARDED_SERVER: HeaderName = HeaderName::from_static("x-forwarded-server");

/// The hostname never changes at runtime, so look it up only once instead of
/// doing a syscall per request.
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

fn backend_request(request: Request<Body>, context: &Context) -> Request<Body> {
    let (mut parts, body) = request.into_parts();

    // HTTP/2 requests carry the host only in the :authority pseudo header
    if !parts.headers.contains_key(HOST) {
        if let Some(authority) = parts.uri.authority() {
            if let Ok(host) = HeaderValue::from_str(authority.as_str()) {
                parts.headers.insert(HOST, host);
            }
        }
    }

    remove_hop_by_hop_headers(&mut parts.headers);
    // Backends are spoken to over HTTP/1.1; hyper rejects HTTP/2 requests on HTTP/1 connections
    parts.version = Version::HTTP_11;
    parts.uri = context.backend_uri.clone();

    let client_ip = context.client_address.ip().to_string();
    let forwarded_for = forwarded_for_header(parts.headers.get(&X_FORWARDED_FOR), &client_ip);
    let headers = &mut parts.headers;
    if let Ok(value) = HeaderValue::from_str(&forwarded_for) {
        headers.insert(X_FORWARDED_FOR.clone(), value);
    }
    // insert() replaces client supplied values, so these headers can not be spoofed
    if let Ok(value) = HeaderValue::from_str(&client_ip) {
        headers.insert(X_REAL_IP.clone(), value);
    }
    let proto = match context.client_scheme {
        Scheme::HTTP => "http",
        Scheme::HTTPS => "https",
    };
    headers.insert(
        X_FORWARDED_PORT.clone(),
        HeaderValue::from(context.listener_port),
    );
    headers.insert(X_FORWARDED_PROTO.clone(), HeaderValue::from_static(proto));
    match forwarded_server_header() {
        Some(hostname) => {
            headers.insert(X_FORWARDED_SERVER.clone(), hostname.clone());
        }
        None => {
            headers.remove(&X_FORWARDED_SERVER);
        }
    }

    Request::from_parts(parts, body)
}

fn remove_hop_by_hop_headers(headers: &mut HeaderMap) {
    // Headers listed in `Connection` are hop-by-hop as well
    let listed: Vec<HeaderName> = headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
        .collect();
    for name in listed.iter().chain(HOP_BY_HOP_HEADERS.iter()) {
        headers.remove(name);
    }
    // Upgrades (e.g. WebSocket) are not proxied yet, so do not ask the backend for one
    headers.remove(UPGRADE);
}

// According to https://docs.oracle.com/en-us/iaas/Content/Balance/Reference/httpheaders.htm
fn forwarded_for_header(existing_forwarded_for: Option<&HeaderValue>, client_ip: &str) -> String {
    match existing_forwarded_for.and_then(|value| value.to_str().ok()) {
        Some(existing) if !existing.trim().is_empty() => format!("{}, {}", existing, client_ip),
        _ => client_ip.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{algorithms::round_robin::RoundRobin, http_client::StrategyNotifyHttpConnector};
    use std::sync::Arc;

    fn forward(request: Request<Body>, scheme: Scheme) -> Request<Body> {
        let client = Client::builder().build(StrategyNotifyHttpConnector::new(Arc::new(Box::new(
            RoundRobin::new(),
        ))));
        let client_address: SocketAddr = "203.0.113.7:51234".parse().unwrap();
        let context = Context {
            client_scheme: &scheme,
            listener_port: 8443,
            client_address: &client_address,
            backend_uri: "http://10.0.0.1:8080/path?q=1".parse().unwrap(),
            client: &client,
        };
        backend_request(request, &context)
    }

    #[test]
    fn spoofed_forwarding_headers_are_overwritten() {
        let request = Request::builder()
            .uri("/path?q=1")
            .header("host", "example.com")
            .header("x-real-ip", "127.0.0.1")
            .header("x-forwarded-proto", "https")
            .header("x-forwarded-for", "1.2.3.4")
            .body(Body::empty())
            .unwrap();
        let forwarded = forward(request, Scheme::HTTP);
        let headers = forwarded.headers();
        assert_eq!(headers.get_all("x-real-ip").iter().count(), 1);
        assert_eq!(headers["x-real-ip"], "203.0.113.7");
        assert_eq!(headers.get_all("x-forwarded-proto").iter().count(), 1);
        assert_eq!(headers["x-forwarded-proto"], "http");
        assert_eq!(headers["x-forwarded-for"], "1.2.3.4, 203.0.113.7");
        assert_eq!(headers["x-forwarded-port"], "8443");
        assert_eq!(headers["host"], "example.com");
        assert_eq!(forwarded.uri(), "http://10.0.0.1:8080/path?q=1");
    }

    #[test]
    fn hop_by_hop_headers_are_removed() {
        let request = Request::builder()
            .uri("/")
            .header("connection", "keep-alive, x-secret")
            .header("keep-alive", "timeout=5")
            .header("x-secret", "1")
            .header("te", "trailers")
            .header("upgrade", "websocket")
            .header("x-kept", "1")
            .body(Body::empty())
            .unwrap();
        let forwarded = forward(request, Scheme::HTTPS);
        let headers = forwarded.headers();
        for name in ["connection", "keep-alive", "x-secret", "te", "upgrade"] {
            assert!(!headers.contains_key(name), "{} was forwarded", name);
        }
        assert!(headers.contains_key("x-kept"));
    }

    #[test]
    fn http2_request_gets_host_header_and_http11_version() {
        let request = Request::builder()
            .version(Version::HTTP_2)
            .uri("https://example.com/path")
            .body(Body::empty())
            .unwrap();
        let forwarded = forward(request, Scheme::HTTPS);
        assert_eq!(forwarded.version(), Version::HTTP_11);
        assert_eq!(forwarded.headers()["host"], "example.com");
    }
}
