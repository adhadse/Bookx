//! Synthesizers (text to audio samples) and what to do with the samples: WAV in and out,
//! resampling for the pitch, joining.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};

use crate::session::Voice;

/// Mono audio, `-1.0..=1.0`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Pcm {
    pub rate: u32,
    pub samples: Vec<f32>,
}

impl Pcm {
    pub fn duration_secs(&self) -> f64 {
        if self.rate == 0 {
            0.0
        } else {
            self.samples.len() as f64 / self.rate as f64
        }
    }

    pub fn rms(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let sum: f64 = self.samples.iter().map(|s| (*s as f64).powi(2)).sum();
        (sum / self.samples.len() as f64).sqrt()
    }

    pub fn to_i16(&self) -> Vec<i16> {
        self.samples
            .iter()
            .map(|s| (s.clamp(-1.0, 1.0) * 32767.0).round() as i16)
            .collect()
    }

    /// Appends `other` (same rate) after `gap_ms` of silence.
    pub fn append(&mut self, other: &Pcm, gap_ms: u32) {
        if self.rate == 0 {
            self.rate = other.rate;
        }
        let gap = (self.rate as u64 * gap_ms as u64 / 1000) as usize;
        self.samples.extend(std::iter::repeat_n(0.0, gap));
        if other.rate == self.rate {
            self.samples.extend_from_slice(&other.samples);
        } else {
            self.samples
                .extend(resample(&other.samples, other.rate, self.rate));
        }
    }

    /// Raises the pitch by `factor` (1.0 = unchanged) by playing the samples faster. This
    /// also shortens the audio by the same factor; the caller asks the engine for a speed of
    /// `speed / factor` to make up for it.
    pub fn with_pitch(self, factor: f64) -> Pcm {
        if !factor.is_finite() || (factor - 1.0).abs() < 0.005 || factor <= 0.0 {
            return self;
        }
        // the samples at `rate * factor` are played at `rate`: resample to rate / factor
        let from = (self.rate as f64 * factor).round() as u32;
        Pcm {
            rate: self.rate,
            samples: resample(&self.samples, from, self.rate),
        }
    }

    /// A 16-bit mono WAV file.
    pub fn to_wav(&self) -> Vec<u8> {
        let data = self.to_i16();
        let bytes = data.len() as u32 * 2;
        let mut w = Vec::with_capacity(44 + bytes as usize);
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + bytes).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&self.rate.to_le_bytes());
        w.extend_from_slice(&(self.rate * 2).to_le_bytes());
        w.extend_from_slice(&2u16.to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&bytes.to_le_bytes());
        for s in data {
            w.extend_from_slice(&s.to_le_bytes());
        }
        w
    }
}

/// The fastest speed the local engines (Kokoro, Piper) are asked for; servers take up to 4.
pub const ENGINE_MAX_SPEED: f64 = 2.0;

/// The most any speed is stretched afterwards (the limit of `bookx_audio::stretch`).
pub const MAX_STRETCH: f64 = 3.0;

/// The engine's own speed limit, from its cache id (servers are `server-…`).
pub fn engine_max_speed(engine_id: &str) -> f64 {
    if engine_id.starts_with("server-") {
        4.0
    } else {
        ENGINE_MAX_SPEED
    }
}

/// Splits the wanted speed (`speed / pitch`) into what the engine is asked for and a factor to
/// speed the audio up afterwards without changing its pitch (1.0 = none). The model speed never
/// exceeds `max`, the stretch never exceeds [`MAX_STRETCH`]; their product is the wanted speed.
pub fn split_speed(wanted: f64, max: f64) -> (f64, f64) {
    let wanted = if wanted.is_finite() { wanted } else { 1.0 }.clamp(0.25, 4.0);
    let model = wanted.min(max);
    (model, (wanted / model).clamp(1.0, MAX_STRETCH))
}

