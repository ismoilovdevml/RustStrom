//! Pingora services besides the proxy: metrics endpoint, health checks, config watcher and the
//! shutdown drain monitor.

use crate::{config::watcher, state::State, upstream::health};
use async_trait::async_trait;
use http::{Response, StatusCode};
use log::{info, warn};
use pingora_core::{
    apps::http_app::ServeHttp, protocols::http::ServerSession, server::ShutdownWatch,
    services::background::BackgroundService,
};
use std::{
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};

/// Serves `/metrics` in the Prometheus text format.
pub struct MetricsApp {
    pub state: Arc<State>,
}

#[async_trait]
impl ServeHttp for MetricsApp {
    async fn response(&self, session: &mut ServerSession) -> Response<Vec<u8>> {
        let response = if session.req_header().uri.path() == "/metrics" {
            match self.state.metrics.encode() {
                Ok((content_type, body)) => Response::builder()
                    .status(StatusCode::OK)
                    .header(http::header::CONTENT_TYPE, content_type)
                    .header(http::header::CONTENT_LENGTH, body.len())
                    .body(body),
                Err(e) => {
                    warn!("Could not encode metrics: {}", e);
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .header(http::header::CONTENT_LENGTH, 0)
                        .body(Vec::new())
                }
            }
        } else {
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(http::header::CONTENT_LENGTH, 0)
                .body(Vec::new())
        };
        response.unwrap_or_else(|_| Response::new(Vec::new()))
    }
}

pub struct HealthService(pub Arc<State>);

#[async_trait]
impl BackgroundService for HealthService {
    async fn start(&self, shutdown: ShutdownWatch) {
        health::run(self.0.clone(), shutdown).await;
    }
}

pub struct ConfigWatchService(pub Arc<State>);

#[async_trait]
impl BackgroundService for ConfigWatchService {
    async fn start(&self, shutdown: ShutdownWatch) {
        watcher::run(self.0.clone(), shutdown).await;
    }
}

/// Pingora waits the whole grace period after SIGTERM even when nothing is in flight anymore.
/// This service ends the process as soon as all in-flight requests are done (or the grace
/// period is over), so a drain takes as long as the slowest request, not always 30 s.
pub struct DrainService {
    pub state: Arc<State>,
    pub grace_period: Duration,
}

#[async_trait]
impl BackgroundService for DrainService {
    async fn start(&self, mut shutdown: ShutdownWatch) {
        while !*shutdown.borrow() {
            if shutdown.changed().await.is_err() {
                return;
            }
        }
        let started = Instant::now();
        info!("Shutdown: draining in-flight requests");
        // Let the listeners stop and requests that were just accepted get their context
        tokio::time::sleep(Duration::from_millis(200)).await;
        loop {
            let active = self.state.active_requests.load(Ordering::Relaxed);
            if active == 0 {
                info!(
                    "Shutdown complete, no request in flight after {:?}",
                    started.elapsed()
                );
                break;
            }
            if started.elapsed() >= self.grace_period {
                warn!(
                    "{} requests did not finish within {:?}, exiting anyway",
                    active, self.grace_period
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        std::process::exit(0);
    }
}
