//! Gapless playback of a [`Synthesizer`]: a worker thread synthesizes the sentences the
//! session queued ahead while a player thread feeds the sink at the speed of playback.
//!
//! * the session decides how far to look ahead ([`Engine::lookahead`]);
//! * `Begin` is reported when a sentence is actually heard, `End` when it has been heard, so
//!   the highlight follows the voice;
//! * stopping bumps a generation number, cancels the synthesis in progress and kills the
//!   audio buffer: nothing of the old sentences is heard or reported afterwards.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};

use crate::cache::{SentenceCache, cache_key};
use crate::session::{Engine, Event, EventKind, Voice};
use crate::sink::AudioSink;
use crate::synth::{
    Cancel, Pcm, SynthRequest, Synthesizer, engine_max_speed, resample, split_speed,
};

#[derive(Debug, Clone)]
struct Settings {
    voice: String,
    language: String,
    speed: f64,
    pitch: f64,
}

enum Work {
    Say {
        id: u64,
        r#gen: u64,
        text: String,
        settings: Settings,
    },
    /// Synthesizes ahead of time; the audio is kept for a later `Say` of the same text.
    Prefetch {
        texts: Vec<String>,
        settings: Settings,
    },
    Voices(Sender<Result<Vec<Voice>>>),
    Quit,
}

enum Play {
    Audio {
        id: u64,
        r#gen: u64,
        pcm: Pcm,
    },
    /// The synthesis of the sentence at this place of the queue failed.
    Failed {
        r#gen: u64,
        message: String,
    },
    Pause,
    Resume,
    Stop(u64),
    Quit,
}

struct Shared {
    r#gen: AtomicU64,
    /// Playback gain as `f32` bits (1.0 = as synthesized).
    volume: AtomicU32,
    cancel: Mutex<Cancel>,
    error: Mutex<Option<String>>,
    scope: Mutex<String>,
}

pub struct PipelineEngine {
    name: String,
    work: Sender<Work>,
    play: Sender<Play>,
    events: Receiver<Event>,
    shared: Arc<Shared>,
    settings: Settings,
    next_id: u64,
    lookahead: usize,
    voices: Option<Vec<Voice>>,
    threads: Vec<JoinHandle<()>>,
    /// The hold on the speech service (see `service`); released when the engine is dropped.
    guard: Option<crate::service::SessionGuard>,
}

