//! Preferences → Reading: the switch for marking your vocabulary terms in the text, and "My Vocabulary",
//! your own definitions as a dictionary that comes first among the dictionaries (it cannot be removed;
//! it can be exported).

use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk;

use super::preferences::{register_group, PrefCtx};
use crate::i18n::{tr, trf};
use crate::settings_bus::{Bus, Subscription};

thread_local! {
    static BUS: Rc<Bus> = Rc::new(Bus::default());
    static SWITCH: std::cell::RefCell<Option<adw::SwitchRow>> = const { std::cell::RefCell::new(None) };
}

/// Readers hear here that the vocabulary or the marking setting changed.
pub fn subscribe(f: impl Fn() + 'static) -> Subscription {
    BUS.with(|b| b.subscribe(f))
}

/// Tells the open readers to look at the vocabulary again.
pub fn changed() {
    BUS.with(|b| b.notify());
}

/// Dev hook (`prefs:vocab-marks=on|off`): the switch of the open Preferences.
pub fn dev(arg: &str) {
    if let Some(v) = arg.strip_prefix("marks=") {
        SWITCH.with(|s| {
            if let Some(r) = s.borrow().as_ref() {
                r.set_active(v != "off");
            }
        });
    }
}

pub fn register() {
    register_group("reading", 56, group);
}

fn group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Vocabulary"));
    let row = adw::SwitchRow::new();
    row.set_title(&tr("Mark my vocabulary terms in the text"));
    row.set_subtitle(&tr(
        "Words and phrases you saved with your own definition get a dotted underline; tap one to see your definition.",
    ));
    row.set_active(ctx.shared.cfg.borrow().mark_vocabulary);
    let shared = ctx.shared.clone();
    row.connect_active_notify(move |r| {
        {
            let mut c = shared.cfg.borrow_mut();
            c.mark_vocabulary = r.is_active();
            c.save();
        }
        changed();
    });
    g.add(&row);
    SWITCH.with(|s| *s.borrow_mut() = Some(row));
    g
}

/// The first row of the dictionaries: "My Vocabulary".
pub fn my_vocabulary_row(ctx: &PrefCtx) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&tr("My Vocabulary"));
    let n = ctx.shared.vocab.borrow().store.own_entries().len();
    row.set_subtitle(&trf(
        "Your own definitions · {} terms · works offline, always first",
        &[&n],
    ));
    let lock = gtk::Image::from_icon_name("changes-prevent-symbolic");
    lock.set_tooltip_text(Some(&tr("Built in, it cannot be removed")));
    lock.update_property(&[gtk::accessible::Property::Label(&tr(
        "Built in, it cannot be removed",
    ))]);
    row.add_prefix(&lock);
    let menu = gtk::MenuButton::new();
    menu.set_label(&tr("Export"));
    menu.add_css_class("flat");
    menu.set_valign(gtk::Align::Center);
    super::audit::label(&menu, &tr("Export My Vocabulary"));
    let pop = gtk::Popover::new();
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for (label, kind, name) in super::vocab_list::export_kinds() {
        if kind == "anki" {
            continue;
        }
        let b = gtk::Button::with_label(&label);
        b.add_css_class("flat");
        let (shared, parent, row2, pop2) = (
            ctx.shared.clone(),
            ctx.parent.clone(),
            row.clone(),
            pop.clone(),
        );
        b.connect_clicked(move |_| {
            pop2.popdown();
            let row3 = row2.clone();
            super::vocab_list::export_choose(
                &shared,
                &parent,
                kind,
                name,
                Rc::new(move |m| row3.set_subtitle(&m)),
            );
        });
        bx.append(&b);
    }
    pop.set_child(Some(&bx));
    menu.set_popover(Some(&pop));
    row.add_suffix(&menu);
    let open = gtk::Button::with_label(&tr("Open"));
    open.add_css_class("flat");
    open.set_valign(gtk::Align::Center);
    super::audit::label(&open, &tr("Open My Vocabulary"));
    open.connect_clicked(|b| {
        let _ = b.activate_action("app.vocab", None);
    });
    row.add_suffix(&open);
    row
}
