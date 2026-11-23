//! The capability sheet (X3): before a plugin is switched on the user sees, in plain words, what it may do,
//! whether it is signed, and chooses Allow or Cancel. Allow is remembered together with a fingerprint of the
//! plugin files (`bookx_plugin::trust`), so a plugin that changes on disk has to be allowed again.
//!
//! Unsigned plugins are allowed with a warning (decision G10); nothing in this build can verify a
//! signature yet, so a `plugin.sig` shows as "signed, but not checked".

use bookx_plugin::trust::UnsupportedVerifier;
use bookx_plugin::{Capability, Manifest, SignatureStatus, TrustState};
use relm4::adw::{self, prelude::*};
use relm4::gtk;

use super::SharedRef;
use crate::i18n::{tr, trf};

/// What the dialog shows, as plain data (tested without GTK).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sheet {
    pub title: String,
    /// One line per thing the plugin may do.
    pub lines: Vec<String>,
    /// What the plugin cannot do, always the same facts, in one sentence.
    pub limits: String,
    /// Shown when the plugin is unsigned, unverifiable or changed.
    pub warning: Option<String>,
}

pub fn sheet(manifest: &Manifest, signature: &SignatureStatus, state: TrustState) -> Sheet {
    let mut lines: Vec<String> = bookx_plugin::trust::capabilities(manifest)
        .iter()
        .map(|c| match c {
            Capability::ReadBook => tr("Read the book that is open in the editor"),
            Capability::ChangeBook => tr(
                "Change the book that is open in the editor (each action is one checkpoint you can restore)",
            ),
            Capability::Network(host) => trf("Connect to {}", &[host]),
        })
        .collect();
    if lines.is_empty() {
        lines.push(tr(
            "Nothing: it can only calculate with what Bookx hands to it",
        ));
    }
    let warning = if state == TrustState::Changed {
        Some(tr(
            "The plugin files changed since you allowed it. Only allow it again if you installed an update.",
        ))
    } else {
        match signature {
            SignatureStatus::Valid { .. } => None,
            SignatureStatus::Unsigned => Some(tr(
                "This plugin is not signed. Only allow plugins from people you trust.",
            )),
            SignatureStatus::Unverified { .. } => Some(tr(
                "This plugin carries a signature, but this version of Bookx cannot check it yet. Treat it as unsigned.",
            )),
            SignatureStatus::Invalid { .. } => Some(tr(
                "The signature does not match the plugin files. Do not allow it unless you changed them yourself.",
            )),
            SignatureStatus::Malformed(_) => Some(tr(
                "The signature file of this plugin is damaged. Treat the plugin as unsigned.",
            )),
        }
    };
    Sheet {
        title: trf("Allow {} to run?", &[&manifest.name]),
        lines,
        limits: tr(
            "It runs in a sandbox: it cannot read your other files, and it can only reach the network hosts listed above.",
        ),
        warning,
    }
}

/// Whether switching `id` on has to go through the sheet first.
pub fn needs_sheet(shared: &SharedRef, id: &str) -> bool {
    let m = shared.plugins.borrow();
    m.manifest_of(id).is_some() && m.trust_state(id) != TrustState::Trusted
}

/// Shows the sheet; `on_allow` runs after the approval was stored.
pub fn show(
    parent: &impl IsA<gtk::Widget>,
    shared: &SharedRef,
    id: &str,
    on_allow: impl Fn() + 'static,
) {
    let (s, name) = {
        let m = shared.plugins.borrow();
        let Some(manifest) = m.manifest_of(id) else {
            return;
        };
        let sig = m
            .signature_status(id, &UnsupportedVerifier)
            .unwrap_or(SignatureStatus::Unsigned);
        (
            sheet(manifest, &sig, m.trust_state(id)),
            manifest.name.clone(),
        )
    };
    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    for l in &s.lines {
        let label = gtk::Label::new(Some(&format!("•  {l}")));
        label.set_wrap(true);
        label.set_xalign(0.0);
        body.append(&label);
    }
    let limits = gtk::Label::new(Some(&s.limits));
    limits.set_wrap(true);
    limits.set_xalign(0.0);
    limits.add_css_class("dim-label");
    body.append(&limits);
    if let Some(w) = &s.warning {
        let label = gtk::Label::new(Some(w));
        label.set_wrap(true);
        label.set_xalign(0.0);
        label.add_css_class("warning");
        body.append(&label);
    }
    let d = adw::AlertDialog::new(
        Some(&s.title),
        Some(&trf("{} asks for these permissions:", &[&name])),
    );
    d.set_extra_child(Some(&body));
    d.add_responses(&[("cancel", &tr("Cancel")), ("allow", &tr("Allow"))]);
    d.set_response_appearance("allow", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("cancel"));
    d.set_close_response("cancel");
    let (shared, id) = (shared.clone(), id.to_string());
    d.connect_response(None, move |_, r| {
        if r == "allow" {
            let stored = shared.plugins.borrow_mut().trust(&id);
            match stored {
                Ok(()) => on_allow(),
                Err(e) => tracing::warn!("cannot remember the approval of {id}: {e:#}"),
            }
        }
    });
    SHEET.with(|c| *c.borrow_mut() = Some(d.clone()));
    d.present(Some(parent));
}

