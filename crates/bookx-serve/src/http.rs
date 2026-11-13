//! Transport-independent HTTP request/response types and helpers shared by the protocol handlers.

use std::collections::HashMap;

use base64::Engine;
use percent_encoding::percent_decode_str;
use serde_json::Value;

/// A parsed request. Header names are lower case. `path` is the raw (still percent-encoded)
/// path without the query string.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn new(method: &str, target: &str) -> Request {
        let (path, query) = match target.split_once('?') {
            Some((p, q)) => (p, q),
            None => (target, ""),
        };
        Request {
            method: method.to_ascii_uppercase(),
            path: path.to_string(),
            query: query.to_string(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Request {
        self.headers
            .push((name.to_ascii_lowercase(), value.to_string()));
        self
    }

    pub fn with_body(mut self, body: &[u8]) -> Request {
        self.body = body.to_vec();
        self
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn query_params(&self) -> HashMap<String, String> {
        parse_query(&self.query)
    }

    /// Path segments, percent-decoded. `None` if any segment is not valid UTF-8 or contains an
    /// encoded `/`, NUL, or is `..` / `.` (path traversal).
    pub fn segments(&self) -> Option<Vec<String>> {
        split_path(&self.path)
    }
}

/// Splits `/a/b%20c/` into `["a", "b c"]`, rejecting traversal attempts.
pub fn split_path(path: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for raw in path.split('/') {
        if raw.is_empty() {
            continue;
        }
        let dec = percent_decode_str(raw).decode_utf8().ok()?;
        if dec == ".."
            || dec == "."
            || dec.contains('/')
            || dec.contains('\\')
            || dec.contains('\0')
        {
            return None;
        }
        out.push(dec.into_owned());
    }
    Some(out)
}

/// Decodes `a=1&b=two%20words&c=x+y` (plus means space).
pub fn parse_query(q: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for pair in q.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        out.insert(decode_component(k), decode_component(v));
    }
    out
}

fn decode_component(s: &str) -> String {
    let plus = s.replace('+', " ");
    percent_decode_str(&plus).decode_utf8_lossy().into_owned()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status: u16, content_type: &str, body: Vec<u8>) -> Response {
        Response {
            status,
            headers: vec![("Content-Type".into(), content_type.into())],
            body,
        }
    }

    pub fn json(status: u16, value: &Value) -> Response {
        Response::new(
            status,
            "application/json",
            serde_json::to_vec(value).unwrap_or_default(),
        )
    }

    pub fn text(status: u16, text: &str) -> Response {
        Response::new(
            status,
            "text/plain; charset=utf-8",
            text.as_bytes().to_vec(),
        )
    }

    pub fn html(status: u16, html: String) -> Response {
        Response::new(status, "text/html; charset=utf-8", html.into_bytes())
    }

    pub fn empty(status: u16) -> Response {
        Response {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Response {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn body_str(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Result of interpreting a `Range` header against a resource of known length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeSpec {
    /// No usable header (absent, other unit, or several ranges): send the whole resource.
    Full,
    /// Inclusive byte range to send.
    Bytes { start: u64, end: u64 },
    /// Syntactically wrong or outside the resource: answer 416.
    Unsatisfiable,
}

/// Parses `bytes=a-b`, `bytes=a-`, `bytes=-n`.
pub fn parse_range(header: Option<&str>, len: u64) -> RangeSpec {
    let Some(h) = header else {
        return RangeSpec::Full;
    };
    let h = h.trim();
    let Some(spec) = h.strip_prefix("bytes=") else {
        return RangeSpec::Full;
    };
    if spec.contains(',') {
        return RangeSpec::Full;
    }
    let Some((a, b)) = spec.trim().split_once('-') else {
        return RangeSpec::Unsatisfiable;
    };
    let (a, b) = (a.trim(), b.trim());
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
    if a.is_empty() {
        // suffix range: last n bytes
        if !digits(b) {
            return RangeSpec::Unsatisfiable;
        }
        let Ok(n) = b.parse::<u64>() else {
            return RangeSpec::Unsatisfiable;
        };
        if n == 0 || len == 0 {
            return RangeSpec::Unsatisfiable;
        }
        return RangeSpec::Bytes {
            start: len.saturating_sub(n),
            end: len - 1,
        };
    }
    if !digits(a) || !(b.is_empty() || digits(b)) {
        return RangeSpec::Unsatisfiable;
    }
    let Ok(start) = a.parse::<u64>() else {
        return RangeSpec::Unsatisfiable;
    };
    let end = if b.is_empty() {
        u64::MAX
    } else {
        match b.parse::<u64>() {
            Ok(e) => e,
            Err(_) => return RangeSpec::Unsatisfiable,
        }
    };
    if start >= len || end < start {
        return RangeSpec::Unsatisfiable;
    }
    RangeSpec::Bytes {
        start,
        end: end.min(len - 1),
    }
}

/// Extracts `(user, password)` from an `Authorization: Basic ...` header value.
pub fn parse_basic_auth(header: &str) -> Option<(String, String)> {
    let (scheme, rest) = header.trim().split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return None;
    }
    let raw = base64::engine::general_purpose::STANDARD
        .decode(rest.trim())
        .ok()?;
    let s = String::from_utf8(raw).ok()?;
    let (u, p) = s.split_once(':')?;
    Some((u.to_string(), p.to_string()))
}

/// Compares two byte strings without short-circuiting on the first difference.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = (a.len() ^ b.len()) as u64;
    let n = a.len().max(b.len());
    for i in 0..n {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        diff |= (x ^ y) as u64;
    }
    diff == 0
}

/// MIME type for an image or book file extension.
pub fn mime_for_extension(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "epub" => "application/epub+zip",
        "kepub" => "application/epub+zip",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "mobi" => "application/x-mobipocket-ebook",
        "azw3" => "application/vnd.amazon.ebook",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_splits_target() {
        let r = Request::new("get", "/a/b?x=1&y=2");
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, "/a/b");
        assert_eq!(r.query, "x=1&y=2");
        let r = Request::new("GET", "/plain");
        assert_eq!(r.query, "");
    }

    #[test]
    fn headers_are_case_insensitive() {
        let r = Request::new("GET", "/").with_header("X-Auth-User", "bob");
        assert_eq!(r.header("x-auth-user"), Some("bob"));
        assert_eq!(r.header("X-AUTH-USER"), Some("bob"));
        assert_eq!(r.header("nope"), None);
    }

    #[test]
    fn query_decoding() {
        let q = parse_query("q=hello+world&tag=a%26b&empty=&flag&u=%E6%97%A5");
        assert_eq!(q["q"], "hello world");
        assert_eq!(q["tag"], "a&b");
        assert_eq!(q["empty"], "");
        assert_eq!(q["flag"], "");
        assert_eq!(q["u"], "日");
        assert!(parse_query("").is_empty());
        assert!(parse_query("&&").is_empty());
    }

    #[test]
    fn invalid_utf8_query_is_lossy_not_a_panic() {
        let q = parse_query("q=%FF%FE");
        assert!(q["q"].contains('\u{FFFD}'));
    }

    #[test]
    fn path_segments_decode() {
        assert_eq!(
            split_path("/opds/authors/Le%20Guin/").unwrap(),
            vec!["opds", "authors", "Le Guin"]
        );
        assert_eq!(split_path("/").unwrap(), Vec::<String>::new());
        assert_eq!(split_path("//a//b").unwrap(), vec!["a", "b"]);
    }

    #[test]
    fn path_traversal_is_rejected() {
        for p in [
            "/../etc/passwd",
            "/a/%2e%2e/b",
            "/a/%2E%2E",
            "/a/./b",
            "/a%2Fb",
            "/a%5Cb",
            "/a/..%2f..",
            "/a/b%00",
            "/%FF",
        ] {
            assert!(split_path(p).is_none(), "{p} should be rejected");
        }
    }

    #[test]
    fn range_none_and_other_units_mean_full() {
        assert_eq!(parse_range(None, 10), RangeSpec::Full);
        assert_eq!(parse_range(Some("items=0-1"), 10), RangeSpec::Full);
        assert_eq!(parse_range(Some("bytes=0-1,3-4"), 10), RangeSpec::Full);
    }

    #[test]
    fn range_single() {
        assert_eq!(
            parse_range(Some("bytes=2-5"), 10),
            RangeSpec::Bytes { start: 2, end: 5 }
        );
        assert_eq!(
            parse_range(Some("bytes=0-0"), 10),
            RangeSpec::Bytes { start: 0, end: 0 }
        );
        assert_eq!(
            parse_range(Some("bytes=5-"), 10),
            RangeSpec::Bytes { start: 5, end: 9 }
        );
        // end past the resource is clamped
        assert_eq!(
            parse_range(Some("bytes=5-999"), 10),
            RangeSpec::Bytes { start: 5, end: 9 }
        );
        assert_eq!(
            parse_range(Some("bytes=0-99999999999999999999999"), 10),
            RangeSpec::Unsatisfiable
        );
    }

    #[test]
    fn range_suffix() {
        assert_eq!(
            parse_range(Some("bytes=-3"), 10),
            RangeSpec::Bytes { start: 7, end: 9 }
        );
        // longer than the file: whole file
        assert_eq!(
            parse_range(Some("bytes=-30"), 10),
            RangeSpec::Bytes { start: 0, end: 9 }
        );
        assert_eq!(parse_range(Some("bytes=-0"), 10), RangeSpec::Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=-5"), 0), RangeSpec::Unsatisfiable);
    }

    #[test]
    fn range_unsatisfiable() {
        for h in [
            "bytes=10-",
            "bytes=10-12",
            "bytes=5-2",
            "bytes=abc",
            "bytes=a-b",
            "bytes=-",
            "bytes=",
            "bytes=-x",
            "bytes=1-x",
            "bytes=+1-2",
        ] {
            assert_eq!(parse_range(Some(h), 10), RangeSpec::Unsatisfiable, "{h}");
        }
        assert_eq!(parse_range(Some("bytes=0-0"), 0), RangeSpec::Unsatisfiable);
    }

    #[test]
    fn basic_auth_parsing() {
        // "alice:s3cret:with:colons"
        let h = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("alice:s3cret:with:colons")
        );
        assert_eq!(
            parse_basic_auth(&h),
            Some(("alice".into(), "s3cret:with:colons".into()))
        );
        assert_eq!(
            parse_basic_auth("basic YTpi"),
            Some(("a".into(), "b".into()))
        );
        assert_eq!(parse_basic_auth("Bearer abc"), None);
        assert_eq!(parse_basic_auth("Basic !!!notbase64"), None);
        assert_eq!(parse_basic_auth("Basic bm9jb2xvbg=="), None);
        assert_eq!(parse_basic_auth("Basic"), None);
        assert_eq!(parse_basic_auth(""), None);
    }

    #[test]
    fn constant_time_compare() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(constant_time_eq(b"", b""));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"abcd", b"abc"));
        assert!(!constant_time_eq(b"", b"\0"));
    }

    #[test]
    fn mime_types() {
        assert_eq!(mime_for_extension("EPUB"), "application/epub+zip");
        assert_eq!(mime_for_extension("jpeg"), "image/jpeg");
        assert_eq!(mime_for_extension("png"), "image/png");
        assert_eq!(mime_for_extension("xyz"), "application/octet-stream");
    }

    #[test]
    fn response_helpers() {
        let r = Response::json(201, &serde_json::json!({"a": 1})).with_header("X-A", "b");
        assert_eq!(r.status, 201);
        assert_eq!(r.header("content-type"), Some("application/json"));
        assert_eq!(r.header("x-a"), Some("b"));
        assert_eq!(r.body_str(), "{\"a\":1}");
        assert_eq!(Response::text(200, "hi").body, b"hi");
        assert!(Response::html(200, "<p>".into())
            .header("content-type")
            .unwrap()
            .starts_with("text/html"));
        assert!(Response::empty(204).body.is_empty());
    }
}
