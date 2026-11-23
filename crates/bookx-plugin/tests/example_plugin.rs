//! The worked example of `docs/plugin-development.md`: `examples/plugins/rawcopy-writer`
//! (WebAssembly text + manifest) is assembled, installed, enabled and used as an output format.

mod common;

use std::path::PathBuf;

use bookx_core::archive::Archive;
use bookx_core::contracts::{FormatWriter, Registry};
use bookx_plugin::{Host, Manifest, PluginKind, PluginManager};
use common::*;

fn example_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins/rawcopy-writer")
}

/// Assembles the example into `<root>/plugins/rawcopy`, like a user would install it.
fn install_example(root: &TempDir) -> PathBuf {
    let src = example_dir();
    let wat = std::fs::read_to_string(src.join("plugin.wat")).unwrap();
    let dir = root.path().join("plugins");
    install(
        &dir,
        "rawcopy",
        &std::fs::read_to_string(src.join("plugin.toml")).unwrap(),
        &wat::parse_str(wat).expect("the example assembles"),
    );
    dir
}

#[test]
fn the_example_manifest_is_valid() {
    let m = Manifest::parse(&std::fs::read_to_string(example_dir().join("plugin.toml")).unwrap())
        .unwrap();
    assert_eq!(m.id, "org.bookx.example.rawcopy");
    assert_eq!(m.kind, PluginKind::FormatWriter);
    assert_eq!(m.extension.as_deref(), Some("epubcopy"));
    assert!(m.permissions.is_empty());
}

#[test]
fn the_example_is_off_until_enabled_then_writes_the_book() {
    let root = TempDir::new("example");
    let dir = install_example(&root);
    let mut mgr = PluginManager::new(dir, root.path().join("state.json"), Host::default());
    mgr.scan();

    let mut writers: Registry<dyn FormatWriter> = Registry::new();
    assert!(mgr.register_writers(&mut writers).is_empty());
    assert_eq!(writers.len(), 0, "decision D5: disabled by default");

    mgr.set_enabled("org.bookx.example.rawcopy", true).unwrap();
    let errs = mgr.register_writers(&mut writers);
    assert!(errs.is_empty(), "{errs:?}");
    let w = writers.iter().next().expect("the writer is registered");
    assert_eq!(w.id(), "org.bookx.example.rawcopy");
    assert_eq!(w.extension(), "epubcopy");

    let book = sample_epub("Example");
    let out = w.write(&book).unwrap();
    assert_eq!(
        out,
        book.archive.to_bytes().unwrap(),
        "the example copies the book"
    );
    assert!(
        Archive::from_bytes(out)
            .unwrap()
            .contains("META-INF/container.xml")
    );
}

#[test]
fn the_example_handles_a_larger_book_and_bad_input() {
    let root = TempDir::new("example-big");
    let dir = install_example(&root);
    let mut mgr = PluginManager::new(dir, root.path().join("state.json"), Host::default());
    mgr.scan();
    mgr.set_enabled("org.bookx.example.rawcopy", true).unwrap();
    let p = mgr.plugin("org.bookx.example.rawcopy").unwrap();

    // bigger than the 2 MiB it starts with: memory grows on demand
    let big = base64_of(&vec![7u8; 3 * 1024 * 1024]);
    let v = p
        .call_json("write", &serde_json::json!({ "book_b64": big }))
        .unwrap();
    assert_eq!(v.as_str().unwrap(), big);

    assert!(
        p.call_json("read", &serde_json::json!({})).is_err(),
        "-2 for other operations"
    );
    assert!(
        p.call_json("write", &serde_json::json!({})).is_err(),
        "-3 for a too short input"
    );
}

fn base64_of(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
