//! Where backups go: the destinations (folder, drive, network share, cloud-drive mount, WebDAV),
//! their status, the schedule, "Back up now" with progress and cancel, and restoring from any
//! destination. Shown inside the Library Health window (`ui/health.rs`); the logic is in
//! `backups.rs` and the `bookx-backup` crate.
//!
//! Every network or disk access runs on a worker thread; the window only shows results.

use crate::i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bookx_backup::crypto::Kdf;
use bookx_backup::places::{self, Place, PlaceKind};
use bookx_backup::repo::{LibrarySource, Plan, RestoreMode};
use bookx_backup::{DestKind, Destination};
use bookx_store::health::{self, SnapshotKind};
use chrono::Utc;
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio, glib};

use super::{window_of, SharedRef};
use crate::backups as bk;
use crate::library_health as lh;

type Notify = Rc<dyn Fn(String)>;

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

fn button(label: &str) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.set_valign(gtk::Align::Center);
    b
}

fn icon_button(icon: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(tip));
    b.set_valign(gtk::Align::Center);
    b.add_css_class("flat");
    b
}

/// A line of text that is not selectable markup.
fn row(title: &str, subtitle: &str) -> adw::ActionRow {
    let r = adw::ActionRow::new();
    r.set_title(&esc(title));
    r.set_subtitle(&esc(subtitle));
    r.set_subtitle_lines(4);
    r
}

fn autoconfirm() -> bool {
    !crate::devtools::script().is_empty()
}

/// What the worker of a long operation reports and the window shows.
#[derive(Debug, Clone, Default)]
struct Prog {
    stage: String,
    detail: String,
    fraction: f64,
}

/// What the developer script chose for the next restore (the real window asks the user).
#[derive(Debug, Clone, Default)]
struct DevRestore {
    missing_only: bool,
    target: Option<PathBuf>,
}

/// The widgets of the open restore dialog, for the developer script.
#[derive(Clone)]
struct Dlg {
    mode: adw::ComboRow,
    preview_btn: gtk::Button,
    restore: gtk::Button,
    preview: adw::ActionRow,
}

/// The lines under a place: what it holds, what the next copy does, the last check.
#[derive(Debug, Clone, Default)]
struct PlaceExtra {
    stats: String,
    plan: String,
    note: String,
}

/// A long operation in progress: its stop flag, its entry in the Jobs panel and the timer that
/// keeps the window and the panel up to date.
struct Op {
    cancel: Arc<AtomicBool>,
    job: u64,
    tick: glib::SourceId,
    progress: Arc<Mutex<Prog>>,
}

/// Keeps the Jobs panel entry in step with what the worker reports, and passes a Cancel from the panel
/// on to the worker. `extra` updates whatever else shows the progress.
fn watch(
    job: u64,
    token: crate::jobs::CancelToken,
    cancel: Arc<AtomicBool>,
    prog: Arc<Mutex<Prog>>,
    extra: impl Fn(&Prog, bool) -> bool + 'static,
) -> glib::SourceId {
    glib::timeout_add_local(std::time::Duration::from_millis(150), move || {
        if token.is_cancelled() {
            cancel.store(true, Ordering::Relaxed);
        }
        let p = prog.lock().unwrap().clone();
        let asked = cancel.load(Ordering::Relaxed);
        if !asked {
            let text = if p.detail.is_empty() {
                p.stage.clone()
            } else {
                format!("{} · {}", p.stage, p.detail)
            };
            crate::jobs::progress(job, Some(p.fraction), &text);
        }
        if extra(&p, asked) {
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        }
    })
}

pub struct Panel {
    shared: SharedRef,
    notify: Notify,
    parent: gtk::Widget,
    /// After a restore file is ready: restore the catalogue from it.
    on_restore: Rc<dyn Fn(PathBuf)>,
    /// After the snapshot list on this computer changed.
    on_local_change: Rc<dyn Fn()>,
    /// After the catalogue of the open library was replaced by a restore.
    on_reloaded: Rc<dyn Fn()>,
    banner: adw::ActionRow,
    estimates: RefCell<HashMap<String, Result<Plan, String>>>,
    bases: RefCell<HashMap<String, String>>,
    extras: RefCell<HashMap<String, PlaceExtra>>,
    dev_restore: RefCell<DevRestore>,
    dlg: RefCell<Option<Dlg>>,
    points: RefCell<Vec<(Destination, bk::repo::RestorePoint, bool)>>,
    detail: gtk::Label,
    job: Cell<Option<u64>>,
    pub where_group: adw::PreferencesGroup,
    pub schedule_group: adw::PreferencesGroup,
    pub restore_group: adw::PreferencesGroup,
    rows: RefCell<Vec<gtk::Widget>>,
    detail_rows: RefCell<HashMap<String, adw::ActionRow>>,
    restore_rows: RefCell<Vec<gtk::Widget>>,
    restore_btns: RefCell<Vec<gtk::Button>>,
    restore_pick: adw::ComboRow,
    generation: Cell<u64>,
    box_busy: gtk::Box,
    stage: gtk::Label,
    bar: gtk::ProgressBar,
    cancel_btn: gtk::Button,
    cancel: RefCell<Arc<AtomicBool>>,
    busy: Cell<bool>,
    progress: Arc<Mutex<Prog>>,
    pass_row: adw::PasswordEntryRow,
    exit_switch: adw::SwitchRow,
    daily_switch: adw::SwitchRow,
    found: RefCell<Vec<Place>>,
    kdf: Kdf,
}

impl Panel {
    pub fn new(
        parent: &gtk::Widget,
        shared: &SharedRef,
        notify: Notify,
        on_restore: Rc<dyn Fn(PathBuf)>,
        on_local_change: Rc<dyn Fn()>,
        on_reloaded: Rc<dyn Fn()>,
    ) -> Rc<Panel> {
        let where_group = adw::PreferencesGroup::new();
        where_group.set_title(&tr("Where backups go"));
        where_group.set_description(Some(&format!(
            "{} {}",
            bk::backup_scope_description(),
            bk::backup_incremental_note()
        )));
        // First row: say what a backup is before the user can run one. It is information only
        // and is not part of `rows`, so refreshing the places never moves or removes it.
        let scope = row(&bk::backup_scope_title(), &bk::backup_scope_subtitle());
        scope.set_selectable(false);
        where_group.add(&scope);
        let banner = adw::ActionRow::new();
        banner.set_title(&tr("Include your books in backups"));
        banner.set_subtitle_lines(4);
        banner.set_visible(false);
        banner.add_css_class("card");
        banner.add_prefix(&gtk::Image::from_icon_name("dialog-information-symbolic"));
        let banner_btn = button(&tr("Back Up"));
        banner_btn.add_css_class("suggested-action");
        super::audit::label(&banner_btn, &tr("Make the first backup of your books"));
        banner.add_suffix(&banner_btn);
        where_group.add(&banner);
        let schedule_group = adw::PreferencesGroup::new();
        schedule_group.set_title(&tr("Copies to other places"));
        let cfg = shared.cfg.borrow();
        let b = &cfg.health.backup;
        let exit_switch = adw::SwitchRow::new();
        exit_switch.set_title(&tr("Copy when Bookx closes"));
        exit_switch.set_active(b.on_exit);
        let daily_switch = adw::SwitchRow::new();
        daily_switch.set_title(&tr("Copy once a day while Bookx is open"));
        daily_switch.set_active(b.daily);
        let pass_row = adw::PasswordEntryRow::new();
        pass_row.set_title(&tr("Backup passphrase (for encrypted places)"));
        pass_row.set_text(&b.passphrase);
        drop(cfg);
        let pass_note = row(
            &tr("If you forget this passphrase, encrypted backups cannot be opened"),
            &crate::secrets::caption_now(),
        );
        schedule_group.add(&exit_switch);
        schedule_group.add(&daily_switch);
        schedule_group.add(&pass_row);
        schedule_group.add(&pass_note);

        let box_busy = gtk::Box::new(gtk::Orientation::Vertical, 6);
        box_busy.set_margin_top(8);
        box_busy.set_margin_bottom(8);
        box_busy.set_margin_start(12);
        box_busy.set_margin_end(12);
        let stage = gtk::Label::new(None);
        stage.set_xalign(0.0);
        let bar = gtk::ProgressBar::new();
        let detail = gtk::Label::new(None);
        detail.set_xalign(0.0);
        detail.add_css_class("dim-label");
        detail.add_css_class("caption");
        detail.set_wrap(true);
        let cancel_btn = button(&tr("Cancel"));
        cancel_btn.set_halign(gtk::Align::End);
        super::audit::label(&cancel_btn, &tr("Cancel the backup"));
        box_busy.append(&stage);
        box_busy.append(&bar);
        box_busy.append(&detail);
        box_busy.append(&cancel_btn);
        box_busy.set_visible(false);

        let restore_group = adw::PreferencesGroup::new();
        restore_group.set_title(&tr("Restore from another place"));
        restore_group.set_description(Some(&tr(
            "Pick a destination to see the backups there. Encrypted ones ask for the passphrase if it is not saved. The current catalogue is saved first, so a restore can be undone.",
        )));
        let restore_pick = adw::ComboRow::new();
        restore_pick.set_title(&tr("Destination"));
        restore_group.add(&restore_pick);

        let p = Rc::new(Panel {
            shared: shared.clone(),
            notify,
            parent: parent.clone(),
            on_restore,
            on_local_change,
            on_reloaded,
            banner: banner.clone(),
            estimates: RefCell::new(HashMap::new()),
            bases: RefCell::new(HashMap::new()),
            extras: RefCell::new(HashMap::new()),
            dev_restore: RefCell::new(DevRestore::default()),
            dlg: RefCell::new(None),
            points: RefCell::new(Vec::new()),
            detail,
            job: Cell::new(None),
            where_group,
            schedule_group,
            restore_group,
            rows: RefCell::new(Vec::new()),
            detail_rows: RefCell::new(HashMap::new()),
            restore_rows: RefCell::new(Vec::new()),
            restore_btns: RefCell::new(Vec::new()),
            restore_pick,
            generation: Cell::new(0),
            box_busy,
            stage,
            bar,
            cancel_btn,
            cancel: RefCell::new(Arc::new(AtomicBool::new(false))),
            busy: Cell::new(false),
            progress: Arc::new(Mutex::new(Prog::default())),
            pass_row,
            exit_switch,
            daily_switch,
            found: RefCell::new(Vec::new()),
            kdf: Kdf::DEFAULT,
        });
        p.schedule_group.add(&p.box_busy);
        {
            let weak = Rc::downgrade(&p);
            banner_btn.connect_clicked(move |_| {
                if let Some(p) = weak.upgrade() {
                    p.run_now(Rc::new(|_| {}));
                }
            });
        }
        p.wire();
        p.refresh();
        p
    }

    /// The groups for the window, in order.
    pub fn groups(&self) -> Vec<adw::PreferencesGroup> {
        vec![
            self.where_group.clone(),
            self.schedule_group.clone(),
            self.restore_group.clone(),
        ]
    }

    fn say(&self, m: impl Into<String>) {
        (self.notify)(m.into());
    }

    fn save_cfg(&self, f: impl FnOnce(&mut crate::config::Config)) {
        let mut c = self.shared.cfg.borrow_mut();
        f(&mut c);
        c.save();
    }

