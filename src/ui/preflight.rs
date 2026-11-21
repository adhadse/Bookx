//! *Check Before Sending…*: the blocking problems and warnings of the selected books for a
//! destination (Kindle, Kobo, e-mail, any device), with a one-click fix wherever one exists.
//! Logic: `crate::preflight`.

use crate::i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use super::SharedRef;
use crate::preflight::{self as pf, BookReport, Dest, Level};

type Notify = Rc<dyn Fn(String)>;

const DESTS: [(Dest, &str); 4] = [
    (Dest::Generic, "Any device"),
    (Dest::Kindle, "Kindle"),
    (Dest::Kobo, "Kobo"),
    (Dest::Email, "E-mail"),
];

struct Ctx {
    shared: SharedRef,
    notify: Notify,
    dialog: adw::Dialog,
    ids: Vec<u64>,
    dest: Cell<Dest>,
    reports: RefCell<Vec<BookReport>>,
    /// Called by *Continue* (and at once when everything is clean).
    go: Option<Rc<dyn Fn()>>,
    page_group: adw::PreferencesGroup,
    rows: RefCell<Vec<gtk::Widget>>,
    summary: gtk::Label,
    fix_btn: gtk::Button,
    go_btn: gtk::Button,
    shown: Cell<bool>,
}

thread_local! {
    static ACTIVE: RefCell<Option<Rc<Ctx>>> = const { RefCell::new(None) };
}

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

/// Where the original of a book is kept before a fix rewrites it.
fn backup_dir(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".bookx").join("preflight-backups")
}

/// Checks `ids` for `dest`. With `go` (the step before converting or sending) the window only opens
/// when something was found, and *Continue* runs `go`; without it the window always opens.
pub fn show(
    parent: &gtk::Window,
    shared: &SharedRef,
    ids: Vec<u64>,
    dest: Dest,
    go: Option<Rc<dyn Fn()>>,
    notify: Notify,
) {
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr("Check Before Sending"));
    dialog.set_content_width(640);
    dialog.set_content_height(560);
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    let summary = gtk::Label::new(Some(&tr("Checking…")));
    summary.set_xalign(0.0);
    summary.set_wrap(true);
    group.set_header_suffix(None::<&gtk::Widget>);
    let dest_row = adw::ComboRow::new();
    dest_row.set_title(&tr("Check for"));
    dest_row.set_model(Some(&gtk::StringList::new(
        &DESTS.iter().map(|d| d.1).collect::<Vec<_>>(),
    )));
    dest_row.set_selected(DESTS.iter().position(|d| d.0 == dest).unwrap_or(0) as u32);
    let opts = adw::PreferencesGroup::new();
    opts.add(&dest_row);
    let auto = adw::SwitchRow::new();
    auto.set_title(&tr("Check automatically before converting or e-mailing"));
    auto.set_active(shared.cfg.borrow().metadata_prefs.preflight);
    let report = adw::SwitchRow::new();
    report.set_title(&tr("Save a report next to converted files"));
    report.set_active(shared.cfg.borrow().metadata_prefs.conversion_report);
    opts.add(&auto);
    opts.add(&report);
    page.add(&opts);
    page.add(&group);

    let header = adw::HeaderBar::new();
    header.set_show_start_title_buttons(false);
    header.set_show_end_title_buttons(false);
    let close = gtk::Button::with_label(&tr("Close"));
    let fix_btn = gtk::Button::with_label(&tr("Fix All"));
    fix_btn.set_sensitive(false);
    fix_btn.set_tooltip_text(Some(
        &tr("Repair what can be repaired automatically. The original of each book is kept in the library's .bookx folder."),
    ));
    let go_btn = gtk::Button::with_label(&tr("Continue"));
    go_btn.add_css_class("suggested-action");
    go_btn.set_visible(go.is_some());
    header.pack_start(&close);
    header.pack_end(&go_btn);
    header.pack_end(&fix_btn);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    summary.set_margin_start(18);
    summary.set_margin_top(8);
    body.append(&summary);
    page.set_vexpand(true);
    body.append(&page);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&body));
    dialog.set_child(Some(&view));

    let ctx = Rc::new(Ctx {
        shared: shared.clone(),
        notify,
        dialog: dialog.clone(),
        ids,
        dest: Cell::new(dest),
        reports: RefCell::new(Vec::new()),
        go,
        page_group: group,
        rows: RefCell::new(Vec::new()),
        summary,
        fix_btn: fix_btn.clone(),
        go_btn: go_btn.clone(),
        shown: Cell::new(false),
    });
    {
        let d = dialog.clone();
        close.connect_clicked(move |_| {
            d.close();
        });
    }
    {
        let c = ctx.clone();
        fix_btn.connect_clicked(move |_| c.fix_all());
    }
    {
        let c = ctx.clone();
        let d = dialog.clone();
        go_btn.connect_clicked(move |_| {
            d.close();
            if let Some(g) = &c.go {
                g();
            }
        });
    }
    {
        let c = ctx.clone();
        dest_row.connect_selected_notify(move |r| {
            c.dest.set(DESTS[(r.selected() as usize).min(3)].0);
            c.check();
        });
    }
    {
        let s = shared.clone();
        auto.connect_active_notify(move |r| {
            s.cfg.borrow_mut().metadata_prefs.preflight = r.is_active();
            s.cfg.borrow().save();
        });
        let s = shared.clone();
        report.connect_active_notify(move |r| {
            s.cfg.borrow_mut().metadata_prefs.conversion_report = r.is_active();
            s.cfg.borrow().save();
        });
    }
    {
        let c = ctx.clone();
        dialog.connect_closed(move |_| {
            ACTIVE.with(|a| {
                let mut a = a.borrow_mut();
                if a.as_ref().is_some_and(|x| Rc::ptr_eq(x, &c)) {
                    *a = None;
                }
            });
        });
    }
    ACTIVE.with(|a| *a.borrow_mut() = Some(ctx.clone()));
    ctx.check_then_present(parent.clone());
}

