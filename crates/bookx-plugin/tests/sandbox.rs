//! Runtime behaviour: ABI, limits, imports, malformed plugins.

mod common;

use std::sync::Arc;

use bookx_plugin::runtime::{url_host, MAX_HTTP_CALLS, MAX_LOG_BYTES, MAX_LOG_LINES};
use bookx_plugin::{Host, Manifest, Plugin};
use common::*;
use serde_json::{json, Value};

fn err_of<T: std::fmt::Debug>(r: anyhow::Result<T>) -> String {
    format!("{:#}", r.unwrap_err())
}

fn load_err(wasm: &[u8]) -> String {
    err_of(Plugin::from_bytes(
        manifest("t.p", "transform", ""),
        wasm,
        Host::default(),
    ))
}

// ---------- happy path and ABI ----------

#[test]
fn canned_reply_roundtrips_through_the_envelope() {
    let p = plugin("transform", "", &always(&ok(r#"{"a":[1,2,3]}"#)));
    assert_eq!(
        p.call_json("anything", &json!({})).unwrap(),
        json!({"a":[1,2,3]})
    );
}

#[test]
fn echo_returns_the_input_json() {
    let p = plugin("transform", "", &echo());
    let input = json!({"k": "v", "n": [1, 2.5, null, true]});
    assert_eq!(p.call_json("echo", &input).unwrap(), input);
}

#[test]
fn operation_name_reaches_the_guest() {
    let p = plugin(
        "transform",
        "",
        &canned(&[(b'a', ok("\"alpha\"")), (b'b', ok("\"beta\""))]),
    );
    assert_eq!(p.call_json("apple", &json!(null)).unwrap(), json!("alpha"));
    assert_eq!(p.call_json("banana", &json!(null)).unwrap(), json!("beta"));
}

#[test]
fn unsupported_operation_is_reported() {
    let p = plugin("transform", "", &canned(&[(b'a', ok("1"))]));
    let e = err_of(p.call_json("zzz", &json!(null)));
    assert!(e.contains("does not support this operation"), "{e}");
}

#[test]
fn unicode_and_escapes_survive() {
    let text = "Ünïcödé 日本語 🦀 \\\" \\n";
    let p = plugin("transform", "", &echo());
    let v = p.call_json("e", &json!({ "t": text })).unwrap();
    assert_eq!(v["t"], text);
    let canned_reply = ok(r#"{"title":"Ça va? \u00e9 \ud83e\udd80 日本"}"#);
    let p = plugin("transform", "", &always(&canned_reply));
    assert_eq!(
        p.call_json("x", &json!(null)).unwrap()["title"],
        "Ça va? é 🦀 日本"
    );
}

#[test]
fn empty_input_and_big_input() {
    let p = plugin("transform", "", &echo());
    assert_eq!(p.call_json("e", &json!(null)).unwrap(), Value::Null);
    let big = "x".repeat(3_000_000);
    let p = plugin("transform", "memory_pages = 1024", &echo());
    let v = p.call_json("e", &json!({ "big": big })).unwrap();
    assert_eq!(v["big"].as_str().unwrap().len(), 3_000_000);
}

#[test]
fn huge_reply_is_returned_up_to_the_cap() {
    let big = ok(&format!("\"{}\"", "y".repeat(2_000_000)));
    let wasm = always(&big);
    let p = plugin("transform", "memory_pages = 1024", &wasm);
    let (raw, stats) = p.call_raw("x", b"").unwrap();
    assert_eq!(raw.len(), big.len());
    assert!(stats.memory_bytes >= big.len());

    let capped = Plugin::from_bytes(
        manifest("t.p", "transform", "memory_pages = 1024"),
        &wasm,
        Host::default(),
    )
    .unwrap()
    .with_max_reply(1_000_000);
    let e = err_of(capped.call_raw("x", b""));
    assert!(e.contains("reply of 2000"), "{e}");
    assert!(e.contains("limit 1000000"), "{e}");
}

#[test]
fn stats_report_fuel_and_memory() {
    let p = plugin("transform", "", &counting_loop(1000));
    let (_, small) = p.call_json_stats("x", &json!(null)).unwrap();
    let p = plugin("transform", "", &counting_loop(100_000));
    let (_, big) = p.call_json_stats("x", &json!(null)).unwrap();
    assert!(small.fuel_used > 1000, "{small:?}");
    assert!(big.fuel_used > small.fuel_used * 50, "{small:?} {big:?}");
    assert!(big.memory_bytes >= 65536);
    assert_eq!(big.http_calls, 0);
}

#[test]
fn info_export_is_optional() {
    let p = plugin("transform", "", &always(&ok("1")));
    assert_eq!(p.info().unwrap(), None);

    let info = br#"{"name":"demo","ops":["lookup"]}"#;
    let src = format!(
        r#"(module
  (memory (export "memory") 1)
  (data (i32.const 1024) "{}")
  (func (export "alloc") (param i32) (result i32) (i32.const 4096))
  (func (export "dealloc") (param i32 i32))
  (func (export "bookx_call") (param i32 i32 i32 i32) (result i64) (i64.const -2))
  (func (export "bookx_info") (result i64) (i64.const {})))"#,
        esc(info),
        (1024i64 << 32) | info.len() as i64
    );
    let p = plugin("transform", "", &wat(&src));
    assert_eq!(p.info().unwrap().unwrap()["name"], "demo");
}

#[test]
fn info_failures_are_errors() {
    let mk = |ret: &str, data: &[u8]| {
        let src = format!(
            r#"(module
  (memory (export "memory") 1)
  (data (i32.const 1024) "{}")
  (func (export "alloc") (param i32) (result i32) (i32.const 4096))
  (func (export "dealloc") (param i32 i32))
  (func (export "bookx_call") (param i32 i32 i32 i32) (result i64) (i64.const -2))
  (func (export "bookx_info") (result i64) {ret}))"#,
            esc(data)
        );
        plugin("transform", "", &wat(&src))
    };
    let e = err_of(mk("(i64.const -3)", b"").info());
    assert!(e.contains("rejected the input"), "{e}");
    let e = err_of(mk(&format!("(i64.const {})", (1024i64 << 32) | 3), b"{x}").info());
    assert!(e.contains("malformed info JSON"), "{e}");
    let e = err_of(mk(&format!("(i64.const {})", (0x7FFF0000i64 << 32) | 3), b"").info());
    assert!(e.contains("invalid info buffer"), "{e}");
    let e = err_of(mk("unreachable", b"").info());
    assert!(e.contains("trapped"), "{e}");
}

#[test]
fn error_envelope_becomes_an_error() {
    let p = plugin("transform", "", &always(br#"{"err":"nope, sorry"}"#));
    let e = err_of(p.call_json("x", &json!(null)));
    assert!(e.contains("nope, sorry"), "{e}");
    assert!(e.contains("test.plugin"), "{e}");
    let p = plugin("transform", "", &always(br#"{"err":{"code":5}}"#));
    assert!(err_of(p.call_json("x", &json!(null))).contains("\"code\":5"));
}

#[test]
fn error_codes_are_described() {
    for (code, msg) in [
        (-1, "reported a failure"),
        (-2, "does not support"),
        (-3, "rejected the input"),
        (-4, "out of memory"),
        (-77, "code -77"),
        (i64::MIN, "code"),
    ] {
        let p = plugin("transform", "", &returns_code(code));
        let e = err_of(p.call_raw("x", b""));
        assert!(e.contains(msg), "{code}: {e}");
    }
}

// ---------- malformed replies ----------

#[test]
fn garbage_replies_are_protocol_errors() {
    for (reply, msg) in [
        (&b"this is not json"[..], "malformed JSON"),
        (b"", "malformed JSON"),
        (b"{\"ok\":", "malformed JSON"),
        (b"\xff\xfe\xfd", "malformed JSON"),
        (b"[1,2,3]", "not a JSON object"),
        (b"\"str\"", "not a JSON object"),
        (b"{}", "neither `ok` nor `err`"),
        (b"{\"other\":1}", "neither `ok` nor `err`"),
    ] {
        let p = plugin("transform", "", &always(reply));
        let e = err_of(p.call_json("x", &json!(null)));
        assert!(e.contains(msg), "{:?}: {e}", String::from_utf8_lossy(reply));
    }
}

#[test]
fn bad_return_pointers_are_rejected() {
    // beyond memory
    let p = plugin("transform", "", &raw_return(0x7000_0000, 10));
    assert!(err_of(p.call_raw("x", b"")).contains("outside its memory"));
    // exactly at the end
    let p = plugin("transform", "", &raw_return(131072 - 4, 100));
    assert!(err_of(p.call_raw("x", b"")).contains("outside its memory"));
    // length beyond the reply cap
    let p = plugin("transform", "", &raw_return(0x7FFF_FFFF, 0xFFFF_FFFF));
    assert!(err_of(p.call_raw("x", b"")).contains("reply of 4294967295 bytes"));
    // all ones is simply the negative error code -1
    let p = plugin("transform", "", &raw_return(0xFFFF_FFFF, 0xFFFF_FFFF));
    assert!(err_of(p.call_raw("x", b"")).contains("reported a failure"));
    // in range, zero length: empty reply, then JSON error
    let p = plugin("transform", "", &raw_return(100, 0));
    let e = err_of(p.call_json("x", &json!(null)));
    assert!(e.contains("malformed JSON"), "{e}");
}

#[test]
fn bad_alloc_pointers_are_rejected() {
    for ptr in [-1, 0x7fff_ff00, 65536 - 1] {
        let p = plugin("transform", "", &bad_alloc(ptr));
        let e = err_of(p.call_raw("op", b"data"));
        assert!(
            e.contains("invalid buffer") || e.contains("out-of-bounds"),
            "{ptr}: {e}"
        );
    }
}

// ---------- fuel, time, memory, traps ----------

#[test]
fn endless_loop_runs_out_of_fuel() {
    let p = plugin("transform", "fuel = 1000000", &infinite_loop());
    let t = std::time::Instant::now();
    let e = err_of(p.call_raw("x", b""));
    assert!(e.contains("budget"), "{e}");
    assert!(t.elapsed().as_secs() < 5);
}

#[test]
fn fuel_limit_is_per_call_and_exact_enough() {
    let wasm = counting_loop(50_000);
    let fails = plugin("transform", "fuel = 20000", &wasm);
    assert!(err_of(fails.call_raw("x", b"")).contains("budget"));
    let fine = plugin("transform", "fuel = 5000000", &wasm);
    for _ in 0..3 {
        fine.call_raw("x", b"").unwrap();
    }
}

#[test]
fn timeout_is_enforced_through_fuel() {
    // 1 ms = 50_000 fuel; the loop needs far more than that.
    let wasm = counting_loop(5_000_000);
    let p = plugin("transform", "timeout_ms = 1", &wasm);
    assert!(err_of(p.call_raw("x", b"")).contains("budget"));
    let p = plugin("transform", "timeout_ms = 60000\nfuel = 10000000000", &wasm);
    p.call_raw("x", b"").unwrap();
}

#[test]
fn start_function_is_metered_too() {
    let p = plugin("transform", "fuel = 100000", &start_loops());
    let e = err_of(p.call_raw("x", b""));
    assert!(e.contains("budget") && e.contains("start-up"), "{e}");
    let p = plugin("transform", "", &start_traps());
    let e = err_of(p.call_raw("x", b""));
    assert!(e.contains("trapped") && e.contains("start-up"), "{e}");
}

#[test]
fn growing_past_the_memory_cap_fails_inside_the_guest() {
    // module starts with 2 pages; cap 16 pages
    let p = plugin("transform", "memory_pages = 16", &grower(100));
    assert!(err_of(p.call_raw("x", b"")).contains("out of memory"));
    // within the cap it works
    let p = plugin("transform", "memory_pages = 16", &grower(10));
    p.call_json("x", &json!(null)).unwrap();
    // a huge request never succeeds even with the maximum cap
    let p = plugin("transform", "memory_pages = 4096", &grower(1_000_000));
    assert!(err_of(p.call_raw("x", b"")).contains("out of memory"));
}

#[test]
fn module_declaring_more_memory_than_allowed_does_not_start() {
    let p = plugin("transform", "memory_pages = 1", &always(&ok("1")));
    // module() declares >= 2 pages
    let e = err_of(p.call_raw("x", b""));
    assert!(
        e.to_lowercase().contains("memory") || e.contains("start-up"),
        "{e}"
    );
}

#[test]
fn input_too_big_for_the_memory_cap_is_an_error_not_a_panic() {
    let p = plugin("transform", "memory_pages = 4", &echo());
    let e = err_of(p.call_raw("e", &vec![b'a'; 1_000_000]));
    assert!(e.contains("invalid buffer") || e.contains("trapped"), "{e}");
}

#[test]
fn traps_become_errors_with_the_plugin_id() {
    for wasm in [trapper(), div_by_zero()] {
        let p = plugin("transform", "", &wasm);
        let e = err_of(p.call_raw("x", b""));
        assert!(e.contains("`test.plugin` trapped"), "{e}");
    }
}

#[test]
fn runaway_recursion_traps_instead_of_crashing() {
    let p = plugin("transform", "", &recursion());
    let e = err_of(p.call_raw("x", b""));
    assert!(e.contains("trapped") || e.contains("budget"), "{e}");
}

#[test]
fn a_trap_does_not_poison_the_next_call() {
    // the same Plugin object is used after failures
    let p = plugin("transform", "fuel = 100000", &infinite_loop());
    for _ in 0..3 {
        assert!(p.call_raw("x", b"").is_err());
    }
    let p = plugin("transform", "", &trapper());
    for _ in 0..3 {
        assert!(p.call_raw("x", b"").is_err());
    }
}

// ---------- no state between calls ----------

#[test]
fn every_call_gets_a_fresh_instance() {
    let p = plugin("transform", "", &stateful());
    for _ in 0..50 {
        assert_eq!(p.call_json("x", &json!(null)).unwrap(), json!(1));
    }
}

// ---------- load-time validation ----------

#[test]
fn not_wasm_is_rejected() {
    assert!(load_err(b"hello, definitely not wasm").contains("invalid WebAssembly module"));
    assert!(load_err(b"").contains("invalid WebAssembly module"));
    assert!(load_err(b"\0asm\x01\0\0\0\xff\xff").contains("invalid WebAssembly module"));
}

#[test]
fn missing_exports_are_named() {
    let no_memory = wat(r#"(module
      (func (export "alloc") (param i32) (result i32) (i32.const 0))
      (func (export "dealloc") (param i32 i32))
      (func (export "bookx_call") (param i32 i32 i32 i32) (result i64) (i64.const 0)))"#);
    assert!(load_err(&no_memory).contains("export its linear memory"));
    for missing in ["alloc", "dealloc", "bookx_call"] {
        let mut src = String::from("(module (memory (export \"memory\") 1)\n");
        for (n, f) in [
            (
                "alloc",
                "(func (export \"alloc\") (param i32) (result i32) (i32.const 0))",
            ),
            ("dealloc", "(func (export \"dealloc\") (param i32 i32))"),
            (
                "bookx_call",
                "(func (export \"bookx_call\") (param i32 i32 i32 i32) (result i64) (i64.const 0))",
            ),
        ] {
            if n != missing {
                src.push_str(f);
            }
        }
        src.push(')');
        let e = load_err(&wat(&src));
        assert!(
            e.contains(&format!("must export `{missing}`")),
            "{missing}: {e}"
        );
    }
}

#[test]
fn wrong_signatures_are_rejected() {
    let bad_call = wat(r#"(module (memory (export "memory") 1)
      (func (export "alloc") (param i32) (result i32) (i32.const 0))
      (func (export "dealloc") (param i32 i32))
      (func (export "bookx_call") (param i32) (result i32) (i32.const 0)))"#);
    assert!(load_err(&bad_call).contains("`bookx_call` has the wrong signature"));
    let bad_alloc = wat(r#"(module (memory (export "memory") 1)
      (func (export "alloc") (param i64) (result i32) (i32.const 0))
      (func (export "dealloc") (param i32 i32))
      (func (export "bookx_call") (param i32 i32 i32 i32) (result i64) (i64.const 0)))"#);
    assert!(load_err(&bad_alloc).contains("`alloc` has the wrong signature"));
    let bad_dealloc = wat(r#"(module (memory (export "memory") 1)
      (func (export "alloc") (param i32) (result i32) (i32.const 0))
      (func (export "dealloc") (param i32) (result i32) (i32.const 0))
      (func (export "bookx_call") (param i32 i32 i32 i32) (result i64) (i64.const 0)))"#);
    assert!(load_err(&bad_dealloc).contains("`dealloc` has the wrong signature"));
    let bad_info = wat(&format!(
        "(module {MINIMAL_EXPORTS} (func (export \"bookx_info\") (result i32) (i32.const 0)))"
    ));
    assert!(load_err(&bad_info).contains("`bookx_info` has the wrong signature"));
}

#[test]
fn exports_of_the_wrong_kind_are_rejected() {
    let mem_fn = wat(
        r#"(module (memory 1) (global (export "memory") i32 (i32.const 0))
      (func (export "alloc") (param i32) (result i32) (i32.const 0))
      (func (export "dealloc") (param i32 i32))
      (func (export "bookx_call") (param i32 i32 i32 i32) (result i64) (i64.const 0)))"#,
    );
    assert!(load_err(&mem_fn).contains("`memory` is not a linear memory"));
    let global_call = wat(r#"(module (memory (export "memory") 1)
      (func (export "alloc") (param i32) (result i32) (i32.const 0))
      (func (export "dealloc") (param i32 i32))
      (global (export "bookx_call") i32 (i32.const 0)))"#);
    assert!(load_err(&global_call).contains("`bookx_call` is not a function"));
}

#[test]
fn imports_are_denied_by_default() {
    let wasi = wat(&format!(
        r#"(module
          (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))
          {MINIMAL_EXPORTS})"#
    ));
    let e = load_err(&wasi);
    assert!(
        e.contains("wasi_snapshot_preview1::fd_write") && e.contains("not allowed"),
        "{e}"
    );
    let env = wat(&format!(
        r#"(module (import "env" "abort" (func (param i32))) {MINIMAL_EXPORTS})"#
    ));
    assert!(load_err(&env).contains("env::abort"));
    let unknown = wat(&format!(
        r#"(module (import "bookx_host" "read_file" (func (param i32) (result i32))) {MINIMAL_EXPORTS})"#
    ));
    let e = load_err(&unknown);
    assert!(
        e.contains("bookx_host::read_file") && e.contains("unknown host function"),
        "{e}"
    );
    let wrong_sig = wat(&format!(
        r#"(module (import "bookx_host" "now_ms" (func (result i32))) {MINIMAL_EXPORTS})"#
    ));
    assert!(load_err(&wrong_sig).contains("wrong signature"));
    let not_func = wat(&format!(
        r#"(module (import "bookx_host" "log" (memory 1)) {MINIMAL_EXPORTS})"#
    ));
    assert!(load_err(&not_func).contains("not a function"));
}

#[test]
fn plugin_load_reads_the_manifest_wasm() {
    let dir = TempDir::new("load");
    install(
        dir.path(),
        "p",
        &manifest_toml("a.b", "transform", ""),
        &always(&ok("5")),
    );
    let m = Manifest::load(&dir.path().join("p")).unwrap();
    let p = Plugin::load(&dir.path().join("p"), m, Host::default()).unwrap();
    assert_eq!(p.id(), "a.b");
    assert_eq!(p.manifest().name, "Test a.b");
    assert_eq!(p.call_json("x", &json!(null)).unwrap(), json!(5));
    // wasm deleted after the manifest was read
    std::fs::remove_file(dir.path().join("p/plugin.wasm")).unwrap();
    let m = manifest("a.b", "transform", "");
    let e = err_of(Plugin::load(&dir.path().join("p"), m, Host::default()));
    assert!(e.contains("reading"), "{e}");
}

// ---------- host imports ----------

#[test]
fn http_allowed_host_returns_the_body() {
    let net = MockNet::replying(br#"{"ok":{"pong":true}}"#);
    let host = Host::default().with_network(net.clone());
    let p = plugin_host(
        "metadata-source",
        "permissions = [\"network:api.good.org\"]",
        &http_client(),
        host,
    );
    let (raw, stats) = p
        .call_raw("get", b"https://API.good.org/v1/search?q=a%20b#frag")
        .unwrap();
    assert_eq!(raw, br#"{"ok":{"pong":true}}"#);
    assert_eq!(stats.http_calls, 1);
    assert_eq!(
        net.urls(),
        vec!["https://API.good.org/v1/search?q=a%20b#frag"]
    );
    // port and http scheme
    p.call_raw("get", b"http://api.good.org:8080/x").unwrap();
    assert_eq!(net.urls().len(), 2);
}

#[test]
fn http_forbidden_hosts_never_reach_the_network() {
    let net = MockNet::replying(b"{}");
    let host = Host::default().with_network(net.clone());
    let p = plugin_host(
        "metadata-source",
        "permissions = [\"network:api.good.org\"]",
        &http_client(),
        host,
    );
    for url in [
        "https://evil.org/",
        "https://good.org/",
        "https://sub.api.good.org/",
        "https://api.good.org.evil.org/",
        "https://user:pw@api.good.org/",
        "https://api.good.org@evil.org/",
    ] {
        let e = err_of(p.call_raw("get", url.as_bytes()));
        assert!(
            e.contains("code -102") || e.contains("code -101"),
            "{url}: {e}"
        );
    }
    assert!(net.urls().is_empty(), "{:?}", net.urls());
}

#[test]
fn http_bad_urls_are_rejected() {
    let net = MockNet::replying(b"{}");
    let host = Host::default().with_network(net.clone());
    let p = plugin_host(
        "metadata-source",
        "permissions = [\"network:api.good.org\"]",
        &http_client(),
        host,
    );
    for url in [
        &b"ftp://api.good.org/"[..],
        b"file:///etc/passwd",
        b"api.good.org/x",
        b"https://",
        b"https:///x",
        b"https://api.good.org:/x",
        b"https://api.good.org:80a/x",
        b"https://[::1]/",
        b"\xff\xfe",
        b"",
    ] {
        let e = err_of(p.call_raw("get", url));
        assert!(
            e.contains("code -101"),
            "{:?}: {e}",
            String::from_utf8_lossy(url)
        );
    }
    assert!(net.urls().is_empty());
}

#[test]
fn http_without_permissions_is_denied() {
    let net = MockNet::replying(b"{}");
    let host = Host::default().with_network(net.clone());
    let p = plugin_host("metadata-source", "", &http_client(), host);
    let e = err_of(p.call_raw("get", b"https://api.good.org/"));
    assert!(e.contains("code -102"), "{e}");
    assert!(net.urls().is_empty());
}

#[test]
fn default_host_has_no_network() {
    let p = plugin(
        "metadata-source",
        "permissions = [\"network:api.good.org\"]",
        &http_client(),
    );
    let e = err_of(p.call_raw("get", b"https://api.good.org/"));
    assert!(e.contains("code -103"), "{e}");
}

#[test]
fn network_failure_and_oversize_bodies() {
    let host = Host::default().with_network(MockNet::failing());
    let p = plugin_host(
        "metadata-source",
        "permissions = [\"network:a.org\"]",
        &http_client(),
        host,
    );
    assert!(err_of(p.call_raw("get", b"https://a.org/")).contains("code -103"));

    let body = vec![b'z'; 8 * 1024 * 1024 + 1];
    let host = Host::default().with_network(MockNet::replying(&body));
    let p = plugin_host(
        "metadata-source",
        "permissions = [\"network:a.org\"]",
        &http_client(),
        host,
    );
    assert!(err_of(p.call_raw("get", b"https://a.org/")).contains("code -104"));
}

#[test]
fn http_response_larger_than_guest_memory_reports_alloc_failure() {
    let body = vec![b'z'; 4 * 1024 * 1024];
    let host = Host::default().with_network(MockNet::replying(&body));
    let p = plugin_host(
        "metadata-source",
        "permissions = [\"network:a.org\"]\nmemory_pages = 8",
        &http_client(),
        host,
    );
    assert!(err_of(p.call_raw("get", b"https://a.org/")).contains("code -105"));
}

#[test]
fn http_calls_per_invocation_are_capped() {
    let net = MockNet::replying(b"{}");
    let host = Host::default().with_network(net.clone());
    let manifest_extra = "permissions = [\"network:a.org\"]";
    let p = plugin_host(
        "metadata-source",
        manifest_extra,
        &http_repeat(MAX_HTTP_CALLS),
        host.clone(),
    );
    p.call_raw("x", b"https://a.org/").unwrap();
    assert_eq!(net.urls().len(), MAX_HTTP_CALLS as usize);
    let p = plugin_host(
        "metadata-source",
        manifest_extra,
        &http_repeat(MAX_HTTP_CALLS + 1),
        host,
    );
    assert!(err_of(p.call_raw("x", b"https://a.org/")).contains("code -106"));
    assert_eq!(net.urls().len(), 2 * MAX_HTTP_CALLS as usize);
}

#[test]
fn logging_reaches_the_host_log() {
    let log = Arc::new(VecLog::default());
    let host = Host::default().with_log(log.clone());
    let p = plugin_host(
        "transform",
        "",
        &logger(3, "héllo wörld".as_bytes(), 2),
        host,
    );
    p.call_raw("x", b"").unwrap();
    let lines = log.0.lock().unwrap().clone();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], ("test.plugin".into(), 3, "héllo wörld".into()));
}

#[test]
fn log_levels_are_clamped_and_volume_is_capped() {
    let log = Arc::new(VecLog::default());
    let host = Host::default().with_log(log.clone());
    let p = plugin_host("transform", "", &logger(99, b"x", 1), host.clone());
    p.call_raw("x", b"").unwrap();
    let p = plugin_host("transform", "", &logger(-5, b"x", 1), host.clone());
    p.call_raw("x", b"").unwrap();
    {
        let lines = log.0.lock().unwrap();
        assert_eq!(lines[0].1, 4);
        assert_eq!(lines[1].1, 0);
    }
    log.0.lock().unwrap().clear();
    let p = plugin_host(
        "transform",
        "",
        &logger(2, b"spam", MAX_LOG_LINES + 500),
        host.clone(),
    );
    p.call_raw("x", b"").unwrap();
    assert_eq!(log.0.lock().unwrap().len(), MAX_LOG_LINES as usize);
    log.0.lock().unwrap().clear();
    let long = vec![b'q'; MAX_LOG_BYTES * 2];
    let p = plugin_host("transform", "", &logger(2, &long, 1), host);
    p.call_raw("x", b"").unwrap();
    assert_eq!(log.0.lock().unwrap()[0].2.len(), MAX_LOG_BYTES);
}

#[test]
fn invalid_utf8_in_logs_is_replaced() {
    let log = Arc::new(VecLog::default());
    let host = Host::default().with_log(log.clone());
    let p = plugin_host("transform", "", &logger(2, &[0x66, 0xff, 0x6f], 1), host);
    p.call_raw("x", b"").unwrap();
    assert_eq!(log.0.lock().unwrap()[0].2, "f\u{fffd}o");
}

#[test]
fn now_ms_is_a_real_clock() {
    let p = plugin("transform", "", &clock_user());
    p.call_json("x", &json!(null)).unwrap();
}

#[test]
fn url_host_parsing() {
    assert_eq!(
        url_host("https://Example.COM/a").as_deref(),
        Some("example.com")
    );
    assert_eq!(url_host("http://a.b:81?x=1").as_deref(), Some("a.b"));
    assert_eq!(url_host("https://a.b#f").as_deref(), Some("a.b"));
    for bad in [
        "ftp://a.b",
        "https://",
        "https://:80/",
        "https://a b/",
        "https://u@a.b/",
        "https://[::1]/",
        "https://a.b:x/",
        "https://a.b:/",
        "",
    ] {
        assert_eq!(url_host(bad), None, "{bad}");
    }
}
