use crate::{
    http_client::StrategyNotifyHttpConnector,
    middleware::{self, Middleware, MiddlewareChain},
    server::Scheme,
};
use async_trait::async_trait;
use hyper::{Body, Request, Response, Uri};
use std::{convert::identity, fmt::Debug, net::SocketAddr};

pub mod ip_hash;
pub mod least_connection;
pub mod random;
pub mod round_robin;
pub mod sticky_cookie;

#[async_trait]
pub trait LoadBalancingStrategy: Send + Sync + std::fmt::Debug {
    fn select_backend<'l>(&'l self, request: &Request<Body>, context: &'l Context) -> RequestForwarder;
    fn on_tcp_open(&self, _remote: &Uri) {}
    fn on_tcp_close(&self, _remote: &Uri) {}
}

pub struct Context<'l> {
    pub client_address: &'l SocketAddr,
    pub backend_addresses: &'l [&'l str],
}

pub struct RequestForwarder<'l> {
    backend_address: &'l str,
    response_mapper: Box<dyn Fn(Response<Body>) -> Response<Body> + Send + Sync + 'l>,
}

impl<'l> RequestForwarder<'l> {
    fn new(address: &str) -> RequestForwarder {
        RequestForwarder::new_with_response_mapper(address, identity)
    }

    fn new_with_response_mapper<'n, F>(address: &'n str, response_mapper: F) -> RequestForwarder<'n>
    where
        F: Fn(Response<Body>) -> Response<Body> + Send + Sync + 'n,
    {
        RequestForwarder {
            backend_address: address,
            response_mapper: Box::new(response_mapper),
        }
    }

    fn map_response<F>(self, response_mapper: F) -> RequestForwarder<'l>
    where
        F: Fn(Response<Body>) -> Response<Body> + Send + Sync + 'l,
    {
        RequestForwarder {
            backend_address: self.backend_address,
            response_mapper: Box::new(move |response| response_mapper((self.response_mapper)(response))),
        }
    }

    pub async fn forward_request_to_backend(
        &self,
        request: Request<Body>,
        chain: &MiddlewareChain,
        client_scheme: &Scheme,
        client_address: &SocketAddr,
        client: &Client<StrategyNotifyHttpConnector, Body>,
    ) -> Response<Body> {
        let context = middleware::Context {
            client_scheme,
            client_address,
            backend_uri: self.backend_uri(&request),
            client,
        };
        self.forward_request(request, chain, &context).await
    }

    fn backend_uri(&self, request: &Request<Body>) -> Uri {
        let path = request.uri().path_and_query().unwrap().clone();
        Uri::builder()
            .scheme("http")
            .authority(self.backend_address)
            .path_and_query(path)
            .build()
            .unwrap()
    }
}

#[async_trait]
impl Middleware for RequestForwarder<'_> {
    async fn modify_response(&self, response: Response<Body>, _context: &middleware::Context<'_>) -> Response<Body> {
        (self.response_mapper)(response)
    }
}

impl Debug for RequestForwarder<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestForwarder")
            .field("backend_address", &self.backend_address)
            .finish()
    }
}

// New addition: `select_server` function
pub fn select_server(servers: &[Server], balance_strategy: &str) -> Option<&Server> {
    match balance_strategy {
        "RoundRobin" => round_robin::select_server(servers),
        "Random" => random::select_server(servers),
        "LeastConnection" => least_connection::select_server(servers),
        "IPHash" => ip_hash::select_server(servers),
        "StickyCookie" => sticky_cookie::select_server(servers),
        _ => None,
    }
}
