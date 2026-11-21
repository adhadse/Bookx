//! *Import Calibre Library* (WIN-L8): the + menu asks for the Calibre folder and the import runs as a
//! background job in the Jobs panel (progress, log), like the export in `calibre_export.rs`. The
//! result is a toast with a *View* button. No Library Health window opens. Files are copied into the
//! library when *Copy books into the library* is on in Preferences (the default).
//! The reading is `bookx_store::import_calibre_into`.

use std::path::PathBuf;
use std::rc::Rc;

use relm4::gtk::{self, gio, glib, prelude::*};

use super::SharedRef;
use crate::i18n::{tr, trf};

/// What the import calls back into the library window.
#[derive(Clone)]
pub struct Ctx {
    /// The books changed: redraw the library.
    pub refresh: Rc<dyn Fn()>,
    /// A plain toast (failures).
    pub toast: Rc<dyn Fn(String)>,
    /// The *View* button of the result toast: show the books that came in.
    pub view: Rc<dyn Fn()>,
}

/// Asks for the folder and starts the import.
pub fn start(parent: &gtk::Window, shared: &SharedRef, ctx: Ctx) {
    let shared = shared.clone();
    let dlg = gtk::FileDialog::builder()
        .title(tr("Choose the Calibre Library Folder"))
        .accept_label(tr("Import"))
        .build();
    if let Some(h) = crate::library_health::calibre_hint() {
        dlg.set_initial_folder(Some(&gio::File::for_path(h)));
    }
    dlg.select_folder(Some(parent), gio::Cancellable::NONE, move |res| {
        if let Some(dir) = res.ok().and_then(|f| f.path()) {
            run(&shared, dir, ctx);
        }
    });
}

/// The words of the result toast for `n` imported books (`Imported 5 Books`).
pub fn result_text(imported: usize) -> String {
    if imported == 0 {
        return tr("No new books to import");
    }
    crate::i18n::ntr("Imported {} book", "Imported {} books", imported)
        .replace("{}", &imported.to_string())
}

/// Starts the import of the Calibre library in `dir` as a background job.
pub fn run(shared: &SharedRef, dir: PathBuf, ctx: Ctx) {
    let store = shared.guard.borrow().store.clone();
    let root = shared.lib.borrow().root.clone();
    let copy = shared.cfg.borrow().copy_on_add;
    let title = trf("Importing the Calibre library {}", &[&dir.display()]);
    let (job, _token) = crate::jobs::start(crate::jobs::Kind::Import, &title, false);
    let (prog_tx, prog_rx) = std::sync::mpsc::channel::<(bookx_store::Stage, usize, usize)>();
    let (done_tx, done_rx) =
        std::sync::mpsc::channel::<Result<bookx_store::CalibreImport, String>>();
    let worker_dir = dir.clone();
    std::thread::spawn(move || {
        let r =
            bookx_store::import_calibre_into(&store, &root, &worker_dir, copy, &mut |s, d, n| {
                let _ = prog_tx.send((s, d, n));
            })
            .map_err(|e| format!("{e:#}"));
        let _ = done_tx.send(r);
    });
    let shared = shared.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(80), move || {
        while let Ok((stage, d, n)) = prog_rx.try_recv() {
            let part = d as f64 / n.max(1) as f64;
            // reading the catalogue is the first half when the files are copied as well
            let frac = match (stage, copy) {
                (bookx_store::Stage::Reading, true) => part / 2.0,
                (bookx_store::Stage::Copying, _) => 0.5 + part / 2.0,
                _ => part,
            };
            let what = match stage {
                bookx_store::Stage::Reading => tr("Reading the Calibre catalogue"),
                bookx_store::Stage::Copying => tr("Copying book files"),
            };
            crate::jobs::progress(job, Some(frac), &trf("{}: {} of {}", &[&what, &d, &n]));
        }
        match done_rx.try_recv() {
            Ok(Ok(out)) => {
                crate::jobs::log(job, &out.summary());
                crate::jobs::finish(job, Ok(()));
                let reloaded = super::health::reload_library(&shared);
                let reload_ok = reloaded.is_ok();
                (ctx.refresh)();
                let msg = match reloaded {
                    Ok(()) => result_text(out.report.imported),
                    Err(e) => trf(
                        "Imported, but the list could not reload: {}",
                        &[&format!("{e:#}")],
                    ),
                };
                eprintln!(
                    "calibre_import: imported={} skipped={} duplicates={} copy_failed={} reload={}",
                    out.report.imported,
                    out.skipped.len(),
                    out.duplicate_titles.len(),
                    out.copy_failed.len(),
                    reload_ok
                );
                match super::dialog_toast::app_overlay().filter(|_| out.report.imported > 0) {
                    Some(o) => {
                        let view = ctx.view.clone();
                        super::dialog_toast::show_action(&o, &msg, &tr("View"), move || view());
                    }
                    None => (ctx.toast)(msg),
                }
                glib::ControlFlow::Break
            }
            Ok(Err(e)) => {
                eprintln!("calibre_import: failed {e}");
                crate::jobs::finish(job, Err(e.clone()));
                (ctx.toast)(trf("Calibre import failed: {}", &[&e]));
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                crate::jobs::finish(job, Err("stopped".into()));
                glib::ControlFlow::Break
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_result_toast_names_the_count() {
        assert_eq!(result_text(0), "No new books to import");
        assert_eq!(result_text(1), "Imported 1 book");
        assert_eq!(result_text(12), "Imported 12 books");
    }
}
