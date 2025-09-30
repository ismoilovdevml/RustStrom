use super::{Context, LoadBalancingStrategy, RequestForwarder};
use hyper::{Body, Request, Uri};
use rand::{thread_rng, Rng};
use std::{collections::HashMap, sync::RwLock};

#[derive(Debug)]
pub struct LeastConnection {
    connections: RwLock<HashMap<String, usize>>,
}

impl LeastConnection {
    pub fn new() -> LeastConnection {
        LeastConnection {
            connections: RwLock::new(HashMap::new()),
        }
    }
}

impl LoadBalancingStrategy for LeastConnection {
    fn on_tcp_open(&self, remote: &Uri) {
        if let Some(authority) = remote.authority() {
            let mut connections = self.connections.write().unwrap();
            *connections.entry(authority.to_string()).or_insert(0) += 1;
        }
    }

    fn on_tcp_close(&self, remote: &Uri) {
        if let Some(authority) = remote.authority() {
            let mut connections = self.connections.write().unwrap();
            if let Some(count) = connections.get_mut(&authority.to_string()) {
                *count -= 1;
            }
        }
    }

    fn select_backend<'l>(
        &'l self,
        _request: &Request<Body>,
        context: &'l Context<'l>,
    ) -> RequestForwarder<'l> {
        let connections = self.connections.read().unwrap();

        // Find the address with the least number of connections.
        let mut least_connection_address = None;
        let mut least_connection_count = usize::MAX;
        for address in context.backend_addresses.iter() {
            let connection_count = *connections.get(*address).unwrap_or(&0);
            if connection_count < least_connection_count {
                least_connection_address = Some(address);
                least_connection_count = connection_count;
            }
        }

        if let Some(address) = least_connection_address {
            RequestForwarder::new(address)
        } else {
            // Fall back to random if there's a tie.
            let mut rng = thread_rng();
            let index = rng.gen_range(0..context.backend_addresses.len());
            RequestForwarder::new(&context.backend_addresses[index])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn least_connection_single_least_address() {
        let request = Request::builder().body(Body::empty()).unwrap();

        let context = Context {
            client_address: &"127.0.0.1:3000".parse().unwrap(),
            backend_addresses: &["127.0.0.1:1", "127.0.0.1:2"],
        };

        let strategy = LeastConnection::new();

        strategy.on_tcp_open(&"127.0.0.1:1".parse().unwrap());

        assert_eq!(
            strategy.select_backend(&request, &context).backend_address,
            context.backend_addresses[1]
        );
    }

    #[test]
    pub fn least_connection_multiple_least_addresses() {
        let request = Request::builder().body(Body::empty()).unwrap();

        let context = Context {
            client_address: &"127.0.0.1:3000".parse().unwrap(),
            backend_addresses: &["127.0.0.1:1", "127.0.0.1:2", "127.0.0.1:3"],
        };

        let strategy = LeastConnection::new();
        strategy.on_tcp_open(&"127.0.0.1:1".parse().unwrap());

        assert_ne!(
            strategy.select_backend(&request, &context).backend_address,
            context.backend_addresses[0]
        );
        assert_ne!(
            strategy.select_backend(&request, &context).backend_address,
            context.backend_addresses[0]
        );
        assert_ne!(
            strategy.select_backend(&request, &context).backend_address,
            context.backend_addresses[0]
        );
    }
}
