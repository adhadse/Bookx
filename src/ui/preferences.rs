//! Preferences dialog.

use crate::i18n::ntr;
use crate::i18n::{tr, trf, N_};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio, glib};

use super::{window_of, SharedRef};
use crate::config::{ReaderTheme, Scheme};
use crate::ui::canvas::LayoutMode;

#[derive(Debug, Clone)]
pub enum PrefChange {
    LibraryDir(PathBuf),
    /// The library layout may have changed and files moved; the text says what happened.
    Layout(String),
    Scheme(Scheme),
    ReaderDefaults,
    /// The Continue Reading banner mode changed.
    Banner,
}

pub fn show(parent: &impl IsA<gtk::Widget>, shared: &SharedRef, on_change: Rc<dyn Fn(PrefChange)>) {
    show_at(parent, shared, on_change, None);
}

// ---- registry ---------------------------------------------------------------------------------
//
// Preferences is a sidebar of sections, each one a page. The built-in sections are made in
// `build_sections`; any other module adds a section or rows to one with `register_page` and
// `register_group` (see `external_registrations` and `_developer/docs.md`).

/// What a group builder gets: the window to parent dialogs on, the shared state, and the callback
/// that tells the application about a changed setting.
#[allow(dead_code)] // read by the group builders other modules register
pub struct PrefCtx {
    pub parent: gtk::Widget,
    pub shared: SharedRef,
    pub on_change: Rc<dyn Fn(PrefChange)>,
}

type GroupBuilder = Rc<dyn Fn(&PrefCtx) -> adw::PreferencesGroup>;

struct PageSpec {
    id: String,
    title: String,
    icon: String,
    order: i32,
}

#[derive(Default)]
struct Registry {
    pages: Vec<PageSpec>,
    groups: Vec<(String, i32, GroupBuilder)>,
    init: bool,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::default();
    static CURRENT: RefCell<Option<Rc<View>>> = const { RefCell::new(None) };
}

/// Ids and sort order of the built-in sections (sidebar order: lower first).
pub const BUILTIN: [(&str, &str, &str, i32); 8] = [
    ("general", N_("General"), "preferences-system-symbolic", 10),
    ("library", N_("Library"), "library-symbolic", 20),
    ("reading", N_("Reading"), "book-open-variant-symbolic", 30),
    (
        "devices",
        N_("Devices and Sharing"),
        "computer-symbolic",
        40,
    ),
    ("email", N_("E-mail"), "mail-send-symbolic", 50),
    ("sync", N_("Sync"), "view-refresh-symbolic", 60),
    (
        "extensions",
        N_("Extensions"),
        "application-x-addon-symbolic",
        80,
    ),
    ("patches", N_("Patches"), "edit-find-replace-symbolic", 90),
];

#[allow(dead_code)] // called by the modules that add a section
/// Adds a section to the sidebar (nothing happens if the id exists). Suggested orders: Backup 70,
/// Text to Speech 100, Advanced 110.
pub fn register_page(id: &str, title: &str, icon: &str, order: i32) {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        if !r.pages.iter().any(|p| p.id == id) {
            r.pages.push(PageSpec {
                id: id.into(),
                title: title.into(),
                icon: icon.into(),
                order,
            });
        }
    });
}

#[allow(dead_code)] // called by the modules that add rows
/// Adds a group of rows to the section `page_id` (a built-in or a `register_page` one). `order`
/// sorts it among the other registered groups of that section; they come after the built-in ones.
pub fn register_group(
    page_id: &str,
    order: i32,
    build: impl Fn(&PrefCtx) -> adw::PreferencesGroup + 'static,
) {
    REGISTRY.with(|r| {
        r.borrow_mut()
            .groups
            .push((page_id.into(), order, Rc::new(build)))
    });
}

/// The one place where other modules hook in. Add a line per module, for example
/// `register_page("backup", &tr("Backup"), "document-save-symbolic", 70);` and a `register_group`.
fn external_registrations() {
    super::tts_settings::register();
    super::metadata_settings::register();
    super::language_settings::register();
    super::reader_extra::register();
    super::reader_footer::register();
    super::shortcuts_dialog::register();
    super::mcp_settings::register();
}

fn ensure_registry() {
    let first = REGISTRY.with(|r| !std::mem::replace(&mut r.borrow_mut().init, true));
    if first {
        external_registrations();
    }
}

/// One section of Preferences: its place in the sidebar and its page.
pub struct Section {
    pub id: String,
    pub title: String,
    pub icon: String,
    pub order: i32,
    pub page: adw::PreferencesPage,
}

/// Do all words of `query` occur (ignoring case) in `text`? An empty query matches everything.
pub fn query_matches(query: &str, text: &str) -> bool {
    let text = text.to_lowercase();
    query
        .split_whitespace()
        .all(|w| text.contains(&w.to_lowercase()))
}

fn new_page(title: &str, icon: &str) -> adw::PreferencesPage {
    let p = adw::PreferencesPage::new();
    p.set_title(title);
    p.set_icon_name(Some(icon));
    p
}

/// Opens Preferences; with `focus` on the page and row of that title (the command palette's settings).
pub fn show_at(
    parent: &impl IsA<gtk::Widget>,
    shared: &SharedRef,
    on_change: Rc<dyn Fn(PrefChange)>,
    focus: Option<&str>,
) {
    let sections = build_sections(parent, shared, on_change);
    let (dialog, view) = assemble(sections);
    CURRENT.with(|c| *c.borrow_mut() = Some(view.clone()));
    dialog.connect_closed(|_| CURRENT.with(|c| *c.borrow_mut() = None));
    dialog.present(Some(parent));
    if let Some(title) = focus {
        for (i, s) in view.sections.iter().enumerate() {
            if let Some(row) = find_row(s.page.upcast_ref(), title) {
                view.select(i);
                let (page_title, want) = (s.title.clone(), title.to_string());
                glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
                    row.grab_focus();
                    if !crate::devtools::script().is_empty() {
                        let focused = row.root().and_then(|r| r.focus()).is_some_and(|f| {
                            f == row || f.is_ancestor(&row) || row.is_ancestor(&f)
                        });
                        eprintln!("prefs_focus: page={page_title} row={want} focused={focused}");
                    }
                });
                break;
            }
        }
    }
}

