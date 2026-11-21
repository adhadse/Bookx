//! Navigation in the code view: Ctrl+click and F12 jump to the file, anchor, class rule or use under the
//! pointer (CE-342, CE-344), the class rename with preview (CE-343) and the help entries of the context menu
//! (CE-341). The decisions are in `bookx_editing::code_nav`.

use super::format_actions::CodeMsg;
use super::search::char_to_byte_idx;
use super::*;
use crate::i18n::{tr, trf};
use bookx_editing::code_nav::{self as nav, Hit, Jump};

fn is_css_buffer(buf: &gtk::TextBuffer) -> bool {
    buf.downcast_ref::<sv::Buffer>()
        .and_then(|b| b.language())
        .is_some_and(|l| l.id() == "css")
}

/// Ctrl+click and the context menu entries of the code view.
pub(super) fn install(view: &sv::View, tx: relm4::Sender<Input>) {
    let actions = gio::SimpleActionGroup::new();
    let goto = gio::SimpleAction::new("goto", Some(glib::VariantTy::INT32));
    let t = tx.clone();
    goto.connect_activate(move |_, p| {
        if let Some(off) = p.and_then(|v| v.get::<i32>()) {
            t.emit(Input::Code(CodeMsg::CtrlClick(off)));
        }
    });
    actions.add_action(&goto);
    let rename = gio::SimpleAction::new("rename-class", Some(glib::VariantTy::STRING));
    let t = tx.clone();
    rename.connect_activate(move |_, p| {
        if let Some(n) = p.and_then(|v| v.str()) {
            t.emit(Input::Code(CodeMsg::RenameClass(n.to_string())));
        }
    });
    actions.add_action(&rename);
    let help = gio::SimpleAction::new("help", Some(glib::VariantTy::STRING));
    let t = tx.clone();
    help.connect_activate(move |_, p| {
        if let Some(u) = p.and_then(|v| v.str()) {
            t.emit(Input::Code(CodeMsg::Help(u.to_string())));
        }
    });
    actions.add_action(&help);
    view.insert_action_group("code", Some(&actions));

    // Ctrl+click
    let click = gtk::GestureClick::new();
    click.set_button(1);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let v = view.clone();
    let t = tx.clone();
    click.connect_pressed(move |g, _, x, y| {
        let st = g.current_event_state();
        if !st.contains(gdk::ModifierType::CONTROL_MASK)
            || st.intersects(gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::ALT_MASK)
        {
            return;
        }
        let (bx, by) = v.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        if let Some(it) = v.iter_at_location(bx, by) {
            g.set_state(gtk::EventSequenceState::Claimed);
            t.emit(Input::Code(CodeMsg::CtrlClick(it.offset())));
        }
    });
    view.add_controller(click);
}

/// Appends the code entries (rename class, go to definition, help) to the view's context menu; called by the
/// spelling code after it filled the menu, so its `remove_all` cannot wipe them.
pub(super) fn append_code_section(
    menu: &gio::Menu,
    view: &sv::View,
    text: &str,
    at: usize,
    char_off: i32,
) {
    let section = code_menu(text, at, char_off, is_css_buffer(&view.buffer()));
    if section.n_items() > 0 {
        menu.append_section(None, &section);
    }
}

