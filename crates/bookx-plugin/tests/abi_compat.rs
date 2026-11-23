//! Compatibility across releases: plugins built against an older host keep working.
//!
//! `tests/golden/` holds plugin binaries that were built once and are never rebuilt by the tests
//! (`SHA256SUMS` next to them is checked by `build-aux/release-check.sh`): an ABI v1 format writer and an
//! ABI v2 editor action. If a host change makes one of them stop loading or behave differently, this
//! test fails, and the change is an ABI break that needs a new `api_version` (see the stability policy in
//! `docs/plugin-development.md`).

mod common;

use std::path::PathBuf;

use bookx_plugin::{
    API_VERSION, API_VERSION_EDITOR, EditorLimits, EditorSession, Host, Manifest, MemBook,
    Permission, Plugin, PluginKind,
};
use common::*;
use serde_json::json;

fn golden(name: &str, file: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
        .join(file);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn load_golden(name: &str) -> Plugin {
    let manifest = String::from_utf8(golden(name, "plugin.toml")).unwrap();
    Plugin::from_bytes(
        Manifest::parse(&manifest).expect("the golden manifest still parses"),
        &golden(name, "plugin.wasm"),
        Host::default(),
    )
    .expect("the golden binary still loads")
}

#[test]
fn the_host_still_speaks_both_abi_versions() {
    assert_eq!(API_VERSION, 1);
    assert_eq!(API_VERSION_EDITOR, 2);
}

#[test]
fn golden_v1_format_writer_loads_and_writes() {
    let p = load_golden("v1-rawcopy");
    assert_eq!(p.manifest().api_version, 1);
    assert_eq!(p.manifest().kind, PluginKind::FormatWriter);
    assert_eq!(p.manifest().extension.as_deref(), Some("epubcopy"));

    let b64 = epub_b64("Golden");
    let out = p
        .call_json("write", &json!({ "book_b64": b64 }))
        .expect("the v1 call convention is unchanged");
    assert_eq!(out.as_str().unwrap(), b64, "the plugin copies the book");
}

#[test]
fn golden_v1_error_codes_are_unchanged() {
    let p = load_golden("v1-rawcopy");
    assert!(
        p.call_json("read", &json!({})).is_err(),
        "-2 unsupported operation"
    );
    assert!(
        p.call_json("write", &json!({})).is_err(),
        "-3 invalid input"
    );
}

#[test]
fn golden_v1_runs_in_a_fresh_instance_every_time() {
    let p = load_golden("v1-rawcopy");
    for i in 0..3 {
        let b64 = epub_b64(&format!("Golden {i}"));
        let out = p.call_json("write", &json!({ "book_b64": b64 })).unwrap();
        assert_eq!(out.as_str().unwrap(), b64);
    }
}

#[test]
fn golden_v2_editor_action_loads_and_edits_through_the_host_calls() {
    let p = load_golden("v2-trim-whitespace");
    assert_eq!(p.manifest().api_version, 2);
    assert_eq!(p.manifest().kind, PluginKind::EditorAction);
    assert_eq!(p.manifest().actions.len(), 1);

    let mut book = MemBook::default();
    book.files
        .insert("OEBPS/a.xhtml".into(), b"<p>a</p>  \n<p>b</p>\t\n".to_vec());
    book.files.insert("OEBPS/c.css".into(), b"p { }\n".to_vec());
    let perms = [
        Permission::parse("editor.read").unwrap(),
        Permission::parse("editor.write").unwrap(),
    ];
    let session = EditorSession::new(
        Box::new(book),
        &perms,
        "Trim Trailing Whitespace",
        EditorLimits::default(),
    );
    let r = p.run_editor_action("trim", &session).unwrap();
    assert_eq!(r.reply, json!("Trimmed 1 file"));
    assert_eq!(r.changes, 1);
    let mut api = std::sync::Arc::try_unwrap(session)
        .ok()
        .unwrap()
        .into_inner()
        .unwrap()
        .into_api();
    assert_eq!(
        api.read_file("OEBPS/a.xhtml").unwrap(),
        b"<p>a</p>\n<p>b</p>\n"
    );
}

#[test]
fn golden_v2_without_the_write_permission_is_refused_by_the_host() {
    let p = load_golden("v2-trim-whitespace");
    let perms = [Permission::parse("editor.read").unwrap()];
    let mut book = MemBook::default();
    book.files.insert("a.xhtml".into(), b"<p>a</p> \n".to_vec());
    let session = EditorSession::new(Box::new(book), &perms, "Trim", EditorLimits::default());
    assert!(p.run_editor_action("trim", &session).is_err());
}

#[test]
fn a_v1_manifest_cannot_claim_the_editor_kind() {
    let text = String::from_utf8(golden("v2-trim-whitespace", "plugin.toml"))
        .unwrap()
        .replace("api_version = 2", "api_version = 1");
    assert!(Manifest::parse(&text).is_err());
}
