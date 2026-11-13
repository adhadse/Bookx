//! Tools menu operations, checkpoints, the book report, spelling, the checker entry point and the results panel.

use super::*;
use crate::i18n::{tr, trf};

impl Editor {
    pub(super) fn refresh_restore_menu(&self) {
        self.w.compare_menu.remove_all();
        for c in self.checkpoints.list().iter().rev() {
            let item = gio::MenuItem::new(Some(&c.name), None);
            item.set_action_and_target_value(
                Some("tools.compare"),
                Some(&c.id.to_string().to_variant()),
            );
            self.w.compare_menu.append_item(&item);
        }
        self.w.restore_menu.remove_all();
        for c in self.checkpoints.list().iter().rev() {
            let when = chrono::DateTime::from_timestamp(c.created, 0)
                .map(|d| {
                    d.with_timezone(&chrono::Local)
                        .format("%H:%M:%S")
                        .to_string()
                })
                .unwrap_or_default();
            let item = gio::MenuItem::new(Some(&format!("{} ({when})", c.name)), None);
            item.set_action_and_target_value(
                Some("tools.restore"),
                Some(&c.id.to_string().to_variant()),
            );
            self.w.restore_menu.append_item(&item);
        }
    }

    pub(super) fn create_checkpoint(&mut self, name: &str) -> u64 {
        self.flush();
        let id = self.checkpoints.create(name, &self.book.borrow().archive);
        self.refresh_restore_menu();
        id
    }

    pub(super) fn run_tool(&mut self, tool: Tool) {
        self.run_op(&tr(tool.label()), |b, _| editor_tools::run(tool, b));
    }

