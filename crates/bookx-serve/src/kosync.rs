//! KOReader progress-sync (KOSync) compatible server logic.
//!
//! Implemented from the public protocol description:
//!
//! | Route | Meaning |
//! |---|---|
//! | `POST /users/create` | body `{"username","password"}` (password is the MD5 hex the client already made) |
//! | `GET /users/auth` | checks the `x-auth-user` / `x-auth-key` headers |
//! | `PUT /syncs/progress` | stores `{document, progress, percentage, device, device_id}` |
//! | `GET /syncs/progress/:document` | returns the stored progress plus `timestamp` |
//! | `GET /healthcheck` | liveness |
//!
//! [`KoSync::handle`] is a pure function of a [`Request`], so the whole protocol is testable without
//! a socket.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::{constant_time_eq, split_path, Request, Response};

/// MD5 hex digest, the "key" KOReader sends for a password.
pub fn md5_hex(password: &str) -> String {
    format!("{:x}", md5::compute(password.as_bytes()))
}

/// One stored reading position.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    pub document: String,
    pub progress: String,
    pub percentage: f64,
    pub device: String,
    pub device_id: String,
    /// Unix seconds when the server stored it.
    pub timestamp: i64,
}

/// Persistence behind the protocol. Implementations must be thread safe.
pub trait KoSyncStore: Send + Sync {
    /// Registers a user. `Ok(false)` if the name is taken.
    fn create_user(&self, user: &str, key: &str) -> Result<bool>;
    /// True if `user` exists and `key` matches (constant-time comparison).
    fn check_user(&self, user: &str, key: &str) -> bool;
    fn get_progress(&self, user: &str, document: &str) -> Option<Progress>;
    fn put_progress(&self, user: &str, progress: Progress) -> Result<()>;
    fn user_count(&self) -> usize;
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct State {
    users: HashMap<String, String>,
    /// user -> document -> progress
    progress: HashMap<String, HashMap<String, Progress>>,
}

impl State {
    fn create_user(&mut self, user: &str, key: &str) -> bool {
        if self.users.contains_key(user) {
            return false;
        }
        self.users.insert(user.to_string(), key.to_string());
        true
    }

