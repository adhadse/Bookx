//! Page thumbnails for the book map: a small picture of the page the reader would show at a
//! place of the book, in the book's reading theme, with the highlights on it.
//!
//! The text is laid out by off-screen [`BookCanvas`]es that have the size and the style of the
//! reader's own, so the page is the one the reader draws (same fonts, pagination, images, writing
//! direction). A thumbnail is **one page**: the half of the view that holds the place in a two
//! column layout, the whole view otherwise (one column, scrolling, vertical text).
//!
//! Pango layouts are tied to the GTK thread, so the work is done on the main loop in slices:
//!
//! * a hover request is debounced (60 ms), served at idle priority and dropped before it starts when
//!   a newer one came;
//! * a prefetch queue (pages in view, nearest to the pointer first) is worked off one page per slice
//!   at a lower priority, so pointer events and the hovered page come first, and it is dropped when
//!   the map closes;
//! * the layouts of the last [`POOL`] chapters are kept, so going back and forth between chapters
//!   does not lay anything out again; thumbnails are kept in an LRU cache ([`CACHE`]);
//! * a page is painted scaled to the thumbnail size (not full size and shrunk).
//!
//! The one chunk that cannot be cut is the layout of a chapter; the time of every slice is recorded
//! (`Stats`) and the numbers are in `docs/perf.md`.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Instant;

use bookx_lookup::thumbs::{
    Debounce, Latest, Lru, group_by_chapter, locate, nearest, prefetch_order,
};
use relm4::gtk::{self, glib};

use super::canvas::{BookCanvas, CanvasStyle, Highlight};
use crate::core::doc::Doc;

/// How many thumbnails are kept.
pub const CACHE: usize = 64;
/// How many chapter layouts are kept.
pub const POOL: usize = 3;
/// How many pages the prefetch makes at most.
pub const PREFETCH: usize = 40;
/// Logical width of a thumbnail.
pub const WIDTH: f64 = 200.0;
const DEBOUNCE_MS: u64 = 60;
/// A prefetch layout waits until the pointer has been still this long (ms).
const QUIET_MS: u64 = 150;

/// What the thumbnails are made of.
pub struct Source {
    pub weights: Vec<f64>,
    pub total: usize,
    pub doc_for: Rc<dyn Fn(usize) -> Option<Doc>>,
    pub highlights_for: Rc<dyn Fn(usize) -> Vec<Highlight>>,
    pub images: Rc<dyn Fn(&str) -> Option<Vec<u8>>>,
    pub style: CanvasStyle,
    pub view: (i32, i32),
    pub lang: String,
    /// Vertical text: the whole view is the page.
    pub vertical: bool,
    /// Where thumbnails are kept on disk and what identifies the book (file and date), `None` for no
    /// disk cache; `hl_sig` changes when the highlights do.
    pub disk: Option<(std::path::PathBuf, String)>,
    pub hl_sig: String,
}

/// A rendered page.
#[derive(Clone)]
pub struct Thumb {
    pub surface: gtk::cairo::ImageSurface,
    /// Pixels of the surface (it is sharper than `WIDTH` on a HiDPI screen).
    pub size: (i32, i32),
    /// How long it took to make, in milliseconds.
    pub ms: f64,
    /// Pixels that are not the page colour (0 for a blank page).
    pub ink: usize,
    /// Which page of the view it is: `(column, columns)`.
    pub column: (usize, usize),
    /// The chapter it is of.
    pub spine: usize,
    /// The first page of the view it was painted from, and the pages of the chapter.
    pub view_page: (usize, usize),
    /// The chapter and layout fingerprint it was made from (`None` when it came from the disk).
    pub sig: Option<(usize, (usize, usize, i64, usize))>,
}

/// Where the time went, in milliseconds.
#[derive(Debug, Clone, Copy, Default)]
pub struct Timing {
    pub layout: f64,
    pub goto: f64,
    pub paint: f64,
    pub ink: f64,
}

