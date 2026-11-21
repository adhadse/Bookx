//! The file list: building it, add, delete, rename (with reference rewriting) and reorder.

use super::*;
use crate::i18n::ntr;
use crate::i18n::{tr, trf};

impl Editor {
    pub(super) fn rebuild_files(&mut self) {
        self.rebuild_files_ext();
    }

    // ---- structure ops --------------------------------------------------

    /// Every selected row of the file list (just the one while the list is single-select).
    pub(super) fn selected_paths(&self) -> Vec<String> {
        self.w
            .files
            .selected_rows()
            .iter()
            .filter_map(|r| unsafe { r.data::<String>("path").map(|p| p.as_ref().clone()) })
            .collect()
    }

    pub(super) fn selected_path(&self) -> Option<String> {
        self.w
            .files
            .selected_row()
            .and_then(|r| unsafe { r.data::<String>("path").map(|p| p.as_ref().clone()) })
    }

    pub(super) fn add_paths(&mut self, paths: Vec<std::path::PathBuf>) {
        let files: Vec<(String, Vec<u8>)> = paths
            .iter()
            .filter_map(|p| {
                let data = std::fs::read(p).ok()?;
                Some((p.file_name()?.to_str()?.to_string(), data))
            })
            .collect();
        if files.is_empty() {
            return;
        }
        let after = self.current.clone();
        self.run_op("Add Files", move |b, _| {
            editor_tools::add_files(b, files, after.as_deref())
        });
    }

    /// One unused file goes at once (the toast offers Undo); two or more files, or a file something refers to,
    /// ask first and say how many.
    pub(super) fn ask_delete_files(&mut self, paths: Vec<String>) {
        if paths.is_empty() {
            return;
        }
        let referenced = editor_tools::any_referenced(&self.book.borrow(), &paths);
        if !editor_tools::delete_needs_confirmation(paths.len(), referenced) {
            self.delete_files(paths);
            return;
        }
        let tx = self.sender.input_sender().clone();
        let what = if paths.len() == 1 {
            format!("“{}”", file_name(&paths[0]))
        } else {
            ntr("{} file", "{} files", paths.len())
        };
        let body = if referenced {
            trf(
                "{} will be removed from the book, but other files still refer to it, so those links will break. A checkpoint is taken first and the toast offers Undo.",
                &[&what],
            )
        } else {
            trf(
                "{} will be removed from the book. A checkpoint is taken first and the toast offers Undo.",
                &[&what],
            )
        };
        *self.ext.last_alert.borrow_mut() = format!("Delete {what}?|{body}");
        alert(
            &self.w.win,
            &trf("Delete {}?", &[&what]),
            &body,
            &tr("Delete"),
            true,
            move || tx.emit(Input::DeleteFiles(paths.clone())),
        );
    }

    pub(super) fn delete_files(&mut self, paths: Vec<String>) {
        if paths.is_empty() {
            return;
        }
        let label = if paths.len() == 1 {
            "Delete File"
        } else {
            "Delete Files"
        };
        self.run_op(label, |b, _| editor_tools::delete_paths(b, &paths));
    }

    pub(super) fn rename_file(&mut self, old: &str, new_name: &str) {
        let target = bookx_editing::files::renamed_in_place(old, new_name);
        self.run_op("Rename File", |b, _| {
            editor_tools::rename_one(b, old, new_name)
        });
        if self.book.borrow().archive.contains(&target) {
            self.select_file(&target);
        }
    }

    pub(super) fn bulk_rename(&mut self, prefix: &str, start: u32) {
        let mut paths = self.selected_paths();
        if paths.is_empty() || prefix.trim().is_empty() {
            return;
        }
        editor_tools::sort_by_reading_order(&self.book.borrow(), &mut paths);
        let plan = bookx_editing::files::bulk_rename_plan(&paths, prefix.trim(), start);
        self.run_op("Bulk Rename", |b, _| editor_tools::rename_many(b, &plan));
    }

    pub(super) fn merge_files(&mut self, paths: Vec<String>) {
        self.run_op("Merge Files", |b, _| editor_tools::merge_many(b, &paths));
    }

