//! The HTTP content server: OPDS, downloads (with `Range`), covers, a tiny HTML page and the
//! KOSync routes, behind optional HTTP basic auth.
//!
//! [`handle`] is a pure function of the request and the library snapshot; [`Server::start`] is the
//! hardened blocking transport in [`crate::net`] around it (size limits, deadlines, connection cap).
//!
//! Auth: when users are configured every route needs HTTP basic auth, except `/healthcheck` and
//! the KOSync protocol routes, which carry their own `x-auth-user` / `x-auth-key` headers
//! (KOReader cannot send basic auth there).
//!
//! Read-only mode answers only `GET`/`HEAD` (405 otherwise), which also switches off KOSync writes.

use std::io::{Read, Seek, SeekFrom};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use bookx_core::library::Book;
use bookx_core::query::Query;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::json;

use crate::accounts::{self, Access, Account};
use crate::annotation_sync::{self, AnnotationSync};
use crate::devices::sanitize_file_name;
use crate::http::{
    RangeSpec, Request, Response, constant_time_eq, mime_for_extension, parse_basic_auth,
    parse_range,
};
use crate::kosync::KoSync;
use crate::net::{self, Limits};
use crate::opds::{self, OpdsConfig};
use crate::xml::escape;

/// A consistent view of the library for one request.
#[derive(Debug, Clone, Default)]
pub struct LibrarySnapshot {
    /// Directory the books' relative `file` / `cover` paths live in.
    pub root: PathBuf,
    pub books: Vec<Book>,
}

pub type LibraryProvider = Arc<dyn Fn() -> LibrarySnapshot + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub name: String,
    pub password: String,
}

/// The TLS server configuration type of [`ServerConfig::tls`] (built by [`net::tls_config`]).
pub type TlsConfig = rustls::ServerConfig;

#[derive(Clone)]
pub struct ServerConfig {
    /// Address to bind, `127.0.0.1` by default. Use `0.0.0.0` to serve the LAN.
    pub bind: String,
    /// `0` picks a free port.
    pub port: u16,
    pub title: String,
    pub page_size: usize,
    /// Basic-auth users; empty means open access.
    pub users: Vec<User>,
    pub read_only: bool,
    pub kosync: Option<Arc<KoSync>>,
    /// KOReader annotation sync (`/bookx/annotations/...`); needs `kosync` for the accounts.
    pub annotations: Option<Arc<AnnotationSync>>,
    pub library: LibraryProvider,
    /// Requests handled at the same time.
    pub threads: usize,
    /// Size, time and connection limits of the transport (see [`Limits`] for the defaults).
    pub limits: Limits,
    /// Serve HTTPS with this configuration (see [`net::tls_config`]); plain HTTP when `None`.
    pub tls: Option<Arc<rustls::ServerConfig>>,
    /// Host names besides IP addresses, single-label and `.local` names that may appear in the
    /// `Host` header (the public name behind a reverse proxy, a `.lan` name). Starts from
    /// `$BOOKX_ALLOWED_HOSTS` and the machine's host name; see [`crate::hostguard`].
    pub allowed_hosts: Vec<String>,
    /// The sub-path a reverse proxy publishes the server under (`/books`), no trailing slash,
    /// empty at the root. Links in the web page and the OPDS feeds start with it; requests may
    /// arrive with or without it (a proxy may keep or strip it). The `/bookx` sync routes need no
    /// setting: the device URL is `https://host/books/bookx`.
    pub public_prefix: String,
    /// Logins with their own password and the collections they may see (see [`crate::accounts`]).
    pub accounts: Vec<Account>,
    /// Collections a visitor without a login may read; `None` means no guest access.
    pub guest: Option<Vec<String>>,
}

/// Normalises a user-typed prefix to `/name` or `/a/b` (`"books/"` becomes `/books`); `None` for
/// something that is not a plain path (spaces, `..`, `?`, `#`, a scheme).
pub fn clean_public_prefix(s: &str) -> Option<String> {
    let t = s.trim().trim_matches('/');
    if t.is_empty() {
        return Some(String::new());
    }
    let ok = t.split('/').all(|seg| {
        !seg.is_empty()
            && seg != "."
            && seg != ".."
            && seg
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~'))
    });
    // the fixed routes must stay reachable
    ok.then(|| format!("/{t}")).filter(|p| {
        !matches!(
            p.as_str(),
            "/bookx" | "/opds" | "/download" | "/cover" | "/healthcheck"
        )
    })
}

impl ServerConfig {
    pub fn new(library: LibraryProvider) -> ServerConfig {
        ServerConfig {
            bind: "127.0.0.1".into(),
            port: 0,
            title: "Bookx Library".into(),
            page_size: 50,
            users: Vec::new(),
            read_only: false,
            kosync: None,
            annotations: None,
            library,
            threads: 8,
            limits: Limits::default(),
            tls: None,
            allowed_hosts: crate::hostguard::default_allowed_hosts(),
            public_prefix: String::new(),
            accounts: Vec::new(),
            guest: None,
        }
    }
}

const INDEX_LIMIT: usize = 200;

/// True if `header` carries valid credentials for one of `users` (all users are always compared).
pub fn check_auth(users: &[User], header: Option<&str>) -> bool {
    let Some((u, p)) = header.and_then(parse_basic_auth) else {
        return false;
    };
    let mut ok = false;
    for user in users {
        let a = constant_time_eq(user.name.as_bytes(), u.as_bytes());
        let b = constant_time_eq(user.password.as_bytes(), p.as_bytes());
        ok |= a & b;
    }
    ok
}

const FILENAME: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// `attachment; filename="ascii"; filename*=UTF-8''...`
pub fn content_disposition(name: &str) -> String {
    let ascii: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                if c == '"' || c == '\\' { '_' } else { c }
            } else {
                '_'
            }
        })
        .collect();
    format!(
        "attachment; filename=\"{ascii}\"; filename*=UTF-8''{}",
        utf8_percent_encode(name, FILENAME)
    )
}

fn error(status: u16, msg: &str) -> Response {
    Response::text(status, msg)
}

fn unauthorized() -> Response {
    error(401, "Authentication required").with_header(
        "WWW-Authenticate",
        "Basic realm=\"Bookx\", charset=\"UTF-8\"",
    )
}

/// Renders the HTML front page (everything user-controlled is escaped).
pub fn render_index(title: &str, q: &str, books: &[&Book], total: usize) -> String {
    render_index_at("", title, q, books, total)
}

