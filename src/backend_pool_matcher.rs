use std::{convert::TryFrom, ops::Deref, str::FromStr};

use hyper::{header::HOST, Body, Method, Request};
use pom::parser::*;
use regex::Regex;

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
pub enum BackendPoolMatcher {
    Host(String),
    HostRegexp(ComparableRegex),
    Method(Method),
    Path(String),
    PathRegexp(ComparableRegex),
    Query(String, String),
    And(Box<BackendPoolMatcher>, Box<BackendPoolMatcher>),
    Or(Box<BackendPoolMatcher>, Box<BackendPoolMatcher>),
}

impl TryFrom<String> for BackendPoolMatcher {
    type Error = String;

    fn try_from(str: String) -> Result<Self, Self::Error> {
        let chars: Vec<char> = str.chars().collect();
        let result = parser()
            .parse(&chars)
            .map_err(|e| format!("Invalid matcher \"{}\": {}", str, e));
        result
    }
}

/// The request host: the `Host` header for HTTP/1.1, the `:authority` pseudo
/// header (exposed as the URI authority) for HTTP/2.
fn request_host(request: &Request<Body>) -> Option<&str> {
    match request.headers().get(HOST) {
        Some(host) => host.to_str().ok(),
        None => request.uri().authority().map(|it| it.as_str()),
    }
}

impl BackendPoolMatcher {
    /// Simplified matches function
    pub fn matches(&self, request: &Request<Body>) -> bool {
        match self {
            BackendPoolMatcher::Host(host) => request_host(request) == Some(host.as_str()),
            BackendPoolMatcher::HostRegexp(host_regex) => {
                request_host(request).is_some_and(|h| host_regex.is_match(h))
            }
            BackendPoolMatcher::Method(method) => request.method() == method,
            BackendPoolMatcher::Path(path) => request.uri().path() == path,
            BackendPoolMatcher::PathRegexp(path_regex) => path_regex.is_match(request.uri().path()),
            BackendPoolMatcher::Query(key, value) => request.uri().query().is_some_and(|v| {
                url::form_urlencoded::parse(v.as_bytes())
                    .any(|(sent_key, sent_value)| sent_key == *key && sent_value == *value)
            }),
            BackendPoolMatcher::And(left, right) => left.matches(request) && right.matches(request),
            BackendPoolMatcher::Or(left, right) => left.matches(request) || right.matches(request),
        }
    }
}

