//! Two more groups of the Library Health window: *File integrity* (SHA-256 checksums of the book
//! files, `bookx_core::fixity`) and *Covers* (small or oddly shaped covers,
//! `bookx_sources::cover_quality`, with a button that looks up better ones). Roadmap PP-G-31,
//! PP-G-28, PP-C-2.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;

use bookx_core::fixity;
use bookx_sources::cover_quality as cq;
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio, glib};

use super::organize::Fetch;
use super::SharedRef;
use crate::i18n::{tr, trf};

type Notify = Rc<dyn Fn(String)>;

/// A poor cover found by the scan.
#[derive(Debug, Clone, PartialEq)]
pub struct PoorCover {
    pub id: u64,
    pub title: String,
    pub size: cq::Size,
}

/// The books whose cover is below `Good`, worst first. `covers` holds every book that has a readable cover.
pub fn poor_covers(covers: &[(u64, String, cq::Size)]) -> Vec<PoorCover> {
    let mut v: Vec<PoorCover> = covers
        .iter()
        .filter(|(_, _, s)| cq::grade(*s) < cq::Grade::Good)
        .map(|(id, title, size)| PoorCover {
            id: *id,
            title: title.clone(),
            size: *size,
        })
        .collect();
    v.sort_by_key(|c| (c.size.0 as u64 * c.size.1 as u64, c.id));
    v
}

/// "3 of 40 covers are small" for the row under the button.
pub fn cover_summary(poor: usize, total: usize) -> String {
    if total == 0 {
        tr("No covers to look at")
    } else if poor == 0 {
        trf("All {} covers are good", &[&total])
    } else {
        trf("{} of {} covers are small", &[&poor, &total])
    }
}

struct Ctx {
    shared: SharedRef,
    notify: Notify,
    fetch: Fetch,
    verify_row: adw::ActionRow,
    verify_btn: gtk::Button,
    problems: RefCell<Vec<adw::ActionRow>>,
    integrity: adw::PreferencesGroup,
    scan_row: adw::ActionRow,
    scan_btn: gtk::Button,
    lookup_btn: gtk::Button,
    covers_group: adw::PreferencesGroup,
    cover_rows: RefCell<Vec<adw::ActionRow>>,
    poor: RefCell<Vec<PoorCover>>,
}

thread_local! {
    static OPEN: RefCell<Option<Rc<Ctx>>> = const { RefCell::new(None) };
}

