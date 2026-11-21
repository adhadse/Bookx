//! The reader's book map (enhancements 18, R12, R27): the whole book as a grid of pages in a
//! dialog. Each page is a small rounded block. Inside a block: a calm accent bar for the time the
//! page was read, a one pixel line in the real colour of each of its highlights, a pencil per
//! note on its bottom edge and a flag for a bookmark. The
//! places you jumped from are numbered circles (1 = where "Back" goes), and the page you are on has
//! an accent outline and a pill under it. A solid card with the chapter, the page and the first
//! lines follows the pointer, also while dragging through the book. The wheel scrolls the map,
//! Ctrl+wheel changes how much is in view.
//!
//! The layout and the rules are `bookx_lookup::mapview` (no GTK, unit tested); this file draws
//! only the cells in view, so a book of thousands of pages costs no more than a short one.

use crate::i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use bookx_lookup::mapview::{
    self, Block, CellMarks, Grid, Level, MarkStyle, Marker, MarkerKind, Rgb8, Swatch, Wheel,
};
use pangocairo::functions::show_layout;
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gdk, glib, pango};

use super::map_thumbs::{Thumb, Thumbs};

/// What the map shows about a page that is under the pointer.
pub struct Preview {
    pub chapter: String,
    pub text: String,
}

pub struct MapData {
    /// The book's title, for the header.
    pub title: String,
    /// Pages in the book (the printed page count when the book has a page-list).
    pub total: usize,
    /// The page on screen, 0-based.
    pub current: usize,
    /// Start page and title of each chapter, in book order.
    pub chapters: Vec<(usize, String)>,
    /// Bookmarks, highlights (with their colours) and notes.
    pub marks: Vec<Marker>,
    /// Pages "Back" returns to, oldest first.
    pub history: Vec<usize>,
    /// Seconds spent on each page (empty when nothing was recorded).
    pub dwell: Vec<f32>,
    /// "Page 12" or a printed label such as "iv".
    pub label: Rc<dyn Fn(usize) -> String>,
    pub preview: Rc<dyn Fn(usize) -> Preview>,
    /// Go to a page.
    pub jump: Rc<dyn Fn(usize)>,
    /// Makes the thumbnail of a page (the card shows the text of `preview` when there is none).
    pub thumbs: Option<Rc<Thumbs>>,
    /// Where the reader's page is in the window: x, y, width, height (the tests cut it out of a screenshot).
    pub reader_rect: (f64, f64, f64, f64),
    /// The reader's chapter and the first page of its view / pages of the chapter.
    pub reader_view: (usize, usize, usize),
    /// The reader's own chapter and layout fingerprint (tests compare the thumbnails' layout with it).
    pub reader_sig: Rc<dyn Fn() -> (usize, (usize, usize, i64, usize))>,
    /// The notes: page, colour of the highlight, text (shown in the card when the pointer is on the pencil).
    pub notes: Vec<(usize, Swatch, String)>,
}

const PAD: f64 = 14.0;
const LABEL_H: f64 = mapview::LABEL_H;
const BLOCK_H: f64 = mapview::BLOCK_H;
const ROW_H: f64 = mapview::ROW_H;
/// Width of a cell when single pages are shown; a longer book scrolls (or is zoomed out).
const CELL_W: f64 = 22.0;
/// The yellow of the reader's highlights: used for a highlight that has no colour.
const YELLOW: Swatch = Swatch {
    rgb: [255, 217, 31],
    style: MarkStyle::Fill,
};

struct State {
    data: MapData,
    starts: Vec<usize>,
    cap: f32,
    grid: RefCell<Grid>,
    /// The marks of every cell for the pages-per-cell they were made for.
    summary: RefCell<(usize, BTreeMap<usize, CellMarks>)>,
    cursor: Cell<usize>,
    pointer: Cell<Option<(f64, f64)>>,
    pressed: Cell<bool>,
    scroll_acc: Cell<f64>,
    /// 0..1 while a zoom change fades in.
    fade: Cell<f64>,
    /// Where the card was put, in the map's coordinates: x, y, width, height.
    card_rect: Cell<(f64, f64, f64, f64)>,
    fader: RefCell<Option<adw::TimedAnimation>>,
    area: gtk::DrawingArea,
    levels: gtk::DrawingArea,
    card: gtk::Box,
    card_title: gtk::Label,
    card_page: gtk::Label,
    card_text: gtk::Label,
    /// The picture of the page in the card (a stand-in or a skeleton while it is made), the note under it.
    thumb_area: gtk::DrawingArea,
    note_label: gtk::Label,
    /// The last thumbnail that was ready, and the page it is of.
    shown: RefCell<Option<(usize, Thumb)>>,
    me: RefCell<std::rc::Weak<State>>,
    /// An announcement is waiting for the layout to settle.
    announce_later: Cell<bool>,
    caption: gtk::Label,
    title: adw::WindowTitle,
    dialog: adw::Dialog,
}

thread_local! {
    static OPEN: RefCell<Option<Rc<State>>> = const { RefCell::new(None) };
}

fn level_name(l: Level) -> String {
    match l {
        Level::Book => tr("Whole book"),
        Level::Section => tr("Sections"),
        Level::Detail => tr("Single pages"),
    }
}

/// The grid for a drawing area of this size.
fn grid_for(total: usize, w: f64, h: f64, level: Level) -> Grid {
    let rows = (((h - 2.0 * PAD) / ROW_H).floor() as usize).max(1);
    let cols = (((w - 2.0 * PAD) / CELL_W).floor() as usize).max(8);
    Grid::new(total, cols, rows, level)
}

fn cell_w(g: &Grid, w: f64) -> f64 {
    (w - 2.0 * PAD) / g.cols as f64
}

/// Left edge of a column (mirrored in right-to-left languages).
fn col_x(g: &Grid, col: usize, w: f64, rtl: bool) -> f64 {
    let cw = cell_w(g, w);
    let c = if rtl { g.cols - 1 - col } else { col };
    PAD + c as f64 * cw
}

/// A pointer position in the unmirrored coordinates of the grid.
fn unmirror(x: f64, w: f64, rtl: bool) -> f64 {
    if rtl {
        w - x
    } else {
        x
    }
}

/// The page under a point of the area, if there is one.
fn page_under(g: &Grid, w: f64, x: f64, y: f64) -> Option<usize> {
    let cw = cell_w(g, w);
    if x < PAD || y < PAD || cw <= 0.0 {
        return None;
    }
    let col = ((x - PAD) / cw) as usize;
    let row = ((y - PAD) / ROW_H) as usize;
    g.page_at(row, col)
}

/// The nearest page to a point (clamped into the grid), for dragging past the edges.
fn page_near(g: &Grid, w: f64, x: f64, y: f64) -> usize {
    let cw = cell_w(g, w).max(1.0);
    let col = (((x - PAD) / cw).floor().max(0.0) as usize).min(g.cols - 1);
    let row = (((y - PAD) / ROW_H).floor().max(0.0) as usize).min(g.rows - 1);
    let cell = ((g.first_row + row) * g.cols + col).min(g.cells().saturating_sub(1));
    g.pages_of(cell).start
}

// ---- colours ----

/// The colours a drawing is made of, for the theme in use.
#[derive(Clone, Copy)]
struct Pal {
    fg: (f64, f64, f64),
    accent: (f64, f64, f64),
    /// The page colour the swatches lie on.
    backdrop: Rgb8,
    /// Thicker lines and outlines everywhere (high contrast or e-ink).
    strong: bool,
    eink: bool,
}

