//! The library actions of track T3 that need the window: search and replace, marks, quickview,
//! add empty book and add by ISBN. A child of `library`, so it sees the window's own state.

use super::*;
use crate::i18n::{ntr, tr, trf};
use std::path::Path;
use std::sync::Arc;

/// How many widgets below `w` have a name that starts with `prefix`.
fn count_prefixed(w: &gtk::Widget, prefix: &str) -> usize {
    let mut n = usize::from(w.widget_name().starts_with(prefix));
    let mut c = w.first_child();
    while let Some(ch) = c {
        n += count_prefixed(&ch, prefix);
        c = ch.next_sibling();
    }
    n
}

/// A text box with the note about an author or a series and a button that saves it.
fn note_editor(
    store: &Arc<bookx_store::SqliteStore>,
    kind: &'static str,
    name: &str,
) -> adw::ExpanderRow {
    let have = store.item_note(kind, name);
    let exp = adw::ExpanderRow::builder()
        .title(glib::markup_escape_text(name))
        .subtitle(if kind == "author" {
            tr("Note about the author")
        } else {
            tr("Note about the series")
        })
        .build();
    exp.set_widget_name(&format!("note-{kind}"));
    exp.set_expanded(have.is_some());
    let view = gtk::TextView::new();
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_top_margin(8);
    view.set_bottom_margin(8);
    view.set_left_margin(10);
    view.set_right_margin(10);
    view.set_size_request(-1, 90);
    view.buffer().set_text(have.as_deref().unwrap_or(""));
    let frame = gtk::ScrolledWindow::new();
    frame.set_min_content_height(90);
    frame.set_child(Some(&view));
    let save = gtk::Button::with_label(&tr("Save Note"));
    save.set_halign(gtk::Align::End);
    save.set_widget_name(&format!("note-save-{kind}"));
    let status = gtk::Label::new(None);
    status.add_css_class("dim-label");
    let boxed = gtk::Box::new(gtk::Orientation::Vertical, 6);
    boxed.set_margin_top(6);
    boxed.set_margin_bottom(6);
    boxed.set_margin_start(12);
    boxed.set_margin_end(12);
    boxed.append(&frame);
    boxed.append(&status);
    boxed.append(&save);
    exp.add_row(&boxed);
    {
        let (store, name, view, status) = (
            store.clone(),
            name.to_string(),
            view.clone(),
            status.clone(),
        );
        save.connect_clicked(move |_| {
            let b = view.buffer();
            let (s, e) = b.bounds();
            let text = b.text(&s, &e, false).to_string();
            match store.set_item_note(kind, &name, &text) {
                Ok(()) => status.set_label(&tr("Note saved")),
                Err(e) => status.set_label(&format!("{e:#}")),
            }
        });
    }
    exp
}

impl LibraryView {
    /// Handles the T3 actions; true when `act` was one of them.
    pub(super) fn t3_act(&mut self, act: &Act) -> bool {
        match act {
            Act::AddEmpty | Act::AddIsbn => {
                let Some(win) = window_of(&self.root) else {
                    return true;
                };
                let tx = self.sender.input_sender().clone();
                let isbn = matches!(act, Act::AddIsbn);
                let done: super::super::import_isbn::Done = Rc::new(move |id, msg| {
                    tx.emit(Input::Refresh);
                    tx.emit(Input::Toast(msg));
                    if isbn {
                        tx.emit(Input::BulkFetch(vec![id]));
                    }
                });
                if isbn {
                    super::super::import_isbn::add_by_isbn(&win, &self.shared, done);
                } else {
                    super::super::import_isbn::add_empty(&win, &self.shared, done);
                }
                true
            }
            Act::ClearMarks => {
                let n = self.set_marks(None, false);
                self.sender
                    .input(Input::Toast(ntr("Cleared {} mark", "Cleared {} marks", n)));
                true
            }
            Act::Mark => {
                let ids = self.selected_books();
                if ids.is_empty() {
                    return true;
                }
                let all = {
                    let lib = self.shared.lib.borrow();
                    ids.iter().all(|i| lib.get(*i).is_some_and(|b| b.marked))
                };
                let n = self.set_marks(Some(&ids), !all);
                let msg = if all {
                    ntr("Unmarked {} book", "Unmarked {} books", n)
                } else {
                    trf(
                        "{} (search marked:true to list them)",
                        &[&ntr("Marked {} book", "Marked {} books", n)],
                    )
                };
                self.sender.input(Input::Toast(msg));
                true
            }
            Act::Replace => {
                let (Some(win), ids) = (window_of(&self.root), self.selected_books()) else {
                    return true;
                };
                if ids.is_empty() {
                    return true;
                }
                let tx = self.sender.input_sender().clone();
                super::super::replace_fields::show(
                    &win,
                    &self.shared,
                    ids,
                    Rc::new(move |m| {
                        tx.emit(Input::Refresh);
                        tx.emit(Input::Toast(m));
                    }),
                );
                true
            }
            Act::Quickview => {
                self.quickview();
                true
            }
            Act::CoverFromPdf | Act::TrimCover => {
                let Some(id) = self.selected.first().copied() else {
                    return true;
                };
                let msg = match self.change_cover(id, matches!(act, Act::CoverFromPdf)) {
                    Ok(m) => m,
                    Err(e) => trf("Could not change the cover: {}", &[&format!("{e:#}")]),
                };
                self.sender.input(Input::Toast(msg));
                self.sender.input(Input::Refresh);
                true
            }
            _ => false,
        }
    }

