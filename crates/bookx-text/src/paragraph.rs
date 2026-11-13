//! Optimal paragraph layout for a renderer that measures with its own font engine (R3).
//!
//! The renderer (Pango in the reader canvas) measures where every character of the paragraph would
//! be on one unbroken line. [`plan`] runs Knuth-Plass over that and answers with *edits* to the
//! text (forced line breaks) and *gaps* (extra width of the spaces that justifies each line). The
//! renderer applies the edits, lays the text out again and stretches the gaps with letter spacing.
//!
//! Edited text keeps its canonical meaning: the characters the edits add are "ghosts"
//! ([`is_ghost`]), they never count as book text, and a space that became a line break reads as
//! a space again ([`canonical_char`]). Callers must keep the book's own U+2010, U+2028 and
//! U+2029 out of the text ([`sanitize`]).

use crate::linebreak::{break_paragraph, Item, Line, Params};

/// A soft hyphen: where words may be hyphenated.
pub const SHY: char = '\u{ad}';
/// Replaces a space at a line break; shows as nothing and reads as a space.
pub const LINE_BREAK: char = '\u{2028}';
/// The hyphen drawn at a hyphenated line break (followed by a [`LINE_BREAK`] ghost).
pub const GHOST_HYPHEN: char = '\u{2010}';

/// Replaces the characters that this module uses as markers.
pub fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            GHOST_HYPHEN => '-',
            LINE_BREAK | '\u{2029}' => ' ',
            c => c,
        })
        .collect()
}

/// Whether `c` (preceded by `prev`) is not part of the book's text.
pub fn is_ghost(prev: Option<char>, c: char) -> bool {
    c == SHY || c == GHOST_HYPHEN || (c == LINE_BREAK && prev == Some(GHOST_HYPHEN))
}

/// The book-text character `c` stands for, `None` for ghosts.
pub fn canonical_char(prev: Option<char>, c: char) -> Option<char> {
    if is_ghost(prev, c) {
        None
    } else if c == LINE_BREAK {
        Some(' ')
    } else {
        Some(c)
    }
}

/// The canonical characters of `text` with their index among all of `text`'s characters.
pub fn canonical_chars(text: &str) -> impl Iterator<Item = (usize, char)> + '_ {
    let mut prev = None;
    text.chars().enumerate().filter_map(move |(i, c)| {
        let out = canonical_char(prev, c).map(|c| (i, c));
        prev = Some(c);
        out
    })
}

/// A change of the text: `old[start..end]` becomes `with`.
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub with: &'static str,
}

/// Extra width (negative: less) to give the space at `old[start..end]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Gap {
    pub start: usize,
    pub end: usize,
    pub extra: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub lines: usize,
    pub edits: Vec<Edit>,
    pub gaps: Vec<Gap>,
    /// Byte offset in the edited text just after the line break that ends each line (so one
    /// entry less than `lines`).
    pub ends: Vec<usize>,
}

impl Plan {
    /// Byte offset in the edited text of `old`, a byte offset in the original.
    pub fn map(&self, old: usize) -> usize {
        let mut delta: isize = 0;
        for e in &self.edits {
            if e.end <= old {
                delta += e.with.len() as isize - (e.end - e.start) as isize;
            } else {
                break;
            }
        }
        (old as isize + delta) as usize
    }
}

/// What to lay out.
pub struct Request<'a> {
    /// The paragraph, with soft hyphens where it may be hyphenated.
    pub text: &'a str,
    /// Horizontal position of the byte offset on one unbroken line.
    pub x: &'a dyn Fn(usize) -> f64,
    /// Width of the hyphen added at a hyphenated break.
    pub hyphen_width: f64,
    pub first_width: f64,
    pub width: f64,
    pub hyphen_penalty: f64,
    /// Text wraps around a drop cap or a float: the first `n` lines (the first one as wide as
    /// `first_width` says) are only `w` wide for the others.
    pub head: Option<(usize, f64)>,
}

