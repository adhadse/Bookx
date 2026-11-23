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

use bookx_serve::kosync_client::{Push, Remote, document_id};
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
    let mut t = crate::kosync_sync::trace_op("pull", book_id, &path, &prefs);
    t.ratio("local", local);
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
                t.fail(&e);
                eprintln!("kosync_pull: error: {e}");
                return;
            }
        };
        t.hash("doc", &doc);
        match &remote {
            Some(r) => t.ratio("remote", r.percentage),
            None => t.word("remote", "none"),
        };
        t.ok();
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
    let r#gen = SESSIONS.with(|s| {
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
    let Some(r#gen) = r#gen else {
        return;
    };
    let shared = shared.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(DEBOUNCE_MS), move || {
        let current = SESSIONS.with(|s| s.borrow().get(&book_id).map(|s| s.generation));
        if current == Some(r#gen) {
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
    let mut t = crate::kosync_sync::trace_op("push", book_id, &path, &prefs);
    t.ratio("local", pct);
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
    poll(rx, move |r| match r {
        Ok(Some(ts)) => {
            t.ok();
            eprintln!("kosync_push: pct={pct:.3} ok ts={}", ts > 0);
            SESSIONS.with(|s| {
                if let Some(s) = s.borrow_mut().get_mut(&book_id) {
                    s.last_pct = pct;
                }
            });
        }
        Ok(None) => {
            t.word("note", "held_back").ok();
            eprintln!(
                "kosync_push: pct={pct:.3} held back, another device is further along and you have not answered yet"
            )
        }
        Err(e) => {
            t.fail(&e);
            eprintln!("kosync_push: error: {e}")
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
            crate::trace::event(crate::trace::Area::Sync, "queue")
                .num("sent", r.sent as u64)
                .num("kept", r.kept as u64)
                .num("dropped", r.dropped as u64)
                .ok();
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

/// "Register" or "Log in" with the saved settings; `report` gets the outcome in words and `ok` is
/// called when the server accepted the account.
pub fn account_action(
    shared: &SharedRef,
    register: bool,
    report: Rc<dyn Fn(String)>,
    ok: Rc<dyn Fn()>,
) {
    let p = shared.cfg.borrow().kosync.clone();
    let client = match p.client() {
        Ok(c) => c,
        Err(e) => {
            report(format!("{e:#}"));
            return;
        }
    };
    if p.password.is_empty() {
        report(tr("Enter the password"));
        return;
    }
    report(if register {
        tr("Registering")
    } else {
        tr("Logging in")
    });
    let mut t = crate::trace::op(
        crate::trace::Area::Sync,
        if register { "register" } else { "login" },
    );
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
                ok();
                tr("Registered and logged in. Reading positions will sync.")
            }
            Ok(()) => {
                ok();
                tr("Logged in. Reading positions will sync.")
            }
            Err(e) => {
                t.fail(&e);
                crate::sync_model::account_hint(&e, register)
            }
        };
        t.ok();
        eprintln!("kosync_account: {msg}");
        report(msg);
    });
}

/// What the reading-position switch says (Advanced).
pub fn position_help() -> String {
    trf(
        "Keeps your place in step with KOReader and other apps through a sync server: your own Bookx server (the address ending in /bookx), a self-hosted one, or the public one at sync.koreader.rocks. {}",
        &[&crate::secrets::caption_now()],
    )
}

fn entry(title: &str, text: &str) -> adw::EntryRow {
    let r = adw::EntryRow::new();
    r.set_title(title);
    r.set_text(text);
    r
}

/// The address of Bookx’s own running server, the one the Sync window shows (ending in /bookx).
fn own_server_url(shared: &SharedRef) -> Option<String> {
    crate::sharing::kosync_url(&shared.sharing.borrow().urls())
}

struct Account {
    server: adw::EntryRow,
    user: adw::EntryRow,
    pass: adw::PasswordEntryRow,
    register: adw::ButtonRow,
    login: gtk::Button,
    status: adw::ActionRow,
}

thread_local! {
    static ACCOUNT: RefCell<Option<Account>> = const { RefCell::new(None) };
}

/// The server address row of the account (it sits under Advanced).
pub fn server_row() -> Option<adw::EntryRow> {
    ACCOUNT.with(|a| a.borrow().as_ref().map(|a| a.server.clone()))
}

/// Dev hook `prefs:sync-account…`: `=<user>;<password>[;<server>]` fills the fields, `-register` and
/// `-login` press the buttons, `-dump` prints what the step shows.
fn count_buttons(w: &gtk::Widget) -> usize {
    let mut n = usize::from(w.is::<gtk::Button>());
    let mut c = w.first_child();
    while let Some(ch) = c {
        n += count_buttons(&ch);
        c = ch.next_sibling();
    }
    n
}

pub fn dev_account(cmd: &str) {
    ACCOUNT.with(|a| {
        let a = a.borrow();
        let Some(a) = &*a else {
            eprintln!("prefs_account: no page");
            return;
        };
        if let Some(v) = cmd.strip_prefix('=') {
            let p: Vec<&str> = v.split(';').collect();
            a.user.set_text(p.first().unwrap_or(&""));
            a.pass.set_text(p.get(1).unwrap_or(&""));
            if let Some(s) = p.get(2) {
                a.server.set_text(s);
            }
        } else if cmd == "-register" {
            a.register.emit_by_name::<()>("activated", &[]);
        } else if cmd == "-login" {
            a.login.emit_clicked();
        } else if cmd == "-dump" {
            eprintln!(
                "prefs_account: server={:?} user={:?} register={} login={} status={:?} status_title={:?} register_sensitive={} login_sensitive={} login_tip={:?} status_controls={}",
                a.server.text(),
                a.user.text(),
                a.register.is_visible(),
                a.login.is_visible(),
                a.status.subtitle().unwrap_or_default(),
                a.status.title(),
                a.register.is_sensitive(),
                a.login.is_sensitive(),
                a.login.tooltip_text().unwrap_or_default(),
                count_buttons(a.status.upcast_ref())
            );
        }
    });
}

/// The rows of step 2 (user name, password, Register / Log in with the result inline) and the
/// server address row for Advanced. It is the Bookx sync account, not the OPDS server password.
pub fn account_rows(shared: &SharedRef) -> (Vec<gtk::Widget>, adw::EntryRow) {
    let cur = shared.cfg.borrow().kosync.clone();
    let server = entry(&tr("Server address"), &cur.server);
    let user = entry(&tr("User name"), &cur.user);
    let pass = adw::PasswordEntryRow::new();
    pass.set_title(&tr("Password"));
    pass.set_text(&cur.password);
    let use_own = gtk::Button::with_label(&tr("Use Bookx server"));
    use_own.set_valign(gtk::Align::Center);
    use_own.set_tooltip_text(Some(&tr(
        "Fill in the address of the OPDS server that is running here (Sync shows the same)",
    )));
    super::audit::label(
        &use_own,
        &tr("Use the address of the Bookx server of this computer"),
    );
    server.add_suffix(&use_own);
    server.set_title(&tr("Server address (Advanced)"));
    let status = adw::ActionRow::new();
    status.set_title(&tr("Account"));
    status.set_subtitle(&tr(
        "Register creates the account on the server; Log in checks it. The result shows here.",
    ));
    status.set_subtitle_lines(0);
    let show_status = {
        let (shared, status) = (shared.clone(), status.clone());
        Rc::new(move || {
            let c = shared.cfg.borrow().kosync.clone();
            let (title, text) = crate::sync_model::account_status(
                &c.user,
                !c.password.is_empty(),
                c.logged_in,
                crate::secrets::vault().available(),
            );
            status.set_title(&glib::markup_escape_text(&title));
            status.set_subtitle(&glib::markup_escape_text(&text));
        })
    };
    show_status();
    {
        let f = show_status.clone();
        super::share::follow(&status, move || f());
    }
    let register = adw::ButtonRow::new();
    register.set_title(&tr("Register"));
    register.add_css_class("suggested-action");
    let login = gtk::Button::with_label(&tr("Log in"));
    login.set_valign(gtk::Align::Center);
    status.add_suffix(&login);
    super::audit::label(&login, &tr("Log in with the user name and password"));

    let save = {
        let shared = shared.clone();
        let (server, user, pass) = (server.clone(), user.clone(), pass.clone());
        Rc::new(move || {
            let changed = {
                let mut c = shared.cfg.borrow_mut();
                let before = (c.kosync.user.clone(), c.kosync.password.is_empty());
                let creds = (
                    c.kosync.server.clone(),
                    c.kosync.user.clone(),
                    c.kosync.password.clone(),
                );
                c.kosync.server = server.text().trim().to_string();
                c.kosync.user = user.text().trim().to_string();
                c.kosync.password = pass.text().to_string();
                let edited = creds
                    != (
                        c.kosync.server.clone(),
                        c.kosync.user.clone(),
                        c.kosync.password.clone(),
                    );
                if edited {
                    c.kosync.logged_in = false;
                }
                c.save();
                edited || before != (c.kosync.user.clone(), c.kosync.password.is_empty())
            };
            // the Account step shows “Done” as soon as a user name and a password are there
            if changed {
                crate::settings_bus::sync().notify();
            }
        })
    };
    for r in [&server, &user] {
        let f = save.clone();
        r.connect_changed(move |_| f());
    }
    let f = save.clone();
    pass.connect_changed(move |_| f());

    // an empty address is filled from the server that runs here: the one the Sync window shows
    let fill = {
        let (shared, server) = (shared.clone(), server.clone());
        Rc::new(move |force: bool| {
            if force || server.text().trim().is_empty() {
                if let Some(u) = own_server_url(&shared) {
                    if server.text() != u {
                        server.set_text(&u);
                    }
                }
            }
        })
    };
    fill(false);
    {
        let f1 = fill.clone();
        use_own.connect_clicked(move |_| f1(true));
        let f2 = fill.clone();
        super::share::follow(&status, move || f2(false));
    }
    // Log in and Register cannot work without a server: insensitive, with the reason as the tooltip
    let gate = {
        let (shared, server, register, login) = (
            shared.clone(),
            server.clone(),
            register.clone(),
            login.clone(),
        );
        Rc::new(move || {
            let own = own_server_url(&shared);
            let blocked = crate::sync_model::account_block(
                &server.text(),
                own.as_deref(),
                shared.cfg.borrow().share.kosync,
            )
            .is_some();
            let tip = blocked.then(|| tr(crate::sync_model::ACCOUNT_BLOCKED_TIP));
            for w in [register.upcast_ref::<gtk::Widget>(), login.upcast_ref()] {
                w.set_sensitive(!blocked);
                w.set_tooltip_text(tip.as_deref());
            }
        })
    };
    gate();
    {
        let g = gate.clone();
        server.connect_changed(move |_| g());
        let g = gate.clone();
        super::share::follow(&status, move || g());
    }
    let handlers: [(Box<dyn Fn(Box<dyn Fn() + 'static>)>, bool); 2] = [
        (
            {
                let r = register.clone();
                Box::new(move |f| {
                    r.connect_activated(move |_| f());
                })
            },
            true,
        ),
        (
            {
                let l = login.clone();
                Box::new(move |f| {
                    l.connect_clicked(move |_| f());
                })
            },
            false,
        ),
    ];
    for (connect, is_register) in handlers {
        let (shared, save, status, fill, server) = (
            shared.clone(),
            save.clone(),
            status.clone(),
            fill.clone(),
            server.clone(),
        );
        connect(Box::new(move || {
            fill(false);
            save();
            let text = server.text().trim().to_string();
            let own = own_server_url(&shared);
            let say = |m: String| status.set_subtitle(&glib::markup_escape_text(&m));
            if text.is_empty() {
                say(tr(crate::sync_model::ACCOUNT_NO_SERVER));
                return;
            }
            if own
                .as_deref()
                .is_some_and(|u| u == text.trim_end_matches('/'))
                && !shared.cfg.borrow().share.kosync
            {
                say(tr(crate::sync_model::ACCOUNT_KOSYNC_OFF));
                return;
            }
            let sh = shared.clone();
            let st = status.clone();
            account_action(
                &shared,
                is_register,
                Rc::new(move |m| st.set_subtitle(&glib::markup_escape_text(&m))),
                Rc::new(move || {
                    // the account works: reading positions sync from now on
                    let was = {
                        let mut c = sh.cfg.borrow_mut();
                        let was = c.kosync.enabled && c.kosync.logged_in;
                        c.kosync.enabled = true;
                        c.kosync.logged_in = true;
                        c.save();
                        was
                    };
                    if !was {
                        crate::settings_bus::sync().notify();
                    }
                }),
            );
        }));
    }
    ACCOUNT.with(|a| {
        *a.borrow_mut() = Some(Account {
            server: server.clone(),
            user: user.clone(),
            pass: pass.clone(),
            register: register.clone(),
            login: login.clone(),
            status: status.clone(),
        })
    });
    (
        vec![
            user.upcast::<gtk::Widget>(),
            pass.upcast(),
            register.upcast(),
            status.upcast(),
        ],
        server,
    )
}

/// The reading-position settings for Advanced (switch, matching, device name); the account rows
/// are [`account_rows`].
pub fn position_rows(shared: &SharedRef) -> Vec<gtk::Widget> {
    let cur = shared.cfg.borrow().kosync.clone();
    let on = adw::SwitchRow::builder()
        .title(tr("Sync the reading position"))
        .subtitle(tr("Asks “Continue from …?” when a book was read further on another device, and shares your place as you read."))
        .build();
    on.set_active(cur.enabled);
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
    let save = {
        let shared = shared.clone();
        let (on, method, name) = (on.clone(), method.clone(), name.clone());
        Rc::new(move || {
            let was = {
                let mut c = shared.cfg.borrow_mut();
                let was = c.kosync.enabled;
                c.kosync.enabled = on.is_active();
                c.kosync.by_filename = method.selected() == 1;
                c.kosync.device_name = name.text().trim().to_string();
                c.save();
                was
            };
            if was != on.is_active() {
                crate::settings_bus::sync().notify();
            }
        })
    };
    let f = save.clone();
    name.connect_changed(move |_| f());
    let f = save.clone();
    on.connect_active_notify(move |_| f());
    let f = save.clone();
    method.connect_selected_notify(move |_| f());
    // a successful Register or Log in switches it on: follow
    let (sh, on2) = (shared.clone(), on.downgrade());
    super::share::follow(&on, move || {
        if let Some(o) = on2.upgrade() {
            let want = sh.cfg.borrow().kosync.enabled;
            if o.is_active() != want {
                o.set_active(want);
            }
        }
    });
    vec![on.upcast(), method.upcast(), name.upcast()]
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
        "register" | "login" => {
            account_action(shared, cmd == "register", Rc::new(|_| {}), Rc::new(|| {}))
        }
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
