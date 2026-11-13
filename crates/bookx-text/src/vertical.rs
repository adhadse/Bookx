//! Vertical writing (CSS `writing-mode: vertical-rl | vertical-lr`) without a font engine.
//!
//! The reader lays a chapter out in a *logical* frame, as for horizontal text: lines run along the
//! inline axis `u` (their length is the height of the page) and stack along the block axis `v`
//! (the flow). A page is a stretch of the flow. [`Frame`] maps that logical space to the screen:
//! in `vertical-rl` the flow runs from the right edge of the page to the left, in `vertical-lr`
//! from the left to the right, and the inline axis always runs down. Everything that is geometry
//! (page cuts, hit testing, selection rectangles) goes through it, so it is tested here.
//!
//! The rest of the module is the typography of vertical Japanese and Chinese that needs no font:
//! kinsoku (which characters cannot start or end a line), tate-chu-yoko (which characters are
//! combined into one upright cell), where punctuation is squeezed, and how far ruby reaches over
//! its neighbours.

pub use crate::css::{TextCombine, TextOrientation, WritingMode};

/// A page (or the visible part of a scrolled flow) on the screen.
///
/// `x, y, w, h` is the rectangle the text occupies on the screen; the stretch of the flow that is
/// shown starts at `v0` and is `w` long when the text is vertical and `h` long otherwise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub mode: WritingMode,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub v0: f64,
}

/// A rectangle `(x, y, width, height)`.
pub type Rect = (f64, f64, f64, f64);

impl Frame {
    pub fn new(mode: WritingMode, rect: Rect, v0: f64) -> Frame {
        Frame {
            mode,
            x: rect.0,
            y: rect.1,
            w: rect.2,
            h: rect.3,
            v0,
        }
    }

    pub fn is_vertical(&self) -> bool {
        self.mode.is_vertical()
    }

    /// How much of the flow the frame shows: its extent along the block axis.
    pub fn block_extent(&self) -> f64 {
        if self.is_vertical() {
            self.w
        } else {
            self.h
        }
    }

    /// How long a line is: its extent along the inline axis.
    pub fn line_extent(&self) -> f64 {
        if self.is_vertical() {
            self.h
        } else {
            self.w
        }
    }

    /// Screen position of the logical point `(u, v)`.
    pub fn point(&self, u: f64, v: f64) -> (f64, f64) {
        match self.mode {
            WritingMode::HorizontalTb => (self.x + u, self.y + (v - self.v0)),
            WritingMode::VerticalRl => (self.x + self.w - (v - self.v0), self.y + u),
            WritingMode::VerticalLr => (self.x + (v - self.v0), self.y + u),
        }
    }

    /// The screen rectangle of the logical one starting at `(u, v)`, `iw` wide along the line and
    /// `bh` tall along the flow.
    pub fn rect(&self, u: f64, v: f64, iw: f64, bh: f64) -> Rect {
        match self.mode {
            WritingMode::HorizontalTb => (self.x + u, self.y + (v - self.v0), iw, bh),
            WritingMode::VerticalRl => (self.x + self.w - (v - self.v0) - bh, self.y + u, bh, iw),
            WritingMode::VerticalLr => (self.x + (v - self.v0), self.y + u, bh, iw),
        }
    }

    /// The logical `(u, v)` under a screen point: the inverse of [`Frame::point`].
    pub fn locate(&self, px: f64, py: f64) -> (f64, f64) {
        match self.mode {
            WritingMode::HorizontalTb => (px - self.x, self.v0 + (py - self.y)),
            WritingMode::VerticalRl => (py - self.y, self.v0 + (self.x + self.w - px)),
            WritingMode::VerticalLr => (py - self.y, self.v0 + (px - self.x)),
        }
    }
}

/// One line of a flow, as far as cutting pages is concerned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlowLine {
    /// The paragraph the line belongs to.
    pub item: usize,
    /// Index of the line in its paragraph.
    pub line: i32,
    /// Position along the flow and thickness (height of a horizontal line, width of a column).
    pub v: f64,
    pub thick: f64,
}

