//! The dictionary downloads of the app: one job per language in the Background Jobs panel, started by
//! the look-up popover (a click), by a new book in the library, or by opening a book. The rules are in
//! `dict_auto`, the installer in `dict_install`; this is the glue to the main loop, the jobs and the
//! settings.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use relm4::gtk::glib;

use super::SharedRef;
use crate::dict_auto::{self, Ask, Decision, Failures};
use crate::dict_catalog::{catalog, language_name, normalize_language};
use crate::dict_install;
use crate::i18n::{tr, trf};
use crate::jobs;
use bookx_tts::synth::Cancel;

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Progress {
        lang: String,
        fraction: Option<f64>,
        line: String,
    },
    Done {
        lang: String,
    },
    Failed {
        lang: String,
        message: String,
        offline: bool,
    },
}

enum Msg {
    Bytes(u64, u64),
    Done(Result<dict_install::Installed, String>),
}

struct Active {
    rx: Receiver<Msg>,
    cancel: Cancel,
    job: u64,
    token: jobs::CancelToken,
    fraction: Option<f64>,
    line: String,
    title: String,
}

#[derive(Default)]
struct State {
    active: HashMap<String, Active>,
    failures: Failures,
    failed: HashMap<String, (String, bool)>,
    observers: Vec<(u64, Rc<dyn Fn(&Event)>)>,
    next: u64,
    notifier: Option<Rc<dyn Fn(String)>>,
    /// Requests that were answered with a toast about consent already.
    told: HashSet<String>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

pub struct Subscription(u64);

impl Drop for Subscription {
    fn drop(&mut self) {
        let id = self.0;
        let _ = STATE.try_with(|s| s.borrow_mut().observers.retain(|(i, _)| *i != id));
    }
}

pub fn subscribe(f: impl Fn(&Event) + 'static) -> Subscription {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.next += 1;
        let id = s.next;
        s.observers.push((id, Rc::new(f)));
        Subscription(id)
    })
}

fn emit(e: Event) {
    let obs: Vec<Rc<dyn Fn(&Event)>> = STATE.with(|s| {
        s.borrow()
            .observers
            .iter()
            .map(|(_, f)| f.clone())
            .collect()
    });
    for f in obs {
        f(&e);
    }
}

pub fn set_notifier(f: Rc<dyn Fn(String)>) {
    STATE.with(|s| s.borrow_mut().notifier = Some(f));
}

fn toast(m: String) {
    if let Some(n) = STATE.with(|s| s.borrow().notifier.clone()) {
        n(m);
    }
}

/// The progress of the download of a language, when it runs.
pub fn running(lang: &str) -> Option<(Option<f64>, String)> {
    STATE.with(|s| {
        s.borrow()
            .active
            .get(lang)
            .map(|a| (a.fraction, a.line.clone()))
    })
}

