use arc_swap::{access::Map, ArcSwap};
use clap::{App, Arg};
use configuration::{read_initial_config, watch_config, RuntimeConfig};
use listeners::{AcceptorProducer, Https};
use server::Scheme;
use std::{io, sync::Arc};
use tls::ReconfigurableCertificateResolver;
use tokio::try_join;
use tokio_rustls::rustls::{NoClientAuth, ServerConfig};
use warp::Filter;
use prometheus::Encoder;

mod acme;
mod backend_pool_matcher;
mod configuration;
mod error_response;
mod health;
mod http_client;
mod listeners;
mod algorithms;
mod logging;
mod middleware;
mod server;
mod tls;
mod utils;

#[tokio::main]
pub async fn main() -> Result<(), io::Error> {
    let matches = App::new("RustStrom")
        .version("1.0")
        .about("Rust-Strom is a powerful and efficient Load Balancer")
        .arg(
            Arg::with_name("config")
                .short("c")
                .long("config")
                .value_name("TOML FILE")
                .help("The path to the configuration in TOML format.")
                .required(true)
                .takes_value(true),
        )
        .get_matches();
    let config_path = matches.value_of("config").unwrap().to_string();

    logging::initialize();

    let config = read_initial_config(&config_path).await?;
    try_join!(
        watch_config(config_path, config.clone()),
        watch_health(config.clone()),
        listen_for_http_request(config.clone()),
        listen_for_https_request(config.clone()),
        serve_metrics()
    )?;
    Ok(())
}

async fn watch_health(config: Arc<ArcSwap<RuntimeConfig>>) -> Result<(), io::Error> {
  let backend_pools = Map::new(config.clone(), |it: &RuntimeConfig| &it.shared_data.backend_pools);
  let health_interval = Map::new(config.clone(), |it: &RuntimeConfig| &it.health_interval);
  health::watch_health(backend_pools, health_interval).await;
  Ok(())
}

async fn listen_for_http_request(config: Arc<ArcSwap<RuntimeConfig>>) -> Result<(), io::Error> {
  let http = listeners::Http;
  let address = config.load().http_address;
  let acceptor = http.produce_acceptor(address).await?;

  server::create(acceptor, config, Scheme::HTTP).await
}

async fn listen_for_https_request(config: Arc<ArcSwap<RuntimeConfig>>) -> Result<(), io::Error> {
  let mut tls_config = ServerConfig::new(NoClientAuth::new());
  let certificates = Map::new(config.clone(), |it: &RuntimeConfig| &it.certificates);
  let cert_resolver = ReconfigurableCertificateResolver::new(certificates);
  tls_config.cert_resolver = Arc::new(cert_resolver);

  let https = Https { tls_config };
  let address = config.load().https_address;
  let acceptor = https.produce_acceptor(address).await?;

  server::create(acceptor, config, Scheme::HTTPS).await
}

async fn serve_metrics() -> Result<(), io::Error> {
  // Define a warp filter that responds with the metrics.
  let metrics_route = warp::path("metrics").map(|| {
      let encoder = prometheus::TextEncoder::new();
      let mut buffer = vec![];
      let metric_families = prometheus::gather();
      encoder.encode(&metric_families, &mut buffer).unwrap();
      warp::reply::with_header(buffer, "content-type", encoder.format_type())
  });

  warp::serve(metrics_route)
      .run(([0, 0, 0, 0], 9091))
      .await;

  Ok(())
}