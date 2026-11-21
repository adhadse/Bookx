//! `BookCanvas`: Bookx's own text layout and paging engine, built on Pango + cairo.
//!
//! It lays a [`Doc`] out into a continuous *flow* of lines. The flow is either shown as
//! one scrolling column, or cut at line boundaries into pages that are drawn as one or
//! two columns. Text selection, highlights, links and search hits are all implemented
//! here, addressing text by *global character offsets* within the chapter.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use relm4::gtk::{self, gdk, gdk_pixbuf::Pixbuf, glib, pango, prelude::*};

use bookx_text::highlight::HighlightStyle;

use super::page_area::{Geometry, PageArea};
use crate::a11y::{PageText, Step};
use crate::gestures::{
    classify_swipe, drag_selects, mirror_tap, right_is_forward, Swipe, TapAction, TapZones,
};
use bookx_text::hyphenate::Hyphenator;
use bookx_text::paragraph;
use bookx_text::vertical::{self, FlowLine, Frame};

use crate::core::doc::*;

#[path = "canvas_src.rs"]
mod src_pos;

pub type Rgb = (f64, f64, f64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum LayoutMode {
    #[default]
    TwoColumns,
    SingleColumn,
    Scroll,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub bg: Rgb,
    pub fg: Rgb,
    pub link: Rgb,
    pub muted: Rgb,
    pub accent: Rgb,
}

#[derive(Debug, Clone)]
pub struct CanvasStyle {
    pub palette: Palette,
    pub font_family: String,
    pub font_size: f64,
    pub line_spacing: f64,
    pub margin: f64,
    pub justify: bool,
    pub hyphenate: bool,
    /// Knuth-Plass line breaking for justified paragraphs (Pango's own breaking otherwise).
    pub optimal_breaks: bool,
    /// Space after paragraphs in em, instead of the default (style tweaks).
    pub paragraph_spacing: Option<f64>,
    /// Fixed-layout book: a page is shown whole, pictures are scaled to fit it.
    pub fixed: bool,
    pub layout: LayoutMode,
    /// Show the colours of the book (text and page) instead of the theme's.
    pub original_colors: bool,
    /// Base weight of the text (100 to 900) when the reader asks for another than the font's own.
    pub weight: Option<i32>,
    /// Extra space between words in em (negative = tighter).
    pub word_spacing: f64,
    /// Pair kerning of the font (off: every letter keeps its plain advance).
    pub kerning: bool,
    /// Least lines of a paragraph at the bottom (orphans) and the top (widows) of a page.
    pub widows_orphans: usize,
    /// Fonts and sizes the book's CSS asks for are used (off: the reader's font everywhere).
    pub book_fonts: bool,
}

impl Default for CanvasStyle {
    fn default() -> Self {
        CanvasStyle {
            palette: Palette {
                bg: (1.0, 1.0, 1.0),
                fg: (0.12, 0.12, 0.12),
                link: (0.1, 0.4, 0.8),
                muted: (0.45, 0.45, 0.45),
                accent: (0.21, 0.52, 0.89),
            },
            font_family: "Serif".into(),
            font_size: 14.0,
            line_spacing: 1.25,
            margin: 48.0,
            justify: true,
            hyphenate: true,
            optimal_breaks: true,
            paragraph_spacing: None,
            fixed: false,
            layout: LayoutMode::TwoColumns,
            original_colors: false,
            weight: None,
            word_spacing: 0.0,
            kerning: true,
            widows_orphans: 2,
            book_fonts: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Highlight {
    pub id: String,
    pub start: i32,
    pub end: i32,
    pub color: Rgb,
    pub style: HighlightStyle,
    pub note: String,
}

/// The note of the highlight covering char `c`, if it has one.
pub fn note_at(highlights: &[Highlight], c: i32) -> Option<&str> {
    highlights
        .iter()
        .find(|h| c >= h.start && c < h.end && !h.note.is_empty())
        .map(|h| h.note.as_str())
}

#[derive(Debug, Clone)]
pub struct SelectionInfo {
    pub text: String,
    pub start: i32,
    pub end: i32,
    /// Bounding box in widget coordinates (x, y, w, h).
    pub rect: (i32, i32, i32, i32),
    /// Id of an existing highlight fully containing the selection / click.
    pub highlight: Option<String>,
}

/// The ends of the selection on the screen (see [`BookCanvas::selection_ends`]).
#[derive(Debug, Clone, Copy)]
pub struct SelEnds {
    pub start: Option<(f64, f64, f64, f64)>,
    pub end: Option<(f64, f64, f64, f64)>,
    pub vertical: bool,
    pub rtl: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct PosInfo {
    pub page: usize,
    pub pages: usize,
    pub frac: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Next,
    Prev,
}

struct TextItem {
    layout: pango::Layout,
    text: String,
    links: Vec<(usize, usize, String)>,
    char_base: i32,
    char_len: i32,
    x: f64,
    bar: bool,
    bg: bool,
    /// Continues the paragraph of the item before it (text that wraps around a drop cap or a float
    /// is laid out in two pieces).
    cont: bool,
    cap: Option<DropCap>,
    /// Annotations (furigana) of ranges of `text`, in bytes.
    ruby: Vec<(usize, usize, String)>,
    /// Ranges of `text` set horizontally in one em of a vertical line (tate-chu-yoko), in bytes.
    combine: Vec<(usize, usize)>,
    /// The upright context the combined digits are drawn with (the layout's own is turned).
    plain: Option<pango::Context>,
    /// Vertical text whose glyphs all lie on their side (`text-orientation: sideways`): the
    /// baseline is the usual one, not the middle of the character cell.
    sideways: bool,
}

/// The big initial of a drop cap, drawn next to the first lines of the paragraph.
struct DropCap {
    layout: pango::Layout,
    /// Offset from the paragraph's origin.
    x: f64,
    y: f64,
}

/// A floated image; drawn on its own, the text flows around it.
struct FloatBox {
    pix: Pixbuf,
    /// The picture's path in the book (for the image viewer).
    src: String,
    /// What the picture is, for a screen reader (`<img alt>`).
    alt: String,
    x: f64,
    y: f64,
    h: f64,
}

/// Space beside a float that the next lines of text keep clear.
#[derive(Clone, Copy)]
struct Zone {
    side: FloatSide,
    w: f64,
    bottom: f64,
}

enum Kind {
    Text(TextItem),
    Image {
        pix: Pixbuf,
        src: String,
        /// What the picture is, for a screen reader (`<img alt>`).
        alt: String,
    },
    Rule,
}

struct Item {
    kind: Kind,
    y: f64,
    h: f64,
    x: f64,
    anchors: Vec<String>,
}

#[derive(Clone)]
struct LineBox {
    item: usize,
    line: i32,
    y: f64,
    h: f64,
    lx: f64,
    ly: f64,
    /// Baseline in the layout of the paragraph.
    base: f64,
    start: i32,
    end: i32,
}

#[derive(Clone, Copy)]
struct Page {
    y0: f64,
    y1: f64,
}

type Cb<T> = RefCell<Option<Rc<dyn Fn(T)>>>;

#[derive(Default)]
struct Callbacks {
    link: Cb<String>,
    selection: Cb<Option<SelectionInfo>>,
    position: Cb<PosInfo>,
    edge: Cb<Edge>,
    tap: Cb<()>,
    highlight_click: Cb<SelectionInfo>,
    /// A click on a term saved with the user's own definition (see `set_vocab_marks`).
    vocab_click: Cb<SelectionInfo>,
    /// A click on a picture: its path in the book.
    image: Cb<String>,
    double_tap: Cb<()>,
    zoom: Cb<f64>,
}

struct State {
    doc: Option<Rc<Doc>>,
    style: CanvasStyle,
    images: Rc<dyn Fn(&str) -> Option<Vec<u8>>>,
    pix_cache: HashMap<String, Option<Pixbuf>>,
    items: Vec<Item>,
    floats: Vec<FloatBox>,
    lines: Vec<LineBox>,
    pages: Vec<Page>,
    flow_h: f64,
    cur: usize,
    scroll_y: f64,
    width: i32,
    height: i32,
    sel_anchor: Option<i32>,
    /// The end of the selection that stays while a handle moves the other one.
    handle_anchor: Option<i32>,
    sel: Option<(i32, i32)>,
    /// (anchor, moving end) of a selection made with the keyboard.
    kb_sel: Option<(i32, i32)>,
    highlights: Vec<Highlight>,
    hits: Vec<(i32, i32)>,
    /// Terms of the user's vocabulary found in the chapter (char ranges), drawn as dotted underlines.
    vocab_marks: Vec<(i32, i32)>,
    /// Counts the clicks on saved terms; a double click (selecting the word) cancels the popover that waits.
    vocab_tok: u32,
    /// The sentence being read aloud.
    speaking: Option<(i32, i32)>,
    /// Speed reading: the word being read and the single letter to rest the eye on.
    focus_word: Option<((i32, i32), (i32, i32))>,
    active_hit: Option<usize>,
    pending_frac: Option<f64>,
    pending_anchor: Option<String>,
    /// A character to bring on the page once the chapter is laid out (a highlight opened from a list).
    pending_char: Option<i32>,
    pending_last: bool,
    drag_start: Option<(f64, f64)>,
    dragged: bool,
    /// The current drag started with a finger (not a mouse).
    touch_drag: bool,
    /// A long press selected a word: the drag now extends the selection.
    long_pressed: bool,
    drag_t0: Option<std::time::Instant>,
    /// A double click has just selected a word: its release must not count as a click.
    word_selected: bool,
    zones: TapZones,
    /// The book turns pages right to left: taps and swipes are mirrored.
    rtl: bool,
    wheel_acc: f64,
    overscroll: f64,
    /// Language of the book (BCP 47), picks the hyphenation patterns.
    lang: String,
    /// How long the last layout of the chapter took.
    layout_ms: f64,
    /// The page-turn animation the reader asked for (R65); what is played also depends on the
    /// system's reduced motion setting and e-ink mode.
    anim_kind: bookx_text::aids::PageAnimation,
    /// The animation that is running: the page that was left, drawn over the new one.
    anim: Option<TurnAnim>,
    /// Link hints (R43): the label and the place of every link on the page, and what was typed.
    hints: Vec<LinkHint>,
    hint_typed: String,
}

struct LinkHint {
    label: String,
    x: f64,
    y: f64,
    href: String,
}

/// What a typed key did to the link hints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HintResult {
    /// Hints are not showing.
    Off,
    /// More letters are needed.
    Pending,
    /// A link was chosen and followed.
    Followed(String),
    /// The letters match no hint: the hints went away.
    Cancelled,
}

struct TurnAnim {
    old: gtk::cairo::ImageSurface,
    t0: std::time::Instant,
    forward: bool,
    kind: bookx_text::aids::PageAnimation,
}

#[derive(Clone)]
pub struct BookCanvas {
    pub area: PageArea,
    /// What the page looked like when the screen reader text was last refreshed.
    a11y_key: Rc<std::cell::Cell<(usize, i64, usize, usize, i32, u64, i64)>>,
    /// How often the page was drawn and laid out (tests: dragging a card must do neither).
    work: Rc<std::cell::Cell<(u64, u64)>>,
    st: Rc<RefCell<State>>,
    cb: Rc<Callbacks>,
    hover: Rc<HoverNote>,
}

/// Instant (no tooltip delay) preview of a note while the pointer is over its highlight.
struct HoverNote {
    pop: gtk::Popover,
    label: gtk::Label,
    cur: RefCell<Option<String>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum HoverAction {
    Show(String),
    Hide,
    Keep,
}

/// What the hover preview should do when the note under the pointer changes.
pub fn hover_action(current: Option<&str>, now: Option<&str>) -> HoverAction {
    match (current, now) {
        (None, None) => HoverAction::Keep,
        (Some(_), None) => HoverAction::Hide,
        (Some(c), Some(n)) if c == n => HoverAction::Keep,
        (_, Some(n)) => HoverAction::Show(n.to_string()),
    }
}

const SCALE: f64 = pango::SCALE as f64;
const V_MARGIN: f64 = 28.0;
/// Size of an annotation (furigana) in proportion to its base text.
const RUBY_SCALE: f64 = 0.5;

fn px(units: i32) -> f64 {
    units as f64 / SCALE
}

fn fire<T: 'static>(slot: &Cb<T>, v: T) {
    let f = slot.borrow().clone();
    if let Some(f) = f {
        f(v);
    }
}

impl BookCanvas {
    pub fn new() -> Self {
        let area = PageArea::new();
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.set_focusable(true);
        area.set_can_focus(true);
        area.add_css_class("book-canvas");
        let st = Rc::new(RefCell::new(State {
            doc: None,
            style: CanvasStyle::default(),
            images: Rc::new(|_| None),
            pix_cache: HashMap::new(),
            items: Vec::new(),
            floats: Vec::new(),
            lines: Vec::new(),
            pages: Vec::new(),
            flow_h: 0.0,
            cur: 0,
            scroll_y: 0.0,
            width: 0,
            height: 0,
            sel_anchor: None,
            handle_anchor: None,
            sel: None,
            kb_sel: None,
            highlights: Vec::new(),
            hits: Vec::new(),
            vocab_marks: Vec::new(),
            vocab_tok: 0,
            speaking: None,
            focus_word: None,
            active_hit: None,
            pending_frac: None,
            pending_anchor: None,
            pending_char: None,
            pending_last: false,
            drag_start: None,
            dragged: false,
            touch_drag: false,
            long_pressed: false,
            drag_t0: None,
            word_selected: false,
            zones: TapZones::default(),
            rtl: false,
            wheel_acc: 0.0,
            overscroll: 0.0,
            lang: String::new(),
            layout_ms: 0.0,
            anim_kind: bookx_text::aids::PageAnimation::None,
            anim: None,
            hints: Vec::new(),
            hint_typed: String::new(),
        }));
        let pop = gtk::Popover::new();
        pop.set_autohide(false);
        pop.set_can_target(false); // never steals the pointer, so it cannot flicker
        pop.set_position(gtk::PositionType::Top);
        pop.add_css_class("note-hover");
        let label = gtk::Label::new(None);
        label.set_wrap(true);
        label.set_max_width_chars(44);
        label.set_xalign(0.0);
        label.set_margin_start(6);
        label.set_margin_end(6);
        label.set_margin_top(4);
        label.set_margin_bottom(4);
        pop.set_child(Some(&label));
        pop.set_parent(&area);
        {
            let pop = pop.clone();
            area.connect_destroy(move |_| pop.unparent());
        }
        let canvas = BookCanvas {
            area: area.clone(),
            a11y_key: Rc::new(std::cell::Cell::new((usize::MAX, 0, 0, 0, 0, 0, 0))),
            work: Rc::default(),
            st,
            cb: Rc::new(Callbacks::default()),
            hover: Rc::new(HoverNote {
                pop,
                label,
                cur: RefCell::new(None),
            }),
        };
        canvas.wire();
        canvas.wire_accessible_geometry();
        canvas
    }

    /// Answers the screen reader's questions "where is this text" and "what is under this point".
    fn wire_accessible_geometry(&self) {
        let st = Rc::downgrade(&self.st);
        let st2 = st.clone();
        self.area.set_geometry(Geometry {
            extents: Rc::new(move |page, start, end| {
                let st = st.upgrade()?;
                let s = st.try_borrow().ok()?;
                let a = page.chapter_char(start as usize)?;
                let b = page.chapter_char((end as usize).max(start as usize + 1) - 1)? + 1;
                let (x, y, w, h) = range_rect(&s, a, b)?;
                Some((x as f32, y as f32, w as f32, h as f32))
            }),
            offset: Rc::new(move |page, x, y| {
                let st = st2.upgrade()?;
                let s = st.try_borrow().ok()?;
                let (c, _) = hit(&s, x as f64, y as f64)?;
                page.page_offset(c)
            }),
        });
    }

    // ---- callbacks -------------------------------------------------------

    pub fn on_link(&self, f: impl Fn(String) + 'static) {
        *self.cb.link.borrow_mut() = Some(Rc::new(f));
    }
    pub fn on_image(&self, f: impl Fn(String) + 'static) {
        *self.cb.image.borrow_mut() = Some(Rc::new(f));
    }
    pub fn on_selection(&self, f: impl Fn(Option<SelectionInfo>) + 'static) {
        *self.cb.selection.borrow_mut() = Some(Rc::new(f));
    }
    pub fn on_position(&self, f: impl Fn(PosInfo) + 'static) {
        *self.cb.position.borrow_mut() = Some(Rc::new(f));
    }
    pub fn on_edge(&self, f: impl Fn(Edge) + 'static) {
        *self.cb.edge.borrow_mut() = Some(Rc::new(f));
    }
    pub fn on_tap(&self, f: impl Fn() + 'static) {
        *self.cb.tap.borrow_mut() = Some(Rc::new(move |_| f()));
    }
    pub fn on_highlight_click(&self, f: impl Fn(SelectionInfo) + 'static) {
        *self.cb.highlight_click.borrow_mut() = Some(Rc::new(f));
    }

    pub fn on_vocab_click(&self, f: impl Fn(SelectionInfo) + 'static) {
        *self.cb.vocab_click.borrow_mut() = Some(Rc::new(f));
    }

    /// Marks the places in the chapter (global char ranges) where a saved vocabulary term stands.
    pub fn set_vocab_marks(&self, marks: Vec<(i32, i32)>) {
        self.st.borrow_mut().vocab_marks = marks;
        self.area.queue_draw();
    }

    /// The saved term under a point of the page, with where it is.
    fn vocab_mark_at(&self, x: f64, y: f64) -> Option<SelectionInfo> {
        let (a, b, rect) = {
            let s = self.st.borrow();
            if s.vocab_marks.is_empty() || !on_text(&s, x, y) {
                return None;
            }
            let (c, _) = hit(&s, x, y)?;
            let &(a, b) = s.vocab_marks.iter().find(|(a, b)| c >= *a && c < *b)?;
            (a, b, range_rect(&s, a, b)?)
        };
        Some(SelectionInfo {
            text: self.text_between(a, b),
            start: a,
            end: b,
            rect,
            highlight: None,
        })
    }

    /// Test hook: the marks `(start, end, text)` and a click on the first one that is on the page.
    pub fn dev_vocab_marks(&self) -> Vec<(i32, i32, String)> {
        let marks = self.st.borrow().vocab_marks.clone();
        marks
            .into_iter()
            .map(|(a, b)| (a, b, self.text_between(a, b)))
            .collect()
    }

    /// Test hook: clicks the middle of the first saved term that is on the page.
    pub fn dev_click_vocab(&self) -> bool {
        let at = {
            let s = self.st.borrow();
            s.vocab_marks.iter().find_map(|(a, b)| {
                visible_range_rects(&s, *a, *b)
                    .first()
                    .map(|r| (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0))
            })
        };
        match at {
            Some((x, y)) => {
                self.click(x, y);
                true
            }
            None => false,
        }
    }

    /// Test hook: how many saved terms are on the page being shown, and where the first one is.
    pub fn dev_visible_vocab(&self) -> (usize, Option<(i32, i32, i32, i32)>) {
        let s = self.st.borrow();
        let on: Vec<_> = s
            .vocab_marks
            .iter()
            .filter(|(a, b)| !visible_range_rects(&s, *a, *b).is_empty())
            .collect();
        let first = on.first().and_then(|(a, b)| range_rect(&s, *a, *b));
        (on.len(), first)
    }

    /// Double click / tap on empty page area (not on text).
    pub fn on_double_tap(&self, f: impl Fn() + 'static) {
        *self.cb.double_tap.borrow_mut() = Some(Rc::new(move |_| f()));
    }
    /// Pinch or Ctrl+wheel; the argument is a relative scale factor (1.0 = unchanged).
    pub fn on_zoom(&self, f: impl Fn(f64) + 'static) {
        *self.cb.zoom.borrow_mut() = Some(Rc::new(f));
    }

    // ---- content ---------------------------------------------------------

    pub fn set_image_provider(&self, f: impl Fn(&str) -> Option<Vec<u8>> + 'static) {
        let mut s = self.st.borrow_mut();
        s.images = Rc::new(f);
        s.pix_cache.clear();
    }

    /// Replaces the shown chapter. `frac` / `anchor` / `last_page` choose the initial position.
    pub fn set_doc(&self, doc: Doc, frac: Option<f64>, anchor: Option<String>, last_page: bool) {
        {
            let mut s = self.st.borrow_mut();
            s.doc = Some(Rc::new(doc));
            s.sel = None;
            s.sel_anchor = None;
            s.hits.clear();
            s.vocab_marks.clear();
            s.active_hit = None;
            s.speaking = None;
            s.focus_word = None;
            s.pending_frac = Some(frac.unwrap_or(0.0));
            s.pending_anchor = anchor;
            s.pending_char = None;
            s.pending_last = last_page;
            s.cur = 0;
            s.scroll_y = 0.0;
        }
        self.relayout();
        fire(&self.cb.selection, None);
    }

    pub fn set_style(&self, style: CanvasStyle) {
        let changed = {
            let s = self.st.borrow();
            s.style.font_family != style.font_family
                || s.style.font_size != style.font_size
                || s.style.line_spacing != style.line_spacing
                || s.style.margin != style.margin
                || s.style.justify != style.justify
                || s.style.hyphenate != style.hyphenate
                || s.style.optimal_breaks != style.optimal_breaks
                || s.style.paragraph_spacing != style.paragraph_spacing
                || s.style.fixed != style.fixed
                || s.style.layout != style.layout
                || s.style.original_colors != style.original_colors
                || s.style.weight != style.weight
                || s.style.word_spacing != style.word_spacing
                || s.style.kerning != style.kerning
                || s.style.widows_orphans != style.widows_orphans
                || s.style.book_fonts != style.book_fonts
        };
        if changed {
            let frac = self.fraction();
            let mut s = self.st.borrow_mut();
            s.style = style;
            s.pending_frac = Some(frac);
            drop(s);
            self.relayout();
        } else {
            self.st.borrow_mut().style = style;
            self.area.queue_draw();
        }
    }

    /// Sets the language used for hyphenation; relayouts when it changes.
    pub fn set_language(&self, lang: &str) {
        let changed = {
            let mut s = self.st.borrow_mut();
            if s.lang == lang {
                false
            } else {
                s.lang = lang.to_string();
                s.style.hyphenate && s.doc.is_some()
            }
        };
        if changed {
            let frac = self.fraction();
            self.st.borrow_mut().pending_frac = Some(frac);
            self.relayout();
        }
    }

    /// Test helper: the page colours as 8-bit values `bg/fg`.
    /// Test hook: (times the page was drawn, times it was laid out).
    pub fn dev_work(&self) -> (u64, u64) {
        self.work.get()
    }

    pub fn dev_palette(&self) -> String {
        let s = self.st.borrow();
        let q = |c: Rgb| {
            format!(
                "{:02x}{:02x}{:02x}",
                (c.0 * 255.0) as u8,
                (c.1 * 255.0) as u8,
                (c.2 * 255.0) as u8
            )
        };
        let page = page_color(&s.style, s.doc.as_deref());
        format!("{}/{}", q(page), q(s.style.palette.fg))
    }

    /// Test helper: (text items with an indented first line, text items in total).
    pub fn dev_indent_counts(&self) -> (usize, usize) {
        let s = self.st.borrow();
        let (mut ind, mut all) = (0, 0);
        for it in &s.items {
            if let Kind::Text(t) = &it.kind {
                all += 1;
                if t.layout.indent() > 0 {
                    ind += 1;
                }
            }
        }
        (ind, all)
    }

    /// Test helper: (italic spans, bold spans, centred blocks) of the current chapter, headings
    /// excluded.
    pub fn dev_style_counts(&self) -> (usize, usize, usize) {
        let s = self.st.borrow();
        let Some(doc) = &s.doc else {
            return (0, 0, 0);
        };
        let (mut it, mut bd, mut ce) = (0, 0, 0);
        for b in &doc.blocks {
            if matches!(b.kind, BlockKind::Heading(_)) {
                continue;
            }
            if b.align == Align::Center {
                ce += 1;
            }
            for sp in &b.spans {
                if let Span::Text { style, .. } = sp {
                    it += style.italic as usize;
                    bd += style.bold as usize;
                }
            }
        }
        (it, bd, ce)
    }

    /// Test helper: how many laid out paragraphs contain text with its own foreground colour.
    pub fn dev_colored_items(&self) -> usize {
        let s = self.st.borrow();
        s.items
            .iter()
            .filter(|it| match &it.kind {
                Kind::Text(t) => t.layout.attributes().is_some_and(|l| {
                    l.attributes()
                        .iter()
                        .any(|a| a.type_() == pango::AttrType::Foreground)
                }),
                _ => false,
            })
            .count()
    }

    /// Test helper: `(margin, line spacing, paragraph spacing)` of the style in use.
    pub fn dev_style(&self) -> (f64, f64, Option<f64>) {
        let s = self.st.borrow();
        (
            s.style.margin,
            s.style.line_spacing,
            s.style.paragraph_spacing,
        )
    }

    /// Test helper: weight, word spacing, kerning, widows and orphans, book fonts, text colour.
    pub fn dev_style_more(&self) -> String {
        let s = self.st.borrow();
        let (r, g, b) = s.style.palette.fg;
        format!(
            "weight={:?} word_spacing={} kerning={} widows={} book_fonts={} fg={:02x}{:02x}{:02x}",
            s.style.weight,
            s.style.word_spacing,
            s.style.kerning,
            s.style.widows_orphans,
            s.style.book_fonts,
            (r * 255.0).round() as u8,
            (g * 255.0).round() as u8,
            (b * 255.0).round() as u8
        )
    }

    /// Test helper: `(fixed layout, pages, width and height of the first picture, its top,
    /// height of the page area)`.
    pub fn dev_fixed_stats(&self) -> (bool, usize, i32, i32, i32, i32) {
        let s = self.st.borrow();
        let img = s.items.iter().find_map(|i| match &i.kind {
            Kind::Image { pix, .. } => Some((pix.width(), pix.height(), i.y as i32)),
            _ => None,
        });
        let (w, h, top) = img.unwrap_or((0, 0, 0));
        (
            s.style.fixed,
            s.pages.len(),
            w,
            h,
            top,
            content_height(&s) as i32,
        )
    }

    /// Test helper: a double click on the first word of the page, as the mouse does it: the
    /// press selects the word, the release must leave it selected.
    pub fn dev_double_click_first_word(&self) {
        let pos = {
            let s = self.st.borrow();
            visible_columns(&s).first().and_then(|col| {
                let l = s.lines.iter().find(|l| l.y + l.h > col.y0)?;
                let item = &s.items[l.item];
                Some(col.frame.point(item.x + l.lx + 6.0, l.y + l.h / 2.0))
            })
        };
        let Some((x, y)) = pos else { return };
        {
            let mut s = self.st.borrow_mut();
            s.drag_start = Some((x, y));
            s.dragged = false;
            s.touch_drag = false;
            s.word_selected = true;
        }
        self.select_word_at(x, y);
        self.on_release(0.0, 0.0);
    }

    /// Test helper: how the chapter is laid out when its text is vertical: `mode=vertical-rl
    /// pages=3 lines=120 ruby=4 combine=2 hit=true rect=(x, y, w, h)`. `hit` says whether the point in
    /// the middle of the first visible line gives the first character of the page, `rect` is the
    /// rectangle of its first three characters.
    pub fn dev_vertical_stats(&self) -> String {
        let s = self.st.borrow();
        let mode = match mode_of(&s) {
            WritingMode::HorizontalTb => "horizontal-tb",
            WritingMode::VerticalRl => "vertical-rl",
            WritingMode::VerticalLr => "vertical-lr",
        };
        let (mut ruby, mut combine) = (0, 0);
        for it in &s.items {
            if let Kind::Text(t) = &it.kind {
                ruby += t.ruby.len();
                combine += t.combine.len();
            }
        }
        let first = visible_columns(&s).first().and_then(|col| {
            let l = s
                .lines
                .iter()
                .find(|l| l.y + l.h > col.y0 && matches!(s.items[l.item].kind, Kind::Text(_)))?;
            let item = &s.items[l.item];
            let (x, y) = col.frame.point(item.x + l.lx + 2.0, l.y + l.h / 2.0);
            Some((x, y))
        });
        let c0 = {
            let y0 = visible_columns(&s).first().map_or(0.0, |c| c.y0);
            let i = s.lines.partition_point(|l| l.y + l.h <= y0);
            s.lines[i.min(s.lines.len())..]
                .iter()
                .find_map(|l| match &s.items[l.item].kind {
                    Kind::Text(t) => {
                        let lc = byte_to_char(&t.text, l.start.max(0) as usize);
                        Some(t.char_base + canon_of_layout_char(&t.text, lc) as i32)
                    }
                    _ => None,
                })
        };
        let hit_ok = match (first, c0) {
            (Some((x, y)), Some(c)) => hit(&s, x, y).map(|(h, _)| h) == Some(c),
            _ => false,
        };
        let rect = c0
            .and_then(|c| range_rect(&s, c, c + 3))
            .unwrap_or((0, 0, 0, 0));
        let speaking = s
            .speaking
            .map_or(0, |(a, b)| visible_range_rects(&s, a, b).len());
        // forbidden breaks inside the paragraphs of the whole chapter
        let mut kinsoku = 0;
        for (i, it) in s.items.iter().enumerate() {
            let Kind::Text(t) = &it.kind else { continue };
            let texts: Vec<String> = s
                .lines
                .iter()
                .filter(|l| l.item == i)
                .filter_map(|l| {
                    let (a, b) = (
                        l.start.max(0) as usize,
                        (l.end.max(0) as usize).min(t.text.len()),
                    );
                    (a < b && t.text.is_char_boundary(a) && t.text.is_char_boundary(b))
                        .then(|| canon_text(&t.text[a..b]).collect())
                })
                .collect();
            let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
            kinsoku += vertical::violations(&refs).len();
        }
        format!(
            "mode={mode} pages={} lines={} ruby={ruby} combine={combine} kinsoku={kinsoku} speaking={speaking} hit={hit_ok} rect={rect:?} size={}x{}",
            s.pages.len(),
            s.lines.len(),
            s.width,
            s.height
        )
    }

    /// Test helper: the text of every line of the pages that are shown, one entry per visual line,
    /// in reading order.
    pub fn dev_page_lines(&self) -> Vec<String> {
        let s = self.st.borrow();
        let mut out = Vec::new();
        for col in visible_columns(&s) {
            let first = s.lines.partition_point(|l| l.y + l.h <= col.y0);
            for l in &s.lines[first.min(s.lines.len())..] {
                if l.y >= col.y1 {
                    break;
                }
                if let Kind::Text(t) = &s.items[l.item].kind {
                    let (a, b) = (
                        l.start.max(0) as usize,
                        (l.end.max(0) as usize).min(t.text.len()),
                    );
                    if a < b && t.text.is_char_boundary(a) && t.text.is_char_boundary(b) {
                        out.push(canon_text(&t.text[a..b]).collect());
                    }
                }
            }
        }
        out
    }

    /// Test helper: the position, in widget coordinates, of the middle of the `n`-th character of
    /// the chapter, and what a click there selects (so that hit testing can be checked on any
    /// geometry): `(x, y, character found)`.
    pub fn dev_probe_char(&self, n: i32) -> Option<(i32, i32, i32)> {
        let s = self.st.borrow();
        let (x, y, w, h) = *visible_range_rects(&s, n, n + 1).first()?;
        let (px_, py_) = (x + w / 2.0, y + h / 2.0);
        let (c, _) = hit(&s, px_, py_)?;
        Some((px_ as i32, py_ as i32, c))
    }

    /// Test helper: selects with the pointer, as a drag does it, from the middle of character `a`
    /// to the middle of character `b`, and describes the result: `sel=(a, b) text=".." rects=[..]`
    /// (the rectangles of the selection on the screen).
    pub fn dev_drag_select(&self, a: i32, b: i32) -> String {
        let pts = {
            let s = self.st.borrow();
            let mid = |c: i32| {
                visible_range_rects(&s, c, c + 1)
                    .first()
                    .map(|r| (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0))
            };
            (mid(a), mid(b))
        };
        let (Some((x0, y0)), Some((x1, y1))) = pts else {
            return "sel=None text=\"\" rects=[]".into();
        };
        {
            let mut s = self.st.borrow_mut();
            let anchor = hit(&s, x0, y0).map(|(c, _)| c);
            let end = hit(&s, x1, y1).map(|(c, _)| c);
            s.sel_anchor = anchor;
            if let (Some(p), Some(q)) = (anchor, end) {
                s.sel = (p != q).then(|| (p.min(q), p.max(q)));
            }
        }
        self.area.queue_draw();
        let s = self.st.borrow();
        let sel = s.sel;
        let text = sel
            .map(|(p, q)| text_in_range(&s.items, p, q))
            .unwrap_or_default();
        let rects: Vec<(i32, i32, i32, i32)> = sel
            .map(|(p, q)| visible_range_rects(&s, p, q))
            .unwrap_or_default()
            .iter()
            .map(|r| (r.0 as i32, r.1 as i32, r.2 as i32, r.3 as i32))
            .collect();
        format!("sel={sel:?} text={text:?} rects={rects:?}")
    }

    /// The style in use (a thumbnail canvas copies it).
    pub fn style(&self) -> CanvasStyle {
        self.st.borrow().style.clone()
    }

    /// The size of the page area, in pixels.
    pub fn view_size(&self) -> (i32, i32) {
        let s = self.st.borrow();
        (s.width, s.height)
    }

    /// Gives a canvas that is not on screen (the book map's thumbnails) the size of a page, as if it
    /// had been allocated; call it before `set_doc`.
    pub fn set_offscreen_size(&self, w: i32, h: i32) {
        let mut s = self.st.borrow_mut();
        s.width = w.max(1);
        s.height = h.max(1);
    }

    /// The first page of the view (0-based) and the number of pages of the chapter.
    pub fn current_index(&self) -> (usize, usize) {
        let s = self.st.borrow();
        (s.cur, s.pages.len())
    }

    /// A fingerprint of the layout (pages, lines, flow height, column count): equal for two canvases
    /// that lay a chapter out the same way (the thumbnails are only faithful when they match the reader's).
    pub fn layout_signature(&self) -> (usize, usize, i64, usize) {
        let s = self.st.borrow();
        (
            s.pages.len(),
            s.lines.len(),
            (s.flow_h * 10.0) as i64,
            columns(&s),
        )
    }

    /// Moves to the place at `frac` of the chapter (as the reader does when it jumps there) and says
    /// which page of the view it falls on: `(column, columns)` (always `(0, 1)` for one column, the
    /// scroll layout and vertical text, where the whole view is the page).
    pub fn goto_fraction_column(&self, frac: f64) -> (usize, usize) {
        let mut s = self.st.borrow_mut();
        apply_fraction(&mut s, frac);
        let cols = columns(&s);
        if s.style.layout == LayoutMode::Scroll || is_vertical(&s) || cols < 2 {
            return (0, 1);
        }
        let y = frac.clamp(0.0, 1.0) * s.flow_h;
        let y = s
            .lines
            .iter()
            .find(|l| l.y + l.h > y)
            .map(|l| l.y)
            .unwrap_or(y);
        let idx = s.pages.iter().rposition(|p| p.y0 <= y + 0.5).unwrap_or(0);
        (idx.saturating_sub(s.cur).min(cols - 1), cols)
    }

    /// Paints the page that column `col` of `cols` stands for (the half of a two column view; the
    /// whole view otherwise) into a surface `target_w` pixels wide, at thumbnail resolution: the
    /// page is drawn scaled, not drawn full size and shrunk. `device_scale` keeps it sharp on a HiDPI screen.
    pub fn render_page_thumbnail(
        &self,
        target_w: f64,
        device_scale: f64,
        col: usize,
        cols: usize,
    ) -> Option<gtk::cairo::ImageSurface> {
        let s = self.st.borrow();
        let (w, h) = (s.width.max(1) as f64, s.height.max(1) as f64);
        let pw = w / cols.max(1) as f64;
        let x0 = pw * col.min(cols.saturating_sub(1)) as f64;
        let (tw, th) = bookx_lookup::thumbs::thumb_size(pw, h, target_w * device_scale);
        let surface =
            gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, tw as i32, th as i32)
                .ok()?;
        let cr = gtk::cairo::Context::new(&surface).ok()?;
        cr.scale(tw as f64 / pw, th as f64 / h);
        cr.translate(-x0, 0.0);
        cr.rectangle(x0, 0.0, pw, h);
        cr.clip();
        draw(&s, &cr, w, h);
        drop(cr);
        surface.flush();
        Some(surface)
    }

    /// Test helper: paints the page into a PNG file.
    #[allow(deprecated)]
    pub fn dev_screenshot(&self, path: &str) -> bool {
        let s = self.st.borrow();
        let (w, h) = (s.width.max(1), s.height.max(1));
        let Ok(surface) = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, w, h) else {
            return false;
        };
        let Ok(cr) = gtk::cairo::Context::new(&surface) else {
            return false;
        };
        draw(&s, &cr, w as f64, h as f64);
        drop(cr);
        gdk::pixbuf_get_from_surface(&surface, 0, 0, w, h)
            .is_some_and(|pix| pix.savev(path, "png", &[]).is_ok())
    }

    /// Test helper: `(right-to-left paragraphs, of them with forced optimal breaks, of them
    /// whose last line sits against the right edge)`.
    pub fn dev_rtl_stats(&self) -> (usize, usize, usize) {
        let s = self.st.borrow();
        let col_w = column_geometry(&s)[0].1;
        let (mut rtl, mut kp, mut right) = (0, 0, 0);
        for it in &s.items {
            let Kind::Text(t) = &it.kind else { continue };
            if pango::find_base_dir(&t.text) != pango::Direction::Rtl {
                continue;
            }
            rtl += 1;
            kp += usize::from(t.text.contains(paragraph::LINE_BREAK));
            let mut it = t.layout.iter();
            let mut logical = it.line_extents().1;
            while it.next_line() {
                logical = it.line_extents().1;
            }
            let w = (col_w - t.x).max(0.0);
            if px(logical.x() + logical.width()) >= w - 3.0 && logical.x() > 0 {
                right += 1;
            }
        }
        (rtl, kp, right)
    }

    /// Test helper: `(drop caps, floated images, pieces that continue a wrapped paragraph)`.
    pub fn dev_wrap_stats(&self) -> (usize, usize, usize) {
        let s = self.st.borrow();
        let (mut caps, mut tails) = (0, 0);
        for it in &s.items {
            if let Kind::Text(t) = &it.kind {
                caps += usize::from(t.cap.is_some());
                tails += usize::from(t.cont);
            }
        }
        (caps, s.floats.len(), tails)
    }

    /// Test helper: (milliseconds the last layout took, milliseconds for 30 page turns or screens
    /// forward and back).
    pub fn dev_bench(&self) -> (f64, f64) {
        let layout = self.st.borrow().layout_ms;
        let t0 = std::time::Instant::now();
        for _ in 0..30 {
            self.next();
        }
        for _ in 0..30 {
            self.prev();
        }
        (layout, t0.elapsed().as_secs_f64() * 1000.0)
    }

    /// Test helper: `(paragraphs with optimal breaks, their lines but the last, how many of those
    /// reach the full column width, hyphenated breaks)`.
    pub fn dev_break_stats(&self) -> (usize, usize, usize, usize) {
        let s = self.st.borrow();
        let col_w = column_geometry(&s)[0].1;
        let (mut paras, mut lines, mut full, mut hy) = (0, 0, 0, 0);
        for it in &s.items {
            let Kind::Text(t) = &it.kind else { continue };
            if !t.text.contains(paragraph::LINE_BREAK) {
                continue;
            }
            paras += 1;
            hy += t.text.matches(paragraph::GHOST_HYPHEN).count();
            let n = t.layout.line_count();
            for i in 0..n - 1 {
                let Some(l) = t.layout.line_readonly(i) else {
                    continue;
                };
                let (_, logical) = l.pixel_extents();
                lines += 1;
                if ((logical.x() + logical.width()) as f64 - (col_w - it.x)).abs() <= 2.0 {
                    full += 1;
                }
            }
        }
        (paras, lines, full, hy)
    }

    /// Test helper: (soft hyphens in the laid out text, characters in the canonical text).
    pub fn dev_hyphen_stats(&self) -> (usize, usize) {
        let s = self.st.borrow();
        let mut shy = 0;
        let mut canon = 0;
        for it in &s.items {
            if let Kind::Text(t) = &it.kind {
                shy += t.text.chars().filter(|c| *c == SHY).count();
                canon += t.char_len as usize;
            }
        }
        (shy, canon)
    }

    pub fn set_highlights(&self, hl: Vec<Highlight>) {
        self.st.borrow_mut().highlights = hl;
        self.area.queue_draw();
    }

    /// Test hook: `(id, start, end, text on the page at that range)` of every highlight the
    /// canvas paints. The text is read back through the same mapping the painting uses.
    pub fn dev_painted_highlights(&self) -> Vec<(String, i32, i32, String)> {
        let s = self.st.borrow();
        s.highlights
            .iter()
            .map(|h| {
                (
                    h.id.clone(),
                    h.start,
                    h.end,
                    text_in_range(&s.items, h.start, h.end),
                )
            })
            .collect()
    }

    /// Test hook: `(id, colour as painted `#rrggbb`, style name)` of every highlight the canvas paints.
    pub fn dev_painted_looks(&self) -> Vec<(String, String, String)> {
        let s = self.st.borrow();
        s.highlights
            .iter()
            .map(|h| {
                let (r, g, b) = h.color;
                (
                    h.id.clone(),
                    format!(
                        "#{:02x}{:02x}{:02x}",
                        (r * 255.0).round() as u8,
                        (g * 255.0).round() as u8,
                        (b * 255.0).round() as u8
                    ),
                    h.style.name().to_string(),
                )
            })
            .collect()
    }

    /// Sets search hit ranges (global char offsets) and optionally scrolls to `active`.
    pub fn set_hits(&self, hits: Vec<(i32, i32)>, active: Option<usize>) {
        {
            let mut s = self.st.borrow_mut();
            s.hits = hits;
            s.active_hit = active;
        }
        if let Some(a) = active {
            let start = self.st.borrow().hits.get(a).map(|h| h.0);
            if let Some(c) = start {
                self.goto_char(c);
            }
        }
        self.area.queue_draw();
    }

    /// The whole chapter as canonical text (paragraphs joined by a line break), so that a
    /// character's index in it is its offset for highlights.
    pub fn canonical_text(&self) -> String {
        let s = self.st.borrow();
        let mut out = String::new();
        for it in &s.items {
            if let Kind::Text(t) = &it.kind {
                if !t.cont && !out.is_empty() {
                    out.push('\n');
                }
                out.extend(canon_text(&t.text));
            }
        }
        out
    }

    /// First character on the page (or screen) being shown.
    pub fn visible_start_char(&self) -> i32 {
        let s = self.st.borrow();
        let y0 = match visible_columns(&s).first() {
            Some(c) => c.y0,
            None => return 0,
        };
        let i = s.lines.partition_point(|l| l.y + l.h <= y0);
        for l in &s.lines[i.min(s.lines.len())..] {
            if let Kind::Text(t) = &s.items[l.item].kind {
                let lc = byte_to_char(&t.text, l.start.max(0) as usize);
                return t.char_base + canon_of_layout_char(&t.text, lc) as i32;
            }
        }
        0
    }

    /// Marks the sentence that is read aloud, and shows it when it is not on the page.
    pub fn set_speaking(&self, range: Option<(i32, i32)>) {
        let off_page = {
            let mut s = self.st.borrow_mut();
            s.speaking = range;
            match range {
                Some((a, b)) => visible_range_rects(&s, a, b).is_empty(),
                None => false,
            }
        };
        if let (true, Some((a, _))) = (off_page, range) {
            self.goto_char(a);
        }
        self.area.queue_draw();
    }

    /// Speed reading: marks `word` broadly and its `focus` letter strongly, and brings the word
    /// into view (turning the page) when it is not on the page.
    pub fn set_focus_word(&self, marks: Option<((i32, i32), (i32, i32))>) {
        let off_page = {
            let mut s = self.st.borrow_mut();
            s.focus_word = marks;
            match marks {
                Some((w, _)) => visible_range_rects(&s, w.0, w.1).is_empty(),
                None => false,
            }
        };
        if let (true, Some((w, _))) = (off_page, marks) {
            self.goto_char(w.0);
        }
        self.area.queue_draw();
    }

    pub fn focus_word(&self) -> Option<((i32, i32), (i32, i32))> {
        self.st.borrow().focus_word
    }

    /// The text of a range of characters.
    pub fn text_between(&self, a: i32, b: i32) -> String {
        text_in_range(&self.st.borrow().items, a, b)
    }

    pub fn speaking(&self) -> Option<(i32, i32)> {
        self.st.borrow().speaking
    }

    pub fn clear_selection(&self) {
        {
            let mut s = self.st.borrow_mut();
            s.sel = None;
            s.sel_anchor = None;
            s.kb_sel = None;
        }
        self.area.queue_draw();
        fire(&self.cb.selection, None);
    }

    /// Extends the selection from the keyboard (Shift+arrows): starts at the first character of the
    /// page, moves the free end by `step` and brings it into view. The popover opens with
    /// [`BookCanvas::commit_selection`].
    pub fn key_select(&self, step: Step, forward: bool) {
        let text = self.canonical_text();
        let start = self.visible_start_char();
        let focus = {
            let mut s = self.st.borrow_mut();
            let (anchor, focus) = match (s.kb_sel, s.sel) {
                (Some((a, f)), Some((x, y))) if (a.min(f), a.max(f)) == (x, y) => (a, f),
                _ => (start, start),
            };
            let nf = crate::a11y::move_focus(&text, focus as usize, step, forward) as i32;
            s.kb_sel = Some((anchor, nf));
            s.sel = (anchor != nf).then(|| (anchor.min(nf), anchor.max(nf)));
            s.sel_anchor = Some(anchor);
            nf
        };
        let probe = if forward { focus - 1 } else { focus };
        let visible =
            !visible_range_rects(&self.st.borrow(), probe.max(0), probe.max(0) + 1).is_empty();
        if !visible {
            self.goto_char(probe.max(0));
        }
        self.area.queue_draw();
    }

    /// Opens the actions for a selection made with the keyboard (what releasing the mouse does).
    pub fn commit_selection(&self) -> bool {
        match self.selection_info() {
            Some(info) => {
                fire(&self.cb.selection, Some(info));
                true
            }
            None => false,
        }
    }

    /// Where the ends of the selection are on the screen, for the handles: the rectangle of the
    /// character the selection starts with and of the one it ends with, each only when it is on
    /// the visible page(s).
    pub fn selection_ends(&self) -> Option<SelEnds> {
        let s = self.st.borrow();
        let (a, b) = s.sel?;
        let rects = visible_range_rects(&s, a, b);
        let first = !visible_range_rects(&s, a, a + 1).is_empty();
        let last = !visible_range_rects(&s, b - 1, b).is_empty();
        Some(SelEnds {
            start: first.then(|| rects.first().copied()).flatten(),
            end: last.then(|| rects.last().copied()).flatten(),
            vertical: is_vertical(&s),
            rtl: s.rtl,
        })
    }

    /// A handle is picked up: the other end of the selection stays where it is.
    pub fn handle_drag_begin(&self, moving_is_start: bool) -> bool {
        let mut s = self.st.borrow_mut();
        let Some((a, b)) = s.sel else {
            return false;
        };
        s.handle_anchor = Some(if moving_is_start { b } else { a });
        true
    }

    /// The handle in hand is at (x, y) of the page area (the point of the text it means). With
    /// `word` the moving end snaps to the edge of its word. Returns whether the moving end is now
    /// the start of the selection (it can have crossed the other end).
    pub fn handle_drag_to(&self, x: f64, y: f64, word: bool) -> Option<bool> {
        let moving_start = {
            let mut s = self.st.borrow_mut();
            let anchor = s.handle_anchor?;
            let keep = s.sel?;
            let (mut c, _) = hit(&s, x, y)?;
            if word {
                if let Some((item, local)) = item_for_char(&s.items, c) {
                    if let Kind::Text(t) = &s.items[item].kind {
                        let chars: Vec<char> = canon_text(&t.text).collect();
                        let snapped =
                            crate::selection_handles::snap_word(&chars, local, c < anchor);
                        c = t.char_base + snapped as i32;
                    }
                }
            }
            let (sel, start) = crate::selection_handles::reorder(anchor, c, keep);
            s.sel = Some(sel);
            start
        };
        self.area.queue_draw();
        Some(moving_start)
    }

    /// The handle was let go. The selection is the same as before the drag started when the
    /// handle did not move; the caller shows the action popover again with `commit_selection`.
    pub fn handle_drag_end(&self) {
        self.st.borrow_mut().handle_anchor = None;
    }

    pub fn has_selection(&self) -> bool {
        self.st.borrow().sel.is_some()
    }

    pub fn selected_text(&self) -> Option<String> {
        let s = self.st.borrow();
        let (a, b) = s.sel?;
        Some(text_in_range(&s.items, a, b))
    }

    /// Case-insensitive search over the chapter; returns global char ranges.
    pub fn find_all(&self, query: &str) -> Vec<(i32, i32)> {
        let q: Vec<char> = query.to_lowercase().chars().collect();
        let mut out = Vec::new();
        if q.is_empty() {
            return out;
        }
        let s = self.st.borrow();
        for it in &s.items {
            if let Kind::Text(t) = &it.kind {
                let chars: Vec<char> = canon_text(&t.text)
                    .map(|c| c.to_lowercase().next().unwrap_or(c))
                    .collect();
                let mut i = 0;
                while i + q.len() <= chars.len() {
                    if chars[i..i + q.len()] == q[..] {
                        out.push((t.char_base + i as i32, t.char_base + (i + q.len()) as i32));
                        i += q.len();
                    } else {
                        i += 1;
                    }
                }
            }
        }
        out
    }

    /// Like [`find_all`], for a compiled search (whole words, case, regular expression).
    pub fn find_all_re(&self, re: &regex::Regex) -> Vec<(i32, i32)> {
        let s = self.st.borrow();
        let mut out = Vec::new();
        for it in &s.items {
            if let Kind::Text(t) = &it.kind {
                let text: String = canon_text(&t.text).collect();
                for (a, b) in bookx_lookup::search::char_ranges(re, &text, 2000) {
                    out.push((t.char_base + a as i32, t.char_base + b as i32));
                }
            }
        }
        out
    }

    /// Dev helper: selects the first `n` words.
    pub fn dev_select_first_words(&self, n: usize) {
        {
            let mut s = self.st.borrow_mut();
            let mut total = 0;
            let mut words = 0;
            'outer: for it in &s.items {
                if let Kind::Text(t) = &it.kind {
                    for (i, c) in canon_text(&t.text).enumerate() {
                        if c.is_whitespace() {
                            words += 1;
                            if words >= n {
                                total = t.char_base + i as i32;
                                break 'outer;
                            }
                        }
                    }
                }
            }
            s.sel = Some((0, total.max(1)));
        }
        self.area.queue_draw();
        let info = self.selection_info();
        fire(&self.cb.selection, info);
    }

    /// Test hook: selects the first place where `text` stands in the chapter.
    pub fn dev_select_text(&self, text: &str) -> bool {
        let Some(&(a, b)) = self.find_all(text).first() else {
            return false;
        };
        self.st.borrow_mut().sel = Some((a, b));
        self.area.queue_draw();
        let info = self.selection_info();
        fire(&self.cb.selection, info);
        true
    }

    pub fn select_all(&self) {
        let total = {
            let s = self.st.borrow();
            s.items
                .iter()
                .filter_map(|i| match &i.kind {
                    Kind::Text(t) => Some(t.char_base + t.char_len),
                    _ => None,
                })
                .max()
                .unwrap_or(0)
        };
        self.st.borrow_mut().sel = Some((0, total));
        self.area.queue_draw();
    }

    // ---- navigation ------------------------------------------------------

    /// How the lines of the chapter that is shown run.
    pub fn writing_mode(&self) -> WritingMode {
        mode_of(&self.st.borrow())
    }

    pub fn is_scroll(&self) -> bool {
        self.st.borrow().style.layout == LayoutMode::Scroll
    }

    pub fn fraction(&self) -> f64 {
        let s = self.st.borrow();
        frac_of(&s)
    }

    /// Moves forward one page/screen. Returns `false` if already at the end of the chapter.
    pub fn next(&self) -> bool {
        let moved = {
            let mut s = self.st.borrow_mut();
            if s.style.layout == LayoutMode::Scroll {
                let max = scroll_max(&s);
                if s.scroll_y >= max - 1.0 {
                    false
                } else {
                    s.scroll_y = (s.scroll_y + scroll_step(&s)).min(max);
                    true
                }
            } else {
                let cols = columns(&s);
                if s.cur + cols >= s.pages.len() {
                    false
                } else {
                    s.cur += cols;
                    true
                }
            }
        };
        if moved {
            self.after_move();
        }
        moved
    }

    pub fn prev(&self) -> bool {
        let moved = {
            let mut s = self.st.borrow_mut();
            if s.style.layout == LayoutMode::Scroll {
                if s.scroll_y <= 0.5 {
                    false
                } else {
                    s.scroll_y = (s.scroll_y - scroll_step(&s)).max(0.0);
                    true
                }
            } else if s.cur == 0 {
                false
            } else {
                let cols = columns(&s);
                s.cur = s.cur.saturating_sub(cols);
                true
            }
        };
        if moved {
            self.after_move();
        }
        moved
    }

    pub fn goto_fraction(&self, f: f64) {
        {
            let mut s = self.st.borrow_mut();
            apply_fraction(&mut s, f);
        }
        self.after_move();
    }

    /// Where the text on screen ends, as a fraction of the chapter (the unit of `PosInfo::frac`).
    pub fn view_end_fraction(&self) -> f64 {
        let s = self.st.borrow();
        if s.flow_h <= 0.0 {
            return 0.0;
        }
        let end = if s.style.layout == LayoutMode::Scroll {
            s.scroll_y + scroll_view(&s)
        } else {
            let cols = columns(&s);
            let last = (s.cur + cols - 1).min(s.pages.len().saturating_sub(1));
            s.pages.get(last).map_or(0.0, |p| p.y1)
        };
        (end / s.flow_h).clamp(0.0, 1.0)
    }

    /// Where an anchor is in the open chapter, as a fraction of its height (the unit of `PosInfo::frac`).
    pub fn anchor_fraction(&self, id: &str) -> Option<f64> {
        let s = self.st.borrow();
        if s.flow_h <= 0.0 {
            return None;
        }
        s.items
            .iter()
            .find(|i| i.anchors.iter().any(|a| a == id))
            .map(|i| (i.y / s.flow_h).clamp(0.0, 1.0))
    }

    /// Test helper: the middle of the first picture on screen (widget coordinates) and its path.
    pub fn dev_first_image(&self) -> Option<(f64, f64, String)> {
        let s = self.st.borrow();
        visible_images(&s)
            .into_iter()
            .next()
            .map(|((x, y, w, h), src)| (x + w / 2.0, y + h / 2.0, src))
    }

    /// Test helper: a click at a point, as the mouse delivers it.
    pub fn dev_click(&self, x: f64, y: f64) {
        self.click(x, y);
    }

    pub fn goto_anchor(&self, id: &str) -> bool {
        let found = {
            let mut s = self.st.borrow_mut();
            let y = s
                .items
                .iter()
                .find(|i| i.anchors.iter().any(|a| a == id))
                .map(|i| i.y);
            if let Some(y) = y {
                goto_y(&mut s, y);
                true
            } else {
                false
            }
        };
        if found {
            self.after_move();
        }
        found
    }

    pub fn goto_char(&self, c: i32) {
        {
            let mut s = self.st.borrow_mut();
            if let Some(y) = char_y(&s, c) {
                goto_y(&mut s, y);
            }
        }
        self.after_move();
    }

    /// Like [`BookCanvas::goto_char`], but waits for the layout when the chapter has not been laid
    /// out yet (the canvas has no size before the page is shown), and then wins over the position the
    /// chapter was opened at.
    pub fn goto_char_when_ready(&self, c: i32) {
        let laid_out = {
            let s = self.st.borrow();
            s.width > 0 && s.height > 0 && s.pending_frac.is_none() && s.pending_anchor.is_none()
        };
        if laid_out {
            self.goto_char(c);
        } else {
            self.st.borrow_mut().pending_char = Some(c);
        }
    }

    /// Selects the characters `a..b` (what a highlight opened from a list shows).
    pub fn select_chars(&self, a: i32, b: i32) {
        self.st.borrow_mut().sel = Some((a.min(b), a.max(b)));
        self.area.queue_draw();
    }

    /// The selected characters.
    pub fn selection_range(&self) -> Option<(i32, i32)> {
        self.st.borrow().sel
    }

    /// Whether any of the characters `a..b` is on the page that is shown.
    pub fn range_visible(&self, a: i32, b: i32) -> bool {
        !visible_range_rects(&self.st.borrow(), a, b).is_empty()
    }

    fn after_move(&self) {
        self.hide_hover();
        self.area.queue_draw();
        let info = {
            let s = self.st.borrow();
            pos_info(&s)
        };
        fire(&self.cb.position, info);
    }

    /// Gives the screen reader the text of the page that is shown now.
    pub fn refresh_a11y(&self) {
        let page = visible_page_text(&self.st.borrow());
        self.area.set_page(page);
        self.sync_a11y_selection();
    }

    /// Tells the screen reader which part of the page text is selected.
    fn sync_a11y_selection(&self) {
        let sel = {
            let s = self.st.borrow();
            let page = self.area.page();
            s.sel
                .and_then(|(a, b)| page.selection_on_page(a, b))
                .map(|(a, b)| (a as u32, b as u32))
        };
        self.area.set_selection_range(sel);
    }

    /// The text of the visible page(s), as the screen reader gets it.
    pub fn page_text(&self) -> String {
        self.refresh_a11y();
        self.area.page_text()
    }

    // ---- layout ----------------------------------------------------------

    fn relayout(&self) {
        {
            let mut s = self.st.borrow_mut();
            if s.width <= 0 || s.height <= 0 {
                return;
            }
            let (d, l) = self.work.get();
            self.work.set((d, l + 1));
            let ctxs = Ctxs::new(self.area.upcast_ref(), is_vertical(&s), &s.lang);
            let t0 = std::time::Instant::now();
            build_flow(&mut s, &ctxs);
            paginate(&mut s);
            s.layout_ms = t0.elapsed().as_secs_f64() * 1000.0;
            // restore position
            if s.pending_last {
                s.pending_last = false;
                s.pending_frac = None;
                if s.style.layout == LayoutMode::Scroll {
                    s.scroll_y = scroll_max(&s);
                } else {
                    let cols = columns(&s);
                    s.cur = (s.pages.len().saturating_sub(1) / cols) * cols;
                }
            } else if let Some(id) = s.pending_anchor.take() {
                s.pending_frac = None;
                let y = s
                    .items
                    .iter()
                    .find(|i| i.anchors.contains(&id))
                    .map(|i| i.y);
                if let Some(y) = y {
                    goto_y(&mut s, y);
                }
            } else if let Some(f) = s.pending_frac.take() {
                apply_fraction(&mut s, f);
            }
            if let Some(c) = s.pending_char.take() {
                if let Some(y) = char_y(&s, c) {
                    goto_y(&mut s, y);
                }
            }
            clamp_pos(&mut s);
        }
        self.area.queue_draw();
        let info = {
            let s = self.st.borrow();
            pos_info(&s)
        };
        fire(&self.cb.position, info);
    }

    // ---- wiring ----------------------------------------------------------

    fn wire(&self) {
        let this = self.clone();
        self.area.set_draw_func(move |_, cr, w, h| {
            let (d, l) = this.work.get();
            this.work.set((d + 1, l));
            let key = {
                let s = this.st.borrow();
                draw(&s, cr, w as f64, h as f64);
                if let Some(a) = &s.anim {
                    draw_turn_animation(a, cr, w as f64);
                }
                draw_hints(&s, cr);
                (
                    s.cur,
                    s.scroll_y as i64,
                    s.items.len(),
                    s.lines.len(),
                    s.width,
                    s.layout_ms.to_bits(),
                    s.sel
                        .map_or(-1, |(a, b)| i64::from(a) * 1_000_003 + i64::from(b)),
                )
            };
            if this.a11y_key.get() != key {
                this.a11y_key.set(key);
                let t = this.clone();
                glib::idle_add_local_once(move || t.refresh_a11y());
            }
        });

        let this = self.clone();
        self.area.connect_resize(move |_, w, h| {
            let changed = {
                let mut s = this.st.borrow_mut();
                let changed = s.width != w || s.height != h;
                if changed {
                    s.width = w;
                    s.height = h;
                    if s.doc.is_some() && s.items.is_empty() {
                        // first allocation: position was set by set_doc
                    } else if s.pending_frac.is_none() {
                        s.pending_frac = Some(frac_of(&s));
                    }
                }
                changed
            };
            if changed {
                this.relayout();
            }
        });

        // mouse
        let drag = gtk::GestureDrag::new();
        drag.set_button(1);
        let this = self.clone();
        drag.connect_drag_begin(move |g, x, y| {
            this.area.grab_focus();
            let touch = g
                .current_event_device()
                .is_some_and(|d| d.source() == gdk::InputSource::Touchscreen);
            let mut s = this.st.borrow_mut();
            s.drag_start = Some((x, y));
            s.dragged = false;
            s.touch_drag = touch;
            s.drag_t0 = Some(std::time::Instant::now());
            if !s.long_pressed {
                s.sel_anchor = None;
            }
            // a long press has already put the anchor on the word it selected
            if drag_selects(s.touch_drag, s.long_pressed) && !s.long_pressed {
                let h = hit(&s, x, y).map(|(c, _)| c);
                s.sel_anchor = h;
            }
        });
        let this = self.clone();
        drag.connect_drag_update(move |g, dx, dy| {
            let Some((sx, sy)) = g.start_point() else {
                return;
            };
            let mut changed = false;
            {
                let mut s = this.st.borrow_mut();
                if !s.dragged && (dx * dx + dy * dy).sqrt() > 4.0 {
                    s.dragged = true;
                }
                if s.dragged && drag_selects(s.touch_drag, s.long_pressed) {
                    if let (Some(a), Some((c, _))) = (s.sel_anchor, hit(&s, sx + dx, sy + dy)) {
                        let (lo, hi) = (a.min(c), a.max(c));
                        s.sel = if hi > lo { Some((lo, hi)) } else { None };
                        changed = true;
                    }
                }
            }
            if changed {
                this.area.queue_draw();
            }
        });
        let this = self.clone();
        drag.connect_drag_end(move |_, dx, dy| this.on_release(dx, dy));
        self.area.add_controller(drag);

        // touch screens: a long press selects the word under the finger, dragging then extends it
        let long = gtk::GestureLongPress::new();
        long.set_touch_only(true);
        let this = self.clone();
        long.connect_pressed(move |_, x, y| {
            if on_text(&this.st.borrow(), x, y) {
                this.select_word_at(x, y);
                let mut s = this.st.borrow_mut();
                s.long_pressed = true;
                s.sel_anchor = s.sel.map(|(a, _)| a);
            }
        });
        self.area.add_controller(long);

        let dbl = gtk::GestureClick::new();
        dbl.set_button(1);
        let this = self.clone();
        dbl.connect_pressed(move |_, n, x, y| {
            if n == 2 {
                this.st.borrow_mut().vocab_tok += 1;
                let on_text = on_text(&this.st.borrow(), x, y);
                if on_text {
                    this.st.borrow_mut().word_selected = true;
                    this.select_word_at(x, y);
                } else {
                    fire(&this.cb.double_tap, ());
                }
            }
        });
        self.area.add_controller(dbl);

        let motion = gtk::EventControllerMotion::new();
        let this = self.clone();
        motion.connect_motion(move |_, x, y| {
            let s = this.st.borrow();
            let over_link = hit(&s, x, y).and_then(|(_, hv)| link_at(&s, hv)).is_some();
            this.area
                .set_cursor_from_name(Some(if over_link { "pointer" } else { "text" }));
            drop(s);
            this.update_hover(x, y);
        });
        let this = self.clone();
        motion.connect_leave(move |_| this.hide_hover());
        self.area.add_controller(motion);

        let scroll = gtk::EventControllerScroll::new(
            gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE,
        );
        let this = self.clone();
        scroll.connect_scroll(move |c, _dx, dy| {
            if c.current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK)
            {
                fire(&this.cb.zoom, if dy < 0.0 { 1.07 } else { 1.0 / 1.07 });
            } else {
                this.wheel(dy);
            }
            glib::Propagation::Stop
        });
        self.area.add_controller(scroll);

        // pinch to zoom: apply once when the gesture ends (relayout is not free)
        let pinch = gtk::GestureZoom::new();
        let last = Rc::new(std::cell::Cell::new(1.0));
        {
            let l2 = last.clone();
            pinch.connect_scale_changed(move |_, s| l2.set(s));
            let this = self.clone();
            pinch.connect_end(move |_, _| {
                let s = last.replace(1.0);
                if (s - 1.0).abs() > 0.03 {
                    fire(&this.cb.zoom, s);
                }
            });
        }
        self.area.add_controller(pinch);

        let kb = gtk::EventControllerKey::new();
        let this = self.clone();
        kb.connect_key_pressed(move |_, key, _, mods| {
            use gdk::Key;
            let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
            if ctrl && (key == Key::c || key == Key::C) {
                if let Some(t) = this.selected_text() {
                    this.area.clipboard().set_text(&t);
                }
                return glib::Propagation::Stop;
            }
            if ctrl && (key == Key::a || key == Key::A) {
                this.select_all();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.area.add_controller(kb);
    }

    /// The mouse button or the finger was let go after a press (and maybe a drag).
    fn on_release(&self, dx: f64, dy: f64) {
        let (dragged, start, touch, long, t0, word) = {
            let s = self.st.borrow();
            (
                s.dragged,
                s.drag_start,
                s.touch_drag,
                s.long_pressed,
                s.drag_t0,
                s.word_selected,
            )
        };
        {
            let mut s = self.st.borrow_mut();
            s.long_pressed = false;
            s.word_selected = false;
        }
        match release_action(dragged, touch, long, word) {
            Release::Swipe => {
                // a finger moving across the page: swipe to turn it
                let ms = t0.map_or(u64::MAX, |t| t.elapsed().as_millis() as u64);
                self.apply_swipe(classify_swipe(dx, dy, ms));
            }
            Release::Selection => {
                let info = self.selection_info();
                fire(&self.cb.selection, info);
            }
            Release::Click => {
                if let Some((x, y)) = start {
                    self.click(x, y);
                }
            }
            Release::Keep => {}
        }
    }

    fn wheel(&self, dy: f64) {
        let scroll_mode = self.is_scroll();
        if scroll_mode {
            let (moved, at_end, at_start) = {
                let mut s = self.st.borrow_mut();
                let max = scroll_max(&s);
                let old = s.scroll_y;
                s.scroll_y = (s.scroll_y + dy * 70.0).clamp(0.0, max);
                (
                    (s.scroll_y - old).abs() > 0.01,
                    old >= max - 1.0 && dy > 0.0,
                    old <= 0.5 && dy < 0.0,
                )
            };
            if moved {
                self.st.borrow_mut().overscroll = 0.0;
                self.after_move();
            } else if at_end || at_start {
                let trigger = {
                    let mut s = self.st.borrow_mut();
                    s.overscroll += dy.abs();
                    s.overscroll >= 3.0
                };
                if trigger {
                    self.st.borrow_mut().overscroll = 0.0;
                    fire(&self.cb.edge, if at_end { Edge::Next } else { Edge::Prev });
                }
            }
        } else {
            let turn = {
                let mut s = self.st.borrow_mut();
                s.wheel_acc += dy;
                if s.wheel_acc.abs() >= 1.0 {
                    let d = s.wheel_acc.signum();
                    s.wheel_acc = 0.0;
                    Some(d)
                } else {
                    None
                }
            };
            if let Some(d) = turn {
                self.turn(d > 0.0);
            }
        }
    }

    pub fn turn(&self, forward: bool) {
        let old = self.snapshot_for_animation();
        let moved = if forward { self.next() } else { self.prev() };
        if moved {
            if let Some((surface, kind)) = old {
                self.start_animation(surface, kind, forward);
            }
        } else {
            fire(&self.cb.edge, if forward { Edge::Next } else { Edge::Prev });
        }
    }

    /// Shows a letter on every link of the page (R43); returns how many links there are.
    pub fn show_link_hints(&self) -> usize {
        let n = {
            let mut s = self.st.borrow_mut();
            let links = visible_links(&s);
            let labels = bookx_text::aids::hint_labels(links.len());
            s.hints = links
                .into_iter()
                .zip(labels)
                .map(|((x, y, href), label)| LinkHint { label, x, y, href })
                .collect();
            s.hint_typed.clear();
            s.hints.len()
        };
        self.area.queue_draw();
        n
    }

    pub fn hints_active(&self) -> bool {
        !self.st.borrow().hints.is_empty()
    }

    pub fn clear_link_hints(&self) {
        {
            let mut s = self.st.borrow_mut();
            s.hints.clear();
            s.hint_typed.clear();
        }
        self.area.queue_draw();
    }

    /// A letter typed while the hints show: follows the link it completes.
    pub fn hint_key(&self, c: char) -> HintResult {
        use bookx_text::aids::{match_hint, HintMatch};
        let href = {
            let mut s = self.st.borrow_mut();
            if s.hints.is_empty() {
                return HintResult::Off;
            }
            s.hint_typed.push(c);
            let labels: Vec<String> = s.hints.iter().map(|h| h.label.clone()).collect();
            match match_hint(&labels, &s.hint_typed) {
                HintMatch::Some(_) => None,
                HintMatch::None => {
                    s.hints.clear();
                    s.hint_typed.clear();
                    drop(s);
                    self.area.queue_draw();
                    return HintResult::Cancelled;
                }
                HintMatch::Hit(i) => {
                    let h = s.hints[i].href.clone();
                    s.hints.clear();
                    s.hint_typed.clear();
                    Some(h)
                }
            }
        };
        self.area.queue_draw();
        match href {
            Some(h) => {
                fire(&self.cb.link, h.clone());
                HintResult::Followed(h)
            }
            None => HintResult::Pending,
        }
    }

    /// Test helper: the hints on the page as `label=href`.
    pub fn dev_hints(&self) -> String {
        self.st
            .borrow()
            .hints
            .iter()
            .map(|h| format!("{}={}", h.label, h.href))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The selection as `(start, end, text)`.
    pub fn dev_selection_range(&self) -> Option<(i32, i32, String)> {
        self.selection_info().map(|i| (i.start, i.end, i.text))
    }

    /// The page-turn animation the reader asks for.
    pub fn set_page_animation(&self, kind: bookx_text::aids::PageAnimation) {
        self.st.borrow_mut().anim_kind = kind;
    }

    /// Test helper: whether an animation is running now.
    pub fn dev_animating(&self) -> bool {
        self.st.borrow().anim.is_some()
    }

    /// What a turn plays now: the choice, unless the system wants reduced motion or e-ink is on.
    fn animation_now(&self) -> bookx_text::aids::PageAnimation {
        let reduced = !gtk::Settings::default().is_some_and(|s| s.is_gtk_enable_animations());
        bookx_text::aids::effective_animation(
            self.st.borrow().anim_kind,
            reduced,
            crate::eink::is_active(),
        )
    }

    /// The current page as a picture, when a turn is to be animated (paged layouts only).
    fn snapshot_for_animation(
        &self,
    ) -> Option<(gtk::cairo::ImageSurface, bookx_text::aids::PageAnimation)> {
        let kind = self.animation_now();
        if kind == bookx_text::aids::PageAnimation::None || !self.area.is_mapped() {
            return None;
        }
        let s = self.st.borrow();
        if s.style.layout == LayoutMode::Scroll || s.width < 2 || s.height < 2 {
            return None;
        }
        let surface =
            gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, s.width, s.height).ok()?;
        let cr = gtk::cairo::Context::new(&surface).ok()?;
        draw(&s, &cr, s.width as f64, s.height as f64);
        drop(cr);
        Some((surface, kind))
    }

    fn start_animation(
        &self,
        old: gtk::cairo::ImageSurface,
        kind: bookx_text::aids::PageAnimation,
        forward: bool,
    ) {
        self.st.borrow_mut().anim = Some(TurnAnim {
            old,
            t0: std::time::Instant::now(),
            forward,
            kind,
        });
        let this = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
            this.area.queue_draw();
            let done = this.st.borrow().anim.as_ref().is_none_or(|a| {
                a.t0.elapsed().as_millis() as u64 >= bookx_text::aids::PAGE_ANIMATION_MS
            });
            if done {
                this.st.borrow_mut().anim = None;
                this.area.queue_draw();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    /// Tooltip text (the note) for the highlight under the pointer.
    pub fn note_tooltip(&self, x: f64, y: f64) -> Option<String> {
        let s = self.st.borrow();
        if !on_text(&s, x, y) {
            return None;
        }
        let (c, _) = hit(&s, x, y)?;
        note_at(&s.highlights, c).map(String::from)
    }

    /// Dev/test helper: the tooltip at the centre of the first highlight that has a note.
    pub fn dev_first_note_tooltip(&self) -> Option<String> {
        let (x, y) = {
            let s = self.st.borrow();
            let h = s.highlights.iter().find(|h| !h.note.is_empty())?;
            let r = range_rect(&s, h.start, h.end)?;
            ((r.0 + r.2 / 2) as f64, (r.1 + r.3 / 2) as f64)
        };
        self.note_tooltip(x, y)
    }

    /// Dev/test helper: the selection info of the first highlight that has a note (for screenshots of the popover).
    pub fn dev_note_highlight_info(&self) -> Option<SelectionInfo> {
        let id = {
            let s = self.st.borrow();
            s.highlights.iter().find(|h| !h.note.is_empty())?.id.clone()
        };
        self.highlight_info(&id)
    }

    /// Dev/test helper: a plain click in the middle of the page.
    pub fn dev_click_middle(&self) {
        let (w, h) = {
            let s = self.st.borrow();
            (s.width as f64, s.height as f64)
        };
        self.click(w / 2.0, h - 4.0);
    }

    /// Shows the note of the highlight under the pointer immediately, hides it when leaving.
    fn update_hover(&self, x: f64, y: f64) {
        let now = self.note_tooltip(x, y);
        let action = hover_action(self.hover.cur.borrow().as_deref(), now.as_deref());
        match action {
            HoverAction::Keep => {}
            HoverAction::Hide => self.hide_hover(),
            HoverAction::Show(text) => {
                self.hover.label.set_text(&text);
                self.hover.pop.set_pointing_to(Some(&gdk::Rectangle::new(
                    x as i32,
                    y as i32 - 8,
                    1,
                    1,
                )));
                self.hover.pop.popup();
                *self.hover.cur.borrow_mut() = Some(text);
            }
        }
    }

    fn hide_hover(&self) {
        if self.hover.cur.borrow_mut().take().is_some() {
            self.hover.pop.popdown();
        }
    }

    /// Text currently shown by the hover preview (tests).
    pub fn hover_text(&self) -> Option<String> {
        self.hover.cur.borrow().clone()
    }

    /// Dev/test helper: moves the "pointer" onto the first highlight that has a note.
    pub fn dev_hover_first_note(&self) {
        let pt = {
            let s = self.st.borrow();
            s.highlights
                .iter()
                .find(|h| !h.note.is_empty())
                .and_then(|h| range_rect(&s, h.start, h.end))
                .map(|r| ((r.0 + r.2 / 2) as f64, (r.1 + r.3 / 2) as f64))
        };
        match pt {
            Some((x, y)) => self.update_hover(x, y),
            None => self.hide_hover(),
        }
    }

    fn click(&self, x: f64, y: f64) {
        self.hide_hover();
        // link?
        let (link, hl, w, scroll) = {
            let s = self.st.borrow();
            let h = hit(&s, x, y);
            let link = h.as_ref().and_then(|(_, hv)| link_at(&s, *hv));
            let hl = h.as_ref().and_then(|(c, _)| {
                s.highlights
                    .iter()
                    .find(|hl| *c >= hl.start && *c < hl.end)
                    .map(|hl| hl.id.clone())
            });
            (
                link,
                hl,
                s.width as f64,
                s.style.layout == LayoutMode::Scroll,
            )
        };
        let had_sel = self.st.borrow().sel.is_some();
        if had_sel {
            self.clear_selection();
            return;
        }
        // Any click that is not on a highlight dismisses an open selection / highlight popover.
        if hl.is_none() || link.is_some() {
            fire(&self.cb.selection, None);
        }
        if let Some(l) = link {
            fire(&self.cb.link, l);
            return;
        }
        if let Some(id) = hl {
            let info = self.highlight_info(&id);
            if let Some(i) = info {
                fire(&self.cb.highlight_click, i);
                return;
            }
        }
        if let Some(i) = self.vocab_mark_at(x, y) {
            if self.cb.vocab_click.borrow().is_some() {
                // a double click selects the word: the definition waits for a second click that does not come
                let tok = {
                    let mut s = self.st.borrow_mut();
                    s.vocab_tok += 1;
                    s.vocab_tok
                };
                let this = self.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(320), move || {
                    if this.st.borrow().vocab_tok == tok {
                        fire(&this.cb.vocab_click, i);
                    }
                });
                return;
            }
        }
        let zones = self.st.borrow().zones;
        let action = if scroll || w <= 0.0 {
            TapAction::Toggle
        } else {
            mirror_tap(zones.action_at(x / w), self.st.borrow().rtl)
        };
        // a tap in the middle of a picture opens it (the page-turn zones at the edges still turn pages)
        if action == TapAction::Toggle {
            let src = {
                let s = self.st.borrow();
                if s.style.fixed {
                    None
                } else {
                    image_at(&s, x, y)
                }
            };
            if let Some(src) = src {
                fire(&self.cb.image, src);
                return;
            }
        }
        self.do_tap_action(action);
    }

    fn do_tap_action(&self, a: TapAction) {
        match a {
            TapAction::PrevPage => self.turn(false),
            TapAction::NextPage => self.turn(true),
            TapAction::Toggle => fire(&self.cb.tap, ()),
        }
    }

    fn apply_swipe(&self, s: Option<Swipe>) {
        // a finger moving left turns to the next page of a left-to-right book, to the previous of a right-to-left one
        let ltr = right_is_forward(self.st.borrow().rtl);
        match s {
            Some(Swipe::Left) => self.turn(ltr),
            Some(Swipe::Right) => self.turn(!ltr),
            None => {}
        }
    }

    pub fn set_tap_zones(&self, z: TapZones) {
        self.st.borrow_mut().zones = z;
    }

    /// The book turns pages right to left.
    pub fn set_rtl(&self, rtl: bool) {
        self.st.borrow_mut().rtl = rtl;
    }

    /// Test helper: a finger movement of `(dx, dy)` pixels in `ms` milliseconds.
    pub fn dev_swipe(&self, dx: f64, dy: f64, ms: u64) -> Option<Swipe> {
        let s = classify_swipe(dx, dy, ms);
        self.apply_swipe(s);
        s
    }

    /// Test helper: a tap at horizontal position `x` (0..1) on empty page area.
    pub fn dev_tap(&self, x: f64) -> TapAction {
        let a = mirror_tap(self.st.borrow().zones.action_at(x), self.st.borrow().rtl);
        self.do_tap_action(a);
        a
    }

    fn select_word_at(&self, x: f64, y: f64) {
        {
            let mut s = self.st.borrow_mut();
            let Some((c, _)) = hit(&s, x, y) else { return };
            let Some((item_idx, local)) = item_for_char(&s.items, c) else {
                return;
            };
            let Kind::Text(t) = &s.items[item_idx].kind else {
                return;
            };
            let chars: Vec<char> = canon_text(&t.text).collect();
            let mut a = local.min(chars.len());
            let mut b = a;
            let is_w = |c: char| c.is_alphanumeric() || c == '\'' || c == '’' || c == '-';
            while a > 0 && is_w(chars[a - 1]) {
                a -= 1;
            }
            while b < chars.len() && is_w(chars[b]) {
                b += 1;
            }
            if b > a {
                s.sel = Some((t.char_base + a as i32, t.char_base + b as i32));
            }
        }
        self.area.queue_draw();
        let info = self.selection_info();
        fire(&self.cb.selection, info);
    }

    fn selection_info(&self) -> Option<SelectionInfo> {
        let s = self.st.borrow();
        let (a, b) = s.sel?;
        let text = text_in_range(&s.items, a, b);
        let rect = range_rect(&s, a, b)?;
        let highlight = s
            .highlights
            .iter()
            .find(|h| h.start <= a && h.end >= b)
            .map(|h| h.id.clone());
        Some(SelectionInfo {
            text,
            start: a,
            end: b,
            rect,
            highlight,
        })
    }

    fn highlight_info(&self, id: &str) -> Option<SelectionInfo> {
        let s = self.st.borrow();
        let h = s.highlights.iter().find(|h| h.id == id)?;
        let rect = range_rect(&s, h.start, h.end).unwrap_or((s.width / 2, s.height / 2, 1, 1));
        Some(SelectionInfo {
            text: text_in_range(&s.items, h.start, h.end),
            start: h.start,
            end: h.end,
            rect,
            highlight: Some(h.id.clone()),
        })
    }
}

// ===========================================================================
// Geometry helpers
// ===========================================================================

fn columns(s: &State) -> usize {
    if s.style.layout == LayoutMode::TwoColumns {
        2
    } else {
        1
    }
}

fn content_height(s: &State) -> f64 {
    (s.height as f64 - 2.0 * V_MARGIN).max(40.0)
}

/// How the lines of the chapter run (a fixed-layout page is always horizontal).
fn mode_of(s: &State) -> WritingMode {
    if s.style.fixed {
        WritingMode::HorizontalTb
    } else {
        s.doc
            .as_ref()
            .map_or(WritingMode::HorizontalTb, |d| d.writing_mode)
    }
}

fn is_vertical(s: &State) -> bool {
    mode_of(s).is_vertical()
}

/// (x, width) of each column, or of each page of a vertical book (the text of a page runs down;
/// its width is what the columns of the flow stack in).
fn column_geometry(s: &State) -> Vec<(f64, f64)> {
    let w = s.width as f64;
    let m = s.style.margin.min(w / 4.0);
    match s.style.layout {
        LayoutMode::TwoColumns => {
            let gutter = (m * 0.8).max(36.0);
            let cw = ((w - 2.0 * m - gutter) / 2.0).max(120.0);
            vec![(m, cw), (m + cw + gutter, cw)]
        }
        LayoutMode::SingleColumn => {
            let cw = (w - 2.0 * m).clamp(120.0, 760.0);
            vec![((w - cw) / 2.0, cw)]
        }
        LayoutMode::Scroll if is_vertical(s) => {
            // columns scroll sideways: the whole width is the window onto the flow
            vec![(m, (w - 2.0 * m).max(120.0))]
        }
        LayoutMode::Scroll => {
            let cw = (w - 2.0 * m).clamp(120.0, 780.0);
            vec![((w - cw) / 2.0, cw)]
        }
    }
}

/// How long a line is: the width of a column, or the height of the page when the text is vertical.
fn line_extent(s: &State) -> f64 {
    if is_vertical(s) {
        content_height(s)
    } else {
        column_geometry(s)[0].1
    }
}

/// How much of the flow a page holds: the height of the page, or its width when the text is vertical.
fn block_extent(s: &State) -> f64 {
    if is_vertical(s) {
        column_geometry(s)[0].1
    } else {
        content_height(s)
    }
}

/// The furthest the flow can be scrolled.
fn scroll_max(s: &State) -> f64 {
    let view = if is_vertical(s) {
        column_geometry(s)[0].1
    } else {
        s.height as f64 - 2.0 * V_MARGIN
    };
    (s.flow_h - view).max(0.0)
}

/// How much of the flow a scrolled view shows.
fn scroll_view(s: &State) -> f64 {
    if is_vertical(s) {
        column_geometry(s)[0].1
    } else {
        s.height as f64 - V_MARGIN
    }
}

/// How far a page-down scrolls.
fn scroll_step(s: &State) -> f64 {
    if is_vertical(s) {
        column_geometry(s)[0].1 * 0.9
    } else {
        s.height as f64 * 0.9
    }
}

fn current_top_y(s: &State) -> f64 {
    if s.style.layout == LayoutMode::Scroll {
        s.scroll_y
    } else {
        s.pages.get(s.cur).map(|p| p.y0).unwrap_or(0.0)
    }
}

fn frac_of(s: &State) -> f64 {
    if s.flow_h <= 0.0 {
        return 0.0;
    }
    (current_top_y(s) / s.flow_h).clamp(0.0, 1.0)
}

fn pos_info(s: &State) -> PosInfo {
    if s.style.layout == LayoutMode::Scroll {
        let page_h = scroll_view(s).max(1.0);
        PosInfo {
            page: (s.scroll_y / page_h) as usize + 1,
            pages: ((s.flow_h / page_h).ceil() as usize).max(1),
            frac: frac_of(s),
        }
    } else {
        PosInfo {
            page: s.cur + 1,
            pages: s.pages.len().max(1),
            frac: frac_of(s),
        }
    }
}

/// The `y` of the line that holds character `c` of the chapter.
fn char_y(s: &State, c: i32) -> Option<f64> {
    s.items.iter().find_map(|i| match &i.kind {
        Kind::Text(t) if c >= t.char_base && c <= t.char_base + t.char_len => {
            let lc = layout_char_of_canon(&t.text, (c - t.char_base) as usize);
            let byte = char_to_byte(&t.text, lc) as i32;
            let line_y = s
                .lines
                .iter()
                .find(|l| std::ptr::eq(&s.items[l.item], i) && byte >= l.start && byte <= l.end)
                .map(|l| l.y);
            Some(line_y.unwrap_or(i.y))
        }
        _ => None,
    })
}

fn goto_y(s: &mut State, y: f64) {
    if s.style.layout == LayoutMode::Scroll {
        let max = scroll_max(s);
        s.scroll_y = (y - 4.0).clamp(0.0, max);
    } else {
        let idx = s.pages.iter().rposition(|p| p.y0 <= y + 0.5).unwrap_or(0);
        let cols = columns(s);
        s.cur = (idx / cols) * cols;
    }
}

fn apply_fraction(s: &mut State, f: f64) {
    let y = f.clamp(0.0, 1.0) * s.flow_h;
    // snap to the line at/after y
    let y = s
        .lines
        .iter()
        .find(|l| l.y + l.h > y)
        .map(|l| l.y)
        .unwrap_or(y);
    goto_y(s, y);
}

fn clamp_pos(s: &mut State) {
    if s.style.layout == LayoutMode::Scroll {
        let max = scroll_max(s);
        s.scroll_y = s.scroll_y.clamp(0.0, max);
    } else {
        let cols = columns(s);
        if s.cur >= s.pages.len() {
            s.cur = (s.pages.len().saturating_sub(1) / cols) * cols;
        }
    }
}

/// Soft hyphen: inserted by the hyphenator, invisible unless a line breaks there.
/// What letting go of the mouse button (or the finger) does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Release {
    /// A finger moved across the page: turn it.
    Swipe,
    /// A drag selected text: offer what it selected.
    Selection,
    /// A plain click or tap.
    Click,
    /// A long press or a double click has just selected a word: leave the selection alone.
    Keep,
}

fn release_action(dragged: bool, touch: bool, long_press: bool, word_selected: bool) -> Release {
    if dragged && touch && !long_press {
        Release::Swipe
    } else if dragged {
        Release::Selection
    } else if long_press || word_selected {
        Release::Keep
    } else {
        Release::Click
    }
}

const SHY: char = '\u{ad}';

/// Index (in chars of the laid out text) of the `c`-th canonical character, see
/// [`paragraph::canonical_chars`]: soft hyphens and the hyphen of an optimal line break do not count.
fn layout_char_of_canon(text: &str, c: usize) -> usize {
    paragraph::canonical_chars(text)
        .nth(c)
        .map_or_else(|| text.chars().count(), |(i, _)| i)
}

/// Number of canonical characters before layout index `i`.
fn canon_of_layout_char(text: &str, i: usize) -> usize {
    paragraph::canonical_chars(text)
        .take_while(|(k, _)| *k < i)
        .count()
}

/// The book's text of a laid out string (ghosts removed, line breaks read as spaces).
fn canon_text(text: &str) -> impl Iterator<Item = char> + '_ {
    paragraph::canonical_chars(text).map(|(_, c)| c)
}

pub fn byte_to_char(s: &str, b: usize) -> usize {
    let b = b.min(s.len());
    let mut b = b;
    while !s.is_char_boundary(b) {
        b -= 1;
    }
    s[..b].chars().count()
}

pub fn char_to_byte(s: &str, c: usize) -> usize {
    s.char_indices().nth(c).map(|(i, _)| i).unwrap_or(s.len())
}

fn item_for_char(items: &[Item], c: i32) -> Option<(usize, usize)> {
    items.iter().enumerate().find_map(|(i, it)| match &it.kind {
        Kind::Text(t) if c >= t.char_base && c <= t.char_base + t.char_len => {
            Some((i, (c - t.char_base) as usize))
        }
        _ => None,
    })
}

fn text_in_range(items: &[Item], a: i32, b: i32) -> String {
    let mut out = String::new();
    for it in items {
        if let Kind::Text(t) = &it.kind {
            let lo = a.max(t.char_base);
            let hi = b.min(t.char_base + t.char_len);
            if hi > lo {
                if !out.is_empty() && !t.cont {
                    out.push_str("\n\n");
                }
                let bl = char_to_byte(
                    &t.text,
                    layout_char_of_canon(&t.text, (lo - t.char_base) as usize),
                );
                let bh = char_to_byte(
                    &t.text,
                    layout_char_of_canon(&t.text, (hi - t.char_base) as usize),
                );
                out.extend(canon_text(&t.text[bl..bh]));
            }
        }
    }
    out
}

/// The pictures on screen: their rectangle in widget coordinates and their path in the book.
fn visible_images(s: &State) -> Vec<((f64, f64, f64, f64), String)> {
    let mut out = Vec::new();
    if mode_of(s).is_vertical() {
        return out;
    }
    for col in visible_columns(s) {
        for item in &s.items {
            if let Kind::Image { pix, src, .. } = &item.kind {
                if item.y < col.y1 && item.y + item.h > col.y0 {
                    let r = col
                        .frame
                        .rect(item.x, item.y, pix.width() as f64, pix.height() as f64);
                    out.push((r, src.clone()));
                }
            }
        }
        for f in &s.floats {
            if f.y < col.y1 && f.y + f.h > col.y0 {
                let r = col
                    .frame
                    .rect(f.x, f.y, f.pix.width() as f64, f.pix.height() as f64);
                out.push((r, f.src.clone()));
            }
        }
    }
    out
}

fn image_at(s: &State, x: f64, y: f64) -> Option<String> {
    visible_images(s)
        .into_iter()
        .find(|((rx, ry, rw, rh), _)| x >= *rx && x < rx + rw && y >= *ry && y < ry + rh)
        .map(|(_, src)| src)
}

fn link_at(s: &State, hit: (usize, i32)) -> Option<String> {
    let (item, byte) = hit;
    if let Kind::Text(t) = &s.items.get(item)?.kind {
        let b = byte as usize;
        return t
            .links
            .iter()
            .find(|(a, e, _)| b >= *a && b < *e)
            .map(|(_, _, h)| h.clone());
    }
    None
}

/// Returns the global char under the point and `(item, byte)` for link lookups.
/// The two are packed so that `hit(..).map(|(c, _)| c)` yields the char.
fn hit(s: &State, x: f64, y: f64) -> Option<(i32, (usize, i32))> {
    // Wrapper to keep the call sites compact.
    let (flow_y, inline) = to_flow(s, x, y)?;
    // nearest line
    let idx = s.lines.partition_point(|l| l.y + l.h <= flow_y);
    let l = s.lines.get(idx).or_else(|| s.lines.last())?;
    let item = &s.items[l.item];
    match &item.kind {
        Kind::Text(t) => {
            let lx = (inline - item.x) * SCALE;
            let ly = (l.ly + l.h / 2.0) * SCALE;
            let (_, index, trailing) = t.layout.xy_to_index(lx as i32, ly as i32);
            // clamp to line range
            let mut index = index.clamp(l.start, l.end.max(l.start));
            let mut lc = byte_to_char(&t.text, index.max(0) as usize);
            if trailing > 0 && index < l.end {
                lc += 1;
                index = char_to_byte(&t.text, lc) as i32;
            }
            let c = canon_of_layout_char(&t.text, lc) as i32;
            let _ = index;
            Some((t.char_base + c, (l.item, index.min(l.end - 1).max(l.start))))
        }
        _ => None,
    }
}

/// True when the point is on a glyph run (not in a margin, gap or past the end of a line).
fn on_text(s: &State, x: f64, y: f64) -> bool {
    let Some((flow_y, inline)) = to_flow(s, x, y) else {
        return false;
    };
    let idx = s.lines.partition_point(|l| l.y + l.h <= flow_y);
    let Some(l) = s.lines.get(idx) else {
        return false;
    };
    if flow_y < l.y {
        return false;
    }
    let item = &s.items[l.item];
    let Kind::Text(t) = &item.kind else {
        return false;
    };
    let lx = (inline - item.x) * SCALE;
    let ly = (l.ly + l.h / 2.0) * SCALE;
    let (inside, _, _) = t.layout.xy_to_index(lx as i32, ly as i32);
    inside
}

/// Maps widget coords to `(position in the flow, position along the line)`; the second is
/// counted from the start of the line (the left edge of a column, the top of a vertical one).
fn to_flow(s: &State, x: f64, y: f64) -> Option<(f64, f64)> {
    let cols = column_geometry(s);
    let (ci, _) = cols
        .iter()
        .enumerate()
        .rev()
        .find(|(_, (cx, _))| x >= *cx - 12.0)
        .or_else(|| cols.iter().enumerate().next())?;
    if s.style.layout == LayoutMode::Scroll {
        let fr = scroll_frame(s);
        let (u, v) = fr.locate(x, y);
        Some((v, u))
    } else {
        let page = s
            .pages
            .get(page_of_column(s, ci))
            .or_else(|| s.pages.last())?;
        let fr = page_frame(s, ci, page);
        let (u, v) = fr.locate(x, y);
        Some((v.clamp(page.y0, page.y1 - 0.5), u))
    }
}

/// Rectangle (widget coords) covering the first visible line of a char range.
fn range_rect(s: &State, a: i32, b: i32) -> Option<(i32, i32, i32, i32)> {
    let ranges = visible_range_rects(s, a, b);
    ranges.first()?;
    let x0 = ranges.iter().map(|r| r.0).fold(f64::MAX, f64::min);
    let x1 = ranges.iter().map(|r| r.0 + r.2).fold(f64::MIN, f64::max);
    let y0 = ranges.iter().map(|r| r.1).fold(f64::MAX, f64::min);
    let y1 = ranges.iter().map(|r| r.1 + r.3).fold(f64::MIN, f64::max);
    Some((
        x0 as i32,
        y0 as i32,
        (x1 - x0).max(2.0) as i32,
        (y1 - y0) as i32,
    ))
}

/// Widget-space rectangles for a char range on the currently visible page(s).
fn visible_range_rects(s: &State, a: i32, b: i32) -> Vec<(f64, f64, f64, f64)> {
    let mut out = Vec::new();
    for col in visible_columns(s) {
        let first = s.lines.partition_point(|l| l.y + l.h <= col.y0);
        for l in &s.lines[first..] {
            if l.y >= col.y1 {
                break;
            }
            let item = &s.items[l.item];
            let Kind::Text(t) = &item.kind else { continue };
            let lo = a.max(t.char_base);
            let hi = b.min(t.char_base + t.char_len);
            if hi <= lo {
                continue;
            }
            let bl = char_to_byte(
                &t.text,
                layout_char_of_canon(&t.text, (lo - t.char_base) as usize),
            ) as i32;
            let bh = char_to_byte(
                &t.text,
                layout_char_of_canon(&t.text, (hi - t.char_base) as usize),
            ) as i32;
            let (sb, eb) = (bl.max(l.start), bh.min(l.end));
            if eb <= sb {
                continue;
            }
            let Some(line) = t.layout.line_readonly(l.line) else {
                continue;
            };
            let x0 = px(line.index_to_x(sb, false));
            let x1 = px(line.index_to_x(eb, false));
            let (x0, x1) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
            out.push(
                col.frame
                    .rect(item.x + l.lx + x0, l.y, (x1 - x0).max(1.0), l.h),
            );
        }
    }
    out
}

/// The paragraphs on the visible page(s) with the start of each visual line.
fn visible_page_text(s: &State) -> PageText {
    // (y of the first line or of the picture, paragraph)
    let mut paras: Vec<(f64, (String, Vec<usize>, i32))> = Vec::new();
    for col in visible_columns(s) {
        let (y0, y1) = (col.y0, col.y1);
        let first = s.lines.partition_point(|l| l.y + l.h <= y0);
        let mut group: Option<(usize, Vec<&LineBox>)> = None;
        let flush = |g: Option<(usize, Vec<&LineBox>)>,
                     out: &mut Vec<(f64, (String, Vec<usize>, i32))>| {
            let Some((item, lines)) = g else { return };
            let Kind::Text(t) = &s.items[item].kind else {
                return;
            };
            let sb = (lines[0].start.max(0) as usize).min(t.text.len());
            let eb = (lines[lines.len() - 1].end.max(0) as usize).min(t.text.len());
            if eb <= sb || !t.text.is_char_boundary(sb) || !t.text.is_char_boundary(eb) {
                return;
            }
            let text: String = canon_text(&t.text[sb..eb]).collect();
            let starts: Vec<usize> = lines
                .iter()
                .map(|l| {
                    let b = (l.start.max(0) as usize).clamp(sb, eb);
                    if t.text.is_char_boundary(b) {
                        canon_text(&t.text[sb..b]).count()
                    } else {
                        0
                    }
                })
                .collect();
            if !text.trim().is_empty() {
                let global =
                    t.char_base + canon_of_layout_char(&t.text, byte_to_char(&t.text, sb)) as i32;
                out.push((lines[0].y, (text, starts, global)));
            }
        };
        for l in &s.lines[first.min(s.lines.len())..] {
            if l.y >= y1 {
                break;
            }
            if !matches!(s.items[l.item].kind, Kind::Text(_)) {
                continue;
            }
            match &mut group {
                Some((item, v)) if *item == l.item => v.push(l),
                _ => {
                    flush(group.take(), &mut paras);
                    group = Some((l.item, vec![l]));
                }
            }
        }
        flush(group.take(), &mut paras);
        // the pictures on this page: what the author says they show
        let pictures = s
            .items
            .iter()
            .filter_map(|i| match &i.kind {
                Kind::Image { alt, .. } => Some((i.y, i.h, alt.as_str())),
                _ => None,
            })
            .chain(s.floats.iter().map(|f| (f.y, f.h, f.alt.as_str())));
        for (y, h, alt) in pictures {
            if y < y1 && y + h > y0 {
                if let Some(p) = crate::a11y::image_paragraph(alt) {
                    paras.push((y, (p, vec![0], crate::a11y::IMAGE_CHAR_BASE)));
                }
            }
        }
    }
    paras.sort_by(|a, b| a.0.total_cmp(&b.0));
    let paras: Vec<(String, Vec<usize>, i32)> = paras.into_iter().map(|p| p.1).collect();
    PageText::from_paragraphs(&paras)
}

/// A page (or the visible part of a scrolled chapter) on the screen.
struct Col {
    frame: Frame,
    /// The stretch of the flow it shows.
    y0: f64,
    y1: f64,
}

/// Index in `State::pages` of the page drawn in the `ci`-th column from the left. A vertical book
/// that turns right to left has its first page on the right.
fn page_of_column(s: &State, ci: usize) -> usize {
    let n = columns(s);
    if mode_of(s) == WritingMode::VerticalRl {
        s.cur + (n - 1 - ci.min(n - 1))
    } else {
        s.cur + ci
    }
}

fn page_frame(s: &State, ci: usize, page: &Page) -> Frame {
    let (cx, cw) = column_geometry(s)[ci];
    Frame::new(mode_of(s), (cx, V_MARGIN, cw, content_height(s)), page.y0)
}

fn scroll_frame(s: &State) -> Frame {
    let (cx, cw) = column_geometry(s)[0];
    if is_vertical(s) {
        Frame::new(
            mode_of(s),
            (cx, V_MARGIN, cw, content_height(s)),
            s.scroll_y,
        )
    } else {
        Frame::new(
            mode_of(s),
            (cx, V_MARGIN, cw, s.height as f64 - V_MARGIN),
            s.scroll_y,
        )
    }
}

/// The visible pages, in reading order.
fn visible_columns(s: &State) -> Vec<Col> {
    if s.style.layout == LayoutMode::Scroll {
        return vec![Col {
            frame: scroll_frame(s),
            y0: s.scroll_y,
            y1: s.scroll_y + scroll_view(s),
        }];
    }
    (0..columns(s))
        .filter_map(|i| {
            let ci = if mode_of(s) == WritingMode::VerticalRl {
                columns(s) - 1 - i
            } else {
                i
            };
            let p = s.pages.get(s.cur + i)?;
            Some(Col {
                frame: page_frame(s, ci, p),
                y0: p.y0,
                y1: p.y1,
            })
        })
        .collect()
}

// ===========================================================================
// Layout
// ===========================================================================

/// A run of text with the Pango markup tags around it.
struct Part {
    open: String,
    text: String,
    close: String,
}

impl Part {
    fn plain(text: &str) -> Part {
        Part {
            open: String::new(),
            text: text.to_string(),
            close: String::new(),
        }
    }
}

fn markup_of(parts: &[Part]) -> String {
    let mut m = String::new();
    for p in parts {
        m.push_str(&p.open);
        m.push_str(&glib::markup_escape_text(&p.text));
        m.push_str(&p.close);
    }
    m
}

#[cfg(test)]
fn markup_for(
    span: &Span,
    palette: &Palette,
    original_colors: bool,
    hy: Option<&Hyphenator>,
) -> Option<(String, String, bool)> {
    let (part, is_link) = markup_part(span, palette, original_colors, hy, true)?;
    Some((markup_of(std::slice::from_ref(&part)), part.text, is_link))
}

fn markup_part(
    span: &Span,
    palette: &Palette,
    original_colors: bool,
    hy: Option<&Hyphenator>,
    book_fonts: bool,
) -> Option<(Part, bool)> {
    let Span::Text { text, style, link } = span else {
        return None;
    };
    // the optimal line breaker uses a few Unicode characters as markers
    let text = paragraph::sanitize(text);
    // code is never hyphenated
    let text = match hy {
        Some(h) if !style.code => h.insert_soft_hyphens(&text),
        _ => text,
    };
    let mut attrs = String::new();
    if style.bold {
        attrs.push_str(" weight=\"bold\"");
    }
    if style.italic {
        attrs.push_str(" style=\"italic\"");
    }
    if style.code {
        attrs.push_str(" font_family=\"monospace\" size=\"92%\"");
    }
    if style.underline {
        attrs.push_str(" underline=\"single\"");
    }
    if style.strike {
        attrs.push_str(" strikethrough=\"true\"");
    }
    if style.small {
        attrs.push_str(" size=\"smaller\"");
    }
    if style.small_caps {
        // Pango sets the lower case letters as small capitals, with the font's own when it has them
        attrs.push_str(" font_scale=\"small-caps\"");
    }
    if book_fonts && !style.code {
        if let Some(f) = style.font.as_deref().filter(|f| is_plain_font_name(f)) {
            attrs.push_str(&format!(" font_family=\"{f}\""));
        }
        if let Some(p) = style.size_pct {
            attrs.push_str(&format!(" size=\"{}%\"", p.clamp(50, 300)));
        }
    }
    if let (true, Some((r, g, b)), None) = (original_colors, style.color, link) {
        attrs.push_str(&format!(" foreground=\"#{r:02x}{g:02x}{b:02x}\""));
    }
    if link.is_some() {
        let (r, g, b) = palette.link;
        attrs.push_str(&format!(
            " foreground=\"#{:02x}{:02x}{:02x}\"",
            (r * 255.0) as u8,
            (g * 255.0) as u8,
            (b * 255.0) as u8
        ));
    }
    let (mut open, mut close) = if attrs.is_empty() {
        (String::new(), String::new())
    } else {
        (format!("<span{attrs}>"), "</span>".to_string())
    };
    if style.sup {
        open.insert_str(0, "<sup>");
        close.push_str("</sup>");
    }
    if style.sub {
        open.insert_str(0, "<sub>");
        close.push_str("</sub>");
    }
    Some((Part { open, text, close }, link.is_some()))
}

/// The Pango weight nearest to a CSS weight (100 to 900).
fn weight_of(w: i32) -> pango::Weight {
    match (w.clamp(100, 900) + 50) / 100 * 100 {
        100 => pango::Weight::Thin,
        200 => pango::Weight::Ultralight,
        300 => pango::Weight::Light,
        400 => pango::Weight::Normal,
        500 => pango::Weight::Medium,
        600 => pango::Weight::Semibold,
        700 => pango::Weight::Bold,
        800 => pango::Weight::Ultrabold,
        _ => pango::Weight::Heavy,
    }
}

/// A font name that can sit inside a markup attribute.
fn is_plain_font_name(f: &str) -> bool {
    !f.is_empty()
        && f.chars().count() <= 64
        && f.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '.' | '_'))
}

