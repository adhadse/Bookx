//! A KOSync client: the other half of [`crate::kosync`]. It talks to Bookx's own server (mounted at
//! `/bookx` of a share) and to any server that implements KOReader's progress-sync protocol
//! (`https://sync.koreader.rocks`, a self-hosted kosync, Calibre-Web's `/kosync`).
//!
//! Protocol, as KOReader's `kosync` plugin speaks it:
//!
//! | Call | Request | Answer |
//! |---|---|---|
//! | register | `POST /users/create` `{"username","password"}`, `password` being the MD5 hex of the real one | `201`; `402` name taken; `403` registration closed |
//! | log in | `GET /users/auth` | `200` or `401` |
//! | push | `PUT /syncs/progress` `{"document","progress","percentage","device","device_id"}` | `200` `{"document","timestamp"}` |
//! | pull | `GET /syncs/progress/<document>` | `200` with the same fields plus `timestamp`, else `404` (some servers answer `200 {}`) |
//!
//! Every request carries `x-auth-user`, `x-auth-key` (the MD5 hex of the password, as KOReader
//! sends it) and `Accept: application/vnd.koreader.v1+json`.
//!
//! A book is named by its **document id**: either the *partial MD5* of the file ([`partial_md5`]:
//! the MD5 of up to 1 KiB read at each of the offsets 0, 1 KiB, 4 KiB, 16 KiB, … 1 GiB, i.e.
//! `1024 << 2i` for `i` = 0…10, stopping at the first offset past the end of the file) or the MD5
//! of the file name without its folder ([`filename_digest`]). These are the two methods of KOReader's
//! "Document matching method" setting; an id only matches when both sides use the same method
//! and, for the first, a byte-identical file.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use crate::kosync::md5_hex;

/// How a book is named on the server (KOReader's *Document matching method*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DocMethod {
    /// MD5 of samples of the file's bytes: survives renaming, breaks when the file changes.
    #[default]
    Binary,
    /// MD5 of the file name: survives a changed file, breaks when it is renamed.
    Filename,
}

/// The partial MD5 KOReader calls a book's "binary" id, from any seekable source.
pub use bookx_core::docid::{partial_md5, partial_md5_of};

/// The MD5 of a file name (without its folder), KOReader's *Filename* id.
pub fn filename_digest(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy();
    Some(md5_hex(&name))
}

/// The id of the book at `path` by `method`.
pub fn document_id(path: &Path, method: DocMethod) -> Result<String> {
    match method {
        DocMethod::Binary => {
            partial_md5(path).with_context(|| format!("reading {}", path.display()))
        }
        DocMethod::Filename => filename_digest(path).ok_or_else(|| anyhow!("no file name")),
    }
}

/// A position the server has.
#[derive(Debug, Clone, PartialEq)]
pub struct Remote {
    pub document: String,
    /// KOReader's place (an xpointer such as `/body/DocFragment[3]/body/p[2]/text().10`, or a page
    /// number for PDFs); opaque to the server.
    pub progress: String,
    /// 0.0 to 1.0
    pub percentage: f64,
    /// Name of the device that pushed it.
    pub device: String,
    pub device_id: String,
    /// Seconds since the epoch, set by the server.
    pub timestamp: i64,
}

/// What a push says.
#[derive(Debug, Clone, PartialEq)]
pub struct Push<'a> {
    pub document: &'a str,
    pub progress: &'a str,
    pub percentage: f64,
    pub device: &'a str,
    pub device_id: &'a str,
}

/// Why a call failed, apart from not reaching the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    BadCredentials,
    NameTaken,
    RegistrationClosed,
    /// The server answered 404 to a push: it has no book with this document id. Servers that match
    /// books by the content of the file (Grimmory, BookLore) answer so when their copy is not
    /// byte-identical to ours.
    UnknownBook(String),
    /// The server answered something else.
    Status(u16),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::BadCredentials => {
                write!(f, "the server did not accept the user name and password")
            }
            Failure::NameTaken => write!(f, "that user name is already registered (use Log in)"),
            Failure::RegistrationClosed => write!(f, "this server does not allow new accounts"),
            Failure::UnknownBook(id) => write!(
                f,
                "the server does not know this book (document id {id}); servers such as Grimmory match a book by the content of its file, so the file here must be identical to the server's copy"
            ),
            Failure::Status(s) => write!(f, "the server answered {s}"),
        }
    }
}

