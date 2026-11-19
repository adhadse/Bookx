//! The only door to the network: a [`Fetcher`] trait, the real [`UreqFetcher`], a scripted
//! [`MockFetcher`] for tests, URL helpers, and the rate-limit / retry helpers (pure, with injected time).

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;

/// A GET request: URL plus headers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Request {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

impl Request {
    pub fn new(url: impl Into<String>) -> Self {
        Request {
            url: url.into(),
            headers: Vec::new(),
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// Value of a header, case-insensitive name.
    pub fn header_value(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Why a fetch failed. Wrapped in `anyhow::Error`; use [`FetchError::of`] to get it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// The server answered with a non-success status.
    Http(u16),
    Timeout,
    /// Connection, TLS, DNS, body too large, ...
    Other(String),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::Http(c) => write!(f, "HTTP error {c}"),
            FetchError::Timeout => write!(f, "request timed out"),
            FetchError::Other(m) => write!(f, "network error: {m}"),
        }
    }
}

impl std::error::Error for FetchError {}

impl FetchError {
    /// Extracts the [`FetchError`] from an error chain, if there is one.
    pub fn of(err: &anyhow::Error) -> Option<&FetchError> {
        err.downcast_ref::<FetchError>()
    }

    /// Worth trying again later: timeouts, 429 and 5xx.
    pub fn is_retryable(&self) -> bool {
        match self {
            FetchError::Http(c) => *c == 429 || (500..600).contains(c),
            FetchError::Timeout => true,
            FetchError::Other(_) => false,
        }
    }
}

/// Fetches a URL. Implementations must treat non-2xx statuses as errors ([`FetchError::Http`]).
pub trait Fetcher: Send + Sync {
    fn get(&self, url: &str, headers: &[(String, String)]) -> Result<Vec<u8>>;

    fn fetch(&self, req: &Request) -> Result<Vec<u8>> {
        self.get(&req.url, &req.headers)
    }
}

/// Real HTTP(S) client (rustls). Everything else in the crate is network free.
pub struct UreqFetcher {
    agent: ureq::Agent,
    max_bytes: u64,
}

impl UreqFetcher {
    pub const USER_AGENT: &'static str =
        "Bookx/0.2 (+https://github.com/adhadse/bookx; e-book manager)";

    pub fn new() -> Self {
        Self::with_limits(Duration::from_secs(20), 20 * 1024 * 1024)
    }

    pub fn with_limits(timeout: Duration, max_bytes: u64) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .user_agent(Self::USER_AGENT)
            .build()
            .into();
        UreqFetcher { agent, max_bytes }
    }
}

impl Default for UreqFetcher {
    fn default() -> Self {
        Self::new()
    }
}

static OFFLINE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Offline mode (PP-C-8, PP-C-22, PP-G-18): while it is on, [`UreqFetcher`] answers every request
/// with an error and sends nothing. It covers everything that fetches through this crate:
/// metadata and covers, web articles and news, online catalogues, link checks and dictionary lists.
pub fn set_offline(on: bool) {
    OFFLINE.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn is_offline() -> bool {
    OFFLINE.load(std::sync::atomic::Ordering::Relaxed)
}

/// The message shown when offline mode stopped a request.
pub const OFFLINE_MESSAGE: &str = "Offline mode is on (Preferences → Privacy)";

impl Fetcher for UreqFetcher {
    fn get(&self, url: &str, headers: &[(String, String)]) -> Result<Vec<u8>> {
        if is_offline() {
            return Err(FetchError::Other(OFFLINE_MESSAGE.into()).into());
        }
        let mut req = self.agent.get(url);
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let mut resp = req.call().map_err(map_ureq_error)?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(FetchError::Http(status).into());
        }
        let body = resp
            .body_mut()
            .with_config()
            .limit(self.max_bytes)
            .read_to_vec()
            .map_err(map_ureq_error)?;
        Ok(body)
    }
}

fn map_ureq_error(e: ureq::Error) -> anyhow::Error {
    match e {
        ureq::Error::Timeout(_) => FetchError::Timeout.into(),
        ureq::Error::StatusCode(c) => FetchError::Http(c).into(),
        other => FetchError::Other(other.to_string()).into(),
    }
}

/// What a [`MockFetcher`] answers for a URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MockReply {
    Body(Vec<u8>),
    Status(u16),
    Timeout,
    Failure(String),
}

/// Scripted fetcher for tests: exact URL or prefix matches, records every request.
#[derive(Default)]
pub struct MockFetcher {
    exact: Mutex<HashMap<String, MockReply>>,
    prefixes: Mutex<Vec<(String, MockReply)>>,
    log: Mutex<Vec<Request>>,
}