/// The markup of a layout: kerning and the space between words of the style wrap the text.
fn wrap_text_markup(markup: String, kerning: bool, word_spacing: f64, em_px: f64) -> String {
    let mut m = markup;
    if word_spacing != 0.0 && word_spacing.is_finite() {
        // letter spacing of the spaces only, in Pango units
        let units = (word_spacing.clamp(-0.25, 1.0) * em_px * SCALE).round() as i32;
        if units != 0 {
            let sp = format!("<span letter_spacing=\"{units}\"> </span>");
            m = replace_spaces_outside_tags(&m, &sp);
        }
    }
    if !kerning {
        m = format!("<span font_features=\"kern=0\">{m}</span>");
    }
    m
}

/// Replaces the spaces of the text (not those inside `<...>` tags) with `with`.
fn replace_spaces_outside_tags(markup: &str, with: &str) -> String {
    let mut out = String::with_capacity(markup.len() + markup.len() / 4);
    let mut in_tag = false;
    for c in markup.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            ' ' if !in_tag => {
                out.push_str(with);
                continue;
            }
            _ => {}
        }
        out.push(c);
    }
    out
}

fn load_pixbuf(s: &mut State, path: &str) -> Option<Pixbuf> {
    if let Some(p) = s.pix_cache.get(path) {
        return p.clone();
    }
    let data = (s.images)(path);
    let pix = data.and_then(|d| {
        let stream = gtk::gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(d));
        Pixbuf::from_stream(&stream, gtk::gio::Cancellable::NONE).ok()
    });
    s.pix_cache.insert(path.to_string(), pix.clone());
    pix
}

