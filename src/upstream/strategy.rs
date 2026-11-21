//! Load balancing strategies. Selection works on a pre-built [`Snapshot`] of working backends
//! and does not allocate on the first attempt of a request.

use super::{Backend, Snapshot};
use cookie::SameSite;
use fnv::FnvHasher;
use http::header::COOKIE;
use pingora_http::RequestHeader;
use rand::Rng;
use std::{
    hash::{Hash, Hasher},
    net::IpAddr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

#[derive(Debug)]
pub enum Strategy {
    RoundRobin(AtomicUsize),
    Random,
    IPHash,
    LeastConnection,
    StickyCookie(Box<StickyCookie>),
}

#[derive(Debug)]
pub struct StickyCookie {
    pub cookie_name: String,
    pub http_only: bool,
    pub secure: bool,
    pub same_site: SameSite,
    pub inner: Strategy,
}

/// The outcome of a selection.
pub struct Selection {
    pub backend: Arc<Backend>,
    /// The client must be (re)issued a sticky cookie for this backend.
    pub set_sticky_cookie: bool,
}

impl Strategy {
    pub fn needs_ring(&self) -> bool {
        match self {
            Strategy::IPHash => true,
            Strategy::StickyCookie(sticky) => sticky.inner.needs_ring(),
            _ => false,
        }
    }

    pub fn sticky(&self) -> Option<&StickyCookie> {
        match self {
            Strategy::StickyCookie(sticky) => Some(sticky),
            _ => None,
        }
    }

    /// Picks a backend from `snapshot`, avoiding the ones in `tried` unless all were tried.
    pub fn select(
        &self,
        snapshot: &Snapshot,
        request: &RequestHeader,
        client_ip: Option<IpAddr>,
        tried: &[Arc<Backend>],
    ) -> Option<Selection> {
        let backends = &snapshot.backends;
        if backends.is_empty() {
            return None;
        }
        // Once every backend was tried, any of them may be used again (e.g. a stale keep-alive
        // connection to the only backend).
        let all_tried = backends.iter().all(|b| is_tried(tried, b));
        let excluded = |b: &Arc<Backend>| !all_tried && is_tried(tried, b);

        match self {
            Strategy::StickyCookie(sticky) => {
                let pinned = sticky_cookie_value(request, &sticky.cookie_name).and_then(|value| {
                    backends
                        .iter()
                        .find(|backend| backend.id == value && !excluded(backend))
                });
                match pinned {
                    Some(backend) => Some(Selection {
                        backend: backend.clone(),
                        set_sticky_cookie: false,
                    }),
                    None => sticky
                        .inner
                        .select_plain(snapshot, client_ip, &excluded)
                        .map(|backend| Selection {
                            backend,
                            set_sticky_cookie: true,
                        }),
                }
            }
            other => other
                .select_plain(snapshot, client_ip, &excluded)
                .map(|backend| Selection {
                    backend,
                    set_sticky_cookie: false,
                }),
        }
    }

    fn select_plain(
        &self,
        snapshot: &Snapshot,
        client_ip: Option<IpAddr>,
        excluded: &dyn Fn(&Arc<Backend>) -> bool,
    ) -> Option<Arc<Backend>> {
        let backends = &snapshot.backends;
        let len = backends.len();
        match self {
            Strategy::RoundRobin(counter) => {
                let start = counter.fetch_add(1, Ordering::Relaxed);
                probe(backends, start, excluded)
            }
            Strategy::Random => {
                let start = rand::thread_rng().gen_range(0..len);
                probe(backends, start, excluded)
            }
            Strategy::IPHash => {
                let key = match client_ip {
                    Some(ip) => hash_ip(ip),
                    None => 0,
                };
                match &snapshot.ring {
                    Some(ring) => ring
                        .walk(key)
                        .filter_map(|index| backends.get(index))
                        .find(|backend| !excluded(backend))
                        .cloned(),
                    None => probe(
                        backends,
                        usize::try_from(key % len as u64).unwrap_or(0),
                        excluded,
                    ),
                }
            }
            Strategy::LeastConnection => least_connection(backends, excluded),
            // A sticky cookie as `inner` behaves like its own inner strategy.
            Strategy::StickyCookie(sticky) => {
                sticky.inner.select_plain(snapshot, client_ip, excluded)
            }
        }
    }
}

fn is_tried(tried: &[Arc<Backend>], backend: &Arc<Backend>) -> bool {
    tried.iter().any(|t| Arc::ptr_eq(t, backend))
}

/// The first backend from `start` (wrapping around) that is not excluded.
fn probe(
    backends: &[Arc<Backend>],
    start: usize,
    excluded: &dyn Fn(&Arc<Backend>) -> bool,
) -> Option<Arc<Backend>> {
    let len = backends.len();
    (0..len)
        .filter_map(|offset| backends.get(start.wrapping_add(offset) % len))
        .find(|backend| !excluded(backend))
        .cloned()
}

/// Power of two choices on in-flight requests: two random candidates, the less loaded wins.
fn least_connection(
    backends: &[Arc<Backend>],
    excluded: &dyn Fn(&Arc<Backend>) -> bool,
) -> Option<Arc<Backend>> {
    let len = backends.len();
    let mut rng = rand::thread_rng();
    if len >= 2 {
        let first = rng.gen_range(0..len);
        let mut second = rng.gen_range(0..len - 1);
        if second >= first {
            second += 1;
        }
        if let (Some(a), Some(b)) = (backends.get(first), backends.get(second)) {
            match (excluded(a), excluded(b)) {
                (false, false) => {
                    let winner = if b.inflight() < a.inflight() { b } else { a };
                    return Some(winner.clone());
                }
                (false, true) => return Some(a.clone()),
                (true, false) => return Some(b.clone()),
                (true, true) => {}
            }
        }
    }
    // Retries or a single backend: the least loaded among the remaining ones
    backends
        .iter()
        .filter(|backend| !excluded(backend))
        .min_by_key(|backend| backend.inflight())
        .cloned()
}

fn hash_ip(ip: IpAddr) -> u64 {
    let mut hasher = FnvHasher::default();
    ip.hash(&mut hasher);
    mix(hasher.finish())
}

/// splitmix64 finalizer: spreads FNV output evenly over the ring.
fn mix(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Returns the value of the sticky cookie, if the client sent one.
///
/// Parses every `Cookie` header as `name=value` pairs separated by `;`.
/// Malformed input is ignored instead of causing a panic.
pub fn sticky_cookie_value<'a>(request: &'a RequestHeader, cookie_name: &str) -> Option<&'a str> {
    request
        .headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|header| header.to_str().ok())
        .flat_map(|header| header.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == cookie_name)
        .map(|(_, value)| value.trim().trim_matches('"'))
}

