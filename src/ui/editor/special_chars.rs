//! Insert special character (CE-300..CE-302): a window with every Unicode character by block, a search by name
//! or code point, favourites (right-click to add, remove or move) and recently used characters. A click inserts
//! the character at the caret of the code view, or into the find / replace field when that has the focus, and
//! closes the window; Ctrl-click inserts and keeps it open. Typing a hex code then Alt+X converts it, and (when
//! the setting is on) typing `;` after `&name` turns the entity into its character.

use super::*;
use crate::editor_chars::CharPrefs;
use crate::i18n::tr;
use crate::i18n::trf;
use bookx_editing::charinfo;

#[derive(Debug, Clone)]
pub enum Msg {
    Open,
    /// Show the current group or search again.
    Reload,
    Close,
    /// Insert a character; `true` keeps the window open.
    Insert(char, bool),
    ToggleFavourite(char),
    Move(char, i32),
    ConvertEntities(bool),
}

/// Results of one search shown at most.
const SEARCH_LIMIT: usize = 300;

struct Win {
    window: adw::Window,
    cats: gtk::ListBox,
    store: gtk::StringList,
    selection: gtk::SingleSelection,
    title: gtk::Label,
    info: gtk::Label,
    search: gtk::SearchEntry,
    /// Which category is shown (0 favourites, 1 recent, then the blocks); `usize::MAX` for search results.
    category: Rc<Cell<usize>>,
}

pub struct State {
    win: Option<Win>,
    prefs: Rc<RefCell<CharPrefs>>,
    /// Whether `;` converts a typed `&name` (read by the key handler of the code view).
    pub entities: Rc<Cell<bool>>,
}

impl State {
    pub fn new(entities: bool) -> State {
        State {
            win: None,
            prefs: Rc::new(RefCell::new(CharPrefs::load())),
            entities: Rc::new(Cell::new(entities)),
        }
    }
}

/// `&`, `<` and `>` must stay escaped in markup.
pub fn markup_text(c: char) -> String {
    match c {
        '&' => "&amp;".into(),
        '<' => "&lt;".into(),
        '>' => "&gt;".into(),
        c => c.to_string(),
    }
}

/// Alt+X on the buffer: converts the hex code before the caret. Returns whether it did.
pub fn apply_alt_x(buf: &gtk::TextBuffer) -> bool {
    let cur = buf.iter_at_mark(&buf.get_insert());
    let before = text_before(buf, &cur);
    let Some((n, c)) = charinfo::alt_x(&before) else {
        return false;
    };
    replace_before(buf, n, &markup_text(c));
    true
}

/// Typing `;`: converts `&name` before the caret into the character. Returns whether it did (the `;` is then
/// not inserted).
pub fn apply_entity(buf: &gtk::TextBuffer) -> bool {
    let cur = buf.iter_at_mark(&buf.get_insert());
    let before = text_before(buf, &cur);
    let Some((n, c)) = charinfo::entity_before_semicolon(&before) else {
        return false;
    };
    replace_before(buf, n, &markup_text(c));
    true
}

/// Up to 40 characters before `at` on its line.
fn text_before(buf: &gtk::TextBuffer, at: &gtk::TextIter) -> String {
    let mut start = *at;
    start.set_line_offset(0);
    let mut from = *at;
    from.backward_chars(40);
    if from.offset() > start.offset() {
        start = from;
    }
    buf.text(&start, at, false).to_string()
}

fn replace_before(buf: &gtk::TextBuffer, chars: usize, with: &str) {
    let mut end = buf.iter_at_mark(&buf.get_insert());
    let mut start = end;
    start.backward_chars(chars as i32);
    buf.begin_user_action();
    buf.delete(&mut start, &mut end);
    buf.insert(&mut start, with);
    buf.end_user_action();
}

