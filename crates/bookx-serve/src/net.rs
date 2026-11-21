//! A defensive, blocking-handler HTTP/1.1 server on [`hyper`] (optionally over TLS).
//!
//! The transport knows nothing about books: hyper parses requests and frames bodies, this module
//! enforces [`Limits`], calls a synchronous [`Handler`] and writes the [`Response`]. The server owns
//! a small tokio runtime on its own threads; handlers run on the blocking pool, at most
//! [`Limits::handler_threads`] at a time, so idle keep-alive connections never starve busy ones.
//!
//! What is ours, what is hyper's:
//! * hyper (httparse) rejects malformed request lines and headers (`400`), too many headers or too
//!   large a head (`431`, [`Limits::max_headers`], [`Limits::max_header_bytes`]), bad or conflicting
//!   `Content-Length`, whitespace before a header colon and obsolete line folding. It frames bodies
//!   by `Content-Length`, answers `Expect: 100-continue` when the body is first read, and keeps
//!   connections alive.
//! * a small `Guard` between the socket and hyper adds what hyper lacks: a deadline for the
//!   request head ([`Limits::header_timeout`], answered `408`), an idle timeout between keep-alive
//!   requests ([`Limits::idle_timeout`]), a write deadline with a minimum transfer rate
//!   ([`Limits::write_timeout`], [`Limits::write_min_rate`]; a stalled reader is dropped) and a
//!   check that rejects bare-LF line endings in a request head.
//! * the service adds the rest: methods must be upper case, targets are normalised (origin-form or
//!   `http://` absolute-form only), request line length (`414`), any `Transfer-Encoding` is refused
//!   (`501`; hyper drops a `Content-Length` that comes with one), unknown `Expect` (`417`), body
//!   size (`413`, before anything is read) and body deadline (`408`, [`Limits::body_timeout`]),
//!   [`Limits::max_requests_per_connection`], HTTP/1.0 always closes, `nosniff` on every response.
//! * connection admission ([`Limits::max_connections`], [`Limits::max_connections_per_ip`]) happens in
//!   the accept loop; excess peers get `503` and are closed.
//! * failed credentials are throttled per peer address ([`AuthThrottle`], pure, injected clock).
//!
//! Known gaps: the bare-LF check covers the head of every request that starts a read burst (all
//! non-pipelined requests), not heads pipelined behind another request in the same TCP segment;
//! an endless request line without any newline is cut by the header deadline or at
//! [`Limits::max_header_bytes`] rather than at [`Limits::max_request_line`]; hyper answers its own
//! parse errors with an empty body.
//!
//! TLS is optional: [`tls_config`] builds a rustls (ring) server configuration from PEM files and
//! [`Running::start_tls`] serves HTTPS with it; [`Running::start`] stays plain HTTP.

use std::collections::HashMap;
use std::convert::Infallible;
use std::future::Future;
use std::io;
use std::net::{IpAddr, Shutdown, SocketAddr};
use std::os::fd::AsFd;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use anyhow::{bail, Context as _, Result};
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::header::{HeaderName, HeaderValue};
use hyper::service::service_fn;
use hyper::{StatusCode, Version};
use hyper_util::rt::TokioIo;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::sync::{watch, Semaphore};
use tokio::time::Sleep;
use tokio_rustls::TlsAcceptor;

use crate::http::{Request, Response};

/// Everything the transport limits. All fields are public so callers (and tests) can tighten them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Longest request line (`GET /x HTTP/1.1`) in bytes, else `414`.
    pub max_request_line: usize,
    /// Largest request head (request line + all headers + final blank line), else `431`.
    /// hyper never buffers less than 8 KiB, so smaller values behave like 8192.
    pub max_header_bytes: usize,
    /// Most header fields, else `431`.
    pub max_headers: usize,
    /// Largest accepted request body (`Content-Length`), else `413`.
    pub max_body: usize,
    /// Total time for a request head to arrive once its first byte has been seen (and for the
    /// first byte of a connection's first request).
    pub header_timeout: Duration,
    /// Total time for the request body.
    pub body_timeout: Duration,
    /// How long an idle keep-alive connection may wait for the next request.
    pub idle_timeout: Duration,
    /// Base time allowed for writing a response, plus `bytes written / write_min_rate`.
    pub write_timeout: Duration,
    /// Slowest transfer rate (bytes per second) a client may drain a response at.
    pub write_min_rate: usize,
    /// Most simultaneous connections; the next one gets `503`.
    pub max_connections: usize,
    /// Most simultaneous connections from one peer address.
    pub max_connections_per_ip: usize,
    /// Requests served on one connection before it is closed.
    pub max_requests_per_connection: usize,
    /// Handlers running at the same time (the rest wait).
    pub handler_threads: usize,
    /// Failed-credential attempts per [`auth_window`](Self::auth_window) before a peer is
    /// blocked; `0` switches the throttle off.
    pub auth_max_failures: u32,
    pub auth_window: Duration,
    /// How long a peer is answered with `429` after too many failures.
    pub auth_block: Duration,
    /// Reverse proxies whose `X-Forwarded-For` is believed, so the throttle counts the real
    /// client instead of the proxy. Empty (the default) trusts no header at all.
    pub trusted_proxies: Vec<ProxyNet>,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_request_line: 8 * 1024,
            max_header_bytes: 16 * 1024,
            max_headers: 100,
            max_body: 1024 * 1024,
            header_timeout: Duration::from_secs(10),
            body_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(15),
            write_timeout: Duration::from_secs(30),
            write_min_rate: 128 * 1024,
            max_connections: 64,
            max_connections_per_ip: 32,
            max_requests_per_connection: 1000,
            handler_threads: 8,
            auth_max_failures: 10,
            auth_window: Duration::from_secs(60),
            auth_block: Duration::from_secs(60),
            trusted_proxies: Vec::new(),
        }
    }
}

// ------------------------------------------------------------------------------ trusted proxies

/// An address or a network (`10.0.0.0/8`) of a reverse proxy the user trusts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProxyNet {
    addr: IpAddr,
    bits: u8,
}

impl ProxyNet {
    /// Parses `192.0.2.1`, `10.0.0.0/8`, `::1` or `fd00::/8`. A prefix of `/0` is refused: it would
    /// trust every address on the Internet.
    pub fn parse(s: &str) -> Option<ProxyNet> {
        let s = s.trim();
        let (a, len) = match s.split_once('/') {
            Some((a, l)) => (a, Some(l.parse::<u8>().ok()?)),
            None => (s, None),
        };
        let addr: IpAddr = a.parse().ok()?;
        let addr = addr.to_canonical();
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let bits = len.unwrap_or(max);
        if bits == 0 || bits > max {
            return None;
        }
        Some(ProxyNet { addr, bits })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        let ip = ip.to_canonical();
        match (self.addr, ip) {
            (IpAddr::V4(a), IpAddr::V4(b)) => {
                let m = u32::MAX << (32 - u32::from(self.bits));
                u32::from(a) & m == u32::from(b) & m
            }
            (IpAddr::V6(a), IpAddr::V6(b)) => {
                let m = u128::MAX << (128 - u32::from(self.bits));
                u128::from(a) & m == u128::from(b) & m
            }
            _ => false,
        }
    }
}

/// Parses a list of proxies separated by commas, spaces or semicolons. `Err` names the first bad item.
pub fn parse_proxy_list(s: &str) -> Result<Vec<ProxyNet>, String> {
    s.split([',', ' ', ';', '\n'])
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(|x| ProxyNet::parse(x).ok_or_else(|| x.to_string()))
        .collect()
}

