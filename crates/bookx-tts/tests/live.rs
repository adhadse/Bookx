//! Opt-in tests with the real models: `BOOKX_TTS_LIVE=1 BOOKX_TTS_DIR=~/.cache/bookx-tts cargo test
//! --release -p bookx-tts --test live -- --nocapture`. They need the packs from
//! `BOOKX_TTS_LIVE_PACK=kokoro-int8` (and `piper-de_DE-thorsten-medium`) downloaded into that folder
//! (see `download::tests::live_download_of_a_real_pack`) and `espeak-ng` installed. Without
//! `BOOKX_TTS_LIVE` they do nothing.

use std::time::{Duration, Instant};

use bookx_tts::factory;
use bookx_tts::kokoro::Kokoro;
use bookx_tts::models::{Store, kokoro_paths, piper_installed};
use bookx_tts::pipeline::PipelineEngine;
use bookx_tts::piper::Piper;
use bookx_tts::sink::MemorySink;
use bookx_tts::synth::{Cancel, SynthRequest, Synthesizer};
use bookx_tts::{Session, State, TtsPrefs, Update};

fn store() -> Option<Store> {
    std::env::var("BOOKX_TTS_LIVE").ok()?;
    let s = Store::new(std::env::var("BOOKX_TTS_DIR").ok()?);
    s.runtime_lib()?;
    bookx_tts::phonemize::espeak_program()?;
    Some(s)
}

fn say(s: &mut dyn Synthesizer, text: &str, voice: &str, lang: &str) -> bookx_tts::synth::Pcm {
    let c = Cancel::new();
    let t = Instant::now();
    let pcm = s
        .synthesize(&SynthRequest {
            text,
            voice,
            language: lang,
            speed: 1.0,
            cancel: &c,
        })
        .unwrap();
    let rtf = t.elapsed().as_secs_f64() / pcm.duration_secs();
    println!(
        "{} chars -> {:.2} s audio, rms {:.3}, rtf {:.3}",
        text.len(),
        pcm.duration_secs(),
        pcm.rms(),
        rtf
    );
    pcm
}

#[test]
fn kokoro_speaks_english_and_french() {
    let Some(store) = store() else { return };
    let Some((model, voices)) = kokoro_paths(&store, "int8") else {
        return;
    };
    let mut k = Kokoro::load(&store.runtime_lib().unwrap(), &model, &voices).unwrap();
    let en = say(
        &mut k,
        "It was the best of times, it was the worst of times.",
        "af_heart",
        "en",
    );
    assert_eq!(en.rate, 24000);
    assert!(
        (2.0..8.0).contains(&en.duration_secs()),
        "{}",
        en.duration_secs()
    );
    assert!(
        en.rms() > 0.01 && en.rms() < 0.5,
        "not silent, not clipping: {}",
        en.rms()
    );
    let fr = say(
        &mut k,
        "Il était une fois un homme qui ne savait pas quoi faire.",
        "",
        "fr",
    );
    assert!(fr.rms() > 0.01);
    // a long sentence is cut into pieces and joined
    let long = "word ".repeat(120);
    let l = say(&mut k, &long, "bf_emma", "en");
    assert!(l.duration_secs() > 20.0);
}

#[test]
fn piper_speaks_german() {
    let Some(store) = store() else { return };
    let voices = piper_installed(&store);
    if voices.is_empty() {
        return;
    }
    let mut p = Piper::new(&store.runtime_lib().unwrap(), voices).unwrap();
    let de = say(
        &mut p,
        "Es war einmal ein Mann, der nicht wusste, was er tun sollte.",
        "",
        "de",
    );
    assert_eq!(de.rate, 22050);
    assert!(de.rms() > 0.01 && de.rms() < 0.5);
    assert!(
        p.synthesize(&SynthRequest {
            text: "Bonjour",
            voice: "",
            language: "fr",
            speed: 1.0,
            cancel: &Cancel::new()
        })
        .is_err()
    );
}

