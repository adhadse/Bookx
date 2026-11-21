//! Insert Image and Insert Hyperlink dialogs (CE-354): pick a file of the book (with thumbnails or a filter that
//! understands abbreviations), an anchor inside a page, the texts, and see the markup before it goes in. The
//! markup is built by `bookx_editing::code_format`, the file ranking by `bookx_editing::fuzzy`.

use std::collections::HashMap;

use super::format_actions::CodeMsg;
use super::*;
use crate::core::util::relative_href;
use crate::i18n::tr;
use bookx_editing::code_format as cf;
use bookx_editing::fuzzy;

/// Most thumbnails decoded for the image list.
const MAX_THUMBS: usize = 300;

fn dialog_shell(title: &str) -> (adw::Dialog, gtk::Box, gtk::Button) {
    let dialog = crate::ui::dialog_toast::new_dialog();
    dialog.set_title(title);
    dialog.set_content_width(520);
    dialog.set_content_height(520);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.set_margin_top(6);
    body.set_margin_bottom(12);
    body.set_margin_start(12);
    body.set_margin_end(12);
    toolbar.set_content(Some(&body));
    dialog.set_child(Some(&toolbar));
    let insert = gtk::Button::with_label(&tr("Insert"));
    insert.add_css_class("suggested-action");
    (dialog, body, insert)
}

fn finish_buttons(body: &gtk::Box, dialog: &adw::Dialog, insert: &gtk::Button) {
    let cancel = gtk::Button::with_label(&tr("Cancel"));
    let d = dialog.clone();
    cancel.connect_clicked(move |_| {
        d.close();
    });
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_halign(gtk::Align::End);
    row.append(&cancel);
    row.append(insert);
    body.append(&row);
    dialog.set_default_widget(Some(insert));
}

fn preview_label() -> gtk::Label {
    let l = gtk::Label::new(None);
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_selectable(true);
    l.add_css_class("monospace");
    l.add_css_class("dim-label");
    crate::ui::audit::label(&l, &tr("Markup that will be inserted"));
    l
}

/// A filtered, ranked list of `items` (paths) in a scrolled window with a search entry.
struct PathList {
    search: gtk::SearchEntry,
    list: gtk::ListBox,
    scores: Rc<RefCell<HashMap<String, i32>>>,
}

fn path_list(items: &[String], body: &gtk::Box, hint: &str) -> PathList {
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some(hint));
    crate::ui::audit::label(&search, hint);
    body.append(&search);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_activate_on_single_click(false);
    crate::ui::audit::label(&list, &tr("Files of the book"));
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_min_content_height(90);
    scroll.set_child(Some(&list));
    body.append(&scroll);
    let scores: Rc<RefCell<HashMap<String, i32>>> = Rc::default();
    for p in items {
        scores.borrow_mut().insert(p.clone(), 0);
    }
    let sc = scores.clone();
    list.set_filter_func(move |row| {
        sc.borrow()
            .get(row.widget_name().as_str())
            .is_some_and(|s| *s >= 0)
    });
    let sc = scores.clone();
    list.set_sort_func(move |a, b| {
        let m = sc.borrow();
        let (x, y) = (
            m.get(a.widget_name().as_str()).copied().unwrap_or(0),
            m.get(b.widget_name().as_str()).copied().unwrap_or(0),
        );
        y.cmp(&x).into()
    });
    // Up and Down in the search entry move through the list
    for (down, sig) in [(true, "next-match"), (false, "previous-match")] {
        let l = list.clone();
        search.connect_local(sig, false, move |_| {
            step_selection(&l, down);
            None
        });
    }
    let (l, sc, all) = (list.clone(), scores.clone(), items.to_vec());
    search.connect_search_changed(move |e| {
        let q = e.text().to_string();
        {
            let mut m = sc.borrow_mut();
            for p in &all {
                m.insert(p.clone(), fuzzy::score(&q, p).unwrap_or(-1));
            }
        }
        l.invalidate_filter();
        l.invalidate_sort();
        // the best match is selected, so Enter inserts it
        let first = (0..).map_while(|i| l.row_at_index(i)).find(|r| {
            r.is_child_visible()
                && sc
                    .borrow()
                    .get(r.widget_name().as_str())
                    .is_some_and(|s| *s >= 0)
        });
        if let Some(r) = first {
            l.select_row(Some(&r));
        }
    });
    PathList {
        search,
        list,
        scores,
    }
}

