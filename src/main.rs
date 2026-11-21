use arc_swap::{access::Map, ArcSwap};
use clap::{App, Arg};
use configuration::{read_initial_config, watch_config, RuntimeConfig};
use hyper::{
    header::CONTENT_TYPE,
    service::{make_service_fn, service_fn},
    Body, Response, Server, StatusCode,
};
use listeners::{AcceptorProducer, Https};
use log::{error, info, warn};
use prometheus::Encoder;
use server::Scheme;
use std::{convert::Infallible, io, net::SocketAddr, sync::Arc, time::Duration};
use tls::ReconfigurableCertificateResolver;
use tokio::try_join;
use tokio_rustls::rustls::ServerConfig;
use tokio_util::sync::CancellationToken;

mod acme;
mod algorithms;
mod backend_pool_matcher;
mod configuration;
mod error_response;
mod health;
mod http_client;
mod listeners;
mod logging;
mod metrics;
mod middleware;
mod server;
mod tls;
mod utils;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// How long in-flight requests may take to finish after a shutdown signal.
const SHUTDOWN_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::main]
pub async fn main() -> Result<(), io::Error> {
    let matches = App::new("RustStrom")
        .version(env!("CARGO_PKG_VERSION"))
        .about("RustStrom - High-Performance Load Balancer written in Rust")
        .arg(
            Arg::with_name("config")
                .short("c")
                .long("config")
                .value_name("TOML FILE")
                .help("The path to the configuration in TOML format.")
                .required(true)
                .takes_value(true),
        )
        .arg(
            Arg::with_name("check")
                .long("check")
                .help("Validate the configuration file and exit."),
        )
        .get_matches();
    let config_path = matches
        .value_of("config")
        .expect("clap enforces the required config argument")
        .to_string();

    logging::initialize();

    let config = read_initial_config(&config_path).await?;
    if matches.is_present("check") {
        println!("Configuration '{}' is valid", config_path);
        return Ok(());
    }

    let shutdown = CancellationToken::new();
    tokio::spawn(wait_for_shutdown_signal(shutdown.clone()));

    let servers = async {
        try_join!(
            listen_for_http_request(config.clone(), shutdown.clone()),
            listen_for_https_request(config.clone(), shutdown.clone()),
        )
        .map(|_| ())
    };
    let background = async {
        try_join!(
            watch_config(config_path, config.clone()),
            watch_health(config.clone()),
            serve_metrics(config.load().metrics_address, shutdown.clone()),
        )
        .map(|_| ())
    };
    let drain_deadline = async {
        shutdown.cancelled().await;
        tokio::time::sleep(SHUTDOWN_DRAIN_TIMEOUT).await;
    };

    tokio::select! {
        result = servers => result?,
        result = background => result?,
        _ = drain_deadline => warn!(
            "In-flight requests did not finish within {:?}, exiting anyway",
            SHUTDOWN_DRAIN_TIMEOUT
        ),
    }
    info!("Shutdown complete");
    Ok(())
}

/// Cancels `token` on SIGINT or SIGTERM (Docker, Kubernetes and systemd send SIGTERM).
async fn wait_for_shutdown_signal(token: CancellationToken) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut sigterm) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {},
                    _ = sigterm.recv() => {},
                }
            }
            Err(e) => {
                error!("Could not install SIGTERM handler: {}", e);
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
    info!("Received shutdown signal, draining connections...");
    token.cancel();
}

async fn watch_health(config: Arc<ArcSwap<RuntimeConfig>>) -> Result<(), io::Error> {
    let backend_pools = Map::new(config.clone(), |it: &RuntimeConfig| {
        &it.shared_data.backend_pools
    });
    let health_interval = Map::new(config.clone(), |it: &RuntimeConfig| &it.health_interval);
    health::watch_health(backend_pools, health_interval).await;
    Ok(())
}

async fn listen_for_http_request(
    config: Arc<ArcSwap<RuntimeConfig>>,
    shutdown: CancellationToken,
) -> Result<(), io::Error> {
    let http = listeners::Http;
    let address = config.load().http_address;
    let acceptor = http.produce_acceptor(address).await?;

    server::create(acceptor, config, Scheme::HTTP, shutdown).await
}

async fn listen_for_https_request(
    config: Arc<ArcSwap<RuntimeConfig>>,
    shutdown: CancellationToken,
) -> Result<(), io::Error> {
    let certificates = Map::new(config.clone(), |it: &RuntimeConfig| &it.certificates);
    let cert_resolver = ReconfigurableCertificateResolver::new(certificates);

    let mut tls_config = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(cert_resolver));
    // Without ALPN clients can never negotiate HTTP/2 over TLS.
    tls_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    let https = Https { tls_config };
    let address = config.load().https_address;
    let acceptor = https.produce_acceptor(address).await?;

    server::create(acceptor, config, Scheme::HTTPS, shutdown).await
}

async fn serve_metrics(address: SocketAddr, shutdown: CancellationToken) -> Result<(), io::Error> {
    let make_service = make_service_fn(|_| async {
        Ok::<_, Infallible>(service_fn(|request: hyper::Request<Body>| async move {
            let response = if request.uri().path() == "/metrics" {
                let encoder = prometheus::TextEncoder::new();
                let mut buffer = Vec::new();
                match encoder.encode(&prometheus::gather(), &mut buffer) {
                    Ok(()) => Response::builder()
                        .header(CONTENT_TYPE, encoder.format_type())
                        .body(Body::from(buffer)),
                    Err(e) => {
                        error!("Could not encode metrics: {}", e);
                        Response::builder()
                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                            .body(Body::empty())
                    }
                }
            } else {
                Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Body::empty())
            };
            Ok::<_, Infallible>(response.unwrap_or_else(|_| Response::new(Body::empty())))
        }))
    });

    let server = Server::try_bind(&address)
        .map_err(|e| io::Error::other(format!("Could not bind metrics on {}: {}", address, e)))?
        .serve(make_service);
    info!("Serving Prometheus metrics on http://{}/metrics", address);
    server
        .with_graceful_shutdown(shutdown.cancelled_owned())
        .await
        .map_err(io::Error::other)
}
