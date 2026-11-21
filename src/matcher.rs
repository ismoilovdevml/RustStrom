//! The v1 matcher language, unchanged: `Host`, `HostRegexp`, `Method`, `Path`, `PathRegexp`,
//! `Query`, combined with `&&`, `||` and parentheses.

use http::{header::HOST, Method};
use pingora_http::RequestHeader;
use pom::parser::*;
use regex::Regex;
use std::{ops::Deref, str::FromStr};

/// A newtype for Regex, which makes it comparable by its string value
#[derive(Debug)]
pub struct ComparableRegex(Regex);

impl Deref for ComparableRegex {
    type Target = Regex;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PartialEq for ComparableRegex {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_str().eq(other.0.as_str())
    }
}

impl ComparableRegex {
    pub fn new(regex: &str) -> Result<ComparableRegex, regex::Error> {
        Ok(ComparableRegex(Regex::new(regex)?))
    }
}

#[derive(Debug, PartialEq)]
pub enum Matcher {
    Host(String),
    HostRegexp(ComparableRegex),
    Method(Method),
    Path(String),
    PathRegexp(ComparableRegex),
    Query(String, String),
    And(Box<Matcher>, Box<Matcher>),
    Or(Box<Matcher>, Box<Matcher>),
}

impl TryFrom<&str> for Matcher {
    type Error = String;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        let chars: Vec<char> = input.chars().collect();
        let result = parser()
            .parse(&chars)
            .map_err(|e| format!("Invalid matcher \"{}\": {}", input, e));
        result
    }
}

/// The request host: the `Host` header for HTTP/1.1, the `:authority` pseudo header (exposed as
/// the URI authority) for HTTP/2. Compared verbatim, like v1.1 (a port is part of the value).
pub fn request_host(request: &RequestHeader) -> Option<&str> {
    // HTTP/2: `:authority` wins over a Host header (RFC 9113 section 8.3.1), and it is what
    // Pingora sends upstream as Host, so routing must use it too.
    if request.version == http::Version::HTTP_2 {
        if let Some(authority) = request.uri.authority() {
            return Some(authority.as_str());
        }
    }
    match request.headers.get(HOST) {
        Some(host) => host.to_str().ok(),
        None => request.uri.authority().map(|it| it.as_str()),
    }
}

impl Matcher {
    pub fn matches(&self, request: &RequestHeader) -> bool {
        match self {
            Matcher::Host(host) => request_host(request) == Some(host.as_str()),
            Matcher::HostRegexp(host_regex) => {
                request_host(request).is_some_and(|h| host_regex.is_match(h))
            }
            Matcher::Method(method) => request.method == *method,
            Matcher::Path(path) => request.uri.path() == path,
            Matcher::PathRegexp(path_regex) => path_regex.is_match(request.uri.path()),
            Matcher::Query(key, value) => request.uri.query().is_some_and(|v| {
                url::form_urlencoded::parse(v.as_bytes())
                    .any(|(sent_key, sent_value)| sent_key == *key && sent_value == *value)
            }),
            Matcher::And(left, right) => left.matches(request) && right.matches(request),
            Matcher::Or(left, right) => left.matches(request) || right.matches(request),
        }
    }

    /// A host every matching request must have, used to index pools by exact host.
    pub fn required_host(&self) -> Option<&str> {
        match self {
            Matcher::Host(host) => Some(host.as_str()),
            Matcher::And(left, right) => left.required_host().or_else(|| right.required_host()),
            _ => None,
        }
    }
}

/// A PEG parser for matcher rules
///
/// ```text
/// "Host('google.de')"
/// "HostRegexp('^(www\.)?google.de$')"
/// "Host('google.de') && Path('/admin')"
/// "Host('google.de') || Path('/admin')"
/// "Host('google.de') && Query('admin', 'true')"
/// "Host('google.de') && Method('GET')"
/// "Host('google.de') && ( Path('/admin') || Path('/moderator') )"
/// ```
fn parser<'a>() -> Parser<'a, char, Matcher> {
    space() * top_level_expression() - end()
}