/// A PEG parser for generating BackendPoolMatcher rules
///
/// # Examples:
///
/// ```
/// "Host('google.de')"
/// "HostRegexp('^(www\.)?google.de$')"
/// "Host('google.de') && Path('/admin')"
/// "Host('google.de') || Path('/admin')"
/// "Host('google.de') && Query('admin', 'true')"
/// "Host('google.de') && Method('GET')"
/// "Host('google.de') && ( Path('/admin') || Path('/moderator') )"
/// ```
fn parser<'a>() -> Parser<'a, char, BackendPoolMatcher> {
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

fn and<'a>() -> Parser<'a, char, (BackendPoolMatcher, BackendPoolMatcher)> {
    call(value) - space() - tag("&&") - space() + call(value)
}

fn or<'a>() -> Parser<'a, char, (BackendPoolMatcher, BackendPoolMatcher)> {
    call(value) - space() - tag("||") - space() + call(value)
}

fn space<'a>() -> Parser<'a, char, ()> {
    one_of(" \t\r\n").repeat(0..).discard()
}

fn value<'a>() -> Parser<'a, char, BackendPoolMatcher> {
    host().map(BackendPoolMatcher::Host)
        | host_regexp().map(BackendPoolMatcher::HostRegexp)
        | method().map(BackendPoolMatcher::Method)
        | path().map(BackendPoolMatcher::Path)
        | path_regexp().map(BackendPoolMatcher::PathRegexp)
        | query().map(|(key, value)| BackendPoolMatcher::Query(key, value))
        | (sym('(') * space() * (chained_expression() | call(value)) - space() - sym(')'))
}

fn chained_expression<'a>() -> Parser<'a, char, BackendPoolMatcher> {
    and().map(|(left, right)| BackendPoolMatcher::And(Box::new(left), Box::new(right)))
        | or().map(|(left, right)| BackendPoolMatcher::Or(Box::new(left), Box::new(right)))
}

fn top_level_expression<'a>() -> Parser<'a, char, BackendPoolMatcher> {
    chained_expression() | value()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_char_vec(str: &'static str) -> Vec<char> {
        str.to_string().chars().collect()
    }

    #[test]
    fn parse_host() {
        let input = to_char_vec("Host('whatisup.localhost')");

        assert_eq!(
            parser().parse(&input),
            Ok(BackendPoolMatcher::Host("whatisup.localhost".into()))
        );
    }

    #[test]
    fn parse_escaped_host() {
        let input = to_char_vec("Host('whatisup\\'.localhost')");

        assert_eq!(
            parser().parse(&input),
            Ok(BackendPoolMatcher::Host("whatisup'.localhost".into()))
        );
    }

    #[test]
    fn parse_empty_host() {
        let input = to_char_vec("Host('')");

        assert_eq!(
            parser().parse(&input),
            Ok(BackendPoolMatcher::Host("".into()))
        );
    }

    #[test]
    fn parse_and() {
        let input = to_char_vec("Host('whoami.localhost')&&Host('whatisup.localhost')");

        let left = Box::new(BackendPoolMatcher::Host("whoami.localhost".to_string()));
        let right = Box::new(BackendPoolMatcher::Host("whatisup.localhost".to_string()));

        assert_eq!(
            parser().parse(&input),
            Ok(BackendPoolMatcher::And(left, right))
        );
    }

    #[test]
    fn parse_nested_single_value_and() {
        let input = to_char_vec("(Host('whoami.localhost')) && Host('whatisup.localhost')");

        let left = Box::new(BackendPoolMatcher::Host("whoami.localhost".to_string()));
        let right = Box::new(BackendPoolMatcher::Host("whatisup.localhost".to_string()));

        assert_eq!(
            parser().parse(&input),
            Ok(BackendPoolMatcher::And(left, right))
        );
    }

    #[test]
    fn parse_nested_sub_expression() {
        let input = to_char_vec("(  Host('1')      || Host('2')   )     &&    Host('3')");

        let left = Box::new(BackendPoolMatcher::Or(
            Box::new(BackendPoolMatcher::Host("1".to_string())),
            Box::new(BackendPoolMatcher::Host("2".to_string())),
        ));
        let right = Box::new(BackendPoolMatcher::Host("3".to_string()));

        assert_eq!(
            parser().parse(&input),
            Ok(BackendPoolMatcher::And(left, right))
        );
    }

    #[test]
    fn parse_escaped_regex() {
        let input = to_char_vec("HostRegexp('\\.')");

        let matcher = BackendPoolMatcher::HostRegexp(ComparableRegex::new("\\.").unwrap());

        assert_eq!(parser().parse(&input), Ok(matcher));
    }

    #[test]
    fn parse_method() {
        let input = to_char_vec("Method('GET')");
        let custom_input = to_char_vec("Method('YOLO')");

        assert_eq!(
            parser().parse(&input),
            Ok(BackendPoolMatcher::Method(Method::GET))
        );
        assert_eq!(
            parser().parse(&custom_input),
            Ok(BackendPoolMatcher::Method(
                Method::from_str("YOLO").unwrap()
            ))
        );
    }

    #[test]
    fn parse_query() {
        let input = to_char_vec("Query('key', 'value')");

        assert_eq!(
            parser().parse(&input),
            Ok(BackendPoolMatcher::Query("key".into(), "value".into()))
        );
    }

    #[test]
    fn invalid_matcher_is_an_error_not_a_panic() {
        assert!(BackendPoolMatcher::try_from("Host('a'".to_string()).is_err());
        assert!(BackendPoolMatcher::try_from("Nonsense()".to_string()).is_err());
        assert!(BackendPoolMatcher::try_from("Host('a')".to_string()).is_ok());
    }

    #[test]
    fn matches_http2_authority_without_host_header() {
        let request = Request::builder()
            .version(hyper::Version::HTTP_2)
            .uri("https://google.de/x")
            .body(Body::empty())
            .unwrap();
        assert!(BackendPoolMatcher::Host("google.de".into()).matches(&request));
    }

    #[test]
    fn matches_host() {
        let request = Request::builder()
            .header("Host", "google.de")
            .body(Body::empty())
            .unwrap();
        let matcher = BackendPoolMatcher::Host("google.de".into());

        assert!(matcher.matches(&request));
    }

    #[test]
    fn matches_host_regex() {
        let request_1 = Request::builder()
            .header("Host", "google.de")
            .body(Body::empty())
            .unwrap();

        let request_2 = Request::builder()
            .header("Host", "www.google.de")
            .body(Body::empty())
            .unwrap();

        let request_3 = Request::builder()
            .header("Host", "www.youtube.de")
            .body(Body::empty())
            .unwrap();

        let matcher =
            BackendPoolMatcher::HostRegexp(ComparableRegex::new(r#"^(www\.)?google.de$"#).unwrap());

        assert!(matcher.matches(&request_1));
        assert!(matcher.matches(&request_2));
        assert!(!matcher.matches(&request_3));
    }

    #[test]
    fn matches_method() {
        let request_1 = Request::builder()
            .method(Method::GET)
            .body(Body::empty())
            .unwrap();
        let request_2 = Request::builder()
            .method(Method::POST)
            .body(Body::empty())
            .unwrap();

        let matcher = BackendPoolMatcher::Method(Method::GET);

        assert!(matcher.matches(&request_1));
        assert!(!matcher.matches(&request_2));
    }

    #[test]
    fn matches_path() {
        let request_1 = Request::builder()
            .uri("https://google.de/admin")
            .body(Body::empty())
            .unwrap();
        let request_2 = Request::builder()
            .uri("https://google.de/")
            .body(Body::empty())
            .unwrap();

        let matcher = BackendPoolMatcher::Path("/admin".into());

        assert!(matcher.matches(&request_1));
        assert!(!matcher.matches(&request_2));
    }

    #[test]
    fn matches_query() {
        let request_1 = Request::builder()
            .uri("https://google.de?admin=true")
            .body(Body::empty())
            .unwrap();
        let request_2 = Request::builder()
            .uri("https://google.de/")
            .body(Body::empty())
            .unwrap();

        let matcher = BackendPoolMatcher::Query("admin".into(), "true".into());

        assert!(matcher.matches(&request_1));
        assert!(!matcher.matches(&request_2));
    }

    #[test]
    fn matches_and() {
        let request_1 = Request::builder()
            .uri("https://google.de?admin=true")
            .header(HOST, "google.de")
            .body(Body::empty())
            .unwrap();
        let request_2 = Request::builder()
            .uri("https://google.de")
            .header(HOST, "google.de")
            .body(Body::empty())
            .unwrap();

        let matcher = BackendPoolMatcher::And(
            Box::new(BackendPoolMatcher::Host("google.de".into())),
            Box::new(BackendPoolMatcher::Query("admin".into(), "true".into())),
        );

        assert!(matcher.matches(&request_1));
        assert!(!matcher.matches(&request_2));
    }

    #[test]
    fn matches_or() {
        let request_1 = Request::builder()
            .uri("https://youtube.de?admin=true")
            .header(HOST, "youtube.de")
            .body(Body::empty())
            .unwrap();
        let request_2 = Request::builder()
            .uri("https://google.de")
            .header(HOST, "google.de")
            .body(Body::empty())
            .unwrap();

        let matcher = BackendPoolMatcher::Or(
            Box::new(BackendPoolMatcher::Host("google.de".into())),
            Box::new(BackendPoolMatcher::Query("admin".into(), "true".into())),
        );

        assert!(matcher.matches(&request_1));
        assert!(matcher.matches(&request_2));
    }
}
