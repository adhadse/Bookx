//! A small blocking HTTP/1.1 client for WebDAV: any method, a request body, `Connection: close`,
//! optional TLS (rustls with the Mozilla roots), bounded answers and real time-outs.
//!
//! Why not `ureq`: it only knows the standard methods and refuses `PROPFIND` and `MKCOL`.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};

/// What came back.
#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Longest answer body read.
pub const MAX_BODY: usize = 8 * 1024 * 1024;
const MAX_HEAD: usize = 64 * 1024;

enum Conn {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Read for Conn {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Conn::Plain(s) => s.read(b),
            Conn::Tls(s) => s.read(b),
        }
    }
}
impl Write for Conn {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        match self {
            Conn::Plain(s) => s.write(b),
            Conn::Tls(s) => s.write(b),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Conn::Plain(s) => s.flush(),
            Conn::Tls(s) => s.flush(),
        }
    }
}

fn tls_config(trust_only: Option<&[u8]>) -> Result<Arc<rustls::ClientConfig>> {
    let mut roots = rustls::RootCertStore::empty();
    match trust_only {
        Some(pem) => {
            use rustls::pki_types::pem::PemObject;
            for c in rustls::pki_types::CertificateDer::pem_slice_iter(pem) {
                roots.add(c.context("reading the certificate")?)?;
            }
        }
        None => roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned()),
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Ok(Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .context("selecting TLS versions")?
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}

/// Sends one request and reads the whole answer. Redirects are not followed.
pub fn request(
    method: &str,
    url: &url::Url,
    headers: &[(&str, &str)],
    body: &[u8],
    timeout: Duration,
) -> Result<Reply> {
    request_trusting(method, url, headers, body, timeout, None)
}

/// Status of `GET url`, trusting only the PEM certificate in `trusted_pem` for HTTPS (a
/// self-signed one), or the Mozilla roots when `None`. Used to check a share's own certificate.
pub fn get_status(url: &str, trusted_pem: Option<&[u8]>) -> Result<u16> {
    let u = url::Url::parse(url).context("not a web address")?;
    request_trusting("GET", &u, &[], b"", Duration::from_secs(10), trusted_pem).map(|r| r.status)
}

/// [`request`] with the certificates to trust for HTTPS given explicitly.
pub fn request_trusting(
    method: &str,
    url: &url::Url,
    headers: &[(&str, &str)],
    body: &[u8],
    timeout: Duration,
    trusted_pem: Option<&[u8]>,
) -> Result<Reply> {
    request_limited(method, url, headers, body, timeout, trusted_pem, MAX_BODY)
}

/// [`request_trusting`] with a chosen limit for the answer body, for downloading backups.
pub fn request_limited(
    method: &str,
    url: &url::Url,
    headers: &[(&str, &str)],
    body: &[u8],
    timeout: Duration,
    trusted_pem: Option<&[u8]>,
    max_body: usize,
) -> Result<Reply> {
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("no host in the address"))?;
    let tls = url.scheme() == "https";
    let port = url
        .port_or_known_default()
        .unwrap_or(if tls { 443 } else { 80 });
    let deadline = Instant::now() + timeout;
    let addr = (host, port)
        .to_socket_addrs()
        .with_context(|| format!("looking up {host}"))?
        .next()
        .ok_or_else(|| anyhow!("{host} has no address"))?;
    let tcp = TcpStream::connect_timeout(&addr, timeout.min(Duration::from_secs(15)))
        .with_context(|| format!("connecting to {host}:{port}"))?;
    tcp.set_nodelay(true).ok();
    tcp.set_read_timeout(Some(timeout.min(Duration::from_secs(30))))?;
    tcp.set_write_timeout(Some(timeout.min(Duration::from_secs(30))))?;
    let mut conn = if tls {
        let name = rustls::pki_types::ServerName::try_from(host.to_string())
            .map_err(|_| anyhow!("{host} is not a valid server name"))?;
        let c = rustls::ClientConnection::new(tls_config(trusted_pem)?, name)?;
        Conn::Tls(Box::new(rustls::StreamOwned::new(c, tcp)))
    } else {
        Conn::Plain(tcp)
    };

    let mut target = url.path().to_string();
    if let Some(q) = url.query() {
        target.push('?');
        target.push_str(q);
    }
    let host_header = match url.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_string(),
    };
    let mut head = format!(
        "{method} {target} HTTP/1.1\r\nHost: {host_header}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (k, v) in headers {
        if k.contains(['\r', '\n', ':']) || v.contains(['\r', '\n']) {
            bail!("bad header");
        }
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    conn.write_all(head.as_bytes())?;
    conn.write_all(body)?;
    conn.flush()?;

    let mut buf = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > MAX_HEAD {
            bail!("the server's answer has too long a header");
        }
        if Instant::now() > deadline {
            bail!("the server took too long to answer");
        }
        match conn.read(&mut chunk) {
            Ok(0) => bail!("the server closed the connection without answering"),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) => return Err(anyhow!("reading the answer: {e}")),
        }
    };
    let head_text = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head_text.lines();
    let status: u16 = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| anyhow!("the server's answer is not HTTP"))?;
    let header = |name: &str| -> Option<String> {
        head_text.lines().skip(1).find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case(name)
                .then(|| v.trim().to_string())
        })
    };
    let chunked =
        header("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    let length: Option<usize> = header("content-length").and_then(|v| v.parse().ok());
    let mut raw = buf[head_end..].to_vec();
    let want_all = chunked || length.is_none();
    loop {
        let done = if chunked {
            decode_chunked(&raw).is_some()
        } else if let Some(n) = length {
            raw.len() >= n
        } else {
            false
        };
        if done {
            break;
        }
        if raw.len() > max_body + 64 * 1024 {
            bail!("the server's answer is too big");
        }
        if Instant::now() > deadline {
            bail!("the server took too long to answer");
        }
        match conn.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&chunk[..n]),
            // a TLS server closing without close_notify after a full answer
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(anyhow!("reading the answer: {e}")),
        }
    }
    let body = if chunked {
        decode_chunked(&raw).ok_or_else(|| anyhow!("the server's answer is cut short"))?
    } else if let Some(n) = length {
        if raw.len() < n && !want_all {
            bail!("the server's answer is cut short");
        }
        raw.truncate(n);
        raw
    } else {
        raw
    };
    if body.len() > max_body {
        bail!("the server's answer is too big");
    }
    Ok(Reply { status, body })
}

