//! Request checks for MCP over local HTTP, applied before a body is read:
//! Host (DNS rebinding), Origin (browsers), path, bearer token, method,
//! content type and protocol version. Pure: no I/O, easy to test.

use http::header::{AUTHORIZATION, CONTENT_TYPE, HOST, ORIGIN};
use http::{HeaderMap, Method, StatusCode};

use crate::server::SUPPORTED_VERSIONS;

/// The only path the server answers.
pub const MCP_PATH: &str = "/mcp";
pub const PROTOCOL_VERSION_HEADER: &str = "mcp-protocol-version";

/// Why a request is refused, mapped to an HTTP status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    pub status: StatusCode,
    pub message: &'static str,
    /// Extra response header (`WWW-Authenticate`, `Allow`).
    pub header: Option<(&'static str, &'static str)>,
}

impl Rejection {
    fn new(status: StatusCode, message: &'static str) -> Self {
        Self {
            status,
            message,
            header: None,
        }
    }

    fn with_header(mut self, name: &'static str, value: &'static str) -> Self {
        self.header = Some((name, value));
        self
    }
}

/// The expected bearer token and the port the server is bound to.
#[derive(Clone)]
pub struct Guard {
    token: String,
    port: u16,
}

impl std::fmt::Debug for Guard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Guard").field("port", &self.port).finish_non_exhaustive()
    }
}

impl Guard {
    pub fn new(token: impl Into<String>, port: u16) -> Self {
        Self {
            token: token.into(),
            port,
        }
    }

    /// Every check that does not need the body, in a fixed order: a request
    /// from a browser or with a foreign Host is refused before the token is
    /// even looked at.
    pub fn check(&self, method: &Method, path: &str, headers: &HeaderMap) -> Result<(), Rejection> {
        if !self.host_ok(headers) {
            return Err(Rejection::new(StatusCode::FORBIDDEN, "Host is not this computer's loopback address."));
        }
        if headers.contains_key(ORIGIN) {
            return Err(Rejection::new(StatusCode::FORBIDDEN, "Requests from web pages are refused."));
        }
        if path != MCP_PATH {
            return Err(Rejection::new(StatusCode::NOT_FOUND, "Not found. The MCP endpoint is /mcp."));
        }
        if !self.bearer_ok(headers) {
            return Err(
                Rejection::new(StatusCode::UNAUTHORIZED, "Missing or wrong bearer token.")
                    .with_header("www-authenticate", "Bearer"),
            );
        }
        if method != Method::POST {
            return Err(
                Rejection::new(StatusCode::METHOD_NOT_ALLOWED, "Only POST is supported (no server stream, no sessions).")
                    .with_header("allow", "POST"),
            );
        }
        if !is_json(headers) {
            return Err(Rejection::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Content-Type must be application/json."));
        }
        if !version_ok(headers) {
            return Err(Rejection::new(StatusCode::BAD_REQUEST, "Unsupported MCP-Protocol-Version."));
        }
        Ok(())
    }

    fn host_ok(&self, headers: &HeaderMap) -> bool {
        let Some(host) = single(headers, HOST.as_str()) else {
            return false;
        };
        let host = host.to_ascii_lowercase();
        host == format!("127.0.0.1:{}", self.port) || host == format!("localhost:{}", self.port)
    }

    fn bearer_ok(&self, headers: &HeaderMap) -> bool {
        let Some(value) = single(headers, AUTHORIZATION.as_str()) else {
            return false;
        };
        let Some((scheme, token)) = value.split_once(' ') else {
            return false;
        };
        scheme.eq_ignore_ascii_case("bearer") && token_eq(token.trim(), &self.token)
    }
}

/// The header's value when it is sent exactly once and is visible ASCII.
fn single<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let first = values.next()?;
    if values.next().is_some() {
        return None;
    }
    first.to_str().ok()
}

fn is_json(headers: &HeaderMap) -> bool {
    single(headers, CONTENT_TYPE.as_str()).is_some_and(|ct| {
        ct.split(';')
            .next()
            .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
    })
}

/// Absent = the client predates the header (2025-03-26 is assumed).
fn version_ok(headers: &HeaderMap) -> bool {
    match headers.get(PROTOCOL_VERSION_HEADER) {
        None => true,
        Some(v) => v
            .to_str()
            .is_ok_and(|v| SUPPORTED_VERSIONS.contains(&v.trim())),
    }
}

