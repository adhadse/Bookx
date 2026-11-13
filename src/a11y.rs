//! Accessibility logic without GTK: the text of the visible page as a screen reader sees it
//! (granularities for `GtkAccessibleText`), keyboard selection movement, contrast maths and the
//! high-contrast palette, and the audit rule for controls without an accessible name.

use crate::ui::canvas::Palette;

/// The text units a screen reader asks for (the subset of `GtkAccessibleTextGranularity` we support).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Char,
    Word,
    Sentence,
    Line,
    Paragraph,
}

/// The visible page: paragraphs separated by `\n`, and where each visual line starts (in chars).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageText {
    pub text: String,
    pub line_starts: Vec<usize>,
    /// Where each paragraph is in the chapter: (first char in the chapter, offset in `text`, length).
    pub segments: Vec<(i32, usize, usize)>,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '\'' || c == '’'
}

fn is_closing(c: char) -> bool {
    matches!(
        c,
        '"' | '\'' | '”' | '’' | ')' | ']' | '»' | '」' | '』' | '）' | '】' | '〕' | '〉' | '》'
    )
}

impl PageText {
    pub fn from_paragraphs(paras: &[(String, Vec<usize>, i32)]) -> PageText {
        let mut text = String::new();
        let mut line_starts = Vec::new();
        let mut segments = Vec::new();
        let mut base = 0usize;
        for (i, (p, lines, global)) in paras.iter().enumerate() {
            if i > 0 {
                text.push('\n');
                base += 1;
            }
            for l in lines {
                line_starts.push(base + l);
            }
            if lines.is_empty() {
                line_starts.push(base);
            }
            text.push_str(p);
            segments.push((*global, base, p.chars().count()));
            base += p.chars().count();
        }
        PageText {
            text,
            line_starts,
            segments,
        }
    }

    /// The chapter char (offset in the chapter) of the page text offset `off`.
    pub fn chapter_char(&self, off: usize) -> Option<i32> {
        self.segments
            .iter()
            .find(|(_, o, len)| off >= *o && off <= o + len)
            .map(|(g, o, _)| g + (off - o) as i32)
    }

    /// The page text offset of a chapter char, if it is on this page.
    pub fn page_offset(&self, chapter_char: i32) -> Option<usize> {
        self.segments
            .iter()
            .find(|(g, _, len)| chapter_char >= *g && chapter_char <= g + *len as i32)
            .map(|(g, o, _)| o + (chapter_char - g) as usize)
    }

    /// The part of a chapter selection `[a, b)` (chars of the chapter) that is on this page, as offsets in `text`.
    pub fn selection_on_page(&self, a: i32, b: i32) -> Option<(usize, usize)> {
        let mut out: Option<(usize, usize)> = None;
        for (g, off, len) in &self.segments {
            let lo = a.max(*g);
            let hi = b.min(*g + *len as i32);
            if hi > lo {
                let s = off + (lo - g) as usize;
                let e = off + (hi - g) as usize;
                out = Some(match out {
                    Some((x, _)) => (x, e),
                    None => (s, e),
                });
            }
        }
        out
    }

    pub fn len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The characters `[start, end)` as a string (clamped).
    pub fn slice(&self, start: usize, end: usize) -> String {
        self.text
            .chars()
            .skip(start)
            .take(end.saturating_sub(start))
            .collect()
    }

