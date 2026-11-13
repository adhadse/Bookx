//! Speaks a text with a downloaded Kokoro or Piper voice into a WAV file and prints the
//! length, the loudness and the real-time factor (synthesis time / audio time):
//!
//!     BOOKX_TTS_DIR=~/.cache/bookx-tts cargo run --release -p bookx-tts --example speak -- \
//!         kokoro af_heart en out.wav "Some text to read."
//!
//! Arguments: engine (kokoro|piper), voice ("" = default), language, output, text.

use std::time::Instant;

use bookx_tts::models::{kokoro_paths, piper_installed, Store};
use bookx_tts::synth::{Cancel, SynthRequest, Synthesizer};

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 6 {
        eprintln!("usage: speak <kokoro|piper> <voice> <language> <out.wav> <text> [speed] [kokoro model int8|fp32]");
        std::process::exit(2);
    }
    let root = std::env::var("BOOKX_TTS_DIR").expect("BOOKX_TTS_DIR");
    let store = Store::new(root);
    let lib = store.runtime_lib().expect("ONNX Runtime is not downloaded");
    let speed: f64 = a.get(6).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let model = a.get(7).map(|s| s.as_str()).unwrap_or("int8");
    let t = Instant::now();
    let mut synth: Box<dyn Synthesizer> = match a[1].as_str() {
        "kokoro" => {
            let (m, v) = kokoro_paths(&store, model).expect("Kokoro is not downloaded");
            Box::new(bookx_tts::kokoro::Kokoro::load(&lib, &m, &v)?)
        }
        _ => Box::new(bookx_tts::piper::Piper::new(&lib, piper_installed(&store))?),
    };
    println!("loaded in {:.2}s", t.elapsed().as_secs_f64());
    let cancel = Cancel::new();
    for round in 0..2 {
        let t = Instant::now();
        let pcm = synth.synthesize(&SynthRequest {
            text: &a[5],
            voice: &a[2],
            language: &a[3],
            speed,
            cancel: &cancel,
        })?;
        let took = t.elapsed().as_secs_f64();
        println!(
            "run {round}: {:.2}s of audio at {} Hz, rms {:.4}, peak {:.3}, took {:.2}s, real-time factor {:.3}",
            pcm.duration_secs(),
            pcm.rate,
            pcm.rms(),
            pcm.samples.iter().fold(0.0f32, |m, s| m.max(s.abs())),
            took,
            took / pcm.duration_secs()
        );
        if round == 1 {
            std::fs::write(&a[4], pcm.to_wav())?;
        }
    }
    Ok(())
}
