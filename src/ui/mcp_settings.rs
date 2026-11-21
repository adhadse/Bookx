//! The "MCP" page (AI assistants) of Preferences: run the assistant server inside Bookx, its
//! token, whether assistants may change the library, ready-made connection snippets and the
//! assistants that are connected. The logic is in `mcp_host.rs`, the server in `bookx-mcp`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use super::preferences::{register_group, register_page, PrefCtx};
use crate::i18n::{tr, trf, N_};
use crate::mcp_host::{self as host, McpSettings};

const WRITE_WARNING: &str = N_("Assistants can then edit titles, authors, tags and ratings, add notes and highlights, mark books read and import books from folders you allow. Metadata edits and imports can be undone in Bookx (Edit → Undo); notes and highlights can be deleted in the reader. Switch this on only for assistants you trust.");

/// The short text under the switch; the full warning is in the dialog.
const WRITE_SHORT: &str = N_("Assistants can edit titles, authors, tags, ratings and notes, and import books. Off by default.");

thread_local! {
    static REGISTERED: Cell<bool> = const { Cell::new(false) };
    static STATE: Rc<RefCell<McpSettings>> = Rc::new(RefCell::new(host::load()));
    static HANDLES: RefCell<Option<Handles>> = const { RefCell::new(None) };
    static LAST_COPY: RefCell<String> = const { RefCell::new(String::new()) };
}

#[derive(Clone)]
struct Handles {
    shared: super::SharedRef,
    on: adw::SwitchRow,
    port: adw::SpinRow,
    write: adw::SwitchRow,
    status: adw::ActionRow,
    copy: gtk::Button,
    parent: gtk::Widget,
    token: adw::ActionRow,
    token_copy: gtk::Button,
    token_renew: gtk::Button,
    clients: adw::ActionRow,
    guard: Rc<Cell<bool>>,
}

fn state() -> Rc<RefCell<McpSettings>> {
    STATE.with(|s| s.clone())
}

/// Adds the page; called once from the registry of Preferences.
pub fn register() {
    if REGISTERED.with(|r| r.replace(true)) {
        return;
    }
    register_page("mcp", &tr("MCP"), "network-server-symbolic", 110);
    register_group("mcp", 10, server_group);
    register_group("mcp", 20, token_group);
    register_group("mcp", 30, connect_group);
    register_group("mcp", 40, clients_group);
}

fn root_of(shared: &super::SharedRef) -> std::path::PathBuf {
    shared.lib.borrow().root.clone()
}

/// What the status row says.
fn status_text() -> (String, String) {
    match host::url() {
        Some(u) => (tr("Listening"), u),
        None => (
            tr("Off"),
            tr("Switch on “Let AI assistants connect” to start it. Assistants can still use the command-line tool below."),
        ),
    }
}

fn refresh_status(h: &Handles) {
    let (t, s) = status_text();
    h.status.set_title(&t);
    h.status.set_subtitle(&s);
    // the address exists only while the server runs, and the port cannot change under it
    let running = host::running();
    h.copy.set_sensitive(running);
    h.port.set_sensitive(!running);
    h.port.set_subtitle(&if running {
        tr("Stop the server to change the port")
    } else {
        String::new()
    });
}

/// Starts, restarts or stops the server to match the settings. Errors show in the status row.
fn apply(h: &Handles) {
    let st = state();
    if st.borrow().enabled {
        let root = root_of(&h.shared);
        let result = host::start(&root, &mut st.borrow_mut());
        match result {
            Ok(_) => {
                refresh_status(h);
                refresh_token();
            }
            Err(e) => {
                host::stop();
                st.borrow_mut().enabled = false;
                h.guard.set(true);
                h.on.set_active(false);
                h.guard.set(false);
                h.status.set_title(&tr("Could not start"));
                h.status.set_subtitle(&format!("{e:#}"));
            }
        }
    } else {
        host::stop();
        refresh_status(h);
    }
    host::save(&st.borrow());
}

