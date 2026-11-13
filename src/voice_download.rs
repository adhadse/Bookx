//! Voice downloads as an application-level job. The download is owned by the application (a worker
//! thread and a timer on the main loop), not by the reader or its player card, so it goes on when the
//! user leaves the book for the library. It is listed in the Background Jobs panel with percent, speed,
//! time left, a log and a Cancel button. Readers and the settings page only ask for it and look at
//! [`status`]. Packs are resumable: after a restart the same call continues from the partial files.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use bookx_tts::models::Pack;
use bookx_tts::setup::{progress_line, Failure, Throughput};
use relm4::gtk::glib;

use crate::i18n::tr;
use crate::jobs::{self, CancelToken};
use crate::tts::{self, Cancel};

/// What the world can see of the download.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Idle,
    Running {
        done: u64,
        total: u64,
        /// `42 MB of 130 MB · 3.6 MB/s · 25 s left`
        line: String,
    },
    /// The last download ended; `seq` is the number [`start`] returned for it.
    Finished {
        seq: u64,
        result: Result<(), Failure>,
    },
}

enum Msg {
    Progress {
        file: String,
        done: u64,
        total: u64,
        origin: usize,
    },
    Done(Result<(), Failure>),
}

struct Active {
    rx: Receiver<Msg>,
    cancel: Cancel,
    job: u64,
    token: CancelToken,
    seq: u64,
    tracker: Tracker,
    started: Instant,
}

#[derive(Default)]
struct Inner {
    status: Option<Status>,
    active: Option<Active>,
    seq: u64,
    notifier: Option<Rc<dyn Fn(String)>>,
}

thread_local! {
    static INNER: RefCell<Inner> = RefCell::new(Inner::default());
}

/// Where toasts go (set once by the application window).
pub fn set_notifier(f: Rc<dyn Fn(String)>) {
    INNER.with(|i| i.borrow_mut().notifier = Some(f));
}

pub fn status() -> Status {
    INNER.with(|i| i.borrow().status.clone().unwrap_or(Status::Idle))
}

pub fn running() -> bool {
    matches!(status(), Status::Running { .. })
}

/// Asks the running download to stop (the partial files stay for the next try).
pub fn cancel() {
    INNER.with(|i| {
        if let Some(a) = i.borrow().active.as_ref() {
            a.cancel.cancel();
        }
    });
}

/// What one progress report turns into: the fraction for the job, its detail line, and a log line
/// when a new file begins.
#[derive(Debug, Default)]
pub struct Tracker {
    speed: Throughput,
    file: String,
    origin: usize,
}

impl Tracker {
    pub fn on_progress(
        &mut self,
        ms: u64,
        file: &str,
        done: u64,
        total: u64,
        origin: usize,
    ) -> (f64, String, Option<String>) {
        self.speed.push(ms, done);
        let mut log = (file != self.file).then(|| {
            self.file = file.to_string();
            self.origin = 0;
            format!("Downloading {file}")
        });
        if origin != self.origin {
            // the origin before was slow, unreachable or not what it should be
            self.origin = origin;
            log = Some(format!(
                "Switching to the next origin for {file} (the first was slow or did not work)"
            ));
        }
        let fraction = if total == 0 {
            0.0
        } else {
            (done as f64 / total as f64).clamp(0.0, 1.0)
        };
        (fraction, progress_line(done, total, &self.speed), log)
    }
}

/// Starts downloading `packs` as a job called `title`, or joins the download that is running.
/// Returns the number of this download (see [`Status::Finished`]).
pub fn start(title: &str, packs: Vec<Pack>) -> u64 {
    if let Some(seq) = INNER.with(|i| {
        let i = i.borrow();
        i.active.as_ref().map(|a| a.seq)
    }) {
        return seq;
    }
    let (job, token) = jobs::start(jobs::Kind::Voice, title, true);
    jobs::log(job, &format!("Starting: {title}"));
    let (tx, rx) = channel();
    let cancel = Cancel::new();
    let c = cancel.clone();
    std::thread::spawn(move || {
        let tx2 = tx.clone();
        let r = tts::download_packs(&packs, &c, &mut |file, done, total, origin| {
            let _ = tx2.send(Msg::Progress {
                file: file.to_string(),
                done,
                total,
                origin,
            });
        });
        let _ = tx.send(Msg::Done(r));
    });
    let seq = INNER.with(|i| {
        let mut i = i.borrow_mut();
        i.seq += 1;
        let seq = i.seq;
        i.status = Some(Status::Running {
            done: 0,
            total: 0,
            line: tr("Starting…"),
        });
        i.active = Some(Active {
            rx,
            cancel,
            job,
            token,
            seq,
            tracker: Tracker::default(),
            started: Instant::now(),
        });
        seq
    });
    glib::timeout_add_local(Duration::from_millis(150), poll);
    seq
}

