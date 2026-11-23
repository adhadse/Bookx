//! Narration of a book (CE-422): per chapter, the sentences are synthesized one by one, written as one
//! audio file, the chapter's sentences get ids, a SMIL document ties both together and the package
//! is updated. The book is complete after **every** chapter, so a cancelled or failed run keeps what
//! is done and a second run continues with the chapters that have no overlay yet.
//!
//! The speech engine comes in through [`SentenceSynth`] (the application implements it with
//! `bookx-tts`), so this module is GTK-free, engine-free and fully testable with a fake.
//!
//! Audio is written as 16-bit mono WAV (`audio/wav`). No pure-Rust encoder for Opus, MP3 or AAC is
//! available in the workspace, and WAV is not a core media type of EPUB 3 (see rule `MED-011`):
//! files are large (about 48 KB per second at 24 kHz) and a few reading systems will not play them.

use std::collections::HashSet;

use anyhow::{Context, Result, anyhow, bail};
use bookx_core::book::OpenBook;
use bookx_core::util::{join_path, parent_dir, relative_href, resolve_href};

use crate::media_overlay::{
    Par, SmilDoc, build_smil, parse_smil, unwrap_sentences, wrap_sentences,
};
use crate::media_overlay_opf::{
    ChapterOverlay, SMIL_TYPE, add_chapter_overlay, remove_items, remove_overlays,
    set_total_duration,
};

/// 16-bit mono audio of one sentence.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Audio {
    pub rate: u32,
    pub samples: Vec<i16>,
}

/// Turns one sentence into audio. May block for a long time.
pub trait SentenceSynth {
    fn synth(&mut self, text: &str) -> Result<Audio>;
}

/// Returned (as the error) when the caller's cancel check said stop.
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

/// Longest audio file of one chapter (bytes); a chapter that would exceed it fails.
pub const MAX_CHAPTER_AUDIO_BYTES: usize = 1 << 30;
/// Silence after a sentence whose synthesis came back empty (ms), so every clip has a length.
pub const MIN_CLIP_MS: u64 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Silence added after every sentence (ms).
    pub gap_ms: u32,
}

impl Default for Options {
    fn default() -> Self {
        Options { gap_ms: 250 }
    }
}

/// 16-bit mono PCM WAV file.
pub fn wav_bytes(rate: u32, samples: &[i16]) -> Vec<u8> {
    let data = samples.len() * 2;
    let mut w = Vec::with_capacity(44 + data);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&((36 + data) as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data as u32).to_le_bytes());
    for s in samples {
        w.extend_from_slice(&s.to_le_bytes());
    }
    w
}

/// Linear resampling (speech at 16..48 kHz, used only when an engine changes rate mid-chapter).
fn resample(s: &[i16], from: u32, to: u32) -> Vec<i16> {
    if from == to || s.is_empty() || from == 0 || to == 0 {
        return s.to_vec();
    }
    let n = (s.len() as u64 * to as u64 / from as u64) as usize;
    (0..n)
        .map(|i| {
            let pos = i as f64 * from as f64 / to as f64;
            let a = pos.floor() as usize;
            let t = pos - a as f64;
            let x = s[a.min(s.len() - 1)] as f64;
            let y = s[(a + 1).min(s.len() - 1)] as f64;
            (x + (y - x) * t).round() as i16
        })
        .collect()
}

/// A chapter, narrated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterNarration {
    pub xhtml: String,
    pub smil: String,
    pub wav: Vec<u8>,
    pub pars: Vec<Par>,
    pub duration_ms: u64,
}

