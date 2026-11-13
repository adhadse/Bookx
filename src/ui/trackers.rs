//! The *Reading Trackers* window: import a Goodreads, StoryGraph or Hardcover CSV export into the
//! library (rating, shelves as tags, reading status, date read) after a preview, and export the
//! library in the layouts those services import. Logic: `crate::tracker_sync`.

use crate::i18n::tr;
use crate::i18n::trf;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use bookx_sources::trackers::Service;
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio};

use super::SharedRef;
use crate::tracker_sync as ts;

/// Saves, redraws, toasts.
pub type Notify = Rc<dyn Fn(String)>;

/// What is loaded in the window.
#[derive(Default)]
struct State {
    import: Option<ts::Import>,
    path: Option<PathBuf>,
}

fn read_csv(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

fn service_of(name: &str) -> Service {
    if name.eq_ignore_ascii_case("storygraph") {
        Service::StoryGraph
    } else {
        Service::Goodreads
    }
}

fn write_export(shared: &SharedRef, service: Service, path: &Path) -> Result<usize, String> {
    let csv = {
        let lib = shared.lib.borrow();
        let guard = shared.guard.borrow();
        ts::export(&lib, &guard.store, service)
    };
    std::fs::write(path, &csv).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(shared.lib.borrow().books.len())
}

/// The window.
pub fn show(parent: &gtk::Window, shared: &SharedRef, notify: Notify) {
    let state = Rc::new(RefCell::new(State::default()));
    let dialog = adw::Dialog::new();
    dialog.set_title(&tr("Reading Trackers"));
    dialog.set_content_width(620);
    dialog.set_content_height(640);
    let page = adw::PreferencesPage::new();

    let import_group = adw::PreferencesGroup::new();
    import_group.set_title(&tr("Import"));
    import_group.set_description(Some(
        "Choose the CSV file you exported from Goodreads, StoryGraph or Hardcover. Books are matched by ISBN, then by title and author. You see what would change before anything is written.",
    ));
    let choose = gtk::Button::with_label(&tr("Choose CSV File"));
    choose.set_valign(gtk::Align::Center);
    let status = adw::ActionRow::new();
    status.set_title(&tr("No file chosen"));
    status.add_suffix(&choose);
    import_group.add(&status);
    let overwrite = adw::SwitchRow::new();
    overwrite.set_title(&tr("Replace ratings I already have"));
    overwrite.set_subtitle(&tr("Off: a book that has a rating keeps it"));
    import_group.add(&overwrite);
    let apply = gtk::Button::with_label(&tr("Import"));
    apply.add_css_class("suggested-action");
    apply.set_sensitive(false);
    apply.set_halign(gtk::Align::End);
    import_group.add(&apply);
    page.add(&import_group);

    let export_group = adw::PreferencesGroup::new();
    export_group.set_title(&tr("Export"));
    export_group.set_description(Some(
        "Writes your library with reading status, rating and date read. Goodreads and Hardcover import the Goodreads layout; StoryGraph has its own.",
    ));
    let gr = gtk::Button::with_label(&tr("Goodreads / Hardcover"));
    let sg = gtk::Button::with_label(&tr("StoryGraph"));
    let row = adw::ActionRow::new();
    row.set_title(&tr("Export CSV"));
    row.add_suffix(&gr);
    row.add_suffix(&sg);
    export_group.add(&row);
    page.add(&export_group);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    dialog.set_child(Some(&view));

    let load = {
        let (state, status, apply, overwrite, shared) = (
            state.clone(),
            status.clone(),
            apply.clone(),
            overwrite.clone(),
            shared.clone(),
        );
        Rc::new(move |path: PathBuf| {
            let plan = read_csv(&path).and_then(|t| {
                ts::plan_import(&t, &shared.lib.borrow().books, overwrite.is_active())
            });
            match plan {
                Ok(imp) => {
                    status.set_title(
                        &path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default(),
                    );
                    let missing: Vec<String> = imp
                        .unmatched()
                        .iter()
                        .take(3)
                        .map(|r| r.title.clone())
                        .collect();
                    status.set_subtitle(&format!(
                        "{}{}",
                        imp.summary(),
                        if missing.is_empty() {
                            String::new()
                        } else {
                            format!(" (not found: {}…)", missing.join(", "))
                        }
                    ));
                    eprintln!("trackers: {}", imp.summary());
                    apply.set_sensitive(!imp.updates.is_empty());
                    let mut st = state.borrow_mut();
                    st.import = Some(imp);
                    st.path = Some(path);
                }
                Err(e) => {
                    status.set_title(&tr("Cannot use this file"));
                    status.set_subtitle(&e);
                    eprintln!("trackers: error {e}");
                    apply.set_sensitive(false);
                    state.borrow_mut().import = None;
                }
            }
        })
    };
    {
        let (load, parent) = (load.clone(), parent.clone());
        choose.connect_clicked(move |_| {
            let d = gtk::FileDialog::builder()
                .title(tr("Reading Tracker Export"))
                .build();
            let load = load.clone();
            d.open(Some(&parent), gio::Cancellable::NONE, move |res| {
                if let Some(p) = res.ok().and_then(|f| f.path()) {
                    load(p);
                }
            });
        });
    }
    {
        let (load, state) = (load.clone(), state.clone());
        overwrite.connect_active_notify(move |_| {
            if let Some(p) = state.borrow().path.clone() {
                load(p);
            }
        });
    }
    {
        let (shared, state, notify, dialog) = (
            shared.clone(),
            state.clone(),
            notify.clone(),
            dialog.clone(),
        );
        apply.connect_clicked(move |_| {
            let Some(imp) = state.borrow().import.clone() else {
                return;
            };
            let res = {
                let guard = shared.guard.borrow();
                ts::apply(&mut shared.lib.borrow_mut(), &guard.store, &imp.updates)
            };
            match res {
                Ok(a) => {
                    eprintln!(
                        "trackers: applied books={} ratings={} tags={} statuses={} dates={}",
                        a.books, a.ratings, a.tags, a.statuses, a.dates
                    );
                    notify(trf("Imported reading data for {} books ({} ratings, {} statuses). Undo takes back the ratings and tags.", &[&a.books, &a.ratings, &a.statuses]));
                    dialog.close();
                }
                Err(e) => notify(trf("Could not import: {}", &[&format!("{:#}", e)])),
            }
        });
    }
    for (btn, svc, name) in [
        (&gr, Service::Goodreads, "bookx-goodreads.csv"),
        (&sg, Service::StoryGraph, "bookx-storygraph.csv"),
    ] {
        let (shared, notify, parent) = (shared.clone(), notify.clone(), parent.clone());
        btn.connect_clicked(move |_| {
            let d = gtk::FileDialog::builder()
                .title(tr("Export CSV"))
                .initial_name(name)
                .build();
            let (shared, notify) = (shared.clone(), notify.clone());
            d.save(Some(&parent), gio::Cancellable::NONE, move |res| {
                if let Some(p) = res.ok().and_then(|f| f.path()) {
                    match write_export(&shared, svc, &p) {
                        Ok(n) => notify(trf("Exported {} books to {}", &[&n, &p.display()])),
                        Err(e) => notify(e),
                    }
                }
            });
        });
    }
    dialog.present(Some(parent));
}

/// `trackers:<command>` of the developer script.
pub fn dev(parent: &gtk::Window, shared: &SharedRef, notify: Notify, cmd: &str) {
    match cmd {
        "open" => show(parent, shared, notify),
        // `plan=/file.csv` prints what an import would do; `import=/file.csv` does it
        c if c.starts_with("plan=") || c.starts_with("import=") => {
            let (apply, path) = match c.split_once('=') {
                Some(("import", p)) => (true, p),
                Some((_, p)) => (false, p),
                None => return,
            };
            let plan = read_csv(Path::new(path))
                .and_then(|t| ts::plan_import(&t, &shared.lib.borrow().books, false));
            match plan {
                Ok(imp) => {
                    eprintln!("trackers: {}", imp.summary());
                    for u in &imp.updates {
                        eprintln!(
                            "trackers_update: {} rating={:?} status={:?} date={:?} tags={}",
                            u.title,
                            u.rating,
                            u.status.map(|s| format!("{s:?}")),
                            u.date_read.map(|d| d.to_string()),
                            u.add_tags.join(",")
                        );
                    }
                    if apply {
                        let res = {
                            let guard = shared.guard.borrow();
                            ts::apply(&mut shared.lib.borrow_mut(), &guard.store, &imp.updates)
                        };
                        match res {
                            Ok(a) => eprintln!(
                                "trackers: applied books={} ratings={} tags={} statuses={} dates={}",
                                a.books, a.ratings, a.tags, a.statuses, a.dates
                            ),
                            Err(e) => eprintln!("trackers: apply failed {e:#}"),
                        }
                        notify(tr("Imported"));
                    }
                }
                Err(e) => eprintln!("trackers: error {e}"),
            }
        }
        // `export=goodreads:/file.csv`
        c if c.starts_with("export=") => {
            if let Some((svc, path)) = c[7..].split_once(':') {
                match write_export(shared, service_of(svc), Path::new(path)) {
                    Ok(n) => eprintln!("trackers: exported {n} books"),
                    Err(e) => eprintln!("trackers: export failed {e}"),
                }
            }
        }
        _ => {}
    }
}
