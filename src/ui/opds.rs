//! The *Online Catalogues* page of the library (a destination of the sidebar): browse, search and download from OPDS catalogues (Project
//! Gutenberg, Standard Ebooks, a Calibre server, another Bookx). Downloaded books go through the
//! normal import. Logic: `crate::opds_browse`; the network runs on worker threads.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use bookx_serve::opds_client::{Entry, Feed};
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use super::SharedRef;
use crate::i18n::tr;
use crate::i18n::trf;
use crate::opds_browse as ob;

/// Imports downloaded files into the library.
pub type Import = Rc<dyn Fn(Vec<PathBuf>)>;

thread_local! {
    static CURRENT: RefCell<Option<Rc<Ui>>> = const { RefCell::new(None) };
}

/// What the busy page is waiting for, so Cancel knows how to go back.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Busy {
    Idle,
    Search,
    Page,
}

struct Ui {
    shared: SharedRef,
    import: Import,
    /// The page itself; the dialogs of the page (Add a Catalogue) are presented over it.
    anchor: gtk::Widget,
    toasts: adw::ToastOverlay,
    title: adw::WindowTitle,
    back: gtk::Button,
    add_catalog: gtk::Button,
    search_bar: gtk::SearchBar,
    search_entry: gtk::SearchEntry,
    search_toggle: gtk::ToggleButton,
    retry: gtk::Button,
    busy_kind: Cell<Busy>,
    stack: gtk::Stack,
    list: gtk::ListBox,
    status: adw::StatusPage,
    catalog: RefCell<Option<ob::Catalog>>,
    trail: RefCell<Vec<(String, String)>>,
    feed: RefCell<Option<Feed>>,
    root_feed: RefCell<Option<Feed>>,
    entries: RefCell<Vec<Entry>>,
    busy: Cell<bool>,
    /// What the **Undo** of the last removal toast does (test hook).
    last_undo: RefCell<Option<Rc<dyn Fn()>>>,
    /// Bumped by every navigation so that a late answer of an older one is dropped.
    epoch: Cell<u64>,
}

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

fn short(s: &str, n: usize) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= n {
        one
    } else {
        format!("{}…", one.chars().take(n).collect::<String>())
    }
}