impl PipelineEngine {
    /// Makes this engine a session of the speech service: another book that wants the service
    /// silences this engine through the hook set here, and dropping the engine releases it.
    pub fn with_session(mut self, guard: crate::service::SessionGuard) -> PipelineEngine {
        let (play, shared) = (Mutex::new(self.play.clone()), self.shared.clone());
        guard.set_stop(Arc::new(move || {
            let r#gen = shared.r#gen.fetch_add(1, Ordering::SeqCst) + 1;
            shared.cancel.lock().unwrap().cancel();
            let _ = play.lock().unwrap().send(Play::Stop(r#gen));
        }));
        self.guard = Some(guard);
        self
    }

    pub fn new(
        name: &str,
        synth: Box<dyn Synthesizer>,
        sink: Box<dyn AudioSink>,
        cache: Option<SentenceCache>,
        lookahead: usize,
    ) -> PipelineEngine {
        let shared = Arc::new(Shared {
            r#gen: AtomicU64::new(1),
            volume: AtomicU32::new(1.0f32.to_bits()),
            cancel: Mutex::new(Cancel::new()),
            error: Mutex::new(None),
            scope: Mutex::new(String::new()),
        });
        let (work, work_rx) = mpsc::channel();
        let (play, play_rx) = mpsc::channel();
        let (ev_tx, events) = mpsc::channel();
        let w = {
            let (play, shared) = (play.clone(), shared.clone());
            std::thread::Builder::new()
                .name("bookx-tts-synth".into())
                .spawn(move || worker_loop(work_rx, play, synth, shared, cache))
                .expect("starting the synthesis thread")
        };
        let p = {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("bookx-tts-play".into())
                .spawn(move || Player::new(sink, ev_tx, shared).run(play_rx))
                .expect("starting the playback thread")
        };
        PipelineEngine {
            name: name.to_string(),
            work,
            play,
            events,
            shared,
            settings: Settings {
                voice: String::new(),
                language: String::new(),
                speed: 1.0,
                pitch: 1.0,
            },
            next_id: 0,
            lookahead: lookahead.max(1),
            voices: None,
            threads: vec![w, p],
            guard: None,
        }
    }
}

impl Drop for PipelineEngine {
    fn drop(&mut self) {
        self.shared.cancel.lock().unwrap().cancel();
        self.shared.r#gen.fetch_add(1, Ordering::SeqCst);
        let _ = self.work.send(Work::Quit);
        let _ = self.play.send(Play::Quit);
        // the threads end by themselves; not joined, a synthesis may still be running
        self.threads.clear();
    }
}

impl Engine for PipelineEngine {
    fn speak(&mut self, text: &str) -> Result<u64> {
        self.next_id += 1;
        self.work
            .send(Work::Say {
                id: self.next_id,
                r#gen: self.shared.r#gen.load(Ordering::SeqCst),
                text: text.to_string(),
                settings: self.settings.clone(),
            })
            .map_err(|_| anyhow!("the speech engine stopped"))?;
        Ok(self.next_id)
    }

    fn stop(&mut self) {
        let r#gen = self.shared.r#gen.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let mut c = self.shared.cancel.lock().unwrap();
            c.cancel();
            *c = Cancel::new();
        }
        *self.shared.error.lock().unwrap() = None;
        let _ = self.play.send(Play::Stop(r#gen));
        while self.events.try_recv().is_ok() {}
    }

    fn pause(&mut self) {
        let _ = self.play.send(Play::Pause);
    }

    fn resume(&mut self) {
        let _ = self.play.send(Play::Resume);
    }

    fn prefetch(&mut self, texts: &[String]) {
        let _ = self.work.send(Work::Prefetch {
            texts: texts.to_vec(),
            settings: self.settings.clone(),
        });
    }

    fn set_speed(&mut self, speed: f64) {
        self.settings.speed = speed;
    }

    fn set_pitch(&mut self, pitch: f64) {
        self.settings.pitch = pitch;
    }

    fn set_volume(&mut self, volume: f64) {
        let v = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            1.0
        };
        self.shared
            .volume
            .store((v as f32).to_bits(), Ordering::Relaxed);
    }

    fn set_voice(&mut self, name: &str) {
        self.settings.voice = name.to_string();
    }

    fn set_language(&mut self, language: &str) {
        self.settings.language = language.to_string();
    }

    fn set_book(&mut self, id: &str) {
        *self.shared.scope.lock().unwrap() = id.to_string();
    }

    fn voices(&mut self) -> Vec<Voice> {
        if let Some(v) = &self.voices {
            return v.clone();
        }
        let (tx, rx) = mpsc::channel();
        if self.work.send(Work::Voices(tx)).is_err() {
            return Vec::new();
        }
        match rx.recv_timeout(Duration::from_secs(8)) {
            Ok(Ok(v)) => {
                self.voices = Some(v.clone());
                v
            }
            Ok(Err(e)) => {
                *self.shared.error.lock().unwrap() = Some(format!("{e:#}"));
                Vec::new()
            }
            Err(_) => Vec::new(),
        }
    }

    fn take_error(&mut self) -> Option<String> {
        self.shared.error.lock().unwrap().take()
    }

    fn poll(&mut self) -> Vec<Event> {
        self.events.try_iter().collect()
    }

    fn name(&self) -> String {
        self.name.clone()
    }

    fn lookahead(&self) -> usize {
        self.lookahead
    }

    fn debug_state(&self) -> String {
        format!(
            "pipeline {} messages={} speed={:.2} pitch={:.2} voice={:?} language={:?}",
            self.name,
            self.next_id,
            self.settings.speed,
            self.settings.pitch,
            self.settings.voice,
            self.settings.language
        )
    }
}

fn worker_loop(
    rx: Receiver<Work>,
    play: Sender<Play>,
    mut synth: Box<dyn Synthesizer>,
    shared: Arc<Shared>,
    cache: Option<SentenceCache>,
) {
    // audio made ahead of time by `Prefetch`, taken by the `Say` of the same text
    let mut warm: Vec<(String, Pcm)> = Vec::new();
    while let Ok(job) = rx.recv() {
        match job {
            Work::Quit => break,
            Work::Prefetch { texts, settings } => {
                let (speed, _) = split_speed(
                    settings.speed / settings.pitch,
                    engine_max_speed(&synth.id()),
                );
                for text in texts {
                    let cancel = shared.cancel.lock().unwrap().clone();
                    if cancel.is_cancelled() {
                        break;
                    }
                    let key = cache_key(
                        &synth.id(),
                        &settings.voice,
                        &settings.language,
                        speed,
                        &text,
                    );
                    if warm.iter().any(|w| w.0 == key) {
                        continue;
                    }
                    let r = synth.synthesize(&SynthRequest {
                        text: &text,
                        voice: &settings.voice,
                        language: &settings.language,
                        speed,
                        cancel: &cancel,
                    });
                    if let (Ok(p), false) = (r, cancel.is_cancelled()) {
                        warm.push((key, p));
                        if warm.len() > 6 {
                            warm.remove(0);
                        }
                    }
                }
            }
            Work::Voices(reply) => {
                let _ = reply.send(synth.voices());
            }
            Work::Say {
                id,
                r#gen,
                text,
                settings,
            } => {
                if r#gen != shared.r#gen.load(Ordering::SeqCst) {
                    continue;
                }
                let cancel = shared.cancel.lock().unwrap().clone();
                // a higher pitch is made by playing faster: ask the model for a slower voice
                let (speed, stretch) = split_speed(
                    settings.speed / settings.pitch,
                    engine_max_speed(&synth.id()),
                );
                let scope = shared.scope.lock().unwrap().clone();
                let key = cache_key(
                    &synth.id(),
                    &settings.voice,
                    &settings.language,
                    speed,
                    &text,
                );
                let cached = warm
                    .iter()
                    .position(|w| w.0 == key)
                    .map(|i| warm.remove(i).1)
                    .or_else(|| cache.as_ref().and_then(|c| c.get(&scope, &key)));
                let result = match cached {
                    Some(p) => Ok(p),
                    None => {
                        let r = synth.synthesize(&SynthRequest {
                            text: &text,
                            voice: &settings.voice,
                            language: &settings.language,
                            speed,
                            cancel: &cancel,
                        });
                        if let (Ok(p), Some(c)) = (&r, &cache)
                            && !cancel.is_cancelled()
                        {
                            c.put(&scope, &key, p);
                        }
                        r
                    }
                };
                if r#gen != shared.r#gen.load(Ordering::SeqCst) {
                    continue;
                }
                match result {
                    Ok(p) => {
                        let _ = play.send(Play::Audio {
                            id,
                            r#gen,
                            pcm: p.time_stretch(stretch).with_pitch(settings.pitch),
                        });
                    }
                    Err(e) => {
                        let _ = play.send(Play::Failed {
                            r#gen,
                            message: format!("{e:#}"),
                        });
                    }
                }
            }
        }
    }
}

