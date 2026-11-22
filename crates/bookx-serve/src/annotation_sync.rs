//! Annotation sync for KOReader: highlights, notes and bookmarks travel between a KOReader device
//! (through the `bookx.koplugin` plugin) and Bookx's built-in server.
//!
//! KOReader has no standard annotation sync protocol, so this is Bookx's own small extension,
//! served next to the KOSync routes (it uses the same account and the same document id, the
//! partial MD5 of the book file):
//!
//! | Route | Meaning |
//! |---|---|
//! | `PUT /bookx/annotations/<document>` | upserts annotations; answers the merged state of the book |
//! | `GET /bookx/annotations/<document>[?since=<seq>]` | the stored annotations (tombstones included) |
//!
//! Both need the `x-auth-user` / `x-auth-key` headers of KOSync. The design and the rules are in
//! `docs/koreader-annotation-sync.md`; in short:
//!
//! * an annotation is identified by `(document, id)`; a `PUT` is an idempotent upsert;
//! * the conflict rule is last write wins per annotation by its `updated` time (seconds); on a tie
//!   a live annotation beats a tombstone and otherwise the stored one stays, so a merge never
//!   loses an annotation to a clock tie;
//! * a deletion is a tombstone (`deleted: true`), kept for a year so every device hears of it;
//! * every request is validated as a whole (nothing is stored if one annotation is invalid),
//!   sizes are limited, and each user may make a limited number of requests per minute.
//!
//! [`AnnotationSync::handle`] is a pure function of a [`Request`], like the KOSync handler.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::http::{Request, Response, split_path};

pub const KIND_HIGHLIGHT: &str = "highlight";
pub const KIND_BOOKMARK: &str = "bookmark";
/// Origins: who made (or last changed) an annotation.
pub const ORIGIN_KOREADER: &str = "koreader";
pub const ORIGIN_BOOKX: &str = "bookx";

pub const MAX_ANNOTATIONS_PER_REQUEST: usize = 1000;
pub const MAX_ANNOTATIONS_PER_DOCUMENT: usize = 5000;
pub const MAX_DOCUMENTS_PER_USER: usize = 5000;
pub const MAX_TEXT_CHARS: usize = 20_000;
pub const MAX_POSITION_CHARS: usize = 1000;
pub const MAX_NAME_CHARS: usize = 128;
pub const MAX_TITLE_CHARS: usize = 512;
/// Largest body the handler accepts (the transport has its own, equal limit).
/// Version of the `/bookx` wire protocol (annotations route, `/info`). Raised when a change would break
/// an installed plugin; `MIN_PLUGIN_PROTOCOL` is the oldest plugin protocol this server still answers.
pub const PROTOCOL: u32 = 1;
pub const MIN_PLUGIN_PROTOCOL: u32 = 1;
pub const MAX_BODY: usize = 1024 * 1024;
/// Requests a user may make per window.
pub const RATE_LIMIT: usize = 120;
pub const RATE_WINDOW_SECS: i64 = 60;
/// An `updated` time further than this in the future is rejected (a wrong clock would otherwise
/// win every later conflict).
pub const MAX_FUTURE_SECS: i64 = 86_400;
/// Tombstones older than this (server time) are forgotten.
pub const TOMBSTONE_KEEP_SECS: i64 = 365 * 86_400;

/// One highlight, note or bookmark as it travels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Annotation {
    /// Chosen by the device that made it; unique inside the book.
    pub id: String,
    /// [`KIND_HIGHLIGHT`] or [`KIND_BOOKMARK`].
    pub kind: String,
    /// The highlighted text. Bookx finds the place in the book by this text.
    #[serde(default)]
    pub text: String,
    /// The reader's note (empty when none).
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub color: String,
    /// KOReader's `drawer`: `lighten`, `underscore`, `strikeout`, `invert`.
    #[serde(default)]
    pub drawer: String,
    #[serde(default)]
    pub chapter: String,
    /// Position hints (KOReader xpointers or PDF positions). Never trusted to place a highlight.
    #[serde(default)]
    pub pos0: String,
    #[serde(default)]
    pub pos1: String,
    #[serde(default)]
    pub page: Option<i64>,
    /// 0..=1 place in the book, for bookmarks.
    #[serde(default)]
    pub percentage: Option<f64>,
    /// When it was made, seconds since the epoch.
    #[serde(default)]
    pub created: i64,
    /// When it was last changed, seconds since the epoch (the conflict rule uses it).
    #[serde(default)]
    pub updated: i64,
    #[serde(default)]
    pub deleted: bool,
}

/// An annotation as the server keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    #[serde(flatten)]
    pub annotation: Annotation,
    /// [`ORIGIN_KOREADER`] or [`ORIGIN_BOOKX`].
    pub origin: String,
    /// Name of the device that wrote this version.
    pub device: String,
    pub device_id: String,
    /// Server counter of the last change: readers ask for everything after the last one they saw.
    pub seq: u64,
    /// Server time of the last change.
    pub stored: i64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct DocState {
    title: String,
    authors: String,
    records: BTreeMap<String, Record>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct State {
    seq: u64,
    /// user -> document -> state
    users: HashMap<String, HashMap<String, DocState>>,
}

/// What a `PUT` did.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Outcome {
    /// New or changed on the server.
    pub accepted: usize,
    /// Already identical (a repeated request).
    pub unchanged: usize,
    /// Older than what the server has, or a tie the stored version won.
    pub stale: usize,
    /// What the accepted ones were (not sent over the wire; for the activity list).
    #[serde(skip)]
    pub detail: Detail,
}

/// The accepted annotations of one request by kind.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Detail {
    pub highlights: usize,
    pub bookmarks: usize,
    /// Highlights that carry a note.
    pub notes: usize,
    pub removed: usize,
}

