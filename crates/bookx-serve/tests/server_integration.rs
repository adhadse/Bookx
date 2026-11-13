//! End-to-end tests: a real server on port 0, talked to with a minimal std `TcpStream` client.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use bookx_core::library::Book;
use bookx_core::opf::Metadata;
use bookx_serve::kosync::{md5_hex, JsonFileStore, KoSync, MemoryStore};
use bookx_serve::opds_client::{expand_search_template, parse_feed, SearchLink};
use bookx_serve::server::{LibrarySnapshot, Server, ServerConfig, ServerHandle, User};
use chrono::Utc;

// ---------------------------------------------------------------- minimal HTTP client

#[derive(Debug)]
struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Resp {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

fn raw_request(addr: SocketAddr, raw: &[u8]) -> std::io::Result<Vec<u8>> {
    raw_request_timeout(addr, raw, Duration::from_secs(10))
}

fn raw_request_timeout(addr: SocketAddr, raw: &[u8], read: Duration) -> std::io::Result<Vec<u8>> {
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
    s.set_read_timeout(Some(read))?;
    s.set_write_timeout(Some(Duration::from_secs(10)))?;
    s.write_all(raw)?;
    let mut out = Vec::new();
    match s.read_to_end(&mut out) {
        Ok(_) => Ok(out),
        // a reset after a complete response still leaves us the data
        Err(e) if !out.is_empty() => {
            let _ = e;
            Ok(out)
        }
        Err(e) => Err(e),
    }
}

fn parse_response(bytes: &[u8]) -> Resp {
    let split = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or_else(|| {
            panic!(
                "no header terminator in {:?}",
                String::from_utf8_lossy(bytes)
            )
        });
    let head = std::str::from_utf8(&bytes[..split]).expect("ascii head");
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap();
    let mut parts = status_line.splitn(3, ' ');
    assert!(
        parts.next().unwrap().starts_with("HTTP/1."),
        "{status_line}"
    );
    let status: u16 = parts.next().unwrap().parse().unwrap();
    let headers = lines
        .map(|l| {
            let (k, v) = l
                .split_once(':')
                .unwrap_or_else(|| panic!("bad header {l:?}"));
            (k.trim().to_string(), v.trim().to_string())
        })
        .collect();
    Resp {
        status,
        headers,
        body: bytes[split + 4..].to_vec(),
    }
}

fn request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Resp {
    let mut raw = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    for (k, v) in headers {
        raw.push_str(&format!("{k}: {v}\r\n"));
    }
    if !body.is_empty() {
        raw.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    raw.push_str("\r\n");
    let mut bytes = raw.into_bytes();
    bytes.extend_from_slice(body);
    let out = raw_request(addr, &bytes).unwrap_or_else(|e| panic!("{method} {path}: {e}"));
    parse_response(&out)
}

fn get(addr: SocketAddr, path: &str) -> Resp {
    request(addr, "GET", path, &[], b"")
}

fn basic(u: &str, p: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{u}:{p}"))
    )
}

// ---------------------------------------------------------------- fixtures

struct Fixture {
    dir: PathBuf,
    files: Vec<Vec<u8>>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

fn make_book(id: u64, title: &str, author: &str, file: &str) -> Book {
    Book {
        id,
        file: file.into(),
        cover: None,
        meta: Metadata {
            title: title.into(),
            authors: vec![author.into()],
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
    }
}

fn fixture() -> (Fixture, Vec<Book>) {
    let dir = std::env::temp_dir().join(format!("bookx-serve-it-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut files = Vec::new();
    let mut books = Vec::new();
    for (i, (title, author)) in [
        ("Dune", "Frank Herbert"),
        ("Emma", "Jane Austen"),
        ("Tom & Jerry <1>", "Ann"),
    ]
    .iter()
    .enumerate()
    {
        let id = i as u64 + 1;
        let data: Vec<u8> = (0..(1000 * id as usize + 17))
            .map(|n| ((n * 31 + i) % 256) as u8)
            .collect();
        std::fs::write(dir.join(format!("{id}.epub")), &data).unwrap();
        files.push(data.clone());
        let mut b = make_book(id, title, author, &format!("{id}.epub"));
        b.size = data.len() as u64;
        if id == 1 {
            std::fs::write(dir.join("1.jpg"), b"JPEGDATA").unwrap();
            b.cover = Some("1.jpg".into());
        }
        books.push(b);
    }
    (Fixture { dir, files }, books)
}

fn start(books: Vec<Book>, root: &Path, tweak: impl FnOnce(&mut ServerConfig)) -> ServerHandle {
    let snap = LibrarySnapshot {
        root: root.to_path_buf(),
        books,
    };
    let mut cfg = ServerConfig::new(Arc::new(move || snap.clone()));
    cfg.threads = 4;
    tweak(&mut cfg);
    Server::start(cfg).expect("server starts")
}

/// The acceptor thread exits asynchronously; poll until connections are refused.
fn wait_closed(addr: SocketAddr) -> bool {
    for _ in 0..60 {
        match TcpStream::connect_timeout(&addr, Duration::from_millis(200)) {
            Err(_) => return true,
            Ok(s) => {
                drop(s);
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    false
}

fn kosync_mem() -> Arc<KoSync> {
    Arc::new(KoSync::new(Box::new(MemoryStore::new()), true))
}

// ---------------------------------------------------------------- tests

#[test]
fn serves_pages_downloads_and_errors() {
    let (fx, books) = fixture();
    let srv = start(books, &fx.dir, |_| {});
    let addr = srv.addr();
    assert_ne!(addr.port(), 0);
    assert!(srv.base_url().starts_with("http://127.0.0.1:"));

    let r = get(addr, "/");
    assert_eq!(r.status, 200);
    assert!(r.header("content-type").unwrap().starts_with("text/html"));
    let page = r.text();
    assert!(page.contains("Dune") && page.contains("Emma"));
    assert!(
        page.contains("Tom &amp; Jerry &lt;1&gt;"),
        "titles are escaped: {page}"
    );
    assert!(!page.contains("Tom & Jerry <1>"));

    let r = get(addr, "/?q=emma");
    assert!(r.text().contains("Emma") && !r.text().contains("Dune"));

    let r = get(addr, "/download/2.epub");
    assert_eq!(r.status, 200);
    assert_eq!(r.body, fx.files[1]);
    assert_eq!(r.header("content-type"), Some("application/epub+zip"));
    assert_eq!(
        r.header("content-length"),
        Some(fx.files[1].len().to_string().as_str())
    );
    assert!(r
        .header("content-disposition")
        .unwrap()
        .contains("Emma - Jane Austen.epub"));

    let r = get(addr, "/cover/1");
    assert_eq!((r.status, r.body.as_slice()), (200, &b"JPEGDATA"[..]));
    assert_eq!(r.header("content-type"), Some("image/jpeg"));
    assert_eq!(get(addr, "/cover/2").status, 404);

    assert_eq!(get(addr, "/download/99.epub").status, 404);
    assert_eq!(get(addr, "/nothing/here").status, 404);
    let r = get(addr, "/opds/new?page=9");
    assert_eq!(r.status, 404);
    srv.shutdown();
}

#[test]
fn opds_over_http_is_valid_xml_and_followable() {
    let (fx, books) = fixture();
    let srv = start(books, &fx.dir, |c| c.page_size = 2);
    let addr = srv.addr();
    let base = format!("http://{addr}");

    let r = get(addr, "/opds");
    assert_eq!(r.status, 200);
    assert!(r
        .header("content-type")
        .unwrap()
        .contains("profile=opds-catalog"));
    roxmltree::Document::parse(&r.text()).expect("valid XML");
    let root = parse_feed(&r.text(), &format!("{base}/opds")).unwrap();
    assert!(!root.is_acquisition());
    assert_eq!(root.entries.len(), 5);

    // follow "Newest": paginated with page_size 2
    let newest = root.entries[0].navigation_link().unwrap().href.clone();
    let r = get(addr, newest.strip_prefix(&base).unwrap());
    assert_eq!(r.status, 200);
    let page1 = parse_feed(&r.text(), &newest).unwrap();
    assert!(page1.is_acquisition());
    assert_eq!(page1.entries.len(), 2);
    let next = page1.next().expect("next link").to_string();
    let r = get(addr, next.strip_prefix(&base).unwrap());
    let page2 = parse_feed(&r.text(), &next).unwrap();
    assert_eq!(page2.entries.len(), 1);
    assert_eq!(page2.next(), None);

    // download through the acquisition link
    let all: Vec<_> = page1.entries.iter().chain(page2.entries.iter()).collect();
    assert_eq!(all.len(), 3);
    let dune = all.iter().find(|e| e.title == "Dune").unwrap();
    let acq = dune.best_acquisition(&["epub"]).unwrap();
    assert_eq!(acq.mime, "application/epub+zip");
    let r = get(addr, acq.href.strip_prefix(&base).unwrap());
    assert_eq!(r.body, fx.files[0]);
    let cover = dune.cover().unwrap();
    assert_eq!(
        get(addr, cover.href.strip_prefix(&base).unwrap()).body,
        b"JPEGDATA"
    );
    let thumb = dune.thumbnail().unwrap();
    assert_eq!(
        get(addr, thumb.href.strip_prefix(&base).unwrap()).status,
        200
    );

    // search through the advertised template
    let Some(SearchLink::Template(tpl)) = page1.search() else {
        panic!("template expected")
    };
    let url = expand_search_template(&tpl, "author:austen");
    let r = get(addr, url.strip_prefix(&base).unwrap());
    let found = parse_feed(&r.text(), &url).unwrap();
    assert_eq!(found.entries.len(), 1);
    assert_eq!(found.entries[0].title, "Emma");

    // opensearch description is served and parseable
    let r = get(addr, "/opds/opensearch.xml");
    let tpl = bookx_serve::opds_client::parse_opensearch_description(
        &r.text(),
        &format!("{base}/opds/opensearch.xml"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(tpl, format!("{base}/opds/search?q={{searchTerms}}"));

    // special characters in titles survive the whole chain
    let r = get(addr, "/opds/search?q=jerry");
    let f = parse_feed(&r.text(), &format!("{base}/opds/search")).unwrap();
    assert_eq!(f.entries[0].title, "Tom & Jerry <1>");
    srv.shutdown();
}

#[test]
fn basic_auth_protects_everything_but_health_and_kosync() {
    let (fx, books) = fixture();
    let srv = start(books, &fx.dir, |c| {
        c.users = vec![User {
            name: "reader".into(),
            password: "s3cret".into(),
        }];
        c.kosync = Some(kosync_mem());
    });
    let addr = srv.addr();
    for p in ["/", "/opds", "/download/1.epub", "/cover/1", "/opds/new"] {
        let r = get(addr, p);
        assert_eq!(r.status, 401, "{p}");
        assert!(r
            .header("www-authenticate")
            .unwrap()
            .starts_with("Basic realm="));
    }
    let good = basic("reader", "s3cret");
    for p in ["/", "/opds", "/download/1.epub", "/cover/1"] {
        let r = request(addr, "GET", p, &[("Authorization", good.as_str())], b"");
        assert_eq!(r.status, 200, "{p}");
    }
    for (u, p) in [
        ("reader", "wrong"),
        ("other", "s3cret"),
        ("", ""),
        ("reader", "s3cret "),
    ] {
        let h = basic(u, p);
        let r = request(addr, "GET", "/opds", &[("Authorization", h.as_str())], b"");
        assert_eq!(r.status, 401, "{u}:{p}");
    }
    let r = request(addr, "GET", "/opds", &[("Authorization", "Basic !!!")], b"");
    assert_eq!(r.status, 401);
    let r = request(
        addr,
        "GET",
        "/opds",
        &[("Authorization", "Bearer abc")],
        b"",
    );
    assert_eq!(r.status, 401);
    assert_eq!(get(addr, "/healthcheck").status, 200);
    assert_eq!(get(addr, "/kosync/healthcheck").status, 200);
    srv.shutdown();
}

#[test]
fn multiple_users() {
    let (fx, books) = fixture();
    let srv = start(books, &fx.dir, |c| {
        c.users = vec![
            User {
                name: "a".into(),
                password: "1".into(),
            },
            User {
                name: "b".into(),
                password: "2".into(),
            },
        ];
    });
    for (u, p, want) in [
        ("a", "1", 200),
        ("b", "2", 200),
        ("a", "2", 401),
        ("b", "1", 401),
    ] {
        let h = basic(u, p);
        let r = request(
            srv.addr(),
            "GET",
            "/opds",
            &[("Authorization", h.as_str())],
            b"",
        );
        assert_eq!(r.status, want, "{u}:{p}");
    }
}

#[test]
fn range_requests() {
    let (fx, books) = fixture();
    let data = fx.files[0].clone();
    let len = data.len();
    let srv = start(books, &fx.dir, |_| {});
    let addr = srv.addr();
    let ranged = |range: &str| request(addr, "GET", "/download/1.epub", &[("Range", range)], b"");

    let r = ranged("bytes=10-19");
    assert_eq!(r.status, 206);
    assert_eq!(r.body, &data[10..20]);
    assert_eq!(
        r.header("content-range"),
        Some(format!("bytes 10-19/{len}").as_str())
    );
    assert_eq!(r.header("content-length"), Some("10"));
    assert_eq!(r.header("accept-ranges"), Some("bytes"));

    let r = ranged("bytes=0-0");
    assert_eq!((r.status, r.body.as_slice()), (206, &data[..1]));

    let r = ranged(&format!("bytes={}-", len - 5));
    assert_eq!((r.status, r.body.as_slice()), (206, &data[len - 5..]));

    let r = ranged("bytes=-7");
    assert_eq!(r.status, 206);
    assert_eq!(r.body, &data[len - 7..]);
    assert_eq!(
        r.header("content-range"),
        Some(format!("bytes {}-{}/{len}", len - 7, len - 1).as_str())
    );

    // end past EOF is clamped
    let r = ranged(&format!("bytes=5-{}", len + 1000));
    assert_eq!(r.status, 206);
    assert_eq!(r.body, &data[5..]);

    // suffix longer than the file: whole file as 206
    let r = ranged(&format!("bytes=-{}", len + 10));
    assert_eq!((r.status, r.body.len()), (206, len));

    // unsatisfiable / invalid
    for bad in [
        format!("bytes={len}-"),
        "bytes=9999999-".into(),
        "bytes=5-2".into(),
        "bytes=abc".into(),
        "bytes=-0".into(),
        "bytes=-".into(),
    ] {
        let r = ranged(&bad);
        assert_eq!(r.status, 416, "{bad}");
        assert_eq!(
            r.header("content-range"),
            Some(format!("bytes */{len}").as_str()),
            "{bad}"
        );
        assert!(r.text().contains("not satisfiable"));
    }

    // multiple ranges and unknown units are ignored: full body
    let r = ranged("bytes=0-1,5-6");
    assert_eq!((r.status, r.body.len()), (200, len));
    let r = ranged("items=0-5");
    assert_eq!((r.status, r.body.len()), (200, len));

    // covers support ranges too
    let r = request(addr, "GET", "/cover/1", &[("Range", "bytes=1-3")], b"");
    assert_eq!((r.status, r.body.as_slice()), (206, &b"PEG"[..]));
    srv.shutdown();
}

#[test]
fn head_requests_have_headers_and_no_body() {
    let (fx, books) = fixture();
    let srv = start(books, &fx.dir, |_| {});
    let addr = srv.addr();
    let r = request(addr, "HEAD", "/download/1.epub", &[], b"");
    assert_eq!(r.status, 200);
    assert!(r.body.is_empty());
    assert_eq!(
        r.header("content-length"),
        Some(fx.files[0].len().to_string().as_str())
    );
    assert_eq!(r.header("content-type"), Some("application/epub+zip"));
    let r = request(addr, "HEAD", "/opds", &[], b"");
    assert_eq!(r.status, 200);
    assert!(r.body.is_empty());
    assert!(
        r.header("content-length")
            .unwrap()
            .parse::<usize>()
            .unwrap()
            > 100
    );
    let r = request(addr, "HEAD", "/download/42.epub", &[], b"");
    assert_eq!(r.status, 404);
    assert!(r.body.is_empty());
    srv.shutdown();
}

#[test]
fn path_traversal_attempts_never_leak_files() {
    let (fx, books) = fixture();
    // a secret outside the library root
    let secret = fx
        .dir
        .parent()
        .unwrap()
        .join(format!("bookx-secret-{}.txt", uuid::Uuid::new_v4()));
    std::fs::write(&secret, "TOP-SECRET").unwrap();
    let secret_name = secret.file_name().unwrap().to_string_lossy().into_owned();
    let srv = start(books, &fx.dir, |_| {});
    let addr = srv.addr();
    let attempts = [
        "/../etc/passwd".to_string(),
        "/download/../../etc/passwd".into(),
        "/download/%2e%2e/%2e%2e/etc/passwd".into(),
        "/download/..%2f..%2fetc%2fpasswd".into(),
        "/download/%2E%2E%5C%2E%2E%5Cetc%5Cpasswd".into(),
        format!("/download/../{secret_name}"),
        format!("/cover/../../{secret_name}"),
        "/opds/../../etc/passwd".into(),
        "//etc/passwd".into(),
        "/download/1.epub%00.txt".into(),
        "/download/\u{0}".into(),
        "/%2e%2e%2f%2e%2e%2fetc%2fpasswd".into(),
        "/..;/etc/passwd".into(),
        "/download/1.epub/../../../etc/passwd".into(),
    ];
    for a in &attempts {
        let raw = format!("GET {a} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
        let out = match raw_request(addr, raw.as_bytes()) {
            Ok(o) if !o.is_empty() => o,
            _ => continue, // the server refused to parse it at all: fine
        };
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(!text.contains("TOP-SECRET"), "leaked via {a:?}");
        assert!(!text.contains("root:x:"), "leaked passwd via {a:?}");
        let r = parse_response(&out);
        assert!(matches!(r.status, 400 | 404), "{a:?} gave {}", r.status);
    }
    // the server still works
    assert_eq!(get(addr, "/opds").status, 200);
    std::fs::remove_file(secret).ok();
    srv.shutdown();
}

#[test]
fn catalogue_entry_pointing_outside_the_root_is_not_served() {
    let (fx, mut books) = fixture();
    let outside = fx
        .dir
        .parent()
        .unwrap()
        .join(format!("bookx-outside-{}.epub", uuid::Uuid::new_v4()));
    std::fs::write(&outside, "OUTSIDE").unwrap();
    books.push(make_book(
        50,
        "Evil",
        "x",
        &format!("../{}", outside.file_name().unwrap().to_string_lossy()),
    ));
    let srv = start(books, &fx.dir, |_| {});
    let r = get(srv.addr(), "/download/50.epub");
    assert_eq!(r.status, 404);
    assert!(!r.text().contains("OUTSIDE"));
    std::fs::remove_file(outside).ok();
}

#[test]
fn huge_headers_and_urls_do_not_break_the_server() {
    let (fx, books) = fixture();
    let srv = start(books, &fx.dir, |_| {});
    let addr = srv.addr();

    // one 256 KiB header value
    let big = "A".repeat(256 * 1024);
    let raw = format!("GET /opds HTTP/1.1\r\nHost: x\r\nX-Big: {big}\r\nConnection: close\r\n\r\n");
    if let Ok(out) = raw_request(addr, raw.as_bytes()) {
        if !out.is_empty() {
            let r = parse_response(&out);
            assert!(matches!(r.status, 200 | 400 | 413 | 431), "{}", r.status);
        }
    }

    // 3000 headers
    let mut raw = String::from("GET /opds HTTP/1.1\r\nHost: x\r\nConnection: close\r\n");
    for i in 0..3000 {
        raw.push_str(&format!("X-H{i}: v\r\n"));
    }
    raw.push_str("\r\n");
    if let Ok(out) = raw_request(addr, raw.as_bytes()) {
        if !out.is_empty() {
            let r = parse_response(&out);
            assert!(matches!(r.status, 200 | 400 | 413 | 431), "{}", r.status);
        }
    }

    // 128 KiB URL
    let url = "a".repeat(128 * 1024);
    let raw = format!("GET /{url} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    if let Ok(out) = raw_request(addr, raw.as_bytes()) {
        if !out.is_empty() {
            let r = parse_response(&out);
            assert!(matches!(r.status, 404 | 400 | 414 | 431), "{}", r.status);
        }
    }

    // garbage that is not HTTP at all
    // (the server may keep waiting for more input, so only wait briefly for an answer)
    let short = Duration::from_millis(300);
    let _ = raw_request_timeout(addr, b"\x00\x01\x02 not http \xff\xfe\r\n\r\n", short);
    let _ = raw_request_timeout(addr, b"GET\r\n\r\n", short);
    let _ = raw_request_timeout(addr, b"", short);

    // still healthy afterwards
    assert_eq!(get(addr, "/opds").status, 200);
    assert_eq!(get(addr, "/healthcheck").status, 200);
    srv.shutdown();
}

#[test]
fn oversized_request_body_is_refused() {
    let (fx, books) = fixture();
    let srv = start(books, &fx.dir, |c| c.kosync = Some(kosync_mem()));
    let addr = srv.addr();
    let body = vec![b'x'; 2 * 1024 * 1024];
    let raw = {
        let mut r = format!(
            "POST /users/create HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        r.extend_from_slice(&body);
        r
    };
    // the server may close the connection before we finished sending
    if let Ok(out) = raw_request(addr, &raw) {
        if !out.is_empty() {
            assert_eq!(parse_response(&out).status, 413);
        }
    }
    assert_eq!(get(addr, "/healthcheck").status, 200);
    srv.shutdown();
}

#[test]
fn sixteen_concurrent_clients() {
    let (fx, books) = fixture();
    let srv = start(books, &fx.dir, |c| c.threads = 8);
    let addr = srv.addr();
    let files = Arc::new(fx.files.clone());
    let failures = Arc::new(Mutex::new(Vec::<String>::new()));
    let handles: Vec<_> = (0..16)
        .map(|t| {
            let files = files.clone();
            let failures = failures.clone();
            std::thread::spawn(move || {
                for i in 0..12 {
                    let id = (t + i) % 3 + 1;
                    let r = get(addr, &format!("/download/{id}.epub"));
                    if r.status != 200 || r.body != files[id - 1] {
                        failures
                            .lock()
                            .unwrap()
                            .push(format!("thread {t} download {id}: {}", r.status));
                    }
                    let r = get(addr, "/opds/all");
                    if r.status != 200 || roxmltree::Document::parse(&r.text()).is_err() {
                        failures
                            .lock()
                            .unwrap()
                            .push(format!("thread {t} opds: {}", r.status));
                    }
                    let r = request(
                        addr,
                        "GET",
                        &format!("/download/{id}.epub"),
                        &[("Range", "bytes=3-9")],
                        b"",
                    );
                    if r.status != 206 || r.body != files[id - 1][3..10] {
                        failures
                            .lock()
                            .unwrap()
                            .push(format!("thread {t} range {id}: {}", r.status));
                    }
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let f = failures.lock().unwrap();
    assert!(f.is_empty(), "{f:?}");
    srv.shutdown();
}

#[test]
fn keep_alive_connection_serves_several_requests() {
    let (fx, books) = fixture();
    let srv = start(books, &fx.dir, |_| {});
    let mut s = TcpStream::connect(srv.addr()).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    for _ in 0..3 {
        s.write_all(b"GET /healthcheck HTTP/1.1\r\nHost: x\r\n\r\n")
            .unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 512];
        // read until the JSON body is complete: {"state":"OK"} is 15 bytes
        loop {
            let n = s.read(&mut chunk).unwrap();
            assert!(n > 0, "connection closed early");
            buf.extend_from_slice(&chunk[..n]);
            if buf.ends_with(b"}") {
                break;
            }
        }
        let r = parse_response(&buf);
        assert_eq!(r.status, 200);
        assert_eq!(r.text(), r#"{"state":"OK"}"#);
    }
}

#[test]
fn shutdown_stops_the_listener() {
    let (fx, books) = fixture();
    let srv = start(books, &fx.dir, |_| {});
    let addr = srv.addr();
    assert_eq!(get(addr, "/healthcheck").status, 200);
    srv.shutdown();
    assert!(wait_closed(addr), "port should be closed after shutdown");
}

#[test]
fn dropping_the_handle_also_shuts_down() {
    let (fx, books) = fixture();
    let addr = {
        let srv = start(books, &fx.dir, |_| {});
        let a = srv.addr();
        assert_eq!(get(a, "/healthcheck").status, 200);
        a
    };
    assert!(wait_closed(addr), "port should be closed after drop");
}

#[test]
fn port_zero_gives_distinct_ports_and_busy_port_is_an_error() {
    let (fx, books) = fixture();
    let a = start(books.clone(), &fx.dir, |_| {});
    let b = start(books.clone(), &fx.dir, |_| {});
    assert_ne!(a.addr().port(), b.addr().port());

    let snap = LibrarySnapshot {
        root: fx.dir.clone(),
        books,
    };
    let mut cfg = ServerConfig::new(Arc::new(move || snap.clone()));
    cfg.port = a.addr().port();
    let err = Server::start(cfg).err().expect("port is busy");
    assert!(format!("{err:#}").contains("binding 127.0.0.1"), "{err:#}");

    let mut cfg = ServerConfig::new(Arc::new(LibrarySnapshot::default));
    cfg.bind = "not an address".into();
    assert!(Server::start(cfg).is_err());
}

#[test]
fn library_changes_are_visible_without_restart() {
    let (fx, books) = fixture();
    let shared = Arc::new(Mutex::new(books));
    let root = fx.dir.clone();
    let s2 = shared.clone();
    let provider = Arc::new(move || LibrarySnapshot {
        root: root.clone(),
        books: s2.lock().unwrap().clone(),
    });
    let srv = Server::start(ServerConfig::new(provider)).unwrap();
    let addr = srv.addr();
    assert!(get(addr, "/opds/all").text().contains("Dune"));
    shared.lock().unwrap().retain(|b| b.meta.title != "Dune");
    let after = get(addr, "/opds/all").text();
    assert!(!after.contains("Dune") && after.contains("Emma"));
    assert_eq!(get(addr, "/download/1.epub").status, 404);
}

#[test]
fn a_panicking_provider_does_not_kill_the_server() {
    let counter = Arc::new(Mutex::new(0));
    let c = counter.clone();
    let provider = Arc::new(move || {
        let mut n = c.lock().unwrap_or_else(|p| p.into_inner());
        *n += 1;
        if *n == 1 {
            panic!("boom");
        }
        LibrarySnapshot::default()
    });
    let mut cfg = ServerConfig::new(provider);
    cfg.threads = 2;
    let srv = Server::start(cfg).unwrap();
    assert_eq!(get(srv.addr(), "/opds").status, 500);
    assert_eq!(get(srv.addr(), "/opds").status, 200);
}

#[test]
fn read_only_mode_over_http() {
    let (fx, books) = fixture();
    let srv = start(books, &fx.dir, |c| {
        c.read_only = true;
        c.kosync = Some(kosync_mem());
    });
    let addr = srv.addr();
    assert_eq!(get(addr, "/opds").status, 200);
    assert_eq!(get(addr, "/download/1.epub").status, 200);
    for m in ["POST", "PUT", "DELETE", "PATCH"] {
        let r = request(addr, m, "/opds", &[], b"x");
        assert_eq!(r.status, 405, "{m}");
        assert_eq!(r.header("allow"), Some("GET, HEAD"));
    }
    let r = request(
        addr,
        "POST",
        "/users/create",
        &[],
        br#"{"username":"a","password":"b"}"#,
    );
    assert_eq!(r.status, 405);
    // reading existing sync data still works (needs auth, so 401 proves the route is live)
    assert_eq!(get(addr, "/users/auth").status, 401);
}

#[test]
fn kosync_protocol_over_http_with_json_file_store() {
    let (fx, books) = fixture();
    let store_path = fx.dir.join("state").join("kosync.json");
    let ko = Arc::new(KoSync::new(
        Box::new(JsonFileStore::open(&store_path).unwrap()),
        true,
    ));
    let srv = start(books, &fx.dir, |c| c.kosync = Some(ko));
    let addr = srv.addr();
    let json = [
        ("Content-Type", "application/json"),
        ("Accept", "application/vnd.koreader.v1+json"),
    ];

    let key = md5_hex("hunter2");
    let create = format!(r#"{{"username":"anna","password":"{key}"}}"#);
    let r = request(addr, "POST", "/users/create", &json, create.as_bytes());
    assert_eq!(r.status, 201);
    assert_eq!(r.header("content-type"), Some("application/json"));
    let r = request(addr, "POST", "/users/create", &json, create.as_bytes());
    assert_eq!(r.status, 402);

    let auth = [("x-auth-user", "anna"), ("x-auth-key", key.as_str())];
    assert_eq!(request(addr, "GET", "/users/auth", &auth, b"").status, 200);
    let bad = [("x-auth-user", "anna"), ("x-auth-key", "00")];
    assert_eq!(request(addr, "GET", "/users/auth", &bad, b"").status, 401);

    let doc = "9e107d9d372bb6826bd81d3542a419d6";
    let put = format!(
        r#"{{"document":"{doc}","progress":"/body/DocFragment[12]/body/p[3]/text().0","percentage":0.375,"device":"PocketBook","device_id":"DEV1"}}"#
    );
    let r = request(addr, "PUT", "/syncs/progress", &auth, put.as_bytes());
    assert_eq!(r.status, 200, "{}", r.text());
    let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
    assert_eq!(v["document"], doc);
    assert!(v["timestamp"].as_i64().unwrap() > 1_600_000_000);

    let r = request(addr, "GET", &format!("/syncs/progress/{doc}"), &auth, b"");
    assert_eq!(r.status, 200);
    let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
    assert_eq!(v["percentage"], 0.375);
    assert_eq!(v["device"], "PocketBook");
    assert_eq!(v["device_id"], "DEV1");
    assert_eq!(v["progress"], "/body/DocFragment[12]/body/p[3]/text().0");

    // the same under the /kosync prefix (what you type into KOReader as the server URL)
    let r = request(
        addr,
        "GET",
        &format!("/kosync/syncs/progress/{doc}"),
        &auth,
        b"",
    );
    assert_eq!(r.status, 200);
    assert_eq!(
        request(addr, "GET", "/syncs/progress/unknown", &auth, b"").status,
        404
    );
    assert_eq!(
        request(addr, "GET", &format!("/syncs/progress/{doc}"), &[], b"").status,
        401
    );
    srv.shutdown();

    // persistence across a restart with a fresh store instance
    let ko2 = Arc::new(KoSync::new(
        Box::new(JsonFileStore::open(&store_path).unwrap()),
        true,
    ));
    let (_fx2, books2) = fixture();
    let srv2 = start(books2, &fx.dir, |c| c.kosync = Some(ko2));
    let r = request(
        srv2.addr(),
        "GET",
        &format!("/syncs/progress/{doc}"),
        &auth,
        b"",
    );
    assert_eq!(r.status, 200);
    assert!(r.text().contains("0.375"));
}

#[test]
fn kosync_users_are_isolated_and_registration_can_be_closed() {
    let (fx, books) = fixture();
    let ko = Arc::new(KoSync::new(Box::new(MemoryStore::new()), false));
    ko.store().create_user("only", "k").unwrap();
    let srv = start(books, &fx.dir, |c| c.kosync = Some(ko));
    let addr = srv.addr();
    let r = request(
        addr,
        "POST",
        "/users/create",
        &[],
        br#"{"username":"new","password":"x"}"#,
    );
    assert_eq!(r.status, 403);
    let auth = [("x-auth-user", "only"), ("x-auth-key", "k")];
    assert_eq!(request(addr, "GET", "/users/auth", &auth, b"").status, 200);
}

#[test]
fn kosync_routes_404_when_not_configured() {
    let (fx, books) = fixture();
    let srv = start(books, &fx.dir, |_| {});
    assert_eq!(get(srv.addr(), "/users/auth").status, 404);
    assert_eq!(get(srv.addr(), "/healthcheck").status, 200);
}

#[test]
fn unicode_search_and_urls() {
    let (fx, mut books) = fixture();
    books.push(make_book(7, "日本語の本", "村上春樹", "1.epub"));
    let srv = start(books, &fx.dir, |_| {});
    let addr = srv.addr();
    let r = get(addr, "/?q=%E6%97%A5%E6%9C%AC");
    assert!(r.text().contains("日本語の本"));
    let r = get(addr, "/opds/authors");
    assert!(r.text().contains("村上春樹"));
    let f = parse_feed(&r.text(), &format!("http://{addr}/opds/authors")).unwrap();
    let e = f.entries.iter().find(|e| e.title == "村上春樹").unwrap();
    let href = e.navigation_link().unwrap().href.clone();
    let r = get(addr, href.strip_prefix(&format!("http://{addr}")).unwrap());
    assert_eq!(r.status, 200);
    assert!(r.text().contains("日本語の本"));
}
