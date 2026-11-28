//! Preferences → Sync: the configuration side of Sync. It starts with a numbered setup checklist
//! (server, account, plugin, test on the device), then what Bookx sync does, USB device sync and
//! the OPDS server; everything technical sits in one collapsed “Advanced” row, so every setting
//! stays reachable (`sync_model::GROUPS`, `docs/sync-ux-notes.md`). The Sync window of the main menu
//! shows the same state as task cards and follows these rows over the sync settings bus.

use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::{gtk, gtk::glib};

use super::SharedRef;
use super::preferences::{PrefCtx, register_group};
use super::share::follow;
use super::stepper::{StepInfo, Stepper};
use crate::i18n::{N_, tr};
use crate::sync_model::{self, Snapshot};

thread_local! {
    /// The shared state the open page was built for (the dev hooks read it).
    static SHARED: RefCell<Option<SharedRef>> = const { RefCell::new(None) };
    static ADVANCED: RefCell<Option<glib::WeakRef<adw::ExpanderRow>>> = const { RefCell::new(None) };
    static NOTES_SW: RefCell<Option<adw::SwitchRow>> = const { RefCell::new(None) };
    static STEPPER: RefCell<Option<Rc<Stepper>>> = const { RefCell::new(None) };
    static CHECK: RefCell<Option<glib::WeakRef<adw::PreferencesGroup>>> = const { RefCell::new(None) };
}

/// The state Sync shows now: settings, the running server, a connected USB e-reader and whether a
/// KOReader device has sent anything.
pub fn current(shared: &SharedRef) -> Snapshot {
    // the last scan (the library window rescans every few seconds): no I/O on the main thread
    let usb = crate::devices::latest()
        .mains()
        .into_iter()
        .next()
        .map(|d| d.name);
    sync_model::snapshot(
        &shared.cfg.borrow(),
        shared.sharing.borrow().running(),
        usb,
        crate::annotation_sync_ui::device_seen(shared),
    )
}

fn activate(w: &gtk::Widget, action: &str) {
    let _ = w.activate_action(action, None);
}

/// A row that opens another view: the whole row is activatable and ends with an arrow. `button` names
/// the action for screen readers and the tooltip.
fn link_row(
    title: &str,
    subtitle: &str,
    button: &str,
    on: impl Fn(&gtk::Widget) + 'static,
) -> adw::ActionRow {
    let r = adw::ActionRow::builder()
        .title(glib::markup_escape_text(title))
        .subtitle(glib::markup_escape_text(subtitle))
        .activatable(true)
        .build();
    r.set_subtitle_lines(0);
    r.set_title_lines(0);
    r.set_tooltip_text(Some(button));
    let arrow = gtk::Image::from_icon_name("go-next-symbolic");
    arrow.set_accessible_role(gtk::AccessibleRole::Presentation);
    r.add_suffix(&arrow);
    r.connect_activated(move |r| on(r.upcast_ref()));
    r
}

fn open_uri(w: &gtk::Widget, uri: &str) {
    let _ = w.activate_action("app.open-uri", Some(&uri.to_variant()));
}

// ---- 1. setup checklist -----------------------------------------------------------------------

/// The content of a step: what to do (plain text), then its controls. Rows go into one boxed list;
/// feedback labels come under it. Explanations live here, never in a row of their own.
pub fn step_body(help: &str, widgets: Vec<gtk::Widget>) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let l = gtk::Label::new(Some(&tr(help)));
    l.set_wrap(true);
    l.set_xalign(0.0);
    l.add_css_class("dim-label");
    b.append(&l);
    if !widgets.is_empty() {
        let g = adw::PreferencesGroup::new();
        for w in &widgets {
            g.add(w);
        }
        b.append(&g);
    }
    b.upcast()
}

/// The four steps as the stepper shows them: state and short status of each.
pub fn step_infos(snap: &Snapshot) -> Vec<StepInfo> {
    snap.step_statuses()
        .into_iter()
        .map(|(done, status)| StepInfo { done, status })
        .collect()
}

