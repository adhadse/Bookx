//! The History dialog: go back in the history of highlights, notes and bookmarks and restore what
//! was deleted (by you, by a device sync, by a merge) or overwritten. Reached from the reader's
//! Notes tab, the library's Notes & Highlights view, a book's context menu and the toast a sync
//! shows after it removed annotations.
//!
//! GNOME HIG: an `AdwDialog` with a header bar, a search entry and an `AdwToggleGroup` filter, a
//! list grouped by day in `AdwPreferencesGroup`s (a bulk change is one `AdwExpanderRow` with the
//! entries beneath it), a Restore button per row and per group, a select mode with *Restore
//! Selected*, an `AdwStatusPage` for the empty state, an undo toast after every restore and an
//! `AdwAlertDialog` only for *Clear History*, which cannot be undone. The data and the rules are in
//! `bookx_core::annotation_journal`, `bookx_store::annotation_history` and `crate::annotation_history`.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use bookx_core::annotation_journal::{
    self as journal, Action, ActionFilter, Entry, Filter, Group, Kind, Outcome, Source,
};
use chrono::{Datelike, Local, Utc};
use relm4::adw::{self, prelude::*};
use relm4::gtk;

use super::SharedRef;
use crate::annotation_history as model;
use crate::annotation_sync as sync;
use crate::i18n::{tr, trf};

/// How many entries the list shows at most (the newest ones).
const SHOWN: usize = 500;

/// Jumps to an annotation: (book id, annotation id).
pub type OpenFn = Rc<dyn Fn(u64, String)>;

struct Ui {
    shared: SharedRef,
    dialog: adw::Dialog,
    overlay: adw::ToastOverlay,
    stack: gtk::Stack,
    list: gtk::Box,
    scope: Option<u64>,
    open: Option<OpenFn>,
    filter: RefCell<Filter>,
    select: Cell<bool>,
    selected: RefCell<HashSet<u64>>,
    actionbar: gtk::ActionBar,
    selected_label: gtk::Label,
    restore_selected: gtk::Button,
    devices: gtk::DropDown,
    device_names: RefCell<Vec<String>>,
    /// What the list shows now, for the dev hooks: (groups, entries, restorable entries).
    shown: Cell<(usize, usize, usize)>,
}

thread_local! {
    static OPEN: RefCell<Option<Rc<Ui>>> = const { RefCell::new(None) };
    /// The sync's removals the toast offers to take back: (toast text, entry ids, book).
    static PENDING: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
    static LAST_TOAST: RefCell<Option<(String, Vec<u64>, u64)>> = const { RefCell::new(None) };
}

/// A sync (or anything else) wrote these entries; the toast of the sync names the removals.
pub fn note_removed(entries: Vec<Entry>) {
    PENDING.with(|p| *p.borrow_mut() = entries);
}

fn store(shared: &SharedRef) -> Arc<bookx_store::SqliteStore> {
    shared.guard.borrow().store.clone()
}

fn local_offset() -> chrono::FixedOffset {
    *Local::now().offset()
}

fn day_label(d: chrono::NaiveDate) -> String {
    let today = Local::now().date_naive();
    if d == today {
        tr("Today")
    } else if Some(d) == today.pred_opt() {
        tr("Yesterday")
    } else {
        crate::i18n::fmt_date(d.year(), d.month(), d.day())
    }
}

fn kind_icon(k: Kind) -> &'static str {
    match k {
        Kind::Highlight => "format-text-underline-symbolic",
        Kind::Note => "document-edit-symbolic",
        Kind::Bookmark => "bookmark-new-symbolic",
    }
}

fn kind_name(k: Kind) -> String {
    match k {
        Kind::Highlight => tr("Highlight"),
        Kind::Note => tr("Note"),
        Kind::Bookmark => tr("Bookmark"),
    }
}

/// What the state of an entry in the book says, when it cannot be restored (words, not colour).
fn state_text(action: Action, o: Outcome) -> Option<String> {
    match o {
        Outcome::AlreadyThere if action == Action::Deleted => Some(tr("Back in the book")),
        Outcome::AlreadyThere => Some(tr("Current version")),
        Outcome::SamePassage => Some(tr("Already marked there")),
        Outcome::BookMissing => Some(tr("Book not in library")),
        _ => None,
    }
}

