//! The library window page: the sidebar (Home, Recent, Favourites, Notes & Highlights, shelves and
//! browse groups, Library Health), the book grid / table, the details panel that follows the
//! selection, search.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gdk, gio, glib};
use relm4::prelude::*;
use relm4::typed_view::column::{RelmColumn, TypedColumnView};
use relm4::typed_view::grid::{RelmGridItem, TypedGridView};

use super::covers;
use super::metadata::{self, CoverChange};
use super::{window_of, SharedRef};
use crate::config::ViewMode;
use crate::core::convert::{export_book, ExportFormat};
use crate::core::library::{AddOutcome, Book, Category, Prepared, SortKey};
use crate::core::util::*;
use crate::grid_columns;
use crate::grid_fill::{self, Chunker};
use crate::library_nav::{self as nav, View};

// ---------------------------------------------------------------------------
// Grid item
// ---------------------------------------------------------------------------

type Ctx = Rc<RefCell<Option<(u64, relm4::Sender<Input>)>>>;

thread_local! {
    /// Jobs panel entries of the news downloads in progress, by subscription key.
    static NEWS_JOBS: RefCell<std::collections::HashMap<String, u64>> = RefCell::new(Default::default());
}

/// What an import reports for a file it skipped because the job was cancelled.
#[path = "library_extra.rs"]
mod extra;

const IMPORT_CANCELLED: &str = "cancelled";

pub struct BookItem {
    id: u64,
    title: String,
    authors: String,
    cover: Option<PathBuf>,
    progress: f64,
    favourite: bool,
    /// Books on this card: 1, or the editions of a work.
    editions: usize,
    /// The book has audio you can play: the cover shows a headphones badge.
    audio: bool,
    /// What assistive technology hears after the title (`library_nav::card_description`).
    description: String,
    size: Rc<Cell<i32>>,
    tx: relm4::Sender<Input>,
}

pub struct ItemWidgets {
    picture: gtk::Picture,
    fallback: gtk::Box,
    fallback_label: gtk::Label,
    title: gtk::Label,
    author: gtk::Label,
    progress: gtk::ProgressBar,
    pct: gtk::Label,
    prog_row: gtk::Box,
    done: gtk::Image,
    fav: gtk::Button,
    badge: gtk::Button,
    audio: gtk::Image,
    ctx: Ctx,
    last_class: RefCell<Option<String>>,
}

thread_local! {
    /// Selecting is under way (a check box or a long press began it): a plain click on a card then
    /// toggles that book instead of choosing it alone. Ends when nothing is selected.
    static SELECTING: Cell<bool> = const { Cell::new(false) };
}

impl RelmGridItem for BookItem {
    type Root = gtk::Box;
    type Widgets = ItemWidgets;

    fn setup(item: &gtk::ListItem) -> (gtk::Box, ItemWidgets) {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.set_margin_all(10);
        root.set_halign(gtk::Align::Center);
        root.set_valign(gtk::Align::Start);
        super::shelves::attach_drag(&root);

        let overlay = gtk::Overlay::new();
        overlay.add_css_class("book-cover");
        overlay.set_overflow(gtk::Overflow::Hidden);

        let fallback = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let fallback_label = gtk::Label::new(None);
        fallback_label.set_wrap(true);
        fallback_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        fallback_label.set_justify(gtk::Justification::Center);
        fallback_label.set_valign(gtk::Align::Center);
        fallback_label.set_vexpand(true);
        fallback_label.set_margin_all(10);
        fallback_label.set_max_width_chars(14);
        fallback_label.add_css_class("cover-title");
        fallback.append(&fallback_label);
        overlay.set_child(Some(&fallback));

        let picture = gtk::Picture::new();
        picture.set_content_fit(gtk::ContentFit::Cover);
        picture.set_can_shrink(true);
        overlay.add_overlay(&picture);

        // reading progress sits *under* the cover: "5%  ▬▬▬▬"
        let progress = gtk::ProgressBar::new();
        progress.add_css_class("cover-progress");
        progress.set_valign(gtk::Align::Center);
        progress.set_hexpand(true);
        let pct = gtk::Label::new(None);
        pct.add_css_class("caption");
        pct.add_css_class("numeric");
        pct.add_css_class("cover-pct");
        let prog_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        prog_row.append(&pct);
        prog_row.append(&progress);

        let done = gtk::Image::from_icon_name("object-select-symbolic");
        done.add_css_class("cover-done");
        done.set_halign(gtk::Align::End);
        done.set_valign(gtk::Align::Start);
        done.set_margin_all(6);
        overlay.add_overlay(&done);

        // the star of a favourite, top left; a click on it takes the book off the favourites
        let fav = gtk::Button::from_icon_name("starred-symbolic");
        fav.add_css_class("fav-badge");
        fav.set_halign(gtk::Align::Start);
        fav.set_valign(gtk::Align::Start);
        fav.set_margin_start(6);
        fav.set_margin_top(6);
        fav.set_visible(false);
        fav.set_focus_on_click(false);
        let fav_tip = tr("Favourite: click to remove from Favourites");
        fav.set_tooltip_text(Some(&fav_tip));
        super::audit::label(&fav, &fav_tip);
        overlay.add_overlay(&fav);
        let badge = super::library_works::make_badge();
        overlay.add_overlay(&badge);
        let audio = super::audiobook_player::cover_badge();
        overlay.add_overlay(&audio);
        // the selection check box: on hover and focus, and while the book is selected
        let check = gtk::CheckButton::new();
        check.add_css_class("selection-mode");
        check.set_widget_name("select-check");
        check.set_halign(gtk::Align::Start);
        check.set_valign(gtk::Align::End);
        check.set_margin_start(6);
        check.set_margin_bottom(6);
        check.set_visible(false);
        check.set_focus_on_click(false);
        super::audit::label(&check, &tr("Select book"));
        overlay.add_overlay(&check);

        let title = gtk::Label::new(None);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_lines(2);
        title.set_wrap(true);
        title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        title.set_justify(gtk::Justification::Center);
        title.add_css_class("heading");
        // The tooltip is answered on demand. `set_tooltip_text` in `bind` made GTK pick the whole window
        // for every bound tile (a style pass over the window each time): 17 s for the first frame of a
        // 20,000 book grid in a real session.
        title.set_has_tooltip(true);
        title.connect_query_tooltip(|l, _, _, _, tip| {
            let text = l.label();
            if text.is_empty() {
                return false;
            }
            tip.set_text(Some(&text));
            true
        });
        let author = gtk::Label::new(None);
        author.set_ellipsize(gtk::pango::EllipsizeMode::End);
        author.add_css_class("dim-label");
        author.add_css_class("caption");

        root.append(&overlay);
        root.append(&prog_row);
        root.append(&title);
        root.append(&author);

        let ctx: Ctx = Rc::new(RefCell::new(None));
        {
            let ctx = ctx.clone();
            fav.connect_clicked(move |_| {
                if let Some((id, tx)) = ctx.borrow().as_ref() {
                    tx.emit(Input::ToggleFav(*id));
                }
            });
        }
        {
            let ctx = ctx.clone();
            badge.connect_clicked(move |b| {
                if let Some((id, tx)) = ctx.borrow().as_ref() {
                    tx.emit(Input::ShowEditions(*id, b.clone().upcast()));
                }
            });
        }
        let click = gtk::GestureClick::new();
        click.set_button(3);
        {
            let ctx = ctx.clone();
            let root_w = root.clone();
            click.connect_pressed(move |_, _, x, y| {
                if let Some((id, tx)) = ctx.borrow().as_ref() {
                    tx.emit(Input::Context(*id, x, y, root_w.clone().upcast()));
                }
            });
        }
        root.add_controller(click);
        // touch: a long press picks the book into the selection (a mouse has right-click and Ctrl)
        let long = gtk::GestureLongPress::new();
        long.set_touch_only(true);
        {
            let ctx = ctx.clone();
            long.connect_pressed(move |g, _, _| {
                if let Some((id, tx)) = ctx.borrow().as_ref() {
                    g.set_state(gtk::EventSequenceState::Claimed);
                    SELECTING.with(|s| s.set(true));
                    tx.emit(Input::Pick(*id, nav::Pick::Ctrl));
                }
            });
        }
        root.add_controller(long);

        // the check box and the selected state follow the selection of the list
        let updating = Rc::new(Cell::new(false));
        {
            let (check, root_w, updating) = (check.clone(), root.clone(), updating.clone());
            let hovered = Rc::new(Cell::new(false));
            let sync = {
                let (check, root_w, updating, hovered) = (
                    check.clone(),
                    root_w.clone(),
                    updating.clone(),
                    hovered.clone(),
                );
                Rc::new(move |selected: bool| {
                    updating.set(true);
                    check.set_active(selected);
                    updating.set(false);
                    check.set_visible(selected || hovered.get());
                    root_w.update_state(&[gtk::accessible::State::Selected(Some(selected))]);
                })
            };
            let s2 = sync.clone();
            item.connect_selected_notify(move |i| s2(i.is_selected()));
            let motion = gtk::EventControllerMotion::new();
            {
                let (h, i, s) = (hovered.clone(), item.clone(), sync.clone());
                motion.connect_enter(move |_, _, _| {
                    h.set(true);
                    s(i.is_selected());
                });
                let (h, i, s) = (hovered.clone(), item.clone(), sync.clone());
                motion.connect_leave(move |_| {
                    h.set(false);
                    s(i.is_selected());
                });
            }
            root.add_controller(motion);
            // keyboard focus on the card shows it too
            let focus = gtk::EventControllerFocus::new();
            {
                let (h, i, s) = (hovered.clone(), item.clone(), sync.clone());
                focus.connect_enter(move |_| {
                    h.set(true);
                    s(i.is_selected());
                });
                let (h, i, s) = (hovered.clone(), item.clone(), sync.clone());
                focus.connect_leave(move |_| {
                    h.set(false);
                    s(i.is_selected());
                });
            }
            root.add_controller(focus);
            let ctx2 = ctx.clone();
            check.connect_toggled(move |_| {
                if updating.get() {
                    return;
                }
                if let Some((id, tx)) = ctx2.borrow().as_ref() {
                    SELECTING.with(|s| s.set(true));
                    tx.emit(Input::Pick(*id, nav::Pick::Ctrl));
                }
            });
        }
        // once selecting has begun a plain click on the card toggles it
        let toggle = gtk::GestureClick::new();
        toggle.set_button(1);
        toggle.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let ctx = ctx.clone();
            let root_w = root.clone();
            toggle.connect_pressed(move |g, _, x, y| {
                if !SELECTING.with(|s| s.get()) {
                    return;
                }
                let mut w = root_w.pick(x, y, gtk::PickFlags::DEFAULT);
                while let Some(cur) = w {
                    if cur == *root_w.upcast_ref::<gtk::Widget>() {
                        break;
                    }
                    if cur.is::<gtk::Button>() || cur.is::<gtk::CheckButton>() {
                        return;
                    }
                    w = cur.parent();
                }
                if let Some((id, tx)) = ctx.borrow().as_ref() {
                    g.set_state(gtk::EventSequenceState::Claimed);
                    tx.emit(Input::Pick(*id, nav::Pick::Ctrl));
                }
            });
        }
        root.add_controller(toggle);

        (
            root,
            ItemWidgets {
                picture,
                fallback,
                fallback_label,
                title,
                author,
                progress,
                pct,
                prog_row,
                done,
                fav,
                badge,
                audio,
                ctx,
                last_class: RefCell::new(None),
            },
        )
    }

    fn bind(&mut self, w: &mut ItemWidgets, root: &mut gtk::Box) {
        root.set_widget_name(&self.id.to_string());
        let size = self.size.get();
        let (cw, ch) = (size, size * 3 / 2);
        w.picture.set_size_request(cw, ch);
        w.fallback.set_size_request(cw, ch);
        w.title.set_width_chars(1);
        w.title.set_max_width_chars((size / 9).max(8));
        w.author.set_max_width_chars((size / 9).max(8));
        w.title.set_label(&self.title);
        w.author.set_label(&self.authors);
        w.fallback_label.set_label(&self.title);
        if let Some(c) = w.last_class.borrow_mut().take() {
            w.fallback.remove_css_class(&c);
        }
        let class = covers::placeholder_class(&self.title);
        w.fallback.add_css_class(&class);
        *w.last_class.borrow_mut() = Some(class);

        match self
            .cover
            .as_deref()
            .and_then(|p| covers::thumbnail(p, size))
        {
            Some(t) => {
                w.picture.set_paintable(Some(&t));
                w.picture.set_visible(true);
            }
            None => {
                w.picture.set_paintable(gdk::Paintable::NONE);
                w.picture.set_visible(false);
            }
        }
        w.progress.set_fraction(self.progress);
        w.pct.set_label(&format!("{:.0}%", self.progress * 100.0));
        w.prog_row.set_size_request(cw, -1);
        w.prog_row
            .set_visible(self.progress > 0.0 && self.progress < 0.98);
        w.done.set_visible(self.progress >= 0.98);
        w.fav.set_visible(self.favourite);
        super::library_works::set_badge(&w.badge, self.editions);
        w.audio.set_visible(self.audio);
        root.update_property(&[
            gtk::accessible::Property::Label(&self.title),
            gtk::accessible::Property::Description(&self.description),
        ]);
        *w.ctx.borrow_mut() = Some((self.id, self.tx.clone()));
    }

    fn unbind(&mut self, w: &mut ItemWidgets, _root: &mut gtk::Box) {
        *w.ctx.borrow_mut() = None;
    }
}

// ---------------------------------------------------------------------------
// Table rows
// ---------------------------------------------------------------------------

pub struct BookRow {
    id: u64,
    title: String,
    authors: String,
    series: String,
    published: String,
    added: String,
    size: u64,
    rating: u8,
    tags: String,
    progress: u32,
    fav: bool,
    editions: u32,
    /// What the library sorts by, so that a click on a column header gives the order of the
    /// View options menu (`library_nav::sort_for_column`).
    title_sort: String,
    author_sort: String,
    series_key: String,
    added_ts: i64,
}

impl BookRow {
    fn from(b: &Book, fav: bool) -> Self {
        BookRow {
            fav,
            editions: 1,
            id: b.id,
            title: b.meta.title.clone(),
            authors: b.meta.authors.join(" & "),
            series: if b.meta.series.is_empty() {
                String::new()
            } else {
                format!(
                    "{} [{}]",
                    b.meta.series,
                    crate::core::opf::format_index(b.meta.series_index)
                )
            },
            published: b.meta.published.chars().take(10).collect(),
            added: b.added.format("%Y-%m-%d").to_string(),
            size: b.size,
            rating: b.meta.rating,
            tags: b.meta.tags.join(", "),
            progress: (b.progress * 100.0).round() as u32,
            title_sort: b.meta.title_sort.to_lowercase(),
            author_sort: b.meta.author_sort.to_lowercase(),
            series_key: format!(
                "{}\u{0}{:012.3}",
                b.meta.series.to_lowercase(),
                b.meta.series_index.max(0.0)
            ),
            added_ts: b.added.timestamp_millis(),
        }
    }
}

macro_rules! column {
    ($name:ident, $label:expr, $ty:ty, $expand:expr, $width:expr, |$i:ident| $val:expr $(, fmt = |$v:ident| $fmt:expr)? $(, sort = $sort:expr)?) => {
        struct $name;
        impl RelmColumn for $name {
            type Root = gtk::Label;
            type Widgets = ();
            type Item = BookRow;
            const COLUMN_NAME: &'static str = $label;
            const ENABLE_RESIZE: bool = true;
            fn header_title() -> String {
                tr($label)
            }
            const ENABLE_EXPAND: bool = $expand;
            fn setup(_: &gtk::ListItem) -> (gtk::Label, ()) {
                let l = gtk::Label::new(None);
                l.set_xalign(0.0);
                l.set_halign(gtk::Align::Fill);
                l.set_margin_start(6);
                l.set_margin_end(6);
                l.set_margin_top(6);
                l.set_margin_bottom(6);
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                l.set_width_chars($width);
                super::shelves::attach_drag(&l);
                (l, ())
            }
            fn bind(item: &mut BookRow, _: &mut (), label: &mut gtk::Label) {
                label.set_widget_name(&item.id.to_string());
                let $i = &*item;
                let value: $ty = $val;
                let text = column!(@fmt value $(, $v, $fmt)?);
                label.set_label(&text);
            }
            fn sort_fn() -> relm4::typed_view::OrdFn<BookRow> {
                Some(Box::new(|a: &BookRow, b: &BookRow| {
                    let ($i, ) = (a, );
                    let va = column!(@sort $ty, $val $(, $sort)?);
                    let ($i, ) = (b, );
                    let vb = column!(@sort $ty, $val $(, $sort)?);
                    va.partial_cmp(&vb)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| a.title_sort.cmp(&b.title_sort))
                }))
            }
        }
    };
    (@sort $ty:ty, $val:expr) => {{ let v: $ty = $val; v }};
    (@sort $ty:ty, $val:expr, $sort:expr) => {{ $sort }};
    (@fmt $value:ident) => { $value.to_string() };
    (@fmt $value:ident, $v:ident, $fmt:expr) => {{
        let $v = &$value;
        $fmt
    }};
}

column!(
    ColFav,
    "★",
    u8,
    false,
    2,
    |r| r.fav as u8,
    fmt = |v| if *v > 0 {
        "★".to_string()
    } else {
        String::new()
    }
);
column!(
    ColTitle,
    "Title",
    String,
    true,
    34,
    |r| r.title.clone(),
    sort = r.title_sort.clone()
);
column!(
    ColEditions,
    "Editions",
    u32,
    false,
    4,
    |r| r.editions,
    fmt = |v| if *v > 1 { v.to_string() } else { String::new() }
);
column!(
    ColAuthors,
    "Author(s)",
    String,
    true,
    18,
    |r| r.authors.clone(),
    sort = r.author_sort.clone()
);
column!(
    ColSeries,
    "Series",
    String,
    false,
    14,
    |r| r.series.clone(),
    sort = r.series_key.clone()
);
column!(ColPublished, "Published", String, false, 10, |r| r
    .published
    .clone());
column!(
    ColAdded,
    "Added",
    String,
    false,
    10,
    |r| r.added.clone(),
    sort = r.added_ts
);
column!(
    ColSize,
    "Size",
    u64,
    false,
    8,
    |r| r.size,
    fmt = |v| human_size(*v)
);
column!(
    ColRating,
    "Rating",
    u8,
    false,
    7,
    |r| r.rating,
    fmt = |v| {
        let s = (*v as usize).div_ceil(2);
        if s == 0 {
            String::new()
        } else {
            "★".repeat(s) + "☆".repeat(5 - s).as_str()
        }
    }
);
column!(ColTags, "Tags", String, false, 16, |r| r.tags.clone());
column!(
    ColProgress,
    "Read",
    u32,
    false,
    5,
    |r| r.progress,
    fmt = |v| format!("{v}%")
);

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

