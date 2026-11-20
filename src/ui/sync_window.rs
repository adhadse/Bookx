//! The Sync window of the main menu: a task-oriented first screen. A status card says whether Bookx
//! sync is off, ready or connected, with one big button (“Set up Bookx sync” or “Sync now”); three
//! choice cards say what to use when (Bookx sync with KOReader over Wi-Fi, a USB device, sharing
//! books over OPDS) with one button each; everything else is under a collapsed “Advanced” row and
//! in Preferences → Sync. The settings themselves live in Preferences → Sync; both follow each other
//! over the sync settings bus (`settings_bus::sync`), so nothing is switched in two places. See
//! `docs/sync-ux-notes.md`.

use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::{gtk, gtk::glib};

use super::share::{self, follow};
use super::stepper::Stepper;
use super::{sync_prefs, SharedRef};
use crate::i18n::tr;
use crate::sync_model::{self, Snapshot};

struct Win {
    dialog: adw::Dialog,
    root: gtk::Widget,
    headline: gtk::Label,
    primary: gtk::Button,
    cards: Vec<(&'static str, gtk::Button, gtk::Label)>,
    account_hint: gtk::Label,
    stepper: Option<Rc<Stepper>>,
    advanced: adw::ExpanderRow,
    result: gtk::Label,
    refresh: Rc<dyn Fn()>,
}

thread_local! {
    static WIN: RefCell<Option<Win>> = const { RefCell::new(None) };
}

fn card_box() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.add_css_class("card");
    b
}

fn padded(inner: &gtk::Box) -> gtk::Box {
    // `.card` draws the frame; the padding is a box inside
    let outer = card_box();
    inner.set_margin_top(14);
    inner.set_margin_bottom(14);
    inner.set_margin_start(16);
    inner.set_margin_end(16);
    outer.append(inner);
    outer
}

fn wrapped(text: &str, class: Option<&str>) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_wrap(true);
    l.set_xalign(0.0);
    l.set_hexpand(true);
    if let Some(c) = class {
        l.add_css_class(c);
    }
    l
}

fn button(label: &str, name: &str) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    super::audit::label(&b, name);
    b
}

