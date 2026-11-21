//! Preferences → Reading → Dictionaries: the switch for the automatic downloads, the dictionaries
//! Bookx installed (with Remove), a way to get the dictionary of another language, and the folder for
//! your own StarDict dictionaries.

use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use super::dict_service;
use super::preferences::{register_group, register_page, PrefCtx};
use crate::dict_auto::size_text;
use crate::dict_catalog::{catalog, language_name};
use crate::dict_install;
use crate::i18n::{tr, trf};

thread_local! {
    static SUB: RefCell<Option<dict_service::Subscription>> = const { RefCell::new(None) };
    static AUTO: RefCell<Option<adw::SwitchRow>> = const { RefCell::new(None) };
    static REMOVE: RefCell<Vec<(String, adw::ActionRow, gtk::Button)>> = const { RefCell::new(Vec::new()) };
}

fn installed_ids() -> String {
    dict_install::managed(&crate::dictionaries::dictionary_dir())
        .iter()
        .map(|i| i.marker.id.clone())
        .collect::<Vec<_>>()
        .join(",")
}

/// Dev hook (`prefs:dict-auto=on|off`): the switch of the open Preferences, through the real widget.
/// `prefs:dict-remove=NAME` presses the remove button of the installed dictionary titled NAME and
/// prints `prefs_dict_remove: name=N row_visible=B installed=ID,ID`; `prefs:dict-undo` presses
/// Undo on the toast and prints `prefs_dict_undo: pressed=B row_visible=B installed=ID,ID`.
pub fn dev(arg: &str) {
    if let Some(name) = arg.strip_prefix("remove=") {
        let hit = REMOVE.with(|r| {
            r.borrow()
                .iter()
                .find(|(t, row, _)| t == name && row.is_visible())
                .map(|(_, row, b)| (row.clone(), b.clone()))
        });
        if let Some((row, b)) = hit {
            b.emit_clicked();
            eprintln!(
                "prefs_dict_remove: name={name} row_visible={} installed={}",
                row.is_visible(),
                installed_ids()
            );
        } else {
            eprintln!("prefs_dict_remove: name={name} not found");
        }
        return;
    }
    if arg == "undo" {
        let pressed = super::preferences::press_undo();
        let visible = REMOVE.with(|r| r.borrow().iter().any(|(_, row, _)| row.is_visible()));
        eprintln!(
            "prefs_dict_undo: pressed={pressed} row_visible={visible} installed={}",
            installed_ids()
        );
        return;
    }
    if let Some(v) = arg.strip_prefix("auto=") {
        AUTO.with(|a| {
            if let Some(r) = a.borrow().as_ref() {
                r.set_active(v != "off");
            }
        });
    }
}

pub fn register() {
    register_page(
        "dictionaries",
        &tr("Dictionaries"),
        "accessories-dictionary-symbolic",
        35,
    );
    register_group("dictionaries", 55, group);
}