fn poll<T: 'static>(rx: std::sync::mpsc::Receiver<T>, done: impl FnOnce(T) + 'static) {
    let done = RefCell::new(Some(done));
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        match rx.try_recv() {
            Ok(v) => {
                if let Some(f) = done.borrow_mut().take() {
                    f(v);
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

/// The page for the library's content stack. The catalogue list shows first; the page keeps its
/// place (catalogue, folder, search) while the user is somewhere else in the library.
pub fn page(shared: &SharedRef, import: Import) -> gtk::Widget {
    let ui = build(shared, import);
    ui.show_catalogs();
    let widget = ui.anchor.clone();
    CURRENT.with(|c| *c.borrow_mut() = Some(ui));
    widget
}

fn build(shared: &SharedRef, import: Import) -> Rc<Ui> {
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    header.add_css_class("toolbar");
    let title = adw::WindowTitle::new(&tr("Online Catalogues"), "");
    title.set_hexpand(true);
    title.set_halign(gtk::Align::Center);
    let back = gtk::Button::from_icon_name("go-previous-symbolic");
    back.set_tooltip_text(Some(&tr("Back")));
    back.update_property(&[gtk::accessible::Property::Label(&tr("Back"))]);
    header.append(&back);
    let add_catalog = gtk::Button::from_icon_name("list-add-symbolic");
    add_catalog.set_tooltip_text(Some(&tr("Add a catalogue")));
    add_catalog.update_property(&[gtk::accessible::Property::Label(&tr("Add a catalogue"))]);
    let search_toggle = gtk::ToggleButton::new();
    search_toggle.set_icon_name("system-search-symbolic");
    search_toggle.set_tooltip_text(Some(&tr("Search This Catalogue")));
    search_toggle.update_property(&[gtk::accessible::Property::Label(&tr(
        "Search This Catalogue",
    ))]);
    search_toggle.set_visible(false);
    header.append(&title);
    header.append(&search_toggle);
    header.append(&add_catalog);

    let search_entry = gtk::SearchEntry::new();
    search_entry.set_placeholder_text(Some(&tr("Search this catalogue")));
    search_entry.set_hexpand(true);
    search_entry.update_property(&[gtk::accessible::Property::Label(&tr(
        "Search this catalogue",
    ))]);
    let search_bar = gtk::SearchBar::new();
    search_bar.set_child(Some(&search_entry));
    search_bar.connect_entry(&search_entry);
    search_bar.set_show_close_button(false);
    search_toggle
        .bind_property("active", &search_bar, "search-mode-enabled")
        .bidirectional()
        .sync_create()
        .build();

    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    list.set_margin_start(12);
    list.set_margin_end(12);
    list.set_margin_bottom(12);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(720);
    clamp.set_child(Some(&list));
    scroll.set_child(Some(&clamp));
    let status = adw::StatusPage::new();
    let retry = gtk::Button::with_label(&tr("Try Again"));
    retry.add_css_class("suggested-action");
    retry.add_css_class("pill");
    retry.set_halign(gtk::Align::Center);
    retry.set_visible(false);
    status.set_child(Some(&retry));
    let spinner = gtk::Spinner::new();
    spinner.start();
    spinner.set_halign(gtk::Align::Center);
    spinner.set_size_request(32, 32);
    let cancel = gtk::Button::with_label(&tr("Cancel"));
    cancel.add_css_class("pill");
    cancel.set_halign(gtk::Align::Center);
    let busy_box = gtk::Box::new(gtk::Orientation::Vertical, 18);
    busy_box.set_halign(gtk::Align::Center);
    busy_box.append(&spinner);
    busy_box.append(&cancel);
    let busy_page = adw::StatusPage::new();
    busy_page.set_title(&tr("Loading"));
    busy_page.set_child(Some(&busy_box));
    let stack = gtk::Stack::new();
    stack.add_named(&scroll, Some("list"));
    stack.add_named(&status, Some("status"));
    stack.add_named(&busy_page, Some("busy"));

    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&search_bar);
    body.append(&stack);
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&body));
    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.set_widget_name("catalogues-page");
    page.append(&header);
    toasts.set_vexpand(true);
    page.append(&toasts);

    let ui = Rc::new(Ui {
        shared: shared.clone(),
        import,
        anchor: page.clone().upcast(),
        toasts,
        title,
        back: back.clone(),
        add_catalog: add_catalog.clone(),
        search_bar: search_bar.clone(),
        search_entry: search_entry.clone(),
        search_toggle: search_toggle.clone(),
        retry: retry.clone(),
        busy_kind: Cell::new(Busy::Idle),
        stack,
        list,
        status,
        catalog: RefCell::new(None),
        trail: RefCell::new(Vec::new()),
        feed: RefCell::new(None),
        root_feed: RefCell::new(None),
        entries: RefCell::new(Vec::new()),
        busy: Cell::new(false),
        last_undo: RefCell::new(None),
        epoch: Cell::new(0),
    });
    {
        let ui = ui.clone();
        back.connect_clicked(move |_| ui.go_back());
    }
    {
        let ui = ui.clone();
        add_catalog.connect_clicked(move |_| ui.add_catalog_dialog());
    }
    {
        let ui = ui.clone();
        search_entry.connect_activate(move |e| ui.search(e.text().as_str()));
    }
    {
        let ui = ui.clone();
        cancel.connect_clicked(move |_| ui.cancel_load());
    }
    {
        let ui = ui.clone();
        retry.connect_clicked(move |_| ui.load_top(false));
    }
    {
        // Ctrl+F opens the search of the catalogue being browsed
        let (toggle, entry) = (search_toggle.clone(), search_entry.clone());
        let sc = gtk::ShortcutController::new();
        sc.set_scope(gtk::ShortcutScope::Local);
        sc.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string("<Control>f"),
            Some(gtk::CallbackAction::new(move |_, _| {
                if toggle.is_visible() {
                    toggle.set_active(true);
                    entry.grab_focus();
                }
                glib::Propagation::Stop
            })),
        ));
        page.add_controller(sc);
    }
    ui
}

impl Ui {
    fn toast(&self, msg: &str) {
        self.toasts.add_toast(adw::Toast::new(msg));
    }

    fn clear(&self) {
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
    }

    fn message(&self, icon: &str, title: &str, text: &str, retry: bool) {
        self.retry.set_visible(retry);
        self.status.set_icon_name(Some(icon));
        self.status.set_title(title);
        self.status.set_description(Some(text));
        self.stack.set_visible_child_name("status");
    }

