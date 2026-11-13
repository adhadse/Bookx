//! `bookx-cli serve --tls-cert/--tls-key` as a separate process, talked to by a real rustls client
//! over loopback: the certificate is checked, the answer arrives over TLS, and a client that does
//! not trust the certificate (or speaks plain HTTP) gets nothing.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustls::pki_types::{CertificateDer, ServerName};

struct Server {
    child: Child,
    addr: String,
    dir: PathBuf,
    cert: CertificateDer<'static>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn start() -> Server {
    let dir = std::env::temp_dir().join(format!("bookx-cli-https-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let pair = bookx_serve::cert::generate_self_signed(&["localhost".into()]).unwrap();
    std::fs::write(dir.join("c.pem"), &pair.cert_pem).unwrap();
    std::fs::write(dir.join("k.pem"), &pair.key_pem).unwrap();
    let cert = rustls::pki_types::pem::PemObject::pem_slice_iter(pair.cert_pem.as_bytes())
        .next()
        .unwrap()
        .unwrap();
    let lib = dir.join("lib");
    let bin = env!("CARGO_BIN_EXE_bookx-cli");
    let md = bookx_core::opf::Metadata {
        title: "Served Over TLS".into(),
        authors: vec!["Ann".into()],
        language: "en".into(),
        ..Default::default()
    };
    let ch = bookx_core::convert::Chapter {
        title: "One".into(),
        body: "<p>Text.</p>".into(),
    };
    let epub = dir.join("book.epub");
    std::fs::write(
        &epub,
        bookx_core::convert::create_epub(&md, &[ch], None)
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    let add = Command::new(bin)
        .args(["--library", lib.to_str().unwrap(), "library", "add"])
        .arg(&epub)
        .output()
        .unwrap();
    assert!(add.status.success(), "{add:?}");
    let mut child = Command::new(bin)
        .args([
            "--library",
            lib.to_str().unwrap(),
            "serve",
            "--port",
            "0",
            "--tls-cert",
            dir.join("c.pem").to_str().unwrap(),
            "--tls-key",
            dir.join("k.pem").to_str().unwrap(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let t = Instant::now();
    while line.is_empty() {
        out.read_line(&mut line).unwrap();
        assert!(t.elapsed() < Duration::from_secs(20), "no start line");
        if line.is_empty() {
            if let Some(st) = child.try_wait().unwrap() {
                let mut err = String::new();
                child.stderr.take().unwrap().read_to_string(&mut err).ok();
                panic!("serve exited with {st}: {err}");
            }
        }
    }
    let addr = line
        .trim()
        .strip_prefix("serving on https://")
        .unwrap_or_else(|| panic!("unexpected start line {line:?}"))
        .to_string();
    Server {
        child,
        addr,
        dir,
        cert,
    }
}

fn client(trusting: Option<&CertificateDer<'static>>) -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    if let Some(c) = trusting {
        roots.add(c.clone()).unwrap();
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

fn get(
    cfg: Arc<rustls::ClientConfig>,
    addr: &str,
    name: &str,
    path: &str,
) -> std::io::Result<String> {
    let tcp = TcpStream::connect(addr)?;
    tcp.set_read_timeout(Some(Duration::from_secs(8)))?;
    let conn = rustls::ClientConnection::new(cfg, ServerName::try_from(name.to_string()).unwrap())
        .map_err(std::io::Error::other)?;
    let mut s = rustls::StreamOwned::new(conn, tcp);
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: {name}\r\nConnection: close\r\n\r\n"
    )?;
    let mut buf = Vec::new();
    match s.read_to_end(&mut buf) {
        Ok(_) => {}
        // the server closes without close_notify after `Connection: close`
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof && !buf.is_empty() => {}
        Err(e) => return Err(e),
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

#[test]
fn a_trusting_client_gets_the_catalogue_over_tls() {
    let s = start();
    let body = get(client(Some(&s.cert)), &s.addr, "localhost", "/opds").unwrap();
    assert!(body.starts_with("HTTP/1.1 200"), "{body}");
    assert!(body.contains("<feed"), "{body}");
    let page = get(client(Some(&s.cert)), &s.addr, "localhost", "/").unwrap();
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    assert!(page.contains("Served Over TLS"), "{page}");
    let missing = get(client(Some(&s.cert)), &s.addr, "localhost", "/nothing").unwrap();
    assert!(missing.starts_with("HTTP/1.1 404"), "{missing}");
}

#[test]
fn a_client_that_does_not_trust_the_certificate_is_refused() {
    let s = start();
    let e = get(client(None), &s.addr, "localhost", "/opds").unwrap_err();
    assert!(
        e.to_string().to_lowercase().contains("cert"),
        "expected a certificate error, got {e}"
    );
}

#[test]
fn the_certificate_must_match_the_host_name() {
    let s = start();
    let e = get(client(Some(&s.cert)), &s.addr, "books.example", "/opds").unwrap_err();
    assert!(e.to_string().to_lowercase().contains("cert"), "{e}");
}

#[test]
fn plain_http_to_the_https_port_gets_no_catalogue() {
    let s = start();
    let mut tcp = TcpStream::connect(&s.addr).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let _ = write!(
        tcp,
        "GET /opds HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
    );
    let mut buf = Vec::new();
    let _ = tcp.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    assert!(
        !text.contains("<feed") && !text.contains("200 OK"),
        "{text}"
    );
}