/// Constant-time comparison for equal lengths. An empty expected token never
/// matches.
pub fn token_eq(given: &str, expected: &str) -> bool {
    !expected.is_empty()
        && given.len() == expected.len()
        && given
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;

    const TOKEN: &str = "0123456789abcdef";

    fn guard() -> Guard {
        Guard::new(TOKEN, 4321)
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    fn good() -> Vec<(&'static str, &'static str)> {
        vec![
            ("host", "127.0.0.1:4321"),
            ("authorization", "Bearer 0123456789abcdef"),
            ("content-type", "application/json"),
        ]
    }

    fn status(pairs: &[(&'static str, &str)]) -> Option<StatusCode> {
        guard()
            .check(&Method::POST, MCP_PATH, &headers(pairs))
            .err()
            .map(|r| r.status)
    }

    fn replace(key: &'static str, value: &'static str) -> Vec<(&'static str, &'static str)> {
        let mut h: Vec<_> = good().into_iter().filter(|(k, _)| *k != key).collect();
        h.push((key, value));
        h
    }

    #[test]
    fn a_well_formed_request_passes() {
        assert_eq!(status(&good()), None);
        assert_eq!(status(&replace("host", "localhost:4321")), None);
        assert_eq!(status(&replace("host", "LOCALHOST:4321")), None);
        assert_eq!(status(&replace("authorization", "bearer 0123456789abcdef")), None);
        assert_eq!(
            status(&replace("content-type", "application/json; charset=utf-8")),
            None
        );
        for v in SUPPORTED_VERSIONS {
            let mut h = good();
            h.push((PROTOCOL_VERSION_HEADER, v));
            assert_eq!(status(&h), None, "{v}");
        }
    }

    #[test]
    fn foreign_hosts_are_refused_before_anything_else() {
        for host in [
            "evil.test:4321",
            "127.0.0.1:4322",
            "127.0.0.1",
            "localhost",
            "0.0.0.0:4321",
            "[::1]:4321",
            "127.0.0.1.evil.test:4321",
        ] {
            assert_eq!(status(&replace("host", host)), Some(StatusCode::FORBIDDEN), "{host}");
        }
        let no_host: Vec<_> = good().into_iter().filter(|(k, _)| *k != "host").collect();
        assert_eq!(status(&no_host), Some(StatusCode::FORBIDDEN));
        let mut twice = good();
        twice.push(("host", "127.0.0.1:4321"));
        assert_eq!(status(&twice), Some(StatusCode::FORBIDDEN));
        // A bad host is refused even without a token.
        let r = guard()
            .check(&Method::POST, MCP_PATH, &headers(&[("host", "evil.test:4321")]))
            .unwrap_err();
        assert_eq!(r.status, StatusCode::FORBIDDEN);
    }

    #[test]
    fn any_origin_is_refused() {
        for origin in ["http://127.0.0.1:4321", "http://localhost:4321", "https://evil.test", "null"] {
            let mut h = good();
            h.push(("origin", origin));
            assert_eq!(status(&h), Some(StatusCode::FORBIDDEN), "{origin}");
        }
    }

    #[test]
    fn other_paths_are_not_found() {
        for path in ["/", "/mcp/", "/MCP", "/sse", "/.well-known/oauth-protected-resource"] {
            let r = guard().check(&Method::POST, path, &headers(&good())).unwrap_err();
            assert_eq!(r.status, StatusCode::NOT_FOUND, "{path}");
        }
    }

    #[test]
    fn a_missing_or_wrong_token_is_unauthorized() {
        for auth in [
            "Bearer wrong",
            "Bearer 0123456789abcdeF",
            "Bearer 0123456789abcde",
            "Bearer ",
            "Basic 0123456789abcdef",
            "0123456789abcdef",
        ] {
            let r = guard()
                .check(&Method::POST, MCP_PATH, &headers(&replace("authorization", auth)))
                .unwrap_err();
            assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{auth}");
            assert_eq!(r.header, Some(("www-authenticate", "Bearer")));
        }
        let none: Vec<_> = good().into_iter().filter(|(k, _)| *k != "authorization").collect();
        assert_eq!(status(&none), Some(StatusCode::UNAUTHORIZED));
        let empty = Guard::new("", 4321);
        let mut h = good();
        h.retain(|(k, _)| *k != "authorization");
        h.push(("authorization", "Bearer "));
        assert!(empty.check(&Method::POST, MCP_PATH, &headers(&h)).is_err());
    }

    #[test]
    fn only_post_is_allowed() {
        for m in [Method::GET, Method::DELETE, Method::PUT, Method::OPTIONS] {
            let r = guard().check(&m, MCP_PATH, &headers(&good())).unwrap_err();
            assert_eq!(r.status, StatusCode::METHOD_NOT_ALLOWED, "{m}");
            assert_eq!(r.header, Some(("allow", "POST")));
        }
    }

    #[test]
    fn content_type_and_version_are_checked() {
        assert_eq!(
            status(&replace("content-type", "text/plain")),
            Some(StatusCode::UNSUPPORTED_MEDIA_TYPE)
        );
        let mut h = good();
        h.push((PROTOCOL_VERSION_HEADER, "1999-01-01"));
        assert_eq!(status(&h), Some(StatusCode::BAD_REQUEST));
    }

    #[test]
    fn token_comparison() {
        assert!(token_eq("abc", "abc"));
        assert!(!token_eq("abd", "abc"));
        assert!(!token_eq("ab", "abc"));
        assert!(!token_eq("", ""));
    }

    #[test]
    fn debug_never_prints_the_token() {
        assert!(!format!("{:?}", guard()).contains(TOKEN));
    }
}