/// The whole stack: the factory opens Kokoro, a session reads three sentences, the sink gets
/// all the samples, the events come in order.
#[test]
fn a_session_reads_with_the_real_engine() {
    let Some(store) = store() else { return };
    if kokoro_paths(&store, "int8").is_none() {
        return;
    }
    let prefs = TtsPrefs {
        engine: "kokoro".into(),
        ..Default::default()
    };
    let opened = factory::open(&prefs, &store, "en", None).unwrap();
    assert_eq!(opened.choice, Some(factory::Choice::Kokoro));
    // the factory's engine plays through the default player; build the same pipeline on a memory sink
    drop(opened);
    let (model, voices) = kokoro_paths(&store, "int8").unwrap();
    let lib = store.runtime_lib().unwrap();
    let sink = MemorySink::default();
    let mut e = PipelineEngine::new(
        "Kokoro",
        Box::new(Kokoro::load(&lib, &model, &voices).unwrap()),
        Box::new(sink.clone()),
        None,
        3,
    );
    use bookx_tts::Engine;
    e.set_language("en");
    let mut s = Session::new();
    s.load(
        &mut e,
        "The first sentence is short. The second one is a little longer than that. Third.",
    );
    s.play_from(&mut e, 0);
    let mut got = Vec::new();
    let t = Instant::now();
    while s.state() != State::Stopped && t.elapsed() < Duration::from_secs(60) {
        got.extend(s.tick(&mut e));
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(got.last(), Some(&Update::Finished), "{got:?}");
    assert_eq!(
        got.iter()
            .filter(|u| matches!(u, Update::Speaking(..)))
            .count(),
        3
    );
    let samples = sink.written.lock().unwrap().len();
    println!(
        "sink got {samples} samples = {:.1} s, session took {:.1} s",
        samples as f64 / 24000.0,
        t.elapsed().as_secs_f64()
    );
    assert!(samples > 24000 * 4);
}

/// The bug of the multi-book owner: the second engine of a run failed with "only one environment is
/// allowed per process". Two engines opened one after the other (two books, or Kokoro then Piper) and a
/// service hand-off between sessions, with the real ONNX Runtime and the real Kokoro model.
#[test]
fn several_engines_and_books_in_one_process_work() {
    let Some(store) = store() else { return };
    let Some((model, voices)) = kokoro_paths(&store, "int8") else {
        return;
    };
    let lib = store.runtime_lib().unwrap();
    // two direct loads: the old code failed on the second one
    let mut a = Kokoro::load(&lib, &model, &voices).unwrap();
    let mut b = Kokoro::load(&lib, &model, &voices).expect("a second engine in the same process");
    assert!(say(&mut a, "First book.", "af_heart", "en").rms() > 0.01);
    assert!(say(&mut b, "Second book.", "af_heart", "en").rms() > 0.01);
    drop((a, b));

    // book A, back to the library, book B, back, book A again: through the factory, as the app does it
    let prefs = TtsPrefs::default();
    let mut times = Vec::new();
    for book in ["A", "B", "A"] {
        let t = Instant::now();
        let opened = factory::open(&prefs, &store, "en", None)
            .unwrap_or_else(|e| panic!("book {book}: {e:#}"));
        let mut engine = opened.engine;
        let mut s = Session::new();
        s.load(engine.as_mut(), "It was a dark night. The rain fell.");
        assert!(s.play_from(engine.as_mut(), 0).is_none());
        let end = Instant::now() + Duration::from_secs(120);
        let mut spoke = 0;
        while Instant::now() < end && s.state() != State::Stopped {
            for u in s.tick(engine.as_mut()) {
                match u {
                    Update::Speaking(..) => spoke += 1,
                    Update::Error(e) => panic!("book {book}: {e}"),
                    Update::Finished => {}
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(spoke >= 2, "book {book}: two sentences were read ({spoke})");
        times.push(t.elapsed().as_secs_f64());
        drop(s);
        drop(engine); // back to the library
    }
    println!("times for A, B, A: {times:?} (the model is loaded once)");
    assert!(
        times[1] < times[0] * 0.8 + 1.0,
        "the second book reuses the loaded model: {times:?}"
    );
}

/// The root cause itself, with the raw library: ONNX Runtime refuses a second environment. (This is
/// what every second engine of a run used to hit; `onnx::environment` now shares the first one.)
#[test]
fn onnx_runtime_allows_one_environment_per_process() {
    let Some(store) = store() else { return };
    let lib = store.runtime_lib().unwrap();
    // a fresh process state is needed for the first build to succeed: this test binary may already
    // have one from another test, which is the same situation the app was in for its second book
    let first = ort::init_from(&lib).unwrap().with_name("raw").build();
    let second = ort::init_from(&lib).unwrap().with_name("raw2").build();
    let err = second
        .expect_err("a second raw environment is refused")
        .to_string();
    assert!(err.contains("only one environment"), "{err}");
    let _ = first;
}
