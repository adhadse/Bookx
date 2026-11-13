//! Which engine reads a book, and how it is opened.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, Result};

use crate::cache::SentenceCache;
use crate::kokoro::{self, Kokoro};
use crate::models::{kokoro_paths, piper_installed, Store};
use crate::phonemize::espeak_program;
use crate::pipeline::PipelineEngine;
use crate::prefs::TtsPrefs;
use crate::server::ServerSynth;
use crate::service::SharedSynth;
use crate::session::{Engine, MockEngine, Voice};
use crate::sink::CommandSink;
use crate::synth::{Pcm, SynthRequest, Synthesizer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Kokoro,
    Piper,
    Server,
}

impl Choice {
    pub fn id(&self) -> &'static str {
        match self {
            Choice::Kokoro => "kokoro",
            Choice::Piper => "piper",
            Choice::Server => "server",
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Choice::Kokoro => "Kokoro",
            Choice::Piper => "Piper",
            Choice::Server => "Voice server",
        }
    }
}

/// What is ready on this machine.
#[derive(Debug, Clone, Default)]
pub struct Availability {
    pub kokoro: bool,
    /// Languages of the installed Piper voices.
    pub piper_languages: Vec<String>,
    pub espeak: bool,
    pub server: bool,
}

impl Availability {
    pub fn detect(store: &Store, prefs: &TtsPrefs) -> Availability {
        let runtime = store.runtime_lib().is_some();
        Availability {
            kokoro: runtime && kokoro_paths(store, &prefs.kokoro_model).is_some(),
            piper_languages: if runtime {
                piper_installed(store).into_iter().map(|v| v.1).collect()
            } else {
                vec![]
            },
            espeak: espeak_program().is_some(),
            server: !prefs.server.url.trim().is_empty(),
        }
    }
}

/// Why nothing can read the book yet. The application turns the first two into an offer to
/// download the default voice or to install `espeak-ng`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotReady {
    /// No voice model is downloaded (and no voice server is chosen).
    NoVoice,
    /// A model is there (or can be downloaded) but `espeak-ng` is missing.
    NoEspeak,
    /// The downloaded voices cannot read this language (its name).
    Language(String),
}

impl std::fmt::Display for NotReady {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NotReady::NoVoice => write!(f, "No voice is downloaded yet"),
            NotReady::NoEspeak => write!(f, "espeak-ng is not installed, the voices need it"),
            NotReady::Language(l) => write!(f, "No downloaded voice reads {l}"),
        }
    }
}

impl std::error::Error for NotReady {}

/// The engine for a book in `language` and why it is not the one asked for, if so; or why none
/// can read it.
pub fn choose(
    wanted: &str,
    language: &str,
    a: &Availability,
) -> Result<(Choice, Option<String>), NotReady> {
    let lang = crate::text::primary_language(language);
    let kokoro_ok = a.kokoro && a.espeak && kokoro::supports(&lang);
    let piper_ok = a.espeak
        && (lang.is_empty() && !a.piper_languages.is_empty() || a.piper_languages.contains(&lang));
    let best_local = if kokoro_ok {
        Some(Choice::Kokoro)
    } else if piper_ok {
        Some(Choice::Piper)
    } else {
        None
    };
    let fallback = |why: String| match best_local {
        Some(c) => Ok((c, Some(format!("{why}; reading with {}", c.title())))),
        None if !a.espeak => Err(NotReady::NoEspeak),
        None if a.kokoro || !a.piper_languages.is_empty() => {
            Err(NotReady::Language(language_name(&lang)))
        }
        None => Err(NotReady::NoVoice),
    };
    match wanted {
        "server" if a.server => Ok((Choice::Server, None)),
        "server" => fallback("no voice server address is set".into()),
        "piper" if piper_ok => Ok((Choice::Piper, None)),
        "piper" if a.piper_languages.is_empty() => fallback("no Piper voice is downloaded".into()),
        "piper" if !a.espeak => fallback("espeak-ng is not installed, Piper needs it".into()),
        "piper" => fallback(format!(
            "no downloaded Piper voice speaks {}",
            language_name(&lang)
        )),
        _ if kokoro_ok => Ok((Choice::Kokoro, None)),
        _ if !a.kokoro => fallback("Kokoro is not downloaded".into()),
        _ if !a.espeak => fallback("espeak-ng is not installed, Kokoro needs it".into()),
        _ => fallback(format!("Kokoro cannot read {}", language_name(&lang))),
    }
}