thread_local! {
    /// Fields that import patches changed in books just added: (book, fields, source).
    static PATCH_NOTES: RefCell<Vec<(u64, Vec<&'static str>, String)>> = const { RefCell::new(Vec::new()) };
    /// Files of the running import that are another format of a book the library has.
    static FORMAT_OFFERS: RefCell<Vec<(crate::book_formats::Offer, Prepared)>> = const { RefCell::new(Vec::new()) };
    /// Books added by the running import (for the optional metadata lookup) and how many of the
    /// files were protected.
    static IMPORT_ADDED: RefCell<(Vec<u64>, usize)> = const { RefCell::new((Vec::new(), 0)) };
}

#[derive(Debug, Clone)]
pub enum Act {
    Read,
    EditMeta,
    EditBook,
    MarkRead(bool),
    /// Export the annotations of the selected books (`crate::annotation_export::FORMATS` id).
    ExportAnn(String),
    /// Copy the citation (APA text) of the selected books to the clipboard.
    CopyCitation,
    /// Import Highlights… (Kindle clippings, KOReader, Kobo, calibre).
    ImportAnn,
    /// Export All Annotations… (one file per book).
    ExportAllAnn,
    /// Export as Calibre Library… (a background job).
    ExportCalibre,
    /// Notes & Highlights (every highlight of the library in the content area).
    Notes,
    /// Star or unstar the selected books.
    Favourite,
    /// Open the Properties dialog of the selected book.
    Properties,
    Convert(ExportFormat),
    /// Convert to an e-book format (`crate::export::TARGETS` id).
    ConvertTo(String),
    /// Send by e-mail to the address set in Preferences.
    Email,
    /// The same after the check before sending has passed.
    EmailGo,
    /// The same after the check before converting has passed.
    ConvertToGo(String),
    /// Fetch Metadata… for the selected books (review table).
    FetchMeta,
    /// Check Before Sending… (the blocking problems and warnings of the selected books).
    Preflight,
    /// Merge as Formats of One Book… (the first selected book stays).
    MergeFormats,
    SaveCopy,
    ShowFolder,
    Delete,
    /// Put the selected books on this shelf.
    ShelfAdd(String),
    /// Ask for a name, make the shelf and put the selected books on it.
    ShelfNew,
    /// Take the selected books off the shelf that is shown.
    ShelfRemove,
    /// Move the selected book earlier (-1) or later (1) on the shelf that is shown.
    ShelfMove(i32),
    /// Link the selected books as editions of one work.
    LinkEditions,
    /// Take every edition of the selected card out of its group.
    UnlinkEditions,
    /// Show the edition list of the selected card.
    ShowEditionList,
}

impl Act {
    /// Whether the action goes to every edition of a selected card (not just the one on top).
    fn reaches_editions(&self) -> bool {
        matches!(
            self,
            Act::MarkRead(_)
                | Act::Favourite
                | Act::ExportAnn(_)
                | Act::CopyCitation
                | Act::Convert(_)
                | Act::ConvertTo(_)
                | Act::ConvertToGo(_)
                | Act::Email
                | Act::EmailGo
                | Act::FetchMeta
                | Act::Preflight
                | Act::Delete
                | Act::ShelfAdd(_)
                | Act::ShelfNew
                | Act::ShelfRemove
                | Act::LinkEditions
                | Act::UnlinkEditions
        )
    }
}

#[derive(Debug)]
pub enum Input {
    Refresh,
    FindDuplicates,
    /// Series / Authors / Audiobooks / Trackers windows.
    Organize(&'static str),
    /// Library Health on this tab (`health::TABS`).
    HealthTab(&'static str),
    /// Show this view of the sidebar (Home, Recent, Favourites, Notes & Highlights).
    SetNav(View),
    /// A filter row of the sidebar: Home with this search, the search bar open.
    GoFilter(String),
    /// A click on nothing: clear the selection.
    ClearSelection,
    /// Star or unstar one book (the badge on its cover, the star of the details panel).
    ToggleFav(u64),
    /// Rating of these books, 0-10 (two points a star).
    SetRating(Vec<u64>, u8, bool),
    /// Add and remove tags on these books.
    EditTags {
        ids: Vec<u64>,
        add: Vec<String>,
        remove: Vec<String>,
        /// The edit was made in the details panel itself, which then keeps what it shows.
        in_panel: bool,
    },
    /// The Continue Reading banner changed size class or setting.
    BannerLayout,
    /// The window crossed the compact width (the bar's extra actions go into More).
    Compact(bool),
    /// How many highlights are selected in the notes view.
    NotesSelection(usize),
    /// A button of the banner.
    BannerAct(u64, super::library_banner::BannerAct),
    /// A click on a book with Ctrl or Shift held.
    Pick(u64, nav::Pick),
    /// Saved Searches / virtual libraries window.
    SavedSearches,
    /// Online (OPDS) catalogues window.
    Opds,
    /// Fetch Metadata… for these books.
    BulkFetch(Vec<u64>),
    /// Library Health window; `true` starts the Calibre import right away.
    Health(bool),
    AddWebDialog,
    /// Text pasted on the library (Ctrl+V): an address opens Add from the Web with it.
    PasteUrl(String),
    AddWeb(String),
    NewsDialog,
    /// Download these subscriptions now (queued, one at a time).
    NewsRun(Vec<String>),
    /// Download every subscription that is due (the scheduler tick).
    NewsDue,
    NewsDone(String, Result<(Prepared, String), String>),
    Requery,
    Search(String),
    Sort(SortKey),
    ToggleOrder,
    SetView(ViewMode),
    Zoom(i32),
    Selection(Vec<u64>),
    /// The badge of a card: its edition list, anchored on the badge.
    ShowEditions(u64, gtk::Widget),
    /// An edition's buttons: send it, prefer it, take it out of the group.
    EditionSend(u64),
    EditionPrefer(u64),
    EditionUnlink(u64),
    /// Turn *Group editions of the same book* on or off.
    GroupEditions(bool),
    Activate(u64),
    Context(u64, f64, f64, gtk::Widget),
    AddFiles,
    AddFolder(bool),
    ImportPaths(Vec<PathBuf>),
    Prepared(Result<Prepared, String>),
    Act(Act),
    Toast(String),
    /// A message with an Undo button, shown in the library window itself.
    ToastUndo(String, crate::app::UndoFn),
    /// The user opened or closed the details panel with its header toggle.
    DetailsPref(bool),
    /// Keep the search in the box under this name (Search Tips → Save This Search).
    SaveSearch(String),
    /// A click on a column header of the table: the sort it means (key, ascending).
    TableSorted(SortKey, bool),
    /// A column of the table switched on or off in View options.
    TableColumn(&'static str, bool),
    /// Back to the columns that fit.
    TableColumnsAuto,
    FocusSearch,
    SelectAll,
    /// The next piece of a book list that is being filled in idle steps (the number of the fill).
    FillStep(u64),
    Dev(String),
    /// The devices connected now (from the periodic look).
    Devices(Vec<bookx_core::contracts::DeviceInfo>),
    DeviceBanner(DeviceChoice),
}

#[derive(Debug)]
pub enum Output {
    Read(u64),
    /// Play the audiobook of this book in the player.
    PlayAudio(u64),
    EditBook(u64),
    Toast(String),
    LibraryChanged,
    /// Open this folder as the library (from the switcher in the header).
    SwitchLibrary(PathBuf),
}

struct W {
    /// Undo toasts of the library window (removing books, deleting a shelf).
    toasts: adw::ToastOverlay,
    last_undo: RefCell<Option<crate::app::UndoFn>>,
    stack: gtk::Stack,
    /// The books (`stack`) or the notes view.
    content: gtk::Stack,
    /// Home, Recent, Favourites, Notes & Highlights.
    top: gtk::ListBox,
    top_counts: Vec<gtk::Label>,
    health_badge: gtk::Label,
    sel_details: gtk::Button,
    sel_del: gtk::Button,
    details_toggle: gtk::ToggleButton,
    table_scroll: gtk::ScrolledWindow,
    /// The table's columns by their (English) title.
    table_cols: Vec<(&'static str, gtk::ColumnViewColumn)>,
    /// Set while the program moves the sort arrow of the table (the header handler then stays quiet).
    table_sync: Rc<Cell<bool>>,
    grid_clamp: adw::Clamp,
    drop_hint: gtk::Box,
    cols_box: gtk::Box,
    sort_dd: gtk::DropDown,
    apply_cols: Rc<dyn Fn()>,
    search: gtk::SearchEntry,
    details: gtk::Box,
    tags: gtk::Box,
    title: adw::WindowTitle,
    search_bar: gtk::SearchBar,
    split_tags: adw::OverlaySplitView,
    split_details: adw::OverlaySplitView,
    spinner: gtk::Spinner,
    toolbar: adw::ToolbarView,
    /// The header title: the view title, or "N selected" while books are selected.
    title_stack: gtk::Stack,
    sel_title: adw::WindowTitle,
    cancel_btn: gtk::Button,
    sel_read: gtk::Button,
    sel_edit: gtk::Button,
    sel_shelf: gtk::MenuButton,
    sel_send: gtk::Button,
    sel_convert: gtk::MenuButton,
    sel_more: gtk::MenuButton,
    add_menu: gio::Menu,
    banner: gtk::Revealer,
    banner_ctl: Rc<super::library_banner::BannerCtl>,
    order_btn: gtk::Button,
    view_btn: gtk::MenuButton,
    grid_only: Vec<gtk::Widget>,
    send_action: gio::SimpleAction,
    header: adw::HeaderBar,
    empty_btns: gtk::FlowBox,
    device_btn: gtk::Button,
    /// The fallback shown in the view-options popover when the header is compact.
    device_more: gtk::Button,
    device_banner: adw::Banner,
}

/// The books still to go into the model, by view.
enum FillRows {
    Grid(Chunker<BookItem>),
    Table(Chunker<BookRow>),
}

/// A fill in progress: what is left, when it began, the scroll position to get back to.
struct Fill {
    rows: FillRows,
    started: std::time::Instant,
    scroll: Option<f64>,
    steps_left: u32,
    busy: bool,
}

impl Fill {
    fn loaded(&self) -> usize {
        match &self.rows {
            FillRows::Grid(c) => c.loaded(),
            FillRows::Table(c) => c.loaded(),
        }
    }
    fn is_done(&self) -> bool {
        match &self.rows {
            FillRows::Grid(c) => c.is_done(),
            FillRows::Table(c) => c.is_done(),
        }
    }
}

pub struct LibraryView {
    shared: SharedRef,
    root: adw::NavigationPage,
    sender: ComponentSender<Self>,
    grid: TypedGridView<BookItem, gtk::MultiSelection>,
    table: TypedColumnView<BookRow, gtk::MultiSelection>,
    cover_size: Rc<Cell<i32>>,
    w: W,
    selected: Vec<u64>,
    /// Cards with several editions, by the edition that stands for them.
    groups: HashMap<u64, crate::work_groups::Card>,
    /// Every book that sits inside a card with several editions → that card's edition.
    card_of: HashMap<u64, u64>,
    /// The books the list shows, one per card, in order.
    card_ids: Vec<u64>,
    /// The book a Shift+click extends from.
    anchor: Option<u64>,
    /// What this run has shown of the banner (hero on a cold start, strip on later visits).
    hero: RefCell<nav::HeroSession>,
    compact: bool,
    notes_sel: usize,
    /// The colours of the banner for the last cover, by file.
    banner_colors: RefCell<Option<(PathBuf, bool, Option<crate::cover_color::BannerColors>)>>,
    /// Set while an edit of the details panel itself redraws the list (the panel keeps its focus).
    skip_details: Cell<bool>,
    /// How many times the book list was rebuilt (the tests check that panels do not do it).
    requeries: u32,
    /// The book list being put into the grid or table in pieces (`grid_fill`), if one is.
    fill: Option<Fill>,
    fill_gen: u64,
    /// The search text of the last fill (a new search starts at the top, a new sort keeps the place).
    fill_query: String,
    view: View,
    notes: Rc<RefCell<Option<super::notes::View>>>,
    query: String,
    pending: usize,
    stats: (usize, usize, usize),
    /// The Jobs panel entry of the import in progress: (id, cancel token, books asked for, books done).
    import_job: Option<(u64, crate::jobs::CancelToken, usize, usize)>,
    expanded: Rc<RefCell<HashSet<usize>>>,
    news: NewsState,
    /// Devices seen at the last look, the one the banner asks about, and the ones declined for now.
    devices: Vec<bookx_core::contracts::DeviceInfo>,
    banner_device: Option<bookx_core::contracts::DeviceInfo>,
    declined: HashSet<String>,
    device_scan: Rc<Cell<bool>>,
}

/// The answers of the "device connected" banner: a one-shot sync or built-in dismiss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceChoice {
    Sync,
    Dismiss,
}

/// The news subscriptions and what the downloader is doing.
struct NewsState {
    store: Rc<RefCell<crate::news::NewsStore>>,
    queue: std::collections::VecDeque<String>,
    running: Option<String>,
    new_issues: usize,
    failures: Vec<String>,
    slot: super::news::RefreshSlot,
}

impl LibraryView {
    /// What the Library Health window calls after it changed the library: save, show, toast.
    fn health_notify(&self) -> Rc<dyn Fn(String)> {
        let tx = self.sender.input_sender().clone();
        let shared = self.shared.clone();
        Rc::new(move |msg| {
            let _ = shared.lib.borrow().save();
            tx.emit(Input::Refresh);
            tx.emit(Input::Toast(msg));
        })
    }

    /// The last file of an import came in: save, tell the user, and ask about the files that are
    /// another format of a book the library has. With *Fetch metadata for new books* on, the
    /// lookup of the new books is queued (empty fields only).
    /// Counts one prepared book for the Jobs panel.
    fn import_step(&mut self, line: &str) {
        if let Some((id, _, total, done)) = self.import_job.as_mut() {
            *done += 1;
            crate::jobs::log(*id, line);
            crate::jobs::progress(
                *id,
                Some(*done as f64 / (*total).max(1) as f64),
                &format!("{done} of {total}"),
            );
        }
    }

    fn finish_import(&mut self, sender: &ComponentSender<Self>) {
        if let Some((id, token, total, done)) = self.import_job.take() {
            let (a, d, f) = self.stats;
            crate::jobs::log(
                id,
                &format!("{a} added, {d} already in the library, {f} failed"),
            );
            let _ = (total, done);
            let result = if f > 0 && a == 0 && d == 0 {
                Err(format!("{f} failed"))
            } else {
                Ok(())
            };
            let _ = token;
            crate::jobs::finish(id, result);
        }
        let _ = self.shared.lib.borrow().save();
        for (id, fields, source) in PATCH_NOTES.with(|n| std::mem::take(&mut *n.borrow_mut())) {
            let _ = self.shared.guard.borrow().store.set_provenance(
                id,
                &fields,
                &source,
                chrono::Utc::now().timestamp(),
            );
        }
        self.w.spinner.stop();
        self.w.spinner.set_visible(false);
        let (a, d, f) = self.stats;
        let (added_ids, protected) = IMPORT_ADDED.with(|x| std::mem::take(&mut *x.borrow_mut()));
        let mut msg = trf("Added {}", &[&ntr("{} book", "{} books", a)]);
        if protected > 0 {
            msg.push_str(&format!(
                ", {protected} protected (Bookx cannot open {}; Bookx does not remove DRM)",
                if protected == 1 { "it" } else { "them" }
            ));
        }
        if d > 0 {
            msg.push_str(&format!(", {d} already in library"));
        }
        if f > 0 {
            msg.push_str(&format!(", {f} failed"));
        }
        let _ = sender.output(Output::Toast(msg));
        self.requery();
        self.rebuild_tags();
        self.lib_changed();

        let offers = FORMAT_OFFERS.with(|f| std::mem::take(&mut *f.borrow_mut()));
        if let Some(win) = window_of(&self.root) {
            if !offers.is_empty() {
                crate::ui::formats::offer_formats(&win, &self.shared, offers, self.health_notify());
            }
            if self.shared.cfg.borrow().metadata_prefs.fetch_on_import && !added_ids.is_empty() {
                crate::ui::bulkfetch::start_import(
                    &win,
                    &self.shared,
                    added_ids,
                    self.health_notify(),
                );
            }
        }
    }

    /// What the *Formats* rows of the details panel can ask for.
    fn format_callbacks(&self) -> crate::ui::formats::Callbacks {
        let tx = self.sender.input_sender().clone();
        let tx2 = tx.clone();
        crate::ui::formats::Callbacks {
            read: Rc::new(move |id| tx.emit(Input::Activate(id))),
            convert: Rc::new(move |t| tx2.emit(Input::Act(Act::ConvertTo(t)))),
            changed: self.health_notify(),
        }
    }

    /// The query with `search:` references expanded and the virtual library applied. A broken
    /// reference (deleted search) leaves the query as typed.
    fn resolved_query(&self, q: &str) -> String {
        let c = self.shared.cfg.borrow();
        crate::saved_searches::resolve(q, c.virtual_library.as_deref(), &c.saved_searches)
            .unwrap_or_else(|_| q.to_string())
    }

    /// Where files downloaded from an online catalogue go: the normal import.
    fn opds_import(&self) -> crate::ui::opds::Import {
        let tx = self.sender.input_sender().clone();
        Rc::new(move |files| tx.emit(Input::ImportPaths(files)))
    }

    fn visible_ids(&self) -> Vec<u64> {
        let key = self.shared.cfg.borrow().sort_key;
        let asc = self.shared.cfg.borrow().sort_ascending;
        let (rest, phrases) = crate::fulltext::split_text_terms(&self.query);
        let rest = self.resolved_query(&rest);
        let (rest, fav_term) = nav::split_fav(&rest);
        let ids = self
            .shared
            .lib
            .borrow()
            .search(&crate::shelves::split_terms(&rest).0, key, asc);
        let (_, ids) = super::shelves::restrict(&self.shared, &rest, ids);
        let ids = match fav_term {
            Some(want) => {
                let favs: HashSet<u64> = self
                    .shared
                    .cfg
                    .borrow()
                    .favourites
                    .iter()
                    .copied()
                    .collect();
                ids.into_iter()
                    .filter(|i| favs.contains(i) == want)
                    .collect()
            }
            None => ids,
        };
        let ids = self.apply_nav_view(ids, !self.query.trim().is_empty());
        if phrases.is_empty() {
            return ids;
        }
        // `text:` terms search the contents of the books through the full-text index
        let ft = self.shared.fulltext.borrow();
        let Some(ft) = ft.as_ref() else {
            return Vec::new();
        };
        let hits: HashSet<u64> = ft
            .matching_books(&phrases, 5000)
            .unwrap_or_default()
            .into_iter()
            .collect();
        ids.into_iter().filter(|id| hits.contains(id)).collect()
    }

    /// The books of the sidebar view (Recent, Favourites) among the ones the search found.
    fn apply_nav_view(&self, ids: Vec<u64>, searching: bool) -> Vec<u64> {
        if matches!(self.view, View::Home | View::Notes) {
            return ids;
        }
        let favs = self.shared.cfg.borrow().favourites.clone();
        nav::apply_view(self.view, ids, &favs, &self.recent_ids(), searching)
    }

    fn recent_ids(&self) -> Vec<u64> {
        let stamps: Vec<nav::Stamp> = self
            .shared
            .lib
            .borrow()
            .books
            .iter()
            .map(|b| nav::Stamp {
                id: b.id,
                last_read: b.last_read.map(|t| t.timestamp()),
                added: b.added.timestamp(),
            })
            .collect();
        nav::recent(&stamps, nav::RECENT_LIMIT)
    }

    fn active_model(&self) -> gtk::MultiSelection {
        if self.shared.cfg.borrow().view_mode == ViewMode::Grid {
            self.grid.selection_model.clone()
        } else {
            self.table.selection_model.clone()
        }
    }

    fn dev_news_add(
        &self,
        title: &str,
        url: &str,
        schedule: &str,
        keep: Option<usize>,
    ) -> Result<String, String> {
        use crate::news::{recipe_from_form, ScheduleChoice, Subscription};
        let off = *chrono::Local::now().offset();
        let title = if title.trim().is_empty() {
            crate::news::feed_title(url, &*crate::web_import::fetcher())
                .map_err(|e| format!("{e:#}"))?
        } else {
            title.to_string()
        };
        let sched = ScheduleChoice::parse(schedule).map_err(|e| format!("{e:#}"))?;
        let recipe = recipe_from_form(&title, url, 25, false, sched.to_schedule(off), None)
            .map_err(|e| format!("{e:#}"))?;
        let mut sub = Subscription::new(recipe);
        if let Some(k) = keep {
            sub.keep_issues = k;
        }
        self.news
            .store
            .borrow_mut()
            .add(sub)
            .map_err(|e| format!("{e:#}"))
    }

    /// Starts the next queued download, if none is running.
    fn news_next(&mut self) {
        if self.news.running.is_some() {
            return;
        }
        let sub = loop {
            let Some(key) = self.news.queue.pop_front() else {
                self.news_summary();
                return;
            };
            if let Some(s) = self.news.store.borrow().get(&key).cloned() {
                break s;
            }
        };
        self.news.running = Some(sub.key.clone());
        let sub_key = sub.key.clone();
        let (job, _) = crate::jobs::start(
            crate::jobs::Kind::News,
            &trf("Downloading {}", &[&sub.recipe.title]),
            false,
        );
        NEWS_JOBS.with(|m| m.borrow_mut().insert(sub.key.clone(), job));
        let now = self.shared.now();
        let (res_tx, res_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let r = crate::news::run_subscription(&sub, &*crate::web_import::fetcher(), now)
                .map_err(|e| format!("{e:#}"));
            let _ = res_tx.send((sub.key, r));
        });
        let key = sub_key;
        let tx = self.sender.input_sender().clone();
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || match res_rx
            .try_recv()
        {
            Ok((key, r)) => {
                tx.emit(Input::NewsDone(key, r));
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => {
                // the worker died without an answer: free the slot instead of blocking all downloads
                tx.emit(Input::NewsDone(
                    key.clone(),
                    Err("the download stopped unexpectedly".into()),
                ));
                glib::ControlFlow::Break
            }
        });
    }

    /// Imports the new issue (or records the failure), prunes old issues and saves.
    fn news_finished(&mut self, key: &str, res: Result<(Prepared, String), String>) {
        self.news.running = None;
        let now = self.shared.now();
        let name = self
            .news
            .store
            .borrow()
            .get(key)
            .map(|s| s.recipe.title.clone())
            .unwrap_or_else(|| key.to_string());
        let outcome = res.and_then(|(prepared, summary)| {
            let mut lib = self.shared.lib.borrow_mut();
            lib.commit_import(prepared, true)
                .map_err(|e| format!("{e:#}"))?;
            let keep = self
                .news
                .store
                .borrow()
                .get(key)
                .map_or(crate::news::DEFAULT_KEEP, |s| s.keep_issues);
            let old = crate::news::prune_plan(&lib.books, key, keep);
            if !old.is_empty() {
                if let Err(e) = lib.remove(&old, true) {
                    tracing::warn!("news: removing old issues failed: {e:#}");
                }
            }
            lib.save().map_err(|e| format!("{e:#}"))?;
            Ok(summary)
        });
        if let Some(id) = NEWS_JOBS.with(|m| m.borrow_mut().remove(key)) {
            match &outcome {
                Ok(summary) => {
                    crate::jobs::log(id, summary);
                    crate::jobs::finish(id, Ok(()));
                }
                Err(e) => crate::jobs::finish(id, Err(e.clone())),
            }
        }
        match outcome {
            Ok(summary) => {
                self.news.new_issues += 1;
                let _ = self.news.store.borrow_mut().record(key, now, &summary);
                self.requery();
                self.rebuild_tags();
                self.lib_changed();
            }
            Err(e) => {
                tracing::warn!("news download failed: {name}: {e}");
                let _ = self
                    .news
                    .store
                    .borrow_mut()
                    .record(key, now, &trf("Failed: {}", &[&e]));
                self.news.failures.push(format!("{name}: {e}"));
            }
        }
        let slot = self.news.slot.borrow().clone();
        if let Some(f) = slot {
            f();
        }
    }

    /// The toast at the end of a batch of downloads.
    fn news_summary(&mut self) {
        let n = std::mem::take(&mut self.news.new_issues);
        let fails = std::mem::take(&mut self.news.failures);
        let msg = match (n, fails.first()) {
            (0, None) => return,
            (n, None) => trf("News: {}", &[&ntr("{} new issue", "{} new issues", n)]),
            (0, Some(f)) => trf("News failed: {}", &[&f]),
            (n, Some(f)) => trf(
                "News: {}, {} failed ({})",
                &[&ntr("{} new issue", "{} new issues", n), &fails.len(), &f],
            ),
        };
        self.sender.input(Input::Toast(msg));
    }

    fn lib_changed(&self) {
        let _ = self.sender.output(Output::LibraryChanged);
    }

    /// Starts putting the books into the grid (or the table): the first piece now, so the list is never
    /// empty, the rest in idle steps. A fill that is still running is dropped (its steps see an old number).
    fn begin_fill(&mut self, items: Vec<BookItem>, rows: Vec<BookRow>, grid_mode: bool) {
        let same_query = self.fill_query == self.query;
        self.fill_query = self.query.clone();
        let adj = self.grid_vadjustment();
        // the place to get back to: the one an unfinished fill was heading for, else where the list is now
        let keep = match self.fill.as_ref().map(|f| f.scroll) {
            Some(s) => s,
            None => Some(adj.value()).filter(|v| *v > 0.5),
        }
        .filter(|_| same_query && grid_mode);
        let old_books = self.grid.len() as usize;
        self.fill = None;
        self.fill_gen += 1;
        self.w.stack.set_cursor(None);
        if !same_query {
            adj.set_value(0.0);
        }
        self.grid.clear();
        self.table.clear();
        let budget = if crate::eink::is_active() {
            grid_fill::BUDGET_EINK_MS
        } else {
            grid_fill::BUDGET_MS
        };
        let first = match keep {
            Some(v) => grid_fill::first_for_scroll(v, adj.page_size(), adj.upper(), old_books),
            None => grid_fill::FIRST_CHUNK,
        };
        let rows = if grid_mode {
            FillRows::Grid(Chunker::new(self.fill_gen, items, first, budget))
        } else {
            FillRows::Table(Chunker::new(self.fill_gen, rows, first, budget))
        };
        self.fill = Some(Fill {
            rows,
            started: std::time::Instant::now(),
            scroll: keep,
            steps_left: 12,
            busy: false,
        });
        // `BOOKX_FILL_SYNC=1`: the whole list at once (the old behaviour; for measuring against)
        let sync = std::env::var_os("BOOKX_FILL_SYNC").is_some();
        self.fill_piece(sync);
        self.schedule_fill();
    }

    /// Puts the next piece (or everything that is left) into the model and selects the books of it that are
    /// selected. Returns false when there is no fill.
    fn fill_piece(&mut self, all: bool) -> bool {
        let Some(mut fill) = self.fill.take() else {
            return false;
        };
        let t = std::time::Instant::now();
        let before = fill.loaded();
        match &mut fill.rows {
            FillRows::Grid(c) => {
                let piece = if all { c.take_all() } else { c.take() };
                let n = piece.len();
                self.grid.extend_from_iter(piece);
                c.record(n, t.elapsed().as_secs_f64() * 1000.0);
            }
            FillRows::Table(c) => {
                let piece = if all { c.take_all() } else { c.take() };
                let n = piece.len();
                self.table.extend_from_iter(piece);
                c.record(n, t.elapsed().as_secs_f64() * 1000.0);
            }
        }
        if !self.selected.is_empty() {
            let model = self.active_model();
            let want: HashSet<u64> = self.selected.iter().copied().collect();
            for i in before..fill.loaded() {
                if self
                    .id_at_model(&model, i as u32)
                    .is_some_and(|id| want.contains(&id))
                {
                    model.select_item(i as u32, false);
                }
            }
        }
        if let Some(v) = fill.scroll {
            // the adjustment learns the new length only when the grid is laid out (a frame later): aim at the
            // place on every step and stop when it holds with the whole list in
            let adj = self.grid_vadjustment();
            let room = adj.upper() >= v + adj.page_size();
            if room && (adj.value() - v).abs() > 1.0 {
                adj.set_value(v);
            }
            if fill.is_done() && room && (adj.value() - v).abs() <= 1.0 {
                fill.scroll = None;
            }
        }
        let elapsed = fill.started.elapsed().as_millis();
        let busy = grid_fill::show_progress(fill.is_done(), elapsed);
        if busy != fill.busy {
            fill.busy = busy;
            self.w
                .stack
                .set_cursor_from_name(busy.then_some("progress"));
        }
        // a finished fill stays until the scroll position was restored (about half a second at most)
        if fill.is_done() {
            fill.steps_left = fill.steps_left.saturating_sub(1);
        }
        if !fill.is_done() || (fill.scroll.is_some() && fill.steps_left > 0) {
            self.fill = Some(fill);
        }
        true
    }

    /// True while books of the list are still to come.
    fn filling(&self) -> bool {
        self.fill.as_ref().is_some_and(|f| !f.is_done())
    }

    /// The `stalls` dev command: four actions in a row, each given 1.5 s, with the longest gap between two
    /// 4 ms timers measured (a block of the main loop by the work or by the frame that follows it).
    fn dev_stalls(&self, sender: &ComponentSender<Self>) {
        let gap = Rc::new(Cell::new((std::time::Instant::now(), 0u128)));
        let g = gap.clone();
        let watch = glib::timeout_add_local(std::time::Duration::from_millis(4), move || {
            let (last, max) = g.get();
            let late = last.elapsed().as_millis().saturating_sub(4);
            g.set((std::time::Instant::now(), max.max(late)));
            glib::ControlFlow::Continue
        });
        let search = self.w.search.clone();
        let other = SortKey::ALL.get(1).copied().unwrap_or(SortKey::ALL[0]);
        let first = SortKey::ALL[0];
        let tx = sender.input_sender().clone();
        type Step = Box<dyn Fn()>;
        let steps: Vec<(&'static str, Step)> = vec![
            ("sort", {
                let tx = tx.clone();
                Box::new(move || tx.emit(Input::Sort(other)))
            }),
            ("search", {
                let s = search.clone();
                Box::new(move || s.set_text("ba"))
            }),
            ("clear", {
                let s = search.clone();
                Box::new(move || s.set_text(""))
            }),
            ("sortback", Box::new(move || tx.emit(Input::Sort(first)))),
        ];
        fn run(
            mut steps: std::collections::VecDeque<(&'static str, Step)>,
            gap: Rc<Cell<(std::time::Instant, u128)>>,
            watch: glib::SourceId,
            mut out: Vec<String>,
            mut worst: u128,
        ) {
            let Some((name, act)) = steps.pop_front() else {
                watch.remove();
                eprintln!("stalls: {} max={worst}", out.join(" "));
                return;
            };
            gap.set((std::time::Instant::now(), 0));
            act();
            glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
                let m = gap.get().1;
                out.push(format!("{name}={m}"));
                worst = worst.max(m);
                run(steps, gap, watch, out, worst);
            });
        }
        run(steps.into(), gap, watch, Vec::new(), 0);
    }

    /// Queues the next step below the redraw priority, so a frame is drawn between two pieces.
    fn schedule_fill(&self) {
        if self.fill.is_none() {
            return;
        }
        let (tx, gen) = (self.sender.input_sender().clone(), self.fill_gen);
        if self.filling() {
            glib::idle_add_local_full(glib::Priority::DEFAULT_IDLE, move || {
                tx.emit(Input::FillStep(gen));
                glib::ControlFlow::Break
            });
        } else {
            // only the scroll position is left: wait for the frame that lays the list out
            glib::timeout_add_local_once(std::time::Duration::from_millis(50), move || {
                tx.emit(Input::FillStep(gen))
            });
        }
    }

    /// Puts all the books that are still to come into the model now (select all, a click on the list).
    fn finish_fill(&mut self) {
        if !self.filling() {
            return;
        }
        self.fill_piece(true);
        self.restore_selection();
    }

    fn requery(&mut self) {
        self.requeries += 1;
        let ids = self.visible_ids();
        let lib = self.shared.lib.borrow();
        let size = self.cover_size.clone();
        let tx = self.sender.input_sender().clone();
        // one pass, not a `get` per id: that is quadratic and took seconds at 100,000 books
        let books: Vec<&Book> = lib.books_for(&ids);
        let favs: HashSet<u64> = self
            .shared
            .cfg
            .borrow()
            .favourites
            .iter()
            .copied()
            .collect();

        // editions of one work: one card each (the preferred edition stands for it)
        let group_on = self.shared.cfg.borrow().group_editions;
        let cards: Vec<crate::work_groups::Card> = if group_on {
            let links = crate::work_groups::Links::from_store(&self.shared.guard.borrow().store);
            crate::work_groups::group(&books, &links)
        } else {
            Vec::new()
        };
        let by_id: HashMap<u64, &Book> = books.iter().map(|b| (b.id, *b)).collect();
        // (card book, the book whose progress it shows, editions on the card, starred)
        let shown: Vec<(&Book, &Book, usize, bool)> = if group_on {
            cards
                .iter()
                .filter_map(|c| {
                    let rep = *by_id.get(&c.rep)?;
                    let prog = by_id.get(&c.progress_from).copied().unwrap_or(rep);
                    let fav = c.members.iter().any(|m| favs.contains(m));
                    Some((rep, prog, c.members.len(), fav))
                })
                .collect()
        } else {
            books
                .iter()
                .map(|b| (*b, *b, 1, favs.contains(&b.id)))
                .collect()
        };
        // the shelves of every book, for what a card tells assistive technology
        let mut on_shelves: HashMap<u64, Vec<String>> = HashMap::new();
        if self.shared.cfg.borrow().view_mode == ViewMode::Grid {
            let store = self.shared.guard.borrow().store.clone();
            for c in super::shelves::list(&self.shared) {
                for id in store.collection_books(c.id) {
                    on_shelves.entry(id).or_default().push(c.name.clone());
                }
            }
        }
        let items: Vec<BookItem> = shown
            .iter()
            .map(|(b, prog, n, fav)| BookItem {
                description: nav::card_description(
                    &b.meta.authors.join(" & "),
                    (prog.progress * 100.0).round() as u32,
                    *fav,
                    on_shelves.get(&b.id).map_or(&[][..], |v| v.as_slice()),
                ),
                id: b.id,
                title: b.meta.title.clone(),
                authors: b.meta.authors.join(" & "),
                cover: lib.cover_path(b),
                progress: prog.progress,
                favourite: *fav,
                editions: *n,
                audio: crate::audiobook::is_audiobook(b),
                size: size.clone(),
                tx: tx.clone(),
            })
            .collect();
        let rows: Vec<BookRow> = shown
            .iter()
            .map(|(b, prog, n, fav)| {
                let mut r = BookRow::from(b, *fav);
                r.progress = (prog.progress * 100.0).round() as u32;
                r.editions = *n as u32;
                r
            })
            .collect();
        let card_ids: Vec<u64> = shown.iter().map(|(b, ..)| b.id).collect();
        self.groups = cards
            .into_iter()
            .filter(|c| c.is_group())
            .map(|c| (c.rep, c))
            .collect();
        self.card_of = self
            .groups
            .values()
            .flat_map(|c| c.members.iter().map(|m| (*m, c.rep)))
            .collect();
        self.card_ids = card_ids;
        let total = lib.books.len();
        let n = books.len();
        drop(lib);

        let mode = self.shared.cfg.borrow().view_mode;
        self.begin_fill(items, rows, mode == ViewMode::Grid);

        let nothing_in_view = n == 0
            && self.query.trim().is_empty()
            && matches!(self.view, View::Recent | View::Favourites);
        if nothing_in_view {
            if let Some(p) = self
                .w
                .stack
                .child_by_name("viewempty")
                .and_downcast::<adw::StatusPage>()
            {
                let (icon, title, text) = if self.view == View::Recent {
                    (
                        "document-open-recent-symbolic",
                        tr("Nothing read yet"),
                        tr("Books you open appear here, the latest first."),
                    )
                } else {
                    (
                        "starred-symbolic",
                        tr("No favourites yet"),
                        tr("Right-click a book and choose to add it to your favourites to find it here."),
                    )
                };
                p.set_icon_name(Some(icon));
                p.set_title(&title);
                p.set_description(Some(&text));
            }
        }
        let page = if nothing_in_view {
            "viewempty"
        } else if total == 0 {
            "empty"
        } else if n == 0 {
            "nomatch"
        } else if mode == ViewMode::Grid {
            "grid"
        } else {
            "table"
        };
        self.w.stack.set_visible_child_name(page);
        self.show_content();
        let vl = self.shared.cfg.borrow().virtual_library.clone();
        let mut sub = if self.query.trim().is_empty() && vl.is_none() {
            ntr("{} book", "{} books", total)
        } else {
            trf("{} of {} books", &[&n, &total])
        };
        if let Some(v) = vl {
            sub = format!("{sub} · {v}");
        }
        self.w.title.set_title(&self.view_title());
        self.w.title.set_subtitle(&sub);
        // a book that is now hidden inside a card is selected through its card
        let mut sel: Vec<u64> = Vec::new();
        for id in &self.selected {
            let id = self.card_of.get(id).copied().unwrap_or(*id);
            if ids.contains(&id) && !sel.contains(&id) {
                sel.push(id);
            }
        }
        self.selected = sel;
        self.restore_selection();
        self.update_banner();
        if !self.skip_details.get() {
            self.update_details();
        }
        self.update_bottom_bar();
    }

    /// `size=WxH cols=N width=W requeries=R` of the realised tiles of the cover grid.
    fn tile_report(&self) -> String {
        let view = self.grid.view.clone();
        let mut tiles: Vec<(f64, f64, i32, i32)> = Vec::new();
        let mut c = view.first_child();
        while let Some(w) = c {
            if let Some(root) = w.first_child() {
                if root.widget_name().parse::<u64>().is_ok() {
                    if let Some(b) = root.compute_bounds(&view) {
                        tiles.push((b.x() as f64, b.y() as f64, root.width(), root.height()));
                    }
                }
            }
            c = w.next_sibling();
        }
        tiles.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.total_cmp(&b.0)));
        let Some(first) = tiles.first().copied() else {
            return format!(
                "size=0x0 cols=0 width={} requeries={}",
                view.width(),
                self.requeries
            );
        };
        let cols = tiles.iter().filter(|t| (t.1 - first.1).abs() < 1.0).count();
        format!(
            "size={}x{} cols={cols} width={} requeries={}",
            first.2,
            first.3,
            view.width(),
            self.requeries
        )
    }

    /// The name of what the content area shows: the view, the shelf or "Search results".
    fn view_title(&self) -> String {
        if self.view != View::Home {
            return match self.view {
                View::Recent => tr("Recent"),
                View::Favourites => tr("Favourites"),
                _ => tr("Notes & Highlights"),
            };
        }
        if self.query.trim().is_empty() {
            return tr("Home");
        }
        match super::shelves::current_shelf(&self.query) {
            Some(name) => name,
            None => tr("Search results"),
        }
    }

    /// The books or the notes view in the content area.
    fn show_content(&self) {
        let notes = self.view == View::Notes;
        self.w
            .content
            .set_visible_child_name(if notes { "notes" } else { "books" });
        self.w.search.set_sensitive(self.view.shows_books());
    }

    /// Shows a view of the sidebar.
    /// Opens the search bar without taking the keyboard focus from where it is (a sidebar row).
    fn open_search_bar(&self) {
        let prev = self.root.root().and_then(|r| r.focus());
        self.w.search_bar.set_search_mode(true);
        if let Some(p) = prev {
            p.grab_focus();
        }
    }

    /// Puts the search bar and entry in the state a sidebar click asks for.
    fn show_search(&mut self, state: nav::SearchState) {
        self.query = state.query.clone();
        if state.open {
            self.open_search_bar();
            if self.w.search.text() != state.query {
                self.w.search.set_text(&state.query);
            }
        } else {
            self.w.search.set_text("");
            self.w.search_bar.set_search_mode(false);
        }
    }

    fn set_view(&mut self, view: View) {
        let changed = self.view != view;
        if changed {
            self.leave_home();
        }
        self.view = view;
        {
            let mut c = self.shared.cfg.borrow_mut();
            if c.sidebar_view != view.key() {
                c.sidebar_view = view.key().to_string();
                c.save();
            }
        }
        if self.w.split_tags.is_collapsed() {
            self.w.split_tags.set_show_sidebar(false);
        }
        self.select_top_row();
        if view == View::Notes {
            self.ensure_notes();
        }
        self.follow_selection();
        if changed {
            self.requery();
        } else {
            self.show_content();
        }
    }

    fn ensure_notes(&self) {
        let mut slot = self.notes.borrow_mut();
        if slot.is_none() {
            let (t1, t2) = (
                self.sender.output_sender().clone(),
                self.sender.input_sender().clone(),
            );
            let v = super::notes::build(
                &self.shared,
                Rc::new(move |id| {
                    let _ = t1.send(Output::Read(id));
                }),
                Rc::new(move |m| t2.emit(Input::Toast(m))),
            );
            self.w.content.add_named(&v.widget, Some("notes"));
            let tx = self.sender.input_sender().clone();
            (v.on_selection)(Rc::new(move |n| tx.emit(Input::NotesSelection(n))));
            *slot = Some(v);
        }
        if let Some(v) = slot.as_ref() {
            (v.reload)();
            v.search.grab_focus();
        }
    }

    fn restore_selection(&self) {
        let model = self.active_model();
        let want: HashSet<u64> = self.selected.iter().copied().collect();
        model.unselect_all();
        for i in 0..model.n_items() {
            if let Some(id) = self.id_at_model(&model, i) {
                if want.contains(&id) {
                    model.select_item(i, false);
                }
            }
        }
    }

    fn id_at_model(&self, model: &gtk::MultiSelection, pos: u32) -> Option<u64> {
        id_from_model(model.upcast_ref(), pos)
    }

    fn current_selection(&self) -> Vec<u64> {
        let model = self.active_model();
        let mut out = Vec::new();
        for i in 0..model.n_items() {
            if model.is_selected(i) {
                if let Some(id) = self.id_at_model(&model, i) {
                    out.push(id);
                }
            }
        }
        out
    }

    /// The details panel follows the selection: it slides in when a book is selected and out when
    /// the selection is cleared. In a narrow window (where it would cover the books) the
    /// "Details" button of the selection bar opens it instead.
    fn follow_selection(&self) {
        let split = &self.w.split_details;
        let want = nav::details_for(self.selected.len()) != nav::Details::Hidden
            && self.view != View::Notes
            && self.shared.cfg.borrow().details_pane;
        if want && split.is_collapsed() {
            return;
        }
        if split.shows_sidebar() != want {
            split.set_show_sidebar(want);
        }
    }

    /// The header button, the banner and the persistent automatic-sync setting follow the
    /// connected devices. Answering the banner never changes that setting.
    fn devices_changed(&mut self, list: Vec<bookx_core::contracts::DeviceInfo>) {
        let fresh = crate::device_sync::newly_connected(&self.devices, &list);
        let gone: Vec<String> = self
            .devices
            .iter()
            .filter(|d| !list.iter().any(|n| n.id == d.id))
            .map(|d| d.id.clone())
            .collect();
        for id in gone {
            self.declined.remove(&id);
        }
        self.devices = list;
        self.w.send_action.set_enabled(!self.devices.is_empty());
        let label = crate::device_sync::button_label(&self.devices);
        if let Some(label) = &label {
            if let Some(c) = self
                .w
                .device_btn
                .child()
                .and_downcast::<adw::ButtonContent>()
            {
                c.set_label(label);
            }
            // the narrow fallback shows the device too, never a generic "Connected device"
            self.w.device_more.set_label(label);
        }
        self.w.device_btn.set_visible(label.is_some());
        if let Some(b) = &self.banner_device {
            if !self.devices.iter().any(|d| d.id == b.id) {
                self.banner_device = None;
                self.w.device_banner.set_revealed(false);
            }
        }
        if self.shared.cfg.borrow().device_sync.auto_sync {
            self.banner_device = None;
            self.w.device_banner.set_revealed(false);
            for d in fresh {
                self.sync_device(d, true);
            }
            return;
        }
        let Some(first) = fresh.into_iter().next() else {
            return;
        };
        if !self.declined.contains(&first.id) {
            self.banner_device = Some(first);
            // an AdwBanner keeps the same title and button for every device
            self.w
                .device_banner
                .set_title(&crate::device_sync::banner_title());
            self.w
                .device_banner
                .set_button_label(Some(&crate::device_sync::banner_button_label()));
            self.w.device_banner.set_revealed(true);
        }
    }

    fn device_banner_answer(&mut self, choice: DeviceChoice) {
        let Some(device) = self.banner_device.take() else {
            return;
        };
        self.w.device_banner.set_revealed(false);
        match choice {
            DeviceChoice::Dismiss => {
                self.declined.insert(device.id);
            }
            DeviceChoice::Sync => self.sync_device(device, false),
        }
    }

    /// Syncs with a device: after a summary the user confirms, or (automatic sync, switched on by
    /// the user) straight away with the result as a toast.
    fn sync_device(&self, device: bookx_core::contracts::DeviceInfo, auto: bool) {
        let Some(win) = window_of(&self.root) else {
            return;
        };
        let tx = self.sender.input_sender().clone();
        let tx2 = tx.clone();
        super::devices::start_sync(
            &win,
            &self.shared,
            device,
            if auto {
                super::devices::SyncMode::Auto
            } else {
                super::devices::SyncMode::Ask
            },
            Rc::new(move |m| tx.emit(Input::Toast(m))),
            Rc::new(move || tx2.emit(Input::Refresh)),
            Rc::new(|_| {}),
        );
    }

    /// The sort key and direction are one state: the View options menu and the arrow of the table
    /// header show the same thing.
    fn sync_sort_controls(&self) {
        let (key, asc) = {
            let c = self.shared.cfg.borrow();
            (c.sort_key, c.sort_ascending)
        };
        let w = &self.w;
        w.table_sync.set(true);
        if let Some(pos) = SortKey::ALL.iter().position(|k| *k == key) {
            w.sort_dd.set_selected(pos as u32);
        }
        super::audit::label(&w.order_btn, &order_label(asc));
        w.order_btn.set_icon_name(if asc {
            "view-sort-ascending-symbolic"
        } else {
            "view-sort-descending-symbolic"
        });
        let col = nav::column_for_sort(key)
            .and_then(|n| w.table_cols.iter().find(|(c, _)| *c == n))
            .map(|(_, c)| c);
        self.table.view.sort_by_column(
            col,
            if asc {
                gtk::SortType::Ascending
            } else {
                gtk::SortType::Descending
            },
        );
        w.table_sync.set(false);
    }

    /// Add from the Web, with `url` already in the box (a pasted address).
    fn add_web_dialog(&self, url: &str) {
        let Some(win) = window_of(&self.root) else {
            return;
        };
        let d = adw::AlertDialog::new(
            Some(&tr("Add from the Web")),
            Some(&tr("Paste the address of an article to read later, or of a news feed (RSS, Atom, JSON Feed) to make a book of its latest articles.")),
        );
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some(&tr("https://example.com/article")));
        entry.set_text(url);
        entry.set_widget_name("add-web-url");
        crate::ui::audit::label(&entry, &tr("Address"));
        entry.set_activates_default(true);
        d.set_extra_child(Some(&entry));
        d.add_responses(&[("cancel", &tr("Cancel")), ("add", &tr("Add"))]);
        d.set_response_appearance("add", adw::ResponseAppearance::Suggested);
        d.set_default_response(Some("add"));
        let tx = self.sender.input_sender().clone();
        d.connect_response(None, move |_, r| {
            if r == "add" {
                tx.emit(Input::AddWeb(entry.text().to_string()));
            }
        });
        d.present(Some(&win));
    }

    fn update_bottom_bar(&self) {
        let in_notes = self.view == View::Notes;
        let n = if in_notes { 0 } else { self.selected.len() };
        if n == 0 {
            SELECTING.with(|s| s.set(false));
        }
        // a single click just selects (the details panel shows the actions); the bar is for acting
        // on several books, and in a narrow window it also has the button that opens the panel
        let narrow = self.w.split_details.is_collapsed();
        let compact = self.compact;
        self.w
            .toolbar
            .set_reveal_bottom_bars(n >= 2 || (n == 1 && narrow));
        let hs = nav::header_state(if in_notes { self.notes_sel } else { n });
        self.w
            .sel_title
            .set_title(&trf("{} selected", &[&hs.selected]));
        self.w
            .title_stack
            .set_visible_child_name(if hs.selecting { "selection" } else { "title" });
        self.w.cancel_btn.set_visible(hs.selecting);
        self.w.sel_details.set_visible(narrow);
        // a long label ("In den Papierkorb verschieben") would push the bar wider than a 360 px window
        if compact {
            self.w.sel_del.set_icon_name("user-trash-symbolic");
        } else {
            self.w.sel_del.set_label(&tr("Move to Trash"));
        }
        let (inline, more) = nav::split_bulk(n, compact);
        let has = |a: nav::BulkAction| inline.contains(&a);
        self.w.sel_read.set_visible(has(nav::BulkAction::Read));
        self.w
            .sel_edit
            .set_visible(has(nav::BulkAction::EditMetadata));
        self.w
            .sel_shelf
            .set_visible(has(nav::BulkAction::AddToShelf));
        self.w
            .sel_send
            .set_visible(has(nav::BulkAction::SendToDevice));
        self.w
            .sel_convert
            .set_visible(has(nav::BulkAction::Convert));
        self.w.sel_more.set_visible(!more.is_empty());
        if n >= 2 {
            self.w
                .sel_shelf
                .set_menu_model(Some(&super::shelves::add_to_shelf_menu(&self.shared)));
            self.w
                .sel_convert
                .set_menu_model(Some(&self.convert_menu()));
        }
        if !more.is_empty() {
            self.w.sel_more.set_menu_model(Some(&self.more_menu(&more)));
        }
    }

    /// Redraws the books and the sidebar after an edit; the details panel stays as it is when the
    /// edit was made in it (it already shows the change and keeps its focus), else it is redrawn.
    fn after_meta_edit(&mut self, in_panel: bool) {
        self.skip_details.set(in_panel);
        self.requery();
        self.rebuild_tags();
        self.skip_details.set(false);
    }

    /// Applies a change to the metadata of these books like the edit dialog does: the file is
    /// rewritten, the catalogue saved and the change recorded as made by the user. Returns how many
    /// books changed.
    fn edit_meta(&mut self, ids: &[u64], f: &dyn Fn(&mut crate::core::opf::Metadata)) -> usize {
        let mut n = 0;
        for id in ids {
            let before = match self.shared.lib.borrow().get(*id) {
                Some(b) => b.meta.clone(),
                None => continue,
            };
            let mut after = before.clone();
            f(&mut after);
            if crate::metadata_fetch::changed_fields(&before, &after).is_empty() {
                continue;
            }
            let ok = self
                .shared
                .lib
                .borrow_mut()
                .update_metadata(*id, after.clone())
                .is_ok();
            if ok {
                crate::library_health::record_edit(
                    &self.shared.guard.borrow().store,
                    *id,
                    &before,
                    &after,
                    &[],
                );
                n += 1;
            }
        }
        if n > 0 {
            let _ = self.shared.lib.borrow().save();
            self.lib_changed();
        }
        n
    }

    /// The vertical adjustment of the scrolled window that holds the cover grid.
    fn grid_vadjustment(&self) -> gtk::Adjustment {
        self.grid
            .view
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast::<gtk::ScrolledWindow>()
            .map(|s| s.vadjustment())
            .unwrap_or_default()
    }

    fn grid_scroll_adjustment(&self) -> gtk::Adjustment {
        self.grid
            .view
            .parent()
            .and_downcast::<gtk::Viewport>()
            .and_then(|v| v.parent())
            .and_downcast::<gtk::ScrolledWindow>()
            .map(|s| s.vadjustment())
            .unwrap_or_default()
    }

    /// The user left Home (a book was opened, another view was chosen): the hero visit is over.
    fn leave_home(&self) {
        nav::left_home(&mut self.hero.borrow_mut(), chrono::Utc::now().timestamp());
    }

    /// The dominant colour of the cover as banner colours (cached for the last cover).
    fn banner_palette(
        &self,
        cover: Option<&std::path::Path>,
    ) -> Option<crate::cover_color::BannerColors> {
        let dark = adw::StyleManager::default().is_dark();
        let path = cover?.to_path_buf();
        if let Some((p, d, c)) = self.banner_colors.borrow().as_ref() {
            if *p == path && *d == dark {
                return *c;
            }
        }
        #[allow(deprecated)]
        let colors = gdk::gdk_pixbuf::Pixbuf::from_file_at_scale(&path, 32, 48, true)
            .ok()
            .and_then(|pb| {
                let (w, h, rs, n) = (
                    pb.width() as usize,
                    pb.height() as usize,
                    pb.rowstride() as usize,
                    pb.n_channels() as usize,
                );
                let bytes = pb.read_pixel_bytes();
                let mut px = Vec::with_capacity(w * h);
                for y in 0..h {
                    for x in 0..w {
                        let i = y * rs + x * n;
                        if let Some(p) = bytes.get(i..i + 3) {
                            px.push([p[0], p[1], p[2]]);
                        }
                    }
                }
                crate::cover_color::dominant(&px)
            })
            .map(|c| crate::cover_color::banner_colors(c, dark));
        *self.banner_colors.borrow_mut() = Some((path, dark, colors));
        colors
    }

    fn update_banner(&self) {
        let lib = self.shared.lib.borrow();
        let mut reading: Vec<&Book> = lib
            .books
            .iter()
            .filter(|b| b.last_read.is_some() && b.progress < 0.98)
            .collect();
        reading.sort_by_key(|b| std::cmp::Reverse(b.last_read));
        let last = reading.first().copied();
        let ctl = self.w.banner_ctl.clone();
        ctl.no_motion.set(self.shared.cfg.borrow().eink);
        ctl.stack
            .set_transition_type(if self.shared.cfg.borrow().eink {
                gtk::StackTransitionType::None
            } else {
                gtk::StackTransitionType::Crossfade
            });
        let mode = self.shared.cfg.borrow().banner_mode.clone();
        let show = last.is_some()
            && mode != "hidden"
            && self.query.trim().is_empty()
            && self.shared.cfg.borrow().view_mode == ViewMode::Grid
            && self.view != View::Notes;
        let Some(b) = last.filter(|_| show) else {
            ctl.set_shown(false, nav::BannerKind::Strip, &mode);
            return;
        };
        let now = chrono::Utc::now().timestamp();
        let kind = nav::banner_kind(&self.hero.borrow(), now);
        if kind == nav::BannerKind::Hero {
            nav::hero_shown(&mut self.hero.borrow_mut(), now);
        }
        let cover_path = lib.cover_path(b);
        let data = super::library_banner::BannerData {
            id: b.id,
            title: b.meta.title.clone(),
            authors: b.meta.authors_display(),
            progress: b.progress,
            cover: cover_path
                .as_deref()
                .and_then(|p| covers::thumbnail(p, 200)),
            chapter: b
                .position
                .map(|p| trf("Chapter {}", &[&(p.spine + 1)]))
                .unwrap_or_default(),
            left: nav::time_left_text(b.size, b.progress),
            others: reading
                .iter()
                .skip(1)
                .take(3)
                .map(|o| {
                    (
                        o.id,
                        o.meta.title.clone(),
                        lib.cover_path(o)
                            .as_deref()
                            .and_then(|p| covers::thumbnail(p, 160)),
                        o.progress,
                    )
                })
                .collect(),
        };
        drop(lib);
        let tx = self.sender.input_sender().clone();
        ctl.set_data(
            &data,
            Rc::new(move |id, what| tx.emit(Input::BannerAct(id, what))),
        );
        ctl.set_colors(self.banner_palette(cover_path.as_deref()));
        ctl.set_shown(true, kind, &mode);
    }

    /// What the shelves section and its dialogs need from this window.
    fn shelf_ctx(&self) -> super::shelves::Ctx {
        let (t1, t2, t3) = (
            self.sender.input_sender().clone(),
            self.sender.input_sender().clone(),
            self.sender.input_sender().clone(),
        );
        super::shelves::Ctx {
            shared: self.shared.clone(),
            search: self.w.search.clone(),
            go: self.sidebar_go(),
            parent: self.root.clone().upcast(),
            refresh: Rc::new(move || t1.emit(Input::Refresh)),
            toast: Rc::new(move |m| t2.emit(Input::Toast(m))),
            undo: Rc::new(move |m, u| t3.emit(Input::ToastUndo(m, u))),
        }
    }

    /// What a click on a shelf, saved search or browse entry does: back to the Home view, then
    /// the search (the narrow window's sidebar overlay closes).
    fn sidebar_go(&self) -> Rc<dyn Fn(String)> {
        let tx = self.sender.input_sender().clone();
        Rc::new(move |q: String| tx.emit(Input::GoFilter(q)))
    }

    /// The counts next to Home, Recent, Favourites and Notes & Highlights.
    fn update_top_counts(&self) {
        let lib = self.shared.lib.borrow();
        // the same rows the Notes & Highlights view lists, so the count never disagrees with it
        let notes: usize = crate::notes_view::rows(&lib).len();
        let counts = [
            lib.books.len(),
            lib.books.len().min(nav::RECENT_LIMIT),
            self.shared.cfg.borrow().favourites.len(),
            notes,
        ];
        for (l, n) in self.w.top_counts.iter().zip(counts) {
            l.set_label(&n.to_string());
        }
        let weak = lib
            .books
            .iter()
            .filter(|b| !crate::core::quality::weak_reasons(b).is_empty())
            .count();
        self.w.health_badge.set_label(&weak.to_string());
        self.w.health_badge.set_visible(weak > 0);
    }

    /// Marks the sidebar row of the view that is shown.
    fn select_top_row(&self) {
        let i = View::ALL.iter().position(|v| *v == self.view).unwrap_or(0);
        if let Some(row) = self.w.top.row_at_index(i as i32) {
            self.w.top.select_row(Some(&row));
        }
    }

    /// The shelves, saved searches and browse groups (series, authors, tags…) under the fixed
    /// entries. Rebuilt when the library changes, not when a view is picked.
    fn rebuild_tags(&self) {
        self.update_top_counts();
        while let Some(c) = self.w.tags.first_child() {
            self.w.tags.remove(&c);
        }
        let list = gtk::ListBox::new();
        list.add_css_class("navigation-sidebar");
        list.set_selection_mode(gtk::SelectionMode::None);
        let go = self.sidebar_go();

        let lib = self.shared.lib.borrow();
        list.append(&super::shelves::section(&self.shelf_ctx()));
        if let Some(row) = super::audiobook_player::sidebar_row(&lib, go.clone()) {
            list.append(&row);
        }

        let (saved, active_vl) = {
            let c = self.shared.cfg.borrow();
            (c.saved_searches.clone(), c.virtual_library.clone())
        };
        {
            let exp = adw::ExpanderRow::builder()
                .title(tr("Saved Searches"))
                .build();
            exp.set_widget_name("saved-searches");
            exp.add_suffix(&count_label(saved.len()));
            exp.add_prefix(&gtk::Image::from_icon_name("system-search-symbolic"));
            let manage = gtk::Button::from_icon_name("view-more-symbolic");
            manage.add_css_class("flat");
            manage.add_css_class("row-actions");
            manage.set_valign(gtk::Align::Center);
            manage.set_tooltip_text(Some(&tr("Manage Saved Searches")));
            super::audit::label(&manage, &tr("Manage Saved Searches"));
            {
                let tx = self.sender.input_sender().clone();
                manage.connect_clicked(move |_| tx.emit(Input::SavedSearches));
            }
            exp.add_suffix(&manage);
            exp.set_expanded(true);
            for s in saved.iter().cloned() {
                let is_vl = active_vl
                    .as_deref()
                    .is_some_and(|a| a.eq_ignore_ascii_case(&s.name));
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(&s.name))
                    .activatable(true)
                    .build();
                if is_vl {
                    row.add_suffix(&gtk::Image::from_icon_name("view-pin-symbolic"));
                    row.set_tooltip_text(Some(&tr("Virtual Library")));
                }
                let (go, q) = (go.clone(), crate::saved_searches::reference_for(&s.name));
                row.connect_activated(move |_| go(q.clone()));
                exp.add_row(&row);
            }
            if saved.is_empty() {
                let hint = adw::ActionRow::builder()
                    .title(tr("No saved searches yet"))
                    .subtitle(tr("Use the search tips in the search bar to save one"))
                    .build();
                hint.add_css_class("placeholder-row");
                exp.add_row(&hint);
            }
            list.append(&exp);
        }

        for (ci, cat) in Category::ALL.iter().enumerate() {
            let mut items = lib.categories(*cat);
            if items.is_empty() {
                continue;
            }
            if *cat == Category::Ratings {
                // always 5 stars .. 1 star and Unrated, even when a row has no books yet
                let unrated = "☆".repeat(5);
                items = (0..=5)
                    .rev()
                    .map(|n| {
                        let key = if n == 0 {
                            unrated.clone()
                        } else {
                            "★".repeat(n) + "☆".repeat(5 - n).as_str()
                        };
                        let c = items.iter().find(|(k, _)| *k == key).map_or(0, |(_, c)| *c);
                        (key, c)
                    })
                    .collect();
            }
            let exp = adw::ExpanderRow::builder().title(tr(cat.label())).build();
            exp.set_widget_name(&format!("group-{}", cat.label()));
            exp.add_suffix(&count_label(items.len()));
            exp.add_prefix(&gtk::Image::from_icon_name(cat.icon()));
            let library_key = lib.root.to_string_lossy().to_string();
            exp.set_expanded(
                self.expanded.borrow().contains(&ci)
                    || nav::is_open(
                        &self.shared.cfg.borrow().sidebar_open,
                        &library_key,
                        cat.label(),
                    ),
            );
            {
                let set = self.expanded.clone();
                let shared = self.shared.clone();
                let (library_key, group) = (library_key.clone(), cat.label());
                exp.connect_expanded_notify(move |e| {
                    if e.is_expanded() {
                        set.borrow_mut().insert(ci);
                    } else {
                        set.borrow_mut().remove(&ci);
                    }
                    let mut c = shared.cfg.borrow_mut();
                    let mut open = std::mem::take(&mut c.sidebar_open);
                    nav::set_open(&mut open, &library_key, group, e.is_expanded());
                    c.sidebar_open = open;
                    c.save();
                });
            }
            let overview = match cat {
                Category::Series => Some(("series", tr("Series overview"))),
                Category::Authors => Some(("authors", tr("Manage authors"))),
                _ => None,
            };
            if let Some((which, tip)) = overview {
                let b = gtk::Button::from_icon_name("view-more-symbolic");
                b.add_css_class("flat");
                b.add_css_class("row-actions");
                b.set_valign(gtk::Align::Center);
                b.set_tooltip_text(Some(&tip));
                super::audit::label(&b, &tip);
                let tx = self.sender.input_sender().clone();
                b.connect_clicked(move |_| tx.emit(Input::Organize(which)));
                exp.add_suffix(&b);
            }
            // the most used entries; the rest are behind "Show All"
            let all_items = items.clone();
            let (items, hidden) = if matches!(cat, Category::Ratings | Category::Status) {
                (items, 0)
            } else {
                nav::top_n(items, nav::SIDEBAR_TOP)
            };
            for (name, count) in items.into_iter() {
                let shown = if *cat == Category::Ratings && !name.contains('★') {
                    tr("Unrated")
                } else {
                    name.clone()
                };
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(&shown))
                    .activatable(true)
                    .build();
                row.add_suffix(&count_label(count));
                let (go, q) = (go.clone(), cat.search_for(&name));
                row.connect_activated(move |_| go(q.clone()));
                exp.add_row(&row);
            }
            if hidden > 0 {
                let cat = *cat;
                exp.add_row(&extra::show_all_row(
                    all_items,
                    hidden,
                    go.clone(),
                    Rc::new(move |n: &str| cat.search_for(n)),
                ));
            }
            list.append(&exp);
        }
        self.w.tags.append(&list);
    }

    fn update_details(&self) {
        // nothing selected: the panel slides out and keeps what it showed until it is gone
        if self.selected.is_empty() {
            return;
        }
        let d = &self.w.details;
        while let Some(c) = d.first_child() {
            d.remove(&c);
        }
        let lib = self.shared.lib.borrow();
        match self.selected.len() {
            0 => {}
            1 => {
                let Some(b) = lib.get(self.selected[0]) else {
                    return;
                };
                let id = b.id;
                let pic = gtk::Picture::new();
                pic.set_size_request(-1, 260);
                pic.set_content_fit(gtk::ContentFit::Contain);
                pic.add_css_class("details-cover");
                if let Some(t) = lib.cover_path(b).and_then(|p| covers::thumbnail(&p, 300)) {
                    pic.set_paintable(Some(&t));
                }
                d.append(&pic);

                let title = gtk::Label::new(Some(&b.meta.title));
                title.add_css_class("title-2");
                title.set_wrap(true);
                title.set_justify(gtk::Justification::Center);
                title.set_selectable(true);
                // a selectable label that is the first thing a focus move reaches makes GTK log
                // "gtk_widget_is_ancestor: assertion 'GTK_IS_WIDGET (widget)' failed" (the mouse still selects)
                title.set_focusable(false);
                d.append(&title);
                let authors = gtk::Label::new(Some(&b.meta.authors_display()));
                authors.add_css_class("dim-label");
                authors.set_wrap(true);
                authors.set_justify(gtk::Justification::Center);
                d.append(&authors);
                if !b.meta.series.is_empty() {
                    let s = gtk::Label::new(Some(&format!(
                        "{} #{}",
                        b.meta.series,
                        crate::core::opf::format_index(b.meta.series_index)
                    )));
                    s.add_css_class("caption");
                    d.append(&s);
                }

                if let Some(banner) = crate::ui::formats::protected_banner(b) {
                    d.append(&banner);
                }
                let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                buttons.set_halign(gtk::Align::Center);
                buttons.set_margin_top(6);
                let read = gtk::Button::with_label(
                    &(if b.progress > 0.0 && b.progress < 0.98 {
                        tr("Continue")
                    } else {
                        tr("Read")
                    }),
                );
                read.add_css_class("suggested-action");
                read.add_css_class("pill");
                let edit = gtk::Button::with_label(&tr("Edit Metadata"));
                edit.add_css_class("pill");
                let tx = self.sender.input_sender().clone();
                read.connect_clicked(move |_| tx.emit(Input::Activate(id)));
                let tx = self.sender.input_sender().clone();
                edit.connect_clicked(move |_| tx.emit(Input::Act(Act::EditMeta)));
                let is_fav = self.shared.cfg.borrow().favourites.contains(&id);
                let star = gtk::ToggleButton::new();
                star.add_css_class("circular");
                star.add_css_class("fav-toggle");
                star.set_active(is_fav);
                star.set_icon_name(if is_fav {
                    "starred-symbolic"
                } else {
                    "non-starred-symbolic"
                });
                let star_tip = if is_fav {
                    tr("Favourite: click to remove from Favourites")
                } else {
                    tr("Add to Favourites")
                };
                star.set_tooltip_text(Some(&star_tip));
                super::audit::label(&star, &tr("Favourite"));
                let tx = self.sender.input_sender().clone();
                star.connect_clicked(move |_| tx.emit(Input::ToggleFav(id)));
                buttons.append(&read);
                buttons.append(&edit);
                buttons.append(&star);
                d.append(&buttons);

                let shown_progress = self
                    .groups
                    .get(&b.id)
                    .and_then(|c| lib.get(c.progress_from))
                    .map_or(b.progress, |x| x.progress);
                if shown_progress > 0.0 {
                    let bar = gtk::ProgressBar::new();
                    bar.set_fraction(shown_progress);
                    bar.set_margin_top(8);
                    bar.set_tooltip_text(Some(&trf(
                        "{}% read",
                        &[&format!("{:.0}", shown_progress * 100.0)],
                    )));
                    d.append(&bar);
                }

                if let Some(g) = self.groups.get(&b.id).and_then(|c| {
                    super::library_works::details_group(&lib, c, &self.edition_actions())
                }) {
                    d.append(&g);
                }
                d.append(&self.rating_row(&[id], b.meta.rating, true));
                d.append(&self.tags_editor(&[id], &b.meta.tags, true));
                let props = gtk::Button::with_label(&tr("Properties"));
                props.add_css_class("flat");
                props.set_halign(gtk::Align::Center);
                props.set_tooltip_text(Some(&tr("Every Detail of This Book")));
                let tx = self.sender.input_sender().clone();
                props.connect_clicked(move |_| tx.emit(Input::Act(Act::Properties)));
                d.append(&props);
                let group = adw::PreferencesGroup::new();
                group.set_margin_top(10);
                let row = |t: &str, v: &str| {
                    if v.trim().is_empty() {
                        return;
                    }
                    let r = adw::ActionRow::builder()
                        .title(t)
                        .subtitle(glib::markup_escape_text(v))
                        .subtitle_selectable(true)
                        .build();
                    r.add_css_class("property");
                    group.add(&r);
                };
                row(&tr("Format"), &trf("EPUB · {}", &[&human_size(b.size)]));
                row(&tr("Language"), &b.meta.language);
                row(&tr("Tags"), &b.meta.tags.join(", "));
                if let Some(l) = b.last_read {
                    row(&tr("Last read"), &l.format("%d %b %Y, %H:%M").to_string());
                }
                d.append(&group);
                if let Some(g) =
                    crate::ui::formats::details_group(&self.shared, b, &self.format_callbacks())
                {
                    d.append(&g);
                }
                {
                    let tx = self.sender.output_sender().clone();
                    let play: Rc<dyn Fn(u64)> = Rc::new(move |id| {
                        let _ = tx.send(Output::PlayAudio(id));
                    });
                    if let Some(g) = super::audiobook_player::details_group(&self.shared, b, play) {
                        d.append(&g);
                    }
                }
                if let Some(g) =
                    crate::ui::audiobooks::details_group(b, self.health_notify(), &self.shared)
                {
                    d.append(&g);
                }

                if !b.meta.description.trim().is_empty() {
                    let heading = gtk::Label::new(Some(&tr("Comments")));
                    heading.add_css_class("heading");
                    heading.set_halign(gtk::Align::Start);
                    heading.set_margin_top(8);
                    d.append(&heading);
                    let text =
                        crate::core::html::parse_html(&b.meta.description, "d.html").to_text();
                    let l = gtk::Label::new(Some(text.trim()));
                    l.set_wrap(true);
                    l.set_xalign(0.0);
                    l.set_selectable(true);
                    d.append(&l);
                }
            }
            n => {
                let total: u64 = self
                    .selected
                    .iter()
                    .filter_map(|id| lib.get(*id))
                    .map(|b| b.size)
                    .sum();
                let s = adw::StatusPage::builder()
                    .icon_name("view-list-symbolic")
                    .title(trf("{} Books Selected", &[&n]))
                    .description(human_size(total))
                    .build();
                s.add_css_class("compact");
                d.append(&s);
                d.append(&self.rating_row(&self.selected_books(), 0, true));
                d.append(&self.tags_editor(&self.selected_books(), &[], true));
                let group = adw::PreferencesGroup::new();
                let favs = self.shared.cfg.borrow().favourites.clone();
                let all_fav = self.selected.iter().all(|i| favs.contains(i));
                for (label, icon, action) in [
                    (
                        tr("Edit Metadata in Bulk"),
                        "document-edit-symbolic",
                        "book.edit-meta",
                    ),
                    (
                        tr("Fetch Metadata"),
                        "folder-download-symbolic",
                        "book.fetch-meta",
                    ),
                    (
                        if all_fav {
                            tr("Remove from Favourites")
                        } else {
                            tr("Add to Favourites")
                        },
                        "starred-symbolic",
                        "book.favourite",
                    ),
                    (
                        tr("Mark as Read"),
                        "object-select-symbolic",
                        "book.mark-read",
                    ),
                    (
                        tr("Mark as Unread"),
                        "edit-undo-symbolic",
                        "book.mark-unread",
                    ),
                    (tr("Send by E-mail"), "mail-send-symbolic", "book.email"),
                    (tr("Move to Trash"), "user-trash-symbolic", "book.delete"),
                ] {
                    let r = adw::ActionRow::builder()
                        .title(&label)
                        .activatable(true)
                        .action_name(action)
                        .build();
                    r.add_prefix(&gtk::Image::from_icon_name(icon));
                    group.add(&r);
                }
                d.append(&group);
            }
        }
    }

    /// A "Rating" line with the five stars for these books (one book: its rating; several: sets all).
    fn rating_row(&self, ids: &[u64], value: u8, in_panel: bool) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.set_margin_top(6);
        row.set_halign(gtk::Align::Center);
        let l = gtk::Label::new(Some(&if ids.len() > 1 {
            tr("Rate all")
        } else {
            tr("Rating")
        }));
        l.add_css_class("dim-label");
        row.append(&l);
        let (tx, ids) = (self.sender.input_sender().clone(), ids.to_vec());
        row.append(&super::meta_edit::rating_stars(
            value,
            Rc::new(move |v| tx.emit(Input::SetRating(ids.clone(), v, in_panel))),
        ));
        row
    }

    /// The tag editor for these books. One book: its tags as chips. Several: the tags of any of
    /// them with how many books have each (clicking one removes it from all) and an entry that
    /// adds a tag to all of them.
    fn tags_editor(&self, ids: &[u64], tags: &[String], in_panel: bool) -> gtk::Box {
        let all: Vec<String> = self
            .shared
            .lib
            .borrow()
            .categories(Category::Tags)
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        let col = gtk::Box::new(gtk::Orientation::Vertical, 4);
        col.set_margin_top(10);
        let head = gtk::Label::new(Some(&if ids.len() > 1 {
            tr("Tags of the selected books")
        } else {
            tr("Tags")
        }));
        head.add_css_class("heading");
        head.set_xalign(0.0);
        col.append(&head);
        let tx = self.sender.input_sender().clone();
        let ids_v = ids.to_vec();
        if ids.len() <= 1 {
            let old = Rc::new(RefCell::new(tags.to_vec()));
            col.append(&super::meta_edit::tag_editor(
                tags,
                all,
                Rc::new(move |new| {
                    let before = old.borrow().clone();
                    let add: Vec<String> = new
                        .iter()
                        .filter(|t| !crate::tags::has(&before, t))
                        .cloned()
                        .collect();
                    let remove: Vec<String> = before
                        .iter()
                        .filter(|t| !crate::tags::has(&new, t))
                        .cloned()
                        .collect();
                    *old.borrow_mut() = new;
                    tx.emit(Input::EditTags {
                        ids: ids_v.clone(),
                        add,
                        remove,
                        in_panel,
                    });
                }),
            ));
            return col;
        }
        // several books
        let lib = self.shared.lib.borrow();
        let lists: Vec<&[String]> = ids
            .iter()
            .filter_map(|i| lib.get(*i))
            .map(|b| b.meta.tags.as_slice())
            .collect();
        let chips = adw::WrapBox::new();
        chips.set_child_spacing(6);
        chips.set_line_spacing(6);
        for (t, n) in crate::tags::union_counts(&lists) {
            let b = gtk::Button::with_label(&format!("{t} · {n}/{}", ids.len()));
            b.add_css_class("pill");
            let tip = trf("Remove the tag {} from all the selected books", &[&t]);
            b.set_tooltip_text(Some(&tip));
            super::audit::label(&b, &tip);
            let (tx, ids) = (self.sender.input_sender().clone(), ids.to_vec());
            b.connect_clicked(move |_| {
                tx.emit(Input::EditTags {
                    ids: ids.clone(),
                    add: Vec::new(),
                    remove: vec![t.clone()],
                    in_panel,
                })
            });
            chips.append(&b);
        }
        col.append(&chips);
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some(&tr("Add a tag to all")));
        super::audit::label(&entry, &tr("Add a tag to all the selected books"));
        let (tx, ids) = (self.sender.input_sender().clone(), ids.to_vec());
        entry.connect_activate(move |e| {
            let add = crate::tags::parse_entry(&e.text());
            e.set_text("");
            if !add.is_empty() {
                tx.emit(Input::EditTags {
                    ids: ids.clone(),
                    add,
                    remove: Vec::new(),
                    in_panel,
                });
            }
        });
        col.append(&entry);
        col
    }

    /// Every detail of the selected book in a dialog.
    fn show_properties(&self) {
        let Some(win) = window_of(&self.root) else {
            return;
        };
        let (Some(id), lib) = (self.selected.first().copied(), self.shared.lib.borrow()) else {
            return;
        };
        let Some(b) = lib.get(id) else { return };
        let page = adw::PreferencesPage::new();
        let edit = adw::PreferencesGroup::new();
        edit.add(&self.rating_row(&[id], b.meta.rating, false));
        edit.add(&self.tags_editor(&[id], &b.meta.tags, false));
        page.add(&edit);
        let group = adw::PreferencesGroup::new();
        let row = |t: &str, v: &str| {
            if v.trim().is_empty() {
                return;
            }
            let r = adw::ActionRow::builder()
                .title(t)
                .subtitle(glib::markup_escape_text(v))
                .subtitle_selectable(true)
                .build();
            r.add_css_class("property");
            group.add(&r);
        };
        row(&tr("Title"), &b.meta.title);
        row(&tr("Authors"), &b.meta.authors_display());
        row(&tr("Format"), &trf("EPUB · {}", &[&human_size(b.size)]));
        row(&tr("Publisher"), &b.meta.publisher);
        row(
            &tr("Published"),
            &b.meta.published.chars().take(10).collect::<String>(),
        );
        row(&tr("Language"), &b.meta.language);
        row(&tr("Identifiers"), &b.meta.identifiers_string());
        row(&tr("Added"), &b.added.format("%d %b %Y, %H:%M").to_string());
        row(
            &tr("Modified"),
            &b.modified.format("%d %b %Y, %H:%M").to_string(),
        );
        if let Some(l) = b.last_read {
            row(&tr("Last read"), &l.format("%d %b %Y, %H:%M").to_string());
        }
        row(
            &tr("Annotations"),
            &if b.highlights.is_empty() && b.bookmarks.is_empty() {
                String::new()
            } else {
                format!(
                    "{} highlights · {} bookmarks",
                    b.highlights.len(),
                    b.bookmarks.len()
                )
            },
        );
        row(
            &tr("Sources"),
            &crate::library_health::provenance_summary(
                &self.shared.guard.borrow().store.provenance(id),
            ),
        );
        row(&tr("Path"), &lib.book_path(b).display().to_string());
        page.add(&group);
        if !b.meta.description.trim().is_empty() {
            let g = adw::PreferencesGroup::new();
            g.set_title(&tr("Comments"));
            let text = crate::core::html::parse_html(&b.meta.description, "d.html").to_text();
            let l = gtk::Label::new(Some(text.trim()));
            l.set_wrap(true);
            l.set_xalign(0.0);
            l.set_selectable(true);
            g.add(&l);
            page.add(&g);
        }
        let dialog = adw::Dialog::new();
        dialog.set_title(&tr("Properties"));
        dialog.set_content_width(560);
        dialog.set_content_height(640);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&page));
        dialog.set_child(Some(&view));
        dialog.present(Some(&win));
    }

    fn pick_files(&self, folder: bool, recursive: bool) {
        let tx = self.sender.input_sender().clone();
        let parent = window_of(&self.root);
        if folder {
            let dialog = gtk::FileDialog::builder()
                .title(tr("Add Books from Folder"))
                .build();
            dialog.select_folder(parent.as_ref(), gio::Cancellable::NONE, move |res| {
                if let Ok(f) = res {
                    if let Some(p) = f.path() {
                        let mut files = Vec::new();
                        crate::import::collect(&p, recursive, &mut files);
                        tx.emit(Input::ImportPaths(files));
                    }
                }
            });
        } else {
            let dialog = gtk::FileDialog::builder().title(tr("Add Books")).build();
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("E-books (EPUB, FB2, MOBI, DOCX, comics, text, …)"));
            for ext in crate::import::extensions() {
                filter.add_suffix(&ext);
            }
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            dialog.set_filters(Some(&filters));
            dialog.set_default_filter(Some(&filter));
            dialog.open_multiple(parent.as_ref(), gio::Cancellable::NONE, move |res| {
                if let Ok(list) = res {
                    let files: Vec<PathBuf> = (0..list.n_items())
                        .filter_map(|i| list.item(i).and_downcast::<gio::File>())
                        .filter_map(|f| f.path())
                        .collect();
                    tx.emit(Input::ImportPaths(files));
                }
            });
        }
    }

    /// The optional check before converting or e-mailing. Returns `true` when it took over: the
    /// window opens only if something was found, and *Continue* (or a clean result) runs `then`.
    fn preflight_gate(&self, ids: &[u64], dest: crate::preflight::Dest, then: Act) -> bool {
        if !self.shared.cfg.borrow().metadata_prefs.preflight {
            return false;
        }
        let Some(win) = window_of(&self.root) else {
            return false;
        };
        let tx = self.sender.input_sender().clone();
        // the follow-up acts on the same selection
        let go: Rc<dyn Fn()> = Rc::new(move || tx.emit(Input::Act(then.clone())));
        crate::ui::preflight::show(
            &win,
            &self.shared,
            ids.to_vec(),
            dest,
            Some(go),
            self.health_notify(),
        );
        true
    }

    /// Callbacks for the Series, Editions and Needs Attention windows.
    fn organize_callbacks(
        &self,
    ) -> (
        crate::ui::organize::Search,
        crate::ui::organize::Fetch,
        crate::ui::organize::Notify,
    ) {
        let search_entry = self.w.search.clone();
        let bar = self.w.search_bar.clone();
        let search: crate::ui::organize::Search = Rc::new(move |q| {
            bar.set_search_mode(true);
            search_entry.set_text(&q);
        });
        let tx = self.sender.input_sender().clone();
        let fetch: crate::ui::organize::Fetch = Rc::new(move |ids| tx.emit(Input::BulkFetch(ids)));
        (search, fetch, self.health_notify())
    }

    /// The books an action on `cards` reaches: a card with several editions stands for all of them.
    fn expanded(&self, cards: &[u64]) -> Vec<u64> {
        let mut out: Vec<u64> = Vec::new();
        for id in cards {
            match self.groups.get(id) {
                Some(c) => out.extend(c.members.iter().copied()),
                None => out.push(*id),
            }
        }
        out.dedup();
        out
    }

    /// The selection as the books it reaches (every edition of a selected card).
    fn selected_books(&self) -> Vec<u64> {
        self.expanded(&self.selected)
    }

    fn work_edit(&mut self, f: impl FnOnce(&bookx_store::SqliteStore) -> anyhow::Result<String>) {
        let res = f(&self.shared.guard.borrow().store);
        match res {
            Ok(msg) => {
                self.sender.input(Input::Refresh);
                self.sender.input(Input::Toast(msg));
            }
            Err(e) => self.sender.input(Input::Toast(trf(
                "Could not change the editions: {}",
                &[&e],
            ))),
        }
    }

    /// The `works=` developer commands.
    fn dev_works(&mut self, cmd: &str, sender: &ComponentSender<Self>) {
        let ids =
            |s: &str| -> Vec<u64> { s.split('+').filter_map(|x| x.trim().parse().ok()).collect() };
        match cmd.split_once(':').unwrap_or((cmd, "")) {
            ("on", _) => sender.input(Input::GroupEditions(true)),
            ("off", _) => sender.input(Input::GroupEditions(false)),
            ("link", a) => {
                self.selected = ids(a);
                self.do_act(Act::LinkEditions);
            }
            ("unlink", a) => sender.input(Input::EditionUnlink(a.parse().unwrap_or(0))),
            ("prefer", a) => sender.input(Input::EditionPrefer(a.parse().unwrap_or(0))),
            ("card", a) => {
                let (id, how) = a.split_once(':').unwrap_or((a, ""));
                let how = if how == "ctrl" {
                    nav::Pick::Ctrl
                } else {
                    nav::Pick::Plain
                };
                sender.input(Input::Pick(id.parse().unwrap_or(0), how));
            }
            ("show", a) => {
                let id: u64 = a.parse().unwrap_or(0);
                let w = self.card_widget(id);
                self.show_editions(id, &w);
                eprintln!(
                    "works: popover for {id} rows={}",
                    self.groups.get(&id).map_or(0, |c| c.members.len())
                );
            }
            // an edition row activated (Enter on it): `activate:first`, or the edition with that id
            ("activate", a) => {
                let want = format!("edition-{a}");
                // the edition list hangs on the grid
                let found = extra::find_named_by(self.grid.view.upcast_ref(), &|n| {
                    if a == "first" {
                        n.starts_with("edition-")
                    } else {
                        n == want
                    }
                })
                .and_then(|w| w.downcast::<gtk::ListBoxRow>().ok());
                match found {
                    Some(row) => {
                        eprintln!(
                            "works: row {} activatable={}",
                            row.widget_name(),
                            row.is_activatable()
                        );
                        row.activate();
                    }
                    None => eprintln!("works: no row for {a}"),
                }
            }
            ("cards", _) => {
                let mut g: Vec<String> = self
                    .groups
                    .values()
                    .map(|c| format!("{}={:?}", c.rep, c.members))
                    .collect();
                g.sort();
                eprintln!(
                    "works: cards={} books={} groups=[{}] subtitle={}",
                    self.card_ids.len(),
                    self.visible_ids().len(),
                    g.join(" "),
                    self.w.title.subtitle()
                );
            }
            ("selected", _) => eprintln!(
                "works: selected={:?} books={:?}",
                self.selected,
                self.selected_books()
            ),
            _ => eprintln!("works: unknown command {cmd}"),
        }
    }

    /// The widget of the card of `id` in the grid, or the whole view.
    fn card_widget(&self, id: u64) -> gtk::Widget {
        let view = self.grid.view.clone();
        let name = id.to_string();
        let mut c = view.first_child();
        while let Some(w) = c {
            if let Some(root) = w.first_child() {
                if root.widget_name() == name {
                    return root;
                }
            }
            c = w.next_sibling();
        }
        view.upcast()
    }

    fn edition_actions(&self) -> super::library_works::EditionActions {
        let tx = self.sender.input_sender().clone();
        let (t1, t2, t3) = (tx.clone(), tx.clone(), tx.clone());
        super::library_works::EditionActions {
            read: Rc::new(move |id| tx.emit(Input::Activate(id))),
            send: Rc::new(move |id| t1.emit(Input::EditionSend(id))),
            prefer: Rc::new(move |id| t2.emit(Input::EditionPrefer(id))),
            unlink: Rc::new(move |id| t3.emit(Input::EditionUnlink(id))),
            can_send: !self.devices.is_empty(),
        }
    }

    fn show_editions(&self, rep: u64, anchor: &gtk::Widget) {
        let Some(card) = self.groups.get(&rep) else {
            return;
        };
        super::library_works::show_popover(
            &self.shared.lib.borrow(),
            card,
            &self.edition_actions(),
            &if self.shared.cfg.borrow().view_mode == ViewMode::Grid {
                self.grid.view.clone().upcast::<gtk::Widget>()
            } else {
                self.table.view.clone().upcast::<gtk::Widget>()
            },
            anchor,
        );
    }

    fn do_act(&mut self, act: Act) {
        let ids = if act.reaches_editions() {
            self.selected_books()
        } else {
            self.selected.clone()
        };
        if ids.is_empty()
            && !matches!(
                act,
                Act::ImportAnn | Act::ExportAllAnn | Act::ExportCalibre | Act::Notes
            )
        {
            return;
        }
        let first = ids.first().copied().unwrap_or_default();
        let parent = window_of(&self.root);
        match act {
            Act::Read => {
                let _ = self.sender.output(Output::Read(first));
            }
            Act::EditBook => {
                let _ = self.sender.output(Output::EditBook(first));
            }
            Act::EditMeta => {
                let Some(parent) = parent else { return };
                if ids.len() == 1 {
                    let (md, cover) = {
                        let lib = self.shared.lib.borrow();
                        let Some(b) = lib.get(first) else { return };
                        let cover = lib.cover_path(b).and_then(|p| std::fs::read(p).ok());
                        (b.meta.clone(), cover)
                    };
                    let shared = self.shared.clone();
                    let tx = self.sender.input_sender().clone();
                    let before = md.clone();
                    metadata::set_series_context(crate::series_view::context(
                        &self.shared.lib.borrow().books,
                        Some(first),
                    ));
                    metadata::edit_dialog(&parent, &md, cover, move |new_md, cover_change| {
                        let res = (|| -> anyhow::Result<()> {
                            let mut lib = shared.lib.borrow_mut();
                            let path = lib.get(first).map(|b| lib.book_path(b));
                            if let Some(cp) =
                                path.and_then(|_| lib.get(first).and_then(|b| lib.cover_path(b)))
                            {
                                covers::forget(&cp);
                            }
                            match cover_change {
                                CoverChange::Keep => {}
                                CoverChange::Set(data, ext) => {
                                    let path = lib.get(first).map(|b| lib.book_path(b)).unwrap();
                                    let mut book = crate::core::book::OpenBook::open(&path)?;
                                    book.set_cover(data, &ext)?;
                                    book.archive.save(&path)?;
                                }
                                CoverChange::Remove => {
                                    let path = lib.get(first).map(|b| lib.book_path(b)).unwrap();
                                    let mut book = crate::core::book::OpenBook::open(&path)?;
                                    book.remove_cover()?;
                                    book.archive.save(&path)?;
                                }
                            }
                            let after = new_md.clone();
                            lib.update_metadata(first, new_md)?;
                            crate::library_health::record_edit(
                                &shared.guard.borrow().store,
                                first,
                                &before,
                                &after,
                                &metadata::take_fetch_sources(),
                            );
                            Ok(())
                        })();
                        match res {
                            Ok(()) => {
                                covers::clear();
                                tx.emit(Input::Refresh);
                                tx.emit(Input::Toast(tr("Metadata saved")));
                            }
                            Err(e) => {
                                tx.emit(Input::Toast(trf("Could not save metadata: {}", &[&e])))
                            }
                        }
                    });
                } else {
                    let shared = self.shared.clone();
                    let tx = self.sender.input_sender().clone();
                    let count = ids.len();
                    metadata::bulk_dialog(&parent, count, move |edit| {
                        let res = metadata::apply_bulk(&shared, &ids, &edit);
                        match res {
                            Ok(n) => {
                                tx.emit(Input::Refresh);
                                tx.emit(Input::Toast(trf("Updated {} books", &[&n])));
                            }
                            Err(e) => tx.emit(Input::Toast(trf("Bulk edit failed: {}", &[&e]))),
                        }
                    });
                }
            }
            Act::ShelfAdd(name) => {
                let t = super::shelves::add_books(&self.shared, &name, &ids);
                self.sender.input(Input::Toast(t));
                self.sender.input(Input::Refresh);
            }
            Act::ShelfNew => super::shelves::new_shelf_with(&self.shelf_ctx(), ids),
            Act::ShelfRemove => {
                if let Some(shelf) = super::shelves::current_shelf(&self.query) {
                    let t = super::shelves::remove_books(&self.shared, &shelf, &ids);
                    self.sender.input(Input::Toast(t));
                    self.sender.input(Input::Refresh);
                }
            }
            Act::ShelfMove(delta) => {
                if let Some(shelf) = super::shelves::current_shelf(&self.query) {
                    super::shelves::move_book(&self.shared, &shelf, first, delta);
                    self.sender.input(Input::Requery);
                }
            }
            Act::MarkRead(read) => {
                {
                    let mut lib = self.shared.lib.borrow_mut();
                    for id in &ids {
                        lib.mark_read(*id, read);
                    }
                    let _ = lib.save();
                }
                self.sender.input(Input::Requery);
            }
            Act::Delete => {
                // reversible: the books go to the Trash and the toast brings them back (no confirmation)
                let n = ids.len();
                let res = self.shared.lib.borrow_mut().remove(&ids, true);
                covers::clear();
                match res {
                    Ok(()) => {
                        let entries: Vec<(String, u64, i64)> = self
                            .shared
                            .lib
                            .borrow()
                            .trash()
                            .iter()
                            .map(|e| (e.name.clone(), e.book.id, e.at.timestamp_millis()))
                            .collect();
                        let names = nav::newest_trash_names(&entries, &ids);
                        let (shared, tx) =
                            (self.shared.clone(), self.sender.input_sender().clone());
                        let undo = crate::app::UndoFn::new(move || {
                            let mut lib = shared.lib.borrow_mut();
                            let back = names
                                .iter()
                                .filter(|n| lib.restore_from_trash(n).is_ok())
                                .count();
                            drop(lib);
                            covers::clear();
                            tx.emit(Input::Refresh);
                            tx.emit(Input::Toast(nav::restored_text(back)));
                        });
                        self.sender.input(Input::Selection(Vec::new()));
                        self.sender.input(Input::Refresh);
                        self.sender
                            .input(Input::ToastUndo(nav::moved_to_trash_text(n), undo));
                    }
                    Err(e) => self
                        .sender
                        .input(Input::Toast(trf("Could not move to Trash: {}", &[&e]))),
                }
            }
            Act::ShowFolder => {
                let lib = self.shared.lib.borrow();
                if let Some(dir) = lib
                    .get(first)
                    .map(|b| lib.book_path(b))
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                {
                    super::open_uri(&gio::File::for_path(dir).uri());
                }
            }
            Act::SaveCopy => {
                let lib = self.shared.lib.borrow();
                let Some(b) = lib.get(first) else { return };
                let name = format!("{}.epub", sanitize_filename(&b.meta.title));
                drop(lib);
                let dialog = gtk::FileDialog::builder()
                    .title(tr("Save a Copy"))
                    .initial_name(name)
                    .build();
                let shared = self.shared.clone();
                let tx = self.sender.input_sender().clone();
                dialog.save(parent.as_ref(), gio::Cancellable::NONE, move |res| {
                    if let Ok(f) = res {
                        if let Some(p) = f.path() {
                            match shared.lib.borrow().export_copy(first, &p) {
                                Ok(()) => tx.emit(Input::Toast(tr("Copy saved"))),
                                Err(e) => tx.emit(Input::Toast(trf("Could not save: {}", &[&e]))),
                            }
                        }
                    }
                });
            }
            Act::CopyCitation => {
                let text = {
                    let lib = self.shared.lib.borrow();
                    let books = lib.books_for(&ids);
                    bookx_sources::citation::render_books(
                        &books,
                        bookx_sources::citation::Style::Apa,
                    )
                };
                self.root.display().clipboard().set_text(&text);
                eprintln!("citation: {}", text.replace('\n', " / "));
                self.sender.input(Input::Toast(ntr(
                    "{} citation copied",
                    "{} citations copied",
                    ids.len(),
                )));
            }
            Act::ExportAnn(fmt) => self.export_many(ids, parent, move |lib, id| {
                let b = lib.get(id)?;
                let chapters = super::annotations::chapter_titles(lib, id);
                let items = crate::annotation_export::items_of(b, &chapters);
                Some((
                    crate::annotation_export::render(&items, &fmt, chrono::Utc::now())?,
                    crate::annotation_export::file_name(&b.meta.title, &fmt),
                ))
            }),
            Act::ImportAnn => {
                if let Some(p) = parent {
                    let (t1, t2) = (
                        self.sender.input_sender().clone(),
                        self.sender.input_sender().clone(),
                    );
                    super::annotations::start_import(
                        &p,
                        &self.shared,
                        Rc::new(move |m| t1.emit(Input::Toast(m))),
                        Rc::new(move || t2.emit(Input::Refresh)),
                    );
                }
            }
            Act::Notes => self.set_view(View::Notes),
            Act::Favourite => {
                let now = {
                    let mut c = self.shared.cfg.borrow_mut();
                    let on = nav::toggle_favourites(&mut c.favourites, &ids);
                    c.save();
                    on
                };
                sender_toast(
                    &self.sender,
                    if now {
                        trf(
                            "Added {} to Favourites",
                            &[&crate::library_health::plural(ids.len(), "book")],
                        )
                    } else {
                        trf(
                            "Removed {} from Favourites",
                            &[&crate::library_health::plural(ids.len(), "book")],
                        )
                    },
                );
                self.update_top_counts();
                // the badges on the covers follow
                self.requery();
            }
            Act::Properties => self.show_properties(),
            Act::LinkEditions => {
                if ids.len() < 2 {
                    self.sender
                        .input(Input::Toast(tr("Select at least two books to link")));
                    return;
                }
                let on = self.shared.cfg.borrow().group_editions;
                if !on {
                    let mut c = self.shared.cfg.borrow_mut();
                    c.group_editions = true;
                    c.save();
                }
                let n = ids.len();
                self.work_edit(move |s| {
                    s.link_editions(&ids)?;
                    Ok(trf("Linked {} books as editions of one work", &[&n]))
                });
            }
            Act::UnlinkEditions => {
                let n = ids.len();
                self.work_edit(move |s| {
                    for id in &ids {
                        s.unlink_edition(*id)?;
                    }
                    Ok(trf("Took {} books out of their group", &[&n]))
                });
            }
            Act::ShowEditionList => {
                if let Some(rep) = self.selected.first().copied() {
                    let w = self.card_widget(rep);
                    self.show_editions(rep, &w);
                }
            }
            Act::ExportAllAnn => {
                if let Some(p) = parent {
                    let t = self.sender.input_sender().clone();
                    super::annotations::start_export_all(
                        &p,
                        &self.shared,
                        Rc::new(move |m| t.emit(Input::Toast(m))),
                    );
                }
            }
            Act::ExportCalibre => {
                if let Some(p) = parent {
                    let t = self.sender.input_sender().clone();
                    super::calibre_export::start(
                        &p,
                        &self.shared,
                        Rc::new(move |m| t.emit(Input::Toast(m))),
                    );
                }
            }
            Act::Email => {
                if !self.preflight_gate(&ids, crate::preflight::Dest::Email, Act::EmailGo) {
                    self.email_books(ids);
                }
            }
            Act::EmailGo => self.email_books(ids),
            Act::ConvertTo(id) => {
                let dest = crate::preflight::Dest::for_target(&id);
                if !self.preflight_gate(&ids, dest, Act::ConvertToGo(id.clone())) {
                    if let Some(t) = crate::export::target(&id) {
                        self.convert_to(ids, parent, t);
                    }
                }
            }
            Act::ConvertToGo(id) => {
                if let Some(t) = crate::export::target(&id) {
                    self.convert_to(ids, parent, t);
                }
            }
            Act::FetchMeta => self.sender.input(Input::BulkFetch(ids)),
            Act::MergeFormats => {
                if let Some(win) = parent {
                    if ids.len() > 1 {
                        crate::ui::formats::show_merge(
                            &win,
                            &self.shared,
                            ids,
                            self.health_notify(),
                        );
                    }
                }
            }
            Act::Preflight => {
                if let Some(win) = parent {
                    crate::ui::preflight::show(
                        &win,
                        &self.shared,
                        ids,
                        crate::preflight::Dest::Generic,
                        None,
                        self.health_notify(),
                    );
                }
            }
            Act::Convert(fmt) => self.export_many(ids, parent, move |lib, id| {
                let b = lib.get(id)?;
                let book = lib.open_book(id).ok()?;
                Some((
                    export_book(&book, fmt),
                    format!("{}.{}", sanitize_filename(&b.meta.title), fmt.extension()),
                ))
            }),
        }
    }

    /// Mails the books to the address in the settings, on a worker thread.
    fn email_books(&self, ids: Vec<u64>) {
        let settings = self.shared.cfg.borrow().email.clone();
        if let Some(p) = settings.problem() {
            self.sender.input(Input::Toast(p));
            return;
        }
        let jobs: Vec<(PathBuf, String, String)> = {
            let lib = self.shared.lib.borrow();
            ids.iter()
                .filter_map(|id| {
                    lib.get(*id).map(|b| {
                        (
                            lib.path_for(b, crate::core::formats::Target::Generic),
                            b.meta.title.clone(),
                            b.meta.authors.join(" & "),
                        )
                    })
                })
                .collect()
        };
        let (done_tx, done_rx) = std::sync::mpsc::channel::<String>();
        let (job_id, _) = crate::jobs::start(
            crate::jobs::Kind::Send,
            &trf(
                "E-mailing {}",
                &[&crate::library_health::plural(jobs.len(), "book")],
            ),
            false,
        );
        std::thread::spawn(move || {
            let (mut ok, mut failed) = (0, Vec::new());
            for (path, title, author) in &jobs {
                match crate::emailing::send_book(&settings, path, title, author) {
                    Ok(_) => ok += 1,
                    Err(e) => failed.push(format!("{title}: {e:#}")),
                }
            }
            let _ = done_tx.send(if failed.is_empty() {
                trf(
                    "Sent {} to {}",
                    &[&ntr("{} book", "{} books", ok), &settings.to.trim()],
                )
            } else {
                trf("Could not send: {}", &[&failed.join("; ")])
            });
        });
        let tx = self.sender.input_sender().clone();
        glib::timeout_add_local(std::time::Duration::from_millis(60), move || match done_rx
            .try_recv()
        {
            Ok(msg) => {
                crate::jobs::log(job_id, &msg);
                crate::jobs::finish(
                    job_id,
                    if msg.starts_with("Could not") {
                        Err(msg.clone())
                    } else {
                        Ok(())
                    },
                );
                tx.emit(Input::Toast(msg));
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                crate::jobs::finish(job_id, Err("stopped".into()));
                glib::ControlFlow::Break
            }
        });
    }

    /// Converts the books to an e-book format on a worker thread and saves the results: a save
    /// dialog for one book, a folder chooser for several.
    fn convert_to(
        &self,
        ids: Vec<u64>,
        parent: Option<gtk::Window>,
        t: &'static crate::export::Target,
    ) {
        // (source path, title) of every book, read now because the worker must not touch the library
        let jobs: Vec<(PathBuf, String)> = {
            let lib = self.shared.lib.borrow();
            ids.iter()
                .filter_map(|id| {
                    lib.get(*id).map(|b| {
                        (
                            lib.path_for(b, crate::core::formats::Target::Convert),
                            b.meta.title.clone(),
                        )
                    })
                })
                .collect()
        };
        if jobs.is_empty() {
            return;
        }
        let report = self.shared.cfg.borrow().metadata_prefs.conversion_report;
        let tx = self.sender.input_sender().clone();
        let run = move |dest_for: Box<dyn Fn(&str) -> PathBuf + Send>,
                        jobs: Vec<(PathBuf, String)>| {
            let (done_tx, done_rx) = std::sync::mpsc::channel::<String>();
            let (prog_tx, prog_rx) = std::sync::mpsc::channel::<String>();
            let total = jobs.len();
            let (job_id, token) = crate::jobs::start(
                crate::jobs::Kind::Convert,
                &trf(
                    "Converting {} to {}",
                    &[&crate::library_health::plural(total, "book"), &t.label],
                ),
                true,
            );
            std::thread::spawn(move || {
                let (mut ok, mut failed) = (0, Vec::new());
                for (path, title) in &jobs {
                    if token.is_cancelled() {
                        let _ = prog_tx.send("Cancelled".into());
                        break;
                    }
                    let _ = prog_tx.send(trf("Converting {}", &[&title]));
                    let res = crate::export::convert_epub_file(path, t).and_then(|bytes| {
                        let dest = dest_for(title);
                        // with a report next to the result: what the conversion found
                        crate::preflight::write_converted(&dest, title, t, path, &bytes, report)?;
                        Ok(dest)
                    });
                    match res {
                        Ok(_) => ok += 1,
                        Err(e) => failed.push(format!("{title}: {e:#}")),
                    }
                    let _ = prog_tx.send(format!("\u{1}{}", ok + failed.len()));
                }
                let _ = done_tx.send(if failed.is_empty() {
                    if jobs.len() == 1 {
                        trf("Converted to {}", &[&t.label])
                    } else {
                        trf("Converted {} books to {}", &[&ok, &t.label])
                    }
                } else {
                    trf("Could not convert: {}", &[&failed.join("; ")])
                });
            });
            let tx = tx.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(60), move || {
                while let Ok(line) = prog_rx.try_recv() {
                    match line
                        .strip_prefix('\u{1}')
                        .and_then(|n| n.parse::<usize>().ok())
                    {
                        Some(n) => crate::jobs::progress(
                            job_id,
                            Some(n as f64 / total.max(1) as f64),
                            &format!("{n} of {total}"),
                        ),
                        None => crate::jobs::log(job_id, &line),
                    }
                }
                match done_rx.try_recv() {
                    Ok(msg) => {
                        crate::jobs::log(job_id, &msg);
                        crate::jobs::finish(
                            job_id,
                            if msg.starts_with("Could not") {
                                Err(msg.clone())
                            } else {
                                Ok(())
                            },
                        );
                        tx.emit(Input::Toast(msg));
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        crate::jobs::finish(job_id, Err("stopped".into()));
                        glib::ControlFlow::Break
                    }
                }
            });
        };
        if jobs.len() == 1 {
            let name = crate::export::file_name(&jobs[0].1, t);
            let dialog = gtk::FileDialog::builder()
                .title(trf("Convert to {}", &[&t.label]))
                .initial_name(name)
                .build();
            dialog.save(parent.as_ref(), gio::Cancellable::NONE, move |res| {
                if let Some(p) = res.ok().and_then(|f| f.path()) {
                    run(Box::new(move |_| p.clone()), jobs);
                }
            });
        } else {
            let dialog = gtk::FileDialog::builder()
                .title(tr("Choose a folder for the converted books"))
                .build();
            dialog.select_folder(parent.as_ref(), gio::Cancellable::NONE, move |res| {
                if let Some(dir) = res.ok().and_then(|f| f.path()) {
                    run(
                        Box::new(move |title| dir.join(crate::export::file_name(title, t))),
                        jobs,
                    );
                }
            });
        }
    }

    /// Generates one text file per book: a save dialog for one, a folder chooser for many.
    fn export_many(
        &self,
        ids: Vec<u64>,
        parent: Option<gtk::Window>,
        gen: impl Fn(&crate::core::library::Library, u64) -> Option<(String, String)> + 'static,
    ) {
        let shared = self.shared.clone();
        let tx = self.sender.input_sender().clone();
        let gen = Rc::new(gen);
        if ids.len() == 1 {
            let Some((text, name)) = gen(&shared.lib.borrow(), ids[0]) else {
                tx.emit(Input::Toast(tr("Could not read the book")));
                return;
            };
            let dialog = gtk::FileDialog::builder()
                .title(tr("Export"))
                .initial_name(name)
                .build();
            dialog.save(parent.as_ref(), gio::Cancellable::NONE, move |res| {
                if let Ok(f) = res {
                    if let Some(p) = f.path() {
                        match std::fs::write(&p, text) {
                            Ok(()) => tx.emit(Input::Toast(trf("Saved {}", &[&p.display()]))),
                            Err(e) => tx.emit(Input::Toast(trf("Could not save: {}", &[&e]))),
                        }
                    }
                }
            });
        } else {
            let dialog = gtk::FileDialog::builder()
                .title(tr("Choose a folder for the exported files"))
                .build();
            dialog.select_folder(parent.as_ref(), gio::Cancellable::NONE, move |res| {
                if let Ok(f) = res {
                    if let Some(dir) = f.path() {
                        let mut n = 0;
                        for id in &ids {
                            if let Some((text, name)) = gen(&shared.lib.borrow(), *id) {
                                if std::fs::write(dir.join(name), text).is_ok() {
                                    n += 1;
                                }
                            }
                        }
                        tx.emit(Input::Toast(trf("Exported {} files", &[&n])));
                    }
                }
            });
        }
    }

    fn show_context_menu(&mut self, id: u64, x: f64, y: f64, widget: &gtk::Widget) {
        let (sel, changed) = nav::context_selection(&self.selected, id);
        if changed {
            self.selected = sel;
            self.anchor = Some(id);
            self.restore_selection();
            self.follow_selection();
            self.update_details();
            self.update_bottom_bar();
        }
        super::devices::set_library_selection(&self.selected_books());
        super::shelves::set_selection(&self.selected_books());
        let menu = self.book_menu(self.selected.len() > 1);
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.set_parent(widget);
        popover.set_has_arrow(false);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        popover.popup();
    }

    /// The "Convert To" formats: devices, plugin tools and text formats.
    fn convert_menu(&self) -> gio::Menu {
        let conv = gio::Menu::new();
        let devices = gio::Menu::new();
        for t in crate::export::TARGETS
            .iter()
            .chain(crate::plugins::writer_targets())
        {
            let item = gio::MenuItem::new(Some(&nav::plain_label(t.label)), None);
            item.set_action_and_target_value(Some("book.convert-to"), Some(&t.id.to_variant()));
            devices.append_item(&item);
        }
        conv.append_section(None, &devices);
        let plugin_tools = gio::Menu::new();
        for t in crate::plugins::tool_targets() {
            let item = gio::MenuItem::new(Some(&nav::plain_label(t.label)), None);
            item.set_action_and_target_value(Some("book.convert-to"), Some(&t.id.to_variant()));
            plugin_tools.append_item(&item);
        }
        if plugin_tools.n_items() > 0 {
            conv.append_section(None, &plugin_tools);
        }
        let text = gio::Menu::new();
        for f in ExportFormat::ALL {
            let item = gio::MenuItem::new(Some(f.label()), None);
            item.set_action_and_target_value(
                Some("book.convert"),
                Some(&f.extension().to_variant()),
            );
            text.append_item(&item);
        }
        conv.append_section(None, &text);
        conv
    }

    /// The overflow of the selection bar in a narrow window.
    fn more_menu(&self, actions: &[nav::BulkAction]) -> gio::Menu {
        let menu = gio::Menu::new();
        for a in actions {
            match a {
                nav::BulkAction::EditMetadata => {
                    menu.append(Some(&tr("Edit Metadata")), Some("book.edit-meta"))
                }
                nav::BulkAction::AddToShelf => menu.append_submenu(
                    Some(&tr("Add to Shelf")),
                    &super::shelves::add_to_shelf_menu(&self.shared),
                ),
                nav::BulkAction::SendToDevice => {
                    menu.append(Some(&tr("Send to Device")), Some("book.send-device"))
                }
                nav::BulkAction::Convert => {
                    menu.append_submenu(Some(&tr("Convert To")), &self.convert_menu())
                }
                _ => {}
            }
        }
        menu
    }

    /// The right-click menu: only what concerns the selected book(s), nothing about devices. At most
    /// 12 entries at the first level and no submenu inside a submenu:
    /// Read · Favourite · Edit ▸ · Add to Shelf ▸ · Reading Status ▸ · Send To ▸ · Convert To ▸ ·
    /// Export Annotations ▸ · Editions ▸ · Properties · Show in Files · Move to Trash.
    fn book_menu(&self, multi: bool) -> gio::Menu {
        let menu = gio::Menu::new();
        if !multi {
            let s = gio::Menu::new();
            s.append(Some(&tr("Read")), Some("book.read"));
            menu.append_section(None, &s);
        }

        let all_fav = {
            let favs = self.shared.cfg.borrow().favourites.clone();
            !self.selected.is_empty() && self.selected.iter().all(|i| favs.contains(i))
        };
        let s = gio::Menu::new();
        s.append(
            Some(
                &(if all_fav {
                    tr("Remove from Favourites")
                } else {
                    tr("Add to Favourites")
                }),
            ),
            Some("book.favourite"),
        );
        menu.append_section(None, &s);

        let s = gio::Menu::new();
        let edit = gio::Menu::new();
        edit.append(
            Some(
                &(if multi {
                    tr("Edit Metadata in Bulk")
                } else {
                    tr("Edit Metadata")
                }),
            ),
            Some("book.edit-meta"),
        );
        if !multi {
            edit.append(Some(&tr("Edit Book")), Some("book.edit-book"));
        }
        edit.append(Some(&tr("Fetch Metadata")), Some("book.fetch-meta"));
        s.append_submenu(Some(&tr("Edit")), &edit);

        // the shelves, and on a shelf also what only applies to it
        let on_shelf = super::shelves::current_shelf(&self.query).is_some();
        let shelf_menu = if on_shelf {
            let m = gio::Menu::new();
            m.append_section(
                Some(&tr("Add to Shelf")),
                &super::shelves::add_to_shelf_menu(&self.shared),
            );
            let here = gio::Menu::new();
            here.append(
                Some(&tr("Remove from This Shelf")),
                Some("book.shelf-remove"),
            );
            if !multi {
                here.append(
                    Some(&tr("Move Earlier on This Shelf")),
                    Some("book.shelf-earlier"),
                );
                here.append(
                    Some(&tr("Move Later on This Shelf")),
                    Some("book.shelf-later"),
                );
            }
            m.append_section(Some(&tr("This Shelf")), &here);
            m
        } else {
            super::shelves::add_to_shelf_menu(&self.shared)
        };
        s.append_submenu(
            Some(
                &(if on_shelf {
                    tr("Shelves")
                } else {
                    tr("Add to Shelf")
                }),
            ),
            &shelf_menu,
        );

        let reading = gio::Menu::new();
        reading.append(Some(&tr("Mark as Read")), Some("book.mark-read"));
        reading.append(Some(&tr("Mark as Unread")), Some("book.mark-unread"));
        s.append_submenu(Some(&tr("Reading Status")), &reading);

        let send = gio::Menu::new();
        // always there; usable while a device is connected
        let dev = gio::MenuItem::new(Some(&tr("Send to Device")), Some("book.send-device"));
        dev.set_attribute_value(
            "tooltip",
            Some(
                &(if self.devices.is_empty() {
                    "No device connected"
                } else {
                    "Open the Device window with these books ready to send"
                })
                .to_variant(),
            ),
        );
        send.append_item(&dev);
        send.append(Some(&tr("Send by E-mail")), Some("book.email"));
        send.append(Some(&tr("Check Before Sending")), Some("book.preflight"));
        send.append(Some(&tr("Copy Citation")), Some("book.copy-citation"));
        if !multi {
            send.append(Some(&tr("Save a Copy")), Some("book.save-copy"));
        }
        s.append_submenu(Some(&tr("Send To")), &send);
        s.append_submenu(Some(&tr("Convert To")), &self.convert_menu());

        let ann = gio::Menu::new();
        for f in crate::annotation_export::FORMATS
            .iter()
            .filter(|f| f.id != "readwise")
        {
            let item = gio::MenuItem::new(Some(f.label), None);
            item.set_action_and_target_value(Some("book.export-ann"), Some(&f.id.to_variant()));
            ann.append_item(&item);
        }
        s.append_submenu(Some(&tr("Export Annotations")), &ann);

        if multi || self.selected.iter().any(|id| self.groups.contains_key(id)) {
            let eds = gio::Menu::new();
            if multi {
                eds.append(
                    Some(&tr("Merge as Formats of One Book")),
                    Some("book.merge-formats"),
                );
                eds.append(Some(&tr("Link as Editions")), Some("book.link-editions"));
            }
            if self.selected.iter().any(|id| self.groups.contains_key(id)) {
                if !multi {
                    eds.append(Some(&tr("Show Editions")), Some("book.show-editions"));
                }
                eds.append(
                    Some(&tr("Unlink All Editions")),
                    Some("book.unlink-editions"),
                );
            }
            s.append_submenu(Some(&tr("Editions")), &eds);
        }
        menu.append_section(None, &s);

        if !multi {
            let s = gio::Menu::new();
            s.append(Some(&tr("Properties")), Some("book.properties"));
            s.append(Some(&tr("Show in Files")), Some("book.show-folder"));
            menu.append_section(None, &s);
        }
        let s = gio::Menu::new();
        s.append(Some(&tr("Move to Trash")), Some("book.delete"));
        menu.append_section(None, &s);
        menu
    }
}

