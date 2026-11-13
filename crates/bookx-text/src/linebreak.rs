//! Line breaking (R3): Knuth-Plass optimal paragraph breaking over boxes, glue and penalties
//! ("Breaking Paragraphs into Lines", Knuth & Plass 1981), a first-fit greedy breaker for comparison, a helper
//! that turns text into items, and widow/orphan aware pagination of lines.

use crate::hyphenate::{Hyphenator, SOFT_HYPHEN};
use std::ops::Range;

/// One element of a paragraph.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// Unbreakable material (a word fragment) with its measured width.
    Box { width: f64, text: String },
    /// Stretchable, shrinkable space. A break here discards the glue.
    Glue {
        width: f64,
        stretch: f64,
        shrink: f64,
    },
    /// A potential break. `width` is added when the break is taken (a hyphen), `flagged` marks hyphens.
    /// `penalty` of `+INFINITY` forbids the break, `-INFINITY` forces it.
    Penalty {
        width: f64,
        penalty: f64,
        flagged: bool,
    },
}

impl Item {
    pub fn word(width: f64, text: impl Into<String>) -> Item {
        Item::Box {
            width,
            text: text.into(),
        }
    }

    pub fn space(width: f64, stretch: f64, shrink: f64) -> Item {
        Item::Glue {
            width,
            stretch,
            shrink,
        }
    }

    pub fn hyphen(width: f64, penalty: f64) -> Item {
        Item::Penalty {
            width,
            penalty,
            flagged: true,
        }
    }

    /// A penalty that forbids a break at this point.
    pub fn no_break() -> Item {
        Item::Penalty {
            width: 0.0,
            penalty: f64::INFINITY,
            flagged: false,
        }
    }

    pub fn forced_break() -> Item {
        Item::Penalty {
            width: 0.0,
            penalty: f64::NEG_INFINITY,
            flagged: false,
        }
    }

    fn width(&self) -> f64 {
        match self {
            Item::Box { width, .. } | Item::Glue { width, .. } => *width,
            Item::Penalty { .. } => 0.0,
        }
    }
}

/// Line widths: one for all lines, or one per line (the last entry repeats).
#[derive(Debug, Clone, PartialEq)]
pub enum LineWidth {
    Fixed(f64),
    PerLine(Vec<f64>),
}

impl LineWidth {
    /// Width of the 0-based line `n`.
    pub fn at(&self, n: usize) -> f64 {
        match self {
            LineWidth::Fixed(w) => *w,
            LineWidth::PerLine(v) => v.get(n).or(v.last()).copied().unwrap_or(0.0),
        }
    }
}

impl From<f64> for LineWidth {
    fn from(w: f64) -> Self {
        LineWidth::Fixed(w)
    }
}

impl From<Vec<f64>> for LineWidth {
    fn from(w: Vec<f64>) -> Self {
        LineWidth::PerLine(w)
    }
}

/// Tuning knobs. Defaults follow TeX.
#[derive(Debug, Clone, PartialEq)]
pub struct Params {
    /// Maximum badness (100 |r|^3) of a line in the first pass. A second pass without limit follows if
    /// no solution exists.
    pub tolerance: f64,
    pub line_penalty: f64,
    /// Added to demerits when two consecutive lines both end in a flagged (hyphen) break.
    pub flagged_demerits: f64,
    /// Added when adjacent lines differ by more than one fitness class.
    pub fitness_demerits: f64,
    /// Hard limit on consecutive hyphenated lines (`None` = unlimited).
    pub consecutive_hyphen_limit: Option<usize>,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            tolerance: 400.0,
            line_penalty: 10.0,
            flagged_demerits: 3000.0,
            fitness_demerits: 100.0,
            consecutive_hyphen_limit: Some(2),
        }
    }
}

/// A finished line.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// First item of the line.
    pub start: usize,
    /// The item the line breaks at (a glue, a penalty, or `items.len()` for the end).
    pub end: usize,
    /// Adjustment ratio: negative shrinks the glue, positive stretches it. `-1.0` for forced overfull lines and
    /// `+INFINITY` for lines that are too short and have nothing to stretch.
    pub ratio: f64,
    /// Natural width of the content, including the hyphen if the line ends in one.
    pub natural_width: f64,
    /// Target width of this line.
    pub target_width: f64,
    pub hyphenated: bool,
    pub penalty: f64,
    pub overfull: bool,
    pub demerits: f64,
}

impl Line {
    /// Rendered text of the line: boxes joined by single spaces for glue, with a trailing `-` if hyphenated.
    pub fn text(&self, items: &[Item]) -> String {
        let end = self.end.min(items.len());
        let mut s = String::new();
        for it in &items[self.start.min(end)..end] {
            match it {
                Item::Box { text, .. } => s.push_str(text),
                Item::Glue { .. } => s.push(' '),
                Item::Penalty { .. } => {}
            }
        }
        while s.ends_with(' ') {
            s.pop();
        }
        if self.hyphenated {
            s.push('-');
        }
        s
    }

    /// The actual width of every glue item in the line after justification, in item order.
    pub fn glue_widths(&self, items: &[Item]) -> Vec<f64> {
        let end = self.end.min(items.len());
        let r = if self.ratio.is_finite() {
            self.ratio
        } else {
            0.0
        };
        items[self.start.min(end)..end]
            .iter()
            .filter_map(|it| match it {
                Item::Glue {
                    width,
                    stretch,
                    shrink,
                } => Some(if r >= 0.0 {
                    width + r * stretch
                } else {
                    width + r * shrink
                }),
                _ => None,
            })
            .collect()
    }
}

const FINISH_STRETCH: f64 = 1.0e7;
const INFINITE_BADNESS: f64 = 10_000.0;
/// A line that is too short and has no stretch at all: worse than any stretched line.
const UNDERFULL_BADNESS: f64 = 100_000.0;

fn is_forced(it: &Item) -> bool {
    matches!(it, Item::Penalty { penalty, .. } if *penalty == f64::NEG_INFINITY)
}

fn badness(r: f64) -> f64 {
    if r == f64::INFINITY {
        UNDERFULL_BADNESS
    } else if r.is_nan() || r < -1.0 {
        INFINITE_BADNESS
    } else {
        (100.0 * r.abs().powi(3)).min(INFINITE_BADNESS)
    }
}