    fn catalogs(&self) -> Vec<(ob::Catalog, bool)> {
        let mut v: Vec<(ob::Catalog, bool)> =
            ob::presets().into_iter().map(|c| (c, false)).collect();
        v.extend(
            self.shared
                .cfg
                .borrow()
                .opds_catalogs
                .iter()
                .cloned()
                .map(|c| (c, true)),
        );
        v
    }

    fn show_catalogs(self: &Rc<Self>) {
        self.epoch.set(self.epoch.get() + 1);
        self.busy.set(false);
        *self.catalog.borrow_mut() = None;
        self.trail.borrow_mut().clear();
        *self.feed.borrow_mut() = None;
        *self.root_feed.borrow_mut() = None;
        self.entries.borrow_mut().clear();
        self.title.set_title(&tr("Online Catalogues"));
        self.title.set_subtitle("");
        self.back.set_visible(false);
        self.add_catalog.set_visible(true);
        self.search_toggle.set_active(false);
        self.search_toggle.set_visible(false);
        self.clear();
        for (c, removable) in self.catalogs() {
            let row = adw::ActionRow::new();
            row.set_title(&esc(&c.name));
            row.set_subtitle(&esc(&c.url));
            row.set_activatable(true);
            row.add_prefix(&gtk::Image::from_icon_name("network-server-symbolic"));
            if removable {
                let del = gtk::Button::from_icon_name("user-trash-symbolic");
                del.set_valign(gtk::Align::Center);
                del.add_css_class("flat");
                del.set_tooltip_text(Some(&tr("Remove this catalogue")));
                del.update_property(&[gtk::accessible::Property::Label(&trf(
                    "Remove {}",
                    &[&c.name],
                ))]);
                let (ui, id) = (self.clone(), c.id.clone());
                del.connect_clicked(move |_| ui.remove_catalog(&id));
                row.add_suffix(&del);
            }
            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            let ui = self.clone();
            row.connect_activated(move |_| ui.open_catalog(c.clone()));
            self.list.append(&row);
        }
        self.stack.set_visible_child_name("list");
    }

    /// Removes a catalogue of the user; a toast with **Undo** puts it back where it was.
    fn remove_catalog(self: &Rc<Self>, id: &str) {
        let removed = {
            let mut c = self.shared.cfg.borrow_mut();
            let at = c.opds_catalogs.iter().position(|x| x.id == id);
            let gone = at.map(|i| (i, c.opds_catalogs.remove(i)));
            c.save();
            gone
        };
        self.show_catalogs();
        if let Some((at, cat)) = removed {
            let ui = self.clone();
            let name = cat.name.clone();
            let undo: Rc<dyn Fn()> = Rc::new(move || {
                {
                    let mut c = ui.shared.cfg.borrow_mut();
                    let at = at.min(c.opds_catalogs.len());
                    c.opds_catalogs.insert(at, cat.clone());
                    c.save();
                }
                ui.show_catalogs();
            });
            *self.last_undo.borrow_mut() = Some(undo.clone());
            super::dialog_toast::show_action(
                &self.toasts,
                &trf("Removed {}", &[&name]),
                &tr("Undo"),
                move || undo(),
            );
        }
    }

    fn open_catalog(self: &Rc<Self>, c: ob::Catalog) {
        *self.catalog.borrow_mut() = Some(c.clone());
        *self.trail.borrow_mut() = vec![(c.name.clone(), c.url.clone())];
        *self.root_feed.borrow_mut() = None;
        self.back.set_visible(true);
        self.add_catalog.set_visible(false);
        self.search_entry.set_text("");
        self.search_toggle.set_visible(true);
        self.search_toggle.set_active(true);
        self.load_top(false);
    }

    fn go_back(self: &Rc<Self>) {
        let deeper = self.trail.borrow().len() > 1;
        if deeper {
            self.trail.borrow_mut().pop();
            self.load_top(false);
        } else {
            self.show_catalogs();
        }
    }

    fn navigate(self: &Rc<Self>, title: &str, url: &str) {
        self.trail
            .borrow_mut()
            .push((title.to_string(), url.to_string()));
        self.load_top(false);
    }

