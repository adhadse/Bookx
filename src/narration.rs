//! Narration of EPUBs (CE-422), the glue between the editor and the engines: the read-aloud engines
//! of `bookx-tts` behind `bookx_editing::media_overlay_narrate::SentenceSynth`, and the numbers shown
//! before the long run starts. No GTK in here, so it is unit tested; the dialogs are in
//! `ui/editor/narration.rs`.

use anyhow::Result;
use bookx_editing::media_overlay_narrate::{Audio, SentenceSynth};
use bookx_tts::synth::{Cancel, SynthRequest, Synthesizer};

use crate::i18n::{tr, trf};

/// A [`Synthesizer`] with the voice, speed and pitch of the settings.
pub struct TtsSynth {
    pub synth: Box<dyn Synthesizer>,
    pub voice: String,
    pub language: String,
    pub speed: f64,
    pub pitch: f64,
    pub cancel: Cancel,
}

impl SentenceSynth for TtsSynth {
    fn synth(&mut self, text: &str) -> Result<Audio> {
        let model_speed = (self.speed / self.pitch).clamp(0.25, 4.0);
        let pcm = self
            .synth
            .synthesize(&SynthRequest {
                text,
                voice: &self.voice,
                language: &self.language,
                speed: model_speed,
                cancel: &self.cancel,
            })?
            .with_pitch(self.pitch);
        Ok(Audio {
            rate: pcm.rate,
            samples: pcm.to_i16(),
        })
    }
}

/// Characters of text per second of speech (about 150 words a minute).
const CHARS_PER_SECOND: f64 = 14.0;
/// 16-bit mono at 24 kHz.
const BYTES_PER_SECOND: f64 = 48_000.0;

/// How long the narration of `chars` characters of text is, in seconds.
pub fn estimated_seconds(chars: usize) -> u64 {
    (chars as f64 / CHARS_PER_SECOND).round() as u64
}

/// About how many bytes the WAV files take.
pub fn estimated_bytes(chars: usize) -> u64 {
    (estimated_seconds(chars) as f64 * BYTES_PER_SECOND) as u64
}

/// "about 2 h 5 min" / "about 12 min".
pub fn duration_text(seconds: u64) -> String {
    let (h, m) = (seconds / 3600, seconds / 60 % 60);
    if h > 0 {
        trf("{} h {} min", &[&h, &m])
    } else {
        trf("{} min", &[&m.max(1)])
    }
}

/// The body of the confirmation dialog.
pub fn confirm_text(chapters: usize, chars: usize) -> String {
    format!(
        "{}\n\n{}",
        trf(
            "{} chapters will be read aloud with the voice chosen in Preferences → Text to Speech and saved inside the book as audio (WAV). The audio is about {} long and takes about {} of disk space; synthesizing it can take much longer than listening.",
            &[
                &chapters,
                &duration_text(estimated_seconds(chars)),
                &bookx_core::util::human_size(estimated_bytes(chars))
            ],
        ),
        tr("You can cancel at any time: finished chapters are kept and running this again continues with the rest. Everything can be undone with Restore Checkpoint.")
    )
}

