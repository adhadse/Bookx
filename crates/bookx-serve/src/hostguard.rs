//! `Host` and `Origin` checks for every listener (content server, OPDS, `/bookx` sync, MCP).
//!
//! A web page can aim a browser at a server on `127.0.0.1` or on the LAN in two ways:
//!
//! * **DNS rebinding**: `evil.example` first resolves to the attacker, then to `127.0.0.1`. The
//!   browser now treats the local server as same-origin with the page, so the page can read the
//!   library and send requests without any `Origin` header on simple GETs. The tell is the `Host`
//!   header: it still says `evil.example`. A server that only answers to names it knows is immune.
//! * **Cross-site requests** (CSRF): a form or `fetch` from another site posts to the server. The
//!   browser says where it came from in `Origin` (and `Sec-Fetch-Site`).
//!
//! Allowed `Host` values: an IP literal (rebinding needs a name), a single-label name (`localhost`,
//! `mybox`: it cannot be a public registered domain), `*.local`, `*.localhost`, `*.home.arpa`, `*.lan`,
//! `*.internal` (none of these can be registered by a web attacker), the
//! machine's own host name, and anything the user listed (`BOOKX_ALLOWED_HOSTS`, `--allow-host`,
//! the public name behind a reverse proxy). Native clients (KOReader, OPDS apps, curl) send no
//! `Origin` and are not affected.

use std::net::IpAddr;

use crate::http::{Request, Response};

/// The lower-case host of a `Host` header or authority: no port, brackets or trailing dot.
pub fn host_name(h: &str) -> String {
    let h = h.trim();
    let name = if let Some(rest) = h.strip_prefix('[') {
        // an unclosed bracket is malformed: no name, so never allowed
        rest.split_once(']').map_or("", |(inner, _)| inner)
    } else {
        h.rsplit_once(':')
            .filter(|(_, port)| port.bytes().all(|b| b.is_ascii_digit()))
            .map_or(h, |(n, _)| n)
    };
    name.trim_end_matches('.').to_ascii_lowercase()
}

/// The names the user allowed through `BOOKX_ALLOWED_HOSTS` (comma or space separated) and the
/// machine's own host name.
pub fn default_allowed_hosts() -> Vec<String> {
    let mut v: Vec<String> = std::env::var("BOOKX_ALLOWED_HOSTS")
        .unwrap_or_default()
        .split([',', ' ', ';'])
        .map(host_name)
        .filter(|h| !h.is_empty())
        .collect();
    if let Ok(h) = std::fs::read_to_string("/proc/sys/kernel/hostname") {
        let h = host_name(&h);
        if !h.is_empty() {
            v.push(h);
        }
    }
    v
}

/// Parses what a user typed into a list of allowed names (comma, space, semicolon or line
/// separated; a port or trailing dot is dropped). `Err` holds the first item that is not a plain
/// host name or IP address (a URL, a path, a wildcard, an `@`).
pub fn parse_host_list(s: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for item in s
        .split([',', ' ', ';', '\n', '\t'])
        .filter(|x| !x.is_empty())
    {
        let h = host_name(item);
        let labels_ok = h.parse::<IpAddr>().is_ok()
            || (name_chars_ok(&h)
                && !h.contains(':')
                && h.split('.').all(|l| {
                    !l.is_empty() && !l.starts_with('-') && !l.ends_with('-') && l.len() <= 63
                }));
        if !labels_ok || item.contains(['/', '@', '*', '?', '#', '\\']) {
            return Err(item.to_string());
        }
        if !out.contains(&h) {
            out.push(h);
        }
    }
    Ok(out)
}

fn name_chars_ok(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 253
        && h.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':'))
}

/// May a request with this `Host` header be answered?
pub fn host_allowed(host_header: &str, extra: &[String]) -> bool {
    let h = host_name(host_header);
    if !name_chars_ok(&h) {
        return false;
    }
    if extra.iter().any(|e| host_name(e) == h) {
        return true;
    }
    if h.parse::<IpAddr>().is_ok() {
        return true;
    }
    if !h.contains('.') && !h.contains(':') {
        return true;
    }
    [".local", ".localhost", ".home.arpa", ".lan", ".internal"]
        .iter()
        .any(|s| h.ends_with(s) && h.len() > s.len())
}

