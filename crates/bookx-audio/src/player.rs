//! The playback engine: a worker thread that decodes the parts of an [`AudioBook`] one after the
//! other, changes the speed without changing the pitch and writes 16-bit samples to an
//! [`AudioSink`].
//!
//! The default sink pipes the samples into `pacat` / `pw-cat` / `aplay`, like the read-aloud
//! feature does (no native sound library, a Flatpak needs only the PulseAudio socket). Because
//! the pipe buffers a fraction of a second, a pause flushes it and the position reported is what
//! was written minus the output latency.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};

use crate::decode::Decoder;
use crate::model::AudioBook;
use crate::stretch::{clamp_speed, Stretcher};

// ---- sinks ---------------------------------------------------------------------------------

pub trait AudioSink: Send {
    /// Plays interleaved 16-bit samples after what was written before. May block.
    fn write(&mut self, rate: u32, channels: u8, samples: &[i16]) -> Result<()>;
    /// Drops what is buffered and silences the output.
    fn flush(&mut self);
    /// What the output buffers before a sample is heard.
    fn latency(&self) -> Duration {
        Duration::ZERO
    }
}

/// Keeps the samples (tests). `delay` makes every write take that long, so a test can act while
/// "playing".
#[derive(Clone, Default)]
pub struct MemorySink {
    pub written: Arc<Mutex<Vec<i16>>>,
    pub formats: Arc<Mutex<Vec<(u32, u8)>>>,
    pub flushes: Arc<Mutex<usize>>,
    pub delay: Duration,
}

impl MemorySink {
    pub fn with_delay(delay: Duration) -> MemorySink {
        MemorySink {
            delay,
            ..Default::default()
        }
    }
}

impl AudioSink for MemorySink {
    fn write(&mut self, rate: u32, channels: u8, samples: &[i16]) -> Result<()> {
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        let mut f = self.formats.lock().unwrap();
        if f.last() != Some(&(rate, channels)) {
            f.push((rate, channels));
        }
        self.written.lock().unwrap().extend_from_slice(samples);
        Ok(())
    }

    fn flush(&mut self) {
        *self.flushes.lock().unwrap() += 1;
    }
}

/// A sink that plays nothing but takes the time the audio would take, `speedup` times faster
/// (`0` = no waiting at all). For the UI scenarios and machines without a sound server.
pub struct NullSink {
    pub speedup: f64,
}

impl AudioSink for NullSink {
    fn write(&mut self, rate: u32, channels: u8, samples: &[i16]) -> Result<()> {
        if self.speedup > 0.0 {
            let secs = samples.len() as f64 / (rate.max(1) as f64 * channels.max(1) as f64);
            std::thread::sleep(Duration::from_secs_f64(secs / self.speedup));
        }
        Ok(())
    }

    fn flush(&mut self) {}
}

fn in_path(program: &str) -> bool {
    crate::command::system_program(program).is_some()
}

/// `(program, args)` of the command line players to try for this format, in order.
pub fn player_candidates(rate: u32, channels: u8) -> Vec<(String, Vec<String>)> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let mut pacat = s(&[
        "--playback",
        "--raw",
        "--format=s16le",
        "--latency-msec=100",
        "--client-name=Bookx",
        "--stream-name=Audiobook",
    ]);
    pacat.push(format!("--channels={channels}"));
    pacat.push(format!("--rate={rate}"));
    let mut pw = s(&["--playback", "--raw", "--format", "s16"]);
    pw.extend([
        "--channels".into(),
        channels.to_string(),
        "--rate".into(),
        rate.to_string(),
        "-".into(),
    ]);
    let mut aplay = s(&["-q", "-t", "raw", "-f", "S16_LE"]);
    aplay.extend([
        "-c".into(),
        channels.to_string(),
        "-r".into(),
        rate.to_string(),
        "-".into(),
    ]);
    vec![
        ("pacat".into(), pacat),
        ("pw-cat".into(), pw),
        ("aplay".into(), aplay),
    ]
}

/// Pipes samples into a command line player.
pub struct CommandSink {
    format: (u32, u8),
    child: Option<Child>,
    custom: Option<String>,
}

impl Default for CommandSink {
    fn default() -> Self {
        CommandSink::new()
    }
}