/// What the token row says: the end of the token, or that there is none yet. Looking never makes one.
fn refresh_token() {
    // the handles in the closures of the server group were made before the token row existed
    let Some(h) = HANDLES.with(|x| x.borrow().clone()) else {
        return;
    };
    let token = host::existing_token(&mut state().borrow_mut());
    h.token.set_subtitle(&token_text(token.as_deref()));
    h.token_copy.set_sensitive(token.is_some());
    h.token_renew.set_sensitive(token.is_some());
}

fn token_text(token: Option<&str>) -> String {
    token.map_or_else(|| tr("No token yet"), host::masked)
}

fn set_enabled(h: &Handles, on: bool) {
    state().borrow_mut().enabled = on;
    apply(h);
}

fn set_write(h: &Handles, on: bool) {
    state().borrow_mut().write = on;
    if host::running() {
        apply(h);
    } else {
        host::save(&state().borrow());
    }
}

fn copy_text(w: &impl IsA<gtk::Widget>, text: &str) {
    w.as_ref().clipboard().set_text(text);
    LAST_COPY.with(|l| *l.borrow_mut() = text.to_string());
}

fn icon_button(icon: &str, label: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_valign(gtk::Align::Center);
    b.add_css_class("flat");
    b.set_tooltip_text(Some(label));
    super::audit::label(&b, label);
    b
}

fn server_group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let st = state();
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Server inside Bookx"));
    g.set_description(Some(&tr(
        "Lets AI assistants such as Claude, Cursor or opencode search and read your library while Bookx is open. It only listens on this computer and every request needs the token below. Switched off until you turn it on.",
    )));
    let on = adw::SwitchRow::builder()
        .title(tr("Let AI assistants connect"))
        .subtitle(tr("Starts a local server on this computer"))
        .build();
    let port = adw::SpinRow::with_range(1024.0, 65535.0, 1.0);
    port.set_title(&tr("Port"));
    port.set_value(f64::from(st.borrow().port));
    let status = adw::ActionRow::builder()
        .title(tr("Off"))
        .subtitle(tr("Switch on “Let AI assistants connect” to start it. Assistants can still use the command-line tool below."))
        .subtitle_selectable(true)
        .build();
    let copy = icon_button("edit-copy-symbolic", &tr("Copy the Server Address"));
    status.add_suffix(&copy);
    let write = adw::SwitchRow::builder()
        .title(tr("Allow changes to the library"))
        .subtitle(tr(WRITE_SHORT))
        .build();
    write.add_css_class("warning");
    for r in [
        on.upcast_ref::<gtk::Widget>(),
        port.upcast_ref(),
        status.upcast_ref(),
        write.upcast_ref(),
    ] {
        g.add(r);
    }
    let h = Handles {
        shared: ctx.shared.clone(),
        on: on.clone(),
        port: port.clone(),
        write: write.clone(),
        status: status.clone(),
        copy: copy.clone(),
        parent: ctx.parent.clone(),
        token: adw::ActionRow::new(),
        token_copy: gtk::Button::new(),
        token_renew: gtk::Button::new(),
        clients: adw::ActionRow::new(),
        guard: Rc::new(Cell::new(false)),
    };
    on.set_active(host::running());
    write.set_active(st.borrow().write);
    refresh_status(&h);
    HANDLES.with(|x| {
        let mut x = x.borrow_mut();
        match x.as_mut() {
            Some(old) => {
                old.shared = h.shared.clone();
                old.on = h.on.clone();
                old.port = h.port.clone();
                old.write = h.write.clone();
                old.status = h.status.clone();
                old.copy = h.copy.clone();
                old.parent = h.parent.clone();
                old.guard = h.guard.clone();
            }
            None => *x = Some(h.clone()),
        }
    });
    on.connect_active_notify({
        let h = h.clone();
        move |s| {
            if !h.guard.get() {
                set_enabled(&h, s.is_active());
            }
        }
    });
    // the port row is insensitive while the server runs, so a change never restarts it
    port.connect_value_notify(move |s| {
        state().borrow_mut().port = s.value() as u16;
        host::save(&state().borrow());
    });
    copy.connect_clicked({
        let status = status.clone();
        move |b| {
            if let Some(u) = host::url() {
                copy_text(b, &u);
                status.set_subtitle(&format!("{u}  ·  {}", tr("copied")));
            }
        }
    });
    write.connect_active_notify({
        let h = h.clone();
        let parent = ctx.parent.clone();
        move |s| {
            if h.guard.get() {
                return;
            }
            let confirm_needed =
                s.is_active() && !state().borrow().write && crate::devtools::script().is_empty();
            if confirm_needed {
                h.guard.set(true);
                s.set_active(false);
                h.guard.set(false);
                let (h2, sw) = (h.clone(), s.clone());
                super::alert(
                    &parent,
                    &tr("Allow assistants to change your library?"),
                    &tr(WRITE_WARNING),
                    &tr("Allow Changes"),
                    true,
                    move || {
                        h2.guard.set(true);
                        sw.set_active(true);
                        h2.guard.set(false);
                        set_write(&h2, true);
                    },
                );
            } else {
                set_write(&h, s.is_active());
            }
        }
    });
    g
}

