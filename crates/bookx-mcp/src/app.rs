//! The state behind the protocol: configuration, the catalogue (read side and guarded write side),
//! the audit log, the rate limiter and the list of connected clients.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use bookx_core::library::{Book, Library};
use bookx_sources::fetch::Fetcher;
use bookx_store::SqliteStore;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::text::TextCache;

/// How the server may change the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritePolicy {
    /// Read only: write tools are not even listed.
    Off,
    /// Each change takes the library's single-writer lock for its duration, and fails with a clear
    /// message while another Bookx (the app) holds it.
    Locked,
    /// This process is the app itself and owns the lock already.
    InApp,
}

#[derive(Clone)]
pub struct Config {
    pub root: PathBuf,
    pub write: WritePolicy,
    /// Folders `add_book_from_path` may read from.
    pub allow_dirs: Vec<PathBuf>,
    /// Network tools (`fetch_metadata`) are offered only when this is set and a fetcher is given.
    pub allow_network: bool,
    pub fetcher: Option<Arc<dyn Fetcher>>,
    /// The reading statistics file (`stats.json` of the app), when there is one.
    pub stats_path: Option<PathBuf>,
    /// Items per page in `*/list` answers and in tools' default `limit`.
    pub page_size: usize,
    /// Most characters of book text in one answer.
    pub max_text_chars: usize,
    /// At most `.0` tool calls per `.1`; `None` switches the limit off.
    pub rate_limit: Option<(u32, Duration)>,
    /// A tool call that runs longer stops (cooperatively) and says so.
    pub call_timeout: Duration,
    /// Name used for the lock and in the audit log.
    pub app_name: String,
    /// How often the stdio transport looks for library changes to tell subscribed clients about
    /// (`notifications/resources/*`); zero switches the watcher off.
    pub watch_interval: Duration,
}

impl Config {
    pub fn new(root: &Path) -> Config {
        Config {
            root: root.to_path_buf(),
            write: WritePolicy::Off,
            allow_dirs: Vec::new(),
            allow_network: false,
            fetcher: None,
            stats_path: None,
            page_size: 50,
            max_text_chars: 20_000,
            rate_limit: Some((240, Duration::from_secs(60))),
            call_timeout: Duration::from_secs(30),
            app_name: "bookx-mcp".into(),
            watch_interval: Duration::from_secs(2),
        }
    }

    pub fn can_write(&self) -> bool {
        self.write != WritePolicy::Off
    }
}

/// A tool failed in a way the model should see (as `isError`, not as a protocol error).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolError(pub String);

impl<E: std::fmt::Display> From<E> for ToolError {
    fn from(e: E) -> ToolError {
        ToolError(format!("{e:#}"))
    }
}

pub fn fail<T>(msg: impl Into<String>) -> Result<T, ToolError> {
    Err(ToolError(msg.into()))
}

/// What a tool returns.
pub struct ToolOutput {
    pub structured: Value,
    /// The text block; the pretty JSON of `structured` when `None`.
    pub text: Option<String>,
}

impl ToolOutput {
    pub fn json(v: Value) -> ToolOutput {
        ToolOutput {
            structured: v,
            text: None,
        }
    }
}

// -- reading the catalogue ----------------------------------------------------------------------

struct CState {
    store: Option<Arc<SqliteStore>>,
    version: Option<i64>,
    books: Arc<Vec<Book>>,
}

