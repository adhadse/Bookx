//! Tools → Reports (CE-303..CE-311): a window with a tab for Files, Words, Images, Style rules, Style classes,
//! Characters and Links. Every row is hot-linked to its place in the book (double-click or Enter), the columns
//! sort, a filter narrows the rows, and the visible rows export as CSV. The tables come from
//! `bookx_editing::reports_tables`; tabs are built when first shown.

use crate::i18n::{tr, trf};
use std::collections::HashMap;

use super::*;
use bookx_editing::reports_tables::{self as rt, Loc, Table, TableKind};

#[derive(Debug)]
pub enum Msg {
    /// Show the window, on this tab when given.
    Open(Option<TableKind>),
    /// The tab was shown: build it if needed.
    TabShown(TableKind),
    Refresh,
    /// Row `n` (an index into the table) was activated.
    Activate(TableKind, usize),
    NextPlace(TableKind),
    DeleteSelected(TableKind),
    DeleteUnused,
    Export,
    Exported(std::path::PathBuf),
}

struct Tab {
    table: Rc<Table>,
    view: gtk::ColumnView,
    model: gtk::MultiSelection,
    status: gtk::Label,
    search: gtk::SearchEntry,
    /// Which place of the selected row "Next place" goes to next: (row, place).
    cursor: Rc<Cell<(usize, usize)>>,
}

struct Win {
    window: adw::Window,
    stack: adw::ViewStack,
    holders: HashMap<TableKind, gtk::Box>,
    tabs: HashMap<TableKind, Tab>,
}

#[derive(Default)]
pub struct State {
    win: Option<Win>,
}

fn icon_for(k: TableKind) -> &'static str {
    match k {
        TableKind::Files => "folder-symbolic",
        TableKind::Words => "text-x-generic-symbolic",
        TableKind::Images => "image-x-generic-symbolic",
        TableKind::StyleRules => "format-text-rich-symbolic",
        TableKind::StyleClasses => "tag-symbolic",
        TableKind::Characters => "preferences-desktop-font-symbolic",
        TableKind::Links => "insert-link-symbolic",
    }
}

