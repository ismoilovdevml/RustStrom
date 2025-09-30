use crate::{
    acme::AcmeHandler,
    algorithms::{self, LoadBalancingStrategy},
    backend_pool_matcher::BackendPoolMatcher,
    configuration::RuntimeConfig,
    error_response::{bad_gateway, not_found},
    health::{HealthConfig, Healthiness},
    http_client::StrategyNotifyHttpConnector,
    listeners::RemoteAddress,
    metrics,
    middleware::MiddlewareChain,
};
use arc_swap::ArcSwap;
use futures::Future;
use futures::TryFutureExt;
use hyper::{
    server::accept::Accept,
    service::{make_service_fn, Service},
    Body, Client, Request, Response, Server,
};
use log::debug;
use serde::Deserialize;
use std::{
    collections::HashSet,
    error::Error,
    fmt::Display,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncWrite};

pub async fn create<I, IE, IO>(
    acceptor: I,
    config: Arc<ArcSwap<RuntimeConfig>>,
    scheme: Scheme,
) -> Result<(), io::Error>
where
    I: Accept<Conn = IO, Error = IE>,
    IE: Into<Box<dyn Error + Send + Sync>>,
    IO: AsyncRead + AsyncWrite + Unpin + Send + RemoteAddress + 'static,
{
    let service = make_service_fn(move |stream: &IO| {
        let client_address = stream.remote_addr().expect("No remote SocketAddr");
        let config = config.clone();

        async move {
            Ok::<_, io::Error>(MainService {
                client_address,
                config,
                scheme,
            })
        }
    });
    let server = Server::builder(acceptor)
        .http1_keepalive(true)
        .http1_half_close(true)
        .http2_keep_alive_interval(Some(Duration::from_secs(20)))
        .http2_keep_alive_timeout(Duration::from_secs(20))
        .serve(service);

    // Graceful shutdown handling
    let graceful = server.with_graceful_shutdown(async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to listen for shutdown signal");
        log::info!("Received shutdown signal, gracefully shutting down...");
    });

    graceful
        .map_err(|e| {
            let msg = format!("Failed to listen server: {}", e);
            io::Error::other(msg)
        })
        .await
}

pub struct MainService {
    client_address: SocketAddr,
    config: Arc<ArcSwap<RuntimeConfig>>,
    scheme: Scheme,
}

impl Service<Request<Body>> for MainService {
    type Response = Response<Body>;
    type Error = hyper::Error;