fn checklist_group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    SHARED.with(|s| *s.borrow_mut() = Some(ctx.shared.clone()));
    let shared = &ctx.shared;
    let g = adw::PreferencesGroup::new();
    g.set_title(&sync_model::title("checklist"));
    g.set_description(Some(&tr(
        "Four steps to sync with KOReader over Wi-Fi. Open any step; the first one that is not done is open for you.",
    )));
    CHECK.with(|c| *c.borrow_mut() = Some(g.downgrade()));

    let (account_rows, _server) = super::kosync::account_rows(shared);
    let steps = vec![
        (
            tr(sync_model::STEP_SERVER),
            step_body(
                sync_model::STEP_SERVER_HELP,
                super::share::step_server_rows(shared),
            ),
        ),
        (
            tr(sync_model::STEP_ACCOUNT),
            step_body(sync_model::STEP_ACCOUNT_HELP, account_rows),
        ),
        (
            tr(sync_model::STEP_PLUGIN),
            step_body(
                sync_model::STEP_PLUGIN_HELP,
                crate::annotation_sync_ui::install_rows(&ctx.parent, shared),
            ),
        ),
        (
            tr(sync_model::STEP_TEST),
            step_body(sync_model::STEP_TEST_HELP, Vec::new()),
        ),
    ];
    let stepper = Stepper::new(steps, false);
    g.add(stepper.widget());
    STEPPER.with(|s| *s.borrow_mut() = Some(stepper.clone()));

    // the steps follow the settings (and the 3 s timer: a device can sync without any change here)
    let update = {
        let (shared, stepper) = (shared.clone(), stepper.clone());
        Rc::new(move || stepper.update(&step_infos(&current(&shared))))
    };
    update();
    follow(stepper.widget(), {
        let u = update.clone();
        move || u()
    });
    let weak = g.downgrade();
    glib::timeout_add_seconds_local(3, move || match weak.upgrade() {
        Some(g) => {
            if g.is_mapped() {
                update();
            }
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });
    g
}

// ---- 2. what Bookx sync does ------------------------------------------------------------------

fn what_group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let (g, switch) = notes_group_for(&ctx.parent, &ctx.shared);
    NOTES_SW.with(|n| *n.borrow_mut() = Some(switch));
    g.add(&offer_jump_row(&ctx.shared));
    g
}

/// The switch *Offer to jump when a device is ahead*: while a book is open, a message offers to
/// continue where an e-reader has got to. On by default; the page never moves by itself.
fn offer_jump_row(shared: &SharedRef) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder()
        .title(tr("Offer to jump when a device is ahead"))
        .subtitle(tr(
            "While you read, a message offers to continue where your e-reader is. The page never moves by itself.",
        ))
        .build();
    row.set_active(shared.cfg.borrow().share.offer_jump);
    {
        let shared = shared.clone();
        row.connect_active_notify(move |r| {
            {
                let mut c = shared.cfg.borrow_mut();
                if c.share.offer_jump == r.is_active() {
                    return;
                }
                c.share.offer_jump = r.is_active();
                c.save();
            }
            crate::settings_bus::sync().notify();
        });
    }
    let (sh, sw) = (shared.clone(), row.downgrade());
    follow(&row, move || {
        let on = sh.cfg.borrow().share.offer_jump;
        if let Some(s) = sw.upgrade() {
            if s.is_active() != on {
                s.set_active(on);
            }
        }
    });
    row
}

/// The group “What Bookx sync does” (also the switch for highlights, notes and bookmarks).
pub fn notes_group_for(
    parent: &gtk::Widget,
    shared: &SharedRef,
) -> (adw::PreferencesGroup, adw::SwitchRow) {
    let shared = shared.clone();
    let (g, switch) = crate::annotation_sync_ui::group(parent, &shared, {
        let shared = shared.clone();
        Rc::new(move || {
            let _ = super::share::reapply(&shared);
        })
    });
    g.set_title(&sync_model::title("what"));
    g.set_description(Some(&glib::markup_escape_text(&format!(
        "{}\n\n{}",
        tr(sync_model::BRAND_EXPLAINED),
        tr(sync_model::WINDOW_BOOKS_NOTE)
    ))));
    let (sh, sw) = (shared, switch.downgrade());
    follow(&g, move || {
        let c = sh.cfg.borrow().share.clone();
        if let Some(s) = sw.upgrade() {
            if s.is_active() != c.annotations {
                s.set_active(c.annotations);
            }
        }
    });
    (g, switch)
}