    fn wire(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.exit_switch.connect_active_notify({
            let weak = weak.clone();
            move |s| {
                if let Some(p) = weak.upgrade() {
                    p.save_cfg(|c| c.health.backup.on_exit = s.is_active());
                }
            }
        });
        self.daily_switch.connect_active_notify({
            let weak = weak.clone();
            move |s| {
                if let Some(p) = weak.upgrade() {
                    p.save_cfg(|c| c.health.backup.daily = s.is_active());
                }
            }
        });
        self.pass_row.connect_changed({
            let weak = weak.clone();
            move |r| {
                if let Some(p) = weak.upgrade() {
                    let t = r.text().to_string();
                    p.save_cfg(|c| c.health.backup.passphrase = t);
                }
            }
        });
        self.cancel_btn.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(p) = weak.upgrade() {
                    p.cancel.borrow().store(true, Ordering::Relaxed);
                    if let Some(j) = p.job.get() {
                        crate::jobs::cancel(j);
                    }
                    p.stage.set_text(&tr("Cancelling…"));
                    p.detail.set_text("");
                }
            }
        });
        self.restore_pick.connect_selected_notify({
            let weak = weak.clone();
            move |_| {
                if let Some(p) = weak.upgrade() {
                    p.show_remote_backups();
                }
            }
        });
    }

    // -- the list ---------------------------------------------------------------------------------

    fn clear_rows(&self) {
        for w in self.rows.borrow_mut().drain(..) {
            self.where_group.remove(&w);
        }
        self.detail_rows.borrow_mut().clear();
    }

    fn push(&self, w: &impl IsA<gtk::Widget>) {
        self.where_group.add(w);
        self.rows.borrow_mut().push(w.clone().upcast());
    }

    pub fn refresh(self: &Rc<Self>) {
        *self.found.borrow_mut() = places::detect_places();
        self.clear_rows();
        self.bases.borrow_mut().clear();
        let root = self.shared.lib.borrow().root.clone();
        let local = bk::local_summary(&root);
        let r = row(
            &tr("On this computer"),
            &format!(
                "{}\n{}",
                local.path.display(),
                bk::summary_line(local.count, local.bytes, local.newest)
            ),
        );
        r.add_prefix(&gtk::Image::from_icon_name("drive-harddisk-symbolic"));
        self.push(&r);

        let dests = self.shared.cfg.borrow().health.backup.destinations.clone();
        let found = self.found.borrow().clone();
        for (i, d) in dests.iter().enumerate() {
            let note = bk::privacy_note(d, &found)
                .map(|n| format!("\n{n}"))
                .unwrap_or_default();
            let base = format!(
                "{}\n{}\n{}{note}",
                bk::destination_subtitle(d, &found),
                bk::scope_label(d),
                bk::status_text(d)
            );
            let r = row(&d.name, &base);
            self.bases.borrow_mut().insert(d.id.clone(), base);
            r.add_prefix(&gtk::Image::from_icon_name(match d.kind {
                DestKind::Webdav => "network-server-symbolic",
                DestKind::Folder => match places::classify(Path::new(d.location.trim()), &found) {
                    PlaceKind::External => "media-removable-symbolic",
                    PlaceKind::Local => "folder-symbolic",
                    _ => "folder-remote-symbolic",
                },
            }));
            let on = gtk::Switch::new();
            on.set_valign(gtk::Align::Center);
            on.set_active(d.enabled);
            on.set_tooltip_text(Some(&tr("Copy to this place")));
            super::audit::label(&on, &trf("Copy to {}", &[&d.name]));
            let more = gtk::MenuButton::new();
            more.set_icon_name("view-more-symbolic");
            more.set_valign(gtk::Align::Center);
            more.add_css_class("flat");
            more.set_tooltip_text(Some(&tr("More for this place")));
            super::audit::label(&more, &trf("More for {}", &[&d.name]));
            let edit = icon_button("document-edit-symbolic", &tr("Edit"));
            super::audit::label(&edit, &trf("Edit {}", &[&d.name]));
            let del = icon_button(
                "user-trash-symbolic",
                &tr("Remove this place (the copies stay)"),
            );
            super::audit::label(&del, &trf("Remove {}", &[&d.name]));
            r.add_suffix(&on);
            r.add_suffix(&more);
            r.add_suffix(&edit);
            r.add_suffix(&del);
            let weak = Rc::downgrade(self);
            let id = d.id.clone();
            self.fill_menu(&more, &id);
            on.connect_active_notify({
                let (weak, id) = (weak.clone(), id.clone());
                move |s| {
                    if let Some(p) = weak.upgrade() {
                        let a = s.is_active();
                        p.save_cfg(|c| {
                            if let Some(d) =
                                c.health.backup.destinations.iter_mut().find(|d| d.id == id)
                            {
                                d.enabled = a;
                            }
                        });
                        p.refresh();
                    }
                }
            });
            edit.connect_clicked({
                let weak = weak.clone();
                move |_| {
                    if let Some(p) = weak.upgrade() {
                        p.edit_dialog(Some(i));
                    }
                }
            });
            del.connect_clicked({
                let (weak, id) = (weak.clone(), id.clone());
                move |_| {
                    if let Some(p) = weak.upgrade() {
                        p.ask_remove(&id);
                    }
                }
            });
            self.detail_rows
                .borrow_mut()
                .insert(d.id.clone(), r.clone());
            self.push(&r);
        }

        let add = adw::ActionRow::new();
        add.set_title(&tr("Add a place"));
        add.set_subtitle(&tr(
            "Folder, external drive, network share, cloud drive or WebDAV",
        ));
        add.set_activatable(true);
        add.add_prefix(&gtk::Image::from_icon_name("list-add-symbolic"));
        let weak = Rc::downgrade(self);
        add.connect_activated(move |_| {
            if let Some(p) = weak.upgrade() {
                p.edit_dialog(None);
            }
        });
        self.push(&add);

        let cloud = row(&tr("Google Drive, OneDrive, iCloud"), places::CLOUD_HELP);
        let open = button(&tr("Online Accounts"));
        open.connect_clicked(|_| open_online_accounts());
        cloud.add_suffix(&open);
        self.push(&cloud);

        self.fill_restore_pick(&dests);
        self.load_stats(&dests);
    }

    /// The actions of one place, in a small popover.
    fn fill_menu(self: &Rc<Self>, more: &gtk::MenuButton, id: &str) {
        let pop = gtk::Popover::new();
        let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let items: [(String, u8); 4] = [
            (tr("Back Up This Place"), 0),
            (tr("Check Backup"), 1),
            (tr("Check Backup in Depth"), 2),
            (tr("Remove Old Backups"), 3),
        ];
        for (label, what) in items {
            let b = gtk::Button::with_label(&label);
            b.add_css_class("flat");
            if what == 2 {
                b.set_tooltip_text(Some(&tr(
                    "Reads every file back and compares it. Slow on a network place.",
                )));
            }
            let (weak, id, pop2) = (Rc::downgrade(self), id.to_string(), pop.clone());
            b.connect_clicked(move |_| {
                pop2.popdown();
                let Some(p) = weak.upgrade() else { return };
                match what {
                    0 => p.run_places(Some(id.clone()), Rc::new(|_| {})),
                    1 => p.check_place(&id, false),
                    2 => p.check_place(&id, true),
                    _ => p.ask_clean(&id),
                }
            });
            col.append(&b);
        }
        pop.set_child(Some(&col));
        more.set_popover(Some(&pop));
    }

    fn passphrase(&self) -> String {
        self.shared.cfg.borrow().health.backup.passphrase.clone()
    }

    fn place(&self, id: &str) -> Option<Destination> {
        self.shared
            .cfg
            .borrow()
            .health
            .backup
            .destinations
            .iter()
            .find(|d| d.id == id)
            .cloned()
    }

    /// Rebuilds the text of a place from its fixed part and what was learned about it.
    fn render_place(&self, id: &str) {
        let row = self.detail_rows.borrow().get(id).cloned();
        let (Some(row), Some(base)) = (row, self.bases.borrow().get(id).cloned()) else {
            return;
        };
        let ex = self.extras.borrow().get(id).cloned().unwrap_or_default();
        let mut text = esc(&base);
        for line in [&ex.stats, &ex.plan, &ex.note] {
            if !line.is_empty() {
                text.push('\n');
                text.push_str(&esc(line));
            }
        }
        row.set_subtitle(&text);
    }

    fn set_extra(&self, id: &str, f: impl FnOnce(&mut PlaceExtra)) {
        f(self.extras.borrow_mut().entry(id.to_string()).or_default());
        self.render_place(id);
    }

    /// Asks every place what it holds and what the next backup would copy, one worker each, and puts
    /// it under its name. The estimate is shown before anything is copied.
    fn load_stats(self: &Rc<Self>, dests: &[Destination]) {
        self.extras.borrow_mut().clear();
        self.estimates.borrow_mut().clear();
        self.update_banner();
        let pass = self.passphrase();
        let (root, store) = (
            self.shared.lib.borrow().root.clone(),
            self.shared.guard.borrow().store.clone(),
        );
        for d in dests.iter().filter(|d| d.enabled) {
            let (d2, pass2) = (d.clone(), pass.clone());
            let (d3, pass3, root3, store3) = (d.clone(), pass.clone(), root.clone(), store.clone());
            let weak = Rc::downgrade(self);
            glib::spawn_future_local(async move {
                let (id, name) = (d2.id.clone(), d2.name.clone());
                let r = gio::spawn_blocking(move || {
                    let r: anyhow::Result<String> = match bk::repo::repo_summary(&d2, &pass2) {
                        Ok(Some(s)) => Ok(bk::repo::summary_text(&s)),
                        Ok(None) => d2
                            .target()
                            .and_then(|t| bookx_backup::summarize(t.as_ref()))
                            .map(|s| {
                                let mut t = if s.count == 0 {
                                    tr("No backups yet")
                                } else {
                                    trf(
                                        "{} (catalogue only)",
                                        &[&bk::summary_line(s.count, s.bytes, s.newest)],
                                    )
                                };
                                if let Some(f) = s.free {
                                    t.push_str(&format!(
                                        " · {}",
                                        trf("{} free", &[&lh::size_text(f)])
                                    ));
                                }
                                t
                            }),
                        Err(e) => Err(e),
                    };
                    r.map_err(|e| format!("{e:#}"))
                })
                .await
                .unwrap_or_else(|_| Err("stopped unexpectedly".into()));
                let Some(p) = weak.upgrade() else { return };
                let line = match r {
                    Ok(t) => {
                        eprintln!("backup_stats: {name} | {t}");
                        t
                    }
                    Err(e) => {
                        eprintln!("backup: place {id} unreachable");
                        trf("Cannot read this place now: {}", &[&e])
                    }
                };
                p.set_extra(&id, |x| x.stats = line);
                drop(p);
                // one request stream per place: what it holds first, then what the next copy would add
                let id = d3.id.clone();
                let id2 = id.clone();
                let (name, plan) = gio::spawn_blocking(move || {
                    let lib = LibrarySource::with_store(&root3, store3);
                    bk::repo::estimate_for(&lib, &[d3], &pass3, &AtomicBool::new(false))
                        .into_iter()
                        .find(|e| e.id == id2)
                        .map(|e| (e.name, e.plan))
                        .unwrap_or_else(|| (String::new(), Err("no answer".into())))
                })
                .await
                .unwrap_or_else(|_| (String::new(), Err("stopped unexpectedly".into())));
                let Some(p) = weak.upgrade() else { return };
                let text = match &plan {
                    Ok(pl) => bk::repo::plan_text(pl),
                    Err(e) => trf("Cannot estimate the next backup: {}", &[e]),
                };
                eprintln!("backup_plan: {name} | {text}");
                p.estimates.borrow_mut().insert(id.clone(), plan);
                p.set_extra(&id, |x| x.plan = text);
                p.update_banner();
            });
        }
    }

    /// The one-time row that says what the first copy is and that it waits for the user.
    fn update_banner(&self) {
        let waiting: Vec<Plan> = {
            let c = self.shared.cfg.borrow();
            let est = self.estimates.borrow();
            c.health
                .backup
                .destinations
                .iter()
                .filter(|d| d.enabled && !d.auto_ok && d.include_books)
                .filter_map(|d| est.get(&d.id).and_then(|r| r.as_ref().ok()).cloned())
                .collect()
        };
        match bk::repo::banner_text(&waiting) {
            Some(t) => {
                self.banner.set_subtitle(&esc(&t));
                self.banner.set_visible(true);
            }
            None => self.banner.set_visible(false),
        }
    }

    fn ask_remove(self: &Rc<Self>, id: &str) {
        let (weak, id) = (Rc::downgrade(self), id.to_string());
        let go = move || {
            if let Some(p) = weak.upgrade() {
                p.remove(&id);
            }
        };
        if autoconfirm() {
            go();
        } else {
            super::alert(
                &self.parent,
                "Stop copying to this place?",
                "The backups already there stay where they are. The saved password is removed.",
                "Remove",
                true,
                go,
            );
        }
    }

    fn remove(self: &Rc<Self>, id: &str) {
        self.save_cfg(|c| c.health.backup.destinations.retain(|d| d.id != id));
        crate::secrets::vault().forget(&crate::config::backup_password_key(id));
        self.refresh();
    }

    /// Adds or replaces a destination (the edit window, and the developer hook).
    pub fn upsert(self: &Rc<Self>, d: Destination) {
        self.save_cfg(|c| {
            let list = &mut c.health.backup.destinations;
            match list.iter_mut().find(|x| x.id == d.id) {
                Some(x) => *x = d,
                None => list.push(d),
            }
        });
        self.refresh();
    }

    // -- long operations: one at a time, with progress, Cancel and an entry in the Jobs panel ----------

    /// Starts an operation, or `None` when another one is running.
    fn op_begin(self: &Rc<Self>, title: &str, stage: &str) -> Option<Op> {
        if self.busy.replace(true) {
            return None;
        }
        let (job, token) = crate::jobs::start(crate::jobs::Kind::Backup, title, true);
        self.job.set(Some(job));
        let cancel = Arc::new(AtomicBool::new(false));
        *self.cancel.borrow_mut() = cancel.clone();
        let progress = self.progress.clone();
        *progress.lock().unwrap() = Prog {
            stage: stage.to_string(),
            detail: String::new(),
            fraction: 0.02,
        };
        self.box_busy.set_visible(true);
        self.cancel_btn.set_sensitive(true);
        self.bar.set_fraction(0.0);
        self.stage.set_text(stage);
        self.detail.set_text("");
        let weak = Rc::downgrade(self);
        let tick = watch(
            job,
            token,
            cancel.clone(),
            progress.clone(),
            move |p, asked| {
                let Some(w) = weak.upgrade() else {
                    return false;
                };
                if !w.busy.get() {
                    return false;
                }
                if !asked {
                    w.stage.set_text(&p.stage);
                    w.detail.set_text(&p.detail);
                }
                w.bar.set_fraction(p.fraction);
                true
            },
        );
        Some(Op {
            cancel,
            job,
            tick,
            progress,
        })
    }

    fn op_end(&self, op: Op, result: Result<(), String>) {
        op.tick.remove();
        crate::jobs::log(op.job, &self.stage.text());
        crate::jobs::finish(op.job, result);
        self.job.set(None);
        self.busy.set(false);
        self.box_busy.set_visible(false);
    }

    // -- back up now -------------------------------------------------------------------------------

    /// A snapshot on this computer, then a backup to every switched-on place, with progress. `done`
    /// is called with (books, what to tell the user) when finished.
    pub fn run_now(self: &Rc<Self>, done: Rc<dyn Fn(Result<(usize, String), String>)>) {
        self.run_places(None, done);
    }

    /// As [`Panel::run_now`], for one place (`Some(id)`) or all of them.
    pub fn run_places(
        self: &Rc<Self>,
        only: Option<String>,
        done: Rc<dyn Fn(Result<(usize, String), String>)>,
    ) {
        let store = self.shared.guard.borrow().store.clone();
        let root = self.shared.lib.borrow().root.clone();
        let (dests, pass) = {
            let c = self.shared.cfg.borrow();
            (
                c.health
                    .backup
                    .destinations
                    .iter()
                    .filter(|d| d.enabled && only.as_ref().is_none_or(|o| *o == d.id))
                    .cloned()
                    .collect::<Vec<_>>(),
                c.health.backup.passphrase.clone(),
            )
        };
        let Some(op) = self.op_begin(&tr("Backing up"), &tr("Saving a snapshot")) else {
            return;
        };
        let (cancel, progress) = (op.cancel.clone(), op.progress.clone());
        let kdf = self.kdf;
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let r = gio::spawn_blocking(move || {
                let snap = health::take_snapshot(&store, &root, SnapshotKind::Manual, Utc::now())
                    .map_err(|e| format!("{e:#}"))?;
                let lib = LibrarySource::with_store(&root, store);
                let names: Vec<String> = dests.iter().map(|d| d.name.clone()).collect();
                let n = dests.len().max(1) as f64;
                let outcomes = bk::repo::run_all(
                    &lib,
                    &dests,
                    &pass,
                    true,
                    Utc::now(),
                    kdf,
                    &cancel,
                    &|name, p| {
                        let i = names.iter().position(|x| x == name).unwrap_or(0) as f64;
                        *progress.lock().unwrap() = Prog {
                            stage: bk::repo::progress_stage(name, p),
                            detail: bk::repo::progress_detail(p),
                            fraction: (i + bk::repo::progress_fraction(p)) / n,
                        };
                    },
                );
                Ok::<_, String>((snap.books, outcomes))
            })
            .await
            .unwrap_or_else(|_| Err("the backup stopped unexpectedly".into()));
            let Some(p) = weak.upgrade() else { return };
            match r {
                Err(e) => {
                    p.op_end(op, Err(e.clone()));
                    done(Err(e));
                }
                Ok((books, outcomes)) => {
                    let failed = outcomes
                        .iter()
                        .filter(|o| o.result.is_err() && !o.cancelled)
                        .count();
                    p.op_end(
                        op,
                        if failed > 0 {
                            Err(trf("{} failed", &[&lh::plural(failed, "place")]))
                        } else {
                            Ok(())
                        },
                    );
                    let msg = p.finish(&outcomes);
                    (p.on_local_change)();
                    done(Ok((books, msg)));
                }
            }
        });
    }

    /// Records the outcomes, logs them and says what happened. Returns the message.
    fn finish(self: &Rc<Self>, outcomes: &[bk::repo::RepoOutcome]) -> String {
        let now = Utc::now();
        self.save_cfg(|c| bk::repo::apply_repo(&mut c.health.backup, outcomes, now));
        for o in outcomes {
            let enc = self.place(&o.id).is_some_and(|d| d.encrypt);
            match &o.result {
                Ok(r) => eprintln!(
                    "backup: copied {} encrypted={} verified={} new_files={} reused_files={} up_to_date={} left_out={}",
                    o.name,
                    enc,
                    r.verified,
                    r.new_files,
                    r.reused_files,
                    r.up_to_date,
                    r.too_big.len() + r.skipped.len()
                ),
                Err(_) if o.cancelled => eprintln!("backup: cancelled {}", o.name),
                Err(e) => eprintln!("backup: failed {} {e}", o.name),
            }
        }
        let msg = bk::repo::outcomes_text(outcomes);
        if !msg.is_empty() {
            self.say(msg.clone());
        }
        self.refresh();
        msg
    }

    // -- check a place, remove its old backups ---------------------------------------------------------

    fn check_place(self: &Rc<Self>, id: &str, deep: bool) {
        let Some(d) = self.place(id) else { return };
        let pass = self.passphrase();
        let Some(op) = self.op_begin(
            &trf("Checking the backup at {}", &[&d.name]),
            &tr("Checking the backup"),
        ) else {
            return;
        };
        let (cancel, progress) = (op.cancel.clone(), op.progress.clone());
        let (weak, id) = (Rc::downgrade(self), id.to_string());
        glib::spawn_future_local(async move {
            let d2 = d.clone();
            let r = gio::spawn_blocking(move || {
                bk::repo::check_place(&d2, &pass, deep, &cancel, &|done, total| {
                    let f = if total == 0 {
                        1.0
                    } else {
                        done as f64 / total as f64
                    };
                    let mut g = progress.lock().unwrap();
                    g.fraction = f.clamp(0.0, 1.0);
                    g.detail = trf("{} of {}", &[&lh::size_text(done), &lh::size_text(total)]);
                })
                .map_err(|e| format!("{e:#}"))
            })
            .await
            .unwrap_or_else(|_| Err("stopped unexpectedly".into()));
            let Some(p) = weak.upgrade() else { return };
            let cancelled = p.cancel.borrow().load(Ordering::Relaxed);
            let (text, res) = match r {
                Ok(rep) => {
                    let t = bk::repo::check_text(&rep);
                    eprintln!("backup: check {} ok={} | {t}", d.name, rep.ok());
                    let res = if rep.ok() { Ok(()) } else { Err(t.clone()) };
                    (t, res)
                }
                Err(_) if cancelled => (tr("Check cancelled"), Ok(())),
                Err(e) => {
                    eprintln!("backup: check {} failed {e}", d.name);
                    (trf("Could not check: {}", &[&e]), Err(e))
                }
            };
            p.op_end(op, res);
            p.set_extra(&id, |x| x.note = text.clone());
            p.say(text);
        });
    }

    fn ask_clean(self: &Rc<Self>, id: &str) {
        let (weak, id) = (Rc::downgrade(self), id.to_string());
        let go = move || {
            if let Some(p) = weak.upgrade() {
                p.clean_place(&id);
            }
        };
        if autoconfirm() {
            go();
        } else {
            super::alert(
                &self.parent,
                &tr("Remove old backups?"),
                &tr("Backups beyond the number this place keeps are removed, and so are the files only they needed. Manual backups and the newest ones stay."),
                &tr("Remove"),
                true,
                go,
            );
        }
    }

    fn clean_place(self: &Rc<Self>, id: &str) {
        let Some(d) = self.place(id) else { return };
        let pass = self.passphrase();
        let Some(op) = self.op_begin(
            &trf("Removing old backups at {}", &[&d.name]),
            &tr("Removing old backups"),
        ) else {
            return;
        };
        let cancel = op.cancel.clone();
        let (weak, id) = (Rc::downgrade(self), id.to_string());
        glib::spawn_future_local(async move {
            let d2 = d.clone();
            let r = gio::spawn_blocking(move || {
                bk::repo::clean_place(&d2, &pass, Utc::now(), &cancel).map_err(|e| format!("{e:#}"))
            })
            .await
            .unwrap_or_else(|_| Err("stopped unexpectedly".into()));
            let Some(p) = weak.upgrade() else { return };
            let text = match &r {
                Ok(rep) => {
                    let t = bk::repo::clean_text(rep);
                    eprintln!(
                        "backup: clean {} snapshots={} files={} | {t}",
                        d.name, rep.snapshots_removed, rep.files_removed
                    );
                    t
                }
                Err(e) => {
                    eprintln!("backup: clean {} failed {e}", d.name);
                    trf("Could not remove old backups: {}", &[e])
                }
            };
            p.op_end(op, r.map(|_| ()));
            p.say(text.clone());
            p.refresh();
            p.set_extra(&id, |x| x.note = text);
        });
    }

    // -- restore from a destination ---------------------------------------------------------------------

    fn add_file_row(self: &Rc<Self>) {
        let file_row = adw::ActionRow::new();
        file_row.set_title(&tr("Restore from a file"));
        file_row.set_subtitle(&tr("A .db snapshot or an encrypted .bookxbackup file"));
        let b = button(&tr("Choose File"));
        file_row.add_suffix(&b);
        let weak = Rc::downgrade(self);
        b.connect_clicked(move |_| {
            if let Some(p) = weak.upgrade() {
                p.pick_backup_file();
            }
        });
        self.restore_group.add(&file_row);
        self.restore_rows.borrow_mut().push(file_row.upcast());
    }

    fn fill_restore_pick(self: &Rc<Self>, dests: &[Destination]) {
        let names: Vec<&str> = dests.iter().map(|d| d.name.as_str()).collect();
        self.restore_pick
            .set_model(Some(&gtk::StringList::new(&names)));
        self.restore_pick.set_sensitive(!dests.is_empty());
        if dests.is_empty() {
            self.generation.set(self.generation.get() + 1);
            for w in self.restore_rows.borrow_mut().drain(..) {
                self.restore_group.remove(&w);
            }
            self.restore_btns.borrow_mut().clear();
            let r = row(
                &tr("No other places yet"),
                &tr("Add one above to restore from it. You can also restore from a backup file."),
            );
            self.restore_group.add(&r);
            self.restore_rows.borrow_mut().push(r.upcast());
            self.add_file_row();
        } else {
            self.show_remote_backups();
        }
    }

    fn selected_destination(&self) -> Option<Destination> {
        let i = self.restore_pick.selected() as usize;
        self.shared
            .cfg
            .borrow()
            .health
            .backup
            .destinations
            .get(i)
            .cloned()
    }

    /// Lists the backups at the place chosen in the combo: the snapshots of the book backup first,
    /// then the old whole-catalogue files that may sit next to them.
    fn show_remote_backups(self: &Rc<Self>) {
        // the rows after the combo are replaced; the combo itself stays
        let keep: Vec<gtk::Widget> = self.restore_rows.borrow_mut().drain(..).collect();
        for w in keep {
            self.restore_group.remove(&w);
        }
        self.restore_btns.borrow_mut().clear();
        let gen = self.generation.get() + 1;
        self.generation.set(gen);
        let Some(d) = self.selected_destination() else {
            return;
        };
        let loading = row(&tr("Looking"), &d.name);
        self.restore_group.add(&loading);
        self.restore_rows
            .borrow_mut()
            .push(loading.clone().upcast());
        let pass = self.passphrase();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let d2 = d.clone();
            let (legacy, repo) = gio::spawn_blocking(move || {
                let (legacy, repo) = bk::repo::listing(&d2, &pass);
                (
                    legacy
                        .into_iter()
                        .map(|(f, n)| (f.name, f.size, n))
                        .collect::<Vec<_>>(),
                    repo.map_err(|e| format!("{e:#}")),
                )
            })
            .await
            .unwrap_or_else(|_| (Vec::new(), Err("stopped unexpectedly".into())));
            let Some(p) = weak.upgrade() else { return };
            // another place was picked (or the list rebuilt) meanwhile
            if p.generation.get() != gen {
                return;
            }
            p.restore_group.remove(&loading);
            p.restore_rows.borrow_mut().clear();
            let read_only = p.shared.lib.borrow().is_read_only();
            let mut shown = 0usize;
            match repo {
                Ok(choices) => {
                    eprintln!(
                        "backup: remote list {} count={} legacy={}",
                        d.id,
                        choices.points.len(),
                        legacy.len()
                    );
                    if let Some(t) = choices.notes_until {
                        let r = row(
                            &tr("Notes are protected"),
                            &trf(
                                "Highlights, notes and bookmarks up to {} come back with a restore.",
                                &[&lh::when(t)],
                            ),
                        );
                        p.restore_group.add(&r);
                        p.restore_rows.borrow_mut().push(r.upcast());
                    }
                    *p.points.borrow_mut() = choices
                        .points
                        .iter()
                        .map(|pt| (d.clone(), pt.clone(), choices.encrypted))
                        .collect();
                    for pt in choices.points.iter().take(12) {
                        let r = row(
                            &lh::when(pt.at),
                            &format!(
                                "{} · {} · {}{}",
                                lh::plural(pt.books, "book"),
                                lh::plural(pt.files, "file"),
                                lh::size_text(pt.bytes),
                                if pt.manual {
                                    format!(" · {}", tr("manual"))
                                } else {
                                    String::new()
                                },
                            ),
                        );
                        let b = button(&tr("Restore"));
                        super::audit::label(
                            &b,
                            &trf("Restore the backup of {}", &[&lh::when(pt.at)]),
                        );
                        b.set_sensitive(!read_only);
                        r.add_suffix(&b);
                        let (weak, d, pt) = (Rc::downgrade(&p), d.clone(), pt.clone());
                        let enc = choices.encrypted;
                        b.connect_clicked(move |_| {
                            if let Some(p) = weak.upgrade() {
                                p.ask_restore_repo(d.clone(), pt.clone(), enc);
                            }
                        });
                        p.restore_btns.borrow_mut().push(b);
                        p.restore_group.add(&r);
                        p.restore_rows.borrow_mut().push(r.upcast());
                        shown += 1;
                    }
                    for (name, why) in choices.unreadable.iter().take(3) {
                        let r = row(&tr("A backup cannot be read"), &format!("{name}: {why}"));
                        p.restore_group.add(&r);
                        p.restore_rows.borrow_mut().push(r.upcast());
                    }
                }
                Err(e) if legacy.is_empty() => {
                    let r = row(&tr("Cannot read this place"), &e);
                    p.restore_group.add(&r);
                    p.restore_rows.borrow_mut().push(r.upcast());
                    shown += 1;
                }
                Err(e) => {
                    let r = row(&tr("Cannot read the backups of the books here"), &e);
                    p.restore_group.add(&r);
                    p.restore_rows.borrow_mut().push(r.upcast());
                }
            }
            for (name, size, n) in legacy.into_iter().take(12) {
                let r = row(
                    &lh::when(n.at),
                    &format!(
                        "{} · {}{} · {} · {}",
                        lh::plural(n.books, "book"),
                        n.kind.label(),
                        if n.encrypted { " · encrypted" } else { "" },
                        lh::size_text(size),
                        tr("catalogue only"),
                    ),
                );
                let b = button(&tr("Restore"));
                super::audit::label(
                    &b,
                    &trf("Restore the catalogue backup of {}", &[&lh::when(n.at)]),
                );
                b.set_sensitive(!read_only);
                r.add_suffix(&b);
                let (weak, d, name) = (Rc::downgrade(&p), d.clone(), name.clone());
                b.connect_clicked(move |_| {
                    if let Some(p) = weak.upgrade() {
                        p.ask_restore_remote(d.clone(), name.clone(), n.encrypted);
                    }
                });
                p.restore_btns.borrow_mut().push(b);
                p.restore_group.add(&r);
                p.restore_rows.borrow_mut().push(r.upcast());
                shown += 1;
            }
            if shown == 0 {
                let r = row(&tr("No backups here yet"), &d.name);
                p.restore_group.add(&r);
                p.restore_rows.borrow_mut().push(r.upcast());
            }
            p.add_file_row();
        });
    }

    // -- restoring a book backup ---------------------------------------------------------------------

    fn ask_restore_repo(self: &Rc<Self>, d: Destination, pt: bk::repo::RestorePoint, enc: bool) {
        self.with_passphrase(enc, move |p, pass| {
            let pass = pass.unwrap_or_default();
            if autoconfirm() {
                let dev = p.dev_restore.borrow().clone();
                let root = p.shared.lib.borrow().root.clone();
                let target = dev.target.unwrap_or(root);
                let mode = if dev.missing_only {
                    RestoreMode::MissingOnly
                } else {
                    RestoreMode::ReplaceAll
                };
                p.restore_flow(d.clone(), pass, pt.snapshot.clone(), target, mode, true);
            } else {
                p.restore_dialog(d.clone(), pass, pt.clone());
            }
        });
    }

    /// Asks where to restore to and how, shows what would change, and only then restores.
    fn restore_dialog(self: &Rc<Self>, d: Destination, pass: String, pt: bk::repo::RestorePoint) {
        let root = self.shared.lib.borrow().root.clone();
        let empty = self.shared.lib.borrow().books.is_empty();
        let dialog = adw::Dialog::new();
        dialog.set_title(&tr("Restore Backup"));
        dialog.set_content_width(480);
        let group = adw::PreferencesGroup::new();
        group.set_description(Some(&esc(&format!(
            "{} — {} · {} · {}",
            d.name,
            lh::when(pt.at),
            lh::plural(pt.books, "book"),
            lh::size_text(pt.bytes)
        ))));
        let into = adw::ComboRow::builder()
            .title(tr("Restore into"))
            .model(&gtk::StringList::new(&[
                &tr("This library"),
                &tr("Another folder"),
            ]))
            .build();
        into.set_subtitle(&esc(&root.display().to_string()));
        let folder = adw::EntryRow::new();
        folder.set_title(&tr("Folder"));
        folder.set_visible(false);
        let pick = button(&tr("Choose"));
        folder.add_suffix(&pick);
        let mode = adw::ComboRow::builder()
            .title(tr("What to restore"))
            .model(&gtk::StringList::new(&[
                &tr("Replace everything"),
                &tr("Only the missing books"),
            ]))
            .build();
        mode.set_subtitle(&tr(
            "Replacing saves the current catalogue first. Missing only never changes a file or the catalogue that is already there.",
        ));
        mode.set_selected(if empty { 0 } else { 1 });
        let preview = row(
            &tr("What will happen"),
            &tr("Press Preview to see it first"),
        );
        let preview_btn = button(&tr("Preview"));
        preview.add_suffix(&preview_btn);
        group.add(&into);
        group.add(&folder);
        group.add(&mode);
        group.add(&preview);
        let page = adw::PreferencesPage::new();
        page.add(&group);
        let restore = button(&tr("Restore"));
        restore.add_css_class("destructive-action");
        restore.set_sensitive(false);
        let cancel = button(&tr("Cancel"));
        let header = adw::HeaderBar::new();
        header.set_show_end_title_buttons(false);
        header.pack_start(&cancel);
        header.pack_end(&restore);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&page));
        dialog.set_child(Some(&view));
        {
            let dialog = dialog.clone();
            cancel.connect_clicked(move |_| {
                dialog.close();
            });
        }
        let target = {
            let (into, folder, root) = (into.clone(), folder.clone(), root.clone());
            move || -> Option<PathBuf> {
                if into.selected() == 0 {
                    Some(root.clone())
                } else {
                    let t = folder.text().trim().to_string();
                    (!t.is_empty()).then(|| PathBuf::from(t))
                }
            }
        };
        let mode_of = {
            let mode = mode.clone();
            move || {
                if mode.selected() == 0 {
                    RestoreMode::ReplaceAll
                } else {
                    RestoreMode::MissingOnly
                }
            }
        };
        let reset: Rc<dyn Fn()> = {
            let (restore, preview) = (restore.clone(), preview.clone());
            Rc::new(move || {
                restore.set_sensitive(false);
                preview.set_subtitle(&tr("Press Preview to see it first"));
            })
        };
        {
            let (folder2, reset1) = (folder.clone(), reset.clone());
            into.connect_selected_notify(move |c| {
                folder2.set_visible(c.selected() == 1);
                reset1();
            });
            let reset2 = reset.clone();
            mode.connect_selected_notify(move |_| reset2());
            let reset3 = reset.clone();
            folder.connect_changed(move |_| reset3());
        }
        {
            let (folder, parent) = (folder.clone(), self.parent.clone());
            pick.connect_clicked(move |_| {
                let dlg = gtk::FileDialog::builder()
                    .title(tr("Choose the Folder to Restore Into"))
                    .build();
                let folder = folder.clone();
                dlg.select_folder(
                    window_of(&parent).as_ref(),
                    gio::Cancellable::NONE,
                    move |res| {
                        if let Some(p) = res.ok().and_then(|f| f.path()) {
                            folder.set_text(&p.display().to_string());
                        }
                    },
                );
            });
        }
        {
            let (d, pass, pt, target, mode_of) = (
                d.clone(),
                pass.clone(),
                pt.clone(),
                target.clone(),
                mode_of.clone(),
            );
            let (preview, restore, btn) = (preview.clone(), restore.clone(), preview_btn.clone());
            btn.connect_clicked(move |b| {
                let Some(to) = target() else {
                    preview.set_subtitle(&tr("Choose a folder first"));
                    return;
                };
                b.set_sensitive(false);
                preview.set_subtitle(&tr("Looking"));
                let (d, pass, snap, mode) =
                    (d.clone(), pass.clone(), pt.snapshot.clone(), mode_of());
                let (preview, restore, b) = (preview.clone(), restore.clone(), b.clone());
                glib::spawn_future_local(async move {
                    let r = gio::spawn_blocking(move || {
                        bk::repo::restore_from_place(
                            &d,
                            &pass,
                            Some(&snap),
                            &to,
                            mode,
                            true,
                            None,
                            &AtomicBool::new(false),
                            &|_| {},
                        )
                        .map_err(|e| format!("{e:#}"))
                    })
                    .await
                    .unwrap_or_else(|_| Err("stopped unexpectedly".into()));
                    b.set_sensitive(true);
                    match r {
                        Ok(rep) => {
                            preview.set_subtitle(&esc(&bk::repo::restore_plan_text(&rep)));
                            restore.set_sensitive(true);
                        }
                        Err(e) => preview.set_subtitle(&esc(&trf("Cannot restore: {}", &[&e]))),
                    }
                });
            });
        }
        {
            let weak = Rc::downgrade(self);
            let dialog = dialog.clone();
            let (d, pass, pt) = (d.clone(), pass.clone(), pt.clone());
            restore.connect_clicked(move |_| {
                let (Some(p), Some(to)) = (weak.upgrade(), target()) else {
                    return;
                };
                dialog.close();
                p.restore_flow(
                    d.clone(),
                    pass.clone(),
                    pt.snapshot.clone(),
                    to,
                    mode_of(),
                    false,
                );
            });
        }
        *self.dlg.borrow_mut() = Some(Dlg {
            mode: mode.clone(),
            preview_btn: preview_btn.clone(),
            restore: restore.clone(),
            preview: preview.clone(),
        });
        let weak = Rc::downgrade(self);
        dialog.connect_closed(move |_| {
            if let Some(p) = weak.upgrade() {
                *p.dlg.borrow_mut() = None;
            }
        });
        dialog.present(Some(&self.parent));
    }

    /// Restores (after a dry run when `dry_first`: the developer script has no one to ask), with
    /// progress and Cancel, then reloads the library when it was the one that changed.
    fn restore_flow(
        self: &Rc<Self>,
        d: Destination,
        pass: String,
        snapshot: String,
        target: PathBuf,
        mode: RestoreMode,
        dry_first: bool,
    ) {
        let Some(op) = self.op_begin(
            &trf("Restoring from {}", &[&d.name]),
            &tr("Looking at the backup"),
        ) else {
            return;
        };
        let (cancel, progress) = (op.cancel.clone(), op.progress.clone());
        let into_library = target == self.shared.lib.borrow().root;
        let (weak, to2) = (Rc::downgrade(self), target.clone());
        glib::spawn_future_local(async move {
            let r = gio::spawn_blocking(move || {
                let report = |p: &bookx_backup::repo::RunProgress| {
                    *progress.lock().unwrap() = Prog {
                        stage: tr("Restoring files"),
                        detail: bk::repo::progress_detail(p),
                        fraction: bk::repo::progress_fraction(p),
                    };
                };
                let dry = if dry_first {
                    let r = bk::repo::restore_from_place(
                        &d,
                        &pass,
                        Some(&snapshot),
                        &target,
                        mode,
                        true,
                        None,
                        &cancel,
                        &|_| {},
                    )
                    .map_err(|e| format!("{e:#}"))?;
                    Some(r)
                } else {
                    None
                };
                let done = bk::repo::restore_from_place(
                    &d,
                    &pass,
                    Some(&snapshot),
                    &target,
                    mode,
                    false,
                    None,
                    &cancel,
                    &report,
                )
                .map_err(|e| format!("{e:#}"))?;
                Ok::<_, String>((dry, done))
            })
            .await
            .unwrap_or_else(|_| Err("stopped unexpectedly".into()));
            let Some(p) = weak.upgrade() else { return };
            let cancelled = p.cancel.borrow().load(Ordering::Relaxed);
            match r {
                Ok((dry, rep)) => {
                    p.op_end(op, Ok(()));
                    if let Some(dry) = dry {
                        eprintln!(
                            "backup: restore plan | {}",
                            bk::repo::restore_plan_text(&dry)
                        );
                    }
                    let mut msg = bk::repo::restore_done_text(&rep);
                    eprintln!(
                        "backup: restored files={} failed={} catalogue={:?} into={}",
                        rep.files_written,
                        rep.failed.len(),
                        rep.catalogue,
                        to2.display()
                    );
                    if into_library {
                        (p.on_reloaded)();
                    } else {
                        msg.push(' ');
                        msg.push_str(&trf(
                            "It is in {}. Open that folder as a library to use it.",
                            &[&to2.display()],
                        ));
                    }
                    p.say(msg);
                }
                Err(_) if cancelled => {
                    p.op_end(op, Ok(()));
                    p.say(tr("Restore cancelled; files already restored are complete"));
                }
                Err(e) => {
                    eprintln!("backup: restore failed {e}");
                    p.op_end(op, Err(e.clone()));
                    p.say(trf("Could not restore: {}", &[&e]));
                }
            }
        });
    }

    fn pick_backup_file(self: &Rc<Self>) {
        let dlg = gtk::FileDialog::builder()
            .title(tr("Choose a Backup File"))
            .build();
        let weak = Rc::downgrade(self);
        dlg.open(
            window_of(&self.parent).as_ref(),
            gio::Cancellable::NONE,
            move |res| {
                if let (Ok(f), Some(p)) = (res, weak.upgrade()) {
                    if let Some(path) = f.path() {
                        p.restore_file(path);
                    }
                }
            },
        );
    }

    /// Restores from a file the user picked.
    pub fn restore_file(self: &Rc<Self>, path: PathBuf) {
        let encrypted = std::fs::read(&path).is_ok_and(|b| bookx_backup::crypto::is_encrypted(&b));
        self.with_passphrase(encrypted, move |p, pass| {
            let root = p.shared.lib.borrow().root.clone();
            let weak = Rc::downgrade(&p);
            let path = path.clone();
            glib::spawn_future_local(async move {
                let tmp = root.join(health::BACKUP_DIR).join("restore-tmp");
                let t2 = tmp.clone();
                let r = gio::spawn_blocking(move || {
                    bookx_backup::open_file(&path, pass.as_deref(), &t2)
                        .map_err(|e| format!("{e:#}"))
                })
                .await
                .unwrap_or_else(|_| Err("stopped unexpectedly".into()));
                if let Some(p) = weak.upgrade() {
                    p.restore_ready(r, &tmp);
                }
            });
        });
    }

    fn ask_restore_remote(self: &Rc<Self>, d: Destination, name: String, encrypted: bool) {
        self.with_passphrase(encrypted, move |p, pass| {
            let root = p.shared.lib.borrow().root.clone();
            let weak = Rc::downgrade(&p);
            let (d, name) = (d.clone(), name.clone());
            glib::spawn_future_local(async move {
                let tmp = root.join(health::BACKUP_DIR).join("restore-tmp");
                let t2 = tmp.clone();
                let r = gio::spawn_blocking(move || {
                    d.target()
                        .and_then(|t| {
                            bookx_backup::fetch_to_file(t.as_ref(), &name, pass.as_deref(), &t2)
                        })
                        .map_err(|e| format!("{e:#}"))
                })
                .await
                .unwrap_or_else(|_| Err("stopped unexpectedly".into()));
                if let Some(p) = weak.upgrade() {
                    p.restore_ready(r, &tmp);
                }
            });
        });
    }

    fn restore_ready(self: &Rc<Self>, r: Result<PathBuf, String>, tmp: &Path) {
        match r {
            Ok(path) => {
                eprintln!("backup: downloaded for restore");
                (self.on_restore)(path);
                let _ = std::fs::remove_dir_all(tmp);
            }
            Err(e) => {
                eprintln!("backup: restore failed {e}");
                self.say(trf("Could not restore: {}", &[&e]));
            }
        }
    }

    /// Runs `go` with the passphrase: the saved one, or one typed now when the backup is
    /// encrypted and none is saved.
    fn with_passphrase(
        self: &Rc<Self>,
        needed: bool,
        go: impl Fn(Rc<Panel>, Option<String>) + 'static,
    ) {
        let saved = self.shared.cfg.borrow().health.backup.passphrase.clone();
        if !needed || !saved.is_empty() || autoconfirm() {
            go(self.clone(), (!saved.is_empty()).then_some(saved));
            return;
        }
        let d = adw::AlertDialog::new(
            Some(&tr("Passphrase needed")),
            Some(&tr(
                "This backup is encrypted. Enter the passphrase it was made with.",
            )),
        );
        let entry = gtk::PasswordEntry::new();
        entry.set_show_peek_icon(true);
        d.set_extra_child(Some(&entry));
        d.add_responses(&[("cancel", &tr("Cancel")), ("ok", &tr("Restore"))]);
        d.set_response_appearance("ok", adw::ResponseAppearance::Destructive);
        d.set_close_response("cancel");
        let go = Rc::new(go);
        let weak = Rc::downgrade(self);
        d.connect_response(None, move |_, resp| {
            if resp == "ok" {
                if let Some(p) = weak.upgrade() {
                    go(p, Some(entry.text().to_string()));
                }
            }
        });
        d.present(Some(&self.parent));
    }

    // -- add / edit a place ----------------------------------------------------------------------------------

    fn edit_dialog(self: &Rc<Self>, index: Option<usize>) {
        let existing = index.and_then(|i| {
            self.shared
                .cfg
                .borrow()
                .health
                .backup
                .destinations
                .get(i)
                .cloned()
        });
        let is_new = existing.is_none();
        let d0 = existing.unwrap_or_else(|| {
            let id = self.shared.cfg.borrow().health.backup.new_id();
            Destination {
                id,
                ..Destination::default()
            }
        });
        let dialog = adw::Dialog::new();
        dialog.set_title(
            &(if is_new {
                tr("Add a Place")
            } else {
                tr("Edit Place")
            }),
        );
        dialog.set_content_width(560);
        dialog.set_content_height(640);

        let kinds = gtk::StringList::new(&[
            "Folder or external drive",
            "Network share (SMB, NFS, SFTP…)",
            "Cloud drive (Google Drive, OneDrive…)",
            "WebDAV (Nextcloud, ownCloud…)",
        ]);
        let kind = adw::ComboRow::builder()
            .title(tr("Kind"))
            .model(&kinds)
            .build();
        let name = adw::EntryRow::new();
        name.set_title(&tr("Name"));
        name.set_text(&d0.name);

        // folder
        let folder_group = adw::PreferencesGroup::new();
        let folder = adw::EntryRow::new();
        folder.set_title(&tr("Folder"));
        let pick = button(&tr("Choose"));
        folder.add_suffix(&pick);
        folder_group.add(&folder);
        // network
        let net_group = adw::PreferencesGroup::new();
        net_group.set_description(Some(&tr(
            "Enter the address of the share, for example smb://nas/books or sftp://me@server/home/me. It is mounted through GNOME (the same as Files → Other Locations); a password prompt may appear.",
        )));
        let uri = adw::EntryRow::new();
        uri.set_title(&tr("Network address"));
        let connect = button(&tr("Connect"));
        uri.add_suffix(&connect);
        let net_note = row("", "");
        net_note.set_visible(false);
        let mounted = adw::ComboRow::new();
        mounted.set_title(&tr("Or pick a mounted share"));
        net_group.add(&uri);
        net_group.add(&net_note);
        net_group.add(&mounted);
        // cloud
        let cloud_group = adw::PreferencesGroup::new();
        cloud_group.set_description(Some(places::CLOUD_HELP));
        let clouds = adw::ComboRow::new();
        clouds.set_title(&tr("Connected accounts"));
        let accounts_btn = button(&tr("Online Accounts"));
        accounts_btn.connect_clicked(|_| open_online_accounts());
        clouds.add_suffix(&accounts_btn);
        cloud_group.add(&clouds);
        // webdav
        let dav_group = adw::PreferencesGroup::new();
        let url = adw::EntryRow::new();
        url.set_title(&tr(
            "Address, e.g. https://cloud.example.com/remote.php/dav/files/me/",
        ));
        url.set_text(if d0.kind == DestKind::Webdav {
            &d0.location
        } else {
            ""
        });
        let user = adw::EntryRow::new();
        user.set_title(&tr("User name"));
        user.set_text(&d0.user);
        let pass = adw::PasswordEntryRow::new();
        pass.set_title(&tr(
            "Password (an app password if you use two-factor sign-in)",
        ));
        pass.set_text(&d0.password);
        let dav_folder = adw::EntryRow::new();
        dav_folder.set_title(&tr("Folder on the server"));
        dav_folder.set_text(&d0.folder);
        let dav_note = row("", &crate::secrets::caption_now());
        dav_group.add(&url);
        dav_group.add(&user);
        dav_group.add(&pass);
        dav_group.add(&dav_note);
        dav_group.add(&dav_folder);
        // common
        let common = adw::PreferencesGroup::new();
        let keep = adw::SpinRow::with_range(1.0, 100.0, 1.0);
        keep.set_title(&tr("Automatic backups to keep here"));
        keep.set_subtitle(&tr(
            "Plus the newest of each of the last 7 days. Manual backups are never removed.",
        ));
        keep.set_value(d0.keep as f64);
        let encrypt = adw::SwitchRow::new();
        encrypt.set_title(&tr("Encrypt the copies"));
        encrypt.set_subtitle(&tr("With the backup passphrase (ChaCha20-Poly1305, key from Argon2id). Recommended for any place other people run."));
        encrypt.set_active(d0.encrypt);
        let books = adw::SwitchRow::new();
        books.set_title(&tr("Include book files"));
        books.set_subtitle(&tr(
            "Off: only the catalogue with your notes is copied, not the books",
        ));
        books.set_active(d0.include_books);
        let covers = adw::SwitchRow::new();
        covers.set_title(&tr("Include covers"));
        covers.set_active(d0.include_covers);
        let max_mb = adw::SpinRow::with_range(0.0, 100_000.0, 10.0);
        max_mb.set_title(&tr("Largest file to copy, in MB"));
        max_mb.set_subtitle(&tr(
            "0 means no limit. Bigger files are left out and listed.",
        ));
        max_mb.set_value(d0.max_file_mb as f64);
        let auto_first = adw::SwitchRow::new();
        auto_first.set_title(&tr("Make the first copy automatically"));
        auto_first.set_subtitle(&esc(&match self.estimates.borrow().get(&d0.id) {
            Some(Ok(p)) => format!(
                "{} {}",
                bk::repo::plan_text(p),
                tr("Off: nothing is copied until you press Back Up.")
            ),
            _ => tr("Off: nothing is copied until you press Back Up. Switch on to let Bookx make the first copy when it closes."),
        }));
        auto_first.set_active(d0.auto_ok);
        auto_first.set_visible(!d0.auto_ok);
        let result = row("", "");
        result.set_visible(false);
        let scope_group = adw::PreferencesGroup::new();
        scope_group.set_title(&tr("What to copy"));
        scope_group.add(&books);
        scope_group.add(&covers);
        scope_group.add(&max_mb);
        scope_group.add(&auto_first);
        common.add(&keep);
        common.add(&encrypt);
        common.add(&result);

        // initial state
        let found = self.found.borrow().clone();
        let net_places: Vec<Place> = found
            .iter()
            .filter(|p| matches!(p.kind, PlaceKind::Network(_)))
            .cloned()
            .collect();
        let cloud_places: Vec<Place> = found
            .iter()
            .filter(|p| {
                matches!(
                    p.kind,
                    PlaceKind::Cloud(_) | PlaceKind::WebDav | PlaceKind::ICloud
                )
            })
            .cloned()
            .collect();
        let labels = |v: &[Place], none: &str| -> gtk::StringList {
            if v.is_empty() {
                gtk::StringList::new(&[none])
            } else {
                let s: Vec<String> = v
                    .iter()
                    .map(|p| format!("{} — {}", p.kind.label(), p.name))
                    .collect();
                gtk::StringList::new(&s.iter().map(String::as_str).collect::<Vec<_>>())
            }
        };
        mounted.set_model(Some(&labels(&net_places, "No network share is mounted")));
        clouds.set_model(Some(&labels(&cloud_places, "No account is connected yet")));
        let initial_kind = match d0.kind {
            DestKind::Webdav => 3,
            DestKind::Folder => match places::classify(Path::new(d0.location.trim()), &found) {
                PlaceKind::Network(_) => 1,
                PlaceKind::Cloud(_) | PlaceKind::WebDav | PlaceKind::ICloud => 2,
                _ => 0,
            },
        };
        if d0.kind == DestKind::Folder {
            folder.set_text(&d0.location);
        }
        kind.set_selected(initial_kind);

        let show = {
            let (kind, fg, ng, cg, dg) = (
                kind.clone(),
                folder_group.clone(),
                net_group.clone(),
                cloud_group.clone(),
                dav_group.clone(),
            );
            move || {
                let k = kind.selected();
                fg.set_visible(k <= 2);
                ng.set_visible(k == 1);
                cg.set_visible(k == 2);
                dg.set_visible(k == 3);
            }
        };
        show();
        kind.connect_selected_notify(move |_| show());
        if is_new {
            // an off-site server should not see the catalogue in clear by default
            let encrypt = encrypt.clone();
            kind.connect_selected_notify(move |k| {
                if k.selected() == 3 {
                    encrypt.set_active(true);
                }
            });
        }

        // helpers fill the folder row
        {
            let (folder1, name1) = (folder.clone(), name.clone());
            let np = net_places.clone();
            mounted.connect_selected_notify(move |c| {
                if let Some(p) = np.get(c.selected() as usize) {
                    folder1.set_text(&p.path.join("Bookx backups").display().to_string());
                    if name1.text().is_empty() {
                        name1.set_text(&p.name);
                    }
                }
            });
            let (folder2, name2) = (folder.clone(), name.clone());
            let cp = cloud_places.clone();
            clouds.connect_selected_notify(move |c| {
                if let Some(p) = cp.get(c.selected() as usize) {
                    folder2.set_text(&p.path.join("Bookx backups").display().to_string());
                    if name2.text().is_empty() {
                        name2.set_text(&p.name);
                    }
                }
            });
        }
        {
            let folder = folder.clone();
            let parent = self.parent.clone();
            pick.connect_clicked(move |_| {
                let dlg = gtk::FileDialog::builder()
                    .title(tr("Choose the Backup Folder"))
                    .build();
                let folder = folder.clone();
                dlg.select_folder(
                    window_of(&parent).as_ref(),
                    gio::Cancellable::NONE,
                    move |res| {
                        if let Ok(f) = res {
                            if let Some(p) = f.path() {
                                folder.set_text(&p.display().to_string());
                            }
                        }
                    },
                );
            });
        }
        {
            let (uri, folder, note, parent, name) = (
                uri.clone(),
                folder.clone(),
                net_note.clone(),
                self.parent.clone(),
                name.clone(),
            );
            connect.connect_clicked(move |b| {
                let text = uri.text().to_string();
                if let Some(p) = bk::uri_problem(&text) {
                    note.set_visible(true);
                    note.set_title(&tr("Cannot connect"));
                    note.set_subtitle(&esc(&p));
                    return;
                }
                b.set_sensitive(false);
                note.set_visible(true);
                note.set_title(&tr("Connecting…"));
                note.set_subtitle("");
                mount_uri(&parent, text.trim(), {
                    let (folder, note, b, name) =
                        (folder.clone(), note.clone(), b.clone(), name.clone());
                    move |r| {
                        b.set_sensitive(true);
                        match r {
                            Ok(path) => {
                                note.set_title(&tr("Connected"));
                                note.set_subtitle(&esc(&path.display().to_string()));
                                folder.set_text(&path.join("Bookx backups").display().to_string());
                                if name.text().is_empty() {
                                    name.set_text(&tr("Network share"));
                                }
                            }
                            Err(e) => {
                                note.set_title(&tr("Could not connect"));
                                note.set_subtitle(&esc(&e));
                            }
                        }
                    }
                });
            });
        }

        // build the destination from the fields
        let collect = {
            let (books, covers, max_mb, auto_first) = (
                books.clone(),
                covers.clone(),
                max_mb.clone(),
                auto_first.clone(),
            );
            let (kind, name, folder, url, user, pass, dav_folder, keep, encrypt) = (
                kind.clone(),
                name.clone(),
                folder.clone(),
                url.clone(),
                user.clone(),
                pass.clone(),
                dav_folder.clone(),
                keep.clone(),
                encrypt.clone(),
            );
            let d0 = d0.clone();
            move || -> Destination {
                let k = kind.selected();
                let mut d = d0.clone();
                d.kind = if k == 3 {
                    DestKind::Webdav
                } else {
                    DestKind::Folder
                };
                d.location = if k == 3 {
                    url.text().trim().to_string()
                } else {
                    folder.text().trim().to_string()
                };
                d.user = user.text().trim().to_string();
                d.password = pass.text().to_string();
                d.folder = dav_folder.text().trim().to_string();
                d.keep = keep.value() as usize;
                d.encrypt = encrypt.is_active();
                d.include_books = books.is_active();
                d.include_covers = covers.is_active();
                d.max_file_mb = max_mb.value() as u64;
                d.auto_ok = d0.auto_ok || auto_first.is_active();
                let n = name.text().trim().to_string();
                d.name = if n.is_empty() {
                    d.location
                        .rsplit('/')
                        .find(|s| !s.is_empty())
                        .unwrap_or("Backups")
                        .to_string()
                } else {
                    n
                };
                d
            }
        };

        let save = button(&tr("Save"));
        save.add_css_class("suggested-action");
        let test = button(&tr("Test"));
        let cancel = button(&tr("Cancel"));
        let header = adw::HeaderBar::new();
        header.set_show_end_title_buttons(false);
        header.pack_start(&cancel);
        header.pack_end(&save);
        header.pack_end(&test);
        {
            let dialog = dialog.clone();
            cancel.connect_clicked(move |_| {
                dialog.close();
            });
        }
        {
            let (collect, result, test_btn) = (collect.clone(), result.clone(), test.clone());
            let weak = Rc::downgrade(self);
            test.connect_clicked(move |_| {
                let Some(p) = weak.upgrade() else { return };
                let d = collect();
                let has_pass = !p.shared.cfg.borrow().health.backup.passphrase.is_empty();
                result.set_visible(true);
                if let Some(e) = d.problem(has_pass) {
                    result.set_title(&tr("Cannot use this yet"));
                    result.set_subtitle(&esc(&e));
                    return;
                }
                result.set_title(&tr("Testing…"));
                result.set_subtitle("");
                test_btn.set_sensitive(false);
                let (result, test_btn) = (result.clone(), test_btn.clone());
                glib::spawn_future_local(async move {
                    let r = gio::spawn_blocking(move || {
                        d.target()
                            .and_then(|t| {
                                t.check()?;
                                let free = t.free_space();
                                Ok((t.label(), free))
                            })
                            .map_err(|e| format!("{e:#}"))
                    })
                    .await
                    .unwrap_or_else(|_| Err("stopped unexpectedly".into()));
                    test_btn.set_sensitive(true);
                    match r {
                        Ok((label, free)) => {
                            result.set_title(&tr("Works"));
                            result.set_subtitle(&esc(&trf(
                                "Can write to {}{}",
                                &[
                                    &label,
                                    &(free
                                        .map(|f| format!(" ({} free)", lh::size_text(f)))
                                        .unwrap_or_default()),
                                ],
                            )));
                        }
                        Err(e) => {
                            result.set_title(&tr("Does not work"));
                            result.set_subtitle(&esc(&e));
                        }
                    }
                });
            });
        }
        {
            let dialog = dialog.clone();
            let weak = Rc::downgrade(self);
            let (result, collect) = (result.clone(), collect.clone());
            save.connect_clicked(move |_| {
                let Some(p) = weak.upgrade() else { return };
                let d = collect();
                if d.location.is_empty() {
                    result.set_visible(true);
                    result.set_title(&tr("Cannot save"));
                    result.set_subtitle(&tr("Choose a folder or enter an address first"));
                    return;
                }
                p.upsert(d);
                dialog.close();
            });
        }

        let page = adw::PreferencesPage::new();
        let top = adw::PreferencesGroup::new();
        top.add(&kind);
        top.add(&name);
        page.add(&top);
        page.add(&folder_group);
        page.add(&net_group);
        page.add(&cloud_group);
        page.add(&dav_group);
        page.add(&scope_group);
        page.add(&common);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&page));
        dialog.set_child(Some(&view));
        dialog.present(Some(&self.parent));
    }

    // -- developer hooks (UI tests) ---------------------------------------------------------------------

    /// `health:backup-…` commands of the developer script.
    pub fn dev(self: &Rc<Self>, cmd: &str) -> bool {
        if let Some(spec) = cmd.strip_prefix("backup-add=") {
            // name|path[|encrypt][|keep=N]
            let parts: Vec<&str> = spec.split('|').collect();
            let id = self.shared.cfg.borrow().health.backup.new_id();
            let mut d = Destination::folder(&id, parts[0], parts.get(1).copied().unwrap_or(""));
            d.encrypt = parts.contains(&"encrypt");
            d.auto_ok = parts.contains(&"auto");
            d.include_books = !parts.contains(&"nobooks");
            d.include_covers = !parts.contains(&"nocovers");
            if let Some(k) = parts.iter().find_map(|p| p.strip_prefix("keep=")) {
                d.keep = k.parse().unwrap_or(10);
            }
            if let Some(m) = parts.iter().find_map(|p| p.strip_prefix("maxmb=")) {
                d.max_file_mb = m.parse().unwrap_or(0);
            }
            self.upsert(d);
            return true;
        }
        if let Some(spec) = cmd.strip_prefix("backup-add-dav=") {
            // name|url|user|password|folder
            let p: Vec<&str> = spec.split('|').collect();
            let id = self.shared.cfg.borrow().health.backup.new_id();
            let mut d = Destination::webdav(
                &id,
                p[0],
                p.get(1).copied().unwrap_or(""),
                p.get(2).copied().unwrap_or(""),
                p.get(3).copied().unwrap_or(""),
            );
            if let Some(f) = p.get(4) {
                d.folder = f.to_string();
            }
            d.encrypt = p.contains(&"encrypt");
            self.upsert(d);
            return true;
        }
        if let Some(pw) = cmd.strip_prefix("backup-passphrase=") {
            self.pass_row.set_text(pw);
            return true;
        }
        if let Some(n) = cmd.strip_prefix("backup-pick=") {
            self.restore_pick.set_selected(n.parse().unwrap_or(0));
            return true;
        }
        if let Some(m) = cmd.strip_prefix("backup-dialog-mode=") {
            if let Some(d) = self.dlg.borrow().as_ref() {
                d.mode.set_selected(if m == "missing" { 1 } else { 0 });
            }
            return true;
        }
        if let Some(m) = cmd.strip_prefix("backup-restore-mode=") {
            self.dev_restore.borrow_mut().missing_only = m == "missing";
            return true;
        }
        if let Some(t) = cmd.strip_prefix("backup-restore-to=") {
            self.dev_restore.borrow_mut().target = Some(PathBuf::from(t));
            return true;
        }
        if let Some(n) = cmd.strip_prefix("backup-place=") {
            // `backup-place=N` backs up only the Nth place (1-based)
            let id = n
                .parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .and_then(|i| {
                    self.shared
                        .cfg
                        .borrow()
                        .health
                        .backup
                        .destinations
                        .get(i)
                        .map(|d| d.id.clone())
                });
            if let Some(id) = id {
                self.run_places(Some(id), Rc::new(|_| {}));
            }
            return true;
        }
        if let Some(n) = cmd
            .strip_prefix("backup-check=")
            .or_else(|| cmd.strip_prefix("backup-check-deep="))
        {
            let id = n
                .parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .and_then(|i| {
                    self.shared
                        .cfg
                        .borrow()
                        .health
                        .backup
                        .destinations
                        .get(i)
                        .map(|d| d.id.clone())
                });
            if let Some(id) = id {
                self.check_place(&id, cmd.starts_with("backup-check-deep="));
            }
            return true;
        }
        if let Some(n) = cmd.strip_prefix("backup-clean=") {
            let id = n
                .parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .and_then(|i| {
                    self.shared
                        .cfg
                        .borrow()
                        .health
                        .backup
                        .destinations
                        .get(i)
                        .map(|d| d.id.clone())
                });
            if let Some(id) = id {
                self.ask_clean(&id);
            }
            return true;
        }
        match cmd {
            "backup-remove1" => {
                let id = self
                    .shared
                    .cfg
                    .borrow()
                    .health
                    .backup
                    .destinations
                    .first()
                    .map(|d| d.id.clone());
                if let Some(id) = id {
                    self.remove(&id);
                }
                true
            }
            "backup-restore-remote1" => {
                let b = self.restore_btns.borrow().first().cloned();
                match b {
                    Some(b) if b.is_sensitive() => b.emit_clicked(),
                    _ => eprintln!("backup: no remote backup to restore"),
                }
                true
            }
            "backup-cancel" => {
                self.cancel_btn.emit_clicked();
                true
            }
            "backup-dump" => {
                self.dump();
                true
            }
            "backup-scope" => {
                self.dump_scope();
                true
            }
            "backup-reload" => {
                self.refresh();
                true
            }
            "backup-restore-dialog" => {
                // the real window, not the script's shortcut
                let first = self.points.borrow().first().cloned();
                match first {
                    Some((d, pt, enc)) => {
                        let saved = self.passphrase();
                        let _ = enc;
                        self.restore_dialog(d, saved, pt);
                    }
                    None => eprintln!("backup: no snapshot to restore"),
                }
                true
            }
            "backup-dialog-preview" => {
                if let Some(d) = self.dlg.borrow().as_ref() {
                    d.preview_btn.emit_clicked();
                }
                true
            }
            "backup-dialog-restore" => {
                let b = self.dlg.borrow().as_ref().map(|d| d.restore.clone());
                match b {
                    Some(b) if b.is_sensitive() => b.emit_clicked(),
                    _ => eprintln!("backup: the dialog does not allow restoring yet"),
                }
                true
            }
            "backup-dialog-dump" => {
                match self.dlg.borrow().as_ref() {
                    Some(d) => eprintln!(
                        "backup_dialog: restore_sensitive={} mode={} | {}",
                        d.restore.is_sensitive(),
                        d.mode.selected(),
                        d.preview.subtitle().unwrap_or_default()
                    ),
                    None => eprintln!("backup_dialog: closed"),
                }
                true
            }
            "backup-journal" => {
                journal_now(&self.shared, self.kdf);
                true
            }
            _ => false,
        }
    }

    /// One line with the scope copy, for `backup-scope` and `backup-dump`.
    fn dump_scope(&self) {
        eprintln!(
            "backup_scope: title={} description={} included={} excluded={} incremental={}",
            bk::backup_scope_title(),
            bk::backup_scope_description(),
            bk::backup_included_label(),
            bk::backup_excluded_label(),
            bk::backup_incremental_note(),
        );
    }

    fn dump(&self) {
        self.dump_scope();
        let c = self.shared.cfg.borrow();
        let b = &c.health.backup;
        eprintln!("backup_places: {}", b.destinations.len());
        for d in &b.destinations {
            eprintln!(
                "backup_place: {} | {} | enabled={} | encrypt={} | {} | books={} covers={} max_mb={} auto={}",
                d.name,
                d.status().label(),
                d.enabled,
                d.encrypt,
                match d.status() {
                    bookx_backup::DestStatus::Failed { error, .. } => error,
                    _ => String::new(),
                },
                d.include_books,
                d.include_covers,
                d.max_file_mb,
                d.auto_ok
            );
        }
        eprintln!(
            "backup_remote_buttons: {}",
            self.restore_btns.borrow().len()
        );
        eprintln!("backup_on_exit: {}", b.on_exit);
        eprintln!(
            "backup_banner: visible={} | {}",
            self.banner.is_visible(),
            self.banner.subtitle().unwrap_or_default()
        );
        eprintln!("backup_job: {:?}", self.job.get().is_some());
        eprintln!("backup_busy: {}", self.busy.get());
        eprintln!("backup_pass_set: {}", !b.passphrase.is_empty());
    }
}