/// Dev hook `prefs:ARG` (besides the plain `prefs:` that opens the dialog): `search=TEXT`,
/// `pick=SECTION`, `dump`.
pub fn dev(arg: &str) {
    let Some(view) = CURRENT.with(|c| c.borrow().clone()) else {
        eprintln!("prefs: not open");
        return;
    };
    if let Some(f) = arg.strip_prefix("footer-") {
        super::reader_footer::dev(f);
    } else if let Some(q) = arg.strip_prefix("search=") {
        view.search.set_text(q);
    } else if let Some(name) = arg.strip_prefix("pick=") {
        if let Some(i) = view.sections.iter().position(|s| s.title == name) {
            view.select(i);
        }
    } else if arg == "icons" {
        // every section of the sidebar has an icon, and it exists in the icon theme
        let theme = gtk::IconTheme::for_display(&gtk::gdk::Display::default().unwrap());
        let missing: Vec<String> = view
            .sections
            .iter()
            .filter(|s| s.icon.is_empty() || !theme.has_icon(&s.icon))
            .map(|s| format!("{} ({})", s.title, s.icon))
            .collect();
        eprintln!(
            "prefs_icons: checked={} sections={} missing={:?}",
            view.sections.len(),
            view.sections
                .iter()
                .map(|s| s.title.as_str())
                .collect::<Vec<_>>()
                .join("|"),
            missing
        );
    } else if arg == "dump" {
        let shown: Vec<String> = view
            .sections
            .iter()
            .zip(&view.rows)
            .filter(|(_, r)| r.is_visible())
            .map(|(s, _)| s.title.clone())
            .collect();
        let sel = view
            .list
            .selected_row()
            .map(|r| view.sections[r.index() as usize].title.clone())
            .unwrap_or_default();
        let hits = view.hits.borrow().len();
        eprintln!(
            "prefs: query={:?} sections={} selected={sel} hits={hits} collapsed={} empty={}",
            view.search.text(),
            shown.join("|"),
            view.split.is_collapsed(),
            view.stack.visible_child_name().as_deref() == Some("empty")
        );
    }
}

/// The widgets of the open dialog.
struct View {
    sections: Vec<Section>,
    rows: Vec<gtk::ListBoxRow>,
    list: gtk::ListBox,
    stack: gtk::Stack,
    search: gtk::SearchEntry,
    split: adw::OverlaySplitView,
    title: adw::WindowTitle,
    hidden: RefCell<Vec<gtk::Widget>>,
    hits: RefCell<Vec<gtk::Widget>>,
}

impl View {
    fn select(&self, i: usize) {
        if let Some(r) = self.rows.get(i) {
            self.list.select_row(Some(r));
        }
    }

    /// Filters the rows of every section by `query` (words, ignoring case); sections without a
    /// match leave the sidebar, matching rows are highlighted. A match in a section's own title
    /// or in a group's title keeps the whole section or group.
    fn filter(&self, query: &str) {
        for w in self.hidden.borrow_mut().drain(..) {
            w.set_visible(true);
        }
        for w in self.hits.borrow_mut().drain(..) {
            w.remove_css_class("pref-hit");
        }
        let q = query.trim();
        let mut first_visible = None;
        for (i, (s, row)) in self.sections.iter().zip(&self.rows).enumerate() {
            let visible =
                q.is_empty() || query_matches(q, &s.title) || self.filter_page(&s.page, q) > 0;
            row.set_visible(visible);
            if visible && first_visible.is_none() {
                first_visible = Some(i);
            }
        }
        let current_ok = self.list.selected_row().is_some_and(|r| r.is_visible());
        match first_visible {
            None => self.stack.set_visible_child_name("empty"),
            Some(i) => {
                if !current_ok {
                    self.select(i);
                } else if let Some(r) = self.list.selected_row() {
                    self.show_section(r.index() as usize);
                }
            }
        }
    }

    /// Hides what does not match in `page`; returns how many rows match.
    fn filter_page(&self, page: &adw::PreferencesPage, q: &str) -> usize {
        let mut groups = Vec::new();
        collect_groups(page.upcast_ref(), &mut groups);
        let mut total = 0;
        for g in groups {
            let head = format!("{} {}", g.title(), g.description().unwrap_or_default());
            let mut rows = Vec::new();
            top_rows(g.upcast_ref(), &mut rows);
            if query_matches(q, &head) {
                total += rows.len().max(1);
                continue;
            }
            let mut any = false;
            for r in rows {
                if query_matches(q, &row_text(&r)) {
                    r.add_css_class("pref-hit");
                    self.hits.borrow_mut().push(r.clone().upcast());
                    any = true;
                    total += 1;
                } else {
                    r.set_visible(false);
                    self.hidden.borrow_mut().push(r.upcast());
                }
            }
            if !any {
                g.set_visible(false);
                self.hidden.borrow_mut().push(g.upcast());
            }
        }
        total
    }

    fn show_section(&self, i: usize) {
        if let Some(s) = self.sections.get(i) {
            self.stack.set_visible_child_name(&s.id);
            self.title.set_title(&s.title);
        }
    }
}