impl Pcm {
    /// Plays `factor` times faster at the same pitch (WSOLA, `bookx_audio::stretch`). Below 1.02
    /// the audio is returned unchanged.
    pub fn time_stretch(self, factor: f64) -> Pcm {
        if !factor.is_finite() || factor < 1.02 || self.samples.is_empty() || self.rate == 0 {
            return self;
        }
        let mut st = bookx_audio::stretch::Stretcher::new(self.rate, 1, factor as f32);
        let mut out = Vec::with_capacity((self.samples.len() as f64 / factor) as usize + 4096);
        st.push(&self.samples, &mut out);
        st.finish(&mut out);
        Pcm {
            rate: self.rate,
            samples: out,
        }
    }
}

/// Linear resampling from `from` Hz to `to` Hz.
pub fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() || from == 0 || to == 0 {
        return samples.to_vec();
    }
    let n = (samples.len() as u64 * to as u64 / from as u64) as usize;
    let step = from as f64 / to as f64;
    (0..n)
        .map(|i| {
            let pos = i as f64 * step;
            let a = pos as usize;
            let frac = (pos - a as f64) as f32;
            let x = samples[a.min(samples.len() - 1)];
            let y = samples[(a + 1).min(samples.len() - 1)];
            x + (y - x) * frac
        })
        .collect()
}

/// Reads a WAV file: 8/16/24/32-bit integer or 32/64-bit float, any channel count (mixed down
/// to mono). A data size that is zero or too big (servers that stream the file) means "up to
/// the end".
pub fn parse_wav(bytes: &[u8]) -> Result<Pcm> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        bail!("the server did not answer with a WAV file");
    }
    let mut pos = 12usize;
    let (mut format, mut channels, mut rate, mut bits) = (0u16, 0u16, 0u32, 0u16);
    let mut data: Option<&[u8]> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = pos + 8;
        if id == b"fmt " {
            if body + 16 > bytes.len() {
                bail!("the WAV header is cut off");
            }
            format = u16::from_le_bytes([bytes[body], bytes[body + 1]]);
            channels = u16::from_le_bytes([bytes[body + 2], bytes[body + 3]]);
            rate = u32::from_le_bytes(bytes[body + 4..body + 8].try_into().unwrap());
            bits = u16::from_le_bytes([bytes[body + 14], bytes[body + 15]]);
            if format == 0xfffe && size >= 26 && body + 26 <= bytes.len() {
                // WAVE_FORMAT_EXTENSIBLE: the real format is the first two bytes of the GUID
                format = u16::from_le_bytes([bytes[body + 24], bytes[body + 25]]);
            }
        } else if id == b"data" {
            let end = if size == 0 || size == 0xffff_ffff || body + size > bytes.len() {
                bytes.len()
            } else {
                body + size
            };
            data = Some(&bytes[body..end]);
            break;
        }
        pos = body + size + (size & 1);
    }
    let data = data.ok_or_else(|| anyhow!("the WAV file has no audio"))?;
    if channels == 0 || rate == 0 {
        bail!("the WAV header is incomplete");
    }
    let frame = (bits as usize / 8) * channels as usize;
    if frame == 0 {
        bail!("the WAV header is incomplete");
    }
    let mut mono = Vec::with_capacity(data.len() / frame);
    for f in data.chunks_exact(frame) {
        let mut acc = 0.0f32;
        for c in 0..channels as usize {
            let b = &f[c * (bits as usize / 8)..];
            acc += match (format, bits) {
                (1, 8) => (b[0] as f32 - 128.0) / 128.0,
                (1, 16) => i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0,
                (1, 24) => (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
                (1, 32) => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
                (3, 32) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                (3, 64) => f64::from_le_bytes(b[..8].try_into().unwrap()) as f32,
                _ => bail!("WAV format {format} with {bits} bits is not supported"),
            };
        }
        mono.push(acc / channels as f32);
    }
    Ok(Pcm {
        rate,
        samples: mono,
    })
}

