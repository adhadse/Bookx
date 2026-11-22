//! An on-disk cache of source responses, so asking for the same book twice (a re-run of a bulk
//! lookup, a second fetch of one book) does not go to the network again.
//!
//! * Entries are files named by a hash of the URL; the first lines say when they expire.
//! * The lifetime is short and depends on the source ([`ttl_secs`]): the terms of the sources do
//!   not allow keeping their data around for long, and a book's page changes rarely but does change.
//! * Only answers are cached: a body, or "not found" (HTTP 404). Timeouts, rate limits and server
//!   errors never are, so a source that is down is asked again.
//! * The cache is capped in total size, per entry and by age; the oldest entries go first.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;

use crate::fetch::{FetchError, Fetcher};

const MAGIC: &str = "BOOKX-CACHE 1";

/// Limits of the cache.
#[derive(Debug, Clone)]
pub struct CacheLimits {
    /// All entries together.
    pub max_bytes: u64,
    /// One response; larger ones are passed through uncached.
    pub max_entry: u64,
    /// Whatever the source says, nothing is kept longer than this.
    pub max_age_secs: u64,
}

impl Default for CacheLimits {
    fn default() -> Self {
        CacheLimits {
            max_bytes: 32 * 1024 * 1024,
            max_entry: 4 * 1024 * 1024,
            max_age_secs: 24 * 3600,
        }
    }
}

/// How long a response of `url` may be reused, in seconds.
pub fn ttl_secs(url: &str, negative: bool) -> u64 {
    if negative {
        return 3600; // "not found" is checked again after an hour
    }
    match crate::health::source_of(url) {
        // Google's terms do not allow keeping results for long
        Some("google") => 6 * 3600,
        Some("openlibrary") | Some("wikidata") => 24 * 3600,
        _ => 3600,
    }
}

/// Stable 64-bit FNV-1a, used for file names (the std hasher is not stable across versions).
fn fnv(data: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in data.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The file name for `url`.
pub fn key_of(url: &str) -> String {
    // two passes with different seeds make an accidental clash very unlikely
    let a = fnv(url);
    let b = fnv(&format!("{a:x}|{url}"));
    format!("{a:016x}{b:016x}.resp")
}

/// Counters, for the status line and the tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
}