// ---- 3. USB -----------------------------------------------------------------------------------

fn usb_group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let shared = &ctx.shared;
    let g = adw::PreferencesGroup::new();
    g.set_title(&sync_model::title("usb"));
    g.set_description(Some(&tr(
        "Plug in a Kobo, Kindle or other e-reader. Books, progress and notes sync over the cable, never wirelessly. Progress only moves forward and nothing is deleted.",
    )));
    // The canonical "Always sync when connected" switch lives in Devices → Settings; this page
    // only shows the USB result and the notes option so the two pages do not duplicate a control.
    let include_notes = adw::SwitchRow::builder()
        .title(tr(crate::device_sync::NOTES_TITLE))
        .subtitle(tr(crate::device_sync::NOTES_SUBTITLE))
        .build();
    include_notes.set_active(shared.cfg.borrow().device_sync.include_notes);
    {
        let sh = shared.clone();
        include_notes.connect_active_notify(move |r| {
            {
                let mut c = sh.cfg.borrow_mut();
                if c.device_sync.include_notes == r.is_active() {
                    return;
                }
                c.device_sync.include_notes = r.is_active();
                c.save();
            }
            crate::settings_bus::sync().notify();
        });
        let (sh, n) = (shared.clone(), include_notes.downgrade());
        follow(&g, move || {
            let notes = sh.cfg.borrow().device_sync.include_notes;
            if let Some(r) = n.upgrade() {
                if r.is_active() != notes {
                    r.set_active(notes);
                }
            }
        });
    }
    g.add(&include_notes);
    g.add(&link_row(
        &tr("Device window"),
        &tr("Send books, remove them and sync reading state now"),
        &tr("Open"),
        |w| activate(w, "app.devices"),
    ));
    g
}

// ---- 4. OPDS server -----------------------------------------------------------------------

fn servers_group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&sync_model::title("servers"));
    g.set_description(Some(&tr(
        "Let other apps and e-readers browse and download your books from this computer: KOReader, Moon+ Reader, Librera, Kybook and more.",
    )));
    for w in super::share::opds_rows(&ctx.shared) {
        g.add(&w);
    }
    g
}

// ---- 5. advanced ------------------------------------------------------------------------------

/// One status line (“Wi-Fi sync: on · USB device: connected · OPDS: off”) for the description of
/// the Advanced group.
fn status_line(shared: &SharedRef) -> String {
    current(shared)
        .lines()
        .into_iter()
        .map(|l| l.text)
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A requirement as a small table (what the row of the Requirements table says), not a row.
fn requirement_table(r: &sync_model::Requirement) -> gtk::Grid {
    let g = gtk::Grid::new();
    g.set_column_spacing(12);
    g.set_row_spacing(6);
    g.set_margin_top(6);
    g.set_margin_bottom(10);
    g.set_margin_start(12);
    g.set_margin_end(12);
    let cells = [
        (1, r.works_between),
        (2, r.bookx_needs),
        (3, r.reader_needs),
        (4, r.connection),
        (5, r.syncs),
    ];
    let mut row = 0;
    for (col, text) in cells {
        let k = gtk::Label::new(Some(&tr(sync_model::COLUMNS[col])));
        k.add_css_class("dim-label");
        k.set_xalign(0.0);
        k.set_yalign(0.0);
        k.set_wrap(true);
        k.set_max_width_chars(14);
        let v = gtk::Label::new(Some(&tr(text)));
        v.set_wrap(true);
        v.set_xalign(0.0);
        v.set_hexpand(true);
        v.set_selectable(true);
        g.attach(&k, 0, row, 1, 1);
        g.attach(&v, 1, row, 1, 1);
        row += 1;
    }
    let menu = match r.id {
        "notes" => Some(sync_model::KOREADER_PATH),
        "progress" => Some(sync_model::KOREADER_PROGRESS_PATH),
        _ => None,
    };
    if let Some(m) = menu {
        let l = gtk::Label::new(Some(&tr(m)));
        l.set_wrap(true);
        l.set_xalign(0.0);
        l.set_tooltip_text(Some(&tr(m)));
        g.attach(&l, 0, row, 2, 1);
    }
    g
}

const REQUIREMENTS_TITLE: &str = N_("What each sync type needs");

/// The requirements table as a dialog: one group per sync type with its columns as a small table, the stock
/// Kobo/Kindle note on top. The page only has a row that opens it.
pub fn requirements_dialog(parent: &gtk::Widget) -> adw::Dialog {
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr(REQUIREMENTS_TITLE));
    dialog.set_content_width(560);
    dialog.set_content_height(520);
    let page = adw::PreferencesPage::new();
    let first = adw::PreferencesGroup::new();
    first.set_description(Some(&glib::markup_escape_text(&tr(sync_model::STOCK_NOTE))));
    page.add(&first);
    for r in &sync_model::REQUIREMENTS {
        let g = adw::PreferencesGroup::new();
        g.set_title(&glib::markup_escape_text(&tr(r.sync_type)));
        g.set_description(Some(&glib::markup_escape_text(&format!(
            "{} · {}",
            tr(r.connection),
            tr(r.syncs)
        ))));
        g.add(&requirement_table(r));
        page.add(&g);
    }
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    dialog.set_child(Some(&view));
    dialog.present(Some(parent));
    dialog
}

