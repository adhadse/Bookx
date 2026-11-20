//! Audiobooks in the library (roadmap PP-G-25): importing a folder, a zip or a file of DRM-free
//! audio as a catalogue entry, loading it for playback, and the listening session that ties the
//! player engine ([`bookx_audio`]) to the saved position, the sleep timer, the bookmarks and the
//! listening statistics. No GTK in here, so it is unit tested; the page is `ui/audiobook_player.rs`.
//!
//! An audiobook is a book of the catalogue whose file is a small generated EPUB (title, narrator,
//! chapter list: what the e-book machinery needs), with an `audiobook` resource that points at
//! the audio (a file or a folder, where it already is) and a format entry for `format:m4b`
//! searches. The scan of the parts is cached next to the settings, so opening a book of 500
//! chapters does not scan it again.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use bookx_audio::model::AudioBook;
use bookx_audio::player::{AudioSink, Event, Player, State as PlayState};
use bookx_audio::scan::{self, Scanned};
use bookx_audio::state::State;
use bookx_lookup::stats::{ActiveSession, StatsConfig, StatsStore};
use bookx_tts::SleepTimer;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use crate::core::formats::{FormatFile, Resource, AUDIOBOOK};
use crate::core::library::{Book, Library, Prepared, ReadPos};
use crate::core::opf::{Identifier, Metadata};

/// The identifier scheme that marks an audiobook made by an import (its file is the generated page).
pub const MARKER: &str = "bookx-audiobook";

/// Skip lengths of the buttons.
pub const SKIP_BACK_MS: i64 = -15_000;
pub const SKIP_FORWARD_MS: i64 = 30_000;

/// Where the cache, the state of every book and the listening statistics live.
pub fn data_dir() -> Option<PathBuf> {
    crate::config::stats_path().and_then(|p| p.parent().map(|d| d.join("audiobooks")))
}

pub fn listening_stats_path(data: &Path) -> PathBuf {
    data.join("listening.json")
}

