//! Application window: navigation between library, reader and the editor windows.

use crate::i18n::trf;
use crate::i18n::{tr, N_};
use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio, glib};
use relm4::prelude::*;

use crate::config::{Config, Scheme, APP_ID, APP_NAME, VERSION};
use crate::ui::preferences::{self, PrefChange};
use crate::ui::{editor, library, notes, reader, reader_comic, reader_pdf, Shared, SharedRef};

pub struct App {
    shared: SharedRef,
    window: adw::ApplicationWindow,
    nav: adw::NavigationView,
    toasts: adw::ToastOverlay,
    library: Controller<library::LibraryView>,
    reader: Option<Controller<reader::Reader>>,
    /// The PDF page view, when the open book is shown from its PDF.
    pdf: Option<Controller<reader_pdf::ReaderPdf>>,
    comic: Option<Controller<reader_comic::ReaderComic>>,
    /// The next `Read` opens the e-book (reflowed text) of a book that also has a PDF.
    force_epub: bool,
    reader_id: Option<u64>,
    loading: Option<u64>,
    was_maximized: Option<bool>,
    pending_read: Option<u64>,
    editors: Rc<RefCell<Vec<(u64, Controller<editor::Editor>)>>>,
    /// The audiobook player, while one is open.
    audio: Option<crate::ui::audiobook_player::Page>,
    /// The page shown while a book opens (it turns into the error page when opening fails).
    loading_page: Option<adw::NavigationPage>,
    /// The Undo of the toast shown last (`app:toast-undo-press` runs it in tests).
    last_undo: Option<UndoFn>,
    /// Dev hook `app:fail-open`: the next book fails to open, to show the error page.
    dev_fail_open: bool,
}

/// What an Undo button of a toast does. Cloneable and `Debug` so it can travel in a message.
#[derive(Clone)]
pub struct UndoFn(pub Rc<dyn Fn()>);

impl UndoFn {
    pub fn new(f: impl Fn() + 'static) -> Self {
        UndoFn(Rc::new(f))
    }
}

impl std::fmt::Debug for UndoFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UndoFn")
    }
}

/// The title and the text of the page that replaces the opening page when a book cannot be opened.
pub fn open_error_texts(pdf: bool, error: &str) -> (String, String) {
    let title = if pdf {
        tr("Could not open the PDF")
    } else {
        tr("Could not open the book")
    };
    (title, error.trim().to_string())
}

/// An EPUB opened on a worker thread.
pub struct LoadedBook(crate::core::book::OpenBook);

impl std::fmt::Debug for LoadedBook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LoadedBook")
    }
}

#[derive(Debug)]
pub enum Msg {
    Read(u64),
    /// Play the audiobook of a book (an audiobook of its own, or the one linked to an e-book).
    PlayAudio(u64),
    AudioPopped,
    CloseReader,
    /// Leave the comic that is open and read this book (a neighbouring volume), maybe at its end.
    OpenVolume(u64, bool),
    BookLoaded(u64, Result<LoadedBook, String>),
    /// A PDF read from disk, for the page view.
    PdfLoaded(u64, Result<Vec<u8>, String>),
    /// Close the PDF view and open the reflowed text of the book.
    ReadReflow(u64),
    LoadingPopped,
    ReloadReader,
    OpenFileDialog,
    OpenedFile(std::path::PathBuf),
    ReaderPopped,
    EditBook(u64),
    /// `bookx --edit FILE [name ...] [--select-text TEXT]`: the book is added to the library if it is not there.
    EditFile(crate::editor_state::EditRequest),
    /// A dev command for the editor of one book (opening named files after the editor appeared).
    EditorDev(u64, String),
    EditorSaved,
    EditorClosed(u64),
    Toast(String),
    /// A message with an Undo button (XC-1): what was removed can be brought back for ten seconds.
    ToastUndo(String, UndoFn),
    LibraryChanged,
    Prefs(PrefChange),
    ShowPrefs,
    /// Preferences on the row with this title (from the command palette).
    ShowPrefsAt(String),
    /// Preferences on the page with this id (`preferences::register_page`), over the front window.
    ShowPrefsPage(String),
    ShowPalette,
    /// A book chosen in the palette: from the reader it closes the reader first.
    PaletteBook(u64),
    ShowShortcuts,
    ShowStats,
    ShowShare,
    ShowSyncPrefs,
    /// The Device window.
    ShowDevices,
    /// Reading progress changed from outside the library view.
    DevicesChanged,
    ShowVocab,
    ShowAbout,
    /// Copy the diagnostics report to the clipboard.
    CopyDiagnostics,
    /// A `bookx://` link (from the desktop, another Bookx process or the command line).
    OpenUri(String),
    Dev(String),
    OpenPaths(Vec<std::path::PathBuf>),
    /// `bookx --doc FILE`: import if the library does not have it, then read it.
    OpenDoc(std::path::PathBuf),
}

fn apply_scheme(s: Scheme) {
    adw::StyleManager::default().set_color_scheme(match s {
        Scheme::System => adw::ColorScheme::Default,
        Scheme::Light => adw::ColorScheme::ForceLight,
        Scheme::Dark => adw::ColorScheme::ForceDark,
    });
}

impl SimpleComponent for App {
    type Init = ();
    type Input = Msg;
    type Output = ();
    type Root = adw::ApplicationWindow;
    type Widgets = ();

    fn init_root() -> Self::Root {
        adw::ApplicationWindow::builder()
            .title(APP_NAME)
            .application(&relm4::main_adw_application())
            .build()
    }

    fn init(_: (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let mut cfg = Config::load();
        let key_moved = crate::tts::migrate_env_key(
            &mut cfg.tts,
            std::env::var("BOOKX_TTS_API_KEY").ok().as_deref(),
        );
        if crate::tts::migrate_engine(&mut cfg.tts) || key_moved {
            cfg.save();
        }
        crate::i18n::set_language(&cfg.language);
        crate::privacy::apply(cfg.offline);
        crate::source_priority::install(&cfg.source_priority);
        crate::ui::language_settings::apply_direction();
        if crate::i18n::current_is_rtl() {
            root.set_direction(gtk::TextDirection::Rtl);
        }
        apply_scheme(cfg.scheme);

        let (lib, guard, migrated) = match crate::library_health::open(&cfg.library_dir) {
            Ok(o) => o,
            Err(e) => {
                tracing::error!("cannot open library {}: {e:#}", cfg.library_dir.display());
                let fallback = std::env::temp_dir().join("bookx-library");
                crate::library_health::open(&fallback).expect("cannot create a library folder")
            }
        };
        let mut lib = lib;
        crate::annotation_ids::migrate_library(&mut lib);
        let repaired = crate::highlight_repair::one_time_pass(&mut lib);
        let fulltext = crate::fulltext::FullText::new(guard.store.clone());
        let shared: SharedRef = Rc::new(Shared {
            lib: RefCell::new(lib),
            cfg: RefCell::new(cfg.clone()),
            stats: RefCell::new(crate::reading_stats::StatsHub::load(
                crate::config::stats_path(),
            )),
            clock_skew: std::cell::Cell::new(0),
            sharing: RefCell::new(crate::sharing::Sharing::default()),
            plugins: RefCell::new({
                let mut m = crate::plugins::new_manager(crate::web_import::fetcher());
                for (id, e) in crate::plugins::reload(&mut m) {
                    tracing::warn!("plugin {id} could not be loaded: {e}");
                }
                m
            }),
            vocab: RefCell::new(crate::vocabulary::VocabHub::load(
                crate::config::vocab_path(),
            )),
            fulltext: RefCell::new(Some(fulltext)),
            guard: RefCell::new(guard),
        });
        shared.start_indexing();
        {
            let tx = sender.input_sender().clone();
            crate::ui::secrets_unlock::install(
                &shared,
                root.upcast_ref::<gtk::Window>(),
                Rc::new(move |m| tx.emit(Msg::Toast(m))),
            );
        }
        crate::annotation_sync_ui::install(&shared);
        if !crate::safe_mode::active() {
            crate::ui::kosync::start_queue(&shared);
            crate::ui::share::restore(&shared);
        } else {
            let tx = sender.input_sender().clone();
            glib::idle_add_local_once(move || tx.emit(Msg::Toast(crate::safe_mode::summary())));
        }
        {
            let root = shared.lib.borrow().root.clone();
            if !crate::safe_mode::active() {
                crate::mcp_host::autostart(&root);
            }
        }
        {
            let tx = sender.input_sender().clone();
            crate::ui::health::on_startup(&shared, Rc::new(move |m| tx.emit(Msg::Toast(m))));
            let tx = sender.input_sender().clone();
            crate::voice_download::set_notifier(Rc::new(move |m| tx.emit(Msg::Toast(m))));
            let tx = sender.input_sender().clone();
            crate::ui::dict_service::set_notifier(Rc::new(move |m| tx.emit(Msg::Toast(m))));
            // the languages of the books that are there are noted; dictionaries that waited for the network are retried
            crate::ui::dict_service::library_changed(&shared);
            crate::ui::dict_service::start_retry_timer(&shared);
            // loaded voice models are kept for ten minutes after their last use, then freed
            relm4::gtk::glib::timeout_add_seconds_local(60, || {
                bookx_tts::service::global().sweep(std::time::Instant::now());
                relm4::gtk::glib::ControlFlow::Continue
            });
            let s2 = shared.clone();
            relm4::main_adw_application()
                .connect_shutdown(move |_| crate::ui::health::on_exit(&s2));
        }
        if let Some(r) = repaired {
            tracing::info!(
                "re-checked highlights: {} moved, {} without a place",
                r.moved,
                r.unanchored
            );
            let tx = sender.input_sender().clone();
            let msg = crate::highlight_repair::message(&r);
            glib::timeout_add_local_once(std::time::Duration::from_millis(2400), move || {
                tx.emit(Msg::Toast(msg));
            });
        }
        if let Some(n) = migrated {
            tracing::info!("moved {n} books from bookx-library.json into the database");
            let tx = sender.input_sender().clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
                tx.emit(Msg::Toast(trf("Your library ({} books) now lives in a database; the old catalogue was kept as bookx-library.json.migrated", &[&n])));
            });
        }