fn pal(w: &gtk::Widget) -> Pal {
    let c = w.color();
    let sm = adw::StyleManager::default();
    let a = sm.accent_color_rgba();
    let eink = crate::eink::is_active();
    let hc = sm.is_high_contrast();
    let (fg, accent) = if eink {
        ((0.0, 0.0, 0.0), (0.0, 0.0, 0.0))
    } else {
        (
            (c.red() as f64, c.green() as f64, c.blue() as f64),
            (a.red() as f64, a.green() as f64, a.blue() as f64),
        )
    };
    let backdrop = if eink {
        [255, 255, 255]
    } else if sm.is_dark() {
        [36, 36, 36]
    } else {
        [250, 250, 250]
    };
    Pal {
        fg,
        accent,
        backdrop,
        strong: hc || eink,
        eink,
    }
}

fn rgb_f(c: Rgb8) -> (f64, f64, f64) {
    (
        c[0] as f64 / 255.0,
        c[1] as f64 / 255.0,
        c[2] as f64 / 255.0,
    )
}

fn set_rgb(cr: &gtk::cairo::Context, c: (f64, f64, f64), a: f64) {
    cr.set_source_rgba(c.0, c.1, c.2, a);
}

/// A rounded rectangle path.
fn round_rect(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let pi = std::f64::consts::PI;
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -pi / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, pi / 2.0);
    cr.arc(x + r, y + h - r, r, pi / 2.0, pi);
    cr.arc(x + r, y + r, r, pi, 1.5 * pi);
    cr.close_path();
}

// ---- marks ----

/// A highlight: a line one logical pixel high in its own colour (`y` is device pixel aligned). A
/// colour that would vanish against the page is drawn in a darker or lighter shade of itself, so
/// the line stays one pixel in all.
fn draw_line(cr: &gtk::cairo::Context, p: &Pal, sw: Swatch, x: f64, y: f64, w: f64) {
    let rgb = if p.eink {
        mapview::to_gray(sw.rgb)
    } else {
        sw.rgb
    };
    let rgb = mapview::outline_for(rgb, p.backdrop, p.eink).unwrap_or(rgb);
    set_rgb(cr, rgb_f(rgb), 1.0);
    cr.rectangle(x, y, w, 1.0);
    let _ = cr.fill();
}

/// A pencil (a note) in the note's colour, `x, y` is its middle.
fn draw_pencil(cr: &gtk::cairo::Context, p: &Pal, sw: Swatch, x: f64, y: f64) {
    let rgb = if p.eink {
        mapview::to_gray(sw.rgb)
    } else {
        sw.rgb
    };
    let halo = mapview::outline_for(rgb, p.backdrop, true).unwrap_or([0, 0, 0]);
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    set_rgb(cr, rgb_f(halo), 1.0);
    cr.set_line_width(4.2);
    cr.move_to(x - 3.0, y + 3.0);
    cr.line_to(x + 3.0, y - 3.0);
    let _ = cr.stroke();
    set_rgb(cr, rgb_f(rgb), 1.0);
    cr.set_line_width(2.4);
    cr.move_to(x - 3.0, y + 3.0);
    cr.line_to(x + 3.0, y - 3.0);
    let _ = cr.stroke();
    cr.set_line_cap(gtk::cairo::LineCap::Butt);
}

/// A bookmark flag hanging from `x, y` (its top middle).
fn draw_flag(cr: &gtk::cairo::Context, p: &Pal, x: f64, y: f64) {
    let path = |cr: &gtk::cairo::Context| {
        cr.move_to(x - 3.5, y);
        cr.line_to(x + 3.5, y);
        cr.line_to(x + 3.5, y + 11.0);
        cr.line_to(x, y + 8.0);
        cr.line_to(x - 3.5, y + 11.0);
        cr.close_path();
    };
    path(cr);
    set_rgb(cr, rgb_f(p.backdrop), 1.0);
    cr.set_line_width(3.0);
    cr.set_line_join(gtk::cairo::LineJoin::Round);
    let _ = cr.stroke();
    path(cr);
    set_rgb(cr, p.accent, 1.0);
    let _ = cr.fill();
}

/// A numbered circle with a thin ring, centred on `x, y`.
fn draw_number(cr: &gtk::cairo::Context, p: &Pal, n: u8, x: f64, y: f64, r: f64) {
    let fill = (p.accent.0 * 255.0, p.accent.1 * 255.0, p.accent.2 * 255.0);
    let fill8 = [fill.0 as u8, fill.1 as u8, fill.2 as u8];
    let ink = mapview::ink_on(fill8);
    // a halo in the page colour, the disc, a thin ring inside
    cr.arc(x, y, r + 1.5, 0.0, std::f64::consts::TAU);
    set_rgb(cr, rgb_f(p.backdrop), 1.0);
    let _ = cr.fill();
    cr.arc(x, y, r, 0.0, std::f64::consts::TAU);
    set_rgb(cr, p.accent, 1.0);
    let _ = cr.fill();
    cr.arc(x, y, r - 1.6, 0.0, std::f64::consts::TAU);
    set_rgb(cr, rgb_f(ink), 0.8);
    cr.set_line_width(if p.strong { 1.4 } else { 0.8 });
    let _ = cr.stroke();
    cr.select_font_face(
        "Sans",
        gtk::cairo::FontSlant::Normal,
        gtk::cairo::FontWeight::Bold,
    );
    cr.set_font_size(if n > 9 { 7.0 } else { 9.0 });
    let t = n.to_string();
    let (tw, th, bx, by) = cr
        .text_extents(&t)
        .map(|e| (e.width(), e.height(), e.x_bearing(), e.y_bearing()))
        .unwrap_or((4.0, 6.0, 0.0, -6.0));
    set_rgb(cr, rgb_f(ink), 1.0);
    cr.move_to(x - tw / 2.0 - bx, y - th / 2.0 - by);
    let _ = cr.show_text(&t);
}

// ---- the grid ----

/// A device pixel aligned coordinate.
fn snapper(w: &gtk::Widget) -> impl Fn(f64) -> f64 {
    let s = w
        .native()
        .and_then(|n| n.surface())
        .map(|s| s.scale())
        .unwrap_or_else(|| w.scale_factor() as f64)
        .max(1.0);
    move |v: f64| (v * s).round() / s
}

fn draw(st: &State, cr: &gtk::cairo::Context, w: i32, _h: i32) {
    let fade = st.fade.get();
    if fade < 1.0 {
        cr.push_group();
    }
    draw_grid(st, cr, w as f64);
    if fade < 1.0 {
        let _ = cr.pop_group_to_source();
        let _ = cr.paint_with_alpha(0.2 + 0.8 * fade);
    }
}

