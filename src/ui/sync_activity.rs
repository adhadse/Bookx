//! *Sync activity*: which e-readers talked to the Bookx sync server and what they sent. One block of
//! groups used by the Sync window and by Preferences → Sync: books that were not found in the library
//! (only when there are some), **Connected devices** (a row per device with a status), **Recent
//! activity** (the newest rows, `Show All` for the rest). The text is `crate::sync_activity`; the
//! facts are the server's activity log and the sync ledger. See `docs/koreader-annotation-sync.md`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use bookx_serve::activity::{ActivityLog, MAX_EVENTS};
use relm4::adw::{self, prelude::*};
use relm4::gtk;

use super::share::follow;
use super::SharedRef;
use crate::annotation_sync::{ledger_path, Ledger};
use crate::i18n::{ntr, tr, trf};
use crate::sync_activity as model;

/// Widget name of the block: the Sync page test that looks for rows without an action skips it.
pub const BLOCK_NAME: &str = "sync-activity";
const SHOWN: usize = 5;

thread_local! {
    /// What the library window offers: read the books again, show the Notes view for a book.
    static HOOKS: RefCell<Option<(Rc<dyn Fn()>, Rc<dyn Fn(String)>)>> = const { RefCell::new(None) };
}

/// The library window says how to refresh its views and how to open Notes & Highlights for a book.
pub fn set_hooks(refresh: Rc<dyn Fn()>, show_notes: Rc<dyn Fn(String)>) {
    HOOKS.with(|h| *h.borrow_mut() = Some((refresh, show_notes)));
}

thread_local! {
    static OPEN_HOOK: RefCell<Option<Rc<dyn Fn(u64)>>> = const { RefCell::new(None) };
}

/// The library window says how to open a book in the reader (the toast of a sync offers it).
pub fn set_open_hook(open: Rc<dyn Fn(u64)>) {
    OPEN_HOOK.with(|h| *h.borrow_mut() = Some(open));
}

/// Opens a book in the reader (nothing when the library window is not there).
pub fn open_book(id: u64) {
    if let Some(f) = OPEN_HOOK.with(|h| h.borrow().clone()) {
        f(id);
    }
}

/// Runs `f` (a redraw of lists) and puts every scrolled window below `root` back where it was, so a
/// live update does not throw the reader to the top of the list. The content of a rebuilt list has
/// no height until it is laid out, so the position is put back again when the window is idle.
pub fn keep_scroll(root: &gtk::Widget, f: impl FnOnce()) {
    let mut saved: Vec<(gtk::Adjustment, f64)> = Vec::new();
    fn collect(w: &gtk::Widget, out: &mut Vec<(gtk::Adjustment, f64)>) {
        if let Some(sw) = w.downcast_ref::<gtk::ScrolledWindow>() {
            let a = sw.vadjustment();
            out.push((a.clone(), a.value()));
        }
        let mut c = w.first_child();
        while let Some(x) = c {
            collect(&x, out);
            c = x.next_sibling();
        }
    }
    collect(root, &mut saved);
    f();
    let put_back = move |saved: &[(gtk::Adjustment, f64)]| {
        for (a, v) in saved {
            if (a.value() - v).abs() > 0.5 {
                a.set_value(*v);
            }
        }
    };
    put_back(&saved);
    relm4::gtk::glib::idle_add_local_once(move || put_back(&saved));
}

/// Opens Notes & Highlights narrowed to `book_title`.
pub fn show_notes(book_title: &str) {
    if let Some(f) = HOOKS.with(|h| h.borrow().as_ref().map(|h| h.1.clone())) {
        f(book_title.to_string());
    }
}

/// Everything the block shows, read at one moment.
pub struct Data {
    pub devices: Vec<model::DeviceRow>,
    pub events: Vec<model::ActivityRow>,
    pub today_line: String,
    pub unmatched: Vec<(String, String)>,
    pub address: Option<String>,
}

/// The activity log of the running server, else the file it left (so the history shows with the
/// server off).
pub fn log_of(shared: &SharedRef) -> Option<Arc<ActivityLog>> {
    if let Some(l) = shared.sharing.borrow().activity() {
        return Some(l);
    }
    let path = crate::sharing::activity_path(&crate::annotation_sync_ui::data_dir());
    path.is_file()
        .then(|| ActivityLog::open(&path).ok().map(Arc::new))
        .flatten()
}

use std::sync::Arc;

fn local_midnight(now: i64) -> i64 {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_opt(now, 0)
        .single()
        .and_then(|t| t.date_naive().and_hms_opt(0, 0, 0))
        .and_then(|d| Local.from_local_datetime(&d).earliest())
        .map_or(now - now.rem_euclid(86_400), |t| t.timestamp())
}