use crate::i18n::ntr;
use crate::i18n::N_;

/// Names of `Category::label` (the core crate cannot translate): listed here so that they are extracted.
const _CATEGORY_NAMES: [&str; 7] = [
    N_("Authors"),
    N_("Series"),
    N_("Tags"),
    N_("Publishers"),
    N_("Languages"),
    N_("Ratings"),
    N_("Reading status"),
];
use crate::i18n::{tr, trf};

/// The entries of the main (hamburger) menu; an empty entry is a separator between groups. Eight
/// entries (Saved Searches live in the sidebar and in the search tips): the destinations (Reading Statistics, Vocabulary, ...) are in `LIBRARY_MENU`, imports in
/// the + menu, Copy Diagnostics in About.
const MAIN_MENU: [(&str, &str); 11] = [
    (N_("Open Library Folder"), "lib.open-folder"),
    ("", ""),
    (N_("Sync"), "app.sync"),
    (N_("Devices"), "app.devices"),
    ("", ""),
    (N_("Library Health"), "lib.health"),
    ("", ""),
    (N_("Preferences"), "app.preferences"),
    (N_("Keyboard Shortcuts"), "app.shortcuts"),
    (N_("Help"), "app.help"),
    (N_("About Bookx"), "app.about"),
];

/// The secondary Library menu in the content header: where to look, and the library-wide exports.
const LIBRARY_MENU: [(&str, &str); 9] = [
    (N_("Reading Statistics"), "app.stats"),
    (N_("Vocabulary"), "app.vocab"),
    (N_("Reading Trackers"), "lib.trackers"),
    (N_("Audiobooks"), "lib.audiobooks"),
    (N_("Online Catalogues"), "lib.opds"),
    (N_("Subscribe to News"), "lib.news"),
    ("", ""),
    (N_("Export as Calibre Library"), "lib.export-calibre"),
    (N_("Export Citations"), "lib.citations"),
];