fn step_selection(list: &gtk::ListBox, down: bool) {
    let rows: Vec<gtk::ListBoxRow> = (0..)
        .map_while(|i| list.row_at_index(i))
        .filter(|r| r.is_mapped() || r.is_child_visible())
        .collect();
    if rows.is_empty() {
        return;
    }
    let cur = list
        .selected_row()
        .and_then(|s| rows.iter().position(|r| *r == s));
    let next = match (cur, down) {
        (None, _) => 0,
        (Some(i), true) => (i + 1).min(rows.len() - 1),
        (Some(i), false) => i.saturating_sub(1),
    };
    list.select_row(Some(&rows[next]));
    rows[next].grab_focus();
    // keep typing in the search entry possible: focus returns on the next keypress via the key forwarding
}

impl Editor {
    fn insert_target_ok(&self) -> bool {
        let ok = self.code_buffer().is_some()
            && self.current.as_deref().is_some_and(|p| {
                bookx_editing::markup::is_markup_path(p)
                    || matches!(extension(p).as_str(), "xml" | "svg")
            });
        if !ok {
            self.toast(&tr("Open an HTML page to insert into"));
        }
        ok
    }

    pub(super) fn open_insert_image(&mut self) {
        if !self.insert_target_ok() {
            return;
        }
        let current = self.current.clone().unwrap_or_default();
        let images: Vec<String> = self
            .book
            .borrow()
            .file_list()
            .into_iter()
            .filter(|n| is_image_path(n))
            .collect();
        if images.is_empty() {
            self.toast(&tr("The book has no images yet; add some with Add Files"));
            return;
        }
        let (dialog, body, insert) = dialog_shell(&tr("Insert Image"));
        let pl = path_list(&images, &body, &tr("Filter images (c2 finds cover-2.jpg)"));
        let mut rows: Vec<(gtk::Image, String)> = Vec::new();
        for p in &images {
            let row = adw::ActionRow::new();
            row.set_widget_name(p);
            row.set_title(&glib::markup_escape_text(file_name(p)));
            row.set_subtitle(&glib::markup_escape_text(p));
            row.set_activatable(true);
            let thumb = gtk::Image::from_icon_name("image-x-generic-symbolic");
            thumb.set_pixel_size(48);
            row.add_prefix(&thumb);
            pl.list.append(&row);
            rows.push((thumb, p.clone()));
        }
        // thumbnails are decoded one per idle tick so the dialog stays responsive
        {
            let book = self.book.clone();
            let mut queue: Vec<(gtk::Image, String)> =
                rows.into_iter().take(MAX_THUMBS).rev().collect();
            glib::idle_add_local(move || {
                let Some((img, path)) = queue.pop() else {
                    return glib::ControlFlow::Break;
                };
                if img.root().is_none() {
                    return glib::ControlFlow::Break;
                }
                let data = book.borrow().archive.get(&path).map(|d| d.to_vec());
                if let Some(t) = data.and_then(|d| super::metadata::texture_from_bytes(&d)) {
                    img.set_paintable(Some(&t));
                }
                glib::ControlFlow::Continue
            });
        }
        let alt = adw::EntryRow::new();
        alt.set_title(&tr("Alternative text (leave empty for a decorative image)"));
        crate::ui::audit::label(&alt, &tr("Alternative text"));
        let title = adw::EntryRow::new();
        title.set_title(&tr("Title (optional)"));
        crate::ui::audit::label(&title, &tr("Title"));
        let g = adw::PreferencesGroup::new();
        g.add(&alt);
        g.add(&title);
        body.append(&g);
        let preview = preview_label();
        body.append(&preview);
        finish_buttons(&body, &dialog, &insert);

        let update: Rc<dyn Fn()> = {
            let (list, alt, title, preview, insert, current) = (
                pl.list.clone(),
                alt.clone(),
                title.clone(),
                preview.clone(),
                insert.clone(),
                current.clone(),
            );
            Rc::new(move || match list.selected_row() {
                Some(r) => {
                    let href = relative_href(&current, r.widget_name().as_str());
                    preview.set_text(&cf::image_markup(&href, &alt.text(), &title.text()));
                    insert.set_sensitive(true);
                }
                None => {
                    preview.set_text("");
                    insert.set_sensitive(false);
                }
            })
        };
        for w in [&alt, &title] {
            let u = update.clone();
            w.connect_changed(move |_| u());
        }
        let u = update.clone();
        pl.list.connect_row_selected(move |_, _| u());
        let accept: Rc<dyn Fn()> = {
            let (list, alt, title, dialog, tx, current) = (
                pl.list.clone(),
                alt.clone(),
                title.clone(),
                dialog.clone(),
                self.sender.input_sender().clone(),
                current.clone(),
            );
            Rc::new(move || {
                if let Some(r) = list.selected_row() {
                    tx.emit(Input::Code(CodeMsg::ImageDo {
                        href: relative_href(&current, r.widget_name().as_str()),
                        alt: alt.text().to_string(),
                        title: title.text().to_string(),
                    }));
                    dialog.close();
                }
            })
        };
        let a = accept.clone();
        insert.connect_clicked(move |_| a());
        let a = accept.clone();
        pl.list.connect_row_activated(move |_, _| a());
        let a = accept.clone();
        pl.search.connect_activate(move |_| a());
        let a = accept.clone();
        alt.connect_entry_activated(move |_| a());
        // the image of the same folder as the page first when nothing is typed
        if let Some(first) = pl.list.row_at_index(0) {
            pl.list.select_row(Some(&first));
        }
        update();
        let _ = &pl.scores;
        dialog.set_focus(Some(&pl.search));
        dialog.present(Some(&self.w.win));
    }