/// A short stable name for a path (cache and state file names).
pub fn key_of(path: &Path) -> String {
    let h = Sha256::digest(path.to_string_lossy().as_bytes());
    h.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// Can this path be imported as an audiobook: an audio file, a zip with audio, a folder with audio?
pub fn is_audio_source(p: &Path) -> bool {
    scan::looks_like_audiobook(p)
}

pub fn is_audiobook(b: &Book) -> bool {
    b.resources.iter().any(|r| r.kind == AUDIOBOOK)
}

/// Made by an import (as opposed to an e-book that has an audiobook linked to it).
pub fn is_own(b: &Book) -> bool {
    b.meta.identifier(MARKER).is_some()
}

/// The audio of a book, as a path.
pub fn audio_path(lib_root: &Path, b: &Book) -> Option<PathBuf> {
    let r = b.resources.iter().find(|r| r.kind == AUDIOBOOK)?;
    let p = Path::new(&r.path);
    Some(if p.is_absolute() {
        p.to_path_buf()
    } else {
        lib_root.join(p)
    })
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn describe_html(a: &AudioBook) -> String {
    let mut h = format!("<h1>{}</h1>\n", esc(&a.title));
    if !a.authors.is_empty() {
        h.push_str(&format!("<p>{}</p>\n", esc(&a.authors.join(" & "))));
    }
    if !a.narrator.is_empty() {
        h.push_str(&format!("<p>Narrated by {}</p>\n", esc(&a.narrator)));
    }
    let (hh, mm) = bookx_audio::model::hours_minutes(a.total_ms);
    h.push_str(&format!(
        "<p>{hh} h {mm} min, {} chapters</p>\n",
        a.chapters.len()
    ));
    h.push_str("<p>This is an audiobook. Open it in Bookx to listen.</p>\n<ol>\n");
    for c in a.chapters.iter().take(500) {
        h.push_str(&format!("<li>{}</li>\n", esc(&c.title)));
    }
    h.push_str("</ol>\n");
    h
}

/// The catalogue metadata of a scanned audiobook.
pub fn metadata_of(a: &AudioBook) -> Metadata {
    let authors = if a.authors.is_empty() {
        vec!["Unknown".to_string()]
    } else {
        a.authors.clone()
    };
    let mut m = Metadata {
        title: a.title.clone(),
        authors: authors.clone(),
        author_sort: authors.join(" & "),
        language: a.language.clone(),
        description: a.narrator_line(),
        tags: vec!["Audiobook".into()],
        series: a.series.clone(),
        series_index: a.series_index.unwrap_or(0.0),
        ..Default::default()
    };
    m.identifiers.push(Identifier {
        scheme: MARKER.into(),
        value: "1".into(),
        xml_id: None,
    });
    m
}

trait NarratorLine {
    fn narrator_line(&self) -> String;
}

impl NarratorLine for AudioBook {
    /// The description the catalogue shows: the comment of the tags, else the narrator.
    fn narrator_line(&self) -> String {
        if !self.description.trim().is_empty() {
            self.description.trim().to_string()
        } else if !self.narrator.is_empty() {
            format!("Narrated by {}", self.narrator)
        } else {
            String::new()
        }
    }
}

/// Reads an audio file, folder or zip into something the library can add. A zip is unpacked
/// under `data/unpacked`. Safe to call from a worker thread.
pub fn prepare(path: &Path, data: &Path) -> Result<Prepared> {
    let (scanned, source): (Scanned, PathBuf) = if path.is_dir() {
        let src = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        (scan::scan_folder(&src)?, src)
    } else if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
    {
        let stem = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let dest = data.join("unpacked").join(format!(
            "{}-{}",
            bookx_audio::text::title_from_file_name(&stem).replace('/', "_"),
            &uuid::Uuid::new_v4().to_string()[..8]
        ));
        let s = scan::scan_zip(path, &dest).inspect_err(|_| {
            let _ = std::fs::remove_dir_all(&dest);
        })?;
        (s, dest)
    } else {
        let src = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        (scan::scan_file(&src)?, src)
    };
    save_cache(data, &source, &scanned.book);
    let a = &scanned.book;
    let meta = metadata_of(a);
    let chapter = crate::core::convert::Chapter {
        title: a.title.clone(),
        body: describe_html(a),
    };
    let cover = scanned
        .cover
        .as_ref()
        .map(|c| (c.data.clone(), c.ext.as_str()));
    let archive = crate::core::convert::create_epub(&meta, &[chapter], cover);
    let bytes = archive.to_bytes()?;
    Ok(Prepared {
        meta,
        cover: scanned.cover.map(|c| (c.data, c.ext)),
        bytes,
        origin: None,
        drm: String::new(),
        source_file: Some(source),
    })
}

fn cache_file(data: &Path, source: &Path) -> PathBuf {
    data.join(format!("{}.json", key_of(source)))
}

fn state_file(data: &Path, source: &Path) -> PathBuf {
    data.join(format!("{}.state.json", key_of(source)))
}

fn save_cache(data: &Path, source: &Path, book: &AudioBook) {
    let _ = std::fs::create_dir_all(data);
    let _ = scan::save_manifest(book, &cache_file(data, source));
}

/// After the catalogue has the book: links the audio and lists its format.
pub fn after_import(lib: &mut Library, id: u64, source: &Path, data: &Path) -> Result<()> {
    let book = load_book_at(source, data)?.0;
    let size: u64 = book
        .parts
        .iter()
        .filter_map(|p| std::fs::metadata(base_of(source).join(&p.path)).ok())
        .map(|m| m.len())
        .sum();
    let first = book.parts.first().context("an audiobook without parts")?;
    let first_path = base_of(source).join(&first.path);
    let format = first.format.name().to_string();
    let label = book.title.clone();
    lib.set_resources(
        id,
        vec![Resource {
            kind: AUDIOBOOK.into(),
            path: source.to_string_lossy().into_owned(),
            label,
        }],
        format!("Linked the audio of \"{}\"", book.title),
    )?;
    if let Some(b) = lib.get_mut(id) {
        b.formats.push(FormatFile {
            file: first_path.to_string_lossy().into_owned(),
            format,
            size,
        });
    }
    lib.save()
}

/// The folder the part paths of a source are relative to.
fn base_of(source: &Path) -> PathBuf {
    if source.is_dir() {
        source.to_path_buf()
    } else {
        source.parent().unwrap_or(Path::new(".")).to_path_buf()
    }
}

/// Adds an audio file, folder or zip to the library at once (the importer of the dev hooks and
/// tests; the window imports on a worker thread through the generic importer).
pub fn import_now(lib: &mut Library, path: &Path, data: &Path) -> Result<u64> {
    use crate::core::library::AddOutcome;
    let p = prepare(path, data)?;
    let source = p.source_file.clone().context("no source")?;
    match lib.commit_import(p, false)? {
        AddOutcome::Added(id) => {
            after_import(lib, id, &source, data)?;
            Ok(id)
        }
        AddOutcome::Duplicate(id) => Ok(id),
    }
}

/// What is needed to play a book.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub book: AudioBook,
    /// Part paths are relative to this folder.
    pub root: PathBuf,
    /// The audio file or folder.
    pub source: PathBuf,
}

fn load_book_at(source: &Path, data: &Path) -> Result<(AudioBook, PathBuf)> {
    let root = base_of(source);
    if let Some(cached) = scan::load_manifest(&cache_file(data, source)) {
        if !cached.parts.is_empty() && cached.parts.iter().all(|p| root.join(&p.path).is_file()) {
            return Ok((cached, root));
        }
    }
    let s = if source.is_dir() {
        scan::scan_folder(source)?
    } else {
        scan::scan_file(source)?
    };
    save_cache(data, source, &s.book);
    Ok((s.book, root))
}

/// Opens the audio of `b` for playback.
pub fn load(lib_root: &Path, b: &Book, data: &Path) -> Result<Loaded> {
    let source = audio_path(lib_root, b).context("this book has no audio linked")?;
    if !source.exists() {
        anyhow::bail!("the audio is not there any more: {}", source.display());
    }
    let (book, root) = load_book_at(&source, data)?;
    Ok(Loaded { book, root, source })
}

/// What the catalogue should show after the listener moved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    pub position_ms: u64,
    pub chapter: usize,
    pub frac: f64,
    pub fraction: f64,
    pub finished: bool,
}