    /// Replaces the cover of a book with the first page of its PDF (`from_pdf`) or with itself cut to
    /// 2:3. The book file, the catalogue and the undo log are updated like an edit of the metadata.
    pub(super) fn change_cover(&mut self, id: u64, from_pdf: bool) -> anyhow::Result<String> {
        let (path, meta, cover, pdf) = {
            let lib = self.shared.lib.borrow();
            let b = lib.get(id).ok_or_else(|| anyhow::anyhow!("no such book"))?;
            let pdf = b
                .all_formats()
                .into_iter()
                .find(|f| f.format.eq_ignore_ascii_case("pdf"))
                .map(|f| lib.format_path(&f));
            (lib.book_path(b), b.meta.clone(), lib.cover_path(b), pdf)
        };
        let data = if from_pdf {
            let pdf = pdf.ok_or_else(|| anyhow::anyhow!("{}", tr("This book has no PDF")))?;
            crate::library_cover::pdf_cover(std::fs::read(pdf)?)?
        } else {
            let cover = cover.ok_or_else(|| anyhow::anyhow!("{}", tr("This book has no cover")))?;
            crate::library_cover::trim_cover(&std::fs::read(cover)?)?
        };
        let ext = crate::library_cover::image_ext(&data);
        if let Some(cp) = self
            .shared
            .lib
            .borrow()
            .get(id)
            .and_then(|b| self.shared.lib.borrow().cover_path(b))
        {
            covers::forget(&cp);
        }
        let mut book = crate::core::book::OpenBook::open(&path)?;
        book.set_cover(data, ext)?;
        book.archive.save(&path)?;
        self.shared.lib.borrow_mut().update_metadata(id, meta)?;
        covers::clear();
        Ok(if from_pdf {
            tr("The cover is now the first page of the PDF")
        } else {
            tr("The cover was cut to 2:3")
        })
    }

    /// Marks or unmarks `ids` (all books when `None`); returns how many books changed.
    fn set_marks(&mut self, ids: Option<&[u64]>, on: bool) -> usize {
        let mut n = 0;
        {
            let mut lib = self.shared.lib.borrow_mut();
            for b in lib.books.iter_mut() {
                if ids.is_none_or(|v| v.contains(&b.id)) && b.marked != on {
                    b.marked = on;
                    n += 1;
                }
            }
        }
        self.requery();
        n
    }

