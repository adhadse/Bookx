//! Discovery, enabling, lazy loading and hot reload.

mod common;

use std::sync::Arc;

use bookx_core::contracts::{FormatReader, FormatWriter, MetadataSource, Registry, Transform};
use bookx_plugin::{Host, PluginKind, PluginManager};
use common::*;
use serde_json::json;

struct Env {
    root: TempDir,
}

impl Env {
    fn new() -> Env {
        Env {
            root: TempDir::new("mgr"),
        }
    }
    fn plugins(&self) -> std::path::PathBuf {
        self.root.path().join("plugins")
    }
    fn state(&self) -> std::path::PathBuf {
        self.root.path().join("state").join("plugins.json")
    }
    fn manager(&self) -> PluginManager {
        PluginManager::new(self.plugins(), self.state(), Host::default())
    }
    fn add(&self, folder: &str, id: &str, kind: &str, extra: &str, wasm: &[u8]) {
        install(
            &self.plugins(),
            folder,
            &manifest_toml(id, kind, extra),
            wasm,
        );
    }
}

fn status<'a>(l: &'a [bookx_plugin::PluginStatus], id: &str) -> &'a bookx_plugin::PluginStatus {
    l.iter()
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("no {id} in {l:?}"))
}

#[test]
fn missing_or_empty_directory_is_fine() {
    let env = Env::new();
    let mut m = env.manager();
    let r = m.scan();
    assert!(r.added.is_empty() && r.changed.is_empty() && r.removed.is_empty());
    assert!(m.list().is_empty());
    std::fs::create_dir_all(env.plugins()).unwrap();
    std::fs::write(env.plugins().join("stray-file.txt"), "x").unwrap();
    assert!(m.scan().added.is_empty());
    assert_eq!(m.dir(), env.plugins());
}

#[test]
fn discovers_valid_plugins_disabled_and_unloaded() {
    let env = Env::new();
    env.add(
        "one",
        "org.one",
        "transform",
        "description = \"First\"\npermissions = [\"network:a.org\"]",
        &echo(),
    );
    env.add("two", "org.two", "metadata-source", "", &echo());
    let mut m = env.manager();
    let r = m.scan();
    assert_eq!(r.added, vec!["org.one", "org.two"]);
    let l = m.list();
    assert_eq!(l.len(), 2);
    let one = status(&l, "org.one");
    assert_eq!(one.name, "Test org.one");
    assert_eq!(one.version, "1.0.0");
    assert_eq!(one.kind, Some(PluginKind::Transform));
    assert_eq!(one.description, "First");
    assert_eq!(one.permissions, vec!["network:a.org"]);
    assert_eq!(one.dir, env.plugins().join("one"));
    assert!(!one.enabled && !one.loaded && one.error.is_none());
    assert!(!m.is_enabled("org.one"));
}

#[test]
fn disabled_plugins_cannot_be_used() {
    let env = Env::new();
    env.add("one", "org.one", "transform", "", &echo());
    let mut m = env.manager();
    m.scan();
    let e = m.plugin("org.one").unwrap_err().to_string();
    assert!(e.contains("is disabled"), "{e}");
    let e = m.plugin("org.nope").unwrap_err().to_string();
    assert!(e.contains("no plugin with id"), "{e}");
}

#[test]
fn enabling_is_persisted_and_loads_lazily() {
    let env = Env::new();
    env.add("one", "org.one", "transform", "", &always(&ok("42")));
    let mut m = env.manager();
    m.scan();
    m.set_enabled("org.one", true).unwrap();
    // enabling validated the module, so it is cached already
    let l = m.list();
    assert!(status(&l, "org.one").enabled && status(&l, "org.one").loaded);
    let text = std::fs::read_to_string(env.state()).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v, json!({"enabled": ["org.one"]}));
    assert!(!env.state().with_extension("json.tmp").exists());

    let p = m.plugin("org.one").unwrap();
    assert_eq!(p.call_json("x", &json!(null)).unwrap(), json!(42));

    // a new manager (next app start) sees the state, but nothing is loaded until used
    let mut m2 = env.manager();
    m2.scan();
    let l = m2.list();
    assert!(status(&l, "org.one").enabled);
    assert!(!status(&l, "org.one").loaded);
    m2.plugin("org.one").unwrap();
    assert!(status(&m2.list(), "org.one").loaded);

    m2.set_enabled("org.one", false).unwrap();
    assert!(m2.plugin("org.one").is_err());
    let mut m3 = env.manager();
    m3.scan();
    assert!(!status(&m3.list(), "org.one").enabled);
}

