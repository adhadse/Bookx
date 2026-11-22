//! Performance budgets of comic reading. Same rules as `crates/bookx-core/tests/perf_budgets.rs`:
//! release budgets, debug gets 30x, `BOOKX_PERF_SCALE` scales them (`make perf` runs them in release).

use std::time::{Duration, Instant};

use bookx_comic::decode::{Opts, decode_page, dimensions};
use bookx_comic::layout::{SpreadOpts, place_spread, plan_spreads, strip};
use bookx_comic::panels::{Opts as PanelOpts, detect};
use bookx_comic::testgen::{Layout, jpeg, page_gray};

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

#[test]
fn decoding_a_4000_by_6000_page_with_panels() {
    let bytes = jpeg(4000, 6000, Layout::Mixed, 255);
    let t = Instant::now();
    let p = decode_page(&bytes, &Opts::default()).unwrap();
    let took = t.elapsed();
    assert_eq!((p.width, p.height), (2730, 4096));
    assert_eq!(p.panels.len(), 6);
    check("decode + reduce + panels of 4000x6000", took, budget(1500));
}

#[test]
fn panel_detection_alone_is_fast() {
    let g = page_gray(2730, 4096, Layout::Grid(2, 3), 255);
    let t = Instant::now();
    let p = detect(&g, false, PanelOpts::default());
    check(
        "panel detection on a 2730x4096 page",
        t.elapsed(),
        budget(120),
    );
    assert_eq!(p.len(), 6);
}

#[test]
fn reading_the_sizes_of_500_pages_and_laying_them_out() {
    let one = jpeg(400, 600, Layout::Splash, 255);
    let t = Instant::now();
    let sizes: Vec<(u32, u32)> = (0..500).map(|_| dimensions(&one).unwrap()).collect();
    let plan = plan_spreads(
        &sizes,
        &[],
        SpreadOpts {
            two_page: true,
            cover_alone: true,
            wide_alone: true,
        },
    );
    let (_, _) = place_spread(&sizes, &plan[10], true);
    let (p, _) = strip(&sizes);
    check(
        "sizes + spreads + strip of 500 pages",
        t.elapsed(),
        budget(150),
    );
    assert_eq!(plan.len(), 251);
    assert_eq!(p.len(), 500);
}

#[test]
fn preloading_the_pages_around_the_reader() {
    use bookx_comic::worker::{Job, Source, Worker};
    use std::sync::Arc;
    struct S(Vec<Vec<u8>>);
    impl Source for S {
        fn count(&self) -> usize {
            self.0.len()
        }
        fn bytes(&self, i: usize) -> Option<&[u8]> {
            self.0.get(i).map(|v| v.as_slice())
        }
    }
    let page = jpeg(1600, 2400, Layout::Grid(2, 2), 255);
    let w = Worker::new(Arc::new(S(vec![page; 6])), 2);
    let t = Instant::now();
    w.request(
        (0..6)
            .map(|p| Job {
                page: p,
                tag: 0,
                opts: Opts::default(),
            })
            .collect(),
    );
    for _ in 0..6 {
        assert!(w.recv_timeout(60_000).unwrap().result.is_ok());
    }
    check(
        "six 1600x2400 pages decoded on two threads",
        t.elapsed(),
        budget(1200),
    );
}
