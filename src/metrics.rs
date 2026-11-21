// src/metrics.rs

use lazy_static::lazy_static;
use prometheus::{
    opts, register_counter, register_histogram, register_histogram_vec, register_int_counter_vec,
    register_int_gauge, Counter, Histogram, HistogramVec, IntCounterVec, IntGauge,
};

/// Sub-millisecond buckets so that proxy overhead is visible, not only slow backends.
const LATENCY_BUCKETS: [f64; 14] = [
    0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

lazy_static! {
    // Total number of HTTP requests made.
    pub static ref HTTP_REQUESTS_TOTAL: Counter = register_counter!(
        opts!("http_requests_total", "Total number of HTTP requests made.")
    ).unwrap();

    // Number of HTTP requests currently being processed.
    pub static ref ACTIVE_HTTP_CONNECTIONS: IntGauge = register_int_gauge!(
        opts!("active_http_connections", "Number of active HTTP connections.")
    ).unwrap();

    // Histogram to capture HTTP request processing duration.
    pub static ref HTTP_REQUEST_DURATION: Histogram = register_histogram!(
        "http_request_duration_seconds", "HTTP request processing duration.",
        LATENCY_BUCKETS.to_vec()
    ).unwrap();

    // Backend failures.
    pub static ref BACKEND_FAILURES: IntCounterVec = register_int_counter_vec!(
        "backend_failures_total", "Total number of backend failures.",
        &["backend"]
    ).unwrap();

    // Backend response time.
    pub static ref BACKEND_RESPONSE_TIME: HistogramVec = register_histogram_vec!(
        "backend_response_time_seconds", "Backend response time for processing requests.",
        &["backend"],
        LATENCY_BUCKETS.to_vec()
    ).unwrap();

    // HTTP status codes distribution.
    pub static ref HTTP_STATUS_CODES: IntCounterVec = register_int_counter_vec!(
        "http_status_codes_total", "Total number of HTTP requests by status code.",
        &["code"]
    ).unwrap();

    // HTTP errors (5xx responses).
    pub static ref HTTP_ERRORS_TOTAL: Counter = register_counter!(
        opts!("http_errors_total", "Total number of HTTP errors (5xx responses).")
    ).unwrap();

    // HTTP timeouts.
    pub static ref HTTP_TIMEOUTS_TOTAL: Counter = register_counter!(
        opts!("http_timeouts_total", "Total number of HTTP request timeouts.")
    ).unwrap();
}