fn string<'a>() -> Parser<'a, char, String> {
    let special_char = (sym('\\') * sym('\'')) | sym('\\');

    let char_string = (none_of("\\\'") | special_char)
        .repeat(1..)
        .map(String::from_iter);
    let string = sym('\'') * char_string.repeat(0..) - sym('\'');
    string.map(|strings| strings.concat())
}

fn host<'a>() -> Parser<'a, char, String> {
    tag("Host(") * string() - sym(')')
}

fn host_regexp<'a>() -> Parser<'a, char, ComparableRegex> {
    let host_regexp = tag("HostRegexp(") * string() - sym(')');
    host_regexp.convert(|regex| ComparableRegex::new(&regex))
}

fn path<'a>() -> Parser<'a, char, String> {
    tag("Path(") * string() - sym(')')
}

fn path_regexp<'a>() -> Parser<'a, char, ComparableRegex> {
    let path_regexp = tag("PathRegexp(") * string() - sym(')');
    path_regexp.convert(|regex| ComparableRegex::new(&regex))
}

fn method<'a>() -> Parser<'a, char, Method> {
    let method = tag("Method(") * string() - sym(')');
    method.convert(|method| Method::from_str(&method))
}

fn query<'a>() -> Parser<'a, char, (String, String)> {
    tag("Query(") * string() - space() - sym(',') - space() + string() - sym(')')
}

fn and<'a>() -> Parser<'a, char, (Matcher, Matcher)> {
    call(value) - space() - tag("&&") - space() + call(value)
}

fn or<'a>() -> Parser<'a, char, (Matcher, Matcher)> {
    call(value) - space() - tag("||") - space() + call(value)
}

fn space<'a>() -> Parser<'a, char, ()> {
    one_of(" \t\r\n").repeat(0..).discard()
}

fn value<'a>() -> Parser<'a, char, Matcher> {
    host().map(Matcher::Host)
        | host_regexp().map(Matcher::HostRegexp)
        | method().map(Matcher::Method)
        | path().map(Matcher::Path)
        | path_regexp().map(Matcher::PathRegexp)
        | query().map(|(key, value)| Matcher::Query(key, value))
        | (sym('(') * space() * (chained_expression() | call(value)) - space() - sym(')'))
}

fn chained_expression<'a>() -> Parser<'a, char, Matcher> {
    and().map(|(left, right)| Matcher::And(Box::new(left), Box::new(right)))
        | or().map(|(left, right)| Matcher::Or(Box::new(left), Box::new(right)))
}