/// Where the data of one request comes from.
#[derive(Debug, Clone)]
pub struct Source<'a> {
    pub origin: &'a str,
    pub device: &'a str,
    pub device_id: &'a str,
    pub title: &'a str,
    pub authors: &'a str,
}

/// One change for the Bookx side to read.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub user: String,
    pub document: String,
    pub title: String,
    pub authors: String,
    pub record: Record,
}

/// Why a request was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Invalid(String),
    TooMany(&'static str),
}

/// The annotation store and protocol handler.
pub struct AnnotationSync {
    state: Mutex<State>,
    path: Option<PathBuf>,
    clock: Box<dyn Fn() -> i64 + Send + Sync>,
    rate: Mutex<HashMap<String, VecDeque<i64>>>,
    rate_limit: usize,
    activity: Option<std::sync::Arc<crate::activity::ActivityLog>>,
    notifier: crate::notify::Notifier,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// True if the path (without the `/bookx` prefix) belongs to the annotation routes.
pub fn owns(path: &str) -> bool {
    path.trim_start_matches('/').split('/').next() == Some("annotations")
}

/// A document id: the 32 hex digits of an MD5, as KOReader sends them. Lower-cased.
pub fn valid_document(doc: &str) -> Option<String> {
    (doc.len() == 32 && doc.bytes().all(|b| b.is_ascii_hexdigit())).then(|| doc.to_lowercase())
}

fn message(status: u16, msg: &str) -> Response {
    Response::json(status, &json!({ "message": msg }))
}

fn has_control(s: &str) -> bool {
    s.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
}

fn clean_id(s: &str) -> bool {
    !s.is_empty()
        && s.chars().count() <= MAX_NAME_CHARS
        && s.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '@' | '+'))
}

fn str_field(v: &Value, key: &str, max: usize, text: bool) -> Result<String, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(s)) => {
            if s.chars().count() > max {
                return Err(format!("`{key}` is too long"));
            }
            if (!text && s.chars().any(|c| c.is_control())) || (text && has_control(s)) {
                return Err(format!("`{key}` has control characters"));
            }
            Ok(s.clone())
        }
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

fn int_field(v: &Value, key: &str) -> Result<Option<i64>, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(n) => n
            .as_i64()
            .map(Some)
            .ok_or_else(|| format!("`{key}` must be an integer")),
    }
}

/// Validates one annotation of a request.
pub fn parse_annotation(v: &Value, now: i64) -> Result<Annotation, String> {
    if !v.is_object() {
        return Err("an annotation must be an object".into());
    }
    let id = str_field(v, "id", MAX_NAME_CHARS, false)?;
    if !clean_id(&id) {
        return Err("`id` is missing or has characters that are not allowed".into());
    }
    let kind = str_field(v, "kind", 32, false)?;
    if kind != KIND_HIGHLIGHT && kind != KIND_BOOKMARK {
        return Err(format!("`kind` of `{id}` must be highlight or bookmark"));
    }
    let deleted = match v.get("deleted") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err("`deleted` must be true or false".into()),
    };
    let text = str_field(v, "text", MAX_TEXT_CHARS, true)?;
    if kind == KIND_HIGHLIGHT && !deleted && text.trim().is_empty() {
        return Err(format!("highlight `{id}` has no text"));
    }
    let percentage = match v.get("percentage") {
        None | Some(Value::Null) => None,
        Some(n) => match n.as_f64() {
            Some(p) if p.is_finite() && (0.0..=1.0).contains(&p) => Some(p),
            _ => return Err("`percentage` must be a number from 0 to 1".into()),
        },
    };
    let created = int_field(v, "created")?.unwrap_or(0);
    let updated = int_field(v, "updated")?.ok_or("`updated` is required")?;
    if created < 0 || updated < 0 {
        return Err("times must not be negative".into());
    }
    if updated > now + MAX_FUTURE_SECS || created > now + MAX_FUTURE_SECS {
        return Err("a time is too far in the future (is the device clock right?)".into());
    }
    Ok(Annotation {
        id,
        kind,
        text,
        note: str_field(v, "note", MAX_TEXT_CHARS, true)?,
        color: str_field(v, "color", 32, false)?,
        drawer: str_field(v, "drawer", 32, false)?,
        chapter: str_field(v, "chapter", MAX_POSITION_CHARS, false)?,
        pos0: str_field(v, "pos0", MAX_POSITION_CHARS, false)?,
        pos1: str_field(v, "pos1", MAX_POSITION_CHARS, false)?,
        page: int_field(v, "page")?,
        percentage,
        created,
        updated,
        deleted,
    })
}

/// The body of a `PUT`: who sends it and the annotations.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub origin: String,
    pub device: String,
    pub device_id: String,
    pub title: String,
    pub authors: String,
    pub annotations: Vec<Annotation>,
}

/// Validates a whole request body; nothing of it is used if one part is invalid.
pub fn parse_request(body: &[u8], now: i64) -> Result<Parsed, String> {
    if body.len() > MAX_BODY {
        return Err("the request is too large".into());
    }
    let v: Value = serde_json::from_slice(body).map_err(|_| "the body is not valid JSON")?;
    if !v.is_object() {
        return Err("the body must be a JSON object".into());
    }
    let origin = match str_field(&v, "origin", 32, false)?.as_str() {
        "" | ORIGIN_KOREADER => ORIGIN_KOREADER.to_string(),
        ORIGIN_BOOKX => ORIGIN_BOOKX.to_string(),
        _ => return Err("`origin` must be koreader or bookx".into()),
    };
    let device = str_field(&v, "device", MAX_NAME_CHARS, false)?;
    let list = v
        .get("annotations")
        .and_then(Value::as_array)
        .ok_or("`annotations` must be a list")?;
    if list.len() > MAX_ANNOTATIONS_PER_REQUEST {
        return Err(format!(
            "at most {MAX_ANNOTATIONS_PER_REQUEST} annotations per request"
        ));
    }
    let mut annotations = Vec::with_capacity(list.len());
    let mut seen = std::collections::HashSet::new();
    for a in list {
        let a = parse_annotation(a, now)?;
        if !seen.insert(a.id.clone()) {
            return Err(format!("the id `{}` appears twice", a.id));
        }
        annotations.push(a);
    }
    Ok(Parsed {
        origin,
        device,
        device_id: str_field(&v, "device_id", MAX_NAME_CHARS, false)?,
        title: str_field(&v, "title", MAX_TITLE_CHARS, false)?,
        authors: str_field(&v, "authors", MAX_TITLE_CHARS, false)?,
        annotations,
    })
}