/// Opens GNOME Settings at Online Accounts.
fn open_online_accounts() {
    let ok = gio::AppInfo::create_from_commandline(
        "gnome-control-center online-accounts",
        None,
        gio::AppInfoCreateFlags::NONE,
    )
    .and_then(|a| a.launch(&[], gio::AppLaunchContext::NONE));
    if let Err(e) = ok {
        eprintln!("backup: cannot open Online Accounts: {e}");
    }
}

/// Mounts `smb://…` and friends through GIO and gives back the folder GVfs shows it as.
fn mount_uri(
    parent: &gtk::Widget,
    uri: &str,
    done: impl FnOnce(Result<PathBuf, String>) + 'static,
) {
    let file = gio::File::for_uri(uri);
    let op = gtk::MountOperation::new(window_of(parent).as_ref());
    let f2 = file.clone();
    file.mount_enclosing_volume(
        gio::MountMountFlags::NONE,
        Some(&op),
        gio::Cancellable::NONE,
        move |res| {
            if let Err(e) = &res {
                if !e.matches(gio::IOErrorEnum::AlreadyMounted) {
                    done(Err(e.message().to_string()));
                    return;
                }
            }
            match f2.path() {
                Some(p) => done(Ok(p)),
                None => done(Err(
                    "The share is mounted, but GNOME does not offer it as a folder (gvfs-fuse is not running). Open it in Files and pick it from “mounted share”."
                        .into(),
                )),
            }
        },
    );
}

