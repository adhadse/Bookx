//! *Save as audio…*: a chapter or a whole book read into WAV files. This file plans the files
//! and runs the synthesis (the caller does it on a worker thread); the dialog is in
//! `ui::reader_tts`. The audio comes from the engine the settings ask for (Kokoro, Piper or a
//! voice server); the basic voice cannot give audio. MP3/OGG need an encoder, `ffmpeg -i x.wav
//! x.mp3` does it.

use std::io::BufWriter;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use bookx_lookup::export::sanitize_filename;
use bookx_tts::export::synthesize_to_wav;
use bookx_tts::synth::{Cancel, Synthesizer};
use bookx_tts::text::Sentence;

/// Milliseconds of silence between two sentences in a file.
pub const GAP_MS: u32 = 250;

/// One file to write.
pub struct Job {
    pub file: String,
    pub title: String,
    pub sentences: Vec<Sentence>,
}

/// The files for `chapters` (title, text): one per chapter that has something to read, named
/// `01 - Title.wav`. The numbers count chapters of the book, so a chapter without text leaves a gap
/// in the numbering and the order of the files stays the book's.
pub fn plan(chapters: &[(String, String)]) -> Vec<Job> {
    let width = chapters.len().to_string().len().max(2);
    chapters
        .iter()
        .enumerate()
        .filter_map(|(i, (title, text))| {
            let sentences = crate::tts::split_sentences(text);
            if sentences.is_empty() {
                return None;
            }
            let t = if title.trim().is_empty() {
                format!("Chapter {}", i + 1)
            } else {
                title.trim().to_string()
            };
            Some(Job {
                file: format!("{:0width$} - {}.wav", i + 1, sanitize_filename(&t)),
                title: t,
                sentences,
            })
        })
        .collect()
}

/// A job for a single chapter written to the file `name`.
pub fn single(title: &str, text: &str, name: &str) -> Option<Job> {
    let sentences = crate::tts::split_sentences(text);
    (!sentences.is_empty()).then(|| Job {
        file: name.to_string(),
        title: title.to_string(),
        sentences,
    })
}

/// Writes every job into `dir`. `progress` gets (sentences done, sentences in all, title). A
/// cancelled or failed run removes the file it was writing; finished files stay. Returns the files.
#[allow(clippy::too_many_arguments)]
pub fn run(
    jobs: &[Job],
    dir: &Path,
    synth: &mut dyn Synthesizer,
    voice: &str,
    language: &str,
    speed: f64,
    pitch: f64,
    cancel: &Cancel,
    progress: &mut dyn FnMut(usize, usize, &str),
) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let total: usize = jobs.iter().map(|j| j.sentences.len()).sum();
    let mut done = 0usize;
    let mut out = Vec::new();
    for j in jobs {
        let path = dir.join(&j.file);
        let file =
            std::fs::File::create(&path).with_context(|| format!("creating {}", path.display()))?;
        let base = done;
        let r = synthesize_to_wav(
            synth,
            &j.sentences,
            voice,
            language,
            speed,
            pitch,
            GAP_MS,
            cancel,
            &mut |d, _| progress(base + d, total, &j.title),
            BufWriter::new(file),
        );
        match r {
            Ok(_) => {
                done += j.sentences.len();
                out.push(path);
            }
            Err(e) => {
                let _ = std::fs::remove_file(&path);
                return Err(e);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_tts::session::Voice;
    use bookx_tts::synth::{Pcm, SynthRequest};

    struct Beep;
    impl Synthesizer for Beep {
        fn id(&self) -> String {
            "beep".into()
        }
        fn voices(&mut self) -> Result<Vec<Voice>> {
            Ok(vec![])
        }
        fn synthesize(&mut self, r: &SynthRequest) -> Result<Pcm> {
            Ok(Pcm {
                rate: 8000,
                samples: vec![0.2; r.text.len() * 100],
            })
        }
    }

    fn tmp() -> PathBuf {
        std::env::temp_dir().join(format!("bookx-tts-export-{}", uuid::Uuid::new_v4()))
    }

    fn chapters() -> Vec<(String, String)> {
        vec![
            ("Cover".into(), "".into()),
            ("One: The/Start".into(), "First. Second.".into()),
            ("".into(), "Third one here.".into()),
        ]
    }

    #[test]
    fn the_plan_numbers_the_chapters_of_the_book_and_skips_empty_ones() {
        let jobs = plan(&chapters());
        let names: Vec<_> = jobs.iter().map(|j| j.file.as_str()).collect();
        assert_eq!(names.len(), 2);
        assert!(names[0].starts_with("02 - One") && names[0].ends_with(".wav"));
        assert!(!names[0].contains('/'));
        assert_eq!(names[1], "03 - Chapter 3.wav");
        assert_eq!(jobs[0].sentences.len(), 2);
        assert!(single("t", "  ", "x.wav").is_none());
    }

    #[test]
    fn a_run_writes_valid_wav_files_and_reports_progress_over_the_whole_book() {
        let dir = tmp();
        let jobs = plan(&chapters());
        let mut seen = Vec::new();
        let files = run(
            &jobs,
            &dir,
            &mut Beep,
            "",
            "en",
            1.0,
            1.0,
            &Cancel::new(),
            &mut |d, t, title| seen.push((d, t, title.to_string())),
        )
        .unwrap();
        assert_eq!(files.len(), 2);
        let pcm = bookx_tts::synth::parse_wav(&std::fs::read(&files[0]).unwrap()).unwrap();
        assert_eq!(pcm.rate, 8000);
        assert_eq!(pcm.samples.len(), (6 + 7) * 100 + 2000);
        assert_eq!(seen.last().map(|s| (s.0, s.1)), Some((3, 3)));
        assert_eq!(seen[0].2, jobs[0].title);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_cancelled_run_leaves_no_half_written_file() {
        let dir = tmp();
        let jobs = plan(&chapters());
        let c = Cancel::new();
        let c2 = c.clone();
        let e = run(
            &jobs,
            &dir,
            &mut Beep,
            "",
            "en",
            1.0,
            1.0,
            &c,
            &mut |d, _, _| {
                if d == 1 {
                    c2.cancel();
                }
            },
        )
        .unwrap_err();
        assert_eq!(e.to_string(), "cancelled");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
