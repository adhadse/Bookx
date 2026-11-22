//! *Fetch Metadata…* for many books: the lookups run in the background, then a review table lists
//! every proposed change (book, field, current → proposed, source, confidence). Nothing is written
//! until rows are accepted and *Apply* is pressed, unless the user switched on *Apply safe
//! changes automatically* (only empty fields). Logic: `crate::bulk_fetch`.

use crate::i18n::{tr, trf};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use super::SharedRef;
use crate::bulk_fetch::{self as bf, Applied, Review};
use crate::metadata_fetch as mf;

type Notify = Rc<dyn Fn(String)>;

/// Rows drawn in the table; accepting works on all rows, also those not drawn.
const DRAWN: usize = 400;

struct Ctx {
    shared: SharedRef,
    notify: Notify,
    dialog: adw::Dialog,
    review: RefCell<Review>,
    /// Batches applied from this window, newest last (for *Undo*).
    applied: RefCell<Vec<Applied>>,
    running: std::cell::Cell<bool>,
    cancel: Arc<AtomicBool>,
    list: gtk::ListBox,
    progress: gtk::ProgressBar,
    health: gtk::Label,
    summary: gtk::Label,
    apply_btn: gtk::Button,
    undo_btn: gtk::Button,
    stop_btn: gtk::Button,
    field_box: gtk::Box,
    auto: gtk::Switch,
    checks: RefCell<Vec<gtk::CheckButton>>,
    guard: std::cell::Cell<bool>,
    /// A lookup after an import: the window is never shown.
    silent: std::cell::Cell<bool>,
}

thread_local! {
    static ACTIVE: RefCell<Option<Rc<Ctx>>> = const { RefCell::new(None) };
}

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

/// Opens the window for `ids` and starts the lookups.
pub fn start(parent: &gtk::Window, shared: &SharedRef, ids: Vec<u64>, notify: Notify) {
    start_with(parent, shared, ids, notify, false);
}

/// The lookup after an import (*Fetch metadata for new books*): no window. Only values for fields
/// the books have nothing in are written, everything else is left for *Fetch Metadata…*.
pub fn start_import(parent: &gtk::Window, shared: &SharedRef, ids: Vec<u64>, notify: Notify) {
    start_with(parent, shared, ids, notify, true);
}