fn group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Dictionaries"));
    g.set_description(Some(&tr(
        "Look-ups use dictionaries on this computer. Bookx can download a free one for the language of a book.",
    )));
    let auto = adw::SwitchRow::new();
    auto.set_title(&tr("Download dictionaries automatically"));
    auto.set_subtitle(&tr(
        "When a book in a new language is added or opened, its dictionary is fetched in the background.",
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
            REMOVE.with(|r| r.borrow_mut().clear());
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
                REMOVE.with(|r| {
                    r.borrow_mut()
                        .push((i.marker.title.clone(), row.clone(), rm.clone()))
                });
                rm.connect_clicked(move |_| remove_with_undo(&row2, &i));
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
    let base_subtitle = langs.subtitle();
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
    let (langs_w, get_w) = (langs.downgrade(), get.downgrade());
    let fill = fill.clone();
    SUB.with(|s| {
        *s.borrow_mut() = Some(dict_service::subscribe(move |e| {
            if weak.upgrade().is_none() {
                return;
            }
            let (Some(langs), Some(get)) = (langs_w.upgrade(), get_w.upgrade()) else {
                return;
            };
            let total = |lang: &str| {
                catalog()
                    .languages
                    .iter()
                    .find(|l| l.code == lang)
                    .map_or(0, |l| l.dictionaries[0].size)
            };
            match e {
                dict_service::Event::Progress { lang, fraction, .. } => {
                    langs.set_subtitle(&progress_text(*fraction, total(lang)));
                    get.set_sensitive(false);
                }
                dict_service::Event::Done { .. } => {
                    langs.set_subtitle(&base_subtitle.clone().unwrap_or_default());
                    get.set_sensitive(true);
                    fill();
                }
                dict_service::Event::Failed { message, .. } => {
                    langs.set_subtitle(&glib::markup_escape_text(&trf(
                        "Could not download: {}",
                        &[message],
                    )));
                    get.set_sensitive(true);
                }
            }
        }));
    });
    g
}

/// "Downloading… 3.1 MB of 12 MB" while the size of the part done is known, else "Downloading…".
pub fn progress_text(fraction: Option<f64>, total: u64) -> String {
    match fraction {
        Some(f) if total > 0 => {
            let done = (f.clamp(0.0, 1.0) * total as f64) as u64;
            trf(
                "Downloading… {} of {}",
                &[&size_text(done), &size_text(total)],
            )
        }
        _ => tr("Downloading…"),
    }
}

/// Hides the row at once and removes the dictionary only when the toast ends without **Undo**:
/// the folder moves out of the dictionaries folder meanwhile, so nothing looks it up any more.
fn remove_with_undo(row: &adw::ActionRow, i: &dict_install::Installed) {
    let root = crate::dictionaries::dictionary_dir();
    let name = i.marker.title.clone();
    let Some(stashed) = stash(&i.dir, &trash_dir(&root)) else {
        let _ = dict_install::remove(i);
        row.set_visible(false);
        return;
    };
    row.set_visible(false);
    let (row_u, orig, stashed_u) = (row.clone(), i.dir.clone(), stashed.clone());
    super::preferences::toast_undo(
        &trf("Removed {}", &[&name]),
        8,
        move || {
            if unstash(&stashed_u, &orig).is_ok() {
                row_u.set_visible(true);
            }
        },
        move || purge(&stashed),
    );
}

/// Where removed dictionaries wait for the end of the undo toast: next to the dictionaries folder,
/// never inside it.
pub fn trash_dir(root: &std::path::Path) -> std::path::PathBuf {
    root.parent().unwrap_or(root).join("dictionaries-removed")
}

/// Moves `dir` into `trash`; `None` when that is not possible.
pub fn stash(dir: &std::path::Path, trash: &std::path::Path) -> Option<std::path::PathBuf> {
    std::fs::create_dir_all(trash).ok()?;
    let name = dir.file_name()?;
    let mut to = trash.join(name);
    let mut n = 1;
    while to.exists() {
        to = trash.join(format!("{}-{n}", name.to_string_lossy()));
        n += 1;
    }
    std::fs::rename(dir, &to).ok()?;
    Some(to)
}

/// Puts a stashed dictionary back where it was.
pub fn unstash(stashed: &std::path::Path, original: &std::path::Path) -> std::io::Result<()> {
    std::fs::rename(stashed, original)
}

/// Deletes a stashed dictionary for good (only a folder that still has Bookx's marker).
pub fn purge(stashed: &std::path::Path) {
    if stashed.join(dict_install::MARKER).is_file() {
        let _ = std::fs::remove_dir_all(stashed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-dset-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_stashed_dictionary_comes_back_or_is_deleted() {
        let root = dir("undo");
        let dict = root.join("dictionaries/de-1");
        std::fs::create_dir_all(&dict).unwrap();
        std::fs::write(dict.join(dict_install::MARKER), "{}").unwrap();
        let trash = trash_dir(&root.join("dictionaries"));
        assert_eq!(trash, root.join("dictionaries-removed"));
        let stashed = stash(&dict, &trash).unwrap();
        assert!(!dict.exists() && stashed.exists());
        unstash(&stashed, &dict).unwrap();
        assert!(dict.join(dict_install::MARKER).is_file());
        let stashed = stash(&dict, &trash).unwrap();
        purge(&stashed);
        assert!(!stashed.exists() && !dict.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn purge_ignores_a_folder_without_the_marker() {
        let root = dir("nomarker");
        let other = root.join("mine");
        std::fs::create_dir_all(&other).unwrap();
        purge(&other);
        assert!(other.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn progress_names_what_is_done_of_the_total() {
        let t = progress_text(Some(0.5), 2_000_000);
        assert!(t.starts_with("Downloading… ") && t.contains(" of "), "{t}");
        assert_eq!(progress_text(None, 5), "Downloading…");
    }
}