/// A menu model from entries like `MAIN_MENU`: groups become sections, labels are translated and get
/// access keys that are unique inside the menu.
fn build_entry_menu(entries: &[(&str, &str)]) -> gio::Menu {
    let items: Vec<&(&str, &str)> = entries.iter().filter(|(l, _)| !l.is_empty()).collect();
    let labels: Vec<String> = items.iter().map(|(l, _)| tr(l)).collect();
    let keyed = nav::access_keys(&labels);
    let menu = gio::Menu::new();
    let mut section = gio::Menu::new();
    let mut i = 0;
    for (label, _) in entries {
        if label.is_empty() {
            if section.n_items() > 0 {
                menu.append_section(None, &section);
                section = gio::Menu::new();
            }
            continue;
        }
        section.append(Some(&keyed[i]), Some(items[i].1));
        i += 1;
    }
    if section.n_items() > 0 {
        menu.append_section(None, &section);
    }
    menu
}

/// The accessible name of the reverse-order button: it says which way the list is sorted now.
fn order_label(ascending: bool) -> String {
    if ascending {
        tr("Reverse order (now ascending)")
    } else {
        tr("Reverse order (now descending)")
    }
}

/// The attribute of the item with this label, searching submenus and sections.
fn menu_attr(m: &gio::MenuModel, label: &str, attr: &str) -> Option<String> {
    for i in 0..m.n_items() {
        if m.item_attribute_value(i, "label", None)
            .and_then(|v| v.str().map(String::from))
            .as_deref()
            == Some(label)
        {
            if let Some(v) = m.item_attribute_value(i, attr, None) {
                return v.str().map(String::from);
            }
        }
        let links = m.iterate_item_links(i);
        while let Some((_, sub)) = links.next() {
            if let Some(v) = menu_attr(&sub, label, attr) {
                return Some(v);
            }
        }
    }
    None
}