fn start_with(
    parent: &gtk::Window,
    shared: &SharedRef,
    ids: Vec<u64>,
    notify: Notify,
    silent: bool,
) {
    let cover_files: std::collections::HashMap<u64, std::path::PathBuf> = {
        let lib = shared.lib.borrow();
        ids.iter()
            .filter_map(|id| lib.get(*id))
            .filter_map(|b| lib.cover_path(b).map(|p| (b.id, p)))
            .collect()
    };
    let items: Vec<(u64, crate::core::opf::Metadata, bool)> = {
        let lib = shared.lib.borrow();
        ids.iter()
            .filter_map(|id| lib.get(*id))
            .map(|b| (b.id, b.meta.clone(), b.cover.is_some()))
            .collect()
    };
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr("Fetch Metadata"));
    dialog.set_content_width(640);
    dialog.set_content_height(560);

    let progress = gtk::ProgressBar::new();
    progress.set_show_text(true);
    let health = gtk::Label::new(None);
    health.set_xalign(0.0);
    health.add_css_class("dim-label");
    health.set_wrap(true);
    let summary = gtk::Label::new(None);
    summary.set_xalign(0.0);
    summary.set_wrap(true);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    let field_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);

    let auto = gtk::Switch::new();
    auto.set_valign(gtk::Align::Center);
    auto.set_active(silent || shared.cfg.borrow().metadata_prefs.auto_apply_safe);
    let auto_row = adw::ActionRow::new();
    auto_row.set_title(&tr("Apply safe changes automatically"));
    auto_row.set_subtitle(&tr(
        "Only fills fields that are empty. Everything else waits here for review.",
    ));
    auto_row.add_suffix(&auto);
    auto_row.set_activatable_widget(Some(&auto));
    let auto_group = adw::PreferencesGroup::new();
    auto_group.add(&auto_row);

    let accept_all = gtk::Button::with_label(&tr("Accept All"));
    let accept_none = gtk::Button::with_label(&tr("Accept None"));
    let accept_safe = gtk::Button::with_label(&tr("Only Empty Fields"));
    accept_safe.set_tooltip_text(Some(&tr(
        "Accept just the rows that fill a field the book has nothing in",
    )));
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    for b in [&accept_all, &accept_none, &accept_safe] {
        bar.append(b);
    }
    let fields_scroll = gtk::ScrolledWindow::new();
    fields_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
    fields_scroll.set_child(Some(&field_box));

    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&list));

    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    for m in ["top", "bottom", "start", "end"] {
        match m {
            "top" => body.set_margin_top(12),
            "bottom" => body.set_margin_bottom(12),
            "start" => body.set_margin_start(12),
            _ => body.set_margin_end(12),
        }
    }
    body.append(&progress);
    body.append(&health);
    body.append(&summary);
    body.append(&bar);
    body.append(&fields_scroll);
    body.append(&scroll);
    body.append(&auto_group);

    let header = adw::HeaderBar::new();
    header.set_show_start_title_buttons(false);
    header.set_show_end_title_buttons(false);
    let close = gtk::Button::with_label(&tr("Close"));
    let apply_btn = gtk::Button::with_label(&tr("Apply"));
    apply_btn.add_css_class("suggested-action");
    apply_btn.set_sensitive(false);
    let undo_btn = gtk::Button::with_label(&tr("Undo Last Batch"));
    undo_btn.set_sensitive(false);
    let stop_btn = gtk::Button::with_label(&tr("Stop"));
    header.pack_start(&close);
    header.pack_start(&stop_btn);
    header.pack_end(&apply_btn);
    header.pack_end(&undo_btn);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&body));
    dialog.set_child(Some(&toolbar));

    let ctx = Rc::new(Ctx {
        shared: shared.clone(),
        notify,
        dialog: dialog.clone(),
        review: RefCell::new(Review::default()),
        applied: RefCell::new(Vec::new()),
        running: std::cell::Cell::new(true),
        cancel: Arc::new(AtomicBool::new(false)),
        list,
        progress,
        health,
        summary,
        apply_btn: apply_btn.clone(),
        undo_btn: undo_btn.clone(),
        stop_btn: stop_btn.clone(),
        field_box,
        auto: auto.clone(),
        checks: RefCell::new(Vec::new()),
        guard: std::cell::Cell::new(false),
        silent: std::cell::Cell::new(false),
    });

    {
        let c = ctx.clone();
        close.connect_clicked(move |_| {
            c.cancel.store(true, Ordering::Relaxed);
            c.dialog.close();
        });
    }
    {
        let c = ctx.clone();
        stop_btn.connect_clicked(move |_| c.cancel.store(true, Ordering::Relaxed));
    }
    {
        let c = ctx.clone();
        auto.connect_active_notify(move |s| {
            c.shared.cfg.borrow_mut().metadata_prefs.auto_apply_safe = s.is_active();
            c.shared.cfg.borrow().save();
        });
    }
    for (b, f) in [
        (
            &accept_all,
            Box::new(|r: &mut Review| r.accept_all(true)) as Box<dyn Fn(&mut Review)>,
        ),
        (&accept_none, Box::new(|r: &mut Review| r.accept_all(false))),
        (&accept_safe, Box::new(|r: &mut Review| r.accept_safe())),
    ] {
        let c = ctx.clone();
        b.connect_clicked(move |_| {
            f(&mut c.review.borrow_mut());
            c.sync_checks();
        });
    }
    {
        let c = ctx.clone();
        apply_btn.connect_clicked(move |_| c.apply());
    }
    {
        let c = ctx.clone();
        undo_btn.connect_clicked(move |_| c.undo());
    }
    {
        let c = ctx.clone();
        dialog.connect_closed(move |_| {
            c.cancel.store(true, Ordering::Relaxed);
            ACTIVE.with(|a| {
                let mut a = a.borrow_mut();
                if a.as_ref().is_some_and(|x| Rc::ptr_eq(x, &c)) {
                    *a = None;
                }
            });
        });
    }
    ctx.silent.set(silent);
    ACTIVE.with(|a| *a.borrow_mut() = Some(ctx.clone()));
    ctx.update_state();
    if !silent {
        dialog.present(Some(parent));
    }
    ctx.run(items, cover_files);
}

