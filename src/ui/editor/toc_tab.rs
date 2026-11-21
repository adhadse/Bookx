//! The table-of-contents tab.

use super::*;
use crate::i18n::{tr, trf};
use bookx_editing::toc_ops;

impl Editor {
    /// Refills the Contents list. It is a virtual `ListView`: only the rows in view have widgets, so a book
    /// with 20 000 entries costs one store update, not 20 000 rows of widgets.
    pub(super) fn rebuild_toc(&self) {
        let st = &self.ext.structure;
        // Keyboard moves must keep the moved entry in view; a drag already happened in view, and scrolling
        // the virtual list here would pull the rows out from under the pointer between two drops.
        let refocus = st.toc_refocus.replace(false);
        st.toc_syncing.set(true);
        let items: Vec<glib::BoxedAnyObject> = self
            .toc
            .iter()
            .map(|t| glib::BoxedAnyObject::new(t.clone()))
            .collect();
        self.w
            .toc_store
            .splice(0, self.w.toc_store.n_items(), &items);
        if let Some(h) = st.toc_hint.borrow().as_ref() {
            h.set_visible(self.toc.is_empty());
        }
        let sel = self
            .toc_selected
            .filter(|i| *i < self.toc.len())
            .map_or(gtk::INVALID_LIST_POSITION, |i| i as u32);
        self.w.toc_sel.set_selected(sel);
        st.toc_syncing.set(false);
        if refocus && sel != gtk::INVALID_LIST_POSITION {
            // The focused entry is brought into view by GTK; an explicit `scroll_to` here aligns it to the
            // edge of the virtual list and hides the neighbour that gives the move its context.
            let entries = st.toc_entries.clone();
            glib::idle_add_local_once(move || {
                let hit = entries.borrow().iter().find_map(|(i, e)| {
                    let (i, e) = (i.upgrade()?, e.upgrade()?);
                    (i.position() == sel).then_some(e)
                });
                if let Some(e) = hit {
                    e.grab_focus();
                }
            });
        }
    }

