//! Playing the narration of an EPUB 3 media overlay in the reader (CE-422). The read-aloud player
//! asks its engine to speak one sentence after the other; [`OverlaySynth`] answers with the clip of
//! the book that says exactly that sentence, so the player card, the highlight, previous/next
//! sentence, speed and the sleep timer work as for synthesized speech, and seeking by sentence is
//! the player's own. No GTK in here.
//!
//! Only 16-bit PCM WAV audio is decoded (what Add Narration writes): there is no MP3/AAC/Opus
//! decoder in Bookx. Overlays with other audio are not used; the normal voice reads the book then.
//! The book is untrusted: SMIL parsing is bounded (`bookx_editing::media_overlay`), references
//! stay inside the book, WAV headers are bounds-checked and clips are cut from the file, never
//! larger than [`MAX_CLIP_MS`].

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use bookx_core::book::OpenBook;
use bookx_core::util::{parent_dir, resolve_href};
use bookx_editing::media_overlay::{fragment_texts, parse_smil};
use bookx_tts::Voice;
use bookx_tts::synth::{Pcm, SynthRequest, Synthesizer};

/// Longest clip played (ms); a longer one in a hostile book is cut.
pub const MAX_CLIP_MS: u64 = 10 * 60 * 1000;
/// Most clips kept for one book.
pub const MAX_CLIPS: usize = 500_000;
/// Audio that is not copied into memory beyond this many bytes (all files together).
pub const MAX_AUDIO_BYTES: usize = 1_500_000_000;

/// One sentence of the narration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clip {
    /// Text of the sentence, spaces collapsed.
    pub text: String,
    /// Index into [`Overlay::files`].
    pub file: usize,
    pub begin_ms: u64,
    pub end_ms: u64,
}

/// The narration of a book: its clips in reading order and the audio files.
#[derive(Debug, Default)]
pub struct Overlay {
    pub clips: Vec<Clip>,
    pub files: Vec<Arc<Vec<u8>>>,
}

fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Does the book carry a media overlay at all (cheap check on the package)?
pub fn has_overlay(book: &OpenBook) -> bool {
    bookx_editing::media_overlay_check::has_overlays(&book.archive)
}

impl Overlay {
    /// Reads the overlay of `book`. `None` when there is none, nothing in it is usable, or the
    /// audio is not WAV.
    pub fn load(book: &OpenBook) -> Option<Overlay> {
        let opf_path = &book.package.opf_path;
        let base = parent_dir(opf_path).to_string();
        let text = book.archive.get_string(opf_path)?;
        let doc = bookx_core::util::parse_xml(&text).ok()?;
        let mut smil_of: HashMap<String, String> = HashMap::new();
        let mut path_of: HashMap<String, String> = HashMap::new();
        for n in doc
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "item")
        {
            let id = n.attribute("id").unwrap_or("").to_string();
            path_of.insert(
                id.clone(),
                resolve_href(&base, n.attribute("href").unwrap_or("")).0,
            );
            if let Some(m) = n.attribute("media-overlay") {
                smil_of.insert(id, m.to_string());
            }
        }
        if smil_of.is_empty() {
            return None;
        }
        let mut ov = Overlay::default();
        let mut file_index: HashMap<String, usize> = HashMap::new();
        let mut total_audio = 0usize;
        for it in &book.package.spine {
            let Some(smil_id) = smil_of.get(&it.idref) else {
                continue;
            };
            let (Some(smil_path), Some(chapter)) = (path_of.get(smil_id), path_of.get(&it.idref))
            else {
                continue;
            };
            let Some(xml) = book.archive.get_string(smil_path) else {
                continue;
            };
            let Ok(read) = parse_smil(smil_path, &xml) else {
                continue;
            };
            let ids: HashSet<&str> = read
                .pars
                .iter()
                .filter(|p| p.text_path == *chapter)
                .filter_map(|p| p.text_id.as_deref())
                .collect();
            let texts = book
                .archive
                .get_string(chapter)
                .map(|x| fragment_texts(&x, &ids))
                .unwrap_or_default();
            for p in read.pars {
                if ov.clips.len() >= MAX_CLIPS {
                    break;
                }
                if p.text_path != *chapter {
                    continue;
                }
                let Some(t) = p.text_id.as_ref().and_then(|i| texts.get(i)) else {
                    continue;
                };
                let t = normalize(t);
                if t.is_empty() {
                    continue;
                }
                let file = match file_index.get(&p.audio_path) {
                    Some(i) => *i,
                    None => {
                        let bytes = book.archive.get(&p.audio_path)?;
                        wav_info(bytes)?;
                        total_audio = total_audio.saturating_add(bytes.len());
                        if total_audio > MAX_AUDIO_BYTES {
                            return None;
                        }
                        ov.files.push(Arc::new(bytes.to_vec()));
                        file_index.insert(p.audio_path.clone(), ov.files.len() - 1);
                        ov.files.len() - 1
                    }
                };
                ov.clips.push(Clip {
                    text: t,
                    file,
                    begin_ms: p.begin_ms,
                    end_ms: p.end_ms.min(p.begin_ms + MAX_CLIP_MS),
                });
            }
        }
        (!ov.clips.is_empty()).then_some(ov)
    }
}