#[derive(Clone, Copy)]
enum Meta {
    Word,
    Space(usize, usize),
    Hyphen(usize, usize),
}

/// Knuth-Plass breaks for the paragraph. `None` when the text is not fit for it (a forced break
/// inside, no words) and the renderer should keep its own breaking.
pub fn plan(req: &Request) -> Option<Plan> {
    let text = req.text;
    if text.contains(['\n', '\r', LINE_BREAK, GHOST_HYPHEN, '\u{2029}']) || req.width < 1.0 {
        return None;
    }
    let mut items: Vec<Item> = Vec::new();
    let mut meta: Vec<Meta> = Vec::new();
    // words: runs of non-space characters; glue: runs of spaces
    let bytes: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    while i < bytes.len() && bytes[i].1 == ' ' {
        i += 1;
    }
    let mut seen_word = false;
    while i < bytes.len() {
        let wstart = bytes[i].0;
        let mut j = i;
        while j < bytes.len() && bytes[j].1 != ' ' {
            j += 1;
        }
        let wend = bytes.get(j).map_or(text.len(), |b| b.0);
        // pieces of the word between soft hyphens
        let mut piece_start = wstart;
        let mut k = i;
        while k < j {
            if bytes[k].1 == SHY {
                let (a, b) = (bytes[k].0, bytes[k].0 + SHY.len_utf8());
                if a > piece_start && b < wend {
                    items.push(Item::word((req.x)(a) - (req.x)(piece_start), ""));
                    meta.push(Meta::Word);
                    items.push(Item::hyphen(req.hyphen_width, req.hyphen_penalty));
                    meta.push(Meta::Hyphen(a, b));
                    piece_start = b;
                }
            }
            k += 1;
        }
        items.push(Item::word((req.x)(wend) - (req.x)(piece_start), ""));
        meta.push(Meta::Word);
        seen_word = true;
        // the spaces after the word
        let mut s = j;
        while s < bytes.len() && bytes[s].1 == ' ' {
            s += 1;
        }
        if s < bytes.len() && s > j {
            let (a, b) = (bytes[j].0, bytes[s].0);
            let w = ((req.x)(b) - (req.x)(a)).max(0.0);
            items.push(Item::space(w, w / 2.0, w / 3.0));
            meta.push(Meta::Space(a, b));
        }
        i = s;
    }
    if !seen_word {
        return None;
    }
    items.push(Item::no_break());
    items.push(Item::space(0.0, 1.0e7, 0.0));
    items.push(Item::forced_break());
    meta.extend([Meta::Word; 3]);

    let mut widths = vec![req.first_width.max(1.0)];
    if let Some((n, w)) = req.head {
        widths.extend(std::iter::repeat_n(w.max(1.0), n.saturating_sub(1)));
    }
    widths.push(req.width);
    let lines: Vec<Line> = break_paragraph(&items, widths, &Params::default());
    if lines.is_empty() {
        return None;
    }
    let mut edits = Vec::new();
    let mut gaps = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        let widths = line.glue_widths(&items);
        let mut w = widths.iter();
        for idx in line.start..line.end.min(items.len()) {
            if let (Item::Glue { width, .. }, Meta::Space(a, b)) = (&items[idx], meta[idx]) {
                let actual = *w.next().unwrap_or(width);
                if (actual - width).abs() > 0.01 {
                    gaps.push(Gap {
                        start: a,
                        end: b,
                        extra: actual - width,
                    });
                }
            }
        }
        if n + 1 == lines.len() {
            break;
        }
        match meta.get(line.end) {
            Some(Meta::Space(a, b)) => edits.push(Edit {
                start: *a,
                end: *b,
                with: "\u{2028}",
            }),
            Some(Meta::Hyphen(a, b)) => edits.push(Edit {
                start: *a,
                end: *b,
                with: "\u{2010}\u{2028}",
            }),
            _ => return None,
        }
    }
    edits.sort_by_key(|e| e.start);
    let mut plan = Plan {
        lines: lines.len(),
        edits,
        gaps,
        ends: Vec::new(),
    };
    plan.ends = plan
        .edits
        .iter()
        .map(|e| plan.map(e.start) + e.with.len())
        .collect();
    Some(plan)
}