    pub(super) fn toc_op(&mut self, op: TocOp) {
        let sel = self.toc_selected;
        match op {
            TocOp::Delete => {
                if let Some(i) = sel.filter(|i| *i < self.toc.len()) {
                    let title = self.toc[i].title.clone();
                    *self.toc_undo.borrow_mut() = Some((self.toc.clone(), self.toc_selected));
                    toc_ops::delete(&mut self.toc, i, false);
                    self.toast_toc_undo(&trf("Deleted “{}” from the contents", &[&title]));
                    self.toc_selected = if self.toc.is_empty() {
                        None
                    } else {
                        Some(i.min(self.toc.len() - 1))
                    };
                }
            }
            TocOp::Up => {
                if let Some(n) = sel.and_then(|i| toc_ops::move_up(&mut self.toc, i)) {
                    self.toc_selected = Some(n);
                }
            }
            TocOp::Down => {
                if let Some(n) = sel.and_then(|i| toc_ops::move_down(&mut self.toc, i)) {
                    self.toc_selected = Some(n);
                }
            }
            TocOp::Indent => {
                if let Some(i) = sel {
                    toc_ops::demote(&mut self.toc, i);
                }
            }
            TocOp::Outdent => {
                if let Some(n) = sel.and_then(|i| toc_ops::promote(&mut self.toc, i)) {
                    self.toc_selected = Some(n);
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
                let mut href = href;
                let mut title = if href.is_empty() {
                    "New entry".to_string()
                } else {
                    file_name(&href)
                        .trim_end_matches(".xhtml")
                        .trim_end_matches(".html")
                        .to_string()
                };
                if let Some((id, t)) = self.anchor_at_cursor() {
                    href = format!("{href}#{id}");
                    if !t.is_empty() {
                        title = t;
                    }
                }
                let at = toc_ops::insert_after(
                    &mut self.toc,
                    sel,
                    FlatToc {
                        depth: 0,
                        title,
                        href,
                    },
                );
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

    /// The Undo button of the delete toast: the contents as they were.
    pub(super) fn toc_undo_delete(&mut self) {
        let Some((toc, sel)) = self.toc_undo.borrow_mut().take() else {
            return;
        };
        self.toc = toc;
        self.toc_selected = sel;
        self.toc_dirty = true;
        self.rebuild_toc();
        self.update_title();
        self.toast(&tr("Undone"));
    }

    fn toast_toc_undo(&self, text: &str) {
        let _ = self.toast_button(text, &tr("Undo"), || Input::TocUndo);
    }

    /// The `id` the cursor of the open page sits in or after, with a title for it.
    fn anchor_at_cursor(&self) -> Option<(String, String)> {
        let cur = self.current.as_ref().filter(|c| is_text_path(c))?;
        let buf = self.buffers.get(cur)?;
        let text = buffer_text(buf);
        let off = buf.iter_at_mark(&buf.get_insert()).offset() as usize;
        let byte = text
            .char_indices()
            .nth(off)
            .map(|(b, _)| b)
            .unwrap_or(text.len());
        toc_ops::nearest_anchor(&text, byte)
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
        if !is_text_path(&path) || self.w.center.visible_child_name().as_deref() == Some("longline")
        {
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

/// What the Contents list is made of: the virtual list, its store (one `BoxedAnyObject<FlatToc>` per entry),
/// its selection and the hint over it.
pub(super) struct TocList {
    pub list: gtk::ListView,
    pub store: gio::ListStore,
    pub sel: gtk::SingleSelection,
    pub root: gtk::Overlay,
}

pub(super) fn toc_list_new(tx: relm4::Sender<Input>, st: &super::struct_hub::State) -> TocList {
    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    let sel = gtk::SingleSelection::new(Some(store.clone()));
    sel.set_autoselect(false);
    sel.set_can_unselect(true);
    let list = gtk::ListView::new(Some(sel.clone()), Some(toc_factory(tx.clone(), st)));
    list.add_css_class("navigation-sidebar");
    list.set_single_click_activate(true);
    {
        let (tx, syncing) = (tx.clone(), st.toc_syncing.clone());
        sel.connect_selected_notify(move |s| {
            if !syncing.get() && s.selected() != gtk::INVALID_LIST_POSITION {
                tx.emit(Input::TocSelect(s.selected() as usize));
            }
        });
    }
    list.connect_activate(move |_, i| tx.emit(Input::TocGo(i as usize)));
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&list));
    let hint = gtk::Label::new(Some(&tr(
        "No table of contents yet.\nUse “Generate” to build one from headings.",
    )));
    hint.add_css_class("dim-label");
    hint.set_margin_start(16);
    hint.set_margin_end(16);
    hint.set_justify(gtk::Justification::Center);
    hint.set_wrap(true);
    hint.set_valign(gtk::Align::Center);
    hint.set_can_target(false);
    hint.set_visible(false);
    let root = gtk::Overlay::new();
    root.set_vexpand(true);
    root.set_child(Some(&scroll));
    root.add_overlay(&hint);
    *st.toc_hint.borrow_mut() = Some(hint);
    TocList {
        list,
        store,
        sel,
        root,
    }
}

fn toc_factory(
    tx: relm4::Sender<Input>,
    st: &super::struct_hub::State,
) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    // set while bind fills an entry, so that it does not come back as an edit
    let binding = Rc::new(Cell::new(false));
    let entries = st.toc_entries.clone();
    {
        let (tx, binding) = (tx, binding.clone());
        factory.connect_setup(move |_, obj| {
            let Some(item) = obj.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let bx = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            bx.append(&super::toc_dnd::toc_row_dnd(&bx, item, tx.clone()));
            bx.set_margin_end(6);
            bx.set_margin_top(3);
            bx.set_margin_bottom(3);
            let e = gtk::Entry::new();
            e.set_hexpand(true);
            e.add_css_class("flat");
            crate::ui::audit::label(&e, &tr("Contents entry title"));
            let focus = gtk::EventControllerFocus::new();
            {
                let (tx, item) = (tx.clone(), item.downgrade());
                focus.connect_enter(move |_| {
                    if let Some(i) = item.upgrade() {
                        tx.emit(Input::TocSelect(i.position() as usize));
                    }
                });
            }
            e.add_controller(focus);
            {
                let (tx, item, binding) = (tx.clone(), item.downgrade(), binding.clone());
                e.connect_changed(move |e| {
                    if let (false, Some(i)) = (binding.get(), item.upgrade()) {
                        tx.emit(Input::TocTitle(i.position() as usize, e.text().to_string()));
                    }
                });
            }
            let link = gtk::Label::new(None);
            link.add_css_class("dim-label");
            link.add_css_class("caption");
            link.set_ellipsize(pango::EllipsizeMode::Middle);
            link.set_max_width_chars(18);
            bx.append(&e);
            bx.append(&link);
            item.set_child(Some(&bx));
        });
    }
    factory.connect_bind(move |_, obj| {
        let Some(item) = obj.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let (Some(bx), Some(data)) = (
            item.child().and_downcast::<gtk::Box>(),
            item.item().and_downcast::<glib::BoxedAnyObject>(),
        ) else {
            return;
        };
        let t = data.borrow::<FlatToc>();
        let Some(e) = bx
            .first_child()
            .and_then(|g| g.next_sibling())
            .and_downcast::<gtk::Entry>()
        else {
            return;
        };
        let Some(link) = e.next_sibling().and_downcast::<gtk::Label>() else {
            return;
        };
        bx.set_margin_start(6 + 18 * t.depth as i32);
        binding.set(true);
        if e.text() != t.title {
            e.set_text(&t.title);
        }
        binding.set(false);
        link.set_text(file_name(&t.href));
        link.set_tooltip_text(Some(&t.href));
        let mut map = entries.borrow_mut();
        map.retain(|(i, w)| i.upgrade().is_some() && w.upgrade().is_some());
        if !map.iter().any(|(i, _)| i.upgrade().as_ref() == Some(item)) {
            map.push((item.downgrade(), e.downgrade()));
        }
    });
    factory
}
