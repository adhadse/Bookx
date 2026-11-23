//! The flight recorder: what the app did last, in every area, in a form a maintainer reads at a glance.
//!
//! Any module can record one operation with one line:
//!
//! ```ignore
//! let mut t = trace::op(Area::Import, "import");
//! t.fmt("epub").size_class("size", bytes);
//! match run() { Ok(_) => t.ok(), Err(e) => t.fail(&format!("{e:#}")) }
//! ```
//!
//! and one moment with [`event`]. An [`Entry`] holds **structured facts only**: the area, a fixed
//! operation name, whether it worked, the HTTP status, an error *class* ([`ErrorClass`]), the duration
//! and a few bounded fields. A field value can only be a number, a size class, a hash, a format or a
//! `&'static str` word ([`Op::word`]), never free text, so it cannot carry a title, a path or an address.
//! The one place free text may go is the scrubbed, shortened `err=` message of a failure, which is
//! cleaned by [`crate::diag_addr::clean_text`] when it is stored and by the scrubber again before it is
//! written to the log or shown in a report.
//!
//! The recorder keeps the last [`PER_AREA`] entries per area ([`TOTAL`] in all). Failures, slow
//! operations and events are also written to the log as one breadcrumb line each (`trace sync push fail
//! http=404 ...`, [`Entry::to_line`] and [`Entry::from_line`] are inverse), and [`persist_tail`] writes
//! the newest entries of every area next to the log when the app is closed or crashes, so a report made
//! after a restart still has the tail of the last session.
//!
//! Snapshot providers ([`register`]) are the second half: each feature registers one function that
//! returns the facts about its *state* ([`crate::diag_facts::Fact`]: switches, counts, classes), so a new
//! feature adds its facts in one place and the report picks them up.
//!
//! No GTK, no I/O besides the log line and [`persist_tail`].

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::Mutex;
use std::time::Instant;

use crate::diag_facts::Fact;

/// Entries kept per area.
pub const PER_AREA: usize = 30;
/// Entries kept in all.
pub const TOTAL: usize = 400;
/// An operation at least this slow (milliseconds) is written to the log even when it worked.
pub const SLOW_MS: u64 = 250;
/// Longest error message kept.
pub const MSG_BYTES: usize = 160;
/// Fields per entry.
const MAX_FIELDS: usize = 12;

macro_rules! areas {
    ($($v:ident => $w:literal, $label:literal;)*) => {
        /// A feature area of the app.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Area { $($v),* }
        impl Area {
            pub const ALL: &'static [Area] = &[$(Area::$v),*];
            pub fn word(self) -> &'static str { match self { $(Area::$v => $w),* } }
            pub fn label(self) -> &'static str { match self { $(Area::$v => $label),* } }
            pub fn from_word(w: &str) -> Option<Area> { match w { $($w => Some(Area::$v),)* _ => None } }
        }
    };
}

areas! {
    Library => "library", "Library";
    Import => "import", "Import and add books";
    Convert => "convert", "Conversion";
    Reader => "reader", "Reader";
    Editor => "editor", "Editor";
    Search => "search", "Search";
    Tts => "tts", "Read aloud";
    Audiobook => "audiobook", "Audiobooks";
    Devices => "devices", "Devices";
    Sync => "sync", "Sync (KOSync client)";
    Server => "server", "Built-in server";
    Opds => "opds", "OPDS and web import";
    Backup => "backup", "Backups";
    News => "news", "News and read later";
    Metadata => "metadata", "Metadata and lookup";
    Mcp => "mcp", "AI assistants (MCP)";
    Plugins => "plugins", "Plugins and patches";
    Settings => "settings", "Settings";
    Secrets => "secrets", "Passwords and keyring";
    Database => "database", "Library database";
    Network => "network", "Network";
    Ui => "ui", "Display and interface";
}

/// Why a call failed, as a class. Never the text of the error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// 401 / 403: the other side did not accept the login.
    Unauthorized,
    /// Another HTTP status: the other side answered, but not with what was asked (404 is "not found").
    Status,
    Dns,
    Refused,
    Timeout,
    Tls,
    Unreachable,
    BadAddress,
    BadAnswer,
    Io,
    DiskFull,
    Locked,
    PortInUse,
    NotMounted,
    Missing,
    Denied,
    Corrupt,
    Unsupported,
    HashMismatch,
    Aborted,
    Other,
}

const CLASSES: &[(ErrorClass, &str)] = &[
    (ErrorClass::Unauthorized, "unauthorized"),
    (ErrorClass::Status, "status"),
    (ErrorClass::Dns, "dns"),
    (ErrorClass::Refused, "refused"),
    (ErrorClass::Timeout, "timeout"),
    (ErrorClass::Tls, "tls"),
    (ErrorClass::Unreachable, "unreachable"),
    (ErrorClass::BadAddress, "bad_address"),
    (ErrorClass::BadAnswer, "bad_answer"),
    (ErrorClass::Io, "io"),
    (ErrorClass::DiskFull, "disk_full"),
    (ErrorClass::Locked, "locked"),
    (ErrorClass::PortInUse, "port_in_use"),
    (ErrorClass::NotMounted, "not_mounted"),
    (ErrorClass::Missing, "missing"),
    (ErrorClass::Denied, "denied"),
    (ErrorClass::Corrupt, "corrupt"),
    (ErrorClass::Unsupported, "unsupported"),
    (ErrorClass::HashMismatch, "hash_mismatch"),
    (ErrorClass::Aborted, "aborted"),
    (ErrorClass::Other, "other"),
];