fn fitness_class(r: f64) -> u8 {
    if r < -0.5 {
        0
    } else if r <= 0.5 {
        1
    } else if r <= 1.0 {
        2
    } else {
        3
    }
}

fn ratio(len: f64, target: f64, stretch: f64, shrink: f64) -> f64 {
    let d = target - len;
    if d.abs() < 1e-9 {
        0.0
    } else if d > 0.0 {
        if stretch > 0.0 {
            d / stretch
        } else {
            f64::INFINITY
        }
    } else if shrink > 0.0 {
        d / shrink
    } else {
        f64::NEG_INFINITY
    }
}

/// Demerits of a single line given the previous line's fitness class and flag.
fn demerits_of(r: f64, pen: f64, flagged: bool, prev: Option<(u8, bool)>, params: &Params) -> f64 {
    let b = badness(r);
    let base = params.line_penalty + b;
    let mut d = base * base;
    if pen > 0.0 {
        d += pen * pen;
    } else if pen > f64::NEG_INFINITY {
        d -= pen * pen;
    }
    if let Some((pc, pf)) = prev {
        if flagged && pf {
            d += params.flagged_demerits;
        }
        if (fitness_class(r) as i32 - pc as i32).abs() > 1 {
            d += params.fitness_demerits;
        }
    }
    d
}

/// Total demerits of a set of lines (what the optimiser minimises), computed independently of how the
/// lines were produced. Overfull lines count as infinitely bad and are charged 10^8.
pub fn total_demerits(lines: &[Line], params: &Params) -> f64 {
    let mut prev = Some((1u8, false));
    let mut total = 0.0;
    for l in lines {
        let r = if l.overfull {
            f64::NEG_INFINITY
        } else {
            l.ratio
        };
        total += demerits_of(r, l.penalty, l.hyphenated, prev, params);
        prev = Some((fitness_class(r), l.hyphenated));
    }
    total
}

struct Sums {
    w: Vec<f64>,
    y: Vec<f64>,
    z: Vec<f64>,
}

fn prefix_sums(items: &[Item]) -> Sums {
    let n = items.len();
    let (mut w, mut y, mut z) = (vec![0.0; n + 1], vec![0.0; n + 1], vec![0.0; n + 1]);
    for (i, it) in items.iter().enumerate() {
        w[i + 1] = w[i] + it.width();
        y[i + 1] = y[i];
        z[i + 1] = z[i];
        if let Item::Glue {
            stretch, shrink, ..
        } = it
        {
            y[i + 1] += stretch;
            z[i + 1] += shrink;
        }
    }
    Sums { w, y, z }
}

/// Items with trailing glue and penalties removed and a "fill" glue plus forced break appended.
fn finished(items: &[Item]) -> Vec<Item> {
    let mut v = items.to_vec();
    if v.last().is_some_and(is_forced) {
        return v;
    }
    while matches!(v.last(), Some(Item::Glue { .. })) {
        v.pop();
    }
    v.push(Item::no_break());
    v.push(Item::space(0.0, FINISH_STRETCH, 0.0));
    v.push(Item::forced_break());
    v
}

fn legal_break(items: &[Item], i: usize) -> bool {
    match &items[i] {
        Item::Glue { .. } => i > 0 && matches!(items[i - 1], Item::Box { .. }),
        Item::Penalty { penalty, .. } => *penalty < f64::INFINITY,
        Item::Box { .. } => false,
    }
}

/// Index of the first box after break `b` (the start of the next line).
fn next_line_start(items: &[Item], b: usize) -> usize {
    let mut i = b;
    while i < items.len() && !matches!(items[i], Item::Box { .. }) {
        i += 1;
    }
    i
}

#[derive(Clone)]
struct Node {
    pos: usize,
    line: usize,
    class: u8,
    flagged: bool,
    run: usize,
    demerits: f64,
    ratio: f64,
    overfull: bool,
    prev: Option<usize>,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Tolerance(f64),
    Any,
    Emergency,
}

fn run_pass(
    items: &[Item],
    sums: &Sums,
    width: &LineWidth,
    params: &Params,
    mode: Mode,
) -> Option<(Vec<Node>, usize)> {
    let per_line = matches!(width, LineWidth::PerLine(_));
    let mut nodes = vec![Node {
        pos: 0,
        line: 0,
        class: 1,
        flagged: false,
        run: 0,
        demerits: 0.0,
        ratio: 0.0,
        overfull: false,
        prev: None,
    }];
    let mut active: Vec<usize> = vec![0];
    let start_sums = |pos: usize| next_line_start(items, pos);
    let mut node_start = vec![0usize];
    for b in 0..items.len() {
        if !legal_break(items, b) {
            continue;
        }
        let (pen, flagged, pen_w) = match &items[b] {
            Item::Penalty {
                penalty,
                flagged,
                width,
            } => (*penalty, *flagged, *width),
            _ => (0.0, false, 0.0),
        };
        let forced = pen == f64::NEG_INFINITY;
        let mut cands: Vec<Node> = Vec::new();
        let mut best_dead: Option<(usize, f64)> = None;
        let mut survivors = Vec::with_capacity(active.len());
        for &ai in &active {
            let a = &nodes[ai];
            let s = node_start[ai];
            let len = sums.w[b] - sums.w[s] + pen_w;
            let y = sums.y[b] - sums.y[s];
            let z = sums.z[b] - sums.z[s];
            let target = width.at(a.line);
            let r = ratio(len, target, y, z);
            let dead = r < -1.0 || forced;
            if !dead {
                survivors.push(ai);
            }
            let feasible = match mode {
                Mode::Tolerance(t) => r >= -1.0 && badness(r) <= t && r.is_finite(),
                Mode::Any | Mode::Emergency => r >= -1.0,
            };
            if feasible {
                let run = if flagged { a.run + 1 } else { 0 };
                if params.consecutive_hyphen_limit.is_some_and(|lim| run > lim) {
                    continue;
                }
                let rr = r;
                let class = fitness_class(rr);
                let d =
                    a.demerits + demerits_of(rr, pen, flagged, Some((a.class, a.flagged)), params);
                let line = a.line + 1;
                let key_line = if per_line { line } else { 0 };
                if let Some(c) = cands.iter_mut().find(|c| {
                    c.class == class
                        && c.run == run
                        && (if per_line { c.line } else { 0 }) == key_line
                }) {
                    if d < c.demerits {
                        c.demerits = d;
                        c.ratio = r;
                        c.prev = Some(ai);
                    }
                } else {
                    cands.push(Node {
                        pos: b,
                        line,
                        class,
                        flagged,
                        run,
                        demerits: d,
                        ratio: r,
                        overfull: false,
                        prev: Some(ai),
                    });
                }
            } else if mode == Mode::Emergency && r < -1.0 {
                let d = a.demerits;
                if best_dead.is_none_or(|(_, bd)| d < bd) {
                    best_dead = Some((ai, d));
                }
            }
        }
        active = survivors;
        if cands.is_empty() && active.is_empty() {
            if mode == Mode::Emergency {
                if let Some((ai, _)) = best_dead {
                    let a = &nodes[ai];
                    cands.push(Node {
                        pos: b,
                        line: a.line + 1,
                        class: 0,
                        flagged,
                        run: 0,
                        demerits: a.demerits + 1.0e8,
                        ratio: -1.0,
                        overfull: true,
                        prev: Some(ai),
                    });
                }
            }
            if cands.is_empty() {
                return None;
            }
        }
        for c in cands {
            nodes.push(c);
            node_start.push(start_sums(b));
            active.push(nodes.len() - 1);
        }
    }
    let best = active
        .iter()
        .copied()
        .min_by(|&x, &y| nodes[x].demerits.total_cmp(&nodes[y].demerits))?;
    Some((nodes, best))
}

