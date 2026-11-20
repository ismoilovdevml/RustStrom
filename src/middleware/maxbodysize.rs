use super::{super::error_response, Context, Middleware};
use async_trait::async_trait;
use futures::StreamExt;
use hyper::{header::CONTENT_LENGTH, Body, Request, Response};
use std::io;

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
        let limit = u64::try_from(self.limit).unwrap_or(0);

        if let Some(length) = request.headers().get(CONTENT_LENGTH) {
            // hyper enforces Content-Length framing, so the body can not exceed it
            return match length.to_str().ok().and_then(|s| s.parse::<u64>().ok()) {
                Some(length) if length <= limit => Ok(request),
                Some(_) => Err(error_response::request_entity_to_large()),
                None => Err(error_response::bad_request("Invalid Content-Length")),
            };
        }

        // Chunked HTTP/1.1 and HTTP/2 bodies have no declared length: count the
        // bytes while streaming and abort the upload once the limit is exceeded.
        let (parts, body) = request.into_parts();
        let mut received: u64 = 0;
        let limited = body.map(move |chunk| match chunk {
            Ok(bytes) => {
                received = received.saturating_add(bytes.len() as u64);
                if received > limit {
                    Err(
                        Box::new(io::Error::other("request body exceeds MaxBodySize"))
                            as Box<dyn std::error::Error + Send + Sync>,
                    )
                } else {
                    Ok(bytes)
                }
            }
            Err(e) => Err(Box::new(e) as Box<dyn std::error::Error + Send + Sync>),
        });
        Ok(Request::from_parts(parts, Body::wrap_stream(limited)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        algorithms::round_robin::RoundRobin, http_client::StrategyNotifyHttpConnector,
        server::Scheme,
    };
    use hyper::{body::HttpBody, Client};
    use std::{net::SocketAddr, sync::Arc};

    #[allow(clippy::result_large_err)]
    async fn apply(request: Request<Body>, limit: i64) -> Result<Request<Body>, Response<Body>> {
        let client = Client::builder().build(StrategyNotifyHttpConnector::new(Arc::new(Box::new(
            RoundRobin::new(),
        ))));
        let client_address: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let context = Context {
            client_scheme: &Scheme::HTTP,
            client_address: &client_address,
            backend_uri: "http://127.0.0.1:2/".parse().unwrap(),
            client: &client,
        };
        MaxBodySize { limit }
            .modify_request(request, &context)
            .await
    }

    #[tokio::test]
    async fn rejects_large_content_length() {
        let request = Request::builder()
            .header(CONTENT_LENGTH, "11")
            .body(Body::from("hello world"))
            .unwrap();
        let response = apply(request, 10).await.unwrap_err();
        assert_eq!(response.status(), 413);
    }

    #[tokio::test]
    async fn chunked_body_over_limit_fails_while_streaming() {
        let chunks: Vec<Result<&'static str, io::Error>> = vec![Ok("12345"), Ok("67890"), Ok("x")];
        let request = Request::builder()
            .body(Body::wrap_stream(futures::stream::iter(chunks)))
            .unwrap();
        let mut body = apply(request, 10).await.unwrap().into_body();
        let mut failed = false;
        while let Some(chunk) = body.data().await {
            if chunk.is_err() {
                failed = true;
                break;
            }
        }
        assert!(
            failed,
            "body over the limit must not be forwarded completely"
        );
    }

    #[tokio::test]
    async fn chunked_body_within_limit_passes() {
        let chunks: Vec<Result<&'static str, io::Error>> = vec![Ok("12345"), Ok("67890")];
        let request = Request::builder()
            .body(Body::wrap_stream(futures::stream::iter(chunks)))
            .unwrap();
        let body = apply(request, 10).await.unwrap().into_body();
        let bytes = hyper::body::to_bytes(body).await.unwrap();
        assert_eq!(bytes.len(), 10);
    }
}
