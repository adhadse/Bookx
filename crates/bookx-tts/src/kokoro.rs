//! Kokoro-82M (Apache-2.0) through ONNX Runtime: espeak-ng phonemes in, 24 kHz speech out.
//!
//! The model is a StyleTTS2 derivative with 82 million parameters. Its inputs are the token
//! ids of the phonemes, a 256 numbers style vector (taken from the voice file by the number
//! of tokens) and the speed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use ort::environment::Environment;
use ort::session::Session;
use ort::value::Tensor;

use crate::catalog_data::KOKORO_VOCAB;
use crate::models::{kokoro_voice_language, KOKORO_LANGUAGES};
use crate::onnx::{load_model, trim_silence};
use crate::phonemize::phonemize;
use crate::session::Voice;
use crate::synth::{Pcm, SynthRequest, Synthesizer};
use crate::text::chunk_text;

pub const RATE: u32 = 24000;
/// The model takes 510 tokens; stay below it with room for the padding.
const MAX_TOKENS: usize = 500;
/// Characters of text per run: long sentences are cut at clauses.
const MAX_CHARS: usize = 260;

/// The voice to use for a language when the user picked none.
pub fn default_voice(language: &str) -> &'static str {
    match language {
        "es" => "ef_dora",
        "fr" => "ff_siwis",
        "hi" => "hf_alpha",
        "it" => "if_sara",
        "pt" => "pf_dora",
        _ => "af_heart",
    }
}

/// Can Kokoro read this language here?
pub fn supports(language: &str) -> bool {
    language.is_empty() || KOKORO_LANGUAGES.contains(&language)
}

fn vocab() -> HashMap<char, i64> {
    KOKORO_VOCAB.iter().copied().collect()
}

/// The token ids of phonemes: unknown symbols are dropped, the ends get the padding token 0.
/// Returns the ids and the number of phoneme tokens (without padding).
pub fn tokenize(phonemes: &str, vocab: &HashMap<char, i64>) -> (Vec<i64>, usize) {
    let mut ids = vec![0i64];
    ids.extend(
        phonemes
            .chars()
            .filter_map(|c| vocab.get(&c).copied())
            .take(MAX_TOKENS),
    );
    let n = ids.len() - 1;
    ids.push(0);
    (ids, n)
}

