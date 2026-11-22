//! The PDF reader page: pages as the PDF draws them (continuous or one at a time), zoom, crop,
//! night colours, thumbnails, contents, search, text selection with highlights and notes, and
//! links. The engine is `bookx-pdf` on a worker thread; the layout maths is in `crate::pdf`.
//!
//! Rendering never blocks the window: pages are requested from the worker for the visible part
//! (and a screen around it), come back as events, and are kept in a cache of cairo surfaces with a
//! byte budget. While a page is being rendered at a new zoom the last rendering of it is drawn
//! scaled.

use crate::i18n::{tr, trf, N_};
use crate::side_notes;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use bookx_pdf::{
    Bitmap, Cancel, Event, Hit, Link, LinkTarget, OpenError, PageCache, PageText, Summary, Worker,
};
use bookx_text::highlight::HighlightStyle;
use chrono::Utc;
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, cairo, gdk, gio, glib};
use relm4::prelude::*;

use super::reader::HIGHLIGHT_COLORS;
use super::web;
use super::{open_external_link, open_uri, window_of, SharedRef};
use crate::core::library::{Bookmark, Highlight, ReadPos};
use crate::pdf::{self, Layout, PdfPrefs, Place, ViewMode, ZoomMode};

/// Budget of the cache of rendered pages.
const CACHE_BYTES: usize = 384 << 20;
/// Thumbnails are rendered at this many pixels per point.
const THUMB_SCALE: f32 = 0.2;
/// Pages sampled to find the margins to crop.
const CROP_SAMPLES: usize = 9;

type Key = (usize, u32, bool);

struct Surf {
    surface: cairo::ImageSurface,
    /// Pixels per point it was rendered at.
    scale: f32,
}

#[derive(Debug, Clone, PartialEq)]
struct Sel {
    page: usize,
    start: usize,
    end: usize,
    /// The stored highlight this selection stands for.
    highlight: Option<String>,
}

struct HlDraw {
    id: String,
    page: usize,
    rects: Vec<[f32; 4]>,
    color: (f64, f64, f64),
    style: HighlightStyle,
    has_note: bool,
}

/// What the drawing function and the pointer handlers share.
struct ViewState {
    layout: Layout,
    places: Vec<Place>,
    current: usize,
    cache: PageCache<Key, Rc<Surf>>,
    fallback: HashMap<usize, Rc<Surf>>,
    texts: HashMap<usize, Arc<PageText>>,
    links: HashMap<usize, Arc<Vec<Link>>>,
    highlights: Vec<HlDraw>,
    selection: Option<Sel>,
    hits: Vec<(usize, Range<usize>)>,
    active_hit: Option<usize>,
    invert: bool,
    dark: bool,
}

#[derive(Debug, Clone)]
pub struct SelectionInfo {
    #[allow(dead_code)]
    pub page: usize,
    pub text: String,
}

#[derive(Debug)]
pub enum Input {
    Poll,
    Resize,
    Scrolled,
    GoPage(usize),
    #[allow(dead_code)]
    GoDest(usize, Option<f32>),
    Turn(bool),
    Seek(f64),
    PageBox(String),
    SetZoom(ZoomMode),
    ZoomStep(bool),
    ZoomBy(f64),
    SetView(ViewMode),
    SetCrop(bool),
    SetInvert(bool),
    Press(f64, f64),
    Drag(f64, f64),
    #[allow(dead_code)]
    DragEnd(f64, f64),
    Click(f64, f64, i32),
    Hover(f64, f64),
    Highlight(String),
    HlStyle(HighlightStyle),
    SaveNote(String),
    RemoveHighlight,
    /// Undo of a removal: the items are back in the library, show them again.
    Restored,
    /// A sync changed this book, or a device reported a position (see `sync_bus`).
    Sync(crate::sync_bus::SyncEvent),
    /// A button of the sync banner.
    LiveAnswer(super::reader_live::Answer),
    PopClosed,
    Copy,
    Dict,
    Wiki,
    Translate,
    TranslateLang(String),
    Search(String),
    SearchResult(usize),
    SearchStep(bool),
    ToggleBookmark,
    GoBookmark(String),
    RemoveBookmark(String),
    GoHighlight(String),
    DeleteHighlight(String),
    ThumbWant(usize),
    ThumbDrop(usize),
    ShowTab(&'static str),
    Back,
    Reflow,
    Properties,
    Fullscreen,
    Password(Option<String>),
    SelectionKey(&'static str),
    Quit,
    Dev(String),
}

#[derive(Debug)]
pub enum Output {
    Close,
    Toast(String),
    /// A message with an Undo button (a highlight or bookmark was removed).
    ToastUndo(String, crate::app::UndoFn),
    /// Open the reflowed text of this book in the EPUB reader.
    Reflow,
}

struct W {
    page: adw::NavigationPage,
    title: adw::WindowTitle,
    split: adw::OverlaySplitView,
    stack: adw::ViewStack,
    area: gtk::DrawingArea,
    #[allow(dead_code)]
    vbar: gtk::Scrollbar,
    hbar: gtk::Scrollbar,
    content: gtk::Stack,
    toc_list: gtk::ListBox,
    bm_list: gtk::ListBox,
    hl_list: gtk::ListBox,
    thumbs: gtk::ListView,
    search_entry: gtk::SearchEntry,
    search_btn: gtk::MenuButton,
    search_list: gtk::ListBox,
    search_status: gtk::Label,
    search_empty: adw::StatusPage,
    slider: gtk::Scale,
    page_entry: gtk::Entry,
    primary: gtk::MenuButton,
    header: adw::HeaderBar,
    narrow: Rc<Cell<bool>>,
    fullscreen_btn: gtk::Button,
    page_total: gtk::Label,
    caption: gtk::Label,
    bookmark_btn: gtk::Button,
    back_btn: gtk::Button,
    prev_btn: gtk::Button,
    next_btn: gtk::Button,
    zoom_label: gtk::Label,
    sel_pop: gtk::Popover,
    sel_stack: gtk::Stack,
    sel_remove: gtk::Button,
    switching: Rc<Cell<bool>>,
    note_view: gtk::TextView,
    note_btn: gtk::Button,
    note_preview: gtk::Button,
    note_label: gtk::Label,
    swatches: Vec<gtk::Button>,
    style_btns: Vec<(HighlightStyle, gtk::ToggleButton)>,
    dict_text: gtk::Label,
    wiki_text: gtk::Label,
    wiki_spin: gtk::Spinner,
    wiki_url: Rc<RefCell<String>>,
    tr_text: gtk::Label,
    tr_spin: gtk::Spinner,
    tr_dd: gtk::DropDown,
    zoom_btns: Vec<(String, gtk::CheckButton)>,
    view_btns: Vec<(ViewMode, gtk::CheckButton)>,
    crop_sw: gtk::Switch,
    invert_sw: gtk::Switch,
    thumb_pics: Rc<RefCell<HashMap<usize, glib::WeakRef<gtk::Picture>>>>,
    thumb_tex: Rc<RefCell<HashMap<usize, gdk::Texture>>>,
}

struct SearchRow {
    page: usize,
    range: Range<usize>,
    snippet: String,
    mark: Range<usize>,
}

pub struct ReaderPdf {
    shared: SharedRef,
    book_id: u64,
    title: String,
    language: String,
    data: Arc<Vec<u8>>,
    worker: Option<Worker>,
    summary: Option<Rc<Summary>>,
    state: Rc<RefCell<ViewState>>,
    hadj: gtk::Adjustment,
    vadj: gtk::Adjustment,
    w: W,
    sender: ComponentSender<Self>,
    prefs: PdfPrefs,
    pending: HashMap<Key, Cancel>,
    text_pending: HashSet<usize>,
    thumb_pending: HashMap<usize, Cancel>,
    crop_wait: usize,
    crop_boxes: Vec<[f32; 4]>,
    selection: Option<SelectionInfo>,
    drag: Option<(usize, usize, f64, f64)>,
    press_at: (f64, f64),
    history: Vec<(usize, Option<f32>)>,
    search_id: u64,
    search_rows: Vec<SearchRow>,
    search_done: bool,
    search_active: Option<usize>,
    dirty: Rc<Cell<bool>>,
    timer: Option<glib::SourceId>,
    poll: Option<glib::SourceId>,
    fetch_gen: Rc<Cell<u64>>,
    hl_style: HighlightStyle,
    fullscreen: bool,
    opened: bool,
    asked_password: bool,
    updating_ui: Rc<Cell<bool>>,
    last_page_stat: usize,
    sync_banner: Rc<super::reader_live::LiveBanner>,
    _sync_sub: Option<crate::sync_bus::Subscription>,
    sync_stats: Cell<(u32, u32)>,
    last_turn: Cell<usize>,
}

fn rgb_of(name: &str) -> (f64, f64, f64) {
    HIGHLIGHT_COLORS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, c)| *c)
        .unwrap_or(HIGHLIGHT_COLORS[0].1)
}

fn icon_button(icon: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(tip));
    super::audit::label(&b, tip);
    b
}

fn surface_of(b: &Bitmap) -> Option<cairo::ImageSurface> {
    let (w, h) = (b.width as usize, b.height as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let mut s = cairo::ImageSurface::create(cairo::Format::ARgb32, w as i32, h as i32).ok()?;
    let stride = s.stride() as usize;
    {
        let mut d = s.data().ok()?;
        for y in 0..h {
            let src = &b.rgba[y * w * 4..(y + 1) * w * 4];
            let dst = &mut d[y * stride..y * stride + w * 4];
            for (s4, d4) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                d4[0] = s4[2];
                d4[1] = s4[1];
                d4[2] = s4[0];
                d4[3] = s4[3];
            }
        }
    }
    Some(s)
}

fn texture_of(b: &Bitmap) -> gdk::Texture {
    gdk::MemoryTexture::new(
        b.width as i32,
        b.height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from(&b.rgba[..]),
        b.width as usize * 4,
    )
    .upcast()
}

impl ViewState {
    fn size_pt(&self, page: usize) -> (f32, f32) {
        self.layout
            .sizes
            .get(page)
            .copied()
            .unwrap_or((612.0, 792.0))
    }

    fn place_of(&self, page: usize) -> Option<&Place> {
        self.places.iter().find(|p| p.page == page)
    }

    fn to_px(&self, pl: &Place, r: [f32; 4]) -> [f32; 4] {
        pdf::to_px(pl, self.layout.crop, self.size_pt(pl.page), r)
    }

    /// The page and the point on it (points) under a position of the document in pixels; the
    /// nearest page when the position is between pages and `nearest` is set.
    fn page_at(&self, x: f32, y: f32, nearest: bool) -> Option<(usize, f32, f32)> {
        let hit = self
            .places
            .iter()
            .find(|p| x >= p.x && x <= p.x + p.w && y >= p.y && y <= p.y + p.h);
        let pl = match hit {
            Some(p) => p,
            None if nearest => self.places.iter().min_by(|a, b| {
                let d = |p: &Place| {
                    if y < p.y {
                        p.y - y
                    } else if y > p.y + p.h {
                        y - (p.y + p.h)
                    } else {
                        0.0
                    }
                };
                d(a).total_cmp(&d(b))
            })?,
            None => return None,
        };
        let (px, py) = pdf::to_pt(pl, self.layout.crop, self.size_pt(pl.page), x, y);
        Some((pl.page, px, py))
    }
}

impl ReaderPdf {
    fn page_count(&self) -> usize {
        self.summary
            .as_ref()
            .map(|s| s.page_sizes.len())
            .unwrap_or(0)
    }

    fn labels(&self) -> Vec<String> {
        self.summary
            .as_ref()
            .map(|s| s.labels.clone())
            .unwrap_or_default()
    }

    fn view_size(&self) -> (f32, f32) {
        (
            self.w.area.width().max(1) as f32,
            self.w.area.height().max(1) as f32,
        )
    }

    /// Lays the pages out for the window, the zoom and the mode, keeps the reading position.
    fn relayout(&mut self) {
        let (vw, vh) = self.view_size();
        let (page, frac) = {
            let st = self.state.borrow();
            let top = self.vadj.value() as f32;
            // the reading line of the layout that is on screen now, not of the new window size
            let anchor = top + st.layout.viewport.1 * 0.35;
            match st.places.iter().find(|p| p.y + p.h >= anchor) {
                Some(p) if st.layout.mode == ViewMode::Continuous => {
                    (p.page, ((anchor - p.y) / p.h.max(1.0)).clamp(0.0, 1.0))
                }
                _ => (st.current, 0.0),
            }
        };
        {
            let mut st = self.state.borrow_mut();
            st.layout.viewport = (vw, vh);
            st.current = page.min(st.layout.sizes.len().saturating_sub(1));
            st.places = st.layout.places(st.current);
        }
        self.configure_adjustments(Some((page, frac)));
        self.update_zoom_label();
        self.w.area.queue_draw();
        self.request_visible();
    }

    fn configure_adjustments(&self, keep: Option<(usize, f32)>) {
        let (vw, vh) = self.view_size();
        let st = self.state.borrow();
        let (cw, ch) = Layout::content_size(&st.places, st.layout.margin, (vw, vh));
        let target = keep
            .and_then(|(page, frac)| st.place_of(page).map(|p| p.y + p.h * frac - vh * 0.35))
            .unwrap_or(self.vadj.value() as f32);
        let hv = self.hadj.value();
        drop(st);
        self.vadj
            .configure(0.0, 0.0, ch as f64, 48.0, (vh * 0.9) as f64, vh as f64);
        self.vadj
            .set_value((target as f64).clamp(0.0, (ch - vh).max(0.0) as f64));
        self.hadj.configure(
            hv.clamp(0.0, (cw - vw).max(0.0) as f64),
            0.0,
            cw as f64,
            32.0,
            (vw * 0.9) as f64,
            vw as f64,
        );
        self.w.hbar.set_visible(cw > vw + 1.0);
    }

    fn update_zoom_label(&self) {
        let st = self.state.borrow();
        let s = st.places.first().map(|p| p.scale).unwrap_or(1.0);
        let pct = (s / pdf::PX_PER_PT * 100.0).round() as i64;
        self.w.zoom_label.set_label(&format!("{pct}%"));
        let name = st.layout.zoom.name();
        drop(st);
        self.updating_ui.set(true);
        for (n, b) in &self.w.zoom_btns {
            if *n == name {
                b.set_active(true);
            }
        }
        self.updating_ui.set(false);
    }

    /// Asks the worker for the pages in and around the window.
    fn request_visible(&mut self) {
        let Some(worker) = &self.worker else { return };
        let dpr = self.w.area.scale_factor().max(1) as f32;
        let (_, vh) = self.view_size();
        let top = self.vadj.value() as f32;
        let mut wanted: HashSet<Key> = HashSet::new();
        let st = self.state.borrow();
        let vis = pdf::visible(&st.places, top - vh * 0.5, vh * 2.0);
        for idx in vis {
            let pl = st.places[idx];
            let key = pdf::render_key(pl.page, pdf::render_scale(pl.scale * dpr), st.invert);
            wanted.insert(key);
            if !st.cache.contains(&key) && !self.pending.contains_key(&key) {
                let c = worker.render(pl.page, key.1 as f32 / 1000.0, st.invert);
                self.pending.insert(key, c);
            }
            if !st.texts.contains_key(&pl.page) && self.text_pending.insert(pl.page) {
                worker.page(pl.page);
            }
        }
        drop(st);
        self.pending.retain(|k, c| {
            if wanted.contains(k) {
                true
            } else {
                c.cancel();
                false
            }
        });
    }

    fn current_page(&self) -> usize {
        self.state.borrow().current
    }

    /// The page under the reading line has changed: footer, slider, title, progress.
    fn on_scrolled(&mut self) {
        let (vh, top) = (self.view_size().1, self.vadj.value() as f32);
        let changed = {
            let mut st = self.state.borrow_mut();
            if st.layout.mode == ViewMode::Continuous && !st.places.is_empty() {
                let i = pdf::current_in(&st.places, top, vh);
                let page = st.places[i].page;
                let c = st.current != page;
                st.current = page;
                c
            } else {
                false
            }
        };
        self.w.area.queue_draw();
        self.request_visible();
        if changed || !self.opened {
            self.update_position_ui();
        }
        self.dirty.set(true);
    }

    fn page_fraction(&self) -> f64 {
        let st = self.state.borrow();
        let (vh, top) = (self.view_size().1, self.vadj.value() as f32);
        match st.place_of(st.current) {
            Some(p) if st.layout.mode == ViewMode::Continuous => {
                (((top + vh * 0.35 - p.y) / p.h.max(1.0)).clamp(0.0, 1.0)) as f64
            }
            Some(p) => ((top / p.h.max(1.0)).clamp(0.0, 1.0)) as f64,
            None => 0.0,
        }
    }

