use super::{super::error_response, Context, Middleware};
use async_trait::async_trait;
use hyper::{header::CONTENT_LENGTH, Body, Request, Response};

#[derive(Debug, Clone, Copy)]
pub struct MaxBodySize {
    pub(crate) limit: i64,
}

#[async_trait]
impl Middleware for MaxBodySize {
    async fn modify_request(
        &self,
        request: Request<Body>,
        _context: &Context<'_>,
    ) -> Result<Request<Body>, Response<Body>> {
        // Directly check the content length and compare to the limit.
        // This avoids multiple function calls and streamlines the check.
        if let Some(length) = request.headers().get(CONTENT_LENGTH) {
            if let Ok(s) = length.to_str() {
                if let Ok(parsed_length) = s.parse::<i64>() {
                    if parsed_length > self.limit {
                        return Err(error_response::request_entity_to_large());
                    }
                }
            }
        }
        Ok(request)
    }
}