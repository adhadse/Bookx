//! Performance budgets (release builds; debug gets 30x room, `BOOKX_PERF_SCALE` scales them).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bookx_audio::scan::scan_folder;
use bookx_audio::stretch::Stretcher;
use bookx_audio::testgen::wav_silence;

fn budget(release_secs: f64) -> Duration {
    let scale: f64 = std::env::var("BOOKX_PERF_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let debug = if cfg!(debug_assertions) { 30.0 } else { 1.0 };
    Duration::from_secs_f64(release_secs * scale * debug)
}

fn tmp() -> PathBuf {
    let d = std::env::temp_dir().join(format!("bookx-audio-perf-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn scanning_a_folder_of_500_chapters() {
    let d = tmp().join("Long Book");
    std::fs::create_dir_all(&d).unwrap();
    let wav = wav_silence(8000, 200);
    for n in 0..500 {
        std::fs::write(d.join(format!("{n:03} - Chapter.wav")), &wav).unwrap();
    }
    let t = Instant::now();
    let s = scan_folder(&d).unwrap();
    let took = t.elapsed();
    assert_eq!(s.book.parts.len(), 500);
    assert_eq!(s.book.chapters.len(), 500);
    assert_eq!(s.book.total_ms, 500 * 200);
    assert!(took < budget(1.5), "scanning 500 parts took {took:?}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn stretching_runs_far_faster_than_real_time() {
    // one minute of 44.1 kHz stereo at 1.5x must take well under a second of CPU
    let rate = 44_100u32;
    let frames = rate as usize * 60;
    let input: Vec<f32> = (0..frames * 2)
        .map(|i| ((i / 2) as f32 * 0.03).sin() * 0.4)
        .collect();
    let mut s = Stretcher::new(rate, 2, 1.5);
    let mut out = Vec::new();
    let t = Instant::now();
    for c in input.chunks(8192) {
        s.push(c, &mut out);
    }
    s.finish(&mut out);
    let took = t.elapsed();
    assert!(out.len() > frames / 2, "{}", out.len());
    assert!(took < budget(2.0), "stretching a minute took {took:?}");
}