    /// The `[start, end)` range of the unit that contains `offset`; `None` past the end.
    pub fn range_at(&self, offset: usize, unit: Unit) -> Option<(usize, usize)> {
        let chars: Vec<char> = self.text.chars().collect();
        let n = chars.len();
        if offset >= n {
            return None;
        }
        Some(match unit {
            Unit::Char => (offset, offset + 1),
            Unit::Word => {
                let w = is_word(chars[offset]);
                let mut s = offset;
                while s > 0 && is_word(chars[s - 1]) == w && chars[s - 1] != '\n' {
                    s -= 1;
                }
                let mut e = offset;
                while e < n && is_word(chars[e]) == w && chars[e] != '\n' {
                    e += 1;
                }
                if e == s {
                    e = s + 1;
                }
                (s, e)
            }
            Unit::Paragraph => {
                let s = chars[..offset]
                    .iter()
                    .rposition(|c| *c == '\n')
                    .map_or(0, |i| i + 1);
                let e = chars[offset..]
                    .iter()
                    .position(|c| *c == '\n')
                    .map_or(n, |i| offset + i + 1);
                (s, e)
            }
            Unit::Line => {
                let starts = &self.line_starts;
                if starts.is_empty() {
                    return self.range_at(offset, Unit::Paragraph);
                }
                let i = starts.partition_point(|s| *s <= offset).saturating_sub(1);
                let e = starts.get(i + 1).copied().unwrap_or(n);
                (starts[i], e)
            }
            Unit::Sentence => {
                let bounds = sentence_starts(&chars);
                let i = bounds.partition_point(|s| *s <= offset).saturating_sub(1);
                let e = bounds.get(i + 1).copied().unwrap_or(n);
                (bounds[i], e)
            }
        })
    }
}

