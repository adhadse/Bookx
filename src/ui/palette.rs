//! The command palette dialog (Ctrl+Shift+P or Ctrl+K): a search entry above a list of commands,
//! books and settings. The ranking is in [`crate::palette`].

use crate::i18n::tr;
use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gdk, glib};

use crate::palette::{rank, Entry};

const MAX_RESULTS: usize = 40;

struct State {
    entries: Vec<Entry>,
    shown: RefCell<Vec<usize>>,
    entry: gtk::SearchEntry,
    list: gtk::ListBox,
    dialog: adw::Dialog,
    on_pick: Rc<dyn Fn(Entry)>,
}

thread_local! {
    static OPEN: RefCell<Option<Rc<State>>> = const { RefCell::new(None) };
    /// Full names (`lib.add-books`) of the actions the windows registered, for the registry check.
    static GROUPS: RefCell<std::collections::HashSet<String>> = RefCell::default();
}

/// Remembers the actions of a window's group (the UI scenario `palette_registry` checks the palette's commands against them).
pub fn register_group(prefix: &str, group: &relm4::gtk::gio::SimpleActionGroup) {
    use relm4::gtk::gio::prelude::ActionGroupExt;
    GROUPS.with(|g| {
        g.borrow_mut()
            .extend(group.list_actions().iter().map(|a| format!("{prefix}.{a}")))
    });
}

/// The commands whose action does not exist (empty when the registry is sound).
pub fn missing_actions(scope: crate::palette::Scope) -> Vec<String> {
    use relm4::gtk::gio::prelude::ActionGroupExt;
    let app = relm4::main_adw_application();
    crate::palette::commands(scope)
        .into_iter()
        .filter_map(|e| match e.target {
            crate::palette::Target::Action(n) => Some(n),
            _ => None,
        })
        .filter(|n| match n.strip_prefix("app.") {
            Some(short) => !app.has_action(short),
            None => !GROUPS.with(|g| g.borrow().contains(n)),
        })
        .collect()
}

impl State {
    fn refill(&self) {
        let q = self.entry.text().to_string();
        let idx = rank(&self.entries, &q, MAX_RESULTS);
        while let Some(r) = self.list.first_child() {
            self.list.remove(&r);
        }
        for i in &idx {
            self.list.append(&row(&self.entries[*i]));
        }
        if let Some(first) = self.list.row_at_index(0) {
            self.list.select_row(Some(&first));
        } else {
            self.list.append(&empty_row());
        }
        *self.shown.borrow_mut() = idx;
    }

    fn move_selection(&self, delta: i32) {
        let n = self.shown.borrow().len() as i32;
        if n == 0 {
            return;
        }
        let cur = self.list.selected_row().map_or(0, |r| r.index());
        let next = (cur + delta).clamp(0, n - 1);
        if let Some(r) = self.list.row_at_index(next) {
            self.list.select_row(Some(&r));
            // keep the row in view without moving the keyboard focus out of the entry
            r.grab_focus();
            self.entry.grab_focus();
            self.entry.select_region(-1, -1);
        }
    }

    fn pick(&self) {
        let Some(row) = self.list.selected_row() else {
            return;
        };
        let Some(i) = self.shown.borrow().get(row.index() as usize).copied() else {
            return;
        };
        let entry = self.entries[i].clone();
        self.dialog.close();
        let pick = self.on_pick.clone();
        // after the dialog is gone, so that the action sees the window and not the dialog
        glib::idle_add_local_once(move || pick(entry));
    }
}

fn empty_row() -> gtk::ListBoxRow {
    let l = gtk::Label::new(Some(&tr("Nothing matches")));
    l.add_css_class("dim-label");
    l.set_margin_top(18);
    l.set_margin_bottom(18);
    let r = gtk::ListBoxRow::new();
    r.set_child(Some(&l));
    r.set_selectable(false);
    r.set_activatable(false);
    r
}

fn row(e: &Entry) -> gtk::ListBoxRow {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    b.set_margin_top(6);
    b.set_margin_bottom(6);
    b.set_margin_start(10);
    b.set_margin_end(10);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_hexpand(true);
    let title = gtk::Label::new(Some(&e.title));
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&title);
    if !e.detail.is_empty() {
        let d = gtk::Label::new(Some(&e.detail));
        d.set_xalign(0.0);
        d.add_css_class("dim-label");
        d.add_css_class("caption");
        d.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&d);
    }
    b.append(&text);
    let kind = gtk::Label::new(Some(&e.kind.display()));
    kind.add_css_class("dim-label");
    kind.add_css_class("caption");
    b.append(&kind);
    if !e.shortcut.is_empty() {
        let k = gtk::Label::new(Some(&e.shortcut));
        k.add_css_class("keycap");
        k.add_css_class("caption");
        b.append(&k);
    }
    let r = gtk::ListBoxRow::new();
    r.set_child(Some(&b));
    let said = if e.shortcut.is_empty() {
        format!("{}, {}", e.title, e.kind.display())
    } else {
        format!("{}, {}, shortcut {}", e.title, e.kind.display(), e.shortcut)
    };
    r.update_property(&[gtk::accessible::Property::Label(&said)]);
    r
}

