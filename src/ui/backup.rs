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

pub struct Panel {
    shared: SharedRef,
    notify: Notify,
    parent: gtk::Widget,
    /// After a restore file is ready: restore the catalogue from it.
    on_restore: Rc<dyn Fn(PathBuf)>,
    /// After the snapshot list on this computer changed.
    on_local_change: Rc<dyn Fn()>,
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
    progress: Arc<Mutex<(String, f64)>>,
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
    ) -> Rc<Panel> {
        let where_group = adw::PreferencesGroup::new();
        where_group.set_title(&tr("Where backups go"));
        where_group.set_description(Some(
            "Every backup is a snapshot of the catalogue kept on this computer. Add places to copy it to as well: an external drive, a network share, a cloud drive or a WebDAV server. Backups are one-way copies; Bookx does not sync a library between computers.",
        ));
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
            crate::secrets::caption_now(),
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
        let cancel_btn = button(&tr("Cancel"));
        cancel_btn.set_halign(gtk::Align::End);
        box_busy.append(&stage);
        box_busy.append(&bar);
        box_busy.append(&cancel_btn);
        box_busy.set_visible(false);

        let restore_group = adw::PreferencesGroup::new();
        restore_group.set_title(&tr("Restore from another place"));
        restore_group.set_description(Some(
            "Pick a destination to see the backups there. Encrypted ones ask for the passphrase if it is not saved. The current catalogue is saved first, so a restore can be undone.",
        ));
        let restore_pick = adw::ComboRow::new();
        restore_pick.set_title(&tr("Destination"));
        restore_group.add(&restore_pick);

        let p = Rc::new(Panel {
            shared: shared.clone(),
            notify,
            parent: parent.clone(),
            on_restore,
            on_local_change,
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
            progress: Arc::new(Mutex::new((String::new(), 0.0))),
            pass_row,
            exit_switch,
            daily_switch,
            found: RefCell::new(Vec::new()),
            kdf: Kdf::DEFAULT,
        });
        p.schedule_group.add(&p.box_busy);
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
                    p.stage.set_text(&tr("Cancelling…"));
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
            let r = row(
                &d.name,
                &format!(
                    "{}\n{}{note}",
                    bk::destination_subtitle(d, &found),
                    bk::status_text(d)
                ),
            );
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
            let edit = icon_button("document-edit-symbolic", &tr("Edit"));
            let del = icon_button(
                "user-trash-symbolic",
                &tr("Remove this place (the copies stay)"),
            );
            r.add_suffix(&on);
            r.add_suffix(&edit);
            r.add_suffix(&del);
            let weak = Rc::downgrade(self);
            let id = d.id.clone();
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

    /// Asks every destination what it holds, one worker each, and puts it under its name.
    fn load_stats(self: &Rc<Self>, dests: &[Destination]) {
        for d in dests.iter().filter(|d| d.enabled) {
            let d2 = d.clone();
            let weak = Rc::downgrade(self);
            glib::spawn_future_local(async move {
                let id = d2.id.clone();
                let r = gio::spawn_blocking(move || {
                    d2.target()
                        .and_then(|t| bookx_backup::summarize(t.as_ref()))
                        .map_err(|e| format!("{e:#}"))
                })
                .await
                .unwrap_or_else(|_| Err("stopped unexpectedly".into()));
                let Some(p) = weak.upgrade() else { return };
                let row = p.detail_rows.borrow().get(&id).cloned();
                let Some(row) = row else { return };
                let base = row.subtitle().unwrap_or_default().to_string();
                let line = match r {
                    Ok(s) => {
                        let mut t = bk::summary_line(s.count, s.bytes, s.newest);
                        if let Some(f) = s.free {
                            t.push_str(&format!(" · {} free", lh::size_text(f)));
                        }
                        eprintln!("backup: place {id} count={} ", s.count);
                        t
                    }
                    Err(e) => {
                        eprintln!("backup: place {id} unreachable");
                        trf("Cannot read this place now: {}", &[&e])
                    }
                };
                row.set_subtitle(&format!("{base}\n{}", esc(&line)));
            });
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

    // -- back up now -------------------------------------------------------------------------------

    /// A snapshot on this computer, then a copy to every switched-on place, with progress.
    /// `done` is called with (books, per-place results) when finished.
    pub fn run_now(self: &Rc<Self>, done: Rc<dyn Fn(Result<usize, String>)>) {
        if self.busy.replace(true) {
            return;
        }
        let store = self.shared.guard.borrow().store.clone();
        let root = self.shared.lib.borrow().root.clone();
        let (dests, pass) = {
            let c = self.shared.cfg.borrow();
            (
                c.health
                    .backup
                    .destinations
                    .iter()
                    .filter(|d| d.enabled)
                    .cloned()
                    .collect::<Vec<_>>(),
                c.health.backup.passphrase.clone(),
            )
        };
        let cancel = Arc::new(AtomicBool::new(false));
        *self.cancel.borrow_mut() = cancel.clone();
        let progress = self.progress.clone();
        *progress.lock().unwrap() = ("Saving a snapshot".into(), 0.02);
        self.box_busy.set_visible(true);
        self.cancel_btn.set_sensitive(!dests.is_empty());
        self.bar.set_fraction(0.0);
        self.stage.set_text(&tr("Saving a snapshot"));
        // the bar follows what the worker reports
        let weak = Rc::downgrade(self);
        let tick = glib::timeout_add_local(std::time::Duration::from_millis(120), {
            let progress = progress.clone();
            move || {
                let Some(p) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                if !p.busy.get() {
                    return glib::ControlFlow::Break;
                }
                let (t, f) = progress.lock().unwrap().clone();
                if !p.cancel.borrow().load(Ordering::Relaxed) {
                    p.stage.set_text(&t);
                }
                p.bar.set_fraction(f);
                glib::ControlFlow::Continue
            }
        });
        let kdf = self.kdf;
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let r = gio::spawn_blocking(move || {
                let snap = health::take_snapshot(&store, &root, SnapshotKind::Manual, Utc::now())
                    .map_err(|e| format!("{e:#}"))?;
                let n = dests.len().max(1) as f64;
                let seen = std::cell::Cell::new(0usize);
                let outcomes = bk::push_all(
                    &dests,
                    &pass,
                    &snap,
                    Utc::now(),
                    kdf,
                    &cancel,
                    &|name, p| {
                        if p.stage == bookx_backup::PushStage::Done {
                            seen.set(seen.get() + 1);
                        }
                        *progress.lock().unwrap() = (
                            format!("{}: {}", name, p.stage.label()),
                            0.05 + 0.95 * (seen.get() as f64 + p.fraction.min(0.99)) / n,
                        );
                    },
                );
                Ok::<_, String>((snap.books, outcomes))
            })
            .await
            .unwrap_or_else(|_| Err("the backup stopped unexpectedly".into()));
            tick.remove();
            let Some(p) = weak.upgrade() else { return };
            p.busy.set(false);
            p.box_busy.set_visible(false);
            match r {
                Err(e) => done(Err(e)),
                Ok((books, outcomes)) => {
                    p.finish(&outcomes);
                    (p.on_local_change)();
                    done(Ok(books));
                }
            }
        });
    }

    fn finish(self: &Rc<Self>, outcomes: &[bk::Outcome]) {
        let now = Utc::now();
        self.save_cfg(|c| bk::apply(&mut c.health.backup, outcomes, now));
        let ok = outcomes.iter().filter(|o| o.result.is_ok()).count();
        let bad: Vec<String> = outcomes
            .iter()
            .filter(|o| o.result.is_err() && !o.cancelled)
            .map(|o| format!("{}: {}", o.name, o.result.as_ref().unwrap_err()))
            .collect();
        for o in outcomes {
            match &o.result {
                Ok(r) => eprintln!(
                    "backup: copied {} encrypted={} verified={}",
                    o.name, r.encrypted, r.verified
                ),
                Err(_) if o.cancelled => eprintln!("backup: cancelled {}", o.name),
                Err(e) => eprintln!("backup: failed {} {e}", o.name),
            }
        }
        if outcomes.iter().any(|o| o.cancelled) {
            self.say(tr("Backup cancelled; the copy in progress was removed"));
        } else if !bad.is_empty() {
            self.say(trf("Could not copy to: {}", &[&bad.join("; ")]));
        } else if ok > 0 {
            self.say(trf("Copied to {}", &[&lh::plural(ok, "place")]));
        }
        self.refresh();
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

    /// Lists the backups at the destination chosen in the combo.
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
        let loading = row(&tr("Looking…"), &d.name);
        self.restore_group.add(&loading);
        self.restore_rows
            .borrow_mut()
            .push(loading.clone().upcast());
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let d2 = d.clone();
            let r = gio::spawn_blocking(move || {
                d2.target()
                    .and_then(|t| bookx_backup::list_backups(t.as_ref()))
                    .map(|v| {
                        v.into_iter()
                            .map(|(f, n)| (f.name, f.size, n))
                            .collect::<Vec<_>>()
                    })
                    .map_err(|e| format!("{e:#}"))
            })
            .await
            .unwrap_or_else(|_| Err("stopped unexpectedly".into()));
            let Some(p) = weak.upgrade() else { return };
            // another place was picked (or the list rebuilt) meanwhile
            if p.generation.get() != gen {
                return;
            }
            p.restore_group.remove(&loading);
            p.restore_rows.borrow_mut().clear();
            match r {
                Err(e) => {
                    let r = row(&tr("Cannot read this place"), &e);
                    p.restore_group.add(&r);
                    p.restore_rows.borrow_mut().push(r.upcast());
                }
                Ok(list) if list.is_empty() => {
                    let r = row(&tr("No backups here yet"), &d.name);
                    p.restore_group.add(&r);
                    p.restore_rows.borrow_mut().push(r.upcast());
                }
                Ok(list) => {
                    eprintln!("backup: remote list {} count={}", d.id, list.len());
                    for (name, size, n) in list.into_iter().take(12) {
                        let r = row(
                            &lh::when(n.at),
                            &format!(
                                "{} · {}{} · {}",
                                lh::plural(n.books, "book"),
                                n.kind.label(),
                                if n.encrypted { " · encrypted" } else { "" },
                                lh::size_text(size)
                            ),
                        );
                        let b = button(&tr("Restore"));
                        b.set_sensitive(!p.shared.lib.borrow().is_read_only());
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
                    }
                }
            }
            p.add_file_row();
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
        net_group.set_description(Some(
            "Enter the address of the share, for example smb://nas/books or sftp://me@server/home/me. It is mounted through GNOME (the same as Files → Other Locations); a password prompt may appear.",
        ));
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
        let dav_note = row("", crate::secrets::caption_now());
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
        let result = row("", "");
        result.set_visible(false);
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
            if let Some(k) = parts.iter().find_map(|p| p.strip_prefix("keep=")) {
                d.keep = k.parse().unwrap_or(10);
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
            "backup-reload" => {
                self.refresh();
                true
            }
            _ => false,
        }
    }

    fn dump(&self) {
        let c = self.shared.cfg.borrow();
        let b = &c.health.backup;
        eprintln!("backup_places: {}", b.destinations.len());
        for d in &b.destinations {
            eprintln!(
                "backup_place: {} | {} | enabled={} | encrypt={} | {}",
                d.name,
                d.status().label(),
                d.enabled,
                d.encrypt,
                match d.status() {
                    bookx_backup::DestStatus::Failed { error, .. } => error,
                    _ => String::new(),
                }
            );
        }
        eprintln!(
            "backup_remote_buttons: {}",
            self.restore_btns.borrow().len()
        );
        eprintln!("backup_on_exit: {}", b.on_exit);
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

// -- scheduled copies (on exit, once a day) --------------------------------------------------------------

/// The daily timer: every ten minutes, copy to the places that are due. Starts after `first` seconds.
pub fn start_timer(shared: &SharedRef, toast: Notify) {
    let weak = Rc::downgrade(shared);
    let busy = Rc::new(Cell::new(false));
    glib::timeout_add_seconds_local(600, move || {
        let Some(shared) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        tick(&shared, toast.clone(), busy.clone());
        glib::ControlFlow::Continue
    });
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
    if settings
        .due(bookx_backup::Trigger::Tick, Utc::now())
        .is_empty()
    {
        return;
    }
    busy.set(true);
    let store = shared.guard.borrow().store.clone();
    let shared = shared.clone();
    let (job, _) = crate::jobs::start(crate::jobs::Kind::Backup, "Scheduled backup", false);
    glib::spawn_future_local(async move {
        let out = gio::spawn_blocking(move || {
            // a fresh automatic snapshot when something changed, then copy the newest
            let _ = lh::auto_snapshot(&store, &root, &prefs);
            bk::run_due(
                &root,
                &settings,
                bookx_backup::Trigger::Tick,
                Utc::now(),
                Kdf::DEFAULT,
                &AtomicBool::new(false),
            )
        })
        .await
        .unwrap_or_default();
        busy.set(false);
        for o in &out {
            match &o.result {
                Ok(_) => crate::jobs::log(job, &format!("{}: copied", o.name)),
                Err(e) => crate::jobs::log(job, &format!("{}: {e}", o.name)),
            }
        }
        let failed = out.iter().filter(|o| o.result.is_err()).count();
        crate::jobs::finish(
            job,
            if failed > 0 {
                Err(format!("{failed} place(s) failed"))
            } else {
                Ok(())
            },
        );
        if out.is_empty() {
            return;
        }
        {
            let mut c = shared.cfg.borrow_mut();
            bk::apply(&mut c.health.backup, &out, Utc::now());
            c.save();
        }
        for o in &out {
            match &o.result {
                Ok(_) => eprintln!("backup: scheduled copy to {} ok", o.name),
                Err(e) => {
                    eprintln!("backup: scheduled copy to {} failed {e}", o.name);
                    toast(trf("Could not back up to {}: {}", &[&o.name, &e]));
                }
            }
        }
    });
}

/// Closing the app: copy to the places that are due, but give up after twenty seconds.
pub fn on_exit(shared: &SharedRef) {
    let (root, settings) = {
        let c = shared.cfg.borrow();
        (shared.lib.borrow().root.clone(), c.health.backup.clone())
    };
    let out = bk::run_due_within(
        &root,
        &settings,
        bookx_backup::Trigger::Exit,
        Utc::now(),
        std::time::Duration::from_secs(20),
    );
    if out.is_empty() {
        return;
    }
    let mut c = shared.cfg.borrow_mut();
    bk::apply(&mut c.health.backup, &out, Utc::now());
    c.save();
    for o in &out {
        match &o.result {
            Ok(_) => eprintln!("backup: exit copy to {} ok", o.name),
            Err(e) => eprintln!("backup: exit copy to {} failed {e}", o.name),
        }
    }
}