fn draw_grid(st: &State, cr: &gtk::cairo::Context, w: f64) {
    let p = pal(st.area.upcast_ref());
    let rtl = st.area.direction() == gtk::TextDirection::Rtl;
    let snap = snapper(st.area.upcast_ref());
    let g = st.grid.borrow();
    let cw = cell_w(&g, w);
    let gap = if cw >= 16.0 { 3.0 } else { 1.5 };
    let radius = (cw * 0.22).clamp(2.0, 6.0);
    let line = if p.strong { 2.0 } else { 1.0 };
    let summary = st.summary_for(&g);
    let current_cell = g.cell_of(st.data.current);
    let cursor_cell = g.cell_of(st.cursor.get());
    let detail = g.ppc == 1;

    let geometry = |cell: usize| {
        let (row, col) = g.row_col(cell);
        let x = snap(col_x(&g, col, w, rtl) + gap / 2.0);
        let y = snap(PAD + (row - g.first_row) as f64 * ROW_H + LABEL_H);
        (x, y, snap(cw - gap).max(2.0))
    };

    for cell in g.visible_cells() {
        let (x, y, bw) = geometry(cell);
        let bh = BLOCK_H;
        // the page
        round_rect(cr, x, y, bw, bh, radius);
        set_rgb(cr, p.fg, if p.strong { 0.14 } else { 0.08 });
        let _ = cr.fill();
        // the time it was read: a calm accent bar from the bottom
        let bar = mapview::cell_bar(&g, &st.data.dwell, st.cap, cell) as f64;
        if bar > 0.0 {
            let h = (bar * bh).max(3.0);
            let _ = cr.save();
            round_rect(cr, x, y, bw, bh, radius);
            cr.clip();
            if p.eink {
                set_rgb(cr, p.fg, 0.7);
            } else {
                let grad = gtk::cairo::LinearGradient::new(0.0, y + bh - h, 0.0, y + bh);
                grad.add_color_stop_rgba(0.0, p.accent.0, p.accent.1, p.accent.2, 0.55);
                grad.add_color_stop_rgba(1.0, p.accent.0, p.accent.1, p.accent.2, 0.9);
                let _ = cr.set_source(&grad);
            }
            cr.rectangle(x, y + bh - h, bw, h);
            let _ = cr.fill();
            let _ = cr.restore();
        }
        // what the page holds
        if let Some(m) = summary.get(&cell) {
            // highlights: one logical pixel lines, edge to edge, a pixel apart, in book order
            // (clipped to the rounded block so that no corner pokes out)
            let blk = Block { x, y, w: bw, h: bh };
            let (shown, more) =
                mapview::highlight_lines(m.highlights.len(), mapview::highlight_room(), 8.0);
            let _ = cr.save();
            round_rect(cr, x, y, bw, bh, radius);
            cr.clip();
            for (i, s) in m.highlights.iter().take(shown).enumerate() {
                let (lx, ly, lw) = mapview::highlight_line(&blk, i);
                draw_line(cr, &p, *s, lx, snap(ly), lw);
            }
            if more > 0 {
                let t = format!("+{more}");
                cr.select_font_face(
                    "Sans",
                    gtk::cairo::FontSlant::Normal,
                    gtk::cairo::FontWeight::Bold,
                );
                cr.set_font_size(7.0);
                let tw = cr.text_extents(&t).map(|e| e.width()).unwrap_or(10.0);
                set_rgb(cr, p.fg, 0.85);
                cr.move_to(x + (bw - tw) / 2.0, y + 3.0 + shown as f64 * 2.0 + 7.0);
                let _ = cr.show_text(&t);
            }
            let _ = cr.restore();
            // notes: pencils under the block, from the end corner inwards
            for (i, s) in m.notes.iter().enumerate() {
                let (px, py) = mapview::pencil_centre(&blk, i, rtl);
                draw_pencil(cr, &p, *s, px, py);
            }
            if m.bookmark {
                let fx = if rtl { x + 6.0 } else { x + bw - 6.0 };
                draw_flag(cr, &p, fx, y - 4.0);
            }
            if let Some(n) = m.history {
                let r = (bw / 2.0 - 1.0).clamp(6.0, 8.0);
                draw_number(cr, &p, n, x + bw / 2.0, y + bh / 2.0, r);
            }
        }
        // where the pointer is
        if cell == cursor_cell {
            round_rect(cr, x - 0.5, y - 0.5, bw + 1.0, bh + 1.0, radius);
            set_rgb(cr, p.accent, if p.strong { 0.30 } else { 0.18 });
            let _ = cr.fill_preserve();
            set_rgb(cr, p.accent, 0.75);
            cr.set_line_width(line);
            let _ = cr.stroke();
        }
        // the page you are on
        if cell == current_cell {
            round_rect(cr, x - 1.0, y - 1.0, bw + 2.0, bh + 2.0, radius + 1.0);
            set_rgb(cr, p.accent, 1.0);
            cr.set_line_width(line + 1.0);
            let _ = cr.stroke();
            let (pxx, pyy, pw, ph) = mapview::pill_rect(&Block { x, y, w: bw, h: bh });
            round_rect(cr, pxx, pyy, pw, ph, ph / 2.0);
            let _ = cr.fill();
        }
    }
    // chapter starts: a soft line, and the title above it when single pages are shown
    let bounds = mapview::chapter_boundaries(&g, &st.starts);
    for (n, (cell, ch)) in bounds.iter().enumerate() {
        let (x, y, _) = geometry(*cell);
        let lx = if rtl {
            x + cw - gap / 2.0
        } else {
            x - gap / 2.0
        };
        let lx = snap(lx);
        set_rgb(cr, p.fg, if p.strong { 0.7 } else { 0.35 });
        cr.set_line_width(line);
        cr.move_to(lx, y - 5.0);
        cr.line_to(lx, y + BLOCK_H + 3.0);
        let _ = cr.stroke();
        if !detail {
            continue;
        }
        // the room until the next chapter in the same row, or the end of the row
        let (row, col) = g.row_col(*cell);
        let next_col = bounds
            .get(n + 1)
            .map(|(c, _)| g.row_col(*c))
            .filter(|(r, _)| *r == row)
            .map(|(_, c)| c)
            .unwrap_or(g.cols);
        let room = (next_col - col) as f64 * cw - 8.0;
        if room < 34.0 {
            continue;
        }
        let title = &st.data.chapters[*ch].1;
        let layout = st.area.create_pango_layout(Some(title));
        let mut fd = pango::FontDescription::new();
        fd.set_size(8 * pango::SCALE);
        fd.set_weight(pango::Weight::Semibold);
        layout.set_font_description(Some(&fd));
        layout.set_width((room * pango::SCALE as f64) as i32);
        layout.set_ellipsize(pango::EllipsizeMode::End);
        if rtl {
            layout.set_alignment(pango::Alignment::Right);
        }
        let tx = if rtl {
            x + cw - gap - room - 4.0
        } else {
            x + 4.0
        };
        set_rgb(cr, p.fg, 0.7);
        cr.move_to(tx, y - 13.0);
        show_layout(cr, &layout);
    }
}

fn draw_levels(st: &State, cr: &gtk::cairo::Context, w: i32, h: i32) {
    let p = pal(st.levels.upcast_ref());
    let g = st.grid.borrow();
    let (w, h) = (w as f64, h as f64);
    let track_h = (h - 2.0 * PAD).max(10.0);
    // zoom level: three notches
    let x1 = w * 0.3;
    set_rgb(cr, p.fg, 0.2);
    cr.set_line_width(3.0);
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    cr.move_to(x1, PAD);
    cr.line_to(x1, PAD + track_h);
    let _ = cr.stroke();
    let (idx, n) = g.zoom_indicator();
    for i in 0..n {
        let y = PAD + track_h * (i as f64 + 0.5) / n as f64;
        if i == idx {
            set_rgb(cr, p.accent, 1.0);
            cr.arc(x1, y, 5.0, 0.0, std::f64::consts::TAU);
        } else {
            set_rgb(cr, p.fg, 0.45);
            cr.arc(x1, y, 2.5, 0.0, std::f64::consts::TAU);
        }
        let _ = cr.fill();
    }
    // the part of the book in view
    let x2 = w * 0.75;
    set_rgb(cr, p.fg, 0.12);
    cr.set_line_width(6.0);
    cr.move_to(x2, PAD);
    cr.line_to(x2, PAD + track_h);
    let _ = cr.stroke();
    let (start, size) = g.extent_indicator();
    set_rgb(cr, p.accent, 0.9);
    cr.move_to(x2, PAD + track_h * start);
    cr.line_to(x2, PAD + track_h * (start + size).min(1.0));
    let _ = cr.stroke();
}

impl State {
    fn marks_all(&self) -> Vec<Marker> {
        let mut v = self.data.marks.clone();
        v.extend(mapview::history_markers(&self.data.history, 9));
        v
    }