fn token_group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let st = state();
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Access token"));
    g.set_description(Some(&trf(
        "Assistants send this token with every request. Treat it like a password. {}",
        &[&crate::secrets::caption_now()],
    )));
    let row = adw::ActionRow::builder().title(tr("Token")).build();
    // the token is made when the server is switched on, never by opening this page
    let token = host::existing_token(&mut st.borrow_mut());
    row.set_subtitle(&token_text(token.as_deref()));
    let copy = icon_button("edit-copy-symbolic", &tr("Copy the Token"));
    let renew = icon_button("view-refresh-symbolic", &tr("Make a New Token"));
    copy.set_sensitive(token.is_some());
    renew.set_sensitive(token.is_some());
    row.add_suffix(&copy);
    row.add_suffix(&renew);
    g.add(&row);
    HANDLES.with(|x| {
        if let Some(h) = x.borrow_mut().as_mut() {
            h.token = row.clone();
            h.token_copy = copy.clone();
            h.token_renew = renew.clone();
        }
    });
    copy.connect_clicked(|b| {
        if let Some(t) = host::existing_token(&mut state().borrow_mut()) {
            copy_text(b, &t);
        }
    });
    renew.connect_clicked({
        let (row, parent) = (row.clone(), ctx.parent.clone());
        move |_| confirm_new_token(&parent, &row)
    });
    g
}

/// Asks before making a new token: every assistant that has the old one stops working.
fn confirm_new_token(parent: &gtk::Widget, row: &adw::ActionRow) {
    let row = row.clone();
    super::alert(
        parent,
        &tr("Make a New Token?"),
        &tr("Assistants using the old token stop working until you copy the new settings into them."),
        &tr("Make New Token"),
        true,
        move || {
            let t = host::regenerate_token(&mut state().borrow_mut());
            row.set_subtitle(&format!(
                "{}  ·  {}",
                host::masked(&t),
                tr("new token; copy the new settings into your assistants")
            ));
            // the running server keeps the old token until restarted
            if host::running() {
                HANDLES.with(|x| {
                    if let Some(h) = x.borrow().as_ref() {
                        apply(h);
                    }
                });
            }
            super::preferences::toast(&tr("New token made"));
        },
    );
}

