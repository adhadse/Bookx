//! Reader additions that live outside `reader.rs`: the footnote popup, and its Preferences row.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gdk};

use super::preferences::{register_group, PrefCtx};
use crate::i18n::tr;

thread_local! {
    static CLICK: Cell<(f64, f64)> = const { Cell::new((0.0, 0.0)) };
    static LAST: RefCell<Option<(String, gtk::Popover)>> = const { RefCell::new(None) };
}

pub fn register() {
    register_group("reading", 50, group);
}

fn group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Links"));
    let row = adw::SwitchRow::new();
    row.set_title(&tr("Show footnotes in a popup"));
    row.set_subtitle(&tr(
        "Following a link to a footnote shows the note where you are, with a button to go to it.",
    ));
    row.set_active(ctx.shared.cfg.borrow().footnote_popups);
    let shared = ctx.shared.clone();
    row.connect_active_notify(move |r| {
        shared.cfg.borrow_mut().footnote_popups = r.is_active();
        shared.cfg.borrow().save();
    });
    g.add(&row);
    g
}

/// The buttons of the selection popover must not take the keyboard focus when they are clicked: the popover stays open
/// (it is not modal) and the focus would sit in it, where Ctrl+F, the arrows and the other page keys do not reach the
/// reader. Tab still reaches them.
pub fn keep_focus_on_page(root: &impl IsA<gtk::Widget>) {
    let mut c = root.as_ref().first_child();
    while let Some(w) = c {
        if let Some(b) = w.downcast_ref::<gtk::Button>() {
            b.set_focus_on_click(false);
        }
        keep_focus_on_page(&w);
        c = w.next_sibling();
    }
}

/// Remembers where the pointer last went down on the page, to point the popup at it.
pub fn track_clicks(area: &impl IsA<gtk::Widget>) {
    let g = gtk::GestureClick::new();
    g.set_propagation_phase(gtk::PropagationPhase::Capture);
    g.connect_pressed(|_, _, x, y| CLICK.with(|c| c.set((x, y))));
    area.add_controller(g);
}

/// Shows `text` in a popover at the last click; `go` runs when the user wants the note itself.
pub fn show_footnote(area: &impl IsA<gtk::Widget>, text: &str, go: impl Fn() + 'static) {
    // a note popup that is replaced by another one: the new one opens a moment after the old one closed, not in
    // the same breath (Wayland surface reuse, protocol error 71; see reader.rs `repopup`)
    let had_old = LAST.with(|l| {
        l.borrow_mut()
            .take()
            .map(|(_, old)| {
                old.popdown();
                old.unparent();
            })
            .is_some()
    });
    let pop = gtk::Popover::new();
    pop.set_parent(area);
    super::card_drag::avoid(&pop);
    let (x, y) = CLICK.with(|c| c.get());
    pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    pop.add_css_class("footnote-popup");
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.set_margin_top(8);
    body.set_margin_bottom(8);
    body.set_margin_start(8);
    body.set_margin_end(8);
    let label = gtk::Label::new(Some(text));
    label.set_wrap(true);
    label.set_max_width_chars(48);
    label.set_xalign(0.0);
    label.set_selectable(true);
    body.append(&label);
    let button = gtk::Button::with_label(&tr("Go to Note"));
    button.set_halign(gtk::Align::End);
    button.add_css_class("flat");
    let go = Rc::new(go);
    {
        let (pop, go) = (pop.clone(), go.clone());
        button.connect_clicked(move |_| {
            pop.popdown();
            go();
        });
    }
    body.append(&button);
    pop.set_child(Some(&body));
    if had_old {
        let p = pop.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(80), move || p.popup());
    } else {
        pop.popup();
    }
    LAST.with(|l| *l.borrow_mut() = Some((text.to_string(), pop)));
}

/// Forgets the last popup (a link that is not a note was followed).
pub fn clear() {
    LAST.with(|l| {
        if let Some((_, old)) = l.borrow_mut().take() {
            old.popdown();
            old.unparent();
        }
    });
}

/// Test hook: the state of the last footnote popup.
pub fn dev_dump() -> String {
    LAST.with(|l| match l.borrow().as_ref() {
        Some((t, p)) => format!("visible={} text={t}", p.is_visible()),
        None => "none".into(),
    })
}