impl ErrorClass {
    pub fn word(self) -> &'static str {
        CLASSES
            .iter()
            .find(|(c, _)| *c == self)
            .map_or("other", |(_, w)| w)
    }
    pub fn from_word(w: &str) -> Option<ErrorClass> {
        CLASSES.iter().find(|(_, n)| *n == w).map(|(c, _)| *c)
    }
}

/// The HTTP status inside "the server answered 404" or "status code 404", when there is one.
pub fn status_in(msg: &str) -> Option<u16> {
    let l = msg.to_ascii_lowercase();
    for key in ["answered ", "status code ", "http status ", "http error ", "status: ", "http "] {
        if let Some(i) = l.find(key) {
            let digits: String = l[i + key.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            if let Some(s) = digits.parse().ok().filter(|s| (100..600).contains(s)) {
                return Some(s);
            }
        }
    }
    None
}

/// The class of an error message (the message itself is cleaned or dropped by the caller).
pub fn classify_error(msg: &str) -> (ErrorClass, Option<u16>) {
    let l = msg.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| l.contains(w));
    let status = status_in(&l);
    let class = if matches!(status, Some(401 | 403)) || has(&["did not accept the user name", "unauthorized", "bad credentials", "wrong password"]) {
        ErrorClass::Unauthorized
    } else if status.is_some() {
        ErrorClass::Status
    } else if has(&["certificate", "tls ", "tls:", "ssl", "handshake", "peer sent no"]) {
        ErrorClass::Tls
    } else if has(&["dns", "failed to lookup", "name or service not known", "no such host", "resolve", "nodename nor servname", "temporary failure in name"]) {
        ErrorClass::Dns
    } else if has(&["timed out", "timeout", "took too long"]) {
        ErrorClass::Timeout
    } else if has(&["refused"]) {
        ErrorClass::Refused
    } else if has(&["unreachable", "no route", "network is down", "offline mode"]) {
        ErrorClass::Unreachable
    } else if has(&["address already in use", "addr in use", "port is in use", "already in use"]) {
        ErrorClass::PortInUse
    } else if has(&["no space left", "disk full", "quota exceeded", "disk quota", "is the disk full"]) {
        ErrorClass::DiskFull
    } else if has(&["database is locked", "database table is locked", "already open in", "locked by"]) {
        ErrorClass::Locked
    } else if has(&["hash mismatch", "checksum mismatch", "sha256 mismatch", "digest mismatch", "does not match (got", "does not match the expected"]) {
        ErrorClass::HashMismatch
    } else if has(&["must start with http", "enter the address", "invalid url", "relative url"]) {
        ErrorClass::BadAddress
    } else if has(&["the server's answer", "expected value", "invalid json", "unexpected end", "key must be"]) {
        ErrorClass::BadAnswer
    } else if has(&["not mounted", "is not connected", "not connected any more"]) {
        ErrorClass::NotMounted
    } else if has(&["permission denied", "operation not permitted", "read-only file system", "access denied", "sandbox", "portal denied"]) {
        ErrorClass::Denied
    } else if has(&["corrupt", "malformed", "file is not a database", "not a zip", "invalid zip", "bad zip", "damaged", "not a valid", "checksum failed"]) {
        ErrorClass::Corrupt
    } else if has(&["unsupported", "not supported", "drm", "protected", "cannot convert", "no text layer", "opens no format"]) {
        ErrorClass::Unsupported
    } else if has(&["no such file", "not found", "not available", "need ", "install it", "no piper voice", "missing", "is not enabled", "is not installed", "cannot find", "could not find", "no audio output", "no sink", "no such device"]) {
        ErrorClass::Missing
    } else if has(&["i/o error", "broken pipe", "input/output", "unable to open database"]) {
        ErrorClass::Io
    } else {
        ErrorClass::Other
    };
    (class, status)
}

/// One thing that happened.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Unix seconds; set when recorded.
    pub time: i64,
    pub area: Area,
    /// A fixed word of the call site (`push`, `import`, `open_book`).
    pub name: String,
    pub ok: bool,
    pub class: Option<ErrorClass>,
    pub http: Option<u16>,
    pub ms: Option<u64>,
    /// `(key, value)`: values are numbers, classes, hashes or fixed words.
    pub fields: Vec<(String, String)>,
    /// The cleaned, shortened message of a failure.
    pub msg: Option<String>,
    /// How many times in a row the same thing happened (1 for once); a page turn that lays out a
    /// chapter every time must not push the rest out of the ring.
    pub repeats: u32,
}