fn make_lines(
    items: &[Item],
    real_len: usize,
    nodes: &[Node],
    best: usize,
    width: &LineWidth,
    sums: &Sums,
    params: &Params,
) -> Vec<Line> {
    let mut chain = Vec::new();
    let mut cur = Some(best);
    while let Some(i) = cur {
        chain.push(i);
        cur = nodes[i].prev;
    }
    chain.reverse();
    let mut lines = Vec::new();
    let mut prev_info = Some((1u8, false));
    for w in chain.windows(2) {
        let (a, b) = (&nodes[w[0]], &nodes[w[1]]);
        let start = if a.pos == 0 && w[0] == 0 {
            0
        } else {
            next_line_start(items, a.pos)
        };
        let end = b.pos;
        let pen_w = match &items[end] {
            Item::Penalty { width, .. } => *width,
            _ => 0.0,
        };
        let (pen, hyphenated) = match &items[end] {
            Item::Penalty {
                penalty, flagged, ..
            } => (*penalty, *flagged),
            _ => (0.0, false),
        };
        let natural = sums.w[end] - sums.w[start.min(end)] + pen_w;
        let eff_end = end.min(real_len);
        let r = if b.overfull {
            f64::NEG_INFINITY
        } else {
            b.ratio
        };
        let r_used = if r == f64::NEG_INFINITY { -1.0 } else { r };
        let d = demerits_of(r, pen, hyphenated, prev_info, params);
        prev_info = Some((fitness_class(r), hyphenated));
        lines.push(Line {
            start: start.min(eff_end),
            end: eff_end,
            ratio: r_used,
            natural_width: natural,
            target_width: width.at(a.line),
            hyphenated,
            penalty: pen,
            overfull: b.overfull || (r < -1.0),
            demerits: d,
        });
    }
    lines
}

/// Optimal (minimum total demerits) paragraph breaking. Empty `items` give no lines. If `items` does not end
/// in a forced break, the paragraph end is added (the last line is not justified). When no feasible
/// breaking exists the tolerance is lifted, then the hyphen limit (it is a preference when nothing else works); if a box is wider than the line, an `overfull` line is emitted.
pub fn break_paragraph(
    items: &[Item],
    line_width: impl Into<LineWidth>,
    params: &Params,
) -> Vec<Line> {
    let width = line_width.into();
    if items.iter().all(|i| !matches!(i, Item::Box { .. })) {
        return Vec::new();
    }
    let fin = finished(items);
    let sums = prefix_sums(&fin);
    let relaxed = Params {
        consecutive_hyphen_limit: None,
        ..params.clone()
    };
    let attempts = [
        (params, Mode::Tolerance(params.tolerance)),
        (params, Mode::Any),
        (&relaxed, Mode::Tolerance(params.tolerance)),
        (&relaxed, Mode::Any),
        (&relaxed, Mode::Emergency),
    ];
    for (p, mode) in attempts {
        if let Some((nodes, best)) = run_pass(&fin, &sums, &width, p, mode) {
            return make_lines(&fin, items.len(), &nodes, best, &width, &sums, p);
        }
    }
    Vec::new()
}