struct Pending {
    parts: Vec<Part>,
    text: String,
    links: Vec<(usize, usize, String)>,
    ruby: Vec<(usize, usize, String)>,
    combine: Vec<(usize, usize)>,
}

impl Pending {
    fn new() -> Pending {
        Pending {
            parts: Vec::new(),
            text: String::new(),
            links: Vec::new(),
            ruby: Vec::new(),
            combine: Vec::new(),
        }
    }
}

/// The Pango contexts a chapter is laid out with: the widget's own for horizontal text, and for
/// vertical lines one whose glyphs are turned by the gravity of the line (`mixed`: ideographs
/// upright, Latin turned; `upright`: everything upright). Text turned a quarter like Latin
/// (`text-orientation: sideways`) uses the upright context: the page is turned instead.
struct Ctxs {
    plain: pango::Context,
    mixed: Option<pango::Context>,
    upright: Option<pango::Context>,
}

impl Ctxs {
    fn new(area: &gtk::Widget, vertical: bool, lang: &str) -> Ctxs {
        let make = |hint: pango::GravityHint| {
            let c = area.create_pango_context();
            c.set_base_gravity(pango::Gravity::East);
            c.set_gravity_hint(hint);
            if !lang.is_empty() {
                c.set_language(Some(&pango::Language::from_string(lang)));
            }
            c
        };
        Ctxs {
            plain: area.pango_context(),
            mixed: vertical.then(|| make(pango::GravityHint::Natural)),
            upright: vertical.then(|| make(pango::GravityHint::Strong)),
        }
    }

