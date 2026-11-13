//! Layout and rules of the book map (enhancement 18), without GTK: the whole book as a grid of
//! cells (a cell is one page, or several when the book is zoomed out), what is drawn on each cell
//! (time bars, markers, chapter starts) and how keys, wheel and pointer move around.
//!
//! The widget draws only the cells in view, so a 3000-page book costs the same as a small one.

use std::collections::BTreeMap;
use std::ops::Range;

/// How much of the book is in view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// The whole book fits.
    Book,
    /// In between.
    Section,
    /// One page per cell.
    Detail,
}

impl Level {
    pub const ALL: [Level; 3] = [Level::Book, Level::Section, Level::Detail];

    pub fn index(self) -> usize {
        match self {
            Level::Book => 0,
            Level::Section => 1,
            Level::Detail => 2,
        }
    }

    pub fn from_index(i: usize) -> Level {
        Level::ALL[i.min(2)]
    }

    /// One step towards single pages.
    pub fn deeper(self) -> Level {
        Level::from_index(self.index() + 1)
    }

    /// One step towards the whole book.
    pub fn shallower(self) -> Level {
        Level::from_index(self.index().saturating_sub(1))
    }

    pub fn label(self) -> &'static str {
        match self {
            Level::Book => "Whole book",
            Level::Section => "Sections",
            Level::Detail => "Single pages",
        }
    }
}

/// Pages in one cell for a grid of `cols` × `rows` cells.
pub fn pages_per_cell(total: usize, cols: usize, rows: usize, level: Level) -> usize {
    let capacity = (cols * rows).max(1);
    let fit = total.div_ceil(capacity).max(1);
    match level {
        Level::Book => fit,
        Level::Section => ((fit as f64).sqrt().round() as usize).max(1),
        Level::Detail => 1,
    }
}

/// The book laid out in cells, and the part of it in view.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub total: usize,
    pub cols: usize,
    pub rows: usize,
    pub level: Level,
    pub ppc: usize,
    /// The first row in view.
    pub first_row: usize,
}

impl Grid {
    pub fn new(total: usize, cols: usize, rows: usize, level: Level) -> Grid {
        let (cols, rows) = (cols.max(1), rows.max(1));
        Grid {
            total,
            cols,
            rows,
            level,
            ppc: pages_per_cell(total, cols, rows, level),
            first_row: 0,
        }
    }

    pub fn cells(&self) -> usize {
        self.total.div_ceil(self.ppc)
    }

    pub fn total_rows(&self) -> usize {
        self.cells().div_ceil(self.cols)
    }

    fn max_first(&self) -> usize {
        self.total_rows().saturating_sub(self.rows)
    }

    /// The cell a page is in.
    pub fn cell_of(&self, page: usize) -> usize {
        page.min(self.total.saturating_sub(1)) / self.ppc
    }

    /// The pages of a cell.
    pub fn pages_of(&self, cell: usize) -> Range<usize> {
        let a = (cell * self.ppc).min(self.total);
        a..((cell + 1) * self.ppc).min(self.total)
    }

    /// Row and column (absolute row) of a cell.
    pub fn row_col(&self, cell: usize) -> (usize, usize) {
        (cell / self.cols, cell % self.cols)
    }

    /// Where a page is on screen: row in view and column, `None` when it is out of view.
    pub fn view_pos(&self, page: usize) -> Option<(usize, usize)> {
        let (row, col) = self.row_col(self.cell_of(page));
        (row >= self.first_row && row < self.first_row + self.rows)
            .then(|| (row - self.first_row, col))
    }

    /// The page at a position in view (the first page of the cell); `None` outside the book.
    pub fn page_at(&self, view_row: usize, col: usize) -> Option<usize> {
        if col >= self.cols || view_row >= self.rows {
            return None;
        }
        let cell = (self.first_row + view_row) * self.cols + col;
        (cell < self.cells()).then(|| cell * self.ppc)
    }

    /// The cells in view.
    pub fn visible_cells(&self) -> Range<usize> {
        let a = (self.first_row * self.cols).min(self.cells());
        a..((self.first_row + self.rows) * self.cols).min(self.cells())
    }

    pub fn scroll_rows(&mut self, rows: i32) {
        let to = self.first_row as i64 + rows as i64;
        self.first_row = to.clamp(0, self.max_first() as i64) as usize;
    }

    /// Scrolls the least that shows `page`.
    pub fn reveal(&mut self, page: usize) {
        let row = self.row_col(self.cell_of(page)).0;
        if row < self.first_row {
            self.first_row = row;
        } else if row >= self.first_row + self.rows {
            self.first_row = row + 1 - self.rows;
        }
        self.first_row = self.first_row.min(self.max_first());
    }

    /// Scrolls so that `page` is in the middle.
    pub fn centre_on(&mut self, page: usize) {
        let row = self.row_col(self.cell_of(page)).0;
        self.first_row = row.saturating_sub(self.rows / 2).min(self.max_first());
    }

    /// Changes the zoom keeping `page` in view.
    pub fn set_level(&mut self, level: Level, page: usize) {
        self.level = level;
        self.ppc = pages_per_cell(self.total, self.cols, self.rows, level);
        self.centre_on(page);
    }

    /// The same book in a grid of another size (the dialog was resized).
    pub fn resized(&self, cols: usize, rows: usize, page: usize) -> Grid {
        let mut g = Grid::new(self.total, cols, rows, self.level);
        g.centre_on(page);
        g
    }