/// The part of a sentence played per step.
const CHUNK: Duration = Duration::from_millis(50);
/// `samples` scaled by `gain` (volume 0..=1); untouched at full volume.
fn apply_gain(samples: &[i16], gain: f32) -> std::borrow::Cow<'_, [i16]> {
    if gain >= 0.999 {
        return std::borrow::Cow::Borrowed(samples);
    }
    std::borrow::Cow::Owned(
        samples
            .iter()
            .map(|s| (*s as f32 * gain).round() as i16)
            .collect(),
    )
}

/// How far the sink may run ahead of the clock.
const LEAD: Duration = Duration::from_millis(250);

struct Playing {
    id: u64,
    samples: Vec<i16>,
    pos: usize,
}

struct Player {
    sink: Box<dyn AudioSink>,
    out: Sender<Event>,
    shared: Arc<Shared>,
    r#gen: u64,
    rate: Option<u32>,
    ready: VecDeque<Result<(u64, Pcm), String>>,
    cur: Option<Playing>,
    paused: Option<Instant>,
    /// When sample 0 of the stream is (was) heard.
    clock: Option<Instant>,
    written: Duration,
    due: VecDeque<(Instant, Event)>,
    /// The last samples handed to the sink (about what it still buffers), for a pause that
    /// silences the output at once and plays the unheard part again on resume.
    tail: VecDeque<i16>,
    /// What the pause took back from the sink; written again first on resume.
    replay: Vec<i16>,
    /// Stream position that had been heard when the pause drained the sink.
    drained_at: Option<Duration>,
}

impl Player {
    fn new(sink: Box<dyn AudioSink>, out: Sender<Event>, shared: Arc<Shared>) -> Player {
        Player {
            sink,
            out,
            shared,
            r#gen: 1,
            rate: None,
            ready: VecDeque::new(),
            cur: None,
            paused: None,
            clock: None,
            written: Duration::ZERO,
            due: VecDeque::new(),
            tail: VecDeque::new(),
            replay: Vec::new(),
            drained_at: None,
        }
    }