    fn pick(&self, vertical: bool, o: TextOrientation) -> &pango::Context {
        match (vertical, o) {
            (true, TextOrientation::Mixed) => self.mixed.as_ref().unwrap_or(&self.plain),
            (true, TextOrientation::Upright) => self.upright.as_ref().unwrap_or(&self.plain),
            _ => &self.plain,
        }
    }
}

fn build_flow(s: &mut State, ctxs: &Ctxs) {
    s.items.clear();
    s.floats.clear();
    s.lines.clear();
    let Some(doc) = s.doc.clone() else { return };
    let vertical = is_vertical(s);
    let ctx: &pango::Context = ctxs.pick(vertical, doc.orientation);
    let col_w = line_extent(s);
    let style = s.style.clone();
    let em = style.font_size * 96.0 / 72.0;
    let max_img_h = block_extent(s) * if s.style.fixed { 1.0 } else { 0.92 };
    let hy = style
        .hyphenate
        .then(|| Hyphenator::for_language(&s.lang))
        .filter(|h| h.is_supported());

    let mut y: f64 = 0.0;
    let mut prev_bottom: f64 = 0.0;
    let mut char_base: i32 = 0;
    let mut first = true;
    let mut zone: Option<Zone> = None;

    for block in &doc.blocks {
        // Split block content at images.
        let mut pieces: Vec<Result<Pending, (String, FloatSide, String)>> = Vec::new();
        let mut cur = Pending::new();
        let mut anchors: Vec<String> = Vec::new();
        if let Some(m) = &block.marker {
            let t = format!("{m}\t");
            cur.parts.push(Part::plain(&t));
            cur.text.push_str(&t);
        }
        for sp in &block.spans {
            match sp {
                Span::Text {
                    link, style: sty, ..
                } => {
                    // annotated and combined text is never hyphenated
                    let decorated =
                        sty.ruby.is_some() || (vertical && sty.combine != TextCombine::None);
                    let hy_here = if decorated { None } else { hy.as_ref() };
                    if let Some((mut part, is_link)) = markup_part(
                        sp,
                        &style.palette,
                        style.original_colors,
                        hy_here,
                        style.book_fonts,
                    ) {
                        let start = cur.text.len();
                        if let Some(rt) = &sty.ruby {
                            // an annotation wider than its base spreads the base
                            let spread = vertical::ruby_spread(&part.text, rt, RUBY_SCALE);
                            if spread > 0.0 {
                                let ls = (spread * style.font_size * SCALE) as i32;
                                part.open
                                    .push_str(&format!("<span letter_spacing=\"{ls}\">"));
                                part.close.insert_str(0, "</span>");
                            }
                        }
                        cur.text.push_str(&part.text);
                        if let Some(rt) = &sty.ruby {
                            cur.ruby.push((start, cur.text.len(), rt.clone()));
                        }
                        if vertical && sty.combine != TextCombine::None {
                            for (a, b) in vertical::combine_ranges(&part.text, sty.combine) {
                                cur.combine.push((start + a, start + b));
                            }
                        }
                        cur.parts.push(part);
                        if is_link {
                            cur.links
                                .push((start, cur.text.len(), link.clone().unwrap()));
                        }
                    }
                }
                Span::Break => {
                    cur.parts.push(Part::plain("\n"));
                    cur.text.push('\n');
                }
                Span::Anchor(a) => anchors.push(a.clone()),
                Span::Image {
                    path, float, alt, ..
                } => {
                    if !cur.text.trim().is_empty() {
                        pieces.push(Ok(std::mem::replace(&mut cur, Pending::new())));
                    } else {
                        cur = Pending::new();
                    }
                    pieces.push(Err((path.clone(), *float, alt.clone())));
                }
            }
        }
        if !cur.text.trim().is_empty() || (pieces.is_empty() && block.kind == BlockKind::Rule) {
            pieces.push(Ok(cur));
        }
        if block.kind == BlockKind::Rule {
            if let Some(z) = zone.take() {
                y = y.max(z.bottom);
            }
            let top = em * 0.8;
            let gap = if first { 0.0 } else { prev_bottom.max(top) };
            let item_y = y + gap;
            s.items.push(Item {
                kind: Kind::Rule,
                y: item_y,
                h: 2.0,
                x: 0.0,
                anchors: std::mem::take(&mut anchors),
            });
            let idx = s.items.len() - 1;
            s.lines.push(LineBox {
                item: idx,
                line: 0,
                y: item_y,
                h: 2.0,
                lx: 0.0,
                ly: 0.0,
                base: 0.0,
                start: 0,
                end: 0,
            });
            y = item_y + 2.0;
            prev_bottom = em * 0.8;
            first = false;
            continue;
        }
        if pieces.is_empty() {
            // anchors only: attach to the next position
            if !anchors.is_empty() {
                s.items.push(Item {
                    kind: Kind::Rule,
                    y,
                    h: 0.0,
                    x: 0.0,
                    anchors,
                });
            }
            continue;
        }

        let (scale, bold, italic, mono, top_em, bot_em) = match block.kind {
            BlockKind::Heading(n) => {
                let sc = [2.0, 1.6, 1.32, 1.15, 1.05, 1.0][(n.clamp(1, 6) - 1) as usize];
                (sc, true, false, false, 1.1 * sc, 0.5)
            }
            BlockKind::Quote => (1.0, false, true, false, 0.2, 0.7),
            BlockKind::Code => (0.9, false, false, true, 0.3, 0.8),
            _ => {
                if block.marker.is_some() {
                    (1.0, false, false, false, 0.0, 0.3)
                } else {
                    (1.0, false, false, false, 0.0, 0.65)
                }
            }
        };

        // the book's own spacing, where its CSS says something
        let top_em = block.css.margin_top.map_or(top_em, f64::from);
        let bot_em = block.css.margin_bottom.map_or(bot_em, f64::from);
        let bot_em = match style.paragraph_spacing {
            Some(v)
                if block.marker.is_none()
                    && matches!(block.kind, BlockKind::Paragraph | BlockKind::Quote) =>
            {
                v
            }
            _ => bot_em,
        };
        let indent_px = block.indent as f64 * 1.5 * em + block.css.margin_left as f64 * em
            - if block.marker.is_some() {
                1.5 * em
            } else {
                0.0
            };
        let indent_px = indent_px.max(0.0)
            + if block.marker.is_some() {
                0.5 * em
            } else {
                0.0
            };

        for piece in pieces {
            match piece {
                Err((path, side, alt)) => {
                    let Some(pix) = load_pixbuf(s, &path) else {
                        continue;
                    };
                    let (iw, ih) = (pix.width() as f64, pix.height() as f64);
                    if side != FloatSide::None && !vertical {
                        // floated: the text flows around it
                        if let Some(z) = zone.take() {
                            y = y.max(z.bottom);
                        }
                        let k = (col_w * 0.45 / iw).min(max_img_h * 0.7 / ih).min(1.0);
                        let (tw, th) = ((iw * k).max(1.0), (ih * k).max(1.0));
                        let scaled = pix
                            .scale_simple(
                                tw as i32,
                                th as i32,
                                gtk::gdk_pixbuf::InterpType::Bilinear,
                            )
                            .unwrap_or(pix);
                        let gap = if first {
                            0.0
                        } else {
                            prev_bottom.max(em * 0.4)
                        };
                        let fy = y + gap;
                        s.floats.push(FloatBox {
                            pix: scaled,
                            src: path.clone(),
                            alt: alt.clone(),
                            x: if side == FloatSide::Left {
                                0.0
                            } else {
                                col_w - tw
                            },
                            y: fy,
                            h: th,
                        });
                        if !anchors.is_empty() {
                            s.items.push(Item {
                                kind: Kind::Rule,
                                y: fy,
                                h: 0.0,
                                x: 0.0,
                                anchors: std::mem::take(&mut anchors),
                            });
                        }
                        zone = Some(Zone {
                            side,
                            w: tw + 0.9 * em,
                            bottom: fy + th + 0.5 * em,
                        });
                        continue;
                    }
                    if let Some(z) = zone.take() {
                        y = y.max(z.bottom);
                    }
                    let max_w = col_w;
                    // in a vertical line the picture's height runs along the line and its width
                    // across the flow
                    let (along, across) = if vertical { (ih, iw) } else { (iw, ih) };
                    let k = (max_w / along).min(max_img_h / across).min(if style.fixed {
                        6.0
                    } else {
                        1.6
                    });
                    let (pw, ph) = ((iw * k).max(1.0), (ih * k).max(1.0));
                    let scaled = pix
                        .scale_simple(pw as i32, ph as i32, gtk::gdk_pixbuf::InterpType::Bilinear)
                        .unwrap_or(pix);
                    let (tw, th) = if vertical { (ph, pw) } else { (pw, ph) };
                    let top = em * 0.4;
                    let gap = if first { 0.0 } else { prev_bottom.max(top) };
                    let item_y = y + gap;
                    let x = match block.align {
                        Align::Start | Align::Center => (col_w - tw) / 2.0,
                        Align::End => col_w - tw,
                    };
                    s.items.push(Item {
                        kind: Kind::Image {
                            pix: scaled,
                            src: path.clone(),
                            alt: alt.clone(),
                        },
                        y: item_y,
                        h: th,
                        x,
                        anchors: std::mem::take(&mut anchors),
                    });
                    let idx = s.items.len() - 1;
                    s.lines.push(LineBox {
                        item: idx,
                        line: 0,
                        y: item_y,
                        h: th,
                        lx: 0.0,
                        ly: 0.0,
                        base: 0.0,
                        start: 0,
                        end: 0,
                    });
                    y = item_y + th;
                    prev_bottom = em * 0.6;
                    first = false;
                }
                Ok(mut p) => {
                    let mut fd = pango::FontDescription::new();
                    fd.set_family(if mono {
                        "monospace"
                    } else {
                        &style.font_family
                    });
                    fd.set_size((style.font_size * scale * SCALE) as i32);
                    if let Some(w) = style.weight {
                        fd.set_weight(weight_of(w));
                    }
                    if bold {
                        fd.set_weight(pango::Weight::Bold);
                    }
                    if italic {
                        fd.set_style(pango::Style::Italic);
                    }
                    let item_x = indent_px;
                    let width = (col_w - item_x).max(60.0);
                    let wrappable = block.marker.is_none()
                        && block.align == Align::Start
                        && matches!(block.kind, BlockKind::Paragraph | BlockKind::Quote);
                    if !wrappable {
                        if let Some(z) = zone.take() {
                            y = y.max(z.bottom);
                        }
                    }
                    let first_indent = if wrappable && block.css.text_indent > 0.0 {
                        block.css.text_indent as f64 * em
                    } else {
                        0.0
                    };
                    let justify = style.justify && wrappable;
                    let has_ruby = !p.ruby.is_empty();
                    let has_deco = has_ruby || !p.combine.is_empty();
                    // a vertical line needs more room between columns, and ruby more still
                    let spacing = {
                        let mut v = if matches!(block.kind, BlockKind::Heading(_)) {
                            1.0
                        } else {
                            style.line_spacing as f32
                        };
                        if vertical {
                            v = v.max(if has_ruby { 1.9 } else { 1.5 });
                        } else if has_ruby {
                            v = v.max(1.7);
                        }
                        v
                    };
                    let decor = (p.text.len(), p.combine.clone());
                    let em_px = style.font_size * scale * 96.0 / 72.0;
                    let make = |parts: &[Part],
                                wrap_width: f64,
                                justify: bool,
                                indent: f64,
                                last_line: bool| {
                        let layout = pango::Layout::new(ctx);
                        layout.set_font_description(Some(&fd));
                        layout.set_width(if wrap_width < 0.0 {
                            -1
                        } else {
                            (wrap_width * SCALE) as i32
                        });
                        layout.set_wrap(pango::WrapMode::WordChar);
                        layout.set_line_spacing(spacing);
                        layout.set_alignment(match block.align {
                            Align::Center => pango::Alignment::Center,
                            Align::End => pango::Alignment::Right,
                            Align::Start => pango::Alignment::Left,
                        });
                        if justify {
                            layout.set_justify(true);
                            layout.set_justify_last_line(last_line);
                        }
                        if indent > 0.0 {
                            // indent of the first line, as in print
                            layout.set_indent((indent * SCALE) as i32);
                        }
                        if block.marker.is_some() {
                            let mut tabs = pango::TabArray::new(1, true);
                            tabs.set_tab(0, pango::TabAlign::Left, (1.4 * em) as i32);
                            layout.set_tabs(Some(&tabs));
                            layout.set_indent(-((1.4 * em * SCALE) as i32));
                        }
                        layout.set_markup(&wrap_text_markup(
                            markup_of(parts),
                            style.kerning,
                            style.word_spacing,
                            em_px,
                        ));
                        let whole = layout.text().len() == decor.0;
                        decorate(&layout, if whole { &decor.1 } else { &[] }, em_px);
                        layout
                    };

                    let top = em * top_em;
                    let gap = if first { 0.0 } else { prev_bottom.max(top) };
                    let item_y = y + gap;

                    // a drop cap: the first letter is drawn big beside the first lines
                    let mut cap_letter = None;
                    if wrappable && block.css.drop_cap >= 2 && !vertical && !has_deco {
                        cap_letter = take_drop_cap(&mut p.parts);
                    }
                    let line_h = {
                        let trial = make(&p.parts, width, false, 0.0, false);
                        let (a, b) = trial.iter().line_yrange();
                        px(b - a).max(4.0)
                    };
                    let mut wrap: Option<(usize, f64, FloatSide)> = None;
                    let mut cap = None;
                    if let Some((letter, cap_part)) = cap_letter {
                        let n = block.css.drop_cap as usize;
                        let cap_px = ((n - 1) as f64 * line_h + 0.7 * em) / 0.7;
                        let cl = pango::Layout::new(ctx);
                        let mut cfd = fd.clone();
                        cfd.set_size((cap_px * 0.75 * SCALE) as i32);
                        cl.set_font_description(Some(&cfd));
                        cl.set_markup(&format!(
                            "{}{}{}",
                            cap_part.open,
                            glib::markup_escape_text(&letter),
                            cap_part.close
                        ));
                        let capw = px(cl.size().0) + 0.3 * em;
                        wrap = Some((n, capw, FloatSide::Left));
                        cap = Some(cl);
                    } else if wrappable && !has_deco {
                        if let Some(z) = zone {
                            if item_y + 0.5 * line_h >= z.bottom {
                                zone = None;
                            } else {
                                let n = (((z.bottom - item_y) / line_h).ceil() as usize).max(1);
                                wrap = Some((n, z.w, z.side));
                            }
                        }
                    }
                    let rtl = !vertical && pango::find_base_dir(&p.text) == pango::Direction::Rtl;
                    let kp = style.optimal_breaks
                        && justify
                        && !rtl
                        && !vertical
                        && !has_deco
                        && block.marker.is_none()
                        && !matches!(block.kind, BlockKind::Code);
                    let segs = segments(
                        &make,
                        &p,
                        &fd,
                        ctx,
                        &Shape {
                            width,
                            first_indent,
                            justify,
                            kp,
                            head: wrap.map(|(n, w, _)| (n, w)),
                        },
                    );
                    let head_dx = match wrap {
                        Some((_, w, FloatSide::Left)) => w,
                        _ => 0.0,
                    };

                    let mut seg_y = item_y;
                    let n_segs = segs.len();
                    for (si, seg) in segs.into_iter().enumerate() {
                        let layout = seg.layout;
                        let text = layout.text().to_string();
                        let (_, lh) = layout.pixel_size();
                        // a piece that another follows has no spacing after its last line
                        let lh = if si + 1 < n_segs {
                            lh.max((layout.line_count() as f64 * line_h).round() as i32)
                        } else {
                            lh
                        };
                        let char_len = paragraph::canonical_chars(&text).count() as i32;
                        let idx = s.items.len();
                        let seg_x = item_x + if si == 0 { head_dx } else { 0.0 };
                        let item_y = seg_y;

                        // collect lines
                        let mut it = layout.iter();
                        let mut li = 0;
                        let mut cap_base = None;
                        loop {
                            let (y0, y1) = it.line_yrange();
                            let (_, logical) = it.line_extents();
                            let (start, len) = it
                                .line_readonly()
                                .map(|l| (l.start_index(), l.length()))
                                .unwrap_or((0, 0));
                            s.lines.push(LineBox {
                                item: idx,
                                line: li,
                                y: item_y + px(y0),
                                h: px(y1 - y0),
                                lx: px(logical.x()),
                                ly: px(y0),
                                base: px(it.baseline()),
                                start,
                                end: start + len,
                            });
                            if si == 0
                                && cap.is_some()
                                && li as usize + 1 == block.css.drop_cap as usize
                            {
                                cap_base = Some(px(it.baseline()));
                            }
                            li += 1;
                            if !it.next_line() {
                                break;
                            }
                        }
                        let drop_cap = if si == 0 {
                            cap.take().map(|cl| {
                                let base = cap_base.unwrap_or(lh as f64);
                                DropCap {
                                    x: -head_dx,
                                    y: base - px(cl.baseline()),
                                    layout: cl,
                                }
                            })
                        } else {
                            None
                        };
                        s.items.push(Item {
                            kind: Kind::Text(TextItem {
                                layout,
                                text,
                                links: seg.links,
                                char_base,
                                char_len,
                                x: seg_x,
                                bar: block.kind == BlockKind::Quote,
                                bg: block.kind == BlockKind::Code,
                                cont: si > 0,
                                cap: drop_cap,
                                ruby: if n_segs == 1 {
                                    p.ruby.clone()
                                } else {
                                    Vec::new()
                                },
                                combine: if n_segs == 1 {
                                    p.combine.clone()
                                } else {
                                    Vec::new()
                                },
                                plain: (!p.combine.is_empty()).then(|| ctxs.plain.clone()),
                                sideways: vertical && doc.orientation == TextOrientation::Sideways,
                            }),
                            y: item_y,
                            h: lh as f64,
                            x: seg_x,
                            anchors: if si == 0 {
                                std::mem::take(&mut anchors)
                            } else {
                                Vec::new()
                            },
                        });
                        // the pieces of a wrapped paragraph are one paragraph for the offsets
                        char_base += char_len + i32::from(si + 1 == n_segs);
                        seg_y = item_y + lh as f64;
                    }
                    y = seg_y;
                    prev_bottom = em * bot_em;
                    first = false;
                }
            }
        }
        if !anchors.is_empty() {
            if let Some(last) = s.items.last_mut() {
                last.anchors.extend(anchors);
            }
        }
    }
    if let Some(z) = zone {
        y = y.max(z.bottom);
    }
    if style.fixed {
        // a page of a fixed-layout book sits in the middle of the screen
        let off = ((content_height(s) - y) / 2.0).max(0.0);
        if off > 0.0 {
            for it in &mut s.items {
                it.y += off;
            }
            for l in &mut s.lines {
                l.y += off;
            }
            for f in &mut s.floats {
                f.y += off;
            }
            y += off;
        }
    }
    s.flow_h = y;
    // Zero-height anchor-only items need a y that exists; fine.
}

