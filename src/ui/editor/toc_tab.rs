//! The table-of-contents tab.

use super::*;
use crate::i18n::{tr, trf};

impl Editor {
    pub(super) fn rebuild_toc(&self) {
        while let Some(c) = self.w.toc_list.first_child() {
            self.w.toc_list.remove(&c);
        }
        for (i, t) in self.toc.iter().enumerate() {
            let row = gtk::ListBoxRow::new();
            let bx = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            bx.set_margin_start(6 + 18 * t.depth as i32);
            bx.set_margin_end(6);
            bx.set_margin_top(3);
            bx.set_margin_bottom(3);
            let e = gtk::Entry::new();
            e.set_text(&t.title);
            e.set_hexpand(true);
            e.add_css_class("flat");
            crate::ui::audit::label(&e, &tr("Contents entry title"));
            let focus = gtk::EventControllerFocus::new();
            let tx = self.sender.input_sender().clone();
            focus.connect_enter(move |_| tx.emit(Input::TocSelect(i)));
            e.add_controller(focus);
            let tx = self.sender.input_sender().clone();
            e.connect_changed(move |e| tx.emit(Input::TocTitle(i, e.text().to_string())));
            let link = gtk::Label::new(Some(file_name(&t.href)));
            link.add_css_class("dim-label");
            link.add_css_class("caption");
            link.set_ellipsize(pango::EllipsizeMode::Middle);
            link.set_max_width_chars(18);
            link.set_tooltip_text(Some(&t.href));
            bx.append(&e);
            bx.append(&link);
            row.set_child(Some(&bx));
            self.w.toc_list.append(&row);
        }
        if self.toc.is_empty() {
            let l = gtk::Label::new(Some(
                "No table of contents yet.\nUse “Generate” to build one from headings.",
            ));
            l.add_css_class("dim-label");
            l.set_margin_all(16);
            l.set_justify(gtk::Justification::Center);
            self.w.toc_list.append(&l);
        }
        if let Some(i) = self.toc_selected {
            if let Some(r) = self.w.toc_list.row_at_index(i as i32) {
                self.w.toc_list.select_row(Some(&r));
            }
        }
    }

    pub(super) fn toc_op(&mut self, op: TocOp) {
        let sel = self.toc_selected;
        match op {
            TocOp::Delete => {
                if let Some(i) = sel.filter(|i| *i < self.toc.len()) {
                    let d = self.toc[i].depth;
                    self.toc.remove(i);
                    // promote orphaned children one level
                    let mut k = i;
                    while k < self.toc.len() && self.toc[k].depth > d {
                        self.toc[k].depth -= 1;
                        k += 1;
                    }
                    self.toc_selected = if self.toc.is_empty() {
                        None
                    } else {
                        Some(i.min(self.toc.len() - 1))
                    };
                }
            }
            TocOp::Up => {
                if let Some(i) = sel.filter(|i| *i > 0 && *i < self.toc.len()) {
                    self.toc.swap(i, i - 1);
                    self.toc_selected = Some(i - 1);
                }
            }
            TocOp::Down => {
                if let Some(i) = sel.filter(|i| i + 1 < self.toc.len()) {
                    self.toc.swap(i, i + 1);
                    self.toc_selected = Some(i + 1);
                }
            }
            TocOp::Indent => {
                if let Some(i) = sel.filter(|i| *i > 0 && *i < self.toc.len()) {
                    if self.toc[i].depth <= self.toc[i - 1].depth {
                        self.toc[i].depth += 1;
                    }
                }
            }
            TocOp::Outdent => {
                if let Some(i) = sel.filter(|i| *i < self.toc.len()) {
                    self.toc[i].depth = self.toc[i].depth.saturating_sub(1);
                }
            }
            TocOp::Add => {
                let href = self
                    .current
                    .clone()
                    .filter(|c| {
                        matches!(Lang::for_path(c), Lang::Xml)
                            && extension(c) != "opf"
                            && extension(c) != "ncx"
                    })
                    .unwrap_or_default();
                let title = if href.is_empty() {
                    "New entry".to_string()
                } else {
                    file_name(&href)
                        .trim_end_matches(".xhtml")
                        .trim_end_matches(".html")
                        .to_string()
                };
                let at = sel
                    .map(|i| i + 1)
                    .unwrap_or(self.toc.len())
                    .min(self.toc.len());
                let depth = sel
                    .and_then(|i| self.toc.get(i))
                    .map(|t| t.depth)
                    .unwrap_or(0);
                self.toc.insert(at, FlatToc { depth, title, href });
                self.toc_selected = Some(at);
            }
            TocOp::LinkCurrent => {
                if let (Some(i), Some(c)) = (sel, self.current.clone()) {
                    if let Some(t) = self.toc.get_mut(i) {
                        t.href = c;
                    }
                }
            }
            TocOp::Generate => {
                self.generate_toc(&TocOptions::default());
                return;
            }
        }
        self.toc_dirty = true;
        self.rebuild_toc();
        self.update_title();
    }