/// The toast after a run.
pub fn summary(narrated: usize, cancelled: bool, duration_ms: u64) -> String {
    let t = duration_text(duration_ms / 1000);
    match (narrated, cancelled) {
        (0, _) => tr("Nothing was narrated"),
        (n, false) => trf("Narrated {} chapters ({})", &[&n, &t]),
        (n, true) => trf(
            "Cancelled: kept {} narrated chapters ({}); run it again to continue",
            &[&n, &t],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_tts::synth::Pcm;
    use bookx_tts::TtsPrefs;

    struct Tone;
    impl Synthesizer for Tone {
        fn id(&self) -> String {
            "t".into()
        }
        fn voices(&mut self) -> Result<Vec<bookx_tts::Voice>> {
            Ok(vec![])
        }
        fn synthesize(&mut self, req: &SynthRequest) -> Result<Pcm> {
            // length encodes the speed the adapter asked for
            Ok(Pcm {
                rate: 1000,
                samples: vec![0.5; (req.speed * 100.0) as usize],
            })
        }
    }

    fn adapter(speed: f64, pitch: f64) -> TtsSynth {
        TtsSynth {
            synth: Box::new(Tone),
            voice: String::new(),
            language: "en".into(),
            speed,
            pitch,
            cancel: Cancel::new(),
        }
    }

    #[test]
    fn the_adapter_passes_speed_and_converts_to_i16() {
        let a = adapter(1.5, 1.0).synth("x").unwrap();
        assert_eq!(a.rate, 1000);
        assert_eq!(a.samples.len(), 150);
        assert_eq!(a.samples[0], 16384);
        // speed and pitch are clamped to what engines accept
        assert_eq!(adapter(100.0, 1.0).synth("x").unwrap().samples.len(), 400);
        assert_eq!(adapter(0.01, 1.0).synth("x").unwrap().samples.len(), 25);
    }

    #[test]
    fn estimates_scale_with_the_text() {
        assert_eq!(estimated_seconds(0), 0);
        assert_eq!(estimated_seconds(1400), 100);
        assert_eq!(estimated_bytes(1400), 4_800_000);
        assert!(duration_text(30).contains('1'));
        assert!(duration_text(7500).contains("2 h"));
        let t = confirm_text(12, 500_000);
        assert!(t.contains("12 chapters"));
        assert!(t.contains("cancel"));
    }

    #[test]
    fn summaries() {
        assert_eq!(summary(0, false, 0), "Nothing was narrated");
        assert!(summary(2, false, 90_000).contains("Narrated 2 chapters"));
        assert!(summary(1, true, 90_000).contains("Cancelled"));
    }

    /// Opt-in: `BOOKX_TTS_LIVE=1 BOOKX_TTS_DIR=~/.cache/bookx-tts cargo test --release -p bookx
    /// narration::tests::live -- --nocapture` (needs the Kokoro pack and `espeak-ng`, see
    /// `crates/bookx-tts/tests/live.rs`). Narrates a short chapter with the real engine, checks the
    /// clips against the audio and prints the real-time factor.
    #[test]
    fn live_a_short_chapter_with_kokoro() {
        use bookx_editing::media_overlay_narrate::{narrate_book, Options};
        let Some(dir) = std::env::var("BOOKX_TTS_LIVE")
            .ok()
            .and_then(|_| std::env::var("BOOKX_TTS_DIR").ok())
        else {
            return;
        };
        if std::env::var("BOOKX_TTS_MOCK").is_ok() {
            return;
        }
        let store = bookx_tts::models::Store::new(dir);
        let prefs = TtsPrefs::default();
        let Ok((synth, id)) = bookx_tts::factory::open_synthesizer(&prefs, &store, "en") else {
            println!("no usable voice in BOOKX_TTS_DIR: skipped");
            return;
        };
        let mut ts = TtsSynth {
            synth,
            voice: prefs.voice_for(&id, "en"),
            language: "en".into(),
            speed: 1.0,
            pitch: 1.0,
            cancel: Cancel::new(),
        };
        let md = bookx_core::opf::Metadata::default();
        let ch = bookx_core::convert::Chapter {
            title: "One".into(),
            body: "<p>It was a bright cold day in April. The clocks were striking thirteen. Winston Smith slipped quickly through the glass doors of Victory Mansions.</p>\n".into(),
        };
        let mut book = bookx_core::book::OpenBook::from_archive(bookx_core::convert::create_epub(
            &md,
            &[ch],
            None,
        ))
        .unwrap();
        let t = std::time::Instant::now();
        let r = narrate_book(
            &mut book,
            &mut ts,
            &Options::default(),
            &|| false,
            &mut |_| {},
        )
        .unwrap();
        let took = t.elapsed().as_secs_f64();
        let audio = r.duration_ms as f64 / 1000.0;
        println!(
            "engine {id}: {} chapter, {audio:.1} s of audio in {took:.1} s (real-time factor {:.2})",
            r.narrated,
            took / audio
        );
        assert_eq!(r.narrated, 1);
        assert!(audio > 5.0 && audio < 40.0, "{audio}");
        let bad: Vec<_> = bookx_editing::media_overlay_check::check_overlays(&book.archive)
            .into_iter()
            .filter(|m| m.id != "MED-011")
            .collect();
        assert!(bad.is_empty(), "{bad:?}");
    }
}