/// Counters for the tests and the performance notes.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stats {
    pub layouts: u32,
    pub paints: u32,
    /// The longest single slice of prefetch work (the longest the main loop was held), ms.
    pub max_slice_ms: f64,
    pub prefetch_done: u32,
    /// Thumbnails read from the disk cache.
    pub disk_hits: u32,
    /// The longest prefetch slice that did not lay a chapter out (painting, or reading from disk), ms.
    pub max_paint_slice_ms: f64,
    pub last: Timing,
}

type Deliver = Rc<dyn Fn(usize, Thumb)>;

struct Slot {
    spine: usize,
    canvas: BookCanvas,
    used: u64,
}

pub struct Thumbs {
    src: Source,
    scale: f64,
    pool: RefCell<Vec<Slot>>,
    tick: Cell<u64>,
    cache: RefCell<Lru<usize, Thumb>>,
    latest: RefCell<Latest>,
    debounce: RefCell<Debounce<usize>>,
    /// The page the card waits for, and who to tell when it is ready (by any route).
    wanted: RefCell<Option<(usize, Deliver)>>,
    queue: RefCell<VecDeque<usize>>,
    prefetching: Cell<bool>,
    closed: Cell<bool>,
    started: Instant,
    /// The folder of this layout in the disk cache.
    folder: Option<std::path::PathBuf>,
    /// Pointer activity (ms since start): layouts wait until it has been quiet for a moment.
    activity: Cell<u64>,
    /// The chapter and layout fingerprint of the last page made (tests compare it with the reader's).
    pub sig: Cell<Option<(usize, (usize, usize, i64, usize))>>,
    pub stats: Cell<Stats>,
}

impl Thumbs {
    pub fn new(src: Source, device_scale: f64) -> Rc<Thumbs> {
        let folder = src.disk.as_ref().map(|(root, book)| {
            let layout = format!(
                "{:?}|{:?}|{}|{}|{}|{}|{}",
                src.style, src.view, src.lang, src.vertical, device_scale, src.hl_sig, WIDTH
            );
            crate::map_thumb_disk::folder(root, book, &layout)
        });
        Rc::new(Thumbs {
            folder,
            activity: Cell::new(0),
            src,
            scale: device_scale.max(1.0),
            pool: RefCell::new(Vec::new()),
            tick: Cell::new(0),
            cache: RefCell::new(Lru::new(CACHE)),
            latest: RefCell::new(Latest::default()),
            debounce: RefCell::new(Debounce::new(DEBOUNCE_MS)),
            wanted: RefCell::new(None),
            queue: RefCell::new(VecDeque::new()),
            prefetching: Cell::new(false),
            closed: Cell::new(false),
            started: Instant::now(),
            sig: Cell::new(None),
            stats: Cell::new(Stats::default()),
        })
    }

    fn view(&self) -> (i32, i32) {
        if self.src.view.0 > 40 && self.src.view.1 > 40 {
            self.src.view
        } else {
            (800, 1000)
        }
    }

    fn two_columns(&self) -> bool {
        self.src.style.layout == super::canvas::LayoutMode::TwoColumns && !self.src.vertical
    }

    /// The colour of the page itself, for the placeholder.
    pub fn page_colour(&self) -> (f64, f64, f64) {
        self.src.style.palette.bg
    }

    /// The shape of a thumbnail: logical width and height (one page of the view).
    pub fn logical_size(&self) -> (f64, f64) {
        let (w, h) = self.view();
        let w = if self.two_columns() {
            w as f64 / 2.0
        } else {
            w as f64
        };
        let (tw, th) = bookx_lookup::thumbs::thumb_size(w, h as f64, WIDTH);
        (tw as f64, th as f64)
    }

    pub fn cached(&self, page: usize) -> Option<Thumb> {
        self.cache.borrow_mut().get(&page)
    }

    /// The nearest thumbnail that is ready, as a stand-in for `page`.
    pub fn stand_in(&self, page: usize) -> Option<(usize, Thumb)> {
        let keys = self.cache.borrow().keys();
        let p = nearest(&keys, page)?;
        self.cached(p).map(|t| (p, t))
    }

    fn bump(&self, f: impl FnOnce(&mut Stats)) {
        let mut s = self.stats.get();
        f(&mut s);
        self.stats.set(s);
    }

