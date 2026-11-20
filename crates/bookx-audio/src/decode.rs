//! Decoding with Symphonia (pure Rust): MP3, AAC/M4A/M4B, ALAC, Vorbis, FLAC, WAV/PCM.
//! Opus is not decodable (Symphonia has no Opus decoder); such files are listed but not played.

use std::fs::File;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder as SymDecoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

/// Stream parameters read from the container.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreamInfo {
    pub rate: u32,
    pub channels: u8,
    pub duration_ms: u64,
}

fn open_reader(path: &Path) -> Result<Box<dyn FormatReader>> {
    let f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(f), Default::default());
    let mut hint = Hint::new();
    if let Some(e) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(e);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )
        .map_err(|e| anyhow!("{}: {e}", path.display()))?;
    Ok(probed.format)
}

/// Rate, channels and duration of the first audio track. The duration comes from the container;
/// when it has none (MP3 without a Xing header) the packets are counted without decoding them.
pub fn stream_info(path: &Path) -> Option<StreamInfo> {
    let mut format = open_reader(path).ok()?;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)?
        .clone();
    let rate = track.codec_params.sample_rate?;
    let channels = track
        .codec_params
        .channels
        .map(|c| c.count() as u8)
        .unwrap_or(2);
    let mut frames = track.codec_params.n_frames;
    if frames.is_none() {
        let mut n = 0u64;
        let mut guard = 0u64;
        while let Ok(p) = format.next_packet() {
            guard += 1;
            if guard > 50_000_000 {
                break;
            }
            if p.track_id() == track.id {
                n += p.dur;
            }
        }
        if n > 0 {
            // `dur` is in the track's time base; for audio tracks that is 1 / rate
            frames = Some(match track.codec_params.time_base {
                Some(tb) => {
                    (n as u128 * tb.numer as u128 * rate as u128 / tb.denom.max(1) as u128) as u64
                }
                None => n,
            });
        }
    }
    Some(StreamInfo {
        rate,
        channels,
        duration_ms: frames.map(|f| f * 1000 / rate as u64).unwrap_or(0),
    })
}

/// A seekable stream of decoded samples.
pub struct Decoder {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn SymDecoder>,
    track_id: u32,
    pub rate: u32,
    pub channels: u8,
    /// Frames to throw away after a seek that landed before the wanted time.
    skip: u64,
    time_base_is_rate: bool,
    buf: Option<SampleBuffer<f32>>,
}

