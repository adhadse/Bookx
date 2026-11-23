//! The master password of Bookx's encrypted password store (used when there is no system keyring):
//! "Create a Master Password" the first time a password needs saving, "Unlock Saved Passwords" once
//! per session, and "Not Now" / "Don't Save" (the passwords stay in memory and are forgotten when
//! Bookx closes). The logic is in `bookx-secrets`; this file is the three dialogs and the
//! `secrets:` test hooks.
//!
//! Nothing here stores the master password: the entry text goes straight to the key derivation and
//! the buffer is wiped.

use std::rc::Rc;

use bookx_secrets::{OpenError, Prompt};
use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use zeroize::Zeroizing;

use super::SharedRef;
use crate::i18n::tr;

/// Starts the watcher: the vault asks for a dialog when it needs one, the dialog is shown here.
/// `notify` shows a toast. Under `BOOKX_DEV_SCRIPT` the dialogs stay away unless
/// `BOOKX_SECRETS_PROMPT=1` (the scripts drive the vault through `secrets:` hooks).
pub fn install(shared: &SharedRef, window: &gtk::Window, notify: Rc<dyn Fn(String)>) {
    {
        let notify = notify.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
            if crate::config::take_scrub_notice() {
                notify(tr("Saved passwords were removed from the settings file"));
            }
        });
    }
    let quiet = !crate::devtools::script().is_empty()
        && std::env::var("BOOKX_SECRETS_PROMPT").as_deref() != Ok("1");
    if quiet {
        return;
    }
    let (shared, window) = (shared.clone(), window.clone());
    gtk::glib::timeout_add_seconds_local(1, move || {
        if !DIALOG_OPEN.with(|d| d.get()) {
            match crate::secrets::vault().take_prompt() {
                Some(Prompt::Create) => create_dialog(&shared, &window, notify.clone()),
                Some(Prompt::Unlock) => unlock_dialog(&shared, &window, notify.clone(), false),
                None => {}
            }
        }
        gtk::glib::ControlFlow::Continue
    });
}

