use super::{Context, LoadBalancingStrategy, RequestForwarder};
use hyper::{Body, Request};
use std::{
  collections::hash_map::DefaultHasher,
  hash::{Hash, Hasher},
};

#[derive(Debug)]
pub struct IPHash {}

impl IPHash {
  pub fn new() -> IPHash {
    IPHash {}
  }
}

impl LoadBalancingStrategy for IPHash {
  fn select_backend<'l>(&'l self, _request: &Request<Body>, context: &'l Context) -> RequestForwarder {
    let mut hasher = DefaultHasher::new();
    context.client_address.ip().hash(&mut hasher);
    let index = (hasher.finish() % (context.backend_addresses.len() as u64)) as usize;
    let address = &context.backend_addresses[index];
    RequestForwarder::new(address)
  }
}