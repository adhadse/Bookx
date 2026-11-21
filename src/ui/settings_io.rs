//! Preferences → General → Export Settings / Import Settings (X10). The file format, the limits and the
//! preview are in `crate::settings_io`; this is the dialogs.
//!
//! Export writes the settings without passwords, tokens or anything of this machine. Import reads a file
//! (at most 1 MiB), checks all of it, shows which settings would change and only then applies them, keeping
//! the current secrets.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio};

use super::{window_of, SharedRef};
use crate::i18n::{tr, trf};
use crate::settings_io::{self, Imported, Preview};

/// The group of the General page.
pub fn group(parent: &impl IsA<gtk::Widget>, shared: &SharedRef) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::builder()
        .title(tr("Settings Backup"))
        .description(tr(
            "Move your preferences to another computer. Passwords and tokens are never included.",
        ))
        .build();

    let export = adw::ActionRow::builder()
        .title(tr("Export Settings"))
        .subtitle(tr("Save your preferences to a file"))
        .build();
    let b = gtk::Button::with_label(&tr("Export"));
    b.set_valign(gtk::Align::Center);
    super::audit::label(&b, &tr("Export Settings"));
    export.add_suffix(&b);
    export.set_activatable_widget(Some(&b));
    {
        let (parent, shared) = (parent.clone().upcast::<gtk::Widget>(), shared.clone());
        b.connect_clicked(move |_| ask_export(&parent, &shared));
    }
    g.add(&export);

    let import = adw::ActionRow::builder()
        .title(tr("Import Settings"))
        .subtitle(tr(
            "Read preferences from a file and look at the changes first",
        ))
        .build();
    let b = gtk::Button::with_label(&tr("Import"));
    b.set_valign(gtk::Align::Center);
    super::audit::label(&b, &tr("Import Settings"));
    import.add_suffix(&b);
    import.set_activatable_widget(Some(&b));
    {
        let (parent, shared) = (parent.clone().upcast::<gtk::Widget>(), shared.clone());
        b.connect_clicked(move |_| ask_import(&parent, &shared));
    }
    g.add(&import);
    g
}

fn ask_export(parent: &gtk::Widget, shared: &SharedRef) {
    let dlg = gtk::FileDialog::builder()
        .title(tr("Export Settings"))
        .initial_name("bookx-settings.json")
        .build();
    let shared = shared.clone();
    dlg.save(
        window_of(parent).as_ref(),
        gio::Cancellable::NONE,
        move |res| {
            let Some(path) = res.ok().and_then(|f| f.path()) else {
                return;
            };
            match write_export(&shared, &path) {
                Ok(()) => super::preferences::toast(&tr("Settings Exported")),
                Err(e) => super::preferences::toast(&trf("Could not export: {}", &[&e])),
            }
        },
    );
}

fn write_export(shared: &SharedRef, path: &Path) -> Result<(), String> {
    let text = settings_io::export(&shared.cfg.borrow());
    std::fs::write(path, text).map_err(|e| e.to_string())
}

fn ask_import(parent: &gtk::Widget, shared: &SharedRef) {
    let dlg = gtk::FileDialog::builder()
        .title(tr("Import Settings"))
        .build();
    let (shared, parent2) = (shared.clone(), parent.clone());
    dlg.open(
        window_of(parent).as_ref(),
        gio::Cancellable::NONE,
        move |res| {
            let Some(path) = res.ok().and_then(|f| f.path()) else {
                return;
            };
            show_preview(&parent2, &shared, &path);
        },
    );
}

/// Reads the file with the size limit.
fn read_limited(path: &Path) -> Result<String, String> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > settings_io::MAX_BYTES {
        return Err(trf(
            "the file is larger than {} KiB",
            &[&(settings_io::MAX_BYTES / 1024).to_string()],
        ));
    }
    std::fs::read_to_string(path).map_err(|e| e.to_string())
}

thread_local! {
    /// The preview on screen, for the dev hook.
    static PREVIEW: RefCell<Option<adw::AlertDialog>> = const { RefCell::new(None) };
}

/// The words of the preview.
pub fn preview_text(p: &Preview) -> String {
    if p.changed.is_empty() {
        return tr("Nothing would change: these settings are the same as yours.");
    }
    let mut t = trf("{} settings would change:", &[&p.changed.len().to_string()]);
    t.push('\n');
    t.push_str(&p.changed.join(", "));
    if !p.unknown.is_empty() {
        t.push_str("\n\n");
        t.push_str(&trf(
            "{} settings are not known to this version and are left out.",
            &[&p.unknown.len().to_string()],
        ));
    }
    t
}

