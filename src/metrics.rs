// src/metrics.rs

use lazy_static::lazy_static;
use prometheus::{Counter, IntGauge, Histogram, Encoder, TextEncoder, IntCounterVec, HistogramVec};

lazy_static! {
    // Total number of HTTP requests made.
    pub static ref HTTP_REQUESTS_TOTAL: Counter = register_counter!(
        opts!("http_requests_total", "Total number of HTTP requests made.")
    ).unwrap();

    // Number of active HTTP connections.
    pub static ref ACTIVE_HTTP_CONNECTIONS: IntGauge = register_int_gauge!(
        opts!("active_http_connections", "Number of active HTTP connections.")
    ).unwrap();

    // Histogram to capture HTTP request processing duration.
    pub static ref HTTP_REQUEST_DURATION: Histogram = register_histogram!(
        "http_request_duration_seconds", "HTTP request processing duration.", 
        vec![0.025, 0.05, 0.1, 0.5, 1.0, 2.5, 5.0, 10.0]
    ).unwrap();

    // Backend failures.
    pub static ref BACKEND_FAILURES: IntCounterVec = register_int_counter_vec!(
        "backend_failures_total", "Total number of backend failures.",
        &["backend"]
    ).unwrap();

    // Bytes transferred.
    pub static ref BYTES_TRANSFERRED: IntCounterVec = register_int_counter_vec!(
        "bytes_transferred_total", "Total bytes transferred by the load balancer.",
        &["direction"]  // "inbound" or "outbound"
    ).unwrap();

    // Backend response time.
    pub static ref BACKEND_RESPONSE_TIME: HistogramVec = register_histogram_vec!(
        "backend_response_time_seconds", "Backend response time for processing requests.",
        &["backend"],
        vec![0.025, 0.05, 0.1, 0.5, 1.0, 2.5, 5.0, 10.0]
    ).unwrap();
}

pub fn gather_metrics() -> Vec<u8> {
    let encoder = TextEncoder::new();
    let mut buffer = vec![];
    let metric_families = prometheus::gather();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    buffer
}
