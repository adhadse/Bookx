//! Preferences → Library → *Sidebar*: show, hide and reorder the entries of the library sidebar
//! (Home, Online Catalogues, Notes & Highlights, Shelves, Series, Authors, Tags...). The model (order,
//! hidden entries, what is kept per library) is `crate::library_nav::SidebarLayout`; a change is saved
//! at once and the open library window follows through `settings_bus::sidebar()`. An entry that is
//! hidden stays reachable from the command palette.

use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use super::SharedRef;
use super::preferences::register_group;
use crate::i18n::{tr, trf};
use crate::library_nav::{SidebarItem, SidebarLayout};

struct Ui {
    shared: SharedRef,
    group: adw::PreferencesGroup,
    rows: RefCell<Vec<gtk::Widget>>,
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<Ui>>> = const { RefCell::new(None) };
}

/// Adds the group to the Library page of Preferences.
pub fn register() {
    register_group("library", 60, |ctx| group(&ctx.shared));
}

fn library_key(shared: &SharedRef) -> String {
    shared.lib.borrow().root.to_string_lossy().to_string()
}

fn group(shared: &SharedRef) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&tr("Sidebar"));
    group.set_description(Some(&tr(
        "Choose what the library sidebar shows and in which order. Hidden entries stay available in the command palette.",
    )));
    let tree = adw::SwitchRow::builder()
        .title(tr("Show tags as a tree"))
        .subtitle(tr("A tag written like Fiction.Fantasy sits under Fiction; choosing Fiction shows every book below it"))
        .build();
    tree.set_widget_name("tag-tree-switch");
    tree.set_active(shared.cfg.borrow().tag_tree);
    {
        let shared = shared.clone();
        tree.connect_active_notify(move |s| {
            {
                let mut c = shared.cfg.borrow_mut();
                c.tag_tree = s.is_active();
                c.save();
            }
            crate::settings_bus::sidebar().notify();
        });
    }
    group.add(&tree);
    let ui = Rc::new(Ui {
        shared: shared.clone(),
        group: group.clone(),
        rows: RefCell::new(Vec::new()),
    });
    ui.fill();
    CURRENT.with(|c| *c.borrow_mut() = Some(ui));
    group
}

impl Ui {
    fn layout(&self) -> SidebarLayout {
        self.shared
            .cfg
            .borrow()
            .sidebar_layout(&library_key(&self.shared))
    }

    /// Keeps a new layout, tells the library window and redraws the rows.
    fn change(self: &Rc<Self>, layout: SidebarLayout) {
        {
            let mut c = self.shared.cfg.borrow_mut();
            c.set_sidebar_layout(&library_key(&self.shared), layout);
            c.save();
        }
        crate::settings_bus::sidebar().notify();
        self.fill();
    }

