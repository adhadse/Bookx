//! The engine interface and the reading session on top of it.
//!
//! An [`Engine`] speaks messages in the order they were queued and reports when each one
//! begins and ends. The [`Session`] keeps up to [`Engine::lookahead`] sentences queued, so an
//! engine that synthesizes can work on the next sentences while the current one is played
//! (gapless), and highlights a sentence when its `Begin` arrives.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};

use crate::text::{Sentence, split_sentences};

#[derive(Debug, Clone, PartialEq)]
pub struct Voice {
    pub name: String,
    pub language: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Begin,
    End,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub id: u64,
    pub kind: EventKind,
}

/// A text-to-speech back end.
pub trait Engine {
    /// Queues `text`; returns the id the events of this message carry.
    fn speak(&mut self, text: &str) -> Result<u64>;
    /// Drops everything queued and silences what is playing.
    fn stop(&mut self);
    fn pause(&mut self);
    fn resume(&mut self);
    /// 1.0 = normal. Applies to messages queued afterwards.
    fn set_speed(&mut self, speed: f64);
    /// 1.0 = normal. Applies to messages queued afterwards.
    fn set_pitch(&mut self, _pitch: f64) {}
    /// Playback volume 0.0..=1.0 for engines that mix the audio themselves.
    fn set_volume(&mut self, _volume: f64) {}
    fn set_voice(&mut self, name: &str);
    /// The language of the text (two letters, may be empty) for engines that need it.
    fn set_language(&mut self, _language: &str) {}
    /// The book being read (a stable id), for engines that keep a cache per book.
    fn set_book(&mut self, _id: &str) {}
    fn voices(&mut self) -> Vec<Voice>;
    /// Asks the engine to prepare these texts (the first sentences of the next chapter) so that
    /// speaking them later starts without a gap. Engines that cannot do this ignore it.
    fn prefetch(&mut self, _texts: &[String]) {}
    /// A problem that happened while speaking (a server that does not answer, say): the
    /// session stops and shows it.
    fn take_error(&mut self) -> Option<String> {
        None
    }
    /// Events that arrived since the last call.
    fn poll(&mut self) -> Vec<Event>;
    fn name(&self) -> String;
    /// How many messages the session may keep queued (the one playing included).
    fn lookahead(&self) -> usize {
        1
    }
    /// What the tests may print about the engine.
    fn debug_state(&self) -> String {
        String::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Stopped,
    Playing,
    Paused,
}

/// What the reader has to do after a tick.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    /// This sentence is being spoken: highlight it.
    Speaking(i32, i32),
    /// The chapter is read to the end.
    Finished,
    Error(String),
}

pub struct Session {
    sentences: Vec<Sentence>,
    /// The sentence being spoken.
    idx: usize,
    /// The next sentence to queue.
    next: usize,
    queued: VecDeque<(u64, usize)>,
    state: State,
}

impl Default for Session {
    fn default() -> Self {
        Session::new()
    }
}