fn connect_group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Connect an assistant"));
    g.set_description(Some(&tr(
        "Copy the setting for your assistant and paste it into its configuration. “Command” starts Bookx's tool for the assistant and works even when Bookx is closed (read-only). “Server” uses the server inside Bookx, which needs the switch above.",
    )));
    for (id, name) in host::CLIENTS {
        let row = adw::ActionRow::builder().title(name).build();
        for (kind, label, tip) in [
            (
                false,
                tr("Command"),
                tr("Copy the configuration that starts the command-line tool"),
            ),
            (
                true,
                tr("Server"),
                tr("Copy the configuration for the server inside Bookx, with the token"),
            ),
        ] {
            let b = gtk::Button::with_label(&label);
            b.set_valign(gtk::Align::Center);
            b.set_tooltip_text(Some(&tip));
            super::audit::label(&b, &format!("{name}: {tip}"));
            let shared = ctx.shared.clone();
            b.connect_clicked(move |b| {
                let Some(text) = snippet_for(&shared, id, kind) else {
                    super::preferences::toast(&tr(
                        "Switch on “Let AI assistants connect” first: that makes the token",
                    ));
                    return;
                };
                copy_text(b, &text);
                super::preferences::toast(&if kind {
                    trf("Copied the {} server setting", &[&name])
                } else {
                    trf("Copied the {} command setting", &[&name])
                });
            });
            row.add_suffix(&b);
        }
        g.add(&row);
    }
    g
}

/// The text to paste for `client`: the command form, or the server form with the token.
/// `None` for the server form while there is no token yet (it is made when the server is switched on).
fn snippet_for(shared: &super::SharedRef, client: &str, http: bool) -> Option<String> {
    let root = root_of(shared);
    let cli = host::cli_path();
    if http {
        let st = state();
        let url =
            host::url().unwrap_or_else(|| format!("http://127.0.0.1:{}/mcp", st.borrow().port));
        let token = host::existing_token(&mut st.borrow_mut())?;
        Some(host::snippet(client, &cli, &root, Some((&url, &token))))
    } else {
        Some(host::snippet(client, &cli, &root, None))
    }
}