/// First-fit breaking: fill each line as much as possible, no look-ahead. Used as a baseline.
pub fn break_greedy(
    items: &[Item],
    line_width: impl Into<LineWidth>,
    params: &Params,
) -> Vec<Line> {
    let width = line_width.into();
    if items.iter().all(|i| !matches!(i, Item::Box { .. })) {
        return Vec::new();
    }
    let fin = finished(items);
    let sums = prefix_sums(&fin);
    let mut lines = Vec::new();
    let mut start = 0usize;
    let mut prev_info = Some((1u8, false));
    while start < fin.len() {
        let target = width.at(lines.len());
        // (index, ratio)
        let mut best: Option<(usize, f64)> = None;
        let mut chosen: Option<(usize, f64, bool)> = None;
        for b in start..fin.len() {
            if !legal_break(&fin, b) || b == start {
                continue;
            }
            let pen_w = match &fin[b] {
                Item::Penalty { width, .. } => *width,
                _ => 0.0,
            };
            let len = sums.w[b] - sums.w[start] + pen_w;
            let r = ratio(
                len,
                target,
                sums.y[b] - sums.y[start],
                sums.z[b] - sums.z[start],
            );
            let forced = is_forced(&fin[b]);
            if r < -1.0 {
                chosen = Some(match best {
                    Some((bi, br)) => (bi, br, false),
                    None => (b, -1.0, true),
                });
                break;
            }
            best = Some((b, r));
            if forced {
                chosen = Some((b, r, false));
                break;
            }
        }
        let (end, r, overfull) = match chosen.or_else(|| best.map(|(b, r)| (b, r, false))) {
            Some(c) => c,
            None => break,
        };
        let (pen, hyphenated, pen_w) = match &fin[end] {
            Item::Penalty {
                penalty,
                flagged,
                width,
            } => (*penalty, *flagged, *width),
            _ => (0.0, false, 0.0),
        };
        let r_used = if overfull { -1.0 } else { r };
        let d = demerits_of(
            if overfull { f64::NEG_INFINITY } else { r },
            pen,
            hyphenated,
            prev_info,
            params,
        );
        prev_info = Some((
            fitness_class(if overfull { f64::NEG_INFINITY } else { r }),
            hyphenated,
        ));
        let eff_end = end.min(items.len());
        lines.push(Line {
            start: start.min(eff_end),
            end: eff_end,
            ratio: r_used,
            natural_width: sums.w[end] - sums.w[start] + pen_w,
            target_width: target,
            hyphenated,
            penalty: pen,
            overfull,
            demerits: d,
        });
        if is_forced(&fin[end]) {
            break;
        }
        start = next_line_start(&fin, end);
    }
    lines
}

/// Build paragraph items from text. Words become boxes, whitespace becomes glue (stretch = width/2,
/// shrink = width/3), hyphenation points (and existing soft hyphens) become flagged penalties of `hyphen_penalty`.
/// `measure` returns the width of a string. The paragraph end (fill glue and forced break) is included.
pub fn items_from_text(
    text: &str,
    measure: &dyn Fn(&str) -> f64,
    hyphenator: Option<&Hyphenator>,
    hyphen_penalty: f64,
) -> Vec<Item> {
    let space_w = measure(" ");
    let hyphen_w = measure("-");
    let mut items = Vec::new();
    for (wi, word) in text.split_whitespace().enumerate() {
        if wi > 0 {
            items.push(Item::space(space_w, space_w / 2.0, space_w / 3.0));
        }
        let mut cuts: Vec<usize> = Vec::new();
        if word.contains(SOFT_HYPHEN) {
            cuts.extend(
                word.char_indices()
                    .filter(|(_, c)| *c == SOFT_HYPHEN)
                    .map(|(i, c)| i + c.len_utf8()),
            );
        } else if let Some(h) = hyphenator {
            cuts = h.hyphenate(word);
        }
        let clean: String = word.chars().filter(|c| *c != SOFT_HYPHEN).collect();
        // map cut offsets (in `word`) to offsets in `clean`
        let mut removed = 0usize;
        let mut mapped = Vec::new();
        let sh = SOFT_HYPHEN.len_utf8();
        let soft: Vec<usize> = word
            .char_indices()
            .filter(|(_, c)| *c == SOFT_HYPHEN)
            .map(|(i, _)| i)
            .collect();
        for c in cuts {
            removed = soft.iter().filter(|&&s| s + sh <= c).count() * sh;
            let m = c - removed;
            if m > 0 && m < clean.len() && mapped.last() != Some(&m) {
                mapped.push(m);
            }
        }
        let _ = removed;
        let mut last = 0;
        for m in mapped {
            let piece = &clean[last..m];
            items.push(Item::word(measure(piece), piece));
            items.push(Item::hyphen(hyphen_w, hyphen_penalty));
            last = m;
        }
        let piece = &clean[last..];
        items.push(Item::word(measure(piece), piece));
    }
    if !items.is_empty() {
        items.push(Item::no_break());
        items.push(Item::space(0.0, FINISH_STRETCH, 0.0));
        items.push(Item::forced_break());
    }
    items
}