    /// Runs a transform plugin on the open book.
    pub(super) fn run_plugin_tool(&mut self, id: &str) {
        let loaded = crate::plugins::current();
        let label = loaded
            .names
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.to_string());
        let id = id.to_string();
        self.run_op(&label, move |b, _| {
            let (files, notes) = crate::plugins::run_transform(&loaded, &id, b)?;
            let summary = match files {
                0 => "Nothing changed".to_string(),
                1 => "Changed 1 file".to_string(),
                n => trf("Changed {} files", &[&n]),
            };
            Ok(editor_tools::Outcome {
                changed: files > 0,
                summary,
                details: notes,
            })
        });
    }

    /// Runs an operation on the book (given the file selected in the editor, if any). The book is
    /// snapshotted first, so every operation can be undone, and rolled back if it fails.
    pub(super) fn run_op(
        &mut self,
        label: &str,
        op: impl FnOnce(
            &mut crate::core::book::OpenBook,
            Option<&str>,
        ) -> anyhow::Result<editor_tools::Outcome>,
    ) {
        self.flush();
        self.commit_toc();
        let snapshot = self.create_checkpoint(&format!("Before: {label}"));
        let current = self.current.clone();
        let res = op(&mut self.book.borrow_mut(), current.as_deref());
        match res {
            Ok(o) => {
                if o.changed {
                    self.after_archive_changed();
                    let rows: Vec<_> = o
                        .details
                        .iter()
                        .map(|d| {
                            (
                                "object-select-symbolic".to_string(),
                                d.clone(),
                                String::new(),
                                "",
                            )
                        })
                        .collect();
                    self.results = rows
                        .iter()
                        .map(|_| ResultRow {
                            file: String::new(),
                            line: None,
                            range: None,
                        })
                        .collect();
                    self.result_kind = ResultKind::Check;
                    if !rows.is_empty() {
                        self.show_results(&o.summary, rows);
                    }
                    self.toast(&trf(
                        "{} — save to keep your changes, or restore the checkpoint to undo",
                        &[&o.summary],
                    ));
                } else {
                    // nothing changed: the snapshot is not needed
                    let _ = self.checkpoints.delete(snapshot);
                    self.refresh_restore_menu();
                    self.toast(&o.summary);
                }
            }
            Err(e) => {
                // the tool may have stopped half way: go back to the snapshot
                let _ = self
                    .checkpoints
                    .restore_into(snapshot, &mut self.book.borrow_mut().archive);
                self.after_archive_changed();
                self.toast(&trf("{} failed: {}", &[&label, &format!("{:#}", e)]));
            }
        }
    }

    /// Lists what differs between a checkpoint and the book as it is now.
    pub(super) fn compare_checkpoint(&mut self, id: u64) {
        self.flush();
        let name = self
            .checkpoints
            .list()
            .into_iter()
            .find(|c| c.id == id)
            .map(|c| c.name)
            .unwrap_or_default();
        let diff = self.checkpoints.diff_against(
            id,
            &self.book.borrow().archive,
            bookx_editing::compare::CompareOptions {
                normalize_xml: true,
            },
        );
        let diff = match diff {
            Ok(d) => d,
            Err(e) => {
                self.toast(&trf("Could not compare: {}", &[&e]));
                return;
            }
        };
        if diff.is_empty() {
            self.toast(&trf("No differences from “{}”", &[&name]));
            return;
        }
        let mut rows: Vec<(String, String, String, &'static str)> = Vec::new();
        for p in &diff.added {
            rows.push(("list-add-symbolic".into(), p.clone(), "added".into(), ""));
        }
        for p in &diff.removed {
            rows.push((
                "list-remove-symbolic".into(),
                p.clone(),
                "removed".into(),
                "",
            ));
        }
        for (a, b) in &diff.renamed {
            rows.push((
                "document-edit-symbolic".into(),
                format!("{a} → {b}"),
                "renamed".into(),
                "",
            ));
        }
        for c in &diff.changed {
            rows.push((
                "document-edit-symbolic".into(),
                c.path.clone(),
                format!("changed ({} → {} bytes)", c.old_size, c.new_size),
                "",
            ));
        }
        self.results = rows
            .iter()
            .map(|r| ResultRow {
                file: if self.book.borrow().archive.contains(&r.1) {
                    r.1.clone()
                } else {
                    String::new()
                },
                line: None,
                range: None,
            })
            .collect();
        self.result_kind = ResultKind::Check;
        let title = trf(
            "Compared with “{}”: {} added, {} removed, {} changed",
            &[
                &name,
                &diff.added.len(),
                &diff.removed.len(),
                &(diff.changed.len() + diff.renamed.len()),
            ],
        );
        self.show_results(&title, rows);
        self.pv_open_checkpoint_compare(id);
        self.toast(&title);
    }

    pub(super) fn restore_checkpoint(&mut self, id: u64) {
        self.flush();
        // the state before restoring is kept too, so a restore can be undone
        self.create_checkpoint("Before: Restore Checkpoint");
        let name = self
            .checkpoints
            .list()
            .into_iter()
            .find(|c| c.id == id)
            .map(|c| c.name)
            .unwrap_or_default();
        let res = self
            .checkpoints
            .restore_into(id, &mut self.book.borrow_mut().archive);
        match res {
            Ok(()) => {
                self.after_archive_changed();
                self.toast(&trf("Restored “{}”", &[&name]));
            }
            Err(e) => self.toast(&trf("Could not restore: {}", &[&e])),
        }
    }

    pub(super) fn show_report(&mut self) {
        self.flush();
        let rows = editor_tools::report_rows(&self.book.borrow());
        let dialog = adw::Dialog::new();
        dialog.set_title(&tr("Book Report"));
        dialog.set_content_width(480);
        dialog.set_content_height(460);
        let page = adw::PreferencesPage::new();
        let group = adw::PreferencesGroup::new();
        for (k, v) in rows {
            let row = adw::ActionRow::new();
            row.set_title(&glib::markup_escape_text(&k));
            let l = gtk::Label::new(Some(&v));
            l.set_wrap(true);
            l.set_xalign(1.0);
            l.set_max_width_chars(34);
            l.add_css_class("dim-label");
            row.add_suffix(&l);
            group.add(&row);
        }
        page.add(&group);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&page));
        dialog.set_child(Some(&view));
        dialog.present(Some(&self.w.win));
    }

    pub(super) fn show_results(
        &self,
        title: &str,
        rows: Vec<(String, String, String, &'static str)>,
    ) {
        self.w.problems.root.set_reveal_child(false);
        self.w.results_title.set_label(title);
        while let Some(c) = self.w.results.first_child() {
            self.w.results.remove(&c);
        }
        for (icon, t, sub, class) in rows {
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&t))
                .subtitle(glib::markup_escape_text(&sub))
                .activatable(true)
                .build();
            let img = gtk::Image::from_icon_name(&icon);
            if !class.is_empty() {
                img.add_css_class(class);
            }
            row.add_prefix(&img);
            row.set_subtitle_lines(1);
            row.set_title_lines(1);
            self.w.results.append(&row);
        }
        self.w.results_rev.set_reveal_child(true);
    }

    pub(super) fn run_spelling(&mut self) {
        self.flush();
        let (issues, note) =
            editor_tools::spelling_issues(&self.book.borrow(), &editor_tools::dictionary_dirs());
        if issues.is_empty() {
            self.toast(&note.unwrap_or_else(|| "No spelling mistakes found".into()));
            return;
        }
        let rows: Vec<_> = issues
            .iter()
            .map(|i| {
                (
                    "tools-check-spelling-symbolic".to_string(),
                    i.message.clone(),
                    match i.line {
                        Some(l) => format!("{}:{l}", i.file),
                        None => i.file.clone(),
                    },
                    "issue-warning",
                )
            })
            .collect();
        self.results = issues
            .iter()
            .map(|i| ResultRow {
                file: i.file.clone(),
                line: i.line,
                range: None,
            })
            .collect();
        self.result_kind = ResultKind::Check;
        let title = match note {
            Some(n) => format!("{} possible mistakes. {n}", issues.len()),
            None => format!("{} possible spelling mistakes", issues.len()),
        };
        self.show_results(&title, rows);
    }

    /// F7: the Problems panel (see `check_panel.rs`).
    pub(super) fn run_check(&mut self) {
        self.problems_run();
    }
}