    pub(super) fn open_insert_link(&mut self) {
        if !self.insert_target_ok() {
            return;
        }
        let Some(buf) = self.code_buffer() else {
            return;
        };
        let current = self.current.clone().unwrap_or_default();
        let (text, cursor, sel) = self.code_state(&buf);
        let selected_plain = if sel.0 < sel.1 && cf::is_balanced(&text[sel.0..sel.1]) {
            bookx_editing::markup::plain_text(&text[sel.0..sel.1])
                .trim()
                .to_string()
        } else {
            String::new()
        };
        let existing = cf::link_at(&text, cursor);
        let files: Vec<String> = self.book.borrow().file_list();
        let (dialog, body, insert) = dialog_shell(&tr("Insert Hyperlink"));
        let pl = path_list(
            &files,
            &body,
            &tr("Filter files (ch3 finds chapter-3.xhtml)"),
        );
        for p in &files {
            let row = adw::ActionRow::new();
            row.set_widget_name(p);
            row.set_title(&glib::markup_escape_text(file_name(p)));
            row.set_subtitle(&glib::markup_escape_text(p));
            row.set_activatable(true);
            pl.list.append(&row);
        }
        let anchors = gtk::StringList::new(&[]);
        let anchor_row = adw::ComboRow::new();
        anchor_row.set_title(&tr("Anchor in the page"));
        crate::ui::audit::label(&anchor_row, &tr("Anchor in the page"));
        anchor_row.set_model(Some(&anchors));
        let address = adw::EntryRow::new();
        address.set_title(&tr("Address (a file, #anchor or a web address)"));
        crate::ui::audit::label(&address, &tr("Address"));
        let label_row = adw::EntryRow::new();
        label_row.set_title(&tr("Link text"));
        crate::ui::audit::label(&label_row, &tr("Link text"));
        label_row.set_text(&selected_plain);
        let title = adw::EntryRow::new();
        title.set_title(&tr("Title (optional)"));
        crate::ui::audit::label(&title, &tr("Title"));
        let g = adw::PreferencesGroup::new();
        g.add(&anchor_row);
        g.add(&address);
        g.add(&label_row);
        g.add(&title);
        body.append(&g);
        let preview = preview_label();
        body.append(&preview);
        finish_buttons(&body, &dialog, &insert);

        // (id, snippet) of the anchors of the selected file
        let anchor_data: Rc<RefCell<Vec<(String, String)>>> = Rc::default();
        let busy = Rc::new(Cell::new(false));
        let compose: Rc<dyn Fn()> = {
            let (address, label_row, title, preview, insert) = (
                address.clone(),
                label_row.clone(),
                title.clone(),
                preview.clone(),
                insert.clone(),
            );
            let selected_plain = selected_plain.clone();
            Rc::new(move || {
                let href = address.text().to_string();
                let t = label_row.text().to_string();
                let shown = if t.is_empty() { href.clone() } else { t };
                let text = if t_is_selection(&label_row.text(), &selected_plain)
                    && !selected_plain.is_empty()
                {
                    selected_plain.clone()
                } else {
                    shown
                };
                preview.set_text(&cf::link_markup(&href, &text, &title.text()));
                insert.set_sensitive(!href.trim().is_empty());
            })
        };
        // choosing a file or an anchor writes the address
        let on_pick: Rc<dyn Fn()> = {
            let (list, address, anchors, anchor_row, anchor_data, busy, compose, label_row) = (
                pl.list.clone(),
                address.clone(),
                anchors.clone(),
                anchor_row.clone(),
                anchor_data.clone(),
                busy.clone(),
                compose.clone(),
                label_row.clone(),
            );
            let (book, current, comp, selected_plain) = (
                self.book.clone(),
                current.clone(),
                self.comp.clone(),
                selected_plain.clone(),
            );
            Rc::new(move || {
                if busy.get() {
                    return;
                }
                busy.set(true);
                if let Some(r) = list.selected_row() {
                    let file = r.widget_name().to_string();
                    let ids = if bookx_editing::markup::is_markup_path(&file) {
                        let t = comp
                            .borrow()
                            .edited_snips
                            .get(&file)
                            .cloned()
                            .unwrap_or_else(|| {
                                book.borrow()
                                    .archive
                                    .get_string(&file)
                                    .map(|t| bookx_editing::code_marks::anchor_snippets(&t))
                                    .unwrap_or_default()
                            });
                        t
                    } else {
                        Vec::new()
                    };
                    // rebuild the anchor list only when the file changed
                    let same =
                        *anchor_data.borrow() == ids && anchors.n_items() as usize == ids.len() + 1;
                    if !same {
                        anchors.splice(0, anchors.n_items(), &[]);
                        anchors.append(&tr("(the page itself)"));
                        for (id, snip) in &ids {
                            if snip.is_empty() {
                                anchors.append(&format!("#{id}"));
                            } else {
                                anchors.append(&format!("#{id} — {snip}"));
                            }
                        }
                        *anchor_data.borrow_mut() = ids.clone();
                        anchor_row.set_selected(0);
                    }
                    let sel = anchor_row.selected() as usize;
                    let base = if file == current {
                        String::new()
                    } else {
                        relative_href(&current, &file)
                    };
                    let frag = if sel >= 1 {
                        ids.get(sel - 1).map(|(i, _)| format!("#{i}"))
                    } else {
                        None
                    };
                    let href = format!("{base}{}", frag.clone().unwrap_or_default());
                    address.set_text(&href);
                    // a blank text field follows the choice
                    if selected_plain.is_empty() && !label_row.has_focus() {
                        let default = match (&frag, sel) {
                            (Some(_), s) if s >= 1 => ids
                                .get(s - 1)
                                .map(|(_, t)| t.clone())
                                .filter(|t| !t.is_empty())
                                .unwrap_or_else(|| file_name(&file).to_string()),
                            _ => file_name(&file).to_string(),
                        };
                        label_row.set_text(&default);
                    }
                }
                busy.set(false);
                compose();
            })
        };
        let p = on_pick.clone();
        pl.list.connect_row_selected(move |_, _| p());
        let p = on_pick.clone();
        anchor_row.connect_selected_notify(move |_| p());
        for w in [&address, &label_row, &title] {
            let c = compose.clone();
            w.connect_changed(move |_| c());
        }
        let accept: Rc<dyn Fn()> = {
            let (address, label_row, title, dialog, tx, selected_plain) = (
                address.clone(),
                label_row.clone(),
                title.clone(),
                dialog.clone(),
                self.sender.input_sender().clone(),
                selected_plain.clone(),
            );
            Rc::new(move || {
                let href = address.text().to_string();
                if href.trim().is_empty() {
                    return;
                }
                let t = label_row.text().to_string();
                // an untouched text keeps the selected markup as the link content
                let text = if (!selected_plain.is_empty() && t == selected_plain) || t.is_empty() {
                    None
                } else {
                    Some(t)
                };
                tx.emit(Input::Code(CodeMsg::LinkDo {
                    href,
                    text,
                    title: title.text().to_string(),
                }));
                dialog.close();
            })
        };
        let a = accept.clone();
        insert.connect_clicked(move |_| a());
        let a = accept.clone();
        address.connect_entry_activated(move |_| a());
        let a = accept.clone();
        label_row.connect_entry_activated(move |_| a());
        let a = accept.clone();
        pl.list.connect_row_activated(move |_, _| a());
        let a = accept.clone();
        pl.search.connect_activate(move |_| a());
        if let Some(h) = existing {
            address.set_text(&h);
        }
        compose();
        dialog.set_focus(Some(&pl.search));
        dialog.present(Some(&self.w.win));
    }