impl Session {
    pub fn new() -> Session {
        Session {
            sentences: Vec::new(),
            idx: 0,
            next: 0,
            queued: VecDeque::new(),
            state: State::Stopped,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn sentences(&self) -> &[Sentence] {
        &self.sentences
    }

    /// The sentence being spoken.
    pub fn current(&self) -> Option<&Sentence> {
        (self.state != State::Stopped)
            .then(|| self.sentences.get(self.idx))
            .flatten()
    }

    /// Index of the sentence being spoken in [`Session::sentences`].
    pub fn index(&self) -> Option<usize> {
        (self.state != State::Stopped && self.idx < self.sentences.len()).then_some(self.idx)
    }

    /// Sets the text of the chapter (stops what is playing).
    pub fn load(&mut self, engine: &mut dyn Engine, text: &str) {
        self.stop(engine);
        self.sentences = split_sentences(text);
        self.idx = 0;
        self.next = 0;
    }

    /// Starts reading at the sentence that contains `char_pos` or the first one after it.
    pub fn play_from(&mut self, engine: &mut dyn Engine, char_pos: i32) -> Option<Update> {
        let at = self
            .sentences
            .iter()
            .position(|s| s.end > char_pos)
            .unwrap_or(self.sentences.len());
        self.play_at(engine, at)
    }

    fn play_at(&mut self, engine: &mut dyn Engine, at: usize) -> Option<Update> {
        engine.stop();
        self.queued.clear();
        self.idx = at;
        self.next = at;
        if at >= self.sentences.len() {
            self.state = State::Stopped;
            return Some(Update::Finished);
        }
        self.state = State::Playing;
        self.top_up(engine)
    }

    /// Reads on from the start of the sentence being spoken (after the voice or speed changed).
    pub fn restart(&mut self, engine: &mut dyn Engine) -> Option<Update> {
        if self.state == State::Stopped {
            return None;
        }
        let at = self.idx;
        self.play_at(engine, at)
    }

    /// Jumps `delta` sentences forward (or back) from the one being spoken.
    pub fn skip(&mut self, engine: &mut dyn Engine, delta: i32) -> Option<Update> {
        if self.state == State::Stopped || self.sentences.is_empty() {
            return None;
        }
        let at = (self.idx as i64 + delta as i64).clamp(0, self.sentences.len() as i64 - 1);
        self.play_at(engine, at as usize)
    }

    /// Jumps to sentence number `at` (the player's seek bar), clamped to the chapter.
    pub fn seek(&mut self, engine: &mut dyn Engine, at: usize) -> Option<Update> {
        if self.state == State::Stopped || self.sentences.is_empty() {
            return None;
        }
        self.play_at(engine, at.min(self.sentences.len() - 1))
    }

    fn top_up(&mut self, engine: &mut dyn Engine) -> Option<Update> {
        let want = engine.lookahead().max(1);
        while self.state == State::Playing
            && self.queued.len() < want
            && self.next < self.sentences.len()
        {
            match engine.speak(&self.sentences[self.next].text) {
                Ok(id) => {
                    self.queued.push_back((id, self.next));
                    self.next += 1;
                }
                Err(e) => {
                    engine.stop();
                    self.state = State::Stopped;
                    self.queued.clear();
                    return Some(Update::Error(format!("{e:#}")));
                }
            }
        }
        None
    }

    pub fn pause(&mut self, engine: &mut dyn Engine) {
        if self.state == State::Playing {
            engine.pause();
            self.state = State::Paused;
        }
    }

    pub fn resume(&mut self, engine: &mut dyn Engine) {
        if self.state == State::Paused {
            engine.resume();
            self.state = State::Playing;
        }
    }

    pub fn stop(&mut self, engine: &mut dyn Engine) {
        if self.state != State::Stopped {
            engine.stop();
        }
        self.state = State::Stopped;
        self.queued.clear();
    }

    /// Reads the events of the engine, keeps the queue of the engine filled and tells what
    /// is being spoken now.
    pub fn tick(&mut self, engine: &mut dyn Engine) -> Vec<Update> {
        let mut out = Vec::new();
        for ev in engine.poll() {
            if self.state == State::Stopped {
                continue;
            }
            let Some(pos) = self.queued.iter().position(|q| q.0 == ev.id) else {
                continue;
            };
            match ev.kind {
                EventKind::Begin => {
                    self.queued.drain(..pos);
                    let at = self.queued[0].1;
                    self.idx = at;
                    let s = &self.sentences[at];
                    out.push(Update::Speaking(s.start, s.end));
                }
                EventKind::End => {
                    self.queued.drain(..=pos);
                    if self.queued.is_empty() && self.next >= self.sentences.len() {
                        self.state = State::Stopped;
                        out.push(Update::Finished);
                    }
                }
                EventKind::Cancel => {
                    // somebody else stopped the speech: stop with it
                    self.state = State::Stopped;
                    self.queued.clear();
                }
            }
        }
        if let Some(e) = engine.take_error()
            && self.state != State::Stopped
        {
            engine.stop();
            self.state = State::Stopped;
            self.queued.clear();
            out.push(Update::Error(e));
            return out;
        }
        if self.state == State::Playing {
            out.extend(self.top_up(engine));
        }
        out
    }
}

/// The sleep timer: stop reading after some minutes or at the end of the chapter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SleepTimer {
    Off,
    At(Instant),
    EndOfChapter,
}

impl SleepTimer {
    pub fn after_minutes(m: u32) -> SleepTimer {
        if m == 0 {
            SleepTimer::Off
        } else {
            SleepTimer::At(Instant::now() + Duration::from_secs(m as u64 * 60))
        }
    }

    /// Time is up: stop now (checked between sentences by the reader).
    pub fn expired(&self, now: Instant) -> bool {
        matches!(self, SleepTimer::At(t) if now >= *t)
    }

