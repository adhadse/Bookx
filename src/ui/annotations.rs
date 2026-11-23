//! Moving annotations in and out of the library: *Import Highlights…* (Kindle clippings, KOReader,
//! Kobo database, calibre, Bookx exports) with a review before anything is added, and *Export All
//! Annotations…*. The logic is in `crate::annotation_import` and `crate::annotation_export`; this
//! file is the dialogs and a worker thread for reading files.

use crate::i18n::ntr;
use crate::i18n::{tr, trf};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio, glib};

use super::SharedRef;
use crate::annotation_export as export;
use crate::annotation_import as import;
use crate::core::toc;

thread_local! {
    /// What the open review dialog does when confirmed (the UI tests confirm through it).
    static PENDING_IMPORT: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
}

fn poll<T: 'static>(rx: std::sync::mpsc::Receiver<T>, done: impl FnOnce(T) + 'static) {
    let done = RefCell::new(Some(done));
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        match rx.try_recv() {
            Ok(v) => {
                if let Some(f) = done.borrow_mut().take() {
                    f(v);
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

/// Chapter titles by spine position, for the chapter of each annotation.
pub fn chapter_titles(lib: &crate::core::library::Library, id: u64) -> Vec<String> {
    lib.open_book(id)
        .ok()
        .map(|bk| {
            let paths = bk.spine_paths();
            let flat = toc::flatten(&bk.toc());
            paths
                .iter()
                .map(|p| {
                    flat.iter()
                        .find(|t| t.href.split('#').next() == Some(p.as_str()))
                        .map(|t| t.title.clone())
                        .unwrap_or_else(|| crate::core::util::file_name(p).to_string())
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// One sentence: what can be chosen (files, or a folder, which may be a connected e-reader) and that
/// nothing is added before the review.
pub const IMPORT_INTRO: &str = crate::i18n::N_(
    "Choose files, or a folder or e-reader, with notes from Kindle, KOReader, Kobo, calibre or Bookx; you review them before anything is added.",
);

/// The first step: asks where the highlights are (files or a folder), then reads them.
pub fn start_import(
    parent: &gtk::Window,
    shared: &SharedRef,
    toast: Rc<dyn Fn(String)>,
    changed: Rc<dyn Fn()>,
) {
    let d = adw::AlertDialog::new(Some(&tr("Import Highlights")), Some(&tr(IMPORT_INTRO)));
    d.set_widget_name("import-highlights");
    d.add_responses(&[
        ("cancel", &tr("Cancel")),
        ("folder", &tr("Choose Folder")),
        ("files", &tr("Choose Files")),
    ]);
    d.set_response_appearance("files", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("files"));
    d.set_close_response("cancel");
    let (parent2, shared, toast, changed) = (
        parent.clone(),
        shared.clone(),
        toast.clone(),
        changed.clone(),
    );
    d.connect_response(None, move |_, resp| {
        let dialog = gtk::FileDialog::builder()
            .title(tr("Import Highlights"))
            .build();
        let (parent, shared, toast, changed) = (
            parent2.clone(),
            shared.clone(),
            toast.clone(),
            changed.clone(),
        );
        match resp {
            "files" => dialog.open_multiple(Some(&parent2), gio::Cancellable::NONE, move |res| {
                if let Ok(list) = res {
                    let paths: Vec<PathBuf> = list
                        .iter::<gio::File>()
                        .flatten()
                        .filter_map(|f| f.path())
                        .collect();
                    run(&parent, &shared, paths, toast, changed);
                }
            }),
            "folder" => dialog.select_folder(Some(&parent2), gio::Cancellable::NONE, move |res| {
                if let Ok(f) = res {
                    if let Some(p) = f.path() {
                        run(&parent, &shared, vec![p], toast, changed);
                    }
                }
            }),
            _ => {}
        }
    });
    d.present(Some(parent));
}

/// Reads the paths and plans on a worker thread, then shows the review.
pub fn run(
    parent: &gtk::Window,
    shared: &SharedRef,
    paths: Vec<PathBuf>,
    toast: Rc<dyn Fn(String)>,
    changed: Rc<dyn Fn()>,
) {
    let (books, locator) = {
        let lib = shared.lib.borrow();
        let mut books = crate::device_sync::library_books(&lib.books);
        crate::device_sync::attach_looks(&mut books, &lib.root);
        (books, crate::device_sync::book_locator(&lib))
    };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (items, problems) = import::read_paths(&paths);
        let plan = import::plan(&items, &books, &locator);
        let _ = tx.send((items.len(), plan, problems));
    });
    let (parent, shared) = (parent.clone(), shared.clone());
    poll(rx, move |(found, plan, problems)| {
        for l in plan.lines() {
            eprintln!("import_summary: {l}");
        }
        for p in &problems {
            eprintln!("import_problem: {p}");
        }
        eprintln!(
            "import_plan: found={found} new={} notes={} already={} unplaced={} unmatched={}",
            plan.new_highlights(),
            plan.notes_added(),
            plan.already(),
            plan.unplaced(),
            plan.unmatched.len()
        );
        if found == 0 {
            toast(match problems.first() {
                Some(p) => trf("Nothing to import: {}", &[&p]),
                None => tr("No highlights were found there"),
            });
            return;
        }
        if plan.is_empty() {
            toast(trf(
                "Nothing new to import ({} already in your library)",
                &[&plan.already()],
            ));
            return;
        }
        let plan = Rc::new(plan);
        let apply: Rc<dyn Fn()> = {
            let (shared, toast, changed, plan) =
                (shared.clone(), toast.clone(), changed.clone(), plan.clone());
            Rc::new(move || {
                let done = {
                    let mut lib = shared.lib.borrow_mut();
                    let snap = crate::sync_bus::Snapshot::take(&lib.books);
                    let a = import::apply(&mut lib, &plan);
                    let _ = lib.save();
                    crate::sync_bus::publish_changes(
                        &snap,
                        &lib.books,
                        "",
                        crate::sync_bus::Via::Import,
                    );
                    a
                };
                eprintln!(
                    "import_done: highlights={} notes={}",
                    done.highlights, done.notes
                );
                if done.highlights + done.notes > 0 {
                    changed();
                }
                toast(trf(
                    "Imported {} and {}",
                    &[
                        &ntr("{} highlight", "{} highlights", done.highlights),
                        &ntr("{} note", "{} notes", done.notes),
                    ],
                ));
            })
        };
        PENDING_IMPORT.with(|p| *p.borrow_mut() = Some(apply.clone()));
        review_dialog(&parent, &plan, &problems, apply);
    });
}

/// The review: header Cancel and "Add N Highlights", one row per book with what it gets, and what
/// is left out. Nothing is written until the button is pressed.
fn review_dialog(
    parent: &gtk::Window,
    plan: &import::ImportPlan,
    problems: &[String],
    apply: Rc<dyn Fn()>,
) {
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr("Import Highlights"));
    dialog.set_content_width(520);
    dialog.set_content_height(520);
    let cancel = gtk::Button::with_label(&tr("Cancel"));
    let add = gtk::Button::with_label(&plan.confirm_label());
    add.add_css_class("suggested-action");
    let header = adw::HeaderBar::new();
    header.set_show_start_title_buttons(false);
    header.set_show_end_title_buttons(false);
    header.pack_start(&cancel);
    header.pack_end(&add);
    dialog.set_default_widget(Some(&add));
    let page = adw::PreferencesPage::new();
    let books = adw::PreferencesGroup::new();
    books.set_title(&tr("Books"));
    books.set_description(Some(&tr(
        "Nothing is deleted, nothing you already have is changed, and no quote is added twice.",
    )));
    for (title, what) in plan.review_rows() {
        let r = adw::ActionRow::new();
        r.set_title(&glib::markup_escape_text(&title));
        r.set_subtitle(&what);
        books.add(&r);
    }
    page.add(&books);
    let mut left = plan.left_out();
    left.extend(problems.iter().map(|p| trf("Not read: {}", &[p])));
    if !left.is_empty() {
        let g = adw::PreferencesGroup::new();
        g.set_title(&tr("Left Out"));
        for l in left {
            let r = adw::ActionRow::new();
            r.set_title(&glib::markup_escape_text(&l));
            r.set_title_lines(0);
            g.add(&r);
        }
        page.add(&g);
    }
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
    {
        let dialog = dialog.clone();
        add.connect_clicked(move |_| {
            PENDING_IMPORT.with(|p| p.borrow_mut().take());
            apply();
            dialog.close();
        });
    }
    dialog.connect_closed(|_| {
        PENDING_IMPORT.with(|p| p.borrow_mut().take());
    });
    REVIEW_LABEL.with(|l| *l.borrow_mut() = plan.confirm_label());
    dialog.present(Some(parent));
}

thread_local! {
    /// The confirm label of the last review dialog (the UI tests read it back).
    static REVIEW_LABEL: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Test hook: confirms the open review dialog.
pub fn confirm_pending() {
    eprintln!(
        "import_review: label=\"{}\"",
        REVIEW_LABEL.with(|l| l.borrow().clone())
    );
    match PENDING_IMPORT.with(|p| p.borrow_mut().take()) {
        Some(f) => f(),
        None => eprintln!("import_summary: no dialog"),
    }
}

/// *Export All Annotations…*: asks for the format and a folder, writes one file per book that has
/// annotations.
pub fn start_export_all(parent: &gtk::Window, shared: &SharedRef, toast: Rc<dyn Fn(String)>) {
    let labels: Vec<&str> = export::FORMATS.iter().map(|f| f.label).collect();
    let drop = gtk::DropDown::from_strings(&labels);
    let d = adw::AlertDialog::new(
        Some(&tr("Export All Annotations")),
        Some(&tr(
            "Writes one file per book that has highlights, notes or bookmarks, into a folder you choose.",
        )),
    );
    d.set_extra_child(Some(&drop));
    d.add_responses(&[("cancel", &tr("Cancel")), ("ok", &tr("Choose Folder"))]);
    d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    d.set_close_response("cancel");
    let (parent2, shared, toast) = (parent.clone(), shared.clone(), toast.clone());
    d.connect_response(None, move |_, resp| {
        if resp != "ok" {
            return;
        }
        let id = export::FORMATS[drop.selected() as usize].id;
        let (shared, toast, parent3) = (shared.clone(), toast.clone(), parent2.clone());
        gtk::FileDialog::builder()
            .title(tr("Choose a folder for the exported files"))
            .build()
            .select_folder(Some(&parent2), gio::Cancellable::NONE, move |res| {
                if let Some(dir) = res.ok().and_then(|f| f.path()) {
                    let (text, written) = export_all(&shared, id, &dir);
                    match super::dialog_toast::find_overlay(&parent3).filter(|_| written > 0) {
                        Some(o) => super::dialog_toast::show_action(
                            &o,
                            &text,
                            &tr("Open Folder"),
                            move || super::open_uri(&gio::File::for_path(&dir).uri()),
                        ),
                        None => toast(text),
                    }
                }
            });
    });
    d.present(Some(parent));
}

/// Writes the files of [`export::library_files`] into `dir`; returns the message to show.
pub fn export_all_to(shared: &SharedRef, id: &str, dir: &std::path::Path) -> String {
    export_all(shared, id, dir).0
}

/// [`export_all_to`] with the number of files written.
fn export_all(shared: &SharedRef, id: &str, dir: &std::path::Path) -> (String, usize) {
    let files = {
        let lib = shared.lib.borrow();
        let with: Vec<(&crate::core::library::Book, Vec<String>)> = lib
            .books
            .iter()
            .filter(|b| !b.highlights.is_empty() || !b.bookmarks.is_empty())
            .map(|b| (b, chapter_titles(&lib, b.id)))
            .collect();
        export::library_files(&with, id, chrono::Utc::now())
    };
    if files.is_empty() {
        return (
            tr("No book has highlights, notes or bookmarks to export"),
            0,
        );
    }
    let mut written = 0;
    for (name, text) in &files {
        if std::fs::write(dir.join(name), text).is_ok() {
            written += 1;
        }
    }
    eprintln!("export_all: format={id} files={written}");
    (
        trf(
            "Exported the annotations of {} to {}",
            &[&ntr("{} book", "{} books", written), &dir.display()],
        ),
        written,
    )
}