fn when(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

fn root_of(shared: &SharedRef) -> PathBuf {
    shared.lib.borrow().root.clone()
}

impl Ctx {
    fn last_verified(&self) -> String {
        match fixity::last_checked(&root_of(&self.shared)) {
            Some(t) => trf("Last verified {}", &[&when(t)]),
            None => tr("Never verified"),
        }
    }

    fn verify(self: &Rc<Self>) {
        let (root, files) = {
            let lib = self.shared.lib.borrow();
            let files: Vec<String> = lib
                .books
                .iter()
                .flat_map(|b| b.all_formats().into_iter().map(|f| f.file))
                .collect();
            (lib.root.clone(), files)
        };
        self.verify_btn.set_sensitive(false);
        self.verify_row.set_subtitle(&tr("Checking"));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let report = gio::spawn_blocking(move || {
                fixity::verify(
                    &root,
                    &files,
                    chrono::Utc::now().timestamp(),
                    &AtomicBool::new(false),
                )
            })
            .await;
            if let Some(ctx) = weak.upgrade() {
                ctx.verify_btn.set_sensitive(true);
                match report {
                    Ok(Ok(r)) => ctx.show_report(&r),
                    Ok(Err(e)) => ctx.verify_row.set_subtitle(&format!("{e:#}")),
                    Err(_) => ctx.verify_row.set_subtitle(&tr("The check did not finish")),
                }
            }
        });
    }

    fn show_report(self: &Rc<Self>, r: &fixity::Report) {
        for row in self.problems.borrow_mut().drain(..) {
            self.integrity.remove(&row);
        }
        self.verify_row.set_subtitle(&r.summary());
        eprintln!("health_fixity: {}", r.summary());
        for path in r.damaged() {
            let row = adw::ActionRow::new();
            row.set_title(&glib::markup_escape_text(path));
            row.set_subtitle(&tr(
                "The content changed but the date and size did not: a failing disk or a bad sync. Restore this book from a backup, or accept it as correct.",
            ));
            row.set_subtitle_lines(0);
            let accept = gtk::Button::with_label(&tr("Accept"));
            accept.set_valign(gtk::Align::Center);
            let (me, p, row2) = (Rc::downgrade(self), path.to_string(), row.clone());
            accept.connect_clicked(move |_| {
                if let Some(me) = me.upgrade() {
                    let root = root_of(&me.shared);
                    match fixity::accept(&root, &p, chrono::Utc::now().timestamp()) {
                        Ok(()) => {
                            row2.set_visible(false);
                            (me.notify)(trf("Accepted {}", &[&p]));
                        }
                        Err(e) => (me.notify)(format!("{e:#}")),
                    }
                }
            });
            row.add_suffix(&accept);
            self.integrity.add(&row);
            self.problems.borrow_mut().push(row);
        }
        for path in r.missing() {
            let row = adw::ActionRow::new();
            row.set_title(&glib::markup_escape_text(path));
            row.set_subtitle(&tr("This file is gone. Library check can remove the entry or you can restore a backup."));
            row.set_subtitle_lines(0);
            self.integrity.add(&row);
            self.problems.borrow_mut().push(row);
        }
    }

    fn scan(self: &Rc<Self>) {
        let items: Vec<(u64, String, PathBuf)> = {
            let lib = self.shared.lib.borrow();
            lib.books
                .iter()
                .filter_map(|b| lib.cover_path(b).map(|p| (b.id, b.meta.title.clone(), p)))
                .collect()
        };
        self.scan_btn.set_sensitive(false);
        self.scan_row.set_subtitle(&tr("Looking at the covers"));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let found = gio::spawn_blocking(move || {
                let covers: Vec<(u64, String, cq::Size)> = items
                    .into_iter()
                    .filter_map(|(id, t, p)| {
                        let bytes = std::fs::read(p).ok()?;
                        Some((id, t, cq::size_of(&bytes)?))
                    })
                    .collect();
                let total = covers.len();
                (poor_covers(&covers), total)
            })
            .await;
            if let (Some(ctx), Ok((poor, total))) = (weak.upgrade(), found) {
                ctx.scan_btn.set_sensitive(true);
                ctx.show_poor(poor, total);
            }
        });
    }

    fn show_poor(self: &Rc<Self>, poor: Vec<PoorCover>, total: usize) {
        for row in self.cover_rows.borrow_mut().drain(..) {
            self.covers_group.remove(&row);
        }
        self.scan_row
            .set_subtitle(&cover_summary(poor.len(), total));
        eprintln!("health_covers: poor={} total={total}", poor.len());
        for c in poor.iter().take(100) {
            let row = adw::ActionRow::new();
            row.set_title(&glib::markup_escape_text(&c.title));
            row.set_subtitle(&cq::describe_quality(c.size));
            self.covers_group.add(&row);
            self.cover_rows.borrow_mut().push(row);
        }
        self.lookup_btn.set_sensitive(!poor.is_empty());
        *self.poor.borrow_mut() = poor;
    }

    fn lookup(&self) {
        let ids: Vec<u64> = self.poor.borrow().iter().map(|c| c.id).collect();
        if !ids.is_empty() {
            (self.fetch)(ids);
        }
    }
}