    fn search(self: &Rc<Self>, terms: &str) {
        let Some(c) = self.catalog.borrow().clone() else {
            return;
        };
        let feed = self
            .feed
            .borrow()
            .clone()
            .filter(|f| f.search().is_some())
            .or_else(|| self.root_feed.borrow().clone());
        let Some(feed) = feed else {
            return;
        };
        let terms = terms.to_string();
        let epoch = self.epoch.get() + 1;
        self.epoch.set(epoch);
        self.set_busy(Busy::Search);
        let (tx, rx) = std::sync::mpsc::channel();
        let t2 = terms.clone();
        std::thread::spawn(move || {
            let f = ob::fetcher();
            let _ = tx.send(ob::search_url(&*f, &c, &feed, &t2).map_err(|e| format!("{e:#}")));
        });
        let ui = self.clone();
        poll(rx, move |res| {
            if ui.epoch.get() != epoch {
                return;
            }
            match res {
                Ok(url) => ui.navigate(&trf("Search: {}", &[&terms]), &url),
                Err(e) => {
                    ui.busy.set(false);
                    ui.stack.set_visible_child_name("list");
                    ui.toast(&e);
                }
            }
        });
    }

    fn set_busy(&self, kind: Busy) {
        self.busy.set(true);
        self.busy_kind.set(kind);
        self.stack.set_visible_child_name("busy");
    }

    /// The Cancel button of the loading page: drops the running request (its answer is ignored)
    /// and returns to where the user came from.
    fn cancel_load(self: &Rc<Self>) {
        self.epoch.set(self.epoch.get() + 1);
        self.busy.set(false);
        match self.busy_kind.replace(Busy::Idle) {
            Busy::Search => self.stack.set_visible_child_name("list"),
            Busy::Page => self.go_back(),
            Busy::Idle => {}
        }
    }

    /// Loads the page at the top of the trail.
    fn load_top(self: &Rc<Self>, append: bool) {
        let Some(c) = self.catalog.borrow().clone() else {
            return;
        };
        let (title, url) = match self.trail.borrow().last().cloned() {
            Some(t) => t,
            None => return,
        };
        self.fetch(c, title, url, append);
    }

    fn fetch(self: &Rc<Self>, c: ob::Catalog, title: String, url: String, append: bool) {
        let epoch = self.epoch.get() + 1;
        self.epoch.set(epoch);
        if !append {
            self.set_busy(Busy::Page);
        }
        self.title.set_title(&title);
        self.title.set_subtitle(&c.name);
        let (tx, rx) = std::sync::mpsc::channel();
        let (c2, url2) = (c.clone(), url.clone());
        std::thread::spawn(move || {
            let f = ob::fetcher();
            let _ = tx.send(ob::load(&*f, &c2, &url2).map_err(|e| format!("{e:#}")));
        });
        let ui = self.clone();
        poll(rx, move |res| {
            if ui.epoch.get() != epoch {
                return;
            }
            ui.busy.set(false);
            match res {
                Ok(feed) => ui.render(feed, append),
                Err(e) => {
                    if append {
                        ui.stack.set_visible_child_name("list");
                        ui.toast(&e);
                    } else {
                        ui.message(
                            "dialog-error-symbolic",
                            &tr("Could not Load This Page"),
                            &e,
                            true,
                        );
                    }
                }
            }
        });
    }

