use super::super::config::Backend;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub fn ip_hash(client_ip: IpAddr, backends: &Arc<Mutex<impl Iterator<Item = Backend>>>) -> Option<Backend> {
    // Generate a consistent hash of the client's IP address
    let mut hasher = DefaultHasher::new();
    client_ip.hash(&mut hasher);
    let hash_val = hasher.finish();

    // Get a consistent backend based on the hash value
    let backends_vec: Vec<Backend> = backends.lock().unwrap().clone().collect();
    let idx = (hash_val % backends_vec.len() as u64) as usize;

    Some(backends_vec[idx].clone())
}