    /// Stop instead of going on with the next chapter.
    pub fn stops_at_chapter_end(&self) -> bool {
        matches!(self, SleepTimer::EndOfChapter)
    }
}

// ---- the mock ----------------------------------------------------------------------------

/// A speaker that does not make a sound: every message "plays" for `delay`, one after the
/// other like a real engine's queue.
pub struct MockEngine {
    pub spoken: Arc<Mutex<Vec<String>>>,
    pub log: Arc<Mutex<Vec<String>>>,
    next_id: u64,
    delay: Duration,
    queue: VecDeque<u64>,
    current: Option<(u64, Instant)>,
    paused: Option<Duration>,
    ahead: usize,
    fail: bool,
}

impl MockEngine {
    pub fn new(delay: Duration) -> MockEngine {
        MockEngine {
            spoken: Arc::default(),
            log: Arc::default(),
            next_id: 0,
            delay,
            queue: VecDeque::new(),
            current: None,
            paused: None,
            ahead: 1,
            fail: false,
        }
    }

    /// A mock that lets the session queue `n` messages ahead.
    pub fn with_lookahead(mut self, n: usize) -> MockEngine {
        self.ahead = n;
        self
    }

    /// A mock whose `speak` fails.
    pub fn broken() -> MockEngine {
        let mut m = MockEngine::new(Duration::ZERO);
        m.fail = true;
        m
    }

    fn note(&self, s: &str) {
        self.log.lock().unwrap().push(s.to_string());
    }
}

impl Engine for MockEngine {
    fn speak(&mut self, text: &str) -> Result<u64> {
        if self.fail {
            bail!("the mock speaker is out of order");
        }
        self.next_id += 1;
        self.spoken.lock().unwrap().push(text.to_string());
        self.queue.push_back(self.next_id);
        Ok(self.next_id)
    }

    fn stop(&mut self) {
        self.note("stop");
        self.queue.clear();
        self.current = None;
        self.paused = None;
    }

    fn pause(&mut self) {
        self.note("pause");
        if let Some((_, due)) = self.current {
            self.paused = Some(due.saturating_duration_since(Instant::now()));
        } else {
            self.paused = Some(Duration::ZERO);
        }
    }

    fn resume(&mut self) {
        self.note("resume");
        if let Some(left) = self.paused.take()
            && let Some(c) = self.current.as_mut()
        {
            c.1 = Instant::now() + left;
        }
    }

    fn set_speed(&mut self, speed: f64) {
        self.note(&format!("speed {speed:.1}"));
    }

    fn set_pitch(&mut self, pitch: f64) {
        self.note(&format!("pitch {pitch:.2}"));
    }

    fn set_voice(&mut self, name: &str) {
        self.note(&format!("voice {name}"));
    }

    fn set_language(&mut self, language: &str) {
        self.note(&format!("language {language}"));
    }

    fn voices(&mut self) -> Vec<Voice> {
        vec![
            Voice {
                name: "mock-a".into(),
                language: "en".into(),
            },
            Voice {
                name: "mock-b".into(),
                language: "de".into(),
            },
        ]
    }

    fn poll(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        if self.paused.is_some() {
            return out;
        }
        loop {
            if self.current.is_none() {
                let Some(id) = self.queue.pop_front() else {
                    break;
                };
                self.current = Some((id, Instant::now() + self.delay));
                out.push(Event {
                    id,
                    kind: EventKind::Begin,
                });
            }
            match self.current {
                Some((id, due)) if Instant::now() >= due => {
                    self.current = None;
                    out.push(Event {
                        id,
                        kind: EventKind::End,
                    });
                }
                _ => break,
            }
        }
        out
    }

    fn name(&self) -> String {
        "mock".into()
    }

    fn lookahead(&self) -> usize {
        self.ahead
    }