fn advanced_group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let shared = &ctx.shared;
    let g = adw::PreferencesGroup::new();
    // the live status line is the description: it needs no row of its own
    g.set_description(Some(&glib::markup_escape_text(&status_line(shared))));
    follow(&g, {
        let (shared, g) = (shared.clone(), g.downgrade());
        move || {
            if let Some(g) = g.upgrade() {
                g.set_description(Some(&glib::markup_escape_text(&status_line(&shared))));
            }
        }
    });
    let adv = adw::ExpanderRow::new();
    adv.set_title(&sync_model::title("advanced"));
    adv.set_subtitle(&tr(
        "Server address, port and firewall, HTTPS, matching, privacy and offline mode, reverse proxy, requirements",
    ));
    adv.set_subtitle_lines(0);
    super::audit::label(&adv, &sync_model::title("advanced"));
    ADVANCED.with(|a| *a.borrow_mut() = Some(adv.downgrade()));

    // the account server and the reading-position settings
    if let Some(server) = super::kosync::server_row() {
        server.set_tooltip_text(Some(&super::kosync::position_help()));
        adv.add_row(&server);
    }
    for w in super::kosync::position_rows(shared) {
        adv.add_row(&w);
    }
    // the technical server settings
    for w in super::share::advanced_rows(&ctx.parent, shared) {
        adv.add_row(&w);
    }
    for w in super::proxy_settings::rows(shared) {
        adv.add_row(&w);
    }
    for w in super::accounts::rows(shared) {
        adv.add_row(&w);
    }
    // reverse proxy (with the request limits), privacy and other transfers
    adv.add_row(&link_row(
        &tr("Reverse proxy and remote access"),
        &crate::i18n::trf(
            "Put nginx or Caddy in front of Bookx for HTTPS on your own address. Bookx sync answers on the /bookx path and accepts up to {} requests per minute for each account; a reverse proxy can limit more.",
            &[&bookx_serve::annotation_sync::RATE_LIMIT],
        ),
        &tr("Open guide"),
        |w| open_uri(w, "https://bookx.adhadse.com/reverse-proxy/"),
    ));
    adv.add_row(&link_row(
        &tr("Offline mode and the list of connections"),
        &tr("Offline mode refuses online look-ups; your own servers keep working. The list names everything Bookx can connect to, and when."),
        &tr("Open Privacy"),
        |_| super::preferences::goto_page("privacy"),
    ));
    adv.add_row(&link_row(
        &tr("Send by e-mail"),
        &tr("Account and address for sending books to a Kindle or another address"),
        &tr("Open"),
        |_| super::preferences::goto_page("email"),
    ));
    adv.add_row(&link_row(
        &tr(REQUIREMENTS_TITLE),
        &tr("Which device needs what, over which connection, and what syncs"),
        &tr("Show"),
        |w| {
            requirements_dialog(w);
        },
    ));
    g.add(&adv);
    g
}

// ---- hooks ------------------------------------------------------------------------------------

