//! An audiobook as parts (files) and chapters, and the arithmetic on positions.
//!
//! A position is milliseconds from the start of the whole book (all parts end to end). Chapters
//! come from the files' chapter marks or, without marks, one per part.

use serde::{Deserialize, Serialize};

use crate::format::Format;
use crate::limits::{MAX_CHAPTERS, MAX_PARTS};
use crate::probe::ChapterMark;

/// One audio file of the book.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Part {
    /// Relative to the folder of the audiobook, or absolute.
    pub path: String,
    pub title: String,
    pub duration_ms: u64,
    pub format: Format,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chapter {
    pub title: String,
    pub start_ms: u64,
    pub end_ms: u64,
    /// Index of the part the chapter starts in.
    pub part: usize,
}

impl Chapter {
    pub fn duration_ms(&self) -> u64 {
        self.end_ms.saturating_sub(self.start_ms)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioBook {
    pub title: String,
    pub authors: Vec<String>,
    pub narrator: String,
    pub series: String,
    pub series_index: Option<f64>,
    pub language: String,
    pub description: String,
    pub parts: Vec<Part>,
    pub chapters: Vec<Chapter>,
    pub total_ms: u64,
}

/// A press of "previous chapter" within this time of the chapter start goes to the chapter before.
pub const RESTART_WINDOW_MS: u64 = 3_000;
/// Resuming rewinds this much so you hear the sentence you stopped in.
pub const RESUME_REWIND_MS: u64 = 2_000;

impl AudioBook {
    /// Puts parts end to end and numbers the chapters. `parts` come with their own chapter marks
    /// (times from the start of that part).
    pub fn assemble(mut book: AudioBook, parts: Vec<(Part, Vec<ChapterMark>)>) -> AudioBook {
        book.parts.clear();
        book.chapters.clear();
        let mut start = 0u64;
        for (i, (part, marks)) in parts.into_iter().take(MAX_PARTS).enumerate() {
            let marks: Vec<&ChapterMark> = marks
                .iter()
                .filter(|m| m.start_ms < part.duration_ms.max(1))
                .collect();
            if marks.is_empty() {
                book.chapters.push(Chapter {
                    title: part.title.clone(),
                    start_ms: start,
                    end_ms: start + part.duration_ms,
                    part: i,
                });
            } else {
                for (k, m) in marks.iter().enumerate() {
                    if book.chapters.len() >= MAX_CHAPTERS {
                        break;
                    }
                    // the first chapter also covers any lead-in before its mark
                    let s = if k == 0 { start } else { start + m.start_ms };
                    book.chapters.push(Chapter {
                        title: m.title.clone(),
                        start_ms: s,
                        end_ms: s,
                        part: i,
                    });
                }
            }
            start += part.duration_ms;
            book.parts.push(part);
        }
        book.total_ms = start;
        // each chapter ends where the next one starts
        let n = book.chapters.len();
        for i in 0..n {
            let end = if i + 1 < n {
                book.chapters[i + 1].start_ms
            } else {
                book.total_ms
            };
            book.chapters[i].end_ms = end.max(book.chapters[i].start_ms);
        }
        for (i, c) in book.chapters.iter_mut().enumerate() {
            if c.title.trim().is_empty() {
                c.title = format!("Chapter {}", i + 1);
            }
        }
        book
    }

    pub fn part_start(&self, idx: usize) -> u64 {
        self.parts.iter().take(idx).map(|p| p.duration_ms).sum()
    }

    /// (part index, milliseconds into that part). Past the end: the end of the last part.
    pub fn locate(&self, pos_ms: u64) -> (usize, u64) {
        let mut start = 0u64;
        for (i, p) in self.parts.iter().enumerate() {
            if pos_ms < start + p.duration_ms || i + 1 == self.parts.len() {
                return (i, pos_ms.saturating_sub(start).min(p.duration_ms));
            }
            start += p.duration_ms;
        }
        (0, 0)
    }

    pub fn chapter_index_at(&self, pos_ms: u64) -> usize {
        self.chapters
            .iter()
            .rposition(|c| c.start_ms <= pos_ms)
            .unwrap_or_default()
    }

    pub fn chapter_at(&self, pos_ms: u64) -> Option<&Chapter> {
        self.chapters.get(self.chapter_index_at(pos_ms))
    }

    /// Position of the start of chapter `idx` (clamped).
    pub fn chapter_start(&self, idx: usize) -> u64 {
        self.chapters
            .get(idx.min(self.chapters.len().saturating_sub(1)))
            .map(|c| c.start_ms)
            .unwrap_or(0)
    }

    /// "Previous chapter": the start of this chapter, or of the one before when it just began.
    pub fn previous_chapter(&self, pos_ms: u64) -> u64 {
        let i = self.chapter_index_at(pos_ms);
        let start = self.chapter_start(i);
        if pos_ms.saturating_sub(start) > RESTART_WINDOW_MS || i == 0 {
            start
        } else {
            self.chapter_start(i - 1)
        }
    }

    /// "Next chapter": `None` in the last one.
    pub fn next_chapter(&self, pos_ms: u64) -> Option<u64> {
        let i = self.chapter_index_at(pos_ms);
        self.chapters.get(i + 1).map(|c| c.start_ms)
    }

    /// Moves by `delta_ms` (negative: back), clamped to the book.
    pub fn skip(&self, pos_ms: u64, delta_ms: i64) -> u64 {
        let p = pos_ms as i128 + delta_ms as i128;
        p.clamp(0, self.total_ms as i128) as u64
    }

    /// 0..=1 of the whole book.
    pub fn progress(&self, pos_ms: u64) -> f64 {
        if self.total_ms == 0 {
            0.0
        } else {
            (pos_ms.min(self.total_ms) as f64 / self.total_ms as f64).clamp(0.0, 1.0)
        }
    }

    /// Listening time left at `speed`.
    pub fn time_left_ms(&self, pos_ms: u64, speed: f32) -> u64 {
        let left = self.total_ms.saturating_sub(pos_ms);
        (left as f64 / crate::stretch::clamp_speed(speed) as f64) as u64
    }

    /// Listening time left in the current chapter at `speed`.
    pub fn chapter_left_ms(&self, pos_ms: u64, speed: f32) -> u64 {
        let end = self
            .chapter_at(pos_ms)
            .map(|c| c.end_ms)
            .unwrap_or(self.total_ms);
        (end.saturating_sub(pos_ms) as f64 / crate::stretch::clamp_speed(speed) as f64) as u64
    }

    /// The catalogue's reading position: (chapter, fraction of the chapter).
    pub fn to_readpos(&self, pos_ms: u64) -> (usize, f64) {
        let i = self.chapter_index_at(pos_ms);
        match self.chapters.get(i) {
            Some(c) if c.duration_ms() > 0 => (
                i,
                ((pos_ms.saturating_sub(c.start_ms)) as f64 / c.duration_ms() as f64)
                    .clamp(0.0, 1.0),
            ),
            _ => (i, 0.0),
        }
    }

    /// The inverse of [`AudioBook::to_readpos`].
    pub fn from_readpos(&self, chapter: usize, frac: f64) -> u64 {
        match self.chapters.get(chapter) {
            Some(c) => {
                let f = if frac.is_finite() {
                    frac.clamp(0.0, 1.0)
                } else {
                    0.0
                };
                (c.start_ms + (c.duration_ms() as f64 * f) as u64).min(self.total_ms)
            }
            None => 0,
        }
    }

    /// Where playback starts when the book is opened at `pos_ms`: a little before, but never
    /// before the start of the book, and from the start when the book was finished.
    pub fn resume_position(&self, pos_ms: u64) -> u64 {
        if self.total_ms > 0 && pos_ms + 1_000 >= self.total_ms {
            return 0;
        }
        pos_ms.saturating_sub(RESUME_REWIND_MS)
    }
}

/// `1:02:03` or `4:05`.
pub fn clock(ms: u64) -> String {
    let s = ms / 1000;
    let (h, m, sec) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

/// `(hours, minutes)` rounded to the minute, for "3 h 12 min left".
pub fn hours_minutes(ms: u64) -> (u64, u64) {
    let m = (ms + 30_000) / 60_000;
    (m / 60, m % 60)
}

/// Rounds to the 15 seconds the skip buttons use; avoids sub-second drift after many skips.
pub fn snap_ms(ms: u64) -> u64 {
    ms / 1000 * 1000
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(name: &str, secs: u64) -> Part {
        Part {
            path: name.into(),
            title: name.into(),
            duration_ms: secs * 1000,
            format: Format::Mp3,
        }
    }

    fn mark(t: &str, secs: u64) -> ChapterMark {
        ChapterMark {
            title: t.into(),
            start_ms: secs * 1000,
        }
    }

    fn book() -> AudioBook {
        AudioBook::assemble(
            AudioBook::default(),
            vec![
                (part("a", 100), vec![mark("One", 0), mark("Two", 40)]),
                (part("b", 50), vec![]),
                (part("c", 150), vec![mark("", 10)]),
            ],
        )
    }

    #[test]
    fn parts_and_chapters_are_laid_end_to_end() {
        let b = book();
        assert_eq!(b.total_ms, 300_000);
        let c: Vec<_> = b
            .chapters
            .iter()
            .map(|c| (c.title.as_str(), c.start_ms / 1000, c.end_ms / 1000, c.part))
            .collect();
        assert_eq!(
            c,
            vec![
                ("One", 0, 40, 0),
                ("Two", 40, 100, 0),
                ("b", 100, 150, 1),
                ("Chapter 4", 150, 300, 2)
            ]
        );
        assert_eq!(b.part_start(2), 150_000);
    }

    #[test]
    fn locate_finds_the_part_and_offset() {
        let b = book();
        assert_eq!(b.locate(0), (0, 0));
        assert_eq!(b.locate(99_999), (0, 99_999));
        assert_eq!(b.locate(100_000), (1, 0));
        assert_eq!(b.locate(160_500), (2, 10_500));
        assert_eq!(b.locate(9_999_999), (2, 150_000), "past the end is the end");
        assert_eq!(AudioBook::default().locate(5), (0, 0));
    }

    #[test]
    fn chapter_navigation() {
        let b = book();
        assert_eq!(b.chapter_index_at(39_999), 0);
        assert_eq!(b.chapter_index_at(40_000), 1);
        assert_eq!(b.next_chapter(10_000), Some(40_000));
        assert_eq!(b.next_chapter(290_000), None);
        // previous: restart the chapter, or go back when it just began
        assert_eq!(b.previous_chapter(60_000), 40_000);
        assert_eq!(b.previous_chapter(41_000), 0);
        assert_eq!(b.previous_chapter(1_000), 0);
        assert_eq!(b.chapter_start(99), 150_000, "clamped");
    }

    #[test]
    fn skipping_is_clamped() {
        let b = book();
        assert_eq!(b.skip(10_000, -15_000), 0);
        assert_eq!(b.skip(10_000, 30_000), 40_000);
        assert_eq!(b.skip(299_000, 30_000), 300_000);
        assert_eq!(b.skip(0, i64::MIN), 0);
        assert_eq!(b.skip(u64::MAX / 2, i64::MAX), 300_000);
    }

    #[test]
    fn progress_and_time_left_follow_speed() {
        let b = book();
        assert!((b.progress(150_000) - 0.5).abs() < 1e-9);
        assert_eq!(b.progress(u64::MAX), 1.0);
        assert_eq!(b.time_left_ms(150_000, 1.0), 150_000);
        assert_eq!(b.time_left_ms(150_000, 2.0), 75_000);
        assert_eq!(b.time_left_ms(150_000, 0.5), 300_000);
        assert_eq!(
            b.time_left_ms(150_000, 99.0),
            50_000,
            "speed is clamped to 3x"
        );
        assert_eq!(b.chapter_left_ms(30_000, 2.0), 5_000);
        assert_eq!(AudioBook::default().progress(5), 0.0);
    }

    #[test]
    fn the_catalogue_position_round_trips() {
        let b = book();
        for pos in [0u64, 1, 39_999, 40_000, 77_777, 149_999, 150_000, 299_999] {
            let (c, f) = b.to_readpos(pos);
            let back = b.from_readpos(c, f);
            assert!(back.abs_diff(pos) <= 1, "{pos} -> ({c},{f}) -> {back}");
        }
        assert_eq!(b.from_readpos(99, 0.5), 0);
        assert_eq!(b.from_readpos(0, f64::NAN), 0);
        assert_eq!(b.from_readpos(0, 7.0), 40_000);
    }

    #[test]
    fn resume_rewinds_a_little_and_restarts_a_finished_book() {
        let b = book();
        assert_eq!(b.resume_position(50_000), 48_000);
        assert_eq!(b.resume_position(1_000), 0);
        assert_eq!(b.resume_position(299_500), 0, "finished");
        assert_eq!(b.resume_position(250_000), 248_000);
    }

    #[test]
    fn clocks() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(65_000), "1:05");
        assert_eq!(clock(3_723_000), "1:02:03");
        assert_eq!(hours_minutes(11_130_000), (3, 6));
        assert_eq!(hours_minutes(29_999), (0, 0));
        assert_eq!(snap_ms(12_345), 12_000);
    }

    #[test]
    fn marks_beyond_the_part_are_ignored() {
        let b = AudioBook::assemble(
            AudioBook::default(),
            vec![(part("a", 10), vec![mark("X", 0), mark("Late", 500)])],
        );
        assert_eq!(b.chapters.len(), 1);
    }

    #[test]
    fn json_round_trip() {
        let b = book();
        let s = serde_json::to_string(&b).unwrap();
        assert_eq!(serde_json::from_str::<AudioBook>(&s).unwrap(), b);
    }
}