/// Where the samples of a 16-bit PCM WAV file are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavInfo {
    pub rate: u32,
    pub channels: u16,
    pub data_start: usize,
    pub data_len: usize,
}

/// Reads the header of a 16-bit PCM WAV file; `None` for anything else or a damaged header.
pub fn wav_info(b: &[u8]) -> Option<WavInfo> {
    if b.len() < 44 || &b[..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return None;
    }
    let mut pos = 12usize;
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    while pos.checked_add(8)? <= b.len() {
        let id = &b[pos..pos + 4];
        let size = u32::from_le_bytes(b[pos + 4..pos + 8].try_into().ok()?) as usize;
        let body = pos + 8;
        if id == b"fmt " {
            if size < 16 || body.checked_add(16)? > b.len() {
                return None;
            }
            fmt = Some((
                u16::from_le_bytes([b[body], b[body + 1]]),
                u16::from_le_bytes([b[body + 2], b[body + 3]]),
                u32::from_le_bytes(b[body + 4..body + 8].try_into().ok()?),
                u16::from_le_bytes([b[body + 14], b[body + 15]]),
            ));
        } else if id == b"data" {
            let (format, channels, rate, bits) = fmt?;
            if format != 1
                || bits != 16
                || !(1..=2).contains(&channels)
                || !(4000..=192_000).contains(&rate)
            {
                return None;
            }
            let avail = b.len() - body;
            let len = if size == 0 || size == 0xffff_ffff || size > avail {
                avail
            } else {
                size
            };
            return Some(WavInfo {
                rate,
                channels,
                data_start: body,
                data_len: len,
            });
        }
        pos = body.checked_add(size)?.checked_add(size & 1)?;
    }
    None
}

/// The samples of `[begin_ms, end_ms)` of a WAV file as mono floats.
pub fn clip_pcm(file: &[u8], begin_ms: u64, end_ms: u64) -> Result<Pcm> {
    let info =
        wav_info(file).ok_or_else(|| anyhow!("the narration audio is not a 16-bit WAV file"))?;
    let frame = 2 * info.channels as usize;
    let frames = info.data_len / frame;
    let a = ((begin_ms * info.rate as u64 / 1000) as usize).min(frames);
    let z = ((end_ms * info.rate as u64 / 1000) as usize).clamp(a, frames);
    let data = &file[info.data_start..info.data_start + info.data_len];
    let samples = (a..z)
        .map(|i| {
            let at = i * frame;
            let l = i16::from_le_bytes([data[at], data[at + 1]]) as f32;
            let v = if info.channels == 2 {
                (l + i16::from_le_bytes([data[at + 2], data[at + 3]]) as f32) / 2.0
            } else {
                l
            };
            v / 32768.0
        })
        .collect();
    Ok(Pcm {
        rate: info.rate,
        samples,
    })
}

/// A "synthesizer" that answers with the clips of the book.
pub struct OverlaySynth {
    overlay: Arc<Overlay>,
    /// The clip after the one played last.
    cursor: usize,
    /// The clip played last and what of its text has been asked for since.
    last_clip: Option<usize>,
}

impl OverlaySynth {
    pub fn new(overlay: Arc<Overlay>) -> OverlaySynth {
        OverlaySynth {
            overlay,
            cursor: 0,
            last_clip: None,
        }
    }