#[test]
fn enabling_unknown_or_broken_plugins_fails_and_stays_disabled() {
    let env = Env::new();
    env.add("bad", "org.bad", "transform", "", b"not wasm at all");
    let mut m = env.manager();
    m.scan();
    let e = format!("{:#}", m.set_enabled("org.nothing", true).unwrap_err());
    assert!(e.contains("no plugin with id"), "{e}");
    let e = format!("{:#}", m.set_enabled("org.bad", true).unwrap_err());
    assert!(
        e.contains("cannot enable `org.bad`") && e.contains("invalid WebAssembly"),
        "{e}"
    );
    assert!(!m.is_enabled("org.bad"));
    assert!(!env.state().exists());
    let l = m.list();
    assert!(status(&l, "org.bad")
        .error
        .as_deref()
        .unwrap()
        .contains("invalid WebAssembly"));
    assert!(!status(&l, "org.bad").loaded);
    // disabling something unknown also fails
    assert!(m.set_enabled("org.nothing", false).is_err());
}

#[test]
fn invalid_manifests_are_listed_with_their_error() {
    let env = Env::new();
    let w = echo();
    let bad = |folder: &str, toml: &str| install(&env.plugins(), folder, toml, &w);
    bad("unknown-kind", &manifest_toml("a.kind", "teleporter", ""));
    bad(
        "old-api",
        &manifest_toml("a.api", "transform", "").replace("api_version = 1", "api_version = 7"),
    );
    bad(
        "bad-perm",
        &manifest_toml("a.perm", "transform", "permissions = [\"fs:read\"]"),
    );
    bad("bad-toml", "id = ");
    bad(
        "no-wasm",
        &manifest_toml("a.nowasm", "transform", "wasm = \"missing.wasm\""),
    );
    std::fs::create_dir_all(env.plugins().join("empty-folder")).unwrap();
    env.add("good", "a.good", "transform", "", &w);
    let mut m = env.manager();
    m.scan();
    let l = m.list();
    assert_eq!(l.len(), 7);
    let expect = [
        ("unknown-kind", "unknown kind `teleporter`"),
        ("old-api", "api_version 7"),
        ("bad-perm", "never granted"),
        ("bad-toml", "invalid TOML"),
        ("no-wasm", "`missing.wasm` is missing"),
        ("empty-folder", "plugin.toml"),
    ];
    for (id, msg) in expect {
        let s = status(&l, id);
        assert!(
            s.error.as_deref().unwrap().contains(msg),
            "{id}: {:?}",
            s.error
        );
        assert!(!s.enabled && s.kind.is_none() && !s.loaded);
        assert_eq!(s.name, id);
        let e = format!("{:#}", m.set_enabled(id, true).unwrap_err());
        assert!(e.contains(msg), "{id}: {e}");
        assert!(m.plugin(id).is_err());
    }
    assert!(status(&l, "a.good").error.is_none());
}

#[test]
fn duplicate_ids_keep_the_first_folder() {
    let env = Env::new();
    env.add("a-first", "dup.id", "transform", "", &always(&ok("1")));
    env.add("b-second", "dup.id", "transform", "", &always(&ok("2")));
    let mut m = env.manager();
    m.scan();
    let l = m.list();
    assert_eq!(l.len(), 2);
    assert!(status(&l, "dup.id").error.is_none());
    assert_eq!(status(&l, "dup.id").dir, env.plugins().join("a-first"));
    let dup = status(&l, "b-second");
    assert!(dup
        .error
        .as_deref()
        .unwrap()
        .contains("duplicate plugin id `dup.id`"));
    assert!(dup.error.as_deref().unwrap().contains("a-first"));
    m.set_enabled("dup.id", true).unwrap();
    assert_eq!(
        m.plugin("dup.id")
            .unwrap()
            .call_json("x", &json!(null))
            .unwrap(),
        json!(1)
    );
}

#[test]
fn corrupt_state_file_disables_everything_and_is_reported() {
    let env = Env::new();
    env.add("one", "org.one", "transform", "", &echo());
    std::fs::create_dir_all(env.state().parent().unwrap()).unwrap();
    std::fs::write(env.state(), "{{{ not json").unwrap();
    let mut m = env.manager();
    m.scan();
    assert!(m.state_error().unwrap().contains("corrupt"));
    assert!(!m.is_enabled("org.one"));
    m.set_enabled("org.one", true).unwrap();
    assert!(m.state_error().is_none());
    let mut again = env.manager();
    again.scan();
    assert!(again.state_error().is_none());
    assert!(again.is_enabled("org.one"));
}