impl Progress {
    pub fn read_pos(&self) -> ReadPos {
        ReadPos {
            spine: self.chapter,
            frac: self.frac,
        }
    }
}

/// What the sleep timer is set to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sleep {
    Off,
    Minutes(u32),
    EndOfChapter,
}

/// Everything about one listening session of one book.
pub struct Listening {
    pub player: Player,
    pub book: AudioBook,
    pub state: State,
    pub book_id: u64,
    pub sleep: SleepTimer,
    state_path: PathBuf,
    stats_path: PathBuf,
    stats: StatsStore,
    session: Option<ActiveSession>,
    armed: Option<u64>,
    last_persist: Instant,
    last_playing: bool,
    /// The sleep timer stopped playback since the last [`Listening::take_slept`].
    slept: bool,
    /// The listener did something that changes the place (play, seek, skip): until then the
    /// saved place is left as it is, so opening a book and closing it again keeps it.
    moved: bool,
    on_progress: Box<dyn FnMut(Progress)>,
}

/// Persist the position this often while playing.
const PERSIST_EVERY_SECS: u64 = 5;

impl Listening {
    pub fn open(
        loaded: Loaded,
        book_id: u64,
        data: &Path,
        sink: Box<dyn AudioSink>,
        on_progress: Box<dyn FnMut(Progress)>,
        catalogue_pos: Option<ReadPos>,
    ) -> Listening {
        let state_path = state_file(data, &loaded.source);
        let mut state = State::load(&state_path);
        let a = loaded.book.clone();
        // the catalogue's position wins when the saved state is older than it never is: use the
        // state, and the catalogue's chapter position only when there is no state
        if state.updated == 0 && state.position_ms == 0 {
            if let Some(p) = catalogue_pos {
                state.position_ms = a.from_readpos(p.spine, p.frac);
            }
        }
        let start = a.resume_position(state.position_ms);
        let stats_path = listening_stats_path(data);
        let stats = StatsStore::load(&stats_path)
            .or_else(|_| StatsStore::new(StatsConfig::default()))
            .unwrap_or_default();
        let player = Player::start(loaded.book, loaded.root, sink, start, state.speed);
        Listening {
            player,
            book: a,
            state,
            book_id,
            sleep: SleepTimer::Off,
            state_path,
            stats_path,
            stats,
            session: None,
            armed: None,
            last_persist: Instant::now(),
            last_playing: false,
            slept: false,
            moved: false,
            on_progress,
        }
    }

    pub fn position_ms(&self) -> u64 {
        self.player.status().position_ms
    }

