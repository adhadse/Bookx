//! The settings of read aloud (stored in Bookx's config as `tts`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const MIN_SPEED: f64 = 0.5;
pub const MAX_SPEED: f64 = 2.5;
pub const MIN_PITCH: f64 = 0.8;
pub const MAX_PITCH: f64 = 1.25;

/// The engines the user can choose. Older configs may hold `auto` or `speechd` (the basic voice,
/// removed); [`TtsPrefs::clamped`] turns them into `kokoro`.
pub const ENGINES: [&str; 3] = ["kokoro", "piper", "server"];

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerPrefs {
    /// `openai` (POST /v1/audio/speech), `xtts` (xtts-api-server) or `fish` (Fish Speech API).
    pub kind: String,
    pub url: String,
    /// The `model` field of OpenAI-style requests.
    pub model: String,
    /// Voice (OpenAI), speaker (XTTS) or reference id (Fish) when none is picked.
    pub voice: String,
    /// Bearer key for the server. The application keeps it in the system keyring (and fills it in
    /// at load); `$BOOKX_TTS_API_KEY` is used when this is empty.
    pub api_key: String,
}

impl std::fmt::Debug for ServerPrefs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerPrefs")
            .field("kind", &self.kind)
            .field("url", &self.url)
            .field("model", &self.model)
            .field("voice", &self.voice)
            .field(
                "api_key",
                &if self.api_key.is_empty() { "" } else { "<set>" },
            )
            .finish()
    }
}

impl Default for ServerPrefs {
    fn default() -> Self {
        ServerPrefs {
            kind: "openai".into(),
            url: "http://127.0.0.1:8880".into(),
            model: "tts-1".into(),
            voice: String::new(),
            api_key: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TtsPrefs {
    /// Speaking speed, 1.0 = normal.
    pub speed: f64,
    /// Pitch factor, 1.0 = normal (neural engines only).
    pub pitch: f64,
    /// Playback volume of the player card, 0.0..=1.0.
    pub volume: f64,
    /// One of [`ENGINES`].
    pub engine: String,
    /// The voice chosen per engine and language: `kokoro/en` -> `af_heart`.
    pub voices: BTreeMap<String, String>,
    pub server: ServerPrefs,
    /// Size cap of the cache of synthesized sentences in MB; 0 = no cache.
    pub cache_mb: u32,
    /// The user said "always" to downloading the default voice when Read Aloud is pressed
    /// without one (asked once, with the licence note and the size).
    pub download_without_asking: bool,
    /// Kokoro model: `int8` (small, fast) or `fp32` (best quality).
    pub kokoro_model: String,
}

impl Default for TtsPrefs {
    fn default() -> Self {
        TtsPrefs {
            speed: 1.0,
            pitch: 1.0,
            volume: 1.0,
            engine: "kokoro".into(),
            voices: BTreeMap::new(),
            server: ServerPrefs::default(),
            cache_mb: 0,
            download_without_asking: false,
            kokoro_model: "int8".into(),
        }
    }
}

impl TtsPrefs {
    pub fn clamped(&self) -> TtsPrefs {
        let mut p = self.clone();
        p.speed = if self.speed.is_finite() {
            self.speed.clamp(MIN_SPEED, MAX_SPEED)
        } else {
            1.0
        };
        p.volume = if self.volume.is_finite() {
            self.volume.clamp(0.0, 1.0)
        } else {
            1.0
        };
        p.pitch = if self.pitch.is_finite() {
            self.pitch.clamp(MIN_PITCH, MAX_PITCH)
        } else {
            1.0
        };
        if !ENGINES.contains(&p.engine.as_str()) {
            p.engine = "kokoro".into();
        }
        if p.kokoro_model != "fp32" {
            p.kokoro_model = "int8".into();
        }
        p.server.url = p.server.url.trim().trim_end_matches('/').to_string();
        p.cache_mb = p.cache_mb.min(5000);
        p
    }

    /// The voice picked for `language` on `engine` ("" = the engine chooses).
    pub fn voice_for(&self, engine: &str, language: &str) -> String {
        self.voices
            .get(&format!("{engine}/{language}"))
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_voice_for(&mut self, engine: &str, language: &str, voice: &str) {
        let key = format!("{engine}/{language}");
        if voice.is_empty() {
            self.voices.remove(&key);
        } else {
            self.voices.insert(key, voice.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_are_clamped_and_old_configs_still_load() {
        let p = TtsPrefs {
            speed: 0.1,
            pitch: f64::NAN,
            engine: "nonsense".into(),
            kokoro_model: "weird".into(),
            cache_mb: 999_999,
            server: ServerPrefs {
                url: " http://h:1/ ".into(),
                ..Default::default()
            },
            ..Default::default()
        }
        .clamped();
        assert_eq!(p.speed, 0.5);
        assert_eq!(p.pitch, 1.0);
        assert_eq!(
            TtsPrefs {
                volume: 7.0,
                ..Default::default()
            }
            .clamped()
            .volume,
            1.0
        );
        assert_eq!(
            TtsPrefs {
                volume: -1.0,
                ..Default::default()
            }
            .clamped()
            .volume,
            0.0
        );
        assert!(!p.download_without_asking);
        assert_eq!(p.engine, "kokoro");
        assert_eq!(p.kokoro_model, "int8");
        assert_eq!(p.cache_mb, 5000);
        assert_eq!(p.server.url, "http://h:1");
        // a config written before the engines existed
        let old: TtsPrefs = serde_json::from_str(r#"{"speed":1.4,"voice":"en-us"}"#).unwrap();
        assert_eq!(old.speed, 1.4);
        assert_eq!(old.clamped().engine, "kokoro");
    }

    #[test]
    fn the_removed_engines_become_kokoro() {
        for old in ["speechd", "auto", ""] {
            let p = TtsPrefs {
                engine: old.into(),
                ..Default::default()
            };
            assert_eq!(p.clamped().engine, "kokoro", "{old}");
        }
        for keep in ["piper", "server", "kokoro"] {
            let p = TtsPrefs {
                engine: keep.into(),
                ..Default::default()
            };
            assert_eq!(p.clamped().engine, keep);
        }
    }

    #[test]
    fn voices_are_remembered_per_engine_and_language() {
        let mut p = TtsPrefs::default();
        p.set_voice_for("kokoro", "en", "af_heart");
        p.set_voice_for("kokoro", "fr", "ff_siwis");
        p.set_voice_for("piper", "en", "en_US-lessac-medium");
        assert_eq!(p.voice_for("kokoro", "en"), "af_heart");
        assert_eq!(p.voice_for("piper", "en"), "en_US-lessac-medium");
        assert_eq!(p.voice_for("kokoro", "de"), "");
        p.set_voice_for("kokoro", "en", "");
        assert_eq!(p.voice_for("kokoro", "en"), "");
    }
}