        let (mut win_w, mut win_h) = (cfg.window_width, cfg.window_height);
        if let Some((w, h)) = std::env::var("BOOKX_SIZE").ok().and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        }) {
            (win_w, win_h) = (w, h);
        }
        root.set_default_size(win_w, win_h);
        if cfg.maximized {
            root.maximize();
        }
        if crate::config::APP_ID.ends_with("Devel") {
            root.add_css_class("devel");
        }

        let library = library::LibraryView::builder()
            .launch(shared.clone())
            .forward(sender.input_sender(), |out| match out {
                library::Output::Read(id) => Msg::Read(id),
                library::Output::PlayAudio(id) => Msg::PlayAudio(id),
                library::Output::EditBook(id) => Msg::EditBook(id),
                library::Output::Toast(t) => Msg::Toast(t),
                library::Output::LibraryChanged => Msg::LibraryChanged,
                library::Output::SwitchLibrary(p) => Msg::Prefs(PrefChange::LibraryDir(p)),
            });

        let nav = adw::NavigationView::new();
        nav.add(library.widget());
        {
            let tx = sender.input_sender().clone();
            nav.connect_popped(move |_, page| match page.tag().as_deref() {
                Some("reader") => tx.emit(Msg::ReaderPopped),
                Some("loading") => tx.emit(Msg::LoadingPopped),
                Some(t) if t.starts_with("audiobook") => tx.emit(Msg::AudioPopped),
                _ => {}
            });
        }
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&nav));
        root.set_content(Some(&toasts));

        // Collapse the sidebars of the library on narrow windows.
        let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            900.0,
            adw::LengthUnit::Sp,
        ));
        root.add_breakpoint(bp);

        // app-level actions
        let app = relm4::main_adw_application();
        let add_action = |name: &str, msg: Rc<dyn Fn()>| {
            let a = gio::SimpleAction::new(name, None);
            a.connect_activate(move |_, _| msg());
            app.add_action(&a);
        };
        {
            let tx = sender.input_sender().clone();
            add_action("preferences", Rc::new(move || tx.emit(Msg::ShowPrefs)));
            {
                // `app.preferences-page` takes the id of a page ("editor"): the editor opens the
                // Editor page of Preferences instead of having a preferences window of its own
                let tx = sender.input_sender().clone();
                let a = gio::SimpleAction::new("preferences-page", Some(glib::VariantTy::STRING));
                a.connect_activate(move |_, p| {
                    if let Some(id) = p.and_then(|v| v.str()) {
                        tx.emit(Msg::ShowPrefsPage(id.to_string()));
                    }
                });
                app.add_action(&a);
            }
            let tx = sender.input_sender().clone();
            add_action("shortcuts", Rc::new(move || tx.emit(Msg::ShowShortcuts)));
            let tx = sender.input_sender().clone();
            add_action(
                "help",
                Rc::new(move || tx.emit(Msg::OpenUri("https://bookx.adhadse.com".into()))),
            );
            let tx = sender.input_sender().clone();
            add_action("palette", Rc::new(move || tx.emit(Msg::ShowPalette)));
            let tx = sender.input_sender().clone();
            add_action("stats", Rc::new(move || tx.emit(Msg::ShowStats)));
            let tx = sender.input_sender().clone();
            add_action("share", Rc::new(move || tx.emit(Msg::ShowShare)));
            let tx = sender.input_sender().clone();
            add_action("sync", Rc::new(move || tx.emit(Msg::ShowShare)));
            let tx = sender.input_sender().clone();
            add_action("sync-prefs", Rc::new(move || tx.emit(Msg::ShowSyncPrefs)));
            let tx = sender.input_sender().clone();
            add_action("devices", Rc::new(move || tx.emit(Msg::ShowDevices)));
            let tx = sender.input_sender().clone();
            add_action("vocab", Rc::new(move || tx.emit(Msg::ShowVocab)));
            let tx = sender.input_sender().clone();
            add_action("about", Rc::new(move || tx.emit(Msg::ShowAbout)));
            let tx = sender.input_sender().clone();
            add_action(
                "copy-diagnostics",
                Rc::new(move || tx.emit(Msg::CopyDiagnostics)),
            );
            let tx = sender.input_sender().clone();
            let open_uri = gio::SimpleAction::new("open-uri", Some(glib::VariantTy::STRING));
            open_uri.connect_activate(move |_, p| {
                if let Some(uri) = p.and_then(|p| p.str()) {
                    tx.emit(Msg::OpenUri(uri.to_string()));
                }
            });
            app.add_action(&open_uri);
            // "Open With" while Bookx runs: the second process sends the (absolute) paths here
            let tx = sender.input_sender().clone();
            let open_files =
                gio::SimpleAction::new("open-files", Some(glib::VariantTy::STRING_ARRAY));
            open_files.connect_activate(move |_, p| {
                if let Some(list) = p.and_then(|p| p.get::<Vec<String>>()) {
                    tx.emit(Msg::OpenPaths(
                        list.into_iter().map(std::path::PathBuf::from).collect(),
                    ));
                }
            });
            app.add_action(&open_files);
            let tx = sender.input_sender().clone();
            let open_doc = gio::SimpleAction::new("open-doc", Some(glib::VariantTy::STRING));
            open_doc.connect_activate(move |_, p| {
                if let Some(path) = p.and_then(|p| p.str()) {
                    tx.emit(Msg::OpenDoc(std::path::PathBuf::from(path)));
                }
            });
            app.add_action(&open_doc);
            let win = root.clone();
            add_action("quit", Rc::new(move || win.close()));
        }
        crate::ui::shortcuts_dialog::init(&app);
        crate::ui::library_switcher::note_current(&shared.lib.borrow().root);

        // persist window geometry
        {
            let shared = shared.clone();
            root.connect_close_request(move |w| {
                let mut c = shared.cfg.borrow_mut();
                c.maximized = w.is_maximized();
                if !w.is_maximized() {
                    c.window_width = w.width();
                    c.window_height = w.height();
                }
                c.save();
                let _ = shared.lib.borrow().save();
                glib::Propagation::Proceed
            });
        }

        // `bookx --edit file.epub [name …] [--select-text TEXT]` opens the book in the editor.
        let edit_request =
            crate::editor_state::parse_edit_args(&std::env::args().skip(1).collect::<Vec<_>>());
        if let Some(req) = edit_request.clone() {
            let tx = sender.input_sender().clone();
            glib::idle_add_local_once(move || tx.emit(Msg::EditFile(req)));
        }
        // `bookx bookx://open?id=3` (the scheme handler), `bookx file.epub …` (and "Open With" from the file
        // manager) and `bookx --doc file.epub` (import if needed, then read).
        let launch = crate::launch::parse(&std::env::args().skip(1).collect::<Vec<_>>());
        for uri in launch.links {
            let tx = sender.input_sender().clone();
            glib::idle_add_local_once(move || tx.emit(Msg::OpenUri(uri)));
        }
        if edit_request.is_none() {
            if !launch.files.is_empty() {
                let tx = sender.input_sender().clone();
                let files = launch.files;
                glib::idle_add_local_once(move || tx.emit(Msg::OpenPaths(files)));
            }
            for doc in launch.docs {
                let tx = sender.input_sender().clone();
                glib::idle_add_local_once(move || tx.emit(Msg::OpenDoc(doc)));
            }
        }

        {
            let tx = sender.input_sender().clone();
            let cmds = crate::devtools::script();
            if !cmds.is_empty() {
                glib::idle_add_local_once(move || {
                    crate::devtools::run_script(cmds, move |c| tx.emit(Msg::Dev(c.to_string())));
                });
            }
        }

        let model = App {
            shared,
            window: root,
            nav,
            toasts,
            library,
            reader: None,
            pdf: None,
            comic: None,
            force_epub: false,
            reader_id: None,
            loading: None,
            was_maximized: None,
            pending_read: None,
            editors: Rc::new(RefCell::new(Vec::new())),
            audio: None,
            loading_page: None,
            last_undo: None,
            dev_fail_open: false,
        };
        model.apply_touch_ui();
        model.apply_eink();
        ComponentParts { model, widgets: () }
    }

    fn update(&mut self, msg: Msg, sender: ComponentSender<Self>) {
        match msg {
            Msg::Read(id) => {
                // an audiobook of its own has no pages: it opens the player
                if self
                    .shared
                    .lib
                    .borrow()
                    .get(id)
                    .is_some_and(crate::audiobook::is_own)
                {
                    sender.input(Msg::PlayAudio(id));
                    return;
                }
                if self.reader.is_some()
                    || self.pdf.is_some()
                    || self.comic.is_some()
                    || self.loading.is_some()
                {
                    return;
                }
                let as_text = std::mem::take(&mut self.force_epub);
                let pdf_file = if as_text || !crate::pdf::PdfPrefs::load().pages {
                    None
                } else {
                    let lib = self.shared.lib.borrow();
                    lib.get(id).and_then(|b| {
                        b.formats
                            .iter()
                            .find(|f| f.format == "pdf")
                            .map(|f| lib.format_path(f))
                            .filter(|p| p.exists())
                    })
                };
                let path = {
                    let lib = self.shared.lib.borrow();
                    match lib.get(id) {
                        Some(b) if !b.drm.is_empty() => {
                            let msg = crate::book_formats::protected_message(&b.drm);
                            drop(lib);
                            sender.input(Msg::Toast(msg));
                            return;
                        }
                        Some(b) => lib.path_for(b, crate::core::formats::Target::Reader),
                        None => return,
                    }
                };
                self.loading = Some(id);
                // The reader opens maximised; the library size is restored when you come back.
                self.was_maximized = Some(self.window.is_maximized());
                self.window.maximize();

                let page = adw::NavigationPage::builder()
                    .title(tr("Opening"))
                    .tag("loading")
                    .build();
                let status = adw::StatusPage::builder().title(tr("Opening book")).build();
                let spinner = adw::Spinner::new();
                spinner.set_size_request(48, 48);
                status.set_child(Some(&spinner));
                let tb = adw::ToolbarView::new();
                let hb = adw::HeaderBar::new();
                hb.add_css_class("flat");
                tb.add_top_bar(&hb);
                tb.set_content(Some(&status));
                page.set_child(Some(&tb));
                self.nav.push(&page);
                self.loading_page = Some(page);

                if let Some(pdf_path) = pdf_file {
                    let (tx_done, rx) = std::sync::mpsc::channel();
                    std::thread::spawn(move || {
                        let res = std::fs::read(&pdf_path)
                            .map_err(|e| format!("{}: {e}", pdf_path.display()));
                        let _ = tx_done.send(res);
                    });
                    let tx = sender.input_sender().clone();
                    glib::timeout_add_local(std::time::Duration::from_millis(30), move || match rx
                        .try_recv()
                    {
                        Ok(res) => {
                            tx.emit(Msg::PdfLoaded(id, res));
                            glib::ControlFlow::Break
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(_) => glib::ControlFlow::Break,
                    });
                    return;
                }

                let fail = std::mem::take(&mut self.dev_fail_open);
                let (tx_done, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let res = if fail {
                        Err("The file is damaged (test)".to_string())
                    } else {
                        crate::core::book::OpenBook::open(&path)
                            .map(|mut b| {
                                crate::plugins::patch_on_open(&mut b.archive);
                                LoadedBook(b)
                            })
                            .map_err(|e| e.to_string())
                    };
                    let _ = tx_done.send(res);
                });
                let tx = sender.input_sender().clone();
                glib::timeout_add_local(std::time::Duration::from_millis(30), move || {
                    match rx.try_recv() {
                        Ok(res) => {
                            tx.emit(Msg::BookLoaded(id, res));
                            glib::ControlFlow::Break
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(_) => glib::ControlFlow::Break,
                    }
                });
            }
            Msg::BookLoaded(id, res) => {
                if self.loading != Some(id) {
                    return;
                }
                self.loading = None;
                match res {
                    Ok(book) => {
                        let book = match crate::comic::open_comic(book.0) {
                            Ok(comic) => {
                                let ctl = reader_comic::ReaderComic::builder()
                                    .launch((self.shared.clone(), id, comic))
                                    .forward(sender.input_sender(), |out| match out {
                                        reader_comic::Output::Close => Msg::CloseReader,
                                        reader_comic::Output::Toast(t) => Msg::Toast(t),
                                        reader_comic::Output::OpenVolume { id, last_page } => {
                                            Msg::OpenVolume(id, last_page)
                                        }
                                    });
                                self.nav.replace(&[
                                    self.library.widget().clone(),
                                    ctl.widget().clone(),
                                ]);
                                self.comic = Some(ctl);
                                self.reader_id = Some(id);
                                return;
                            }
                            Err(b) => LoadedBook(*b),
                        };
                        let from_pdf = book
                            .0
                            .archive
                            .get_string("OEBPS/content.opf")
                            .is_some_and(|o| o.contains("bookx:reflowed-from"));
                        if from_pdf {
                            sender.input(Msg::Toast(
                                tr("Reflowed text of a PDF (experimental): columns, tables and formulas may come out wrong"),
                            ));
                        }
                        let ctl = reader::Reader::builder()
                            .launch((self.shared.clone(), id, book.0))
                            .forward(sender.input_sender(), |out| match out {
                                reader::Output::Close => Msg::CloseReader,
                                reader::Output::Toast(t) => Msg::Toast(t),
                                reader::Output::ToastUndo(t, u) => Msg::ToastUndo(t, u),
                                reader::Output::Reload => Msg::ReloadReader,
                                reader::Output::OpenFile => Msg::OpenFileDialog,
                                reader::Output::OpenPreferences(t) => Msg::ShowPrefsAt(t),
                            });
                        self.nav
                            .replace(&[self.library.widget().clone(), ctl.widget().clone()]);
                        if let Some(h) = notes::take_jump(id) {
                            ctl.emit(reader::Input::GoHighlight(h));
                        }
                        self.reader = Some(ctl);
                        self.reader_id = Some(id);
                    }
                    Err(e) => {
                        // a highlight asked for in a book that did not open must not jump the next time
                        let _ = notes::take_jump(id);
                        self.restore_window();
                        self.show_open_error(false, &e);
                    }
                }
            }
            Msg::PdfLoaded(id, res) => {
                if self.loading != Some(id) {
                    return;
                }
                self.loading = None;
                match res {
                    Ok(bytes) => {
                        let ctl = reader_pdf::ReaderPdf::builder()
                            .launch((self.shared.clone(), id, bytes))
                            .forward(sender.input_sender(), move |out| match out {
                                reader_pdf::Output::Close => Msg::CloseReader,
                                reader_pdf::Output::Toast(t) => Msg::Toast(t),
                                reader_pdf::Output::ToastUndo(t, u) => Msg::ToastUndo(t, u),
                                reader_pdf::Output::Reflow => Msg::ReadReflow(id),
                            });
                        self.nav
                            .replace(&[self.library.widget().clone(), ctl.widget().clone()]);
                        if let Some(h) = notes::take_jump(id) {
                            ctl.emit(reader_pdf::Input::GoHighlight(h));
                        }
                        self.pdf = Some(ctl);
                        self.reader_id = Some(id);
                    }
                    Err(e) => {
                        self.restore_window();
                        self.show_open_error(true, &e);
                    }
                }
            }
            Msg::ReadReflow(id) => {
                self.force_epub = true;
                self.pending_read = Some(id);
                self.nav.pop();
            }
            Msg::LoadingPopped => {
                if self.loading.take().is_some() {
                    self.restore_window();
                }
            }
            Msg::ReloadReader => {
                if let Some(id) = self.reader_id {
                    self.pending_read = Some(id);
                    self.nav.pop();
                }
            }
            Msg::OpenFileDialog => {
                let dlg = gtk::FileDialog::builder().title(tr("Open Book")).build();
                let filter = gtk::FileFilter::new();
                filter.set_name(Some("E-books"));
                for ext in crate::import::extensions() {
                    filter.add_suffix(&ext);
                }
                let filters = gio::ListStore::new::<gtk::FileFilter>();
                filters.append(&filter);
                dlg.set_filters(Some(&filters));
                let tx = sender.input_sender().clone();
                dlg.open(Some(&self.window), gio::Cancellable::NONE, move |res| {
                    if let Some(p) = res.ok().and_then(|f| f.path()) {
                        tx.emit(Msg::OpenedFile(p));
                    }
                });
            }
            Msg::OpenedFile(path) => {
                use crate::core::library::AddOutcome;
                let res = self.shared.lib.borrow_mut().add_path(&path, false);
                match res {
                    Ok(AddOutcome::Added(id)) | Ok(AddOutcome::Duplicate(id)) => {
                        self.library.emit(library::Input::Refresh);
                        self.pending_read = Some(id);
                        if self.reader.is_some() || self.pdf.is_some() || self.comic.is_some() {
                            self.nav.pop();
                        } else {
                            sender.input(Msg::Read(id));
                            self.pending_read = None;
                        }
                    }
                    Err(e) => sender.input(Msg::Toast(trf("Cannot open that file: {}", &[&e]))),
                }
            }
            Msg::CloseReader => {
                self.nav.pop();
            }
            Msg::OpenVolume(id, last_page) => {
                if last_page {
                    crate::comic::start_at_last_page(id);
                }
                self.pending_read = Some(id);
                self.nav.pop();
            }
            Msg::PlayAudio(id) => {
                if self.audio.is_some()
                    || self.reader.is_some()
                    || self.pdf.is_some()
                    || self.comic.is_some()
                    || self.loading.is_some()
                {
                    return;
                }
                let tx = sender.input_sender().clone();
                let toast: Rc<dyn Fn(String)> = Rc::new(move |t| tx.emit(Msg::Toast(t)));
                match crate::ui::audiobook_player::open(&self.shared, id, toast) {
                    Ok(page) => {
                        self.nav.push(&page.page);
                        self.audio = Some(page);
                    }
                    Err(e) => sender.input(Msg::Toast(trf(
                        "Could not play the audiobook: {}",
                        &[&format!("{e:#}")],
                    ))),
                }
            }
            Msg::AudioPopped => {
                if let Some(p) = self.audio.take() {
                    p.close();
                }
                let _ = self.shared.lib.borrow().save();
                // the library may already be gone when the window closes while the reader is open
                let _ = self.library.sender().send(library::Input::Requery);
            }
            Msg::ReaderPopped => {
                self.reader = None;
                self.pdf = None;
                self.comic = None;
                self.reader_id = None;
                self.restore_window();
                if let Some(id) = self.pending_read.take() {
                    sender.input(Msg::Read(id));
                }
                let _ = self.shared.lib.borrow().save();
                self.window.set_fullscreened(false);
                // the library may already be gone when the window closes while the reader is open
                let _ = self.library.sender().send(library::Input::Requery);
            }
            Msg::EditBook(id) => {
                if let Some((_, c)) = self.editors.borrow().iter().find(|(i, _)| *i == id) {
                    c.widget().present();
                    return;
                }
                let ctl = editor::Editor::builder()
                    .launch((self.shared.clone(), id))
                    .forward(sender.input_sender(), move |out| match out {
                        editor::Output::Saved => Msg::EditorSaved,
                        editor::Output::Closed => Msg::EditorClosed(id),
                        editor::Output::Toast(t) => Msg::Toast(t),
                    });
                ctl.widget().set_transient_for(Some(&self.window));
                ctl.widget().present();
                self.editors.borrow_mut().push((id, ctl));
            }
            Msg::EditFile(req) => {
                let added = self.shared.lib.borrow_mut().add_path(&req.file, false);
                match added {
                    Ok(out) => {
                        let id = match out {
                            crate::core::library::AddOutcome::Added(i)
                            | crate::core::library::AddOutcome::Duplicate(i) => i,
                        };
                        let _ = self.shared.lib.borrow().save();
                        self.library.emit(library::Input::Refresh);
                        sender.input(Msg::EditBook(id));
                        if !req.names.is_empty() || req.select.is_some() {
                            let cmd = format!(
                                "x:open-names={}|{}",
                                req.names.join(","),
                                req.select.clone().unwrap_or_default()
                            );
                            let tx = sender.input_sender().clone();
                            glib::timeout_add_local_once(
                                std::time::Duration::from_millis(600),
                                move || tx.emit(Msg::EditorDev(id, cmd)),
                            );
                        }
                    }
                    Err(e) => sender.input(Msg::Toast(trf(
                        "Cannot open {} for editing: {}",
                        &[&req.file.display(), &e],
                    ))),
                }
            }
            Msg::EditorDev(id, cmd) => {
                if let Some((_, c)) = self.editors.borrow().iter().find(|(i, _)| *i == id) {
                    c.emit(editor::Input::Dev(cmd));
                }
            }
            Msg::EditorSaved => {
                crate::ui::covers::clear();
                self.library.emit(library::Input::Refresh);
            }
            Msg::EditorClosed(id) => {
                let editors = self.editors.clone();
                glib::idle_add_local_once(move || editors.borrow_mut().retain(|(i, _)| *i != id));
            }
            Msg::Toast(t) => {
                let toast = adw::Toast::new(&t);
                toast.set_use_markup(false);
                self.toasts.add_toast(toast);
            }
            Msg::ToastUndo(text, undo) => {
                self.last_undo = Some(undo.clone());
                crate::ui::dialog_toast::show_undo(&self.toasts, &text, move || (undo.0)());
            }
            Msg::LibraryChanged => {
                self.shared.start_indexing();
                crate::ui::dict_service::library_changed(&self.shared);
            }
            Msg::OpenPaths(paths) => {
                let mut files = Vec::new();
                for p in paths {
                    if p.is_dir() {
                        crate::import::collect(&p, true, &mut files);
                    } else {
                        files.push(p);
                    }
                }
                self.library.emit(library::Input::ImportPaths(files));
            }
            Msg::OpenDoc(path) => {
                let outcome = if crate::core::library::is_supported_file(&path) {
                    self.shared.lib.borrow_mut().add_path(&path, false)
                } else {
                    Err(anyhow::anyhow!("not an e-book Bookx can open directly"))
                };
                match outcome {
                    Ok(crate::core::library::AddOutcome::Added(id))
                    | Ok(crate::core::library::AddOutcome::Duplicate(id)) => {
                        self.library.emit(library::Input::Refresh);
                        self.window.present();
                        sender.input(Msg::Read(id));
                    }
                    // other formats (and anything that failed) go through the normal import
                    Err(_) => sender.input(Msg::OpenPaths(vec![path])),
                }
            }
            Msg::ShowPrefs => {
                let tx = sender.input_sender().clone();
                preferences::show(
                    &self.front_window(),
                    &self.shared,
                    Rc::new(move |c| tx.emit(Msg::Prefs(c))),
                );
            }
            Msg::ShowPrefsPage(id) => {
                let tx = sender.input_sender().clone();
                preferences::show_at(
                    &self.front_window(),
                    &self.shared,
                    Rc::new(move |c| tx.emit(Msg::Prefs(c))),
                    None,
                );
                preferences::goto_page(&id);
            }
            Msg::ShowPrefsAt(title) => {
                let tx = sender.input_sender().clone();
                preferences::show_at(
                    &self.window,
                    &self.shared,
                    Rc::new(move |c| tx.emit(Msg::Prefs(c))),
                    Some(&title),
                );
            }
            Msg::ShowPalette => {
                // in a book editor window the palette is the editor's own
                let front = self.front_window();
                if front != *self.window.upcast_ref::<gtk::Window>() {
                    let _ = front.activate_action("ext.win", Some(&"palette".to_variant()));
                } else {
                    self.show_palette(&sender)
                }
            }
            Msg::PaletteBook(id) => {
                if self.reader.is_some() || self.pdf.is_some() || self.comic.is_some() {
                    self.pending_read = Some(id);
                    self.nav.pop();
                } else {
                    sender.input(Msg::Read(id));
                }
            }
            Msg::Prefs(c) => match c {
                PrefChange::Scheme(s) => {
                    self.shared.cfg.borrow_mut().scheme = s;
                    self.shared.cfg.borrow().save();
                    apply_scheme(s);
                }
                PrefChange::Banner => self.library.emit(library::Input::BannerLayout),
                PrefChange::ReaderDefaults => {
                    self.apply_touch_ui();
                    self.apply_eink();
                }
                PrefChange::LibraryDir(p) if p == self.shared.lib.borrow().root => {}
                PrefChange::LibraryDir(p) => match crate::library_health::open(&p) {
                    Ok((lib, guard, _)) => {
                        // leave the old library in order: a last snapshot, then its lock goes
                        crate::ui::health::on_exit(&self.shared);
                        *self.shared.fulltext.borrow_mut() =
                            Some(crate::fulltext::FullText::new(guard.store.clone()));
                        *self.shared.lib.borrow_mut() = lib;
                        *self.shared.guard.borrow_mut() = guard;
                        self.shared.start_indexing();
                        self.shared.cfg.borrow_mut().library_dir = p.clone();
                        self.shared.cfg.borrow().save();
                        crate::ui::library_switcher::note_current(&p);
                        crate::ui::covers::clear();
                        self.library.emit(library::Input::Refresh);
                        {
                            let tx = sender.input_sender().clone();
                            crate::ui::health::on_startup(
                                &self.shared,
                                Rc::new(move |m| tx.emit(Msg::Toast(m))),
                            );
                        }
                        sender.input(Msg::Toast(trf("Library switched to {}", &[&p.display()])));
                    }
                    Err(e) => sender.input(Msg::Toast(trf("Cannot open that folder: {}", &[&e]))),
                },
                PrefChange::Layout(msg) => {
                    self.library.emit(library::Input::Refresh);
                    sender.input(Msg::Toast(msg));
                }
            },
            Msg::ShowShortcuts => show_shortcuts(&self.front_window()),
            Msg::ShowStats => crate::ui::stats::show(&self.window, &self.shared),
            Msg::ShowShare => crate::ui::sync_window::show(&self.window, &self.shared),
            Msg::ShowSyncPrefs => {
                let tx = sender.input_sender().clone();
                preferences::show_at(
                    &self.window,
                    &self.shared,
                    Rc::new(move |c| tx.emit(Msg::Prefs(c))),
                    None,
                );
                preferences::goto_page(crate::sync_model::PAGE_ID);
            }
            Msg::ShowDevices => {
                let tx = sender.input_sender().clone();
                crate::ui::devices::show(
                    &self.window,
                    &self.shared,
                    std::rc::Rc::new(move || tx.emit(Msg::DevicesChanged)),
                );
            }
            Msg::DevicesChanged => self.library.emit(library::Input::Refresh),
            Msg::ShowVocab => crate::ui::vocab::show(&self.window, &self.shared),
            Msg::Dev(cmd) => {
                let (name, arg) = cmd.split_once(':').unwrap_or((cmd.as_str(), ""));
                match name {
                    "shot" => {
                        let (path, which) = arg
                            .rsplit_once('@')
                            .map(|(p, w)| (p, w.parse().unwrap_or(0)))
                            .unwrap_or((arg, 0));
                        crate::devtools::screenshot(path, which)
                    }
                    // `read:<id>` waits (up to ~80 s) for a book that is still being imported,
                    // so scripted tests do not depend on import timing.
                    "read" => {
                        let (id, tries) = match arg.split_once('/') {
                            Some((i, t)) => (i.parse().unwrap_or(1), t.parse().unwrap_or(0u32)),
                            None => (arg.parse().unwrap_or(1), 0),
                        };
                        let known = self.shared.lib.borrow().get(id).is_some();
                        if known || tries >= 160 {
                            sender.input(Msg::Read(id));
                        } else {
                            let tx = sender.input_sender().clone();
                            glib::timeout_add_local_once(
                                std::time::Duration::from_millis(500),
                                move || tx.emit(Msg::Dev(format!("read:{id}/{}", tries + 1))),
                            );
                        }
                    }
                    "edit" => sender.input(Msg::EditBook(arg.parse().unwrap_or(1))),
                    // `edit-file:FILE|name,name|text`: what `bookx --edit FILE name... --select-text text` does
                    "edit-file" => {
                        let mut p = arg.split('|');
                        let mut args =
                            vec!["--edit".to_string(), p.next().unwrap_or("").to_string()];
                        args.extend(
                            p.next()
                                .unwrap_or("")
                                .split(';')
                                .filter(|n| !n.is_empty())
                                .map(str::to_string),
                        );
                        if let Some(t) = p.next().filter(|t| !t.is_empty()) {
                            args.push("--select-text".into());
                            args.push(t.to_string());
                        }
                        if let Some(req) = crate::editor_state::parse_edit_args(&args) {
                            sender.input(Msg::EditFile(req));
                        }
                    }
                    "close" => sender.input(Msg::CloseReader),
                    // `app:toast-undo-test` shows an undo toast that prints `toast_undo: pressed` when its button
                    // is used; `app:toast-undo-press` presses it; `app:fail-open` makes the next book fail to open;
                    // `app:open-error` prints `open_error: shown=B title="..." button="..."`
                    "app" => match arg {
                        "toast-undo-test" => sender.input(Msg::ToastUndo(
                            tr("Test item removed"),
                            UndoFn::new(|| eprintln!("toast_undo: pressed")),
                        )),
                        "toast-undo-press" => match self.last_undo.take() {
                            Some(u) => (u.0)(),
                            None => eprintln!("toast_undo: none"),
                        },
                        "fail-open" => self.dev_fail_open = true,
                        // `app:win-size=WxH` resizes the window (it leaves the maximised state);
                        // `app:win-dump` prints `window: WxH`
                        a if a.starts_with("win-size=") => {
                            if let Some((w, h)) = a[9..].split_once('x') {
                                self.window.unmaximize();
                                self.window.set_default_size(
                                    w.parse().unwrap_or(900),
                                    h.parse().unwrap_or(600),
                                );
                            }
                        }
                        // `app:text-scale=1.5` scales the text like the accessibility setting (Xft DPI)
                        a if a.starts_with("text-scale=") => {
                            let f: f64 = a[11..].parse().unwrap_or(1.0);
                            if let Some(s) = gtk::Settings::default() {
                                s.set_gtk_xft_dpi((96.0 * f * 1024.0) as i32);
                            }
                        }
                        "win-dump" => eprintln!(
                            "window: {}x{} maximized={}",
                            self.window.width(),
                            self.window.height(),
                            self.window.is_maximized()
                        ),
                        "open-error" => {
                            let found = self.nav.visible_page().and_then(|p| {
                                p.child()
                                    .and_downcast::<adw::ToolbarView>()
                                    .and_then(|t| t.content())
                                    .and_downcast::<adw::StatusPage>()
                            });
                            match found.filter(|s| s.widget_name() == "open-error") {
                                Some(s) => {
                                    let button = s
                                        .child()
                                        .and_downcast::<gtk::Button>()
                                        .and_then(|b| b.label())
                                        .unwrap_or_default();
                                    eprintln!(
                                        "open_error: shown=true title=\"{}\" description=\"{}\" button=\"{}\"",
                                        s.title(),
                                        s.description().unwrap_or_default(),
                                        button
                                    );
                                }
                                None => eprintln!("open_error: shown=false"),
                            }
                        }
                        _ => {}
                    },
                    // `audio:add=PATH` imports an audiobook, `audio:open=ID` plays it, the rest drives the
                    // player (see ui/audiobook_player.rs `dev`): `audio:dump` prints its state
                    "audio" => {
                        let (sub, val) = arg.split_once('=').unwrap_or((arg, ""));
                        match sub {
                            "add" => {
                                let r = crate::audiobook::data_dir().map(|d| {
                                    crate::audiobook::import_now(
                                        &mut self.shared.lib.borrow_mut(),
                                        std::path::Path::new(val),
                                        &d,
                                    )
                                });
                                match r {
                                    Some(Ok(id)) => eprintln!("audio: added id={id}"),
                                    Some(Err(e)) => eprintln!("audio: add failed: {e:#}"),
                                    None => eprintln!("audio: no data folder"),
                                }
                                self.library.emit(library::Input::Refresh);
                            }
                            "open" => sender.input(Msg::PlayAudio(val.parse().unwrap_or(1))),
                            // `audio:link=BOOK:PATH` links audio to an e-book; `audio:continue-listening=BOOK`
                            // and `audio:continue-reading=BOOK` carry the place over (what the details rows do)
                            "link" => {
                                let (id, path) = val.split_once(':').unwrap_or(("0", ""));
                                let id: u64 = id.parse().unwrap_or(0);
                                let mut lib = self.shared.lib.borrow_mut();
                                let res = lib.get(id).map(|b| {
                                    let mut r = b.resources.clone();
                                    r.push(crate::audiobook_link::resource_for(
                                        std::path::Path::new(path),
                                    ));
                                    r
                                });
                                match res {
                                    Some(r) => match lib.set_resources(id, r, "link".into()) {
                                        Ok(()) => eprintln!("audio: linked id={id}"),
                                        Err(e) => eprintln!("audio: link failed: {e:#}"),
                                    },
                                    None => eprintln!("audio: link failed: no book {id}"),
                                }
                            }
                            "continue-listening" => {
                                let id: u64 = val.parse().unwrap_or(0);
                                let r = crate::audiobook::data_dir().map(|d| {
                                    crate::audiobook::continue_in_audiobook(
                                        &self.shared.lib.borrow(),
                                        id,
                                        &d,
                                    )
                                });
                                match r {
                                    Some(Ok(ms)) => eprintln!("audio: continue-listening ms={ms}"),
                                    Some(Err(e)) => {
                                        eprintln!("audio: continue-listening failed: {e:#}")
                                    }
                                    None => eprintln!("audio: no data folder"),
                                }
                            }
                            "continue-reading" => {
                                let id: u64 = val.parse().unwrap_or(0);
                                let r = crate::audiobook::data_dir().map(|d| {
                                    crate::audiobook::continue_reading(
                                        &mut self.shared.lib.borrow_mut(),
                                        id,
                                        &d,
                                    )
                                });
                                match r {
                                    Some(Ok(p)) => eprintln!(
                                        "audio: continue-reading spine={} frac={:.2}",
                                        p.spine, p.frac
                                    ),
                                    Some(Err(e)) => {
                                        eprintln!("audio: continue-reading failed: {e:#}")
                                    }
                                    None => eprintln!("audio: no data folder"),
                                }
                            }
                            "stats" => eprintln!(
                                "{}",
                                crate::ui::audiobook_player::dev_stats(&self.shared)
                            ),
                            "close" => {
                                if self.audio.is_some() {
                                    self.nav.pop();
                                }
                            }
                            _ => match &self.audio {
                                Some(p) => {
                                    let out = p.dev(arg);
                                    if !out.is_empty() {
                                        eprintln!("{out}");
                                    }
                                }
                                None => eprintln!("audio: no player"),
                            },
                        }
                    }
                    // `share:start[:port]`, `share:stop`, `share:dump`: drive sharing without dialogs
                    "share" => {
                        let (sub, rest) = arg.split_once(':').unwrap_or((arg, ""));
                        match sub {
                            "start" => {
                                let mut s = self.shared.cfg.borrow().share.clone();
                                s.port = rest.parse().unwrap_or(0);
                                if let Some(pw) = std::env::var("BOOKX_SHARE_PASSWORD")
                                    .ok()
                                    .filter(|p| !p.is_empty())
                                {
                                    s.password = pw;
                                }
                                match crate::ui::share::apply(&self.shared, &s, true) {
                                    Ok(u) => eprintln!("share_started: {}", u.join(" ")),
                                    Err(e) => eprintln!("share_error: {e}"),
                                }
                            }
                            "lan" => {
                                let mut s = self.shared.cfg.borrow().share.clone();
                                s.port = 0;
                                s.lan = true;
                                s.password = rest.to_string();
                                match crate::ui::share::apply(&self.shared, &s, true) {
                                    Ok(u) => eprintln!("share_started: {}", u.join(" ")),
                                    Err(e) => eprintln!("share_error: {e}"),
                                }
                            }
                            "stop" => {
                                let _ = crate::ui::share::apply(
                                    &self.shared,
                                    &self.shared.cfg.borrow().share.clone(),
                                    false,
                                );
                                eprintln!("share_stopped");
                            }
                            // `share:kosync` serves with the KOReader switch on (kept in the settings,
                            // so the window opens with it), `share:kodump` prints what the window shows
                            "kosync" => {
                                let mut s = self.shared.cfg.borrow().share.clone();
                                s.port = 0;
                                s.kosync = true;
                                self.shared.cfg.borrow_mut().share = s.clone();
                                match crate::ui::share::apply(&self.shared, &s, true) {
                                    Ok(u) => eprintln!("share_started: {}", u.join(" ")),
                                    Err(e) => eprintln!("share_error: {e}"),
                                }
                            }
                            "kodump" => crate::ui::share::dev_dump(&self.shared),
                            // `share:port0` serves on a free port from now on (parallel tests)
                            "port0" => {
                                let mut c = self.shared.cfg.borrow_mut();
                                c.share.port = 0;
                                c.save();
                            }
                            "show" => sender.input(Msg::ShowShare),
                            // `share:https` serves HTTPS with the self-signed certificate,
                            // `share:httpsfiles:<cert>:<key>` with the given files
                            "https" | "httpsfiles" => {
                                let mut s = self.shared.cfg.borrow().share.clone();
                                s.port = 0;
                                s.https = true;
                                if sub == "httpsfiles" {
                                    let (c, k) = rest.split_once(':').unwrap_or((rest, ""));
                                    s.cert_file = c.to_string();
                                    s.key_file = k.to_string();
                                }
                                match crate::ui::share::apply(&self.shared, &s, true) {
                                    Ok(u) => eprintln!("share_started: {}", u.join(" ")),
                                    Err(e) => eprintln!("share_error: {e}"),
                                }
                            }
                            // `share:httpsown` serves HTTPS with a certificate and key made as
                            // files of the user's own (a folder next to the config)
                            "httpsown" => {
                                let data = crate::config::stats_path()
                                    .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                                    .unwrap_or_else(std::env::temp_dir);
                                match bookx_serve::cert::write_self_signed(
                                    &data.join("own-tls"),
                                    &[],
                                ) {
                                    Ok((c, k)) => {
                                        let mut s = self.shared.cfg.borrow().share.clone();
                                        s.port = 0;
                                        s.https = true;
                                        s.cert_file = c.display().to_string();
                                        s.key_file = k.display().to_string();
                                        match crate::ui::share::apply(&self.shared, &s, true) {
                                            Ok(u) => eprintln!("share_started: {}", u.join(" ")),
                                            Err(e) => eprintln!("share_error: {e}"),
                                        }
                                    }
                                    Err(e) => eprintln!("share_error: {e:#}"),
                                }
                            }
                            // `share:gen` makes a new self-signed certificate
                            "gen" => {
                                let s = self.shared.cfg.borrow().share.clone();
                                let data = crate::config::stats_path()
                                    .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                                    .unwrap_or_else(std::env::temp_dir);
                                match crate::sharing::generate_certificate(&s, &data) {
                                    Ok((c, k)) => {
                                        eprintln!("share_cert: {} {}", c.is_file(), k.is_file())
                                    }
                                    Err(e) => eprintln!("share_cert_error: {e:#}"),
                                }
                            }
                            // `share:fetchtls:<path>[:untrusted]` asks over HTTPS, trusting only
                            // the share's own certificate (or nothing)
                            "fetchtls" => {
                                let (path, untrusted) = match rest.strip_suffix(":untrusted") {
                                    Some(p) => (p, true),
                                    None => (rest, false),
                                };
                                let url = self.shared.sharing.borrow().urls().first().cloned();
                                let data = crate::config::stats_path()
                                    .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                                    .unwrap_or_else(std::env::temp_dir);
                                let pem = std::fs::read(
                                    crate::sharing::tls_dir(&data)
                                        .join(bookx_serve::cert::CERT_FILE),
                                )
                                .ok();
                                if let Some(u) = url {
                                    let trust = if untrusted { None } else { pem.as_deref() };
                                    let r = bookx_serve::httpc::get_status(
                                        &format!("{}{}", u.trim_end_matches('/'), path),
                                        trust,
                                    );
                                    match r {
                                        Ok(code) => eprintln!(
                                            "share_fetchtls {path} untrusted={untrusted}: {code}"
                                        ),
                                        Err(_) => eprintln!(
                                            "share_fetchtls {path} untrusted={untrusted}: refused"
                                        ),
                                    }
                                }
                            }
                            "fetch" => {
                                // fetch:<path> from the running server, printing the status line
                                let url = self.shared.sharing.borrow().urls().first().cloned();
                                if let Some(u) = url {
                                    let addr = u
                                        .trim_start_matches("http://")
                                        .trim_end_matches('/')
                                        .to_string();
                                    eprintln!("share_fetch {rest}: {}", http_status(&addr, rest));
                                }
                            }
                            _ => {}
                        }
                    }
                    // `koannot:<command>`: the KOReader highlight and note sync (see `annotation_sync_ui::dev_command`)
                    "koannot" => crate::annotation_sync_ui::dev_command(&self.shared, arg),
                    // `wallabag:<command>`: the Wallabag import form (see `ui::wallabag::dev_command`)
                    "wallabag" => crate::ui::wallabag::dev_command(
                        &self.shared,
                        self.window.upcast_ref(),
                        arg,
                    ),
                    // `kosync:<command>`: the reading-position sync (see `ui::kosync::dev_command`)
                    "kosync" => crate::ui::kosync::dev_command(&self.shared, arg),
                    // `devwin:<command>`: drive the Device window (see `ui::devices::dev_command`)
                    "devwin" => {
                        let tx = sender.input_sender().clone();
                        crate::ui::devices::dev_command(
                            &self.window,
                            &self.shared,
                            std::rc::Rc::new(move || tx.emit(Msg::DevicesChanged)),
                            arg,
                        );
                    }
                    // `plugins:dump`, `plugins:enable=<id>`, `plugins:disable=<id>`, `plugins:show`
                    "plugins" if arg.starts_with("sheet") || arg == "state" => {
                        crate::ui::plugin_trust::dev(&self.shared, arg);
                    }
                    "plugins" => {
                        let mut m = self.shared.plugins.borrow_mut();
                        if let Some(id) = arg.strip_prefix("enable=") {
                            eprintln!(
                                "plugin_enable: {:?}",
                                m.set_enabled(id, true).map_err(|e| format!("{e:#}"))
                            );
                        } else if let Some(id) = arg.strip_prefix("disable=") {
                            eprintln!(
                                "plugin_disable: {:?}",
                                m.set_enabled(id, false).map_err(|e| format!("{e:#}"))
                            );
                        } else if let Some(id) = arg.strip_prefix("auto=") {
                            let path = crate::plugins::auto_transforms_path();
                            eprintln!(
                                "plugin_auto: {:?}",
                                crate::plugins::set_auto_transform(&path, id, true)
                                    .map_err(|e| format!("{e:#}"))
                            );
                        } else if arg == "show" {
                            drop(m);
                            sender.input(Msg::ShowPrefs);
                            return;
                        }
                        crate::plugins::reload(&mut m);
                        let cur = crate::plugins::current();
                        for p in m.list() {
                            eprintln!("plugin: {} enabled={} error={:?}", p.id, p.enabled, p.error);
                        }
                        eprintln!(
                            "plugins_active: sources={} readers={} writers={} transforms={} devices={}",
                            cur.sources.len(),
                            cur.readers.len(),
                            cur.writers.len(),
                            cur.transforms.len(),
                            cur.devices.len()
                        );
                    }
                    // `patches:dump`, `patches:enable=<id>`, `patches:disable=<id>`, `patches:up=<id>`,
                    // `patches:down=<id>`, `patches:preview=<id>` (first library book), `patches:show`
                    "patches" => {
                        let mut m = crate::plugins::patch_manager();
                        if let Some(id) = arg.strip_prefix("enable=") {
                            eprintln!(
                                "patch_enable: {:?}",
                                m.set_enabled(id, true).map_err(|e| format!("{e:#}"))
                            );
                        } else if let Some(id) = arg.strip_prefix("disable=") {
                            eprintln!(
                                "patch_disable: {:?}",
                                m.set_enabled(id, false).map_err(|e| format!("{e:#}"))
                            );
                        } else if let Some(id) = arg.strip_prefix("up=") {
                            let _ = m.move_by(id, -1);
                        } else if let Some(id) = arg.strip_prefix("down=") {
                            let _ = m.move_by(id, 1);
                        } else if let Some(id) = arg.strip_prefix("preview=") {
                            let path = {
                                let lib = self.shared.lib.borrow();
                                lib.books.first().map(|b| lib.book_path(b))
                            };
                            match crate::plugins::preview_patch(&m, id, path.as_deref()) {
                                Ok(r) => eprintln!(
                                    "patch_preview: {id} changes={} files={}",
                                    r.total_changes(),
                                    r.files.len()
                                ),
                                Err(e) => eprintln!("patch_preview: {id} error={e:#}"),
                            }
                        } else if arg == "show" {
                            sender.input(Msg::ShowPrefs);
                            return;
                        }
                        let set = crate::plugins::reload_patches();
                        for p in m.list() {
                            eprintln!(
                                "patch: {} enabled={} hook={:?} error={:?}",
                                p.id,
                                p.enabled,
                                p.hook.map(|h| h.as_str()),
                                p.error
                            );
                        }
                        eprintln!("patches_active: {}", set.len());
                    }
                    // `touch:on|off|auto` sets the large-controls preference
                    "touch" => {
                        use crate::gestures::TouchUi;
                        let t = match arg {
                            "on" => TouchUi::On,
                            "off" => TouchUi::Off,
                            _ => TouchUi::Auto,
                        };
                        self.shared.cfg.borrow_mut().reader.touch_ui = t;
                        self.apply_touch_ui();
                        eprintln!("touch_ui: {}", self.window.has_css_class("touch"));
                    }
                    // `eink:on|off`
                    "eink" => {
                        let on = arg == "on";
                        {
                            let mut c = self.shared.cfg.borrow_mut();
                            c.eink = on;
                            if on {
                                crate::eink::tune_prefs(&mut c.reader);
                            }
                        }
                        self.apply_eink();
                        eprintln!(
                            "eink: {} animations={} window_class={}",
                            on,
                            gtk::Settings::default().is_some_and(|s| s.is_gtk_enable_animations()),
                            self.window.has_css_class("eink")
                        );
                    }
                    "vocab" => {
                        let now = self.shared.now();
                        match arg {
                            "show" => sender.input(Msg::ShowVocab),
                            "dump" => {
                                let hub = self.shared.vocab.borrow();
                                let c = hub.counts(now);
                                eprintln!(
                                    "vocab: total={} due={} learning={} mastered={}",
                                    c.total, c.due, c.learning, c.mastered
                                );
                                let words: Vec<String> = hub
                                    .store
                                    .entries()
                                    .iter()
                                    .map(|e| {
                                        format!(
                                            "{}={}",
                                            e.word,
                                            e.definition.clone().unwrap_or_default()
                                        )
                                    })
                                    .collect();
                                eprintln!("vocab_words: {}", words.join(" | "));
                            }
                            "show-words" => {
                                crate::ui::vocab::show_with(&self.window, &self.shared, true, None)
                            }
                            // your own definitions: `own=term|definition[|book]` saves one without the dialog
                            a if a.starts_with("own=") => {
                                let mut it = a[4..].splitn(3, '|');
                                let (t, d) = (it.next().unwrap_or(""), it.next().unwrap_or(""));
                                let scope = match it.next() {
                                    Some(b) if !b.is_empty() => {
                                        bookx_lookup::vocab::Scope::Book(b.to_string())
                                    }
                                    _ => bookx_lookup::vocab::Scope::All,
                                };
                                let r = self.shared.vocab.borrow_mut().save_own(
                                    &bookx_lookup::vocab::OwnTerm {
                                        term: t.into(),
                                        definition: d.into(),
                                        scope,
                                        ..Default::default()
                                    },
                                    now,
                                );
                                crate::ui::vocab_settings::changed();
                                eprintln!("vocab_own: {:?}", r.ok());
                            }
                            a if a.starts_with("list=") => {
                                eprintln!("{}", crate::ui::vocab_list::dev(&a[5..]));
                            }
                            a if a.starts_with("dialog=") => {
                                eprintln!("{}", crate::ui::vocab_own::dev(&a[7..]));
                            }
                            "own-dump" => {
                                let hub = self.shared.vocab.borrow();
                                let own: Vec<String> = hub
                                    .store
                                    .own_entries()
                                    .iter()
                                    .map(|e| {
                                        format!(
                                            "{}={}@{:?}",
                                            e.word,
                                            e.own.clone().unwrap_or_default(),
                                            e.scope
                                        )
                                    })
                                    .collect();
                                eprintln!("vocab_own_entries: {}", own.join(" | "));
                            }
                            a if a.starts_with("grade:") => {
                                let q: u8 = a[6..].parse().unwrap_or(4);
                                let r = self.shared.vocab.borrow_mut().grade_first_due(q, now);
                                eprintln!("vocab_graded: {:?}", r.ok().flatten());
                            }
                            a if a.starts_with("skew:") => {
                                let n: i64 = a[5..].parse().unwrap_or(0);
                                self.shared.clock_skew.set(self.shared.clock_skew.get() + n);
                            }
                            _ => {}
                        }
                    }
                    "stats" => match arg {
                        "show" => sender.input(Msg::ShowStats),
                        "dump" => self.dump_stats(),
                        // `stats:state` prints `stats: empty=BOOL` (nothing read yet)
                        "state" => {
                            let hub = self.shared.stats.borrow();
                            let sum = crate::reading_stats::summarize(
                                &hub.store,
                                self.shared.now(),
                                &chrono::Local,
                            );
                            eprintln!("stats: empty={}", crate::ui::stats::is_empty(&sum));
                        }
                        a if a.starts_with("goal:") => {
                            crate::ui::stats::dev_goals(&self.shared, &a[5..])
                        }
                        a if a.starts_with("skew:") => {
                            let n: i64 = a[5..].parse().unwrap_or(0);
                            let s = &self.shared;
                            s.clock_skew.set(s.clock_skew.get() + n);
                        }
                        _ => {}
                    },
                    "secrets" => crate::ui::secrets_unlock::dev_command(
                        &self.shared,
                        self.window.upcast_ref::<gtk::Window>(),
                        arg,
                    ),
                    "prefs" if arg.is_empty() => sender.input(Msg::ShowPrefs),
                    "prefs" => preferences::dev(arg),
                    // `dialog:state` prints whether an AdwDialog is on the window, `dialog:close` closes it
                    "dialog" => {
                        if arg == "close" {
                            if let Some(d) = self.window.visible_dialog() {
                                d.close();
                            }
                        }
                        // `dialog:ctrlw` runs the Ctrl+W shortcut action of the visible dialog (X9)
                        if arg == "ctrlw" {
                            crate::ui::dialog_toast::dev_ctrl_w(&self.window);
                        }
                        eprintln!(
                            "dialog: open={} count={}",
                            self.window.visible_dialog().is_some(),
                            self.window.dialogs().n_items()
                        );
                    }
                    // `sync:dump|server=on|kosync=on|notes=on|plugin|prefs|close`: the Sync window
                    "sync" if arg == "prefs" => sender.input(Msg::ShowSyncPrefs),
                    "sync" => crate::ui::sync_window::dev(arg, &self.shared),
                    "tts" => crate::ui::tts_settings::dev(arg),
                    "mcp" => crate::ui::mcp_settings::dev(arg),
                    "keys" => crate::ui::shortcuts_dialog::dev(arg),
                    "jobs" => eprintln!("jobs: {}", crate::ui::jobs::dev(arg)),
                    "convert" => crate::ui::convert_options::dev(arg, &self.window),
                    // `action:NAME` activates an application action (`share`, `devices`, `stats`, `palette` …)
                    "action" => {
                        gio::prelude::ActionGroupExt::activate_action(
                            &relm4::main_adw_application(),
                            arg,
                            None,
                        );
                    }
                    "open-uri" => sender.input(Msg::OpenUri(arg.to_string())),
                    "settings" => crate::ui::settings_io::dev(arg, &self.shared),
                    // `safe:dump` prints `safe_mode: active=B plugins_dir=PATH plugins_found=N offline=B`
                    "safe" => eprintln!(
                        "safe_mode: active={} plugins_dir={} plugins_found={} offline={}",
                        crate::safe_mode::active(),
                        crate::plugins::plugins_dir().display(),
                        self.shared.plugins.borrow().list().len(),
                        bookx_sources::fetch::is_offline()
                    ),
                    // `launch:write=PATH|TEXT` (`\n` is a new line) makes a file; `launch:spawn=ARGS` starts a second
                    // `bookx` with these (space separated) arguments, as a launcher or file manager does, and
                    // prints `launch_spawn: exit=N` when it ends (0 = the running instance took the request); `launch:titles` lists the
                    // library's titles
                    "launch" => {
                        if let Some(w) = arg.strip_prefix("write=") {
                            if let Some((path, text)) = w.split_once('|') {
                                let r = std::fs::write(path, text.replace("\\n", "\n"));
                                eprintln!("launch_write: {:?}", r.map_err(|e| e.to_string()));
                            }
                        } else if arg == "titles" {
                            let t: Vec<String> = self
                                .shared
                                .lib
                                .borrow()
                                .books
                                .iter()
                                .map(|b| b.meta.title.clone())
                                .collect();
                            eprintln!("launch_titles: {}", t.join(" | "));
                        } else if let Some(args) = arg.strip_prefix("spawn=") {
                            let exe = std::env::current_exe().unwrap_or_default();
                            let child = std::process::Command::new(exe)
                                .args(args.split(' ').filter(|a| !a.is_empty()))
                                .env_remove("BOOKX_DEV_SCRIPT")
                                .spawn();
                            match child {
                                Ok(mut c) => {
                                    std::thread::spawn(move || {
                                        let code = c.wait().ok().and_then(|s| s.code());
                                        eprintln!("launch_spawn: exit={code:?}");
                                    });
                                }
                                Err(e) => eprintln!("launch_spawn: failed {e}"),
                            }
                        }
                    }
                    // `resolve-uri:bookx://…` prints which book of this library a link names
                    "resolve-uri" => {
                        let out = match crate::links::parse(arg) {
                            None => "invalid".to_string(),
                            Some(link) => {
                                match crate::links::resolve(&link, &self.shared.lib.borrow()) {
                                    Some(id) => format!("book {id}"),
                                    None => "no book".to_string(),
                                }
                            }
                        };
                        eprintln!("resolve_uri: {out}");
                    }
                    // `ntr:3` prints the plural forms in the current language
                    "ntr" => {
                        let n: u64 = arg.parse().unwrap_or(0);
                        eprintln!(
                            "ntr: {} | {}",
                            crate::i18n::ntr("{} book", "{} books", n),
                            crate::i18n::fmt_int(1_234_567)
                        );
                    }
                    "diag" => {
                        let books = self.shared.lib.borrow().books.len();
                        let text = crate::diagnostics::report(&self.shared.cfg.borrow(), books);
                        eprintln!("diag: {}", text.replace('\n', " | "));
                    }
                    "a11y-audit" => crate::ui::audit::report(),
                    "palette" => match arg {
                        "open" => sender.input(Msg::ShowPalette),
                        a => {
                            if let Some(out) = crate::ui::palette::dev(a) {
                                eprintln!("palette: {out}");
                            }
                        }
                    },
                    "reader" => {
                        if let Some(r) = &self.reader {
                            r.emit(reader::Input::Dev(arg.to_string()));
                        }
                        if let Some(r) = &self.pdf {
                            r.emit(reader_pdf::Input::Dev(arg.to_string()));
                        }
                        if let Some(r) = &self.comic {
                            r.emit(reader_comic::Input::Dev(arg.to_string()));
                        }
                    }
                    "library" => self.library.emit(library::Input::Dev(arg.to_string())),
                    "editor" => {
                        for (_, e) in self.editors.borrow().iter() {
                            e.emit(editor::Input::Dev(arg.to_string()));
                        }
                    }
                    "quit" => relm4::main_adw_application().quit(),
                    _ => {}
                }
            }
            Msg::CopyDiagnostics => {
                let books = self.shared.lib.borrow().books.len();
                let text = crate::diagnostics::report(&self.shared.cfg.borrow(), books);
                self.window.clipboard().set_text(&text);
                sender.input(Msg::Toast(tr("Diagnostics Copied")));
            }
            Msg::OpenUri(uri) => match crate::links::parse(&uri) {
                Some(link) => {
                    let found = crate::links::resolve(&link, &self.shared.lib.borrow());
                    match found {
                        Some(id) => {
                            self.window.present();
                            sender.input(Msg::Read(id));
                        }
                        None => sender
                            .input(Msg::Toast(tr("No book in this library matches that link."))),
                    }
                }
                None => sender.input(Msg::Toast(tr("That link is not one Bookx understands."))),
            },
            Msg::ShowAbout => {
                let d = adw::AboutDialog::builder()
                    .application_name(APP_NAME)
                    .application_icon(APP_ID)
                    .developer_name("Anurag Dhadse")
                    .version(VERSION)
                    .comments(tr("A modern library, reader and editor for e-books — written entirely in Rust."))
                    .website("https://github.com/adhadse/Bookx")
                    .issue_url("https://github.com/adhadse/Bookx/issues")
                    .license_type(gtk::License::Gpl30)
                    .copyright("© 2023–2026 Anurag Dhadse")
                    .developers(vec!["Anurag Dhadse".to_string()])
                    .translator_credits(translator_credits(&tr("translator-credits")))
                    .release_notes_version(VERSION)
                    .release_notes(release_notes(APPDATA, VERSION).unwrap_or_default())
                    .debug_info(crate::debug_log::export(
                        &self.shared.cfg.borrow(),
                        &self.shared.lib.borrow(),
                    ))
                    .debug_info_filename("bookx-debug-log.txt")
                    .build();
                d.present(Some(&self.front_window()));
            }
        }
    }
}