    /// The canvas that has `spine` laid out (made now when it is not among the kept ones).
    fn slot_for(&self, spine: usize, local: f64) -> Option<usize> {
        let t = self.tick.get() + 1;
        self.tick.set(t);
        let mut pool = self.pool.borrow_mut();
        if let Some(i) = pool.iter().position(|s| s.spine == spine) {
            pool[i].used = t;
            return Some(i);
        }
        let t0 = Instant::now();
        let doc = (self.src.doc_for)(spine)?;
        let canvas = if pool.len() >= POOL {
            // the layout that was used longest ago is reused for the new chapter
            let (i, _) = pool.iter().enumerate().min_by_key(|(_, s)| s.used)?;
            pool.remove(i).canvas
        } else {
            let c = BookCanvas::new();
            let (w, h) = self.view();
            c.set_image_provider({
                let f = self.src.images.clone();
                move |p| f(p)
            });
            c.set_offscreen_size(w, h);
            c.set_language(&self.src.lang);
            c.set_style(self.src.style.clone());
            c
        };
        canvas.set_highlights((self.src.highlights_for)(spine));
        canvas.set_doc(doc, Some(local), None, false);
        pool.push(Slot {
            spine,
            canvas,
            used: t,
        });
        self.bump(|s| {
            s.layouts += 1;
            s.last.layout = t0.elapsed().as_secs_f64() * 1000.0;
        });
        Some(pool.len() - 1)
    }

    /// Makes the thumbnail of a page now (laying out its chapter first when needed).
    pub fn render(&self, page: usize) -> Option<Thumb> {
        if let Some(t) = self.cached(page) {
            return Some(t);
        }
        if let Some(t) = self.load_disk(page) {
            self.cache.borrow_mut().insert(page, t.clone());
            return Some(t);
        }
        self.render_laid(page)
    }

    /// Makes the thumbnail by laying the chapter out, ignoring the memory and disk caches
    /// (the dev hooks compare it with the reader's own page).
    pub fn render_laid(&self, page: usize) -> Option<Thumb> {
        let t0 = Instant::now();
        let (spine, local) = locate(&self.src.weights, self.src.total, page);
        let (idx, laid) = {
            let before = Instant::now();
            let i = self.slot_for(spine, local)?;
            (i, before.elapsed().as_secs_f64() * 1000.0)
        };
        let pool = self.pool.borrow();
        let canvas = &pool[idx].canvas;
        let t1 = Instant::now();
        let (col, cols) = canvas.goto_fraction_column(local);
        let goto = t1.elapsed().as_secs_f64() * 1000.0;
        self.sig.set(Some((spine, canvas.layout_signature())));
        let t2 = Instant::now();
        let mut surface = canvas.render_page_thumbnail(WIDTH, self.scale, col, cols)?;
        let paint = t2.elapsed().as_secs_f64() * 1000.0;
        let t3 = Instant::now();
        let ink = count_ink(&mut surface, self.page_colour());
        let thumb = Thumb {
            ink,
            size: (surface.width(), surface.height()),
            surface,
            ms: t0.elapsed().as_secs_f64() * 1000.0,
            column: (col, cols),
            spine,
            view_page: canvas.current_index(),
            sig: Some((spine, canvas.layout_signature())),
        };
        let ink_ms = t3.elapsed().as_secs_f64() * 1000.0;
        self.bump(|s| {
            s.paints += 1;
            s.last.goto = goto;
            s.last.paint = paint;
            s.last.ink = ink_ms;
            if laid < 1.0 {
                s.last.layout = 0.0;
            }
        });
        self.to_disk(page, &thumb);
        self.cache.borrow_mut().insert(page, thumb.clone());
        Some(thumb)
    }

