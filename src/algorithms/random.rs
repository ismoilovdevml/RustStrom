use super::config::Backend;
use rand::seq::SliceRandom;
use std::sync::Arc;
use std::sync::Mutex;

pub fn random_pick(backends: &Arc<Mutex<impl Iterator<Item = Backend>>>) -> Option<Backend> {
    let backends_vec: Vec<Backend> = backends.lock().unwrap().clone().collect();
    backends_vec.choose(&mut rand::thread_rng()).cloned()
}