/// Raw little-endian 16-bit mono samples.
pub fn pcm_from_s16le(bytes: &[u8], rate: u32) -> Pcm {
    Pcm {
        rate,
        samples: bytes
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect(),
    }
}

/// Lets the reader abandon a synthesis that is not wanted any more.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Cancel {
        Cancel::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

pub struct SynthRequest<'a> {
    pub text: &'a str,
    /// The voice picked by the user; empty = the engine chooses for the language.
    pub voice: &'a str,
    /// Two letters, may be empty.
    pub language: &'a str,
    /// 1.0 = normal.
    pub speed: f64,
    pub cancel: &'a Cancel,
}

/// An engine that turns one piece of text into audio. Runs in the worker thread of the
/// pipeline, so it may block.
pub trait Synthesizer: Send {
    /// Identifies the engine and its model in cache keys.
    fn id(&self) -> String;
    fn voices(&mut self) -> Result<Vec<Voice>>;
    fn synthesize(&mut self, req: &SynthRequest) -> Result<Pcm>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_above_what_an_engine_takes_is_made_up_by_stretching() {
        assert_eq!(split_speed(1.5, 2.0), (1.5, 1.0));
        let (m, s) = split_speed(2.5, 2.0);
        assert_eq!(m, 2.0);
        assert!((m * s - 2.5).abs() < 1e-9);
        // a server takes it all
        assert_eq!(
            split_speed(2.5, engine_max_speed("server-openai-x")),
            (2.5, 1.0)
        );
        assert_eq!(engine_max_speed("kokoro"), 2.0);
        // never more than the stretcher allows, never NaN
        let (m, s) = split_speed(4.0, 0.5);
        assert!(m <= 0.5 && s <= MAX_STRETCH);
        assert_eq!(split_speed(f64::NAN, 2.0), (1.0, 1.0));
    }

    #[test]
    fn stretched_speech_is_shorter_by_the_factor_and_keeps_its_pitch() {
        let rate = 16000;
        let p = Pcm {
            rate,
            samples: (0..rate as usize * 2)
                .map(|i| (i as f32 / rate as f32 * 200.0 * std::f32::consts::TAU).sin() * 0.5)
                .collect(),
        };
        let q = p.clone().time_stretch(1.5);
        let ratio = p.samples.len() as f64 / q.samples.len() as f64;
        assert!((ratio - 1.5).abs() < 0.1, "{ratio}");
        // zero crossings per second stay at about 2 x 200
        let zc = |x: &Pcm| {
            x.samples
                .windows(2)
                .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
                .count() as f64
                / x.duration_secs()
        };
        assert!((zc(&q) - zc(&p)).abs() < 25.0, "{} vs {}", zc(&q), zc(&p));
        assert_eq!(p.clone().time_stretch(1.0), p, "no change below 1.02");
    }

    fn sine(rate: u32, hz: f32, secs: f32) -> Pcm {
        Pcm {
            rate,
            samples: (0..(rate as f32 * secs) as usize)
                .map(|i| (i as f32 / rate as f32 * hz * std::f32::consts::TAU).sin() * 0.5)
                .collect(),
        }
    }

    #[test]
    fn wav_round_trips_and_has_a_valid_header() {
        let p = sine(24000, 440.0, 0.1);
        let w = p.to_wav();
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(w.len(), 44 + p.samples.len() * 2);
        let back = parse_wav(&w).unwrap();
        assert_eq!(back.rate, 24000);
        assert_eq!(back.samples.len(), p.samples.len());
        for (a, b) in p.samples.iter().zip(&back.samples) {
            assert!((a - b).abs() < 1e-3);
        }
    }

