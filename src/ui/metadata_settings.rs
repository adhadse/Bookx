//! The *Metadata lookup* group on the Library page of Preferences: fetch metadata for new books
//! after an import (off by default) and the on-disk cache of source answers.

use crate::i18n::{tr, trf};
use relm4::adw::{self, prelude::*};
use relm4::gtk;

use super::preferences::{register_group, PrefCtx};
use crate::metadata_fetch as mf;

/// Adds the group; called once from the registry of Preferences.
pub fn register() {
    register_group("library", 40, group);
}

fn group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Metadata lookup"));
    g.set_description(Some(
        "Open Library and Google Books are asked only when you ask, or after an import if you switch this on.",
    ));
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
    clear.connect_clicked({
        let cache = cache.clone();
        move |_| {
            let n = mf::clear_cache();
            cache.set_subtitle(&trf("Cleared {} saved answers.", &[&n]));
        }
    });
    cache.add_suffix(&clear);
    g.add(&cache);
    g
}