thread_local! {
    /// The sheet on screen, for the dev hook.
    static SHEET: std::cell::RefCell<Option<adw::AlertDialog>> = const { std::cell::RefCell::new(None) };
}

/// Dev hook `plugins:sheet=ID`: opens the sheet; `plugins:sheet-dump` prints it (`plugin_sheet: …`);
/// `plugins:sheet-allow` presses Allow.
pub fn dev(shared: &SharedRef, arg: &str) {
    if let Some(id) = arg.strip_prefix("sheet=") {
        let Some(win) = relm4::main_adw_application().active_window() else {
            eprintln!("plugin_sheet: no window");
            return;
        };
        let s2 = shared.clone();
        let id2 = id.to_string();
        show(&win, shared, id, move || {
            eprintln!(
                "plugin_sheet: allowed state={:?}",
                s2.plugins.borrow().trust_state(&id2)
            );
        });
        let m = shared.plugins.borrow();
        if let Some(manifest) = m.manifest_of(id) {
            let sig = m
                .signature_status(id, &UnsupportedVerifier)
                .unwrap_or(SignatureStatus::Unsigned);
            let s = sheet(manifest, &sig, m.trust_state(id));
            eprintln!(
                "plugin_sheet: title={:?} lines={} warning={}",
                s.title,
                s.lines.join("|"),
                s.warning.is_some()
            );
        }
    } else if arg == "sheet-allow" {
        let d = SHEET.with(|c| c.borrow_mut().take());
        eprintln!("plugin_sheet: allow_pressed={}", d.is_some());
        if let Some(d) = d {
            d.emit_by_name_with_details::<()>(
                "response",
                relm4::gtk::glib::Quark::from_str("allow"),
                &[&"allow"],
            );
            d.force_close();
        }
    } else if arg == "state" {
        let ids: Vec<String> = shared
            .plugins
            .borrow()
            .list()
            .into_iter()
            .map(|p| p.id)
            .collect();
        for id in ids {
            eprintln!(
                "plugin_trust: {id} {:?}",
                shared.plugins.borrow().trust_state(&id)
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(perms: &str) -> Manifest {
        Manifest::parse(&format!(
            "id = \"t.p\"\nname = \"Tidy\"\nversion = \"1.0\"\napi_version = 2\nkind = \"editor-action\"\npermissions = [{perms}]\n[[action]]\nid = \"a\"\nlabel = \"A\"\n"
        ))
        .unwrap()
    }

    #[test]
    fn the_sheet_lists_every_permission_in_words() {
        let m = manifest("\"editor.read\", \"editor.write\"");
        let s = sheet(&m, &SignatureStatus::Unsigned, TrustState::NeverAsked);
        assert_eq!(s.lines.len(), 2);
        assert!(s.lines[0].contains("Read the book"));
        assert!(s.lines[1].contains("Change the book"));
        assert!(s.title.contains("Tidy"));
    }

    #[test]
    fn a_plugin_without_permissions_says_so() {
        let s = sheet(
            &manifest(""),
            &SignatureStatus::Unsigned,
            TrustState::NeverAsked,
        );
        assert_eq!(s.lines.len(), 1);
        assert!(s.lines[0].starts_with("Nothing"));
    }

    #[test]
    fn warnings_follow_signature_and_trust() {
        let m = manifest("");
        let w = |sig: SignatureStatus, st| sheet(&m, &sig, st).warning;
        assert!(
            w(SignatureStatus::Unsigned, TrustState::NeverAsked)
                .unwrap()
                .contains("not signed")
        );
        assert!(
            w(
                SignatureStatus::Valid { key_id: "x".into() },
                TrustState::NeverAsked
            )
            .is_none()
        );
        assert!(
            w(
                SignatureStatus::Unverified { key_id: "x".into() },
                TrustState::NeverAsked
            )
            .unwrap()
            .contains("cannot check")
        );
        assert!(
            w(
                SignatureStatus::Invalid { key_id: "x".into() },
                TrustState::NeverAsked
            )
            .unwrap()
            .contains("does not match")
        );
        // a changed plugin warns even when it is validly signed
        assert!(
            w(
                SignatureStatus::Valid { key_id: "x".into() },
                TrustState::Changed
            )
            .unwrap()
            .contains("changed")
        );
    }

    #[test]
    fn network_hosts_are_named() {
        let m = manifest("\"editor.read\"");
        let mut net = m.clone();
        net.permissions
            .push(bookx_plugin::manifest::Permission::Network("a.org".into()));
        let s = sheet(&net, &SignatureStatus::Unsigned, TrustState::NeverAsked);
        assert!(s.lines.iter().any(|l| l.contains("a.org")));
    }
}