impl Ctx {
    fn jobs(&self) -> Vec<(u64, String, std::path::PathBuf)> {
        let lib = self.shared.lib.borrow();
        self.ids
            .iter()
            .filter_map(|id| {
                lib.get(*id)
                    .map(|b| (b.id, b.meta.title.clone(), lib.book_path(b)))
            })
            .collect()
    }

    /// Runs the checks on a worker thread and calls `done` with the reports.
    fn run(self: &Rc<Self>, done: impl Fn(Vec<BookReport>) + 'static) {
        let jobs = self.jobs();
        let dest = self.dest.get();
        self.summary.set_text(&tr("Checking…"));
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let reports: Vec<BookReport> = jobs
                .into_iter()
                .map(|(book, title, path)| {
                    let issues = pf::check_file(&path, dest).unwrap_or_else(|e| {
                        vec![pf::Issue {
                            level: Level::Blocking,
                            id: "PRE-000".into(),
                            text: trf("The file cannot be read: {}", &[&format!("{:#}", e)]),
                            fixable: false,
                        }]
                    });
                    BookReport {
                        book,
                        title,
                        issues,
                    }
                })
                .collect();
            let _ = tx.send(reports);
        });
        glib::timeout_add_local(std::time::Duration::from_millis(60), move || {
            match rx.try_recv() {
                Ok(r) => {
                    done(r);
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => glib::ControlFlow::Break,
            }
        });
    }

    fn check(self: &Rc<Self>) {
        let me = self.clone();
        self.run(move |r| me.set_reports(r));
    }

    fn check_then_present(self: &Rc<Self>, parent: gtk::Window) {
        let me = self.clone();
        self.run(move |r| {
            let clean = r.iter().all(|b| b.is_clean());
            me.set_reports(r);
            if clean && me.go.is_some() {
                // nothing to say: straight on to the conversion or the mail
                if let Some(g) = &me.go {
                    g();
                }
                ACTIVE.with(|a| *a.borrow_mut() = None);
            } else {
                me.shown.set(true);
                me.dialog.present(Some(&parent));
            }
        });
    }

    fn set_reports(&self, reports: Vec<BookReport>) {
        for w in self.rows.borrow_mut().drain(..) {
            self.page_group.remove(&w);
        }
        let blocking: usize = reports.iter().map(|r| r.blocking().len()).sum();
        let warnings: usize = reports.iter().map(|r| r.warnings().len()).sum();
        let ready = reports.iter().filter(|r| r.is_clean()).count();
        self.summary.set_text(&trf(
            "{} checked for {}: {} ready, {} blocking, {}",
            &[
                &(crate::library_health::plural(reports.len(), "book")),
                &(DESTS
                    .iter()
                    .find(|d| d.0 == self.dest.get())
                    .map_or("", |d| d.1)),
                &ready,
                &blocking,
                &crate::library_health::plural(warnings, "warning"),
            ],
        ));
        for r in reports.iter().take(200) {
            let ex = adw::ExpanderRow::new();
            ex.set_title(&esc(&r.title));
            ex.set_subtitle(&esc(&r.summary()));
            ex.add_prefix(&gtk::Image::from_icon_name(if !r.blocking().is_empty() {
                "dialog-error-symbolic"
            } else if !r.warnings().is_empty() {
                "dialog-warning-symbolic"
            } else {
                "object-select-symbolic"
            }));
            for i in &r.issues {
                let row = adw::ActionRow::new();
                row.set_title(&esc(&i.text));
                row.set_title_lines(3);
                row.set_subtitle(&esc(&format!(
                    "{} · {}",
                    i.id,
                    if i.level == Level::Blocking {
                        "blocking"
                    } else {
                        "warning"
                    }
                )));
                if i.fixable {
                    let t = gtk::Label::new(Some(&tr("can be fixed")));
                    t.add_css_class("dim-label");
                    row.add_suffix(&t);
                }
                ex.add_row(&row);
            }
            self.page_group.add(&ex);
            self.rows.borrow_mut().push(ex.upcast());
        }
        self.fix_btn
            .set_sensitive(reports.iter().any(|r| r.fixable()));
        self.go_btn.set_label(
            &(if blocking > 0 {
                tr("Continue Anyway")
            } else {
                tr("Continue")
            }),
        );
        *self.reports.borrow_mut() = reports;
        eprintln!(
            "preflight: books={} ready={ready} blocking={blocking} warnings={warnings} dest={:?}",
            self.reports.borrow().len(),
            self.dest.get()
        );
    }

    fn fix_all(self: &Rc<Self>) {
        let todo: Vec<u64> = self
            .reports
            .borrow()
            .iter()
            .filter(|r| r.fixable())
            .map(|r| r.book)
            .collect();
        let mut fixed = 0;
        let mut failed = Vec::new();
        for id in todo {
            let (path, root, title) = {
                let lib = self.shared.lib.borrow();
                match lib.get(id) {
                    Some(b) => (lib.book_path(b), lib.root.clone(), b.meta.title.clone()),
                    None => continue,
                }
            };
            let res = (|| -> anyhow::Result<usize> {
                let archive = crate::core::archive::Archive::open(&path)?;
                let (fixed_archive, lines) = pf::fix_archive(archive)?;
                if lines.is_empty() {
                    return Ok(0);
                }
                let dir = backup_dir(&root);
                std::fs::create_dir_all(&dir)?;
                std::fs::copy(
                    &path,
                    dir.join(format!(
                        "{id}-{}.epub",
                        chrono::Utc::now().format("%Y%m%dT%H%M%S")
                    )),
                )?;
                self.shared
                    .lib
                    .borrow_mut()
                    .replace_archive(id, &fixed_archive)?;
                Ok(lines.len())
            })();
            match res {
                Ok(n) if n > 0 => fixed += 1,
                Ok(_) => {}
                Err(e) => failed.push(format!("{title}: {e:#}")),
            }
        }
        eprintln!("preflight: fixed={fixed} failed={}", failed.len());
        (self.notify)(if failed.is_empty() {
            trf("Repaired {} books", &[&fixed])
        } else {
            trf(
                "Repaired {} books, {} failed: {}",
                &[&fixed, &failed.len(), &(failed[0])],
            )
        });
        self.check();
    }
}

/// `preflight:<command>` of the developer script.
pub fn dev(cmd: &str) {
    let Some(ctx) = ACTIVE.with(|a| a.borrow().clone()) else {
        eprintln!("preflight: window not open for {cmd}");
        return;
    };
    match cmd {
        "dump" => {
            for r in ctx.reports.borrow().iter() {
                eprintln!(
                    "preflight_book: {} | {} | fixable={} | {}",
                    r.title,
                    r.summary(),
                    r.fixable(),
                    r.issues
                        .iter()
                        .map(|i| i.id.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                );
            }
            eprintln!(
                "preflight_state: shown={} fix_enabled={} go_label={:?}",
                ctx.shown.get(),
                ctx.fix_btn.is_sensitive(),
                ctx.go_btn.label()
            );
        }
        "fix" => ctx.fix_all(),
        "continue" => ctx.go_btn.emit_clicked(),
        other => {
            if let Some(d) = other.strip_prefix("dest=") {
                let i = match d {
                    "kindle" => 1,
                    "kobo" => 2,
                    "email" => 3,
                    _ => 0,
                };
                ctx.dest.set(DESTS[i].0);
                ctx.check();
            }
        }
    }
}
