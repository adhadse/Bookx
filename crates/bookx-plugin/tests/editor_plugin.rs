//! ABI v2 editor plugins: the host calls, permissions, limits and sandbox escape attempts, plus the
//! real Rust example (`examples/plugins/trim-whitespace`).

mod common;

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};

use bookx_plugin::{
    EditorApi, EditorLimits, EditorSession, Host, Manifest, MemBook, Permission, Plugin,
    SharedSession,
};
use common::*;
use serde_json::{Value, json};

fn manifest_text(perms: &str, extra: &str) -> String {
    format!(
        "id = \"test.editor\"\nname = \"Ed\"\nversion = \"1.0\"\napi_version = 2\nkind = \"editor-action\"\n\
         permissions = [{perms}]\n{extra}\n[[action]]\nid = \"run\"\nlabel = \"Run\"\n"
    )
}

fn perms(p: &[&str]) -> Vec<Permission> {
    p.iter().map(|s| Permission::parse(s).unwrap()).collect()
}

const IMPORT_EDITOR: &str = r#"(import "bookx_host" "editor_call" (func $editor_call (param i32 i32 i32 i32 i32) (result i32)))"#;

/// A plugin whose only action performs one fixed host call and returns the host's own reply.
fn prober(op: &str, input: &str) -> Vec<u8> {
    let body = format!(
        r#"(func (export "bookx_call") (param i32 i32 i32 i32) (result i64)
             (drop (call $editor_call (i32.const 2048) (i32.const {}) (i32.const 3072) (i32.const {}) (i32.const 256)))
             (i64.or
               (i64.shl (i64.extend_i32_u (i32.load (i32.const 256))) (i64.const 32))
               (i64.extend_i32_u (i32.load (i32.const 260)))))"#,
        op.len(),
        input.len()
    );
    module(
        IMPORT_EDITOR,
        &body,
        &[
            (2048, op.as_bytes().to_vec()),
            (3072, input.as_bytes().to_vec()),
        ],
    )
}

fn load(manifest: &str, wasm: &[u8]) -> anyhow::Result<Plugin> {
    Plugin::from_bytes(Manifest::parse(manifest)?, wasm, Host::default())
}

fn book() -> MemBook {
    let mut b = MemBook::default();
    b.files
        .insert("OEBPS/a.xhtml".into(), b"<p>a</p>  \n".to_vec());
    b.files
        .insert("mimetype".into(), b"application/epub+zip".to_vec());
    b.current = Some("OEBPS/a.xhtml".into());
    b
}

/// Records what the host did to the book.
#[derive(Clone, Default)]
struct Spy(Arc<Mutex<Vec<String>>>);

struct SpyBook {
    inner: MemBook,
    log: Spy,
}

impl EditorApi for SpyBook {
    fn list_files(&mut self) -> Vec<String> {
        self.inner.list_files()
    }
    fn read_file(&mut self, p: &str) -> Option<Vec<u8>> {
        self.inner.read_file(p)
    }
    fn write_file(&mut self, p: &str, d: &[u8]) -> Result<(), String> {
        self.log.0.lock().unwrap().push(format!("write {p}"));
        self.inner.write_file(p, d)
    }
    fn add_file(&mut self, p: &str, d: &[u8]) -> Result<(), String> {
        self.log.0.lock().unwrap().push(format!("add {p}"));
        self.inner.add_file(p, d)
    }
    fn rename_file(&mut self, f: &str, t: &str) -> Result<usize, String> {
        self.log.0.lock().unwrap().push(format!("rename {f} {t}"));
        self.inner.rename_file(f, t)
    }
    fn current_file(&mut self) -> Option<String> {
        self.inner.current_file()
    }
    fn selection(&mut self) -> Option<(String, String)> {
        self.inner.selection()
    }
    fn replace_selection(&mut self, t: &str) -> Result<(), String> {
        self.inner.replace_selection(t)
    }
    fn insert_at_cursor(&mut self, t: &str) -> Result<(), String> {
        self.inner.insert_at_cursor(t)
    }
    fn metadata(&mut self) -> Value {
        self.inner.metadata()
    }
    fn set_metadata(&mut self, f: &Value) -> Result<(), String> {
        self.inner.set_metadata(f)
    }
    fn checkpoint(&mut self, n: &str) -> Result<(), String> {
        self.log.0.lock().unwrap().push(format!("checkpoint {n}"));
        self.inner.checkpoint(n)
    }
}