/// The body of a complete chunked message, `None` while it is incomplete or malformed.
fn decode_chunked(raw: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        let line_end = raw[i..].windows(2).position(|w| w == b"\r\n")? + i;
        let size_text = std::str::from_utf8(&raw[i..line_end]).ok()?;
        let size = usize::from_str_radix(size_text.split(';').next()?.trim(), 16).ok()?;
        i = line_end + 2;
        if size == 0 {
            return Some(out);
        }
        if raw.len() < i + size + 2 {
            return None;
        }
        out.extend_from_slice(&raw[i..i + size]);
        i += size + 2;
        if out.len() > MAX_BODY + 1 {
            return Some(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn serve_once(reply: Vec<u8>) -> url::Url {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let mut b = [0u8; 4096];
                let _ = s.read(&mut b);
                let _ = s.write_all(&reply);
            }
        });
        url::Url::parse(&format!("http://127.0.0.1:{port}/x")).unwrap()
    }

    #[test]
    fn any_method_with_a_length_body() {
        let u = serve_once(b"HTTP/1.1 207 Multi\r\nContent-Length: 5\r\n\r\nhello".to_vec());
        let r = request(
            "PROPFIND",
            &u,
            &[("Depth", "1")],
            b"<x/>",
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!((r.status, r.body.as_slice()), (207, &b"hello"[..]));
    }

    #[test]
    fn chunked_and_close_delimited_bodies_are_read() {
        let u = serve_once(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2;x=1\r\nde\r\n0\r\n\r\n".to_vec(),
        );
        let r = request("GET", &u, &[], b"", Duration::from_secs(5)).unwrap();
        assert_eq!(r.body, b"abcde");
        let u = serve_once(b"HTTP/1.1 200 OK\r\n\r\nuntil close".to_vec());
        let r = request("GET", &u, &[], b"", Duration::from_secs(5)).unwrap();
        assert_eq!(r.body, b"until close");
    }

    #[test]
    fn broken_answers_are_errors() {
        let u = serve_once(b"garbage\r\n\r\n".to_vec());
        assert!(request("GET", &u, &[], b"", Duration::from_secs(5)).is_err());
        let u = serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 50\r\n\r\nshort".to_vec());
        assert!(request("GET", &u, &[], b"", Duration::from_secs(5)).is_err());
        let u = serve_once(Vec::new());
        assert!(request("GET", &u, &[], b"", Duration::from_secs(5)).is_err());
        let u =
            serve_once(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nab".to_vec());
        assert!(request("GET", &u, &[], b"", Duration::from_secs(5)).is_err());
    }

    #[test]
    fn header_injection_is_refused() {
        let u = url::Url::parse("http://127.0.0.1:9/").unwrap();
        assert!(request(
            "GET",
            &u,
            &[("X", "a\r\nEvil: 1")],
            b"",
            Duration::from_secs(1)
        )
        .is_err());
    }

    #[test]
    fn https_refuses_a_self_signed_certificate() {
        let dir = std::env::temp_dir().join(format!("bookx-httpc-{}", uuid::Uuid::new_v4()));
        let (c, k) = crate::cert::write_self_signed(&dir, &[]).unwrap();
        let cfg = crate::net::tls_config(&c, &k).unwrap();
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((tcp, _)) = l.accept() {
                let conn = rustls::ServerConnection::new(cfg).unwrap();
                let mut s = rustls::StreamOwned::new(conn, tcp);
                let mut b = [0u8; 64];
                let _ = s.read(&mut b);
            }
        });
        let u = url::Url::parse(&format!("https://localhost:{port}/")).unwrap();
        let e = request("GET", &u, &[], b"", Duration::from_secs(5)).unwrap_err();
        let text = format!("{e:#}").to_lowercase();
        assert!(
            text.contains("cert") || text.contains("unknown") || text.contains("reading"),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn chunk_decoder_waits_for_the_end() {
        assert_eq!(decode_chunked(b"1\r\na\r\n0\r\n\r\n"), Some(b"a".to_vec()));
        assert_eq!(decode_chunked(b"1\r\na\r\n"), None);
        assert_eq!(decode_chunked(b"zz\r\n"), None);
    }
}