    /// Pause: silence the sink now instead of letting its buffer play out. The part that was
    /// written but not yet heard is kept for [`Player::undrain`].
    fn drain(&mut self, now: Instant) {
        let (Some(clock), Some(rate)) = (self.clock, self.rate) else {
            return;
        };
        let latency = self.sink.latency();
        let heard = now
            .saturating_duration_since(clock)
            .saturating_sub(latency)
            .min(self.written);
        let unheard = self.written - heard;
        if unheard < Duration::from_millis(5) {
            return;
        }
        let n = ((unheard.as_secs_f64() * rate as f64) as usize).min(self.tail.len());
        let from = self.tail.len() - n;
        self.replay = self.tail.iter().skip(from).copied().collect();
        self.tail.clear();
        self.sink.flush();
        self.written = heard;
        self.drained_at = Some(heard);
    }

    fn gain(&self) -> f32 {
        f32::from_bits(self.shared.volume.load(Ordering::Relaxed))
    }

    /// Resume after a drained pause: the unheard audio is written again and the clock and the
    /// pending events are put back in line with it.
    fn undrain(&mut self, now: Instant) {
        let Some(heard) = self.drained_at.take() else {
            return;
        };
        let (Some(rate), Some(old_clock)) = (self.rate, self.clock) else {
            return;
        };
        let replay = std::mem::take(&mut self.replay);
        if !replay.is_empty() {
            let heard = apply_gain(&replay, self.gain());
            if let Err(e) = self.sink.write(rate, &heard) {
                *self.shared.error.lock().unwrap() = Some(format!("{e:#}"));
                return;
            }
            self.written += Duration::from_secs_f64(replay.len() as f64 / rate as f64);
            self.tail.extend(replay);
        }
        // the audio at stream position p was due at old_clock + p + latency; now at now + (p - heard) + latency
        let new_clock = now.checked_sub(heard).unwrap_or(now);
        self.clock = Some(new_clock);
        let shift = new_clock.saturating_duration_since(old_clock);
        for d in self.due.iter_mut() {
            d.0 += shift;
        }
    }

