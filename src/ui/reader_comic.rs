//! The comic reader: CBZ/CBT/CBR books as pictures (single page, two-page spread, vertical
//! webtoon strip), smart zoom to the panel under the pointer, guided view that steps through the
//! panels, manga direction and a strip of page thumbnails.
//!
//! Pages are decoded on worker threads (`bookx-comic`), reduced to a size that fits the memory, and
//! kept in a cache with a byte budget. Everything that can be decided without a window (layout,
//! panels, camera maths, preload order) is in the `bookx-comic` crate and unit tested there; this
//! file draws, handles input and keeps the settings.

use crate::i18n::{ntr, tr, trf};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use bookx_comic::cache::{PageCache, preload_order};
use bookx_comic::decode::Opts as DecodeOpts;
use bookx_comic::layout::{self, Placed, SpreadOpts, Turn};
use bookx_comic::meta;
use bookx_comic::panels::{self, Rect, Step};
use bookx_comic::settings::{Effective, Fit, Mode, Store};
use bookx_comic::smart::{self, Stop};
use bookx_comic::view::{self, Cam, Frame};
use bookx_comic::worker::{Done, Job, Source, Worker};
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, cairo, gdk, gio, glib};
use relm4::prelude::*;

use super::{SharedRef, window_of};
use crate::comic::ComicBook;
use crate::core::library::ReadPos;

/// Budget of the decoded pages.
const CACHE_BYTES: usize = 320 << 20;
/// Thumbnails: longest side in pixels, and the height of a row of the strip.
const THUMB_EDGE: u32 = 240;
const ROW_H: i32 = 176;
/// Margin around a panel when it is zoomed to (share of the window).
const PANEL_MARGIN: f64 = 0.03;
/// Pages decoded ahead of the reader.
const AHEAD: usize = 3;

struct Surf {
    surface: cairo::ImageSurface,
    w: f64,
    h: f64,
    panels: Vec<Rect>,
    bytes: usize,
    grey: bool,
}

struct Anim {
    from: Cam,
    to: Cam,
    start: Instant,
    ms: u32,
}

/// What the drawing function and the input handlers share.
struct View {
    placed: Vec<Placed>,
    content: (f64, f64),
    cam: Cam,
    vw: f64,
    vh: f64,
    cache: PageCache<Rc<Surf>>,
    failed: HashSet<usize>,
    anim: Option<Anim>,
    anim_gen: u64,
    dark: bool,
    /// The panel of guided view, in content units.
    outline: Option<[f64; 4]>,
    /// How many times the page area was drawn and how long the last one took.
    draws: Cell<u64>,
    last_draw_ms: Cell<f64>,
}

impl View {
    fn frame(&self) -> Frame {
        Frame {
            cw: self.content.0,
            ch: self.content.1,
            vw: self.vw,
            vh: self.vh,
        }
    }

    fn panels_of(&self, page: usize) -> Vec<Rect> {
        self.cache
            .peek(page)
            .map(|s| s.panels.clone())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug)]
pub enum Input {
    Poll,
    /// A device reported a position (see `sync_bus`).
    Sync(crate::sync_bus::SyncEvent),
    /// A button of the sync banner.
    LiveAnswer(super::reader_live::Answer),
    Resize(i32, i32),
    Turn(Turn),
    Arrow(Dir),
    Space(bool),
    GoPage(usize),
    /// A click on a thumbnail: go there and give the keyboard back to the page.
    ThumbGo(usize),
    Seek(f64),
    Wheel {
        dx: f64,
        dy: f64,
        ctrl: bool,
        x: f64,
        y: f64,
    },
    Pinch(f64, f64, f64),
    ZoomKey(f64),
    ZoomReset,
    DragBegin,
    DragUpdate(f64, f64),
    DragEnd(f64, f64),
    Click(i32, f64, f64),
    SingleClick(f64, f64),
    SetMode(Mode),
    CycleMode,
    SetFit(Fit),
    SetRtl(bool),
    SetGuided(bool),
    SetGrey(bool),
    SetCoverAlone(bool),
    SetWideAlone(bool),
    ResetBook,
    /// Go to Page (Ctrl+G / Ctrl+L, Main Menu).
    GotoDialog,
    /// Properties (Main Menu).
    Properties,
    SidebarShown,
    ThumbScroll,
    Fullscreen,
    Quit,
    Dev(String),
    /// Open the previous (`false`) or next volume of the series.
    Volume(bool),
}

#[derive(Debug)]
pub enum Output {
    Close,
    /// Close this volume and open another one; `last_page` starts it at its end.
    OpenVolume {
        id: u64,
        last_page: bool,
    },
    #[allow(dead_code)]
    Toast(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Pending {
    First,
    Last,
}

struct W {
    page: adw::NavigationPage,
    area: gtk::DrawingArea,
    title: adw::WindowTitle,
    slider: gtk::Scale,
    prev_btn: gtk::Button,
    next_btn: gtk::Button,
    label: gtk::Label,
    split: adw::OverlaySplitView,
    toolbar: adw::ToolbarView,
    thumb_list: gtk::ListBox,
    thumb_scroll: gtk::ScrolledWindow,
    thumb_pics: Vec<gtk::Picture>,
    modes: Vec<(Mode, gtk::CheckButton)>,
    fits: Vec<(Fit, gtk::CheckButton)>,
    rtl: gtk::CheckButton,
    guided: gtk::CheckButton,
    cover: gtk::CheckButton,
    wide: gtk::CheckButton,
    grey: gtk::CheckButton,
    reset: gtk::Button,
    primary: gtk::MenuButton,
    header: adw::HeaderBar,
    view_pop: gtk::Popover,
    narrow: Rc<Cell<bool>>,
    fullscreen_btn: gtk::Button,
}

pub struct ReaderComic {
    shared: SharedRef,
    book_id: u64,
    title: String,
    book_rtl: bool,
    library_key: String,
    /// The previous and next volume of the series, and the end of the book that was just reached
    /// (a second push on the same side opens the volume).
    volumes: [Option<(u64, String)>; 2],
    edge_armed: Option<(bool, Instant)>,
    worker: Worker,
    sizes: Vec<(u32, u32)>,
    double: Vec<bool>,
    store: Store,
    eff: Effective,
    plan: Vec<Vec<usize>>,
    /// The first page on screen (in a webtoon: the page in view).
    page: usize,
    /// A device that is further along is offered here (see `sync_live`).
    sync_banner: Rc<super::reader_live::LiveBanner>,
    _sync_sub: Option<crate::sync_bus::Subscription>,
    seen_page: Cell<usize>,
    /// The stop of guided view the camera is on.
    stop: Option<usize>,
    pending: Option<Pending>,
    forward: bool,
    r#gen: u64,
    view: Rc<RefCell<View>>,
    w: W,
    sender: ComponentSender<Self>,
    dirty: Rc<Cell<bool>>,
    timer: Option<glib::SourceId>,
    poll: Option<glib::SourceId>,
    click_timer: Option<glib::SourceId>,
    drag_from: Option<Cam>,
    last_turn: Instant,
    updating_ui: Rc<Cell<bool>>,
    thumbs_done: HashSet<usize>,
    laid_out: bool,
    last_visible: Vec<usize>,
    last_eink: bool,
    animate_override: Option<bool>,
    alive: Rc<Cell<bool>>,
}

fn icon_button(icon: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(tip));
    b.set_focus_on_click(false);
    super::audit::label(&b, tip);
    b
}

fn surf_of(page: bookx_comic::decode::Page) -> Option<Surf> {
    let (w, h) = (page.width as i32, page.height as i32);
    let bytes = page.bytes();
    let panels = page.panels.clone();
    let grey = page
        .rgba
        .chunks_exact(4)
        .step_by(61)
        .all(|p| p[0] == p[1] && p[1] == p[2]);
    let surface =
        cairo::ImageSurface::create_for_data(page.rgba, cairo::Format::ARgb32, w, h, w * 4).ok()?;
    Some(Surf {
        surface,
        w: w as f64,
        h: h as f64,
        panels,
        bytes,
        grey,
    })
}

/// The fit that applies in a mode: a strip is as wide as the window unless it is shown at its
/// own size.
fn mode_fit(mode: Mode, fit: Fit) -> Fit {
    match (mode, fit) {
        (Mode::Webtoon, Fit::Original) => Fit::Original,
        (Mode::Webtoon, _) => Fit::Width,
        (_, f) => f,
    }
}

/// What an empty page says: its number while it loads, "This page cannot be shown" when it is damaged.
fn placeholder_text(failed: bool, page: usize) -> String {
    if failed {
        tr("This page cannot be shown")
    } else {
        trf("Page {}", &[&(page + 1)])
    }
}

fn draw(v: &View, cr: &cairo::Context, w: i32, h: i32) {
    let bg = if crate::eink::is_active() {
        (1.0, 1.0, 1.0)
    } else if v.dark {
        (0.11, 0.11, 0.12)
    } else {
        (0.88, 0.88, 0.89)
    };
    cr.set_source_rgb(bg.0, bg.1, bg.2);
    let _ = cr.paint();
    let cam = v.cam;
    for p in &v.placed {
        let (x, y) = cam.to_view(p.x, p.y);
        let (pw, ph) = (p.w * cam.s, p.h * cam.s);
        if x + pw < 0.0 || y + ph < 0.0 || x > w as f64 || y > h as f64 {
            continue;
        }
        match v.cache.peek(p.page) {
            Some(s) => {
                let _ = cr.save();
                cr.translate(x, y);
                cr.scale(pw / s.w, ph / s.h);
                let _ = cr.set_source_surface(&s.surface, 0.0, 0.0);
                let pat = cr.source();
                pat.set_extend(cairo::Extend::Pad);
                pat.set_filter(if pw < s.w {
                    cairo::Filter::Good
                } else {
                    cairo::Filter::Bilinear
                });
                cr.rectangle(0.0, 0.0, s.w, s.h);
                let _ = cr.fill();
                let _ = cr.restore();
            }
            None => {
                let paper = if v.dark { 0.17 } else { 0.97 };
                cr.set_source_rgb(paper, paper, paper);
                cr.rectangle(x, y, pw, ph);
                let _ = cr.fill();
                let ink = if v.dark { 0.7 } else { 0.35 };
                cr.set_source_rgb(ink, ink, ink);
                cr.set_font_size(16.0);
                let text = placeholder_text(v.failed.contains(&p.page), p.page);
                let ext = cr.text_extents(&text).ok();
                let tw = ext.map(|e| e.width()).unwrap_or(60.0);
                let (tx, ty) = (
                    (x + pw / 2.0 - tw / 2.0)
                        .max(x + 4.0)
                        .min(w as f64 - tw - 4.0)
                        .max(4.0),
                    (y + ph / 2.0).clamp(24.0, h as f64 - 12.0),
                );
                cr.move_to(tx, ty);
                let _ = cr.show_text(&text);
            }
        }
    }
    if let Some(r) = v.outline {
        let (x, y) = cam.to_view(r[0], r[1]);
        cr.set_source_rgba(0.21, 0.52, 0.89, 0.9);
        cr.set_line_width(2.0);
        cr.rectangle(x, y, r[2] * cam.s, r[3] * cam.s);
        let _ = cr.stroke();
    }
}

fn key_input(key: gdk::Key, ctrl: bool, shift: bool) -> Option<Input> {
    use gdk::Key;
    Some(match key {
        Key::Right => Input::Arrow(Dir::Right),
        Key::Left => Input::Arrow(Dir::Left),
        Key::Up => Input::Arrow(Dir::Up),
        Key::Down => Input::Arrow(Dir::Down),
        Key::Page_Down if ctrl => Input::Volume(true),
        Key::Page_Up if ctrl => Input::Volume(false),
        Key::Page_Down => Input::Turn(Turn::Next),
        Key::Page_Up => Input::Turn(Turn::Prev),
        Key::space => Input::Space(shift),
        Key::BackSpace => Input::Space(true),
        Key::Home => Input::GoPage(0),
        Key::End => Input::GoPage(usize::MAX),
        Key::plus | Key::equal | Key::KP_Add => Input::ZoomKey(1.25),
        Key::minus | Key::KP_Subtract => Input::ZoomKey(0.8),
        Key::_0 | Key::KP_0 => Input::ZoomReset,
        Key::g | Key::l if ctrl => Input::GotoDialog,
        Key::g if !ctrl => Input::Dev("toggle-guided".into()),
        Key::m if !ctrl => Input::CycleMode,
        Key::r if !ctrl => Input::Dev("toggle-rtl".into()),
        Key::t | Key::F9 => Input::Dev("toggle-sidebar".into()),
        Key::F11 => Input::Fullscreen,
        Key::Escape => Input::Quit,
        _ => return None,
    })
}

impl ReaderComic {
    fn n(&self) -> usize {
        self.sizes.len()
    }