/// Narrates one chapter. `text_path`, `smil_path` and `audio_path` are container paths; the SMIL's
/// references are computed from them. `progress` gets `(done, total)` sentences; `cancel` is asked
/// before every sentence. `Ok(None)`: the chapter has nothing to read.
#[allow(clippy::too_many_arguments)]
pub fn narrate_chapter(
    xhtml: &str,
    text_path: &str,
    smil_path: &str,
    audio_path: &str,
    synth: &mut dyn SentenceSynth,
    gap_ms: u32,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Option<ChapterNarration>> {
    let wrapped = wrap_sentences(xhtml)?;
    if wrapped.sentences.is_empty() {
        return Ok(None);
    }
    let total = wrapped.sentences.len();
    let mut rate = 0u32;
    let mut samples: Vec<i16> = Vec::new();
    let mut pars = Vec::with_capacity(total);
    for (i, s) in wrapped.sentences.iter().enumerate() {
        if cancel() {
            return Err(Cancelled.into());
        }
        let mut a = synth
            .synth(&s.text)
            .with_context(|| format!("synthesizing sentence {} of {total}", i + 1))?;
        if a.rate == 0 {
            bail!("the speech engine returned audio without a sample rate");
        }
        if rate == 0 {
            rate = a.rate;
        } else if a.rate != rate {
            a.samples = resample(&a.samples, a.rate, rate);
        }
        let begin_ms = samples.len() as u64 * 1000 / rate as u64;
        samples.extend_from_slice(&a.samples);
        let gap = (rate as u64 * gap_ms as u64 / 1000) as usize;
        samples.resize(samples.len() + gap, 0);
        let mut end_ms = samples.len() as u64 * 1000 / rate as u64;
        if end_ms <= begin_ms {
            let extra = (rate as u64 * MIN_CLIP_MS / 1000) as usize;
            samples.resize(samples.len() + extra, 0);
            end_ms = samples.len() as u64 * 1000 / rate as u64;
        }
        if samples.len() * 2 > MAX_CHAPTER_AUDIO_BYTES {
            bail!("the audio of this chapter would be larger than 1 GiB");
        }
        pars.push(Par {
            text_id: s.id.clone(),
            begin_ms,
            end_ms,
        });
        progress(i + 1, total);
    }
    // the last clip ends exactly where the file ends
    let duration_ms = samples.len() as u64 * 1000 / rate as u64;
    if let Some(last) = pars.last_mut() {
        last.end_ms = duration_ms.max(last.begin_ms + 1);
    }
    let smil = build_smil(&SmilDoc {
        text_href: relative_href(smil_path, text_path),
        audio_href: relative_href(smil_path, audio_path),
        pars: pars.clone(),
    });
    Ok(Some(ChapterNarration {
        xhtml: wrapped.xhtml,
        smil,
        wav: wav_bytes(rate, &samples),
        pars,
        duration_ms,
    }))
}

struct ChapterItem {
    id: String,
    path: String,
    overlay: bool,
}

/// The spine documents that can be narrated, in reading order.
fn chapters(book: &OpenBook) -> Result<Vec<ChapterItem>> {
    let opf_path = book.package.opf_path.clone();
    let base = parent_dir(&opf_path).to_string();
    let text = book
        .archive
        .get_string(&opf_path)
        .context("the package document is missing")?;
    let doc = bookx_core::util::parse_xml(&text)?;
    let mut attrs = std::collections::HashMap::new();
    for n in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "item")
    {
        attrs.insert(
            n.attribute("id").unwrap_or("").to_string(),
            n.attribute("media-overlay").is_some(),
        );
    }
    let mut out = Vec::new();
    for it in &book.package.spine {
        let Some(m) = book.package.manifest.iter().find(|m| m.id == it.idref) else {
            continue;
        };
        if m.media_type != "application/xhtml+xml"
            || m.properties.split_whitespace().any(|p| p == "nav")
        {
            continue;
        }
        out.push(ChapterItem {
            id: m.id.clone(),
            path: resolve_href(&base, &m.href).0,
            overlay: attrs.get(&m.id).copied().unwrap_or(false),
        });
    }
    Ok(out)
}

/// How many chapters there are, how many are narrated already.
pub fn narration_status(book: &OpenBook) -> (usize, usize) {
    match chapters(book) {
        Ok(c) => (c.len(), c.iter().filter(|c| c.overlay).count()),
        Err(_) => (0, 0),
    }
}

