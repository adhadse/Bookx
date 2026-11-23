//! Hostile bytes for the wire formats `bookx-serve` accepts: the `/bookx` annotation and progress
//! JSON, the KOReader sidecar Lua reader, and request shapes the transport must refuse. Every case
//! is an error or a clean refusal, quickly, never a panic. Fuzz twin: `fuzz/fuzz_targets/wire.rs`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bookx_serve::annotation_sync::{AnnotationSync, MAX_BODY, parse_request};
use bookx_serve::http::Request;
use bookx_serve::kosync::{KoSync, MemoryStore, md5_hex};
use bookx_serve::lua;
use bookx_serve::server::{LibrarySnapshot, ServerConfig, handle};
use serde_json::json;

const NOW: i64 = 1_800_000_000;
const DOC: &str = "0123456789abcdef0123456789abcdef";

fn timed<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let r = f();
    assert!(
        t.elapsed() < Duration::from_secs(10),
        "{label} took {:?}",
        t.elapsed()
    );
    r
}

fn bodies() -> Vec<(&'static str, Vec<u8>)> {
    let deep = |n: usize| format!("{}1{}", "[".repeat(n), "]".repeat(n));
    let ann = |extra: &str| {
        format!(
            r#"{{"annotations":[{{"id":"a","kind":"highlight","text":"t","position":"p"{extra}}}]}}"#
        )
    };
    vec![
        ("empty", vec![]),
        ("null", b"null".to_vec()),
        ("array", b"[]".to_vec()),
        ("string", br#""x""#.to_vec()),
        ("garbage", vec![0xff, 0xfe, 0, 1, 2, 3]),
        ("truncated", br#"{"annotations":[{"id":"a","#.to_vec()),
        ("deep-array", deep(100_000).into_bytes()),
        (
            "deep-object",
            format!("{}1{}", r#"{"a":"#.repeat(50_000), "}".repeat(50_000)).into_bytes(),
        ),
        (
            "deep-in-annotations",
            format!(r#"{{"annotations":{}}}"#, deep(10_000)).into_bytes(),
        ),
        (
            "huge-list",
            format!(r#"{{"annotations":[{}]}}"#, vec!["{}"; 200_000].join(",")).into_bytes(),
        ),
        (
            "huge-text",
            ann(&format!(r#","note":"{}""#, "x".repeat(MAX_BODY))).into_bytes(),
        ),
        (
            "control-chars",
            ann(r#","note":"a\u0000b\u202e""#).into_bytes(),
        ),
        (
            "nan-numbers",
            ann(r#","time":1e999,"page":-1e999,"x":NaN"#).into_bytes(),
        ),
        (
            "huge-ints",
            ann(r#","time":99999999999999999999999999,"page":18446744073709551616"#).into_bytes(),
        ),
        (
            "wrong-types",
            br#"{"annotations":"x","origin":5,"device":[]}"#.to_vec(),
        ),
        (
            "duplicate-keys",
            br#"{"annotations":[],"annotations":[{"id":1}]}"#.to_vec(),
        ),
        ("bad-utf8", {
            let mut v = br#"{"annotations":[{"id":""#.to_vec();
            v.extend([0xc3, 0x28]);
            v.extend(br#""}]}"#);
            v
        }),
        (
            "unicode-escape-lone-surrogate",
            br#"{"annotations":[{"id":"\ud800"}]}"#.to_vec(),
        ),
        (
            "negative-and-future-times",
            ann(r#","time":-9223372036854775808,"updated":9223372036854775807"#).into_bytes(),
        ),
    ]
}

#[test]
fn hostile_annotation_bodies_are_refused_without_panicking() {
    for (label, b) in bodies() {
        let r = timed(label, || parse_request(&b, NOW));
        if let Ok(p) = &r {
            assert!(p.annotations.len() <= 1000, "{label}");
        }
    }
    // the transport-level body cap is separate: parse_request refuses bigger ones itself
    assert!(parse_request(&vec![b' '; MAX_BODY + 1], NOW).is_err());
}

fn server_cfg() -> ServerConfig {
    let mut cfg = ServerConfig::new(Arc::new(LibrarySnapshot::default));
    cfg.kosync = Some(Arc::new(KoSync::new(Box::new(MemoryStore::new()), true)));
    cfg.annotations = Some(Arc::new(AnnotationSync::memory()));
    cfg
}

#[test]
fn hostile_requests_to_the_sync_routes_get_a_status_never_a_panic() {
    let cfg = server_cfg();
    let create = Request::new("POST", "/bookx/users/create")
        .with_body(br#"{"username":"alice","password":"pw"}"#);
    assert_eq!(handle(&cfg, &create).status, 201);
    let key = md5_hex("pw");
    let authed = |m: &str, p: &str| {
        Request::new(m, p)
            .with_header("x-auth-user", "alice")
            .with_header("x-auth-key", &key)
    };
    for (label, b) in bodies() {
        for (m, p) in [
            ("PUT", format!("/bookx/annotations/{DOC}")),
            ("PUT", "/bookx/syncs/progress".to_string()),
            ("POST", "/bookx/users/create".to_string()),
        ] {
            let r = timed(label, || handle(&cfg, &authed(m, &p).with_body(&b)));
            assert!(
                r.status < 600 && r.status >= 200,
                "{label} {m} {p}: {}",
                r.status
            );
            assert_ne!(r.status, 500, "{label} {m} {p}");
        }
    }
    // paths: traversal, encoded separators, NUL, enormous segments, wrong ids
    for p in [
        "/bookx/annotations/..%2f..%2fetc%2fpasswd",
        "/bookx/annotations/%2e%2e",
        "/bookx/annotations/%00",
        "/bookx/annotations/zzzz",
        &format!("/bookx/annotations/{}", "a".repeat(100_000)),
        "/bookx/annotations/",
        "/bookx/annotations",
        "/bookx/annotations/a/b/c",
        "/bookx//annotations//x",
        "/bookx/syncs/progress/%ff",
        "/bookx/users/auth/../create",
        "/bookx/%2e%2e/etc",
        "/%62ookx/info",
    ] {
        for m in ["GET", "PUT", "DELETE", "PATCH", "TRACE"] {
            let r = timed(p, || handle(&cfg, &authed(m, p)));
            assert_ne!(r.status, 500, "{m} {p}");
            assert_ne!(r.status, 200, "{m} {p} must not succeed");
        }
    }
    // wrong or missing credentials never reach a route
    let r = handle(
        &cfg,
        &Request::new("PUT", &format!("/bookx/annotations/{DOC}")).with_body(b"{}"),
    );
    assert_eq!(r.status, 401);
    let r = handle(
        &cfg,
        &Request::new("PUT", &format!("/bookx/annotations/{DOC}"))
            .with_header("x-auth-user", "alice")
            .with_header("x-auth-key", "nope")
            .with_body(b"{}"),
    );
    assert_eq!(r.status, 401);
}

#[test]
fn a_user_name_cannot_be_used_to_attack_the_store() {
    let cfg = server_cfg();
    for name in [
        "../../etc/passwd",
        "a\0b",
        "a b",
        "",
        &"n".repeat(100_000),
        "\u{202e}root",
        "x'; DROP TABLE users;--",
        "{{7*7}}",
    ] {
        let body = json!({"username": name, "password": "pw"}).to_string();
        let r = handle(
            &cfg,
            &Request::new("POST", "/bookx/users/create").with_body(body.as_bytes()),
        );
        assert_ne!(r.status, 500, "{name:?}");
        assert!(
            r.status == 201 || r.status == 400 || r.status == 402,
            "{name:?} {}",
            r.status
        );
    }
}

#[test]
fn koreader_sidecar_lua_is_parsed_as_data_with_limits() {
    let deep_tables = format!("return {}1{}", "{".repeat(100_000), "}".repeat(100_000));
    let deep_keys = format!("return {}", r#"{["a"]="#.repeat(50_000));
    let cases: Vec<(&str, String)> = vec![
        ("empty", String::new()),
        ("return-nothing", "return".into()),
        ("deep-tables", deep_tables),
        ("deep-keys", deep_keys),
        ("unterminated-string", "return { [\"a\"] = \"abc".repeat(100)),
        ("unterminated-long-bracket", "return { a = [[".to_string() + &"x".repeat(1 << 20)),
        ("comment-flood", "--[[".repeat(100_000)),
        ("huge-table", format!("return {{{}}}", "1,".repeat(1_000_000))),
        ("many-keys", format!("return {{{}}}", (0..200_000).map(|i| format!("[\"k{i}\"]=1,")).collect::<String>())),
        ("code-is-not-run", "return { a = os.execute('touch /tmp/bookx-pwned'), b = (function() end)(), c = io.open('/etc/passwd') }".into()),
        ("numbers", "return { 1e999, -1e999, 0x7fffffffffffffff, 0xffffffffffffffffff, 1e-999, .5, 5., 0x.1p4 }".into()),
        ("escapes", r#"return { "\x", "\999", "\u{110000}", "\z   ", "\
", "\xZZ" }"#.into()),
        ("nul-and-bom", "\u{feff}return { \"a\0b\" }\0".into()),
        ("keywords", "return { nil, true, false, and, end, function }".into()),
        ("garbage", String::from_utf8_lossy(&[0xff, 0xfe, b'{', b'}', 0, 1]).into_owned()),
    ];
    for (label, text) in &cases {
        let r = timed(label, || lua::parse(text));
        if let Some(v) = r {
            let _ = v.get("a");
            let _ = v.items().count();
        }
    }
    assert!(!std::path::Path::new("/tmp/bookx-pwned").exists());
    // the table that is allowed still parses
    let ok = lua::parse(r#"return { ["highlight"] = { [1] = { ["text"] = "hi", ["pos0"] = "a" } }, ["percent_finished"] = 0.5 }"#).unwrap();
    assert_eq!(ok.num("percent_finished"), Some(0.5));
}

#[test]
fn request_heads_with_hostile_hosts_never_reach_the_library() {
    let cfg = server_cfg();
    for host in [
        "evil.example",
        "127.0.0.1.evil.example",
        "localhost@evil.example",
        "[::1",
        "a b",
        &"h".repeat(70_000),
        "\u{0}",
    ] {
        let r = handle(
            &cfg,
            &Request::new("GET", "/opds").with_header("Host", host),
        );
        assert_eq!(r.status, 403, "{host:?}");
    }
}