    fn fill(self: &Rc<Self>) {
        for r in self.rows.borrow_mut().drain(..) {
            self.group.remove(&r);
        }
        let layout = self.layout();
        let mut items = layout.ordered();
        items.push(SidebarItem::Health);
        let last_movable = items.len().saturating_sub(2);
        for (n, item) in items.iter().copied().enumerate() {
            let title = tr(item.label());
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&title))
                .build();
            row.set_widget_name(&format!("sidebar-{}", item.key()));
            if item.can_move() {
                let up = move_button("go-up-symbolic", &trf("Move {} Up", &[&title]));
                up.set_sensitive(n > 0);
                let down = move_button("go-down-symbolic", &trf("Move {} Down", &[&title]));
                down.set_sensitive(n < last_movable);
                for (b, delta, name) in [(&up, -1, "up"), (&down, 1, "down")] {
                    b.set_widget_name(&format!("sidebar-{name}-{}", item.key()));
                    let ui = self.clone();
                    b.connect_clicked(move |_| ui.move_item(item, delta));
                }
                row.add_suffix(&up);
                row.add_suffix(&down);
            }
            let sw = gtk::Switch::new();
            sw.set_valign(gtk::Align::Center);
            sw.set_active(!layout.is_hidden(item));
            sw.set_sensitive(item.can_hide());
            sw.set_widget_name(&format!("sidebar-switch-{}", item.key()));
            super::audit::label(&sw, &trf("Show {} in the Sidebar", &[&title]));
            {
                let ui = self.clone();
                sw.connect_active_notify(move |s| ui.set_shown(item, s.is_active()));
            }
            row.add_suffix(&sw);
            row.set_activatable_widget(Some(&sw));
            self.group.add(&row);
            self.rows.borrow_mut().push(row.upcast());
        }
        let reset = adw::ActionRow::builder()
            .title(tr("Reset the Sidebar"))
            .subtitle(tr("Show every entry in the usual order"))
            .build();
        reset.set_widget_name("sidebar-reset-row");
        let b = gtk::Button::with_label(&tr("Reset to Default"));
        b.set_valign(gtk::Align::Center);
        b.set_widget_name("sidebar-reset");
        b.set_sensitive(!layout.is_default());
        {
            let ui = self.clone();
            b.connect_clicked(move |_| {
                let mut l = ui.layout();
                l.reset();
                ui.change(l);
            });
        }
        reset.add_suffix(&b);
        self.group.add(&reset);
        self.rows.borrow_mut().push(reset.upcast());
    }

    fn set_shown(self: &Rc<Self>, item: SidebarItem, shown: bool) {
        let mut l = self.layout();
        if l.is_hidden(item) != shown {
            return;
        }
        l.set_hidden(item, !shown);
        self.change(l);
    }

    fn move_item(self: &Rc<Self>, item: SidebarItem, delta: i32) {
        let mut l = self.layout();
        if l.move_item(item, delta) {
            self.change(l);
            // the row moved: the keyboard focus follows its button
            let first = if delta < 0 { "up" } else { "down" };
            let second = if delta < 0 { "down" } else { "up" };
            for what in [first, second] {
                if let Some(b) = self.find(&format!("sidebar-{what}-{}", item.key())) {
                    if b.is_sensitive() {
                        b.grab_focus();
                        break;
                    }
                }
            }
        }
    }

    fn find(&self, name: &str) -> Option<gtk::Widget> {
        fn walk(w: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
            if w.widget_name() == name {
                return Some(w.clone());
            }
            let mut c = w.first_child();
            while let Some(x) = c {
                if let Some(f) = walk(&x, name) {
                    return Some(f);
                }
                c = x.next_sibling();
            }
            None
        }
        walk(self.group.upcast_ref(), name)
    }
}

fn move_button(icon: &str, label: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.add_css_class("flat");
    b.set_valign(gtk::Align::Center);
    b.set_tooltip_text(Some(label));
    b.update_property(&[gtk::accessible::Property::Label(label)]);
    b
}

/// Test hook (`library:sidebar-config=...`, with Preferences open on the Library page): `dump`,
/// `hide=KEY`, `show=KEY`, `up=KEY`, `down=KEY`, `reset`; it presses the real buttons and switches.
pub fn dev(cmd: &str) {
    let Some(ui) = CURRENT.with(|c| c.borrow().clone()) else {
        eprintln!("sidebar_config: not open");
        return;
    };
    match cmd.split_once('=') {
        None if cmd == "dump" => {
            let l = ui.layout();
            let rows: Vec<String> = ui
                .rows
                .borrow()
                .iter()
                .map(|r| r.widget_name().trim_start_matches("sidebar-").to_string())
                .filter(|n| n != "reset-row")
                .collect();
            eprintln!(
                "sidebar_config: rows={} hidden={}",
                rows.join(","),
                l.hidden.join(",")
            );
        }
        None if cmd == "reset" => {
            if let Some(b) = ui.find("sidebar-reset").and_downcast::<gtk::Button>() {
                b.emit_clicked();
            }
        }
        Some((what @ ("up" | "down"), k)) => {
            match ui
                .find(&format!("sidebar-{what}-{k}"))
                .and_downcast::<gtk::Button>()
            {
                Some(b) => b.emit_clicked(),
                None => eprintln!("sidebar_config: no {what} button for {k}"),
            }
        }
        Some((what @ ("hide" | "show"), k)) => {
            match ui
                .find(&format!("sidebar-switch-{k}"))
                .and_downcast::<gtk::Switch>()
            {
                Some(s) => s.set_active(what == "show"),
                None => eprintln!("sidebar_config: no switch for {k}"),
            }
        }
        _ => eprintln!("sidebar_config: unknown command {cmd}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_has_a_label_without_an_ellipsis() {
        for i in SidebarItem::ALL {
            assert!(!i.label().is_empty());
            assert!(!i.label().ends_with('…') && !i.label().ends_with("..."));
        }
    }
}