/// Read access to the catalogue that never takes the writer lock: it follows the database like the
/// sharing server does (re-reading only when something was committed).
pub struct Catalog {
    root: PathBuf,
    state: Mutex<CState>,
}

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Catalog {
    pub fn new(root: &Path) -> Catalog {
        Catalog {
            root: root.to_path_buf(),
            state: Mutex::new(CState {
                store: None,
                version: None,
                books: Arc::new(Vec::new()),
            }),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn open_store(&self, st: &mut CState) -> Result<Arc<SqliteStore>> {
        if let Some(s) = &st.store {
            return Ok(s.clone());
        }
        if !bookx_store::has_library(&self.root) {
            bail!("there is no Bookx library in the configured folder");
        }
        let s = Arc::new(SqliteStore::open(&bookx_store::library_db_path(
            &self.root,
        ))?);
        st.store = Some(s.clone());
        Ok(s)
    }

    /// The store, for the full-text index, collections and reading status.
    pub fn store(&self) -> Result<Arc<SqliteStore>> {
        let mut st = lock(&self.state);
        self.open_store(&mut st)
    }

    /// The books as of the last commit.
    pub fn books(&self) -> Result<Arc<Vec<Book>>> {
        let mut st = lock(&self.state);
        let store = self.open_store(&mut st)?;
        let current = store.data_version();
        if st.version != Some(current) {
            st.books = Arc::new(store.all_books()?);
            st.version = Some(current);
        }
        Ok(st.books.clone())
    }

    pub fn book(&self, id: u64) -> Result<Book, ToolError> {
        self.books()?
            .iter()
            .find(|b| b.id == id)
            .cloned()
            .ok_or_else(|| ToolError(format!("there is no book with id {id}")))
    }
}

// -- audit --------------------------------------------------------------------------------------

#[derive(Default)]
pub struct Audit {
    path: Option<PathBuf>,
    recent: Mutex<VecDeque<Value>>,
}

const SECRET_KEYS: [&str; 6] = [
    "token",
    "password",
    "secret",
    "authorization",
    "key",
    "cookie",
];

/// Arguments for the log: secrets blanked, long strings cut.
pub fn redact(v: &Value) -> Value {
    match v {
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| {
                    let l = k.to_lowercase();
                    if SECRET_KEYS.iter().any(|s| l.contains(s)) {
                        (k.clone(), json!("[redacted]"))
                    } else {
                        (k.clone(), redact(v))
                    }
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().take(20).map(redact).collect()),
        Value::String(s) if s.chars().count() > 200 => {
            Value::String(format!("{}…", s.chars().take(200).collect::<String>()))
        }
        other => other.clone(),
    }
}

impl Audit {
    pub fn new(root: &Path) -> Audit {
        Audit {
            path: Some(
                root.join(bookx_core::history::STATE_DIR)
                    .join("mcp-audit.log"),
            ),
            recent: Mutex::new(VecDeque::new()),
        }
    }

    pub fn record(&self, client: &str, tool: &str, args: &Value, ok: bool, summary: &str) {
        let entry = json!({
            "at": Utc::now().to_rfc3339(),
            "client": client,
            "tool": tool,
            "arguments": redact(args),
            "ok": ok,
            "summary": summary,
        });
        if let Some(p) = &self.path {
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
            {
                use std::io::Write;
                let _ = writeln!(f, "{entry}");
            }
        }
        let mut r = lock(&self.recent);
        r.push_back(entry);
        while r.len() > 100 {
            r.pop_front();
        }
    }

    pub fn recent(&self) -> Vec<Value> {
        lock(&self.recent).iter().cloned().collect()
    }
}

// -- rate limit ---------------------------------------------------------------------------------

pub struct Limiter {
    limit: Option<(u32, Duration)>,
    calls: Mutex<VecDeque<Instant>>,
}

impl Limiter {
    pub fn new(limit: Option<(u32, Duration)>) -> Limiter {
        Limiter {
            limit,
            calls: Mutex::new(VecDeque::new()),
        }
    }

    /// Counts a call at `now`; `Err(wait)` when the budget is used up.
    pub fn check(&self, now: Instant) -> Result<(), Duration> {
        let Some((max, window)) = self.limit else {
            return Ok(());
        };
        let mut c = lock(&self.calls);
        while c.front().is_some_and(|t| now.duration_since(*t) >= window) {
            c.pop_front();
        }
        if c.len() as u32 >= max {
            let oldest = *c.front().expect("not empty");
            return Err(window.saturating_sub(now.duration_since(oldest)));
        }
        c.push_back(now);
        Ok(())
    }
}

// -- clients ------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ClientStat {
    pub session: String,
    pub name: String,
    pub version: String,
    pub since: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub calls: u64,
}

// -- the app ------------------------------------------------------------------------------------

pub struct App {
    pub cfg: Config,
    pub catalog: Catalog,
    pub texts: TextCache,
    pub audit: Audit,
    pub limiter: Limiter,
    clients: Mutex<Vec<ClientStat>>,
    /// Lets the host switch the whole server off (every call then fails).
    pub stopped: AtomicBool,
}

impl App {
    pub fn new(cfg: Config) -> Arc<App> {
        Arc::new(App {
            catalog: Catalog::new(&cfg.root),
            texts: TextCache::new(),
            audit: Audit::new(&cfg.root),
            limiter: Limiter::new(cfg.rate_limit),
            clients: Mutex::new(Vec::new()),
            stopped: AtomicBool::new(false),
            cfg,
        })
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    pub fn client_seen(&self, session: &str, name: &str, version: &str, call: bool) {
        let mut c = lock(&self.clients);
        let now = Utc::now();
        match c.iter_mut().find(|s| s.session == session) {
            Some(s) => {
                s.last_seen = now;
                if !name.is_empty() {
                    s.name = name.into();
                    s.version = version.into();
                }
                s.calls += u64::from(call);
            }
            None => c.push(ClientStat {
                session: session.into(),
                name: name.into(),
                version: version.into(),
                since: now,
                last_seen: now,
                calls: u64::from(call),
            }),
        }
    }

    pub fn client_gone(&self, session: &str) {
        lock(&self.clients).retain(|s| s.session != session);
    }

    /// Clients seen in the last `idle` (all of them when `None`).
    pub fn clients(&self, idle: Option<Duration>) -> Vec<ClientStat> {
        let now = Utc::now();
        lock(&self.clients)
            .iter()
            .filter(|s| {
                idle.is_none_or(|d| {
                    (now - s.last_seen)
                        .to_std()
                        .map(|age| age <= d)
                        .unwrap_or(true)
                })
            })
            .cloned()
            .collect()
    }

    /// Runs `f` on a freshly loaded library with the guarded write rules, saves, and lets go of the
    /// lock. Every metadata edit and import made through `Library` lands in the operation log and
    /// can be undone in the app.
    pub fn with_library<T>(
        &self,
        f: impl FnOnce(&mut Library, &SqliteStore) -> Result<T, ToolError>,
    ) -> Result<T, ToolError> {
        let _lock = match self.cfg.write {
            WritePolicy::Off => {
                return fail("this server is read-only (start it with write access to change the library)")
            }
            WritePolicy::Locked => match bookx_store::guard::acquire(&self.cfg.root, &self.cfg.app_name)? {
                bookx_store::guard::Lock::Acquired(l) => Some(l),
                bookx_store::guard::Lock::Busy(h) => {
                    return fail(format!(
                        "the library is open in {}; writes are refused while it is. Use the server inside the app, or close it",
                        h.describe()
                    ))
                }
            },
            WritePolicy::InApp => None,
        };
        let opened = bookx_store::open_library(&self.cfg.root).map_err(|e| anyhow!("{e:#}"))?;
        let mut lib = opened.library;
        let out = f(&mut lib, &opened.store)?;
        lib.save()?;
        Ok(out)
    }

    /// Replaces the library path in a message so the model never sees where the library lives.
    pub fn scrub(&self, msg: &str) -> String {
        msg.replace(self.cfg.root.to_string_lossy().as_ref(), "<library>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_redacted_and_long_text_is_cut() {
        let v = json!({"title":"x","api_token":"abc","nested":{"Password":"p","ok":1},
            "long":"y".repeat(500),"list":[{"authorization":"Bearer z"}]});
        let r = redact(&v);
        assert_eq!(r["api_token"], "[redacted]");
        assert_eq!(r["nested"]["Password"], "[redacted]");
        assert_eq!(r["nested"]["ok"], 1);
        assert_eq!(r["list"][0]["authorization"], "[redacted]");
        assert!(r["long"].as_str().unwrap().chars().count() <= 201);
        assert!(!r.to_string().contains("abc"));
    }

    #[test]
    fn limiter_counts_a_window() {
        let l = Limiter::new(Some((2, Duration::from_secs(10))));
        let t = Instant::now();
        assert!(l.check(t).is_ok());
        assert!(l.check(t).is_ok());
        let wait = l.check(t).unwrap_err();
        assert!(wait <= Duration::from_secs(10) && wait > Duration::ZERO);
        assert!(l.check(t + Duration::from_secs(11)).is_ok());
        assert!(Limiter::new(None).check(t).is_ok());
    }
}