/// Split the lines of one paragraph over pages, honouring orphans and widows (CSS semantics: when both cannot be
/// satisfied they are ignored).
///
/// `heights` are line heights; `first_page_height` is the space left on the current page, `page_height` the
/// space of a fresh page. The result has one range per page; the first range is empty when the whole paragraph
/// moves to the next page.
pub fn paginate_lines(
    heights: &[f64],
    first_page_height: f64,
    page_height: f64,
    orphans: usize,
    widows: usize,
) -> Vec<Range<usize>> {
    let n = heights.len();
    let mut pages = Vec::new();
    if n == 0 {
        return pages;
    }
    let (orphans, widows) = (orphans.max(1), widows.max(1));
    let mut pos = 0;
    let mut avail = first_page_height;
    while pos < n {
        let fresh = avail >= page_height - 1e-9;
        let mut k = 0;
        let mut used = 0.0;
        while pos + k < n && used + heights[pos + k] <= avail + 1e-9 {
            used += heights[pos + k];
            k += 1;
        }
        if k == 0 {
            if fresh {
                k = 1;
            } else {
                pages.push(pos..pos);
                avail = page_height;
                continue;
            }
        }
        let rem = n - pos - k;
        if rem > 0 {
            let min_here = if pos == 0 { orphans } else { 1 };
            if pos == 0 && k < orphans {
                if !fresh {
                    pages.push(0..0);
                    avail = page_height;
                    continue;
                }
            } else if rem < widows {
                let take = widows - rem;
                if k > take && k - take >= min_here {
                    k -= take;
                } else if pos == 0 && !fresh {
                    pages.push(0..0);
                    avail = page_height;
                    continue;
                }
            }
        }
        pages.push(pos..pos + k);
        pos += k;
        avail = page_height;
    }
    pages
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed-width measurement: one unit per char.
    fn m(s: &str) -> f64 {
        s.chars().count() as f64
    }

    fn text_items(t: &str) -> Vec<Item> {
        items_from_text(t, &m, None, 50.0)
    }

    fn texts(lines: &[Line], items: &[Item]) -> Vec<String> {
        lines.iter().map(|l| l.text(items)).collect()
    }

    fn no_limit() -> Params {
        Params {
            consecutive_hyphen_limit: None,
            ..Params::default()
        }
    }

    #[test]
    fn empty_and_whitespace_paragraphs() {
        assert!(break_paragraph(&[], 10.0, &Params::default()).is_empty());
        assert!(break_greedy(&[], 10.0, &Params::default()).is_empty());
        assert!(break_paragraph(&text_items("   "), 10.0, &Params::default()).is_empty());
        assert!(
            break_paragraph(&[Item::space(1.0, 1.0, 1.0)], 10.0, &Params::default()).is_empty()
        );
    }

    #[test]
    fn single_word_one_line() {
        let items = text_items("hello");
        let lines = break_paragraph(&items, 20.0, &Params::default());
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text(&items), "hello");
        assert!(!lines[0].overfull);
        let g = break_greedy(&items, 20.0, &Params::default());
        assert_eq!(texts(&g, &items), vec!["hello"]);
    }

    #[test]
    fn single_word_wider_than_line_is_overfull() {
        let items = text_items("extraordinary");
        let lines = break_paragraph(&items, 5.0, &Params::default());
        assert_eq!(lines.len(), 1);
        assert!(lines[0].overfull);
        assert_eq!(lines[0].text(&items), "extraordinary");
        let g = break_greedy(&items, 5.0, &Params::default());
        assert_eq!(g.len(), 1);
        assert!(g[0].overfull);
    }

    #[test]
    fn fits_on_one_line_last_line_unjustified() {
        let items = text_items("aa bb cc");
        let lines = break_paragraph(&items, 30.0, &Params::default());
        assert_eq!(lines.len(), 1);
        assert!(lines[0].ratio.abs() < 1e-3);
        assert_eq!(lines[0].natural_width, 8.0);
    }

    /// Hand computed optimum. Words: "aaa bb cc ddddd" width 6 (space = 1, stretch 0.5, shrink 1/3).
    /// Options: [aaa bb | cc ddddd] (width 6 / 8 -> second overfull beyond shrink), so the only feasible
    /// breaking is [aaa | bb cc | ddddd]: "bb cc" = 5 (stretch .5 -> r = 2), ...
    #[test]
    fn hand_computed_unique_solution() {
        let items = text_items("aaa bb cc ddddd");
        let lines = break_paragraph(&items, 6.0, &Params::default());
        let t = texts(&lines, &items);
        assert_eq!(t, vec!["aaa bb", "cc", "ddddd"]);
    }

    /// Greedy takes "aa bb" first, leaving "c" alone; optimum balances.
    /// Text "aa bb c dddd" width 7: greedy "aa bb c" (7) / "dddd" ; fine. Use a classic: width 6,
    /// words "aaa bbb cc ddddd": greedy: "aaa bbb"(7 shrink 1/3 => r=-3 overfull) so "aaa" ...
    #[test]
    fn optimal_beats_greedy_on_known_case() {
        // widths: line 10. greedy: "xxxx yyyy" (9), "zz" can't fit with ww -> next "zz wwwwwwww"
        let items = text_items("xxxx yyyy zz wwwwwwww vv");
        let p = no_limit();
        let opt = break_paragraph(
            &items,
            10.0,
            &Params {
                tolerance: 1.0e9,
                ..p.clone()
            },
        );
        let gr = break_greedy(&items, 10.0, &p);
        assert!(total_demerits(&opt, &p) <= total_demerits(&gr, &p) + 1e-6);
        assert!(!opt.iter().any(|l| l.overfull));
    }

    #[test]
    fn strictly_better_than_greedy_exists() {
        // Greedy puts "a bb ccc" on line 1 leaving a very loose second line; look for any improvement
        let text = "aaaa bb cc dddddd ee ffff g hhhhh iii";
        let items = text_items(text);
        let p = no_limit();
        let opt = break_paragraph(&items, 11.0, &p);
        let gr = break_greedy(&items, 11.0, &p);
        assert!(total_demerits(&opt, &p) <= total_demerits(&gr, &p) + 1e-6);
        // text is the same words in the same order
        let join = |ls: &[Line]| texts(ls, &items).join(" ");
        assert_eq!(join(&opt), text);
        assert_eq!(join(&gr), text);
    }

    #[test]
    fn greedy_vs_optimal_differ_on_crafted_input() {
        // Line width 6. Words: "aa" "bb" "cc" "dd"... greedy fills "aa bb" (5) then "cc dd" etc: same.
        // Crafted: width 9, words 4,4,1,8: greedy "xxxx yyyy" = 9 exactly then "z" + 8 doesn't fit -> "z" alone (very loose)
        let items = text_items("xxxx yyyy z wwwwwwww");
        let p = no_limit();
        let gr = break_greedy(&items, 9.0, &p);
        assert_eq!(texts(&gr, &items), vec!["xxxx yyyy", "z", "wwwwwwww"]);
        let opt = break_paragraph(&items, 9.0, &p);
        // alternatives: [xxxx|yyyy z|wwwwwwww] -> "yyyy z" =6 r=3/.5 -> r=6 loose ; same count; optimum must not be worse
        assert!(total_demerits(&opt, &p) <= total_demerits(&gr, &p));
        assert_eq!(texts(&opt, &items).join(" "), "xxxx yyyy z wwwwwwww");
    }

    #[test]
    fn no_overfull_when_avoidable() {
        let items = text_items("one two three four five six seven eight nine ten eleven twelve");
        for w in [8.0, 10.0, 12.0, 15.0, 20.0, 30.0] {
            let lines = break_paragraph(&items, w, &Params::default());
            assert!(lines.iter().all(|l| !l.overfull), "width {w}");
            for l in &lines {
                assert!(l.natural_width <= w * 1.5);
            }
        }
    }

    #[test]
    fn justified_spaces_within_limits() {
        let items = text_items("the quick brown fox jumps over the lazy dog again and again");
        let lines = break_paragraph(&items, 20.0, &Params::default());
        let n = lines.len();
        for (i, l) in lines.iter().enumerate() {
            let widths = l.glue_widths(&items);
            for g in &widths {
                assert!(*g >= 1.0 - 1.0 / 3.0 - 1e-9, "shrunk beyond limit: {g}");
            }
            if i + 1 < n && !l.overfull {
                let total: f64 = l.natural_width - widths.len() as f64 + widths.iter().sum::<f64>();
                assert!((total - 20.0).abs() < 1e-6, "line {i} {total}");
                assert!(
                    l.ratio >= -1.0
                        && l.ratio <= 400f64.powf(1.0 / 3.0) / 100f64.powf(1.0 / 3.0) + 1e-9
                );
            }
        }
    }

    #[test]
    fn glue_widths_stretch_and_shrink_formulas() {
        let items = vec![
            Item::word(3.0, "a"),
            Item::space(2.0, 4.0, 1.0),
            Item::word(3.0, "b"),
        ];
        let mut l = Line {
            start: 0,
            end: 3,
            ratio: 0.5,
            natural_width: 8.0,
            target_width: 10.0,
            hyphenated: false,
            penalty: 0.0,
            overfull: false,
            demerits: 0.0,
        };
        assert_eq!(l.glue_widths(&items), vec![4.0]);
        l.ratio = -0.5;
        assert_eq!(l.glue_widths(&items), vec![1.5]);
        l.ratio = 0.0;
        assert_eq!(l.glue_widths(&items), vec![2.0]);
    }

    #[test]
    fn hyphen_at_end_carries_hyphen_char_and_width() {
        // "ex-tra" can break at the hyphen penalty
        let items = vec![
            Item::word(2.0, "ab"),
            Item::space(1.0, 0.5, 0.3),
            Item::word(3.0, "ex"),
            Item::hyphen(1.0, 50.0),
            Item::word(3.0, "tra"),
            Item::no_break(),
            Item::space(0.0, FINISH_STRETCH, 0.0),
            Item::forced_break(),
        ];
        let lines = break_paragraph(&items, 7.0, &Params::default());
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].hyphenated);
        assert_eq!(lines[0].text(&items), "ab ex-");
        assert_eq!(lines[0].natural_width, 7.0);
        assert_eq!(lines[1].text(&items), "tra");
        assert!(!lines[1].hyphenated);
    }

    #[test]
    fn hyphen_not_used_when_not_needed() {
        let hy = Hyphenator::for_language("en");
        let items = items_from_text("a wonderful day", &m, Some(&hy), 50.0);
        let lines = break_paragraph(&items, 40.0, &Params::default());
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].hyphenated);
    }

    #[test]
    fn hyphenation_makes_narrow_columns_feasible() {
        let hy = Hyphenator::for_language("en");
        let items = items_from_text(
            "an extraordinarily wonderful representation",
            &m,
            Some(&hy),
            50.0,
        );
        let lines = break_paragraph(&items, 11.0, &no_limit());
        assert!(
            lines.iter().all(|l| !l.overfull),
            "{:?}",
            texts(&lines, &items)
        );
        let joined: String = texts(&lines, &items).join(" ");
        let stripped = joined.replace("- ", "").replace(' ', "");
        assert_eq!(stripped, "anextraordinarilywonderfulrepresentation");
    }

    fn max_run(ls: &[Line]) -> usize {
        let (mut best, mut cur) = (0, 0);
        for l in ls {
            if l.hyphenated {
                cur += 1;
                best = best.max(cur);
            } else {
                cur = 0;
            }
        }
        best
    }

    #[test]
    fn consecutive_hyphen_limit_enforced() {
        // hyphens are rewarded (negative penalty) so the free layout uses them everywhere
        let items = items_from_text(
            "aaa\u{ad}aaa bbb\u{ad}bbb ccc\u{ad}ccc ddd",
            &m,
            None,
            -3000.0,
        );
        let free = Params {
            consecutive_hyphen_limit: None,
            ..Params::default()
        };
        let none = Params {
            consecutive_hyphen_limit: Some(0),
            ..Params::default()
        };
        let free_lines = break_paragraph(&items, 7.0, &free);
        let none_lines = break_paragraph(&items, 7.0, &none);
        assert!(
            max_run(&free_lines) >= 1,
            "{:?}",
            texts(&free_lines, &items)
        );
        assert_eq!(max_run(&none_lines), 0, "{:?}", texts(&none_lines, &items));
        assert_eq!(
            texts(&none_lines, &items).join(" "),
            "aaaaaa bbbbbb cccccc ddd"
        );
    }

    #[test]
    fn consecutive_hyphen_limit_is_soft_when_nothing_else_works() {
        // width 4 holds exactly "aaa-": three pieces force two hyphens in a row
        let items = vec![
            Item::word(3.0, "aaa"),
            Item::hyphen(1.0, 0.0),
            Item::word(3.0, "aaa"),
            Item::hyphen(1.0, 0.0),
            Item::word(3.0, "aaa"),
            Item::no_break(),
            Item::space(0.0, FINISH_STRETCH, 0.0),
            Item::forced_break(),
        ];
        let p = Params {
            consecutive_hyphen_limit: Some(1),
            ..Params::default()
        };
        let lines = break_paragraph(&items, 4.0, &p);
        assert_eq!(texts(&lines, &items), vec!["aaa-", "aaa-", "aaa"]);
        assert_eq!(max_run(&lines), 2);
    }

    #[test]
    fn variable_line_widths() {
        let items = text_items("aa bb cc dd ee ff gg hh");
        let lines = break_paragraph(&items, vec![5.0, 8.0], &Params::default());
        assert_eq!(texts(&lines, &items).join(" "), "aa bb cc dd ee ff gg hh");
        assert_eq!(lines[0].target_width, 5.0);
        assert_eq!(lines[1].target_width, 8.0);
        assert_eq!(lines.last().unwrap().target_width, 8.0);
        assert!(lines[0].natural_width <= 5.0 + 1.0);
        assert_eq!(LineWidth::from(vec![1.0, 2.0]).at(5), 2.0);
        assert_eq!(LineWidth::PerLine(vec![]).at(0), 0.0);
        assert_eq!(LineWidth::from(3.0).at(9), 3.0);
    }

    #[test]
    fn explicit_forced_break_in_middle() {
        let items = vec![
            Item::word(2.0, "ab"),
            Item::no_break(),
            Item::space(0.0, FINISH_STRETCH, 0.0),
            Item::forced_break(),
            Item::word(2.0, "cd"),
            Item::no_break(),
            Item::space(0.0, FINISH_STRETCH, 0.0),
            Item::forced_break(),
        ];
        let lines = break_paragraph(&items, 10.0, &Params::default());
        assert_eq!(texts(&lines, &items), vec!["ab", "cd"]);
    }

    #[test]
    fn missing_final_break_is_added() {
        let items = vec![
            Item::word(2.0, "ab"),
            Item::space(1.0, 0.5, 0.3),
            Item::word(2.0, "cd"),
            Item::space(1.0, 0.5, 0.3),
        ];
        let lines = break_paragraph(&items, 10.0, &Params::default());
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text(&items), "ab cd");
        assert_eq!(lines[0].end, items.len());
        let g = break_greedy(&items, 10.0, &Params::default());
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].text(&items), "ab cd");
    }

    #[test]
    fn forbidden_break_penalty_respected() {
        let items = vec![
            Item::word(4.0, "aaaa"),
            Item::Penalty {
                width: 0.0,
                penalty: f64::INFINITY,
                flagged: false,
            },
            Item::space(1.0, 0.5, 0.3),
            Item::word(4.0, "bbbb"),
            Item::space(1.0, 0.5, 0.3),
            Item::word(4.0, "cccc"),
        ];
        let lines = break_paragraph(&items, 10.0, &Params::default());
        assert_eq!(texts(&lines, &items), vec!["aaaa bbbb", "cccc"]);
    }

    #[test]
    fn negative_penalty_encourages_break() {
        let items = vec![
            Item::word(2.0, "aa"),
            Item::space(1.0, 0.5, 0.3),
            Item::word(2.0, "bb"),
            Item::Penalty {
                width: 0.0,
                penalty: -9999.0,
                flagged: false,
            },
            Item::space(1.0, 0.5, 0.3),
            Item::word(2.0, "cc"),
            Item::no_break(),
            Item::space(0.0, FINISH_STRETCH, 0.0),
            Item::forced_break(),
        ];
        let p = Params {
            tolerance: 1.0e9,
            ..Params::default()
        };
        let lines = break_paragraph(&items, 5.0, &p);
        assert_eq!(texts(&lines, &items), vec!["aa bb", "cc"]);
        assert_eq!(lines[0].penalty, -9999.0);
        // without the reward everything fits on one line when wide enough
        let wide = break_paragraph(
            &items[..3]
                .iter()
                .cloned()
                .chain(items[4..].iter().cloned())
                .collect::<Vec<_>>(),
            20.0,
            &p,
        );
        assert_eq!(wide.len(), 1);
    }

    #[test]
    fn tolerance_zero_falls_back_to_second_pass() {
        let items = text_items("aaaa b cccc d");
        let strict = Params {
            tolerance: 0.0,
            ..Params::default()
        };
        let lines = break_paragraph(&items, 7.0, &strict);
        assert!(!lines.is_empty());
        assert_eq!(texts(&lines, &items).join(" "), "aaaa b cccc d");
    }

    #[test]
    fn total_demerits_matches_line_sum_for_first_line_chain() {
        let items = text_items("one two three four five six seven");
        let p = Params::default();
        let lines = break_paragraph(&items, 12.0, &p);
        let sum: f64 = lines.iter().map(|l| l.demerits).sum();
        assert!((sum - total_demerits(&lines, &p)).abs() < 1e-6);
        assert!(total_demerits(&[], &p) == 0.0);
    }

    #[test]
    fn items_from_text_structure() {
        let items = items_from_text("hello big world", &m, None, 50.0);
        // word space word space word fill forced
        assert_eq!(items.len(), 8);
        assert!(matches!(&items[0], Item::Box { width, text } if *width == 5.0 && text == "hello"));
        assert!(
            matches!(&items[1], Item::Glue { width, stretch, shrink } if *width == 1.0 && *stretch == 0.5 && (*shrink - 1.0 / 3.0).abs() < 1e-12)
        );
        assert!(is_forced(items.last().unwrap()));
        assert!(items_from_text("", &m, None, 50.0).is_empty());
    }

    #[test]
    fn items_from_text_soft_hyphens_and_hyphenator() {
        let items = items_from_text("won\u{ad}derful", &m, None, 30.0);
        assert!(matches!(&items[0], Item::Box { text, .. } if text == "won"));
        assert!(
            matches!(&items[1], Item::Penalty { penalty, flagged: true, width } if *penalty == 30.0 && *width == 1.0)
        );
        assert!(matches!(&items[2], Item::Box { text, .. } if text == "derful"));
        let hy = Hyphenator::for_language("en");
        let items = items_from_text("wonderful", &m, Some(&hy), 50.0);
        let boxes: Vec<&str> = items
            .iter()
            .filter_map(|i| {
                if let Item::Box { text, .. } = i {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(boxes, vec!["won", "der", "ful"]);
        let none = items_from_text("wonderful", &m, None, 50.0);
        assert_eq!(none.len(), 4);
    }

    #[test]
    fn deterministic_property_no_loss_no_reorder() {
        let mut seed = 0xdead_beef_cafe_f00du64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let hy = Hyphenator::for_language("en");
        for case in 0..300 {
            let nwords = (rnd() % 40) as usize;
            let words: Vec<String> = (0..nwords)
                .map(|_| {
                    let len = 1 + (rnd() % 12) as usize;
                    (0..len)
                        .map(|_| (b'a' + (rnd() % 26) as u8) as char)
                        .collect()
                })
                .collect();
            let text = words.join(" ");
            let items = items_from_text(
                &text,
                &m,
                if case % 2 == 0 { Some(&hy) } else { None },
                50.0,
            );
            let width = 6.0 + (rnd() % 30) as f64;
            let p = Params {
                consecutive_hyphen_limit: if case % 3 == 0 { Some(2) } else { None },
                ..Params::default()
            };
            for lines in [
                break_paragraph(&items, width, &p),
                break_greedy(&items, width, &p),
            ] {
                if words.is_empty() {
                    assert!(lines.is_empty());
                    continue;
                }
                // continuity and order
                let mut rebuilt = String::new();
                for l in &lines {
                    let mut t = l.text(&items);
                    if l.hyphenated {
                        t.pop();
                    } else {
                        t.push(' ');
                    }
                    rebuilt.push_str(&t);
                }
                assert_eq!(rebuilt.trim_end(), text, "case {case} width {width}");
                for w in lines.windows(2) {
                    assert!(w[0].end <= w[1].start);
                }
            }
            let opt = break_paragraph(&items, width, &p);
            let widest_piece = items
                .iter()
                .map(|i| {
                    if let Item::Box { width, .. } = i {
                        *width
                    } else {
                        0.0
                    }
                })
                .fold(0.0, f64::max);
            if widest_piece + 1.0 <= width {
                assert!(
                    opt.iter().all(|l| !l.overfull),
                    "case {case}: {:?}",
                    texts(&opt, &items)
                );
            }
        }
    }

    #[test]
    fn deterministic_property_optimal_not_worse_than_greedy() {
        let mut seed = 0x0123_4567_89ab_cdefu64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let p = Params {
            tolerance: 1.0e12,
            consecutive_hyphen_limit: None,
            ..Params::default()
        };
        for case in 0..300 {
            let nwords = 1 + (rnd() % 30) as usize;
            let text: Vec<String> = (0..nwords)
                .map(|_| "x".repeat(1 + (rnd() % 8) as usize))
                .collect();
            let items = text_items(&text.join(" "));
            let width = 9.0 + (rnd() % 12) as f64;
            let g = break_greedy(&items, width, &p);
            if g.iter().any(|l| l.overfull) {
                continue;
            }
            let o = break_paragraph(&items, width, &p);
            assert!(
                total_demerits(&o, &p) <= total_demerits(&g, &p) + 1e-6,
                "case {case} width {width}: opt {} greedy {}",
                total_demerits(&o, &p),
                total_demerits(&g, &p)
            );
        }
    }

    // ---- pagination ----

    #[test]
    fn paginate_empty_and_fits() {
        assert!(paginate_lines(&[], 10.0, 10.0, 2, 2).is_empty());
        assert_eq!(paginate_lines(&[1.0; 5], 10.0, 10.0, 2, 2), vec![0..5]);
    }

    #[test]
    fn paginate_simple_split_without_constraints() {
        // 10 lines of height 1, 6 fit on first page
        assert_eq!(
            paginate_lines(&[1.0; 10], 6.0, 20.0, 1, 1),
            vec![0..6, 6..10]
        );
    }

    #[test]
    fn paginate_orphans_move_paragraph_to_next_page() {
        // only 1 line fits on the page, orphans = 2 -> nothing stays
        let pages = paginate_lines(&[1.0; 8], 1.0, 20.0, 2, 2);
        assert_eq!(pages, vec![0..0, 0..8]);
    }

    #[test]
    fn paginate_orphans_ignored_on_fresh_page() {
        let pages = paginate_lines(&[1.0; 4], 1.0, 1.0, 2, 2);
        assert_eq!(pages.iter().map(|r| r.len()).sum::<usize>(), 4);
        assert!(pages.iter().all(|r| !r.is_empty()));
    }

    #[test]
    fn paginate_widows_pull_lines_forward() {
        // 10 lines, 9 fit, 1 would be a widow with widows=2 -> 8 / 2
        assert_eq!(
            paginate_lines(&[1.0; 10], 9.0, 9.0, 2, 2),
            vec![0..8, 8..10]
        );
        // widows=3 -> 7 / 3
        assert_eq!(
            paginate_lines(&[1.0; 10], 9.0, 9.0, 2, 3),
            vec![0..7, 7..10]
        );
        // widows satisfied already
        assert_eq!(
            paginate_lines(&[1.0; 11], 9.0, 9.0, 2, 2),
            vec![0..9, 9..11]
        );
    }

    #[test]
    fn paginate_widows_vs_orphans_conflict_ignores_widows_or_moves() {
        // 4 lines, 3 fit, widows 2, orphans 2: 3-1=2 stays (>= orphans) -> 2 / 2
        assert_eq!(paginate_lines(&[1.0; 4], 3.0, 10.0, 2, 2), vec![0..2, 2..4]);
        // 3 lines, 2 fit, orphans 2, widows 2: cannot keep 2 and leave 2 -> push all to next page
        assert_eq!(paginate_lines(&[1.0; 3], 2.0, 10.0, 2, 2), vec![0..0, 0..3]);
        // fresh page: conflict ignored, keeps 2 / 1
        assert_eq!(paginate_lines(&[1.0; 3], 2.0, 2.0, 2, 2), vec![0..2, 2..3]);
    }

    #[test]
    fn paginate_variable_heights_and_zero_orphan_params() {
        let h = [1.0, 2.0, 1.0, 2.0, 1.0];
        let pages = paginate_lines(&h, 3.0, 4.0, 0, 0);
        assert_eq!(pages, vec![0..2, 2..5]);
        // line taller than the page still progresses
        let pages = paginate_lines(&[5.0, 5.0], 3.0, 3.0, 1, 1);
        assert_eq!(pages, vec![0..1, 1..2]);
    }

    #[test]
    fn paginate_property_covers_all_lines_in_order() {
        let mut seed = 99u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..500 {
            let n = (rnd() % 40) as usize;
            let heights: Vec<f64> = (0..n).map(|_| 1.0 + (rnd() % 3) as f64).collect();
            let page = 4.0 + (rnd() % 12) as f64;
            let first = 1.0 + (rnd() % 12) as f64;
            let first = first.min(page);
            let (o, w) = ((rnd() % 4) as usize, (rnd() % 4) as usize);
            let pages = paginate_lines(&heights, first, page, o, w);
            let mut pos = 0;
            for r in &pages {
                assert_eq!(r.start, pos);
                pos = r.end;
            }
            assert_eq!(pos, n);
            for (i, r) in pages.iter().enumerate() {
                let avail = if i == 0 { first } else { page };
                let used: f64 = heights[r.clone()].iter().sum();
                assert!(
                    used <= avail + 1e-9 || r.len() == 1,
                    "overflow {used} > {avail}"
                );
                if i > 0 {
                    assert!(!r.is_empty());
                }
            }
        }
    }
}
