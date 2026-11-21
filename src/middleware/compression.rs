use crate::utils::split_once;

use super::{Context, Middleware, MiddlewareChain};
use async_compression::{
    tokio::bufread::{BrotliEncoder, DeflateEncoder, GzipEncoder},
    Level,
};
use async_trait::async_trait;
use futures::TryStreamExt;
use hyper::{
    header::{
        HeaderValue, ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE,
        CONTENT_TYPE, VARY,
    },
    Body, HeaderMap, Method, Request, Response, StatusCode,
};
use std::{
    fmt::Display,
    io::{self},
};
use tokio::io::AsyncRead;
use tokio_util::{
    codec::{BytesCodec, FramedRead},
    io::StreamReader,
};
use Encoding::{BROTLI, DEFLATE, GZIP};

#[derive(Debug)]
pub struct Compression;

#[async_trait]
impl Middleware for Compression {
    async fn forward_request(
        &self,
        request: Request<Body>,
        chain: &MiddlewareChain,
        context: &Context<'_>,
    ) -> Response<Body> {
        let encoding = get_preferred_encoding(request.headers());
        let is_head = request.method() == Method::HEAD;
        let mut response = chain.forward_request(request, context).await;
        // The response varies by Accept-Encoding whether or not we compress it
        response
            .headers_mut()
            .append(VARY, HeaderValue::from_static("accept-encoding"));
        match encoding {
            Some(encoding) if !is_head && should_compress(&response) => {
                self.compress_response(response, &encoding)
            }
            _ => response,
        }
    }
}

impl Compression {
    fn compress_response(&self, response: Response<Body>, encoding: &Encoding) -> Response<Body> {
        let (parts, body) = response.into_parts();

        let stream = StreamReader::new(body.map_err(io::Error::other));

        let body = match encoding {
            // Default brotli quality is 11 (archival); 4-5 is the usual on-the-fly setting
            BROTLI => to_body(BrotliEncoder::with_quality(stream, Level::Precise(4))),
            DEFLATE => to_body(DeflateEncoder::with_quality(stream, Level::Precise(5))),
            GZIP => to_body(GzipEncoder::with_quality(stream, Level::Precise(5))),
        };
        fn to_body<S>(stream: S) -> hyper::Body
        where
            S: AsyncRead + Send + 'static,
        {
            Body::wrap_stream(FramedRead::new(stream, BytesCodec::new()))
        }

        let mut response = Response::from_parts(parts, body);
        let headers = response.headers_mut();
        headers.insert(CONTENT_ENCODING, encoding.into());
        headers.remove(CONTENT_LENGTH);
        response
    }
}

/// Responses smaller than this are not worth the CPU time of compressing.
const MIN_COMPRESS_LENGTH: u64 = 1024;

fn should_compress(response: &Response<Body>) -> bool {
    let status = response.status();
    if status == StatusCode::NO_CONTENT
        || status == StatusCode::NOT_MODIFIED
        || status.is_informational()
    {
        return false;
    }
    let headers = response.headers();
    if headers.contains_key(CONTENT_ENCODING) || headers.contains_key(CONTENT_RANGE) {
        return false;
    }
    if let Some(length) = headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
    {
        if length < MIN_COMPRESS_LENGTH {
            return false;
        }
    }
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(is_compressible_content_type)
}

/// Text based formats compress well; images, video, fonts like woff2 and
/// archives are already compressed. Streams such as server-sent events must not
/// be buffered by an encoder.
fn is_compressible_content_type(content_type: &str) -> bool {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if mime == "text/event-stream" {
        return false;
    }
    mime.starts_with("text/")
        || mime.ends_with("+json")
        || mime.ends_with("+xml")
        || matches!(
            mime.as_str(),
            "application/json"
                | "application/javascript"
                | "application/x-javascript"
                | "application/xml"
                | "application/wasm"
                | "image/svg+xml"
                | "application/vnd.ms-fontobject"
                | "font/ttf"
                | "font/otf"
        )
}

#[derive(Debug, PartialEq)]
#[allow(clippy::upper_case_acronyms)]
enum Encoding {
    BROTLI,
    DEFLATE,
    GZIP,
}

