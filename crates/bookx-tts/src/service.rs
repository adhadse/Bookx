//! The speech service of the application: what outlives a book.
//!
//! ONNX Runtime allows one environment per process, and loading a voice model takes seconds and
//! hundreds of MB. So the loaded models live here, in the application, and are shared by whichever
//! book is open; a reader only holds a light [`SessionGuard`] on the service:
//!
//! * [`Service::shared_model`] builds a model once and hands out the same `Arc` afterwards; a model
//!   nobody has used for [`MODEL_TTL`] is freed by [`Service::sweep`].
//! * [`Service::acquire`] is the hand-off between books: when another session is still active (a
//!   worker thread stopping, audio draining) it is asked to stop and waited for briefly; after the
//!   timeout the new session takes over anyway. It never fails.
//!
//! (Before this, every reader built its own ONNX environment and the second book of a run failed
//! with "only one environment is allowed per process".)

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};

use crate::session::Voice;
use crate::synth::{Pcm, SynthRequest, Synthesizer};

/// How long a loaded model is kept after its last use.
pub const MODEL_TTL: Duration = Duration::from_secs(10 * 60);
/// How long a new session waits for the previous one to wind down.
pub const HANDOFF_WAIT: Duration = Duration::from_secs(3);
/// How long one synthesis waits for the model while another session still uses it.
const MODEL_WAIT: Duration = Duration::from_secs(20);

type Model = Arc<Mutex<Box<dyn Synthesizer>>>;
type Stop = Arc<dyn Fn() + Send + Sync>;

struct Cached {
    model: Model,
    last_used: Instant,
}

struct Active {
    id: u64,
    stop: Option<Stop>,
}

#[derive(Default)]
struct Inner {
    models: HashMap<String, Cached>,
    active: Option<Active>,
    next_id: u64,
}

pub struct Service {
    inner: Mutex<Inner>,
    released: Condvar,
}

/// The one service of the process.
pub fn global() -> &'static Service {
    static S: OnceLock<Service> = OnceLock::new();
    S.get_or_init(Service::new)
}

impl Service {
    pub fn new() -> Service {
        Service {
            inner: Mutex::new(Inner::default()),
            released: Condvar::new(),
        }
    }

    /// Starts a session. When another one is active it is asked to stop and waited for up to `wait`;
    /// then this one takes over (the old guard becomes a no-op). Never fails.
    pub fn acquire(&'static self, wait: Duration) -> SessionGuard {
        let deadline = Instant::now() + wait;
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let stop = inner.active.as_ref().and_then(|a| a.stop.clone());
        if let Some(stop) = stop {
            // outside the lock: the hook talks to threads that may want the service
            drop(inner);
            stop();
            inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        }
        while inner.active.is_some() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            inner = self
                .released
                .wait_timeout(inner, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        inner.next_id += 1;
        let id = inner.next_id;
        inner.active = Some(Active { id, stop: None });
        SessionGuard { service: self, id }
    }

    /// True while a session is active.
    pub fn busy(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active
            .is_some()
    }

    fn release(&self, id: u64) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.active.as_ref().is_some_and(|a| a.id == id) {
            inner.active = None;
        }
        self.released.notify_all();
    }

    /// The model for `key`, built by `make` the first time. Later calls (also from other books)
    /// get the same model.
    pub fn shared_model(
        &self,
        key: &str,
        make: impl FnOnce() -> Result<Box<dyn Synthesizer>>,
    ) -> Result<Model> {
        if let Some(m) = self.touch(key) {
            return Ok(m);
        }
        // loading takes a while: not under the lock
        let built: Model = Arc::new(Mutex::new(make()?));
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let entry = inner.models.entry(key.to_string()).or_insert(Cached {
            model: built,
            last_used: Instant::now(),
        });
        entry.last_used = Instant::now();
        Ok(entry.model.clone())
    }

