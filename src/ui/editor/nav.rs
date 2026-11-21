//! Navigation between files: tabs for the files that were opened, next/previous file in the reading
//! order, and quick open (Ctrl+P).

use super::*;
use crate::i18n::tr;

impl Editor {
    fn tab_for(&self, path: &str) -> Option<adw::TabPage> {
        (0..self.w.tabs.n_pages())
            .map(|i| self.w.tabs.nth_page(i))
            .find(|p| tab_path(p).as_deref() == Some(path))
    }

    /// Makes sure a tab for `path` exists and is the selected one, without telling the editor again.
    pub(super) fn show_tab(&self, path: &str) {
        let page = self.tab_for(path).unwrap_or_else(|| {
            let page = self
                .w
                .tabs
                .append(&gtk::Box::new(gtk::Orientation::Vertical, 0));
            page.set_title(file_name(path));
            page.set_tooltip(path);
            super::row_data::set_path(&page, path.to_string());
            page
        });
        self.tabs_guard.set(true);
        self.w.tabs.set_selected_page(&page);
        self.tabs_guard.set(false);
    }

    /// Drops the tabs of files that no longer exist (after a rename, delete, restore…).
    pub(super) fn prune_tabs(&self) {
        self.tabs_guard.set(true);
        let gone: Vec<adw::TabPage> = (0..self.w.tabs.n_pages())
            .map(|i| self.w.tabs.nth_page(i))
            .filter(|p| tab_path(p).is_none_or(|path| !self.book.borrow().archive.contains(&path)))
            .collect();
        for p in gone {
            self.w.tabs.close_page(&p);
        }
        self.tabs_guard.set(false);
    }

    pub(super) fn tab_paths(&self) -> Vec<String> {
        (0..self.w.tabs.n_pages())
            .filter_map(|i| tab_path(&self.w.tabs.nth_page(i)))
            .collect()
    }

    pub(super) fn close_current_tab(&self) {
        if let Some(p) = self.w.tabs.selected_page() {
            self.w.tabs.close_page(&p);
        }
    }

    /// The next (`step` = 1) or previous (-1) file in the reading order, from the open one.
    pub(super) fn step_file(&mut self, step: i32) {
        let spine = self.book.borrow().spine_paths();
        if spine.is_empty() {
            return;
        }
        let at = self
            .current
            .as_ref()
            .and_then(|c| spine.iter().position(|s| s == c));
        let next = match at {
            Some(i) => (i as i32 + step).clamp(0, spine.len() as i32 - 1) as usize,
            None => 0,
        };
        self.select_file(&spine[next].clone());
    }

    pub(super) fn quick_open_results(&self, query: &str) -> Vec<String> {
        let files: Vec<String> = self
            .book
            .borrow()
            .file_list()
            .into_iter()
            .filter(|f| f != "mimetype")
            .collect();
        bookx_editing::files::quick_open(query, &files)
    }

    pub(super) fn quick_open(&self) {
        let pop = gtk::Popover::new();
        pop.set_parent(&self.w.win);
        pop.set_position(gtk::PositionType::Bottom);
        pop.set_has_arrow(false);
        let bx = gtk::Box::new(gtk::Orientation::Vertical, 6);
        bx.set_margin_all(8);
        bx.set_width_request(420);
        let entry = gtk::SearchEntry::new();
        entry.set_placeholder_text(Some(&tr("Open a file by part of its name")));
        let list = gtk::ListBox::new();
        list.add_css_class("navigation-sidebar");
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_min_content_height(260);
        scroll.set_child(Some(&list));
        bx.append(&entry);
        bx.append(&scroll);
        pop.set_child(Some(&bx));

        let all = self.quick_open_results("");
        let fill = {
            let list = list.clone();
            Rc::new(move |q: &str, all: &[String]| {
                list.remove_all();
                for p in bookx_editing::files::quick_open(q, all)
                    .into_iter()
                    .take(40)
                {
                    let row = gtk::ListBoxRow::new();
                    let l = gtk::Label::new(Some(&p));
                    l.set_xalign(0.0);
                    l.set_ellipsize(pango::EllipsizeMode::Start);
                    l.set_margin_all(4);
                    row.set_child(Some(&l));
                    super::row_data::set_path(&row, p);
                    list.append(&row);
                }
                if let Some(r) = list.row_at_index(0) {
                    list.select_row(Some(&r));
                }
            })
        };
        fill("", &all);
        {
            let fill = fill.clone();
            let all = all.clone();
            entry.connect_search_changed(move |e| fill(&e.text(), &all));
        }
        let open = {
            let tx = self.sender.input_sender().clone();
            let pop = pop.clone();
            Rc::new(move |row: &gtk::ListBoxRow| {
                if let Some(p) = super::row_data::path(row) {
                    tx.emit(Input::SelectFile(p));
                }
                pop.popdown();
            })
        };
        {
            let open = open.clone();
            list.connect_row_activated(move |_, r| open(r));
        }
        {
            let list = list.clone();
            entry.connect_activate(move |_| {
                if let Some(r) = list.selected_row() {
                    open(&r);
                }
            });
        }
        {
            let list = list.clone();
            let keys = gtk::EventControllerKey::new();
            keys.connect_key_pressed(move |_, key, _, _| {
                let step = match key {
                    gdk::Key::Down => 1,
                    gdk::Key::Up => -1,
                    _ => return glib::Propagation::Proceed,
                };
                let at = list.selected_row().map_or(0, |r| r.index()) + step;
                if let Some(r) = list.row_at_index(at.max(0)) {
                    list.select_row(Some(&r));
                }
                glib::Propagation::Stop
            });
            entry.add_controller(keys);
        }
        pop.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        pop.popup();
        entry.grab_focus();
    }
}

fn tab_path(p: &adw::TabPage) -> Option<String> {
    super::row_data::path(p)
}
