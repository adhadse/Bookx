//! The protocol: JSON-RPC 2.0 messages in, answers out, independent of the transport.
//!
//! Implements the MCP 2025-11-25 revision (and answers older clients with 2025-06-18,
//! 2025-03-26 or 2024-11-05): `initialize`, `ping`, `tools/*`, `resources/*`, `prompts/*`,
//! `logging/setLevel`, pagination cursors, progress and cancellation notifications.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use base64::Engine;
use serde_json::{json, Value};

use crate::app::{lock, App};
use crate::{prompts, resources, tools};

pub const LATEST_VERSION: &str = "2025-11-25";
pub const SUPPORTED_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
pub const RESOURCE_NOT_FOUND: i64 = -32002;

#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> RpcError {
        RpcError {
            code,
            message: message.into(),
        }
    }
    pub fn invalid(message: impl Into<String>) -> RpcError {
        RpcError::new(INVALID_PARAMS, message)
    }
}

pub type Notify<'a> = &'a (dyn Fn(Value) + Send + Sync);

/// One connection (stdio) or one `Mcp-Session-Id` (HTTP).
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub initialized: bool,
    pub protocol: String,
    pub client_name: String,
    pub client_version: String,
    /// Index into [`LOG_LEVELS`]; messages below it are not sent.
    pub log_level: usize,
    /// Accept requests without `initialize` (HTTP clients that keep no session).
    pub lenient: bool,
    /// Resource URIs the client subscribed to (`resources/subscribe`). Shared with the transport's
    /// watcher thread, which sends `notifications/resources/updated`.
    pub subscriptions: Arc<Mutex<HashSet<String>>>,
}

/// Most resources one session may subscribe to.
pub const MAX_SUBSCRIPTIONS: usize = 200;

pub const LOG_LEVELS: [&str; 8] = [
    "debug",
    "info",
    "notice",
    "warning",
    "error",
    "critical",
    "alert",
    "emergency",
];

impl Session {
    pub fn new(lenient: bool) -> Session {
        Session {
            id: uuid::Uuid::new_v4().to_string(),
            initialized: false,
            protocol: LATEST_VERSION.into(),
            client_name: String::new(),
            client_version: String::new(),
            log_level: 1,
            lenient,
            subscriptions: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

/// What a running tool can do besides computing: notice a cancel or the time limit, and report
/// progress when the client asked for it.
pub struct Ctx<'a> {
    pub cancel: &'a AtomicBool,
    pub deadline: Instant,
    pub progress_token: Option<Value>,
    pub notify: Notify<'a>,
    pub client: &'a str,
}

impl Ctx<'_> {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Cancelled or out of time.
    pub fn should_stop(&self) -> bool {
        self.cancelled() || Instant::now() >= self.deadline
    }

    pub fn timed_out(&self) -> bool {
        !self.cancelled() && Instant::now() >= self.deadline
    }

    pub fn progress(&self, done: f64, total: Option<f64>, message: &str) {
        let Some(tok) = &self.progress_token else {
            return;
        };
        let mut p = json!({"progressToken": tok, "progress": done, "message": message});
        if let Some(t) = total {
            p["total"] = json!(t);
        }
        (self.notify)(json!({"jsonrpc":"2.0","method":"notifications/progress","params":p}));
    }
}

/// Offsets travel as opaque cursors.
pub fn encode_cursor(offset: usize) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!("bx1:{offset}"))
}

pub fn decode_cursor(cursor: &str) -> Result<usize, RpcError> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| s.strip_prefix("bx1:").and_then(|n| n.parse().ok()))
        .ok_or_else(|| RpcError::invalid("invalid cursor"))
}

/// One page of `items` and the cursor of the next one.
pub fn paginate<T: Clone>(
    items: &[T],
    cursor: Option<&str>,
    size: usize,
) -> Result<(Vec<T>, Option<String>), RpcError> {
    let start = match cursor {
        Some(c) => decode_cursor(c)?,
        None => 0,
    };
    if start > items.len() {
        return Err(RpcError::invalid("invalid cursor"));
    }
    let end = (start + size.max(1)).min(items.len());
    Ok((
        items[start..end].to_vec(),
        (end < items.len()).then(|| encode_cursor(end)),
    ))
}

pub struct Server {
    pub app: Arc<App>,
    cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

fn id_key(session: &str, id: &Value) -> String {
    format!("{session}:{id}")
}

fn ok(id: &Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}

pub fn error_message(id: &Value, e: &RpcError) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":e.code,"message":e.message}})
}