impl Entry {
    pub fn field(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// The breadcrumb text: `trace sync push fail http=404 class=status ms=120 fmt=pdf ... err=...`.
    pub fn to_line(&self) -> String {
        self.to_line_with(&|m| m.to_string())
    }

    /// [`Entry::to_line`] with the message cleaned by `clean` (the fields are safe by construction).
    pub fn to_line_with(&self, clean: &dyn Fn(&str) -> String) -> String {
        let mut s = format!(
            "trace {} {} {}",
            self.area.word(),
            self.name,
            if self.ok { "ok" } else { "fail" }
        );
        if let Some(h) = self.http {
            let _ = write!(s, " http={h}");
        }
        if let Some(c) = self.class {
            let _ = write!(s, " class={}", c.word());
        }
        if let Some(ms) = self.ms {
            let _ = write!(s, " ms={ms}");
        }
        for (k, v) in &self.fields {
            let _ = write!(s, " {k}={v}");
        }
        if let Some(m) = &self.msg {
            let _ = write!(s, " err={}", clean(m));
        }
        s
    }

    /// Reads a line made by [`Entry::to_line`] (with or without the log prefix before `trace `).
    pub fn from_line(line: &str) -> Option<Entry> {
        let start = line.find("trace ")?;
        let body = &line[start + 6..];
        let (head, msg) = match body.split_once(" err=") {
            Some((h, m)) => (h, Some(m.trim().to_string()).filter(|m| !m.is_empty())),
            None => (body, None),
        };
        let mut it = head.split_whitespace();
        let area = Area::from_word(it.next()?)?;
        let name = safe_name(it.next()?)?;
        let ok = match it.next()? {
            "ok" => true,
            "fail" => false,
            _ => return None,
        };
        let mut e = Entry {
            time: 0,
            area,
            name,
            ok,
            class: None,
            http: None,
            ms: None,
            fields: Vec::new(),
            msg,
            repeats: 1,
        };
        for kv in it {
            let Some((k, v)) = kv.split_once('=') else {
                continue;
            };
            match k {
                "http" => e.http = v.parse().ok(),
                "class" => e.class = ErrorClass::from_word(v),
                "ms" => e.ms = v.parse().ok(),
                _ if e.fields.len() < MAX_FIELDS => {
                    e.fields.push((safe_word(k), safe_value(v)));
                }
                _ => {}
            }
        }
        Some(e)
    }

    /// `12:03:11Z sync/push FAIL http=404 class=status 120 ms fmt=pdf ...` for the report.
    pub fn summary_line(&self) -> String {
        self.summary_with(&|m| m.to_string())
    }

    /// [`Entry::summary_line`] with the message cleaned by `clean`.
    pub fn summary_with(&self, clean: &dyn Fn(&str) -> String) -> String {
        let t = chrono::DateTime::from_timestamp(self.time, 0)
            .map(|d| d.format("%H:%M:%SZ").to_string())
            .unwrap_or_else(|| "--:--:--".into());
        let mut s = format!(
            "{t} {}/{} {}",
            self.area.word(),
            self.name,
            if self.ok { "ok" } else { "FAIL" }
        );
        if let Some(h) = self.http {
            let _ = write!(s, " http={h}");
        }
        if let Some(c) = self.class {
            let _ = write!(s, " class={}", c.word());
        }
        if let Some(ms) = self.ms {
            let _ = write!(s, " {ms} ms");
        }
        for (k, v) in &self.fields {
            let _ = write!(s, " {k}={v}");
        }
        if let Some(m) = &self.msg {
            let _ = write!(s, " err=\"{}\"", clean(m));
        }
        if self.repeats > 1 {
            let _ = write!(s, " [×{}]", self.repeats);
        }
        s
    }
}

/// `[a-z0-9_.\-]` only, at most 32 characters.
pub fn safe_word(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        .take(32)
        .collect::<String>()
        .to_ascii_lowercase()
}

fn safe_name(s: &str) -> Option<String> {
    let w = safe_word(s);
    (!w.is_empty() && w == s).then_some(w)
}

/// A field value: letters, digits and `_ . - : % ,` only, 40 characters at most; spaces become `_`.
pub fn safe_value(s: &str) -> String {
    s.chars()
        .map(|c| if c == ' ' { '_' } else { c })
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | ':' | '%' | ','))
        .take(40)
        .collect()
}

/// A hash of 8 to 64 hex digits, else nothing.
pub fn hex_id(s: &str) -> Option<String> {
    let s = s.trim();
    let ok = (8..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit());
    ok.then(|| s.to_ascii_lowercase())
}

/// A size as a class (`lt100kb` ... `ge1gb`): enough to tell a small book from a large one, without
/// the exact number of bytes of someone's file.
pub fn size_class(bytes: u64) -> &'static str {
    match bytes {
        0 => "empty",
        b if b < 100 << 10 => "lt100kb",
        b if b < 1 << 20 => "lt1mb",
        b if b < 10 << 20 => "lt10mb",
        b if b < 100 << 20 => "lt100mb",
        b if b < 1 << 30 => "lt1gb",
        _ => "ge1gb",
    }
}

/// A count as a class (`0`, `1-9`, `10-99`, `100-999`, `1k-9k`, `10k+`), for numbers that would
/// identify a person when exact.
pub fn count_class(n: u64) -> &'static str {
    match n {
        0 => "0",
        1..=9 => "1-9",
        10..=99 => "10-99",
        100..=999 => "100-999",
        1000..=9999 => "1k-9k",
        _ => "10k+",
    }
}

/// How a device is named in a report: its product name when Bookx knows the model, the model code when
/// it looks like one (`leaf2`, `Kobo_nova`), else `<custom name>`. A name the user typed (the device
/// name field of the plugin, a rename) is never shown.
pub fn device_label(reported: &str) -> String {
    let t = reported.trim();
    if t.is_empty() {
        return "<unknown>".into();
    }
    if let Some(n) = bookx_serve::device_names::known_name(t) {
        return n;
    }
    if looks_like_model_code(t) {
        return t.to_string();
    }
    "<custom name>".into()
}

