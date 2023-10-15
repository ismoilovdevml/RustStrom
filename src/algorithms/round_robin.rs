use super::{Context, LoadBalancingStrategy, RequestForwarder};
use hyper::{Body, Request};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug)]
pub struct RoundRobin {
  rrc: AtomicUsize,
}

impl RoundRobin {
  pub fn new() -> RoundRobin {
    RoundRobin {
      rrc: AtomicUsize::new(0),
    }
  }
}

impl LoadBalancingStrategy for RoundRobin {
  fn select_backend<'l>(&'l self, _request: &Request<Body>, context: &'l Context) -> RequestForwarder {
    let len = context.backend_addresses.len();
    if len == 0 {
        panic!("No backend addresses provided");
    }

    let idx = self.rrc.fetch_add(1, Ordering::Relaxed) % len;
    let address = &context.backend_addresses[idx];
    RequestForwarder::new(address)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  pub fn round_robin_strategy_single_address() {
    let request = Request::builder().body(Body::empty()).unwrap();
    let address = "127.0.0.1:1";
    let context = Context {
      client_address: &"127.0.0.1:3000".parse().unwrap(),
      backend_addresses: &mut [address],
    };
    let strategy = RoundRobin::new();

    assert_eq!(strategy.select_backend(&request, &context).backend_address, address);
    assert_eq!(strategy.select_backend(&request, &context).backend_address, address);
    assert_eq!(strategy.select_backend(&request, &context).backend_address, address);
  }

  #[test]
  pub fn round_robin_strategy_multiple_addresses() {
    let request = Request::builder().body(Body::empty()).unwrap();
    let address_1 = "127.0.0.1:1";
    let address_2 = "127.0.0.1:2";
    let context = Context {
      client_address: &"127.0.0.1:3000".parse().unwrap(),
      backend_addresses: &mut [address_1, address_2],
    };
    let strategy = RoundRobin::new();

    assert_eq!(strategy.select_backend(&request, &context).backend_address, address_1);
    assert_eq!(strategy.select_backend(&request, &context).backend_address, address_2);
    assert_eq!(strategy.select_backend(&request, &context).backend_address, address_1);
    assert_eq!(strategy.select_backend(&request, &context).backend_address, address_2);
  }
}