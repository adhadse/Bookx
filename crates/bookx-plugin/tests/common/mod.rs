//! Test plugins written in WebAssembly text format, assembled with the `wat` crate, plus helpers.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use bookx_core::book::OpenBook;
use bookx_core::convert::{Chapter, create_epub};
use bookx_core::opf::Metadata;
use bookx_plugin::{Host, HostLog, HostNetwork, Manifest, Plugin};

pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "bookx-plugin-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `\xx` escapes for a WAT string literal.
pub fn esc(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("\\{b:02x}")).collect()
}

pub const IMPORT_LOG: &str = r#"(import "bookx_host" "log" (func $log (param i32 i32 i32)))"#;
pub const IMPORT_HTTP: &str =
    r#"(import "bookx_host" "http_get" (func $http_get (param i32 i32 i32) (result i32)))"#;
pub const IMPORT_NOW: &str = r#"(import "bookx_host" "now_ms" (func $now_ms (result i64)))"#;

/// Builds a module with a bump allocator, exported `memory`, `alloc`, `dealloc`, your `imports`, your
/// `body` (extra functions/exports) and data segments `(offset, bytes)`. The heap starts after the
/// highest data segment. `extra_exports` is added verbatim before the body.
pub fn module(imports: &str, body: &str, data: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let end = data
        .iter()
        .map(|(o, b)| o + b.len() as u32)
        .max()
        .unwrap_or(0)
        .max(1024);
    let heap = (end + 4095) & !4095;
    let pages = heap / 65536 + 2;
    let segs: String = data
        .iter()
        .map(|(o, b)| format!("(data (i32.const {o}) \"{}\")\n", esc(b)))
        .collect();
    let src = format!(
        r#"(module
  {imports}
  (memory (export "memory") {pages})
  (global $heap (mut i32) (i32.const {heap}))
  (func $alloc (export "alloc") (param $n i32) (result i32)
    (local $p i32) (local $end i32) (local $need i32)
    (local.set $p (global.get $heap))
    (local.set $end (i32.add (local.get $p) (i32.and (i32.add (local.get $n) (i32.const 7)) (i32.const -8))))
    (if (i32.gt_u (local.get $end) (i32.shl (memory.size) (i32.const 16)))
      (then
        (local.set $need (i32.shr_u
          (i32.add (i32.sub (local.get $end) (i32.shl (memory.size) (i32.const 16))) (i32.const 65535))
          (i32.const 16)))
        (if (i32.eq (memory.grow (local.get $need)) (i32.const -1))
          (then (return (i32.const -1))))))
    (global.set $heap (local.get $end))
    (local.get $p))
  (func (export "dealloc") (param i32 i32))
  {segs}
  {body}
)"#
    );
    wat::parse_str(&src).unwrap_or_else(|e| panic!("bad WAT: {e}\n{src}"))
}

fn pack(ptr: u32, len: usize) -> i64 {
    ((ptr as i64) << 32) | len as i64
}

const CALL_SIG: &str =
    r#"(param $op i32) (param $ol i32) (param $ip i32) (param $il i32) (result i64)"#;

/// A plugin whose `bookx_call` answers by the first byte of the operation name with a fixed reply;
/// unknown operations return -2. Replies are laid out from offset 1024.
pub fn canned(replies: &[(u8, Vec<u8>)]) -> Vec<u8> {
    canned_with("", replies)
}

pub fn canned_with(imports: &str, replies: &[(u8, Vec<u8>)]) -> Vec<u8> {
    let mut data = Vec::new();
    let mut checks = String::new();
    let mut off = 1024u32;
    for (c, reply) in replies {
        data.push((off, reply.clone()));
        checks.push_str(&format!(
            "(if (i32.eq (local.get $c) (i32.const {c})) (then (return (i64.const {}))))\n",
            pack(off, reply.len())
        ));
        off += reply.len() as u32 + 16;
    }
    let body = format!(
        r#"(func (export "bookx_call") {CALL_SIG}
            (local $c i32)
            (local.set $c (i32.load8_u (local.get $op)))
            {checks}
            (i64.const -2))"#
    );
    module(imports, &body, &data)
}