fn looks_like_model_code(t: &str) -> bool {
    use std::sync::OnceLock;
    static CODE: OnceLock<regex::Regex> = OnceLock::new();
    CODE.get_or_init(|| {
        regex::Regex::new(
            r"^(?:(?:Kobo|Kindle|PB|Onyx|Boox|reMarkable|Tolino|Pocketbook|Likebook|Meebook|Supernote|Bigme|Hisense|Inkbook|Xteink|Bookeen|Nook)[A-Za-z0-9_\-]{0,20}|[a-z]{2,10}[0-9][a-z0-9_]{0,8})$",
        )
        .expect("static pattern")
    })
    .is_match(t)
}

// ---- recording ----

/// A call being measured. Finish it with [`Op::ok`] or [`Op::fail`]; dropped unfinished it is recorded
/// as `aborted`.
pub struct Op {
    entry: Entry,
    start: Instant,
    done: bool,
    persist: bool,
    took: Option<u64>,
}

/// Starts measuring an operation of `area`. `name` is a fixed word of the call site.
pub fn op(area: Area, name: &'static str) -> Op {
    note_started(area, name);
    Op {
        entry: Entry {
            time: 0,
            area,
            name: safe_word(name),
            ok: true,
            class: None,
            http: None,
            ms: None,
            fields: Vec::new(),
            msg: None,
            repeats: 1,
        },
        start: Instant::now(),
        done: false,
        persist: false,
        took: None,
    }
}

/// Runs `f` as the operation `name` of `area`: its duration, and whether it returned `Ok`, are recorded.
/// This is the one-line way to trace a whole function: put its body in the closure.
pub fn traced<T, E: std::fmt::Display>(
    area: Area,
    name: &'static str,
    f: impl FnOnce(&mut Op) -> Result<T, E>,
) -> Result<T, E> {
    let mut o = op(area, name);
    let r = f(&mut o);
    o.result(&r);
    r
}

/// [`traced`] for an error type that only has `Debug`.
pub fn traced_dbg<T, E: std::fmt::Debug>(
    area: Area,
    name: &'static str,
    f: impl FnOnce(&mut Op) -> Result<T, E>,
) -> Result<T, E> {
    let mut o = op(area, name);
    let r = f(&mut o);
    match &r {
        Ok(_) => o.ok(),
        Err(e) => o.fail(&format!("{e:?}")),
    }
    r
}

/// Runs `f` as the operation `name` of `area` and records its duration (it cannot fail).
pub fn timed<T>(area: Area, name: &'static str, f: impl FnOnce(&mut Op) -> T) -> T {
    let mut o = op(area, name);
    let r = f(&mut o);
    o.ok();
    r
}

/// Records the outcome of something that already ran (no duration).
pub fn done<T, E: std::fmt::Display>(area: Area, name: &'static str, r: &Result<T, E>) {
    op(area, name).result(r);
}

/// Records a moment (a server started, a device connected). Always written to the log.
pub fn event(area: Area, name: &'static str) -> Op {
    let mut o = op(area, name);
    o.persist = true;
    o
}

impl Op {
    fn put(&mut self, key: &str, value: String) -> &mut Op {
        let key = safe_word(key);
        if self.entry.fields.len() < MAX_FIELDS && !key.is_empty() && !value.is_empty() {
            self.entry.fields.push((key, value));
        }
        self
    }
    /// A fixed word (`partial-md5`, `held_back`): only a literal in the source can be passed.
    pub fn word(&mut self, key: &str, value: &'static str) -> &mut Op {
        self.put(key, safe_value(value))
    }
    pub fn num(&mut self, key: &str, n: u64) -> &mut Op {
        self.put(key, n.to_string())
    }
    pub fn flag(&mut self, key: &str, on: bool) -> &mut Op {
        self.put(key, if on { "yes" } else { "no" }.into())
    }
    pub fn size_class(&mut self, key: &str, bytes: u64) -> &mut Op {
        self.put(key, size_class(bytes).into())
    }
    pub fn count_class(&mut self, key: &str, n: u64) -> &mut Op {
        self.put(key, count_class(n).into())
    }
    /// A format or an extension (`epub`, `pdf`): letters and digits only.
    pub fn fmt(&mut self, format: &str) -> &mut Op {
        let f: String = format
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(8)
            .collect::<String>()
            .to_ascii_lowercase();
        self.put("fmt", f)
    }
    pub fn lang(&mut self, lang: &str) -> &mut Op {
        let l: String = lang
            .chars()
            .filter(|c| c.is_ascii_alphabetic() || *c == '-')
            .take(12)
            .collect();
        self.put("lang", l)
    }
    /// A hash of bytes (a document id, a sha256): validated as hex.
    pub fn hash(&mut self, key: &str, h: &str) -> &mut Op {
        match hex_id(h) {
            Some(h) => self.put(key, h),
            None => self,
        }
    }
    /// A position as a share of the book (`0.373`).
    pub fn ratio(&mut self, key: &str, v: f64) -> &mut Op {
        if v.is_finite() {
            self.put(key, format!("{:.3}", v.clamp(0.0, 1.0)))
        } else {
            self
        }
    }
    /// The id of a book in the library (a number: the report can name the book if the user allows it).
    pub fn book(&mut self, id: u64) -> &mut Op {
        self.num("book", id)
    }
    /// The device as [`device_label`] names it.
    pub fn device(&mut self, reported: &str) -> &mut Op {
        self.put("device", safe_value(&device_label(reported)))
    }
    /// The format (extension) and the size class of a file; nothing else about it (never its name).
    pub fn file(&mut self, path: &std::path::Path) -> &mut Op {
        if let Some(e) = path.extension().and_then(|e| e.to_str()) {
            self.fmt(e);
        }
        if let Ok(m) = std::fs::metadata(path) {
            self.size_class("size", m.len());
        }
        self
    }
    pub fn http(&mut self, status: u16) -> &mut Op {
        self.entry.http = Some(status);
        self
    }
    /// The duration, when it was measured elsewhere (a stall of the main loop).
    pub fn took(&mut self, ms: u64) -> &mut Op {
        self.took = Some(ms);
        self
    }
    /// Not written to the log, even for an event (it happens often).
    pub fn quiet(&mut self) -> &mut Op {
        self.persist = false;
        self
    }
    /// Written to the log even when it worked and was quick.
    pub fn keep(&mut self) -> &mut Op {
        self.persist = true;
        self
    }
    pub fn ok(&mut self) {
        self.finish(true, None, None);
    }
    /// Finishes with success and gives the entry back to the caller's `let _`, for tests.
    #[cfg(test)]
    pub fn ok_dropped(mut self) -> Op {
        self.ok();
        self
    }
    /// Failed: the class and the HTTP status come from the message; the message is cleaned and cut.
    pub fn fail(&mut self, msg: &str) {
        let (class, status) = classify_error(msg);
        if self.entry.http.is_none() {
            self.entry.http = status;
        }
        self.finish(false, Some(class), Some(msg));
    }
    /// Failed with a known class.
    pub fn fail_class(&mut self, class: ErrorClass) {
        self.finish(false, Some(class), None);
    }
    /// `ok` or `fail` from a result.
    pub fn result<T, E: std::fmt::Display>(&mut self, r: &Result<T, E>) {
        match r {
            Ok(_) => self.ok(),
            Err(e) => self.fail(&e.to_string()),
        }
    }