/// `(label, action, target)` of the code entries for the text at byte `at`.
fn menu_entries(
    text: &str,
    at: usize,
    char_off: i32,
    css: bool,
) -> Vec<(String, &'static str, glib::Variant)> {
    let mut v: Vec<(String, &'static str, glib::Variant)> = Vec::new();
    match nav::hit_at(text, at, css) {
        Some(Hit::Class { name, .. }) => {
            v.push((
                trf("Rename Class “{}”", &[&name]),
                "code.rename-class",
                name.to_variant(),
            ));
            v.push((tr("Go to Definition"), "code.goto", char_off.to_variant()));
        }
        Some(Hit::Link { .. }) | Some(Hit::Id { .. }) => {
            v.push((tr("Go to Definition"), "code.goto", char_off.to_variant()));
        }
        None => {}
    }
    if let Some(topic) = nav::help_topic_at(text, at, css) {
        if let Some(url) = nav::mdn_url(&topic) {
            v.push((
                trf("Help for {}", &[&topic.label()]),
                "code.help",
                url.to_variant(),
            ));
        }
    }
    v
}

fn code_menu(text: &str, at: usize, char_off: i32, css: bool) -> gio::Menu {
    let m = gio::Menu::new();
    for (label, action, target) in menu_entries(text, at, char_off, css) {
        let it = gio::MenuItem::new(Some(&label), None);
        it.set_action_and_target_value(Some(action), Some(&target));
        m.append_item(&it);
    }
    m
}

impl Editor {
    /// The text of every HTML and CSS file: the open buffer when there is one, else the archive.
    pub(super) fn all_code_texts(&self) -> Vec<(String, String)> {
        let book = self.book.borrow();
        book.archive
            .names()
            .iter()
            .filter(|n| {
                bookx_editing::markup::is_markup_path(n) || bookx_editing::markup::is_css_path(n)
            })
            .filter_map(|n| {
                let t = match self.buffers.get(n) {
                    Some(b) => buffer_text(b),
                    None => book.archive.get_string(n)?,
                };
                Some((n.clone(), t))
            })
            .collect()
    }

    fn code_text_of(&self, path: &str) -> Option<String> {
        match self.buffers.get(path) {
            Some(b) => Some(buffer_text(b)),
            None => self.book.borrow().archive.get_string(path),
        }
    }

    /// Ctrl+click at a character offset of the open file.
    pub(super) fn ctrl_click(&mut self, char_off: i32) {
        let Some(buf) = self.code_buffer() else {
            return;
        };
        let Some(from) = self.current.clone() else {
            return;
        };
        let text = buffer_text(&buf);
        let at = char_to_byte_idx(&text, char_off.max(0) as usize);
        self.jump_from(&from, &text, at);
    }

    /// F12: the same for the caret.
    pub(super) fn goto_definition(&mut self) {
        let Some(buf) = self.code_buffer() else {
            return;
        };
        let Some(from) = self.current.clone() else {
            return;
        };
        let (text, cursor, _) = self.code_state(&buf);
        self.jump_from(&from, &text, cursor);
    }

    fn jump_from(&mut self, from: &str, text: &str, at: usize) {
        let css = extension(from) == "css";
        let Some(hit) = nav::hit_at(text, at, css) else {
            self.toast(&tr("There is nothing to jump to here"));
            return;
        };
        let names = self.book.borrow().archive.names().to_vec();
        let plan = nav::plan_jump(&hit, from, text, &names, &|p| self.code_text_of(p));
        match plan {
            Jump::To { path, offset } => {
                let name = match &hit {
                    Hit::Class { name, .. } => Some(name.clone()),
                    Hit::Id { name, .. } => Some(name.clone()),
                    _ => None,
                };
                self.goto_offset(&path, offset);
                // select the class or id name so the target is easy to see
                if let (Some(n), true) = (name, is_text_path(&path)) {
                    let buf = self.w.view.buffer();
                    let t = buffer_text(&buf);
                    if t[offset.min(t.len())..].starts_with(&n) {
                        let s = super::search::byte_to_char_idx(&t, offset) as i32;
                        let e = s + n.chars().count() as i32;
                        buf.select_range(&buf.iter_at_offset(s), &buf.iter_at_offset(e));
                    }
                }
                self.w.view.grab_focus();
            }
            Jump::Missing(what) => {
                self.toast(&trf("Nothing found for {}", &[&what]));
            }
            Jump::External(url) => {
                self.toast(&trf(
                    "{} is a web address; it is not opened from the editor",
                    &[&url],
                ));
            }
        }
    }

    // ---- class rename ------------------------------------------------------------------

    pub(super) fn open_rename_class(&mut self, old: &str) {
        let texts = Rc::new(self.all_code_texts());
        let dialog = crate::ui::dialog_toast::new_dialog();
        dialog.set_title(&tr("Rename Class"));
        dialog.set_content_width(520);
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.set_margin_top(12);
        body.set_margin_bottom(12);
        body.set_margin_start(12);
        body.set_margin_end(12);
        let group = adw::PreferencesGroup::new();
        group.set_description(Some(&trf(
            "Every use of “{}” in the pages and style sheets of the book is renamed. A checkpoint is taken first.",
            &[&old],
        )));
        let entry = adw::EntryRow::new();
        entry.set_title(&tr("New class name"));
        entry.set_text(old);
        crate::ui::audit::label(&entry, &tr("New class name"));
        group.add(&entry);
        body.append(&group);
        let status = gtk::Label::new(None);
        status.set_xalign(0.0);
        status.set_wrap(true);
        status.add_css_class("dim-label");
        body.append(&status);
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        crate::ui::audit::label(&list, &tr("Files that change"));
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_min_content_height(160);
        scroll.set_max_content_height(260);
        scroll.set_propagate_natural_height(true);
        scroll.set_child(Some(&list));
        body.append(&scroll);
        let rename_btn = gtk::Button::with_label(&tr("Rename"));
        rename_btn.add_css_class("suggested-action");
        rename_btn.set_sensitive(false);
        let cancel = gtk::Button::with_label(&tr("Cancel"));
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_halign(gtk::Align::End);
        buttons.append(&cancel);
        buttons.append(&rename_btn);
        body.append(&buttons);
        toolbar.set_content(Some(&body));
        dialog.set_child(Some(&toolbar));
        dialog.set_default_widget(Some(&rename_btn));
        dialog.set_focus(Some(&entry));

        let refresh: Rc<dyn Fn()> = {
            let (entry, status, list, btn, texts) = (
                entry.clone(),
                status.clone(),
                list.clone(),
                rename_btn.clone(),
                texts.clone(),
            );
            let old = old.to_string();
            Rc::new(move || {
                while let Some(c) = list.first_child() {
                    list.remove(&c);
                }
                let new = entry.text().to_string();
                let plan = nav::rename_class(&texts, &old, &new);
                match plan {
                    None => {
                        btn.set_sensitive(false);
                        status.set_text(&if new == old {
                            tr("Type the new name")
                        } else {
                            tr("A class name starts with a letter or _ and has no spaces")
                        });
                    }
                    Some(p) => {
                        btn.set_sensitive(p.total > 0);
                        let mut s = trf("{} changes in {} files", &[&p.total, &p.changes.len()]);
                        if p.collides {
                            s.push_str(&format!(
                                " — {}",
                                trf("“{}” exists already; the two classes will merge", &[&new])
                            ));
                        }
                        status.set_text(&s);
                        for c in p.changes.iter().take(200) {
                            let row = adw::ActionRow::new();
                            row.set_title(&glib::markup_escape_text(&c.path));
                            if let Some(l) = c.lines.first() {
                                let (b, a) = clip_around(&l.before, &l.after);
                                row.set_subtitle(&glib::markup_escape_text(&format!("{b} → {a}")));
                            }
                            let n = gtk::Label::new(Some(&c.count.to_string()));
                            n.add_css_class("dim-label");
                            row.add_suffix(&n);
                            list.append(&row);
                        }
                    }
                }
            })
        };
        refresh();
        let r = refresh.clone();
        let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
        entry.connect_changed(move |_| {
            if let Some(id) = pending.borrow_mut().take() {
                id.remove();
            }
            let r = r.clone();
            let slot = pending.clone();
            let id =
                glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
                    slot.borrow_mut().take();
                    r();
                });
            *pending.borrow_mut() = Some(id);
        });
        let d = dialog.clone();
        cancel.connect_clicked(move |_| {
            d.close();
        });
        let (d, tx, old_name, e) = (
            dialog.clone(),
            self.sender.input_sender().clone(),
            old.to_string(),
            entry.clone(),
        );
        let apply = move || {
            tx.emit(Input::Code(CodeMsg::RenameDo {
                old: old_name.clone(),
                new: e.text().to_string(),
            }));
            d.close();
        };
        let a = apply.clone();
        rename_btn.connect_clicked(move |_| a());
        entry.connect_entry_activated(move |_| apply());
        dialog.present(Some(&self.w.win));
    }

    pub(super) fn do_rename_class(&mut self, old: &str, new: &str) {
        let texts = self.all_code_texts();
        let Some(plan) = nav::rename_class(&texts, old, new) else {
            self.toast(&tr("That is not a valid class name"));
            return;
        };
        if plan.total == 0 {
            self.toast(&tr("The class is not used"));
            return;
        }
        self.create_checkpoint(&format!("Before: Rename class {old}"));
        let n_files = plan.changes.len();
        for c in &plan.changes {
            let buf = self.buffer_for(&c.path);
            let cursor = buf.iter_at_mark(&buf.get_insert()).offset();
            buf.begin_user_action();
            let (mut s, mut e) = buf.bounds();
            buf.delete(&mut s, &mut e);
            let mut at = buf.start_iter();
            buf.insert(&mut at, &c.new_text);
            buf.end_user_action();
            buf.place_cursor(&buf.iter_at_offset(cursor.min(buf.char_count())));
        }
        self.toast(&trf(
            "Renamed {} uses in {} files (a checkpoint was taken)",
            &[&plan.total, &n_files],
        ));
        eprintln!("editor_code: rename uses={} files={n_files}", plan.total);
        self.run_live();
    }

    // ---- dev hooks -----------------------------------------------------------------------

    pub(super) fn dev_offset(&self, arg: &str) -> Option<(i32, usize)> {
        let (needle, k) = arg.split_once('|').unwrap_or((arg, "0"));
        let buf = self.w.view.buffer();
        let text = buffer_text(&buf);
        let b = text.find(needle)?;
        let k: usize = k.parse().unwrap_or(0);
        let c = super::search::byte_to_char_idx(&text, b) as i32 + k as i32;
        Some((c, b + k))
    }

    pub(super) fn dev_navigate(&mut self, c: &str) -> bool {
        if let Some(a) = c.strip_prefix("ctrlclick=") {
            match self.dev_offset(a) {
                Some((off, _)) => {
                    self.ctrl_click(off);
                    let buf = self.w.view.buffer();
                    eprintln!(
                        "editor_code: jumped current={} at={} toast={}",
                        self.current.clone().unwrap_or_default(),
                        buf.iter_at_mark(&buf.get_insert()).offset(),
                        self.last_toast.borrow()
                    );
                    eprintln!("editor_code: selected {}", self.code_dump(&buf));
                }
                None => eprintln!("editor_code: jumped not-found"),
            }
        } else if c == "gotodef" {
            self.goto_definition();
            eprintln!(
                "editor_code: jumped current={} toast={}",
                self.current.clone().unwrap_or_default(),
                self.last_toast.borrow()
            );
        } else if let Some(a) = c.strip_prefix("menu=") {
            if let Some((off, b)) = self.dev_offset(a) {
                let buf = self.w.view.buffer();
                let text = buffer_text(&buf);
                let css = self.current.as_deref().map(extension).as_deref() == Some("css");
                let labels: Vec<String> = menu_entries(&text, b, off, css)
                    .into_iter()
                    .map(|e| e.0)
                    .collect();
                eprintln!("editor_code: menu {}", labels.join(" | "));
            }
        } else if let Some(a) = c.strip_prefix("help=") {
            if let Some((_, b)) = self.dev_offset(a) {
                let buf = self.w.view.buffer();
                let text = buffer_text(&buf);
                let css = self.current.as_deref().map(extension).as_deref() == Some("css");
                let url = nav::help_topic_at(&text, b, css).and_then(|t| nav::mdn_url(&t));
                eprintln!("editor_code: help {}", url.unwrap_or_default());
            }
        } else if let Some(a) = c.strip_prefix("rename-preview=") {
            let (old, new) = a.split_once('|').unwrap_or((a, ""));
            let texts = self.all_code_texts();
            match nav::rename_class(&texts, old, new) {
                Some(p) => eprintln!(
                    "editor_code: rename-preview total={} files={} collides={} {}",
                    p.total,
                    p.changes.len(),
                    p.collides,
                    p.changes
                        .iter()
                        .map(|c| format!("{}:{}", c.path, c.count))
                        .collect::<Vec<_>>()
                        .join(",")
                ),
                None => eprintln!("editor_code: rename-preview invalid"),
            }
        } else if let Some(a) = c.strip_prefix("rename=") {
            let (old, new) = a.split_once('|').unwrap_or((a, ""));
            self.do_rename_class(old, new);
        } else if let Some(a) = c.strip_prefix("rename-dialog=") {
            self.open_rename_class(a);
        } else {
            return false;
        }
        true
    }
}

