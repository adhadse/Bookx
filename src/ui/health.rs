//! *Library Health*: backups and restore, the library check, Calibre import, the import folder,
//! history with undo, the trash and name tidying, in one window (roadmap PP-C-1/2/3/7/16,
//! PP-G-1/2/3/32/33). The logic is in `library_health.rs`, `bookx-store` and `bookx-core`; this file
//! only shows it and wires the buttons. Long work runs on worker threads.

use crate::i18n::{tr, trf};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use bookx_core::inbox::{self, Candidate, InboxState};
use bookx_core::layout::Layout;
use bookx_core::library::{AddOutcome, Library};
use bookx_store::health::{self, CheckOptions, Finding, Fix};
use bookx_store::{SqliteCatalog, SqliteStore, Stage};
use chrono::Utc;
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio, glib};

use super::{SharedRef, window_of};
use crate::backups as bk;
use crate::i18n::N_;
use crate::library_health as lh;

type Notify = Rc<dyn Fn(String)>;

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

fn row(title: &str, subtitle: &str) -> adw::ActionRow {
    let r = adw::ActionRow::new();
    r.set_title(&esc(title));
    r.set_subtitle(&esc(subtitle));
    r.set_subtitle_lines(3);
    r
}

fn button(label: &str) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.set_valign(gtk::Align::Center);
    b
}

/// A group whose rows are replaced as a whole.
#[derive(Clone)]
struct Dynamic {
    group: adw::PreferencesGroup,
    rows: Rc<RefCell<Vec<gtk::Widget>>>,
}

impl Dynamic {
    fn new(title: &str, description: &str) -> Dynamic {
        let group = adw::PreferencesGroup::new();
        group.set_title(&esc(title));
        group.set_description(Some(&esc(description)));
        Dynamic {
            group,
            rows: Rc::new(RefCell::new(Vec::new())),
        }
    }

    fn clear(&self) {
        for w in self.rows.borrow_mut().drain(..) {
            self.group.remove(&w);
        }
    }

    fn push(&self, w: &impl IsA<gtk::Widget>) {
        self.group.add(w);
        self.rows.borrow_mut().push(w.clone().upcast());
    }

    fn len(&self) -> usize {
        self.rows.borrow().len()
    }
}

/// Everything the window shows, shared by the button handlers (which hold it weakly).
struct Ctx {
    shared: SharedRef,
    notify: Notify,
    parent: gtk::Widget,
    // library
    status: Dynamic,
    // backups
    auto_switch: adw::SwitchRow,
    keep_spin: adw::SpinRow,
    backup_btn: gtk::Button,
    snapshots: Dynamic,
    restore_btns: RefCell<Vec<gtk::Button>>,
    panel: RefCell<Option<Rc<super::backup::Panel>>>,
    // check
    check_btn: gtk::Button,
    drift_switch: adw::SwitchRow,
    check_summary: adw::ActionRow,
    findings: Dynamic,
    found: RefCell<Vec<Finding>>,
    // calibre
    calibre_btn: gtk::Button,
    calibre_group: adw::PreferencesGroup,
    calibre_copy: adw::SwitchRow,
    calibre_progress: gtk::ProgressBar,
    calibre_report: adw::ActionRow,
    calibre_pick: RefCell<Option<PathBuf>>,
    // import folder
    inbox_folder: adw::ActionRow,
    inbox_scan: gtk::Button,
    inbox_add_all: gtk::Button,
    inbox: Dynamic,
    candidates: RefCell<Vec<Candidate>>,
    // history and trash
    undo_btn: gtk::Button,
    history: Dynamic,
    trash: Dynamic,
    trash_btns: RefCell<Vec<gtk::Button>>,
    empty_btn: gtk::Button,
    // names
    tidy_summary: adw::ActionRow,
    tidy_apply: gtk::Button,
    series: Dynamic,
    /// The tabs of the window (set once they are built).
    tabs: RefCell<Option<adw::ViewStack>>,
}

thread_local! {
    /// The window that is open, so the developer script can press its buttons.
    static OPEN: RefCell<Option<Rc<Ctx>>> = const { RefCell::new(None) };
}

fn autoconfirm() -> bool {
    !crate::devtools::script().is_empty()
}

fn confirm(
    ctx: &Ctx,
    heading: &str,
    body: &str,
    label: &str,
    destructive: bool,
    f: impl Fn() + 'static,
) {
    if autoconfirm() {
        f();
    } else {
        super::alert(&ctx.parent, heading, body, label, destructive, f);
    }
}

fn root_of(shared: &SharedRef) -> PathBuf {
    shared.lib.borrow().root.clone()
}

fn store_of(shared: &SharedRef) -> Arc<SqliteStore> {
    shared.guard.borrow().store.clone()
}

fn read_only(shared: &SharedRef) -> bool {
    shared.lib.borrow().is_read_only()
}

/// Reloads the books from the database after it was restored or imported into.
pub(super) fn reload_library(shared: &SharedRef) -> anyhow::Result<()> {
    let root = root_of(shared);
    let ro = read_only(shared);
    let mut lib = Library::with_backend(&root, Box::new(SqliteCatalog::new(store_of(shared))))?;
    lib.set_read_only(ro);
    *shared.lib.borrow_mut() = lib;
    super::covers::clear();
    Ok(())
}

impl Ctx {
    fn changed(&self, msg: impl Into<String>) {
        (self.notify)(msg.into());
    }

    fn lock_buttons(&self) {
        let ro = read_only(&self.shared);
        for b in [
            &self.backup_btn,
            &self.calibre_btn,
            &self.undo_btn,
            &self.empty_btn,
            &self.tidy_apply,
            &self.inbox_add_all,
        ] {
            b.set_sensitive(!ro);
        }
    }

    // -- status ---------------------------------------------------------------------------------