/// One laid out piece of a paragraph. A paragraph that wraps around a drop cap or a float is laid
/// out in two: the first lines, which are narrower, and the rest.
struct Seg {
    layout: pango::Layout,
    links: Vec<(usize, usize, String)>,
}

/// How a paragraph is to be shaped.
struct Shape {
    width: f64,
    first_indent: f64,
    justify: bool,
    /// Knuth-Plass breaking is wanted.
    kp: bool,
    /// `(lines, space taken)`: the first lines are narrower by that much.
    head: Option<(usize, f64)>,
}

type Make<'a> = dyn Fn(&[Part], f64, bool, f64, bool) -> pango::Layout + 'a;

/// Splits the text of `parts` at byte `at` (a character boundary).
fn split_parts(parts: &[Part], at: usize) -> (Vec<Part>, Vec<Part>) {
    let (mut head, mut tail) = (Vec::new(), Vec::new());
    let mut pos = 0;
    for q in parts {
        let end = pos + q.text.len();
        let copy = |text: &str| Part {
            open: q.open.clone(),
            text: text.to_string(),
            close: q.close.clone(),
        };
        if end <= at {
            head.push(copy(&q.text));
        } else if pos >= at {
            tail.push(copy(&q.text));
        } else {
            let k = at - pos;
            head.push(copy(&q.text[..k]));
            tail.push(copy(&q.text[k..]));
        }
        pos = end;
    }
    (head, tail)
}