impl Editor {
    pub(super) fn reports(&mut self, msg: Msg) {
        match msg {
            Msg::Open(kind) => self.reports_open(kind),
            Msg::TabShown(k) => self.reports_build_tab(k),
            Msg::Refresh => {
                self.flush();
                let kinds: Vec<TableKind> = match &self.reports.win {
                    Some(w) => w.tabs.keys().copied().collect(),
                    None => return,
                };
                for k in kinds {
                    if let Some(w) = self.reports.win.as_mut() {
                        w.tabs.remove(&k);
                        if let Some(h) = w.holders.get(&k) {
                            while let Some(c) = h.first_child() {
                                h.remove(&c);
                            }
                        }
                    }
                }
                let cur = self.reports_current();
                if let Some(k) = cur {
                    self.reports_build_tab(k);
                }
            }
            Msg::Activate(k, row) => {
                let loc = self
                    .reports
                    .win
                    .as_ref()
                    .and_then(|w| w.tabs.get(&k))
                    .and_then(|t| t.table.rows.get(row))
                    .and_then(|r| r.locs.first().cloned());
                if let Some(l) = loc {
                    self.reports_goto(&l);
                }
            }
            Msg::NextPlace(k) => {
                let Some(t) = self.reports.win.as_ref().and_then(|w| w.tabs.get(&k)) else {
                    return;
                };
                let Some(row) = selected_rows(t).first().copied() else {
                    self.toast(&tr("Select a row first"));
                    return;
                };
                let n = t.table.rows[row].locs.len();
                if n == 0 {
                    return;
                }
                let (r, p) = t.cursor.get();
                let next = if r == row { (p + 1) % n } else { 0 };
                // links: the first place is the link, the second its target
                t.cursor.set((row, next));
                let loc = t.table.rows[row].locs[next].clone();
                self.reports_goto(&loc);
            }
            Msg::DeleteSelected(k) => {
                let paths: Vec<String> = self
                    .reports
                    .win
                    .as_ref()
                    .and_then(|w| w.tabs.get(&k))
                    .map(|t| {
                        selected_rows(t)
                            .iter()
                            .map(|r| t.table.rows[*r].key.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                self.reports_delete(paths);
            }
            Msg::DeleteUnused => {
                let paths = rt::unused_images(&self.book.borrow());
                if paths.is_empty() {
                    self.toast(&tr("Every image is used"));
                } else {
                    self.reports_delete(paths);
                }
            }
            Msg::Export => self.reports_export(),
            Msg::Exported(p) => self.toast(&trf("Saved {}", &[&p.display()])),
        }
    }

    fn reports_current(&self) -> Option<TableKind> {
        let w = self.reports.win.as_ref()?;
        let n = w.stack.visible_child_name()?;
        TableKind::from_id(&n)
    }

    fn reports_delete(&mut self, paths: Vec<String>) {
        if paths.is_empty() {
            self.toast(&tr("Select the files to delete first"));
            return;
        }
        self.delete_files(paths);
        self.reports(Msg::Refresh);
    }

    /// Opens the file of a place; with a byte range it selects that text, else it goes to the line.
    fn reports_goto(&mut self, loc: &Loc) {
        if !self.book.borrow().archive.contains(&loc.file) {
            self.toast(&tr(
                "That file is not in the book any more: refresh the report",
            ));
            return;
        }
        self.select_file(&loc.file);
        if !is_text_path(&loc.file) {
            return;
        }
        let buf = self.w.view.buffer();
        let text = buffer_text(&buf);
        if let Some((a, b)) = loc
            .range
            .filter(|(a, b)| a <= b && text.get(*a..*b).is_some())
        {
            self.select_range(&loc.file, a, b);
            return;
        }
        let off = (loc.line.max(1) as i32 - 1).min(buf.line_count() - 1);
        if let Some(mut it) = buf.iter_at_line(off) {
            buf.place_cursor(&it);
            self.w.view.scroll_to_iter(&mut it, 0.1, true, 0.5, 0.3);
        }
        self.w.view.grab_focus();
    }

    fn reports_open(&mut self, kind: Option<TableKind>) {
        self.flush();
        if self.reports.win.is_none() {
            let w = self.reports_build_window();
            self.reports.win = Some(w);
        }
        let (win, first) = {
            let w = self.reports.win.as_ref().unwrap();
            (w.window.clone(), w.tabs.is_empty())
        };
        let k = kind.unwrap_or(TableKind::Files);
        if let Some(w) = self.reports.win.as_ref() {
            w.stack.set_visible_child_name(k.id());
        }
        win.present();
        // the first tab is built by the stack's notify when it changes; build it directly otherwise
        if first
            || self
                .reports
                .win
                .as_ref()
                .is_some_and(|w| !w.tabs.contains_key(&k))
        {
            self.reports_build_tab(k);
        }
    }

    fn reports_build_window(&self) -> Win {
        let window = adw::Window::new();
        window.set_title(Some(&tr("Reports")));
        window.set_default_size(980, 640);
        window.set_transient_for(Some(&self.w.win));
        window.set_destroy_with_parent(true);
        let stack = adw::ViewStack::new();
        let mut holders = HashMap::new();
        for k in TableKind::ALL {
            let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
            holder.set_vexpand(true);
            let page = stack.add_titled(&holder, Some(k.id()), k.title());
            page.set_icon_name(Some(icon_for(k)));
            holders.insert(k, holder);
        }
        let switcher = adw::ViewSwitcher::new();
        switcher.set_stack(Some(&stack));
        switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
        let bar = adw::ViewSwitcherBar::new();
        bar.set_stack(Some(&stack));
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&switcher));
        let export = gtk::Button::from_icon_name("document-save-symbolic");
        export.set_tooltip_text(Some(&tr("Export the rows shown as CSV")));
        export.update_property(&[gtk::accessible::Property::Label(&tr(
            "Export the rows shown as CSV",
        ))]);
        let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh.set_tooltip_text(Some(&tr(
            "Build the reports again from the book as it is now",
        )));
        refresh.update_property(&[gtk::accessible::Property::Label(&tr("Refresh the reports"))]);
        header.pack_start(&refresh);
        header.pack_end(&export);
        let tx = self.sender.input_sender().clone();
        export.connect_clicked(move |_| tx.emit(Input::Reports(Msg::Export)));
        let tx = self.sender.input_sender().clone();
        refresh.connect_clicked(move |_| tx.emit(Input::Reports(Msg::Refresh)));
        let tx = self.sender.input_sender().clone();
        stack.connect_visible_child_name_notify(move |s| {
            if let Some(k) = s.visible_child_name().and_then(|n| TableKind::from_id(&n)) {
                tx.emit(Input::Reports(Msg::TabShown(k)));
            }
        });
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.add_bottom_bar(&bar);
        view.set_content(Some(&stack));
        window.set_content(Some(&view));
        let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            720.0,
            adw::LengthUnit::Sp,
        ));
        bp.add_setter(&bar, "reveal", Some(&true.to_value()));
        bp.add_setter(&switcher, "visible", Some(&false.to_value()));
        window.add_breakpoint(bp);
        let sc = gtk::ShortcutController::new();
        sc.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string("Escape"),
            Some(gtk::CallbackAction::new({
                let w = window.clone();
                move |_, _| {
                    w.close();
                    glib::Propagation::Stop
                }
            })),
        ));
        window.add_controller(sc);
        // closing only hides the window, so the tabs stay
        window.set_hide_on_close(true);
        Win {
            window,
            stack,
            holders,
            tabs: HashMap::new(),
        }
    }

    fn reports_build_tab(&mut self, kind: TableKind) {
        match self.reports.win.as_ref() {
            Some(w) if !w.tabs.contains_key(&kind) => {}
            _ => return,
        }
        self.flush();
        let table = Rc::new(rt::build_table(&self.book.borrow(), kind));
        let tab = make_tab(&table, kind, self.sender.input_sender().clone());
        let holder = self.reports.win.as_ref().unwrap().holders[&kind].clone();
        holder.append(&tab_widget(&tab, kind, self.sender.input_sender().clone()));
        if let Some(w) = self.reports.win.as_mut() {
            w.tabs.insert(kind, tab);
        }
        update_status(self.reports.win.as_ref().unwrap().tabs.get(&kind).unwrap());
    }

    fn reports_export(&mut self) {
        let Some(k) = self.reports_current() else {
            return;
        };
        self.reports_build_tab(k);
        let name = format!(
            "{}-{}.csv",
            self.book_title()
                .chars()
                .map(|c| if c.is_alphanumeric() { c } else { '-' })
                .collect::<String>(),
            k.id()
        );
        let dlg = gtk::FileDialog::builder()
            .title(tr("Export the Report"))
            .initial_name(name)
            .build();
        let tx = self.sender.input_sender().clone();
        let csv = self.reports_csv(k);
        dlg.save(
            Some(&self.reports.win.as_ref().unwrap().window),
            gio::Cancellable::NONE,
            move |r| {
                if let Some(p) = r.ok().and_then(|f| f.path()) {
                    match std::fs::write(&p, csv.unwrap_or_default()) {
                        Ok(()) => tx.emit(Input::Reports(Msg::Exported(p))),
                        Err(e) => tx.emit(Input::SpellPanel(super::spelling::Msg::Toast(trf(
                            "Could not save: {}",
                            &[&e],
                        )))),
                    }
                }
            },
        );
    }

    /// CSV of the rows shown (filtered and sorted as on screen) in a tab.
    fn reports_csv(&self, k: TableKind) -> Option<String> {
        let t = self.reports.win.as_ref()?.tabs.get(&k)?;
        let idx = visible_rows(t);
        Some(t.table.to_csv(&idx))
    }

    /// Dev hooks of the reports window (`reports:…`), see build-aux/ui-tests.sh.
    pub(super) fn dev_reports(&mut self, c: &str) -> bool {
        let Some(c) = c.strip_prefix("reports:") else {
            return false;
        };
        if let Some(t) = c.strip_prefix("open=") {
            self.reports(Msg::Open(TableKind::from_id(t)));
        } else if c == "open" {
            self.reports(Msg::Open(None));
        } else if let Some(t) = c.strip_prefix("tab=") {
            if let (Some(k), Some(w)) = (TableKind::from_id(t), self.reports.win.as_ref()) {
                w.stack.set_visible_child_name(k.id());
            }
            if let Some(k) = TableKind::from_id(t) {
                self.reports_build_tab(k);
            }
        } else if let Some(t) = c.strip_prefix("filter=") {
            if let Some(tab) = self
                .reports_current()
                .and_then(|k| self.reports.win.as_ref()?.tabs.get(&k))
            {
                tab.search.set_text(t);
            }
        } else if let Some(a) = c.strip_prefix("sort=") {
            // `sort=<column index>|asc|desc`
            let (col, dir) = a.split_once('|').unwrap_or((a, "asc"));
            if let Some(tab) = self
                .reports_current()
                .and_then(|k| self.reports.win.as_ref()?.tabs.get(&k))
            {
                if let Some(cc) = col
                    .parse::<u32>()
                    .ok()
                    .and_then(|i| tab.view.columns().item(i))
                    .and_downcast::<gtk::ColumnViewColumn>()
                {
                    tab.view.sort_by_column(
                        Some(&cc),
                        if dir == "desc" {
                            gtk::SortType::Descending
                        } else {
                            gtk::SortType::Ascending
                        },
                    );
                }
            }
        } else if let Some(n) = c.strip_prefix("activate=") {
            // the n-th visible row
            if let Some(k) = self.reports_current() {
                let row = self
                    .reports
                    .win
                    .as_ref()
                    .and_then(|w| w.tabs.get(&k))
                    .and_then(|t| {
                        visible_rows(t)
                            .get(n.parse::<usize>().unwrap_or(0))
                            .copied()
                    });
                if let Some(r) = row {
                    self.reports(Msg::Activate(k, r));
                }
            }
        } else if let Some(n) = c.strip_prefix("select=") {
            if let Some(tab) = self
                .reports_current()
                .and_then(|k| self.reports.win.as_ref()?.tabs.get(&k))
            {
                tab.model.unselect_all();
                for i in n.split('|').filter_map(|s| s.parse::<u32>().ok()) {
                    tab.model.select_item(i, false);
                }
            }
        } else if c == "next-place" {
            if let Some(k) = self.reports_current() {
                self.reports(Msg::NextPlace(k));
            }
        } else if c == "delete-selected" {
            if let Some(k) = self.reports_current() {
                self.reports(Msg::DeleteSelected(k));
            }
        } else if c == "delete-unused" {
            self.reports(Msg::DeleteUnused);
        } else if c == "refresh" {
            self.reports(Msg::Refresh);
        } else if let Some(p) = c.strip_prefix("csv=") {
            if let Some(csv) = self.reports_current().and_then(|k| self.reports_csv(k)) {
                let _ = std::fs::write(p, csv);
            }
        } else if c == "close" {
            if let Some(w) = self.reports.win.as_ref() {
                w.window.close();
            }
        } else if c == "dump" {
            let Some(k) = self.reports_current() else {
                eprintln!("editor_reports: closed");
                return true;
            };
            let Some(t) = self.reports.win.as_ref().and_then(|w| w.tabs.get(&k)) else {
                eprintln!("editor_reports: tab={} not built", k.id());
                return true;
            };
            let vis = visible_rows(t);
            let first: Vec<String> = vis
                .iter()
                .take(4)
                .map(|i| {
                    t.table.rows[*i]
                        .cells
                        .iter()
                        .map(|c| c.text.as_str())
                        .collect::<Vec<_>>()
                        .join("/")
                })
                .collect();
            eprintln!(
                "editor_reports: tab={} rows={} shown={} cols={} first={} current={} line={} status={}",
                k.id(),
                t.table.rows.len(),
                vis.len(),
                t.table.columns.len(),
                first.join(" | "),
                self.current.clone().unwrap_or_default(),
                self.w
                    .view
                    .buffer()
                    .iter_at_offset(self.w.view.buffer().cursor_position())
                    .line()
                    + 1,
                t.status.label()
            );
        }
        true
    }
}