/// The message of the last failure of a language (and whether it looks like no network).
pub fn last_failure(lang: &str) -> Option<(String, bool)> {
    STATE.with(|s| s.borrow().failed.get(lang).cloned())
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Decides and, when the answer is to start, starts the download of the dictionary for `lang`.
/// `explicit`: the user pressed a button (no switch, no waiting, no consent needed).
pub fn request(shared: &SharedRef, lang: &str, explicit: bool) -> Decision {
    let Some(lang) = normalize_language(lang) else {
        return Decision::NoSource;
    };
    let dir = crate::dictionaries::dictionary_dir();
    let presence = dict_install::presence(&dir);
    let (running, failures) = STATE.with(|s| {
        let s = s.borrow();
        (
            s.active.keys().cloned().collect::<HashSet<_>>(),
            s.failures.clone(),
        )
    });
    let (auto, big_ok) = {
        let c = shared.cfg.borrow();
        (
            dict_auto::auto_enabled(&c.dictionaries),
            c.dictionaries.big_ok.clone(),
        )
    };
    let d = dict_auto::decide(&Ask {
        lang: &lang,
        explicit,
        auto,
        presence: &presence,
        running: &running,
        offline: dict_auto::offline(),
        failures: &failures,
        now: now_secs(),
        big_ok: &big_ok,
        catalog: catalog(),
    });
    match &d {
        Decision::Start { dict } => start(shared, &lang, dict, &dir),
        Decision::Offline | Decision::Wait(_) if !explicit => {
            // tried again later
            let mut c = shared.cfg.borrow_mut();
            if !c.dictionaries.pending.contains(&lang) {
                c.dictionaries.pending.push(lang.clone());
                c.save();
            }
        }
        Decision::NeedsConsent { size } => {
            let first = STATE.with(|s| s.borrow_mut().told.insert(lang.clone()));
            if first {
                toast(trf(
                    "The {} dictionary is big ({}). Open a book in that language and press Download dictionary.",
                    &[&language_name(&lang), &dict_auto::size_text(*size)],
                ));
            }
        }
        _ => {}
    }
    d
}

fn start(shared: &SharedRef, lang: &str, dict_id: &str, dir: &std::path::Path) {
    let Some((_, dict)) = catalog().find(dict_id) else {
        return;
    };
    let dict = dict.clone();
    let title = trf("Dictionary for {}", &[&language_name(lang)]);
    let (job, token) = jobs::start(jobs::Kind::Dictionary, &title, true);
    jobs::log(job, &format!("{} ({})", dict.title, dict.licence));
    jobs::log(
        job,
        &format!(
            "From {}",
            dict.origins
                .first()
                .map(|o| o.url.as_str())
                .unwrap_or_default()
        ),
    );
    let (tx, rx) = channel();
    let cancel = Cancel::new();
    let (c2, dir2, lang2) = (cancel.clone(), dir.to_path_buf(), lang.to_string());
    std::thread::spawn(move || {
        let tx2 = tx.clone();
        let r = dict_install::install(&dict, &lang2, &dir2, &c2, &mut |d, t| {
            let _ = tx2.send(Msg::Bytes(d, t));
        });
        let _ = tx.send(Msg::Done(r.map_err(|e| format!("{e:#}"))));
    });
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.failed.remove(lang);
        s.active.insert(
            lang.to_string(),
            Active {
                rx,
                cancel,
                job,
                token,
                fraction: None,
                line: tr("Starting"),
                title,
            },
        );
    });
    {
        let mut c = shared.cfg.borrow_mut();
        if !c.dictionaries.considered.iter().any(|l| l == lang) {
            c.dictionaries.considered.push(lang.to_string());
            c.save();
        }
    }
    emit(Event::Progress {
        lang: lang.to_string(),
        fraction: None,
        line: tr("Starting"),
    });
    let (shared, lang) = (shared.clone(), lang.to_string());
    glib::timeout_add_local(Duration::from_millis(150), move || poll(&shared, &lang));
}