/// The address a request really came from. Only when the socket peer is a trusted proxy is
/// `X-Forwarded-For` read, from the right: each hop that is itself a trusted proxy is skipped and
/// the first other address is the client (what the nearest proxy saw). Everything left of it is
/// client-controlled and ignored. A malformed entry stops the walk at the peer's side.
pub fn client_ip(peer: IpAddr, forwarded_for: &[&str], trusted: &[ProxyNet]) -> IpAddr {
    let peer = peer.to_canonical();
    let is_trusted = |ip: IpAddr| trusted.iter().any(|n| n.contains(ip));
    if !is_trusted(peer) {
        return peer;
    }
    let hops: Vec<&str> = forwarded_for
        .iter()
        .flat_map(|h| h.split(','))
        .map(str::trim)
        .collect();
    let mut best = peer;
    for h in hops.iter().rev() {
        match h.parse::<IpAddr>() {
            Ok(ip) => {
                best = ip.to_canonical();
                if !is_trusted(best) {
                    return best;
                }
            }
            Err(_) => return best,
        }
    }
    best
}

/// The application behind the transport.
pub type Handler = Arc<dyn Fn(&Request) -> Response + Send + Sync>;

// ------------------------------------------------------------------------------ auth throttle

const MAX_TRACKED_PEERS: usize = 4096;

#[derive(Debug, Clone)]
struct Entry {
    window_start: Instant,
    failures: u32,
    blocked_until: Option<Instant>,
}

/// Per-address failed-authentication throttle. Pure: the caller passes the current time.
///
/// `max_failures` failures within `window` block the address for `block`; failures while blocked
/// do not extend the block; a success does not reset anything (so a valid credential cannot be
/// used to launder guesses). `max_failures == 0` disables the throttle.
#[derive(Debug, Clone)]
pub struct AuthThrottle {
    max_failures: u32,
    window: Duration,
    block: Duration,
    entries: HashMap<IpAddr, Entry>,
}

impl AuthThrottle {
    pub fn new(max_failures: u32, window: Duration, block: Duration) -> AuthThrottle {
        AuthThrottle {
            max_failures,
            window,
            block,
            entries: HashMap::new(),
        }
    }

    /// `Some(time left)` while `ip` is blocked.
    pub fn check(&self, ip: IpAddr, now: Instant) -> Option<Duration> {
        if self.max_failures == 0 {
            return None;
        }
        let until = self.entries.get(&ip)?.blocked_until?;
        (until > now).then(|| until - now)
    }

    pub fn record_failure(&mut self, ip: IpAddr, now: Instant) {
        if self.max_failures == 0 {
            return;
        }
        if self.entries.len() >= MAX_TRACKED_PEERS && !self.entries.contains_key(&ip) {
            self.prune(now);
            if self.entries.len() >= MAX_TRACKED_PEERS {
                return;
            }
        }
        let (max, window, block) = (self.max_failures, self.window, self.block);
        let e = self.entries.entry(ip).or_insert(Entry {
            window_start: now,
            failures: 0,
            blocked_until: None,
        });
        if e.blocked_until.is_some_and(|u| u > now) {
            return;
        }
        if now.saturating_duration_since(e.window_start) >= window {
            e.window_start = now;
            e.failures = 0;
        }
        e.failures += 1;
        if e.failures >= max {
            e.blocked_until = Some(now + block);
            e.failures = 0;
            e.window_start = now;
        }
    }

    /// Forgets addresses that are neither blocked nor inside a counting window.
    pub fn prune(&mut self, now: Instant) {
        let window = self.window;
        self.entries.retain(|_, e| {
            e.blocked_until.is_some_and(|u| u > now)
                || now.saturating_duration_since(e.window_start) < window
        });
    }

    /// Number of addresses currently tracked.
    pub fn tracked(&self) -> usize {
        self.entries.len()
    }
}

// ------------------------------------------------------------------------ request validation

/// A transport-level error and the status to answer with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HttpError {
    status: u16,
    message: &'static str,
}

fn err(status: u16, message: &'static str) -> HttpError {
    HttpError { status, message }
}

/// Reduces a request target to origin-form (`/path?query`): `http://` absolute-form is accepted
/// and stripped of its authority, everything else (`*`, `https://`, authority-form, userinfo) is
/// refused.
fn normalize_target(t: &str) -> Result<String, HttpError> {
    if t.is_empty() || !t.bytes().all(|b| (0x21..0x7f).contains(&b)) {
        return Err(err(400, "Bad request target"));
    }
    if t.contains('#') {
        return Err(err(400, "Fragment in request target"));
    }
    if t.starts_with('/') {
        return Ok(t.to_string());
    }
    if t.len() < 7 || !t[..7].eq_ignore_ascii_case("http://") {
        return Err(err(400, "Unsupported request target"));
    }
    let rest = &t[7..];
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if authority.is_empty() || authority.contains('@') {
        return Err(err(400, "Bad authority in request target"));
    }
    Ok(if tail.is_empty() {
        "/".to_string()
    } else if tail.starts_with('?') {
        format!("/{tail}")
    } else {
        tail.to_string()
    })
}

fn valid_method(m: &str) -> bool {
    !m.is_empty() && m.len() <= 16 && m.bytes().all(|b| b.is_ascii_uppercase())
}

/// Checks everything about a request head that hyper does not, returning the normalised target.
fn check_head(parts: &hyper::http::request::Parts, lim: &Limits) -> Result<String, HttpError> {
    let method = parts.method.as_str();
    if !valid_method(method) {
        return Err(err(400, "Bad method"));
    }
    let raw = parts.uri.to_string();
    let line = method.len() + 1 + raw.len() + 1 + "HTTP/1.1".len();
    if line > lim.max_request_line {
        return Err(err(414, "Request line too long"));
    }
    let target = normalize_target(&raw)?;
    let headers = &parts.headers;
    if headers.contains_key(hyper::header::TRANSFER_ENCODING) {
        return Err(err(501, "Transfer-Encoding is not supported"));
    }
    for v in headers.get_all(hyper::header::EXPECT) {
        if !v.as_bytes().eq_ignore_ascii_case(b"100-continue") {
            return Err(err(417, "Expectation failed"));
        }
    }
    if let Some(v) = headers.get(hyper::header::CONTENT_LENGTH) {
        let n = v.to_str().ok().and_then(|s| s.parse::<u64>().ok());
        match n {
            Some(n) if n > lim.max_body as u64 => return Err(err(413, "Request body too large")),
            Some(_) => {}
            None => return Err(err(400, "Bad Content-Length")),
        }
    }
    Ok(target)
}

fn wants_close(headers: &hyper::HeaderMap) -> bool {
    headers
        .get_all(hyper::header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|v| v.split(',').any(|t| t.trim().eq_ignore_ascii_case("close")))
}