fn row_index(obj: &glib::Object) -> usize {
    obj.downcast_ref::<glib::BoxedAnyObject>()
        .map(|o| *o.borrow::<usize>())
        .unwrap_or(0)
}

/// Table indices of the selected rows.
fn selected_rows(t: &Tab) -> Vec<usize> {
    let sel = t.model.selection();
    let mut out = Vec::new();
    for pos in 0..t.model.n_items() {
        if sel.contains(pos) {
            if let Some(o) = t.model.item(pos) {
                out.push(row_index(&o));
            }
        }
    }
    out
}

/// Table indices of the rows on screen, in screen order.
fn visible_rows(t: &Tab) -> Vec<usize> {
    (0..t.model.n_items())
        .filter_map(|p| t.model.item(p))
        .map(|o| row_index(&o))
        .collect()
}

fn update_status(t: &Tab) {
    let shown = t.model.n_items() as usize;
    let all = t.table.rows.len();
    let mut s = if shown == all {
        rt::summary(&t.table)
    } else {
        format!("{shown} of {all} rows shown")
    };
    if t.model.selection().size() > 0 {
        s.push_str(&format!(" · {} selected", t.model.selection().size()));
    }
    t.status.set_label(&s);
}

fn make_tab(table: &Rc<Table>, kind: TableKind, tx: relm4::Sender<Input>) -> Tab {
    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    for i in 0..table.rows.len() {
        store.append(&glib::BoxedAnyObject::new(i));
    }
    let needle = Rc::new(RefCell::new(String::new()));
    let filter = {
        let (t, n) = (table.clone(), needle.clone());
        gtk::CustomFilter::new(move |o| t.matches(row_index(o), &n.borrow()))
    };
    let filtered = gtk::FilterListModel::new(Some(store), Some(filter.clone()));
    let view = gtk::ColumnView::new(None::<gtk::SelectionModel>);
    let sorted = gtk::SortListModel::new(Some(filtered), view.sorter());
    let model = gtk::MultiSelection::new(Some(sorted));
    view.set_model(Some(&model));
    view.set_show_column_separators(true);
    view.set_show_row_separators(true);
    view.set_vexpand(true);
    view.update_property(&[gtk::accessible::Property::Label(kind.title())]);
    for (ci, (title, numeric)) in table.columns.iter().enumerate() {
        let factory = gtk::SignalListItemFactory::new();
        let numeric = *numeric;
        factory.connect_setup(move |_, item| {
            let l = gtk::Label::new(None);
            l.set_xalign(if numeric { 1.0 } else { 0.0 });
            l.set_ellipsize(pango::EllipsizeMode::End);
            l.set_margin_start(6);
            l.set_margin_end(6);
            if numeric {
                l.add_css_class("numeric");
            }
            if let Some(i) = item.downcast_ref::<gtk::ListItem>() {
                i.set_child(Some(&l));
            }
        });
        let t = table.clone();
        factory.connect_bind(move |_, item| {
            let Some(i) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let (Some(o), Some(l)) = (i.item(), i.child().and_downcast::<gtk::Label>()) else {
                return;
            };
            let text = &t.rows[row_index(&o)].cells[ci].text;
            l.set_text(text);
            l.set_tooltip_text(Some(text));
        });
        let col = gtk::ColumnViewColumn::new(Some(title), Some(factory));
        col.set_resizable(true);
        col.set_expand(ci == 0 || (kind == TableKind::Links && ci < 3));
        let t = table.clone();
        col.set_sorter(Some(&gtk::CustomSorter::new(move |a, b| {
            t.compare(ci, row_index(a), row_index(b)).into()
        })));
        view.append_column(&col);
    }
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.add_css_class("dim-label");
    status.set_hexpand(true);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some(&tr("Filter")));
    search.update_property(&[gtk::accessible::Property::Label(&tr("Filter the rows"))]);
    search.set_hexpand(true);
    {
        let (n, f, model2, st) = (
            needle.clone(),
            filter.clone(),
            model.clone(),
            status.clone(),
        );
        let t = table.clone();
        search.connect_search_changed(move |e| {
            *n.borrow_mut() = e.text().trim().to_lowercase();
            f.changed(gtk::FilterChange::Different);
            let (shown, all) = (model2.n_items() as usize, t.rows.len());
            st.set_label(&if shown == all {
                rt::summary(&t)
            } else {
                format!("{shown} of {all} rows shown")
            });
        });
        let (model2, t, st) = (model.clone(), table.clone(), status.clone());
        model.connect_selection_changed(move |_, _, _| {
            let n = model2.selection().size();
            let base = if model2.n_items() as usize == t.rows.len() {
                rt::summary(&t)
            } else {
                format!("{} of {} rows shown", model2.n_items(), t.rows.len())
            };
            st.set_label(&if n > 0 {
                format!("{base} · {n} selected")
            } else {
                base
            });
        });
        let (model2, tx) = (model.clone(), tx);
        view.connect_activate(move |_, pos| {
            if let Some(o) = model2.item(pos) {
                tx.emit(Input::Reports(Msg::Activate(kind, row_index(&o))));
            }
        });
    }
    Tab {
        table: table.clone(),
        view,
        model,
        status,
        search,
        cursor: Rc::new(Cell::new((usize::MAX, 0))),
    }
}