    fn touch(&self, key: &str) -> Option<Model> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.models.get_mut(key).map(|c| {
            c.last_used = Instant::now();
            c.model.clone()
        })
    }

    /// Frees the models nobody has used for [`MODEL_TTL`] (and that no session holds).
    pub fn sweep(&self, now: Instant) -> usize {
        self.sweep_older_than(now, MODEL_TTL)
    }

    pub fn sweep_older_than(&self, now: Instant, ttl: Duration) -> usize {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let before = inner.models.len();
        inner.models.retain(|_, c| {
            Arc::strong_count(&c.model) > 1 || now.saturating_duration_since(c.last_used) < ttl
        });
        before - inner.models.len()
    }

    /// Models in memory (tests, settings).
    pub fn cached_models(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .models
            .len()
    }
}

impl Default for Service {
    fn default() -> Self {
        Service::new()
    }
}

/// A reader's hold on the service. Dropping it releases the session.
pub struct SessionGuard {
    service: &'static Service,
    id: u64,
}

impl SessionGuard {
    /// What to call to silence this session when another one wants the service.
    pub fn set_stop(&self, stop: Stop) {
        let mut inner = self.service.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(a) = inner.active.as_mut().filter(|a| a.id == self.id) {
            a.stop = Some(stop);
        }
    }

    /// Is this still the active session (nobody took over)?
    pub fn is_current(&self) -> bool {
        self.service
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active
            .as_ref()
            .is_some_and(|a| a.id == self.id)
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.service.release(self.id);
    }
}

type Factory = Box<dyn FnOnce() -> Result<Box<dyn Synthesizer>> + Send>;

/// A synthesizer that is a handle on a model of the [`Service`]: the model is built the first time
/// any session speaks (on the worker thread), kept for later sessions and other books, and locked
/// for the time of one sentence.
pub struct SharedSynth {
    service: &'static Service,
    key: String,
    id: String,
    voices: Vec<Voice>,
    factory: Option<Factory>,
    model: Option<Model>,
}

impl SharedSynth {
    pub fn new(
        service: &'static Service,
        key: &str,
        id: &str,
        voices: Vec<Voice>,
        factory: Factory,
    ) -> SharedSynth {
        SharedSynth {
            service,
            key: key.to_string(),
            id: id.to_string(),
            voices,
            factory: Some(factory),
            model: None,
        }
    }

    fn model(&mut self) -> Result<Model> {
        if let Some(m) = &self.model {
            return Ok(m.clone());
        }
        let m = match self.factory.take() {
            Some(f) => self.service.shared_model(&self.key, f)?,
            // the factory ran (or failed) before: ask the service again, it may have the model
            None => self
                .service
                .touch(&self.key)
                .ok_or_else(|| anyhow!("The voice could not be started"))?,
        };
        self.model = Some(m.clone());
        Ok(m)
    }
}

impl Synthesizer for SharedSynth {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn voices(&mut self) -> Result<Vec<Voice>> {
        Ok(self.voices.clone())
    }

