//! Wave 9 of the library window: the dev hooks that look at the table, the cards, the details
//! toggle, the sidebar groups and the search tips (see `build-aux/ui-tests-lib8.sh`). A child of
//! `library`, so it sees the window's own state.

use super::*;
use crate::i18n::{tr, trf};

/// The widget with this name, searching the tree below `w`.
pub(super) fn find_named(w: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if w.widget_name() == name {
        return Some(w.clone());
    }
    let mut c = w.first_child();
    while let Some(ch) = c {
        if let Some(f) = find_named(&ch, name) {
            return Some(f);
        }
        c = ch.next_sibling();
    }
    None
}

/// A short name for a focused widget: its kind and what names it.
fn describe_focus(w: &gtk::Widget) -> String {
    let kind = w.type_().name().trim_start_matches("Gtk").to_string();
    let named = w.widget_name();
    let text = w
        .tooltip_text()
        .map(|t| t.to_string())
        .or_else(|| {
            w.downcast_ref::<gtk::Button>()
                .and_then(|b| b.label())
                .map(|l| l.to_string())
        })
        .or_else(|| {
            w.downcast_ref::<gtk::MenuButton>()
                .and_then(|b| b.icon_name())
                .map(|l| l.to_string())
        })
        .unwrap_or_default();
    if !text.is_empty() {
        format!("{kind}[{text}]")
    } else if named != kind && !named.starts_with("Gtk") && !named.is_empty() {
        format!("{kind}[{named}]")
    } else {
        kind
    }
}

/// Every action row (and the other rows) below `w`.
fn collect_rows(w: &gtk::Widget, out: &mut Vec<gtk::Widget>) {
    let mut c = w.first_child();
    while let Some(ch) = c {
        if ch.is::<adw::ActionRow>() {
            out.push(ch.clone());
        }
        // a popover of a row is not a row of the group
        if !ch.is::<gtk::Popover>() {
            collect_rows(&ch, out);
        }
        c = ch.next_sibling();
    }
}

/// The rows of an expander, without its own header row.
fn group_rows(e: &adw::ExpanderRow) -> Vec<gtk::Widget> {
    let mut rows = Vec::new();
    collect_rows(e.upcast_ref(), &mut rows);
    let title = e.title();
    rows.into_iter()
        .filter(|r| {
            r.downcast_ref::<adw::ActionRow>()
                .is_none_or(|a| a.title() != title)
        })
        .collect()
}

/// The first widget below `w` whose name the test accepts.
pub(super) fn find_named_by(w: &gtk::Widget, test: &dyn Fn(&str) -> bool) -> Option<gtk::Widget> {
    if test(&w.widget_name()) {
        return Some(w.clone());
    }
    let mut c = w.first_child();
    while let Some(ch) = c {
        if let Some(f) = find_named_by(&ch, test) {
            return Some(f);
        }
        c = ch.next_sibling();
    }
    None
}

fn all_named(w: &gtk::Widget, name: &str, out: &mut Vec<gtk::Widget>) {
    if w.widget_name() == name {
        out.push(w.clone());
    }
    let mut c = w.first_child();
    while let Some(ch) = c {
        all_named(&ch, name, out);
        c = ch.next_sibling();
    }
}

/// The first controller of type `T` of a widget.
fn controller<T: IsA<gtk::EventController> + IsA<glib::Object>>(w: &gtk::Widget) -> Option<T> {
    let list = w.observe_controllers();
    (0..list.n_items()).find_map(|i| list.item(i).and_downcast::<T>())
}

impl LibraryView {
    /// The card widget of the n-th book shown (the grid must have it bound).
    fn card_at(&self, pos: usize) -> Option<gtk::Widget> {
        let id = *self.visible_ids().get(pos)?;
        find_named(self.grid.view.upcast_ref(), &id.to_string())
    }