/// `(chapters with text that have no overlay yet, characters of text in them)`: what a run would do.
pub fn pending(book: &OpenBook) -> (usize, usize) {
    let Ok(all) = chapters(book) else {
        return (0, 0);
    };
    let mut n = 0;
    let mut chars = 0;
    for c in all.iter().filter(|c| !c.overlay) {
        let Some(x) = book.archive.get_string(&c.path) else {
            continue;
        };
        if let Ok(w) = wrap_sentences(&x) {
            if !w.sentences.is_empty() {
                n += 1;
                chars += w
                    .sentences
                    .iter()
                    .map(|s| s.text.chars().count())
                    .sum::<usize>();
            }
        }
    }
    (n, chars)
}

/// Progress of [`narrate_book`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    /// 1-based position among the chapters that still have to be narrated.
    pub chapter: usize,
    pub chapters: usize,
    pub sentence: usize,
    pub sentences: usize,
    pub path: String,
}

/// What [`narrate_book`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub narrated: usize,
    /// Chapters that already had an overlay.
    pub skipped: usize,
    /// Chapters without text.
    pub empty: usize,
    pub duration_ms: u64,
    pub cancelled: bool,
}

fn unique_path(archive: &bookx_core::archive::Archive, dir: &str, stem: &str, ext: &str) -> String {
    let mut n = 0;
    loop {
        let name = if n == 0 {
            format!("{stem}.{ext}")
        } else {
            format!("{stem}-{n}.{ext}")
        };
        let p = join_path(dir, &name);
        if !archive.contains(&p) {
            return p;
        }
        n += 1;
    }
}

fn unique_id(taken: &mut HashSet<String>, want: &str) -> String {
    let mut id = want.to_string();
    let mut n = 1;
    while !taken.insert(id.clone()) {
        n += 1;
        id = format!("{want}-{n}");
    }
    id
}

fn sanitize(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if t.is_empty() { "chapter".into() } else { t }
}

/// Sum of the per-chapter durations declared in the package.
fn declared_total(opf: &str) -> u64 {
    let Ok(doc) = bookx_core::util::parse_xml(opf) else {
        return 0;
    };
    doc.descendants()
        .filter(|n| {
            n.is_element()
                && n.tag_name().name() == "meta"
                && n.attribute("property") == Some("media:duration")
                && n.attribute("refines").is_some()
        })
        .filter_map(|n| crate::media_overlay::parse_clock(n.text().unwrap_or("")))
        .sum()
}