    /// The marks of every cell for the current zoom (kept until the zoom changes).
    fn summary_for(&self, g: &Grid) -> std::cell::Ref<'_, BTreeMap<usize, CellMarks>> {
        if self.summary.borrow().0 != g.ppc {
            let map = mapview::cell_summary(g, &self.marks_all(), YELLOW);
            *self.summary.borrow_mut() = (g.ppc, map);
        }
        std::cell::Ref::map(self.summary.borrow(), |s| &s.1)
    }

    fn page_text(&self, page: usize) -> String {
        let label = (self.data.label)(page);
        let pct = mapview::fraction_of_page(page, self.data.total) * 100.0;
        format!("{label} · {pct:.0}%")
    }

    fn chapter_name(&self, page: usize) -> String {
        mapview::chapter_of(&self.starts, page)
            .and_then(|i| self.data.chapters.get(i))
            .map(|c| c.1.clone())
            .unwrap_or_default()
    }

    fn rtl(&self) -> bool {
        self.area.direction() == gtk::TextDirection::Rtl
    }

    /// What a screen reader (and the tests) can read about the map: zoom level, rows in view and
    /// the page under the cursor.
    fn announce(&self) {
        let g = self.grid.borrow();
        let text = format!(
            "Book map: {}, row {} of {}, {}",
            level_name(g.level),
            g.first_row + 1,
            g.total_rows().max(1),
            self.caption.text()
        );
        let (cx, cy, cw, ch) = self.card_rect.get();
        let card = format!(
            "card {} {:.0},{:.0},{:.0},{:.0}",
            if self.card.is_visible() {
                "shown"
            } else {
                "hidden"
            },
            cx,
            cy,
            cw,
            ch
        );
        let thumb = self
            .thumb_area
            .compute_bounds(&self.area)
            .filter(|_| self.data.thumbs.is_some())
            .map(|r| {
                format!(
                    " thumb {:.0},{:.0},{:.0},{:.0} {}",
                    r.x(),
                    r.y(),
                    r.width(),
                    r.height(),
                    if self
                        .shown
                        .borrow()
                        .as_ref()
                        .is_some_and(|(p, _)| *p == self.cursor.get())
                    {
                        "ready"
                    } else {
                        "pending"
                    }
                )
            })
            .unwrap_or_default();
        // the same in the window's coordinates (the tests read the screenshot with them)
        let abs = |w: &gtk::Widget| {
            self.area
                .root()
                .and_then(|r| w.compute_bounds(r.upcast_ref::<gtk::Widget>()))
                .map(|r| {
                    format!(
                        "{:.0},{:.0},{:.0},{:.0}",
                        r.x(),
                        r.y(),
                        r.width(),
                        r.height()
                    )
                })
                .unwrap_or_default()
        };
        let g = self.grid.borrow();
        let col = self
            .shown
            .borrow()
            .as_ref()
            .map(|(_, t)| format!("{}/{}", t.column.0 + 1, t.column.1))
            .unwrap_or_default();
        let tv = self
            .shown
            .borrow()
            .as_ref()
            .map(|(_, t)| format!("{},{},{}", t.spine, t.view_page.0, t.view_page.1))
            .unwrap_or_default();
        let rv = {
            let (spine, c, n) = self.data.reader_view;
            format!("{spine},{c},{n}")
        };
        let (rx, ry, rw, rh) = self.data.reader_rect;
        let absolute = format!(
            " cardwin {} thumbwin {} areawin {} readerwin {rx:.0},{ry:.0},{rw:.0},{rh:.0} grid {},{},{},{},{} pages {} thumbcol {col} thumbview {tv} readerview {rv} cursor {} current {} tstats {}",
            abs(self.card.upcast_ref()),
            abs(self.thumb_area.upcast_ref()),
            abs(self.area.upcast_ref()),
            g.cols,
            g.rows,
            g.first_row,
            g.ppc,
            self.area.width(),
            self.data.total,
            self.cursor.get(),
            self.data.current,
            self.data
                .thumbs
                .as_ref()
                .map(|t| {
                    let x = t.stats.get();
                    format!(
                        "{},{},{},{},{:.0}",
                        x.layouts,
                        x.paints,
                        x.disk_hits,
                        t.queued(),
                        x.max_slice_ms
                    )
                })
                .unwrap_or_default(),
        );
        self.area.update_property(&[
            gtk::accessible::Property::Label(&text),
            gtk::accessible::Property::Description(&format!("{card}{thumb}{absolute}")),
        ]);
    }

    /// Asks for the thumbnail of `page`; the last one that was ready stays until the new one is.
    fn want_thumb(&self, page: usize) {
        let Some(th) = self.data.thumbs.clone() else {
            return;
        };
        // never an empty card: the nearest page that is ready stands in (dimmed) until the real one is
        if th.cached(page).is_none() {
            let better = self
                .shown
                .borrow()
                .as_ref()
                .is_none_or(|(p, _)| p.abs_diff(page) > 3);
            if better {
                if let Some(si) = th.stand_in(page) {
                    *self.shown.borrow_mut() = Some(si);
                }
            }
        }
        let me = self.me.borrow().clone();
        th.request(
            page,
            Rc::new(move |p, t| {
                let Some(st) = me.upgrade() else {
                    return;
                };
                *st.shown.borrow_mut() = Some((p, t));
                st.thumb_area.queue_draw();
                st.announce();
            }),
        );
        self.prefetch();
    }

    /// Makes the pages in view ahead, nearest to the cursor first.
    fn prefetch(&self) {
        let Some(th) = self.data.thumbs.clone() else {
            return;
        };
        let g = self.grid.borrow();
        let visible: Vec<usize> = g.visible_cells().map(|c| g.pages_of(c).start).collect();
        th.prefetch(&visible, self.cursor.get());
    }

    /// The note whose pencil is at this point of the map.
    /// The notes of the cell `page` is in, one per line (what the card shows when the cursor is
    /// moved with the keyboard).
    fn cell_notes(&self, page: usize) -> Option<String> {
        let g = self.grid.borrow();
        let cell = g.cell_of(page);
        let mut notes: Vec<&(usize, Swatch, String)> = self
            .data
            .notes
            .iter()
            .filter(|n| g.cell_of(n.0) == cell)
            .collect();
        notes.sort_by_key(|n| n.0);
        let lines: Vec<&str> = notes
            .iter()
            .map(|n| n.2.as_str())
            .filter(|t| !t.trim().is_empty())
            .take(mapview::MAX_NOTES)
            .collect();
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    fn note_at(&self, at: (f64, f64)) -> Option<String> {
        let g = self.grid.borrow();
        let w = self.area.width() as f64;
        let rtl = self.rtl();
        let x = unmirror(at.0, w, rtl);
        let cw = cell_w(&g, w);
        if x < PAD || at.1 < PAD || cw <= 0.0 {
            return None;
        }
        let col = ((x - PAD) / cw) as usize;
        let row = ((at.1 - PAD) / ROW_H) as usize;
        let page = g.page_at(row, col)?;
        let cell = g.cell_of(page);
        let gap = if cw >= 16.0 { 3.0 } else { 1.5 };
        let blk = Block {
            x: col_x(&g, col, w, rtl) + gap / 2.0,
            y: PAD + row as f64 * ROW_H + LABEL_H,
            w: cw - gap,
            h: BLOCK_H,
        };
        // the distinct note colours of the cell, in the order they are drawn
        let mut notes: Vec<&(usize, Swatch, String)> = self
            .data
            .notes
            .iter()
            .filter(|n| g.cell_of(n.0) == cell)
            .collect();
        notes.sort_by_key(|n| n.0);
        let mut distinct: Vec<&(usize, Swatch, String)> = Vec::new();
        for n in notes {
            if !distinct.iter().any(|d| d.1 == n.1) && distinct.len() < mapview::MAX_NOTES {
                distinct.push(n);
            }
        }
        distinct
            .iter()
            .enumerate()
            .find(|(i, _)| mapview::on_pencil(&blk, *i, rtl, at))
            .map(|(_, n)| n.2.clone())
    }

    /// Moves the cursor to `page`: the card, the caption and the drawing follow.
    fn set_cursor(&self, page: usize, at: Option<(f64, f64)>) {
        let page = page.min(self.data.total.saturating_sub(1));
        self.cursor.set(page);
        let p = (self.data.preview)(page);
        let chapter = if p.chapter.is_empty() {
            self.chapter_name(page)
        } else {
            p.chapter
        };
        self.card_title.set_text(&chapter);
        self.card_page.set_text(&self.page_text(page));
        self.card_text.set_text(&p.text);
        self.card_text
            .set_visible(self.data.thumbs.is_none() && !p.text.is_empty());
        self.want_thumb(page);
        // on a pencil: the note
        // (the keys have no pointer: they get the notes of the cell, so the card is not hover only)
        let note = match at {
            Some(a) => self.note_at(a),
            None => self.cell_notes(page),
        };
        self.note_label.set_text(note.as_deref().unwrap_or(""));
        self.note_label.set_visible(note.is_some());
        self.caption
            .set_text(&format!("{chapter} · {}", self.page_text(page)));
        self.place_card(at);
        self.area.queue_draw();
        self.announce();
    }

    /// Puts the card next to the pointer (or the cursor cell when the keys move it), inside the map.
    fn place_card(&self, at: Option<(f64, f64)>) {
        let g = self.grid.borrow();
        let (w, h) = (self.area.width() as f64, self.area.height() as f64);
        let rtl = self.rtl();
        let at = at.unwrap_or_else(|| {
            let (row, col) = g.row_col(g.cell_of(self.cursor.get()));
            let cw = cell_w(&g, w);
            (
                col_x(&g, col, w, rtl) + cw / 2.0,
                PAD + (row.saturating_sub(g.first_row)) as f64 * ROW_H + ROW_H,
            )
        });
        // a widget's size includes its margins: take the ones of the last placement off
        // (changing them to measure would make GTK lay everything out again every time)
        // (a hidden widget measures as nothing)
        self.card.set_visible(true);
        let (ml, mt) = (self.card.margin_start(), self.card.margin_top());
        let (min, nat, _, _) = self.card.measure(gtk::Orientation::Horizontal, -1);
        let cw = (nat.max(min) - ml).max(120) as f64;
        // the width asked for includes the margin too
        let (_, nath, _, _) = self
            .card
            .measure(gtk::Orientation::Vertical, cw as i32 + ml);
        let nath = nath - mt;
        let (x, y) = mapview::card_origin(at, (cw, nath.max(60) as f64), (w, h), 14.0);
        self.card_rect.set((x, y, cw, nath.max(60) as f64));
        if self.card.margin_start() != x as i32 || self.card.margin_top() != y as i32 {
            self.card.set_margin_start(x as i32);
            self.card.set_margin_top(y as i32);
        }
        self.card.set_visible(true);
        // the positions the tests read are only right once GTK has laid the card out
        if !self.announce_later.replace(true) {
            let me = self.me.borrow().clone();
            glib::idle_add_local_full(glib::Priority::LOW, move || {
                if let Some(st) = me.upgrade() {
                    st.announce_later.set(false);
                    st.announce();
                }
                glib::ControlFlow::Break
            });
        }
    }

    fn pointer_page(&self) -> Option<usize> {
        let (x, y) = self.pointer.get()?;
        let g = self.grid.borrow();
        let w = self.area.width() as f64;
        let x = unmirror(x, w, self.rtl());
        if self.pressed.get() {
            Some(page_near(&g, w, x, y))
        } else {
            page_under(&g, w, x, y)
        }
    }

    /// After the view moved under a resting pointer.
    fn follow_pointer(&self) {
        if let Some(p) = self.pointer_page() {
            self.set_cursor(p, self.pointer.get());
        }
        self.levels.queue_draw();
        self.area.queue_draw();
        self.announce();
    }

    fn scroll(&self, rows: i32) {
        self.grid.borrow_mut().scroll_rows(rows);
        self.follow_pointer();
    }

    fn animations(&self) -> bool {
        !crate::eink::is_active()
            && gtk::Settings::default().is_some_and(|s| s.is_gtk_enable_animations())
    }

    /// A short fade-in of the new layout after a zoom change (not with e-ink or reduced motion).
    fn start_fade(self: &Rc<Self>) {
        if !self.animations() {
            return;
        }
        let me = self.clone();
        let target = adw::CallbackAnimationTarget::new(move |v| {
            me.fade.set(v);
            me.area.queue_draw();
        });
        let anim = adw::TimedAnimation::new(&self.area, 0.0, 1.0, 220, target);
        anim.set_easing(adw::Easing::EaseOutCubic);
        anim.play();
        *self.fader.borrow_mut() = Some(anim);
    }

    fn zoom(self: &Rc<Self>, level: Level) {
        let anchor = self.pointer_page().unwrap_or(self.cursor.get());
        self.grid.borrow_mut().set_level(level, anchor);
        self.start_fade();
        self.follow_pointer();
        self.set_cursor(self.cursor.get(), self.pointer.get());
    }

    fn zoom_step(self: &Rc<Self>, deeper: bool) {
        let l = self.grid.borrow().level;
        let to = if deeper { l.deeper() } else { l.shallower() };
        if to != l {
            self.zoom(to);
        }
    }

    fn wheel(self: &Rc<Self>, dy: f64, ctrl: bool) {
        let level = self.grid.borrow().level;
        match mapview::wheel(dy, ctrl, level) {
            Wheel::Scroll(n) => self.scroll(n),
            Wheel::Zoom(l) => self.zoom(l),
            Wheel::None => {}
        }
    }

    fn key(&self, name: &str) -> bool {
        // in a right-to-left language the map runs the other way
        let name = match (self.rtl(), name) {
            (true, "Left") => "Right",
            (true, "Right") => "Left",
            _ => name,
        };
        let to = {
            let g = self.grid.borrow();
            mapview::move_cursor(&g, self.cursor.get(), name)
        };
        let Some(to) = to else { return false };
        self.grid.borrow_mut().reveal(to);
        self.pointer.set(None);
        self.set_cursor(to, None);
        self.levels.queue_draw();
        true
    }

    fn go(&self, page: usize) {
        (self.data.jump)(page);
        self.dialog.close();
    }

    fn resize(&self) {
        let (w, h) = (self.area.width() as f64, self.area.height() as f64);
        if w < 40.0 || h < 40.0 {
            return;
        }
        let level = self.grid.borrow().level;
        let fresh = grid_for(self.data.total, w, h, level);
        let same = {
            let g = self.grid.borrow();
            g.cols == fresh.cols && g.rows == fresh.rows
        };
        if !same {
            let g = self
                .grid
                .borrow()
                .resized(fresh.cols, fresh.rows, self.cursor.get());
            *self.grid.borrow_mut() = g;
            self.set_cursor(self.cursor.get(), None);
            self.levels.queue_draw();
        }
    }
}

