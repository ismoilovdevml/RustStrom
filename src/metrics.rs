//! Prometheus metrics. The names are the ones of v1.1 so existing dashboards keep working.

use prometheus::{
    Counter, Encoder, Histogram, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge,
    Opts, Registry, TextEncoder,
};

/// Sub-millisecond buckets so that proxy overhead is visible, not only slow backends.
const LATENCY_BUCKETS: [f64; 14] = [
    0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

pub struct Metrics {
    registry: Registry,
    /// Total number of HTTP requests made.
    pub http_requests_total: Counter,
    /// Number of requests currently being processed (v1.1 name kept for compatibility).
    pub active_http_connections: IntGauge,
    pub http_request_duration: Histogram,
    pub backend_failures: IntCounterVec,
    pub backend_response_time: HistogramVec,
    pub http_status_codes: IntCounterVec,
    pub http_errors_total: Counter,
    pub http_timeouts_total: Counter,
    /// New in v2: attempts that were retried on another backend.
    pub upstream_retries_total: IntCounter,
    /// New in v2: health state per backend (0 down, 1 slow, 2 healthy).
    pub backend_health: prometheus::IntGaugeVec,
}

impl Metrics {
    pub fn new() -> Result<Metrics, prometheus::Error> {
        let registry = Registry::new();
        let http_requests_total = Counter::with_opts(Opts::new(
            "http_requests_total",
            "Total number of HTTP requests made.",
        ))?;
        let active_http_connections = IntGauge::with_opts(Opts::new(
            "active_http_connections",
            "Number of active HTTP connections.",
        ))?;
        let http_request_duration = Histogram::with_opts(
            HistogramOpts::new(
                "http_request_duration_seconds",
                "HTTP request processing duration.",
            )
            .buckets(LATENCY_BUCKETS.to_vec()),
        )?;
        let backend_failures = IntCounterVec::new(
            Opts::new(
                "backend_failures_total",
                "Total number of backend failures.",
            ),
            &["backend"],
        )?;
        let backend_response_time = HistogramVec::new(
            HistogramOpts::new(
                "backend_response_time_seconds",
                "Backend response time for processing requests.",
            )
            .buckets(LATENCY_BUCKETS.to_vec()),
            &["backend"],
        )?;
        let http_status_codes = IntCounterVec::new(
            Opts::new(
                "http_status_codes_total",
                "Total number of HTTP requests by status code.",
            ),
            &["code"],
        )?;
        let http_errors_total = Counter::with_opts(Opts::new(
            "http_errors_total",
            "Total number of HTTP errors (5xx responses).",
        ))?;
        let http_timeouts_total = Counter::with_opts(Opts::new(
            "http_timeouts_total",
            "Total number of HTTP request timeouts.",
        ))?;
        let upstream_retries_total = IntCounter::with_opts(Opts::new(
            "upstream_retries_total",
            "Total number of upstream attempts retried on another backend.",
        ))?;
        let backend_health = prometheus::IntGaugeVec::new(
            Opts::new(
                "backend_health",
                "Backend health state: 0 down, 1 slow, 2 healthy.",
            ),
            &["backend"],
        )?;

        registry.register(Box::new(http_requests_total.clone()))?;
        registry.register(Box::new(active_http_connections.clone()))?;
        registry.register(Box::new(http_request_duration.clone()))?;
        registry.register(Box::new(backend_failures.clone()))?;
        registry.register(Box::new(backend_response_time.clone()))?;
        registry.register(Box::new(http_status_codes.clone()))?;
        registry.register(Box::new(http_errors_total.clone()))?;
        registry.register(Box::new(http_timeouts_total.clone()))?;
        registry.register(Box::new(upstream_retries_total.clone()))?;
        registry.register(Box::new(backend_health.clone()))?;

        Ok(Metrics {
            registry,
            http_requests_total,
            active_http_connections,
            http_request_duration,
            backend_failures,
            backend_response_time,
            http_status_codes,
            http_errors_total,
            http_timeouts_total,
            upstream_retries_total,
            backend_health,
        })
    }

    /// Counter for a status code. Status codes are bounded (100-999), so the label set is too.
    pub fn status_code(&self, code: u16) -> Option<IntCounter> {
        let mut buffer = [0u8; 5];
        let label = format_status(code, &mut buffer)?;
        self.http_status_codes
            .get_metric_with_label_values(&[label])
            .ok()
    }

    /// Drops the per-backend series of a backend that is no longer configured.
    pub fn remove_backend(&self, address: &str) {
        // Err only means the series did not exist
        let _ = self.backend_response_time.remove_label_values(&[address]);
        let _ = self.backend_failures.remove_label_values(&[address]);
        let _ = self.backend_health.remove_label_values(&[address]);
    }

    /// Text exposition of all metrics.
    pub fn encode(&self) -> Result<(String, Vec<u8>), prometheus::Error> {
        let encoder = TextEncoder::new();
        let mut buffer = Vec::new();
        encoder.encode(&self.registry.gather(), &mut buffer)?;
        Ok((encoder.format_type().to_string(), buffer))
    }
}

/// Formats a status code without allocating.
fn format_status(code: u16, buffer: &mut [u8; 5]) -> Option<&str> {
    if !(100..=999).contains(&code) {
        return None;
    }
    let digits = [code / 100, (code / 10) % 10, code % 10];
    for (slot, digit) in buffer.iter_mut().zip(digits) {
        *slot = b'0' + u8::try_from(digit).ok()?;
    }
    std::str::from_utf8(&buffer[..3]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_labels() {
        let mut buffer = [0u8; 5];
        assert_eq!(format_status(200, &mut buffer), Some("200"));
        assert_eq!(format_status(504, &mut buffer), Some("504"));
        assert_eq!(format_status(42, &mut buffer), None);
    }

    #[test]
    fn removed_backend_series_disappear() {
        let metrics = Metrics::new().unwrap();
        metrics
            .backend_failures
            .with_label_values(&["10.0.0.1:80"])
            .inc();
        metrics
            .backend_health
            .with_label_values(&["10.0.0.1:80"])
            .set(2);
        metrics.remove_backend("10.0.0.1:80");
        metrics.remove_backend("never-existed:1");
        let (_, body) = metrics.encode().unwrap();
        assert!(!String::from_utf8(body).unwrap().contains("10.0.0.1:80"));
    }

    #[test]
    fn exposes_v1_names_and_sub_ms_buckets() {
        let metrics = Metrics::new().unwrap();
        metrics.http_request_duration.observe(0.0001);
        metrics.http_requests_total.inc();
        metrics.status_code(200).unwrap().inc();
        metrics
            .backend_response_time
            .with_label_values(&["127.0.0.1:1"])
            .observe(0.1);
        metrics
            .backend_failures
            .with_label_values(&["127.0.0.1:1"])
            .inc();
        let (_, body) = metrics.encode().unwrap();
        let text = String::from_utf8(body).unwrap();
        for name in [
            "http_requests_total",
            "active_http_connections",
            "http_request_duration_seconds_bucket{le=\"0.0005\"}",
            "backend_failures_total",
            "backend_response_time_seconds",
            "http_status_codes_total{code=\"200\"}",
            "http_errors_total",
            "http_timeouts_total",
        ] {
            assert!(text.contains(name), "{} missing", name);
        }
    }
}