/// The AppStream file that ships with the app; the About dialog shows its notes of this version.
const APPDATA: &str = include_str!("../data/com.adhadse.Bookx.metainfo.xml");

/// The release notes of `version` in an AppStream metainfo file: the inside of the `<description>`
/// of its `<release>` (paragraphs and lists, which is what the About dialog shows).
fn release_notes(metainfo: &str, version: &str) -> Option<String> {
    let start = metainfo.find(&format!("<release version=\"{version}\""))?;
    let rest = &metainfo[start..];
    let end = rest.find("</release>")?;
    let rel = &rest[..end];
    let d0 = rel.find("<description>")? + "<description>".len();
    let d1 = rel.find("</description>")?;
    let lines: Vec<&str> = rel[d0..d1]
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// The translators line of the About dialog: empty (hidden) while a language has not translated
/// the `translator-credits` marker, as the gettext convention goes.
fn translator_credits(translated: &str) -> String {
    if translated == "translator-credits" {
        String::new()
    } else {
        translated.to_string()
    }
}

/// Minimal blocking GET used by the UI tests: the status code of `path` on `addr`.
fn http_status(addr: &str, path: &str) -> String {
    use std::io::{Read, Write};
    let Ok(mut s) = std::net::TcpStream::connect(addr) else {
        return "connection refused".into();
    };
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(3)));
    let _ = write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
    );
    let mut buf = String::new();
    let _ = s.read_to_string(&mut buf);
    buf.split_whitespace()
        .nth(1)
        .unwrap_or("no answer")
        .to_string()
}

