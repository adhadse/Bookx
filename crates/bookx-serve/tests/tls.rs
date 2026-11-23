//! HTTPS: a server started with a rustls configuration from PEM files, talked to by a rustls
//! client that trusts a self-signed certificate generated here.

mod common;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bookx_serve::net::tls_config;
use bookx_serve::server::{LibrarySnapshot, Server, ServerConfig, ServerHandle};
use rustls::pki_types::{CertificateDer, ServerName};

struct Fixture {
    dir: PathBuf,
    cert_der: CertificateDer<'static>,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = std::env::temp_dir().join(format!("bookx-tls-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        std::fs::write(dir.join("cert.pem"), cert.cert.pem()).unwrap();
        std::fs::write(dir.join("key.pem"), cert.signing_key.serialize_pem()).unwrap();
        Fixture {
            dir,
            cert_der: cert.cert.der().clone(),
        }
    }

    fn start(&self, tweak: impl FnOnce(&mut ServerConfig)) -> ServerHandle {
        let mut cfg = ServerConfig::new(Arc::new(LibrarySnapshot::default));
        cfg.tls = Some(tls_config(&self.dir.join("cert.pem"), &self.dir.join("key.pem")).unwrap());
        cfg.limits.header_timeout = Duration::from_millis(800);
        cfg.limits.idle_timeout = Duration::from_millis(800);
        tweak(&mut cfg);
        Server::start(cfg).expect("tls server starts")
    }

    fn client_config(&self) -> Arc<rustls::ClientConfig> {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(self.cert_der.clone()).unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        Arc::new(
            rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

type Tls = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

fn connect(cfg: &Arc<rustls::ClientConfig>, addr: SocketAddr, name: &str) -> Tls {
    let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
    let conn =
        rustls::ClientConnection::new(cfg.clone(), ServerName::try_from(name.to_string()).unwrap())
            .unwrap();
    rustls::StreamOwned::new(conn, tcp)
}

/// Reads one response with a `Content-Length` body from a TLS stream.
fn read_response(s: &mut Tls) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        let n = s.read(&mut chunk).expect("read head");
        assert!(n > 0, "eof before the head");
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let status: u16 = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers: Vec<(String, String)> = lines
        .map(|l| {
            let (k, v) = l.split_once(':').unwrap();
            (k.trim().to_ascii_lowercase(), v.trim().to_string())
        })
        .collect();
    let len: usize = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .map(|(_, v)| v.parse().unwrap())
        .unwrap_or(0);
    let mut body = buf[end + 4..].to_vec();
    while body.len() < len {
        let n = s.read(&mut chunk).expect("read body");
        assert!(n > 0, "eof in the body");
        body.extend_from_slice(&chunk[..n]);
    }
    (status, headers, body)
}

#[test]
fn https_request_response_and_keep_alive() {
    let fx = Fixture::new();
    let srv = fx.start(|_| {});
    assert!(srv.base_url().starts_with("https://127.0.0.1:"));
    let cfg = fx.client_config();
    let mut s = connect(&cfg, srv.addr(), "localhost");
    for _ in 0..3 {
        s.write_all(b"GET /healthcheck HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let (status, headers, body) = read_response(&mut s);
        assert_eq!(status, 200);
        assert_eq!(body, br#"{"state":"OK"}"#);
        assert!(headers.contains(&("connection".into(), "keep-alive".into())));
    }
    assert_eq!(srv.open_connections(), 1);
    let proto = s.conn.protocol_version().expect("negotiated");
    assert!(
        matches!(
            proto,
            rustls::ProtocolVersion::TLSv1_3 | rustls::ProtocolVersion::TLSv1_2
        ),
        "{proto:?}"
    );
    assert_eq!(s.conn.alpn_protocol(), None, "client offered no ALPN");
    drop(s);
    srv.shutdown();
}

#[test]
fn https_post_with_a_body_and_expect_continue() {
    let fx = Fixture::new();
    let srv = fx.start(|c| {
        c.kosync = Some(Arc::new(bookx_serve::kosync::KoSync::new(
            Box::new(bookx_serve::kosync::MemoryStore::new()),
            true,
        )));
    });
    let cfg = fx.client_config();
    let mut s = connect(&cfg, srv.addr(), "localhost");
    let body = br#"{"username":"tls","password":"pw"}"#;
    s.write_all(
        format!(
            "POST /bookx/users/create HTTP/1.1\r\nHost: x\r\nExpect: 100-continue\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .as_bytes(),
    )
    .unwrap();
    let (status, ..) = read_response(&mut s);
    assert_eq!(status, 100);
    s.write_all(body).unwrap();
    let (status, ..) = read_response(&mut s);
    assert_eq!(status, 201);
}

#[test]
fn https_large_download_arrives_intact() {
    let fx = Fixture::new();
    let dir = std::env::temp_dir().join(format!("bookx-tls-dl-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let data: Vec<u8> = (0..3 * 1024 * 1024u32).map(|i| (i % 253) as u8).collect();
    std::fs::write(dir.join("1.epub"), &data).unwrap();
    let book = {
        use chrono::Utc;
        bookx_core::library::Book {
            id: 1,
            file: "1.epub".into(),
            cover: None,
            meta: bookx_core::opf::Metadata {
                title: "T".into(),
                authors: vec!["A".into()],
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
    };
    let snap = LibrarySnapshot {
        root: dir.clone(),
        books: vec![book],
    };
    let srv = fx.start(|c| c.library = Arc::new(move || snap.clone()));
    let mut s = connect(&fx.client_config(), srv.addr(), "localhost");
    s.write_all(b"GET /download/1.epub HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    let (status, _, body) = read_response(&mut s);
    assert_eq!(status, 200);
    assert!(body == data);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_client_that_does_not_trust_the_certificate_is_refused() {
    let fx = Fixture::new();
    let srv = fx.start(|_| {});
    let other = Fixture::new();
    let mut s = connect(&other.client_config(), srv.addr(), "localhost");
    let r = s.write_all(b"GET /healthcheck HTTP/1.1\r\nHost: x\r\n\r\n");
    let mut out = Vec::new();
    let r2 = s.read_to_end(&mut out);
    assert!(r.is_err() || r2.is_err(), "handshake must fail");
    // and a wrong server name fails as well
    let mut s = connect(&fx.client_config(), srv.addr(), "example.org");
    let _ = s.write_all(b"GET /healthcheck HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(s.read_to_end(&mut out).is_err());
}

#[test]
fn plain_http_to_a_tls_port_gets_no_plaintext_answer() {
    let fx = Fixture::new();
    let srv = fx.start(|_| {});
    let mut tcp = TcpStream::connect(srv.addr()).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    tcp.write_all(b"GET /healthcheck HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    let mut out = Vec::new();
    let _ = tcp.read_to_end(&mut out);
    assert!(
        !String::from_utf8_lossy(&out).contains("200 OK"),
        "served plaintext over a TLS port: {out:?}"
    );
    drop(tcp);
    // the server is fine afterwards
    let mut s = connect(&fx.client_config(), srv.addr(), "localhost");
    s.write_all(b"GET /healthcheck HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .unwrap();
    assert_eq!(read_response(&mut s).0, 200);
}

#[test]
fn a_stalled_handshake_is_dropped_and_frees_its_slot() {
    let fx = Fixture::new();
    let srv = fx.start(|_| {});
    let silent = TcpStream::connect(srv.addr()).unwrap();
    assert!(common::eventually(Duration::from_secs(2), || srv
        .open_connections()
        == 1));
    assert!(
        common::eventually(Duration::from_secs(4), || srv.open_connections() == 0),
        "handshake deadline"
    );
    drop(silent);
}

#[test]
fn https_idle_and_slow_head_are_closed() {
    let fx = Fixture::new();
    let srv = fx.start(|_| {});
    let mut s = connect(&fx.client_config(), srv.addr(), "localhost");
    s.write_all(b"GET /healthcheck HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    assert_eq!(read_response(&mut s).0, 200);
    // idle past idle_timeout: the server closes (EOF or close_notify)
    let mut b = [0u8; 16];
    let n = s.read(&mut b).unwrap_or(0);
    assert_eq!(n, 0);
    drop(s);

    let mut s = connect(&fx.client_config(), srv.addr(), "localhost");
    s.write_all(b"GET /opds HTTP/1.1\r\nX-Slow: ").unwrap();
    let n = s.read(&mut b).unwrap_or(0);
    assert_eq!(n, 0, "closed without plaintext inside the TLS stream");
    assert!(common::eventually(Duration::from_secs(3), || srv
        .open_connections()
        == 0));
}

#[test]
fn https_shutdown_closes_open_connections_promptly() {
    let fx = Fixture::new();
    let srv = fx.start(|c| {
        c.limits.idle_timeout = Duration::from_secs(60);
        c.limits.header_timeout = Duration::from_secs(60);
    });
    let addr = srv.addr();
    let mut s = connect(&fx.client_config(), addr, "localhost");
    s.write_all(b"GET /healthcheck HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    assert_eq!(read_response(&mut s).0, 200);
    let started = std::time::Instant::now();
    srv.shutdown();
    assert!(started.elapsed() < Duration::from_secs(3));
    let mut b = [0u8; 8];
    assert_eq!(s.read(&mut b).unwrap_or(0), 0);
    assert!(TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_err());
}

#[test]
fn http_stays_the_default() {
    let mut cfg = ServerConfig::new(Arc::new(LibrarySnapshot::default));
    assert!(cfg.tls.is_none());
    cfg.limits.header_timeout = Duration::from_millis(500);
    let srv = Server::start(cfg).unwrap();
    assert!(srv.base_url().starts_with("http://"));
}