thread_local! {
    static DIALOG_OPEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Fills empty settings from the store and saves the settings (which stores values that waited in
/// memory). Called after the store was created or unlocked.
pub fn apply(shared: &SharedRef) {
    let mut cfg = shared.cfg.borrow_mut();
    crate::secrets::vault().load(&mut cfg.secret_fields());
    cfg.save();
}

fn password_text(e: &gtk::PasswordEntry) -> Zeroizing<String> {
    Zeroizing::new(e.text().to_string())
}

fn present(d: &adw::AlertDialog, window: &gtk::Window) {
    DIALOG_OPEN.with(|o| o.set(true));
    d.connect_closed(|_| DIALOG_OPEN.with(|o| o.set(false)));
    d.present(Some(window));
}

fn unlock_dialog(
    shared: &SharedRef,
    window: &gtk::Window,
    notify: Rc<dyn Fn(String)>,
    wrong: bool,
) {
    let body = if wrong {
        tr("That master password did not work. Try again.")
    } else {
        tr(
            "Enter your master password to use the passwords Bookx saved (for e-mail, sync and backups). Bookx never stores this password.",
        )
    };
    let d = adw::AlertDialog::new(Some(&tr("Unlock Saved Passwords")), Some(&body));
    let entry = gtk::PasswordEntry::new();
    entry.set_show_peek_icon(true);
    entry.set_activates_default(true);
    entry.update_property(&[gtk::accessible::Property::Label(&tr("Master password"))]);
    d.set_extra_child(Some(&entry));
    d.add_response("later", &tr("Not Now"));
    d.add_response("reset", &tr("Forgot Password"));
    d.add_response("unlock", &tr("Unlock"));
    d.set_response_appearance("unlock", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("unlock"));
    d.set_close_response("later");
    // the keyboard starts in the field (it was on the Unlock button, so typing went nowhere)
    d.set_focus(Some(&entry));
    let (shared, window2) = (shared.clone(), window.clone());
    let e2 = entry.clone();
    d.connect_response(None, move |_, id| match id {
        "unlock" => {
            let pass = password_text(&e2);
            let unlocked = crate::secrets::vault().unlock_vault(&pass);
            match &unlocked {
                Ok(()) => crate::trace::event(crate::trace::Area::Secrets, "unlock").ok(),
                Err(OpenError::WrongOrDamaged) => {
                    crate::trace::event(crate::trace::Area::Secrets, "unlock")
                        .fail_class(crate::trace::ErrorClass::Unauthorized)
                }
                Err(e) => {
                    crate::trace::event(crate::trace::Area::Secrets, "unlock").fail(&e.to_string())
                }
            }
            match unlocked {
                Ok(()) => {
                    apply(&shared);
                    notify(tr("Saved passwords unlocked"));
                }
                Err(OpenError::WrongOrDamaged) => {
                    unlock_dialog(&shared, &window2, notify.clone(), true)
                }
                Err(e) => notify(e.to_string()),
            }
        }
        "reset" => reset_dialog(&shared, &window2, notify.clone()),
        _ => {
            crate::secrets::vault().decline();
            notify(tr(
                "Saved passwords stay locked; passwords you enter are forgotten when Bookx closes",
            ));
        }
    });
    present(&d, window);
}

fn reset_dialog(shared: &SharedRef, window: &gtk::Window, notify: Rc<dyn Fn(String)>) {
    let d = adw::AlertDialog::new(
        Some(&tr("Forgot the Master Password?")),
        Some(&tr(
            "The saved passwords cannot be recovered without it. You can delete the password store and enter the passwords again.",
        )),
    );
    d.add_response("cancel", &tr("Cancel"));
    d.add_response("delete", &tr("Delete Saved Passwords"));
    d.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    d.set_close_response("cancel");
    let (shared, window2) = (shared.clone(), window.clone());
    d.connect_response(None, move |_, id| {
        if id == "delete" {
            match crate::secrets::vault().reset_vault() {
                Ok(()) => notify(tr("The password store was deleted")),
                Err(e) => notify(e.to_string()),
            }
        } else {
            unlock_dialog(&shared, &window2, notify.clone(), false);
        }
    });
    present(&d, window);
}

fn create_dialog(shared: &SharedRef, window: &gtk::Window, notify: Rc<dyn Fn(String)>) {
    let d = adw::AlertDialog::new(
        Some(&tr("Create a Master Password")),
        Some(&tr(
            "No system keyring was found. Bookx can keep your passwords in its own encrypted store, opened with a master password that it asks for once each time it starts and never saves.",
        )),
    );
    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let first = gtk::PasswordEntry::new();
    first.set_show_peek_icon(true);
    first.update_property(&[gtk::accessible::Property::Label(&tr("Master password"))]);
    let again = gtk::PasswordEntry::new();
    again.update_property(&[gtk::accessible::Property::Label(&tr(
        "Repeat the master password",
    ))]);
    let hint = gtk::Label::new(Some(&tr("Use at least 10 characters.")));
    hint.add_css_class("dim-label");
    hint.set_xalign(0.0);
    hint.set_wrap(true);
    col.append(&first);
    col.append(&again);
    col.append(&hint);
    d.set_extra_child(Some(&col));
    d.add_response("no", &tr("Don't Save"));
    d.add_response("create", &tr("Create"));
    d.set_response_appearance("create", adw::ResponseAppearance::Suggested);
    d.set_response_enabled("create", false);
    d.set_default_response(Some("create"));
    d.set_close_response("no");
    let check = {
        let (first, again, hint, d) = (first.clone(), again.clone(), hint.clone(), d.clone());
        Rc::new(move || {
            let (a, b) = (password_text(&first), password_text(&again));
            let problem = bookx_secrets::check_passphrase(&a).err();
            let ok = problem.is_none() && *a == *b;
            hint.set_text(&match (problem, a.is_empty() || b.is_empty()) {
                (Some(p), _) => tr(p),
                (None, false) if *a != *b => tr("The two passwords are different."),
                _ => tr("Use at least 10 characters."),
            });
            d.set_response_enabled("create", ok);
        })
    };
    for e in [&first, &again] {
        let c = check.clone();
        e.connect_changed(move |_| c());
    }
    let shared = shared.clone();
    d.connect_response(None, move |_, id| {
        if id == "create" {
            let pass = password_text(&first);
            match crate::secrets::vault().create_vault(&pass) {
                Ok(()) => {
                    apply(&shared);
                    notify(tr("Your passwords are saved in an encrypted store"));
                }
                Err(e) => notify(e.to_string()),
            }
        } else {
            crate::secrets::vault().decline();
            notify(tr("Not saved: it is forgotten when Bookx closes."));
        }
    });
    present(&d, window);
}

// -- test hooks ------------------------------------------------------------------------------------------

/// `secrets:<command>` in `BOOKX_DEV_SCRIPT`: `set=KEY:VALUE` (a secret of `Config::secret_fields`,
/// then saves), `create=PASSWORD`, `unlock=PASSWORD`, `relock`, `decline`, `reset`, `state`, `caption`,
/// `has=KEY:VALUE` (prints whether the live value equals it), `show-create` / `show-unlock` (open a dialog).
pub fn dev_command(shared: &SharedRef, window: &gtk::Window, cmd: &str) {
    let vault = crate::secrets::vault();
    let (name, arg) = cmd.split_once('=').unwrap_or((cmd, ""));
    match name {
        "set" => {
            let (k, v) = arg.split_once(':').unwrap_or((arg, ""));
            let mut cfg = shared.cfg.borrow_mut();
            let mut found = false;
            for (key, field) in cfg.secret_fields() {
                if key == k {
                    *field = v.to_string();
                    found = true;
                }
            }
            if found {
                cfg.save();
            }
            eprintln!("secrets_set: key={k} found={found}");
        }
        "create" => match vault.create_vault(arg) {
            Ok(()) => {
                apply(shared);
                eprintln!("secrets_create: ok");
            }
            Err(e) => eprintln!("secrets_create: error {e}"),
        },
        "unlock" => match vault.unlock_vault(arg) {
            Ok(()) => {
                apply(shared);
                eprintln!("secrets_unlock: ok");
            }
            Err(e) => eprintln!("secrets_unlock: error {e}"),
        },
        "decline" => {
            vault.decline();
            eprintln!("secrets_decline: ok");
        }
        "relock" => {
            // what a restart does: keys and values leave memory, the settings are read again without them
            vault.lock_vault();
            let mut cfg = shared.cfg.borrow_mut();
            for (_, f) in cfg.secret_fields() {
                f.clear();
            }
            eprintln!("secrets_relock: mode={:?}", vault.mode());
        }
        "reset" => eprintln!("secrets_reset: {:?}", vault.reset_vault().is_ok()),
        "state" => eprintln!(
            "secrets_state: mode={:?} file={} declined={}",
            vault.mode(),
            vault.has_file(),
            vault.declined()
        ),
        "caption" => eprintln!("secrets_caption: {}", crate::secrets::caption_now()),
        "has" => {
            let (k, v) = arg.split_once(':').unwrap_or((arg, ""));
            let mut cfg = shared.cfg.borrow_mut();
            let held = cfg
                .secret_fields()
                .into_iter()
                .find(|(key, _)| key == k)
                .map(|(_, f)| f.as_str() == v)
                .unwrap_or(false);
            eprintln!("secrets_has: key={k} equal={held}");
        }
        "show-create" => {
            create_dialog(shared, window, Rc::new(|m| eprintln!("secrets_toast: {m}")))
        }
        "show-unlock" => unlock_dialog(
            shared,
            window,
            Rc::new(|m| eprintln!("secrets_toast: {m}")),
            false,
        ),
        other => eprintln!("secrets: unknown command {other}"),
    }
}