impl App {
    /// E-ink mode: no animations, pure black on white, bars that stay.
    fn apply_eink(&self) {
        let on = self.shared.cfg.borrow().eink;
        crate::eink::set_active(on);
        let fx = crate::eink::effects(on);
        if let Some(settings) = gtk::Settings::default() {
            settings.set_gtk_enable_animations(fx.animations);
        }
        if on {
            self.window.add_css_class("eink");
        } else {
            self.window.remove_css_class("eink");
        }
        if let Some(r) = &self.reader {
            r.emit(reader::Input::Prefs(
                self.shared.cfg.borrow().reader.clone(),
            ));
        }
    }

    /// Large touch targets on touch screens (or when switched on).
    fn apply_touch_ui(&self) {
        let has_touch = gtk::gdk::Display::default()
            .and_then(|d| d.default_seat())
            .is_some_and(|s| s.capabilities().contains(gtk::gdk::SeatCapabilities::TOUCH));
        let on = self.shared.cfg.borrow().reader.touch_ui.enabled(has_touch);
        if on {
            self.window.add_css_class("touch");
        } else {
            self.window.remove_css_class("touch");
        }
    }

    /// Test hook: prints what the statistics window would show.
    fn dump_stats(&self) {
        let now = self.shared.now();
        let hub = self.shared.stats.borrow();
        let sum = crate::reading_stats::summarize(&hub.store, now, &chrono::Local);
        eprintln!("touch_ui: {}", self.window.has_css_class("touch"));
        eprintln!("stats_sessions: {}", hub.store.sessions().len());
        eprintln!("stats_today_secs: {}", sum.today_secs);
        eprintln!("stats_total_pages: {}", sum.total_pages);
        eprintln!("stats_streak: {}", sum.streak_current);
        for (k, v) in crate::ui::stats::summary_lines(&sum) {
            eprintln!("stats_line: {k} = {v}");
        }
        let review = crate::goals::review(&hub.store, now, &chrono::Local);
        for (k, v) in crate::goals::review_lines(&review) {
            eprintln!("stats_review: {k} = {v}");
        }
    }