    fn refresh_status(&self) {
        self.status.clear();
        let lib = self.shared.lib.borrow();
        let loc = row(&tr("Location"), &lib.root.display().to_string());
        let open = button(&tr("Open"));
        super::audit::label(&open, &tr("Open the library folder"));
        let root = lib.root.clone();
        open.connect_clicked(move |b| {
            gtk::FileLauncher::new(Some(&gio::File::for_path(&root))).launch(
                window_of(b.upcast_ref::<gtk::Widget>()).as_ref(),
                gio::Cancellable::NONE,
                |_| {},
            );
        });
        loc.add_suffix(&open);
        self.status.push(&loc);
        let g = self.shared.guard.borrow();
        if let Some(note) = lh::read_only_note(&g) {
            let r = row(&tr("Read-only"), &note);
            r.add_prefix(&gtk::Image::from_icon_name("changes-prevent-symbolic"));
            self.status.push(&r);
        } else if lib.is_read_only() {
            self.status.push(&row(
                &tr("Read-only"),
                &tr("Changes are switched off for this session."),
            ));
        }
        for w in &g.warnings {
            let r = row(&tr("Careful with this folder"), &w.message);
            r.add_prefix(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
            self.status.push(&r);
        }
        let layout = lib.layout().clone();
        let text = match &layout {
            Layout::Template(t) => format!("{} — {t}", layout.label()),
            l => l.label().to_string(),
        };
        self.status.push(&row(&tr("Where books are kept"), &text));
        drop(g);
        drop(lib);
        self.lock_buttons();
    }

    // -- backups --------------------------------------------------------------------------------

    fn refresh_snapshots(self: &Rc<Self>) {
        self.snapshots.clear();
        self.restore_btns.borrow_mut().clear();
        let snaps = health::list_snapshots(&root_of(&self.shared));
        if snaps.is_empty() {
            self.snapshots.push(&row(
                &tr("No backups yet"),
                &tr("Back up now, or leave automatic backups on: a snapshot is taken when Bookx starts and when it closes."),
            ));
        }
        for s in snaps.into_iter().take(12) {
            let r = row(
                &lh::when(s.at),
                &format!(
                    "{} · {} · {}",
                    lh::plural(s.books, "book"),
                    s.kind.label(),
                    lh::size_text(s.size)
                ),
            );
            let b = button(&tr("Restore"));
            b.set_sensitive(!read_only(&self.shared));
            r.add_suffix(&b);
            let weak = Rc::downgrade(self);
            let path = s.path.clone();
            b.connect_clicked(move |_| {
                if let Some(ctx) = weak.upgrade() {
                    ctx.ask_restore(path.clone());
                }
            });
            self.restore_btns.borrow_mut().push(b);
            self.snapshots.push(&r);
        }
    }

    fn backup_now(self: &Rc<Self>) {
        let Some(panel) = self.panel.borrow().clone() else {
            return;
        };
        let weak = Rc::downgrade(self);
        self.backup_btn.set_sensitive(false);
        panel.run_now(Rc::new(move |r| {
            let Some(ctx) = weak.upgrade() else { return };
            ctx.backup_btn.set_sensitive(!read_only(&ctx.shared));
            match r {
                Ok((books, msg)) => {
                    ctx.refresh_snapshots();
                    // the Backups page already said what was copied; this says it for the button
                    if msg.is_empty() {
                        ctx.changed(trf("Backed up {}", &[&lh::plural(books, "book")]));
                    }
                    eprintln!("health: backup ok books={books}");
                }
                Err(e) => {
                    ctx.changed(trf("Backup failed: {}", &[&e]));
                    eprintln!("health: backup failed {e}");
                }
            }
        }));
    }

    fn ask_restore(self: &Rc<Self>, path: PathBuf) {
        let weak = Rc::downgrade(self);
        confirm(
            self,
            &tr("Restore this backup?"),
            &tr(
                "The catalogue goes back to how it was then. The current one is saved first as a \"Before a restore\" backup, so you can undo this.",
            ),
            &tr("Restore"),
            true,
            move || {
                if let Some(ctx) = weak.upgrade() {
                    ctx.restore(&path);
                }
            },
        );
    }

    fn restore(self: &Rc<Self>, path: &Path) {
        let store = store_of(&self.shared);
        let root = root_of(&self.shared);
        let r = health::restore_snapshot(&store, &root, path, Utc::now())
            .and_then(|_| reload_library(&self.shared));
        match r {
            Ok(()) => {
                let n = self.shared.lib.borrow().books.len();
                self.refresh_all();
                self.changed(trf("Restored the backup ({})", &[&lh::plural(n, "book")]));
                eprintln!("health: restored books={n}");
            }
            Err(e) => {
                self.changed(trf("Could not restore: {}", &[&format!("{:#}", e)]));
                eprintln!("health: restore failed {e:#}");
            }
        }
    }

    // -- check ----------------------------------------------------------------------------------

    fn run_check(self: &Rc<Self>) {
        let (books, root) = {
            let lib = self.shared.lib.borrow();
            (lib.books.clone(), lib.root.clone())
        };
        let store = store_of(&self.shared);
        let drift = self.drift_switch.is_active();
        self.check_btn.set_sensitive(false);
        self.check_summary.set_subtitle(&tr("Checking"));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let n = books.len();
            let findings = gio::spawn_blocking(move || {
                let mut lib = Library::in_memory(&root).expect("in-memory library");
                lib.books = books;
                health::check_library(&lib, Some(&store), CheckOptions { drift }).findings
            })
            .await
            .unwrap_or_default();
            if let Some(ctx) = weak.upgrade() {
                ctx.check_btn.set_sensitive(true);
                ctx.show_findings(n, findings);
            }
        });
    }

    fn show_findings(self: &Rc<Self>, books: usize, findings: Vec<Finding>) {
        self.check_summary
            .set_subtitle(&esc(&lh::check_summary(books, &findings)));
        eprintln!("health: check books={books} findings={}", findings.len());
        self.findings.clear();
        self.findings.group.set_visible(true);
        if findings.is_empty() {
            let r = row(
                &tr("No Problems Found"),
                &tr("Everything in the catalogue matches the files."),
            );
            r.add_prefix(&gtk::Image::from_icon_name("emblem-ok-symbolic"));
            self.findings.push(&r);
        }
        for (i, f) in findings.iter().enumerate().take(60) {
            let r = adw::ActionRow::new();
            r.set_title(&esc(&f.message));
            r.set_title_lines(3);
            r.add_prefix(&gtk::Image::from_icon_name(lh::finding_icon(f)));
            for fix in &f.fixes {
                let label = match fix {
                    Fix::RemoveEntry => "Remove entry",
                    Fix::ImportFile(_) => "Add book",
                    Fix::UseFile => "Use file",
                    Fix::UseCatalogue => "Use catalogue",
                };
                let b = button(label);
                b.set_sensitive(!read_only(&self.shared));
                let weak = Rc::downgrade(self);
                let fix = fix.clone();
                b.connect_clicked(move |_| {
                    if let Some(ctx) = weak.upgrade() {
                        ctx.apply_fix(i, &fix);
                    }
                });
                r.add_suffix(&b);
            }
            self.findings.push(&r);
        }
        if findings.len() > 60 {
            self.findings
                .push(&row("…", &trf("and {} more", &[&(findings.len() - 60)])));
        }
        *self.found.borrow_mut() = findings;
    }

    fn apply_fix(self: &Rc<Self>, index: usize, fix: &Fix) {
        let Some(f) = self.found.borrow().get(index).cloned() else {
            return;
        };
        let r = {
            let mut lib = self.shared.lib.borrow_mut();
            health::apply_fix(&mut lib, &f, fix).and_then(|m| lib.save().map(|_| m))
        };
        match r {
            Ok(m) => {
                self.changed(m);
                self.refresh_all();
                self.run_check();
            }
            Err(e) => self.changed(trf("Could not fix: {}", &[&format!("{:#}", e)])),
        }
    }

    // -- calibre ----------------------------------------------------------------------------------

    fn start_calibre(self: &Rc<Self>) {
        if let Some(dir) = self.calibre_pick.borrow_mut().take() {
            self.import_calibre(dir);
            return;
        }
        let dlg = gtk::FileDialog::builder()
            .title(tr("Choose the Calibre Library Folder"))
            .build();
        if let Some(h) = lh::calibre_hint() {
            dlg.set_initial_folder(Some(&gio::File::for_path(h)));
        }
        let weak = Rc::downgrade(self);
        dlg.select_folder(
            window_of(&self.parent).as_ref(),
            gio::Cancellable::NONE,
            move |res| {
                if let (Ok(f), Some(ctx)) = (res, weak.upgrade()) {
                    if let Some(p) = f.path() {
                        ctx.import_calibre(p);
                    }
                }
            },
        );
    }

    fn import_calibre(self: &Rc<Self>, dir: PathBuf) {
        self.calibre_group.set_visible(true);
        let store = store_of(&self.shared);
        let root = root_of(&self.shared);
        let copy = self.calibre_copy.is_active();
        let state = Arc::new(Mutex::new((Stage::Reading, 0usize, 1usize)));
        self.calibre_btn.set_sensitive(false);
        self.calibre_progress.set_visible(true);
        self.calibre_progress.set_fraction(0.0);
        self.calibre_report.set_subtitle(&tr("Importing"));
        let worker_state = state.clone();
        let handle = gio::spawn_blocking(move || {
            bookx_store::import_calibre_into(&store, &root, &dir, copy, &mut |s, d, n| {
                *worker_state.lock().unwrap_or_else(|e| e.into_inner()) = (s, d, n);
            })
            .map_err(|e| format!("{e:#}"))
        });
        // progress while the worker runs
        let weak = Rc::downgrade(self);
        let ticker = glib::timeout_add_local(Duration::from_millis(100), {
            let state = state.clone();
            move || {
                let Some(ctx) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                if !ctx.calibre_progress.is_visible() {
                    return glib::ControlFlow::Break;
                }
                let (stage, d, n) = *state.lock().unwrap_or_else(|e| e.into_inner());
                let frac = if n == 0 { 1.0 } else { d as f64 / n as f64 };
                // reading is the first half, copying the second when files are copied
                let frac = match (stage, ctx.calibre_copy.is_active()) {
                    (Stage::Reading, true) => frac / 2.0,
                    (Stage::Copying, _) => 0.5 + frac / 2.0,
                    _ => frac,
                };
                ctx.calibre_progress.set_fraction(frac);
                ctx.calibre_report.set_subtitle(match stage {
                    Stage::Reading => "Reading the Calibre catalogue",
                    Stage::Copying => "Copying book files",
                });
                glib::ControlFlow::Continue
            }
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let r = handle
                .await
                .unwrap_or_else(|_| Err("the import stopped unexpectedly".into()));
            ticker.remove();
            let Some(ctx) = weak.upgrade() else { return };
            ctx.calibre_btn.set_sensitive(!read_only(&ctx.shared));
            ctx.calibre_progress.set_visible(false);
            {
                let mut t = crate::trace::event(crate::trace::Area::Import, "calibre");
                match &r {
                    Ok(_) => t.ok(),
                    Err(e) => t.fail(e),
                }
            }
            match r {
                Ok(out) => {
                    let summary = out.summary();
                    let reload = reload_library(&ctx.shared);
                    ctx.calibre_report.set_subtitle(&esc(&summary));
                    ctx.refresh_all();
                    match reload {
                        Ok(()) => ctx.changed(summary.clone()),
                        Err(e) => ctx.changed(trf(
                            "Imported, but the list could not reload: {}",
                            &[&format!("{:#}", e)],
                        )),
                    }
                    eprintln!(
                        "health: calibre imported={} skipped={} duplicates={} copy_failed={}",
                        out.report.imported,
                        out.skipped.len(),
                        out.duplicate_titles.len(),
                        out.copy_failed.len()
                    );
                }
                Err(e) => {
                    ctx.calibre_report
                        .set_subtitle(&esc(&trf("Import failed: {}", &[&e])));
                    ctx.changed(trf("Calibre import failed: {}", &[&e]));
                    eprintln!("health: calibre failed {e}");
                }
            }
        });
    }

    // -- import folder ----------------------------------------------------------------------------

    fn inbox_state(&self) -> InboxState {
        InboxState::load(&root_of(&self.shared))
    }

    fn refresh_inbox_header(&self) {
        let st = self.inbox_state();
        match &st.folder {
            Some(f) => self
                .inbox_folder
                .set_subtitle(&esc(&f.display().to_string())),
            None => self.inbox_folder.set_subtitle(&tr("No folder chosen")),
        }
        self.inbox_scan.set_sensitive(st.folder.is_some());
    }

    fn pick_inbox_folder(self: &Rc<Self>) {
        let dlg = gtk::FileDialog::builder()
            .title(tr("Choose the Folder to Watch"))
            .build();
        let weak = Rc::downgrade(self);
        dlg.select_folder(
            window_of(&self.parent).as_ref(),
            gio::Cancellable::NONE,
            move |res| {
                if let (Ok(f), Some(ctx)) = (res, weak.upgrade()) {
                    if let Some(p) = f.path() {
                        ctx.set_inbox_folder(p);
                    }
                }
            },
        );
    }

    fn set_inbox_folder(self: &Rc<Self>, folder: PathBuf) {
        let mut st = self.inbox_state();
        st.folder = Some(folder);
        if let Err(e) = st.save(&root_of(&self.shared)) {
            self.changed(trf("Could not save the folder: {}", &[&format!("{:#}", e)]));
            return;
        }
        self.refresh_inbox_header();
        self.scan_inbox();
    }

    fn scan_inbox(self: &Rc<Self>) {
        let st = self.inbox_state();
        let books = self.shared.lib.borrow().books.clone();
        let root = root_of(&self.shared);
        self.inbox_scan.set_sensitive(false);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let found = gio::spawn_blocking(move || scan_off_thread(&st, &root, books))
                .await
                .unwrap_or_default();
            if let Some(ctx) = weak.upgrade() {
                ctx.inbox_scan
                    .set_sensitive(ctx.inbox_state().folder.is_some());
                ctx.show_candidates(found);
            }
        });
    }

    fn show_candidates(self: &Rc<Self>, found: Vec<Candidate>) {
        self.inbox.clear();
        eprintln!("health: inbox waiting={}", found.len());
        if found.is_empty() {
            self.inbox.push(&row(
                &tr("Nothing waiting"),
                &tr("New books in the folder show up here for you to add or dismiss. Files still being copied wait a few seconds."),
            ));
        }
        for (i, c) in found.iter().enumerate().take(50) {
            let mut sub = c.authors.join(" & ");
            if let Some(e) = &c.error {
                sub = trf("Cannot be read: {}", &[&e]);
            } else if let Some(id) = c.duplicate_of {
                sub = format!("{sub} · looks like book #{id} already in the library");
            }
            let r = row(&c.title, &sub);
            if c.error.is_none() {
                let add = button(
                    &(if c.duplicate_of.is_some() {
                        tr("Add anyway")
                    } else {
                        tr("Add")
                    }),
                );
                add.set_sensitive(!read_only(&self.shared));
                let weak = Rc::downgrade(self);
                add.connect_clicked(move |_| {
                    if let Some(ctx) = weak.upgrade() {
                        ctx.accept_candidate(i);
                    }
                });
                r.add_suffix(&add);
            }
            let dismiss = button(&tr("Dismiss"));
            let weak = Rc::downgrade(self);
            dismiss.connect_clicked(move |_| {
                if let Some(ctx) = weak.upgrade() {
                    ctx.dismiss_candidate(i);
                }
            });
            r.add_suffix(&dismiss);
            self.inbox.push(&r);
        }
        *self.candidates.borrow_mut() = found;
    }

    fn accept_candidate(self: &Rc<Self>, index: usize) {
        let Some(c) = self.candidates.borrow().get(index).cloned() else {
            return;
        };
        let mut st = self.inbox_state();
        let r = {
            let mut lib = self.shared.lib.borrow_mut();
            inbox::accept(&mut st, &mut lib, &c, true, &crate::import::prepare)
        };
        match r {
            Ok(out) => {
                let _ = st.save(&root_of(&self.shared));
                let msg = match out {
                    AddOutcome::Added(_) => trf("Added “{}”", &[&c.title]),
                    AddOutcome::Duplicate(_) => trf("“{}” is already in the library", &[&c.title]),
                };
                self.changed(msg);
                self.refresh_all();
                self.scan_inbox();
            }
            Err(e) => self.changed(trf(
                "Could not add “{}”: {}",
                &[&c.title, &format!("{:#}", e)],
            )),
        }
    }

    fn dismiss_candidate(self: &Rc<Self>, index: usize) {
        let Some(c) = self.candidates.borrow().get(index).cloned() else {
            return;
        };
        let mut st = self.inbox_state();
        inbox::dismiss(&mut st, &c);
        let _ = st.save(&root_of(&self.shared));
        self.scan_inbox();
    }

    fn add_all_candidates(self: &Rc<Self>) {
        let list = self.candidates.borrow().clone();
        let mut st = self.inbox_state();
        let (mut added, mut dups, mut failed) = (0, 0, 0);
        for c in list.iter().filter(|c| c.error.is_none()) {
            let r = {
                let mut lib = self.shared.lib.borrow_mut();
                inbox::accept(&mut st, &mut lib, c, false, &crate::import::prepare)
            };
            match r {
                Ok(AddOutcome::Added(_)) => added += 1,
                Ok(AddOutcome::Duplicate(_)) => dups += 1,
                Err(_) => failed += 1,
            }
        }
        let _ = st.save(&root_of(&self.shared));
        self.changed(trf(
            "{} added, {} already in the library, {} failed",
            &[&added, &dups, &failed],
        ));
        eprintln!("health: inbox added={added} duplicates={dups} failed={failed}");
        self.refresh_all();
        self.scan_inbox();
    }

    // -- history and trash --------------------------------------------------------------------------

    fn refresh_history(self: &Rc<Self>) {
        self.history.clear();
        let lib = self.shared.lib.borrow();
        let ops: Vec<_> = lib.history().iter().rev().take(6).cloned().collect();
        let can_undo = bookx_core::history::last_undoable(lib.history()).is_some();
        drop(lib);
        self.undo_btn
            .set_sensitive(can_undo && !read_only(&self.shared));
        if ops.is_empty() {
            self.history.push(&row(
                &tr("Nothing yet"),
                &tr("Imports, deletions, edits and moves are listed here."),
            ));
        }
        for o in &ops {
            self.history.push(&row(&lh::describe_op(o), ""));
        }
        eprintln!("health: ops={}", self.shared.lib.borrow().history().len());
    }

    fn undo(self: &Rc<Self>) {
        let r = {
            let mut lib = self.shared.lib.borrow_mut();
            lib.undo_last().and_then(|l| lib.save().map(|_| l))
        };
        match r {
            Ok(Some(label)) => {
                self.changed(trf("Undone: {}", &[&label]));
                eprintln!("health: undone {label}");
                self.refresh_all();
            }
            Ok(None) => self.changed(tr("Nothing to undo")),
            Err(e) => self.changed(trf("Could not undo: {}", &[&format!("{:#}", e)])),
        }
    }

    fn refresh_trash(self: &Rc<Self>) {
        self.trash.clear();
        self.trash_btns.borrow_mut().clear();
        let entries = self.shared.lib.borrow().trash();
        self.empty_btn
            .set_sensitive(!entries.is_empty() && !read_only(&self.shared));
        eprintln!("health: trash={}", entries.len());
        if entries.is_empty() {
            self.trash.push(&row(
                &tr("The trash is empty"),
                &tr("Deleted books wait here until you empty the trash."),
            ));
        }
        for e in entries.iter().take(30) {
            let r = row(&e.book.meta.title, &lh::describe_trash(e));
            let b = button(&tr("Restore"));
            b.set_sensitive(!read_only(&self.shared));
            let weak = Rc::downgrade(self);
            let name = e.name.clone();
            b.connect_clicked(move |_| {
                if let Some(ctx) = weak.upgrade() {
                    ctx.restore_trash(&name);
                }
            });
            r.add_suffix(&b);
            self.trash_btns.borrow_mut().push(b);
            self.trash.push(&r);
        }
    }

    fn restore_trash(self: &Rc<Self>, name: &str) {
        let r = {
            let mut lib = self.shared.lib.borrow_mut();
            lib.restore_from_trash(name)
                .and_then(|id| lib.save().map(|_| id))
        };
        match r {
            Ok(id) => {
                self.changed(tr("Book restored"));
                eprintln!("health: trash restored #{id}");
                self.refresh_all();
            }
            Err(e) => self.changed(trf("Could not restore: {}", &[&format!("{:#}", e)])),
        }
    }

    fn ask_empty_trash(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let n = self.shared.lib.borrow().trash().len();
        confirm(
            self,
            &trash_heading(n),
            &tr("The deleted books and their files are erased for good."),
            &tr("Delete Permanently"),
            true,
            move || {
                if let Some(ctx) = weak.upgrade() {
                    let r = ctx.shared.lib.borrow_mut().empty_trash();
                    match r {
                        Ok(n) => {
                            ctx.changed(trf("Erased {}", &[&lh::plural(n, "book")]));
                            eprintln!("health: trash emptied {n}");
                            ctx.refresh_all();
                        }
                        Err(e) => ctx
                            .changed(trf("Could not empty the trash: {}", &[&format!("{:#}", e)])),
                    }
                }
            },
        );
    }

    // -- names ----------------------------------------------------------------------------------------

    fn refresh_names(self: &Rc<Self>) {
        let lib = self.shared.lib.borrow();
        let changes = bookx_sources::tidy::plan(&lib.books);
        let overview = bookx_sources::tidy::series_overview(&lib.books);
        drop(lib);
        self.tidy_summary.set_subtitle(&if changes.is_empty() {
            "Author and series names are consistent.".to_string()
        } else {
            let shown: Vec<String> = changes
                .iter()
                .take(3)
                .map(|c| format!("“{}” → “{}”", c.from, c.to))
                .collect();
            format!(
                "{} to change: {}{}",
                lh::plural(changes.len(), "name"),
                shown.join(", "),
                if changes.len() > 3 { ", …" } else { "" }
            )
        });
        self.tidy_apply
            .set_sensitive(!changes.is_empty() && !read_only(&self.shared));
        eprintln!("health: tidy={}", changes.len());
        self.series.clear();
        let with_gaps: Vec<_> = overview.iter().filter(|g| !g.missing.is_empty()).collect();
        if with_gaps.is_empty() {
            self.series.push(&row(
                &tr("Series"),
                &trf(
                    "{} found, no numbers missing in between.",
                    &[&overview.len()],
                ),
            ));
        }
        for g in with_gaps.iter().take(20) {
            let missing: Vec<String> = g.missing.iter().take(8).map(|n| n.to_string()).collect();
            self.series.push(&row(
                &g.name,
                &trf(
                    "{} in the library; missing number {}",
                    &[&(lh::plural(g.entries.len(), "book")), &missing.join(", ")],
                ),
            ));
        }
    }

    fn apply_tidy(self: &Rc<Self>) {
        let r = {
            let mut lib = self.shared.lib.borrow_mut();
            let changes = bookx_sources::tidy::plan(&lib.books);
            let ids: Vec<u64> = changes.iter().map(|c| c.id).collect();
            let before = lh::snapshot_meta(&lib, &ids);
            let r =
                bookx_sources::tidy::apply(&mut lib, &changes).and_then(|n| lib.save().map(|_| n));
            lh::record_changes(
                &self.shared.guard.borrow().store,
                &lib,
                &before,
                lh::TIDY,
                chrono::Utc::now().timestamp(),
            );
            r
        };
        match r {
            Ok(n) => {
                self.changed(trf("Tidied {}", &[&lh::plural(n, "book")]));
                eprintln!("health: tidied {n}");
                self.refresh_all();
            }
            Err(e) => self.changed(trf("Could not tidy: {}", &[&format!("{:#}", e)])),
        }
    }

    // -- all ------------------------------------------------------------------------------------------

    fn refresh_all(self: &Rc<Self>) {
        self.refresh_status();
        self.refresh_snapshots();
        self.refresh_history();
        self.refresh_trash();
        self.refresh_names();
        self.refresh_inbox_header();
    }

    /// One line per fact, for the UI tests.
    fn dump(&self) {
        eprintln!("health_books: {}", self.shared.lib.borrow().books.len());
        eprintln!(
            "health_snapshots: {}",
            health::list_snapshots(&root_of(&self.shared)).len()
        );
        eprintln!("health_findings: {}", self.found.borrow().len());
        eprintln!("health_trash: {}", self.shared.lib.borrow().trash().len());
        eprintln!("health_inbox: {}", self.candidates.borrow().len());
        eprintln!("health_read_only: {}", read_only(&self.shared));
        eprintln!(
            "health_warnings: {}",
            self.shared.guard.borrow().warnings.len()
        );
        eprintln!("health_status_rows: {}", self.status.len());
        eprintln!(
            "health_restore_buttons: {}",
            self.restore_btns.borrow().len()
        );
        eprintln!("health_undo_sensitive: {}", self.undo_btn.is_sensitive());
        eprintln!(
            "health_backup_sensitive: {}",
            self.backup_btn.is_sensitive()
        );
        eprintln!(
            "health_summary: {}",
            self.check_summary.subtitle().unwrap_or_default()
        );
        eprintln!(
            "health_calibre: {}",
            self.calibre_report.subtitle().unwrap_or_default()
        );
        eprintln!(
            "health_ui: problems_visible={} calibre_visible={} check_button={:?} status_titles={}",
            self.findings.group.is_visible(),
            self.calibre_group.is_visible(),
            self.check_btn.label().unwrap_or_default(),
            self.status
                .rows
                .borrow()
                .iter()
                .filter_map(|w| w.clone().downcast::<adw::ActionRow>().ok())
                .map(|r| r.title().to_string())
                .collect::<Vec<_>>()
                .join("|")
        );
        eprintln!("health_auto: {}", self.auto_switch.is_active());
        eprintln!("health_keep: {}", self.keep_spin.value());
        if let Some(p) = self.panel.borrow().clone() {
            p.dev("backup-dump");
        }
    }
}