/// Cuts a flow into pages of at most `extent` along the block axis, at line boundaries, and returns
/// `(first, last + 1)` positions along the flow for every page. A paragraph is not left with a
/// single orphan or widow line when it can be avoided. Works for horizontal text (`extent` is the
/// height of the text area) and for vertical text (`extent` is its width): the flow does not care
/// which way it is drawn.
pub fn paginate(lines: &[FlowLine], extent: f64) -> Vec<(f64, f64)> {
    let n = lines.len();
    let mut pages = Vec::new();
    let mut i = 0;
    while i < n {
        let start = lines[i].v;
        let mut j = i;
        while j < n && lines[j].v + lines[j].thick <= start + extent + 0.5 {
            j += 1;
        }
        if j == i {
            j = i + 1;
        }
        if j < n && j - i > 3 {
            let (a, b) = (&lines[j - 1], &lines[j]);
            if a.item == b.item && a.line == 0 {
                j -= 1;
            } else if b.item == a.item {
                if let Some(c) = lines.get(j + 1) {
                    if c.item != b.item {
                        j -= 1;
                    }
                }
            }
        }
        let last = &lines[j - 1];
        pages.push((start, last.v + last.thick));
        i = j;
    }
    if pages.is_empty() {
        pages.push((0.0, 1.0));
    }
    pages
}

// ---------------------------------------------------------------------------------------------
// Kinsoku
// ---------------------------------------------------------------------------------------------

/// Characters that may not start a line (JIS X 4051, the strict set): closing brackets, commas and
/// full stops, marks like `?` and `!`, the prolonged sound mark and the small kana, iteration marks.
pub fn cannot_start_line(c: char) -> bool {
    matches!(c,
        '、' | '。' | '，' | '．' | '､' | '｡' | '）' | '〕' | '］' | '｝' | '〉' | '》' | '」' | '』' | '】'
        | '〙' | '〗' | '〟' | '’' | '”' | '｠' | '»' | ')' | ']' | '}'
        | '・' | '･' | '：' | '；' | '？' | '！' | '‼' | '⁇' | '⁈' | '⁉' | '…' | '‥'
        | 'ー' | 'ｰ' | '〜' | '～' | '゠' | '‐' | '–'
        | 'ぁ' | 'ぃ' | 'ぅ' | 'ぇ' | 'ぉ' | 'っ' | 'ゃ' | 'ゅ' | 'ょ' | 'ゎ' | 'ゕ' | 'ゖ'
        | 'ァ' | 'ィ' | 'ゥ' | 'ェ' | 'ォ' | 'ッ' | 'ャ' | 'ュ' | 'ョ' | 'ヮ' | 'ヵ' | 'ヶ'
        | 'ㇰ'..='ㇿ' | 'ｧ'..='ｯ'
        | '々' | '〻' | 'ゝ' | 'ゞ' | 'ヽ' | 'ヾ' | '゛' | '゜' | '\u{3099}' | '\u{309a}'
        | '℃' | '‰' | '′' | '″' | '°' | '％')
}

/// Characters that may not end a line: opening brackets and the symbols that go before a number.
pub fn cannot_end_line(c: char) -> bool {
    matches!(
        c,
        '（' | '〔'
            | '［'
            | '｛'
            | '〈'
            | '《'
            | '「'
            | '『'
            | '【'
            | '〘'
            | '〖'
            | '〝'
            | '‘'
            | '“'
            | '｟'
            | '«'
            | '('
            | '['
            | '{'
            | '＄'
            | '￥'
            | '£'
            | '＃'
            | '№'
    )
}