    /// The opening page turns into a page that says why the book did not open, with a way back.
    fn show_open_error(&mut self, pdf: bool, error: &str) {
        let Some(page) = self.loading_page.take() else {
            self.nav.pop();
            return;
        };
        let (title, text) = open_error_texts(pdf, error);
        let status = adw::StatusPage::builder()
            .icon_name("dialog-error-symbolic")
            .title(title)
            .description(glib::markup_escape_text(&text))
            .build();
        status.set_widget_name("open-error");
        let back = gtk::Button::with_label(&tr("Back to Library"));
        back.add_css_class("pill");
        back.add_css_class("suggested-action");
        back.set_halign(gtk::Align::Center);
        let nav = self.nav.clone();
        back.connect_clicked(move |_| {
            nav.pop();
        });
        status.set_child(Some(&back));
        if let Some(tb) = page.child().and_downcast::<adw::ToolbarView>() {
            tb.set_content(Some(&status));
        }
        page.set_title(&tr("Could not open"));
    }

    fn restore_window(&mut self) {
        if self.was_maximized.take() == Some(false) {
            self.window.unmaximize();
        }
    }
}

impl App {
    /// The command palette: commands of the window that is showing, books, recent books, settings.
    /// The window a dialog opens over: the active window of the application (a book editor is its own window),
    /// else the main one.
    fn front_window(&self) -> gtk::Window {
        relm4::main_adw_application()
            .active_window()
            .filter(|w| w.is_visible())
            .unwrap_or_else(|| self.window.clone().upcast())
    }