/// One entry of the legend: a small drawing and its words.
fn legend_item(kind: &'static str, text: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    let icon = gtk::DrawingArea::new();
    icon.set_content_width(22);
    icon.set_content_height(18);
    icon.set_valign(gtk::Align::Center);
    icon.set_draw_func(move |a, cr, w, h| {
        let p = pal(a.upcast_ref());
        let (w, h) = (w as f64, h as f64);
        let x = w / 2.0;
        let block = |cr: &gtk::cairo::Context| {
            round_rect(cr, 2.0, 3.0, w - 4.0, h - 6.0, 3.0);
            set_rgb(cr, p.fg, 0.1);
            let _ = cr.fill();
        };
        match kind {
            "here" => {
                round_rect(cr, x - 7.0, 1.5, 14.0, h - 6.5, 3.5);
                set_rgb(cr, p.accent, 1.0);
                cr.set_line_width(2.0);
                let _ = cr.stroke();
                round_rect(cr, x - 5.0, h - 3.5, 10.0, 3.0, 1.5);
                let _ = cr.fill();
            }
            "bar" => {
                block(cr);
                round_rect(cr, 2.0, 9.0, w - 4.0, h - 12.0, 3.0);
                set_rgb(cr, p.accent, 0.85);
                let _ = cr.fill();
            }
            "chapter" => {
                set_rgb(cr, p.fg, 0.4);
                cr.set_line_width(1.0);
                cr.move_to(3.5, 1.0);
                cr.line_to(3.5, h - 1.0);
                let _ = cr.stroke();
                block(cr);
            }
            "bookmark" => {
                block(cr);
                draw_flag(cr, &p, x, 1.0);
            }
            "highlight" => {
                block(cr);
                let blue = Swatch {
                    rgb: [64, 140, 255],
                    style: MarkStyle::Fill,
                };
                draw_line(cr, &p, YELLOW, 4.0, 6.0, w - 8.0);
                draw_line(cr, &p, blue, 4.0, 8.0, w - 8.0);
                draw_line(cr, &p, YELLOW, 4.0, 10.0, w - 8.0);
            }
            "note" => {
                block(cr);
                draw_pencil(cr, &p, YELLOW, x + 3.0, h - 8.0);
            }
            _ => {
                block(cr);
                draw_number(cr, &p, 1, x, h / 2.0, 6.5);
            }
        }
    });
    icon.set_accessible_role(gtk::AccessibleRole::Presentation);
    let l = gtk::Label::new(Some(text));
    l.add_css_class("caption");
    b.append(&icon);
    b.append(&l);
    b
}