    pub(super) fn do_insert_image(&mut self, href: &str, alt: &str, title: &str) {
        let Some(buf) = self.code_buffer() else {
            return;
        };
        let (text, _, sel) = self.code_state(&buf);
        match cf::insert_image(&text, sel.0..sel.1, href, alt, title) {
            Some(edit) => self.apply_code_edit(&buf, &text, &edit),
            None => self.toast(&tr("The cursor is inside a tag; move it into the text")),
        }
    }

    pub(super) fn do_insert_link(&mut self, href: &str, text_arg: Option<&str>, title: &str) {
        let Some(buf) = self.code_buffer() else {
            return;
        };
        let (text, cursor, mut sel) = self.code_state(&buf);
        // nothing selected and no text given: the caret sits in a link? then edit that link's address
        if sel.0 == sel.1 && text_arg.is_none() {
            sel = (cursor, cursor);
        }
        let fallback;
        let text_arg = match (text_arg, sel.0 == sel.1) {
            (None, true) => {
                fallback = href.rsplit(['/', '#']).next().unwrap_or(href).to_string();
                Some(fallback.as_str())
            }
            (t, _) => t,
        };
        match cf::insert_link(&text, sel.0..sel.1, href, text_arg, title) {
            Ok(edit) => self.apply_code_edit(&buf, &text, &edit),
            Err(cf::FormatError::Unbalanced) => self.toast(&tr(
                "The selection cuts through tags; select whole elements or plain text",
            )),
            Err(cf::FormatError::NotText) => {
                self.toast(&tr("The cursor is inside a tag; move it into the text"))
            }
        }
    }