/// Finds the new books in the watched folder, on a worker thread.
fn scan_off_thread(
    st: &InboxState,
    root: &Path,
    books: Vec<bookx_core::library::Book>,
) -> Vec<Candidate> {
    let Ok(mut lib) = Library::in_memory(root) else {
        return Vec::new();
    };
    lib.books = books;
    inbox::scan(
        st,
        &lib,
        SystemTime::now(),
        inbox::SETTLE,
        &crate::import::prepare,
    )
}

fn switch_row(title: &str, subtitle: &str, on: bool) -> adw::SwitchRow {
    let s = adw::SwitchRow::new();
    s.set_title(&esc(title));
    s.set_subtitle(&esc(subtitle));
    s.set_active(on);
    s
}

fn page_of(ctx_groups: &[&adw::PreferencesGroup]) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    for g in ctx_groups {
        page.add(*g);
    }
    page
}

/// What the Duplicates, Editions & Formats and Needs Attention tabs need from the library window.
#[derive(Clone)]
pub struct Links {
    /// Shows a search in the library (the window closes).
    pub search: super::organize::Search,
    /// Starts *Fetch Metadata* for these books.
    pub fetch: super::organize::Fetch,
}

impl Links {
    fn none() -> Links {
        Links {
            search: Rc::new(|_| {}),
            fetch: Rc::new(|_| {}),
        }
    }
}

