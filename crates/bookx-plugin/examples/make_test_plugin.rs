//! Writes a tiny metadata-source plugin for manual and UI testing:
//!
//! ```text
//! cargo run -p bookx-plugin --example make_test_plugin -- <plugins dir> [publisher]
//! ```
//!
//! It creates `<dir>/test-source/plugin.toml` and `plugin.wasm`. The plugin answers every
//! `lookup` with one canned book whose publisher is `publisher` (default `Plugin Press`), and
//! answers nothing else.

use std::fmt::Write as _;

fn pack(ptr: u32, len: usize) -> i64 {
    ((ptr as i64) << 32) | len as i64
}

fn esc(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "\\{b:02x}");
        s
    })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(dir) = args.next() else {
        std::process::exit(2);
    };
    let publisher = args.next().unwrap_or_else(|| "Plugin Press".to_string());
    let reply = format!(
        r#"{{"ok":[{{"title":"","publisher":"{publisher}","published":"1999-09-09","tags":["from a plugin"],"language":"en"}}]}}"#
    );
    let src = format!(
        r#"(module
  (memory (export "memory") 2)
  (global $heap (mut i32) (i32.const 4096))
  (func (export "alloc") (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (local.get $p) (i32.and (i32.add (local.get $n) (i32.const 7)) (i32.const -8))))
    (local.get $p))
  (func (export "dealloc") (param i32 i32))
  (data (i32.const 1024) "{data}")
  (func (export "bookx_call") (param $op i32) (param $ol i32) (param $ip i32) (param $il i32) (result i64)
    (if (i32.eq (i32.load8_u (local.get $op)) (i32.const 108))
      (then (return (i64.const {packed}))))
    (i64.const -2))
)"#,
        data = esc(reply.as_bytes()),
        packed = pack(1024, reply.len()),
    );
    let wasm = wat::parse_str(&src).expect("the plugin source is valid WAT");
    let folder = std::path::Path::new(&dir).join("test-source");
    std::fs::create_dir_all(&folder).expect("cannot create the plugin folder");
    std::fs::write(
        folder.join("plugin.toml"),
        "id = \"test.source\"\nname = \"Test Source\"\nversion = \"1.0.0\"\napi_version = 1\nkind = \"metadata-source\"\ndescription = \"Answers every lookup with one canned book\"\n",
    )
    .expect("cannot write plugin.toml");
    std::fs::write(folder.join("plugin.wasm"), wasm).expect("cannot write plugin.wasm");
}
