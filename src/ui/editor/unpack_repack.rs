//! Editing books that are not EPUB (CE-429): Book → Open a Book of Another Format to Edit converts an AZW3, MOBI,
//! FB2, DOCX, RTF, HTMLZ, TXTZ… file to a temporary EPUB in memory (a KEPUB is edited as it is, spans kept). Saving
//! asks: save as an EPUB, or convert back into the original format (the original is kept as a backup). The
//! decisions and the file handling are in `editor_structure.rs`.

use std::path::Path;

use crate::editor_structure as es;
use crate::i18n::{tr, trf};

use super::struct_hub::StructCmd;
use super::tools_ui::*;
use super::*;

impl Editor {
    /// Asks which file to open (after the unsaved-changes question).
    pub(super) fn ask_open_foreign(&mut self) {
        if self.is_dirty() {
            let tx = self.sender.input_sender().clone();
            alert(
                &self.w.win,
                &tr("Discard Unsaved Changes?"),
                &tr("This window will show another book. Save first if you want to keep your changes."),
                &tr("Discard and Continue"),
                true,
                move || tx.emit(Input::Ext(Cmd::Open("open-foreign-now".into()))),
            );
            return;
        }
        self.pick_foreign_file();
    }

    pub(super) fn pick_foreign_file(&self) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(&tr("Books to edit")));
        for f in es::FOREIGN_FORMATS {
            for e in f.extensions {
                filter.add_pattern(&format!("*.{e}"));
            }
        }
        filter.add_pattern("*.kepub.epub");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dlg = gtk::FileDialog::builder()
            .title(tr("Open a Book of Another Format to Edit"))
            .filters(&filters)
            .build();
        let tx = self.sender.input_sender().clone();
        dlg.open(Some(&self.w.win), gio::Cancellable::NONE, move |res| {
            if let Some(p) = res.ok().and_then(|f| f.path()) {
                tx.emit(Input::Ext(Cmd::Struct(StructCmd::OpenForeign(p))));
            }
        });
    }

    /// Converts the file and shows it in this window. Nothing is added to the library.
    pub(super) fn open_foreign_file(&mut self, path: &Path) {
        match es::open_foreign(path) {
            Ok((archive, origin)) => match OpenBook::from_archive(archive) {
                Ok(book) => {
                    let warn = origin.format.warning;
                    let label = origin.format.label;
                    *self.ext.structure.origin.borrow_mut() = Some(origin);
                    self.install_book(0, book);
                    self.show_foreign_banner();
                    self.toast(&trf(
                        "Editing a {} book through a temporary EPUB. {}",
                        &[&label, &tr(warn)],
                    ));
                }
                Err(e) => self.toast(&trf("Could not open that book: {}", &[&format!("{e:#}")])),
            },
            Err(e) => self.toast(&trf("Could not open that book: {}", &[&format!("{e:#}")])),
        }
    }

    /// Save on a book that came from another format: choose between EPUB and the original format.
    pub(super) fn prompt_save_foreign(&mut self) {
        let Some(origin) = self.ext.structure.origin.borrow().clone() else {
            return;
        };
        let name = origin
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("book")
            .to_string();
        let can_back = origin.format.writer.is_some() || origin.format.id == "kepub";
        let body = format!(
            "{}\n\n{}",
            tr(origin.format.warning),
            if can_back {
                trf(
                    "Converting back replaces {} and keeps the old file next to it as a backup.",
                    &[&name],
                )
            } else {
                tr("Bookx cannot write this format, so the book can only be saved as an EPUB.")
            }
        );
        let d = adw::AlertDialog::new(Some(&trf("Save “{}”", &[&name])), Some(&body));
        d.add_response("cancel", &tr("Cancel"));
        d.add_response("epub", &tr("Save as EPUB"));
        if can_back {
            d.add_response("back", &trf("Convert Back to {}", &[&origin.format.label]));
        }
        // Save as EPUB is the safe default: it never touches the original file
        d.set_response_appearance("epub", adw::ResponseAppearance::Suggested);
        d.set_default_response(Some("epub"));
        d.set_close_response("cancel");
        let tx = self.sender.input_sender().clone();
        let win = self.w.win.clone();
        let stem = Path::new(&name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("book")
            .trim_end_matches(".kepub")
            .to_string();
        d.connect_response(None, move |_, r| match r {
            "back" => tx.emit(Input::Ext(Cmd::Struct(StructCmd::SaveBack))),
            "epub" => {
                let dlg = gtk::FileDialog::builder()
                    .title(tr("Save as EPUB"))
                    .initial_name(format!("{stem}.epub"))
                    .build();
                let tx = tx.clone();
                dlg.save(Some(&win), gio::Cancellable::NONE, move |res| {
                    if let Some(p) = res.ok().and_then(|f| f.path()) {
                        tx.emit(Input::Ext(Cmd::Struct(StructCmd::SaveEpub(p))));
                    }
                });
            }
            _ => {}
        });
        d.present(Some(&self.w.win));
    }

    fn mark_foreign_saved(&mut self) {
        self.modified.clear();
        self.structure_dirty = false;
        for (_, (_, dot)) in self.file_rows.iter() {
            dot.set_visible(false);
        }
        let _ = self.book.borrow_mut().reload_package();
        self.update_title();
    }

    pub(super) fn save_foreign_epub(&mut self, dest: &Path) {
        self.flush();
        if !self.commit_toc() {
            return;
        }
        let res = es::save_as_epub(&self.book.borrow().archive, dest);
        match res {
            Ok(p) => {
                self.mark_foreign_saved();
                self.toast(&trf("Saved an EPUB to {}", &[&p.display()]));
            }
            Err(e) => self.toast(&trf("Could not save: {}", &[&format!("{e:#}")])),
        }
    }

    pub(super) fn save_foreign_back(&mut self) {
        let Some(origin) = self.ext.structure.origin.borrow().clone() else {
            return;
        };
        self.flush();
        if !self.commit_toc() {
            return;
        }
        let res = es::save_back(&origin, &self.book.borrow().archive);
        match res {
            Ok(s) => {
                self.mark_foreign_saved();
                self.toast(&match &s.backup {
                    Some(b) => trf(
                        "Saved {}; the old file is kept as {}",
                        &[&s.path.display(), &b.display()],
                    ),
                    None => trf("Saved {}", &[&s.path.display()]),
                });
            }
            Err(e) => self.toast(&trf("Could not convert back: {}", &[&format!("{e:#}")])),
        }
    }

    /// `x:foreign-save=epub|<dest>` or `back`.
    pub(super) fn dev_foreign_save(&mut self, arg: &str) {
        match arg.split_once('|') {
            Some(("epub", dest)) => self.save_foreign_epub(Path::new(dest)),
            _ if arg == "back" => self.save_foreign_back(),
            _ if arg == "prompt" => self.prompt_save_foreign(),
            _ => self.toast(&tr("Unknown foreign-save command")),
        }
    }

    pub(super) fn dev_foreign_status(&self) {
        let o = self.ext.structure.origin.borrow();
        eprintln!(
            "editor_x: foreign: origin={} format={} dirty={} title={}",
            o.as_ref()
                .map(|o| o.path.display().to_string())
                .unwrap_or_default(),
            o.as_ref().map(|o| o.format.id).unwrap_or(""),
            self.is_dirty(),
            self.book_title()
        );
    }
}
