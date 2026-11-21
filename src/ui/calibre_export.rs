//! *Export as Calibre Library…* (PP-C-18): writes the library as a Calibre folder into an empty folder
//! the user picks. It runs as a background job (Jobs panel, can be cancelled); the library is only read.
//! The writing is `bookx_store::export_calibre`.

use crate::i18n::{tr, trf};
use std::path::PathBuf;
use std::rc::Rc;

use relm4::gtk::{self, gio, glib, prelude::*};

use super::SharedRef;

/// Asks for the folder and starts the export.
pub fn start(parent: &gtk::Window, shared: &SharedRef, toast: Rc<dyn Fn(String)>) {
    let shared = shared.clone();
    gtk::FileDialog::builder()
        .title(tr("Choose an Empty Folder for the Calibre Library"))
        .accept_label(tr("Export"))
        .build()
        .select_folder(Some(parent), gio::Cancellable::NONE, move |res| {
            if let Some(dir) = res.ok().and_then(|f| f.path()) {
                run(&shared, dir, toast);
            }
        });
}

/// Starts the export into `dir` as a background job; `toast` gets the outcome.
pub fn run(shared: &SharedRef, dir: PathBuf, toast: Rc<dyn Fn(String)>) {
    let (books, root) = {
        let lib = shared.lib.borrow();
        (lib.books.clone(), lib.root.clone())
    };
    if let Err(e) = bookx_store::check_calibre_export_target(&root, &dir) {
        let msg = trf("Could not export: {}", &[&format!("{e:#}")]);
        eprintln!("calibre_export: refused {e:#}");
        toast(msg);
        return;
    }
    let title = trf("Exporting {} to a Calibre library", &[&books.len()]);
    let (job, token) = crate::jobs::start(crate::jobs::Kind::Export, &title, true);
    let (prog_tx, prog_rx) = std::sync::mpsc::channel::<(usize, usize)>();
    let (done_tx, done_rx) =
        std::sync::mpsc::channel::<Result<bookx_store::CalibreExport, String>>();
    let (target, worker_token) = (dir.clone(), token.clone());
    std::thread::spawn(move || {
        let r = bookx_store::export_calibre(
            &books,
            &root,
            &target,
            &mut |d, n| {
                let _ = prog_tx.send((d, n));
            },
            &|| worker_token.is_cancelled(),
        )
        .map_err(|e| format!("{e:#}"));
        let _ = done_tx.send(r);
    });
    glib::timeout_add_local(std::time::Duration::from_millis(80), move || {
        while let Ok((d, n)) = prog_rx.try_recv() {
            crate::jobs::progress(
                job,
                Some(d as f64 / n.max(1) as f64),
                &trf("{} of {}", &[&d, &n]),
            );
        }
        match done_rx.try_recv() {
            Ok(Ok(r)) => {
                let msg = if r.cancelled {
                    tr("Export cancelled; nothing was kept")
                } else {
                    trf("Exported to {}: {}", &[&dir.display(), &r.summary()])
                };
                crate::jobs::log(job, &msg);
                for p in &r.problems {
                    crate::jobs::log(job, p);
                }
                eprintln!(
                    "calibre_export: books={} files={} covers={} problems={} cancelled={}",
                    r.books,
                    r.files,
                    r.covers,
                    r.problems.len(),
                    r.cancelled
                );
                crate::jobs::finish(job, Ok(()));
                // the folder can be opened from the toast; a cancelled export has nothing to open
                match super::dialog_toast::app_overlay().filter(|_| !r.cancelled) {
                    Some(o) => {
                        let dir = dir.clone();
                        super::dialog_toast::show_action(&o, &msg, &tr("Open Folder"), move || {
                            super::open_uri(&gio::File::for_path(&dir).uri())
                        });
                    }
                    None => toast(msg),
                }
                glib::ControlFlow::Break
            }
            Ok(Err(e)) => {
                eprintln!("calibre_export: failed {e}");
                crate::jobs::finish(job, Err(e.clone()));
                toast(trf("Could not export: {}", &[&e]));
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