type Links = Vec<(usize, usize, String)>;

fn split_links(links: &[(usize, usize, String)], at: usize) -> (Links, Links) {
    let head = links
        .iter()
        .filter(|(a, _, _)| *a < at)
        .map(|(a, b, h)| (*a, (*b).min(at), h.clone()))
        .collect();
    let tail = links
        .iter()
        .filter(|(_, b, _)| *b > at)
        .map(|(a, b, h)| (a.max(&at) - at, b - at, h.clone()))
        .collect();
    (head, tail)
}

/// Stretches the spaces of a line with letter spacing: `(start, end, extra width)` in bytes.
fn stretch(layout: &pango::Layout, gaps: &[(usize, usize, f64)]) {
    if gaps.is_empty() {
        return;
    }
    let list = layout.attributes().unwrap_or_default();
    for (a, b, extra) in gaps {
        let per_char = extra / (b - a).max(1) as f64;
        let mut at: pango::Attribute =
            pango::AttrInt::new_letter_spacing((per_char * SCALE) as i32).into();
        at.set_start_index(*a as u32);
        at.set_end_index(*b as u32);
        list.insert(at);
    }
    layout.set_attributes(Some(&list));
}

/// Takes the first letter of the paragraph out for a drop cap. The letter stays in the text
/// (offsets of highlights do not move) but shrinks to an invisible dot; the returned letter and
/// its tags are for drawing the big initial.
fn take_drop_cap(parts: &mut Vec<Part>) -> Option<(String, Part)> {
    let i = parts.iter().position(|q| !q.text.is_empty())?;
    let c = parts[i]
        .text
        .chars()
        .next()
        .filter(|c| c.is_alphanumeric())?;
    let rest = parts[i].text[c.len_utf8()..].to_string();
    let q = &parts[i];
    let big = Part {
        open: q.open.clone(),
        text: c.to_string(),
        close: q.close.clone(),
    };
    let dot = Part {
        open: format!("{}<span size=\"1024\" fgalpha=\"1\">", q.open),
        text: c.to_string(),
        close: format!("</span>{}", q.close),
    };
    let tail = Part {
        open: q.open.clone(),
        text: rest,
        close: q.close.clone(),
    };
    parts.splice(i..=i, [dot, tail]);
    Some((c.to_string(), big))
}

/// Lays the paragraph out: Knuth-Plass when wanted and possible, Pango's own breaking otherwise.
fn segments(
    make: &Make,
    p: &Pending,
    fd: &pango::FontDescription,
    ctx: &pango::Context,
    shape: &Shape,
) -> Vec<Seg> {
    if shape.kp {
        if let Some(v) = optimal_segments(make, p, fd, ctx, shape) {
            return v;
        }
    }
    let Some((n, taken)) = shape.head else {
        return vec![Seg {
            layout: make(
                &p.parts,
                shape.width,
                shape.justify,
                shape.first_indent,
                false,
            ),
            links: p.links.clone(),
        }];
    };
    let head_w = (shape.width - taken).max(60.0);
    let trial = make(&p.parts, head_w, shape.justify, shape.first_indent, false);
    let split = match trial.line_readonly(n as i32) {
        Some(l) => l.start_index() as usize,
        None => {
            return vec![Seg {
                layout: trial,
                links: p.links.clone(),
            }]
        }
    };
    let (hp, tp) = split_parts(&p.parts, split);
    let (hl, tl) = split_links(&p.links, split);
    vec![
        Seg {
            layout: make(&hp, head_w, shape.justify, shape.first_indent, true),
            links: hl,
        },
        Seg {
            layout: make(&tp, shape.width, shape.justify, 0.0, false),
            links: tl,
        },
    ]
}