fn tab_widget(tab: &Tab, kind: TableKind, tx: relm4::Sender<Input>) -> gtk::Widget {
    let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
    tab.search.set_margin_start(12);
    tab.search.set_margin_end(12);
    tab.search.set_margin_top(8);
    col.append(&tab.search);
    let sc = gtk::ScrolledWindow::new();
    sc.set_child(Some(&tab.view));
    sc.set_vexpand(true);
    col.append(&sc);
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bar.set_margin_start(12);
    bar.set_margin_end(12);
    bar.set_margin_bottom(8);
    bar.append(&tab.status);
    let button = |label: &str, tip: &str, msg: fn(TableKind) -> Msg, destructive: bool| {
        let b = gtk::Button::with_label(label);
        b.set_tooltip_text(Some(tip));
        if destructive {
            b.add_css_class("destructive-action");
        }
        let tx = tx.clone();
        b.connect_clicked(move |_| tx.emit(Input::Reports(msg(kind))));
        b
    };
    match kind {
        TableKind::Files => {
            let b = button(
                &tr("Delete Selected"),
                &tr("Delete the selected files from the book"),
                Msg::DeleteSelected,
                true,
            );
            bar.append(&b);
        }
        TableKind::Images => {
            let b = gtk::Button::with_label(&tr("Delete Unused"));
            b.set_tooltip_text(Some(&tr("Delete every image that nothing refers to")));
            b.add_css_class("destructive-action");
            let tx = tx.clone();
            b.connect_clicked(move |_| tx.emit(Input::Reports(Msg::DeleteUnused)));
            bar.append(&b);
        }
        TableKind::Links => {
            bar.append(&button(
                &tr("Go to Target"),
                &tr("Go to the place the selected link points to"),
                Msg::NextPlace,
                false,
            ));
        }
        TableKind::Words | TableKind::Characters | TableKind::StyleClasses => {
            bar.append(&button(
                &tr("Next Place"),
                &tr("Go to the next place where the selected row occurs"),
                Msg::NextPlace,
                false,
            ));
        }
        TableKind::StyleRules => {}
    }
    col.append(&bar);
    col.upcast()
}
