//! Reading a chapter into one audio file (WAV: no encoder is needed; `ffmpeg` converts to MP3
//! or OGG). [`synthesize_to_wav`] streams to a file so a long chapter does not sit in memory; the
//! reader's *Save as audio…* uses it from a worker thread.

use std::io::{Seek, SeekFrom, Write};

use anyhow::{bail, Result};

use crate::synth::{engine_max_speed, split_speed, Cancel, Pcm, SynthRequest, Synthesizer};
use crate::text::Sentence;

/// Synthesizes the sentences one after the other with `gap_ms` between them. `progress`
/// gets (done, total) after each sentence.
#[allow(clippy::too_many_arguments)]
pub fn synthesize_all(
    synth: &mut dyn Synthesizer,
    sentences: &[Sentence],
    voice: &str,
    language: &str,
    speed: f64,
    pitch: f64,
    gap_ms: u32,
    cancel: &Cancel,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Pcm> {
    let mut out = Pcm::default();
    let (model_speed, stretch) = split_speed(speed / pitch, engine_max_speed(&synth.id()));
    for (i, s) in sentences.iter().enumerate() {
        if cancel.is_cancelled() {
            bail!("cancelled");
        }
        let pcm = synth
            .synthesize(&SynthRequest {
                text: &s.text,
                voice,
                language,
                speed: model_speed,
                cancel,
            })?
            .time_stretch(stretch)
            .with_pitch(pitch);
        out.append(&pcm, if i == 0 { 0 } else { gap_ms });
        progress(i + 1, sentences.len());
    }
    Ok(out)
}

/// Writes a 16-bit mono WAV file sample by sample; the sizes in the header are patched at the end.
pub struct WavWriter<W: Write + Seek> {
    w: W,
    rate: u32,
    data_bytes: u32,
}

impl<W: Write + Seek> WavWriter<W> {
    pub fn new(mut w: W, rate: u32) -> std::io::Result<Self> {
        let rate = rate.max(1);
        w.write_all(b"RIFF\0\0\0\0WAVEfmt ")?;
        w.write_all(&16u32.to_le_bytes())?;
        w.write_all(&1u16.to_le_bytes())?;
        w.write_all(&1u16.to_le_bytes())?;
        w.write_all(&rate.to_le_bytes())?;
        w.write_all(&(rate * 2).to_le_bytes())?;
        w.write_all(&2u16.to_le_bytes())?;
        w.write_all(&16u16.to_le_bytes())?;
        w.write_all(b"data\0\0\0\0")?;
        Ok(WavWriter {
            w,
            rate,
            data_bytes: 0,
        })
    }

    /// Appends `gap_ms` of silence and then `pcm` (resampled when its rate differs).
    pub fn push(&mut self, pcm: &Pcm, gap_ms: u32) -> std::io::Result<()> {
        let gap = (self.rate as u64 * gap_ms as u64 / 1000) as usize;
        let mut out = vec![0i16; gap];
        let body = if pcm.rate == self.rate {
            pcm.to_i16()
        } else {
            Pcm {
                rate: self.rate,
                samples: crate::synth::resample(&pcm.samples, pcm.rate, self.rate),
            }
            .to_i16()
        };
        out.extend(body);
        let bytes: Vec<u8> = out.iter().flat_map(|s| s.to_le_bytes()).collect();
        self.w.write_all(&bytes)?;
        self.data_bytes = self.data_bytes.saturating_add(bytes.len() as u32);
        Ok(())
    }

    /// Seconds written so far.
    pub fn seconds(&self) -> f64 {
        self.data_bytes as f64 / 2.0 / self.rate as f64
    }

    pub fn finish(mut self) -> std::io::Result<W> {
        self.w.seek(SeekFrom::Start(4))?;
        self.w
            .write_all(&(36u32.saturating_add(self.data_bytes)).to_le_bytes())?;
        self.w.seek(SeekFrom::Start(40))?;
        self.w.write_all(&self.data_bytes.to_le_bytes())?;
        self.w.seek(SeekFrom::End(0))?;
        self.w.flush()?;
        Ok(self.w)
    }
}

/// Like [`synthesize_all`], but writes a WAV file to `out` as it goes. Returns `out` and the
/// seconds of audio. `progress` gets (done, total) after each sentence. Nothing is written for
/// an empty list of sentences (the error says so).
#[allow(clippy::too_many_arguments)]
pub fn synthesize_to_wav<W: Write + Seek>(
    synth: &mut dyn Synthesizer,
    sentences: &[Sentence],
    voice: &str,
    language: &str,
    speed: f64,
    pitch: f64,
    gap_ms: u32,
    cancel: &Cancel,
    progress: &mut dyn FnMut(usize, usize),
    out: W,
) -> Result<(W, f64)> {
    if sentences.is_empty() {
        bail!("there is no text to read");
    }
    let (model_speed, stretch) = split_speed(speed / pitch, engine_max_speed(&synth.id()));
    let mut out = Some(out);
    let mut writer: Option<WavWriter<W>> = None;
    for (i, s) in sentences.iter().enumerate() {
        if cancel.is_cancelled() {
            bail!("cancelled");
        }
        let pcm = synth
            .synthesize(&SynthRequest {
                text: &s.text,
                voice,
                language,
                speed: model_speed,
                cancel,
            })?
            .time_stretch(stretch)
            .with_pitch(pitch);
        if writer.is_none() {
            writer = Some(WavWriter::new(out.take().expect("one output"), pcm.rate)?);
        }
        if let Some(w) = writer.as_mut() {
            w.push(&pcm, if i == 0 { 0 } else { gap_ms })?;
        }
        progress(i + 1, sentences.len());
    }
    let w = writer.expect("at least one sentence");
    let secs = w.seconds();
    Ok((w.finish()?, secs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Voice;
    use crate::text::split_sentences;

    struct Beep;
    impl Synthesizer for Beep {
        fn id(&self) -> String {
            "beep".into()
        }
        fn voices(&mut self) -> Result<Vec<Voice>> {
            Ok(vec![])
        }
        fn synthesize(&mut self, r: &SynthRequest) -> Result<Pcm> {
            let n = (r.text.len() as f64 * 100.0 / r.speed) as usize;
            Ok(Pcm {
                rate: 8000,
                samples: vec![0.2; n],
            })
        }
    }

    #[test]
    fn a_chapter_becomes_one_wav_with_gaps_between_sentences() {
        let s = split_sentences("One. Two. Three.");
        let mut seen = Vec::new();
        let pcm = synthesize_all(
            &mut Beep,
            &s,
            "",
            "en",
            1.0,
            1.0,
            250,
            &Cancel::new(),
            &mut |d, t| seen.push((d, t)),
        )
        .unwrap();
        // 4 + 4 + 6 characters at 100 samples, two gaps of 250 ms at 8 kHz
        assert_eq!(pcm.samples.len(), 1400 + 2 * 2000);
        assert_eq!(seen, [(1, 3), (2, 3), (3, 3)]);
        let wav = pcm.to_wav();
        assert_eq!(
            crate::synth::parse_wav(&wav).unwrap().samples.len(),
            pcm.samples.len()
        );
    }

    #[test]
    fn the_streamed_wav_is_the_same_file_as_the_in_memory_one() {
        let s = split_sentences("One. Two. Three.");
        let mut seen = Vec::new();
        let (cur, secs) = synthesize_to_wav(
            &mut Beep,
            &s,
            "",
            "en",
            1.0,
            1.0,
            250,
            &Cancel::new(),
            &mut |d, t| seen.push((d, t)),
            std::io::Cursor::new(Vec::new()),
        )
        .unwrap();
        let bytes = cur.into_inner();
        let pcm = synthesize_all(
            &mut Beep,
            &s,
            "",
            "en",
            1.0,
            1.0,
            250,
            &Cancel::new(),
            &mut |_, _| {},
        )
        .unwrap();
        assert_eq!(bytes, pcm.to_wav(), "header sizes included");
        assert!((secs - pcm.duration_secs()).abs() < 1e-9);
        assert_eq!(seen.last(), Some(&(3, 3)));
    }

    #[test]
    fn nothing_to_read_and_cancel_are_errors_that_write_no_file_body() {
        let e = synthesize_to_wav(
            &mut Beep,
            &[],
            "",
            "en",
            1.0,
            1.0,
            0,
            &Cancel::new(),
            &mut |_, _| {},
            std::io::Cursor::new(Vec::new()),
        )
        .unwrap_err();
        assert!(e.to_string().contains("no text"));
        let c = Cancel::new();
        c.cancel();
        let e = synthesize_to_wav(
            &mut Beep,
            &split_sentences("One."),
            "",
            "en",
            1.0,
            1.0,
            0,
            &c,
            &mut |_, _| {},
            std::io::Cursor::new(Vec::new()),
        )
        .unwrap_err();
        assert_eq!(e.to_string(), "cancelled");
    }

    #[test]
    fn cancelling_stops_the_export() {
        let s = split_sentences("One. Two.");
        let c = Cancel::new();
        c.cancel();
        let e =
            synthesize_all(&mut Beep, &s, "", "en", 1.0, 1.0, 0, &c, &mut |_, _| {}).unwrap_err();
        assert_eq!(e.to_string(), "cancelled");
    }
}