fn collect_groups(w: &gtk::Widget, out: &mut Vec<adw::PreferencesGroup>) {
    if let Some(g) = w.downcast_ref::<adw::PreferencesGroup>() {
        out.push(g.clone());
        return;
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        collect_groups(&child, out);
        c = child.next_sibling();
    }
}

/// The rows of a group that are not inside another row (an expander keeps its rows).
fn top_rows(w: &gtk::Widget, out: &mut Vec<adw::PreferencesRow>) {
    let mut c = w.first_child();
    while let Some(child) = c {
        if let Some(r) = child.downcast_ref::<adw::PreferencesRow>() {
            out.push(r.clone());
        } else {
            top_rows(&child, out);
        }
        c = child.next_sibling();
    }
}

fn row_text(r: &adw::PreferencesRow) -> String {
    let mut out = String::new();
    fn walk(w: &gtk::Widget, out: &mut String) {
        if let Some(r) = w.downcast_ref::<adw::PreferencesRow>() {
            out.push_str(&r.title());
            out.push(' ');
        }
        if let Some(a) = w.downcast_ref::<adw::ActionRow>() {
            out.push_str(&a.subtitle().unwrap_or_default());
            out.push(' ');
        }
        let mut c = w.first_child();
        while let Some(child) = c {
            walk(&child, out);
            c = child.next_sibling();
        }
    }
    walk(r.upcast_ref(), &mut out);
    out
}

/// The dialog: a sidebar of sections with a search entry on top and the section's page next to
/// it. On a narrow window the sidebar becomes an overlay that a button in the header opens.
fn assemble(sections: Vec<Section>) -> (adw::Dialog, Rc<View>) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&tr("Preferences"));
    dialog.set_content_width(900);
    dialog.set_content_height(680);
    dialog.set_width_request(360);
    dialog.set_height_request(400);

    let split = adw::OverlaySplitView::new();
    split.set_min_sidebar_width(220.0);
    split.set_max_sidebar_width(300.0);
    split.set_sidebar_width_fraction(0.28);

    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some(&tr("Search settings")));
    super::audit::label(&search, &tr("Search settings"));
    search.set_margin_start(8);
    search.set_margin_end(8);
    search.set_margin_bottom(6);

    let list = gtk::ListBox::new();
    list.add_css_class("navigation-sidebar");
    list.set_selection_mode(gtk::SelectionMode::Single);
    super::audit::label(&list, &tr("Settings sections"));
    let mut rows = Vec::new();
    for s in &sections {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        b.append(&gtk::Image::from_icon_name(&s.icon));
        let l = gtk::Label::new(Some(&s.title));
        l.set_xalign(0.0);
        l.set_hexpand(true);
        b.append(&l);
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&b));
        super::audit::label(&row, &s.title);
        list.append(&row);
        rows.push(row);
    }
    let side_scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let side_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    side_box.append(&search);
    side_box.append(&side_scroll);
    let side_head = adw::HeaderBar::new();
    side_head.set_show_end_title_buttons(false);
    side_head.set_title_widget(Some(&adw::WindowTitle::new(&tr("Preferences"), "")));
    let side = adw::ToolbarView::new();
    side.add_top_bar(&side_head);
    side.set_content(Some(&side_box));
    split.set_sidebar(Some(&side));

    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    for s in &sections {
        stack.add_named(&s.page, Some(&s.id));
    }
    let empty = adw::StatusPage::new();
    empty.set_icon_name(Some("edit-find-symbolic"));
    empty.set_title(&tr("No Results"));
    empty.set_description(Some(&tr("No setting matches the search.")));
    stack.add_named(&empty, Some("empty"));

    let title = adw::WindowTitle::new("", "");
    let head = adw::HeaderBar::new();
    head.set_title_widget(Some(&title));
    let toggle = gtk::ToggleButton::new();
    toggle.set_icon_name("sidebar-show-symbolic");
    toggle.set_tooltip_text(Some(&tr("Show sections")));
    super::audit::label(&toggle, &tr("Show sections"));
    split
        .bind_property("show-sidebar", &toggle, "active")
        .bidirectional()
        .sync_create()
        .build();
    split
        .bind_property("collapsed", &toggle, "visible")
        .sync_create()
        .build();
    head.pack_start(&toggle);
    let content = adw::ToolbarView::new();
    content.add_top_bar(&head);
    content.set_content(Some(&stack));
    split.set_content(Some(&content));

    let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        620.0,
        adw::LengthUnit::Sp,
    ));
    bp.add_setter(&split, "collapsed", Some(&true.to_value()));
    dialog.add_breakpoint(bp);
    dialog.set_child(Some(&split));

    let view = Rc::new(View {
        sections,
        rows,
        list: list.clone(),
        stack,
        search: search.clone(),
        split: split.clone(),
        title,
        hidden: RefCell::default(),
        hits: RefCell::default(),
    });
    {
        let v = Rc::downgrade(&view);
        list.connect_row_selected(move |_, row| {
            if let (Some(v), Some(row)) = (v.upgrade(), row) {
                v.show_section(row.index() as usize);
                if v.split.is_collapsed() {
                    v.split.set_show_sidebar(false);
                }
            }
        });
    }
    {
        let v = Rc::downgrade(&view);
        search.connect_search_changed(move |e| {
            if let Some(v) = v.upgrade() {
                v.filter(&e.text());
            }
        });
        let d = dialog.clone();
        search.connect_stop_search(move |e| {
            if e.text().is_empty() {
                d.close();
            } else {
                e.set_text("");
            }
        });
    }
    {
        // Ctrl+F jumps to the search (opening the sidebar when it is an overlay)
        let (s, sp) = (search.clone(), split.clone());
        let sc = gtk::ShortcutController::new();
        sc.set_scope(gtk::ShortcutScope::Managed);
        sc.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string("<Control>f"),
            Some(gtk::CallbackAction::new(move |_, _| {
                sp.set_show_sidebar(true);
                s.grab_focus();
                glib::Propagation::Stop
            })),
        ));
        dialog.add_controller(sc);
    }
    view.select(0);
    (dialog, view)
}