fn restorable(action: Action, o: Outcome) -> bool {
    action != Action::Added && matches!(o, Outcome::Restored | Outcome::Replaced)
}

impl Ui {
    fn outcome(&self, e: &Entry) -> Outcome {
        journal::check(e, self.shared.lib.borrow().get(e.book))
    }

    fn rebuild(self: &Rc<Self>) {
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let st = store(&self.shared);
        let mut f = self.filter.borrow().clone();
        f.book = self.scope;
        let entries = st.history(&f, SHOWN).unwrap_or_default();
        let nothing_at_all = entries.is_empty() && {
            let all = Filter {
                book: self.scope,
                ..Filter::default()
            };
            st.history(&all, 1).map(|v| v.is_empty()).unwrap_or(true)
        };
        let days = journal::group_by_day(&entries, &f, local_offset());
        self.refresh_devices(&st);
        let now = Utc::now().timestamp();
        let (mut groups, mut rows, mut can) = (0, 0, 0);
        for day in &days {
            let g = adw::PreferencesGroup::new();
            g.set_title(&day_label(day.date));
            for grp in &day.groups {
                groups += 1;
                rows += grp.entries.len();
                can += grp
                    .entries
                    .iter()
                    .filter(|e| restorable(e.action, self.outcome(e)))
                    .count();
                if grp.entries.len() == 1 {
                    g.add(&self.entry_row(&grp.entries[0], now, true));
                } else {
                    g.add(&self.group_row(grp, now));
                }
            }
            self.list.append(&g);
        }
        if entries.len() >= SHOWN {
            let l = gtk::Label::new(Some(&trf("Showing the newest {} changes", &[&SHOWN])));
            l.add_css_class("dim-label");
            l.add_css_class("caption");
            self.list.append(&l);
        }
        self.shown.set((groups, rows, can));
        self.stack.set_visible_child_name(if !days.is_empty() {
            "list"
        } else if nothing_at_all {
            "empty"
        } else {
            "none"
        });
        self.update_bar();
    }

    fn refresh_devices(&self, st: &bookx_store::SqliteStore) {
        let names = st.history_devices();
        if *self.device_names.borrow() == names {
            return;
        }
        let mut items = vec![tr("Everyone"), tr("You")];
        items.extend(names.iter().cloned());
        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
        let keep = self.devices.selected();
        self.devices.set_model(Some(&gtk::StringList::new(&refs)));
        self.devices.set_selected(if (keep as usize) < items.len() {
            keep
        } else {
            0
        });
        *self.device_names.borrow_mut() = names;
    }

    fn subtitle(&self, e: &Entry, now: i64, with_book: bool) -> String {
        let mut s = format!(
            "{} · {}",
            model::what_happened(e.action, &e.source),
            crate::sync_activity::relative(now, e.at.timestamp())
        );
        if with_book && self.scope.is_none() {
            s.push_str(&format!(" · {}", e.title));
        }
        if !e.record.note().trim().is_empty() {
            let note: String = e
                .record
                .note()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let cut: String = note.chars().take(90).collect();
            s.push('\n');
            s.push_str(&trf("Note: {}", &[&cut]));
        }
        s
    }