    /// `library:` commands of this wave; true when `c` was one of them.
    pub(super) fn dev_wave9(&mut self, c: &str, sender: &ComponentSender<Self>) -> bool {
        match c {
            "tablecols" => {
                // the frame clock of a headless display is slow: do now what its tick does
                (self.w.apply_cols)();
                let shown: Vec<&str> = self
                    .w
                    .table_cols
                    .iter()
                    .filter(|(_, col)| col.is_visible())
                    .map(|(n, _)| *n)
                    .collect();
                let adj = self.w.table_scroll.hadjustment();
                eprintln!(
                    "tablecols: width={} visible={} hscroll={} custom={}",
                    self.w.table_scroll.width(),
                    shown.join(","),
                    adj.upper() > adj.page_size() + 1.0,
                    !self.shared.cfg.borrow().table_columns.is_empty()
                );
            }
            // a click on a column header: `sortcol=Author(s):desc`
            other if other.starts_with("sortcol=") => {
                let arg = &other[8..];
                let (name, dir) = arg.rsplit_once(':').unwrap_or((arg, "asc"));
                if let Some((_, col)) = self.w.table_cols.iter().find(|(n, _)| *n == name) {
                    self.table.view.sort_by_column(
                        Some(col),
                        if dir == "desc" {
                            gtk::SortType::Descending
                        } else {
                            gtk::SortType::Ascending
                        },
                    );
                }
            }
            "sortstate" => {
                let (key, asc) = {
                    let c = self.shared.cfg.borrow();
                    (c.sort_key, c.sort_ascending)
                };
                let sorter = self
                    .table
                    .view
                    .sorter()
                    .and_downcast::<gtk::ColumnViewSorter>();
                let arrow = sorter
                    .as_ref()
                    .and_then(|s| s.primary_sort_column())
                    .and_then(|c| {
                        self.w
                            .table_cols
                            .iter()
                            .find(|(_, tc)| *tc == c)
                            .map(|(n, _)| *n)
                    })
                    .unwrap_or("none");
                let dir =
                    sorter.is_some_and(|s| s.primary_sort_order() == gtk::SortType::Ascending);
                let first: Vec<String> = (0..self.table.len().min(3))
                    .filter_map(|i| self.table.get(i).map(|r| r.borrow().title.clone()))
                    .collect();
                eprintln!(
                    "sortstate: key={} asc={asc} dropdown={} arrow={arrow} arrow_asc={dir} order=\"{}\" first=[{}]",
                    key.label(),
                    SortKey::ALL
                        .get(self.w.sort_dd.selected() as usize)
                        .map_or("?", |k| k.label()),
                    self.w
                        .order_btn
                        .icon_name()
                        .unwrap_or_default(),
                    first.join("|")
                );
            }
            // the check box of a card shows on hover: `hover=N` enters, `unhover=N` leaves
            other if other.starts_with("hover=") || other.starts_with("unhover=") => {
                let enter = other.starts_with("hover=");
                let n: usize = other
                    .split_once('=')
                    .and_then(|x| x.1.parse().ok())
                    .unwrap_or(0);
                if let Some(card) = self.card_at(n) {
                    if let Some(m) = controller::<gtk::EventControllerMotion>(&card) {
                        if enter {
                            m.emit_by_name::<()>("enter", &[&0f64, &0f64]);
                        } else {
                            m.emit_by_name::<()>("leave", &[]);
                        }
                    }
                }
            }
            "checks" => {
                let mut all = Vec::new();
                all_named(self.grid.view.upcast_ref(), "select-check", &mut all);
                let visible = all.iter().filter(|c| c.is_visible()).count();
                let active = all
                    .iter()
                    .filter(|c| {
                        c.downcast_ref::<gtk::CheckButton>()
                            .is_some_and(|c| c.is_active())
                    })
                    .count();
                eprintln!(
                    "checks: bound={} visible={visible} active={active} selecting={} selected={}",
                    all.len(),
                    SELECTING.with(|s| s.get()),
                    self.selected.len()
                );
            }
            // the check box of the n-th card clicked
            other if other.starts_with("check=") => {
                let n: usize = other[6..].parse().unwrap_or(0);
                if let Some(card) = self.card_at(n) {
                    if let Some(c) = find_named(&card, "select-check")
                        .and_then(|c| c.downcast::<gtk::CheckButton>().ok())
                    {
                        c.set_visible(true);
                        c.set_active(!c.is_active());
                    }
                }
            }
            // a long press on the n-th card
            other if other.starts_with("longpress=") => {
                let n: usize = other[10..].parse().unwrap_or(0);
                if let Some(card) = self.card_at(n) {
                    if let Some(g) = controller::<gtk::GestureLongPress>(&card) {
                        g.emit_by_name::<()>("pressed", &[&1f64, &1f64]);
                    }
                }
            }
            // a plain click on the n-th card (its capture gesture)
            other if other.starts_with("cardclick=") => {
                let n: usize = other[10..].parse().unwrap_or(0);
                if let Some(card) = self.card_at(n) {
                    let list = card.observe_controllers();
                    let clicks: Vec<gtk::GestureClick> = (0..list.n_items())
                        .filter_map(|i| list.item(i).and_downcast::<gtk::GestureClick>())
                        .filter(|g| g.button() == 1)
                        .collect();
                    for g in clicks {
                        g.emit_by_name::<()>("pressed", &[&1i32, &4f64, &4f64]);
                    }
                }
            }
            other if other.starts_with("carddesc=") => {
                let n: u32 = other[9..].parse().unwrap_or(0);
                if let Some(item) = self.grid.get(n) {
                    let it = item.borrow();
                    eprintln!(
                        "carddesc: pos={n} title={:?} desc={:?}",
                        it.title, it.description
                    );
                }
            }
            // a drag of files entering or leaving the books
            "drag-enter" | "drag-leave" => self.w.drop_hint.set_visible(c == "drag-enter"),
            "drophint" => {
                eprintln!(
                    "drophint: visible={} targetable={} text={:?}",
                    self.w.drop_hint.is_visible(),
                    self.w.drop_hint.can_target(),
                    tr("Drop to Add")
                );
            }
            // the Add from the Web dialog: is it open, and what is in its box
            "webdialog" => {
                let win = window_of(&self.root).and_downcast::<adw::ApplicationWindow>();
                let url = win
                    .and_then(|w| w.visible_dialog())
                    .and_then(|d| find_named(d.upcast_ref(), "add-web-url"))
                    .and_then(|e| e.downcast::<gtk::Entry>().ok())
                    .map(|e| e.text().to_string());
                eprintln!(
                    "webdialog: open={} url={:?}",
                    url.is_some(),
                    url.unwrap_or_default()
                );
            }
            other if other.starts_with("paste=") => {
                sender.input(Input::PasteUrl(other[6..].to_string()))
            }
            // the Import Highlights dialog: its text and its response labels
            "import-dialog" => {
                if let Some(win) = window_of(&self.root) {
                    super::super::annotations::start_import(
                        &win,
                        &self.shared,
                        Rc::new(|_| {}),
                        Rc::new(|| {}),
                    );
                    let d = win
                        .downcast_ref::<adw::ApplicationWindow>()
                        .and_then(|w| w.visible_dialog())
                        .and_then(|d| d.downcast::<adw::AlertDialog>().ok());
                    if let Some(d) = d {
                        eprintln!(
                            "import_dialog: body={:?} files={:?} folder={:?} default={:?}",
                            d.body(),
                            d.response_label("files"),
                            d.response_label("folder"),
                            d.default_response()
                        );
                    } else {
                        eprintln!("import_dialog: none");
                    }
                }
            }
            // the dialog that is open: its heading
            "dialog" => {
                let d = window_of(&self.root)
                    .and_then(|w| w.downcast::<adw::ApplicationWindow>().ok())
                    .and_then(|w| w.visible_dialog());
                match d {
                    Some(d) => eprintln!(
                        "dialog: name={:?} heading={:?}",
                        d.widget_name(),
                        d.downcast_ref::<adw::AlertDialog>()
                            .map(|a| a.heading().unwrap_or_default().to_string())
                            .unwrap_or_else(|| d.title().to_string())
                    ),
                    None => eprintln!("dialog: none"),
                }
            }
            "dialog-close" => {
                if let Some(d) = window_of(&self.root)
                    .and_then(|w| w.downcast::<adw::ApplicationWindow>().ok())
                    .and_then(|w| w.visible_dialog())
                {
                    d.force_close();
                }
            }
            "add-web-open" => sender.input(Input::AddWebDialog),
            "copy-citation" => self.do_act(Act::CopyCitation),
            // a sidebar group: its rows, Show All, and whether it is open
            other if other.starts_with("group=") => {
                let name = &other[6..];
                match find_named(self.w.tags.upcast_ref(), &format!("group-{name}"))
                    .and_then(|w| w.downcast::<adw::ExpanderRow>().ok())
                {
                    Some(e) => {
                        let rows = group_rows(&e);
                        let show_all = rows.iter().any(|r| r.widget_name() == "show-all");
                        let titles: Vec<String> = rows
                            .iter()
                            .filter(|r| r.widget_name() != "show-all")
                            .filter_map(|r| {
                                r.downcast_ref::<adw::ActionRow>()
                                    .map(|a| a.title().to_string())
                            })
                            .collect();
                        eprintln!(
                            "group: {name} rows={} show_all={show_all} expanded={} first={:?}",
                            titles.len(),
                            e.is_expanded(),
                            titles.first()
                        );
                    }
                    None => eprintln!("group: {name} missing"),
                }
            }
            other if other.starts_with("group-expand=") => {
                if let Some(e) =
                    find_named(self.w.tags.upcast_ref(), &format!("group-{}", &other[13..]))
                        .and_then(|w| w.downcast::<adw::ExpanderRow>().ok())
                {
                    e.set_expanded(true);
                }
            }
            // Show All of a group: open its popover and count what it lists
            other if other.starts_with("group-showall=") => {
                if let Some(row) =
                    find_named(self.w.tags.upcast_ref(), &format!("group-{}", &other[14..]))
                        .and_then(|e| find_named(&e, "show-all"))
                        .and_then(|w| w.downcast::<adw::ActionRow>().ok())
                {
                    row.emit_by_name::<()>("activated", &[]);
                    let mut c = row.first_child();
                    let mut listed = 0;
                    while let Some(ch) = c {
                        if let Some(p) = ch.downcast_ref::<gtk::Popover>() {
                            let mut rows = Vec::new();
                            if let Some(child) = p.child() {
                                collect_rows(&child, &mut rows);
                            }
                            listed = rows.len();
                            p.popdown();
                        }
                        c = ch.next_sibling();
                    }
                    eprintln!("showall: listed={listed}");
                }
            }
            "saved-state" => {
                match find_named(self.w.tags.upcast_ref(), "saved-searches")
                    .and_then(|w| w.downcast::<adw::ExpanderRow>().ok())
                {
                    Some(e) => {
                        let rows = group_rows(&e);
                        let titles: Vec<String> = rows
                            .iter()
                            .filter_map(|r| {
                                r.downcast_ref::<adw::ActionRow>()
                                    .map(|a| a.title().to_string())
                            })
                            .collect();
                        eprintln!("saved: rows={} titles={}", titles.len(), titles.join("|"));
                    }
                    None => eprintln!("saved: missing"),
                }
            }
            // the search tips of the search bar
            "tips-open" => {
                if let Some(b) = find_named(self.w.toolbar.upcast_ref(), "search-tips")
                    .and_then(|w| w.downcast::<gtk::MenuButton>().ok())
                {
                    b.popup();
                }
            }
            "tips-close" => {
                if let Some(b) = find_named(self.w.toolbar.upcast_ref(), "search-tips")
                    .and_then(|w| w.downcast::<gtk::MenuButton>().ok())
                {
                    b.popdown();
                }
            }
            "tips" => {
                let save = find_named(self.w.toolbar.upcast_ref(), "save-search")
                    .and_then(|w| w.downcast::<gtk::Button>().ok());
                let name = find_named(self.w.toolbar.upcast_ref(), "save-search-name")
                    .and_then(|w| w.downcast::<gtk::Entry>().ok());
                eprintln!(
                    "tips: button={} save_sensitive={:?} name={:?}",
                    find_named(self.w.toolbar.upcast_ref(), "search-tips").is_some(),
                    save.map(|b| b.is_sensitive()),
                    name.map(|n| n.text().to_string())
                );
            }
            other if other.starts_with("tips-save=") => {
                sender.input(Input::SaveSearch(other[10..].to_string()))
            }
            // the empty Shelves group: its "New Shelf" row
            "shelf-empty" => {
                let row = find_named(self.w.tags.upcast_ref(), "new-shelf-row")
                    .and_then(|w| w.downcast::<adw::ActionRow>().ok());
                eprintln!(
                    "shelf_empty: row={} activatable={} title={:?}",
                    row.is_some(),
                    row.as_ref().is_some_and(|r| r.is_activatable()),
                    row.as_ref().map(|r| r.title().to_string())
                );
            }
            "shelf-empty-click" => {
                if let Some(r) = find_named(self.w.tags.upcast_ref(), "new-shelf-row") {
                    r.emit_by_name::<()>("activated", &[]);
                }
            }
            // keyboard: where Tab goes from the start (the first 40 stops)
            "focus-order" => {
                if let Some(win) = window_of(&self.root) {
                    gtk::prelude::GtkWindowExt::set_focus(&win, gtk::Widget::NONE);
                    let mut seen: Vec<String> = Vec::new();
                    for _ in 0..40 {
                        if !win.child_focus(gtk::DirectionType::TabForward) {
                            break;
                        }
                        let Some(f) = gtk::prelude::GtkWindowExt::focus(&win) else {
                            break;
                        };
                        let d = describe_focus(&f);
                        if seen.last() != Some(&d) {
                            seen.push(d);
                        }
                    }
                    eprintln!("focus_order: {}", seen.join(" > "));
                }
            }
            "focus-grid" => {
                self.grid.view.grab_focus();
            }
            "focus-where" => {
                let f = window_of(&self.root).and_then(|w| gtk::prelude::GtkWindowExt::focus(&w));
                let in_grid = f.as_ref().is_some_and(|f: &gtk::Widget| {
                    *f == *self.grid.view.upcast_ref::<gtk::Widget>()
                        || f.is_ancestor(&self.grid.view)
                });
                eprintln!(
                    "focus_where: in_grid={in_grid} widget={}",
                    f.map(|f| describe_focus(&f)).unwrap_or_default()
                );
            }
            "details-click" => self.w.details_toggle.emit_clicked(),
            "details-state" => eprintln!(
                "details: shown={} toggle={} pref={} toggle_visible={} icon={}",
                self.w.split_details.shows_sidebar(),
                self.w.details_toggle.is_active(),
                self.shared.cfg.borrow().details_pane,
                self.w.details_toggle.is_visible(),
                self.w.details_toggle.icon_name().unwrap_or_default()
            ),
            "gridwidth" => eprintln!(
                "gridwidth: max={} clamp={} window={} scroll={} margin={}",
                GRID_CAP.with(|g| g.get()),
                self.grid.view.width(),
                self.root.width(),
                self.w.grid_scroll.width(),
                self.grid.view.margin_start()
            ),
            c if c.starts_with("gridcap=") => {
                if let Ok(n) = c[8..].parse::<i32>() {
                    GRID_CAP.with(|g| g.set(n));
                }
            }
            "gridbg" => {
                let bg = |w: &gtk::Widget| w.css_classes().join(".");
                eprintln!(
                    "gridbg: scroll=[{}] view=[{}] page=[{}]",
                    bg(self.w.grid_scroll.upcast_ref()),
                    bg(self.grid.view.upcast_ref()),
                    bg(self.w.grid_page.upcast_ref())
                );
            }
            c if c.starts_with("click-empty=") => {
                // a click at `margin` (left of the capped grid), `below` (under the last row) or `inside`
                // (on the first card): is it the book list's handler that gets it, and does it clear?
                // points are in the coordinates of the whole grid page, as a click on the window would be
                let sc = &self.w.grid_scroll;
                let page = &self.w.grid_page;
                let (pw, ph) = (page.width() as f64, page.height() as f64);
                let (px, py) = match &c[12..] {
                    "margin" => (3.0, ph / 2.0),
                    "below" => (pw / 2.0, ph - 30.0),
                    _ => (pw / 2.0, ph / 2.0 - sc.height() as f64 / 2.0 + 40.0),
                };
                let hit = page.pick(px, py, gtk::PickFlags::DEFAULT);
                let reaches = hit
                    .as_ref()
                    .is_some_and(|h| h == sc.upcast_ref::<gtk::Widget>() || h.is_ancestor(sc));
                let (x, y) = page
                    .compute_point(sc, &gtk::graphene::Point::new(px as f32, py as f32))
                    .map(|p| (p.x() as f64, p.y() as f64))
                    .unwrap_or((px, py));
                let clears = reaches && !hits_item(sc, x, y);
                eprintln!(
                    "click-empty: {} reaches_list={reaches} clears={clears}",
                    &c[12..]
                );
                if clears {
                    sender.input(Input::ClearSelection);
                }
            }
            _ => return self.dev_wave9_more(c, sender),
        }
        true
    }
}