impl Ctx {
    fn run(
        self: &Rc<Self>,
        items: Vec<(u64, crate::core::opf::Metadata, bool)>,
        cover_files: std::collections::HashMap<u64, std::path::PathBuf>,
    ) {
        let total = items.len();
        self.progress.set_text(Some(&trf("0 of {}", &[&total])));
        let (tx, rx) = mpsc::channel::<Msg>();
        let cancel = self.cancel.clone();
        let (job, token) = crate::jobs::start(
            crate::jobs::Kind::Fetch,
            &format!(
                "Fetching metadata for {}",
                crate::library_health::plural(total, "book")
            ),
            true,
        );
        let job_cancel = self.cancel.clone();
        // a polite pause between books, none for the canned answers of the tests
        let pause = if std::env::var_os("BOOKX_SOURCES_MOCK").is_some() {
            0
        } else {
            250
        };
        std::thread::spawn(move || {
            let tx2 = tx.clone();
            let plugins = crate::plugins::current();
            let mut results = bf::run_batch(
                items,
                mf::fetcher(),
                &plugins,
                pause,
                &cancel,
                &move |d, t| {
                    let _ = tx2.send(Msg::Progress(d, t));
                },
            );
            // how big the covers are now, so a clearly larger one can replace a poor one
            for r in results.iter_mut().filter(|r| r.has_cover) {
                r.cover_px = cover_files
                    .get(&r.book)
                    .and_then(|p| std::fs::read(p).ok())
                    .and_then(|b| bookx_sources::cover_quality::size_of(&b));
            }
            mf::persist_health();
            let _ = tx.send(Msg::Done(results));
        });
        let me = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(80), move || {
            me.health.set_text(&bf::health_line(&mf::health_board()));
            if token.is_cancelled() {
                job_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            loop {
                match rx.try_recv() {
                    Ok(Msg::Progress(d, t)) => {
                        crate::jobs::progress(
                            job,
                            Some(d as f64 / t.max(1) as f64),
                            &format!("{d} of {t}"),
                        );
                        me.progress.set_fraction(d as f64 / t.max(1) as f64);
                        me.progress.set_text(Some(&trf("{} of {}", &[&d, &t])));
                    }
                    Ok(Msg::Done(results)) => {
                        let found = results.iter().filter(|r| r.found.is_some()).count();
                        crate::jobs::log(job, &format!("{found} of {} books found", results.len()));
                        crate::jobs::finish(job, Ok(()));
                        me.finish(results);
                        return glib::ControlFlow::Break;
                    }
                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        crate::jobs::finish(job, Err("stopped unexpectedly".into()));
                        me.running.set(false);
                        me.update_state();
                        return glib::ControlFlow::Break;
                    }
                }
            }
        });
    }

    fn finish(self: &Rc<Self>, results: Vec<bf::BookResult>) {
        self.running.set(false);
        *self.review.borrow_mut() = Review::new(results);
        self.health.set_text(&bf::health_line(&mf::health_board()));
        self.progress.set_visible(false);
        self.stop_btn.set_visible(false);
        if self.auto.is_active() {
            let rows = {
                let mut r = self.review.borrow_mut();
                r.accept_safe();
                r.take_accepted()
            };
            if !rows.is_empty() {
                self.write(&rows, "Applied automatically");
            }
        }
        self.rebuild();
        if self.silent.get() {
            let left = self.review.borrow().rows.len();
            let found = self
                .review
                .borrow()
                .results
                .iter()
                .filter(|r| r.found.is_some())
                .count();
            let total = self.review.borrow().results.len();
            eprintln!(
                "bulkfetch: import lookup found={found} of {total}, {left} proposals left for review"
            );
            if left > 0 {
                (self.notify)(format!(
                    "{left} more proposals wait: select the books and use Fetch Metadata… to review them"
                ));
            }
        }
    }

    fn write(self: &Rc<Self>, rows: &[bf::Row], what: &str) {
        let now = chrono::Utc::now().timestamp();
        let applied = {
            let review = self.review.borrow();
            let mut lib = self.shared.lib.borrow_mut();
            let guard = self.shared.guard.borrow();
            bf::apply_rows(&mut lib, &guard.store, &review, rows, now)
        };
        let msg = if applied.errors.is_empty() {
            format!(
                "{what}: {} fields in {} books",
                applied.fields, applied.books
            )
        } else {
            format!(
                "{what}: {} books, {} failed ({})",
                applied.books,
                applied.errors.len(),
                applied.errors[0]
            )
        };
        eprintln!(
            "bulkfetch: applied books={} fields={} ops={} errors={}",
            applied.books,
            applied.fields,
            applied.ops,
            applied.errors.len()
        );
        if applied.ops > 0 {
            self.applied.borrow_mut().push(applied);
        }
        (self.notify)(msg);
    }

    fn apply(self: &Rc<Self>) {
        let rows = self.review.borrow_mut().take_accepted();
        if rows.is_empty() {
            return;
        }
        self.write(&rows, "Applied");
        self.rebuild();
    }

    fn undo(self: &Rc<Self>) {
        let Some(batch) = self.applied.borrow_mut().pop() else {
            return;
        };
        let res = {
            let mut lib = self.shared.lib.borrow_mut();
            let guard = self.shared.guard.borrow();
            bf::undo_applied(&mut lib, &guard.store, &batch)
        };
        match res {
            Ok(n) => (self.notify)(trf("Undone: {} books restored", &[&n])),
            Err(e) => (self.notify)(trf("Could not undo: {}", &[&format!("{:#}", e)])),
        }
        eprintln!("bulkfetch: undone");
        self.update_state();
    }

    fn update_state(&self) {
        let r = self.review.borrow();
        self.apply_btn.set_sensitive(r.accepted() > 0);
        self.apply_btn.set_label(&if r.accepted() > 0 {
            trf("Apply {}", &[&r.accepted()])
        } else {
            "Apply".into()
        });
        self.undo_btn
            .set_sensitive(!self.applied.borrow().is_empty());
        if self.running.get() {
            self.summary.set_text(&tr("Looking the books up"));
        } else {
            let mut s = r.summary();
            if r.not_found() > 0 {
                s += &format!(", {} not found", r.not_found());
            }
            self.summary.set_text(&s);
        }
    }

    /// Redraws the table and the per-field buttons after the rows changed.
    fn rebuild(self: &Rc<Self>) {
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        while let Some(c) = self.field_box.first_child() {
            self.field_box.remove(&c);
        }
        self.checks.borrow_mut().clear();
        let r = self.review.borrow();
        for f in r.fields_present() {
            let n = r.rows.iter().filter(|x| x.field == f).count();
            let b = gtk::Button::with_label(&trf("All {} ({})", &[&f, &n]));
            b.add_css_class("flat");
            let me = self.clone();
            b.connect_clicked(move |_| {
                me.review.borrow_mut().accept_field(f, true);
                me.sync_checks();
            });
            self.field_box.append(&b);
        }
        for (i, row) in r.rows.iter().enumerate().take(DRAWN) {
            let ar = adw::ActionRow::new();
            ar.set_title(&esc(&format!("{} · {}", row.book_title, row.field)));
            let cur = if row.current.is_empty() {
                "(empty)"
            } else {
                &row.current
            };
            ar.set_subtitle(&esc(&format!("{cur} → {}", row.proposed)));
            ar.set_subtitle_lines(3);
            let check = gtk::CheckButton::new();
            check.update_property(&[gtk::accessible::Property::Label(&trf(
                "Accept {} of {}",
                &[&row.field, &row.book_title],
            ))]);
            check.set_active(row.accepted);
            let me = self.clone();
            check.connect_toggled(move |c| {
                if me.guard.get() {
                    return;
                }
                let mut rev = me.review.borrow_mut();
                if let Some(x) = rev.rows.get_mut(i) {
                    x.accepted = c.is_active();
                }
                drop(rev);
                me.update_state();
            });
            ar.add_prefix(&check);
            ar.set_activatable_widget(Some(&check));
            let tag = gtk::Label::new(Some(&format!(
                "{} · {}%{}",
                bookx_sources::health::display_name(&row.source),
                (row.confidence * 100.0).round() as u32,
                if row.safe { " · fills empty" } else { "" }
            )));
            tag.add_css_class("dim-label");
            tag.add_css_class("caption");
            ar.add_suffix(&tag);
            self.list.append(&ar);
            self.checks.borrow_mut().push(check);
        }
        if r.rows.len() > DRAWN {
            let more = adw::ActionRow::new();
            more.set_title(&trf(
                "… and {} more rows (Accept All includes them)",
                &[&(r.rows.len() - DRAWN)],
            ));
            self.list.append(&more);
        }
        if r.rows.is_empty() && !self.running.get() {
            let msg = adw::ActionRow::new();
            msg.set_title(
                &(if r.results.is_empty() {
                    tr("Nothing to look up")
                } else {
                    tr("Nothing to review")
                }),
            );
            msg.set_subtitle(&esc(&r.failures().first().map_or_else(
                || "The sources found nothing new for these books.".to_string(),
                |(t, e)| format!("{t}: {e}"),
            )));
            self.list.append(&msg);
        }
        drop(r);
        self.update_state();
    }

    /// Makes the check buttons show the state of the rows.
    fn sync_checks(&self) {
        self.guard.set(true);
        {
            let r = self.review.borrow();
            for (c, row) in self.checks.borrow().iter().zip(r.rows.iter()) {
                c.set_active(row.accepted);
            }
        }
        self.guard.set(false);
        self.update_state();
    }
}

