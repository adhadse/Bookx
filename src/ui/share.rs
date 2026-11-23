//! The OPDS server behind Sync: OPDS, the web page and Bookx sync. The rows live in Preferences
//! → Sync (`step_server_rows`, `opds_rows`, `advanced_rows`); the Sync window (`sync_window`)
//! starts and stops the same server through [`set_server`], and both follow each other over the
//! sync settings bus.

use crate::i18n::{tr, trf};
use relm4::adw::{self, prelude::*};
use relm4::gtk;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::SharedRef;
use crate::sharing::ShareSettings;

thread_local! {
    /// The address row of step 1 in Preferences → Sync (the UI tests read it).
    static KO_PREFS: RefCell<Option<adw::ActionRow>> = const { RefCell::new(None) };
    /// The Bookx sync switch (step 1), the OPDS server switch and the port of Preferences → Sync.
    static SYNC_SW: RefCell<Option<adw::SwitchRow>> = const { RefCell::new(None) };
    static OPDS_SW: RefCell<Option<adw::SwitchRow>> = const { RefCell::new(None) };
    static PORT_SP: RefCell<Option<adw::SpinRow>> = const { RefCell::new(None) };
    /// Why the server could not start, until the next change.
    static LAST_ERROR: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Dev hook `prefs:sync-server=on|off` (Bookx sync, step 1), `prefs:sync-opds=on|off`,
/// `prefs:sync-port=N`, `prefs:sync-dump`: the rows of Preferences → Sync.
pub fn dev_prefs(cmd: &str, shared: Option<&SharedRef>) {
    let on = |v: &str| v != "off";
    if let Some(v) = cmd.strip_prefix("server=") {
        SYNC_SW.with(|s| s.borrow().as_ref().map(|s| s.set_active(on(v))));
    } else if let Some(v) = cmd.strip_prefix("opds=") {
        OPDS_SW.with(|s| s.borrow().as_ref().map(|s| s.set_active(on(v))));
    } else if let Some(v) = cmd.strip_prefix("port=") {
        PORT_SP.with(|s| {
            s.borrow()
                .as_ref()
                .map(|s| s.set_value(v.parse().unwrap_or(8080.0)))
        });
    } else if cmd == "ko" {
        dev_dump_prefs();
    } else if cmd == "dump" {
        let snap = shared.map(super::sync_prefs::current).unwrap_or_default();
        let sync = SYNC_SW.with(|s| s.borrow().as_ref().map(|s| s.is_active()));
        let opds = OPDS_SW.with(|s| s.borrow().as_ref().map(|s| s.is_active()));
        match (sync, opds) {
            (Some(a), Some(b)) => eprintln!("prefs_sync: server={a} opds={b} {}", snap.dump()),
            _ => eprintln!("prefs_sync: no page"),
        }
    }
}

/// Test hook `share:kodump`: what the server says to type into KOReader (from the state, not a
/// window).
pub fn dev_dump(shared: &SharedRef) {
    let urls = shared.sharing.borrow().urls();
    let on = shared.cfg.borrow().share.kosync;
    eprintln!(
        "share_ko: visible={on} address={}",
        crate::sharing::kosync_url(&urls).unwrap_or_default()
    );
}

/// Test hook: the address row of step 1 in Preferences → Sync.
pub fn dev_dump_prefs() {
    KO_PREFS.with(|k| match &*k.borrow() {
        Some(r) => eprintln!(
            "prefs_share_ko: visible={} address={}",
            r.is_visible(),
            r.subtitle().unwrap_or_default()
        ),
        None => eprintln!("prefs_share_ko: no page"),
    });
}

fn stats_dir() -> std::path::PathBuf {
    crate::config::stats_path()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(std::env::temp_dir)
}

/// Starts or stops the server according to `settings`. Returns what the window should say. Every
/// view of the server (Preferences, the Sync window) is told afterwards.
pub fn apply(
    shared: &SharedRef,
    settings: &ShareSettings,
    on: bool,
) -> Result<Vec<String>, String> {
    let r = {
        let mut sh = shared.sharing.borrow_mut();
        if !on {
            sh.stop();
            Ok(Vec::new())
        } else {
            let root = shared.lib.borrow().root.clone();
            sh.start(settings, &root, &stats_dir())
                .map_err(|e| format!("{e:#}"))
        }
    };
    match &r {
        Ok(_) if on => tracing::info!(target: "bookx_crumb", "sync server started"),
        Ok(_) => tracing::info!(target: "bookx_crumb", "sync server stopped"),
        Err(e) => tracing::warn!("sync server could not start: {e}"),
    }
    {
        let mut t = crate::trace::event(crate::trace::Area::Server, if on { "start" } else { "stop" });
        t.num("port", settings.port as u64).flag("https", settings.https).flag("lan", settings.lan).flag("kosync", settings.kosync);
        match &r {
            Ok(_) => t.ok(),
            Err(e) => t.fail(e),
        }
    }
    LAST_ERROR.with(|e| *e.borrow_mut() = r.as_ref().err().cloned().unwrap_or_default());
    crate::settings_bus::sync().notify();
    r
}

/// Why the server could not start (empty when it did, or was not asked to).
pub fn last_error() -> String {
    LAST_ERROR.with(|e| e.borrow().clone())
}

/// Step 1 of the setup: the server on, answering Bookx sync, reachable from the e-reader. Needs a
/// password for the library pages when other devices connect; one is made when there is none.
pub fn enable_bookx_sync(shared: &SharedRef) -> Result<Vec<String>, String> {
    {
        let mut c = shared.cfg.borrow_mut();
        c.share.kosync = true;
        c.share.lan = true;
        if c.share.password.chars().count() < crate::sharing::MIN_PASSWORD {
            c.share.password = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
        }
        c.save();
    }
    set_server(shared, true)
}

/// Restarts the running server with the saved settings (nothing happens while it is off).
pub fn reapply(shared: &SharedRef) -> Option<Result<Vec<String>, String>> {
    if !shared.sharing.borrow().running() {
        crate::settings_bus::sync().notify();
        return None;
    }
    let s = shared.cfg.borrow().share.clone();
    Some(apply(shared, &s, true))
}

/// Switches the server on or off with the saved settings.
pub fn set_server(shared: &SharedRef, on: bool) -> Result<Vec<String>, String> {
    let s = shared.cfg.borrow().share.clone();
    let r = apply(shared, &s, on);
    let changed = shared.cfg.borrow_mut().share.remember(on, r.is_ok());
    if changed {
        shared.cfg.borrow().save();
    }
    r
}

/// At launch: starts the server again when it was on when Bookx last ran. A failure (the port is
/// taken, the password is missing) is shown on the Sync page and keeps the wish for the next launch.
pub fn restore(shared: &SharedRef) {
    let s = shared.cfg.borrow().share.clone();
    if !s.restore_at_launch() {
        return;
    }
    if let Err(e) = apply(shared, &s, true) {
        tracing::warn!("the sync server could not be started again: {e}");
    }
}

/// Keeps `f` called on every change of the sync settings while `w` exists.
pub fn follow(w: &impl IsA<gtk::Widget>, f: impl Fn() + 'static) {
    let sub = RefCell::new(Some(crate::settings_bus::sync().subscribe(f)));
    w.connect_destroy(move |_| {
        sub.borrow_mut().take();
    });
}

fn window_of(w: &gtk::Widget) -> Option<gtk::Window> {
    w.root().and_then(|r| r.downcast::<gtk::Window>().ok())
}

fn copy_button(label: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name("edit-copy-symbolic");
    b.set_valign(gtk::Align::Center);
    b.add_css_class("flat");
    b.set_tooltip_text(Some(label));
    super::audit::label(&b, label);
    b
}

/// An address row: the text to copy, and a copy button (it also follows the bus through `view`).
fn address_row(title: &str) -> (adw::ActionRow, gtk::Button) {
    let r = adw::ActionRow::builder().title(title).build();
    r.set_subtitle_lines(0);
    r.set_subtitle_selectable(true);
    r.add_css_class("monospace-subtitle");
    let b = copy_button(&tr("Copy address"));
    r.add_suffix(&b);
    (r, b)
}

/// A line that says why the server could not start; hidden while there is nothing to say. (A plain
/// label under the rows: it is feedback for the step, not a row of its own.)
pub fn error_label() -> gtk::Label {
    let l = gtk::Label::new(None);
    l.set_wrap(true);
    l.set_xalign(0.0);
    l.set_selectable(true);
    l.add_css_class("error");
    l.set_visible(false);
    let ll = l.downgrade();
    let show = move || {
        if let Some(l) = ll.upgrade() {
            let e = last_error();
            l.set_visible(!e.is_empty());
            l.set_label(&e);
        }
    };
    follow(&l, show);
    l
}

/// Step 1 of the setup: the Bookx sync switch and the address to type on the e-reader.
pub fn step_server_rows(shared: &SharedRef) -> Vec<gtk::Widget> {
    let sw = adw::SwitchRow::builder()
        .title(tr("Bookx sync server"))
        .build();
    let (addr, copy) = address_row(&tr("Address for the e-reader"));
    addr.add_suffix(&super::qr_dialog::button(tr("Address for the e-reader"), {
        let shared = shared.clone();
        move || crate::sharing::kosync_url(&shared.sharing.borrow().urls())
    }));
    let quiet = Rc::new(Cell::new(false));
    let view = {
        let (shared, sw, addr, copy, quiet) = (
            shared.clone(),
            sw.clone(),
            addr.clone(),
            copy.clone(),
            quiet.clone(),
        );
        Rc::new(move || {
            quiet.set(true);
            let urls = shared.sharing.borrow().urls();
            let on = shared.sharing.borrow().running() && shared.cfg.borrow().share.kosync;
            if sw.is_active() != on {
                sw.set_active(on);
            }
            match crate::sharing::kosync_url(&urls).filter(|_| on) {
                Some(u) => {
                    addr.set_subtitle(&relm4::gtk::glib::markup_escape_text(&u));
                    copy.set_sensitive(true);
                }
                None => {
                    addr.set_subtitle(&tr("Appears here when the server is on."));
                    copy.set_sensitive(false);
                }
            }
            quiet.set(false);
        })
    };
    view();
    follow(&sw, {
        let v = view.clone();
        move || v()
    });
    {
        let (shared1, quiet) = (shared.clone(), quiet.clone());
        sw.connect_active_notify(move |s| {
            let shared = shared1.clone();
            if quiet.get() {
                return;
            }
            let running = shared.sharing.borrow().running() && shared.cfg.borrow().share.kosync;
            if s.is_active() == running {
                return;
            }
            let _ = if s.is_active() {
                enable_bookx_sync(&shared)
            } else {
                set_server(&shared, false)
            };
        });
        let shared = shared.clone();
        copy.connect_clicked(move |b| {
            if let Some(u) = crate::sharing::kosync_url(&shared.sharing.borrow().urls()) {
                b.clipboard().set_text(&u);
            }
        });
    }
    SYNC_SW.with(|s| *s.borrow_mut() = Some(sw.clone()));
    KO_PREFS.with(|k| *k.borrow_mut() = Some(addr.clone()));
    vec![sw.upcast(), addr.upcast(), error_label().upcast()]
}

/// The OPDS server (catalogue and web page) switch with its address.
pub fn opds_rows(shared: &SharedRef) -> Vec<gtk::Widget> {
    let sw = adw::SwitchRow::builder()
        .title(tr("OPDS server"))
        .subtitle(tr(
            "OPDS catalogue and web page for other apps and e-readers. Off until you switch it on.",
        ))
        .build();
    let (addr, copy) = address_row(&tr("Address"));
    addr.add_suffix(&super::qr_dialog::button(tr("OPDS catalogue address"), {
        let shared = shared.clone();
        move || {
            let urls = shared.sharing.borrow().urls();
            urls.last()
                .map(|u| format!("{}/opds", u.trim_end_matches('/')))
        }
    }));
    let quiet = Rc::new(Cell::new(false));
    let view = {
        let (shared, sw, addr, copy, quiet) = (
            shared.clone(),
            sw.clone(),
            addr.clone(),
            copy.clone(),
            quiet.clone(),
        );
        Rc::new(move || {
            quiet.set(true);
            let urls = shared.sharing.borrow().urls();
            let running = shared.sharing.borrow().running();
            if sw.is_active() != running {
                sw.set_active(running);
            }
            if urls.is_empty() {
                addr.set_subtitle(&tr("Sharing is off."));
                copy.set_sensitive(false);
            } else {
                addr.set_subtitle(&relm4::gtk::glib::markup_escape_text(&urls.join("\n")));
                copy.set_sensitive(true);
            }
            quiet.set(false);
        })
    };
    view();
    follow(&sw, {
        let v = view.clone();
        move || v()
    });
    {
        let (shared1, quiet) = (shared.clone(), quiet.clone());
        sw.connect_active_notify(move |s| {
            let shared = shared1.clone();
            if quiet.get() || shared.sharing.borrow().running() == s.is_active() {
                return;
            }
            let _ = set_server(&shared, s.is_active());
        });
        let shared = shared.clone();
        copy.connect_clicked(move |b| {
            if let Some(u) = shared.sharing.borrow().urls().last() {
                b.clipboard().set_text(u);
            }
        });
    }
    OPDS_SW.with(|s| *s.borrow_mut() = Some(sw.clone()));
    vec![sw.upcast(), addr.upcast(), error_label().upcast()]
}

/// The technical server rows for Advanced: network access and password, port, HTTPS.
pub fn advanced_rows(parent: &gtk::Widget, shared: &SharedRef) -> Vec<gtk::Widget> {
    let cfg = shared.cfg.borrow().share.clone();
    let lan = adw::SwitchRow::builder()
        .title(tr("Allow other devices on the network"))
        .subtitle(trf(
            "Needs a password. Only on networks you trust. {}",
            &[&crate::secrets::caption_now()],
        ))
        .build();
    lan.set_active(cfg.lan);
    let password = adw::PasswordEntryRow::new();
    password.set_title(&tr("Library password (user name: bookx)"));
    password.set_text(&cfg.password);
    let port = adw::SpinRow::with_range(1024.0, 65535.0, 1.0);
    port.set_title(&tr("Port"));
    port.set_subtitle(&tr(
        "Allow this port in your firewall for the e-reader to connect.",
    ));
    port.set_value(if cfg.port == 0 {
        8080.0
    } else {
        cfg.port as f64
    });
    let https = adw::SwitchRow::builder()
        .title(tr("Use HTTPS"))
        .subtitle(tr("Encrypts the connection, including the password. Without your own certificate Bookx makes a self-signed one: browsers and readers warn until you trust it."))
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
        super::audit::label(&pick, &trf("Choose the {} file", &[&what]));
        let target = row.clone();
        let dialog_parent = parent.clone();
        pick.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::builder()
                .title(trf("Choose the {} file", &[&what]))
                .build();
            let row = target.clone();
            dialog.open(
                window_of(&dialog_parent).as_ref(),
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
    let error = error_label();
    let result = gtk::Label::new(None);
    result.set_wrap(true);
    result.set_xalign(0.0);
    result.add_css_class("error");
    result.set_visible(false);

    let quiet = Rc::new(Cell::new(false));
    let view = {
        let (shared, lan, password, port, https, cert, key, generate) = (
            shared.clone(),
            lan.clone(),
            password.clone(),
            port.clone(),
            https.clone(),
            cert.clone(),
            key.clone(),
            generate.clone(),
        );
        let quiet = quiet.clone();
        Rc::new(move || {
            quiet.set(true);
            let cfg = shared.cfg.borrow().share.clone();
            if lan.is_active() != cfg.lan {
                lan.set_active(cfg.lan);
            }
            if https.is_active() != cfg.https {
                https.set_active(cfg.https);
            }
            if password.text().as_str() != cfg.password {
                password.set_text(&cfg.password);
            }
            if cfg.port != 0 && port.value() as u16 != cfg.port {
                port.set_value(cfg.port as f64);
            }
            if cert.text().as_str() != cfg.cert_file {
                cert.set_text(&cfg.cert_file);
            }
            if key.text().as_str() != cfg.key_file {
                key.set_text(&cfg.key_file);
            }
            password.set_visible(lan.is_active());
            for w in [&cert, &key] {
                w.set_visible(https.is_active());
            }
            generate.set_visible(https.is_active());
            quiet.set(false);
        })
    };
    view();
    follow(&lan, {
        let v = view.clone();
        move || v()
    });
    let refresh = {
        let (shared, lan, password, port, https, cert, key) = (
            shared.clone(),
            lan.clone(),
            password.clone(),
            port.clone(),
            https.clone(),
            cert.clone(),
            key.clone(),
        );
        let (quiet, view) = (quiet.clone(), view.clone());
        Rc::new(move || {
            if quiet.get() {
                return;
            }
            {
                let mut c = shared.cfg.borrow_mut();
                c.share = ShareSettings {
                    port: port.value() as u16,
                    lan: lan.is_active(),
                    password: password.text().to_string(),
                    https: https.is_active(),
                    cert_file: cert.text().trim().to_string(),
                    key_file: key.text().trim().to_string(),
                    ..c.share.clone()
                };
                c.save();
            }
            // a running server restarts with the new settings; otherwise only the views follow
            if reapply(&shared).is_none() {
                view();
            }
        })
    };
    lan.connect_active_notify({
        let r = refresh.clone();
        move |_| r()
    });
    https.connect_active_notify({
        let r = refresh.clone();
        move |_| r()
    });
    port.connect_value_notify({
        let r = refresh.clone();
        move |_| r()
    });
    {
        let r = refresh.clone();
        password.connect_apply(move |_| r());
        let r = refresh.clone();
        password.connect_entry_activated(move |_| r());
        for e in [&cert, &key] {
            let r = refresh.clone();
            e.connect_apply(move |_| r());
            let r = refresh.clone();
            e.connect_entry_activated(move |_| r());
        }
        let (shared2, result2, r) = (shared.clone(), result.clone(), refresh.clone());
        let (lan2, https2) = (lan.clone(), https.clone());
        generate.connect_activated(move |_| {
            let settings = {
                let mut s = shared2.cfg.borrow().share.clone();
                s.lan = lan2.is_active();
                s.https = https2.is_active();
                s
            };
            match crate::sharing::generate_certificate(&settings, &stats_dir()) {
                Ok(_) => {
                    result2.set_visible(false);
                    r()
                }
                Err(e) => {
                    result2.set_visible(true);
                    result2.set_label(&trf(
                        "Could not make a certificate: {}",
                        &[&format!("{:#}", e)],
                    ));
                }
            }
        });
    }
    PORT_SP.with(|s| *s.borrow_mut() = Some(port.clone()));
    vec![
        lan.upcast(),
        password.upcast(),
        port.upcast(),
        https.upcast(),
        cert.upcast(),
        key.upcast(),
        generate.upcast(),
        result.upcast(),
        error.upcast(),
    ]
}