/// Narrates every chapter that has no overlay yet. The package must be EPUB 3. After each chapter
/// the book is consistent. A cancel check that returns `true` stops after the sentence in hand: the
/// chapter in progress is dropped, earlier chapters stay (the report says `cancelled`). Engine
/// errors are returned; chapters finished before it stay as well.
pub fn narrate_book(
    book: &mut OpenBook,
    synth: &mut dyn SentenceSynth,
    opts: &Options,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(&Progress),
) -> Result<Report> {
    if !book.package.is_epub3() {
        bail!(
            "narration needs an EPUB 3 package (this one is version {}); upgrade the book first",
            book.package.version
        );
    }
    let all = chapters(book)?;
    let todo: Vec<&ChapterItem> = all.iter().filter(|c| !c.overlay).collect();
    let mut report = Report {
        skipped: all.len() - todo.len(),
        ..Default::default()
    };
    let opf_path = book.package.opf_path.clone();
    let base = parent_dir(&opf_path).to_string();
    let mut taken: HashSet<String> = book.package.manifest.iter().map(|m| m.id.clone()).collect();
    for (n, ch) in todo.iter().enumerate() {
        if cancel() {
            report.cancelled = true;
            break;
        }
        let Some(xhtml) = book.archive.get_string(&ch.path) else {
            report.empty += 1;
            continue;
        };
        let stem = sanitize(&ch.id);
        let smil_path = unique_path(&book.archive, &join_path(&base, "smil"), &stem, "smil");
        let audio_path = unique_path(&book.archive, &join_path(&base, "audio"), &stem, "wav");
        let mut cb = |done: usize, total: usize| {
            progress(&Progress {
                chapter: n + 1,
                chapters: todo.len(),
                sentence: done,
                sentences: total,
                path: ch.path.clone(),
            })
        };
        let r = narrate_chapter(
            &xhtml,
            &ch.path,
            &smil_path,
            &audio_path,
            synth,
            opts.gap_ms,
            cancel,
            &mut cb,
        );
        let done = match r {
            Ok(Some(d)) => d,
            Ok(None) => {
                report.empty += 1;
                continue;
            }
            Err(e) if e.downcast_ref::<Cancelled>().is_some() => {
                report.cancelled = true;
                break;
            }
            Err(e) => return Err(e),
        };
        let opf = book
            .archive
            .get_string(&opf_path)
            .context("the package document is missing")?;
        let smil_id = unique_id(&mut taken, &format!("mo-{stem}"));
        let audio_id = unique_id(&mut taken, &format!("au-{stem}"));
        let mut new_opf = add_chapter_overlay(
            &opf,
            &ChapterOverlay {
                chapter_id: ch.id.clone(),
                smil_id,
                smil_href: relative_href(&opf_path, &smil_path),
                audio_id,
                audio_href: relative_href(&opf_path, &audio_path),
                audio_type: "audio/wav".into(),
                duration_ms: done.duration_ms,
            },
        )?;
        let total = declared_total(&new_opf);
        new_opf = set_total_duration(&new_opf, total)?;
        bookx_core::util::parse_xml(&new_opf)
            .map_err(|e| anyhow!("the edited package is not well-formed: {e}"))?;
        book.archive.set_string(&ch.path, &done.xhtml);
        book.archive.set_string(&smil_path, &done.smil);
        book.archive.set(&audio_path, done.wav);
        book.archive.set_string(&opf_path, &new_opf);
        book.reload_package()?;
        report.narrated += 1;
        report.duration_ms += done.duration_ms;
    }
    Ok(report)
}

/// What [`remove_narration`] removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemovedNarration {
    pub chapters: usize,
    pub files: usize,
}

