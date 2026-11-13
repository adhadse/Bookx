//! The sharing window: serve the library to other devices (OPDS, web page, KOReader sync).

use crate::i18n::{tr, trf};
use relm4::adw::{self, prelude::*};
use relm4::gtk;

use std::cell::RefCell;

use super::SharedRef;
use crate::sharing::ShareSettings;

thread_local! {
    /// The KOReader address group of the open window (the UI tests read it).
    static KO: RefCell<Option<(adw::PreferencesGroup, gtk::Label)>> = const { RefCell::new(None) };
}

/// Test hook: what the KOReader part of the window shows.
pub fn dev_dump() {
    KO.with(|k| match &*k.borrow() {
        Some((g, l)) => eprintln!("share_ko: visible={} address={}", g.is_visible(), l.text()),
        None => eprintln!("share_ko: no window"),
    });
}

/// Starts or stops the server according to `settings`. Returns what the window should say.
pub fn apply(
    shared: &SharedRef,
    settings: &ShareSettings,
    on: bool,
) -> Result<Vec<String>, String> {
    let mut sh = shared.sharing.borrow_mut();
    if !on {
        sh.stop();
        return Ok(Vec::new());
    }
    let root = shared.lib.borrow().root.clone();
    let data = crate::config::stats_path()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(std::env::temp_dir);
    sh.start(settings, &root, &data)
        .map_err(|e| format!("{e:#}"))
}