    pub fn is_playing(&self) -> bool {
        self.player.status().state == PlayState::Playing
    }

    pub fn play_pause(&mut self) {
        self.moved = true;
        if self.is_playing() {
            self.player.pause();
        } else {
            self.player.play();
        }
    }

    pub fn skip(&mut self, delta_ms: i64) {
        self.moved = true;
        self.player.skip(delta_ms);
    }

    pub fn seek(&mut self, ms: u64) {
        self.moved = true;
        self.player.seek(ms.min(self.book.total_ms));
    }

    pub fn next_chapter(&mut self) {
        if let Some(p) = self.book.next_chapter(self.position_ms()) {
            self.moved = true;
            self.player.seek(p);
        }
    }

    pub fn previous_chapter(&mut self) {
        let p = self.book.previous_chapter(self.position_ms());
        self.moved = true;
        self.player.seek(p);
    }

    pub fn goto_chapter(&mut self, idx: usize) {
        self.moved = true;
        self.player.seek(self.book.chapter_start(idx));
    }

    pub fn set_speed(&mut self, s: f32) {
        self.player.set_speed(s);
        self.state.speed = self.player.status().speed;
    }

    /// Sets the sleep timer. `now` is the clock the timer is measured on.
    pub fn set_sleep(&mut self, s: Sleep, now: Instant) {
        self.armed = None;
        self.player.pause_at(None);
        self.sleep = match s {
            Sleep::Off => SleepTimer::Off,
            Sleep::EndOfChapter => SleepTimer::EndOfChapter,
            Sleep::Minutes(m) => {
                if m == 0 {
                    SleepTimer::Off
                } else {
                    SleepTimer::At(now + std::time::Duration::from_secs(m as u64 * 60))
                }
            }
        };
    }

    pub fn add_bookmark(&mut self, note: &str, wall: DateTime<Utc>) -> Option<String> {
        let id = self
            .state
            .add_bookmark(self.position_ms(), note, wall.timestamp());
        self.save_state(wall);
        id
    }

    pub fn remove_bookmark(&mut self, id: &str, wall: DateTime<Utc>) -> bool {
        let ok = self.state.remove_bookmark(id);
        self.save_state(wall);
        ok
    }

    pub fn go_to_bookmark(&mut self, id: &str) -> bool {
        match self
            .state
            .bookmarks
            .iter()
            .find(|b| b.id == id)
            .map(|b| b.position_ms)
        {
            Some(p) => {
                self.seek(p);
                true
            }
            None => false,
        }
    }

    /// The sleep timer stopped the book since the last call.
    pub fn take_slept(&mut self) -> bool {
        std::mem::take(&mut self.slept)
    }

    fn save_state(&mut self, wall: DateTime<Utc>) {
        if self.moved || self.is_playing() {
            self.state.position_ms = self.position_ms();
        }
        self.state.updated = wall.timestamp();
        let _ = self.state.save(&self.state_path);
    }

    fn progress(&self, finished: bool) -> Progress {
        let pos = self.position_ms();
        let (chapter, frac) = self.book.to_readpos(pos);
        Progress {
            position_ms: pos,
            chapter,
            frac,
            fraction: if finished {
                1.0
            } else {
                self.book.progress(pos)
            },
            finished,
        }
    }

    fn end_session(&mut self, wall: DateTime<Utc>) {
        if let Some(s) = self.session.take() {
            let frac = self.book.progress(self.position_ms());
            let session = s.finish(wall, frac, 0, 0, 0);
            let _ = self.stats.record(session);
            let _ = self.stats.save(&self.stats_path);
        }
    }