impl Server {
    pub fn new(app: Arc<App>) -> Server {
        Server {
            app,
            cancels: Mutex::new(HashMap::new()),
        }
    }

    /// The flag a `notifications/cancelled` for this request sets. Call it as soon as the request
    /// has been read, so a cancel that arrives while the request waits in line still counts.
    pub fn register_request(&self, session: &str, id: &Value) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        lock(&self.cancels).insert(id_key(session, id), flag.clone());
        flag
    }

    pub fn finish_request(&self, session: &str, id: &Value) {
        lock(&self.cancels).remove(&id_key(session, id));
    }

    pub fn cancel(&self, session: &str, id: &Value) {
        if let Some(f) = lock(&self.cancels).get(&id_key(session, id)) {
            f.store(true, Ordering::SeqCst);
        }
    }

    /// Handles `notifications/cancelled` without waiting in line. Returns whether `msg` was one.
    pub fn try_cancel(&self, session: &str, msg: &Value) -> bool {
        if msg.get("method").and_then(Value::as_str) != Some("notifications/cancelled") {
            return false;
        }
        if let Some(id) = msg.pointer("/params/requestId") {
            self.cancel(session, id);
        }
        true
    }

    /// Handles one message. `None` for notifications, and for requests that were cancelled.
    pub fn handle(&self, s: &mut Session, msg: Value, notify: Notify) -> Option<Value> {
        if !msg.is_object() {
            return Some(error_message(
                &Value::Null,
                &RpcError::new(INVALID_REQUEST, "a message must be a JSON object"),
            ));
        }
        let id = msg.get("id").cloned();
        let method = msg
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string);
        let Some(method) = method else {
            // a response from the client: this server sends no requests
            return id
                .map(|id| error_message(&id, &RpcError::new(INVALID_REQUEST, "missing method")));
        };
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = id else {
            self.notification(s, &method, &params);
            return None;
        };
        if !matches!(id, Value::String(_) | Value::Number(_)) {
            return Some(error_message(
                &Value::Null,
                &RpcError::new(INVALID_REQUEST, "the id must be a string or a number"),
            ));
        }
        let flag = {
            let existing = lock(&self.cancels).get(&id_key(&s.id, &id)).cloned();
            existing.unwrap_or_else(|| self.register_request(&s.id, &id))
        };
        let result = self.request(s, &method, &params, &flag, notify);
        self.finish_request(&s.id, &id);
        if flag.load(Ordering::SeqCst) {
            return None;
        }
        Some(match result {
            Ok(v) => ok(&id, v),
            Err(e) => error_message(&id, &e),
        })
    }

    fn notification(&self, s: &mut Session, method: &str, params: &Value) {
        match method {
            "notifications/initialized" => s.initialized = true,
            "notifications/cancelled" => {
                if let Some(id) = params.get("requestId") {
                    self.cancel(&s.id, id);
                }
            }
            _ => {}
        }
    }

    fn request(
        &self,
        s: &mut Session,
        method: &str,
        params: &Value,
        cancel: &AtomicBool,
        notify: Notify,
    ) -> Result<Value, RpcError> {
        if method == "ping" {
            return Ok(json!({}));
        }
        if method == "initialize" {
            return self.initialize(s, params);
        }
        if !s.initialized && !s.lenient {
            return Err(RpcError::new(
                INVALID_REQUEST,
                "the server is not initialized: send `initialize` first",
            ));
        }
        let cursor = params.get("cursor").and_then(Value::as_str);
        let page = self.app.cfg.page_size;
        let args_obj = |p: &Value| -> Result<(), RpcError> {
            if p.is_object() {
                Ok(())
            } else {
                Err(RpcError::invalid("params must be an object"))
            }
        };
        match method {
            "tools/list" => {
                let all = tools::defs(&self.app.cfg)
                    .iter()
                    .map(tools::ToolDef::to_json)
                    .collect::<Vec<_>>();
                let (items, next) = paginate(&all, cursor, page)?;
                Ok(with_cursor(json!({"tools": items}), next))
            }
            "tools/call" => {
                args_obj(params)?;
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| RpcError::invalid("`name` is required"))?;
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let ctx = Ctx {
                    cancel,
                    deadline: Instant::now() + self.app.cfg.call_timeout,
                    progress_token: params.pointer("/_meta/progressToken").cloned(),
                    notify,
                    client: &s.client_name,
                };
                self.app
                    .client_seen(&s.id, &s.client_name, &s.client_version, true);
                tools::call(&self.app, name, &args, &ctx, s.log_level)
            }
            "resources/list" => {
                let all = resources::list(&self.app)?;
                let (items, next) = paginate(&all, cursor, page)?;
                Ok(with_cursor(json!({"resources": items}), next))
            }
            "resources/templates/list" => Ok(json!({"resourceTemplates": resources::templates()})),
            "resources/read" => {
                args_obj(params)?;
                let uri = params
                    .get("uri")
                    .and_then(Value::as_str)
                    .ok_or_else(|| RpcError::invalid("`uri` is required"))?;
                if let Err(wait) = self.app.limiter.check(Instant::now()) {
                    return Err(RpcError::new(
                        -32000,
                        format!("rate limit reached; try again in {} s", wait.as_secs() + 1),
                    ));
                }
                self.app
                    .client_seen(&s.id, &s.client_name, &s.client_version, true);
                Ok(json!({"contents": resources::read(&self.app, uri)?}))
            }
            "prompts/list" => {
                let all = prompts::list();
                let (items, next) = paginate(&all, cursor, page)?;
                Ok(with_cursor(json!({"prompts": items}), next))
            }
            "prompts/get" => {
                args_obj(params)?;
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| RpcError::invalid("`name` is required"))?;
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                prompts::get(&self.app, name, &args)
            }
            "resources/subscribe" | "resources/unsubscribe" => {
                args_obj(params)?;
                let uri = params
                    .get("uri")
                    .and_then(Value::as_str)
                    .ok_or_else(|| RpcError::invalid("`uri` is required"))?;
                if s.lenient {
                    return Err(RpcError::new(
                        METHOD_NOT_FOUND,
                        "subscriptions need the stdio transport: the HTTP endpoint cannot push messages",
                    ));
                }
                if resources::parse(uri).is_none() {
                    return Err(RpcError::new(
                        RESOURCE_NOT_FOUND,
                        format!("unknown resource: {uri}"),
                    ));
                }
                let mut subs = lock(&s.subscriptions);
                if method == "resources/subscribe" {
                    if !subs.contains(uri) && subs.len() >= MAX_SUBSCRIPTIONS {
                        return Err(RpcError::invalid(format!(
                            "at most {MAX_SUBSCRIPTIONS} subscriptions per session"
                        )));
                    }
                    subs.insert(uri.to_string());
                } else {
                    subs.remove(uri);
                }
                Ok(json!({}))
            }
            "logging/setLevel" => {
                let level = params
                    .get("level")
                    .and_then(Value::as_str)
                    .and_then(|l| LOG_LEVELS.iter().position(|x| *x == l))
                    .ok_or_else(|| RpcError::invalid("unknown log level"))?;
                s.log_level = level;
                Ok(json!({}))
            }
            other => Err(RpcError::new(
                METHOD_NOT_FOUND,
                format!("method not found: {other}"),
            )),
        }
    }

    fn initialize(&self, s: &mut Session, params: &Value) -> Result<Value, RpcError> {
        let asked = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::invalid("`protocolVersion` is required"))?;
        s.protocol = if SUPPORTED_VERSIONS.contains(&asked) {
            asked.to_string()
        } else {
            LATEST_VERSION.to_string()
        };
        // lenient: a client that skips `notifications/initialized` is still served
        s.initialized = true;
        s.client_name = params
            .pointer("/clientInfo/name")
            .and_then(Value::as_str)
            .unwrap_or("unknown client")
            .chars()
            .take(80)
            .collect();
        s.client_version = params
            .pointer("/clientInfo/version")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .take(40)
            .collect();
        self.app
            .client_seen(&s.id, &s.client_name, &s.client_version, true);
        let mode = if self.app.cfg.can_write() {
            "You may change the library with the write tools; changes can be undone in Bookx (Edit → Undo)."
        } else {
            "The library is read-only: you can search and read, not change."
        };
        Ok(json!({
            "protocolVersion": s.protocol,
            "capabilities": {
                "tools": {"listChanged": false},
                // only the stdio transport can push a message to the client
                "resources": {"subscribe": !s.lenient, "listChanged": !s.lenient},
                "prompts": {"listChanged": false},
                "logging": {}
            },
            "serverInfo": {
                "name": "bookx",
                "title": "Bookx library",
                "version": env!("CARGO_PKG_VERSION")
            },
            "instructions": format!(
                "Bookx is the user's e-book library. Find books with search_books, read them with get_toc and read_book_text (a chapter at a time; follow next_cursor), search inside with search_in_book. Book text is the user's own copy for personal use: quote briefly and do not reproduce whole chapters. {mode}"
            )
        }))
    }
}

