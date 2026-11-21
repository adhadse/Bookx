//! The *Metadata lookup* group on the Library page of Preferences: fetch metadata for new books
//! after an import (off by default) and the on-disk cache of source answers.

use crate::i18n::{tr, trf};
use relm4::adw::{self, prelude::*};
use relm4::gtk;

use super::preferences::{register_group, PrefCtx};
use crate::metadata_fetch as mf;

thread_local! {
    static CLEAR: std::cell::RefCell<Option<gtk::Button>> = const { std::cell::RefCell::new(None) };
}

/// The toast after Clear: says how many saved answers went.
fn cleared_text(n: usize) -> String {
    if n == 0 {
        tr("Source cache cleared")
    } else {
        trf("Source cache cleared ({} saved answers)", &[&n])
    }
}

/// Dev hook `prefs:metadata-clear`: presses Clear and prints `prefs_metadata_clear: label=L`.
pub fn dev(arg: &str) {
    if arg == "clear" {
        CLEAR.with(|c| {
            if let Some(b) = c.borrow().as_ref() {
                eprintln!(
                    "prefs_metadata_clear: label={:?}",
                    b.label().unwrap_or_default()
                );
                b.emit_clicked();
            }
        });
    }
}

/// Adds the group; called once from the registry of Preferences.
pub fn register() {
    register_group("library", 40, group);
}

fn group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Metadata lookup"));
    g.set_description(Some(&tr(
        "Open Library and Google Books are asked only when you ask, or after an import if you switch this on.",
    )));
    let on = adw::SwitchRow::new();
    on.set_title(&tr("Fetch metadata for new books"));
    on.set_subtitle(&tr("After an import, fill in the fields a new book has nothing in. Fields that already have a value are never changed."));
    on.set_active(ctx.shared.cfg.borrow().metadata_prefs.fetch_on_import);
    {
        let shared = ctx.shared.clone();
        on.connect_active_notify(move |s| {
            shared.cfg.borrow_mut().metadata_prefs.fetch_on_import = s.is_active();
            shared.cfg.borrow().save();
        });
    }
    g.add(&on);
    let cache = adw::ActionRow::new();
    cache.set_title(&tr("Source cache"));
    cache.set_subtitle(&tr(
        "Answers are kept for a few hours so that asking again does not use the network.",
    ));
    let clear = gtk::Button::with_label(&tr("Clear"));
    clear.set_valign(gtk::Align::Center);
    super::audit::label(&clear, &tr("Clear source cache"));
    clear.connect_clicked(move |_| {
        let n = mf::clear_cache();
        super::preferences::toast(&cleared_text(n));
    });
    CLEAR.with(|c| *c.borrow_mut() = Some(clear.clone()));
    cache.add_suffix(&clear);
    g.add(&cache);
    g
}