// -- scheduled copies (on exit, once a day, notes every few minutes) -----------------------------------

/// The timers: every ten minutes look at what is due for the daily copy, every five minutes record
/// the notes that changed. Both stay quiet when nothing is switched on.
pub fn start_timer(shared: &SharedRef, toast: Notify) {
    let busy = Rc::new(Cell::new(false));
    let weak = Rc::downgrade(shared);
    glib::timeout_add_seconds_local(600, {
        let (toast, busy) = (toast.clone(), busy.clone());
        move || {
            let Some(shared) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            tick(&shared, toast.clone(), busy.clone());
            glib::ControlFlow::Continue
        }
    });
    let weak = Rc::downgrade(shared);
    let warned = Rc::new(Cell::new(false));
    glib::timeout_add_seconds_local(JOURNAL_EVERY_SECS, move || {
        let Some(shared) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        journal_tick(&shared, toast.clone(), busy.clone(), warned.clone());
        glib::ControlFlow::Continue
    });
}

/// How often the notes are recorded at the places that are on.
pub const JOURNAL_EVERY_SECS: u32 = 300;

/// The books of the open library, for the engine.
fn source_of(shared: &SharedRef) -> LibrarySource {
    LibrarySource::with_store(
        &shared.lib.borrow().root,
        shared.guard.borrow().store.clone(),
    )
}

