//! Background jobs the user can see (L27, C10), without GTK: imports, conversions, sends, metadata
//! fetches, backups and news downloads publish what they are doing here and the Jobs panel in the
//! library header shows it.
//!
//! Everything is called on the GTK main thread (workers report to it through the channels they
//! already have). A worker that can be stopped holds a [`CancelToken`] and checks it between steps.

use crate::i18n::tr;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// How many finished jobs the panel keeps.
const KEEP_FINISHED: usize = 30;
/// How many log lines a job keeps (the newest).
const KEEP_LOG: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Import,
    Convert,
    Send,
    Fetch,
    Backup,
    News,
    Voice,
    Dictionary,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Import => "Import",
            Kind::Convert => "Conversion",
            Kind::Send => "Send",
            Kind::Fetch => "Metadata",
            Kind::Backup => "Backup",
            Kind::News => "News",
            Kind::Voice => "Voice download",
            Kind::Dictionary => "Dictionary download",
        }
    }

    /// What the job is doing, for its status line ("Downloading · 130 of 132 MB").
    pub fn verb(self) -> String {
        match self {
            Kind::Import => tr("Importing"),
            Kind::Convert => tr("Converting"),
            Kind::Send => tr("Sending"),
            Kind::Fetch => tr("Looking up metadata"),
            Kind::Backup => tr("Backing up"),
            Kind::News => tr("Downloading news"),
            Kind::Voice | Kind::Dictionary => tr("Downloading"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Kind::Import => "document-open-symbolic",
            Kind::Convert => "emblem-synchronizing-symbolic",
            Kind::Send => "send-to-symbolic",
            Kind::Fetch => "system-search-symbolic",
            Kind::Backup => "document-save-symbolic",
            Kind::News => "application-rss+xml-symbolic",
            Kind::Voice => "audio-speakers-symbolic",
            Kind::Dictionary => "accessories-dictionary-symbolic",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Running,
    Done,
    Failed(String),
    Cancelled,
}

/// Shared with the worker: set when the user presses Cancel.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: u64,
    pub kind: Kind,
    pub title: String,
    pub state: State,
    /// 0..=1 when the job knows how far it is.
    pub progress: Option<f64>,
    /// The step it is on ("3 of 12", "Fetching…").
    pub detail: String,
    pub log: Vec<String>,
    pub started_ms: i64,
    pub finished_ms: Option<i64>,
    pub cancellable: bool,
    pub token: CancelToken,
}

impl Job {
    pub fn running(&self) -> bool {
        self.state == State::Running
    }

    /// "Running", "Done", "Failed: reason", "Cancelled".
    pub fn state_text(&self) -> String {
        match &self.state {
            State::Running if self.cancel_asked() => "Cancelling…".into(),
            State::Running => "Running".into(),
            State::Done => "Done".into(),
            State::Failed(e) => format!("Failed: {e}"),
            State::Cancelled => "Cancelled".into(),
        }
    }

    pub fn cancel_asked(&self) -> bool {
        self.token.is_cancelled()
    }

    pub fn elapsed_ms(&self, now_ms: i64) -> i64 {
        (self.finished_ms.unwrap_or(now_ms) - self.started_ms).max(0)
    }
}

/// The two dim lines under a job's title.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StatusLines {
    /// What it is doing and how far: "Downloading · 130 of 132 MB · 1.4 MB/s".
    pub primary: String,
    /// While running the time left ("about 2 s left", when the job knows); after the end how long
    /// it took. Never both.
    pub secondary: String,
}

/// The longest reason a failed job shows in its row (the log has all of it).
const REASON_MAX: usize = 140;

/// "130 MB of 132 MB" becomes "130 of 132 MB" (the same unit twice is said once).
fn tidy_sizes(seg: &str) -> String {
    let parts: Vec<&str> = seg.split_whitespace().collect();
    if let [a, ua, "of", b, ub] = parts.as_slice() {
        if ua == ub {
            return format!("{a} of {b} {ua}");
        }
    }
    seg.to_string()
}

