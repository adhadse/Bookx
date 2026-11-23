//! Window and workflow: the Book menu (save a copy, open another book, new book, import), revert to before/after,
//! closing several tabs, line wrapping, remembered tabs and cursors, help, and opening named files from the
//! command line (CE-005, CE-008, CE-011..CE-015, CE-017, CE-018).

use crate::i18n::{tr, trf};
use bookx_core::library::AddOutcome;

use super::tools_ui::*;
use super::*;
use crate::editor_state::{self, BookState, CloseMode, StateStore, WindowPrefs};

pub const HELP_URL: &str = "https://bookx.adhadse.com/editing/";

/// What the extra editor features keep between calls.
pub struct State {
    pub prefs: RefCell<WindowPrefs>,
    /// Redo points made by "Revert to Before", newest last.
    pub redo: RefCell<Vec<u64>>,
    /// Watches on files opened in other programs.
    pub monitors: RefCell<Vec<gio::FileMonitor>>,
    /// The buttons and preview text of the tool dialog opened last (the dev hooks click them).
    pub last_dialog: RefCell<Option<(gtk::Button, gtk::Button, gtk::Label)>>,
    /// Heading and body of the last delete confirmation (read by the dev hook `delete-ask=`).
    pub last_alert: RefCell<String>,
    /// A file changed by another program that waits for the user's answer (path in the book, temporary file).
    pub pending_external: Rc<RefCell<Option<(String, std::path::PathBuf)>>>,
    /// The tab a context menu was opened on.
    pub menu_page: Rc<RefCell<Option<adw::TabPage>>>,
    /// The structure tools (`struct_hub.rs`).
    pub structure: Rc<super::struct_hub::State>,
    /// The preview was hidden by the spelling pane in a narrow window (it comes back when the pane closes).
    pub spell_hid_preview: Cell<bool>,
    /// The `ext` action group (the dev hooks read the enabled state of its actions).
    pub group: RefCell<Option<gio::SimpleActionGroup>>,
}

impl State {
    pub fn new() -> State {
        State {
            prefs: RefCell::new(WindowPrefs::load()),
            redo: RefCell::new(Vec::new()),
            monitors: RefCell::new(Vec::new()),
            last_dialog: RefCell::new(None),
            last_alert: RefCell::new(String::new()),
            pending_external: Rc::new(RefCell::new(None)),
            menu_page: Rc::new(RefCell::new(None)),
            structure: Rc::new(super::struct_hub::State::default()),
            group: RefCell::new(None),
            spell_hid_preview: Cell::new(false),
        }
    }
}

fn find_menu(e: &Editor, name: &str) -> Option<gio::Menu> {
    e.w.menus
        .iter()
        .find(|(n, _)| n == name)
        .and_then(|(_, m)| m.clone().downcast::<gio::Menu>().ok())
}

fn toggle_action(
    name: &str,
    on: bool,
    tx: &relm4::Sender<Input>,
    make: fn(bool) -> Cmd,
) -> gio::SimpleAction {
    let act = gio::SimpleAction::new_stateful(name, None, &on.to_variant());
    let tx = tx.clone();
    act.connect_activate(move |a, _| {
        let now = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(false);
        a.set_state(&now.to_variant());
        tx.emit(Input::Ext(make(now)));
    });
    act
}