/// How many entries the menu shows at its first level (sections are flattened, submenus count as one).
fn menu_top_count(m: &gio::MenuModel) -> usize {
    let mut n = 0;
    for i in 0..m.n_items() {
        match m.item_link(i, "section") {
            Some(sec) => n += menu_top_count(&sec),
            None => n += 1,
        }
    }
    n
}

/// How deep submenus nest: 0 for a flat menu, 1 when it has submenus, 2 when a submenu has one.
fn menu_submenu_depth(m: &gio::MenuModel) -> usize {
    let mut depth = 0;
    for i in 0..m.n_items() {
        if let Some(sec) = m.item_link(i, "section") {
            depth = depth.max(menu_submenu_depth(&sec));
        }
        if let Some(sub) = m.item_link(i, "submenu") {
            depth = depth.max(1 + menu_submenu_depth(&sub));
        }
    }
    depth
}

/// `Label=N` for every submenu of a menu (its entries, sections flattened).
fn menu_submenu_sizes(m: &gio::MenuModel) -> String {
    let mut out = Vec::new();
    fn walk(m: &gio::MenuModel, out: &mut Vec<String>) {
        for i in 0..m.n_items() {
            if let Some(sec) = m.item_link(i, "section") {
                walk(&sec, out);
            }
            if let Some(sub) = m.item_link(i, "submenu") {
                let label = m
                    .item_attribute_value(i, "label", None)
                    .and_then(|v| v.str().map(String::from))
                    .unwrap_or_default();
                out.push(format!("{label}={}", menu_top_count(&sub)));
            }
        }
    }
    walk(m, &mut out);
    out.join(",")
}

/// Every label of a menu, submenus included (for the UI tests).
fn menu_labels(m: &gio::MenuModel, out: &mut Vec<String>) {
    for i in 0..m.n_items() {
        if let Some(l) = m
            .item_attribute_value(i, "label", None)
            .and_then(|v| v.str().map(String::from))
        {
            out.push(l);
        }
        let links = m.iterate_item_links(i);
        while let Some((_, sub)) = links.next() {
            menu_labels(&sub, out);
        }
    }
}

impl SimpleComponent for LibraryView {
    type Init = SharedRef;
    type Input = Input;
    type Output = Output;
    type Root = adw::NavigationPage;
    type Widgets = ();

    fn init_root() -> Self::Root {
        adw::NavigationPage::builder()
            .title("Bookx")
            .tag("library")
            .build()
    }

    fn init(
        shared: SharedRef,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let cfg = shared.cfg.borrow().clone();
        let cover_size = Rc::new(Cell::new(cfg.cover_size));

        // ---- typed views ----------------------------------------------------
        let grid: TypedGridView<BookItem, gtk::MultiSelection> = TypedGridView::new();
        // GtkGridView keeps `max_columns * 32 + 1` tiles alive (40 columns: 1281 bound tiles, most of the cost of a
        // rebuild): the cap follows the width, see `grid_columns::max_for`
        grid.view.set_max_columns(grid_columns::START_MAX);
        grid.view.set_min_columns(2);
        {
            let size = cover_size.clone();
            grid.view.add_tick_callback(move |view, _| {
                let want = grid_columns::max_for(view.width(), size.get(), view.max_columns());
                if want != view.max_columns() {
                    view.set_max_columns(want);
                }
                glib::ControlFlow::Continue
            });
        }
        grid.view.set_enable_rubberband(true);
        grid.view.add_css_class("book-grid");
        let mut table: TypedColumnView<BookRow, gtk::MultiSelection> = TypedColumnView::new();
        table.append_column::<ColFav>();
        table.append_column::<ColTitle>();
        table.append_column::<ColEditions>();
        table.append_column::<ColAuthors>();
        table.append_column::<ColSeries>();
        table.append_column::<ColRating>();
        table.append_column::<ColProgress>();
        table.append_column::<ColPublished>();
        table.append_column::<ColAdded>();
        table.append_column::<ColSize>();
        table.append_column::<ColTags>();
        table.view.set_show_row_separators(true);
        table.view.set_enable_rubberband(true);
        let table_cols: Vec<(&'static str, gtk::ColumnViewColumn)> = nav::TABLE_COLUMNS
            .iter()
            .filter_map(|(n, _, _)| table.get_columns().get(n).map(|c| (*n, c.clone())))
            .collect();
        let table_sync = Rc::new(Cell::new(false));
        // a click on a column header is a sort: the View options menu follows
        if let Some(cs) = table.view.sorter().and_downcast::<gtk::ColumnViewSorter>() {
            let (tx, cols, guard) = (
                sender.input_sender().clone(),
                table_cols.clone(),
                table_sync.clone(),
            );
            cs.connect_changed(move |s, _| {
                if guard.get() {
                    return;
                }
                let Some(col) = s.primary_sort_column() else {
                    return;
                };
                let Some(name) = cols.iter().find(|(_, c)| *c == col).map(|(n, _)| *n) else {
                    return;
                };
                if let Some(k) = nav::sort_for_column(name) {
                    tx.emit(Input::TableSorted(
                        k,
                        s.primary_sort_order() == gtk::SortType::Ascending,
                    ));
                }
            });
        }

        {
            let tx = sender.input_sender().clone();
            grid.selection_model
                .connect_selection_changed(move |m, _, _| {
                    let ids: Vec<u64> = (0..m.n_items())
                        .filter(|i| m.is_selected(*i))
                        .filter_map(|i| id_from_model(m.upcast_ref(), i))
                        .collect();
                    tx.emit(Input::Selection(ids));
                });
            let tx = sender.input_sender().clone();
            table
                .selection_model
                .connect_selection_changed(move |m, _, _| {
                    let ids: Vec<u64> = (0..m.n_items())
                        .filter(|i| m.is_selected(*i))
                        .filter_map(|i| id_from_model(m.upcast_ref(), i))
                        .collect();
                    tx.emit(Input::Selection(ids));
                });
            let tx = sender.input_sender().clone();
            let model = grid.selection_model.clone();
            grid.view.connect_activate(move |_, pos| {
                if let Some(id) = id_from_model(model.upcast_ref(), pos) {
                    tx.emit(Input::Activate(id));
                }
            });
            let tx = sender.input_sender().clone();
            let model = table.selection_model.clone();
            table.view.connect_activate(move |_, pos| {
                if let Some(id) = id_from_model(model.upcast_ref(), pos) {
                    tx.emit(Input::Activate(id));
                }
            });
        }

        // ---- pages ----------------------------------------------------------
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);

        let grid_scroll = gtk::ScrolledWindow::new();
        grid_scroll.set_vexpand(true);
        grid_scroll.set_child(Some(&grid.view));
        attach_empty_click(&grid_scroll, sender.input_sender().clone());
        // the banner is the top of the book list: the wheel shrinks it and slides it off first
        let banner_ctl = super::library_banner::BannerCtl::new(&grid_scroll);
        let banner = banner_ctl.reveal.clone();
        let grid_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        grid_page.append(&banner_ctl.holder);
        // on a very wide window the covers stop spreading
        let grid_clamp = adw::Clamp::new();
        grid_clamp.set_maximum_size(nav::GRID_MAX_WIDTH);
        grid_clamp.set_tightening_threshold(nav::GRID_MAX_WIDTH);
        grid_clamp.set_vexpand(true);
        grid_clamp.set_child(Some(&grid_scroll));
        grid_page.append(&grid_clamp);
        {
            let sh = shared.clone();
            banner_ctl.connect(
                &grid_page,
                Rc::new(move || sh.cfg.borrow().banner_mode.clone()),
            );
        }
        stack.add_named(&grid_page, Some("grid"));

        let table_scroll = gtk::ScrolledWindow::new();
        table_scroll.set_child(Some(&table.view));
        attach_empty_click(&table_scroll, sender.input_sender().clone());
        stack.add_named(&table_scroll, Some("table"));

        let empty = adw::StatusPage::builder()
            .icon_name("com.adhadse.Bookx")
            .title(tr("Welcome to Bookx"))
            .description(tr("Add some books to start your library.\nEPUB, PDF, MOBI, FB2, comics, audiobooks, plain text and more — drop them here."))
            .build();
        // the buttons wrap in a narrow window
        let empty_btns = gtk::FlowBox::new();
        empty_btns.set_selection_mode(gtk::SelectionMode::None);
        empty_btns.set_column_spacing(12);
        empty_btns.set_row_spacing(8);
        empty_btns.set_max_children_per_line(3);
        empty_btns.set_homogeneous(false);
        empty_btns.set_halign(gtk::Align::Center);
        let add_books = gtk::Button::with_label(&tr("Add Books"));
        add_books.add_css_class("suggested-action");
        add_books.add_css_class("pill");
        add_books.set_action_name(Some("lib.add-books"));
        let add_folder = gtk::Button::with_label(&tr("Add Folder"));
        add_folder.add_css_class("pill");
        add_folder.set_action_name(Some("lib.add-folder"));
        empty_btns.insert(&add_books, -1);
        empty_btns.insert(&add_folder, -1);
        let add_news = gtk::Button::with_label(&tr("News Subscriptions"));
        add_news.add_css_class("pill");
        add_news.set_tooltip_text(Some(&tr(
            "Turn news sites and blogs (RSS, Atom) into books that update by themselves",
        )));
        add_news.set_action_name(Some("lib.news"));
        empty_btns.insert(&add_news, -1);
        empty.set_child(Some(&empty_btns));
        stack.add_named(&empty, Some("empty"));

        let nomatch = adw::StatusPage::builder()
            .icon_name("edit-find-symbolic")
            .title(tr("No Results"))
            .description(tr("No book matches your search"))
            .build();
        stack.add_named(&nomatch, Some("nomatch"));
        // Recent and Favourites with nothing in them: say what fills them (the welcome page is for Home)
        let view_empty = adw::StatusPage::builder()
            .icon_name("view-list-symbolic")
            .build();
        stack.add_named(&view_empty, Some("viewempty"));

        // drag & drop of files
        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        {
            let tx = sender.input_sender().clone();
            drop.connect_drop(move |_, value, _, _| {
                if let Ok(list) = value.get::<gdk::FileList>() {
                    let mut files = Vec::new();
                    for f in list.files() {
                        if let Some(p) = f.path() {
                            if p.is_dir() {
                                let before = files.len();
                                crate::import::collect(&p, true, &mut files);
                                // a folder of audio and no e-books is one audiobook
                                if files.len() == before && crate::audiobook::is_audio_source(&p) {
                                    files.push(p);
                                }
                            } else if crate::import::is_supported(&p)
                                || crate::audiobook::is_audio_source(&p)
                            {
                                files.push(p);
                            }
                        }
                    }
                    tx.emit(Input::ImportPaths(files));
                    return true;
                }
                false
            });
        }
        // while files are dragged over the window the books say what a drop does
        let drop_hint = gtk::Box::new(gtk::Orientation::Vertical, 8);
        drop_hint.add_css_class("drop-hint");
        drop_hint.set_widget_name("drop-hint");
        drop_hint.set_halign(gtk::Align::Fill);
        drop_hint.set_valign(gtk::Align::Fill);
        drop_hint.set_can_target(false);
        drop_hint.set_visible(false);
        {
            let inner = gtk::Box::new(gtk::Orientation::Vertical, 8);
            inner.set_halign(gtk::Align::Center);
            inner.set_valign(gtk::Align::Center);
            inner.set_vexpand(true);
            let icon = gtk::Image::from_icon_name("folder-download-symbolic");
            icon.set_pixel_size(48);
            let text = gtk::Label::new(Some(&tr("Drop to Add")));
            text.add_css_class("title-2");
            inner.append(&icon);
            inner.append(&text);
            drop_hint.append(&inner);
        }
        {
            let hint = drop_hint.clone();
            drop.connect_current_drop_notify(move |t| hint.set_visible(t.current_drop().is_some()));
            let hint = drop_hint.clone();
            drop.connect_drop(move |_, _, _, _| {
                hint.set_visible(false);
                false
            });
        }
        stack.add_controller(drop);
        let stack_overlay = gtk::Overlay::new();
        stack_overlay.set_child(Some(&stack));
        stack_overlay.add_overlay(&drop_hint);

        // ---- sidebars ---------------------------------------------------------
        // left: Home, Recent, Favourites, Notes & Highlights, shelves and browse groups, Library
        // Health at the bottom. Right: the details of the selection.
        let tags = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let tags_scroll = gtk::ScrolledWindow::new();
        tags_scroll.set_child(Some(&tags));
        tags_scroll.set_vexpand(true);
        tags_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);

        let top = gtk::ListBox::new();
        top.add_css_class("navigation-sidebar");
        top.set_selection_mode(gtk::SelectionMode::Single);
        super::audit::label(&top, &tr("Library views"));
        let mut top_counts = Vec::new();
        for (view, title, icon) in [
            (View::Home, tr("Home"), "user-home-symbolic"),
            (View::Recent, tr("Recent"), "document-open-recent-symbolic"),
            (View::Favourites, tr("Favourites"), "starred-symbolic"),
            (
                View::Notes,
                tr("Notes & Highlights"),
                "document-edit-symbolic",
            ),
        ] {
            let (row, count) = sidebar_row(&title, icon);
            row.set_widget_name(&format!("nav-{}", view.key()));
            top.append(&row);
            top_counts.push(count);
        }
        {
            let tx = sender.input_sender().clone();
            top.connect_row_activated(move |_, row| {
                if let Some(v) = View::ALL.get(row.index() as usize) {
                    tx.emit(Input::SetNav(*v));
                }
            });
        }
        let health_list = gtk::ListBox::new();
        health_list.add_css_class("navigation-sidebar");
        health_list.set_selection_mode(gtk::SelectionMode::None);
        let (health_row, health_badge) =
            sidebar_row(&tr("Library Health"), "security-high-symbolic");
        health_badge.add_css_class("warning");
        health_row.set_widget_name("nav-health");
        health_row.set_tooltip_text(Some(&tr(
            "Checks, duplicates, editions, books that need attention, backups, trash",
        )));
        health_list.append(&health_row);
        {
            let tx = sender.input_sender().clone();
            health_list.connect_row_activated(move |_, _| tx.emit(Input::HealthTab("health")));
        }
        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar.add_css_class("bookx-sidebar");
        top.set_margin_top(6);
        sidebar.append(&top);
        let sep = gtk::Separator::new(gtk::Orientation::Horizontal);
        sep.set_margin_top(6);
        sep.set_margin_bottom(2);
        sidebar.append(&sep);
        sidebar.append(&tags_scroll);
        let sep = gtk::Separator::new(gtk::Orientation::Horizontal);
        sep.set_margin_bottom(2);
        sidebar.append(&sep);
        health_list.set_margin_bottom(6);
        sidebar.append(&health_list);

        let details = gtk::Box::new(gtk::Orientation::Vertical, 6);
        details.set_margin_all(16);
        let details_scroll = gtk::ScrolledWindow::new();
        details_scroll.set_child(Some(&details));
        details_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        details_scroll.add_css_class("sidebar-pane");

        let content = gtk::Stack::new();
        content.set_transition_type(gtk::StackTransitionType::None);
        content.add_named(&stack_overlay, Some("books"));

        let split_details = adw::OverlaySplitView::new();
        split_details.set_sidebar_position(gtk::PackType::End);
        split_details.set_min_sidebar_width(300.0);
        split_details.set_max_sidebar_width(380.0);
        split_details.set_sidebar_width_fraction(0.28);
        split_details.set_sidebar(Some(&details_scroll));
        split_details.set_content(Some(&content));
        split_details.set_show_sidebar(false);

        let split_tags = adw::OverlaySplitView::new();
        split_tags.set_min_sidebar_width(250.0);
        split_tags.set_max_sidebar_width(320.0);
        split_tags.set_sidebar_width_fraction(0.2);
        // the sidebar has its own header bar and the content has its own, as in Files: the
        // separator runs all the way up
        let sidebar_header = adw::HeaderBar::new();
        sidebar_header.set_show_start_title_buttons(false);
        sidebar_header.set_show_end_title_buttons(false);
        let sidebar_view = adw::ToolbarView::new();
        sidebar_view.add_top_bar(&sidebar_header);
        // in a very short window the whole sidebar scrolls instead of making the window taller
        let sidebar_scroll = gtk::ScrolledWindow::new();
        sidebar_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        sidebar_scroll.set_child(Some(&sidebar));
        sidebar_view.set_content(Some(&sidebar_scroll));
        split_tags.set_sidebar(Some(&sidebar_view));
        split_tags.set_show_sidebar(true);
        // narrow windows: both panels become overlays on top of the books, closed to begin with
        let split_bin = adw::BreakpointBin::new();
        split_bin.set_width_request(crate::library_nav::MIN_WIDTH);
        split_bin.set_height_request(crate::library_nav::MIN_HEIGHT);
        split_bin.set_child(Some(&split_tags));
        // the docked sidebar leaves the content at least 670 px; with the details panel docked as
        // well the grid keeps 470 px (see `library_nav::window_layout`)
        let bp = |w: f64| {
            adw::Breakpoint::new(adw::BreakpointCondition::new_length(
                adw::BreakpointConditionLengthType::MaxWidth,
                w,
                adw::LengthUnit::Sp,
            ))
        };
        // each breakpoint sets everything that holds from its width down: only the narrowest
        // matching one is in force
        let mid = bp(crate::library_nav::DETAILS_COLLAPSE_AT);
        mid.add_setter(&split_details, "collapsed", Some(&true.to_value()));
        split_bin.add_breakpoint(mid);
        let narrow = bp(crate::library_nav::SIDEBAR_COLLAPSE_AT);
        narrow.add_setter(&split_tags, "collapsed", Some(&true.to_value()));
        narrow.add_setter(&split_tags, "show-sidebar", Some(&false.to_value()));
        narrow.add_setter(&split_details, "collapsed", Some(&true.to_value()));
        split_bin.add_breakpoint(narrow);

        // ---- header: just the essentials -------------------------------------------
        // title · add · search · menu. Everything else (browse panel, details, view options)
        // lives behind the menu, so the first impression is the books.
        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        let title = adw::WindowTitle::new(&tr("Home"), "");
        let sel_title = adw::WindowTitle::new(&trf("{} selected", &[&0]), "");
        let title_stack = gtk::Stack::new();
        title_stack.add_named(&title, Some("title"));
        title_stack.add_named(&sel_title, Some("selection"));
        title_stack.set_hhomogeneous(false);
        title_stack.set_transition_type(gtk::StackTransitionType::None);
        header.set_title_widget(Some(&title_stack));
        let cancel_btn = gtk::Button::with_label(&tr("Cancel"));
        cancel_btn.set_visible(false);
        cancel_btn.set_tooltip_text(Some(&tr("Cancel Selection (Esc)")));
        {
            let tx = sender.input_sender().clone();
            cancel_btn.connect_clicked(move |_| tx.emit(Input::ClearSelection));
        }
        header.pack_start(&cancel_btn);

        let add_menu = gio::Menu::new();
        add_menu.append(Some(&tr("Add Books")), Some("lib.add-books"));
        add_menu.append(Some(&tr("Add from the Web")), Some("lib.add-web"));
        add_menu.append(Some(&tr("Add Folder")), Some("lib.add-folder"));
        add_menu.append(Some(&tr("Add Audiobook")), Some("lib.add-audiobook"));
        let imports = gio::Menu::new();
        imports.append(Some(&tr("Import Calibre Library")), Some("lib.calibre"));
        imports.append(
            Some(&tr("Import Highlights")),
            Some("lib.import-highlights"),
        );
        add_menu.append_section(None, &imports);
        let add_btn = gtk::MenuButton::new();
        add_btn.set_icon_name("list-add-symbolic");
        add_btn.set_menu_model(Some(&add_menu));
        super::audit::name_menu_items(&add_btn);
        add_btn.set_tooltip_text(Some(&tr("Add Books (Ctrl+O)")));
        sidebar_header.pack_start(&add_btn);

        // the sidebar is always there; in a narrow window it is an overlay and this button opens it
        let tags_toggle = gtk::ToggleButton::new();
        tags_toggle.set_icon_name("sidebar-show-symbolic");
        tags_toggle.set_tooltip_text(Some(&tr("Sidebar")));
        super::audit::label(&tags_toggle, &tr("Sidebar"));
        split_tags
            .bind_property("show-sidebar", &tags_toggle, "active")
            .bidirectional()
            .sync_create()
            .build();
        split_tags
            .bind_property("collapsed", &tags_toggle, "visible")
            .sync_create()
            .build();
        header.pack_start(&tags_toggle);

        let spinner = gtk::Spinner::new();
        spinner.set_visible(false);
        header.pack_start(&spinner);
        {
            let (out, tx) = (
                sender.output_sender().clone(),
                sender.input_sender().clone(),
            );
            let switcher = super::library_switcher::build(&super::library_switcher::Ctx {
                switch: Rc::new(move |p| out.emit(Output::SwitchLibrary(p))),
                toast: Rc::new(move |m| tx.emit(Input::Toast(m))),
            });
            sidebar_header.set_title_widget(Some(&switcher));
        }
        header.pack_start(&super::jobs::build());

        let search = gtk::SearchEntry::new();
        search.set_hexpand(true);
        search.set_placeholder_text(Some(&tr("Search books")));
        search.set_tooltip_text(Some(&tr("Calibre-style search:\n  title: author: tag: series: publisher: language: rating: status: format: identifier:\n  = exact, >= <= comparison, - to exclude, OR to combine\n  text:word searches the contents of the books")));
        let search_clamp = adw::Clamp::new();
        search_clamp.set_maximum_size(560);
        search_clamp.set_tightening_threshold(360);
        let search_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        search_row.append(&search);
        {
            let tx = sender.input_sender().clone();
            search_row.append(&extra::search_tips(
                &search,
                Rc::new(move |name| tx.emit(Input::SaveSearch(name))),
            ));
        }
        search_clamp.set_child(Some(&search_row));
        let search_bar = gtk::SearchBar::new();
        search_bar.set_child(Some(&search_clamp));
        search_bar.connect_entry(&search);
        search_bar.set_key_capture_widget(Some(&root));
        {
            let tx = sender.input_sender().clone();
            search.connect_search_changed(move |e| tx.emit(Input::Search(e.text().to_string())));
        }
        {
            // closing the search bar also clears the filter
            let e = search.clone();
            search_bar.connect_search_mode_enabled_notify(move |b| {
                if !b.is_search_mode() {
                    e.set_text("");
                }
            });
        }
        let search_toggle = gtk::ToggleButton::new();
        search_toggle.set_icon_name("system-search-symbolic");
        search_toggle.set_tooltip_text(Some(&tr("Search (Ctrl+F)")));
        search_toggle
            .bind_property("active", &search_bar, "search-mode-enabled")
            .bidirectional()
            .sync_create()
            .build();

        // view options popover
        let opts = gtk::Box::new(gtk::Orientation::Vertical, 10);
        opts.set_margin_all(12);
        opts.set_width_request(260);
        let view_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        view_box.add_css_class("linked");
        view_box.set_halign(gtk::Align::Center);
        let view_grid = gtk::ToggleButton::new();
        view_grid.set_icon_name("view-grid-symbolic");
        view_grid.set_tooltip_text(Some(&tr("Cover Grid")));
        let view_table = gtk::ToggleButton::new();
        view_table.set_icon_name("view-list-symbolic");
        view_table.set_tooltip_text(Some(&tr("Table")));
        view_table.set_group(Some(&view_grid));
        view_grid.set_active(cfg.view_mode == ViewMode::Grid);
        view_table.set_active(cfg.view_mode == ViewMode::Table);
        view_box.append(&view_grid);
        view_box.append(&view_table);
        {
            let tx = sender.input_sender().clone();
            view_grid.connect_toggled(move |b| {
                if b.is_active() {
                    tx.emit(Input::SetView(ViewMode::Grid));
                }
            });
            let tx = sender.input_sender().clone();
            view_table.connect_toggled(move |b| {
                if b.is_active() {
                    tx.emit(Input::SetView(ViewMode::Table));
                }
            });
        }
        opts.append(&view_box);

        let sort_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let sort_lbl = gtk::Label::new(Some(&tr("Sort by")));
        sort_lbl.set_hexpand(true);
        sort_lbl.set_xalign(0.0);
        let labels: Vec<&str> = SortKey::ALL.iter().map(|k| k.label()).collect();
        let sort_dd = gtk::DropDown::from_strings(&labels);
        super::audit::label(&sort_dd, &tr("Sort by"));
        sort_dd.set_selected(
            SortKey::ALL
                .iter()
                .position(|k| *k == cfg.sort_key)
                .unwrap_or(0) as u32,
        );
        {
            let tx = sender.input_sender().clone();
            let guard = table_sync.clone();
            sort_dd.connect_selected_notify(move |d| {
                if guard.get() {
                    return;
                }
                if let Some(k) = SortKey::ALL.get(d.selected() as usize) {
                    tx.emit(Input::Sort(*k));
                }
            });
        }
        let order_btn = gtk::Button::from_icon_name(if cfg.sort_ascending {
            "view-sort-ascending-symbolic"
        } else {
            "view-sort-descending-symbolic"
        });
        order_btn.set_tooltip_text(Some(&tr("Reverse Order")));
        super::audit::label(&order_btn, &order_label(cfg.sort_ascending));
        {
            let tx = sender.input_sender().clone();
            order_btn.connect_clicked(move |_| tx.emit(Input::ToggleOrder));
        }
        sort_row.append(&sort_lbl);
        sort_row.append(&sort_dd);
        sort_row.append(&order_btn);
        opts.append(&sort_row);