    fn show_palette(&self, sender: &ComponentSender<Self>) {
        use crate::palette::{self, Scope, Target};
        if !self.window.is_active() && !self.window.is_visible() {
            return;
        }
        let in_reader = self.reader.is_some() && self.loading.is_none();
        let scope = if in_reader {
            Scope::Reader
        } else {
            Scope::Library
        };
        let mut entries = palette::commands(scope);
        {
            let lib = self.shared.lib.borrow();
            let mut books: Vec<&crate::core::library::Book> = lib.books.iter().collect();
            books.sort_by_key(|b| std::cmp::Reverse(b.last_read));
            for (n, b) in books.iter().enumerate() {
                let recent = n < 5 && b.last_read.is_some();
                entries.push(palette::book_entry(
                    b.id,
                    &b.meta.title,
                    &b.meta.authors.join(", "),
                    recent,
                ));
            }
        }
        entries.extend(preferences::index(&self.window, &self.shared));
        let ctx: gtk::Widget = match (&self.reader, in_reader) {
            (Some(r), true) => r.widget().clone().upcast(),
            _ => self.library.widget().clone().upcast(),
        };
        let tx = sender.input_sender().clone();
        crate::ui::palette::show(
            &self.window,
            entries,
            Rc::new(move |e| {
                if !crate::devtools::script().is_empty() {
                    eprintln!("palette: picked {} -> {:?}", e.title, e.target);
                }
                match e.target {
                    Target::Action(name) => match name.strip_prefix("app.") {
                        Some(short) => {
                            if let Some(app) = gio::Application::default() {
                                gio::prelude::ActionGroupExt::activate_action(&app, short, None);
                            }
                        }
                        None => {
                            let _ = ctx.activate_action(&name, None);
                        }
                    },
                    Target::Book(id) => tx.emit(Msg::PaletteBook(id)),
                    Target::Setting(title) => tx.emit(Msg::ShowPrefsAt(title)),
                    Target::ActionWith(..) | Target::Editor(_) | Target::File(_) => {}
                }
            }),
        );
    }
}