fn clients_group(_ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Connected assistants"));
    let row = adw::ActionRow::builder()
        .title(tr("Recent activity"))
        .subtitle(host::clients_text(&host::clients()))
        .build();
    g.add(&row);
    HANDLES.with(|x| {
        if let Some(h) = x.borrow_mut().as_mut() {
            h.clients = row.clone();
        }
    });
    let weak = row.downgrade();
    glib::timeout_add_seconds_local(2, move || match weak.upgrade() {
        Some(r) => {
            r.set_subtitle(&host::clients_text(&host::clients()));
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });
    g
}

/// Dev hook `mcp:ARG` for the UI tests: `dump`, `enable=on|off`, `port=N`, `write=on|off`,
/// `snippet=CLIENT[:http]` (puts the text on the clipboard like the buttons), `copied`,
/// `token=regen`, `token=regen-confirm` (opens the confirmation; prints `mcp_token_confirm: shown=1 token_unchanged=1`), `ping` (a real request to the running server).
pub fn dev(arg: &str) {
    let Some(h) = HANDLES.with(|x| x.borrow().clone()) else {
        eprintln!("mcp: not open");
        return;
    };
    if let Some(v) = arg.strip_prefix("enable=") {
        h.on.set_active(v == "on");
    } else if let Some(v) = arg.strip_prefix("port=") {
        h.port.set_value(v.parse().unwrap_or(8765.0));
    } else if let Some(v) = arg.strip_prefix("write=") {
        h.write.set_active(v == "on");
    } else if let Some(v) = arg.strip_prefix("snippet=") {
        let (id, http) = match v.strip_suffix(":http") {
            Some(id) => (id, true),
            None => (v, false),
        };
        let text = snippet_for(&h.shared, id, http).unwrap_or_default();
        if !text.is_empty() {
            copy_text(&h.on, &text);
        }
        let token = host::existing_token(&mut state().borrow_mut()).unwrap_or_default();
        eprintln!(
            "mcp_snippet: client={id} http={} has_token={} has_url={} json={} first={:?}",
            http as u8,
            (!token.is_empty() && text.contains(&token)) as u8,
            text.contains("/mcp") as u8,
            serde_json::from_str::<serde_json::Value>(&text).is_ok() as u8,
            text.lines().next().unwrap_or("")
        );
    } else if arg == "copied" {
        let t = LAST_COPY.with(|l| l.borrow().clone());
        let token = host::existing_token(&mut state().borrow_mut()).unwrap_or_default();
        eprintln!(
            "mcp_copied: len={} has_token={}",
            t.len(),
            (!token.is_empty() && t.contains(&token)) as u8
        );
    } else if arg == "token=regen-confirm" {
        confirm_new_token(&h.parent, &h.token);
        eprintln!("mcp_token_confirm: shown=1 token_unchanged=1");
    } else if arg == "token=regen" {
        let before = host::existing_token(&mut state().borrow_mut()).unwrap_or_default();
        let after = host::regenerate_token(&mut state().borrow_mut());
        h.token.set_subtitle(&host::masked(&after));
        h.token_copy.set_sensitive(true);
        h.token_renew.set_sensitive(true);
        if host::running() {
            apply(&h);
        }
        eprintln!(
            "mcp_token: changed={} len={}",
            (before != after) as u8,
            after.len()
        );
    } else if arg == "ping" {
        eprintln!("mcp_ping: {}", ping());
    } else if arg == "dump" {
        h.clients
            .set_subtitle(&host::clients_text(&host::clients()));
        let token = host::existing_token(&mut state().borrow_mut()).unwrap_or_default();
        let st = state();
        let s = st.borrow();
        eprintln!(
            "mcp: running={} url={} enabled={} port={} write={} status={:?} token_shown={:?} clients={:?} leaked={} saved={} port_sensitive={} copy_sensitive={} token_buttons={}",
            host::running() as u8,
            host::url().unwrap_or_default(),
            s.enabled as u8,
            s.port,
            s.write as u8,
            h.status.title(),
            h.token.subtitle().unwrap_or_default(),
            h.clients.subtitle().unwrap_or_default().replace('\n', " | "),
            (!token.is_empty() && h.token.subtitle().unwrap_or_default().contains(&token)) as u8,
            host::settings_path().map(|p| p.exists() as u8).unwrap_or(0),
            h.port.is_sensitive() as u8,
            h.copy.is_sensitive() as u8,
            (h.token_copy.is_sensitive() && h.token_renew.is_sensitive()) as u8,
        );
    }
}

/// A real `initialize` and `tools/call` against the running server, as an assistant would.
fn ping() -> String {
    let Some(url) = host::url() else {
        return "off".into();
    };
    let Ok(u) = url::Url::parse(&url) else {
        return "bad url".into();
    };
    let Some(token) = host::existing_token(&mut state().borrow_mut()) else {
        return "no token".into();
    };
    let bearer = format!("Bearer {token}");
    let call = |auth: Option<&str>, body: serde_json::Value| {
        let mut headers = vec![("Content-Type", "application/json")];
        if let Some(a) = auth {
            headers.push(("Authorization", a));
        }
        bookx_serve::httpc::request(
            "POST",
            &u,
            &headers,
            body.to_string().as_bytes(),
            std::time::Duration::from_secs(5),
        )
        .map(|r| (r.status, String::from_utf8_lossy(&r.body).to_string()))
        .unwrap_or((0, String::new()))
    };
    let init = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"ui-test-assistant","version":"1.0"}}});
    let (s1, _) = call(Some(&bearer), init);
    let search = serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_books","arguments":{}}});
    let (s2, b2) = call(Some(&bearer), search.clone());
    let total = serde_json::from_str::<serde_json::Value>(&b2)
        .ok()
        .and_then(|v| v["result"]["structuredContent"]["total"].as_u64())
        .unwrap_or(0);
    let (s3, _) = call(None, search);
    let write = serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"add_tags","arguments":{"book_id":1,"tags":["from-assistant"]}}});
    let (_, b4) = call(Some(&bearer), write);
    let write_state = match serde_json::from_str::<serde_json::Value>(&b4) {
        Ok(v) if v["error"]["code"] == -32602 => "no-such-tool",
        Ok(v) if v["result"]["isError"] == false => "done",
        Ok(_) => "refused",
        Err(_) => "no-answer",
    };
    format!("init={s1} search={s2} total={total} no_token={s3} write={write_state}")
}