fn all_text(w: &gtk::Widget, out: &mut Vec<String>) {
    if let Some(l) = w.downcast_ref::<gtk::Label>() {
        out.push(l.text().to_string());
    }
    if let Some(r) = w.downcast_ref::<adw::PreferencesRow>() {
        out.push(r.title().to_string());
    }
    if let Some(r) = w.downcast_ref::<adw::ActionRow>() {
        out.push(r.subtitle().unwrap_or_default().to_string());
    }
    if let Some(t) = w.tooltip_text() {
        out.push(t.to_string());
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        all_text(&child, out);
        c = child.next_sibling();
    }
}

/// Plain action rows that have only text: nothing to press, nothing to type. Explanations belong in
/// a group description, a tooltip or the step itself, so the Sync page must have none.
fn info_only_rows(w: &gtk::Widget) -> Vec<String> {
    fn walk(w: &gtk::Widget, out: &mut Vec<String>) {
        if let Some(r) = w.downcast_ref::<adw::PreferencesRow>() {
            let plain = r.type_() == adw::ActionRow::static_type() && !in_activity(r.upcast_ref());
            if plain && !r.is_activatable() && !has_button(r.upcast_ref()) && r.is_visible() {
                out.push(r.title().to_string());
            }
        }
        let mut c = w.first_child();
        while let Some(ch) = c {
            walk(&ch, out);
            c = ch.next_sibling();
        }
    }
    fn has_button(w: &gtk::Widget) -> bool {
        if w.is::<gtk::Button>() || w.is::<gtk::MenuButton>() || w.is::<gtk::Switch>() {
            return true;
        }
        let mut c = w.first_child();
        while let Some(ch) = c {
            if has_button(&ch) {
                return true;
            }
            c = ch.next_sibling();
        }
        false
    }
    let mut out = Vec::new();
    walk(w, &mut out);
    out
}

/// True for a row inside the Sync activity groups: their rows show facts, not controls.
fn in_activity(w: &gtk::Widget) -> bool {
    let mut p = w.parent();
    while let Some(x) = p {
        if x.widget_name() == super::sync_activity::BLOCK_NAME {
            return true;
        }
        p = x.parent();
    }
    false
}

fn expander_titles(w: &gtk::Widget, out: &mut Vec<String>) {
    if let Some(e) = w.downcast_ref::<adw::ExpanderRow>() {
        out.push(e.title().to_string());
    }
    let mut c = w.first_child();
    while let Some(ch) = c {
        expander_titles(&ch, out);
        c = ch.next_sibling();
    }
}

fn find_button(w: &gtk::Widget, label: &str) -> Option<gtk::Button> {
    if let Some(b) = w.downcast_ref::<gtk::Button>() {
        if b.label().as_deref() == Some(label) {
            return Some(b.clone());
        }
    }
    let mut c = w.first_child();
    while let Some(ch) = c {
        if let Some(f) = find_button(&ch, label) {
            return Some(f);
        }
        c = ch.next_sibling();
    }
    None
}

fn find_titled(w: &gtk::Widget, title: &str) -> Option<gtk::Widget> {
    if let Some(r) = w.downcast_ref::<adw::PreferencesRow>() {
        if r.title() == title {
            return Some(w.clone());
        }
    }
    let mut c = w.first_child();
    while let Some(ch) = c {
        if let Some(f) = find_titled(&ch, title) {
            return Some(f);
        }
        c = ch.next_sibling();
    }
    None
}

