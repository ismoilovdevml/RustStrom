use super::{Context, LoadBalancingStrategy, RequestForwarder};
use cookie::{Cookie, SameSite};
use fnv::FnvHasher;
use hyper::{
    header::{HeaderValue, COOKIE, SET_COOKIE},
    Body, Request, Response, Uri,
};
use log::warn;
use std::hash::Hasher;

#[derive(Debug)]
pub struct StickyCookie {
    pub cookie_name: String,
    pub inner: Box<dyn LoadBalancingStrategy>,
    pub http_only: bool,
    pub secure: bool,
    pub same_site: SameSite,
}

impl StickyCookie {
    pub fn new(
        cookie_name: String,
        inner: Box<dyn LoadBalancingStrategy>,
        http_only: bool,
        secure: bool,
        same_site: SameSite,
    ) -> StickyCookie {
        StickyCookie {
            cookie_name,
            inner,
            http_only,
            secure,
            same_site,
        }
    }

    /// Returns the value of the sticky cookie, if the client sent one.
    ///
    /// Parses every `Cookie` header as `name=value` pairs separated by `;`.
    /// Malformed input is ignored instead of causing a panic.
    fn sticky_cookie_value<'a>(&self, request: &'a Request<Body>) -> Option<&'a str> {
        request
            .headers()
            .get_all(COOKIE)
            .iter()
            .filter_map(|header| header.to_str().ok())
            .flat_map(|header| header.split(';'))
            .filter_map(|pair| pair.trim().split_once('='))
            .find(|(name, _)| *name == self.cookie_name)
            .map(|(_, value)| value.trim().trim_matches('"'))
    }

    fn modify_response(
        &self,
        mut response: Response<Body>,
        backend_address: &str,
    ) -> Response<Body> {
        let cookie = Cookie::build(self.cookie_name.as_str(), backend_id(backend_address))
            .path("/")
            .http_only(self.http_only)
            .secure(self.secure)
            .same_site(self.same_site)
            .finish();

        match HeaderValue::from_str(&cookie.to_string()) {
            Ok(cookie_val) => {
                response.headers_mut().append(SET_COOKIE, cookie_val);
            }
            Err(e) => warn!("Could not set sticky cookie '{}': {}", self.cookie_name, e),
        }

        response
    }
}

/// An opaque, stable identifier for a backend so that the cookie does not
/// reveal internal IP addresses and ports to clients.
fn backend_id(backend_address: &str) -> String {
    let mut hasher = FnvHasher::default();
    hasher.write(b"ruststrom-sticky:");
    hasher.write(backend_address.as_bytes());
    format!("{:016x}", hasher.finish())
}

impl LoadBalancingStrategy for StickyCookie {
    fn select_backend<'l>(
        &'l self,
        request: &Request<Body>,
        context: &'l Context<'l>,
    ) -> RequestForwarder<'l> {
        let backend_address = self.sticky_cookie_value(request).and_then(|value| {
            context
                .backend_addresses
                .iter()
                .find(|address| backend_id(address) == value)
        });

        if let Some(backend_address) = backend_address {
            RequestForwarder::new(backend_address)
        } else {
            let backend = self.inner.select_backend(request, context);
            let backend_address = backend.backend_address;
            backend.map_response(move |response| self.modify_response(response, backend_address))
        }
    }

    fn on_tcp_open(&self, remote: &Uri) {
        self.inner.on_tcp_open(remote);
    }

    fn on_tcp_close(&self, remote: &Uri) {
        self.inner.on_tcp_close(remote);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algorithms::round_robin::RoundRobin;

    fn strategy() -> StickyCookie {
        StickyCookie::new(
            "RUSTSTROM_SESSION".into(),
            Box::new(RoundRobin::new()),
            true,
            true,
            SameSite::Strict,
        )
    }

    fn request_with_cookie(cookie: &str) -> Request<Body> {
        Request::builder()
            .header(COOKIE, cookie)
            .body(Body::empty())
            .unwrap()
    }

    #[test]
    fn cookie_without_trailing_semicolon_does_not_panic() {
        // Regression: this exact header used to slice out of bounds and abort the process
        let request = request_with_cookie("RUSTSTROM_SESSION=127.0.0.1:8080");
        assert_eq!(
            strategy().sticky_cookie_value(&request),
            Some("127.0.0.1:8080")
        );
    }

    #[test]
    fn malformed_cookie_headers_do_not_panic() {
        for header in [
            "",
            ";",
            "RUSTSTROM_SESSION",
            "RUSTSTROM_SESSION=",
            "a=b; RUSTSTROM_SESSION",
            "x=1;;;RUSTSTROM_SESSION=;",
            "RUSTSTROM_SESSION=ä",
        ] {
            let request = request_with_cookie(header);
            let _ = strategy().sticky_cookie_value(&request);
        }
    }

    #[test]
    fn selects_backend_from_cookie() {
        let backends = ["127.0.0.1:1", "127.0.0.1:2", "127.0.0.1:3"];
        let context = Context {
            client_address: &"127.0.0.1:3000".parse().unwrap(),
            backend_addresses: &backends,
        };
        let cookie = format!("other=1; RUSTSTROM_SESSION={}", backend_id("127.0.0.1:3"));
        let request = request_with_cookie(&cookie);
        assert_eq!(
            strategy()
                .select_backend(&request, &context)
                .backend_address,
            "127.0.0.1:3"
        );
    }

    #[test]
    fn set_cookie_hides_backend_address_and_sets_path() {
        let response = strategy().modify_response(Response::new(Body::empty()), "10.0.0.5:8080");
        let set_cookie = response.headers()[SET_COOKIE].to_str().unwrap();
        assert!(!set_cookie.contains("10.0.0.5"));
        assert!(set_cookie.contains("Path=/"));
    }
}
