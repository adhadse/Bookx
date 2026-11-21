//! Wallabag against a stub server on the loopback interface, through the real (guarded) HTTP client:
//! sign-in by POST, the list with the bearer token, pictures of the user's own server are fetched and
//! pictures on another private address are not (SSRF). No Internet; skipped with `NET=0`.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bookx_sources::fetch::UreqFetcher;
use bookx_sources::readability::{article_to_epub, fetch_images, ImageLimits};
use bookx_sources::wallabag::{self, Credentials, Query};

fn skip() -> bool {
    std::env::var("NET").is_ok_and(|v| v == "0")
}

type Seen = (String, String, Vec<(String, String)>, Vec<u8>);

fn read_request(s: &mut TcpStream) -> Option<Seen> {
    s.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let n = s.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.lines();
    let first = lines.next()?.to_string();
    let mut it = first.split(' ');
    let method = it.next()?.to_string();
    let target = it.next()?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let len = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    while buf.len() < head_end + len {
        let n = s.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Some((method, target, headers, buf[head_end..].to_vec()))
}

fn answer(s: &mut TcpStream, status: &str, ctype: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

#[test]
fn import_from_a_wallabag_server_on_the_local_network() {
    if skip() {
        return;
    }
    let png = bookx_sources::cover::generate_cover("T", "A", "", 200, 300).unwrap();

    // a second private address that must never be contacted
    let other = TcpListener::bind("127.0.0.32:0").unwrap();
    other.set_nonblocking(true).unwrap();
    let other_port = other.local_addr().unwrap().port();
    let other_hits = Arc::new(AtomicUsize::new(0));
    {
        let hits = other_hits.clone();
        std::thread::spawn(move || {
            let end = std::time::Instant::now() + Duration::from_secs(20);
            while std::time::Instant::now() < end {
                if other.accept().is_ok() {
                    hits.fetch_add(1, Ordering::SeqCst);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
    }

    let server = TcpListener::bind("127.0.0.31:0").unwrap();
    let port = server.local_addr().unwrap().port();
    let log: Arc<Mutex<Vec<Seen>>> = Arc::default();
    {
        let log = log.clone();
        let png = png.clone();
        std::thread::spawn(move || {
            for stream in server.incoming().take(40) {
                let Ok(mut s) = stream else { continue };
                let Some(req) = read_request(&mut s) else {
                    continue;
                };
                let (method, target) = (req.0.clone(), req.1.clone());
                let auth = req
                    .2
                    .iter()
                    .find(|(k, _)| k == "authorization")
                    .map(|(_, v)| v.clone());
                let body = String::from_utf8_lossy(&req.3).to_string();
                log.lock().unwrap().push(req);
                if method == "POST" && target == "/oauth/v2/token" {
                    if body.contains("grant_type=password") && body.contains("password=pw%21") {
                        answer(
                            &mut s,
                            "200 OK",
                            "application/json",
                            br#"{"access_token":"TOK123","expires_in":3600}"#,
                        );
                    } else {
                        answer(&mut s, "400 Bad Request", "application/json", b"{}");
                    }
                } else if target.starts_with("/api/entries.json") {
                    if auth.as_deref() != Some("Bearer TOK123") {
                        answer(&mut s, "401 Unauthorized", "application/json", b"{}");
                        continue;
                    }
                    let content = format!(
                        "<p>Saved text about gardening.</p><img src=\\\"http://127.0.0.31:{port}/own.png\\\"><img src=\\\"http://127.0.0.32:{other_port}/secret.png\\\">"
                    );
                    let json = format!(
                        r#"{{"page":1,"pages":1,"_embedded":{{"items":[{{"id":42,"url":"https://blog.example/garden","title":"Garden","content":"{content}","language":"en","published_by":["Pat"],"created_at":"2024-05-01T10:00:00+0000","tags":[],"is_archived":0}}]}}}}"#
                    );
                    answer(&mut s, "200 OK", "application/json", json.as_bytes());
                } else if target == "/own.png" {
                    answer(&mut s, "200 OK", "image/png", &png);
                } else {
                    answer(&mut s, "404 Not Found", "text/plain", b"no");
                }
            }
        });
    }

    let base = wallabag::normalize_base(&format!("http://127.0.0.31:{port}/")).unwrap();
    let fetcher = UreqFetcher::with_limits(Duration::from_secs(5), 5 * 1024 * 1024);
    let creds = Credentials {
        client_id: "cid".into(),
        client_secret: "sec".into(),
        username: "pat".into(),
        password: "pw!".into(),
    };
    let token = wallabag::sign_in(&fetcher, &base, &creds).unwrap();
    assert_eq!(token, "TOK123");
    // a wrong password is a clear message, not a hang
    let bad = Credentials {
        password: "nope".into(),
        ..creds.clone()
    };
    let e = wallabag::sign_in(&fetcher, &base, &bad).unwrap_err();
    assert!(format!("{e:#}").contains("refused the sign-in"), "{e:#}");
    let e = wallabag::fetch_entries(&fetcher, &base, "stale", &Query::default(), &|_| false)
        .unwrap_err();
    assert!(format!("{e:#}").contains("sign in again"), "{e:#}");

    let entries =
        wallabag::fetch_entries(&fetcher, &base, &token, &Query::default(), &|_| false).unwrap();
    assert_eq!(entries.len(), 1);
    let article = wallabag::entry_article(&entries[0]);
    assert!(article.content.contains("gardening"));
    assert_eq!(article.images.len(), 2);
    let images = fetch_images(&fetcher, &article.images, &ImageLimits::default());
    assert_eq!(images.len(), 1, "only the picture of the user's own server");
    assert!(images[0].url.contains("/own.png"));
    assert_eq!(
        other_hits.load(Ordering::SeqCst),
        0,
        "no connection to the other private address"
    );
    let epub = article_to_epub(&article, &images).unwrap();
    assert!(epub.get_string("mimetype").is_some());

    // the sign-in was a POST with a form body, the list a GET with the bearer header
    let log = log.lock().unwrap();
    assert!(log.iter().any(|r| r.0 == "POST"
        && r.2
            .iter()
            .any(|(k, v)| k == "content-type" && v == "application/x-www-form-urlencoded")));
    assert!(log
        .iter()
        .any(|r| r.0 == "GET" && r.1.starts_with("/api/entries.json")));
}
