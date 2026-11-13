//! Bookx as a KOSync client in the interface: the settings group of Preferences (server, account,
//! Register and Log in), and the reader's side of it: pull when a book opens (with the "Continue
//! from <device> at 63%?" question), push after page turns and when the book closes. The rules are
//! in `crate::kosync_sync`; the protocol in `bookx_serve::kosync_client`. Every network call runs
//! on a worker thread.

use crate::i18n::tr;
use crate::i18n::trf;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use bookx_serve::kosync_client::{document_id, Push, Remote};
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use super::SharedRef;
use crate::kosync_queue as queue;
use crate::kosync_sync::{self as ks, Decision, KoSyncPrefs};

/// How long a page turn waits for the next one before the position is pushed.
const DEBOUNCE_MS: u64 = 4000;

struct Session {
    path: PathBuf,
    /// The position the server is known to have from us (or that we got from it).
    last_pct: f64,
    /// The user answered the question, or there was nothing to ask.
    answered: bool,
    pending: Option<(f64, usize)>,
    generation: u64,
}

thread_local! {
    static SESSIONS: RefCell<HashMap<u64, Session>> = RefCell::new(HashMap::new());
    /// What the open question does (`true` = continue); the UI tests answer through it.
    static PENDING_PROMPT: RefCell<Option<Rc<dyn Fn(bool)>>> = const { RefCell::new(None) };
}