impl std::error::Error for Failure {}

pub struct Client {
    base: url::Url,
    user: String,
    key: String,
    timeout: Duration,
    trusted_pem: Option<Vec<u8>>,
}

/// `server` as a base URL: a scheme is added (`https://`) when there is none, and a trailing
/// slash is dropped.
pub fn base_url(server: &str) -> Result<url::Url> {
    let s = server.trim();
    if s.is_empty() {
        bail!("enter the address of the sync server");
    }
    let with_scheme = if s.contains("://") {
        s.to_string()
    } else {
        format!("https://{s}")
    };
    let mut u = url::Url::parse(&with_scheme).context("that is not a web address")?;
    if !matches!(u.scheme(), "http" | "https") {
        bail!("the address must start with http:// or https://");
    }
    let path = u.path().trim_end_matches('/').to_string();
    u.set_path(&path);
    u.set_query(None);
    u.set_fragment(None);
    Ok(u)
}

impl Client {
    /// `password` is the real one; the client sends its MD5 hex like KOReader does.
    pub fn new(server: &str, user: &str, password: &str) -> Result<Client> {
        if user.trim().is_empty() {
            bail!("enter the user name");
        }
        Ok(Client {
            base: base_url(server)?,
            user: user.trim().to_string(),
            key: md5_hex(password),
            timeout: Duration::from_secs(15),
            trusted_pem: None,
        })
    }

    pub fn with_timeout(mut self, t: Duration) -> Client {
        self.timeout = t;
        self
    }

    /// Trust this PEM certificate instead of the public roots (a self-signed home server).
    pub fn trusting(mut self, pem: Vec<u8>) -> Client {
        self.trusted_pem = Some(pem);
        self
    }

    fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<crate::httpc::Reply> {
        let mut url = self.base.clone();
        let full = format!("{}{}", self.base.path().trim_end_matches('/'), path);
        url.set_path(&full);
        let json = body.map(|b| b.to_string()).unwrap_or_default();
        let mut headers: Vec<(&str, &str)> = vec![
            ("Accept", "application/vnd.koreader.v1+json"),
            ("x-auth-user", &self.user),
            ("x-auth-key", &self.key),
        ];
        if body.is_some() {
            headers.push(("Content-Type", "application/json"));
        }
        crate::httpc::request_trusting(
            method,
            &url,
            &headers,
            json.as_bytes(),
            self.timeout,
            self.trusted_pem.as_deref(),
        )
        .with_context(|| format!("talking to {}", self.base))
    }

    /// Creates the account.
    pub fn register(&self) -> Result<()> {
        let body = json!({"username": self.user, "password": self.key});
        let r = self.call("POST", "/users/create", Some(&body))?;
        match r.status {
            200 | 201 => Ok(()),
            402 => Err(Failure::NameTaken.into()),
            403 => Err(Failure::RegistrationClosed.into()),
            s => Err(Failure::Status(s).into()),
        }
    }

    /// Checks the user name and password.
    pub fn login(&self) -> Result<()> {
        match self.call("GET", "/users/auth", None)?.status {
            200 => Ok(()),
            401 => Err(Failure::BadCredentials.into()),
            s => Err(Failure::Status(s).into()),
        }
    }