/// The numbers and texts of the block (no widgets).
pub fn read(shared: &SharedRef) -> Data {
    let now = chrono::Utc::now().timestamp();
    let today = local_midnight(now);
    let ledger = Ledger::load(&ledger_path(&crate::annotation_sync_ui::data_dir()));
    let known: HashMap<String, String> = {
        let lib = shared.lib.borrow();
        ledger
            .docs
            .iter()
            .filter_map(|(doc, id)| lib.get(*id).map(|b| (doc.clone(), b.meta.title.clone())))
            .collect()
    };
    let log = log_of(shared);
    let (devices, events) = match &log {
        Some(l) => (l.devices(), l.events(MAX_EVENTS)),
        None => (Vec::new(), Vec::new()),
    };
    let unmatched = ledger
        .unmatched
        .iter()
        .map(|(doc, u)| {
            let title = model::title_for(doc, &u.title, &HashMap::new());
            let sub = format!(
                "{} · {}",
                ntr(
                    "{} highlight not imported",
                    "{} highlights not imported",
                    u.count
                ),
                trf(
                    "from {}, hash {}",
                    &[&model::device_name(&u.device), &model::short_hash(doc)]
                )
            );
            (title, sub)
        })
        .collect();
    let address = crate::sharing::kosync_url(&shared.sharing.borrow().urls());
    Data {
        today_line: model::today_line(&events, today),
        devices: devices.iter().map(|d| model::device_row(d, now)).collect(),
        events: events
            .iter()
            .map(|e| {
                let mut e = e.clone();
                if let Some(l) = &log {
                    e.device = l.name_for(&e.device_id, &e.device);
                }
                model::activity_row(&e, now, &known)
            })
            .collect(),
        unmatched,
        address,
    }
}

/// The text of the block for tests and the `sync:activity` dev hook.
pub fn dump(shared: &SharedRef) -> String {
    let d = read(shared);
    let mut out = vec![format!(
        "devices={} events={} unmatched={}",
        d.devices.len(),
        d.events.len(),
        d.unmatched.len()
    )];
    for r in &d.devices {
        out.push(format!(
            "device: {} | {} | {}",
            r.title, r.subtitle, r.status
        ));
    }
    out.push(format!("today: {}", d.today_line));
    for r in &d.events {
        out.push(format!(
            "activity: {} | {}{}",
            r.title,
            r.subtitle,
            if r.problem { " | problem" } else { "" }
        ));
    }
    for (t, s) in &d.unmatched {
        out.push(format!("unmatched: {t} | {s}"));
    }
    out.join("\n")
}

fn status_label(text: &str, connected: bool) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class(if connected { "success" } else { "dim-label" });
    l.set_valign(gtk::Align::Center);
    l.set_wrap(true);
    l.set_xalign(1.0);
    l
}

/// The function that reads the facts again, filled in once the block exists.
type AfterRename = Rc<RefCell<Option<Weak<dyn Fn()>>>>;

fn device_row(r: &model::DeviceRow, shared: &SharedRef, after: AfterRename) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib_escape(&r.title))
        .subtitle(glib_escape(&r.subtitle))
        .build();
    row.set_title_lines(0);
    row.set_subtitle_lines(0);
    row.add_prefix(&gtk::Image::from_icon_name("tablet-symbolic"));
    row.add_suffix(&status_label(&r.status, r.connected));
    let rename = gtk::Button::from_icon_name("document-edit-symbolic");
    rename.add_css_class("flat");
    rename.set_valign(gtk::Align::Center);
    rename.set_tooltip_text(Some(&tr("Rename this device")));
    super::audit::label(&rename, &trf("Rename {}", &[&r.title]));
    {
        let (shared, id, current) = (shared.clone(), r.id.clone(), r.title.clone());
        let row = row.downgrade();
        rename.connect_clicked(move |_| {
            if let Some(row) = row.upgrade() {
                rename_dialog(&row, &shared, &id, &current, after.clone());
            }
        });
    }
    row.add_suffix(&rename);
    super::audit::label(&row, &format!("{}, {}, {}", r.title, r.subtitle, r.status));
    row
}