    fn update_position_ui(&self) {
        let n = self.page_count();
        if n == 0 {
            return;
        }
        let cur = self.current_page();
        let labels = self.labels();
        self.updating_ui.set(true);
        self.w
            .caption
            .set_label(&pdf::page_caption(&labels, cur, n));
        self.w
            .title
            .set_subtitle(&pdf::page_caption(&labels, cur, n));
        self.w.slider.set_value(cur as f64);
        self.w.page_entry.set_text(
            labels
                .get(cur)
                .filter(|l| !l.is_empty())
                .cloned()
                .unwrap_or_else(|| (cur + 1).to_string())
                .as_str(),
        );
        self.w.page_total.set_label(&trf("of {}", &[&n]));
        self.w.prev_btn.set_sensitive(cur > 0);
        self.w.next_btn.set_sensitive(cur + 1 < n);
        self.updating_ui.set(false);
        self.update_bookmark_button();
        self.w.toc_list.unselect_all();
        if let Some(s) = &self.summary {
            if let Some(i) = s
                .outline
                .iter()
                .enumerate()
                .rev()
                .find(|(_, o)| o.dest.is_some_and(|d| d.page <= cur))
                .map(|(i, _)| i)
            {
                if let Some(row) = self.w.toc_list.row_at_index(i as i32) {
                    self.w.toc_list.select_row(Some(&row));
                }
            }
        }
    }

    fn update_bookmark_button(&self) {
        let cur = self.current_page();
        if self.last_turn.replace(cur) != cur {
            self.sync_banner.page_turned();
        }
        let has = self
            .shared
            .lib
            .borrow()
            .get(self.book_id)
            .is_some_and(|b| b.bookmarks.iter().any(|m| m.spine == cur));
        self.w.bookmark_btn.set_icon_name(if has {
            "user-bookmarks-symbolic"
        } else {
            "bookmark-new-symbolic"
        });
        self.w.bookmark_btn.set_tooltip_text(Some(
            &(if has {
                tr("Remove the bookmark on this page (Ctrl+B)")
            } else {
                tr("Add a bookmark on this page (Ctrl+B)")
            }),
        ));
    }

    fn goto_page(&mut self, page: usize, y_pt: Option<f32>) {
        let n = self.page_count();
        if n == 0 {
            return;
        }
        let page = page.min(n - 1);
        let single = self.state.borrow().layout.mode == ViewMode::Single;
        if single {
            {
                let mut st = self.state.borrow_mut();
                st.current = page;
                st.places = st.layout.places(page);
            }
            self.configure_adjustments(None);
        }
        let (_, vh) = self.view_size();
        let target = {
            let st = self.state.borrow();
            st.place_of(page).map(|p| {
                let off = y_pt
                    .map(|y| {
                        let crop_y = st
                            .layout
                            .crop
                            .map(|c| c[1] * st.size_pt(page).1)
                            .unwrap_or(0.0);
                        ((y - crop_y) * p.scale - 8.0).max(0.0)
                    })
                    .unwrap_or(0.0);
                if single {
                    off
                } else {
                    p.y - st.layout.margin + off
                }
            })
        };
        if let Some(t) = target {
            let max = (self.vadj.upper() - vh as f64).max(0.0);
            self.vadj.set_value((t as f64).clamp(0.0, max));
        }
        self.state.borrow_mut().current = page;
        self.hadj.set_value(0.0);
        self.w.area.queue_draw();
        self.request_visible();
        self.update_position_ui();
        self.save_progress();
    }

    fn save_progress(&self) {
        let n = self.page_count();
        if n == 0 || !self.opened {
            return;
        }
        let page = self.current_page();
        let frac = self.page_fraction();
        let p = pdf::progress(page, frac, n);
        {
            let mut lib = self.shared.lib.borrow_mut();
            lib.set_progress(
                self.book_id,
                ReadPos { spine: page, frac },
                if page + 1 >= n && frac > 0.6 { 1.0 } else { p },
            );
        }
        self.dirty.set(true);
        let now = self.shared.now();
        let mut hub = self.shared.stats.borrow_mut();
        hub.on_position(self.book_id, now, (0, page), p, 250);
        if p >= 0.98 {
            hub.mark_finished(self.book_id, now.with_timezone(&chrono::Local).date_naive());
        }
    }

