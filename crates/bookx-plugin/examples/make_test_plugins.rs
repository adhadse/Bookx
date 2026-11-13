//! Writes plugins of the other kinds for UI testing:
//!
//! ```text
//! cargo run -p bookx-plugin --example make_test_plugins -- <plugins dir> <mounts dir>
//! ```
//!
//! * `test-writer`: the worked example of `docs/plugin-development.md` (output format `.epubcopy`)
//! * `test-transform`: answers every `apply` with `files_changed = 2` and one note
//! * `test-device`: recognises the folder `<mounts dir>/PluginReader` and takes books into
//!   `plugin-books/last-sent.bin`

use std::fmt::Write as _;
use std::path::Path;

fn esc(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "\\{b:02x}");
        s
    })
}

/// A module whose `bookx_call` answers by the first letter of the operation with a fixed reply.
fn canned(replies: &[(u8, String)]) -> Vec<u8> {
    let mut data = String::new();
    let mut checks = String::new();
    let mut off = 1024usize;
    for (c, reply) in replies {
        let _ = writeln!(
            data,
            "(data (i32.const {off}) \"{}\")",
            esc(reply.as_bytes())
        );
        let packed = ((off as i64) << 32) | reply.len() as i64;
        let _ = writeln!(
            checks,
            "(if (i32.eq (local.get $c) (i32.const {c})) (then (return (i64.const {packed}))))"
        );
        off += reply.len() + 16;
    }
    let heap = (off + 4095) & !4095;
    let src = format!(
        r#"(module
  (memory (export "memory") 2)
  (global $heap (mut i32) (i32.const {heap}))
  (func (export "alloc") (param $n i32) (result i32) (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (local.get $p) (i32.and (i32.add (local.get $n) (i32.const 7)) (i32.const -8))))
    (local.get $p))
  (func (export "dealloc") (param i32 i32))
  {data}
  (func (export "bookx_call") (param $op i32) (param $ol i32) (param $ip i32) (param $il i32) (result i64)
    (local $c i32)
    (local.set $c (i32.load8_u (local.get $op)))
    {checks}
    (i64.const -2)))"#
    );
    wat::parse_str(&src).expect("the plugin source is valid WAT")
}

fn write(root: &Path, folder: &str, toml: &str, wasm: &[u8]) {
    let dir = root.join(folder);
    std::fs::create_dir_all(&dir).expect("cannot create the plugin folder");
    std::fs::write(dir.join("plugin.toml"), toml).expect("cannot write plugin.toml");
    std::fs::write(dir.join("plugin.wasm"), wasm).expect("cannot write plugin.wasm");
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(dir), Some(mounts)) = (args.next(), args.next()) else {
        eprintln!("usage: make_test_plugins <plugins dir> <mounts dir>");
        std::process::exit(2);
    };
    let root = Path::new(&dir);

    let example =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins/rawcopy-writer");
    let wat_text = std::fs::read_to_string(example.join("plugin.wat")).expect("example source");
    write(
        root,
        "test-writer",
        &std::fs::read_to_string(example.join("plugin.toml")).expect("example manifest"),
        &wat::parse_str(wat_text).expect("example assembles"),
    );

    write(
        root,
        "test-transform",
        "id = \"test.transform\"\nname = \"Test Transform\"\nversion = \"1.0.0\"\napi_version = 1\nkind = \"transform\"\ndescription = \"Reports two changed files\"\n",
        &canned(&[(
            b'a',
            r#"{"ok":{"files_changed":2,"notes":["plugin transform ran"]}}"#.to_string(),
        )]),
    );

    let mount = Path::new(&mounts).join("PluginReader");
    std::fs::create_dir_all(&mount).expect("cannot create the mount");
    let mount_json = serde_json_string(&mount.to_string_lossy());
    write(
        root,
        "test-device",
        "id = \"test.device\"\nname = \"Test Device\"\nversion = \"1.0.0\"\napi_version = 1\nkind = \"device\"\nextensions = [\"epub\"]\ndescription = \"Takes books into plugin-books/\"\n",
        &canned(&[
            (
                b'd',
                format!(r#"{{"ok":[{{"id":"r1","name":"Plugin Reader","mount":{mount_json}}}]}}"#),
            ),
            (b's', r#"{"ok":{"path":"plugin-books/last-sent.bin"}}"#.to_string()),
            (b'l', r#"{"ok":["plugin-books/last-sent.bin"]}"#.to_string()),
        ]),
    );
}

fn serde_json_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}