    pub(super) fn new_file(&mut self, name: &str, template: &str, link: bool) {
        let Some(t) = bookx_editing::files::Template::from_id(template) else {
            self.toast(&trf("Unknown template {}", &[&template]));
            return;
        };
        let after = self.current.clone();
        let created = std::cell::RefCell::new(None);
        self.run_op("New File", |b, _| {
            let o = editor_tools::new_file(b, name, t, after.as_deref(), link)?;
            *created.borrow_mut() = Some(o.summary.trim_start_matches("Created ").to_string());
            Ok(o)
        });
        if let Some(p) = created.into_inner() {
            self.select_file(&p);
        }
    }

    pub(super) fn link_styles(&mut self, css: Vec<String>, remove: bool) {
        let html: Vec<String> = self
            .selected_paths()
            .into_iter()
            .filter(|p| bookx_editing::markup::is_markup_path(p))
            .collect();
        if html.is_empty() || css.is_empty() {
            self.toast(&tr("Select pages and stylesheets to link"));
            return;
        }
        self.run_op("Link Stylesheets", |b, _| {
            editor_tools::link_styles(b, &html, &css, remove)
        });
    }

    pub(super) fn export_files(&mut self, dir: &std::path::Path) {
        self.flush();
        let paths = self.selected_paths();
        if let [one] = paths.as_slice() {
            if !dir.is_dir() {
                let data = self.book.borrow().archive.get(one).map(|d| d.to_vec());
                match data.map(|d| std::fs::write(dir, d)) {
                    Some(Ok(())) => self.toast(&trf("Exported {}", &[&file_name(one)])),
                    _ => self.toast(&tr("Could not export the file")),
                }
                return;
            }
        }
        let res = bookx_editing::files::export_files(&self.book.borrow(), &paths, dir);
        match res {
            Ok(w) => self.toast(&trf(
                "Exported {} to {}",
                &[&ntr("{} file", "{} files", w.len()), &dir.display()],
            )),
            Err(e) => self.toast(&trf("Could not export: {}", &[&e])),
        }
    }

    pub(super) fn move_file(&mut self, up: bool) {
        let Some(path) = self.selected_path() else {
            return;
        };
        self.run_op("Move File", |b, _| {
            editor_tools::move_in_spine(b, &path, up)
        });
        self.select_file(&path);
    }

    // ---- dialogs --------------------------------------------------------

    pub(super) fn ask(
        &self,
        title: &str,
        body: &str,
        extra: &impl IsA<gtk::Widget>,
        ok: &str,
        on_ok: impl Fn() + 'static,
    ) {
        let d = adw::AlertDialog::new(Some(title), Some(body));
        d.set_extra_child(Some(extra));
        d.add_responses(&[("cancel", &tr("Cancel")), ("ok", ok)]);
        d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        d.set_default_response(Some("ok"));
        d.set_close_response("cancel");
        d.connect_response(None, move |_, r| {
            if r == "ok" {
                on_ok();
            }
        });
        d.present(Some(&self.w.win));
    }

    pub(super) fn prompt_merge(&self) {
        let mut paths: Vec<String> = self
            .selected_paths()
            .into_iter()
            .filter(|p| bookx_editing::markup::is_markup_path(p))
            .collect();
        if paths.len() < 2 {
            self.toast(&tr("Select at least two pages to merge"));
            return;
        }
        editor_tools::sort_by_reading_order(&self.book.borrow(), &mut paths);
        let names: Vec<&str> = paths.iter().map(|p| file_name(p)).collect();
        let drop = gtk::DropDown::from_strings(&names);
        let tx = self.sender.input_sender().clone();
        let d = drop.clone();
        self.ask(
            "Merge Files",
            "The pages are joined in reading order into the file you choose as master. Links to the others are repaired.",
            &drop,
            "Merge",
            move || {
                let mut order = paths.clone();
                let master = order.remove(d.selected() as usize);
                order.insert(0, master);
                tx.emit(Input::MergeFiles(order));
            },
        );
    }

