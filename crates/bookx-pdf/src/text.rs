//! Text of a page with positions: lines, reading order, selection geometry and search.
//!
//! The input is a list of [`RawGlyph`]s in page space (points, origin top left, y down) as the
//! content stream draws them; [`PageText::build`] groups them into rows and lines, orders the
//! lines (columns are read one after the other) and keeps the box of every character, so that a
//! click maps to a character and a character range maps to rectangles.
//!
//! No PDF types in here: everything is unit tested with synthetic glyphs.

/// A box `[x0, y0, x1, y1]` in page space (points, y down).
pub type Rect = [f32; 4];

/// One drawn glyph with its Unicode text.
#[derive(Debug, Clone, PartialEq)]
pub struct RawGlyph {
    pub text: String,
    /// Left end of the glyph.
    pub x: f32,
    /// Baseline, y down.
    pub y: f32,
    pub width: f32,
    /// Font size in points.
    pub size: f32,
}

/// A character of the page text with its box. Spaces and line breaks the layout implies have a
/// synthetic box.
#[derive(Debug, Clone, PartialEq)]
pub struct TextChar {
    pub ch: char,
    pub rect: Rect,
}

/// A line of text: `start..end` are indexes into [`PageText::chars`] (the line break is at `end`).
#[derive(Debug, Clone, PartialEq)]
pub struct TextLine {
    pub start: usize,
    pub end: usize,
    pub rect: Rect,
    pub size: f32,
    /// Lines of the same block (a column or a paragraph region) share the number.
    pub block: usize,
    /// Baseline y.
    pub baseline: f32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PageText {
    pub width: f32,
    pub height: f32,
    pub chars: Vec<TextChar>,
    pub lines: Vec<TextLine>,
}

/// More lines than this are not analysed for columns (plain top to bottom order).
const MAX_LINES_FOR_COLUMNS: usize = 4000;

#[derive(Clone)]
struct Seg {
    glyphs: Vec<RawGlyph>,
    rect: Rect,
    size: f32,
    baseline: f32,
}

fn glyph_rect(g: &RawGlyph) -> Rect {
    [g.x, g.y - g.size * 0.8, g.x + g.width, g.y + g.size * 0.2]
}

fn union(a: Rect, b: Rect) -> Rect {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

impl PageText {
    pub fn build(width: f32, height: f32, glyphs: Vec<RawGlyph>) -> PageText {
        let glyphs: Vec<RawGlyph> = glyphs
            .into_iter()
            .filter(|g| {
                !g.text.is_empty()
                    && g.x.is_finite()
                    && g.y.is_finite()
                    && g.width.is_finite()
                    && g.size.is_finite()
                    && g.size > 0.0
            })
            .collect();
        let segs = segments(glyphs);
        let order = reading_order(&segs);
        let mut out = PageText {
            width,
            height,
            ..Default::default()
        };
        for (block, group) in order.iter().enumerate() {
            for &i in group {
                let s = &segs[i];
                let start = out.chars.len();
                push_segment(&mut out.chars, s);
                let end = out.chars.len();
                if end == start {
                    continue;
                }
                out.lines.push(TextLine {
                    start,
                    end,
                    rect: s.rect,
                    size: s.size,
                    block,
                    baseline: s.baseline,
                });
                out.chars.push(TextChar {
                    ch: '\n',
                    rect: [s.rect[2], s.rect[1], s.rect[2], s.rect[3]],
                });
            }
        }
        out
    }

    /// The text of the page.
    pub fn text(&self) -> String {
        self.chars.iter().map(|c| c.ch).collect()
    }

    /// The text of the characters `range` (clamped).
    pub fn text_for(&self, range: std::ops::Range<usize>) -> String {
        let end = range.end.min(self.chars.len());
        let start = range.start.min(end);
        self.chars[start..end].iter().map(|c| c.ch).collect()
    }

    /// The character under or nearest to the point, if it is within a line (or a few points of it).
    pub fn char_at(&self, x: f32, y: f32) -> Option<usize> {
        let line = self
            .lines
            .iter()
            .filter(|l| y >= l.rect[1] - l.size * 0.3 && y <= l.rect[3] + l.size * 0.3)
            .min_by(|a, b| {
                let da = (y - (a.rect[1] + a.rect[3]) / 2.0).abs();
                let db = (y - (b.rect[1] + b.rect[3]) / 2.0).abs();
                da.total_cmp(&db)
            })?;
        let mut best = line.start;
        let mut best_d = f32::MAX;
        for (i, ch) in self
            .chars
            .iter()
            .enumerate()
            .take(line.end)
            .skip(line.start)
        {
            let r = ch.rect;
            let d = if x < r[0] {
                r[0] - x
            } else if x > r[2] {
                x - r[2]
            } else {
                0.0
            };
            if d < best_d {
                best_d = d;
                best = i;
            }
        }
        Some(best)
    }

    /// The first character at or after the point in reading order (used for drag selection
    /// outside the text: the nearest character wins).
    pub fn nearest_char(&self, x: f32, y: f32) -> Option<usize> {
        if let Some(i) = self.char_at(x, y) {
            return Some(i);
        }
        self.lines
            .iter()
            .min_by(|a, b| {
                let d = |l: &TextLine| {
                    let dy = if y < l.rect[1] {
                        l.rect[1] - y
                    } else if y > l.rect[3] {
                        y - l.rect[3]
                    } else {
                        0.0
                    };
                    let dx = if x < l.rect[0] {
                        l.rect[0] - x
                    } else if x > l.rect[2] {
                        x - l.rect[2]
                    } else {
                        0.0
                    };
                    dy * dy + dx * dx
                };
                d(a).total_cmp(&d(b))
            })
            .map(|l| {
                if y < l.rect[1] || (y <= l.rect[3] && x < l.rect[0]) {
                    l.start
                } else {
                    l.end.saturating_sub(1).max(l.start)
                }
            })
    }

    /// One rectangle per line for the characters `range` (the line breaks are left out).
    pub fn rects_for(&self, range: std::ops::Range<usize>) -> Vec<Rect> {
        let end = range.end.min(self.chars.len());
        let mut out = Vec::new();
        for l in &self.lines {
            let a = range.start.max(l.start);
            let b = end.min(l.end);
            if a >= b {
                continue;
            }
            let mut r = self.chars[a].rect;
            for c in &self.chars[a..b] {
                r = union(r, c.rect);
            }
            // the box of the line, not of the glyphs, so that the height is even
            r[1] = l.rect[1];
            r[3] = l.rect[3];
            out.push(r);
        }
        out
    }

    /// All matches of `query` (character ranges), ignoring case; whitespace in the query matches
    /// any run of whitespace, line breaks included.
    pub fn find(&self, query: &str) -> Vec<std::ops::Range<usize>> {
        let q: Vec<char> = query
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .flat_map(|c| c.to_lowercase())
            .collect();
        if q.is_empty() {
            return Vec::new();
        }
        // text with whitespace runs collapsed; `map[i]` is the char index of collapsed char i
        let mut flat: Vec<char> = Vec::with_capacity(self.chars.len());
        let mut map: Vec<usize> = Vec::with_capacity(self.chars.len());
        let mut last_space = true;
        for (i, c) in self.chars.iter().enumerate() {
            if c.ch.is_whitespace() {
                if !last_space {
                    flat.push(' ');
                    map.push(i);
                    last_space = true;
                }
            } else {
                for l in c.ch.to_lowercase() {
                    flat.push(l);
                    map.push(i);
                }
                last_space = false;
            }
        }
        let mut out = Vec::new();
        let mut i = 0;
        while i + q.len() <= flat.len() {
            if flat[i..i + q.len()] == q[..] {
                let start = map[i];
                let end = map[i + q.len() - 1] + 1;
                out.push(start..end);
                i += q.len();
            } else {
                i += 1;
            }
        }
        out
    }

    /// The range of the word at `idx` (letters, digits and apostrophes).
    pub fn word_at(&self, idx: usize) -> Option<std::ops::Range<usize>> {
        let word = |c: &TextChar| c.ch.is_alphanumeric() || c.ch == '\'' || c.ch == '’';
        if !self.chars.get(idx).is_some_and(word) {
            return None;
        }
        let mut a = idx;
        while a > 0 && word(&self.chars[a - 1]) {
            a -= 1;
        }
        let mut b = idx + 1;
        while b < self.chars.len() && word(&self.chars[b]) {
            b += 1;
        }
        Some(a..b)
    }

    /// The range of the line holding `idx`.
    pub fn line_at(&self, idx: usize) -> Option<std::ops::Range<usize>> {
        self.lines
            .iter()
            .find(|l| idx >= l.start && idx <= l.end)
            .map(|l| l.start..l.end)
    }

    /// Re-anchors a stored quote: the range if `quote` still sits at `start..end`, otherwise the
    /// nearest match of the quote in the page, otherwise `None`.
    pub fn anchor(&self, start: usize, end: usize, quote: &str) -> Option<std::ops::Range<usize>> {
        let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        if end <= self.chars.len() && start < end && norm(&self.text_for(start..end)) == norm(quote)
        {
            return Some(start..end);
        }
        self.find(quote)
            .into_iter()
            .min_by_key(|r| r.start.abs_diff(start))
    }

    /// A short piece of text around `range` for a result list: `…before hit after…` with the
    /// whitespace collapsed, and the range of the hit inside that string (in characters).
    pub fn snippet(
        &self,
        range: std::ops::Range<usize>,
        context: usize,
    ) -> (String, std::ops::Range<usize>) {
        let n = self.chars.len();
        let (hs, he) = (range.start.min(n), range.end.min(n));
        let a = hs.saturating_sub(context);
        let b = (he + context).min(n);
        let flat = |r: std::ops::Range<usize>| -> String {
            let s: String = self.chars[r].iter().map(|c| c.ch).collect();
            s.split_whitespace().collect::<Vec<_>>().join(" ")
        };
        let before = flat(a..hs);
        let hit = flat(hs..he);
        let after = flat(he..b);
        let pre = if a > 0 { "…" } else { "" };
        let post = if b < n { "…" } else { "" };
        let lead = pre.chars().count() + before.chars().count() + usize::from(!before.is_empty());
        let s = format!(
            "{pre}{before}{}{hit}{}{after}{post}",
            if before.is_empty() { "" } else { " " },
            if after.is_empty() { "" } else { " " }
        );
        (s, lead..lead + hit.chars().count())
    }

    /// True when the page has no text layer.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

fn median(v: &mut [f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

/// Rows of glyphs on one baseline, split at wide gaps.
fn segments(mut glyphs: Vec<RawGlyph>) -> Vec<Seg> {
    glyphs.sort_by(|a, b| a.y.total_cmp(&b.y));
    let mut rows: Vec<Vec<RawGlyph>> = Vec::new();
    let mut row_y = 0.0f32;
    let mut row_size = 0.0f32;
    let mut count = 0.0f32;
    for g in glyphs {
        let tol = 0.45 * row_size.max(g.size);
        if rows.is_empty() || (g.y - row_y).abs() > tol {
            rows.push(vec![g.clone()]);
            row_y = g.y;
            row_size = g.size;
            count = 1.0;
        } else {
            row_y = (row_y * count + g.y) / (count + 1.0);
            count += 1.0;
            row_size = row_size.max(g.size);
            rows.last_mut().unwrap().push(g);
        }
    }
    let mut segs = Vec::new();
    for mut row in rows {
        row.sort_by(|a, b| a.x.total_cmp(&b.x));
        let mut cur: Vec<RawGlyph> = Vec::new();
        let mut end = f32::MIN;
        for g in row {
            let gap_limit = 2.0 * g.size.max(1.0);
            if !cur.is_empty() && g.x - end > gap_limit {
                segs.push(make_seg(std::mem::take(&mut cur)));
            }
            end = end.max(g.x + g.width);
            cur.push(g);
        }
        if !cur.is_empty() {
            segs.push(make_seg(cur));
        }
    }
    segs
}

fn make_seg(glyphs: Vec<RawGlyph>) -> Seg {
    let mut rect = glyph_rect(&glyphs[0]);
    let mut sizes: Vec<f32> = Vec::new();
    let mut base = 0.0;
    for g in &glyphs {
        rect = union(rect, glyph_rect(g));
        sizes.push(g.size);
        base += g.y;
    }
    // the box of a line is as high as its dominant font, so that superscripts do not stretch it
    let size = median(&mut sizes);
    let baseline = base / glyphs.len() as f32;
    rect[1] = baseline - size * 0.8;
    rect[3] = baseline + size * 0.2;
    Seg {
        glyphs,
        rect,
        size,
        baseline,
    }
}

fn push_segment(chars: &mut Vec<TextChar>, s: &Seg) {
    let mut prev_end: Option<f32> = None;
    let mut prev_space = true;
    for g in &s.glyphs {
        let r = [g.x, s.rect[1], g.x + g.width, s.rect[3]];
        if let Some(pe) = prev_end
            && g.x - pe > 0.18 * g.size.max(s.size * 0.7)
            && !prev_space
            && !g.text.starts_with(' ')
        {
            chars.push(TextChar {
                ch: ' ',
                rect: [pe, s.rect[1], g.x.max(pe), s.rect[3]],
            });
            prev_space = true;
        }
        let n = g.text.chars().count().max(1) as f32;
        for (k, ch) in g.text.chars().enumerate() {
            let w = (r[2] - r[0]) / n;
            let ch = if ch.is_control() || ch == '\u{a0}' {
                ' '
            } else {
                ch
            };
            if ch == ' ' && prev_space {
                continue;
            }
            chars.push(TextChar {
                ch,
                rect: [r[0] + w * k as f32, r[1], r[0] + w * (k as f32 + 1.0), r[3]],
            });
            prev_space = ch == ' ';
        }
        prev_end = Some(prev_end.map_or(r[2], |p| p.max(r[2])));
    }
    // trailing spaces do not belong to the line
    while chars.last().is_some_and(|c| c.ch == ' ') {
        chars.pop();
    }
}

/// Reading order of the segments: groups of indexes, a group is a block read top to bottom.
fn reading_order(segs: &[Seg]) -> Vec<Vec<usize>> {
    let all: Vec<usize> = (0..segs.len()).collect();
    if segs.len() > MAX_LINES_FOR_COLUMNS {
        let mut v = all;
        v.sort_by(|a, b| {
            (segs[*a].baseline, segs[*a].rect[0])
                .partial_cmp(&(segs[*b].baseline, segs[*b].rect[0]))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        return vec![v];
    }
    let mut out = Vec::new();
    cut(segs, all, &mut out, 0);
    out
}

fn avg_size(segs: &[Seg], idx: &[usize]) -> f32 {
    let s: f32 = idx.iter().map(|i| segs[*i].size).sum();
    (s / idx.len().max(1) as f32).max(1.0)
}

fn cut(segs: &[Seg], idx: Vec<usize>, out: &mut Vec<Vec<usize>>, depth: usize) {
    if idx.is_empty() {
        return;
    }
    if idx.len() > 1 && depth < 24 {
        if let Some(parts) = vertical_cut(segs, &idx) {
            for p in parts {
                cut(segs, p, out, depth + 1);
            }
            return;
        }
        if let Some(parts) = horizontal_cut(segs, &idx) {
            for p in parts {
                cut(segs, p, out, depth + 1);
            }
            return;
        }
    }
    let mut v = idx;
    v.sort_by(|a, b| {
        let (sa, sb) = (&segs[*a], &segs[*b]);
        if (sa.baseline - sb.baseline).abs() < 0.45 * sa.size.max(sb.size) {
            sa.rect[0].total_cmp(&sb.rect[0])
        } else {
            sa.baseline.total_cmp(&sb.baseline)
        }
    });
    out.push(v);
}

/// Splits at a vertical strip no line crosses (a column gutter), left part first.
fn vertical_cut(segs: &[Seg], idx: &[usize]) -> Option<Vec<Vec<usize>>> {
    let min_gap = 1.2 * avg_size(segs, idx);
    let mut spans: Vec<(f32, f32)> = idx
        .iter()
        .map(|i| (segs[*i].rect[0], segs[*i].rect[2]))
        .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut reach = spans[0].1;
    let mut best: Option<(f32, f32)> = None;
    for (a, b) in spans.iter().skip(1) {
        if *a - reach >= min_gap && best.is_none_or(|(x0, x1)| a - reach > x1 - x0) {
            best = Some((reach, *a));
        }
        reach = reach.max(*b);
    }
    let (g0, g1) = best?;
    let mid = (g0 + g1) / 2.0;
    let left: Vec<usize> = idx
        .iter()
        .copied()
        .filter(|i| segs[*i].rect[2] <= mid)
        .collect();
    let right: Vec<usize> = idx
        .iter()
        .copied()
        .filter(|i| segs[*i].rect[2] > mid)
        .collect();
    if left.is_empty() || right.is_empty() {
        return None;
    }
    Some(vec![left, right])
}

/// Splits at vertical gaps larger than a line (headings, paragraphs of different layout).
fn horizontal_cut(segs: &[Seg], idx: &[usize]) -> Option<Vec<Vec<usize>>> {
    let min_gap = 0.9 * avg_size(segs, idx);
    let mut sorted = idx.to_vec();
    sorted.sort_by(|a, b| segs[*a].rect[1].total_cmp(&segs[*b].rect[1]));
    let mut parts: Vec<Vec<usize>> = Vec::new();
    let mut cur = vec![sorted[0]];
    let mut reach = segs[sorted[0]].rect[3];
    for &i in sorted.iter().skip(1) {
        if segs[i].rect[1] - reach >= min_gap {
            parts.push(std::mem::take(&mut cur));
        }
        reach = reach.max(segs[i].rect[3]);
        cur.push(i);
    }
    parts.push(cur);
    if parts.len() > 1 { Some(parts) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line of `text` starting at `x`, baseline `y`, 10 points per character of 12 points size.
    fn line(x: f32, y: f32, text: &str) -> Vec<RawGlyph> {
        text.chars()
            .enumerate()
            .map(|(i, c)| RawGlyph {
                text: c.to_string(),
                x: x + 6.0 * i as f32,
                y,
                width: 6.0,
                size: 12.0,
            })
            .collect()
    }

    fn page(lines: Vec<Vec<RawGlyph>>) -> PageText {
        PageText::build(612.0, 792.0, lines.into_iter().flatten().collect())
    }

    #[test]
    fn words_get_spaces_from_gaps_and_lines_from_baselines() {
        let mut a = line(72.0, 100.0, "Hello");
        a.extend(line(72.0 + 6.0 * 5.0 + 3.5, 100.0, "world"));
        let p = page(vec![a, line(72.0, 116.0, "second line")]);
        assert_eq!(p.text(), "Hello world\nsecond line\n");
        assert_eq!(p.lines.len(), 2);
    }

    #[test]
    fn glyphs_in_any_stream_order_come_out_in_reading_order() {
        let mut g: Vec<RawGlyph> = line(72.0, 120.0, "bottom");
        g.extend(line(72.0, 100.0, "top"));
        g.reverse();
        let p = PageText::build(612.0, 792.0, g);
        assert_eq!(p.text(), "top\nbottom\n");
    }

    #[test]
    fn two_columns_are_read_one_after_the_other() {
        let p = page(vec![
            line(72.0, 100.0, "left one"),
            line(350.0, 100.0, "right one"),
            line(72.0, 114.0, "left two"),
            line(350.0, 114.0, "right two"),
            line(72.0, 128.0, "left three"),
            line(350.0, 128.0, "right three"),
        ]);
        assert_eq!(
            p.text(),
            "left one\nleft two\nleft three\nright one\nright two\nright three\n"
        );
    }

    #[test]
    fn a_heading_across_both_columns_comes_first() {
        let p = page(vec![
            line(
                72.0,
                60.0,
                "A wide heading across the whole page of text here",
            ),
            line(72.0, 100.0, "left one"),
            line(350.0, 100.0, "right one"),
            line(72.0, 114.0, "left two"),
            line(350.0, 114.0, "right two"),
        ]);
        let t = p.text();
        assert!(t.starts_with("A wide heading"), "{t}");
        assert!(
            t.find("left two").unwrap() < t.find("right one").unwrap(),
            "{t}"
        );
    }

    #[test]
    fn single_column_with_ragged_lines_is_not_split() {
        let p = page(vec![
            line(72.0, 100.0, "a long first line of the paragraph runs"),
            line(72.0, 114.0, "short"),
            line(72.0, 128.0, "another long line of the paragraph runs"),
        ]);
        assert_eq!(
            p.text(),
            "a long first line of the paragraph runs\nshort\nanother long line of the paragraph runs\n"
        );
        assert_eq!(p.lines.iter().map(|l| l.block).max(), Some(0));
    }

    #[test]
    fn ligature_glyphs_split_their_box() {
        let g = vec![RawGlyph {
            text: "fi".into(),
            x: 10.0,
            y: 50.0,
            width: 10.0,
            size: 12.0,
        }];
        let p = PageText::build(100.0, 100.0, g);
        assert_eq!(p.text(), "fi\n");
        assert_eq!(p.chars[0].rect[0], 10.0);
        assert_eq!(p.chars[0].rect[2], 15.0);
        assert_eq!(p.chars[1].rect[2], 20.0);
    }

    #[test]
    fn hit_testing_finds_the_character_and_rects_cover_lines() {
        let p = page(vec![
            line(72.0, 100.0, "abcdef"),
            line(72.0, 116.0, "ghijkl"),
        ]);
        // second character of the first line: x in 78..84
        assert_eq!(p.char_at(80.0, 96.0), Some(1));
        assert_eq!(p.char_at(80.0, 400.0), None);
        let r = p.rects_for(2..10); // "cdef\nghi"
        assert_eq!(r.len(), 2);
        assert_eq!(r[0][0], 72.0 + 12.0);
        assert!(r[0][1] < 100.0 && r[0][3] > 100.0);
        assert_eq!(r[1][2], 72.0 + 18.0);
        assert_eq!(p.text_for(2..10), "cdef\nghi");
    }

    #[test]
    fn nearest_char_clamps_outside_the_text() {
        let p = page(vec![line(72.0, 100.0, "abc"), line(72.0, 116.0, "def")]);
        assert_eq!(p.nearest_char(0.0, 0.0), Some(0));
        assert_eq!(p.nearest_char(500.0, 700.0), Some(6));
    }

    #[test]
    fn search_ignores_case_and_matches_across_line_breaks() {
        let p = page(vec![
            line(72.0, 100.0, "The quick brown"),
            line(72.0, 116.0, "fox jumps"),
        ]);
        let hits = p.find("QUICK");
        assert_eq!(hits.len(), 1);
        assert_eq!(p.text_for(hits[0].clone()), "quick");
        let across = p.find("brown fox");
        assert_eq!(across.len(), 1);
        assert_eq!(p.text_for(across[0].clone()), "brown\nfox");
        assert!(p.find("zebra").is_empty());
        assert!(p.find("   ").is_empty());
    }

    #[test]
    fn word_and_line_at() {
        let p = page(vec![line(72.0, 100.0, "one two")]);
        assert_eq!(p.word_at(5), Some(4..7));
        assert_eq!(p.word_at(3), None);
        assert_eq!(p.line_at(2), Some(0..7));
    }

    #[test]
    fn anchors_survive_a_shifted_text_and_fail_when_the_quote_is_gone() {
        let p = page(vec![line(72.0, 100.0, "xx the quick fox")]);
        assert_eq!(p.anchor(3, 12, "the quick"), Some(3..12));
        assert_eq!(p.anchor(0, 5, "the quick"), Some(3..12));
        assert_eq!(p.anchor(0, 5, "gone"), None);
    }

    #[test]
    fn snippets_mark_the_hit() {
        let p = page(vec![line(
            10.0,
            100.0,
            "the quick brown fox jumps over the lazy dog",
        )]);
        let (s, r) = p.snippet(10..15, 8);
        assert_eq!(s, "…e quick brown fox jum…");
        assert_eq!(
            s.chars().skip(r.start).take(r.len()).collect::<String>(),
            "brown"
        );
        let (s, r) = p.snippet(0..3, 5);
        assert!(s.starts_with("the "));
        assert_eq!(r, 0..3);
        let _ = p.snippet(900..950, 5); // out of range: clamped, no panic
    }

    #[test]
    fn garbage_glyphs_are_ignored() {
        let g = vec![
            RawGlyph {
                text: "a".into(),
                x: f32::NAN,
                y: 1.0,
                width: 1.0,
                size: 12.0,
            },
            RawGlyph {
                text: "".into(),
                x: 1.0,
                y: 1.0,
                width: 1.0,
                size: 12.0,
            },
            RawGlyph {
                text: "b".into(),
                x: 1.0,
                y: 1.0,
                width: 1.0,
                size: 0.0,
            },
        ];
        let p = PageText::build(10.0, 10.0, g);
        assert!(p.is_empty());
        assert_eq!(p.text(), "");
    }

    #[test]
    fn huge_pages_fall_back_to_plain_order() {
        let mut g = Vec::new();
        for i in 0..(MAX_LINES_FOR_COLUMNS + 10) {
            g.extend(line(10.0, 10.0 + i as f32 * 20.0, "x"));
        }
        let p = PageText::build(100.0, 100000.0, g);
        assert_eq!(p.lines.len(), MAX_LINES_FOR_COLUMNS + 10);
    }
}