/// Characters that strict kinsoku keeps off the start of a line although the Unicode line breaking
/// rules (which Pango follows) allow a break before them: the small kana and the prolonged sound
/// mark. The reader forbids the break before them with an `allow-breaks` attribute.
pub fn strict_no_break_before(c: char) -> bool {
    matches!(c,
        'ぁ' | 'ぃ' | 'ぅ' | 'ぇ' | 'ぉ' | 'っ' | 'ゃ' | 'ゅ' | 'ょ' | 'ゎ' | 'ゕ' | 'ゖ'
        | 'ァ' | 'ィ' | 'ゥ' | 'ェ' | 'ォ' | 'ッ' | 'ャ' | 'ュ' | 'ョ' | 'ヮ' | 'ヵ' | 'ヶ'
        | 'ㇰ'..='ㇿ' | 'ー' | 'ｰ' | 'ｧ'..='ｯ')
}

/// Pairs that are never split: dashes and ellipses are written as two characters.
fn inseparable(a: char, b: char) -> bool {
    matches!(
        (a, b),
        ('―', '―') | ('—', '—') | ('…', '…') | ('‥', '‥') | ('〳', '〵')
    )
}

/// Whether a line may break between `prev` and `next`.
pub fn may_break_between(prev: char, next: char) -> bool {
    !cannot_start_line(next) && !cannot_end_line(prev) && !inseparable(prev, next)
}

/// A line break that kinsoku forbids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Violation {
    /// Index of the line that starts after the offending break.
    pub line: usize,
    /// What is wrong with it.
    pub kind: ViolationKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViolationKind {
    /// The line starts with a character that cannot start a line.
    StartsWith(char),
    /// The line before ends with a character that cannot end a line.
    PreviousEndsWith(char),
    /// An inseparable pair (`――`, `……`) is split.
    Split(char, char),
}

