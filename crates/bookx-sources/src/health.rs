//! Health of the metadata sources (roadmap PP-G-1): which source answers, which is slow, which
//! is failing and for how long it is left alone.
//!
//! [`HealthBoard`] is the bookkeeping (pure, the clock is passed in). [`HealthFetcher`] wraps any
//! [`Fetcher`], times every request, files it under the source its host belongs to and refuses
//! requests to a source that is in backoff, so one dead source never slows a bulk lookup down.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Result;

use crate::fetch::{FetchError, Fetcher, backoff_delay_ms};

/// How a source is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Nothing asked yet.
    Unknown,
    Ok,
    /// Answers, but the recent answers took long.
    Slow,
    /// The last requests failed; in backoff after [`FAILS_BEFORE_BACKOFF`].
    Failing,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Unknown => "not used yet",
            Status::Ok => "ok",
            Status::Slow => "slow",
            Status::Failing => "failing",
        }
    }
}

/// Failures in a row after which a source is left alone for a while.
pub const FAILS_BEFORE_BACKOFF: u32 = 3;
/// Average of the last answers above which a source is called slow.
pub const SLOW_MS: u64 = 4000;
const BASE_BACKOFF_MS: u64 = 30_000;
const CAP_BACKOFF_MS: u64 = 15 * 60_000;
const WINDOW: usize = 5;

#[derive(Debug, Clone, Default)]
struct State {
    ok: u32,
    failed: u32,
    in_a_row: u32,
    recent: VecDeque<u64>,
    last_error: String,
    retry_at_ms: u64,
}

/// One line of the health display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceReport {
    pub source: String,
    pub status: Status,
    pub ok: u32,
    pub failed: u32,
    pub avg_ms: u64,
    /// Milliseconds until requests are sent again; 0 when the source is not paused.
    pub retry_in_ms: u64,
    pub last_error: String,
}

impl SourceReport {
    pub fn describe(&self) -> String {
        let mut s = format!("{}: {}", display_name(&self.source), self.status.label());
        if self.retry_in_ms > 0 {
            s += &format!(" (paused, retry in {}s)", self.retry_in_ms.div_ceil(1000));
        }
        s
    }
}

/// Bookkeeping for all sources. Cheap to share (`Arc`).
pub struct HealthBoard {
    inner: Mutex<BTreeMap<String, State>>,
    epoch: Instant,
}

impl Default for HealthBoard {
    fn default() -> Self {
        HealthBoard {
            inner: Mutex::default(),
            epoch: Instant::now(),
        }
    }
}