/// An opaque, stable identifier for a backend so that the cookie does not reveal internal IP
/// addresses and ports to clients. Same function as v1.1, so existing cookies stay valid.
pub fn backend_id(backend_address: &str) -> String {
    let mut hasher = FnvHasher::default();
    hasher.write(b"ruststrom-sticky:");
    hasher.write(backend_address.as_bytes());
    format!("{:016x}", hasher.finish())
}

impl StickyCookie {
    pub fn set_cookie_header(&self, backend: &Backend) -> String {
        cookie::Cookie::build(self.cookie_name.as_str(), backend.id.as_str())
            .path("/")
            .http_only(self.http_only)
            .secure(self.secure)
            .same_site(self.same_site)
            .finish()
            .to_string()
    }
}

/// Consistent hash ring (Ketama style): every backend owns many points on a 64 bit ring, a key
/// maps to the next point clockwise. Adding or removing a backend moves only ~1/N of the keys.
#[derive(Debug)]
pub struct Ring {
    points: Vec<(u64, usize)>,
}

const POINTS_PER_BACKEND: u32 = 160;

impl Ring {
    pub fn new(backends: &[Arc<Backend>]) -> Ring {
        let mut points = Vec::with_capacity(backends.len() * POINTS_PER_BACKEND as usize);
        for (index, backend) in backends.iter().enumerate() {
            for replica in 0..POINTS_PER_BACKEND {
                let mut hasher = FnvHasher::default();
                hasher.write(backend.address.as_bytes());
                hasher.write(b"-");
                hasher.write(&replica.to_le_bytes());
                points.push((mix(hasher.finish()), index));
            }
        }
        points.sort_unstable();
        Ring { points }
    }

