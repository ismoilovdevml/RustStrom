//! Compression with v1.1 rules (q-value negotiation, brotli 4 / gzip 5, skip small, binary and
//! streaming responses, `Vary: Accept-Encoding`), executed by Pingora's compression module.

use http::{
    header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE},
    HeaderMap, StatusCode,
};
use pingora_core::{
    modules::http::{compression::ResponseCompression, HttpModuleCtx},
    protocols::http::compression::Algorithm,
};
use pingora_http::{RequestHeader, ResponseHeader};
use std::sync::OnceLock;

/// Default brotli quality is 11 (archival); 4-5 is the usual on-the-fly setting.
const BROTLI_LEVEL: u32 = 4;
const GZIP_LEVEL: u32 = 5;
/// Responses smaller than this are not worth the CPU time of compressing.
const MIN_COMPRESS_LENGTH: u64 = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Brotli,
    Gzip,
}

/// Enables the downstream compression module for this response if the v1.1 rules allow it.
/// Must be called from `response_filter`, i.e. before the module sees the response header.
pub fn enable_for_response(
    modules: &mut HttpModuleCtx,
    encoding: Encoding,
    response: &ResponseHeader,
) -> bool {
    if !should_compress(response) {
        return false;
    }
    let Some(module) = modules.get_mut::<ResponseCompression>() else {
        return false;
    };
    if !module.is_header_phase() {
        return false;
    }
    let Some(request) = accept_only(encoding) else {
        return false;
    };
    let (algorithm, level) = match encoding {
        Encoding::Brotli => (Algorithm::Brotli, BROTLI_LEVEL),
        Encoding::Gzip => (Algorithm::Gzip, GZIP_LEVEL),
    };
    module.adjust_algorithm_level(algorithm, level);
    // Our own negotiation already chose the encoding: hand the module exactly that one
    module.request_filter(request);
    true
}

/// A request header that accepts only `encoding`, to feed the module's negotiation.
fn accept_only(encoding: Encoding) -> Option<&'static RequestHeader> {
    static BROTLI: OnceLock<Option<RequestHeader>> = OnceLock::new();
    static GZIP: OnceLock<Option<RequestHeader>> = OnceLock::new();
    let (cell, value) = match encoding {
        Encoding::Brotli => (&BROTLI, "br"),
        Encoding::Gzip => (&GZIP, "gzip"),
    };
    cell.get_or_init(|| {
        let mut request = RequestHeader::build("GET", b"/", Some(1)).ok()?;
        request.insert_header(ACCEPT_ENCODING, value).ok()?;
        Some(request)
    })
    .as_ref()
}

pub fn should_compress(response: &ResponseHeader) -> bool {
    let status = response.status;
    if status == StatusCode::NO_CONTENT
        || status == StatusCode::NOT_MODIFIED
        || status.is_informational()
    {
        return false;
    }
    let headers = &response.headers;
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

/// Text based formats compress well; images, video, fonts like woff2 and archives are already
/// compressed. Streams such as server-sent events must not be buffered by an encoder.
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

/// Parses `Accept-Encoding` (RFC 9110 section 12.5.3) and returns the preferred supported
/// encoding by quality value; on equal quality the first listed wins. `*` is not supported.
pub fn preferred_encoding(headers: &HeaderMap) -> Option<Encoding> {
    headers
        .get(ACCEPT_ENCODING)?
        .to_str()
        .ok()?
        .split(',')
        .map(|it| it.trim())
        .filter_map(parse_encoding_and_qvalue)
        .rev() // max_by_key returns the last of equal elements, so reverse to get the first
        .max_by_key(|(_encoding, qvalue)| *qvalue)
        .map(|(encoding, _qvalue)| encoding)
}

fn parse_encoding_and_qvalue(encoding_and_qvalue: &str) -> Option<(Encoding, u32)> {
    let (encoding, qvalue) = encoding_and_qvalue
        .split_once(';')
        .unwrap_or((encoding_and_qvalue, "q=1"));
    let encoding = match encoding.trim() {
        "br" => Encoding::Brotli,
        "gzip" => Encoding::Gzip,
        _ => return None,
    };
    let qvalue = parse_qvalue(qvalue.trim())?;
    Some((encoding, qvalue))
}

/// Quality value as an integer in thousandths, so it implements `Ord`.
fn parse_qvalue(qvalue: &str) -> Option<u32> {
    let qvalue = qvalue.strip_prefix("q=")?;
    if qvalue == "1" || qvalue == "1.0" || qvalue == "1.00" || qvalue == "1.000" {
        Some(1000)
    } else {
        let digits = qvalue
            .strip_prefix("0.")
            .filter(|digits| digits.len() <= 3)?;
        format!("{:0<3}", digits)
            .parse()
            .ok()
            .filter(|qvalue| *qvalue != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;

    fn accept(value: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static(value));
        headers
    }

    fn response(content_type: &str, length: Option<&str>) -> ResponseHeader {
        let mut response = ResponseHeader::build(200, None).unwrap();
        response
            .insert_header(CONTENT_TYPE, content_type.to_string())
            .unwrap();
        if let Some(length) = length {
            response
                .insert_header(CONTENT_LENGTH, length.to_string())
                .unwrap();
        }
        response
    }

    #[test]
    fn negotiation_uses_q_values() {
        assert_eq!(
            preferred_encoding(&accept("gzip, br")),
            Some(Encoding::Gzip)
        );
        assert_eq!(
            preferred_encoding(&accept("gzip;q=0.5, br")),
            Some(Encoding::Brotli)
        );
        assert_eq!(
            preferred_encoding(&accept("br;q=0, gzip")),
            Some(Encoding::Gzip)
        );
        assert_eq!(preferred_encoding(&accept("deflate, identity")), None);
        assert_eq!(preferred_encoding(&accept("gzip;q=garbage")), None);
        assert_eq!(preferred_encoding(&HeaderMap::new()), None);
    }

    #[test]
    fn compress_rules() {
        assert!(should_compress(&response("text/html; charset=utf-8", None)));
        assert!(should_compress(&response("application/json", Some("5000"))));
        assert!(!should_compress(&response("application/json", Some("100"))));
        assert!(!should_compress(&response("image/png", Some("5000"))));
        assert!(!should_compress(&response("text/event-stream", None)));
        let mut encoded = response("text/html", None);
        encoded.insert_header(CONTENT_ENCODING, "gzip").unwrap();
        assert!(!should_compress(&encoded));
        let mut no_content = response("text/html", None);
        no_content.set_status(204).unwrap();
        assert!(!should_compress(&no_content));
    }
}