pub fn language_name(code: &str) -> String {
    match code {
        "en" => "English",
        "de" => "German",
        "fr" => "French",
        "es" => "Spanish",
        "it" => "Italian",
        "pt" => "Portuguese",
        "nl" => "Dutch",
        "ru" => "Russian",
        "ja" => "Japanese",
        "zh" => "Chinese",
        "ko" => "Korean",
        "hi" => "Hindi",
        "ar" => "Arabic",
        "pl" => "Polish",
        "cs" => "Czech",
        "tr" => "Turkish",
        "" => "this language",
        other => other,
    }
    .to_string()
}

/// An opened engine and what to tell the user about it.
pub struct Opened {
    pub engine: Box<dyn Engine>,
    pub choice: Option<Choice>,
    /// Why this is not the engine the settings ask for.
    pub notice: Option<String>,
}

/// A synthesizer with its display name and look-ahead.
type Built = (&'static str, Box<dyn Synthesizer>, usize);

/// The synthesizer of an engine choice with its display name and look-ahead.
fn synthesizer_for(choice: Choice, prefs: &TtsPrefs, store: &Store) -> Result<Built> {
    Ok(match choice {
        Choice::Server => (
            "Voice server",
            Box::new(ServerSynth::new(&prefs.server)) as Box<dyn Synthesizer>,
            2,
        ),
        Choice::Kokoro => {
            let (model, voices_dir) = kokoro_paths(store, &prefs.kokoro_model)
                .ok_or_else(|| anyhow!("Kokoro is not downloaded"))?;
            let lib = store
                .runtime_lib()
                .ok_or_else(|| anyhow!("ONNX Runtime is not downloaded"))?;
            let voices = Kokoro::list_voices(&voices_dir);
            let id = format!(
                "kokoro-{}",
                model
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("model")
            );
            let key = format!("kokoro|{}|{}", model.display(), voices_dir.display());
            let lazy = SharedSynth::new(
                crate::service::global(),
                &key,
                &id,
                voices,
                Box::new(move || Ok(Box::new(Kokoro::load(&lib, &model, &voices_dir)?))),
            );
            ("Kokoro", Box::new(lazy), 3)
        }
        Choice::Piper => {
            let lib = store
                .runtime_lib()
                .ok_or_else(|| anyhow!("ONNX Runtime is not downloaded"))?;
            let installed = piper_installed(store);
            let voices: Vec<Voice> = installed
                .iter()
                .map(|v| Voice {
                    name: v.0.clone(),
                    language: v.1.clone(),
                })
                .collect();
            let key = format!(
                "piper|{}",
                installed
                    .iter()
                    .map(|v| v.0.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            let lazy = SharedSynth::new(
                crate::service::global(),
                &key,
                "piper",
                voices,
                Box::new(move || Ok(Box::new(crate::piper::Piper::new(&lib, installed)?))),
            );
            ("Piper", Box::new(lazy), 3)
        }
    })
}

/// A silent stand-in for the UI tests (`$BOOKX_TTS_MOCK`): a 200 Hz tone, 40 ms per character.
struct ToneSynth;

impl Synthesizer for ToneSynth {
    fn id(&self) -> String {
        "mock".into()
    }
    fn voices(&mut self) -> Result<Vec<Voice>> {
        Ok(Vec::new())
    }
    fn synthesize(&mut self, req: &SynthRequest) -> Result<Pcm> {
        let rate = 8000u32;
        let n =
            (req.text.chars().count() as f64 * 0.04 * rate as f64 / req.speed.max(0.1)) as usize;
        Ok(Pcm {
            rate,
            samples: (0..n)
                .map(|i| (i as f32 / rate as f32 * 200.0 * std::f32::consts::TAU).sin() * 0.3)
                .collect(),
        })
    }
}

/// The synthesizer the settings ask for, to write audio files. Returns the engine's id (`kokoro`, `piper`,
/// `server`, `mock`) too, the key of the voice setting.
pub fn open_synthesizer(
    prefs: &TtsPrefs,
    store: &Store,
    language: &str,
) -> Result<(Box<dyn Synthesizer>, String)> {
    if std::env::var("BOOKX_TTS_MOCK").is_ok_and(|v| v.parse::<u64>().is_ok()) {
        return Ok((Box::new(ToneSynth), "mock".into()));
    }
    let avail = Availability::detect(store, prefs);
    let (choice, _) = choose(&prefs.engine, language, &avail)?;
    let (_, synth, _) = synthesizer_for(choice, prefs, store)?;
    Ok((synth, choice.id().to_string()))
}

/// Opens the engine for a book in `language`. `$BOOKX_TTS_MOCK` (milliseconds per sentence)
/// gives the silent engine of the UI tests.
pub fn open(
    prefs: &TtsPrefs,
    store: &Store,
    language: &str,
    cache_dir: Option<PathBuf>,
) -> Result<Opened> {
    if let Some(ms) = std::env::var("BOOKX_TTS_MOCK")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        return Ok(Opened {
            engine: Box::new(MockEngine::new(Duration::from_millis(ms)).with_lookahead(2)),
            choice: None,
            notice: None,
        });
    }
    let avail = Availability::detect(store, prefs);
    let (choice, notice) = choose(&prefs.engine, language, &avail)?;
    let cache = cache_dir
        .filter(|_| prefs.cache_mb > 0)
        .map(|d| SentenceCache::new(d, prefs.cache_mb));
    let (name, synth, lookahead) = synthesizer_for(choice, prefs, store)?;
    // a light hold on the application's speech service: a book that is still winding down is asked
    // to stop and waited for briefly, never an error
    let session = crate::service::global().acquire(crate::service::HANDOFF_WAIT);
    let engine: Box<dyn Engine> = Box::new(
        PipelineEngine::new(name, synth, Box::new(CommandSink::new()), cache, lookahead)
            .with_session(session),
    );
    Ok(Opened {
        engine,
        choice: Some(choice),
        notice,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_to_a_file_without_a_voice_says_what_is_missing() {
        let store = Store::new(std::env::temp_dir().join("bookx-tts-none-factory"));
        if std::env::var("BOOKX_TTS_MOCK").is_err() && espeak_program().is_some() {
            let e = open_synthesizer(&TtsPrefs::default(), &store, "en")
                .err()
                .unwrap();
            assert_eq!(e.downcast_ref::<NotReady>(), Some(&NotReady::NoVoice));
        }
    }

    #[test]
    fn the_tone_synth_is_as_long_as_the_text_asks() {
        let c = crate::synth::Cancel::new();
        let p = ToneSynth
            .synthesize(&SynthRequest {
                text: "ten chars.",
                voice: "",
                language: "",
                speed: 1.0,
                cancel: &c,
            })
            .unwrap();
        assert_eq!(p.samples.len(), 3200);
    }

    fn all() -> Availability {
        Availability {
            kokoro: true,
            piper_languages: vec!["de".into(), "en".into()],
            espeak: true,
            server: true,
        }
    }

    fn picked(wanted: &str, lang: &str, a: &Availability) -> Choice {
        choose(wanted, lang, a).unwrap().0
    }

    #[test]
    fn kokoro_is_the_default_and_piper_reads_what_kokoro_cannot() {
        assert_eq!(
            choose("kokoro", "en-GB", &all()),
            Ok((Choice::Kokoro, None))
        );
        // Kokoro does not speak German, a downloaded Piper voice does
        let (c, why) = choose("kokoro", "de", &all()).unwrap();
        assert_eq!(c, Choice::Piper);
        assert!(why.unwrap().contains("Kokoro cannot read German"));
        // no language known: the best thing that is installed
        assert_eq!(picked("kokoro", "", &all()), Choice::Kokoro);
        // the old names of the removed settings behave like Kokoro
        assert_eq!(picked("auto", "en", &all()), Choice::Kokoro);
        assert_eq!(picked("speechd", "en", &all()), Choice::Kokoro);
    }

    #[test]
    fn nothing_ready_says_what_is_missing() {
        let none = Availability {
            espeak: true,
            ..Default::default()
        };
        assert_eq!(choose("kokoro", "en", &none), Err(NotReady::NoVoice));
        assert_eq!(choose("piper", "en", &none), Err(NotReady::NoVoice));
        // espeak first: downloading would not help yet
        assert_eq!(
            choose("kokoro", "en", &Availability::default()),
            Err(NotReady::NoEspeak)
        );
        assert_eq!(
            choose(
                "kokoro",
                "en",
                &Availability {
                    espeak: false,
                    ..all()
                }
            ),
            Err(NotReady::NoEspeak)
        );
        // nobody speaks Japanese here
        assert_eq!(
            choose("kokoro", "ja", &all()),
            Err(NotReady::Language("Japanese".into()))
        );
        assert!(NotReady::NoVoice.to_string().contains("No voice"));
    }

    #[test]
    fn the_server_is_only_used_when_asked_for() {
        assert_eq!(picked("kokoro", "en", &all()), Choice::Kokoro);
        assert_eq!(choose("server", "en", &all()), Ok((Choice::Server, None)));
        // a server needs no model and no espeak
        let only_server = Availability {
            server: true,
            ..Default::default()
        };
        assert_eq!(picked("server", "ja", &only_server), Choice::Server);
        assert_eq!(
            choose("kokoro", "en", &only_server),
            Err(NotReady::NoEspeak),
            "a server is never used silently"
        );
        let (c, why) = choose(
            "server",
            "en",
            &Availability {
                server: false,
                ..all()
            },
        )
        .unwrap();
        assert_eq!(c, Choice::Kokoro);
        assert!(why.unwrap().contains("no voice server address"));
    }

    #[test]
    fn a_wish_that_cannot_be_met_falls_back_and_says_why() {
        let (c, why) = choose(
            "kokoro",
            "en",
            &Availability {
                kokoro: false,
                ..all()
            },
        )
        .unwrap();
        assert_eq!(c, Choice::Piper);
        assert!(why.unwrap().contains("Kokoro is not downloaded"));

        let (c, why) = choose("piper", "fr", &all()).unwrap();
        assert_eq!(c, Choice::Kokoro);
        assert!(why
            .unwrap()
            .contains("no downloaded Piper voice speaks French"));
    }

    #[test]
    fn the_mock_is_what_the_environment_asks_for() {
        // read, not set: tests run in parallel and must not touch the environment
        if std::env::var("BOOKX_TTS_MOCK").is_ok() {
            let o = open(
                &TtsPrefs::default(),
                &Store::new("/nonexistent"),
                "en",
                None,
            )
            .unwrap();
            assert!(o.choice.is_none());
        }
    }

    #[test]
    fn an_installed_kokoro_is_found_whatever_model_the_settings_name() {
        use crate::models::{find_pack, Store};
        let d = std::env::temp_dir().join(format!("bookx-avail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let store = Store::new(&d);
        let install = |id: &str| {
            let p = find_pack(id).unwrap();
            for f in &p.files {
                let path = store.dir_of(&p).join(&f.rel);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::File::create(&path)
                    .unwrap()
                    .set_len(f.size)
                    .unwrap();
            }
            store.mark_installed(&p).unwrap();
        };
        let prefs = TtsPrefs {
            kokoro_model: "fp32".into(),
            ..Default::default()
        };
        assert!(!Availability::detect(&store, &prefs).kokoro, "nothing yet");
        install("kokoro-voices");
        install("kokoro-int8");
        assert!(
            !Availability::detect(&store, &prefs).kokoro,
            "no runtime yet"
        );
        std::fs::create_dir_all(d.join("onnxruntime")).unwrap();
        std::fs::write(d.join("onnxruntime/libonnxruntime.so"), b"x").unwrap();
        let a = Availability::detect(&store, &prefs);
        assert!(
            a.kokoro,
            "int8 is installed, the setting says fp32: it is used all the same"
        );
        // with espeak around, the choice is Kokoro and nothing is offered for download
        let with_espeak = Availability { espeak: true, ..a };
        assert_eq!(
            choose("kokoro", "en", &with_espeak).map(|c| c.0),
            Ok(Choice::Kokoro)
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