pub fn show(parent: &adw::ApplicationWindow, shared: &SharedRef) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&tr(sync_model::TITLE));
    dialog.set_content_width(520);
    dialog.set_content_height(760);
    let snap = sync_prefs::current(shared);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 14);
    column.set_margin_top(12);
    column.set_margin_bottom(18);
    column.set_margin_start(14);
    column.set_margin_end(14);

    // ---- the status card ----------------------------------------------------------------
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let headline = wrapped(&snap.readiness().headline(), Some("title-2"));
    let sentence = wrapped(&tr(sync_model::WINDOW_SENTENCE), None);
    let primary = button(&snap.primary(), &tr("Main action of Bookx sync"));
    primary.add_css_class("suggested-action");
    primary.add_css_class("pill");
    primary.set_valign(gtk::Align::Center);
    let link = button(
        &tr("Open Preferences → Sync"),
        &tr("Open Preferences → Sync"),
    );
    link.add_css_class("flat");
    link.set_halign(gtk::Align::Start);
    let result = wrapped("", Some("dim-label"));
    result.set_visible(false);
    result.set_selectable(true);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.append(&primary);
    actions.append(&link);
    for w in [
        headline.upcast_ref::<gtk::Widget>(),
        sentence.upcast_ref(),
        actions.upcast_ref(),
        result.upcast_ref(),
    ] {
        inner.append(w);
    }
    column.append(&padded(&inner));

    // ---- the three choice cards ---------------------------------------------------------
    let mut cards: Vec<(&'static str, gtk::Button, gtk::Label)> = Vec::new();
    let mut wifi_stepper: Option<Rc<Stepper>> = None;
    let mut message: Option<gtk::Label> = None;
    let mut account_hint = gtk::Label::new(None);
    for card in snap.cards() {
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        top.append(&wrapped(&card.title, Some("title-4")));
        let status = wrapped(&card.status, Some("heading"));
        status.set_hexpand(false);
        status.set_xalign(1.0);
        status.set_halign(gtk::Align::End);
        top.append(&status);
        inner.append(&top);
        inner.append(&wrapped(&card.use_when, Some("dim-label")));
        if card.id == "wifi" {
            // the four steps of the setup as a compact stepper; every step opens to its text
            let open_prefs = |help: &str, with_button: bool| -> gtk::Widget {
                let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
                b.append(&wrapped(&tr(help), Some("dim-label")));
                if with_button {
                    let btn = button(
                        &tr("Open Preferences → Sync"),
                        &tr("Open Preferences → Sync"),
                    );
                    btn.set_halign(gtk::Align::Start);
                    btn.connect_clicked(|b| {
                        let _ = b.activate_action("app.sync-prefs", None);
                    });
                    b.append(&btn);
                }
                b.upcast()
            };
            let st = Stepper::new(
                vec![
                    (
                        tr(sync_model::STEP_SERVER),
                        open_prefs(sync_model::STEP_SERVER_HELP, true),
                    ),
                    (
                        tr(sync_model::STEP_ACCOUNT),
                        open_prefs(sync_model::STEP_ACCOUNT_HELP, true),
                    ),
                    (
                        tr(sync_model::STEP_PLUGIN),
                        open_prefs(sync_model::STEP_PLUGIN_HELP, true),
                    ),
                    (
                        tr(sync_model::STEP_TEST),
                        open_prefs(sync_model::STEP_TEST_HELP, false),
                    ),
                ],
                true,
            );
            st.close_all();
            inner.append(st.widget());
            wifi_stepper = Some(st);
            inner.append(&wrapped(&tr(sync_model::STEP_TEST_HELP), Some("dim-label")));
            account_hint = wrapped(&tr(sync_model::ACCOUNT_ONE_LINER), Some("dim-label"));
            inner.append(&account_hint);
        }
        if card.id == "opds" {
            let m = wrapped("", Some("error"));
            m.set_visible(false);
            inner.append(&m);
            message = Some(m);
        }
        let b = button(&card.button, &format!("{}: {}", card.button, card.title));
        b.set_halign(gtk::Align::Start);
        b.set_margin_top(4);
        inner.append(&b);
        column.append(&padded(&inner));
        cards.push((card.id, b, status));
    }
    let message = message.unwrap_or_default();

    // ---- advanced -----------------------------------------------------------------------
    let adv_group = adw::PreferencesGroup::new();
    let advanced = adw::ExpanderRow::new();
    advanced.set_title(&tr("Advanced"));
    advanced.set_subtitle(&tr("Reverse proxy, port and firewall, every setting"));
    super::audit::label(&advanced, &tr("Advanced"));
    let proxy = adw::ActionRow::builder()
        .title(tr("Reverse proxy and remote access"))
        .subtitle(tr(
            "Put nginx or Caddy in front of Bookx for HTTPS on your own address.",
        ))
        .build();
    proxy.set_subtitle_lines(0);
    let proxy_btn = button(&tr("Open guide"), &tr("Open the reverse proxy guide"));
    proxy_btn.set_valign(gtk::Align::Center);
    proxy.add_suffix(&proxy_btn);
    // the port and the firewall hint are the text of the row that opens every setting
    let prefs_row = adw::ActionRow::builder().title(tr("Every setting")).build();
    prefs_row.set_subtitle_lines(0);
    let prefs_btn = button(
        &tr("Open Preferences → Sync"),
        &tr("Open Preferences → Sync"),
    );
    prefs_btn.set_valign(gtk::Align::Center);
    prefs_row.add_suffix(&prefs_btn);
    for r in [&proxy, &prefs_row] {
        advanced.add_row(r);
    }
    adv_group.add(&advanced);
    column.append(&adv_group);

    // ---- state ----------------------------------------------------------------------------
    let refresh = {
        let (shared, headline, primary, cards, wifi_stepper, prefs_row, account_hint) = (
            shared.clone(),
            headline.clone(),
            primary.clone(),
            cards.clone(),
            wifi_stepper.clone(),
            prefs_row.clone(),
            account_hint.clone(),
        );
        Rc::new(move || {
            let snap: Snapshot = sync_prefs::current(&shared);
            headline.set_label(&snap.readiness().headline());
            primary.set_label(&snap.primary());
            for (card, (_, b, status)) in snap.cards().iter().zip(&cards) {
                status.set_label(&card.status);
                b.set_label(&card.button);
            }
            if let Some(st) = &wifi_stepper {
                st.update(&sync_prefs::step_infos(&snap));
            }
            let cfg = shared.cfg.borrow().share.clone();
            prefs_row.set_subtitle(&crate::i18n::trf(
                "Port {}: allow it in your firewall for the e-reader to connect. Server address, HTTPS, matching, privacy and offline mode are in Preferences → Sync.",
                &[&(if cfg.port == 0 { 8080 } else { cfg.port })],
            ));
            account_hint.set_visible(!snap.account);
        })
    };
    refresh();
    follow(&column, {
        let r = refresh.clone();
        move || r()
    });
    let weak = column.downgrade();
    let timer_refresh = refresh.clone();
    glib::timeout_add_seconds_local(3, move || match weak.upgrade() {
        Some(c) => {
            if c.is_mapped() {
                timer_refresh();
            }
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });

    // ---- actions --------------------------------------------------------------------------
    {
        let (sh, result) = (shared.clone(), result.clone());
        primary.connect_clicked(move |b| {
            if sync_prefs::current(&sh).primary_syncs() {
                let text = match crate::annotation_sync_ui::sync_once(&sh, true) {
                    Some(r) => crate::i18n::trf("Last sync: {}", &[&r.text()]),
                    None => tr("Nothing to merge yet. Turn on “Sync highlights, notes and bookmarks” in Preferences → Sync. Reading position syncs by itself as you read."),
                };
                result.set_label(&text);
                result.set_visible(true);
            } else {
                let _ = b.activate_action("app.sync-prefs", None);
            }
        });
        for b in [&link, &prefs_btn] {
            b.connect_clicked(|b| {
                let _ = b.activate_action("app.sync-prefs", None);
            });
        }
        proxy_btn.connect_clicked(|b| {
            let uri = "https://bookx.adhadse.com/reverse-proxy/".to_variant();
            let _ = b.activate_action("app.open-uri", Some(&uri));
        });
        for (id, b, _) in &cards {
            let (sh, msg) = (shared.clone(), message.clone());
            match *id {
                "wifi" => b.connect_clicked(|b| {
                    let _ = b.activate_action("app.sync-prefs", None);
                }),
                "usb" => b.connect_clicked(|b| {
                    let _ = b.activate_action("app.devices", None);
                }),
                _ => b.connect_clicked(move |_| {
                    let on = !sh.sharing.borrow().running();
                    match share::set_server(&sh, on) {
                        Ok(_) => msg.set_visible(false),
                        Err(e) => {
                            msg.set_label(&e);
                            msg.set_visible(true);
                        }
                    }
                }),
            };
        }
    }

    let scroll = gtk::ScrolledWindow::builder()
        .child(&column)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    let clamp = adw::Clamp::builder()
        .maximum_size(640)
        .child(&scroll)
        .build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&clamp));
    dialog.set_child(Some(&toolbar));
    WIN.with(|w| {
        *w.borrow_mut() = Some(Win {
            dialog: dialog.clone(),
            root: column.clone().upcast(),
            headline,
            primary,
            cards,
            account_hint: account_hint.clone(),
            stepper: wifi_stepper.clone(),
            advanced,
            result,
            refresh: refresh.clone(),
        })
    });
    dialog.connect_closed(|_| WIN.with(|w| *w.borrow_mut() = None));
    dialog.present(Some(parent));
}

