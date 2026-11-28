//! The device-driver adapter with mock plugins: detection, sending inside the mount point,
//! listing, and the manager's registration.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use bookx_core::contracts::{DeviceDriver, DeviceInfo, Registry};
use bookx_plugin::{Host, MountProvider, PluginDevice, PluginManager};
use common::*;

fn mounts(list: Vec<PathBuf>) -> MountProvider {
    Arc::new(move || list.clone())
}

struct Setup {
    _dir: TempDir,
    mount: PathBuf,
}

fn setup() -> Setup {
    let dir = TempDir::new("device");
    let mount = dir.path().join("Reader");
    std::fs::create_dir_all(mount.join("system")).unwrap();
    Setup { _dir: dir, mount }
}

fn detect_reply(mount: &std::path::Path) -> Vec<u8> {
    ok(&format!(
        r#"[{{"id":"r1","name":"Plugin Reader","mount":{}}}]"#,
        serde_json::to_string(&mount.to_string_lossy()).unwrap()
    ))
}

fn driver(replies: &[(u8, Vec<u8>)], mount: &std::path::Path) -> PluginDevice {
    PluginDevice::new(
        arc(
            "device",
            "extensions = [\"epub\", \"txt\"]",
            &canned(replies),
        ),
        mounts(vec![mount.to_path_buf()]),
    )
    .unwrap()
}

fn device(s: &Setup) -> DeviceInfo {
    DeviceInfo {
        id: "test.plugin:r1".into(),
        name: "Plugin Reader".into(),
        mount: s.mount.clone(),
        parent: None,
    }
}

#[test]
fn detect_namespaces_ids_and_keeps_only_offered_mounts() {
    let s = setup();
    let d = driver(&[(b'd', detect_reply(&s.mount))], &s.mount);
    assert_eq!(d.id(), "test.plugin");
    assert_eq!(d.preferred_formats(), &["epub", "txt"]);
    let found = d.detect();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "test.plugin:r1");
    assert_eq!(found[0].name, "Plugin Reader");
    assert_eq!(found[0].mount, s.mount);

    // a mount the host never offered (the plugin cannot make the host touch other folders)
    let d = driver(
        &[(b'd', detect_reply(std::path::Path::new("/etc")))],
        &s.mount,
    );
    assert!(d.detect().is_empty());
}

#[test]
fn detect_survives_garbage_and_errors() {
    let s = setup();
    for reply in [
        ok("null"),
        ok("{}"),
        ok(r#"[{"id":"x"}]"#),
        ok(r#"[{"id":"","name":"n","mount":"/"}]"#),
        br#"{"err":"nope"}"#.to_vec(),
    ] {
        assert!(driver(&[(b'd', reply)], &s.mount).detect().is_empty());
    }
    // unsupported operation
    assert!(driver(&[], &s.mount).detect().is_empty());
}

#[test]
fn send_writes_inside_the_mount() {
    let s = setup();
    let d = driver(&[(b's', ok(r#"{"path":"books/sub/last.bin"}"#))], &s.mount);
    let at = d.send(&device(&s), "My Book.epub", b"BOOK").unwrap();
    assert_eq!(at, s.mount.join("books/sub/last.bin"));
    assert_eq!(std::fs::read(&at).unwrap(), b"BOOK");
}

#[test]
fn send_may_replace_the_bytes() {
    let s = setup();
    let reply = ok(&format!(
        r#"{{"path":"x.bin","data_b64":"{}"}}"#,
        B64.encode(b"CHANGED")
    ));
    let d = driver(&[(b's', reply)], &s.mount);
    let at = d.send(&device(&s), "b.epub", b"BOOK").unwrap();
    assert_eq!(std::fs::read(at).unwrap(), b"CHANGED");
}

#[test]
fn send_refuses_paths_that_leave_the_device() {
    let s = setup();
    for bad in ["../escape.bin", "/tmp/abs.bin", "a/../../b", ""] {
        let reply = ok(&format!(
            r#"{{"path":{}}}"#,
            serde_json::to_string(bad).unwrap()
        ));
        let d = driver(&[(b's', reply)], &s.mount);
        let e = d.send(&device(&s), "b.epub", b"BOOK").unwrap_err();
        assert!(
            e.to_string().contains("leaves the device") || e.to_string().contains("empty"),
            "{bad}: {e}"
        );
    }
    assert!(!s.mount.parent().unwrap().join("escape.bin").exists());
}

#[test]
fn send_checks_the_device_belongs_to_the_plugin_and_is_connected() {
    let s = setup();
    let d = driver(&[(b's', ok(r#"{"path":"a.bin"}"#))], &s.mount);
    let mut other = device(&s);
    other.id = "kobo:abc".into();
    assert!(d.send(&other, "b.epub", b"x").is_err());
    let mut gone = device(&s);
    gone.mount = s.mount.join("unplugged");
    assert!(
        d.send(&gone, "b.epub", b"x")
            .unwrap_err()
            .to_string()
            .contains("no longer connected")
    );
    // plugin errors come through
    let d = driver(&[(b's', br#"{"err":"disk full"}"#.to_vec())], &s.mount);
    assert!(
        format!("{:#}", d.send(&device(&s), "b.epub", b"x").unwrap_err()).contains("disk full")
    );
}

#[test]
fn list_joins_relative_paths_to_the_mount() {
    let s = setup();
    let d = driver(&[(b'l', ok(r#"["a.epub","books/b.epub"]"#))], &s.mount);
    assert_eq!(
        d.list(&device(&s)).unwrap(),
        vec![s.mount.join("a.epub"), s.mount.join("books/b.epub")]
    );
    let d = driver(&[(b'l', ok(r#"["../../etc/passwd"]"#))], &s.mount);
    assert!(d.list(&device(&s)).is_err());
    let d = driver(&[(b'l', ok(r#"{"a":1}"#))], &s.mount);
    assert!(d.list(&device(&s)).is_err());
}

#[test]
fn only_device_plugins_are_accepted() {
    let s = setup();
    let e = PluginDevice::new(
        arc("transform", "", &canned(&[])),
        mounts(vec![s.mount.clone()]),
    )
    .err()
    .unwrap();
    assert!(e.to_string().contains("not a device"));
}

#[test]
fn the_manager_registers_enabled_device_plugins_only() {
    let s = setup();
    let root = TempDir::new("device-mgr");
    install(
        &root.path().join("plugins"),
        "dev",
        &manifest_toml("test.dev", "device", "extensions = [\"epub\"]"),
        &canned(&[(b'd', detect_reply(&s.mount))]),
    );
    let mut m = PluginManager::new(
        root.path().join("plugins"),
        root.path().join("state.json"),
        Host::default(),
    );
    m.scan();
    let mut reg: Registry<dyn DeviceDriver> = Registry::new();
    assert!(
        m.register_devices(&mut reg, mounts(vec![s.mount.clone()]))
            .is_empty()
    );
    assert_eq!(reg.len(), 0, "plugins start disabled");
    m.set_enabled("test.dev", true).unwrap();
    let errs = m.register_devices(&mut reg, mounts(vec![s.mount.clone()]));
    assert!(errs.is_empty(), "{errs:?}");
    assert_eq!(reg.len(), 1);
    let found = reg.iter().next().unwrap().detect();
    assert_eq!(found[0].id, "test.dev:r1");
}