/// True if `new` replaces `old` under the conflict rule.
pub fn supersedes(new: &Annotation, old: &Annotation) -> bool {
    if new.updated != old.updated {
        return new.updated > old.updated;
    }
    // a tie: a live annotation beats a tombstone, otherwise the stored one stays
    old.deleted && !new.deleted
}

impl State {
    fn upsert(
        &mut self,
        user: &str,
        doc: &str,
        src: &Source,
        list: Vec<Annotation>,
        now: i64,
    ) -> Result<Outcome, Refusal> {
        let docs = self.users.entry(user.to_string()).or_default();
        if !docs.contains_key(doc) && docs.len() >= MAX_DOCUMENTS_PER_USER {
            return Err(Refusal::TooMany("documents"));
        }
        let state = docs.entry(doc.to_string()).or_default();
        let fresh = list
            .iter()
            .filter(|a| !state.records.contains_key(&a.id))
            .count();
        if state.records.len() + fresh > MAX_ANNOTATIONS_PER_DOCUMENT {
            return Err(Refusal::TooMany("annotations"));
        }
        let mut out = Outcome::default();
        let mut seq = self.seq;
        for a in list {
            match state.records.get(&a.id) {
                Some(old) if old.annotation == a => out.unchanged += 1,
                Some(old) if !supersedes(&a, &old.annotation) => out.stale += 1,
                _ => {
                    seq += 1;
                    out.accepted += 1;
                    if a.deleted {
                        out.detail.removed += 1;
                    } else if a.kind == KIND_BOOKMARK {
                        out.detail.bookmarks += 1;
                    } else {
                        out.detail.highlights += 1;
                        if !a.note.trim().is_empty() {
                            out.detail.notes += 1;
                        }
                    }
                    state.records.insert(
                        a.id.clone(),
                        Record {
                            annotation: a,
                            origin: src.origin.to_string(),
                            device: src.device.to_string(),
                            device_id: src.device_id.to_string(),
                            seq,
                            stored: now,
                        },
                    );
                }
            }
        }
        self.seq = seq;
        if !src.title.is_empty() && src.origin == ORIGIN_KOREADER {
            state.title = src.title.to_string();
            state.authors = src.authors.to_string();
        }
        state
            .records
            .retain(|_, r| !(r.annotation.deleted && now - r.stored > TOMBSTONE_KEEP_SECS));
        Ok(out)
    }
}

impl AnnotationSync {
    /// In memory only (tests).
    pub fn memory() -> AnnotationSync {
        AnnotationSync {
            state: Mutex::new(State::default()),
            path: None,
            clock: Box::new(|| chrono::Utc::now().timestamp()),
            rate: Mutex::new(HashMap::new()),
            rate_limit: RATE_LIMIT,
            activity: None,
            notifier: crate::notify::Notifier::default(),
        }
    }

    /// Records devices and what they sent in `log` (see [`crate::activity`]).
    pub fn with_activity(
        mut self,
        log: std::sync::Arc<crate::activity::ActivityLog>,
    ) -> AnnotationSync {
        self.activity = Some(log);
        self
    }

    /// Kept in a JSON file, written atomically after every change; starts empty if it is not there.
    pub fn open(path: &Path) -> Result<AnnotationSync> {
        let state = if path.exists() {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            if text.trim().is_empty() {
                State::default()
            } else {
                serde_json::from_str(&text).context("corrupt annotation store")?
            }
        } else {
            State::default()
        };
        Ok(AnnotationSync {
            state: Mutex::new(state),
            path: Some(path.to_path_buf()),
            ..AnnotationSync::memory()
        })
    }

    pub fn with_clock(mut self, clock: impl Fn() -> i64 + Send + Sync + 'static) -> AnnotationSync {
        self.clock = Box::new(clock);
        self
    }

