//! The EPUB reader page.

use crate::i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use chrono::Utc;
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gdk, gio, glib, pango};
use relm4::prelude::*;

use super::canvas::{
    BookCanvas, Edge, Highlight as CHighlight, LayoutMode, PosInfo, Rgb, SelectionInfo,
};
use super::web;
use super::{open_uri, system_is_dark, window_of, SharedRef};
use crate::config::{ColorMode, ReaderPrefs, ReaderTheme};
use crate::gestures::right_is_forward;
use bookx_text::highlight::HighlightStyle;

use crate::core::book::OpenBook;
use crate::core::library::{Bookmark, Highlight, ReadPos};
use crate::core::toc::{flatten, FlatToc};
use crate::core::util::*;

pub const HIGHLIGHT_COLORS: [(&str, Rgb); 5] = [
    ("yellow", (1.0, 0.85, 0.12)),
    ("purple", (0.72, 0.45, 0.95)),
    ("blue", (0.25, 0.55, 1.0)),
    ("red", (1.0, 0.38, 0.32)),
    ("green", (0.2, 0.8, 0.42)),
];

fn color_of(name: &str) -> Rgb {
    HIGHLIGHT_COLORS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, c)| *c)
        .unwrap_or(HIGHLIGHT_COLORS[0].1)
}