    fn check_user(&self, user: &str, key: &str) -> bool {
        match self.users.get(user) {
            Some(k) => constant_time_eq(k.as_bytes(), key.as_bytes()),
            None => {
                // spend comparable time so unknown users are not distinguishable by timing
                let _ = constant_time_eq(b"", key.as_bytes());
                false
            }
        }
    }
}

/// In-memory store, for tests and ephemeral servers.
#[derive(Default)]
pub struct MemoryStore {
    state: Mutex<State>,
}

impl MemoryStore {
    pub fn new() -> MemoryStore {
        MemoryStore::default()
    }
}

fn lock(m: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl KoSyncStore for MemoryStore {
    fn create_user(&self, user: &str, key: &str) -> Result<bool> {
        Ok(lock(&self.state).create_user(user, key))
    }
    fn check_user(&self, user: &str, key: &str) -> bool {
        lock(&self.state).check_user(user, key)
    }
    fn get_progress(&self, user: &str, document: &str) -> Option<Progress> {
        lock(&self.state)
            .progress
            .get(user)
            .and_then(|m| m.get(document))
            .cloned()
    }
    fn put_progress(&self, user: &str, progress: Progress) -> Result<()> {
        lock(&self.state)
            .progress
            .entry(user.to_string())
            .or_default()
            .insert(progress.document.clone(), progress);
        Ok(())
    }
    fn user_count(&self) -> usize {
        lock(&self.state).users.len()
    }
}

/// JSON file backed store; every change is written atomically (temp file + rename).
pub struct JsonFileStore {
    path: PathBuf,
    state: Mutex<State>,
}

impl JsonFileStore {
    /// Opens the file, or starts empty if it does not exist yet.
    pub fn open(path: &Path) -> Result<JsonFileStore> {
        let state = if path.exists() {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            if text.trim().is_empty() {
                State::default()
            } else {
                serde_json::from_str(&text).context("corrupt KOSync store")?
            }
        } else {
            State::default()
        };
        Ok(JsonFileStore {
            path: path.to_path_buf(),
            state: Mutex::new(state),
        })
    }

    fn save(&self, state: &State) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(state)?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

impl KoSyncStore for JsonFileStore {
    fn create_user(&self, user: &str, key: &str) -> Result<bool> {
        let mut st = lock(&self.state);
        if !st.create_user(user, key) {
            return Ok(false);
        }
        self.save(&st)?;
        Ok(true)
    }
    fn check_user(&self, user: &str, key: &str) -> bool {
        lock(&self.state).check_user(user, key)
    }
    fn get_progress(&self, user: &str, document: &str) -> Option<Progress> {
        lock(&self.state)
            .progress
            .get(user)
            .and_then(|m| m.get(document))
            .cloned()
    }
    fn put_progress(&self, user: &str, progress: Progress) -> Result<()> {
        let mut st = lock(&self.state);
        st.progress
            .entry(user.to_string())
            .or_default()
            .insert(progress.document.clone(), progress);
        self.save(&st)
    }
    fn user_count(&self) -> usize {
        lock(&self.state).users.len()
    }
}

/// The protocol handler.
pub struct KoSync {
    store: Box<dyn KoSyncStore>,
    allow_registration: bool,
    clock: Box<dyn Fn() -> i64 + Send + Sync>,
}

const MAX_NAME: usize = 128;

fn message(status: u16, msg: &str) -> Response {
    Response::json(status, &json!({ "message": msg }))
}

fn valid_user(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_NAME
        && !name.contains(':')
        && !name.chars().any(|c| c.is_control())
}

fn valid_document(doc: &str) -> bool {
    !doc.is_empty()
        && doc.chars().count() <= MAX_NAME * 2
        && !doc.contains(':')
        && !doc.chars().any(|c| c.is_control())
}

impl KoSync {
    pub fn new(store: Box<dyn KoSyncStore>, allow_registration: bool) -> KoSync {
        KoSync {
            store,
            allow_registration,
            clock: Box::new(|| chrono::Utc::now().timestamp()),
        }
    }

    /// Replaces the clock (tests).
    pub fn with_clock(mut self, clock: impl Fn() -> i64 + Send + Sync + 'static) -> KoSync {
        self.clock = Box::new(clock);
        self
    }

    pub fn store(&self) -> &dyn KoSyncStore {
        &*self.store
    }

    /// True if the path (without prefix) belongs to this protocol.
    pub fn owns(path: &str) -> bool {
        let first = path.trim_start_matches('/').split('/').next().unwrap_or("");
        matches!(first, "users" | "syncs" | "healthcheck")
    }

    /// Authenticated user name from the headers, if valid.
    fn authenticate(&self, req: &Request) -> Option<String> {
        let user = req.header("x-auth-user")?;
        let key = req.header("x-auth-key")?;
        if self.store.check_user(user, key) {
            Some(user.to_string())
        } else {
            None
        }
    }

    pub fn handle(&self, req: &Request) -> Response {
        let Some(segs) = split_path(&req.path) else {
            return message(400, "Invalid path");
        };
        let segs: Vec<&str> = segs.iter().map(|s| s.as_str()).collect();
        let method = req.method.as_str();
        match (method, segs.as_slice()) {
            ("GET" | "HEAD", ["healthcheck"]) => Response::json(200, &json!({"state": "OK"})),
            ("POST", ["users", "create"]) => self.create(req),
            ("GET", ["users", "auth"]) => match self.authenticate(req) {
                Some(_) => Response::json(200, &json!({"authorized": "OK"})),
                None => message(401, "Unauthorized"),
            },
            ("PUT", ["syncs", "progress"]) => self.put(req),
            ("GET", ["syncs", "progress", doc]) => self.get(req, doc),
            (_, ["healthcheck"])
            | (_, ["users", "create"])
            | (_, ["users", "auth"])
            | (_, ["syncs", "progress"])
            | (_, ["syncs", "progress", _]) => {
                message(405, "Method not allowed").with_header("Allow", "GET, POST, PUT")
            }
            _ => message(404, "Not found"),
        }
    }

    fn create(&self, req: &Request) -> Response {
        if !self.allow_registration {
            return message(403, "Registration is disabled");
        }
        let Ok(v) = serde_json::from_slice::<Value>(&req.body) else {
            return message(400, "Invalid request");
        };
        let (Some(user), Some(pass)) = (
            v.get("username").and_then(Value::as_str),
            v.get("password").and_then(Value::as_str),
        ) else {
            return message(400, "Invalid request");
        };
        if !valid_user(user) || pass.is_empty() || pass.chars().count() > MAX_NAME {
            return message(400, "Invalid request");
        }
        match self.store.create_user(user, pass) {
            Ok(true) => Response::json(201, &json!({"username": user})),
            Ok(false) => message(402, "Username is already registered."),
            Err(_) => message(500, "Unknown server error."),
        }
    }

    fn put(&self, req: &Request) -> Response {
        let Some(user) = self.authenticate(req) else {
            return message(401, "Unauthorized");
        };
        let Ok(v) = serde_json::from_slice::<Value>(&req.body) else {
            return message(400, "Invalid request");
        };
        let doc = v.get("document").and_then(Value::as_str).unwrap_or("");
        let progress = v.get("progress").and_then(Value::as_str);
        let pct = v.get("percentage").and_then(Value::as_f64);
        let device = v.get("device").and_then(Value::as_str);
        let (Some(progress), Some(pct), Some(device)) = (progress, pct, device) else {
            return message(400, "Invalid request");
        };
        if !valid_document(doc) || !pct.is_finite() || !(0.0..=1.0).contains(&pct) {
            return message(400, "Invalid request");
        }
        let ts = (self.clock)();
        let p = Progress {
            document: doc.to_string(),
            progress: progress.to_string(),
            percentage: pct,
            device: device.to_string(),
            device_id: v
                .get("device_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            timestamp: ts,
        };
        match self.store.put_progress(&user, p) {
            Ok(()) => Response::json(200, &json!({"document": doc, "timestamp": ts})),
            Err(_) => message(500, "Unknown server error."),
        }
    }

    fn get(&self, req: &Request, doc: &str) -> Response {
        let Some(user) = self.authenticate(req) else {
            return message(401, "Unauthorized");
        };
        if !valid_document(doc) {
            return message(400, "Invalid request");
        }
        match self.store.get_progress(&user, doc) {
            Some(p) => Response::json(
                200,
                &json!({
                    "document": p.document,
                    "progress": p.progress,
                    "percentage": p.percentage,
                    "device": p.device,
                    "device_id": p.device_id,
                    "timestamp": p.timestamp,
                }),
            ),
            None => message(404, "Not found"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn sync() -> KoSync {
        KoSync::new(Box::new(MemoryStore::new()), true).with_clock(|| 1_700_000_000)
    }

    fn create(s: &KoSync, user: &str, pass: &str) -> Response {
        s.handle(
            &Request::new("POST", "/users/create").with_body(
                json!({"username": user, "password": md5_hex(pass)})
                    .to_string()
                    .as_bytes(),
            ),
        )
    }

    fn authed(method: &str, target: &str, user: &str, pass: &str) -> Request {
        Request::new(method, target)
            .with_header("x-auth-user", user)
            .with_header("x-auth-key", &md5_hex(pass))
    }

    fn put_body(doc: &str, pct: f64) -> Vec<u8> {
        json!({"document": doc, "progress": "/body/DocFragment[3]/body/p[2]/text().10",
               "percentage": pct, "device": "Kindle", "device_id": "ABC123"})
        .to_string()
        .into_bytes()
    }

    #[test]
    fn md5_matches_known_vector() {
        assert_eq!(md5_hex("password"), "5f4dcc3b5aa765d61d8327deb882cf99");
        assert_eq!(md5_hex(""), "d41d8cd98f00b204e9800998ecf8427e");
    }

    #[test]
    fn healthcheck() {
        let r = sync().handle(&Request::new("GET", "/healthcheck"));
        assert_eq!(r.status, 200);
        assert_eq!(r.body_str(), r#"{"state":"OK"}"#);
    }

    #[test]
    fn owns_only_kosync_paths() {
        assert!(KoSync::owns("/users/auth"));
        assert!(KoSync::owns("/syncs/progress/x"));
        assert!(KoSync::owns("/healthcheck"));
        assert!(!KoSync::owns("/opds"));
        assert!(!KoSync::owns("/"));
    }

    #[test]
    fn create_user_then_duplicate_is_402() {
        let s = sync();
        let r = create(&s, "alice", "pw");
        assert_eq!(r.status, 201);
        assert_eq!(r.body_str(), r#"{"username":"alice"}"#);
        let r = create(&s, "alice", "other");
        assert_eq!(r.status, 402);
        assert!(r.body_str().contains("already registered"));
        assert_eq!(s.store().user_count(), 1);
    }

    #[test]
    fn create_user_validation() {
        let s = sync();
        let post = |body: &[u8]| s.handle(&Request::new("POST", "/users/create").with_body(body));
        assert_eq!(post(b"not json").status, 400);
        assert_eq!(post(b"{}").status, 400);
        assert_eq!(post(br#"{"username":"a"}"#).status, 400);
        assert_eq!(post(br#"{"username":"","password":"x"}"#).status, 400);
        assert_eq!(post(br#"{"username":"a:b","password":"x"}"#).status, 400);
        assert_eq!(post(br#"{"username":"a\nb","password":"x"}"#).status, 400);
        assert_eq!(post(br#"{"username":"a","password":""}"#).status, 400);
        assert_eq!(post(br#"{"username":1,"password":"x"}"#).status, 400);
        let long = "x".repeat(200);
        assert_eq!(
            post(
                json!({"username": long, "password": "x"})
                    .to_string()
                    .as_bytes()
            )
            .status,
            400
        );
        assert_eq!(s.store().user_count(), 0);
    }

    #[test]
    fn registration_can_be_disabled() {
        let s = KoSync::new(Box::new(MemoryStore::new()), false);
        let r = create(&s, "alice", "pw");
        assert_eq!(r.status, 403);
        assert_eq!(s.store().user_count(), 0);
    }

    #[test]
    fn auth_ok_wrong_password_unknown_user_and_missing_headers() {
        let s = sync();
        create(&s, "alice", "pw");
        assert_eq!(
            s.handle(&authed("GET", "/users/auth", "alice", "pw"))
                .status,
            200
        );
        let ok = s.handle(&authed("GET", "/users/auth", "alice", "pw"));
        assert_eq!(ok.body_str(), r#"{"authorized":"OK"}"#);
        assert_eq!(
            s.handle(&authed("GET", "/users/auth", "alice", "bad"))
                .status,
            401
        );
        assert_eq!(
            s.handle(&authed("GET", "/users/auth", "bob", "pw")).status,
            401
        );
        assert_eq!(s.handle(&Request::new("GET", "/users/auth")).status, 401);
        let only_user = Request::new("GET", "/users/auth").with_header("x-auth-user", "alice");
        assert_eq!(s.handle(&only_user).status, 401);
    }

    #[test]
    fn progress_roundtrip_with_all_fields() {
        let s = sync();
        create(&s, "alice", "pw");
        let r = s.handle(
            &authed("PUT", "/syncs/progress", "alice", "pw").with_body(&put_body("doc1", 0.42)),
        );
        assert_eq!(r.status, 200);
        let v: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(v["document"], "doc1");
        assert_eq!(v["timestamp"], 1_700_000_000);

        let r = s.handle(&authed("GET", "/syncs/progress/doc1", "alice", "pw"));
        assert_eq!(r.status, 200);
        let v: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(v["document"], "doc1");
        assert_eq!(v["progress"], "/body/DocFragment[3]/body/p[2]/text().10");
        assert_eq!(v["percentage"], 0.42);
        assert_eq!(v["device"], "Kindle");
        assert_eq!(v["device_id"], "ABC123");
        assert_eq!(v["timestamp"], 1_700_000_000);
    }

    #[test]
    fn later_put_overwrites_and_users_are_isolated() {
        let s = sync();
        create(&s, "alice", "pw");
        create(&s, "bob", "pw2");
        s.handle(&authed("PUT", "/syncs/progress", "alice", "pw").with_body(&put_body("d", 0.1)));
        s.handle(&authed("PUT", "/syncs/progress", "alice", "pw").with_body(&put_body("d", 0.9)));
        let r = s.handle(&authed("GET", "/syncs/progress/d", "alice", "pw"));
        assert!(r.body_str().contains("0.9"));
        let r = s.handle(&authed("GET", "/syncs/progress/d", "bob", "pw2"));
        assert_eq!(r.status, 404);
    }

    #[test]
    fn unknown_document_is_404() {
        let s = sync();
        create(&s, "alice", "pw");
        let r = s.handle(&authed("GET", "/syncs/progress/nothing", "alice", "pw"));
        assert_eq!(r.status, 404);
    }

    #[test]
    fn progress_requires_auth() {
        let s = sync();
        create(&s, "alice", "pw");
        let r = s.handle(&Request::new("PUT", "/syncs/progress").with_body(&put_body("d", 0.5)));
        assert_eq!(r.status, 401);
        let r = s.handle(&Request::new("GET", "/syncs/progress/d"));
        assert_eq!(r.status, 401);
        let r = s.handle(
            &authed("PUT", "/syncs/progress", "alice", "wrong").with_body(&put_body("d", 0.5)),
        );
        assert_eq!(r.status, 401);
    }

    #[test]
    fn put_progress_validation() {
        let s = sync();
        create(&s, "alice", "pw");
        let put = |body: Value| {
            s.handle(
                &authed("PUT", "/syncs/progress", "alice", "pw")
                    .with_body(body.to_string().as_bytes()),
            )
            .status
        };
        let ok = json!({"document":"d","progress":"p","percentage":0.5,"device":"x"});
        assert_eq!(put(ok.clone()), 200); // device_id is optional
        let mut bad = ok.clone();
        bad["percentage"] = json!(1.5);
        assert_eq!(put(bad), 400);
        let mut bad = ok.clone();
        bad["percentage"] = json!(-0.1);
        assert_eq!(put(bad), 400);
        let mut bad = ok.clone();
        bad["percentage"] = json!("half");
        assert_eq!(put(bad), 400);
        let mut bad = ok.clone();
        bad["document"] = json!("");
        assert_eq!(put(bad), 400);
        let mut bad = ok.clone();
        bad["document"] = json!("a:b");
        assert_eq!(put(bad), 400);
        let mut bad = ok.clone();
        bad["progress"] = json!(5);
        assert_eq!(put(bad), 400);
        let mut bad = ok;
        bad.as_object_mut().unwrap().remove("device");
        assert_eq!(put(bad), 400);
        let r = s.handle(&authed("PUT", "/syncs/progress", "alice", "pw").with_body(b"[[["));
        assert_eq!(r.status, 400);
    }

    #[test]
    fn document_ids_with_unicode_and_encoding() {
        let s = sync();
        create(&s, "alice", "pw");
        s.handle(
            &authed("PUT", "/syncs/progress", "alice", "pw").with_body(&put_body("日本 語", 0.3)),
        );
        let r = s.handle(&authed(
            "GET",
            "/syncs/progress/%E6%97%A5%E6%9C%AC%20%E8%AA%9E",
            "alice",
            "pw",
        ));
        assert_eq!(r.status, 200);
    }

    #[test]
    fn real_koreader_document_hash_and_extra_fields() {
        // KOReader sends the partial MD5 of the file (32 hex chars) and extra keys are ignored.
        let s = sync();
        create(&s, "alice", "pw");
        let doc = "0123456789abcdef0123456789abcdef";
        let body = json!({"document": doc, "progress": "12", "percentage": 0.0, "device": "Boox", "device_id": "1", "extra": [1]});
        let r = s.handle(
            &authed("PUT", "/syncs/progress", "alice", "pw").with_body(body.to_string().as_bytes()),
        );
        assert_eq!(r.status, 200);
        let r = s.handle(&authed(
            "GET",
            &format!("/syncs/progress/{doc}"),
            "alice",
            "pw",
        ));
        assert_eq!(r.status, 200);
    }

    #[test]
    fn wrong_methods_and_unknown_paths() {
        let s = sync();
        assert_eq!(
            s.handle(&Request::new("DELETE", "/syncs/progress")).status,
            405
        );
        assert_eq!(s.handle(&Request::new("GET", "/users/create")).status, 405);
        assert_eq!(s.handle(&Request::new("POST", "/healthcheck")).status, 405);
        assert_eq!(s.handle(&Request::new("GET", "/nope")).status, 404);
        assert_eq!(
            s.handle(&Request::new("GET", "/syncs/progress/a/b")).status,
            404
        );
        assert_eq!(s.handle(&Request::new("GET", "/syncs/../x")).status, 400);
    }

    #[test]
    fn json_store_persists_and_reloads() {
        let dir = std::env::temp_dir().join(format!("bookx-kosync-{}", uuid::Uuid::new_v4()));
        let path = dir.join("sub").join("kosync.json");
        {
            let st = JsonFileStore::open(&path).unwrap();
            assert!(st.create_user("alice", "k").unwrap());
            assert!(!st.create_user("alice", "k2").unwrap());
            st.put_progress(
                "alice",
                Progress {
                    document: "d".into(),
                    progress: "p".into(),
                    percentage: 0.5,
                    device: "dev".into(),
                    device_id: "id".into(),
                    timestamp: 7,
                },
            )
            .unwrap();
        }
        let st = JsonFileStore::open(&path).unwrap();
        assert!(st.check_user("alice", "k"));
        assert!(!st.check_user("alice", "k2"));
        let p = st.get_progress("alice", "d").unwrap();
        assert_eq!(
            (p.percentage, p.timestamp, p.device_id.as_str()),
            (0.5, 7, "id")
        );
        assert_eq!(st.user_count(), 1);
        assert!(!path.with_extension("json.tmp").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn json_store_rejects_corrupt_file_and_accepts_empty() {
        let dir = std::env::temp_dir().join(format!("bookx-kosync-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("bad.json");
        std::fs::write(&bad, "{{{").unwrap();
        let err = JsonFileStore::open(&bad).err().unwrap();
        assert!(format!("{err:#}").contains("corrupt KOSync store"));
        let empty = dir.join("empty.json");
        std::fs::write(&empty, "  \n").unwrap();
        assert_eq!(JsonFileStore::open(&empty).unwrap().user_count(), 0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn memory_store_is_thread_safe() {
        let s = Arc::new(sync());
        create(&s, "alice", "pw");
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let s = s.clone();
                std::thread::spawn(move || {
                    for j in 0..20 {
                        let doc = format!("doc{i}_{j}");
                        let r = s.handle(
                            &authed("PUT", "/syncs/progress", "alice", "pw")
                                .with_body(&put_body(&doc, 0.5)),
                        );
                        assert_eq!(r.status, 200);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let r = s.handle(&authed("GET", "/syncs/progress/doc7_19", "alice", "pw"));
        assert_eq!(r.status, 200);
    }

    #[test]
    fn unknown_user_check_is_false() {
        let st = MemoryStore::new();
        assert!(!st.check_user("ghost", "k"));
    }
}