/// `close`: `Some(true)` asks for `Connection: close`, `Some(false)` for `keep-alive`, `None`
/// leaves the header to hyper (HTTP/1.0 peers, where hyper manages it itself).
fn to_hyper(resp: Response, head_only: bool, close: Option<bool>) -> hyper::Response<Full<Bytes>> {
    let status = StatusCode::from_u16(resp.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let bodyless = status.is_informational()
        || status == StatusCode::NO_CONTENT
        || status == StatusCode::NOT_MODIFIED;
    let len = resp.body.len();
    let body = if bodyless || head_only {
        Bytes::new()
    } else {
        Bytes::from(resp.body)
    };
    let mut out = hyper::Response::new(Full::new(body));
    *out.status_mut() = status;
    let headers = out.headers_mut();
    let mut nosniff = false;
    for (k, v) in &resp.headers {
        let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(k.as_bytes()),
            HeaderValue::from_str(v),
        ) else {
            continue;
        };
        if matches!(
            name.as_str(),
            "content-length" | "connection" | "transfer-encoding" | "date" | "keep-alive"
        ) {
            continue;
        }
        nosniff |= name == hyper::header::X_CONTENT_TYPE_OPTIONS;
        headers.append(name, value);
    }
    if !nosniff {
        headers.insert(
            hyper::header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
    }
    if !bodyless {
        headers.insert(hyper::header::CONTENT_LENGTH, HeaderValue::from(len));
    }
    if let Some(close) = close {
        headers.insert(
            hyper::header::CONNECTION,
            HeaderValue::from_static(if close { "close" } else { "keep-alive" }),
        );
    }
    out
}

// ------------------------------------------------------------------------------- the guard

/// What the service and the guard share about one connection.
#[derive(Default)]
struct Ctl {
    /// A request is being handled (set while the service future runs).
    serving: AtomicBool,
    served: AtomicUsize,
    /// We answered with an error or cut the peer off: drain what is still in flight before closing,
    /// so the peer sees our answer instead of a reset.
    linger: AtomicBool,
}

/// Where a connection is in its request cycle, as far as the guard can tell.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// Between requests (or before the first byte).
    Idle,
    /// Part of a request head has arrived.
    Head,
    /// A request is being handled or its response written.
    Request,
}

/// What [`HeadScan`] found wrong with the bytes of a request head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanError {
    /// A `\n` without a preceding `\r`.
    BareLf,
    /// The request line grew past [`Limits::max_request_line`] without ending.
    LineTooLong,
}

/// Watches the raw bytes of a request head until the blank line ends it: rejects bare LF line
/// endings and cuts off a request line that never ends.
#[derive(Debug)]
struct HeadScan {
    max_line: usize,
    line_len: usize,
    line_done: bool,
    started: bool,
    done: bool,
    prev: u8,
    state: u8,
}

impl HeadScan {
    fn new(max_line: usize) -> HeadScan {
        HeadScan {
            max_line,
            line_len: 0,
            line_done: false,
            started: false,
            done: false,
            prev: 0,
            state: 0,
        }
    }

    fn feed(&mut self, bytes: &[u8]) -> Result<(), ScanError> {
        for &b in bytes {
            if self.done {
                return Ok(());
            }
            if b == b'\n' && self.prev != b'\r' {
                return Err(ScanError::BareLf);
            }
            self.prev = b;
            // leading blank lines before the request line do not end a head
            self.started |= b != b'\r' && b != b'\n';
            if !self.started {
                continue;
            }
            if !self.line_done {
                if b == b'\r' || b == b'\n' {
                    self.line_done = true;
                } else {
                    self.line_len += 1;
                    if self.line_len > self.max_line {
                        return Err(ScanError::LineTooLong);
                    }
                }
            }
            self.state = match (self.state, b) {
                (0 | 1 | 3, b'\r') => 1,
                (1, b'\n') => 2,
                (2, b'\r') => 3,
                (3, b'\n') => {
                    self.done = true;
                    4
                }
                _ => 0,
            };
        }
        Ok(())
    }
}