/// The window as a sidebar of destinations and the page of the chosen one (LIB-L1). Narrow windows show the
/// sidebar as an overlay that a header button opens.
fn sidebar_layout(dialog: &adw::Dialog, stack: &adw::ViewStack) -> adw::OverlaySplitView {
    let split = adw::OverlaySplitView::new();
    split.set_min_sidebar_width(170.0);
    split.set_max_sidebar_width(230.0);
    split.set_sidebar_width_fraction(0.26);

    let list = gtk::ListBox::new();
    list.add_css_class("navigation-sidebar");
    list.set_selection_mode(gtk::SelectionMode::Single);
    super::audit::label(&list, &tr("Library Health sections"));
    let mut rows = Vec::new();
    for (id, title, icon) in TABS {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        b.append(&gtk::Image::from_icon_name(icon));
        let l = gtk::Label::new(Some(&tr(title)));
        l.set_xalign(0.0);
        b.append(&l);
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&b));
        row.set_widget_name(id);
        super::audit::label(&row, &tr(title));
        list.append(&row);
        rows.push(row);
    }
    let side_head = adw::HeaderBar::new();
    side_head.set_show_end_title_buttons(false);
    side_head.set_title_widget(Some(&adw::WindowTitle::new(&tr("Library Health"), "")));
    let side = adw::ToolbarView::new();
    side.add_top_bar(&side_head);
    side.set_content(Some(
        &gtk::ScrolledWindow::builder()
            .child(&list)
            .vexpand(true)
            .build(),
    ));
    split.set_sidebar(Some(&side));

    let title = adw::WindowTitle::new("", "");
    let head = adw::HeaderBar::new();
    head.set_title_widget(Some(&title));
    let toggle = gtk::ToggleButton::new();
    toggle.set_icon_name("sidebar-show-symbolic");
    toggle.set_tooltip_text(Some(&tr("Show Sections")));
    super::audit::label(&toggle, &tr("Show Sections"));
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
    content.set_content(Some(stack));
    split.set_content(Some(&content));

    let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        620.0,
        adw::LengthUnit::Sp,
    ));
    bp.add_setter(&split, "collapsed", Some(&true.to_value()));
    dialog.add_breakpoint(bp);

    {
        let (stack, split) = (stack.clone(), split.clone());
        list.connect_row_selected(move |_, row| {
            if let Some(row) = row {
                stack.set_visible_child_name(&row.widget_name());
                if split.is_collapsed() {
                    split.set_show_sidebar(false);
                }
            }
        });
    }
    let sync = {
        let (list, rows, title) = (list.clone(), rows.clone(), title.clone());
        move |stack: &adw::ViewStack| {
            let id = stack.visible_child_name().unwrap_or_default();
            if let Some(i) = TABS.iter().position(|t| t.0 == id) {
                list.select_row(rows.get(i));
                title.set_title(&tr(TABS[i].1));
            }
        }
    };
    sync(stack);
    stack.connect_visible_child_name_notify(sync);
    split
}