        let zoom_lbl = gtk::Label::new(Some(&tr("Cover size")));
        zoom_lbl.set_xalign(0.0);
        let zoom = gtk::Scale::with_range(gtk::Orientation::Horizontal, 100.0, 260.0, 10.0);
        zoom.set_value(cfg.cover_size as f64);
        zoom.set_draw_value(false);
        super::audit::label(&zoom, &tr("Cover size"));
        {
            let tx = sender.input_sender().clone();
            zoom.connect_value_changed(move |s| tx.emit(Input::Zoom(s.value() as i32)));
        }
        opts.append(&zoom_lbl);
        opts.append(&zoom);
        let group_check = gtk::CheckButton::with_label(&tr("Group editions of the same book"));
        group_check.set_active(cfg.group_editions);
        group_check.set_tooltip_text(Some(&tr(
            "Show editions and formats of one book as one card, with a list of the editions",
        )));
        {
            let tx = sender.input_sender().clone();
            group_check.connect_toggled(move |c| tx.emit(Input::GroupEditions(c.is_active())));
        }
        opts.append(&group_check);
        for w in [
            zoom_lbl.upcast_ref::<gtk::Widget>(),
            zoom.upcast_ref(),
            group_check.upcast_ref(),
        ] {
            w.set_visible(nav::grid_only_options_visible(
                cfg.view_mode == ViewMode::Table,
            ));
        }
        // table only: which columns show (by default the ones that fit)
        let cols_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let cols_lbl = gtk::Label::new(Some(&tr("Columns")));
        cols_lbl.set_xalign(0.0);
        cols_lbl.add_css_class("heading");
        cols_box.append(&cols_lbl);
        let mut col_checks: Vec<(&'static str, gtk::CheckButton)> = Vec::new();
        for (name, _, optional) in nav::TABLE_COLUMNS {
            if !optional {
                continue;
            }
            let c = gtk::CheckButton::with_label(&tr(name));
            let (tx, name_s) = (sender.input_sender().clone(), name);
            let guard = table_sync.clone();
            c.connect_toggled(move |c| {
                if !guard.get() {
                    tx.emit(Input::TableColumn(name_s, c.is_active()));
                }
            });
            cols_box.append(&c);
            col_checks.push((name, c));
        }
        let cols_auto = gtk::Button::with_label(&tr("Fit to Window"));
        cols_auto.set_tooltip_text(Some(&tr("Show the columns that fit the width")));
        cols_auto.add_css_class("flat");
        cols_auto.set_halign(gtk::Align::Start);
        {
            let tx = sender.input_sender().clone();
            cols_auto.connect_clicked(move |_| tx.emit(Input::TableColumnsAuto));
        }
        cols_box.append(&cols_auto);
        cols_box.set_visible(cfg.view_mode == ViewMode::Table);
        opts.append(&cols_box);
        let apply_cols: Rc<dyn Fn()> = {
            let (shared, scroll, cols, checks, guard) = (
                shared.clone(),
                table_scroll.clone(),
                table_cols.clone(),
                col_checks.clone(),
                table_sync.clone(),
            );
            let last = Rc::new(RefCell::new(Vec::<&'static str>::new()));
            Rc::new(move || {
                let want = nav::effective_columns(
                    &shared.cfg.borrow().table_columns,
                    scroll.width().max(1),
                );
                if *last.borrow() == want {
                    return;
                }
                for (n, c) in &cols {
                    c.set_visible(want.contains(n));
                }
                guard.set(true);
                for (n, c) in &checks {
                    c.set_active(want.contains(n));
                }
                guard.set(false);
                *last.borrow_mut() = want;
            })
        };
        {
            let f = apply_cols.clone();
            table_scroll.add_tick_callback(move |_, _| {
                f();
                glib::ControlFlow::Continue
            });
        }
        let view_pop = gtk::Popover::new();
        view_pop.set_child(Some(&opts));
        let view_btn = gtk::MenuButton::new();
        view_btn.set_icon_name(nav::view_button_icon(cfg.view_mode == ViewMode::Table));
        view_btn.set_tooltip_text(Some(&tr("View Options")));
        super::audit::label(&view_btn, &tr("View options"));
        view_btn.set_popover(Some(&view_pop));
        let details_toggle = gtk::ToggleButton::new();
        details_toggle.set_icon_name("sidebar-show-right-symbolic");
        details_toggle.set_tooltip_text(Some(&tr("Details (Shift+F9)")));
        super::audit::label(&details_toggle, &tr("Details"));
        split_details
            .bind_property("show-sidebar", &details_toggle, "active")
            .bidirectional()
            .sync_create()
            .build();
        {
            // a click of the user is remembered; the panel following the selection is not
            let tx = sender.input_sender().clone();
            details_toggle.connect_clicked(move |b| tx.emit(Input::DetailsPref(b.is_active())));
        }
        header.pack_end(&details_toggle);
        header.pack_end(&view_btn);
        let menu_btn = gtk::MenuButton::new();
        menu_btn.set_icon_name("open-menu-symbolic");
        menu_btn.set_tooltip_text(Some(&tr("Main Menu")));
        super::audit::label(&menu_btn, &tr("Main Menu"));
        menu_btn.set_menu_model(Some(&build_entry_menu(&MAIN_MENU)));
        menu_btn.set_primary(true);
        super::audit::name_menu_items(&menu_btn);
        let library_btn = gtk::MenuButton::new();
        library_btn.set_icon_name("view-more-horizontal-symbolic");
        library_btn.set_tooltip_text(Some(&tr("Library")));
        super::audit::label(&library_btn, &tr("Library"));
        library_btn.set_menu_model(Some(&build_entry_menu(&LIBRARY_MENU)));
        super::audit::name_menu_items(&library_btn);
        header.pack_end(&library_btn);
        sidebar_header.pack_end(&menu_btn);
        sidebar_header.pack_end(&search_toggle);
        let device_btn = gtk::Button::new();
        device_btn.set_visible(false);
        device_btn.add_css_class("device-button");
        {
            let content = adw::ButtonContent::new();
            content.set_icon_name("drive-removable-media-symbolic");
            device_btn.set_child(Some(&content));
        }
        device_btn.set_tooltip_text(Some(&tr("Connected e-reader: sync, send and manage books")));
        device_btn.set_action_name(Some("lib.device"));
        // in a very narrow header the device button moves into the view options popover
        let device_slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        device_slot.append(&device_btn);
        header.pack_end(&device_slot);
        let device_more = gtk::Button::new();
        device_more.set_tooltip_text(Some(&tr("Connected e-reader: sync, send and manage books")));
        device_more.add_css_class("flat");
        device_more.set_action_name(Some("lib.device"));
        device_more.set_visible(false);
        opts.append(&device_more);
        {
            let (b, s, m) = (device_btn.clone(), device_slot.clone(), device_more.clone());
            let sync = move || m.set_visible(b.is_visible() && !s.is_visible());
            let f = sync.clone();
            device_btn.connect_visible_notify(move |_| f());
            let f = sync.clone();
            device_slot.connect_visible_notify(move |_| f());
        }
        let small = bp(crate::library_nav::HEADER_COMPACT_AT);
        small.add_setter(&split_tags, "collapsed", Some(&true.to_value()));
        small.add_setter(&split_tags, "show-sidebar", Some(&false.to_value()));
        small.add_setter(&split_details, "collapsed", Some(&true.to_value()));
        small.add_setter(&device_slot, "visible", Some(&false.to_value()));

        // one-shot sync; the built-in dismiss button hides the banner without changing settings
        let device_banner = adw::Banner::builder()
            .title(crate::device_sync::banner_title())
            .button_label(crate::device_sync::banner_button_label())
            .revealed(false)
            .build();
        {
            let tx = sender.input_sender().clone();
            device_banner.connect_button_clicked(move |_| {
                tx.emit(Input::DeviceBanner(DeviceChoice::Sync));
            });
        }
        {
            // AdwBanner's own close button sets `revealed` to false; keep the model in step
            let tx = sender.input_sender().clone();
            device_banner.connect_revealed_notify(move |banner| {
                if !banner.is_revealed() {
                    tx.emit(Input::DeviceBanner(DeviceChoice::Dismiss));
                }
            });
        }

        // ---- bottom (selection) bar --------------------------------------------
        let bar = gtk::ActionBar::new();
        let sel_read = gtk::Button::with_label(&tr("Read"));
        sel_read.add_css_class("suggested-action");
        sel_read.set_action_name(Some("book.read"));
        let sel_edit = gtk::Button::with_label(&tr("Edit Metadata"));
        sel_edit.set_action_name(Some("book.edit-meta"));
        let sel_shelf = gtk::MenuButton::new();
        sel_shelf.set_label(&tr("Add to Shelf"));
        sel_shelf.set_always_show_arrow(true);
        let sel_send = gtk::Button::with_label(&tr("Send to Device"));
        sel_send.set_action_name(Some("book.send-device"));
        let sel_convert = gtk::MenuButton::new();
        sel_convert.set_label(&tr("Convert"));
        sel_convert.set_always_show_arrow(true);
        let sel_more = gtk::MenuButton::new();
        sel_more.set_icon_name("view-more-symbolic");
        sel_more.set_tooltip_text(Some(&tr("More Actions")));
        super::audit::label(&sel_more, &tr("More actions"));
        sel_more.set_visible(false);
        let sel_del = gtk::Button::with_label(&tr("Move to Trash"));
        sel_del.set_action_name(Some("book.delete"));
        sel_del.set_tooltip_text(Some(&tr("Move to Trash")));
        super::audit::label(&sel_del, &tr("Move to Trash"));
        let sel_details = gtk::Button::with_label(&tr("Details"));
        sel_details.set_visible(false);
        {
            let split = split_details.clone();
            sel_details.connect_clicked(move |_| split.set_show_sidebar(true));
        }
        bar.pack_start(&sel_edit);
        bar.pack_start(&sel_shelf);
        bar.pack_start(&sel_send);
        bar.pack_start(&sel_convert);
        bar.pack_start(&sel_more);
        bar.pack_end(&sel_del);
        bar.pack_end(&sel_details);
        bar.pack_end(&sel_read);

        {
            // at 600 sp and below the extra actions of the bar move into "More"
            let tx = sender.input_sender().clone();
            small.connect_apply(move |_| tx.emit(Input::Compact(true)));
            let tx = sender.input_sender().clone();
            small.connect_unapply(move |_| tx.emit(Input::Compact(false)));
        }
        split_bin.add_breakpoint(small);

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.add_top_bar(&search_bar);
        toolbar.add_top_bar(&device_banner);
        toolbar.set_content(Some(&split_details));
        toolbar.add_bottom_bar(&bar);
        toolbar.set_reveal_bottom_bars(false);
        split_tags.set_content(Some(&toolbar));
        let toasts = super::dialog_toast::overlay(&split_bin);
        root.set_child(Some(&toasts));

        // ---- actions ----------------------------------------------------------
        let lib_actions = gio::SimpleActionGroup::new();
        let add = |name: &str, f: Box<dyn Fn()>| {
            let a = gio::SimpleAction::new(name, None);
            a.connect_activate(move |_, _| f());
            lib_actions.add_action(&a);
        };
        {
            let tx = sender.input_sender().clone();
            add("add-books", Box::new(move || tx.emit(Input::AddFiles)));
            let tx = sender.input_sender().clone();
            add("add-web", Box::new(move || tx.emit(Input::AddWebDialog)));
            let tx = sender.input_sender().clone();
            add("news", Box::new(move || tx.emit(Input::NewsDialog)));
            add(
                "device",
                Box::new(|| {
                    if let Some(app) = gio::Application::default() {
                        gio::prelude::ActionGroupExt::activate_action(&app, "devices", None);
                    }
                }),
            );
            let tx = sender.input_sender().clone();
            add(
                "duplicates",
                Box::new(move || tx.emit(Input::HealthTab("duplicates"))),
            );
            let tx = sender.input_sender().clone();
            add("saved", Box::new(move || tx.emit(Input::SavedSearches)));
            let tx = sender.input_sender().clone();
            add("opds", Box::new(move || tx.emit(Input::Opds)));
            let tx = sender.input_sender().clone();
            add("health", Box::new(move || tx.emit(Input::Health(false))));
            for (name, which) in [
                ("series", "series"),
                ("authors", "authors"),
                ("audiobooks", "audiobooks"),
                ("trackers", "trackers"),
                ("citations", "citations"),
            ] {
                let tx = sender.input_sender().clone();
                add(name, Box::new(move || tx.emit(Input::Organize(which))));
            }
            for (name, tab) in [("works", "editions"), ("attention", "attention")] {
                let tx = sender.input_sender().clone();
                add(name, Box::new(move || tx.emit(Input::HealthTab(tab))));
            }
            for v in View::ALL {
                let tx = sender.input_sender().clone();
                add(
                    &format!("view-{}", v.key()),
                    Box::new(move || tx.emit(Input::SetNav(v))),
                );
            }
            let tx = sender.input_sender().clone();
            add(
                "favourite",
                Box::new(move || tx.emit(Input::Act(Act::Favourite))),
            );
            let tx = sender.input_sender().clone();
            add("calibre", Box::new(move || tx.emit(Input::Health(true))));
            let tx = sender.input_sender().clone();
            add(
                "notes",
                Box::new(move || tx.emit(Input::SetNav(View::Notes))),
            );
            let tx = sender.input_sender().clone();
            add(
                "import-highlights",
                Box::new(move || tx.emit(Input::Act(Act::ImportAnn))),
            );
            let tx = sender.input_sender().clone();
            add(
                "export-annotations",
                Box::new(move || tx.emit(Input::Act(Act::ExportAllAnn))),
            );
            let tx = sender.input_sender().clone();
            add(
                "export-calibre",
                Box::new(move || tx.emit(Input::Act(Act::ExportCalibre))),
            );
            let tx = sender.input_sender().clone();
            add(
                "add-folder",
                Box::new(move || tx.emit(Input::AddFolder(true))),
            );
            let tx = sender.input_sender().clone();
            let win_root = root.clone();
            add(
                "add-audiobook",
                Box::new(move || {
                    let tx = tx.clone();
                    super::audiobook_player::pick(
                        window_of(&win_root),
                        Rc::new(move |paths| tx.emit(Input::ImportPaths(paths))),
                    );
                }),
            );
            let tx = sender.input_sender().clone();
            add(
                "add-folder-recursive",
                Box::new(move || tx.emit(Input::AddFolder(true))),
            );
            let sh = shared.clone();
            add(
                "open-folder",
                Box::new(move || {
                    super::open_uri(&gio::File::for_path(&sh.lib.borrow().root).uri());
                }),
            );
            let tx = sender.input_sender().clone();
            add(
                "focus-search",
                Box::new(move || tx.emit(Input::FocusSearch)),
            );
            let tx = sender.input_sender().clone();
            add("select-all", Box::new(move || tx.emit(Input::SelectAll)));
        }
        root.insert_action_group("lib", Some(&lib_actions));
        super::palette::register_group("lib", &lib_actions);

        let book_actions = gio::SimpleActionGroup::new();
        let send_action: gio::SimpleAction;
        let simple = |name: &str, act: Act| {
            let a = gio::SimpleAction::new(name, None);
            let tx = sender.input_sender().clone();
            a.connect_activate(move |_, _| tx.emit(Input::Act(act.clone())));
            book_actions.add_action(&a);
        };
        simple("read", Act::Read);
        simple("edit-meta", Act::EditMeta);
        simple("fetch-meta", Act::FetchMeta);
        simple("preflight", Act::Preflight);
        simple("merge-formats", Act::MergeFormats);
        simple("link-editions", Act::LinkEditions);
        simple("unlink-editions", Act::UnlinkEditions);
        simple("show-editions", Act::ShowEditionList);
        simple("edit-book", Act::EditBook);
        simple("mark-read", Act::MarkRead(true));
        simple("mark-unread", Act::MarkRead(false));
        simple("save-copy", Act::SaveCopy);
        simple("show-folder", Act::ShowFolder);
        simple("delete", Act::Delete);
        simple("favourite", Act::Favourite);
        simple("properties", Act::Properties);
        simple("shelf-new", Act::ShelfNew);
        simple("shelf-remove", Act::ShelfRemove);
        simple("shelf-earlier", Act::ShelfMove(-1));
        simple("shelf-later", Act::ShelfMove(1));
        {
            let a = gio::SimpleAction::new("shelf-add", Some(glib::VariantTy::STRING));
            let tx = sender.input_sender().clone();
            a.connect_activate(move |_, p| {
                if let Some(name) = p.and_then(|v| v.str()) {
                    tx.emit(Input::Act(Act::ShelfAdd(name.to_string())));
                }
            });
            book_actions.add_action(&a);
        }
        {
            let a = gio::SimpleAction::new("export-ann", Some(glib::VariantTy::STRING));
            let tx = sender.input_sender().clone();
            a.connect_activate(move |_, p| {
                let fmt = p.and_then(|v| v.str()).unwrap_or("json").to_string();
                tx.emit(Input::Act(Act::ExportAnn(fmt)));
            });
            book_actions.add_action(&a);
            simple("import-ann", Act::ImportAnn);
            simple("copy-citation", Act::CopyCitation);
            let a = gio::SimpleAction::new("email", None);
            let tx = sender.input_sender().clone();
            a.connect_activate(move |_, _| tx.emit(Input::Act(Act::Email)));
            book_actions.add_action(&a);
            let a = gio::SimpleAction::new("send-device", None);
            a.set_enabled(false);
            a.connect_activate(|_, _| {
                if let Some(app) = gio::Application::default() {
                    gio::prelude::ActionGroupExt::activate_action(&app, "devices", None);
                }
            });
            book_actions.add_action(&a);
            send_action = a;
            let a = gio::SimpleAction::new("convert-to", Some(glib::VariantTy::STRING));
            let tx = sender.input_sender().clone();
            a.connect_activate(move |_, p| {
                if let Some(id) = p.and_then(|v| v.str()) {
                    tx.emit(Input::Act(Act::ConvertTo(id.to_string())));
                }
            });
            book_actions.add_action(&a);
            let a = gio::SimpleAction::new("convert", Some(glib::VariantTy::STRING));
            let tx = sender.input_sender().clone();
            a.connect_activate(move |_, p| {
                let fmt = match p.and_then(|v| v.str()).unwrap_or("txt") {
                    "md" => ExportFormat::Markdown,
                    "html" => ExportFormat::Html,
                    _ => ExportFormat::Txt,
                };
                tx.emit(Input::Act(Act::Convert(fmt)));
            });
            book_actions.add_action(&a);
        }
        root.insert_action_group("book", Some(&book_actions));

        // keyboard shortcuts local to the library (the user can change them: Preferences)
        {
            let current: Rc<RefCell<Option<gtk::ShortcutController>>> = Rc::new(RefCell::new(None));
            let weak = root.downgrade();
            let install = Rc::new(move || {
                let Some(root) = weak.upgrade() else { return };
                if let Some(old) = current.borrow_mut().take() {
                    root.remove_controller(&old);
                }
                let sc = gtk::ShortcutController::new();
                sc.set_scope(gtk::ShortcutScope::Local);
                for (accel, action) in super::shortcuts_dialog::library_local_shortcuts() {
                    if let Some(t) = gtk::ShortcutTrigger::parse_string(&accel) {
                        sc.add_shortcut(gtk::Shortcut::new(
                            Some(t),
                            Some(gtk::NamedAction::new(&action)),
                        ));
                    }
                }
                root.add_controller(sc.clone());
                *current.borrow_mut() = Some(sc);
            });
            install();
            crate::shortcuts::on_change(move || install());
        }

        // Ctrl+V on Home: a pasted address opens Add from the Web with it (an entry that has the
        // focus takes the paste itself)
        {
            let tx = sender.input_sender().clone();
            let sc = gtk::ShortcutController::new();
            sc.set_scope(gtk::ShortcutScope::Local);
            sc.set_propagation_phase(gtk::PropagationPhase::Bubble);
            let root_w = root.clone();
            sc.add_shortcut(gtk::Shortcut::new(
                gtk::ShortcutTrigger::parse_string("<Control>v"),
                Some(gtk::CallbackAction::new(move |_, _| {
                    let tx = tx.clone();
                    root_w.display().clipboard().read_text_async(
                        gio::Cancellable::NONE,
                        move |res| {
                            if let Ok(Some(t)) = res {
                                tx.emit(Input::PasteUrl(t.to_string()));
                            }
                        },
                    );
                    glib::Propagation::Proceed
                })),
            ));
            root.add_controller(sc);
        }

        // Shift+F9 shows or hides the details panel
        {
            let sc = gtk::ShortcutController::new();
            sc.set_scope(gtk::ShortcutScope::Local);
            let toggle = details_toggle.clone();
            sc.add_shortcut(gtk::Shortcut::new(
                gtk::ShortcutTrigger::parse_string("<Shift>F9"),
                Some(gtk::CallbackAction::new(move |_, _| {
                    toggle.activate();
                    glib::Propagation::Stop
                })),
            ));
            root.add_controller(sc);
        }

        // Escape clears the selection (the search bar and menus have taken it before when open)
        {
            let tx = sender.input_sender().clone();
            let sc = gtk::ShortcutController::new();
            sc.set_scope(gtk::ShortcutScope::Local);
            sc.set_propagation_phase(gtk::PropagationPhase::Bubble);
            sc.add_shortcut(gtk::Shortcut::new(
                gtk::ShortcutTrigger::parse_string("Escape"),
                Some(gtk::CallbackAction::new(move |_, _| {
                    tx.emit(Input::ClearSelection);
                    glib::Propagation::Proceed
                })),
            ));
            root.add_controller(sc);
        }

        let model = LibraryView {
            shared: shared.clone(),
            root: root.clone(),
            sender: sender.clone(),
            grid,
            table,
            cover_size,
            w: W {
                toasts,
                last_undo: RefCell::new(None),
                stack,
                content,
                top,
                top_counts,
                health_badge,
                sel_details,
                sel_del,
                details_toggle,
                table_scroll,
                table_cols,
                table_sync: table_sync.clone(),
                grid_clamp,
                search,
                details,
                tags,
                title,
                search_bar,
                split_tags,
                split_details,
                spinner,
                toolbar,
                title_stack,
                sel_title,
                cancel_btn,
                sel_read,
                sel_edit,
                sel_shelf,
                sel_send,
                sel_convert,
                sel_more,
                add_menu: add_menu.clone(),
                banner,
                banner_ctl,
                order_btn,
                view_btn: view_btn.clone(),
                drop_hint,
                cols_box: cols_box.clone(),
                sort_dd: sort_dd.clone(),
                apply_cols,
                grid_only: vec![
                    zoom_lbl.clone().upcast(),
                    zoom.clone().upcast(),
                    group_check.clone().upcast(),
                ],
                send_action,
                header: header.clone(),
                empty_btns,
                device_btn,
                device_more,
                device_banner,
            },
            selected: Vec::new(),
            groups: HashMap::new(),
            card_of: HashMap::new(),
            card_ids: Vec::new(),
            anchor: None,
            hero: RefCell::new(nav::HeroSession::default()),
            compact: false,
            notes_sel: 0,
            banner_colors: RefCell::new(None),
            skip_details: Cell::new(false),
            requeries: 0,
            fill: None,
            fill_gen: 0,
            fill_query: String::new(),
            view: View::from_key(&cfg.sidebar_view),
            notes: Rc::new(RefCell::new(None)),
            query: String::new(),
            pending: 0,
            stats: (0, 0, 0),
            import_job: None,
            expanded: Rc::new(RefCell::new(HashSet::new())),
            news: NewsState {
                store: Rc::new(RefCell::new(crate::news::NewsStore::load(
                    crate::config::news_path(),
                ))),
                queue: Default::default(),
                running: None,
                new_issues: 0,
                failures: Vec::new(),
                slot: Default::default(),
            },
            devices: Vec::new(),
            banner_device: None,
            declined: HashSet::new(),
            device_scan: Rc::new(Cell::new(false)),
        };
        model.sync_sort_controls();
        sender.input(Input::Refresh);
        sender.input(Input::SetNav(model.view));
        {
            // look for devices now and every few seconds, off the main thread
            let tx = sender.input_sender().clone();
            let busy = model.device_scan.clone();
            let scan: Rc<dyn Fn()> = Rc::new(move || {
                if busy.replace(true) {
                    return;
                }
                let (rtx, rrx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let _ = rtx.send(crate::devices::detect_physical());
                });
                let (tx, busy) = (tx.clone(), busy.clone());
                glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                    match rrx.try_recv() {
                        Ok(v) => {
                            busy.set(false);
                            tx.emit(Input::Devices(v));
                            glib::ControlFlow::Break
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            busy.set(false);
                            glib::ControlFlow::Break
                        }
                    }
                });
            });
            scan();
            glib::timeout_add_seconds_local(3, move || {
                scan();
                glib::ControlFlow::Continue
            });
        }
        {
            // the scheduler: once shortly after start, then every minute
            let tx = sender.input_sender().clone();
            glib::timeout_add_seconds_local_once(15, move || tx.emit(Input::NewsDue));
            let tx = sender.input_sender().clone();
            glib::timeout_add_seconds_local(60, move || {
                tx.emit(Input::NewsDue);
                glib::ControlFlow::Continue
            });
        }
        {
            let tx = sender.input_sender().clone();
            super::language_settings::on_change(move || {
                let _ = tx.send(Input::Requery);
            });
        }
        ComponentParts { model, widgets: () }
    }

    fn update(&mut self, msg: Input, sender: ComponentSender<Self>) {
        match msg {
            Input::Refresh => {
                let existing: HashSet<u64> = self
                    .shared
                    .lib
                    .borrow()
                    .books
                    .iter()
                    .map(|b| b.id)
                    .collect();
                {
                    let mut c = self.shared.cfg.borrow_mut();
                    if nav::prune_favourites(&mut c.favourites, &existing) {
                        c.save();
                    }
                }
                self.requery();
                self.rebuild_tags();
            }
            Input::SetNav(v) => {
                self.show_search(nav::search_for(nav::NavTarget::View(v)));
                self.set_view(v);
            }
            Input::GoFilter(q) => {
                self.show_search(nav::search_for(nav::NavTarget::Filter(&q)));
                self.set_view(View::Home);
            }
            Input::ToggleFav(id) => {
                {
                    let mut c = self.shared.cfg.borrow_mut();
                    nav::toggle_favourites(&mut c.favourites, &[id]);
                    c.save();
                }
                self.update_top_counts();
                self.requery();
            }
            Input::SetRating(ids, r, in_panel) => {
                if self.edit_meta(&ids, &|m| m.rating = r) > 0 {
                    self.after_meta_edit(in_panel);
                }
            }
            Input::EditTags {
                ids,
                add,
                remove,
                in_panel,
            } => {
                let n = self.edit_meta(&ids, &|m| {
                    for t in &remove {
                        crate::tags::remove(&mut m.tags, t);
                    }
                    for t in &add {
                        crate::tags::add(&mut m.tags, t);
                    }
                });
                if n > 0 {
                    self.after_meta_edit(in_panel);
                }
            }
            Input::BannerLayout => self.update_banner(),
            Input::NotesSelection(n) => {
                self.notes_sel = n;
                self.update_bottom_bar();
            }
            Input::Compact(on) => {
                self.compact = on;
                self.update_bottom_bar();
            }
            Input::BannerAct(id, what) => {
                use super::library_banner::BannerAct;
                match what {
                    BannerAct::Continue => sender.input(Input::Activate(id)),
                    BannerAct::Details => {
                        self.selected = vec![id];
                        self.anchor = Some(id);
                        self.restore_selection();
                        self.follow_selection();
                        self.update_details();
                        self.update_bottom_bar();
                    }
                    BannerAct::FromStart => {
                        {
                            let mut lib = self.shared.lib.borrow_mut();
                            if let Some(b) = lib.books.iter_mut().find(|b| b.id == id) {
                                b.position = None;
                                b.progress = 0.0;
                            }
                            let _ = lib.save();
                        }
                        sender.input(Input::Activate(id));
                    }
                    BannerAct::Finish => {
                        self.selected = vec![id];
                        self.do_act(Act::MarkRead(true));
                    }
                }
            }
            Input::HealthTab(tab) => {
                if let Some(win) = window_of(&self.root) {
                    let (search, fetch, _) = self.organize_callbacks();
                    crate::ui::health::show_tab(
                        &win,
                        &self.shared,
                        self.health_notify(),
                        crate::ui::health::Links { search, fetch },
                        tab,
                    );
                }
            }
            Input::ClearSelection => {
                if self.view == View::Notes {
                    if let Some(v) = self.notes.borrow().as_ref() {
                        (v.clear_selection)();
                    }
                    return;
                }
                let sel = nav::empty_click(&self.selected);
                if !self.selected.is_empty() {
                    self.selected = sel;
                    self.anchor = None;
                    self.restore_selection();
                    self.follow_selection();
                    self.update_bottom_bar();
                }
            }
            Input::Pick(id, how) => {
                let shown = self.card_ids.clone();
                let sel = nav::pick(&self.selected, &shown, self.anchor, id, how);
                if how != nav::Pick::Shift {
                    self.anchor = Some(id);
                }
                self.selected = sel;
                self.restore_selection();
                self.follow_selection();
                self.update_details();
                self.update_bottom_bar();
            }
            Input::AddWebDialog => self.add_web_dialog(""),
            Input::PasteUrl(text) => {
                // pasting an address on Home asks to add it
                if self.view == View::Home {
                    if let Some(url) = nav::pasted_url(&text) {
                        self.add_web_dialog(&url);
                    }
                }
            }
            Input::AddWeb(url) => {
                if self.pending == 0 {
                    self.stats = (0, 0, 0);
                }
                self.pending += 1;
                self.w.spinner.set_visible(true);
                self.w.spinner.start();
                let (res_tx, res_rx) =
                    std::sync::mpsc::channel::<Result<(Prepared, String), String>>();
                std::thread::spawn(move || {
                    let r = crate::web_import::import(
                        &url,
                        &*crate::web_import::fetcher(),
                        chrono::Utc::now(),
                    )
                    .map(|i| {
                        tracing::info!("added from the web: {:?}", i.kind);
                        (i.prepared, i.summary)
                    })
                    .map_err(|e| format!("{e:#}"));
                    let _ = res_tx.send(r);
                });
                let tx = sender.input_sender().clone();
                glib::timeout_add_local(std::time::Duration::from_millis(60), move || match res_rx
                    .try_recv()
                {
                    Ok(Ok((prepared, summary))) => {
                        tx.emit(Input::Prepared(Ok(prepared)));
                        tx.emit(Input::Toast(summary));
                        glib::ControlFlow::Break
                    }
                    Ok(Err(e)) => {
                        tx.emit(Input::Prepared(Err(e.clone())));
                        tx.emit(Input::Toast(e));
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
                });
            }
            Input::NewsDialog => {
                if let Some(win) = window_of(&self.root) {
                    let tx = self.sender.input_sender().clone();
                    let tx2 = tx.clone();
                    super::news::show(
                        &win,
                        super::news::Ctx {
                            store: self.news.store.clone(),
                            run_keys: Rc::new(move |k| tx.emit(Input::NewsRun(k))),
                            run_due: Rc::new(move || tx2.emit(Input::NewsDue)),
                            slot: self.news.slot.clone(),
                        },
                    );
                }
            }
            Input::NewsDue => {
                let keys = self.news.store.borrow().due_keys(self.shared.now());
                if !keys.is_empty() {
                    sender.input(Input::NewsRun(keys));
                }
            }
            Input::NewsRun(keys) => {
                for k in keys {
                    if self.news.running.as_ref() != Some(&k) && !self.news.queue.contains(&k) {
                        self.news.queue.push_back(k);
                    }
                }
                self.news_next();
            }
            Input::NewsDone(key, res) => {
                self.news_finished(&key, res);
                self.news_next();
            }
            Input::Health(calibre) => {
                if let Some(win) = window_of(&self.root) {
                    let notify = self.health_notify();
                    if calibre {
                        crate::ui::health::show_and_import_calibre(&win, &self.shared, notify);
                    } else {
                        crate::ui::health::show(&win, &self.shared, notify);
                    }
                }
            }
            Input::Opds => {
                if let Some(win) = window_of(&self.root) {
                    crate::ui::opds::show(&win, &self.shared, self.opds_import());
                }
            }
            Input::SavedSearches => {
                if let Some(win) = window_of(&self.root) {
                    let (search, _, notify) = self.organize_callbacks();
                    crate::ui::saved::show(&win, &self.shared, &self.query, search, notify);
                }
            }
            Input::Organize(which) => {
                if let Some(win) = window_of(&self.root) {
                    let (search, fetch, notify) = self.organize_callbacks();
                    match which {
                        "series" => {
                            crate::ui::organize::show_series(&win, &self.shared, search, fetch)
                        }
                        "authors" => crate::ui::authors::show(&win, &self.shared, search, notify),
                        "audiobooks" => {
                            crate::ui::audiobooks::show(&win, &self.shared, notify, Vec::new())
                        }
                        "citations" => {
                            crate::ui::citations::show(&win, &self.shared, &self.query, notify)
                        }
                        _ => crate::ui::trackers::show(&win, &self.shared, notify),
                    }
                }
            }
            Input::BulkFetch(ids) => {
                if let Some(win) = window_of(&self.root) {
                    crate::ui::bulkfetch::start(&win, &self.shared, ids, self.health_notify());
                }
            }
            Input::FindDuplicates => {
                if let Some(win) = window_of(&self.root) {
                    let tx = self.sender.input_sender().clone();
                    let shared = self.shared.clone();
                    crate::ui::duplicates::show(
                        &win,
                        &self.shared,
                        Rc::new(move |msg| {
                            let _ = shared.lib.borrow().save();
                            tx.emit(Input::Refresh);
                            tx.emit(Input::Toast(msg));
                        }),
                    );
                }
            }
            Input::Requery => self.requery(),
            Input::Search(q) => {
                // a filter from anywhere (sidebar, shelves, a dialog) shows the bar
                if !q.is_empty() && !self.w.search_bar.is_search_mode() {
                    self.open_search_bar();
                }
                self.query = q;
                self.requery();
            }
            Input::Sort(k) => {
                self.shared.cfg.borrow_mut().sort_key = k;
                self.shared.cfg.borrow().save();
                self.sync_sort_controls();
                self.requery();
            }
            Input::ToggleOrder => {
                let asc = {
                    let mut c = self.shared.cfg.borrow_mut();
                    c.sort_ascending = !c.sort_ascending;
                    c.save();
                    c.sort_ascending
                };
                super::audit::label(&self.w.order_btn, &order_label(asc));
                self.w.order_btn.set_icon_name(if asc {
                    "view-sort-ascending-symbolic"
                } else {
                    "view-sort-descending-symbolic"
                });
                self.sync_sort_controls();
                self.requery();
            }
            Input::SetView(m) => {
                {
                    let mut c = self.shared.cfg.borrow_mut();
                    if c.view_mode == m {
                        return;
                    }
                    c.view_mode = m;
                    c.save();
                }
                let table = m == ViewMode::Table;
                self.w.view_btn.set_icon_name(nav::view_button_icon(table));
                for w in &self.w.grid_only {
                    w.set_visible(nav::grid_only_options_visible(table));
                }
                self.w.cols_box.set_visible(table);
                self.requery();
            }
            Input::Zoom(z) => {
                self.cover_size.set(z);
                {
                    let mut c = self.shared.cfg.borrow_mut();
                    c.cover_size = z;
                    c.save();
                }
                self.requery();
            }
            Input::ShowEditions(id, w) => self.show_editions(id, &w),
            Input::EditionSend(id) => {
                super::devices::set_library_selection(&[id]);
                if self.w.send_action.is_enabled() {
                    self.w.send_action.activate(None);
                }
            }
            Input::EditionPrefer(id) => {
                let members = self
                    .card_of
                    .get(&id)
                    .and_then(|r| self.groups.get(r))
                    .map(|c| c.members.clone())
                    .unwrap_or_else(|| vec![id]);
                self.work_edit(move |s| {
                    s.prefer_edition(id, &members)?;
                    Ok(tr("This edition opens first now"))
                });
            }
            Input::EditionUnlink(id) => {
                self.work_edit(move |s| {
                    s.unlink_edition(id)?;
                    Ok(tr("Took the edition out of its group"))
                });
            }
            Input::GroupEditions(on) => {
                {
                    let mut c = self.shared.cfg.borrow_mut();
                    if c.group_editions == on {
                        return;
                    }
                    c.group_editions = on;
                    c.save();
                }
                self.requery();
            }
            Input::Selection(ids) => {
                // Only the visible view drives the selection.
                let live = self.current_selection();
                if self.filling() {
                    // while the list is still coming in, the model holds only the books that arrived: what it
                    // says is ours unless the user changed it, and then the rest must be there before it counts
                    let loaded = self.fill.as_ref().map_or(0, Fill::loaded);
                    let mine: HashSet<u64> = self.selected.iter().copied().collect();
                    let expected: Vec<u64> = self
                        .card_ids
                        .iter()
                        .take(loaded)
                        .filter(|id| mine.contains(id))
                        .copied()
                        .collect();
                    if live == expected {
                        return;
                    }
                    let seen: HashSet<u64> = self.card_ids.iter().take(loaded).copied().collect();
                    let mut now = live;
                    now.extend(self.selected.iter().filter(|id| !seen.contains(id)));
                    self.selected = now;
                    self.finish_fill();
                    super::devices::set_library_selection(&self.selected_books());
                    super::shelves::set_selection(&self.selected_books());
                    self.follow_selection();
                    self.update_details();
                    self.update_bottom_bar();
                    return;
                }
                let ids = if ids.len() == live.len() || ids.is_empty() {
                    live
                } else {
                    ids
                };
                if ids != self.selected {
                    if ids.len() == 1 {
                        self.anchor = ids.first().copied();
                    }
                    self.selected = ids;
                    super::devices::set_library_selection(&self.selected_books());
                    super::shelves::set_selection(&self.selected_books());
                    self.follow_selection();
                    self.update_details();
                    self.update_bottom_bar();
                }
            }
            Input::Activate(id) => {
                self.leave_home();
                let _ = sender.output(Output::Read(id));
            }
            Input::Devices(list) => self.devices_changed(list),
            Input::DeviceBanner(choice) => self.device_banner_answer(choice),
            Input::Context(id, x, y, w) => self.show_context_menu(id, x, y, &w),
            Input::AddFiles => self.pick_files(false, false),
            Input::AddFolder(r) => self.pick_files(true, r),
            Input::ImportPaths(paths) => {
                if paths.is_empty() {
                    let _ = sender.output(Output::Toast(tr("No supported books found")));
                    return;
                }
                if self.pending == 0 {
                    self.stats = (0, 0, 0);
                }
                self.pending += paths.len();
                self.w.spinner.set_visible(true);
                self.w.spinner.start();
                let token = match self.import_job.as_mut() {
                    Some(j) => {
                        j.2 += paths.len();
                        j.1.clone()
                    }
                    None => {
                        let (id, token) = crate::jobs::start(
                            crate::jobs::Kind::Import,
                            &trf(
                                "Importing {}",
                                &[&crate::library_health::plural(paths.len(), "file")],
                            ),
                            true,
                        );
                        self.import_job = Some((id, token.clone(), paths.len(), 0));
                        token
                    }
                };
                let (res_tx, res_rx) = std::sync::mpsc::channel::<Result<Prepared, String>>();
                std::thread::spawn(move || {
                    for p in paths {
                        if token.is_cancelled() {
                            if res_tx.send(Err(IMPORT_CANCELLED.into())).is_err() {
                                break;
                            }
                            continue;
                        }
                        let res =
                            crate::import::prepare(&p).map_err(|e| format!("{}: {e}", p.display()));
                        if res_tx.send(res).is_err() {
                            break;
                        }
                    }
                });
                let tx = sender.input_sender().clone();
                glib::timeout_add_local(std::time::Duration::from_millis(40), move || loop {
                    match res_rx.try_recv() {
                        Ok(r) => tx.emit(Input::Prepared(r)),
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            return glib::ControlFlow::Continue
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            return glib::ControlFlow::Break
                        }
                    }
                });
            }
            Input::Prepared(res) => {
                self.pending = self.pending.saturating_sub(1);
                if matches!(&res, Err(e) if e == IMPORT_CANCELLED) {
                    self.import_step("Skipped (cancelled)");
                    if self.pending == 0 {
                        self.finish_import(&sender);
                    }
                    return;
                }
                match &res {
                    Ok(p) => self.import_step(&trf("Read {}", &[&p.meta.title])),
                    Err(e) => self.import_step(&trf("Failed: {}", &[&e])),
                }
                match res {
                    Ok(p) => {
                        let note = crate::import::take_patch_note(&p);
                        // the same book in a format the library lacks: offered, not imported
                        let offer = crate::book_formats::offer_for(&self.shared.lib.borrow(), &p);
                        if let Some(o) = offer {
                            FORMAT_OFFERS.with(|f| f.borrow_mut().push((o, p)));
                            if self.pending == 0 {
                                self.finish_import(&sender);
                            }
                            return;
                        }
                        let protected = !p.drm.is_empty();
                        let audio_src = p
                            .meta
                            .identifier(crate::audiobook::MARKER)
                            .and(p.source_file.clone());
                        let committed = self.shared.lib.borrow_mut().commit_import(p, false);
                        match committed {
                            Ok(AddOutcome::Added(id)) => {
                                self.stats.0 += 1;
                                if let (Some(src), Some(data)) =
                                    (audio_src, crate::audiobook::data_dir())
                                {
                                    let r = crate::audiobook::after_import(
                                        &mut self.shared.lib.borrow_mut(),
                                        id,
                                        &src,
                                        &data,
                                    );
                                    if let Err(e) = r {
                                        tracing::warn!("linking the audio failed: {e:#}");
                                    }
                                }
                                IMPORT_ADDED.with(|a| {
                                    let mut a = a.borrow_mut();
                                    a.0.push(id);
                                    a.1 += protected as usize;
                                });
                                if let Some((fields, source)) = note {
                                    // recorded once the catalogue has the book (after the save)
                                    PATCH_NOTES.with(|n| n.borrow_mut().push((id, fields, source)));
                                }
                            }
                            Ok(AddOutcome::Duplicate(_)) => self.stats.1 += 1,
                            Err(_) => self.stats.2 += 1,
                        }
                    }
                    Err(e) => {
                        tracing::warn!("import failed: {e}");
                        self.stats.2 += 1;
                    }
                }
                if self.pending == 0 {
                    self.finish_import(&sender);
                }
            }
            Input::Act(a) => self.do_act(a),
            Input::TableColumn(name, on) => {
                let shown = nav::effective_columns(
                    &self.shared.cfg.borrow().table_columns,
                    self.w.table_scroll.width().max(1),
                );
                {
                    let mut c = self.shared.cfg.borrow_mut();
                    c.table_columns = nav::toggle_column(&shown, name, on);
                    c.save();
                }
                (self.w.apply_cols)();
            }
            Input::TableColumnsAuto => {
                {
                    let mut c = self.shared.cfg.borrow_mut();
                    c.table_columns.clear();
                    c.save();
                }
                (self.w.apply_cols)();
            }
            Input::SaveSearch(name) => {
                let q = self.w.search.text().to_string();
                match crate::ui::saved::save(&self.shared, &name, &q) {
                    Ok(()) => {
                        self.rebuild_tags();
                        sender.input(Input::Toast(trf("Saved search “{}”", &[&name])));
                    }
                    Err(e) => sender.input(Input::Toast(e)),
                }
            }
            Input::DetailsPref(on) => {
                let mut c = self.shared.cfg.borrow_mut();
                c.details_pane = on;
                c.save();
            }
            Input::TableSorted(k, asc) => {
                {
                    let mut c = self.shared.cfg.borrow_mut();
                    if c.sort_key == k && c.sort_ascending == asc {
                        return;
                    }
                    c.sort_key = k;
                    c.sort_ascending = asc;
                    c.save();
                }
                self.sync_sort_controls();
                self.requery();
            }
            Input::ToastUndo(t, undo) => {
                *self.w.last_undo.borrow_mut() = Some(undo.clone());
                eprintln!("toast: {t} [undo]");
                super::dialog_toast::show_undo(&self.w.toasts, &t, move || (undo.0)());
            }
            Input::Toast(t) => {
                let _ = sender.output(Output::Toast(t));
            }
            Input::FocusSearch => {
                self.w.search_bar.set_search_mode(true);
                self.w.search.grab_focus();
            }
            Input::Dev(c) => {
                if self.dev_wave9(&c, &sender) {
                    return;
                }
                match c.as_str() {
                    "table" => sender.input(Input::SetView(ViewMode::Table)),
                    "grid" => sender.input(Input::SetView(ViewMode::Grid)),
                    // `mailcfg` points the e-mail settings at the fake server of the UI tests
                    // (`$BOOKX_SMTP_PORT`), `emailsel` mails the selected books
                    "mailcfg" => {
                        let port = std::env::var("BOOKX_SMTP_PORT")
                            .ok()
                            .and_then(|p| p.parse().ok())
                            .unwrap_or(2525);
                        let mut c = self.shared.cfg.borrow_mut();
                        c.email = crate::emailing::EmailSettings {
                            host: "127.0.0.1".into(),
                            port,
                            encryption: crate::emailing::Encryption::None,
                            from: "me@example.com".into(),
                            to: "friend@example.org".into(),
                            ..Default::default()
                        };
                    }
                    "emailsel" => sender.input(Input::Act(Act::Email)),
                    "select1" => {
                        let m = self.active_model();
                        let _ = m.select_item(0, true);
                    }
                    "search-toggle" => {
                        let on = !self.w.search_bar.is_search_mode();
                        self.w.search_bar.set_search_mode(on);
                    }
                    // the Details button of the selection bar in a narrow window
                    "details-open" => self.w.split_details.set_show_sidebar(true),
                    // reading state for the banner tests: the first book was read to 40 % just now
                    "lastread" => {
                        {
                            let mut lib = self.shared.lib.borrow_mut();
                            if let Some(b) = lib.books.first_mut() {
                                b.last_read = Some(chrono::Utc::now());
                                b.progress = 0.4;
                            }
                            let _ = lib.save();
                        }
                        sender.input(Input::Refresh);
                    }
                    // four books being read (the hero shows the first, the others as chips)
                    "lastread-many" => {
                        {
                            let mut lib = self.shared.lib.borrow_mut();
                            let now = chrono::Utc::now();
                            for (i, b) in lib.books.iter_mut().take(4).enumerate() {
                                b.last_read = Some(now - chrono::Duration::minutes(i as i64));
                                b.progress = [0.4, 0.25, 0.6, 0.1][i];
                            }
                            let _ = lib.save();
                        }
                        sender.input(Input::Refresh);
                    }
                    // `banner-mode=large|compact|hidden|auto`, `banner` prints how it looks now
                    other if other.starts_with("banner-mode=") => {
                        {
                            let mut c = self.shared.cfg.borrow_mut();
                            c.banner_mode = match &other[12..] {
                                "auto" => String::new(),
                                m => m.to_string(),
                            };
                            c.save();
                        }
                        self.update_banner();
                    }
                    "banner-focus-chip" => {
                        eprintln!("banner-focus-chip: {}", self.w.banner_ctl.focus_chip())
                    }
                    // where the header's widgets are against the header: nothing may stick out
                    "header" => {
                        eprintln!("{}", header_report(&self.w.header));
                        let w = self.w.split_tags.root().map_or(0, |r| r.width());
                        let want = crate::library_nav::window_layout(w);
                        eprintln!(
                            "layout: window={w} sidebar_overlay={} details_overlay={} expected={}/{} content={}",
                            self.w.split_tags.is_collapsed(),
                            self.w.split_details.is_collapsed(),
                            want.sidebar_overlay,
                            want.details_overlay,
                            self.w.toolbar.width()
                        );
                        let minw = |w: &gtk::Widget| w.measure(gtk::Orientation::Horizontal, -1).0;
                        eprintln!(
                            "minwidth: header={} search={} details_split={} bar={} toolbar={}",
                            minw(self.w.header.upcast_ref()),
                            minw(self.w.search_bar.upcast_ref()),
                            minw(self.w.split_details.upcast_ref()),
                            minw(self.w.banner.upcast_ref()),
                            minw(self.w.toolbar.upcast_ref()),
                        );
                        let minh = |w: &gtk::Widget| w.measure(gtk::Orientation::Vertical, -1).0;
                        eprintln!(
                            "minheight: header={} toolbar={} details_split={} sidebar={}",
                            minh(self.w.header.upcast_ref()),
                            minh(self.w.toolbar.upcast_ref()),
                            minh(self.w.split_details.upcast_ref()),
                            minh(self.w.split_tags.upcast_ref()),
                        );
                    }
                    "banner" => eprintln!(
                        "banner: {} mode={:?}",
                        self.w.banner_ctl.describe(),
                        self.shared.cfg.borrow().banner_mode
                    ),
                    // `viewport=<height>x<width>` plans the banner for that room, `viewport=` for the real one
                    other if other.starts_with("viewport=") => {
                        let v = other[9..].split_once('x').and_then(|(h, w)| {
                            Some((h.parse::<f64>().ok()?, w.parse::<f64>().ok()?))
                        });
                        let mode = self.shared.cfg.borrow().banner_mode.clone();
                        self.w.banner_ctl.force_viewport(v, &mode);
                    }
                    // the banner's visit state: `hero-reset` (a cold start), `hero-leave` (left Home),
                    // `hero-away` (left Home 31 minutes ago)
                    "hero-reset" => {
                        *self.hero.borrow_mut() = nav::HeroSession::default();
                        self.update_banner();
                    }
                    "hero-leave" => {
                        self.leave_home();
                        self.update_banner();
                    }
                    "hero-away" => {
                        {
                            let mut h = self.hero.borrow_mut();
                            h.active = false;
                            h.last_hero = Some(chrono::Utc::now().timestamp() - 31 * 60);
                        }
                        self.update_banner();
                    }
                    // `resize=<w>x<h>` resizes the window
                    other if other.starts_with("resize=") => {
                        if let (Some(win), Some((w, h))) = (
                            window_of(&self.root),
                            other[7..].split_once('x').and_then(|(w, h)| {
                                Some((w.parse::<i32>().ok()?, h.parse::<i32>().ok()?))
                            }),
                        ) {
                            win.set_default_size(w, h);
                        }
                    }
                    // prints `banner:` every 600 ms (for the tests that resize the window with the pointer)
                    "banner-watch" => {
                        let tx = self.sender.input_sender().clone();
                        glib::timeout_add_local(std::time::Duration::from_millis(600), move || {
                            tx.emit(Input::Dev("banner".into()));
                            glib::ControlFlow::Continue
                        });
                    }
                    // `banner-scroll=<px>`: the wheel by that many pixels (the banner first, then the books)
                    other if other.starts_with("banner-scroll=") => {
                        let px: f64 = other[14..].parse().unwrap_or(0.0);
                        if !self.w.banner_ctl.wheel(px) {
                            let a = self.grid_scroll_adjustment();
                            a.set_value(a.value() + px);
                        }
                    }
                    // `stalls` sorts, searches and clears the search with a main-loop watcher running and prints
                    // the longest time the loop was blocked in each (`stalls: sort=N search=N clear=N sortback=N max=N`)
                    "stalls" => self.dev_stalls(&sender),
                    // a rebuild, then select all before the rest of the list has arrived
                    "fillselect" => {
                        self.requery();
                        let during = self.filling();
                        let early = self.grid.len();
                        sender.input(Input::SelectAll);
                        eprintln!(
                            "fillselect: filling={during} loaded={early} books={}",
                            self.card_ids.len()
                        );
                    }
                    // `scroll-to=PX`: the cover grid to that offset
                    other if other.starts_with("scroll-to=") => {
                        let px: f64 = other[10..].parse().unwrap_or(0.0);
                        self.grid_vadjustment().set_value(px);
                    }
                    "fillstate" => eprintln!(
                        "fillstate: tiles={} view={}x{} adj={:.0}/{:.0}/{:.0} grid={} table={} filling={} books={}",
                        {
                            let mut n = 0;
                            let mut c = self.grid.view.first_child();
                            while let Some(w) = c {
                                n += 1;
                                c = w.next_sibling();
                            }
                            n
                        },
                        self.grid.view.width(),
                        self.grid.view.height(),
                        self.grid_vadjustment().value(),
                        self.grid_vadjustment().page_size(),
                        self.grid_vadjustment().upper(),
                        self.grid.len(),
                        self.table.len(),
                        self.filling(),
                        self.card_ids.len()
                    ),
                    // `requery-time` rebuilds the grid and table models and prints how long it took
                    // (the scale scenario, docs/performance.md)
                    "requery-time" => {
                        let t = std::time::Instant::now();
                        self.requery();
                        self.finish_fill();
                        eprintln!(
                            "requery_time: books={} ms={}",
                            self.card_ids.len(),
                            t.elapsed().as_millis()
                        );
                    }
                    // `grid-bench=<frames>` scrolls the cover grid by a screen every frame and prints the
                    // frame intervals: `grid_bench: frames=N avg_ms=A max_ms=M wall_ms=W moved=P`
                    other if other.starts_with("grid-bench=") => {
                        let frames: u32 = other["grid-bench=".len()..].parse().unwrap_or(60);
                        // the grid view is the scrolled window's direct child
                        let a = self
                            .grid
                            .view
                            .ancestor(gtk::ScrolledWindow::static_type())
                            .and_downcast::<gtk::ScrolledWindow>()
                            .map(|s| s.vadjustment())
                            .unwrap_or_else(|| self.grid_scroll_adjustment());
                        let step = (a.page_size() / 3.0).max(40.0);
                        let start = (a.value(), std::time::Instant::now());
                        let state = Rc::new(std::cell::RefCell::new((0u32, 0i64, 0i64, 0i64)));
                        let slow = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
                        self.grid.view.add_tick_callback(move |_, clock| {
                            let mut s = state.borrow_mut();
                            let now = clock.frame_time();
                            if s.1 != 0 {
                                let dt = now - s.1;
                                s.2 += dt;
                                s.3 = s.3.max(dt);
                                if dt > 250_000 && slow.borrow().len() < 8 {
                                    slow.borrow_mut().push(format!("{}:{}ms", s.0, dt / 1000));
                                }
                            }
                            s.1 = now;
                            s.0 += 1;
                            a.set_value(a.value() + step);
                            if s.0 >= frames {
                                eprintln!(
                                    "grid_bench: frames={} avg_ms={:.1} max_ms={:.1} wall_ms={} moved={:.0} slow=[{}]",
                                    s.0,
                                    s.2 as f64 / 1000.0 / (s.0.max(2) - 1) as f64,
                                    s.3 as f64 / 1000.0,
                                    start.1.elapsed().as_millis(),
                                    a.value() - start.0,
                                    slow.borrow().join(" ")
                                );
                                return glib::ControlFlow::Break;
                            }
                            glib::ControlFlow::Continue
                        });
                    }
                    // where the first book is and how far the books are scrolled
                    "gridpos" => {
                        let a = self.grid_scroll_adjustment();
                        let top = self
                            .grid
                            .view
                            .compute_bounds(&self.w.content)
                            .map(|b| b.y())
                            .unwrap_or(-1.0);
                        eprintln!("gridpos: scrolled={:.0} view_top={top:.0}", a.value());
                    }
                    // the sidebar views: `nav=recent`
                    other if other.starts_with("nav=") => {
                        sender.input(Input::SetNav(View::from_key(&other[4..])));
                    }
                    "favourite" => self.do_act(Act::Favourite),
                    // `rate=<0-10>` rates the selection, `tags=+a,b` / `tags=-a` edits the tags of the selection
                    other if other.starts_with("rate=") => {
                        let v: u8 = other[5..].parse().unwrap_or(0);
                        sender.input(Input::SetRating(self.selected_books(), v, false));
                    }
                    other if other.starts_with("tags=") => {
                        let spec = &other[5..];
                        let (add, remove) = match spec.chars().next() {
                            Some('-') => (Vec::new(), crate::tags::parse_entry(&spec[1..])),
                            Some('+') => (crate::tags::parse_entry(&spec[1..]), Vec::new()),
                            _ => (crate::tags::parse_entry(spec), Vec::new()),
                        };
                        sender.input(Input::EditTags {
                            ids: self.selected_books(),
                            add,
                            remove,
                            in_panel: false,
                        });
                    }
                    // the star badges on the covers and what the selection has
                    // `audiobadges`: the headphones badges on the covers and the sidebar's Audiobooks row
                    "audiobadges" => {
                        let mut n = 0;
                        super::audiobook_player::audio_badges(self.grid.view.upcast_ref(), &mut n);
                        let row = super::audiobook_player::has_named(
                            self.w.tags.upcast_ref(),
                            "audiobooks-row",
                        );
                        eprintln!("audiobadges: visible={n} sidebar_row={row}");
                    }
                    "favbadges" => {
                        let mut n = 0;
                        count_badges(self.grid.view.upcast_ref(), &mut n);
                        let info: Vec<String> = self
                            .selected
                            .iter()
                            .filter_map(|id| {
                                self.shared.lib.borrow().get(*id).map(|b| {
                                    format!(
                                        "rating={} tags={}",
                                        b.meta.rating,
                                        b.meta.tags.join("|")
                                    )
                                })
                            })
                            .collect();
                        eprintln!(
                            "favbadges: visible={n} favourites={} selected=[{}]",
                            self.shared.cfg.borrow().favourites.len(),
                            info.join("; ")
                        );
                    }
                    other if other.starts_with("togglefav=") => {
                        let shown = self.visible_ids();
                        if let Some(id) =
                            other[10..].parse::<usize>().ok().and_then(|p| shown.get(p))
                        {
                            sender.input(Input::ToggleFav(*id));
                        }
                    }
                    "properties" => self.do_act(Act::Properties),
                    // editions of one work: `works=on|off`, `works=link:1,2`, `works=unlink:ID`,
                    // `works=prefer:ID`, `works=card:ID[:ctrl]` (click that card), `works=show:ID`
                    // (its edition list), `works=cards` and `works=selected` print what is there
                    other if other.starts_with("works=") => self.dev_works(&other[6..], &sender),
                    // what a click does: `click=<position>[:ctrl|shift]` on the n-th book shown,
                    // `rightclick=<position>`, `click-empty` (the handlers the gestures call)
                    "click-empty" => sender.input(Input::ClearSelection),
                    other if other.starts_with("click=") || other.starts_with("rightclick=") => {
                        let right = other.starts_with("rightclick=");
                        let arg = other.split_once('=').map(|x| x.1).unwrap_or("");
                        let (pos, how) = arg.split_once(':').unwrap_or((arg, ""));
                        let shown = self.visible_ids();
                        if let Some(id) = pos.parse::<usize>().ok().and_then(|p| shown.get(p)) {
                            if right {
                                let (sel, _) = nav::context_selection(&self.selected, *id);
                                self.selected = sel;
                                self.restore_selection();
                                self.follow_selection();
                                self.update_details();
                                self.update_bottom_bar();
                            } else {
                                let how = match how {
                                    "ctrl" => nav::Pick::Ctrl,
                                    "shift" => nav::Pick::Shift,
                                    _ => nav::Pick::Plain,
                                };
                                sender.input(Input::Pick(*id, how));
                            }
                        }
                    }
                    // the selected books and the panel, for the tests
                    // `selhdr`: the header and the bar while books are selected
                    "selhdr" => {
                        let w = &self.w;
                        let mut bar: Vec<String> = Vec::new();
                        for (name, shown) in [
                            ("read", w.sel_read.is_visible()),
                            ("edit", w.sel_edit.is_visible()),
                            ("shelf", w.sel_shelf.is_visible()),
                            ("send", w.sel_send.is_visible()),
                            ("convert", w.sel_convert.is_visible()),
                            ("more", w.sel_more.is_visible()),
                        ] {
                            if shown {
                                bar.push(name.to_string());
                            }
                        }
                        let mut more = Vec::new();
                        if let Some(m) = w.sel_more.menu_model() {
                            for i in 0..m.n_items() {
                                if let Some(l) = m
                                    .item_attribute_value(i, "label", None)
                                    .and_then(|v| v.str().map(String::from))
                                {
                                    more.push(l);
                                }
                            }
                        }
                        eprintln!(
                            "selhdr: mode={} title={:?} cancel={} bar={} buttons={} more=[{}]",
                            w.title_stack.visible_child_name().unwrap_or_default(),
                            if w.title_stack.visible_child_name().as_deref() == Some("selection") {
                                w.sel_title.title().to_string()
                            } else {
                                w.title.title().to_string()
                            },
                            w.cancel_btn.is_visible(),
                            w.toolbar.reveals_bottom_bars(),
                            bar.join(","),
                            more.join("|")
                        );
                    }
                    "viewopts" => eprintln!(
                        "viewopts: icon={} grid_only_visible={} order={:?}",
                        self.w.view_btn.icon_name().unwrap_or_default(),
                        self.w.grid_only.iter().any(|w| w.property::<bool>("visible")),
                        order_label(self.shared.cfg.borrow().sort_ascending)
                    ),
                    "searchph" => eprintln!(
                        "searchph: placeholder={:?}",
                        self.w.search.placeholder_text().unwrap_or_default()
                    ),
                    "cancel-selection" => self.w.cancel_btn.emit_clicked(),
                    "compact" | "regular" => sender.input(Input::Compact(c == "compact")),
                    "selection" => {
                        let shown = self.visible_ids();
                        let pos: Vec<usize> = self
                            .current_selection()
                            .iter()
                            .filter_map(|id| shown.iter().position(|s| s == id))
                            .collect();
                        eprintln!(
                            "selection: {:?} | model={:?} | panel={}",
                            pos,
                            self.selected.len(),
                            self.w.split_details.shows_sidebar()
                        );
                    }
                    // the size of the cover tiles, how many fit in a row and how often the list was
                    // rebuilt (opening a panel must change the columns, never the tile)
                    "tiles" => eprintln!("tiles: {}", self.tile_report()),
                    // every icon name used by the widgets of the windows must exist in the icon theme
                    "icons" => {
                        let mut seen = Vec::new();
                        for w in gtk::Window::list_toplevels() {
                            collect_icons(&w, &mut seen);
                        }
                        let theme = gtk::IconTheme::for_display(&gdk::Display::default().unwrap());
                        let missing: Vec<&String> =
                            seen.iter().filter(|n| !theme.has_icon(n)).collect();
                        eprintln!("icons: checked={} missing={:?}", seen.len(), missing);
                    }
                    "sidebar-toggle" => {
                        let on = !self.w.split_tags.shows_sidebar();
                        self.w.split_tags.set_show_sidebar(on);
                    }
                    "sidebar" => {
                        let mut rows = Vec::new();
                        let mut c = self.w.top.first_child();
                        while let Some(w) = c {
                            rows.push(w.widget_name().to_string());
                            c = w.next_sibling();
                        }
                        eprintln!(
                            "sidebar: view={} rows={} counts={} panel={} collapsed={}",
                            self.view.key(),
                            rows.join(","),
                            self.w
                                .top_counts
                                .iter()
                                .map(|l| l.label().to_string())
                                .collect::<Vec<_>>()
                                .join(","),
                            self.w.split_tags.shows_sidebar(),
                            self.w.split_tags.is_collapsed()
                        );
                    }
                    "menus" => {
                        let items: Vec<String> = MAIN_MENU
                            .iter()
                            .filter(|(l, _)| !l.is_empty())
                            .map(|(l, a)| format!("{l} [{a}]"))
                            .collect();
                        eprintln!("mainmenu: {}", items.join(" | "));
                        let lib: Vec<String> = LIBRARY_MENU
                            .iter()
                            .filter(|(l, _)| !l.is_empty())
                            .map(|(l, a)| format!("{l} [{a}]"))
                            .collect();
                        eprintln!("librarymenu: {}", lib.join(" | "));
                        eprintln!(
                            "menu_counts: main={} library={} add={} main_keys={}",
                            MAIN_MENU.iter().filter(|(l, _)| !l.is_empty()).count(),
                            LIBRARY_MENU.iter().filter(|(l, _)| !l.is_empty()).count(),
                            menu_top_count(self.w.add_menu.upcast_ref::<gio::MenuModel>()),
                            {
                                let mut l = Vec::new();
                                menu_labels(build_entry_menu(&MAIN_MENU).upcast_ref(), &mut l);
                                l.join("|")
                            }
                        );
                        let mut welcome = Vec::new();
                        let mut c = self.w.empty_btns.first_child();
                        while let Some(w) = c {
                            let inner = w
                                .downcast_ref::<gtk::FlowBoxChild>()
                                .and_then(|fc| fc.child())
                                .unwrap_or_else(|| w.clone());
                            if let Some(b) = inner.downcast_ref::<gtk::Button>() {
                                welcome.push(format!(
                                    "{} [{}]",
                                    b.label().unwrap_or_default(),
                                    b.action_name().unwrap_or_default()
                                ));
                            }
                            c = w.next_sibling();
                        }
                        eprintln!("welcome: {}", welcome.join(" | "));
                    }
                    // `import-highlights=<path;path>` reads and plans like the dialog (the review
                    // is then confirmed with `import-confirm`), `export-all=<format>:<folder>`
                    other if other.starts_with("import-highlights=") => {
                        if let Some(p) = window_of(&self.root) {
                            let paths: Vec<std::path::PathBuf> = other
                                ["import-highlights=".len()..]
                                .split(';')
                                .map(Into::into)
                                .collect();
                            let (t1, t2) = (
                                self.sender.input_sender().clone(),
                                self.sender.input_sender().clone(),
                            );
                            super::annotations::run(
                                &p,
                                &self.shared,
                                paths,
                                Rc::new(move |m| t1.emit(Input::Toast(m))),
                                Rc::new(move || t2.emit(Input::Refresh)),
                            );
                        }
                    }
                    "import-confirm" => super::annotations::confirm_pending(),
                    "notes" => self.do_act(Act::Notes),
                    other if other.starts_with("notes:") => {
                        super::notes::dev_command(&other["notes:".len()..])
                    }
                    // `export-calibre=<folder>` runs the Calibre export job without the folder dialog
                    other if other.starts_with("export-calibre=") => {
                        let t = self.sender.input_sender().clone();
                        super::calibre_export::run(
                            &self.shared,
                            std::path::PathBuf::from(&other["export-calibre=".len()..]),
                            Rc::new(move |m| t.emit(Input::Toast(m))),
                        );
                    }
                    other if other.starts_with("export-all=") => {
                        if let Some((id, dir)) = other["export-all=".len()..].split_once(':') {
                            let m = super::annotations::export_all_to(
                                &self.shared,
                                id,
                                std::path::Path::new(dir),
                            );
                            eprintln!("export_all: {m}");
                        }
                    }
                    "autosync=on" => {
                        let mut c = self.shared.cfg.borrow_mut();
                        c.device_sync.auto_sync = true;
                        c.save();
                    }
                    "forget-devices" => {
                        self.devices.clear();
                        self.declined.clear();
                    }
                    // activates the menu entry like a click would (nothing happens while it is disabled)
                    "send-to-device" => {
                        let ids = self.selected.clone();
                        super::devices::set_library_selection(&ids);
                        if self.w.send_action.is_enabled() {
                            self.w.send_action.activate(None);
                        } else {
                            eprintln!("send_to_device: disabled");
                        }
                    }
                    "ctxmenu" => {
                        let mut labels = Vec::new();
                        menu_labels(self.book_menu(false).upcast_ref(), &mut labels);
                        eprintln!("ctxmenu: {}", labels.join(" | "));
                        let menu = self.book_menu(false);
                        eprintln!(
                            "ctxmenu_send: enabled={} tooltip={}",
                            self.w.send_action.is_enabled(),
                            menu_attr(menu.upcast_ref(), "Send to Device", "tooltip")
                                .unwrap_or_default()
                        );
                        eprintln!("ctxmenu_top: {}", menu_top_count(menu.upcast_ref()));
                        // the fullest variant: several books, on a shelf
                        let busiest = self.book_menu(true);
                        eprintln!(
                            "ctxmenu_shape: single_top={} multi_top={} depth={} {}",
                            menu_top_count(menu.upcast_ref()),
                            menu_top_count(busiest.upcast_ref()),
                            menu_submenu_depth(menu.upcast_ref())
                                .max(menu_submenu_depth(busiest.upcast_ref())),
                            menu_submenu_sizes(menu.upcast_ref())
                        );
                    }
                    "unselect" => {
                        let _ = self.active_model().unselect_all();
                    }
                    "narrow" | "wide" => {
                        // what the breakpoint does on a narrow window
                        let on = c == "narrow";
                        self.w.split_details.set_collapsed(on);
                        self.w.split_tags.set_collapsed(on);
                        if on {
                            self.w.split_tags.set_show_sidebar(false);
                            self.w.split_details.set_show_sidebar(false);
                        } else {
                            self.w.split_tags.set_show_sidebar(true);
                            self.w.split_details.set_show_sidebar(false);
                        }
                        self.update_bottom_bar();
                    }
                    "devices-scan" => {
                        let list = crate::devices::detect_physical();
                        self.devices_changed(list);
                    }
                    "device-banner=sync" => self.device_banner_answer(DeviceChoice::Sync),
                    "device-banner=dismiss" => self.device_banner_answer(DeviceChoice::Dismiss),
                    // old hook names stay as aliases; neither changes the auto-sync setting
                    "device-banner=always" | "device-banner=later" => {
                        self.device_banner_answer(DeviceChoice::Dismiss)
                    }
                    "device-button" => {
                        if let Some(app) = gio::Application::default() {
                            gio::prelude::ActionGroupExt::activate_action(&app, "devices", None);
                        }
                    }
                    "dump" => {
                        let label = self
                            .w
                            .device_btn
                            .child()
                            .and_downcast::<adw::ButtonContent>()
                            .map(|c| c.label().to_string())
                            .unwrap_or_default();
                        eprintln!(
                            "device_button: visible={} label={label}",
                            self.w.device_btn.is_visible()
                        );
                        eprintln!(
                            "device_banner: widget=AdwBanner title={} button={} dismiss=true revealed={}",
                            self.w.device_banner.title(),
                            self.w.device_banner.button_label().unwrap_or_default(),
                            self.w.device_banner.is_revealed()
                        );
                        eprintln!(
                            "details_auto: {} | auto_sync: {} | include_notes: {}",
                            self.shared.cfg.borrow().sidebar_view,
                            self.shared.cfg.borrow().device_sync.auto_sync,
                            self.shared.cfg.borrow().device_sync.include_notes
                        );
                        eprintln!(
                            "close_button_in_header: {}",
                            self.w.header.shows_end_title_buttons()
                        );
                        eprintln!("browse_panel: {}", self.w.split_tags.shows_sidebar());
                        eprintln!("view: {}", self.view.key());
                        eprintln!("details_panel: {}", self.w.split_details.shows_sidebar());
                        eprintln!("search_bar: {}", self.w.search_bar.is_search_mode());
                        eprintln!("selection_bar: {}", self.w.toolbar.reveals_bottom_bars());
                        eprintln!("banner: {}", self.w.banner.reveals_child());
                        eprintln!(
                            "title: {} | {}",
                            self.w.title.title(),
                            self.w.title.subtitle()
                        );
                    }
                    // `send=<text in the device path>` sends the first book to that device, synchronously
                    other if other.starts_with("send=") => {
                        let settings = self.shared.cfg.borrow().device.clone();
                        let drivers = crate::devices::drivers(&settings);
                        let dev = drivers
                            .detect()
                            .into_iter()
                            .find(|d| d.mount.to_string_lossy().contains(&other[5..]));
                        let book = self.shared.lib.borrow().books.first().map(|b| {
                            (
                                self.shared.lib.borrow().book_path(b),
                                crate::devices::fields_of(&b.meta),
                            )
                        });
                        match (dev, book) {
                            (Some(d), Some((p, f))) => {
                                let r = crate::devices::send_full(
                                    &drivers,
                                    &d,
                                    &p,
                                    &f,
                                    &crate::devices::SendOptions::from(&settings),
                                    None,
                                )
                                .map(|s| {
                                    format!(
                                        "{} ({}){}",
                                        s.path.display(),
                                        s.format,
                                        s.note.map(|n| format!(" note={n}")).unwrap_or_default()
                                    )
                                })
                                .map_err(|e| format!("{e:#}"));
                                eprintln!("send: {r:?}");
                            }
                            _ => eprintln!("send: no device or book"),
                        }
                    }
                    // `annotate` gives the first book one highlight with a note and one bookmark
                    "annotate" => {
                        let mut lib = self.shared.lib.borrow_mut();
                        if let Some(b) = lib.books.first_mut() {
                            b.highlights.push(bookx_core::library::Highlight {
                                id: "dev-h".into(),
                                spine: 0,
                                start: 0,
                                end: 5,
                                color: "yellow".into(),
                                style: String::new(),
                                note: "devnote remember this".into(),
                                text: "devhighlight text".into(),
                                created: chrono::Utc::now(),
                            });
                            b.bookmarks.push(bookx_core::library::Bookmark {
                                id: "dev-b".into(),
                                spine: 0,
                                frac: 0.25,
                                label: "devbookmark".into(),
                                created: chrono::Utc::now(),
                            });
                        }
                        let _ = lib.save();
                        eprintln!("annotate: done");
                    }
                    // `annotate-more` gives the first book three more highlights (a green one without
                    // a note, a PDF-style one on page 5) and the second book one blue one
                    "annotate-more" => {
                        let mut lib = self.shared.lib.borrow_mut();
                        let mk = |id: &str,
                                  spine: usize,
                                  color: &str,
                                  style: &str,
                                  note: &str,
                                  text: &str,
                                  day: u32| {
                            bookx_core::library::Highlight {
                                id: id.into(),
                                spine,
                                start: 0,
                                end: 5,
                                color: color.into(),
                                style: style.into(),
                                note: note.into(),
                                text: text.into(),
                                created: chrono::TimeZone::with_ymd_and_hms(
                                    &chrono::Utc,
                                    2026,
                                    10,
                                    day,
                                    8,
                                    0,
                                    0,
                                )
                                .unwrap(),
                            }
                        };
                        if let Some(b) = lib.books.first_mut() {
                            b.highlights.push(mk(
                                "m1",
                                1,
                                "green",
                                "",
                                "",
                                "greenquote about functionality",
                                2,
                            ));
                            b.highlights.push(mk(
                                "m2",
                                4,
                                "yellow",
                                "pdf|background|0,0,1,1",
                                "pdfnote",
                                "pdfquote on a page",
                                3,
                            ));
                        }
                        if let Some(b) = lib.books.get_mut(1) {
                            b.highlights.push(mk(
                                "m3",
                                0,
                                "blue",
                                "",
                                "bluenote",
                                "bluequote in the other book",
                                9,
                            ));
                        }
                        let _ = lib.save();
                        eprintln!("annotate_more: done");
                    }
                    // `sendnotes=<text in the device path>`: like `send=` but with the annotations
                    other if other.starts_with("sendnotes=") => {
                        let drv = crate::devices::driver();
                        let dev = bookx_core::contracts::DeviceDriver::detect(&drv)
                            .into_iter()
                            .find(|d| d.mount.to_string_lossy().contains(&other[10..]));
                        let book = self.shared.lib.borrow().books.first().map(|b| {
                            (
                                self.shared.lib.borrow().book_path(b),
                                b.meta.title.clone(),
                                crate::send_notes::Notes::new(&b.highlights, &b.bookmarks),
                            )
                        });
                        match (dev, book) {
                            (Some(d), Some((p, t, n))) => {
                                let r = crate::devices::send_book_with(&drv, &d, &p, &t, Some(&n))
                                    .map(|s| {
                                        format!(
                                            "{} sidecar={}",
                                            s.path.display(),
                                            s.sidecar.is_some()
                                        )
                                    })
                                    .map_err(|e| format!("{e:#}"));
                                eprintln!("sendnotes: {r:?}");
                            }
                            _ => eprintln!("sendnotes: no device or book"),
                        }
                    }
                    // `targets` lists the output formats of the Convert To menu that plugins add
                    "targets" => {
                        let labels: Vec<_> = crate::plugins::writer_targets()
                            .iter()
                            .chain(crate::plugins::tool_targets().iter())
                            .map(|t| format!("{}={}", t.id, t.label))
                            .collect();
                        eprintln!("targets: {}", labels.join(" | "));
                    }
                    "devices" => {
                        let n = crate::devices::detect_all().len();
                        eprintln!("devices: {n}");
                    }
                    // `convertto=<target>:<path>` converts the selected (or first) book without dialogs
                    other if other.starts_with("convertto=") => {
                        if let Some((id, dest)) = other[10..].rsplit_once(':') {
                            let book =
                                self.selected.first().copied().or_else(|| {
                                    self.shared.lib.borrow().books.first().map(|b| b.id)
                                });
                            if let (Some(t), Some(book)) = (crate::export::target(id), book) {
                                let src = {
                                    let lib = self.shared.lib.borrow();
                                    lib.get(book).map(|b| lib.book_path(b))
                                };
                                let res = src.map(|s| {
                                    crate::export::convert_epub_file(&s, t)
                                        .map_err(|e| format!("{e:#}"))
                                        .and_then(|b| {
                                            let report = self
                                                .shared
                                                .cfg
                                                .borrow()
                                                .metadata_prefs
                                                .conversion_report;
                                            let title = self
                                                .shared
                                                .lib
                                                .borrow()
                                                .get(book)
                                                .map(|b| b.meta.title.clone())
                                                .unwrap_or_default();
                                            crate::preflight::write_converted(
                                                std::path::Path::new(dest),
                                                &title,
                                                t,
                                                &s,
                                                &b,
                                                report,
                                            )
                                            .map_err(|e| e.to_string())
                                        })
                                });
                                eprintln!("convertto: {id} -> {:?}", res);
                            }
                        }
                    }
                    // `dups` lists duplicate groups, `mergedups` merges every group (keeping the default)
                    other if other.starts_with("health=") => {
                        if let Some(win) = window_of(&self.root) {
                            let notify = self.health_notify();
                            crate::ui::health::dev(
                                win.upcast_ref(),
                                &self.shared,
                                notify,
                                &other[7..],
                            );
                        }
                    }
                    "bulkfetch" => sender.input(Input::Act(Act::FetchMeta)),
                    "preflight" => sender.input(Input::Act(Act::Preflight)),
                    other if other.starts_with("bulkfetch=") => {
                        crate::ui::bulkfetch::dev(&other[10..])
                    }
                    other if other.starts_with("preflight=") => {
                        crate::ui::preflight::dev(&other[10..])
                    }
                    // `bulkedit=tag:+extra|publisher:Bulk Press` applies the bulk editor to the selection
                    other if other.starts_with("bulkedit=") => {
                        let ids = self.selected.clone();
                        let edit = metadata::BulkEdit::from_spec(&other[9..]);
                        match metadata::apply_bulk(&self.shared, &ids, &edit) {
                            Ok(n) => eprintln!("bulkedit: {n} books"),
                            Err(e) => eprintln!("bulkedit: failed {e:#}"),
                        }
                        sender.input(Input::Refresh);
                    }
                    other if other.starts_with("formats=") => {
                        if let Some(win) = window_of(&self.root) {
                            crate::ui::formats::dev(
                                &win,
                                &self.shared,
                                self.health_notify(),
                                &other[8..],
                            );
                        }
                    }
                    other if other.starts_with("authors=") => {
                        if let Some(win) = window_of(&self.root) {
                            let (search, _, notify) = self.organize_callbacks();
                            crate::ui::authors::dev(
                                &win,
                                &self.shared,
                                search,
                                notify,
                                &other[8..],
                            );
                        }
                    }
                    other if other.starts_with("citations=") => {
                        if let Some(win) = window_of(&self.root) {
                            crate::ui::citations::dev(
                                &win,
                                &self.shared,
                                self.health_notify(),
                                &other[10..],
                            );
                        }
                    }
                    other if other.starts_with("trackers=") => {
                        if let Some(win) = window_of(&self.root) {
                            crate::ui::trackers::dev(
                                &win,
                                &self.shared,
                                self.health_notify(),
                                &other[9..],
                            );
                        }
                    }
                    other if other.starts_with("audiobooks=") => {
                        if let Some(win) = window_of(&self.root) {
                            crate::ui::audiobooks::dev(
                                &win,
                                &self.shared,
                                self.health_notify(),
                                &other[11..],
                            );
                        }
                    }
                    other if other.starts_with("organize=") => {
                        if let Some(win) = window_of(&self.root) {
                            let (search, fetch, notify) = self.organize_callbacks();
                            crate::ui::organize::dev(
                                &win,
                                &self.shared,
                                search,
                                fetch,
                                notify,
                                &other[9..],
                            );
                        }
                    }
                    // `undo=N` takes back the newest N entries of the operation log
                    other if other.starts_with("undo=") => {
                        let n: usize = other[5..].parse().unwrap_or(1);
                        let mut done = 0;
                        for _ in 0..n {
                            if self
                                .shared
                                .lib
                                .borrow_mut()
                                .undo_last()
                                .ok()
                                .flatten()
                                .is_some()
                            {
                                done += 1;
                            }
                        }
                        eprintln!("undo: {done}");
                        self.lib_changed();
                        sender.input(Input::Refresh);
                    }
                    "dupsapart" | "dupsvisible" => {
                        let root = self.shared.lib.borrow().root.clone();
                        let mut apart = crate::duplicates::Apart::load(&root);
                        let groups = crate::duplicates::find(&self.shared.lib.borrow().books);
                        if c == "dupsapart" {
                            for g in &groups {
                                apart.keep_apart(&g.ids);
                            }
                            let saved = apart.save(&root).is_ok();
                            eprintln!("dupsapart: {} groups saved={saved}", groups.len());
                        }
                        eprintln!("dupsvisible: {}", apart.hide(groups).len());
                    }
                    "dups" => {
                        let groups = crate::duplicates::find(&self.shared.lib.borrow().books);
                        eprintln!(
                            "dups: {} groups {:?}",
                            groups.len(),
                            groups
                                .iter()
                                .map(|g| (g.ids.clone(), g.keep))
                                .collect::<Vec<_>>()
                        );
                    }
                    other if other.starts_with("newsadd=") => {
                        let mut f = other[8..].split('|');
                        let (title, url) = (f.next().unwrap_or(""), f.next().unwrap_or(""));
                        let sched = f.next().unwrap_or("manual");
                        let keep = f.next().and_then(|k| k.parse().ok());
                        let r = self.dev_news_add(title, url, sched, keep);
                        eprintln!("newsadd: {r:?}");
                    }
                    other if other.starts_with("newsopml=") => {
                        let mut f = other[9..].split('|');
                        let (title, path) = (f.next().unwrap_or(""), f.next().unwrap_or(""));
                        let r = std::fs::read_to_string(path)
                            .map_err(|e| e.to_string())
                            .and_then(|t| crate::news::parse_opml(&t).map_err(|e| format!("{e:#}")))
                            .and_then(|feeds| {
                                let urls: Vec<String> = feeds.into_iter().map(|f| f.1).collect();
                                self.dev_news_add(title, &urls.join("\n"), "manual", None)
                            });
                        eprintln!("newsopml: {r:?}");
                    }
                    other if other.starts_with("newsfeedoff=") => {
                        let mut f = other[12..].split('|');
                        let (key, url) = (f.next().unwrap_or(""), f.next().unwrap_or(""));
                        let mut st = self.news.store.borrow_mut();
                        let r = st.get(key).cloned().map(|mut s| {
                            s.disabled_feeds.push(url.to_string());
                            st.update(s).map_err(|e| format!("{e:#}"))
                        });
                        eprintln!("newsfeedoff: {r:?}");
                    }
                    "newsrun=all" => {
                        let keys: Vec<String> = self
                            .news
                            .store
                            .borrow()
                            .subs
                            .iter()
                            .map(|s| s.key.clone())
                            .collect();
                        sender.input(Input::NewsRun(keys));
                    }
                    "newsrun=due" | "newstick" => sender.input(Input::NewsDue),
                    "newsshow" => sender.input(Input::NewsDialog),
                    "newsdialog" => {
                        eprintln!("news_dialog: open={}", self.news.slot.borrow().is_some())
                    }
                    "newsdump" => {
                        let off = *chrono::Local::now().offset();
                        let st = self.news.store.borrow();
                        eprintln!(
                            "news: count={} running={:?} queued={}",
                            st.subs.len(),
                            self.news.running,
                            self.news.queue.len()
                        );
                        for s in &st.subs {
                            eprintln!(
                                "news_sub: key={} title={:?} feeds={} off={} schedule={:?} enabled={} keep={} attempts={} last_run={} status={:?}",
                                s.key,
                                s.recipe.title,
                                s.recipe.feeds.len(),
                                s.disabled_feeds.len(),
                                crate::news::schedule_text(s, off),
                                s.enabled,
                                s.keep_issues,
                                s.attempts,
                                s.last_run.map_or("none".to_string(), |t| t.to_rfc3339()),
                                s.last_status
                            );
                        }
                    }
                    other if other.starts_with("addweb=") => {
                        sender.input(Input::AddWeb(other[7..].to_string()));
                    }
                    "dupsshow" => sender.input(Input::FindDuplicates),
                    "mergedups" => {
                        let groups = crate::duplicates::find(&self.shared.lib.borrow().books);
                        let mut removed = 0;
                        for g in groups {
                            removed += crate::duplicates::merge(
                                &mut self.shared.lib.borrow_mut(),
                                &g.ids,
                                g.keep,
                            )
                            .unwrap_or(0);
                        }
                        eprintln!("mergedups: removed {removed}");
                        self.lib_changed();
                        sender.input(Input::Refresh);
                    }
                    "fetchmeta" => {
                        eprintln!("fetchmeta: {}", crate::ui::metadata::dev_fetch(false));
                    }
                    "fetchmeta-replace" => {
                        eprintln!("fetchmeta: {}", crate::ui::metadata::dev_fetch(true));
                    }
                    "metadump" => eprintln!("metadump: {}", crate::ui::metadata::dev_dump()),
                    other if other.starts_with("seriestext=") => eprintln!(
                        "seriestext: {}",
                        crate::ui::metadata::dev_series_text(&other[11..])
                    ),
                    "seriesdump" => {
                        eprintln!("seriesdump: {}", crate::ui::metadata::dev_series_dump())
                    }
                    "metasave" => eprintln!("metasave: {}", crate::ui::metadata::dev_save()),
                    "fulltext" => {
                        let n = self
                            .shared
                            .fulltext
                            .borrow()
                            .as_ref()
                            .map(|f| f.indexed_books());
                        eprintln!("fulltext: indexed={n:?}");
                    }
                    "editmeta" => self.do_act(Act::EditMeta),
                    "delete" => self.do_act(Act::Delete),
                    "undo-last" => {
                        let u = self.w.last_undo.borrow_mut().take();
                        match u {
                            Some(u) => (u.0)(),
                            None => eprintln!("undo-last: nothing"),
                        }
                    }
                    "selectall" => {
                        self.finish_fill();
                        self.active_model().select_all();
                    }
                    other if other.starts_with("import=") => {
                        let mut files = Vec::new();
                        crate::import::collect(std::path::Path::new(&other[7..]), true, &mut files);
                        sender.input(Input::ImportPaths(files));
                    }
                    other if other.starts_with("search=") => self.w.search.set_text(&other[7..]),
                    other if other.starts_with("opds=") => {
                        if let Some(win) = window_of(&self.root) {
                            crate::ui::opds::dev(
                                &win,
                                &self.shared,
                                self.opds_import(),
                                &other[5..],
                            );
                        }
                    }
                    other if other.starts_with("libs=") => {
                        let (out, tx) = (
                            sender.output_sender().clone(),
                            sender.input_sender().clone(),
                        );
                        let ctx = super::library_switcher::Ctx {
                            switch: Rc::new(move |p| out.emit(Output::SwitchLibrary(p))),
                            toast: Rc::new(move |m| tx.emit(Input::Toast(m))),
                        };
                        eprintln!("libs: {}", super::library_switcher::dev(&ctx, &other[5..]));
                    }
                    other if other.starts_with("shelf=") => {
                        let out =
                            super::shelves::dev(&self.shelf_ctx(), &self.selected, &other[6..]);
                        eprintln!("shelves: {out}");
                    }
                    other if other.starts_with("saved=") => {
                        if let Some(win) = window_of(&self.root) {
                            let (search, _, notify) = self.organize_callbacks();
                            crate::ui::saved::dev(&win, &self.shared, search, notify, &other[6..]);
                        }
                    }
                    // `sort=N`: the Nth sort key of the sort menu
                    other if other.starts_with("sort=") => {
                        let key = other[5..]
                            .parse::<usize>()
                            .ok()
                            .and_then(|n| SortKey::ALL.get(n).copied());
                        if let Some(k) = key {
                            sender.input(Input::Sort(k));
                        }
                    }
                    other if other.starts_with("language=") => {
                        crate::ui::language_settings::dev_set(&self.shared, &other[9..]);
                    }
                    "menus-tr" => {
                        let items: Vec<String> = MAIN_MENU
                            .iter()
                            .chain(LIBRARY_MENU.iter())
                            .filter(|(l, _)| !l.is_empty())
                            .map(|(l, _)| crate::i18n::tr(l))
                            .collect();
                        eprintln!("mainmenu_tr: {}", items.join(" | "));
                    }
                    // `go=author:=Ian Miell`: what a click on that sidebar filter row does
                    other if other.starts_with("go=") => {
                        (self.sidebar_go())(other[3..].to_string())
                    }
                    // the search bar after a click: open, the entry's text, the books shown, the focus
                    "searchstate" => {
                        let focus = self
                            .root
                            .root()
                            .and_then(|r| r.focus())
                            .map(|w| {
                                let n = w.widget_name();
                                if n.is_empty() {
                                    w.type_().name().to_string()
                                } else {
                                    n.to_string()
                                }
                            })
                            .unwrap_or_default();
                        eprintln!(
                            "searchstate: bar={} text={:?} books={} view={} focus={}",
                            self.w.search_bar.is_search_mode(),
                            self.w.search.text(),
                            self.visible_ids().len(),
                            self.view.key(),
                            focus
                        );
                    }
                    "visible" => {
                        let ids = self.visible_ids();
                        eprintln!("visible: {} | {:?}", ids.len(), ids);
                        eprintln!("subtitle: {}", self.w.title.subtitle());
                    }
                    _ => {}
                }
            }
            Input::FillStep(gen) => {
                if self.fill.as_ref().is_some_and(|f| match &f.rows {
                    FillRows::Grid(c) => c.is_for(gen),
                    FillRows::Table(c) => c.is_for(gen),
                }) {
                    self.fill_piece(false);
                    self.schedule_fill();
                }
            }
            Input::SelectAll => {
                if self.view == View::Notes {
                    if let Some(v) = self.notes.borrow().as_ref() {
                        (v.select_all)();
                    }
                    return;
                }
                self.finish_fill();
                self.active_model().select_all();
            }
        }
    }
}

