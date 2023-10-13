use super::super::config::Backend;
use std::sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}};
use std::collections::HashMap;

pub fn least_connection(backends: &Arc<Mutex<impl Iterator<Item = Backend>>>) -> Option<Backend> {
    // Placeholder: This would typically query a shared state (or external system) to find out the current connection count for each backend.
    // For simplicity, let's use a dummy hashmap here. 
    // In a real-world scenario, you'd replace this with a more dynamic system.
    let connection_counts = dummy_backend_connections();

    let backends_vec: Vec<Backend> = backends.lock().unwrap().clone().collect();

    // Find the backend with the least connections
    backends_vec.iter().min_by_key(|backend| {
        connection_counts.get(&backend.ip).unwrap_or(&0)
    }).cloned()
}

// This is a dummy function to simulate the number of connections for each backend.
// In a real-world scenario, you'd replace this with a system that can provide real-time counts.
fn dummy_backend_connections() -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    counts.insert("100.0.0.2".to_string(), 5);
    // ... add other backends and their dummy connection counts here ...

    counts
}