fn top_level_expression<'a>() -> Parser<'a, char, Matcher> {
    chained_expression() | value()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> Result<Matcher, String> {
        Matcher::try_from(input)
    }

    fn request(method: &str, uri: &str, host: Option<&str>) -> RequestHeader {
        let mut req = RequestHeader::build(method, uri.as_bytes(), None).unwrap();
        if let Some(host) = host {
            req.insert_header("Host", host).unwrap();
        }
        req
    }

    #[test]
    fn parse_host() {
        assert_eq!(
            parse("Host('whatisup.localhost')"),
            Ok(Matcher::Host("whatisup.localhost".into()))
        );
    }

    #[test]
    fn parse_escaped_host() {
        assert_eq!(
            parse("Host('whatisup\\'.localhost')"),
            Ok(Matcher::Host("whatisup'.localhost".into()))
        );
    }

    #[test]
    fn parse_empty_host() {
        assert_eq!(parse("Host('')"), Ok(Matcher::Host("".into())));
    }

    #[test]
    fn parse_and() {
        let left = Box::new(Matcher::Host("whoami.localhost".to_string()));
        let right = Box::new(Matcher::Host("whatisup.localhost".to_string()));
        assert_eq!(
            parse("Host('whoami.localhost')&&Host('whatisup.localhost')"),
            Ok(Matcher::And(left, right))
        );
    }

    #[test]
    fn parse_nested_single_value_and() {
        let left = Box::new(Matcher::Host("whoami.localhost".to_string()));
        let right = Box::new(Matcher::Host("whatisup.localhost".to_string()));
        assert_eq!(
            parse("(Host('whoami.localhost')) && Host('whatisup.localhost')"),
            Ok(Matcher::And(left, right))
        );
    }

    #[test]
    fn parse_nested_sub_expression() {
        let left = Box::new(Matcher::Or(
            Box::new(Matcher::Host("1".to_string())),
            Box::new(Matcher::Host("2".to_string())),
        ));
        let right = Box::new(Matcher::Host("3".to_string()));
        assert_eq!(
            parse("(  Host('1')      || Host('2')   )     &&    Host('3')"),
            Ok(Matcher::And(left, right))
        );
    }

    #[test]
    fn parse_escaped_regex() {
        let matcher = Matcher::HostRegexp(ComparableRegex::new("\\.").unwrap());
        assert_eq!(parse("HostRegexp('\\.')"), Ok(matcher));
    }

    #[test]
    fn parse_method() {
        assert_eq!(parse("Method('GET')"), Ok(Matcher::Method(Method::GET)));
        assert_eq!(
            parse("Method('YOLO')"),
            Ok(Matcher::Method(Method::from_str("YOLO").unwrap()))
        );
    }

    #[test]
    fn parse_query() {
        assert_eq!(
            parse("Query('key', 'value')"),
            Ok(Matcher::Query("key".into(), "value".into()))
        );
    }

    #[test]
    fn invalid_matcher_is_an_error_not_a_panic() {
        assert!(parse("Host('a'").is_err());
        assert!(parse("Nonsense()").is_err());
        assert!(parse("HostRegexp('(')").is_err());
        assert!(parse("Host('a')").is_ok());
    }

    #[test]
    fn matches_http2_authority_without_host_header() {
        let mut req = request("GET", "/x", None);
        req.set_uri("https://google.de/x".parse().unwrap());
        assert!(Matcher::Host("google.de".into()).matches(&req));
    }

    #[test]
    fn http2_authority_wins_over_host_header() {
        let mut req = request("GET", "/x", Some("evil.test"));
        req.set_uri("https://good.test/x".parse().unwrap());
        req.set_version(http::Version::HTTP_2);
        assert_eq!(request_host(&req), Some("good.test"));
        // HTTP/1.1 keeps v1.1 semantics: the Host header
        req.set_version(http::Version::HTTP_11);
        assert_eq!(request_host(&req), Some("evil.test"));
    }

    #[test]
    fn matches_host_and_regex() {
        let matcher = Matcher::HostRegexp(ComparableRegex::new(r"^(www\.)?google.de$").unwrap());
        assert!(matcher.matches(&request("GET", "/", Some("google.de"))));
        assert!(matcher.matches(&request("GET", "/", Some("www.google.de"))));
        assert!(!matcher.matches(&request("GET", "/", Some("www.youtube.de"))));
        assert!(Matcher::Host("google.de".into()).matches(&request("GET", "/", Some("google.de"))));
    }

    #[test]
    fn matches_method_path_query() {
        assert!(Matcher::Method(Method::GET).matches(&request("GET", "/", None)));
        assert!(!Matcher::Method(Method::GET).matches(&request("POST", "/", None)));
        assert!(Matcher::Path("/admin".into()).matches(&request("GET", "/admin", None)));
        assert!(!Matcher::Path("/admin".into()).matches(&request("GET", "/", None)));
        let query = Matcher::Query("admin".into(), "true".into());
        assert!(query.matches(&request("GET", "/?admin=true", None)));
        assert!(!query.matches(&request("GET", "/", None)));
    }

    #[test]
    fn matches_and_or() {
        let and = parse("Host('google.de') && Query('admin', 'true')").unwrap();
        assert!(and.matches(&request("GET", "/?admin=true", Some("google.de"))));
        assert!(!and.matches(&request("GET", "/", Some("google.de"))));
        let or = parse("Host('google.de') || Query('admin', 'true')").unwrap();
        assert!(or.matches(&request("GET", "/?admin=true", Some("youtube.de"))));
        assert!(or.matches(&request("GET", "/", Some("google.de"))));
    }

    #[test]
    fn required_host_for_indexing() {
        assert_eq!(parse("Host('a')").unwrap().required_host(), Some("a"));
        assert_eq!(
            parse("Path('/x') && Host('a')").unwrap().required_host(),
            Some("a")
        );
        assert_eq!(
            parse("Host('a') || Path('/x')").unwrap().required_host(),
            None
        );
        assert_eq!(parse("PathRegexp('^/')").unwrap().required_host(), None);
    }
}