impl Encoding {
    fn to_str(&self) -> &'static str {
        match self {
            BROTLI => "br",
            DEFLATE => "deflate",
            GZIP => "gzip",
        }
    }

    fn from_str(s: &str) -> Option<Encoding> {
        match s {
            "br" => Some(BROTLI),
            "deflate" => Some(DEFLATE),
            "gzip" => Some(GZIP),
            _ => None,
        }
    }
}

impl From<&Encoding> for HeaderValue {
    fn from(encoding: &Encoding) -> Self {
        HeaderValue::from_static(encoding.to_str())
    }
}

impl Display for Encoding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.to_str())
    }
}

/// Parses the Header `Accept-Encoding` (as definied in [RFC 7231, section 5.3.4: Accept-Encoding](https://tools.ietf.org/html/rfc7231#section-5.3.4))
/// in the specified `HeaderMap` returning the preferred supported `Encoding` if one exists.
///
/// Do determine which `Encoding` is preferred by the client, Quality Values (as defined in [RFC 7231, section 5.3.1: Quality Values](https://tools.ietf.org/html/rfc7231#section-5.3.1)) are used.
///
/// The special encoding `*` is not currently supported.
fn get_preferred_encoding(headers: &HeaderMap) -> Option<Encoding> {
    headers
        .get(ACCEPT_ENCODING)?
        .to_str()
        .ok()?
        .split(',')
        .map(|it| it.trim())
        .filter_map(parse_encoding_and_qvalue)
        .rev() // max_by_key returns last of equal elements, so reverse to get the first instead
        .max_by_key(|(_encoding, qvalue)| *qvalue)
        .map(|(encoding, _qvalue)| encoding)
}

fn parse_encoding_and_qvalue(encoding_and_qvalue: &str) -> Option<(Encoding, u32)> {
    let (encoding, qvalue) =
        split_once(encoding_and_qvalue, ';').unwrap_or((encoding_and_qvalue, "q=1"));
    let encoding = Encoding::from_str(encoding)?;
    let qvalue = parse_qvalue(qvalue)?;
    Some((encoding, qvalue))
}

/// Parses the Quality Value (as defined in [RFC 7231, section 5.3.1: Quality Values](https://tools.ietf.org/html/rfc7231#section-5.3.1)) as an `u32`.
///
/// Using `u32` instead of `f32` is possible because the precision is limited to 3 digits after the decimal point.
/// `u32` has the advantage that it implements `Ord` and not just `PartialOrd` which is important for methods like `max_by_key`.
fn parse_qvalue(qvalue: &str) -> Option<u32> {
    let qvalue = qvalue.strip_prefix("q=")?;
    if qvalue == "1" {
        Some(1000)
    } else {
        let qvalue = qvalue
            .strip_prefix("0.")
            .filter(|digits| digits.len() <= 3)?;
        format!("{:0<3}", qvalue)
            .parse()
            .ok()
            .filter(|qvalue| *qvalue != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(status: u16, content_type: Option<&str>, length: Option<u64>) -> Response<Body> {
        let mut builder = Response::builder().status(status);
        if let Some(content_type) = content_type {
            builder = builder.header(CONTENT_TYPE, content_type);
        }
        if let Some(length) = length {
            builder = builder.header(CONTENT_LENGTH, length);
        }
        builder.body(Body::empty()).unwrap()
    }

    #[test]
    fn compresses_large_text() {
        assert!(should_compress(&response(
            200,
            Some("text/html; charset=utf-8"),
            Some(4096)
        )));
        assert!(should_compress(&response(
            200,
            Some("application/json"),
            None
        )));
    }

    #[test]
    fn skips_small_binary_and_empty_responses() {
        assert!(!should_compress(&response(
            200,
            Some("application/json"),
            Some(100)
        )));
        assert!(!should_compress(&response(
            200,
            Some("image/png"),
            Some(100_000)
        )));
        assert!(!should_compress(&response(
            200,
            Some("font/woff2"),
            Some(100_000)
        )));
        assert!(!should_compress(&response(
            200,
            Some("text/event-stream"),
            None
        )));
        assert!(!should_compress(&response(204, Some("text/html"), None)));
        assert!(!should_compress(&response(304, Some("text/html"), None)));
        assert!(!should_compress(&response(200, None, Some(100_000))));
    }

    #[test]
    fn parses_preferred_encoding() {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("gzip;q=0.5, br"));
        assert_eq!(get_preferred_encoding(&headers), Some(BROTLI));
    }
}