impl HealthBoard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Milliseconds since the board was made: the clock the backoff is measured on.
    pub fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    /// [`HealthBoard::snapshot`] at the current time.
    pub fn snapshot_now(&self) -> Vec<SourceReport> {
        self.snapshot(self.now_ms())
    }

    fn with<R>(&self, f: impl FnOnce(&mut BTreeMap<String, State>) -> R) -> R {
        f(&mut self.inner.lock().unwrap_or_else(|p| p.into_inner()))
    }

    pub fn record_ok(&self, source: &str, latency_ms: u64) {
        self.with(|m| {
            let s = m.entry(source.to_string()).or_default();
            s.ok += 1;
            s.in_a_row = 0;
            s.retry_at_ms = 0;
            s.recent.push_back(latency_ms);
            while s.recent.len() > WINDOW {
                s.recent.pop_front();
            }
        });
    }

    pub fn record_failure(&self, source: &str, error: &str, now_ms: u64) {
        self.with(|m| {
            let s = m.entry(source.to_string()).or_default();
            s.failed += 1;
            s.in_a_row += 1;
            s.last_error = error.to_string();
            if s.in_a_row >= FAILS_BEFORE_BACKOFF {
                let delay = backoff_delay_ms(
                    s.in_a_row - FAILS_BEFORE_BACKOFF,
                    BASE_BACKOFF_MS,
                    CAP_BACKOFF_MS,
                );
                s.retry_at_ms = now_ms.saturating_add(delay);
            }
        });
    }

    /// `Ok` when a request may go out, else the milliseconds to wait.
    pub fn allow(&self, source: &str, now_ms: u64) -> std::result::Result<(), u64> {
        self.with(|m| match m.get(source) {
            Some(s) if s.retry_at_ms > now_ms => Err(s.retry_at_ms - now_ms),
            _ => Ok(()),
        })
    }

    pub fn status(&self, source: &str, now_ms: u64) -> Status {
        self.report(source, now_ms)
            .map_or(Status::Unknown, |r| r.status)
    }

    pub fn report(&self, source: &str, now_ms: u64) -> Option<SourceReport> {
        self.with(|m| m.get(source).map(|s| make_report(source, s, now_ms)))
    }

    /// Every source seen so far, by name.
    pub fn snapshot(&self, now_ms: u64) -> Vec<SourceReport> {
        self.with(|m| m.iter().map(|(k, s)| make_report(k, s, now_ms)).collect())
    }

    /// Forgets everything (the user pressed *Retry now*).
    pub fn reset(&self) {
        self.with(|m| m.clear());
    }

    /// The state as JSON for the next session. `unix_secs` is the wall clock; a pause that is
    /// still running is saved as the time left.
    pub fn to_json(&self, unix_secs: u64) -> String {
        let now = self.now_ms();
        let sources = self.with(|m| {
            m.iter()
                .map(|(k, s)| {
                    (
                        k.clone(),
                        SavedState {
                            ok: s.ok,
                            failed: s.failed,
                            in_a_row: s.in_a_row,
                            recent: s.recent.iter().copied().collect(),
                            last_error: s.last_error.clone(),
                            retry_in_ms: s.retry_at_ms.saturating_sub(now),
                        },
                    )
                })
                .collect()
        });
        serde_json::to_string_pretty(&Saved {
            saved_at: unix_secs,
            sources,
        })
        .unwrap_or_default()
    }

    /// Takes the state saved by [`HealthBoard::to_json`] (damaged or older than a week: ignored).
    /// A pause continues where it left off, minus the time that passed. Returns the number of
    /// sources restored.
    pub fn load_json(&self, json: &str, unix_secs: u64) -> usize {
        let Ok(saved) = serde_json::from_str::<Saved>(json) else {
            return 0;
        };
        let age = unix_secs.saturating_sub(saved.saved_at);
        if age > 7 * 24 * 3600 {
            return 0;
        }
        let now = self.now_ms();
        let n = saved.sources.len();
        self.with(|m| {
            for (k, s) in saved.sources {
                let left = s.retry_in_ms.saturating_sub(age * 1000);
                m.insert(
                    k,
                    State {
                        ok: s.ok,
                        failed: s.failed,
                        in_a_row: s.in_a_row,
                        recent: s.recent.into_iter().take(WINDOW).collect(),
                        last_error: s.last_error,
                        retry_at_ms: if left > 0 { now + left } else { 0 },
                    },
                );
            }
        });
        n
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedState {
    ok: u32,
    failed: u32,
    in_a_row: u32,
    recent: Vec<u64>,
    last_error: String,
    retry_in_ms: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    saved_at: u64,
    sources: BTreeMap<String, SavedState>,
}

fn make_report(source: &str, s: &State, now_ms: u64) -> SourceReport {
    let avg_ms = if s.recent.is_empty() {
        0
    } else {
        s.recent.iter().sum::<u64>() / s.recent.len() as u64
    };
    let retry_in_ms = s.retry_at_ms.saturating_sub(now_ms);
    let status = if retry_in_ms > 0 || s.in_a_row >= 2 {
        Status::Failing
    } else if s.recent.len() >= 2 && avg_ms >= SLOW_MS {
        Status::Slow
    } else if s.ok + s.failed == 0 {
        Status::Unknown
    } else {
        Status::Ok
    };
    SourceReport {
        source: source.to_string(),
        status,
        ok: s.ok,
        failed: s.failed,
        avg_ms,
        retry_in_ms,
        last_error: s.last_error.clone(),
    }
}

/// The name shown to the user.
pub fn display_name(source: &str) -> &str {
    match source {
        "openlibrary" => "Open Library",
        "google" => "Google Books",
        "wikidata" => "Wikidata",
        other => other,
    }
}

/// Which source a URL belongs to (the ids of [`crate::sources`]).
pub fn source_of(url: &str) -> Option<&'static str> {
    let host = url
        .split("://")
        .nth(1)?
        .split(['/', '?', '#'])
        .next()?
        .to_ascii_lowercase();
    if host.ends_with("openlibrary.org") {
        Some("openlibrary")
    } else if host.ends_with("googleapis.com") {
        Some("google")
    } else if host.ends_with("wikidata.org") {
        Some("wikidata")
    } else {
        None
    }
}

