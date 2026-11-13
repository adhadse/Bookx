//! Piper voices (VITS models in ONNX) run directly: espeak-ng phonemes in, speech out.
//!
//! Piper's own program is not used, only the voice files of the `rhasspy/piper-voices`
//! collection. The `.onnx.json` next to a voice has the sample rate, the espeak voice and
//! the table from phonemes to ids.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use ort::environment::Environment;
use ort::session::Session;
use ort::value::Tensor;
use serde::Deserialize;

use crate::onnx::{load_model, trim_silence};
use crate::phonemize::phonemize;
use crate::session::Voice;
use crate::synth::{Pcm, SynthRequest, Synthesizer};
use crate::text::chunk_text;

#[derive(Debug, Deserialize)]
struct Audio {
    sample_rate: u32,
}

#[derive(Debug, Deserialize, Default)]
struct Espeak {
    #[serde(default)]
    voice: String,
}

#[derive(Debug, Deserialize)]
struct Inference {
    #[serde(default = "d_noise")]
    noise_scale: f32,
    #[serde(default = "d_length")]
    length_scale: f32,
    #[serde(default = "d_noise_w")]
    noise_w: f32,
}

fn d_noise() -> f32 {
    0.667
}
fn d_length() -> f32 {
    1.0
}
fn d_noise_w() -> f32 {
    0.8
}

impl Default for Inference {
    fn default() -> Self {
        Inference {
            noise_scale: d_noise(),
            length_scale: d_length(),
            noise_w: d_noise_w(),
        }
    }
}

/// The `.onnx.json` of a Piper voice.
#[derive(Debug, Deserialize)]
pub struct PiperConfig {
    audio: Audio,
    #[serde(default)]
    espeak: Espeak,
    #[serde(default)]
    inference: Inference,
    phoneme_id_map: HashMap<String, Vec<i64>>,
}

impl PiperConfig {
    pub fn parse(json: &str) -> Result<PiperConfig> {
        serde_json::from_str(json).context("the voice's .onnx.json is not valid")
    }

    pub fn sample_rate(&self) -> u32 {
        self.audio.sample_rate
    }

    /// Piper's ids for phonemes: `^`, then every phoneme followed by the pad `_`, then `$`.
    /// Phonemes the voice does not know are skipped.
    pub fn ids(&self, phonemes: &str) -> Vec<i64> {
        let get = |k: &str| self.phoneme_id_map.get(k).cloned().unwrap_or_default();
        let pad = get("_");
        let mut ids = get("^");
        for c in phonemes.chars() {
            if let Some(i) = self.phoneme_id_map.get(&c.to_string()) {
                ids.extend(i);
                ids.extend(&pad);
            }
        }
        ids.extend(get("$"));
        ids
    }
}

/// The installed voice to use: the named one, else one of the language, else (no language
/// known) the first.
pub fn pick_voice(
    voices: &[(String, String, PathBuf)],
    voice: &str,
    language: &str,
) -> Result<(String, PathBuf)> {
    let lang = language.split(['-', '_']).next().unwrap_or("");
    let found = voices
        .iter()
        .find(|v| v.0 == voice)
        .or_else(|| voices.iter().find(|v| !lang.is_empty() && v.1 == lang))
        .or_else(|| {
            if lang.is_empty() {
                voices.first()
            } else {
                None
            }
        });
    match found {
        Some(v) => Ok((v.0.clone(), v.2.clone())),
        None if voices.is_empty() => bail!("no Piper voice is downloaded"),
        None => bail!("no downloaded Piper voice speaks \"{lang}\"; download one in the settings"),
    }
}

struct Loaded {
    session: Session,
    cfg: PiperConfig,
    needs_sid: bool,
}

pub struct Piper {
    env: Environment,
    /// id, language, model path
    voices: Vec<(String, String, PathBuf)>,
    loaded: HashMap<String, Loaded>,
}

impl Piper {
    pub fn new(runtime_lib: &Path, voices: Vec<(String, String, PathBuf)>) -> Result<Piper> {
        Ok(Piper {
            env: crate::onnx::environment(runtime_lib)?,
            voices,
            loaded: HashMap::new(),
        })
    }

    fn load(&mut self, id: &str, path: &Path) -> Result<&mut Loaded> {
        if !self.loaded.contains_key(id) {
            let json = std::fs::read_to_string(path.with_extension("onnx.json"))
                .with_context(|| format!("reading the configuration of {id}"))?;
            let cfg = PiperConfig::parse(&json)?;
            let session = load_model(&self.env, path)?;
            let needs_sid = session.inputs().iter().any(|i| i.name() == "sid");
            self.loaded.insert(
                id.to_string(),
                Loaded {
                    session,
                    cfg,
                    needs_sid,
                },
            );
        }
        Ok(self.loaded.get_mut(id).unwrap())
    }
}

impl Synthesizer for Piper {
    fn id(&self) -> String {
        "piper".into()
    }

