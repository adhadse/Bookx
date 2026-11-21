//! A small installable web client for the content server (`/app/`): the library list, search, and
//! books saved on the device for reading offline. It is plain files embedded in the binary (no
//! build step, no third-party code, no request to any other site). The page and the service worker
//! may only talk to the server they came from: `Content-Security-Policy` allows scripts, styles
//! and connections from `'self'` only and no inline script, and the script writes text only with
//! `textContent`.
//!
//! The page itself needs the login of the server (it is the catalogue's front door); the files a
//! browser fetches without credentials (manifest, icon, script, style, service worker) are public
//! static text with nothing of the library in them.

use crate::http::{Request, Response};

const INDEX: &str = include_str!("pwa/index.html");
const JS: &str = include_str!("pwa/app.js");
const CSS: &str = include_str!("pwa/app.css");
const SW: &str = include_str!("pwa/sw.js");
const MANIFEST: &str = include_str!("pwa/manifest.webmanifest");
const ICON: &str = include_str!("pwa/icon.svg");

/// The policy of every page and script of the client.
pub const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self'; manifest-src 'self'; worker-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// Does this path (below any public prefix) belong to the client?
pub fn owns(path: &str) -> bool {
    path == "/app" || path.starts_with("/app/")
}

/// Needs no login: everything of the client except the page itself.
pub fn is_public(path: &str) -> bool {
    matches!(
        path,
        "/app/app.js"
            | "/app/app.css"
            | "/app/sw.js"
            | "/app/manifest.webmanifest"
            | "/app/icon.svg"
    )
}

/// Answers a `GET`/`HEAD` of the client. `sw_scope` is the widest scope the service worker may
/// take (`/` or the public prefix plus `/`).
pub fn serve(req: &Request, sw_scope: &str) -> Response {
    if req.method != "GET" && req.method != "HEAD" {
        return Response::text(405, "Method not allowed").with_header("Allow", "GET, HEAD");
    }
    let (ctype, body) = match req.path.as_str() {
        "/app" | "/app/" | "/app/index.html" => ("text/html; charset=utf-8", INDEX),
        "/app/app.js" => ("text/javascript; charset=utf-8", JS),
        "/app/app.css" => ("text/css; charset=utf-8", CSS),
        "/app/sw.js" => ("text/javascript; charset=utf-8", SW),
        "/app/manifest.webmanifest" => ("application/manifest+json", MANIFEST),
        "/app/icon.svg" => ("image/svg+xml", ICON),
        _ => return Response::text(404, "Not found"),
    };
    let mut r = Response::new(200, ctype, body.as_bytes().to_vec())
        .with_header("Content-Security-Policy", CSP)
        .with_header("Cache-Control", "no-cache")
        .with_header("X-Content-Type-Options", "nosniff")
        .with_header("Referrer-Policy", "no-referrer");
    if req.path == "/app/sw.js" {
        r = r.with_header("Service-Worker-Allowed", sw_scope);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_asset_is_served_with_the_strict_policy() {
        for p in [
            "/app",
            "/app/",
            "/app/app.js",
            "/app/app.css",
            "/app/sw.js",
            "/app/manifest.webmanifest",
            "/app/icon.svg",
        ] {
            let r = serve(&Request::new("GET", p), "/");
            assert_eq!(r.status, 200, "{p}");
            assert_eq!(r.header("content-security-policy"), Some(CSP), "{p}");
            assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
            assert!(!r.body.is_empty());
        }
        assert_eq!(serve(&Request::new("GET", "/app/nope"), "/").status, 404);
        assert_eq!(serve(&Request::new("POST", "/app/"), "/").status, 405);
        assert_eq!(
            serve(&Request::new("GET", "/app/sw.js"), "/books/").header("service-worker-allowed"),
            Some("/books/")
        );
    }

    #[test]
    fn the_policy_allows_nothing_from_outside() {
        assert!(!CSP.contains("unsafe-inline") && !CSP.contains("unsafe-eval"));
        assert!(!CSP.contains("http:") && !CSP.contains("https:") && !CSP.contains('*'));
        assert!(CSP.starts_with("default-src 'none'"));
    }

    #[test]
    fn no_file_loads_anything_from_another_site_or_runs_inline() {
        for (name, text) in [
            ("index", INDEX),
            ("js", JS),
            ("css", CSS),
            ("sw", SW),
            ("manifest", MANIFEST),
        ] {
            assert!(!text.contains("http://") || name == "js", "{name}");
            assert!(!text.contains("https://"), "{name}");
            assert!(!text.contains("@import"), "{name}");
        }
        // the only absolute URLs in the script are XML and OPDS namespace names
        for line in JS.lines().filter(|l| l.contains("http://")) {
            assert!(
                line.contains("w3.org/2005/Atom") || line.contains("opds-spec.org"),
                "{line}"
            );
        }
        // no inline script or handler in the page, and no markup built from catalogue text
        assert!(!INDEX.contains("<script>") && !INDEX.contains(" onclick="));
        for bad in [
            "innerHTML",
            "outerHTML",
            "insertAdjacentHTML",
            "document.write",
            "eval(",
            "new Function",
        ] {
            assert!(!JS.contains(bad) && !SW.contains(bad), "{bad}");
        }
        // the page links only its own files
        for l in INDEX
            .lines()
            .filter(|l| l.contains("href=") || l.contains("src="))
        {
            assert!(!l.contains("//"), "{l}");
        }
    }

    #[test]
    fn the_manifest_is_valid_json_with_relative_urls() {
        let v: serde_json::Value = serde_json::from_str(MANIFEST).unwrap();
        assert_eq!(v["start_url"], "./");
        assert_eq!(v["scope"], "./");
        assert_eq!(v["display"], "standalone");
        assert_eq!(v["icons"][0]["src"], "icon.svg");
    }

    #[test]
    fn ownership() {
        assert!(owns("/app") && owns("/app/x") && !owns("/apple") && !owns("/opds"));
        assert!(is_public("/app/sw.js") && !is_public("/app/") && !is_public("/app"));
    }
}
