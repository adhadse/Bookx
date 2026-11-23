//! Read aloud (roadmap R10, enhancement 9): the GTK-free parts live in the `bookx-tts` crate
//! (sentences, engines, model downloads, playback); this module is what the application adds:
//! where the downloaded models and the audio cache live, and the engine for a book.

use std::path::PathBuf;

use anyhow::Result;

pub use bookx_tts::factory::NotReady;
pub use bookx_tts::factory::{Availability, Opened};
pub use bookx_tts::models::Store;
pub use bookx_tts::setup::{self, Failure};
pub use bookx_tts::synth::Cancel;
pub use bookx_tts::text::{book_language, primary_language};
pub use bookx_tts::voices_for;
pub use bookx_tts::{Engine, Session, SleepTimer, State, TtsPrefs, Update, Voice, split_sentences};

/// Where the downloaded models are (`$BOOKX_TTS_DIR`, else `<data dir>/tts`). The first time, packs
/// that earlier versions (or the live tests) left in `~/.cache/bookx-tts` are moved here, so an
/// installed voice is never offered for download again.
pub fn store() -> Store {
    if let Some(d) = std::env::var_os("BOOKX_TTS_DIR") {
        return Store::new(d);
    }
    let dir = directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.data_dir().join("tts"))
        .unwrap_or_else(|| PathBuf::from("tts"));
    static MIGRATED: std::sync::Once = std::sync::Once::new();
    MIGRATED.call_once(|| {
        let legacy: Vec<PathBuf> = directories::BaseDirs::new()
            .map(|b| {
                vec![
                    b.cache_dir().join("bookx-tts"),
                    b.home_dir().join(".cache/bookx-tts"),
                ]
            })
            .unwrap_or_default();
        bookx_tts::models::migrate_legacy(&dir, &legacy);
    });
    Store::new(dir)
}

/// The cache of synthesized sentences (cleared from the settings).
pub fn cache_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx").map(|d| d.cache_dir().join("tts"))
}

/// How the UI tests stand in for the download of the voice (`$BOOKX_TTS_MOCK_DOWNLOAD`): a number
/// of milliseconds the fake download takes, or `offline` for one that fails like no network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockDownload {
    Takes(u64),
    /// Like `Takes`, but the first origin is given up a third of the way and the second one used.
    SwitchOrigin(u64),
    Offline,
}

pub fn mock_download() -> Option<MockDownload> {
    parse_mock_download(std::env::var("BOOKX_TTS_MOCK_DOWNLOAD").ok().as_deref())
}

fn parse_mock_download(v: Option<&str>) -> Option<MockDownload> {
    match v?.trim() {
        "offline" => Some(MockDownload::Offline),
        n if n.starts_with("switch:") => n[7..].parse().ok().map(MockDownload::SwitchOrigin),
        n => n.parse().ok().map(MockDownload::Takes),
    }
}

fn mock_marker() -> PathBuf {
    store().root.join(".mock-voice")
}

/// The engine for a book in `language`: what the settings ask for, or what works. When nothing
/// can read yet the error is a [`NotReady`] (download the voice, install espeak-ng, ...).
pub fn open_engine(prefs: &TtsPrefs, language: &str) -> Result<Opened> {
    if mock_download().is_some() && !mock_marker().exists() {
        return Err(NotReady::NoVoice.into());
    }
    if std::env::var("BOOKX_TTS_MOCK_ESPEAK").as_deref() == Ok("missing") {
        return Err(NotReady::NoEspeak.into());
    }
    bookx_tts::factory::open(prefs, &store(), language, cache_dir())
}

/// What downloading the default voice (Kokoro, small model) involves, for the first-time dialog.
pub fn default_voice(prefs: &TtsPrefs) -> Option<bookx_tts::items::Item> {
    let mut p = prefs.clone();
    p.kokoro_model = "int8".into();
    bookx_tts::items::items(&p, &store())
        .into_iter()
        .find(|i| i.key == "kokoro")
}

/// The packs of the default voice, for the first-time download.
pub fn default_voice_packs(prefs: &TtsPrefs) -> Vec<bookx_tts::models::Pack> {
    let Some(item) = default_voice(prefs) else {
        return vec![];
    };
    packs_of(&item.plan)
}

/// The catalogue packs with these ids.
pub fn packs_of(ids: &[String]) -> Vec<bookx_tts::models::Pack> {
    let all = bookx_tts::models::catalog();
    ids.iter()
        .filter_map(|id| all.iter().find(|p| &p.id == id).cloned())
        .collect()
}

/// Downloads `packs` (resumable, in parallel segments, every file checked against its checksum).
/// `progress(file, done, total, origin)` gets bytes and the number of the origin in use. Blocks; run it on a worker thread.
pub fn download_packs(
    packs: &[bookx_tts::models::Pack],
    cancel: &Cancel,
    progress: &mut dyn FnMut(&str, u64, u64, usize),
) -> std::result::Result<(), Failure> {
    if let Some(m) = mock_download() {
        let steps = 20u64;
        let (ms, switch) = match m {
            MockDownload::Takes(ms) => (ms, false),
            MockDownload::SwitchOrigin(ms) => (ms, true),
            MockDownload::Offline => {
                return Err(setup::classify(
                    "cannot reach https://example.invalid/model: offline",
                ));
            }
        };
        for i in 0..=steps {
            if cancel.is_cancelled() {
                return Err(Failure::Cancelled);
            }
            progress(
                "mock-voice.onnx",
                i * 1_000_000,
                steps * 1_000_000,
                usize::from(switch && i > steps / 3),
            );
            std::thread::sleep(std::time::Duration::from_millis(ms / steps));
        }
        let _ = std::fs::create_dir_all(store().root);
        let _ = std::fs::write(mock_marker(), b"ok");
        return Ok(());
    }
    bookx_tts::download::install_all(&store(), packs, cancel, &mut |p| {
        progress(&p.file, p.done, p.total, p.origin)
    })
    .map_err(|e| setup::classify(&format!("{e:#}")))
}