    /// The clips `[a, b)` whose texts, joined by spaces, are exactly `want`; searches forward from
    /// the cursor first (reading in order), then from the start (the reader jumped back).
    fn locate(&self, want: &str) -> Option<(usize, usize)> {
        let n = self.overlay.clips.len();
        let order = (self.cursor..n).chain(0..self.cursor.min(n));
        for i in order {
            let mut acc = String::new();
            for (j, clip) in self
                .overlay
                .clips
                .iter()
                .enumerate()
                .take(n.min(i + 40))
                .skip(i)
            {
                if !acc.is_empty() {
                    acc.push(' ');
                }
                acc.push_str(&clip.text);
                if acc == want {
                    return Some((i, j + 1));
                }
                if !want.starts_with(&acc) {
                    break;
                }
            }
        }
        None
    }

    /// A clip whose text contains `want` (the reader splits finer than the narration did).
    fn locate_inside(&self, want: &str) -> Option<usize> {
        let n = self.overlay.clips.len();
        (self.cursor.saturating_sub(1)..n)
            .chain(0..self.cursor.saturating_sub(1).min(n))
            .find(|&i| self.overlay.clips[i].text.contains(want))
    }
}

impl Synthesizer for OverlaySynth {
    fn id(&self) -> String {
        "narration".into()
    }

    fn voices(&mut self) -> Result<Vec<Voice>> {
        Ok(Vec::new())
    }

    fn synthesize(&mut self, req: &SynthRequest) -> Result<Pcm> {
        let want = normalize(req.text);
        if want.is_empty() {
            return Ok(Pcm {
                rate: 8000,
                samples: Vec::new(),
            });
        }
        let ov = self.overlay.clone();
        if let Some((a, b)) = self.locate(&want) {
            let mut out = Pcm::default();
            for c in &ov.clips[a..b] {
                let p = clip_pcm(&ov.files[c.file], c.begin_ms, c.end_ms)?;
                if out.rate == 0 {
                    out.rate = p.rate;
                }
                if p.rate != out.rate {
                    bail!("the narration changes its sample rate");
                }
                out.samples.extend(p.samples);
            }
            self.cursor = b;
            self.last_clip = Some(b - 1);
            return Ok(out);
        }
        if let Some(i) = self.locate_inside(&want) {
            if self.last_clip == Some(i) {
                // the rest of a clip that was played whole for an earlier part of it
                return Ok(Pcm {
                    rate: 8000,
                    samples: Vec::new(),
                });
            }
            let c = &ov.clips[i];
            self.cursor = i + 1;
            self.last_clip = Some(i);
            return clip_pcm(&ov.files[c.file], c.begin_ms, c.end_ms);
        }
        bail!("this sentence is not in the narration of the book")
    }
}

/// A speaker for tests and the UI scenarios: "plays" for as long as the audio lasts, in steps of
/// 10 ms, without a sound. `speedup` makes it faster than real time.
pub struct SilentSink {
    r#gen: Arc<std::sync::atomic::AtomicU64>,
    speedup: u32,
}

impl SilentSink {
    pub fn new(speedup: u32) -> SilentSink {
        SilentSink {
            r#gen: Arc::default(),
            speedup: speedup.max(1),
        }
    }
}

