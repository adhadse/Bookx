//! ABI v2: editor plugins (CE-019). A plugin of kind `editor-action` may act on the book that is
//! open in the editor through **one** host import, `bookx_host::editor_call`.
//!
//! ```text
//! editor_call(op_ptr, op_len, in_ptr, in_len, out) -> i32
//! ```
//!
//! `op` is the operation name, the input a UTF-8 JSON object. The host calls the guest's `alloc`,
//! stores a JSON reply (`{"ok": value}` or `{"err": "message"}`) there and writes `ptr` and `len`
//! as two little-endian `u32` at `out` (8 bytes); the guest owns the buffer. The return value:
//! `0` ok, `-1` bad arguments or unknown operation, `-2` permission not granted, `-3` limit
//! reached, `-4` the operation failed (the reply says why), `-5` guest allocation failed, `-6`
//! called outside an editor action, `-7` timed out.
//!
//! | op | needs | input | `ok` |
//! |---|---|---|---|
//! | `list_files` | `editor.read` | `{}` | `["OEBPS/ch1.xhtml", …]` |
//! | `read_file` | `editor.read` | `{"path"}` | `{"text"}` (UTF-8) or `{"b64"}` (anything else) |
//! | `current_file` | `editor.read` | `{}` | path or `null` |
//! | `get_selection` | `editor.read` | `{}` | `{"file","text"}` or `null` |
//! | `get_metadata` | `editor.read` | `{}` | metadata object |
//! | `write_file` | `editor.write` | `{"path","text"\|"b64"}` | `null` (the file must exist) |
//! | `add_file` | `editor.write` | `{"path","text"\|"b64"}` | `null` (the file must not exist) |
//! | `rename_file` | `editor.write` | `{"from","to"}` | `{"references_updated": n}` |
//! | `replace_selection` | `editor.write` | `{"text"}` | `null` |
//! | `insert_at_cursor` | `editor.write` | `{"text"}` | `null` |
//! | `set_metadata` | `editor.write` | metadata object (only the given fields change) | `null` |
//! | `add_checkpoint` | `editor.write` | `{"name"}` | `null` |
//! | `show_message` | nothing | `{"text"}` | `null` |
//!
//! Paths are book-relative with `/`: no absolute paths, no `..`, no backslashes, and `mimetype` and
//! `META-INF/` cannot be written. Nothing outside the book is reachable (no file system, no
//! network). Every write is preceded by an automatic checkpoint `Before: <action label>`, and the
//! editor makes the whole action one undo step.
//!
//! Guest entry point: `bookx_call("run_action", {"action": "<id>"})`, reply `{"ok": null | "message"}`.

use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use serde_json::{json, Value};

use crate::manifest::Permission;

/// The open book as the editor shows it to a plugin. Implemented by the application (and by
/// [`MemBook`] for tests); permission checks, path rules and limits are enforced by the host
/// before any of these is called.
pub trait EditorApi: Send {
    fn list_files(&mut self) -> Vec<String>;
    fn read_file(&mut self, path: &str) -> Option<Vec<u8>>;
    /// Replaces an existing file.
    fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), String>;
    /// Creates a new file (error when it exists).
    fn add_file(&mut self, path: &str, data: &[u8]) -> Result<(), String>;
    /// Renames with references following; returns how many references were rewritten.
    fn rename_file(&mut self, from: &str, to: &str) -> Result<usize, String>;
    fn current_file(&mut self) -> Option<String>;
    /// `(file, selected text)`.
    fn selection(&mut self) -> Option<(String, String)>;
    fn replace_selection(&mut self, text: &str) -> Result<(), String>;
    fn insert_at_cursor(&mut self, text: &str) -> Result<(), String>;
    fn metadata(&mut self) -> Value;
    fn set_metadata(&mut self, fields: &Value) -> Result<(), String>;
    fn checkpoint(&mut self, name: &str) -> Result<(), String>;
}

/// Per action limits (the wasm time, fuel and memory limits of the manifest apply as well).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditorLimits {
    pub max_calls: u32,
    pub max_file_bytes: usize,
    pub max_read_bytes: usize,
    pub max_write_bytes: usize,
    pub max_writes: u32,
    pub max_new_files: u32,
    pub max_messages: u32,
    pub max_message_bytes: usize,
    pub max_list: usize,
}