    /// `(index, count)` of the zoom level, for the level indicator.
    pub fn zoom_indicator(&self) -> (usize, usize) {
        (self.level.index(), Level::ALL.len())
    }

    /// `(start, size)` of the part in view as fractions of the book, for the extent indicator.
    pub fn extent_indicator(&self) -> (f64, f64) {
        let rows = self.total_rows().max(1) as f64;
        (
            self.first_row as f64 / rows,
            (self.rows as f64 / rows).min(1.0),
        )
    }
}

// ---- time bars ----

/// Bar height 0..1 of a page where `secs` were spent; `cap` is the time of a full bar. A page that
/// was read at all gets at least a stub, and the height grows slowly with the time.
pub fn bar_height(secs: f32, cap: f32) -> f32 {
    if secs <= 0.0 || !secs.is_finite() {
        return 0.0;
    }
    let cap = cap.max(1.0);
    let t = ((1.0 + secs).ln() / (1.0 + cap).ln()).clamp(0.0, 1.0);
    0.2 + 0.8 * t
}

/// The time of a full bar: a bit above what most read pages took (the 90th percentile), at least 15 s.
pub fn full_bar_secs(per_page: &[f32]) -> f32 {
    let mut v: Vec<f32> = per_page.iter().copied().filter(|s| *s > 0.0).collect();
    if v.is_empty() {
        return 15.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let i = ((v.len() as f64 * 0.9) as usize).min(v.len() - 1);
    v[i].max(15.0)
}

/// The bar of a cell: the tallest of its pages.
pub fn cell_bar(g: &Grid, per_page: &[f32], cap: f32, cell: usize) -> f32 {
    g.pages_of(cell)
        .map(|p| bar_height(per_page.get(p).copied().unwrap_or(0.0), cap))
        .fold(0.0, f32::max)
}

// ---- markers ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MarkerKind {
    Bookmark,
    Highlight,
    Note,
    /// A place you jumped from; 1 is the last one (where "go back" returns to).
    History(u8),
}

pub type Rgb8 = [u8; 3];

/// How a highlight is drawn in the text: a fill, an underline or a line through the words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MarkStyle {
    Fill,
    Underline,
    Strike,
}

impl MarkStyle {
    /// From the style name stored with a highlight (`background`, `underline`, `squiggly`,
    /// `strikethrough`, `invert`; anything else is a fill).
    pub fn from_name(name: &str) -> MarkStyle {
        match name {
            "underline" | "squiggly" => MarkStyle::Underline,
            "strikethrough" => MarkStyle::Strike,
            _ => MarkStyle::Fill,
        }
    }
}

/// The colour and style of a highlight or note, as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Swatch {
    pub rgb: Rgb8,
    pub style: MarkStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marker {
    pub page: usize,
    pub kind: MarkerKind,
    pub swatch: Option<Swatch>,
}

impl Marker {
    pub fn new(page: usize, kind: MarkerKind) -> Marker {
        Marker {
            page,
            kind,
            swatch: None,
        }
    }

    pub fn coloured(page: usize, kind: MarkerKind, swatch: Swatch) -> Marker {
        Marker {
            page,
            kind,
            swatch: Some(swatch),
        }
    }
}

/// Numbered markers for the places in the jump history, `back` oldest first. Only the last
/// `limit` places are shown; the newest is 1.
pub fn history_markers(back: &[usize], limit: usize) -> Vec<Marker> {
    back.iter()
        .rev()
        .take(limit.min(99))
        .enumerate()
        .map(|(i, &page)| Marker::new(page, MarkerKind::History(i as u8 + 1)))
        .collect()
}

/// How many notes a cell shows side by side.
pub const MAX_NOTES: usize = 3;

/// What is drawn inside one cell.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CellMarks {
    pub bookmark: bool,
    /// Every highlight of the cell, one line each, in the order of their place in the book.
    pub highlights: Vec<Swatch>,
    /// Distinct note colours (a pencil each).
    pub notes: Vec<Swatch>,
    /// The lowest number of the places jumped from that are in this cell.
    pub history: Option<u8>,
}

impl CellMarks {
    pub fn is_empty(&self) -> bool {
        *self == CellMarks::default()
    }
}

/// The marks of every cell that has any. Markers are taken in book order (a stable order for
/// markers on the same page); a highlight without a colour counts as `fallback`.
pub fn cell_summary(g: &Grid, markers: &[Marker], fallback: Swatch) -> BTreeMap<usize, CellMarks> {
    let mut sorted: Vec<&Marker> = markers.iter().collect();
    sorted.sort_by_key(|m| m.page);
    let mut out: BTreeMap<usize, CellMarks> = BTreeMap::new();
    for m in sorted {
        let c = out.entry(g.cell_of(m.page)).or_default();
        let sw = m.swatch.unwrap_or(fallback);
        match m.kind {
            MarkerKind::Bookmark => c.bookmark = true,
            MarkerKind::Highlight => c.highlights.push(sw),
            MarkerKind::Note => {
                if !c.notes.contains(&sw) && c.notes.len() < MAX_NOTES {
                    c.notes.push(sw);
                }
            }
            MarkerKind::History(n) => c.history = Some(c.history.map_or(n, |h| h.min(n))),
        }
    }
    out
}

