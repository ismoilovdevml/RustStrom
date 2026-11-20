//! Fixed window rate limiter keyed by client IP (v1.1 semantics), lock-sharded so that cores do
//! not contend on one mutex, with a bounded number of tracked clients.

use fnv::FnvHasher;
use linked_hash_map::LinkedHashMap;
use std::{
    hash::{Hash, Hasher},
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

/// Upper bound of tracked clients, protects against memory exhaustion when many distinct
/// addresses (e.g. a whole IPv6 prefix) send requests.
const MAX_TRACKED_CLIENTS: usize = 100_000;
const SHARDS: usize = 64;
const PER_SHARD: usize = MAX_TRACKED_CLIENTS / SHARDS;

#[derive(Debug)]
pub struct RateLimiter {
    /// Each shard is ordered by window start: the oldest window is always at the front.
    shards: Vec<Mutex<LinkedHashMap<IpAddr, Window>>>,
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
            shards: (0..SHARDS)
                .map(|_| Mutex::new(LinkedHashMap::new()))
                .collect(),
            limit,
            window: Duration::from_secs(window_sec.max(1)),
        }
    }

    /// Records a request of `client_ip` and returns whether it is allowed.
    pub fn register_request(&self, client_ip: IpAddr, now: Instant) -> bool {
        let mut hasher = FnvHasher::default();
        client_ip.hash(&mut hasher);
        let index = usize::try_from(hasher.finish() % SHARDS as u64).unwrap_or(0);
        let Some(shard) = self.shards.get(index) else {
            return true;
        };
        let mut windows = shard.lock().unwrap_or_else(|e| e.into_inner());

        // Drop expired windows; the map is ordered by start time, so stop at the first live one
        while let Some((_, window)) = windows.front() {
            if now.saturating_duration_since(window.start) >= self.window {
                windows.pop_front();
            } else {
                break;
            }
        }

        if !windows.contains_key(&client_ip) {
            if windows.len() >= PER_SHARD {
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
        }
        match windows.get_mut(&client_ip) {
            Some(window) => {
                // Rejected requests count too, but they never extend the window
                window.count = window.count.saturating_add(1);
                window.count <= self.limit
            }
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_per_ip_and_window() {
        let limiter = RateLimiter::new(2, 1);
        let a: IpAddr = "10.0.0.1".parse().unwrap();
        let b: IpAddr = "10.0.0.2".parse().unwrap();
        let now = Instant::now();
        assert!(limiter.register_request(a, now));
        assert!(limiter.register_request(a, now));
        assert!(!limiter.register_request(a, now));
        assert!(limiter.register_request(b, now));
        let later = now + Duration::from_millis(1001);
        assert!(limiter.register_request(a, later));
    }

    #[test]
    fn memory_is_bounded() {
        let limiter = RateLimiter::new(1, 3600);
        let now = Instant::now();
        for i in 0..(MAX_TRACKED_CLIENTS as u32 + 5000) {
            limiter.register_request(IpAddr::from(i.to_be_bytes()), now);
        }
        let tracked: usize = limiter.shards.iter().map(|s| s.lock().unwrap().len()).sum();
        assert!(tracked <= MAX_TRACKED_CLIENTS);
    }
}
