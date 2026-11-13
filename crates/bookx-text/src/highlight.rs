//! Highlight model (R7, R29, R30): styles, colours, ranges, overlap resolution and selection adjustment.
//!
//! Offsets are `char` indices into the text of a chapter (not bytes), so they survive re-layout.

use serde::de::{self, Deserializer};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

/// How a highlight is painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HighlightStyle {
    #[default]
    Background,
    Underline,
    Strikethrough,
    Squiggly,
    Invert,
}

impl HighlightStyle {
    pub const ALL: [HighlightStyle; 5] = [
        HighlightStyle::Background,
        HighlightStyle::Underline,
        HighlightStyle::Strikethrough,
        HighlightStyle::Squiggly,
        HighlightStyle::Invert,
    ];

    pub fn name(self) -> &'static str {
        match self {
            HighlightStyle::Background => "background",
            HighlightStyle::Underline => "underline",
            HighlightStyle::Strikethrough => "strikethrough",
            HighlightStyle::Squiggly => "squiggly",
            HighlightStyle::Invert => "invert",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|v| v.name().eq_ignore_ascii_case(s.trim()))
    }
}

/// Highlight colour: one of the palette or any custom RGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HighlightColor {
    #[default]
    Yellow,
    Orange,
    Red,
    Pink,
    Purple,
    Blue,
    Green,
    Gray,
    Custom(u8, u8, u8),
}

impl HighlightColor {
    pub const PALETTE: [HighlightColor; 8] = [
        HighlightColor::Yellow,
        HighlightColor::Orange,
        HighlightColor::Red,
        HighlightColor::Pink,
        HighlightColor::Purple,
        HighlightColor::Blue,
        HighlightColor::Green,
        HighlightColor::Gray,
    ];

    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            HighlightColor::Yellow => (0xff, 0xd9, 0x1f),
            HighlightColor::Orange => (0xff, 0x92, 0x2e),
            HighlightColor::Red => (0xff, 0x61, 0x52),
            HighlightColor::Pink => (0xf7, 0x6f, 0xb7),
            HighlightColor::Purple => (0xb8, 0x73, 0xf2),
            HighlightColor::Blue => (0x40, 0x8c, 0xff),
            HighlightColor::Green => (0x33, 0xcc, 0x6b),
            HighlightColor::Gray => (0x9a, 0x9a, 0x9a),
            HighlightColor::Custom(r, g, b) => (r, g, b),
        }
    }

    /// Components in 0..=1 for drawing.
    pub fn rgb_f64(self) -> (f64, f64, f64) {
        let (r, g, b) = self.rgb();
        (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0)
    }

    /// Lower-case `#rrggbb`.
    pub fn hex(self) -> String {
        let (r, g, b) = self.rgb();
        format!("#{r:02x}{g:02x}{b:02x}")
    }

    /// Palette name, or `None` for custom colours.
    pub fn name(self) -> Option<&'static str> {
        Some(match self {
            HighlightColor::Yellow => "yellow",
            HighlightColor::Orange => "orange",
            HighlightColor::Red => "red",
            HighlightColor::Pink => "pink",
            HighlightColor::Purple => "purple",
            HighlightColor::Blue => "blue",
            HighlightColor::Green => "green",
            HighlightColor::Gray => "gray",
            HighlightColor::Custom(..) => return None,
        })
    }

    /// Parse a palette name (`yellow`), `#rgb` or `#rrggbb` (case-insensitive). A hex value equal to a
    /// palette colour becomes that palette entry.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if let Some(c) = Self::PALETTE
            .into_iter()
            .find(|c| c.name().is_some_and(|n| n.eq_ignore_ascii_case(s)))
        {
            return Some(c);
        }
        if s.eq_ignore_ascii_case("grey") {
            return Some(HighlightColor::Gray);
        }
        let hex = s.strip_prefix('#')?;
        if !hex.is_ascii() {
            return None;
        }
        let (r, g, b) = match hex.len() {
            3 => {
                let v: Vec<u8> = hex
                    .chars()
                    .map(|c| c.to_digit(16).map(|d| (d * 17) as u8))
                    .collect::<Option<_>>()?;
                (v[0], v[1], v[2])
            }
            6 => (
                u8::from_str_radix(&hex[0..2], 16).ok()?,
                u8::from_str_radix(&hex[2..4], 16).ok()?,
                u8::from_str_radix(&hex[4..6], 16).ok()?,
            ),
            _ => return None,
        };
        Some(
            Self::PALETTE
                .into_iter()
                .find(|c| c.rgb() == (r, g, b))
                .unwrap_or(HighlightColor::Custom(r, g, b)),
        )
    }
}