/// "Show All (N)" under a sidebar group that was cut to its top entries: a popover with every entry
/// and a search field.
pub(super) fn show_all_row(
    items: Vec<(String, usize)>,
    hidden: usize,
    go: Rc<dyn Fn(String)>,
    search_for: Rc<dyn Fn(&str) -> String>,
) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(trf("Show All ({} more)", &[&hidden]))
        .activatable(true)
        .build();
    row.set_widget_name("show-all");
    row.add_prefix(&gtk::Image::from_icon_name("view-more-horizontal-symbolic"));
    let pop = gtk::Popover::new();
    pop.set_parent(&row);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    body.set_margin_all(8);
    let entry = gtk::SearchEntry::new();
    entry.set_placeholder_text(Some(&tr("Filter")));
    crate::ui::audit::label(&entry, &tr("Filter"));
    body.append(&entry);
    let list = gtk::ListBox::new();
    list.add_css_class("navigation-sidebar");
    list.set_selection_mode(gtk::SelectionMode::None);
    for (name, count) in &items {
        let r = adw::ActionRow::builder()
            .title(glib::markup_escape_text(name))
            .activatable(true)
            .build();
        r.add_suffix(&count_label(*count));
        let (go, q, pop) = (go.clone(), search_for(name), pop.clone());
        r.connect_activated(move |_| {
            pop.popdown();
            go(q.clone());
        });
        list.append(&r);
    }
    {
        let e2 = entry.clone();
        list.set_filter_func(move |r| {
            let q = e2.text().to_lowercase();
            q.is_empty()
                || r.downcast_ref::<adw::ActionRow>()
                    .is_some_and(|a| a.title().to_lowercase().contains(&q))
        });
        let list = list.clone();
        entry.connect_search_changed(move |_| list.invalidate_filter());
    }
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    scroll.set_min_content_width(260);
    scroll.set_min_content_height(120);
    scroll.set_max_content_height(320);
    scroll.set_propagate_natural_height(true);
    scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    body.append(&scroll);
    pop.set_child(Some(&body));
    pop.connect_closed(|p| {
        // the filter starts empty next time
        if let Some(e) = p
            .child()
            .and_then(|b| b.first_child())
            .and_downcast::<gtk::SearchEntry>()
        {
            e.set_text("");
        }
    });
    {
        let pop = pop.clone();
        row.connect_destroy(move |_| pop.unparent());
    }
    row.connect_activated(move |_| pop.popup());
    row
}