    fn run(mut self, rx: Receiver<Play>) {
        loop {
            let idle = self.cur.is_none() && self.ready.is_empty() && self.due.is_empty();
            let wait = if idle || self.paused.is_some() {
                Duration::from_millis(40)
            } else {
                Duration::from_millis(5)
            };
            match rx.recv_timeout(wait) {
                Ok(m) => {
                    if self.handle(m) {
                        break;
                    }
                    while let Ok(m) = rx.try_recv() {
                        if self.handle(m) {
                            return;
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.step();
        }
        self.sink.flush();
    }

    fn handle(&mut self, m: Play) -> bool {
        match m {
            Play::Quit => return true,
            Play::Audio { id, r#gen, pcm } => {
                if r#gen == self.r#gen {
                    self.ready.push_back(Ok((id, pcm)));
                }
            }
            Play::Failed { r#gen, message } => {
                if r#gen == self.r#gen {
                    self.ready.push_back(Err(message));
                }
            }
            Play::Pause => {
                if self.paused.is_none() {
                    let now = Instant::now();
                    self.paused = Some(now);
                    self.drain(now);
                }
            }
            Play::Resume => {
                if let Some(at) = self.paused.take() {
                    if self.drained_at.is_some() {
                        self.undrain(Instant::now());
                    } else {
                        let gap = at.elapsed();
                        if let Some(c) = self.clock.as_mut() {
                            *c += gap;
                        }
                        for d in self.due.iter_mut() {
                            d.0 += gap;
                        }
                    }
                }
            }
            Play::Stop(r#gen) => {
                self.r#gen = r#gen;
                self.ready.clear();
                self.cur = None;
                self.due.clear();
                self.paused = None;
                self.clock = None;
                self.written = Duration::ZERO;
                self.rate = None;
                self.tail.clear();
                self.replay.clear();
                self.drained_at = None;
                self.sink.flush();
            }
        }
        false
    }

    fn emit(&mut self, at: Instant, id: u64, kind: EventKind) {
        self.due.push_back((at, Event { id, kind }));
    }

    fn step(&mut self) {
        if self.paused.is_some() {
            return;
        }
        let now = Instant::now();
        while self.due.front().is_some_and(|d| d.0 <= now) {
            let (_, ev) = self.due.pop_front().unwrap();
            let _ = self.out.send(ev);
        }
        if self.cur.is_none() {
            if let (Some(Err(message)), true) = (self.ready.front().cloned(), self.due.is_empty()) {
                // the sentence before it has been played: now report
                self.ready.clear();
                *self.shared.error.lock().unwrap() = Some(message);
                return;
            }
            if matches!(self.ready.front(), Some(Ok(_))) {
                let Some(Ok((id, pcm))) = self.ready.pop_front() else {
                    return;
                };
                let rate = *self.rate.get_or_insert(pcm.rate.max(1));
                let samples = if pcm.rate == rate {
                    pcm.to_i16()
                } else {
                    Pcm {
                        rate,
                        samples: resample(&pcm.samples, pcm.rate, rate),
                    }
                    .to_i16()
                };
                self.cur = Some(Playing {
                    id,
                    samples,
                    pos: 0,
                });
            }
        }
        let Some(cur) = self.cur.as_mut() else {
            // idle for good: the next sentence starts a fresh clock
            if self.ready.is_empty() && self.due.is_empty() {
                self.clock = None;
                self.written = Duration::ZERO;
                self.tail.clear();
            }
            return;
        };
        let rate = self.rate.unwrap_or(24000);
        let latency = self.sink.latency();
        // when would the next sample be heard
        let mut play_at = match self.clock {
            Some(c) => c + self.written,
            None => now,
        };
        if self.clock.is_none() || play_at < now {
            // the sink ran dry (or nothing was played yet): what we write now is heard now
            self.clock = Some(now - self.written);
            play_at = now;
        }
        if play_at > now + LEAD {
            return;
        }
        let n = ((rate as u128 * CHUNK.as_millis() / 1000) as usize).max(1);
        let end = (cur.pos + n).min(cur.samples.len());
        let first = cur.pos == 0;
        let chunk = &cur.samples[cur.pos..end];
        let id = cur.id;
        let gain = f32::from_bits(self.shared.volume.load(Ordering::Relaxed));
        let heard = apply_gain(chunk, gain);
        if let Err(e) = self.sink.write(rate, &heard) {
            *self.shared.error.lock().unwrap() = Some(format!("{e:#}"));
            self.cur = None;
            self.ready.clear();
            return;
        }
        let dur = Duration::from_secs_f64(chunk.len() as f64 / rate as f64);
        self.tail.extend(chunk.iter().copied());
        let keep = (rate as u128 * (LEAD + latency + CHUNK * 3).as_millis() / 1000) as usize;
        while self.tail.len() > keep {
            self.tail.pop_front();
        }
        cur.pos = end;
        let finished = cur.pos >= cur.samples.len();
        self.written += dur;
        if first {
            self.emit(play_at + latency, id, EventKind::Begin);
        }
        if finished {
            let end_at = play_at + dur + latency;
            self.emit(end_at, id, EventKind::End);
            self.cur = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Session, State, Update};
    use crate::sink::MemorySink;

    /// 5 ms of audio per character at 8 kHz; speed shortens it. Records what it was asked.
    struct Fake {
        calls: Arc<Mutex<Vec<(String, f64, String)>>>,
        delay: Duration,
        fail_on: Option<&'static str>,
    }

    impl Synthesizer for Fake {
        fn id(&self) -> String {
            "fake".into()
        }
        fn voices(&mut self) -> Result<Vec<Voice>> {
            Ok(vec![Voice {
                name: "f1".into(),
                language: "en".into(),
            }])
        }
        fn synthesize(&mut self, r: &SynthRequest) -> Result<Pcm> {
            self.calls
                .lock()
                .unwrap()
                .push((r.text.to_string(), r.speed, r.voice.to_string()));
            let t = Instant::now();
            while t.elapsed() < self.delay {
                if r.cancel.is_cancelled() {
                    return Err(anyhow!("cancelled"));
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            if self.fail_on.is_some_and(|f| r.text.contains(f)) {
                return Err(anyhow!("the server said no"));
            }
            let n = (r.text.chars().count() as f64 * 40.0 / r.speed) as usize;
            Ok(Pcm {
                rate: 8000,
                samples: vec![0.25; n],
            })
        }
    }

    type Calls = Arc<Mutex<Vec<(String, f64, String)>>>;

    /// A sound card that plays in real time: what was written but not yet heard when it is
    /// flushed is lost, like the buffer of a real player that is killed.
    #[derive(Clone)]
    struct SimSink {
        heard: Arc<Mutex<Vec<i16>>>,
        t0: Arc<Mutex<Option<Instant>>>,
        flushes: Arc<Mutex<usize>>,
        latency: Duration,
    }

    impl SimSink {
        fn new(latency_ms: u64) -> SimSink {
            SimSink {
                heard: Arc::default(),
                t0: Arc::default(),
                flushes: Arc::default(),
                latency: Duration::from_millis(latency_ms),
            }
        }
    }

    impl AudioSink for SimSink {
        fn write(&mut self, _rate: u32, samples: &[i16]) -> Result<()> {
            self.t0.lock().unwrap().get_or_insert_with(Instant::now);
            self.heard.lock().unwrap().extend_from_slice(samples);
            Ok(())
        }
        fn flush(&mut self) {
            *self.flushes.lock().unwrap() += 1;
            if let Some(t0) = self.t0.lock().unwrap().take() {
                let audible = t0.elapsed().saturating_sub(self.latency);
                let n = (audible.as_secs_f64() * 8000.0) as usize;
                self.heard.lock().unwrap().truncate(n);
            }
        }
        fn latency(&self) -> Duration {
            self.latency
        }
    }

    fn engine(delay_ms: u64, fail_on: Option<&'static str>) -> (PipelineEngine, MemorySink, Calls) {
        let calls: Calls = Arc::default();
        let sink = MemorySink::default();
        let e = PipelineEngine::new(
            "fake",
            Box::new(Fake {
                calls: calls.clone(),
                delay: Duration::from_millis(delay_ms),
                fail_on,
            }),
            Box::new(sink.clone()),
            None,
            3,
        );
        (e, sink, calls)
    }

    fn run(s: &mut Session, e: &mut PipelineEngine, secs: u64) -> Vec<Update> {
        let end = Instant::now() + Duration::from_secs(secs);
        let mut got = Vec::new();
        while Instant::now() < end && s.state() != State::Stopped {
            got.extend(s.tick(e));
            std::thread::sleep(Duration::from_millis(5));
        }
        got
    }

    #[test]
    fn sentences_are_played_in_order_with_their_events() {
        let (mut e, sink, calls) = engine(0, None);
        let mut s = Session::new();
        s.load(&mut e, "One. Two. Three.");
        assert_eq!(s.play_from(&mut e, 0), None);
        let got = run(&mut s, &mut e, 10);
        assert_eq!(
            got,
            vec![
                Update::Speaking(0, 4),
                Update::Speaking(5, 9),
                Update::Speaking(10, 16),
                Update::Finished
            ]
        );
        let texts: Vec<String> = calls.lock().unwrap().iter().map(|c| c.0.clone()).collect();
        assert_eq!(texts, vec!["One.", "Two.", "Three."]);
        // 4 + 4 + 6 characters at 40 samples: all of it reached the sink, back to back
        let w = sink.written.lock().unwrap();
        assert_eq!(w.len(), 14 * 40);
        assert!(w.iter().all(|s| *s == 8191 || *s == 8192));
        assert_eq!(*sink.rate.lock().unwrap(), 8000);
    }

    #[test]
    fn the_next_sentences_are_synthesized_while_the_first_plays() {
        let (mut e, _sink, calls) = engine(0, None);
        let mut s = Session::new();
        // long sentences: ~1 s of audio each
        let t = format!(
            "{}. {}. {}. {}.",
            "a".repeat(200),
            "b".repeat(200),
            "c".repeat(200),
            "d".repeat(200)
        );
        s.load(&mut e, &t);
        s.play_from(&mut e, 0);
        let start = Instant::now();
        while calls.lock().unwrap().len() < 3 && start.elapsed() < Duration::from_secs(3) {
            s.tick(&mut e);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            calls.lock().unwrap().len(),
            3,
            "three ahead before the first ended"
        );
        let begun = run(&mut s, &mut e, 1);
        assert!(begun.contains(&Update::Speaking(0, 201)), "{begun:?}");
        s.stop(&mut e);
    }

    #[test]
    fn stopping_silences_everything_at_once() {
        let (mut e, sink, _calls) = engine(0, None);
        let mut s = Session::new();
        let t = format!("{}. {}.", "a".repeat(400), "b".repeat(400));
        s.load(&mut e, &t);
        s.play_from(&mut e, 0);
        run(&mut s, &mut e, 0);
        std::thread::sleep(Duration::from_millis(120));
        s.stop(&mut e);
        std::thread::sleep(Duration::from_millis(30));
        let n = sink.written.lock().unwrap().len();
        assert!(n > 0, "something was played");
        assert!(n < 400 * 40, "but not all of it: {n}");
        assert!(*sink.flushes.lock().unwrap() >= 1, "the buffer was dropped");
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(sink.written.lock().unwrap().len(), n, "nothing after stop");
        assert!(e.poll().is_empty(), "and no events");
    }

    #[test]
    fn stopping_cancels_a_synthesis_in_progress() {
        let (mut e, sink, calls) = engine(5000, None);
        let id = e.speak("Slow one.").unwrap();
        assert_eq!(id, 1);
        let start = Instant::now();
        while calls.lock().unwrap().is_empty() && start.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(5));
        }
        e.stop();
        // the worker is free again quickly: a new message is synthesized (the voices call
        // goes through the same worker)
        let t = Instant::now();
        assert_eq!(e.voices().len(), 1);
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
        assert!(sink.written.lock().unwrap().is_empty());
        assert!(
            e.take_error().is_none(),
            "a cancelled synthesis is not an error"
        );
    }

    #[test]
    fn a_failing_synthesis_stops_the_session_with_the_message() {
        let (mut e, _sink, _calls) = engine(0, Some("boom"));
        let mut s = Session::new();
        s.load(&mut e, "Fine. Then boom. After.");
        s.play_from(&mut e, 0);
        let got = run(&mut s, &mut e, 5);
        assert!(got.contains(&Update::Speaking(0, 5)), "{got:?}");
        assert!(
            got.iter()
                .any(|u| matches!(u, Update::Error(m) if m.contains("the server said no"))),
            "{got:?}"
        );
        assert_eq!(s.state(), State::Stopped);
    }

    #[test]
    fn speed_above_two_asks_the_engine_for_two_and_stretches_the_rest() {
        let (mut e, sink, calls) = engine(0, None);
        e.set_speed(3.0);
        e.set_voice("f1");
        e.set_language("en");
        let mut s = Session::new();
        s.load(&mut e, &"x".repeat(400));
        s.play_from(&mut e, 0);
        run(&mut s, &mut e, 5);
        let c = calls.lock().unwrap();
        assert!(
            (c[0].1 - 2.0).abs() < 1e-9,
            "the engine gets its maximum: {}",
            c[0].1
        );
        // 400 chars at speed 2 = 8000 samples, stretched by 1.5 = about 5333
        let n = sink.written.lock().unwrap().len() as i64;
        assert!((n - 5333).abs() < 600, "{n}");
    }

    #[test]
    fn pitch_asks_for_a_slower_voice_and_plays_faster() {
        let (mut e, sink, calls) = engine(0, None);
        e.set_speed(1.0);
        e.set_pitch(1.25);
        e.set_voice("f1");
        e.set_language("en");
        let mut s = Session::new();
        s.load(&mut e, &"x".repeat(100));
        s.play_from(&mut e, 0);
        run(&mut s, &mut e, 5);
        let c = calls.lock().unwrap();
        assert!(
            (c[0].1 - 0.8).abs() < 1e-9,
            "the model speed is 1/1.25: {}",
            c[0].1
        );
        assert_eq!(c[0].2, "f1");
        // 100 chars at speed 0.8 = 5000 samples, played 1.25 times faster = 4000
        let n = sink.written.lock().unwrap().len();
        assert!((n as i64 - 4000).abs() < 4, "{n}");
        assert!(e.debug_state().contains("pitch=1.25"));
    }

    #[test]
    fn the_cache_makes_the_second_reading_free() {
        let dir = std::env::temp_dir().join(format!("bookx-pipe-{}", uuid::Uuid::new_v4()));
        let calls: Calls = Arc::default();
        let sink = MemorySink::default();
        let mut e = PipelineEngine::new(
            "fake",
            Box::new(Fake {
                calls: calls.clone(),
                delay: Duration::ZERO,
                fail_on: None,
            }),
            Box::new(sink.clone()),
            Some(SentenceCache::new(&dir, 5)),
            2,
        );
        e.set_book("book-1");
        for round in 0..2 {
            let mut s = Session::new();
            s.load(&mut e, "Hello there. Again.");
            s.play_from(&mut e, 0);
            let got = run(&mut s, &mut e, 5);
            assert_eq!(got.last(), Some(&Update::Finished), "round {round}");
        }
        assert_eq!(
            calls.lock().unwrap().len(),
            2,
            "the second round came from the cache"
        );
        assert_eq!(sink.written.lock().unwrap().len(), 2 * (12 + 6) * 40);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pause_holds_the_audio_and_resume_continues() {
        let (mut e, sink, _calls) = engine(0, None);
        let mut s = Session::new();
        let t = format!("{}. {}.", "a".repeat(300), "b".repeat(300));
        s.load(&mut e, &t);
        s.play_from(&mut e, 0);
        run(&mut s, &mut e, 0);
        std::thread::sleep(Duration::from_millis(100));
        s.pause(&mut e);
        std::thread::sleep(Duration::from_millis(60));
        let n = sink.written.lock().unwrap().len();
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(
            sink.written.lock().unwrap().len(),
            n,
            "nothing is written while paused"
        );
        s.resume(&mut e);
        let got = run(&mut s, &mut e, 10);
        assert_eq!(got.last(), Some(&Update::Finished));
        // a pause takes back what the sink had buffered, so the unheard tail (at most 0.5 s) is written twice
        let n = sink.written.lock().unwrap().len();
        assert!((602 * 40..=602 * 40 + 4000).contains(&n), "{n}");
    }

    #[test]
    fn pausing_silences_the_buffered_audio_at_once_and_resume_loses_nothing() {
        let sink = SimSink::new(120);
        let calls: Calls = Arc::default();
        let mut e = PipelineEngine::new(
            "fake",
            Box::new(Fake {
                calls,
                delay: Duration::ZERO,
                fail_on: None,
            }),
            Box::new(sink.clone()),
            None,
            3,
        );
        let mut s = Session::new();
        let t = format!("{}. {}.", "a".repeat(300), "b".repeat(300));
        s.load(&mut e, &t);
        s.play_from(&mut e, 0);
        run(&mut s, &mut e, 0);
        std::thread::sleep(Duration::from_millis(400));
        s.pause(&mut e);
        std::thread::sleep(Duration::from_millis(80));
        assert!(
            *sink.flushes.lock().unwrap() >= 1,
            "the sink was emptied by the pause"
        );
        let at_pause = sink.heard.lock().unwrap().len();
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(
            sink.heard.lock().unwrap().len(),
            at_pause,
            "silent while paused"
        );
        s.resume(&mut e);
        let got = run(&mut s, &mut e, 10);
        assert_eq!(got.last(), Some(&Update::Finished));
        let total = sink.heard.lock().unwrap().len();
        let want = 602 * 40;
        assert!(
            total.abs_diff(want) < 8000 / 20,
            "all the audio is heard exactly once (within 50 ms): {total} of {want}"
        );
    }

    #[test]
    fn prefetched_text_is_synthesized_before_it_is_spoken_and_only_once() {
        let (mut e, _sink, calls) = engine(0, None);
        e.prefetch(&["Next chapter opening.".to_string()]);
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(calls.lock().unwrap().len(), 1, "made ahead of time");
        let mut s = Session::new();
        s.load(&mut e, "Next chapter opening.");
        s.play_from(&mut e, 0);
        let got = run(&mut s, &mut e, 5);
        assert_eq!(got.last(), Some(&Update::Finished));
        assert_eq!(
            calls.lock().unwrap().len(),
            1,
            "speaking it used the audio made ahead"
        );
    }

    #[test]
    fn a_sink_that_fails_is_reported() {
        struct Broken;
        impl AudioSink for Broken {
            fn write(&mut self, _: u32, _: &[i16]) -> Result<()> {
                Err(anyhow!("no audio player found"))
            }
            fn flush(&mut self) {}
        }
        let calls: Calls = Arc::default();
        let mut e = PipelineEngine::new(
            "fake",
            Box::new(Fake {
                calls,
                delay: Duration::ZERO,
                fail_on: None,
            }),
            Box::new(Broken),
            None,
            2,
        );
        let mut s = Session::new();
        s.load(&mut e, "Hello.");
        s.play_from(&mut e, 0);
        let got = run(&mut s, &mut e, 5);
        assert!(
            got.iter()
                .any(|u| matches!(u, Update::Error(m) if m.contains("no audio player")))
        );
    }

    #[test]
    fn voices_come_from_the_synthesizer_once() {
        let (mut e, _s, _c) = engine(0, None);
        assert_eq!(e.voices()[0].name, "f1");
        assert_eq!(e.voices().len(), 1);
        assert_eq!(e.name(), "fake");
        assert_eq!(e.lookahead(), 3);
    }

    #[test]
    fn volume_scales_the_samples_and_full_volume_leaves_them() {
        let s = [1000i16, -2000, 0];
        assert!(matches!(apply_gain(&s, 1.0), std::borrow::Cow::Borrowed(_)));
        assert_eq!(apply_gain(&s, 0.5).as_ref(), &[500, -1000, 0]);
        assert_eq!(apply_gain(&s, 0.0).as_ref(), &[0, 0, 0]);
    }

    #[test]
    fn the_volume_reaches_the_sink() {
        let (mut e, sink, _calls) = engine(0, None);
        e.set_volume(0.5);
        e.set_volume(f64::NAN);
        e.set_volume(0.5);
        e.set_voice("v");
        let id = e.speak("abcd").unwrap();
        let t = Instant::now();
        let mut done = false;
        while t.elapsed() < Duration::from_secs(5) && !done {
            done = e
                .poll()
                .iter()
                .any(|ev| ev.id == id && ev.kind == EventKind::End);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(done);
        let w = sink.written.lock().unwrap();
        let peak = w.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
        assert!(
            (4090..=4100).contains(&peak),
            "half of the synthesized peak: {peak}"
        );
    }
}