fn raw_response(status: u16, reason: &str, body: &str, extra: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\nX-Content-Type-Options: nosniff\r\n{extra}Connection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// Sits between the socket and hyper: request-head and idle deadlines, write deadline with a
/// minimum rate, bare-LF check.
struct Guard<S> {
    inner: S,
    header_timeout: Duration,
    idle_timeout: Duration,
    write_timeout: Duration,
    write_min_rate: usize,
    ctl: Arc<Ctl>,
    phase: Phase,
    since: tokio::time::Instant,
    scan: HeadScan,
    max_request_line: usize,
    read_timer: Option<Pin<Box<Sleep>>>,
    write_start: Option<tokio::time::Instant>,
    write_bytes: usize,
    write_timer: Option<Pin<Box<Sleep>>>,
    /// A second handle on a plain TCP socket, used to put an error response on the wire when
    /// hyper is not going to (never set under TLS).
    notify: Option<std::net::TcpStream>,
}

impl<S> Guard<S> {
    fn new(inner: S, lim: &Limits, ctl: Arc<Ctl>, notify: Option<std::net::TcpStream>) -> Guard<S> {
        Guard {
            inner,
            header_timeout: lim.header_timeout,
            idle_timeout: lim.idle_timeout,
            write_timeout: lim.write_timeout,
            write_min_rate: lim.write_min_rate.max(1),
            ctl,
            phase: Phase::Idle,
            since: tokio::time::Instant::now(),
            scan: HeadScan::new(lim.max_request_line),
            max_request_line: lim.max_request_line,
            read_timer: None,
            write_start: None,
            write_bytes: 0,
            write_timer: None,
            notify,
        }
    }

    fn sync(&mut self) {
        if self.ctl.serving.load(Ordering::Acquire) {
            self.phase = Phase::Request;
        }
    }

    fn reject(&mut self, status: u16, reason: &str, body: &str) {
        self.ctl.linger.store(true, Ordering::Release);
        if let Some(raw) = &mut self.notify {
            use std::io::Write;
            let _ = raw.write(&raw_response(status, reason, body, ""));
        }
    }

    /// `Some(phase)` once the deadline of the current phase has passed.
    fn expired(&mut self, cx: &mut Context<'_>) -> Option<Phase> {
        let limit = match self.phase {
            Phase::Request => {
                self.read_timer = None;
                return None;
            }
            Phase::Head => self.header_timeout,
            Phase::Idle if self.ctl.served.load(Ordering::Relaxed) == 0 => self.header_timeout,
            Phase::Idle => self.idle_timeout,
        };
        let deadline = self.since + limit;
        let timer = self
            .read_timer
            .get_or_insert_with(|| Box::pin(tokio::time::sleep_until(deadline)));
        if timer.deadline() != deadline {
            timer.as_mut().reset(deadline);
        }
        timer.as_mut().poll(cx).is_ready().then_some(self.phase)
    }

    /// Checks the write deadline while a write or flush is pending.
    fn write_expired(&mut self, cx: &mut Context<'_>) -> bool {
        let Some(start) = self.write_start else {
            return false;
        };
        let allowed = self.write_timeout
            + Duration::from_secs_f64(self.write_bytes as f64 / self.write_min_rate as f64);
        let deadline = start + allowed;
        let timer = self
            .write_timer
            .get_or_insert_with(|| Box::pin(tokio::time::sleep_until(deadline)));
        if timer.deadline() != deadline {
            timer.as_mut().reset(deadline);
        }
        timer.as_mut().poll(cx).is_ready()
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Guard<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.sync();
        let before = buf.filled().len();
        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                let got = &buf.filled()[before..];
                if !got.is_empty() && this.phase != Phase::Request {
                    if this.phase == Phase::Idle {
                        this.phase = Phase::Head;
                        this.since = tokio::time::Instant::now();
                    }
                    if let Err(e) = this.scan.feed(got) {
                        match e {
                            ScanError::BareLf => {
                                this.reject(400, "Bad Request", "Bare LF in request")
                            }
                            ScanError::LineTooLong => {
                                this.reject(414, "URI Too Long", "Request line too long")
                            }
                        }
                        return Poll::Ready(Err(io::ErrorKind::InvalidData.into()));
                    }
                }
                Poll::Ready(Ok(()))
            }
            Poll::Pending => match this.expired(cx) {
                None => Poll::Pending,
                Some(Phase::Head) => {
                    this.reject(408, "Request Timeout", "Request timeout");
                    Poll::Ready(Err(io::ErrorKind::TimedOut.into()))
                }
                // nothing arrived: end of stream, hyper closes quietly
                Some(_) => Poll::Ready(Ok(())),
            },
            other => other,
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Guard<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        this.sync();
        this.write_start
            .get_or_insert_with(tokio::time::Instant::now);
        match Pin::new(&mut this.inner).poll_write(cx, data) {
            Poll::Ready(Ok(n)) => {
                this.write_bytes += n;
                Poll::Ready(Ok(n))
            }
            Poll::Pending if this.write_expired(cx) => {
                this.ctl.linger.store(true, Ordering::Release);
                Poll::Ready(Err(io::ErrorKind::TimedOut.into()))
            }
            other => other,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_flush(cx) {
            Poll::Ready(Ok(())) => {
                if this.write_start.take().is_some() {
                    this.write_bytes = 0;
                    this.write_timer = None;
                    this.sync();
                    if this.phase != Phase::Request || !this.ctl.serving.load(Ordering::Acquire) {
                        this.phase = Phase::Idle;
                        this.since = tokio::time::Instant::now();
                        this.scan = HeadScan::new(this.max_request_line);
                    }
                }
                Poll::Ready(Ok(()))
            }
            Poll::Pending if this.write_expired(cx) => {
                Poll::Ready(Err(io::ErrorKind::TimedOut.into()))
            }
            other => other,
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

// ---------------------------------------------------------------------------------- the server

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

#[derive(Default)]
struct State {
    total: usize,
    rejecting: usize,
    per_ip: HashMap<IpAddr, usize>,
    /// The accept loop is still running (its listener is open).
    accepting: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stop {
    Run,
    /// Finish the request being answered, close the rest.
    Graceful,
    Force,
}

struct Shared {
    handler: Handler,
    limits: Limits,
    stopping: AtomicBool,
    state: Mutex<State>,
    state_cv: Condvar,
    slots: Arc<Semaphore>,
    throttle: Mutex<AuthThrottle>,
    tls: Option<TlsAcceptor>,
}

impl Shared {
    fn changed(&self) {
        self.state_cv.notify_all();
    }
}

struct ConnGuard {
    shared: Arc<Shared>,
    ip: IpAddr,
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        let mut st = lock(&self.shared.state);
        st.total = st.total.saturating_sub(1);
        if let Some(n) = st.per_ip.get_mut(&self.ip) {
            *n -= 1;
            if *n == 0 {
                st.per_ip.remove(&self.ip);
            }
        }
        drop(st);
        self.shared.changed();
    }
}

/// A running transport. Dropping it (or [`stop`](Self::stop)) shuts it down.
pub struct Running {
    addr: SocketAddr,
    shared: Arc<Shared>,
    stop_tx: watch::Sender<Stop>,
    runtime: Option<tokio::runtime::Runtime>,
}

impl Running {
    /// Plain HTTP.
    pub fn start(bind: &str, port: u16, limits: Limits, handler: Handler) -> Result<Running> {
        Running::start_tls(bind, port, limits, handler, None)
    }

    /// HTTPS when `tls` is given (see [`tls_config`]), plain HTTP otherwise.
    pub fn start_tls(
        bind: &str,
        port: u16,
        limits: Limits,
        handler: Handler,
        tls: Option<Arc<rustls::ServerConfig>>,
    ) -> Result<Running> {
        let listener = std::net::TcpListener::bind((bind, port))
            .with_context(|| format!("binding {bind}:{port}"))?;
        listener
            .set_nonblocking(true)
            .context("configuring the listener")?;
        let addr = listener.local_addr().context("reading the bound address")?;
        let throttle = AuthThrottle::new(
            limits.auth_max_failures,
            limits.auth_window,
            limits.auth_block,
        );
        let handlers = limits.handler_threads.max(1);
        let shared = Arc::new(Shared {
            handler,
            slots: Arc::new(Semaphore::new(handlers)),
            limits,
            stopping: AtomicBool::new(false),
            state: Mutex::new(State {
                accepting: true,
                ..State::default()
            }),
            state_cv: Condvar::new(),
            throttle: Mutex::new(throttle),
            tls: tls.map(TlsAcceptor::from),
        });
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .max_blocking_threads(handlers)
            .thread_name("bookx-http")
            .enable_io()
            .enable_time()
            .build()
            .context("starting the HTTP runtime")?;
        let (stop_tx, stop_rx) = watch::channel(Stop::Run);
        let sh = shared.clone();
        runtime.spawn(async move {
            if let Ok(l) = tokio::net::TcpListener::from_std(listener) {
                accept_loop(l, sh.clone(), stop_rx).await;
            }
            lock(&sh.state).accepting = false;
            sh.changed();
        });
        Ok(Running {
            addr,
            shared,
            stop_tx,
            runtime: Some(runtime),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Number of connections currently open (for tests and diagnostics).
    pub fn open_connections(&self) -> usize {
        lock(&self.shared.state).total
    }

    /// Stops accepting (the port is free when this returns), lets a request being answered finish,
    /// closes every other connection and waits for them.
    pub fn stop(&mut self) {
        let Some(runtime) = self.runtime.take() else {
            return;
        };
        self.shared.stopping.store(true, Ordering::SeqCst);
        let _ = self.stop_tx.send(Stop::Graceful);
        let released = self.wait_state(Duration::from_secs(3), |st| !st.accepting);
        debug_assert!(released);
        if !self.wait_state(Duration::from_secs(3), |st| st.total + st.rejecting == 0) {
            let _ = self.stop_tx.send(Stop::Force);
            self.wait_state(Duration::from_secs(5), |st| st.total + st.rejecting == 0);
        }
        runtime.shutdown_timeout(Duration::from_secs(2));
    }

    fn wait_state(&self, max: Duration, done: impl Fn(&State) -> bool) -> bool {
        let deadline = Instant::now() + max;
        let mut st = lock(&self.shared.state);
        while !done(&st) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            st = self
                .shared
                .state_cv
                .wait_timeout(st, left)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
        true
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Builds a TLS server configuration (rustls, ring provider, HTTP/1.1 only) from a PEM
/// certificate chain and a PEM private key (PKCS#8, PKCS#1 or SEC1).
pub fn tls_config(cert: &Path, key: &Path) -> Result<Arc<rustls::ServerConfig>> {
    let chain = CertificateDer::pem_file_iter(cert)
        .with_context(|| format!("reading {}", cert.display()))?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("parsing {}", cert.display()))?;
    if chain.is_empty() {
        bail!("no certificate in {}", cert.display());
    }
    let key = PrivateKeyDer::from_pem_file(key)
        .with_context(|| format!("reading the private key {}", key.display()))?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .context("selecting TLS versions")?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .context("the certificate and key do not match")?;
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(cfg))
}

async fn accept_loop(
    listener: tokio::net::TcpListener,
    shared: Arc<Shared>,
    mut stop: watch::Receiver<Stop>,
) {
    loop {
        tokio::select! {
            _ = stop.changed() => break,
            accepted = listener.accept() => match accepted {
                Ok((tcp, peer)) => admit(&shared, tcp, peer, stop.clone()),
                Err(e) => {
                    if !matches!(
                        e.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::ConnectionAborted
                    ) {
                        // out of file descriptors or similar: do not spin
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
            },
        }
    }
}

fn admit(
    shared: &Arc<Shared>,
    tcp: tokio::net::TcpStream,
    peer: SocketAddr,
    stop: watch::Receiver<Stop>,
) {
    let ip = peer.ip().to_canonical();
    let lim = &shared.limits;
    {
        let mut st = lock(&shared.state);
        let from_ip = st.per_ip.get(&ip).copied().unwrap_or(0);
        if st.total >= lim.max_connections || from_ip >= lim.max_connections_per_ip {
            drop(st);
            reject_busy(shared, tcp);
            return;
        }
        st.total += 1;
        *st.per_ip.entry(ip).or_insert(0) += 1;
    }
    let guard = ConnGuard {
        shared: shared.clone(),
        ip,
    };
    let sh = shared.clone();
    tokio::spawn(async move {
        let _guard = guard;
        serve_conn(sh, tcp, ip, stop).await;
    });
}

const MAX_REJECTORS: usize = 16;

/// Reads and discards what the peer still sends (bounded) so it sees our answer instead of a
/// reset, then closes.
async fn drain_close(mut s: tokio::net::TcpStream) {
    let _ = s.shutdown().await;
    let _ = tokio::time::timeout(Duration::from_secs(1), async {
        let mut left = 4 * 1024 * 1024usize;
        let mut chunk = [0u8; 16 * 1024];
        while left > 0 {
            match s.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => left = left.saturating_sub(n),
            }
        }
    })
    .await;
}

/// Answers `503` on a throw-away task (bounded) so a flood cannot slow the accept loop down.
fn reject_busy(shared: &Arc<Shared>, mut tcp: tokio::net::TcpStream) {
    {
        let mut st = lock(&shared.state);
        if st.rejecting >= MAX_REJECTORS {
            return; // beyond even that: just drop the connection
        }
        st.rejecting += 1;
    }
    struct Done(Arc<Shared>);
    impl Drop for Done {
        fn drop(&mut self) {
            let mut st = lock(&self.0.state);
            st.rejecting = st.rejecting.saturating_sub(1);
            drop(st);
            self.0.changed();
        }
    }
    let done = Done(shared.clone());
    tokio::spawn(async move {
        let _done = done;
        let data = raw_response(
            503,
            "Service Unavailable",
            "Server busy",
            "Retry-After: 1\r\n",
        );
        let wrote = tokio::time::timeout(Duration::from_secs(1), tcp.write_all(&data)).await;
        if matches!(wrote, Ok(Ok(()))) {
            drain_close(tcp).await;
        }
    });
}

async fn serve_conn(
    sh: Arc<Shared>,
    tcp: tokio::net::TcpStream,
    ip: IpAddr,
    stop: watch::Receiver<Stop>,
) {
    let _ = tcp.set_nodelay(true);
    let dup = |tcp: &tokio::net::TcpStream| {
        tcp.as_fd()
            .try_clone_to_owned()
            .ok()
            .map(std::net::TcpStream::from)
    };
    let (closer, notify) = (dup(&tcp), dup(&tcp));
    let ctl = Arc::new(Ctl::default());
    let forced = match sh.tls.clone() {
        None => {
            let io = Guard::new(tcp, &sh.limits, ctl.clone(), notify);
            run_conn(io, &sh, ctl.clone(), ip, stop, &closer).await
        }
        Some(acceptor) => {
            let hs = tokio::time::timeout(sh.limits.header_timeout, acceptor.accept(tcp)).await;
            let Ok(Ok(tls)) = hs else {
                return;
            };
            let io = Guard::new(tls, &sh.limits, ctl.clone(), None);
            run_conn(io, &sh, ctl.clone(), ip, stop, &closer).await
        }
    };
    if !forced && ctl.linger.load(Ordering::Acquire) {
        if let Some(Ok(s)) = closer.map(|c| {
            let _ = c.shutdown(Shutdown::Write);
            tokio::net::TcpStream::from_std(c)
        }) {
            drain_close(s).await;
        }
    }
}

/// Drives one connection until it ends. `true`: it was cut off by a forced stop.
async fn run_conn<S>(
    io: Guard<S>,
    sh: &Arc<Shared>,
    ctl: Arc<Ctl>,
    ip: IpAddr,
    mut stop: watch::Receiver<Stop>,
    closer: &Option<std::net::TcpStream>,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let lim = &sh.limits;
    let svc_sh = sh.clone();
    let svc = service_fn(move |req: hyper::Request<Incoming>| {
        let (sh, ctl) = (svc_sh.clone(), ctl.clone());
        async move { Ok::<_, Infallible>(respond(sh, ctl, ip, req).await) }
    });
    let conn = hyper::server::conn::http1::Builder::new()
        .half_close(true)
        .max_buf_size(lim.max_header_bytes.max(8192))
        .max_headers(lim.max_headers)
        .serve_connection(TokioIo::new(io), svc);
    tokio::pin!(conn);
    let mut graceful = false;
    loop {
        let state = *stop.borrow();
        match state {
            Stop::Force => return true,
            Stop::Graceful if !graceful => {
                graceful = true;
                conn.as_mut().graceful_shutdown();
                if let Some(c) = closer {
                    let _ = c.shutdown(Shutdown::Read);
                }
            }
            _ => {}
        }
        tokio::select! {
            _ = conn.as_mut() => return false,
            changed = stop.changed() => {
                if changed.is_err() {
                    return true;
                }
            }
        }
    }
}

struct Serving(Arc<Ctl>);

impl Drop for Serving {
    fn drop(&mut self) {
        self.0.serving.store(false, Ordering::Release);
    }
}

async fn respond(
    sh: Arc<Shared>,
    ctl: Arc<Ctl>,
    ip: IpAddr,
    req: hyper::Request<Incoming>,
) -> hyper::Response<Full<Bytes>> {
    ctl.serving.store(true, Ordering::Release);
    let _serving = Serving(ctl.clone());
    let served = ctl.served.fetch_add(1, Ordering::Relaxed) + 1;
    let lim = &sh.limits;
    let head_only = req.method() == hyper::Method::HEAD;
    let http10 = req.version() == Version::HTTP_10;
    let close_wanted = wants_close(req.headers())
        || served >= lim.max_requests_per_connection
        || sh.stopping.load(Ordering::SeqCst);
    let (resp, fatal) = match handle(&sh, ip, req).await {
        Ok(resp) => (resp, false),
        Err(e) => (Response::text(e.status, e.message), true),
    };
    let cut = fatal || resp.status == 429;
    if cut {
        ctl.linger.store(true, Ordering::Release);
    }
    to_hyper(resp, head_only, (!http10).then_some(cut || close_wanted))
}

async fn handle(
    sh: &Arc<Shared>,
    ip: IpAddr,
    req: hyper::Request<Incoming>,
) -> Result<Response, HttpError> {
    let lim = &sh.limits;
    let (parts, body) = req.into_parts();
    let target = check_head(&parts, lim)?;
    let ip = if lim.trusted_proxies.is_empty() {
        ip
    } else {
        let xff: Vec<&str> = parts
            .headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect();
        client_ip(ip, &xff, &lim.trusted_proxies)
    };
    if let Some(wait) = lock(&sh.throttle).check(ip, Instant::now()) {
        return Ok(
            Response::text(429, "Too many failed logins; try again later")
                .with_header("Retry-After", &wait.as_secs().max(1).to_string()),
        );
    }
    let mut request = Request::new(parts.method.as_str(), &target);
    for (k, v) in &parts.headers {
        let v = v
            .to_str()
            .map_err(|_| err(400, "Invalid characters in header"))?;
        request
            .headers
            .push((k.as_str().to_string(), v.to_string()));
    }
    let has_body = parts
        .headers
        .get(hyper::header::CONTENT_LENGTH)
        .is_some_and(|v| v != "0");
    if has_body {
        let collected = tokio::time::timeout(lim.body_timeout, body.collect())
            .await
            .map_err(|_| err(408, "Request timeout"))?
            .map_err(|_| err(400, "Bad request body"))?;
        request.body = collected.to_bytes().to_vec();
    }
    let presented =
        request.header("authorization").is_some() || request.header("x-auth-key").is_some();
    let permit = sh
        .slots
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| err(503, "Server stopping"))?;
    let handler = sh.handler.clone();
    let resp = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(&request)))
    })
    .await
    .ok()
    .and_then(|r| r.ok())
    .unwrap_or_else(|| Response::text(500, "Internal error"));
    if resp.status == 401 && presented {
        lock(&sh.throttle).record_failure(ip, Instant::now());
    }
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(t: &str) -> u16 {
        normalize_target(t).unwrap_err().status
    }

    fn ip(n: u8) -> IpAddr {
        IpAddr::from([10, 0, 0, n])
    }

    fn parts(req: &str) -> hyper::http::request::Parts {
        let mut lines = req.split("\r\n");
        let mut first = lines.next().unwrap().split(' ');
        let (m, t) = (first.next().unwrap(), first.next().unwrap());
        let mut b = hyper::Request::builder().method(m).uri(t);
        for l in lines {
            let (k, v) = l.split_once(':').unwrap();
            b = b.header(k, v.trim());
        }
        b.body(()).unwrap().into_parts().0
    }

    fn head_status(req: &str, lim: &Limits) -> Result<String, u16> {
        check_head(&parts(req), lim).map_err(|e| e.status)
    }

    #[test]
    fn origin_form_targets_pass_unchanged() {
        assert_eq!(normalize_target("/a/b?x=1").unwrap(), "/a/b?x=1");
        assert_eq!(normalize_target("/").unwrap(), "/");
        assert_eq!(normalize_target("/a%2f..%2f").unwrap(), "/a%2f..%2f");
    }

    #[test]
    fn absolute_form_targets() {
        assert_eq!(normalize_target("http://h:80/a/b?q=1").unwrap(), "/a/b?q=1");
        assert_eq!(normalize_target("HTTP://h").unwrap(), "/");
        assert_eq!(normalize_target("http://h?x=1").unwrap(), "/?x=1");
        assert_eq!(status("https://h/"), 400);
        assert_eq!(status("ftp://h/"), 400);
        assert_eq!(status("http:///a"), 400);
        assert_eq!(status("http://user@h/a"), 400);
        assert_eq!(status("http:/h/a"), 400);
    }

    #[test]
    fn odd_targets_are_refused() {
        for t in [
            "", "*", "nonslash", "host:443", "/a#frag", "/é", "/a b", "/\u{1}", "/\x7f",
        ] {
            assert_eq!(status(t), 400, "{t:?}");
        }
    }

    #[test]
    fn methods_must_be_upper_case_tokens() {
        assert!(valid_method("GET") && valid_method("PROPFIND"));
        for m in ["", "get", "Get", "G3T", &"A".repeat(17), "GE T"] {
            assert!(!valid_method(m), "{m:?}");
        }
        let lim = Limits::default();
        assert_eq!(head_status("get / HTTP/1.1", &lim), Err(400));
        assert_eq!(head_status("GET / HTTP/1.1", &lim), Ok("/".into()));
    }

    #[test]
    fn request_line_length_limit_is_exact() {
        let lim = Limits {
            max_request_line: 30,
            ..Limits::default()
        };
        // "GET /aaaaaaaaaaaaaa HTTP/1.1" is 28 bytes
        assert!(head_status("GET /aaaaaaaaaaaaaa HTTP/1.1", &lim).is_ok());
        assert!(head_status("GET /aaaaaaaaaaaaaaaa HTTP/1.1", &lim).is_ok()); // exactly 30
        assert_eq!(
            head_status("GET /aaaaaaaaaaaaaaaaa HTTP/1.1", &lim),
            Err(414)
        );
    }

    #[test]
    fn transfer_encoding_and_expect_are_refused() {
        let lim = Limits::default();
        for te in ["chunked", "identity", "gzip, chunked", "xchunked"] {
            let r = format!("PUT / HTTP/1.1\r\nTransfer-Encoding: {te}");
            assert_eq!(head_status(&r, &lim), Err(501), "{te}");
        }
        assert!(head_status("PUT / HTTP/1.1\r\nExpect: 100-continue", &lim).is_ok());
        assert!(head_status("PUT / HTTP/1.1\r\nExpect: 100-CONTINUE", &lim).is_ok());
        assert_eq!(
            head_status("PUT / HTTP/1.1\r\nExpect: banana", &lim),
            Err(417)
        );
    }

    #[test]
    fn content_length_is_checked_against_max_body() {
        let lim = Limits::default();
        let max = lim.max_body;
        assert!(head_status(&format!("PUT / HTTP/1.1\r\nContent-Length: {max}"), &lim).is_ok());
        assert_eq!(
            head_status(
                &format!("PUT / HTTP/1.1\r\nContent-Length: {}", max + 1),
                &lim
            ),
            Err(413)
        );
        assert_eq!(
            head_status(
                "PUT / HTTP/1.1\r\nContent-Length: 18446744073709551615",
                &lim
            ),
            Err(413)
        );
        assert_eq!(
            head_status(
                "PUT / HTTP/1.1\r\nContent-Length: 99999999999999999999999",
                &lim
            ),
            Err(400)
        );
    }

    #[test]
    fn connection_header_tokens() {
        let h = |v: &str| {
            let mut m = hyper::HeaderMap::new();
            m.append(hyper::header::CONNECTION, HeaderValue::from_str(v).unwrap());
            wants_close(&m)
        };
        assert!(h("close") && h("keep-alive, Close") && h("CLOSE"));
        assert!(!h("keep-alive") && !h("upgrade"));
        assert!(!wants_close(&hyper::HeaderMap::new()));
    }

    #[test]
    fn response_conversion_sets_framing_and_drops_unsafe_headers() {
        let r = Response::text(200, "hi")
            .with_header("Content-Length", "9999")
            .with_header("Connection", "upgrade")
            .with_header("Transfer-Encoding", "chunked")
            .with_header("Date", "x")
            .with_header("X-Ok", "1")
            .with_header("X-Evil", "a\r\nSet-Cookie: x=1")
            .with_header("Bad Name", "v");
        let out = to_hyper(r, false, Some(false));
        let h = out.headers();
        assert_eq!(h["content-length"], "2");
        assert_eq!(h["connection"], "keep-alive");
        assert_eq!(h["x-ok"], "1");
        assert_eq!(h["x-content-type-options"], "nosniff");
        for gone in ["transfer-encoding", "date", "x-evil", "set-cookie"] {
            assert!(h.get(gone).is_none(), "{gone}");
        }
        assert_eq!(h.get_all("content-length").iter().count(), 1);
        assert_eq!(
            to_hyper(Response::text(200, "x"), false, Some(true)).headers()["connection"],
            "close"
        );
    }

    #[test]
    fn nosniff_is_not_duplicated() {
        let r = Response::text(200, "x").with_header("x-content-type-options", "nosniff");
        let out = to_hyper(r, false, Some(false));
        assert_eq!(
            out.headers()
                .get_all("x-content-type-options")
                .iter()
                .count(),
            1
        );
    }

    #[test]
    fn bodyless_statuses_and_head() {
        let out = to_hyper(Response::empty(204), false, Some(false));
        assert!(out.headers().get("content-length").is_none());
        let out = to_hyper(Response::text(304, "nope"), false, Some(false));
        assert!(out.headers().get("content-length").is_none());
        // HEAD keeps the length of the body it does not send
        let out = to_hyper(Response::text(200, "12345"), true, Some(false));
        assert_eq!(out.headers()["content-length"], "5");
        // an unusual status survives, an impossible one becomes 500
        assert_eq!(
            to_hyper(Response::empty(299), false, Some(false)).status(),
            299
        );
        assert_eq!(
            to_hyper(Response::empty(1000), false, Some(false)).status(),
            500
        );
    }

    #[test]
    fn raw_responses_are_well_formed() {
        let r = String::from_utf8(raw_response(
            503,
            "Service Unavailable",
            "busy",
            "Retry-After: 1\r\n",
        ))
        .unwrap();
        assert!(r.starts_with("HTTP/1.1 503 Service Unavailable\r\n"));
        assert!(r.contains("Content-Length: 4\r\n") && r.contains("Retry-After: 1\r\n"));
        assert!(r.contains("nosniff") && r.contains("Connection: close\r\n"));
        assert!(r.ends_with("\r\n\r\nbusy"));
    }

    // ---------------------------------------------------------------------- head scan

    fn scan() -> HeadScan {
        HeadScan::new(8192)
    }

    #[test]
    fn head_scan_accepts_crlf_heads_and_stops_at_the_blank_line() {
        let mut s = scan();
        assert_eq!(s.feed(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n"), Ok(()));
        assert!(s.done);
        // anything after the head (a body with bare LFs) is not looked at
        assert_eq!(s.feed(b"line one\nline two\n"), Ok(()));
    }

    #[test]
    fn head_scan_rejects_bare_lf_anywhere_in_the_head() {
        for raw in [
            &b"GET / HTTP/1.1\nHost: x\n\n"[..],
            b"GET / HTTP/1.1\r\nX: a\nGET /opds HTTP/1.1\r\n\r\n",
            b"GET / HTTP/1.1\r\nHost: x\r\n\n",
            b"\nGET / HTTP/1.1\r\n\r\n",
        ] {
            assert_eq!(scan().feed(raw), Err(ScanError::BareLf), "{raw:?}");
        }
    }

    #[test]
    fn head_scan_works_across_chunk_boundaries() {
        let mut s = scan();
        assert_eq!(s.feed(b"GET / HTTP/1.1\r"), Ok(()));
        assert_eq!(s.feed(b"\nHost: x\r\n"), Ok(()));
        assert_eq!(s.feed(b"\r"), Ok(()));
        assert!(!s.done);
        assert_eq!(s.feed(b"\n"), Ok(()));
        assert!(s.done);
        let mut s = scan();
        assert_eq!(s.feed(b"GET / HTTP/1.1\r\nX: a"), Ok(()));
        assert_eq!(s.feed(b"\nb"), Err(ScanError::BareLf));
    }

    #[test]
    fn head_scan_ignores_leading_blank_lines() {
        let mut s = scan();
        assert_eq!(s.feed(b"\r\n\r\n"), Ok(()));
        assert!(
            !s.done,
            "blank lines before the request line are not a head"
        );
        assert_eq!(s.feed(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n"), Ok(()));
        assert!(s.done);
    }

    #[test]
    fn head_scan_resyncs_after_partial_terminator() {
        let mut s = scan();
        assert_eq!(s.feed(b"GET / HTTP/1.1\r\nA: b\r\r\n\r\n"), Ok(()));
        assert!(s.done);
        let mut s = scan();
        assert_eq!(s.feed(b"GET / HTTP/1.1\r\nA: b\r\n\rX\r\n\r\n"), Ok(()));
        assert!(s.done);
    }

    #[test]
    fn head_scan_cuts_off_a_request_line_that_never_ends() {
        let mut s = HeadScan::new(10);
        assert_eq!(s.feed(b"GET /abcde"), Ok(())); // exactly 10
        assert_eq!(s.feed(b"f"), Err(ScanError::LineTooLong));
        // the limit is for the request line only; headers may be longer
        let mut s = HeadScan::new(10);
        assert_eq!(s.feed(b"GET /a HTTP\r\n"), Err(ScanError::LineTooLong));
        let mut s = HeadScan::new(14);
        assert_eq!(
            s.feed(b"GET /a HTTP\r\nX: a-very-long-header-value\r\n\r\n"),
            Ok(())
        );
        // many small chunks add up
        let mut s = HeadScan::new(5);
        for b in b"abcdef" {
            let r = s.feed(&[*b]);
            if *b == b'f' {
                assert_eq!(r, Err(ScanError::LineTooLong));
            } else {
                assert_eq!(r, Ok(()));
            }
        }
    }

    // -------------------------------------------------------------------------- misc

    #[test]
    fn default_limits_match_the_documentation() {
        let l = Limits::default();
        assert_eq!(l.max_request_line, 8192);
        assert_eq!(l.max_header_bytes, 16384);
        assert_eq!(l.max_headers, 100);
        assert_eq!(l.max_body, 1024 * 1024);
        assert_eq!(l.max_connections, 64);
        assert_eq!(l.max_connections_per_ip, 32);
        assert_eq!(l.max_requests_per_connection, 1000);
        assert_eq!(l.handler_threads, 8);
        assert_eq!(l.header_timeout, Duration::from_secs(10));
        assert_eq!(l.body_timeout, Duration::from_secs(30));
        assert_eq!(l.idle_timeout, Duration::from_secs(15));
        assert_eq!(l.write_timeout, Duration::from_secs(30));
        assert_eq!(l.write_min_rate, 128 * 1024);
        assert_eq!(
            (l.auth_max_failures, l.auth_window, l.auth_block),
            (10, Duration::from_secs(60), Duration::from_secs(60))
        );
    }

    fn tmpdir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-net-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn tls_config_reports_missing_and_broken_files() {
        let d = tmpdir();
        let e = tls_config(&d.join("no.pem"), &d.join("no.key")).unwrap_err();
        assert!(format!("{e:#}").contains("no.pem"), "{e:#}");

        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        std::fs::write(d.join("c.pem"), cert.cert.pem()).unwrap();
        std::fs::write(d.join("k.pem"), cert.signing_key.serialize_pem()).unwrap();
        std::fs::write(d.join("empty.pem"), "").unwrap();
        std::fs::write(d.join("junk.key"), "-----BEGIN NOPE-----\n").unwrap();

        let e = tls_config(&d.join("empty.pem"), &d.join("k.pem")).unwrap_err();
        assert!(format!("{e:#}").contains("no certificate"), "{e:#}");
        let e = tls_config(&d.join("c.pem"), &d.join("junk.key")).unwrap_err();
        assert!(format!("{e:#}").contains("junk.key"), "{e:#}");
        let e = tls_config(&d.join("c.pem"), &d.join("c.pem")).unwrap_err();
        assert!(format!("{e:#}").contains("private key"), "{e:#}");

        // a certificate with somebody else's key is refused
        let other = rcgen::generate_simple_self_signed(vec!["x".to_string()]).unwrap();
        std::fs::write(d.join("o.key"), other.signing_key.serialize_pem()).unwrap();
        let e = tls_config(&d.join("c.pem"), &d.join("o.key")).unwrap_err();
        assert!(format!("{e:#}").contains("do not match"), "{e:#}");

        let ok = tls_config(&d.join("c.pem"), &d.join("k.pem")).unwrap();
        assert_eq!(ok.alpn_protocols, vec![b"http/1.1".to_vec()]);
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn start_reports_a_busy_port() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h: Handler = Arc::new(|_| Response::empty(204));
        let e = Running::start("127.0.0.1", port, Limits::default(), h)
            .err()
            .unwrap();
        assert!(format!("{e:#}").contains("binding"), "{e:#}");
    }

    #[test]
    fn stop_is_idempotent_and_frees_the_port() {
        let h: Handler = Arc::new(|_| Response::empty(204));
        let mut r = Running::start("127.0.0.1", 0, Limits::default(), h).unwrap();
        let addr = r.addr();
        assert_eq!(r.open_connections(), 0);
        r.stop();
        r.stop();
        std::net::TcpListener::bind(addr).expect("port is free again");
    }

    // ------------------------------------------------------------------------ throttle

    fn nets(s: &str) -> Vec<ProxyNet> {
        parse_proxy_list(s).unwrap()
    }

    #[test]
    fn proxy_nets_parse_and_match() {
        assert!(ProxyNet::parse("10.0.0.0/8")
            .unwrap()
            .contains("10.9.8.7".parse().unwrap()));
        assert!(!ProxyNet::parse("10.0.0.0/8")
            .unwrap()
            .contains("11.0.0.1".parse().unwrap()));
        assert!(ProxyNet::parse("192.0.2.1")
            .unwrap()
            .contains("192.0.2.1".parse().unwrap()));
        assert!(!ProxyNet::parse("192.0.2.1")
            .unwrap()
            .contains("192.0.2.2".parse().unwrap()));
        assert!(ProxyNet::parse("fd00::/8")
            .unwrap()
            .contains("fd12::1".parse().unwrap()));
        // an IPv4-mapped peer matches the IPv4 entry
        assert!(ProxyNet::parse("127.0.0.1")
            .unwrap()
            .contains("::ffff:127.0.0.1".parse().unwrap()));
        for bad in [
            "",
            "0.0.0.0/0",
            "::/0",
            "10.0.0.0/33",
            "x",
            "10.0.0.0/",
            "example.com",
            "1.2.3.4/-1",
        ] {
            assert!(ProxyNet::parse(bad).is_none(), "{bad}");
        }
        assert_eq!(nets("127.0.0.1, 10.0.0.0/8;::1").len(), 3);
        assert_eq!(parse_proxy_list("127.0.0.1 nope").unwrap_err(), "nope");
        assert!(parse_proxy_list("").unwrap().is_empty());
    }

    #[test]
    fn forwarded_for_is_believed_only_from_a_trusted_proxy() {
        let trusted = nets("10.0.0.1");
        let peer: IpAddr = "10.0.0.1".parse().unwrap();
        let stranger: IpAddr = "203.0.113.9".parse().unwrap();
        // an untrusted peer cannot choose its address
        assert_eq!(client_ip(stranger, &["1.2.3.4"], &trusted), stranger);
        assert_eq!(
            client_ip(peer, &["198.51.100.7"], &trusted),
            "198.51.100.7".parse::<IpAddr>().unwrap()
        );
        // nothing to read: the proxy itself
        assert_eq!(client_ip(peer, &[], &trusted), peer);
        // the client's own forged left part is ignored, the proxy appended the real one
        assert_eq!(
            client_ip(peer, &["6.6.6.6, 198.51.100.7"], &trusted),
            "198.51.100.7".parse::<IpAddr>().unwrap()
        );
        // two proxies in a chain are skipped
        let t2 = nets("10.0.0.0/24");
        assert_eq!(
            client_ip(peer, &["198.51.100.7", "10.0.0.2"], &t2),
            "198.51.100.7".parse::<IpAddr>().unwrap()
        );
        // garbage stops the walk
        assert_eq!(client_ip(peer, &["1.2.3.4, nonsense"], &trusted), peer);
        // no trusted proxies: never believed
        assert_eq!(client_ip(peer, &["1.2.3.4"], &[]), peer);
    }

    fn throttle() -> AuthThrottle {
        AuthThrottle::new(3, Duration::from_secs(60), Duration::from_secs(120))
    }

    #[test]
    fn throttle_blocks_after_n_failures() {
        let mut t = throttle();
        let t0 = Instant::now();
        assert_eq!(t.check(ip(1), t0), None);
        t.record_failure(ip(1), t0);
        t.record_failure(ip(1), t0 + Duration::from_secs(1));
        assert_eq!(t.check(ip(1), t0 + Duration::from_secs(2)), None);
        t.record_failure(ip(1), t0 + Duration::from_secs(2));
        assert_eq!(
            t.check(ip(1), t0 + Duration::from_secs(2)),
            Some(Duration::from_secs(120))
        );
        assert_eq!(
            t.check(ip(1), t0 + Duration::from_secs(62)),
            Some(Duration::from_secs(60))
        );
        // other addresses are unaffected
        assert_eq!(t.check(ip(2), t0 + Duration::from_secs(2)), None);
    }

    #[test]
    fn throttle_block_expires_and_counting_restarts() {
        let mut t = throttle();
        let t0 = Instant::now();
        for _ in 0..3 {
            t.record_failure(ip(1), t0);
        }
        assert!(t.check(ip(1), t0 + Duration::from_secs(119)).is_some());
        assert_eq!(t.check(ip(1), t0 + Duration::from_secs(120)), None);
        let t1 = t0 + Duration::from_secs(121);
        t.record_failure(ip(1), t1);
        t.record_failure(ip(1), t1);
        assert_eq!(t.check(ip(1), t1), None, "needs three fresh failures");
        t.record_failure(ip(1), t1);
        assert!(t.check(ip(1), t1).is_some());
    }

    #[test]
    fn throttle_window_forgets_old_failures() {
        let mut t = throttle();
        let t0 = Instant::now();
        t.record_failure(ip(1), t0);
        t.record_failure(ip(1), t0);
        // the third failure comes after the 60 s window closed: counts as the first
        t.record_failure(ip(1), t0 + Duration::from_secs(61));
        assert_eq!(t.check(ip(1), t0 + Duration::from_secs(61)), None);
        t.record_failure(ip(1), t0 + Duration::from_secs(62));
        assert_eq!(t.check(ip(1), t0 + Duration::from_secs(62)), None);
        t.record_failure(ip(1), t0 + Duration::from_secs(63));
        assert!(t.check(ip(1), t0 + Duration::from_secs(63)).is_some());
    }

    #[test]
    fn failures_while_blocked_do_not_extend_the_block() {
        let mut t = throttle();
        let t0 = Instant::now();
        for _ in 0..3 {
            t.record_failure(ip(1), t0);
        }
        for s in 1..100 {
            t.record_failure(ip(1), t0 + Duration::from_secs(s));
        }
        assert_eq!(t.check(ip(1), t0 + Duration::from_secs(120)), None);
    }

    #[test]
    fn throttle_disabled_with_zero() {
        let mut t = AuthThrottle::new(0, Duration::from_secs(60), Duration::from_secs(60));
        let t0 = Instant::now();
        for _ in 0..1000 {
            t.record_failure(ip(1), t0);
        }
        assert_eq!(t.check(ip(1), t0), None);
        assert_eq!(t.tracked(), 0);
    }

    #[test]
    fn throttle_prunes_and_is_bounded() {
        let mut t = throttle();
        let t0 = Instant::now();
        t.record_failure(ip(1), t0);
        for _ in 0..3 {
            t.record_failure(ip(2), t0);
        }
        assert_eq!(t.tracked(), 2);
        t.prune(t0 + Duration::from_secs(61));
        assert_eq!(t.tracked(), 1, "ip 2 is still blocked");
        t.prune(t0 + Duration::from_secs(121));
        assert_eq!(t.tracked(), 0);

        // many distinct peers cannot grow the table without bound
        for i in 0..(MAX_TRACKED_PEERS as u32 + 500) {
            t.record_failure(IpAddr::from(i.to_be_bytes()), t0);
        }
        assert!(t.tracked() <= MAX_TRACKED_PEERS);
        // once the windows expired there is room again
        let later = t0 + Duration::from_secs(61);
        t.record_failure(ip(99), later);
        assert!(t.check(ip(99), later).is_none());
        assert!(t.tracked() <= MAX_TRACKED_PEERS);
    }

    #[test]
    fn ipv6_addresses_work_too() {
        let mut t = throttle();
        let a: IpAddr = "fe80::1".parse().unwrap();
        let t0 = Instant::now();
        for _ in 0..3 {
            t.record_failure(a, t0);
        }
        assert!(t.check(a, t0).is_some());
        assert!(t.check("fe80::2".parse().unwrap(), t0).is_none());
    }
}