/// The Library section of the Keyboard Shortcuts window: `@action` takes the keys the user has
/// set, a literal is a fixed key. Shortcuts that work the same in every window (Preferences,
/// Keyboard Shortcuts, Help, the main menu) are listed in each section.
pub(crate) const LIBRARY_SHORTCUTS: &[(&str, &str)] = &[
    (N_("Add books"), "@lib.add-books"),
    (N_("Search"), "@lib.focus-search"),
    (N_("Edit metadata"), "@book.edit-meta"),
    (N_("Edit book"), "@book.edit-book"),
    (N_("Book properties"), "@book.properties"),
    (N_("Select all books"), "@lib.select-all"),
    (N_("Move selected books to Trash"), "@book.delete"),
    (N_("Clear the selection"), "Escape"),
    (N_("Show or hide the sidebar"), "@lib.toggle-sidebar"),
    (N_("Show or hide the details panel"), "<Shift>F9"),
    (N_("Open the main menu"), "F10"),
    (N_("Command palette"), "@app.palette"),
    (N_("Preferences"), "@app.preferences"),
    (N_("Keyboard Shortcuts"), "@app.shortcuts"),
    (N_("Online help"), "@app.help"),
    (N_("Quit"), "@app.quit"),
];

/// The Book editor section of the Keyboard Shortcuts window.
pub(crate) const EDITOR_SHORTCUTS: &[(&str, &str)] = &[
    (N_("Save"), "@editor.save"),
    (N_("Find & replace"), "@editor.find"),
    (N_("Find next"), "@editor.find-next"),
    (N_("Go to line"), "@editor.go-to-line"),
    (N_("Open a file"), "@editor.quick-open"),
    (N_("Show or hide the sidebar"), "@editor.sidebar"),
    (N_("Open the main menu"), "F10"),
    (N_("Command palette"), "@editor.palette"),
    (N_("Preferences"), "@app.preferences"),
    (N_("Keyboard Shortcuts"), "@app.shortcuts"),
    (N_("Check book"), "@editor.check"),
    (N_("Next problem"), "@editor.next-problem"),
    (N_("Previous problem"), "@editor.previous-problem"),
    (N_("Mark selected text"), "@editor.mark"),
    (N_("Insert a snippet"), "@editor.insert-snippet"),
    (N_("Undo"), "<Control>z"),
    (N_("Redo"), "<Control><Shift>z"),
];