#[test]
fn unreadable_state_path_is_reported() {
    let env = Env::new();
    // a directory where the file should be
    std::fs::create_dir_all(env.state()).unwrap();
    let m = env.manager();
    assert!(m
        .state_error()
        .unwrap()
        .contains("cannot read plugin state"));
}

#[test]
fn state_without_enabled_key_is_empty() {
    let env = Env::new();
    std::fs::create_dir_all(env.state().parent().unwrap()).unwrap();
    std::fs::write(env.state(), "{}").unwrap();
    let m = env.manager();
    assert!(m.state_error().is_none());
}

#[test]
fn hot_reload_picks_up_changed_wasm() {
    let env = Env::new();
    env.add("one", "org.one", "transform", "", &always(&ok("1")));
    let mut m = env.manager();
    m.scan();
    m.set_enabled("org.one", true).unwrap();
    let first = m.plugin("org.one").unwrap();
    assert_eq!(first.call_json("x", &json!(null)).unwrap(), json!(1));

    // nothing changed: same cached plugin
    let r = m.reload_changed();
    assert!(r.added.is_empty() && r.changed.is_empty() && r.removed.is_empty());
    assert!(Arc::ptr_eq(&first, &m.plugin("org.one").unwrap()));

    // new bytes (different length so the stamp differs even on coarse clocks)
    std::fs::write(
        env.plugins().join("one/plugin.wasm"),
        always(&ok("\"changed\"")),
    )
    .unwrap();
    let r = m.reload_changed();
    assert_eq!(r.changed, vec!["org.one"]);
    assert!(!status(&m.list(), "org.one").loaded);
    let second = m.plugin("org.one").unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(
        second.call_json("x", &json!(null)).unwrap(),
        json!("changed")
    );
    // old handles keep working
    assert_eq!(first.call_json("x", &json!(null)).unwrap(), json!(1));
}

#[test]
fn hot_reload_picks_up_manifest_changes_added_and_removed() {
    let env = Env::new();
    env.add("one", "org.one", "transform", "", &echo());
    let mut m = env.manager();
    m.scan();
    m.set_enabled("org.one", true).unwrap();
    m.plugin("org.one").unwrap();

    std::fs::write(
        env.plugins().join("one/plugin.toml"),
        manifest_toml(
            "org.one",
            "transform",
            "fuel = 12345\ndescription = \"edited\"",
        ),
    )
    .unwrap();
    env.add("two", "org.two", "transform", "", &echo());
    let r = m.scan();
    assert_eq!(r.changed, vec!["org.one"]);
    assert_eq!(r.added, vec!["org.two"]);
    assert_eq!(m.plugin("org.one").unwrap().manifest().limits.fuel, 12345);
    assert_eq!(status(&m.list(), "org.one").description, "edited");

    std::fs::remove_dir_all(env.plugins().join("two")).unwrap();
    let r = m.scan();
    assert_eq!(r.removed, vec!["org.two"]);
    assert_eq!(m.list().len(), 1);

    // breaking the manifest turns the entry into an error entry under its folder name
    std::fs::write(env.plugins().join("one/plugin.toml"), "garbage = ").unwrap();
    let r = m.scan();
    assert_eq!(r.changed, vec!["one"]);
    assert!(status(&m.list(), "one").error.is_some());
    assert!(m.plugin("org.one").is_err());
}

#[test]
fn broken_wasm_after_reload_reports_an_error() {
    let env = Env::new();
    env.add("one", "org.one", "transform", "", &echo());
    let mut m = env.manager();
    m.scan();
    m.set_enabled("org.one", true).unwrap();
    std::fs::write(
        env.plugins().join("one/plugin.wasm"),
        b"corrupted wasm bytes!",
    )
    .unwrap();
    m.reload_changed();
    let e = m.plugin("org.one").unwrap_err().to_string();
    assert!(e.contains("invalid WebAssembly"), "{e}");
    assert!(status(&m.list(), "org.one").error.is_some());
    // the failure is cached until the files change; fix it and reload
    std::fs::write(env.plugins().join("one/plugin.wasm"), always(&ok("1"))).unwrap();
    m.reload_changed();
    assert!(m.plugin("org.one").is_ok());
    assert!(status(&m.list(), "org.one").error.is_none());
}

