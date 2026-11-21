//! One place in an e-book and its audiobook: mapping a reading position to a listening position
//! and back. Chapters with the same title in both are anchors; between anchors the position is
//! interpolated linearly (so with no matching title at all it is the same percentage of the
//! book). Pure functions: no files, no window.

use crate::model::AudioBook;

/// A chapter (spine item) of the e-book.
#[derive(Debug, Clone, PartialEq)]
pub struct TextChapter {
    /// Title from the table of contents; empty when the contents do not name this item.
    pub title: String,
    /// Share of the book: the size of the item's text (any unit, only ratios count).
    pub weight: f64,
}

/// Lower-case letters and digits only, so `Chapter 3: The Fall` and `chapter 3 - the fall`
/// match.
pub fn normalize(title: &str) -> String {
    title
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Points of the two scales that are known to be the same place: (share of the e-book text,
/// milliseconds of audio), both increasing.
fn anchors(text: &[TextChapter], audio: &AudioBook) -> Vec<(f64, f64)> {
    let total: f64 = text.iter().map(|c| c.weight.max(0.0)).sum();
    let total_ms = audio.total_ms as f64;
    let mut out = vec![(0.0, 0.0)];
    if total <= 0.0 || total_ms <= 0.0 {
        out.push((1.0, 1.0));
        return out;
    }
    // titles that occur once on each side
    let count = |name: &str, it: &mut dyn Iterator<Item = String>| it.filter(|t| t == name).count();
    let mut before = 0.0;
    for c in text {
        let start = before / total;
        before += c.weight.max(0.0);
        let name = normalize(&c.title);
        if name.is_empty() {
            continue;
        }
        if count(&name, &mut text.iter().map(|t| normalize(&t.title))) != 1 {
            continue;
        }
        let mut hits = audio
            .chapters
            .iter()
            .filter(|a| normalize(&a.title) == name);
        let (Some(a), None) = (hits.next(), hits.next()) else {
            continue;
        };
        let (t, ms) = (start, a.start_ms as f64);
        let last = *out.last().unwrap_or(&(0.0, 0.0));
        if t == 0.0 && out.len() == 1 && ms < total_ms {
            // the first chapter of the text starts after something the text does not have
            out[0] = (0.0, ms);
        } else if t > last.0 && ms > last.1 && ms < total_ms {
            out.push((t, ms));
        }
    }
    out.push((1.0, total_ms));
    out
}

fn interpolate(points: &[(f64, f64)], x: f64, from_first: bool) -> f64 {
    let a_of = |p: &(f64, f64)| if from_first { p.0 } else { p.1 };
    let b_of = |p: &(f64, f64)| if from_first { p.1 } else { p.0 };
    for w in points.windows(2) {
        let (lo, hi) = (a_of(&w[0]), a_of(&w[1]));
        if x <= hi {
            if hi <= lo {
                return b_of(&w[0]);
            }
            let f = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
            return b_of(&w[0]) + (b_of(&w[1]) - b_of(&w[0])) * f;
        }
    }
    points.last().map_or(0.0, b_of)
}

/// The share (0..1) of the e-book text before position (`spine`, `frac`).
fn text_share(text: &[TextChapter], spine: usize, frac: f64) -> f64 {
    let total: f64 = text.iter().map(|c| c.weight.max(0.0)).sum();
    if total <= 0.0 {
        return 0.0;
    }
    let frac = if frac.is_finite() {
        frac.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let before: f64 = text.iter().take(spine).map(|c| c.weight.max(0.0)).sum();
    let here = text.get(spine).map_or(0.0, |c| c.weight.max(0.0)) * frac;
    ((before + here) / total).clamp(0.0, 1.0)
}

/// Where in the audiobook (milliseconds) the reader of the e-book at (`spine`, `frac`) is.
pub fn text_to_audio(text: &[TextChapter], spine: usize, frac: f64, audio: &AudioBook) -> u64 {
    let pts = anchors(text, audio);
    let share = text_share(text, spine, frac);
    let ms = interpolate(&pts, share, true);
    (ms.max(0.0) as u64).min(audio.total_ms)
}

/// Where in the e-book (item, fraction of the item) the listener at `pos_ms` is.
pub fn audio_to_text(audio: &AudioBook, pos_ms: u64, text: &[TextChapter]) -> (usize, f64) {
    if text.is_empty() {
        return (0, 0.0);
    }
    let pts = anchors(text, audio);
    let share = interpolate(&pts, pos_ms.min(audio.total_ms) as f64, false);
    let total: f64 = text.iter().map(|c| c.weight.max(0.0)).sum();
    if total <= 0.0 {
        return (0, 0.0);
    }
    let target = share * total;
    let mut before = 0.0;
    for (i, c) in text.iter().enumerate() {
        let w = c.weight.max(0.0);
        if target < before + w || i + 1 == text.len() {
            let f = if w > 0.0 {
                ((target - before) / w).clamp(0.0, 1.0)
            } else {
                0.0
            };
            return (i, f);
        }
        before += w;
    }
    (text.len() - 1, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AudioBook, Chapter};

    fn audio(chapters: &[(&str, u64, u64)]) -> AudioBook {
        let total = chapters.iter().map(|c| c.2).max().unwrap_or(0);
        AudioBook {
            total_ms: total,
            chapters: chapters
                .iter()
                .map(|(t, s, e)| Chapter {
                    title: (*t).into(),
                    start_ms: *s,
                    end_ms: *e,
                    part: 0,
                })
                .collect(),
            ..Default::default()
        }
    }

    fn text(items: &[(&str, f64)]) -> Vec<TextChapter> {
        items
            .iter()
            .map(|(t, w)| TextChapter {
                title: (*t).into(),
                weight: *w,
            })
            .collect()
    }

    #[test]
    fn titles_match_without_regard_to_case_and_punctuation() {
        assert_eq!(
            normalize("Chapter 3: The Fall"),
            normalize("chapter 3 - the fall")
        );
        assert_ne!(normalize("Chapter 3"), normalize("Chapter 4"));
    }

    #[test]
    fn without_matching_titles_the_percentage_is_kept() {
        let a = audio(&[("Part 1", 0, 600_000), ("Part 2", 600_000, 1_000_000)]);
        let t = text(&[("", 10.0), ("", 30.0)]);
        // the middle of the second item is 25/40 of the text, so 25/40 of the audio
        assert_eq!(text_to_audio(&t, 1, 0.5, &a), 625_000);
        let (i, f) = audio_to_text(&a, 625_000, &t);
        assert_eq!(i, 1);
        assert!((f - 0.5).abs() < 1e-9);
    }

    #[test]
    fn matching_chapter_titles_anchor_the_two_scales() {
        // the audio has a long intro the text does not; chapter titles line the books up
        let a = audio(&[
            ("Intro", 0, 300_000),
            ("One", 300_000, 400_000),
            ("Two", 400_000, 600_000),
        ]);
        let t = text(&[("One", 10.0), ("Two", 10.0)]);
        // halfway through "Two" in the text is halfway through "Two" in the audio
        assert_eq!(text_to_audio(&t, 1, 0.5, &a), 500_000);
        assert_eq!(text_to_audio(&t, 0, 0.0, &a), 300_000);
        let (i, f) = audio_to_text(&a, 500_000, &t);
        assert_eq!((i, (f * 100.0).round() as i32), (1, 50));
        // in the intro, the reader is at the start
        assert_eq!(audio_to_text(&a, 100_000, &t).0, 0);
    }

    #[test]
    fn ambiguous_or_missing_titles_are_not_anchors() {
        let a = audio(&[("A", 0, 500_000), ("A", 500_000, 1_000_000)]);
        let t = text(&[("A", 1.0), ("A", 1.0)]);
        assert_eq!(text_to_audio(&t, 1, 0.0, &a), 500_000);
    }

    #[test]
    fn the_round_trip_stays_close_and_edges_are_safe() {
        let a = audio(&[("One", 0, 400_000), ("Two", 400_000, 1_000_000)]);
        let t = text(&[("One", 4.0), ("Two", 6.0)]);
        for ms in [0u64, 1, 123_456, 400_000, 999_999, 1_000_000] {
            let (i, f) = audio_to_text(&a, ms, &t);
            let back = text_to_audio(&t, i, f, &a);
            assert!(back.abs_diff(ms) < 2, "{ms} -> {back}");
        }
        assert_eq!(text_to_audio(&t, 99, 7.0, &a), 1_000_000);
        assert_eq!(text_to_audio(&t, 0, f64::NAN, &a), 0);
        assert_eq!(audio_to_text(&a, 5_000_000, &t).0, 1);
        assert_eq!(audio_to_text(&a, 10, &[]), (0, 0.0));
        let empty = audio(&[]);
        assert_eq!(text_to_audio(&t, 1, 0.5, &empty), 0);
        assert_eq!(text_to_audio(&[], 0, 0.5, &a), 0);
    }

    #[test]
    fn anchors_that_run_backwards_are_dropped() {
        // the audio lists the chapters in another order than the text
        let a = audio(&[("Two", 0, 500_000), ("One", 500_000, 1_000_000)]);
        let t = text(&[("One", 1.0), ("Two", 1.0)]);
        let ms = text_to_audio(&t, 1, 0.0, &a);
        assert!(ms <= 1_000_000);
    }
}