    #[test]
    fn wav_readers_cope_with_float_stereo_and_streamed_sizes() {
        // 2 frames, stereo, float32, data size 0xffffffff like a streamed answer
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF\xff\xff\xff\xffWAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        for v in [3u16, 2] {
            w.extend_from_slice(&v.to_le_bytes());
        }
        w.extend_from_slice(&16000u32.to_le_bytes());
        w.extend_from_slice(&(16000u32 * 8).to_le_bytes());
        w.extend_from_slice(&8u16.to_le_bytes());
        w.extend_from_slice(&32u16.to_le_bytes());
        w.extend_from_slice(b"data\xff\xff\xff\xff");
        for v in [0.5f32, -0.5, 1.0, 0.0] {
            w.extend_from_slice(&v.to_le_bytes());
        }
        let p = parse_wav(&w).unwrap();
        assert_eq!(p.rate, 16000);
        assert_eq!(p.samples, vec![0.0, 0.5]);
    }

    #[test]
    fn wav_errors_are_readable() {
        assert!(parse_wav(b"<html>nope</html>").is_err());
        assert!(parse_wav(b"RIFF\0\0\0\0WAVE").is_err());
        let mut p = sine(8000, 100.0, 0.01).to_wav();
        p[34] = 7; // bits per sample
        assert!(parse_wav(&p).is_err());
    }

    #[test]
    fn resampling_changes_the_length_by_the_ratio() {
        let p = sine(16000, 200.0, 0.5);
        assert_eq!(resample(&p.samples, 16000, 16000).len(), p.samples.len());
        assert_eq!(resample(&p.samples, 16000, 24000).len(), 12000);
        assert_eq!(resample(&p.samples, 16000, 8000).len(), 4000);
        assert!(resample(&[], 1, 2).is_empty());
    }

    #[test]
    fn a_higher_pitch_is_shorter_and_one_is_untouched() {
        let p = sine(24000, 200.0, 1.0);
        let same = p.clone().with_pitch(1.0);
        assert_eq!(same, p);
        let up = p.clone().with_pitch(1.25);
        assert!(
            (up.duration_secs() - 0.8).abs() < 0.01,
            "{}",
            up.duration_secs()
        );
        let down = p.with_pitch(0.8);
        assert!((down.duration_secs() - 1.25).abs() < 0.01);
        assert_eq!(down.rate, 24000);
    }

    #[test]
    fn pitch_changes_the_frequency_of_a_tone() {
        fn crossings(p: &Pcm) -> usize {
            p.samples
                .windows(2)
                .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
                .count()
        }
        let p = sine(24000, 200.0, 1.0);
        let up = p.clone().with_pitch(1.25);
        let hz = crossings(&up) as f64 / up.duration_secs();
        assert!((hz - 250.0).abs() < 6.0, "{hz}");
        assert!((crossings(&p) as f64 - 200.0).abs() < 3.0);
    }

    #[test]
    fn appending_inserts_a_gap_and_converts_the_rate() {
        let mut a = sine(24000, 100.0, 0.1);
        let b = sine(12000, 100.0, 0.1);
        a.append(&b, 50);
        assert_eq!(a.samples.len(), 2400 + 1200 + 2400);
        let mut empty = Pcm::default();
        empty.append(&b, 0);
        assert_eq!(empty.rate, 12000);
    }

    #[test]
    fn loudness_and_duration_are_measured() {
        let p = sine(24000, 440.0, 0.5);
        assert!((p.rms() - 0.3535).abs() < 0.01);
        assert!((p.duration_secs() - 0.5).abs() < 1e-6);
        assert_eq!(Pcm::default().rms(), 0.0);
        let raw = pcm_from_s16le(&[0, 0x40, 0, 0xc0], 8000);
        assert_eq!(raw.samples, vec![0.5, -0.5]);
    }

    #[test]
    fn cancel_is_shared_between_clones() {
        let c = Cancel::new();
        let d = c.clone();
        assert!(!d.is_cancelled());
        c.cancel();
        assert!(d.is_cancelled());
    }
}