/// The heading of the confirmation before the trash is emptied: it names how many books go.
pub fn trash_heading(n: usize) -> String {
    if n == 0 {
        return tr("Empty the Trash?");
    }
    crate::i18n::ntr(
        "Delete {} Book Permanently?",
        "Delete {} Books Permanently?",
        n,
    )
}

/// The tabs of the window: (id, title, icon).
pub const TABS: [(&str, &str, &str); 6] = [
    ("health", N_("Health"), "security-high-symbolic"),
    ("duplicates", N_("Duplicates"), "edit-copy-symbolic"),
    ("editions", N_("Editions"), "view-dual-symbolic"),
    (
        "attention",
        N_("Needs Attention"),
        "dialog-warning-symbolic",
    ),
    ("backups", N_("Backups"), "document-save-symbolic"),
    ("history", N_("Trash"), "user-trash-symbolic"),
];

pub fn show(parent: &impl IsA<gtk::Widget>, shared: &SharedRef, notify: Notify) {
    show_tab(parent, shared, notify, Links::none(), "health");
}

pub fn show_tab(
    parent: &impl IsA<gtk::Widget>,
    shared: &SharedRef,
    notify: Notify,
    links: Links,
    tab: &str,
) {
    let dialog = crate::ui::dialog_toast::new_dialog();
    dialog.set_title(&tr("Library Health"));
    dialog.set_content_width(860);
    dialog.set_content_height(560);
    dialog.set_width_request(360);
    dialog.set_height_request(400);

    let status = Dynamic::new(&tr("Library"), "");
    // backups
    let backups = adw::PreferencesGroup::new();
    backups.set_title(&tr("Backups"));
    backups.set_description(Some(&bk::backup_scope_description()));
    let prefs = shared.cfg.borrow().health.clone();
    let auto_switch = switch_row(
        &tr("Automatic backups"),
        &tr("Take a snapshot on this computer when Bookx starts and closes"),
        prefs.auto_backup,
    );
    let keep_spin = adw::SpinRow::with_range(1.0, 50.0, 1.0);
    keep_spin.set_title(&tr("Snapshots to keep"));
    keep_spin.set_subtitle(&tr("Plus the newest one of each of the last 7 days"));
    keep_spin.set_value(prefs.keep as f64);
    let backup_row = adw::ActionRow::new();
    backup_row.set_title(&tr("Back up now"));
    backup_row.set_subtitle(&tr(
        "Saves a snapshot here and copies new and changed books, covers and notes to your places",
    ));
    let backup_btn = button(&tr("Back Up"));
    backup_btn.add_css_class("suggested-action");
    backup_row.add_suffix(&backup_btn);
    backups.add(&auto_switch);
    backups.add(&keep_spin);
    backups.add(&backup_row);
    let snapshots = Dynamic::new(&tr("Restore from a backup"), "");
    // check
    let check = adw::PreferencesGroup::new();
    check.set_title(&tr("Check"));
    check.set_description(Some(&tr(
        "Looks for damage in the database, missing files and covers, and book files nobody lists.",
    )));
    let drift_switch = switch_row(
        &tr("Compare with the book files"),
        &tr(
            "Also finds books whose title, authors or series differ from what the file says (slower)",
        ),
        false,
    );
    let check_summary = row(&tr("Check library"), &tr("Not checked yet"));
    let check_btn = button(&tr("Check Now"));
    super::audit::label(&check_btn, &tr("Check library now"));
    check_summary.add_suffix(&check_btn);
    check.add(&drift_switch);
    check.add(&check_summary);
    let findings = Dynamic::new(&tr("Problems"), "");
    findings.group.set_visible(false);
    // calibre
    let calibre = adw::PreferencesGroup::new();
    calibre.set_title(&tr("Import from Calibre"));
    calibre.set_description(Some(&tr(
        "Reads the metadata.db of a Calibre library. Your Calibre library is not changed.",
    )));
    let calibre_copy = switch_row(
        &tr("Copy the book files into this library"),
        &tr("Off: the books stay in the Calibre folder and are only listed here"),
        true,
    );
    let calibre_row = adw::ActionRow::new();
    calibre_row.set_title(&tr("Import a Calibre library"));
    let calibre_btn = button(&tr("Import Calibre Library"));
    calibre_row.add_suffix(&calibre_btn);
    let calibre_progress = gtk::ProgressBar::new();
    calibre_progress.set_visible(false);
    calibre_progress.set_margin_start(12);
    calibre_progress.set_margin_end(12);
    calibre_progress.set_margin_top(6);
    calibre_progress.set_margin_bottom(6);
    let calibre_report = row(&tr("Report"), &tr("Nothing imported in this window yet"));
    calibre.add(&calibre_copy);
    calibre.add(&calibre_row);
    calibre.add(&calibre_progress);
    calibre.add(&calibre_report);
    calibre.set_visible(false);
    // import folder
    let inbox_group = adw::PreferencesGroup::new();
    inbox_group.set_title(&tr("Import folder"));
    inbox_group.set_description(Some(&tr("New books that appear in this folder wait here for your approval, unless you switch on adding them automatically.")));
    let inbox_folder = row(&tr("Watched folder"), &tr("No folder chosen"));
    let inbox_pick = button(&tr("Choose"));
    inbox_folder.add_suffix(&inbox_pick);
    let inbox_scan = button(&tr("Check Now"));
    inbox_scan.set_sensitive(false);
    let inbox_add_all = button(&tr("Add All"));
    let inbox_actions = adw::ActionRow::new();
    inbox_actions.set_title(&tr("Waiting books"));
    inbox_actions.add_suffix(&inbox_scan);
    inbox_actions.add_suffix(&inbox_add_all);
    inbox_group.add(&inbox_folder);
    inbox_group.add(&inbox_actions);
    inbox_group.add(&super::folder_watch::switch_row(shared));
    let inbox = Dynamic::new("", "");
    // history and trash
    let history_group = adw::PreferencesGroup::new();
    history_group.set_title(&tr("History"));
    history_group.set_description(Some(&tr(
        "Imports, deletions, edits and moves of the library can be taken back, newest first.",
    )));
    let undo_row = adw::ActionRow::new();
    undo_row.set_title(&tr("Undo the last operation"));
    let undo_btn = button(&tr("Undo"));
    undo_row.add_suffix(&undo_btn);
    history_group.add(&undo_row);
    let history = Dynamic::new("", "");
    let trash_group = adw::PreferencesGroup::new();
    trash_group.set_title(&tr("Trash"));
    trash_group.set_description(Some(&tr("Deleting a book moves its files here. Restore them, or empty the trash to erase them for good.")));
    let empty_row = adw::ActionRow::new();
    empty_row.set_title(&tr("Erase everything in the trash"));
    let empty_btn = button(&tr("Empty Trash"));
    empty_btn.add_css_class("destructive-action");
    empty_row.add_suffix(&empty_btn);
    trash_group.add(&empty_row);
    let trash = Dynamic::new("", "");
    // names
    let names = adw::PreferencesGroup::new();
    names.set_title(&tr("Names and series"));
    names.set_description(Some(&tr(
        "One spelling for each author and series, and a look at series with numbers missing.",
    )));
    let tidy_summary = row(&tr("Tidy names"), "…");
    let tidy_apply = button(&tr("Tidy Names"));
    tidy_summary.add_suffix(&tidy_apply);
    names.add(&tidy_summary);
    let series = Dynamic::new("", "");

    let ctx = Rc::new(Ctx {
        shared: shared.clone(),
        notify,
        parent: parent.clone().upcast(),
        status: status.clone(),
        auto_switch: auto_switch.clone(),
        keep_spin: keep_spin.clone(),
        backup_btn: backup_btn.clone(),
        snapshots: snapshots.clone(),
        restore_btns: RefCell::new(Vec::new()),
        panel: RefCell::new(None),
        check_btn: check_btn.clone(),
        drift_switch,
        check_summary,
        findings: findings.clone(),
        found: RefCell::new(Vec::new()),
        calibre_btn: calibre_btn.clone(),
        calibre_group: calibre.clone(),
        calibre_copy,
        calibre_progress,
        calibre_report,
        calibre_pick: RefCell::new(None),
        inbox_folder,
        inbox_scan: inbox_scan.clone(),
        inbox_add_all: inbox_add_all.clone(),
        inbox: inbox.clone(),
        candidates: RefCell::new(Vec::new()),
        undo_btn: undo_btn.clone(),
        history: history.clone(),
        trash: trash.clone(),
        trash_btns: RefCell::new(Vec::new()),
        empty_btn: empty_btn.clone(),
        tidy_summary,
        tidy_apply: tidy_apply.clone(),
        series: series.clone(),
        tabs: RefCell::new(None),
    });

    macro_rules! wire {
        ($btn:expr_2021, $method:ident) => {{
            let weak = Rc::downgrade(&ctx);
            $btn.connect_clicked(move |_| {
                if let Some(c) = weak.upgrade() {
                    c.$method();
                }
            });
        }};
    }
    wire!(backup_btn, backup_now);
    wire!(check_btn, run_check);
    wire!(calibre_btn, start_calibre);
    wire!(inbox_pick, pick_inbox_folder);
    wire!(inbox_scan, scan_inbox);
    wire!(inbox_add_all, add_all_candidates);
    wire!(undo_btn, undo);
    wire!(empty_btn, ask_empty_trash);
    wire!(tidy_apply, apply_tidy);
    {
        let weak = Rc::downgrade(&ctx);
        auto_switch.connect_active_notify(move |s| {
            if let Some(c) = weak.upgrade() {
                let mut cfg = c.shared.cfg.borrow_mut();
                cfg.health.auto_backup = s.is_active();
                cfg.save();
            }
        });
        let weak = Rc::downgrade(&ctx);
        keep_spin.connect_value_notify(move |s| {
            if let Some(c) = weak.upgrade() {
                let mut cfg = c.shared.cfg.borrow_mut();
                cfg.health.keep = s.value() as usize;
                cfg.save();
            }
        });
    }

    let panel = {
        let weak = Rc::downgrade(&ctx);
        let weak2 = weak.clone();
        let weak3 = weak.clone();
        super::backup::Panel::new(
            &ctx.parent,
            shared,
            ctx.notify.clone(),
            Rc::new(move |path| {
                if let Some(c) = weak.upgrade() {
                    c.ask_restore(path);
                }
            }),
            Rc::new(move || {
                if let Some(c) = weak2.upgrade() {
                    c.refresh_snapshots();
                }
            }),
            Rc::new(move || {
                if let Some(c) = weak3.upgrade() {
                    match reload_library(&c.shared) {
                        Ok(()) => {
                            let n = c.shared.lib.borrow().books.len();
                            c.refresh_all();
                            eprintln!("health: restored books={n}");
                        }
                        Err(e) => {
                            c.changed(trf("Could not restore: {}", &[&format!("{e:#}")]));
                            eprintln!("health: restore failed {e:#}");
                        }
                    }
                }
            }),
        )
    };
    *ctx.panel.borrow_mut() = Some(panel.clone());
    ctx.refresh_all();

    let panel_groups = panel.groups();
    let page_health = page_of(&[&status.group, &check, &findings.group]);
    for g in [&calibre, &inbox_group, &inbox.group, &names, &series.group] {
        page_health.add(g);
    }
    for g in super::health_extra::groups(shared, links.fetch.clone(), ctx.notify.clone()) {
        page_health.add(&g);
    }
    let mut groups: Vec<&adw::PreferencesGroup> = vec![&backups];
    groups.extend(panel_groups.iter().take(2));
    groups.push(&snapshots.group);
    groups.push(&panel_groups[2]);
    groups.push(&history_group);
    groups.push(&history.group);
    let page_backups = page_of(&groups);
    let page_history = page_of(&[&trash_group, &trash.group]);

    let win = parent.root().and_downcast::<gtk::Window>();
    let changed = ctx.notify.clone();
    let stack = adw::ViewStack::new();
    let parent_win = win.clone();
    for (id, title, icon) in TABS {
        let title = tr(title);
        let child: gtk::Widget = match id {
            "health" => page_health.clone().upcast(),
            "backups" => page_backups.clone().upcast(),
            "history" => page_history.clone().upcast(),
            "duplicates" => {
                let (shared, changed) = (shared.clone(), changed.clone());
                super::duplicates::page(&shared, changed)
            }
            "editions" => match &parent_win {
                Some(w) => super::organize::works_page(
                    &dialog,
                    w,
                    shared,
                    links.search.clone(),
                    changed.clone(),
                )
                .upcast(),
                None => gtk::Box::new(gtk::Orientation::Vertical, 0).upcast(),
            },
            _ => {
                let (page, fetch_btn) = super::organize::attention_page(
                    &dialog,
                    shared,
                    links.search.clone(),
                    links.fetch.clone(),
                    changed.clone(),
                );
                let g = adw::PreferencesGroup::new();
                g.set_title(&tr("Look Up Online"));
                g.set_description(Some(&tr(
                    "Fill in the missing details of these books from online sources",
                )));
                g.set_header_suffix(Some(&fetch_btn));
                page.add(&g);
                page.upcast()
            }
        };
        let p = stack.add_titled(&child, Some(id), &title);
        p.set_icon_name(Some(icon));
    }
    stack.set_visible_child_name(tab);
    dialog.set_child(Some(&sidebar_layout(&dialog, &stack)));
    ctx.tabs.replace(Some(stack.clone()));
    OPEN.with(|o| *o.borrow_mut() = Some(ctx.clone()));
    dialog.connect_closed(move |_| {
        OPEN.with(|o| *o.borrow_mut() = None);
    });
    dialog.present(Some(parent));
    ctx.scan_inbox_if_watching();
}