    fn finish(&mut self, ok: bool, class: Option<ErrorClass>, msg: Option<&str>) {
        if self.done {
            return;
        }
        self.done = true;
        let mut e = self.entry.clone();
        e.ok = ok;
        e.class = class;
        e.ms = Some(self.took.unwrap_or_else(|| self.start.elapsed().as_millis() as u64));
        e.msg = msg.map(clean_message).filter(|m| !m.is_empty());
        e.time = chrono::Utc::now().timestamp();
        let slow = e.ms.is_some_and(|m| m >= SLOW_MS);
        if self.persist || !ok || slow {
            tracing::info!(target: "bookx_crumb", "{}", e.to_line());
        }
        record(e);
    }
}

impl Drop for Op {
    fn drop(&mut self) {
        if !self.done {
            self.finish(false, Some(ErrorClass::Aborted), None);
        }
    }
}

/// The message of a failure as the recorder keeps it: addresses reduced to their shape, one line, at
/// most [`MSG_BYTES`]. Paths and titles are cleaned where the scrubber is known (when the entry is
/// written to the log or shown in a report, `diag_facts::clean_full`).
pub fn clean_message(msg: &str) -> String {
    let one: String = msg
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    let mut s = crate::diag_addr::clean_urls(&one);
    if s.len() > MSG_BYTES {
        let mut c = MSG_BYTES;
        while !s.is_char_boundary(c) {
            c -= 1;
        }
        s.truncate(c);
    }
    s
}

struct Store {
    q: VecDeque<Entry>,
}

static STORE: Mutex<Store> = Mutex::new(Store { q: VecDeque::new() });
static LAST_OP: Mutex<Option<(Area, &'static str, Instant)>> = Mutex::new(None);

fn note_started(area: Area, name: &'static str) {
    if let Ok(mut l) = LAST_OP.try_lock() {
        *l = Some((area, name, Instant::now()));
    }
}

static ACTIVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);

