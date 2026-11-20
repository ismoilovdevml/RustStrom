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
            let mut connections = self.connections.write().unwrap_or_else(|e| e.into_inner());
            *connections.entry(authority.to_string()).or_insert(0) += 1;
        }
    }

    fn on_tcp_close(&self, remote: &Uri) {
        if let Some(authority) = remote.authority() {
            let mut connections = self.connections.write().unwrap_or_else(|e| e.into_inner());
            if let Some(count) = connections.get_mut(authority.as_str()) {
                *count = count.saturating_sub(1);
            }
        }
    }

    fn select_backend<'l>(
        &'l self,
        _request: &Request<Body>,
        context: &'l Context<'l>,
    ) -> RequestForwarder<'l> {
        let connections = self.connections.read().unwrap_or_else(|e| e.into_inner());

        // Pick uniformly at random among all addresses sharing the lowest count,
        // so idle backends do not all funnel to the first address in the list.
        let mut least_count = usize::MAX;
        let mut candidates = 0usize;
        let mut chosen = context.backend_addresses[0];
        let mut rng = thread_rng();
        for address in context.backend_addresses.iter() {
            let count = connections.get(*address).copied().unwrap_or(0);
            if count < least_count {
                least_count = count;
                candidates = 1;
                chosen = address;
            } else if count == least_count {
                candidates += 1;
                // Reservoir sampling: keep each tied candidate with probability 1/n
                if rng.gen_range(0..candidates) == 0 {
                    chosen = address;
                }
            }
        }
        RequestForwarder::new(chosen)
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

#[cfg(test)]
mod underflow_tests {
    use super::*;

    #[test]
    fn close_without_open_does_not_underflow() {
        let strategy = LeastConnection::new();
        strategy.on_tcp_open(&"127.0.0.1:1".parse().unwrap());
        strategy.on_tcp_close(&"127.0.0.1:1".parse().unwrap());
        strategy.on_tcp_close(&"127.0.0.1:1".parse().unwrap());
        let connections = strategy.connections.read().unwrap();
        assert_eq!(connections.get("127.0.0.1:1"), Some(&0));
    }
}