impl Ctx {
    fn scan_inbox_if_watching(self: &Rc<Self>) {
        if self.inbox_state().folder.is_some() {
            self.scan_inbox();
        } else {
            self.show_candidates(Vec::new());
        }
    }
}

// -- layout (Preferences) ------------------------------------------------------------------------------

/// Changes the layout after showing what would move. `done` gets a sentence either way.
pub fn change_layout(parent: &gtk::Widget, shared: &SharedRef, new: Layout, done: Notify) {
    let (plans, current) = {
        let lib = shared.lib.borrow();
        (lib.plan_layout(&new), lib.layout().clone())
    };
    if new == current {
        return;
    }
    let apply = {
        let shared = shared.clone();
        let new = new.clone();
        let done = done.clone();
        move || {
            let r = shared.lib.borrow_mut().change_layout(new.clone());
            match r {
                Ok(n) => {
                    super::covers::clear();
                    done(trf("Layout changed: {} moved", &[&lh::plural(n, "book")]));
                    eprintln!("health: layout {} moved={n}", new.label());
                }
                Err(e) => {
                    done(trf(
                        "The layout was not changed: {}",
                        &[&format!("{:#}", e)],
                    ));
                    eprintln!("health: layout failed {e:#}");
                }
            }
        }
    };
    if plans.is_empty() || autoconfirm() {
        apply();
    } else {
        super::alert(
            parent,
            "Move the books?",
            &lh::layout_preview(&plans),
            "Move Books",
            false,
            apply,
        );
    }
}