fn id_from_model(model: &gio::ListModel, pos: u32) -> Option<u64> {
    // Items are `BoxedAnyObject`s wrapping either a BookItem or a BookRow.
    let obj = model.item(pos)?.downcast::<glib::BoxedAnyObject>().ok()?;
    if let Ok(item) = obj.try_borrow::<BookItem>() {
        return Some(item.id);
    }
    obj.try_borrow::<BookRow>().ok().map(|r| r.id)
}

/// The dim count at the right end of a sidebar row.
pub fn count_label(n: usize) -> gtk::Label {
    let c = gtk::Label::new(Some(&n.to_string()));
    c.add_css_class("dim-label");
    c.add_css_class("numeric");
    c.set_valign(gtk::Align::Center);
    c
}

/// A sidebar row with an icon, a title and a count.
fn sidebar_row(title: &str, icon: &str) -> (gtk::ListBoxRow, gtk::Label) {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    b.set_margin_top(2);
    b.set_margin_bottom(2);
    b.append(&gtk::Image::from_icon_name(icon));
    let l = gtk::Label::new(Some(title));
    l.set_xalign(0.0);
    l.set_hexpand(true);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    b.append(&l);
    let count = gtk::Label::new(None);
    count.add_css_class("dim-label");
    count.add_css_class("numeric");
    b.append(&count);
    let row = gtk::ListBoxRow::new();
    row.set_child(Some(&b));
    super::audit::label(&row, title);
    (row, count)
}