/// Asks for a name of the user's own for a device. Empty takes it back (the name the device
/// reports is shown again).
fn rename_dialog(
    parent: &adw::ActionRow,
    shared: &SharedRef,
    id: &str,
    current: &str,
    after: AfterRename,
) {
    let d = adw::AlertDialog::new(
        Some(&tr("Rename this device")),
        Some(&tr(
            "The name is only shown in Bookx. Leave it empty to show the name the device reports.",
        )),
    );
    let entry = gtk::Entry::new();
    entry.set_text(current);
    entry.set_activates_default(true);
    d.set_extra_child(Some(&entry));
    d.add_responses(&[("cancel", &tr("Cancel")), ("rename", &tr("Rename"))]);
    d.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("rename"));
    d.set_close_response("cancel");
    let (shared, id) = (shared.clone(), id.to_string());
    d.connect_response(None, move |_, resp| {
        if resp != "rename" {
            return;
        }
        if let Some(l) = log_of(&shared) {
            l.rename(&id, entry.text().as_str());
        }
        let f = after.borrow().as_ref().and_then(Weak::upgrade);
        if let Some(f) = f {
            f();
        }
    });
    d.present(Some(parent));
}

fn glib_escape(s: &str) -> String {
    relm4::gtk::glib::markup_escape_text(s).to_string()
}

fn activity_row(r: &model::ActivityRow) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib_escape(&r.title))
        .subtitle(glib_escape(&r.subtitle))
        .build();
    row.set_title_lines(0);
    row.set_subtitle_lines(0);
    if r.problem {
        let icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
        icon.add_css_class("warning");
        icon.set_accessible_role(gtk::AccessibleRole::Presentation);
        row.add_prefix(&icon);
    }
    row.set_activatable(true);
    let arrow = gtk::Image::from_icon_name("go-next-symbolic");
    arrow.set_accessible_role(gtk::AccessibleRole::Presentation);
    row.add_suffix(&arrow);
    row.set_tooltip_text(Some(&tr("Show this book in Notes and Highlights")));
    let title = r.title.clone();
    row.connect_activated(move |_| show_notes(&title));
    super::audit::label(&row, &format!("{}, {}", r.title, r.subtitle));
    row
}

fn empty_placeholder(address: Option<&str>) -> adw::ActionRow {
    let row = placeholder(&tr("Nothing has synced yet"));
    row.set_subtitle(&glib_escape(&model::empty_text(address)));
    row.set_subtitle_lines(0);
    row
}

fn placeholder(text: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(glib_escape(text)).build();
    row.set_title_lines(0);
    row.set_activatable(false);
    row.add_css_class("dim-label");
    row
}

/// The dialog with every recorded event (the newest 200).
pub fn show_all(parent: &gtk::Widget, shared: &SharedRef) {
    let d = read(shared);
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr("All sync activity"));
    dialog.set_content_width(520);
    dialog.set_content_height(640);
    let g = adw::PreferencesGroup::new();
    g.set_description(Some(&glib_escape(&d.today_line)));
    if d.events.is_empty() {
        g.add(&empty_placeholder(d.address.as_deref()));
    }
    for r in &d.events {
        g.add(&activity_row(r));
    }
    let page = adw::PreferencesPage::new();
    page.add(&g);
    let tb = adw::ToolbarView::new();
    tb.add_top_bar(&adw::HeaderBar::new());
    tb.set_content(Some(&page));
    dialog.set_child(Some(&tb));
    dialog.present(Some(parent));
}

struct Block {
    unmatched: adw::PreferencesGroup,
    devices: adw::PreferencesGroup,
    activity: adw::PreferencesGroup,
    rows: RefCell<Vec<(adw::PreferencesGroup, gtk::Widget)>>,
    last: RefCell<Vec<String>>,
}

impl Block {
    fn clear(&self) {
        for (g, w) in self.rows.borrow_mut().drain(..) {
            g.remove(&w);
        }
    }

    fn add(&self, g: &adw::PreferencesGroup, w: &impl IsA<gtk::Widget>) {
        g.add(w);
        self.rows.borrow_mut().push((g.clone(), w.clone().upcast()));
    }
}

/// The three groups and the function that reads the facts again.
pub struct Groups {
    pub unmatched: adw::PreferencesGroup,
    pub devices: adw::PreferencesGroup,
    pub activity: adw::PreferencesGroup,
    pub refresh: Rc<dyn Fn()>,
}