    /// A thumbnail from the disk cache (no layout at all).
    fn load_disk(&self, page: usize) -> Option<Thumb> {
        let folder = self.folder.as_ref()?;
        let t0 = Instant::now();
        let bytes = crate::map_thumb_disk::load(&crate::map_thumb_disk::file(folder, page))?;
        let pb = gtk::gdk_pixbuf::Pixbuf::from_read(std::io::Cursor::new(bytes)).ok()?;
        let meta: Vec<usize> = pb
            .option("tEXt::bookx")?
            .split(',')
            .filter_map(|v| v.parse().ok())
            .collect();
        let [col, cols, spine, v, n] = meta[..] else {
            return None;
        };
        let mut surface =
            gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, pb.width(), pb.height())
                .ok()?;
        {
            let cr = gtk::cairo::Context::new(&surface).ok()?;
            use gtk::gdk::prelude::GdkCairoContextExt;
            cr.set_source_pixbuf(&pb, 0.0, 0.0);
            cr.paint().ok()?;
        }
        let ink = count_ink(&mut surface, self.page_colour());
        self.bump(|s| s.disk_hits += 1);
        Some(Thumb {
            ink,
            size: (surface.width(), surface.height()),
            surface,
            ms: t0.elapsed().as_secs_f64() * 1000.0,
            column: (col, cols),
            spine,
            view_page: (v, n),
            sig: None,
        })
    }

    fn to_disk(&self, page: usize, t: &Thumb) {
        let (Some(folder), Some((root, _))) = (self.folder.as_ref(), self.src.disk.as_ref()) else {
            return;
        };
        #[allow(deprecated)]
        let Some(pb) = gtk::gdk::pixbuf_get_from_surface(&t.surface, 0, 0, t.size.0, t.size.1)
        else {
            return;
        };
        let meta = format!(
            "{},{},{},{},{}",
            t.column.0, t.column.1, t.spine, t.view_page.0, t.view_page.1
        );
        if let Ok(bytes) = pb.save_to_bufferv("png", &[("tEXt::bookx", meta.as_str())]) {
            let _ = crate::map_thumb_disk::store(
                root,
                &crate::map_thumb_disk::file(folder, page),
                &bytes,
                crate::map_thumb_disk::CAP_BYTES,
            );
        }
    }

    /// Tells the card when the page it waits for is ready (whichever way it was made).
    fn notify(&self, page: usize, t: &Thumb) {
        let hit = {
            let w = self.wanted.borrow();
            w.as_ref()
                .filter(|(p, _)| *p == page)
                .map(|(_, d)| d.clone())
        };
        if let Some(d) = hit {
            d(page, t.clone());
        }
    }

    /// Asks for the thumbnail of `page`; `deliver` is called with it (at once when it is cached,
    /// else after the debounce and when the main loop is idle). Only the newest request is served.
    pub fn request(self: &Rc<Self>, page: usize, deliver: Deliver) {
        *self.wanted.borrow_mut() = Some((page, deliver.clone()));
        self.activity.set(self.started.elapsed().as_millis() as u64);
        if let Some(t) = self.cached(page) {
            self.latest.borrow_mut().cancel();
            self.debounce.borrow_mut().cancel();
            deliver(page, t);
            return;
        }
        let now = self.started.elapsed().as_millis() as u64;
        self.debounce.borrow_mut().push(page, now);
        let token = self.latest.borrow_mut().issue();
        self.after_debounce(token, DEBOUNCE_MS);
    }

    /// Waits for the debounce to be over (a timer can fire a moment early: then it waits a little
    /// more), then serves the request at idle priority.
    fn after_debounce(self: &Rc<Self>, token: u64, wait_ms: u64) {
        let me = self.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(wait_ms), move || {
            if me.closed.get() || !me.latest.borrow().is_current(token) {
                return; // a newer request restarted the wait
            }
            let now = me.started.elapsed().as_millis() as u64;
            let ready = me.debounce.borrow_mut().ready(now);
            let Some(page) = ready else {
                if me.debounce.borrow().is_waiting() {
                    me.after_debounce(token, 5);
                }
                return;
            };
            let me2 = me.clone();
            glib::idle_add_local_full(glib::Priority::HIGH_IDLE, move || {
                if !me2.closed.get() && me2.latest.borrow().is_current(token) {
                    if let Some(t) = me2.render(page) {
                        me2.notify(page, &t);
                    }
                }
                glib::ControlFlow::Break
            });
        });
    }

    /// Makes the pages in view ahead, nearest to `centre` first (at most [`PREFETCH`]), one page per
    /// slice of the main loop at a lower priority than the hovered page. The pages are taken chapter
    /// by chapter (so a layout is used for all its pages before the next chapter is laid out) and at
    /// most [`POOL`] chapters are laid out ahead: more would only push each other out. After a slice
    /// the loop rests as long as the slice took, so pointer events and drawing always
    /// get their turn. A new call replaces the queue.
    pub fn prefetch(self: &Rc<Self>, visible: &[usize], centre: usize) {
        let order = prefetch_order(visible, centre, PREFETCH, |p| {
            self.cache.borrow().contains(&p)
        });
        let ordered = group_by_chapter(&order, centre, POOL, |p| {
            locate(&self.src.weights, self.src.total, p).0
        });
        *self.queue.borrow_mut() = ordered.into();
        if self.prefetching.replace(true) {
            return;
        }
        self.schedule_slice(0);
    }

    fn schedule_slice(self: &Rc<Self>, rest_ms: u64) {
        let me = self.clone();
        glib::timeout_add_local_full(
            std::time::Duration::from_millis(rest_ms.max(1)),
            glib::Priority::LOW,
            move || {
                if me.closed.get() {
                    me.prefetching.set(false);
                    return glib::ControlFlow::Break;
                }
                let next = me.queue.borrow().front().copied();
                let Some(page) = next else {
                    me.prefetching.set(false);
                    return glib::ControlFlow::Break;
                };
                // a layout holds the main loop for as long as it takes: not while the pointer moves
                let now = me.started.elapsed().as_millis() as u64;
                if me.needs_layout(page) && now.saturating_sub(me.activity.get()) < QUIET_MS {
                    me.schedule_slice(100);
                    return glib::ControlFlow::Break;
                }
                me.queue.borrow_mut().pop_front();
                let layouts = me.stats.get().layouts;
                let t0 = Instant::now();
                if let Some(t) = me.render(page) {
                    me.notify(page, &t);
                }
                let ms = t0.elapsed().as_secs_f64() * 1000.0;
                let laid = me.stats.get().layouts > layouts;
                me.bump(|s| {
                    s.max_slice_ms = s.max_slice_ms.max(ms);
                    if !laid {
                        s.max_paint_slice_ms = s.max_paint_slice_ms.max(ms);
                    }
                    s.prefetch_done += 1;
                });
                me.schedule_slice(ms as u64);
                glib::ControlFlow::Break
            },
        );
    }

    /// True when making this page means laying a chapter out (it is neither kept laid out nor on disk).
    fn needs_layout(&self, page: usize) -> bool {
        if self.cache.borrow().contains(&page) {
            return false;
        }
        if let Some(f) = &self.folder {
            if crate::map_thumb_disk::file(f, page).exists() {
                return false;
            }
        }
        let spine = locate(&self.src.weights, self.src.total, page).0;
        !self.pool.borrow().iter().any(|s| s.spine == spine)
    }

    /// Pages still waiting in the prefetch queue.
    pub fn queued(&self) -> usize {
        self.queue.borrow().len()
    }

    /// Drops what is waiting (the map was closed).
    pub fn cancel(&self) {
        self.closed.set(true);
        self.queue.borrow_mut().clear();
        self.latest.borrow_mut().cancel();
        self.debounce.borrow_mut().cancel();
        *self.wanted.borrow_mut() = None;
    }
}

/// How many pixels of a painted surface are not the page colour: a thumbnail that shows a page has
/// some (text, pictures), a blank one has none.
fn count_ink(surface: &mut gtk::cairo::ImageSurface, page: (f64, f64, f64)) -> usize {
    let (w, h, stride) = (
        surface.width() as usize,
        surface.height() as usize,
        surface.stride() as usize,
    );
    let Ok(data) = surface.data() else { return 0 };
    let bg = [
        (page.2 * 255.0).round() as i32,
        (page.1 * 255.0).round() as i32,
        (page.0 * 255.0).round() as i32,
    ];
    let mut n = 0;
    for y in 0..h {
        for x in 0..w {
            let i = y * stride + x * 4;
            if (0..3).any(|k| (data[i + k] as i32 - bg[k]).abs() > 24) {
                n += 1;
            }
        }
    }
    n
}
