//! Raw-socket client and fixtures for the transport hardening tests.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use bookx_serve::kosync::{KoSync, MemoryStore};
use bookx_serve::net::Limits;
use bookx_serve::server::{LibrarySnapshot, Server, ServerConfig, ServerHandle, User};

#[derive(Debug)]
pub struct Resp {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

pub struct Client {
    pub s: TcpStream,
    buf: Vec<u8>,
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

impl Client {
    pub fn connect(addr: SocketAddr) -> Client {
        let s = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).expect("connect");
        s.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
        s.set_write_timeout(Some(Duration::from_secs(8))).unwrap();
        Client { s, buf: Vec::new() }
    }

    pub fn set_timeout(&mut self, d: Duration) {
        self.s.set_read_timeout(Some(d)).unwrap();
    }

    pub fn send(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.s.write_all(bytes)
    }

    fn more(&mut self) -> bool {
        let mut chunk = [0u8; 16384];
        match self.s.read(&mut chunk) {
            Ok(0) | Err(_) => false,
            Ok(n) => {
                self.buf.extend_from_slice(&chunk[..n]);
                true
            }
        }
    }

    /// Reads one response; `None` if the connection ended (or timed out) before a full head.
    pub fn read_response(&mut self, head_only: bool) -> Option<Resp> {
        let end = loop {
            if let Some(i) = find(&self.buf, b"\r\n\r\n") {
                break i;
            }
            if !self.more() {
                return None;
            }
        };
        let head = String::from_utf8_lossy(&self.buf[..end]).into_owned();
        self.buf.drain(..end + 4);
        let mut lines = head.split("\r\n");
        let status_line = lines.next()?;
        let status: u16 = status_line.split(' ').nth(1)?.parse().ok()?;
        assert!(
            status_line.starts_with("HTTP/1.1 ") || status_line.starts_with("HTTP/1.0 "),
            "{status_line}"
        );
        let headers: Vec<(String, String)> = lines
            .map(|l| {
                let (k, v) = l.split_once(':').expect("header colon");
                (k.trim().to_string(), v.trim().to_string())
            })
            .collect();
        let mut r = Resp {
            status,
            headers,
            body: Vec::new(),
        };
        let bodyless = head_only || matches!(status, 100..=199 | 204 | 304);
        if !bodyless {
            let n: usize = r.header("content-length")?.parse().ok()?;
            while self.buf.len() < n {
                if !self.more() {
                    return None;
                }
            }
            r.body = self.buf.drain(..n).collect();
        }
        Some(r)
    }

    pub fn get(&mut self, path: &str, extra: &str) -> Resp {
        self.send(format!("GET {path} HTTP/1.1\r\nHost: x\r\n{extra}\r\n").as_bytes())
            .unwrap();
        self.read_response(false).expect("response")
    }

    /// True once the server has closed the connection (EOF or reset) within `within`.
    pub fn is_closed(&mut self, within: Duration) -> bool {
        self.s.set_read_timeout(Some(within)).unwrap();
        let mut b = [0u8; 4096];
        let deadline = Instant::now() + within;
        loop {
            match self.s.read(&mut b) {
                Ok(0) => return true,
                Ok(_) if Instant::now() < deadline => continue, // trailing bytes, keep reading
                Ok(_) => return false,
                Err(e) => {
                    return !matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    )
                }
            }
        }
    }
}

/// One request on a fresh connection, `Connection: close`.
pub fn once(addr: SocketAddr, raw: &[u8]) -> Option<Resp> {
    let mut c = Client::connect(addr);
    c.send(raw).ok()?;
    c.read_response(false)
}

pub fn basic(u: &str, p: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{u}:{p}"))
    )
}

pub const USER: &str = "reader";
pub const PASS: &str = "s3cret";

pub fn auth_header() -> String {
    format!("Authorization: {}\r\n", basic(USER, PASS))
}

pub fn start_with(
    tweak_limits: impl FnOnce(&mut Limits),
    tweak: impl FnOnce(&mut ServerConfig),
) -> ServerHandle {
    let mut cfg = ServerConfig::new(Arc::new(LibrarySnapshot::default));
    cfg.users = vec![User {
        name: USER.into(),
        password: PASS.into(),
    }];
    cfg.kosync = Some(Arc::new(KoSync::new(Box::new(MemoryStore::new()), true)));
    cfg.threads = 4;
    tweak_limits(&mut cfg.limits);
    tweak(&mut cfg);
    Server::start(cfg).expect("server starts")
}

/// Fast limits so timeouts show up in a few hundred milliseconds.
pub fn start_fast(tweak_limits: impl FnOnce(&mut Limits)) -> ServerHandle {
    start_with(
        |l| {
            l.header_timeout = Duration::from_millis(500);
            l.body_timeout = Duration::from_millis(500);
            l.idle_timeout = Duration::from_millis(500);
            l.write_timeout = Duration::from_secs(5);
            tweak_limits(l);
        },
        |_| {},
    )
}

/// Polls `cond` for up to `max`.
pub fn eventually(max: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + max;
    while Instant::now() < end {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    cond()
}