/// The two lines cut to a window around the first difference, so a long line still shows what changed.
fn clip_around(before: &str, after: &str) -> (String, String) {
    const WIDTH: usize = 56;
    let (b, a): (Vec<char>, Vec<char>) = (before.chars().collect(), after.chars().collect());
    let common = b.iter().zip(&a).take_while(|(x, y)| x == y).count();
    let start = common.saturating_sub(20);
    let cut = |v: &[char]| {
        let end = (start + WIDTH).min(v.len());
        let mut s: String = v[start.min(v.len())..end].iter().collect();
        if start > 0 {
            s.insert(0, '…');
        }
        if end < v.len() {
            s.push('…');
        }
        s
    };
    (cut(&b), cut(&a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_entries_for_classes_links_and_properties() {
        let html = "<p class=\"note\"><a href=\"c2.xhtml\">x</a></p>";
        let at = html.find("note").unwrap() + 1;
        let e = menu_entries(html, at, at as i32, false);
        let labels: Vec<&str> = e.iter().map(|e| e.0.as_str()).collect();
        assert!(labels.iter().any(|l| l.contains("Rename Class")));
        assert!(labels.contains(&"Go to Definition"));
        let at = html.find("c2.xhtml").unwrap() + 1;
        let e = menu_entries(html, at, at as i32, false);
        assert_eq!(
            e.len(),
            1,
            "a link has only the jump entry (its attribute name is not under the pointer)"
        );
        let css = "p { margin: 0 }";
        let e = menu_entries(css, css.find("margin").unwrap() + 1, 5, true);
        assert_eq!(e.len(), 1);
        assert!(e[0].0.contains("margin"));
        assert!(e[0].2.str().unwrap().ends_with("/CSS/margin"));
        assert!(menu_entries("plain text", 3, 3, false).is_empty());
    }

    #[test]
    fn long_lines_are_cut_around_the_change() {
        assert_eq!(
            clip_around("short", "shirt"),
            ("short".into(), "shirt".into())
        );
        let before = format!("{}class=\"old\"{}", "a".repeat(100), "b".repeat(100));
        let after = before.replace("old", "new");
        let (b, a) = clip_around(&before, &after);
        assert!(
            b.starts_with('…') && b.ends_with('…') && b.contains("old"),
            "{b}"
        );
        assert!(a.contains("new") && !a.contains("old"), "{a}");
        assert!(b.chars().count() <= 58 && a.chars().count() <= 58);
        assert_eq!(clip_around("", ""), (String::new(), String::new()));
    }
}
