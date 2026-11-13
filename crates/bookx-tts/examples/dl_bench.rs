//! Times the download of the default Kokoro voice (runtime + model + voices) from the real
//! sources into a fresh folder and prints the throughput:
//!
//!     cargo run --release -p bookx-tts --example dl_bench -- /tmp/bench-dir [pack-id ...]
//!
//! Without pack ids the whole default plan is downloaded; `BENCH_ONLY=1` skips the requirements of
//! the named packs. `BENCH_SECS=n` stops after n seconds (the throughput is still printed).
//! `BENCH_ORIGIN=github` uses only the GitHub origin of the runtime.

use std::time::Instant;

use bookx_tts::download::install_all;
use bookx_tts::models::{catalog, install_plan, Store};
use bookx_tts::synth::Cancel;

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let dir = a.get(1).expect("usage: dl_bench <dir> [pack ...]");
    let store = Store::new(dir);
    let all = catalog();
    let ids: Vec<&str> = if a.len() > 2 {
        a[2..].iter().map(String::as_str).collect()
    } else {
        vec!["kokoro-int8"]
    };
    let mut packs: Vec<bookx_tts::models::Pack> = Vec::new();
    for id in ids {
        let plan = if std::env::var("BENCH_ONLY").is_ok() {
            all.iter().filter(|p| p.id == id).cloned().collect()
        } else {
            install_plan(&all, &store, id)
        };
        for p in plan {
            if !packs.iter().any(|q: &bookx_tts::models::Pack| q.id == p.id) {
                packs.push(p);
            }
        }
    }
    // BENCH_ORIGIN=github: only the GitHub origin of the runtime (the way it was before the NuGet origin)
    if std::env::var("BENCH_ORIGIN").as_deref() == Ok("github") {
        for p in packs.iter_mut() {
            for f in p.files.iter_mut() {
                if let Some(o) = f.origins().get(1).cloned() {
                    f.url = o.url;
                    f.sha256 = o.sha256;
                    f.size = o.size;
                    f.extract = o.extract;
                    f.fallbacks.clear();
                }
            }
        }
    }
    let total: u64 = packs.iter().map(|p| p.download_size()).sum();
    println!(
        "plan: {} ({:.0} MB)",
        packs
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        total as f64 / 1e6
    );
    let t = Instant::now();
    let mut last = Instant::now();
    let cancel = Cancel::new();
    let limit: Option<u64> = std::env::var("BENCH_SECS")
        .ok()
        .and_then(|v| v.parse().ok());
    let mut got = 0u64;
    let r = install_all(&store, &packs, &cancel, &mut |p| {
        got = p.done;
        if limit.is_some_and(|l| t.elapsed().as_secs() >= l) {
            cancel.cancel();
        }
        if last.elapsed().as_secs() >= 2 {
            last = Instant::now();
            println!(
                "  {:5.1}%  {:.1} MB/s  {}",
                100.0 * p.done as f64 / p.total.max(1) as f64,
                p.done as f64 / 1e6 / t.elapsed().as_secs_f64(),
                p.file
            );
        }
    });
    let s = t.elapsed().as_secs_f64();
    match r {
        Ok(()) => println!("done in {s:.1} s = {:.1} MB/s", total as f64 / 1e6 / s),
        Err(e) => println!(
            "stopped after {s:.1} s ({e}): {:.2} MB = {:.3} MB/s",
            got as f64 / 1e6,
            got as f64 / 1e6 / s
        ),
    }
    Ok(())
}