fn sender_toast(sender: &ComponentSender<LibraryView>, msg: String) {
    sender.input(Input::Toast(msg));
}

/// Whether a point of a book list lies on a book, a column header or a scroll bar (the css nodes
/// of the items of a grid view and a column view).
fn hits_item(scroll: &gtk::ScrolledWindow, x: f64, y: f64) -> bool {
    let mut w = scroll.pick(x, y, gtk::PickFlags::DEFAULT);
    while let Some(cur) = w {
        if cur == *scroll.upcast_ref::<gtk::Widget>() {
            return false;
        }
        if matches!(
            cur.css_name().as_str(),
            "child" | "row" | "cell" | "header" | "scrollbar" | "button"
        ) {
            return true;
        }
        w = cur.parent();
    }
    false
}

/// A click on the empty space of the book list (grid or table) clears the selection, as in Files.
/// Ctrl or Shift held, a drag (the rubber band) or a click on a book do nothing here.
fn attach_empty_click(scroll: &gtk::ScrolledWindow, tx: relm4::Sender<Input>) {
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    let start = Rc::new(Cell::new((0.0, 0.0)));
    {
        let start = start.clone();
        click.connect_pressed(move |_, _, x, y| start.set((x, y)));
    }
    let sc = scroll.clone();
    click.connect_released(move |g, _, x, y| {
        let (sx, sy) = start.get();
        if (x - sx).abs() > 4.0 || (y - sy).abs() > 4.0 {
            return;
        }
        let mods = g.current_event_state();
        if mods.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK) {
            return;
        }
        if !hits_item(&sc, x, y) {
            tx.emit(Input::ClearSelection);
        }
    });
    scroll.add_controller(click);
}

/// The icon names used by a widget and everything under it (also what sits in menu button popovers).
fn collect_icons(w: &gtk::Widget, out: &mut Vec<String>) {
    fn add(out: &mut Vec<String>, n: Option<glib::GString>) {
        if let Some(n) = n.filter(|n| !n.is_empty()) {
            let n = n.to_string();
            if !out.contains(&n) {
                out.push(n);
            }
        }
    }
    if let Some(i) = w.downcast_ref::<gtk::Image>() {
        add(out, i.icon_name());
    }
    if let Some(b) = w.downcast_ref::<gtk::Button>() {
        add(out, b.icon_name());
    }
    if let Some(b) = w.downcast_ref::<gtk::MenuButton>() {
        add(out, b.icon_name());
        if let Some(c) = b.popover().and_then(|p| p.child()) {
            collect_icons(&c, out);
        }
    }
    if let Some(b) = w.downcast_ref::<adw::ButtonContent>() {
        add(out, Some(b.icon_name()));
    }
    let mut c = w.first_child();
    while let Some(ch) = c {
        collect_icons(&ch, out);
        c = ch.next_sibling();
    }
}

/// How many star badges of the cover grid are showing.
fn count_badges(w: &gtk::Widget, n: &mut usize) {
    if w.has_css_class("fav-badge") && w.is_visible() {
        *n += 1;
    }
    let mut c = w.first_child();
    while let Some(ch) = c {
        count_badges(&ch, n);
        c = ch.next_sibling();
    }
}

/// One line: the header width, how many visible widgets it has, and which of them leave it
/// (`clipped=[...]`; empty when all is inside). Buttons count as clipped when not fully inside.
fn header_report(header: &adw::HeaderBar) -> String {
    fn walk(w: &gtk::Widget, header: &gtk::Widget, out: &mut Vec<(String, f32, f32)>) {
        let mut c = w.first_child();
        while let Some(child) = c {
            if child.is_visible() && child.is_mapped() {
                let leaf = child.is::<gtk::Button>()
                    || child.is::<gtk::MenuButton>()
                    || child.is::<gtk::Label>()
                    || child.is::<gtk::Image>();
                if leaf {
                    if let Some(b) = child.compute_bounds(header) {
                        let name = child
                            .tooltip_text()
                            .map(|t| t.to_string())
                            .or_else(|| {
                                child
                                    .downcast_ref::<gtk::Label>()
                                    .map(|l| l.label().to_string())
                            })
                            .unwrap_or_else(|| child.type_().name().to_string());
                        out.push((name, b.x(), b.x() + b.width()));
                    }
                }
                if !child.is::<gtk::Button>() && !child.is::<gtk::MenuButton>() {
                    walk(&child, header, out);
                }
            }
            c = child.next_sibling();
        }
    }
    let hw = header.width() as f32;
    let mut all = Vec::new();
    walk(header.upcast_ref(), header.upcast_ref(), &mut all);
    let clipped: Vec<String> = all
        .iter()
        .filter(|(_, l, r)| *l < -0.5 || *r > hw + 0.5)
        .map(|(n, l, r)| format!("{n}@{l:.0}-{r:.0}"))
        .collect();
    let rightmost = all.iter().map(|(_, _, r)| *r).fold(0.0, f32::max);
    format!(
        "header: width={} children={} rightmost={:.0} clipped=[{}]",
        hw as i32,
        all.len(),
        rightmost,
        clipped.join(", ")
    )
}

#[cfg(test)]
mod main_menu_tests {
    use super::{LIBRARY_MENU, MAIN_MENU};

    fn count(m: &[(&str, &str)]) -> usize {
        m.iter().filter(|(l, _)| !l.is_empty()).count()
    }

    #[test]
    fn the_main_menu_is_short_and_ends_with_the_standard_group() {
        assert_eq!(count(&MAIN_MENU), 8);
        assert!(count(&MAIN_MENU) <= 12 && count(&LIBRARY_MENU) <= 12);
        let labels: Vec<&str> = MAIN_MENU
            .iter()
            .map(|(l, _)| *l)
            .filter(|l| !l.is_empty())
            .collect();
        let n = labels.len();
        assert_eq!(
            &labels[n - 4..],
            ["Preferences", "Keyboard Shortcuts", "Help", "About Bookx"]
        );
        assert!(MAIN_MENU.contains(&("Help", "app.help")));
        assert!(!MAIN_MENU.iter().any(|(l, _)| *l == "Copy Diagnostics"));
        // Saved Searches moved to the sidebar (its action stays for the palette)
        assert!(!MAIN_MENU.iter().any(|(l, _)| *l == "Saved Searches"));
    }

    #[test]
    fn every_old_destination_is_still_in_a_menu() {
        for old in [
            "app.stats",
            "app.vocab",
            "lib.trackers",
            "lib.audiobooks",
            "lib.opds",
            "lib.export-calibre",
            "lib.citations",
        ] {
            assert!(LIBRARY_MENU.iter().any(|(_, a)| *a == old), "{old}");
        }
    }
}

#[cfg(test)]
mod sync_menu_tests {
    use super::MAIN_MENU;

    #[test]
    fn the_menu_has_one_sync_entry_and_no_share_library() {
        assert_eq!(MAIN_MENU.iter().filter(|(l, _)| *l == "Sync").count(), 1);
        assert!(MAIN_MENU.contains(&("Sync", "app.sync")));
        assert!(!MAIN_MENU.iter().any(|(l, _)| *l == "Share Library"));
        // the USB window stays next to it, and several devices are named in the plural
        let i = MAIN_MENU.iter().position(|(l, _)| *l == "Sync").unwrap();
        assert_eq!(MAIN_MENU[i + 1], ("Devices", "app.devices"));
        assert!(!MAIN_MENU.iter().any(|(l, _)| *l == "Device"));
    }
}
