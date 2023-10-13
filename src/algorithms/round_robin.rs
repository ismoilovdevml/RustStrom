use super::config::Backend;
use std::sync::Arc;
use std::sync::Mutex;

pub fn round_robin(backends: &Arc<Mutex<impl Iterator<Item = Backend>>>) -> Option<Backend> {
    backends.lock().unwrap().next().cloned()
}
