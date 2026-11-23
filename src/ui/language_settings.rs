//! The *Language* group on the General page of Preferences.

use relm4::adw::{self, prelude::*};
use relm4::gtk;

use super::preferences::{PrefCtx, register_group};
use crate::i18n::{self, tr};

thread_local! {
    static LISTENERS: std::cell::RefCell<Vec<Box<dyn Fn()>>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Runs `f` whenever the interface language is changed while the app runs (the library window
/// re-translates its title and subtitle).
pub fn on_change(f: impl Fn() + 'static) {
    LISTENERS.with(|l| l.borrow_mut().push(Box::new(f)));
}

fn changed() {
    LISTENERS.with(|l| l.borrow().iter().for_each(|f| f()));
}

pub fn register() {
    register_group("general", 30, group);
}

/// Mirrors the interface for right-to-left languages (windows created after this call).
pub fn apply_direction() {
    gtk::Widget::set_default_direction(if i18n::current_is_rtl() {
        gtk::TextDirection::Rtl
    } else {
        gtk::TextDirection::Ltr
    });
}

/// Entries of the picker: the system language first, then every language we have.
pub fn choices() -> Vec<(String, String)> {
    let mut v = vec![(String::new(), tr("System language"))];
    v.extend(i18n::available());
    v
}

fn group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Language"));
    let choices = choices();
    let names: Vec<&str> = choices.iter().map(|(_, n)| n.as_str()).collect();
    let row = adw::ComboRow::new();
    row.set_title(&tr("Interface language"));
    row.set_subtitle(&tr(
        "Open windows keep their language. Restart Bookx to change everything.",
    ));
    row.set_model(Some(&gtk::StringList::new(&names)));
    let current = ctx.shared.cfg.borrow().language.clone();
    row.set_selected(choices.iter().position(|(c, _)| *c == current).unwrap_or(0) as u32);
    let shared = ctx.shared.clone();
    row.connect_selected_notify(move |r| {
        let Some((code, _)) = choices.get(r.selected() as usize) else {
            return;
        };
        shared.cfg.borrow_mut().language = code.clone();
        shared.cfg.borrow().save();
        i18n::set_language(code);
        apply_direction();
        changed();
        super::preferences::toast(&tr("Language changed"));
    });
    g.add(&row);
    g
}

/// Test hook (`prefs:language=CODE`): the same as picking it in the row.
pub fn dev_set(shared: &super::SharedRef, code: &str) {
    shared.cfg.borrow_mut().language = code.to_string();
    shared.cfg.borrow().save();
    i18n::set_language(code);
    changed();
    eprintln!("language: {} | {}", code, i18n::current());
}