/// [`render_index`] for a server published under `base` (see [`ServerConfig::public_prefix`]).
pub fn render_index_at(base: &str, title: &str, q: &str, books: &[&Book], total: usize) -> String {
    let mut s = String::new();
    s.push_str("<!DOCTYPE html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">");
    s.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    s.push_str(&format!("<title>{}</title>", escape(title)));
    s.push_str("<style>body{font-family:sans-serif;max-width:48rem;margin:1rem auto;padding:0 1rem}li{margin:.4rem 0}img{height:3rem;vertical-align:middle;margin-right:.5rem}</style>");
    s.push_str("</head><body>");
    s.push_str(&format!("<h1>{}</h1>", escape(title)));
    s.push_str(&format!(
        "<form action=\"{}/\" method=\"get\"><input type=\"search\" name=\"q\" value=\"{}\" placeholder=\"Search\"> <button type=\"submit\">Search</button></form>",
        escape(base),
        escape(q)
    ));
    s.push_str(&format!(
        "<p><a href=\"{0}/opds\">OPDS catalogue</a> · <a href=\"{0}/app/\">App</a></p>",
        escape(base)
    ));
    if total > books.len() {
        s.push_str(&format!(
            "<p>Showing {} of {} books.</p>",
            books.len(),
            total
        ));
    } else {
        s.push_str(&format!(
            "<p>{total} book{}.</p>",
            if total == 1 { "" } else { "s" }
        ));
    }
    s.push_str("<ul>");
    for b in books {
        s.push_str("<li>");
        if b.cover.is_some() {
            s.push_str(&format!(
                "<img src=\"{}\" alt=\"\">",
                escape(&format!("{base}{}", opds::thumbnail_href(b.id)))
            ));
        }
        s.push_str(&format!(
            "<a href=\"{}\">{}</a> <small>{}</small>",
            escape(&format!("{base}{}", opds::download_href(b.id))),
            escape(if b.meta.title.trim().is_empty() {
                "Untitled"
            } else {
                &b.meta.title
            }),
            escape(&b.meta.authors_display())
        ));
        s.push_str("</li>");
    }
    s.push_str("</ul></body></html>");
    s
}

fn read_range(path: &Path, start: u64, len: u64) -> std::io::Result<Vec<u8>> {
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity(len.min(64 * 1024 * 1024) as usize);
    f.take(len).read_to_end(&mut buf)?;
    Ok(buf)
}

/// Resolves `rel` below `root`, refusing anything that ends up outside it (symlinks, `..`).
fn safe_join(root: &Path, rel: &str) -> Option<PathBuf> {
    let root = std::fs::canonicalize(root).ok()?;
    let p = std::fs::canonicalize(root.join(rel)).ok()?;
    (p.starts_with(&root) && p.is_file()).then_some(p)
}

fn serve_file(req: &Request, path: &Path, mime: &str, disposition: Option<String>) -> Response {
    let Ok(meta) = std::fs::metadata(path) else {
        return error(404, "Not found");
    };
    let len = meta.len();
    let resp = |status: u16, body: Vec<u8>| {
        let mut r = Response::new(status, mime, body).with_header("Accept-Ranges", "bytes");
        if let Some(d) = &disposition {
            r = r.with_header("Content-Disposition", d);
        }
        r.with_header("X-Content-Type-Options", "nosniff")
    };
    match parse_range(req.header("range"), len) {
        RangeSpec::Full => match std::fs::read(path) {
            Ok(b) => resp(200, b),
            Err(_) => error(404, "Not found"),
        },
        RangeSpec::Bytes { start, end } => match read_range(path, start, end - start + 1) {
            Ok(b) => {
                resp(206, b).with_header("Content-Range", &format!("bytes {start}-{end}/{len}"))
            }
            Err(_) => error(404, "Not found"),
        },
        RangeSpec::Unsatisfiable => error(416, "Range not satisfiable")
            .with_header("Content-Range", &format!("bytes */{len}"))
            .with_header("Accept-Ranges", "bytes"),
    }
}

fn parse_id(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) || s.len() > 19 {
        return None;
    }
    s.parse().ok()
}

fn route_download(req: &Request, snap: &LibrarySnapshot, file: &str) -> Response {
    let Some(id) = file.strip_suffix(".epub").and_then(parse_id) else {
        return error(404, "Not found");
    };
    let Some(book) = snap.books.iter().find(|b| b.id == id) else {
        return error(404, "Not found");
    };
    let Some(path) = safe_join(&snap.root, &book.file) else {
        return error(404, "Not found");
    };
    let author = book
        .meta
        .authors
        .first()
        .map(|s| s.as_str())
        .unwrap_or("Unknown");
    let name = sanitize_file_name(&format!("{} - {}.epub", book.meta.title, author));
    serve_file(
        req,
        &path,
        "application/epub+zip",
        Some(content_disposition(&name)),
    )
}

fn route_cover(req: &Request, snap: &LibrarySnapshot, segs: &[&str]) -> Response {
    let (id, thumb) = match segs {
        [id] => (parse_id(id), false),
        [id, "thumbnail"] => (parse_id(id), true),
        _ => (None, false),
    };
    let _ = thumb; // thumbnails are the cover itself: no image decoding in this crate
    let Some(book) = id.and_then(|id| snap.books.iter().find(|b| b.id == id)) else {
        return error(404, "Not found");
    };
    let Some(cover) = &book.cover else {
        return error(404, "Not found");
    };
    let Some(path) = safe_join(&snap.root, cover) else {
        return error(404, "Not found");
    };
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("jpg");
    let mime = mime_for_extension(ext);
    let mime = if mime.starts_with("image/") {
        mime
    } else {
        "image/jpeg"
    };
    serve_file(req, &path, mime, None)
}

fn route_index(cfg: &ServerConfig, snap: &LibrarySnapshot, req: &Request) -> Response {
    let q = req.query_params().remove("q").unwrap_or_default();
    let parsed = Query::parse(&q);
    let mut v: Vec<&Book> = snap.books.iter().filter(|b| parsed.matches(b)).collect();
    v.sort_by_key(|b| b.meta.title.to_lowercase());
    let total = v.len();
    v.truncate(INDEX_LIMIT);
    Response::html(
        200,
        render_index_at(&cfg.public_prefix, &cfg.title, &q, &v, total),
    )
    .with_header("X-Content-Type-Options", "nosniff")
    .with_header(
        "Content-Security-Policy",
        "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; form-action 'self'",
    )
}

fn ensure_header(resp: &mut Response, name: &str, value: &str) {
    if resp.header(name).is_none() {
        resp.headers.push((name.into(), value.into()));
    }
}

impl ServerConfig {
    /// Whether anybody has to log in (or is limited as a guest).
    pub fn secured(&self) -> bool {
        !self.users.is_empty() || !self.accounts.is_empty() || self.guest.is_some()
    }
}