    /// Called often (a few times a second) from the UI's timer: reacts to the player's events,
    /// the sleep timer, and saves the position now and then. `now` is a monotonic clock for the
    /// sleep timer, `wall` the time for statistics.
    pub fn tick(&mut self, now: Instant, wall: DateTime<Utc>) {
        let st = self.player.status();
        let playing = st.state == PlayState::Playing;
        for e in self.player.poll_events() {
            match e {
                Event::Ended => {
                    self.state.finished = true;
                    self.end_session(wall);
                    self.state.position_ms = 0;
                    self.state.updated = wall.timestamp();
                    let _ = self.state.save(&self.state_path);
                    let mut p = self.progress(true);
                    p.position_ms = self.book.total_ms;
                    (self.on_progress)(p);
                    self.stats
                        .mark_finished(&self.book_id.to_string(), wall.date_naive());
                    let _ = self.stats.save(&self.stats_path);
                    self.last_playing = false;
                    return;
                }
                Event::PausedAt(_) => {
                    if matches!(self.sleep, SleepTimer::EndOfChapter) {
                        self.sleep = SleepTimer::Off;
                        self.slept = true;
                    }
                    self.armed = None;
                }
                Event::Error(_) => {}
            }
        }
        // sleep timer
        if playing && self.sleep.expired(now) {
            self.player.pause();
            self.sleep = SleepTimer::Off;
            self.slept = true;
        }
        if self.sleep.stops_at_chapter_end() && playing {
            let end = self
                .book
                .chapter_at(st.position_ms)
                .map(|c| c.end_ms)
                .unwrap_or(self.book.total_ms);
            if self.armed != Some(end) {
                self.player.pause_at(Some(end));
                self.armed = Some(end);
            }
        }
        // statistics follow what is audible
        match (self.last_playing, playing) {
            (false, true) => {
                self.session = Some(ActiveSession::begin(
                    &self.book_id.to_string(),
                    wall,
                    self.book.progress(st.position_ms),
                ));
            }
            (true, false) => {
                self.end_session(wall);
                self.save_state(wall);
                let p = self.progress(false);
                (self.on_progress)(p);
            }
            _ => {}
        }
        self.last_playing = playing;
        if playing && self.last_persist.elapsed().as_secs() >= PERSIST_EVERY_SECS {
            self.last_persist = Instant::now();
            self.save_state(wall);
            let p = self.progress(false);
            (self.on_progress)(p);
        }
    }