    fn debug_state(&self) -> String {
        let spoken = self.spoken.lock().unwrap();
        format!(
            "spoken={} last={:?} log={:?}",
            spoken.len(),
            spoken
                .last()
                .map(|s| s.chars().take(24).collect::<String>()),
            self.log.lock().unwrap()
        )
    }
}

/// The voices to offer for a book in `language`: those of the language (all when none match),
/// sorted by name, at most `limit`.
pub fn voices_for(voices: &[Voice], language: &str, limit: usize) -> Vec<Voice> {
    let code = language
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_lowercase();
    let mut v: Vec<Voice> = voices
        .iter()
        .filter(|x| !code.is_empty() && x.language.to_lowercase().starts_with(&code))
        .cloned()
        .collect();
    if v.is_empty() {
        v = voices.to_vec();
    }
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v.dedup_by(|a, b| a.name == b.name);
    v.truncate(limit);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock() -> MockEngine {
        MockEngine::new(Duration::ZERO)
    }

    /// Ticks until the session has nothing more to say (or `n` ticks passed).
    fn run(s: &mut Session, e: &mut dyn Engine, n: usize) -> Vec<Update> {
        let mut got = Vec::new();
        for _ in 0..n {
            got.extend(s.tick(e));
            if s.state() == State::Stopped {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        got
    }

    #[test]
    fn a_session_reads_sentence_after_sentence() {
        let mut e = mock();
        let mut s = Session::new();
        s.load(&mut e, "One. Two. Three.");
        assert_eq!(s.sentences().len(), 3);
        assert_eq!(s.play_from(&mut e, 0), None);
        assert_eq!(s.state(), State::Playing);
        let got = run(&mut s, &mut e, 50);
        assert_eq!(
            got,
            vec![
                Update::Speaking(0, 4),
                Update::Speaking(5, 9),
                Update::Speaking(10, 16),
                Update::Finished
            ]
        );
        assert_eq!(s.state(), State::Stopped);
        assert_eq!(*e.spoken.lock().unwrap(), vec!["One.", "Two.", "Three."]);
    }

    #[test]
    fn the_session_keeps_the_lookahead_queued() {
        let mut e = MockEngine::new(Duration::from_millis(30)).with_lookahead(3);
        let mut s = Session::new();
        s.load(&mut e, "One. Two. Three. Four. Five.");
        s.play_from(&mut e, 0);
        assert_eq!(
            e.spoken.lock().unwrap().len(),
            3,
            "three are queued at once"
        );
        // the first one starts: its slot is refilled
        std::thread::sleep(Duration::from_millis(5));
        let u = s.tick(&mut e);
        assert_eq!(u, vec![Update::Speaking(0, 4)]);
        std::thread::sleep(Duration::from_millis(40));
        s.tick(&mut e);
        assert_eq!(e.spoken.lock().unwrap().len(), 4);
        let rest = run(&mut s, &mut e, 400);
        assert_eq!(rest.last(), Some(&Update::Finished));
        assert_eq!(e.spoken.lock().unwrap().len(), 5);
    }

    #[test]
    fn reading_starts_at_the_sentence_after_the_position() {
        let mut e = mock();
        let mut s = Session::new();
        s.load(&mut e, "One. Two. Three.");
        // in the middle of "Two."
        assert_eq!(s.play_from(&mut e, 6), None);
        assert_eq!(run(&mut s, &mut e, 20)[0], Update::Speaking(5, 9));
        // past the end: nothing left
        assert_eq!(s.play_from(&mut e, 999), Some(Update::Finished));
        assert_eq!(s.state(), State::Stopped);
    }

    #[test]
    fn pause_holds_the_next_sentence_back_and_resume_goes_on() {
        let mut e = MockEngine::new(Duration::from_millis(5));
        let mut s = Session::new();
        s.load(&mut e, "One. Two.");
        s.play_from(&mut e, 0);
        s.pause(&mut e);
        assert_eq!(s.state(), State::Paused);
        std::thread::sleep(Duration::from_millis(20));
        assert!(s.tick(&mut e).is_empty(), "nothing is read while paused");
        s.resume(&mut e);
        let got = run(&mut s, &mut e, 100);
        assert_eq!(got.last(), Some(&Update::Finished));
        assert!(got.contains(&Update::Speaking(5, 9)));
        assert_eq!(
            *e.log.lock().unwrap(),
            vec!["stop", "pause", "resume"],
            "the engine was told"
        );
    }

    #[test]
    fn stop_ignores_the_late_events_of_the_old_messages() {
        let mut e = MockEngine::new(Duration::from_millis(5));
        let mut s = Session::new();
        s.load(&mut e, "One. Two.");
        s.play_from(&mut e, 0);
        s.stop(&mut e);
        assert_eq!(s.state(), State::Stopped);
        assert!(s.current().is_none());
        std::thread::sleep(Duration::from_millis(15));
        assert!(s.tick(&mut e).is_empty());
        assert_eq!(e.spoken.lock().unwrap().len(), 1);
    }

    #[test]
    fn restart_and_skip_move_within_the_chapter() {
        let mut e = MockEngine::new(Duration::from_millis(50));
        let mut s = Session::new();
        s.load(&mut e, "One. Two. Three.");
        assert!(
            s.restart(&mut e).is_none(),
            "nothing plays: nothing to restart"
        );
        s.play_from(&mut e, 0);
        std::thread::sleep(Duration::from_millis(5));
        s.tick(&mut e);
        s.skip(&mut e, 1);
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(s.tick(&mut e), vec![Update::Speaking(5, 9)]);
        s.restart(&mut e);
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(s.tick(&mut e), vec![Update::Speaking(5, 9)]);
        s.skip(&mut e, -5);
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(
            s.tick(&mut e),
            vec![Update::Speaking(0, 4)],
            "clamped to the first"
        );
        s.skip(&mut e, 99);
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(
            s.tick(&mut e),
            vec![Update::Speaking(10, 16)],
            "and to the last"
        );
    }

    #[test]
    fn seek_jumps_to_a_sentence_number_and_index_tells_where_we_are() {
        let mut e = MockEngine::new(Duration::from_millis(50));
        let mut s = Session::new();
        s.load(&mut e, "One. Two. Three.");
        assert_eq!(s.index(), None);
        assert!(s.seek(&mut e, 1).is_none(), "stopped: nothing to seek");
        s.play_from(&mut e, 0);
        assert_eq!(s.index(), Some(0));
        s.seek(&mut e, 2);
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(s.tick(&mut e), vec![Update::Speaking(10, 16)]);
        assert_eq!(s.index(), Some(2));
        s.seek(&mut e, 99);
        assert_eq!(s.index(), Some(2), "clamped to the last sentence");
        s.stop(&mut e);
        assert_eq!(s.index(), None);
    }

    #[test]
    fn events_of_other_messages_are_ignored() {
        struct Odd(Vec<Event>);
        impl Engine for Odd {
            fn speak(&mut self, _: &str) -> Result<u64> {
                Ok(7)
            }
            fn stop(&mut self) {}
            fn pause(&mut self) {}
            fn resume(&mut self) {}
            fn set_speed(&mut self, _: f64) {}
            fn set_voice(&mut self, _: &str) {}
            fn voices(&mut self) -> Vec<Voice> {
                vec![]
            }
            fn poll(&mut self) -> Vec<Event> {
                std::mem::take(&mut self.0)
            }
            fn name(&self) -> String {
                "odd".into()
            }
        }
        let mut e = Odd(vec![Event {
            id: 3,
            kind: EventKind::End,
        }]);
        let mut s = Session::new();
        s.load(&mut e, "One. Two.");
        s.play_from(&mut e, 0);
        assert!(s.tick(&mut e).is_empty());
        e.0.push(Event {
            id: 7,
            kind: EventKind::Cancel,
        });
        s.tick(&mut e);
        assert_eq!(
            s.state(),
            State::Stopped,
            "a cancel from outside stops the session"
        );
    }

    #[test]
    fn an_engine_error_stops_the_session_with_a_message() {
        let mut e = MockEngine::broken();
        let mut s = Session::new();
        s.load(&mut e, "One.");
        match s.play_from(&mut e, 0) {
            Some(Update::Error(m)) => assert!(m.contains("out of order")),
            other => panic!("{other:?}"),
        }
        assert_eq!(s.state(), State::Stopped);
    }

    #[test]
    fn the_sleep_timer_expires_or_waits_for_the_chapter_end() {
        assert_eq!(SleepTimer::after_minutes(0), SleepTimer::Off);
        let t = SleepTimer::after_minutes(1);
        assert!(!t.expired(Instant::now()));
        assert!(t.expired(Instant::now() + Duration::from_secs(61)));
        assert!(!SleepTimer::Off.expired(Instant::now() + Duration::from_secs(9999)));
        assert!(SleepTimer::EndOfChapter.stops_at_chapter_end());
        assert!(!t.stops_at_chapter_end());
    }

    #[test]
    fn voices_are_filtered_by_language_sorted_and_limited() {
        let v = |n: &str, l: &str| Voice {
            name: n.into(),
            language: l.into(),
        };
        let all = vec![v("b", "en-US"), v("a", "en"), v("c", "de"), v("a", "en")];
        let en = voices_for(&all, "en-GB", 10);
        assert_eq!(
            en.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(voices_for(&all, "xx", 10).len(), 3, "no match: everything");
        assert_eq!(voices_for(&all, "en", 1).len(), 1);
    }
}