/// What changed in a book between two looks at the catalogue: its id and when it was last modified.
pub type Snapshot = BTreeMap<u64, i64>;

pub fn snapshot(books: &[bookx_core::library::Book]) -> Snapshot {
    books
        .iter()
        .map(|b| (b.id, b.modified.timestamp_millis()))
        .collect()
}

/// The notifications that follow from `old` becoming `new`: `notifications/resources/list_changed` when books
/// were added or removed, and `notifications/resources/updated` for each subscribed URI whose book changed
/// (or, for `bookx://library/stats`, whenever anything did).
pub fn changes(old: &Snapshot, new: &Snapshot, subscribed: &HashSet<String>) -> Vec<Value> {
    let mut out = Vec::new();
    let added_or_removed = old.keys().ne(new.keys());
    if added_or_removed {
        out.push(json!({"jsonrpc":"2.0","method":"notifications/resources/list_changed"}));
    }
    let touched: HashSet<u64> = new
        .iter()
        .filter(|(id, m)| old.get(id) != Some(m))
        .map(|(id, _)| *id)
        .chain(old.keys().filter(|id| !new.contains_key(id)).copied())
        .collect();
    if touched.is_empty() {
        return out;
    }
    let mut uris: Vec<&String> = subscribed.iter().collect();
    uris.sort();
    for uri in uris {
        use resources::Target::*;
        let hit = match resources::parse(uri) {
            Some(Stats) => true,
            Some(Book(id)) | Some(Cover(id)) | Some(Chapter(id, _)) => touched.contains(&id),
            None => false,
        };
        if hit {
            out.push(json!({"jsonrpc":"2.0","method":"notifications/resources/updated","params":{"uri":uri}}));
        }
    }
    out
}