    fn eink(&self) -> bool {
        crate::eink::is_active()
    }

    fn grey(&self) -> bool {
        self.store.global.grey || self.eink()
    }

    fn animate_on(&self) -> bool {
        self.animate_override.unwrap_or(self.store.global.animate)
            && !self.eink()
            && gtk::Settings::default().is_some_and(|s| s.is_gtk_enable_animations())
    }

    fn spread_opts(&self) -> SpreadOpts {
        SpreadOpts {
            two_page: self.eff.mode == Mode::Spread,
            cover_alone: self.store.global.cover_alone,
            wide_alone: self.store.global.wide_alone,
        }
    }

    fn replan(&mut self) {
        self.plan = layout::plan_spreads(&self.sizes, &self.double, self.spread_opts());
    }

    fn screen(&self) -> usize {
        layout::spread_of(&self.plan, self.page)
    }

    fn decode_opts(&self) -> DecodeOpts {
        DecodeOpts {
            max_edge: if self.eink() { 2400 } else { 4096 },
            max_pixels: 12_000_000,
            grey: self.grey(),
            panels: Some(self.eff.rtl),
        }
    }

    /// The pages on screen.
    fn visible_pages(&self) -> Vec<usize> {
        if self.eff.mode == Mode::Webtoon {
            let v = self.view.borrow();
            let top = -v.cam.y / v.cam.s;
            let bottom = (v.vh - v.cam.y) / v.cam.s;
            layout::visible_in_strip(&v.placed, top, bottom).collect()
        } else {
            self.plan.get(self.screen()).cloned().unwrap_or_default()
        }
    }

    // ---- scene and camera ---------------------------------------------------------------

    fn rebuild_scene(&mut self) {
        let (placed, content) = if self.eff.mode == Mode::Webtoon {
            layout::strip(&self.sizes)
        } else {
            let pages = self.plan.get(self.screen()).cloned().unwrap_or_default();
            layout::place_spread(&self.sizes, &pages, self.eff.rtl)
        };
        let mut v = self.view.borrow_mut();
        v.placed = placed;
        v.content = content;
        v.outline = None;
    }

    fn fit(&self) -> Fit {
        mode_fit(self.eff.mode, self.eff.fit)
    }