/// Dev hook `prefs:sync-…`: `dump` (the state), `req` (what the page says), `advanced=open|close`,
/// `adv` (what Advanced holds and whether its fields are on screen), `server=on|off`,
/// `opds=on|off`, `port=N`.
pub fn dev(cmd: &str) {
    if let Some(f) = cmd.strip_prefix("qr") {
        let parent = ADVANCED.with(|a| a.borrow().as_ref().and_then(|w| w.upgrade()));
        super::qr_dialog::dev(f, parent.map(|p| p.upcast()));
        return;
    }
    if let Some(f) = cmd.strip_prefix("proxy") {
        let sh = SHARED.with(|s| s.borrow().clone());
        super::proxy_settings::dev(f, sh.as_ref());
        return;
    }
    if let Some(v) = cmd.strip_prefix("advanced=") {
        if let Some(a) = ADVANCED.with(|a| a.borrow().as_ref().and_then(|w| w.upgrade())) {
            a.set_expanded(v != "close");
        }
        return;
    }
    if cmd == "notes" {
        // the highlights, notes and bookmarks switch: on by default, independent of the server
        let on = NOTES_SW.with(|n| n.borrow().as_ref().map(|s| s.is_active()));
        let sh = SHARED.with(|s| s.borrow().clone());
        match (on, sh) {
            (Some(on), Some(sh)) => {
                eprintln!(
                    "prefs_notes: switch={on} config={} listening={} store={}",
                    sh.cfg.borrow().share.annotations,
                    sh.sharing.borrow().running(),
                    sh.sharing.borrow().annotations().is_some()
                );
            }
            _ => {
                eprintln!("prefs_notes: no page");
            }
        }
        return;
    }
    if let Some(v) = cmd.strip_prefix("notes=") {
        NOTES_SW.with(|n| n.borrow().as_ref().map(|s| s.set_active(v != "off")));
        return;
    }
    if let Some(i) = cmd.strip_prefix("step=") {
        if let (Some(st), Ok(i)) = (STEPPER.with(|s| s.borrow().clone()), i.parse::<usize>()) {
            st.pick(i.saturating_sub(1));
        }
        return;
    }
    if cmd == "stepper" {
        match STEPPER.with(|s| s.borrow().clone()) {
            Some(st) => eprintln!("prefs_sync_stepper: {}", st.dump()),
            None => eprintln!("prefs_sync_stepper: no page"),
        }
        return;
    }
    if cmd == "plugin" {
        // presses “Copy to the device” of step 3 and says whether main.lua landed in KOReader
        let page = CHECK
            .with(|r| r.borrow().as_ref().and_then(|w| w.upgrade()))
            .and_then(|g| g.ancestor(adw::PreferencesPage::static_type()));
        let button = page.and_then(|p| find_button(&p, &tr("Copy to the device")));
        if let Some(b) = button {
            b.emit_clicked();
        }
        // the copy looks for the reader on a worker thread: say what landed once it had time to
        glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || {
            let copied = crate::devices::latest()
                .devices
                .into_iter()
                .find_map(|d| crate::annotation_sync_ui::plugins_folder(&d.mount))
                .map(|p| {
                    p.join(crate::annotation_sync_ui::PLUGIN_DIR)
                        .join("main.lua")
                        .is_file()
                });
            eprintln!("prefs_plugin: copied={}", copied == Some(true));
        });
        return;
    }
    if cmd == "adv" {
        let Some(a) = ADVANCED.with(|a| a.borrow().as_ref().and_then(|w| w.upgrade())) else {
            eprintln!("prefs_sync_adv: no page");
            return;
        };
        let mut text = Vec::new();
        all_text(a.upcast_ref(), &mut text);
        let all = text.join("\n");
        let has = |t: &str| all.contains(&tr(t));
        let mapped = |t: &str| find_titled(a.upcast_ref(), &tr(t)).is_some_and(|w| w.is_visible());
        eprintln!(
            "prefs_sync_adv: expanded={} server_address={} port={} https={} matching={} device_name={} privacy={} reverse_proxy={} requirements={} firewall={} port_reachable={}",
            a.is_expanded(),
            has("Server address (Advanced)"),
            has("Port"),
            has("Use HTTPS"),
            has("Match books by"),
            has("This device is called"),
            has("Offline mode and the list of connections"),
            has("Reverse proxy and remote access"),
            has(REQUIREMENTS_TITLE),
            all.contains(&tr(
                "Allow this port in your firewall for the e-reader to connect."
            )),
            a.is_expanded() && mapped("Port"),
        );
        return;
    }
    if cmd == "requirements" {
        let parent = CHECK
            .with(|r| r.borrow().as_ref().and_then(|w| w.upgrade()))
            .map(|g| g.upcast::<gtk::Widget>());
        let Some(parent) = parent else {
            eprintln!("prefs_sync_requirements: no page");
            return;
        };
        let d = requirements_dialog(&parent);
        let mut groups = Vec::new();
        fn titles(w: &gtk::Widget, out: &mut Vec<String>) {
            if let Some(g) = w.downcast_ref::<adw::PreferencesGroup>() {
                if !g.title().is_empty() {
                    out.push(g.title().to_string());
                }
            }
            let mut c = w.first_child();
            while let Some(ch) = c {
                titles(&ch, out);
                c = ch.next_sibling();
            }
        }
        if let Some(c) = d.child() {
            titles(&c, &mut groups);
        }
        eprintln!(
            "prefs_sync_requirements: groups={} titles={}",
            groups.len(),
            groups.join("|")
        );
        return;
    }
    if cmd == "req" {
        let Some(g) = CHECK.with(|r| r.borrow().as_ref().and_then(|w| w.upgrade())) else {
            eprintln!("prefs_sync_req: no page");
            return;
        };
        let page = g.ancestor(adw::PreferencesPage::static_type());
        let root = page.as_ref().unwrap_or(g.upcast_ref());
        let mut titles = Vec::new();
        expander_titles(root, &mut titles);
        let mut text = Vec::new();
        all_text(root, &mut text);
        let all = text.join("\n");
        let steps_ok = sync_model::Checklist::default()
            .steps()
            .iter()
            .all(|s| all.contains(&tr(s.1)));
        let info_only = info_only_rows(root);
        let kosync_labels = text.iter().filter(|t| t.contains("KOSync")).count();
        let req_titles: Vec<String> = titles
            .iter()
            .filter(|t| {
                sync_model::REQUIREMENTS
                    .iter()
                    .any(|r| tr(r.sync_type) == **t)
            })
            .cloned()
            .collect();
        eprintln!(
            "prefs_sync_req: rows={} titles={} columns={} notsupported={} bookx_path={} kosync_text={} koreader_menu={} steps={} account_help={} not_share={} kosync_labels={} brand={} advanced={} info_rows={}",
            req_titles.len(),
            req_titles.join("|"),
            sync_model::COLUMNS[1..]
                .iter()
                .all(|c| all.contains(&tr(c))),
            all.contains("not supported"),
            all.contains("/bookx"),
            all.contains("/kosync"),
            all.contains("More tools → Bookx Sync"),
            steps_ok,
            all.contains("Choose any user name and password"),
            all.contains("not the OPDS server password"),
            kosync_labels,
            all.contains(&tr(sync_model::BRAND_EXPLAINED)),
            titles.iter().any(|t| *t == tr("Advanced")),
            info_only.len(),
        );
        if !info_only.is_empty() {
            eprintln!("prefs_sync_info_rows: {}", info_only.join(" | "));
        }
        return;
    }
    super::share::dev_prefs(cmd, SHARED.with(|s| s.borrow().clone()).as_ref());
}