    /// Backend indexes in ring order starting at `key`; each backend at most once.
    pub fn walk(&self, key: u64) -> impl Iterator<Item = usize> + '_ {
        let start = self.points.partition_point(|(point, _)| *point < key);
        let len = self.points.len();
        let mut first: Option<usize> = None;
        // Only used when the first backend is excluded (retries), so the common case never
        // allocates
        let mut seen: Vec<usize> = Vec::new();
        (0..len).filter_map(move |offset| {
            let (_, index) = *self.points.get((start + offset) % len)?;
            match first {
                None => {
                    first = Some(index);
                    return Some(index);
                }
                Some(first) if first == index => return None,
                Some(_) => {}
            }
            if seen.contains(&index) {
                None
            } else {
                seen.push(index);
                Some(index)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upstream::test_backend;
    use std::collections::HashMap;

    fn snapshot(addresses: &[&str], ring: bool) -> Snapshot {
        let backends: Vec<_> = addresses.iter().map(|a| test_backend(a)).collect();
        Snapshot::new(backends, ring)
    }

    fn get(path: &str) -> RequestHeader {
        RequestHeader::build("GET", path.as_bytes(), None).unwrap()
    }

    #[test]
    fn round_robin_cycles() {
        let snap = snapshot(&["a:1", "b:1", "c:1"], false);
        let strategy = Strategy::RoundRobin(AtomicUsize::new(0));
        let req = get("/");
        let picks: Vec<_> = (0..6)
            .map(|_| {
                strategy
                    .select(&snap, &req, None, &[])
                    .unwrap()
                    .backend
                    .address
                    .clone()
            })
            .collect();
        assert_eq!(picks, ["a:1", "b:1", "c:1", "a:1", "b:1", "c:1"]);
    }

    #[test]
    fn retry_avoids_tried_backend() {
        let snap = snapshot(&["a:1", "b:1"], false);
        let req = get("/");
        for strategy in [
            Strategy::RoundRobin(AtomicUsize::new(0)),
            Strategy::Random,
            Strategy::LeastConnection,
            Strategy::IPHash,
        ] {
            for _ in 0..20 {
                let first = strategy
                    .select(&snap, &req, Some("10.0.0.1".parse().unwrap()), &[])
                    .unwrap()
                    .backend;
                let second = strategy
                    .select(
                        &snap,
                        &req,
                        Some("10.0.0.1".parse().unwrap()),
                        std::slice::from_ref(&first),
                    )
                    .unwrap()
                    .backend;
                assert!(!Arc::ptr_eq(&first, &second), "{:?}", strategy);
            }
        }
    }

    #[test]
    fn all_tried_falls_back_to_any() {
        let snap = snapshot(&["a:1"], false);
        let tried = vec![snap.backends[0].clone()];
        let pick = Strategy::Random.select(&snap, &get("/"), None, &tried);
        assert!(pick.is_some());
    }

    #[test]
    fn empty_snapshot_selects_nothing() {
        let snap = snapshot(&[], true);
        assert!(Strategy::IPHash
            .select(&snap, &get("/"), None, &[])
            .is_none());
        assert!(Strategy::LeastConnection
            .select(&snap, &get("/"), None, &[])
            .is_none());
    }

    #[test]
    fn ip_hash_is_stable_and_consistent() {
        let three = snapshot(&["a:1", "b:1", "c:1"], true);
        let four = snapshot(&["a:1", "b:1", "c:1", "d:1"], true);
        let req = get("/");
        let mut moved = 0;
        let total = 2000;
        for i in 0..total {
            let ip: IpAddr = format!("10.{}.{}.{}", i / 65536, (i / 256) % 256, i % 256)
                .parse()
                .unwrap();
            let a = Strategy::IPHash
                .select(&three, &req, Some(ip), &[])
                .unwrap()
                .backend;
            let again = Strategy::IPHash
                .select(&three, &req, Some(ip), &[])
                .unwrap()
                .backend;
            assert_eq!(a.address, again.address);
            let b = Strategy::IPHash
                .select(&four, &req, Some(ip), &[])
                .unwrap()
                .backend;
            if a.address != b.address {
                assert_eq!(b.address, "d:1", "a key moved between surviving backends");
                moved += 1;
            }
        }
        // ~1/4 of the keys move to the new backend
        assert!(moved > total / 8 && moved < total / 2, "moved {}", moved);
    }

    #[test]
    fn ip_hash_spreads_keys() {
        let snap = snapshot(&["a:1", "b:1", "c:1"], true);
        let req = get("/");
        let mut counts: HashMap<String, usize> = HashMap::new();
        for i in 0..3000u32 {
            let ip = IpAddr::from(i.to_be_bytes());
            let pick = Strategy::IPHash.select(&snap, &req, Some(ip), &[]).unwrap();
            *counts.entry(pick.backend.address.clone()).or_default() += 1;
        }
        for (address, count) in counts {
            assert!(count > 600, "{} got only {}", address, count);
        }
    }

    #[test]
    fn least_connection_prefers_idle_backend() {
        let snap = snapshot(&["busy:1", "idle:1"], false);
        let _guards: Vec<_> = (0..5).map(|_| snap.backends[0].track_inflight()).collect();
        for _ in 0..50 {
            let pick = Strategy::LeastConnection
                .select(&snap, &get("/"), None, &[])
                .unwrap();
            assert_eq!(pick.backend.address, "idle:1");
        }
    }

    #[test]
    fn sticky_cookie_pins_and_falls_back() {
        let snap = snapshot(&["a:1", "b:1"], false);
        let strategy = Strategy::StickyCookie(Box::new(StickyCookie {
            cookie_name: "S".into(),
            http_only: true,
            secure: false,
            same_site: SameSite::Lax,
            inner: Strategy::RoundRobin(AtomicUsize::new(0)),
        }));
        let mut req = get("/");
        let fresh = strategy.select(&snap, &req, None, &[]).unwrap();
        assert!(fresh.set_sticky_cookie);

        let id = backend_id("b:1");
        req.insert_header("Cookie", format!("x=1; S={}", id))
            .unwrap();
        for _ in 0..5 {
            let pinned = strategy.select(&snap, &req, None, &[]).unwrap();
            assert_eq!(pinned.backend.address, "b:1");
            assert!(!pinned.set_sticky_cookie);
        }

        // A raw address (pre 1.1 cookie) or garbage is ignored, never a panic
        let mut bad = get("/");
        bad.insert_header("Cookie", "S=127.0.0.1:8080; S").unwrap();
        assert!(
            strategy
                .select(&snap, &bad, None, &[])
                .unwrap()
                .set_sticky_cookie
        );
    }

    #[test]
    fn sticky_cookie_header_is_opaque() {
        let sticky = StickyCookie {
            cookie_name: "RUSTSTROM_SESSION".into(),
            http_only: true,
            secure: true,
            same_site: SameSite::Strict,
            inner: Strategy::Random,
        };
        let backend = test_backend("127.0.0.1:8080");
        let header = sticky.set_cookie_header(&backend);
        assert!(header.contains("Path=/"));
        assert!(header.contains("HttpOnly"));
        assert!(header.contains("Secure"));
        assert!(!header.contains("127.0.0.1"));
        assert_eq!(backend_id("127.0.0.1:8080").len(), 16);
    }
}