enum Msg {
    Progress(usize, usize),
    Done(Vec<bf::BookResult>),
}

/// `bulkfetch:<command>` of the developer script (`library:bulkfetch` starts the window).
pub fn dev(cmd: &str) {
    if cmd == "cache" {
        let c = mf::cache_stats();
        eprintln!("bulkfetch_cache: hits={} misses={}", c.hits, c.misses);
        return;
    }
    let Some(ctx) = ACTIVE.with(|a| a.borrow().clone()) else {
        eprintln!("bulkfetch: window not open for {cmd}");
        return;
    };
    match cmd {
        "dump" => {
            let r = ctx.review.borrow();
            eprintln!(
                "bulkfetch: state={} rows={} accepted={} applied_batches={} auto={}",
                if ctx.running.get() {
                    "running"
                } else {
                    "review"
                },
                r.rows.len(),
                r.accepted(),
                ctx.applied.borrow().len(),
                ctx.auto.is_active()
            );
            eprintln!("bulkfetch: {}", ctx.summary.text());
            eprintln!("bulkfetch_health: {}", ctx.health.text());
            for (i, x) in r.rows.iter().enumerate() {
                eprintln!(
                    "bulkrow: {i} book={:?} field={} current={:?} proposed={:?} source={:?} conf={} safe={} accepted={}",
                    x.book_title,
                    x.field,
                    x.current,
                    x.proposed,
                    x.source,
                    (x.confidence * 100.0).round() as u32,
                    x.safe,
                    x.accepted
                );
            }
        }
        "accept-all" => ctx.accept(|r| r.accept_all(true)),
        "accept-none" => ctx.accept(|r| r.accept_all(false)),
        "accept-safe" => ctx.accept(|r| r.accept_safe()),
        "apply" => ctx.apply(),
        "undo" => ctx.undo(),
        "auto-on" => ctx.auto.set_active(true),
        "auto-off" => ctx.auto.set_active(false),
        "close" => {
            ctx.dialog.close();
        }
        other => {
            if let Some(f) = other.strip_prefix("accept-field=") {
                ctx.accept(|r| r.accept_field(f, true));
            } else if let Some(i) = other.strip_prefix("toggle=").and_then(|i| i.parse().ok()) {
                ctx.accept(|r| r.toggle(i));
            } else if let Some(i) = other
                .strip_prefix("check=")
                .and_then(|i| i.parse::<usize>().ok())
            {
                // the user's click on the check button of a row
                let b = ctx.checks.borrow().get(i).cloned();
                if let Some(b) = b {
                    b.set_active(!b.is_active());
                }
            }
        }
    }
}

impl Ctx {
    fn accept(&self, f: impl FnOnce(&mut Review)) {
        f(&mut self.review.borrow_mut());
        self.sync_checks();
    }
}