#[test]
fn register_helpers_only_take_enabled_plugins_of_the_right_kind() {
    let env = Env::new();
    let zip = base64_zip();
    env.add(
        "src",
        "p.src",
        "metadata-source",
        "",
        &canned(&[(b'l', ok(r#"[{"title":"Found"}]"#))]),
    );
    env.add(
        "rd",
        "p.reader",
        "format-reader",
        "extensions = [\"zzz\"]",
        &canned(&[(b'r', ok(&format!("\"{zip}\"")))]),
    );
    env.add(
        "wr",
        "p.writer",
        "format-writer",
        "extension = \"out\"",
        &canned(&[(b'w', ok("\"aGk=\""))]),
    );
    env.add(
        "tr",
        "p.transform",
        "transform",
        "",
        &canned(&[(b'a', ok(r#"{"files_changed":2}"#))]),
    );
    env.add("off", "p.off", "metadata-source", "", &echo());
    env.add("dev", "p.device", "device", "", &echo());
    let mut m = env.manager();
    m.scan();
    for id in ["p.src", "p.reader", "p.writer", "p.transform", "p.device"] {
        m.set_enabled(id, true).unwrap();
    }

    let mut sources: Registry<dyn MetadataSource> = Registry::new();
    assert!(m.register_metadata_sources(&mut sources).is_empty());
    assert_eq!(sources.len(), 1);
    let s = sources.iter().next().unwrap();
    assert_eq!(s.id(), "p.src");
    assert_eq!(s.lookup(&Default::default()).unwrap()[0].title, "Found");

    let mut readers: Registry<dyn FormatReader> = Registry::new();
    assert!(m.register_readers(&mut readers).is_empty());
    assert_eq!(readers.len(), 1);
    assert_eq!(readers.iter().next().unwrap().extensions(), &["zzz"]);
    assert!(readers.iter().next().unwrap().read(b"x").is_ok());

    let mut writers: Registry<dyn FormatWriter> = Registry::new();
    assert!(m.register_writers(&mut writers).is_empty());
    assert_eq!(
        writers
            .iter()
            .next()
            .unwrap()
            .write(&sample_epub("x"))
            .unwrap(),
        b"hi"
    );

    let mut transforms: Registry<dyn Transform> = Registry::new();
    assert!(m.register_transforms(&mut transforms).is_empty());
    let mut book = sample_epub("x");
    assert_eq!(
        transforms
            .iter()
            .next()
            .unwrap()
            .apply(&mut book)
            .unwrap()
            .files_changed,
        2
    );
}

#[test]
fn register_reports_plugins_that_fail_to_load() {
    let env = Env::new();
    env.add(
        "ok",
        "p.ok",
        "metadata-source",
        "",
        &canned(&[(b'l', ok("[]"))]),
    );
    env.add("broken", "p.broken", "metadata-source", "", &echo());
    let mut m = env.manager();
    m.scan();
    m.set_enabled("p.ok", true).unwrap();
    m.set_enabled("p.broken", true).unwrap();
    // wasm goes bad behind the manager's back after enabling
    std::fs::write(env.plugins().join("broken/plugin.wasm"), b"junk").unwrap();
    m.scan();
    let mut reg: Registry<dyn MetadataSource> = Registry::new();
    let errors = m.register_metadata_sources(&mut reg);
    assert_eq!(reg.len(), 1);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].0, "p.broken");
    assert!(errors[0].1.contains("invalid WebAssembly"));
}

#[test]
fn manager_uses_the_injected_network() {
    let env = Env::new();
    env.add(
        "net",
        "p.net",
        "transform",
        "permissions = [\"network:a.org\"]",
        &http_client(),
    );
    let net = MockNet::replying(br#"{"ok":"from mock"}"#);
    let mut m = PluginManager::new(
        env.plugins(),
        env.state(),
        Host::default().with_network(net.clone()),
    );
    m.scan();
    m.set_enabled("p.net", true).unwrap();
    let p = m.plugin("p.net").unwrap();
    let (raw, _) = p.call_raw("get", b"https://a.org/x").unwrap();
    assert_eq!(raw, br#"{"ok":"from mock"}"#);
    assert_eq!(net.urls(), vec!["https://a.org/x"]);
}

fn base64_zip() -> String {
    epub_b64("Reader out")
}