/// Opens the map over `parent`.
pub fn show(parent: &gtk::Widget, data: MapData) {
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr("Book Map"));
    dialog.set_content_width(760);
    dialog.set_content_height(560);
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let title = adw::WindowTitle::new(&data.title, "");
    header.set_title_widget(Some(&title));
    let zoom_out = gtk::Button::from_icon_name("zoom-out-symbolic");
    zoom_out.set_tooltip_text(Some(&tr("Show More of the Book")));
    super::audit::label(&zoom_out, &tr("Show More of the Book"));
    let zoom_in = gtk::Button::from_icon_name("zoom-in-symbolic");
    zoom_in.set_tooltip_text(Some(&tr("Show Single Pages")));
    super::audit::label(&zoom_in, &tr("Show Single Pages"));
    let legend_btn = gtk::ToggleButton::new();
    legend_btn.set_icon_name("help-contents-symbolic");
    legend_btn.set_active(true);
    legend_btn.set_tooltip_text(Some(&tr("Show or Hide What the Marks Mean")));
    super::audit::label(&legend_btn, &tr("What the marks mean"));
    header.pack_start(&zoom_out);
    header.pack_start(&zoom_in);
    header.pack_end(&legend_btn);
    toolbar.add_top_bar(&header);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.set_vexpand(true);
    let overlay = gtk::Overlay::new();
    overlay.set_hexpand(true);
    let area = gtk::DrawingArea::new();
    area.set_hexpand(true);
    area.set_vexpand(true);
    area.set_focusable(true);
    area.set_can_focus(true);
    area.set_accessible_role(gtk::AccessibleRole::Group);
    super::audit::label(&area, &tr("Book map: pages of the book"));
    overlay.set_child(Some(&area));
    // a solid card (see `.map-preview` in style.css), never see-through
    let card = gtk::Box::new(gtk::Orientation::Vertical, 2);
    card.add_css_class("map-preview");
    card.set_halign(gtk::Align::Start);
    card.set_valign(gtk::Align::Start);
    card.set_can_target(false);
    card.set_width_request(if data.thumbs.is_some() { 224 } else { 250 });
    let card_title = gtk::Label::new(None);
    card_title.add_css_class("heading");
    card_title.set_xalign(0.0);
    card_title.set_ellipsize(pango::EllipsizeMode::End);
    let card_page = gtk::Label::new(None);
    card_page.add_css_class("caption");
    card_page.add_css_class("dim-label");
    card_page.set_xalign(0.0);
    let card_text = gtk::Label::new(None);
    card_text.set_wrap(true);
    card_text.set_xalign(0.0);
    card_text.set_max_width_chars(34);
    card_text.set_lines(4);
    card_text.set_ellipsize(pango::EllipsizeMode::End);
    card_text.add_css_class("caption");
    card_text.set_margin_top(4);
    let (tw, th) = data
        .thumbs
        .as_ref()
        .map(|t| t.logical_size())
        .unwrap_or((0.0, 0.0));
    let thumb_area = gtk::DrawingArea::new();
    thumb_area.set_content_width(tw as i32);
    thumb_area.set_content_height(th as i32);
    thumb_area.set_halign(gtk::Align::Start);
    thumb_area.set_accessible_role(gtk::AccessibleRole::Presentation);
    let thumb_overlay = gtk::Overlay::new();
    thumb_overlay.set_child(Some(&thumb_area));
    thumb_overlay.set_margin_bottom(6);
    thumb_overlay.set_halign(gtk::Align::Start);
    thumb_overlay.set_visible(data.thumbs.is_some());
    let note_label = gtk::Label::new(None);
    note_label.set_wrap(true);
    note_label.set_xalign(0.0);
    note_label.set_max_width_chars(30);
    note_label.set_lines(6);
    note_label.set_ellipsize(pango::EllipsizeMode::End);
    note_label.add_css_class("map-note");
    note_label.set_margin_top(6);
    note_label.set_visible(false);
    card.append(&thumb_overlay);
    card.append(&card_title);
    card.append(&card_page);
    card.append(&card_text);
    card.append(&note_label);
    card.set_visible(false);
    overlay.add_overlay(&card);
    row.append(&overlay);
    let levels = gtk::DrawingArea::new();
    levels.set_content_width(44);
    levels.set_vexpand(true);
    levels.set_accessible_role(gtk::AccessibleRole::Presentation);
    levels.set_tooltip_text(Some(&tr(
        "Zoom level (dots) and the part of the book in view (bar)",
    )));
    row.append(&levels);
    body.append(&row);

    let caption = gtk::Label::new(None);
    caption.set_xalign(0.0);
    caption.set_margin_start(16);
    caption.set_margin_end(16);
    caption.set_margin_bottom(6);
    caption.set_ellipsize(pango::EllipsizeMode::End);
    caption.add_css_class("dim-label");
    body.append(&caption);
    let legend = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    legend.set_margin_start(16);
    legend.set_margin_end(16);
    legend.set_margin_bottom(12);
    legend.set_halign(gtk::Align::Start);
    for (k, t) in [
        ("here", tr("You are here")),
        ("bar", tr("Read (taller: more time)")),
        ("chapter", tr("Chapter start")),
        ("bookmark", tr("Bookmark")),
        ("highlight", tr("Highlight (its colour)")),
        ("note", tr("Note")),
        ("history", tr("Previous place")),
    ] {
        legend.append(&legend_item(k, &t));
    }
    let legend_scroll = gtk::ScrolledWindow::new();
    legend_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
    legend_scroll.set_child(Some(&legend));
    let revealer = gtk::Revealer::new();
    revealer.set_transition_type(gtk::RevealerTransitionType::SlideUp);
    revealer.set_reveal_child(true);
    revealer.set_child(Some(&legend_scroll));
    body.append(&revealer);
    {
        let r = revealer.clone();
        legend_btn.connect_toggled(move |b| r.set_reveal_child(b.is_active()));
    }
    toolbar.set_content(Some(&body));
    // the dialog follows the application's style, not the book's reading theme
    toolbar.add_css_class("book-map");
    dialog.set_child(Some(&toolbar));

    let starts: Vec<usize> = data.chapters.iter().map(|c| c.0).collect();
    let cap = mapview::full_bar_secs(&data.dwell);
    let total = data.total;
    let current = data.current.min(total.saturating_sub(1));
    let mut grid = grid_for(total, 800.0, 520.0, Level::Detail);
    grid.set_level(Level::Detail, current);
    let st = Rc::new(State {
        data,
        starts,
        cap,
        grid: RefCell::new(grid),
        summary: RefCell::new((0, BTreeMap::new())),
        cursor: Cell::new(current),
        pointer: Cell::new(None),
        pressed: Cell::new(false),
        scroll_acc: Cell::new(0.0),
        fade: Cell::new(1.0),
        card_rect: Cell::new((0.0, 0.0, 0.0, 0.0)),
        fader: RefCell::new(None),
        area: area.clone(),
        levels: levels.clone(),
        card,
        card_title,
        card_page,
        card_text,
        thumb_area,
        note_label,
        shown: RefCell::new(None),
        me: RefCell::new(std::rc::Weak::new()),
        announce_later: Cell::new(false),
        caption,
        title,
        dialog: dialog.clone(),
    });
    *st.me.borrow_mut() = Rc::downgrade(&st);
    {
        // the picture of the page in the card: the page colour while it is made, a hairline border
        let s2 = st.clone();
        st.thumb_area.set_draw_func(move |a, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            let page = s2
                .data
                .thumbs
                .as_ref()
                .map(|t| t.page_colour())
                .unwrap_or((1.0, 1.0, 1.0));
            let p = pal(a.upcast_ref());
            round_rect(cr, 0.0, 0.0, w, h, 4.0);
            cr.clip();
            set_rgb(cr, page, 1.0);
            let _ = cr.paint();
            if let Some((shown_page, t)) = s2.shown.borrow().as_ref() {
                let _ = cr.save();
                cr.scale(w / t.size.0 as f64, h / t.size.1 as f64);
                let _ = cr.set_source_surface(&t.surface, 0.0, 0.0);
                cr.source().set_filter(gtk::cairo::Filter::Good);
                // a stand-in (the nearest page that was ready) is dimmed until the real one is made
                let alpha = if *shown_page == s2.cursor.get() {
                    1.0
                } else {
                    0.5
                };
                let _ = cr.paint_with_alpha(alpha);
                let _ = cr.restore();
            } else {
                // a skeleton page: lines of text in a faint tone, the way a page looks from afar
                let lum = (page.0 + page.1 + page.2) / 3.0;
                let tone = if lum > 0.5 { 0.0 } else { 1.0 };
                cr.set_source_rgba(tone, tone, tone, 0.12);
                let (mx, my) = (w * 0.12, h * 0.1);
                let mut y = my;
                let mut n = 0;
                while y + 3.0 < h - my {
                    let lw = if n % 7 == 6 {
                        (w - 2.0 * mx) * 0.5
                    } else {
                        w - 2.0 * mx
                    };
                    round_rect(cr, mx, y, lw, 3.0, 1.5);
                    let _ = cr.fill();
                    y += 8.0;
                    n += 1;
                }
            }
            cr.reset_clip();
            round_rect(cr, 0.5, 0.5, w - 1.0, h - 1.0, 4.0);
            set_rgb(cr, p.fg, 0.28);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
        });
    }
    {
        let pct = mapview::fraction_of_page(current, total) * 100.0;
        st.title.set_subtitle(&trf(
            "Book map · {} pages · {}%",
            &[&total.to_string(), &format!("{pct:.0}")],
        ));
    }
    {
        let s1 = st.clone();
        area.set_draw_func(move |_, cr, w, h| draw(&s1, cr, w, h));
        let st = st.clone();
        levels.set_draw_func(move |_, cr, w, h| draw_levels(&st, cr, w, h));
    }
    {
        let st = st.clone();
        area.connect_resize(move |_, _, _| st.resize());
    }
    // the pointer: hovering and dragging move the cursor and the card; letting go goes there
    {
        let motion = gtk::EventControllerMotion::new();
        let s = st.clone();
        motion.connect_motion(move |_, x, y| {
            // GTK repeats the last position when something under the pointer changes (the card moves
            // after every placement): answering that would loop for ever
            if s.pointer.get() == Some((x, y)) {
                return;
            }
            s.pointer.set(Some((x, y)));
            if let Some(p) = s.pointer_page() {
                s.set_cursor(p, Some((x, y)));
            }
        });
        let s = st.clone();
        motion.connect_leave(move |_| {
            if !s.pressed.get() {
                s.pointer.set(None);
                s.card.set_visible(false);
            }
        });
        area.add_controller(motion);
        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_PRIMARY);
        let s = st.clone();
        click.connect_pressed(move |_, _, x, y| {
            s.pressed.set(true);
            s.pointer.set(Some((x, y)));
            s.area.grab_focus();
            if let Some(p) = s.pointer_page() {
                s.set_cursor(p, Some((x, y)));
            }
        });
        let s = st.clone();
        click.connect_released(move |_, _, x, y| {
            s.pointer.set(Some((x, y)));
            let page = s.pointer_page().unwrap_or(s.cursor.get());
            s.pressed.set(false);
            s.go(page);
        });
        let s = st.clone();
        click.connect_cancel(move |_, _| s.pressed.set(false));
        area.add_controller(click);
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let s = st.clone();
        wheel.connect_scroll(move |c, _, dy| {
            let ctrl = c
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK);
            let acc = s.scroll_acc.get() + dy;
            if acc.abs() >= 1.0 {
                s.scroll_acc.set(acc - acc.signum());
                s.wheel(acc.signum(), ctrl);
            } else {
                s.scroll_acc.set(acc);
            }
            glib::Propagation::Stop
        });
        area.add_controller(wheel);
    }
    {
        let keys = gtk::EventControllerKey::new();
        let s = st.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            let name = match key {
                gdk::Key::Right | gdk::Key::KP_Right => "Right",
                gdk::Key::Left | gdk::Key::KP_Left => "Left",
                gdk::Key::Up | gdk::Key::KP_Up => "Up",
                gdk::Key::Down | gdk::Key::KP_Down => "Down",
                gdk::Key::Page_Up => "PageUp",
                gdk::Key::Page_Down => "PageDown",
                gdk::Key::Home => "Home",
                gdk::Key::End => "End",
                gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space => {
                    s.go(s.cursor.get());
                    return glib::Propagation::Stop;
                }
                gdk::Key::plus | gdk::Key::KP_Add | gdk::Key::equal => {
                    s.zoom_step(true);
                    return glib::Propagation::Stop;
                }
                gdk::Key::minus | gdk::Key::KP_Subtract => {
                    s.zoom_step(false);
                    return glib::Propagation::Stop;
                }
                _ => return glib::Propagation::Proceed,
            };
            if s.key(name) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        area.add_controller(keys);
    }
    {
        let s = st.clone();
        zoom_in.connect_clicked(move |_| s.zoom_step(true));
        let s = st.clone();
        zoom_out.connect_clicked(move |_| s.zoom_step(false));
    }
    {
        let s = st.clone();
        dialog.connect_closed(move |_| {
            // work that is still waiting for a thumbnail is dropped
            if let Some(t) = &s.data.thumbs {
                t.cancel();
            }
            OPEN.with(|o| *o.borrow_mut() = None)
        });
    }
    st.set_cursor(current, None);
    st.card.set_visible(false);
    OPEN.with(|o| *o.borrow_mut() = Some(st.clone()));
    dialog.present(Some(parent));
    area.grab_focus();
}