/// Offsets at which a sentence starts (always includes 0).
fn sentence_starts(chars: &[char]) -> Vec<usize> {
    let mut out = vec![0];
    let n = chars.len();
    let mut i = 0;
    while i < n {
        let c = chars[i];
        if c == '\n' {
            out.push(i + 1);
        } else if matches!(c, '.' | '!' | '?' | '…' | '。' | '！' | '？') {
            let mut j = i + 1;
            while j < n && (matches!(chars[j], '.' | '!' | '?' | '…') || is_closing(chars[j])) {
                j += 1;
            }
            if j >= n || chars[j].is_whitespace() || matches!(c, '。' | '！' | '？') {
                while j < n && chars[j].is_whitespace() && chars[j] != '\n' {
                    j += 1;
                }
                if j < n && chars[j] != '\n' {
                    out.push(j);
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out.retain(|s| *s < n);
    out.dedup();
    out
}

/// What a keyboard selection moves by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Char,
    Word,
    Sentence,
}

/// The new end of a keyboard selection (`focus` moved one `step` forward or back), in chars of `text`.
pub fn move_focus(text: &str, focus: usize, step: Step, forward: bool) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let focus = focus.min(n);
    match step {
        Step::Char => {
            if forward {
                (focus + 1).min(n)
            } else {
                focus.saturating_sub(1)
            }
        }
        Step::Word => {
            if forward {
                let mut i = focus;
                while i < n && !is_word(chars[i]) {
                    i += 1;
                }
                while i < n && is_word(chars[i]) {
                    i += 1;
                }
                i
            } else {
                let mut i = focus;
                while i > 0 && !is_word(chars[i - 1]) {
                    i -= 1;
                }
                while i > 0 && is_word(chars[i - 1]) {
                    i -= 1;
                }
                i
            }
        }
        Step::Sentence => {
            let starts = sentence_starts(&chars);
            if forward {
                starts.into_iter().find(|s| *s > focus).unwrap_or(n)
            } else {
                starts.into_iter().rev().find(|s| *s < focus).unwrap_or(0)
            }
        }
    }
}

// ---- contrast ---------------------------------------------------------------------------------

#[cfg(test)]
fn lin(c: f64) -> f64 {
    if c <= 0.03928 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG relative luminance of an sRGB colour (0..1 per channel).
#[cfg(test)]
pub fn luminance(c: (f64, f64, f64)) -> f64 {
    0.2126 * lin(c.0) + 0.7152 * lin(c.1) + 0.0722 * lin(c.2)
}

/// WCAG contrast ratio, 1 (same colours) to 21 (black on white).
#[cfg(test)]
pub fn contrast_ratio(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// The reader palette while the system asks for high contrast: black and white only.
pub fn high_contrast_palette(dark: bool) -> Palette {
    let (bg, fg) = if dark {
        ((0.0, 0.0, 0.0), (1.0, 1.0, 1.0))
    } else {
        ((1.0, 1.0, 1.0), (0.0, 0.0, 0.0))
    };
    Palette {
        bg,
        fg,
        link: fg,
        muted: fg,
        accent: fg,
    }
}

// ---- audit ------------------------------------------------------------------------------------

/// One control found by the accessibility audit (`app:a11y-audit`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Control {
    pub kind: &'static str,
    pub path: String,
    /// The accessible label, the visible label or the tooltip: whatever a screen reader would say.
    pub name: String,
    pub icon_only: bool,
    pub tooltip: bool,
}

/// What is wrong with a control, if anything.
pub fn control_problem(c: &Control) -> Option<&'static str> {
    if c.name.trim().is_empty() {
        Some("no accessible name")
    } else if c.icon_only && !c.tooltip {
        Some("icon-only without a tooltip")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> PageText {
        PageText::from_paragraphs(&[
            ("Hello world. This is it!".into(), vec![0, 13], 100),
            ("Second paragraph.".into(), vec![0], 125),
        ])
    }

    #[test]
    fn page_text_joins_paragraphs_and_maps_lines() {
        let p = page();
        assert_eq!(p.text, "Hello world. This is it!\nSecond paragraph.");
        assert_eq!(p.line_starts, vec![0, 13, 25]);
        assert_eq!(p.len(), 42);
        assert_eq!(p.slice(6, 11), "world");
        assert_eq!(p.slice(32, 999), "paragraph.");
    }

    #[test]
    fn a_chapter_selection_is_mapped_onto_the_page() {
        let p = page();
        // chars 106..111 of the chapter are "world" of the first paragraph
        assert_eq!(p.selection_on_page(106, 111), Some((6, 11)));
        // a selection from the first into the second paragraph
        assert_eq!(p.selection_on_page(120, 130), Some((20, 30)));
        // a selection that is on another page
        assert_eq!(p.selection_on_page(0, 50), None);
        assert_eq!(p.selection_on_page(500, 600), None);
        // it may start before the page and end on it
        assert_eq!(p.selection_on_page(0, 105), Some((0, 5)));
    }

    #[test]
    fn page_offsets_and_chapter_chars_convert_both_ways() {
        let p = page();
        assert_eq!(p.chapter_char(6), Some(106));
        assert_eq!(p.chapter_char(30), Some(130));
        assert_eq!(p.page_offset(106), Some(6));
        assert_eq!(p.page_offset(130), Some(30));
        assert_eq!(p.page_offset(50), None);
        for off in [0usize, 5, 24, 25, 41] {
            let c = p.chapter_char(off).unwrap();
            assert_eq!(p.page_offset(c), Some(off), "offset {off}");
        }
    }

    #[test]
    fn granularities() {
        let p = page();
        assert_eq!(p.range_at(0, Unit::Char), Some((0, 1)));
        assert_eq!(p.range_at(7, Unit::Word), Some((6, 11)));
        assert_eq!(p.range_at(5, Unit::Word), Some((5, 6)), "the space");
        assert_eq!(p.range_at(14, Unit::Line), Some((13, 25)));
        assert_eq!(p.range_at(2, Unit::Line), Some((0, 13)));
        assert_eq!(p.range_at(30, Unit::Line), Some((25, 42)));
        assert_eq!(p.range_at(3, Unit::Paragraph), Some((0, 25)));
        assert_eq!(p.range_at(30, Unit::Paragraph), Some((25, 42)));
        assert_eq!(p.range_at(3, Unit::Sentence), Some((0, 13)));
        assert_eq!(p.range_at(15, Unit::Sentence), Some((13, 25)));
        assert_eq!(p.range_at(26, Unit::Sentence), Some((25, 42)));
        assert_eq!(p.range_at(42, Unit::Char), None);
        assert_eq!(PageText::default().range_at(0, Unit::Word), None);
    }

    #[test]
    fn sentences_keep_abbreviations_without_space_and_quotes() {
        let p = PageText::from_paragraphs(&[(
            "He said, “Go.” Then 3.5 apples. Done".into(),
            vec![],
            0,
        )]);
        let s = |o| p.range_at(o, Unit::Sentence).unwrap();
        assert_eq!(s(2), (0, 15));
        assert_eq!(s(16), (15, 32), "3.5 does not end a sentence");
        assert_eq!(s(34), (32, 36));
    }

    #[test]
    fn a_japanese_sentence_takes_its_closing_bracket_along() {
        // 「今日は晴れです。」と言った。 : the bracket after the full stop belongs to the first sentence
        let p = PageText::from_paragraphs(&[(
            "「今日は晴れです。」と言った。次の文。".into(),
            vec![],
            0,
        )]);
        let s = |o| p.range_at(o, Unit::Sentence).unwrap();
        assert_eq!(s(2), (0, 10));
        assert_eq!(s(11), (10, 15));
        assert_eq!(s(16), (15, 19));
    }

    #[test]
    fn keyboard_selection_moves_by_unit() {
        let t = "One two. Three four.";
        assert_eq!(move_focus(t, 0, Step::Char, true), 1);
        assert_eq!(move_focus(t, 0, Step::Char, false), 0);
        assert_eq!(move_focus(t, 20, Step::Char, true), 20);
        assert_eq!(move_focus(t, 0, Step::Word, true), 3);
        assert_eq!(move_focus(t, 3, Step::Word, true), 7);
        assert_eq!(move_focus(t, 7, Step::Word, false), 4);
        assert_eq!(move_focus(t, 3, Step::Word, false), 0);
        assert_eq!(move_focus(t, 2, Step::Sentence, true), 9);
        assert_eq!(move_focus(t, 15, Step::Sentence, false), 9);
        assert_eq!(move_focus(t, 15, Step::Sentence, true), 20);
        assert_eq!(move_focus(t, 5, Step::Sentence, false), 0);
    }

    #[test]
    fn contrast_maths() {
        let b = (0.0, 0.0, 0.0);
        let w = (1.0, 1.0, 1.0);
        assert!((contrast_ratio(b, w) - 21.0).abs() < 1e-9);
        assert!((contrast_ratio(w, b) - 21.0).abs() < 1e-9);
        assert!((contrast_ratio(w, w) - 1.0).abs() < 1e-9);
        for dark in [false, true] {
            let p = high_contrast_palette(dark);
            assert!(contrast_ratio(p.fg, p.bg) >= 21.0 - 1e-9);
            assert!(contrast_ratio(p.link, p.bg) >= 7.0);
        }
    }

    #[test]
    fn every_reader_theme_is_readable() {
        use crate::config::{ReaderPrefs, ReaderTheme};
        for theme in ReaderTheme::ALL {
            for dark in [false, true] {
                let p = ReaderPrefs {
                    theme,
                    ..Default::default()
                }
                .canvas_style(dark)
                .palette;
                let text = contrast_ratio(p.fg, p.bg);
                assert!(
                    text >= 7.0,
                    "{theme:?} dark={dark}: text contrast {text:.1}"
                );
                let link = contrast_ratio(p.link, p.bg);
                assert!(
                    link >= 3.0,
                    "{theme:?} dark={dark}: link contrast {link:.1}"
                );
                let muted = contrast_ratio(p.muted, p.bg);
                assert!(
                    muted >= 3.0,
                    "{theme:?} dark={dark}: muted contrast {muted:.1}"
                );
            }
        }
    }

    #[test]
    fn audit_rule() {
        let c = |name: &str, icon_only, tooltip| Control {
            kind: "button",
            path: "x".into(),
            name: name.into(),
            icon_only,
            tooltip,
        };
        assert_eq!(
            control_problem(&c("", false, false)),
            Some("no accessible name")
        );
        assert_eq!(
            control_problem(&c("  ", false, true)),
            Some("no accessible name")
        );
        assert_eq!(
            control_problem(&c("Open", true, false)),
            Some("icon-only without a tooltip")
        );
        assert_eq!(control_problem(&c("Open", true, true)), None);
        assert_eq!(control_problem(&c("Open", false, false)), None);
    }
}
