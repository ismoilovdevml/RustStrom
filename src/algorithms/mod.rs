use crate::{
    http_client::StrategyNotifyHttpConnector,
    middleware::{self, Middleware, MiddlewareChain},
    server::Scheme,
};
use async_trait::async_trait;
use hyper::{Body, Client, Request, Response, Uri};
use std::{fmt::Debug, net::SocketAddr};

pub mod ip_hash;
pub mod least_connection;
pub mod random;
pub mod round_robin;
pub mod sticky_cookie;

/// A trait for implementing load balancing, see
/// [`select_backend`](LoadBalancingStrategy::select_backend) for more details.
#[async_trait]
pub trait LoadBalancingStrategy: Send + Sync + std::fmt::Debug {
    /// Select the appropriate backend server and return a [`RequestForwarder`]
    /// for it.
    ///
    /// The [`RequestForwarder`] implements [`Middleware`]  to allow a
    /// [`LoadBalancingStrategy`] to modify the response if necessary. This is for
    /// example used to set a cookie in the strategy
    /// [`StickyCookie`](sticky_cookie::StickyCookie).
    fn select_backend<'l>(
        &'l self,
        request: &Request<Body>,
        context: &'l Context<'l>,
    ) -> RequestForwarder<'l>;

    /// Called when a new TCP connection to a backend server opened.
    fn on_tcp_open(&self, _remote: &Uri) {}

    /// Called when an existing backend TCP connection is closed.
    fn on_tcp_close(&self, _remote: &Uri) {}
}

pub struct Context<'l> {
    pub client_address: &'l SocketAddr,
    pub backend_addresses: &'l [&'l str],
}

/// A struct representing a backend server and allowing a final transformation
/// of the response before it is returned to the calling client.
///
/// [`RequestForwarder`] implements [`Middleware`] for convenience, but you can
/// not call [`forward_request`](RequestForwarder::forward_request) on it,
/// because you don't get access to the backend server address to construct the
/// [`middleware::Context`]. Instead you should call
/// [`forward_request_to_backend`][].
///
/// [`forward_request_to_backend`]: RequestForwarder::forward_request_to_backend
pub struct RequestForwarder<'l> {
    pub backend_address: &'l str,
    /// `None` means the response is passed through unchanged (no allocation).
    response_mapper: Option<ResponseMapper<'l>>,
}

type ResponseMapper<'l> = Box<dyn Fn(Response<Body>) -> Response<Body> + Send + Sync + 'l>;

impl<'l> RequestForwarder<'l> {
    /// Constructs a new [`RequestForwarder`] which does not perform a final
    /// response transformation.
    fn new(address: &'l str) -> RequestForwarder<'l> {
        RequestForwarder {
            backend_address: address,
            response_mapper: None,
        }
    }

    /// Add a `response_mapper` to perform a final response transformation.
    fn map_response<F>(self, response_mapper: F) -> RequestForwarder<'l>
    where
        F: Fn(Response<Body>) -> Response<Body> + Send + Sync + 'l,
    {
        let response_mapper: ResponseMapper<'l> = match self.response_mapper {
            None => Box::new(response_mapper),
            Some(previous) => Box::new(move |response| response_mapper(previous(response))),
        };
        RequestForwarder {
            backend_address: self.backend_address,
            response_mapper: Some(response_mapper),
        }
    }

    fn apply_response_mapper(&self, response: Response<Body>) -> Response<Body> {
        match &self.response_mapper {
            Some(mapper) => mapper(response),
            None => response,
        }
    }

    /// Forwards the `request` through `chain` to the backend server and applies
    /// the final response transformation of this [`RequestForwarder`].
    pub async fn forward_request_to_backend(
        &self,
        request: Request<Body>,
        chain: &MiddlewareChain,
        client_scheme: &Scheme,
        listener_port: u16,
        client_address: &SocketAddr,
        client: &Client<StrategyNotifyHttpConnector, Body>,
    ) -> Response<Body> {
        let backend_uri = match self.backend_uri(&request) {
            Ok(uri) => uri,
            Err(e) => {
                log::error!(
                    "Could not build URI for backend {}: {}",
                    self.backend_address,
                    e
                );
                return crate::error_response::bad_gateway();
            }
        };
        let context = middleware::Context {
            client_scheme,
            listener_port,
            client_address,
            backend_uri,
            client,
        };

        // Add request timeout protection (30 seconds default)
        let timeout_duration = std::time::Duration::from_secs(30);
        match tokio::time::timeout(
            timeout_duration,
            self.forward_request(request, chain, &context),
        )
        .await
        {
            Ok(response) => response,
            Err(_) => {
                // Timeout occurred
                log::warn!(
                    "Request to backend {} timed out after {:?}",
                    self.backend_address,
                    timeout_duration
                );
                crate::metrics::HTTP_TIMEOUTS_TOTAL.inc();
                crate::error_response::gateway_timeout()
            }
        }
    }

    fn backend_uri(&self, request: &Request<Body>) -> Result<Uri, hyper::http::Error> {
        // `OPTIONS * HTTP/1.1` and similar requests have no path-and-query
        let path = request
            .uri()
            .path_and_query()
            .map(|it| it.as_str())
            .unwrap_or("/");
        Uri::builder()
            .scheme("http")
            .authority(self.backend_address)
            .path_and_query(path)
            .build()
    }
}

#[async_trait]
impl Middleware for RequestForwarder<'_> {
    async fn modify_response(
        &self,
        response: Response<Body>,
        _context: &middleware::Context<'_>,
    ) -> Response<Body> {
        self.apply_response_mapper(response)
    }
}

impl Debug for RequestForwarder<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestForwarder")
            .field("backend_address", &self.backend_address)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_uri_without_path_does_not_panic() {
        let forwarder = RequestForwarder::new("127.0.0.1:8080");
        let request = Request::builder()
            .method("OPTIONS")
            .uri("*")
            .body(Body::empty())
            .unwrap();
        assert!(forwarder.backend_uri(&request).is_ok());
    }

    #[test]
    fn backend_uri_with_invalid_authority_is_an_error() {
        let forwarder = RequestForwarder::new("not a host");
        let request = Request::builder().uri("/").body(Body::empty()).unwrap();
        assert!(forwarder.backend_uri(&request).is_err());
    }
}