/// Every setting of the dialog for the command palette: page, section, title and hint.
pub fn index(parent: &impl IsA<gtk::Widget>, shared: &SharedRef) -> Vec<crate::palette::Entry> {
    let sections = build_sections(parent, shared, Rc::new(|_| {}));
    let mut out = Vec::new();
    for s in &sections {
        let mut rows = Vec::new();
        collect_rows(s.page.upcast_ref(), "", &mut rows);
        for (section, title, subtitle) in rows {
            out.push(crate::palette::setting_entry(
                &s.title, &section, &title, &subtitle,
            ));
        }
    }
    out
}

fn collect_rows(w: &gtk::Widget, section: &str, out: &mut Vec<(String, String, String)>) {
    let mut section = section.to_string();
    if let Some(g) = w.downcast_ref::<adw::PreferencesGroup>() {
        section = g.title().to_string();
    }
    if let Some(r) = w.downcast_ref::<adw::PreferencesRow>() {
        let title = r.title().to_string();
        if !title.is_empty() {
            let subtitle = if let Some(a) = w.downcast_ref::<adw::ActionRow>() {
                a.subtitle().map(|s| s.to_string()).unwrap_or_default()
            } else {
                String::new()
            };
            out.push((section.clone(), title, subtitle));
        }
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        collect_rows(&child, &section, out);
        c = child.next_sibling();
    }
}

fn find_row(w: &gtk::Widget, title: &str) -> Option<gtk::Widget> {
    if let Some(r) = w.downcast_ref::<adw::PreferencesRow>() {
        if r.title() == title {
            return Some(w.clone());
        }
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        if let Some(r) = find_row(&child, title) {
            return Some(r);
        }
        c = child.next_sibling();
    }
    None
}