    fn voices(&mut self) -> Result<Vec<Voice>> {
        Ok(self
            .voices
            .iter()
            .map(|v| Voice {
                name: v.0.clone(),
                language: v.1.clone(),
            })
            .collect())
    }

    fn synthesize(&mut self, req: &SynthRequest) -> Result<Pcm> {
        let (id, path) = pick_voice(&self.voices, req.voice, req.language)?;
        let model = self.load(&id, &path)?;
        let espeak = if model.cfg.espeak.voice.is_empty() {
            req.language.to_string()
        } else {
            model.cfg.espeak.voice.clone()
        };
        let rate = model.cfg.sample_rate();
        let mut out = Pcm {
            rate,
            samples: vec![],
        };
        for chunk in chunk_text(req.text, 300) {
            if req.cancel.is_cancelled() {
                bail!("cancelled");
            }
            let ph = phonemize(&chunk, &espeak)?;
            let ids = model.cfg.ids(&ph);
            if ids.len() <= 2 {
                continue;
            }
            let n = ids.len();
            let scales = vec![
                model.cfg.inference.noise_scale,
                model.cfg.inference.length_scale / req.speed.clamp(0.5, 2.0) as f32,
                model.cfg.inference.noise_w,
            ];
            let mut inputs = ort::inputs![
                "input" => Tensor::from_array(([1usize, n], ids)).map_err(|e| anyhow!("{e}"))?,
                "input_lengths" => Tensor::from_array(([1usize], vec![n as i64])).map_err(|e| anyhow!("{e}"))?,
                "scales" => Tensor::from_array(([3usize], scales)).map_err(|e| anyhow!("{e}"))?,
            ];
            if model.needs_sid {
                inputs.push((
                    "sid".into(),
                    Tensor::from_array(([1usize], vec![0i64]))
                        .map_err(|e| anyhow!("{e}"))?
                        .into(),
                ));
            }
            let outputs = model
                .session
                .run(inputs)
                .map_err(|e| anyhow!("the model failed: {e}"))?;
            let (_, wave) = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(|e| anyhow!("unexpected model output: {e}"))?;
            let pcm = trim_silence(
                Pcm {
                    rate,
                    samples: wave.to_vec(),
                },
                0.004,
                40,
            );
            out.append(&pcm, if out.samples.is_empty() { 0 } else { 60 });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: &str = r#"{
        "audio": {"sample_rate": 22050},
        "espeak": {"voice": "de"},
        "inference": {"noise_scale": 0.5, "length_scale": 1.1, "noise_w": 0.3},
        "num_speakers": 1,
        "phoneme_id_map": {"_": [0], "^": [1], "$": [2], " ": [3], "a": [10], "ː": [11, 12], ",": [4]}
    }"#;

    #[test]
    fn the_voice_configuration_is_read() {
        let c = PiperConfig::parse(CFG).unwrap();
        assert_eq!(c.sample_rate(), 22050);
        assert_eq!(c.espeak.voice, "de");
        assert_eq!(c.inference.length_scale, 1.1);
        assert!(PiperConfig::parse("{}").is_err());
        // defaults for the optional parts
        let c =
            PiperConfig::parse(r#"{"audio":{"sample_rate":16000},"phoneme_id_map":{}}"#).unwrap();
        assert_eq!(c.inference.noise_scale, 0.667);
        assert_eq!(c.espeak.voice, "");
    }

    #[test]
    fn ids_have_the_start_the_end_and_a_pad_after_every_phoneme() {
        let c = PiperConfig::parse(CFG).unwrap();
        assert_eq!(
            c.ids("a aː,"),
            vec![1, 10, 0, 3, 0, 10, 0, 11, 12, 0, 4, 0, 2]
        );
        assert_eq!(c.ids("ZZ"), vec![1, 2], "unknown phonemes are skipped");
        assert_eq!(c.ids(""), vec![1, 2]);
    }

    #[test]
    fn picking_a_voice_prefers_the_named_one_then_the_language() {
        let v = vec![
            (
                "de_DE-thorsten-medium".to_string(),
                "de".to_string(),
                PathBuf::new(),
            ),
            (
                "en_US-lessac-medium".to_string(),
                "en".to_string(),
                PathBuf::new(),
            ),
        ];
        let pick = |voice, lang| pick_voice(&v, voice, lang).map(|x| x.0);
        assert_eq!(
            pick("en_US-lessac-medium", "de").unwrap(),
            "en_US-lessac-medium"
        );
        assert_eq!(pick("", "de-AT").unwrap(), "de_DE-thorsten-medium");
        assert_eq!(pick("", "").unwrap(), "de_DE-thorsten-medium");
        assert!(pick("", "fr").unwrap_err().to_string().contains("\"fr\""));
        assert!(pick_voice(&[], "", "en")
            .unwrap_err()
            .to_string()
            .contains("no Piper voice"));
    }
}