impl Decoder {
    pub fn open(path: &Path) -> Result<Decoder> {
        let reader = open_reader(path)?;
        let track = reader
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| anyhow!("no audio track in {}", path.display()))?
            .clone();
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|e| anyhow!("cannot decode {}: {e}", path.display()))?;
        let rate = track.codec_params.sample_rate.unwrap_or(44_100);
        let channels = track
            .codec_params
            .channels
            .map(|c| c.count() as u8)
            .unwrap_or(2);
        let time_base_is_rate = track
            .codec_params
            .time_base
            .map(|tb| tb.numer == 1 && tb.denom == rate)
            .unwrap_or(true);
        Ok(Decoder {
            reader,
            decoder,
            track_id: track.id,
            rate,
            channels: channels.max(1),
            skip: 0,
            time_base_is_rate,
            buf: None,
        })
    }

    /// Moves to `ms` from the start of the file.
    pub fn seek_ms(&mut self, ms: u64) -> Result<()> {
        let secs = ms / 1000;
        let frac = (ms % 1000) as f64 / 1000.0;
        let to = SeekTo::Time {
            time: Time::new(secs, frac),
            track_id: Some(self.track_id),
        };
        match self.reader.seek(SeekMode::Accurate, to) {
            Ok(s) => {
                self.decoder.reset();
                self.skip = if self.time_base_is_rate {
                    s.required_ts.saturating_sub(s.actual_ts)
                } else {
                    0
                };
                Ok(())
            }
            Err(SymError::SeekError(_)) | Err(SymError::Unsupported(_)) => {
                bail!("cannot seek in this file")
            }
            Err(e) => Err(anyhow!("seek failed: {e}")),
        }
    }

    /// Appends the next decoded packet (interleaved `f32`) to `out`. `Ok(false)` at the end.
    pub fn next_chunk(&mut self, out: &mut Vec<f32>) -> Result<bool> {
        let mut bad = 0;
        loop {
            let packet = match self.reader.next_packet() {
                Ok(p) => p,
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(false)
                }
                Err(SymError::ResetRequired) => return Ok(false),
                Err(e) => return Err(anyhow!("read failed: {e}")),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(audio) => {
                    let spec = *audio.spec();
                    self.rate = spec.rate;
                    self.channels = spec.channels.count().max(1) as u8;
                    let need = audio.capacity() as u64;
                    if self
                        .buf
                        .as_ref()
                        .map(|b| (b.capacity() as u64) < need * spec.channels.count() as u64)
                        .unwrap_or(true)
                    {
                        self.buf = Some(SampleBuffer::<f32>::new(need, spec));
                    }
                    let buf = self.buf.as_mut().unwrap();
                    buf.copy_interleaved_ref(audio);
                    let mut s = buf.samples();
                    if self.skip > 0 {
                        let drop = (self.skip as usize * self.channels as usize).min(s.len());
                        self.skip -= (drop / self.channels as usize) as u64;
                        s = &s[drop..];
                    }
                    if s.is_empty() {
                        continue;
                    }
                    out.extend_from_slice(s);
                    return Ok(true);
                }
                Err(SymError::DecodeError(_)) | Err(SymError::IoError(_)) => {
                    bad += 1;
                    if bad > 50 {
                        bail!("too many damaged packets");
                    }
                }
                Err(e) => return Err(anyhow!("decode failed: {e}")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgen::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-audio-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d.join(name)
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    fn drain(d: &mut Decoder) -> Vec<f32> {
        let mut v = Vec::new();
        while d.next_chunk(&mut v).unwrap() {}
        v
    }

    #[test]
    fn wav_decodes_to_the_same_samples() {
        let p = tmp("t.wav");
        std::fs::write(&p, wav_tone(8000, 500, 440.0)).unwrap();
        let mut d = Decoder::open(&p).unwrap();
        assert_eq!((d.rate, d.channels), (8000, 1));
        let s = drain(&mut d);
        assert_eq!(s.len(), 4000);
        assert!(s.iter().any(|x| x.abs() > 0.1));
    }

    #[test]
    fn seeking_a_wav_lands_on_the_right_sample() {
        let p = tmp("s.wav");
        let samples: Vec<i16> = (0..16000).map(|i| (i % 30000) as i16).collect();
        std::fs::write(&p, wav_samples(8000, 1, &samples)).unwrap();
        let mut d = Decoder::open(&p).unwrap();
        d.seek_ms(1000).unwrap();
        let mut v = Vec::new();
        d.next_chunk(&mut v).unwrap();
        let first = (v[0] * 32768.0).round() as i32;
        assert!((first - 8000).abs() <= 1, "{first}");
        let rest = {
            let mut r = v.clone();
            while d.next_chunk(&mut r).unwrap() {}
            r
        };
        assert_eq!(rest.len(), 8000);
    }

    #[test]
    fn generated_flac_decodes() {
        let p = tmp("q.flac");
        std::fs::write(&p, flac_silence(8000, 10_000, &[])).unwrap();
        let mut d = Decoder::open(&p).unwrap();
        let s = drain(&mut d);
        assert_eq!(s.len(), 10_000);
        assert!(s.iter().all(|&x| x == 0.0));
        let i = stream_info(&p).unwrap();
        assert_eq!((i.rate, i.channels, i.duration_ms), (8000, 1, 1250));
    }

    #[test]
    fn real_mp3_aac_ogg_flac_decode_with_the_right_length() {
        for (name, ms) in [
            ("tiny.mp3", 2000u64),
            ("tiny.m4b", 2500),
            ("tiny.ogg", 2000),
            ("tiny.flac", 2000),
        ] {
            let mut d = Decoder::open(&fixture(name)).unwrap();
            let s = drain(&mut d);
            let got = s.len() as u64 * 1000 / (d.rate as u64 * d.channels as u64);
            // AAC adds encoder priming (about 190 ms at 8 kHz) that the file does not declare
            assert!(
                got.abs_diff(ms) <= 250,
                "{name}: {got} ms, wanted about {ms}"
            );
            assert!(s.iter().any(|x| x.abs() > 0.05), "{name} is not silent");
            let i = stream_info(&fixture(name)).unwrap();
            assert!(
                i.duration_ms.abs_diff(ms) <= 150,
                "{name}: info {}",
                i.duration_ms
            );
        }
    }

    #[test]
    fn seeking_in_compressed_files_skips_the_right_amount() {
        for name in ["tiny.mp3", "tiny.m4b", "tiny.ogg", "tiny.flac"] {
            let mut d = Decoder::open(&fixture(name)).unwrap();
            let total = drain(&mut d).len();
            let mut d = Decoder::open(&fixture(name)).unwrap();
            d.seek_ms(1000).unwrap();
            let rest = drain(&mut d).len();
            let want = total - (d.rate as usize * d.channels as usize);
            let tol = d.rate as usize * d.channels as usize / 10;
            assert!(rest.abs_diff(want) <= tol, "{name}: {rest} vs {want}");
        }
    }

    #[test]
    fn opus_and_junk_are_refused_not_crashed() {
        assert!(Decoder::open(&fixture("tiny.opus")).is_err());
        let p = tmp("junk.mp3");
        std::fs::write(&p, vec![0x55u8; 5000]).unwrap();
        assert!(Decoder::open(&p).is_err());
        assert!(stream_info(&p).is_none());
        assert!(Decoder::open(&p.with_file_name("none.mp3")).is_err());
    }
}