    // let's allow this complex type. A refactor would make it more complicated due to the used trait types
    #[allow(clippy::type_complexity)]
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request<Body>) -> Self::Future {
        debug!(
            "{:#?} {} {}",
            request.version(),
            request.method(),
            request.uri()
        );

        // Increment total requests counter
        metrics::HTTP_REQUESTS_TOTAL.inc();

        // Increment active connections
        metrics::ACTIVE_HTTP_CONNECTIONS.inc();

        let config = self.config.load();
        let shared_data = &config.shared_data;

        if let Some(response) = shared_data.acme_handler.respond_to_challenge(&request) {
            metrics::ACTIVE_HTTP_CONNECTIONS.dec();
            return Box::pin(async move { Ok(response) });
        }

        match pool_by_req(shared_data, &request, &self.scheme) {
            Some(pool) => {
                let client_scheme = self.scheme;
                let client_address = self.client_address;

                Box::pin(async move {
                    let start_time = std::time::Instant::now();
                    // clone, filter, map, LoadBalancingContext:backend_addresses
                    let mut working_addresses = pool
                        .addresses
                        .iter()
                        .filter(|(_, healthiness)| {
                            healthiness.load().as_ref() == &Healthiness::Healthy
                        })
                        .map(|(address, _)| address.as_str())
                        .collect::<Vec<_>>();

                    if working_addresses.is_empty() {
                        // replace healthy addresses with slow addresses
                        working_addresses = pool
                            .addresses
                            .iter()
                            .filter(|(_, healthiness)| {
                                matches!(healthiness.load().as_ref(), Healthiness::Slow(_))
                            })
                            .map(|(address, _)| address.as_str())
                            .collect::<Vec<_>>();
                    }
                    if working_addresses.is_empty() {
                        // we don't have any working addresses, so don't call load balancer strategy and abort early
                        // middlewares are also not running
                        metrics::ACTIVE_HTTP_CONNECTIONS.dec();
                        let duration = start_time.elapsed().as_secs_f64();
                        metrics::HTTP_REQUEST_DURATION.observe(duration);

                        let response = bad_gateway();
                        metrics::HTTP_STATUS_CODES.with_label_values(&["502"]).inc();
                        metrics::HTTP_ERRORS_TOTAL.inc();

                        Ok(response)
                    } else {
                        let context = algorithms::Context {
                            client_address: &client_address,
                            backend_addresses: &working_addresses,
                        };
                        let backend = pool.strategy.select_backend(&request, &context);

                        let backend_start = std::time::Instant::now();
                        let result = backend
                            .forward_request_to_backend(
                                request,
                                &pool.chain,
                                &client_scheme,
                                &client_address,
                                &pool.client,
                            )
                            .await;

                        // Track backend response time
                        let backend_duration = backend_start.elapsed().as_secs_f64();
                        let backend_addr = backend.backend_address;
                        metrics::BACKEND_RESPONSE_TIME
                            .with_label_values(&[backend_addr])
                            .observe(backend_duration);

                        // Track status code
                        let status = result.status();
                        let status_code = status.as_u16().to_string();
                        metrics::HTTP_STATUS_CODES
                            .with_label_values(&[&status_code])
                            .inc();

                        // Track errors (5xx)
                        if status.is_server_error() {
                            metrics::HTTP_ERRORS_TOTAL.inc();
                        }

                        // Decrement active connections and record request duration
                        metrics::ACTIVE_HTTP_CONNECTIONS.dec();
                        let duration = start_time.elapsed().as_secs_f64();
                        metrics::HTTP_REQUEST_DURATION.observe(duration);

                        Ok(result)
                    }
                })
            }
            _ => {
                metrics::ACTIVE_HTTP_CONNECTIONS.dec();
                metrics::HTTP_STATUS_CODES.with_label_values(&["404"]).inc();
                Box::pin(async { Ok(not_found()) })
            }
        }
    }
}

fn pool_by_req(
    shared_data: &SharedData,
    request: &Request<Body>,
    scheme: &Scheme,
) -> Option<Arc<BackendPool>> {
    shared_data
        .backend_pools
        .iter()
        .filter(|pool| pool.supports(scheme))
        .find(|pool| pool.matcher.matches(request))
        .cloned()
}

pub struct SharedData {
    pub backend_pools: Vec<Arc<BackendPool>>,
    pub acme_handler: Arc<AcmeHandler>,
}

#[derive(Debug)]
pub struct BackendPool {
    pub matcher: BackendPoolMatcher,
    pub addresses: Vec<(String, ArcSwap<Healthiness>)>,
    pub health_config: HealthConfig,
    pub strategy: Arc<Box<dyn LoadBalancingStrategy>>,
    pub chain: MiddlewareChain,
    pub client: Client<StrategyNotifyHttpConnector, Body>,
    pub schemes: HashSet<Scheme>,
}

impl BackendPool {
    fn supports(&self, scheme: &Scheme) -> bool {
        self.schemes.contains(scheme)
    }
}

impl PartialEq for BackendPool {
    fn eq(&self, other: &Self) -> bool {
        self.matcher.eq(&other.matcher)
    }
}

pub struct BackendPoolBuilder {
    matcher: BackendPoolMatcher,
    addresses: Vec<(String, ArcSwap<Healthiness>)>,
    health_config: HealthConfig,
    strategy: Box<dyn LoadBalancingStrategy>,
    chain: MiddlewareChain,
    schemes: HashSet<Scheme>,
    pool_idle_timeout: Option<Duration>,
    pool_max_idle_per_host: Option<usize>,
}