pub fn show(parent: &adw::ApplicationWindow, shared: &SharedRef) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&tr("Share Library"));
    dialog.set_content_width(460);
    dialog.set_content_height(560);

    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.set_description(Some(
        "Other apps and e-readers can browse and download your books over OPDS: KOReader, Moon+ Reader, Librera, Kybook and more.",
    ));
    let cfg = shared.cfg.borrow().share.clone();

    let on = adw::SwitchRow::builder()
        .title(tr("Share this library"))
        .build();
    on.set_active(shared.sharing.borrow().running());
    let lan = adw::SwitchRow::builder()
        .title(tr("Allow other devices on my network"))
        .subtitle(trf(
            "Needs a password. Only on networks you trust. {}",
            &[&crate::secrets::caption_now()],
        ))
        .build();
    lan.set_active(cfg.lan);
    let password = adw::PasswordEntryRow::new();
    password.set_title(&tr("Password (user name: bookx)"));
    password.set_text(&cfg.password);
    password.set_visible(cfg.lan);
    let port = adw::SpinRow::with_range(1024.0, 65535.0, 1.0);
    port.set_title(&tr("Port"));
    port.set_value(if cfg.port == 0 {
        8080.0
    } else {
        cfg.port as f64
    });
    let kosync = adw::SwitchRow::builder()
        .title(tr("KOReader progress sync"))
        .subtitle(crate::sharing::KOSYNC_SUBTITLE)
        .build();
    kosync.add_suffix(&super::help_button(crate::sharing::KOSYNC_HELP));
    kosync.set_active(cfg.kosync);
    for r in [
        on.upcast_ref::<gtk::Widget>(),
        lan.upcast_ref(),
        password.upcast_ref(),
        port.upcast_ref(),
        kosync.upcast_ref(),
    ] {
        group.add(r);
    }
    page.add(&group);

    let tls_group = adw::PreferencesGroup::new();
    tls_group.set_title(&tr("Encryption (HTTPS)"));
    tls_group.set_description(Some(
        "Without your own certificate Bookx makes a self-signed one: the connection is encrypted, but browsers and readers warn until you trust it.",
    ));
    let https = adw::SwitchRow::builder()
        .title(tr("Use HTTPS"))
        .subtitle(tr("Encrypts the connection, including the password"))
        .build();
    https.set_active(cfg.https);
    let cert = adw::EntryRow::builder()
        .title(tr("Certificate file (PEM, optional)"))
        .build();
    cert.set_text(&cfg.cert_file);
    let key = adw::EntryRow::builder()
        .title(tr("Private key file (PEM, optional)"))
        .build();
    key.set_text(&cfg.key_file);
    for (row, what) in [(&cert, "certificate"), (&key, "private key")] {
        let pick = gtk::Button::from_icon_name("document-open-symbolic");
        pick.set_valign(gtk::Align::Center);
        pick.add_css_class("flat");
        pick.set_tooltip_text(Some(&trf("Choose the {} file", &[&what])));
        let target = row.clone();
        let dialog_parent = parent.clone();
        pick.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::builder()
                .title(trf("Choose the {} file", &[&what]))
                .build();
            let row = target.clone();
            dialog.open(
                Some(&dialog_parent),
                relm4::gtk::gio::Cancellable::NONE,
                move |res| {
                    if let Some(p) = res.ok().and_then(|f| f.path()) {
                        row.set_text(&p.to_string_lossy());
                        row.emit_by_name::<()>("apply", &[]);
                    }
                },
            );
        });
        row.add_suffix(&pick);
    }
    let generate = adw::ActionRow::builder()
        .title(tr("Make a new self-signed certificate"))
        .subtitle(tr("Used when no certificate file is chosen. Clients that trusted the old one must trust the new one."))
        .activatable(true)
        .build();
    generate.add_suffix(&gtk::Image::from_icon_name("view-refresh-symbolic"));
    for r in [
        https.upcast_ref::<gtk::Widget>(),
        cert.upcast_ref(),
        key.upcast_ref(),
        generate.upcast_ref(),
    ] {
        tls_group.add(r);
    }
    cert.set_visible(cfg.https);
    key.set_visible(cfg.https);
    generate.set_visible(cfg.https);
    page.add(&tls_group);

    let status = adw::PreferencesGroup::new();
    let status_label = gtk::Label::new(None);
    status_label.set_wrap(true);
    status_label.set_selectable(true);
    status_label.set_xalign(0.0);
    status.add(&status_label);
    let copy = gtk::Button::with_label(&tr("Copy address"));
    copy.add_css_class("flat");
    copy.set_halign(gtk::Align::Start);
    copy.set_visible(false);
    status.add(&copy);
    page.add(&status);

    // the address to type into KOReader, shown while the switch is on
    let ko_group = adw::PreferencesGroup::new();
    ko_group.set_title(&tr("KOReader progress sync"));
    ko_group.set_description(Some(
        "In KOReader: Tools → Progress sync → Custom sync server, type this address, then Register with a user name and password of your choice (not the Share password). Only the reading position is synced, not highlights or notes.",
    ));
    let ko_url = gtk::Label::new(None);
    ko_url.set_wrap(true);
    ko_url.set_selectable(true);
    ko_url.set_xalign(0.0);
    ko_url.add_css_class("monospace");
    let ko_copy = gtk::Button::with_label(&tr("Copy KOReader address"));
    ko_copy.add_css_class("flat");
    ko_copy.set_halign(gtk::Align::Start);
    ko_group.add(&ko_url);
    ko_group.add(&ko_copy);
    ko_group.set_visible(cfg.kosync);
    page.add(&ko_group);
    KO.with(|k| *k.borrow_mut() = Some((ko_group.clone(), ko_url.clone())));
    {
        let u = ko_url.clone();
        ko_copy.connect_clicked(move |b| b.clipboard().set_text(&u.text()));
    }
    let show_ko_url = {
        let (ko_group, ko_url, ko_copy, kosync) = (
            ko_group.clone(),
            ko_url.clone(),
            ko_copy.clone(),
            kosync.clone(),
        );
        std::rc::Rc::new(move |urls: &[String]| {
            ko_group.set_visible(kosync.is_active());
            match crate::sharing::kosync_url(urls) {
                Some(u) => {
                    ko_url.set_label(&u);
                    ko_copy.set_sensitive(true);
                }
                None => {
                    ko_url.set_label(&tr("Turn on “Share this library” to get the address."));
                    ko_copy.set_sensitive(false);
                }
            }
        })
    };

    let urls = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let refresh = {
        let show_ko_url2 = show_ko_url.clone();
        let (shared, on, lan, password, port, kosync, status_label, copy, urls) = (
            shared.clone(),
            on.clone(),
            lan.clone(),
            password.clone(),
            port.clone(),
            kosync.clone(),
            status_label.clone(),
            copy.clone(),
            urls.clone(),
        );
        let (https, cert, key, generate) =
            (https.clone(), cert.clone(), key.clone(), generate.clone());
        let busy = std::rc::Rc::new(std::cell::Cell::new(false));
        std::rc::Rc::new(move || {
            if busy.replace(true) {
                return;
            }
            password.set_visible(lan.is_active());
            for w in [&cert, &key] {
                w.set_visible(https.is_active());
            }
            generate.set_visible(https.is_active());
            let settings = ShareSettings {
                port: port.value() as u16,
                lan: lan.is_active(),
                password: password.text().to_string(),
                kosync: kosync.is_active(),
                https: https.is_active(),
                cert_file: cert.text().trim().to_string(),
                key_file: key.text().trim().to_string(),
            };
            {
                let mut c = shared.cfg.borrow_mut();
                c.share = settings.clone();
                c.save();
            }
            match apply(&shared, &settings, on.is_active()) {
                Ok(u) => {
                    if u.is_empty() {
                        status_label.set_label(&tr("Sharing is off."));
                        copy.set_visible(false);
                    } else {
                        status_label.set_label(&trf("Sharing at:\n{}", &[&u.join("\n")]));
                        copy.set_visible(true);
                    }
                    show_ko_url2(&u);
                    *urls.borrow_mut() = u;
                }
                Err(e) => {
                    on.set_active(false);
                    show_ko_url2(&[]);
                    status_label.set_label(&e);
                    copy.set_visible(false);
                }
            }
            busy.set(false);
        })
    };
    {
        let u = urls.clone();
        let c = copy.clone();
        copy.connect_clicked(move |b| {
            if let Some(first) = u.borrow().last() {
                b.clipboard().set_text(first);
                let _ = &c;
            }
        });
    }
    // show the current state
    {
        let u = shared.sharing.borrow().urls();
        if u.is_empty() {
            status_label.set_label(&tr("Sharing is off."));
        } else {
            status_label.set_label(&trf("Sharing at:\n{}", &[&u.join("\n")]));
            copy.set_visible(true);
        }
        show_ko_url(&u);
        *urls.borrow_mut() = u;
    }
    for sw in [&on, &lan, &kosync, &https] {
        let r = refresh.clone();
        sw.connect_active_notify(move |_| r());
    }
    {
        let r = refresh.clone();
        password.connect_apply(move |_| r());
        let r = refresh.clone();
        password.connect_entry_activated(move |_| r());
        let r = refresh.clone();
        port.connect_value_notify(move |_| r());
        for e in [&cert, &key] {
            let r = refresh.clone();
            e.connect_apply(move |_| r());
            let r = refresh.clone();
            e.connect_entry_activated(move |_| r());
        }
        let (shared2, status2, r) = (shared.clone(), status_label.clone(), refresh.clone());
        let (lan2, https2) = (lan.clone(), https.clone());
        generate.connect_activated(move |_| {
            let settings = {
                let mut s = shared2.cfg.borrow().share.clone();
                s.lan = lan2.is_active();
                s.https = https2.is_active();
                s
            };
            let data = crate::config::stats_path()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                .unwrap_or_else(std::env::temp_dir);
            match crate::sharing::generate_certificate(&settings, &data) {
                Ok(_) => r(),
                Err(e) => status2.set_label(&trf(
                    "Could not make a certificate: {}",
                    &[&format!("{:#}", e)],
                )),
            }
        });
    }

    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    dialog.set_child(Some(&view));
    dialog.present(Some(parent));
}