impl Default for EditorLimits {
    fn default() -> Self {
        EditorLimits {
            max_calls: 5_000,
            max_file_bytes: 16 * 1024 * 1024,
            max_read_bytes: 128 * 1024 * 1024,
            max_write_bytes: 64 * 1024 * 1024,
            max_writes: 2_000,
            max_new_files: 100,
            max_messages: 20,
            max_message_bytes: 2_000,
            max_list: 20_000,
        }
    }
}

/// State of one editor action: the book, what the plugin may do, what it did.
pub struct EditorSession {
    api: Box<dyn EditorApi>,
    read: bool,
    write: bool,
    limits: EditorLimits,
    label: String,
    checkpointed: bool,
    calls: u32,
    read_bytes: usize,
    write_bytes: usize,
    writes: u32,
    new_files: u32,
    /// Messages for the user (`show_message`), already truncated.
    pub messages: Vec<String>,
    /// Number of calls that changed the book.
    pub changes: u32,
}

/// Shared handle: the runtime keeps one clone during the call, the application the other.
pub type SharedSession = Arc<Mutex<EditorSession>>;

impl EditorSession {
    /// `permissions` are the manifest's; `label` names the automatic checkpoint.
    pub fn new(
        api: Box<dyn EditorApi>,
        permissions: &[Permission],
        label: &str,
        limits: EditorLimits,
    ) -> SharedSession {
        Arc::new(Mutex::new(EditorSession {
            api,
            read: permissions.contains(&Permission::EditorRead),
            write: permissions.contains(&Permission::EditorWrite),
            limits,
            label: label.to_string(),
            checkpointed: false,
            calls: 0,
            read_bytes: 0,
            write_bytes: 0,
            writes: 0,
            new_files: 0,
            messages: Vec::new(),
            changes: 0,
        }))
    }

    /// Gives the book back once the action is over.
    pub fn into_api(self) -> Box<dyn EditorApi> {
        self.api
    }

    pub fn api_mut(&mut self) -> &mut dyn EditorApi {
        &mut *self.api
    }

    pub fn calls(&self) -> u32 {
        self.calls
    }
}

pub const OK: i32 = 0;
pub const BAD: i32 = -1;
pub const DENIED: i32 = -2;
pub const LIMIT: i32 = -3;
pub const FAILED: i32 = -4;

/// Longest book path a plugin may name.
pub const MAX_PATH_BYTES: usize = 512;

/// Whether `p` is an acceptable book-relative path (see the module docs).
pub fn valid_book_path(p: &str) -> bool {
    if p.is_empty() || p.len() > MAX_PATH_BYTES || p.starts_with('/') || p.contains('\\') {
        return false;
    }
    if p.chars().any(|c| c.is_control() || c == ':') {
        return false;
    }
    p.split('/').all(|s| !s.is_empty() && s != "." && s != "..")
}

/// Paths a plugin may read but never change.
pub fn is_protected(p: &str) -> bool {
    let l = p.to_ascii_lowercase();
    l == "mimetype" || l == "meta-inf" || l.starts_with("meta-inf/")
}

fn err(code: i32, msg: impl Into<String>) -> (i32, Value) {
    (code, json!({ "err": msg.into() }))
}

fn ok(v: Value) -> (i32, Value) {
    (OK, json!({ "ok": v }))
}

fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn payload(v: &Value) -> Result<Vec<u8>, String> {
    if let Some(t) = str_field(v, "text") {
        return Ok(t.as_bytes().to_vec());
    }
    if let Some(b) = str_field(v, "b64") {
        return B64.decode(b).map_err(|_| "`b64` is not base64".to_string());
    }
    Err("give `text` or `b64`".to_string())
}

impl EditorSession {
    fn ensure_checkpoint(&mut self) -> Result<(), String> {
        if !self.checkpointed {
            let name = format!("Before: {}", self.label);
            self.api.checkpoint(&name)?;
            self.checkpointed = true;
        }
        Ok(())
    }