/// The Preferences rows for where books are kept, the template, and the backup settings.
pub fn preferences_group(
    parent: &impl IsA<gtk::Widget>,
    shared: &SharedRef,
    notify: Notify,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&tr("Library files"));
    group.set_description(Some(&tr(
        "Where Bookx keeps the book files. Changing it moves them (with a preview), all or nothing.",
    )));
    let names_owned: Vec<String> = lh::layout_choices().iter().map(|c| tr(c.1)).collect();
    let names: Vec<&str> = names_owned.iter().map(|s| s.as_str()).collect();
    let combo = adw::ComboRow::builder()
        .title(tr("Where books are kept"))
        .model(&gtk::StringList::new(&names))
        .build();
    let current = shared.lib.borrow().layout().clone();
    combo.set_selected(lh::layout_index(&current));
    let template = adw::EntryRow::new();
    template.set_title(&tr("Path template"));
    template.set_tooltip_text(Some(&tr("For example {author}/{series}/{title}")));
    if let Layout::Template(t) = &current {
        template.set_text(t);
    } else {
        template.set_text(bookx_core::layout::DEFAULT_TEMPLATE);
    }
    template.set_show_apply_button(true);
    template.set_visible(matches!(current, Layout::Template(_)));
    let health_row = adw::ActionRow::new();
    health_row.set_title(&tr("Backups, check, trash and history"));
    health_row.set_subtitle(&tr(
        "Backups of your books and notes, library checks and the trash",
    ));
    health_row.set_activatable(true);
    health_row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    group.add(&combo);
    group.add(&template);
    group.add(&health_row);

    let parent_w: gtk::Widget = parent.clone().upcast();
    let guard = Rc::new(std::cell::Cell::new(false));
    let apply = {
        let shared = shared.clone();
        let notify = notify.clone();
        let parent_w = parent_w.clone();
        let combo = combo.clone();
        let template = template.clone();
        let guard = guard.clone();
        Rc::new(move || {
            if guard.get() {
                return;
            }
            let name = lh::layout_choices()[combo.selected() as usize].0;
            template.set_visible(name == "template");
            let Some(layout) = Layout::parse(name, &template.text()) else {
                return;
            };
            if let Layout::Template(t) = &layout {
                let f: bookx_core::layout::TemplateFn =
                    bookx_sources::template::render_library_path;
                if let Err(e) = bookx_core::layout::check_template(t, f) {
                    notify(trf("That template does not work: {}", &[&e]));
                    return;
                }
            }
            // after a refusal or failure the choice goes back to what the library really uses
            let (shared2, combo2, guard2) = (shared.clone(), combo.clone(), guard.clone());
            let notify2 = notify.clone();
            let done: Notify = Rc::new(move |m| {
                guard2.set(true);
                combo2.set_selected(lh::layout_index(shared2.lib.borrow().layout()));
                guard2.set(false);
                notify2(m);
            });
            change_layout(&parent_w, &shared, layout, done);
        })
    };
    {
        let a = apply.clone();
        combo.connect_selected_notify(move |_| a());
        let a = apply.clone();
        template.connect_apply(move |_| a());
    }
    {
        let shared = shared.clone();
        let notify = notify.clone();
        let parent_w = parent_w.clone();
        health_row.connect_activated(move |_| show(&parent_w, &shared, notify.clone()));
    }
    group
}