    fn entry_row(self: &Rc<Self>, e: &Entry, now: i64, with_book: bool) -> adw::ActionRow {
        let o = self.outcome(e);
        let can = restorable(e.action, o);
        let row = adw::ActionRow::new();
        row.set_use_markup(false);
        row.set_title(&e.record.excerpt(140));
        row.set_title_lines(2);
        row.set_subtitle(&self.subtitle(e, now, with_book));
        row.set_subtitle_lines(3);
        row.add_prefix(&gtk::Image::from_icon_name(kind_icon(e.kind)));
        let what = format!("{}: {}", kind_name(e.kind), e.record.excerpt(60));
        let status = state_text(e.action, o);
        super::audit::label(&row, &what);
        row.upcast_ref::<gtk::Widget>()
            .update_property(&[gtk::accessible::Property::Description(&format!(
                "{}{}",
                self.subtitle(e, now, with_book).replace('\n', ". "),
                status
                    .as_ref()
                    .map(|s| format!(". {s}"))
                    .unwrap_or_default()
            ))]);
        if self.select.get() && can {
            let c = gtk::CheckButton::new();
            c.set_active(self.selected.borrow().contains(&e.id));
            super::audit::label(&c, &trf("Select {}", &[&e.record.excerpt(40)]));
            let (ui, id) = (self.clone(), e.id);
            c.connect_toggled(move |c| ui.toggle(id, c.is_active()));
            row.add_prefix(&c);
            row.set_activatable_widget(Some(&c));
        } else if can {
            let b = gtk::Button::with_label(&tr("Restore"));
            b.set_valign(gtk::Align::Center);
            super::audit::label(&b, &trf("Restore {}", &[&e.record.excerpt(40)]));
            let (ui, id) = (self.clone(), e.id);
            b.connect_clicked(move |_| ui.restore(vec![id]));
            row.add_suffix(&b);
        } else if let Some(s) = status {
            let l = gtk::Label::new(Some(&s));
            l.add_css_class("dim-label");
            l.add_css_class("caption");
            l.set_valign(gtk::Align::Center);
            row.add_suffix(&l);
        }
        if let (Some(open), true) = (&self.open, self.exists_now(e)) {
            let b = gtk::Button::from_icon_name("go-jump-symbolic");
            b.add_css_class("flat");
            b.set_valign(gtk::Align::Center);
            b.set_tooltip_text(Some(&tr("Open in Book")));
            super::audit::label(&b, &trf("Open {} in Book", &[&e.record.excerpt(40)]));
            let (open, book, id, dialog) = (
                open.clone(),
                e.book,
                e.record.id().to_string(),
                self.dialog.clone(),
            );
            b.connect_clicked(move |_| {
                dialog.close();
                open(book, id.clone());
            });
            row.add_suffix(&b);
        }
        row
    }

    /// The record is in its book right now (so there is a place to open).
    fn exists_now(&self, e: &Entry) -> bool {
        let lib = self.shared.lib.borrow();
        let Some(b) = lib.get(e.book) else {
            return false;
        };
        match &e.record {
            journal::Record::Highlight(h) => b.highlights.iter().any(|x| x.id == h.id),
            journal::Record::Bookmark(m) => b.bookmarks.iter().any(|x| x.id == m.id),
        }
    }

    fn group_row(self: &Rc<Self>, g: &Group, now: i64) -> adw::ExpanderRow {
        let row = adw::ExpanderRow::new();
        row.set_use_markup(false);
        row.set_title(&model::group_title(g));
        row.set_title_lines(2);
        row.set_subtitle(&format!(
            "{} · {}",
            model::what_happened(g.action, &g.source),
            crate::sync_activity::relative(now, g.at.timestamp())
        ));
        row.add_prefix(&gtk::Image::from_icon_name(kind_icon(g.entries[0].kind)));
        let ids: Vec<u64> = g
            .entries
            .iter()
            .filter(|e| restorable(e.action, self.outcome(e)))
            .map(|e| e.id)
            .collect();
        let name = model::group_title(g);
        super::audit::label(&row, &name);
        if !ids.is_empty() {
            if self.select.get() {
                let c = gtk::CheckButton::new();
                c.set_active(ids.iter().all(|i| self.selected.borrow().contains(i)));
                super::audit::label(&c, &trf("Select all: {}", &[&name]));
                let (ui, ids) = (self.clone(), ids.clone());
                c.connect_toggled(move |c| {
                    for i in &ids {
                        ui.toggle_quiet(*i, c.is_active());
                    }
                    ui.update_bar();
                    ui.rebuild_keep();
                });
                row.add_prefix(&c);
            } else {
                let b = gtk::Button::with_label(&tr("Restore All"));
                b.set_valign(gtk::Align::Center);
                super::audit::label(&b, &trf("Restore all: {}", &[&name]));
                let ui = self.clone();
                b.connect_clicked(move |_| ui.restore(ids.clone()));
                row.add_suffix(&b);
            }
        }
        for e in &g.entries {
            row.add_row(&self.entry_row(e, now, false));
        }
        row
    }