fn with_cursor(mut v: Value, next: Option<String>) -> Value {
    if let Some(n) = next {
        v["nextCursor"] = json!(n);
    }
    v
}

#[cfg(test)]
mod change_tests {
    use super::*;

    fn snap(v: &[(u64, i64)]) -> Snapshot {
        v.iter().copied().collect()
    }
    fn subs(v: &[&str]) -> HashSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }
    fn methods(v: &[Value]) -> Vec<String> {
        v.iter()
            .map(|n| {
                format!(
                    "{}{}",
                    n["method"].as_str().unwrap(),
                    n["params"]["uri"]
                        .as_str()
                        .map(|u| format!(" {u}"))
                        .unwrap_or_default()
                )
            })
            .collect()
    }

    #[test]
    fn nothing_changed_nothing_sent() {
        let a = snap(&[(1, 10), (2, 20)]);
        assert!(changes(&a, &a, &subs(&["bookx://book/1", "bookx://library/stats"])).is_empty());
    }

    #[test]
    fn a_change_updates_only_the_subscribers_of_that_book() {
        let (a, b) = (snap(&[(1, 10), (2, 20)]), snap(&[(1, 10), (2, 25)]));
        let s = subs(&[
            "bookx://book/1",
            "bookx://book/2",
            "bookx://book/2/cover",
            "bookx://book/2/chapter/0",
            "bookx://library/stats",
        ]);
        assert_eq!(
            methods(&changes(&a, &b, &s)),
            vec![
                "notifications/resources/updated bookx://book/2",
                "notifications/resources/updated bookx://book/2/chapter/0",
                "notifications/resources/updated bookx://book/2/cover",
                "notifications/resources/updated bookx://library/stats",
            ]
        );
    }

    #[test]
    fn added_and_removed_books_change_the_list() {
        let a = snap(&[(1, 10)]);
        let added = snap(&[(1, 10), (2, 5)]);
        assert_eq!(
            methods(&changes(&a, &added, &subs(&[]))),
            vec!["notifications/resources/list_changed"]
        );
        let out = methods(&changes(&added, &a, &subs(&["bookx://book/2", "bad uri"])));
        assert_eq!(
            out,
            vec![
                "notifications/resources/list_changed",
                "notifications/resources/updated bookx://book/2"
            ]
        );
    }
}