impl Serialize for HighlightColor {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.hex())
    }
}

impl<'de> Deserialize<'de> for HighlightColor {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        HighlightColor::parse(&s)
            .ok_or_else(|| de::Error::custom(format!("invalid highlight colour {s:?}")))
    }
}

/// A half-open range `[start, end)` of char offsets. Always normalised: `start <= end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Range {
    pub start: usize,
    pub end: usize,
}

impl Range {
    /// Build a range from two offsets in any order (a backwards selection is swapped).
    pub fn new(a: usize, b: usize) -> Self {
        Range {
            start: a.min(b),
            end: a.max(b),
        }
    }

    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    pub fn contains(&self, offset: usize) -> bool {
        offset >= self.start && offset < self.end
    }

    /// Share at least one character.
    pub fn overlaps(&self, other: &Range) -> bool {
        self.start < other.end && other.start < self.end
    }

    /// Overlap or touch (`[0,3)` and `[3,5)`).
    pub fn touches(&self, other: &Range) -> bool {
        self.start <= other.end && other.start <= self.end
    }

    pub fn intersection(&self, other: &Range) -> Option<Range> {
        let (s, e) = (self.start.max(other.start), self.end.min(other.end));
        (s < e).then_some(Range { start: s, end: e })
    }

    /// Smallest range covering both.
    pub fn hull(&self, other: &Range) -> Range {
        Range {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    /// Clamp into `0..=len`.
    pub fn clamped(&self, len: usize) -> Range {
        Range {
            start: self.start.min(len),
            end: self.end.min(len),
        }
    }

    /// The covered text.
    pub fn slice(&self, text: &str) -> String {
        text.chars().skip(self.start).take(self.len()).collect()
    }
}

/// A stored highlight.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Highlight {
    pub id: String,
    pub range: Range,
    #[serde(default)]
    pub style: HighlightStyle,
    #[serde(default)]
    pub color: HighlightColor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Highlight {
    pub fn new(
        id: impl Into<String>,
        range: Range,
        style: HighlightStyle,
        color: HighlightColor,
    ) -> Self {
        Highlight {
            id: id.into(),
            range,
            style,
            color,
            note: None,
        }
    }
}

/// A paint segment: a sub-range and the highlights covering it, bottom to top (creation order).
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub range: Range,
    pub ids: Vec<String>,
}

/// The highlights of one chapter or book.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HighlightSet {
    pub highlights: Vec<Highlight>,
}

impl HighlightSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a highlight (overlaps are allowed). Empty ranges are ignored; returns whether it was added.
    pub fn add(&mut self, h: Highlight) -> bool {
        if h.range.is_empty() {
            return false;
        }
        self.highlights.push(h);
        true
    }

    pub fn get(&self, id: &str) -> Option<&Highlight> {
        self.highlights.iter().find(|h| h.id == id)
    }

    pub fn remove(&mut self, id: &str) -> Option<Highlight> {
        let i = self.highlights.iter().position(|h| h.id == id)?;
        Some(self.highlights.remove(i))
    }

    /// Replace the range of a highlight (used after adjusting its edges). False if empty or unknown.
    pub fn set_range(&mut self, id: &str, range: Range) -> bool {
        if range.is_empty() {
            return false;
        }
        match self.highlights.iter_mut().find(|h| h.id == id) {
            Some(h) => {
                h.range = range;
                true
            }
            None => false,
        }
    }

    /// All highlights covering the char at `offset`, in creation order (the last one is painted on top).
    /// This is what an "overlapping highlights picker" shows.
    pub fn overlapping_at(&self, offset: usize) -> Vec<&Highlight> {
        self.highlights
            .iter()
            .filter(|h| h.range.contains(offset))
            .collect()
    }

    /// Highlights intersecting a range.
    pub fn overlapping_range(&self, range: Range) -> Vec<&Highlight> {
        self.highlights
            .iter()
            .filter(|h| h.range.overlaps(&range))
            .collect()
    }

    /// Merge highlights with the same style and colour that overlap or touch. The first one's id and note
    /// survive (notes are joined with a newline when several exist). Returns the number of merges.
    pub fn merge_same_style(&mut self) -> usize {
        let mut merged = 0;
        let mut i = 0;
        while i < self.highlights.len() {
            let mut j = i + 1;
            while j < self.highlights.len() {
                let (a, b) = (&self.highlights[i], &self.highlights[j]);
                if a.style == b.style && a.color == b.color && a.range.touches(&b.range) {
                    let b = self.highlights.remove(j);
                    let a = &mut self.highlights[i];
                    a.range = a.range.hull(&b.range);
                    a.note = match (a.note.take(), b.note) {
                        (Some(x), Some(y)) if x != y => Some(format!("{x}\n{y}")),
                        (x, y) => x.or(y),
                    };
                    merged += 1;
                    j = i + 1;
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
        merged
    }

    /// Split into non-overlapping paint segments sorted by position. Each lists the covering highlight ids
    /// bottom to top. Uncovered gaps are omitted.
    pub fn segments(&self) -> Vec<Segment> {
        let mut cuts: Vec<usize> = self
            .highlights
            .iter()
            .flat_map(|h| [h.range.start, h.range.end])
            .collect();
        cuts.sort_unstable();
        cuts.dedup();
        let mut out: Vec<Segment> = Vec::new();
        for w in cuts.windows(2) {
            let r = Range {
                start: w[0],
                end: w[1],
            };
            let ids: Vec<String> = self
                .highlights
                .iter()
                .filter(|h| h.range.start <= r.start && h.range.end >= r.end)
                .map(|h| h.id.clone())
                .collect();
            if ids.is_empty() {
                continue;
            }
            match out.last_mut() {
                Some(last) if last.range.end == r.start && last.ids == ids => {
                    last.range.end = r.end
                }
                _ => out.push(Segment { range: r, ids }),
            }
        }
        out
    }

    /// JSON roundtrip helper.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{\"highlights\":[]}".into())
    }

    pub fn from_json(s: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(s)?)
    }
}

