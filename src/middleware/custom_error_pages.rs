use super::{Context, Middleware};
use async_trait::async_trait;
use bytes::Bytes;
use hyper::{
    header::{HeaderValue, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE},
    Body, Response,
};
use log::warn;
use std::{collections::HashMap, convert::TryFrom, fs, path::Path};
use toml::value::Table;

/// Replaces error responses with HTML pages from `<location>/<status>.html`.
///
/// The pages are read once when the configuration is loaded, so serving an
/// error never touches the disk (and never blocks a runtime worker thread).
#[derive(Debug)]
pub struct CustomErrorPages {
    pages: HashMap<u16, Bytes>,
}

#[async_trait]
impl Middleware for CustomErrorPages {
    async fn modify_response(
        &self,
        response: Response<Body>,
        _context: &Context<'_>,
    ) -> Response<Body> {
        match self.pages.get(&response.status().as_u16()) {
            Some(page) => replace_body(response, page.clone()),
            None => response,
        }
    }
}

impl TryFrom<Table> for CustomErrorPages {
    type Error = ();

    fn try_from(t: Table) -> Result<Self, Self::Error> {
        let location = t.get("location").ok_or(())?.as_str().ok_or(())?;
        let errors = t
            .get("errors")
            .ok_or(())?
            .as_array()
            .ok_or(())?
            .iter()
            .filter_map(|x| x.as_integer())
            .filter_map(|x| u16::try_from(x).ok());

        let pages = errors
            .map(|status| (status, load_page(location, status)))
            .collect();
        Ok(CustomErrorPages { pages })
    }
}

fn load_page(location: &str, status: u16) -> Bytes {
    let path = Path::new(location).join(format!("{}.html", status));
    match fs::read(&path) {
        Ok(content) => Bytes::from(content),
        Err(e) => {
            warn!(
                "Custom error page '{}' could not be read ({}), using a plain text fallback",
                path.display(),
                e
            );
            let reason = hyper::StatusCode::from_u16(status)
                .ok()
                .and_then(|it| it.canonical_reason())
                .unwrap_or("");
            Bytes::from(format!("{} - {}\n", status, reason))
        }
    }
}

fn replace_body(response: Response<Body>, page: Bytes) -> Response<Body> {
    let (mut parts, _) = response.into_parts();
    parts.headers.remove(CONTENT_LENGTH);
    parts.headers.remove(CONTENT_ENCODING);
    parts.headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    Response::from_parts(parts, Body::from(page))
}