/// Knuth-Plass: the text is measured on one line, the breaks are planned by
/// [`paragraph::plan`], forced into the text, and the spaces of every line are stretched with
/// letter spacing. `None` (keep Pango's breaking) when the plan does not come out the way it was
/// planned.
fn optimal_segments(
    make: &Make,
    p: &Pending,
    fd: &pango::FontDescription,
    ctx: &pango::Context,
    shape: &Shape,
) -> Option<Vec<Seg>> {
    let probe = make(&p.parts, -1.0, false, 0.0, false);
    if probe.text() != p.text {
        return None;
    }
    let hy = pango::Layout::new(ctx);
    hy.set_font_description(Some(fd));
    hy.set_text(&paragraph::GHOST_HYPHEN.to_string());
    let table = (!p.text.contains('\n') && !super::canvas_xtable::has_rtl(&p.text))
        .then(|| cluster_table(&probe, p.text.len()));
    let x = |b: usize| {
        table
            .as_ref()
            .and_then(|t| t.get(b))
            .unwrap_or_else(|| probe.index_to_pos(b as i32).x() as f64 / SCALE)
    };
    let head_w = shape
        .head
        .map_or(shape.width, |(_, t)| (shape.width - t).max(60.0));
    let plan = paragraph::plan(&paragraph::Request {
        text: &p.text,
        x: &x,
        hyphen_width: hy.size().0 as f64 / SCALE,
        first_width: head_w - shape.first_indent,
        width: shape.width,
        hyphen_penalty: 50.0,
        head: shape.head.map(|(n, _)| (n, head_w)),
    })?;
    if plan.lines < 2 {
        return None;
    }
    let mut texts: Vec<String> = p.parts.iter().map(|q| q.text.clone()).collect();
    paragraph::apply_edits(&mut texts, &plan.edits);
    let parts: Vec<Part> = p
        .parts
        .iter()
        .zip(texts)
        .map(|(q, text)| Part {
            open: q.open.clone(),
            text,
            close: q.close.clone(),
        })
        .collect();
    let links: Links = p
        .links
        .iter()
        .map(|(a, b, h)| (plan.map(*a), plan.map(*b), h.clone()))
        .collect();
    let gaps: Vec<(usize, usize, f64)> = plan
        .gaps
        .iter()
        .map(|g| (plan.map(g.start), plan.map(g.end), g.extra))
        .collect();
    // a little room, so that a line stretched to exactly the width does not wrap on rounding
    let room = 3.0;
    let split = shape
        .head
        .filter(|(n, _)| plan.lines > *n)
        .map(|(n, _)| (n, plan.ends[n - 1]));
    let Some((n, at)) = split else {
        let layout = make(&parts, head_w + room, false, shape.first_indent, false);
        stretch(&layout, &gaps);
        if layout.line_count() as usize != plan.lines {
            return None;
        }
        return Some(vec![Seg { layout, links }]);
    };
    let (hp, tp) = split_parts(&parts, at);
    let (hl, tl) = split_links(&links, at);
    let hg: Vec<_> = gaps.iter().filter(|g| g.1 <= at).copied().collect();
    let tg: Vec<_> = gaps
        .iter()
        .filter(|g| g.0 >= at)
        .map(|g| (g.0 - at, g.1 - at, g.2))
        .collect();
    let head = make(&hp, head_w + room, false, shape.first_indent, false);
    let tail = make(&tp, shape.width + room, false, 0.0, false);
    stretch(&head, &hg);
    stretch(&tail, &tg);
    if head.line_count() as usize != n || tail.line_count() as usize != plan.lines - n {
        return None;
    }
    Some(vec![
        Seg {
            layout: head,
            links: hl,
        },
        Seg {
            layout: tail,
            links: tl,
        },
    ])
}

/// The x of every cluster start of a one-line layout in one pass (see `canvas_xtable`).
fn cluster_table(layout: &pango::Layout, len: usize) -> super::canvas_xtable::XTable {
    let mut at = Vec::with_capacity(len / 2 + 2);
    let mut it = layout.iter();
    let mut end = None;
    loop {
        let (_, logical) = it.cluster_extents();
        let b = it.index();
        if b >= 0 {
            at.push((b as usize, logical.x() as f64 / SCALE));
            end = Some(((logical.x() + logical.width()) as f64 / SCALE, b));
        }
        if !it.next_cluster() {
            break;
        }
    }
    if let Some((x, _)) = end {
        at.push((len, x));
    }
    super::canvas_xtable::XTable::new(at)
}

/// The links that are on the visible page: where each one starts and where it leads.
fn visible_links(s: &State) -> Vec<(f64, f64, String)> {
    let mut out = Vec::new();
    for it in &s.items {
        let Kind::Text(t) = &it.kind else { continue };
        for (a, _, href) in &t.links {
            if out.len() >= 676 {
                return out;
            }
            let lc = byte_to_char(&t.text, *a);
            let c = t.char_base + canon_of_layout_char(&t.text, lc) as i32;
            if let Some(r) = visible_range_rects(s, c, c + 1).first() {
                out.push((r.0, r.1, href.clone()));
            }
        }
    }
    out
}

/// The letters of the link hints, drawn over the page.
fn draw_hints(s: &State, cr: &gtk::cairo::Context) {
    if s.hints.is_empty() {
        return;
    }
    let layout = pangocairo::functions::create_layout(cr);
    let mut fd = pango::FontDescription::new();
    fd.set_family("monospace");
    fd.set_weight(pango::Weight::Bold);
    fd.set_size((10.0 * SCALE) as i32);
    layout.set_font_description(Some(&fd));
    for h in &s.hints {
        if !h.label.starts_with(&s.hint_typed) {
            continue;
        }
        layout.set_text(&h.label.to_uppercase());
        let (tw, th) = layout.pixel_size();
        let (w, hh) = (tw as f64 + 8.0, th as f64 + 2.0);
        let (x, y) = (h.x, (h.y - hh * 0.35).max(0.0));
        set_rgb(cr, s.style.palette.accent, 1.0);
        cr.rectangle(x, y, w, hh);
        let _ = cr.fill();
        set_rgb(cr, s.style.palette.bg, 1.0);
        cr.move_to(x + 4.0, y + 1.0);
        pangocairo::functions::show_layout(cr, &layout);
    }
}

/// Paints the page that was left over the new one: it slides away (slide) or fades out (fade).
fn draw_turn_animation(a: &TurnAnim, cr: &gtk::cairo::Context, w: f64) {
    let p = bookx_text::aids::ease_out(
        a.t0.elapsed().as_millis() as u64,
        bookx_text::aids::PAGE_ANIMATION_MS,
    );
    if p >= 1.0 {
        return;
    }
    let _ = cr.save();
    match a.kind {
        bookx_text::aids::PageAnimation::Slide => {
            let dx = if a.forward { -p * w } else { p * w };
            cr.set_source_surface(&a.old, dx, 0.0).ok();
            let _ = cr.paint();
        }
        _ => {
            cr.set_source_surface(&a.old, 0.0, 0.0).ok();
            let _ = cr.paint_with_alpha(1.0 - p);
        }
    }
    let _ = cr.restore();
}

fn paginate(s: &mut State) {
    s.pages.clear();
    if s.style.layout == LayoutMode::Scroll {
        return;
    }
    let flow: Vec<FlowLine> = s
        .lines
        .iter()
        .map(|l| FlowLine {
            item: l.item,
            line: l.line,
            v: l.y,
            thick: l.h,
        })
        .collect();
    s.pages = vertical::paginate_with(
        &flow,
        block_extent(s),
        s.style.widows_orphans,
        s.style.widows_orphans,
    )
    .into_iter()
    .map(|(y0, y1)| Page { y0, y1 })
    .collect();
}

// ===========================================================================
// Drawing
// ===========================================================================

/// Asks the font for the half-width forms of punctuation in a row (`halt`, `vhal`), forbids the
/// breaks that strict kinsoku forbids and Unicode allows, and reserves one em for each run of digits
/// that is set upright side by side in a vertical line.
fn decorate(layout: &pango::Layout, combine: &[(usize, usize)], em_px: f64) {
    let text = layout.text();
    let squeeze = vertical::squeezed(&text);
    let kana: Vec<(usize, char)> = text
        .char_indices()
        .filter(|(_, c)| vertical::strict_no_break_before(*c))
        .collect();
    if squeeze.is_empty() && combine.is_empty() && kana.is_empty() {
        return;
    }
    let list = layout.attributes().unwrap_or_default();
    // strict kinsoku: no line starts with a small kana or the long vowel mark, so the break
    // between such a character and the one before it is forbidden
    for (b, c) in kana {
        let Some(prev) = text[..b].chars().next_back() else {
            continue;
        };
        let mut at: pango::Attribute = pango::AttrInt::new_allow_breaks(false).into();
        at.set_start_index((b - prev.len_utf8()) as u32);
        at.set_end_index((b + c.len_utf8()) as u32);
        list.insert(at);
    }
    if !squeeze.is_empty() {
        let offsets: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
        for i in squeeze {
            let a = offsets[i];
            let b = offsets.get(i + 1).copied().unwrap_or(text.len());
            let mut at: pango::Attribute = pango::AttrFontFeatures::new("halt=1,vhal=1").into();
            at.set_start_index(a as u32);
            at.set_end_index(b as u32);
            list.insert(at);
        }
    }
    for &(a, b) in combine {
        let n = text[a..b].chars().count().max(1);
        let em = (em_px * SCALE) as i32;
        let rect = pango::Rectangle::new(0, -em / 2, em / n as i32, em);
        for (off, c) in text[a..b].char_indices() {
            let mut at: pango::Attribute = pango::AttrShape::new(&rect, &rect).into();
            at.set_start_index((a + off) as u32);
            at.set_end_index((a + off + c.len_utf8()) as u32);
            list.insert(at);
        }
    }
    layout.set_attributes(Some(&list));
}

/// Puts the origin of the paragraph's layout where line `l` of it belongs on the screen. Text
/// is laid out upright; in a vertical page the layout is turned a quarter, so that its lines run
/// down and its next line is to the left of the one before (`vertical-rl`) or, when the flow
/// runs left to right (`vertical-lr`), each line is placed by itself.
fn line_frame(
    cr: &gtk::cairo::Context,
    fr: &Frame,
    item: &Item,
    t: &TextItem,
    l: &LineBox,
    y0: f64,
) {
    match fr.mode {
        WritingMode::HorizontalTb => cr.translate(fr.x + t.x, fr.y + (item.y - y0)),
        WritingMode::VerticalRl => {
            cr.translate(fr.x + fr.w - (item.y - y0), fr.y);
            cr.rotate(std::f64::consts::FRAC_PI_2);
            cr.translate(t.x, 0.0);
        }
        WritingMode::VerticalLr => {
            cr.translate(fr.x + (item.y - y0) + 2.0 * l.ly + l.h, fr.y);
            cr.rotate(std::f64::consts::FRAC_PI_2);
            cr.translate(t.x, 0.0);
        }
    }
}

/// Annotations above (horizontal) or beside (vertical) their base text, and the digits that are
/// set upright in one em. The origin is that of the paragraph's layout, see [`line_frame`].
fn draw_decorations(cr: &gtk::cairo::Context, t: &TextItem, l: &LineBox, vertical: bool) {
    let Some(line) = t.layout.line_readonly(l.line) else {
        return;
    };
    let Some(fd) = t.layout.font_description() else {
        return;
    };
    let em = fd.size() as f64 / SCALE * 96.0 / 72.0;
    let (ls, le) = (l.start.max(0) as usize, l.end.max(0) as usize);
    for (a, b, note) in &t.ruby {
        let (sb, eb) = ((*a).max(ls), (*b).min(le));
        if eb <= sb || !t.text.is_char_boundary(sb) || !t.text.is_char_boundary(eb) {
            continue;
        }
        // a base that wraps takes its annotation along in proportion
        let total = t.text[*a..*b].chars().count().max(1);
        let before = t.text[*a..sb].chars().count();
        let here = t.text[sb..eb].chars().count();
        let notes: Vec<char> = note.chars().collect();
        let (n0, n1) = (
            (notes.len() * before + total / 2) / total,
            (notes.len() * (before + here) + total / 2) / total,
        );
        let part: String = notes[n0..n1.min(notes.len())].iter().collect();
        if part.is_empty() {
            continue;
        }
        let x0 = l.lx + px(line.index_to_x(sb as i32, false));
        let x1 = l.lx + px(line.index_to_x(eb as i32, false));
        let (x0, x1) = (x0.min(x1), x0.max(x1));
        let rl = pango::Layout::new(&t.layout.context());
        let mut rfd = fd.clone();
        rfd.set_size((fd.size() as f64 * RUBY_SCALE) as i32);
        rl.set_font_description(Some(&rfd));
        rl.set_text(&part);
        let rw = px(rl.size().0);
        let rbase = px(rl.baseline());
        // the line the annotation sits on, measured from the base's own baseline
        let y = if vertical && !t.sideways {
            // glyphs of a vertical line are centred on the baseline: the annotation goes beside
            // them, to the side the lines are read from
            l.base - em / 2.0 - em * RUBY_SCALE / 2.0 - 0.04 * em - rbase
        } else {
            l.base - 0.9 * em - 0.06 * em - rbase
        };
        cr.move_to((x0 + x1) / 2.0 - rw / 2.0, y);
        pangocairo::functions::show_layout(cr, &rl);
    }
    if vertical {
        for &(a, b) in &t.combine {
            if a < ls || b > le || !t.text.is_char_boundary(a) || !t.text.is_char_boundary(b) {
                continue;
            }
            let Some(ctx) = &t.plain else { continue };
            let x0 = l.lx + px(line.index_to_x(a as i32, false));
            let x1 = l.lx + px(line.index_to_x(b as i32, false));
            let dl = pango::Layout::new(ctx);
            dl.set_font_description(Some(&fd));
            dl.set_text(&t.text[a..b]);
            let w = px(dl.size().0);
            if w > em {
                let mut small = fd.clone();
                small.set_size((fd.size() as f64 * em / w) as i32);
                dl.set_font_description(Some(&small));
            }
            let (dw, dh) = (px(dl.size().0), px(dl.size().1));
            let _ = cr.save();
            // (the digits of sideways text sit on the usual baseline, a third of an em above it)
            let mid = if t.sideways {
                l.base - 0.34 * em
            } else {
                l.base
            };
            cr.translate((x0 + x1) / 2.0, mid);
            cr.rotate(-std::f64::consts::FRAC_PI_2);
            cr.move_to(-dw / 2.0, -dh / 2.0);
            pangocairo::functions::show_layout(cr, &dl);
            let _ = cr.restore();
        }
    }
}

fn set_rgb(cr: &gtk::cairo::Context, c: Rgb, a: f64) {
    cr.set_source_rgba(c.0, c.1, c.2, a);
}

/// The colour of the page: the theme's, or in original mode the one the book's CSS puts on its
/// body (when it has one).
fn page_color(style: &CanvasStyle, doc: Option<&Doc>) -> Rgb {
    match (style.original_colors, doc.and_then(|d| d.background)) {
        (true, Some((r, g, b))) => (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0),
        _ => style.palette.bg,
    }
}