    fn before_write(&mut self, bytes: usize) -> Option<(i32, Value)> {
        if !self.write {
            return Some(err(DENIED, "permission `editor.write` is not granted"));
        }
        if self.writes >= self.limits.max_writes {
            return Some(err(LIMIT, "too many writes in one action"));
        }
        if self.write_bytes.saturating_add(bytes) > self.limits.max_write_bytes {
            return Some(err(LIMIT, "too much data written in one action"));
        }
        if bytes > self.limits.max_file_bytes {
            return Some(err(LIMIT, "file is too large"));
        }
        if let Err(e) = self.ensure_checkpoint() {
            return Some(err(FAILED, format!("cannot create the checkpoint: {e}")));
        }
        None
    }

    fn wrote(&mut self, bytes: usize) {
        self.writes += 1;
        self.write_bytes += bytes;
        self.changes += 1;
    }

    /// Runs one host call. Returns the status code and the JSON reply.
    pub fn dispatch(&mut self, op: &str, input: &[u8]) -> (i32, Value) {
        self.calls += 1;
        if self.calls > self.limits.max_calls {
            return err(LIMIT, "too many host calls in one action");
        }
        let input: Value = if input.is_empty() {
            json!({})
        } else {
            match serde_json::from_slice(input) {
                Ok(v @ Value::Object(_)) => v,
                _ => return err(BAD, "the input must be a JSON object"),
            }
        };
        let needs_read = matches!(
            op,
            "list_files" | "read_file" | "current_file" | "get_selection" | "get_metadata"
        );
        if needs_read && !self.read {
            return err(DENIED, "permission `editor.read` is not granted");
        }
        match op {
            "list_files" => {
                let mut files = self.api.list_files();
                files.truncate(self.limits.max_list);
                ok(json!(files))
            }
            "read_file" => {
                let Some(path) = str_field(&input, "path").filter(|p| valid_book_path(p)) else {
                    return err(BAD, "invalid path");
                };
                let Some(data) = self.api.read_file(path) else {
                    return err(FAILED, format!("no such file: {path}"));
                };
                if data.len() > self.limits.max_file_bytes {
                    return err(LIMIT, "file is too large to read");
                }
                if self.read_bytes.saturating_add(data.len()) > self.limits.max_read_bytes {
                    return err(LIMIT, "too much data read in one action");
                }
                self.read_bytes += data.len();
                match String::from_utf8(data) {
                    Ok(t) => ok(json!({ "text": t })),
                    Err(e) => ok(json!({ "b64": B64.encode(e.into_bytes()) })),
                }
            }
            "current_file" => ok(json!(self.api.current_file())),
            "get_selection" => match self.api.selection() {
                Some((file, text)) => ok(json!({ "file": file, "text": text })),
                None => ok(Value::Null),
            },
            "get_metadata" => ok(self.api.metadata()),
            "write_file" | "add_file" => {
                let Some(path) = str_field(&input, "path").filter(|p| valid_book_path(p)) else {
                    return err(BAD, "invalid path");
                };
                if is_protected(path) {
                    return err(DENIED, format!("`{path}` is protected"));
                }
                let data = match payload(&input) {
                    Ok(d) => d,
                    Err(e) => return err(BAD, e),
                };
                if let Some(r) = self.before_write(data.len()) {
                    return r;
                }
                let adding = op == "add_file";
                if adding {
                    if self.new_files >= self.limits.max_new_files {
                        return err(LIMIT, "too many new files in one action");
                    }
                    if let Err(e) = self.api.add_file(path, &data) {
                        return err(FAILED, e);
                    }
                    self.new_files += 1;
                } else if let Err(e) = self.api.write_file(path, &data) {
                    return err(FAILED, e);
                }
                self.wrote(data.len());
                ok(Value::Null)
            }
            "rename_file" => {
                let (Some(from), Some(to)) = (str_field(&input, "from"), str_field(&input, "to"))
                else {
                    return err(BAD, "give `from` and `to`");
                };
                if !valid_book_path(from) || !valid_book_path(to) {
                    return err(BAD, "invalid path");
                }
                if is_protected(from) || is_protected(to) {
                    return err(DENIED, "protected path");
                }
                if let Some(r) = self.before_write(0) {
                    return r;
                }
                match self.api.rename_file(from, to) {
                    Ok(n) => {
                        self.wrote(0);
                        ok(json!({ "references_updated": n }))
                    }
                    Err(e) => err(FAILED, e),
                }
            }
            "replace_selection" | "insert_at_cursor" => {
                let Some(text) = str_field(&input, "text") else {
                    return err(BAD, "give `text`");
                };
                if let Some(r) = self.before_write(text.len()) {
                    return r;
                }
                let res = if op == "replace_selection" {
                    self.api.replace_selection(text)
                } else {
                    self.api.insert_at_cursor(text)
                };
                match res {
                    Ok(()) => {
                        self.wrote(text.len());
                        ok(Value::Null)
                    }
                    Err(e) => err(FAILED, e),
                }
            }
            "set_metadata" => {
                if let Some(r) = self.before_write(input.to_string().len()) {
                    return r;
                }
                match self.api.set_metadata(&input) {
                    Ok(()) => {
                        self.wrote(0);
                        ok(Value::Null)
                    }
                    Err(e) => err(FAILED, e),
                }
            }
            "add_checkpoint" => {
                if !self.write {
                    return err(DENIED, "permission `editor.write` is not granted");
                }
                let Some(name) = str_field(&input, "name").map(str::trim) else {
                    return err(BAD, "give `name`");
                };
                if name.is_empty() || name.len() > 120 || name.chars().any(char::is_control) {
                    return err(BAD, "invalid checkpoint name");
                }
                if let Err(e) = self.ensure_checkpoint() {
                    return err(FAILED, e);
                }
                match self.api.checkpoint(name) {
                    Ok(()) => ok(Value::Null),
                    Err(e) => err(FAILED, e),
                }
            }
            "show_message" => {
                let Some(text) = str_field(&input, "text") else {
                    return err(BAD, "give `text`");
                };
                if self.messages.len() as u32 >= self.limits.max_messages {
                    return err(LIMIT, "too many messages");
                }
                let mut t = text.to_string();
                if t.len() > self.limits.max_message_bytes {
                    let mut cut = self.limits.max_message_bytes;
                    while !t.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    t.truncate(cut);
                }
                self.messages.push(
                    t.chars()
                        .filter(|c| !c.is_control() || *c == '\n')
                        .collect(),
                );
                ok(Value::Null)
            }
            _ => err(BAD, format!("unknown operation `{op}`")),
        }
    }
}