    fn toggle_quiet(&self, id: u64, on: bool) {
        if on {
            self.selected.borrow_mut().insert(id);
        } else {
            self.selected.borrow_mut().remove(&id);
        }
    }

    fn toggle(&self, id: u64, on: bool) {
        self.toggle_quiet(id, on);
        self.update_bar();
    }

    /// Redraws after a group checkbox changed its entries, without losing the scroll position.
    fn rebuild_keep(self: &Rc<Self>) {
        let ui = self.clone();
        super::sync_activity::keep_scroll(self.list.upcast_ref(), move || ui.rebuild());
    }

    fn update_bar(&self) {
        let n = self.selected.borrow().len();
        self.actionbar.set_revealed(self.select.get());
        self.selected_label.set_text(&trf("{} selected", &[&n]));
        self.restore_selected.set_sensitive(n > 0);
    }

    fn set_select(self: &Rc<Self>, on: bool) {
        self.select.set(on);
        self.selected.borrow_mut().clear();
        self.rebuild();
    }

    fn toast(&self, text: &str) {
        super::dialog_toast::show_plain(&self.overlay, text);
    }

    fn restore(self: &Rc<Self>, ids: Vec<u64>) {
        let r = restore_ids(&self.shared, &ids);
        match r {
            Err(e) => super::dialog_toast::show_error(
                &self.overlay,
                &trf("Could not restore: {}", &[&format!("{e:#}")]),
            ),
            Ok(r) => {
                let text = model::restored_toast(&r);
                if r.count() > 0 {
                    let (ui, mark) = (self.clone(), r.mark);
                    super::dialog_toast::show_undo(&self.overlay, &text, move || {
                        ui.undo(mark);
                    });
                } else {
                    self.toast(&text);
                }
            }
        }
        self.selected.borrow_mut().clear();
        self.rebuild();
    }

    fn undo(self: &Rc<Self>, mark: u64) {
        match undo_restore(&self.shared, mark) {
            Ok(n) if n > 0 => self.toast(&tr("Restore undone")),
            Ok(_) => self.toast(&tr("Nothing to undo")),
            Err(e) => super::dialog_toast::show_error(
                &self.overlay,
                &trf("Could not undo: {}", &[&format!("{e:#}")]),
            ),
        }
        self.rebuild();
    }

    fn clear(self: &Rc<Self>) {
        let st = store(&self.shared);
        let n = st.history_count();
        let ui = self.clone();
        super::alert(
            &self.dialog,
            &tr("Clear History?"),
            &trf(
                "This permanently deletes the history of {} changes for all books, so deleted highlights, notes and bookmarks can no longer be restored. Your books and everything now in them are not affected. This cannot be undone.",
                &[&n],
            ),
            &tr("Clear History"),
            true,
            move || {
                let st = store(&ui.shared);
                let _ = st.history_clear();
                let _ = st.history_compact();
                ui.toast(&tr("History cleared"));
                ui.rebuild();
            },
        );
    }

    fn selected_ids(&self) -> Vec<u64> {
        let mut v: Vec<u64> = self.selected.borrow().iter().copied().collect();
        v.sort_unstable();
        v
    }
}

/// Restores `ids` into the library (and settles the sync ledger). Used by the dialog and by the
/// toast of a sync.
pub fn restore_ids(shared: &SharedRef, ids: &[u64]) -> anyhow::Result<model::Restored> {
    let st = store(shared);
    let mut lib = shared
        .lib
        .try_borrow_mut()
        .map_err(|_| anyhow::anyhow!("{}", tr("the library is busy, try again in a moment")))?;
    let path = sync::ledger_path(&crate::annotation_sync_ui::data_dir());
    let mut ledger = sync::Ledger::load(&path);
    let r = model::restore(&mut lib, &st, &mut ledger, ids, Utc::now().timestamp())?;
    if r.count() > 0 {
        let _ = ledger.save(&path);
    }
    drop(lib);
    if r.count() > 0 {
        crate::settings_bus::sync().notify();
    }
    Ok(r)
}