/// The key handler of the code view for Alt+X and the entity semicolon.
pub fn install_typing(view: &sv::View, entities: Rc<Cell<bool>>) {
    let kc = gtk::EventControllerKey::new();
    kc.set_propagation_phase(gtk::PropagationPhase::Capture);
    let v = view.clone();
    kc.connect_key_pressed(move |_, key, _, state| {
        let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
        let alt = state.contains(gdk::ModifierType::ALT_MASK);
        let buf = v.buffer();
        if !v.is_editable() {
            return glib::Propagation::Proceed;
        }
        if alt && !ctrl && matches!(key, gdk::Key::x | gdk::Key::X) && apply_alt_x(&buf) {
            return glib::Propagation::Stop;
        }
        if !alt && !ctrl && key == gdk::Key::semicolon && entities.get() && apply_entity(&buf) {
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    view.add_controller(kc);
}

fn category_chars(prefs: &CharPrefs, cat: usize) -> Vec<char> {
    match cat {
        0 => prefs.favourites.clone(),
        1 => prefs.recent.clone(),
        n => charinfo::chars_in_block(n - 2),
    }
}

fn describe(c: char) -> String {
    match charinfo::name(c) {
        Some(n) => format!("{} · {n}", charinfo::code_label(c)),
        None => charinfo::code_label(c),
    }
}

impl Editor {
    pub(super) fn chars(&mut self, msg: Msg) {
        match msg {
            Msg::Open => self.chars_open(),
            Msg::Reload => self.chars_reload(),
            Msg::Close => {
                if let Some(w) = &self.chars.win {
                    w.window.close();
                }
            }
            Msg::Insert(c, keep) => self.chars_insert(c, keep),
            Msg::ToggleFavourite(c) => {
                let now = {
                    let mut p = self.chars.prefs.borrow_mut();
                    let n = p.toggle_favourite(c);
                    p.save();
                    n
                };
                self.toast(&if now {
                    format!("{c} added to the favourites")
                } else {
                    format!("{c} removed from the favourites")
                });
                self.chars_reload();
            }
            Msg::Move(c, d) => {
                {
                    let mut p = self.chars.prefs.borrow_mut();
                    if p.move_favourite(c, d) {
                        p.save();
                    }
                }
                self.chars_reload();
            }
            Msg::ConvertEntities(on) => {
                self.chars.entities.set(on);
                self.prefs.convert_entities = on;
                self.prefs.save();
                self.toast(
                    &(if on {
                        tr("Typing ; after &name now turns the entity into its character")
                    } else {
                        tr("Entities are left as typed")
                    }),
                );
            }
        }
    }

    fn chars_open(&mut self) {
        if self.chars.win.is_none() {
            let w = self.chars_build();
            self.chars.win = Some(w);
        }
        let w = self.chars.win.as_ref().unwrap();
        w.window.present();
        w.search.grab_focus();
        self.chars_reload();
    }

    /// Shows the current category (or search) again.
    fn chars_reload(&self) {
        let Some(w) = &self.chars.win else { return };
        let cat = w.category.get();
        let chars: Vec<char> = if cat == usize::MAX {
            charinfo::search(&w.search.text(), SEARCH_LIMIT)
        } else {
            category_chars(&self.chars.prefs.borrow(), cat)
        };
        let items: Vec<String> = chars.iter().map(|c| c.to_string()).collect();
        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
        w.store.splice(0, w.store.n_items(), &refs);
        w.selection.set_selected(gtk::INVALID_LIST_POSITION);
        w.title.set_label(&if cat == usize::MAX {
            format!("{} results for “{}”", chars.len(), w.search.text())
        } else {
            match cat {
                0 => trf(
                    "Favourites ({}). Right-click a character to move or remove it",
                    &[&chars.len()],
                ),
                1 => trf("Recently used ({})", &[&chars.len()]),
                n => format!("{} ({})", charinfo::BLOCKS[n - 2].2, chars.len()),
            }
        });
        w.info.set_label(
            &(if chars.is_empty() && cat == 0 {
                tr("No favourites yet: right-click a character to add it")
            } else {
                tr("Click a character to insert it. Ctrl+click keeps this window open")
            }),
        );
    }

    fn chars_build(&self) -> Win {
        let window = adw::Window::new();
        window.set_title(Some(&tr("Insert Special Character")));
        window.set_default_size(700, 540);
        window.set_transient_for(Some(&self.w.win));
        window.set_destroy_with_parent(true);
        window.set_hide_on_close(true);

        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some(&tr("Search by name or code, like “acute” or U+00E9")));
        search.set_hexpand(true);
        search.update_property(&[gtk::accessible::Property::Label(&tr(
            "Search characters by name or code point",
        ))]);
        search.set_margin_start(8);
        search.set_margin_end(8);
        search.set_margin_top(6);

        let cats = gtk::ListBox::new();
        cats.add_css_class("navigation-sidebar");
        cats.update_property(&[gtk::accessible::Property::Label(&tr("Character groups"))]);
        let names: Vec<String> = [tr("★ Favourites"), tr("Recently used")]
            .into_iter()
            .chain(charinfo::BLOCKS.iter().map(|b| b.2.to_string()))
            .collect();
        for n in &names {
            let l = gtk::Label::new(Some(n));
            l.set_xalign(0.0);
            l.set_margin_top(4);
            l.set_margin_bottom(4);
            l.set_margin_start(6);
            cats.append(&l);
        }
        let cat_scroll = gtk::ScrolledWindow::new();
        cat_scroll.set_child(Some(&cats));
        cat_scroll.set_width_request(210);
        cat_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);

        let store = gtk::StringList::new(&[]);
        let selection = gtk::SingleSelection::new(Some(store.clone()));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let l = gtk::Label::new(None);
            l.add_css_class("title-3");
            l.add_css_class("char-cell");
            l.set_width_chars(2);
            l.set_margin_top(6);
            l.set_margin_bottom(6);
            l.set_margin_start(8);
            l.set_margin_end(8);
            if let Some(i) = item.downcast_ref::<gtk::ListItem>() {
                i.set_child(Some(&l));
            }
        });
        factory.connect_bind(|_, item| {
            let Some(i) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let (Some(o), Some(l)) = (
                i.item().and_downcast::<gtk::StringObject>(),
                i.child().and_downcast::<gtk::Label>(),
            ) else {
                return;
            };
            let s = o.string();
            l.set_text(&s);
            if let Some(c) = s.chars().next() {
                let d = describe(c);
                l.set_tooltip_text(Some(&d));
                l.update_property(&[gtk::accessible::Property::Label(
                    &charinfo::name(c)
                        .unwrap_or_else(|| charinfo::code_label(c))
                        .to_lowercase(),
                )]);
            }
        });
        let grid = gtk::GridView::new(Some(selection.clone()), Some(factory));
        grid.set_single_click_activate(true);
        grid.set_min_columns(4);
        grid.set_max_columns(24);
        grid.update_property(&[gtk::accessible::Property::Label(&tr("Characters"))]);
        let grid_scroll = gtk::ScrolledWindow::new();
        grid_scroll.set_child(Some(&grid));
        grid_scroll.set_vexpand(true);

        let title = gtk::Label::new(None);
        title.set_xalign(0.0);
        title.add_css_class("heading");
        title.set_margin_start(8);
        title.set_margin_top(6);
        let info = gtk::Label::new(None);
        info.set_xalign(0.0);
        info.add_css_class("dim-label");
        info.set_margin_start(8);
        info.set_margin_bottom(6);
        info.set_wrap(true);

        // below 720sp the group list becomes a drop-down above the grid
        let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let cat_drop = gtk::DropDown::from_strings(&name_refs);
        cat_drop.set_visible(false);
        cat_drop.set_widget_name("chars-drop");
        cat_scroll.set_widget_name("chars-list");
        cat_drop.set_margin_start(8);
        cat_drop.set_margin_end(8);
        cat_drop.set_margin_top(6);
        crate::ui::audit::label(&cat_drop, &tr("Character group"));
        let right = gtk::Box::new(gtk::Orientation::Vertical, 4);
        right.append(&cat_drop);
        right.append(&search);
        right.append(&title);
        right.append(&grid_scroll);
        right.append(&info);
        right.set_hexpand(true);
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let cat_sep = gtk::Separator::new(gtk::Orientation::Vertical);
        body.append(&cat_scroll);
        body.append(&cat_sep);
        body.append(&right);
        let tb = adw::ToolbarView::new();
        tb.add_top_bar(&adw::HeaderBar::new());
        tb.set_content(Some(&body));
        window.set_content(Some(&tb));
        {
            let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
                adw::BreakpointConditionLengthType::MaxWidth,
                720.0,
                adw::LengthUnit::Sp,
            ));
            bp.add_setter(&cat_scroll, "visible", Some(&false.to_value()));
            bp.add_setter(&cat_sep, "visible", Some(&false.to_value()));
            bp.add_setter(&cat_drop, "visible", Some(&true.to_value()));
            window.add_breakpoint(bp);
            let cats2 = cats.clone();
            cat_drop.connect_selected_notify(move |d| {
                if let Some(r) = cats2.row_at_index(d.selected() as i32) {
                    if !r.is_selected() {
                        cats2.select_row(Some(&r));
                    }
                }
            });
            let d2 = cat_drop.clone();
            cats.connect_row_selected(move |_, row| {
                if let Some(r) = row {
                    if d2.selected() != r.index() as u32 {
                        d2.set_selected(r.index() as u32);
                    }
                }
            });
        }

        let category = Rc::new(Cell::new(if self.chars.prefs.borrow().recent.is_empty() {
            2
        } else {
            1
        }));
        if let Some(r) = cats.row_at_index(category.get() as i32) {
            cats.select_row(Some(&r));
        }
        let tx = self.sender.input_sender().clone();
        // the modifier keys at the moment of the click, for Ctrl-click
        let ctrl = Rc::new(Cell::new(false));
        {
            let ctrl = ctrl.clone();
            let click = gtk::GestureClick::new();
            click.set_button(1);
            click.set_propagation_phase(gtk::PropagationPhase::Capture);
            click.connect_pressed(move |g, _, _, _| {
                ctrl.set(
                    g.current_event_state()
                        .contains(gdk::ModifierType::CONTROL_MASK),
                );
            });
            grid.add_controller(click);
        }
        {
            let (tx, store) = (tx.clone(), store.clone());
            let ctrl = ctrl.clone();
            grid.connect_activate(move |_, pos| {
                if let Some(c) = store.string(pos).and_then(|s| s.chars().next()) {
                    tx.emit(Input::Chars(Msg::Insert(c, ctrl.get())));
                }
            });
            let info2 = info.clone();
            selection.connect_selected_notify(move |s| {
                if let Some(c) = s
                    .selected_item()
                    .and_downcast::<gtk::StringObject>()
                    .and_then(|o| o.string().chars().next())
                {
                    info2.set_label(&describe(c));
                }
            });
        }
        {
            // right-click: favourites menu for the character under the pointer
            let right_click = gtk::GestureClick::new();
            right_click.set_button(3);
            let (tx, grid2, prefs, cat) = (
                tx.clone(),
                grid.clone(),
                self.chars.prefs.clone(),
                category.clone(),
            );
            right_click.connect_pressed(move |g, _, x, y| {
                let Some(c) = char_under(&grid2, x, y) else {
                    return;
                };
                g.set_state(gtk::EventSequenceState::Claimed);
                let pop = gtk::Popover::new();
                pop.set_parent(&grid2);
                pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
                let fav = prefs.borrow().is_favourite(c);
                let mut entries: Vec<(String, Msg)> = vec![(
                    if fav {
                        "Remove from Favourites"
                    } else {
                        "Add to Favourites"
                    }
                    .to_string(),
                    Msg::ToggleFavourite(c),
                )];
                if fav && cat.get() == 0 {
                    entries.push(("Move Earlier".into(), Msg::Move(c, -1)));
                    entries.push(("Move Later".into(), Msg::Move(c, 1)));
                }
                for (label, msg) in entries {
                    let b = gtk::Button::with_label(&label);
                    b.add_css_class("flat");
                    let (tx, pop2) = (tx.clone(), pop.clone());
                    b.connect_clicked(move |_| {
                        pop2.popdown();
                        tx.emit(Input::Chars(msg.clone()));
                    });
                    col.append(&b);
                }
                pop.set_child(Some(&col));
                pop.connect_closed(|p| p.unparent());
                pop.popup();
            });
            grid.add_controller(right_click);
        }
        {
            let (tx1, category1, search2) = (tx.clone(), category.clone(), search.clone());
            cats.connect_row_selected(move |_, row| {
                if let Some(r) = row {
                    category1.set(r.index() as usize);
                    if !search2.text().is_empty() {
                        search2.set_text("");
                    }
                    tx1.emit(Input::Chars(Msg::Reload));
                }
            });
            let (tx, category, cats2) = (tx.clone(), category.clone(), cats.clone());
            search.connect_search_changed(move |e| {
                if e.text().is_empty() {
                    if category.get() == usize::MAX {
                        category.set(2);
                        if let Some(r) = cats2.row_at_index(2) {
                            cats2.select_row(Some(&r));
                        }
                    }
                } else {
                    category.set(usize::MAX);
                    cats2.unselect_all();
                }
                tx.emit(Input::Chars(Msg::Reload));
            });
        }
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
        Win {
            window,
            cats,
            store,
            selection,
            title,
            info,
            search,
            category,
        }
    }

    /// Puts `c` at the caret of the code view, or into the find or replace field when that has the focus.
    fn chars_insert(&mut self, c: char, keep: bool) {
        let focus = gtk::prelude::GtkWindowExt::focus(&self.w.win);
        let in_field = focus
            .as_ref()
            .and_then(|f| f.dynamic_cast_ref::<gtk::Editable>().cloned())
            .filter(|e: &gtk::Editable| e.is_editable());
        match in_field {
            Some(e) => {
                let mut pos = e.position();
                e.insert_text(&c.to_string(), &mut pos);
                e.set_position(pos);
            }
            None => {
                if self.w.center.visible_child_name().as_deref() != Some("code")
                    || !self.w.view.is_editable()
                {
                    self.toast(&tr("Open a text file to insert a character"));
                    return;
                }
                self.w.view.buffer().delete_selection(true, true);
                self.w.view.buffer().insert_at_cursor(&markup_text(c));
            }
        }
        {
            let mut p = self.chars.prefs.borrow_mut();
            p.used(c);
            p.save();
        }
        if keep {
            self.chars_reload();
        } else if let Some(w) = &self.chars.win {
            w.window.close();
            self.w.win.present();
        }
    }

    /// Dev hooks (`chars:…`), see build-aux/ui-tests.sh.
    pub(super) fn dev_chars(&mut self, c: &str) -> bool {
        let Some(c) = c.strip_prefix("chars:") else {
            return false;
        };
        if c == "open" {
            self.chars(Msg::Open);
        } else if let Some(r) = c.strip_prefix("size=") {
            // `chars:size=<w>x<h>` resizes the window (the breakpoint scenario)
            self.chars(Msg::Open);
            if let (Some(w), Some((a, b))) = (&self.chars.win, r.split_once('x')) {
                // a mapped window ignores a new default size (Broadway never resizes it), so hide
                // it first and let `present` map it again at the new size
                w.window.set_visible(false);
                w.window
                    .set_default_size(a.parse().unwrap_or(700), b.parse().unwrap_or(540));
                w.window.present();
            }
        } else if c == "layout" {
            // `editor_chars_layout: width=W list=yes|no drop=yes|no`
            if let Some(w) = &self.chars.win {
                let root = w.window.clone().upcast::<gtk::Widget>();
                let shown =
                    |n: &str| super::dev9::find_named(&root, n).is_some_and(|x| x.is_visible());
                eprintln!(
                    "editor_chars_layout: width={} list={} drop={}",
                    w.window.width(),
                    if shown("chars-list") { "yes" } else { "no" },
                    if shown("chars-drop") { "yes" } else { "no" },
                );
            }
        } else if c == "close" {
            self.chars(Msg::Close);
        } else if let Some(q) = c.strip_prefix("search=") {
            self.chars(Msg::Open);
            if let Some(w) = &self.chars.win {
                w.search.set_text(q);
                w.search.emit_by_name::<()>("search-changed", &[]);
            }
        } else if let Some(n) = c.strip_prefix("category=") {
            self.chars(Msg::Open);
            if let Some(w) = &self.chars.win {
                let idx = charinfo::BLOCKS
                    .iter()
                    .position(|b| b.2 == n)
                    .map(|i| i + 2)
                    .or(match n {
                        "favourites" => Some(0),
                        "recent" => Some(1),
                        _ => None,
                    });
                if let Some(r) = idx.and_then(|i| w.cats.row_at_index(i as i32)) {
                    w.cats.select_row(Some(&r));
                }
            }
        } else if let Some(s) = c.strip_prefix("insert=") {
            // `insert=é` (a click) or `insert=é|keep` (Ctrl-click)
            let (ch, keep) = s.split_once('|').map_or((s, false), |(a, _)| (a, true));
            if let Some(ch) = ch.chars().next() {
                self.chars(Msg::Insert(ch, keep));
            }
        } else if let Some(s) = c.strip_prefix("fav=") {
            if let Some(ch) = s.chars().next() {
                self.chars(Msg::ToggleFavourite(ch));
            }
        } else if let Some(s) = c.strip_prefix("move=") {
            // `move=é|-1`
            if let Some((ch, d)) = s.split_once('|') {
                if let (Some(ch), Ok(d)) = (ch.chars().next(), d.parse()) {
                    self.chars(Msg::Move(ch, d));
                }
            }
        } else if c == "focus-find" {
            self.w.find_rev.set_reveal_child(true);
            self.w.find_entry.grab_focus();
        } else if c == "field" {
            eprintln!("editor_chars_field: {}", self.w.find_entry.text());
        } else if c == "altx" {
            let ok = apply_alt_x(self.w.view.buffer().upcast_ref());
            eprintln!("editor_chars_altx: {ok}");
        } else if let Some(v) = c.strip_prefix("entities=") {
            self.chars(Msg::ConvertEntities(v == "on"));
        } else if c == "semicolon" {
            // what typing `;` does: converts when the setting is on, else inserts it
            let buf = self.w.view.buffer();
            let done = self.chars.entities.get() && apply_entity(buf.upcast_ref());
            if !done {
                buf.insert_at_cursor(";");
            }
            eprintln!("editor_chars_semicolon: converted={done}");
        } else if c == "dump" {
            let (cat, shown, title, info) = self
                .chars
                .win
                .as_ref()
                .map(|w| {
                    let first: Vec<String> = (0..w.store.n_items().min(16))
                        .filter_map(|i| w.store.string(i))
                        .map(|s| s.to_string())
                        .collect();
                    (
                        w.category.get().min(999),
                        format!("{}:{}", w.store.n_items(), first.join("")),
                        w.title.label().to_string(),
                        w.info.label().to_string(),
                    )
                })
                .unwrap_or_default();
            let p = self.chars.prefs.borrow();
            eprintln!(
                "editor_chars: open={} cat={} shown={} title={} favourites={} recent={} entities={} info={}",
                self.chars
                    .win
                    .as_ref()
                    .is_some_and(|w| w.window.is_visible()),
                cat,
                shown,
                title,
                p.favourites.iter().collect::<String>(),
                p.recent.iter().collect::<String>(),
                self.chars.entities.get(),
                info
            );
        }
        true
    }
}

/// The character of the grid cell under a point of the grid view.
fn char_under(grid: &gtk::GridView, x: f64, y: f64) -> Option<char> {
    let mut w = grid.pick(x, y, gtk::PickFlags::DEFAULT);
    while let Some(cur) = w {
        if cur.has_css_class("char-cell") {
            return cur
                .downcast_ref::<gtk::Label>()
                .and_then(|l| l.text().chars().next());
        }
        if cur == *grid.upcast_ref::<gtk::Widget>() {
            return None;
        }
        w = cur.parent();
    }
    None
}