/// What the request may see, or `None` when it must log in. Presented credentials that are wrong
/// are never turned into guest access.
fn access_of(cfg: &ServerConfig, req: &Request) -> Option<Access> {
    if !cfg.secured() {
        return Some(Access::All);
    }
    let header = req.header("authorization");
    if !cfg.users.is_empty() && check_auth(&cfg.users, header) {
        return Some(Access::All);
    }
    if let Some(a) = accounts::login(&cfg.accounts, header) {
        return Some(Access::of_collections(&a.collections));
    }
    match (&cfg.guest, header) {
        (Some(g), None) if req.method == "GET" || req.method == "HEAD" => {
            Some(Access::of_collections(g)).filter(|a| *a != Access::All)
        }
        _ => None,
    }
}

/// `/books/opds` becomes `/opds` for the prefix `/books` (`/books` alone becomes `/`); `None` when
/// the path does not start with the prefix as a whole segment, or there is no prefix.
fn strip_public_prefix(prefix: &str, path: &str) -> Option<String> {
    if prefix.is_empty() {
        return None;
    }
    let rest = path.strip_prefix(prefix)?;
    match rest {
        "" => Some("/".into()),
        r if r.starts_with('/') => Some(r.to_string()),
        _ => None,
    }
}

/// Handles one request. Pure apart from reading the files named by the snapshot.
///
/// Every response carries `X-Content-Type-Options: nosniff`; when users are configured, everything
/// but `/healthcheck` also carries `Cache-Control: no-store` so credentials-protected content is
/// not kept by browsers or proxies.
pub fn handle(cfg: &ServerConfig, req: &Request) -> Response {
    let mut resp = match crate::hostguard::check(req, &cfg.allowed_hosts) {
        Some(refused) => refused,
        None => match strip_public_prefix(&cfg.public_prefix, &req.path) {
            Some(path) => {
                let mut inner = req.clone();
                inner.path = path;
                route(cfg, &inner)
            }
            None => route(cfg, req),
        },
    };
    if cfg.secured() && req.path != "/healthcheck" {
        ensure_header(&mut resp, "Cache-Control", "no-store");
    }
    ensure_header(&mut resp, "X-Content-Type-Options", "nosniff");
    resp
}

/// The one prefix of the sync endpoint: reading progress (KOReader's KOSync protocol) and Bookx's
/// annotation routes share it. This is the address shown to the user. There is no other mount
/// (no `/kosync`, no root-level routes).
pub const SYNC_PREFIX: &str = "/bookx";

/// The path below `/bookx` (empty for `/bookx` itself), or `None` when the path is not under it.
/// The prefix must end at a `/` or at the end of the path.
pub fn strip_sync_prefix(path: &str) -> Option<&str> {
    let rest = path.strip_prefix(SYNC_PREFIX)?;
    (rest.is_empty() || rest.starts_with('/')).then_some(rest)
}

/// `GET /bookx/info`: what this server speaks, so the KOReader plugin can say "server too old" or
/// "plugin too old" instead of failing in the middle of a sync. No login is needed and it carries only
/// protocol numbers and which parts are switched on (nothing about users, books or the version of Bookx).
fn sync_info(cfg: &ServerConfig, req: &Request) -> Response {
    if req.method != "GET" && req.method != "HEAD" {
        return error(405, "Method not allowed").with_header("Allow", "GET, HEAD");
    }
    if cfg.kosync.is_none() {
        return error(404, "Not found");
    }
    Response::json(
        200,
        &json!({
            "name": "bookx",
            "protocol": annotation_sync::PROTOCOL,
            "min_plugin_protocol": annotation_sync::MIN_PLUGIN_PROTOCOL,
            "progress": true,
            "annotations": cfg.annotations.is_some(),
        }),
    )
}

fn route(cfg: &ServerConfig, req: &Request) -> Response {
    if crate::pwa::owns(&req.path) {
        // the page needs the login; the files a browser fetches without credentials do not
        if !crate::pwa::is_public(&req.path) && access_of(cfg, req).is_none() {
            return unauthorized();
        }
        return crate::pwa::serve(req, &format!("{}/", cfg.public_prefix));
    }
    let is_info = strip_sync_prefix(&req.path) == Some("/info");
    let path_only_kosync = is_info
        || strip_sync_prefix(&req.path)
            .is_some_and(|p| KoSync::owns(p) || annotation_sync::owns(p));
    let is_health = req.path == "/healthcheck";
    let access = if is_health || path_only_kosync {
        Access::All
    } else {
        match access_of(cfg, req) {
            Some(a) => a,
            None => return unauthorized(),
        }
    };
    let is_read = req.method == "GET" || req.method == "HEAD";
    if cfg.read_only && !is_read {
        return error(405, "Server is read-only").with_header("Allow", "GET, HEAD");
    }
    if is_health {
        return Response::json(200, &json!({"state": "OK"}));
    }
    if is_info {
        return sync_info(cfg, req);
    }
    if path_only_kosync {
        let rel = strip_sync_prefix(&req.path).unwrap_or_default().to_string();
        if annotation_sync::owns(&rel) {
            return match (&cfg.kosync, &cfg.annotations) {
                (Some(k), Some(a)) => {
                    let mut inner = req.clone();
                    inner.path = rel;
                    a.handle(&inner, &|u, key| k.store().check_user(u, key))
                }
                _ => error(404, "Not found"),
            };
        }
        return match &cfg.kosync {
            Some(k) => {
                let mut inner = req.clone();
                inner.path = rel;
                k.handle(&inner)
            }
            None => error(404, "Not found"),
        };
    }
    if !is_read {
        return error(405, "Method not allowed").with_header("Allow", "GET, HEAD");
    }
    let Some(segs) = req.segments() else {
        return error(400, "Bad request");
    };
    let segs: Vec<&str> = segs.iter().map(|s| s.as_str()).collect();
    let mut snap = (cfg.library)();
    if access != Access::All {
        snap.books.retain(|b| access.allows(b));
    }
    match segs.as_slice() {
        [] => route_index(cfg, &snap, req),
        ["opds", rest @ ..] => {
            let ocfg = OpdsConfig {
                title: cfg.title.clone(),
                page_size: cfg.page_size,
                prefix: format!("{}/opds", cfg.public_prefix),
                base: cfg.public_prefix.clone(),
                ..OpdsConfig::default()
            };
            let rest: Vec<String> = rest.iter().map(|s| s.to_string()).collect();
            match opds::route(&ocfg, &snap.books, &rest, &req.query_params()) {
                Some(doc) => Response::new(200, doc.content_type, doc.body.into_bytes()),
                None => error(404, "Not found"),
            }
        }
        ["download", file] => route_download(req, &snap, file),
        ["cover", rest @ ..] => route_cover(req, &snap, rest),
        _ => error(404, "Not found"),
    }
}

/// Namespace for [`Server::start`].
pub struct Server;