    pub fn with_rate_limit(mut self, per_window: usize) -> AnnotationSync {
        self.rate_limit = per_window;
        self
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    /// Tells `listener` (on the thread of the request) about every request of a device that
    /// changed the store, once per request. Bookx's own publishing does not notify.
    pub fn on_change(&self, listener: Option<crate::notify::Listener>) {
        self.notifier.set(listener);
    }

    /// The activity log this store reports to, if any.
    pub fn activity(&self) -> Option<std::sync::Arc<crate::activity::ActivityLog>> {
        self.activity.clone()
    }

    fn save(&self, st: &State) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(st)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Stores annotations (the `PUT` and Bookx's own side). Nothing is stored when it fails.
    pub fn put(
        &self,
        user: &str,
        doc: &str,
        src: &Source,
        list: Vec<Annotation>,
    ) -> Result<Outcome, Refusal> {
        let now = self.now();
        let mut st = lock(&self.state);
        let backup = st.clone();
        let out = st.upsert(user, doc, src, list, now)?;
        if out.accepted > 0 && self.save(&st).is_err() {
            *st = backup;
            return Err(Refusal::Invalid("the server could not save".into()));
        }
        Ok(out)
    }

    /// The records of one book with `seq > since`, oldest change first.
    pub fn records(&self, user: &str, doc: &str, since: u64) -> Vec<Record> {
        let st = lock(&self.state);
        let mut v: Vec<Record> = st
            .users
            .get(user)
            .and_then(|d| d.get(doc))
            .map(|d| {
                d.records
                    .values()
                    .filter(|r| r.seq > since)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        v.sort_by_key(|r| r.seq);
        v
    }

    /// Highest change number so far.
    pub fn seq(&self) -> u64 {
        lock(&self.state).seq
    }

    /// Every change after `since` (all users, all books), for the Bookx side to merge.
    pub fn changes_since(&self, since: u64) -> Vec<Change> {
        let st = lock(&self.state);
        let mut v = Vec::new();
        for (user, docs) in &st.users {
            for (doc, d) in docs {
                for r in d.records.values().filter(|r| r.seq > since) {
                    v.push(Change {
                        user: user.clone(),
                        document: doc.clone(),
                        title: d.title.clone(),
                        authors: d.authors.clone(),
                        record: r.clone(),
                    });
                }
            }
        }
        v.sort_by_key(|c| c.record.seq);
        v
    }

    /// Every (user, document) the server knows, with the title KOReader sent.
    pub fn documents(&self) -> Vec<(String, String, String, String)> {
        let st = lock(&self.state);
        let mut v = Vec::new();
        for (user, docs) in &st.users {
            for (doc, d) in docs {
                v.push((
                    user.clone(),
                    doc.clone(),
                    d.title.clone(),
                    d.authors.clone(),
                ));
            }
        }
        v.sort();
        v
    }

    fn allow(&self, user: &str) -> Result<(), i64> {
        let now = self.now();
        let mut rate = lock(&self.rate);
        let q = rate.entry(user.to_string()).or_default();
        while q.front().is_some_and(|t| now - *t >= RATE_WINDOW_SECS) {
            q.pop_front();
        }
        if q.len() >= self.rate_limit {
            let wait = RATE_WINDOW_SECS - (now - q.front().copied().unwrap_or(now));
            return Err(wait.max(1));
        }
        q.push_back(now);
        Ok(())
    }

    /// Handles a request whose path (without `/bookx`) starts with `annotations`.
    /// `auth(user, key)` says whether the credentials are right.
    pub fn handle(&self, req: &Request, auth: &dyn Fn(&str, &str) -> bool) -> Response {
        let (Some(user), Some(key)) = (req.header("x-auth-user"), req.header("x-auth-key")) else {
            return message(401, "Unauthorized");
        };
        if !auth(user, key) {
            return message(401, "Unauthorized");
        }
        let resp = self.handle_authed(req, user);
        if resp.status >= 400 {
            self.note_refusal(req, user, &resp);
        }
        resp
    }

    /// A refused request of a logged-in device is a visible event, not a silent one.
    fn note_refusal(&self, req: &Request, user: &str, resp: &Response) {
        let Some(log) = &self.activity else { return };
        let who = crate::activity::Ident::from_request(req);
        log.touch(&who, user, false);
        let reason = serde_json::from_slice::<Value>(&resp.body)
            .ok()
            .and_then(|v| v["message"].as_str().map(str::to_string))
            .unwrap_or_default();
        let doc = split_path(&req.path)
            .and_then(|s| s.get(1).cloned())
            .unwrap_or_default();
        log.record_for(
            &who,
            &doc,
            "",
            crate::activity::What::Rejected {
                status: resp.status,
                reason,
            },
        );
    }

    fn handle_authed(&self, req: &Request, user: &str) -> Response {
        if let Some(log) = &self.activity {
            if req.method != "PUT" {
                log.touch(&crate::activity::Ident::from_request(req), user, false);
            }
        }
        let Some(segs) = split_path(&req.path) else {
            return message(400, "Invalid path");
        };
        let segs: Vec<&str> = segs.iter().map(|s| s.as_str()).collect();
        let [first, doc] = segs.as_slice() else {
            return message(404, "Not found");
        };
        if *first != "annotations" {
            return message(404, "Not found");
        }
        let Some(doc) = valid_document(doc) else {
            return message(400, "Invalid document id");
        };
        let method = req.method.as_str();
        if !matches!(method, "PUT" | "GET" | "HEAD") {
            return message(405, "Method not allowed").with_header("Allow", "GET, PUT");
        }
        if let Err(wait) = self.allow(user) {
            return message(429, "Too many requests").with_header("Retry-After", &wait.to_string());
        }
        let since = req
            .query_params()
            .get("since")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        if method == "PUT" {
            self.put_request(user, &doc, req, since)
        } else {
            self.listing(user, &doc, since, None)
        }
    }

    fn note_put(&self, req: &Request, user: &str, doc: &str, parsed: &Parsed, out: &Outcome) {
        let Some(log) = &self.activity else { return };
        let who =
            crate::activity::Ident::from_request(req).with_body(&parsed.device, &parsed.device_id);
        log.touch(&who, user, true);
        let d = &out.detail;
        let what = if out.accepted == 0 {
            crate::activity::What::Nothing
        } else {
            crate::activity::What::Received {
                highlights: d.highlights as u32,
                bookmarks: d.bookmarks as u32,
                notes: d.notes as u32,
                removed: d.removed as u32,
            }
        };
        log.record_for(&who, doc, &parsed.title, what);
    }

    fn listing(&self, user: &str, doc: &str, since: u64, outcome: Option<Outcome>) -> Response {
        let records = self.records(user, doc, since);
        let list: Vec<Value> = records.iter().map(wire).collect();
        let mut v = json!({"document": doc, "seq": self.seq(), "annotations": list});
        if let Some(o) = outcome {
            v["result"] = serde_json::to_value(o).unwrap_or(Value::Null);
            // whether Bookx has this book (null until the app has looked): the plugin lists the ones it has not
            v["in_library"] = self
                .activity
                .as_ref()
                .and_then(|a| a.in_library(doc))
                .map_or(Value::Null, Value::Bool);
        }
        Response::json(200, &v)
    }

    fn put_request(&self, user: &str, doc: &str, req: &Request, since: u64) -> Response {
        if req.body.len() > MAX_BODY {
            return message(413, "Request body too large");
        }
        let mut parsed = match parse_request(&req.body, self.now()) {
            Ok(p) => p,
            Err(e) => return message(422, &e),
        };
        let src = Source {
            origin: &parsed.origin,
            device: &parsed.device,
            device_id: &parsed.device_id,
            title: &parsed.title,
            authors: &parsed.authors,
        };
        match self.put(user, doc, &src, std::mem::take(&mut parsed.annotations)) {
            Ok(out) => {
                self.note_put(req, user, doc, &parsed, &out);
                if out.accepted > 0 && parsed.origin != ORIGIN_BOOKX {
                    self.notifier.emit(&crate::notify::Note::Annotations {
                        user: user.to_string(),
                        document: doc.to_string(),
                        device: parsed.device.clone(),
                        seq: self.seq(),
                        highlights: out.detail.highlights,
                        bookmarks: out.detail.bookmarks,
                        notes: out.detail.notes,
                        removed: out.detail.removed,
                    });
                }
                self.listing(user, doc, since, Some(out))
            }
            Err(Refusal::Invalid(m)) => message(500, &m),
            Err(Refusal::TooMany(what)) => message(413, &format!("Too many {what} stored")),
        }
    }
}

/// A record as the HTTP answer carries it.
pub fn wire(r: &Record) -> Value {
    let a = &r.annotation;
    json!({
        "id": a.id, "kind": a.kind, "text": a.text, "note": a.note, "color": a.color,
        "drawer": a.drawer, "chapter": a.chapter, "pos0": a.pos0, "pos1": a.pos1,
        "page": a.page, "percentage": a.percentage,
        "created": a.created, "updated": a.updated, "deleted": a.deleted,
        "origin": r.origin, "device": r.device, "seq": r.seq,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const DOC: &str = "0123456789abcdef0123456789abcdef";
    const NOW: i64 = 1_700_000_000;

    fn sync() -> AnnotationSync {
        AnnotationSync::memory().with_clock(|| NOW)
    }

    fn ok_auth(u: &str, k: &str) -> bool {
        u == "ann" && k == "key"
    }

    fn req(method: &str, doc: &str, body: &Value) -> Request {
        Request::new(method, &format!("/annotations/{doc}"))
            .with_header("x-auth-user", "ann")
            .with_header("x-auth-key", "key")
            .with_body(body.to_string().as_bytes())
    }

    fn hl(id: &str, text: &str, updated: i64) -> Value {
        json!({"id": id, "kind": "highlight", "text": text, "updated": updated, "created": updated})
    }

    fn put(s: &AnnotationSync, list: Vec<Value>) -> Response {
        s.handle(
            &req(
                "PUT",
                DOC,
                &json!({"device": "Libra", "title": "Dune", "annotations": list}),
            ),
            &ok_auth,
        )
    }

    fn body(r: &Response) -> Value {
        serde_json::from_slice(&r.body).unwrap()
    }

    fn logged() -> (AnnotationSync, std::sync::Arc<crate::activity::ActivityLog>) {
        let log = std::sync::Arc::new(crate::activity::ActivityLog::memory());
        (sync().with_activity(log.clone()), log)
    }

    fn heard(s: &AnnotationSync) -> std::sync::Arc<std::sync::Mutex<Vec<crate::notify::Note>>> {
        let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let g = got.clone();
        s.on_change(Some(std::sync::Arc::new(move |n| {
            g.lock().unwrap().push(n.clone())
        })));
        got
    }

    #[test]
    fn a_request_that_changes_the_store_notifies_exactly_once_and_a_repeat_not_at_all() {
        let s = sync();
        let got = heard(&s);
        let list = vec![
            hl("a", "first words", NOW - 9),
            hl("b", "second words", NOW - 9),
        ];
        assert_eq!(put(&s, list.clone()).status, 200);
        assert_eq!(
            got.lock().unwrap().len(),
            1,
            "one notice for two annotations"
        );
        match &got.lock().unwrap()[0] {
            crate::notify::Note::Annotations {
                document,
                device,
                highlights,
                seq,
                ..
            } => {
                assert_eq!(document, DOC);
                assert_eq!(device, "Libra");
                assert_eq!(*highlights, 2);
                assert!(*seq >= 2);
            }
            other => panic!("{other:?}"),
        }
        put(&s, list);
        assert_eq!(
            got.lock().unwrap().len(),
            1,
            "an identical repeat changes nothing"
        );
        put(&s, vec![hl("a", "first words", NOW - 20)]);
        assert_eq!(
            got.lock().unwrap().len(),
            1,
            "a stale version changes nothing"
        );
    }

    #[test]
    fn a_refused_request_and_bookx_own_publishing_do_not_notify() {
        let s = sync();
        let got = heard(&s);
        let bad = s.handle(
            &req("PUT", DOC, &json!({"annotations": [{"id": "x"}]})),
            &ok_auth,
        );
        assert_eq!(bad.status, 422);
        let src = Source {
            origin: ORIGIN_BOOKX,
            device: "Bookx",
            device_id: "",
            title: "",
            authors: "",
        };
        let a = parse_annotation(&hl("c", "from bookx here", NOW - 5), NOW).unwrap();
        s.put("ann", DOC, &src, vec![a]).unwrap();
        assert!(got.lock().unwrap().is_empty());
    }

    #[test]
    fn a_sync_shows_up_as_a_device_and_an_event_with_the_counts() {
        use crate::activity::What;
        let (s, log) = logged();
        let mut note = hl("b", "noted words here", NOW - 9);
        note["note"] = json!("my note");
        let mark = json!({"id": "m", "kind": "bookmark", "updated": NOW - 8, "created": NOW - 8});
        let r = s.handle(
            &req(
                "PUT",
                DOC,
                &json!({"device": "Kobo Libra", "device_id": "dev-1", "title": "Dune",
                    "annotations": [hl("a", "plain words here", NOW - 10), note, mark]}),
            )
            .with_header("x-bookx-client", "KOReader/2026.07.2 bookx-plugin/0.3.1")
            .with_header("x-bookx-protocol", "1"),
            &ok_auth,
        );
        assert_eq!(r.status, 200);
        let d = log.devices();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].name, "Kobo Libra");
        assert_eq!(d[0].id, "dev-1");
        assert_eq!(d[0].client, "KOReader 2026.07.2");
        assert_eq!(d[0].plugin, "0.3.1");
        assert!(d[0].last_sync.is_some());
        let e = log.events(5);
        assert_eq!(e[0].title, "Dune");
        assert_eq!(e[0].document, DOC);
        assert_eq!(
            e[0].what,
            What::Received {
                highlights: 2,
                bookmarks: 1,
                notes: 1,
                removed: 0
            }
        );
        // the same request again: the device is seen, nothing is new
        put(&s, vec![hl("a", "plain words here", NOW - 10)]);
        assert_eq!(log.events(5)[0].what, What::Nothing);
    }

    #[test]
    fn a_refused_request_is_an_event_not_silence() {
        use crate::activity::What;
        let (s, log) = logged();
        let r = s.handle(&req("PUT", DOC, &json!({"annotations": "nope"})), &ok_auth);
        assert_eq!(r.status, 422);
        let e = log.events(5);
        assert_eq!(e.len(), 1);
        match &e[0].what {
            What::Rejected { status, reason } => {
                assert_eq!(*status, 422);
                assert!(reason.contains("annotations"), "{reason}");
            }
            w => panic!("{w:?}"),
        }
        assert_eq!(
            log.devices().len(),
            1,
            "the device that was refused is still seen"
        );
    }

    #[test]
    fn a_request_that_does_not_log_in_leaves_no_trace() {
        let (s, log) = logged();
        let r = s.handle(
            &Request::new("PUT", &format!("/annotations/{DOC}"))
                .with_header("x-auth-user", "ann")
                .with_header("x-auth-key", "wrong")
                .with_header("x-bookx-device", "Intruder"),
            &ok_auth,
        );
        assert_eq!(r.status, 401);
        assert!(log.devices().is_empty() && log.events(5).is_empty());
    }

    #[test]
    fn the_answer_says_whether_bookx_has_the_book_once_the_app_knows() {
        let (s, log) = logged();
        assert!(body(&put(&s, vec![hl("a", "words words", NOW - 5)]))["in_library"].is_null());
        log.set_in_library(DOC, false);
        assert_eq!(
            body(&put(&s, vec![hl("a", "words words", NOW - 5)]))["in_library"],
            false
        );
        log.set_in_library(DOC, true);
        assert_eq!(
            body(&put(&s, vec![hl("a", "words words", NOW - 5)]))["in_library"],
            true
        );
    }

    #[test]
    fn routes_are_owned() {
        assert!(owns("/annotations/x"));
        assert!(owns("annotations"));
        assert!(!owns("/syncs/progress/x"));
    }

    #[test]
    fn needs_credentials() {
        let s = sync();
        let r = s.handle(
            &Request::new("GET", &format!("/annotations/{DOC}")),
            &ok_auth,
        );
        assert_eq!(r.status, 401);
        let r = s.handle(
            &Request::new("GET", &format!("/annotations/{DOC}"))
                .with_header("x-auth-user", "ann")
                .with_header("x-auth-key", "wrong"),
            &ok_auth,
        );
        assert_eq!(r.status, 401);
    }

    #[test]
    fn put_then_get_round_trip() {
        let s = sync();
        let r = put(&s, vec![hl("a", "Fear is the mind-killer", NOW - 10)]);
        assert_eq!(r.status, 200);
        assert_eq!(body(&r)["result"]["accepted"], 1);
        let g = s.handle(&req("GET", DOC, &json!({})), &ok_auth);
        let v = body(&g);
        assert_eq!(v["annotations"][0]["text"], "Fear is the mind-killer");
        assert_eq!(v["annotations"][0]["device"], "Libra");
        assert_eq!(v["annotations"][0]["origin"], "koreader");
    }

    #[test]
    fn repeating_a_put_changes_nothing() {
        let s = sync();
        put(&s, vec![hl("a", "text one", NOW - 10)]);
        let seq = s.seq();
        let r = put(&s, vec![hl("a", "text one", NOW - 10)]);
        assert_eq!(body(&r)["result"]["unchanged"], 1);
        assert_eq!(s.seq(), seq);
    }

    #[test]
    fn newer_wins_older_is_stale() {
        let s = sync();
        put(&s, vec![hl("a", "first", NOW - 10)]);
        let r = put(&s, vec![hl("a", "second", NOW - 5)]);
        assert_eq!(body(&r)["result"]["accepted"], 1);
        let r = put(&s, vec![hl("a", "third", NOW - 20)]);
        assert_eq!(body(&r)["result"]["stale"], 1);
        assert_eq!(s.records("ann", DOC, 0)[0].annotation.text, "second");
    }

    #[test]
    fn a_tie_never_loses_a_live_annotation() {
        let s = sync();
        put(&s, vec![hl("a", "keep me", NOW - 10)]);
        let mut gone = hl("a", "keep me", NOW - 10);
        gone["deleted"] = json!(true);
        let r = put(&s, vec![gone.clone()]);
        assert_eq!(body(&r)["result"]["stale"], 1);
        assert!(!s.records("ann", DOC, 0)[0].annotation.deleted);
        // and the other way round: a live one resurrects a tombstone of the same second
        let s = sync();
        put(&s, vec![gone]);
        let r = put(&s, vec![hl("a", "keep me", NOW - 10)]);
        assert_eq!(body(&r)["result"]["accepted"], 1);
    }

    #[test]
    fn tombstones_are_kept_and_win_when_newer() {
        let s = sync();
        put(&s, vec![hl("a", "to delete", NOW - 100)]);
        let mut gone = hl("a", "", NOW - 50);
        gone["deleted"] = json!(true);
        put(&s, vec![gone]);
        let recs = s.records("ann", DOC, 0);
        assert_eq!(recs.len(), 1);
        assert!(recs[0].annotation.deleted);
        // an older edit does not bring it back
        let r = put(&s, vec![hl("a", "old edit", NOW - 80)]);
        assert_eq!(body(&r)["result"]["stale"], 1);
        assert!(s.records("ann", DOC, 0)[0].annotation.deleted);
    }

    #[test]
    fn old_tombstones_are_forgotten() {
        let t = Arc::new(Mutex::new(NOW));
        let c = t.clone();
        let s = AnnotationSync::memory().with_clock(move || *lock(&c));
        let mut gone = hl("a", "", NOW - 5);
        gone["deleted"] = json!(true);
        put(&s, vec![gone]);
        *lock(&t) = NOW + TOMBSTONE_KEEP_SECS + 10;
        put(&s, vec![hl("b", "another one", NOW + TOMBSTONE_KEEP_SECS)]);
        let recs = s.records("ann", DOC, 0);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].annotation.id, "b");
    }

    #[test]
    fn since_returns_only_newer_changes() {
        let s = sync();
        put(
            &s,
            vec![hl("a", "one one", NOW - 10), hl("b", "two two", NOW - 10)],
        );
        let seq = s.seq();
        put(&s, vec![hl("c", "three", NOW - 5)]);
        let g = s.handle(
            &Request::new("GET", &format!("/annotations/{DOC}?since={seq}"))
                .with_header("x-auth-user", "ann")
                .with_header("x-auth-key", "key"),
            &ok_auth,
        );
        let v = body(&g);
        assert_eq!(v["annotations"].as_array().unwrap().len(), 1);
        assert_eq!(v["annotations"][0]["id"], "c");
    }

    #[test]
    fn invalid_requests_store_nothing() {
        let s = sync();
        let bad = |b: Value| s.handle(&req("PUT", DOC, &b), &ok_auth).status;
        assert_eq!(bad(json!({"annotations": "no"})), 422);
        assert_eq!(bad(json!({"annotations": [{"id": "a"}]})), 422);
        assert_eq!(bad(json!({"annotations": [hl("a", "  ", NOW)]})), 422);
        assert_eq!(bad(json!({"annotations": [hl("../x", "text", NOW)]})), 422);
        assert_eq!(
            bad(
                json!({"annotations": [{"id": "a", "kind": "weird", "text": "t", "updated": NOW}]})
            ),
            422
        );
        assert_eq!(
            bad(json!({"annotations": [hl("a", "t", NOW + MAX_FUTURE_SECS + 5)]})),
            422
        );
        assert_eq!(bad(json!({"annotations": [hl("a", "t", -1)]})), 422);
        assert_eq!(
            bad(
                json!({"annotations": [{"id": "a", "kind": "highlight", "text": "text", "updated": 1.5}]})
            ),
            422
        );
        assert_eq!(bad(json!({"origin": "x", "annotations": []})), 422);
        // a good annotation next to a bad one is not stored either
        assert_eq!(
            bad(json!({"annotations": [hl("good", "fine text", NOW), hl("bad", "", NOW)]})),
            422
        );
        // duplicate ids inside a request
        assert_eq!(
            bad(json!({"annotations": [hl("a", "one", NOW), hl("a", "two", NOW)]})),
            422
        );
        assert_eq!(s.seq(), 0);
    }

    #[test]
    fn malformed_json_and_wrong_shapes() {
        let s = sync();
        let r = s.handle(
            &Request::new("PUT", &format!("/annotations/{DOC}"))
                .with_header("x-auth-user", "ann")
                .with_header("x-auth-key", "key")
                .with_body(b"{not json"),
            &ok_auth,
        );
        assert_eq!(r.status, 422);
        let r = s.handle(
            &Request::new("PUT", &format!("/annotations/{DOC}"))
                .with_header("x-auth-user", "ann")
                .with_header("x-auth-key", "key")
                .with_body(b"[1,2]"),
            &ok_auth,
        );
        assert_eq!(r.status, 422);
    }

    #[test]
    fn rejects_bad_documents_and_paths() {
        let s = sync();
        for p in [
            "/annotations/short",
            "/annotations/zzzz456789abcdef0123456789abcdef",
            "/annotations/..%2Fkosync",
            "/annotations/%2e%2e",
            "/annotations",
            "/annotations/a/b",
        ] {
            let r = s.handle(
                &Request::new("GET", p)
                    .with_header("x-auth-user", "ann")
                    .with_header("x-auth-key", "key"),
                &ok_auth,
            );
            assert!(matches!(r.status, 400 | 404), "{p}: {}", r.status);
        }
        // upper-case digits are the same document
        let r = s.handle(
            &req(
                "PUT",
                &DOC.to_uppercase(),
                &json!({"annotations": [hl("a", "text", NOW)]}),
            ),
            &ok_auth,
        );
        assert_eq!(r.status, 200);
        assert_eq!(s.records("ann", DOC, 0).len(), 1);
    }

    #[test]
    fn oversize_is_refused() {
        let s = sync();
        let big = "x".repeat(MAX_TEXT_CHARS + 1);
        assert_eq!(put(&s, vec![hl("a", &big, NOW)]).status, 422);
        let r = s.handle(
            &Request::new("PUT", &format!("/annotations/{DOC}"))
                .with_header("x-auth-user", "ann")
                .with_header("x-auth-key", "key")
                .with_body(&vec![b' '; MAX_BODY + 1]),
            &ok_auth,
        );
        assert_eq!(r.status, 413);
        let many: Vec<Value> = (0..MAX_ANNOTATIONS_PER_REQUEST + 1)
            .map(|i| hl(&format!("a{i}"), "some text", NOW))
            .collect();
        assert_eq!(put(&s, many).status, 422);
    }

    #[test]
    fn per_document_cap() {
        let s = sync();
        for chunk in 0..MAX_ANNOTATIONS_PER_DOCUMENT / 1000 {
            let list: Vec<Value> = (0..1000)
                .map(|i| hl(&format!("a{chunk}-{i}"), "some text", NOW))
                .collect();
            assert_eq!(put(&s, list).status, 200);
        }
        assert_eq!(
            put(&s, vec![hl("one-too-many", "some text", NOW)]).status,
            413
        );
        // an update of a stored one still works
        assert_eq!(put(&s, vec![hl("a0-0", "changed text", NOW)]).status, 200);
    }

    #[test]
    fn rate_limit_per_user() {
        let s = AnnotationSync::memory()
            .with_clock(|| NOW)
            .with_rate_limit(3);
        for _ in 0..3 {
            assert_eq!(put(&s, vec![]).status, 200);
        }
        let r = put(&s, vec![]);
        assert_eq!(r.status, 429);
        assert!(r.headers.iter().any(|(k, _)| k == "Retry-After"));
        // another user is not affected
        let other = s.handle(
            &Request::new("GET", &format!("/annotations/{DOC}"))
                .with_header("x-auth-user", "bob")
                .with_header("x-auth-key", "b"),
            &|u: &str, _: &str| u == "bob",
        );
        assert_eq!(other.status, 200);
    }

    #[test]
    fn users_do_not_see_each_other() {
        let s = sync();
        put(&s, vec![hl("a", "private text", NOW)]);
        let r = s.handle(
            &Request::new("GET", &format!("/annotations/{DOC}"))
                .with_header("x-auth-user", "bob")
                .with_header("x-auth-key", "b"),
            &|u: &str, _: &str| u == "bob",
        );
        assert_eq!(body(&r)["annotations"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn methods() {
        let s = sync();
        let r = s.handle(&req("DELETE", DOC, &json!({})), &ok_auth);
        assert_eq!(r.status, 405);
    }

    #[test]
    fn changes_for_the_bookx_side() {
        let s = sync();
        put(&s, vec![hl("a", "one one", NOW - 1)]);
        let c = s.changes_since(0);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].title, "Dune");
        assert_eq!(c[0].document, DOC);
        assert!(s.changes_since(s.seq()).is_empty());
        // Bookx's own annotations do not set the title
        let src = Source {
            origin: ORIGIN_BOOKX,
            device: "Bookx",
            device_id: "",
            title: "Other",
            authors: "",
        };
        let a = parse_annotation(&hl("bx-1", "from bookx", NOW - 1), NOW).unwrap();
        s.put("ann", DOC, &src, vec![a]).unwrap();
        assert_eq!(s.changes_since(0)[0].title, "Dune");
    }

    #[test]
    fn file_store_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("bookx-annsync-{}", uuid::Uuid::new_v4()));
        let path = dir.join("annotations.json");
        {
            let s = AnnotationSync::open(&path).unwrap().with_clock(|| NOW);
            put(&s, vec![hl("a", "persisted", NOW - 1)]);
        }
        let s = AnnotationSync::open(&path).unwrap().with_clock(|| NOW);
        assert_eq!(s.records("ann", DOC, 0)[0].annotation.text, "persisted");
        assert_eq!(s.seq(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn concurrent_puts_lose_nothing() {
        let s = Arc::new(sync().with_rate_limit(10_000));
        let mut threads = Vec::new();
        for t in 0..8 {
            let s = s.clone();
            threads.push(std::thread::spawn(move || {
                for i in 0..25 {
                    let r = put(
                        &s,
                        vec![hl(&format!("t{t}-{i}"), "concurrent text", NOW - 1)],
                    );
                    assert_eq!(r.status, 200);
                }
            }));
        }
        for t in threads {
            t.join().unwrap();
        }
        let recs = s.records("ann", DOC, 0);
        assert_eq!(recs.len(), 200);
        let mut seqs: Vec<u64> = recs.iter().map(|r| r.seq).collect();
        seqs.sort();
        seqs.dedup();
        assert_eq!(seqs.len(), 200, "every change has its own number");
    }

    #[test]
    fn concurrent_updates_of_one_annotation_keep_the_newest() {
        let s = Arc::new(sync());
        let mut threads = Vec::new();
        for t in 0..8i64 {
            let s = s.clone();
            threads.push(std::thread::spawn(move || {
                put(&s, vec![hl("same", &format!("version {t}"), NOW - 100 + t)]);
            }));
        }
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(s.records("ann", DOC, 0)[0].annotation.text, "version 7");
    }

    #[test]
    fn bookmark_needs_no_text() {
        let s = sync();
        let r = put(
            &s,
            vec![
                json!({"id": "bm1", "kind": "bookmark", "page": 12, "percentage": 0.25, "updated": NOW}),
            ],
        );
        assert_eq!(r.status, 200);
    }
}