/// One look at what is due. Also called by the developer hook.
pub fn tick(shared: &SharedRef, toast: Notify, busy: Rc<Cell<bool>>) {
    if busy.get() || shared.lib.borrow().is_read_only() {
        return;
    }
    let (root, settings, prefs) = {
        let c = shared.cfg.borrow();
        (
            shared.lib.borrow().root.clone(),
            c.health.backup.clone(),
            c.health.clone(),
        )
    };
    if bk::repo::scheduled_places(&settings, bookx_backup::Trigger::Tick, Utc::now()).is_empty() {
        return;
    }
    busy.set(true);
    let store = shared.guard.borrow().store.clone();
    let lib = source_of(shared);
    let shared = shared.clone();
    let (job, token) = crate::jobs::start(crate::jobs::Kind::Backup, &tr("Scheduled backup"), true);
    let cancel = Arc::new(AtomicBool::new(false));
    let prog = Arc::new(Mutex::new(Prog {
        stage: tr("Looking at your library"),
        ..Prog::default()
    }));
    let watcher = watch(job, token, cancel.clone(), prog.clone(), |_, _| true);
    glib::spawn_future_local(async move {
        let out = gio::spawn_blocking(move || {
            // a fresh automatic snapshot when something changed, then the places that are due
            let _ = lh::auto_snapshot(&store, &root, &prefs);
            let names: Vec<String> =
                bk::repo::scheduled_places(&settings, bookx_backup::Trigger::Tick, Utc::now())
                    .into_iter()
                    .map(|d| d.name)
                    .collect();
            let n = names.len().max(1) as f64;
            let dests =
                bk::repo::scheduled_places(&settings, bookx_backup::Trigger::Tick, Utc::now());
            bk::repo::run_all(
                &lib,
                &dests,
                &settings.passphrase,
                false,
                Utc::now(),
                Kdf::DEFAULT,
                &cancel,
                &|name, p| {
                    let i = names.iter().position(|x| x == name).unwrap_or(0) as f64;
                    *prog.lock().unwrap() = Prog {
                        stage: bk::repo::progress_stage(name, p),
                        detail: bk::repo::progress_detail(p),
                        fraction: (i + bk::repo::progress_fraction(p)) / n,
                    };
                },
            )
        })
        .await
        .unwrap_or_default();
        watcher.remove();
        busy.set(false);
        for o in &out {
            match &o.result {
                Ok(r) => {
                    crate::jobs::log(job, &format!("{}: {}", o.name, bk::repo::report_text(r)))
                }
                Err(e) => crate::jobs::log(job, &format!("{}: {e}", o.name)),
            }
        }
        let failed = out
            .iter()
            .filter(|o| o.result.is_err() && !o.cancelled)
            .count();
        crate::jobs::finish(
            job,
            if failed > 0 {
                Err(trf("{} failed", &[&lh::plural(failed, "place")]))
            } else {
                Ok(())
            },
        );
        if out.is_empty() {
            return;
        }
        {
            let mut c = shared.cfg.borrow_mut();
            bk::repo::apply_repo(&mut c.health.backup, &out, Utc::now());
            c.save();
        }
        for o in &out {
            match &o.result {
                Ok(_) => eprintln!("backup: scheduled copy to {} ok", o.name),
                Err(e) if o.cancelled => {
                    eprintln!("backup: scheduled copy to {} cancelled {e}", o.name)
                }
                Err(e) => {
                    eprintln!("backup: scheduled copy to {} failed {e}", o.name);
                    toast(trf("Could not back up to {}: {}", &[&o.name, &e]));
                }
            }
        }
    });
}