impl BackendPoolBuilder {
    pub fn new(
        matcher: BackendPoolMatcher,
        addresses: Vec<(String, ArcSwap<Healthiness>)>,
        health_config: HealthConfig,
        strategy: Box<dyn LoadBalancingStrategy>,
        chain: MiddlewareChain,
        schemes: HashSet<Scheme>,
    ) -> BackendPoolBuilder {
        BackendPoolBuilder {
            matcher,
            addresses,
            health_config,
            strategy,
            chain,
            schemes,
            pool_idle_timeout: None,
            pool_max_idle_per_host: None,
        }
    }

    pub fn pool_idle_timeout(&mut self, duration: Duration) -> &BackendPoolBuilder {
        self.pool_idle_timeout = Some(duration);
        self
    }

    pub fn pool_max_idle_per_host(&mut self, max_idle: usize) -> &BackendPoolBuilder {
        self.pool_max_idle_per_host = Some(max_idle);
        self
    }

    pub fn build(self) -> BackendPool {
        let mut client_builder = Client::builder();
        if let Some(pool_idle_timeout) = self.pool_idle_timeout {
            client_builder.pool_idle_timeout(pool_idle_timeout);
        }
        if let Some(pool_max_idle_per_host) = self.pool_max_idle_per_host {
            client_builder.pool_max_idle_per_host(pool_max_idle_per_host);
        }

        let strategy = Arc::new(self.strategy);
        let client: Client<_, Body> =
            client_builder.build(StrategyNotifyHttpConnector::new(strategy.clone()));

        BackendPool {
            matcher: self.matcher,
            addresses: self.addresses,
            health_config: self.health_config,
            strategy,
            chain: self.chain,
            client,
            schemes: self.schemes,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Deserialize, Hash)]
#[allow(clippy::upper_case_acronyms)]
pub enum Scheme {
    HTTP,
    HTTPS,
}

impl Display for Scheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Scheme::HTTP => write!(f, "http"),
            Scheme::HTTPS => write!(f, "https"),
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::algorithms::random::Random;
    use std::{collections::HashMap, iter::FromIterator};

    fn generate_config(shared_data: SharedData) -> RuntimeConfig {
        RuntimeConfig {
            shared_data,
            http_address: "0.0.0.0:80".parse().unwrap(),
            https_address: "0.0.0.0:443".parse().unwrap(),
            certificates: HashMap::new(),
            health_interval: std::time::Duration::from_secs(60),
        }
    }
    fn generate_test_service(host: String, scheme: Scheme) -> MainService {
        MainService {
            scheme,
            client_address: "127.0.0.1:3000".parse().unwrap(),
            config: Arc::new(ArcSwap::from_pointee(generate_config(SharedData {
                backend_pools: vec![Arc::new(
                    BackendPoolBuilder::new(
                        BackendPoolMatcher::Host(host),
                        vec![(
                            "127.0.0.1:8084".into(),
                            ArcSwap::from_pointee(Healthiness::Healthy),
                        )],
                        HealthConfig {
                            slow_threshold: 200,
                            timeout: 500,
                            path: String::from("/"),
                        },
                        Box::new(Random::new()),
                        MiddlewareChain::Empty,
                        HashSet::from_iter(vec![Scheme::HTTP]),
                    )
                    .build(),
                )],
                acme_handler: Arc::new(AcmeHandler::new()),
            }))),
        }
    }

    #[test]
    fn pool_by_req_no_matching_pool() {
        let service = generate_test_service("whoami.localhost".into(), Scheme::HTTP);
        let config = service.config.load();
        let shared_data = &config.shared_data;
        let request = Request::builder()
            .header("host", "whoami.de")
            .body(Body::empty())
            .unwrap();

        let pool = pool_by_req(shared_data, &request, &service.scheme);

        assert_eq!(pool, None);
    }

    #[test]
    fn pool_by_req_matching_pool() {
        let service = generate_test_service("whoami.localhost".into(), Scheme::HTTP);
        let config = service.config.load();
        let shared_data = &config.shared_data;
        let request = Request::builder()
            .header("host", "whoami.localhost")
            .body(Body::empty())
            .unwrap();

        let pool = pool_by_req(shared_data, &request, &service.scheme);

        assert_eq!(pool, Some(shared_data.backend_pools[0].clone()));
    }
}