#[derive(Debug)]
pub enum Input {
    Turn(bool),
    Chapter(bool),
    GoSpine(usize, Option<f64>, Option<String>),
    GoHref(String),
    Position(PosInfo),
    Link(String),
    /// Follow a link without looking for a footnote popup first.
    FollowLink(String),
    /// Speed Reading window on the open chapter.
    Rsvp,
    RsvpCmd(super::rsvp::Cmd),
    RsvpTick,
    /// Bring back the cards the book was left with (once the first page is laid out).
    RestoreCards,
    Edge(Edge),
    Seek(f64),
    Selection(Option<SelectionInfo>),
    HighlightClick(SelectionInfo),
    Highlight(String),
    SaveNote(String),
    HlStyle(HighlightStyle),
    RemoveHighlight,
    Copy,
    PopClosed,
    Wiki,
    Dict,
    /// Download the dictionary for the language of the book (the button in the look-up popover).
    DictDownload,
    /// Preferences → Reading → Dictionaries.
    DictChoose,
    DictLearn,
    /// Progress of a dictionary download.
    DictEvent(super::dict_service::Event),
    Translate,
    TranslateLang(String),
    Zoom(f64),
    DoubleTap,
    Properties,
    /// A request from the sidebar.
    Side(super::reader_side::Cmd),
    /// The note written for a highlight in the sidebar (id, text).
    SideNote(String, String),
    /// The footer statistics were edited in the footer's own popover.
    Footer(crate::footer_stats::FooterConfig),
    /// The footer settings of this book only (true) or of all books (false).
    FooterScope(bool),
    FooterTick,
    /// Someone edited the footer settings: show them now.
    FooterChanged,
    FooterConfigure,
    /// A selection handle was let go: show the action popover again.
    HandlesDone,
    Reload,
    OpenBook,
    Prefs(ReaderPrefs),
    Tts(super::reader_tts::Cmd),
    TtsTick,
    Tweaks(crate::tweaks::StyleTweaks, bool),
    ToggleBookmark,
    GoBookmark(String),
    RemoveBookmark(String),
    GoHighlight(String),
    DeleteHighlight(String),
    Search(String),
    SearchResult(usize),
    SearchStep(bool),
    SearchDone(u64, Vec<(usize, usize, String)>, bool),
    Fullscreen,
    Tap,
    Back,
    /// Alt+Right: the place you went back from.
    Forward,
    /// The Go to… dialog.
    GotoDialog,
    /// The colour theme editor (a new theme, or the named custom theme).
    ThemeEditor(Option<String>),
    /// The look of this book becomes the default for new books.
    UseAsDefault,
    /// This book goes back to the default look.
    ResetLook,
    /// The custom themes changed: rebuild the swatches in the menu.
    ThemesChanged,
    /// The book map strip over the slider.
    BookMap,
    /// A jump chosen on the book map (fraction of the book).
    MapJump(f64),
    /// A click on a picture of the page (its path in the book).
    OpenImage(String),
    /// A place typed in the Go to… dialog.
    GotoText(String),
    ShowTab(&'static str),
    Quit,
    /// Shift+arrows: extend the keyboard selection by a step.
    KeySelect(crate::a11y::Step, bool),
    /// A reader key that works on the selection: commit, dict, wiki, translate, highlight, note, copy, clear.
    SelectionKey(&'static str),
    Dev(String),
}

#[derive(Debug)]
pub enum Output {
    Close,
    Toast(String),
    Reload,
    OpenFile,
    /// Preferences on the row with this title.
    OpenPreferences(String),
}

/// The offer to download a dictionary in the look-up popover.
#[derive(Clone)]
struct DictPromptUi {
    root: gtk::Box,
    title: gtk::Label,
    body: gtk::Label,
    bar: gtk::ProgressBar,
    primary: gtk::Button,
}

struct W {
    page: adw::NavigationPage,
    title: adw::WindowTitle,
    split: adw::OverlaySplitView,
    stack: adw::ViewStack,
    side: super::reader_side::Side,
    search_entry: gtk::SearchEntry,
    search_btn: gtk::MenuButton,
    search_list: gtk::ListBox,
    search_status: gtk::Label,
    slider: gtk::Scale,
    slider_row: gtk::Box,
    footer_pop: gtk::Popover,
    handles: super::sel_handles::Handles,
    left_label: gtk::Label,
    mid_label: gtk::Label,
    right_label: gtk::Label,
    bookmark_btn: gtk::Button,
    back_btn: gtk::Button,
    forward_btn: gtk::Button,
    sel_pop: gtk::Popover,
    sel_stack: gtk::Stack,
    sel_remove: gtk::Button,
    /// The selection popover is being re-presented: its `closed` signal is not a dismissal.
    switching: Rc<Cell<bool>>,
    dict_btn: gtk::Button,
    wiki_btn: gtk::Button,
    note_view: gtk::TextView,
    note_btn: gtk::Button,
    note_preview: gtk::Button,
    note_label: gtk::Label,
    note_save: gtk::Button,
    prev_btn: gtk::Button,
    next_btn: gtk::Button,
    fullscreen_btn: gtk::Button,
    /// The "Reader focus": when the bars fade (see `chrome_visibility`).
    chrome: Rc<ChromeCtl>,
    /// Looks now whether the bars fade (it also runs four times a second).
    chrome_eval: Rc<dyn Fn()>,
    /// Shows the bars again (an interaction).
    chrome_wake: Rc<dyn Fn()>,
    tts: super::reader_tts::TtsUi,
    chrome_probe: Rc<dyn Fn() -> bool>,
    swatches: Vec<gtk::Button>,
    style_btns: Vec<(HighlightStyle, gtk::ToggleButton)>,
    theme_btns: Vec<(ReaderTheme, gtk::ToggleButton)>,
    themes_ui: Rc<super::reader_theme_row::ThemeRow>,
    colors: super::colors::ColorControls,
    layout_btns: Vec<(LayoutMode, gtk::CheckButton)>,
    /// Columns and hyphenation in the text appearance popover.
    columns_dd: gtk::DropDown,
    hyphen_sw: gtk::Switch,
    zoom_label: gtk::Label,
    wiki_text: gtk::Label,
    dict_text: gtk::Label,
    dict_prompt: DictPromptUi,
    wiki_spin: gtk::Spinner,
    wiki_img: (gtk::Box, gtk::Stack, gtk::Picture),
    wiki_url: Rc<RefCell<String>>,
    tr_text: gtk::Label,
    tr_spin: gtk::Spinner,
    tr_dd: gtk::DropDown,
}

pub struct Reader {
    shared: SharedRef,
    book_id: u64,
    book: Rc<OpenBook>,
    canvas: BookCanvas,
    w: W,
    sender: ComponentSender<Self>,
    spine: Vec<String>,
    weights: Vec<f64>,
    toc: Vec<FlatToc>,
    spine_idx: usize,
    chapter_words: usize,
    pos: PosInfo,
    selection: Option<SelectionInfo>,
    jumps: crate::jump_history::JumpHistory,
    /// The printed pages of the book (EPUB page-list), when it has them.
    page_map: Option<bookx_text::pages::PageMap>,
    /// Where the reading time of the page on screen is credited, and since when.
    dwell_at: Option<(f64, std::time::Instant)>,
    page_marks: Vec<crate::page_list::Mark>,
    last_search: String,
    search_gen: Rc<Cell<u64>>,
    search_hits: Vec<(usize, usize)>,
    search_active: Option<usize>,
    pending_hit: Option<(String, usize)>,
    dirty: Rc<Cell<bool>>,
    timer: Option<glib::SourceId>,
    fullscreen: bool,
    chapter_pages: RefCell<HashMap<usize, usize>>,
    cur_prefs: Rc<RefCell<ReaderPrefs>>,
    fetch_gen: Rc<Cell<u64>>,
    /// Style used for the next highlight (and shown for the selected one).
    hl_style: HighlightStyle,
    tweaks_ui: Rc<RefCell<Option<super::reader_style::TweaksUi>>>,
    tts: TtsCtl,
    /// Speed reading: the card and the word marks on the page.
    rsvp: Rc<super::rsvp::RsvpCtl>,
    /// Fixed-layout (pre-paginated) book.
    fixed: bool,
    /// (reading speed in words per minute, seconds read today) from the statistics, for the footer.
    live: Cell<(Option<f64>, u64)>,
    /// When this book was opened (the footer's session time).
    opened: std::time::Instant,
    footer_timer: Option<glib::SourceId>,
    /// Hears every edit of the footer settings (Preferences, popovers, other readers).
    _footer_sub: Option<crate::settings_bus::Subscription>,
    /// Hears the progress of dictionary downloads.
    _dict_sub: Option<super::dict_service::Subscription>,
    /// The language of the book for dictionaries (`de`), once known.
    dict_lang: RefCell<Option<String>>,
    /// The download of a dictionary was asked for by a look-up: the definition follows it.
    dict_waiting: Cell<bool>,
    /// What the dictionary page offers now (for the dev dump).
    dict_prompt_state: RefCell<crate::dict_auto::Prompt>,
    /// The book was opened and its dictionary looked at (once).
    dict_opened: Cell<bool>,
    /// Closes the selection popover when a dialog, window or menu comes over the reader.
    overlay_guard: Rc<super::overlay_guard::OverlayGuard>,
}

/// Read aloud: the engine (opened on first use), the sentences being read and the clock that
/// polls the engine.
struct TtsCtl {
    engine: Option<Box<dyn crate::tts::Engine>>,
    session: crate::tts::Session,
    timer: Option<glib::SourceId>,
    /// The reader itself is turning to the next chapter.
    advancing: bool,
    /// Which engine is open (`kokoro`, `piper`, `server`, `speechd`, `mock`).
    engine_id: String,
    /// The language the book is read in.
    language: String,
    sleep: crate::tts::SleepTimer,
    /// The chapter whose next chapter was already prepared by the engine (no gap between them).
    prefetched: Option<usize>,
    /// The default voice is downloading; reading starts from `from` when it is ready.
    prep: Option<TtsPrep>,
    /// Why nothing is playing although read aloud was asked for (shown on the player card).
    notice: Option<crate::tts_player::Phase>,
    /// Why the last attempt to open an engine failed.
    not_ready: Option<crate::tts::NotReady>,
    /// Where reading was asked to start while the voice is prepared.
    want_from: i32,
    /// The chapter the player card shows the title of.
    shown_chapter: Option<usize>,
    /// Media keys and desktop controls, while the player card is shown.
    mpris: Option<crate::mpris::Mpris>,
    /// How many times the download offer was shown (tests).
    offered: u32,
    /// The card is open at a saved place and nothing plays: restored with the book, or reading was
    /// paused by leaving the chapter. Play continues from here.
    parked: Option<TtsParked>,
    /// The cards were last written to the config at this time (writes are spaced out while reading).
    cards_saved: Option<std::time::Instant>,
    /// The book is being left: the state of the cards is kept as it is.
    cards_frozen: bool,
}

/// Where the read-aloud card waits.
#[derive(Debug, Clone, PartialEq)]
struct TtsParked {
    chapter: usize,
    offset: i32,
    sentence: usize,
    sentences: usize,
    text: String,
}

/// This reader waits for the voice download (an application-level job, see `voice_download`):
/// it does not own the download, so leaving the book leaves it running.
struct TtsPrep {
    seq: u64,
    done: u64,
    total: u64,
}

/// Dev hook: prints how long the frames after a sidebar toggle took (`side_time:` line): the time
/// to the first frame, the longest gap between two frames of the animation (an idle gap of 400 ms
/// or more ends it) and the longest update, layout and paint of a frame.
fn trace_frames(page: &adw::NavigationPage, open: bool, t0: std::time::Instant) {
    use std::time::Instant;
    let ms = |from: Instant, to: Instant| to.duration_since(from).as_secs_f64() * 1000.0;
    // first frame, last frame, longest gap, frames, longest frame work, start of this frame's work
    let st = Rc::new(Cell::new((
        None::<f64>,
        t0,
        0.0f64,
        0u32,
        0.0f64,
        None::<Instant>,
    )));
    let clock = page.frame_clock();
    let mut ids = Vec::new();
    if let Some(clock) = &clock {
        let w = st.clone();
        ids.push(clock.connect_local("update", false, move |_| {
            let mut v = w.get();
            v.5 = Some(Instant::now());
            w.set(v);
            None
        }));
        let w = st.clone();
        ids.push(clock.connect_local("after-paint", false, move |_| {
            let mut v = w.get();
            if let Some(b) = v.5.take() {
                v.4 = v.4.max(ms(b, Instant::now()));
            }
            w.set(v);
            None
        }));
    }
    let w = st.clone();
    let tick = page.add_tick_callback(move |_, _| {
        let now = Instant::now();
        let mut v = w.get();
        v.0 = v.0.or(Some(ms(t0, now)));
        let gap = ms(v.1, now);
        if gap < 400.0 {
            v.2 = v.2.max(gap);
        }
        v.1 = now;
        v.3 += 1;
        w.set(v);
        glib::ControlFlow::Continue
    });
    glib::timeout_add_local_once(std::time::Duration::from_millis(900), move || {
        tick.remove();
        if let Some(c) = clock {
            for id in ids {
                c.disconnect(id);
            }
        }
        let v = st.get();
        eprintln!(
            "side_time: {} first_frame_ms={:.1} max_gap_ms={:.1} frames={} work_ms={:.1}",
            if open { "open" } else { "close" },
            v.0.unwrap_or(0.0),
            v.2,
            v.3,
            v.4
        );
    });
}

impl Reader {
    fn chapter_title(&self, idx: usize) -> String {
        let path = self.spine.get(idx).cloned().unwrap_or_default();
        let matches = |t: &&FlatToc| t.href.split('#').next() == Some(path.as_str());
        self.toc
            .iter()
            .find(|t| matches(t) && !t.href.contains('#'))
            .or_else(|| self.toc.iter().find(matches))
            .map(|t| t.title.clone())
            .unwrap_or_else(|| {
                let n = file_name(&path);
                n.rsplit_once('.').map(|(a, _)| a).unwrap_or(n).to_string()
            })
    }

    fn overall_fraction(&self) -> f64 {
        let total: f64 = self.weights.iter().sum();
        if total <= 0.0 {
            return 0.0;
        }
        let before: f64 = self.weights[..self.spine_idx.min(self.weights.len())]
            .iter()
            .sum();
        let cur = self.weights.get(self.spine_idx).copied().unwrap_or(0.0);
        ((before + cur * self.pos.frac) / total).clamp(0.0, 1.0)
    }

    /// The text of the footnote a link points at, `None` when it is not a note.
    fn footnote_for(&self, href: &str) -> Option<String> {
        let (i, frag) = self.book.locate(href)?;
        let src = self.book.chapter_source(self.spine.get(i)?)?;
        crate::footnotes::preview(&src, &frag?)
    }

    fn load_chapter(&mut self, idx: usize, frac: Option<f64>, anchor: Option<String>, last: bool) {
        let idx = idx.min(self.spine.len().saturating_sub(1));
        let Some(path) = self.spine.get(idx).cloned() else {
            return;
        };
        if !self.tts.advancing {
            // going to another chapter: what was running waits at its place, the cards stay
            if self.tts.session.state() != crate::tts::State::Stopped {
                self.tts_park();
            } else if self.tts.notice.is_some() {
                self.tts_stop();
            }
            self.rsvp.park_current(&self.canvas);
        }
        self.spine_idx = idx;
        let (styled, user_css, hyphen_lang) = {
            let cfg = self.shared.cfg.borrow();
            let r = &cfg.reader;
            let tweaks = cfg.tweaks.effective(self.book_id);
            (
                r.publisher_styles || r.colors == ColorMode::Original,
                tweaks.css(),
                tweaks.hyphenation_language(&self.book.metadata().language),
            )
        };
        let doc = self
            .book
            .chapter_doc_layered(&path, styled, &user_css)
            .unwrap_or_default();
        self.chapter_words = doc.word_count();
        self.canvas.set_language(&hyphen_lang);
        self.canvas.set_doc(doc, frac, anchor, last);
        self.refresh_highlights();
        self.w.mid_label.set_label(&self.chapter_title(idx));
        self.w.title.set_subtitle(&self.chapter_title(idx));
        // the contents entry of this chapter (the one with the anchor of the chapter itself, else the last in it)
        let row_idx = self
            .toc
            .iter()
            .enumerate()
            .filter(|(_, t)| t.href.split('#').next() == Some(path.as_str()))
            .find(|(_, t)| !t.href.contains('#'))
            .or_else(|| {
                self.toc
                    .iter()
                    .enumerate()
                    .rfind(|(_, t)| t.href.split('#').next() == Some(path.as_str()))
            })
            .map(|(n, _)| n);
        self.w.side.select_toc(row_idx);
        // hits of the last search stay marked in every chapter you visit
        let hits = if self.last_search.chars().count() >= 2 {
            self.canvas.find_all(&self.last_search)
        } else {
            Vec::new()
        };
        self.canvas.set_hits(hits, None);
        if let Some((q, nth)) = self.pending_hit.take() {
            let hits = self.canvas.find_all(&q);
            let nth = nth.min(hits.len().saturating_sub(1));
            self.canvas.set_hits(hits, Some(nth));
        }
        self.update_bookmark_button();
    }

    fn refresh_highlights(&self) {
        let lib = self.shared.lib.borrow();
        let Some(b) = lib.get(self.book_id) else {
            return;
        };
        let hl: Vec<CHighlight> = b
            .highlights
            .iter()
            .filter(|h| h.spine == self.spine_idx && !crate::pdf::is_pdf(h))
            .map(|h| CHighlight {
                id: h.id.clone(),
                start: h.start,
                end: h.end,
                color: color_of(&h.color),
                style: HighlightStyle::from_name(&h.style).unwrap_or_default(),
                note: h.note.clone(),
            })
            .collect();
        self.canvas.set_highlights(hl);
    }

    /// Feeds the reading statistics (sessions, pages, words) with the current position.
    fn track_reading(&self) {
        let wpp = (self.chapter_words / self.pos.pages.max(1)) as u64;
        let frac = self.overall_fraction();
        let now = self.shared.now();
        let mut hub = self.shared.stats.borrow_mut();
        hub.on_position(
            self.book_id,
            now,
            (self.spine_idx, self.pos.page),
            frac,
            wpp,
        );
        if frac >= 0.98 {
            let today = now.with_timezone(&chrono::Local).date_naive();
            hub.mark_finished(self.book_id, today);
        }
    }

    fn save_progress(&self) {
        let frac = self.overall_fraction();
        let mut lib = self.shared.lib.borrow_mut();
        lib.set_progress(
            self.book_id,
            ReadPos {
                spine: self.spine_idx,
                frac: self.pos.frac,
            },
            // the very last page counts as finished
            if self.spine_idx + 1 >= self.spine.len() && self.pos.page >= self.pos.pages {
                1.0
            } else {
                frac
            },
        );
        self.dirty.set(true);
        super::kosync::position(&self.shared, self.book_id, self.spine_idx, frac);
    }

    /// (page in book, pages in book) — chapters that were not laid out yet are estimated
    /// from the page density of those that were.
    fn book_pages(&self) -> (usize, usize) {
        estimate_book_pages(
            &self.chapter_pages.borrow(),
            &self.weights,
            self.spine_idx,
            self.pos.page,
            self.pos.pages,
        )
    }

    /// The printed page label of the page on screen (the break on it, else the last one before it).
    fn printed_label(&self) -> Option<String> {
        if self.page_marks.is_empty() {
            return None;
        }
        let before = self
            .page_marks
            .iter()
            .rfind(|m| m.spine < self.spine_idx)
            .map(|m| m.label.as_str());
        let here: Vec<(f64, String)> = self
            .page_marks
            .iter()
            .filter(|m| m.spine == self.spine_idx)
            .map(|m| {
                let f = m
                    .fragment
                    .as_deref()
                    .and_then(|a| self.canvas.anchor_fraction(a))
                    .unwrap_or(0.0);
                (f, m.label.clone())
            })
            .collect();
        let end = self.canvas.view_end_fraction() - 1e-6;
        crate::page_list::label_for(before, &here, end)
    }

    /// "12 of 340", or the printed page ("Page iv") when the book has a page-list.
    fn page_label_text(&self, page: usize, total: usize) -> String {
        match self.printed_label() {
            Some(l) => format!("Page {l}"),
            None => format!("{page} of {total}"),
        }
    }

    /// What the footer can say right now (see `footer_stats`).
    fn footer_facts(&self) -> crate::footer_stats::Facts {
        let (page, total) = self.book_pages();
        let overall = self.overall_fraction();
        let (live_wpm, today_secs) = self.live.get();
        let wpm = live_wpm.filter(|w| *w > 0.0).unwrap_or(230.0);
        let secs = |words: f64| (words.max(0.0) * 60.0 / wpm).ceil() as u64;
        let chapter_left = self.chapter_words as f64 * (1.0 - self.pos.frac);
        let weight_total: f64 = self.weights.iter().sum();
        let weight_here = self.weights.get(self.spine_idx).copied().unwrap_or(0.0);
        let book_words = if weight_here > 0.0 {
            self.chapter_words as f64 * weight_total / weight_here
        } else {
            0.0
        };
        let meta = self.book.metadata();
        crate::footer_stats::Facts {
            page_text: self.page_label_text(page, total),
            chapter_page: self.pos.page,
            chapter_pages: self.pos.pages,
            fraction: overall,
            chapter_secs_left: Some(secs(chapter_left)),
            book_secs_left: (book_words > 0.0).then(|| secs(book_words * (1.0 - overall))),
            wpm: live_wpm,
            clock: chrono::Local::now().format("%H:%M").to_string(),
            battery: crate::footer_stats::system_battery(),
            chapter_title: self.chapter_title(self.spine_idx),
            book_title: meta.title.clone(),
            author: meta.authors.join(", "),
            chapter_no: self.toc_chapter_no(),
            chapters: self.spine.len(),
            session_secs: self.opened.elapsed().as_secs(),
            today_secs: today_secs + self.opened.elapsed().as_secs(),
        }
    }

    /// The footer was edited: kept for this book when it has a footer of its own, else for all.
    fn set_footer(&self, c: crate::footer_stats::FooterConfig) {
        {
            let mut cfg = self.shared.cfg.borrow_mut();
            if let Some(own) = cfg.footer_books.get_mut(&self.book_id) {
                *own = c;
            } else {
                cfg.footer = c;
            }
            cfg.save();
        }
        self.update_position_ui();
        crate::settings_bus::footer().notify();
    }

    /// The footer of this book only (it starts as a copy of the common one) or the common one again.
    fn set_footer_scope(&self, own: bool) {
        {
            let mut cfg = self.shared.cfg.borrow_mut();
            if own {
                let c = cfg.footer_for(self.book_id);
                cfg.footer_books.insert(self.book_id, c);
            } else {
                cfg.footer_books.remove(&self.book_id);
            }
            cfg.save();
        }
        self.update_position_ui();
        crate::settings_bus::footer().notify();
    }

    /// The position of the open chapter in the book (1-based).
    fn toc_chapter_no(&self) -> usize {
        self.spine_idx + 1
    }

    /// Reading speed and today's reading come from the statistics; they change slowly.
    fn refresh_live(&self) {
        let now = self.shared.now();
        let hub = self.shared.stats.borrow();
        let s = crate::reading_stats::summarize(&hub.store, now, &chrono::Local);
        self.live.set((s.wpm, s.today_secs));
    }

    fn update_position_ui(&self) {
        let overall = self.overall_fraction();
        self.w.slider.set_value(overall * 1000.0);
        let fc = self.shared.cfg.borrow().footer_for(self.book_id);
        let r = crate::footer_stats::render(&fc, &self.footer_facts());
        self.w.left_label.set_label(&r.left);
        self.w.mid_label.set_label(&r.middle);
        self.w.right_label.set_label(&r.right);
        self.w.slider_row.set_visible(r.bar);
        super::reader_footer::style_labels(
            &[&self.w.left_label, &self.w.mid_label, &self.w.right_label],
            &fc,
        );
        self.w.slider.set_tooltip_text(Some(&trf(
            "{}% of the book",
            &[&format!("{:.0}", overall * 100.0)],
        )));
    }

    fn mark_swatch(&self, color: Option<&str>) {
        for (i, b) in self.w.swatches.iter().enumerate() {
            if color == Some(HIGHLIGHT_COLORS[i].0) {
                b.add_css_class("active");
            } else {
                b.remove_css_class("active");
            }
        }
    }

    fn mark_style(&self, style: HighlightStyle) {
        for (st, b) in &self.w.style_btns {
            b.set_active(*st == style);
        }
    }

    fn highlight_style(&self, id: &str) -> Option<HighlightStyle> {
        self.shared.lib.borrow().get(self.book_id).and_then(|b| {
            b.highlights
                .iter()
                .find(|h| h.id == id)
                .map(|h| HighlightStyle::from_name(&h.style).unwrap_or_default())
        })
    }

    fn highlight_color(&self, id: &str) -> Option<String> {
        self.shared.lib.borrow().get(self.book_id).and_then(|b| {
            b.highlights
                .iter()
                .find(|h| h.id == id)
                .map(|h| h.color.clone())
        })
    }

    fn save_note(&mut self, note: String) {
        let Some(sel) = self.selection.clone() else {
            return;
        };
        {
            let mut lib = self.shared.lib.borrow_mut();
            if let Some(b) = lib.get_mut(self.book_id) {
                if let Some(id) = &sel.highlight {
                    if let Some(h) = b.highlights.iter_mut().find(|h| &h.id == id) {
                        h.note = note.trim().to_string();
                    }
                } else {
                    b.highlights.push(Highlight {
                        id: uuid::Uuid::new_v4().to_string(),
                        spine: self.spine_idx,
                        start: sel.start,
                        end: sel.end,
                        color: "yellow".into(),
                        style: self.hl_style.name().to_string(),
                        note: note.trim().to_string(),
                        text: sel.text.clone(),
                        created: Utc::now(),
                    });
                }
            }
            let _ = lib.save();
        }
        self.w.sel_pop.popdown();
        self.w.sel_stack.set_visible_child_name("actions");
        self.refresh_highlights();
        self.rebuild_lists();
    }

    /// Test hooks of the sidebar and the footer (`reader:side-*`, `reader:footer*`, `reader:bulk-highlights=N`).
    fn dev_side(&mut self, cmd: &str, sender: &ComponentSender<Self>) {
        use super::reader_side::Cmd;
        let (name, arg) = cmd.split_once('=').unwrap_or((cmd, ""));
        let first_id = |me: &Self, n: usize| {
            me.shared.lib.borrow().get(me.book_id).and_then(|b| {
                let mut v: Vec<&Highlight> = b
                    .highlights
                    .iter()
                    .filter(|h| !crate::pdf::is_pdf(h))
                    .collect();
                v.sort_by_key(|h| (h.spine, h.start));
                v.get(n).map(|h| h.id.clone())
            })
        };
        match name {
            "side-open" | "side-close" => {
                self.w.split.set_show_sidebar(name == "side-open");
            }
            "side-tab" => self.w.stack.set_visible_child_name(arg),
            "side-time" => {
                // time from the toggle to the first frame, and the longest gap between frames
                let open = arg != "close";
                let t0 = std::time::Instant::now();
                self.w.split.set_show_sidebar(open);
                trace_frames(&self.w.page, open, t0);
            }
            "side-watch" => {
                // the same for every toggle the user makes
                let page = self.w.page.clone();
                self.w.split.connect_show_sidebar_notify(move |s| {
                    trace_frames(&page, s.shows_sidebar(), std::time::Instant::now());
                });
            }
            "side-state" => {
                eprintln!(
                    "side: shown={} collapsed={} {}",
                    self.w.split.shows_sidebar(),
                    self.w.split.is_collapsed(),
                    self.w.side.dev_summary()
                );
            }
            "side-filter" => {
                // text|colour|style|note
                let p: Vec<&str> = arg.split('|').collect();
                let n = self.w.side.dev_filter(
                    p.first().copied().unwrap_or(""),
                    p.get(1).copied().unwrap_or(""),
                    p.get(2).copied().unwrap_or(""),
                    p.get(3).is_some_and(|v| *v == "note"),
                );
                eprintln!("side_filter: {arg:?} -> {n}");
            }
            "side-act" => {
                // act=N:recolor:green | restyle:underline | note:text | delete | go
                let p: Vec<&str> = arg.splitn(3, ':').collect();
                let n = p.first().and_then(|v| v.parse().ok()).unwrap_or(0);
                let Some(id) = first_id(self, n) else {
                    eprintln!("side_act: no highlight {n}");
                    return;
                };
                let a = p.get(1).copied().unwrap_or("");
                let v = p.get(2).copied().unwrap_or("").to_string();
                match a {
                    "recolor" => sender.input(Input::Side(Cmd::Recolor(id, v))),
                    "restyle" => sender.input(Input::Side(Cmd::Restyle(id, v))),
                    "note" => sender.input(Input::SideNote(id, v)),
                    "delete" => sender.input(Input::Side(Cmd::DeleteHighlight(id))),
                    _ => sender.input(Input::Side(Cmd::GoHighlight(id))),
                }
            }
            "bulk-highlights" => {
                let n: usize = arg.parse().unwrap_or(0);
                {
                    let mut lib = self.shared.lib.borrow_mut();
                    if let Some(b) = lib.get_mut(self.book_id) {
                        let styles = ["background", "underline", "strikethrough", "squiggly"];
                        for i in 0..n {
                            b.highlights.push(Highlight {
                                id: format!("bulk-{i}"),
                                spine: i % self.spine.len().max(1),
                                start: (i / self.spine.len().max(1)) as i32 * 40,
                                end: (i / self.spine.len().max(1)) as i32 * 40 + 30,
                                color: HIGHLIGHT_COLORS[i % 5].0.into(),
                                style: styles[i % 4].into(),
                                note: if i % 3 == 0 {
                                    format!("Note number {i} about this passage")
                                } else {
                                    String::new()
                                },
                                text: format!("Highlighted passage number {i} of the big book"),
                                created: Utc::now(),
                            });
                        }
                    }
                }
                self.refresh_highlights();
                self.rebuild_lists();
            }
            "handles" => {
                self.w.handles.refresh();
                let sel = self.canvas.selected_text().unwrap_or_default();
                eprintln!(
                    "handles: {} popover={} sel={:?}",
                    self.w.handles.dev_dump(),
                    self.w.sel_pop.is_visible(),
                    sel
                );
            }
            "handle-drag" => {
                // handle-drag=start|end:dx:dy
                self.w.handles.refresh();
                let p: Vec<&str> = arg.split(':').collect();
                let ok = self.w.handles.dev_drag(
                    p.first() == Some(&"start"),
                    p.get(1).and_then(|v| v.parse().ok()).unwrap_or(0.0),
                    p.get(2).and_then(|v| v.parse().ok()).unwrap_or(0.0),
                );
                eprintln!("handle_drag: {ok}");
            }
            "handle-clear" => {
                self.canvas.clear_selection();
            }
            "demo-highlights" => {
                // for the screenshots of the wiki: real passages of the open chapter, with colours, styles and notes
                let demos = [
                    (0, 80, "yellow", "background", ""),
                    (
                        300,
                        420,
                        "blue",
                        "underline",
                        "Where the story really starts.",
                    ),
                    (800, 900, "red", "strikethrough", ""),
                    (
                        1300,
                        1420,
                        "green",
                        "squiggly",
                        "Check this against chapter two.",
                    ),
                    (1900, 2000, "purple", "background", "Lovely sentence."),
                ];
                {
                    let mut lib = self.shared.lib.borrow_mut();
                    if let Some(b) = lib.get_mut(self.book_id) {
                        for (i, (from, to, color, style, note)) in demos.iter().enumerate() {
                            b.highlights.push(Highlight {
                                id: format!("demo-{i}"),
                                spine: self.spine_idx,
                                start: *from,
                                end: *to,
                                color: (*color).into(),
                                style: (*style).into(),
                                note: (*note).into(),
                                text: self.canvas.text_between(*from, *to),
                                created: Utc::now(),
                            });
                        }
                    }
                }
                self.refresh_highlights();
                self.rebuild_lists();
            }
            "footer" => {
                let fc = self.shared.cfg.borrow().footer_for(self.book_id);
                eprintln!(
                    "footer: {} | {} | {} bar={} own={} items={} align={:?} size={} opacity={} clock={}",
                    self.w.left_label.label(),
                    self.w.mid_label.label(),
                    self.w.right_label.label(),
                    self.w.slider_row.is_visible(),
                    self.shared.cfg.borrow().footer_books.contains_key(&self.book_id),
                    super::reader_footer::names(&fc),
                    fc.align(),
                    fc.size_pct,
                    fc.opacity_pct,
                    fc.needs_clock(),
                );
            }
            "footer-preset" => {
                let p = crate::footer_stats::Preset::ALL
                    .into_iter()
                    .find(|p| p.label().eq_ignore_ascii_case(arg));
                if let Some(p) = p {
                    self.set_footer(p.config());
                }
            }
            "footer-toggle" => {
                let mut fc = self.shared.cfg.borrow().footer_for(self.book_id);
                if let Some(i) = crate::footer_stats::Item::from_key(arg) {
                    let on = fc.is_on(i);
                    fc.set_on(i, !on);
                    self.set_footer(fc);
                }
            }
            "footer-move" => {
                let mut fc = self.shared.cfg.borrow().footer_for(self.book_id);
                if let Some((a, b)) = arg.split_once(':') {
                    fc.move_item(a.parse().unwrap_or(0), b.parse().unwrap_or(0));
                    self.set_footer(fc);
                }
            }
            "footer-align" => {
                let mut fc = self.shared.cfg.borrow().footer_for(self.book_id);
                fc.align = Some(if arg == "spread" {
                    crate::footer_stats::Align::Spread
                } else {
                    crate::footer_stats::Align::Centered
                });
                self.set_footer(fc);
            }
            "properties" => sender.input(Input::Properties),
            "footer-own" => self.set_footer_scope(arg != "off"),
            "footer-open" => sender.input(Input::FooterConfigure),
            "footer-tick" => {
                self.refresh_live();
                self.update_position_ui();
            }
            _ => {}
        }
    }

    /// The sidebar lies over the page: after a jump it gets out of the way.
    fn close_sidebar_over_page(&self) {
        if self.w.split.is_collapsed() && self.w.split.shows_sidebar() {
            self.w.split.set_show_sidebar(false);
            self.canvas.area.grab_focus();
        }
    }

    /// Changes one highlight of the book and brings the page and the sidebar up to date.
    fn edit_highlight(&self, id: &str, f: impl FnOnce(&mut Highlight)) {
        {
            let mut lib = self.shared.lib.borrow_mut();
            if let Some(h) = lib
                .get_mut(self.book_id)
                .and_then(|b| b.highlights.iter_mut().find(|h| h.id == id))
            {
                f(h);
            }
            let _ = lib.save();
        }
        self.refresh_highlights();
        self.rebuild_lists();
    }

    fn show_properties(&self) {
        let lib = self.shared.lib.borrow();
        let Some(b) = lib.get(self.book_id) else {
            return;
        };
        let m = &b.meta;
        let d = adw::Dialog::new();
        d.set_title(&tr("Properties"));
        d.set_content_width(480);
        d.set_content_height(620);
        let page = adw::PreferencesPage::new();
        let g = adw::PreferencesGroup::new();
        let rows: Vec<(&str, String)> = vec![
            ("Title", m.title.clone()),
            ("Author(s)", m.authors_display()),
            (
                "Series",
                if m.series.is_empty() {
                    String::new()
                } else {
                    format!(
                        "{} #{}",
                        m.series,
                        crate::core::opf::format_index(m.series_index)
                    )
                },
            ),
            ("Publisher", m.publisher.clone()),
            ("Language", m.language.clone()),
            ("Published", m.published.chars().take(10).collect()),
            ("Identifiers", m.identifiers_string()),
            ("Tags", m.tags.join(", ")),
            ("Format", trf("EPUB · {}", &[&human_size(b.size)])),
            ("Sections", self.spine.len().to_string()),
            ("Added", b.added.format("%d %b %Y, %H:%M").to_string()),
            ("Progress", format!("{:.0}%", b.progress * 100.0)),
            ("File", lib.book_path(b).display().to_string()),
        ];
        for (k, v) in rows {
            if v.trim().is_empty() {
                continue;
            }
            let r = adw::ActionRow::builder()
                .title(k)
                .subtitle(glib::markup_escape_text(&v))
                .subtitle_selectable(true)
                .build();
            r.add_css_class("property");
            g.add(&r);
        }
        page.add(&g);
        let tb = adw::ToolbarView::new();
        tb.add_top_bar(&adw::HeaderBar::new());
        tb.set_content(Some(&page));
        d.set_child(Some(&tb));
        d.present(Some(&self.w.page));
    }

    fn update_bookmark_button(&self) {
        let lib = self.shared.lib.borrow();
        let has = lib
            .get(self.book_id)
            .map(|b| {
                b.bookmarks
                    .iter()
                    .any(|m| m.spine == self.spine_idx && (m.frac - self.pos.frac).abs() < 0.04)
            })
            .unwrap_or(false);
        self.w.bookmark_btn.set_icon_name(if has {
            "user-bookmarks-symbolic"
        } else {
            "bookmark-new-symbolic"
        });
        self.w.bookmark_btn.set_tooltip_text(Some(
            &(if has {
                tr("Remove bookmark (Ctrl+B)")
            } else {
                tr("Add bookmark (Ctrl+B)")
            }),
        ));
    }

    fn rebuild_lists(&self) {
        let lib = self.shared.lib.borrow();
        let Some(b) = lib.get(self.book_id) else {
            return;
        };
        let titles: HashMap<usize, String> = b
            .bookmarks
            .iter()
            .map(|m| m.spine)
            .chain(b.highlights.iter().map(|h| h.spine))
            .map(|i| (i, self.chapter_title(i)))
            .collect();
        let bms = b
            .bookmarks
            .iter()
            .map(|m| super::reader_side::BmItem {
                id: m.id.clone(),
                title: titles.get(&m.spine).cloned().unwrap_or_default(),
                sub: format!("{} · {}", m.label, m.created.format("%d %b %Y")),
            })
            .collect();
        self.w.side.set_bookmarks(bms);
        let entries = b
            .highlights
            .iter()
            .filter(|h| !crate::pdf::is_pdf(h))
            .map(|h| crate::side_notes::Entry {
                id: h.id.clone(),
                spine: h.spine,
                start: h.start,
                chapter: titles.get(&h.spine).cloned().unwrap_or_default(),
                text: h.text.clone(),
                note: h.note.clone(),
                color: h.color.clone(),
                style: h.style.clone(),
                created: h.created,
            })
            .collect();
        self.w.side.set_entries(entries);
    }

    fn apply_prefs(&self) {
        let prefs = self.shared.cfg.borrow().reader.clone();
        let dark = system_is_dark();
        let mut style = prefs.canvas_style(dark);
        let tweaks = self.shared.cfg.borrow().tweaks.effective(self.book_id);
        tweaks.apply(&mut style);
        if self.fixed {
            // pages of a fixed-layout book are shown whole, one at a time
            style.fixed = true;
            style.layout = LayoutMode::SingleColumn;
            style.margin = 8.0;
        }
        self.canvas
            .set_language(&tweaks.hyphenation_language(&self.book.metadata().language));
        if crate::eink::is_active() {
            style.palette = crate::eink::palette();
        } else if adw::StyleManager::default().is_high_contrast()
            && prefs.colors == ColorMode::Theme
        {
            style.palette = crate::a11y::high_contrast_palette(dark);
        }
        self.canvas.set_style(style);
        self.canvas.set_tap_zones(prefs.tap_zones);
        if crate::eink::is_active() {
            // bars that already faded come back and stay
            self.w.page.remove_css_class("chrome-idle");
        }
        for c in [
            "reader-light",
            "reader-sepia",
            "reader-dark",
            "reader-black",
        ] {
            self.w.page.remove_css_class(c);
        }
        self.w.page.add_css_class(prefs.chrome_class(dark));
        // keep the menu widgets and the shared prefs copy in sync (no feedback: handlers compare first)
        *self.cur_prefs.borrow_mut() = prefs.clone();
        self.w.colors.sync(&prefs);
        self.w.themes_ui.sync(&prefs);
        for (t, b) in &self.w.theme_btns {
            if *t == prefs.theme && prefs.colors == ColorMode::Theme && !b.is_active() {
                b.set_active(true);
            }
        }
        self.w.columns_dd.set_selected(match prefs.layout {
            LayoutMode::TwoColumns => 0,
            LayoutMode::SingleColumn => 1,
            LayoutMode::Scroll => 2,
        });
        if self.w.hyphen_sw.is_active() != prefs.hyphenate {
            self.w.hyphen_sw.set_active(prefs.hyphenate);
        }
        for (m, b) in &self.w.layout_btns {
            if *m == prefs.layout && !b.is_active() {
                b.set_active(true);
            }
        }
        self.w
            .zoom_label
            .set_label(&format!("{:.0}%", prefs.font_size / 14.0 * 100.0));
        self.chapter_pages.borrow_mut().clear();
        self.update_position_ui();
    }

    /// Shows a look-up page (dictionary, Wikipedia, translation) in the selection popover. The
    /// page has another size than the buttons, and a popover that is only resized in place can
    /// vanish on a real compositor, so it is presented again at the new size.
    fn show_lookup_page(&self, name: &str) {
        let w = &self.w;
        repopup(&w.sel_pop, &w.switching, || {
            w.sel_stack.set_visible_child_name(name)
        });
    }

    /// Opens the visible selection popover again at its current size.
    fn represent_popover(&self) {
        let w = &self.w;
        if w.sel_pop.is_visible() {
            repopup(&w.sel_pop, &w.switching, || {});
        }
    }

    fn show_selection_popover(&mut self, info: &SelectionInfo) {
        // the popover points beyond the handles, so it does not cover them
        let (x, y, w, h) = self.w.handles.popover_rect(info.rect);
        self.w
            .sel_pop
            .set_pointing_to(Some(&gdk::Rectangle::new(x, y, w.max(1), h.max(1))));
        self.w.sel_stack.set_visible_child_name("actions");
        self.w.sel_remove.set_visible(info.highlight.is_some());
        let c = info
            .highlight
            .as_ref()
            .and_then(|id| self.highlight_color(id));
        self.mark_swatch(c.as_deref());
        let st = info
            .highlight
            .as_ref()
            .and_then(|id| self.highlight_style(id))
            .unwrap_or(self.hl_style);
        self.mark_style(st);
        // pre-fill the note of an existing highlight
        let note = info
            .highlight
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
        self.w.note_view.buffer().set_text(&note);
        self.w.note_label.set_text(&note);
        self.w.note_preview.set_visible(!note.trim().is_empty());
        self.w.sel_pop.popup();
    }

    // ---- read aloud ------------------------------------------------------------------------

    /// The language the book is read in: its metadata, else what the text looks like.
    /// The language of the book for its dictionary: `dc:language` (`en-GB` is `en`), else a guess from
    /// the text on the page. Empty while nothing tells.
    fn dict_language(&self) -> String {
        if let Some(l) = self.dict_lang.borrow().clone().filter(|l| !l.is_empty()) {
            return l;
        }
        let sample: String = self.canvas.canonical_text().chars().take(3000).collect();
        let l = crate::dict_catalog::book_language(&self.book.metadata().language, &sample);
        if !l.is_empty() {
            *self.dict_lang.borrow_mut() = Some(l.clone());
        }
        l
    }

    /// Shows what the dictionary page says besides the definition (the offer to download).
    fn show_dict_prompt(&self, p: &crate::dict_auto::Prompt) {
        let u = &self.w.dict_prompt;
        u.root.set_visible(p.visible());
        u.title.set_label(&p.heading());
        u.body.set_label(&p.body());
        u.primary.set_label(&p.primary());
        u.primary.set_sensitive(p.primary_enabled());
        match p.progress() {
            Some(f) => {
                u.bar.set_visible(true);
                u.bar.set_fraction(f);
            }
            None => u.bar.set_visible(false),
        }
        *self.dict_prompt_state.borrow_mut() = p.clone();
    }

    /// Fills the dictionary page for `term`: the definition from the installed dictionaries, and the offer
    /// to download one when the language of the book has none.
    fn fill_dict(&self, term: &str) {
        let dir = crate::dictionaries::dictionary_dir();
        let loaded = crate::dictionaries::load(&dir);
        let lang = self.dict_language();
        let presence = crate::dict_install::presence(&dir);
        let prompt = crate::dict_auto::prompt_for(
            &lang,
            &presence,
            crate::dict_catalog::catalog(),
            super::dict_service::running(&lang),
            super::dict_service::last_failure(&lang),
        );
        self.show_dict_prompt(&prompt);
        let text = if loaded.set.names().is_empty() {
            if prompt.visible() {
                String::new()
            } else {
                glib::markup_escape_text(&crate::dictionaries::empty_help(&dir)).to_string()
            }
        } else {
            let lookup = loaded.set.lookup_all(term);
            let mut t = crate::dictionaries::render(term, &lookup);
            let definition = lookup
                .results
                .first()
                .and_then(|r| r.entries.first())
                .map(|e| e.plain_text());
            if definition.is_some() {
                let sel = self
                    .selection
                    .as_ref()
                    .map(|s| s.text.clone())
                    .unwrap_or_default();
                let now = self.shared.now();
                let added = self.shared.vocab.borrow_mut().record_lookup(
                    term,
                    &sel,
                    self.book_id,
                    definition.as_deref(),
                    now,
                );
                if matches!(added, Ok(true)) {
                    t.push_str("\n\n<small>Saved to your vocabulary.</small>");
                }
            }
            for (d, e) in &loaded.failed {
                t.push_str(&format!(
                    "\n\n<small>{} could not be opened: {}</small>",
                    glib::markup_escape_text(d),
                    glib::markup_escape_text(e)
                ));
            }
            t
        };
        self.w.dict_text.set_markup(&text);
    }

    fn tts_language(&self) -> String {
        let sample: String = self.canvas.canonical_text().chars().take(3000).collect();
        crate::tts::book_language(&self.book.metadata().language, &sample)
    }

    fn tts_engine(&mut self) -> bool {
        if self.tts.engine.is_some() {
            return true;
        }
        let prefs = self.shared.cfg.borrow().tts.clamped();
        let language = self.tts_language();
        // a book with its own narration (EPUB 3 media overlay) is read with that
        let narration = crate::overlay_play::open_engine(&self.book);
        let has_narration = narration.is_some();
        match narration.map_or_else(|| crate::tts::open_engine(&prefs, &language), Ok) {
            Ok(opened) => {
                let id = if has_narration {
                    "narration"
                } else {
                    opened.choice.map_or("mock", |c| c.id())
                };
                let title = if has_narration {
                    "Narration of the book"
                } else {
                    opened.choice.map_or("Silent test voice", |c| c.title())
                };
                let mut e = opened.engine;
                e.set_book(&self.book_id.to_string());
                e.set_language(&language);
                e.set_speed(prefs.speed);
                e.set_pitch(prefs.pitch);
                e.set_voice(&prefs.voice_for(id, &language));
                e.set_volume(prefs.volume);
                self.w.tts.player.set_volume(prefs.volume);
                self.tts.engine = Some(e);
                self.tts.engine_id = id.to_string();
                self.tts.language = language;
                self.w.tts.set_engine(title, opened.notice.as_deref());
                if let Some(n) = opened.notice {
                    let _ = self.sender.output(Output::Toast(n));
                }
                true
            }
            Err(err) => {
                if let Some(nr) = err.downcast_ref::<crate::tts::NotReady>() {
                    self.tts.not_ready = Some(nr.clone());
                } else {
                    let human = tr(&crate::tts::setup::human_error(&format!("{err:#}")));
                    self.tts.notice = Some(crate::tts_player::Phase::Failed(human.clone()));
                    let _ = self.sender.output(Output::Toast(human));
                    self.tts_sync_player();
                }
                false
            }
        }
    }

    fn tts_load_voices(&mut self) {
        if !self.tts_engine() {
            return;
        }
        let all = self
            .tts
            .engine
            .as_deref_mut()
            .map(|e| e.voices())
            .unwrap_or_default();
        let list = crate::tts::voices_for(&all, &self.tts.language, 80);
        let current = self
            .shared
            .cfg
            .borrow()
            .tts
            .voice_for(&self.tts.engine_id, &self.tts.language);
        self.w.tts.set_voices(&list, &current);
    }

    /// Reads on from the sentence being spoken (after the voice or the speed changed).
    fn tts_restart(&mut self) {
        let update = match self.tts.engine.as_deref_mut() {
            Some(e) => self.tts.session.restart(e),
            None => None,
        };
        self.tts_apply(update.into_iter().collect());
    }

    fn tts_command(&mut self, cmd: super::reader_tts::Cmd) {
        use super::reader_tts::Cmd;
        let remember = matches!(
            cmd,
            Cmd::PlayPause
                | Cmd::Skip(_)
                | Cmd::Seek(_)
                | Cmd::Chapter(_)
                | Cmd::Speed(_)
                | Cmd::Voice(_)
        );
        self.tts_command_inner(cmd);
        if remember {
            self.save_cards(true);
        }
    }

    fn tts_command_inner(&mut self, cmd: super::reader_tts::Cmd) {
        use super::reader_tts::Cmd;
        use crate::tts::State;
        match cmd {
            Cmd::Opened => self.tts_load_voices(),
            Cmd::Stop => self.tts_stop(),
            Cmd::PlayPause => match self.tts.session.state() {
                State::Playing => {
                    if let Some(e) = self.tts.engine.as_deref_mut() {
                        self.tts.session.pause(e);
                    }
                    self.w.tts.set_playing(false);
                    self.tts_sync_player();
                }
                State::Paused => {
                    self.rsvp_pause();
                    if let Some(e) = self.tts.engine.as_deref_mut() {
                        self.tts.session.resume(e);
                    }
                    self.w.tts.set_playing(true);
                    self.tts_sync_player();
                }
                State::Stopped if self.tts.prep.is_some() => {}
                State::Stopped if self.tts.parked.is_some() => {
                    // a card that waited at a saved place: that chapter, from that sentence
                    self.tts.notice = None;
                    let p = self.tts.parked.clone().unwrap_or_else(|| self.tts_place());
                    let chapter = p.chapter.min(self.spine.len().saturating_sub(1));
                    if chapter != self.spine_idx {
                        self.tts.advancing = true;
                        self.load_chapter(chapter, Some(0.0), None, false);
                        self.tts.advancing = false;
                    }
                    let len = self.canvas.canonical_text().chars().count() as i32;
                    self.tts_start(if p.offset >= len { 0 } else { p.offset });
                }
                State::Stopped => {
                    self.tts.notice = None;
                    let from = self
                        .selection
                        .as_ref()
                        .map_or_else(|| self.canvas.visible_start_char(), |s| s.start);
                    self.tts_start(from);
                }
            },
            Cmd::Skip(delta) => {
                let update = match self.tts.engine.as_deref_mut() {
                    Some(e) => self.tts.session.skip(e, delta),
                    None => None,
                };
                self.tts_apply(update.into_iter().collect());
            }
            Cmd::Speed(v) => {
                let prefs = {
                    let mut cfg = self.shared.cfg.borrow_mut();
                    cfg.tts.speed = v;
                    cfg.tts = cfg.tts.clamped();
                    cfg.save();
                    cfg.tts.clone()
                };
                if let Some(e) = self.tts.engine.as_deref_mut() {
                    e.set_speed(prefs.speed);
                }
                self.w.tts.show_speed(prefs.speed);
                self.tts_restart();
                self.tts_sync_player();
            }
            Cmd::Seek(at) => {
                let update = match self.tts.engine.as_deref_mut() {
                    Some(e) => self.tts.session.seek(e, at),
                    None => None,
                };
                self.tts_apply(update.into_iter().collect());
            }
            Cmd::Chapter(delta) => self.tts_goto_chapter(delta),
            Cmd::Volume(v) => {
                {
                    let mut cfg = self.shared.cfg.borrow_mut();
                    cfg.tts.volume = v;
                    cfg.tts = cfg.tts.clamped();
                    cfg.save();
                }
                if let Some(e) = self.tts.engine.as_deref_mut() {
                    e.set_volume(v);
                }
            }
            Cmd::CancelPrep => self.tts_stop(),
            Cmd::Layout => self.save_cards(true),
            Cmd::DownloadVoice(always) => {
                if always {
                    let mut cfg = self.shared.cfg.borrow_mut();
                    cfg.tts.download_without_asking = true;
                    cfg.save();
                }
                self.tts_download_start();
            }
            Cmd::Pitch(v) => {
                let prefs = {
                    let mut cfg = self.shared.cfg.borrow_mut();
                    cfg.tts.pitch = v;
                    cfg.tts = cfg.tts.clamped();
                    cfg.save();
                    cfg.tts.clone()
                };
                if let Some(e) = self.tts.engine.as_deref_mut() {
                    e.set_pitch(prefs.pitch);
                }
                self.tts_restart();
            }
            Cmd::Voice(name) => {
                {
                    let mut cfg = self.shared.cfg.borrow_mut();
                    let (id, lang) = (self.tts.engine_id.clone(), self.tts.language.clone());
                    cfg.tts.set_voice_for(&id, &lang, &name);
                    cfg.save();
                }
                if let Some(e) = self.tts.engine.as_deref_mut() {
                    e.set_voice(&name);
                }
                self.w.tts.player.set_voice_name(&name);
                self.tts_restart();
            }
            Cmd::Sleep(t) => {
                self.tts.sleep = t;
            }
            Cmd::Settings => {
                let _ = self
                    .sender
                    .output(Output::OpenPreferences("Voice engine".into()));
            }
            Cmd::SaveAudio(whole) => {
                let chapters = self.audio_chapters(whole);
                let title = self
                    .shared
                    .lib
                    .borrow()
                    .get(self.book_id)
                    .map(|b| b.meta.title.clone())
                    .unwrap_or_default();
                let tx = self.sender.output_sender().clone();
                super::reader_tts::save_audio(
                    self.w.page.upcast_ref(),
                    &self.shared,
                    whole,
                    &title,
                    chapters,
                    self.tts_language(),
                    Rc::new(move |m| {
                        let _ = tx.send(Output::Toast(m));
                    }),
                    None,
                );
            }
        }
    }

    /// (title, text) of the chapter on screen, or of every chapter of the book, for *Save as audio*.
    fn audio_chapters(&self, whole: bool) -> Vec<(String, String)> {
        if !whole {
            return vec![(
                self.chapter_title(self.spine_idx),
                self.canvas.canonical_text(),
            )];
        }
        (0..self.spine.len())
            .map(|i| {
                let text = self
                    .book
                    .chapter_doc_layered(&self.spine[i], false, "")
                    .map(|d| {
                        d.blocks
                            .iter()
                            .map(|b| b.plain_text())
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                (self.chapter_title(i), text)
            })
            .collect()
    }

    /// Runs a speed-reading command. Only one of read aloud and speed reading speaks or moves at a
    /// time: starting this one pauses the other (its card stays, with its place).
    fn rsvp_run(&mut self, c: super::rsvp::Cmd) {
        if c == super::rsvp::Cmd::PlayPause && !self.rsvp.playing() {
            self.tts_pause();
        }
        let ev = self.rsvp.command(c, &self.canvas);
        self.rsvp_event(ev);
        self.sync_chrome_playing();
        self.save_cards(true);
    }

    /// Tells the reader focus whether something plays (the short idle delay then applies).
    fn sync_chrome_playing(&self) {
        let playing = self.tts.session.state() == crate::tts::State::Playing || self.rsvp.playing();
        let was = self.w.chrome.playing.replace(playing);
        if playing && !was {
            // play was just pressed: the countdown starts now
            self.w.chrome.touch();
        }
    }

    /// Pauses read aloud (the card stays) because speed reading started.
    fn tts_pause(&mut self) {
        if self.tts.session.state() == crate::tts::State::Playing {
            if let Some(e) = self.tts.engine.as_deref_mut() {
                self.tts.session.pause(e);
            }
            self.w.tts.set_playing(false);
            self.tts_sync_player();
        }
    }

    /// Pauses speed reading (the card stays) because read aloud started.
    fn rsvp_pause(&mut self) {
        if self.rsvp.playing() {
            let ev = self.rsvp.command(super::rsvp::Cmd::PlayPause, &self.canvas);
            self.rsvp_event(ev);
        }
    }

    /// Speed reading ended or ran out of words: remember the speed, or go on with the next chapter.
    fn rsvp_event(&mut self, ev: Option<super::rsvp::Event>) {
        use super::rsvp::Event;
        match ev {
            Some(Event::Closed(wpm)) => {
                let mut cfg = self.shared.cfg.borrow_mut();
                cfg.rsvp_wpm = wpm;
                cfg.save();
            }
            Some(Event::TextEnd) => {
                let mut idx = self.spine_idx;
                loop {
                    idx += 1;
                    if idx >= self.spine.len() {
                        let _ = self.sender.output(Output::Toast(tr("Finished reading")));
                        return;
                    }
                    self.tts.advancing = true;
                    self.load_chapter(idx, Some(0.0), None, false);
                    self.tts.advancing = false;
                    let text = self.canvas.canonical_text();
                    if !text.trim().is_empty() {
                        self.rsvp.continue_with(&self.canvas, &text, self.spine_idx);
                        if self.rsvp.is_on() {
                            let ev = self.rsvp.command(super::rsvp::Cmd::PlayPause, &self.canvas);
                            self.rsvp_event(ev);
                        }
                        return;
                    }
                }
            }
            Some(Event::Unpark(k)) => {
                // play on a card that waited at a saved place: its chapter, its word, then run
                let chapter = k.chapter.min(self.spine.len().saturating_sub(1));
                if chapter != self.spine_idx {
                    self.tts.advancing = true;
                    self.load_chapter(chapter, Some(0.0), None, false);
                    self.tts.advancing = false;
                }
                let text = self.canvas.canonical_text();
                let len = text.chars().count() as i32;
                let from = if k.offset >= len { 0 } else { k.offset.max(0) } as usize;
                self.rsvp.start(&self.canvas, &text, from, k.wpm, chapter);
                self.rsvp_run(super::rsvp::Cmd::PlayPause);
            }
            None => {}
        }
    }

    /// Starts reading the chapter at character `from`.
    fn tts_start(&mut self, from: i32) {
        self.rsvp_pause();
        self.tts.notice = None;
        if !self.tts_engine() {
            if let Some(nr) = self.tts.not_ready.take() {
                self.tts.want_from = from;
                self.tts_not_ready(nr);
            }
            return;
        }
        self.tts.parked = None;
        let text = self.canvas.canonical_text();
        let update = match self.tts.engine.as_deref_mut() {
            Some(e) => {
                self.tts.session.load(e, &text);
                self.tts.session.play_from(e, from)
            }
            None => None,
        };
        self.tts_apply(update.into_iter().collect());
        let first = match self.tts.engine.as_deref_mut() {
            Some(e) => self.tts.session.tick(e),
            None => Vec::new(),
        };
        self.tts_apply(first);
        if self.tts.timer.is_none() && self.tts.session.state() != crate::tts::State::Stopped {
            let tx = self.sender.input_sender().clone();
            self.tts.timer = Some(super::reader_tts::every_tick(move || {
                tx.emit(Input::TtsTick)
            }));
        }
    }

    fn tts_apply(&mut self, updates: Vec<crate::tts::Update>) {
        use crate::tts::Update;
        for u in updates {
            match u {
                Update::Speaking(a, b) => {
                    self.canvas.set_speaking(Some((a, b)));
                    self.tts_prefetch_next();
                    if self.tts.sleep.expired(std::time::Instant::now()) {
                        self.tts_stop();
                        let _ = self
                            .sender
                            .output(Output::Toast(tr("Sleep timer: reading stopped")));
                        return;
                    }
                }
                Update::Finished => {
                    if self.tts.sleep.stops_at_chapter_end() {
                        self.tts_stop();
                        let _ = self.sender.output(Output::Toast(tr(
                            "Sleep timer: stopped at the end of the chapter",
                        )));
                        return;
                    }
                    self.tts_next_chapter();
                    return;
                }
                Update::Error(m) => {
                    // plain words and what to do, with Try again on the card; never the raw engine text
                    let human = tr(&crate::tts::setup::human_error(&m));
                    let keep = self.tts_place();
                    self.tts_stop();
                    self.tts.parked = Some(keep);
                    self.tts.notice = Some(crate::tts_player::Phase::Failed(human.clone()));
                    let _ = self.sender.output(Output::Toast(human));
                    self.tts_sync_player();
                    return;
                }
            }
        }
        self.w
            .tts
            .set_playing(self.tts.session.state() == crate::tts::State::Playing);
        self.tts_sync_player();
    }

    /// Near the end of the chapter, lets the engine make the first sentences of the next one
    /// ahead of time, so the chapter change has no gap.
    fn tts_prefetch_next(&mut self) {
        if self.tts.prefetched == Some(self.spine_idx) {
            return;
        }
        let sentences = self.tts.session.sentences();
        let Some(cur) = self.tts.session.current() else {
            return;
        };
        let Some(at) = sentences.iter().position(|s| s.start == cur.start) else {
            return;
        };
        if at + 2 < sentences.len() {
            return;
        }
        self.tts.prefetched = Some(self.spine_idx);
        let texts = (self.spine_idx + 1..self.spine.len().min(self.spine_idx + 4))
            .find_map(|i| {
                let text = self
                    .book
                    .chapter_doc_layered(&self.spine[i], false, "")?
                    .blocks
                    .iter()
                    .map(|b| b.plain_text())
                    .collect::<Vec<_>>()
                    .join("\n");
                let first: Vec<String> = crate::tts::split_sentences(&text)
                    .into_iter()
                    .take(2)
                    .map(|s| s.text)
                    .collect();
                (!first.is_empty()).then_some(first)
            })
            .unwrap_or_default();
        if let (false, Some(e)) = (texts.is_empty(), self.tts.engine.as_deref_mut()) {
            e.prefetch(&texts);
        }
    }

    /// The chapter is read: go on with the next one that has text.
    fn tts_next_chapter(&mut self) {
        let mut idx = self.spine_idx;
        loop {
            idx += 1;
            if idx >= self.spine.len() {
                self.tts_stop();
                let _ = self.sender.output(Output::Toast(tr("Finished reading")));
                return;
            }
            self.tts.advancing = true;
            self.load_chapter(idx, Some(0.0), None, false);
            self.tts.advancing = false;
            if !crate::tts::split_sentences(&self.canvas.canonical_text()).is_empty() {
                self.tts_start(0);
                return;
            }
        }
    }

    fn tts_stop(&mut self) {
        if let Some(e) = self.tts.engine.as_deref_mut() {
            self.tts.session.stop(e);
        }
        if self.tts.prep.take().is_some() {
            // the user stopped it from the card: the job is cancelled (leaving the book does not)
            crate::voice_download::cancel();
        }
        self.tts.notice = None;
        if let Some(t) = self.tts.timer.take() {
            t.remove();
        }
        self.tts.sleep = crate::tts::SleepTimer::Off;
        self.w.tts.set_sleep_off();
        self.canvas.set_speaking(None);
        self.w.tts.set_playing(false);
        self.tts.parked = None;
        self.tts_sync_player();
        self.save_cards(true);
    }

    /// Read aloud was asked for but nothing can speak yet.
    fn tts_not_ready(&mut self, nr: crate::tts::NotReady) {
        use crate::tts::NotReady;
        use crate::tts_player::Phase;
        match nr {
            NotReady::NoVoice => {
                if self.shared.cfg.borrow().tts.download_without_asking {
                    self.tts_download_start();
                } else {
                    self.tts_offer_download();
                }
            }
            NotReady::NoEspeak => {
                let cmd = if crate::tts::setup::in_flatpak() {
                    None
                } else {
                    crate::tts::espeak_install_command()
                };
                self.w
                    .tts
                    .player
                    .set_install_command(cmd.as_deref().unwrap_or(""));
                self.tts.notice = Some(Phase::NeedsEspeak);
                self.tts_arm_timer();
                self.tts_sync_player();
            }
            NotReady::Language(l) => {
                let msg = trf(
                    "No downloaded voice reads {}. Download a voice for it in Preferences → Text to Speech.",
                    &[&l],
                );
                let _ = self.sender.output(Output::Toast(msg.clone()));
                self.tts.notice = Some(Phase::Failed(msg));
                self.tts_sync_player();
            }
        }
    }

    /// The first time: a short dialog with the licence note and the size.
    fn tts_offer_download(&mut self) {
        let prefs = self.shared.cfg.borrow().tts.clamped();
        let Some(item) = crate::tts::default_voice(&prefs) else {
            return;
        };
        let body = trf(
            "Read aloud needs a voice. Bookx can download the natural voice Kokoro ({}) in the background and start reading as soon as it is ready.\n\nLicence: {}\n{}",
            &[
                &bookx_tts::items::format_size(item.download_bytes.max(1)),
                &item.license_name,
                &item.license_note,
            ],
        );
        self.tts.offered += 1;
        let d = adw::AlertDialog::new(Some(&tr("Download a voice?")), Some(&body));
        let always = gtk::CheckButton::with_label(&tr("Always do this"));
        super::audit::label(&always, &tr("Always download the voice without asking"));
        d.set_extra_child(Some(&always));
        d.add_responses(&[("cancel", &tr("Not now")), ("go", &tr("Download"))]);
        d.set_response_appearance("go", adw::ResponseAppearance::Suggested);
        d.set_default_response(Some("go"));
        d.set_close_response("cancel");
        let tx = self.sender.input_sender().clone();
        d.connect_response(None, move |_, r| {
            if r == "go" {
                tx.emit(Input::Tts(super::reader_tts::Cmd::DownloadVoice(
                    always.is_active(),
                )));
            }
        });
        d.present(Some(self.w.page.upcast_ref::<gtk::Widget>()));
    }

    /// Asks for the download of the default voice (an application-level job that goes on when this
    /// book is closed); the clock follows it and reading starts when it is done.
    fn tts_download_start(&mut self) {
        if self.tts.prep.is_some() {
            return;
        }
        let prefs = self.shared.cfg.borrow().tts.clamped();
        let seq = crate::voice_download::start(
            &tr("Downloading the default voice (Kokoro)"),
            crate::tts::default_voice_packs(&prefs),
        );
        self.tts.prep = Some(TtsPrep {
            seq,
            done: 0,
            total: 0,
        });
        self.tts.notice = None;
        self.tts_arm_timer();
        self.tts_sync_player();
    }

    /// Makes sure the clock that polls the engine and the download runs.
    fn tts_arm_timer(&mut self) {
        if self.tts.timer.is_none() {
            let tx = self.sender.input_sender().clone();
            self.tts.timer = Some(super::reader_tts::every_tick(move || {
                tx.emit(Input::TtsTick)
            }));
        }
    }

    /// Takes in what the download thread reported; starts reading when the voice is there.
    fn tts_poll_prep(&mut self) {
        use super::reader_tts::Cmd;
        use crate::mpris::Control;
        let controls = self
            .tts
            .mpris
            .as_ref()
            .map(|m| m.controls())
            .unwrap_or_default();
        for c in controls {
            let playing = self.tts.session.state() == crate::tts::State::Playing;
            match c {
                Control::Play if playing => {}
                Control::Pause if !playing => {}
                Control::Play | Control::Pause | Control::PlayPause => {
                    self.tts_command(Cmd::PlayPause)
                }
                Control::Stop => self.tts_command(Cmd::Stop),
                Control::Next => self.tts_command(Cmd::Skip(1)),
                Control::Previous => self.tts_command(Cmd::Skip(-1)),
            }
        }
        let Some(seq) = self.tts.prep.as_ref().map(|p| p.seq) else {
            return;
        };
        use crate::voice_download::Status;
        match crate::voice_download::status() {
            Status::Running { done, total, .. } => {
                if let Some(p) = self.tts.prep.as_mut() {
                    p.done = done;
                    p.total = total;
                }
                self.tts_sync_player();
            }
            Status::Finished { seq: s, result } if s >= seq => {
                self.tts.prep = None;
                match result {
                    Ok(()) => {
                        let from = self.tts.want_from;
                        self.tts_start(from);
                    }
                    Err(crate::tts::Failure::Cancelled) => self.tts_stop(),
                    Err(f) => {
                        // the application already showed a toast for it
                        let msg = tr(&crate::tts::setup::failure_message(&f));
                        self.tts.notice = Some(crate::tts_player::Phase::Failed(msg));
                        self.tts_sync_player();
                    }
                }
            }
            _ => {}
        }
    }

    /// The phase the player card shows.
    fn tts_phase(&self) -> crate::tts_player::Phase {
        use crate::tts_player::Phase;
        if let Some(p) = &self.tts.prep {
            return Phase::Preparing(crate::tts::setup::percent(p.done, p.total));
        }
        if let Some(n) = &self.tts.notice {
            return n.clone();
        }
        match self.tts.session.state() {
            crate::tts::State::Playing => Phase::Playing,
            crate::tts::State::Paused => Phase::Paused,
            crate::tts::State::Stopped if self.tts.parked.is_some() => Phase::Idle,
            crate::tts::State::Stopped => Phase::Stopped,
        }
    }

    /// Shows the state of read aloud on the player card.
    fn tts_sync_player(&mut self) {
        let parked = self
            .tts
            .parked
            .clone()
            .filter(|_| self.tts.session.index().is_none());
        let card_chapter = parked.as_ref().map_or(self.spine_idx, |p| p.chapter);
        let model = crate::tts_player::Model {
            phase: self.tts_phase(),
            sentence: self
                .tts
                .session
                .index()
                .or(parked.as_ref().map(|p| p.sentence))
                .unwrap_or(0),
            sentences: parked
                .as_ref()
                .map_or(self.tts.session.sentences().len(), |p| p.sentences),
            chapter: card_chapter,
            chapters: self.spine.len(),
            speed: self.shared.cfg.borrow().tts.speed,
            collapsed: self.w.tts.player.collapsed(),
        };
        let visible = model.phase != crate::tts_player::Phase::Stopped;
        if visible && self.tts.shown_chapter != Some(card_chapter) {
            self.tts.shown_chapter = Some(card_chapter);
            let (title, cover) = {
                let lib = self.shared.lib.borrow();
                match lib.get(self.book_id) {
                    Some(b) => (
                        b.meta.title.clone(),
                        lib.cover_path(b)
                            .and_then(|p| super::covers::thumbnail(&p, 16)),
                    ),
                    None => (String::new(), None),
                }
            };
            self.w
                .tts
                .player
                .set_book(&title, &self.chapter_title(card_chapter), cover.as_ref());
        }
        if !visible {
            self.tts.shown_chapter = None;
            self.tts.mpris = None;
        } else if self.tts.mpris.is_none() && std::env::var_os("BOOKX_NO_MPRIS").is_none() {
            self.tts.mpris = crate::mpris::Mpris::start(None);
        }
        if let Some(m) = &self.tts.mpris {
            let (title, chapter) = (
                self.w.tts.player.book_title(),
                self.chapter_title(self.spine_idx),
            );
            let v = crate::tts_player::view(&model);
            m.update(crate::mpris::Snapshot {
                status: crate::mpris::playback_status(&model.phase),
                title,
                chapter,
                can_next: v.next_sentence,
                can_previous: v.prev_sentence,
            });
        }
        let sentence = self
            .tts
            .session
            .current()
            .map(|s| s.text.clone())
            .or(parked.map(|p| p.text))
            .unwrap_or_default();
        self.w.tts.player.set_sentence(&sentence);
        self.w.tts.player.apply(&model);
        self.sync_chrome_playing();
        self.save_cards(false);
    }

    /// Read aloud waits at its place (the chapter was left): the card stays, nothing plays.
    fn tts_park(&mut self) {
        let parked = self.tts_place();
        if let Some(e) = self.tts.engine.as_deref_mut() {
            self.tts.session.stop(e);
        }
        if let Some(t) = self.tts.timer.take() {
            t.remove();
        }
        self.tts.sleep = crate::tts::SleepTimer::Off;
        self.w.tts.set_sleep_off();
        self.canvas.set_speaking(None);
        self.w.tts.set_playing(false);
        self.tts.parked = Some(parked);
        self.tts_sync_player();
        self.save_cards(true);
    }

    /// Where read aloud is: the sentence being read, the place it waits at, or the top of the page.
    fn tts_place(&self) -> TtsParked {
        if let Some(i) = self.tts.session.index() {
            let s = &self.tts.session.sentences()[i];
            return TtsParked {
                chapter: self.spine_idx,
                offset: s.start,
                sentence: i,
                sentences: self.tts.session.sentences().len(),
                text: s.text.clone(),
            };
        }
        if let Some(p) = &self.tts.parked {
            return p.clone();
        }
        TtsParked {
            chapter: self.spine_idx,
            offset: self.tts.want_from.max(0),
            sentence: 0,
            sentences: 0,
            text: String::new(),
        }
    }

    /// The cards of this book as they are now.
    fn cards_now(&self) -> crate::reader_cards::BookCards {
        use crate::reader_cards::{BookCards, RsvpCard, Top, TtsCard};
        let path = |c: usize| self.spine.get(c).cloned().unwrap_or_default();
        let tts = (self.tts_phase() != crate::tts_player::Phase::Stopped).then(|| {
            let p = self.tts_place();
            let cfg = self.shared.cfg.borrow();
            let pos = self.w.tts.player.pos.get();
            TtsCard {
                x: pos.0,
                y: pos.1,
                compact: self.w.tts.player.collapsed(),
                chapter: p.chapter,
                chapter_path: path(p.chapter),
                sentence: p.sentence,
                sentences: p.sentences,
                offset: p.offset,
                text: p.text,
                speed: cfg.tts.speed,
                voice: cfg.tts.voice_for(&self.tts.engine_id, &self.tts.language),
            }
        });
        let rsvp = self.rsvp.snapshot().map(|k| {
            let pos = self.rsvp.pos.get();
            RsvpCard {
                x: pos.0,
                y: pos.1,
                chapter: k.chapter,
                chapter_path: path(k.chapter),
                word: k.word,
                words: k.words,
                offset: k.offset,
                text: k.text,
                wpm: k.wpm,
            }
        });
        BookCards {
            tts,
            rsvp,
            top: if super::card_drag::top() == 1 {
                Top::Rsvp
            } else {
                Top::Tts
            },
        }
    }

    /// Writes the cards to the config. Unforced writes are spaced out (reading moves on all the time).
    fn save_cards(&mut self, force: bool) {
        if self.tts.cards_frozen {
            return;
        }
        let due = self
            .tts
            .cards_saved
            .is_none_or(|t| t.elapsed() >= std::time::Duration::from_millis(1500));
        if !force && !due {
            return;
        }
        self.tts.cards_saved = Some(std::time::Instant::now());
        let cards = self.cards_now();
        let mut cfg = self.shared.cfg.borrow_mut();
        if cfg.cards.get(self.book_id).as_ref() == Some(&cards.clamped())
            || (cards.is_empty() && cfg.cards.get(self.book_id).is_none())
        {
            return;
        }
        cfg.cards.set(self.book_id, cards);
        cfg.save();
    }

    /// Brings back the cards the book was left with: where they were, how big, at which place; paused
    /// (never playing), with the page at the spoken sentence or the speed-read word.
    fn restore_cards(&mut self) {
        use crate::reader_cards::{resolve_chapter, resolve_offset, Top};
        let Some(c) = self.shared.cfg.borrow().cards.get(self.book_id) else {
            return;
        };
        let mut target: Option<(usize, i32)> = None;
        if let Some(t) = c.tts.as_ref() {
            if let Some(r) = resolve_chapter(t.chapter, &t.chapter_path, &self.spine) {
                self.w.tts.player.set_pos((t.x, t.y));
                self.w.tts.player.set_collapsed(t.compact);
                let offset = resolve_offset(t.offset, r.exact, i32::MAX);
                self.tts.parked = Some(TtsParked {
                    chapter: r.chapter,
                    offset,
                    sentence: if r.exact { t.sentence } else { 0 },
                    sentences: if r.exact { t.sentences } else { 0 },
                    text: if r.exact {
                        t.text.clone()
                    } else {
                        String::new()
                    },
                });
                if c.top == Top::Tts || c.rsvp.is_none() {
                    target = Some((r.chapter, offset));
                }
            }
        }
        if let Some(k) = c.rsvp.as_ref() {
            if let Some(r) = resolve_chapter(k.chapter, &k.chapter_path, &self.spine) {
                let offset = resolve_offset(k.offset, r.exact, i32::MAX);
                self.rsvp.set_pos((k.x, k.y));
                self.rsvp.park(
                    &self.canvas,
                    super::rsvp::Parked {
                        chapter: r.chapter,
                        offset,
                        word: if r.exact { k.word } else { 0 },
                        words: if r.exact { k.words } else { 0 },
                        text: if r.exact {
                            k.text.clone()
                        } else {
                            String::new()
                        },
                        wpm: k.wpm,
                    },
                );
                if c.top == Top::Rsvp || c.tts.is_none() {
                    target = Some((r.chapter, offset));
                }
            }
        }
        if c.tts.is_some() && c.rsvp.is_some() && c.top == Top::Rsvp {
            super::card_drag::raise(self.w.tts.player.widget.upcast_ref(), 0);
            super::card_drag::raise(self.rsvp.widget.upcast_ref(), 1);
        } else if c.tts.is_some() && c.rsvp.is_some() {
            super::card_drag::raise(self.rsvp.widget.upcast_ref(), 1);
            super::card_drag::raise(self.w.tts.player.widget.upcast_ref(), 0);
        }
        // the page shows the place of the card that is on top
        if let Some((chapter, offset)) = target {
            if chapter != self.spine_idx {
                self.tts.advancing = true;
                self.load_chapter(chapter, Some(0.0), None, false);
                self.tts.advancing = false;
            }
            self.canvas.goto_char(offset);
        }
        self.tts_sync_player();
        // the play button of the top card is where the keyboard waits
        let (tts_player, rsvp) = (self.w.tts.player.clone(), self.rsvp.clone());
        let tts_top = c.top == Top::Tts || c.rsvp.is_none();
        glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
            if tts_top {
                tts_player.focus_play();
            } else {
                rsvp.focus_play();
            }
        });
    }

    /// Previous (-1) or next (1) chapter that has text, and read it from the start.
    fn tts_goto_chapter(&mut self, delta: i32) {
        let mut idx = self.spine_idx as i64;
        loop {
            idx += delta.signum() as i64;
            if idx < 0 || idx as usize >= self.spine.len() {
                return;
            }
            self.tts.advancing = true;
            self.load_chapter(idx as usize, Some(0.0), None, false);
            self.tts.advancing = false;
            if !crate::tts::split_sentences(&self.canvas.canonical_text()).is_empty() {
                self.tts_start(0);
                return;
            }
        }
    }

    /// Remembers where you are before a jump (Alt+Left comes back here).
    fn note_jump(&mut self) {
        self.jumps.record(crate::jump_history::Place::new(
            self.spine_idx,
            self.pos.frac,
        ));
        self.update_jump_buttons();
    }

    /// The same for a stream of jumps such as dragging the slider or stepping through search hits.
    fn note_jump_burst(&mut self) {
        self.jumps.record_burst(
            crate::jump_history::Place::new(self.spine_idx, self.pos.frac),
            glib::monotonic_time() / 1000,
        );
        self.update_jump_buttons();
    }

    fn update_jump_buttons(&self) {
        self.w.back_btn.set_visible(self.jumps.can_back());
        self.w.forward_btn.set_visible(self.jumps.can_forward());
    }

    fn show_place(&mut self, to: crate::jump_history::Place) {
        if to.spine == self.spine_idx {
            self.canvas.goto_fraction(to.frac);
        } else {
            self.load_chapter(to.spine, Some(to.frac), None, false);
        }
    }

    fn goto_book(&self) -> crate::goto::Book<'_> {
        crate::goto::Book {
            layout_pages: self.book_pages().1,
            weights: &self.weights,
            chapters: self.toc.len(),
            page_list: self.page_map.as_ref(),
        }
    }

    /// The fraction of the book at a place in a chapter.
    fn overall_of(&self, spine: usize, frac: f64) -> f64 {
        let total: f64 = self.weights.iter().sum();
        if total <= 0.0 {
            return 0.0;
        }
        let before: f64 = self.weights[..spine.min(self.weights.len())].iter().sum();
        let cur = self.weights.get(spine).copied().unwrap_or(0.0);
        ((before + cur * frac) / total).clamp(0.0, 1.0)
    }

    /// Credits the time since the last position change to the place that was on screen.
    fn dwell_tick(&mut self) {
        let now = std::time::Instant::now();
        if let Some((f, t)) = self.dwell_at {
            crate::reading_dwell::record(self.book_id, f, now.duration_since(t).as_secs_f64());
        }
        self.dwell_at = Some((self.overall_fraction(), now));
    }

    /// Opens the book map: every page of the book, the time spent on them, bookmarks, highlights,
    /// notes and the places you jumped from.
    fn book_map(&mut self) {
        use bookx_lookup::mapview::{
            fraction_of_page, page_of_fraction, MarkStyle, Marker, MarkerKind, Swatch,
        };
        self.dwell_tick();
        crate::reading_dwell::flush();
        let (layout_total, _) = (self.book_pages().1, ());
        let total = match &self.page_map {
            Some(m) if m.total_pages() > 0 => m.total_pages(),
            _ => layout_total.max(1),
        };
        let page_of =
            |spine: usize, frac: f64| page_of_fraction(self.overall_of(spine, frac), total);
        // a chapter's start page; chapters that share a spine file are listed once
        let mut chapters: Vec<(usize, String)> = Vec::new();
        for i in 0..self.spine.len() {
            let start = page_of(i, 0.0);
            let title = self.chapter_title(i);
            if chapters
                .last()
                .is_some_and(|(p, t)| *p == start && *t == title)
            {
                continue;
            }
            chapters.push((start, title));
        }
        let mut marks: Vec<Marker> = Vec::new();
        {
            let lib = self.shared.lib.borrow();
            let Some(b) = lib.get(self.book_id) else {
                return;
            };
            let w = |i: usize| self.weights.get(i).copied().unwrap_or(1.0).max(1.0);
            marks.extend(
                b.bookmarks
                    .iter()
                    .filter(|m| m.spine < self.spine.len())
                    .map(|m| Marker::new(page_of(m.spine, m.frac), MarkerKind::Bookmark)),
            );
            // a highlight knows its character position, not its fraction: this is an estimate;
            // its colour and style are the stored ones, a note adds a pencil in the same colour
            for h in b.highlights.iter().filter(|h| h.spine < self.spine.len()) {
                let page = page_of(
                    h.spine,
                    (h.start.max(0) as f64 / (w(h.spine) * 0.7)).clamp(0.0, 1.0),
                );
                let c = color_of(&h.color);
                let swatch = Swatch {
                    rgb: [
                        (c.0 * 255.0).round() as u8,
                        (c.1 * 255.0).round() as u8,
                        (c.2 * 255.0).round() as u8,
                    ],
                    style: MarkStyle::from_name(&h.style),
                };
                marks.push(Marker::coloured(page, MarkerKind::Highlight, swatch));
                if !h.note.is_empty() {
                    marks.push(Marker::coloured(page, MarkerKind::Note, swatch));
                }
            }
        }
        let history: Vec<usize> = self
            .jumps
            .back_places()
            .iter()
            .map(|p| page_of(p.spine, p.frac))
            .collect();
        let dwell = {
            let b = crate::reading_dwell::buckets(self.book_id);
            bookx_lookup::dwell::per_page(&b, total)
        };
        let current = page_of_fraction(self.overall_fraction(), total);
        // printed page labels when the book has a page-list
        let label: Rc<dyn Fn(usize) -> String> = {
            let (map, weights) = (self.page_map.clone(), self.weights.clone());
            let wsum: f64 = weights.iter().sum();
            Rc::new(move |page| match &map {
                Some(m) => {
                    let f = fraction_of_page(page, total) * wsum;
                    let (mut acc, mut spine, mut local) = (0.0, 0, 0.0);
                    for (i, w) in weights.iter().enumerate() {
                        if acc + w >= f || i + 1 == weights.len() {
                            spine = i;
                            local = ((f - acc) / w.max(1e-9)).clamp(0.0, 1.0);
                            break;
                        }
                        acc += w;
                    }
                    match m.page_at_fraction(spine, local) {
                        Some(r) => format!("Page {}", r.label),
                        None => format!("Page {}", page + 1),
                    }
                }
                None => format!("Page {}", page + 1),
            })
        };
        let preview: Rc<dyn Fn(usize) -> super::reader_map::Preview> = {
            let book = self.book.clone();
            let spine = self.spine.clone();
            let weights = self.weights.clone();
            let titles: Vec<String> = (0..self.spine.len())
                .map(|i| self.chapter_title(i))
                .collect();
            let cache: RefCell<Option<(usize, String)>> = RefCell::new(None);
            Rc::new(move |page| {
                let wsum: f64 = weights.iter().sum();
                let f = fraction_of_page(page, total) * wsum;
                let (mut acc, mut idx, mut local) = (0.0, 0, 0.0);
                for (i, w) in weights.iter().enumerate() {
                    if acc + w >= f || i + 1 == weights.len() {
                        idx = i;
                        local = ((f - acc) / w.max(1e-9)).clamp(0.0, 1.0);
                        break;
                    }
                    acc += w;
                }
                let mut c = cache.borrow_mut();
                if c.as_ref().map(|c| c.0) != Some(idx) {
                    let text = spine
                        .get(idx)
                        .and_then(|p| book.chapter_doc(p))
                        .map(|d| {
                            d.blocks
                                .iter()
                                .map(|b| b.plain_text())
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default();
                    *c = Some((idx, text));
                }
                let text = c.as_ref().map(|c| c.1.as_str()).unwrap_or("");
                super::reader_map::Preview {
                    chapter: titles.get(idx).cloned().unwrap_or_default(),
                    text: bookx_lookup::mapview::snippet(text, local, 150),
                }
            })
        };
        // the notes (their text shows in the card when the pointer is on the pencil)
        let notes: Vec<(usize, Swatch, String)> = {
            let lib = self.shared.lib.borrow();
            lib.get(self.book_id)
                .map(|b| {
                    b.highlights
                        .iter()
                        .filter(|h| h.spine < self.spine.len() && !h.note.is_empty())
                        .map(|h| {
                            let c = color_of(&h.color);
                            let sw = Swatch {
                                rgb: [
                                    (c.0 * 255.0).round() as u8,
                                    (c.1 * 255.0).round() as u8,
                                    (c.2 * 255.0).round() as u8,
                                ],
                                style: MarkStyle::from_name(&h.style),
                            };
                            let page = page_of(
                                h.spine,
                                (h.start.max(0) as f64
                                    / (self.weights.get(h.spine).copied().unwrap_or(1.0).max(1.0)
                                        * 0.7))
                                    .clamp(0.0, 1.0),
                            );
                            (page, sw, h.note.clone())
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        // thumbnails: a small canvas of the reader's size and style lays out the hovered chapter
        let thumbs = {
            let (styled, user_css, lang) = {
                let cfg = self.shared.cfg.borrow();
                let r = &cfg.reader;
                let tweaks = cfg.tweaks.effective(self.book_id);
                (
                    r.publisher_styles || r.colors == ColorMode::Original,
                    tweaks.css(),
                    tweaks.hyphenation_language(&self.book.metadata().language),
                )
            };
            let (book, spine) = (self.book.clone(), self.spine.clone());
            let doc_for: Rc<dyn Fn(usize) -> Option<crate::core::doc::Doc>> = {
                let (book, spine) = (book.clone(), spine.clone());
                Rc::new(move |i| {
                    spine
                        .get(i)
                        .and_then(|p| book.chapter_doc_layered(p, styled, &user_css))
                })
            };
            let highlights_for: Rc<dyn Fn(usize) -> Vec<CHighlight>> = {
                let (shared, id) = (self.shared.clone(), self.book_id);
                Rc::new(move |i| {
                    shared
                        .lib
                        .borrow()
                        .get(id)
                        .map(|b| {
                            b.highlights
                                .iter()
                                .filter(|h| h.spine == i && !crate::pdf::is_pdf(h))
                                .map(|h| CHighlight {
                                    id: h.id.clone(),
                                    start: h.start,
                                    end: h.end,
                                    color: color_of(&h.color),
                                    style: HighlightStyle::from_name(&h.style).unwrap_or_default(),
                                    note: h.note.clone(),
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                })
            };
            let images: Rc<dyn Fn(&str) -> Option<Vec<u8>>> =
                Rc::new(move |p| book.archive.get(p).map(|d| d.to_vec()));
            Some(super::map_thumbs::Thumbs::new(
                super::map_thumbs::Source {
                    weights: self.weights.clone(),
                    total,
                    doc_for,
                    highlights_for,
                    images,
                    style: self.canvas.style(),
                    view: self.canvas.view_size(),
                    lang,
                    vertical: self.canvas.writing_mode().is_vertical(),
                    disk: {
                        // the book file and its date: a changed book is a new folder
                        let lib = self.shared.lib.borrow();
                        lib.get(self.book_id).and_then(|b| {
                            let path = lib.book_path(b);
                            let meta = std::fs::metadata(&path).ok()?;
                            let mtime = meta
                                .modified()
                                .ok()
                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                .map_or(0, |d| d.as_secs());
                            let root = directories::ProjectDirs::from("com", "adhadse", "Bookx")?
                                .cache_dir()
                                .join("map-thumbs");
                            Some((root, format!("{} {} {mtime}", path.display(), meta.len())))
                        })
                    },
                    hl_sig: {
                        // highlights, the user's style sheet and whether the book's colours are shown
                        let cfg = self.shared.cfg.borrow();
                        let extra = format!(
                            "{}|{:?}|{}",
                            cfg.tweaks.effective(self.book_id).css(),
                            cfg.reader.colors,
                            cfg.reader.publisher_styles
                        );
                        let lib = self.shared.lib.borrow();
                        let hl: String = lib
                            .get(self.book_id)
                            .map(|b| {
                                b.highlights
                                    .iter()
                                    .map(|h| {
                                        format!(
                                            "{}:{}:{}:{}:{}",
                                            h.spine, h.start, h.end, h.color, h.style
                                        )
                                    })
                                    .collect::<Vec<_>>()
                                    .join(";")
                            })
                            .unwrap_or_default();
                        format!("{extra}|{hl}")
                    },
                },
                self.w.page.scale_factor() as f64,
            ))
        };
        let tx = self.sender.input_sender().clone();
        super::reader_map::show(
            self.w.slider.upcast_ref(),
            super::reader_map::MapData {
                title: self
                    .shared
                    .lib
                    .borrow()
                    .get(self.book_id)
                    .map(|b| b.meta.title.clone())
                    .unwrap_or_default(),
                total,
                current,
                chapters,
                marks,
                history,
                dwell,
                label,
                preview,
                jump: Rc::new(move |page| tx.emit(Input::MapJump(fraction_of_page(page, total)))),
                thumbs,
                notes,
                reader_rect: self
                    .w
                    .page
                    .root()
                    .and_then(|r| {
                        self.canvas
                            .area
                            .compute_bounds(r.upcast_ref::<gtk::Widget>())
                    })
                    .map(|r| {
                        (
                            r.x() as f64,
                            r.y() as f64,
                            r.width() as f64,
                            r.height() as f64,
                        )
                    })
                    .unwrap_or((0.0, 0.0, 0.0, 0.0)),
                reader_view: {
                    let (c, n) = self.canvas.current_index();
                    (self.spine_idx, c, n)
                },
                reader_sig: {
                    // the reader's own layout of the chapter on screen
                    let (spine, sig) = (self.spine_idx, self.canvas.layout_signature());
                    Rc::new(move || (spine, sig))
                },
            },
        );
    }

    /// What the theme editor needs.
    fn profile_ctx(&self) -> super::reader_theme_editor::Ctx {
        let (t1, t2) = (
            self.sender.input_sender().clone(),
            self.sender.output_sender().clone(),
        );
        let t3 = self.sender.input_sender().clone();
        super::reader_theme_editor::Ctx {
            shared: self.shared.clone(),
            book_id: self.book_id,
            apply: Rc::new(move |p| t1.emit(Input::Prefs(p))),
            toast: Rc::new(move |m| t2.emit(Output::Toast(m))),
            changed: Rc::new(move || t3.emit(Input::ThemesChanged)),
        }
    }

    /// Opens a picture of the book in the image viewer.
    fn open_image(&self, src: &str) {
        let Some(bytes) = self.book.archive.get(src).map(|d| d.to_vec()) else {
            return;
        };
        let title = self.chapter_title(self.spine_idx);
        if !super::image_viewer::show(self.w.page.upcast_ref(), &title, bytes, src) {
            let _ = self.sender.output(Output::Toast(tr(
                "This picture cannot be shown in the viewer",
            )));
        }
    }

    fn goto_dialog(&self) {
        let hint = crate::goto::hint(&self.goto_book());
        let (tx, tx2) = (
            self.sender.input_sender().clone(),
            self.sender.input_sender().clone(),
        );
        // the checks run in the dialog's own closure, on a copy of what they need
        let weights = self.weights.clone();
        let chapters: Vec<String> = self.toc.iter().map(|t| t.title.clone()).collect();
        let layout_pages = self.book_pages().1;
        let page_map = self.page_map.clone();
        let spine_titles: Vec<String> = (0..self.spine.len())
            .map(|i| self.chapter_title(i))
            .collect();
        let check = Rc::new(move |text: &str| {
            let book = crate::goto::Book {
                layout_pages,
                weights: &weights,
                chapters: chapters.len(),
                page_list: page_map.as_ref(),
            };
            let dest = crate::goto::resolve(&crate::goto::parse(text)?, &book)?;
            Ok(match dest {
                crate::goto::Dest::Fraction(f) => format!(
                    "{:.0}% of the book · location {}",
                    f * 100.0,
                    crate::goto::location_at(&book, f)
                ),
                crate::goto::Dest::Spine(i, _) => {
                    format!("In “{}”", spine_titles.get(i).map_or("", String::as_str))
                }
                crate::goto::Dest::Chapter(n) => trf(
                    "Chapter: {}",
                    &[&(chapters.get(n).map_or("", String::as_str))],
                ),
            })
        });
        let _ = tx2;
        super::reader_goto::show(
            self.w.page.upcast_ref(),
            &hint,
            check,
            Rc::new(move |t| tx.emit(Input::GotoText(t))),
        );
    }

    /// Jumps to what was typed in Go to…; the reason when it does not lead anywhere.
    fn goto_text(&mut self, text: &str) -> Result<(), String> {
        let dest =
            crate::goto::parse(text).and_then(|t| crate::goto::resolve(&t, &self.goto_book()));
        match dest {
            Ok(d) => {
                self.note_jump();
                match d {
                    crate::goto::Dest::Fraction(f) => self.goto_overall(f),
                    crate::goto::Dest::Spine(i, f) => {
                        // a printed page: go to its anchor, which is exact
                        let label = text
                            .trim()
                            .trim_start_matches("page ")
                            .trim_start_matches("p. ");
                        let mark = self
                            .page_marks
                            .iter()
                            .find(|m| m.spine == i && m.label == label)
                            .cloned();
                        match mark {
                            Some(m) => self.load_chapter(i, Some(0.0), m.fragment, false),
                            None => self.load_chapter(i, Some(f), None, false),
                        }
                    }
                    crate::goto::Dest::Chapter(n) => {
                        if let Some(h) = self.toc.get(n).map(|t| t.href.clone()) {
                            if let Some((i, frag)) = self.book.locate(&h) {
                                self.load_chapter(i, Some(0.0), frag, false);
                            }
                        }
                    }
                }
                Ok(())
            }
            Err(e) => {
                let _ = self.sender.output(Output::Toast(e.clone()));
                Err(e)
            }
        }
    }

    fn goto_overall(&mut self, f: f64) {
        let total: f64 = self.weights.iter().sum();
        let target = f.clamp(0.0, 1.0) * total;
        let mut acc = 0.0;
        for (i, w) in self.weights.iter().enumerate() {
            if acc + w >= target || i + 1 == self.weights.len() {
                let frac = ((target - acc) / w).clamp(0.0, 1.0);
                self.load_chapter(i, Some(frac), None, false);
                return;
            }
            acc += w;
        }
    }
}

/// (page in book, pages in book). Chapters without a measured page count are estimated from
/// the page density (pages per byte) of the chapters that were laid out.
pub fn estimate_book_pages(
    known: &HashMap<usize, usize>,
    weights: &[f64],
    idx: usize,
    page: usize,
    pages: usize,
) -> (usize, usize) {
    let (mut kp, mut kw) = (0.0, 0.0);
    for (i, p) in known.iter() {
        kp += *p as f64;
        kw += weights.get(*i).copied().unwrap_or(0.0);
    }
    let density = if kw > 0.0 { kp / kw } else { 1.0 / 2000.0 };
    let pages_of = |i: usize| {
        known
            .get(&i)
            .map(|p| *p as f64)
            .unwrap_or_else(|| density * weights.get(i).copied().unwrap_or(0.0))
    };
    let before = (0..idx).map(pages_of).sum::<f64>().round() as usize;
    let total = (0..weights.len()).map(pages_of).sum::<f64>().round() as usize;
    (before + page, total.max(before + pages))
}

/// GTK does not hand a widget's reading direction down to its children: set it on the whole tree.
fn set_direction_deep(w: &gtk::Widget, dir: gtk::TextDirection) {
    w.set_direction(dir);
    let mut c = w.first_child();
    while let Some(child) = c {
        set_direction_deep(&child, dir);
        c = child.next_sibling();
    }
}

/// What the reader knows about the user's interaction with the header and the navigation row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChromeUse {
    /// A menu or popover of the bars is open (search, menu, appearance, selection, read aloud).
    pub popover_open: bool,
    /// Pointer height in the page, negative when it is outside.
    pub pointer_y: f64,
    pub height: f64,
    /// The pointer is over one of the bars.
    pub hovering: bool,
    /// The keyboard focus (shown focus ring) is on a control of the bars.
    pub keyboard_focus: bool,
    /// A button is held down while moving, for instance the page slider.
    pub dragging: bool,
}

impl ChromeUse {
    #[cfg(test)]
    pub fn idle(height: f64) -> Self {
        ChromeUse {
            popover_open: false,
            pointer_y: -1.0,
            height,
            hovering: false,
            keyboard_focus: false,
            dragging: false,
        }
    }
}

/// The header and navigation row must not fade while they are being used: a popover is open (this
/// includes the read-aloud controls), the pointer rests on a bar, the keyboard focus is on one of
/// their controls or a slider is being dragged. The idle countdown starts when this turns false.
pub fn chrome_busy(u: ChromeUse) -> bool {
    u.popover_open
        || u.hovering
        || u.keyboard_focus
        || u.dragging
        || (u.pointer_y >= 0.0 && (u.pointer_y < 64.0 || u.pointer_y > u.height - 96.0))
}

/// What the "Reader focus" (the bars fading away while reading) needs between two looks; the decision
/// itself is `crate::chrome_visibility`.
pub struct ChromeCtl {
    /// Read aloud or speed reading is playing: the short idle delay applies.
    pub playing: Cell<bool>,
    /// Pointer height in the window, negative when it is outside.
    pub pointer_y: Cell<f64>,
    last: Cell<crate::chrome_visibility::Interaction>,
    start: std::time::Instant,
    /// Time added to the clock by the dev hook, so tests can let the idle delay pass.
    skew_ms: Cell<u64>,
}

impl ChromeCtl {
    pub fn new() -> ChromeCtl {
        ChromeCtl {
            playing: Cell::new(false),
            pointer_y: Cell::new(-1.0),
            last: Cell::default(),
            start: std::time::Instant::now(),
            skew_ms: Cell::new(0),
        }
    }

    pub fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64 + self.skew_ms.get()
    }

    /// The user did something (pointer moved or pressed, key pressed) or the bars are in use.
    pub fn touch(&self) {
        let mut l = self.last.get();
        l.touch(self.now_ms());
        self.last.set(l);
    }

    pub fn idle_ms(&self) -> u64 {
        self.last.get().idle_ms(self.now_ms())
    }

    pub fn advance(&self, ms: u64) {
        self.skew_ms.set(self.skew_ms.get() + ms);
    }
}

/// Closes a popover, runs `change`, and opens it again a moment later. The pause matters: GTK4 on
/// Wayland re-uses the popover's surface, and a hide followed by a show in the same breath made
/// the surface get its buffer before it had its popup role again, so Mutter killed the app with
/// "wl_surface already has a buffer committed" (protocol error 71; found by the real-session tests).
/// `switching` keeps the `closed` signal from being taken for the user dismissing the popover.
fn repopup(pop: &gtk::Popover, switching: &Rc<Cell<bool>>, change: impl FnOnce()) {
    repopup_then(pop, switching, change, || {});
}

/// `repopup`, then `after` once the popover is open again (to focus a widget in it).
fn repopup_then(
    pop: &gtk::Popover,
    switching: &Rc<Cell<bool>>,
    change: impl FnOnce(),
    after: impl FnOnce() + 'static,
) {
    switching.set(true);
    pop.popdown();
    change();
    let (pop, sw) = (pop.clone(), switching.clone());
    glib::timeout_add_local_once(
        std::time::Duration::from_millis(REPOPUP_PAUSE_MS),
        move || {
            pop.popup();
            sw.set(false);
            after();
        },
    );
}

/// How long a popover stays closed while it is presented again (see `repopup`).
const REPOPUP_PAUSE_MS: u64 = 80;

/// Pointer and touch events that stop at the selection popover instead of bubbling on to the page
/// canvas (where they would count as a click on the page and clear the selection).
fn popover_event_stops_here(t: gdk::EventType) -> bool {
    use gdk::EventType::*;
    matches!(
        t,
        ButtonPress
            | ButtonRelease
            | TouchBegin
            | TouchUpdate
            | TouchEnd
            | TouchCancel
            | Scroll
            | GrabBroken
    )
}

fn icon_button(icon: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(tip));
    b
}

impl SimpleComponent for Reader {
    type Init = (SharedRef, u64, OpenBook);
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
        (shared, book_id, book): Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let fixed = book.is_fixed_layout();
        let book = Rc::new(book);
        let title_text = book.metadata().title.clone();
        let spine = book.spine_paths();
        let weights = book.spine_weights();
        let toc = flatten(&book.toc());

        // ---- canvas ---------------------------------------------------------
        let canvas = BookCanvas::new();
        let rtl = book.is_rtl();
        canvas.set_rtl(rtl);
        canvas.area.update_property(&[
            gtk::accessible::Property::Label(&trf("{}, page text", &[&title_text])),
            gtk::accessible::Property::Description(
                "Right and Left turn the page, Shift with the arrows selects text",
            ),
        ]);
        {
            let b = book.clone();
            canvas.set_image_provider(move |p| b.archive.get(p).map(|d| d.to_vec()));
            let tx = sender.input_sender().clone();
            canvas.on_link(move |l| tx.emit(Input::Link(l)));
            let tx = sender.input_sender().clone();
            canvas.on_image(move |src| tx.emit(Input::OpenImage(src)));
            super::reader_extra::track_clicks(&canvas.area);
            let tx = sender.input_sender().clone();
            canvas.on_position(move |p| tx.emit(Input::Position(p)));
            let tx = sender.input_sender().clone();
            canvas.on_edge(move |e| tx.emit(Input::Edge(e)));
            let tx = sender.input_sender().clone();
            canvas.on_selection(move |s| tx.emit(Input::Selection(s)));
            let tx = sender.input_sender().clone();
            canvas.on_highlight_click(move |s| tx.emit(Input::HighlightClick(s)));
            let tx = sender.input_sender().clone();
            canvas.on_tap(move || tx.emit(Input::Tap));
            let tx = sender.input_sender().clone();
            canvas.on_double_tap(move || tx.emit(Input::DoubleTap));
            let tx = sender.input_sender().clone();
            canvas.on_zoom(move |f| tx.emit(Input::Zoom(f)));
        }

        // ---- selection popover ----------------------------------------------
        let sel_pop = gtk::Popover::new();
        sel_pop.set_autohide(false);
        sel_pop.set_has_arrow(true);
        sel_pop.set_parent(&canvas.area);
        super::card_drag::avoid(&sel_pop);
        sel_pop.add_css_class("selection-popover");
        let switching = Rc::new(Cell::new(false));
        let sel_stack = gtk::Stack::new();
        sel_stack.set_hhomogeneous(false);
        sel_stack.set_vhomogeneous(false);
        sel_stack.set_interpolate_size(false);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let mut swatches: Vec<gtk::Button> = Vec::new();
        for (name, _) in HIGHLIGHT_COLORS {
            let b = gtk::Button::new();
            b.add_css_class("hl-swatch");
            b.set_size_request(26, 26);
            b.set_halign(gtk::Align::Center);
            b.set_valign(gtk::Align::Center);
            b.set_hexpand(false);
            b.set_vexpand(false);
            b.add_css_class(&format!("hl-{name}"));
            b.set_tooltip_text(Some(&trf("Highlight {}", &[&name])));
            let tx = sender.input_sender().clone();
            b.connect_clicked(move |_| tx.emit(Input::Highlight(name.to_string())));
            actions.append(&b);
            swatches.push(b);
        }
        actions.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        let note_btn = icon_button("document-edit-symbolic", &tr("Add a note"));
        let copy_btn = icon_button("edit-copy-symbolic", &tr("Copy"));
        let dict_btn = icon_button(
            "accessories-dictionary-symbolic",
            &tr("Look up in a dictionary"),
        );
        let wiki_btn = icon_button("wikipedia-symbolic", &tr("Look up on Wikipedia"));
        let tr_btn = icon_button("translate-symbolic", &tr("Translate"));
        let sel_remove = icon_button("user-trash-symbolic", &tr("Remove highlight"));
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
        // The note of an existing highlight is shown right in the popover, because touch
        // screens have no hover. Without a note the popover is just the buttons.
        let note_preview = gtk::Button::new();
        note_preview.add_css_class("flat");
        note_preview.add_css_class("note-preview");
        note_preview.set_visible(false);
        note_preview.set_tooltip_text(Some(&tr("Edit note")));
        let note_label = gtk::Label::new(None);
        note_label.set_wrap(true);
        note_label.set_max_width_chars(42);
        note_label.set_xalign(0.0);
        note_preview.set_child(Some(&note_label));
        {
            let b = note_btn.clone();
            note_preview.connect_clicked(move |_| b.emit_clicked());
        }
        // how the highlight is drawn: background, underline, strikethrough or squiggly
        let style_row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        style_row.set_halign(gtk::Align::Start);
        let mut style_btns: Vec<(HighlightStyle, gtk::ToggleButton)> = Vec::new();
        for (st, markup, tip) in [
            (
                HighlightStyle::Background,
                "<span background=\"#f6d32d\" foreground=\"#241f31\"> A </span>",
                "Highlight with a background",
            ),
            (HighlightStyle::Underline, "<u>A</u>", "Underline"),
            (HighlightStyle::Strikethrough, "<s>A</s>", "Strike through"),
            (
                HighlightStyle::Squiggly,
                "<span underline=\"error\">A</span>",
                "Squiggly underline",
            ),
        ] {
            let b = gtk::ToggleButton::new();
            let l = gtk::Label::new(None);
            l.set_markup(markup);
            b.set_child(Some(&l));
            b.add_css_class("flat");
            b.add_css_class("hl-style");
            b.set_tooltip_text(Some(tip));
            b.set_active(st == HighlightStyle::Background);
            let tx = sender.input_sender().clone();
            b.connect_clicked(move |_| tx.emit(Input::HlStyle(st)));
            style_row.append(&b);
            style_btns.push((st, b));
        }
        let actions_page = gtk::Box::new(gtk::Orientation::Vertical, 4);
        actions_page.append(&note_preview);
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
        let note_back = gtk::Button::from_icon_name("go-previous-symbolic");
        note_back.add_css_class("flat");
        note_back.set_tooltip_text(Some(&tr("Back to the actions")));
        let note_save = gtk::Button::with_label(&tr("Save Note"));
        note_save.add_css_class("suggested-action");
        note_save.set_hexpand(true);
        note_actions.append(&note_back);
        note_actions.append(&note_save);
        note_box.append(&note_scroll);
        note_box.append(&note_actions);
        sel_stack.add_named(&note_box, Some("note"));
        {
            // The note box needs the keyboard. A non-modal (autohide = false) popover gets no
            // keyboard focus on Wayland, so the popover is re-opened modal for the note page.
            let (s, pop, nv, sw) = (
                sel_stack.clone(),
                sel_pop.clone(),
                note_view.clone(),
                switching.clone(),
            );
            note_btn.connect_clicked(move |_| {
                let (s, nv, p) = (s.clone(), nv.clone(), pop.clone());
                // the note box needs a modal popover for the keyboard: set while it is closed
                repopup_then(
                    &pop,
                    &sw,
                    move || {
                        p.set_autohide(true);
                        s.set_visible_child_name("note");
                    },
                    move || {
                        nv.grab_focus();
                    },
                );
            });
            let (s, pop, sw) = (sel_stack.clone(), sel_pop.clone(), switching.clone());
            note_back.connect_clicked(move |_| {
                let p = pop.clone();
                repopup(&pop, &sw, || {
                    p.set_autohide(false);
                    s.set_visible_child_name("actions");
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
            // keyboard only: Tab leaves the box for the buttons, Ctrl+Enter saves
            note_view.set_accepts_tab(false);
            note_save.set_tooltip_text(Some(&tr("Save the note (Ctrl+Enter)")));
            let keys = gtk::EventControllerKey::new();
            let save = note_save.clone();
            keys.connect_key_pressed(move |_, key, _, mods| {
                let enter = matches!(key, gdk::Key::Return | gdk::Key::KP_Enter);
                if enter && mods.contains(gdk::ModifierType::CONTROL_MASK) {
                    save.emit_clicked();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            note_view.add_controller(keys);
        }
        // Wikipedia and translate pages (look-ups run through GIO, see web.rs)
        let lookup_page = |title: &str, height: i32| {
            let page = gtk::Box::new(gtk::Orientation::Vertical, 8);
            page.set_size_request(340, -1);
            let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let back = gtk::Button::from_icon_name("go-previous-symbolic");
            back.add_css_class("flat");
            back.set_tooltip_text(Some(&tr("Back to the actions")));
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
            // always there, only invisible: hiding it would shrink the popover after it was shown
            spin.set_opacity(0.0);
            spin.set_size_request(-1, 20);
            let text = gtk::Label::new(None);
            text.set_wrap(true);
            text.set_selectable(false);
            text.set_xalign(0.0);
            text.set_max_width_chars(44);
            text.set_margin_start(8);
            text.set_margin_end(8);
            // The text sits in a scroll area of a fixed height: the popover keeps its size when
            // the answer arrives (a popover that grows after it is shown can vanish).
            let scroll = gtk::ScrolledWindow::new();
            scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
            scroll.set_propagate_natural_height(false);
            scroll.set_min_content_height(height);
            scroll.set_max_content_height(height);
            scroll.set_child(Some(&text));
            page.append(&spin);
            page.append(&scroll);
            (page, head, spin, text)
        };
        let (dict_page, _dict_head, _dict_spin, dict_text) = lookup_page("Dictionary", 210);
        dict_text.set_use_markup(true);
        // "No dictionary for German yet": the offer to download one, above the text, inside the scroll
        // area of a fixed height (the popover never changes size when the state changes)
        let dict_prompt = {
            let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
            root.set_margin_start(8);
            root.set_margin_end(8);
            root.set_margin_bottom(8);
            root.set_visible(false);
            let title = gtk::Label::new(None);
            title.add_css_class("heading");
            title.set_xalign(0.0);
            title.set_wrap(true);
            let body = gtk::Label::new(None);
            body.add_css_class("dim-label");
            body.set_xalign(0.0);
            body.set_wrap(true);
            body.set_max_width_chars(44);
            let bar = gtk::ProgressBar::new();
            super::audit::label(&bar, &tr("Dictionary download"));
            bar.set_visible(false);
            let primary = gtk::Button::with_label(&tr("Download dictionary"));
            primary.add_css_class("suggested-action");
            primary.add_css_class("pill");
            primary.set_halign(gtk::Align::Start);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let secondary = gtk::Button::with_label(&tr("Choose another"));
            secondary.add_css_class("flat");
            let learn = gtk::Button::with_label(&tr("Learn more"));
            learn.add_css_class("flat");
            row.append(&secondary);
            row.append(&learn);
            root.append(&title);
            root.append(&body);
            root.append(&bar);
            root.append(&primary);
            root.append(&row);
            if let Some(scroll) = dict_page
                .last_child()
                .and_then(|w| w.downcast::<gtk::ScrolledWindow>().ok())
            {
                scroll.set_child(None::<&gtk::Widget>);
                let body_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
                body_box.append(&root);
                body_box.append(&dict_text);
                scroll.set_child(Some(&body_box));
            }
            let tx = sender.input_sender().clone();
            primary.connect_clicked(move |_| tx.emit(Input::DictDownload));
            let tx = sender.input_sender().clone();
            secondary.connect_clicked(move |_| tx.emit(Input::DictChoose));
            let tx = sender.input_sender().clone();
            learn.connect_clicked(move |_| tx.emit(Input::DictLearn));
            DictPromptUi {
                root,
                title,
                body,
                bar,
                primary,
            }
        };
        {
            let foot = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            foot.set_halign(gtk::Align::End);
            let open = gtk::Button::with_label(&tr("Open dictionary folder"));
            open.add_css_class("flat");
            open.add_css_class("caption");
            open.connect_clicked(|_| {
                let dir = crate::dictionaries::dictionary_dir();
                let _ = std::fs::create_dir_all(&dir);
                open_uri(&format!("file://{}", dir.display()));
            });
            foot.append(&open);
            dict_page.append(&foot);
        }
        sel_stack.add_named(&dict_page, Some("dict"));
        let (wiki_page, _wiki_head, wiki_spin, wiki_text) = lookup_page("Wikipedia", 250);
        let wiki_url = Rc::new(RefCell::new(String::new()));
        // the lead picture of the article above its text: a frame of a fixed size (a placeholder
        // until the picture arrives), inside the scroll area, so the popover never changes size
        let wiki_img = {
            let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
            frame.set_halign(gtk::Align::Center);
            frame.set_margin_bottom(8);
            frame.set_visible(false);
            frame.add_css_class("card");
            frame.set_overflow(gtk::Overflow::Hidden);
            let stack = gtk::Stack::new();
            let ph = gtk::Image::from_icon_name("image-x-generic-symbolic");
            ph.set_pixel_size(32);
            ph.add_css_class("dim-label");
            ph.set_size_request(240, 120);
            stack.set_vhomogeneous(false);
            stack.add_named(&ph, Some("placeholder"));
            let pic = gtk::Picture::new();
            pic.set_content_fit(gtk::ContentFit::Contain);
            pic.set_can_shrink(true);
            pic.set_size_request(240, 120);
            pic.set_vexpand(true);
            super::audit::label(&pic, &tr("Picture from Wikipedia"));
            pic.set_accessible_role(gtk::AccessibleRole::Img);
            stack.add_named(&pic, Some("picture"));
            frame.append(&stack);
            if let Some(scroll) = wiki_page
                .last_child()
                .and_then(|w| w.downcast::<gtk::ScrolledWindow>().ok())
            {
                scroll.set_child(None::<&gtk::Widget>);
                let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
                body.append(&frame);
                body.append(&wiki_text);
                scroll.set_child(Some(&body));
            }
            (frame, stack, pic)
        };
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
        let (tr_page, tr_head, tr_spin, tr_text) = lookup_page("Translate", 170);
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
            let foot = gtk::Label::new(Some(&tr("Google Translate")));
            foot.add_css_class("dim-label");
            foot.add_css_class("caption");
            foot.set_halign(gtk::Align::End);
            tr_page.append(&foot);
            dd
        };
        sel_stack.add_named(&tr_page, Some("translate"));
        // Events inside the popover bubble up to its parent, the page canvas. Without this a
        // press on the popover's padding or text reaches the canvas as a click, which clears
        // the selection and closes the popover under the user's finger. The events are stopped
        // here, after the buttons inside have had them. (A `GestureClick` that *claims* the
        // press does the same but also cancels the buttons' own gesture on a real compositor, so
        // no popover button ever fired its `clicked`; Broadway did not show that.)
        let shield = gtk::EventControllerLegacy::new();
        shield.set_propagation_phase(gtk::PropagationPhase::Bubble);
        shield.connect_event(|_, e| {
            if popover_event_stops_here(e.event_type()) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        sel_pop.add_controller(shield);
        sel_pop.set_child(Some(&sel_stack));

        // ---- sidebar ----------------------------------------------------------
        let side = {
            let (author, cover) = {
                let lib = shared.lib.borrow();
                match lib.get(book_id) {
                    Some(b) => (b.meta.authors_display(), lib.cover_path(b)),
                    None => (String::new(), None),
                }
            };
            let tx = sender.input_sender().clone();
            let side = super::reader_side::Side::build(
                &super::reader_side::Head {
                    title: title_text.clone(),
                    author,
                    cover,
                },
                Rc::new(move |c| tx.emit(Input::Side(c))),
            );
            let items: Vec<super::reader_side::TocItem> = toc
                .iter()
                .map(|t| super::reader_side::TocItem {
                    title: t.title.clone(),
                    depth: t.depth,
                    href: t.href.clone(),
                })
                .collect();
            side.set_toc(&items);
            side
        };
        let stack = side.stack.clone();

        // Search lives in its own dropdown (header button). The query and the results are
        // kept when it closes, so reopening it shows the previous search.
        let search_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        search_box.set_size_request(380, 460);
        let search_entry = gtk::SearchEntry::new();
        search_entry.set_placeholder_text(Some(&tr("Search in book")));
        search_entry.set_margin_all(8);
        let search_status = gtk::Label::new(None);
        search_status.add_css_class("dim-label");
        search_status.add_css_class("caption");
        let search_list = gtk::ListBox::new();
        search_list.add_css_class("navigation-sidebar");
        let search_scroll = gtk::ScrolledWindow::new();
        search_scroll.set_vexpand(true);
        search_scroll.set_child(Some(&search_list));
        search_box.append(&search_entry);
        search_box.append(&search_status);
        search_box.append(&search_scroll);
        let search_pop = gtk::Popover::new();
        search_pop.set_child(Some(&search_box));
        let search_btn = gtk::MenuButton::new();
        search_btn.set_icon_name("system-search-symbolic");
        search_btn.set_tooltip_text(Some(&tr("Search in book (Ctrl+F)")));
        search_btn.set_popover(Some(&search_pop));
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
            let b = search_btn.clone();
            search_list.connect_row_activated(move |_, row| {
                tx.emit(Input::SearchResult(row.index() as usize));
                b.popdown();
            });
            let tx = sender.input_sender().clone();
            search_entry.connect_next_match(move |_| tx.emit(Input::SearchStep(true)));
            let tx = sender.input_sender().clone();
            search_entry.connect_previous_match(move |_| tx.emit(Input::SearchStep(false)));
            let tx = sender.input_sender().clone();
            search_entry.connect_activate(move |_| tx.emit(Input::SearchStep(true)));
        }

        // ---- content ----------------------------------------------------------
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        // the page, with the selection handles on a layer over it
        let page_overlay = gtk::Overlay::new();
        page_overlay.set_hexpand(true);
        page_overlay.set_vexpand(true);
        page_overlay.set_child(Some(&canvas.area));
        let handles = {
            let (pop, sw) = (sel_pop.clone(), switching.clone());
            let (tx, sw2) = (sender.input_sender().clone(), switching.clone());
            super::sel_handles::Handles::build(
                &page_overlay,
                &canvas,
                move || {
                    // the action popover steps aside while a handle is in hand
                    sw.set(true);
                    pop.popdown();
                },
                move || {
                    // and comes back, after a pause (a popover closed and opened at once can kill the app)
                    let (tx, sw2) = (tx.clone(), sw2.clone());
                    glib::timeout_add_local_once(
                        std::time::Duration::from_millis(REPOPUP_PAUSE_MS + 20),
                        move || {
                            sw2.set(false);
                            tx.emit(Input::HandlesDone);
                        },
                    );
                },
            )
        };
        content.append(&page_overlay);
        super::reading_ruler::set_current(super::reading_ruler::Ruler::install(
            &page_overlay,
            &canvas,
            &shared,
        ));

        let slider = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1000.0, 1.0);
        super::audit::label(&slider, &tr("Position in the book"));
        slider.set_draw_value(false);
        slider.set_hexpand(true);
        slider.add_css_class("reader-slider");
        let prev_btn = icon_button(
            "go-previous-symbolic",
            &(if rtl {
                tr("Previous page (→)")
            } else {
                tr("Previous page (←)")
            }),
        );
        let next_btn = icon_button(
            "go-next-symbolic",
            &(if rtl {
                tr("Next page (←)")
            } else {
                tr("Next page (→)")
            }),
        );
        for b in [&prev_btn, &next_btn] {
            b.add_css_class("flat");
            b.add_css_class("circular");
        }
        let slider_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        slider_row.add_css_class("reader-nav");
        slider_row.append(&prev_btn);
        slider_row.append(&slider);
        slider_row.append(&next_btn);
        let left_label = gtk::Label::new(None);
        let mid_label = gtk::Label::new(None);
        let right_label = gtk::Label::new(None);
        for l in [&left_label, &mid_label, &right_label] {
            l.add_css_class("caption");
            l.add_css_class("dim-label");
        }
        mid_label.set_ellipsize(pango::EllipsizeMode::End);
        mid_label.set_hexpand(true);
        mid_label.set_max_width_chars(120);
        left_label.set_xalign(0.0);
        right_label.set_xalign(1.0);
        let label_row = gtk::CenterBox::new();
        label_row.set_start_widget(Some(&left_label));
        label_row.set_center_widget(Some(&mid_label));
        label_row.set_end_widget(Some(&right_label));
        let bottom = gtk::Box::new(gtk::Orientation::Vertical, 0);
        bottom.add_css_class("reader-bottom");
        bottom.set_margin_start(16);
        bottom.set_margin_end(16);
        bottom.set_margin_bottom(6);
        bottom.append(&slider_row);
        bottom.append(&label_row);
        if rtl {
            // a right-to-left book is read from the right: the footer, its arrows and the
            // progress bar run the other way round and the contents open on the right
            set_direction_deep(bottom.upcast_ref(), gtk::TextDirection::Rtl);
        }

        // right click on the footer (or the menu entry) configures the statistics it shows
        let footer_pop = {
            let pop = gtk::Popover::new();
            pop.set_parent(&bottom);
            pop.set_position(gtk::PositionType::Top);
            let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
            col.set_margin_all(10);
            let title = gtk::Label::new(Some(&tr("Footer Stats")));
            title.add_css_class("heading");
            title.set_xalign(0.0);
            col.append(&title);
            let tx = sender.input_sender().clone();
            let editor = super::reader_footer::editor(
                shared.cfg.borrow().footer_for(book_id),
                Rc::new(move |c| tx.emit(Input::Footer(c))),
            );
            let scroll = gtk::ScrolledWindow::new();
            scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
            scroll.set_min_content_height(380);
            scroll.set_max_content_height(380);
            scroll.set_min_content_width(380);
            scroll.set_propagate_natural_width(true);
            scroll.set_child(Some(&editor));
            col.append(&scroll);
            let own_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let own_label = gtk::Label::new(Some(&tr("Only for this book")));
            own_label.set_xalign(0.0);
            own_label.set_hexpand(true);
            let own = gtk::Switch::new();
            own.set_valign(gtk::Align::Center);
            super::audit::label(&own, &tr("Only for this book"));
            own.set_active(shared.cfg.borrow().footer_books.contains_key(&book_id));
            let tx = sender.input_sender().clone();
            own.connect_active_notify(move |r| tx.emit(Input::FooterScope(r.is_active())));
            own_row.append(&own_label);
            own_row.append(&own);
            col.append(&own_row);
            pop.set_child(Some(&col));
            let click = gtk::GestureClick::new();
            click.set_button(3);
            let p = pop.clone();
            click.connect_pressed(move |g, _, _, _| {
                g.set_state(gtk::EventSequenceState::Claimed);
                p.popup();
            });
            bottom.add_controller(click);
            pop
        };

        let split = adw::OverlaySplitView::new();
        split.set_sidebar(Some(&side.root));
        split.set_content(Some(&content));
        // The sidebar lies over the page: opening it must not make the page lay itself out again
        // for the narrower width (a chapter of a big book takes long), nor move the text.
        split.set_collapsed(true);
        split.set_min_sidebar_width(300.0);
        split.set_max_sidebar_width(380.0);
        split.set_show_sidebar(false);
        if rtl {
            split.set_sidebar_position(gtk::PackType::End);
        }
        {
            // the list is where the keyboard goes when the sidebar opens
            let side = side.clone();
            split.connect_show_sidebar_notify(move |s| {
                if s.shows_sidebar() {
                    let side = side.clone();
                    glib::idle_add_local_once(move || side.focus_current());
                }
            });
        }

        // ---- header -------------------------------------------------------------
        let header = adw::HeaderBar::new();
        let title = adw::WindowTitle::new(&title_text, "");
        header.set_title_widget(Some(&title));
        let sidebar_btn = gtk::ToggleButton::new();
        sidebar_btn.set_icon_name("sidebar-show-symbolic");
        sidebar_btn.set_tooltip_text(Some(&tr("Contents, bookmarks and notes (F9)")));
        split
            .bind_property("show-sidebar", &sidebar_btn, "active")
            .bidirectional()
            .sync_create()
            .build();
        if rtl {
            header.pack_end(&sidebar_btn);
        } else {
            header.pack_start(&sidebar_btn);
        }
        let back_btn = icon_button("go-previous-symbolic", &tr("Back to previous position"));
        back_btn.set_label(&tr("Back"));
        back_btn.set_icon_name("edit-undo-symbolic");
        back_btn.set_visible(false);
        header.pack_start(&back_btn);
        let forward_btn = icon_button(
            "go-next-symbolic",
            &tr("Forward to the place you came back from (Alt+Right)"),
        );
        forward_btn.set_label(&tr("Forward"));
        forward_btn.set_visible(false);
        header.pack_start(&forward_btn);
        let bookmark_btn = icon_button("bookmark-new-symbolic", &tr("Add bookmark (Ctrl+B)"));
        header.pack_start(&bookmark_btn);

        let primary = gtk::MenuButton::new();
        primary.set_icon_name("open-menu-symbolic");
        primary.set_primary(true);
        primary.set_tooltip_text(Some(&tr("Main menu")));
        header.pack_end(&primary);
        let fullscreen_btn = icon_button("view-fullscreen-symbolic", &tr("Fullscreen (F11)"));
        header.pack_end(&fullscreen_btn);
        let chrome_ctl = Rc::new(ChromeCtl::new());
        let tts_ui = {
            let tx = sender.input_sender().clone();
            let prefs = shared.cfg.borrow().tts.clone();
            super::reader_tts::TtsUi::build(&prefs, move |c| tx.emit(Input::Tts(c)))
        };
        header.pack_end(&tts_ui.button);

        // the settings file keeps the defaults; this book opens with its own look when it has one
        {
            let mut c = shared.cfg.borrow_mut();
            let defaults = c
                .reader_defaults
                .clone()
                .unwrap_or_else(|| c.reader.clone());
            c.reader = c.book_looks.for_open(book_id, &defaults);
            c.reader_defaults = Some(defaults);
        }
        let prefs0 = shared.cfg.borrow().reader.clone();
        let cur_prefs = Rc::new(RefCell::new(prefs0.clone()));
        let emit_prefs = {
            let tx = sender.input_sender().clone();
            let cur = cur_prefs.clone();
            Rc::new(move || tx.emit(Input::Prefs(cur.borrow().clone())))
        };

        // ---- main menu: theme, zoom, book actions, layout, app actions -----------
        let mpop = gtk::Popover::new();
        let mstack = gtk::Stack::new();
        mstack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
        mstack.set_vhomogeneous(false);
        let main = gtk::Box::new(gtk::Orientation::Vertical, 4);
        main.set_margin_all(10);
        main.set_width_request(280);

        let theme_box = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        theme_box.set_halign(gtk::Align::Center);
        theme_box.set_margin_bottom(6);
        let mut first_theme: Option<gtk::ToggleButton> = None;
        let mut theme_btns = Vec::new();
        for t in ReaderTheme::ALL {
            let b = gtk::ToggleButton::new();
            b.add_css_class("theme-swatch");
            b.add_css_class(&format!(
                "swatch-{}",
                t.css_class(false).trim_start_matches("reader-")
            ));
            if t == ReaderTheme::Auto {
                b.add_css_class("swatch-auto");
            }
            b.set_tooltip_text(Some(t.label()));
            b.set_active(t == prefs0.theme);
            if let Some(f) = &first_theme {
                b.set_group(Some(f));
            } else {
                first_theme = Some(b.clone());
            }
            let cur = cur_prefs.clone();
            let emit = emit_prefs.clone();
            b.connect_toggled(move |b| {
                if b.is_active()
                    && (cur.borrow().theme != t || cur.borrow().colors != ColorMode::Theme)
                {
                    {
                        let mut p = cur.borrow_mut();
                        p.theme = t;
                        p.colors = ColorMode::Theme;
                    }
                    emit();
                }
            });
            theme_box.append(&b);
            theme_btns.push((t, b));
        }
        main.append(&theme_box);
        let themes_ui = super::reader_theme_row::ThemeRow::new(
            shared.clone(),
            cur_prefs.clone(),
            emit_prefs.clone(),
            first_theme.clone().expect("theme swatches"),
            {
                let (tx, pop) = (sender.input_sender().clone(), mpop.clone());
                Rc::new(move |name| {
                    pop.popdown();
                    tx.emit(Input::ThemeEditor(name));
                })
            },
        );
        main.append(&themes_ui.root);
        let colors = super::colors::ColorControls::new(cur_prefs.clone(), emit_prefs.clone());
        colors.root.set_margin_bottom(6);
        main.append(&colors.root);

        let size_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        size_row.set_margin_bottom(6);
        let minus = icon_button("list-remove-symbolic", &tr("Smaller text (Ctrl+−)"));
        let plus = icon_button("list-add-symbolic", &tr("Larger text (Ctrl++)"));
        let reset = icon_button("zoom-original-symbolic", &tr("Reset text size"));
        let zoom_label = gtk::Label::new(Some(&format!("{:.0}%", prefs0.font_size / 14.0 * 100.0)));
        zoom_label.set_hexpand(true);
        zoom_label.add_css_class("heading");
        for b in [&minus, &plus, &reset] {
            b.add_css_class("circular");
        }
        size_row.append(&minus);
        size_row.append(&zoom_label);
        size_row.append(&plus);
        size_row.append(&reset);
        for (btn, delta) in [(&minus, -1.0), (&plus, 1.0)] {
            let cur = cur_prefs.clone();
            let emit = emit_prefs.clone();
            btn.connect_clicked(move |_| {
                {
                    let mut p = cur.borrow_mut();
                    p.font_size = (p.font_size + delta).clamp(8.0, 40.0);
                }
                emit();
            });
        }
        {
            let cur = cur_prefs.clone();
            let emit = emit_prefs.clone();
            reset.connect_clicked(move |_| {
                cur.borrow_mut().font_size = 14.0;
                emit();
            });
        }
        main.append(&size_row);
        main.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        let item = |label: &str, accel: &str, chevron: bool| -> gtk::Button {
            let b = gtk::Button::new();
            b.add_css_class("flat");
            b.add_css_class("menu-item");
            let bx = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let l = gtk::Label::new(Some(label));
            l.set_xalign(0.0);
            l.set_hexpand(true);
            bx.append(&l);
            if !accel.is_empty() {
                let a = gtk::Label::new(Some(&crate::shortcuts::display(accel, "")));
                a.add_css_class("dim-label");
                a.add_css_class("caption");
                bx.append(&a);
                let (weak, action) = (a.downgrade(), accel.to_string());
                crate::shortcuts::on_change(move || {
                    if let Some(l) = weak.upgrade() {
                        l.set_text(&crate::shortcuts::display(&action, ""));
                    }
                });
            }
            if chevron {
                bx.append(&gtk::Image::from_icon_name("go-next-symbolic"));
            }
            b.set_child(Some(&bx));
            b
        };
        let open_b = item(&tr("Open Book"), "", false);
        let reload_b = item(&tr("Reload"), "reader.reload", false);
        let goto_b = item(&tr("Go to"), "reader.goto", false);
        let layout_b = item(&tr("Layout"), "", true);
        let rsvp_b = item(&tr("Speed Reading"), "", false);
        let map_b = item(&tr("Book Map"), "reader.map", false);
        let ruler_b = item(&tr("Reading Ruler"), "reader.ruler", false);
        let footer_b = item(&tr("Footer Stats"), "", false);
        let prefs_b = item(&tr("Preferences"), "app.preferences", false);
        let keys_b = item(&tr("Keyboard Shortcuts"), "app.shortcuts", false);
        let about_b = item(&tr("About Bookx"), "", false);
        for b in [&open_b, &reload_b, &goto_b] {
            main.append(b);
        }
        goto_b.set_action_name(Some("reader.goto"));
        {
            let pop = mpop.clone();
            goto_b.connect_clicked(move |_| pop.popdown());
        }
        main.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        main.append(&layout_b);
        main.append(&rsvp_b);
        main.append(&map_b);
        main.append(&ruler_b);
        main.append(&footer_b);
        main.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        for b in [&prefs_b, &keys_b, &about_b] {
            main.append(b);
        }
        prefs_b.set_action_name(Some("app.preferences"));
        keys_b.set_action_name(Some("app.shortcuts"));
        about_b.set_action_name(Some("app.about"));
        for b in [&prefs_b, &keys_b, &about_b] {
            let pop = mpop.clone();
            b.connect_clicked(move |_| pop.popdown());
        }
        mstack.add_named(&main, Some("main"));

        let layout_page = gtk::Box::new(gtk::Orientation::Vertical, 4);
        layout_page.set_margin_all(10);
        let lhead = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let lback = gtk::Button::from_icon_name("go-previous-symbolic");
        lback.add_css_class("flat");
        lback.set_tooltip_text(Some(&tr("Back to the menu")));
        let ltitle = gtk::Label::new(Some(&tr("Layout")));
        ltitle.add_css_class("heading");
        lhead.append(&lback);
        lhead.append(&ltitle);
        layout_page.append(&lhead);
        let mut first_layout: Option<gtk::CheckButton> = None;
        let mut layout_btns = Vec::new();
        for (mode, label) in [
            (LayoutMode::TwoColumns, "Two Columns"),
            (LayoutMode::SingleColumn, "Single Column"),
            (LayoutMode::Scroll, "Scroll"),
        ] {
            let c = gtk::CheckButton::with_label(label);
            c.set_active(prefs0.layout == mode);
            if let Some(f) = &first_layout {
                c.set_group(Some(f));
            } else {
                first_layout = Some(c.clone());
            }
            let cur = cur_prefs.clone();
            let emit = emit_prefs.clone();
            let pop = mpop.clone();
            c.connect_toggled(move |c| {
                if c.is_active() && cur.borrow().layout != mode {
                    cur.borrow_mut().layout = mode;
                    emit();
                    pop.popdown();
                }
            });
            layout_page.append(&c);
            layout_btns.push((mode, c));
        }
        mstack.add_named(&layout_page, Some("layout"));
        {
            let s = mstack.clone();
            layout_b.connect_clicked(move |_| s.set_visible_child_name("layout"));
            let s = mstack.clone();
            lback.connect_clicked(move |_| s.set_visible_child_name("main"));
            let s = mstack.clone();
            mpop.connect_closed(move |_| s.set_visible_child_name("main"));
        }
        mpop.set_child(Some(&mstack));
        primary.set_popover(Some(&mpop));
        {
            let wire = |b: &gtk::Button, make: fn() -> Input| {
                let pop = mpop.clone();
                let tx = sender.input_sender().clone();
                b.connect_clicked(move |_| {
                    pop.popdown();
                    tx.emit(make());
                });
            };
            wire(&open_b, || Input::OpenBook);
            wire(&reload_b, || Input::Reload);
            wire(&rsvp_b, || Input::Rsvp);
            wire(&map_b, || Input::BookMap);
            wire(&footer_b, || Input::FooterConfigure);
        }

        // ---- text popover: font, spacing, margins, justification -----------------
        let appearance = gtk::MenuButton::new();
        appearance.set_icon_name("format-text-rich-symbolic");
        appearance.set_tooltip_text(Some(&tr("Text appearance")));
        let pop = gtk::Popover::new();
        let ab = gtk::Box::new(gtk::Orientation::Vertical, 12);
        ab.set_margin_all(14);
        ab.set_width_request(340);
        let heading = |text: &str| {
            let l = gtk::Label::new(Some(text));
            l.add_css_class("heading");
            l.set_xalign(0.0);
            l.set_accessible_role(gtk::AccessibleRole::Heading);
            l
        };
        // the style tweaks live here too: one place for everything about how the text looks
        let tweaks_ui = {
            let book = book_id;
            let get: Rc<dyn Fn() -> crate::tweaks::StyleTweaks> = {
                let shared = shared.clone();
                Rc::new(move || shared.cfg.borrow().tweaks.effective(book))
            };
            let tx = sender.input_sender().clone();
            let change: Rc<dyn Fn(crate::tweaks::StyleTweaks)> =
                Rc::new(move |t| tx.emit(Input::Tweaks(t, true)));
            super::reader_style::TweaksUi::build(get, change)
        };

        let fonts = [
            "Serif",
            "Sans",
            "Cantarell",
            "Noto Serif",
            "Source Serif 4",
            "Libertinus Serif",
            "Atkinson Hyperlegible",
            "OpenDyslexic",
            "Monospace",
        ];
        let font_dd = gtk::DropDown::from_strings(&fonts);
        super::audit::label(&font_dd, &tr("Font"));
        font_dd.set_selected(
            fonts
                .iter()
                .position(|f| *f == prefs0.font_family)
                .unwrap_or(0) as u32,
        );
        {
            let cur = cur_prefs.clone();
            let emit = emit_prefs.clone();
            font_dd.connect_selected_notify(move |d| {
                let f = fonts[d.selected() as usize % fonts.len()].to_string();
                if cur.borrow().font_family != f {
                    cur.borrow_mut().font_family = f;
                    emit();
                }
            });
        }
        let font_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let fl = gtk::Label::new(Some(&tr("Font")));
        fl.set_hexpand(true);
        fl.set_xalign(0.0);
        font_row.append(&fl);
        font_row.append(&font_dd);
        ab.append(&heading(&tr("Text")));
        ab.append(&font_row);
        ab.append(&tweaks_ui.text_group);
        ab.append(&heading(&tr("Layout")));
        let columns_dd = gtk::DropDown::from_strings(&["Two columns", "Single column", "Scroll"]);
        super::audit::label(&columns_dd, &tr("Columns"));
        let mode_at = |m: LayoutMode| match m {
            LayoutMode::TwoColumns => 0,
            LayoutMode::SingleColumn => 1,
            LayoutMode::Scroll => 2,
        };
        columns_dd.set_selected(mode_at(prefs0.layout));
        {
            let cur = cur_prefs.clone();
            let emit = emit_prefs.clone();
            columns_dd.connect_selected_notify(move |d| {
                let m = [
                    LayoutMode::TwoColumns,
                    LayoutMode::SingleColumn,
                    LayoutMode::Scroll,
                ][d.selected() as usize % 3];
                if cur.borrow().layout != m {
                    cur.borrow_mut().layout = m;
                    emit();
                }
            });
        }
        let col_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let cl = gtk::Label::new(Some(&tr("Columns")));
        cl.set_hexpand(true);
        cl.set_xalign(0.0);
        col_row.append(&cl);
        col_row.append(&columns_dd);
        ab.append(&col_row);

        let scale_row = |label: &str,
                         min: f64,
                         max: f64,
                         step: f64,
                         val: f64,
                         f: Box<dyn Fn(&mut ReaderPrefs, f64)>| {
            let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let l = gtk::Label::new(Some(label));
            l.set_xalign(0.0);
            let s = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, step);
            super::audit::label(&s, label);
            s.set_value(val);
            s.set_draw_value(false);
            let cur = cur_prefs.clone();
            let emit = emit_prefs.clone();
            s.connect_value_changed(move |s| {
                f(&mut cur.borrow_mut(), s.value());
                emit();
            });
            b.append(&l);
            b.append(&s);
            b
        };
        ab.append(&scale_row(
            "Line spacing",
            1.0,
            2.0,
            0.05,
            prefs0.line_spacing,
            Box::new(|p, v| p.line_spacing = v),
        ));
        ab.append(&scale_row(
            "Margins",
            8.0,
            160.0,
            4.0,
            prefs0.margin,
            Box::new(|p, v| p.margin = v),
        ));

        let just = gtk::Switch::new();
        super::audit::label(&just, &tr("Justify text"));
        just.set_active(prefs0.justify);
        just.set_valign(gtk::Align::Center);
        {
            let cur = cur_prefs.clone();
            let emit = emit_prefs.clone();
            just.connect_active_notify(move |s| {
                if cur.borrow().justify != s.is_active() {
                    cur.borrow_mut().justify = s.is_active();
                    emit();
                }
            });
        }
        let jr = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let jl = gtk::Label::new(Some(&tr("Justify text")));
        jl.set_hexpand(true);
        jl.set_xalign(0.0);
        jr.append(&jl);
        jr.append(&just);
        ab.append(&jr);
        let hyphen_sw = gtk::Switch::new();
        super::audit::label(&hyphen_sw, &tr("Hyphenate words"));
        hyphen_sw.set_active(prefs0.hyphenate);
        hyphen_sw.set_valign(gtk::Align::Center);
        {
            let cur = cur_prefs.clone();
            let emit = emit_prefs.clone();
            hyphen_sw.connect_active_notify(move |s| {
                if cur.borrow().hyphenate != s.is_active() {
                    cur.borrow_mut().hyphenate = s.is_active();
                    emit();
                }
            });
        }
        let hr = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let hl = gtk::Label::new(Some(&tr("Hyphenate words")));
        hl.set_hexpand(true);
        hl.set_xalign(0.0);
        hr.append(&hl);
        hr.append(&hyphen_sw);
        ab.append(&hr);
        ab.append(&tweaks_ui.advanced_group);
        ab.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let look_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        look_row.set_homogeneous(true);
        let default_b = gtk::Button::with_label(&tr("Use as default for new books"));
        let reset_b = gtk::Button::with_label(&tr("Reset this book to defaults"));
        default_b.set_tooltip_text(Some(&tr(
            "Every change you make is kept for this book. This makes the current look the starting point of books you have not changed yet.",
        )));
        reset_b.set_tooltip_text(Some(&tr(
            "Forget this book's own look and use the defaults again",
        )));
        for (b, input) in [
            (&default_b, (|| Input::UseAsDefault) as fn() -> Input),
            (&reset_b, || Input::ResetLook),
        ] {
            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                l.set_wrap(true);
                l.set_justify(gtk::Justification::Center);
            }
            let (tx, pop) = (sender.input_sender().clone(), pop.clone());
            b.connect_clicked(move |_| {
                pop.popdown();
                tx.emit(input());
            });
            look_row.append(b);
        }
        ab.append(&look_row);
        // never taller than the window: it scrolls
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_propagate_natural_height(true);
        scroller.set_max_content_height(540);
        scroller.set_min_content_width(340);
        scroller.set_child(Some(&ab));
        pop.set_child(Some(&scroller));
        appearance.set_popover(Some(&pop));
        header.pack_end(&appearance);

        header.pack_end(&search_btn);

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        let rsvp_ctl = {
            let (tx, tx2) = (sender.input_sender().clone(), sender.input_sender().clone());
            super::rsvp::RsvpCtl::new(
                Rc::new(move |c| tx.emit(Input::RsvpCmd(c))),
                Rc::new(move || tx2.emit(Input::RsvpTick)),
            )
        };
        {
            // conditions for `until:` in dev scripts (tests wait on these instead of sleeping)
            let p = tts_ui.player.clone();
            crate::devtools::probe("tts-playing", move || {
                let d = p.dev_dump();
                p.widget.is_mapped()
                    && d.contains("visible=1")
                    && d.contains("icon=media-playback-pause")
            });
            crate::devtools::probe("cards-settled", super::card_drag::settled);
            let o = split.clone();
            crate::devtools::probe("reader", move || o.is_mapped());
            // the book is laid out: the page has text
            let (o, c) = (split.clone(), canvas.clone());
            crate::devtools::probe("page-ready", move || {
                o.is_mapped() && !c.area.page_text().is_empty()
            });
            let p = tts_ui.player.clone();
            crate::devtools::probe("tts-card", move || {
                p.widget.is_mapped() && p.dev_dump().contains("visible=1")
            });
            let p = tts_ui.player.clone();
            crate::devtools::probe("tts-card-paused", move || {
                let d = p.dev_dump();
                p.widget.is_mapped()
                    && d.contains("visible=1")
                    && d.contains("icon=media-playback-start")
            });
        }
        let page_overlay = gtk::Overlay::new();
        page_overlay.set_child(Some(&split));
        page_overlay.add_overlay(&tts_ui.player.widget);
        page_overlay.add_overlay(&rsvp_ctl.widget);
        super::card_drag::install(
            &page_overlay,
            vec![
                (
                    tts_ui.player.widget.clone().upcast(),
                    tts_ui.player.pos.clone(),
                    crate::reader_cards::TTS_PLACE,
                ),
                (
                    rsvp_ctl.widget.clone().upcast(),
                    rsvp_ctl.pos.clone(),
                    crate::reader_cards::RSVP_PLACE,
                ),
            ],
        );
        toolbar.set_content(Some(&page_overlay));
        toolbar.add_bottom_bar(&bottom);
        root.set_child(Some(&toolbar));
        root.set_title(&title_text);
        root.add_css_class("reader-page");

        // Chrome that merges into the page: after a few idle seconds the header and the
        // navigation row fade out (only the title and the status line stay). Moving the
        // pointer or pressing a key brings them back, and they never hide while you are
        // interacting with them: an open menu or dropdown, or the pointer resting on a bar.
        let (chrome_probe, chrome_eval, chrome_wake): (
            Rc<dyn Fn() -> bool>,
            Rc<dyn Fn()>,
            Rc<dyn Fn()>,
        ) = {
            let hovering = Rc::new(Cell::new(false));
            let dragging = Rc::new(Cell::new(false));
            for bar in [header.upcast_ref::<gtk::Widget>(), bottom.upcast_ref()] {
                let m = gtk::EventControllerMotion::new();
                let h = hovering.clone();
                m.connect_enter(move |_, _, _| h.set(true));
                let h = hovering.clone();
                m.connect_leave(move |_| h.set(false));
                bar.add_controller(m);
            }
            let busy: Rc<dyn Fn() -> bool> = {
                let pops: Vec<Option<gtk::Popover>> = vec![
                    search_btn.popover(),
                    primary.popover(),
                    appearance.popover(),
                    tts_ui.button.popover(),
                    Some(sel_pop.clone()),
                ];
                let player = tts_ui.player.clone();
                let rsvp = rsvp_ctl.clone();
                let root = root.clone();
                let ctl = chrome_ctl.clone();
                let (hv, dr) = (hovering.clone(), dragging.clone());
                let (hd, bt) = (header.clone(), bottom.clone());
                Rc::new(move || {
                    let open = pops.iter().flatten().any(|p| p.is_visible())
                        || player.in_use()
                        || rsvp.in_use();
                    let keyboard_focus = root.root().and_then(|r| r.focus()).is_some_and(|f| {
                        f.state_flags().contains(gtk::StateFlags::FOCUS_VISIBLE)
                            && (f.is_ancestor(&hd) || f.is_ancestor(&bt))
                    });
                    chrome_busy(ChromeUse {
                        popover_open: open,
                        pointer_y: ctl.pointer_y.get(),
                        height: root.height() as f64,
                        hovering: hv.get(),
                        keyboard_focus,
                        dragging: dr.get(),
                    })
                })
            };
            // The bars fade after the idle delay (1.5 s while read aloud or speed reading plays, else 4 s)
            // and never while they are in use: use restarts the countdown.
            let eval: Rc<dyn Fn()> = {
                let (root, busy, ctl) = (root.clone(), busy.clone(), chrome_ctl.clone());
                Rc::new(move || {
                    if !crate::eink::effects(crate::eink::is_active()).chrome_fades {
                        return; // e-ink: the bars stay
                    }
                    if busy() {
                        ctl.touch();
                    } else if crate::chrome_visibility::fade_now(
                        ctl.playing.get(),
                        ctl.idle_ms(),
                        false,
                    ) {
                        root.add_css_class("chrome-idle");
                        // while something plays nothing of the bars is left (not the title nor the status line)
                        if ctl.playing.get() {
                            root.add_css_class("chrome-playing");
                        } else {
                            root.remove_css_class("chrome-playing");
                        }
                    } else if !ctl.playing.get() {
                        root.remove_css_class("chrome-playing");
                    }
                })
            };
            let wake: Rc<dyn Fn()> = {
                let (root, ctl) = (root.clone(), chrome_ctl.clone());
                Rc::new(move || {
                    root.remove_css_class("chrome-idle");
                    root.remove_css_class("chrome-playing");
                    ctl.touch();
                })
            };
            {
                let (weak, e) = (root.downgrade(), eval.clone());
                glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
                    if weak.upgrade().is_none() {
                        return glib::ControlFlow::Break;
                    }
                    e();
                    glib::ControlFlow::Continue
                });
            }
            let motion = gtk::EventControllerMotion::new();
            motion.set_propagation_phase(gtk::PropagationPhase::Capture);
            let w = wake.clone();
            let ctl = chrome_ctl.clone();
            let dr = dragging.clone();
            // GTK sends the same motion again whenever what is under a resting pointer changes (the
            // highlight moves, a card redraws): only a pointer that really moved is an interaction
            let last_at = Cell::new((f64::NAN, f64::NAN));
            motion.connect_motion(move |m, x, y| {
                ctl.pointer_y.set(y);
                dr.set(
                    m.current_event_state()
                        .contains(gtk::gdk::ModifierType::BUTTON1_MASK),
                );
                if last_at.replace((x, y)) != (x, y) {
                    w();
                }
            });
            let ctl = chrome_ctl.clone();
            motion.connect_leave(move |_| ctl.pointer_y.set(-1.0));
            root.add_controller(motion);
            let press = gtk::GestureClick::new();
            press.set_propagation_phase(gtk::PropagationPhase::Capture);
            let (dr, w) = (dragging.clone(), wake.clone());
            press.connect_pressed(move |g, _, _, _| {
                g.set_state(gtk::EventSequenceState::None);
                dr.set(true);
                w();
            });
            let (dr, w) = (dragging.clone(), wake.clone());
            press.connect_released(move |_, _, _, _| {
                dr.set(false);
                w(); // letting go of the mouse starts the countdown
            });
            let dr = dragging.clone();
            press.connect_cancel(move |_, _| dr.set(false));
            root.add_controller(press);
            let k = gtk::EventControllerKey::new();
            k.set_propagation_phase(gtk::PropagationPhase::Capture);
            let w = wake.clone();
            k.connect_key_pressed(move |_, _, _, _| {
                w();
                glib::Propagation::Proceed
            });
            root.add_controller(k);
            // popovers close -> start the idle countdown again
            for p in [
                search_btn.popover(),
                primary.popover(),
                appearance.popover(),
                tts_ui.button.popover(),
            ]
            .into_iter()
            .flatten()
            {
                let w = wake.clone();
                p.connect_closed(move |_| w());
                let w = wake.clone();
                p.connect_show(move |_| w()); // opening a menu reveals faded bars
            }
            wake();
            (busy, eval, wake)
        };

        // wiring of misc buttons
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
            forward_btn.connect_clicked(move |_| tx.emit(Input::Forward));
            let tx = sender.input_sender().clone();
            fullscreen_btn.connect_clicked(move |_| tx.emit(Input::Fullscreen));
            let tx = sender.input_sender().clone();
            slider.connect_change_value(move |_, _, v| {
                tx.emit(Input::Seek((v / 1000.0).clamp(0.0, 1.0)));
                glib::Propagation::Proceed
            });
        }

        // while speed reading is on, space, the arrows and Escape belong to it
        {
            let rk = gtk::EventControllerKey::new();
            rk.set_propagation_phase(gtk::PropagationPhase::Capture);
            let (tx, on, root_c) = (
                sender.input_sender().clone(),
                rsvp_ctl.active.clone(),
                root.clone(),
            );
            rk.connect_key_pressed(move |_, key, _, mods| {
                if !on.get()
                    || mods
                        .intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK)
                {
                    return glib::Propagation::Proceed;
                }
                let typing = root_c
                    .root()
                    .and_then(|r| r.focus())
                    .is_some_and(|f| f.is::<gtk::Editable>());
                match super::rsvp::key_cmd(key) {
                    // Escape belongs to the card that has the focus (it closes that card only)
                    Some(super::rsvp::Cmd::Close) => glib::Propagation::Proceed,
                    Some(c) if !typing => {
                        tx.emit(Input::RsvpCmd(c));
                        glib::Propagation::Stop
                    }
                    _ => glib::Propagation::Proceed,
                }
            });
            root.add_controller(rk);
        }

        // keyboard
        let keys = gtk::EventControllerKey::new();
        {
            let tx = sender.input_sender().clone();
            let root_c = root.clone();
            let kb_canvas = canvas.clone();
            keys.connect_key_pressed(move |_, key, _, mods| {
                use gdk::Key;
                // let text entries keep their keys
                if let Some(win) = window_of(&root_c) {
                    if let Some(f) = gtk::prelude::GtkWindowExt::focus(&win) {
                        if f.is::<gtk::Text>() || f.is::<gtk::TextView>() {
                            return glib::Propagation::Proceed;
                        }
                    }
                }
                // keys the user has changed (Preferences → Keyboard Shortcuts)
                if super::shortcuts_dialog::handle_reader_key(root_c.upcast_ref(), key, mods)
                    .is_some()
                {
                    return glib::Propagation::Stop;
                }
                let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
                let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
                let alt = mods.contains(gdk::ModifierType::ALT_MASK);
                let has_sel = kb_canvas.has_selection();
                if shift && !matches!(key, Key::space) {
                    use crate::a11y::Step;
                    // in vertical text the characters run down and the sentences across
                    let mode = kb_canvas.writing_mode();
                    let (step, fwd) = if mode.is_vertical() {
                        let char_step = if ctrl { Step::Word } else { Step::Char };
                        let rl = mode == crate::core::doc::WritingMode::VerticalRl;
                        match key {
                            Key::Down => (char_step, true),
                            Key::Up => (char_step, false),
                            Key::Left => (Step::Sentence, rl),
                            Key::Right => (Step::Sentence, !rl),
                            _ => (Step::Char, true),
                        }
                    } else {
                        match key {
                            Key::Right => (if ctrl { Step::Word } else { Step::Char }, true),
                            Key::Left => (if ctrl { Step::Word } else { Step::Char }, false),
                            Key::Down => (Step::Sentence, true),
                            Key::Up => (Step::Sentence, false),
                            _ => (Step::Char, true),
                        }
                    };
                    if matches!(key, Key::Right | Key::Left | Key::Down | Key::Up) {
                        tx.emit(Input::KeySelect(step, fwd));
                        return glib::Propagation::Stop;
                    }
                }
                if has_sel && !ctrl {
                    let act = match key {
                        Key::Return | Key::KP_Enter => Some("commit"),
                        Key::d => Some("dict"),
                        Key::w => Some("wiki"),
                        Key::t => Some("translate"),
                        Key::h => Some("highlight"),
                        Key::n => Some("note"),
                        Key::Escape => Some("clear"),
                        _ => None,
                    };
                    if let Some(a) = act {
                        tx.emit(Input::SelectionKey(a));
                        return glib::Propagation::Stop;
                    }
                }
                if has_sel && ctrl && matches!(key, Key::c) {
                    tx.emit(Input::SelectionKey("copy"));
                    return glib::Propagation::Stop;
                }
                match key {
                    Key::Left if alt => tx.emit(Input::Back),
                    Key::Right if alt => tx.emit(Input::Forward),
                    Key::Down if alt => super::reading_ruler::act(1),
                    Key::Up if alt => super::reading_ruler::act(-1),
                    Key::g if ctrl => tx.emit(Input::GotoDialog),
                    Key::Right if !ctrl => tx.emit(Input::Turn(right_is_forward(rtl))),
                    Key::Left if !ctrl => tx.emit(Input::Turn(!right_is_forward(rtl))),
                    Key::Page_Down | Key::Down if !ctrl => tx.emit(Input::Turn(true)),
                    Key::Page_Up | Key::Up if !ctrl => tx.emit(Input::Turn(false)),
                    Key::space => tx.emit(Input::Turn(!shift)),
                    Key::Right => tx.emit(Input::Chapter(right_is_forward(rtl))),
                    Key::Left => tx.emit(Input::Chapter(!right_is_forward(rtl))),
                    Key::Down => tx.emit(Input::Chapter(true)),
                    Key::Up => tx.emit(Input::Chapter(false)),
                    Key::Home => tx.emit(Input::Seek(0.0)),
                    Key::End => tx.emit(Input::Seek(1.0)),
                    Key::F11 => tx.emit(Input::Fullscreen),
                    Key::Escape => tx.emit(Input::Quit),
                    Key::b if ctrl => tx.emit(Input::ToggleBookmark),
                    Key::r if ctrl => tx.emit(Input::Reload),
                    Key::f if ctrl => tx.emit(Input::ShowTab("search")),
                    Key::plus | Key::equal | Key::KP_Add if ctrl => {
                        tx.emit(Input::Prefs(ReaderPrefs {
                            font_size: -1.0,
                            ..Default::default()
                        }))
                    }
                    Key::minus | Key::KP_Subtract if ctrl => tx.emit(Input::Prefs(ReaderPrefs {
                        font_size: -2.0,
                        ..Default::default()
                    })),
                    _ => {
                        // keys from other readers (Calibre, Foliate, KOReader, Okular) that the
                        // lines above do not know: the registry's defaults of the reader
                        if super::shortcuts_dialog::handle_reader_default(
                            root_c.upcast_ref(),
                            key,
                            mods,
                        )
                        .is_none()
                        {
                            return glib::Propagation::Proceed;
                        }
                    }
                }
                glib::Propagation::Stop
            });
        }
        root.add_controller(keys);

        {
            let tx = sender.input_sender().clone();
            adw::StyleManager::default().connect_high_contrast_notify(move |_| {
                tx.emit(Input::Dev("restyle".into()));
            });
        }

        // the actions of the command palette: the same things the keys and buttons do
        {
            let group = gio::SimpleActionGroup::new();
            let add = |name: &str, input: Rc<dyn Fn() -> Input>| {
                let a = gio::SimpleAction::new(name, None);
                let tx = sender.input_sender().clone();
                a.connect_activate(move |_, _| tx.emit(input()));
                group.add_action(&a);
            };
            add("next-page", Rc::new(|| Input::Turn(true)));
            add("prev-page", Rc::new(|| Input::Turn(false)));
            add("next-chapter", Rc::new(|| Input::Chapter(true)));
            add("prev-chapter", Rc::new(|| Input::Chapter(false)));
            add("start", Rc::new(|| Input::Seek(0.0)));
            add("end", Rc::new(|| Input::Seek(1.0)));
            add("bookmark", Rc::new(|| Input::ToggleBookmark));
            add("goto", Rc::new(|| Input::GotoDialog));
            add("map", Rc::new(|| Input::BookMap));
            add("new-theme", Rc::new(|| Input::ThemeEditor(None)));
            add("use-as-default", Rc::new(|| Input::UseAsDefault));
            add("reset-look", Rc::new(|| Input::ResetLook));
            add("jump-back", Rc::new(|| Input::Back));
            add("jump-forward", Rc::new(|| Input::Forward));
            add("search", Rc::new(|| Input::ShowTab("search")));
            add("contents", Rc::new(|| Input::ShowTab("contents")));
            add("notes", Rc::new(|| Input::ShowTab("notes")));
            add("fullscreen", Rc::new(|| Input::Fullscreen));
            add("reload", Rc::new(|| Input::Reload));
            add("properties", Rc::new(|| Input::Properties));
            add("rsvp", Rc::new(|| Input::Rsvp));
            for (name, dir) in [("ruler", 0), ("ruler-up", -1), ("ruler-down", 1)] {
                let a = gio::SimpleAction::new(name, None);
                a.connect_activate(move |_, _| super::reading_ruler::act(dir));
                group.add_action(&a);
            }
            add("back", Rc::new(|| Input::Quit));
            let size = |d: f64| {
                Rc::new(move || {
                    Input::Prefs(ReaderPrefs {
                        font_size: d,
                        ..Default::default()
                    })
                }) as Rc<dyn Fn() -> Input>
            };
            add("open", Rc::new(|| Input::OpenBook));
            add(
                "read-aloud",
                Rc::new(|| Input::Tts(super::reader_tts::Cmd::PlayPause)),
            );
            {
                let sh = shared.clone();
                add(
                    "reset-size",
                    Rc::new(move || {
                        let mut p = sh.cfg.borrow().reader.clone();
                        p.font_size = 14.0;
                        Input::Prefs(p)
                    }),
                );
                let sh = shared.clone();
                add(
                    "cycle-theme",
                    Rc::new(move || {
                        let mut p = sh.cfg.borrow().reader.clone();
                        p.theme = next_theme(p.theme);
                        p.colors = ColorMode::Theme;
                        Input::Prefs(p)
                    }),
                );
            }
            add("larger", size(-1.0));
            add("smaller", size(-2.0));
            for (name, theme) in [
                ("theme-light", ReaderTheme::Light),
                ("theme-sepia", ReaderTheme::Sepia),
                ("theme-dark", ReaderTheme::Dark),
            ] {
                let sh = shared.clone();
                add(
                    name,
                    Rc::new(move || {
                        let mut p = sh.cfg.borrow().reader.clone();
                        p.theme = theme;
                        Input::Prefs(p)
                    }),
                );
            }
            for (name, layout) in [
                ("layout-two", LayoutMode::TwoColumns),
                ("layout-single", LayoutMode::SingleColumn),
                ("layout-scroll", LayoutMode::Scroll),
            ] {
                let sh = shared.clone();
                add(
                    name,
                    Rc::new(move || {
                        let mut p = sh.cfg.borrow().reader.clone();
                        p.layout = layout;
                        Input::Prefs(p)
                    }),
                );
            }
            root.insert_action_group("reader", Some(&group));
            super::palette::register_group("reader", &group);
        }

        // periodic persistence
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

        let (sel_pop_guard, footer_pop_guard, area_guard) =
            (sel_pop.clone(), footer_pop.clone(), canvas.area.clone());
        let w = W {
            page: root.clone(),
            title,
            split,
            stack,
            side,
            search_entry,
            search_btn,
            search_list,
            search_status,
            slider,
            slider_row: slider_row.clone(),
            footer_pop,
            handles,
            left_label,
            mid_label,
            right_label,
            bookmark_btn,
            back_btn,
            forward_btn,
            sel_pop,
            sel_stack,
            sel_remove,
            switching: switching.clone(),
            dict_btn,
            wiki_btn,
            note_view,
            note_btn,
            note_preview,
            note_label,
            note_save,
            prev_btn,
            next_btn,
            fullscreen_btn,
            chrome: chrome_ctl.clone(),
            chrome_eval: chrome_eval.clone(),
            chrome_wake: chrome_wake.clone(),
            tts: tts_ui,
            chrome_probe,
            swatches,
            style_btns,
            theme_btns,
            themes_ui,
            colors,
            layout_btns,
            columns_dd,
            hyphen_sw,
            zoom_label,
            wiki_text,
            dict_text,
            dict_prompt,
            wiki_spin,
            wiki_img,
            wiki_url,
            tr_text,
            tr_spin,
            tr_dd,
        };
        let page_map = crate::page_list::page_map(&book);
        let page_marks = if page_map.is_some() {
            crate::page_list::marks(&book)
        } else {
            Vec::new()
        };
        let mut model = Reader {
            shared: shared.clone(),
            book_id,
            book,
            canvas,
            w,
            sender: sender.clone(),
            spine,
            weights,
            toc,
            spine_idx: 0,
            chapter_words: 0,
            pos: PosInfo {
                page: 1,
                pages: 1,
                frac: 0.0,
            },
            selection: None,
            jumps: Default::default(),
            page_map,
            dwell_at: None,
            page_marks,
            last_search: String::new(),
            search_gen: Rc::new(Cell::new(0)),
            search_hits: Vec::new(),
            search_active: None,
            pending_hit: None,
            dirty,
            timer: Some(timer),
            fullscreen: false,
            chapter_pages: RefCell::new(HashMap::new()),
            cur_prefs,
            fetch_gen: Rc::new(Cell::new(0)),
            hl_style: HighlightStyle::default(),
            tweaks_ui: Rc::new(RefCell::new(Some(tweaks_ui))),
            fixed,
            live: Cell::new((None, 0)),
            opened: std::time::Instant::now(),
            footer_timer: None,
            overlay_guard: {
                let (pop, area) = (sel_pop_guard.clone(), area_guard.clone());
                let pop2 = pop.clone();
                let g = super::overlay_guard::OverlayGuard::install(
                    &root,
                    move || {
                        pop.popdown();
                        super::overlay_guard::drop_tooltip(&area);
                    },
                    move || pop2.is_autohide(),
                );
                g.watch_popover(&footer_pop_guard);
                g
            },
            _dict_sub: Some({
                let tx = sender.input_sender().clone();
                super::dict_service::subscribe(move |e| tx.emit(Input::DictEvent(e.clone())))
            }),
            dict_lang: RefCell::new(None),
            dict_waiting: Cell::new(false),
            dict_prompt_state: RefCell::new(crate::dict_auto::Prompt::Hidden),
            dict_opened: Cell::new(false),
            _footer_sub: Some({
                let tx = sender.input_sender().clone();
                crate::settings_bus::footer().subscribe(move || tx.emit(Input::FooterChanged))
            }),
            rsvp: rsvp_ctl,
            tts: TtsCtl {
                engine: None,
                session: crate::tts::Session::new(),
                timer: None,
                advancing: false,
                prefetched: None,
                prep: None,
                notice: None,
                not_ready: None,
                want_from: 0,
                shown_chapter: None,
                mpris: None,
                offered: 0,
                parked: None,
                cards_saved: None,
                cards_frozen: false,
                engine_id: String::new(),
                language: String::new(),
                sleep: crate::tts::SleepTimer::Off,
            },
        };
        model.apply_prefs();
        model.rebuild_lists();
        model.refresh_live();
        model.footer_timer = {
            let tx = sender.input_sender().clone();
            Some(glib::timeout_add_seconds_local(30, move || {
                tx.emit(Input::FooterTick);
                glib::ControlFlow::Continue
            }))
        };

        {
            let start = shared.lib.borrow().get(book_id).and_then(|b| b.position);
            let (idx, frac) = start.map(|p| (p.spine, Some(p.frac))).unwrap_or((0, None));
            // Defer the first layout until the canvas has a size.
            let tx = sender.input_sender().clone();
            glib::idle_add_local_once(move || {
                tx.emit(Input::GoSpine(idx, frac, None));
                let tx2 = tx.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
                    tx2.emit(Input::RestoreCards)
                });
            });
        }
        {
            // reading position sync: ask the server (after the window exists, for the question)
            let (shared, weights, page) =
                (shared.clone(), model.weights.clone(), model.w.page.clone());
            let tx = sender.input_sender().clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || {
                super::kosync::open_book(
                    &shared,
                    book_id,
                    weights,
                    page.upcast(),
                    Rc::new(move |spine, frac| tx.emit(Input::GoSpine(spine, Some(frac), None))),
                );
            });
        }
        model.canvas.area.grab_focus();
        {
            // the page is not in the window yet: put the keyboard on the text once it is shown, so that Enter
            // and the arrows act on the book and not on the Back button
            let area = model.canvas.area.clone();
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
            Input::Turn(fwd) => self.canvas.turn(fwd),
            Input::Chapter(fwd) => {
                let i = self.spine_idx;
                if fwd && i + 1 < self.spine.len() {
                    self.load_chapter(i + 1, None, None, false);
                } else if !fwd && i > 0 {
                    self.load_chapter(i - 1, None, None, false);
                }
            }
            Input::Edge(e) => match e {
                Edge::Next => {
                    if self.spine_idx + 1 < self.spine.len() {
                        self.load_chapter(self.spine_idx + 1, None, None, false);
                    } else {
                        self.save_progress();
                        let _ =
                            sender.output(Output::Toast(tr("You've reached the end of the book")));
                    }
                }
                Edge::Prev => {
                    if self.spine_idx > 0 {
                        self.load_chapter(self.spine_idx - 1, None, None, true);
                    }
                }
            },
            Input::GoSpine(i, f, a) => self.load_chapter(i, f, a, false),
            Input::GoHref(h) => {
                if let Some((i, frag)) = self.book.locate(&h) {
                    self.note_jump();
                    if i == self.spine_idx {
                        if let Some(f) = frag {
                            self.canvas.goto_anchor(&f);
                        } else {
                            self.canvas.goto_fraction(0.0);
                        }
                    } else {
                        self.load_chapter(i, Some(0.0), frag, false);
                    }
                    if self.w.split.is_collapsed() {
                        self.w.split.set_show_sidebar(false);
                    }
                    self.canvas.area.grab_focus();
                }
            }
            Input::Link(l) => {
                let note = (!is_external(&l) && self.shared.cfg.borrow().footnote_popups)
                    .then(|| self.footnote_for(&l))
                    .flatten();
                match note {
                    Some(text) => {
                        let tx = sender.input_sender().clone();
                        super::reader_extra::show_footnote(&self.canvas.area, &text, move || {
                            tx.emit(Input::FollowLink(l.clone()))
                        });
                    }
                    None => {
                        super::reader_extra::clear();
                        sender.input(Input::FollowLink(l))
                    }
                }
            }
            Input::FollowLink(l) => {
                if is_external(&l) {
                    open_uri(&l);
                } else if let Some((i, frag)) = self.book.locate(&l) {
                    self.note_jump();
                    if i == self.spine_idx {
                        match frag {
                            Some(f) => {
                                self.canvas.goto_anchor(&f);
                            }
                            None => self.canvas.goto_fraction(0.0),
                        }
                    } else {
                        self.load_chapter(i, Some(0.0), frag, false);
                    }
                } else {
                    let _ = sender.output(Output::Toast(tr("That link points outside the book")));
                }
            }
            Input::Back => {
                let here = crate::jump_history::Place::new(self.spine_idx, self.pos.frac);
                if let Some(to) = self.jumps.back(here) {
                    self.show_place(to);
                }
                self.update_jump_buttons();
            }
            Input::Forward => {
                let here = crate::jump_history::Place::new(self.spine_idx, self.pos.frac);
                if let Some(to) = self.jumps.forward(here) {
                    self.show_place(to);
                }
                self.update_jump_buttons();
            }
            Input::GotoDialog => self.goto_dialog(),
            Input::BookMap => self.book_map(),
            Input::MapJump(f) => {
                self.note_jump();
                // a chapter's start belongs to that chapter, not to the end of the one before
                self.goto_overall((f + 1e-6).min(1.0));
            }
            Input::ThemeEditor(name) => super::reader_theme_editor::show_themes(
                self.w.page.upcast_ref(),
                &self.profile_ctx(),
                name.as_deref(),
            ),
            Input::UseAsDefault => {
                {
                    let mut cfg = self.shared.cfg.borrow_mut();
                    cfg.reader_defaults = Some(cfg.reader.clone());
                    // the style tweaks of this book become the ones of books without their own
                    let t = cfg.tweaks.effective(self.book_id);
                    cfg.tweaks.set(self.book_id, false, t);
                    cfg.save();
                }
                let _ = sender.output(Output::Toast(tr("Saved as the default for new books")));
            }
            Input::ResetLook => {
                let defaults = {
                    let mut cfg = self.shared.cfg.borrow_mut();
                    cfg.book_looks.reset(self.book_id);
                    cfg.tweaks.reset_book(self.book_id);
                    let d = cfg
                        .reader_defaults
                        .clone()
                        .unwrap_or_else(|| cfg.reader.clone());
                    cfg.save();
                    d
                };
                self.shared.cfg.borrow_mut().reader = defaults;
                self.apply_prefs();
                if let Some(ui) = self.tweaks_ui.borrow().as_ref() {
                    ui.refresh();
                }
                // the style sheet of the book may have changed too
                let frac = self.pos.frac;
                self.load_chapter(self.spine_idx, Some(frac), None, false);
                let _ = sender.output(Output::Toast(tr("This book uses the default look again")));
            }
            Input::ThemesChanged => {
                self.w.themes_ui.rebuild();
                let p = self.shared.cfg.borrow().reader.clone();
                self.w.themes_ui.sync(&p);
            }
            Input::OpenImage(src) => self.open_image(&src),
            Input::GotoText(text) => {
                let _ = self.goto_text(&text);
            }
            Input::Position(p) => {
                self.pos = p;
                if !self.dict_opened.replace(true) {
                    // a book whose language has no dictionary: with the setting on, it is fetched now
                    let lang = self.dict_language();
                    super::dict_service::book_opened(&self.shared, &lang);
                }
                self.w.handles.refresh();
                self.chapter_pages
                    .borrow_mut()
                    .insert(self.spine_idx, p.pages);
                self.update_position_ui();
                self.update_bookmark_button();
                self.save_progress();
                self.track_reading();
                self.dwell_tick();
            }
            Input::Seek(f) => {
                self.note_jump_burst();
                self.goto_overall(f)
            }
            Input::Selection(s) => match &s {
                Some(info) => {
                    self.selection = Some(info.clone());
                    self.w.handles.refresh();
                    self.show_selection_popover(info);
                }
                None => {
                    self.w.handles.refresh();
                    // clicking away while the note box is open keeps what was typed
                    if self.w.sel_stack.visible_child_name().as_deref() == Some("note") {
                        let b = self.w.note_view.buffer();
                        let (st, en) = b.bounds();
                        let text = b.text(&st, &en, false).trim().to_string();
                        let old = self
                            .selection
                            .as_ref()
                            .and_then(|s| s.highlight.as_ref())
                            .and_then(|id| {
                                self.shared.lib.borrow().get(self.book_id).and_then(|bk| {
                                    bk.highlights
                                        .iter()
                                        .find(|h| &h.id == id)
                                        .map(|h| h.note.clone())
                                })
                            })
                            .unwrap_or_default();
                        if !text.is_empty() && text != old {
                            self.save_note(text);
                        }
                    }
                    self.selection = None;
                    self.w.sel_pop.popdown();
                }
            },
            Input::HighlightClick(info) => {
                self.selection = Some(info.clone());
                self.show_selection_popover(&info);
            }
            Input::Highlight(color) => {
                if let Some(sel) = self.selection.clone() {
                    let mut new_id = None;
                    {
                        let mut lib = self.shared.lib.borrow_mut();
                        if let Some(b) = lib.get_mut(self.book_id) {
                            if let Some(id) = &sel.highlight {
                                if let Some(h) = b.highlights.iter_mut().find(|h| &h.id == id) {
                                    h.color = color.clone();
                                }
                            } else {
                                let id = uuid::Uuid::new_v4().to_string();
                                new_id = Some(id.clone());
                                b.highlights.push(Highlight {
                                    id,
                                    spine: self.spine_idx,
                                    start: sel.start,
                                    end: sel.end,
                                    color: color.clone(),
                                    style: self.hl_style.name().to_string(),
                                    note: String::new(),
                                    text: sel.text.clone(),
                                    created: Utc::now(),
                                });
                            }
                        }
                        let _ = lib.save();
                    }
                    if let (Some(id), Some(s)) = (new_id, self.selection.as_mut()) {
                        s.highlight = Some(id);
                    }
                    // The selection stays: the colour row now shows the active colour and
                    // the popover offers note / remove for the new highlight.
                    self.mark_swatch(Some(&color));
                    self.w.sel_remove.set_visible(true);
                    // The row got one button wider: present the popover again, a popover that is
                    // resized in place vanishes on a real compositor (see `show_lookup_page`).
                    self.represent_popover();
                    self.refresh_highlights();
                    self.rebuild_lists();
                }
            }
            Input::HlStyle(style) => {
                self.hl_style = style;
                self.mark_style(style);
                if let Some(id) = self.selection.as_ref().and_then(|s| s.highlight.clone()) {
                    {
                        let mut lib = self.shared.lib.borrow_mut();
                        if let Some(h) = lib
                            .get_mut(self.book_id)
                            .and_then(|b| b.highlights.iter_mut().find(|h| h.id == id))
                        {
                            h.style = style.name().to_string();
                        }
                        let _ = lib.save();
                    }
                    self.refresh_highlights();
                }
            }
            Input::SaveNote(note) => {
                self.save_note(note);
                self.canvas.clear_selection();
            }
            Input::RemoveHighlight => {
                if let Some(id) = self.selection.as_ref().and_then(|s| s.highlight.clone()) {
                    sender.input(Input::DeleteHighlight(id));
                    self.canvas.clear_selection();
                }
            }
            Input::DeleteHighlight(id) => {
                {
                    let mut lib = self.shared.lib.borrow_mut();
                    if let Some(b) = lib.get_mut(self.book_id) {
                        b.highlights.retain(|h| h.id != id);
                    }
                    let _ = lib.save();
                }
                self.refresh_highlights();
                self.rebuild_lists();
            }
            Input::GoHighlight(id) => {
                let target = self.shared.lib.borrow().get(self.book_id).and_then(|b| {
                    b.highlights
                        .iter()
                        .find(|h| h.id == id)
                        .map(|h| (h.spine, h.start))
                });
                if let Some((spine, start)) = target {
                    self.note_jump();
                    eprintln!("goto_highlight: id={id} spine={spine}");
                    if spine != self.spine_idx {
                        self.load_chapter(spine, Some(0.0), None, false);
                    }
                    self.canvas.goto_char(start);
                }
            }
            Input::Copy => {
                if let Some(t) = self
                    .canvas
                    .selected_text()
                    .or_else(|| self.selection.as_ref().map(|s| s.text.clone()))
                {
                    self.canvas.area.clipboard().set_text(&t);
                    let _ = sender.output(Output::Toast(tr("Copied to clipboard")));
                }
            }
            Input::PopClosed => {
                // dismissed (Esc / click away) while writing a note: keep what was typed
                if self.w.sel_stack.visible_child_name().as_deref() == Some("note") {
                    let b = self.w.note_view.buffer();
                    let (st, en) = b.bounds();
                    let text = b.text(&st, &en, false).trim().to_string();
                    if !text.is_empty() && self.selection.is_some() {
                        self.save_note(text);
                    }
                }
                self.w.sel_pop.set_autohide(false);
                self.w.sel_stack.set_visible_child_name("actions");
            }
            Input::Dict => {
                let Some(term) = self
                    .selection
                    .as_ref()
                    .and_then(|s| crate::dictionaries::term_for_selection(&s.text))
                else {
                    return;
                };
                self.show_lookup_page("dict");
                self.fill_dict(&term);
            }
            Input::DictDownload => {
                let lang = self.dict_language();
                self.dict_waiting.set(true);
                let d = super::dict_service::request(&self.shared, &lang, true);
                if d == crate::dict_auto::Decision::Offline {
                    self.show_dict_prompt(&crate::dict_auto::Prompt::Failed {
                        lang,
                        message: String::new(),
                        offline: true,
                    });
                } else if let Some(term) = self
                    .selection
                    .as_ref()
                    .and_then(|s| crate::dictionaries::term_for_selection(&s.text))
                {
                    self.fill_dict(&term);
                }
            }
            Input::DictChoose => {
                self.w.sel_pop.popdown();
                let _ = sender.output(Output::OpenPreferences(tr("Dictionaries")));
            }
            Input::DictLearn => {
                open_uri("https://bookx.adhadse.com/reading/#dictionaries");
            }
            Input::DictEvent(e) => {
                use super::dict_service::Event;
                let lang = self.dict_language();
                let (ev_lang, p) = match &e {
                    Event::Progress {
                        lang: l,
                        fraction,
                        line,
                    } => (
                        l.clone(),
                        crate::dict_auto::Prompt::Working {
                            lang: l.clone(),
                            fraction: *fraction,
                            line: line.clone(),
                        },
                    ),
                    Event::Failed {
                        lang: l,
                        message,
                        offline,
                    } => (
                        l.clone(),
                        crate::dict_auto::Prompt::Failed {
                            lang: l.clone(),
                            message: message.clone(),
                            offline: *offline,
                        },
                    ),
                    Event::Done { lang: l } => (l.clone(), crate::dict_auto::Prompt::Hidden),
                };
                if ev_lang != lang
                    || self.w.sel_stack.visible_child_name().as_deref() != Some("dict")
                {
                    return;
                }
                if matches!(e, Event::Done { .. }) {
                    // the definition of the word that was looked up appears in the same popover
                    self.dict_waiting.set(false);
                    if let Some(term) = self
                        .selection
                        .as_ref()
                        .and_then(|s| crate::dictionaries::term_for_selection(&s.text))
                    {
                        self.fill_dict(&term);
                    }
                } else {
                    self.show_dict_prompt(&p);
                }
            }
            Input::Wiki => {
                if let Some(sel) = &self.selection {
                    let term: String = sel
                        .text
                        .split_whitespace()
                        .take(8)
                        .collect::<Vec<_>>()
                        .join(" ");
                    let lang = self
                        .book
                        .metadata()
                        .language
                        .split('-')
                        .next()
                        .unwrap_or("en")
                        .to_string();
                    let lang = if lang.len() == 2 { lang } else { "en".into() };
                    *self.w.wiki_url.borrow_mut() = format!(
                        "https://{lang}.wikipedia.org/w/index.php?search={}",
                        web::encode(&term)
                    );
                    self.show_lookup_page("wiki");
                    self.w.wiki_text.set_label("");
                    self.w.wiki_img.0.set_visible(false);
                    self.w.wiki_img.2.set_paintable(None::<&gdk::Paintable>);
                    self.w.wiki_spin.set_opacity(1.0);
                    self.w.wiki_spin.start();
                    let gen = self.fetch_gen.get() + 1;
                    self.fetch_gen.set(gen);
                    let (g, spin, label, url, img) = (
                        self.fetch_gen.clone(),
                        self.w.wiki_spin.clone(),
                        self.w.wiki_text.clone(),
                        self.w.wiki_url.clone(),
                        self.w.wiki_img.clone(),
                    );
                    web::wikipedia(&lang, &term, move |res| {
                        if g.get() != gen {
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
                                // the lead picture, when the article has one: a placeholder first
                                if let Some(u) = r.thumb {
                                    let (frame, stack, pic) = img;
                                    frame.set_visible(true);
                                    stack.set_visible_child_name("placeholder");
                                    let g = g.clone();
                                    web::image(&u, move |res| {
                                        if g.get() != gen {
                                            return;
                                        }
                                        match res {
                                            Ok(t) => {
                                                pic.set_paintable(Some(&t));
                                                stack.set_visible_child_name("picture");
                                            }
                                            // no picture: the text stays, the frame goes
                                            Err(_) => frame.set_visible(false),
                                        }
                                    });
                                }
                            }
                            Err(e) => label.set_label(&trf("Couldn't load a summary.\n{}\n\nUse “Open Wikipedia” to search in the browser.", &[&e])),
                        }
                    });
                }
            }
            Input::Translate => {
                if let Some(sel) = &self.selection {
                    let text: String = sel
                        .text
                        .split_whitespace()
                        .take(120)
                        .collect::<Vec<_>>()
                        .join(" ");
                    let cur = self.shared.cfg.borrow().reader.translate_to.clone();
                    let to = web::pick_translate_target(&cur, &self.book.metadata().language);
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
                    self.show_lookup_page("translate");
                    self.w.tr_text.set_label("");
                    self.w.tr_spin.set_opacity(1.0);
                    self.w.tr_spin.start();
                    let gen = self.fetch_gen.get() + 1;
                    self.fetch_gen.set(gen);
                    let (g, spin, label) = (
                        self.fetch_gen.clone(),
                        self.w.tr_spin.clone(),
                        self.w.tr_text.clone(),
                    );
                    web::translate(&to, &text, move |res| {
                        if g.get() != gen {
                            return;
                        }
                        spin.stop();
                        spin.set_opacity(0.0);
                        match res {
                            Ok(t) => label.set_label(&t),
                            Err(e) => {
                                label.set_label(&trf("Couldn't translate this text.\n{}", &[&e]))
                            }
                        }
                    });
                }
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
            Input::Zoom(f) => {
                {
                    let mut c = self.shared.cfg.borrow_mut();
                    c.reader.font_size = (c.reader.font_size * f).clamp(8.0, 40.0).round();
                    c.save();
                }
                self.apply_prefs();
            }
            Input::DoubleTap => sender.input(Input::Fullscreen),
            Input::Properties => self.show_properties(),
            Input::Footer(c) => self.set_footer(c),
            Input::FooterScope(own) => self.set_footer_scope(own),
            Input::FooterTick => {
                self.refresh_live();
                if self
                    .shared
                    .cfg
                    .borrow()
                    .footer_for(self.book_id)
                    .needs_clock()
                {
                    self.update_position_ui();
                }
            }
            Input::FooterChanged => self.update_position_ui(),
            Input::FooterConfigure => self.w.footer_pop.popup(),
            Input::HandlesDone => {
                self.canvas.commit_selection();
            }
            Input::Side(cmd) => {
                use super::reader_side::Cmd;
                match cmd {
                    Cmd::GoHref(h) => sender.input(Input::GoHref(h)),
                    Cmd::GoBookmark(id) => {
                        sender.input(Input::GoBookmark(id));
                        self.close_sidebar_over_page();
                    }
                    Cmd::RemoveBookmark(id) => sender.input(Input::RemoveBookmark(id)),
                    Cmd::GoHighlight(id) => {
                        sender.input(Input::GoHighlight(id));
                        self.close_sidebar_over_page();
                    }
                    Cmd::DeleteHighlight(id) => sender.input(Input::DeleteHighlight(id)),
                    Cmd::Properties => self.show_properties(),
                    Cmd::EditNote(id) => {
                        let note = self
                            .shared
                            .lib
                            .borrow()
                            .get(self.book_id)
                            .and_then(|b| b.highlights.iter().find(|h| h.id == id))
                            .map(|h| h.note.clone())
                            .unwrap_or_default();
                        let tx = sender.input_sender().clone();
                        super::reader_side::edit_note_dialog(&self.w.page, &note, move |t| {
                            tx.emit(Input::SideNote(id.clone(), t))
                        });
                    }
                    Cmd::Recolor(id, color) => {
                        self.edit_highlight(&id, |h| h.color = color);
                    }
                    Cmd::Restyle(id, style) => {
                        self.edit_highlight(&id, |h| h.style = style);
                    }
                }
            }
            Input::SideNote(id, text) => {
                self.edit_highlight(&id, |h| h.note = text.trim().to_string());
            }
            Input::Rsvp if self.rsvp.is_on() => {}
            Input::Rsvp => {
                self.tts_pause();
                let full = self.canvas.canonical_text();
                let from = crate::rsvp_view::start_offset(&full, &self.canvas.page_text());
                let wpm = self.shared.cfg.borrow().rsvp_wpm;
                self.rsvp
                    .start(&self.canvas, &full, from, wpm, self.spine_idx);
                self.save_cards(true);
            }
            Input::RsvpCmd(c) => self.rsvp_run(c),
            Input::RsvpTick => {
                let ev = self.rsvp.on_tick(&self.canvas);
                self.rsvp_event(ev);
                self.sync_chrome_playing();
                self.save_cards(false);
            }
            Input::RestoreCards => self.restore_cards(),
            Input::Reload => {
                let _ = sender.output(Output::Reload);
            }
            Input::OpenBook => {
                let _ = sender.output(Output::OpenFile);
            }
            Input::Tts(cmd) => self.tts_command(cmd),
            Input::TtsTick => {
                let updates = match self.tts.engine.as_deref_mut() {
                    Some(e) => self.tts.session.tick(e),
                    None => Vec::new(),
                };
                self.tts_apply(updates);
                self.tts_poll_prep();
            }
            Input::Tweaks(t, per_book) => {
                let before = self.shared.cfg.borrow().tweaks.effective(self.book_id);
                {
                    let mut cfg = self.shared.cfg.borrow_mut();
                    cfg.tweaks.set(self.book_id, per_book, t);
                    cfg.save();
                }
                let after = self.shared.cfg.borrow().tweaks.effective(self.book_id);
                self.apply_prefs();
                if before.css() != after.css() {
                    let frac = self.pos.frac;
                    self.load_chapter(self.spine_idx, Some(frac), None, false);
                }
            }
            Input::Prefs(p) => {
                // Ctrl +/- send sentinel values
                let mut cfg = self.shared.cfg.borrow_mut();
                let was_original = cfg.reader.colors == ColorMode::Original;
                if p.font_size < 0.0 {
                    let d = if p.font_size == -1.0 { 1.0 } else { -1.0 };
                    cfg.reader.font_size = (cfg.reader.font_size + d).clamp(8.0, 40.0);
                } else {
                    cfg.reader = p;
                }
                // every change is the book's own look (new books start from the defaults)
                let look = cfg.reader.clone();
                cfg.book_looks.set(self.book_id, &look);
                cfg.save();
                // the book's colours are only in the parsed chapter when they were asked for
                let reparse = (cfg.reader.colors == ColorMode::Original) != was_original;
                drop(cfg);
                self.apply_prefs();
                if reparse {
                    let frac = self.pos.frac;
                    self.load_chapter(self.spine_idx, Some(frac), None, false);
                }
            }
            Input::ToggleBookmark => {
                {
                    let mut lib = self.shared.lib.borrow_mut();
                    if let Some(b) = lib.get_mut(self.book_id) {
                        let near = b.bookmarks.iter().position(|m| {
                            m.spine == self.spine_idx && (m.frac - self.pos.frac).abs() < 0.04
                        });
                        match near {
                            Some(i) => {
                                b.bookmarks.remove(i);
                            }
                            None => b.bookmarks.push(Bookmark {
                                id: uuid::Uuid::new_v4().to_string(),
                                spine: self.spine_idx,
                                frac: self.pos.frac,
                                label: format!("Page {}", self.pos.page),
                                created: Utc::now(),
                            }),
                        }
                    }
                    let _ = lib.save();
                }
                self.update_bookmark_button();
                self.rebuild_lists();
            }
            Input::GoBookmark(id) => {
                let target = self.shared.lib.borrow().get(self.book_id).and_then(|b| {
                    b.bookmarks
                        .iter()
                        .find(|m| m.id == id)
                        .map(|m| (m.spine, m.frac))
                });
                if let Some((s, f)) = target {
                    self.note_jump();
                    self.load_chapter(s, Some(f), None, false);
                }
            }
            Input::RemoveBookmark(id) => {
                {
                    let mut lib = self.shared.lib.borrow_mut();
                    if let Some(b) = lib.get_mut(self.book_id) {
                        b.bookmarks.retain(|m| m.id != id);
                    }
                    let _ = lib.save();
                }
                self.rebuild_lists();
                self.update_bookmark_button();
            }
            Input::ShowTab(t) => {
                if t == "toc" {
                    self.canvas.area.grab_focus();
                    return;
                }
                if t == "contents" {
                    // F9 / Ctrl+T toggle the sidebar
                    if self.w.split.shows_sidebar() {
                        self.w.split.set_show_sidebar(false);
                        self.canvas.area.grab_focus();
                    } else {
                        self.w.split.set_show_sidebar(true);
                        self.w.stack.set_visible_child_name("toc");
                    }
                    return;
                }
                if t == "search" {
                    self.w.search_btn.popup();
                    self.w.search_entry.grab_focus();
                    return;
                }
                self.w.split.set_show_sidebar(true);
                self.w.stack.set_visible_child_name(t);
            }
            Input::Search(q) => {
                let q = q.trim().to_string();
                self.last_search = q.clone();
                self.search_hits.clear();
                self.search_active = None;
                while let Some(c) = self.w.search_list.first_child() {
                    self.w.search_list.remove(&c);
                }
                self.canvas.set_hits(Vec::new(), None);
                if q.chars().count() < 2 {
                    self.w.search_status.set_label("");
                    return;
                }
                self.w.search_status.set_label(&tr("Searching…"));
                let hits_now = self.canvas.find_all(&q);
                self.canvas.set_hits(hits_now, None);
                let gen = self.search_gen.get() + 1;
                self.search_gen.set(gen);
                // Parse chapters incrementally so the UI stays responsive.
                let book = self.book.clone();
                let spine = self.spine.clone();
                let gen_cell = self.search_gen.clone();
                let tx = sender.input_sender().clone();
                let ql = q.to_lowercase();
                let total = Rc::new(Cell::new(0usize));
                let next = Rc::new(Cell::new(0usize));
                glib::idle_add_local(move || {
                    if gen_cell.get() != gen {
                        return glib::ControlFlow::Break;
                    }
                    let i = next.get();
                    if i >= spine.len() || total.get() > 400 {
                        tx.emit(Input::SearchDone(gen, Vec::new(), true));
                        return glib::ControlFlow::Break;
                    }
                    next.set(i + 1);
                    let mut batch: Vec<(usize, usize, String)> = Vec::new();
                    if let Some(doc) = book.chapter_doc(&spine[i]) {
                        let mut nth = 0;
                        for b in &doc.blocks {
                            let text = b.plain_text();
                            let lower = text.to_lowercase();
                            let mut from = 0;
                            while let Some(pos) = lower[from..].find(&ql) {
                                let at = from + pos;
                                let start = lower[..at]
                                    .char_indices()
                                    .rev()
                                    .nth(30)
                                    .map(|(i, _)| i)
                                    .unwrap_or(0);
                                let end = lower[at..]
                                    .char_indices()
                                    .nth(ql.chars().count() + 40)
                                    .map(|(i, _)| at + i)
                                    .unwrap_or(lower.len());
                                let snippet =
                                    text.get(start..end).unwrap_or(&text).replace('\n', " ");
                                batch.push((i, nth, snippet));
                                nth += 1;
                                from = at + ql.len().max(1);
                                if from >= lower.len() {
                                    break;
                                }
                            }
                        }
                    }
                    if !batch.is_empty() {
                        total.set(total.get() + batch.len());
                        tx.emit(Input::SearchDone(gen, batch, false));
                    }
                    glib::ControlFlow::Continue
                });
            }
            Input::SearchDone(gen, results, finished) => {
                if gen != self.search_gen.get() {
                    return;
                }
                for (spine, nth, snippet) in &results {
                    let row = adw::ActionRow::builder()
                        .title(glib::markup_escape_text(&format!("…{}…", snippet.trim())))
                        .subtitle(glib::markup_escape_text(&self.chapter_title(*spine)))
                        .title_lines(2)
                        .activatable(true)
                        .build();
                    self.w.search_list.append(&row);
                    self.search_hits.push((*spine, *nth));
                }
                let n = self.search_hits.len();
                self.w.search_status.set_label(&if finished && n == 0 {
                    "No results".to_string()
                } else if n > 400 {
                    "More than 400 results".to_string()
                } else if finished {
                    format!("{n} results")
                } else {
                    format!("{n} results so far…")
                });
            }
            Input::SearchResult(i) => {
                if let Some((spine, nth)) = self.search_hits.get(i).copied() {
                    self.note_jump_burst();
                    self.search_active = Some(i);
                    self.w
                        .search_list
                        .select_row(self.w.search_list.row_at_index(i as i32).as_ref());
                    if spine == self.spine_idx {
                        let hits = self.canvas.find_all(&self.last_search);
                        let nth = nth.min(hits.len().saturating_sub(1));
                        self.canvas.set_hits(hits, Some(nth));
                    } else {
                        self.pending_hit = Some((self.last_search.clone(), nth));
                        self.load_chapter(spine, Some(0.0), None, false);
                    }
                }
            }
            Input::SearchStep(fwd) => {
                if self.search_hits.is_empty() {
                    return;
                }
                let n = self.search_hits.len();
                let cur = self.search_active.unwrap_or(if fwd { n - 1 } else { 0 });
                let next = if fwd {
                    (cur + 1) % n
                } else {
                    (cur + n - 1) % n
                };
                self.w
                    .search_list
                    .select_row(self.w.search_list.row_at_index(next as i32).as_ref());
                sender.input(Input::SearchResult(next));
            }
            Input::Fullscreen => {
                if let Some(win) = window_of(&self.w.page) {
                    self.fullscreen = !win.is_fullscreen();
                    win.set_fullscreened(self.fullscreen);
                    self.w.fullscreen_btn.set_icon_name(if self.fullscreen {
                        "view-restore-symbolic"
                    } else {
                        "view-fullscreen-symbolic"
                    });
                }
            }
            Input::Tap => {
                if self.w.split.shows_sidebar() && self.w.split.is_collapsed() {
                    self.w.split.set_show_sidebar(false);
                }
            }
            Input::Dev(c) => {
                let mut p = self.shared.cfg.borrow().reader.clone();
                match c.as_str() {
                    "two" => p.layout = LayoutMode::TwoColumns,
                    "single" => p.layout = LayoutMode::SingleColumn,
                    "scroll" => p.layout = LayoutMode::Scroll,
                    "dark" => p.theme = ReaderTheme::Dark,
                    "sepia" => p.theme = ReaderTheme::Sepia,
                    "light" => p.theme = ReaderTheme::Light,
                    "next" => {
                        self.canvas.turn(true);
                        return;
                    }
                    "rsvp" => {
                        sender.input(Input::Rsvp);
                        return;
                    }
                    c if c.starts_with("side-")
                        || c.starts_with("footer")
                        || c.starts_with("handle")
                        || c == "demo-highlights"
                        || c == "properties"
                        || c.starts_with("bulk-highlights=") =>
                    {
                        self.dev_side(c, &sender);
                        return;
                    }
                    r if r.starts_with("rsvp-") => {
                        use super::rsvp::Cmd;
                        let step = crate::rsvp_view::STEP_WPM as i32;
                        let cmd = match &r[5..] {
                            "play" | "pause" => Some(Cmd::PlayPause),
                            "faster" => Some(Cmd::Speed(step)),
                            "slower" => Some(Cmd::Speed(-step)),
                            "back" => Some(Cmd::SentenceBack),
                            "next" => Some(Cmd::SentenceForward),
                            "close" => Some(Cmd::Close),
                            c => c
                                .strip_prefix("step=")
                                .map(|n| Cmd::Step(n.parse().unwrap_or(0))),
                        };
                        let skip = match &r[5..] {
                            "play" => self.rsvp.playing(),
                            "pause" => !self.rsvp.playing(),
                            _ => false,
                        };
                        if let (Some(c), false) = (cmd, skip) {
                            self.rsvp_run(c);
                        }
                        if let Some(xy) = r[5..].strip_prefix("drag:") {
                            let v: Vec<f64> =
                                xy.split(':').filter_map(|n| n.parse().ok()).collect();
                            if let [x, y] = v[..] {
                                self.rsvp.set_pos((x, y));
                                super::card_drag::raise(self.rsvp.widget.upcast_ref(), 1);
                                self.save_cards(true);
                            }
                        }
                        if let Some(k) = r[5..].strip_prefix("key:") {
                            let key = match k {
                                "space" => gdk::Key::space,
                                "right" => gdk::Key::Right,
                                "left" => gdk::Key::Left,
                                "up" => gdk::Key::Up,
                                "down" => gdk::Key::Down,
                                _ => gdk::Key::Escape,
                            };
                            if let Some(c) = super::rsvp::key_cmd(key) {
                                self.rsvp_run(c);
                            }
                        }
                        eprintln!(
                            "rsvp: {} page_start={}",
                            self.rsvp.dump(&self.canvas),
                            self.canvas.visible_start_char()
                        );
                        return;
                    }
                    "footnotes-off" => {
                        self.shared.cfg.borrow_mut().footnote_popups = false;
                        return;
                    }
                    l if l.starts_with("link=") => {
                        sender.input(Input::Link(l[5..].to_string()));
                        return;
                    }
                    g if g.starts_with("goto=") => {
                        sender.input(Input::GoSpine(g[5..].parse().unwrap_or(0), Some(0.0), None));
                        return;
                    }
                    "chapter" => {
                        sender.input(Input::Chapter(true));
                        return;
                    }
                    "toc" => {
                        self.w.split.set_show_sidebar(true);
                        return;
                    }
                    "notes" => {
                        self.w.split.set_show_sidebar(true);
                        self.w.stack.set_visible_child_name("notes");
                        return;
                    }
                    "hl" => {
                        sender.input(Input::Highlight("yellow".into()));
                        return;
                    }
                    // drives the real buttons: Note → type → Save Note
                    "ui-note" => {
                        self.w.note_btn.emit_clicked();
                        eprintln!("ui-note: page={:?}", self.w.sel_stack.visible_child_name());
                        eprintln!("ui-note: autohide={}", self.w.sel_pop.is_autohide());
                        return;
                    }
                    // typed and saved after the popover was re-opened (`repopup` takes a moment)
                    "ui-note-save" => {
                        self.w
                            .note_view
                            .buffer()
                            .set_text(&tr("typed through the UI"));
                        self.w.note_save.emit_clicked();
                        return;
                    }
                    "note" => {
                        sender.input(Input::SaveNote("Great opening — revisit this idea.".into()));
                        return;
                    }
                    "wiki" => {
                        self.canvas.dev_select_first_words(1);
                        sender.input(Input::Wiki);
                        return;
                    }
                    "tr" => {
                        self.canvas.dev_select_first_words(3);
                        sender.input(Input::Translate);
                        return;
                    }
                    "dump" => {
                        eprintln!("footnote_popup: {}", super::reader_extra::dev_dump());
                        eprintln!("wiki: {}", self.w.wiki_text.label());
                        {
                            let p = self.dict_prompt_state.borrow();
                            eprintln!(
                                "dict_prompt: state={} lang={} title={:?} primary={:?} enabled={} progress={:?} visible={}",
                                p.key(),
                                self.dict_language(),
                                p.heading(),
                                p.primary(),
                                p.primary_enabled(),
                                p.progress(),
                                self.w.dict_prompt.root.is_visible()
                            );
                            eprintln!(
                                "dict_service: {}",
                                super::dict_service::dev_dump(&self.shared)
                            );
                        }
                        eprintln!(
                            "wiki_image: frame={} shows={} size={}",
                            self.w.wiki_img.0.is_visible(),
                            self.w.wiki_img.1.visible_child_name().unwrap_or_default(),
                            self.w
                                .wiki_img
                                .2
                                .paintable()
                                .map_or("none".to_string(), |p| format!(
                                    "{}x{}",
                                    p.intrinsic_width(),
                                    p.intrinsic_height()
                                ))
                        );
                        eprintln!("tr: {}", self.w.tr_text.label());
                        eprintln!(
                            "dict: {} | {}",
                            self.w.sel_stack.visible_child_name().unwrap_or_default(),
                            self.w.dict_text.label().replace('\n', " / ")
                        );
                        eprintln!(
                            "labels: {} | {} | {}",
                            // the one centred line is shown where the left part was
                            if self.w.left_label.label().is_empty() {
                                self.w.mid_label.label()
                            } else {
                                self.w.left_label.label()
                            },
                            if self.w.left_label.label().is_empty() {
                                "".into()
                            } else {
                                self.w.mid_label.label()
                            },
                            self.w.right_label.label()
                        );
                        eprintln!("idle: {}", self.w.page.has_css_class("chrome-idle"));
                        eprintln!("search_status: {}", self.w.search_status.label());
                        eprintln!("search_query: {}", self.w.search_entry.text());
                        eprintln!("autohide: {}", self.w.sel_pop.is_autohide());
                        eprintln!("popover: {}", self.w.sel_pop.is_visible());
                        let (shy, canon) = self.canvas.dev_hyphen_stats();
                        eprintln!("hyphens: {shy} {canon}");
                        let (kp, kl, kf, kh) = self.canvas.dev_break_stats();
                        eprintln!("breaks: paragraphs={kp} lines={kl} full={kf} hyphens={kh}");
                        let (caps, floats, tails) = self.canvas.dev_wrap_stats();
                        eprintln!("wrap: caps={caps} floats={floats} tails={tails}");
                        let (rtl, rkp, rright) = self.canvas.dev_rtl_stats();
                        eprintln!("rtl: paragraphs={rtl} optimal={rkp} right_aligned={rright}");
                        let (fx, fp, fw, fh, ft, fc) = self.canvas.dev_fixed_stats();
                        eprintln!(
                            "fixed: on={fx} pages={fp} image={fw}x{fh} top={ft} area={fc} spine={}",
                            self.spine_idx
                        );
                        eprintln!("palette: {}", self.canvas.dev_palette());
                        let (it, bd, ce) = self.canvas.dev_style_counts();
                        eprintln!("styled: italic={it} bold={bd} centered={ce}");
                        let (ind, all) = self.canvas.dev_indent_counts();
                        eprintln!("indented: {ind} of {all}");
                        eprintln!("colored: {}", self.canvas.dev_colored_items());
                        eprintln!("selected: {:?}", self.canvas.selected_text());
                        eprintln!(
                            "hl_styles: {:?} next={}",
                            self.shared
                                .lib
                                .borrow()
                                .get(self.book_id)
                                .map(|b| b
                                    .highlights
                                    .iter()
                                    .map(|h| h.style.clone())
                                    .collect::<Vec<_>>())
                                .unwrap_or_default(),
                            self.hl_style.name()
                        );
                        eprintln!(
                            "style_buttons: {:?}",
                            self.w
                                .style_btns
                                .iter()
                                .filter(|(_, b)| b.is_active())
                                .map(|(s, _)| s.name())
                                .collect::<Vec<_>>()
                        );
                        eprintln!("hover_note: {:?}", self.canvas.hover_text());
                        eprintln!(
                            "popover_note: {} {:?}",
                            self.w.note_preview.is_visible(),
                            self.w.note_label.text()
                        );
                        eprintln!(
                            "bookmarks_notes: {}",
                            self.shared
                                .lib
                                .borrow()
                                .get(self.book_id)
                                .map(|b| b.highlights.len())
                                .unwrap_or(0)
                        );
                        return;
                    }
                    "search-nopop" => {
                        sender.input(Input::Search("the".into()));
                        return;
                    }
                    "search-popup-only" => {
                        self.w.search_btn.popup();
                        return;
                    }
                    // a highlight popover (no canvas selection behind it), then a click on the page
                    "ui-hl-popover" => {
                        let first = self
                            .shared
                            .lib
                            .borrow()
                            .get(self.book_id)
                            .and_then(|b| b.highlights.first().cloned());
                        if let Some(h) = first {
                            sender.input(Input::HighlightClick(SelectionInfo {
                                text: h.text.clone(),
                                start: h.start,
                                end: h.end,
                                rect: (200, 200, 20, 20),
                                highlight: Some(h.id),
                            }));
                        }
                        return;
                    }
                    // the popover of the first highlight that has a note, at the highlight's real place
                    "ui-note-popover" => {
                        if let Some(info) = self.canvas.dev_note_highlight_info() {
                            sender.input(Input::HighlightClick(info));
                        }
                        return;
                    }
                    "ui-click-away" => {
                        self.canvas.dev_click_middle();
                        return;
                    }
                    "ui-style-underline" | "ui-style-squiggly" | "ui-style-strike"
                    | "ui-style-bg" => {
                        let st = match c.as_str() {
                            "ui-style-underline" => HighlightStyle::Underline,
                            "ui-style-squiggly" => HighlightStyle::Squiggly,
                            "ui-style-strike" => HighlightStyle::Strikethrough,
                            _ => HighlightStyle::Background,
                        };
                        if let Some((_, b)) = self.w.style_btns.iter().find(|(s, _)| *s == st) {
                            b.emit_clicked();
                        }
                        return;
                    }
                    "ui-dict" => {
                        sender.input(Input::Dict);
                        return;
                    }
                    "ui-dict-btn" => {
                        self.w.dict_btn.emit_clicked();
                        return;
                    }
                    "ui-dict-prompt-btn" => {
                        self.w.dict_prompt.primary.emit_clicked();
                        return;
                    }
                    "ui-wiki-btn" => {
                        self.w.wiki_btn.emit_clicked();
                        return;
                    }
                    other if other.starts_with("ui-swipe:") => {
                        let v: Vec<f64> = other[9..]
                            .split(':')
                            .filter_map(|x| x.parse().ok())
                            .collect();
                        if let [dx, dy, ms] = v[..] {
                            eprintln!("swipe: {:?}", self.canvas.dev_swipe(dx, dy, ms as u64));
                        }
                        return;
                    }
                    other if other.starts_with("ui-tap:") => {
                        if let Ok(x) = other[7..].parse::<f64>() {
                            eprintln!("tap: {:?}", self.canvas.dev_tap(x));
                        }
                        return;
                    }
                    other if other.starts_with("ui-zones:") => {
                        let z = crate::gestures::TapZones::ALL
                            .iter()
                            .find(|z| format!("{z:?}").eq_ignore_ascii_case(&other[9..]))
                            .copied();
                        if let Some(z) = z {
                            self.shared.cfg.borrow_mut().reader.tap_zones = z;
                            self.apply_prefs();
                        }
                        return;
                    }
                    "ui-colors-theme" | "ui-colors-original" | "ui-colors-custom" => {
                        let mut p = self.shared.cfg.borrow().reader.clone();
                        p.colors = match c.as_str() {
                            "ui-colors-original" => ColorMode::Original,
                            "ui-colors-custom" => {
                                p.custom_fg = "#ffcc00".into();
                                p.custom_bg = "#101010".into();
                                ColorMode::Custom
                            }
                            _ => ColorMode::Theme,
                        };
                        sender.input(Input::Prefs(p));
                        return;
                    }
                    "ui-styles-off" | "ui-styles-on" => {
                        self.shared.cfg.borrow_mut().reader.publisher_styles = c == "ui-styles-on";
                        let frac = self.pos.frac;
                        self.load_chapter(self.spine_idx, Some(frac), None, false);
                        return;
                    }
                    "ui-hyphen-off" | "ui-hyphen-on" => {
                        self.shared.cfg.borrow_mut().reader.hyphenate = c == "ui-hyphen-on";
                        self.apply_prefs();
                        return;
                    }
                    "ui-optimal-off" | "ui-optimal-on" => {
                        self.shared.cfg.borrow_mut().reader.optimal_breaks = c == "ui-optimal-on";
                        self.apply_prefs();
                        return;
                    }
                    "ui-bench" => {
                        let (layout, turns) = self.canvas.dev_bench();
                        eprintln!("bench: layout_ms={layout:.0} turns_ms={turns:.0}");
                        return;
                    }
                    "ui-selectall" => {
                        self.canvas.select_all();
                        let t = self.canvas.selected_text().unwrap_or_default();
                        eprintln!("all: {}", t.replace('\n', "/"));
                        return;
                    }
                    "ui-vertical" => {
                        eprintln!("vertical: {}", self.canvas.dev_vertical_stats());
                        let lines = self.canvas.dev_page_lines();
                        eprintln!("vlines: {}", lines.join("|"));
                        for n in [0, 5, 12] {
                            eprintln!("vprobe: {n} {:?}", self.canvas.dev_probe_char(n));
                        }
                        return;
                    }
                    other if other.starts_with("ui-vsel:") => {
                        let mut it = other[8..].split(':').filter_map(|v| v.parse::<i32>().ok());
                        if let (Some(a), Some(b)) = (it.next(), it.next()) {
                            eprintln!("vsel: {}", self.canvas.dev_drag_select(a, b));
                        }
                        return;
                    }
                    other if other.starts_with("ui-shot:") => {
                        eprintln!("shot: {}", self.canvas.dev_screenshot(&other[8..]));
                        return;
                    }
                    other if other.starts_with("ui-tweak:") => {
                        if let Some((k, v)) = other[9..].split_once('=') {
                            if let Some(ui) = self.tweaks_ui.borrow().as_ref() {
                                ui.dev_set(k, v);
                            }
                        }
                        return;
                    }
                    "ui-tweaks-state" => {
                        let cfg = self.shared.cfg.borrow();
                        eprintln!(
                            "tweaks: general={:?} book={:?}",
                            cfg.tweaks.general,
                            cfg.tweaks.books.get(&self.book_id.to_string())
                        );
                        let (m, ls, para) = self.canvas.dev_style();
                        eprintln!("tweaked: margin={m} line_spacing={ls} paragraph={para:?}");
                        return;
                    }
                    "tts-play" | "tts-pause" | "tts-stop" => {
                        let cmd = if c == "tts-stop" {
                            super::reader_tts::Cmd::Stop
                        } else {
                            super::reader_tts::Cmd::PlayPause
                        };
                        let playing = self.tts.session.state() == crate::tts::State::Playing;
                        // play starts or resumes, pause only acts while playing
                        if c == "tts-pause" && !playing {
                            return;
                        }
                        if c == "tts-play" && playing {
                            return;
                        }
                        self.tts_command(cmd);
                        return;
                    }
                    k if k.starts_with("goto-text=") => {
                        let r = self.goto_text(&k[10..]);
                        eprintln!("reader_goto: {} -> {:?}", &k[10..], r);
                        return;
                    }
                    "back" => {
                        sender.input(Input::Back);
                        return;
                    }
                    "forward" => {
                        sender.input(Input::Forward);
                        return;
                    }
                    "image-click" => {
                        match self.canvas.dev_first_image() {
                            Some((x, y, src)) => {
                                eprintln!("reader_image: click {src} at {x:.0},{y:.0}");
                                self.canvas.dev_click(x, y);
                            }
                            None => eprintln!("reader_image: none on screen"),
                        }
                        return;
                    }
                    k if k.starts_with("image-viewer") => {
                        let cmd = k.trim_start_matches("image-viewer").trim_start_matches('=');
                        eprintln!("image_viewer: {}", super::image_viewer::dev(cmd));
                        return;
                    }
                    "cur" => {
                        let (c, n) = self.canvas.current_index();
                        eprintln!("reader_cur: spine={} view={c}/{n}", self.spine_idx);
                        return;
                    }
                    "use-default" => {
                        sender.input(Input::UseAsDefault);
                        return;
                    }
                    "reset-look" => {
                        sender.input(Input::ResetLook);
                        return;
                    }
                    "theme-row" => {
                        eprintln!("reader_theme_row: {}", self.w.themes_ui.dump());
                        return;
                    }
                    k if k.starts_with("theme-edit=") => {
                        sender.input(Input::ThemeEditor(Some(k[11..].to_string())));
                        return;
                    }
                    k if k.starts_with("profile=") => {
                        let out = super::reader_theme_editor::dev(&self.profile_ctx(), &k[8..]);
                        eprintln!("reader_profile: {out}");
                        return;
                    }
                    "map" => {
                        self.book_map();
                        return;
                    }
                    k if k.starts_with("hl-add=") => {
                        // hl-add=SPINE|START|COLOUR|STYLE|NOTE: a highlight as if it had been made
                        let f: Vec<&str> = k[7..].split('|').collect();
                        let spine = f.first().and_then(|v| v.parse().ok()).unwrap_or(0);
                        let start = f.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                        {
                            let mut lib = self.shared.lib.borrow_mut();
                            if let Some(b) = lib.get_mut(self.book_id) {
                                b.highlights.push(Highlight {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    spine,
                                    start,
                                    end: start + 10,
                                    color: f.get(2).unwrap_or(&"yellow").to_string(),
                                    style: f.get(3).unwrap_or(&"background").to_string(),
                                    note: f.get(4).unwrap_or(&"").to_string(),
                                    text: "sample".into(),
                                    created: chrono::Utc::now(),
                                });
                            }
                        }
                        let _ = self.shared.lib.borrow().save();
                        eprintln!("reader_hl: added");
                        return;
                    }
                    k if k.starts_with("dwell=") => {
                        // dwell=FRACTION|SECONDS: adds reading time as if it had been spent there
                        let (f, secs) = k[6..].split_once('|').unwrap_or((&k[6..], "0"));
                        crate::reading_dwell::record(
                            self.book_id,
                            f.parse().unwrap_or(0.0),
                            secs.parse().unwrap_or(0.0),
                        );
                        eprintln!("reader_dwell: ok");
                        return;
                    }
                    k if k.starts_with("ruler=") => {
                        eprintln!("ruler: {}", super::reading_ruler::dev_reader(&k[6..]));
                        return;
                    }
                    k if k.starts_with("map-") => {
                        eprintln!("book_map: {}", super::reader_map::dev(&k[4..]));
                        return;
                    }
                    "goto-dialog" => {
                        self.goto_dialog();
                        return;
                    }
                    "jumps" => {
                        let (b, f) = self.jumps.depth();
                        eprintln!(
                            "reader_jumps: back={b} forward={f} spine={} frac={:.2} back_btn={} forward_btn={}",
                            self.spine_idx,
                            self.pos.frac,
                            self.w.back_btn.is_visible(),
                            self.w.forward_btn.is_visible()
                        );
                        return;
                    }
                    k if k.starts_with("anchor=") => {
                        eprintln!(
                            "reader_anchor: {} -> {:?}",
                            &k[7..],
                            self.canvas.anchor_fraction(&k[7..])
                        );
                        return;
                    }
                    "pagemap" => {
                        let (page, total) = self.book_pages();
                        eprintln!(
                            "reader_pagemap: printed={} pages={} footer={:?} layout={page}/{total} pos={}/{}@{:.3} locations={}",
                            self.page_map.is_some(),
                            self.page_map.as_ref().map_or(0, |m| m.total_pages()),
                            self.footer_facts().page_text,
                            self.pos.page,
                            self.pos.pages,
                            self.pos.frac,
                            crate::goto::locations(&self.goto_book())
                        );
                        return;
                    }
                    k if k.starts_with("press=") => {
                        // a key as the reader's key handler gets it, `press=<Control>t`
                        let accel = &k[6..];
                        let said = gtk::accelerator_parse(accel)
                            .and_then(|(key, mods)| {
                                super::shortcuts_dialog::handle_reader_default(
                                    self.w.page.upcast_ref(),
                                    key,
                                    mods,
                                )
                            })
                            .unwrap_or_else(|| "none".into());
                        eprintln!("reader_press: {accel} -> {said}");
                        return;
                    }
                    k if k.starts_with("key=") => {
                        // a key press as the user's keyboard shortcuts see it: `key=<Control>r`
                        let accel = &k[4..];
                        let said = gtk::accelerator_parse(accel)
                            .and_then(|(key, mods)| {
                                super::shortcuts_dialog::handle_reader_key(
                                    self.w.page.upcast_ref(),
                                    key,
                                    mods,
                                )
                            })
                            .or_else(|| {
                                crate::shortcuts::current()
                                    .default_reader_action(accel)
                                    .map(|a| format!("default {a}"))
                            })
                            .unwrap_or_else(|| "builtin".into());
                        eprintln!("reader_key: {accel} -> {said}");
                        return;
                    }
                    "nav-layout" => {
                        let w = &self.w;
                        let dx = w
                            .next_btn
                            .compute_point(&w.prev_btn, &gtk::graphene::Point::new(0.0, 0.0))
                            .map_or(0.0, |p| p.x());
                        eprintln!(
                            "nav_layout: next_left_of_prev={} sidebar_end={} rtl={}",
                            dx < 0.0,
                            w.split.sidebar_position() == gtk::PackType::End,
                            self.book.is_rtl()
                        );
                        return;
                    }
                    // `tts-save:chapter:<file.wav>` / `tts-save:book:<folder>`: like Save as Audio…
                    // without the file chooser
                    other if other.starts_with("tts-save:") => {
                        if let Some((kind, path)) = other["tts-save:".len()..].split_once(':') {
                            let whole = kind == "book";
                            let chapters = self.audio_chapters(whole);
                            let tx = self.sender.output_sender().clone();
                            super::reader_tts::save_audio(
                                self.w.page.upcast_ref(),
                                &self.shared,
                                whole,
                                "Book",
                                chapters,
                                self.tts_language(),
                                Rc::new(move |m| {
                                    let _ = tx.send(Output::Toast(m));
                                }),
                                Some(std::path::PathBuf::from(path)),
                            );
                        }
                        return;
                    }
                    "tts-open" => {
                        self.w.tts.button.popup();
                        return;
                    }
                    "tts-close" => {
                        self.w.tts.button.popdown();
                        return;
                    }
                    other if other.starts_with("tts-download:") => {
                        let always = &other["tts-download:".len()..] == "always";
                        self.tts_command(super::reader_tts::Cmd::DownloadVoice(always));
                        return;
                    }
                    "tts-player" => {
                        eprintln!(
                            "tts_player: {} offered={} mpris={:?}",
                            self.w.tts.player.dev_dump(),
                            self.tts.offered,
                            self.tts.mpris.as_ref().map(|m| m.name.clone())
                        );
                        return;
                    }
                    other if other.starts_with("tts-player-click:") => {
                        self.w
                            .tts
                            .player
                            .dev_click(&other["tts-player-click:".len()..]);
                        return;
                    }
                    "work-watch" => {
                        // prints the draw and layout counters of the page every 100 ms (real-session tests)
                        let canvas = self.canvas.clone();
                        let (probe, ctl, page) = (
                            self.w.chrome_probe.clone(),
                            self.w.chrome.clone(),
                            self.w.page.clone(),
                        );
                        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                            eprintln!(
                                "chrome_watch: faded={} busy={} playing={} idle_ms={} y={}",
                                page.has_css_class("chrome-idle"),
                                probe(),
                                ctl.playing.get(),
                                ctl.idle_ms(),
                                ctl.pointer_y.get()
                            );
                            let (d, l) = canvas.dev_work();
                            let t = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map_or(0, |t| t.as_millis());
                            eprintln!(
                                "work: t={t} draws={d} layouts={l} frac={:.4}",
                                canvas.fraction()
                            );
                            glib::ControlFlow::Continue
                        });
                        return;
                    }
                    other if other.starts_with("tts-player-drag:") => {
                        let v: Vec<f64> = other["tts-player-drag:".len()..]
                            .split(':')
                            .filter_map(|n| n.parse().ok())
                            .collect();
                        if let [x, y] = v[..] {
                            self.w.tts.player.set_pos((x, y));
                            super::card_drag::raise(self.w.tts.player.widget.upcast_ref(), 0);
                            self.save_cards(true);
                        }
                        return;
                    }
                    other if other.starts_with("tts-player-seek:") => {
                        self.w
                            .tts
                            .player
                            .dev_seek(other["tts-player-seek:".len()..].parse().unwrap_or(0.0));
                        return;
                    }
                    "chrome" => {
                        eprintln!(
                            "chrome: idle={} busy={} tts_popover={} playing={} idle_ms={}",
                            self.w.page.has_css_class("chrome-idle"),
                            (self.w.chrome_probe)(),
                            self.w.tts.button.popover().is_some_and(|p| p.is_visible()),
                            self.w.chrome.playing.get(),
                            self.w.chrome.idle_ms()
                        );
                        return;
                    }
                    // let time pass for the reader focus without waiting: `chrome-clock:<ms>`; an
                    // interaction: `chrome-touch`; the pointer at a height of the window: `chrome-pointer:<y>`
                    other if other.starts_with("chrome-clock:") => {
                        self.w
                            .chrome
                            .advance(other["chrome-clock:".len()..].parse().unwrap_or(0));
                        (self.w.chrome_eval)();
                        return;
                    }
                    "cards-rects" => {
                        eprintln!("cards: {}", super::card_drag::dump());
                        return;
                    }
                    "chrome-touch" => {
                        (self.w.chrome_wake)();
                        return;
                    }
                    other if other.starts_with("chrome-pointer:") => {
                        self.w
                            .chrome
                            .pointer_y
                            .set(other["chrome-pointer:".len()..].parse().unwrap_or(-1.0));
                        (self.w.chrome_wake)();
                        return;
                    }
                    "tts-voices" => {
                        self.tts_load_voices();
                        eprintln!("tts_voices: {:?}", self.w.tts.dev_voices());
                        return;
                    }
                    other if other.starts_with("tts-pitch:") => {
                        self.w.tts.dev_set_pitch(other[10..].parse().unwrap_or(1.0));
                        return;
                    }
                    other if other.starts_with("tts-sleep:") => {
                        let n = match &other[10..] {
                            "chapter" => 5,
                            m => (m.parse::<u32>().unwrap_or(0) / 15).min(4),
                        };
                        self.w.tts.dev_pick_sleep(n);
                        return;
                    }
                    other if other.starts_with("tts-skip:") => {
                        self.tts_command(super::reader_tts::Cmd::Skip(
                            other[9..].parse().unwrap_or(1),
                        ));
                        return;
                    }
                    other if other.starts_with("tts-speed:") => {
                        self.w.tts.dev_set_speed(other[10..].parse().unwrap_or(1.0));
                        return;
                    }
                    other if other.starts_with("tts-voice:") => {
                        self.w.tts.dev_pick_voice(&other[10..]);
                        return;
                    }
                    "tts-dump" => {
                        let (a, b) = self.canvas.speaking().unwrap_or((0, 0));
                        eprintln!(
                            "tts: state={:?} range={:?} text={:?} icon={} prefs={:?} engine=[{}] engine_text={:?}",
                            self.tts.session.state(),
                            self.canvas.speaking(),
                            self.canvas.text_between(a, b),
                            self.w.tts.dev_play_icon(),
                            self.shared.cfg.borrow().tts,
                            self.tts
                                .engine
                                .as_deref()
                                .map(|e| e.debug_state())
                                .unwrap_or_default(),
                            self.w.tts.dev_engine_text()
                        );
                        eprintln!(
                            "tts_chapter: {} sentences={} current={:?} engine_name={}",
                            self.spine_idx,
                            self.tts.session.sentences().len(),
                            self.tts.session.current().map(|s| (s.start, s.end)),
                            self.tts
                                .engine
                                .as_deref()
                                .map(|e| e.name())
                                .unwrap_or_default()
                        );
                        return;
                    }
                    "ui-dblclick" => {
                        self.canvas.dev_double_click_first_word();
                        return;
                    }
                    "ui-hover" => {
                        self.canvas.dev_hover_first_note();
                        return;
                    }
                    "ui-tooltip" => {
                        eprintln!("tooltip: {:?}", self.canvas.dev_first_note_tooltip());
                        return;
                    }
                    "search-close" => {
                        self.w.search_btn.popdown();
                        return;
                    }
                    "bookmark" => {
                        sender.input(Input::ToggleBookmark);
                        return;
                    }
                    "select" => {
                        self.canvas.dev_select_first_words(6);
                        return;
                    }
                    "page-text" => {
                        eprintln!("page_text: {:?}", self.canvas.page_text());
                        eprintln!("book_rtl: {}", self.book.is_rtl());
                        return;
                    }
                    "restyle" => {}
                    // Shift+arrows without a keyboard: ksel=char+ | char- | word+ | word- | sent+ | sent-
                    k if k.starts_with("ksel=") => {
                        use crate::a11y::Step;
                        let (step, fwd) = match &k[5..] {
                            "char+" => (Step::Char, true),
                            "char-" => (Step::Char, false),
                            "word+" => (Step::Word, true),
                            "word-" => (Step::Word, false),
                            "sent+" => (Step::Sentence, true),
                            _ => (Step::Sentence, false),
                        };
                        sender.input(Input::KeySelect(step, fwd));
                        return;
                    }
                    k if k.starts_with("selkey=") => {
                        let a: &'static str = match &k[7..] {
                            "commit" => "commit",
                            "dict" => "dict",
                            "highlight" => "highlight",
                            "note" => "note",
                            "clear" => "clear",
                            _ => "copy",
                        };
                        sender.input(Input::SelectionKey(a));
                        return;
                    }
                    // a tap at x (0..1) on the page: where it goes in this book
                    k if k.starts_with("tap=") => {
                        let x: f64 = k[4..].parse().unwrap_or(0.5);
                        let before = self.pos.page;
                        let a = self.canvas.dev_tap(x);
                        eprintln!("tap: x={x} action={a:?} page_before={before}");
                        return;
                    }
                    "swipe-left" | "swipe-right" => {
                        let dx = if c == "swipe-left" { -200.0 } else { 200.0 };
                        let before = self.pos.page;
                        self.canvas.dev_swipe(dx, 0.0, 200);
                        eprintln!("swipe: {c} page_before={before}");
                        return;
                    }
                    "search" => {
                        self.w.search_entry.set_text("the");
                        sender.input(Input::Search("the".into()));
                        self.w.search_btn.popup();
                        return;
                    }
                    _ => {}
                }
                self.shared.cfg.borrow_mut().reader = p;
                self.apply_prefs();
            }
            Input::KeySelect(step, fwd) => {
                self.canvas.key_select(step, fwd);
                let said = match self.canvas.selected_text() {
                    Some(t) => {
                        let t: String = t.chars().take(160).collect();
                        trf("Selected: {}", &[&t])
                    }
                    None => "Selection cleared".to_string(),
                };
                self.canvas
                    .area
                    .announce(&said, gtk::AccessibleAnnouncementPriority::Medium);
            }
            Input::SelectionKey(k) => {
                if k == "clear" {
                    self.canvas.clear_selection();
                    return;
                }
                if self.selection.is_none() && self.canvas.has_selection() {
                    self.canvas.commit_selection();
                    if k != "commit" {
                        sender.input(Input::SelectionKey(k));
                    }
                    return;
                }
                match k {
                    "dict" => sender.input(Input::Dict),
                    "wiki" => sender.input(Input::Wiki),
                    "translate" => sender.input(Input::Translate),
                    "highlight" => sender.input(Input::Highlight("yellow".into())),
                    "note" => self.w.note_btn.emit_clicked(),
                    "copy" => sender.input(Input::Copy),
                    _ => {}
                }
            }
            Input::Quit => {
                // Escape first puts the sidebar away, then the full screen, then leaves the book
                if self.w.split.shows_sidebar() && self.w.split.is_collapsed() {
                    self.w.split.set_show_sidebar(false);
                    self.canvas.area.grab_focus();
                    return;
                }
                if let Some(win) = window_of(&self.w.page) {
                    if win.is_fullscreen() {
                        win.set_fullscreened(false);
                        self.fullscreen = false;
                        return;
                    }
                }
                let _ = sender.output(Output::Close);
            }
        }
        let _ = (&self.w.prev_btn, &self.w.next_btn);
    }

    fn shutdown(&mut self, _widgets: &mut Self::Widgets, _output: relm4::Sender<Self::Output>) {
        // what the cards show now is what comes back; the stops below must not erase it
        self.save_cards(true);
        self.tts.cards_frozen = true;
        // the clocks of read aloud and speed reading end with the book; a voice download goes on
        if let Some(t) = self.tts.timer.take() {
            t.remove();
        }
        let ev = self.rsvp.close(&self.canvas);
        self.rsvp_event(ev);
        {
            // the look of this book was only for this reading session: the defaults come back
            let mut cfg = self.shared.cfg.borrow_mut();
            if let Some(d) = cfg.reader_defaults.take() {
                cfg.reader = d;
                cfg.save();
            }
        }
        self.dwell_tick();
        crate::reading_dwell::flush();
        self.save_progress();
        super::kosync::close_book(&self.shared, self.book_id);
        let now = self.shared.now();
        self.shared.stats.borrow_mut().finish(now);
        let _ = self.shared.lib.borrow().save();
        if let Some(t) = self.timer.take() {
            t.remove();
        }
        self.overlay_guard.disconnect();
        self.w.sel_pop.popdown();
        if let Some(t) = self.footer_timer.take() {
            t.remove();
        }
        if self.w.footer_pop.parent().is_some() {
            self.w.footer_pop.unparent();
        }
        // leaving the book does not cancel a voice download (it is the application's job)
        self.tts.prep = None;
        self.tts_stop();
        if self.w.sel_pop.parent().is_some() {
            self.w.sel_pop.unparent();
        }
        if let Some(win) = window_of(&self.w.page) {
            win.set_fullscreened(false);
        }
    }
}

/// The theme `T` switches to: Light, Sepia, Dark, Black, and round again (the system theme counts
/// as Light).
fn next_theme(t: ReaderTheme) -> ReaderTheme {
    match t {
        ReaderTheme::Auto | ReaderTheme::Black => ReaderTheme::Light,
        ReaderTheme::Light => ReaderTheme::Sepia,
        ReaderTheme::Sepia => ReaderTheme::Dark,
        ReaderTheme::Dark => ReaderTheme::Black,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_cycles_through_the_themes() {
        let mut t = ReaderTheme::Auto;
        let mut seen = Vec::new();
        for _ in 0..5 {
            t = next_theme(t);
            seen.push(t);
        }
        assert_eq!(
            seen,
            vec![
                ReaderTheme::Light,
                ReaderTheme::Sepia,
                ReaderTheme::Dark,
                ReaderTheme::Black,
                ReaderTheme::Light
            ]
        );
    }

    #[test]
    fn book_pages_use_measured_and_estimated_chapters() {
        let weights = [1000.0, 1000.0, 2000.0];
        let mut known = HashMap::new();
        known.insert(0, 10); // 10 pages for 1000 bytes
        let (page, total) = estimate_book_pages(&known, &weights, 1, 3, 10);
        assert_eq!(page, 13);
        assert_eq!(total, 40);
        let (page, total) = estimate_book_pages(&HashMap::new(), &[1.0], 0, 5, 7);
        assert_eq!((page, total), (5, 7));
    }

    #[test]
    fn popover_swallows_clicks_but_not_motion_or_keys() {
        use gdk::EventType::*;
        for t in [ButtonPress, ButtonRelease, TouchBegin, TouchEnd, Scroll] {
            assert!(popover_event_stops_here(t), "{t:?}");
        }
        for t in [MotionNotify, KeyPress, KeyRelease, EnterNotify, LeaveNotify] {
            assert!(!popover_event_stops_here(t), "{t:?}");
        }
    }

    #[test]
    fn chrome_stays_while_in_use() {
        let at = |y: f64| ChromeUse {
            pointer_y: y,
            ..ChromeUse::idle(800.0)
        };
        let with = |f: fn(&mut ChromeUse)| {
            let mut u = ChromeUse::idle(800.0);
            f(&mut u);
            u
        };
        assert!(chrome_busy(with(|u| u.popover_open = true)));
        assert!(chrome_busy(at(20.0)));
        assert!(chrome_busy(at(760.0)));
        assert!(!chrome_busy(at(400.0)));
        assert!(!chrome_busy(ChromeUse::idle(800.0)));
        // the read-aloud popover (an open popover) keeps the bars even with the pointer in the page
        assert!(chrome_busy(ChromeUse {
            popover_open: true,
            pointer_y: 400.0,
            ..ChromeUse::idle(800.0)
        }));
        assert!(chrome_busy(with(|u| u.hovering = true)));
        assert!(chrome_busy(with(|u| u.keyboard_focus = true)));
        assert!(chrome_busy(ChromeUse {
            dragging: true,
            pointer_y: 400.0,
            ..ChromeUse::idle(800.0)
        }));
    }

    #[test]
    fn unknown_highlight_colour_falls_back_to_yellow() {
        assert_eq!(color_of("nonsense"), HIGHLIGHT_COLORS[0].1);
        assert_eq!(color_of("green"), HIGHLIGHT_COLORS[4].1);
    }
}