impl CommandSink {
    pub fn new() -> CommandSink {
        CommandSink {
            format: (0, 0),
            child: None,
            custom: std::env::var("BOOKX_AUDIO_PLAYER")
                .ok()
                .filter(|s| !s.trim().is_empty()),
        }
    }

    /// Is there a player on this machine?
    pub fn available() -> bool {
        std::env::var_os("BOOKX_AUDIO_PLAYER").is_some()
            || player_candidates(44_100, 2).iter().any(|c| in_path(&c.0))
    }

    fn spawn(&self, rate: u32, channels: u8) -> Result<Child> {
        if let Some(cmd) = &self.custom {
            let (program, args) = crate::command::parse_player(
                cmd,
                &[
                    ("{rate}", rate.to_string()),
                    ("{channels}", channels.to_string()),
                ],
            )
            .context("BOOKX_AUDIO_PLAYER")?;
            return Command::new(program)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .context("starting the player from BOOKX_AUDIO_PLAYER");
        }
        for (program, args) in player_candidates(rate, channels) {
            let Some(program) = crate::command::system_program(&program) else {
                continue;
            };
            if let Ok(c) = Command::new(program)
                .args(&args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                return Ok(c);
            }
        }
        Err(anyhow!("no sound player found (pacat, pw-cat or aplay)"))
    }

    fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            drop(c.stdin.take());
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for CommandSink {
    fn drop(&mut self) {
        self.stop();
    }
}

impl AudioSink for CommandSink {
    fn write(&mut self, rate: u32, channels: u8, samples: &[i16]) -> Result<()> {
        if self.child.is_none() || self.format != (rate, channels) {
            self.stop();
            self.child = Some(self.spawn(rate, channels)?);
            self.format = (rate, channels);
        }
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for s in samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        let stdin = self
            .child
            .as_mut()
            .and_then(|c| c.stdin.as_mut())
            .ok_or_else(|| anyhow!("the player has no input"))?;
        if let Err(e) = stdin.write_all(&bytes) {
            self.stop();
            return Err(anyhow!("the sound player stopped: {e}"));
        }
        Ok(())
    }

    fn flush(&mut self) {
        self.stop();
    }

    fn latency(&self) -> Duration {
        let (rate, ch) = self.format;
        if rate == 0 {
            return Duration::ZERO;
        }
        // the player's own buffer (100 ms) plus a full pipe (64 KiB)
        let bytes_per_sec = rate as f64 * ch as f64 * 2.0;
        Duration::from_secs_f64(0.1 + 65_536.0 / bytes_per_sec)
    }
}

/// The sink for this process: `BOOKX_AUDIO_SINK=null` plays nothing but takes the time,
/// `null:20` takes a twentieth of it, `null:0` is instant; otherwise the system's sound server.
pub fn sink_from_env() -> Box<dyn AudioSink> {
    match std::env::var("BOOKX_AUDIO_SINK").ok().as_deref() {
        Some("null") => Box::new(NullSink { speedup: 1.0 }),
        Some(s) if s.starts_with("null:") => Box::new(NullSink {
            speedup: s[5..].parse().unwrap_or(1.0),
        }),
        _ => Box::new(CommandSink::new()),
    }
}

// ---- the engine ----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Paused,
    Playing,
    Ended,
}

#[derive(Debug, Clone)]
pub struct Status {
    pub state: State,
    /// Milliseconds from the start of the book that are being heard.
    pub position_ms: u64,
    pub speed: f32,
    pub volume: f32,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The book played to its end.
    Ended,
    /// Playback stopped at the position set with [`Player::pause_at`].
    PausedAt(u64),
    Error(String),
}

enum Cmd {
    Play,
    Pause,
    Seek(u64),
    SetSpeed(f32),
    SetVolume(f32),
    PauseAt(Option<u64>),
    Quit,
}

/// The handle the UI keeps. Dropping it stops the worker.
pub struct Player {
    tx: Sender<Cmd>,
    shared: Arc<Mutex<Status>>,
    events: Receiver<Event>,
    book: AudioBook,
    join: Option<JoinHandle<()>>,
    /// Seeks asked for; the worker does not publish a position while some are still queued.
    issued: Arc<AtomicU64>,
}