    fn set_cam(&mut self, cam: Cam, animate: bool) {
        let ms = view::duration_ms(animate && self.animate_on());
        let mut v = self.view.borrow_mut();
        v.anim_gen += 1;
        let r#gen = v.anim_gen;
        let cam = view::clamp(cam, v.frame());
        if ms == 0
            || (cam.s - v.cam.s).abs() + (cam.x - v.cam.x).abs() + (cam.y - v.cam.y).abs() < 0.5
        {
            v.cam = cam;
            v.anim = None;
            drop(v);
            self.w.area.queue_draw();
            return;
        }
        v.anim = Some(Anim {
            from: v.cam,
            to: cam,
            start: Instant::now(),
            ms,
        });
        drop(v);
        let view = self.view.clone();
        self.w.area.add_tick_callback(move |area, _| {
            let mut v = view.borrow_mut();
            if v.anim_gen != r#gen {
                return glib::ControlFlow::Break;
            }
            let Some(a) = &v.anim else {
                return glib::ControlFlow::Break;
            };
            let t = a.start.elapsed().as_secs_f64() * 1000.0 / a.ms as f64;
            let (from, to) = (a.from, a.to);
            let frame = v.frame();
            v.cam = view::tween(from, to, view::ease_out(t), frame);
            let done = t >= 1.0;
            if done {
                v.anim = None;
            }
            drop(v);
            area.queue_draw();
            if done {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    fn fitted(&self) -> bool {
        let v = self.view.borrow();
        view::is_fitted(v.cam, v.frame(), self.fit())
    }

    /// The camera at the start of what is on screen.
    fn start_cam(&self, end: bool) -> Cam {
        let v = self.view.borrow();
        let f = v.frame();
        let mut cam = view::place(f, self.fit(), self.eff.rtl);
        if self.eff.mode == Mode::Webtoon {
            let y = v.placed.get(self.page).map(|p| p.y).unwrap_or(0.0);
            cam.y = -y * cam.s;
        } else if end {
            cam.y = -1e9;
        }
        cam
    }

    /// Shows a page (the screen it is on) at its start, or at its end.
    fn show(&mut self, page: usize, end: bool) {
        if self.n() == 0 {
            return;
        }
        self.page = page.min(self.n() - 1);
        if self.seen_page.replace(self.page) != self.page {
            self.sync_banner.page_turned();
        }
        if self.eff.mode != Mode::Webtoon {
            self.page = self
                .plan
                .get(self.screen())
                .map(|s| s[0])
                .unwrap_or(self.page);
        }
        self.stop = None;
        self.rebuild_scene();
        let cam = self.start_cam(end);
        {
            let mut v = self.view.borrow_mut();
            v.anim_gen += 1;
            v.anim = None;
            v.cam = view::clamp(cam, v.frame());
        }
        self.after_move();
        self.apply_pending();
    }

    /// After the camera or the page changed: decode what is needed, update the controls, save.
    fn after_move(&mut self) {
        if self.eff.mode == Mode::Webtoon {
            let (top, bottom) = {
                let v = self.view.borrow();
                (-v.cam.y / v.cam.s, (v.vh - v.cam.y) / v.cam.s)
            };
            let cur = {
                let v = self.view.borrow();
                layout::strip_current(&v.placed, top, bottom)
            };
            self.page = cur;
        }
        self.request_pages();
        self.update_ui();
        self.save_progress();
        self.w.area.queue_draw();
    }

    fn request_pages(&mut self) {
        if self.n() == 0 {
            return;
        }
        let visible = self.visible_pages();
        let order: Vec<usize> = if self.eff.mode == Mode::Webtoon {
            preload_order(&visible, self.n(), self.forward, AHEAD)
        } else {
            let screens = preload_order(&[self.screen()], self.plan.len(), self.forward, AHEAD);
            screens
                .iter()
                .flat_map(|s| self.plan[*s].iter().copied())
                .collect()
        };
        let tag = self.r#gen << 1;
        let opts = self.decode_opts();
        let jobs: Vec<Job> = {
            let v = self.view.borrow();
            order
                .iter()
                .filter(|p| !v.cache.contains(**p) && !v.failed.contains(*p))
                .map(|p| Job {
                    page: *p,
                    tag,
                    opts,
                })
                .collect()
        };
        let keep: Vec<usize> = order.iter().take(10).copied().collect();
        {
            let mut v = self.view.borrow_mut();
            for p in &visible {
                let _ = v.cache.get(*p);
            }
            v.cache.trim(&keep);
        }
        if !jobs.is_empty() {
            self.worker.request(jobs);
        }
    }

    fn on_decoded(&mut self, d: Done) {
        if d.tag & 1 == 1 {
            self.on_thumb(d);
            return;
        }
        if d.tag >> 1 != self.r#gen {
            return;
        }
        match d.result.map(|p| p.into_bgra()).map(surf_of) {
            Ok(Some(s)) => {
                let bytes = s.bytes;
                {
                    let mut v = self.view.borrow_mut();
                    v.failed.remove(&d.page);
                    v.cache.insert(d.page, Rc::new(s), bytes);
                }
                self.w.area.queue_draw();
                if self.visible_pages().contains(&d.page) {
                    self.apply_pending();
                }
            }
            _ => {
                self.view.borrow_mut().failed.insert(d.page);
                self.w.area.queue_draw();
                self.apply_pending();
            }
        }
    }

    fn on_thumb(&mut self, d: Done) {
        if d.tag >> 1 != self.r#gen {
            return;
        }
        self.thumbs_done.insert(d.page);
        let Ok(page) = d.result.map(|p| p.into_bgra()) else {
            return;
        };
        if let Some(pic) = self.w.thumb_pics.get(d.page) {
            let tex = gdk::MemoryTexture::new(
                page.width as i32,
                page.height as i32,
                gdk::MemoryFormat::B8g8r8a8Premultiplied,
                &glib::Bytes::from(&page.rgba[..]),
                page.width as usize * 4,
            );
            pic.set_paintable(Some(&tex));
        }
    }

    fn request_thumbs(&mut self) {
        if !self.w.split.shows_sidebar() || self.n() == 0 {
            return;
        }
        let adj = self.w.thumb_scroll.vadjustment();
        let first = (adj.value() / ROW_H as f64).floor().max(0.0) as usize;
        let last =
            ((adj.value() + adj.page_size().max(ROW_H as f64)) / ROW_H as f64).ceil() as usize;
        let lo = first.saturating_sub(3);
        let hi = (last + 3).min(self.n());
        let mid = (first + last) / 2;
        let mut want: Vec<usize> = (lo..hi).filter(|p| !self.thumbs_done.contains(p)).collect();
        want.sort_by_key(|p| p.abs_diff(mid));
        if want.is_empty() {
            return;
        }
        let opts = DecodeOpts {
            max_edge: THUMB_EDGE,
            max_pixels: 1_000_000,
            grey: self.grey(),
            panels: None,
        };
        let tag = (self.r#gen << 1) | 1;
        self.worker.request_thumbs(
            want.into_iter()
                .map(|p| Job { page: p, tag, opts })
                .collect(),
        );
    }

    // ---- guided view and smart zoom --------------------------------------------------------

    /// The stops of the screen, when its pages are decoded.
    fn stops(&self) -> Option<Vec<Stop>> {
        let v = self.view.borrow();
        let ready = v
            .placed
            .iter()
            .filter(|p| {
                // in a webtoon only the pages in view count
                self.eff.mode != Mode::Webtoon || self.visible_pages_in(&v).contains(&p.page)
            })
            .all(|p| v.cache.contains(p.page) || v.failed.contains(&p.page));
        if !ready {
            return None;
        }
        let placed: Vec<Placed> = if self.eff.mode == Mode::Webtoon {
            let vis = self.visible_pages_in(&v);
            v.placed
                .iter()
                .filter(|p| vis.contains(&p.page))
                .copied()
                .collect()
        } else {
            v.placed.clone()
        };
        // reading order: the first page of the screen first
        let mut placed = placed;
        if self.eff.mode == Mode::Spread {
            placed.sort_by_key(|p| p.page);
        }
        Some(smart::stops(&placed, &|p| v.panels_of(p)))
    }

    fn visible_pages_in(&self, v: &View) -> Vec<usize> {
        let top = -v.cam.y / v.cam.s;
        let bottom = (v.vh - v.cam.y) / v.cam.s;
        layout::visible_in_strip(&v.placed, top, bottom).collect()
    }

    fn goto_stop(&mut self, i: usize, stops: &[Stop]) {
        let rect = stops[i].rect;
        self.stop = Some(i);
        let cam = {
            let mut v = self.view.borrow_mut();
            v.outline = self.eff.guided.then_some(rect);
            view::focus(rect, PANEL_MARGIN, v.frame())
        };
        self.set_cam(cam, true);
        self.update_ui();
    }

    fn apply_pending(&mut self) {
        let Some(p) = self.pending else { return };
        let Some(stops) = self.stops() else { return };
        self.pending = None;
        if stops.is_empty() {
            return;
        }
        let i = match p {
            Pending::First => 0,
            Pending::Last => stops.len() - 1,
        };
        self.goto_stop(i, &stops);
    }

    /// Guided view: the next or previous stop (panel) in reading order.
    fn step_panel(&mut self, forward: bool) {
        self.forward = forward;
        let Some(stops) = self.stops() else {
            self.pending = Some(if forward {
                Pending::First
            } else {
                Pending::Last
            });
            self.request_pages();
            return;
        };
        match panels::step(stops.len(), self.stop, forward) {
            Step::Panel(i) if i < stops.len() => self.goto_stop(i, &stops),
            Step::Panel(_) => {}
            Step::NextPage | Step::PrevPage => {
                if self.eff.mode == Mode::Webtoon {
                    self.scroll_screen(forward);
                    return;
                }
                let t = if forward { Turn::Next } else { Turn::Prev };
                if let Some(s) = layout::turned(self.screen(), self.plan.len(), t) {
                    let first = self.plan[s][0];
                    self.show(first, false);
                    self.pending = Some(if forward {
                        Pending::First
                    } else {
                        Pending::Last
                    });
                    self.apply_pending();
                }
            }
        }
    }

    fn smart_zoom(&mut self, x: f64, y: f64) {
        if self.n() == 0 {
            return;
        }
        if !self.fitted() || self.stop.is_some() {
            self.stop = None;
            self.view.borrow_mut().outline = None;
            let cam = self.refit_cam();
            self.set_cam(cam, true);
            self.update_ui();
            return;
        }
        let target = {
            let v = self.view.borrow();
            let (cx, cy) = v.cam.to_content(x, y);
            smart::zoom_target(&v.placed, &|p| v.panels_of(p), cx, cy)
        };
        let Some(t) = target else { return };
        let cam = {
            let mut v = self.view.borrow_mut();
            v.outline = self.eff.guided.then_some(t.rect);
            view::focus(t.rect, PANEL_MARGIN, v.frame())
        };
        self.stop = t.panel.and_then(|_| {
            self.stops()
                .and_then(|s| smart::index_of(&s, t.page, t.panel))
        });
        self.set_cam(cam, true);
        self.update_ui();
    }

    /// The camera that fits what is on screen, keeping the place in a webtoon.
    fn refit_cam(&self) -> Cam {
        if self.eff.mode == Mode::Webtoon {
            let v = self.view.borrow();
            let f = v.frame();
            let top = -v.cam.y / v.cam.s;
            let mut c = view::place(f, self.fit(), false);
            c.y = -top * c.s;
            c
        } else {
            self.start_cam(false)
        }
    }

    // ---- moving -----------------------------------------------------------------------------

    fn scroll_screen(&mut self, forward: bool) {
        let (cam, vh) = {
            let v = self.view.borrow();
            (v.cam, v.vh)
        };
        let d = vh * 0.9 * if forward { -1.0 } else { 1.0 };
        let c = view::pan(cam, 0.0, d, self.view.borrow().frame());
        self.set_cam(c, true);
        self.after_move_soon();
    }

    /// Webtoon: the page in view follows a scroll that is still animating.
    fn after_move_soon(&mut self) {
        let tx = self.sender.input_sender().clone();
        glib::timeout_add_local_once(Duration::from_millis(260), move || tx.emit(Input::Poll));
        self.after_move();
    }

    fn turn(&mut self, t: Turn) {
        self.forward = t == Turn::Next;
        if self.eff.mode == Mode::Webtoon {
            self.scroll_screen(t == Turn::Next);
            return;
        }
        if let Some(s) = layout::turned(self.screen(), self.plan.len(), t) {
            self.show(self.plan[s][0], false);
        } else {
            self.edge_hit(t == Turn::Next);
        }
    }

    /// Opens the neighbouring volume; going back starts it at its last page.
    fn open_volume(&mut self, forward: bool) {
        let v = self.volumes[forward as usize].clone();
        match v {
            Some((id, _)) => {
                let _ = self.sender.output(Output::OpenVolume {
                    id,
                    last_page: !forward,
                });
            }
            None => {
                let msg = if forward {
                    tr("This is the last volume of the series")
                } else {
                    tr("This is the first volume of the series")
                };
                let _ = self.sender.output(Output::Toast(msg));
            }
        }
    }

    /// Pushing past the first or last page: says which volume is next, and a second push within a
    /// few seconds opens it.
    fn edge_hit(&mut self, forward: bool) {
        let Some((_, title)) = self.volumes[forward as usize].clone() else {
            return;
        };
        if self
            .edge_armed
            .is_some_and(|(f, at)| f == forward && at.elapsed() < Duration::from_secs(6))
        {
            self.edge_armed = None;
            self.open_volume(forward);
            return;
        }
        self.edge_armed = Some((forward, Instant::now()));
        let msg = if forward {
            trf(
                "End of the volume. Turn again for the next volume: {}",
                &[&title],
            )
        } else {
            trf(
                "Start of the volume. Turn again for the previous volume: {}",
                &[&title],
            )
        };
        let _ = self.sender.output(Output::Toast(msg));
    }

    /// An arrow key: moves the picture when it can move that way, otherwise turns the page.
    fn arrow(&mut self, d: Dir) {
        let (cam, f) = {
            let v = self.view.borrow();
            (v.cam, v.frame())
        };
        let step_x = f.vw * 0.18;
        let step_y = f.vh * 0.18;
        match d {
            Dir::Left | Dir::Right => {
                let right = d == Dir::Right;
                let wide = f.cw * cam.s > f.vw + 1.0;
                let room = if right {
                    cam.x - (f.vw - f.cw * cam.s)
                } else {
                    -cam.x
                };
                if wide && room > 1.0 {
                    let c = view::pan(cam, if right { -step_x } else { step_x }, 0.0, f);
                    self.set_cam(c, false);
                    self.after_move();
                } else if self.eff.mode != Mode::Webtoon {
                    self.turn(layout::horizontal_turn(self.eff.rtl, right));
                }
            }
            Dir::Up | Dir::Down => {
                let down = d == Dir::Down;
                let room = view::scroll_room(cam, f, down);
                if room > 0.5 {
                    let c = view::pan(cam, 0.0, if down { -step_y } else { step_y }, f);
                    self.set_cam(c, false);
                    self.after_move();
                } else if self.eff.mode != Mode::Webtoon {
                    self.turn_edge(down);
                }
            }
        }
    }

    /// Turns the page; going back shows the end of the page before.
    fn turn_edge(&mut self, forward: bool) {
        self.forward = forward;
        let t = if forward { Turn::Next } else { Turn::Prev };
        if let Some(s) = layout::turned(self.screen(), self.plan.len(), t) {
            let first = self.plan[s][0];
            self.show(first, !forward);
        } else {
            self.edge_hit(forward);
        }
    }

    /// Space: guided view steps through the panels, otherwise it reads on: a screenful further
    /// down the page, then the next page.
    fn space(&mut self, back: bool) {
        if self.eff.guided && self.eff.mode != Mode::Webtoon {
            self.step_panel(!back);
            return;
        }
        let (cam, f) = {
            let v = self.view.borrow();
            (v.cam, v.frame())
        };
        let room = view::scroll_room(cam, f, !back);
        if room > 0.5 {
            let c = view::pan(cam, 0.0, f.vh * 0.9 * if back { 1.0 } else { -1.0 }, f);
            self.set_cam(c, true);
            self.after_move_soon();
        } else if self.eff.mode != Mode::Webtoon {
            self.turn_edge(!back);
        }
    }

    fn zoom_by(&mut self, factor: f64, x: f64, y: f64) {
        let cam = {
            let v = self.view.borrow();
            view::zoom_at(v.cam, factor, x, y, v.frame())
        };
        self.stop = None;
        self.view.borrow_mut().outline = None;
        self.set_cam(cam, false);
        self.after_move();
    }

    fn go_page(&mut self, page: usize) {
        self.forward = page >= self.page;
        self.show(page, false);
    }

    // ---- settings ---------------------------------------------------------------------------

    fn persist(&mut self) {
        let mut disk = crate::comic::load();
        disk.global.cover_alone = self.store.global.cover_alone;
        disk.global.wide_alone = self.store.global.wide_alone;
        disk.global.grey = self.store.global.grey;
        disk.global.thumbs = self.store.global.thumbs;
        if let Some(b) = self.store.books.get(&self.book_id.to_string()) {
            *disk.book_mut(self.book_id) = *b;
        } else {
            disk.forget(self.book_id);
        }
        crate::comic::save(&disk);
        self.store = disk;
        self.w.reset.set_visible(self.store.has_own(self.book_id));
    }

    fn refresh_effective(&mut self) {
        self.eff = self
            .store
            .effective_in(self.book_id, self.book_rtl, &self.library_key);
    }

    /// New mode, direction or colours: lay the pages out again at the same page.
    fn relayout(&mut self, redecode: bool) {
        crate::trace::timed(crate::trace::Area::Reader, "comic_relayout", |_o| {
            let page = self.page;
            if redecode {
                self.r#gen += 1;
                let mut v = self.view.borrow_mut();
                v.cache.clear();
                v.failed.clear();
                drop(v);
                self.thumbs_done.clear();
                for pic in &self.w.thumb_pics {
                    pic.set_paintable(gdk::Paintable::NONE);
                }
            }
            self.replan();
            self.pending = None;
            self.show(page, false);
            self.sync_controls();
            self.request_thumbs();
        })
    }

    /// The Go to Page dialog (the field takes a page number).
    fn goto_dialog(&self) {
        let n = self.n();
        let tx = self.sender.input_sender().clone();
        super::reader_goto::show_with(
            self.w.page.upcast_ref(),
            &tr("Page number"),
            &tr("Go to page"),
            &ntr("{} page", "{} pages", n as u64),
            Rc::new(move |t| page_check(t, n)),
            Rc::new(move |t| {
                if let Ok(p) = t.trim().parse::<usize>() {
                    tx.emit(Input::GoPage(p.saturating_sub(1)));
                }
            }),
        );
    }

    /// The Properties dialog: title, author, pages, reading direction.
    fn properties(&self) {
        let author = self
            .shared
            .lib
            .borrow()
            .get(self.book_id)
            .map(|b| b.meta.authors.join(", "))
            .unwrap_or_default();
        let mut lines = vec![trf("Pages: {}", &[&self.n()])];
        if !author.trim().is_empty() {
            lines.insert(0, trf("Author: {}", &[&author]));
        }
        lines.push(if self.eff.rtl {
            tr("Reading direction: right to left")
        } else {
            tr("Reading direction: left to right")
        });
        let d = adw::AlertDialog::new(Some(&self.title), Some(&lines.join("\n")));
        d.add_response("ok", &tr("Close"));
        d.present(Some(&self.w.page));
    }

    fn sync_controls(&self) {
        self.updating_ui.set(true);
        for (m, b) in &self.w.modes {
            b.set_active(*m == self.eff.mode);
        }
        for (f, b) in &self.w.fits {
            b.set_active(*f == self.eff.fit);
        }
        self.w.rtl.set_active(self.eff.rtl);
        self.w.guided.set_active(self.eff.guided);
        self.w.cover.set_active(self.store.global.cover_alone);
        self.w.wide.set_active(self.store.global.wide_alone);
        self.w.cover.set_sensitive(self.eff.mode == Mode::Spread);
        self.w.wide.set_sensitive(self.eff.mode == Mode::Spread);
        self.w.grey.set_active(self.grey());
        self.w.grey.set_sensitive(!self.eink());
        self.w.reset.set_visible(self.store.has_own(self.book_id));
        self.w
            .slider
            .set_inverted(self.eff.rtl && self.eff.mode != Mode::Webtoon);
        self.updating_ui.set(false);
    }

    // ---- controls and progress --------------------------------------------------------------

    fn page_text(&self) -> String {
        let n = self.n();
        let pages = if self.eff.mode == Mode::Webtoon {
            vec![self.page]
        } else {
            self.plan.get(self.screen()).cloned().unwrap_or_default()
        };
        match pages.as_slice() {
            [] => String::new(),
            [p] => trf("Page {} of {}", &[&(p + 1), &n]),
            [a, .., b] => trf("Pages {}–{} of {}", &[&(a + 1), &(b + 1), &n]),
        }
    }

    fn update_ui(&self) {
        let n = self.n();
        let text = self.page_text();
        self.w.label.set_text(&text);
        self.w.title.set_subtitle(&text);
        super::audit::label(&self.w.area, &format!("{}, {}", self.title, text));
        self.updating_ui.set(true);
        self.w.slider.set_value((self.page + 1) as f64);
        self.updating_ui.set(false);
        let (cur, total) = if self.eff.mode == Mode::Webtoon {
            (self.page, n)
        } else {
            (self.screen(), self.plan.len())
        };
        self.w.prev_btn.set_sensitive(cur > 0);
        self.w.next_btn.set_sensitive(cur + 1 < total);
        if let Some(row) = self.w.thumb_list.row_at_index(self.page as i32) {
            if !row.is_selected() {
                self.w.thumb_list.select_row(Some(&row));
            }
        }
    }

    fn save_progress(&self) {
        let n = self.n();
        if n == 0 || !self.laid_out {
            return;
        }
        let page = self.page;
        let p = layout::progress(page, n);
        {
            let mut lib = self.shared.lib.borrow_mut();
            lib.set_progress(
                self.book_id,
                ReadPos {
                    spine: page,
                    frac: 0.0,
                },
                p,
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

    fn on_resize(&mut self, w: i32, h: i32) {
        let (ow, oh) = {
            let v = self.view.borrow();
            (v.vw, v.vh)
        };
        if (ow - w as f64).abs() < 0.5 && (oh - h as f64).abs() < 0.5 && self.laid_out {
            return;
        }
        if w <= 0 || h <= 0 {
            return;
        }
        if !self.laid_out {
            {
                let mut v = self.view.borrow_mut();
                v.vw = w as f64;
                v.vh = h as f64;
            }
            self.laid_out = true;
            let start = self
                .shared
                .lib
                .borrow()
                .get(self.book_id)
                .and_then(|b| b.position.as_ref().map(|p| p.spine))
                .unwrap_or(0);
            let start = if crate::comic::take_last_page(self.book_id) {
                self.n().saturating_sub(1)
            } else {
                start
            };
            self.show(start, false);
            return;
        }
        // keep what is in the middle of the window
        let (old_frame, old_cam) = {
            let v = self.view.borrow();
            (v.frame(), v.cam)
        };
        let was_fitted = view::is_fitted(old_cam, old_frame, self.fit());
        {
            let mut v = self.view.borrow_mut();
            v.vw = w as f64;
            v.vh = h as f64;
        }
        if self.stop.is_some() {
            if let Some(stops) = self.stops() {
                if let Some(i) = self.stop.filter(|i| *i < stops.len()) {
                    self.goto_stop(i, &stops);
                    return;
                }
            }
        }
        let new_frame = self.view.borrow().frame();
        let cam = if was_fitted {
            self.refit_cam()
        } else {
            let (cx, cy) = view::center_of(old_cam, old_frame);
            Cam {
                s: old_cam.s,
                x: new_frame.vw / 2.0 - cx * old_cam.s,
                y: new_frame.vh / 2.0 - cy * old_cam.s,
            }
        };
        self.set_cam(cam, false);
        self.after_move();
    }

    fn sidebar_changed(&mut self) {
        self.request_thumbs();
    }

    fn state_line(&self) -> String {
        let v = self.view.borrow();
        let f = v.frame();
        let (cx, cy) = view::center_of(v.cam, f);
        let panels_here = self
            .visible_pages_in_or_screen(&v)
            .first()
            .map(|p| v.panels_of(*p).len())
            .unwrap_or(0);
        // what part of the first page of the screen shows in the window, in fractions of it
        let shows = v
            .placed
            .iter()
            .find(|p| p.page == self.page)
            .or(v.placed.first())
            .map(|p| {
                let (x0, y0) = v.cam.to_content(0.0, 0.0);
                let (x1, y1) = v.cam.to_content(v.vw, v.vh);
                let (a, b) = (((x0 - p.x) / p.w).max(0.0), ((y0 - p.y) / p.h).max(0.0));
                let (c, d) = (((x1 - p.x) / p.w).min(1.0), ((y1 - p.y) / p.h).min(1.0));
                format!("{a:.2}:{b:.2}:{c:.2}:{d:.2}")
            })
            .unwrap_or_default();
        let mut by_x: Vec<&Placed> = v.placed.iter().collect();
        by_x.sort_by(|a, b| a.x.total_cmp(&b.x));
        let order: Vec<String> = by_x.iter().map(|p| (p.page + 1).to_string()).collect();
        let grey_pages = v
            .placed
            .iter()
            .filter(|p| v.cache.peek(p.page).is_some_and(|s| s.grey))
            .count();
        format!(
            "comic_state: book={} order={} grey_pages={} pages={} page={} screen={}/{} mode={} fit={} rtl={} guided={} grey={} zoom={:.3} fitted={} cx={:.0} cy={:.0} shows={} stop={} stops={} panels={} cached={} failed={} animating={} draws={} draw_ms={:.0} sidebar={} thumbs={} view={:.0}x{:.0} pending={} gen={} label=\"{}\"",
            self.book_id,
            order.join("-"),
            grey_pages,
            self.n(),
            self.page + 1,
            self.screen() + 1,
            self.plan.len(),
            self.eff.mode.name(),
            self.eff.fit.name(),
            self.eff.rtl,
            self.eff.guided,
            self.grey(),
            v.cam.s,
            view::is_fitted(v.cam, f, self.fit()),
            cx,
            cy,
            shows,
            self.stop
                .map(|s| (s + 1).to_string())
                .unwrap_or_else(|| "none".into()),
            // `stops` borrows the view again, so count them by hand
            v.placed
                .iter()
                .map(|p| v.panels_of(p.page).len().max(1))
                .sum::<usize>(),
            panels_here,
            v.cache.len(),
            v.failed.len(),
            v.anim.is_some(),
            v.draws.get(),
            v.last_draw_ms.get(),
            self.w.split.shows_sidebar(),
            self.thumbs_done.len(),
            v.vw,
            v.vh,
            self.pending.is_some(),
            self.r#gen,
            self.page_text(),
        )
    }

    fn visible_pages_in_or_screen(&self, v: &View) -> Vec<usize> {
        if self.eff.mode == Mode::Webtoon {
            self.visible_pages_in(v)
        } else {
            self.plan.get(self.screen()).cloned().unwrap_or_default()
        }
    }

    fn input(&mut self, msg: Input) {
        match msg {
            Input::Poll => {
                if self.laid_out && self.last_eink != self.eink() {
                    self.last_eink = self.eink();
                    self.relayout(true);
                }
                let mut n = 0;
                while let Some(d) = self.worker.try_recv() {
                    self.on_decoded(d);
                    n += 1;
                    if n >= 4 {
                        break;
                    }
                }
                if self.eff.mode == Mode::Webtoon && self.laid_out {
                    let cur = self.page;
                    self.after_move_page_only();
                    if cur != self.page {
                        self.update_ui();
                    }
                }
            }
            Input::Resize(w, h) => self.on_resize(w, h),
            Input::Turn(t) => self.turn(t),
            Input::Arrow(d) => self.arrow(d),
            Input::Space(back) => self.space(back),
            Input::Sync(ev) => {
                if let Some(a) = &ev.ahead {
                    let (enabled, mine) = {
                        let c = self.shared.cfg.borrow();
                        (c.share.offer_jump, c.kosync.device_id.clone())
                    };
                    let n = self.n();
                    let local = if n > 0 {
                        self.page as f64 / n as f64
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
            Input::LiveAnswer(a) => {
                if let super::reader_live::Answer::Primary(crate::sync_live::Banner::Ahead {
                    percentage,
                    ..
                }) = a
                {
                    let n = self.n();
                    if n > 0 {
                        self.go_page(((percentage * n as f64) as usize).min(n - 1));
                    }
                }
            }
            Input::GoPage(p) => self.go_page(p),
            Input::ThumbGo(p) => {
                self.go_page(p);
                self.w.area.grab_focus();
            }
            Input::Seek(v) => {
                let p = (v.round() as usize).saturating_sub(1);
                if p != self.page {
                    self.go_page(p);
                }
            }
            Input::Wheel { dx, dy, ctrl, x, y } => self.wheel(dx, dy, ctrl, x, y),
            Input::Pinch(f, x, y) => self.zoom_by(f, x, y),
            Input::ZoomKey(f) => {
                let (w, h) = {
                    let v = self.view.borrow();
                    (v.vw / 2.0, v.vh / 2.0)
                };
                self.zoom_by(f, w, h);
            }
            Input::ZoomReset => {
                self.stop = None;
                self.view.borrow_mut().outline = None;
                let cam = self.refit_cam();
                self.set_cam(cam, true);
                self.after_move();
            }
            Input::DragBegin => {
                let mut v = self.view.borrow_mut();
                v.anim_gen += 1;
                v.anim = None;
                self.drag_from = Some(v.cam);
            }
            Input::DragUpdate(dx, dy) => {
                if let Some(c) = self.drag_from {
                    let f = self.view.borrow().frame();
                    let cam = view::pan(c, dx, dy, f);
                    self.view.borrow_mut().cam = cam;
                    self.w.area.queue_draw();
                }
            }
            Input::DragEnd(dx, dy) => {
                let moved = self.drag_from.take();
                if let Some(c) = moved {
                    let f = self.view.borrow().frame();
                    let was_fitted = view::is_fitted(c, f, self.fit());
                    if was_fitted
                        && self.eff.mode != Mode::Webtoon
                        && dx.abs() > 90.0
                        && dx.abs() > dy.abs() * 2.0
                    {
                        // a swipe: dragging to the right shows what is on the left
                        self.turn(layout::horizontal_turn(self.eff.rtl, dx < 0.0));
                    } else {
                        self.stop = None;
                        self.view.borrow_mut().outline = None;
                        self.after_move();
                    }
                }
            }
            Input::Click(n, x, y) => {
                if let Some(t) = self.click_timer.take() {
                    t.remove();
                }
                if n >= 2 {
                    self.smart_zoom(x, y);
                } else {
                    let tx = self.sender.input_sender().clone();
                    let delay = gtk::Settings::default()
                        .map(|s| s.gtk_double_click_time() as u64)
                        .unwrap_or(400)
                        .min(300);
                    self.click_timer = Some(glib::timeout_add_local_once(
                        Duration::from_millis(delay),
                        move || tx.emit(Input::SingleClick(x, y)),
                    ));
                }
            }
            Input::SingleClick(x, y) => {
                self.click_timer = None;
                let vw = self.view.borrow().vw;
                let zoomed = !self.fitted();
                match layout::click_zone(x, vw, self.eff.rtl) {
                    Some(t) if self.eff.mode != Mode::Webtoon && !zoomed => self.turn(t),
                    _ => {
                        let _ = y;
                        let shown = self.w.toolbar.reveals_top_bars();
                        self.w.toolbar.set_reveal_top_bars(!shown);
                        self.w.toolbar.set_reveal_bottom_bars(!shown);
                    }
                }
            }
            Input::SetMode(m) => {
                if m != self.eff.mode {
                    self.store.book_mut(self.book_id).mode = Some(m);
                    self.persist();
                    self.refresh_effective();
                    self.relayout(false);
                }
            }
            Input::CycleMode => {
                let next = match self.eff.mode {
                    Mode::Single => Mode::Spread,
                    Mode::Spread => Mode::Webtoon,
                    Mode::Webtoon => Mode::Single,
                };
                self.input(Input::SetMode(next));
            }
            Input::SetFit(f) => {
                if f != self.eff.fit {
                    self.store.book_mut(self.book_id).fit = Some(f);
                    self.persist();
                    self.refresh_effective();
                    self.stop = None;
                    self.view.borrow_mut().outline = None;
                    let cam = self.refit_cam();
                    self.set_cam(cam, false);
                    self.after_move();
                    self.sync_controls();
                }
            }
            Input::SetRtl(r) => {
                if r != self.eff.rtl {
                    self.store.book_mut(self.book_id).rtl = Some(r);
                    self.persist();
                    self.refresh_effective();
                    self.relayout(true);
                }
            }
            Input::SetGuided(g) => {
                if g != self.eff.guided {
                    self.store.book_mut(self.book_id).guided = Some(g);
                    self.persist();
                    self.refresh_effective();
                    if !g {
                        self.view.borrow_mut().outline = None;
                    }
                    self.sync_controls();
                    self.w.area.queue_draw();
                }
            }
            Input::SetGrey(g) => {
                if g != self.store.global.grey {
                    self.store.global.grey = g;
                    self.persist();
                    self.relayout(true);
                }
            }
            Input::SetCoverAlone(c) => {
                if c != self.store.global.cover_alone {
                    self.store.global.cover_alone = c;
                    self.persist();
                    self.relayout(false);
                }
            }
            Input::SetWideAlone(c) => {
                if c != self.store.global.wide_alone {
                    self.store.global.wide_alone = c;
                    self.persist();
                    self.relayout(false);
                }
            }
            Input::ResetBook => {
                let had_rtl = self.eff.rtl;
                self.store.forget(self.book_id);
                self.persist();
                self.refresh_effective();
                self.relayout(had_rtl != self.eff.rtl);
            }
            Input::Volume(forward) => self.open_volume(forward),
            Input::GotoDialog => self.goto_dialog(),
            Input::Properties => self.properties(),
            Input::SidebarShown => self.sidebar_changed(),
            Input::ThumbScroll => self.request_thumbs(),
            Input::Fullscreen => {
                if let Some(win) = window_of(&self.w.page) {
                    win.set_fullscreened(!win.is_fullscreen());
                }
            }
            Input::Quit => {
                if let Some(win) = window_of(&self.w.page) {
                    if win.is_fullscreen() {
                        win.set_fullscreened(false);
                        return;
                    }
                }
                let _ = self.sender.output(Output::Close);
            }
            Input::Dev(c) => self.dev(&c),
        }
    }

    /// Webtoon: which page is in view, without the rest of `after_move`.
    fn after_move_page_only(&mut self) {
        let (top, bottom) = {
            let v = self.view.borrow();
            (-v.cam.y / v.cam.s, (v.vh - v.cam.y) / v.cam.s)
        };
        let cur = {
            let v = self.view.borrow();
            layout::strip_current(&v.placed, top, bottom)
        };
        let visible = self.visible_pages();
        if cur != self.page {
            self.page = cur;
            self.save_progress();
        }
        if visible != self.last_visible {
            self.last_visible = visible;
            self.request_pages();
        }
    }

    fn wheel(&mut self, dx: f64, dy: f64, ctrl: bool, x: f64, y: f64) {
        if ctrl {
            let f = if dy < 0.0 { 1.15 } else { 1.0 / 1.15 };
            self.zoom_by(f, x, y);
            return;
        }
        let (cam, f) = {
            let v = self.view.borrow();
            (v.cam, v.frame())
        };
        if dx.abs() > 0.0 {
            let c = view::pan(cam, -dx * 60.0, 0.0, f);
            self.set_cam(c, false);
            self.after_move();
        }
        if dy.abs() > 0.0 {
            let down = dy > 0.0;
            let room = view::scroll_room(cam, f, down);
            if room > 0.5 || self.eff.mode == Mode::Webtoon {
                let cam = self.view.borrow().cam;
                let c = view::pan(cam, 0.0, -dy * 60.0, f);
                self.stop = None;
                self.view.borrow_mut().outline = None;
                self.set_cam(c, false);
                self.after_move();
            } else if self.last_turn.elapsed() > Duration::from_millis(350) {
                self.last_turn = Instant::now();
                self.turn_edge(down);
            }
        }
    }

    // ---- developer hooks --------------------------------------------------------------------

    /// Test hooks: `reader:comic-…` commands (see `_developer/docs.md`).
    fn dev(&mut self, c: &str) {
        let (cmd, arg) = c.split_once('=').unwrap_or((c, ""));
        let frac = |a: &str| -> Option<(f64, f64)> {
            let (x, y) = a.split_once(':')?;
            Some((x.parse().ok()?, y.parse().ok()?))
        };
        match cmd {
            "live-state" => {
                eprintln!(
                    "live_state: page={} banner={}",
                    self.page,
                    self.sync_banner.dev_state()
                );
            }
            "live-press" => self.sync_banner.dev_press(arg != "secondary"),
            // `reader:menu-dump` prints the Main Menu, `reader:header-dump` the header, `reader:view-dump` the View popover
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
            "menu-popup" => self.w.primary.popup(),
            "menu-popdown" => self.w.primary.popdown(),
            "header-dump" => {
                let hw = self.w.header.width() as f64;
                let (mut shown, mut clipped) = (0, 0);
                header_buttons(
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
            "view-dump" => {
                // a widget in a closed popover is not "visible" for GTK: open it for the look
                self.w.view_pop.popup();
                let nat = self
                    .w
                    .view_pop
                    .child()
                    .map(|c| c.measure(gtk::Orientation::Vertical, 300).1)
                    .unwrap_or(0);
                eprintln!(
                    "comic_view: natural_height={nat} window_height={} reset_visible={} labels={}",
                    self.w.page.height(),
                    self.w.reset.is_visible(),
                    [&self.w.rtl, &self.w.guided, &self.w.grey]
                        .iter()
                        .filter_map(|c| c.label())
                        .collect::<Vec<_>>()
                        .join("|")
                );
                self.w.view_pop.popdown();
            }
            "comic-failed-text" => eprintln!(
                "comic_failed_text: {:?} label={:?}",
                placeholder_text(true, 0),
                self.w.label.text()
            ),
            "goto-dialog" => self.goto_dialog(),
            "toggle-guided" => self.input(Input::SetGuided(!self.eff.guided)),
            "toggle-rtl" => self.input(Input::SetRtl(!self.eff.rtl)),
            "toggle-sidebar" => {
                let on = !self.w.split.shows_sidebar();
                self.w.split.set_show_sidebar(on);
            }
            "comic-state" => eprintln!("{}", self.state_line()),
            "comic-goto" => {
                let p: usize = arg.parse().unwrap_or(1);
                self.go_page(p.saturating_sub(1));
            }
            "comic-mode" => {
                if let Some(m) = Mode::from_name(arg) {
                    self.input(Input::SetMode(m));
                }
            }
            "comic-fit" => {
                if let Some(f) = Fit::from_name(arg) {
                    self.input(Input::SetFit(f));
                }
            }
            "comic-rtl" => self.input(Input::SetRtl(arg == "on")),
            "comic-guided" => self.input(Input::SetGuided(arg == "on")),
            "comic-grey" => self.input(Input::SetGrey(arg == "on")),
            "comic-cover-alone" => self.input(Input::SetCoverAlone(arg == "on")),
            "comic-wide-alone" => self.input(Input::SetWideAlone(arg == "on")),
            "comic-reset" => self.input(Input::ResetBook),
            // `reader:comic-volume=next|prev` opens the neighbour, `comic-volumes` prints them
            "comic-volume" => self.input(Input::Volume(arg == "next")),
            "comic-volumes" => eprintln!(
                "comic_volumes: prev={} next={}",
                self.volumes[0].as_ref().map_or("-", |v| v.1.as_str()),
                self.volumes[1].as_ref().map_or("-", |v| v.1.as_str())
            ),
            "comic-animate" => self.animate_override = Some(arg == "on"),
            "comic-key" => {
                let (mods, name) = arg.rsplit_once('+').unwrap_or(("", arg));
                let shift = mods.contains("shift");
                let ctrl = mods.contains("ctrl");
                if let Some(i) = gdk::Key::from_name(name).and_then(|k| key_input(k, ctrl, shift)) {
                    self.input(i);
                }
            }
            "comic-click" | "comic-dblclick" | "comic-dblclick-page" => {
                if let Some((fx, fy)) = frac(arg) {
                    let (x, y) = {
                        let v = self.view.borrow();
                        if cmd == "comic-dblclick-page" {
                            let p = v
                                .placed
                                .iter()
                                .find(|p| p.page == self.page)
                                .or(v.placed.first())
                                .copied();
                            p.map(|p| v.cam.to_view(p.x + fx * p.w, p.y + fy * p.h))
                                .unwrap_or((0.0, 0.0))
                        } else {
                            (fx * v.vw, fy * v.vh)
                        }
                    };
                    self.input(Input::Click(if cmd == "comic-click" { 1 } else { 2 }, x, y));
                }
            }
            "comic-wheel" => {
                let (dy, ctrl) = arg
                    .strip_prefix("ctrl:")
                    .map(|a| (a, true))
                    .unwrap_or((arg, false));
                let (w, h) = {
                    let v = self.view.borrow();
                    (v.vw / 2.0, v.vh / 2.0)
                };
                self.input(Input::Wheel {
                    dx: 0.0,
                    dy: dy.parse().unwrap_or(1.0),
                    ctrl,
                    x: w,
                    y: h,
                });
            }
            "comic-drag" => {
                if let Some((dx, dy)) = frac(arg) {
                    self.input(Input::DragBegin);
                    self.input(Input::DragUpdate(dx, dy));
                    self.input(Input::DragEnd(dx, dy));
                }
            }
            "comic-thumb" => {
                // what a click on the row does
                let p: i32 = arg.parse().unwrap_or(1);
                if let Some(row) = self.w.thumb_list.row_at_index(p - 1) {
                    row.activate();
                }
            }
            "comic-sidebar" => {
                self.w.split.set_show_sidebar(arg == "on");
                self.request_thumbs();
            }
            "comic-fullscreen" => self.input(Input::Fullscreen),
            _ => {}
        }
    }
}

fn check(label: &str) -> gtk::CheckButton {
    let b = gtk::CheckButton::with_label(label);
    b.set_focus_on_click(false);
    b
}

/// The line under the Go to Page field: where it leads, or why it does not.
fn page_check(text: &str, pages: usize) -> Result<String, String> {
    let n: usize = text
        .trim()
        .parse()
        .map_err(|_| tr("A page is a number, for example 12."))?;
    if n == 0 || n > pages {
        return Err(trf("This comic has {} pages.", &[&pages]));
    }
    Ok(trf("Page {} of {}", &[&n, &pages]))
}

/// Counts the buttons of a header and those that stick out of it.
pub(super) fn header_buttons(
    w: &gtk::Widget,
    header: &gtk::Widget,
    hw: f64,
    shown: &mut i32,
    clipped: &mut i32,
) {
    let is_button =
        w.is::<gtk::Button>() || w.is::<gtk::MenuButton>() || w.is::<gtk::ToggleButton>();
    if is_button && w.is_mapped() {
        *shown += 1;
        if let Some(b) = w.compute_bounds(header) {
            if b.x() < -1.0 || (b.x() + b.width()) as f64 > hw + 1.0 {
                *clipped += 1;
            }
        }
        return;
    }
    let mut c = w.first_child();
    while let Some(x) = c {
        header_buttons(&x, header, hw, shown, clipped);
        c = x.next_sibling();
    }
}

fn heading(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("heading");
    l.set_halign(gtk::Align::Start);
    l.set_margin_top(6);
    l
}

impl SimpleComponent for ReaderComic {
    type Init = (SharedRef, u64, ComicBook);
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
        (shared, book_id, comic): Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let title_text = shared
            .lib
            .borrow()
            .get(book_id)
            .map(|b| b.meta.title.clone())
            .unwrap_or_default();
        let source = comic.source.clone();
        let n = source.count();
        let sizes: Vec<(u32, u32)> = (0..n)
            .map(|i| {
                source
                    .bytes(i)
                    .and_then(|b| bookx_comic::decode::dimensions(b).ok())
                    .unwrap_or((2, 3))
            })
            .collect();
        let double = meta::double_flags(&comic.info, n);
        let store = crate::comic::load();
        let library_key = shared
            .cfg
            .borrow()
            .library_dir
            .to_string_lossy()
            .into_owned();
        let eff = store.effective_in(book_id, comic.rtl, &library_key);
        let worker = Worker::new(source.clone(), 2);
        let updating_ui = Rc::new(Cell::new(false));
        let alive = Rc::new(Cell::new(true));

        // ---- the page area ------------------------------------------------------------------
        let view_state = Rc::new(RefCell::new(View {
            placed: Vec::new(),
            content: (1.0, 1.0),
            cam: Cam {
                s: 1.0,
                x: 0.0,
                y: 0.0,
            },
            vw: 0.0,
            vh: 0.0,
            cache: PageCache::new(CACHE_BYTES),
            failed: HashSet::new(),
            anim: None,
            anim_gen: 0,
            dark: adw::StyleManager::default().is_dark(),
            outline: None,
            draws: Cell::new(0),
            last_draw_ms: Cell::new(0.0),
        }));
        let area = gtk::DrawingArea::new();
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.set_focusable(true);
        area.set_can_focus(true);
        area.add_css_class("comic-pages");
        super::audit::label(&area, &trf("{}, comic pages", &[&title_text]));
        area.update_property(&[gtk::accessible::Property::Description(
            "Arrow keys and Page Down turn the pages, Space reads on, plus and minus zoom, double-click zooms to a panel",
        )]);
        {
            let st = view_state.clone();
            area.set_draw_func(move |_, cr, w, h| {
                let t = Instant::now();
                let v = st.borrow();
                draw(&v, cr, w, h);
                v.draws.set(v.draws.get() + 1);
                v.last_draw_ms.set(t.elapsed().as_secs_f64() * 1000.0);
            });
            let tx = sender.input_sender().clone();
            area.connect_resize(move |_, w, h| tx.emit(Input::Resize(w, h)));
            let st = view_state.clone();
            let a = area.clone();
            adw::StyleManager::default().connect_dark_notify(move |m| {
                st.borrow_mut().dark = m.is_dark();
                a.queue_draw();
            });
        }
        {
            let drag = gtk::GestureDrag::new();
            drag.set_button(1);
            let tx = sender.input_sender().clone();
            drag.connect_drag_begin(move |_, _, _| tx.emit(Input::DragBegin));
            let tx = sender.input_sender().clone();
            drag.connect_drag_update(move |_, dx, dy| tx.emit(Input::DragUpdate(dx, dy)));
            let tx = sender.input_sender().clone();
            drag.connect_drag_end(move |_, dx, dy| tx.emit(Input::DragEnd(dx, dy)));
            area.add_controller(drag);

            let click = gtk::GestureClick::new();
            click.set_button(1);
            // a drag (a swipe, a pan) also ends with a release: only a press and a release at about the
            // same place is a click
            let pressed_at = Rc::new(Cell::new((0.0f64, 0.0f64)));
            let p = pressed_at.clone();
            click.connect_pressed(move |_, _, x, y| p.set((x, y)));
            let (tx, a) = (sender.input_sender().clone(), area.clone());
            click.connect_released(move |_, n, x, y| {
                a.grab_focus();
                let (px, py) = pressed_at.get();
                if (x - px).abs() <= 8.0 && (y - py).abs() <= 8.0 {
                    tx.emit(Input::Click(n, x, y));
                }
            });
            area.add_controller(click);

            let scroll = gtk::EventControllerScroll::new(
                gtk::EventControllerScrollFlags::BOTH_AXES
                    | gtk::EventControllerScrollFlags::KINETIC,
            );
            let tx = sender.input_sender().clone();
            let motion = Rc::new(Cell::new((0.0f64, 0.0f64)));
            let m = motion.clone();
            let pointer = gtk::EventControllerMotion::new();
            pointer.connect_motion(move |_, x, y| m.set((x, y)));
            area.add_controller(pointer);
            scroll.connect_scroll(move |c, dx, dy| {
                let ctrl = c
                    .current_event_state()
                    .contains(gdk::ModifierType::CONTROL_MASK);
                let (x, y) = motion.get();
                tx.emit(Input::Wheel { dx, dy, ctrl, x, y });
                glib::Propagation::Stop
            });
            area.add_controller(scroll);

            let zoom = gtk::GestureZoom::new();
            let base = Rc::new(Cell::new(1.0f64));
            let tx = sender.input_sender().clone();
            let b = base.clone();
            zoom.connect_begin(move |_, _| b.set(1.0));
            let (b, a) = (base.clone(), area.clone());
            zoom.connect_scale_changed(move |g, s| {
                let f = s / b.get();
                if !(0.98..1.02).contains(&f) {
                    b.set(s);
                    let (cx, cy) = g
                        .bounding_box_center()
                        .unwrap_or((a.width() as f64 / 2.0, a.height() as f64 / 2.0));
                    tx.emit(Input::Pinch(f, cx, cy));
                }
            });
            area.add_controller(zoom);
        }

        // ---- the strip of thumbnails ---------------------------------------------------------
        let thumb_list = gtk::ListBox::new();
        thumb_list.add_css_class("navigation-sidebar");
        thumb_list.set_selection_mode(gtk::SelectionMode::Single);
        super::audit::label(&thumb_list, &tr("Pages"));
        let mut thumb_pics = Vec::with_capacity(n);
        for i in 0..n {
            let pic = gtk::Picture::new();
            pic.set_content_fit(gtk::ContentFit::Contain);
            pic.set_size_request(120, ROW_H - 40);
            pic.set_can_shrink(true);
            let page_label = gtk::Label::new(Some(&(i + 1).to_string()));
            page_label.add_css_class("caption");
            let kind = meta::type_of(&comic.info, i);
            let name = if kind == "Story" {
                trf("Page {}", &[&(i + 1)])
            } else {
                trf("Page {} ({})", &[&(i + 1), &kind])
            };
            let bx = gtk::Box::new(gtk::Orientation::Vertical, 2);
            bx.set_margin_top(4);
            bx.set_margin_bottom(4);
            bx.append(&pic);
            bx.append(&page_label);
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&bx));
            row.set_height_request(ROW_H);
            row.set_tooltip_text(Some(&name));
            super::audit::label(&row, &name);
            thumb_list.append(&row);
            thumb_pics.push(pic);
        }
        let thumb_scroll = gtk::ScrolledWindow::new();
        thumb_scroll.set_child(Some(&thumb_list));
        thumb_scroll.set_vexpand(true);
        thumb_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        {
            let tx = sender.input_sender().clone();
            thumb_list.connect_row_activated(move |_, row| {
                tx.emit(Input::ThumbGo(row.index().max(0) as usize))
            });
            let tx = sender.input_sender().clone();
            thumb_scroll
                .vadjustment()
                .connect_value_changed(move |_| tx.emit(Input::ThumbScroll));
        }
        let side_header = adw::HeaderBar::new();
        side_header.set_show_back_button(false);
        side_header.set_show_end_title_buttons(false);
        side_header.set_show_start_title_buttons(false);
        side_header.set_title_widget(Some(&adw::WindowTitle::new(&tr("Pages"), "")));
        let side_toolbar = adw::ToolbarView::new();
        side_toolbar.add_top_bar(&side_header);
        side_toolbar.set_content(Some(&thumb_scroll));

        // ---- footer -------------------------------------------------------------------------
        let slider =
            gtk::Scale::with_range(gtk::Orientation::Horizontal, 1.0, n.max(2) as f64, 1.0);
        super::audit::label(&slider, &tr("Page"));
        slider.set_draw_value(false);
        slider.set_hexpand(true);
        slider.set_focus_on_click(false);
        slider.add_css_class("reader-slider");
        let prev_btn = icon_button("go-previous-symbolic", &tr("Previous Page (Page Up)"));
        let next_btn = icon_button("go-next-symbolic", &tr("Next Page (Page Down)"));
        for b in [&prev_btn, &next_btn] {
            b.add_css_class("flat");
            b.add_css_class("circular");
        }
        let label = gtk::Label::new(None);
        label.add_css_class("dim-label");
        label.set_width_chars(14);
        let nav = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        nav.add_css_class("reader-nav");
        nav.set_margin_start(16);
        nav.set_margin_end(16);
        nav.set_margin_bottom(6);
        nav.append(&prev_btn);
        nav.append(&slider);
        nav.append(&next_btn);
        nav.append(&label);
        let bottom = gtk::Box::new(gtk::Orientation::Vertical, 0);
        bottom.add_css_class("reader-bottom");
        bottom.append(&nav);

        let split = adw::OverlaySplitView::new();
        split.set_sidebar(Some(&side_toolbar));
        split.set_content(Some(&area));
        split.set_min_sidebar_width(160.0);
        split.set_max_sidebar_width(200.0);
        split.set_show_sidebar(store.global.thumbs);

        // ---- header --------------------------------------------------------------------------
        let header = adw::HeaderBar::new();
        let title = adw::WindowTitle::new(&title_text, "");
        header.set_title_widget(Some(&title));
        let sidebar_btn = gtk::ToggleButton::new();
        sidebar_btn.set_icon_name("sidebar-show-symbolic");
        sidebar_btn.set_tooltip_text(Some(&tr("Page Thumbnails (F9)")));
        sidebar_btn.set_focus_on_click(false);
        super::audit::label(&sidebar_btn, &tr("Page Thumbnails"));
        split
            .bind_property("show-sidebar", &sidebar_btn, "active")
            .bidirectional()
            .sync_create()
            .build();
        header.pack_start(&sidebar_btn);
        let fullscreen_btn = icon_button("view-fullscreen-symbolic", &tr("Fullscreen (F11)"));
        let bin = adw::BreakpointBin::new();
        bin.set_size_request(360, 294);
        let primary = gtk::MenuButton::new();
        primary.set_icon_name("open-menu-symbolic");
        primary.set_primary(true);
        primary.set_tooltip_text(Some(&tr("Main Menu")));
        super::audit::label(&primary, &tr("Main Menu"));
        header.pack_end(&primary);
        header.pack_end(&fullscreen_btn);

        // view options
        let pop = gtk::Popover::new();
        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 2);
        for m in ["start", "end", "top", "bottom"] {
            match m {
                "start" => vbox.set_margin_start(12),
                "end" => vbox.set_margin_end(12),
                "top" => vbox.set_margin_top(10),
                _ => vbox.set_margin_bottom(10),
            }
        }
        vbox.append(&heading(&tr("Layout")));
        let mut modes = Vec::new();
        let mut first: Option<gtk::CheckButton> = None;
        for (m, name) in [
            (Mode::Single, tr("Single page")),
            (Mode::Spread, tr("Two pages")),
            (Mode::Webtoon, tr("Vertical scroll")),
        ] {
            let b = check(&name);
            if let Some(f) = &first {
                b.set_group(Some(f));
            } else {
                first = Some(b.clone());
            }
            let (tx, u) = (sender.input_sender().clone(), updating_ui.clone());
            b.connect_toggled(move |b| {
                if b.is_active() && !u.get() {
                    tx.emit(Input::SetMode(m));
                }
            });
            vbox.append(&b);
            modes.push((m, b));
        }
        let cover = check(&tr("Cover alone"));
        cover.set_tooltip_text(Some(&tr("First Page on Its Own in Two-Page View")));
        let wide = check(&tr("Wide pages alone"));
        wide.set_tooltip_text(Some(&tr("Double-Page Picture on Its Own in Two-Page View")));
        vbox.append(&cover);
        vbox.append(&wide);
        vbox.append(&heading(&tr("Fit")));
        let mut fits = Vec::new();
        let mut first: Option<gtk::CheckButton> = None;
        for (f, name) in [
            (Fit::Page, tr("Fit page")),
            (Fit::Width, tr("Fit width")),
            (Fit::Height, tr("Fit height")),
            (Fit::Original, tr("Original size")),
        ] {
            let b = check(&name);
            if let Some(g) = &first {
                b.set_group(Some(g));
            } else {
                first = Some(b.clone());
            }
            let (tx, u) = (sender.input_sender().clone(), updating_ui.clone());
            b.connect_toggled(move |b| {
                if b.is_active() && !u.get() {
                    tx.emit(Input::SetFit(f));
                }
            });
            vbox.append(&b);
            fits.push((f, b));
        }
        vbox.append(&heading(&tr("Reading")));
        let rtl = check(&tr("Right to left"));
        rtl.set_tooltip_text(Some(&tr(
            "Manga Order: Pages and Panels Run from Right to Left",
        )));
        let guided = check(&tr("Guided view"));
        guided.set_tooltip_text(Some(&tr("Space Goes Panel by Panel")));
        let grey = check(&tr("Grey pages"));
        vbox.append(&rtl);
        vbox.append(&guided);
        vbox.append(&grey);
        let volumes = [
            crate::comic::neighbour_of(&shared.lib.borrow(), book_id, false),
            crate::comic::neighbour_of(&shared.lib.borrow(), book_id, true),
        ];
        vbox.append(&heading(&tr("Series")));
        let prev_vol = gtk::Button::with_label(&tr("Previous Volume"));
        let next_vol = gtk::Button::with_label(&tr("Next Volume"));
        for (b, v, fwd) in [
            (&prev_vol, &volumes[0], false),
            (&next_vol, &volumes[1], true),
        ] {
            b.add_css_class("flat");
            b.set_halign(gtk::Align::Start);
            b.set_widget_name(if fwd {
                "comic-next-volume"
            } else {
                "comic-prev-volume"
            });
            b.set_sensitive(v.is_some());
            if let Some((_, t)) = v {
                b.set_tooltip_text(Some(t));
            }
            let (tx, p) = (sender.input_sender().clone(), pop.clone());
            b.connect_clicked(move |_| {
                p.popdown();
                tx.emit(Input::Volume(fwd));
            });
            vbox.append(b);
        }
        let reset = gtk::Button::with_label(&tr("Use the Defaults for This Book"));
        reset.set_visible(false);
        reset.add_css_class("flat");
        reset.set_margin_top(6);
        vbox.append(&reset);
        {
            let (tx, u) = (sender.input_sender().clone(), updating_ui.clone());
            rtl.connect_toggled(move |b| {
                if !u.get() {
                    tx.emit(Input::SetRtl(b.is_active()))
                }
            });
            let (tx, u) = (sender.input_sender().clone(), updating_ui.clone());
            guided.connect_toggled(move |b| {
                if !u.get() {
                    tx.emit(Input::SetGuided(b.is_active()))
                }
            });
            let (tx, u) = (sender.input_sender().clone(), updating_ui.clone());
            grey.connect_toggled(move |b| {
                if !u.get() {
                    tx.emit(Input::SetGrey(b.is_active()))
                }
            });
            let (tx, u) = (sender.input_sender().clone(), updating_ui.clone());
            cover.connect_toggled(move |b| {
                if !u.get() {
                    tx.emit(Input::SetCoverAlone(b.is_active()))
                }
            });
            let (tx, u) = (sender.input_sender().clone(), updating_ui.clone());
            wide.connect_toggled(move |b| {
                if !u.get() {
                    tx.emit(Input::SetWideAlone(b.is_active()))
                }
            });
            let (tx, p) = (sender.input_sender().clone(), pop.clone());
            reset.connect_clicked(move |_| {
                p.popdown();
                tx.emit(Input::ResetBook);
            });
        }
        let pop_scroll = gtk::ScrolledWindow::new();
        pop_scroll.set_child(Some(&vbox));
        pop_scroll.set_propagate_natural_height(true);
        pop_scroll.set_propagate_natural_width(true);
        pop_scroll.set_max_content_height(320);
        pop_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        pop.set_child(Some(&pop_scroll));
        let view_btn = gtk::MenuButton::new();
        view_btn.set_icon_name("preferences-desktop-display-symbolic");
        view_btn.set_tooltip_text(Some(&tr("View Options")));
        super::audit::label(&view_btn, &tr("View options"));
        view_btn.set_popover(Some(&pop));
        header.pack_end(&view_btn);

        // main menu: the same builder as the e-book reader; a narrow window gives up Fullscreen
        let wide_menu = super::reader_menu::build(&super::reader_menu::COMIC_MENU);
        let narrow_menu = super::reader_menu::build(&super::reader_menu::narrow_entries(
            &super::reader_menu::COMIC_MENU,
            &super::reader_menu::NARROW_PDF,
        ));
        primary.set_menu_model(Some(&wide_menu));
        super::audit::name_menu_items(&primary);
        let narrow = Rc::new(Cell::new(false));
        {
            let narrow_flag = narrow.clone();
            let (primary, fs) = (primary.clone(), fullscreen_btn.clone());
            super::reader_menu::install_narrow(&bin, move |is_narrow| {
                narrow_flag.set(is_narrow);
                fs.set_visible(!is_narrow);
                primary.set_menu_model(Some(if is_narrow { &narrow_menu } else { &wide_menu }));
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
            add("goto", || Input::GotoDialog);
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
        let sync_sub = {
            let tx = sender.input_sender().clone();
            crate::sync_bus::subscribe(Some(book_id), move |e| tx.emit(Input::Sync(e.clone())))
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
        root.add_css_class("comic-reader");

        // wiring of the footer
        {
            let tx = sender.input_sender().clone();
            prev_btn.connect_clicked(move |_| tx.emit(Input::Turn(Turn::Prev)));
            let tx = sender.input_sender().clone();
            next_btn.connect_clicked(move |_| tx.emit(Input::Turn(Turn::Next)));
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
            split.connect_show_sidebar_notify(move |_| tx.emit(Input::SidebarShown));
        }

        // keyboard
        let keys = gtk::EventControllerKey::new();
        {
            let tx = sender.input_sender().clone();
            keys.connect_key_pressed(move |_, key, _, mods| {
                let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
                let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
                match key_input(key, ctrl, shift) {
                    Some(i) => {
                        tx.emit(i);
                        glib::Propagation::Stop
                    }
                    None => glib::Propagation::Proceed,
                }
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
            glib::timeout_add_local(Duration::from_millis(30), move || {
                tx.emit(Input::Poll);
                glib::ControlFlow::Continue
            })
        };
        {
            let (st, alive2) = (Rc::downgrade(&view_state), alive.clone());
            crate::devtools::probe("comic", move || {
                alive2.get()
                    && st.upgrade().is_some_and(|st| {
                        let v = st.borrow();
                        !v.placed.is_empty()
                            && v.placed.iter().take(2).any(|p| v.cache.contains(p.page))
                    })
            });
            let (st, alive) = (Rc::downgrade(&view_state), alive.clone());
            crate::devtools::probe("comic-ahead", move || {
                alive.get()
                    && st.upgrade().is_some_and(|st| {
                        let v = st.borrow();
                        !v.placed.is_empty() && v.cache.len() > AHEAD.min(2)
                    })
            });
        }

        let mut model = ReaderComic {
            sync_banner,
            _sync_sub: Some(sync_sub),
            seen_page: Cell::new(usize::MAX),
            shared,
            book_id,
            title: title_text,
            book_rtl: comic.rtl,
            library_key,
            volumes,
            edge_armed: None,
            worker,
            sizes,
            double,
            store,
            eff,
            plan: Vec::new(),
            page: 0,
            stop: None,
            pending: None,
            forward: true,
            r#gen: 0,
            view: view_state,
            w: W {
                page: root.clone(),
                area: area.clone(),
                title,
                slider,
                prev_btn,
                next_btn,
                label,
                split,
                toolbar,
                thumb_list,
                thumb_scroll,
                thumb_pics,
                modes,
                fits,
                rtl,
                guided,
                cover,
                wide,
                grey,
                reset: reset.clone(),
                primary: primary.clone(),
                header: header.clone(),
                view_pop: pop.clone(),
                narrow,
                fullscreen_btn: fullscreen_btn.clone(),
            },
            sender,
            dirty,
            timer: Some(timer),
            poll: Some(poll),
            click_timer: None,
            drag_from: None,
            last_turn: Instant::now(),
            updating_ui,
            thumbs_done: HashSet::new(),
            laid_out: false,
            last_visible: Vec::new(),
            last_eink: crate::eink::is_active(),
            animate_override: None,
            alive,
        };
        model.replan();
        model.sync_controls();
        model.update_ui();
        area.grab_focus();
        ComponentParts { model, widgets: () }
    }

    fn update(&mut self, msg: Input, _sender: ComponentSender<Self>) {
        self.input(msg);
    }

    fn shutdown(&mut self, _widgets: &mut Self::Widgets, _output: relm4::Sender<Self::Output>) {
        self.alive.set(false);
        self.save_progress();
        let now = self.shared.now();
        self.shared.stats.borrow_mut().finish(now);
        let _ = self.shared.lib.borrow().save();
        for t in [self.timer.take()].into_iter().flatten() {
            t.remove();
        }
        if let Some(p) = self.poll.take() {
            p.remove();
        }
        if let Some(c) = self.click_timer.take() {
            c.remove();
        }
        if let Some(win) = window_of(&self.w.page) {
            win.set_fullscreened(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_go_to_page_line_says_where_it_leads() {
        assert_eq!(page_check("12", 40), Ok("Page 12 of 40".to_string()));
        assert!(page_check("41", 40).is_err());
        assert!(page_check("0", 40).is_err());
        assert!(page_check("ii", 40).is_err());
    }

    #[test]
    fn a_damaged_page_says_so_and_a_loading_one_says_its_number() {
        assert_eq!(placeholder_text(true, 4), "This page cannot be shown");
        assert_eq!(placeholder_text(false, 4), "Page 5");
    }

    #[test]
    fn a_strip_is_as_wide_as_the_window_unless_shown_at_its_own_size() {
        assert_eq!(mode_fit(Mode::Webtoon, Fit::Page), Fit::Width);
        assert_eq!(mode_fit(Mode::Webtoon, Fit::Height), Fit::Width);
        assert_eq!(mode_fit(Mode::Webtoon, Fit::Original), Fit::Original);
        assert_eq!(mode_fit(Mode::Single, Fit::Height), Fit::Height);
        assert_eq!(mode_fit(Mode::Spread, Fit::Page), Fit::Page);
    }

    #[test]
    fn keys_map_to_the_same_actions_as_the_buttons() {
        use gdk::Key;
        assert!(matches!(
            key_input(Key::Page_Down, false, false),
            Some(Input::Turn(Turn::Next))
        ));
        assert!(matches!(
            key_input(Key::space, false, true),
            Some(Input::Space(true))
        ));
        assert!(matches!(
            key_input(Key::Right, false, false),
            Some(Input::Arrow(Dir::Right))
        ));
        assert!(key_input(Key::q, false, false).is_none());
        // Ctrl+G is not the guided-view key (that is plain G): like Ctrl+L it asks which page to go to
        assert!(matches!(
            key_input(Key::g, true, false),
            Some(Input::GotoDialog)
        ));
        assert!(matches!(
            key_input(Key::l, true, false),
            Some(Input::GotoDialog)
        ));
        assert!(matches!(
            key_input(Key::g, false, false),
            Some(Input::Dev(_))
        ));
    }
}
