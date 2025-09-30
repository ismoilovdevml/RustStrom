use super::{Context, LoadBalancingStrategy, RequestForwarder};
use async_trait::async_trait;
use cookie::{Cookie, SameSite};
use hyper::{
    header::{HeaderValue, COOKIE, SET_COOKIE},
    Body, Request, Response,
};

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

    fn try_parse_sticky_cookie<'a>(&self, request: &'a Request<Body>) -> Option<Cookie<'a>> {
        let cookie_header = request.headers().get(COOKIE)?;

        let needle = format!("{}=", self.cookie_name);
        if let Some(start) = cookie_header.to_str().ok()?.find(&needle) {
            let value_start = start + needle.len();
            let end = cookie_header.to_str().ok()?[value_start..]
                .find(';')
                .unwrap_or(cookie_header.to_str().ok()?.len());
            let value = &cookie_header.to_str().ok()?[value_start..value_start + end];
            return Cookie::parse(format!("{}{}", needle, value)).ok();
        }

        None
    }

    fn modify_response(
        &self,
        mut response: Response<Body>,
        backend_address: &str,
    ) -> Response<Body> {
        let headers = response.headers_mut();
        let cookie = Cookie::build(self.cookie_name.as_str(), backend_address)
            .http_only(self.http_only)
            .secure(self.secure)
            .same_site(self.same_site)
            .finish();

        let cookie_val = HeaderValue::from_str(&cookie.to_string()).unwrap();
        headers.append(SET_COOKIE, cookie_val);

        response
    }
}

#[async_trait]
impl LoadBalancingStrategy for StickyCookie {
    fn select_backend<'l>(
        &'l self,
        request: &Request<Body>,
        context: &'l Context<'l>,
    ) -> RequestForwarder<'l> {
        let backend_address = self.try_parse_sticky_cookie(&request).and_then(|cookie| {
            context
                .backend_addresses
                .iter()
                .find(|it| **it == cookie.value())
        });

        if let Some(backend_address) = backend_address {
            RequestForwarder::new(backend_address)
        } else {
            let backend = self.inner.select_backend(request, context);
            let backend_address = backend.backend_address;
            backend.map_response(move |response| self.modify_response(response, backend_address))
        }
    }
}
