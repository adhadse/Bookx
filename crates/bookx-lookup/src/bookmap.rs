//! Data model for the book map / page browser: chapter segments sized by word
//! count with highlight and bookmark marks placed on them.

use anyhow::{bail, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterInfo {
    pub title: String,
    pub words: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkKind {
    Highlight,
    Bookmark,
    Note,
}

/// A mark inside a chapter at `fraction` (0.0..=1.0) of that chapter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mark {
    pub chapter: usize,
    pub fraction: f64,
    pub kind: MarkKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapMark {
    /// Position as a fraction of the whole book.
    pub at: f64,
    pub kind: MarkKind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapSegment {
    pub chapter: usize,
    pub title: String,
    /// Book fractions covered by the chapter.
    pub start: f64,
    pub end: f64,
    pub marks: Vec<MapMark>,
}

/// Lays chapters out along `0.0..=1.0` proportionally to their word counts (empty
/// chapters get a minimal weight of one word) and places the marks, sorted by position.
pub fn build_book_map(chapters: &[ChapterInfo], marks: &[Mark]) -> Result<Vec<MapSegment>> {
    let total: u64 = chapters.iter().map(|c| c.words.max(1)).sum();
    let mut segments = Vec::with_capacity(chapters.len());
    let mut acc = 0u64;
    for (i, c) in chapters.iter().enumerate() {
        let w = c.words.max(1);
        segments.push(MapSegment {
            chapter: i,
            title: c.title.clone(),
            start: acc as f64 / total as f64,
            end: (acc + w) as f64 / total as f64,
            marks: Vec::new(),
        });
        acc += w;
    }
    for m in marks {
        if !m.fraction.is_finite() {
            bail!("mark fraction must be a finite number");
        }
        let Some(seg) = segments.get_mut(m.chapter) else {
            bail!(
                "mark refers to chapter {} but the book has {}",
                m.chapter,
                chapters.len()
            );
        };
        let f = m.fraction.clamp(0.0, 1.0);
        seg.marks.push(MapMark {
            at: seg.start + (seg.end - seg.start) * f,
            kind: m.kind,
        });
    }
    for s in &mut segments {
        s.marks.sort_by(|a, b| a.at.total_cmp(&b.at));
    }
    Ok(segments)
}

/// The chapter containing a book fraction (clamped to the book).
pub fn chapter_at(map: &[MapSegment], fraction: f64) -> Option<usize> {
    if map.is_empty() || fraction.is_nan() {
        return None;
    }
    let f = fraction.clamp(0.0, 1.0);
    map.iter()
        .find(|s| f < s.end)
        .or(map.last())
        .map(|s| s.chapter)
}

/// Number of marks per equal-width bucket of the book (for a density strip).
pub fn mark_density(map: &[MapSegment], buckets: usize) -> Vec<u32> {
    let mut out = vec![0u32; buckets];
    if buckets == 0 {
        return out;
    }
    for m in map.iter().flat_map(|s| &s.marks) {
        let i = ((m.at * buckets as f64) as usize).min(buckets - 1);
        out[i] += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(t: &str, w: u64) -> ChapterInfo {
        ChapterInfo {
            title: t.into(),
            words: w,
        }
    }

    #[test]
    fn layout_is_proportional_and_contiguous() {
        let m = build_book_map(&[ch("a", 100), ch("b", 300), ch("c", 100)], &[]).unwrap();
        assert_eq!(m.len(), 3);
        assert_eq!((m[0].start, m[0].end), (0.0, 0.2));
        assert!((m[1].end - 0.8).abs() < 1e-12);
        assert_eq!(m[2].end, 1.0);
        for w in m.windows(2) {
            assert_eq!(w[0].end, w[1].start);
        }
        assert_eq!(m[1].title, "b");
    }

    #[test]
    fn empty_chapters_get_minimal_weight_and_empty_book_is_ok() {
        let m = build_book_map(&[ch("a", 0), ch("b", 0)], &[]).unwrap();
        assert_eq!((m[0].end, m[1].end), (0.5, 1.0));
        assert!(build_book_map(&[], &[]).unwrap().is_empty());
    }

    #[test]
    fn marks_are_placed_clamped_and_sorted() {
        let marks = [
            Mark {
                chapter: 1,
                fraction: 0.5,
                kind: MarkKind::Highlight,
            },
            Mark {
                chapter: 0,
                fraction: 2.0,
                kind: MarkKind::Bookmark,
            },
            Mark {
                chapter: 1,
                fraction: 0.25,
                kind: MarkKind::Note,
            },
            Mark {
                chapter: 0,
                fraction: -1.0,
                kind: MarkKind::Bookmark,
            },
        ];
        let m = build_book_map(&[ch("a", 100), ch("b", 100)], &marks).unwrap();
        assert_eq!(
            m[0].marks.iter().map(|x| x.at).collect::<Vec<_>>(),
            vec![0.0, 0.5]
        );
        assert_eq!(m[1].marks[0].kind, MarkKind::Note);
        assert!((m[1].marks[0].at - 0.625).abs() < 1e-12);
        assert!((m[1].marks[1].at - 0.75).abs() < 1e-12);
    }

    #[test]
    fn invalid_marks_are_errors() {
        let c = [ch("a", 1)];
        let bad_ch = [Mark {
            chapter: 1,
            fraction: 0.0,
            kind: MarkKind::Note,
        }];
        assert!(build_book_map(&c, &bad_ch)
            .unwrap_err()
            .to_string()
            .contains("chapter 1"));
        let nan = [Mark {
            chapter: 0,
            fraction: f64::NAN,
            kind: MarkKind::Note,
        }];
        assert!(build_book_map(&c, &nan).is_err());
        let inf = [Mark {
            chapter: 0,
            fraction: f64::INFINITY,
            kind: MarkKind::Note,
        }];
        assert!(build_book_map(&c, &inf).is_err());
    }

    #[test]
    fn chapter_lookup_by_fraction() {
        let m = build_book_map(&[ch("a", 100), ch("b", 100)], &[]).unwrap();
        assert_eq!(chapter_at(&m, 0.0), Some(0));
        assert_eq!(chapter_at(&m, 0.49), Some(0));
        assert_eq!(chapter_at(&m, 0.5), Some(1));
        assert_eq!(chapter_at(&m, 1.0), Some(1));
        assert_eq!(chapter_at(&m, 7.0), Some(1));
        assert_eq!(chapter_at(&m, -3.0), Some(0));
        assert_eq!(chapter_at(&m, f64::NAN), None);
        assert_eq!(chapter_at(&[], 0.5), None);
    }

    #[test]
    fn density_buckets() {
        let marks = [
            Mark {
                chapter: 0,
                fraction: 0.0,
                kind: MarkKind::Note,
            },
            Mark {
                chapter: 0,
                fraction: 1.0,
                kind: MarkKind::Note,
            },
            Mark {
                chapter: 1,
                fraction: 1.0,
                kind: MarkKind::Note,
            },
            Mark {
                chapter: 1,
                fraction: 0.1,
                kind: MarkKind::Note,
            },
        ];
        let m = build_book_map(&[ch("a", 10), ch("b", 10)], &marks).unwrap();
        assert_eq!(mark_density(&m, 4), vec![1, 0, 2, 1]);
        assert_eq!(mark_density(&m, 1), vec![4]);
        assert!(mark_density(&m, 0).is_empty());
        assert_eq!(mark_density(&[], 3), vec![0, 0, 0]);
    }
}