    pub(super) fn prompt_bulk_rename(&self) {
        if self.selected_paths().is_empty() {
            return;
        }
        let bx = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let prefix = gtk::Entry::new();
        prefix.set_placeholder_text(Some(&tr("Name prefix, for example part-")));
        prefix.set_text("part-");
        let start = gtk::SpinButton::with_range(0.0, 9999.0, 1.0);
        start.set_value(1.0);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let l = gtk::Label::new(Some(&tr("First number")));
        l.set_hexpand(true);
        l.set_xalign(0.0);
        row.append(&l);
        row.append(&start);
        bx.append(&prefix);
        bx.append(&row);
        let tx = self.sender.input_sender().clone();
        self.ask(
            "Bulk Rename",
            "The selected files are numbered in reading order. References are updated.",
            &bx,
            "Rename",
            move || {
                tx.emit(Input::BulkRename(
                    prefix.text().to_string(),
                    start.value() as u32,
                ))
            },
        );
    }

    pub(super) fn prompt_new_file(&self) {
        let bx = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let name = gtk::Entry::new();
        name.set_placeholder_text(Some(&tr("File name")));
        name.set_activates_default(true);
        let builtin = bookx_editing::files::Template::ALL.len();
        let user = crate::editor_state::user_templates();
        let labels: Vec<String> = bookx_editing::files::Template::ALL
            .iter()
            .map(|t| t.label().to_string())
            .chain(user.iter().map(|t| format!("{} (yours)", t.label)))
            .collect();
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        let drop = gtk::DropDown::from_strings(&labels);
        let link = gtk::CheckButton::with_label(&tr("Link the book’s stylesheets"));
        link.set_active(true);
        bx.append(&name);
        bx.append(&drop);
        bx.append(&link);
        let tx = self.sender.input_sender().clone();
        self.ask(
            "New File",
            "A page is added to the reading order after the open file.",
            &bx,
            "Create",
            move || {
                let sel = drop.selected() as usize;
                if sel >= builtin {
                    tx.emit(Input::Ext(tools_ui::Cmd::NewFromUser(
                        name.text().to_string(),
                        sel - builtin,
                        link.is_active(),
                    )));
                    return;
                }
                let t = bookx_editing::files::Template::ALL[sel];
                tx.emit(Input::NewFile(
                    name.text().to_string(),
                    t.id().to_string(),
                    link.is_active(),
                ));
            },
        );
    }

    pub(super) fn prompt_link_styles(&self) {
        let css: Vec<String> = bookx_editing::markup::css_files(&self.book.borrow().archive);
        let pages = self
            .selected_paths()
            .iter()
            .filter(|p| bookx_editing::markup::is_markup_path(p))
            .count();
        if css.is_empty() || pages == 0 {
            self.toast(&tr(
                "Select pages first; the book needs at least one stylesheet",
            ));
            return;
        }
        let bx = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let checks: Vec<(String, gtk::CheckButton)> = css
            .iter()
            .map(|c| {
                let b = gtk::CheckButton::with_label(file_name(c));
                b.set_active(true);
                bx.append(&b);
                (c.clone(), b)
            })
            .collect();
        let remove = gtk::CheckButton::with_label(&tr("Remove existing stylesheet links first"));
        bx.append(&remove);
        let tx = self.sender.input_sender().clone();
        self.ask(
            "Link Stylesheets",
            &trf(
                "Add the checked stylesheets to {} selected page(s), in this order.",
                &[&pages],
            ),
            &bx,
            "Link",
            move || {
                let chosen = checks
                    .iter()
                    .filter(|(_, b)| b.is_active())
                    .map(|(c, _)| c.clone())
                    .collect();
                tx.emit(Input::LinkStyles(chosen, remove.is_active()));
            },
        );
    }

    pub(super) fn prompt_export(&self) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        let tx = self.sender.input_sender().clone();
        if paths.len() == 1 {
            let dlg = gtk::FileDialog::builder()
                .title(tr("Export File"))
                .initial_name(file_name(&paths[0]))
                .build();
            dlg.save(Some(&self.w.win), gio::Cancellable::NONE, move |res| {
                if let Some(path) = res.ok().and_then(|f| f.path()) {
                    tx.emit(Input::ExportTo(path));
                }
            });
        } else {
            let dlg = gtk::FileDialog::builder()
                .title(tr("Export Files to Folder"))
                .build();
            dlg.select_folder(Some(&self.w.win), gio::Cancellable::NONE, move |res| {
                if let Some(path) = res.ok().and_then(|f| f.path()) {
                    tx.emit(Input::ExportTo(path));
                }
            });
        }
    }
}