/// Opens the palette over `parent`. `on_pick` runs after the dialog has closed.
pub fn show(parent: &impl IsA<gtk::Widget>, entries: Vec<Entry>, on_pick: Rc<dyn Fn(Entry)>) {
    if OPEN.with(|o| o.borrow().is_some()) {
        return;
    }
    let entry = gtk::SearchEntry::new();
    entry.set_placeholder_text(Some(&tr("Type a command, book or setting")));
    entry.set_hexpand(true);
    entry.update_property(&[gtk::accessible::Property::Label(&tr(
        "Command palette search",
    ))]);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.add_css_class("navigation-sidebar");
    list.set_activate_on_single_click(true);
    list.update_property(&[gtk::accessible::Property::Label(&tr(
        "Command palette results",
    ))]);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    scroll.set_vexpand(true);
    scroll.set_min_content_height(300);
    scroll.set_hscrollbar_policy(gtk::PolicyType::Never);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.append(&entry);
    content.append(&scroll);

    let dialog = adw::Dialog::builder()
        .title(tr("Command Palette"))
        .content_width(600)
        .content_height(440)
        .follows_content_size(false)
        .child(&content)
        .build();
    dialog.set_focus(Some(&entry));

    let st = Rc::new(State {
        entries,
        shown: RefCell::new(Vec::new()),
        entry: entry.clone(),
        list: list.clone(),
        dialog: dialog.clone(),
        on_pick,
    });
    st.refill();

    {
        let st = st.clone();
        entry.connect_search_changed(move |_| st.refill());
    }
    {
        let st = st.clone();
        entry.connect_activate(move |_| st.pick());
    }
    {
        let st = st.clone();
        list.connect_row_activated(move |_, _| st.pick());
    }
    {
        let st = st.clone();
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, key, _, mods| {
            if key == gdk::Key::Escape {
                // the search entry would eat the first Escape (it clears itself)
                st.dialog.close();
                return glib::Propagation::Stop;
            }
            let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
            let name = key.name().map(|n| n.to_string()).unwrap_or_default();
            match crate::palette::list_move(&name, ctrl) {
                Some(d) => {
                    st.move_selection(d);
                    glib::Propagation::Stop
                }
                None => glib::Propagation::Proceed,
            }
        });
        dialog.add_controller(keys);
    }
    OPEN.with(|o| *o.borrow_mut() = Some(st));
    dialog.connect_closed(|_| OPEN.with(|o| *o.borrow_mut() = None));
    dialog.present(Some(parent));
    entry.grab_focus();
}

// ---- test hooks (BOOKX_DEV_SCRIPT, "palette:*") ------------------------------------------------

pub fn dev(cmd: &str) -> Option<String> {
    OPEN.with(|o| {
        let st = o.borrow().clone();
        match (cmd, st) {
            ("dump", Some(st)) => {
                let titles: Vec<String> = st
                    .shown
                    .borrow()
                    .iter()
                    .map(|i| format!("{}|{}", st.entries[*i].kind.display(), st.entries[*i].title))
                    .collect();
                let sel = st.list.selected_row().map_or(-1, |r| r.index());
                Some(format!(
                    "query={:?} selected={} results={}",
                    st.entry.text().as_str(),
                    sel,
                    titles.join(" ; ")
                ))
            }
            ("dump", None) => Some("closed".into()),
            ("registry", _) => Some(format!(
                "library_missing={:?} reader_missing={:?}",
                missing_actions(crate::palette::Scope::Library),
                missing_actions(crate::palette::Scope::Reader)
            )),
            ("down", Some(st)) => {
                st.move_selection(1);
                None
            }
            ("up", Some(st)) => {
                st.move_selection(-1);
                None
            }
            ("pick", Some(st)) => {
                st.pick();
                None
            }
            ("close", Some(st)) => {
                st.dialog.close();
                None
            }
            (c, Some(st)) if c.starts_with("type=") => {
                st.entry.set_text(&c[5..]);
                None
            }
            _ => None,
        }
    })
}
