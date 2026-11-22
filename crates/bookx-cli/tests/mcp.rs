//! `bookx-cli mcp` as a real process: stdio and HTTP.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use bookx_core::convert::{Chapter, create_epub};
use bookx_core::library::AddOutcome;
use bookx_core::opf::Metadata;
use serde_json::{Value, json};

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bookx-cli-mcp-{tag}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn library(dir: &Path) -> PathBuf {
    let root = dir.join("lib");
    let mut o = bookx_store::open_library(&root).unwrap();
    let md = Metadata {
        title: "Smoke Test Book".into(),
        authors: vec!["Sam Smoke".into()],
        ..Default::default()
    };
    let ch = vec![Chapter {
        title: "One".into(),
        body: "<p>Hello from the smoke test.</p>".into(),
    }];
    let p = dir.join("smoke.epub");
    std::fs::write(&p, create_epub(&md, &ch, None).to_bytes().unwrap()).unwrap();
    assert!(matches!(
        o.library.add_path(&p, false).unwrap(),
        AddOutcome::Added(_)
    ));
    o.library.save().unwrap();
    root
}

fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_bookx-cli"));
    c.env("BOOKX_SECRETS", "memory");
    c
}

struct Stdio3 {
    child: Child,
    out: BufReader<std::process::ChildStdout>,
}

impl Stdio3 {
    fn start(root: &Path, extra: &[&str]) -> Stdio3 {
        let mut child = bin()
            .arg("--library")
            .arg(root)
            .arg("mcp")
            .args(extra)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        Stdio3 { child, out }
    }

    fn ask(&mut self, v: Value) -> Value {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{v}").unwrap();
        stdin.flush().unwrap();
        loop {
            let mut line = String::new();
            self.out.read_line(&mut line).unwrap();
            let v: Value = serde_json::from_str(line.trim()).unwrap_or_else(|e| {
                panic!("stdout must carry only protocol messages: {e}: {line:?}")
            });
            // notifications (log messages) may come before the answer
            if v.get("id").is_some() {
                return v;
            }
        }
    }
}

#[test]
fn the_real_binary_speaks_mcp_over_stdio() {
    let d = tmp("stdio");
    let root = library(&d);
    let mut s = Stdio3::start(&root, &[]);
    let r = s.ask(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}}}));
    assert_eq!(r["result"]["serverInfo"]["name"], "bookx");
    writeln!(
        s.child.stdin.as_mut().unwrap(),
        "{}",
        json!({"jsonrpc":"2.0","method":"notifications/initialized"})
    )
    .unwrap();
    let r = s.ask(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_books","arguments":{"query":"smoke"}}}));
    assert_eq!(
        r["result"]["structuredContent"]["books"][0]["title"],
        "Smoke Test Book"
    );
    let id = r["result"]["structuredContent"]["books"][0]["id"]
        .as_u64()
        .unwrap();
    let r = s.ask(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"read_book_text","arguments":{"book_id":id}}}));
    assert!(
        r["result"]["structuredContent"]["text"]
            .as_str()
            .unwrap()
            .contains("Hello from the smoke test")
    );
    // read-only by default: no write tools
    let r = s.ask(json!({"jsonrpc":"2.0","id":4,"method":"tools/list"}));
    assert!(
        !r["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "add_tags")
    );
    // closing stdin ends the process cleanly
    drop(s.child.stdin.take());
    let status = s.child.wait().unwrap();
    assert!(status.success(), "{status:?}");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn allow_write_lists_the_write_tools_and_changes_survive() {
    let d = tmp("write");
    let root = library(&d);
    let mut s = Stdio3::start(&root, &["--allow-write"]);
    s.ask(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}}}));
    let r = s.ask(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"add_tags","arguments":{"book_id":1,"tags":["from-mcp"]}}}));
    assert_eq!(r["result"]["isError"], false, "{r}");
    drop(s.child.stdin.take());
    assert!(s.child.wait().unwrap().success());
    let o = bookx_store::open_library(&root).unwrap();
    assert!(
        o.library
            .get(1)
            .unwrap()
            .meta
            .tags
            .contains(&"from-mcp".to_string())
    );
    assert!(root.join(".bookx/mcp-audit.log").exists());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn option_mistakes_are_reported() {
    let d = tmp("opts");
    let root = library(&d);
    let out = bin()
        .arg("--library")
        .arg(&root)
        .args(["mcp", "--allow-dir", "/tmp"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--allow-write"));
    let out = bin()
        .arg("--library")
        .arg(d.join("nothing-here"))
        .arg("mcp")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no library"));
    let out = bin()
        .arg("--library")
        .arg(&root)
        .args(["mcp", "--http", "127.0.0.1:0", "--token", "short"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("16 characters"));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn the_real_binary_serves_http_with_the_token() {
    let d = tmp("http");
    let root = library(&d);
    let token = "abcdefghijklmnopqrstuvwxyz012345";
    let mut child = bin()
        .arg("--library")
        .arg(&root)
        .args(["mcp", "--http", "127.0.0.1:0", "--token", token])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    let url = line
        .trim()
        .strip_prefix("MCP server on ")
        .unwrap_or_else(|| panic!("{line:?}"))
        .to_string();
    let u = url::Url::parse(&url).unwrap();
    let body = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"recent_books","arguments":{}}}).to_string();
    let bearer = format!("Bearer {token}");
    let ok = bookx_serve::httpc::request(
        "POST",
        &u,
        &[("Authorization", bearer.as_str())],
        body.as_bytes(),
        std::time::Duration::from_secs(10),
    )
    .unwrap();
    assert_eq!(ok.status, 200);
    let no = bookx_serve::httpc::request(
        "POST",
        &u,
        &[],
        body.as_bytes(),
        std::time::Duration::from_secs(10),
    )
    .unwrap();
    assert_eq!(no.status, 401);
    let _ = child.kill();
    let _ = child.wait();
    let mut rest = String::new();
    let _ = child.stderr.take().unwrap().read_to_string(&mut rest);
    assert!(
        !rest.contains(token),
        "the token is not echoed when it was given"
    );
    let _ = std::fs::remove_dir_all(d);
}