fn session(p: &[&str], spy: &Spy, limits: EditorLimits) -> SharedSession {
    EditorSession::new(
        Box::new(SpyBook {
            inner: book(),
            log: spy.clone(),
        }),
        &perms(p),
        "Run",
        limits,
    )
}

fn probe(perm: &[&str], op: &str, input: &str) -> (anyhow::Result<Value>, Vec<String>) {
    let perm_text = perm
        .iter()
        .map(|p| format!("\"{p}\""))
        .collect::<Vec<_>>()
        .join(",");
    let p = load(&manifest_text(&perm_text, ""), &prober(op, input)).unwrap();
    let spy = Spy::default();
    let s = session(perm, &spy, EditorLimits::default());
    let r = p.run_editor_action("run", &s).map(|r| r.reply);
    let log = spy.0.lock().unwrap().clone();
    (r, log)
}

#[test]
fn a_permitted_write_is_checkpointed_first() {
    let (r, log) = probe(
        &["editor.read", "editor.write"],
        "write_file",
        r#"{"path":"OEBPS/a.xhtml","text":"x"}"#,
    );
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(log, vec!["checkpoint Before: Run", "write OEBPS/a.xhtml"]);
}

#[test]
fn permissions_are_enforced_inside_the_sandbox() {
    for (granted, op, input) in [
        (vec![], "list_files", "{}"),
        (vec![], "read_file", r#"{"path":"OEBPS/a.xhtml"}"#),
        (
            vec!["editor.read"],
            "write_file",
            r#"{"path":"OEBPS/a.xhtml","text":"x"}"#,
        ),
        (
            vec!["editor.read"],
            "add_file",
            r#"{"path":"n.xhtml","text":"x"}"#,
        ),
        (
            vec!["editor.read"],
            "rename_file",
            r#"{"from":"OEBPS/a.xhtml","to":"b.xhtml"}"#,
        ),
        (vec!["editor.read"], "add_checkpoint", r#"{"name":"x"}"#),
        (
            vec!["editor.write"],
            "read_file",
            r#"{"path":"OEBPS/a.xhtml"}"#,
        ),
    ] {
        let (r, log) = probe(&granted, op, input);
        let e = format!("{:#}", r.unwrap_err());
        assert!(e.contains("permission"), "{op}: {e}");
        assert!(log.is_empty(), "{op} reached the book: {log:?}");
    }
}

#[test]
fn escape_attempts_never_reach_the_book() {
    let both = ["editor.read", "editor.write"];
    for path in [
        "../../etc/passwd",
        "/etc/passwd",
        "OEBPS/../../x",
        "file:///etc/passwd",
        "C:\\\\x",
        "mimetype",
        "META-INF/container.xml",
    ] {
        for op in ["write_file", "add_file"] {
            let (r, log) = probe(&both, op, &format!(r#"{{"path":"{path}","text":"x"}}"#));
            assert!(r.is_err(), "{op} {path}");
            assert!(log.is_empty(), "{op} {path}: {log:?}");
        }
        if path != "mimetype" {
            let (r, _) = probe(&both, "read_file", &format!(r#"{{"path":"{path}"}}"#));
            assert!(r.is_err(), "read {path}");
        }
    }
    let (r, log) = probe(
        &both,
        "rename_file",
        r#"{"from":"OEBPS/a.xhtml","to":"../b"}"#,
    );
    assert!(r.is_err() && log.is_empty());
    let (r, _) = probe(&both, "exec", r#"{"cmd":"ls"}"#);
    assert!(r.is_err());
    let (r, _) = probe(&both, "http_get", "{}");
    assert!(r.is_err());
}

#[test]
fn bad_input_is_an_error_not_a_crash() {
    for input in ["", "not json", "[1,2]", "\u{0}", "{\"path\":5}"] {
        let (r, _) = probe(&["editor.read", "editor.write"], "read_file", input);
        assert!(r.is_err(), "{input:?}");
    }
}

#[test]
fn editor_call_needs_the_editor_kind() {
    let wasm = prober("list_files", "{}");
    let transform =
        "id = \"t.t\"\nname = \"T\"\nversion = \"1\"\napi_version = 2\nkind = \"transform\"\n";
    let e = format!("{:#}", load(transform, &wasm).unwrap_err());
    assert!(e.contains("only for editor-action"), "{e}");
    // and without a session (e.g. a plain call) the import answers -6, never touches a book
    let p = load(&manifest_text("\"editor.read\"", ""), &wasm).unwrap();
    assert!(
        p.call_json("run_action", &json!({"action": "run"}))
            .is_err()
    );
}

#[test]
fn run_editor_action_checks_kind_and_action_id() {
    let spy = Spy::default();
    let s = session(&["editor.read"], &spy, EditorLimits::default());
    let p = load(
        &manifest_text("\"editor.read\"", ""),
        &prober("list_files", "{}"),
    )
    .unwrap();
    let e = format!("{:#}", p.run_editor_action("other", &s).unwrap_err());
    assert!(e.contains("declares no action"), "{e}");
    let wasm = canned(&[(b'r', br#"{"ok":null}"#.to_vec())]);
    let t = Plugin::from_bytes(manifest("a.b", "transform", ""), &wasm, Host::default()).unwrap();
    let e = format!("{:#}", t.run_editor_action("run", &s).unwrap_err());
    assert!(e.contains("not an editor-action"), "{e}");
}

#[test]
fn a_runaway_action_is_cut_off() {
    let body = r#"(func (export "bookx_call") (param i32 i32 i32 i32) (result i64)
        (loop $l (br $l)) (i64.const 0))"#;
    let wasm = module(IMPORT_EDITOR, body, &[]);
    let p = load(&manifest_text("\"editor.write\"", "fuel = 100000"), &wasm).unwrap();
    let spy = Spy::default();
    let s = session(&["editor.write"], &spy, EditorLimits::default());
    let e = format!("{:#}", p.run_editor_action("run", &s).unwrap_err());
    assert!(e.contains("budget"), "{e}");
    // the book is handed back untouched
    assert!(spy.0.lock().unwrap().is_empty());
    let _ = Arc::try_unwrap(s)
        .ok()
        .unwrap()
        .into_inner()
        .unwrap()
        .into_api();
}

#[test]
fn the_call_budget_applies_inside_the_guest() {
    // a loop calling the host 100 times against a limit of 10 ends with the limit error code,
    // the action still returns normally and the host stops doing work
    let body = r#"(func (export "bookx_call") (param i32 i32 i32 i32) (result i64)
        (local $i i32)
        (loop $l
          (drop (call $editor_call (i32.const 2048) (i32.const 12) (i32.const 3072) (i32.const 2) (i32.const 256)))
          (local.set $i (i32.add (local.get $i) (i32.const 1)))
          (br_if $l (i32.lt_u (local.get $i) (i32.const 100))))
        (i64.or
               (i64.shl (i64.extend_i32_u (i32.load (i32.const 256))) (i64.const 32))
               (i64.extend_i32_u (i32.load (i32.const 260)))))"#;
    let wasm = module(
        IMPORT_EDITOR,
        body,
        &[(2048, b"current_file".to_vec()), (3072, b"{}".to_vec())],
    );
    let p = load(&manifest_text("\"editor.read\"", ""), &wasm).unwrap();
    let spy = Spy::default();
    let limits = EditorLimits {
        max_calls: 10,
        ..EditorLimits::default()
    };
    let s = session(&["editor.read"], &spy, limits);
    let e = format!("{:#}", p.run_editor_action("run", &s).unwrap_err());
    assert!(e.contains("too many host calls"), "{e}");
    assert_eq!(s.lock().unwrap().calls(), 100);
}

#[test]
fn messages_are_collected() {
    let (r, _) = probe(&[], "show_message", r#"{"text":"hello"}"#);
    assert!(r.is_ok());
    let p = load(
        &manifest_text("", ""),
        &prober("show_message", r#"{"text":"hello"}"#),
    )
    .unwrap();
    let spy = Spy::default();
    let s = session(&[], &spy, EditorLimits::default());
    let r = p.run_editor_action("run", &s).unwrap();
    assert_eq!(r.messages, vec!["hello"]);
}

// ---------------------------------------------------------------------------------------------
// the real Rust example

fn example_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins/trim-whitespace")
}

/// Builds the example for wasm32 (offline); falls back to the committed `plugin.wasm`.
fn example_wasm() -> Vec<u8> {
    static BUILT: OnceLock<Option<PathBuf>> = OnceLock::new();
    let built = BUILT.get_or_init(|| {
        if std::env::var("BOOKX_NO_WASM_BUILD").as_deref() == Ok("1") {
            return None;
        }
        let sysroot = Command::new("rustc")
            .args(["--print", "sysroot"])
            .output()
            .ok()?;
        let sysroot = String::from_utf8(sysroot.stdout).ok()?;
        if !PathBuf::from(sysroot.trim())
            .join("lib/rustlib/wasm32-unknown-unknown")
            .is_dir()
        {
            return None;
        }
        let target = std::env::temp_dir().join("bookx-example-trim-target");
        let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
            .args([
                "build",
                "--release",
                "--offline",
                "--target",
                "wasm32-unknown-unknown",
            ])
            .current_dir(example_dir())
            .env("CARGO_TARGET_DIR", &target)
            .output()
            .ok()?;
        assert!(
            out.status.success(),
            "building the example failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        Some(target.join("wasm32-unknown-unknown/release/bookx_example_trim_whitespace.wasm"))
    });
    match built {
        Some(p) => std::fs::read(p).unwrap(),
        None => std::fs::read(example_dir().join("plugin.wasm")).unwrap(),
    }
}

fn example_plugin() -> Plugin {
    let text = std::fs::read_to_string(example_dir().join("plugin.toml")).unwrap();
    load(&text, &example_wasm()).unwrap()
}

#[test]
fn the_example_manifest_is_valid_and_the_binary_loads() {
    let text = std::fs::read_to_string(example_dir().join("plugin.toml")).unwrap();
    let m = Manifest::parse(&text).unwrap();
    assert_eq!(m.actions.len(), 1);
    assert_eq!(m.permissions, perms(&["editor.read", "editor.write"]));
    // the committed binary is a valid editor plugin too
    let committed = std::fs::read(example_dir().join("plugin.wasm")).unwrap();
    assert!(load(&text, &committed).is_ok());
}

#[test]
fn the_example_trims_trailing_whitespace_in_one_checkpoint() {
    let mut b = book();
    b.files.insert(
        "OEBPS/b.xhtml".into(),
        "<p>one</p> \t\r\n<p>two</p>  \n<p>é \"q\" \\</p> \n"
            .as_bytes()
            .to_vec(),
    );
    b.files.insert("OEBPS/s.css".into(), b"p { }   \n".to_vec());
    b.files
        .insert("OEBPS/clean.xhtml".into(), b"<p>ok</p>\n".to_vec());
    b.files
        .insert("OEBPS/img.png".into(), vec![0, 1, 2, 32, 10]);
    let spy = Spy::default();
    let s = EditorSession::new(
        Box::new(SpyBook {
            inner: b,
            log: spy.clone(),
        }),
        &perms(&["editor.read", "editor.write"]),
        "Trim Trailing Whitespace",
        EditorLimits::default(),
    );
    let r = example_plugin().run_editor_action("trim", &s).unwrap();
    assert_eq!(r.reply, json!("Trimmed 3 files"));
    assert_eq!(r.messages, vec!["Trimmed 3 files"]);
    assert_eq!(r.changes, 3);
    let log = spy.0.lock().unwrap().clone();
    assert_eq!(log[0], "checkpoint Before: Trim Trailing Whitespace");
    assert_eq!(
        log.iter().filter(|l| l.starts_with("checkpoint")).count(),
        1
    );
    let mut written: Vec<_> = log[1..].to_vec();
    written.sort();
    assert_eq!(
        written,
        vec![
            "write OEBPS/a.xhtml",
            "write OEBPS/b.xhtml",
            "write OEBPS/s.css"
        ]
    );
    let mut api = Arc::try_unwrap(s)
        .ok()
        .unwrap()
        .into_inner()
        .unwrap()
        .into_api();
    assert_eq!(api.read_file("OEBPS/a.xhtml").unwrap(), b"<p>a</p>\n");
    assert_eq!(
        String::from_utf8(api.read_file("OEBPS/b.xhtml").unwrap()).unwrap(),
        "<p>one</p>\r\n<p>two</p>\n<p>\u{e9} \"q\" \\</p>\n"
    );
    assert_eq!(
        api.read_file("OEBPS/img.png").unwrap(),
        vec![0, 1, 2, 32, 10]
    );
}

#[test]
fn the_example_with_only_read_permission_cannot_change_anything() {
    let spy = Spy::default();
    let text = std::fs::read_to_string(example_dir().join("plugin.toml"))
        .unwrap()
        .replace("\"editor.read\", \"editor.write\"", "\"editor.read\"");
    let p = load(&text, &example_wasm()).unwrap();
    let s = session(&["editor.read"], &spy, EditorLimits::default());
    let e = format!("{:#}", p.run_editor_action("trim", &s).unwrap_err());
    assert!(e.contains("permission"), "{e}");
    assert!(spy.0.lock().unwrap().is_empty());
}

#[test]
fn the_example_on_a_clean_book_changes_nothing() {
    let mut b = MemBook::default();
    b.files.insert("a.xhtml".into(), b"<p>x</p>\n".to_vec());
    let spy = Spy::default();
    let s = EditorSession::new(
        Box::new(SpyBook {
            inner: b,
            log: spy.clone(),
        }),
        &perms(&["editor.read", "editor.write"]),
        "Trim",
        EditorLimits::default(),
    );
    let r = example_plugin().run_editor_action("trim", &s).unwrap();
    assert_eq!(r.reply, json!("No trailing whitespace found"));
    assert_eq!(r.changes, 0);
    assert!(spy.0.lock().unwrap().is_empty());
}

#[test]
fn the_manager_lists_actions_keeps_the_plugin_off_and_hands_it_out_once_enabled() {
    let d = TempDir::new("ed-mgr");
    let dest = d.path().join("trim");
    std::fs::create_dir_all(&dest).unwrap();
    for f in ["plugin.toml", "plugin.wasm"] {
        std::fs::copy(example_dir().join(f), dest.join(f)).unwrap();
    }
    let mut m =
        bookx_plugin::PluginManager::new(d.path(), d.path().join("state.json"), Host::default());
    m.scan();
    let st = m.list().remove(0);
    assert!(!st.enabled);
    assert_eq!(st.permissions, vec!["editor.read", "editor.write"]);
    assert_eq!(st.actions, vec!["Trim Trailing Whitespace (<Ctrl><Alt>t)"]);
    assert!(m.editor_plugins().0.is_empty(), "disabled by default");
    assert!(m.plugin(&st.id).is_err());
    m.set_enabled(&st.id, true).unwrap();
    assert_eq!(m.editor_plugins().0.len(), 1);
}
