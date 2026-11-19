pub mod annotations;
pub mod audiobooks;
pub mod audit;
pub mod authors;
pub mod backup;
pub mod bulkfetch;
pub mod canvas;
pub mod card_drag;
pub mod citations;
pub mod colors;
pub mod covers;
pub mod devices;
pub mod devices_list;
pub mod dict_service;
pub mod dict_settings;
pub mod duplicates;
pub mod editor;
pub mod formats;
pub mod health;
pub mod health_extra;
pub mod image_viewer;
pub mod jobs;
pub mod kosync;
pub mod language_settings;
pub mod library;
pub mod library_banner;
pub mod library_switcher;
pub mod map_thumbs;
pub mod mcp_settings;
pub mod meta_edit;
pub mod metadata;
pub mod metadata_settings;
pub mod news;
pub mod note_row;
pub mod notes;
pub mod opds;
pub mod organize;
pub mod overlay_guard;
pub mod page_area;
pub mod palette;
pub mod preferences;
pub mod preflight;
pub mod privacy_settings;
pub mod progress_ring;
pub mod reader;
pub mod reader_extra;
pub mod reader_footer;
pub mod reader_goto;
pub mod reader_map;
pub mod reader_pdf;
pub mod reader_side;
pub mod reader_style;
pub mod reader_theme_editor;
pub mod reader_theme_row;
pub mod reader_tts;
pub mod reading_ruler;
pub mod rsvp;
pub mod saved;
pub mod sel_handles;
pub mod share;
pub mod shelves;
pub mod shortcuts_dialog;
pub mod stats;
pub mod syntax;
pub mod trackers;
pub mod tts_player;
pub mod tts_settings;
pub mod vocab;
pub mod web;

use crate::i18n::tr;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use relm4::gtk::{self, prelude::*};

use crate::config::Config;
use crate::core::library::Library;
use crate::reading_stats::StatsHub;

/// State shared by every window. GTK is single threaded so `Rc<RefCell<_>>` is enough.
pub struct Shared {
    pub lib: RefCell<Library>,
    pub cfg: RefCell<Config>,
    pub stats: RefCell<StatsHub>,
    pub sharing: RefCell<crate::sharing::Sharing>,
    pub vocab: RefCell<crate::vocabulary::VocabHub>,
    pub plugins: RefCell<bookx_plugin::PluginManager>,
    /// Index of book contents, `None` when it could not be opened (search still works without it).
    pub fulltext: RefCell<Option<crate::fulltext::FullText>>,
    /// The catalogue store, the library lock and the notes about the library folder.
    pub guard: RefCell<crate::library_health::LibraryGuard>,
    /// Seconds added to the clock; only the UI tests move it (`stats:skew:N`).
    pub clock_skew: Cell<i64>,
}

impl Shared {
    /// Indexes new or changed books on a worker thread.
    pub fn start_indexing(&self) {
        let Some(ft) = self.fulltext.borrow().clone() else {
            return;
        };
        let (root, books) = {
            let lib = self.lib.borrow();
            (lib.root.clone(), lib.books.clone())
        };
        std::thread::spawn(move || {
            let _ = ft.index_pending(&root, &books);
        });
    }

    /// The current time as the statistics see it.
    pub fn now(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now() + chrono::Duration::seconds(self.clock_skew.get())
    }
}

pub type SharedRef = Rc<Shared>;

pub fn window_of(w: &impl IsA<gtk::Widget>) -> Option<gtk::Window> {
    w.root().and_downcast::<gtk::Window>()
}

pub fn system_is_dark() -> bool {
    relm4::adw::StyleManager::default().is_dark()
}

pub fn open_uri(uri: &str) {
    let _ = gtk::gio::AppInfo::launch_default_for_uri(uri, gtk::gio::AppLaunchContext::NONE);
}

pub fn alert(
    parent: &impl IsA<gtk::Widget>,
    heading: &str,
    body: &str,
    confirm_label: &str,
    destructive: bool,
    on_confirm: impl Fn() + 'static,
) {
    use relm4::adw::{self, prelude::*};
    let d = adw::AlertDialog::new(Some(heading), Some(body));
    d.add_responses(&[("cancel", &tr("Cancel")), ("ok", confirm_label)]);
    d.set_response_appearance(
        "ok",
        if destructive {
            adw::ResponseAppearance::Destructive
        } else {
            adw::ResponseAppearance::Suggested
        },
    );
    d.set_default_response(Some("cancel"));
    d.set_close_response("cancel");
    d.connect_response(None, move |_, resp| {
        if resp == "ok" {
            on_confirm();
        }
    });
    d.present(Some(parent));
}

/// A small "?" button that explains something in a popover.
pub fn help_button(text: &str) -> gtk::MenuButton {
    let label = gtk::Label::new(Some(text));
    label.set_wrap(true);
    label.set_max_width_chars(48);
    label.set_xalign(0.0);
    label.set_margin_top(10);
    label.set_margin_bottom(10);
    label.set_margin_start(12);
    label.set_margin_end(12);
    let pop = gtk::Popover::new();
    pop.set_child(Some(&label));
    let b = gtk::MenuButton::new();
    b.set_icon_name("help-about-symbolic");
    b.set_tooltip_text(Some(&tr("What is this?")));
    b.set_valign(gtk::Align::Center);
    b.add_css_class("flat");
    b.set_popover(Some(&pop));
    b
}