/// The two groups for the Health page. `fetch` starts *Fetch Metadata* for books (the library
/// window's own handler); the review table then offers a clearly larger cover as a safe change.
pub fn groups(shared: &SharedRef, fetch: Fetch, notify: Notify) -> Vec<adw::PreferencesGroup> {
    let integrity = adw::PreferencesGroup::new();
    integrity.set_title(&tr("File integrity"));
    integrity.set_description(Some(&tr(
        "Bookx remembers a SHA-256 checksum of every book file and tells you when a file changed without being edited, which is what a failing disk or a bad sync looks like.",
    )));
    let verify_row = adw::ActionRow::new();
    verify_row.set_title(&tr("Verify book files"));
    let verify_btn = gtk::Button::with_label(&tr("Verify Files"));
    verify_btn.set_valign(gtk::Align::Center);
    verify_row.add_suffix(&verify_btn);
    integrity.add(&verify_row);

    let covers_group = adw::PreferencesGroup::new();
    covers_group.set_title(&tr("Covers"));
    covers_group.set_description(Some(&tr(
        "Finds covers that are small or blurry. The lookup proposes a larger one; you choose what to keep.",
    )));
    let scan_row = adw::ActionRow::new();
    scan_row.set_title(&tr("Find small covers"));
    let scan_btn = gtk::Button::with_label(&tr("Scan Covers"));
    scan_btn.set_valign(gtk::Align::Center);
    let lookup_btn = gtk::Button::with_label(&tr("Look Up Better Covers"));
    lookup_btn.set_valign(gtk::Align::Center);
    lookup_btn.set_sensitive(false);
    scan_row.add_suffix(&lookup_btn);
    scan_row.add_suffix(&scan_btn);
    covers_group.add(&scan_row);

    let ctx = Rc::new(Ctx {
        shared: shared.clone(),
        notify,
        fetch,
        verify_row,
        verify_btn: verify_btn.clone(),
        problems: RefCell::new(Vec::new()),
        integrity: integrity.clone(),
        scan_row,
        scan_btn: scan_btn.clone(),
        lookup_btn: lookup_btn.clone(),
        covers_group: covers_group.clone(),
        cover_rows: RefCell::new(Vec::new()),
        poor: RefCell::new(Vec::new()),
    });
    ctx.verify_row.set_subtitle(&ctx.last_verified());
    ctx.scan_row.set_subtitle(&tr("Not scanned yet"));
    {
        let ctx = ctx.clone();
        verify_btn.connect_clicked(move |_| ctx.verify());
    }
    {
        let ctx = ctx.clone();
        scan_btn.connect_clicked(move |_| ctx.scan());
    }
    {
        let ctx = ctx.clone();
        lookup_btn.connect_clicked(move |_| ctx.lookup());
    }
    OPEN.with(|o| *o.borrow_mut() = Some(ctx));
    vec![integrity, covers_group]
}

/// Developer script commands of the Health window: `verify`, `scan-covers`, `lookup-covers`
/// (the buttons of the two groups).
pub fn dev(cmd: &str) -> bool {
    let Some(ctx) = OPEN.with(|o| o.borrow().clone()) else {
        return false;
    };
    match cmd {
        "verify" => ctx.verify(),
        "scan-covers" => ctx.scan(),
        "lookup-covers" => ctx.lookup(),
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_poor_covers_are_listed_smallest_first() {
        let covers = vec![
            (1, "Big".to_string(), (800, 1200)),
            (2, "Small".to_string(), (320, 480)),
            (3, "Tiny".to_string(), (120, 180)),
            (4, "Banner".to_string(), (2000, 150)),
        ];
        let poor = poor_covers(&covers);
        assert_eq!(poor.iter().map(|c| c.id).collect::<Vec<_>>(), vec![3, 2, 4]);
        assert!(poor_covers(&[(1, "x".into(), (900, 1400))]).is_empty());
    }

    #[test]
    fn summary_wording() {
        assert_eq!(cover_summary(0, 0), "No covers to look at");
        assert_eq!(cover_summary(0, 5), "All 5 covers are good");
        assert_eq!(cover_summary(2, 5), "2 of 5 covers are small");
    }
}