fn build_sections(
    parent: &impl IsA<gtk::Widget>,
    shared: &SharedRef,
    on_change: Rc<dyn Fn(PrefChange)>,
) -> Vec<Section> {
    ensure_registry();
    let ctx = PrefCtx {
        parent: parent.clone().upcast(),
        shared: shared.clone(),
        on_change: on_change.clone(),
    };
    let reading = adw::PreferencesPage::new();
    reading.set_title(&tr("Reading"));
    reading.set_icon_name(Some("view-reader-symbolic"));
    let devices_page = adw::PreferencesPage::new();
    devices_page.set_title(&tr("Devices and Sharing"));
    devices_page.set_icon_name(Some("computer-symbolic"));
    let extras = adw::PreferencesPage::new();
    extras.set_title(&tr("Extensions"));
    extras.set_icon_name(Some("application-x-addon-symbolic"));

    let page = adw::PreferencesPage::new();
    page.set_title(&tr("General"));
    page.set_icon_name(Some("preferences-system-symbolic"));
    let library_page = new_page(&tr("Library"), "library-symbolic");
    let email_page = new_page(&tr("E-mail"), "mail-send-symbolic");
    let sync_page = new_page(&tr("Sync"), "view-refresh-symbolic");
    let patches_page = new_page(&tr("Patches"), "edit-find-replace-symbolic");

    // --- library ---------------------------------------------------------
    let lib = adw::PreferencesGroup::builder()
        .title(tr("Library"))
        .description(tr(
            "Books are stored in a Calibre-style folder tree inside this folder",
        ))
        .build();
    let row = adw::ActionRow::builder()
        .title(tr("Library folder"))
        .subtitle(shared.lib.borrow().root.display().to_string())
        .build();
    let change = gtk::Button::with_label(&tr("Change"));
    change.set_valign(gtk::Align::Center);
    row.add_suffix(&change);
    {
        let on_change = on_change.clone();
        let row = row.clone();
        let parent_w = parent.clone().upcast::<gtk::Widget>();
        change.connect_clicked(move |_| {
            let dlg = gtk::FileDialog::builder()
                .title(tr("Choose Library Folder"))
                .build();
            let on_change = on_change.clone();
            let row = row.clone();
            dlg.select_folder(
                window_of(&parent_w).as_ref(),
                gio::Cancellable::NONE,
                move |res| {
                    if let Ok(f) = res {
                        if let Some(p) = f.path() {
                            row.set_subtitle(&p.display().to_string());
                            on_change(PrefChange::LibraryDir(p));
                        }
                    }
                },
            );
        });
    }
    lib.add(&row);
    let banner = adw::ComboRow::builder()
        .title(tr("Continue reading banner"))
        .subtitle(tr(
            "Adaptive: a slim card on small windows, a large centred card on big ones",
        ))
        .model(&gtk::StringList::new(&[
            &tr("Adaptive"),
            &tr("Large"),
            &tr("Compact"),
            &tr("Hidden"),
        ]))
        .build();
    banner.set_selected(
        crate::library_nav::BANNER_MODES
            .iter()
            .position(|m| *m == shared.cfg.borrow().banner_mode)
            .unwrap_or(0) as u32,
    );
    {
        let (on_change, shared) = (on_change.clone(), shared.clone());
        banner.connect_selected_notify(move |c| {
            {
                let mut cfg = shared.cfg.borrow_mut();
                cfg.banner_mode = crate::library_nav::BANNER_MODES
                    .get(c.selected() as usize)
                    .copied()
                    .unwrap_or("")
                    .to_string();
                cfg.save();
            }
            on_change(PrefChange::Banner);
        });
    }
    lib.add(&banner);
    library_page.add(&lib);
    {
        let on_change = on_change.clone();
        library_page.add(&crate::ui::health::preferences_group(
            parent,
            shared,
            Rc::new(move |m| on_change(PrefChange::Layout(m))),
        ));
    }

    // --- appearance -------------------------------------------------------
    let app = adw::PreferencesGroup::builder()
        .title(tr("Appearance"))
        .build();
    let scheme = adw::ComboRow::builder()
        .title(tr("Color scheme"))
        .model(&gtk::StringList::new(&["Follow system", "Light", "Dark"]))
        .build();
    scheme.set_selected(match shared.cfg.borrow().scheme {
        Scheme::System => 0,
        Scheme::Light => 1,
        Scheme::Dark => 2,
    });
    {
        let on_change = on_change.clone();
        scheme.connect_selected_notify(move |c| {
            on_change(PrefChange::Scheme(match c.selected() {
                1 => Scheme::Light,
                2 => Scheme::Dark,
                _ => Scheme::System,
            }));
        });
    }
    app.add(&scheme);
    page.add(&app);

    // --- reader -----------------------------------------------------------
    let reader = adw::PreferencesGroup::builder()
        .title(tr("Reader defaults"))
        .description(tr(
            "These can also be changed while reading from the Appearance menu",
        ))
        .build();
    let theme = adw::ComboRow::builder()
        .title(tr("Theme"))
        .model(&gtk::StringList::new(&ReaderTheme::ALL.map(|t| t.label())))
        .build();
    theme.set_selected(
        ReaderTheme::ALL
            .iter()
            .position(|t| *t == shared.cfg.borrow().reader.theme)
            .unwrap_or(0) as u32,
    );
    let layout = adw::ComboRow::builder()
        .title(tr("Layout"))
        .model(&gtk::StringList::new(&[
            "Two columns",
            "Single column",
            "Continuous scroll",
        ]))
        .build();
    layout.set_selected(match shared.cfg.borrow().reader.layout {
        LayoutMode::TwoColumns => 0,
        LayoutMode::SingleColumn => 1,
        LayoutMode::Scroll => 2,
    });
    let size = adw::SpinRow::with_range(8.0, 40.0, 1.0);
    size.set_title(&tr("Font size (pt)"));
    size.set_value(shared.cfg.borrow().reader.font_size);
    let justify = adw::SwitchRow::builder().title(tr("Justify text")).build();
    justify.set_active(shared.cfg.borrow().reader.justify);
    let eink = adw::SwitchRow::builder()
        .title(tr("E-ink mode"))
        .subtitle(tr(
            "Black on white, no animations, bars that stay, wide tap zones",
        ))
        .build();
    eink.set_active(shared.cfg.borrow().eink);
    let zones = adw::ComboRow::builder()
        .title(tr("Tap zones"))
        .subtitle(tr("Which part of the page turns it when tapped or clicked"))
        .model(&gtk::StringList::new(
            &crate::gestures::TapZones::ALL
                .iter()
                .map(|z| z.label())
                .collect::<Vec<_>>(),
        ))
        .build();
    zones.set_selected(
        crate::gestures::TapZones::ALL
            .iter()
            .position(|z| *z == shared.cfg.borrow().reader.tap_zones)
            .unwrap_or(0) as u32,
    );
    let touch = adw::ComboRow::builder()
        .title(tr("Large touch controls"))
        .model(&gtk::StringList::new(
            &crate::gestures::TouchUi::ALL
                .iter()
                .map(|t| t.label())
                .collect::<Vec<_>>(),
        ))
        .build();
    touch.set_selected(
        crate::gestures::TouchUi::ALL
            .iter()
            .position(|t| *t == shared.cfg.borrow().reader.touch_ui)
            .unwrap_or(0) as u32,
    );
    let styles = adw::SwitchRow::builder()
        .title(tr("Use the book's own styling"))
        .subtitle(tr(
            "Italic and bold classes, alignment, hidden text. Takes effect when a chapter loads.",
        ))
        .build();
    styles.set_active(shared.cfg.borrow().reader.publisher_styles);
    let hyphenate = adw::SwitchRow::builder()
        .title(tr("Hyphenate words"))
        .subtitle(tr(
            "Break long words at line ends, in the language of the book",
        ))
        .build();
    hyphenate.set_active(shared.cfg.borrow().reader.hyphenate);

    let apply = {
        let shared = shared.clone();
        let on_change = on_change.clone();
        let theme = theme.clone();
        let layout = layout.clone();
        let size = size.clone();
        let justify = justify.clone();
        let hyphenate = hyphenate.clone();
        let styles = styles.clone();
        let zones = zones.clone();
        let touch = touch.clone();
        let eink = eink.clone();
        Rc::new(move || {
            {
                let mut c = shared.cfg.borrow_mut();
                let chosen = ReaderTheme::ALL[theme.selected() as usize % ReaderTheme::ALL.len()];
                if chosen != c.reader.theme {
                    // picking a theme means the theme's colours
                    c.reader.colors = crate::config::ColorMode::Theme;
                }
                c.reader.theme = chosen;
                c.reader.layout = match layout.selected() {
                    1 => LayoutMode::SingleColumn,
                    2 => LayoutMode::Scroll,
                    _ => LayoutMode::TwoColumns,
                };
                c.reader.font_size = size.value();
                c.reader.justify = justify.is_active();
                c.reader.hyphenate = hyphenate.is_active();
                c.reader.publisher_styles = styles.is_active();
                let was = c.eink;
                c.eink = eink.is_active();
                if c.eink && !was {
                    crate::eink::tune_prefs(&mut c.reader);
                }
                c.reader.tap_zones = crate::gestures::TapZones::ALL
                    [zones.selected() as usize % crate::gestures::TapZones::ALL.len()];
                c.reader.touch_ui = crate::gestures::TouchUi::ALL
                    [touch.selected() as usize % crate::gestures::TouchUi::ALL.len()];
                c.save();
            }
            on_change(PrefChange::ReaderDefaults);
        })
    };
    {
        let a = apply.clone();
        theme.connect_selected_notify(move |_| a());
        let a = apply.clone();
        layout.connect_selected_notify(move |_| a());
        let a = apply.clone();
        size.connect_value_notify(move |_| a());
        let a = apply.clone();
        justify.connect_active_notify(move |_| a());
        let a = apply.clone();
        hyphenate.connect_active_notify(move |_| a());
        let a = apply.clone();
        styles.connect_active_notify(move |_| a());
        let a = apply.clone();
        eink.connect_active_notify(move |_| a());
        let a = apply.clone();
        zones.connect_selected_notify(move |_| a());
        let a = apply.clone();
        touch.connect_selected_notify(move |_| a());
    }
    reader.add(&theme);
    reader.add(&layout);
    reader.add(&size);
    reader.add(&justify);
    reader.add(&hyphenate);
    reader.add(&styles);
    reader.add(&eink);
    reader.add(&zones);
    reader.add(&touch);
    reading.add(&reader);

    // ---- plugins: sandboxed extensions (WebAssembly) -------------------------------------
    let plugins = adw::PreferencesGroup::new();
    plugins.set_title(&tr("Plugins"));
    plugins.set_description(Some(
        "Extensions run in a sandbox and can only do what their manifest asks for. They add metadata sources and file formats.",
    ));
    let list: Vec<bookx_plugin::PluginStatus> = shared.plugins.borrow().list();
    if list.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title(&tr("No plugins installed"));
        row.set_subtitle(&relm4::gtk::glib::markup_escape_text(&trf(
            "Put plugin folders (plugin.toml and plugin.wasm) into {}",
            &[&(shared.plugins.borrow().dir().display())],
        )));
        plugins.add(&row);
    }
    for st in list {
        let row = adw::SwitchRow::new();
        row.set_title(&relm4::gtk::glib::markup_escape_text(&format!(
            "{} {}",
            st.name, st.version
        )));
        let kind = st
            .kind
            .map(crate::plugins::kind_summary)
            .unwrap_or_default();
        let sub = match &st.error {
            Some(e) => trf("Problem: {}", &[&e]),
            None => format!(
                "{kind} · {}\n{}",
                st.description,
                crate::plugins::permission_summary(&st.permissions)
            ),
        };
        row.set_subtitle(&relm4::gtk::glib::markup_escape_text(&sub));
        row.set_active(st.enabled);
        row.set_sensitive(st.error.is_none() || st.enabled);
        let shared = shared.clone();
        let id = st.id.clone();
        row.connect_active_notify(move |r| {
            let on = r.is_active();
            let mut m = shared.plugins.borrow_mut();
            match m.set_enabled(&id, on) {
                Ok(()) => {
                    for (pid, e) in crate::plugins::reload(&mut m) {
                        tracing::warn!("plugin {pid} could not be loaded: {e}");
                    }
                }
                Err(e) => {
                    tracing::warn!("cannot switch plugin {id}: {e:#}");
                    r.set_subtitle(&relm4::gtk::glib::markup_escape_text(&trf(
                        "Problem: {}",
                        &[&format!("{:#}", e)],
                    )));
                    r.set_active(!on);
                }
            }
        });
        plugins.add(&row);
        if st.kind == Some(bookx_plugin::PluginKind::Transform) && st.error.is_none() {
            let auto = adw::SwitchRow::new();
            auto.set_title(&relm4::gtk::glib::markup_escape_text(&trf(
                "Run {} in every conversion",
                &[&st.name],
            )));
            auto.set_subtitle(&tr("Applied by Convert To and Send to, after the built-in steps. It also stays available as a one-off tool."));
            let path = crate::plugins::auto_transforms_path();
            auto.set_active(crate::plugins::load_auto_transforms(&path).contains(&st.id));
            auto.set_sensitive(st.enabled);
            let id = st.id.clone();
            auto.connect_active_notify(move |r| {
                if let Err(e) = crate::plugins::set_auto_transform(&path, &id, r.is_active()) {
                    tracing::warn!("cannot save the conversion option of {id}: {e:#}");
                }
            });
            row.bind_property("active", &auto, "sensitive")
                .sync_create()
                .build();
            plugins.add(&auto);
        }
    }
    let open_row = adw::ActionRow::new();
    open_row.set_title(&tr("Open plugins folder"));
    open_row.set_activatable(true);
    open_row.add_suffix(&gtk::Image::from_icon_name("folder-open-symbolic"));
    {
        let shared = shared.clone();
        open_row.connect_activated(move |_| {
            let dir = shared.plugins.borrow().dir().to_path_buf();
            let _ = std::fs::create_dir_all(&dir);
            super::open_uri(&format!("file://{}", dir.display()));
        });
    }
    plugins.add(&open_row);
    extras.add(&plugins);

    // ---- patches: small TOML rule files (X9) -----------------------------------------------
    let patches = adw::PreferencesGroup::new();
    patches.set_title(&tr("Patches"));
    patches.set_description(Some(
        "Small rule files that fix text or style in books when they are imported, converted or opened. They only edit the book itself, never other files. Off until you switch them on; use the magnifier for a dry run on your first book.",
    ));
    let shown: Rc<RefCell<Vec<adw::PreferencesRow>>> = Rc::default();
    fill_patches(&patches, shared, &shown);
    patches_page.add(&patches);

    // ---- devices ----------------------------------------------------------------------------
    let devices = adw::PreferencesGroup::new();
    devices.set_title(&tr("Devices"));
    devices.set_description(Some(
        "What happens when an e-reader is connected. Syncing is always progressive: progress only moves forward, highlights and notes are added to the side that lacks them, and nothing is ever deleted.",
    ));
    let auto_sync = adw::SwitchRow::builder()
        .title(crate::device_sync::AUTO_SYNC_TITLE)
        .subtitle(crate::device_sync::AUTO_SYNC_SUBTITLE)
        .build();
    auto_sync.add_suffix(&crate::ui::help_button(crate::device_sync::AUTO_SYNC_HELP));
    auto_sync.set_active(shared.cfg.borrow().device_sync.auto_sync);
    let include_notes = adw::SwitchRow::builder()
        .title(crate::device_sync::NOTES_TITLE)
        .subtitle(crate::device_sync::NOTES_SUBTITLE)
        .build();
    include_notes.set_active(shared.cfg.borrow().device_sync.include_notes);
    {
        let sh = shared.clone();
        auto_sync.connect_active_notify(move |r| {
            let mut c = sh.cfg.borrow_mut();
            c.device_sync.auto_sync = r.is_active();
            c.save();
        });
        let sh = shared.clone();
        include_notes.connect_active_notify(move |r| {
            let mut c = sh.cfg.borrow_mut();
            c.device_sync.include_notes = r.is_active();
            c.save();
        });
    }
    devices.add(&auto_sync);
    devices.add(&include_notes);
    devices_page.add(&devices);

    // ---- e-mail: for "Send by E-mail" ------------------------------------------------------
    let mail = adw::PreferencesGroup::new();
    mail.set_title(&tr("E-mail"));
    mail.set_description(Some(&trf("Send books to a Kindle or another address. Use an app password if your provider offers one. {}", &[&crate::secrets::caption_now()])));
    let cur = shared.cfg.borrow().email.clone();
    let entry = |title: &str, text: &str| {
        let r = adw::EntryRow::new();
        r.set_title(title);
        r.set_text(text);
        r
    };
    let host = entry(&tr("Mail server"), &cur.host);
    let port = adw::SpinRow::with_range(1.0, 65535.0, 1.0);
    port.set_title(&tr("Mail server port"));
    port.set_value(cur.port as f64);
    let enc = adw::ComboRow::new();
    enc.set_title(&tr("Mail encryption"));
    enc.set_model(Some(&gtk::StringList::new(
        &crate::emailing::Encryption::ALL.map(|e| e.label()),
    )));
    enc.set_selected(
        crate::emailing::Encryption::ALL
            .iter()
            .position(|e| *e == cur.encryption)
            .unwrap_or(0) as u32,
    );
    let user = entry(&tr("Mail user name"), &cur.username);
    let pass = adw::PasswordEntryRow::new();
    pass.set_title(&tr("Mail password"));
    pass.set_text(&cur.password);
    let from = entry(&tr("Your address"), &cur.from);
    let to = entry(&tr("Send books to"), &cur.to);
    for r in [&host, &user, &from, &to] {
        mail.add(r);
    }
    mail.add(&port);
    mail.add(&enc);
    mail.add(&pass);
    let save_mail = {
        let shared = shared.clone();
        let (host, port, enc, user, pass, from, to) = (
            host.clone(),
            port.clone(),
            enc.clone(),
            user.clone(),
            pass.clone(),
            from.clone(),
            to.clone(),
        );
        Rc::new(move || {
            let mut c = shared.cfg.borrow_mut();
            c.email.host = host.text().trim().to_string();
            c.email.port = port.value() as u16;
            c.email.encryption = crate::emailing::Encryption::ALL
                [enc.selected() as usize % crate::emailing::Encryption::ALL.len()];
            c.email.username = user.text().trim().to_string();
            c.email.password = pass.text().to_string();
            c.email.from = from.text().trim().to_string();
            c.email.to = to.text().trim().to_string();
            c.save();
        })
    };
    for r in [&host, &user, &from, &to] {
        let f = save_mail.clone();
        r.connect_changed(move |_| f());
    }
    {
        let f = save_mail.clone();
        pass.connect_changed(move |_| f());
        let f = save_mail.clone();
        port.connect_value_notify(move |_| f());
    }
    {
        // choosing an encryption also suggests its usual port
        let f = save_mail.clone();
        let port = port.clone();
        enc.connect_selected_notify(move |e| {
            let sel = crate::emailing::Encryption::ALL
                [e.selected() as usize % crate::emailing::Encryption::ALL.len()];
            port.set_value(sel.default_port() as f64);
            f();
        });
    }
    email_page.add(&mail);

    sync_page.add(&super::kosync::preferences_group(shared));

    let built = [
        page,
        library_page,
        reading,
        devices_page,
        email_page,
        sync_page,
        extras,
        patches_page,
    ];
    let mut sections: Vec<Section> = built
        .into_iter()
        .zip(BUILTIN)
        .map(|(page, (id, title, icon, order))| Section {
            id: id.into(),
            title: tr(title),
            icon: icon.into(),
            order,
            page,
        })
        .collect();
    let (specs, mut groups) = REGISTRY.with(|r| {
        let r = r.borrow();
        let specs: Vec<(String, String, String, i32)> = r
            .pages
            .iter()
            .map(|p| (p.id.clone(), p.title.clone(), p.icon.clone(), p.order))
            .collect();
        (specs, r.groups.clone())
    });
    for (id, title, icon, order) in specs {
        if !sections.iter().any(|s| s.id == id) {
            sections.push(Section {
                page: new_page(&title, &icon),
                id,
                title,
                icon,
                order,
            });
        }
    }
    groups.sort_by_key(|g| g.1);
    for (page_id, _, build) in groups {
        if let Some(s) = sections.iter().find(|s| s.id == page_id) {
            s.page.add(&build(&ctx));
        }
    }
    sections.retain(|s| page_has_groups(&s.page));
    sections.sort_by_key(|s| s.order);
    sections
}