fn show_shortcuts(parent: &impl IsA<gtk::Widget>) {
    let dialog = adw::ShortcutsDialog::new();
    let section = |title: &str, items: &[(&str, &str)]| {
        let s = adw::ShortcutsSection::new(Some(&tr(title)));
        for (t, a) in items {
            // "@action" takes the keys the user has set for that action
            let keys = match a.strip_prefix('@') {
                Some(action) => crate::shortcuts::current().effective(action).join(" "),
                None => a.to_string(),
            };
            if !keys.is_empty() {
                s.add(adw::ShortcutsItem::new(&tr(t), &keys));
            }
        }
        s
    };
    dialog.add(section(&tr("Library"), LIBRARY_SHORTCUTS));
    dialog.add(section(
        &tr("Reader"),
        &[
            (N_("Next page"), "@reader.next-page"),
            (N_("Previous page"), "@reader.prev-page"),
            (N_("Next chapter"), "@reader.next-chapter"),
            (N_("Previous chapter"), "@reader.prev-chapter"),
            (N_("Select text by character"), "<Shift>Right"),
            (N_("Select text by word"), "<Control><Shift>Right"),
            (N_("Select text by sentence"), "<Shift>Down"),
            (N_("Actions for the selection"), "Return"),
            (N_("Look up, Wikipedia, translate"), "d w t"),
            (N_("Highlight, add a note, copy"), "h n <Control>c"),
            (N_("Add to Vocabulary"), "v <Control><Shift>v"),
            (N_("Command palette"), "@app.palette"),
            (N_("Search in book"), "@reader.search"),
            (N_("Contents"), "@reader.contents"),
            (N_("Toggle bookmark"), "@reader.bookmark"),
            (N_("Bookmarks and highlights"), "@reader.notes"),
            (N_("Go to"), "@reader.goto"),
            (N_("Back to the previous place"), "@reader.jump-back"),
            (N_("Book map"), "@reader.map"),
            (N_("Next theme"), "@reader.cycle-theme"),
            (N_("Read aloud"), "@reader.read-aloud"),
            (N_("Larger text"), "@reader.larger"),
            (N_("Smaller text"), "@reader.smaller"),
            (N_("Reset text size"), "@reader.reset-size"),
            (N_("Fullscreen"), "@reader.fullscreen"),
            (N_("Book properties"), "@reader.properties"),
            (N_("Reload the book"), "@reader.reload"),
            (N_("Back to library"), "@reader.back"),
        ],
    ));
    dialog.add(section(
        &tr("Comics"),
        &[
            (N_("Next or previous page"), "Page_Down Page_Up"),
            (N_("Read on, or next panel in guided view"), "space"),
            (N_("Back, or previous panel in guided view"), "<Shift>space"),
            (N_("Zoom in, out, fit"), "plus minus 0"),
            (N_("Guided view"), "g"),
            (N_("Reading direction"), "r"),
            (N_("Layout: single, two pages, webtoon"), "m"),
            (N_("Page thumbnails"), "F9"),
        ],
    ));
    dialog.add(section(&tr("Book editor"), EDITOR_SHORTCUTS));
    dialog.present(Some(parent));
}

#[cfg(test)]
mod shortcut_list_tests {
    use super::{EDITOR_SHORTCUTS, LIBRARY_SHORTCUTS};

    fn resolved(list: &[(&str, &str)], title: &str) -> String {
        let (_, a) = list.iter().find(|(t, _)| *t == title).expect(title);
        match a.strip_prefix('@') {
            Some(action) => crate::shortcuts::current().effective(action).join(" "),
            None => a.to_string(),
        }
    }

    /// XC-6: the standard keys are documented in the Keyboard Shortcuts window.
    #[test]
    fn the_standard_keys_are_listed_for_the_library_and_the_editor() {
        for (list, side) in [
            (LIBRARY_SHORTCUTS, "Show or hide the sidebar"),
            (EDITOR_SHORTCUTS, "Show or hide the sidebar"),
        ] {
            assert_eq!(resolved(list, side), "F9");
            assert_eq!(resolved(list, "Open the main menu"), "F10");
            assert_eq!(resolved(list, "Preferences"), "<Control>comma");
            assert_eq!(resolved(list, "Keyboard Shortcuts"), "<Control>question");
        }
        assert_eq!(
            resolved(LIBRARY_SHORTCUTS, "Select all books"),
            "<Control>a"
        );
        assert_eq!(
            resolved(LIBRARY_SHORTCUTS, "Move selected books to Trash"),
            "Delete"
        );
        assert_eq!(resolved(LIBRARY_SHORTCUTS, "Clear the selection"), "Escape");
        assert_eq!(resolved(LIBRARY_SHORTCUTS, "Online help"), "F1");
    }

    /// Every `@action` of the lists is an action of the shortcut registry (so it can be rebound).
    #[test]
    fn listed_actions_are_in_the_registry() {
        let reg = crate::shortcuts::registry();
        for (t, a) in LIBRARY_SHORTCUTS.iter().chain(EDITOR_SHORTCUTS) {
            if let Some(action) = a.strip_prefix('@') {
                assert!(reg.iter().any(|r| r.action == action), "{t}: {action}");
            }
        }
    }
}

#[cfg(test)]
mod about_tests {
    use super::{release_notes, translator_credits, APPDATA};
    use crate::config::VERSION;

    #[test]
    fn the_about_dialog_shows_the_notes_of_this_version() {
        let n = release_notes(APPDATA, VERSION).expect("a <release> for the version in Cargo.toml");
        assert!(n.starts_with("<p>") && n.contains("<li>"), "{n}");
        assert!(release_notes(APPDATA, "0.0.0-none").is_none());
    }

    #[test]
    fn untranslated_credits_are_hidden() {
        assert_eq!(translator_credits("translator-credits"), "");
        assert_eq!(
            translator_credits("Ana <ana@example.org>"),
            "Ana <ana@example.org>"
        );
    }
}