    fn synthesize(&mut self, req: &SynthRequest) -> Result<Pcm> {
        let model = self.model()?;
        // another session may still be finishing its last sentence with this model
        let deadline = Instant::now() + MODEL_WAIT;
        let mut guard = loop {
            match model.try_lock() {
                Ok(g) => break g,
                Err(std::sync::TryLockError::Poisoned(p)) => break p.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => {
                    if req.cancel.is_cancelled() {
                        return Err(anyhow!("cancelled"));
                    }
                    if Instant::now() >= deadline {
                        return Err(anyhow!("The voice is busy with another book. Try again"));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        };
        let r = guard.synthesize(req);
        drop(guard);
        let _ = self.service.touch(&self.key);
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::Cancel;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// Each test has its own service (they run in parallel); leaked for the `'static` the guards need.
    fn svc() -> &'static Service {
        Box::leak(Box::new(Service::new()))
    }

    struct Tone(Arc<AtomicUsize>, Duration);
    impl Synthesizer for Tone {
        fn id(&self) -> String {
            "tone".into()
        }
        fn voices(&mut self) -> Result<Vec<Voice>> {
            Ok(vec![])
        }
        fn synthesize(&mut self, _: &SynthRequest) -> Result<Pcm> {
            self.0.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(self.1);
            Ok(Pcm {
                rate: 8000,
                samples: vec![0.1; 16],
            })
        }
    }

    fn req(c: &Cancel) -> SynthRequest<'_> {
        SynthRequest {
            text: "x",
            voice: "",
            language: "",
            speed: 1.0,
            cancel: c,
        }
    }

    fn handle(
        s: &'static Service,
        key: &str,
        built: &Arc<AtomicUsize>,
        calls: &Arc<AtomicUsize>,
    ) -> SharedSynth {
        let (b, c) = (built.clone(), calls.clone());
        SharedSynth::new(
            s,
            key,
            "tone",
            vec![],
            Box::new(move || {
                b.fetch_add(1, Ordering::SeqCst);
                Ok(Box::new(Tone(c, Duration::ZERO)))
            }),
        )
    }

    #[test]
    fn a_second_book_reuses_the_loaded_model_instead_of_starting_another_engine() {
        // the failure this service exists for: every reader used to start its own engine and the
        // second one of the run failed with "only one environment is allowed per process"
        let s = svc();
        let (built, calls) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let c = Cancel::new();
        let mut book_a = handle(s, "kokoro", &built, &calls);
        book_a.synthesize(&req(&c)).unwrap();
        drop(book_a); // back to the library
        let mut book_b = handle(s, "kokoro", &built, &calls);
        book_b.synthesize(&req(&c)).unwrap();
        let mut book_a_again = handle(s, "kokoro", &built, &calls);
        book_a_again.synthesize(&req(&c)).unwrap();
        assert_eq!(
            built.load(Ordering::SeqCst),
            1,
            "built once, for the whole run"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(s.cached_models(), 1);
    }

    #[test]
    fn a_failed_start_is_an_error_and_a_later_try_can_work() {
        let s = svc();
        let c = Cancel::new();
        let mut bad = SharedSynth::new(s, "k", "k", vec![], Box::new(|| Err(anyhow!("no model"))));
        assert!(bad.synthesize(&req(&c)).is_err());
        assert_eq!(s.cached_models(), 0, "a failure is not cached");
        let (built, calls) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let mut good = handle(s, "k", &built, &calls);
        good.synthesize(&req(&c)).unwrap();
        assert_eq!(s.cached_models(), 1);
    }

    #[test]
    fn a_model_is_freed_after_its_time_but_not_while_a_session_holds_it() {
        let s = svc();
        let (built, calls) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let c = Cancel::new();
        let mut h = handle(s, "m", &built, &calls);
        h.synthesize(&req(&c)).unwrap();
        let later = Instant::now() + MODEL_TTL + Duration::from_secs(1);
        assert_eq!(s.sweep(later), 0, "the session still holds it");
        assert_eq!(s.sweep(Instant::now()), 0, "and it was used just now");
        drop(h);
        assert_eq!(
            s.sweep(Instant::now()),
            0,
            "unused for a short time: kept (switching books is fast)"
        );
        assert_eq!(s.sweep(later), 1, "unused for 10 minutes: freed");
        assert_eq!(s.cached_models(), 0);
        // the next use loads it again, cleanly
        let mut again = handle(s, "m", &built, &calls);
        again.synthesize(&req(&c)).unwrap();
        assert_eq!(built.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_new_session_stops_the_old_one_and_takes_over_when_it_is_gone() {
        let s = svc();
        let first = s.acquire(Duration::from_secs(1));
        let stopped = Arc::new(AtomicBool::new(false));
        let st = stopped.clone();
        first.set_stop(Arc::new(move || st.store(true, Ordering::SeqCst)));
        assert!(first.is_current() && s.busy());
        // the old reader winds down a moment after being asked
        let t = std::thread::spawn({
            let stopped = stopped.clone();
            move || {
                while !stopped.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                std::thread::sleep(Duration::from_millis(60));
                drop(first);
            }
        });
        let t0 = Instant::now();
        let second = s.acquire(Duration::from_secs(2));
        assert!(
            stopped.load(Ordering::SeqCst),
            "the old session was asked to stop"
        );
        assert!(t0.elapsed() >= Duration::from_millis(50), "and waited for");
        assert!(t0.elapsed() < Duration::from_secs(1));
        assert!(second.is_current());
        t.join().unwrap();
        assert!(
            second.is_current(),
            "the old guard's drop did not release the new session"
        );
        drop(second);
        assert!(!s.busy());
    }

    #[test]
    fn a_session_that_never_lets_go_is_taken_over_after_the_timeout_not_an_error() {
        let s = svc();
        let stuck = s.acquire(Duration::from_secs(1));
        let asked = Arc::new(AtomicBool::new(false));
        let a = asked.clone();
        stuck.set_stop(Arc::new(move || a.store(true, Ordering::SeqCst)));
        let t0 = Instant::now();
        let next = s.acquire(Duration::from_millis(150));
        let waited = t0.elapsed();
        assert!(asked.load(Ordering::SeqCst));
        assert!(
            waited >= Duration::from_millis(140) && waited < Duration::from_secs(1),
            "{waited:?}"
        );
        assert!(next.is_current() && !stuck.is_current());
        drop(stuck);
        assert!(
            next.is_current(),
            "late release of the old one changes nothing"
        );
        drop(next);
        assert!(!s.busy());
    }

    #[test]
    fn rapid_switching_never_fails_and_never_starts_a_second_engine() {
        // read aloud, leave, another book, read aloud, leave, back: ten times in a row
        let s = svc();
        let (built, calls) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let c = Cancel::new();
        for i in 0..10 {
            let guard = s.acquire(Duration::from_millis(200));
            let mut h = handle(s, "kokoro", &built, &calls);
            if i % 3 != 0 {
                // sometimes the user leaves before the first word was made
                h.synthesize(&req(&c)).unwrap();
            }
            drop(h);
            drop(guard);
        }
        assert_eq!(built.load(Ordering::SeqCst), 1);
        assert!(!s.busy());
    }

    #[test]
    fn two_sessions_using_the_model_at_once_take_turns() {
        let s = svc();
        let built = Arc::new(AtomicUsize::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        let mk = |ms| {
            let (b, c) = (built.clone(), calls.clone());
            SharedSynth::new(
                s,
                "slow",
                "slow",
                vec![],
                Box::new(move || {
                    b.fetch_add(1, Ordering::SeqCst);
                    Ok(Box::new(Tone(c, Duration::from_millis(ms))))
                }),
            )
        };
        let (mut a, mut b) = (mk(80), mk(80));
        let c = Cancel::new();
        let ta = std::thread::spawn(move || {
            let c = Cancel::new();
            a.synthesize(&req(&c)).map(|_| ())
        });
        std::thread::sleep(Duration::from_millis(15));
        let t0 = Instant::now();
        b.synthesize(&req(&c)).unwrap();
        assert!(
            t0.elapsed() >= Duration::from_millis(40),
            "it waited for the first sentence"
        );
        ta.join().unwrap().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(built.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn waiting_for_the_model_can_be_cancelled() {
        let s = svc();
        let built = Arc::new(AtomicUsize::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        let mut first = handle(s, "busy", &built, &calls);
        let c = Cancel::new();
        first.synthesize(&req(&c)).unwrap();
        let model = s.shared_model("busy", || unreachable!()).unwrap();
        let held = model.lock().unwrap(); // a session in the middle of a sentence
        let mut second = handle(s, "busy", &built, &calls);
        let c2 = Cancel::new();
        let c3 = c2.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(60));
            c3.cancel();
        });
        let e = second.synthesize(&req(&c2)).unwrap_err().to_string();
        assert_eq!(e, "cancelled");
        drop(held);
    }
}