impl bookx_tts::sink::AudioSink for SilentSink {
    fn write(&mut self, rate: u32, samples: &[i16]) -> Result<()> {
        use std::sync::atomic::Ordering::SeqCst;
        let g = self.r#gen.load(SeqCst);
        let total = std::time::Duration::from_micros(
            samples.len() as u64 * 1_000_000 / rate.max(1) as u64 / self.speedup as u64,
        );
        let t0 = std::time::Instant::now();
        while t0.elapsed() < total && self.r#gen.load(SeqCst) == g {
            std::thread::sleep(std::time::Duration::from_millis(10).min(total));
        }
        Ok(())
    }

    fn flush(&mut self) {
        self.r#gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// The engine that plays the book's narration, or `None` when the book has none (or not in WAV).
/// `$BOOKX_TTS_MOCK` (any number) makes it silent (but as slow as real speech), for the UI tests.
pub fn open_engine(book: &OpenBook) -> Option<bookx_tts::factory::Opened> {
    if !has_overlay(book) {
        return None;
    }
    let overlay = Arc::new(Overlay::load(book)?);
    let sink: Box<dyn bookx_tts::sink::AudioSink> = if std::env::var("BOOKX_TTS_MOCK").is_ok() {
        Box::new(SilentSink::new(1))
    } else {
        Box::new(bookx_tts::sink::CommandSink::new())
    };
    let engine = bookx_tts::pipeline::PipelineEngine::new(
        "Narration",
        Box::new(OverlaySynth::new(overlay)),
        sink,
        None,
        3,
    );
    Some(bookx_tts::factory::Opened {
        engine: Box::new(engine),
        choice: None,
        notice: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_editing::media_overlay_narrate::{
        Audio, Options, SentenceSynth, narrate_book, remove_narration,
    };
    use bookx_tts::synth::Cancel;

    /// 10 ms of audio per character at 8 kHz, a distinct level per call so clips can be told apart.
    struct Fake(i16);
    impl SentenceSynth for Fake {
        fn synth(&mut self, text: &str) -> Result<Audio> {
            self.0 += 100;
            Ok(Audio {
                rate: 8000,
                samples: vec![self.0; text.chars().count() * 80],
            })
        }
    }

    fn narrated(bodies: &[&str]) -> OpenBook {
        let md = bookx_core::opf::Metadata {
            title: "T".into(),
            ..Default::default()
        };
        let chapters: Vec<bookx_core::convert::Chapter> = bodies
            .iter()
            .enumerate()
            .map(|(i, b)| bookx_core::convert::Chapter {
                title: format!("Ch {i}"),
                body: format!("{b}\n"),
            })
            .collect();
        let mut book =
            OpenBook::from_archive(bookx_core::convert::create_epub(&md, &chapters, None)).unwrap();
        narrate_book(
            &mut book,
            &mut Fake(0),
            &Options { gap_ms: 0 },
            &|| false,
            &mut |_| {},
        )
        .unwrap();
        book
    }

    fn ask(s: &mut OverlaySynth, text: &str) -> Result<Pcm> {
        let c = Cancel::new();
        s.synthesize(&SynthRequest {
            text,
            voice: "",
            language: "",
            speed: 1.0,
            cancel: &c,
        })
    }

    #[test]
    fn a_narrated_book_plays_its_own_sentences() {
        let book = narrated(&["<p>Hello there. How are you?</p>", "<p>Second chapter.</p>"]);
        assert!(has_overlay(&book));
        let ov = Arc::new(Overlay::load(&book).unwrap());
        let texts: Vec<&str> = ov.clips.iter().map(|c| c.text.as_str()).collect();
        assert!(
            texts.contains(&"Hello there.") && texts.contains(&"Second chapter."),
            "{texts:?}"
        );
        let mut s = OverlaySynth::new(ov.clone());
        let a = ask(&mut s, "Hello there.").unwrap();
        assert_eq!(a.rate, 8000);
        assert_eq!(a.samples.len(), 12 * 80);
        // each sentence has its own level: the clip boundaries are exact
        let level = |p: &Pcm| (p.samples[0] * 32768.0).round() as i32;
        let b = ask(&mut s, "How are you?").unwrap();
        assert_eq!(b.samples.len(), 12 * 80);
        assert_ne!(level(&a), level(&b));
        let c = ask(&mut s, "Second   chapter.").unwrap();
        assert_eq!(c.samples.len(), 15 * 80);
    }

    #[test]
    fn seeking_to_a_sentence_and_back_works() {
        let book = narrated(&["<p>One one. Two two. Three three.</p>"]);
        let mut s = OverlaySynth::new(Arc::new(Overlay::load(&book).unwrap()));
        let level = |p: &Pcm| (p.samples[0] * 32768.0).round() as i32;
        let third = ask(&mut s, "Three three.").unwrap();
        let first = ask(&mut s, "One one.").unwrap();
        let second = ask(&mut s, "Two two.").unwrap();
        assert!(level(&first) < level(&second) && level(&second) < level(&third));
    }

    #[test]
    fn sentences_the_reader_merges_or_splits_are_still_played() {
        let book = narrated(&["<p>Aa bb. Cc dd. Ee ff.</p>"]);
        let ov = Arc::new(Overlay::load(&book).unwrap());
        // the reader reads two narrated sentences as one
        let mut s = OverlaySynth::new(ov.clone());
        let p = ask(&mut s, "Aa bb. Cc dd.").unwrap();
        assert_eq!(p.samples.len(), (6 + 6) * 80);
        // or splits one finer: the whole clip once, the rest silent
        let mut s = OverlaySynth::new(ov);
        assert_eq!(ask(&mut s, "Aa").unwrap().samples.len(), 6 * 80);
        assert!(ask(&mut s, "bb.").unwrap().samples.is_empty());
        assert_eq!(ask(&mut s, "Cc dd.").unwrap().samples.len(), 6 * 80);
    }

    #[test]
    fn text_that_is_not_in_the_narration_is_an_error() {
        let book = narrated(&["<p>Only this.</p>"]);
        let mut s = OverlaySynth::new(Arc::new(Overlay::load(&book).unwrap()));
        assert!(ask(&mut s, "Something else entirely.").is_err());
        assert!(ask(&mut s, "   ").unwrap().samples.is_empty());
    }

    #[test]
    fn books_without_usable_narration_give_none() {
        let mut book = narrated(&["<p>Some text.</p>"]);
        let wav = book
            .archive
            .names()
            .iter()
            .find(|n| n.ends_with(".wav"))
            .cloned()
            .unwrap();
        // not WAV any more
        book.archive.set(
            &wav,
            b"ID3 not a wav file at all, mp3 maybe....................".to_vec(),
        );
        assert!(Overlay::load(&book).is_none());
        // gone
        book.archive.remove(&wav);
        assert!(Overlay::load(&book).is_none());
        // removed narration: nothing
        let mut book = narrated(&["<p>Some text.</p>"]);
        remove_narration(&mut book).unwrap();
        assert!(!has_overlay(&book));
        assert!(Overlay::load(&book).is_none());
        assert!(open_engine(&book).is_none());
    }

    #[test]
    fn a_hostile_smil_is_survived() {
        let mut book = narrated(&["<p>Some text.</p>"]);
        let smil = book
            .archive
            .names()
            .iter()
            .find(|n| n.ends_with(".smil"))
            .cloned()
            .unwrap();
        book.archive.set_string(&smil, "<smil><par><text src=\"../../etc/passwd\"/><audio src=\"/etc/passwd\" clipBegin=\"0s\" clipEnd=\"1s\"/></par>");
        assert!(Overlay::load(&book).is_none());
        let t = bookx_editing::media_overlay::build_smil(&bookx_editing::media_overlay::SmilDoc {
            text_href: "../text".into(),
            audio_href: "http://x/y.wav".into(),
            pars: vec![],
        });
        book.archive.set_string(&smil, &t);
        assert!(Overlay::load(&book).is_none());
    }

    #[test]
    fn wav_headers_are_checked() {
        let wav = bookx_editing::media_overlay_narrate::wav_bytes(8000, &[1, 2, 3, 4]);
        let i = wav_info(&wav).unwrap();
        assert_eq!(
            (i.rate, i.channels, i.data_start, i.data_len),
            (8000, 1, 44, 8)
        );
        for bad in [&wav[..43], &b"RIFF\0\0\0\0WAVE"[..], &[0u8; 100][..]] {
            assert!(wav_info(bad).is_none());
        }
        // 8-bit, float and a zero rate
        for (at, v) in [(34usize, 8u8), (20, 3), (24, 0)] {
            let mut w = wav.clone();
            w[at] = v;
            if at == 24 {
                w[25] = 0;
                w[26] = 0;
                w[27] = 0;
            }
            assert!(wav_info(&w).is_none(), "{at}");
        }
        // a data size that lies is clamped to the file
        let mut w = wav.clone();
        w[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(wav_info(&w).unwrap().data_len, 8);
        // clips are cut at the data and out-of-range times are clamped
        let p = clip_pcm(&wav, 0, 10_000).unwrap();
        assert_eq!(p.samples.len(), 4);
        assert!(clip_pcm(&wav, 5_000, 1).unwrap().samples.is_empty());
        assert!(clip_pcm(b"junk", 0, 1).is_err());
    }

    #[test]
    fn stereo_is_mixed_down() {
        let mut w =
            bookx_editing::media_overlay_narrate::wav_bytes(8000, &[1000, 3000, 2000, 2000]);
        w[22] = 2; // two channels
        let p = clip_pcm(&w, 0, 1000).unwrap();
        assert_eq!(p.samples.len(), 2);
        assert!((p.samples[0] * 32768.0 - 2000.0).abs() < 1.0);
    }

    #[test]
    fn the_silent_sink_takes_the_time_the_audio_lasts_and_can_be_cut_short() {
        use bookx_tts::sink::AudioSink;
        let mut s = SilentSink::new(1);
        let t0 = std::time::Instant::now();
        s.write(1000, &[0i16; 100]).unwrap();
        assert!(t0.elapsed() >= std::time::Duration::from_millis(90));
        let mut s = SilentSink::new(10);
        let t0 = std::time::Instant::now();
        s.write(1000, &[0i16; 1000]).unwrap();
        assert!(t0.elapsed() < std::time::Duration::from_millis(400));
    }
}
