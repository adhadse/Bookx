//! 1000 sequential calls must not make the process grow. Own test binary and a single test, so
//! other tests cannot disturb the measurement.

mod common;

use common::*;

fn rss_kib() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = s.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4)
}

#[test]
fn a_thousand_calls_do_not_grow_memory() {
    let Some(_) = rss_kib() else {
        return; // not Linux
    };
    // Every call grows the guest by 40 pages (2.5 MiB) and the stateful plugin dirties its heap; a
    // leaked instance would add ~2.5 GiB over 1000 calls.
    let heavy = plugin("transform", "memory_pages = 256", &grower(40));
    let stateful = plugin("transform", "", &stateful());
    let failing = plugin("transform", "fuel = 50000", &infinite_loop());
    let run = |n: usize| {
        for _ in 0..n {
            heavy.call_raw("x", b"").unwrap();
            stateful.call_raw("x", b"").unwrap();
            assert!(failing.call_raw("x", b"").is_err());
        }
    };
    run(100); // warm up allocator pools
    let before = rss_kib().unwrap();
    run(1000);
    let after = rss_kib().unwrap();
    let growth_mib = after.saturating_sub(before) / 1024;
    assert!(
        growth_mib < 64,
        "RSS grew by {growth_mib} MiB over 1000 calls"
    );
}