/// An in-memory book: the reference implementation of [`EditorApi`] used by tests and by hosts
/// that want to run an action on a copy first.
#[derive(Default, Clone)]
pub struct MemBook {
    pub files: std::collections::BTreeMap<String, Vec<u8>>,
    pub current: Option<String>,
    pub selection: Option<String>,
    pub metadata: serde_json::Map<String, Value>,
    /// Names of the checkpoints taken, in order.
    pub checkpoints: Vec<String>,
    /// Text inserted at the cursor / replacing the selection, in order.
    pub text_edits: Vec<String>,
}

impl EditorApi for MemBook {
    fn list_files(&mut self) -> Vec<String> {
        self.files.keys().cloned().collect()
    }
    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        self.files.get(path).cloned()
    }
    fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        match self.files.get_mut(path) {
            Some(f) => {
                *f = data.to_vec();
                Ok(())
            }
            None => Err(format!("no such file: {path}")),
        }
    }
    fn add_file(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        if self.files.contains_key(path) {
            return Err(format!("{path} already exists"));
        }
        self.files.insert(path.to_string(), data.to_vec());
        Ok(())
    }
    fn rename_file(&mut self, from: &str, to: &str) -> Result<usize, String> {
        if self.files.contains_key(to) {
            return Err(format!("{to} already exists"));
        }
        let d = self
            .files
            .remove(from)
            .ok_or_else(|| format!("no such file: {from}"))?;
        self.files.insert(to.to_string(), d);
        Ok(0)
    }
    fn current_file(&mut self) -> Option<String> {
        self.current.clone()
    }
    fn selection(&mut self) -> Option<(String, String)> {
        Some((self.current.clone()?, self.selection.clone()?))
    }
    fn replace_selection(&mut self, text: &str) -> Result<(), String> {
        if self.selection.is_none() {
            return Err("nothing is selected".into());
        }
        self.text_edits.push(text.to_string());
        Ok(())
    }
    fn insert_at_cursor(&mut self, text: &str) -> Result<(), String> {
        if self.current.is_none() {
            return Err("no file is open".into());
        }
        self.text_edits.push(text.to_string());
        Ok(())
    }
    fn metadata(&mut self) -> Value {
        Value::Object(self.metadata.clone())
    }
    fn set_metadata(&mut self, fields: &Value) -> Result<(), String> {
        let Some(o) = fields.as_object() else {
            return Err("metadata must be an object".into());
        };
        for (k, v) in o {
            self.metadata.insert(k.clone(), v.clone());
        }
        Ok(())
    }
    fn checkpoint(&mut self, name: &str) -> Result<(), String> {
        self.checkpoints.push(name.to_string());
        Ok(())
    }
}

