//! The six v1.1 middlewares, mapped to Pingora phases:
//!
//! | middleware        | phase                                                    |
//! |-------------------|----------------------------------------------------------|
//! | HttpsRedirector   | `request_filter` (301)                                   |
//! | RateLimiter       | `request_filter` (429)                                   |
//! | Authentication    | `request_filter` (401)                                   |
//! | MaxBodySize       | `request_filter` (declared length) + `request_body_filter` (streamed) |
//! | Compression       | `response_filter`, drives Pingora's compression module   |
//! | CustomErrorPages  | `response_filter`/`response_body_filter` and LB errors   |
//!
//! The request-phase middlewares run in the order they appear in the TOML table, like v1.1.

pub mod authentication;
pub mod compression;
pub mod error_pages;
pub mod https_redirector;
pub mod rate_limiter;

use authentication::Authentication;
use error_pages::ErrorPages;
use log::warn;
use rate_limiter::RateLimiter;
use toml::{value::Table, Value};

/// Request-phase middlewares, in configuration order.
#[derive(Debug)]
pub enum RequestStep {
    HttpsRedirector,
    RateLimiter(RateLimiter),
    Authentication(Box<Authentication>),
    MaxBodySize(u64),
}

#[derive(Debug, Default)]
pub struct Middlewares {
    pub request_steps: Vec<RequestStep>,
    pub compression: bool,
    pub error_pages: Option<ErrorPages>,
}

impl Middlewares {
    /// Builds the middlewares of a pool. Like v1.1, an invalid or unknown middleware is ignored
    /// with a warning instead of failing the configuration.
    pub fn from_table(table: Table) -> Middlewares {
        let mut middlewares = Middlewares::default();
        for (name, payload) in table {
            if !middlewares.add(&name, payload) {
                warn!("Ignoring invalid or unknown middleware '{}'", name);
            }
        }
        middlewares
    }

    fn add(&mut self, name: &str, payload: Value) -> bool {
        match (name, payload) {
            ("RateLimiter", Value::Table(t)) => {
                let limit = integer(&t, "limit").and_then(|v| u64::try_from(v).ok());
                let window = integer(&t, "window_sec").and_then(|v| u64::try_from(v).ok());
                match (limit, window) {
                    (Some(limit), Some(window)) => {
                        self.request_steps
                            .push(RequestStep::RateLimiter(RateLimiter::new(limit, window)));
                        true
                    }
                    _ => false,
                }
            }
            ("Authentication", Value::Table(t)) => {
                let string = |key: &str| t.get(key).and_then(Value::as_str).map(str::to_string);
                match (
                    string("ldap_address"),
                    string("user_directory"),
                    string("rdn_identifier"),
                    t.get("recursive").and_then(Value::as_bool),
                ) {
                    (
                        Some(ldap_address),
                        Some(user_directory),
                        Some(rdn_identifier),
                        Some(recursive),
                    ) => {
                        self.request_steps
                            .push(RequestStep::Authentication(Box::new(Authentication::new(
                                ldap_address,
                                user_directory,
                                rdn_identifier,
                                recursive,
                            ))));
                        true
                    }
                    _ => false,
                }
            }
            ("Compression", _) => {
                self.compression = true;
                true
            }
            ("HttpsRedirector", _) => {
                self.request_steps.push(RequestStep::HttpsRedirector);
                true
            }
            ("MaxBodySize", Value::Table(t)) => match integer(&t, "limit") {
                Some(limit) => {
                    // A negative limit rejects every body, like v1.1
                    self.request_steps
                        .push(RequestStep::MaxBodySize(u64::try_from(limit).unwrap_or(0)));
                    true
                }
                None => false,
            },
            ("CustomErrorPages", Value::Table(t)) => match ErrorPages::from_table(&t) {
                Some(pages) => {
                    self.error_pages = Some(pages);
                    true
                }
                None => false,
            },
            _ => false,
        }
    }
}

fn integer(table: &Table, key: &str) -> Option<i64> {
    table.get(key).and_then(Value::as_integer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_order_and_ignores_invalid() {
        let table: Table = toml::from_str(
            r#"
            [HttpsRedirector]
            [RateLimiter]
            limit = 10
            window_sec = 1
            [MaxBodySize]
            limit = -5
            [Compression]
            [Bogus]
            [Authentication]
            ldap_address = "ldap://x"
            "#,
        )
        .unwrap();
        let middlewares = Middlewares::from_table(table);
        assert!(middlewares.compression);
        assert!(matches!(
            middlewares.request_steps.as_slice(),
            [
                RequestStep::HttpsRedirector,
                RequestStep::RateLimiter(_),
                RequestStep::MaxBodySize(0)
            ]
        ));
    }
}