/// A [`Fetcher`] that answers from the cache when it can.
pub struct CachingFetcher {
    inner: Arc<dyn Fetcher>,
    dir: PathBuf,
    limits: CacheLimits,
    now: Arc<dyn Fn() -> u64 + Send + Sync>,
    disabled: bool,
    hits: AtomicU64,
    misses: AtomicU64,
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl CachingFetcher {
    pub fn new(inner: Arc<dyn Fetcher>, dir: &Path) -> Self {
        Self::with_clock(inner, dir, CacheLimits::default(), Arc::new(unix_now))
    }

    /// With limits and a clock (seconds since the epoch), for tests.
    pub fn with_clock(
        inner: Arc<dyn Fetcher>,
        dir: &Path,
        limits: CacheLimits,
        now: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Self {
        CachingFetcher {
            inner,
            dir: dir.to_path_buf(),
            limits,
            now,
            disabled: false,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    /// A cache that never reads or writes anything: every request goes through.
    pub fn disabled(inner: Arc<dyn Fetcher>) -> Self {
        let mut c = Self::new(inner, Path::new(""));
        c.disabled = true;
        c
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
        }
    }

    fn path(&self, url: &str) -> PathBuf {
        self.dir.join(key_of(url))
    }

    /// `Some(Ok(body))`, `Some(Err(404))` for a remembered "not found", or `None`.
    fn lookup(&self, url: &str) -> Option<std::result::Result<Vec<u8>, ()>> {
        let raw = std::fs::read(self.path(url)).ok()?;
        let entry = parse(&raw)?;
        if entry.url != url || entry.expires <= (self.now)() {
            let _ = std::fs::remove_file(self.path(url));
            return None;
        }
        Some(if entry.negative {
            Err(())
        } else {
            Ok(entry.body)
        })
    }

    fn store(&self, url: &str, body: Option<&[u8]>) {
        let now = (self.now)();
        let negative = body.is_none();
        let ttl = ttl_secs(url, negative).min(self.limits.max_age_secs);
        let body = body.unwrap_or(&[]);
        if body.len() as u64 > self.limits.max_entry {
            return;
        }
        let mut out = format!(
            "{MAGIC}\n{}\n{}\n{}\n{}\n",
            now + ttl,
            now,
            if negative { "N" } else { "B" },
            url
        )
        .into_bytes();
        out.extend_from_slice(body);
        if std::fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        let tmp = self.dir.join(format!("{}.tmp", key_of(url)));
        if std::fs::write(&tmp, &out).is_ok() && std::fs::rename(&tmp, self.path(url)).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        self.trim();
    }

    /// Removes expired entries, then the oldest ones until the cache fits. Returns how many went.
    pub fn trim(&self) -> usize {
        let now = (self.now)();
        let mut entries: Vec<(PathBuf, u64, u64)> = Vec::new(); // path, created, size
        let mut removed = 0;
        for e in std::fs::read_dir(&self.dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "resp") {
                continue;
            }
            let Ok(raw) = std::fs::read(&p) else { continue };
            match parse(&raw) {
                Some(en) if en.expires > now && en.created + self.limits.max_age_secs > now => {
                    entries.push((p, en.created, raw.len() as u64));
                }
                _ => {
                    if std::fs::remove_file(&p).is_ok() {
                        removed += 1;
                    }
                }
            }
        }
        entries.sort_by_key(|e| e.1);
        let mut total: u64 = entries.iter().map(|e| e.2).sum();
        for (p, _, size) in entries {
            if total <= self.limits.max_bytes {
                break;
            }
            if std::fs::remove_file(&p).is_ok() {
                total -= size;
                removed += 1;
            }
        }
        removed
    }

    /// Deletes everything (the user asked to clear the cache). Returns the number of entries.
    pub fn clear(&self) -> usize {
        let mut n = 0;
        for e in std::fs::read_dir(&self.dir).into_iter().flatten().flatten() {
            if e.path().extension().is_some_and(|x| x == "resp")
                && std::fs::remove_file(e.path()).is_ok()
            {
                n += 1;
            }
        }
        n
    }
}

struct Entry {
    expires: u64,
    created: u64,
    negative: bool,
    url: String,
    body: Vec<u8>,
}

fn parse(raw: &[u8]) -> Option<Entry> {
    let mut rest = raw;
    let mut line = || -> Option<String> {
        let i = rest.iter().position(|b| *b == b'\n')?;
        let l = String::from_utf8(rest[..i].to_vec()).ok()?;
        rest = &rest[i + 1..];
        Some(l)
    };
    if line()? != MAGIC {
        return None;
    }
    let expires = line()?.parse().ok()?;
    let created = line()?.parse().ok()?;
    let negative = line()? == "N";
    let url = line()?;
    Some(Entry {
        expires,
        created,
        negative,
        url,
        body: rest.to_vec(),
    })
}

impl Fetcher for CachingFetcher {
    fn get(&self, url: &str, headers: &[(String, String)]) -> Result<Vec<u8>> {
        if self.disabled {
            return self.inner.get(url, headers);
        }
        match self.lookup(url) {
            Some(Ok(body)) => {
                self.hits.fetch_add(1, Ordering::Relaxed);
                return Ok(body);
            }
            Some(Err(())) => {
                self.hits.fetch_add(1, Ordering::Relaxed);
                return Err(FetchError::Http(404).into());
            }
            None => {}
        }
        self.misses.fetch_add(1, Ordering::Relaxed);
        match self.inner.get(url, headers) {
            Ok(body) => {
                self.store(url, Some(&body));
                Ok(body)
            }
            Err(e) => {
                if matches!(FetchError::of(&e), Some(FetchError::Http(404))) {
                    self.store(url, None);
                }
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::{MockFetcher, MockReply};
    use std::sync::atomic::AtomicUsize;

    /// Counts the requests that reach the "network".
    struct Counting {
        inner: MockFetcher,
        n: AtomicUsize,
    }

    impl Fetcher for Counting {
        fn get(&self, url: &str, h: &[(String, String)]) -> Result<Vec<u8>> {
            self.n.fetch_add(1, Ordering::SeqCst);
            self.inner.get(url, h)
        }
    }

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir()
            .join(format!("bookx-cache-{}", std::process::id()))
            .join(format!(
                "{:x}",
                fnv(&format!("{:?}", std::time::Instant::now()))
            ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const OL: &str = "https://openlibrary.org/search.json?q=dune";
    const GB: &str = "https://www.googleapis.com/books/v1/volumes?q=dune";

    fn setup(limits: CacheLimits) -> (Arc<Counting>, CachingFetcher, Arc<AtomicU64>, PathBuf) {
        let mock = MockFetcher::new()
            .on_prefix("https://openlibrary.org/", MockReply::Body(b"ol".to_vec()))
            .on_prefix(
                "https://www.googleapis.com/",
                MockReply::Body(b"gb".to_vec()),
            )
            .on_prefix("https://nothing.example/", MockReply::Status(404))
            .on_prefix("https://down.example/", MockReply::Status(503));
        let inner = Arc::new(Counting {
            inner: mock,
            n: AtomicUsize::new(0),
        });
        let clock = Arc::new(AtomicU64::new(1_000_000));
        let c2 = clock.clone();
        let dir = tmp();
        let f = CachingFetcher::with_clock(
            inner.clone(),
            &dir,
            limits,
            Arc::new(move || c2.load(Ordering::SeqCst)),
        );
        (inner, f, clock, dir)
    }

    #[test]
    fn the_second_request_does_not_reach_the_network() {
        let (net, f, _, _) = setup(CacheLimits::default());
        assert_eq!(f.get(OL, &[]).unwrap(), b"ol");
        assert_eq!(f.get(OL, &[]).unwrap(), b"ol");
        assert_eq!(f.get(GB, &[]).unwrap(), b"gb");
        assert_eq!(net.n.load(Ordering::SeqCst), 2);
        assert_eq!(f.stats(), CacheStats { hits: 1, misses: 2 });
    }

    #[test]
    fn entries_expire_by_source_and_the_google_ttl_is_shorter() {
        let (net, f, clock, _) = setup(CacheLimits::default());
        f.get(OL, &[]).unwrap();
        f.get(GB, &[]).unwrap();
        clock.fetch_add(7 * 3600, Ordering::SeqCst);
        f.get(OL, &[]).unwrap(); // still fresh after 7 hours
        assert_eq!(net.n.load(Ordering::SeqCst), 2);
        f.get(GB, &[]).unwrap(); // 6 hours are over
        assert_eq!(net.n.load(Ordering::SeqCst), 3);
        clock.fetch_add(25 * 3600, Ordering::SeqCst);
        f.get(OL, &[]).unwrap();
        assert_eq!(net.n.load(Ordering::SeqCst), 4);
        assert!(ttl_secs(OL, false) <= 24 * 3600 && ttl_secs(GB, false) < ttl_secs(OL, false));
    }

    #[test]
    fn not_found_is_remembered_briefly_but_failures_never() {
        let (net, f, clock, _) = setup(CacheLimits::default());
        let nf = "https://nothing.example/x";
        let e = f.get(nf, &[]).unwrap_err();
        assert_eq!(FetchError::of(&e), Some(&FetchError::Http(404)));
        let e = f.get(nf, &[]).unwrap_err();
        assert_eq!(FetchError::of(&e), Some(&FetchError::Http(404)));
        assert_eq!(net.n.load(Ordering::SeqCst), 1);
        clock.fetch_add(3601, Ordering::SeqCst);
        f.get(nf, &[]).unwrap_err();
        assert_eq!(net.n.load(Ordering::SeqCst), 2, "asked again after an hour");
        let down = "https://down.example/x";
        f.get(down, &[]).unwrap_err();
        f.get(down, &[]).unwrap_err();
        assert_eq!(
            net.n.load(Ordering::SeqCst),
            4,
            "a server error is not cached"
        );
    }

    #[test]
    fn the_cache_is_capped_in_size_and_the_oldest_go_first() {
        let limits = CacheLimits {
            max_bytes: 400,
            max_entry: 150,
            max_age_secs: 24 * 3600,
        };
        let (net, f, clock, dir) = setup(limits);
        let mock = MockFetcher::new()
            .on_prefix("https://openlibrary.org/", MockReply::Body(vec![b'x'; 100]));
        let f2 = CachingFetcher::with_clock(Arc::new(mock), &dir, f.limits.clone(), {
            let c = clock.clone();
            Arc::new(move || c.load(Ordering::SeqCst))
        });
        let _ = net;
        for i in 0..6 {
            clock.fetch_add(10, Ordering::SeqCst);
            f2.get(&format!("https://openlibrary.org/{i}"), &[])
                .unwrap();
        }
        let files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        let total: u64 = files.iter().map(|e| e.metadata().unwrap().len()).sum();
        assert!(total <= 400, "{total}");
        assert!(files.len() >= 2 && files.len() < 6);
        // the newest is still there, the first is not
        assert!(f2.path("https://openlibrary.org/5").exists());
        assert!(!f2.path("https://openlibrary.org/0").exists());
        // a response over the per-entry cap passes through uncached
        let big = MockFetcher::new()
            .on_prefix("https://openlibrary.org/", MockReply::Body(vec![b'y'; 500]));
        let f3 = CachingFetcher::with_clock(Arc::new(big), &dir, f.limits.clone(), Arc::new(|| 1));
        f3.get("https://openlibrary.org/big", &[]).unwrap();
        assert!(!f3.path("https://openlibrary.org/big").exists());
    }

    #[test]
    fn a_disabled_cache_passes_everything_through() {
        let (net, f, _, _) = setup(CacheLimits::default());
        let off = CachingFetcher::disabled(net.clone());
        off.get(OL, &[]).unwrap();
        off.get(OL, &[]).unwrap();
        assert_eq!(net.n.load(Ordering::SeqCst), 2);
        assert_eq!(off.stats(), CacheStats::default());
        assert_eq!(off.clear(), 0);
        let _ = f;
    }

    #[test]
    fn damaged_and_foreign_files_are_ignored_and_clear_empties_the_cache() {
        let (net, f, _, dir) = setup(CacheLimits::default());
        std::fs::write(f.path(OL), b"garbage").unwrap();
        assert_eq!(f.get(OL, &[]).unwrap(), b"ol");
        assert_eq!(net.n.load(Ordering::SeqCst), 1);
        std::fs::write(dir.join("notes.txt"), b"keep").unwrap();
        assert_eq!(f.clear(), 1);
        assert!(dir.join("notes.txt").exists());
    }

    #[test]
    fn keys_are_stable() {
        assert_eq!(key_of("a"), key_of("a"));
        assert_ne!(key_of("a"), key_of("b"));
        assert_eq!(
            key_of("https://x/y"),
            "459fc4b476f0279aa91912f10f02d657.resp"
        );
    }
}