// -- app start and exit ---------------------------------------------------------------------------------

/// What happens once a library is open: the notes about its folder and lock, the automatic
/// snapshot and a look at the import folder. `toast` shows a message.
pub fn on_startup(shared: &SharedRef, toast: Notify) {
    let note = {
        let g = shared.guard.borrow();
        lh::read_only_note(&g)
    };
    if let Some(n) = note {
        toast(n);
    }
    let new_notes = {
        let g = shared.guard.borrow();
        let root = root_of(shared);
        let mut cfg = shared.cfg.borrow_mut();
        let notes = lh::take_new_warnings(&mut cfg.health, &root, &g.warnings);
        if !notes.is_empty() {
            cfg.save();
        }
        notes
    };
    for n in new_notes {
        toast(n);
    }
    if !read_only(shared) {
        let (store, root, prefs) = (
            store_of(shared),
            root_of(shared),
            shared.cfg.borrow().health.clone(),
        );
        std::thread::spawn(move || {
            let _ = lh::auto_snapshot(&store, &root, &prefs);
        });
    }
    super::backup::start_timer(shared, toast.clone());
    // new books in the watched folder
    let st = InboxState::load(&root_of(shared));
    if st.folder.is_some() {
        let (books, root) = {
            let lib = shared.lib.borrow();
            (lib.books.clone(), lib.root.clone())
        };
        glib::spawn_future_local(async move {
            let found = gio::spawn_blocking(move || scan_off_thread(&st, &root, books))
                .await
                .unwrap_or_default();
            let ready = found.iter().filter(|c| c.error.is_none()).count();
            if ready > 0 {
                toast(trf(
                    "{} waiting in the import folder (Main menu → Library Health)",
                    &[&lh::plural(ready, "new book")],
                ));
            }
        });
    }
}

/// Last snapshot, then the lock goes.
pub fn on_exit(shared: &SharedRef) {
    let _ = shared.lib.borrow().save();
    if !read_only(shared) {
        let prefs = shared.cfg.borrow().health.clone();
        let _ = lh::auto_snapshot(&store_of(shared), &root_of(shared), &prefs);
        super::backup::on_exit(shared);
    }
    shared.guard.borrow_mut().lock = None;
}

// -- developer hooks (UI tests) -------------------------------------------------------------------------

/// Presses a button the way a click would: nothing happens when it is switched off.
fn press(b: &gtk::Button) {
    if b.is_sensitive() {
        b.emit_clicked();
    } else {
        eprintln!("health: button is disabled");
    }
}

/// `health:<command>` of the developer script. Opens the window and presses its real buttons.
pub fn dev(parent: &gtk::Widget, shared: &SharedRef, notify: Notify, cmd: &str) {
    if cmd == "open" {
        show(parent, shared, notify);
        return;
    }
    if let Some(tab) = cmd.strip_prefix("open-tab=") {
        show_tab(parent, shared, notify, Links::none(), tab);
        return;
    }
    if let Some(folder) = cmd.strip_prefix("layout=") {
        let (name, template) = folder.split_once('|').unwrap_or((folder, ""));
        if let Some(l) = Layout::parse(name, template) {
            change_layout(parent, shared, l, notify);
        }
        return;
    }
    let Some(ctx) = OPEN.with(|o| o.borrow().clone()) else {
        eprintln!("health: window not open for {cmd}");
        return;
    };
    if let Some(tab) = cmd.strip_prefix("tab=") {
        if let Some(t) = ctx.tabs.borrow().as_ref() {
            t.set_visible_child_name(tab);
        }
        return;
    }
    match cmd {
        "dump" => {
            let tab = ctx
                .tabs
                .borrow()
                .as_ref()
                .and_then(|t| t.visible_child_name())
                .unwrap_or_default();
            eprintln!("health_tab: {tab}");
            ctx.dump()
        }
        "trash-heading" => eprintln!(
            "health_trash_heading: {}",
            trash_heading(ctx.shared.lib.borrow().trash().len())
        ),
        "backup" => press(&ctx.backup_btn),
        "check" => press(&ctx.check_btn),
        "drift-on" => ctx.drift_switch.set_active(true),
        "restore1" => {
            let b = ctx.restore_btns.borrow().first().cloned();
            match b {
                Some(b) => press(&b),
                None => eprintln!("health: no snapshot to restore"),
            }
        }
        "fix1" => {
            // the first button of the first finding that has one
            let f = ctx.found.borrow().first().cloned();
            if let Some(f) = f {
                if let Some(fix) = f.fixes.first() {
                    ctx.apply_fix(0, fix);
                }
            }
        }
        "undo" => press(&ctx.undo_btn),
        "trash-restore1" => {
            let b = ctx.trash_btns.borrow().first().cloned();
            if let Some(b) = b {
                press(&b);
            }
        }
        "trash-empty" => press(&ctx.empty_btn),
        "tidy" => press(&ctx.tidy_apply),
        "inbox-scan" => press(&ctx.inbox_scan),
        "inbox-add-all" => press(&ctx.inbox_add_all),
        "inbox-add1" => ctx.accept_candidate(0),
        "inbox-dismiss1" => ctx.dismiss_candidate(0),
        "auto-off" => ctx.auto_switch.set_active(false),
        other => {
            if let Some(dir) = other.strip_prefix("calibre=") {
                // `calibre=<dir>` or `calibre=<dir>|nocopy`
                let (dir, mode) = dir.split_once('|').unwrap_or((dir, ""));
                ctx.calibre_copy.set_active(mode != "nocopy");
                *ctx.calibre_pick.borrow_mut() = Some(PathBuf::from(dir));
                press(&ctx.calibre_btn);
            } else if let Some(dir) = other.strip_prefix("inbox-folder=") {
                ctx.set_inbox_folder(PathBuf::from(dir));
            } else if other.starts_with("backup-") {
                let panel = ctx.panel.borrow().clone();
                if !panel.is_some_and(|p| p.dev(other)) {
                    eprintln!("health: unknown command {other}");
                }
            } else if super::health_extra::dev(other) {
            } else if let Some(n) = other.strip_prefix("keep=") {
                ctx.keep_spin.set_value(n.parse().unwrap_or(5.0));
            } else {
                eprintln!("health: unknown command {other}");
            }
        }
    }
}