/// Applies `edits` to `parts` (the pieces of one text, in order). An edit that spans several
/// parts is put into the first and removes its bytes from the others.
pub fn apply_edits(parts: &mut [String], edits: &[Edit]) {
    let mut starts = Vec::with_capacity(parts.len());
    let mut at = 0;
    for p in parts.iter() {
        starts.push(at);
        at += p.len();
    }
    for e in edits.iter().rev() {
        let mut first = true;
        for (p, &ps) in parts.iter_mut().zip(&starts) {
            let pe = ps + p.len();
            if e.end <= ps || e.start >= pe {
                continue;
            }
            let a = e.start.max(ps) - ps;
            let b = e.end.min(pe) - ps;
            p.replace_range(a..b, if first { e.with } else { "" });
            first = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monospace: 10 per character, soft hyphens have no width.
    fn mono(text: &str) -> impl Fn(usize) -> f64 + '_ {
        move |b| {
            text[..b.min(text.len())]
                .chars()
                .filter(|c| *c != SHY)
                .count() as f64
                * 10.0
        }
    }

    fn run(text: &str, first: f64, width: f64) -> Option<Plan> {
        let x = mono(text);
        plan(&Request {
            text,
            x: &x,
            hyphen_width: 10.0,
            first_width: first,
            width,
            hyphen_penalty: 50.0,
            head: None,
        })
    }

    fn lay_out(text: &str, p: &Plan) -> Vec<String> {
        let mut parts = vec![text.to_string()];
        apply_edits(&mut parts, &p.edits);
        parts[0]
            .split(LINE_BREAK)
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    }

    const T: &str = "the quick brown fox jumps over the lazy dog and keeps on running far away";

    #[test]
    fn every_line_fits_and_no_word_is_lost() {
        let p = run(T, 200.0, 200.0).unwrap();
        let lines = lay_out(T, &p);
        assert_eq!(lines.len(), p.lines);
        assert!(p.lines > 2);
        for l in &lines {
            assert!(
                l.trim().chars().count() as f64 * 10.0 <= 200.0 + 1e-6,
                "{l:?}"
            );
        }
        let joined: Vec<&str> = lines.iter().flat_map(|l| l.split_whitespace()).collect();
        assert_eq!(joined.join(" "), T);
    }

    #[test]
    fn justified_lines_reach_the_full_width() {
        let p = run(T, 200.0, 200.0).unwrap();
        let lines = lay_out(T, &p);
        let x = mono(T);
        for (n, l) in lines.iter().enumerate().take(lines.len() - 1) {
            // extra width of the gaps inside this line
            let start = T.find(l.trim()).unwrap();
            let end = start + l.trim().len();
            let extra: f64 = p
                .gaps
                .iter()
                .filter(|g| g.start >= start && g.end <= end)
                .map(|g| g.extra)
                .sum();
            let natural = x(end) - x(start);
            assert!((natural + extra - 200.0).abs() < 1e-6, "line {n}: {l:?}");
        }
    }

    #[test]
    fn the_first_line_may_be_narrower() {
        let p = run(T, 100.0, 200.0).unwrap();
        let lines = lay_out(T, &p);
        assert!(lines[0].trim().chars().count() as f64 * 10.0 <= 100.0);
    }

    #[test]
    fn hyphenated_breaks_get_a_ghost_hyphen() {
        let text = "aaa extra\u{ad}ordin\u{ad}ary bbb";
        let p = run(text, 90.0, 90.0).unwrap();
        let mut parts = vec![text.to_string()];
        apply_edits(&mut parts, &p.edits);
        let out = &parts[0];
        if out.contains(GHOST_HYPHEN) {
            assert!(out.contains("\u{2010}\u{2028}"));
        }
        let canon: String = canonical_chars(out).map(|(_, c)| c).collect();
        assert_eq!(canon, "aaa extraordinary bbb");
    }

    #[test]
    fn canonical_text_is_unchanged_by_the_edits() {
        for w in [80.0, 120.0, 160.0, 250.0] {
            let p = run(T, w, w).unwrap();
            let mut parts = vec![T.to_string()];
            apply_edits(&mut parts, &p.edits);
            let canon: String = canonical_chars(&parts[0]).map(|(_, c)| c).collect();
            assert_eq!(canon, T);
        }
    }

    #[test]
    fn character_count_of_canonical_text_is_stable() {
        let text = "ab\u{ad}cd ef gh ij kl";
        let before = canonical_chars(text).count();
        let p = run(text, 50.0, 50.0).unwrap();
        let mut parts = vec![text.to_string()];
        apply_edits(&mut parts, &p.edits);
        assert_eq!(canonical_chars(&parts[0]).count(), before);
    }

    #[test]
    fn short_paragraphs_have_one_line_and_no_edits() {
        let p = run("hello world", 500.0, 500.0).unwrap();
        assert_eq!(p.lines, 1);
        assert!(p.edits.is_empty());
        assert!(p.gaps.is_empty());
    }

    #[test]
    fn a_head_of_narrow_lines_wraps_around_a_drop_cap() {
        let x = mono(T);
        let p = plan(&Request {
            text: T,
            x: &x,
            hyphen_width: 10.0,
            first_width: 120.0,
            width: 240.0,
            hyphen_penalty: 50.0,
            head: Some((3, 120.0)),
        })
        .unwrap();
        let lines = lay_out(T, &p);
        for l in &lines[..3] {
            assert!(l.trim().chars().count() as f64 * 10.0 <= 120.0, "{l:?}");
        }
        assert!(lines[3..lines.len() - 1]
            .iter()
            .any(|l| l.trim().chars().count() as f64 * 10.0 > 120.0));
        // the break after the third line is where the tail starts
        let mut parts = vec![T.to_string()];
        apply_edits(&mut parts, &p.edits);
        let tail = &parts[0][p.ends[2]..];
        assert!(tail.starts_with(lines[3].trim_start()));
    }

    #[test]
    fn unfit_text_is_refused() {
        assert!(run("", 100.0, 100.0).is_none());
        assert!(run("   ", 100.0, 100.0).is_none());
        assert!(run("a\nb", 100.0, 100.0).is_none());
        assert!(run("a b", 0.0, 0.0).is_none());
    }

    #[test]
    fn edits_across_parts() {
        let mut parts = vec!["ab ".to_string(), " cd".to_string()];
        apply_edits(
            &mut parts,
            &[Edit {
                start: 2,
                end: 4,
                with: "\u{2028}",
            }],
        );
        assert_eq!(parts, vec!["ab\u{2028}".to_string(), "cd".to_string()]);
    }

    #[test]
    fn offsets_map_through_the_edits() {
        let p = Plan {
            lines: 2,
            edits: vec![Edit {
                start: 2,
                end: 3,
                with: "\u{2028}",
            }],
            gaps: vec![],
            ends: vec![],
        };
        assert_eq!(p.map(1), 1);
        assert_eq!(p.map(3), 5);
        assert_eq!(p.map(10), 12);
    }

    #[test]
    fn markers_are_ghosts_or_spaces() {
        assert!(is_ghost(None, SHY));
        assert!(is_ghost(None, GHOST_HYPHEN));
        assert!(is_ghost(Some(GHOST_HYPHEN), LINE_BREAK));
        assert!(!is_ghost(Some('a'), LINE_BREAK));
        assert_eq!(canonical_char(Some('a'), LINE_BREAK), Some(' '));
        assert_eq!(sanitize("a\u{2010}b\u{2028}c"), "a-b c");
    }
}