impl Player {
    /// Starts a paused player at `start_ms`. `root` is the folder that part paths are relative to.
    pub fn start(
        book: AudioBook,
        root: PathBuf,
        sink: Box<dyn AudioSink>,
        start_ms: u64,
        speed: f32,
    ) -> Player {
        let (tx, rx) = channel();
        let (etx, events) = channel();
        let issued = Arc::new(AtomicU64::new(0));
        let speed = clamp_speed(speed);
        let start = start_ms.min(book.total_ms);
        let shared = Arc::new(Mutex::new(Status {
            state: State::Paused,
            position_ms: start,
            speed,
            volume: 1.0,
            error: None,
        }));
        let paths: Vec<PathBuf> = book
            .parts
            .iter()
            .map(|p| {
                let q = PathBuf::from(&p.path);
                if q.is_absolute() {
                    q
                } else {
                    root.join(q)
                }
            })
            .collect();
        let mut w = Worker {
            book: book.clone(),
            paths,
            sink,
            rx,
            events: etx,
            shared: shared.clone(),
            issued: issued.clone(),
            applied: 0,
            decoder: None,
            stretcher: None,
            head_ms: start as f64,
            part: 0,
            state: State::Paused,
            speed,
            volume: 1.0,
            pause_at: None,
            src_frames: 0,
            open_offset_ms: 0,
            pending: Vec::new(),
        };
        let join = std::thread::Builder::new()
            .name("bookx-audio".into())
            .spawn(move || w.run())
            .ok();
        Player {
            tx,
            shared,
            events,
            book,
            join,
            issued,
        }
    }

    pub fn book(&self) -> &AudioBook {
        &self.book
    }

    pub fn play(&self) {
        let _ = self.tx.send(Cmd::Play);
    }

    pub fn pause(&self) {
        let _ = self.tx.send(Cmd::Pause);
    }

    pub fn seek(&self, ms: u64) {
        let ms = ms.min(self.book.total_ms);
        self.issued.fetch_add(1, Ordering::SeqCst);
        self.shared.lock().unwrap().position_ms = ms;
        let _ = self.tx.send(Cmd::Seek(ms));
    }

    /// Moves by `delta_ms` from the current position.
    pub fn skip(&self, delta_ms: i64) {
        let pos = self.status().position_ms;
        self.seek(self.book.skip(pos, delta_ms));
    }

    pub fn set_speed(&self, speed: f32) {
        let s = clamp_speed(speed);
        self.shared.lock().unwrap().speed = s;
        let _ = self.tx.send(Cmd::SetSpeed(s));
    }

    pub fn set_volume(&self, v: f32) {
        let v = if v.is_nan() { 1.0 } else { v.clamp(0.0, 1.0) };
        self.shared.lock().unwrap().volume = v;
        let _ = self.tx.send(Cmd::SetVolume(v));
    }

    /// Stops playback when the position reaches `ms` (the end of the chapter for the sleep timer).
    pub fn pause_at(&self, ms: Option<u64>) {
        let _ = self.tx.send(Cmd::PauseAt(ms));
    }

    pub fn status(&self) -> Status {
        self.shared.lock().unwrap().clone()
    }

