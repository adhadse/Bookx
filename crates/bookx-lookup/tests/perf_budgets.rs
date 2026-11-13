//! Performance budgets for lookup, in-book search and statistics (release budgets, debug 30x,
//! `BOOKX_PERF_SCALE` scales them).

use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use bookx_lookup::search::{search, SearchOptions};
use bookx_lookup::stats::{daily_totals, Session, StatsConfig};
use chrono::{TimeZone, Utc};

fn budget(release_ms: u64) -> Duration {
    let scale: f64 = std::env::var("BOOKX_PERF_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let f = if cfg!(debug_assertions) { 30.0 } else { 1.0 };
    Duration::from_micros((release_ms as f64 * 1000.0 * f * scale) as u64)
}

fn check(what: &str, took: Duration, max: Duration) {
    assert!(took <= max, "{what}: took {took:?}, budget {max:?}");
}

fn lorem(bytes: usize) -> String {
    let base =
        "The quick brown fox jumps over the lazy dog while reading a long book about typography. ";
    base.repeat(bytes / base.len() + 1)
}

#[test]
fn searching_a_megabyte_of_text_is_fast() {
    let chapters: Vec<String> = (0..20).map(|_| lorem(50_000)).collect();
    let never = AtomicBool::new(false);
    let opts = SearchOptions {
        max_results: 100,
        ..SearchOptions::default()
    };
    let t = Instant::now();
    let r = search(&chapters, "typography", &opts, Some(&never)).unwrap();
    check("literal search over 1 MB", t.elapsed(), budget(40));
    assert_eq!(r.hits.len(), 100);

    let opts = SearchOptions {
        regex: true,
        max_results: 100,
        ..SearchOptions::default()
    };
    let t = Instant::now();
    let r = search(&chapters, r"qu\w+k\s+brown", &opts, Some(&never)).unwrap();
    check("regex search over 1 MB", t.elapsed(), budget(80));
    assert_eq!(r.hits.len(), 100);
}

#[test]
fn aggregating_20k_sessions_is_fast() {
    let start = Utc.with_ymd_and_hms(2024, 1, 1, 8, 0, 0).unwrap();
    let sessions: Vec<Session> = (0..20_000)
        .map(|i| {
            let s = start + chrono::Duration::minutes(i * 90);
            Session {
                book_id: format!("b{}", i % 50),
                start: s,
                end: s + chrono::Duration::minutes(30),
                pages: 10,
                words: 2500,
                characters: 15_000,
                start_pos: 0.1,
                end_pos: 0.2,
            }
        })
        .collect();
    let t = Instant::now();
    let days = daily_totals(&sessions, &StatsConfig::default(), &Utc);
    check("daily totals of 20 000 sessions", t.elapsed(), budget(150));
    assert!(days.len() > 1000);
}