/// Records the notes that changed at the places that are on. Quiet unless it fails (once).
fn journal_tick(shared: &SharedRef, toast: Notify, busy: Rc<Cell<bool>>, warned: Rc<Cell<bool>>) {
    if busy.get() || shared.lib.borrow().is_read_only() {
        return;
    }
    let (places, pass) = {
        let c = shared.cfg.borrow();
        (
            bk::repo::journal_places(&c.health.backup),
            c.health.backup.passphrase.clone(),
        )
    };
    if places.is_empty() {
        return;
    }
    busy.set(true);
    let lib = source_of(shared);
    glib::spawn_future_local(async move {
        let failed = gio::spawn_blocking(move || {
            bk::repo::journal_all(&lib, &places, &pass, Utc::now(), Kdf::DEFAULT)
        })
        .await
        .unwrap_or_default();
        busy.set(false);
        for (name, why) in &failed {
            eprintln!("backup: journal to {name} failed {why}");
        }
        if let Some((name, why)) = failed.first() {
            if !warned.replace(true) {
                toast(trf(
                    "Your notes could not be copied to {}: {}",
                    &[name, why],
                ));
            }
        } else {
            warned.set(false);
        }
    });
}

/// The developer hook: record the notes now and say how it went.
fn journal_now(shared: &SharedRef, kdf: Kdf) {
    let (places, pass) = {
        let c = shared.cfg.borrow();
        (
            bk::repo::journal_places(&c.health.backup),
            c.health.backup.passphrase.clone(),
        )
    };
    let lib = source_of(shared);
    glib::spawn_future_local(async move {
        let names: Vec<String> = places.iter().map(|d| d.name.clone()).collect();
        let failed = gio::spawn_blocking(move || {
            bk::repo::journal_all(&lib, &places, &pass, Utc::now(), kdf)
        })
        .await
        .unwrap_or_default();
        for n in names {
            match failed.iter().find(|f| f.0 == n) {
                Some((_, why)) => eprintln!("backup: journal {n} failed {why}"),
                None => eprintln!("backup: journal {n} ok"),
            }
        }
    });
}

