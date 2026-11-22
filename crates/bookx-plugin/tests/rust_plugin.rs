//! A plugin compiled from real Rust (`tests/fixtures/rust_plugin`). Skipped when the
//! `wasm32-unknown-unknown` target is not installed or `BOOKX_NO_WASM_BUILD=1`.

mod common;

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, OnceLock};

use bookx_core::contracts::{MetadataQuery, MetadataSource};
use bookx_plugin::{Host, PluginMetadataSource};
use common::*;
use serde_json::json;

fn build() -> Option<&'static PathBuf> {
    static WASM: OnceLock<Option<PathBuf>> = OnceLock::new();
    WASM.get_or_init(|| {
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
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rust_plugin");
        let target = std::env::temp_dir().join("bookx-rust-plugin-target");
        let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
            .args([
                "build",
                "--release",
                "--offline",
                "--target",
                "wasm32-unknown-unknown",
            ])
            .current_dir(&fixture)
            .env("CARGO_TARGET_DIR", &target)
            .env_remove("CARGO_PROFILE_DEV_DEBUG")
            .output()
            .ok()?;
        assert!(
            out.status.success(),
            "building the Rust fixture plugin failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        Some(target.join("wasm32-unknown-unknown/release/bookx_test_rust_plugin.wasm"))
    })
    .as_ref()
}

macro_rules! need_wasm {
    () => {
        match build() {
            Some(p) => std::fs::read(p).unwrap(),
            None => return,
        }
    };
}

#[test]
fn rust_compiled_plugin_loads_and_answers() {
    let wasm = need_wasm!();
    let log = Arc::new(VecLog::default());
    let host = Host::default().with_log(log.clone());
    let p = Arc::new(plugin_host("metadata-source", "", &wasm, host));
    let info = p.info().unwrap().unwrap();
    assert_eq!(info["name"], "rust fixture");

    let src = PluginMetadataSource::new(p).unwrap();
    let r = src
        .lookup(&MetadataQuery {
            title: "Dune".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].title, "Dune (found)");
    assert_eq!(r[0].authors, vec!["Rust Plugin"]);
    assert_eq!(r[0].rating, 10, "now_ms must be a real clock");
    let lines = log.0.lock().unwrap();
    assert_eq!(lines[0].2, "op lookup");
    assert_eq!(lines[0].1, 2);
}

#[test]
fn rust_plugin_unsupported_op_and_panic_are_errors() {
    let wasm = need_wasm!();
    let p = plugin("metadata-source", "", &wasm);
    let e = format!("{:#}", p.call_json("nope", &json!({})).unwrap_err());
    assert!(e.contains("does not support"), "{e}");
    // panic = "abort" compiles to `unreachable`: a trap, not a host crash
    let e = format!("{:#}", p.call_json("panic", &json!({})).unwrap_err());
    assert!(e.contains("trapped"), "{e}");
    assert!(
        p.call_json("lookup", &json!({"title": "Still fine"}))
            .is_ok()
    );
}

#[test]
fn rust_plugin_with_tight_limits() {
    let wasm = need_wasm!();
    let p = plugin("metadata-source", "fuel = 100", &wasm);
    assert!(format!("{:#}", p.call_json("lookup", &json!({})).unwrap_err()).contains("budget"));
    let p = plugin("metadata-source", "memory_pages = 1", &wasm);
    assert!(p.call_json("lookup", &json!({})).is_err());
}