/// The command that installs espeak-ng on this system, if the distribution is known.
pub fn espeak_install_command() -> Option<String> {
    let text = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    setup::espeak_install_command(&text)
}

/// The synthesizer the settings ask for, for writing audio files (and the id of its voice setting).
pub fn open_synthesizer(
    prefs: &TtsPrefs,
    language: &str,
) -> Result<(Box<dyn bookx_tts::synth::Synthesizer>, String)> {
    bookx_tts::factory::open_synthesizer(prefs, &store(), language)
}

/// Moves a key given in `$BOOKX_TTS_API_KEY` into the settings (and so into the keyring) when no
/// key is set there yet. Returns true when the settings changed.
pub fn migrate_env_key(prefs: &mut TtsPrefs, env: Option<&str>) -> bool {
    match env.map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) if prefs.server.api_key.trim().is_empty() => {
            prefs.server.api_key = k.to_string();
            true
        }
        _ => false,
    }
}

/// Old configs may name an engine that is gone (`speechd`, the basic voice, or `auto`): they read
/// with Kokoro now. Returns true when the settings changed.
pub fn migrate_engine(prefs: &mut TtsPrefs) -> bool {
    if bookx_tts::prefs::ENGINES.contains(&prefs.engine.as_str()) {
        return false;
    }
    prefs.engine = "kokoro".into();
    true
}

/// The sentence the voice test reads.
pub fn test_phrase(language: &str) -> &'static str {
    match primary_language(language).as_str() {
        "de" => "Hallo! So klingt diese Stimme, wenn sie dir ein Buch vorliest.",
        "fr" => "Bonjour ! Voici comment cette voix te lit un livre.",
        "es" => "¡Hola! Así suena esta voz cuando te lee un libro.",
        "it" => "Ciao! Ecco come suona questa voce quando ti legge un libro.",
        "pt" => "Olá! É assim que esta voz soa quando lê um livro para você.",
        _ => "Hello! This is how this voice sounds when it reads you a book.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_phrase_follows_the_language() {
        assert!(test_phrase("de-AT").starts_with("Hallo"));
        assert!(test_phrase("fre").starts_with("Bonjour"));
        assert!(test_phrase("").starts_with("Hello"));
        assert!(test_phrase("ja").starts_with("Hello"));
    }

    #[test]
    fn a_key_from_the_environment_moves_into_the_settings_once() {
        let mut p = TtsPrefs::default();
        assert!(!migrate_env_key(&mut p, None));
        assert!(!migrate_env_key(&mut p, Some("  ")));
        assert!(migrate_env_key(&mut p, Some(" k1 ")));
        assert_eq!(p.server.api_key, "k1");
        assert!(!migrate_env_key(&mut p, Some("k2")), "a saved key wins");
        assert_eq!(p.server.api_key, "k1");
    }

    #[test]
    fn a_removed_engine_is_migrated_to_kokoro_once() {
        for old in ["speechd", "auto", "nonsense"] {
            let mut p = TtsPrefs {
                engine: old.into(),
                ..Default::default()
            };
            assert!(migrate_engine(&mut p), "{old}");
            assert_eq!(p.engine, "kokoro");
            assert!(!migrate_engine(&mut p), "second time: nothing to do");
        }
        let mut p = TtsPrefs {
            engine: "server".into(),
            ..Default::default()
        };
        assert!(!migrate_engine(&mut p));
        assert_eq!(p.engine, "server");
        let old: TtsPrefs =
            serde_json::from_str(r#"{"engine":"speechd","voice":"en-us","speed":1.3}"#).unwrap();
        let mut old = old;
        assert!(migrate_engine(&mut old));
        assert_eq!(old.speed, 1.3);
    }

    #[test]
    fn the_mock_download_is_read_from_its_variable() {
        assert_eq!(parse_mock_download(None), None);
        assert_eq!(
            parse_mock_download(Some("offline")),
            Some(MockDownload::Offline)
        );
        assert_eq!(
            parse_mock_download(Some(" 400 ")),
            Some(MockDownload::Takes(400))
        );
        assert_eq!(parse_mock_download(Some("x")), None);
        assert_eq!(
            parse_mock_download(Some("switch:900")),
            Some(MockDownload::SwitchOrigin(900))
        );
    }

    #[test]
    fn the_default_voice_is_kokoro_with_its_licence_and_size() {
        let p = TtsPrefs::default();
        let item = default_voice(&p).expect("Kokoro is in the catalogue");
        assert!(item.title.contains("Kokoro"));
        assert!(!item.license_name.is_empty());
        assert!(item.size_bytes > 50_000_000, "{}", item.size_bytes);
    }

    #[test]
    fn the_store_can_be_moved_for_tests() {
        if std::env::var_os("BOOKX_TTS_DIR").is_none() {
            assert!(store().root.ends_with("tts"));
        }
    }
}