/// Closing the app: copy to the places that are due, record the notes, but give up after twenty seconds.
pub fn on_exit(shared: &SharedRef) {
    let (settings, lib) = (shared.cfg.borrow().health.backup.clone(), source_of(shared));
    let started = std::time::Instant::now();
    let limit = std::time::Duration::from_secs(20);
    let out = bk::repo::run_due_repo_within(
        &lib,
        &settings,
        bookx_backup::Trigger::Exit,
        Utc::now(),
        limit,
    );
    if !out.is_empty() {
        let mut c = shared.cfg.borrow_mut();
        bk::repo::apply_repo(&mut c.health.backup, &out, Utc::now());
        c.save();
        for o in &out {
            match &o.result {
                Ok(_) => eprintln!("backup: exit copy to {} ok", o.name),
                Err(e) => eprintln!("backup: exit copy to {} failed {e}", o.name),
            }
        }
    }
    // notes of places that were not copied just now (the copy already holds them for the others)
    let left = limit.saturating_sub(started.elapsed());
    if left >= std::time::Duration::from_secs(1) {
        let settings = shared.cfg.borrow().health.backup.clone();
        let done: Vec<&str> = out
            .iter()
            .filter(|o| o.result.is_ok())
            .map(|o| o.id.as_str())
            .collect();
        let places: Vec<Destination> = bk::repo::journal_places(&settings)
            .into_iter()
            .filter(|d| !done.contains(&d.id.as_str()))
            .collect();
        if !places.is_empty() {
            let failed = bk::repo::journal_within(&lib, &places, &settings.passphrase, left);
            for (name, why) in failed {
                eprintln!("backup: exit journal to {name} failed {why}");
            }
        }
    }
}