/// The `?` of the search bar: the search syntax in short, and a way to keep the search.
pub(super) fn search_tips(search: &gtk::SearchEntry, save: Rc<dyn Fn(String)>) -> gtk::MenuButton {
    let btn = gtk::MenuButton::new();
    btn.set_icon_name("dialog-question-symbolic");
    btn.set_widget_name("search-tips");
    btn.set_tooltip_text(Some(&tr("Search Tips")));
    btn.add_css_class("flat");
    crate::ui::audit::label(&btn, &tr("Search Tips"));
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.set_margin_all(12);
    body.set_width_request(300);
    let head = gtk::Label::new(Some(&tr("Search Tips")));
    head.add_css_class("heading");
    head.set_xalign(0.0);
    body.append(&head);
    for line in [
        tr("Look in one field: title:, author:, tag:, series:, publisher:, language:, rating:, status:, format: or identifier:"),
        tr("Use = for an exact match, >= or <= to compare, - to leave a word out and OR to combine"),
        tr("text:word searches the contents of the books"),
    ] {
        let l = gtk::Label::new(Some(&line));
        l.set_wrap(true);
        l.set_xalign(0.0);
        l.set_max_width_chars(38);
        l.add_css_class("dim-label");
        body.append(&l);
    }
    body.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let name = gtk::Entry::new();
    name.set_placeholder_text(Some(&tr("Name of the saved search")));
    name.set_widget_name("save-search-name");
    crate::ui::audit::label(&name, &tr("Name of the saved search"));
    let button = gtk::Button::with_label(&tr("Save This Search"));
    button.set_widget_name("save-search");
    button.add_css_class("suggested-action");
    body.append(&name);
    body.append(&button);
    let pop = gtk::Popover::new();
    pop.set_child(Some(&body));
    btn.set_popover(Some(&pop));
    {
        // nothing to save while the search is empty; the name starts as its first words
        let (search, name, button) = (search.clone(), name.clone(), button.clone());
        pop.connect_show(move |_| {
            let q = search.text().to_string();
            button.set_sensitive(!q.trim().is_empty());
            name.set_text(&nav::default_search_name(&q));
        });
    }
    {
        let (name2, pop2) = (name.clone(), pop.clone());
        let save2 = save.clone();
        button.connect_clicked(move |_| {
            let n = name2.text().trim().to_string();
            if !n.is_empty() {
                pop2.popdown();
                save2(n);
            }
        });
        name.connect_activate(move |n| {
            let t = n.text().trim().to_string();
            if !t.is_empty() {
                pop.popdown();
                save(t);
            }
        });
    }
    btn
}