fn poll<T: 'static>(rx: std::sync::mpsc::Receiver<T>, done: impl FnOnce(T) + 'static) {
    let done = RefCell::new(Some(done));
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        match rx.try_recv() {
            Ok(v) => {
                if let Some(f) = done.borrow_mut().take() {
                    f(v);
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

/// The settings with a device id (made and saved on first use), when syncing is switched on.
fn ready_prefs(shared: &SharedRef) -> Option<KoSyncPrefs> {
    let mut c = shared.cfg.borrow_mut();
    if !c.kosync.ready() {
        return None;
    }
    if c.kosync.ensure_device_id() {
        c.save();
    }
    Some(c.kosync.clone())
}

// -- the reader's side ----------------------------------------------------------------------------

/// Called when a book is opened in the reader: asks the server where the book is up to. `anchor`
/// is any widget of the reader (the question is shown over its window), `goto` moves the reader to
/// a chapter and a place in it.
pub fn open_book(
    shared: &SharedRef,
    book_id: u64,
    weights: Vec<f64>,
    anchor: gtk::Widget,
    goto: Rc<dyn Fn(usize, f64)>,
) {
    let Some(prefs) = ready_prefs(shared) else {
        return;
    };
    let (path, local) = {
        let lib = shared.lib.borrow();
        match lib.get(book_id) {
            Some(b) => (lib.book_path(b), b.progress),
            None => return,
        }
    };
    SESSIONS.with(|s| {
        s.borrow_mut().insert(
            book_id,
            Session {
                path: path.clone(),
                last_pct: local,
                answered: false,
                pending: None,
                generation: 0,
            },
        )
    });
    let (tx, rx) = std::sync::mpsc::channel();
    let p = prefs.clone();
    std::thread::spawn(move || {
        let r = (|| -> anyhow::Result<(String, Option<Remote>)> {
            let doc = document_id(&path, p.method())?;
            let remote = p.client()?.get_progress(&doc)?;
            Ok((doc, remote))
        })()
        .map_err(|e| format!("{e:#}"));
        let _ = tx.send(r);
    });
    let shared = shared.clone();
    poll(rx, move |r| {
        let (doc, remote) = match r {
            Ok(v) => v,
            Err(e) => {
                eprintln!("kosync_pull: error: {e}");
                return;
            }
        };
        eprintln!(
            "kosync_pull: doc={doc} remote={} local={:.3}",
            remote
                .as_ref()
                .map_or("none".to_string(), |r| format!("{:.3}", r.percentage)),
            local
        );
        let set = |f: &dyn Fn(&mut Session)| {
            SESSIONS.with(|s| s.borrow_mut().get_mut(&book_id).map(f));
        };
        let Some(remote) = remote else {
            set(&|s| s.answered = true);
            return;
        };
        let local_now = shared
            .lib
            .borrow()
            .get(book_id)
            .map_or(local, |b| b.progress);
        match ks::decide(local_now, &remote, &prefs.device_id) {
            Decision::Nothing => set(&|s| s.answered = true),
            Decision::Offer {
                percentage,
                device,
                when,
            } => {
                let title = ks::prompt_title(&device, percentage);
                eprintln!("kosync_prompt: {title}");
                let answer: Rc<dyn Fn(bool)> = {
                    let weights = weights.clone();
                    Rc::new(move |go| {
                        PENDING_PROMPT.with(|p| p.borrow_mut().take());
                        SESSIONS.with(|s| {
                            if let Some(s) = s.borrow_mut().get_mut(&book_id) {
                                s.answered = true;
                                if go {
                                    s.last_pct = percentage;
                                }
                            }
                        });
                        if go {
                            let (spine, frac) = ks::position_of(&weights, percentage);
                            eprintln!("kosync_goto: spine={spine} frac={frac:.3}");
                            goto(spine, frac);
                        }
                    })
                };
                PENDING_PROMPT.with(|p| *p.borrow_mut() = Some(answer.clone()));
                let d =
                    adw::AlertDialog::new(Some(&title), Some(&ks::prompt_body(local_now, when)));
                d.add_responses(&[("stay", &tr("Stay Here")), ("go", &tr("Continue"))]);
                d.set_response_appearance("go", adw::ResponseAppearance::Suggested);
                d.set_default_response(Some("go"));
                d.set_close_response("stay");
                d.connect_response(None, move |_, r| answer(r == "go"));
                d.present(Some(&anchor));
            }
        }
    });
}

/// Called whenever the reader's position is saved. Does nothing without a session (sync off).
pub fn position(shared: &SharedRef, book_id: u64, spine: usize, pct: f64) {
    let gen = SESSIONS.with(|s| {
        let mut s = s.borrow_mut();
        let sess = s.get_mut(&book_id)?;
        if !ks::worth_pushing(sess.last_pct, pct) {
            sess.pending = None;
            return None;
        }
        sess.pending = Some((pct, spine));
        sess.generation += 1;
        Some(sess.generation)
    });
    let Some(gen) = gen else {
        return;
    };
    let shared = shared.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(DEBOUNCE_MS), move || {
        let current = SESSIONS.with(|s| s.borrow().get(&book_id).map(|s| s.generation));
        if current == Some(gen) {
            flush(&shared, book_id);
        }
    });
}

/// Pushes the pending position now, if there is one.
pub fn flush(shared: &SharedRef, book_id: u64) {
    let Some(prefs) = ready_prefs(shared) else {
        return;
    };
    let job = SESSIONS.with(|s| {
        let mut s = s.borrow_mut();
        let sess = s.get_mut(&book_id)?;
        let (pct, spine) = sess.pending.take()?;
        Some((sess.path.clone(), pct, spine, sess.answered))
    });
    let Some((path, pct, spine, answered)) = job else {
        return;
    };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let qpath = queue::queue_path();
        let doc = document_id(&path, prefs.method()).map_err(|e| format!("{e:#}"));
        let r = match &doc {
            Err(e) => Err(e.clone()),
            Ok(doc) => (|| -> anyhow::Result<Option<i64>> {
                let client = prefs.client()?;
                if !answered {
                    let remote = client.get_progress(doc)?;
                    if !ks::push_allowed(remote.as_ref(), pct, &prefs.device_id, false) {
                        return Ok(None);
                    }
                }
                let ts = client.put_progress(&Push {
                    document: doc,
                    progress: &ks::progress_string(spine),
                    percentage: pct,
                    device: &prefs.display_name(),
                    device_id: &prefs.device_id,
                })?;
                Ok(Some(ts))
            })()
            .inspect_err(|e| {
                if let (Some(q), true) = (&qpath, queue::is_transient(e)) {
                    queue::update(q, |q| q.push(entry_for(&path, doc, &prefs, pct, spine)));
                }
            })
            .map_err(|e| format!("{e:#}")),
        };
        if let (Ok(Some(_)), Some(q)) = (&r, &qpath) {
            queue::update(q, |q| q.forget(&path.to_string_lossy()));
        }
        let _ = tx.send(r);
    });
    poll(rx, move |r| {
        match r {
        Ok(Some(ts)) => {
            eprintln!("kosync_push: pct={pct:.3} ok ts={}", ts > 0);
            SESSIONS.with(|s| {
                if let Some(s) = s.borrow_mut().get_mut(&book_id) {
                    s.last_pct = pct;
                }
            });
        }
        Ok(None) => eprintln!(
            "kosync_push: pct={pct:.3} held back, another device is further along and you have not answered yet"
        ),
        Err(e) => eprintln!("kosync_push: error: {e}"),
    }
    });
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

fn entry_for(
    path: &std::path::Path,
    doc: &str,
    prefs: &KoSyncPrefs,
    pct: f64,
    spine: usize,
) -> queue::Entry {
    queue::Entry {
        path: path.to_string_lossy().into_owned(),
        document: doc.to_string(),
        progress: ks::progress_string(spine),
        percentage: pct,
        device: prefs.display_name(),
        device_id: prefs.device_id.clone(),
        queued_at: now_secs(),
        attempts: 0,
        next_try: now_secs() + queue::backoff(1),
    }
}

/// Sends the queued entries with the saved settings.
struct QueuePusher(KoSyncPrefs);

impl queue::Pusher for QueuePusher {
    fn push(&self, e: &queue::Entry) -> anyhow::Result<bool> {
        let client = self.0.client()?;
        let remote = client.get_progress(&e.document)?;
        if !ks::push_allowed(remote.as_ref(), e.percentage, &e.device_id, false) {
            return Ok(false);
        }
        client.put_progress(&Push {
            document: &e.document,
            progress: &e.progress,
            percentage: e.percentage,
            device: &e.device,
            device_id: &e.device_id,
        })?;
        Ok(true)
    }
}

thread_local! {
    static QUEUE_RUNNING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Retries the queued pushes that are due (`force`: all of them, whatever the pause).
pub fn flush_queue(shared: &SharedRef, force: bool) {
    let Some(prefs) = ready_prefs(shared) else {
        return;
    };
    let Some(path) = queue::queue_path() else {
        return;
    };
    if QUEUE_RUNNING.with(|r| r.replace(true)) {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let now = if force { i64::MAX / 2 } else { now_secs() };
        let r = queue::flush(&path, now, &QueuePusher(prefs));
        let _ = tx.send(r);
    });
    poll(rx, |r| {
        QUEUE_RUNNING.with(|q| q.set(false));
        if r.sent + r.kept + r.dropped > 0 {
            eprintln!(
                "kosync_queue_flush: sent={} kept={} dropped={}",
                r.sent, r.kept, r.dropped
            );
        }
    });
}

/// Starts the retries: now (what was left from the last run), every minute, and when the network
/// comes back.
pub fn start_queue(shared: &SharedRef) {
    let s = shared.clone();
    glib::idle_add_local_once(move || flush_queue(&s, false));
    let s = shared.clone();
    glib::timeout_add_seconds_local(60, move || {
        flush_queue(&s, false);
        glib::ControlFlow::Continue
    });
    let s = shared.clone();
    relm4::gtk::gio::NetworkMonitor::default().connect_network_changed(move |_, up| {
        if up {
            flush_queue(&s, false);
        }
    });
}

/// Called when the reader closes: the last position goes out at once.
pub fn close_book(shared: &SharedRef, book_id: u64) {
    flush(shared, book_id);
    SESSIONS.with(|s| s.borrow_mut().remove(&book_id));
}

// -- Preferences ---------------------------------------------------------------------------------

/// "Register" or "Log in" with the saved settings; `report` gets the outcome in words.
pub fn account_action(shared: &SharedRef, register: bool, report: Rc<dyn Fn(String)>) {
    let p = shared.cfg.borrow().kosync.clone();
    let client = match p.client() {
        Ok(c) => c,
        Err(e) => {
            report(format!("{e:#}"));
            return;
        }
    };
    if p.password.is_empty() {
        report("Enter the password".into());
        return;
    }
    report(if register {
        "Registering…".into()
    } else {
        "Logging in…".into()
    });
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = if register {
            client.register().and_then(|_| client.login())
        } else {
            client.login()
        };
        let _ = tx.send(r.map_err(|e| format!("{e:#}")));
    });
    poll(rx, move |r| {
        let msg = match r {
            Ok(()) if register => {
                "Registered and logged in. Reading positions will sync.".to_string()
            }
            Ok(()) => "Logged in. Reading positions will sync.".to_string(),
            Err(e) => e,
        };
        eprintln!("kosync_account: {msg}");
        report(msg);
    });
}