/// Says which area the user is in now (a window or page was opened): the stall detector and the
/// report use it. Cheap; call it when a window is shown.
pub fn set_active(area: Area) {
    if let Some(i) = Area::ALL.iter().position(|a| *a == area) {
        ACTIVE.store(i, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The area the user is in, if known.
pub fn active() -> Option<Area> {
    Area::ALL
        .get(ACTIVE.load(std::sync::atomic::Ordering::Relaxed))
        .copied()
}

/// The operation that started last (for the stall detector): area, name.
pub fn last_started() -> Option<(Area, &'static str)> {
    LAST_OP.lock().ok().and_then(|l| l.map(|(a, n, _)| (a, n)))
}

fn push(store: &mut Store, mut e: Entry) {
    // the same thing again right after itself is one entry with a count
    if let Some(last) = store.q.iter_mut().rev().find(|x| x.area == e.area) {
        if last.name == e.name && last.ok == e.ok && last.class == e.class && last.http == e.http && last.fields == e.fields && last.msg == e.msg {
            last.repeats = last.repeats.saturating_add(1);
            last.time = e.time;
            last.ms = e.ms;
            return;
        }
    }
    e.repeats = e.repeats.max(1);
    let same = store.q.iter().filter(|x| x.area == e.area).count();
    if same >= PER_AREA {
        if let Some(i) = store.q.iter().position(|x| x.area == e.area) {
            store.q.remove(i);
        }
    }
    if store.q.len() >= TOTAL {
        store.q.pop_front();
    }
    store.q.push_back(e);
}

/// Adds a finished entry.
pub fn record(e: Entry) {
    push(&mut STORE.lock().unwrap_or_else(|p| p.into_inner()), e);
}

/// The entries of this run, oldest first.
pub fn snapshot() -> Vec<Entry> {
    STORE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .q
        .iter()
        .cloned()
        .collect()
}

/// The entries the log still holds (breadcrumb lines), oldest first.
pub fn from_log(log: &[String]) -> Vec<Entry> {
    log.iter()
        .filter_map(|l| {
            let mut e = Entry::from_line(l)?;
            e.time = crate::diag_problems::parse(l)
                .and_then(|p| chrono::DateTime::parse_from_rfc3339(p.time).ok())
                .map_or(0, |d| d.timestamp());
            Some(e)
        })
        .collect()
}

/// This run's entries and, before them, the ones only the log (the last run's) has.
pub fn merged(log: &[String]) -> Vec<Entry> {
    let mem = snapshot();
    let first = mem.first().map_or(i64::MAX, |e| e.time);
    let mut out: Vec<Entry> = from_log(log)
        .into_iter()
        .filter(|e| e.time < first)
        .collect();
    out.extend(mem);
    out
}

/// The newest failure that nothing of the same area and name healed afterwards.
pub fn last_failure(entries: &[Entry]) -> Option<&Entry> {
    entries.iter().enumerate().rev().find_map(|(i, e)| {
        if e.ok {
            return None;
        }
        let healed = entries[i + 1..]
            .iter()
            .any(|n| n.ok && n.area == e.area && n.name == e.name);
        (!healed).then_some(e)
    })
}

const TAIL_FILE: &str = "trace-last-session.txt";
const TAIL_PER_AREA: usize = 10;

/// Writes the newest entries of every area to `dir/trace-last-session.txt` (0600), cleaned by `clean`.
/// Called when the app is closed and by the panic hook.
pub fn persist_tail(dir: &std::path::Path, clean: &dyn Fn(&str) -> String) -> std::io::Result<()> {
    let all = snapshot();
    let mut lines: Vec<String> = Vec::new();
    for a in Area::ALL {
        let mine: Vec<&Entry> = all.iter().filter(|e| e.area == *a).collect();
        let skip = mine.len().saturating_sub(TAIL_PER_AREA);
        for e in &mine[skip..] {
            lines.push(format!("{} {}", e.time, e.to_line_with(clean)));
        }
    }
    if lines.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(dir.join(TAIL_FILE))?;
    f.write_all(lines.join("\n").as_bytes())?;
    f.write_all(b"\n")
}

/// The tail the last session left (oldest first by time), or nothing.
pub fn load_tail(dir: &std::path::Path) -> Vec<Entry> {
    let Ok(t) = std::fs::read_to_string(dir.join(TAIL_FILE)) else {
        return Vec::new();
    };
    let mut v: Vec<Entry> = t
        .lines()
        .filter_map(|l| {
            let (ts, rest) = l.split_once(' ')?;
            let mut e = Entry::from_line(rest)?;
            e.time = ts.parse().ok()?;
            Some(e)
        })
        .collect();
    v.sort_by_key(|e| e.time);
    v
}

// ---- state snapshots ----

/// What a feature tells about its state.
pub type Provider = Box<dyn Fn() -> Vec<(String, Fact)>>;

thread_local! {
    static PROVIDERS: RefCell<Vec<(Area, &'static str, Provider)>> = const { RefCell::new(Vec::new()) };
}

/// Registers the snapshot of a feature (on the thread that builds the report: the main thread). A
/// second registration with the same `(area, id)` replaces the first.
pub fn register(area: Area, id: &'static str, provider: Provider) {
    PROVIDERS.with(|p| {
        let mut p = p.borrow_mut();
        p.retain(|(a, i, _)| !(*a == area && *i == id));
        p.push((area, id, provider));
    });
}

/// The facts of every registered feature, by area.
pub fn collect() -> Vec<(Area, Vec<(String, Fact)>)> {
    PROVIDERS.with(|p| {
        let p = p.borrow();
        let mut out: Vec<(Area, Vec<(String, Fact)>)> = Vec::new();
        for a in Area::ALL {
            let mut rows = Vec::new();
            for (pa, _, f) in p.iter() {
                if pa == a {
                    rows.extend(f());
                }
            }
            if !rows.is_empty() {
                out.push((*a, rows));
            }
        }
        out
    })
}

#[cfg(test)]
pub fn clear_for_test() {
    STORE.lock().unwrap_or_else(|p| p.into_inner()).q.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(area: Area, name: &str, ok: bool) -> Entry {
        Entry {
            time: 1_700_000_000,
            area,
            name: name.into(),
            ok,
            class: (!ok).then_some(ErrorClass::Status),
            http: (!ok).then_some(404),
            ms: Some(120),
            repeats: 1,
            fields: vec![
                ("fmt".into(), "pdf".into()),
                ("size".into(), "lt10mb".into()),
                ("doc".into(), "0123456789abcdef0123456789abcdef".into()),
                ("local".into(), "0.373".into()),
            ],
            msg: (!ok).then(|| "the server answered 404".into()),
        }
    }

    #[test]
    fn a_failed_push_reads_at_a_glance() {
        let e = entry(Area::Sync, "push", false);
        assert_eq!(
            e.to_line(),
            "trace sync push fail http=404 class=status ms=120 fmt=pdf size=lt10mb doc=0123456789abcdef0123456789abcdef local=0.373 err=the server answered 404"
        );
        assert!(e.summary_line().contains("sync/push FAIL http=404 class=status 120 ms fmt=pdf"));
    }

    #[test]
    fn a_line_reads_back_to_the_same_entry() {
        for ok in [true, false] {
            let mut e = entry(Area::Convert, "to_kepub", ok);
            e.time = 0;
            let line = format!("2026-11-23T10:00:00Z  INFO bookx_crumb: {}", e.to_line());
            assert_eq!(Entry::from_line(&line).unwrap(), e);
        }
        assert!(Entry::from_line("INFO bookx_crumb: library opened: 5 books").is_none());
        assert!(Entry::from_line("trace nowhere push ok").is_none());
        assert!(Entry::from_line("trace sync push maybe").is_none());
    }

    #[test]
    fn errors_are_classified() {
        for (msg, class, status) in [
            ("the server answered 404", ErrorClass::Status, Some(404)),
            ("the server answered 401", ErrorClass::Unauthorized, Some(401)),
            ("HTTP status code 403 from the server", ErrorClass::Unauthorized, Some(403)),
            ("the server did not accept the user name and password", ErrorClass::Unauthorized, None),
            ("dns error: failed to lookup address information", ErrorClass::Dns, None),
            ("connection refused (os error 111)", ErrorClass::Refused, None),
            ("operation timed out", ErrorClass::Timeout, None),
            ("invalid peer certificate: UnknownIssuer", ErrorClass::Tls, None),
            ("Network is unreachable (os error 101)", ErrorClass::Unreachable, None),
            ("the address must start with http:// or https://", ErrorClass::BadAddress, None),
            ("the server's answer: expected value at line 1", ErrorClass::BadAnswer, None),
            ("Could not start sharing on port 30260: Address already in use", ErrorClass::PortInUse, None),
            ("No space left on device (os error 28)", ErrorClass::DiskFull, None),
            ("database is locked", ErrorClass::Locked, None),
            ("sha256 mismatch for the voice model", ErrorClass::HashMismatch, None),
            ("the device is not mounted any more", ErrorClass::NotMounted, None),
            ("Permission denied (os error 13)", ErrorClass::Denied, None),
            ("the file is not a zip archive", ErrorClass::Corrupt, None),
            ("this book is protected by DRM", ErrorClass::Unsupported, None),
            ("no Piper voice is downloaded", ErrorClass::Missing, None),
            ("something odd", ErrorClass::Other, None),
        ] {
            assert_eq!(classify_error(msg), (class, status), "{msg}");
        }
        for (c, w) in CLASSES {
            assert_eq!(ErrorClass::from_word(w), Some(*c));
        }
    }

    #[test]
    fn a_stored_message_has_no_address_and_one_line() {
        let m = clean_message(
            "talking to http://10.0.0.7:30260/api/koreader: cannot open /home/alice/Books/Secret Title.pdf\nsecond line",
        );
        assert!(!m.contains("10.0.0.7"), "{m}");
        assert!(!m.contains("second"));
        assert!(clean_message(&"x".repeat(1000)).len() <= MSG_BYTES);
    }

    #[test]
    fn field_values_cannot_carry_text() {
        assert_eq!(safe_value("Alice's Book/Title @ home"), "Alices_BookTitle__home");
        assert_eq!(safe_value(&"a".repeat(100)).len(), 40);
        let mut o = op(Area::Import, "t_fields");
        o.fmt("EPUB!/..").hash("doc", "not a hash").hash("sha", "0123456789ABCDEF").ratio("p", f64::NAN).ratio("q", 3.0);
        assert_eq!(o.entry.field("fmt"), Some("epub"));
        assert!(o.entry.field("doc").is_none());
        assert_eq!(o.entry.field("sha"), Some("0123456789abcdef"));
        assert_eq!(o.entry.field("q"), Some("1.000"));
        o.ok();
    }

    #[test]
    fn sizes_and_counts_are_told_as_classes() {
        assert_eq!(size_class(0), "empty");
        assert_eq!(size_class(50_000), "lt100kb");
        assert_eq!(size_class(2 << 20), "lt10mb");
        assert_eq!(size_class(5 << 30), "ge1gb");
        assert_eq!(count_class(7), "1-9");
        assert_eq!(count_class(1234), "1k-9k");
        assert_eq!(count_class(50_000), "10k+");
    }

    #[test]
    fn devices_are_named_by_product_or_model_code_never_by_a_custom_name() {
        assert_eq!(device_label("Onyx Scubapilot-IDP"), "Boox Leaf 2");
        assert_eq!(device_label("Kobo_nova"), "Kobo Clara HD");
        assert_eq!(device_label("leaf9x"), "leaf9x");
        assert_eq!(device_label("Alice's Kobo"), "<custom name>");
        assert_eq!(device_label("SENT-DEVNAME-777"), "<custom name>");
        assert_eq!(device_label("Alice"), "<custom name>");
        assert_eq!(device_label(""), "<unknown>");
    }

    #[test]
    fn the_same_thing_again_is_counted_not_listed() {
        let mut s = Store { q: VecDeque::new() };
        push(&mut s, entry(Area::Reader, "open_book", true));
        for _ in 0..50 {
            push(&mut s, entry(Area::Reader, "load_chapter", true));
        }
        assert_eq!(s.q.len(), 2);
        assert_eq!(s.q.back().unwrap().repeats, 50);
        assert!(s.q.back().unwrap().summary_line().ends_with("[×50]"));
        // another area in between does not break the run
        push(&mut s, entry(Area::Sync, "pull", true));
        push(&mut s, entry(Area::Reader, "load_chapter", true));
        assert_eq!(s.q.iter().filter(|e| e.area == Area::Reader).count(), 2);
        assert_eq!(s.q.iter().find(|e| e.name == "load_chapter").unwrap().repeats, 51);
    }

    #[test]
    fn the_recorder_keeps_the_newest_per_area_and_in_all() {
        let mut s = Store { q: VecDeque::new() };
        for n in 0..PER_AREA + 10 {
            let mut e = entry(Area::Server, "requests", true);
            e.fields = vec![("n".into(), n.to_string())];
            push(&mut s, e);
        }
        push(&mut s, entry(Area::Sync, "pull", true));
        assert_eq!(s.q.iter().filter(|e| e.area == Area::Server).count(), PER_AREA);
        assert_eq!(s.q.front().unwrap().field("n"), Some("10"));
        for a in Area::ALL.iter().cycle().take(TOTAL * 2) {
            push(&mut s, entry(*a, "x", true));
        }
        assert!(s.q.len() <= TOTAL);
    }

    #[test]
    fn an_operation_records_its_duration_outcome_and_fields() {
        let mut o = op(Area::Convert, "zz_convert_probe");
        o.fmt("epub").word("to", "kepub").size_class("size", 3 << 20);
        o.fail("could not convert the book (the file is not a zip archive)");
        drop(o);
        let e = snapshot()
            .into_iter()
            .rev()
            .find(|e| e.name == "zz_convert_probe")
            .unwrap();
        assert!(!e.ok);
        assert_eq!(e.class, Some(ErrorClass::Corrupt));
        assert_eq!((e.field("fmt"), e.field("to"), e.field("size")), (Some("epub"), Some("kepub"), Some("lt10mb")));
        assert!(e.ms.is_some() && e.time > 0);
    }

    #[test]
    fn an_operation_dropped_unfinished_is_aborted() {
        {
            let _o = op(Area::Reader, "zz_dropped_probe");
        }
        let e = snapshot().into_iter().rev().find(|e| e.name == "zz_dropped_probe").unwrap();
        assert!(!e.ok);
        assert_eq!(e.class, Some(ErrorClass::Aborted));
    }

    #[test]
    fn a_failure_that_was_healed_is_not_the_last_failure() {
        let fail = entry(Area::Sync, "push", false);
        let fixed = entry(Area::Sync, "push", true);
        let other = entry(Area::Server, "start", false);
        assert!(last_failure(&[fail.clone(), fixed]).is_none());
        assert_eq!(last_failure(&[fail.clone(), other.clone()]).unwrap().area, Area::Server);
        assert_eq!(last_failure(&[other, fail]).unwrap().area, Area::Sync);
    }

    #[test]
    fn the_log_and_the_run_are_merged_without_doubles() {
        let mut old = entry(Area::Backup, "run", false);
        old.time = 1_000;
        let log = vec![format!("2023-11-14T22:13:20Z  INFO bookx_crumb: {}", old.to_line())];
        let got = from_log(&log);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].time, 1_700_000_000);
        assert_eq!(got[0].area, Area::Backup);
    }

    #[test]
    fn the_tail_survives_a_restart_in_a_private_file() {
        let dir = std::env::temp_dir().join(format!("bookx-trace-{}", uuid::Uuid::new_v4()));
        let mut o = op(Area::Tts, "zz_tail_probe");
        o.fmt("wav");
        o.fail("no Piper voice is downloaded");
        drop(o);
        persist_tail(&dir, &|s| s.replace("Piper", "<engine>")).unwrap();
        let got = load_tail(&dir);
        let e = got.iter().find(|e| e.name == "zz_tail_probe").unwrap();
        assert_eq!(e.class, Some(ErrorClass::Missing));
        assert!(e.msg.as_deref().unwrap().contains("<engine>"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = std::fs::metadata(dir.join(TAIL_FILE)).unwrap().permissions().mode();
            assert_eq!(m & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn providers_report_by_area_and_a_second_registration_replaces_the_first() {
        register(Area::Tts, "t", Box::new(|| vec![("engine".into(), Fact::word("kokoro"))]));
        register(Area::Tts, "t", Box::new(|| vec![("engine".into(), Fact::word("piper"))]));
        register(Area::Backup, "b", Box::new(|| vec![("places".into(), Fact::count(2))]));
        let c = collect();
        let tts = c.iter().find(|(a, _)| *a == Area::Tts).unwrap();
        assert_eq!(tts.1.len(), 1);
        assert!(c.iter().any(|(a, _)| *a == Area::Backup));
    }

    #[test]
    fn every_area_has_a_unique_word_and_a_label() {
        let mut words = std::collections::BTreeSet::new();
        for a in Area::ALL {
            assert!(words.insert(a.word()));
            assert!(!a.label().is_empty());
            assert_eq!(Area::from_word(a.word()), Some(*a));
        }
    }
}
