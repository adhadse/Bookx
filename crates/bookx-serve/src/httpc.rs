//! A small blocking HTTP/1.1 client for WebDAV: any method, a request body, `Connection: close`,
//! optional TLS (rustls with the Mozilla roots), bounded answers and real time-outs.
//!
//! Why not `ureq`: it only knows the standard methods and refuses `PROPFIND` and `MKCOL`.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

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
    let r = request_stream(
        method,
        url,
        headers,
        Body::Bytes(body),
        timeout,
        trusted_pem,
        max_body,
        None,
    )?;
    Ok(Reply {
        status: r.status,
        body: r.body,
    })
}

/// The request body of [`request_stream`].
pub enum Body<'a> {
    Bytes(&'a [u8]),
    /// Exactly this many bytes are read from the reader and sent; fewer is an error.
    Stream(&'a mut dyn Read, u64),
}

/// An answer whose body may have gone to a writer instead of memory.
#[derive(Debug)]
pub struct StreamReply {
    pub status: u16,
    /// The body, unless it went to the sink.
    pub body: Vec<u8>,
    /// Bytes written to the sink.
    pub streamed: u64,
}

/// Where a `200` answer body goes, and how much of it is accepted.
pub struct Sink<'a> {
    pub out: &'a mut dyn Write,
    pub max: u64,
}

fn connect(url: &url::Url, timeout: Duration, trusted_pem: Option<&[u8]>) -> Result<Conn> {
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("no host in the address"))?;
    let tls = url.scheme() == "https";
    let port = url
        .port_or_known_default()
        .unwrap_or(if tls { 443 } else { 80 });
    let addrs: Vec<_> = (host, port)
        .to_socket_addrs()
        .with_context(|| format!("looking up {host}"))?
        .collect();
    if addrs.is_empty() {
        bail!("{host} has no address");
    }
    // every address in turn: "localhost" may resolve to ::1 first while the server listens on IPv4 only
    let mut last = None;
    let mut found = None;
    for addr in &addrs {
        match TcpStream::connect_timeout(addr, timeout.min(Duration::from_secs(15))) {
            Ok(t) => {
                found = Some(t);
                break;
            }
            Err(e) => last = Some(e),
        }
    }
    let tcp = match (found, last) {
        (Some(t), _) => t,
        (None, e) => {
            return Err(anyhow::Error::from(e.expect("one address was tried")))
                .with_context(|| format!("connecting to {host}:{port}"));
        }
    };
    tcp.set_nodelay(true).ok();
    tcp.set_read_timeout(Some(timeout.min(Duration::from_secs(30))))?;
    tcp.set_write_timeout(Some(timeout.min(Duration::from_secs(30))))?;
    Ok(if tls {
        let name = rustls::pki_types::ServerName::try_from(host.to_string())
            .map_err(|_| anyhow!("{host} is not a valid server name"))?;
        let c = rustls::ClientConnection::new(tls_config(trusted_pem)?, name)?;
        Conn::Tls(Box::new(rustls::StreamOwned::new(c, tcp)))
    } else {
        Conn::Plain(tcp)
    })
}

/// Incremental decoder of a chunked body.
#[derive(Default)]
struct ChunkedDecoder {
    pending: Vec<u8>,
    remaining: usize,
    state: ChunkState,
}

#[derive(Default, PartialEq, Eq, Clone, Copy)]
enum ChunkState {
    #[default]
    Size,
    Data,
    DataEnd,
    Trailer,
    Done,
}

impl ChunkedDecoder {
    fn done(&self) -> bool {
        self.state == ChunkState::Done
    }

    /// Feeds raw bytes; decoded data goes to `out`.
    fn feed(&mut self, data: &[u8], out: &mut dyn FnMut(&[u8]) -> Result<()>) -> Result<()> {
        self.pending.extend_from_slice(data);
        loop {
            match self.state {
                ChunkState::Done => return Ok(()),
                ChunkState::Size | ChunkState::Trailer => {
                    let Some(i) = self.pending.windows(2).position(|w| w == b"\r\n") else {
                        if self.pending.len() > 4096 {
                            bail!("the server's answer is malformed");
                        }
                        return Ok(());
                    };
                    let line: Vec<u8> = self.pending.drain(..i + 2).take(i).collect();
                    if self.state == ChunkState::Trailer {
                        if line.is_empty() {
                            self.state = ChunkState::Done;
                        }
                        continue;
                    }
                    let text = std::str::from_utf8(&line)
                        .map_err(|_| anyhow!("the server's answer is malformed"))?;
                    let size =
                        usize::from_str_radix(text.split(';').next().unwrap_or("").trim(), 16)
                            .map_err(|_| anyhow!("the server's answer is malformed"))?;
                    if size == 0 {
                        self.state = ChunkState::Trailer;
                    } else {
                        self.remaining = size;
                        self.state = ChunkState::Data;
                    }
                }
                ChunkState::Data => {
                    if self.pending.is_empty() {
                        return Ok(());
                    }
                    let n = self.remaining.min(self.pending.len());
                    let part: Vec<u8> = self.pending.drain(..n).collect();
                    out(&part)?;
                    self.remaining -= n;
                    if self.remaining == 0 {
                        self.state = ChunkState::DataEnd;
                    }
                }
                ChunkState::DataEnd => {
                    if self.pending.len() < 2 {
                        return Ok(());
                    }
                    if &self.pending[..2] != b"\r\n" {
                        bail!("the server's answer is malformed");
                    }
                    self.pending.drain(..2);
                    self.state = ChunkState::Size;
                }
            }
        }
    }
}

