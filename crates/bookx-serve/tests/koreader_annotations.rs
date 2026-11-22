//! The KOReader annotation sync over a real socket: the HTTP routes next to KOSync, and the
//! plugin's own Lua logic (`koreader-plugin/`) talking to the server through a mock KOReader
//! environment (the Lua part is skipped when `lua`/`luajit` or `curl` is not installed).
//! KOReader itself is never run here.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use bookx_serve::annotation_sync::{AnnotationSync, ORIGIN_BOOKX, Source, parse_annotation};
use bookx_serve::httpc::request;
use bookx_serve::kosync::{KoSync, MemoryStore, md5_hex};
use bookx_serve::server::{LibrarySnapshot, Server, ServerConfig, ServerHandle};
use serde_json::{Value, json};

const DOC: &str = "0123456789abcdef0123456789abcdef";

fn start(with_annotations: bool) -> (ServerHandle, Arc<AnnotationSync>) {
    let provider: bookx_serve::server::LibraryProvider = Arc::new(LibrarySnapshot::default);
    let mut cfg = ServerConfig::new(provider);
    let log = Arc::new(bookx_serve::activity::ActivityLog::memory());
    cfg.kosync = Some(Arc::new(
        KoSync::new(Box::new(MemoryStore::new()), true).with_activity(log.clone()),
    ));
    let ann = Arc::new(AnnotationSync::memory().with_activity(log));
    if with_annotations {
        cfg.annotations = Some(ann.clone());
    }
    (Server::start(cfg).unwrap(), ann)
}

struct Api {
    base: String,
    key: String,
}