pub const TITLE: &str = "Sync reading position (KOSync)";
pub fn description() -> String {
    trf("Keeps your place in step with KOReader and other apps through a KOReader progress-sync server: Bookx’s own Share Library server (address ending in /kosync), a self-hosted kosync, or the public one at sync.koreader.rocks. Only the position is synced, not highlights. {}", &[&crate::secrets::caption_now()])
}

/// The settings group; `devices_page.add(&group)` in Preferences.
pub fn preferences_group(shared: &SharedRef) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(TITLE);
    g.set_description(Some(&description()));
    let cur = shared.cfg.borrow().kosync.clone();
    let on = adw::SwitchRow::builder()
        .title(tr("Sync the reading position"))
        .subtitle(tr("Asks “Continue from …?” when a book was read further on another device, and shares your place as you read."))
        .build();
    on.set_active(cur.enabled);
    let entry = |title: &str, text: &str| {
        let r = adw::EntryRow::new();
        r.set_title(title);
        r.set_text(text);
        r
    };
    let server = entry(&tr("Server address"), &cur.server);
    let user = entry(&tr("User name"), &cur.user);
    let pass = adw::PasswordEntryRow::new();
    pass.set_title(&tr("Password"));
    pass.set_text(&cur.password);
    let method = adw::ComboRow::new();
    method.set_title(&tr("Match books by"));
    method.set_subtitle(&tr(
        "Use the same as KOReader (Tools → Progress sync → Document matching method)",
    ));
    method.set_model(Some(&gtk::StringList::new(&[
        "Partial MD5 of the file (KOReader’s default)",
        "File name",
    ])));
    method.set_selected(cur.by_filename as u32);
    let name = entry(&tr("This device is called"), &cur.device_name);
    let status = adw::ActionRow::new();
    status.set_title(&tr("Account"));
    status.set_subtitle(&tr(
        "Register creates the account on the server; Log in checks it.",
    ));
    let register = gtk::Button::with_label(&tr("Register"));
    let login = gtk::Button::with_label(&tr("Log in"));
    for b in [&register, &login] {
        b.set_valign(gtk::Align::Center);
        status.add_suffix(b);
    }
    g.add(&on);
    g.add(&server);
    g.add(&user);
    g.add(&pass);
    g.add(&method);
    g.add(&name);
    g.add(&status);
    let save = {
        let shared = shared.clone();
        let (on, server, user, pass, method, name) = (
            on.clone(),
            server.clone(),
            user.clone(),
            pass.clone(),
            method.clone(),
            name.clone(),
        );
        Rc::new(move || {
            let mut c = shared.cfg.borrow_mut();
            c.kosync.enabled = on.is_active();
            c.kosync.server = server.text().trim().to_string();
            c.kosync.user = user.text().trim().to_string();
            c.kosync.password = pass.text().to_string();
            c.kosync.by_filename = method.selected() == 1;
            c.kosync.device_name = name.text().trim().to_string();
            c.save();
        })
    };
    for r in [&server, &user, &name] {
        let f = save.clone();
        r.connect_changed(move |_| f());
    }
    let f = save.clone();
    pass.connect_changed(move |_| f());
    let f = save.clone();
    on.connect_active_notify(move |_| f());
    let f = save.clone();
    method.connect_selected_notify(move |_| f());
    for (button, is_register) in [(&register, true), (&login, false)] {
        let (shared, save, status) = (shared.clone(), save.clone(), status.clone());
        button.connect_clicked(move |_| {
            save();
            let status = status.clone();
            account_action(
                &shared,
                is_register,
                Rc::new(move |m| status.set_subtitle(&glib::markup_escape_text(&m))),
            );
        });
    }
    g
}

