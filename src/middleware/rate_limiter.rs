use super::{Context, Middleware};
use crate::error_response::too_many_requests;
use async_trait::async_trait;
use hyper::{Body, Request, Response};
use linked_hash_map::LinkedHashMap;
use std::net::SocketAddr;
use tokio::{sync::Mutex, time::Instant};

#[derive(Debug)]
pub struct RateLimiter {
    connections: Mutex<LinkedHashMap<SocketAddr, (u64, Instant)>>,
    limit: u64,
    window_sec: u64,
}

impl RateLimiter {
    pub fn new(limit: u64, window_sec: u64) -> RateLimiter {
        RateLimiter {
            connections: Mutex::new(LinkedHashMap::new()),
            limit,
            window_sec,
        }
    }

    async fn register_request(&self, client_address: &SocketAddr) -> bool {
        let mut connections = self.connections.lock().await;
        let now = Instant::now();

        let old_entries = connections
            .iter()
            .take_while(|(_client_address, (_count, time))| {
                now.duration_since(*time).as_secs() > self.window_sec
            })
            .map(|(client_address, _)| *client_address)
            .collect::<Vec<_>>();
        for client_address in old_entries {
            connections.remove(&client_address);
        }

        let count = connections.entry(*client_address).or_insert((0, now));
        count.0 += 1;
        count.1 = now;

        count.0 <= self.limit
    }
}

#[async_trait]
impl Middleware for RateLimiter {
    async fn modify_request(
        &self,
        request: Request<Body>,
        context: &Context<'_>,
    ) -> Result<Request<Body>, Response<Body>> {
        if self.register_request(context.client_address).await {
            Ok(request)
        } else {
            Err(too_many_requests())
        }
    }
}