    /// The position stored for `document`, `None` when the server has none.
    pub fn get_progress(&self, document: &str) -> Result<Option<Remote>> {
        let r = self.call("GET", &format!("/syncs/progress/{document}"), None)?;
        match r.status {
            200 => {}
            404 => return Ok(None),
            401 => return Err(Failure::BadCredentials.into()),
            s => return Err(Failure::Status(s).into()),
        }
        let v: Value = serde_json::from_slice(&r.body).context("the server's answer")?;
        let Some(pct) = v.get("percentage").and_then(Value::as_f64) else {
            // servers that have nothing answer 200 with an empty object
            return Ok(None);
        };
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        Ok(Some(Remote {
            document: document.to_string(),
            progress: s("progress"),
            percentage: pct.clamp(0.0, 1.0),
            device: s("device"),
            device_id: s("device_id"),
            timestamp: v.get("timestamp").and_then(Value::as_i64).unwrap_or(0),
        }))
    }

    /// The position of the first of `ids` the server has, with that id.
    pub fn get_progress_any(&self, ids: &[String]) -> Result<Option<(String, Remote)>> {
        for id in ids {
            if let Some(r) = self.get_progress(id)? {
                return Ok(Some((id.clone(), r)));
            }
        }
        Ok(None)
    }

    /// Stores a position under the first of `ids` the server accepts (a server that matches books by
    /// their content answers 404 for an id it does not know); returns that id and the time stamp.
    /// Every id unknown gives [`Failure::UnknownBook`] for the first.
    pub fn put_progress_any(&self, ids: &[String], p: &Push) -> Result<(String, i64)> {
        for id in ids {
            let p = Push {
                document: id,
                ..p.clone()
            };
            match self.put_progress(&p) {
                Ok(ts) => return Ok((id.clone(), ts)),
                Err(e) if matches!(e.downcast_ref::<Failure>(), Some(Failure::UnknownBook(_))) => {}
                Err(e) => return Err(e),
            }
        }
        Err(Failure::UnknownBook(ids.first().cloned().unwrap_or_default()).into())
    }