/// A running server. Dropping it shuts it down.
pub struct ServerHandle {
    addr: SocketAddr,
    tls: bool,
    running: net::Running,
}

impl ServerHandle {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// `http://127.0.0.1:PORT` (`https://` when the server uses TLS)
    pub fn base_url(&self) -> String {
        format!(
            "{}://{}",
            if self.tls { "https" } else { "http" },
            self.addr
        )
    }

    /// Connections currently open (idle keep-alive ones included).
    pub fn open_connections(&self) -> usize {
        self.running.open_connections()
    }

    /// Stops accepting, finishes requests being answered, closes the rest and waits for the
    /// connection threads.
    pub fn shutdown(mut self) {
        self.running.stop();
    }
}

impl Server {
    pub fn start(config: ServerConfig) -> Result<ServerHandle> {
        let mut limits = config.limits.clone();
        limits.handler_threads = config.threads.max(1);
        let (bind, port, tls) = (config.bind.clone(), config.port, config.tls.clone());
        let cfg = Arc::new(config);
        let handler: net::Handler = Arc::new(move |req| handle(&cfg, req));
        let running = net::Running::start_tls(&bind, port, limits, handler, tls.clone())?;
        Ok(ServerHandle {
            addr: running.addr(),
            tls: tls.is_some(),
            running,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use bookx_core::opf::Metadata;
    use chrono::Utc;

    fn tmp() -> PathBuf {
        let p = std::env::temp_dir().join(format!("bookx-serve-unit-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn book(id: u64, title: &str, file: &str) -> Book {
        Book {
            id,
            file: file.into(),
            cover: None,
            meta: Metadata {
                title: title.into(),
                authors: vec!["Ann".into()],
                ..Default::default()
            },
            added: Utc::now(),
            modified: Utc::now(),
            last_read: None,
            size: 0,
            progress: 0.0,
            position: None,
            bookmarks: vec![],
            highlights: vec![],
            formats: vec![],
            resources: vec![],
            drm: String::new(),
            custom: Default::default(),
            marked: false,
            source_ids: Vec::new(),
        }
    }

    fn cfg_with(root: &Path, books: Vec<Book>) -> ServerConfig {
        let snap = LibrarySnapshot {
            root: root.to_path_buf(),
            books,
        };
        ServerConfig::new(Arc::new(move || snap.clone()))
    }

    fn basic(u: &str, p: &str) -> String {
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{u}:{p}"))
        )
    }

    #[test]
    fn auth_checks_name_and_password() {
        let users = vec![
            User {
                name: "a".into(),
                password: "1".into(),
            },
            User {
                name: "b".into(),
                password: "2".into(),
            },
        ];
        assert!(check_auth(&users, Some(&basic("a", "1"))));
        assert!(check_auth(&users, Some(&basic("b", "2"))));
        assert!(!check_auth(&users, Some(&basic("a", "2"))));
        assert!(!check_auth(&users, Some(&basic("a", ""))));
        assert!(!check_auth(&users, Some(&basic("c", "1"))));
        assert!(!check_auth(&users, Some("Bearer x")));
        assert!(!check_auth(&users, None));
        assert!(!check_auth(&[], Some(&basic("a", "1"))));
    }

    #[test]
    fn content_disposition_has_ascii_and_utf8_forms() {
        let d = content_disposition("Les Misérables \"1\".epub");
        assert!(d.starts_with("attachment; filename=\"Les Mis_rables _1_.epub\""));
        assert!(d.ends_with("filename*=UTF-8''Les%20Mis%C3%A9rables%20%221%22.epub"));
        assert!(d.is_ascii());
    }

    #[test]
    fn index_page_escapes_everything() {
        let root = tmp();
        let mut b = book(1, "<script>alert(1)</script> & \"q\"", "x.epub");
        b.meta.authors = vec!["<b>Evil</b>".into()];
        b.cover = Some("c.jpg".into());
        let cfg = cfg_with(&root, vec![b]);
        let r = handle(&cfg, &Request::new("GET", "/?q=script"));
        let body = r.body_str();
        assert_eq!(r.status, 200);
        assert!(!body.contains("<script"), "{body}");
        assert!(!body.contains("<b>Evil"));
        assert!(body.contains("&lt;script&gt;alert(1)&lt;/script&gt; &amp; &quot;q&quot;"));
        assert!(body.contains("&lt;b&gt;Evil&lt;/b&gt;"));
        assert!(r.header("content-security-policy").is_some());
        // the echoed search term is escaped too
        let r = handle(&cfg, &Request::new("GET", "/?q=%22%3E%3Cscript%3Ex"));
        let body = r.body_str();
        assert!(
            body.contains("value=\"&quot;&gt;&lt;script&gt;x\""),
            "{body}"
        );
        assert!(!body.contains("<script"));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn index_lists_and_searches() {
        let root = tmp();
        let cfg = cfg_with(
            &root,
            vec![book(1, "Dune", "a.epub"), book(2, "Emma", "b.epub")],
        );
        let all = handle(&cfg, &Request::new("GET", "/")).body_str();
        assert!(all.contains("Dune") && all.contains("Emma") && all.contains("2 books."));
        let one = handle(&cfg, &Request::new("GET", "/?q=emma")).body_str();
        assert!(one.contains("Emma") && !one.contains("Dune") && one.contains("1 book."));
        assert!(one.contains("value=\"emma\""));
        assert!(all.contains("href=\"/download/1.epub\""));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn index_is_capped() {
        let root = tmp();
        let books: Vec<Book> = (1..=300).map(|i| book(i, &format!("B{i}"), "x")).collect();
        let cfg = cfg_with(&root, books);
        let body = handle(&cfg, &Request::new("GET", "/")).body_str();
        assert!(body.contains("Showing 200 of 300 books."));
        assert_eq!(body.matches("<li>").count(), 200);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn download_and_range_logic() {
        let root = tmp();
        std::fs::write(root.join("a.epub"), b"0123456789").unwrap();
        let cfg = cfg_with(&root, vec![book(1, "Dune", "a.epub")]);
        let r = handle(&cfg, &Request::new("GET", "/download/1.epub"));
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"0123456789");
        assert_eq!(r.header("content-type"), Some("application/epub+zip"));
        assert_eq!(r.header("accept-ranges"), Some("bytes"));
        assert!(
            r.header("content-disposition")
                .unwrap()
                .contains("Dune - Ann.epub")
        );
        let r = handle(
            &cfg,
            &Request::new("GET", "/download/1.epub").with_header("Range", "bytes=2-4"),
        );
        assert_eq!((r.status, r.body.as_slice()), (206, &b"234"[..]));
        assert_eq!(r.header("content-range"), Some("bytes 2-4/10"));
        let r = handle(
            &cfg,
            &Request::new("GET", "/download/1.epub").with_header("Range", "bytes=-3"),
        );
        assert_eq!((r.status, r.body.as_slice()), (206, &b"789"[..]));
        let r = handle(
            &cfg,
            &Request::new("GET", "/download/1.epub").with_header("Range", "bytes=50-"),
        );
        assert_eq!(r.status, 416);
        assert_eq!(r.header("content-range"), Some("bytes */10"));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn download_rejects_bad_ids_and_escaping_paths() {
        let root = tmp();
        let outside = tmp();
        std::fs::write(outside.join("secret.epub"), b"secret").unwrap();
        std::fs::write(root.join("ok.epub"), b"ok").unwrap();
        let evil = book(
            2,
            "Evil",
            &format!(
                "../{}/secret.epub",
                outside.file_name().unwrap().to_string_lossy()
            ),
        );
        let cfg = cfg_with(&root, vec![book(1, "Ok", "ok.epub"), evil]);
        for p in [
            "/download/2.epub",
            "/download/3.epub",
            "/download/x.epub",
            "/download/1",
            "/download/1.pdf",
            "/download/-1.epub",
            "/download/99999999999999999999999.epub",
            "/download/.epub",
        ] {
            assert_eq!(handle(&cfg, &Request::new("GET", p)).status, 404, "{p}");
        }
        assert_eq!(
            handle(&cfg, &Request::new("GET", "/download/%2e%2e/x")).status,
            400
        );
        assert_eq!(
            handle(&cfg, &Request::new("GET", "/download/1.epub")).status,
            200
        );
        std::fs::remove_dir_all(root).ok();
        std::fs::remove_dir_all(outside).ok();
    }

    #[test]
    fn covers() {
        let root = tmp();
        std::fs::write(root.join("c.png"), b"PNGDATA").unwrap();
        let mut b = book(1, "A", "a.epub");
        b.cover = Some("c.png".into());
        let nocover = book(2, "B", "b.epub");
        let cfg = cfg_with(&root, vec![b, nocover]);
        let r = handle(&cfg, &Request::new("GET", "/cover/1"));
        assert_eq!((r.status, r.body.as_slice()), (200, &b"PNGDATA"[..]));
        assert_eq!(r.header("content-type"), Some("image/png"));
        assert_eq!(
            handle(&cfg, &Request::new("GET", "/cover/1/thumbnail")).status,
            200
        );
        assert_eq!(handle(&cfg, &Request::new("GET", "/cover/2")).status, 404);
        assert_eq!(handle(&cfg, &Request::new("GET", "/cover/9")).status, 404);
        assert_eq!(
            handle(&cfg, &Request::new("GET", "/cover/1/other")).status,
            404
        );
        assert_eq!(handle(&cfg, &Request::new("GET", "/cover")).status, 404);
        assert_eq!(
            handle(&cfg, &Request::new("GET", "/cover/1/thumbnail/x")).status,
            404
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn the_web_client_is_served_and_only_its_page_needs_the_login() {
        let root = tmp();
        let mut cfg = cfg_with(&root, vec![book(1, "Dune", "a.epub")]);
        cfg.users = vec![User {
            name: "u".into(),
            password: "p".into(),
        }];
        assert_eq!(handle(&cfg, &Request::new("GET", "/app/")).status, 401);
        let ok = Request::new("GET", "/app/").with_header("Authorization", &basic("u", "p"));
        let r = handle(&cfg, &ok);
        assert_eq!(r.status, 200);
        assert_eq!(r.header("content-security-policy"), Some(crate::pwa::CSP));
        for p in [
            "/app/manifest.webmanifest",
            "/app/sw.js",
            "/app/app.js",
            "/app/app.css",
            "/app/icon.svg",
        ] {
            assert_eq!(handle(&cfg, &Request::new("GET", p)).status, 200, "{p}");
        }
        // the library behind it still does
        assert_eq!(handle(&cfg, &Request::new("GET", "/opds/all")).status, 401);
        // a foreign Host never gets the client (DNS rebinding)
        let bad = Request::new("GET", "/app/app.js").with_header("Host", "evil.example");
        assert_eq!(handle(&cfg, &bad).status, 403);
        // under a public prefix the worker may control that prefix only
        cfg.public_prefix = "/books".into();
        let sw = handle(&cfg, &Request::new("GET", "/books/app/sw.js"));
        assert_eq!(sw.status, 200);
        assert_eq!(sw.header("service-worker-allowed"), Some("/books/"));
        std::fs::remove_dir_all(root).ok();
    }

    fn tagged(id: u64, title: &str, file: &str, tags: &[&str]) -> Book {
        let mut b = book(id, title, file);
        b.meta.tags = tags.iter().map(|t| t.to_string()).collect();
        b
    }

    fn library_with_collections(root: &Path) -> ServerConfig {
        std::fs::write(root.join("kids.epub"), b"k").unwrap();
        std::fs::write(root.join("adult.epub"), b"a").unwrap();
        cfg_with(
            root,
            vec![
                tagged(1, "Pony Tales", "kids.epub", &["Kids"]),
                tagged(2, "Grown Ups", "adult.epub", &["Adult"]),
            ],
        )
    }

    fn get_as(cfg: &ServerConfig, path: &str, who: Option<(&str, &str)>) -> Response {
        let mut r = Request::new("GET", path);
        if let Some((u, p)) = who {
            r = r.with_header("Authorization", &basic(u, p));
        }
        handle(cfg, &r)
    }

    #[test]
    fn accounts_see_only_their_collections() {
        let root = tmp();
        let mut cfg = library_with_collections(&root);
        cfg.accounts = vec![
            Account::new("kid", "kidpw", &["kids".into()]),
            Account::new("mum", "mumpw", &[]),
        ];
        // no login, wrong login: refused
        assert_eq!(get_as(&cfg, "/opds/all", None).status, 401);
        assert_eq!(
            get_as(&cfg, "/opds/all", Some(("kid", "mumpw"))).status,
            401
        );
        assert_eq!(get_as(&cfg, "/opds/all", Some(("ghost", "x"))).status, 401);
        // the child sees one book in every place a book can show up
        let kid = Some(("kid", "kidpw"));
        let all = get_as(&cfg, "/opds/all", kid).body_str();
        assert!(
            all.contains("Pony Tales") && !all.contains("Grown Ups"),
            "{all}"
        );
        let found = get_as(&cfg, "/opds/search?q=Grown", kid).body_str();
        assert!(!found.contains("Grown Ups"), "{found}");
        let page = get_as(&cfg, "/?q=Grown", kid).body_str();
        assert!(!page.contains("Grown Ups"), "{page}");
        assert_eq!(get_as(&cfg, "/download/1.epub", kid).status, 200);
        assert_eq!(get_as(&cfg, "/download/2.epub", kid).status, 404);
        assert_eq!(get_as(&cfg, "/cover/2", kid).status, 404);
        // an account without collections sees everything
        let mum = Some(("mum", "mumpw"));
        let all = get_as(&cfg, "/opds/all", mum).body_str();
        assert!(all.contains("Pony Tales") && all.contains("Grown Ups"));
        assert_eq!(get_as(&cfg, "/download/2.epub", mum).status, 200);
        // responses to logged-in users are never cached
        assert_eq!(
            get_as(&cfg, "/opds/all", kid).header("cache-control"),
            Some("no-store")
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn the_shared_user_keeps_full_access_next_to_accounts() {
        let root = tmp();
        let mut cfg = library_with_collections(&root);
        cfg.users = vec![User {
            name: "owner".into(),
            password: "pw".into(),
        }];
        cfg.accounts = vec![Account::new("kid", "kidpw", &["kids".into()])];
        assert!(
            get_as(&cfg, "/opds/all", Some(("owner", "pw")))
                .body_str()
                .contains("Grown Ups")
        );
        assert!(
            !get_as(&cfg, "/opds/all", Some(("kid", "kidpw")))
                .body_str()
                .contains("Grown Ups")
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_guest_reads_some_collections_and_nothing_else() {
        let root = tmp();
        let mut cfg = library_with_collections(&root);
        cfg.accounts = vec![Account::new("mum", "mumpw", &[])];
        cfg.guest = Some(vec!["Kids".into()]);
        let all = get_as(&cfg, "/opds/all", None).body_str();
        assert!(
            all.contains("Pony Tales") && !all.contains("Grown Ups"),
            "{all}"
        );
        assert_eq!(get_as(&cfg, "/download/2.epub", None).status, 404);
        assert_eq!(get_as(&cfg, "/download/1.epub", None).status, 200);
        // read-only: any other method needs a login
        assert_eq!(handle(&cfg, &Request::new("POST", "/opds/all")).status, 401);
        // a wrong password is a refusal, not a quiet downgrade to guest (the login limit counts it)
        assert_eq!(
            get_as(&cfg, "/opds/all", Some(("mum", "wrong"))).status,
            401
        );
        // the real login still sees everything
        assert!(
            get_as(&cfg, "/opds/all", Some(("mum", "mumpw")))
                .body_str()
                .contains("Grown Ups")
        );
        // an empty guest list is no guest
        cfg.guest = Some(vec![]);
        assert_eq!(get_as(&cfg, "/opds/all", None).status, 401);
        // the sync routes keep their own accounts and are not opened by the guest
        cfg.guest = Some(vec!["kids".into()]);
        assert_eq!(get_as(&cfg, "/healthcheck", None).status, 200);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn public_prefix_cleaning() {
        assert_eq!(clean_public_prefix(""), Some(String::new()));
        assert_eq!(clean_public_prefix(" / "), Some(String::new()));
        assert_eq!(clean_public_prefix("books/"), Some("/books".into()));
        assert_eq!(clean_public_prefix("/a/b-c"), Some("/a/b-c".into()));
        for bad in [
            "..", "/a/../b", "a b", "a?b", "http://x", "/bookx", "opds", "a//b", "<x>", "/a#b",
        ] {
            assert_eq!(clean_public_prefix(bad), None, "{bad}");
        }
    }

    #[test]
    fn public_prefix_works_with_and_without_the_proxy_stripping_it() {
        let root = tmp();
        let mut cfg = cfg_with(&root, vec![book(1, "Dune", "a.epub")]);
        cfg.public_prefix = "/books".into();
        for path in ["/books/opds", "/opds"] {
            let r = handle(&cfg, &Request::new("GET", path));
            assert_eq!(r.status, 200, "{path}");
            let b = r.body_str();
            assert!(b.contains("href=\"/books/opds/new\""), "{b}");
            assert!(!b.contains("href=\"/opds"), "{b}");
        }
        let r = handle(&cfg, &Request::new("GET", "/books/opds/search?q=dune"));
        let b = r.body_str();
        assert!(b.contains("href=\"/books/download/1.epub\""), "{b}");
        let r = handle(&cfg, &Request::new("GET", "/books"));
        let b = r.body_str();
        assert!(b.contains("action=\"/books/\""), "{b}");
        assert!(b.contains("href=\"/books/opds\""), "{b}");
        assert!(b.contains("href=\"/books/download/1.epub\""), "{b}");
        // the prefix is a whole segment: `/bookshelf` is not `/books` + `helf`
        assert_eq!(
            handle(&cfg, &Request::new("GET", "/bookshelf/opds")).status,
            404
        );
        // the files are reachable under the prefix too
        std::fs::write(root.join("a.epub"), b"x").unwrap();
        assert_eq!(
            handle(&cfg, &Request::new("GET", "/books/download/1.epub")).status,
            200
        );
        // a prefix never lets a request out of the library
        assert_eq!(
            handle(&cfg, &Request::new("GET", "/books/../x")).status,
            400
        );
        // without a prefix nothing changes
        cfg.public_prefix.clear();
        assert_eq!(
            handle(&cfg, &Request::new("GET", "/books/opds")).status,
            404
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn public_prefix_does_not_open_the_login() {
        let root = tmp();
        let mut cfg = cfg_with(&root, vec![book(1, "Dune", "a.epub")]);
        cfg.public_prefix = "/books".into();
        cfg.users = vec![User {
            name: "u".into(),
            password: "p".into(),
        }];
        assert_eq!(
            handle(&cfg, &Request::new("GET", "/books/opds")).status,
            401
        );
        assert_eq!(
            handle(&cfg, &Request::new("GET", "/books/download/1.epub")).status,
            401
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn opds_routes_are_wired() {
        let root = tmp();
        let cfg = cfg_with(&root, vec![book(1, "Dune", "a.epub")]);
        let r = handle(&cfg, &Request::new("GET", "/opds"));
        assert_eq!(r.status, 200);
        assert!(
            r.header("content-type")
                .unwrap()
                .contains("kind=navigation")
        );
        assert_eq!(handle(&cfg, &Request::new("GET", "/opds/")).status, 200);
        let r = handle(&cfg, &Request::new("GET", "/opds/search?q=dune"));
        assert!(r.body_str().contains("<title>Dune</title>"));
        assert_eq!(
            handle(&cfg, &Request::new("GET", "/opds/opensearch.xml")).status,
            200
        );
        assert_eq!(handle(&cfg, &Request::new("GET", "/opds/nope")).status, 404);
        assert_eq!(
            handle(&cfg, &Request::new("GET", "/opds/new?page=5")).status,
            404
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn auth_gate() {
        let root = tmp();
        let mut cfg = cfg_with(&root, vec![]);
        cfg.users = vec![User {
            name: "u".into(),
            password: "p".into(),
        }];
        let r = handle(&cfg, &Request::new("GET", "/opds"));
        assert_eq!(r.status, 401);
        assert!(r.header("www-authenticate").unwrap().starts_with("Basic"));
        assert_eq!(handle(&cfg, &Request::new("GET", "/")).status, 401);
        assert_eq!(
            handle(&cfg, &Request::new("GET", "/download/1.epub")).status,
            401
        );
        let ok = Request::new("GET", "/opds").with_header("Authorization", &basic("u", "p"));
        assert_eq!(handle(&cfg, &ok).status, 200);
        let bad = Request::new("GET", "/opds").with_header("Authorization", &basic("u", "x"));
        assert_eq!(handle(&cfg, &bad).status, 401);
        // health and kosync carry their own auth
        assert_eq!(
            handle(&cfg, &Request::new("GET", "/healthcheck")).status,
            200
        );
        std::fs::remove_dir_all(root).ok();
    }

    fn sync_cfg(root: &std::path::Path) -> ServerConfig {
        let mut cfg = cfg_with(root, vec![]);
        cfg.kosync = Some(Arc::new(KoSync::new(
            Box::new(crate::kosync::MemoryStore::new()),
            true,
        )));
        cfg.annotations = Some(Arc::new(annotation_sync::AnnotationSync::memory()));
        cfg
    }

    fn acct(p: &str) -> Request {
        Request::new("GET", p)
            .with_header("x-auth-user", "k")
            .with_header("x-auth-key", "abc")
    }

    #[test]
    fn info_tells_the_plugin_what_the_server_speaks() {
        let root = tmp();
        let mut cfg = sync_cfg(&root);
        cfg.users = vec![User {
            name: "u".into(),
            password: "p".into(),
        }];
        // no login at all: the plugin asks before it has an account
        let r = handle(&cfg, &Request::new("GET", "/bookx/info"));
        assert_eq!(r.status, 200);
        let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(v["name"], "bookx");
        assert_eq!(v["protocol"], annotation_sync::PROTOCOL);
        assert_eq!(
            v["min_plugin_protocol"],
            annotation_sync::MIN_PLUGIN_PROTOCOL
        );
        assert_eq!(v["annotations"], true);
        // nothing but protocol numbers and switches
        assert_eq!(v.as_object().unwrap().len(), 5);
        assert_eq!(
            handle(&cfg, &Request::new("POST", "/bookx/info")).status,
            405
        );
        // not a second endpoint
        assert_ne!(handle(&cfg, &Request::new("GET", "/info")).status, 200);
        assert_ne!(
            handle(&cfg, &Request::new("GET", "/kosync/info")).status,
            200
        );
        // annotation sync switched off: says so; sync switched off: not found
        cfg.annotations = None;
        let r = handle(&cfg, &Request::new("GET", "/bookx/info"));
        let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(v["annotations"], false);
        cfg.kosync = None;
        assert_eq!(
            handle(&cfg, &Request::new("GET", "/bookx/info")).status,
            404
        );
    }

    #[test]
    fn a_rebinding_host_and_a_cross_origin_post_never_reach_a_route() {
        let dir = tmp();
        let mut cfg = sync_cfg(&dir);
        cfg.allowed_hosts = vec!["books.example.com".into()];
        let ok = |r: Request| handle(&cfg, &r).status;
        // normal clients
        assert_eq!(
            ok(Request::new("GET", "/healthcheck").with_header("Host", "127.0.0.1:8080")),
            200
        );
        assert_eq!(
            ok(Request::new("GET", "/healthcheck").with_header("Host", "books.example.com")),
            200
        );
        assert_eq!(ok(Request::new("GET", "/healthcheck")), 200);
        // DNS rebinding: the page's own name arrives in Host
        assert_eq!(
            ok(Request::new("GET", "/healthcheck").with_header("Host", "evil.example:8080")),
            403
        );
        assert_eq!(
            ok(Request::new("GET", "/opds").with_header("Host", "attacker.test")),
            403
        );
        // CSRF: registering an account from a web page
        let create = Request::new("POST", "/bookx/users/create")
            .with_header("Host", "127.0.0.1:8080")
            .with_body(br#"{"username":"mallory","password":"x"}"#);
        assert_ne!(ok(create.clone()), 403, "native clients still register");
        let csrf = create.clone().with_header("Origin", "https://evil.example");
        let resp = handle(&cfg, &csrf);
        assert_eq!(resp.status, 403);
        assert!(resp.header("x-content-type-options").is_some());
        assert_eq!(ok(create.with_header("Origin", "null")), 403);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sync_is_mounted_under_bookx_only() {
        let root = tmp();
        let mut cfg = sync_cfg(&root);
        cfg.users = vec![User {
            name: "u".into(),
            password: "p".into(),
        }];
        let create = Request::new("POST", "/bookx/users/create")
            .with_body(br#"{"username":"k","password":"abc"}"#);
        assert_eq!(handle(&cfg, &create).status, 201);
        assert_eq!(handle(&cfg, &acct("/bookx/users/auth")).status, 200);
        // the account decides, not the Basic credentials of the library
        assert_eq!(
            handle(&cfg, &Request::new("GET", "/bookx/users/auth")).status,
            401
        );
        assert_eq!(
            handle(&cfg, &Request::new("GET", "/bookx/healthcheck")).status,
            200
        );
        // progress round trip
        let doc = "a".repeat(32);
        let put = Request::new("PUT", "/bookx/syncs/progress")
            .with_header("x-auth-user", "k")
            .with_header("x-auth-key", "abc")
            .with_body(
                format!(
                    r#"{{"document":"{doc}","progress":"1","percentage":0.5,"device":"d","device_id":"i"}}"#
                )
                .as_bytes(),
            );
        assert_eq!(handle(&cfg, &put).status, 200);
        assert_eq!(
            handle(&cfg, &acct(&format!("/bookx/syncs/progress/{doc}"))).status,
            200
        );
        // annotations need the account too
        let path = format!("/bookx/annotations/{doc}");
        let mut anon = Request::new("GET", &path);
        anon.query = "since=0".into();
        assert_eq!(handle(&cfg, &anon).status, 401);
        let mut ok = acct(&path);
        ok.query = "since=0".into();
        assert_eq!(handle(&cfg, &ok).status, 200);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn legacy_kosync_and_root_routes_are_gone() {
        let root = tmp();
        let mut cfg = sync_cfg(&root);
        cfg.users = vec![User {
            name: "u".into(),
            password: "p".into(),
        }];
        let doc = "a".repeat(32);
        for p in [
            "/kosync/users/auth".to_string(),
            "/kosync/healthcheck".to_string(),
            "/kosync/syncs/progress/x".to_string(),
            format!("/kosync/annotations/{doc}"),
            "/users/auth".to_string(),
            format!("/annotations/{doc}"),
            format!("/syncs/progress/{doc}"),
        ] {
            // with the account headers: not served, and not a way around the library's Basic auth
            assert_eq!(handle(&cfg, &acct(&p)).status, 401, "{p}");
        }
        cfg.users.clear();
        for p in [
            "/kosync/users/auth",
            "/kosync/healthcheck",
            "/users/auth",
            "/syncs/progress/x",
        ] {
            assert_eq!(handle(&cfg, &acct(p)).status, 404, "{p}");
        }
        let create = Request::new("POST", "/kosync/users/create")
            .with_body(br#"{"username":"z","password":"abc"}"#);
        assert_ne!(handle(&cfg, &create).status, 201);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn bookx_prefix_is_404_when_sync_is_off() {
        let root = tmp();
        let cfg = cfg_with(&root, vec![]);
        for p in [
            "/bookx/users/auth",
            "/bookx/healthcheck",
            "/bookx/annotations/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ] {
            assert_eq!(handle(&cfg, &Request::new("GET", p)).status, 404, "{p}");
        }
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn sync_prefix_stripping() {
        assert_eq!(strip_sync_prefix("/bookx/users/auth"), Some("/users/auth"));
        assert_eq!(strip_sync_prefix("/bookx"), Some(""));
        assert_eq!(strip_sync_prefix("/kosync/users/auth"), None);
        assert_eq!(strip_sync_prefix("/users/auth"), None);
        assert_eq!(strip_sync_prefix("/bookxish/x"), None);
    }

    #[test]
    fn kosync_missing_is_404() {
        let root = tmp();
        let cfg = cfg_with(&root, vec![]);
        assert_eq!(
            handle(&cfg, &Request::new("GET", "/bookx/users/auth")).status,
            404
        );
        assert_eq!(
            handle(&cfg, &Request::new("GET", "/healthcheck")).status,
            200
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn read_only_mode() {
        let root = tmp();
        let mut cfg = cfg_with(&root, vec![]);
        cfg.read_only = true;
        cfg.kosync = Some(Arc::new(KoSync::new(
            Box::new(crate::kosync::MemoryStore::new()),
            true,
        )));
        assert_eq!(handle(&cfg, &Request::new("GET", "/opds")).status, 200);
        assert_eq!(handle(&cfg, &Request::new("HEAD", "/opds")).status, 200);
        for m in ["POST", "PUT", "DELETE", "PATCH"] {
            let r = handle(&cfg, &Request::new(m, "/opds"));
            assert_eq!(r.status, 405, "{m}");
            assert_eq!(r.header("allow"), Some("GET, HEAD"));
        }
        let create = Request::new("POST", "/bookx/users/create")
            .with_body(br#"{"username":"k","password":"abc"}"#);
        assert_eq!(handle(&cfg, &create).status, 405);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn writes_are_rejected_on_library_routes_even_when_not_read_only() {
        let root = tmp();
        let cfg = cfg_with(&root, vec![]);
        assert_eq!(handle(&cfg, &Request::new("POST", "/opds")).status, 405);
        assert_eq!(
            handle(&cfg, &Request::new("DELETE", "/download/1.epub")).status,
            405
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn traversal_in_paths_is_400() {
        let root = tmp();
        let cfg = cfg_with(&root, vec![]);
        for p in [
            "/../etc/passwd",
            "/opds/%2e%2e/x",
            "/download/..%2f..%2fetc%2fpasswd",
            "/cover/%2e%2e",
        ] {
            assert_eq!(handle(&cfg, &Request::new("GET", p)).status, 400, "{p}");
        }
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn unknown_paths_are_404() {
        let root = tmp();
        let cfg = cfg_with(&root, vec![]);
        for p in [
            "/nope",
            "/download",
            "/download/1.epub/extra",
            "/etc/passwd",
        ] {
            assert_eq!(handle(&cfg, &Request::new("GET", p)).status, 404, "{p}");
        }
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn parse_id_edge_cases() {
        assert_eq!(parse_id("12"), Some(12));
        assert_eq!(parse_id("0"), Some(0));
        assert_eq!(parse_id(""), None);
        assert_eq!(parse_id("-1"), None);
        assert_eq!(parse_id("+1"), None);
        assert_eq!(parse_id("1a"), None);
        assert_eq!(parse_id("99999999999999999999"), None);
    }

    #[test]
    fn safe_join_rules() {
        let root = tmp();
        std::fs::create_dir_all(root.join("d")).unwrap();
        std::fs::write(root.join("d/f.epub"), b"x").unwrap();
        assert!(safe_join(&root, "d/f.epub").is_some());
        assert!(safe_join(&root, "d").is_none(), "directories are not files");
        assert!(safe_join(&root, "missing").is_none());
        assert!(safe_join(&root, "../").is_none());
        assert!(safe_join(&root.join("nonexistent"), "x").is_none());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn authenticated_responses_are_not_cacheable_and_all_are_nosniff() {
        let mut cfg = cfg_with(&tmp(), vec![]);
        cfg.users = vec![User {
            name: "a".into(),
            password: "1".into(),
        }];
        let auth = basic("a", "1");
        for path in ["/", "/opds", "/nothing", "/download/1.epub", "/cover/1"] {
            let ok = handle(
                &cfg,
                &Request::new("GET", path).with_header("authorization", &auth),
            );
            assert_eq!(ok.header("cache-control"), Some("no-store"), "{path}");
            assert_eq!(
                ok.header("x-content-type-options"),
                Some("nosniff"),
                "{path}"
            );
            let denied = handle(&cfg, &Request::new("GET", path));
            assert_eq!(denied.status, 401);
            assert_eq!(denied.header("cache-control"), Some("no-store"), "{path}");
        }
        let health = handle(&cfg, &Request::new("GET", "/healthcheck"));
        assert_eq!(health.status, 200);
        assert_eq!(health.header("cache-control"), None);
        assert_eq!(health.header("x-content-type-options"), Some("nosniff"));

        // no users: nothing to protect, but nosniff stays; no duplicates of an existing header
        let open = cfg_with(&tmp(), vec![]);
        let r = handle(&open, &Request::new("GET", "/"));
        assert_eq!(r.header("cache-control"), None);
        let nosniff = r
            .headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("x-content-type-options"))
            .count();
        assert_eq!(nosniff, 1);
    }

    #[test]
    fn server_config_defaults_to_the_documented_limits() {
        let cfg = cfg_with(&tmp(), vec![]);
        assert_eq!(cfg.limits, Limits::default());
        assert_eq!(cfg.limits.max_connections, 64);
        assert_eq!(cfg.threads, 8);
    }
}
