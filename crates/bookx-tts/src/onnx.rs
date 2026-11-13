//! ONNX Runtime, loaded from the library the user downloaded (so the build links nothing and
//! no model is bundled), and helpers shared by the Kokoro and Piper engines.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use ort::environment::Environment;
use ort::session::Session;

use crate::session::Voice;
use crate::synth::{Pcm, SynthRequest, Synthesizer};

/// Loads ONNX Runtime from `lib` and returns the environment sessions need.
///
/// ONNX Runtime allows **one environment per process** (a second `build()` fails with "only one
/// environment is allowed per process"), and the application has many engines over its life (one per
/// book, Kokoro and Piper, saving audio): the environment is made the first time and every later call
/// gets the same one. A different library path after the first call is not loaded (the process keeps
/// the library it started with).
pub fn environment(lib: &Path) -> Result<Environment> {
    static ENV: std::sync::Mutex<Option<Environment>> = std::sync::Mutex::new(None);
    let mut slot = ENV.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(env) = slot.as_ref() {
        return Ok(env.clone());
    }
    let env = ort::init_from(lib)
        .map_err(|e| anyhow!("cannot load ONNX Runtime from {}: {e}", lib.display()))?
        .with_name("bookx")
        .build()
        .map_err(|e| anyhow!("cannot start ONNX Runtime: {e}"))?;
    *slot = Some(env.clone());
    Ok(env)
}

/// Threads for inference: half the cores, at least 2, at most 8 (`$BOOKX_TTS_THREADS` sets
/// the number).
pub fn threads() -> usize {
    if let Some(n) = std::env::var("BOOKX_TTS_THREADS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        return n.clamp(1, 64);
    }
    std::thread::available_parallelism()
        .map(|n| n.get() / 2)
        .unwrap_or(2)
        .clamp(2, 8)
}

pub fn load_model(env: &Environment, path: &Path) -> Result<Session> {
    Session::builder(env)
        .map_err(|e| anyhow!("{e}"))?
        .with_intra_threads(threads())
        .map_err(|e| anyhow!("{e}"))?
        .commit_from_file(path)
        .map_err(|e| anyhow!("cannot load the model {}: {e}", path.display()))
}

/// Cuts silence from both ends of speech, keeping `keep_ms` of it.
pub fn trim_silence(mut pcm: Pcm, threshold: f32, keep_ms: u32) -> Pcm {
    let keep = (pcm.rate as usize * keep_ms as usize) / 1000;
    let first = pcm.samples.iter().position(|s| s.abs() > threshold);
    let last = pcm.samples.iter().rposition(|s| s.abs() > threshold);
    if let (Some(a), Some(b)) = (first, last) {
        let a = a.saturating_sub(keep);
        let b = (b + keep).min(pcm.samples.len() - 1);
        pcm.samples = pcm.samples[a..=b].to_vec();
    }
    pcm
}

type Factory = Box<dyn FnOnce() -> Result<Box<dyn Synthesizer>> + Send>;

/// A synthesizer that is built the first time it is needed, in the worker thread: loading a
/// model takes a moment and must not freeze the window.
pub struct LazySynth {
    id: String,
    voices: Vec<Voice>,
    factory: Option<Factory>,
    inner: Option<Box<dyn Synthesizer>>,
}

impl LazySynth {
    /// `voices` are what the settings can list without loading anything.
    pub fn new(id: &str, voices: Vec<Voice>, factory: Factory) -> LazySynth {
        LazySynth {
            id: id.to_string(),
            voices,
            factory: Some(factory),
            inner: None,
        }
    }

    fn inner(&mut self) -> Result<&mut Box<dyn Synthesizer>> {
        if self.inner.is_none() {
            let f = self
                .factory
                .take()
                .ok_or_else(|| anyhow!("the voice engine could not be started"))?;
            self.inner = Some(f().context("starting the voice engine")?);
        }
        Ok(self.inner.as_mut().unwrap())
    }
}

impl Synthesizer for LazySynth {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn voices(&mut self) -> Result<Vec<Voice>> {
        Ok(self.voices.clone())
    }

    fn synthesize(&mut self, req: &SynthRequest) -> Result<Pcm> {
        self.inner()?.synthesize(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::Cancel;

    #[test]
    fn silence_is_trimmed_but_a_little_is_kept() {
        let mut s = vec![0.0; 1000];
        s.extend(vec![0.5; 100]);
        s.extend(vec![0.0; 1000]);
        let p = trim_silence(
            Pcm {
                rate: 1000,
                samples: s,
            },
            0.01,
            50,
        );
        assert_eq!(p.samples.len(), 100 + 2 * 50, "{}", p.samples.len());
        let silent = trim_silence(
            Pcm {
                rate: 1000,
                samples: vec![0.0; 10],
            },
            0.01,
            50,
        );
        assert_eq!(silent.samples.len(), 10, "pure silence is left alone");
    }

    #[test]
    fn a_lazy_synth_starts_once_and_reports_a_failure() {
        struct One;
        impl Synthesizer for One {
            fn id(&self) -> String {
                "one".into()
            }
            fn voices(&mut self) -> Result<Vec<Voice>> {
                Ok(vec![])
            }
            fn synthesize(&mut self, _: &SynthRequest) -> Result<Pcm> {
                Ok(Pcm {
                    rate: 8000,
                    samples: vec![0.1; 8],
                })
            }
        }
        fn req(c: &Cancel) -> SynthRequest<'_> {
            SynthRequest {
                text: "x",
                voice: "",
                language: "",
                speed: 1.0,
                cancel: c,
            }
        }
        let c = Cancel::new();
        let started = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let s2 = started.clone();
        let mut l = LazySynth::new(
            "lazy",
            vec![],
            Box::new(move || {
                s2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(Box::new(One))
            }),
        );
        assert_eq!(
            started.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "not at construction"
        );
        assert_eq!(l.id(), "lazy");
        l.synthesize(&req(&c)).unwrap();
        l.synthesize(&req(&c)).unwrap();
        assert_eq!(started.load(std::sync::atomic::Ordering::SeqCst), 1);

        let mut bad = LazySynth::new("bad", vec![], Box::new(|| Err(anyhow!("no model"))));
        let e = format!("{:#}", bad.synthesize(&req(&c)).unwrap_err());
        assert!(e.contains("no model"), "{e}");
        assert!(
            bad.synthesize(&req(&c)).is_err(),
            "and it keeps failing instead of panicking"
        );
    }

    #[test]
    fn a_missing_library_is_an_error_not_a_crash() {
        let e = environment(Path::new("/nonexistent/libonnxruntime.so"))
            .err()
            .unwrap();
        assert!(e.to_string().contains("cannot load ONNX Runtime"), "{e}");
    }

    #[test]
    fn thread_count_is_sensible() {
        assert!((2..=8).contains(&threads()));
    }
}