/// The general request: a body from memory or from a reader, and an answer that is kept in memory
/// (up to `max_body`) or, for a `200`, written to `sink` as it arrives. Redirects are not
/// followed. A reader that fails (a cancelled upload) makes the request fail with its error.
#[allow(clippy::too_many_arguments)]
pub fn request_stream(
    method: &str,
    url: &url::Url,
    headers: &[(&str, &str)],
    body: Body,
    timeout: Duration,
    trusted_pem: Option<&[u8]>,
    max_body: usize,
    mut sink: Option<Sink>,
) -> Result<StreamReply> {
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("no host in the address"))?;
    let deadline = Instant::now() + timeout;
    let mut conn = connect(url, timeout, trusted_pem)?;

    let mut target = url.path().to_string();
    if let Some(q) = url.query() {
        target.push('?');
        target.push_str(q);
    }
    let host_header = match url.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_string(),
    };
    let body_len = match &body {
        Body::Bytes(b) => b.len() as u64,
        Body::Stream(_, n) => *n,
    };
    let mut head = format!(
        "{method} {target} HTTP/1.1\r\nHost: {host_header}\r\nConnection: close\r\nContent-Length: {body_len}\r\n"
    );
    for (k, v) in headers {
        if k.contains(['\r', '\n', ':']) || v.contains(['\r', '\n']) {
            bail!("bad header");
        }
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    conn.write_all(head.as_bytes())?;
    let mut send_error: Option<anyhow::Error> = None;
    match body {
        Body::Bytes(b) => conn.write_all(b)?,
        Body::Stream(r, n) => {
            let mut left = n;
            let mut buf = vec![0u8; 64 * 1024];
            while left > 0 {
                let want = buf.len().min(left as usize);
                let got = match r.read(&mut buf[..want]) {
                    Ok(0) => bail!("the data to send ended early"),
                    Ok(g) => g,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(anyhow::Error::new(e)),
                };
                if let Err(e) = conn.write_all(&buf[..got]) {
                    // the server may have refused early and say why
                    send_error = Some(anyhow!("sending: {e}"));
                    break;
                }
                left -= got as u64;
            }
        }
    }
    if send_error.is_none() {
        conn.flush()?;
    }

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
            Ok(0) => {
                return Err(send_error.unwrap_or_else(|| {
                    anyhow!("the server closed the connection without answering")
                }));
            }
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) => return Err(send_error.unwrap_or_else(|| anyhow!("reading the answer: {e}"))),
        }
    };
    let head_text = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head_text.lines();
    let status: u16 = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| anyhow!("the server's answer is not HTTP"))?;
    if let Some(e) = send_error {
        if status < 400 {
            return Err(e);
        }
    }
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
    let leftover = buf[head_end..].to_vec();

    if status == 200 && sink.is_some() {
        let Sink { out, max } = sink.take().unwrap();
        let mut streamed: u64 = 0;
        let mut put = |data: &[u8]| -> Result<()> {
            streamed += data.len() as u64;
            if streamed > max {
                bail!("the server's answer is too big");
            }
            out.write_all(data).map_err(anyhow::Error::new)
        };
        if chunked {
            let mut dec = ChunkedDecoder::default();
            dec.feed(&leftover, &mut put)?;
            while !dec.done() {
                if Instant::now() > deadline {
                    bail!("the server took too long to answer");
                }
                match conn.read(&mut chunk) {
                    Ok(0) => bail!("the server's answer is cut short"),
                    Ok(n) => dec.feed(&chunk[..n], &mut put)?,
                    Err(e) => return Err(anyhow!("reading the answer: {e}")),
                }
            }
        } else {
            let mut got = 0usize;
            if !leftover.is_empty() {
                got += leftover.len();
                put(&leftover)?;
            }
            loop {
                if length.is_some_and(|n| got >= n) {
                    break;
                }
                if Instant::now() > deadline {
                    bail!("the server took too long to answer");
                }
                match conn.read(&mut chunk) {
                    Ok(0) => {
                        if length.is_some() {
                            bail!("the server's answer is cut short");
                        }
                        break;
                    }
                    Ok(n) => {
                        got += n;
                        put(&chunk[..n])?;
                    }
                    Err(e) if length.is_none() && e.kind() == std::io::ErrorKind::UnexpectedEof => {
                        break;
                    }
                    Err(e) => return Err(anyhow!("reading the answer: {e}")),
                }
            }
        }
        return Ok(StreamReply {
            status,
            body: Vec::new(),
            streamed,
        });
    }

    let mut raw = leftover;
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
    Ok(StreamReply {
        status,
        body,
        streamed: 0,
    })
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

    fn feed_all(parts: &[&[u8]]) -> Result<Vec<u8>> {
        let mut d = ChunkedDecoder::default();
        let mut got = Vec::new();
        for p in parts {
            d.feed(p, &mut |b| {
                got.extend_from_slice(b);
                Ok(())
            })?;
        }
        Ok(got)
    }

    #[test]
    fn the_chunk_decoder_survives_any_split_of_the_bytes() {
        let wire: &[u8] =
            b"3\r\nabc\r\n2;ext=1\r\nde\r\nA\r\n0123456789\r\n0\r\nTrailer: x\r\n\r\n";
        let want = b"abcde0123456789".to_vec();
        assert_eq!(feed_all(&[wire]).unwrap(), want);
        // one byte at a time, as a slow connection delivers it
        let bytes: Vec<&[u8]> = wire.chunks(1).collect();
        let mut d = ChunkedDecoder::default();
        let mut got = Vec::new();
        for b in &bytes {
            d.feed(b, &mut |x| {
                got.extend_from_slice(x);
                Ok(())
            })
            .unwrap();
        }
        assert!(d.done());
        assert_eq!(got, want);
        // bytes after the end are ignored
        d.feed(b"junk", &mut |_| panic!("no data after the end"))
            .unwrap();
    }

    #[test]
    fn the_chunk_decoder_refuses_hostile_framing() {
        for bad in [
            &b"zz\r\nabc\r\n"[..],       // size is not hex
            &b"\xff\xfe\r\n"[..],        // not text
            &b"3\r\nabcXY0\r\n\r\n"[..], // data is not followed by CRLF
        ] {
            let e = feed_all(&[bad]).unwrap_err().to_string();
            assert!(e.contains("malformed"), "{bad:?}: {e}");
        }
        // a size line that never ends is cut off instead of buffered for ever
        let long = vec![b'1'; 5000];
        assert!(
            feed_all(&[&long])
                .unwrap_err()
                .to_string()
                .contains("malformed")
        );
        // an error from the receiver (the disk is full) stops the decoding
        let mut d = ChunkedDecoder::default();
        let e = d
            .feed(b"1\r\na\r\n", &mut |_| Err(anyhow!("disk full")))
            .unwrap_err();
        assert_eq!(e.to_string(), "disk full");
    }

    #[test]
    fn a_streamed_download_goes_to_the_sink_and_is_capped() {
        let u = serve_once(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n"
                .to_vec(),
        );
        let mut out: Vec<u8> = Vec::new();
        let r = request_stream(
            "GET",
            &u,
            &[],
            Body::Bytes(b""),
            Duration::from_secs(5),
            None,
            1024,
            Some(Sink {
                out: &mut out,
                max: 100,
            }),
        )
        .unwrap();
        assert_eq!((r.status, r.streamed), (200, 11));
        assert!(r.body.is_empty());
        assert_eq!(out, b"hello world");
        // a length-delimited body over the cap is refused, whatever the server claims
        let u = serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\nhello world".to_vec());
        let mut out: Vec<u8> = Vec::new();
        let e = request_stream(
            "GET",
            &u,
            &[],
            Body::Bytes(b""),
            Duration::from_secs(5),
            None,
            1024,
            Some(Sink {
                out: &mut out,
                max: 4,
            }),
        )
        .unwrap_err();
        assert!(e.to_string().contains("too big"), "{e}");
        // a chunked body that stops early is cut short, not silently accepted
        let u =
            serve_once(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhel".to_vec());
        let mut out: Vec<u8> = Vec::new();
        let e = request_stream(
            "GET",
            &u,
            &[],
            Body::Bytes(b""),
            Duration::from_secs(5),
            None,
            1024,
            Some(Sink {
                out: &mut out,
                max: 100,
            }),
        )
        .unwrap_err();
        assert!(e.to_string().contains("cut short"), "{e}");
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
        assert!(
            request(
                "GET",
                &u,
                &[("X", "a\r\nEvil: 1")],
                b"",
                Duration::from_secs(1)
            )
            .is_err()
        );
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