/// Did the source misbehave? "Not found" is a proper answer, rate limits and server errors are not.
pub fn counts_as_failure(e: &anyhow::Error) -> bool {
    match FetchError::of(e) {
        Some(FetchError::Http(c)) => matches!(*c, 401 | 403 | 429) || *c >= 500,
        Some(_) => true,
        None => true,
    }
}

/// A [`Fetcher`] that keeps a [`HealthBoard`] up to date and skips sources that are in backoff.
pub struct HealthFetcher {
    inner: Arc<dyn Fetcher>,
    board: Arc<HealthBoard>,
}

impl HealthFetcher {
    pub fn new(inner: Arc<dyn Fetcher>, board: Arc<HealthBoard>) -> Self {
        HealthFetcher { inner, board }
    }

    pub fn board(&self) -> &Arc<HealthBoard> {
        &self.board
    }

    fn now_ms(&self) -> u64 {
        self.board.now_ms()
    }
}

impl Fetcher for HealthFetcher {
    fn get(&self, url: &str, headers: &[(String, String)]) -> Result<Vec<u8>> {
        let Some(source) = source_of(url) else {
            return self.inner.get(url, headers);
        };
        let start = self.now_ms();
        if let Err(wait) = self.board.allow(source, start) {
            return Err(FetchError::Other(format!(
                "{} is paused after repeated failures (retry in {}s)",
                display_name(source),
                wait.div_ceil(1000)
            ))
            .into());
        }
        let res = self.inner.get(url, headers);
        let end = self.now_ms();
        match &res {
            Ok(_) => self.board.record_ok(source, end.saturating_sub(start)),
            Err(e) if counts_as_failure(e) => {
                self.board.record_failure(source, &format!("{e:#}"), end)
            }
            Err(_) => self.board.record_ok(source, end.saturating_sub(start)),
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::{MockFetcher, MockReply};

    #[test]
    fn health_survives_a_restart_and_pauses_run_down_meanwhile() {
        let a = HealthBoard::new();
        a.record_ok("openlibrary", 200);
        for _ in 0..FAILS_BEFORE_BACKOFF {
            a.record_failure("google", "HTTP error 503", a.now_ms());
        }
        assert!(a.allow("google", a.now_ms()).is_err());
        let json = a.to_json(1_000);
        // 10 s later in a new session: 20 s of the 30 s pause are left
        let b = HealthBoard::new();
        assert_eq!(b.load_json(&json, 1_010), 2);
        let wait = b.allow("google", b.now_ms()).unwrap_err();
        assert!((19_000..=20_000).contains(&wait), "{wait}");
        let r = b.report("openlibrary", b.now_ms()).unwrap();
        assert_eq!((r.ok, r.status), (1, Status::Ok));
        assert_eq!(
            b.report("google", b.now_ms()).unwrap().last_error,
            "HTTP error 503"
        );
        // a minute later the pause is over, the counts remain
        let c = HealthBoard::new();
        c.load_json(&json, 1_060);
        assert_eq!(c.allow("google", c.now_ms()), Ok(()));
        assert_eq!(c.report("google", c.now_ms()).unwrap().failed, 3);
        // old or damaged files are ignored
        let d = HealthBoard::new();
        assert_eq!(d.load_json(&json, 1_000 + 8 * 24 * 3600), 0);
        assert_eq!(d.load_json("not json", 1_000), 0);
        assert!(d.snapshot(0).is_empty());
    }

    #[test]
    fn a_source_goes_from_ok_to_slow_to_failing() {
        let b = HealthBoard::new();
        assert_eq!(b.status("google", 0), Status::Unknown);
        b.record_ok("google", 300);
        assert_eq!(b.status("google", 0), Status::Ok);
        b.record_ok("google", 9000);
        b.record_ok("google", 9000);
        assert_eq!(b.status("google", 0), Status::Slow);
        for _ in 0..5 {
            b.record_ok("google", 100);
        }
        assert_eq!(b.status("google", 0), Status::Ok, "recovers");
        b.record_failure("google", "boom", 1000);
        assert_eq!(
            b.status("google", 1000),
            Status::Ok,
            "one failure is a blip"
        );
        b.record_failure("google", "boom", 1000);
        assert_eq!(b.status("google", 1000), Status::Failing);
    }

    #[test]
    fn repeated_failures_back_off_with_growing_delays_and_a_success_ends_it() {
        let b = HealthBoard::new();
        for _ in 0..FAILS_BEFORE_BACKOFF {
            b.record_failure("wikidata", "HTTP error 503", 10_000);
        }
        assert_eq!(b.allow("wikidata", 10_000), Err(30_000));
        assert_eq!(b.allow("wikidata", 25_000), Err(15_000));
        assert_eq!(b.allow("wikidata", 40_000), Ok(()));
        b.record_failure("wikidata", "again", 40_000);
        assert_eq!(b.allow("wikidata", 40_000), Err(60_000), "doubles");
        assert_eq!(b.allow("other", 0), Ok(()));
        b.record_ok("wikidata", 50);
        assert_eq!(b.allow("wikidata", 40_001), Ok(()));
        let r = b.report("wikidata", 40_001).unwrap();
        assert_eq!((r.ok, r.failed), (1, 4));
        assert_eq!(r.last_error, "again");
        b.reset();
        assert!(b.snapshot(0).is_empty());
    }

    #[test]
    fn urls_belong_to_their_sources() {
        assert_eq!(
            source_of("https://openlibrary.org/search.json?q=x"),
            Some("openlibrary")
        );
        assert_eq!(
            source_of("https://covers.openlibrary.org/b/id/1-L.jpg"),
            Some("openlibrary")
        );
        assert_eq!(
            source_of("https://www.googleapis.com/books/v1/volumes"),
            Some("google")
        );
        assert_eq!(
            source_of("https://query.wikidata.org/sparql?q=1"),
            Some("wikidata")
        );
        assert_eq!(source_of("https://example.com/"), None);
        assert_eq!(source_of("not a url"), None);
    }

    #[test]
    fn not_found_is_an_answer_but_rate_limits_are_failures() {
        let mock = MockFetcher::new()
            .on_prefix("https://openlibrary.org/a", MockReply::Status(404))
            .on_prefix("https://openlibrary.org/b", MockReply::Status(429))
            .on_prefix(
                "https://www.googleapis.com/",
                MockReply::Body(b"{}".to_vec()),
            );
        let board = Arc::new(HealthBoard::new());
        let f = HealthFetcher::new(Arc::new(mock), board.clone());
        assert!(f.get("https://openlibrary.org/a", &[]).is_err());
        assert_eq!(board.report("openlibrary", 0).unwrap().failed, 0);
        assert!(f.get("https://openlibrary.org/b", &[]).is_err());
        assert_eq!(board.report("openlibrary", 0).unwrap().failed, 1);
        assert!(f.get("https://www.googleapis.com/x", &[]).is_ok());
        assert_eq!(board.report("google", 0).unwrap().ok, 1);
    }

    #[test]
    fn a_source_in_backoff_is_not_asked_again() {
        let mock = Arc::new(
            MockFetcher::new().on_prefix("https://query.wikidata.org/", MockReply::Status(503)),
        );
        let board = Arc::new(HealthBoard::new());
        let f = HealthFetcher::new(mock.clone(), board.clone());
        for _ in 0..FAILS_BEFORE_BACKOFF {
            assert!(f.get("https://query.wikidata.org/x", &[]).is_err());
        }
        let asked = mock.urls().len();
        let err = f.get("https://query.wikidata.org/x", &[]).unwrap_err();
        assert!(format!("{err:#}").contains("paused"), "{err:#}");
        assert_eq!(mock.urls().len(), asked, "no request went out");
        assert_eq!(board.status("wikidata", f.now_ms()), Status::Failing);
        assert!(board.snapshot(f.now_ms())[0].describe().contains("paused"));
    }
}