    /// Selecting an entry opens its file and scrolls to its anchor; `force` also re-opens the same entry.
    pub(super) fn toc_select(&mut self, i: usize, force: bool) {
        if self.toc_selected == Some(i) && !force {
            return;
        }
        self.toc_selected = Some(i);
        if let Some(href) = self.toc.get(i).map(|t| t.href.clone()) {
            self.open_location(&href);
        }
    }

    /// Opens `path#fragment` in the code view and puts the cursor on the anchor.
    pub(super) fn open_location(&mut self, href: &str) {
        let (path, frag) = match href.split_once('#') {
            Some((p, f)) => (p.to_string(), Some(f.to_string())),
            None => (href.to_string(), None),
        };
        if path.is_empty() || !self.book.borrow().archive.contains(&path) {
            return;
        }
        if self.current.as_deref() != Some(&path) {
            self.select_file(&path);
        }
        if !is_text_path(&path) {
            return;
        }
        let buf = self.buffer_for(&path);
        let text = buffer_text(&buf);
        let mut top = buf.start_iter();
        let Some(frag) = frag else {
            buf.place_cursor(&top);
            self.w.view.scroll_to_iter(&mut top, 0.0, true, 0.0, 0.0);
            return;
        };
        buf.place_cursor(&top);
        if let Some(at) = bookx_editing::toc_gen::anchor_offset(&text, &frag) {
            let mut it = buf.iter_at_offset(super::search::byte_to_char_idx(&text, at) as i32);
            buf.place_cursor(&it);
            self.w.view.scroll_to_iter(&mut it, 0.1, true, 0.5, 0.2);
        }
    }

    pub(super) fn generate_toc(&mut self, opts: &TocOptions) {
        self.flush();
        self.commit_toc();
        let generated = bookx_editing::toc_gen::generate(&self.book.borrow(), opts);
        if generated.is_empty() {
            self.toast(&tr("Nothing to build a table of contents from"));
            return;
        }
        self.create_checkpoint("Before: Generate Contents");
        let n = generated.len();
        self.toc = bookx_editing::toc_gen::combine(&self.toc, generated, opts.mode);
        self.toc_selected = None;
        self.toc_dirty = true;
        self.rebuild_toc();
        self.update_title();
        self.toast(&trf("Contents built: {} entries", &[&n]));
    }

    pub(super) fn prompt_toc_generate(&self) {
        use bookx_editing::toc_gen::{TocMode, TocSource};
        let bx = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let source = gtk::DropDown::from_strings(&[
            "From headings",
            "From the links of a page",
            "One entry per file",
        ]);
        let levels = gtk::SpinButton::with_range(1.0, 6.0, 1.0);
        levels.set_value(3.0);
        let lrow = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let ll = gtk::Label::new(Some(&tr("Deepest heading level")));
        ll.set_hexpand(true);
        ll.set_xalign(0.0);
        lrow.append(&ll);
        lrow.append(&levels);
        let pages: Vec<String> = bookx_editing::markup::markup_files(&self.book.borrow().archive);
        let names: Vec<&str> = pages.iter().map(|p| file_name(p)).collect();
        let page = gtk::DropDown::from_strings(&names);
        page.set_tooltip_text(Some(&tr("The page whose links are used")));
        let mode = gtk::DropDown::from_strings(&["Replace the contents", "Add to the contents"]);
        for w in [
            source.upcast_ref::<gtk::Widget>(),
            lrow.upcast_ref(),
            page.upcast_ref(),
            mode.upcast_ref(),
        ] {
            bx.append(w);
        }
        let tx = self.sender.input_sender().clone();
        self.ask(
            "Generate Contents",
            "Headings up to the chosen level, the links of a contents page, or one entry per file.",
            &bx,
            "Generate",
            move || {
                tx.emit(Input::GenerateToc(TocOptions {
                    source: match source.selected() {
                        1 => TocSource::Links,
                        2 => TocSource::Files,
                        _ => TocSource::Headings,
                    },
                    max_level: levels.value() as u8,
                    mode: if mode.selected() == 1 {
                        TocMode::Append
                    } else {
                        TocMode::Replace
                    },
                    links_file: pages.get(page.selected() as usize).cloned(),
                }))
            },
        );
    }
}