/// Which end of a selection moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Start,
    End,
}

/// Step size when adjusting an edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Char,
    Word,
}

fn is_word_char(chars: &[char], i: usize) -> bool {
    let c = chars[i];
    if c.is_alphanumeric() || c == '_' {
        return true;
    }
    // apostrophes and hyphens inside a word (don't, well-known)
    if matches!(c, '\'' | '\u{2019}' | '-' | '\u{2010}') {
        let before = i
            .checked_sub(1)
            .and_then(|j| chars.get(j))
            .is_some_and(|p| p.is_alphanumeric());
        let after = chars.get(i + 1).is_some_and(|n| n.is_alphanumeric());
        return before && after;
    }
    false
}

/// Expand a range outwards so that it starts and ends on word boundaries.
pub fn snap_to_words(text: &str, range: Range) -> Range {
    let chars: Vec<char> = text.chars().collect();
    let r = range.clamped(chars.len());
    if r.is_empty() {
        return r;
    }
    let mut s = r.start;
    while s > 0 && s < chars.len() && is_word_char(&chars, s) && is_word_char(&chars, s - 1) {
        s -= 1;
    }
    let mut e = r.end;
    while e < chars.len() && e > 0 && is_word_char(&chars, e) && is_word_char(&chars, e - 1) {
        e += 1;
    }
    Range { start: s, end: e }
}

/// Move one edge of a range by one unit (`dir` > 0 moves the edge towards the end of the text, `dir` < 0 towards
/// the start). Extending the start leftwards or the end rightwards grows the range; the opposite shrinks it.
/// A range never shrinks below one char and never leaves the text. Returns the new range.
pub fn adjust_edge(text: &str, range: Range, edge: Edge, unit: Unit, dir: i32) -> Range {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut r = range.clamped(n);
    if dir == 0 {
        return r;
    }
    let word = |i: usize| is_word_char(&chars, i);
    let step = |pos: usize| -> usize {
        let mut p = pos;
        match (unit, edge, dir > 0) {
            (Unit::Char, _, true) => p = (p + 1).min(n),
            (Unit::Char, _, false) => p = p.saturating_sub(1),
            // end of the next word
            (Unit::Word, Edge::End, true) => {
                while p < n && !word(p) {
                    p += 1;
                }
                while p < n && word(p) {
                    p += 1;
                }
            }
            // start of the next word
            (Unit::Word, Edge::Start, true) => {
                while p < n && word(p) {
                    p += 1;
                }
                while p < n && !word(p) {
                    p += 1;
                }
            }
            // start of the previous word
            (Unit::Word, Edge::Start, false) => {
                while p > 0 && !word(p - 1) {
                    p -= 1;
                }
                while p > 0 && word(p - 1) {
                    p -= 1;
                }
            }
            // end of the previous word
            (Unit::Word, Edge::End, false) => {
                while p > 0 && word(p - 1) {
                    p -= 1;
                }
                while p > 0 && !word(p - 1) {
                    p -= 1;
                }
            }
        }
        p
    };
    match edge {
        Edge::Start => {
            let ns = step(r.start);
            if dir > 0 {
                // shrinking: keep at least one char
                if r.len() > 1 {
                    r.start = ns.min(r.end - 1);
                }
            } else {
                r.start = ns;
            }
        }
        Edge::End => {
            let ne = step(r.end);
            if dir < 0 {
                if r.len() > 1 {
                    r.end = ne.max(r.start + 1);
                }
            } else {
                r.end = ne;
            }
        }
    }
    r
}