/// Splits what a job says about its step into the parts to show and the time left (`"2 s left"`),
/// which goes on the second line.
fn split_detail(detail: &str) -> (Vec<String>, Option<String>) {
    let mut parts = Vec::new();
    let mut eta = None;
    for seg in detail.split(" · ").map(str::trim).filter(|s| !s.is_empty()) {
        if seg.ends_with(" left")
            && seg
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit() || seg.starts_with("about"))
        {
            eta = Some(seg.to_string());
        } else {
            parts.push(tidy_sizes(seg));
        }
    }
    (parts, eta)
}

fn cut(text: &str, max: usize) -> String {
    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        return one;
    }
    let mut t: String = one.chars().take(max).collect();
    t.push('…');
    t
}

/// The status lines of a job at `now_ms`: one dim line for what is happening, one for the time left
/// (running) or how long it took (finished); not both at once.
pub fn status_lines(job: &Job, now_ms: i64) -> StatusLines {
    let took = duration_text(job.elapsed_ms(now_ms));
    match &job.state {
        State::Running if job.cancel_asked() => StatusLines {
            primary: tr("Cancelling…"),
            secondary: String::new(),
        },
        State::Running => {
            let (parts, eta) = split_detail(&job.detail);
            let mut primary = job.kind.verb();
            if !parts.is_empty() {
                primary = format!("{primary} · {}", parts.join(" · "));
            }
            let secondary = match eta {
                Some(e) if e.starts_with("about") => e,
                Some(e) => format!("about {e}"),
                None => String::new(),
            };
            StatusLines { primary, secondary }
        }
        State::Done => StatusLines {
            primary: tr("Done"),
            secondary: crate::i18n::trf("Took {}", &[&took]),
        },
        State::Failed(reason) => StatusLines {
            primary: format!("{}: {}", tr("Failed"), cut(reason, REASON_MAX)),
            secondary: crate::i18n::trf("After {}", &[&took]),
        },
        State::Cancelled => StatusLines {
            primary: tr("Cancelled"),
            secondary: crate::i18n::trf("After {}", &[&took]),
        },
    }
}

thread_local! {
    static RETRY: RefCell<Vec<(u64, Rc<dyn Fn()>)>> = const { RefCell::new(Vec::new()) };
}

/// Lets a job be run again after it failed (the panel then shows a Retry button).
pub fn set_retry(id: u64, f: Rc<dyn Fn()>) {
    RETRY.with(|r| {
        let mut r = r.borrow_mut();
        r.retain(|(i, _)| *i != id);
        r.push((id, f));
    });
}

pub fn can_retry(id: u64) -> bool {
    RETRY.with(|r| r.borrow().iter().any(|(i, _)| *i == id))
}

/// Runs the retry of a job.
pub fn retry(id: u64) -> bool {
    let f = RETRY.with(|r| {
        r.borrow()
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, f)| f.clone())
    });
    match f {
        Some(f) => {
            f();
            true
        }
        None => false,
    }
}

/// "850 ms", "12 s", "3 min 05 s".
pub fn duration_text(ms: i64) -> String {
    let s = ms / 1000;
    if ms < 1000 {
        format!("{ms} ms")
    } else if s < 60 {
        format!("{s} s")
    } else {
        format!("{} min {:02} s", s / 60, s % 60)
    }
}

#[derive(Debug, Default)]
pub struct Hub {
    jobs: Vec<Job>,
    next: u64,
}

impl Hub {
    pub fn jobs(&self) -> &[Job] {
        &self.jobs
    }