/// The labels and action targets of the Book menu's top level, in order (sections separated by `None`); the unit
/// test below keeps the count within the 12 entries the audit allows.
pub(super) fn book_menu() -> gio::Menu {
    let m = gio::Menu::new();
    let item = |label: &str, id: &str| menu_item(label, "ext.win", id);
    let file = gio::Menu::new();
    file.append_item(&item(&tr("Save a Copy"), "save-copy"));
    file.append_item(&item(&tr("Save a Copy and Edit It"), "save-copy-edit"));
    m.append_section(None, &file);
    let books = gio::Menu::new();
    books.append_item(&item(&tr("Open Another Book"), "open-book"));
    books.append_item(&item(&tr("New Book"), "new-book"));
    let more = gio::Menu::new();
    more.append_item(&item(&tr("Open an Unzipped Folder"), "open-folder"));
    more.append_item(&menu_item(
        &tr("Open Another Format"),
        "ext.tool",
        "open-foreign",
    ));
    more.append_item(&item(&tr("Import as New Book"), "import-file"));
    books.append_submenu(Some(&tr("More Ways to Open")), &more);
    m.append_section(None, &books);
    let hist = gio::Menu::new();
    let revert = gio::Menu::new();
    revert.append(
        Some(&tr("Revert to Before the Last Change")),
        Some("ext.undo-checkpoint"),
    );
    revert.append(
        Some(&tr("Revert to After (Redo)")),
        Some("ext.redo-checkpoint"),
    );
    hist.append_submenu(Some(&tr("Revert")), &revert);
    m.append_section(None, &hist);
    let view = gio::Menu::new();
    let palette = menu_item(&tr("Command Palette"), "ext.win", "palette");
    if let Some(a) = crate::shortcuts::current()
        .effective("editor.palette")
        .into_iter()
        .next()
    {
        palette.set_attribute_value("accel", Some(&a.to_variant()));
    }
    view.append_item(&palette);
    view.append(Some(&tr("Wrap Long Lines")), Some("ext.wrap"));
    m.append_section(None, &view);
    let app = gio::Menu::new();
    app.append(Some(&tr("Preferences")), Some("app.preferences"));
    app.append(Some(&tr("Keyboard Shortcuts")), Some("app.shortcuts"));
    app.append_item(&item(&tr("Help"), "help"));
    app.append(Some(&tr("Report a Problem")), Some("app.report-problem-from::Editor"));
    app.append(Some(&tr("About Bookx")), Some("app.about"));
    m.append_section(None, &app);
    m
}

/// The labels of the top level of `menu` (items of all sections, submenus counted once).
pub(super) fn top_level_labels(menu: &gio::MenuModel) -> Vec<String> {
    let mut out = Vec::new();
    for i in 0..menu.n_items() {
        match menu.item_link(i, "section") {
            Some(s) => {
                out.extend(top_level_labels(&s));
            }
            _ => {
                if let Some(v) =
                    menu.item_attribute_value(i, "label", Some(glib::VariantTy::STRING))
                {
                    out.push(v.str().unwrap_or_default().to_string());
                }
            }
        }
    }
    out
}

/// The main menu button of the header: the Book menu (the primary menu, so F10 opens it).
pub(super) fn book_button() -> gtk::MenuButton {
    let btn = gtk::MenuButton::new();
    btn.set_icon_name("open-menu-symbolic");
    btn.set_primary(true);
    btn.set_tooltip_text(Some(&tr("Main Menu")));
    crate::ui::audit::label(&btn, &tr("Main Menu"));
    btn.set_menu_model(Some(&book_menu()));
    btn
}