impl LibraryView {
    fn dev_wave9_more(&mut self, c: &str, sender: &ComponentSender<Self>) -> bool {
        self.dev_t3(c, sender)
    }
}

/// A dialog listing every entry of a sidebar group (authors, tags, shelves...) with a search
/// field: how a group that is hidden from the sidebar stays reachable (command palette).
pub(super) fn browse_dialog(
    parent: &impl IsA<gtk::Widget>,
    title: &str,
    items: Vec<(String, usize)>,
    go: Rc<dyn Fn(String)>,
    search_for: Rc<dyn Fn(&str) -> String>,
) {
    let dialog = crate::ui::dialog_toast::new_dialog();
    dialog.set_title(title);
    dialog.set_content_width(400);
    dialog.set_content_height(520);
    let entry = gtk::SearchEntry::new();
    entry.set_placeholder_text(Some(&tr("Filter")));
    crate::ui::audit::label(&entry, &tr("Filter"));
    entry.set_margin_start(12);
    entry.set_margin_end(12);
    entry.set_margin_bottom(6);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    list.set_margin_start(12);
    list.set_margin_end(12);
    list.set_margin_bottom(12);
    for (name, count) in &items {
        let r = adw::ActionRow::builder()
            .title(glib::markup_escape_text(name))
            .activatable(true)
            .build();
        r.add_suffix(&count_label(*count));
        let (go, q, dialog) = (go.clone(), search_for(name), dialog.clone());
        r.connect_activated(move |_| {
            dialog.close();
            go(q.clone());
        });
        list.append(&r);
    }
    {
        let e2 = entry.clone();
        list.set_filter_func(move |r| {
            let q = e2.text().to_lowercase();
            q.is_empty()
                || r.downcast_ref::<adw::ActionRow>()
                    .is_some_and(|a| a.title().to_lowercase().contains(&q))
        });
        let list = list.clone();
        entry.connect_search_changed(move |_| list.invalidate_filter());
    }
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&list));
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&entry);
    body.append(&scroll);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&body));
    dialog.set_child(Some(&view));
    dialog.set_focus(Some(&entry));
    dialog.present(Some(parent));
}