fn poll() -> glib::ControlFlow {
    let mut finished: Option<(u64, u64, Result<(), Failure>)> = None;
    INNER.with(|i| {
        let mut i = i.borrow_mut();
        let Some(a) = i.active.as_mut() else {
            return;
        };
        if a.token.is_cancelled() {
            a.cancel.cancel();
        }
        let mut last = None;
        while let Ok(m) = a.rx.try_recv() {
            match m {
                Msg::Progress {
                    file,
                    done,
                    total,
                    origin,
                } => {
                    let ms = a.started.elapsed().as_millis() as u64;
                    let (fraction, line, log) =
                        a.tracker.on_progress(ms, &file, done, total, origin);
                    if let Some(l) = log {
                        jobs::log(a.job, &l);
                    }
                    last = Some((fraction, line, done, total));
                }
                Msg::Done(r) => finished = Some((a.job, a.seq, r)),
            }
        }
        if let Some((fraction, line, done, total)) = last {
            jobs::progress(a.job, Some(fraction), &line);
            i.status = Some(Status::Running { done, total, line });
        }
    });
    let Some((job, seq, result)) = finished else {
        return glib::ControlFlow::Continue;
    };
    let notifier = INNER.with(|i| {
        let mut i = i.borrow_mut();
        i.active = None;
        i.status = Some(Status::Finished {
            seq,
            result: result.clone(),
        });
        i.notifier.clone()
    });
    let toast = match &result {
        Ok(()) => {
            jobs::log(job, "Done");
            jobs::finish(job, Ok(()));
            Some(tr("Voice ready"))
        }
        Err(Failure::Cancelled) => {
            jobs::log(job, "Cancelled; the part is kept for the next try");
            jobs::cancel(job);
            jobs::finish(job, Err("cancelled".into()));
            None
        }
        Err(f) => {
            let m = bookx_tts::setup::failure_message(f);
            jobs::log(job, &m);
            jobs::finish(job, Err(m.clone()));
            Some(tr(&m))
        }
    };
    if let (Some(t), Some(n)) = (toast, notifier) {
        n(t);
    }
    glib::ControlFlow::Break
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_becomes_a_fraction_a_line_and_one_log_line_per_file() {
        let mut t = Tracker::default();
        let (f, line, log) = t.on_progress(0, "model.onnx", 0, 100_000_000, 0);
        assert_eq!(f, 0.0);
        assert_eq!(line, "0 MB of 100 MB");
        assert_eq!(log.as_deref(), Some("Downloading model.onnx"));
        let (f, line, log) = t.on_progress(1000, "model.onnx", 5_000_000, 100_000_000, 0);
        assert!((f - 0.05).abs() < 1e-9);
        assert_eq!(line, "5 MB of 100 MB · 5.0 MB/s · 19 s left");
        assert!(log.is_none(), "the same file is logged once");
        let (_, _, log) = t.on_progress(2000, "voices.bin", 6_000_000, 100_000_000, 0);
        assert_eq!(log.as_deref(), Some("Downloading voices.bin"));
        // a total of 0 (unknown) and too-large values stay sane
        // a switch of origin is logged once
        let (_, _, log) = t.on_progress(2500, "voices.bin", 100, 100_000_000, 1);
        assert!(log.unwrap().contains("Switching to the next origin"));
        assert!(t
            .on_progress(2600, "voices.bin", 200, 100_000_000, 1)
            .2
            .is_none());
        assert_eq!(t.on_progress(3000, "x", 5, 0, 0).0, 0.0);
        assert_eq!(t.on_progress(4000, "x", 500, 100, 0).0, 1.0);
    }
}