    /// Stops, and saves everything.
    pub fn close(&mut self, wall: DateTime<Utc>) {
        self.player.pause();
        // let the engine publish the paused position
        for _ in 0..50 {
            if self.player.status().state != PlayState::Playing {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        self.end_session(wall);
        self.save_state(wall);
        if self.moved {
            let finished = self.state.finished && self.position_ms() + 1000 >= self.book.total_ms;
            let p = self.progress(finished);
            (self.on_progress)(p);
        }
    }
}

/// The listening statistics as the statistics window shows them, `None` before the first session.
pub fn listening_summary(data: &Path, now: DateTime<Utc>) -> Option<crate::reading_stats::Summary> {
    let store = StatsStore::load(&listening_stats_path(data)).ok()?;
    if store.sessions().is_empty() && store.finished().is_empty() {
        return None;
    }
    Some(crate::reading_stats::summarize(&store, now, &chrono::Local))
}

/// The text of the progress line under a card or in the details: "42 % · 3 h 12 min left".
pub fn progress_text(book: &AudioBook, pos_ms: u64, speed: f32) -> String {
    use crate::i18n::trf;
    let pct = (book.progress(pos_ms) * 100.0).round() as u32;
    let (h, m) = bookx_audio::model::hours_minutes(book.time_left_ms(pos_ms, speed));
    if h > 0 {
        trf("{} % · {} h {} min left", &[&pct, &h, &m])
    } else {
        trf("{} % · {} min left", &[&pct, &m])
    }
}

/// `3 h 12 min` for the length of a book.
pub fn length_text(total_ms: u64) -> String {
    use crate::i18n::trf;
    let (h, m) = bookx_audio::model::hours_minutes(total_ms);
    if h > 0 {
        trf("{} h {} min", &[&h, &m])
    } else {
        trf("{} min", &[&m])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_audio::player::{MemorySink, NullSink};
    use bookx_audio::testgen::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-audiobook-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn folder(secs: &[u64]) -> PathBuf {
        let d = tmp().join("My Audio Tale");
        std::fs::create_dir_all(&d).unwrap();
        for (i, s) in secs.iter().enumerate() {
            std::fs::write(
                d.join(format!("{:02} - Part {}.wav", i + 1, i + 1)),
                wav_silence(8000, s * 1000),
            )
            .unwrap();
        }
        d
    }

    fn lib() -> (Library, PathBuf) {
        let root = tmp().join("lib");
        std::fs::create_dir_all(&root).unwrap();
        (Library::in_memory(&root).unwrap(), root)
    }

    fn wait(mut f: impl FnMut() -> bool) {
        let t = Instant::now();
        while !f() {
            assert!(t.elapsed() < Duration::from_secs(20), "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_folder_becomes_a_catalogue_entry_with_audio_and_a_format() {
        let (mut l, _) = lib();
        let data = tmp();
        let src = folder(&[2, 3]);
        let p = prepare(&src, &data).unwrap();
        assert_eq!(p.meta.title, "My Audio Tale");
        assert_eq!(p.meta.tags, vec!["Audiobook"]);
        assert!(p.bytes.len() > 100);
        let source = p.source_file.clone().unwrap();
        let id = match l.commit_import(p, false).unwrap() {
            crate::core::library::AddOutcome::Added(id) => id,
            _ => panic!("duplicate"),
        };
        after_import(&mut l, id, &source, &data).unwrap();
        let b = l.get(id).unwrap();
        assert!(is_audiobook(b) && is_own(b));
        assert!(b.all_formats().iter().any(|f| f.format == "wav"));
        assert!(crate::core::query::Query::parse("format:wav").matches(b));
        assert!(crate::core::query::Query::parse("audiobook:yes").matches(b));
        // the generated page opens as a book (everything that expects an EPUB keeps working)
        assert!(l.open_book(id).is_ok());
        let loaded = load(&l.root, b, &data).unwrap();
        assert_eq!(loaded.book.total_ms, 5000);
        assert_eq!(loaded.book.chapters.len(), 2);
    }

    #[test]
    fn an_ebook_with_a_linked_audiobook_is_not_own() {
        let mut b = crate::core::library::Book {
            id: 1,
            file: "a.epub".into(),
            cover: None,
            meta: Metadata::default(),
            added: Utc::now(),
            modified: Utc::now(),
            last_read: None,
            size: 1,
            progress: 0.0,
            position: None,
            bookmarks: vec![],
            highlights: vec![],
            formats: vec![],
            resources: vec![Resource {
                kind: AUDIOBOOK.into(),
                path: "/x/y.m4b".into(),
                label: String::new(),
            }],
            drm: String::new(),
        };
        assert!(is_audiobook(&b) && !is_own(&b));
        assert_eq!(
            audio_path(Path::new("/lib"), &b),
            Some(PathBuf::from("/x/y.m4b"))
        );
        b.resources[0].path = "rel/dir".into();
        assert_eq!(
            audio_path(Path::new("/lib"), &b),
            Some(PathBuf::from("/lib/rel/dir"))
        );
        b.resources.clear();
        assert!(audio_path(Path::new("/lib"), &b).is_none());
    }

    #[test]
    fn a_zip_is_unpacked_into_the_data_folder() {
        let data = tmp();
        let z = tmp().join("book.zip");
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            use std::io::Write;
            let mut w = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            w.start_file("a/01.wav", o).unwrap();
            w.write_all(&wav_silence(8000, 1000)).unwrap();
            w.start_file("a/02.wav", o).unwrap();
            w.write_all(&wav_silence(8000, 1000)).unwrap();
            w.finish().unwrap();
        }
        std::fs::write(&z, buf.into_inner()).unwrap();
        assert!(is_audio_source(&z));
        let p = prepare(&z, &data).unwrap();
        let src = p.source_file.unwrap();
        assert!(src.starts_with(data.join("unpacked")), "{src:?}");
        assert_eq!(p.meta.title, "book");
    }

    #[test]
    fn a_missing_or_empty_source_is_an_error_not_a_panic() {
        let data = tmp();
        assert!(prepare(&tmp().join("none"), &data).is_err());
        assert!(prepare(&tmp(), &data).is_err());
        assert!(!is_audio_source(&tmp()));
    }

    #[test]
    fn the_scan_is_cached_and_a_missing_part_triggers_a_rescan() {
        let data = tmp();
        let src = folder(&[1, 1]);
        prepare(&src, &data).unwrap();
        let (b1, root) = load_book_at(&src, &data).unwrap();
        assert_eq!(b1.parts.len(), 2);
        assert!(cache_file(&data, &src.canonicalize().unwrap()).exists());
        std::fs::remove_file(root.join(&b1.parts[1].path)).unwrap();
        let (b2, _) = load_book_at(&src, &data).unwrap();
        assert_eq!(b2.parts.len(), 1, "the cache did not hide the missing part");
    }

    fn listening(
        src: &Path,
        data: &Path,
        sink: Box<dyn AudioSink>,
    ) -> (Listening, Rc<RefCell<Vec<Progress>>>) {
        let (b, root) = load_book_at(src, data).unwrap();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let s2 = seen.clone();
        let l = Listening::open(
            Loaded {
                book: b,
                root,
                source: src.to_path_buf(),
            },
            7,
            data,
            sink,
            Box::new(move |p| s2.borrow_mut().push(p)),
            None,
        );
        (l, seen)
    }

    #[test]
    fn position_speed_and_bookmarks_survive_a_restart() {
        let data = tmp();
        let src = folder(&[10, 10]);
        let (mut l, seen) = listening(&src, &data, Box::new(NullSink { speedup: 0.0 }));
        l.seek(12_345);
        l.set_speed(1.5);
        let bm = l.add_bookmark("the good part", Utc::now()).unwrap();
        l.close(Utc::now());
        let last = *seen.borrow().last().unwrap();
        assert_eq!(last.position_ms, 12_345);
        assert_eq!(last.chapter, 1);
        assert!((last.frac - 0.2345).abs() < 1e-3);
        assert!(!last.finished);
        drop(l);
        let (l2, _) = listening(&src, &data, Box::new(NullSink { speedup: 0.0 }));
        assert_eq!(l2.state.speed, 1.5);
        assert_eq!(
            l2.position_ms(),
            10_345,
            "resumes two seconds before where it stopped"
        );
        assert_eq!(l2.state.bookmarks.len(), 1);
        assert_eq!(l2.state.bookmarks[0].id, bm);
        assert_eq!(l2.state.bookmarks[0].note, "the good part");
    }

    #[test]
    fn opening_and_closing_again_keeps_the_saved_place() {
        let data = tmp();
        let src = folder(&[10, 10]);
        let (mut l, _) = listening(&src, &data, Box::new(NullSink { speedup: 0.0 }));
        l.seek(15_000);
        l.close(Utc::now());
        drop(l);
        // reopened (two seconds before) and closed at once: the place is still 15 s, not 13 s
        let (mut l2, seen) = listening(&src, &data, Box::new(NullSink { speedup: 0.0 }));
        assert_eq!(l2.position_ms(), 13_000);
        l2.close(Utc::now());
        assert!(
            seen.borrow().is_empty(),
            "the catalogue was not told of a move that did not happen"
        );
        drop(l2);
        let (l3, _) = listening(&src, &data, Box::new(NullSink { speedup: 0.0 }));
        assert_eq!(l3.state.position_ms, 15_000);
    }

    #[test]
    fn the_catalogue_position_is_used_when_there_is_no_state() {
        let data = tmp();
        let src = folder(&[10, 10]);
        let (b, root) = load_book_at(&src, &data).unwrap();
        let l = Listening::open(
            Loaded {
                book: b,
                root,
                source: src,
            },
            1,
            &data,
            Box::new(NullSink { speedup: 0.0 }),
            Box::new(|_| {}),
            Some(ReadPos {
                spine: 1,
                frac: 0.5,
            }),
        );
        assert_eq!(l.position_ms(), 13_000);
    }

    #[test]
    fn the_sleep_timer_pauses_after_the_time() {
        let data = tmp();
        let src = folder(&[30, 30]);
        let (mut l, _) = listening(
            &src,
            &data,
            Box::new(MemorySink::with_delay(Duration::from_millis(8))),
        );
        let t0 = Instant::now();
        l.set_sleep(Sleep::Minutes(10), t0);
        l.player.play();
        wait(|| l.is_playing());
        l.tick(t0 + Duration::from_secs(60), Utc::now());
        assert!(l.is_playing(), "10 minutes have not passed");
        l.tick(t0 + Duration::from_secs(601), Utc::now());
        wait(|| !l.is_playing());
        assert!(l.take_slept());
        assert!(!l.take_slept());
        assert_eq!(l.sleep, SleepTimer::Off);
        l.close(Utc::now());
    }

    #[test]
    fn end_of_chapter_stops_exactly_there() {
        let data = tmp();
        let src = folder(&[4, 4]);
        let (mut l, _) = listening(
            &src,
            &data,
            Box::new(MemorySink::with_delay(Duration::from_millis(20))),
        );
        l.seek(1000);
        l.set_sleep(Sleep::EndOfChapter, Instant::now());
        l.player.play();
        let t = Instant::now();
        while !l.take_slept() {
            l.tick(Instant::now(), Utc::now());
            assert!(t.elapsed() < Duration::from_secs(20), "never stopped");
            std::thread::sleep(Duration::from_millis(5));
        }
        wait(|| !l.is_playing());
        assert_eq!(l.position_ms(), 4000);
        assert_eq!(l.sleep, SleepTimer::Off);
        l.close(Utc::now());
    }

    #[test]
    fn listening_time_goes_into_the_statistics_and_finishing_is_recorded() {
        let data = tmp();
        let src = folder(&[7]);
        let (mut l, seen) = listening(&src, &data, Box::new(NullSink { speedup: 0.0 }));
        let w0 = Utc::now();
        l.player.play();
        // the null sink is instant: the book ends at once
        let t = Instant::now();
        loop {
            l.tick(
                Instant::now(),
                w0 + chrono::Duration::seconds(t.elapsed().as_millis() as i64),
            );
            if seen.borrow().last().is_some_and(|p| p.finished) {
                break;
            }
            assert!(t.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(5));
        }
        let p = *seen.borrow().last().unwrap();
        assert_eq!((p.fraction, p.finished), (1.0, true));
        assert!(l.state.finished);
        let store = StatsStore::load(&listening_stats_path(&data)).unwrap();
        assert_eq!(store.finished().len(), 1);
        // a session of 30 s is recorded when playing is paused later: own data folder (the run above may
        // have recorded a session of its own) and a book that cannot end before the pause on a slow machine
        let data = tmp();
        let src = folder(&[120]);
        let (mut l2, _) = listening(
            &src,
            &data,
            Box::new(MemorySink::with_delay(Duration::from_millis(5))),
        );
        l2.player.play();
        let w1 = w0 + chrono::Duration::hours(1);
        wait(|| l2.is_playing());
        l2.tick(Instant::now(), w1);
        l2.player.pause();
        wait(|| !l2.is_playing());
        l2.tick(Instant::now(), w1 + chrono::Duration::seconds(30));
        let store = StatsStore::load(&listening_stats_path(&data)).unwrap();
        assert_eq!(store.sessions().len(), 1);
        assert_eq!(store.sessions()[0].duration_secs(), 30);
        l2.close(Utc::now());
    }

    #[test]
    fn chapter_buttons_and_bookmark_jumps() {
        let data = tmp();
        let src = folder(&[10, 10, 10]);
        let (mut l, _) = listening(&src, &data, Box::new(NullSink { speedup: 0.0 }));
        l.goto_chapter(2);
        assert_eq!(l.position_ms(), 20_000);
        l.previous_chapter();
        assert_eq!(
            l.position_ms(),
            10_000,
            "within the first seconds: the chapter before"
        );
        l.seek(15_000);
        l.previous_chapter();
        assert_eq!(l.position_ms(), 10_000, "later in a chapter: its start");
        l.next_chapter();
        assert_eq!(l.position_ms(), 20_000);
        l.next_chapter();
        assert_eq!(l.position_ms(), 20_000, "no chapter after the last");
        l.skip(SKIP_FORWARD_MS);
        assert_eq!(l.position_ms(), 30_000);
        l.skip(SKIP_BACK_MS);
        assert_eq!(l.position_ms(), 15_000);
        l.seek(5000);
        let id = l.add_bookmark("x", Utc::now()).unwrap();
        l.seek(0);
        assert!(l.go_to_bookmark(&id));
        assert_eq!(l.position_ms(), 5000);
        assert!(!l.go_to_bookmark("nope"));
        assert!(l.remove_bookmark(&id, Utc::now()));
        l.close(Utc::now());
    }

    #[test]
    fn progress_texts() {
        let (b, _) = load_book_at(&folder(&[3600, 3600]), &tmp()).unwrap();
        assert_eq!(progress_text(&b, 3_600_000, 1.0), "50 % · 1 h 0 min left");
        assert_eq!(progress_text(&b, 3_600_000, 2.0), "50 % · 30 min left");
        assert_eq!(length_text(b.total_ms), "2 h 0 min");
        assert_eq!(length_text(90_000), "2 min");
    }
}