/// The forbidden breaks between consecutive lines of one paragraph.
pub fn violations(lines: &[&str]) -> Vec<Violation> {
    let mut out = Vec::new();
    for i in 1..lines.len() {
        let (Some(prev), Some(next)) = (
            lines[i - 1].chars().rfind(|c| !c.is_whitespace()),
            lines[i].chars().find(|c| !c.is_whitespace()),
        ) else {
            continue;
        };
        let kind = if cannot_start_line(next) {
            ViolationKind::StartsWith(next)
        } else if cannot_end_line(prev) {
            ViolationKind::PreviousEndsWith(prev)
        } else if inseparable(prev, next) {
            ViolationKind::Split(prev, next)
        } else {
            continue;
        };
        out.push(Violation { line: i, kind });
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Punctuation squeezing
// ---------------------------------------------------------------------------------------------

fn is_closing(c: char) -> bool {
    matches!(
        c,
        '）' | '〕' | '］' | '｝' | '〉' | '》' | '」' | '』' | '】' | '〙' | '〗' | '〟' | '｠'
    )
}

fn is_opening(c: char) -> bool {
    matches!(
        c,
        '（' | '〔' | '［' | '｛' | '〈' | '《' | '「' | '『' | '【' | '〘' | '〖' | '〝' | '｟'
    )
}

fn is_stop(c: char) -> bool {
    matches!(c, '、' | '。' | '，' | '．')
}

/// Characters (indices in `text`) that are set half an em wide, so that two punctuation marks in a
/// row do not leave a full em of white (JLREQ "yakumono" spacing): a closing bracket, comma or
/// full stop before an opening or closing bracket, and an opening bracket before another. The
/// reader asks the font for its half-width forms (`halt`, `vhal`) for them.
pub fn squeezed(text: &str) -> Vec<usize> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    for i in 0..chars.len().saturating_sub(1) {
        let (a, b) = (chars[i], chars[i + 1]);
        let squeeze = ((is_closing(a) || is_stop(a)) && (is_opening(b) || is_closing(b)))
            || (is_opening(a) && is_opening(b));
        if squeeze {
            out.push(i);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Tate-chu-yoko
// ---------------------------------------------------------------------------------------------

/// Byte ranges of `text` that are set horizontally in one em (`text-combine-upright`): the whole
/// text for `all` (when it is short enough to be legible), every run of ASCII digits that is not
/// longer than the limit for `digits n`.
pub fn combine_ranges(text: &str, mode: TextCombine) -> Vec<(usize, usize)> {
    match mode {
        TextCombine::None => Vec::new(),
        TextCombine::All => {
            let n = text.chars().count();
            if n == 0 || n > 8 || text.contains('\n') {
                Vec::new()
            } else {
                vec![(0, text.len())]
            }
        }
        TextCombine::Digits(limit) => {
            let mut out = Vec::new();
            let mut start = None;
            let mut count = 0usize;
            let push = |start: usize, end: usize, count: usize, out: &mut Vec<_>| {
                if count <= limit as usize {
                    out.push((start, end));
                }
            };
            for (i, c) in text.char_indices() {
                if c.is_ascii_digit() {
                    if start.is_none() {
                        start = Some(i);
                        count = 0;
                    }
                    count += 1;
                } else if let Some(s) = start.take() {
                    push(s, i, count, &mut out);
                }
            }
            if let Some(s) = start {
                push(s, text.len(), count, &mut out);
            }
            out
        }
    }
}

/// Font size factor that makes `chars` characters of width `advance` (in em) fit into one em.
pub fn combine_scale(chars: usize, advance: f64) -> f64 {
    let w = chars as f64 * advance;
    if w <= 1.0 || w <= 0.0 {
        1.0
    } else {
        1.0 / w
    }
}

// ---------------------------------------------------------------------------------------------
// Ruby
// ---------------------------------------------------------------------------------------------

/// Approximate width in em of an annotation set at `scale` of the font size (ruby is half size).
pub fn ruby_width(text: &str, scale: f64) -> f64 {
    text.chars()
        .map(|c| if is_wide(c) { 1.0 } else { 0.55 })
        .sum::<f64>()
        * scale
}

/// Extra space per base character (in em) so that an annotation wider than its base does not
/// run into the neighbouring characters. Zero when the base is at least as wide.
pub fn ruby_spread(base: &str, ruby: &str, scale: f64) -> f64 {
    let base_w: f64 = base
        .chars()
        .map(|c| if is_wide(c) { 1.0 } else { 0.55 })
        .sum();
    let n = base.chars().count();
    if n == 0 {
        return 0.0;
    }
    ((ruby_width(ruby, scale) - base_w) / n as f64).max(0.0)
}

/// East Asian wide or fullwidth characters (an em wide).
pub fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115f | 0x2e80..=0x303e | 0x3041..=0x33ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff
        | 0xa000..=0xa4cf | 0xac00..=0xd7a3 | 0xf900..=0xfaff | 0xfe30..=0xfe6f | 0xff00..=0xff60
        | 0xffe0..=0xffe6 | 0x20000..=0x3fffd)
}

/// How many wide (ideographic) characters `text` has: the characters that are set upright and
/// spaced evenly in a justified vertical line.
pub fn wide_count(text: &str) -> usize {
    text.chars().filter(|c| is_wide(*c)).count()
}

/// Extra advance (in the same unit as the arguments) to give every character of a justified line
/// of `chars` characters whose natural length is `natural` so that it fills `target`. `None`
/// when the line is already full or the gap is too large for even spacing to look right (more than
/// `max_per_char` per character): such a line stays ragged.
pub fn justify_spacing(natural: f64, target: f64, chars: usize, max_per_char: f64) -> Option<f64> {
    if chars < 2 || target <= natural {
        return None;
    }
    let per = (target - natural) / (chars - 1) as f64;
    (per <= max_per_char).then_some(per)
}

#[cfg(test)]
mod tests {
    use super::*;
    use WritingMode::*;

    fn frame(mode: WritingMode) -> Frame {
        Frame::new(mode, (100.0, 20.0, 300.0, 500.0), 1000.0)
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn horizontal_frame_is_the_plain_offset() {
        let f = frame(HorizontalTb);
        assert_eq!(f.point(10.0, 1040.0), (110.0, 60.0));
        assert_eq!(f.rect(10.0, 1040.0, 50.0, 20.0), (110.0, 60.0, 50.0, 20.0));
        assert_eq!(f.block_extent(), 500.0);
        assert_eq!(f.line_extent(), 300.0);
    }

    #[test]
    fn vertical_rl_flows_from_the_right_edge_to_the_left() {
        let f = frame(VerticalRl);
        // the first column sits against the right edge of the page
        let (x, y, w, h) = f.rect(0.0, 1000.0, 500.0, 30.0);
        assert!(close(x + w, 400.0), "right edge of the text area: {x} {w}");
        assert_eq!((y, w, h), (20.0, 30.0, 500.0));
        // the next column is to the left of it
        let (x2, ..) = f.rect(0.0, 1030.0, 500.0, 30.0);
        assert!(close(x2 + 30.0, x), "{x2} {x}");
        // lines run down: the inline axis is y
        assert_eq!(f.point(0.0, 1000.0).1, 20.0);
        assert_eq!(f.point(120.0, 1000.0).1, 140.0);
        assert_eq!(f.block_extent(), 300.0);
        assert_eq!(f.line_extent(), 500.0);
    }

    #[test]
    fn vertical_lr_flows_from_the_left_edge_to_the_right() {
        let f = frame(VerticalLr);
        let (x, y, w, h) = f.rect(0.0, 1000.0, 500.0, 30.0);
        assert_eq!((x, y, w, h), (100.0, 20.0, 30.0, 500.0));
        let (x2, ..) = f.rect(0.0, 1030.0, 500.0, 30.0);
        assert!(close(x2, 130.0));
    }

    #[test]
    fn locate_inverts_point_in_every_mode() {
        for mode in [HorizontalTb, VerticalRl, VerticalLr] {
            let f = frame(mode);
            for (u, v) in [(0.0, 1000.0), (37.5, 1012.25), (260.0, 1200.0)] {
                let (px, py) = f.point(u, v);
                let (u2, v2) = f.locate(px, py);
                assert!(
                    close(u, u2) && close(v, v2),
                    "{mode:?}: {u},{v} -> {u2},{v2}"
                );
            }
        }
    }

    #[test]
    fn a_logical_rect_and_its_corners_agree() {
        // the rectangle covers exactly the points of its logical corners
        for mode in [VerticalRl, VerticalLr] {
            let f = frame(mode);
            let (u, v, iw, bh) = (40.0, 1010.0, 120.0, 25.0);
            let (x, y, w, h) = f.rect(u, v, iw, bh);
            let corners = [f.point(u, v), f.point(u + iw, v + bh)];
            let (xa, xb) = (
                corners[0].0.min(corners[1].0),
                corners[0].0.max(corners[1].0),
            );
            let (ya, yb) = (
                corners[0].1.min(corners[1].1),
                corners[0].1.max(corners[1].1),
            );
            assert!(close(x, xa) && close(x + w, xb), "{mode:?}");
            assert!(close(y, ya) && close(y + h, yb), "{mode:?}");
        }
    }

    fn lines(spec: &[(usize, f64)]) -> Vec<FlowLine> {
        let mut v = 0.0;
        let mut seen: std::collections::HashMap<usize, i32> = Default::default();
        spec.iter()
            .map(|(item, thick)| {
                let line = seen.entry(*item).or_insert(-1);
                *line += 1;
                let l = FlowLine {
                    item: *item,
                    line: *line,
                    v,
                    thick: *thick,
                };
                v += thick;
                l
            })
            .collect()
    }

    #[test]
    fn pages_are_cut_along_the_flow_at_line_boundaries() {
        // 10 lines of 30: a page 100 wide takes three of them
        let l = lines(&[(0, 30.0); 10].map(|(a, b)| (a, b)));
        let p = paginate(&l, 100.0);
        assert_eq!(p.len(), 4);
        assert_eq!(p[0], (0.0, 90.0));
        assert_eq!(p[1], (90.0, 180.0));
        assert_eq!(p[3], (270.0, 300.0));
    }

    #[test]
    fn the_same_flow_gives_the_same_pages_for_a_transposed_extent() {
        // the page cut only depends on the extent along the block axis, never on the direction
        let mut spec = Vec::new();
        for item in 0..6 {
            for _ in 0..(3 + item) {
                spec.push((item, 22.0));
            }
        }
        let l = lines(&spec);
        let height_text = paginate(&l, 440.0);
        let vertical_text = paginate(&l, 440.0);
        assert_eq!(height_text, vertical_text);
        for w in height_text.windows(2) {
            assert!((w[0].1 - w[1].0).abs() < 1e-9, "pages are contiguous");
        }
        assert_eq!(height_text.last().unwrap().1, l.last().unwrap().v + 22.0);
    }

    #[test]
    fn a_single_line_wider_than_the_page_gets_its_own_page() {
        let l = lines(&[(0, 500.0), (1, 20.0)]);
        let p = paginate(&l, 100.0);
        assert_eq!(p, vec![(0.0, 500.0), (500.0, 520.0)]);
    }

    #[test]
    fn a_page_does_not_end_after_the_first_line_of_a_paragraph() {
        // 4 lines of paragraph 0 then a paragraph of 5 lines: the cut falls after line 1 of
        // the second paragraph, which would leave one orphan; it moves back
        let l = lines(&[
            (0, 20.0),
            (0, 20.0),
            (0, 20.0),
            (0, 20.0),
            (1, 20.0),
            (1, 20.0),
            (1, 20.0),
        ]);
        let p = paginate(&l, 100.0);
        assert_eq!(
            p[0],
            (0.0, 80.0),
            "the lone first line moves to the next page"
        );
    }

    #[test]
    fn an_empty_flow_has_one_page() {
        assert_eq!(paginate(&[], 100.0), vec![(0.0, 1.0)]);
    }

    #[test]
    fn closing_punctuation_cannot_start_a_line() {
        for c in "、。，．）〕］｝〉》」』】？！ー々ぁぃっゃゅょァッ・〜".chars()
        {
            assert!(cannot_start_line(c), "{c}");
        }
        for c in "あ漢A「（".chars() {
            assert!(!cannot_start_line(c), "{c}");
        }
    }

    #[test]
    fn small_kana_and_the_long_vowel_mark_are_forbidden_by_strict_kinsoku_only() {
        for c in "ぁっゃァッャーｰｧ".chars() {
            assert!(strict_no_break_before(c), "{c}");
            assert!(cannot_start_line(c), "{c}: the strict set contains it too");
        }
        for c in "あかカ。、漢A".chars() {
            assert!(!strict_no_break_before(c), "{c}");
        }
    }

    #[test]
    fn opening_brackets_cannot_end_a_line() {
        for c in "（〔［｛〈《「『【“‘".chars() {
            assert!(cannot_end_line(c), "{c}");
        }
        assert!(!cannot_end_line('」'));
        assert!(!cannot_end_line('あ'));
    }

    #[test]
    fn dashes_and_ellipses_are_not_split() {
        assert!(!may_break_between('―', '―'));
        assert!(!may_break_between('…', '…'));
        assert!(may_break_between('あ', 'い'));
        assert!(!may_break_between('あ', '。'));
        assert!(!may_break_between('「', 'あ'));
        assert!(may_break_between('。', 'あ'));
    }

    #[test]
    fn violations_name_the_bad_breaks() {
        assert!(violations(&["吾輩は猫である", "名前はまだ無い"]).is_empty());
        let v = violations(&[
            "吾輩は猫である",
            "。名前は",
            "「まだ",
            "無い「",
            "と言",
            "そうだ―",
            "―ので",
        ]);
        assert_eq!(
            v,
            vec![
                Violation {
                    line: 1,
                    kind: ViolationKind::StartsWith('。')
                },
                Violation {
                    line: 4,
                    kind: ViolationKind::PreviousEndsWith('「')
                },
                Violation {
                    line: 6,
                    kind: ViolationKind::Split('―', '―')
                },
            ]
        );
    }

    #[test]
    fn blank_lines_do_not_hide_a_violation_and_do_not_make_one() {
        assert!(violations(&["あ", "", "い"]).is_empty());
        assert_eq!(violations(&["あ  ", "  、い"]).len(), 1);
    }

    #[test]
    fn punctuation_in_a_row_is_squeezed() {
        // 」「 : the closing bracket; 。」 : the full stop; 「「 : the first; a lone mark is not
        assert_eq!(squeezed("彼は」「言った"), vec![2]);
        assert_eq!(squeezed("行った。」"), vec![3]);
        assert_eq!(squeezed("「「あ」」"), vec![0, 3]);
        assert_eq!(squeezed("あ。い"), Vec::<usize>::new());
        assert_eq!(squeezed("（あ）"), Vec::<usize>::new());
        assert_eq!(squeezed("」"), Vec::<usize>::new());
        assert_eq!(squeezed(""), Vec::<usize>::new());
    }

    #[test]
    fn digits_are_combined_up_to_the_limit() {
        let r = |t, m| combine_ranges(t, m);
        // byte ranges: Japanese characters are three bytes
        assert_eq!(r("平成12年", TextCombine::Digits(2)), vec![(6, 8)]);
        assert_eq!(
            r("平成123年", TextCombine::Digits(2)),
            Vec::<(usize, usize)>::new()
        );
        assert_eq!(r("平成123年", TextCombine::Digits(3)), vec![(6, 9)]);
        assert_eq!(r("1月2日", TextCombine::Digits(2)), vec![(0, 1), (4, 5)]);
        assert_eq!(
            r("12345", TextCombine::Digits(4)),
            Vec::<(usize, usize)>::new()
        );
        assert_eq!(r("12年34", TextCombine::Digits(2)), vec![(0, 2), (5, 7)]);
        assert_eq!(
            r("abc", TextCombine::Digits(2)),
            Vec::<(usize, usize)>::new()
        );
        assert_eq!(r("12", TextCombine::None), Vec::<(usize, usize)>::new());
    }

    #[test]
    fn combine_all_takes_the_whole_short_text() {
        assert_eq!(combine_ranges("20!", TextCombine::All), vec![(0, 3)]);
        assert_eq!(
            combine_ranges("", TextCombine::All),
            Vec::<(usize, usize)>::new()
        );
        assert_eq!(
            combine_ranges("123456789", TextCombine::All),
            Vec::<(usize, usize)>::new(),
            "too long to read"
        );
    }

    #[test]
    fn combined_characters_shrink_to_fit_one_em() {
        assert_eq!(combine_scale(1, 0.55), 1.0);
        assert_eq!(combine_scale(2, 0.5), 1.0);
        assert!((combine_scale(4, 0.55) - 1.0 / 2.2).abs() < 1e-9);
    }

    #[test]
    fn a_long_annotation_spreads_its_base() {
        // half size kana over one ideograph: 3 kana are 1.5 em
        assert!((ruby_spread("漢", "かんじ", 0.5) - 0.5).abs() < 1e-9);
        // a short annotation fits
        assert_eq!(ruby_spread("漢字", "かん", 0.5), 0.0);
        assert_eq!(ruby_spread("", "かん", 0.5), 0.0);
        // Latin annotations are narrower
        assert!(ruby_width("abc", 0.5) < ruby_width("かんじ", 0.5));
    }

    #[test]
    fn a_justified_line_is_spread_evenly_when_the_gap_is_small() {
        // 10 characters, 3 px short: 3/9 px between them
        let per = justify_spacing(397.0, 400.0, 10, 5.0).unwrap();
        assert!((per - 3.0 / 9.0).abs() < 1e-9);
        assert_eq!(justify_spacing(400.0, 400.0, 10, 5.0), None);
        assert_eq!(justify_spacing(401.0, 400.0, 10, 5.0), None);
        assert_eq!(
            justify_spacing(100.0, 400.0, 10, 5.0),
            None,
            "too short: stays ragged"
        );
        assert_eq!(justify_spacing(100.0, 400.0, 1, 5.0), None);
    }

    #[test]
    fn wide_characters_are_counted() {
        assert!(is_wide('漢') && is_wide('あ') && is_wide('「') && is_wide('Ａ'));
        assert!(!is_wide('a') && !is_wide('1'));
        assert_eq!(wide_count("漢字abc、"), 3);
    }
}