fn undo_restore(shared: &SharedRef, mark: u64) -> anyhow::Result<usize> {
    let st = store(shared);
    let mut lib = shared
        .lib
        .try_borrow_mut()
        .map_err(|_| anyhow::anyhow!("{}", tr("the library is busy, try again in a moment")))?;
    let n = model::undo(&mut lib, &st, mark)?;
    drop(lib);
    crate::settings_bus::sync().notify();
    Ok(n)
}

/// Opens the History dialog. `book` narrows it to one book; `open` jumps to an annotation.
pub fn show(
    shared: &SharedRef,
    parent: &impl IsA<gtk::Widget>,
    book: Option<u64>,
    open: Option<OpenFn>,
) {
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr("History"));
    dialog.set_content_width(560);
    dialog.set_content_height(680);
    dialog.set_follows_content_size(false);

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let select_btn = gtk::ToggleButton::new();
    select_btn.set_icon_name("selection-mode-symbolic");
    select_btn.set_tooltip_text(Some(&tr("Select Entries")));
    super::audit::label(&select_btn, &tr("Select Entries"));
    header.pack_start(&select_btn);
    let menu = gtk::gio::Menu::new();
    menu.append(Some(&tr("Clear History")), Some("history.clear"));
    let menu_btn = gtk::MenuButton::new();
    menu_btn.set_icon_name("view-more-symbolic");
    menu_btn.set_menu_model(Some(&menu));
    menu_btn.set_tooltip_text(Some(&tr("Main Menu")));
    super::audit::label(&menu_btn, &tr("History menu"));
    header.pack_end(&menu_btn);
    toolbar.add_top_bar(&header);

    // search and filters
    let bar = gtk::Box::new(gtk::Orientation::Vertical, 8);
    bar.set_margin_start(12);
    bar.set_margin_end(12);
    bar.set_margin_top(6);
    bar.set_margin_bottom(6);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some(&tr("Search History")));
    super::audit::label(&search, &tr("Search History"));
    search.set_key_capture_widget(Some(&dialog));
    bar.append(&search);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let toggles = adw::ToggleGroup::new();
    for (name, label) in [
        ("all", tr("All")),
        ("deleted", tr("Deleted")),
        ("edited", tr("Edited")),
    ] {
        toggles.add(adw::Toggle::builder().name(name).label(label).build());
    }
    toggles.set_active_name(Some("all"));
    toggles.set_hexpand(true);
    super::audit::label(&toggles, &tr("Show changes"));
    row.append(&toggles);
    let devices = gtk::DropDown::from_strings(&[&tr("Everyone"), &tr("You")]);
    super::audit::label(&devices, &tr("Changed by"));
    devices.set_tooltip_text(Some(&tr("Changed by")));
    row.append(&devices);
    bar.append(&row);
    toolbar.add_top_bar(&bar);

    // content: list, empty page, no-match page
    let stack = gtk::Stack::new();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 18);
    list.set_margin_top(6);
    list.set_margin_bottom(18);
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(640);
    clamp.set_margin_start(12);
    clamp.set_margin_end(12);
    clamp.set_child(Some(&list));
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroll.set_child(Some(&clamp));
    stack.add_named(&scroll, Some("list"));
    let empty = adw::StatusPage::new();
    empty.set_icon_name(Some("document-open-recent-symbolic"));
    empty.set_title(&tr("No History Yet"));
    empty.set_description(Some(&tr(
        "When a highlight, note or bookmark is deleted or changed, you can find it here and restore it.",
    )));
    stack.add_named(&empty, Some("empty"));
    let none = adw::StatusPage::new();
    none.set_icon_name(Some("system-search-symbolic"));
    none.set_title(&tr("No Matches"));
    none.set_description(Some(&tr("Try another search or filter.")));
    stack.add_named(&none, Some("none"));
    stack.set_visible_child_name("empty");
    toolbar.set_content(Some(&stack));

    let actionbar = gtk::ActionBar::new();
    let selected_label = gtk::Label::new(None);
    actionbar.pack_start(&selected_label);
    let restore_selected = gtk::Button::with_label(&tr("Restore Selected"));
    restore_selected.add_css_class("suggested-action");
    super::audit::label(&restore_selected, &tr("Restore Selected"));
    actionbar.pack_end(&restore_selected);
    actionbar.set_revealed(false);
    toolbar.add_bottom_bar(&actionbar);

    let overlay = super::dialog_toast::overlay(&toolbar);
    dialog.set_child(Some(&overlay));

    let ui = Rc::new(Ui {
        shared: shared.clone(),
        dialog: dialog.clone(),
        overlay,
        stack,
        list,
        scope: book,
        open,
        filter: RefCell::new(Filter::default()),
        select: Cell::new(false),
        selected: RefCell::new(HashSet::new()),
        actionbar,
        selected_label,
        restore_selected: restore_selected.clone(),
        devices: devices.clone(),
        device_names: RefCell::new(Vec::new()),
        shown: Cell::new((0, 0, 0)),
    });

    {
        let u = ui.clone();
        select_btn.connect_toggled(move |b| u.set_select(b.is_active()));
        let u = ui.clone();
        restore_selected.connect_clicked(move |_| u.restore(u.selected_ids()));
    }
    {
        let u = ui.clone();
        toggles.connect_active_name_notify(move |t| {
            u.filter.borrow_mut().action = match t.active_name().as_deref() {
                Some("deleted") => ActionFilter::Deleted,
                Some("edited") => ActionFilter::Edited,
                _ => ActionFilter::All,
            };
            u.rebuild();
        });
        let u = ui.clone();
        devices.connect_selected_notify(move |d| {
            let names = u.device_names.borrow();
            u.filter.borrow_mut().source = match d.selected() {
                0 => None,
                1 => Some(Source::You),
                n => names.get(n as usize - 2).cloned().map(Source::Device),
            };
            drop(names);
            u.rebuild();
        });
        let u = ui.clone();
        search.connect_search_changed(move |e| {
            u.filter.borrow_mut().query = e.text().to_string();
            u.rebuild();
        });
    }
    // Clear History is an action of the dialog
    {
        let group = gtk::gio::SimpleActionGroup::new();
        let act = gtk::gio::SimpleAction::new("clear", None);
        let ui = ui.clone();
        act.connect_activate(move |_, _| ui.clear());
        group.add_action(&act);
        dialog.insert_action_group("history", Some(&group));
    }
    ui.rebuild();
    OPEN.with(|o| *o.borrow_mut() = Some(ui.clone()));
    dialog.connect_closed(|_| OPEN.with(|o| *o.borrow_mut() = None));
    dialog.present(Some(parent));
}

