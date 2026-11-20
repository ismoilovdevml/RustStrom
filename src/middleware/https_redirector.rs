//! Redirects plain HTTP requests to HTTPS with `301 Moved Permanently` (v1.1 semantics: the
//! authority comes from an absolute request URI or the `Host` header, port included).

use http::{
    header::HOST,
    uri::{Authority, Scheme, Uri},
};
use pingora_http::RequestHeader;

/// Why no redirect location could be built: the status and body to answer with.
#[derive(Debug, PartialEq, Eq)]
pub struct Rejection {
    pub status: u16,
    pub message: &'static str,
}

pub fn location(request: &RequestHeader) -> Result<String, Rejection> {
    let authority = match request.uri.authority() {
        Some(authority) => authority.clone(),
        None => host_authority(request)?,
    };
    let path_and_query = request.uri.path_and_query().ok_or(Rejection {
        status: 500,
        message: "",
    })?;
    let uri = Uri::builder()
        .scheme(Scheme::HTTPS)
        .authority(authority)
        .path_and_query(path_and_query.clone())
        .build()
        .map_err(|_| Rejection {
            status: 500,
            message: "",
        })?;
    Ok(uri.to_string())
}

fn host_authority(request: &RequestHeader) -> Result<Authority, Rejection> {
    let host = request.headers.get(HOST).ok_or(Rejection {
        status: 400,
        message: "Missing Host header",
    })?;
    let host = host.to_str().map_err(|_| Rejection {
        status: 400,
        message: "Failed to parse Host header",
    })?;
    Authority::try_from(host).map_err(|_| Rejection {
        status: 400,
        message: "Invalid Host header",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(uri: &str, host: Option<&str>) -> RequestHeader {
        let mut req = RequestHeader::build("GET", uri.as_bytes(), None).unwrap();
        if let Some(host) = host {
            req.insert_header("Host", host).unwrap();
        }
        req
    }

    #[test]
    fn builds_https_location() {
        assert_eq!(
            location(&req("/a?b=c", Some("example.com"))),
            Ok("https://example.com/a?b=c".to_string())
        );
        assert_eq!(
            location(&req("/", Some("example.com:8080"))),
            Ok("https://example.com:8080/".to_string())
        );
        // A URI authority (HTTP/2) wins over the Host header
        let mut with_authority = req("/x", Some("example.com"));
        with_authority.set_uri("http://other.test/x".parse().unwrap());
        assert_eq!(
            location(&with_authority),
            Ok("https://other.test/x".to_string())
        );
    }

    #[test]
    fn rejects_missing_or_invalid_host() {
        assert_eq!(location(&req("/", None)).unwrap_err().status, 400);
        assert_eq!(
            location(&req("/", Some("bad host"))).unwrap_err().status,
            400
        );
    }
}