fn show_preview(parent: &gtk::Widget, shared: &SharedRef, path: &Path) {
    let imp = match read_limited(path)
        .and_then(|t| settings_io::parse(&t).map_err(|e| format!("{e:#}")))
    {
        Ok(i) => i,
        Err(e) => {
            super::preferences::toast(&trf("Could not import: {}", &[&e]));
            return;
        }
    };
    let p = settings_io::preview(&shared.cfg.borrow(), &imp);
    let d = adw::AlertDialog::new(Some(&tr("Import Settings")), Some(&preview_text(&p)));
    d.add_responses(&[("cancel", &tr("Cancel")), ("import", &tr("Import"))]);
    d.set_response_enabled("import", !p.changed.is_empty());
    d.set_response_appearance("import", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("cancel"));
    d.set_close_response("cancel");
    let shared2 = shared.clone();
    d.connect_response(None, move |_, r| {
        if r == "import" {
            match apply(&shared2, &imp) {
                Ok(()) => super::preferences::toast(&tr(
                    "Settings Imported. Restart Bookx to apply all of them.",
                )),
                Err(e) => super::preferences::toast(&trf("Could not import: {}", &[&e])),
            }
        }
    });
    PREVIEW.with(|c| *c.borrow_mut() = Some(d.clone()));
    d.present(Some(parent));
}

fn apply(shared: &SharedRef, imp: &Imported) -> Result<(), String> {
    let new = settings_io::apply(&shared.cfg.borrow(), imp).map_err(|e| format!("{e:#}"))?;
    new.save();
    *shared.cfg.borrow_mut() = new;
    Ok(())
}

/// Dev hook `settings:export=PATH` (`settings_io: exported bytes=N`), `settings:import=PATH` (opens the preview,
/// `settings_io: preview changed=a,b unknown=… skipped=…` or `settings_io: refused REASON`),
/// `settings:import-apply` (`settings_io: applied`), `settings:get=KEY` (`settings_io: KEY=VALUE`).
pub fn dev(arg: &str, shared: &SharedRef) {
    if let Some(p) = arg.strip_prefix("export=") {
        match write_export(shared, &PathBuf::from(p)) {
            Ok(()) => eprintln!(
                "settings_io: exported bytes={}",
                std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
            ),
            Err(e) => eprintln!("settings_io: export failed {e}"),
        }
    } else if let Some(p) = arg.strip_prefix("import=") {
        let path = PathBuf::from(p);
        match read_limited(&path).and_then(|t| settings_io::parse(&t).map_err(|e| format!("{e:#}")))
        {
            Err(e) => eprintln!("settings_io: refused {e}"),
            Ok(imp) => {
                let pv = settings_io::preview(&shared.cfg.borrow(), &imp);
                eprintln!(
                    "settings_io: preview changed={} unknown={} skipped={}",
                    pv.changed.join(","),
                    pv.unknown.join(","),
                    pv.skipped.join(",")
                );
                if let Some(win) = relm4::main_adw_application().active_window() {
                    show_preview(win.upcast_ref(), shared, &path);
                }
            }
        }
    } else if arg == "import-apply" {
        let d = PREVIEW.with(|c| c.borrow_mut().take());
        if let Some(d) = d {
            d.emit_by_name_with_details::<()>(
                "response",
                relm4::gtk::glib::Quark::from_str("import"),
                &[&"import"],
            );
            d.force_close();
            eprintln!("settings_io: applied");
        } else {
            eprintln!("settings_io: no preview");
        }
    } else if let Some(k) = arg.strip_prefix("get=") {
        let v = serde_json::to_value(&*shared.cfg.borrow())
            .ok()
            .and_then(|v| v.get(k).cloned());
        eprintln!(
            "settings_io: {k}={}",
            v.map(|v| v.to_string()).unwrap_or_default()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preview_names_what_changes() {
        let p = Preview {
            changed: vec!["eink".into(), "scheme".into()],
            unknown: vec!["future".into()],
            skipped: vec![],
        };
        let t = preview_text(&p);
        assert!(
            t.starts_with("2 settings would change:\neink, scheme"),
            "{t}"
        );
        assert!(t.contains("1 settings are not known"), "{t}");
        assert!(preview_text(&Preview::default()).starts_with("Nothing would change"));
    }

    #[test]
    fn a_big_file_is_refused_before_it_is_read() {
        let f =
            std::env::temp_dir().join(format!("bookx-settings-big-{}.json", std::process::id()));
        std::fs::write(&f, vec![b' '; settings_io::MAX_BYTES as usize + 1]).unwrap();
        let e = read_limited(&f).unwrap_err();
        assert!(e.contains("larger"), "{e}");
        let _ = std::fs::remove_file(&f);
        assert!(read_limited(&f).is_err());
    }
}
