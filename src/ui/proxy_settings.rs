//! Preferences → Sync → Advanced: the three settings of a reverse proxy in front of the server:
//! the public host names, the proxies whose `X-Forwarded-For` is believed (so the login limit counts
//! each real client) and the sub-path the proxy publishes Bookx under. Input is validated by
//! `ShareSettings::proxy_problem` before anything is saved; a running server restarts with it.

use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk;

use super::SharedRef;
use super::share::reapply;
use crate::i18n::tr;

thread_local! {
    static ROWS: RefCell<Option<(adw::EntryRow, adw::EntryRow, adw::EntryRow, gtk::Label)>> =
        const { RefCell::new(None) };
}

/// Checks `hosts`, `proxies` and `prefix` as typed. `Ok` carries the settings to save (the sub-path
/// normalised), `Err` the sentence to show.
pub fn candidate(
    base: &crate::sharing::ShareSettings,
    hosts: &str,
    proxies: &str,
    prefix: &str,
) -> Result<crate::sharing::ShareSettings, String> {
    let mut s = base.clone();
    s.allowed_hosts = hosts.trim().to_string();
    s.trusted_proxies = proxies.trim().to_string();
    s.public_prefix = prefix.trim().to_string();
    if let Some(p) = s.proxy_problem() {
        return Err(p);
    }
    s.public_prefix =
        bookx_serve::server::clean_public_prefix(&s.public_prefix).unwrap_or_default();
    Ok(s)
}

/// The rows for the Advanced group of Sync.
pub fn rows(shared: &SharedRef) -> Vec<gtk::Widget> {
    let cfg = shared.cfg.borrow().share.clone();
    let mk = |title: &str, text: &str| {
        let r = adw::EntryRow::builder()
            .title(title)
            .show_apply_button(true)
            .build();
        r.set_text(text);
        r
    };
    let hosts = mk(&tr("Allowed host names"), &cfg.allowed_hosts);
    hosts.set_tooltip_text(Some(&tr(
        "Names besides IP addresses, localhost and .local names that may be used to reach the server, such as the public name behind a reverse proxy. Separate several with commas.",
    )));
    let proxies = mk(&tr("Trusted proxy addresses"), &cfg.trusted_proxies);
    proxies.set_tooltip_text(Some(&tr(
        "Addresses or networks (10.0.0.0/8) of your reverse proxy. Only then is its X-Forwarded-For header believed, so the failed-login limit counts each client and not the proxy.",
    )));
    let prefix = mk(&tr("Sub-path behind a proxy"), &cfg.public_prefix);
    prefix.set_tooltip_text(Some(&tr(
        "Set this when the proxy publishes Bookx under an address such as https://example.com/books. Links then start with /books.",
    )));
    let err = gtk::Label::new(None);
    err.set_wrap(true);
    err.set_xalign(0.0);
    err.set_selectable(true);
    err.add_css_class("error");
    err.set_visible(false);

    let commit = {
        let (shared, hosts, proxies, prefix, err) = (
            shared.clone(),
            hosts.clone(),
            proxies.clone(),
            prefix.clone(),
            err.clone(),
        );
        Rc::new(move || {
            let base = shared.cfg.borrow().share.clone();
            match candidate(&base, &hosts.text(), &proxies.text(), &prefix.text()) {
                Err(msg) => {
                    err.set_label(&msg);
                    err.set_visible(true);
                    for r in [&hosts, &proxies, &prefix] {
                        r.add_css_class("error");
                    }
                }
                Ok(s) => {
                    err.set_visible(false);
                    for r in [&hosts, &proxies, &prefix] {
                        r.remove_css_class("error");
                    }
                    if s != base {
                        {
                            let mut c = shared.cfg.borrow_mut();
                            c.share = s;
                            c.save();
                        }
                        reapply(&shared);
                    }
                }
            }
        })
    };
    for r in [&hosts, &proxies, &prefix] {
        let c = commit.clone();
        r.connect_apply(move |_| c());
    }
    ROWS.with(|x| {
        *x.borrow_mut() = Some((hosts.clone(), proxies.clone(), prefix.clone(), err.clone()))
    });
    vec![
        hosts.upcast(),
        proxies.upcast(),
        prefix.upcast(),
        err.upcast(),
    ]
}

/// Dev hook (`prefs:sync-proxy=HOSTS;PROXIES;PREFIX` types into the rows and applies them,
/// `prefs:sync-proxy-dump` prints `prefs_proxy: hosts=.. proxies=.. prefix=.. error=..`).
pub fn dev(cmd: &str, shared: Option<&SharedRef>) {
    ROWS.with(|x| {
        let x = x.borrow();
        let Some((h, p, f, e)) = x.as_ref() else {
            eprintln!("prefs_proxy: no page");
            return;
        };
        if let Some(v) = cmd.strip_prefix('=') {
            let mut it = v.splitn(3, ';');
            h.set_text(it.next().unwrap_or(""));
            p.set_text(it.next().unwrap_or(""));
            f.set_text(it.next().unwrap_or(""));
            h.emit_by_name::<()>("apply", &[]);
            return;
        }
        let saved = shared
            .map(|s| {
                let c = s.cfg.borrow();
                format!(
                    "{}|{}|{}",
                    c.share.allowed_hosts, c.share.trusted_proxies, c.share.public_prefix
                )
            })
            .unwrap_or_default();
        eprintln!(
            "prefs_proxy: hosts={} proxies={} prefix={} error={} saved={saved}",
            h.text(),
            p.text(),
            f.text(),
            if e.is_visible() {
                e.label().to_string()
            } else {
                "none".into()
            }
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sharing::ShareSettings;

    #[test]
    fn good_input_is_normalised() {
        let s = candidate(
            &ShareSettings::default(),
            " Books.Example.com:443, nas ",
            "10.0.0.0/8 ::1",
            "books/",
        )
        .unwrap();
        assert_eq!(s.public_prefix, "/books");
        assert_eq!(s.allowed_hosts, "Books.Example.com:443, nas");
        assert_eq!(
            bookx_serve::hostguard::parse_host_list(&s.allowed_hosts).unwrap(),
            vec!["books.example.com", "nas"]
        );
    }

    #[test]
    fn bad_input_is_refused_with_the_item_named() {
        let d = ShareSettings::default();
        let e = candidate(&d, "https://x.com", "", "").unwrap_err();
        assert!(e.contains("https://x.com"), "{e}");
        let e = candidate(&d, "", "0.0.0.0/0", "").unwrap_err();
        assert!(e.contains("0.0.0.0/0"), "{e}");
        assert!(candidate(&d, "", "", "/a/../b").is_err());
        assert!(candidate(&d, "", "", "/bookx").is_err());
        assert!(candidate(&d, "", "", "").is_ok());
    }

    #[test]
    fn the_server_config_receives_the_settings() {
        let s = candidate(
            &ShareSettings::default(),
            "books.example.com",
            "127.0.0.1",
            "/books",
        )
        .unwrap();
        let lib: bookx_serve::server::LibraryProvider =
            std::sync::Arc::new(bookx_serve::server::LibrarySnapshot::default);
        let cfg =
            crate::sharing::server_config(&s, lib, std::path::Path::new("/nonexistent")).unwrap();
        assert!(cfg.allowed_hosts.contains(&"books.example.com".to_string()));
        assert_eq!(cfg.public_prefix, "/books");
        assert_eq!(cfg.limits.trusted_proxies.len(), 1);
    }
}