/// After a sync removed annotations: a toast with *Undo* (restore) and *Review* (open the history).
pub fn announce_removed(shared: &SharedRef) {
    let entries = PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()));
    let Some((text, ids)) = model::removed_toast(&entries) else {
        return;
    };
    let book = entries.first().map(|e| e.book).unwrap_or(0);
    LAST_TOAST.with(|t| *t.borrow_mut() = Some((text.clone(), ids.clone(), book)));
    let Some(overlay) = super::dialog_toast::app_overlay() else {
        return;
    };
    let (undo_shared, review_shared) = (shared.clone(), shared.clone());
    let parent = overlay.clone();
    super::dialog_toast::show_action_and(
        &overlay,
        &text,
        &tr("Undo"),
        move || {
            toast_undo(&undo_shared);
        },
        (&tr("Review"), move || {
            toast_review(&review_shared, &parent);
        }),
    );
}

/// The *Undo* button of the sync toast: restores what the sync removed.
pub fn toast_undo(shared: &SharedRef) -> bool {
    let Some((_, ids, _)) = LAST_TOAST.with(|t| t.borrow_mut().take()) else {
        return false;
    };
    let text = match restore_ids(shared, &ids) {
        Ok(r) => model::restored_toast(&r),
        Err(e) => trf("Could not restore: {}", &[&format!("{e:#}")]),
    };
    if let Some(o) = super::dialog_toast::app_overlay() {
        super::dialog_toast::show_plain(&o, &text);
    }
    true
}