    /// Stores a position; returns the server's time stamp.
    pub fn put_progress(&self, p: &Push) -> Result<i64> {
        let body = json!({
            "document": p.document,
            "progress": p.progress,
            "percentage": p.percentage.clamp(0.0, 1.0),
            "device": p.device,
            "device_id": p.device_id,
        });
        let r = self.call("PUT", "/syncs/progress", Some(&body))?;
        match r.status {
            200 | 202 => Ok(serde_json::from_slice::<Value>(&r.body)
                .ok()
                .and_then(|v| v.get("timestamp").and_then(Value::as_i64))
                .unwrap_or(0)),
            401 => Err(Failure::BadCredentials.into()),
            404 => Err(Failure::UnknownBook(p.document.to_string()).into()),
            s => Err(Failure::Status(s).into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kosync::{KoSync, MemoryStore};
    use crate::server::{LibrarySnapshot, Server, ServerConfig};
    use std::io::Cursor;
    use std::sync::Arc;

    fn net_off() -> bool {
        std::env::var("NET").is_ok_and(|v| v == "0")
    }

    fn server(registration: bool) -> crate::server::ServerHandle {
        let provider: crate::server::LibraryProvider = Arc::new(|| LibrarySnapshot {
            root: std::env::temp_dir(),
            books: Vec::new(),
        });
        let mut cfg = ServerConfig::new(provider);
        cfg.kosync = Some(Arc::new(KoSync::new(
            Box::new(MemoryStore::new()),
            registration,
        )));
        Server::start(cfg).unwrap()
    }

    fn md5_of(bytes: &[u8]) -> String {
        format!("{:x}", md5::compute(bytes))
    }

    #[test]
    fn partial_md5_samples_the_documented_offsets() {
        // shorter than the second sample: the first KiB only
        let small: Vec<u8> = (0..700u32).map(|i| i as u8).collect();
        assert_eq!(
            partial_md5_of(&mut Cursor::new(&small)).unwrap(),
            md5_of(&small)
        );
        // 5000 bytes: [0,1024) [1024,2048) [4096,5000), then 16384 is past the end
        let data: Vec<u8> = (0..5000u32).map(|i| (i * 7 + 3) as u8).collect();
        let mut want = Vec::new();
        want.extend_from_slice(&data[0..1024]);
        want.extend_from_slice(&data[1024..2048]);
        want.extend_from_slice(&data[4096..5000]);
        assert_eq!(
            partial_md5_of(&mut Cursor::new(&data)).unwrap(),
            md5_of(&want)
        );
        // a file with a gap: the sample at 16384 is read even though 8192 is not an offset
        let big: Vec<u8> = (0..70_000u32).map(|i| (i % 251) as u8).collect();
        let mut want = Vec::new();
        for off in [0usize, 1024, 4096, 16384, 65536] {
            want.extend_from_slice(&big[off..(off + 1024).min(big.len())]);
        }
        assert_eq!(
            partial_md5_of(&mut Cursor::new(&big)).unwrap(),
            md5_of(&want)
        );
        // a change between the samples does not change the id, a change inside one does
        let mut other = big.clone();
        other[10_000] ^= 0xff;
        assert_eq!(
            partial_md5_of(&mut Cursor::new(&other)).unwrap(),
            md5_of(&want)
        );
        other[4100] ^= 0xff;
        assert_ne!(
            partial_md5_of(&mut Cursor::new(&other)).unwrap(),
            md5_of(&want)
        );
        assert_eq!(
            partial_md5_of(&mut Cursor::new(Vec::<u8>::new())).unwrap(),
            md5_of(b"")
        );
    }

    #[test]
    fn the_filename_id_is_the_md5_of_the_name_only() {
        assert_eq!(
            filename_digest(Path::new("/some/folder/Dune.epub")).unwrap(),
            md5_hex("Dune.epub")
        );
        assert_eq!(
            filename_digest(Path::new("Dune.epub")),
            filename_digest(Path::new("/other/Dune.epub"))
        );
        let d = std::env::temp_dir().join(format!("bookx-kc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("A.epub"), vec![1u8; 3000]).unwrap();
        assert_eq!(
            document_id(&d.join("A.epub"), DocMethod::Binary).unwrap(),
            md5_of(&vec![1u8; 2048])
        );
        assert_eq!(
            document_id(&d.join("A.epub"), DocMethod::Filename).unwrap(),
            md5_hex("A.epub")
        );
        assert!(document_id(&d.join("missing.epub"), DocMethod::Binary).is_err());
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn server_addresses_are_normalised() {
        assert_eq!(
            base_url("sync.koreader.rocks").unwrap().as_str(),
            "https://sync.koreader.rocks/"
        );
        assert_eq!(
            base_url(" http://192.168.1.5:8080/bookx/ ").unwrap().path(),
            "/bookx"
        );
        assert!(base_url("").is_err());
        assert!(base_url("ftp://x").is_err());
    }

    #[test]
    fn register_log_in_push_and_pull_against_bookxs_own_server() {
        if net_off() {
            return;
        }
        let s = server(true);
        let url = format!("{}/bookx", s.base_url());
        let a = Client::new(&url, "alice", "pw").unwrap();
        assert!(a.login().unwrap_err().downcast_ref::<Failure>() == Some(&Failure::BadCredentials));
        a.register().unwrap();
        assert_eq!(
            a.register().unwrap_err().downcast_ref::<Failure>(),
            Some(&Failure::NameTaken)
        );
        a.login().unwrap();
        let wrong = Client::new(&url, "alice", "other").unwrap();
        assert_eq!(
            wrong.login().unwrap_err().downcast_ref::<Failure>(),
            Some(&Failure::BadCredentials)
        );
        assert_eq!(a.get_progress("abc123").unwrap(), None);
        let ts = a
            .put_progress(&Push {
                document: "abc123",
                progress: "/body/DocFragment[3]/body",
                percentage: 0.63,
                device: "Bookx on laptop",
                device_id: "dev-1",
            })
            .unwrap();
        assert!(ts > 0);
        // a second client of the same account (another device) sees it
        let b = Client::new(&url, "alice", "pw").unwrap();
        let r = b.get_progress("abc123").unwrap().unwrap();
        assert_eq!(r.percentage, 0.63);
        assert_eq!(r.device, "Bookx on laptop");
        assert_eq!(r.device_id, "dev-1");
        assert_eq!(r.progress, "/body/DocFragment[3]/body");
        assert_eq!(r.timestamp, ts);
        // there is no root-level mount and no /kosync alias: only /bookx answers
        for bad in [s.base_url(), format!("{}/kosync", s.base_url())] {
            let other = Client::new(&bad, "alice", "pw").unwrap();
            assert!(
                other.get_progress("abc123").is_err() || other.login().is_err(),
                "{bad}"
            );
        }
        // another account does not see it
        let c = Client::new(&url, "bob", "pw").unwrap();
        c.register().unwrap();
        assert_eq!(c.get_progress("abc123").unwrap(), None);
        s.shutdown();
    }

    /// A server that matches books by file content like Grimmory: it knows one document id and
    /// answers 404 to a push (or a pull) for any other.
    fn content_matching_server(known: &'static str) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/api/koreader", l.local_addr().unwrap());
        let h = std::thread::spawn(move || {
            for _ in 0..4 {
                let (mut c, _) = l.accept().unwrap();
                let mut buf = vec![0u8; 8192];
                let mut n = 0;
                loop {
                    let k = c.read(&mut buf[n..]).unwrap();
                    n += k;
                    let text = String::from_utf8_lossy(&buf[..n]).to_string();
                    if let Some(h) = text.find("\r\n\r\n") {
                        let len = text
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if n >= h + 4 + len {
                            break;
                        }
                    }
                    if k == 0 {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&buf[..n]).to_string();
                let ok =
                    text.starts_with("PUT") && text.contains(&format!("\"document\":\"{known}\""));
                let (status, body) = if ok {
                    ("200 OK", "{\"document\":\"x\",\"timestamp\":1}")
                } else {
                    ("404 Not Found", "{\"message\":\"Book not found for hash\"}")
                };
                let _ = write!(
                    c,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        (url, h)
    }

    #[test]
    fn a_server_that_matches_by_content_says_unknown_book_and_the_original_id_is_found() {
        // regression for issue 49: Grimmory answered 404 to the id of Bookx's rewritten copy
        if net_off() {
            return;
        }
        let (url, h) = content_matching_server("original-id");
        let c = Client::new(&url, "u", "p").unwrap();
        let p = Push {
            document: "copy-id",
            progress: "5",
            percentage: 0.4,
            device: "Bookx",
            device_id: "d",
        };
        let e = c.put_progress(&p).unwrap_err();
        assert_eq!(
            e.downcast_ref::<Failure>(),
            Some(&Failure::UnknownBook("copy-id".into()))
        );
        let msg = e.to_string();
        assert!(
            msg.contains("copy-id") && msg.contains("content of its file"),
            "{msg}"
        );
        // with the id of the original as another candidate the push lands
        let ids = vec!["copy-id".to_string(), "original-id".to_string()];
        let (doc, _) = c.put_progress_any(&ids, &p).unwrap();
        assert_eq!(doc, "original-id");
        // and when no candidate is known the first is named
        let e = c.put_progress_any(&["nope".to_string()], &p).unwrap_err();
        assert_eq!(
            e.downcast_ref::<Failure>(),
            Some(&Failure::UnknownBook("nope".into()))
        );
        h.join().unwrap();
    }

    #[test]
    fn closed_registration_and_dead_servers_are_reported() {
        if net_off() {
            return;
        }
        let s = server(false);
        let c = Client::new(&format!("{}/bookx", s.base_url()), "alice", "pw").unwrap();
        assert_eq!(
            c.register().unwrap_err().downcast_ref::<Failure>(),
            Some(&Failure::RegistrationClosed)
        );
        let url = s.base_url();
        s.shutdown();
        let dead = Client::new(&url, "alice", "pw")
            .unwrap()
            .with_timeout(Duration::from_secs(2));
        let e = dead.login().unwrap_err();
        assert!(e.downcast_ref::<Failure>().is_none(), "{e:#}");
    }
}