impl Api {
    fn new(h: &ServerHandle) -> Api {
        let api = Api {
            base: h.base_url(),
            key: md5_hex("pw"),
        };
        let r = request(
            "POST",
            &format!("{}/bookx/users/create", api.base).parse().unwrap(),
            &[("Content-Type", "application/json")],
            json!({"username": "ann", "password": api.key})
                .to_string()
                .as_bytes(),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(r.status, 201);
        api
    }

    fn call(&self, method: &str, path: &str, key: &str, body: &[u8]) -> (u16, Value) {
        let r = request(
            method,
            &format!("{}/bookx{path}", self.base).parse().unwrap(),
            &[
                ("x-auth-user", "ann"),
                ("x-auth-key", key),
                ("Content-Type", "application/json"),
            ],
            body,
            Duration::from_secs(10),
        )
        .unwrap();
        (
            r.status,
            serde_json::from_slice(&r.body).unwrap_or(Value::Null),
        )
    }

    fn put(&self, list: Value) -> (u16, Value) {
        self.call(
            "PUT",
            &format!("/annotations/{DOC}"),
            &self.key,
            json!({"device": "Mock", "title": "Dune", "annotations": list})
                .to_string()
                .as_bytes(),
        )
    }
}

fn hl(id: &str, text: &str, updated: i64) -> Value {
    json!({"id": id, "kind": "highlight", "text": text, "updated": updated, "created": updated})
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[test]
fn put_and_get_over_http() {
    if std::env::var("NET").is_ok_and(|v| v == "0") {
        return;
    }
    let (h, ann) = start(true);
    let api = Api::new(&h);
    let t = now() - 5;
    let (s, v) = api.put(json!([hl("a", "Fear is the mind-killer.", t)]));
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["result"]["accepted"], 1);
    let (s, v) = api.call("GET", &format!("/annotations/{DOC}"), &api.key, b"");
    assert_eq!(s, 200);
    assert_eq!(v["annotations"][0]["text"], "Fear is the mind-killer.");
    assert_eq!(ann.changes_since(0).len(), 1);
    // a repeat is a no-op
    let (_, v) = api.put(json!([hl("a", "Fear is the mind-killer.", t)]));
    assert_eq!(v["result"]["unchanged"], 1);
}

#[test]
fn credentials_size_and_shape_are_enforced() {
    if std::env::var("NET").is_ok_and(|v| v == "0") {
        return;
    }
    let (h, ann) = start(true);
    let api = Api::new(&h);
    let body = json!({"annotations": [hl("a", "text text", now())]}).to_string();
    let path = format!("/annotations/{DOC}");
    assert_eq!(api.call("PUT", &path, "wrong", body.as_bytes()).0, 401);
    assert_eq!(api.call("PUT", &path, &api.key, b"{broken").0, 422);
    assert_eq!(api.call("PUT", &path, &api.key, b"[]").0, 422);
    let huge = vec![b' '; 1024 * 1024 + 10];
    assert_eq!(api.call("PUT", &path, &api.key, &huge).0, 413);
    assert_eq!(
        api.call(
            "PUT",
            "/annotations/not-a-digest",
            &api.key,
            body.as_bytes()
        )
        .0,
        400
    );
    assert_eq!(
        api.call(
            "PUT",
            "/annotations/..%2f..%2fetc",
            &api.key,
            body.as_bytes()
        )
        .0,
        400
    );
    assert_eq!(api.call("DELETE", &path, &api.key, b"").0, 405);
    assert_eq!(ann.seq(), 0, "nothing was stored by the refused requests");
}

#[test]
fn routes_are_absent_when_switched_off() {
    if std::env::var("NET").is_ok_and(|v| v == "0") {
        return;
    }
    let (h, _) = start(false);
    let api = Api::new(&h);
    let (s, _) = api.call("GET", &format!("/annotations/{DOC}"), &api.key, b"");
    assert_eq!(s, 404);
}

#[test]
fn bookx_side_records_reach_the_device_in_the_answer() {
    if std::env::var("NET").is_ok_and(|v| v == "0") {
        return;
    }
    let (h, ann) = start(true);
    let api = Api::new(&h);
    let src = Source {
        origin: ORIGIN_BOOKX,
        device: "Bookx",
        device_id: "",
        title: "",
        authors: "",
    };
    let a = parse_annotation(&hl("hl-1", "From the computer", now() - 2), now()).unwrap();
    ann.put("ann", DOC, &src, vec![a]).unwrap();
    let (_, v) = api.put(json!([]));
    assert_eq!(v["annotations"][0]["origin"], "bookx");
    assert_eq!(v["annotations"][0]["text"], "From the computer");
}

fn lua() -> Option<String> {
    ["luajit", "lua", "lua5.4", "lua5.3", "lua5.1"]
        .into_iter()
        .find(|l| Command::new(l).arg("-v").output().is_ok())
        .map(String::from)
}

#[test]
fn the_lua_plugin_logic_against_a_real_server() {
    if std::env::var("NET").is_ok_and(|v| v == "0") {
        return;
    }
    let Some(lua) = lua() else {
        eprintln!("lua not installed: plugin test skipped");
        return;
    };
    if Command::new("curl").arg("--version").output().is_err() {
        eprintln!("curl not installed: plugin test skipped");
        return;
    }
    let (h, ann) = start(true);
    let api = Api::new(&h);
    // a note made in Bookx for the book the mock device will sync
    let src = Source {
        origin: ORIGIN_BOOKX,
        device: "Bookx",
        device_id: "",
        title: "",
        authors: "",
    };
    let a = parse_annotation(&hl("hl-9", "A note from the computer", now() - 2), now()).unwrap();
    let doc = "b".repeat(32);
    ann.put("ann", &doc, &src, vec![a]).unwrap();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../koreader-plugin/tests");
    let out = Command::new(&lua)
        .arg("run.lua")
        .current_dir(&dir)
        .env("BOOKX_URL", format!("{}/bookx", api.base))
        .env("BOOKX_USER", "ann")
        .env("BOOKX_KEY", &api.key)
        .env("BOOKX_DOC", &doc)
        .env("BOOKX_EXPECT_INBOX", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // what the mock device sent is in the store: the kept highlight, the removed one as tombstone
    let recs = ann.records("ann", &doc, 0);
    let live: Vec<_> = recs
        .iter()
        .filter(|r| r.origin == "koreader" && !r.annotation.deleted)
        .collect();
    let gone: Vec<_> = recs
        .iter()
        .filter(|r| r.origin == "koreader" && r.annotation.deleted)
        .collect();
    assert_eq!(live.len(), 1);
    assert_eq!(gone.len(), 1);
    assert_eq!(live[0].annotation.text, "Live quote number one.");
    assert_eq!(live[0].device, "Mock Libra");
    assert_eq!(ann.documents()[0].2, "Live Book");
}

/// A sidecar of the real shape (every KOReader colour with every drawer) goes through the plugin
/// logic and the server: each of the 36 records holds exactly the colour and drawer KOReader wrote.
#[test]
fn a_real_shaped_sidecar_keeps_every_look_over_the_wire() {
    if std::env::var("NET").is_ok_and(|v| v == "0") {
        return;
    }
    let Some(lua) = lua() else {
        eprintln!("lua not installed: plugin test skipped");
        return;
    };
    if Command::new("curl").arg("--version").output().is_err() {
        eprintln!("curl not installed: plugin test skipped");
        return;
    }
    let (h, ann) = start(true);
    let api = Api::new(&h);
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../koreader-plugin/tests");
    let sidecar =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/koreader_all_looks.lua");
    let out = Command::new(&lua)
        .arg("replay_sidecar.lua")
        .arg(&sidecar)
        .current_dir(&dir)
        .env("BOOKX_URL", format!("{}/bookx", api.base))
        .env("BOOKX_USER", "ann")
        .env("BOOKX_KEY", &api.key)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let recs = ann.records("ann", "0123456789abcdef0123456789abcdef", 0);
    assert_eq!(recs.len(), 36);
    for r in recs {
        let t = &r.annotation.text; // "Quote number N in <colour> with <drawer>"
        let words: Vec<&str> = t.split(' ').collect();
        assert_eq!(r.annotation.color, words[4], "{t}");
        assert_eq!(r.annotation.drawer, words[6], "{t}");
    }
}

/// The REAL `main.lua` (menu, Register account, Test connection, Sync now) in a fake KOReader, talking over
/// localhost to the real Bookx sync server: the whole path the owner's Kobo takes, minus KOReader's own
/// widgets. Skipped with NET=0, or when lua or curl is missing.
#[test]
fn the_real_plugin_registers_and_syncs_against_a_real_server() {
    if std::env::var("NET").is_ok_and(|v| v == "0") {
        return;
    }
    let Some(lua) = lua() else {
        eprintln!("lua not installed: plugin test skipped");
        return;
    };
    if Command::new("curl").arg("--version").output().is_err() {
        eprintln!("curl not installed: plugin test skipped");
        return;
    }
    let (h, ann) = start(true);
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../koreader-plugin/tests");
    let out = Command::new(&lua)
        .arg("live.lua")
        .current_dir(&dir)
        .env("BOOKX_URL", format!("{}/bookx", h.base_url()))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let docs = ann.documents();
    assert_eq!(docs.len(), 1, "one book was synced");
    let recs = ann.records("zoe", &docs[0].1, 0);
    assert!(
        recs.len() >= 14,
        "the fixture entries arrived: {}",
        recs.len()
    );
    let odd = recs
        .iter()
        .find(|r| r.annotation.color == "teal")
        .expect("an unknown colour is kept as written");
    assert_eq!(odd.annotation.drawer, "wavy");
    assert!(
        !odd.annotation.text.contains('\u{1}') && !odd.annotation.text.contains('\r'),
        "control characters were removed before sending"
    );
    assert!(
        odd.annotation.text.contains('\u{FFFD}'),
        "invalid bytes were replaced"
    );
    assert_eq!(odd.device, "Live Kobo");
    // the server knows the device and what it did (what the Sync window shows)
    let log = ann.activity().expect("the test server records activity");
    let devices = log.devices();
    assert_eq!(devices.len(), 1, "{devices:?}");
    assert_eq!(devices[0].name, "Live Kobo");
    assert_eq!(devices[0].client, "KOReader 2026.07.2-test");
    assert!(!devices[0].plugin.is_empty(), "the plugin version is sent");
    assert_eq!(devices[0].protocol, 1);
    assert!(devices[0].last_sync.is_some());
    let events = log.events(50);
    assert!(
        events.iter().any(|e| matches!(
            e.what,
            bookx_serve::activity::What::Received { highlights, .. } if highlights >= 14
        )),
        "the sync is an event with its counts: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e.what, bookx_serve::activity::What::Rejected { .. })),
        "a wrong password is not logged as a device event (anyone on the network could fill the list)"
    );
    // the info route the plugin's version check uses
    let r = request(
        "GET",
        &format!("{}/bookx/info", h.base_url()).parse().unwrap(),
        &[],
        b"",
        Duration::from_secs(10),
    )
    .unwrap();
    assert_eq!(r.status, 200);
}

#[test]
fn a_plugin_style_put_over_localhost_notifies_the_listener_once_per_merge() {
    if std::env::var("NET").is_ok_and(|v| v == "0") {
        return;
    }
    use bookx_serve::notify::Note;
    let (h, ann) = start(true);
    let api = Api::new(&h);
    let got: Arc<std::sync::Mutex<Vec<Note>>> = Arc::default();
    let g = got.clone();
    ann.on_change(Some(Arc::new(move |n| g.lock().unwrap().push(n.clone()))));
    let t = now() - 5;
    // 8 devices' worth of racing requests: every one changes the store, each is heard once
    std::thread::scope(|sc| {
        for i in 0..8 {
            let api = &api;
            sc.spawn(move || {
                let (s, _) = api.put(json!([hl(&format!("h{i}"), "A line of text here.", t)]));
                assert_eq!(s, 200);
            });
        }
    });
    assert_eq!(got.lock().unwrap().len(), 8);
    // the same requests again change nothing: nobody is told
    let (_, v) = api.put(json!([hl("h0", "A line of text here.", t)]));
    assert_eq!(v["result"]["unchanged"], 1);
    assert_eq!(got.lock().unwrap().len(), 8);
    // a colour change is a change
    let mut recoloured = hl("h0", "A line of text here.", t + 10);
    recoloured["color"] = json!("purple");
    api.put(json!([recoloured]));
    let heard = got.lock().unwrap();
    assert_eq!(heard.len(), 9);
    assert!(matches!(heard.last(), Some(Note::Annotations { document, .. }) if document == DOC));
}

#[test]
fn a_reading_position_over_localhost_notifies_once() {
    if std::env::var("NET").is_ok_and(|v| v == "0") {
        return;
    }
    use bookx_serve::notify::Note;
    let provider: bookx_serve::server::LibraryProvider = Arc::new(LibrarySnapshot::default);
    let mut cfg = ServerConfig::new(provider);
    let ko = Arc::new(KoSync::new(Box::new(MemoryStore::new()), true));
    let got: Arc<std::sync::Mutex<Vec<Note>>> = Arc::default();
    let g = got.clone();
    ko.on_change(Some(Arc::new(move |n| g.lock().unwrap().push(n.clone()))));
    cfg.kosync = Some(ko);
    let h = Server::start(cfg).unwrap();
    let api = Api::new(&h);
    let (s, _) = api.call(
        "PUT",
        "/syncs/progress",
        &api.key,
        json!({"document": DOC, "progress": "/body/DocFragment[3]/body", "percentage": 0.62, "device": "Kobo Libra", "device_id": "k1"})
            .to_string()
            .as_bytes(),
    );
    assert_eq!(s, 200);
    let heard = got.lock().unwrap();
    assert_eq!(heard.len(), 1);
    assert!(
        matches!(&heard[0], Note::Progress { percentage, device, .. } if (*percentage - 0.62).abs() < 1e-9 && device == "Kobo Libra")
    );
}