    pub(super) fn dev_inserts(&mut self, c: &str) -> bool {
        if let Some(a) = c.strip_prefix("insert-image=") {
            let p: Vec<&str> = a.splitn(3, '|').collect();
            self.do_insert_image(
                p.first().copied().unwrap_or(""),
                p.get(1).copied().unwrap_or(""),
                p.get(2).copied().unwrap_or(""),
            );
            eprintln!(
                "editor_code: insert {}",
                self.code_dump(&self.w.view.buffer())
            );
        } else if let Some(a) = c.strip_prefix("insert-link=") {
            let p: Vec<&str> = a.splitn(3, '|').collect();
            let t = p.get(1).copied().filter(|t| !t.is_empty());
            self.do_insert_link(
                p.first().copied().unwrap_or(""),
                t,
                p.get(2).copied().unwrap_or(""),
            );
            eprintln!(
                "editor_code: insert {}",
                self.code_dump(&self.w.view.buffer())
            );
        } else if c == "dialog=image" {
            self.open_insert_image();
        } else if c == "dialog=link" {
            self.open_insert_link();
        } else {
            return false;
        }
        true
    }
}

fn t_is_selection(text: &str, selected: &str) -> bool {
    !selected.is_empty() && text == selected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_text_detection() {
        assert!(t_is_selection("hello", "hello"));
        assert!(!t_is_selection("", ""));
        assert!(!t_is_selection("x", "hello"));
    }
}