/// Does `Origin` come from the page served by this very server (or a name the user allowed)?
pub fn origin_allowed(origin: &str, host_header: Option<&str>, extra: &[String]) -> bool {
    let Some((scheme, rest)) = origin.split_once("://") else {
        return false; // `null`, empty, garbage
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') || authority.is_empty() {
        return false;
    }
    if let Some(h) = host_header {
        if authority.eq_ignore_ascii_case(h.trim()) && host_allowed(h, extra) {
            return true;
        }
    }
    let name = host_name(authority);
    extra.iter().any(|e| host_name(e) == name) && name_chars_ok(&name)
}

fn forbidden(msg: &str) -> Response {
    Response::text(403, msg)
}

/// `Some(403)` when the request must not be served: a `Host` we do not know, an `Origin` that is
/// not this server, or a browser that says the request is cross-site.
pub fn check(req: &Request, extra: &[String]) -> Option<Response> {
    let host = req.header("host");
    if let Some(h) = host {
        if !host_allowed(h, extra) {
            return Some(forbidden(
                "Host not allowed: use the address shown in Bookx, or list this name in BOOKX_ALLOWED_HOSTS",
            ));
        }
    }
    if let Some(o) = req.header("origin") {
        if !origin_allowed(o, host, extra) {
            return Some(forbidden("Cross-origin requests are not allowed"));
        }
    }
    if req
        .header("sec-fetch-site")
        .is_some_and(|s| s.eq_ignore_ascii_case("cross-site"))
        && req.method != "GET"
        && req.method != "HEAD"
    {
        return Some(forbidden("Cross-site requests are not allowed"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> Vec<String> {
        Vec::new()
    }

    #[test]
    fn host_lists_are_validated() {
        assert_eq!(
            parse_host_list("Books.Example.com:8443, nas ; 192.168.1.5\nbooks.example.com.")
                .unwrap(),
            vec!["books.example.com", "nas", "192.168.1.5"]
        );
        assert!(parse_host_list("  ").unwrap().is_empty());
        for bad in [
            "https://x.com",
            "x.com/path",
            "*.example.com",
            "a@b.com",
            "-x.com",
            "a..b",
            "a_b.com/",
            "x.com?y",
        ] {
            assert_eq!(
                parse_host_list(&format!("ok.example {bad}")),
                Err(bad.to_string()),
                "{bad}"
            );
        }
        assert!(parse_host_list("[::1]").is_ok());
    }

    #[test]
    fn host_names_are_normalised() {
        assert_eq!(host_name("LocalHost:8080"), "localhost");
        assert_eq!(host_name("[::1]:8080"), "::1");
        assert_eq!(host_name("[::1]"), "::1");
        assert_eq!(host_name("example.com."), "example.com");
        assert_eq!(host_name(" 127.0.0.1 "), "127.0.0.1");
        assert_eq!(host_name("[::1"), "", "an unclosed bracket is no host");
    }

    #[test]
    fn local_names_and_addresses_pass() {
        for h in [
            "127.0.0.1:8080",
            "127.0.0.1",
            "[::1]:8080",
            "192.168.1.20:8080",
            "10.0.0.5",
            "[fe80::1]",
            "localhost",
            "LOCALHOST:8080",
            "mybox",
            "mybox:8080",
            "bookx.local",
            "my-pc.local:8080",
            "a.localhost",
            "router.home.arpa",
            "bookx.lan",
            "nas.internal:8080",
        ] {
            assert!(host_allowed(h, &none()), "{h} should pass");
        }
    }

    #[test]
    fn rebinding_names_and_malformed_hosts_are_refused() {
        for h in [
            "evil.example",
            "evil.example:8080",
            "127.0.0.1.evil.example",
            "localhost.evil.example",
            "localhost.evil.example:80",
            "www.attacker.com",
            "evil.com.",
            "a.b.c.d.e",
            ".local",
            ".localhost",
            "",
            "   ",
            "host with space",
            "evil.example/x",
            "user@127.0.0.1",
            "127.0.0.1@evil.example",
            "exa\u{0}mple",
            "\u{65e5}\u{672c}.example",
            "xn--e1afmkfd.xn--p1ai",
        ] {
            assert!(!host_allowed(h, &none()), "{h:?} must be refused");
        }
    }

    #[test]
    fn listed_names_pass_even_with_dots() {
        let extra = vec![
            "books.example.com".to_string(),
            "NAS.fritz.box:8080".to_string(),
        ];
        assert!(host_allowed("books.example.com", &extra));
        assert!(host_allowed("Books.Example.com:443", &extra));
        assert!(host_allowed("nas.fritz.box", &extra));
        assert!(!host_allowed("other.example.com", &extra));
        assert!(!host_allowed("evilbooks.example.com", &extra));
    }

    #[test]
    fn origin_must_be_the_server_itself() {
        let e = none();
        assert!(origin_allowed(
            "http://localhost:8080",
            Some("localhost:8080"),
            &e
        ));
        assert!(origin_allowed(
            "http://127.0.0.1:8080",
            Some("127.0.0.1:8080"),
            &e
        ));
        assert!(origin_allowed("https://MyBox:8443", Some("mybox:8443"), &e));
        assert!(!origin_allowed(
            "http://localhost:3000",
            Some("localhost:8080"),
            &e
        ));
        assert!(!origin_allowed(
            "http://evil.example",
            Some("localhost:8080"),
            &e
        ));
        assert!(!origin_allowed(
            "http://evil.example",
            Some("evil.example"),
            &e
        ));
        assert!(!origin_allowed("null", Some("localhost"), &e));
        assert!(!origin_allowed("", Some("localhost"), &e));
        assert!(!origin_allowed("file://", Some("localhost"), &e));
        assert!(!origin_allowed("chrome-extension://abc", Some("abc"), &e));
        assert!(!origin_allowed(
            "http://localhost@evil.example",
            Some("localhost"),
            &e
        ));
        assert!(!origin_allowed("http://localhost", None, &e));
        let extra = vec!["app.example.org".to_string()];
        assert!(origin_allowed(
            "https://app.example.org",
            Some("localhost:8080"),
            &extra
        ));
        assert!(!origin_allowed(
            "https://other.example.org",
            Some("localhost:8080"),
            &extra
        ));
    }

    #[test]
    fn check_rejects_the_browser_attacks_and_lets_native_clients_through() {
        let e = none();
        let get = |h: &str| Request::new("GET", "/opds").with_header("Host", h);
        assert!(check(&get("127.0.0.1:8080"), &e).is_none());
        // rebinding: same-origin from the browser's view, Host gives it away
        assert_eq!(check(&get("evil.example:8080"), &e).unwrap().status, 403);
        // no Host at all (HTTP/1.0 tools) is allowed
        assert!(check(&Request::new("GET", "/opds"), &e).is_none());
        // native clients send no Origin
        let post = Request::new("POST", "/bookx/users/create").with_header("Host", "mybox:8080");
        assert!(check(&post, &e).is_none());
        // CSRF from another site
        let csrf = post
            .clone()
            .with_header("Origin", "https://evil.example")
            .with_header("Sec-Fetch-Site", "cross-site");
        assert_eq!(check(&csrf, &e).unwrap().status, 403);
        let text_plain = Request::new("POST", "/bookx/users/create")
            .with_header("Host", "127.0.0.1:8080")
            .with_header("Origin", "null");
        assert_eq!(check(&text_plain, &e).unwrap().status, 403);
        // cross-site with a forged-away Origin is still caught by Sec-Fetch-Site
        let sfs = post.clone().with_header("Sec-Fetch-Site", "cross-site");
        assert_eq!(check(&sfs, &e).unwrap().status, 403);
        // a cross-site navigation (link click) to a GET page is not a mutation
        let nav = get("127.0.0.1:8080").with_header("Sec-Fetch-Site", "cross-site");
        assert!(check(&nav, &e).is_none());
        // same-origin page of the server itself
        let own = Request::new("POST", "/bookx/users/create")
            .with_header("Host", "127.0.0.1:8080")
            .with_header("Origin", "http://127.0.0.1:8080")
            .with_header("Sec-Fetch-Site", "same-origin");
        assert!(check(&own, &e).is_none());
    }
}