    fn render(self: &Rc<Self>, feed: Feed, append: bool) {
        if !append {
            self.entries.borrow_mut().clear();
            self.clear();
        } else if let Some(last) = self.list.last_child() {
            // the "Load more" row
            self.list.remove(&last);
        }
        if self.root_feed.borrow().is_none() {
            *self.root_feed.borrow_mut() = Some(feed.clone());
        }
        let c = self.catalog.borrow().clone().unwrap_or_default();
        for e in &feed.entries {
            let idx = self.entries.borrow().len();
            self.entries.borrow_mut().push(e.clone());
            let row = adw::ActionRow::new();
            row.set_title(&esc(&e.title));
            if e.is_acquisition() {
                row.set_subtitle(&esc(&ob::describe(e)));
                if !e.summary.is_empty() {
                    row.set_tooltip_text(Some(&short(&e.summary, 400)));
                }
                row.add_prefix(&gtk::Image::from_icon_name("emblem-documents-symbolic"));
                let btn = gtk::Button::with_label(&tr("Add"));
                btn.set_valign(gtk::Align::Center);
                btn.add_css_class("suggested-action");
                btn.update_property(&[gtk::accessible::Property::Label(&trf(
                    "Add {} to the library",
                    &[&e.title],
                ))]);
                let (ui, c) = (self.clone(), c.clone());
                btn.connect_clicked(move |b| ui.download(&c, idx, b));
                row.add_suffix(&btn);
            } else if let Some(link) = e.navigation_link() {
                row.set_subtitle(&esc(&short(&e.summary, 120)));
                row.set_activatable(true);
                row.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));
                row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
                let (ui, t, u) = (self.clone(), e.title.clone(), link.href.clone());
                row.connect_activated(move |_| ui.navigate(&t, &u));
            } else {
                continue;
            }
            self.list.append(&row);
        }
        if let Some(next) = feed.next() {
            let row = adw::ActionRow::new();
            row.set_title(&tr("Load more"));
            row.set_activatable(true);
            row.add_prefix(&gtk::Image::from_icon_name("view-more-symbolic"));
            let (ui, c, u) = (self.clone(), c.clone(), next.to_string());
            row.connect_activated(move |_| {
                let t = ui
                    .trail
                    .borrow()
                    .last()
                    .map(|t| t.0.clone())
                    .unwrap_or_default();
                ui.fetch(c.clone(), t, u.clone(), true);
            });
            self.list.append(&row);
        }
        *self.feed.borrow_mut() = Some(feed);
        if self.entries.borrow().is_empty() {
            self.message(
                "edit-find-symbolic",
                &tr("Nothing Here"),
                &tr("This page of the catalogue is empty."),
                false,
            );
        } else {
            self.stack.set_visible_child_name("list");
        }
    }

    fn download(self: &Rc<Self>, c: &ob::Catalog, idx: usize, button: &gtk::Button) {
        let Some(entry) = self.entries.borrow().get(idx).cloned() else {
            return;
        };
        button.set_sensitive(false);
        button.set_label(&tr("Adding"));
        let dir = directories::ProjectDirs::from("com", "adhadse", "Bookx")
            .map(|d| d.cache_dir().join("opds"))
            .unwrap_or_else(|| std::env::temp_dir().join("bookx-opds"));
        let (tx, rx) = std::sync::mpsc::channel();
        let c = c.clone();
        let e2 = entry.clone();
        std::thread::spawn(move || {
            let f = ob::fetcher();
            let _ = tx.send(ob::download(&*f, &c, &e2, &dir).map_err(|e| format!("{e:#}")));
        });
        let (ui, button) = (self.clone(), button.clone());
        poll(rx, move |res| match res {
            Ok(path) => {
                button.set_label(&tr("Added"));
                (ui.import)(vec![path]);
            }
            Err(e) => {
                button.set_sensitive(true);
                button.set_label(&tr("Add"));
                ui.toast(&e);
            }
        });
    }

    fn add_catalog_dialog(self: &Rc<Self>) {
        let d = adw::AlertDialog::new(
            Some(&tr("Add a Catalogue")),
            Some(&tr("The address of an OPDS catalogue, such as a Calibre server (http://host:8080/opds) or another Bookx. A user name and password are only needed when the server asks for them.")),
        );
        let group = adw::PreferencesGroup::new();
        let mk = |title: &str| {
            let r = adw::EntryRow::new();
            r.set_title(title);
            r
        };
        let name = mk(&tr("Name"));
        let url = mk(&tr("Address"));
        let user = mk(&tr("User Name"));
        let pass = adw::PasswordEntryRow::new();
        pass.set_title(&tr("Password"));
        for r in [&name, &url, &user] {
            group.add(r);
        }
        group.add(&pass);
        let warning = gtk::Label::new(Some(&tr(
            "This address is not encrypted, so the password would be sent in the clear",
        )));
        warning.set_wrap(true);
        warning.set_xalign(0.0);
        warning.add_css_class("warning");
        warning.set_visible(false);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        body.append(&group);
        body.append(&warning);
        d.set_extra_child(Some(&body));
        d.add_response("cancel", &tr("Cancel"));
        d.add_response("add", &tr("Add"));
        d.set_response_appearance("add", adw::ResponseAppearance::Suggested);
        d.set_default_response(Some("add"));
        d.set_close_response("cancel");
        d.set_response_enabled("add", can_add(&url.text()));
        for r in [&name, &url, &user] {
            r.set_activates_default(true);
        }
        pass.set_activates_default(true);
        let refresh = {
            let (d, url, pass, warning) = (d.clone(), url.clone(), pass.clone(), warning.clone());
            move || {
                d.set_response_enabled("add", can_add(&url.text()));
                warning.set_visible(cleartext_credentials(&url.text(), &pass.text()));
            }
        };
        {
            let refresh = refresh.clone();
            url.connect_changed(move |_| refresh());
        }
        pass.connect_changed(move |_| refresh());
        ADD_DIALOG.with(|c| {
            *c.borrow_mut() = Some(AddDialog {
                dialog: d.downgrade(),
                name: name.clone(),
                url: url.clone(),
                user: user.clone(),
                pass: pass.clone(),
                warning: warning.clone(),
            })
        });
        let ui = self.clone();
        d.connect_response(None, move |_, r| {
            if r != "add" {
                return;
            }
            ui.add_catalog_from(
                name.text().as_str(),
                url.text().as_str(),
                user.text().as_str(),
                pass.text().as_str(),
            );
        });
        d.present(Some(&self.anchor));
    }

    fn add_catalog_from(self: &Rc<Self>, name: &str, url: &str, user: &str, pass: &str) -> bool {
        let res = {
            let mut c = self.shared.cfg.borrow_mut();
            let r = ob::add(&mut c.opds_catalogs, name, url, user, pass);
            if r.is_ok() {
                c.save();
            }
            r
        };
        match res {
            Ok(_) => {
                self.show_catalogs();
                true
            }
            Err(e) => {
                self.toast(&format!("{e:#}"));
                false
            }
        }
    }

    fn dump(&self) {
        let rows: Vec<String> = self
            .entries
            .borrow()
            .iter()
            .map(|e| {
                format!(
                    "{}|{}",
                    e.title,
                    if e.is_acquisition() { "book" } else { "folder" }
                )
            })
            .collect();
        eprintln!(
            "opds_view: title={} | page={} | busy={} | back={} | search={} | search_open={} | retry={} | rows={}",
            self.title.title(),
            self.stack.visible_child_name().unwrap_or_default(),
            self.busy.get(),
            self.back.is_visible(),
            self.search_toggle.is_visible(),
            self.search_bar.is_search_mode(),
            self.retry.is_visible(),
            rows.join(";")
        );
        if self.catalog.borrow().is_none() {
            let names: Vec<String> = self.catalogs().into_iter().map(|(c, _)| c.name).collect();
            eprintln!("opds_catalogs: {}", names.join(";"));
        }
    }
}