impl MockFetcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_body(self, url: &str, body: impl AsRef<[u8]>) -> Self {
        self.on(url, MockReply::Body(body.as_ref().to_vec()))
    }

    pub fn on(self, url: &str, reply: MockReply) -> Self {
        lock(&self.exact).insert(url.to_string(), reply);
        self
    }

    pub fn on_prefix(self, prefix: &str, reply: MockReply) -> Self {
        lock(&self.prefixes).push((prefix.to_string(), reply));
        self
    }

    /// Every request received so far, oldest first.
    pub fn requests(&self) -> Vec<Request> {
        lock(&self.log).clone()
    }

    pub fn urls(&self) -> Vec<String> {
        lock(&self.log).iter().map(|r| r.url.clone()).collect()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Fetcher for MockFetcher {
    fn get(&self, url: &str, headers: &[(String, String)]) -> Result<Vec<u8>> {
        lock(&self.log).push(Request {
            url: url.to_string(),
            headers: headers.to_vec(),
        });
        let reply = lock(&self.exact).get(url).cloned().or_else(|| {
            lock(&self.prefixes)
                .iter()
                .find(|(p, _)| url.starts_with(p.as_str()))
                .map(|(_, r)| r.clone())
        });
        match reply {
            Some(MockReply::Body(b)) => Ok(b),
            Some(MockReply::Status(c)) => Err(FetchError::Http(c).into()),
            Some(MockReply::Timeout) => Err(FetchError::Timeout.into()),
            Some(MockReply::Failure(m)) => Err(FetchError::Other(m).into()),
            None => Err(FetchError::Http(404).into()),
        }
    }
}

/// Percent-encodes a query-string component (RFC 3986 unreserved characters stay).
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Minimum spacing between requests to one host. Pure: the caller passes the clock.
#[derive(Debug, Clone)]
pub struct Throttle {
    min_interval_ms: u64,
    last_ms: Option<u64>,
}

impl Throttle {
    pub fn new(min_interval_ms: u64) -> Self {
        Throttle {
            min_interval_ms,
            last_ms: None,
        }
    }

    /// Milliseconds to wait before a request may be sent at `now_ms`.
    pub fn wait_needed(&self, now_ms: u64) -> u64 {
        match self.last_ms {
            None => 0,
            Some(last) => (last.saturating_add(self.min_interval_ms)).saturating_sub(now_ms),
        }
    }

    /// Records that a request was sent at `now_ms`.
    pub fn record(&mut self, now_ms: u64) {
        self.last_ms = Some(now_ms);
    }
}

/// Exponential backoff: `base * 2^attempt`, capped (attempt 0 is the first retry).
pub fn backoff_delay_ms(attempt: u32, base_ms: u64, cap_ms: u64) -> u64 {
    let factor = 1u64.checked_shl(attempt).unwrap_or(u64::MAX);
    base_ms.saturating_mul(factor).min(cap_ms)
}

/// Fetches with retries on retryable errors. `sleep` receives the delay in ms (injected so tests
/// never sleep). Gives up after `max_attempts` total attempts and returns the last error.
pub fn fetch_with_retry(
    fetcher: &dyn Fetcher,
    req: &Request,
    max_attempts: u32,
    base_ms: u64,
    cap_ms: u64,
    sleep: &mut dyn FnMut(u64),
) -> Result<Vec<u8>> {
    let attempts = max_attempts.max(1);
    let mut last = None;
    for attempt in 0..attempts {
        match fetcher.fetch(req) {
            Ok(b) => return Ok(b),
            Err(e) => {
                let retry = FetchError::of(&e).is_some_and(FetchError::is_retryable);
                if !retry || attempt + 1 == attempts {
                    return Err(e);
                }
                sleep(backoff_delay_ms(attempt, base_ms, cap_ms));
                last = Some(e);
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no attempts made")))
}

type Clock = Box<dyn Fn() -> u64 + Send + Sync>;
type Sleeper = Box<dyn Fn(u64) + Send + Sync>;

/// Wraps a fetcher with a minimum spacing between requests and retries with exponential backoff.
/// Clock and sleep are injected so tests run instantly.
pub struct PoliteFetcher<F: Fetcher> {
    inner: F,
    throttle: Mutex<Throttle>,
    max_attempts: u32,
    base_ms: u64,
    cap_ms: u64,
    clock: Clock,
    sleep: Sleeper,
}

impl<F: Fetcher> PoliteFetcher<F> {
    pub fn new(
        inner: F,
        min_interval_ms: u64,
        max_attempts: u32,
        base_ms: u64,
        cap_ms: u64,
        clock: Clock,
        sleep: Sleeper,
    ) -> Self {
        PoliteFetcher {
            inner,
            throttle: Mutex::new(Throttle::new(min_interval_ms)),
            max_attempts,
            base_ms,
            cap_ms,
            clock,
            sleep,
        }
    }

    /// Real clock and real sleeping; 3 attempts, 500 ms base delay, 8 s cap.
    pub fn system(inner: F, min_interval_ms: u64) -> Self {
        let start = std::time::Instant::now();
        Self::new(
            inner,
            min_interval_ms,
            3,
            500,
            8000,
            Box::new(move || start.elapsed().as_millis() as u64),
            Box::new(|ms| std::thread::sleep(Duration::from_millis(ms))),
        )
    }
}

impl<F: Fetcher> Fetcher for PoliteFetcher<F> {
    fn get(&self, url: &str, headers: &[(String, String)]) -> Result<Vec<u8>> {
        let wait = {
            let mut t = lock(&self.throttle);
            let now = (self.clock)();
            let wait = t.wait_needed(now);
            t.record(now + wait);
            wait
        };
        if wait > 0 {
            (self.sleep)(wait);
        }
        let req = Request {
            url: url.to_string(),
            headers: headers.to_vec(),
        };
        fetch_with_retry(
            &self.inner,
            &req,
            self.max_attempts,
            self.base_ms,
            self.cap_ms,
            &mut |ms| (self.sleep)(ms),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_builder_and_header_lookup() {
        let r = Request::new("http://x").header("Accept", "a/b");
        assert_eq!(r.url, "http://x");
        assert_eq!(r.header_value("accept"), Some("a/b"));
        assert_eq!(r.header_value("nope"), None);
    }

    #[test]
    fn mock_exact_prefix_and_default() {
        let m = MockFetcher::new()
            .with_body("http://a/1", "one")
            .on_prefix("http://a/", MockReply::Body(b"any".to_vec()));
        assert_eq!(m.get("http://a/1", &[]).unwrap(), b"one");
        assert_eq!(m.get("http://a/2", &[]).unwrap(), b"any");
        let e = m.get("http://b/", &[]).unwrap_err();
        assert_eq!(FetchError::of(&e), Some(&FetchError::Http(404)));
        assert_eq!(m.urls(), vec!["http://a/1", "http://a/2", "http://b/"]);
    }

    #[test]
    fn mock_error_kinds() {
        let m = MockFetcher::new()
            .on("http://t", MockReply::Timeout)
            .on("http://s", MockReply::Status(503))
            .on("http://f", MockReply::Failure("dns".into()));
        assert_eq!(
            FetchError::of(&m.get("http://t", &[]).unwrap_err()),
            Some(&FetchError::Timeout)
        );
        assert_eq!(
            FetchError::of(&m.get("http://s", &[]).unwrap_err()),
            Some(&FetchError::Http(503))
        );
        let e = m.get("http://f", &[]).unwrap_err();
        assert_eq!(e.to_string(), "network error: dns");
    }

    #[test]
    fn mock_records_headers_and_fetch_uses_request() {
        let m = MockFetcher::new().with_body("http://a", "x");
        let r = Request::new("http://a").header("X-Test", "1");
        assert_eq!(m.fetch(&r).unwrap(), b"x");
        assert_eq!(m.requests()[0].header_value("x-test"), Some("1"));
    }

    #[test]
    fn retryable_classification() {
        assert!(FetchError::Timeout.is_retryable());
        assert!(FetchError::Http(429).is_retryable());
        assert!(FetchError::Http(500).is_retryable());
        assert!(FetchError::Http(503).is_retryable());
        assert!(!FetchError::Http(404).is_retryable());
        assert!(!FetchError::Http(403).is_retryable());
        assert!(!FetchError::Other("x".into()).is_retryable());
    }

    #[test]
    fn display_messages() {
        assert_eq!(FetchError::Http(404).to_string(), "HTTP error 404");
        assert_eq!(FetchError::Timeout.to_string(), "request timed out");
    }

    #[test]
    fn url_encoding() {
        assert_eq!(url_encode("a b&c"), "a%20b%26c");
        assert_eq!(url_encode("é"), "%C3%A9");
        assert_eq!(url_encode("safe-_.~09Az"), "safe-_.~09Az");
        assert_eq!(url_encode(""), "");
    }

    #[test]
    fn throttle_waits_only_the_remaining_interval() {
        let mut t = Throttle::new(1000);
        assert_eq!(t.wait_needed(5), 0);
        t.record(100);
        assert_eq!(t.wait_needed(100), 1000);
        assert_eq!(t.wait_needed(600), 500);
        assert_eq!(t.wait_needed(1100), 0);
        assert_eq!(t.wait_needed(9999), 0);
        // clock going backwards must not underflow
        assert_eq!(t.wait_needed(0), 1100);
    }

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(backoff_delay_ms(0, 500, 10_000), 500);
        assert_eq!(backoff_delay_ms(1, 500, 10_000), 1000);
        assert_eq!(backoff_delay_ms(3, 500, 10_000), 4000);
        assert_eq!(backoff_delay_ms(10, 500, 10_000), 10_000);
        assert_eq!(backoff_delay_ms(200, 500, 10_000), 10_000);
        assert_eq!(backoff_delay_ms(5, u64::MAX, 7), 7);
    }

    #[test]
    fn retry_succeeds_after_transient_failures() {
        use std::sync::atomic::{AtomicU32, Ordering};
        struct Flaky(AtomicU32);
        impl Fetcher for Flaky {
            fn get(&self, _: &str, _: &[(String, String)]) -> Result<Vec<u8>> {
                if self.0.fetch_add(1, Ordering::SeqCst) < 2 {
                    Err(FetchError::Http(429).into())
                } else {
                    Ok(b"ok".to_vec())
                }
            }
        }
        let mut sleeps = Vec::new();
        let r = fetch_with_retry(
            &Flaky(AtomicU32::new(0)),
            &Request::new("u"),
            5,
            100,
            10_000,
            &mut |ms| sleeps.push(ms),
        )
        .unwrap();
        assert_eq!(r, b"ok");
        assert_eq!(sleeps, vec![100, 200]);
    }

    #[test]
    fn retry_gives_up_and_does_not_retry_permanent_errors() {
        let m = MockFetcher::new()
            .on("http://down", MockReply::Status(503))
            .on("http://gone", MockReply::Status(404));
        let mut sleeps = Vec::new();
        let e = fetch_with_retry(&m, &Request::new("http://down"), 3, 10, 100, &mut |ms| {
            sleeps.push(ms)
        })
        .unwrap_err();
        assert_eq!(FetchError::of(&e), Some(&FetchError::Http(503)));
        assert_eq!(sleeps, vec![10, 20]);
        assert_eq!(m.urls().len(), 3);

        let mut sleeps2 = Vec::new();
        let e = fetch_with_retry(&m, &Request::new("http://gone"), 3, 10, 100, &mut |ms| {
            sleeps2.push(ms)
        })
        .unwrap_err();
        assert_eq!(FetchError::of(&e), Some(&FetchError::Http(404)));
        assert!(sleeps2.is_empty());
    }

    #[test]
    fn retry_with_zero_attempts_still_tries_once() {
        let m = MockFetcher::new().with_body("u", "b");
        let r = fetch_with_retry(&m, &Request::new("u"), 0, 1, 1, &mut |_| {}).unwrap();
        assert_eq!(r, b"b");
    }

    #[test]
    fn ureq_fetcher_reports_bad_url_as_error_not_panic() {
        let f = UreqFetcher::with_limits(Duration::from_millis(200), 1024);
        assert!(f.get("not a url", &[]).is_err());
        let _ = UreqFetcher::default();
    }

    #[test]
    fn polite_fetcher_spaces_requests_and_retries() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;
        let now = Arc::new(AtomicU64::new(10_000));
        let slept = Arc::new(Mutex::new(Vec::<u64>::new()));
        let (n2, s2) = (now.clone(), slept.clone());
        let (n3, _) = (now.clone(), 0);
        let mock = MockFetcher::new()
            .with_body("http://a", "A")
            .on("http://down", MockReply::Status(503));
        let f = PoliteFetcher::new(
            mock,
            1000,
            2,
            100,
            1000,
            Box::new(move || n2.load(Ordering::SeqCst)),
            Box::new(move |ms| {
                lock(&s2).push(ms);
                n3.fetch_add(ms, Ordering::SeqCst);
            }),
        );
        assert_eq!(f.get("http://a", &[]).unwrap(), b"A");
        assert!(lock(&slept).is_empty(), "first request is immediate");
        now.fetch_add(300, Ordering::SeqCst);
        assert_eq!(f.get("http://a", &[]).unwrap(), b"A");
        assert_eq!(
            *lock(&slept),
            vec![700],
            "waits for the rest of the interval"
        );
        let e = f.get("http://down", &[]).unwrap_err();
        assert_eq!(FetchError::of(&e), Some(&FetchError::Http(503)));
        let s = lock(&slept).clone();
        assert_eq!(s[1], 1000, "spacing before the third request");
        assert_eq!(s[2], 100, "one backoff between the two attempts");
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn polite_fetcher_system_clock_constructs() {
        let f = PoliteFetcher::system(MockFetcher::new().with_body("u", "x"), 0);
        assert_eq!(f.get("u", &[]).unwrap(), b"x");
    }
}
