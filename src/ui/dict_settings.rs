//! Preferences → Reading → Dictionaries: the switch for the automatic downloads, the dictionaries
//! Bookx installed (with Remove), a way to get the dictionary of another language, and the folder for
//! your own StarDict dictionaries.

use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use super::dict_service;
use super::preferences::{register_group, PrefCtx};
use crate::dict_auto::size_text;
use crate::dict_catalog::{catalog, language_name};
use crate::dict_install;
use crate::i18n::{tr, trf};

thread_local! {
    static SUB: RefCell<Option<dict_service::Subscription>> = const { RefCell::new(None) };
    static AUTO: RefCell<Option<adw::SwitchRow>> = const { RefCell::new(None) };
}

/// Dev hook (`prefs:dict-auto=on|off`): the switch of the open Preferences, through the real widget.
pub fn dev(arg: &str) {
    if let Some(v) = arg.strip_prefix("auto=") {
        AUTO.with(|a| {
            if let Some(r) = a.borrow().as_ref() {
                r.set_active(v != "off");
            }
        });
    }
}

pub fn register() {
    register_group("reading", 55, group);
}

fn group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Dictionaries"));
    g.set_description(Some(&tr(
        "Look-ups use dictionaries on this computer. Bookx can download a free one for the language of a book.",
    )));
    let auto = adw::SwitchRow::new();
    auto.set_title(&tr(
        "Download dictionaries automatically for the languages of my books",
    ));
    auto.set_subtitle(&tr(
        "When a book in a new language is added, or opened, its dictionary is fetched in the background.",
    ));
    auto.set_active(ctx.shared.cfg.borrow().dictionaries.auto_download);
    {
        let shared = ctx.shared.clone();
        auto.connect_active_notify(move |r| {
            let mut c = shared.cfg.borrow_mut();
            c.dictionaries.auto_download = r.is_active();
            c.save();
        });
    }
    g.add(&auto);
    // your own definitions are a dictionary too, and the first one
    g.add(&super::vocab_settings::my_vocabulary_row(ctx));
    AUTO.with(|a| *a.borrow_mut() = Some(auto.clone()));

    // the dictionaries Bookx installed
    let rows: Rc<RefCell<Vec<adw::ActionRow>>> = Rc::new(RefCell::new(Vec::new()));
    let fill = {
        let (g, rows) = (g.clone(), rows.clone());
        move || {
            for r in rows.borrow_mut().drain(..) {
                g.remove(&r);
            }
            for i in dict_install::managed(&crate::dictionaries::dictionary_dir()) {
                let row = adw::ActionRow::new();
                row.set_title(&glib::markup_escape_text(&i.marker.title));
                row.set_subtitle(&glib::markup_escape_text(&format!(
                    "{} · {}",
                    language_name(&i.marker.lang),
                    i.marker.licence
                )));
                let rm = gtk::Button::from_icon_name("user-trash-symbolic");
                rm.add_css_class("flat");
                rm.set_valign(gtk::Align::Center);
                let tip = trf("Remove {}", &[&i.marker.title]);
                rm.set_tooltip_text(Some(&tip));
                super::audit::label(&rm, &tip);
                let row2 = row.clone();
                rm.connect_clicked(move |_| {
                    let _ = dict_install::remove(&i);
                    row2.set_visible(false);
                });
                row.add_suffix(&rm);
                g.add(&row);
                rows.borrow_mut().push(row);
            }
        }
    };
    fill();

    // another language
    let langs = adw::ComboRow::new();
    langs.set_title(&tr("Get a dictionary"));
    let names: Vec<String> = catalog()
        .languages
        .iter()
        .map(|l| format!("{} ({})", l.name, size_text(l.dictionaries[0].size)))
        .collect();
    langs.set_model(Some(&gtk::StringList::new(
        &names.iter().map(String::as_str).collect::<Vec<_>>(),
    )));
    let get = gtk::Button::with_label(&tr("Download"));
    get.add_css_class("suggested-action");
    get.set_valign(gtk::Align::Center);
    super::audit::label(&get, &tr("Download the dictionary of the chosen language"));
    {
        let (shared, langs) = (ctx.shared.clone(), langs.clone());
        get.connect_clicked(move |_| {
            if let Some(l) = catalog().languages.get(langs.selected() as usize) {
                dict_service::request(&shared, &l.code, true);
            }
        });
    }
    langs.add_suffix(&get);
    g.add(&langs);

    let folder = adw::ActionRow::new();
    folder.set_title(&tr("Your own dictionaries"));
    folder.set_subtitle(&tr(
        "StarDict dictionaries (folders with .ifo, .idx and .dict files) put in the dictionaries folder are used too.",
    ));
    let open = gtk::Button::with_label(&tr("Open folder"));
    open.add_css_class("flat");
    open.set_valign(gtk::Align::Center);
    open.connect_clicked(|_| {
        let dir = crate::dictionaries::dictionary_dir();
        let _ = std::fs::create_dir_all(&dir);
        super::open_uri(&format!("file://{}", dir.display()));
    });
    folder.add_suffix(&open);
    g.add(&folder);

    // a download that finishes while the dialog is open adds its row
    let weak = g.downgrade();
    let fill = fill.clone();
    SUB.with(|s| {
        *s.borrow_mut() = Some(dict_service::subscribe(move |e| {
            if matches!(e, dict_service::Event::Done { .. }) && weak.upgrade().is_some() {
                fill();
            }
        }));
    });
    g
}