    #[cfg(test)]
    pub fn get(&self, id: u64) -> Option<&Job> {
        self.jobs.iter().find(|j| j.id == id)
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut Job> {
        self.jobs.iter_mut().find(|j| j.id == id)
    }

    pub fn start(
        &mut self,
        kind: Kind,
        title: &str,
        cancellable: bool,
        now_ms: i64,
    ) -> (u64, CancelToken) {
        self.next += 1;
        let token = CancelToken::default();
        self.jobs.push(Job {
            id: self.next,
            kind,
            title: title.to_string(),
            state: State::Running,
            progress: None,
            detail: String::new(),
            log: vec![],
            started_ms: now_ms,
            finished_ms: None,
            cancellable,
            token: token.clone(),
        });
        (self.next, token)
    }

    pub fn progress(&mut self, id: u64, fraction: Option<f64>, detail: &str) {
        if let Some(j) = self.get_mut(id).filter(|j| j.running()) {
            j.progress = fraction.map(|f| f.clamp(0.0, 1.0));
            if !detail.is_empty() {
                j.detail = detail.to_string();
            }
        }
    }

    pub fn log(&mut self, id: u64, line: &str) {
        if let Some(j) = self.get_mut(id) {
            j.log.push(line.to_string());
            if j.log.len() > KEEP_LOG {
                let drop = j.log.len() - KEEP_LOG;
                j.log.drain(..drop);
            }
        }
    }

    /// Ends a job. A job whose token was cancelled ends as Cancelled, whatever `result` says.
    pub fn finish(&mut self, id: u64, result: Result<(), String>, now_ms: i64) {
        if let Some(j) = self.get_mut(id).filter(|j| j.running()) {
            j.state = if j.token.is_cancelled() {
                State::Cancelled
            } else {
                match result {
                    Ok(()) => State::Done,
                    Err(e) => State::Failed(e),
                }
            };
            if j.state == State::Done {
                j.progress = Some(1.0);
            }
            j.finished_ms = Some(now_ms);
            if let State::Failed(e) = &j.state {
                let line = format!("Failed: {e}");
                j.log.push(line);
            }
        }
        self.trim();
    }

    /// Asks a running job that can be stopped to stop. False when it cannot be.
    pub fn cancel(&mut self, id: u64) -> bool {
        match self.get_mut(id) {
            Some(j) if j.running() && j.cancellable => {
                j.token.cancel();
                j.log.push(tr("Cancel requested"));
                true
            }
            _ => false,
        }
    }

    pub fn running_count(&self) -> usize {
        self.jobs.iter().filter(|j| j.running()).count()
    }

    pub fn failed_count(&self) -> usize {
        self.jobs
            .iter()
            .filter(|j| matches!(j.state, State::Failed(_)))
            .count()
    }

    pub fn clear_finished(&mut self) {
        self.jobs.retain(|j| j.running());
    }

    fn trim(&mut self) {
        let finished = self.jobs.iter().filter(|j| !j.running()).count();
        if finished > KEEP_FINISHED {
            let mut drop = finished - KEEP_FINISHED;
            self.jobs.retain(|j| {
                if !j.running() && drop > 0 {
                    drop -= 1;
                    false
                } else {
                    true
                }
            });
        }
    }

    /// Text for the header button's tooltip.
    pub fn summary(&self) -> String {
        let running = self.running_count();
        let failed = self.failed_count();
        match (running, failed, self.jobs.len()) {
            (0, 0, 0) => tr("No background jobs"),
            (0, 0, n) => format!("{n} finished"),
            (0, f, _) => format!("{f} failed"),
            (r, 0, _) => format!("{r} running"),
            (r, f, _) => format!("{r} running, {f} failed"),
        }
    }

    /// Overall progress of the running jobs that know theirs.
    pub fn overall(&self) -> Option<f64> {
        let known: Vec<f64> = self
            .jobs
            .iter()
            .filter(|j| j.running())
            .filter_map(|j| j.progress)
            .collect();
        if known.is_empty() {
            None
        } else {
            Some(known.iter().sum::<f64>() / known.len() as f64)
        }
    }
}

// ---- the hub the application uses (GTK main thread only) ----

type Listener = Rc<dyn Fn()>;

thread_local! {
    static HUB: RefCell<Hub> = RefCell::new(Hub::default());
    static LISTENERS: RefCell<Vec<Listener>> = const { RefCell::new(Vec::new()) };
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn changed() {
    let ls: Vec<Listener> = LISTENERS.with(|l| l.borrow().clone());
    for l in ls {
        l();
    }
}

pub fn on_change(f: impl Fn() + 'static) {
    LISTENERS.with(|l| l.borrow_mut().push(Rc::new(f)));
}

pub fn with<R>(f: impl FnOnce(&Hub) -> R) -> R {
    HUB.with(|h| f(&h.borrow()))
}

pub fn start(kind: Kind, title: &str, cancellable: bool) -> (u64, CancelToken) {
    let r = HUB.with(|h| h.borrow_mut().start(kind, title, cancellable, now_ms()));
    changed();
    r
}

pub fn progress(id: u64, fraction: Option<f64>, detail: &str) {
    HUB.with(|h| h.borrow_mut().progress(id, fraction, detail));
    changed();
}

pub fn log(id: u64, line: &str) {
    HUB.with(|h| h.borrow_mut().log(id, line));
    changed();
}

pub fn finish(id: u64, result: Result<(), String>) {
    HUB.with(|h| h.borrow_mut().finish(id, result, now_ms()));
    changed();
}

pub fn cancel(id: u64) -> bool {
    let r = HUB.with(|h| h.borrow_mut().cancel(id));
    changed();
    r
}

pub fn clear_finished() {
    HUB.with(|h| h.borrow_mut().clear_finished());
    changed();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_runs_and_is_done() {
        let mut h = Hub::default();
        let (id, _) = h.start(Kind::Import, "Importing 3 books", true, 1_000);
        assert_eq!(h.running_count(), 1);
        h.progress(id, Some(1.0 / 3.0), "1 of 3");
        assert_eq!(h.get(id).unwrap().detail, "1 of 3");
        h.finish(id, Ok(()), 4_500);
        let j = h.get(id).unwrap();
        assert_eq!(j.state, State::Done);
        assert_eq!(j.progress, Some(1.0));
        assert_eq!(j.elapsed_ms(99_999), 3_500);
        assert_eq!(h.running_count(), 0);
    }

    #[test]
    fn a_failed_job_keeps_the_reason_in_its_log() {
        let mut h = Hub::default();
        let (id, _) = h.start(Kind::Backup, "Backup to NAS", false, 0);
        h.log(id, "connecting");
        h.finish(id, Err("no route to host".into()), 10);
        let j = h.get(id).unwrap();
        assert_eq!(j.state_text(), "Failed: no route to host");
        assert_eq!(j.log.last().unwrap(), "Failed: no route to host");
        assert_eq!(h.failed_count(), 1);
        assert_eq!(h.summary(), "1 failed");
    }

    #[test]
    fn only_jobs_that_support_it_can_be_cancelled() {
        let mut h = Hub::default();
        let (a, ta) = h.start(Kind::Import, "a", true, 0);
        let (b, tb) = h.start(Kind::Send, "b", false, 0);
        assert!(h.cancel(a));
        assert!(ta.is_cancelled());
        assert_eq!(h.get(a).unwrap().state_text(), "Cancelling…");
        assert!(!h.cancel(b));
        assert!(!tb.is_cancelled());
        // the worker notices and ends: the job is Cancelled even if it reports success
        h.finish(a, Ok(()), 5);
        assert_eq!(h.get(a).unwrap().state, State::Cancelled);
        // a finished job cannot be cancelled again
        assert!(!h.cancel(a));
    }

    #[test]
    fn a_finished_job_does_not_change_again() {
        let mut h = Hub::default();
        let (id, _) = h.start(Kind::News, "n", false, 0);
        h.finish(id, Ok(()), 1);
        h.finish(id, Err("late".into()), 2);
        h.progress(id, Some(0.2), "late");
        let j = h.get(id).unwrap();
        assert_eq!(j.state, State::Done);
        assert_eq!(j.finished_ms, Some(1));
        assert_eq!(j.progress, Some(1.0));
    }

    #[test]
    fn progress_is_clamped_and_overall_averages_the_known_ones() {
        let mut h = Hub::default();
        let (a, _) = h.start(Kind::Import, "a", false, 0);
        let (b, _) = h.start(Kind::Convert, "b", false, 0);
        let (_c, _) = h.start(Kind::Fetch, "c", false, 0);
        h.progress(a, Some(2.0), "");
        h.progress(b, Some(0.5), "");
        assert_eq!(h.get(a).unwrap().progress, Some(1.0));
        assert_eq!(h.overall(), Some(0.75));
        assert_eq!(h.summary(), "3 running");
    }

    #[test]
    fn the_log_and_the_finished_list_are_bounded() {
        let mut h = Hub::default();
        let (id, _) = h.start(Kind::News, "n", false, 0);
        for i in 0..(KEEP_LOG + 20) {
            h.log(id, &format!("line {i}"));
        }
        assert_eq!(h.get(id).unwrap().log.len(), KEEP_LOG);
        assert_eq!(
            h.get(id).unwrap().log.last().unwrap(),
            &format!("line {}", KEEP_LOG + 19)
        );
        for i in 0..(KEEP_FINISHED + 10) {
            let (j, _) = h.start(Kind::Send, &format!("job {i}"), false, 0);
            h.finish(j, Ok(()), 1);
        }
        let finished = h.jobs().iter().filter(|j| !j.running()).count();
        assert_eq!(finished, KEEP_FINISHED);
        // the oldest finished ones went, the running job stayed
        assert!(h.get(id).is_some());
        assert_eq!(h.jobs().iter().filter(|j| j.running()).count(), 1);
    }

    #[test]
    fn clearing_removes_only_finished_jobs() {
        let mut h = Hub::default();
        let (a, _) = h.start(Kind::Import, "a", false, 0);
        let (b, _) = h.start(Kind::Import, "b", false, 0);
        h.finish(a, Ok(()), 1);
        h.clear_finished();
        assert!(h.get(a).is_none());
        assert!(h.get(b).is_some());
    }

    #[test]
    fn texts() {
        assert_eq!(duration_text(850), "850 ms");
        assert_eq!(duration_text(12_400), "12 s");
        assert_eq!(duration_text(185_000), "3 min 05 s");
        assert_eq!(Hub::default().summary(), "No background jobs");
        let mut h = Hub::default();
        let (a, _) = h.start(Kind::Import, "a", false, 0);
        h.finish(a, Ok(()), 1);
        assert_eq!(h.summary(), "1 finished");
        let (_, _) = h.start(Kind::Import, "b", false, 0);
        assert_eq!(h.summary(), "1 running");
    }

    #[test]
    fn the_global_hub_notifies() {
        use std::cell::Cell;
        let n = Rc::new(Cell::new(0));
        let n2 = n.clone();
        on_change(move || n2.set(n2.get() + 1));
        let (id, _) = start(Kind::Fetch, "x", true);
        progress(id, Some(0.5), "half");
        log(id, "hello");
        finish(id, Ok(()));
        assert_eq!(n.get(), 4);
        assert_eq!(
            with(|h| h.get(id).map(|j| j.state.clone())),
            Some(State::Done)
        );
    }

    fn job(kind: Kind, state: State, detail: &str, title: &str) -> Job {
        Job {
            id: 1,
            kind,
            title: title.into(),
            state,
            progress: None,
            detail: detail.into(),
            log: vec![],
            started_ms: 0,
            finished_ms: None,
            cancellable: true,
            token: CancelToken::default(),
        }
    }

    #[test]
    fn a_running_download_says_what_it_does_and_the_time_left_on_the_second_line() {
        let j = job(
            Kind::Voice,
            State::Running,
            "130 MB of 132 MB · 1.4 MB/s · 2 s left",
            "Voice",
        );
        let l = status_lines(&j, 87_000);
        assert_eq!(l.primary, "Downloading · 130 of 132 MB · 1.4 MB/s");
        assert_eq!(l.secondary, "about 2 s left");
        // no elapsed time while it runs (it contradicted the time left)
        assert!(!l.primary.contains("min") && !l.secondary.contains("1 min"));
    }

    #[test]
    fn a_job_without_numbers_or_with_unknown_totals_just_says_what_it_does() {
        let j = job(Kind::Import, State::Running, "", "Importing 3 files");
        let l = status_lines(&j, 5000);
        assert_eq!(
            (l.primary.as_str(), l.secondary.as_str()),
            ("Importing", "")
        );
        let j = job(Kind::Fetch, State::Running, "3 of 12", "x");
        assert_eq!(status_lines(&j, 0).primary, "Looking up metadata · 3 of 12");
        // zero speed, no total: nothing is invented
        let j = job(Kind::Voice, State::Running, "0 MB · 0 B/s", "x");
        assert_eq!(status_lines(&j, 0).primary, "Downloading · 0 MB · 0 B/s");
        // different units are kept
        let j = job(Kind::Voice, State::Running, "900 KB of 2 MB", "x");
        assert_eq!(status_lines(&j, 0).primary, "Downloading · 900 KB of 2 MB");
        // an estimate that already says "about"
        let j = job(
            Kind::Voice,
            State::Running,
            "5 MB of 9 MB · about 3 min left",
            "x",
        );
        assert_eq!(status_lines(&j, 0).secondary, "about 3 min left");
    }

    #[test]
    fn a_finished_job_shows_the_result_and_how_long_it_took() {
        let mut j = job(Kind::Backup, State::Done, "", "Backup");
        j.finished_ms = Some(87_000);
        let l = status_lines(&j, 999_999);
        assert_eq!(l.primary, "Done");
        assert_eq!(l.secondary, "Took 1 min 27 s");
        let mut j = job(Kind::Send, State::Failed("disk full".into()), "", "Send");
        j.finished_ms = Some(12_000);
        let l = status_lines(&j, 999_999);
        assert_eq!(l.primary, "Failed: disk full");
        assert_eq!(l.secondary, "After 12 s");
        let mut j = job(Kind::Send, State::Cancelled, "", "Send");
        j.finished_ms = Some(500);
        assert_eq!(status_lines(&j, 9).secondary, "After 500 ms");
    }

    #[test]
    fn a_long_reason_is_cut_and_a_cancel_in_progress_is_clear() {
        let reason = "x ".repeat(400);
        let j = job(Kind::Send, State::Failed(reason), "", "Send");
        let l = status_lines(&j, 0);
        assert!(
            l.primary.chars().count() <= "Failed: ".len() + REASON_MAX + 1,
            "{}",
            l.primary.len()
        );
        assert!(l.primary.ends_with('…'));
        let j = job(
            Kind::Convert,
            State::Running,
            "2 of 5 · 4 s left",
            "Convert",
        );
        j.token.cancel();
        let l = status_lines(&j, 0);
        assert_eq!(
            (l.primary.as_str(), l.secondary.as_str()),
            ("Cancelling…", "")
        );
    }

    #[test]
    fn long_titles_never_reach_the_status_lines() {
        let j = job(
            Kind::Import,
            State::Running,
            "1 of 2",
            &"very long title ".repeat(30),
        );
        assert_eq!(status_lines(&j, 0).primary, "Importing · 1 of 2");
    }

    #[test]
    fn a_retry_can_be_registered_and_run() {
        let hits = Rc::new(std::cell::Cell::new(0));
        let h = hits.clone();
        assert!(!can_retry(777));
        set_retry(777, Rc::new(move || h.set(h.get() + 1)));
        assert!(can_retry(777) && retry(777));
        assert_eq!(hits.get(), 1);
        assert!(!retry(778));
    }
}