/// The 256 numbers of style for a sentence of `n` tokens from the voice file (rows of 256
/// little-endian f32, one per length).
pub fn style_row(file: &[u8], n: usize) -> Result<Vec<f32>> {
    let rows = file.len() / 1024;
    if rows == 0 || !file.len().is_multiple_of(1024) {
        bail!("the voice file is damaged");
    }
    let r = n.min(rows - 1);
    Ok(file[r * 1024..(r + 1) * 1024]
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

pub struct Kokoro {
    _env: Environment,
    session: Session,
    voices_dir: PathBuf,
    vocab: HashMap<char, i64>,
    styles: HashMap<String, Vec<u8>>,
    id: String,
}

impl Kokoro {
    pub fn load(runtime_lib: &Path, model: &Path, voices_dir: &Path) -> Result<Kokoro> {
        let env = crate::onnx::environment(runtime_lib)?;
        let session = load_model(&env, model)?;
        Ok(Kokoro {
            _env: env,
            session,
            voices_dir: voices_dir.to_path_buf(),
            vocab: vocab(),
            styles: HashMap::new(),
            id: format!(
                "kokoro-{}",
                model
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("model")
            ),
        })
    }

    /// The voices on disk.
    pub fn list_voices(voices_dir: &Path) -> Vec<Voice> {
        let mut v: Vec<Voice> = std::fs::read_dir(voices_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                let name = name.strip_suffix(".bin")?.to_string();
                let language = kokoro_voice_language(&name);
                KOKORO_LANGUAGES.contains(&language).then(|| Voice {
                    name,
                    language: language.to_string(),
                })
            })
            .collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    fn style_file(&mut self, voice: &str) -> Result<&Vec<u8>> {
        if !self.styles.contains_key(voice) {
            if voice.is_empty() || voice.contains(['/', '\\', '.']) {
                bail!("\"{voice}\" is not a Kokoro voice");
            }
            let bytes = std::fs::read(self.voices_dir.join(format!("{voice}.bin")))
                .with_context(|| format!("the Kokoro voice \"{voice}\" is not downloaded"))?;
            self.styles.insert(voice.to_string(), bytes);
        }
        Ok(&self.styles[voice])
    }

    fn run(&mut self, phonemes: &str, voice: &str, speed: f64) -> Result<Pcm> {
        let (ids, n) = tokenize(phonemes, &self.vocab);
        if n == 0 {
            return Ok(Pcm {
                rate: RATE,
                samples: vec![],
            });
        }
        let style = style_row(self.style_file(voice)?, n)?;
        let len = ids.len();
        let inputs = ort::inputs![
            "input_ids" => Tensor::from_array(([1usize, len], ids)).map_err(|e| anyhow!("{e}"))?,
            "style" => Tensor::from_array(([1usize, 256], style)).map_err(|e| anyhow!("{e}"))?,
            "speed" => Tensor::from_array(([1usize], vec![speed as f32])).map_err(|e| anyhow!("{e}"))?,
        ];
        let outputs = self
            .session
            .run(inputs)
            .map_err(|e| anyhow!("the model failed: {e}"))?;
        let (_, wave) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow!("unexpected model output: {e}"))?;
        Ok(trim_silence(
            Pcm {
                rate: RATE,
                samples: wave.to_vec(),
            },
            0.004,
            40,
        ))
    }
}

impl Synthesizer for Kokoro {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn voices(&mut self) -> Result<Vec<Voice>> {
        Ok(Kokoro::list_voices(&self.voices_dir))
    }

    fn synthesize(&mut self, req: &SynthRequest) -> Result<Pcm> {
        let voice = if req.voice.is_empty() {
            default_voice(req.language).to_string()
        } else {
            req.voice.to_string()
        };
        let lang = kokoro_voice_language(&voice);
        let lang = if lang.is_empty() { req.language } else { lang };
        // British voices ("b…") use the British accent of espeak
        let espeak_lang = if voice.starts_with('b') {
            "en-gb"
        } else {
            lang
        };
        let mut out = Pcm {
            rate: RATE,
            samples: vec![],
        };
        for chunk in chunk_text(req.text, MAX_CHARS) {
            if req.cancel.is_cancelled() {
                bail!("cancelled");
            }
            let ph = phonemize(&chunk, espeak_lang)?;
            let pcm = self.run(&ph, &voice, req.speed.clamp(0.5, 2.0))?;
            out.append(&pcm, if out.samples.is_empty() { 0 } else { 60 });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phonemes_become_ids_with_padding_and_unknown_symbols_are_dropped() {
        let v = vocab();
        let (ids, n) = tokenize("həlˈoʊ, wˈɜːld.", &v);
        assert_eq!(ids[0], 0);
        assert_eq!(*ids.last().unwrap(), 0);
        assert_eq!(n, ids.len() - 2);
        // "," is 3 and "." is 4 in the vocabulary
        assert!(ids.contains(&3) && ids.contains(&4));
        let (ids, n) = tokenize("☃☃", &v);
        assert_eq!((ids, n), (vec![0, 0], 0));
    }

    #[test]
    fn the_vocabulary_has_what_espeak_produces() {
        let v = vocab();
        assert_eq!(v.len(), 115);
        for c in "ˈˌːəɪɛæɑɔʊʌðθŋʃʒɹ".chars() {
            assert!(v.contains_key(&c), "{c} missing");
        }
        assert_eq!(v[&' '], 16);
    }

    #[test]
    fn long_phoneme_strings_are_cut_below_the_models_limit() {
        let v = vocab();
        let (ids, n) = tokenize(&"a".repeat(2000), &v);
        assert_eq!(n, MAX_TOKENS);
        assert_eq!(ids.len(), MAX_TOKENS + 2);
    }

    #[test]
    fn the_style_row_depends_on_the_length() {
        let mut f = Vec::new();
        for row in 0..4u32 {
            for _ in 0..256 {
                f.extend_from_slice(&(row as f32).to_le_bytes());
            }
        }
        assert_eq!(style_row(&f, 0).unwrap()[0], 0.0);
        assert_eq!(style_row(&f, 2).unwrap()[255], 2.0);
        assert_eq!(
            style_row(&f, 99).unwrap()[0],
            3.0,
            "past the end: the last row"
        );
        assert!(style_row(&f[..100], 1).is_err());
        assert!(style_row(&[], 1).is_err());
    }

    #[test]
    fn languages_and_default_voices_fit_together() {
        for l in KOKORO_LANGUAGES {
            assert_eq!(kokoro_voice_language(default_voice(l)), l, "{l}");
        }
        assert!(supports("en") && supports("") && !supports("de") && !supports("ja"));
    }

    #[test]
    fn voices_are_listed_from_the_directory() {
        let d = std::env::temp_dir().join(format!("bookx-kv-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        for n in [
            "af_heart.bin",
            "bm_george.bin",
            "zf_xiaobei.bin",
            "ff_siwis.bin",
            "notes.txt",
        ] {
            std::fs::write(d.join(n), b"x").unwrap();
        }
        let v = Kokoro::list_voices(&d);
        assert_eq!(
            v.iter()
                .map(|v| (v.name.as_str(), v.language.as_str()))
                .collect::<Vec<_>>(),
            [("af_heart", "en"), ("bm_george", "en"), ("ff_siwis", "fr")],
            "Chinese is not offered: no phonemizer for it"
        );
        std::fs::remove_dir_all(&d).ok();
    }
}