// -- test hooks ---------------------------------------------------------------------------------------

/// `kosync:<command>` in `BOOKX_DEV_SCRIPT`: `setup=<url>;<user>;<password>[;filename]` (switches
/// syncing on), `register`, `login`, `continue` / `stay` (answer the question), `flush`, `dump`.
pub fn dev_command(shared: &SharedRef, cmd: &str) {
    if let Some(v) = cmd.strip_prefix("setup=") {
        let parts: Vec<&str> = v.split(';').collect();
        let mut c = shared.cfg.borrow_mut();
        c.kosync.enabled = true;
        c.kosync.server = parts.first().unwrap_or(&"").to_string();
        c.kosync.user = parts.get(1).unwrap_or(&"").to_string();
        c.kosync.password = parts.get(2).unwrap_or(&"").to_string();
        c.kosync.by_filename = parts.get(3) == Some(&"filename");
        c.kosync.device_name = "Bookx test".into();
        c.save();
        return;
    }
    match cmd {
        "register" | "login" => account_action(shared, cmd == "register", Rc::new(|_| {})),
        "continue" | "stay" => match PENDING_PROMPT.with(|p| p.borrow_mut().take()) {
            Some(f) => f(cmd == "continue"),
            None => eprintln!("kosync_prompt: none"),
        },
        "flush" => {
            let ids: Vec<u64> = SESSIONS.with(|s| s.borrow().keys().copied().collect());
            for id in ids {
                flush(shared, id);
            }
        }
        "queue-flush" => flush_queue(shared, true),
        "queue-dump" => {
            let n = queue::queue_path()
                .map(|p| queue::Queue::load(&p).entries.len())
                .unwrap_or(0);
            eprintln!("kosync_queue: entries={n}");
        }
        "dump" => {
            let c = shared.cfg.borrow();
            eprintln!(
                "kosync_state: enabled={} ready={} device_id_set={} sessions={}",
                c.kosync.enabled,
                c.kosync.ready(),
                !c.kosync.device_id.is_empty(),
                SESSIONS.with(|s| s.borrow().len())
            );
        }
        other => eprintln!("kosync: unknown command {other}"),
    }
}