/// What an action did, for the application to show.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionResult {
    /// The plugin's own reply (`null` or a message).
    pub reply: Value,
    pub messages: Vec<String>,
    pub changes: u32,
    pub calls: u32,
}

/// Parses the `run_action` reply.
pub(crate) fn action_result(reply: Value, s: &EditorSession) -> Result<ActionResult> {
    if !(reply.is_null() || reply.is_string()) {
        return Err(anyhow!(
            "the action returned something other than null or text"
        ));
    }
    Ok(ActionResult {
        reply,
        messages: s.messages.clone(),
        changes: s.changes,
        calls: s.calls,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(read: bool, write: bool, book: MemBook) -> SharedSession {
        let mut p = Vec::new();
        if read {
            p.push(Permission::EditorRead);
        }
        if write {
            p.push(Permission::EditorWrite);
        }
        EditorSession::new(Box::new(book), &p, "Test", EditorLimits::default())
    }

    fn book() -> MemBook {
        let mut b = MemBook::default();
        b.files.insert("OEBPS/a.xhtml".into(), b"<p>a</p>".to_vec());
        b.files
            .insert("mimetype".into(), b"application/epub+zip".to_vec());
        b.current = Some("OEBPS/a.xhtml".into());
        b.selection = Some("a".into());
        b
    }

    fn call(s: &SharedSession, op: &str, input: Value) -> (i32, Value) {
        s.lock()
            .unwrap()
            .dispatch(op, serde_json::to_vec(&input).unwrap().as_slice())
    }

    #[test]
    fn path_rules() {
        for ok in ["a.xhtml", "OEBPS/a b.xhtml", "x/y/z"] {
            assert!(valid_book_path(ok), "{ok}");
        }
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "a/../x",
            "a//b",
            "./a",
            "a\\b",
            "C:/x",
            "a\0b",
            "a/",
            &"a".repeat(513),
        ] {
            assert!(!valid_book_path(bad), "{bad:?}");
        }
        assert!(is_protected("mimetype") && is_protected("META-INF/container.xml"));
        assert!(is_protected("meta-inf/x") && !is_protected("OEBPS/mimetype"));
    }

    #[test]
    fn read_needs_permission() {
        let s = session(false, true, book());
        let (c, v) = call(&s, "list_files", json!({}));
        assert_eq!(c, DENIED);
        assert!(v["err"].as_str().unwrap().contains("editor.read"));
        let s = session(true, false, book());
        assert_eq!(call(&s, "list_files", json!({})).0, OK);
        let (c, v) = call(&s, "read_file", json!({"path": "OEBPS/a.xhtml"}));
        assert_eq!((c, v["ok"]["text"].as_str()), (OK, Some("<p>a</p>")));
    }

    #[test]
    fn write_needs_permission_and_checkpoints_first() {
        let s = session(true, false, book());
        let (c, _) = call(
            &s,
            "write_file",
            json!({"path": "OEBPS/a.xhtml", "text": "x"}),
        );
        assert_eq!(c, DENIED);
        for op in [
            "add_file",
            "rename_file",
            "replace_selection",
            "set_metadata",
            "add_checkpoint",
        ] {
            assert_eq!(
                call(
                    &s,
                    op,
                    json!({"path":"n","text":"x","from":"a","to":"b","name":"n"})
                )
                .0,
                DENIED,
                "{op}"
            );
        }
        let s = session(true, true, book());
        assert_eq!(
            call(
                &s,
                "write_file",
                json!({"path": "OEBPS/a.xhtml", "text": "new"})
            )
            .0,
            OK
        );
        assert_eq!(
            call(
                &s,
                "write_file",
                json!({"path": "OEBPS/a.xhtml", "text": "new2"})
            )
            .0,
            OK
        );
        assert_eq!(s.lock().unwrap().changes, 2);
        let session = Arc::try_unwrap(s).ok().unwrap().into_inner().unwrap();
        let mut api = session.into_api();
        assert_eq!(api.read_file("OEBPS/a.xhtml").unwrap(), b"new2");
    }

    #[test]
    fn checkpoint_names_recorded_by_the_session() {
        struct Spy(std::sync::Arc<Mutex<Vec<String>>>);
        impl EditorApi for Spy {
            fn list_files(&mut self) -> Vec<String> {
                vec![]
            }
            fn read_file(&mut self, _: &str) -> Option<Vec<u8>> {
                None
            }
            fn write_file(&mut self, _: &str, _: &[u8]) -> Result<(), String> {
                self.0.lock().unwrap().push("write".into());
                Ok(())
            }
            fn add_file(&mut self, _: &str, _: &[u8]) -> Result<(), String> {
                Ok(())
            }
            fn rename_file(&mut self, _: &str, _: &str) -> Result<usize, String> {
                Ok(0)
            }
            fn current_file(&mut self) -> Option<String> {
                None
            }
            fn selection(&mut self) -> Option<(String, String)> {
                None
            }
            fn replace_selection(&mut self, _: &str) -> Result<(), String> {
                Ok(())
            }
            fn insert_at_cursor(&mut self, _: &str) -> Result<(), String> {
                Ok(())
            }
            fn metadata(&mut self) -> Value {
                json!({})
            }
            fn set_metadata(&mut self, _: &Value) -> Result<(), String> {
                Ok(())
            }
            fn checkpoint(&mut self, n: &str) -> Result<(), String> {
                self.0.lock().unwrap().push(format!("cp:{n}"));
                Ok(())
            }
        }
        let log = Arc::new(Mutex::new(Vec::new()));
        let s = EditorSession::new(
            Box::new(Spy(log.clone())),
            &[Permission::EditorWrite],
            "Trim",
            EditorLimits::default(),
        );
        call(&s, "write_file", json!({"path": "a", "text": "1"}));
        call(&s, "write_file", json!({"path": "a", "text": "2"}));
        call(&s, "add_checkpoint", json!({"name": "mine"}));
        assert_eq!(
            *log.lock().unwrap(),
            vec!["cp:Before: Trim", "write", "write", "cp:mine"]
        );
    }

    #[test]
    fn escape_attempts_are_refused() {
        let s = session(true, true, book());
        for p in [
            "../../etc/passwd",
            "/etc/passwd",
            "a/../../x",
            "C:\\x",
            "..",
            "OEBPS/../../x",
        ] {
            assert_eq!(call(&s, "read_file", json!({"path": p})).0, BAD, "{p}");
            assert_eq!(
                call(&s, "write_file", json!({"path": p, "text": "x"})).0,
                BAD,
                "{p}"
            );
            assert_eq!(
                call(&s, "add_file", json!({"path": p, "text": "x"})).0,
                BAD,
                "{p}"
            );
            assert_eq!(
                call(&s, "rename_file", json!({"from": "OEBPS/a.xhtml", "to": p})).0,
                BAD,
                "{p}"
            );
        }
        for p in ["mimetype", "META-INF/container.xml", "meta-inf/x"] {
            assert_eq!(
                call(&s, "write_file", json!({"path": p, "text": "x"})).0,
                DENIED
            );
            assert_eq!(
                call(&s, "add_file", json!({"path": p, "text": "x"})).0,
                DENIED
            );
            assert_eq!(
                call(&s, "rename_file", json!({"from": p, "to": "z"})).0,
                DENIED
            );
        }
        assert_eq!(call(&s, "read_file", json!({"path": "mimetype"})).0, OK);
        assert_eq!(call(&s, "no_such_op", json!({})).0, BAD);
        assert_eq!(s.lock().unwrap().dispatch("read_file", b"[1]").0, BAD);
        assert_eq!(s.lock().unwrap().dispatch("read_file", b"\xff").0, BAD);
        // nothing was changed
        assert_eq!(s.lock().unwrap().changes, 0);
    }

    #[test]
    fn limits_are_enforced() {
        let limits = EditorLimits {
            max_calls: 5,
            max_file_bytes: 10,
            max_write_bytes: 12,
            max_writes: 2,
            max_new_files: 1,
            max_messages: 1,
            max_message_bytes: 4,
            ..EditorLimits::default()
        };
        let perms = [Permission::EditorRead, Permission::EditorWrite];
        let s = EditorSession::new(Box::new(book()), &perms, "T", limits);
        assert_eq!(
            call(
                &s,
                "write_file",
                json!({"path": "OEBPS/a.xhtml", "text": "01234567890"})
            )
            .0,
            LIMIT
        );
        assert_eq!(
            call(&s, "add_file", json!({"path": "n1", "text": "123456"})).0,
            OK
        );
        assert_eq!(
            call(&s, "add_file", json!({"path": "n2", "text": "1"})).0,
            LIMIT
        );
        assert_eq!(
            call(&s, "write_file", json!({"path": "n1", "text": "123456"})).0,
            OK
        );
        // 12 bytes written, writes == 2
        assert_eq!(
            call(&s, "write_file", json!({"path": "n1", "text": "1"})).0,
            LIMIT
        );
        let s2 = EditorSession::new(Box::new(book()), &perms, "T", limits);
        assert_eq!(
            call(&s2, "show_message", json!({"text": "héllo wörld"})).0,
            OK
        );
        assert_eq!(call(&s2, "show_message", json!({"text": "again"})).0, LIMIT);
        assert_eq!(s2.lock().unwrap().messages, vec!["h\u{e9}l"]);
        // call budget
        let s3 = EditorSession::new(Box::new(book()), &perms, "T", limits);
        for _ in 0..5 {
            assert_eq!(call(&s3, "current_file", json!({})).0, OK);
        }
        assert_eq!(call(&s3, "current_file", json!({})).0, LIMIT);
    }

    #[test]
    fn binary_files_round_trip_as_base64() {
        let mut b = book();
        b.files.insert("img.png".into(), vec![0, 159, 146, 150]);
        let s = session(true, true, b);
        let (c, v) = call(&s, "read_file", json!({"path": "img.png"}));
        assert_eq!(c, OK);
        let back = B64.decode(v["ok"]["b64"].as_str().unwrap()).unwrap();
        assert_eq!(back, vec![0, 159, 146, 150]);
        let (c, _) = call(&s, "write_file", json!({"path": "img.png", "b64": "!!"}));
        assert_eq!(c, BAD);
        let (c, _) = call(&s, "write_file", json!({"path": "img.png"}));
        assert_eq!(c, BAD);
    }

    #[test]
    fn selection_and_metadata() {
        let s = session(true, true, book());
        let (_, v) = call(&s, "get_selection", json!({}));
        assert_eq!(v["ok"]["text"], "a");
        assert_eq!(call(&s, "replace_selection", json!({"text": "b"})).0, OK);
        assert_eq!(call(&s, "set_metadata", json!({"title": "T"})).0, OK);
        let (_, v) = call(&s, "get_metadata", json!({}));
        assert_eq!(v["ok"]["title"], "T");
        let (_, v) = call(&s, "current_file", json!({}));
        assert_eq!(v["ok"], "OEBPS/a.xhtml");
        let mut empty = MemBook::default();
        assert!(empty.selection().is_none());
        assert!(empty.replace_selection("x").is_err());
    }
}