// ---- geometry of a block and what lies in and around it ----

/// Room above a block for the chapter's title (and a bookmark flag).
pub const LABEL_H: f64 = 15.0;
pub const BLOCK_H: f64 = 30.0;
/// Room below a block for the pencils of notes and the pill of the current page.
pub const ROW_GAP: f64 = 16.0;
pub const ROW_H: f64 = LABEL_H + BLOCK_H + ROW_GAP;
/// Radius of the halo around a pencil.
pub const PENCIL_R: f64 = 4.5;

/// A block on screen (device independent pixels).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Block {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Block {
    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }
}

/// The `i`-th highlight line of a block: `(x, y, width)`. One pixel high, as wide as the block
/// (clip it to the block's rounded corners), a pixel of gap between two.
pub fn highlight_line(b: &Block, i: usize) -> (f64, f64, f64) {
    (b.x, b.y + 3.0 + i as f64 * 2.0, b.w)
}

/// Room for highlight lines inside a block.
pub fn highlight_room() -> f64 {
    BLOCK_H - 3.0 - 3.0
}

/// The middle of the `i`-th pencil of a block: below the block, from its end corner inwards.
pub fn pencil_centre(b: &Block, i: usize, rtl: bool) -> (f64, f64) {
    let off = 3.5 + i as f64 * 7.0;
    let x = if rtl { b.x + off } else { b.x + b.w - off };
    (x, b.bottom() + 9.5)
}

/// The pill under the page you are on: `(x, y, w, h)`.
pub fn pill_rect(b: &Block) -> (f64, f64, f64, f64) {
    let w = (b.w * 0.5).clamp(8.0, 12.0);
    (b.x + (b.w - w) / 2.0, b.bottom() + 1.5, w, 3.0)
}

/// True when a point is on the `i`-th pencil of a block.
pub fn on_pencil(b: &Block, i: usize, rtl: bool, at: (f64, f64)) -> bool {
    let (cx, cy) = pencil_centre(b, i, rtl);
    (at.0 - cx).abs() <= PENCIL_R + 1.0 && (at.1 - cy).abs() <= PENCIL_R + 1.0
}

/// Where highlight lines go: `count` lines of one logical pixel with a gap of one between them in
/// `room` pixels. Returns the lines shown and how many did not fit; when some do not fit,
/// `label_h` pixels at the bottom are kept for a "+N".
pub fn highlight_lines(count: usize, room: f64, label_h: f64) -> (usize, usize) {
    let slots = |r: f64| {
        if r < 1.0 {
            0
        } else {
            ((r + 1.0) / 2.0).floor() as usize
        }
    };
    if count <= slots(room) {
        return (count, 0);
    }
    let shown = slots(room - label_h);
    (shown, count - shown)
}

// ---- colours ----