/// Test hook (`library:opds=…`): `open`, `dump`, `catalog=NAME`, `go=TITLE`, `back`,
/// `search=TERMS`, `cancel` (the loading page's Cancel), `add=TITLE`, `add-catalog=NAME|URL|USER|PASSWORD`, `more`.
/// The *Add a Catalogue* button needs an address.
pub fn can_add(address: &str) -> bool {
    !address.trim().is_empty()
}

/// A password typed for an `http://` address would cross the network in the clear.
pub fn cleartext_credentials(address: &str, password: &str) -> bool {
    !password.is_empty()
        && address
            .trim_start()
            .get(..7)
            .is_some_and(|p| p.eq_ignore_ascii_case("http://"))
}

/// The fields of the open *Add a Catalogue* dialog (test hook).
struct AddDialog {
    dialog: glib::WeakRef<adw::AlertDialog>,
    name: adw::EntryRow,
    url: adw::EntryRow,
    user: adw::EntryRow,
    pass: adw::PasswordEntryRow,
    warning: gtk::Label,
}

thread_local! {
    static ADD_DIALOG: RefCell<Option<AddDialog>> = const { RefCell::new(None) };
}

/// `opds:add-dialog` opens the dialog, `opds:add-fill=NAME|ADDRESS|USER|PASSWORD` types into it,
/// `opds:add-state` prints `opds_add: add_enabled=BOOL warning=BOOL`.
fn dev_add_dialog(ui: &Rc<Ui>, cmd: &str) -> bool {
    if cmd == "add-dialog" {
        ui.add_catalog_dialog();
    } else if let Some(v) = cmd.strip_prefix("add-fill=") {
        let mut p = v.splitn(4, '|');
        ADD_DIALOG.with(|c| {
            if let Some(a) = c.borrow().as_ref() {
                a.name.set_text(p.next().unwrap_or(""));
                a.url.set_text(p.next().unwrap_or(""));
                a.user.set_text(p.next().unwrap_or(""));
                a.pass.set_text(p.next().unwrap_or(""));
            }
        });
    } else if cmd != "add-state" {
        return false;
    }
    ADD_DIALOG.with(|c| match c.borrow().as_ref() {
        Some(a) => eprintln!(
            "opds_add: add_enabled={} warning={}",
            a.dialog
                .upgrade()
                .is_some_and(|d| d.is_response_enabled("add")),
            a.warning.is_visible()
        ),
        None => eprintln!("opds_add: closed"),
    });
    true
}

