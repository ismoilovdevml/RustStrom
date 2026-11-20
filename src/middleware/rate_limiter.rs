use super::{Context, Middleware};
use crate::error_response::too_many_requests;
use async_trait::async_trait;
use hyper::{Body, Request, Response};
use linked_hash_map::LinkedHashMap;
use std::{
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

/// Upper bound of tracked clients, protects against memory exhaustion when
/// many distinct addresses (e.g. a whole IPv6 prefix) send requests.
const MAX_TRACKED_CLIENTS: usize = 100_000;

/// Fixed window rate limiter keyed by client IP address.
#[derive(Debug)]
pub struct RateLimiter {
    /// Ordered by window start: the oldest window is always at the front.
    windows: Mutex<LinkedHashMap<IpAddr, Window>>,
    limit: u64,
    window: Duration,
}

#[derive(Debug, Clone, Copy)]
struct Window {
    start: Instant,
    count: u64,
}

impl RateLimiter {
    pub fn new(limit: u64, window_sec: u64) -> RateLimiter {
        RateLimiter {
            windows: Mutex::new(LinkedHashMap::new()),
            limit,
            window: Duration::from_secs(window_sec.max(1)),
        }
    }

    /// Records a request of `client_ip` and returns whether it is allowed.
    fn register_request(&self, client_ip: IpAddr, now: Instant) -> bool {
        let mut windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());

        // Drop expired windows; the map is ordered by start time, so stop at the first live one
        while let Some((_, window)) = windows.front() {
            if now.saturating_duration_since(window.start) >= self.window {
                windows.pop_front();
            } else {
                break;
            }
        }

        let window = match windows.get_mut(&client_ip) {
            Some(window) => window,
            None => {
                if windows.len() >= MAX_TRACKED_CLIENTS {
                    windows.pop_front();
                }
                // A new window goes to the back, keeping the map ordered by start time
                windows.insert(
                    client_ip,
                    Window {
                        start: now,
                        count: 0,
                    },
                );
                windows
                    .get_mut(&client_ip)
                    .expect("entry was inserted right above")
            }
        };
        // Rejected requests count too, but they never extend the window
        window.count = window.count.saturating_add(1);
        window.count <= self.limit
    }
}

#[async_trait]
impl Middleware for RateLimiter {
    async fn modify_request(
        &self,
        request: Request<Body>,
        context: &Context<'_>,
    ) -> Result<Request<Body>, Response<Body>> {
        if self.register_request(context.client_address.ip(), Instant::now()) {
            Ok(request)
        } else {
            Err(too_many_requests())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_applies_per_ip_not_per_connection() {
        let limiter = RateLimiter::new(2, 60);
        let now = Instant::now();
        let ip: IpAddr = "203.0.113.7".parse().unwrap();
        assert!(limiter.register_request(ip, now));
        assert!(limiter.register_request(ip, now));
        // A new source port must not reset the budget
        assert!(!limiter.register_request(ip, now));
        let other: IpAddr = "203.0.113.8".parse().unwrap();
        assert!(limiter.register_request(other, now));
    }

    #[test]
    fn window_resets_and_is_not_extended_by_rejected_requests() {
        let limiter = RateLimiter::new(1, 10);
        let start = Instant::now();
        let ip: IpAddr = "198.51.100.1".parse().unwrap();
        assert!(limiter.register_request(ip, start));
        assert!(!limiter.register_request(ip, start + Duration::from_secs(5)));
        assert!(!limiter.register_request(ip, start + Duration::from_secs(9)));
        assert!(limiter.register_request(ip, start + Duration::from_secs(10)));
    }

    #[test]
    fn expired_entries_are_evicted() {
        let limiter = RateLimiter::new(5, 1);
        let start = Instant::now();
        for i in 0..100u8 {
            limiter.register_request(IpAddr::from([10, 0, 0, i]), start);
        }
        limiter.register_request(IpAddr::from([10, 0, 1, 1]), start + Duration::from_secs(2));
        assert_eq!(limiter.windows.lock().unwrap().len(), 1);
    }
}