    /// A window with the other books that share the selected book's author, series or tags.
    fn quickview(&mut self) {
        let (Some(win), Some(id)) = (window_of(&self.root), self.selected.first().copied()) else {
            return;
        };
        let groups = crate::library_related::related(&self.shared.lib.borrow().books, id, 6);
        let dialog = super::super::dialog_toast::new_dialog();
        dialog.set_title(&tr("Related Books"));
        dialog.set_content_width(480);
        dialog.set_content_height(560);
        let page = adw::PreferencesPage::new();
        if groups.is_empty() {
            let s = adw::StatusPage::builder()
                .icon_name("view-list-symbolic")
                .title(tr("No Related Books"))
                .description(tr(
                    "No other book shares this book's author, series or tags",
                ))
                .build();
            s.set_widget_name("quickview-empty");
            page.add(&{
                let g = adw::PreferencesGroup::new();
                g.add(&s);
                g
            });
        }
        let (search, bar) = (self.w.search.clone(), self.w.search_bar.clone());
        for g in groups {
            let exp = adw::ExpanderRow::builder()
                .title(glib::markup_escape_text(&g.value))
                .subtitle(match g.kind {
                    "author" => tr("Same author"),
                    "series" => tr("Same series"),
                    _ => tr("Same tag"),
                })
                .build();
            exp.set_widget_name(&format!("quickview-{}", g.kind));
            let show =
                gtk::Button::with_label(&ntr("Show {} Book", "Show {} Books", g.books.len() + 1));
            show.set_valign(gtk::Align::Center);
            {
                let (search, bar, q, d) =
                    (search.clone(), bar.clone(), g.query.clone(), dialog.clone());
                show.connect_clicked(move |_| {
                    bar.set_search_mode(true);
                    search.set_text(&q);
                    d.close();
                });
            }
            exp.add_suffix(&show);
            for (_, title) in g.books.iter().take(30) {
                exp.add_row(
                    &adw::ActionRow::builder()
                        .title(glib::markup_escape_text(title))
                        .build(),
                );
            }
            let grp = adw::PreferencesGroup::new();
            grp.add(&exp);
            page.add(&grp);
        }
        // notes about the book's authors and series
        let items: Vec<(&'static str, String)> = {
            let lib = self.shared.lib.borrow();
            lib.get(id)
                .map(|b| {
                    let mut v: Vec<(&'static str, String)> = b
                        .meta
                        .authors
                        .iter()
                        .map(|a| ("author", a.clone()))
                        .collect();
                    if !b.meta.series.is_empty() {
                        v.push(("series", b.meta.series.clone()));
                    }
                    v
                })
                .unwrap_or_default()
        };
        if !items.is_empty() {
            let notes = adw::PreferencesGroup::builder().title(tr("Notes")).build();
            let store = self.shared.guard.borrow().store.clone();
            for (kind, name) in items {
                notes.add(&note_editor(&store, kind, &name));
            }
            page.add(&notes);
        }
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        toolbar.set_content(Some(&page));
        dialog.set_child(Some(&toolbar));
        dialog.present(Some(&win));
    }

    /// `library:` commands of track T3; true when `c` was one of them.
    pub(super) fn dev_t3(&mut self, c: &str, sender: &ComponentSender<Self>) -> bool {
        match c {
            other if other.starts_with("indexstatus") => {
                let arg = other.strip_prefix("indexstatus=").unwrap_or("");
                eprintln!("{}", super::super::index_status::dev(&self.shared, arg));
            }
            other if other.starts_with("inboxwatch=") => {
                eprintln!(
                    "{}",
                    super::super::folder_watch::dev(&self.shared, &other[11..])
                );
            }
            // `tagtree=on|off` and `tagtree=rows` print the rows of the tag tree
            other if other.starts_with("tagtree=") => {
                let arg = &other[8..];
                if arg == "on" || arg == "off" {
                    {
                        let mut c = self.shared.cfg.borrow_mut();
                        c.tag_tree = arg == "on";
                        c.save();
                    }
                    self.rebuild_tags();
                }
                let rows: Vec<String> =
                    crate::library_tagtree::rows(&self.shared.lib.borrow().books)
                        .iter()
                        .map(|r| format!("{}{}({})", "-".repeat(r.indent), r.label, r.count))
                        .collect();
                let shown = count_prefixed(self.w.tags.upcast_ref(), "tagtree-");
                eprintln!(
                    "tagtree: on={} rows=[{}] widgets={shown}",
                    self.shared.cfg.borrow().tag_tree,
                    rows.join(" ")
                );
            }
            // `note=set|kind|name|text`, `note=get|kind|name`
            other if other.starts_with("note=") => {
                let p: Vec<&str> = other[5..].splitn(4, '|').collect();
                let store = self.shared.guard.borrow().store.clone();
                match p.as_slice() {
                    ["set", k, n, t] => match store.set_item_note(k, n, t) {
                        Ok(()) => eprintln!("note: saved"),
                        Err(e) => eprintln!("note: error {e:#}"),
                    },
                    ["get", k, n] => eprintln!("note: {:?}", store.item_note(k, n)),
                    _ => eprintln!("note: usage set|kind|name|text or get|kind|name"),
                }
            }
            "customcols" => {
                eprintln!("{}", super::super::custom_columns::dev_dump(&self.shared));
            }
            "replace" | "mark" | "clearmarks" | "quickview" => {
                let act = match c {
                    "replace" => Act::Replace,
                    "mark" => Act::Mark,
                    "clearmarks" => Act::ClearMarks,
                    _ => Act::Quickview,
                };
                self.t3_act(&act);
            }
            // `customcol=add|Name|type|a,b` / `customcol=set|ID|label|value` / `customcol=cell|ID|label`
            other if other.starts_with("customcol=") => {
                let msg = self.dev_customcol(&other[10..]);
                eprintln!("customcol: {msg}");
                sender.input(Input::Refresh);
            }
            // `customtable=label:on|off` ticks a custom column of the table
            other if other.starts_with("customtable=") => {
                let (label, on) = other[12..].rsplit_once(':').unwrap_or((&other[12..], "on"));
                {
                    let defs = self.shared.lib.borrow().columns().to_vec();
                    let mut cfg = self.shared.cfg.borrow_mut();
                    cfg.table_custom =
                        crate::library_custom::toggle(&cfg.table_custom, &defs, label, on == "on");
                    cfg.save();
                }
                super::super::custom_columns::sync_table(&self.table.view, &self.shared);
                eprintln!("{}", super::super::custom_columns::dev_dump(&self.shared));
            }
            // `customsort=label:asc|desc` is a click on the header of a custom column
            other if other.starts_with("customsort=") => {
                let (label, dir) = other[11..]
                    .rsplit_once(':')
                    .unwrap_or((&other[11..], "asc"));
                let col = super::super::custom_columns::column_of(label);
                if let Some(col) = col {
                    self.table.view.sort_by_column(
                        Some(&col),
                        if dir == "desc" {
                            gtk::SortType::Descending
                        } else {
                            gtk::SortType::Ascending
                        },
                    );
                    let first: Vec<String> = (0..self.table.len().min(4))
                        .filter_map(|i| self.table.get(i).map(|r| r.borrow().title.clone()))
                        .collect();
                    eprintln!("customsort: first=[{}]", first.join("|"));
                }
            }
            other if other.starts_with("record=") => {
                match super::super::import_isbn::dev_add(&self.shared, &other[7..]) {
                    Ok(id) => eprintln!("record: added id={id}"),
                    Err(e) => eprintln!("record: error {e}"),
                }
                sender.input(Input::Refresh);
            }
            other if other.starts_with("replace=") => {
                eprintln!(
                    "{}",
                    super::super::replace_fields::dev_apply(&self.shared, &other[8..])
                );
                sender.input(Input::Refresh);
            }
            other if other.starts_with("marked=") => {
                let ids: Vec<u64> = other[7..]
                    .split(',')
                    .filter_map(|s| s.parse().ok())
                    .collect();
                let n = self.set_marks(Some(&ids), true);
                eprintln!("marked: {n}");
            }
            _ => return false,
        }
        true
    }

    fn dev_customcol(&mut self, spec: &str) -> String {
        use crate::core::custom::{label_from_name, ColumnDef, ColumnKind};
        let p: Vec<&str> = spec.split('|').collect();
        let mut lib = self.shared.lib.borrow_mut();
        match p.as_slice() {
            ["add", name, kind, rest @ ..] => {
                let Some(kind) = ColumnKind::parse(kind) else {
                    return format!("unknown type {kind}");
                };
                let mut defs = lib.columns().to_vec();
                let arg = rest.first().copied().unwrap_or("");
                defs.push(ColumnDef {
                    label: label_from_name(name),
                    name: name.to_string(),
                    kind,
                    values: if kind == ColumnKind::Enumeration {
                        arg.split('/').map(|s| s.trim().to_string()).collect()
                    } else {
                        Vec::new()
                    },
                    template: if kind == ColumnKind::Composite {
                        arg.to_string()
                    } else {
                        String::new()
                    },
                });
                match lib.set_columns(defs) {
                    Ok(()) => format!("added {}", label_from_name(name)),
                    Err(e) => format!("error {e:#}"),
                }
            }
            ["set", id, label, value] => match id.parse::<u64>() {
                Ok(id) => match lib.set_custom(id, label, value) {
                    Ok(()) => format!(
                        "set {label}={}",
                        lib.get(id).map_or("", |b| b.custom_value(label))
                    ),
                    Err(e) => format!("error {e:#}"),
                },
                Err(_) => "bad id".into(),
            },
            ["undo"] => match lib.undo_last() {
                Ok(l) => format!("undone {l:?}"),
                Err(e) => format!("error {e:#}"),
            },
            ["remove", label] => {
                let defs: Vec<ColumnDef> = lib
                    .columns()
                    .iter()
                    .filter(|c| c.label != *label)
                    .cloned()
                    .collect();
                match lib.set_columns(defs) {
                    Ok(()) => format!("removed {label}"),
                    Err(e) => format!("error {e:#}"),
                }
            }
            _ => "usage: add|Name|type|arg, set|ID|label|value, undo, remove|label".into(),
        }
    }

    /// Takes the ZIP archives that hold several books out of `paths` and asks which of their books
    /// to import (the chosen ones come back through `Input::ImportPaths`). Other paths stay.
    pub(super) fn split_bundles(&mut self, paths: Vec<PathBuf>) -> (Vec<PathBuf>, bool) {
        let mut keep = Vec::new();
        let mut asked = false;
        for p in paths {
            let is_zip = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"));
            let list = if is_zip {
                crate::core::archive_books::list_books(&p, &|n| {
                    crate::import::is_supported(Path::new(n))
                })
                .ok()
                .flatten()
            } else {
                None
            };
            match list {
                Some(l) => {
                    asked = true;
                    self.pick_bundle(p, l)
                }
                None => keep.push(p),
            }
        }
        (keep, asked)
    }

    fn pick_bundle(&self, zip: PathBuf, list: Vec<crate::core::archive_books::Entry>) {
        let Some(win) = window_of(&self.root) else {
            return;
        };
        let dialog = super::super::dialog_toast::new_dialog();
        dialog.set_title(&tr("Books in This Archive"));
        dialog.set_content_width(520);
        dialog.set_content_height(560);
        let page = adw::PreferencesPage::new();
        let g = adw::PreferencesGroup::builder()
            .description(trf(
                "{} holds several books. Choose the ones to add.",
                &[&zip
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()],
            ))
            .build();
        let rows: Vec<(crate::core::archive_books::Entry, adw::SwitchRow)> = list
            .into_iter()
            .map(|e| {
                let r = adw::SwitchRow::builder()
                    .title(glib::markup_escape_text(&e.name))
                    .subtitle(human_size(e.size))
                    .active(true)
                    .build();
                g.add(&r);
                (e, r)
            })
            .collect();
        page.add(&g);
        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);
        let cancel = gtk::Button::with_label(&tr("Cancel"));
        let add = gtk::Button::with_label(&tr("Add"));
        add.add_css_class("suggested-action");
        add.set_widget_name("bundle-add");
        header.pack_start(&cancel);
        header.pack_end(&add);
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&page));
        dialog.set_child(Some(&toolbar));
        {
            let d = dialog.clone();
            cancel.connect_clicked(move |_| {
                d.close();
            });
        }
        let tx = self.sender.input_sender().clone();
        {
            let d = dialog.clone();
            add.connect_clicked(move |_| {
                let picked: Vec<usize> = rows
                    .iter()
                    .filter(|(_, r)| r.is_active())
                    .map(|(e, _)| e.index)
                    .collect();
                d.close();
                if picked.is_empty() {
                    return;
                }
                let (zip, tx) = (zip.clone(), tx.clone());
                // extracted on a worker thread (a book can be big); the main loop picks up the result
                let (done_tx, done_rx) = std::sync::mpsc::channel::<Vec<PathBuf>>();
                std::thread::spawn(move || {
                    let dir =
                        std::env::temp_dir().join(format!("bookx-bundle-{}", uuid::Uuid::new_v4()));
                    let files: Vec<PathBuf> = picked
                        .into_iter()
                        .filter_map(|i| crate::core::archive_books::extract(&zip, i, &dir).ok())
                        .collect();
                    let _ = done_tx.send(files);
                });
                glib::timeout_add_local(
                    std::time::Duration::from_millis(100),
                    move || match done_rx.try_recv() {
                        Ok(files) => {
                            tx.emit(Input::ImportPaths(files));
                            glib::ControlFlow::Break
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(_) => glib::ControlFlow::Break,
                    },
                );
            });
        }
        dialog.present(Some(&win));
    }
}