/// `reader:map-*` dev commands; printed as `book_map: …`.
///
/// `dump`, `wheel=DY` / `wheel=DY|ctrl`, `key=NAME`, `hover=PAGE` (the pointer rests on a page),
/// `drag=A|B` (press on page A, drag to page B, release), `zoom=in|out`, `click=FRACTION`
/// (go to a place of the book), `focus-note` (keyboard cursor on a note), `legend`, `shot=FILE`, `close`.
pub fn dev(cmd: &str) -> String {
    let Some(st) = OPEN.with(|o| o.borrow().clone()) else {
        return "closed".into();
    };
    let (name, arg) = cmd.split_once('=').unwrap_or((cmd, ""));
    let centre_of = |page: usize| -> Option<(f64, f64)> {
        let g = st.grid.borrow();
        let (row, col) = g.row_col(g.cell_of(page));
        let w = st.area.width() as f64;
        (row >= g.first_row && row < g.first_row + g.rows).then(|| {
            (
                col_x(&g, col, w, st.rtl()) + cell_w(&g, w) / 2.0,
                PAD + (row - g.first_row) as f64 * ROW_H + LABEL_H + BLOCK_H / 2.0,
            )
        })
    };
    match name {
        "wheel" => {
            let (d, c) = arg.split_once('|').unwrap_or((arg, ""));
            st.wheel(d.parse().unwrap_or(0.0), c == "ctrl");
        }
        "key" => {
            if arg == "Return" {
                st.go(st.cursor.get());
                return "went".into();
            }
            st.key(arg);
        }
        // the keyboard cursor onto the first page that has a note: the card shows the note too
        "focus-note" => {
            if let Some(page) = st.data.notes.first().map(|n| n.0) {
                st.grid.borrow_mut().reveal(page);
                st.pointer.set(None);
                st.set_cursor(page, None);
            }
        }
        "hover" => {
            let page: usize = arg.parse().unwrap_or(0);
            st.grid.borrow_mut().reveal(page);
            st.area.queue_draw();
            let at = centre_of(page);
            st.pointer.set(at);
            st.set_cursor(page, at);
        }
        "drag" => {
            let (a, b) = arg.split_once('|').unwrap_or((arg, ""));
            let (a, b): (usize, usize) = (a.parse().unwrap_or(0), b.parse().unwrap_or(0));
            st.grid.borrow_mut().reveal(a);
            st.pressed.set(true);
            st.set_cursor(a, centre_of(a));
            st.grid.borrow_mut().reveal(b);
            let at = centre_of(b);
            st.pointer.set(at);
            st.set_cursor(b, at);
            st.pressed.set(false);
            let to = st.cursor.get();
            st.go(to);
            return format!("dragged {a} to {to}");
        }
        "stats" => {
            let Some(th) = st.data.thumbs.clone() else {
                return "stats none".into();
            };
            let x = th.stats.get();
            return format!(
                "stats layouts={} paints={} prefetch_done={} disk_hits={} queued={} max_slice_ms={:.0} max_paint_slice_ms={:.0}",
                x.layouts,
                x.paints,
                x.prefetch_done,
                x.disk_hits,
                th.queued(),
                x.max_slice_ms,
                x.max_paint_slice_ms
            );
        }
        "thumb" => {
            // thumb=PAGE[|FILE]: makes it now, says how long it took, where the time went and
            // whether it was laid out like the reader's own page
            let Some(th) = st.data.thumbs.clone() else {
                return "thumb none".into();
            };
            let page: usize = arg
                .split('|')
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let cached = th.cached(page).is_some();
            let t0 = std::time::Instant::now();
            let made = if cached {
                th.render(page)
            } else {
                // not from the disk cache: the check is about how the page is laid out
                th.render_laid(page)
            };
            return match made {
                Some(t) if arg.contains('|') => {
                    let file = arg.split_once('|').map(|x| x.1).unwrap_or("");
                    #[allow(deprecated)]
                    let ok = gdk::pixbuf_get_from_surface(&t.surface, 0, 0, t.size.0, t.size.1)
                        .is_some_and(|p| p.savev(file, "png", &[]).is_ok());
                    format!("thumb saved {ok}")
                }
                Some(t) => {
                    let x = th.stats.get();
                    let same = match (t.sig, (st.data.reader_sig)()) {
                        (Some((a, sa)), (b, sb)) if a == b => format!("{}", sa == sb),
                        _ => "n/a".into(),
                    };
                    format!(
                        "thumb page={page} size={}x{} col={}/{} ink={} ms={:.0} made={:.0} layout={:.0} goto={:.0} paint={:.0} cached={cached} layouts={} paints={} same_layout={same} spine={} view={}/{}",
                        t.size.0,
                        t.size.1,
                        t.column.0 + 1,
                        t.column.1,
                        t.ink,
                        t0.elapsed().as_secs_f64() * 1000.0,
                        t.ms,
                        x.last.layout,
                        x.last.goto,
                        x.last.paint,
                        x.layouts,
                        x.paints,
                        t.spine,
                        t.view_page.0,
                        t.view_page.1
                    )
                }
                None => format!("thumb page={page} failed"),
            };
        }
        "zoom" => st.zoom_step(arg == "in"),
        "click" => {
            let f: f64 = arg.parse().unwrap_or(0.0);
            let page = mapview::page_of_fraction(f, st.data.total);
            st.go(page);
            return format!("clicked {f}");
        }
        "close" => {
            st.dialog.close();
            return "closed".into();
        }
        _ => {}
    }
    let g = st.grid.borrow();
    let marks = st.marks_all();
    let count = |f: fn(&MarkerKind) -> bool| marks.iter().filter(|m| f(&m.kind)).count();
    let read = st.data.dwell.iter().filter(|s| **s > 0.0).count();
    let hist: Vec<String> = marks
        .iter()
        .filter_map(|m| match m.kind {
            MarkerKind::History(n) => Some(format!("{n}@{}", m.page)),
            _ => None,
        })
        .collect();
    // the colours the cells show, distinct, as hex
    let sum = st.summary_for(&g);
    let mut colours: Vec<String> = Vec::new();
    let mut stack_max = 0;
    for m in sum.values() {
        stack_max = stack_max.max(m.highlights.len());
        for s in &m.highlights {
            let c = format!("{:02x}{:02x}{:02x}", s.rgb[0], s.rgb[1], s.rgb[2]);
            if !colours.contains(&c) {
                colours.push(c);
            }
        }
    }
    colours.sort();
    format!(
        "pages={} level={:?} ppc={} grid={}x{} first_row={} cursor={} current={} chapters={} read={} bookmarks={} highlights={} notes={} history=[{}] colours=[{}] stack={} preview={:?} card={} thumb={} note={:?}",
        st.data.total,
        g.level,
        g.ppc,
        g.cols,
        g.rows,
        g.first_row,
        st.cursor.get(),
        st.data.current,
        st.data.chapters.len(),
        read,
        count(|k| *k == MarkerKind::Bookmark),
        count(|k| *k == MarkerKind::Highlight),
        count(|k| *k == MarkerKind::Note),
        hist.join(","),
        colours.join(","),
        stack_max,
        st.caption.text().to_string(),
        st.card.is_visible(),
        st.shown
            .borrow()
            .as_ref()
            .map(|(p, t)| format!("{p}:{}x{}", t.size.0, t.size.1))
            .unwrap_or_else(|| "none".into()),
        if st.note_label.is_visible() {
            st.note_label.text().to_string()
        } else {
            String::new()
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn right_to_left_mirrors_the_columns_and_the_pointer() {
        let g = Grid::new(100, 10, 5, Level::Detail);
        let w = 2.0 * PAD + 10.0 * 20.0;
        assert_eq!(col_x(&g, 0, w, false), PAD);
        assert_eq!(col_x(&g, 0, w, true), PAD + 9.0 * 20.0);
        assert_eq!(col_x(&g, 9, w, true), PAD);
        // the pointer over the first cell of a mirrored row is the first page
        let x = unmirror(w - PAD - 5.0, w, true);
        assert_eq!(page_under(&g, w, x, PAD + 2.0), Some(0));
        assert_eq!(unmirror(30.0, w, false), 30.0);
        // on the other end
        let x = unmirror(PAD + 5.0, w, true);
        assert_eq!(page_under(&g, w, x, PAD + 2.0), Some(9));
    }

    #[test]
    fn the_grid_fits_the_area_and_dragging_past_the_edge_stays_in_the_book() {
        let g = grid_for(5000, 880.0, 520.0, Level::Detail);
        assert!(g.cols >= 8 && g.rows >= 1);
        assert!(cell_w(&g, 880.0) >= CELL_W - 0.5);
        assert!(page_near(&g, 880.0, -50.0, -50.0) < 5000);
        assert_eq!(page_near(&g, 880.0, 9999.0, 9999.0), {
            let last = (g.first_row + g.rows - 1) * g.cols + g.cols - 1;
            g.pages_of(last.min(g.cells() - 1)).start
        });
        assert_eq!(page_under(&g, 880.0, 2.0, 2.0), None);
    }
}