const SENTENCE_END: &[char] = &[
    '.', '!', '?', '\u{2026}', '\u{3002}', '\u{ff01}', '\u{ff1f}',
];
const CLOSERS: &[char] = &[
    '"', '\'', '\u{201d}', '\u{2019}', ')', ']', '}', '\u{bb}', '\u{203a}',
];
const OPENERS: &[char] = &[
    '"', '\'', '\u{201c}', '\u{2018}', '(', '[', '{', '\u{ab}', '\u{2039}', '\u{bf}', '\u{a1}',
];

/// Punctuation-aware extension: when the selection ends right before sentence punctuation that closes a
/// sentence (`.`, `!`, `?`, `…` possibly followed by closing quotes/brackets and then whitespace or the end of the
/// text), include that punctuation. When the selection starts right after an opening quote or bracket that is
/// closed inside the selection, include it too.
pub fn extend_with_punctuation(text: &str, range: Range) -> Range {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut r = range.clamped(n);
    if r.is_empty() {
        return r;
    }
    // trailing punctuation
    let mut e = r.end;
    let last = chars[e - 1];
    if last.is_alphanumeric() || CLOSERS.contains(&last) {
        let mut p = e;
        let mut saw_end = false;
        while p < n && SENTENCE_END.contains(&chars[p]) {
            p += 1;
            saw_end = true;
        }
        if saw_end {
            while p < n && CLOSERS.contains(&chars[p]) {
                p += 1;
            }
            if p == n || chars[p].is_whitespace() {
                e = p;
            }
        }
    }
    r.end = e;
    // an opening quote or bracket directly before the selection
    if r.start > 0 {
        let prev = chars[r.start - 1];
        if OPENERS.contains(&prev) && chars[r.start].is_alphanumeric() {
            let closer = match prev {
                '\u{201c}' => Some('\u{201d}'),
                '\u{2018}' => Some('\u{2019}'),
                '(' => Some(')'),
                '[' => Some(']'),
                '{' => Some('}'),
                '\u{ab}' => Some('\u{bb}'),
                '\u{2039}' => Some('\u{203a}'),
                '"' => Some('"'),
                '\'' => Some('\''),
                _ => None,
            };
            match closer {
                None => r.start -= 1,
                Some(c) => {
                    if chars[r.start..r.end].contains(&c) {
                        r.start -= 1;
                    } else if chars.get(r.end) == Some(&c) {
                        r.start -= 1;
                        r.end += 1;
                    }
                }
            }
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(a: usize, b: usize) -> Range {
        Range::new(a, b)
    }

    fn hl(id: &str, a: usize, b: usize, style: HighlightStyle, color: HighlightColor) -> Highlight {
        Highlight::new(id, r(a, b), style, color)
    }

    fn bg(id: &str, a: usize, b: usize) -> Highlight {
        hl(id, a, b, HighlightStyle::Background, HighlightColor::Yellow)
    }

    #[test]
    fn style_names_and_serde() {
        for s in HighlightStyle::ALL {
            assert_eq!(HighlightStyle::from_name(s.name()), Some(s));
            let j = serde_json::to_string(&s).unwrap();
            assert_eq!(j, format!("\"{}\"", s.name()));
            assert_eq!(serde_json::from_str::<HighlightStyle>(&j).unwrap(), s);
        }
        assert_eq!(
            HighlightStyle::from_name(" Squiggly "),
            Some(HighlightStyle::Squiggly)
        );
        assert_eq!(HighlightStyle::from_name("wavy"), None);
        assert_eq!(HighlightStyle::default(), HighlightStyle::Background);
        assert!(serde_json::from_str::<HighlightStyle>("\"nope\"").is_err());
    }

    #[test]
    fn colour_hex_roundtrip_and_parse() {
        for c in HighlightColor::PALETTE {
            assert_eq!(HighlightColor::parse(&c.hex()), Some(c));
            assert_eq!(HighlightColor::parse(c.name().unwrap()), Some(c));
            let j = serde_json::to_string(&c).unwrap();
            assert_eq!(serde_json::from_str::<HighlightColor>(&j).unwrap(), c);
        }
        assert_eq!(
            HighlightColor::parse("#FFD91F"),
            Some(HighlightColor::Yellow)
        );
        assert_eq!(
            HighlightColor::parse("#123456"),
            Some(HighlightColor::Custom(0x12, 0x34, 0x56))
        );
        assert_eq!(
            HighlightColor::parse("#fa0"),
            Some(HighlightColor::Custom(0xff, 0xaa, 0x00))
        );
        assert_eq!(HighlightColor::parse("GREY"), Some(HighlightColor::Gray));
        assert_eq!(HighlightColor::parse(" Blue "), Some(HighlightColor::Blue));
        for bad in [
            "",
            "#",
            "#12",
            "#12345",
            "#ggg",
            "#1234567",
            "chartreuse",
            "ffd91f",
            "#é1234",
        ] {
            assert_eq!(HighlightColor::parse(bad), None, "{bad:?}");
        }
        assert_eq!(HighlightColor::Custom(1, 2, 3).hex(), "#010203");
        assert_eq!(HighlightColor::Custom(1, 2, 3).name(), None);
        assert_eq!(HighlightColor::default(), HighlightColor::Yellow);
        let (rr, gg, bb) = HighlightColor::Custom(255, 0, 51).rgb_f64();
        assert!((rr - 1.0).abs() < 1e-9 && gg == 0.0 && (bb - 0.2).abs() < 1e-9);
    }

    #[test]
    fn colour_deserialize_error_message() {
        let e = serde_json::from_str::<HighlightColor>("\"mauve\"")
            .unwrap_err()
            .to_string();
        assert!(e.contains("invalid highlight colour \"mauve\""), "{e}");
        assert!(serde_json::from_str::<HighlightColor>("12").is_err());
    }

    #[test]
    fn range_normalisation_and_basics() {
        assert_eq!(Range::new(9, 3), Range { start: 3, end: 9 });
        assert_eq!(r(3, 9).len(), 6);
        assert!(r(4, 4).is_empty());
        assert!(!r(4, 5).is_empty());
        assert!(
            r(3, 9).contains(3)
                && r(3, 9).contains(8)
                && !r(3, 9).contains(9)
                && !r(3, 9).contains(2)
        );
        assert!(!r(4, 4).contains(4));
    }

    #[test]
    fn range_overlap_touch_intersection_hull() {
        assert!(r(0, 5).overlaps(&r(4, 8)));
        assert!(!r(0, 5).overlaps(&r(5, 8)));
        assert!(r(0, 5).touches(&r(5, 8)));
        assert!(!r(0, 4).touches(&r(5, 8)));
        assert_eq!(r(0, 5).intersection(&r(3, 8)), Some(r(3, 5)));
        assert_eq!(r(0, 5).intersection(&r(5, 8)), None);
        assert_eq!(r(0, 5).hull(&r(7, 9)), r(0, 9));
        assert_eq!(r(2, 50).clamped(10), r(2, 10));
        assert_eq!(r(20, 50).clamped(10), r(10, 10));
    }

    #[test]
    fn range_slice_is_char_based() {
        assert_eq!(r(1, 4).slice("aéßdef"), "éßd");
        assert_eq!(r(4, 40).slice("abcdef"), "ef");
        assert_eq!(r(40, 50).slice("abc"), "");
    }

    #[test]
    fn set_add_get_remove_set_range() {
        let mut s = HighlightSet::new();
        assert!(s.add(bg("a", 0, 5)));
        assert!(!s.add(bg("empty", 3, 3)));
        assert!(s.get("a").is_some());
        assert!(s.get("empty").is_none());
        assert!(s.set_range("a", r(2, 9)));
        assert_eq!(s.get("a").unwrap().range, r(2, 9));
        assert!(!s.set_range("a", r(4, 4)));
        assert!(!s.set_range("zz", r(4, 6)));
        assert_eq!(s.remove("a").unwrap().id, "a");
        assert!(s.remove("a").is_none());
        assert!(s.highlights.is_empty());
    }

    #[test]
    fn overlapping_at_returns_creation_order() {
        let mut s = HighlightSet::new();
        s.add(bg("a", 0, 10));
        s.add(hl(
            "b",
            5,
            15,
            HighlightStyle::Underline,
            HighlightColor::Blue,
        ));
        s.add(bg("c", 20, 25));
        let ids = |o: usize| {
            s.overlapping_at(o)
                .iter()
                .map(|h| h.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(0), vec!["a"]);
        assert_eq!(ids(5), vec!["a", "b"]);
        assert_eq!(ids(9), vec!["a", "b"]);
        assert_eq!(ids(10), vec!["b"]);
        assert!(ids(17).is_empty());
        assert_eq!(ids(24), vec!["c"]);
        assert!(ids(25).is_empty());
        assert_eq!(s.overlapping_range(r(8, 21)).len(), 3);
        assert_eq!(s.overlapping_range(r(15, 20)).len(), 0);
    }

    #[test]
    fn merge_same_style_merges_overlapping_and_touching() {
        let mut s = HighlightSet::new();
        s.add(bg("a", 0, 5));
        s.add(bg("b", 5, 8));
        s.add(bg("c", 7, 12));
        s.add(hl(
            "d",
            3,
            4,
            HighlightStyle::Underline,
            HighlightColor::Yellow,
        ));
        s.add(hl(
            "e",
            3,
            4,
            HighlightStyle::Background,
            HighlightColor::Blue,
        ));
        s.add(bg("f", 30, 31));
        let n = s.merge_same_style();
        assert_eq!(n, 2);
        assert_eq!(s.highlights.len(), 4);
        assert_eq!(s.get("a").unwrap().range, r(0, 12));
        assert!(s.get("b").is_none() && s.get("c").is_none());
        assert_eq!(s.get("d").unwrap().range, r(3, 4));
        assert_eq!(s.get("f").unwrap().range, r(30, 31));
        assert_eq!(s.merge_same_style(), 0);
    }

    #[test]
    fn merge_chains_through_later_highlights_and_joins_notes() {
        let mut s = HighlightSet::new();
        let mut a = bg("a", 0, 3);
        a.note = Some("one".into());
        let mut c = bg("c", 6, 9);
        c.note = Some("two".into());
        s.add(a);
        s.add(c);
        s.add(bg("b", 3, 6)); // bridges a and c
        assert_eq!(s.merge_same_style(), 2);
        let a = s.get("a").unwrap();
        assert_eq!(a.range, r(0, 9));
        assert_eq!(a.note.as_deref(), Some("one\ntwo"));
        let mut s = HighlightSet::new();
        let mut x = bg("x", 0, 3);
        x.note = Some("same".into());
        let mut y = bg("y", 2, 4);
        y.note = Some("same".into());
        s.add(x);
        s.add(y);
        s.merge_same_style();
        assert_eq!(s.get("x").unwrap().note.as_deref(), Some("same"));
        let mut s = HighlightSet::new();
        s.add(bg("p", 0, 3));
        let mut q = bg("q", 2, 4);
        q.note = Some("n".into());
        s.add(q);
        s.merge_same_style();
        assert_eq!(s.get("p").unwrap().note.as_deref(), Some("n"));
    }

    #[test]
    fn segments_split_on_boundaries() {
        let mut s = HighlightSet::new();
        s.add(bg("a", 0, 10));
        s.add(hl(
            "b",
            5,
            15,
            HighlightStyle::Underline,
            HighlightColor::Red,
        ));
        s.add(bg("c", 20, 25));
        let seg = s.segments();
        assert_eq!(
            seg,
            vec![
                Segment {
                    range: r(0, 5),
                    ids: vec!["a".into()]
                },
                Segment {
                    range: r(5, 10),
                    ids: vec!["a".into(), "b".into()]
                },
                Segment {
                    range: r(10, 15),
                    ids: vec!["b".into()]
                },
                Segment {
                    range: r(20, 25),
                    ids: vec!["c".into()]
                },
            ]
        );
        assert!(HighlightSet::new().segments().is_empty());
    }

    #[test]
    fn segments_coalesce_equal_neighbours() {
        let mut s = HighlightSet::new();
        s.add(bg("a", 0, 10));
        s.add(bg("a2", 0, 10));
        // a cut at 5 produced by an unrelated zero-effect boundary (same coverage on both sides)
        s.add(hl("z", 5, 5, HighlightStyle::Invert, HighlightColor::Gray)); // ignored: empty
        let seg = s.segments();
        assert_eq!(seg.len(), 1);
        assert_eq!(seg[0].range, r(0, 10));
        assert_eq!(seg[0].ids, vec!["a", "a2"]);
    }

    #[test]
    fn set_json_roundtrip_and_errors() {
        let mut s = HighlightSet::new();
        let mut h = hl(
            "a",
            1,
            9,
            HighlightStyle::Squiggly,
            HighlightColor::Custom(1, 2, 3),
        );
        h.note = Some("hi".into());
        s.add(h);
        s.add(bg("b", 10, 12));
        let back = HighlightSet::from_json(&s.to_json()).unwrap();
        assert_eq!(back, s);
        assert!(s.to_json().contains("\"#010203\""));
        assert!(!s.to_json().contains("\"note\":null"));
        let golden = r##"{"highlights":[{"id":"x","range":{"start":2,"end":4}}]}"##;
        let g = HighlightSet::from_json(golden).unwrap();
        assert_eq!(g.highlights[0].style, HighlightStyle::Background);
        assert_eq!(g.highlights[0].color, HighlightColor::Yellow);
        assert!(HighlightSet::from_json("not json").is_err());
    }

    #[test]
    fn snap_to_words_expands() {
        let t = "The quick brown fox";
        assert_eq!(snap_to_words(t, r(6, 8)), r(4, 9));
        assert_eq!(snap_to_words(t, r(4, 9)), r(4, 9));
        assert_eq!(snap_to_words(t, r(0, 19)), r(0, 19));
        assert_eq!(snap_to_words(t, r(5, 5)), r(5, 5));
        assert_eq!(snap_to_words(t, r(17, 99)), r(16, 19));
        let t = "don't stop well-known";
        assert_eq!(snap_to_words(t, r(1, 2)), r(0, 5));
        assert_eq!(snap_to_words(t, r(13, 15)), r(11, 21));
        let t = "naïve café";
        assert_eq!(snap_to_words(t, r(2, 3)), r(0, 5));
    }

    #[test]
    fn adjust_edge_chars() {
        let t = "abcdefghij";
        let base = r(3, 6);
        assert_eq!(adjust_edge(t, base, Edge::End, Unit::Char, 1), r(3, 7));
        assert_eq!(adjust_edge(t, base, Edge::End, Unit::Char, -1), r(3, 5));
        assert_eq!(adjust_edge(t, base, Edge::Start, Unit::Char, -1), r(2, 6));
        assert_eq!(adjust_edge(t, base, Edge::Start, Unit::Char, 1), r(4, 6));
        assert_eq!(adjust_edge(t, base, Edge::Start, Unit::Char, 0), base);
    }

    #[test]
    fn adjust_edge_chars_bounds_and_minimum() {
        let t = "abcdefghij";
        assert_eq!(
            adjust_edge(t, r(0, 3), Edge::Start, Unit::Char, -1),
            r(0, 3)
        );
        assert_eq!(adjust_edge(t, r(5, 10), Edge::End, Unit::Char, 1), r(5, 10));
        assert_eq!(adjust_edge(t, r(3, 4), Edge::End, Unit::Char, -1), r(3, 4));
        assert_eq!(adjust_edge(t, r(3, 4), Edge::Start, Unit::Char, 1), r(3, 4));
        assert_eq!(adjust_edge(t, r(8, 99), Edge::End, Unit::Char, -1), r(8, 9));
    }

    #[test]
    fn adjust_edge_words() {
        let t = "one two three four";
        // range = "two" (4..7)
        let base = r(4, 7);
        assert_eq!(adjust_edge(t, base, Edge::End, Unit::Word, 1), r(4, 13));
        assert_eq!(adjust_edge(t, base, Edge::Start, Unit::Word, -1), r(0, 7));
        assert_eq!(adjust_edge(t, r(4, 13), Edge::End, Unit::Word, -1), r(4, 7));
        assert_eq!(
            adjust_edge(t, r(0, 13), Edge::Start, Unit::Word, 1),
            r(4, 13)
        );
        // cannot shrink past the last word: keeps one char
        let w = adjust_edge(t, base, Edge::End, Unit::Word, -1);
        assert!(!w.is_empty() && w.start == 4);
        let w = adjust_edge(t, base, Edge::Start, Unit::Word, 1);
        assert!(!w.is_empty() && w.end == 7);
        // at the text edges
        assert_eq!(
            adjust_edge(t, r(14, 18), Edge::End, Unit::Word, 1),
            r(14, 18)
        );
        assert_eq!(
            adjust_edge(t, r(0, 3), Edge::Start, Unit::Word, -1),
            r(0, 3)
        );
    }

    #[test]
    fn extend_punctuation_trailing_sentence_end() {
        let t = "He left. She stayed! Why? Nobody knew...";
        // "He left" -> includes the full stop
        assert_eq!(extend_with_punctuation(t, r(0, 7)), r(0, 8));
        // "She stayed" -> "!"
        assert_eq!(extend_with_punctuation(t, r(9, 19)), r(9, 20));
        // "Why" -> "?"
        assert_eq!(extend_with_punctuation(t, r(21, 24)), r(21, 25));
        // ellipsis of dots at the end of the text
        assert_eq!(extend_with_punctuation(t, r(26, 37)), r(26, 40));
        // mid-sentence: no change
        assert_eq!(extend_with_punctuation(t, r(3, 7)), r(3, 8));
        assert_eq!(extend_with_punctuation("a b, c", r(0, 3)), r(0, 3));
        assert_eq!(extend_with_punctuation("end", r(0, 3)), r(0, 3));
    }

    #[test]
    fn extend_punctuation_closing_quotes_and_brackets() {
        let t = "He said \u{201c}go home.\u{201d} Then left.";
        // selection "go home" ends before '.' + closing quote + space
        let s = t.chars().position(|c| c == 'g').unwrap();
        let e = s + "go home".chars().count();
        let out = extend_with_punctuation(t, r(s, e));
        assert_eq!(out.slice(t), "\u{201c}go home.\u{201d}");
        let t = "(see above.) Next";
        assert_eq!(
            extend_with_punctuation(t, r(1, 10)).slice(t),
            "(see above.)"
        );
    }

    #[test]
    fn extend_punctuation_abbreviation_inside_word_not_extended() {
        // '.' followed by a letter is not a sentence end (version numbers, domains)
        let t = "see example.com now";
        assert_eq!(extend_with_punctuation(t, r(4, 11)), r(4, 11));
        let t = "pi is 3.14 ok";
        assert_eq!(extend_with_punctuation(t, r(6, 7)), r(6, 7));
    }

    #[test]
    fn extend_punctuation_leading_opener() {
        let t = "He said \u{201c}hello there\u{201d} loudly";
        let s = t.chars().position(|c| c == 'h').unwrap();
        let out = extend_with_punctuation(t, r(s, s + 11));
        assert!(out.slice(t).starts_with('\u{201c}'));
        // opener without closer inside the selection (curly) is not included
        let out = extend_with_punctuation(t, r(s, s + 5));
        assert_eq!(out.start, s);
        // straight quotes have an ambiguous side; included only when it is at a word start
        let t = "say \"hi\" now";
        assert_eq!(extend_with_punctuation(t, r(5, 7)).slice(t), "\"hi\"");
    }

    #[test]
    fn extend_punctuation_empty_and_out_of_range() {
        assert_eq!(extend_with_punctuation("abc.", r(2, 2)), r(2, 2));
        assert_eq!(extend_with_punctuation("abc.", r(0, 100)), r(0, 4));
        assert_eq!(extend_with_punctuation("", r(0, 0)), r(0, 0));
    }

    #[test]
    fn property_random_sets_segments_consistent() {
        let mut seed = 7u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..200 {
            let mut s = HighlightSet::new();
            for i in 0..(rnd() % 8) {
                let a = (rnd() % 40) as usize;
                let b = (rnd() % 40) as usize;
                let style = HighlightStyle::ALL[(rnd() % 5) as usize];
                let color = HighlightColor::PALETTE[(rnd() % 3) as usize];
                s.add(hl(&format!("h{i}"), a, b, style, color));
            }
            let segs = s.segments();
            for w in segs.windows(2) {
                assert!(w[0].range.end <= w[1].range.start);
            }
            for off in 0..45 {
                let direct: Vec<String> =
                    s.overlapping_at(off).iter().map(|h| h.id.clone()).collect();
                let via = segs
                    .iter()
                    .find(|g| g.range.contains(off))
                    .map(|g| g.ids.clone())
                    .unwrap_or_default();
                assert_eq!(direct, via, "offset {off}");
            }
            let before: Vec<bool> = (0..45).map(|o| !s.overlapping_at(o).is_empty()).collect();
            s.merge_same_style();
            let after: Vec<bool> = (0..45).map(|o| !s.overlapping_at(o).is_empty()).collect();
            assert_eq!(before, after, "merging must not change covered characters");
        }
    }

    #[test]
    fn property_adjust_edge_never_invalid() {
        let t = "The quick, brown fox — jumps over; the lazy dog.";
        let n = t.chars().count();
        let mut seed = 5u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..2000 {
            let a = (rnd() as usize) % (n + 3);
            let b = (rnd() as usize) % (n + 3);
            let range = Range::new(a, b);
            let edge = if rnd() % 2 == 0 {
                Edge::Start
            } else {
                Edge::End
            };
            let unit = if rnd() % 2 == 0 {
                Unit::Char
            } else {
                Unit::Word
            };
            let dir = (rnd() % 3) as i32 - 1;
            let out = adjust_edge(t, range, edge, unit, dir);
            assert!(
                out.start <= out.end && out.end <= n,
                "{range:?} {edge:?} {unit:?} {dir} -> {out:?}"
            );
            let snapped = snap_to_words(t, range);
            assert!(snapped.start <= snapped.end && snapped.end <= n);
            let ext = extend_with_punctuation(t, range);
            assert!(ext.start <= ext.end && ext.end <= n);
            if !range.clamped(n).is_empty() {
                assert!(ext.start <= range.clamped(n).start && ext.end >= range.clamped(n).end);
                assert!(
                    snapped.start <= range.clamped(n).start && snapped.end >= range.clamped(n).end
                );
            }
        }
    }
}
