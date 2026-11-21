//! The *Reading Trackers* window: import a Goodreads, StoryGraph or Hardcover CSV export into the
//! library (rating, shelves as tags, reading status, date read) after a preview, and export the
//! library in the layouts those services import. Logic: `crate::tracker_sync`.

use crate::i18n::tr;
use crate::i18n::{ntr, trf};
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

/// The widgets the developer hook reads back.
struct Ui {
    apply: gtk::Button,
    preview: adw::PreferencesGroup,
    rows: adw::ActionRow,
    matched: adw::ActionRow,
    changes: adw::ActionRow,
    not_found: adw::ExpanderRow,
    file: adw::ActionRow,
    not_found_rows: RefCell<Vec<adw::ActionRow>>,
    load: Rc<dyn Fn(PathBuf)>,
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<Ui>>> = const { RefCell::new(None) };
}

/// Text of the suggested button: "Import" until a file says how many books would change.
pub fn import_label(changes: usize) -> String {
    if changes == 0 {
        tr("Import")
    } else {
        ntr("Import {} Book", "Import {} Books", changes)
    }
}

fn count_row(title: &str) -> adw::ActionRow {
    let r = adw::ActionRow::new();
    r.set_title(title);
    r.add_css_class("property");
    r
}

/// Fills the preview group from a planned import: three counts and the titles that match no book.
fn fill_preview(ui: &Ui, imp: &ts::Import) {
    let p = imp.preview();
    ui.rows.set_subtitle(&p.rows.to_string());
    ui.matched.set_subtitle(&p.matched.to_string());
    ui.changes.set_subtitle(&p.changes.to_string());
    for r in ui.not_found_rows.borrow_mut().drain(..) {
        ui.not_found.remove(&r);
    }
    let total = p.not_found.len() + p.not_found_more;
    ui.not_found.set_title(&trf("Not Found ({})", &[&total]));
    ui.not_found.set_visible(total > 0);
    ui.not_found.set_expanded(false);
    let add = |title: &str| {
        let r = adw::ActionRow::new();
        r.set_title_lines(1);
        r.set_title(&gtk::glib::markup_escape_text(title));
        ui.not_found.add_row(&r);
        ui.not_found_rows.borrow_mut().push(r);
    };
    for t in &p.not_found {
        add(t);
    }
    if p.not_found_more > 0 {
        add(&trf("And {} more", &[&p.not_found_more]));
    }
    ui.preview.set_visible(true);
}

