use super::{Context, LoadBalancingStrategy, RequestForwarder};
use fnv::FnvHasher;
use hyper::{Body, Request};
use std::hash::{Hash, Hasher};

#[derive(Debug)]
pub struct IPHash {}

impl IPHash {
    pub fn new() -> IPHash {
        IPHash {}
    }
}

impl LoadBalancingStrategy for IPHash {
    fn select_backend<'l>(
        &'l self,
        _request: &Request<Body>,
        context: &'l Context,
    ) -> RequestForwarder {
        let mut hasher = FnvHasher::default();
        context.client_address.ip().hash(&mut hasher);
        let index = (hasher.finish() % (context.backend_addresses.len() as u64)) as usize;
        let address = &context.backend_addresses[index];
        RequestForwarder::new(address)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn ip_hash_strategy_same_ip() {
        let request = Request::builder().body(Body::empty()).unwrap();
        let context = Context {
            client_address: &"127.0.0.1:3000".parse().unwrap(),
            backend_addresses: &mut ["127.0.0.1:1", "127.0.0.1:2"],
        };
        let strategy = IPHash::new();

        let address = strategy.select_backend(&request, &context).backend_address;
        assert_eq!(
            strategy.select_backend(&request, &context).backend_address,
            address
        );
        assert_eq!(
            strategy.select_backend(&request, &context).backend_address,
            address
        );
        assert_eq!(
            strategy.select_backend(&request, &context).backend_address,
            address
        );
        assert_eq!(
            strategy.select_backend(&request, &context).backend_address,
            address
        );
    }

    #[test]
    pub fn ip_hash_strategy_different_ip() {
        let strategy = IPHash::new();

        // Test that different IPs can produce different backends
        // Store addresses to ensure they live long enough
        let addresses = [
            "192.168.0.1:3000".parse().unwrap(),
            "192.168.0.2:3000".parse().unwrap(),
            "192.168.0.3:3000".parse().unwrap(),
            "192.168.0.4:3000".parse().unwrap(),
            "192.168.0.5:3000".parse().unwrap(),
            "192.168.0.6:3000".parse().unwrap(),
            "192.168.0.7:3000".parse().unwrap(),
            "192.168.0.8:3000".parse().unwrap(),
        ];

        let backend_addresses = &["127.0.0.1:1", "127.0.0.1:2", "127.0.0.1:3", "127.0.0.1:4"];
        let mut results = std::collections::HashSet::new();

        // Test several different IPs
        for addr in &addresses {
            let request = Request::builder().body(Body::empty()).unwrap();
            let context = Context {
                client_address: addr,
                backend_addresses,
            };
            let backend = strategy.select_backend(&request, &context).backend_address;
            results.insert(backend.to_string());
        }

        // With 8 different IPs and 4 backends, we should get at least 2 different backends
        assert!(results.len() >= 2, "Expected at least 2 different backends, got {}", results.len());
    }
}