fn lin(c: u8) -> f64 {
    let c = c as f64 / 255.0;
    if c <= 0.03928 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Relative luminance (0 black .. 1 white).
pub fn luminance(c: Rgb8) -> f64 {
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

/// WCAG contrast ratio (1..21).
pub fn contrast(a: Rgb8, b: Rgb8) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn mix(a: Rgb8, b: Rgb8, t: f64) -> Rgb8 {
    let m = |i: usize| {
        (a[i] as f64 + (b[i] as f64 - a[i] as f64) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    [m(0), m(1), m(2)]
}

/// The outline for a colour drawn on `backdrop`: `None` when the colour stands out well enough
/// (3:1) on its own, else the colour darkened (on a light backdrop) or lightened (on a dark one)
/// until the outline itself has 3:1 against the backdrop. With `always` (e-ink, high contrast) there
/// is always an outline, black or white at the extreme.
pub fn outline_for(fill: Rgb8, backdrop: Rgb8, always: bool) -> Option<Rgb8> {
    if !always && contrast(fill, backdrop) >= 3.0 {
        return None;
    }
    let target = if luminance(backdrop) > 0.4 {
        [0, 0, 0]
    } else {
        [255, 255, 255]
    };
    let mut best = target;
    for step in 1..=10 {
        let c = mix(fill, target, step as f64 / 10.0);
        if contrast(c, backdrop) >= 3.0 {
            best = c;
            break;
        }
    }
    Some(best)
}

/// Black or white, whichever is easier to read on `fill` (for the number in a circle).
pub fn ink_on(fill: Rgb8) -> Rgb8 {
    if contrast(fill, [255, 255, 255]) >= contrast(fill, [0, 0, 0]) {
        [255, 255, 255]
    } else {
        [0, 0, 0]
    }
}

/// The colour as gray for an e-ink screen (a swatch there is a level of gray plus its outline).
pub fn to_gray(c: Rgb8) -> Rgb8 {
    let g = (luminance(c).powf(1.0 / 2.2) * 255.0)
        .round()
        .clamp(0.0, 255.0) as u8;
    [g, g, g]
}

// ---- the preview card ----

/// Where the top left corner of a card of `card` size goes next to a pointer at `at` in an area of
/// `area` size: right below and to the right by default, flipped to the other side near an edge
/// and, as a last resort, clamped so that no part is cut off (when it fits at all).
pub fn card_origin(at: (f64, f64), card: (f64, f64), area: (f64, f64), gap: f64) -> (f64, f64) {
    let place = |p: f64, size: f64, room: f64| {
        let after = p + gap;
        let before = p - gap - size;
        let v = if after + size <= room {
            after
        } else if before >= 0.0 {
            before
        } else {
            after
        };
        v.min(room - size).max(0.0)
    };
    (
        place(at.0, card.0, area.0),
        place(at.1 + gap, card.1, area.1),
    )
}

// ---- chapters ----

/// The index of the chapter a page is in (chapter starts in page order).
pub fn chapter_of(starts: &[usize], page: usize) -> Option<usize> {
    if starts.is_empty() {
        return None;
    }
    Some(starts.partition_point(|&s| s <= page).saturating_sub(1))
}

/// The cells in view where a chapter begins: `(cell, chapter index)`; chapters that start in the
/// same cell show the last one.
pub fn chapter_boundaries(g: &Grid, starts: &[usize]) -> Vec<(usize, usize)> {
    let view = g.visible_cells();
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (i, &s) in starts.iter().enumerate() {
        let cell = g.cell_of(s);
        if !view.contains(&cell) {
            continue;
        }
        match out.last_mut() {
            Some(l) if l.0 == cell => l.1 = i,
            _ => out.push((cell, i)),
        }
    }
    out
}

// ---- pointer, wheel and keys ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wheel {
    Scroll(i32),
    Zoom(Level),
    None,
}

/// What the wheel does: it scrolls the map by rows (down goes on in the book); with Ctrl it
/// changes the zoom (up zooms in).
pub fn wheel(dy: f64, ctrl: bool, level: Level) -> Wheel {
    if dy == 0.0 || !dy.is_finite() {
        return Wheel::None;
    }
    if ctrl {
        let to = if dy < 0.0 {
            level.deeper()
        } else {
            level.shallower()
        };
        return if to == level {
            Wheel::None
        } else {
            Wheel::Zoom(to)
        };
    }
    Wheel::Scroll(if dy > 0.0 { 1 } else { -1 })
}

/// Where a key moves the cursor page (`None`: not a movement key). Left and Right move a cell,
/// Up and Down a row, Page keys a screen, Home and End to the ends.
pub fn move_cursor(g: &Grid, cursor: usize, key: &str) -> Option<usize> {
    if g.total == 0 {
        return None;
    }
    let last = g.total - 1;
    let step = g.ppc;
    let row = g.cols * g.ppc;
    let screen = row * g.rows.saturating_sub(1).max(1);
    let to = match key {
        "Left" => cursor.saturating_sub(step),
        "Right" => cursor + step,
        "Up" => cursor.saturating_sub(row),
        "Down" => cursor + row,
        "PageUp" => cursor.saturating_sub(screen),
        "PageDown" => cursor + screen,
        "Home" => 0,
        "End" => last,
        _ => return None,
    };
    Some(to.min(last))
}

/// The text shown in the preview: about `max_chars` characters of `text` from `frac` of it, from
/// the start of a word, whitespace collapsed.
pub fn snippet(text: &str, frac: f64, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() || max_chars == 0 {
        return String::new();
    }
    let f = if frac.is_finite() {
        frac.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut i = ((f * chars.len() as f64) as usize).min(chars.len() - 1);
    while i > 0 && !chars[i - 1].is_whitespace() {
        i -= 1;
    }
    let s: String = chars[i..]
        .iter()
        .take(max_chars * 2)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut out: String = s.chars().take(max_chars).collect();
    if s.chars().count() > max_chars {
        while out.ends_with(|c: char| !c.is_whitespace()) && out.contains(' ') {
            out.pop();
        }
        out = out.trim_end().to_string();
        out.push('…');
    }
    if i > 0 {
        out.insert(0, '…');
    }
    out
}

/// The page (0-based) where a fraction of the book falls.
pub fn page_of_fraction(frac: f64, total: usize) -> usize {
    if total == 0 || !frac.is_finite() {
        return 0;
    }
    ((frac.clamp(0.0, 1.0) * total as f64) as usize).min(total - 1)
}

/// The fraction of the book at the start of a page.
pub fn fraction_of_page(page: usize, total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    page.min(total - 1) as f64 / total as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(total: usize, level: Level) -> Grid {
        Grid::new(total, 50, 8, level) // 400 cells
    }

    #[test]
    fn the_levels_go_from_the_whole_book_to_single_pages() {
        assert_eq!(pages_per_cell(3000, 50, 8, Level::Book), 8);
        assert_eq!(pages_per_cell(3000, 50, 8, Level::Section), 3);
        assert_eq!(pages_per_cell(3000, 50, 8, Level::Detail), 1);
        // a small book is one page per cell on every level
        assert_eq!(pages_per_cell(120, 50, 8, Level::Book), 1);
        assert_eq!(pages_per_cell(0, 50, 8, Level::Book), 1);
        assert_eq!(Level::Book.deeper(), Level::Section);
        assert_eq!(Level::Detail.deeper(), Level::Detail);
        assert_eq!(Level::Book.shallower(), Level::Book);
    }

    #[test]
    fn the_whole_book_fits_and_detail_scrolls() {
        let g = grid(3000, Level::Book);
        assert!(g.total_rows() <= 8);
        assert_eq!(g.visible_cells().end, g.cells());
        let mut d = grid(3000, Level::Detail);
        assert_eq!(d.total_rows(), 60);
        d.scroll_rows(1000);
        assert_eq!(d.first_row, 52);
        d.scroll_rows(-1000);
        assert_eq!(d.first_row, 0);
    }

    #[test]
    fn pages_cells_and_positions_agree() {
        let mut g = grid(3000, Level::Detail);
        assert_eq!(g.cell_of(2999), 2999);
        assert_eq!(g.view_pos(120), Some((2, 20)));
        assert_eq!(g.page_at(2, 20), Some(120));
        assert_eq!(g.page_at(8, 0), None);
        assert_eq!(g.page_at(0, 50), None);
        g.first_row = 10;
        assert_eq!(g.view_pos(120), None);
        let z = grid(3000, Level::Section);
        assert_eq!(z.pages_of(1), 3..6);
        assert_eq!(z.pages_of(z.cells() - 1).end, 3000);
        // the last cell of an uneven grid has no page beyond the book
        let g = Grid::new(7, 4, 2, Level::Detail);
        assert_eq!(g.page_at(1, 3), None);
        assert_eq!(g.page_at(1, 2), Some(6));
    }

    #[test]
    fn revealing_and_centring_keep_the_page_in_view() {
        let mut g = grid(3000, Level::Detail);
        g.reveal(2000); // row 40
        assert!(g.view_pos(2000).is_some());
        assert_eq!(g.first_row, 33);
        g.reveal(5);
        assert_eq!(g.first_row, 0);
        g.centre_on(1500);
        assert!(g.view_pos(1500).is_some());
        g.centre_on(2999);
        assert_eq!(g.first_row, g.total_rows() - 8);
    }

    #[test]
    fn zooming_keeps_the_place() {
        let mut g = grid(3000, Level::Book);
        g.set_level(Level::Detail, 1800);
        assert_eq!(g.ppc, 1);
        assert!(g.view_pos(1800).is_some());
        g.set_level(Level::Book, 1800);
        assert_eq!(g.first_row, 0);
        let r = g.resized(30, 5, 1800);
        assert_eq!(r.cols, 30);
        assert!(r.view_pos(1800).is_some() || r.level == Level::Book);
    }

    #[test]
    fn an_empty_book_is_safe() {
        let mut g = Grid::new(0, 10, 4, Level::Detail);
        assert_eq!(g.cells(), 0);
        assert_eq!(g.visible_cells(), 0..0);
        g.scroll_rows(3);
        g.reveal(5);
        g.centre_on(5);
        assert_eq!(g.page_at(0, 0), None);
        assert_eq!(move_cursor(&g, 0, "Right"), None);
        assert_eq!(page_of_fraction(0.5, 0), 0);
        assert_eq!(fraction_of_page(3, 0), 0.0);
    }

    #[test]
    fn bars_grow_with_time_and_pages_read_at_all_show() {
        assert_eq!(bar_height(0.0, 60.0), 0.0);
        assert_eq!(bar_height(f32::NAN, 60.0), 0.0);
        let stub = bar_height(0.5, 60.0);
        assert!((0.2..0.4).contains(&stub));
        assert!(bar_height(10.0, 60.0) < bar_height(40.0, 60.0));
        assert_eq!(bar_height(600.0, 60.0), 1.0);
    }

    #[test]
    fn a_full_bar_is_what_most_pages_took() {
        assert_eq!(full_bar_secs(&[]), 15.0);
        assert_eq!(full_bar_secs(&[0.0, 0.0]), 15.0);
        let v: Vec<f32> = (1..=100).map(|x| x as f32).collect();
        assert_eq!(full_bar_secs(&v), 91.0);
        // one very long page does not flatten the others
        let mut w = vec![30.0; 20];
        w.push(9000.0);
        assert_eq!(full_bar_secs(&w), 30.0);
    }

    #[test]
    fn a_cell_shows_its_tallest_page() {
        let g = Grid::new(10, 5, 1, Level::Book);
        assert_eq!(g.ppc, 2);
        let mut secs = vec![0.0f32; 10];
        secs[3] = 60.0;
        assert_eq!(cell_bar(&g, &secs, 60.0, 0), 0.0);
        assert_eq!(cell_bar(&g, &secs, 60.0, 1), 1.0);
        assert_eq!(cell_bar(&g, &secs, 60.0, 99), 0.0);
    }

    #[test]
    fn history_markers_are_numbered_from_the_last_place() {
        let m = history_markers(&[10, 20, 30], 9);
        assert_eq!(
            m.iter().map(|m| (m.page, m.kind)).collect::<Vec<_>>(),
            vec![
                (30, MarkerKind::History(1)),
                (20, MarkerKind::History(2)),
                (10, MarkerKind::History(3))
            ]
        );
        assert_eq!(history_markers(&[1, 2, 3, 4], 2).len(), 2);
        assert!(history_markers(&[], 9).is_empty());
    }

    const YELLOW: Swatch = Swatch {
        rgb: [255, 217, 31],
        style: MarkStyle::Fill,
    };
    const GREEN: Swatch = Swatch {
        rgb: [51, 204, 107],
        style: MarkStyle::Fill,
    };
    const BLUE: Swatch = Swatch {
        rgb: [64, 140, 255],
        style: MarkStyle::Underline,
    };
    const RED: Swatch = Swatch {
        rgb: [255, 97, 82],
        style: MarkStyle::Strike,
    };

    #[test]
    fn every_highlight_of_a_cell_is_a_line_in_book_order() {
        let g = Grid::new(100, 10, 2, Level::Book); // 5 pages per cell
        let hl = |p, sw| Marker::coloured(p, MarkerKind::Highlight, sw);
        let ms = [
            hl(4, GREEN),
            hl(1, YELLOW),
            hl(3, YELLOW),
            hl(2, BLUE),
            hl(0, RED),
            Marker::coloured(2, MarkerKind::Note, BLUE),
            Marker::coloured(3, MarkerKind::Note, BLUE),
            Marker::new(4, MarkerKind::Bookmark),
            Marker::new(7, MarkerKind::History(2)),
            Marker::new(8, MarkerKind::History(1)),
            hl(50, GREEN),
        ];
        let c = cell_summary(&g, &ms, YELLOW);
        // all of them, by page; repeats are separate lines
        assert_eq!(c[&0].highlights, vec![RED, YELLOW, BLUE, YELLOW, GREEN]);
        // notes are one pencil per colour
        assert_eq!(c[&0].notes, vec![BLUE]);
        assert!(c[&0].bookmark);
        assert_eq!(c[&0].history, None);
        assert_eq!(c[&1].history, Some(1));
        assert_eq!(c[&10].highlights, vec![GREEN]);
        assert!(!c.contains_key(&5));
    }

    fn block() -> Block {
        Block {
            x: 100.0,
            y: 50.0,
            w: 20.0,
            h: BLOCK_H,
        }
    }

    #[test]
    fn highlight_lines_run_edge_to_edge_a_pixel_apart() {
        let b = block();
        for i in 0..5 {
            let (x, y, w) = highlight_line(&b, i);
            assert_eq!((x, w), (b.x, b.w), "no padding");
            assert_eq!(y, b.y + 3.0 + 2.0 * i as f64);
        }
        // the gap between two lines is exactly one pixel
        let (_, y0, _) = highlight_line(&b, 0);
        let (_, y1, _) = highlight_line(&b, 1);
        assert_eq!(y1 - y0 - 1.0, 1.0);
        // the last line that fits ends inside the block
        let (shown, _) = highlight_lines(100, highlight_room(), 8.0);
        let (_, y, _) = highlight_line(&b, shown - 1);
        assert!(y + 1.0 <= b.bottom() - 3.0 + 1e-9 || shown == 0);
    }

    #[test]
    fn pencils_sit_below_the_block_inside_the_gap_and_clear_of_everything_else() {
        let b = block();
        for rtl in [false, true] {
            for i in 0..MAX_NOTES {
                let (cx, cy) = pencil_centre(&b, i, rtl);
                // outside the block, under its bottom edge
                assert!(cy - PENCIL_R > b.bottom(), "{i} {rtl}");
                // in the row gap: above the next row's label strip
                assert!(cy + PENCIL_R < b.bottom() + ROW_GAP, "{i} {rtl}");
                // inside the width of the block (at least its centre)
                assert!(cx >= b.x && cx <= b.x + b.w);
                // not on top of the pill of the current page
                let (px, py, pw, ph) = pill_rect(&b);
                let apart =
                    cx + PENCIL_R < px || cx - PENCIL_R > px + pw || cy - PENCIL_R > py + ph;
                assert!(apart, "pencil {i} touches the pill");
            }
        }
        // the first pencil is at the end corner: right in left-to-right, left in right-to-left
        assert!(pencil_centre(&b, 0, false).0 > b.x + b.w / 2.0);
        assert!(pencil_centre(&b, 0, true).0 < b.x + b.w / 2.0);
        // the next row's bookmark flag (it hangs 4 px above its block) is further down than any pencil
        let next_flag_top = b.bottom() + ROW_GAP + LABEL_H - 4.0;
        assert!(pencil_centre(&b, 0, false).1 + PENCIL_R < next_flag_top);
        assert_eq!(ROW_H, LABEL_H + BLOCK_H + ROW_GAP);
    }

    #[test]
    fn a_pointer_on_a_pencil_is_found() {
        let b = block();
        let (cx, cy) = pencil_centre(&b, 1, false);
        assert!(on_pencil(&b, 1, false, (cx, cy)));
        assert!(on_pencil(&b, 1, false, (cx + 4.0, cy - 4.0)));
        assert!(!on_pencil(&b, 1, false, (cx + 9.0, cy)));
        assert!(!on_pencil(&b, 0, false, (cx, cy)));
        assert!(!on_pencil(&b, 1, false, (cx, b.y + 5.0)));
    }

    #[test]
    fn highlight_lines_are_one_pixel_with_a_one_pixel_gap() {
        // n lines need 2n - 1 pixels
        assert_eq!(highlight_lines(0, 18.0, 8.0), (0, 0));
        assert_eq!(highlight_lines(1, 1.0, 8.0), (1, 0));
        assert_eq!(highlight_lines(2, 3.0, 8.0), (2, 0));
        assert_eq!(highlight_lines(9, 17.0, 8.0), (9, 0));
        assert_eq!(highlight_lines(9, 18.0, 8.0), (9, 0));
        // too many: the lines that fit above a "+N" label
        let (shown, more) = highlight_lines(20, 18.0, 8.0);
        assert_eq!((shown, more), (5, 15));
        assert!(shown * 2 - 1 + 8 <= 18);
        // no room at all
        assert_eq!(highlight_lines(3, 0.5, 8.0), (0, 3));
        assert_eq!(highlight_lines(3, 4.0, 8.0), (0, 3));
    }

    #[test]
    fn a_highlight_without_a_colour_gets_the_fallback_and_styles_are_read_from_names() {
        let g = Grid::new(10, 5, 2, Level::Detail);
        let c = cell_summary(&g, &[Marker::new(3, MarkerKind::Highlight)], GREEN);
        assert_eq!(c[&3].highlights, vec![GREEN]);
        assert!(CellMarks::default().is_empty());
        assert!(!c[&3].is_empty());
        assert_eq!(MarkStyle::from_name("underline"), MarkStyle::Underline);
        assert_eq!(MarkStyle::from_name("squiggly"), MarkStyle::Underline);
        assert_eq!(MarkStyle::from_name("strikethrough"), MarkStyle::Strike);
        assert_eq!(MarkStyle::from_name("background"), MarkStyle::Fill);
        assert_eq!(MarkStyle::from_name(""), MarkStyle::Fill);
    }

    #[test]
    fn contrast_is_the_wcag_ratio() {
        assert!((contrast([0, 0, 0], [255, 255, 255]) - 21.0).abs() < 1e-6);
        assert!((contrast([10, 20, 30], [10, 20, 30]) - 1.0).abs() < 1e-9);
        assert!(luminance([255, 255, 255]) > 0.99 && luminance([0, 0, 0]) < 0.001);
    }

    #[test]
    fn a_pale_colour_on_a_pale_page_gets_a_readable_outline_and_a_strong_one_none() {
        let light = [250, 250, 250];
        let dark = [36, 36, 36];
        let sepia = [244, 236, 216];
        let black = [0, 0, 0];
        // yellow vanishes on light and sepia pages
        for bg in [light, sepia] {
            let o = outline_for(YELLOW.rgb, bg, false)
                .expect("yellow needs an outline on a light page");
            assert!(contrast(o, bg) >= 3.0, "{o:?} on {bg:?}");
        }
        // but stands out on dark ones
        assert_eq!(outline_for(YELLOW.rgb, dark, false), None);
        assert_eq!(outline_for(YELLOW.rgb, black, false), None);
        // a dark blue vanishes on a dark page and gets a light outline
        let navy = [20, 40, 120];
        let o = outline_for(navy, dark, false).unwrap();
        assert!(luminance(o) > luminance(dark));
        assert!(contrast(o, dark) >= 3.0);
        // every highlight colour of the reader is readable somewhere on every theme
        for sw in [YELLOW, GREEN, BLUE, RED] {
            for bg in [light, dark, sepia, black, [255, 255, 255]] {
                let ok = match outline_for(sw.rgb, bg, false) {
                    None => contrast(sw.rgb, bg) >= 3.0,
                    Some(o) => contrast(o, bg) >= 3.0,
                };
                assert!(ok, "{sw:?} on {bg:?}");
            }
        }
    }

    #[test]
    fn e_ink_and_high_contrast_always_outline_in_black_or_white() {
        assert_eq!(
            outline_for(GREEN.rgb, [255, 255, 255], true)
                .map(|o| contrast(o, [255, 255, 255]) >= 3.0),
            Some(true)
        );
        assert_eq!(
            outline_for([0, 0, 0], [255, 255, 255], true),
            Some([0, 0, 0])
        );
        assert_eq!(
            outline_for([255, 255, 255], [0, 0, 0], true),
            Some([255, 255, 255])
        );
    }

    #[test]
    fn numbers_are_readable_on_their_circle() {
        for c in [
            YELLOW.rgb,
            GREEN.rgb,
            BLUE.rgb,
            [0, 0, 0],
            [255, 255, 255],
            [30, 90, 200],
        ] {
            assert!(contrast(ink_on(c), c) >= 4.0, "{c:?}");
        }
        let g = to_gray(YELLOW.rgb);
        assert_eq!(g[0], g[1]);
        assert!(to_gray([255, 255, 255])[0] > 250 && to_gray([0, 0, 0])[0] == 0);
    }

    #[test]
    fn the_card_follows_the_pointer_and_never_leaves_the_area() {
        let area = (800.0, 500.0);
        let card = (260.0, 120.0);
        // plenty of room: below and to the right
        assert_eq!(
            card_origin((100.0, 100.0), card, area, 14.0),
            (114.0, 128.0)
        );
        // near the right edge: flipped to the left of the pointer
        let (x, _) = card_origin((700.0, 100.0), card, area, 14.0);
        assert_eq!(x, 700.0 - 14.0 - 260.0);
        // near the bottom: above the pointer
        let (_, y) = card_origin((100.0, 450.0), card, area, 14.0);
        assert_eq!(y, 450.0 + 14.0 - 14.0 - 120.0);
        // in a corner and everywhere else it stays inside
        for px in [0.0, 5.0, 400.0, 795.0, 800.0] {
            for py in [0.0, 5.0, 250.0, 495.0, 500.0] {
                let (x, y) = card_origin((px, py), card, area, 14.0);
                assert!(x >= 0.0 && x + card.0 <= area.0, "{px},{py} -> {x}");
                assert!(y >= 0.0 && y + card.1 <= area.1, "{px},{py} -> {y}");
            }
        }
        // a card bigger than the area is pinned to the top left
        assert_eq!(
            card_origin((10.0, 10.0), (900.0, 600.0), area, 14.0),
            (0.0, 0.0)
        );
    }

    #[test]
    fn chapters_are_found_by_page() {
        let starts = [0, 10, 10, 40];
        assert_eq!(chapter_of(&starts, 0), Some(0));
        assert_eq!(chapter_of(&starts, 9), Some(0));
        assert_eq!(chapter_of(&starts, 10), Some(2));
        assert_eq!(chapter_of(&starts, 99), Some(3));
        assert_eq!(chapter_of(&[], 3), None);
        assert_eq!(chapter_of(&[5], 2), Some(0));
    }

    #[test]
    fn chapter_boundaries_are_the_cells_in_view() {
        let g = Grid::new(100, 10, 2, Level::Book);
        let b = chapter_boundaries(&g, &[0, 3, 40, 99]);
        // chapters 0 and 1 begin in the same cell: the later one is shown
        assert_eq!(b, vec![(0, 1), (8, 2), (19, 3)]);
        let mut d = Grid::new(100, 10, 2, Level::Detail);
        d.first_row = 4;
        assert_eq!(chapter_boundaries(&d, &[0, 40, 99]), vec![(40, 1)]);
    }

    #[test]
    fn the_wheel_scrolls_and_ctrl_zooms() {
        assert_eq!(wheel(1.0, false, Level::Book), Wheel::Scroll(1));
        assert_eq!(wheel(-1.0, false, Level::Book), Wheel::Scroll(-1));
        assert_eq!(wheel(-1.0, true, Level::Book), Wheel::Zoom(Level::Section));
        assert_eq!(wheel(1.0, true, Level::Detail), Wheel::Zoom(Level::Section));
        assert_eq!(wheel(-1.0, true, Level::Detail), Wheel::None);
        assert_eq!(wheel(1.0, true, Level::Book), Wheel::None);
        assert_eq!(wheel(0.0, false, Level::Book), Wheel::None);
        assert_eq!(wheel(f64::NAN, false, Level::Book), Wheel::None);
    }

    #[test]
    fn keys_move_the_cursor_by_cell_row_and_screen() {
        let g = grid(3000, Level::Detail);
        assert_eq!(move_cursor(&g, 100, "Right"), Some(101));
        assert_eq!(move_cursor(&g, 100, "Left"), Some(99));
        assert_eq!(move_cursor(&g, 100, "Down"), Some(150));
        assert_eq!(move_cursor(&g, 100, "Up"), Some(50));
        assert_eq!(move_cursor(&g, 100, "PageDown"), Some(450));
        assert_eq!(move_cursor(&g, 100, "PageUp"), Some(0));
        assert_eq!(move_cursor(&g, 100, "Home"), Some(0));
        assert_eq!(move_cursor(&g, 100, "End"), Some(2999));
        assert_eq!(move_cursor(&g, 2990, "PageDown"), Some(2999));
        assert_eq!(move_cursor(&g, 0, "Left"), Some(0));
        assert_eq!(move_cursor(&g, 0, "x"), None);
        let z = grid(3000, Level::Section);
        assert_eq!(move_cursor(&z, 0, "Right"), Some(3));
    }

    #[test]
    fn snippets_start_at_a_word_and_end_cleanly() {
        let text = "Call me Ishmael. Some years ago—never mind how long precisely—having little or no money in my purse.";
        let s = snippet(text, 0.0, 40);
        assert!(s.starts_with("Call me Ishmael."));
        assert!(s.ends_with('…'));
        assert!(s.chars().count() <= 41);
        let mid = snippet(text, 0.5, 30);
        assert!(mid.starts_with('…'));
        assert!(!mid.contains("  "));
        assert_eq!(snippet("", 0.5, 10), "");
        assert_eq!(snippet("  a \n b  ", 0.0, 10), "a b");
        assert!(snippet("日本語のテキスト", 0.5, 4).chars().count() <= 6);
        assert_eq!(snippet("abc", f64::NAN, 10), "abc");
    }

    #[test]
    fn fractions_and_pages_round_trip() {
        assert_eq!(page_of_fraction(0.0, 300), 0);
        assert_eq!(page_of_fraction(1.0, 300), 299);
        assert_eq!(page_of_fraction(0.5, 300), 150);
        assert_eq!(page_of_fraction(f64::NAN, 300), 0);
        assert!((fraction_of_page(150, 300) - 0.5).abs() < 1e-12);
        assert_eq!(page_of_fraction(fraction_of_page(77, 300), 300), 77);
    }

    #[test]
    fn the_indicators_follow_the_view() {
        let mut g = grid(3000, Level::Detail);
        assert_eq!(g.zoom_indicator(), (2, 3));
        assert_eq!(g.extent_indicator().0, 0.0);
        g.scroll_rows(30);
        let (start, size) = g.extent_indicator();
        assert!((start - 0.5).abs() < 1e-9);
        assert!((size - 8.0 / 60.0).abs() < 1e-9);
        let b = grid(100, Level::Book);
        assert_eq!(b.extent_indicator(), (0.0, 1.0));
    }

    #[test]
    fn a_huge_book_is_cheap() {
        let t = std::time::Instant::now();
        let mut g = Grid::new(1_000_000, 200, 12, Level::Detail);
        for i in 0..2000 {
            g.scroll_rows(1);
            let _ = g.visible_cells();
            let _ = g.view_pos(i * 500);
        }
        assert!(t.elapsed().as_millis() < 500);
    }
}
