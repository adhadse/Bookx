//! Preferences → Privacy: the offline switch and the list of everything Bookx can connect to
//! (`crate::privacy`). No telemetry, no account, no update check: the list is the whole story.

use std::cell::RefCell;

use relm4::adw::{self, prelude::*};

use super::preferences::{info_row, register_group, register_page, PrefCtx};
use crate::i18n::tr;
use crate::privacy;

thread_local! {
    static SWITCH: RefCell<Option<adw::SwitchRow>> = const { RefCell::new(None) };
    static LEDGER: RefCell<Option<relm4::gtk::glib::WeakRef<adw::PreferencesGroup>>> = const { RefCell::new(None) };
}

/// Dev hook (`prefs:privacy-offline=on|off`): the switch of the open Preferences.
/// `prefs:privacy-dump` prints `prefs_privacy: rows=N focusable=K` for the list of connections.
pub fn dev(arg: &str) {
    if arg == "dump" {
        let (mut rows, mut focusable) = (0, 0);
        if let Some(g) = LEDGER.with(|l| l.borrow().as_ref().and_then(|w| w.upgrade())) {
            count_rows(g.upcast_ref(), &mut rows, &mut focusable);
        }
        eprintln!("prefs_privacy: rows={rows} focusable={focusable}");
        return;
    }
    if let Some(v) = arg.strip_prefix("offline=") {
        SWITCH.with(|s| {
            if let Some(r) = s.borrow().as_ref() {
                r.set_active(v != "off");
            }
        });
    }
}

pub fn register() {
    register_page("privacy", &tr("Privacy"), "security-high-symbolic", 90);
    register_group("privacy", 10, network_group);
    register_group("privacy", 20, ledger_group);
}

fn network_group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Network"));
    g.set_description(Some(&tr(
        "Bookx has no telemetry, no account and no update check. It only connects when you use a feature that needs it.",
    )));
    let off = adw::SwitchRow::new();
    off.set_title(&tr("Offline mode"));
    off.set_subtitle(&tr(
        "Refuse online look-ups and downloads: metadata, covers, Wikipedia, translation, catalogues, web articles, news, dictionaries and voices. Servers you set up yourself (mail, WebDAV, sync) are not affected.",
    ));
    off.set_active(ctx.shared.cfg.borrow().offline);
    {
        let shared = ctx.shared.clone();
        off.connect_active_notify(move |r| {
            let on = r.is_active();
            {
                let mut c = shared.cfg.borrow_mut();
                c.offline = on;
                c.save();
            }
            privacy::apply(on);
            crate::settings_bus::sync().notify();
        });
    }
    g.add(&off);
    SWITCH.with(|s| *s.borrow_mut() = Some(off));
    g
}

fn ledger_group(_: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("What Bookx can connect to"));
    g.set_description(Some(&tr(
        "Only when you use the feature. The same list is on the wiki page Privacy.",
    )));
    for c in privacy::LEDGER.iter() {
        g.add(&info_row(&tr(c.feature), &privacy::describe(c)));
    }
    LEDGER.with(|l| *l.borrow_mut() = Some(g.downgrade()));
    g
}

fn count_rows(w: &relm4::gtk::Widget, rows: &mut usize, focusable: &mut usize) {
    if w.is::<adw::ActionRow>() && !w.is::<adw::SwitchRow>() {
        *rows += 1;
        if w.is_focusable() {
            *focusable += 1;
        }
    }
    let mut c = w.first_child();
    while let Some(ch) = c {
        count_rows(&ch, rows, focusable);
        c = ch.next_sibling();
    }
}