/// Every operation answers the same reply.
pub fn always(reply: &[u8]) -> Vec<u8> {
    let data = vec![(1024u32, reply.to_vec())];
    let body = format!(
        r#"(func (export "bookx_call") {CALL_SIG} (i64.const {}))"#,
        pack(1024, reply.len())
    );
    module("", &body, &data)
}

/// `bookx_call` with a custom body that must leave an i64.
pub fn with_call(imports: &str, call_body: &str, data: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let body = format!(r#"(func (export "bookx_call") {CALL_SIG} {call_body})"#);
    module(imports, &body, data)
}

/// Replies `{"ok": <input>}`, so the input JSON is echoed back as the result.
pub fn echo() -> Vec<u8> {
    with_call(
        "",
        r#"(local $out i32)
           (local.set $out (call $alloc (i32.add (local.get $il) (i32.const 7))))
           (memory.copy (local.get $out) (i32.const 1024) (i32.const 6))
           (memory.copy (i32.add (local.get $out) (i32.const 6)) (local.get $ip) (local.get $il))
           (i32.store8 (i32.add (local.get $out) (i32.add (local.get $il) (i32.const 6))) (i32.const 125))
           (i64.or
             (i64.shl (i64.extend_i32_u (local.get $out)) (i64.const 32))
             (i64.extend_i32_u (i32.add (local.get $il) (i32.const 7))))"#,
        &[(1024, br#"{"ok":"#.to_vec())],
    )
}

pub fn infinite_loop() -> Vec<u8> {
    with_call("", "(loop $l (br $l)) (i64.const 0)", &[])
}

/// Counts to `n` then answers `{"ok":true}`.
pub fn counting_loop(n: u32) -> Vec<u8> {
    let reply = br#"{"ok":true}"#.to_vec();
    with_call(
        "",
        &format!(
            r#"(local $i i32)
               (loop $l
                 (local.set $i (i32.add (local.get $i) (i32.const 1)))
                 (br_if $l (i32.lt_u (local.get $i) (i32.const {n}))))
               (i64.const {})"#,
            pack(1024, reply.len())
        ),
        &[(1024, reply)],
    )
}

/// Tries to grow memory by `pages`; reports -4 when the host refuses, else `{"ok":"grown"}`.
pub fn grower(pages: u32) -> Vec<u8> {
    let reply = br#"{"ok":"grown"}"#.to_vec();
    with_call(
        "",
        &format!(
            r#"(if (i32.eq (memory.grow (i32.const {pages})) (i32.const -1))
                 (then (return (i64.const -4))))
               (i64.const {})"#,
            pack(1024, reply.len())
        ),
        &[(1024, reply)],
    )
}

pub fn trapper() -> Vec<u8> {
    with_call("", "unreachable", &[])
}

pub fn div_by_zero() -> Vec<u8> {
    with_call(
        "",
        "(i64.extend_i32_s (i32.div_s (i32.const 1) (i32.sub (local.get $il) (local.get $il))))",
        &[],
    )
}

pub fn recursion() -> Vec<u8> {
    let body = r#"
      (func $r (param i32) (result i32) (call $r (i32.add (local.get 0) (i32.const 1))))
      (func (export "bookx_call") (param $op i32) (param $ol i32) (param $ip i32) (param $il i32) (result i64)
        (i64.extend_i32_s (call $r (i32.const 0))))"#;
    module("", body, &[])
}

/// Input is a URL; replies with the raw body of `http_get(url)` (so the mock should answer a valid
/// envelope), or error code `-100 + http_code` when the host refuses.
pub fn http_client() -> Vec<u8> {
    with_call(
        IMPORT_HTTP,
        r#"(local $r i32)
           (local.set $r (call $http_get (local.get $ip) (local.get $il) (i32.const 512)))
           (if (i32.ne (local.get $r) (i32.const 0))
             (then (return (i64.add (i64.const -100) (i64.extend_i32_s (local.get $r))))))
           (i64.or
             (i64.shl (i64.extend_i32_u (i32.load (i32.const 512))) (i64.const 32))
             (i64.extend_i32_u (i32.load (i32.const 516))))"#,
        &[],
    )
}

/// Calls `http_get` `n` times with the input URL, then answers `{"ok":<last code>}`-ish: the last
/// result code is returned as `-100 + code` if non-zero else a fixed ok reply.
pub fn http_repeat(n: u32) -> Vec<u8> {
    let reply = br#"{"ok":"done"}"#.to_vec();
    with_call(
        IMPORT_HTTP,
        &format!(
            r#"(local $i i32) (local $r i32)
               (loop $l
                 (local.set $r (call $http_get (local.get $ip) (local.get $il) (i32.const 512)))
                 (if (i32.ne (local.get $r) (i32.const 0))
                   (then (return (i64.add (i64.const -100) (i64.extend_i32_s (local.get $r))))))
                 (local.set $i (i32.add (local.get $i) (i32.const 1)))
                 (br_if $l (i32.lt_u (local.get $i) (i32.const {n}))))
               (i64.const {})"#,
            pack(1024, reply.len())
        ),
        &[(1024, reply)],
    )
}

/// Writes `msg` to the log `n` times at level `level`.
pub fn logger(level: i32, msg: &[u8], n: u32) -> Vec<u8> {
    let reply = br#"{"ok":null}"#.to_vec();
    with_call(
        IMPORT_LOG,
        &format!(
            r#"(local $i i32)
               (loop $l
                 (call $log (i32.const {level}) (i32.const 2048) (i32.const {}))
                 (local.set $i (i32.add (local.get $i) (i32.const 1)))
                 (br_if $l (i32.lt_u (local.get $i) (i32.const {n}))))
               (i64.const {})"#,
            msg.len(),
            pack(1024, reply.len())
        ),
        &[(1024, reply), (2048, msg.to_vec())],
    )
}

/// Answers `{"ok":true}` only when `now_ms()` looks like a real clock, else traps.
pub fn clock_user() -> Vec<u8> {
    let reply = br#"{"ok":true}"#.to_vec();
    with_call(
        IMPORT_NOW,
        &format!(
            r#"(if (i64.lt_s (call $now_ms) (i64.const 1600000000000)) (then unreachable))
               (i64.const {})"#,
            pack(1024, reply.len())
        ),
        &[(1024, reply)],
    )
}

/// Keeps a counter in a global; if state survived between calls the reply digit would change.
pub fn stateful() -> Vec<u8> {
    let reply = br#"{"ok":0}"#.to_vec();
    let body = format!(
        r#"(global $count (mut i32) (i32.const 0))
           (func (export "bookx_call") {CALL_SIG}
             (global.set $count (i32.add (global.get $count) (i32.const 1)))
             (i32.store8 (i32.const 1030) (i32.add (i32.const 48) (global.get $count)))
             ;; also dirty the heap so a leak would show up
             (i32.store (call $alloc (i32.const 64)) (i32.const 7))
             (i64.const {}))"#,
        pack(1024, reply.len())
    );
    module("", &body, &[(1024, reply)])
}

/// A `start` function that never ends.
pub fn start_loops() -> Vec<u8> {
    let body = format!(
        r#"(func $s (loop $l (br $l)))
           (start $s)
           (func (export "bookx_call") {CALL_SIG} (i64.const -2))"#
    );
    module("", &body, &[])
}

pub fn start_traps() -> Vec<u8> {
    let body = format!(
        r#"(func $s unreachable)
           (start $s)
           (func (export "bookx_call") {CALL_SIG} (i64.const -2))"#
    );
    module("", &body, &[])
}

/// `alloc` that returns a pointer outside the memory.
pub fn bad_alloc(ptr: i32) -> Vec<u8> {
    let src = format!(
        r#"(module
  (memory (export "memory") 1)
  (func (export "alloc") (param i32) (result i32) (i32.const {ptr}))
  (func (export "dealloc") (param i32 i32))
  (func (export "bookx_call") {CALL_SIG} (i64.const -2)))"#
    );
    wat::parse_str(src).unwrap()
}

/// Reply pointing to `ptr`/`len` as given (possibly invalid).
pub fn raw_return(ptr: u32, len: u32) -> Vec<u8> {
    with_call(
        "",
        &format!("(i64.const {})", ((ptr as i64) << 32) | len as i64),
        &[],
    )
}

pub fn returns_code(code: i64) -> Vec<u8> {
    with_call("", &format!("(i64.const {code})"), &[])
}

pub fn wat(src: &str) -> Vec<u8> {
    wat::parse_str(src).unwrap_or_else(|e| panic!("bad WAT: {e}"))
}

pub const MINIMAL_EXPORTS: &str = r#"
  (memory (export "memory") 1)
  (func (export "alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "dealloc") (param i32 i32))
  (func (export "bookx_call") (param i32 i32 i32 i32) (result i64) (i64.const -2))
"#;

/// Reply bytes for `{"ok": value}`.
pub fn ok(value: &str) -> Vec<u8> {
    format!(r#"{{"ok":{value}}}"#).into_bytes()
}

pub fn manifest_toml(id: &str, kind: &str, extra: &str) -> String {
    format!(
        "id = \"{id}\"\nname = \"Test {id}\"\nversion = \"1.0.0\"\napi_version = 1\nkind = \"{kind}\"\n{extra}\n"
    )
}

pub fn manifest(id: &str, kind: &str, extra: &str) -> Manifest {
    Manifest::parse(&manifest_toml(id, kind, extra)).unwrap()
}

pub fn plugin(kind: &str, extra: &str, wasm: &[u8]) -> Plugin {
    plugin_host(kind, extra, wasm, Host::default())
}

pub fn plugin_host(kind: &str, extra: &str, wasm: &[u8], host: Host) -> Plugin {
    Plugin::from_bytes(manifest("test.plugin", kind, extra), wasm, host).unwrap()
}

pub fn arc(kind: &str, extra: &str, wasm: &[u8]) -> Arc<Plugin> {
    Arc::new(plugin(kind, extra, wasm))
}

/// Writes `<root>/<folder>/plugin.toml` and `plugin.wasm`.
pub fn install(root: &Path, folder: &str, toml: &str, wasm: &[u8]) -> PathBuf {
    let d = root.join(folder);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("plugin.toml"), toml).unwrap();
    std::fs::write(d.join("plugin.wasm"), wasm).unwrap();
    d
}

#[derive(Default)]
pub struct MockNet {
    pub calls: Mutex<Vec<String>>,
    pub body: Vec<u8>,
    pub fail: bool,
}

impl MockNet {
    pub fn replying(body: &[u8]) -> Arc<MockNet> {
        Arc::new(MockNet {
            body: body.to_vec(),
            ..Default::default()
        })
    }

    pub fn failing() -> Arc<MockNet> {
        Arc::new(MockNet {
            fail: true,
            ..Default::default()
        })
    }

    pub fn urls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl HostNetwork for MockNet {
    fn get(&self, url: &str, _max: usize) -> Result<Vec<u8>> {
        self.calls.lock().unwrap().push(url.to_string());
        if self.fail {
            bail!("mock network down");
        }
        Ok(self.body.clone())
    }
}

#[derive(Default)]
pub struct VecLog(pub Mutex<Vec<(String, u8, String)>>);

impl HostLog for VecLog {
    fn log(&self, id: &str, level: u8, msg: &str) {
        self.0.lock().unwrap().push((id.into(), level, msg.into()));
    }
}

pub fn sample_epub(title: &str) -> OpenBook {
    let md = Metadata {
        title: title.into(),
        authors: vec!["Tester".into()],
        language: "en".into(),
        ..Default::default()
    };
    let ch = Chapter {
        title: "One".into(),
        body: "<p>hello</p>".into(),
    };
    OpenBook::from_archive(create_epub(&md, &[ch], None)).unwrap()
}

pub fn epub_b64(title: &str) -> String {
    B64.encode(sample_epub(title).archive.to_bytes().unwrap())
}