fn texts(w: &gtk::Widget, out: &mut Vec<String>) {
    if let Some(l) = w.downcast_ref::<gtk::Label>() {
        out.push(l.text().to_string());
    }
    if let Some(r) = w.downcast_ref::<adw::PreferencesRow>() {
        out.push(r.title().to_string());
    }
    if let Some(r) = w.downcast_ref::<adw::ActionRow>() {
        out.push(r.subtitle().unwrap_or_default().to_string());
    }
    if let Some(b) = w.downcast_ref::<gtk::Button>() {
        if let Some(l) = b.label() {
            out.push(l.to_string());
        }
    }
    let mut c = w.first_child();
    while let Some(ch) = c {
        texts(&ch, out);
        c = ch.next_sibling();
    }
}

/// Dev hook `sync:…`: `dump`, `primary` (press the big button), `card=wifi|usb|opds` (press that
/// card’s button), `advanced=open|close`, `close`.
pub fn dev(arg: &str, shared: &SharedRef) {
    if arg == "close" {
        let d = WIN.with(|w| w.borrow().as_ref().map(|w| w.dialog.clone()));
        if let Some(d) = d {
            d.close();
        }
        return;
    }
    // pressing a button may open a dialog or change the state: no borrow while it runs
    let target = WIN.with(|w| {
        let w = w.borrow();
        let win = w.as_ref()?;
        if arg == "primary" {
            return Some(win.primary.clone());
        }
        arg.strip_prefix("card=")
            .and_then(|id| win.cards.iter().find(|c| c.0 == id).map(|c| c.1.clone()))
    });
    if let Some(b) = target {
        b.emit_clicked();
        return;
    }
    WIN.with(|w| {
        let w = w.borrow();
        let Some(win) = &*w else {
            eprintln!("sync_window: not open");
            return;
        };
        if let Some(v) = arg.strip_prefix("advanced=") {
            win.advanced.set_expanded(v != "close");
        } else if let Some(i) = arg.strip_prefix("step=") {
            if let (Some(st), Ok(i)) = (&win.stepper, i.parse::<usize>()) {
                st.pick(i.saturating_sub(1));
            }
        } else if arg == "dump" {
            (win.refresh)();
            let mut all = Vec::new();
            texts(&win.root, &mut all);
            let joined = all.join("\n");
            let cards: Vec<String> = win
                .cards
                .iter()
                .map(|(id, b, s)| format!("{id}:{}:{}", b.label().unwrap_or_default(), s.text()))
                .collect();
            eprintln!(
                "sync_window: title={} headline={:?} primary={:?} cards={} use_when={} sentence={} kosync_labels={} advanced={} account_hint={} result={:?} stepper=[{}] {}",
                win.dialog.title(),
                win.headline.text(),
                win.primary.label().unwrap_or_default(),
                cards.join("|"),
                all.iter().filter(|t| t.starts_with("Use this ")).count(),
                joined.contains("Use USB for devices without KOReader"),
                all.iter().filter(|t| t.contains("KOSync")).count(),
                if win.advanced.is_expanded() { "open" } else { "collapsed" },
                win.account_hint.is_visible(),
                if win.result.is_visible() { win.result.text().to_string() } else { String::new() },
                win.stepper.as_ref().map(|s| s.dump()).unwrap_or_default(),
                sync_prefs::current(shared).dump()
            );
        }
    });
}