fn draw(s: &State, cr: &gtk::cairo::Context, w: f64, h: f64) {
    let p = &s.style.palette;
    let page = page_color(&s.style, s.doc.as_deref());
    set_rgb(cr, page, 1.0);
    let _ = cr.paint();
    if s.items.is_empty() {
        return;
    }

    // Highlights, search hits and the selection are painted *behind* the text (like <mark>),
    // so glyphs keep their full colour. Dark pages use a gentler tint to keep text readable.
    let dark = (page.0 + page.1 + page.2) / 3.0 < 0.5;
    let hl_alpha = if dark { 0.42 } else { 0.6 };
    let _ = cr.save();
    for hl in &s.highlights {
        for r in visible_range_rects(s, hl.start, hl.end) {
            match hl.style {
                HighlightStyle::Background => {
                    set_rgb(cr, hl.color, hl_alpha);
                    cr.rectangle(r.0, r.1, r.2, r.3);
                    let _ = cr.fill();
                }
                HighlightStyle::Invert => {
                    set_rgb(cr, hl.color, 0.92);
                    cr.rectangle(r.0, r.1, r.2, r.3);
                    let _ = cr.fill();
                }
                // drawn on top of the text, below
                _ => {}
            }
            if !hl.note.is_empty() {
                set_rgb(cr, hl.color, 1.0);
                if is_vertical(s) {
                    cr.rectangle(r.0 + r.2 - 2.0, r.1, 2.0, r.3);
                } else {
                    cr.rectangle(r.0, r.1 + r.3 - 2.0, r.2, 2.0);
                }
                let _ = cr.fill();
            }
        }
    }
    if let Some((a, b)) = s.speaking {
        for r in visible_range_rects(s, a, b) {
            set_rgb(cr, p.accent, if dark { 0.38 } else { 0.28 });
            cr.rectangle(r.0, r.1, r.2, r.3);
            let _ = cr.fill();
        }
    }
    if let Some(((wa, wb), (fa, fb))) = s.focus_word {
        for r in visible_range_rects(s, wa, wb) {
            set_rgb(cr, p.accent, if dark { 0.38 } else { 0.28 });
            cr.rectangle(r.0, r.1, r.2, r.3);
            let _ = cr.fill();
        }
        for r in visible_range_rects(s, fa, fb) {
            cr.set_source_rgba(0.95, 0.2, 0.1, if dark { 0.7 } else { 0.55 });
            cr.rectangle(r.0, r.1, r.2, r.3);
            let _ = cr.fill();
            cr.set_source_rgba(0.85, 0.1, 0.05, 1.0);
            cr.rectangle(r.0, r.1 + r.3 - 2.5, r.2, 2.5);
            let _ = cr.fill();
        }
    }
    for (i, (a, b)) in s.hits.iter().enumerate() {
        let active = s.active_hit == Some(i);
        for r in visible_range_rects(s, *a, *b) {
            if active {
                cr.set_source_rgba(1.0, 0.55, 0.0, if dark { 0.5 } else { 0.7 });
            } else {
                cr.set_source_rgba(1.0, 0.85, 0.0, if dark { 0.38 } else { 0.55 });
            }
            cr.rectangle(r.0, r.1, r.2, r.3);
            let _ = cr.fill();
        }
    }
    // a selection sitting exactly on a highlight keeps the highlight's true colour
    let sel = s
        .sel
        .filter(|(a, b)| !s.highlights.iter().any(|h| h.start <= *a && h.end >= *b));
    if let Some((a, b)) = sel {
        for r in visible_range_rects(s, a, b) {
            set_rgb(cr, p.accent, if dark { 0.45 } else { 0.32 });
            cr.rectangle(r.0, r.1, r.2, r.3);
            let _ = cr.fill();
        }
    }
    let _ = cr.restore();

    let vertical = is_vertical(s);
    for col in visible_columns(s) {
        let fr = col.frame;
        let (y0, y1) = (col.y0, col.y1);
        let _ = cr.save();
        if vertical {
            cr.rectangle(fr.x - 1.0, 0.0, fr.w + 2.0, h);
        } else {
            cr.rectangle(0.0, fr.y, w, (y1 - y0).min(h - fr.y));
        }
        cr.clip();
        let first_item = s.items.partition_point(|i| i.y + i.h <= y0);
        for item in &s.items[first_item..] {
            if item.y >= y1 {
                break;
            }
            let iy = fr.y + (item.y - y0);
            match &item.kind {
                Kind::Text(_) if vertical => {}
                Kind::Text(t) => {
                    if t.bg {
                        set_rgb(cr, p.fg, 0.07);
                        cr.rectangle(fr.x + t.x - 6.0, iy - 3.0, fr.w - t.x + 12.0, item.h + 6.0);
                        let _ = cr.fill();
                    }
                    if t.bar {
                        set_rgb(cr, p.accent, 0.8);
                        cr.rectangle(fr.x + t.x - 12.0, iy, 3.0, item.h);
                        let _ = cr.fill();
                    }
                    // highlights, hits, selection (behind text)
                    cr.move_to(fr.x + t.x, iy);
                    set_rgb(cr, p.fg, 1.0);
                    pangocairo::functions::show_layout(cr, &t.layout);
                    if let Some(c) = &t.cap {
                        cr.move_to(fr.x + t.x + c.x, iy + c.y);
                        pangocairo::functions::show_layout(cr, &c.layout);
                    }
                }
                Kind::Image { pix, .. } => {
                    // a picture stays upright on a vertical page: it is as wide across the flow
                    // as the page shows it
                    let (iw, bh) = if vertical {
                        (pix.height() as f64, pix.width() as f64)
                    } else {
                        (pix.width() as f64, pix.height() as f64)
                    };
                    let (x, y, _, _) = fr.rect(item.x, item.y, iw, bh);
                    let _ = cr.save();
                    cr.translate(x, y);
                    gtk::gdk::prelude::GdkCairoContextExt::set_source_pixbuf(cr, pix, 0.0, 0.0);
                    let _ = cr.paint();
                    let _ = cr.restore();
                }
                Kind::Rule => {
                    if item.h > 0.0 {
                        let len = fr.line_extent();
                        set_rgb(cr, p.muted, 0.6);
                        let (x, y, rw, rh) = fr.rect(len * 0.3, item.y, len * 0.4, 1.0);
                        cr.rectangle(x, y, rw, rh);
                        let _ = cr.fill();
                    }
                }
            }
        }
        // vertical text is drawn a line at a time, and annotations and combined digits go on top
        let first_line = s.lines.partition_point(|l| l.y + l.h <= y0);
        for l in &s.lines[first_line.min(s.lines.len())..] {
            if l.y >= y1 {
                break;
            }
            let item = &s.items[l.item];
            let Kind::Text(t) = &item.kind else { continue };
            if !vertical && t.ruby.is_empty() {
                continue;
            }
            let _ = cr.save();
            line_frame(cr, &fr, item, t, l, y0);
            set_rgb(cr, p.fg, 1.0);
            if vertical {
                if let Some(line) = t.layout.line_readonly(l.line) {
                    cr.move_to(l.lx, l.base);
                    pangocairo::functions::show_layout_line(cr, &line);
                }
            }
            draw_decorations(cr, t, l, vertical);
            let _ = cr.restore();
        }
        if !vertical {
            for f in &s.floats {
                if f.y >= y1 || f.y + f.h <= y0 {
                    continue;
                }
                let _ = cr.save();
                cr.translate(fr.x + f.x, fr.y + (f.y - y0));
                gtk::gdk::prelude::GdkCairoContextExt::set_source_pixbuf(cr, &f.pix, 0.0, 0.0);
                let _ = cr.paint();
                let _ = cr.restore();
            }
        }
        let _ = cr.restore();
    }

    // underline, strikethrough and squiggly highlights are lines drawn over the glyphs
    for hl in &s.highlights {
        if !matches!(
            hl.style,
            HighlightStyle::Underline | HighlightStyle::Strikethrough | HighlightStyle::Squiggly
        ) {
            continue;
        }
        set_rgb(cr, hl.color, 1.0);
        cr.set_line_width(2.0);
        for r in visible_range_rects(s, hl.start, hl.end) {
            let (x0, y0, w, h) = r;
            match hl.style {
                // a vertical line is underlined on the right, struck through down the middle
                HighlightStyle::Underline if vertical => {
                    cr.rectangle(x0 + w - 3.0, y0, 2.0, h);
                    let _ = cr.fill();
                }
                HighlightStyle::Strikethrough if vertical => {
                    cr.rectangle(x0 + w * 0.48, y0, 2.0, h);
                    let _ = cr.fill();
                }
                HighlightStyle::Underline => {
                    cr.rectangle(x0, y0 + h - 3.0, w, 2.0);
                    let _ = cr.fill();
                }
                HighlightStyle::Strikethrough => {
                    cr.rectangle(x0, y0 + h * 0.52, w, 2.0);
                    let _ = cr.fill();
                }
                _ if vertical => {
                    let base = x0 + w - 3.0;
                    let step = 3.0;
                    cr.move_to(base, y0);
                    let mut y = y0;
                    let mut up = true;
                    while y < y0 + h {
                        y = (y + step).min(y0 + h);
                        cr.line_to(if up { base + 2.0 } else { base - 1.0 }, y);
                        up = !up;
                    }
                    let _ = cr.stroke();
                }
                _ => {
                    let base = y0 + h - 3.0;
                    let step = 3.0;
                    cr.move_to(x0, base);
                    let mut x = x0;
                    let mut up = true;
                    while x < x0 + w {
                        x = (x + step).min(x0 + w);
                        cr.line_to(x, if up { base - 2.0 } else { base + 1.0 });
                        up = !up;
                    }
                    let _ = cr.stroke();
                }
            }
        }
    }

    // terms saved with the user's own definition: a dotted underline in the reading colour
    if !s.vocab_marks.is_empty() {
        let _ = cr.save();
        set_rgb(cr, p.fg, 0.8);
        cr.set_line_width(1.4);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        cr.set_dash(&[0.1, 3.0], 0.0);
        for (a, b) in &s.vocab_marks {
            for (x0, y0, w, h) in visible_range_rects(s, *a, *b) {
                if vertical {
                    cr.move_to(x0 + w - 1.5, y0);
                    cr.line_to(x0 + w - 1.5, y0 + h);
                } else {
                    cr.move_to(x0, y0 + h - 1.5);
                    cr.line_to(x0 + w, y0 + h - 1.5);
                }
                let _ = cr.stroke();
            }
        }
        let _ = cr.restore();
    }

    // page divider for two columns
    if s.style.layout == LayoutMode::TwoColumns {
        let geo = column_geometry(s);
        if geo.len() == 2 {
            let x = (geo[0].0 + geo[0].1 + geo[1].0) / 2.0;
            set_rgb(cr, p.muted, 0.18);
            cr.rectangle(x, V_MARGIN + 12.0, 1.0, h - 2.0 * V_MARGIN - 24.0);
            let _ = cr.fill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_byte_conversion_handles_multibyte() {
        let t = "añb—c";
        assert_eq!(byte_to_char(t, 0), 0);
        assert_eq!(byte_to_char(t, 3), 2);
        assert_eq!(char_to_byte(t, 2), 3);
        assert_eq!(char_to_byte(t, 99), t.len());
        assert_eq!(byte_to_char(t, 2), 1); // inside a multibyte char: rounds down
    }
}

#[cfg(test)]
mod note_tests {
    use super::*;

    fn hl(id: &str, start: i32, end: i32, note: &str) -> Highlight {
        Highlight {
            style: HighlightStyle::Background,
            id: id.into(),
            start,
            end,
            color: (1.0, 1.0, 0.0),
            note: note.into(),
        }
    }

    #[test]
    fn notes_are_found_by_character_offset() {
        let all = [
            hl("a", 10, 20, "first"),
            hl("b", 30, 40, ""),
            hl("c", 50, 60, "third"),
        ];
        assert_eq!(note_at(&all, 10), Some("first"));
        assert_eq!(note_at(&all, 19), Some("first"));
        assert_eq!(note_at(&all, 20), None); // end is exclusive
        assert_eq!(note_at(&all, 35), None); // highlight without a note: no tooltip
        assert_eq!(note_at(&all, 55), Some("third"));
        assert_eq!(note_at(&[], 0), None);
    }
}

#[cfg(test)]
mod hover_tests {
    use super::*;

    #[test]
    fn hover_preview_follows_the_note_under_the_pointer() {
        assert_eq!(hover_action(None, None), HoverAction::Keep);
        assert_eq!(hover_action(None, Some("a")), HoverAction::Show("a".into()));
        assert_eq!(hover_action(Some("a"), Some("a")), HoverAction::Keep);
        assert_eq!(
            hover_action(Some("a"), Some("b")),
            HoverAction::Show("b".into())
        );
        assert_eq!(hover_action(Some("a"), None), HoverAction::Hide);
    }
}

#[cfg(test)]
mod hyphen_tests {
    use super::*;

    const T: &str = "hy\u{ad}phen\u{ad}ated word";

    #[test]
    fn canonical_to_layout_skips_soft_hyphens() {
        // canonical: "hyphenated word"
        assert_eq!(layout_char_of_canon(T, 0), 0);
        assert_eq!(layout_char_of_canon(T, 2), 3); // 'p' sits after the first soft hyphen
        assert_eq!(layout_char_of_canon(T, 6), 8); // 'a'
        assert_eq!(layout_char_of_canon(T, 99), T.chars().count());
        assert_eq!(layout_char_of_canon("plain", 3), 3);
        assert_eq!(layout_char_of_canon("", 0), 0);
    }

    #[test]
    fn layout_to_canonical_counts_visible_chars() {
        assert_eq!(canon_of_layout_char(T, 0), 0);
        assert_eq!(canon_of_layout_char(T, 3), 2);
        assert_eq!(canon_of_layout_char(T, T.chars().count()), 15);
        assert_eq!(canon_of_layout_char(T, 999), 15);
        assert_eq!(canon_of_layout_char("plain", 3), 3);
    }

    #[test]
    fn the_two_mappings_round_trip_for_every_visible_char() {
        let canon: Vec<char> = T.chars().filter(|c| *c != SHY).collect();
        for (c, ch) in canon.iter().enumerate() {
            let l = layout_char_of_canon(T, c);
            assert_eq!(T.chars().nth(l), Some(*ch), "char {c}");
            assert_eq!(canon_of_layout_char(T, l), c, "char {c}");
        }
    }

    #[test]
    fn text_with_only_soft_hyphens_is_empty_canonically() {
        let t = "\u{ad}\u{ad}";
        assert_eq!(canon_of_layout_char(t, 2), 0);
        assert_eq!(layout_char_of_canon(t, 0), 2);
    }
}

#[cfg(test)]
mod colour_tests {
    use super::*;
    use crate::core::doc::Style;

    fn doc_with(bg: Option<(u8, u8, u8)>) -> Doc {
        Doc {
            blocks: vec![],
            background: bg,
            ..Default::default()
        }
    }

    #[test]
    fn the_page_takes_the_books_colour_only_in_original_mode() {
        let mut st = CanvasStyle::default();
        let d = doc_with(Some((255, 0, 0)));
        assert_eq!(page_color(&st, Some(&d)), st.palette.bg);
        st.original_colors = true;
        assert_eq!(page_color(&st, Some(&d)), (1.0, 0.0, 0.0));
        assert_eq!(page_color(&st, Some(&doc_with(None))), st.palette.bg);
        assert_eq!(page_color(&st, None), st.palette.bg);
    }

    #[test]
    fn small_caps_fonts_and_sizes_of_the_book_reach_the_markup() {
        let pal = CanvasStyle::default().palette;
        let span = Span::Text {
            text: "Hi".into(),
            style: Style {
                small_caps: true,
                font: Some("EB Garamond".into()),
                size_pct: Some(150),
                ..Default::default()
            },
            link: None,
        };
        let (on, _, _) = markup_part(&span, &pal, false, None, true)
            .map(|(p, l)| (markup_of(std::slice::from_ref(&p)), 0, l))
            .unwrap();
        assert!(on.contains("font_scale=\"small-caps\""), "{on}");
        assert!(on.contains("font_family=\"EB Garamond\""), "{on}");
        assert!(on.contains("size=\"150%\""), "{on}");
        let (off, _, _) = markup_part(&span, &pal, false, None, false)
            .map(|(p, l)| (markup_of(std::slice::from_ref(&p)), 0, l))
            .unwrap();
        assert!(
            off.contains("small-caps") && !off.contains("font_family"),
            "{off}"
        );
        assert!(!off.contains("size="), "{off}");
        // a name that could break the markup is never written
        let bad = Span::Text {
            text: "x".into(),
            style: Style {
                font: Some("a\" onload=\"".into()),
                ..Default::default()
            },
            link: None,
        };
        let (m, _) = markup_part(&bad, &pal, false, None, true).unwrap();
        assert!(!m.open.contains("onload"), "{}", m.open);
    }

    #[test]
    fn word_spacing_and_kerning_wrap_the_markup() {
        let m = wrap_text_markup(
            "a b <span foreground=\"#fff\">c d</span>".into(),
            true,
            0.0,
            16.0,
        );
        assert_eq!(m, "a b <span foreground=\"#fff\">c d</span>");
        let m = wrap_text_markup("a b".into(), false, 0.5, 16.0);
        assert!(m.starts_with("<span font_features=\"kern=0\">"), "{m}");
        assert_eq!(m.matches("letter_spacing=").count(), 1, "{m}");
        // spaces inside tags are left alone
        let m = wrap_text_markup(
            "<span foreground=\"#fff\">c d</span>".into(),
            true,
            0.25,
            16.0,
        );
        assert!(
            m.contains("<span foreground=\"#fff\">c<span letter_spacing="),
            "{m}"
        );
        assert_eq!(weight_of(590), pango::Weight::Semibold);
        assert_eq!(weight_of(-5), pango::Weight::Thin);
        assert_eq!(weight_of(9000), pango::Weight::Heavy);
    }

    #[test]
    fn text_colour_of_the_book_shows_in_the_markup_only_in_original_mode() {
        let pal = CanvasStyle::default().palette;
        let span = Span::Text {
            text: "hi".into(),
            style: Style {
                color: Some((0x12, 0xab, 0xef)),
                ..Default::default()
            },
            link: None,
        };
        let (off, _, _) = markup_for(&span, &pal, false, None).unwrap();
        assert!(!off.contains("foreground"), "{off}");
        let (on, _, _) = markup_for(&span, &pal, true, None).unwrap();
        assert!(on.contains("foreground=\"#12abef\""), "{on}");
        // links keep the link colour
        let link = Span::Text {
            text: "l".into(),
            style: Style {
                color: Some((1, 2, 3)),
                ..Default::default()
            },
            link: Some("x.xhtml".into()),
        };
        let (m, _, is_link) = markup_for(&link, &pal, true, None).unwrap();
        assert!(is_link && !m.contains("#010203"), "{m}");
    }
}

#[cfg(test)]
mod break_tests {
    use super::*;

    const LB: char = paragraph::LINE_BREAK;
    const GH: char = paragraph::GHOST_HYPHEN;

    #[test]
    fn ghost_characters_do_not_count_as_book_text() {
        // "extra-" at a hyphenated break, a space turned into a line break, a soft hyphen
        let t = format!("ex{GH}{LB}tra{LB}ord\u{ad}er");
        assert_eq!(canon_text(&t).collect::<String>(), "extra order");
        assert_eq!(paragraph::canonical_chars(&t).count(), 11);
        // canonical index <-> layout index
        assert_eq!(layout_char_of_canon(&t, 2), 4);
        assert_eq!(layout_char_of_canon(&t, 5), 5 + 2);
        assert_eq!(canon_of_layout_char(&t, 4), 2);
        assert_eq!(canon_of_layout_char(&t, t.chars().count()), 11);
    }

    #[test]
    fn the_two_mappings_agree_for_every_canonical_char() {
        let t = format!("a{LB}b\u{ad}c d{GH}{LB}e");
        for c in 0..paragraph::canonical_chars(&t).count() {
            let l = layout_char_of_canon(&t, c);
            assert_eq!(canon_of_layout_char(&t, l), c);
        }
    }

    fn parts(texts: &[&str]) -> Vec<Part> {
        texts
            .iter()
            .enumerate()
            .map(|(i, t)| Part {
                open: format!("<o{i}>"),
                text: t.to_string(),
                close: format!("</o{i}>"),
            })
            .collect()
    }

    #[test]
    fn a_paragraph_is_split_inside_a_part_with_the_tags_repeated() {
        let (h, t) = split_parts(&parts(&["abc", "defg", "hi"]), 5);
        let join = |v: &[Part]| v.iter().map(|p| p.text.clone()).collect::<String>();
        assert_eq!(join(&h), "abcde");
        assert_eq!(join(&t), "fghi");
        assert_eq!(h[1].open, "<o1>");
        assert_eq!(t[0].open, "<o1>");
        // on a boundary nothing is duplicated
        let (h, t) = split_parts(&parts(&["abc", "def"]), 3);
        assert_eq!((h.len(), t.len()), (1, 1));
    }

    #[test]
    fn links_follow_the_split() {
        let links = vec![(0, 4, "a".to_string()), (6, 9, "b".to_string())];
        let (h, t) = split_links(&links, 3);
        assert_eq!(h, vec![(0, 3, "a".to_string())]);
        assert_eq!(t, vec![(0, 1, "a".to_string()), (3, 6, "b".to_string())]);
    }

    #[test]
    fn the_drop_cap_letter_stays_in_the_text_as_an_invisible_dot() {
        let mut p = parts(&["Once upon", " a time"]);
        let before: String = p.iter().map(|q| q.text.clone()).collect();
        let (letter, _) = take_drop_cap(&mut p).unwrap();
        assert_eq!(letter, "O");
        let after: String = p.iter().map(|q| q.text.clone()).collect();
        assert_eq!(before, after, "the text, and so every offset, is unchanged");
        assert!(p[0].open.contains("fgalpha"));
        // no drop cap for a quote or a digit-less start
        assert!(take_drop_cap(&mut parts(&["“Quoted"])).is_none());
        assert!(take_drop_cap(&mut parts(&[""])).is_none());
    }
}

#[cfg(test)]
mod release_tests {
    use super::*;

    #[test]
    fn a_double_click_or_long_press_keeps_the_word_it_selected() {
        // regression (bug 1): the release after the selecting press counted as a click, which
        // cleared the selection and closed the popover with the dictionary and Wikipedia
        assert_eq!(release_action(false, false, false, true), Release::Keep);
        assert_eq!(release_action(false, true, true, false), Release::Keep);
    }

    #[test]
    fn the_other_releases_are_unchanged() {
        assert_eq!(release_action(false, false, false, false), Release::Click);
        assert_eq!(
            release_action(true, false, false, false),
            Release::Selection
        );
        assert_eq!(release_action(true, true, false, false), Release::Swipe);
        // a finger dragging after a long press extends the selection
        assert_eq!(release_action(true, true, true, false), Release::Selection);
    }
}

#[cfg(test)]
mod vertical_tests {
    use super::*;

    /// A Pango context for vertical lines, as the reader builds it.
    fn vertical_context(hint: pango::GravityHint) -> pango::Context {
        let ctx = pangocairo::FontMap::default().create_context();
        ctx.set_base_gravity(pango::Gravity::East);
        ctx.set_gravity_hint(hint);
        ctx
    }

    fn layout(ctx: &pango::Context, text: &str, width_em: f64) -> pango::Layout {
        let l = pango::Layout::new(ctx);
        let mut fd = pango::FontDescription::new();
        fd.set_family("Noto Serif CJK JP, Serif");
        fd.set_absolute_size(20.0 * SCALE);
        l.set_font_description(Some(&fd));
        l.set_width((width_em * 20.0 * SCALE) as i32);
        l.set_wrap(pango::WrapMode::WordChar);
        l.set_text(text);
        l
    }

    fn line_texts(l: &pango::Layout) -> Vec<String> {
        let text = l.text().to_string();
        (0..l.line_count())
            .filter_map(|i| l.line_readonly(i))
            .map(|line| {
                let (a, n) = (line.start_index() as usize, line.length() as usize);
                text[a..a + n].to_string()
            })
            .collect()
    }

    #[test]
    fn pango_keeps_kinsoku_at_every_line_width() {
        let ctx = vertical_context(pango::GravityHint::Natural);
        let texts = [
            "吾輩は猫である。名前はまだ無い。どこで生れたかとんと見当がつかぬ。「何でも」薄暗い、じめじめした所で。",
            "彼は「行こう」と言った。（本当に？）ええ、……そうです――ですから！ニャーニャー泣いていた事だけ。",
            "ぁぃぅっゃゅょ小さな仮名やー長音、々踊り字。」」あ。。い、、うれしいッ！ちゃんとした。",
        ];
        for text in texts {
            for width in 6..=14 {
                let l = layout(&ctx, text, width as f64);
                decorate(&l, &[], 20.0);
                let lines = line_texts(&l);
                assert!(lines.len() > 1, "{text}");
                let bad =
                    vertical::violations(&lines.iter().map(String::as_str).collect::<Vec<_>>());
                assert!(bad.is_empty(), "width {width}: {lines:?} {bad:?}");
            }
        }
    }

    #[test]
    fn a_vertical_context_lays_lines_out_along_the_x_axis_of_the_layout() {
        // the page is turned afterwards, so what is measured along the line is the layout's width
        let ctx = vertical_context(pango::GravityHint::Natural);
        let l = layout(&ctx, "あいうえおかきくけこ", 6.0);
        assert_eq!(l.line_count(), 2);
        let (w, h) = l.pixel_size();
        assert!((90..=6 * 20 + 2).contains(&w), "width {w}");
        assert!(h >= 2 * 20, "two lines stack along y: {h}");
    }

    #[test]
    fn combined_digits_reserve_one_em_in_the_line() {
        let ctx = vertical_context(pango::GravityHint::Natural);
        let l = layout(&ctx, "あ12い", 40.0);
        let combine = [("あ".len(), "あ12".len())];
        decorate(&l, &combine, 20.0);
        let a = px(l.index_to_pos("あ".len() as i32).width());
        let b = px(l.index_to_pos("あ1".len() as i32).width());
        assert!(
            (a + b - 20.0).abs() < 1.5,
            "the two digits take one em: {a} + {b}"
        );
    }

    #[test]
    fn squeezed_punctuation_is_never_wider() {
        let ctx = vertical_context(pango::GravityHint::Natural);
        let plain = layout(&ctx, "彼は「あ」「い」と。」", 40.0);
        let before = px(plain.size().0);
        let squeezed = layout(&ctx, "彼は「あ」「い」と。」", 40.0);
        decorate(&squeezed, &[], 20.0);
        let after = px(squeezed.size().0);
        assert!(after <= before + 0.5, "{before} -> {after}");
        // with the real font the closing brackets before an opening one are half an em narrower
        let has_cjk = pangocairo::FontMap::default()
            .list_families()
            .iter()
            .any(|f| f.name().contains("CJK"));
        if has_cjk {
            assert!(after < before - 5.0, "{before} -> {after}");
        }
    }

    #[test]
    fn upright_orientation_stands_latin_letters_up_and_mixed_turns_them() {
        let mixed = layout(&vertical_context(pango::GravityHint::Natural), "ABCD", 40.0);
        let upright = layout(&vertical_context(pango::GravityHint::Strong), "ABCD", 40.0);
        // upright letters take their full height along the line, turned ones their width
        assert!(
            px(upright.size().0) > px(mixed.size().0) + 4.0,
            "{} vs {}",
            px(upright.size().0),
            px(mixed.size().0)
        );
    }
}

#[cfg(test)]
mod xtable_tests {
    use super::*;

    fn one_line(text: &str) -> pango::Layout {
        let ctx = pangocairo::FontMap::default().create_context();
        let l = pango::Layout::new(&ctx);
        let mut fd = pango::FontDescription::new();
        fd.set_family("Serif");
        fd.set_absolute_size(18.0 * SCALE);
        l.set_font_description(Some(&fd));
        l.set_width(-1);
        l.set_text(text);
        l
    }

    /// The table answers exactly what `index_to_pos` answers, for every offset it knows, and it knows
    /// the start of every word (the offsets the Knuth-Plass planner asks for).
    #[test]
    fn cluster_table_matches_pango_positions() {
        for text in [
            "The quick brown fox jumps over the lazy dog, and then it rests.",
            "Käse, Brötchen und Ärger über Müllers Straße — naïve café",
            "fi fl ffi ligatures: office, affluent, finish",
        ] {
            let l = one_line(text);
            let t = cluster_table(&l, text.len());
            assert!(
                t.len() > text.chars().count() / 2,
                "{text}: {} entries",
                t.len()
            );
            let mut known = 0;
            for b in (0..=text.len()).filter(|b| text.is_char_boundary(*b)) {
                if let Some(x) = t.get(b) {
                    known += 1;
                    let want = l.index_to_pos(b as i32).x() as f64 / SCALE;
                    assert!((x - want).abs() < 0.01, "{text}: byte {b}: {x} vs {want}");
                }
            }
            assert!(known > 10);
            for (b, _) in text.match_indices(' ') {
                assert!(t.get(b).is_some(), "{text}: no entry at the space at {b}");
            }
            assert!(
                t.get(text.len()).is_some(),
                "the end of the text is in the table"
            );
        }
    }
}