fn poll(shared: &SharedRef, lang: &str) -> glib::ControlFlow {
    let mut events = Vec::new();
    let mut finished: Option<(u64, String, Result<dict_install::Installed, String>)> = None;
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let Some(a) = s.active.get_mut(lang) else {
            return;
        };
        if a.token.is_cancelled() {
            a.cancel.cancel();
        }
        while let Ok(m) = a.rx.try_recv() {
            match m {
                Msg::Bytes(d, t) => {
                    a.fraction = Some((d as f64 / t.max(1) as f64).clamp(0.0, 1.0));
                    a.line = trf(
                        "{} of {}",
                        &[&dict_auto::size_text(d), &dict_auto::size_text(t)],
                    );
                }
                Msg::Done(r) => finished = Some((a.job, a.title.clone(), r)),
            }
        }
        if finished.is_none() {
            events.push(Event::Progress {
                lang: lang.to_string(),
                fraction: a.fraction,
                line: a.line.clone(),
            });
            jobs::progress(a.job, a.fraction, &a.line);
        }
    });
    for e in events {
        emit(e);
    }
    let Some((job, title, result)) = finished else {
        return if STATE.with(|s| s.borrow().active.contains_key(lang)) {
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        };
    };
    STATE.with(|s| s.borrow_mut().active.remove(lang));
    match result {
        Ok(i) => {
            jobs::log(job, &format!("Installed in {}", i.dir.display()));
            jobs::finish(job, Ok(()));
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                s.failures.clear(lang);
                s.failed.remove(lang);
            });
            {
                let mut c = shared.cfg.borrow_mut();
                c.dictionaries.pending.retain(|l| l != lang);
                c.save();
            }
            toast(trf("Dictionary for {} ready", &[&language_name(lang)]));
            emit(Event::Done {
                lang: lang.to_string(),
            });
        }
        Err(m) => {
            let cancelled = m.contains("cancelled");
            let offline = dict_auto::looks_offline(&m);
            jobs::log(job, &m);
            if cancelled {
                jobs::cancel(job);
                jobs::finish(job, Err("cancelled".into()));
            } else {
                jobs::finish(job, Err(m.lines().next().unwrap_or("failed").to_string()));
                STATE.with(|s| {
                    let mut s = s.borrow_mut();
                    s.failures.record(lang, now_secs());
                    s.failed.insert(lang.to_string(), (m.clone(), offline));
                });
                if offline {
                    let mut c = shared.cfg.borrow_mut();
                    if !c.dictionaries.pending.iter().any(|l| l == lang) {
                        c.dictionaries.pending.push(lang.to_string());
                        c.save();
                    }
                }
            }
            let _ = title;
            emit(Event::Failed {
                lang: lang.to_string(),
                message: if cancelled { tr("Cancelled") } else { m },
                offline,
            });
        }
    }
    glib::ControlFlow::Break
}

/// The library changed (books were added): a language that is new gets its dictionary, by itself when
/// the setting is on.
pub fn library_changed(shared: &SharedRef) {
    let langs: Vec<String> = {
        let lib = shared.lib.borrow();
        let mut v: Vec<String> = lib
            .books
            .iter()
            .filter_map(|b| normalize_language(&b.meta.language))
            .collect();
        v.sort();
        v.dedup();
        v
    };
    let fresh = {
        let mut c = shared.cfg.borrow_mut();
        let before = c.dictionaries.clone();
        let f = dict_auto::new_languages(&langs, &mut c.dictionaries);
        if c.dictionaries != before {
            c.save();
        }
        f
    };
    for l in fresh {
        request(shared, &l, false);
    }
}

/// A book was opened: its language has no dictionary yet and the setting is on: start the download now.
pub fn book_opened(shared: &SharedRef, lang: &str) {
    if !lang.is_empty() {
        request(shared, lang, false);
    }
}

/// Languages that waited for the network are tried again (every few minutes and at start-up).
pub fn retry_pending(shared: &SharedRef) {
    let pending = shared.cfg.borrow().dictionaries.pending.clone();
    for l in pending {
        request(shared, &l, false);
    }
}

pub fn start_retry_timer(shared: &SharedRef) {
    let s = shared.clone();
    glib::timeout_add_seconds_local(300, move || {
        retry_pending(&s);
        glib::ControlFlow::Continue
    });
    let s = shared.clone();
    glib::timeout_add_seconds_local(20, move || {
        retry_pending(&s);
        glib::ControlFlow::Break
    });
}

/// Dev hook: what the service knows.
pub fn dev_dump(shared: &SharedRef) -> String {
    let (active, failed): (Vec<String>, Vec<String>) = STATE.with(|s| {
        let s = s.borrow();
        let mut a: Vec<String> = s.active.keys().cloned().collect();
        a.sort();
        let mut f: Vec<String> = s.failed.keys().cloned().collect();
        f.sort();
        (a, f)
    });
    let dir = crate::dictionaries::dictionary_dir();
    let managed: Vec<String> = dict_install::managed(&dir)
        .iter()
        .map(|i| format!("{}:{}", i.marker.lang, i.marker.id))
        .collect();
    let c = shared.cfg.borrow();
    format!(
        "active={:?} failed={:?} installed={:?} pending={:?} considered={:?} auto={}",
        active,
        failed,
        managed,
        c.dictionaries.pending,
        c.dictionaries.considered,
        dict_auto::auto_enabled(&c.dictionaries)
    )
}
