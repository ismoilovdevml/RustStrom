//! Replaces error responses with HTML pages from `<location>/<status>.html`.
//!
//! The pages are read once when the configuration is loaded, so serving an error never touches
//! the disk.

use bytes::Bytes;
use log::warn;
use std::{collections::HashMap, path::Path};
use toml::{value::Table, Value};

#[derive(Debug)]
pub struct ErrorPages {
    pages: HashMap<u16, Bytes>,
}

impl ErrorPages {
    pub fn from_table(table: &Table) -> Option<ErrorPages> {
        let location = table.get("location")?.as_str()?;
        let errors = table.get("errors")?.as_array()?;
        let pages = errors
            .iter()
            .filter_map(Value::as_integer)
            .filter_map(|x| u16::try_from(x).ok())
            .map(|status| (status, load_page(location, status)))
            .collect();
        Some(ErrorPages { pages })
    }

    pub fn get(&self, status: u16) -> Option<&Bytes> {
        self.pages.get(&status)
    }
}

fn load_page(location: &str, status: u16) -> Bytes {
    let path = Path::new(location).join(format!("{}.html", status));
    match std::fs::read(&path) {
        Ok(content) => Bytes::from(content),
        Err(e) => {
            warn!(
                "Custom error page '{}' could not be read ({}), using a plain text fallback",
                path.display(),
                e
            );
            let reason = http::StatusCode::from_u16(status)
                .ok()
                .and_then(|it| it.canonical_reason())
                .unwrap_or("");
            Bytes::from(format!("{} - {}\n", status, reason))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_pages_with_fallback() {
        let dir = std::env::temp_dir().join(format!("ruststrom-pages-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("404.html"), "<h1>gone</h1>").unwrap();
        let table: Table = toml::from_str(&format!(
            "location = {:?}\nerrors = [404, 502, 70000]",
            dir.display().to_string()
        ))
        .unwrap();
        let pages = ErrorPages::from_table(&table).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(pages.get(404).unwrap().as_ref(), b"<h1>gone</h1>");
        assert_eq!(pages.get(502).unwrap().as_ref(), b"502 - Bad Gateway\n");
        assert!(pages.get(500).is_none());
    }
}