    /// Events since the last call.
    pub fn poll_events(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Quit);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

struct Worker {
    book: AudioBook,
    paths: Vec<PathBuf>,
    sink: Box<dyn AudioSink>,
    rx: Receiver<Cmd>,
    events: Sender<Event>,
    shared: Arc<Mutex<Status>>,
    issued: Arc<AtomicU64>,
    applied: u64,
    decoder: Option<Decoder>,
    stretcher: Option<(Stretcher, u32, u8)>,
    /// Book time (ms) of the last source sample that went to the stretcher.
    head_ms: f64,
    part: usize,
    state: State,
    speed: f32,
    volume: f32,
    pause_at: Option<u64>,
    src_frames: u64,
    open_offset_ms: u64,
    pending: Vec<f32>,
}

impl Worker {
    fn run(&mut self) {
        loop {
            let cmd = if self.state == State::Playing {
                match self.rx.try_recv() {
                    Ok(c) => Some(c),
                    Err(std::sync::mpsc::TryRecvError::Empty) => None,
                    Err(_) => return,
                }
            } else {
                match self.rx.recv_timeout(Duration::from_millis(250)) {
                    Ok(c) => Some(c),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            };
            if let Some(c) = cmd {
                if !self.command(c) {
                    return;
                }
                continue;
            }
            if self.state == State::Playing {
                if let Err(e) = self.step() {
                    self.fail(format!("{e:#}"));
                }
            }
        }
    }

    /// Changes the published status. The position is left alone while a seek that the UI already
    /// showed is still waiting in the queue (a late publish would put the old place back).
    fn publish(&self, f: impl FnOnce(&mut Status)) {
        let mut g = self.shared.lock().unwrap();
        let before = g.position_ms;
        f(&mut g);
        if self.applied != self.issued.load(Ordering::SeqCst) {
            g.position_ms = before;
        }
    }

    fn heard_ms(&self) -> u64 {
        let lat = self.sink.latency().as_secs_f64() * 1000.0 * self.speed as f64;
        (self.head_ms - lat).max(0.0).min(self.book.total_ms as f64) as u64
    }

    fn fail(&mut self, msg: String) {
        self.sink.flush();
        self.decoder = None;
        self.state = State::Paused;
        self.head_ms = self.heard_ms() as f64;
        let pos = self.head_ms as u64;
        self.publish(|s| {
            s.state = State::Paused;
            s.position_ms = pos;
            s.error = Some(msg.clone());
        });
        let _ = self.events.send(Event::Error(msg));
    }

    fn command(&mut self, c: Cmd) -> bool {
        match c {
            Cmd::Quit => {
                self.sink.flush();
                return false;
            }
            Cmd::Play => {
                if self.state == State::Ended || self.head_ms as u64 >= self.book.total_ms {
                    self.head_ms = 0.0;
                    self.drop_stream();
                }
                self.state = State::Playing;
                self.publish(|s| {
                    s.state = State::Playing;
                    s.error = None;
                });
            }
            Cmd::Pause => self.pause_now(),
            Cmd::Seek(ms) => {
                self.applied += 1;
                self.sink.flush();
                self.drop_stream();
                self.head_ms = ms as f64;
                if self.state == State::Ended {
                    self.state = State::Paused;
                    self.publish(|s| s.state = State::Paused);
                }
                self.publish(|s| s.position_ms = ms);
            }
            Cmd::SetSpeed(s) => {
                self.speed = s;
                if let Some((st, _, _)) = self.stretcher.as_mut() {
                    st.set_speed(s);
                }
            }
            Cmd::SetVolume(v) => self.volume = v,
            Cmd::PauseAt(p) => self.pause_at = p,
        }
        true
    }

    fn pause_now(&mut self) {
        if self.state == State::Playing {
            let pos = self.heard_ms();
            self.sink.flush();
            self.drop_stream();
            self.head_ms = pos as f64;
            self.state = State::Paused;
            self.publish(|s| {
                s.state = State::Paused;
                s.position_ms = pos;
            });
        }
    }

    fn drop_stream(&mut self) {
        self.decoder = None;
        self.stretcher = None;
        self.pending.clear();
        self.src_frames = 0;
    }

    fn open_decoder(&mut self) -> Result<()> {
        let (part, offset) = self.book.locate(self.head_ms as u64);
        let path = self
            .paths
            .get(part)
            .ok_or_else(|| anyhow!("this audiobook has no parts"))?;
        if !self.book.parts[part].format.playable() {
            return Err(anyhow!(
                "Bookx cannot play {} files yet",
                self.book.parts[part].format.name()
            ));
        }
        let mut d = Decoder::open(path)?;
        if offset > 0 {
            // a failed seek still plays, from the start of the part
            if d.seek_ms(offset).is_err() {
                self.head_ms = self.book.part_start(part) as f64;
                self.open_offset_ms = 0;
            } else {
                self.open_offset_ms = offset;
            }
        } else {
            self.open_offset_ms = 0;
        }
        self.part = part;
        self.src_frames = 0;
        self.decoder = Some(d);
        Ok(())
    }

    fn step(&mut self) -> Result<()> {
        // the end of the book, or the sleep timer's stop
        if let Some(at) = self.pause_at {
            if self.head_ms as u64 >= at {
                self.drain_and_flush();
                self.pause_at = None;
                let pos = at.min(self.book.total_ms);
                self.state = State::Paused;
                self.head_ms = pos as f64;
                self.drop_stream();
                self.publish(|s| {
                    s.state = State::Paused;
                    s.position_ms = pos;
                });
                let _ = self.events.send(Event::PausedAt(pos));
                return Ok(());
            }
        }
        if self.decoder.is_none() {
            if self.head_ms as u64 >= self.book.total_ms {
                return self.finish_book();
            }
            self.open_decoder()?;
        }
        let mut raw = Vec::new();
        let more = self.decoder.as_mut().unwrap().next_chunk(&mut raw)?;
        if !more {
            self.decoder = None;
            let next = self.part + 1;
            if next >= self.book.parts.len() {
                self.head_ms = self.book.total_ms as f64;
                return self.finish_book();
            }
            self.head_ms = self.book.part_start(next) as f64;
            return Ok(());
        }
        let d = self.decoder.as_ref().unwrap();
        let (rate, ch) = (d.rate, d.channels);
        let mut frames = (raw.len() / ch as usize) as u64;
        // a sleep-timer stop inside this chunk: play up to it, not past it
        let old_head = self.head_ms;
        if let Some(at) = self.pause_at {
            let end = old_head + frames as f64 * 1000.0 / rate as f64;
            if (at as f64) < end {
                frames = (((at as f64 - old_head).max(0.0)) * rate as f64 / 1000.0) as u64;
                raw.truncate(frames as usize * ch as usize);
            }
        }
        self.src_frames += frames;
        self.head_ms = (self.book.part_start(self.part) + self.open_offset_ms) as f64
            + self.src_frames as f64 * 1000.0 / rate as f64;
        let need_new = !matches!(&self.stretcher, Some((_, r, c)) if *r == rate && *c == ch);
        if need_new {
            self.drain_stretcher();
            self.stretcher = Some((Stretcher::new(rate, ch, self.speed), rate, ch));
        }
        let (st, _, _) = self.stretcher.as_mut().unwrap();
        st.set_speed(self.speed);
        let mut out = std::mem::take(&mut self.pending);
        out.clear();
        st.push(&raw, &mut out);
        self.write_samples(rate, ch, &out)?;
        self.pending = out;
        let pos = self.heard_ms();
        self.publish(|s| s.position_ms = pos);
        Ok(())
    }

    fn write_samples(&mut self, rate: u32, ch: u8, samples: &[f32]) -> Result<()> {
        if samples.is_empty() {
            return Ok(());
        }
        let v = self.volume;
        let pcm: Vec<i16> = samples
            .iter()
            .map(|&x| (x * v * 32767.0).clamp(-32768.0, 32767.0) as i16)
            .collect();
        self.sink.write(rate, ch, &pcm)
    }

    fn drain_stretcher(&mut self) {
        if let Some((mut st, rate, ch)) = self.stretcher.take() {
            let mut out = Vec::new();
            st.finish(&mut out);
            let _ = self.write_samples(rate, ch, &out);
        }
    }

    /// Lets what the sink has buffered play out before it is silenced.
    fn drain_and_flush(&mut self) {
        self.drain_stretcher();
        let lat = self.sink.latency();
        if !lat.is_zero() {
            std::thread::sleep(lat);
        }
        self.sink.flush();
    }

    fn finish_book(&mut self) -> Result<()> {
        self.drain_and_flush();
        self.drop_stream();
        self.state = State::Ended;
        let total = self.book.total_ms;
        self.head_ms = total as f64;
        self.publish(|s| {
            s.state = State::Ended;
            s.position_ms = total;
        });
        let _ = self.events.send(Event::Ended);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::scan_folder;
    use crate::testgen::*;
    use std::path::Path;
    use std::time::Instant;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-audio-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Three parts of constant (DC) samples: 1000 for 1 s, 2000 for 2 s, 3000 for 1 s (8 kHz mono).
    fn dc_book() -> (AudioBook, PathBuf) {
        let d = tmp();
        for (i, (v, ms)) in [(1000i16, 1000u64), (2000, 2000), (3000, 1000)]
            .iter()
            .enumerate()
        {
            let n = 8 * *ms as usize;
            std::fs::write(
                d.join(format!("{}.wav", i + 1)),
                wav_samples(8000, 1, &vec![*v; n]),
            )
            .unwrap();
        }
        let s = scan_folder(&d).unwrap();
        (s.book, d)
    }

    fn wait(what: &str, mut f: impl FnMut() -> bool) {
        let t = Instant::now();
        while !f() {
            assert!(
                t.elapsed() < Duration::from_secs(20),
                "timed out waiting for {what}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn plays_all_parts_in_order_and_ends() {
        let (book, root) = dc_book();
        let sink = MemorySink::default();
        let written = sink.written.clone();
        let p = Player::start(book, root, Box::new(sink), 0, 1.0);
        assert_eq!(p.status().state, State::Paused);
        p.play();
        wait("the end", || p.status().state == State::Ended);
        let w = written.lock().unwrap();
        assert_eq!(w.len(), 32_000);
        assert!(w[..8000].iter().all(|&s| (s - 1000).abs() <= 1));
        assert!((w[8000] - 2000).abs() <= 1 && (w[31_999] - 3000).abs() <= 1);
        assert_eq!(p.status().position_ms, 4000);
        assert!(p.poll_events().contains(&Event::Ended));
    }

    #[test]
    fn starting_in_the_middle_of_a_later_part() {
        let (book, root) = dc_book();
        let sink = MemorySink::default();
        let written = sink.written.clone();
        let p = Player::start(book, root, Box::new(sink), 2500, 1.0);
        p.play();
        wait("the end", || p.status().state == State::Ended);
        let w = written.lock().unwrap();
        assert_eq!(w.len(), 12_000, "1.5 s of part 2 and 1 s of part 3");
        assert!((w[0] - 2000).abs() <= 1);
    }

    #[test]
    fn faster_playback_writes_fewer_samples() {
        let (book, root) = dc_book();
        let sink = MemorySink::default();
        let written = sink.written.clone();
        let p = Player::start(book, root, Box::new(sink), 0, 2.0);
        p.play();
        wait("the end", || p.status().state == State::Ended);
        let n = written.lock().unwrap().len();
        assert!((14_000..=19_500).contains(&n), "{n} samples at 2x of 32000");
    }

    #[test]
    fn pause_seek_and_resume() {
        let (book, root) = dc_book();
        let sink = MemorySink::with_delay(Duration::from_millis(2));
        let written = sink.written.clone();
        let flushes = sink.flushes.clone();
        let p = Player::start(book, root, Box::new(sink), 0, 1.0);
        p.play();
        wait("some progress", || p.status().position_ms > 300);
        p.pause();
        wait("paused", || p.status().state == State::Paused);
        let pos = p.status().position_ms;
        assert!(pos > 300 && pos < 4000);
        assert!(*flushes.lock().unwrap() >= 1);
        let n = written.lock().unwrap().len();
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(
            written.lock().unwrap().len(),
            n,
            "nothing is written while paused"
        );
        // seek into part 3 and play to the end
        p.seek(3200);
        assert_eq!(p.status().position_ms, 3200);
        p.play();
        wait("the end", || p.status().state == State::Ended);
        let w = written.lock().unwrap();
        assert!((w[w.len() - 1] - 3000).abs() <= 1);
        // after the end, play starts over
        drop(w);
        p.play();
        wait("playing again", || {
            p.status().position_ms < 3000 && p.status().state == State::Playing
        });
        p.pause();
    }

    #[test]
    fn a_seek_is_not_overwritten_by_the_position_of_a_write_that_was_under_way() {
        // regression: the worker published the position of the chunk it was writing after the UI had
        // already shown the new place, so a skip right after a seek started from the old place
        let (book, root) = dc_book();
        let p = Player::start(
            book,
            root,
            Box::new(MemorySink::with_delay(Duration::from_millis(120))),
            0,
            1.0,
        );
        p.play();
        wait("some progress", || p.status().position_ms > 100);
        p.seek(3000);
        p.seek(2800);
        let t = Instant::now();
        while t.elapsed() < Duration::from_millis(600) {
            let pos = p.status().position_ms;
            assert!(
                pos >= 2800,
                "the position fell back to {pos} ms after a seek to 2800"
            );
            std::thread::yield_now();
        }
        p.pause();
    }

    #[test]
    fn skip_moves_relative_to_the_position() {
        let (book, root) = dc_book();
        let p = Player::start(book, root, Box::new(NullSink { speedup: 0.0 }), 1000, 1.0);
        p.skip(-5000);
        assert_eq!(p.status().position_ms, 0);
        p.skip(1500);
        assert_eq!(p.status().position_ms, 1500);
        p.skip(1_000_000);
        assert_eq!(p.status().position_ms, 4000);
    }

    #[test]
    fn pause_at_stops_at_the_chapter_end() {
        let (book, root) = dc_book();
        let end = book.chapters[1].end_ms;
        assert_eq!(end, 3000);
        let sink = MemorySink::with_delay(Duration::from_millis(1));
        let p = Player::start(book, root, Box::new(sink), 1200, 1.0);
        p.pause_at(Some(end));
        p.play();
        wait("the sleep stop", || {
            p.status().state == State::Paused && p.status().position_ms >= 2900
        });
        assert_eq!(p.status().position_ms, 3000);
        assert!(p.poll_events().contains(&Event::PausedAt(3000)));
        // playing on goes to the end
        p.play();
        wait("the end", || p.status().state == State::Ended);
    }

    #[test]
    fn a_missing_part_pauses_with_an_error() {
        let (book, root) = dc_book();
        std::fs::remove_file(root.join("2.wav")).unwrap();
        let p = Player::start(book, root, Box::new(MemorySink::default()), 1500, 1.0);
        p.play();
        wait("the error", || p.status().error.is_some());
        assert_eq!(p.status().state, State::Paused);
        assert!(p.poll_events().iter().any(|e| matches!(e, Event::Error(_))));
    }

    #[test]
    fn opus_is_reported_not_played() {
        let d = tmp();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tiny.opus"),
            d.join("a.opus"),
        )
        .unwrap();
        let s = scan_folder(&d).unwrap();
        let p = Player::start(s.book, d, Box::new(MemorySink::default()), 0, 1.0);
        p.play();
        wait("the error", || p.status().error.is_some());
    }

    #[test]
    fn speed_and_volume_are_clamped_and_volume_scales_samples() {
        let (book, root) = dc_book();
        let sink = MemorySink::default();
        let written = sink.written.clone();
        let p = Player::start(book, root, Box::new(sink), 0, 99.0);
        assert_eq!(p.status().speed, 3.0);
        p.set_speed(0.0);
        assert_eq!(p.status().speed, 0.5);
        p.set_speed(1.0);
        p.set_volume(0.5);
        p.set_volume(f32::NAN);
        p.set_volume(0.5);
        p.play();
        wait("the end", || p.status().state == State::Ended);
        let w = written.lock().unwrap();
        assert!((w[100] - 500).abs() <= 1, "{}", w[100]);
    }

    #[test]
    fn real_files_play_through_the_memory_sink() {
        let d = tmp();
        let fx = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        std::fs::copy(fx.join("tiny.mp3"), d.join("1.mp3")).unwrap();
        std::fs::copy(fx.join("tiny.m4b"), d.join("2.m4b")).unwrap();
        std::fs::copy(fx.join("tiny.ogg"), d.join("3.ogg")).unwrap();
        std::fs::copy(fx.join("tiny.flac"), d.join("4.flac")).unwrap();
        let s = scan_folder(&d).unwrap();
        let total = s.book.total_ms;
        let sink = MemorySink::default();
        let written = sink.written.clone();
        let p = Player::start(s.book, d, Box::new(sink), 0, 1.0);
        p.play();
        wait("the end", || p.status().state == State::Ended);
        let n = written.lock().unwrap().len() as u64;
        // all four are 8 kHz mono; the gapless trimming may remove a few ms
        let ms = n * 1000 / 8000;
        assert!(
            ms.abs_diff(total) < 300,
            "{ms} ms written for a {total} ms book"
        );
    }
}