/// Removes all narration: SMIL and audio files and their manifest items, `media-overlay`
/// attributes, durations, and the sentence spans of the chapters.
pub fn remove_narration(book: &mut OpenBook) -> Result<RemovedNarration> {
    let opf_path = book.package.opf_path.clone();
    let base = parent_dir(&opf_path).to_string();
    let opf = book
        .archive
        .get_string(&opf_path)
        .context("the package document is missing")?;
    let (new_opf, removed) = remove_overlays(&opf);
    // audio and chapter files the SMIL documents referred to
    let mut audio: Vec<String> = Vec::new();
    let mut chapter_files: Vec<String> = Vec::new();
    let mut smil_paths: Vec<String> = Vec::new();
    for href in &removed.files {
        let p = resolve_href(&base, href).0;
        if let Some(x) = book.archive.get_string(&p) {
            if let Ok(r) = parse_smil(&p, &x) {
                for par in r.pars {
                    if !audio.contains(&par.audio_path) {
                        audio.push(par.audio_path);
                    }
                    if !chapter_files.contains(&par.text_path) {
                        chapter_files.push(par.text_path);
                    }
                }
            }
        }
        smil_paths.push(p);
    }
    let still_used: HashSet<String> = book
        .package
        .manifest
        .iter()
        .filter(|m| m.media_type != SMIL_TYPE)
        .map(|m| resolve_href(&base, &m.href).0)
        .collect();
    // audio files that were only the narration's: those the SMIL documents named and that no
    // chapter references (chapters hold no audio element of their own in our output)
    let mut referenced_by_text: HashSet<String> = HashSet::new();
    for p in book.archive.names().to_vec() {
        if matches!(
            bookx_core::util::extension(&p).as_str(),
            "xhtml" | "html" | "htm"
        ) {
            if let Some(t) = book.archive.get_string(&p) {
                for a in &audio {
                    let file = bookx_core::util::file_name(a);
                    if t.contains(file) {
                        referenced_by_text.insert(a.clone());
                    }
                }
            }
        }
    }
    let audio_to_remove: Vec<String> = audio
        .into_iter()
        .filter(|a| !referenced_by_text.contains(a) && still_used.contains(a))
        .collect();
    let audio_hrefs: Vec<String> = audio_to_remove
        .iter()
        .map(|a| relative_href(&opf_path, a))
        .collect();
    let new_opf = remove_items(&new_opf, &audio_hrefs);
    bookx_core::util::parse_xml(&new_opf)
        .map_err(|e| anyhow!("the edited package is not well-formed: {e}"))?;
    let mut files = 0;
    for p in smil_paths.iter().chain(audio_to_remove.iter()) {
        if book.archive.remove(p) {
            files += 1;
        }
    }
    for p in chapter_files {
        if let Some(t) = book.archive.get_string(&p) {
            let back = unwrap_sentences(&t);
            if back != t {
                book.archive.set_string(&p, &back);
            }
        }
    }
    book.archive.set_string(&opf_path, &new_opf);
    book.reload_package()?;
    Ok(RemovedNarration {
        chapters: removed.chapters,
        files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_overlay_check::check_overlays;
    use crate::testutil::make_book;

    /// 10 ms of audio per character at 8 kHz, a constant tone.
    struct Fake {
        calls: Vec<String>,
        fail_on: Option<usize>,
    }

    impl Fake {
        fn new() -> Fake {
            Fake {
                calls: Vec::new(),
                fail_on: None,
            }
        }
    }

    impl SentenceSynth for Fake {
        fn synth(&mut self, text: &str) -> Result<Audio> {
            if self.fail_on == Some(self.calls.len()) {
                bail!("engine exploded");
            }
            self.calls.push(text.to_string());
            Ok(Audio {
                rate: 8000,
                samples: vec![1000; text.chars().count() * 80],
            })
        }
    }

    fn never() -> bool {
        false
    }

    const BODY1: &str = "<h1>One</h1><p>Hello there. How are you?</p>";
    const BODY2: &str = "<p>Second chapter. It has text &amp; more.</p>";

    #[test]
    fn a_chapter_gets_clips_that_add_up() {
        let xhtml = "<html><body><p>Hello. World!</p></body></html>";
        let mut f = Fake::new();
        let mut seen = Vec::new();
        let n = narrate_chapter(
            xhtml,
            "OEBPS/text/c.xhtml",
            "OEBPS/smil/c.smil",
            "OEBPS/audio/c.wav",
            &mut f,
            250,
            &never,
            &mut |d, t| seen.push((d, t)),
        )
        .unwrap()
        .unwrap();
        assert_eq!(f.calls, vec!["Hello.", "World!"]);
        assert_eq!(seen, vec![(1, 2), (2, 2)]);
        // 6 chars * 10 ms + 250 ms gap
        assert_eq!(n.pars[0].begin_ms, 0);
        assert_eq!(n.pars[0].end_ms, 310);
        assert_eq!(n.pars[1].begin_ms, 310);
        assert_eq!(n.pars[1].end_ms, 620);
        assert_eq!(n.duration_ms, 620);
        // the WAV is as long as the SMIL says
        let data = u32::from_le_bytes(n.wav[40..44].try_into().unwrap()) as u64;
        assert_eq!(data * 1000 / 2 / 8000, n.duration_ms);
        assert_eq!(n.wav.len() as u64, 44 + data);
        assert!(n.smil.contains("<text src=\"../text/c.xhtml#bxs2\"/>"));
        assert!(n.smil.contains(
            "<audio src=\"../audio/c.wav\" clipBegin=\"0:00:00.310\" clipEnd=\"0:00:00.620\"/>"
        ));
        // contiguous clips
        for w in n.pars.windows(2) {
            assert_eq!(w[0].end_ms, w[1].begin_ms);
        }
    }

    #[test]
    fn empty_synthesis_still_gets_a_clip() {
        struct Silent;
        impl SentenceSynth for Silent {
            fn synth(&mut self, _: &str) -> Result<Audio> {
                Ok(Audio {
                    rate: 8000,
                    samples: vec![],
                })
            }
        }
        let n = narrate_chapter(
            "<body><p>Hi.</p></body>",
            "a.xhtml",
            "s.smil",
            "a.wav",
            &mut Silent,
            0,
            &never,
            &mut |_, _| {},
        )
        .unwrap()
        .unwrap();
        assert_eq!(n.pars[0].end_ms - n.pars[0].begin_ms, MIN_CLIP_MS);
    }

    #[test]
    fn a_changing_sample_rate_is_resampled() {
        struct Two(u32);
        impl SentenceSynth for Two {
            fn synth(&mut self, _: &str) -> Result<Audio> {
                self.0 += 1;
                let rate = if self.0 == 1 { 8000 } else { 16000 };
                Ok(Audio {
                    rate,
                    samples: vec![5; rate as usize / 10],
                })
            }
        }
        let n = narrate_chapter(
            "<body><p>A b. C d.</p></body>",
            "a.xhtml",
            "s.smil",
            "a.wav",
            &mut Two(0),
            0,
            &never,
            &mut |_, _| {},
        )
        .unwrap()
        .unwrap();
        assert_eq!(n.duration_ms, 200);
    }

    #[test]
    fn a_chapter_without_text_is_none_and_zero_rate_is_an_error() {
        let mut f = Fake::new();
        let r = narrate_chapter(
            "<body><p>***</p></body>",
            "a",
            "b",
            "c",
            &mut f,
            0,
            &never,
            &mut |_, _| {},
        )
        .unwrap();
        assert!(r.is_none());
        struct Bad;
        impl SentenceSynth for Bad {
            fn synth(&mut self, _: &str) -> Result<Audio> {
                Ok(Audio::default())
            }
        }
        assert!(
            narrate_chapter(
                "<body><p>Hi.</p></body>",
                "a",
                "b",
                "c",
                &mut Bad,
                0,
                &never,
                &mut |_, _| {}
            )
            .is_err()
        );
    }

    #[test]
    fn a_book_is_narrated_valid_and_removable() {
        let mut book = make_book(&[BODY1, BODY2]);
        let before = book.archive.clone();
        let (todo, chars) = pending(&book);
        assert_eq!(todo, 2);
        assert_eq!(
            chars,
            "One".len()
                + "Hello there.".len()
                + "How are you?".len()
                + "Second chapter.".len()
                + "It has text & more.".len()
        );
        let opf_path = book.package.opf_path.clone();
        let opf_before = before.get_string(&opf_path).unwrap();
        let mut f = Fake::new();
        let mut last = None;
        let r = narrate_book(&mut book, &mut f, &Options::default(), &never, &mut |p| {
            last = Some(p.clone())
        })
        .unwrap();
        assert_eq!(
            (r.narrated, r.skipped, r.empty, r.cancelled),
            (2, 0, 0, false)
        );
        assert_eq!(last.unwrap().chapter, 2);
        assert_eq!(narration_status(&book), (2, 2));
        assert_eq!(pending(&book), (0, 0));
        let opf = book.archive.get_string(&opf_path).unwrap();
        assert!(opf.contains("media-overlay=\"mo-"), "{opf}");
        assert!(opf.contains("application/smil+xml"));
        assert!(opf.contains("media:active-class"));
        // the declared total is the sum of the chapters
        assert_eq!(declared_total(&opf), r.duration_ms);
        // every reference resolves and the clocks agree: no overlay finding at all
        let findings = check_overlays(&book.archive);
        let serious: Vec<_> = findings.iter().filter(|m| m.id != "MED-011").collect();
        assert!(serious.is_empty(), "{serious:?}");
        assert!(findings.iter().any(|m| m.id == "MED-011"));
        let rep = crate::validate::validate(&book.archive);
        assert!(!rep
            .messages
            .iter()
            .any(|m| m.id.starts_with("MED-") && m.severity == crate::validate::Severity::Error));
        // the second run has nothing to do
        let again = narrate_book(
            &mut book,
            &mut Fake::new(),
            &Options::default(),
            &never,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!((again.narrated, again.skipped), (0, 2));
        // removal gives the original book back
        let rm = remove_narration(&mut book).unwrap();
        assert_eq!(rm.chapters, 2);
        assert_eq!(rm.files, 4);
        for name in before.names() {
            if name == &opf_path {
                continue;
            }
            assert_eq!(
                book.archive.get(name),
                before.get(name),
                "{name} differs after removal"
            );
        }
        assert_eq!(book.archive.names().len(), before.names().len());
        assert_eq!(book.archive.get_string(&opf_path).unwrap(), opf_before);
        assert!(check_overlays(&book.archive).is_empty());
        assert_eq!(narration_status(&book), (2, 0));
    }

    #[test]
    fn cancelling_keeps_finished_chapters_and_resume_finishes() {
        let mut book = make_book(&[BODY1, BODY2]);
        let count = std::cell::Cell::new(0);
        // cancel while the second chapter is in progress (after 3 sentences were asked for)
        let cancel = || count.get() >= 3;
        struct Counting<'a>(&'a std::cell::Cell<usize>);
        impl SentenceSynth for Counting<'_> {
            fn synth(&mut self, t: &str) -> Result<Audio> {
                self.0.set(self.0.get() + 1);
                Ok(Audio {
                    rate: 8000,
                    samples: vec![0; t.len() * 80],
                })
            }
        }
        let r = narrate_book(
            &mut book,
            &mut Counting(&count),
            &Options::default(),
            &cancel,
            &mut |_| {},
        )
        .unwrap();
        assert!(r.cancelled);
        assert_eq!(r.narrated, 1);
        assert_eq!(narration_status(&book), (2, 1));
        // the partial book is valid
        let bad: Vec<_> = check_overlays(&book.archive)
            .into_iter()
            .filter(|m| m.id != "MED-011")
            .collect();
        assert!(bad.is_empty(), "{bad:?}");
        // the unfinished chapter has no spans left behind
        let ch2 = book.package.spine.get(1).map(|s| s.idref.clone()).unwrap();
        let path = resolve_href(
            parent_dir(&book.package.opf_path),
            &book
                .package
                .manifest
                .iter()
                .find(|m| m.id == ch2)
                .unwrap()
                .href,
        )
        .0;
        assert!(!book.archive.get_string(&path).unwrap().contains("bxs1"));
        // resume
        let r2 = narrate_book(
            &mut book,
            &mut Fake::new(),
            &Options::default(),
            &never,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!((r2.narrated, r2.skipped, r2.cancelled), (1, 1, false));
        assert_eq!(narration_status(&book), (2, 2));
        let opf = book.archive.get_string(&book.package.opf_path).unwrap();
        assert_eq!(opf.matches("<meta property=\"media:duration\">").count(), 1);
    }

    #[test]
    fn an_engine_error_keeps_what_is_done() {
        let mut book = make_book(&[BODY1, BODY2]);
        let mut f = Fake::new();
        f.fail_on = Some(3);
        let e =
            narrate_book(&mut book, &mut f, &Options::default(), &never, &mut |_| {}).unwrap_err();
        assert!(format!("{e:#}").contains("engine exploded"));
        assert_eq!(narration_status(&book), (2, 1));
    }

    #[test]
    fn epub2_is_refused() {
        let files = crate::testepub::clean_epub2();
        let mut book = OpenBook::from_archive(crate::testepub::to_archive(&files)).unwrap();
        let e = narrate_book(
            &mut book,
            &mut Fake::new(),
            &Options::default(),
            &never,
            &mut |_| {},
        )
        .unwrap_err();
        assert!(format!("{e}").contains("EPUB 3"));
    }
}
