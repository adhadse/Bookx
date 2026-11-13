//! Performance budgets. Budgets are for release builds, debug builds get 30x room and
//! `BOOKX_PERF_SCALE` scales every budget (like `bookx-core/tests/perf_budgets.rs`).

mod common;

use std::time::{Duration, Instant};

use common::*;
use serde_json::json;

fn budget(ms: u64) -> Duration {
    let debug = if cfg!(debug_assertions) { 30.0 } else { 1.0 };
    let scale: f64 = std::env::var("BOOKX_PERF_SCALE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0);
    Duration::from_secs_f64(ms as f64 / 1000.0 * debug * scale)
}

fn timed<T>(name: &str, ms: u64, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let out = f();
    let took = t.elapsed();
    assert!(
        took <= budget(ms),
        "{name} took {took:?}, budget {:?}",
        budget(ms)
    );
    out
}

#[test]
fn thousand_trivial_calls() {
    let p = plugin("transform", "", &always(&ok("1")));
    timed("1000 trivial calls", 500, || {
        for _ in 0..1000 {
            p.call_json("x", &json!({})).unwrap();
        }
    });
}

#[test]
fn thousand_echo_calls_with_payload() {
    let p = plugin("transform", "", &echo());
    let payload = json!({"title": "x".repeat(1000), "authors": ["a", "b"]});
    timed("1000 echo calls", 800, || {
        for _ in 0..1000 {
            assert_eq!(p.call_json("x", &payload).unwrap(), payload);
        }
    });
}

#[test]
fn loading_a_plugin_is_cheap() {
    let wasm = echo();
    timed("100 loads", 500, || {
        for _ in 0..100 {
            let _ = plugin("transform", "", &wasm);
        }
    });
}

#[test]
fn endless_loop_is_cut_off_quickly() {
    let p = plugin("transform", "fuel = 5000000", &infinite_loop());
    timed("fuel exhaustion", 1000, || {
        assert!(p.call_raw("x", b"").is_err());
    });
}

#[test]
fn a_patch_over_a_large_book_is_fast() {
    use bookx_core::archive::Archive;
    use bookx_plugin::Patch;
    let patch = Patch::parse(
        "hook = \"convert-before\"\n[[regex]]\nfind = '\\s{2,}'\nreplace = ' '\n[[regex]]\nfind = '[“”]'\nreplace = '\"'\n[[css]]\nselector = 'p'\ndeclarations = 'margin: 0'\n",
        "perf",
    )
    .unwrap();
    let mut a = Archive::default();
    let para = "<p>He said “hi”   and   left.  </p>\n".repeat(400);
    for i in 0..200 {
        a.set(&format!("OEBPS/ch{i}.xhtml"), para.clone().into_bytes());
    }
    a.set("OEBPS/style.css", b"body{}".to_vec());
    let r = timed("patch 200 chapters", 1500, || patch.apply(&mut a));
    assert_eq!(r.files.len(), 201);
}