    fn refresh_highlights(&self) {
        let lib = self.shared.lib.borrow();
        let list: Vec<HlDraw> = lib
            .get(self.book_id)
            .map(|b| {
                b.highlights
                    .iter()
                    .filter(|h| pdf::is_pdf(h))
                    .filter_map(|h| {
                        let (style, rects) = pdf::parse_pdf_style(&h.style)?;
                        Some(HlDraw {
                            id: h.id.clone(),
                            page: h.spine,
                            rects,
                            color: rgb_of(&h.color),
                            style: HighlightStyle::from_name(&style).unwrap_or_default(),
                            has_note: !h.note.is_empty(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        drop(lib);
        self.state.borrow_mut().highlights = list;
        self.w.area.queue_draw();
    }

    /// The toast "Highlight removed" / "Bookmark removed" with an Undo that puts the item back.
    fn offer_undo(&self, r: crate::remove_undo::Removed, sender: &ComponentSender<Self>) {
        let text = crate::remove_undo::message(&r);
        let stash = Rc::new(RefCell::new(Some(r)));
        let (shared, id, tx) = (
            self.shared.clone(),
            self.book_id,
            sender.input_sender().clone(),
        );
        let undo = crate::app::UndoFn::new(move || {
            let Some(r) = stash.borrow_mut().take() else {
                return;
            };
            {
                let mut lib = shared.lib.borrow_mut();
                if let Some(b) = lib.get_mut(id) {
                    crate::remove_undo::restore(&mut b.highlights, &mut b.bookmarks, r);
                }
                let _ = lib.save();
            }
            tx.emit(Input::Restored);
        });
        let _ = sender.output(Output::ToastUndo(text, undo));
    }

    /// A sync changed this book: paint the highlights again, refresh the lists and the bookmark
    /// button (the lists keep their scroll position), and offer to jump when a device is ahead.
    fn on_sync(&mut self, ev: &crate::sync_bus::SyncEvent) {
        let (heard, repaints) = self.sync_stats.get();
        self.sync_stats.set((heard + 1, repaints));
        if ev.kinds.annotations() {
            let root: gtk::Widget = self.w.split.clone().upcast();
            super::sync_activity::keep_scroll(&root, || {
                self.refresh_highlights();
                self.rebuild_lists();
                self.update_bookmark_button();
            });
            self.sync_stats.set((heard + 1, repaints + 1));
        }
        if let Some(a) = &ev.ahead {
            let (enabled, mine) = {
                let c = self.shared.cfg.borrow();
                (c.share.offer_jump, c.kosync.device_id.clone())
            };
            let n = self.page_count();
            let local = if n > 0 {
                self.current_page() as f64 / n as f64
            } else {
                0.0
            };
            let same = !mine.is_empty() && a.device_id == mine;
            if crate::sync_live::worth_offering(enabled, local, a.percentage, same) {
                let page = crate::sync_live::page_at(a.percentage, n);
                self.sync_banner.offer_ahead(&ev.device, a.percentage, page);
            }
        }
    }

    fn rebuild_lists(&self) {
        // contents
        clear_list(&self.w.toc_list);
        if let Some(s) = &self.summary {
            if s.outline.is_empty() {
                self.w.toc_list.append(&placeholder(
                    &tr("No table of contents"),
                    &tr("This PDF has no bookmarks"),
                ));
            }
            for o in &s.outline {
                let row = gtk::ListBoxRow::new();
                let l = gtk::Label::new(Some(if o.title.is_empty() {
                    "(untitled)"
                } else {
                    &o.title
                }));
                l.set_xalign(0.0);
                l.set_wrap(true);
                l.set_margin_start(8 + 14 * o.level.min(8) as i32);
                l.set_margin_end(8);
                l.set_margin_top(6);
                l.set_margin_bottom(6);
                if o.level == 0 {
                    l.add_css_class("heading");
                }
                row.set_child(Some(&l));
                row.set_sensitive(o.dest.is_some());
                self.w.toc_list.append(&row);
            }
        }
        // bookmarks and notes
        clear_list(&self.w.bm_list);
        clear_list(&self.w.hl_list);
        let labels = self.labels();
        let lib = self.shared.lib.borrow();
        let Some(b) = lib.get(self.book_id) else {
            return;
        };
        let page_name = |p: usize| {
            labels
                .get(p)
                .cloned()
                .unwrap_or_else(|| (p + 1).to_string())
        };
        let mut marks: Vec<&Bookmark> = b.bookmarks.iter().collect();
        marks.sort_by_key(|m| m.spine);
        if marks.is_empty() {
            self.w.bm_list.append(&placeholder(
                &tr("No bookmarks"),
                &tr("Press Ctrl+B to bookmark the page"),
            ));
        }
        for m in marks {
            let row = adw::ActionRow::builder()
                .title(trf("Page {}", &[&page_name(m.spine)]))
                .subtitle(glib::markup_escape_text(&m.label))
                .activatable(true)
                .build();
            let del = icon_button("user-trash-symbolic", &tr("Remove Bookmark"));
            del.add_css_class("flat");
            del.set_valign(gtk::Align::Center);
            let (tx, id) = (self.sender.input_sender().clone(), m.id.clone());
            del.connect_clicked(move |_| tx.emit(Input::RemoveBookmark(id.clone())));
            row.add_suffix(&del);
            let (tx, id) = (self.sender.input_sender().clone(), m.id.clone());
            row.connect_activated(move |_| tx.emit(Input::GoBookmark(id.clone())));
            self.w.bm_list.append(&row);
        }
        let mut notes: Vec<&Highlight> = b.highlights.iter().filter(|h| pdf::is_pdf(h)).collect();
        notes.sort_by_key(|h| (h.spine, h.start));
        if notes.is_empty() {
            self.w.hl_list.append(&placeholder(
                &tr("No highlights or notes"),
                &tr("Select text on a page to highlight it"),
            ));
        }
        for h in notes {
            let sub = if h.note.is_empty() {
                glib::markup_escape_text(&h.text).to_string()
            } else {
                format!(
                    "{}\n<i>{}</i>",
                    glib::markup_escape_text(&h.text),
                    glib::markup_escape_text(&h.note)
                )
            };
            let row = adw::ActionRow::builder()
                .title(trf("Page {}", &[&page_name(h.spine)]))
                .subtitle(sub)
                .subtitle_lines(4)
                .activatable(true)
                .build();
            let del = icon_button("user-trash-symbolic", &tr("Delete Highlight"));
            del.add_css_class("flat");
            del.set_valign(gtk::Align::Center);
            let (tx, id) = (self.sender.input_sender().clone(), h.id.clone());
            del.connect_clicked(move |_| tx.emit(Input::DeleteHighlight(id.clone())));
            row.add_suffix(&del);
            let (tx, id) = (self.sender.input_sender().clone(), h.id.clone());
            row.connect_activated(move |_| tx.emit(Input::GoHighlight(id.clone())));
            self.w.hl_list.append(&row);
        }
    }

    fn selection_text(&self, sel: &Sel) -> String {
        let st = self.state.borrow();
        let Some(t) = st.texts.get(&sel.page) else {
            return String::new();
        };
        let raw = t.text_for(sel.start..sel.end);
        let mut out = String::new();
        for line in raw.split('\n') {
            bookx_pdf::reflow::join_lines(&mut out, line);
        }
        out
    }

    fn set_selection(&mut self, sel: Option<Sel>) {
        let info = sel.as_ref().map(|s| SelectionInfo {
            page: s.page,
            text: self.selection_text(s),
        });
        self.state.borrow_mut().selection = sel;
        self.selection = info.filter(|i| !i.text.trim().is_empty());
        self.w.area.queue_draw();
    }

    /// Opens the actions popover next to the selection.
    fn show_popover(&self) {
        let st = self.state.borrow();
        let Some(sel) = st.selection.clone() else {
            return;
        };
        let (Some(pl), Some(t)) = (st.place_of(sel.page), st.texts.get(&sel.page)) else {
            return;
        };
        let rects = t.rects_for(sel.start..sel.end);
        let Some(first) = rects.first() else { return };
        let r = st.to_px(pl, *first);
        let rr = rects.last().map(|r| st.to_px(pl, *r)).unwrap_or(r);
        drop(st);
        let (hv, vv) = (self.hadj.value() as f32, self.vadj.value() as f32);
        let (x0, x1) = (r[0].min(rr[0]) - hv, r[2].max(rr[2]) - hv);
        let (vw, vh) = self.view_size();
        let y0 = (r[1] - vv).clamp(0.0, vh - 4.0);
        let rect = gdk::Rectangle::new(
            x0.clamp(0.0, vw - 4.0) as i32,
            y0 as i32,
            ((x1 - x0).max(4.0)).min(vw) as i32,
            ((r[3] - r[1]).max(4.0)) as i32,
        );
        let editing = sel.highlight.clone();
        self.w.sel_remove.set_visible(editing.is_some());
        let note = editing
            .as_ref()
            .and_then(|id| {
                self.shared.lib.borrow().get(self.book_id).and_then(|b| {
                    b.highlights
                        .iter()
                        .find(|h| &h.id == id)
                        .map(|h| h.note.clone())
                })
            })
            .unwrap_or_default();
        self.w.note_preview.set_visible(!note.is_empty());
        self.w.note_label.set_label(&note);
        self.w.note_view.buffer().set_text(&note);
        self.w.sel_stack.set_visible_child_name("actions");
        self.w.sel_pop.set_autohide(false);
        self.w.sel_pop.set_pointing_to(Some(&rect));
        self.w.sel_pop.popup();
    }

    fn make_highlight(&mut self, color: &str, note: Option<String>) {
        let Some(sel) = self.state.borrow().selection.clone() else {
            return;
        };
        let (rects, text) = {
            let st = self.state.borrow();
            let Some(t) = st.texts.get(&sel.page) else {
                return;
            };
            (t.rects_for(sel.start..sel.end), self.selection_text(&sel))
        };
        let mut new_id = None;
        {
            let mut lib = self.shared.lib.borrow_mut();
            if let Some(b) = lib.get_mut(self.book_id) {
                if let Some(id) = &sel.highlight {
                    if let Some(h) = b.highlights.iter_mut().find(|h| &h.id == id) {
                        h.color = color.to_string();
                        if let Some(n) = &note {
                            h.note = n.trim().to_string();
                        }
                        h.style = pdf::pdf_style(self.hl_style.name(), &rects);
                    }
                } else {
                    let id = uuid::Uuid::new_v4().to_string();
                    new_id = Some(id.clone());
                    b.highlights.push(Highlight {
                        id,
                        spine: sel.page,
                        start: sel.start as i32,
                        end: sel.end as i32,
                        color: color.to_string(),
                        style: pdf::pdf_style(self.hl_style.name(), &rects),
                        note: note.unwrap_or_default().trim().to_string(),
                        text,
                        created: Utc::now(),
                    });
                }
            }
            let _ = lib.save();
        }
        if let Some(id) = new_id {
            if let Some(s) = self.state.borrow_mut().selection.as_mut() {
                s.highlight = Some(id);
            }
            self.w.sel_remove.set_visible(true);
        }
        self.refresh_highlights();
        self.rebuild_lists();
    }

    fn apply_prefs_to_ui(&self) {
        self.updating_ui.set(true);
        self.w.crop_sw.set_active(self.prefs.crop);
        self.w.invert_sw.set_active(self.prefs.invert);
        for (m, b) in &self.w.view_btns {
            if *m == self.prefs.view_mode() {
                b.set_active(true);
            }
        }
        self.updating_ui.set(false);
    }

    fn rerender_everything(&mut self) {
        for c in self.pending.values() {
            c.cancel();
        }
        self.pending.clear();
        {
            let mut st = self.state.borrow_mut();
            st.cache.clear();
            st.fallback.clear();
            st.invert = self.prefs.invert;
        }
        for c in self.thumb_pending.values() {
            c.cancel();
        }
        self.thumb_pending.clear();
        self.w.thumb_tex.borrow_mut().clear();
        self.w.thumbs.queue_draw();
        let pics: Vec<usize> = self.w.thumb_pics.borrow().keys().copied().collect();
        for p in pics {
            self.sender.input(Input::ThumbWant(p));
        }
        self.w.area.queue_draw();
        self.request_visible();
    }

    fn start_crop(&mut self) {
        self.crop_boxes.clear();
        let n = self.page_count();
        if n == 0 {
            return;
        }
        let step = (n / CROP_SAMPLES).max(1);
        let pages: Vec<usize> = (0..n).step_by(step).take(CROP_SAMPLES).collect();
        self.crop_wait = pages.len();
        if let Some(w) = &self.worker {
            for p in pages {
                w.crop(p);
            }
        }
    }

    fn apply_crop(&mut self) {
        let boxes = std::mem::take(&mut self.crop_boxes);
        let crop = if boxes.is_empty() {
            None
        } else {
            let mut u = boxes[0];
            for b in &boxes {
                u = [
                    u[0].min(b[0]),
                    u[1].min(b[1]),
                    u[2].max(b[2]),
                    u[3].max(b[3]),
                ];
            }
            // a little air round the text
            let pad = 0.012;
            let c = [
                (u[0] - pad).max(0.0),
                (u[1] - pad).max(0.0),
                (u[2] + pad).min(1.0),
                (u[3] + pad).min(1.0),
            ];
            // nothing worth cutting
            if c[0] < 0.02 && c[1] < 0.02 && c[2] > 0.98 && c[3] > 0.98 {
                None
            } else {
                Some(c)
            }
        };
        self.state.borrow_mut().layout.crop = crop;
        self.relayout();
    }

    fn handle_event(&mut self, ev: Event, sender: &ComponentSender<Self>) {
        match ev {
            Event::Opened(Err(e)) => match e {
                OpenError::NeedsPassword | OpenError::WrongPassword => {
                    self.worker = None;
                    let wrong = e == OpenError::WrongPassword;
                    self.ask_password(wrong);
                }
                other => {
                    self.worker = None;
                    self.w.content.set_visible_child_name("error");
                    let _ = sender.output(Output::Toast(trf("Cannot open the PDF: {}", &[&other])));
                }
            },
            Event::Opened(Ok(s)) => {
                self.summary = Some(Rc::new(s));
                self.on_opened(sender);
            }
            Event::Rendered {
                page,
                scale,
                invert,
                bitmap,
            } => {
                let key = pdf::render_key(page, scale, invert);
                self.pending.remove(&key);
                if self.thumb_pending.contains_key(&page)
                    && (scale - THUMB_SCALE).abs() < 1e-3
                    && invert == self.prefs.invert
                {
                    self.thumb_pending.remove(&page);
                    let tex = texture_of(&bitmap);
                    if let Some(pic) = self
                        .w
                        .thumb_pics
                        .borrow()
                        .get(&page)
                        .and_then(|p| p.upgrade())
                    {
                        pic.set_paintable(Some(&tex));
                    }
                    self.w.thumb_tex.borrow_mut().insert(page, tex);
                    return;
                }
                if let Some(surface) = surface_of(&bitmap) {
                    let surf = Rc::new(Surf { surface, scale });
                    let mut st = self.state.borrow_mut();
                    if invert == st.invert {
                        st.cache.insert(key, surf.clone(), bitmap.bytes());
                        st.fallback.insert(page, surf);
                    }
                }
                self.w.area.queue_draw();
            }
            Event::RenderFailed { page } => {
                self.pending.retain(|k, _| k.0 != page);
                self.thumb_pending.remove(&page);
            }
            Event::Page { page, text, links } => {
                self.text_pending.remove(&page);
                {
                    let mut st = self.state.borrow_mut();
                    st.texts.insert(page, text);
                    st.links.insert(page, links);
                }
                self.w.area.queue_draw();
            }
            Event::SearchHits { id, page, hits } => {
                if id != self.search_id {
                    return;
                }
                self.add_hits(page, hits);
            }
            Event::SearchDone { id } => {
                if id == self.search_id {
                    self.search_done = true;
                    self.update_search_status();
                }
            }
            Event::Crop { bbox, .. } => {
                if let Some(b) = bbox {
                    self.crop_boxes.push(b);
                }
                self.crop_wait = self.crop_wait.saturating_sub(1);
                if self.crop_wait == 0 && self.prefs.crop {
                    self.apply_crop();
                }
            }
        }
    }

    fn ask_password(&mut self, wrong: bool) {
        if self.asked_password && !wrong {
            return;
        }
        self.asked_password = true;
        let dialog = adw::AlertDialog::new(
            Some(&tr("Password needed")),
            Some(
                &(if wrong {
                    tr("That password did not open the PDF. Try again.")
                } else {
                    tr("This PDF is protected. Enter its password to read it.")
                }),
            ),
        );
        let entry = gtk::PasswordEntry::new();
        entry.set_show_peek_icon(true);
        entry.set_activates_default(true);
        super::audit::label(&entry, &tr("PDF password"));
        dialog.set_extra_child(Some(&entry));
        dialog.add_response("cancel", &tr("Cancel"));
        dialog.add_response("open", &tr("Open"));
        dialog.set_response_appearance("open", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("open"));
        dialog.set_close_response("cancel");
        let tx = self.sender.input_sender().clone();
        let e = entry.clone();
        dialog.connect_response(None, move |_, r| {
            if r == "open" {
                tx.emit(Input::Password(Some(e.text().to_string())));
            } else {
                tx.emit(Input::Password(None));
            }
        });
        let page = self.w.page.clone();
        let e = entry.clone();
        glib::idle_add_local_once(move || {
            dialog.present(Some(&page));
            e.grab_focus();
        });
    }

    fn on_opened(&mut self, sender: &ComponentSender<Self>) {
        let s = self.summary.clone().unwrap();
        let n = s.page_sizes.len();
        {
            let mut st = self.state.borrow_mut();
            st.layout = {
                let mut l = Layout::new(s.page_sizes.clone());
                l.zoom = self.prefs.zoom_mode();
                l.mode = self.prefs.view_mode();
                l
            };
            st.invert = self.prefs.invert;
        }
        self.w.slider.set_range(0.0, (n.saturating_sub(1)) as f64);
        self.w.slider.set_increments(1.0, 10.0);
        self.w.content.set_visible_child_name("view");
        if let Some(t) = Some(s.info.title.trim()).filter(|t| !t.is_empty()) {
            self.w.title.set_title(t);
        }
        // thumbnails
        let model = gtk::StringList::new(&[]);
        let labels = &s.labels;
        for i in 0..n {
            model.append(labels.get(i).map(|l| l.as_str()).unwrap_or(""));
        }
        self.w
            .thumbs
            .set_model(Some(&gtk::SingleSelection::new(Some(model))));
        self.rebuild_lists();
        self.refresh_highlights();
        self.relayout();
        if self.prefs.crop {
            self.start_crop();
        }
        // where the reader left off
        let pos = self
            .shared
            .lib
            .borrow()
            .get(self.book_id)
            .and_then(|b| b.position);
        let (page, frac) = pos
            .map(|p| (p.spine.min(n - 1), p.frac))
            .unwrap_or((0, 0.0));
        self.opened = true;
        self.goto_page(page, None);
        if frac > 0.0 {
            let st = self.state.borrow();
            if let Some(p) = st.place_of(page) {
                let (_, vh) = self.view_size();
                let y = if st.layout.mode == ViewMode::Continuous {
                    p.y + p.h * frac as f32 - vh * 0.35
                } else {
                    p.h * frac as f32
                };
                drop(st);
                self.vadj
                    .set_value((y as f64).clamp(0.0, (self.vadj.upper() - vh as f64).max(0.0)));
            }
        }
        self.update_position_ui();
        self.w.area.grab_focus();
        let _ = sender;
    }

    fn add_hits(&mut self, page: usize, hits: Vec<Hit>) {
        let labels = self.labels();
        let label = labels
            .get(page)
            .cloned()
            .unwrap_or_else(|| (page + 1).to_string());
        for h in hits {
            if self.search_rows.len() >= 2000 {
                break;
            }
            let idx = self.search_rows.len();
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&format!("Page {label}")))
                .activatable(true)
                .build();
            let l = gtk::Label::new(None);
            l.set_wrap(true);
            l.set_xalign(0.0);
            l.add_css_class("caption");
            let chars: Vec<char> = h.snippet.chars().collect();
            let a: String = chars[..h.mark.start.min(chars.len())].iter().collect();
            let m: String = chars[h.mark.start.min(chars.len())..h.mark.end.min(chars.len())]
                .iter()
                .collect();
            let z: String = chars[h.mark.end.min(chars.len())..].iter().collect();
            l.set_markup(&format!(
                "{}<b>{}</b>{}",
                glib::markup_escape_text(&a),
                glib::markup_escape_text(&m),
                glib::markup_escape_text(&z)
            ));
            row.set_subtitle("");
            row.add_suffix(&gtk::Label::new(None));
            let bx = gtk::Box::new(gtk::Orientation::Vertical, 2);
            bx.append(&l);
            row.set_title(&trf("Page {}", &[&label]));
            row.add_prefix(&bx);
            let tx = self.sender.input_sender().clone();
            row.connect_activated(move |_| tx.emit(Input::SearchResult(idx)));
            self.w.search_list.append(&row);
            self.search_rows.push(SearchRow {
                page,
                range: h.range.clone(),
                snippet: h.snippet,
                mark: h.mark,
            });
            self.state.borrow_mut().hits.push((page, h.range));
        }
        self.update_search_status();
        self.w.area.queue_draw();
        if self.search_active.is_none() && !self.search_rows.is_empty() && self.search_done_first()
        {
            self.activate_hit(0, false);
        }
    }

    fn search_done_first(&self) -> bool {
        true
    }

    fn update_search_status(&self) {
        let n = self.search_rows.len();
        let t = if self.search_id == 0 {
            String::new()
        } else if self.search_done {
            super::reader_search::search_status(n, true)
        } else {
            trf("Searching, {} so far", &[&n])
        };
        self.w.search_status.set_label(&t);
        super::reader_search::set_empty_state(
            &self.w.search_list,
            &self.w.search_empty,
            self.search_id != 0 && self.search_done && n == 0,
        );
    }

    fn activate_hit(&mut self, idx: usize, close: bool) {
        let Some(row) = self.search_rows.get(idx) else {
            return;
        };
        let (page, range) = (row.page, row.range.clone());
        self.search_active = Some(idx);
        self.state.borrow_mut().active_hit = Some(idx);
        // move to the line of the hit when its text is known, else the top of the page
        let y = self
            .state
            .borrow()
            .texts
            .get(&page)
            .and_then(|t| t.rects_for(range.clone()).first().map(|r| r[1]));
        self.goto_page(page, y);
        if close {
            self.w.search_btn.popdown();
        }
        self.w.area.queue_draw();
    }

    fn open_reflow(&self) {
        let _ = self.sender.output(Output::Reflow);
    }

    fn activate_link(&mut self, link: &Link) {
        match &link.target {
            LinkTarget::Page(d) => {
                self.history.push((self.current_page(), None));
                self.w.back_btn.set_visible(true);
                self.goto_page(d.page, d.y);
            }
            LinkTarget::Uri(u) => {
                if !open_external_link(u) {
                    let _ = self
                        .sender
                        .output(Output::Toast(tr("That link is not an address Bookx opens")));
                }
            }
        }
    }

    /// The link under a document position, with its page.
    fn link_at(&self, x: f32, y: f32) -> Option<Link> {
        let st = self.state.borrow();
        let (page, px, py) = st.page_at(x, y, false)?;
        st.links
            .get(&page)?
            .iter()
            .find(|l| px >= l.rect[0] && px <= l.rect[2] && py >= l.rect[1] && py <= l.rect[3])
            .cloned()
    }

    fn highlight_at(&self, x: f32, y: f32) -> Option<(String, usize)> {
        let st = self.state.borrow();
        let (page, px, py) = st.page_at(x, y, false)?;
        st.highlights
            .iter()
            .filter(|h| h.page == page)
            .find(|h| {
                h.rects
                    .iter()
                    .any(|r| px >= r[0] && px <= r[2] && py >= r[1] && py <= r[3])
            })
            .map(|h| (h.id.clone(), page))
    }

    fn select_stored(&mut self, id: &str) {
        let (page, start, end) = {
            let lib = self.shared.lib.borrow();
            let Some(h) = lib
                .get(self.book_id)
                .and_then(|b| b.highlights.iter().find(|h| h.id == id))
            else {
                return;
            };
            (h.spine, h.start.max(0) as usize, h.end.max(0) as usize)
        };
        let range = {
            let st = self.state.borrow();
            let quote = self
                .shared
                .lib
                .borrow()
                .get(self.book_id)
                .and_then(|b| {
                    b.highlights
                        .iter()
                        .find(|h| h.id == id)
                        .map(|h| h.text.clone())
                })
                .unwrap_or_default();
            st.texts
                .get(&page)
                .and_then(|t| t.anchor(start, end, &quote))
                .unwrap_or(start..end)
        };
        self.set_selection(Some(Sel {
            page,
            start: range.start,
            end: range.end,
            highlight: Some(id.to_string()),
        }));
        // an annotation whose text could not be found again still shows its popover
        self.selection = Some(SelectionInfo {
            page,
            text: self
                .shared
                .lib
                .borrow()
                .get(self.book_id)
                .and_then(|b| {
                    b.highlights
                        .iter()
                        .find(|h| h.id == id)
                        .map(|h| h.text.clone())
                })
                .unwrap_or_default(),
        });
        self.show_popover();
    }

    fn doc_pos(&self, x: f64, y: f64) -> (f32, f32) {
        (
            (x + self.hadj.value()) as f32,
            (y + self.vadj.value()) as f32,
        )
    }

    fn properties(&self) {
        let Some(s) = &self.summary else { return };
        let mut lines = vec![format!("Pages: {}", s.page_sizes.len())];
        for (k, v) in [
            ("Title", &s.info.title),
            ("Author", &s.info.author),
            ("Subject", &s.info.subject),
            ("Keywords", &s.info.keywords),
            ("Created with", &s.info.creator),
            ("Produced by", &s.info.producer),
        ] {
            if !v.trim().is_empty() {
                lines.push(format!("{k}: {v}"));
            }
        }
        lines.push(format!(
            "Size: {}",
            crate::library_health::size_text(self.data.len() as u64)
        ));
        let d = adw::AlertDialog::new(Some(&tr("PDF properties")), Some(&lines.join("\n")));
        d.add_response("ok", &tr("Close"));
        d.present(Some(&self.w.page));
    }
}

fn clear_list(l: &gtk::ListBox) {
    while let Some(c) = l.first_child() {
        l.remove(&c);
    }
}

fn placeholder(title: &str, sub: &str) -> adw::ActionRow {
    let r = adw::ActionRow::builder()
        .title(title)
        .subtitle(sub)
        .activatable(false)
        .selectable(false)
        .build();
    r.add_css_class("dim-label");
    r
}

fn draw_page_view(st: &ViewState, hv: f64, vv: f64, cr: &cairo::Context, w: i32, h: i32) {
    let bg = if st.dark || st.invert {
        (0.13, 0.13, 0.14)
    } else {
        (0.87, 0.87, 0.88)
    };
    cr.set_source_rgb(bg.0, bg.1, bg.2);
    let _ = cr.paint();
    let vis = pdf::visible(&st.places, vv as f32, h as f32);
    for idx in vis {
        let pl = st.places[idx];
        let (x, y) = (pl.x as f64 - hv, pl.y as f64 - vv);
        // paper
        if st.invert {
            cr.set_source_rgb(0.0, 0.0, 0.0);
        } else {
            cr.set_source_rgb(1.0, 1.0, 1.0);
        }
        cr.rectangle(x, y, pl.w as f64, pl.h as f64);
        let _ = cr.fill();
        let dpr_key = st.cache.contains(&pdf::render_key(pl.page, 0.0, st.invert));
        let _ = dpr_key;
        // the rendering: the exact one, else the last one scaled
        let surf = st.fallback.get(&pl.page).cloned();
        if let Some(s) = surf {
            let (sw, sh) = st.size_pt(pl.page);
            let _ = (sw, sh);
            let k = pl.scale as f64 / s.scale as f64;
            let (ox, oy) = match st.layout.crop {
                Some(c) => (
                    c[0] as f64 * st.size_pt(pl.page).0 as f64,
                    c[1] as f64 * st.size_pt(pl.page).1 as f64,
                ),
                None => (0.0, 0.0),
            };
            let _ = cr.save();
            cr.rectangle(x, y, pl.w as f64, pl.h as f64);
            cr.clip();
            cr.translate(x - ox * pl.scale as f64, y - oy * pl.scale as f64);
            cr.scale(k, k);
            let _ = cr.set_source_surface(&s.surface, 0.0, 0.0);
            let filter = if (k - 1.0).abs() < 0.01 {
                cairo::Filter::Nearest
            } else {
                cairo::Filter::Good
            };
            cr.source().set_filter(filter);
            let _ = cr.paint();
            let _ = cr.restore();
        }
        // highlights
        for hl in st.highlights.iter().filter(|h| h.page == pl.page) {
            for r in &hl.rects {
                let p = st.to_px(&pl, *r);
                let (rx, ry, rw, rh) = (
                    p[0] as f64 - hv,
                    p[1] as f64 - vv,
                    (p[2] - p[0]) as f64,
                    (p[3] - p[1]) as f64,
                );
                let _ = cr.save();
                cr.set_source_rgba(hl.color.0, hl.color.1, hl.color.2, 0.9);
                match hl.style {
                    HighlightStyle::Background => {
                        cr.set_operator(cairo::Operator::Multiply);
                        cr.set_source_rgb(hl.color.0, hl.color.1, hl.color.2);
                        cr.rectangle(rx, ry, rw, rh);
                        let _ = cr.fill();
                    }
                    HighlightStyle::Invert => {
                        cr.set_operator(cairo::Operator::Difference);
                        cr.set_source_rgb(1.0, 1.0, 1.0);
                        cr.rectangle(rx, ry, rw, rh);
                        let _ = cr.fill();
                    }
                    HighlightStyle::Underline => {
                        cr.set_line_width(2.0);
                        cr.move_to(rx, ry + rh - 1.0);
                        cr.line_to(rx + rw, ry + rh - 1.0);
                        let _ = cr.stroke();
                    }
                    HighlightStyle::Strikethrough => {
                        cr.set_line_width(1.5);
                        cr.move_to(rx, ry + rh * 0.55);
                        cr.line_to(rx + rw, ry + rh * 0.55);
                        let _ = cr.stroke();
                    }
                    HighlightStyle::Squiggly => {
                        cr.set_line_width(1.2);
                        let base = ry + rh - 1.5;
                        let mut xx = rx;
                        cr.move_to(xx, base);
                        let mut up = true;
                        while xx < rx + rw {
                            xx += 2.5;
                            cr.line_to(xx.min(rx + rw), if up { base - 2.0 } else { base });
                            up = !up;
                        }
                        let _ = cr.stroke();
                    }
                }
                let _ = cr.restore();
            }
            if hl.has_note {
                if let Some(r) = hl.rects.first() {
                    let p = st.to_px(&pl, *r);
                    cr.set_source_rgba(0.1, 0.1, 0.1, 0.8);
                    cr.arc(
                        p[0] as f64 - hv - 3.0,
                        p[1] as f64 - vv + 3.0,
                        3.0,
                        0.0,
                        std::f64::consts::TAU,
                    );
                    let _ = cr.fill();
                }
            }
        }
        // search hits
        if let Some(t) = st.texts.get(&pl.page) {
            for (i, (pg, range)) in st.hits.iter().enumerate() {
                if *pg != pl.page {
                    continue;
                }
                let active = st.active_hit == Some(i);
                for r in t.rects_for(range.clone()) {
                    let p = st.to_px(&pl, r);
                    let _ = cr.save();
                    cr.set_operator(cairo::Operator::Multiply);
                    if active {
                        cr.set_source_rgba(1.0, 0.55, 0.1, 1.0);
                    } else {
                        cr.set_source_rgba(1.0, 0.85, 0.3, 1.0);
                    }
                    cr.rectangle(
                        p[0] as f64 - hv,
                        p[1] as f64 - vv,
                        (p[2] - p[0]) as f64,
                        (p[3] - p[1]) as f64,
                    );
                    let _ = cr.fill();
                    let _ = cr.restore();
                }
            }
            if let Some(sel) = st.selection.as_ref().filter(|s| s.page == pl.page) {
                for r in t.rects_for(sel.start..sel.end) {
                    let p = st.to_px(&pl, r);
                    cr.set_source_rgba(0.21, 0.52, 0.89, 0.38);
                    cr.rectangle(
                        p[0] as f64 - hv,
                        p[1] as f64 - vv,
                        (p[2] - p[0]) as f64,
                        (p[3] - p[1]) as f64,
                    );
                    let _ = cr.fill();
                }
            }
        }
        let _ = w;
    }
}

impl SimpleComponent for ReaderPdf {
    /// The library, the book id, the file and the title to show.
    type Init = (SharedRef, u64, Vec<u8>);
    type Input = Input;
    type Output = Output;
    type Root = adw::NavigationPage;
    type Widgets = ();

    fn init_root() -> Self::Root {
        adw::NavigationPage::builder()
            .title(tr("Reader"))
            .tag("reader")
            .build()
    }

    fn init(
        (shared, book_id, bytes): Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let (title_text, language) = {
            let lib = shared.lib.borrow();
            lib.get(book_id)
                .map(|b| (b.meta.title.clone(), b.meta.language.clone()))
                .unwrap_or_default()
        };
        let prefs = PdfPrefs::load();
        let data = Arc::new(bytes);

        // ---- the page area ----------------------------------------------------------
        let area = gtk::DrawingArea::new();
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.set_focusable(true);
        area.set_can_focus(true);
        area.add_css_class("pdf-pages");
        super::audit::label(&area, &trf("{}, PDF pages", &[&title_text]));
        area.update_property(&[gtk::accessible::Property::Description(
            "Page Down and Page Up move through the pages, plus and minus zoom",
        )]);
        let hadj = gtk::Adjustment::new(0.0, 0.0, 1.0, 32.0, 100.0, 1.0);
        let vadj = gtk::Adjustment::new(0.0, 0.0, 1.0, 48.0, 100.0, 1.0);
        let state = Rc::new(RefCell::new(ViewState {
            layout: Layout::new(Vec::new()),
            places: Vec::new(),
            current: 0,
            cache: PageCache::new(CACHE_BYTES),
            fallback: HashMap::new(),
            texts: HashMap::new(),
            links: HashMap::new(),
            highlights: Vec::new(),
            selection: None,
            hits: Vec::new(),
            active_hit: None,
            invert: prefs.invert,
            dark: adw::StyleManager::default().is_dark(),
        }));
        {
            let (st, ha, va) = (state.clone(), hadj.clone(), vadj.clone());
            area.set_draw_func(move |_, cr, w, h| {
                draw_page_view(&st.borrow(), ha.value(), va.value(), cr, w, h);
            });
            let tx = sender.input_sender().clone();
            vadj.connect_value_changed(move |_| tx.emit(Input::Scrolled));
            let tx = sender.input_sender().clone();
            hadj.connect_value_changed(move |_| tx.emit(Input::Scrolled));
            let tx = sender.input_sender().clone();
            area.connect_resize(move |_, _, _| tx.emit(Input::Resize));
            let st = state.clone();
            adw::StyleManager::default().connect_dark_notify(move |m| {
                st.borrow_mut().dark = m.is_dark();
            });
        }
        let vbar = gtk::Scrollbar::new(gtk::Orientation::Vertical, Some(&vadj));
        let hbar = gtk::Scrollbar::new(gtk::Orientation::Horizontal, Some(&hadj));
        hbar.set_visible(false);
        super::audit::label(&vbar, &tr("Scroll the pages"));
        super::audit::label(&hbar, &tr("Scroll sideways"));

        // pointer
        {
            let drag = gtk::GestureDrag::new();
            drag.set_button(1);
            let tx = sender.input_sender().clone();
            drag.connect_drag_begin(move |_, x, y| tx.emit(Input::Press(x, y)));
            let tx = sender.input_sender().clone();
            drag.connect_drag_update(move |_, dx, dy| tx.emit(Input::Drag(dx, dy)));
            let tx = sender.input_sender().clone();
            drag.connect_drag_end(move |g, dx, dy| {
                let (sx, sy) = g.start_point().unwrap_or((0.0, 0.0));
                tx.emit(Input::DragEnd(sx + dx, sy + dy));
            });
            area.add_controller(drag);

            let click = gtk::GestureClick::new();
            click.set_button(1);
            let tx = sender.input_sender().clone();
            click.connect_released(move |_, n, x, y| tx.emit(Input::Click(x, y, n)));
            area.add_controller(click);

            let motion = gtk::EventControllerMotion::new();
            let tx = sender.input_sender().clone();
            motion.connect_motion(move |_, x, y| tx.emit(Input::Hover(x, y)));
            area.add_controller(motion);

            let scroll = gtk::EventControllerScroll::new(
                gtk::EventControllerScrollFlags::BOTH_AXES
                    | gtk::EventControllerScrollFlags::KINETIC,
            );
            let (va, ha, tx) = (vadj.clone(), hadj.clone(), sender.input_sender().clone());
            scroll.connect_scroll(move |c, dx, dy| {
                let ctrl = c
                    .current_event_state()
                    .contains(gdk::ModifierType::CONTROL_MASK);
                if ctrl {
                    tx.emit(Input::ZoomStep(dy < 0.0));
                    return glib::Propagation::Stop;
                }
                let shift = c
                    .current_event_state()
                    .contains(gdk::ModifierType::SHIFT_MASK);
                if shift {
                    ha.set_value(ha.value() + dy * 40.0);
                } else {
                    ha.set_value(ha.value() + dx * 40.0);
                    let before = va.value();
                    va.set_value(before + dy * 48.0);
                    // single page mode: scrolling past the edge turns the page
                    if dy.abs() > 0.0 {
                        let max = (va.upper() - va.page_size()).max(0.0);
                        if dy > 0.0 && before >= max - 0.5 {
                            tx.emit(Input::Turn(true));
                        } else if dy < 0.0 && before <= 0.5 {
                            tx.emit(Input::Turn(false));
                        }
                    }
                }
                glib::Propagation::Stop
            });
            area.add_controller(scroll);

            let zoom = gtk::GestureZoom::new();
            let base = Rc::new(Cell::new(1.0f64));
            let tx = sender.input_sender().clone();
            let b = base.clone();
            zoom.connect_begin(move |_, _| b.set(1.0));
            let b = base.clone();
            zoom.connect_scale_changed(move |_, s| {
                let f = s / b.get();
                if !(0.97..1.03).contains(&f) {
                    b.set(s);
                    tx.emit(Input::ZoomBy(f));
                }
            });
            area.add_controller(zoom);

            // touch: dragging moves the page (a mouse selects)
            let pan = gtk::GestureDrag::new();
            pan.set_touch_only(true);
            let (va, ha) = (vadj.clone(), hadj.clone());
            let start = Rc::new(Cell::new((0.0, 0.0)));
            let s = start.clone();
            let (va2, ha2) = (va.clone(), ha.clone());
            pan.connect_drag_begin(move |_, _, _| s.set((ha2.value(), va2.value())));
            let s = start.clone();
            pan.connect_drag_update(move |_, dx, dy| {
                let (h0, v0) = s.get();
                ha.set_value(h0 - dx);
                va.set_value(v0 - dy);
            });
            area.add_controller(pan);
        }

        // ---- selection popover ---------------------------------------------------------
        let sel_pop = gtk::Popover::new();
        sel_pop.set_autohide(false);
        sel_pop.set_has_arrow(true);
        sel_pop.set_parent(&area);
        sel_pop.add_css_class("selection-popover");
        let switching = Rc::new(Cell::new(false));
        let sel_stack = gtk::Stack::new();
        sel_stack.set_hhomogeneous(false);
        sel_stack.set_vhomogeneous(false);
        // swatches and actions wrap instead of making the popover wider than a narrow window
        let swatch_row = adw::WrapBox::new();
        swatch_row.set_child_spacing(8);
        swatch_row.set_line_spacing(4);
        swatch_row.set_natural_line_length(340);
        let actions = adw::WrapBox::new();
        actions.set_child_spacing(4);
        actions.set_line_spacing(4);
        actions.set_natural_line_length(340);
        let mut swatches = Vec::new();
        for (name, _) in HIGHLIGHT_COLORS {
            let b = gtk::Button::new();
            b.add_css_class("hl-swatch");
            b.add_css_class(&format!("hl-{name}"));
            b.set_size_request(32, 32);
            b.set_halign(gtk::Align::Center);
            b.set_valign(gtk::Align::Center);
            super::audit::label(&b, &trf("Highlight {}", &[&side_notes::colour_label(name)]));
            b.set_tooltip_text(Some(&trf(
                "Highlight {}",
                &[&side_notes::colour_label(name)],
            )));
            let tx = sender.input_sender().clone();
            b.connect_clicked(move |_| tx.emit(Input::Highlight(name.to_string())));
            swatch_row.append(&b);
            swatches.push(b);
        }
        let note_btn = icon_button("document-edit-symbolic", &tr("Add Note"));
        let copy_btn = icon_button("edit-copy-symbolic", &tr("Copy"));
        let dict_btn = icon_button(
            "accessories-dictionary-symbolic",
            &tr("Look Up in Dictionary"),
        );
        let wiki_btn = icon_button("wikipedia-symbolic", &tr("Look Up on Wikipedia"));
        let tr_btn = icon_button("translate-symbolic", &tr("Translate"));
        let sel_remove = icon_button("user-trash-symbolic", &tr("Remove Highlight"));
        for b in [
            &note_btn,
            &copy_btn,
            &dict_btn,
            &wiki_btn,
            &tr_btn,
            &sel_remove,
        ] {
            b.add_css_class("flat");
            actions.append(b);
        }
        sel_remove.set_visible(false);
        {
            let tx = sender.input_sender().clone();
            copy_btn.connect_clicked(move |_| tx.emit(Input::Copy));
            let tx = sender.input_sender().clone();
            wiki_btn.connect_clicked(move |_| tx.emit(Input::Wiki));
            let tx = sender.input_sender().clone();
            dict_btn.connect_clicked(move |_| tx.emit(Input::Dict));
            let tx = sender.input_sender().clone();
            tr_btn.connect_clicked(move |_| tx.emit(Input::Translate));
            let tx = sender.input_sender().clone();
            sel_remove.connect_clicked(move |_| tx.emit(Input::RemoveHighlight));
        }
        let note_preview = gtk::Button::new();
        note_preview.add_css_class("flat");
        note_preview.add_css_class("note-preview");
        note_preview.set_visible(false);
        note_preview.set_tooltip_text(Some(&tr("Edit Note")));
        let note_label = gtk::Label::new(None);
        note_label.set_wrap(true);
        note_label.set_max_width_chars(42);
        note_label.set_xalign(0.0);
        note_preview.set_child(Some(&note_label));
        {
            let b = note_btn.clone();
            note_preview.connect_clicked(move |_| b.emit_clicked());
        }
        let style_row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        style_row.set_halign(gtk::Align::Start);
        let mut style_btns = Vec::new();
        for (st, markup, tip) in [
            (
                HighlightStyle::Background,
                "<span background=\"#f6d32d\" foreground=\"#241f31\"> A </span>",
                N_("Highlight with a Background"),
            ),
            (HighlightStyle::Underline, "<u>A</u>", N_("Underline")),
            (
                HighlightStyle::Strikethrough,
                "<s>A</s>",
                N_("Strike Through"),
            ),
            (
                HighlightStyle::Squiggly,
                "<span underline=\"error\">A</span>",
                N_("Squiggly Underline"),
            ),
        ] {
            let b = gtk::ToggleButton::new();
            let l = gtk::Label::new(None);
            l.set_markup(markup);
            b.set_child(Some(&l));
            b.add_css_class("flat");
            super::audit::label(&b, &tr(tip));
            b.set_tooltip_text(Some(&tr(tip)));
            b.set_active(st == HighlightStyle::Background);
            let tx = sender.input_sender().clone();
            b.connect_clicked(move |_| tx.emit(Input::HlStyle(st)));
            style_row.append(&b);
            style_btns.push((st, b));
        }
        let actions_page = gtk::Box::new(gtk::Orientation::Vertical, 4);
        actions_page.append(&note_preview);
        actions_page.append(&swatch_row);
        actions_page.append(&actions);
        actions_page.append(&style_row);
        sel_stack.add_named(&actions_page, Some("actions"));
        let note_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        note_box.set_size_request(280, -1);
        let note_view = gtk::TextView::new();
        super::audit::label(&note_view, &tr("Note"));
        note_view.set_wrap_mode(gtk::WrapMode::WordChar);
        note_view.set_top_margin(6);
        note_view.set_left_margin(6);
        note_view.set_right_margin(6);
        let note_scroll = gtk::ScrolledWindow::new();
        note_scroll.set_min_content_height(80);
        note_scroll.set_child(Some(&note_view));
        note_scroll.add_css_class("card");
        let note_actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let note_back = icon_button("go-previous-symbolic", &tr("Back to Actions"));
        note_back.add_css_class("flat");
        let note_save = gtk::Button::with_label(&tr("Save Note"));
        note_save.add_css_class("suggested-action");
        note_save.set_hexpand(true);
        note_actions.append(&note_back);
        note_actions.append(&note_save);
        note_box.append(&note_scroll);
        note_box.append(&note_actions);
        sel_stack.add_named(&note_box, Some("note"));
        {
            let (s, pop, nv, sw) = (
                sel_stack.clone(),
                sel_pop.clone(),
                note_view.clone(),
                switching.clone(),
            );
            note_btn.connect_clicked(move |_| {
                // the note box needs the keyboard: a modal popover, re-opened for it
                // closed and opened again a moment later, not in the same breath: a popup that is re-shown at
                // once reuses a Wayland surface that still has its buffer (protocol error 71, see reader.rs `repopup`)
                sw.set(true);
                pop.popdown();
                pop.set_autohide(true);
                s.set_visible_child_name("note");
                let (pop, sw, nv) = (pop.clone(), sw.clone(), nv.clone());
                glib::timeout_add_local_once(std::time::Duration::from_millis(80), move || {
                    pop.popup();
                    sw.set(false);
                    nv.grab_focus();
                });
            });
            let (s, pop, sw) = (sel_stack.clone(), sel_pop.clone(), switching.clone());
            note_back.connect_clicked(move |_| {
                sw.set(true);
                pop.popdown();
                pop.set_autohide(false);
                s.set_visible_child_name("actions");
                let (pop, sw) = (pop.clone(), sw.clone());
                glib::timeout_add_local_once(std::time::Duration::from_millis(80), move || {
                    pop.popup();
                    sw.set(false);
                });
            });
            let (tx, sw) = (sender.input_sender().clone(), switching.clone());
            sel_pop.connect_closed(move |_| {
                if !sw.get() {
                    tx.emit(Input::PopClosed);
                }
            });
            let tx = sender.input_sender().clone();
            let nv = note_view.clone();
            note_save.connect_clicked(move |_| {
                let b = nv.buffer();
                let (s, e) = b.bounds();
                tx.emit(Input::SaveNote(b.text(&s, &e, false).to_string()));
            });
            note_view.set_accepts_tab(false);
        }
        let lookup_page = |title: &str| {
            let page = gtk::Box::new(gtk::Orientation::Vertical, 8);
            page.set_size_request(340, -1);
            let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let back = icon_button("go-previous-symbolic", &tr("Back to Actions"));
            back.add_css_class("flat");
            let s = sel_stack.clone();
            back.connect_clicked(move |_| s.set_visible_child_name("actions"));
            head.append(&back);
            let t = gtk::Label::new(Some(title));
            t.add_css_class("heading");
            t.set_hexpand(true);
            t.set_xalign(0.0);
            head.append(&t);
            page.append(&head);
            let spin = gtk::Spinner::new();
            spin.set_opacity(0.0);
            spin.set_size_request(-1, 20);
            let text = gtk::Label::new(None);
            text.set_wrap(true);
            text.set_xalign(0.0);
            text.set_max_width_chars(44);
            text.set_margin_start(8);
            text.set_margin_end(8);
            let scroll = gtk::ScrolledWindow::new();
            scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
            scroll.set_propagate_natural_height(false);
            scroll.set_min_content_height(170);
            scroll.set_max_content_height(170);
            scroll.set_child(Some(&text));
            page.append(&spin);
            page.append(&scroll);
            (page, head, spin, text)
        };
        let (dict_page, _h, _s, dict_text) = lookup_page(&tr("Dictionary"));
        dict_text.set_use_markup(true);
        sel_stack.add_named(&dict_page, Some("dict"));
        let (wiki_page, _h, wiki_spin, wiki_text) = lookup_page(&tr("Wikipedia"));
        let wiki_url = Rc::new(RefCell::new(String::new()));
        {
            let foot = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            foot.set_halign(gtk::Align::End);
            let open = gtk::Button::with_label(&tr("Open Wikipedia"));
            open.add_css_class("flat");
            open.add_css_class("caption");
            let u = wiki_url.clone();
            open.connect_clicked(move |_| open_uri(&u.borrow()));
            foot.append(&open);
            wiki_page.append(&foot);
        }
        sel_stack.add_named(&wiki_page, Some("wiki"));
        let (tr_page, tr_head, tr_spin, tr_text) = lookup_page(&tr("Translate"));
        let tr_dd = {
            let names: Vec<&str> = web::LANGUAGES.iter().map(|(_, n)| *n).collect();
            let dd = gtk::DropDown::from_strings(&names);
            super::audit::label(&dd, &tr("Translate to"));
            let cur = shared.cfg.borrow().reader.translate_to.clone();
            dd.set_selected(
                web::LANGUAGES
                    .iter()
                    .position(|(c, _)| *c == cur)
                    .unwrap_or(0) as u32,
            );
            let tx = sender.input_sender().clone();
            dd.connect_selected_notify(move |d| {
                if let Some((c, _)) = web::LANGUAGES.get(d.selected() as usize) {
                    tx.emit(Input::TranslateLang(c.to_string()));
                }
            });
            tr_head.append(&dd);
            dd
        };
        sel_stack.add_named(&tr_page, Some("translate"));
        let shield = gtk::EventControllerLegacy::new();
        shield.set_propagation_phase(gtk::PropagationPhase::Bubble);
        shield.connect_event(|_, e| {
            use gdk::EventType::*;
            if matches!(
                e.event_type(),
                ButtonPress | ButtonRelease | TouchBegin | TouchUpdate | TouchEnd | Scroll
            ) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        sel_pop.add_controller(shield);
        sel_pop.set_child(Some(&sel_stack));
        super::reader_extra::keep_focus_on_page(&sel_stack);

        // ---- sidebar: pages, contents, bookmarks, notes ----------------------------------
        let stack = adw::ViewStack::new();
        let thumb_pics: Rc<RefCell<HashMap<usize, glib::WeakRef<gtk::Picture>>>> =
            Rc::new(RefCell::new(HashMap::new()));
        let thumb_tex: Rc<RefCell<HashMap<usize, gdk::Texture>>> =
            Rc::new(RefCell::new(HashMap::new()));
        let thumbs = gtk::ListView::new(None::<gtk::SingleSelection>, None::<gtk::ListItemFactory>);
        thumbs.add_css_class("navigation-sidebar");
        super::audit::label(&thumbs, &tr("Page Thumbnails"));
        {
            let factory = gtk::SignalListItemFactory::new();
            factory.connect_setup(|_, item| {
                let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                    return;
                };
                let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
                b.set_margin_top(6);
                b.set_margin_bottom(6);
                let pic = gtk::Picture::new();
                pic.set_size_request(120, 160);
                pic.set_halign(gtk::Align::Center);
                pic.set_can_shrink(true);
                pic.add_css_class("card");
                let l = gtk::Label::new(None);
                l.add_css_class("caption");
                b.append(&pic);
                b.append(&l);
                item.set_child(Some(&b));
            });
            let (pics, tex, tx) = (
                thumb_pics.clone(),
                thumb_tex.clone(),
                sender.input_sender().clone(),
            );
            factory.connect_bind(move |_, item| {
                let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                    return;
                };
                let Some(b) = item.child().and_downcast::<gtk::Box>() else {
                    return;
                };
                let Some(pic) = b.first_child().and_downcast::<gtk::Picture>() else {
                    return;
                };
                let Some(l) = b.last_child().and_downcast::<gtk::Label>() else {
                    return;
                };
                let page = item.position() as usize;
                let label = item
                    .item()
                    .and_downcast::<gtk::StringObject>()
                    .map(|s| s.string().to_string())
                    .unwrap_or_default();
                l.set_label(&if label.is_empty() {
                    (page + 1).to_string()
                } else {
                    label
                });
                pics.borrow_mut().insert(page, pic.downgrade());
                match tex.borrow().get(&page) {
                    Some(t) => pic.set_paintable(Some(t)),
                    None => {
                        pic.set_paintable(None::<&gdk::Paintable>);
                        tx.emit(Input::ThumbWant(page));
                    }
                }
            });
            let (pics, tx) = (thumb_pics.clone(), sender.input_sender().clone());
            factory.connect_unbind(move |_, item| {
                let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                    return;
                };
                let page = item.position() as usize;
                pics.borrow_mut().remove(&page);
                tx.emit(Input::ThumbDrop(page));
            });
            thumbs.set_factory(Some(&factory));
            let tx = sender.input_sender().clone();
            thumbs.connect_activate(move |_, pos| tx.emit(Input::GoPage(pos as usize)));
        }
        let thumbs_scroll = gtk::ScrolledWindow::new();
        thumbs_scroll.set_child(Some(&thumbs));
        stack.add_titled_with_icon(&thumbs_scroll, Some("pages"), "Pages", "view-grid-symbolic");

        let toc_list = gtk::ListBox::new();
        toc_list.add_css_class("navigation-sidebar");
        {
            let tx = sender.input_sender().clone();
            let st = state.clone();
            let outline: Rc<RefCell<Vec<(usize, Option<f32>)>>> = Rc::new(RefCell::new(Vec::new()));
            let _ = (&st, &outline);
            toc_list.connect_row_activated(move |_, row| {
                tx.emit(Input::Dev(format!("toc-row={}", row.index())));
            });
        }
        let toc_scroll = gtk::ScrolledWindow::new();
        toc_scroll.set_child(Some(&toc_list));
        stack.add_titled_with_icon(&toc_scroll, Some("toc"), "Contents", "view-list-symbolic");
        let bm_list = gtk::ListBox::new();
        bm_list.add_css_class("navigation-sidebar");
        bm_list.set_selection_mode(gtk::SelectionMode::None);
        let bm_scroll = gtk::ScrolledWindow::new();
        bm_scroll.set_child(Some(&bm_list));
        stack.add_titled_with_icon(
            &bm_scroll,
            Some("bookmarks"),
            "Bookmarks",
            "user-bookmarks-symbolic",
        );
        let hl_list = gtk::ListBox::new();
        hl_list.add_css_class("navigation-sidebar");
        hl_list.set_selection_mode(gtk::SelectionMode::None);
        let hl_scroll = gtk::ScrolledWindow::new();
        hl_scroll.set_child(Some(&hl_list));
        stack.add_titled_with_icon(&hl_scroll, Some("notes"), "Notes", "document-edit-symbolic");
        stack.set_visible_child_name("toc");

        let switcher = adw::ViewSwitcherBar::new();
        switcher.set_stack(Some(&stack));
        switcher.set_reveal(true);
        let side_header = adw::HeaderBar::new();
        side_header.set_show_end_title_buttons(false);
        side_header.set_show_start_title_buttons(false);
        side_header.set_title_widget(Some(&adw::WindowTitle::new(&title_text, "")));
        let side_toolbar = adw::ToolbarView::new();
        side_toolbar.add_top_bar(&side_header);
        side_toolbar.set_content(Some(&stack));
        side_toolbar.add_bottom_bar(&switcher);

        // ---- content ---------------------------------------------------------------------
        let page_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
        left.append(&area);
        left.append(&hbar);
        left.set_hexpand(true);
        page_box.append(&left);
        page_box.append(&vbar);
        let content = gtk::Stack::new();
        let loading = adw::StatusPage::builder().title(tr("Opening PDF")).build();
        let spinner = adw::Spinner::new();
        spinner.set_size_request(48, 48);
        loading.set_child(Some(&spinner));
        content.add_named(&loading, Some("loading"));
        content.add_named(&page_box, Some("view"));
        let err = adw::StatusPage::builder()
            .title(tr("This PDF cannot be shown"))
            .description(tr(
                "The file is damaged or uses something Bookx cannot read.",
            ))
            .icon_name("dialog-warning-symbolic")
            .build();
        content.add_named(&err, Some("error"));
        content.set_visible_child_name("loading");

        let slider = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 1.0);
        super::audit::label(&slider, &tr("Page"));
        slider.set_draw_value(false);
        slider.set_hexpand(true);
        slider.add_css_class("reader-slider");
        let prev_btn = icon_button("go-previous-symbolic", &tr("Previous Page (Page Up)"));
        let next_btn = icon_button("go-next-symbolic", &tr("Next Page (Page Down)"));
        for b in [&prev_btn, &next_btn] {
            b.add_css_class("flat");
            b.add_css_class("circular");
        }
        let page_entry = gtk::Entry::new();
        page_entry.set_width_chars(5);
        page_entry.set_max_width_chars(8);
        gtk::prelude::EditableExt::set_alignment(&page_entry, 1.0);
        super::audit::label(&page_entry, &tr("Go to Page"));
        page_entry.set_tooltip_text(Some(&tr("Go to Page (Ctrl+G); a number or a page label")));
        let page_total = gtk::Label::new(None);
        page_total.add_css_class("dim-label");
        let nav = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        nav.add_css_class("reader-nav");
        nav.set_margin_start(16);
        nav.set_margin_end(16);
        nav.set_margin_bottom(6);
        nav.append(&prev_btn);
        nav.append(&slider);
        nav.append(&next_btn);
        nav.append(&page_entry);
        nav.append(&page_total);
        let caption = gtk::Label::new(None);
        caption.add_css_class("caption");
        caption.add_css_class("dim-label");
        let bottom = gtk::Box::new(gtk::Orientation::Vertical, 0);
        bottom.add_css_class("reader-bottom");
        bottom.append(&nav);

        let split = adw::OverlaySplitView::new();
        split.set_sidebar(Some(&side_toolbar));
        split.set_content(Some(&content));
        split.set_min_sidebar_width(240.0);
        split.set_max_sidebar_width(320.0);
        split.set_show_sidebar(false);

        // ---- header ------------------------------------------------------------------------
        let header = adw::HeaderBar::new();
        let title = adw::WindowTitle::new(&title_text, "");
        header.set_title_widget(Some(&title));
        let sidebar_btn = gtk::ToggleButton::new();
        sidebar_btn.set_icon_name("sidebar-show-symbolic");
        sidebar_btn.set_tooltip_text(Some(&tr("Pages, Contents, Bookmarks and Notes (F9)")));
        super::audit::label(&sidebar_btn, &tr("Sidebar"));
        split
            .bind_property("show-sidebar", &sidebar_btn, "active")
            .bidirectional()
            .sync_create()
            .build();
        header.pack_start(&sidebar_btn);
        let back_btn = icon_button(
            "go-jump-symbolic",
            &tr("Return to Previous Position (Alt+Left)"),
        );
        back_btn.set_visible(false);
        super::reader_menu::jump_style(&back_btn, None, false);
        header.pack_start(&back_btn);
        let bookmark_btn = icon_button(
            "bookmark-new-symbolic",
            &tr("Add a bookmark on this page (Ctrl+B)"),
        );
        header.pack_start(&bookmark_btn);

        let primary = gtk::MenuButton::new();
        primary.set_icon_name("open-menu-symbolic");
        primary.set_primary(true);
        primary.set_tooltip_text(Some(&tr("Main Menu")));
        let bin = adw::BreakpointBin::new();
        bin.set_size_request(360, 294);
        super::audit::label(&primary, &tr("Main Menu"));
        header.pack_end(&primary);
        let fullscreen_btn = icon_button("view-fullscreen-symbolic", &tr("Fullscreen (F11)"));
        header.pack_end(&fullscreen_btn);

        // search
        let search_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        search_box.set_size_request(380, 330);
        let search_entry = gtk::SearchEntry::new();
        search_entry.set_placeholder_text(Some(&tr("Search in the PDF")));
        search_entry.set_margin_start(8);
        search_entry.set_margin_end(8);
        search_entry.set_margin_top(8);
        super::audit::label(&search_entry, &tr("Search in the PDF"));
        let search_status = gtk::Label::new(None);
        search_status.add_css_class("dim-label");
        search_status.add_css_class("caption");
        let search_list = gtk::ListBox::new();
        search_list.add_css_class("navigation-sidebar");
        search_list.set_selection_mode(gtk::SelectionMode::None);
        let search_scroll = gtk::ScrolledWindow::new();
        search_scroll.set_vexpand(true);
        search_scroll.set_child(Some(&search_list));
        search_box.append(&search_entry);
        search_box.append(&search_status);
        search_box.append(&search_scroll);
        let search_empty = super::reader_search::empty_page();
        let search_pop = gtk::Popover::new();
        search_pop.set_child(Some(&search_box));
        {
            let b = search_box.clone();
            search_pop.connect_show(move |_| super::reader_search::fit(&b));
        }
        let search_btn = gtk::MenuButton::new();
        search_btn.set_icon_name("system-search-symbolic");
        search_btn.set_tooltip_text(Some(&tr("Search in the PDF (Ctrl+F)")));
        super::audit::label(&search_btn, &tr("Search in the PDF"));
        search_btn.set_popover(Some(&search_pop));
        header.pack_end(&search_btn);
        {
            let e = search_entry.clone();
            search_pop.connect_show(move |_| {
                e.grab_focus();
            });
            let tx = sender.input_sender().clone();
            search_entry
                .connect_search_changed(move |e| tx.emit(Input::Search(e.text().to_string())));
            let b = search_btn.clone();
            search_entry.connect_stop_search(move |_| b.popdown());
            let tx = sender.input_sender().clone();
            search_entry.connect_next_match(move |_| tx.emit(Input::SearchStep(true)));
            let tx = sender.input_sender().clone();
            search_entry.connect_previous_match(move |_| tx.emit(Input::SearchStep(false)));
            let tx = sender.input_sender().clone();
            search_entry.connect_activate(move |_| tx.emit(Input::SearchStep(true)));
        }

        // view menu
        let updating_ui = Rc::new(Cell::new(false));
        let view_pop = gtk::Popover::new();
        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 6);
        vbox.set_margin_start(12);
        vbox.set_margin_end(12);
        vbox.set_margin_top(12);
        vbox.set_margin_bottom(12);
        vbox.set_width_request(260);
        let zoom_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let minus = icon_button("zoom-out-symbolic", &tr("Zoom Out (Ctrl+−)"));
        let plus = icon_button("zoom-in-symbolic", &tr("Zoom In (Ctrl++)"));
        let zoom_label = gtk::Label::new(Some("100%"));
        zoom_label.set_hexpand(true);
        zoom_label.add_css_class("heading");
        for b in [&minus, &plus] {
            b.add_css_class("circular");
        }
        zoom_row.append(&minus);
        zoom_row.append(&zoom_label);
        zoom_row.append(&plus);
        vbox.append(&zoom_row);
        {
            let tx = sender.input_sender().clone();
            minus.connect_clicked(move |_| tx.emit(Input::ZoomStep(false)));
            let tx = sender.input_sender().clone();
            plus.connect_clicked(move |_| tx.emit(Input::ZoomStep(true)));
        }
        let mut zoom_btns: Vec<(String, gtk::CheckButton)> = Vec::new();
        let mut first_z: Option<gtk::CheckButton> = None;
        for (name, label) in [
            ("width", N_("Fit width")),
            ("page", N_("Fit page")),
            ("100", N_("100%")),
        ] {
            let c = gtk::CheckButton::with_label(&tr(label));
            if let Some(f) = &first_z {
                c.set_group(Some(f));
            } else {
                first_z = Some(c.clone());
            }
            let (tx, u) = (sender.input_sender().clone(), updating_ui.clone());
            c.connect_toggled(move |c| {
                if c.is_active() && !u.get() {
                    if let Some(z) = ZoomMode::from_name(name) {
                        tx.emit(Input::SetZoom(z));
                    }
                }
            });
            vbox.append(&c);
            zoom_btns.push((name.to_string(), c));
        }
        vbox.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let mut view_btns = Vec::new();
        let mut first_v: Option<gtk::CheckButton> = None;
        for (mode, label) in [
            (ViewMode::Continuous, N_("Continuous scroll")),
            (ViewMode::Single, N_("One page at a time")),
        ] {
            let c = gtk::CheckButton::with_label(&tr(label));
            if let Some(f) = &first_v {
                c.set_group(Some(f));
            } else {
                first_v = Some(c.clone());
            }
            let (tx, u) = (sender.input_sender().clone(), updating_ui.clone());
            c.connect_toggled(move |c| {
                if c.is_active() && !u.get() {
                    tx.emit(Input::SetView(mode));
                }
            });
            vbox.append(&c);
            view_btns.push((mode, c));
        }
        vbox.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let switch_row = |label: &str| -> (gtk::Box, gtk::Switch) {
            let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let l = gtk::Label::new(Some(label));
            l.set_xalign(0.0);
            l.set_hexpand(true);
            let s = gtk::Switch::new();
            s.set_valign(gtk::Align::Center);
            super::audit::label(&s, label);
            r.append(&l);
            r.append(&s);
            (r, s)
        };
        let (crop_row, crop_sw) = switch_row(&tr("Crop margins"));
        let (invert_row, invert_sw) = switch_row(&tr("Dark pages (invert colours)"));
        vbox.append(&crop_row);
        vbox.append(&invert_row);
        {
            let (tx, u) = (sender.input_sender().clone(), updating_ui.clone());
            crop_sw.connect_active_notify(move |s| {
                if !u.get() {
                    tx.emit(Input::SetCrop(s.is_active()));
                }
            });
            let (tx, u) = (sender.input_sender().clone(), updating_ui.clone());
            invert_sw.connect_active_notify(move |s| {
                if !u.get() {
                    tx.emit(Input::SetInvert(s.is_active()));
                }
            });
        }
        vbox.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let item = |label: &str| -> gtk::Button {
            let b = gtk::Button::with_label(label);
            b.add_css_class("flat");
            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                l.set_xalign(0.0);
            }
            b
        };
        let reflow_b = item(&tr("Reflow as text (experimental)"));
        reflow_b.set_tooltip_text(Some(
            &tr("Opens the text of the PDF in the e-book reader. Experimental: columns, tables and formulas may come out wrong."),
        ));
        let props_b = item(&tr("Properties"));
        vbox.append(&reflow_b);
        vbox.append(&props_b);
        {
            let (tx, pop) = (sender.input_sender().clone(), view_pop.clone());
            reflow_b.connect_clicked(move |_| {
                pop.popdown();
                tx.emit(Input::Reflow);
            });
            let (tx, pop) = (sender.input_sender().clone(), view_pop.clone());
            props_b.connect_clicked(move |_| {
                pop.popdown();
                tx.emit(Input::Properties);
            });
        }
        view_pop.set_child(Some(&vbox));
        let view_btn = gtk::MenuButton::new();
        view_btn.set_icon_name("preferences-desktop-display-symbolic");
        view_btn.set_tooltip_text(Some(&tr("View: zoom, layout, margins, colours")));
        super::audit::label(&view_btn, &tr("View options"));
        view_btn.set_popover(Some(&view_pop));
        header.pack_end(&view_btn);

        // main menu: the same builder as the e-book reader
        let wide_menu = super::reader_menu::build(&super::reader_menu::PDF_MENU);
        let narrow_menu = super::reader_menu::build(&super::reader_menu::narrow_entries(
            &super::reader_menu::PDF_MENU,
            &super::reader_menu::NARROW_PDF,
        ));
        primary.set_menu_model(Some(&wide_menu));
        super::audit::name_menu_items(&primary);
        let narrow = Rc::new(Cell::new(false));
        {
            let narrow_flag = narrow.clone();
            let (primary, fs, back) = (primary.clone(), fullscreen_btn.clone(), back_btn.clone());
            super::reader_menu::install_narrow(&bin, move |is_narrow| {
                narrow_flag.set(is_narrow);
                fs.set_visible(!is_narrow);
                primary.set_menu_model(Some(if is_narrow { &narrow_menu } else { &wide_menu }));
                super::reader_menu::jump_style(&back, None, is_narrow);
            });
        }
        {
            let group = gio::SimpleActionGroup::new();
            let add = |name: &str, input: fn() -> Input| {
                let a = gio::SimpleAction::new(name, None);
                let tx = sender.input_sender().clone();
                a.connect_activate(move |_, _| tx.emit(input()));
                group.add_action(&a);
            };
            add("goto", || Input::ShowTab("goto"));
            add("reflow", || Input::Reflow);
            add("properties", || Input::Properties);
            add("fullscreen", || Input::Fullscreen);
            root.insert_action_group("reader", Some(&group));
        }

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        let sync_banner = {
            let tx = sender.input_sender().clone();
            super::reader_live::LiveBanner::new(move |a| tx.emit(Input::LiveAnswer(a)))
        };
        let page_overlay = gtk::Overlay::new();
        page_overlay.set_child(Some(&split));
        page_overlay.add_overlay(&sync_banner.widget);
        toolbar.set_content(Some(&page_overlay));
        toolbar.add_bottom_bar(&bottom);
        bin.set_child(Some(&toolbar));
        root.set_child(Some(&bin));
        root.set_title(&title_text);
        root.add_css_class("reader-page");
        root.add_css_class("pdf-reader");

        // wiring of the footer
        {
            let tx = sender.input_sender().clone();
            prev_btn.connect_clicked(move |_| tx.emit(Input::Turn(false)));
            let tx = sender.input_sender().clone();
            next_btn.connect_clicked(move |_| tx.emit(Input::Turn(true)));
            let tx = sender.input_sender().clone();
            bookmark_btn.connect_clicked(move |_| tx.emit(Input::ToggleBookmark));
            let tx = sender.input_sender().clone();
            back_btn.connect_clicked(move |_| tx.emit(Input::Back));
            let tx = sender.input_sender().clone();
            fullscreen_btn.connect_clicked(move |_| tx.emit(Input::Fullscreen));
            let (tx, u) = (sender.input_sender().clone(), updating_ui.clone());
            slider.connect_change_value(move |_, _, v| {
                if !u.get() {
                    tx.emit(Input::Seek(v));
                }
                glib::Propagation::Proceed
            });
            let tx = sender.input_sender().clone();
            page_entry.connect_activate(move |e| tx.emit(Input::PageBox(e.text().to_string())));
        }

        // keyboard
        let keys = gtk::EventControllerKey::new();
        {
            let tx = sender.input_sender().clone();
            let root_c = root.clone();
            keys.connect_key_pressed(move |_, key, _, mods| {
                use gdk::Key;
                if let Some(win) = window_of(&root_c) {
                    if let Some(f) = gtk::prelude::GtkWindowExt::focus(&win) {
                        if f.is::<gtk::Text>() || f.is::<gtk::TextView>() {
                            return glib::Propagation::Proceed;
                        }
                    }
                }
                let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
                let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
                match key {
                    Key::Page_Down => tx.emit(Input::Turn(true)),
                    Key::Page_Up => tx.emit(Input::Turn(false)),
                    Key::space => tx.emit(Input::Turn(!shift)),
                    Key::Down | Key::Up | Key::Left | Key::Right if !ctrl => {
                        tx.emit(Input::Dev(format!(
                            "nudge={}",
                            match key {
                                Key::Down => "down",
                                Key::Up => "up",
                                Key::Left => "left",
                                _ => "right",
                            }
                        )))
                    }
                    Key::Home => tx.emit(Input::GoPage(0)),
                    Key::End => tx.emit(Input::GoPage(usize::MAX)),
                    Key::F11 => tx.emit(Input::Fullscreen),
                    Key::Escape => tx.emit(Input::Quit),
                    Key::b if ctrl => tx.emit(Input::ToggleBookmark),
                    Key::f if ctrl => tx.emit(Input::ShowTab("search")),
                    Key::g if ctrl => tx.emit(Input::ShowTab("goto")),
                    Key::plus | Key::equal | Key::KP_Add if ctrl => tx.emit(Input::ZoomStep(true)),
                    Key::minus | Key::KP_Subtract if ctrl => tx.emit(Input::ZoomStep(false)),
                    Key::_0 | Key::KP_0 if ctrl => tx.emit(Input::SetZoom(ZoomMode::FitWidth)),
                    Key::c if ctrl => tx.emit(Input::SelectionKey("copy")),
                    Key::F9 => tx.emit(Input::ShowTab("sidebar")),
                    _ => return glib::Propagation::Proceed,
                }
                glib::Propagation::Stop
            });
        }
        root.add_controller(keys);

        let dirty = Rc::new(Cell::new(false));
        let timer = {
            let dirty = dirty.clone();
            let shared = shared.clone();
            glib::timeout_add_seconds_local(5, move || {
                if dirty.replace(false) {
                    let _ = shared.lib.borrow().save();
                }
                glib::ControlFlow::Continue
            })
        };
        let poll = {
            let tx = sender.input_sender().clone();
            glib::timeout_add_local(std::time::Duration::from_millis(20), move || {
                tx.emit(Input::Poll);
                glib::ControlFlow::Continue
            })
        };

        let worker = Worker::open(data.as_ref().clone(), None);
        let w = W {
            page: root.clone(),
            title,
            split,
            stack,
            area: area.clone(),
            vbar,
            hbar,
            content,
            toc_list,
            bm_list,
            hl_list,
            thumbs,
            search_entry,
            search_btn,
            search_list,
            search_status,
            search_empty,
            slider,
            page_entry,
            primary: primary.clone(),
            header: header.clone(),
            narrow,
            fullscreen_btn: fullscreen_btn.clone(),
            page_total,
            caption,
            bookmark_btn,
            back_btn,
            prev_btn,
            next_btn,
            zoom_label,
            sel_pop,
            sel_stack,
            sel_remove,
            switching,
            note_view,
            note_btn,
            note_preview,
            note_label,
            swatches,
            style_btns,
            dict_text,
            wiki_text,
            wiki_spin,
            wiki_url,
            tr_text,
            tr_spin,
            tr_dd,
            zoom_btns,
            view_btns,
            crop_sw,
            invert_sw,
            thumb_pics,
            thumb_tex,
        };
        let model = ReaderPdf {
            shared,
            book_id,
            title: title_text,
            language,
            data,
            worker: Some(worker),
            summary: None,
            state,
            hadj,
            vadj,
            w,
            sender: sender.clone(),
            prefs,
            pending: HashMap::new(),
            text_pending: HashSet::new(),
            thumb_pending: HashMap::new(),
            crop_wait: 0,
            crop_boxes: Vec::new(),
            selection: None,
            drag: None,
            press_at: (0.0, 0.0),
            history: Vec::new(),
            search_id: 0,
            search_rows: Vec::new(),
            search_done: true,
            search_active: None,
            dirty,
            timer: Some(timer),
            poll: Some(poll),
            fetch_gen: Rc::new(Cell::new(0)),
            hl_style: HighlightStyle::default(),
            fullscreen: false,
            opened: false,
            asked_password: false,
            updating_ui,
            last_page_stat: usize::MAX,
            sync_banner,
            _sync_sub: Some({
                let tx = sender.input_sender().clone();
                crate::sync_bus::subscribe(Some(book_id), move |e| tx.emit(Input::Sync(e.clone())))
            }),
            sync_stats: Cell::new((0, 0)),
            last_turn: Cell::new(usize::MAX),
        };
        model.apply_prefs_to_ui();
        {
            let area = model.w.area.clone();
            model.w.page.connect_map(move |_| {
                let a = area.clone();
                glib::idle_add_local_once(move || {
                    a.grab_focus();
                });
            });
        }
        ComponentParts { model, widgets: () }
    }

    fn update(&mut self, msg: Input, sender: ComponentSender<Self>) {
        match msg {
            Input::Poll => {
                let mut budget = 24;
                while budget > 0 {
                    let ev = self.worker.as_ref().and_then(|w| w.try_recv());
                    let Some(ev) = ev else { break };
                    self.handle_event(ev, &sender);
                    budget -= 1;
                }
            }
            Input::Resize => {
                if self.opened || self.summary.is_some() {
                    self.relayout();
                }
            }
            Input::Scrolled => {
                if self.summary.is_some() {
                    self.on_scrolled();
                }
            }
            Input::GoPage(p) => {
                self.goto_page(p, None);
                self.w.area.grab_focus();
            }
            Input::GoDest(p, y) => self.goto_page(p, y),
            Input::Turn(fwd) => {
                let n = self.page_count();
                if n == 0 {
                    return;
                }
                let single = self.state.borrow().layout.mode == ViewMode::Single;
                let cur = self.current_page();
                if single {
                    let next = if fwd {
                        (cur + 1).min(n - 1)
                    } else {
                        cur.saturating_sub(1)
                    };
                    self.goto_page(next, None);
                    if !fwd && next != cur {
                        let max = (self.vadj.upper() - self.vadj.page_size()).max(0.0);
                        self.vadj.set_value(max);
                    }
                } else {
                    // a screen at a time
                    let step = self.vadj.page_increment();
                    let v = self.vadj.value() + if fwd { step } else { -step };
                    let max = (self.vadj.upper() - self.vadj.page_size()).max(0.0);
                    self.vadj.set_value(v.clamp(0.0, max));
                }
                self.save_progress();
            }
            Input::Seek(v) => {
                let p = v.round().max(0.0) as usize;
                if p != self.current_page() {
                    self.goto_page(p, None);
                }
            }
            Input::PageBox(t) => match pdf::resolve_page(&t, &self.labels(), self.page_count()) {
                Some(p) => {
                    self.goto_page(p, None);
                    self.w.area.grab_focus();
                }
                None => {
                    let _ = sender.output(Output::Toast(tr("There is no such page")));
                    self.update_position_ui();
                }
            },
            Input::SetZoom(z) => {
                self.prefs.zoom = z.name();
                self.prefs.save();
                self.state.borrow_mut().layout.zoom = z;
                self.relayout();
            }
            Input::ZoomStep(inc) => {
                let pct = {
                    let st = self.state.borrow();
                    st.places
                        .first()
                        .map(|p| p.scale / pdf::PX_PER_PT * 100.0)
                        .unwrap_or(100.0)
                };
                let z = ZoomMode::Percent(pdf::step_zoom(pct, inc));
                sender.input(Input::SetZoom(z));
            }
            Input::ZoomBy(f) => {
                let pct = {
                    let st = self.state.borrow();
                    st.places
                        .first()
                        .map(|p| p.scale / pdf::PX_PER_PT * 100.0)
                        .unwrap_or(100.0)
                };
                let z = ZoomMode::Percent(((pct as f64 * f).round() as u32).clamp(10, 800));
                sender.input(Input::SetZoom(z));
            }
            Input::SetView(m) => {
                self.prefs.single = m == ViewMode::Single;
                self.prefs.save();
                self.state.borrow_mut().layout.mode = m;
                let cur = self.current_page();
                {
                    let mut st = self.state.borrow_mut();
                    st.places = st.layout.places(cur);
                }
                self.relayout();
                self.goto_page(cur, None);
            }
            Input::SetCrop(on) => {
                self.prefs.crop = on;
                self.prefs.save();
                if on {
                    self.start_crop();
                } else {
                    self.state.borrow_mut().layout.crop = None;
                    self.relayout();
                }
            }
            Input::SetInvert(on) => {
                self.prefs.invert = on;
                self.prefs.save();
                self.rerender_everything();
            }
            Input::Press(x, y) => {
                self.press_at = (x, y);
                self.drag = None;
                if self.w.sel_pop.is_visible() {
                    self.w.sel_pop.popdown();
                }
                if self.state.borrow().selection.is_some() {
                    self.set_selection(None);
                }
                self.w.area.grab_focus();
                let (dx, dy) = self.doc_pos(x, y);
                let st = self.state.borrow();
                if let Some((page, px, py)) = st.page_at(dx, dy, false) {
                    if let Some(i) = st.texts.get(&page).and_then(|t| t.char_at(px, py)) {
                        drop(st);
                        self.drag = Some((page, i, x, y));
                    }
                }
            }
            Input::Drag(dx, dy) => {
                let Some((page, anchor, sx, sy)) = self.drag else {
                    return;
                };
                if dx.abs() + dy.abs() < 3.0 {
                    return;
                }
                let (docx, docy) = self.doc_pos(sx + dx, sy + dy);
                let cur = {
                    let st = self.state.borrow();
                    let Some(pl) = st.place_of(page) else { return };
                    let (px, py) = pdf::to_pt(pl, st.layout.crop, st.size_pt(page), docx, docy);
                    st.texts.get(&page).and_then(|t| t.nearest_char(px, py))
                };
                if let Some(c) = cur {
                    let (a, b) = (anchor.min(c), anchor.max(c) + 1);
                    let sel = Sel {
                        page,
                        start: a,
                        end: b,
                        highlight: None,
                    };
                    self.set_selection(Some(sel));
                }
            }
            Input::DragEnd(_, _) => {
                if self.drag.take().is_some() && self.selection.is_some() {
                    self.show_popover();
                }
            }
            Input::Click(x, y, n) => {
                let moved = (x - self.press_at.0).abs() + (y - self.press_at.1).abs();
                if moved > 6.0 {
                    return;
                }
                let (dx, dy) = self.doc_pos(x, y);
                if n >= 2 {
                    let sel = {
                        let st = self.state.borrow();
                        st.page_at(dx, dy, false).and_then(|(page, px, py)| {
                            let t = st.texts.get(&page)?;
                            let i = t.char_at(px, py)?;
                            let r = if n == 2 { t.word_at(i)? } else { t.line_at(i)? };
                            Some(Sel {
                                page,
                                start: r.start,
                                end: r.end,
                                highlight: None,
                            })
                        })
                    };
                    if sel.is_some() {
                        self.set_selection(sel);
                        self.show_popover();
                    }
                    return;
                }
                if let Some(l) = self.link_at(dx, dy) {
                    self.activate_link(&l);
                    return;
                }
                if let Some((id, _)) = self.highlight_at(dx, dy) {
                    self.select_stored(&id);
                    return;
                }
                // a tap near the edge turns the page in single page mode
                let single = self.state.borrow().layout.mode == ViewMode::Single;
                if single {
                    let (vw, _) = self.view_size();
                    if x < f64::from(vw) * 0.2 {
                        sender.input(Input::Turn(false));
                    } else if x > f64::from(vw) * 0.8 {
                        sender.input(Input::Turn(true));
                    }
                }
            }
            Input::Hover(x, y) => {
                let (dx, dy) = self.doc_pos(x, y);
                let name = if self.link_at(dx, dy).is_some() || self.highlight_at(dx, dy).is_some()
                {
                    "pointer"
                } else {
                    let st = self.state.borrow();
                    let over_text = st
                        .page_at(dx, dy, false)
                        .and_then(|(p, px, py)| st.texts.get(&p).and_then(|t| t.char_at(px, py)))
                        .is_some();
                    if over_text {
                        "text"
                    } else {
                        "default"
                    }
                };
                self.w.area.set_cursor_from_name(Some(name));
            }
            Input::Highlight(color) => {
                if self.state.borrow().selection.is_none() {
                    return;
                }
                self.make_highlight(&color, None);
            }
            Input::HlStyle(st) => {
                self.hl_style = st;
                for (s, b) in &self.w.style_btns {
                    b.set_active(*s == st);
                }
                // changing the style of a stored highlight applies at once
                let sel = self.state.borrow().selection.clone();
                if let Some(Sel {
                    highlight: Some(_), ..
                }) = sel
                {
                    let color = self
                        .shared
                        .lib
                        .borrow()
                        .get(self.book_id)
                        .and_then(|b| {
                            b.highlights
                                .iter()
                                .find(|h| {
                                    Some(&h.id) == sel.as_ref().and_then(|s| s.highlight.as_ref())
                                })
                                .map(|h| h.color.clone())
                        })
                        .unwrap_or_else(|| "yellow".into());
                    self.make_highlight(&color, None);
                }
            }
            Input::SaveNote(note) => {
                if self.state.borrow().selection.is_none() {
                    return;
                }
                let existing_color = {
                    let sel = self.state.borrow().selection.clone();
                    sel.and_then(|s| s.highlight).and_then(|id| {
                        self.shared.lib.borrow().get(self.book_id).and_then(|b| {
                            b.highlights
                                .iter()
                                .find(|h| h.id == id)
                                .map(|h| h.color.clone())
                        })
                    })
                };
                self.make_highlight(
                    &existing_color.unwrap_or_else(|| "yellow".into()),
                    Some(note),
                );
                self.w.switching.set(true);
                self.w.sel_pop.popdown();
                self.w.sel_pop.set_autohide(false);
                self.w.sel_stack.set_visible_child_name("actions");
                self.w.switching.set(false);
                self.set_selection(None);
            }
            Input::RemoveHighlight => {
                let id = self
                    .state
                    .borrow()
                    .selection
                    .as_ref()
                    .and_then(|s| s.highlight.clone());
                if let Some(id) = id {
                    sender.input(Input::DeleteHighlight(id));
                }
                self.w.sel_pop.popdown();
                self.set_selection(None);
            }
            Input::DeleteHighlight(id) => {
                let removed = {
                    let mut lib = self.shared.lib.borrow_mut();
                    let r = lib
                        .get_mut(self.book_id)
                        .and_then(|b| crate::remove_undo::take_highlight(&mut b.highlights, &id));
                    let _ = lib.save();
                    r
                };
                self.refresh_highlights();
                self.rebuild_lists();
                if let Some(r) = removed {
                    self.offer_undo(r, &sender);
                }
            }
            Input::Sync(ev) => self.on_sync(&ev),
            Input::LiveAnswer(a) => {
                use super::reader_live::Answer;
                if let Answer::Primary(crate::sync_live::Banner::Ahead { percentage, .. }) = a {
                    let n = self.page_count();
                    if n > 0 {
                        let page = ((percentage * n as f64) as usize).min(n - 1);
                        self.history.push((self.current_page(), None));
                        self.w.back_btn.set_visible(true);
                        self.goto_page(page, None);
                    }
                }
            }
            Input::Restored => {
                self.refresh_highlights();
                self.rebuild_lists();
                self.update_bookmark_button();
            }
            Input::PopClosed => {
                if self.w.sel_stack.visible_child_name().as_deref() == Some("note") {
                    let b = self.w.note_view.buffer();
                    let (s, e) = b.bounds();
                    let text = b.text(&s, &e, false).trim().to_string();
                    if !text.is_empty() && self.state.borrow().selection.is_some() {
                        sender.input(Input::SaveNote(text));
                        return;
                    }
                }
                self.w.sel_pop.set_autohide(false);
                self.w.sel_stack.set_visible_child_name("actions");
                // a stored highlight that was only looked at stops being the selection
                if self
                    .state
                    .borrow()
                    .selection
                    .as_ref()
                    .is_some_and(|s| s.highlight.is_some())
                {
                    self.set_selection(None);
                }
            }
            Input::Copy | Input::SelectionKey("copy") => {
                let sel = self.state.borrow().selection.clone();
                if let Some(s) = sel {
                    let t = self.selection_text(&s);
                    if !t.is_empty() {
                        self.w.area.clipboard().set_text(&t);
                        let _ = sender.output(Output::Toast(tr("Copied to clipboard")));
                    }
                }
            }
            Input::SelectionKey(k) => match k {
                "dict" => sender.input(Input::Dict),
                "wiki" => sender.input(Input::Wiki),
                "translate" => sender.input(Input::Translate),
                "highlight" => sender.input(Input::Highlight("yellow".into())),
                "note" => self.w.note_btn.emit_clicked(),
                "clear" => {
                    self.set_selection(None);
                    self.w.sel_pop.popdown();
                }
                _ => {}
            },
            Input::Dict => {
                let Some(term) = self
                    .selection
                    .as_ref()
                    .and_then(|s| crate::dictionaries::term_for_selection(&s.text))
                else {
                    return;
                };
                self.w.sel_stack.set_visible_child_name("dict");
                let dir = crate::dictionaries::dictionary_dir();
                let loaded = crate::dictionaries::load(&dir);
                let text = if loaded.set.names().is_empty() {
                    glib::markup_escape_text(&crate::dictionaries::empty_help(&dir)).to_string()
                } else {
                    let lookup = loaded.set.lookup_all(&term);
                    let mut t = crate::dictionaries::render(&term, &lookup);
                    let definition = lookup
                        .results
                        .first()
                        .and_then(|r| r.entries.first())
                        .map(|e| e.plain_text());
                    if definition.is_some() {
                        let sel_text = self
                            .selection
                            .as_ref()
                            .map(|s| s.text.clone())
                            .unwrap_or_default();
                        let now = self.shared.now();
                        let added = self.shared.vocab.borrow_mut().record_lookup(
                            &term,
                            &sel_text,
                            self.book_id,
                            definition.as_deref(),
                            now,
                        );
                        if matches!(added, Ok(true)) {
                            t.push_str("\n\n<small>Saved to your vocabulary.</small>");
                        }
                    }
                    t
                };
                self.w.dict_text.set_markup(&text);
            }
            Input::Wiki => {
                let Some(sel) = &self.selection else { return };
                let term: String = sel
                    .text
                    .split_whitespace()
                    .take(8)
                    .collect::<Vec<_>>()
                    .join(" ");
                let lang = self.language.split('-').next().unwrap_or("en").to_string();
                let lang = if lang.len() == 2 { lang } else { "en".into() };
                *self.w.wiki_url.borrow_mut() = format!(
                    "https://{lang}.wikipedia.org/w/index.php?search={}",
                    web::encode(&term)
                );
                self.w.sel_stack.set_visible_child_name("wiki");
                self.w.wiki_text.set_label("");
                self.w.wiki_spin.set_opacity(1.0);
                self.w.wiki_spin.start();
                let r#gen = self.fetch_gen.get() + 1;
                self.fetch_gen.set(r#gen);
                let (g, spin, label, url) = (
                    self.fetch_gen.clone(),
                    self.w.wiki_spin.clone(),
                    self.w.wiki_text.clone(),
                    self.w.wiki_url.clone(),
                );
                web::wikipedia(&lang, &term, move |res| {
                    if g.get() != r#gen {
                        return;
                    }
                    spin.stop();
                    spin.set_opacity(0.0);
                    match res {
                        Ok(r) => {
                            label.set_markup(&format!(
                                "<b>{}</b>\n{}",
                                glib::markup_escape_text(&r.title),
                                glib::markup_escape_text(&r.extract)
                            ));
                            *url.borrow_mut() = r.url;
                        }
                        Err(e) => label.set_label(&trf("Couldn't load a summary.\n{}\n\nUse “Open Wikipedia” to search in the browser.", &[&e])),
                    }
                });
            }
            Input::Translate => {
                let Some(sel) = &self.selection else { return };
                let text: String = sel
                    .text
                    .split_whitespace()
                    .take(120)
                    .collect::<Vec<_>>()
                    .join(" ");
                let cur = self.shared.cfg.borrow().reader.translate_to.clone();
                let to = web::pick_translate_target(&cur, &self.language);
                if to != cur {
                    {
                        let mut c = self.shared.cfg.borrow_mut();
                        c.reader.translate_to = to.clone();
                        c.save();
                    }
                    if let Some(i) = web::LANGUAGES.iter().position(|(c, _)| *c == to) {
                        self.w.tr_dd.set_selected(i as u32);
                    }
                }
                self.w.sel_stack.set_visible_child_name("translate");
                self.w.tr_text.set_label("");
                self.w.tr_spin.set_opacity(1.0);
                self.w.tr_spin.start();
                let r#gen = self.fetch_gen.get() + 1;
                self.fetch_gen.set(r#gen);
                let (g, spin, label) = (
                    self.fetch_gen.clone(),
                    self.w.tr_spin.clone(),
                    self.w.tr_text.clone(),
                );
                web::translate(&to, &text, move |res| {
                    if g.get() != r#gen {
                        return;
                    }
                    spin.stop();
                    spin.set_opacity(0.0);
                    match res {
                        Ok(t) => label.set_label(&t),
                        Err(e) => label.set_label(&trf("Couldn't translate this text.\n{}", &[&e])),
                    }
                });
            }
            Input::TranslateLang(code) => {
                if code == self.shared.cfg.borrow().reader.translate_to {
                    return;
                }
                {
                    let mut c = self.shared.cfg.borrow_mut();
                    c.reader.translate_to = code;
                    c.save();
                }
                if self.w.sel_stack.visible_child_name().as_deref() == Some("translate") {
                    sender.input(Input::Translate);
                }
            }
            Input::Search(q) => {
                self.search_id += 1;
                self.search_rows.clear();
                self.search_active = None;
                {
                    let mut st = self.state.borrow_mut();
                    st.hits.clear();
                    st.active_hit = None;
                }
                clear_list(&self.w.search_list);
                self.w.area.queue_draw();
                let q = q.trim().to_string();
                if q.is_empty() {
                    self.search_done = true;
                    self.search_id += 1;
                    self.w.search_status.set_label("");
                    return;
                }
                self.search_done = false;
                if let Some(w) = &self.worker {
                    w.search(self.search_id, &q);
                }
                self.update_search_status();
            }
            Input::SearchResult(i) => self.activate_hit(i, true),
            Input::SearchStep(fwd) => {
                let hits: Vec<(usize, usize)> = self
                    .search_rows
                    .iter()
                    .map(|r| (r.page, r.range.start))
                    .collect();
                let (page, pos) = match self.search_active {
                    Some(i) => hits.get(i).copied().unwrap_or((self.current_page(), 0)),
                    None => (self.current_page(), 0),
                };
                let from = if self.search_active.is_none() && fwd {
                    (page, 0)
                } else {
                    (page, pos)
                };
                let next = if self.search_active.is_none() && fwd {
                    hits.iter().position(|h| *h >= from).or(if hits.is_empty() {
                        None
                    } else {
                        Some(0)
                    })
                } else {
                    pdf::step_hit(&hits, from.0, from.1, fwd)
                };
                if let Some(i) = next {
                    self.activate_hit(i, false);
                }
            }
            Input::ToggleBookmark => {
                let cur = self.current_page();
                let n = self.page_count();
                if n == 0 {
                    return;
                }
                let label = {
                    let st = self.state.borrow();
                    st.texts
                        .get(&cur)
                        .and_then(|t| t.lines.first().map(|l| t.text_for(l.start..l.end)))
                        .unwrap_or_default()
                };
                let mut removed = None;
                {
                    let mut lib = self.shared.lib.borrow_mut();
                    if let Some(b) = lib.get_mut(self.book_id) {
                        if let Some(i) = b.bookmarks.iter().position(|m| m.spine == cur) {
                            let id = b.bookmarks[i].id.clone();
                            removed = crate::remove_undo::take_bookmark(&mut b.bookmarks, &id);
                        } else {
                            b.bookmarks.push(Bookmark {
                                id: uuid::Uuid::new_v4().to_string(),
                                spine: cur,
                                frac: 0.0,
                                label: label.chars().take(80).collect(),
                                created: Utc::now(),
                            });
                        }
                    }
                    let _ = lib.save();
                }
                self.update_bookmark_button();
                self.rebuild_lists();
                if let Some(r) = removed {
                    self.offer_undo(r, &sender);
                }
            }
            Input::GoBookmark(id) => {
                let page = self
                    .shared
                    .lib
                    .borrow()
                    .get(self.book_id)
                    .and_then(|b| b.bookmarks.iter().find(|m| m.id == id).map(|m| m.spine));
                if let Some(p) = page {
                    self.goto_page(p, None);
                }
            }
            Input::RemoveBookmark(id) => {
                let removed = {
                    let mut lib = self.shared.lib.borrow_mut();
                    let r = lib
                        .get_mut(self.book_id)
                        .and_then(|b| crate::remove_undo::take_bookmark(&mut b.bookmarks, &id));
                    let _ = lib.save();
                    r
                };
                self.update_bookmark_button();
                self.rebuild_lists();
                if let Some(r) = removed {
                    self.offer_undo(r, &sender);
                }
            }
            Input::GoHighlight(id) => {
                let page = self.shared.lib.borrow().get(self.book_id).and_then(|b| {
                    b.highlights
                        .iter()
                        .find(|h| h.id == id)
                        .map(|h| {
                            let y = pdf::parse_pdf_style(&h.style)
                                .and_then(|(_, r)| r.first().map(|r| r[1]));
                            (h.spine, y)
                        })
                        // the Notes list also opens a bookmark by its id
                        .or_else(|| {
                            b.bookmarks
                                .iter()
                                .find(|m| m.id == id)
                                .map(|m| (m.spine, None))
                        })
                });
                if let Some((p, y)) = page {
                    self.goto_page(p, y);
                }
            }
            Input::ThumbWant(page) => {
                if self.thumb_pending.contains_key(&page)
                    || self.w.thumb_tex.borrow().contains_key(&page)
                {
                    return;
                }
                if let Some(w) = &self.worker {
                    let c = w.render(page, THUMB_SCALE, self.prefs.invert);
                    self.thumb_pending.insert(page, c);
                }
            }
            Input::ThumbDrop(page) => {
                if let Some(c) = self.thumb_pending.remove(&page) {
                    c.cancel();
                }
            }
            Input::ShowTab(t) => match t {
                "search" => {
                    self.w.search_btn.popup();
                }
                "goto" => {
                    self.w.page_entry.grab_focus();
                    self.w.page_entry.select_region(0, -1);
                }
                "sidebar" => {
                    let s = !self.w.split.shows_sidebar();
                    self.w.split.set_show_sidebar(s);
                }
                tab => {
                    self.w.split.set_show_sidebar(true);
                    self.w.stack.set_visible_child_name(tab);
                }
            },
            Input::Back => {
                if let Some((p, y)) = self.history.pop() {
                    self.goto_page(p, y);
                }
                self.w.back_btn.set_visible(!self.history.is_empty());
            }
            Input::Reflow => self.open_reflow(),
            Input::Properties => self.properties(),
            Input::Fullscreen => {
                if let Some(win) = window_of(&self.w.page) {
                    self.fullscreen = !win.is_fullscreen();
                    win.set_fullscreened(self.fullscreen);
                }
            }
            Input::Password(pw) => match pw {
                Some(pw) => {
                    self.asked_password = false;
                    self.worker = Some(Worker::open(self.data.as_ref().clone(), Some(pw)));
                }
                None => {
                    let _ = sender.output(Output::Close);
                }
            },
            Input::Quit => {
                if let Some(win) = window_of(&self.w.page) {
                    if win.is_fullscreen() {
                        win.set_fullscreened(false);
                        self.fullscreen = false;
                        return;
                    }
                }
                let _ = sender.output(Output::Close);
            }
            Input::Dev(c) => self.dev(&c, &sender),
        }
        let _ = (
            &self.last_page_stat,
            &self.title,
            &self.w.swatches,
            &self.w.zoom_btns,
            &self.w.bm_list,
        );
    }

    fn shutdown(&mut self, _widgets: &mut Self::Widgets, _output: relm4::Sender<Self::Output>) {
        self.save_progress();
        let now = self.shared.now();
        self.shared.stats.borrow_mut().finish(now);
        let _ = self.shared.lib.borrow().save();
        if let Some(t) = self.timer.take() {
            t.remove();
        }
        if let Some(p) = self.poll.take() {
            p.remove();
        }
        for c in self.pending.values() {
            c.cancel();
        }
        self.worker = None;
        if self.w.sel_pop.parent().is_some() {
            self.w.sel_pop.unparent();
        }
        if let Some(win) = window_of(&self.w.page) {
            win.set_fullscreened(false);
        }
    }
}

impl ReaderPdf {
    /// Test hooks: `reader:pdf-…` commands (see `_developer/docs.md`).
    fn dev(&mut self, c: &str, sender: &ComponentSender<Self>) {
        let (cmd, arg) = c.split_once('=').unwrap_or((c, ""));
        match cmd {
            "toc-row" => {
                let i: usize = arg.parse().unwrap_or(0);
                let dest = self
                    .summary
                    .as_ref()
                    .and_then(|s| s.outline.get(i))
                    .and_then(|o| o.dest);
                if let Some(d) = dest {
                    self.history.push((self.current_page(), None));
                    self.w.back_btn.set_visible(true);
                    self.goto_page(d.page, d.y);
                    self.w.area.grab_focus();
                }
            }
            "nudge" => {
                let (v, h) = (self.vadj.step_increment(), self.hadj.step_increment());
                let single = self.state.borrow().layout.mode == ViewMode::Single;
                match arg {
                    "down" => self.vadj.set_value(self.vadj.value() + v),
                    "up" => self.vadj.set_value(self.vadj.value() - v),
                    "left" if single => sender.input(Input::Turn(false)),
                    "right" if single => sender.input(Input::Turn(true)),
                    "left" => self.hadj.set_value(self.hadj.value() - h),
                    _ => self.hadj.set_value(self.hadj.value() + h),
                }
            }
            "pdf-state" => {
                let st = self.state.borrow();
                let top = self.vadj.value();
                eprintln!(
                    "pdf_state: opened={} pages={} page={} zoom={} mode={:?} crop={} invert={} scale={:.3} rendered={} texts={} top={:.0} upper={:.0} sel={} hl={} links={} hits={} sidebar={}",
                    self.opened,
                    self.page_count(),
                    st.current + 1,
                    st.layout.zoom.name(),
                    st.layout.mode,
                    st.layout.crop.is_some(),
                    st.invert,
                    st.places.first().map(|p| p.scale).unwrap_or(0.0),
                    st.cache.len(),
                    st.texts.len(),
                    top,
                    self.vadj.upper(),
                    self.selection.as_ref().map(|s| s.text.as_str()).unwrap_or(""),
                    st.highlights.len(),
                    st.links.values().map(|l| l.len()).sum::<usize>(),
                    self.search_rows.len(),
                    self.w.split.shows_sidebar(),
                );
            }
            "pdf-goto" => {
                let p = pdf::resolve_page(arg, &self.labels(), self.page_count());
                if let Some(p) = p {
                    self.goto_page(p, None);
                }
            }
            "pdf-zoom" => {
                if let Some(z) = ZoomMode::from_name(arg) {
                    sender.input(Input::SetZoom(z));
                }
            }
            "pdf-zoom-in" => sender.input(Input::ZoomStep(true)),
            "pdf-zoom-out" => sender.input(Input::ZoomStep(false)),
            "pdf-view" => sender.input(Input::SetView(if arg == "single" {
                ViewMode::Single
            } else {
                ViewMode::Continuous
            })),
            "pdf-crop" => sender.input(Input::SetCrop(arg == "on")),
            "pdf-invert" => sender.input(Input::SetInvert(arg == "on")),
            "pdf-scroll-by" => {
                let d: f64 = arg.parse().unwrap_or(0.0);
                self.vadj.set_value(self.vadj.value() + d);
            }
            "pdf-turn" => sender.input(Input::Turn(arg != "back")),
            // pdf-select=TEXT: select the first match of TEXT on the current page (or later pages)
            "pdf-select" => {
                let from = self.current_page();
                let n = self.page_count();
                for p in from..n {
                    let known = self.state.borrow().texts.contains_key(&p);
                    if !known {
                        if let Some(w) = &self.worker {
                            w.page(p);
                        }
                        // text arrives with the next poll; try again from the script
                        eprintln!("pdf_select: text of page {} not loaded yet", p + 1);
                        return;
                    }
                    let hit = self
                        .state
                        .borrow()
                        .texts
                        .get(&p)
                        .and_then(|t| t.find(arg).into_iter().next());
                    if let Some(r) = hit {
                        self.goto_page(p, None);
                        self.set_selection(Some(Sel {
                            page: p,
                            start: r.start,
                            end: r.end,
                            highlight: None,
                        }));
                        self.show_popover();
                        eprintln!(
                            "pdf_select: page {} text={:?}",
                            p + 1,
                            self.selection.as_ref().map(|s| s.text.clone())
                        );
                        return;
                    }
                }
                eprintln!("pdf_select: not found");
            }
            "pdf-highlight" => sender.input(Input::Highlight(if arg.is_empty() {
                "yellow".into()
            } else {
                arg.into()
            })),
            "pdf-style" => {
                if let Some(s) = HighlightStyle::from_name(arg) {
                    sender.input(Input::HlStyle(s));
                }
            }
            "pdf-note" => sender.input(Input::SaveNote(arg.to_string())),
            "pdf-copy" => sender.input(Input::Copy),
            "pdf-dict" => sender.input(Input::Dict),
            "pdf-clear" => sender.input(Input::SelectionKey("clear")),
            "pdf-search" => {
                self.w.search_entry.set_text(arg);
                sender.input(Input::Search(arg.to_string()));
            }
            "pdf-search-next" => sender.input(Input::SearchStep(true)),
            "pdf-search-prev" => sender.input(Input::SearchStep(false)),
            "pdf-search-open" => self.w.search_btn.popup(),
            "pdf-search-state" => {
                eprintln!(
                    "pdf_search: id={} done={} hits={} active={:?} first={:?}",
                    self.search_id,
                    self.search_done,
                    self.search_rows.len(),
                    self.search_active,
                    self.search_rows.first().map(|r| (
                        r.page + 1,
                        r.snippet.clone(),
                        r.mark.clone()
                    ))
                );
            }
            "pdf-bookmark" => sender.input(Input::ToggleBookmark),
            "pdf-sidebar" => self.w.split.set_show_sidebar(true),
            "pdf-tab" => {
                let t: &'static str = match arg {
                    "pages" => "pages",
                    "bookmarks" => "bookmarks",
                    "notes" => "notes",
                    _ => "toc",
                };
                sender.input(Input::ShowTab(t));
            }
            "pdf-link" => {
                let i: usize = arg.parse().unwrap_or(0);
                let cur = self.current_page();
                let l = self
                    .state
                    .borrow()
                    .links
                    .get(&cur)
                    .and_then(|l| l.get(i).cloned());
                match l {
                    // an address is reported, never opened, so that tests do not launch a browser
                    Some(Link {
                        target: LinkTarget::Uri(u),
                        ..
                    }) => eprintln!("pdf_link: uri {u}"),
                    Some(l) => self.activate_link(&l),
                    None => eprintln!("pdf_link: none"),
                }
            }
            "pdf-back" => sender.input(Input::Back),
            "pdf-password" => sender.input(Input::Password(Some(arg.to_string()))),
            // `reader:menu-dump` and `reader:header-dump` as in the e-book reader
            "menu-dump" => {
                let mut list = Vec::new();
                let (mut n, mut sections) = (0, 0);
                if let Some(m) = self.w.primary.menu_model() {
                    for i in 0..m.n_items() {
                        if let Some(sec) = m.item_link(i, "section") {
                            sections += 1;
                            for j in 0..sec.n_items() {
                                n += 1;
                                let l = sec
                                    .item_attribute_value(j, "label", None)
                                    .and_then(|v| v.str().map(String::from))
                                    .unwrap_or_default();
                                let a = sec
                                    .item_attribute_value(j, "action", None)
                                    .and_then(|v| v.str().map(String::from))
                                    .unwrap_or_default();
                                list.push(format!("{} [{a}]", l.replace('_', "")));
                            }
                        }
                    }
                }
                eprintln!(
                    "reader_menu: entries={n} sections={sections} submenus=0 narrow={} | {}",
                    self.w.narrow.get(),
                    list.join(" | ")
                );
            }
            "header-dump" => {
                let hw = self.w.header.width() as f64;
                let (mut shown, mut clipped) = (0, 0);
                super::reader_comic::header_buttons(
                    self.w.header.upcast_ref(),
                    self.w.header.upcast_ref(),
                    hw,
                    &mut shown,
                    &mut clipped,
                );
                eprintln!(
                    "reader_header: width={hw:.0} title={} buttons={shown} clipped={clipped} fullscreen={} narrow={}",
                    self.w.title.width(),
                    self.w.fullscreen_btn.is_visible(),
                    self.w.narrow.get()
                );
            }
            "pdf-reflow" => sender.input(Input::Reflow),
            "pdf-fullscreen" => sender.input(Input::Fullscreen),
            "pdf-thumbs" => {
                eprintln!(
                    "pdf_thumbs: shown={} rendered={}",
                    self.w.thumb_pics.borrow().len(),
                    self.w.thumb_tex.borrow().len()
                );
            }
            "pdf-lists" => {
                let rows = |l: &gtk::ListBox| {
                    let mut n = 0;
                    let mut c = l.first_child();
                    while let Some(w) = c {
                        n += 1;
                        c = w.next_sibling();
                    }
                    n
                };
                eprintln!(
                    "pdf_lists: toc={} bookmarks={} notes={} page_entry={:?} caption={:?}",
                    rows(&self.w.toc_list),
                    rows(&self.w.bm_list),
                    rows(&self.w.hl_list),
                    self.w.page_entry.text().to_string(),
                    self.w.caption.label().to_string()
                );
            }
            "live-state" => {
                let lib = self.shared.lib.borrow();
                let (h, n, b, cols) =
                    lib.get(self.book_id)
                        .map_or((0, 0, 0, String::new()), |bk| {
                            let mut cols: Vec<String> =
                                bk.highlights.iter().map(|h| h.color.clone()).collect();
                            cols.sort();
                            (
                                bk.highlights.len(),
                                bk.highlights.iter().filter(|h| !h.note.is_empty()).count(),
                                bk.bookmarks.len(),
                                cols.join("+"),
                            )
                        });
                let painted = self.state.borrow().highlights.len();
                let (heard, repaints) = self.sync_stats.get();
                eprintln!(
                    "live_state: highlights={h} notes={n} bookmarks={b} colours={cols} painted={painted} events={heard} repaints={repaints} banner={}",
                    self.sync_banner.dev_state()
                );
            }
            "live-press" => self.sync_banner.dev_press(arg != "secondary"),
            "pdf-highlights" => {
                let lib = self.shared.lib.borrow();
                let hs: Vec<String> = lib
                    .get(self.book_id)
                    .map(|b| {
                        b.highlights
                            .iter()
                            .filter(|h| pdf::is_pdf(h))
                            .map(|h| {
                                format!(
                                    "p{}:{:?}:{}:{}",
                                    h.spine + 1,
                                    h.text,
                                    h.note,
                                    pdf::parse_pdf_style(&h.style)
                                        .map(|x| x.1.len())
                                        .unwrap_or(0)
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                eprintln!("pdf_highlights: {}", hs.join(" | "));
            }
            "pdf-pixel" => {
                // pdf-pixel=x,y: is there ink at this window position (0/1) — a draw check
                eprintln!(
                    "pdf_pixel: area={}x{}",
                    self.w.area.width(),
                    self.w.area.height()
                );
            }
            _ => {}
        }
    }
}
