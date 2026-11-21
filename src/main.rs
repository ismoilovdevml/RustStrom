#![deny(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod acme;
mod config;
mod frontend;
mod logging;
mod matcher;
mod metrics;
mod middleware;
mod proxy;
mod router;
mod services;
mod state;
mod tls;
mod upstream;

use clap::{Arg, ArgAction, Command};
use log::info;
use pingora_core::{
    server::{configuration::Opt, configuration::ServerConf, Server},
    services::{background::background_service, listening::Service},
};
use std::{path::PathBuf, sync::Arc, time::Duration};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn cli() -> Command {
    Command::new("RustStrom")
        .version(env!("CARGO_PKG_VERSION"))
        .about("RustStrom - High-Performance Load Balancer written in Rust")
        .arg(
            Arg::new("config")
                .short('c')
                .long("config")
                .value_name("TOML FILE")
                .help("The path to the configuration in TOML format.")
                .required(true),
        )
        .arg(
            Arg::new("check")
                .long("check")
                .action(ArgAction::SetTrue)
                .help("Validate the configuration file and exit."),
        )
        .arg(
            Arg::new("upgrade")
                .short('u')
                .long("upgrade")
                .action(ArgAction::SetTrue)
                .help("Take over the listening sockets of a running instance (graceful upgrade)."),
        )
        .arg(
            Arg::new("daemon")
                .short('d')
                .long("daemon")
                .action(ArgAction::SetTrue)
                .help("Run in the background."),
        )
}

fn run() -> Result<(), String> {
    let matches = cli().get_matches();
    let config_path = matches
        .get_one::<String>("config")
        .cloned()
        .ok_or_else(|| "--config is required".to_string())?;

    logging::initialize();

    let metrics = Arc::new(metrics::Metrics::new().map_err(|e| e.to_string())?);
    let config = config::load(std::path::Path::new(&config_path), &metrics, None)?;
    if matches.get_flag("check") {
        println!("Configuration '{}' is valid", config_path);
        return Ok(());
    }

    let http_address = config.http_address.to_string();
    let https_socket = config.https_address;
    let https_address = https_socket.to_string();
    let metrics_address = config.metrics_address.to_string();
    let process = config.process.clone();
    let state = Arc::new(state::State::new(
        config,
        metrics,
        PathBuf::from(&config_path),
    ));

    let opt = Opt {
        upgrade: matches.get_flag("upgrade"),
        daemon: matches.get_flag("daemon"),
        nocapture: false,
        test: false,
        conf: None,
    };
    let mut server_conf = ServerConf {
        threads: process.threads,
        grace_period_seconds: Some(process.grace_period_secs),
        graceful_shutdown_timeout_seconds: Some(5),
        upstream_keepalive_pool_size: process.upstream_keepalive_pool_size,
        upgrade_sock: process.upgrade_sock.clone(),
        pid_file: process.pid_file.clone(),
        max_retries: process.max_retries.max(1),
        ..ServerConf::default()
    };
    server_conf.version = 1;
    let mut server = Server::new_with_opt_and_conf(opt, server_conf);
    server.bootstrap();

    let http_proxy =
        pingora_proxy::http_proxy(&server.configuration, proxy::Proxy::new(state.clone()));
    let frontend = frontend::Frontend::new(
        http_proxy,
        https_socket,
        tls::SniResolver::new(state.clone()),
    )?;
    let mut proxy = Service::new("RustStrom proxy".to_string(), frontend);
    // Both listeners are plain TCP for Pingora; TLS is terminated in the frontend
    proxy.add_tcp(&http_address);
    proxy.add_tcp(&https_address);
    info!(
        "Listening on http://{} and https://{} with {} worker threads",
        http_address, https_address, process.threads
    );

    let mut metrics_service = Service::new(
        "RustStrom metrics".to_string(),
        services::MetricsApp {
            state: state.clone(),
        },
    );
    metrics_service.add_tcp(&metrics_address);
    metrics_service.threads = Some(1);
    info!(
        "Serving Prometheus metrics on http://{}/metrics",
        metrics_address
    );

    let health = background_service("health checks", services::HealthService(state.clone()));
    let watcher = background_service(
        "config watcher",
        services::ConfigWatchService(state.clone()),
    );
    let drain = background_service(
        "drain",
        services::DrainService {
            state: state.clone(),
            grace_period: Duration::from_secs(process.grace_period_secs),
        },
    );

    let acme = background_service(
        "acme",
        acme::AcmeService {
            state: state.clone(),
        },
    );

    let proxy_handle = server.add_service(proxy);
    server.add_service(metrics_service);
    server.add_service(health);
    server.add_service(watcher);
    server.add_service(drain);
    // Orders start only once the HTTP listener can answer the HTTP-01 challenges
    let acme_handle = server.add_service(acme);
    acme_handle.add_dependency(&proxy_handle);
    server.run_forever();
}