thread_local! {
    /// The three activity groups are made together (they share one set of rows); the later two wait here.
    static ACTIVITY: RefCell<Option<super::sync_activity::Groups>> = const { RefCell::new(None) };
}

fn activity_unmatched_group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = super::sync_activity::build(&ctx.shared);
    let first = g.unmatched.clone();
    ACTIVITY.with(|a| *a.borrow_mut() = Some(g));
    first
}

fn activity_devices_group(_ctx: &PrefCtx) -> adw::PreferencesGroup {
    ACTIVITY
        .with(|a| a.borrow().as_ref().map(|g| g.devices.clone()))
        .unwrap_or_default()
}

fn activity_recent_group(_ctx: &PrefCtx) -> adw::PreferencesGroup {
    ACTIVITY
        .with(|a| a.borrow().as_ref().map(|g| g.activity.clone()))
        .unwrap_or_default()
}

pub fn register() {
    let o = sync_model::order;
    register_group("sync", o("activity"), activity_unmatched_group);
    register_group("sync", o("activity") + 1, activity_devices_group);
    register_group("sync", o("activity") + 2, activity_recent_group);
    register_group("sync", o("checklist"), checklist_group);
    register_group("sync", o("what"), what_group);
    register_group("sync", o("usb"), usb_group);
    register_group("sync", o("servers"), servers_group);
    register_group("sync", o("advanced"), advanced_group);
}