/// The window.
pub fn show(parent: &gtk::Window, shared: &SharedRef, notify: Notify) {
    let state = Rc::new(RefCell::new(State::default()));
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr("Reading Trackers"));
    dialog.set_content_width(620);
    dialog.set_content_height(560);
    let page = adw::PreferencesPage::new();

    let import_group = adw::PreferencesGroup::new();
    import_group.set_title(&tr("Import"));
    import_group.set_description(Some(&tr(
        "Choose the CSV file exported from Goodreads, StoryGraph or Hardcover. Books are matched by ISBN, then by title and author. The summary shows what would change before anything is written.",
    )));
    let choose = gtk::Button::with_label(&tr("Choose CSV File"));
    choose.set_valign(gtk::Align::Center);
    let status = adw::ActionRow::new();
    status.set_title(&tr("No file chosen"));
    status.add_suffix(&choose);
    status.set_activatable_widget(Some(&choose));
    import_group.add(&status);
    let overwrite = adw::SwitchRow::new();
    overwrite.set_title(&tr("Replace Existing Ratings"));
    overwrite.set_subtitle(&tr("Off: a book that has a rating keeps it"));
    import_group.add(&overwrite);
    page.add(&import_group);

    let preview = adw::PreferencesGroup::new();
    preview.set_title(&tr("What Would Change"));
    preview.set_visible(false);
    let rows_row = count_row(&tr("Rows in the File"));
    let matched_row = count_row(&tr("Matched with Your Books"));
    let changes_row = count_row(&tr("Would Change"));
    let not_found = adw::ExpanderRow::new();
    not_found.set_visible(false);
    preview.add(&rows_row);
    preview.add(&matched_row);
    preview.add(&changes_row);
    preview.add(&not_found);
    page.add(&preview);

    let export_group = adw::PreferencesGroup::new();
    export_group.set_title(&tr("Export"));
    export_group.set_description(Some(&tr(
        "Writes the library with reading status, rating and date read. Goodreads and Hardcover import the Goodreads layout; StoryGraph has its own.",
    )));
    let gr = gtk::Button::with_label(&tr("Export"));
    let sg = gtk::Button::with_label(&tr("Export"));
    for (btn, title) in [
        (&gr, tr("Goodreads and Hardcover")),
        (&sg, tr("StoryGraph")),
    ] {
        btn.set_valign(gtk::Align::Center);
        btn.update_property(&[gtk::accessible::Property::Label(&trf(
            "Export for {}",
            &[&title],
        ))]);
        let row = adw::ActionRow::new();
        row.set_title(&title);
        row.add_suffix(btn);
        row.set_activatable_widget(Some(btn));
        export_group.add(&row);
    }
    page.add(&export_group);

    let apply = gtk::Button::with_label(&import_label(0));
    apply.add_css_class("suggested-action");
    apply.set_sensitive(false);
    let header = adw::HeaderBar::new();
    header.pack_end(&apply);
    dialog.set_default_widget(Some(&apply));
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    let toasts = super::dialog_toast::overlay(&view);
    dialog.set_child(Some(&toasts));

    let ui: Rc<RefCell<Option<std::rc::Weak<Ui>>>> = Rc::new(RefCell::new(None));
    let load = {
        let (state, status, apply, overwrite, shared, ui) = (
            state.clone(),
            status.clone(),
            apply.clone(),
            overwrite.clone(),
            shared.clone(),
            ui.clone(),
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
                    status.set_subtitle(&trf(
                        "{} rows, {} would change",
                        &[&imp.rows.len(), &imp.updates.len()],
                    ));
                    apply.set_label(&import_label(imp.updates.len()));
                    apply.set_sensitive(!imp.updates.is_empty());
                    if let Some(ui) = ui.borrow().as_ref().and_then(|w| w.upgrade()) {
                        fill_preview(&ui, &imp);
                    }
                    let mut st = state.borrow_mut();
                    st.import = Some(imp);
                    st.path = Some(path);
                }
                Err(e) => {
                    status.set_title(&tr("Cannot use this file"));
                    status.set_subtitle(&e);
                    apply.set_label(&import_label(0));
                    apply.set_sensitive(false);
                    state.borrow_mut().import = None;
                    if let Some(ui) = ui.borrow().as_ref().and_then(|w| w.upgrade()) {
                        ui.preview.set_visible(false);
                    }
                }
            }
        })
    };
    let handles = Rc::new(Ui {
        apply: apply.clone(),
        preview: preview.clone(),
        rows: rows_row,
        matched: matched_row,
        changes: changes_row,
        not_found,
        file: status.clone(),
        not_found_rows: RefCell::new(Vec::new()),
        load: load.clone(),
    });
    *ui.borrow_mut() = Some(Rc::downgrade(&handles));
    CURRENT.with(|c| *c.borrow_mut() = Some(handles));
    dialog.connect_closed(|_| CURRENT.with(|c| *c.borrow_mut() = None));
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
        let (shared, state, notify, dialog, toasts) = (
            shared.clone(),
            state.clone(),
            notify.clone(),
            dialog.clone(),
            toasts.clone(),
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
                    notify(trf("Imported reading data for {} books ({} ratings, {} statuses). Undo takes back the ratings and tags.", &[&a.books, &a.ratings, &a.statuses]));
                    dialog.close();
                }
                Err(e) => super::dialog_toast::show_error(
                    &toasts,
                    &trf("Could not import: {}", &[&format!("{:#}", e)]),
                ),
            }
        });
    }
    for (btn, svc, name) in [
        (&gr, Service::Goodreads, "bookx-goodreads.csv"),
        (&sg, Service::StoryGraph, "bookx-storygraph.csv"),
    ] {
        let (shared, notify, parent, toasts) = (
            shared.clone(),
            notify.clone(),
            parent.clone(),
            toasts.clone(),
        );
        btn.connect_clicked(move |_| {
            let d = gtk::FileDialog::builder()
                .title(tr("Export CSV"))
                .initial_name(name)
                .build();
            let (shared, notify, toasts) = (shared.clone(), notify.clone(), toasts.clone());
            d.save(Some(&parent), gio::Cancellable::NONE, move |res| {
                if let Some(p) = res.ok().and_then(|f| f.path()) {
                    match write_export(&shared, svc, &p) {
                        Ok(n) => notify(trf("Exported {} books to {}", &[&n, &p.display()])),
                        Err(e) => super::dialog_toast::show_error(&toasts, &e),
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
        // `window=/file.csv` opens the window and loads the file; `state` prints the window
        "state" => CURRENT.with(|c| {
            if let Some(ui) = c.borrow().as_ref() {
                eprintln!(
                    "trackers_window: apply=\"{}\" apply_sensitive={} preview_visible={} matched=\"{}\" changes=\"{}\" not_found=\"{}\" not_found_rows={} file=\"{}\"",
                    ui.apply.label().unwrap_or_default(),
                    ui.apply.is_sensitive(),
                    ui.preview.is_visible(),
                    ui.matched.subtitle().unwrap_or_default(),
                    ui.changes.subtitle().unwrap_or_default(),
                    ui.not_found.title(),
                    ui.not_found_rows.borrow().len(),
                    ui.file.title(),
                );
            } else {
                eprintln!("trackers_window: closed");
            }
        }),
        // (the handles are copied out first: pressing Import closes the window)
        "press-import" => {
            let ui = CURRENT.with(|c| c.borrow().clone());
            if let Some(ui) = ui {
                ui.apply.emit_clicked();
            }
        }
        c if c.starts_with("window=") => {
            show(parent, shared, notify);
            let path = PathBuf::from(&c[7..]);
            if let Some(ui) = CURRENT.with(|c| c.borrow().clone()) {
                (ui.load)(path);
            }
        }
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

#[cfg(test)]
mod tests {
    use super::import_label;

    #[test]
    fn the_import_button_says_how_many_books_change() {
        assert_eq!(import_label(0), "Import");
        assert_eq!(import_label(1), "Import 1 Book");
        assert_eq!(import_label(12), "Import 12 Books");
    }

    /// AGENTS.md section 5: debug output stays in the developer hook, not in the window code.
    #[test]
    fn the_window_code_prints_nothing() {
        let src = include_str!("trackers.rs");
        let window = &src[..src.find("/// `trackers:<command>`").unwrap()];
        assert!(!window.contains("eprintln!"));
    }
}