fn page_has_groups(p: &adw::PreferencesPage) -> bool {
    let mut g = Vec::new();
    collect_groups(p.upcast_ref(), &mut g);
    !g.is_empty()
}

/// (Re)builds the rows of the patch list: one switch per patch with move and dry-run buttons.
fn fill_patches(
    group: &adw::PreferencesGroup,
    shared: &SharedRef,
    rows: &Rc<RefCell<Vec<adw::PreferencesRow>>>,
) {
    for r in rows.borrow_mut().drain(..) {
        group.remove(&r);
    }
    let mgr = crate::plugins::patch_manager();
    let list = mgr.list();
    let add = |row: adw::PreferencesRow| {
        group.add(&row);
        rows.borrow_mut().push(row);
    };
    if list.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title(&tr("No patches installed"));
        row.set_subtitle(&relm4::gtk::glib::markup_escape_text(&trf(
            "Put .toml patch files into {}",
            &[&(mgr.dir().display())],
        )));
        add(row.upcast());
    }
    let last = list.len().saturating_sub(1);
    for (i, st) in list.into_iter().enumerate() {
        let row = adw::SwitchRow::new();
        row.set_title(&relm4::gtk::glib::markup_escape_text(&st.name));
        let sub = match (&st.error, st.hook) {
            (Some(e), _) => trf("Problem: {}", &[&e]),
            (None, hook) => trf(
                "{} · {} · {}\n{}",
                &[
                    &(hook.map(|h| h.label()).unwrap_or("")),
                    &ntr("{} rule", "{} rules", st.rules),
                    &st.description,
                    &bookx_plugin::patch::PERMISSIONS,
                ],
            ),
        };
        row.set_subtitle(&relm4::gtk::glib::markup_escape_text(&sub));
        row.set_active(st.enabled);
        row.set_sensitive(st.error.is_none());
        {
            let id = st.id.clone();
            row.connect_active_notify(move |r| {
                let mut m = crate::plugins::patch_manager();
                if let Err(e) = m.set_enabled(&id, r.is_active()) {
                    tracing::warn!("cannot switch patch {id}: {e:#}");
                    r.set_active(false);
                }
                crate::plugins::reload_patches();
            });
        }
        let tools = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        tools.set_valign(gtk::Align::Center);
        let button = |icon: &str, tip: &str| {
            let b = gtk::Button::from_icon_name(icon);
            b.set_tooltip_text(Some(tip));
            b.add_css_class("flat");
            b
        };
        let up = button("go-up-symbolic", &tr("Apply earlier"));
        up.set_sensitive(i > 0);
        let down = button("go-down-symbolic", &tr("Apply later"));
        down.set_sensitive(i < last);
        let dry = button("system-search-symbolic", &tr("Dry run on your first book"));
        dry.set_sensitive(st.error.is_none());
        for (b, delta) in [(&up, -1isize), (&down, 1)] {
            let id = st.id.clone();
            let (group, shared, rows) = (group.clone(), shared.clone(), rows.clone());
            b.connect_clicked(move |_| {
                let mut m = crate::plugins::patch_manager();
                if let Err(e) = m.move_by(&id, delta) {
                    tracing::warn!("cannot move patch {id}: {e:#}");
                }
                crate::plugins::reload_patches();
                fill_patches(&group, &shared, &rows);
            });
        }
        {
            let id = st.id.clone();
            let shared = shared.clone();
            dry.connect_clicked(move |b| {
                let epub = {
                    let lib = shared.lib.borrow();
                    lib.books.first().map(|bk| lib.book_path(bk))
                };
                let m = crate::plugins::patch_manager();
                let text = match crate::plugins::preview_patch(&m, &id, epub.as_deref()) {
                    Ok(r) => r.summary(),
                    Err(e) => format!("{e:#}"),
                };
                let d = adw::AlertDialog::new(Some(&tr("Dry run")), Some(&text));
                d.add_response("ok", &tr("Close"));
                d.present(Some(b));
            });
        }
        tools.append(&dry);
        tools.append(&up);
        tools.append(&down);
        row.add_suffix(&tools);
        add(row.upcast());
    }
    let open_row = adw::ActionRow::new();
    open_row.set_title(&tr("Open patches folder"));
    open_row.set_activatable(true);
    open_row.add_suffix(&gtk::Image::from_icon_name("folder-open-symbolic"));
    open_row.connect_activated(|_| {
        let dir = crate::plugins::patches_dir();
        let _ = std::fs::create_dir_all(&dir);
        super::open_uri(&format!("file://{}", dir.display()));
    });
    add(open_row.upcast());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_words_all_have_to_match() {
        assert!(query_matches("", "Anything"));
        assert!(query_matches("MAIL server", "Mail server (SMTP) host name"));
        assert!(!query_matches("mail kindle", "Mail server"));
        assert!(query_matches("  hyphen ", "Hyphenate words"));
    }

    #[test]
    fn builtin_sections_are_ordered_and_unique() {
        let orders: Vec<i32> = BUILTIN.iter().map(|b| b.3).collect();
        assert!(orders.windows(2).all(|w| w[0] < w[1]));
        let mut ids: Vec<&str> = BUILTIN.iter().map(|b| b.0).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), BUILTIN.len());
    }

    #[test]
    fn registering_a_page_twice_keeps_the_first() {
        register_page("t-test", "First", "x", 5);
        register_page("t-test", "Second", "y", 6);
        let n = REGISTRY.with(|r| {
            r.borrow()
                .pages
                .iter()
                .filter(|p| p.id == "t-test" && p.title == "First")
                .count()
        });
        assert_eq!(n, 1);
    }
}