/// The groups of the block, with the function that reads the facts again (called by the timer, the
/// settings bus and the dev hooks). Rows are rebuilt only when their text changed.
pub fn build(shared: &SharedRef) -> Groups {
    let unmatched = adw::PreferencesGroup::new();
    unmatched.set_title(&tr("Books not found in your library"));
    unmatched.set_description(Some(&glib_escape(&tr(
        "These books are on your e-reader, but Bookx has no matching book, so their highlights were not imported. Add the book to your library, then press Sync now.",
    ))));
    let devices = adw::PreferencesGroup::new();
    devices.set_title(&tr("Connected devices"));
    let activity = adw::PreferencesGroup::new();
    activity.set_title(&tr("Recent activity"));
    for g in [&unmatched, &devices, &activity] {
        g.set_widget_name(BLOCK_NAME);
    }
    // the visible word is a child that screen readers skip: a button takes its name from its own label
    // first, so the longer name below would never reach them
    let all = gtk::Button::new();
    let shown = gtk::Label::new(Some(&tr("Show All")));
    shown.update_state(&[gtk::accessible::State::Hidden(true)]);
    all.set_child(Some(&shown));
    all.add_css_class("flat");
    all.set_valign(gtk::Align::Center);
    super::audit::label(&all, &tr("Show all sync activity"));
    activity.set_header_suffix(Some(&all));
    let block = Rc::new(Block {
        unmatched,
        devices,
        activity,
        rows: RefCell::new(Vec::new()),
        last: RefCell::new(Vec::new()),
    });
    {
        let (sh, a) = (shared.clone(), activity_weak(&block.activity));
        all.connect_clicked(move |_| {
            if let Some(a) = a.upgrade() {
                show_all(a.upcast_ref(), &sh);
            }
        });
    }

    let again: AfterRename = Rc::new(RefCell::new(None));
    let refresh: Rc<dyn Fn()> = {
        let (shared, block, again) = (shared.clone(), block.clone(), again.clone());
        Rc::new(move || {
            let d = read(&shared);
            let mut sig: Vec<String> = Vec::new();
            sig.push(d.today_line.clone());
            sig.extend(
                d.devices
                    .iter()
                    .map(|r| format!("{}|{}|{}", r.title, r.subtitle, r.status)),
            );
            sig.extend(
                d.events
                    .iter()
                    .take(SHOWN)
                    .map(|r| format!("{}|{}", r.title, r.subtitle)),
            );
            sig.extend(d.unmatched.iter().map(|(a, b)| format!("{a}|{b}")));
            sig.push(format!("{}", d.events.len()));
            sig.push(d.address.clone().unwrap_or_default());
            if *block.last.borrow() == sig && !block.rows.borrow().is_empty() {
                return;
            }
            *block.last.borrow_mut() = sig;
            block.clear();
            block.unmatched.set_visible(!d.unmatched.is_empty());
            for (t, s) in &d.unmatched {
                let row = adw::ActionRow::builder()
                    .title(glib_escape(t))
                    .subtitle(glib_escape(s))
                    .build();
                row.set_subtitle_lines(0);
                let icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
                icon.add_css_class("warning");
                icon.set_accessible_role(gtk::AccessibleRole::Presentation);
                row.add_prefix(&icon);
                super::audit::label(&row, &format!("{t}, {s}"));
                block.add(&block.unmatched, &row);
            }
            if d.devices.is_empty() {
                block.add(
                    &block.devices,
                    &placeholder(&tr("No e-reader has connected to this computer yet.")),
                );
            }
            for r in &d.devices {
                block.add(&block.devices, &device_row(r, &shared, again.clone()));
            }
            block
                .activity
                .set_description(Some(&glib_escape(&d.today_line)));
            if d.events.is_empty() {
                block.add(&block.activity, &empty_placeholder(d.address.as_deref()));
            }
            for r in d.events.iter().take(SHOWN) {
                block.add(&block.activity, &activity_row(r));
            }
        })
    };
    *again.borrow_mut() = Some(Rc::downgrade(&refresh));
    refresh();
    follow(&block.devices, {
        let r = refresh.clone();
        move || r()
    });
    let weak = block.devices.downgrade();
    let timer = refresh.clone();
    relm4::gtk::glib::timeout_add_seconds_local(3, move || match weak.upgrade() {
        Some(c) => {
            if c.is_mapped() {
                timer();
            }
            relm4::gtk::glib::ControlFlow::Continue
        }
        None => relm4::gtk::glib::ControlFlow::Break,
    });
    Groups {
        unmatched: block.unmatched.clone(),
        devices: block.devices.clone(),
        activity: block.activity.clone(),
        refresh,
    }
}

fn activity_weak(g: &adw::PreferencesGroup) -> relm4::gtk::glib::WeakRef<adw::PreferencesGroup> {
    g.downgrade()
}

/// The three groups in a column (the Sync window).
pub fn column(shared: &SharedRef) -> (gtk::Box, Rc<dyn Fn()>) {
    let g = build(shared);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 18);
    col.append(&g.unmatched);
    col.append(&g.devices);
    col.append(&g.activity);
    (col, g.refresh)
}