pub fn dev(cmd: &str) {
    let Some(ui) = CURRENT.with(|c| c.borrow().clone()) else {
        eprintln!("opds_error: not open");
        return;
    };
    let by_title =
        |t: &str| -> Option<usize> { ui.entries.borrow().iter().position(|e| e.title == t) };
    if dev_add_dialog(&ui, cmd) {
        return;
    }
    match cmd {
        "dump" => ui.dump(),
        "catalogs" => {
            let names: Vec<String> = ui
                .shared
                .cfg
                .borrow()
                .opds_catalogs
                .iter()
                .map(|c| c.name.clone())
                .collect();
            eprintln!("opds_catalogs: {}", names.join("|"));
        }
        "undo-remove" => {
            let undo = ui.last_undo.borrow_mut().take();
            match undo {
                Some(u) => u(),
                None => eprintln!("opds_error: nothing to undo"),
            }
        }
        c if c.starts_with("remove=") => {
            let id = ui
                .shared
                .cfg
                .borrow()
                .opds_catalogs
                .iter()
                .find(|x| x.name == c[7..])
                .map(|x| x.id.clone());
            match id {
                Some(id) => ui.remove_catalog(&id),
                None => eprintln!("opds_error: no catalogue {}", &c[7..]),
            }
        }
        "back" => ui.go_back(),
        "cancel" => ui.cancel_load(),
        "more" => {
            if let (Some(c), Some(next)) = (
                ui.catalog.borrow().clone(),
                ui.feed
                    .borrow()
                    .as_ref()
                    .and_then(|f| f.next().map(String::from)),
            ) {
                let t = ui
                    .trail
                    .borrow()
                    .last()
                    .map(|t| t.0.clone())
                    .unwrap_or_default();
                ui.fetch(c, t, next, true);
            }
        }
        c if c.starts_with("catalog=") => {
            match ui.catalogs().into_iter().find(|(x, _)| x.name == c[8..]) {
                Some((cat, _)) => ui.open_catalog(cat),
                None => eprintln!("opds_error: no catalogue {}", &c[8..]),
            }
        }
        c if c.starts_with("go=") => {
            let found = by_title(&c[3..]).and_then(|i| {
                let e = ui.entries.borrow()[i].clone();
                e.navigation_link()
                    .map(|l| (e.title.clone(), l.href.clone()))
            });
            match found {
                Some((t, u)) => ui.navigate(&t, &u),
                None => eprintln!("opds_error: no folder {}", &c[3..]),
            }
        }
        c if c.starts_with("search=") => ui.search(&c[7..]),
        c if c.starts_with("add=") => match by_title(&c[4..]) {
            Some(i) => {
                let cat = ui.catalog.borrow().clone().unwrap_or_default();
                ui.download(&cat, i, &gtk::Button::new());
            }
            None => eprintln!("opds_error: no book {}", &c[4..]),
        },
        c if c.starts_with("add-catalog=") => {
            let p: Vec<&str> = c[12..].split('|').collect();
            let get = |i: usize| p.get(i).copied().unwrap_or("");
            if !ui.add_catalog_from(get(0), get(1), get(2), get(3)) {
                eprintln!("opds_error: not added");
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_needs_an_address() {
        assert!(!can_add(""));
        assert!(!can_add("  "));
        assert!(can_add("https://x/opds"));
    }

    #[test]
    fn a_password_for_plain_http_is_flagged() {
        assert!(cleartext_credentials("http://host:8080/opds", "pw"));
        assert!(cleartext_credentials(" HTTP://host/opds", "pw"));
        assert!(!cleartext_credentials("https://host/opds", "pw"));
        assert!(!cleartext_credentials("http://host/opds", ""));
        assert!(!cleartext_credentials("héé", "pw"));
    }
}
