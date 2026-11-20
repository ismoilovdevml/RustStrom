//! Pre-indexed routing: pools whose matcher requires an exact host are found through a
//! `HashMap`, all others are tried in order. The first matching pool in config order wins,
//! exactly like the linear scan of v1.1.

use crate::{
    matcher::request_host,
    upstream::{Pool, Scheme},
};
use pingora_http::RequestHeader;
use std::{collections::HashMap, sync::Arc};

#[derive(Debug, Default)]
pub struct Router {
    /// Pool indexes (ascending) by the host their matcher requires.
    by_host: HashMap<String, Vec<usize>>,
    /// Pool indexes (ascending) that can match any host.
    fallback: Vec<usize>,
}

impl Router {
    pub fn new(pools: &[Arc<Pool>]) -> Router {
        let mut router = Router::default();
        for (index, pool) in pools.iter().enumerate() {
            match pool.matcher.required_host() {
                Some(host) => router
                    .by_host
                    .entry(host.to_string())
                    .or_default()
                    .push(index),
                None => router.fallback.push(index),
            }
        }
        router
    }

    pub fn route<'a>(
        &self,
        pools: &'a [Arc<Pool>],
        request: &RequestHeader,
        scheme: Scheme,
    ) -> Option<&'a Arc<Pool>> {
        let hosted: &[usize] = request_host(request)
            .and_then(|host| self.by_host.get(host))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        // Merge the two ascending index lists to keep config order
        let (mut h, mut f) = (0, 0);
        loop {
            let index = match (hosted.get(h), self.fallback.get(f)) {
                (Some(&a), Some(&b)) if a < b => {
                    h += 1;
                    a
                }
                (_, Some(&b)) => {
                    f += 1;
                    b
                }
                (Some(&a), None) => {
                    h += 1;
                    a
                }
                (None, None) => return None,
            };
            if let Some(pool) = pools.get(index) {
                if pool.supports(scheme) && pool.matcher.matches(request) {
                    return Some(pool);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        matcher::Matcher,
        middleware::Middlewares,
        upstream::{strategy::Strategy, HealthCheck, HealthKind, PoolParts, UpstreamSettings},
    };
    use std::time::Duration;

    fn pool(matcher: &str, http: bool, https: bool) -> Arc<Pool> {
        Arc::new(Pool::new(PoolParts {
            name: matcher.into(),
            matcher: Matcher::try_from(matcher).unwrap(),
            http,
            https,
            backends: vec![],
            strategy: Strategy::Random,
            health_check: HealthCheck {
                slow_threshold_ms: 300,
                timeout: Duration::from_millis(500),
                path: "/".into(),
                rise: 2,
                fall: 3,
                kind: HealthKind::Http,
                expected_status: (200, 399),
            },
            upstream: UpstreamSettings::default(),
            middlewares: Middlewares::default(),
        }))
    }

    fn req(host: Option<&str>, path: &str) -> RequestHeader {
        let mut req = RequestHeader::build("GET", path.as_bytes(), None).unwrap();
        if let Some(host) = host {
            req.insert_header("Host", host).unwrap();
        }
        req
    }

    #[test]
    fn first_match_in_config_order_wins() {
        let pools = vec![
            pool("PathRegexp('^/first')", true, true),
            pool("Host('a.test')", true, true),
            pool("Host('a.test') && Path('/x')", true, true),
            pool("PathRegexp('^/')", true, true),
        ];
        let router = Router::new(&pools);
        let route = |host, path| {
            router
                .route(&pools, &req(host, path), Scheme::Http)
                .map(|p| p.name.clone())
        };
        assert_eq!(
            route(Some("a.test"), "/first"),
            Some("PathRegexp('^/first')".into())
        );
        assert_eq!(route(Some("a.test"), "/x"), Some("Host('a.test')".into()));
        assert_eq!(route(Some("b.test"), "/x"), Some("PathRegexp('^/')".into()));
        assert_eq!(route(None, "/y"), Some("PathRegexp('^/')".into()));
    }

    #[test]
    fn scheme_filter_and_no_match() {
        let pools = vec![pool("Host('a.test')", false, true)];
        let router = Router::new(&pools);
        assert!(router
            .route(&pools, &req(Some("a.test"), "/"), Scheme::Http)
            .is_none());
        assert!(router
            .route(&pools, &req(Some("a.test"), "/"), Scheme::Https)
            .is_some());
        assert!(router
            .route(&pools, &req(Some("b.test"), "/"), Scheme::Https)
            .is_none());
    }

    #[test]
    fn http2_authority_is_indexed_host() {
        let pools = vec![pool("Host('a.test')", true, true)];
        let router = Router::new(&pools);
        // HTTP/2 requests carry the authority in the URI and no Host header
        let mut h2 = RequestHeader::build("GET", b"/x", None).unwrap();
        h2.set_uri("https://a.test/x".parse().unwrap());
        assert!(router.route(&pools, &h2, Scheme::Https).is_some());
    }
}
