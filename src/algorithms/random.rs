use super::{Context, LoadBalancingStrategy, RequestForwarder};
use hyper::{Body, Request};
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub struct RoundRobin {
    rrc: Arc<Mutex<usize>>,
}

impl RoundRobin {
    pub fn new() -> RoundRobin {
        RoundRobin {
            rrc: Arc::new(Mutex::new(0)),
        }
    }
}

impl LoadBalancingStrategy for RoundRobin {
    fn select_backend<'l>(&'l self, _request: &Request<Body>, context: &'l Context) -> RequestForwarder {
        let mut rrc_handle = self.rrc.lock().unwrap();
        *rrc_handle = (*rrc_handle + 1) % context.backend_addresses.len();
        let address = &context.backend_addresses[*rrc_handle];
        RequestForwarder::new(address)
    }
}