impl Editor {
    /// Hooks everything of this file and `tools_ui.rs`/`files_extra.rs` into the window. Called once at the end of
    /// `init`.
    pub(super) fn ext_install(&mut self) {
        let tx = self.sender.input_sender().clone();
        let group = tools_ui::actions(&tx);
        for name in ["files", "win"] {
            let a = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
            let t = tx.clone();
            a.connect_activate(move |_, p| {
                if let Some(id) = p.and_then(|v| v.str()) {
                    t.emit(Input::Ext(if name == "files" {
                        Cmd::Files(id.to_string())
                    } else {
                        Cmd::Win(id.to_string())
                    }));
                }
            });
            group.add_action(&a);
        }
        for id in ["undo-checkpoint", "redo-checkpoint"] {
            let a = gio::SimpleAction::new(id, None);
            let t = tx.clone();
            a.connect_activate(move |_, _| t.emit(Input::Ext(Cmd::Win(id.to_string()))));
            group.add_action(&a);
        }
        let prefs = self.ext.prefs.borrow().clone();
        group.add_action(&toggle_action("wrap", prefs.wrap_lines, &tx, |on| {
            Cmd::Win(format!("wrap:{on}"))
        }));
        group.add_action(&toggle_action("full-paths", prefs.full_paths, &tx, |_| {
            Cmd::Files("full-paths".into())
        }));
        self.w.win.insert_action_group("ext", Some(&group));
        *self.ext.group.borrow_mut() = Some(group.clone());
        self.struct_install();

        if let Some(m) = find_menu(self, "Files") {
            files_extra::extend_files_menu(&m);
        }
        self.install_tab_menu();
        self.install_shortcuts();
        self.refresh_revert_actions();
        self.apply_wrap(prefs.wrap_lines);
        let t = tx.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
            t.emit(Input::Ext(Cmd::Win("restore-state".into())));
        });
    }

    fn install_tab_menu(&self) {
        let menu = gio::Menu::new();
        menu.append_item(&menu_item(&tr("Close Tab"), "ext.win", "tab-close"));
        menu.append_item(&menu_item(&tr("Close Other Tabs"), "ext.win", "tab-others"));
        menu.append_item(&menu_item(
            &tr("Close Tabs to the Right"),
            "ext.win",
            "tab-right",
        ));
        self.w.tabs.set_menu_model(Some(&menu));
        // the entries act on the tab the menu was opened on
        let page = self.ext.menu_page.clone();
        self.w.tabs.connect_setup_menu(move |_, p| {
            if let Some(p) = p {
                *page.borrow_mut() = Some(p.clone());
            }
        });
    }

    fn install_shortcuts(&self) {
        let sc = gtk::ShortcutController::new();
        sc.set_scope(gtk::ShortcutScope::Local);
        let add = |accel: &str, id: &'static str| {
            let tx = self.sender.input_sender().clone();
            if let Some(t) = gtk::ShortcutTrigger::parse_string(accel) {
                sc.add_shortcut(gtk::Shortcut::new(
                    Some(t),
                    Some(gtk::CallbackAction::new(move |_, _| {
                        tx.emit(Input::Ext(Cmd::Win(id.to_string())));
                        glib::Propagation::Stop
                    })),
                ));
            }
        };
        add("<Control><Shift>s", "save-copy");
        add("<Control><Alt><Shift>s", "save-copy-edit");
        add("<Control>o", "open-book");
        add("<Control><Alt>z", "undo-checkpoint");
        add("<Control><Alt><Shift>z", "redo-checkpoint");
        add("<Control><Shift>w", "close-others");
        add("<Alt><Shift>w", "close-right");
        add("F1", "help");
        self.w.win.add_controller(sc);
    }

    pub(super) fn apply_wrap(&self, on: bool) {
        self.w.view.set_wrap_mode(if on {
            gtk::WrapMode::WordChar
        } else {
            gtk::WrapMode::None
        });
    }

    // ---- actions -----------------------------------------------------------

    pub(super) fn window_action(&mut self, id: &str) {
        if let Some(on) = id.strip_prefix("wrap:") {
            let on = on == "true";
            {
                let mut p = self.ext.prefs.borrow_mut();
                p.wrap_lines = on;
                p.save();
            }
            self.apply_wrap(on);
            return;
        }
        if let Some(rest) = id.strip_prefix("open-book:") {
            if let Ok(n) = rest.parse::<u64>() {
                self.switch_book(n);
            }
            return;
        }
        if let Some(rest) = id.strip_prefix("force:") {
            self.leave_confirmed(rest);
            return;
        }
        match id {
            "save-copy" | "save-copy-edit" => self.prompt_save_copy(id == "save-copy-edit"),
            "open-book" | "open-folder" | "new-book" | "import-file" => {
                if self.is_dirty() {
                    let tx = self.sender.input_sender().clone();
                    let id = id.to_string();
                    alert(
                        &self.w.win,
                        "Discard Unsaved Changes?",
                        "This window will show another book. Save first if you want to keep your changes.",
                        "Discard and Continue",
                        true,
                        move || tx.emit(Input::Ext(Cmd::Win(format!("force:{id}")))),
                    );
                } else {
                    self.leave_confirmed(id);
                }
            }
            "undo-checkpoint" => self.revert_before(),
            "redo-checkpoint" => self.revert_after(),
            "close-others" => self.close_tabs(CloseMode::Others, None),
            "close-right" => self.close_tabs(CloseMode::Right, None),
            "tab-close" => self.close_tab_page(),
            "tab-others" | "tab-right" => {
                let page = self.ext.menu_page.borrow_mut().take();
                let path = page.and_then(|p| super::row_data::path(&p));
                let mode = if id == "tab-others" {
                    CloseMode::Others
                } else {
                    CloseMode::Right
                };
                self.close_tabs(mode, path);
            }
            "help" => self.open_help(),
            "palette" => self.show_palette(),
            "restore-state" => self.restore_state(),
            "save-state" => self.save_state(),
            other => self.toast(&trf("Unknown action {}", &[&other])),
        }
    }

    fn leave_confirmed(&mut self, id: &str) {
        match id {
            "open-book" => self.pick_library_book(),
            "open-folder" => {
                let dlg = gtk::FileDialog::builder()
                    .title(tr("Open an Unzipped EPUB Folder"))
                    .build();
                let tx = self.sender.input_sender().clone();
                dlg.select_folder(Some(&self.w.win), gio::Cancellable::NONE, move |res| {
                    if let Some(p) = res.ok().and_then(|f| f.path()) {
                        tx.emit(Input::Ext(Cmd::Dev(format!("open-folder={}", p.display()))));
                    }
                });
            }
            "new-book" => self.prompt_new_book(),
            "import-file" => {
                let filter = gtk::FileFilter::new();
                filter.set_name(Some(&tr("HTML, DOCX, text and other documents")));
                for p in [
                    "*.html", "*.htm", "*.xhtml", "*.docx", "*.odt", "*.rtf", "*.txt", "*.md",
                    "*.fb2",
                ] {
                    filter.add_pattern(p);
                }
                let filters = gio::ListStore::new::<gtk::FileFilter>();
                filters.append(&filter);
                let dlg = gtk::FileDialog::builder()
                    .title(tr("Import as a New Book"))
                    .filters(&filters)
                    .build();
                let tx = self.sender.input_sender().clone();
                dlg.open(Some(&self.w.win), gio::Cancellable::NONE, move |res| {
                    if let Some(p) = res.ok().and_then(|f| f.path()) {
                        tx.emit(Input::Ext(Cmd::Dev(format!("import-file={}", p.display()))));
                    }
                });
            }
            _ => {}
        }
    }

    // ---- save a copy ---------------------------------------------------------

    fn prompt_save_copy(&self, edit: bool) {
        let dlg = gtk::FileDialog::builder()
            .title(if edit {
                tr("Save a Copy and Edit It")
            } else {
                tr("Save a Copy")
            })
            .initial_name(format!("{}.epub", self.book_title()))
            .build();
        let tx = self.sender.input_sender().clone();
        dlg.save(Some(&self.w.win), gio::Cancellable::NONE, move |res| {
            if let Some(p) = res.ok().and_then(|f| f.path()) {
                let verb = if edit { "save-copy-edit" } else { "save-copy" };
                tx.emit(Input::Ext(Cmd::Dev(format!("{verb}={}", p.display()))));
            }
        });
    }

    /// Writes the book as it is now (unsaved edits included) to `path`; with `edit` the copy is added to the
    /// library and this window continues with the copy.
    pub(super) fn save_copy(&mut self, path: &std::path::Path, edit: bool) {
        self.flush();
        if !self.commit_toc() {
            return;
        }
        let res = self.book.borrow().archive.save(path);
        if let Err(e) = res {
            self.toast(&trf("Could not save the copy: {}", &[&e]));
            return;
        }
        if !edit {
            self.toast(&trf("Saved a copy to {}", &[&path.display()]));
            return;
        }
        match self.add_to_library(path) {
            Ok(id) => self.switch_book(id),
            Err(e) => self.toast(&trf(
                "Saved the copy, but could not add it to the library: {}",
                &[&format!("{:#}", e)],
            )),
        }
    }

    fn add_to_library(&self, path: &std::path::Path) -> anyhow::Result<u64> {
        let out = {
            let mut lib = self.shared.lib.borrow_mut();
            let out = lib.add_path(path, true)?;
            let _ = lib.save();
            out
        };
        let _ = self.sender.output(Output::Saved);
        Ok(match out {
            AddOutcome::Added(id) | AddOutcome::Duplicate(id) => id,
        })
    }

    // ---- other books ----------------------------------------------------------

    /// Shows another book of the library in this window.
    pub(super) fn switch_book(&mut self, id: u64) {
        let opened = self.shared.lib.borrow().open_book(id);
        let book = match opened {
            Ok(b) => b,
            Err(e) => {
                self.toast(&trf("Cannot open that book: {}", &[&e]));
                return;
            }
        };
        *self.ext.structure.origin.borrow_mut() = None;
        self.install_book(id, book);
    }

    pub(super) fn install_book(&mut self, id: u64, book: OpenBook) {
        self.save_state();
        *self.book.borrow_mut() = book;
        self.book_id = id;
        self.checkpoints = Checkpoints::new(25);
        self.baseline = None;
        self.original.clear();
        self.marks.clear();
        self.ext.redo.borrow_mut().clear();
        self.tabs_guard.set(true);
        while self.w.tabs.n_pages() > 0 {
            let p = self.w.tabs.nth_page(0);
            self.w.tabs.close_page(&p);
        }
        self.tabs_guard.set(false);
        self.current = None;
        self.after_archive_changed();
        self.structure_dirty = false;
        self.refresh_restore_menu();
        self.update_title();
        self.w.banner.set_revealed(false);
        // after the selection changes the rebuilt list has queued
        self.sender
            .input(Input::Ext(Cmd::Win("restore-state".into())));
        self.toast(&trf("Editing “{}”", &[&self.book_title()]));
    }

    fn pick_library_book(&self) {
        let dialog = crate::ui::dialog_toast::new_dialog();
        dialog.set_title(&tr("Open Another Book"));
        let (w, h) = fit_dialog(&self.w.win, 480, 520);
        dialog.set_content_width(w);
        dialog.set_content_height(h);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some(&tr("Search the library")));
        crate::ui::audit::label(&search, &tr("Search the library"));
        let list = gtk::ListBox::new();
        list.add_css_class("navigation-sidebar");
        let books: Vec<(u64, String)> = self
            .shared
            .lib
            .borrow()
            .books
            .iter()
            .map(|b| {
                (
                    b.id,
                    format!("{} — {}", b.meta.title, b.meta.authors.join(", ")),
                )
            })
            .collect();
        let fill = {
            let list = list.clone();
            Rc::new(move |q: &str| {
                list.remove_all();
                let q = q.to_lowercase();
                for (id, label) in books
                    .iter()
                    .filter(|(_, l)| l.to_lowercase().contains(&q))
                    .take(200)
                {
                    let row = gtk::ListBoxRow::new();
                    let l = gtk::Label::new(Some(label));
                    l.set_xalign(0.0);
                    l.set_ellipsize(pango::EllipsizeMode::End);
                    l.set_margin_all(6);
                    row.set_child(Some(&l));
                    super::row_data::set_id(&row, *id);
                    list.append(&row);
                }
            })
        };
        fill("");
        {
            let f = fill.clone();
            search.connect_search_changed(move |e| f(&e.text()));
        }
        {
            let (tx, d) = (self.sender.input_sender().clone(), dialog.clone());
            list.connect_row_activated(move |_, r| {
                if let Some(id) = super::row_data::id(r) {
                    tx.emit(Input::Ext(Cmd::Win(format!("open-book:{id}"))));
                    d.close();
                }
            });
        }
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_child(Some(&list));
        let bx = gtk::Box::new(gtk::Orientation::Vertical, 6);
        bx.set_margin_all(10);
        bx.append(&search);
        bx.append(&scroll);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&bx));
        dialog.set_child(Some(&view));
        dialog.present(Some(&self.w.win));
    }

    fn prompt_new_book(&self) {
        let entry = gtk::Entry::new();
        entry.set_text(&tr("New Book"));
        entry.set_activates_default(true);
        crate::ui::audit::label(&entry, &tr("Title of the new book"));
        let tx = self.sender.input_sender().clone();
        self.ask(
            "New Book",
            "A book with one empty chapter is added to the library and opened here.",
            &entry.clone(),
            "Create",
            move || tx.emit(Input::Ext(Cmd::Dev(format!("new-book={}", entry.text())))),
        );
    }

    pub(super) fn new_book(&mut self, title: &str) {
        let title = if title.trim().is_empty() {
            "New Book"
        } else {
            title.trim()
        };
        let md = crate::core::opf::Metadata {
            title: title.to_string(),
            authors: vec!["Unknown".into()],
            language: "en".into(),
            ..Default::default()
        };
        let a = crate::core::convert::create_epub(
            &md,
            &[crate::core::convert::Chapter {
                title: "Chapter 1".into(),
                body: "<h1>Chapter 1</h1>\n<p></p>".into(),
            }],
            None,
        );
        self.add_archive(a, title);
    }

    pub(super) fn open_folder(&mut self, dir: &std::path::Path) {
        match editor_tools::archive_from_folder(dir) {
            Ok(a) => {
                let name = dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("book")
                    .to_string();
                self.add_archive(a, &name);
            }
            Err(e) => self.toast(&format!("{e:#}")),
        }
    }

    fn add_archive(&mut self, a: crate::core::archive::Archive, name: &str) {
        let dir = std::env::temp_dir().join(format!("bookx-new-{}", uuid::Uuid::new_v4()));
        let safe: String = name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == ' ' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let path = dir.join(format!("{safe}.epub"));
        let res = std::fs::create_dir_all(&dir)
            .map_err(anyhow::Error::from)
            .and_then(|_| a.save(&path))
            .and_then(|_| self.add_to_library(&path));
        let _ = std::fs::remove_dir_all(&dir);
        match res {
            Ok(id) => self.switch_book(id),
            Err(e) => self.toast(&trf("Could not add the book: {}", &[&format!("{:#}", e)])),
        }
    }

    pub(super) fn import_file(&mut self, path: &std::path::Path) {
        match self.add_to_library(path) {
            Ok(id) => self.switch_book(id),
            Err(e) => self.toast(&trf(
                "Could not import that file: {}",
                &[&format!("{:#}", e)],
            )),
        }
    }

    // ---- revert to before / after (CE-008) ---------------------------------------

    fn checkpoint_names(&self) -> Vec<(u64, String)> {
        self.checkpoints
            .list()
            .into_iter()
            .map(|c| (c.id, c.name))
            .collect()
    }

    /// Greys out the two revert items while there is nothing to revert or redo.
    pub(super) fn refresh_revert_actions(&self) {
        let (undo, redo) =
            editor_state::revert_enabled(&self.checkpoint_names(), &self.ext.redo.borrow());
        if let Some(g) = self.ext.group.borrow().as_ref() {
            for (id, on) in [("undo-checkpoint", undo), ("redo-checkpoint", redo)] {
                if let Some(a) = g.lookup_action(id).and_downcast::<gio::SimpleAction>() {
                    a.set_enabled(on);
                }
            }
        }
    }

    pub(super) fn revert_before(&mut self) {
        self.flush();
        let Some(target) = editor_state::undo_target(&self.checkpoint_names()) else {
            self.toast(&tr("Nothing to revert to"));
            return;
        };
        let name = self
            .checkpoints
            .list()
            .into_iter()
            .find(|c| c.id == target)
            .map(|c| c.name)
            .unwrap_or_default();
        let label = name.strip_prefix("Before: ").unwrap_or(&name).to_string();
        let redo = self.create_checkpoint(&format!("{}{label}", editor_state::REDO_PREFIX));
        let res = self
            .checkpoints
            .restore_into(target, &mut self.book.borrow_mut().archive);
        match res {
            Ok(()) => {
                let _ = self.checkpoints.delete(target);
                self.ext.redo.borrow_mut().push(redo);
                self.after_archive_changed();
                self.refresh_restore_menu();
                let _ = self.toast_button(
                    &trf("Reverted to before “{}”", &[&label]),
                    &tr("Redo"),
                    || Input::Ext(Cmd::Win("redo-checkpoint".into())),
                );
            }
            Err(e) => {
                let _ = self.checkpoints.delete(redo);
                self.toast(&trf("Could not revert: {}", &[&e]));
            }
        }
    }

    pub(super) fn revert_after(&mut self) {
        self.flush();
        let top = self.ext.redo.borrow().last().copied();
        let Some(redo) =
            top.filter(|r| editor_state::redo_still_valid(&self.checkpoint_names(), *r))
        else {
            self.ext.redo.borrow_mut().clear();
            self.toast(&tr("Nothing to redo"));
            return;
        };
        self.create_checkpoint(editor_state::REDO_GUARD);
        let res = self
            .checkpoints
            .restore_into(redo, &mut self.book.borrow_mut().archive);
        match res {
            Ok(()) => {
                let _ = self.checkpoints.delete(redo);
                self.ext.redo.borrow_mut().pop();
                self.after_archive_changed();
                self.refresh_restore_menu();
                let _ = self.toast_button(&tr("Reverted to after the change"), &tr("Undo"), || {
                    Input::Ext(Cmd::Win("undo-checkpoint".into()))
                });
            }
            Err(e) => self.toast(&trf("Could not redo: {}", &[&e])),
        }
    }

    // ---- tabs (CE-011) --------------------------------------------------------------

    fn close_tab_page(&self) {
        let page = self.ext.menu_page.borrow_mut().take();
        match page {
            Some(p) => self.w.tabs.close_page(&p),
            None => self.close_current_tab(),
        }
    }

    pub(super) fn close_tabs(&mut self, mode: CloseMode, keep: Option<String>) {
        let keep = keep.or_else(|| self.current.clone()).unwrap_or_default();
        let tabs = self.tab_paths();
        let gone = editor_state::tabs_to_close(&tabs, &keep, mode);
        if gone.is_empty() {
            return;
        }
        self.tabs_guard.set(true);
        for i in (0..self.w.tabs.n_pages()).rev() {
            let page = self.w.tabs.nth_page(i);
            let path = super::row_data::path(&page);
            if path.is_some_and(|p| gone.contains(&p)) {
                self.w.tabs.close_page(&page);
            }
        }
        self.tabs_guard.set(false);
        if !keep.is_empty() && self.book.borrow().archive.contains(&keep) {
            self.select_file(&keep);
        }
    }

    // ---- remembered state (CE-015) -----------------------------------------------------

    pub(super) fn save_state(&self) {
        if !self.ext.prefs.borrow().restore_state {
            return;
        }
        let mut st = BookState {
            tabs: self.tab_paths(),
            current: self.current.clone(),
            ..Default::default()
        };
        for (path, buf) in &self.buffers {
            let it = buf.iter_at_mark(&buf.get_insert());
            st.cursors
                .insert(path.clone(), (it.line(), it.line_offset()));
        }
        let mut store = StateStore::load();
        store.put(&self.book_id.to_string(), st);
        store.save();
    }

    pub(super) fn restore_state(&mut self) {
        if !self.ext.prefs.borrow().restore_state {
            return;
        }
        let store = StateStore::load();
        let Some(mut st) = store.get(&self.book_id.to_string()).cloned() else {
            return;
        };
        {
            let b = self.book.borrow();
            st.retain_existing(|p| b.archive.contains(p));
        }
        for t in &st.tabs {
            self.show_tab(t);
        }
        for (path, (line, col)) in &st.cursors {
            if is_text_path(path) {
                let buf = self.buffer_for(path);
                if let Some(mut it) = buf.iter_at_line(*line) {
                    it.forward_chars(*col);
                    buf.place_cursor(&it);
                }
            }
        }
        if let Some(c) = st.current.clone().or_else(|| st.tabs.first().cloned()) {
            self.select_file(&c);
            let buf = self.w.view.buffer();
            let mut it = buf.iter_at_mark(&buf.get_insert());
            self.w.view.scroll_to_iter(&mut it, 0.1, true, 0.5, 0.3);
        }
    }

    // ---- help and command line ------------------------------------------------------------

    fn open_help(&self) {
        gtk::UriLauncher::new(HELP_URL).launch(Some(&self.w.win), gio::Cancellable::NONE, |_| {});
    }

    /// Opens the named files in tabs and, with `select`, selects the first occurrence of the text in the last one
    /// (`bookx --edit book.epub OEBPS/ch1.xhtml --select-text "some words"`).
    pub(super) fn open_names(&mut self, names: &[String], select: Option<&str>) {
        let mut last = None;
        for n in names {
            let found = {
                let b = self.book.borrow();
                if b.archive.contains(n) {
                    Some(n.clone())
                } else {
                    let tail = format!("/{n}");
                    b.file_list()
                        .into_iter()
                        .find(|f| f.ends_with(&tail) || file_name(f) == n)
                }
            };
            match found {
                Some(p) => {
                    self.select_file(&p);
                    last = Some(p);
                }
                None => self.toast(&trf("The book has no file called {}", &[&n])),
            }
        }
        if last.is_none() {
            last = self.current.clone();
        }
        if let (Some(text), Some(p)) = (select, last) {
            self.select_file(&p);
            let buf = self.w.view.buffer();
            let full = buffer_text(&buf);
            if let Some(at) = full.find(text) {
                let a = super::search::byte_to_char_idx(&full, at) as i32;
                let (s, e) = (
                    buf.iter_at_offset(a),
                    buf.iter_at_offset(a + text.chars().count() as i32),
                );
                buf.select_range(&s, &e);
                let mut it = buf.iter_at_offset(a);
                self.w.view.scroll_to_iter(&mut it, 0.1, true, 0.5, 0.3);
                self.w.view.grab_focus();
            } else {
                self.toast(&trf("“{}” is not in {}", &[&text, &file_name(&p)]));
            }
        }
    }
}

#[cfg(test)]
mod menu_tests {
    use super::*;

    #[test]
    fn the_main_menu_stays_within_twelve_entries_and_ends_with_the_standard_group() {
        let top = top_level_labels(book_menu().upcast_ref());
        assert!(top.len() <= 12, "{top:?}");
        assert_eq!(
            top[top.len() - 4..],
            ["Preferences", "Keyboard Shortcuts", "Help", "About Bookx"]
        );
        let mut sorted = top.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), top.len(), "a label is repeated: {top:?}");
        assert!(!top.iter().any(|l| l.starts_with("Close")));
    }
}