/// The history of the book the last toast was about, from the main window.
pub fn toast_review(shared: &SharedRef, parent: &impl IsA<gtk::Widget>) {
    let book = LAST_TOAST.with(|t| t.borrow().as_ref().map(|t| t.2));
    show(shared, parent, book, None);
}

/// `history:<command>`, the test hooks (`_developer/docs.md`).
pub fn dev_command(shared: &SharedRef, parent: &impl IsA<gtk::Widget>, arg: &str) {
    let (cmd, rest) = arg.split_once('=').unwrap_or((arg, ""));
    let ui = OPEN.with(|o| o.borrow().clone());
    match (cmd, ui) {
        ("open", _) => show(shared, parent, rest.parse().ok(), None),
        ("toast", _) => eprintln!(
            "history_toast: {:?}",
            LAST_TOAST.with(|t| t.borrow().as_ref().map(|t| (t.0.clone(), t.1.len())))
        ),
        ("toast-undo", _) => eprintln!("history_toast_undo: {}", toast_undo(shared)),
        ("count", _) => eprintln!("history_count: {}", store(shared).history_count()),
        ("state", Some(ui)) => {
            let (g, n, r) = ui.shown.get();
            eprintln!(
                "history_state: groups={g} entries={n} restorable={r} select={} page={} selected={} filter={:?}",
                ui.select.get() as u8,
                ui.stack.visible_child_name().unwrap_or_default(),
                ui.selected.borrow().len(),
                ui.filter.borrow().action
            );
            if let Some(first) = journal_first(&ui) {
                eprintln!("history_first: {first}");
            }
        }
        ("filter", Some(ui)) => {
            ui.filter.borrow_mut().action = match rest {
                "deleted" => ActionFilter::Deleted,
                "edited" => ActionFilter::Edited,
                _ => ActionFilter::All,
            };
            ui.rebuild();
        }
        ("search", Some(ui)) => {
            ui.filter.borrow_mut().query = rest.to_string();
            ui.rebuild();
        }
        ("restore-all", Some(ui)) => {
            let ids = restorable_ids(&ui);
            ui.restore(ids);
        }
        ("select", Some(ui)) => {
            ui.set_select(true);
            for id in restorable_ids(&ui) {
                ui.selected.borrow_mut().insert(id);
            }
            ui.rebuild();
        }
        ("restore-selected", Some(ui)) => ui.restore(ui.selected_ids()),
        ("clear", Some(ui)) => {
            let st = store(&ui.shared);
            let _ = st.history_clear();
            ui.rebuild();
        }
        ("close", Some(ui)) => {
            ui.dialog.close();
        }
        (_, None) => eprintln!("history_error: the History dialog is not open"),
        _ => eprintln!("history_error: unknown command {cmd}"),
    }
}

fn restorable_ids(ui: &Rc<Ui>) -> Vec<u64> {
    let mut f = ui.filter.borrow().clone();
    f.book = ui.scope;
    store(&ui.shared)
        .history(&f, SHOWN)
        .unwrap_or_default()
        .iter()
        .filter(|e| restorable(e.action, ui.outcome(e)))
        .map(|e| e.id)
        .collect()
}

fn journal_first(ui: &Rc<Ui>) -> Option<String> {
    let mut f = ui.filter.borrow().clone();
    f.book = ui.scope;
    let es = store(&ui.shared).history(&f, 1).ok()?;
    es.last().map(|e| {
        format!(
            "{} {} by {} book={} text={:?}",
            e.action.as_str(),
            e.kind.as_str(),
            e.source.as_db(),
            e.book,
            e.record.excerpt(60)
        )
    })
}
