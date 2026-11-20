//! The file browser: categories in reading order, hover details, icons for special files, drag and drop in the
//! reading order, roles, replace, extension, open with, image grid, user templates (CE-020..CE-046).

use crate::i18n::N_;
use crate::i18n::{tr, trf};
use bookx_editing::book_tools::{self, FileNote, PageRole};

use super::tools_ui::*;
use super::*;
use crate::editor_state;

const GROUPS: [&str; 5] = ["Text", "Styles", "Images", "Fonts", "Package"];

/// An external editor may write the file in several steps, and not every file monitor backend sends
/// the same final `ChangesDoneHint`. Any write/create event is a reason to look at the file again;
/// the check is debounced below so a partial write cannot open the update dialog.
fn external_file_changed(ev: gio::FileMonitorEvent) -> bool {
    matches!(
        ev,
        gio::FileMonitorEvent::Changed
            | gio::FileMonitorEvent::Created
            | gio::FileMonitorEvent::ChangesDoneHint
    )
}

fn install_css() {
    thread_local!(static DONE: Cell<bool> = const { Cell::new(false) });
    if DONE.with(|d| d.replace(true)) {
        return;
    }
    let css = gtk::CssProvider::new();
    css.load_from_string(
        ".drop-before { box-shadow: inset 0 2px 0 0 @accent_bg_color; } \
         .drop-after { box-shadow: inset 0 -2px 0 0 @accent_bg_color; }",
    );
    if let Some(d) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &d,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// The "files" part of the Files menu (added to the existing one).
pub fn extend_files_menu(menu: &gio::Menu) {
    let sel = gio::Menu::new();
    for (label, id) in [
        (N_("All Text Files"), "select:Text"),
        (N_("All Stylesheets"), "select:Styles"),
        (N_("All Images"), "select:Images"),
        (N_("All Fonts"), "select:Fonts"),
        (N_("Select None"), "select:none"),
    ] {
        sel.append_item(&menu_item(&tr(label), "ext.files", id));
    }
    let top = gio::Menu::new();
    top.append_submenu(Some(&tr("Select")), &sel);
    menu.append_section(None, &top);
    let roles = gio::Menu::new();
    for (label, id) in [
        (N_("Use as Cover Image"), "cover-image"),
        (N_("Use as Cover Page"), "cover-page"),
        (N_("Use as Title Page"), "title-page"),
        (N_("Use as Table of Contents"), "toc-page"),
        (N_("Toggle Not in Main Reading Order"), "nonlinear"),
    ] {
        roles.append_item(&menu_item(&tr(label), "ext.files", id));
    }
    menu.append_section(None, &roles);
    let more = gio::Menu::new();
    for (label, id) in [
        (N_("Change Extension"), "extension"),
        (N_("Move to Folder"), "move"),
        (N_("Replace with File"), "replace"),
        (N_("Open With"), "open-with"),
        (N_("Browse Images"), "grid"),
    ] {
        more.append_item(&menu_item(&tr(label), "ext.files", id));
    }
    menu.append_section(None, &more);
    let view = gio::Menu::new();
    view.append(Some(&tr("Show Full Paths")), Some("ext.full-paths"));
    menu.append_section(None, &view);
}

impl Editor {
    /// Rebuilds the file list: text in reading order, the other categories alphabetical (CE-020).
    pub(super) fn rebuild_files_ext(&mut self) {
        install_css();
        self.w.files.remove_all();
        self.file_rows.clear();
        let full_paths = self.ext.prefs.borrow().full_paths;
        let groups = {
            let b = self.book.borrow();
            let files: Vec<String> = b
                .file_list()
                .into_iter()
                .filter(|f| f != "mimetype")
                .collect();
            editor_tools::browser_order(&b, &files, group_of)
        };
        let spine = self.book.borrow().spine_paths();
        let semantics = bookx_editing::semantics::read_semantics(&self.book.borrow());
        for (g, files) in groups {
            for f in files {
                let row = self.file_row(&f, g, &spine, &semantics, full_paths);
                self.w.files.append(&row.0);
                self.file_rows.insert(f, row);
            }
        }
        if let Some(c) = self.current.clone() {
            if let Some((row, _)) = self.file_rows.get(&c) {
                self.w.files.select_row(Some(row));
            }
        }
        self.prune_tabs();
    }

    fn file_row(
        &self,
        f: &str,
        group: &'static str,
        spine: &[String],
        semantics: &[bookx_editing::semantics::SemanticEntry],
        full_paths: bool,
    ) -> (gtk::ListBoxRow, gtk::Image) {
        let row = gtk::ListBoxRow::new();
        row.set_widget_name(group);
        let bx = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bx.set_margin_start(8);
        bx.set_margin_end(8);
        bx.set_margin_top(5);
        bx.set_margin_bottom(5);
        bx.append(&gtk::Image::from_icon_name(file_icon(f)));
        let name = gtk::Label::new(Some(if full_paths { f } else { file_name(f) }));
        name.set_xalign(0.0);
        name.set_hexpand(true);
        name.set_ellipsize(pango::EllipsizeMode::Middle);
        bx.append(&name);
        let notes = book_tools::file_notes_with(&self.book.borrow(), f, semantics);
        for n in notes.iter().take(3) {
            let img = gtk::Image::from_icon_name(n.icon());
            img.set_tooltip_text(Some(n.tooltip()));
            if matches!(
                n,
                FileNote::NotInManifest | FileNote::NotInSpine | FileNote::TwiceInSpine
            ) {
                img.add_css_class("warning");
            } else {
                img.add_css_class("dim-label");
            }
            bx.append(&img);
        }
        let at = spine.iter().position(|s| s == f);
        if let Some(i) = at {
            let n = gtk::Label::new(Some(&format!("{}", i + 1)));
            n.add_css_class("dim-label");
            n.add_css_class("caption");
            n.add_css_class("numeric");
            n.set_tooltip_text(Some(&tr("Position in the reading order")));
            bx.append(&n);
        }
        let dot = gtk::Image::from_icon_name("media-record-symbolic");
        dot.add_css_class("accent");
        dot.set_visible(self.modified.contains(f));
        bx.append(&dot);
        row.set_child(Some(&bx));
        row.set_tooltip_text(Some(&editor_tools::file_tooltip_with(
            &self.book.borrow(),
            f,
            &notes,
        )));
        unsafe { row.set_data("path", f.to_string()) };
        if at.is_some() {
            self.make_draggable(&row, f);
        }
        (row, dot)
    }

    /// Drag a page row onto another page row to reorder the reading order (CE-029).
    fn make_draggable(&self, row: &gtk::ListBoxRow, path: &str) {
        let src = gtk::DragSource::new();
        src.set_actions(gdk::DragAction::MOVE);
        let p = path.to_string();
        src.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&p.to_value())));
        row.add_controller(src);
        let target = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
        let clear = |w: &gtk::Widget| {
            w.remove_css_class("drop-before");
            w.remove_css_class("drop-after");
        };
        {
            let row = row.clone();
            let merge_ok = bookx_editing::markup::is_markup_path(path);
            target.connect_motion(move |_, _, y| {
                let after = y > row.height() as f64 / 2.0;
                row.remove_css_class("drop-before");
                row.remove_css_class("drop-after");
                row.remove_css_class("toc-drop-into");
                if merge_ok && super::struct_hub::is_merge_zone(y, row.height()) {
                    super::toc_dnd::install_css();
                    row.add_css_class("toc-drop-into");
                } else {
                    row.add_css_class(if after { "drop-after" } else { "drop-before" });
                }
                gdk::DragAction::MOVE
            });
        }
        {
            let row = row.clone();
            target.connect_leave(move |_| {
                clear(row.upcast_ref());
                row.remove_css_class("toc-drop-into");
            });
        }
        {
            let row = row.clone();
            let onto = path.to_string();
            let tx = self.sender.input_sender().clone();
            let merge_ok = bookx_editing::markup::is_markup_path(path);
            target.connect_drop(move |_, v, _, y| {
                row.remove_css_class("drop-before");
                row.remove_css_class("drop-after");
                row.remove_css_class("toc-drop-into");
                let Ok(from) = v.get::<String>() else {
                    return false;
                };
                if merge_ok && super::struct_hub::is_merge_zone(y, row.height()) {
                    tx.emit(Input::Ext(Cmd::Struct(
                        super::struct_hub::StructCmd::DropMerge(from, onto.clone()),
                    )));
                    return true;
                }
                tx.emit(Input::Ext(Cmd::DropRow(
                    from,
                    onto.clone(),
                    y > row.height() as f64 / 2.0,
                )));
                true
            });
        }
        row.add_controller(target);
    }

    /// The drop handler (also called by the dev hook `x:drop`).
    pub(super) fn drop_row(&mut self, from: &str, onto: &str, after: bool) {
        let spine = self.book.borrow().spine_paths();
        let (Some(f), Some(t)) = (
            spine.iter().position(|s| s == from),
            spine.iter().position(|s| s == onto),
        ) else {
            self.toast(&tr("Only files in the reading order can be reordered"));
            return;
        };
        let to = book_tools::drop_index(f, t, after);
        if to == f {
            return;
        }
        let from = from.to_string();
        self.run_op("Move File", move |b, _| {
            editor_tools::spine_drop(b, &from, to)
        });
        self.reselect(&[]);
    }

    fn reselect(&mut self, _paths: &[String]) {
        if let Some(c) = self.current.clone() {
            if let Some((row, _)) = self.file_rows.get(&c) {
                self.w.files.select_row(Some(row));
            }
        }
    }

    /// The file an action applies to: the one selected row, else the open file.
    fn target_file(&self) -> Option<String> {
        self.selected_path().or_else(|| self.current.clone())
    }

    pub(super) fn files_action(&mut self, id: &str) {
        if let Some(g) = id.strip_prefix("select:") {
            self.select_group(g);
            return;
        }
        if let Some(rest) = id.strip_prefix("external|") {
            if let Some((path, tmp)) = rest.split_once('|') {
                self.external_changed(path, std::path::Path::new(tmp));
            }
            return;
        }
        match id {
            "cover-image" => self.sender.input(Input::SetCover),
            "cover-page" => self.mark_role(PageRole::Cover),
            "title-page" => self.mark_role(PageRole::TitlePage),
            "toc-page" => self.mark_role(PageRole::Toc),
            "nonlinear" => {
                let Some(p) = self.target_file() else { return };
                self.run_op("Reading Order", move |b, _| {
                    editor_tools::toggle_nonlinear(b, &p)
                });
            }
            "extension" => self.prompt_extension(),
            "move" => self.open_tool("move-files"),
            "replace" => self.prompt_replace(),
            "open-with" => self.open_with(),
            "grid" => self.image_grid(),
            "full-paths" => {
                let now = {
                    let mut p = self.ext.prefs.borrow_mut();
                    p.full_paths = !p.full_paths;
                    p.save();
                    p.full_paths
                };
                self.rebuild_files();
                self.toast(
                    &(if now {
                        tr("Showing full file paths")
                    } else {
                        tr("Showing file names")
                    }),
                );
            }
            other => self.toast(&trf("Unknown file action {}", &[&other])),
        }
    }

    fn select_group(&self, g: &str) {
        if g == "none" {
            self.w.files.unselect_all();
            return;
        }
        if !GROUPS.contains(&g) {
            return;
        }
        self.w.files.unselect_all();
        for (row, _) in self.file_rows.values() {
            if row.widget_name() == g {
                self.w.files.select_row(Some(row));
            }
        }
    }

    fn mark_role(&mut self, role: PageRole) {
        let Some(p) = self
            .target_file()
            .filter(|p| bookx_editing::markup::is_markup_path(p))
        else {
            self.toast(&tr("Select a page first"));
            return;
        };
        self.run_op("Mark Page", move |b, _| {
            editor_tools::mark_role(b, &p, role)
        });
    }

    fn prompt_extension(&self) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            self.toast(&tr("Select the files first"));
            return;
        }
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some(&tr("New extension, for example xhtml")));
        crate::ui::audit::label(&entry, &tr("New file extension"));
        entry.set_activates_default(true);
        let tx = self.sender.input_sender().clone();
        let typed = entry.clone();
        self.ask(
            "Change Extension",
            &trf(
                "The {} selected file(s) get the new extension. Links to them are updated.",
                &[&paths.len()],
            ),
            &entry,
            "Change",
            move || {
                tx.emit(Input::Ext(Cmd::Dev(format!(
                    "ext={}|{}",
                    typed.text(),
                    paths.join("|")
                ))))
            },
        );
    }

    fn prompt_replace(&self) {
        let Some(old) = self.target_file() else {
            return;
        };
        let dlg = gtk::FileDialog::builder()
            .title(tr("Replace with File"))
            .build();
        let tx = self.sender.input_sender().clone();
        let win = self.w.win.clone();
        dlg.open(Some(&win.clone()), gio::Cancellable::NONE, move |res| {
            let Some(path) = res.ok().and_then(|f| f.path()) else {
                return;
            };
            let entry = gtk::Entry::new();
            entry.set_text(file_name(&old));
            crate::ui::audit::label(&entry, &tr("Name of the file in the book"));
            entry.set_activates_default(true);
            let d = adw::AlertDialog::new(
                Some(&tr("Replace File")),
                Some(&trf("The content of “{}” is replaced by “{}”. If you change the name, links to the file follow.", &[&file_name(&old), &(path.file_name().and_then(|n| n.to_str()).unwrap_or("the file"))])),
            );
            d.set_extra_child(Some(&entry));
            d.add_responses(&[("cancel", &tr("Cancel")), ("ok", &tr("Replace"))]);
            d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
            d.set_default_response(Some("ok"));
            d.set_close_response("cancel");
            let old = old.clone();
            let tx = tx.clone();
            d.connect_response(None, move |_, r| {
                if r == "ok" {
                    tx.emit(Input::Ext(Cmd::Dev(format!(
                        "replace={old}|{}|{}",
                        entry.text(),
                        path.display()
                    ))));
                }
            });
            d.present(Some(&win));
        });
    }

    /// Opens the file in another program (CE-042): it is written to a temporary folder and the book is updated
    /// when that program saves it.
    fn open_with(&mut self) {
        let Some(p) = self.target_file() else { return };
        self.flush();
        let Some(data) = self.book.borrow().archive.get(&p).map(|d| d.to_vec()) else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("bookx-open-with-{}", uuid::Uuid::new_v4()));
        let tmp = dir.join(file_name(&p));
        if std::fs::create_dir_all(&dir)
            .and_then(|_| std::fs::write(&tmp, data))
            .is_err()
        {
            self.toast(&tr("Could not write the temporary file"));
            return;
        }
        let file = gio::File::for_path(&tmp);
        if let Ok(mon) = file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
            let tx = self.sender.input_sender().clone();
            let (path, tmp2) = (p.clone(), tmp.clone());
            let debounce: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
            mon.connect_changed(move |_, _, _, ev| {
                if !external_file_changed(ev) {
                    return;
                }
                if let Some(id) = debounce.borrow_mut().take() {
                    id.remove();
                }
                let (tx, path, tmp) = (tx.clone(), path.clone(), tmp2.clone());
                let debounce = debounce.clone();
                *debounce.borrow_mut() = Some(glib::timeout_add_local_once(
                    std::time::Duration::from_millis(150),
                    move || {
                        tx.emit(Input::Ext(Cmd::Files(format!(
                            "external|{path}|{}",
                            tmp.display()
                        ))));
                    },
                ));
            });
            self.ext.monitors.borrow_mut().push(mon);
        }
        // Tests name a program (an absolute path, started without a shell) instead of the system chooser.
        if let Some(prog) = std::env::var_os("BOOKX_OPEN_WITH_PROGRAM")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute())
        {
            let started = std::process::Command::new(prog).arg(&tmp).spawn().is_ok();
            self.toast(&if started {
                trf(
                    "Opened {} in another program; the book updates when you save it there",
                    &[&file_name(&p)],
                )
            } else {
                tr("Could not start the program")
            });
            return;
        }
        let launcher = gtk::FileLauncher::new(Some(&file));
        launcher.set_always_ask(true);
        let tx = self.sender.input_sender().clone();
        launcher.launch(Some(&self.w.win), gio::Cancellable::NONE, move |res| {
            if let Err(e) = res {
                if !e.matches(gio::IOErrorEnum::Cancelled)
                    && !e.matches(gtk::DialogError::Dismissed)
                {
                    tx.emit(Input::Ext(Cmd::Dev(format!("toast={e}"))));
                }
            }
        });
        self.toast(&trf(
            "Opened {} in another program; the book updates when you save it there",
            &[&file_name(&p)],
        ));
    }

    /// The other program saved the temporary file (or the dev hook says so).
    pub(super) fn external_changed(&mut self, path: &str, tmp: &std::path::Path) {
        if self.ext.pending_external.borrow().is_some() {
            return;
        }
        let Ok(data) = std::fs::read(tmp) else { return };
        if self.book.borrow().archive.get(path) == Some(&data[..]) {
            return;
        }
        let (tx, path2) = (self.sender.input_sender().clone(), path.to_string());
        let tmp = tmp.to_path_buf();
        *self.ext.pending_external.borrow_mut() = Some((path.to_string(), tmp.clone()));
        alert(
            &self.w.win,
            "Update the File?",
            &trf(
                "“{}” was changed by another program. Replace it in the book with the new version?",
                &[&file_name(path)],
            ),
            "Update",
            false,
            move || {
                tx.emit(Input::Ext(Cmd::Dev(format!(
                    "replace={path2}||{}",
                    tmp.display()
                ))))
            },
        );
    }

    /// Replaces a file by the content of a file on disk (`new_name` empty = keep the name).
    pub(super) fn replace_from_disk(&mut self, path: &str, new_name: &str, src: &std::path::Path) {
        let Ok(data) = std::fs::read(src) else {
            self.toast(&tr("Could not read that file"));
            return;
        };
        let (p, n) = (path.to_string(), new_name.to_string());
        self.run_op("Replace File", move |b, _| {
            editor_tools::replace_with(b, &p, &n, data)
        });
        let target = if new_name.is_empty() {
            path.to_string()
        } else {
            bookx_editing::files::renamed_in_place(path, new_name)
        };
        if self.book.borrow().archive.contains(&target) {
            self.select_file(&target);
        }
    }

    pub(super) fn new_file_from_user(&mut self, name: &str, idx: usize, link: bool) {
        let templates = editor_state::user_templates();
        let Some(t) = templates.get(idx) else {
            self.toast(&tr("That template is gone"));
            return;
        };
        let Some(file) = t.file_name(name) else {
            self.toast(&tr("Enter a file name without a folder"));
            return;
        };
        let body = t.render(name.trim());
        let after = self.current.clone();
        let created = std::cell::RefCell::new(None);
        self.run_op("New File", |b, _| {
            let at = bookx_editing::files::add_file(b, &file, body.into_bytes(), after.as_deref())?;
            let mut details = vec![format!("{at} created")];
            if link && bookx_editing::markup::is_markup_path(&at) {
                let css = bookx_editing::markup::css_files(&b.archive);
                if !css.is_empty() {
                    bookx_editing::files::link_stylesheets(
                        b,
                        std::slice::from_ref(&at),
                        &css,
                        false,
                    )?;
                    details.push("stylesheets linked".to_string());
                }
            }
            *created.borrow_mut() = Some(at.clone());
            Ok(editor_tools::Outcome {
                changed: true,
                summary: trf("Created {}", &[&at]),
                details,
            })
        });
        if let Some(p) = created.into_inner() {
            self.select_file(&p);
        }
    }

    // ---- image grid (CE-046) ---------------------------------------------

    fn image_grid(&mut self) {
        self.flush();
        let (images, used) = {
            let b = self.book.borrow();
            let imgs: Vec<String> = b
                .file_list()
                .into_iter()
                .filter(|f| is_image_path(f))
                .collect();
            let unused: HashSet<String> = book_tools::unused_media(&b)
                .into_iter()
                .map(|u| u.path)
                .collect();
            (imgs, unused)
        };
        if images.is_empty() {
            self.toast(&tr("The book has no images"));
            return;
        }
        let flow = gtk::FlowBox::new();
        flow.set_selection_mode(gtk::SelectionMode::Multiple);
        flow.set_max_children_per_line(6);
        flow.set_min_children_per_line(2);
        flow.set_homogeneous(true);
        flow.set_column_spacing(8);
        flow.set_row_spacing(8);
        flow.set_margin_all(12);
        crate::ui::audit::label(&flow, &tr("Images of the book"));
        let mut cells: Vec<(String, gtk::FlowBoxChild)> = Vec::new();
        for p in &images {
            let size = self.book.borrow().archive.size_of(p);
            let data = self
                .book
                .borrow()
                .archive
                .get(p)
                .map(|d| d.to_vec())
                .unwrap_or_default();
            let pic = gtk::Picture::new();
            pic.set_size_request(120, 120);
            pic.set_content_fit(gtk::ContentFit::ScaleDown);
            pic.set_can_shrink(true);
            if let Some(t) = crate::ui::metadata::texture_from_bytes(&data) {
                pic.set_paintable(Some(&t));
            }
            let name = gtk::Label::new(Some(file_name(p)));
            name.set_ellipsize(pango::EllipsizeMode::Middle);
            name.set_max_width_chars(16);
            let info = gtk::Label::new(Some(&format!(
                "{}{}",
                human_size(size as u64),
                if used.contains(p) { " · unused" } else { "" }
            )));
            info.add_css_class("caption");
            info.add_css_class(if used.contains(p) {
                "warning"
            } else {
                "dim-label"
            });
            let cell = gtk::Box::new(gtk::Orientation::Vertical, 4);
            cell.append(&pic);
            cell.append(&name);
            cell.append(&info);
            cell.set_tooltip_text(Some(&editor_tools::file_tooltip(&self.book.borrow(), p)));
            let child = gtk::FlowBoxChild::new();
            child.set_child(Some(&cell));
            flow.append(&child);
            cells.push((p.clone(), child));
        }
        let dialog = adw::Dialog::new();
        dialog.set_title(&tr("Images"));
        let (w, h) = fit_dialog(&self.w.win, 760, 560);
        dialog.set_content_width(w);
        dialog.set_content_height(h);
        let open = gtk::Button::with_label(&tr("Open"));
        let delete = gtk::Button::with_label(&tr("Delete Selected"));
        delete.add_css_class("destructive-action");
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bar.set_margin_all(10);
        bar.set_halign(gtk::Align::End);
        bar.append(&delete);
        bar.append(&open);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_child(Some(&flow));
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&scroll));
        view.add_bottom_bar(&bar);
        dialog.set_child(Some(&view));
        let chosen = {
            let flow = flow.clone();
            let cells = cells.clone();
            Rc::new(move || -> Vec<String> {
                cells
                    .iter()
                    .filter(|(_, c)| flow.selected_children().contains(c))
                    .map(|(p, _)| p.clone())
                    .collect()
            })
        };
        {
            let (tx, d, chosen) = (
                self.sender.input_sender().clone(),
                dialog.clone(),
                chosen.clone(),
            );
            let open_it = move || {
                if let Some(p) = chosen().into_iter().next() {
                    tx.emit(Input::SelectFile(p));
                    d.close();
                }
            };
            let o = open_it.clone();
            open.connect_clicked(move |_| o());
            flow.connect_child_activated(move |_, _| open_it());
        }
        {
            let (tx, d, chosen) = (
                self.sender.input_sender().clone(),
                dialog.clone(),
                chosen.clone(),
            );
            delete.connect_clicked(move |_| {
                let paths = chosen();
                if !paths.is_empty() {
                    tx.emit(Input::DeleteFiles(paths));
                    d.close();
                }
            });
        }
        dialog.present(Some(&self.w.win));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_editor_write_events_are_recognised() {
        assert!(external_file_changed(gio::FileMonitorEvent::Changed));
        assert!(external_file_changed(gio::FileMonitorEvent::Created));
        assert!(external_file_changed(
            gio::FileMonitorEvent::ChangesDoneHint
        ));
        assert!(!external_file_changed(gio::FileMonitorEvent::Deleted));
        assert!(!external_file_changed(
            gio::FileMonitorEvent::AttributeChanged
        ));
    }
}
