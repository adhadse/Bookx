//! Shared fixtures: a small generated library and a scripted stdio client over in-memory pipes.
#![allow(dead_code)]

use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use bookx_core::convert::{Chapter, create_epub};
use bookx_core::library::{AddOutcome, Highlight, Library};
use bookx_core::opf::Metadata;
use bookx_mcp::{Config, Server};
use chrono::Utc;
use serde_json::{Value, json};

pub fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bookx-mcp-{tag}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

pub struct Fixture {
    pub dir: PathBuf,
    pub root: PathBuf,
    /// ids by short name
    pub dune: u64,
    pub dune2: u64,
    pub cook: u64,
    pub dup: u64,
    pub locked: u64,
    pub big: u64,
}

fn epub(dir: &Path, file: &str, md: &Metadata, chapters: &[(&str, String)]) -> PathBuf {
    let ch: Vec<Chapter> = chapters
        .iter()
        .map(|(t, b)| Chapter {
            title: t.to_string(),
            body: b.clone(),
        })
        .collect();
    let p = dir.join(file);
    // the writer keeps only the basics: set the rest (series, tags, rating) the way an edit does
    let mut book = bookx_core::book::OpenBook::from_archive(create_epub(md, &ch, None)).unwrap();
    book.set_metadata(md.clone()).unwrap();
    std::fs::write(&p, book.archive.to_bytes().unwrap()).unwrap();
    p
}

fn meta(title: &str, author: &str, series: &str, idx: f64, tags: &[&str]) -> Metadata {
    Metadata {
        title: title.into(),
        title_sort: title.into(),
        authors: vec![author.into()],
        author_sort: author.into(),
        series: series.into(),
        series_index: idx,
        tags: tags.iter().map(|t| t.to_string()).collect(),
        language: "en".into(),
        rating: 8,
        ..Default::default()
    }
}

fn add(lib: &mut Library, p: &Path, dup: bool) -> u64 {
    match lib.add_path(p, dup).unwrap() {
        AddOutcome::Added(i) | AddOutcome::Duplicate(i) => i,
    }
}

impl Fixture {
    pub fn new(tag: &str) -> Fixture {
        let dir = tmp(tag);
        let root = dir.join("lib");
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let mut o = bookx_store::open_library(&root).unwrap();
        let lib = &mut o.library;
        let dune = add(
            lib,
            &epub(
                &src,
                "dune.epub",
                &meta("Dune Alpha", "Frank Herbert", "Alpha Saga", 1.0, &["sf", "classic"]),
                &[
                    ("Desert", "<h1>Desert</h1><p>The spice must flow through the desert of Arrakis.</p><p>Second paragraph about sandworms.</p>".into()),
                    ("Water", "<h1>Water</h1><p>Water is life on the desert planet.</p>".into()),
                    ("Home", "<h1>Home</h1><p>The final chapter returns home.</p>".into()),
                ],
            ),
            false,
        );
        let dune2 = add(
            lib,
            &epub(
                &src,
                "dune2.epub",
                &meta("Alpha Two", "Frank Herbert", "Alpha Saga", 2.0, &["sf"]),
                &[(
                    "Messiah",
                    "<h1>Messiah</h1><p>A second book of the saga.</p>".into(),
                )],
            ),
            false,
        );
        let cook = add(
            lib,
            &epub(
                &src,
                "cook.epub",
                &meta("Cooking Basics", "Julia Child", "", 0.0, &["cooking"]),
                &[("Eggs", "<h1>Eggs</h1><p>Whisk the eggs gently.</p>".into())],
            ),
            false,
        );
        let dup = add(
            lib,
            &epub(
                &src,
                "dune-copy.epub",
                &meta("Dune Alpha", "Frank Herbert", "Alpha Saga", 1.0, &["sf"]),
                &[(
                    "Desert",
                    "<h1>Desert</h1><p>A copy of the first book.</p>".into(),
                )],
            ),
            true,
        );
        let locked = add(
            lib,
            &epub(
                &src,
                "locked.epub",
                &meta("Locked Book", "Secret Author", "", 0.0, &["drm"]),
                &[("One", "<h1>One</h1><p>You may not read this.</p>".into())],
            ),
            false,
        );
        let huge = format!(
            "<h1>Long</h1>{}",
            "<p>word salad and more words to fill the page.</p>".repeat(3000)
        );
        let big = add(
            lib,
            &epub(
                &src,
                "big.epub",
                &meta("The Long Book", "Wordy Writer", "", 0.0, &["long"]),
                &[
                    ("Long", huge),
                    ("Short", "<h1>Short</h1><p>The end.</p>".into()),
                ],
            ),
            false,
        );
        lib.get_mut(locked).unwrap().drm = "Adobe ADEPT".into();
        {
            let b = lib.get_mut(dune).unwrap();
            b.progress = 0.4;
            b.last_read = Some(Utc::now());
            b.highlights.push(Highlight {
                id: "h1".into(),
                spine: 0,
                start: 4,
                end: 9,
                color: "yellow".into(),
                style: String::new(),
                note: "remember this".into(),
                text: "spice".into(),
                created: Utc::now(),
            });
            b.highlights.push(Highlight {
                id: "h2".into(),
                spine: 1,
                start: 0,
                end: 5,
                color: "green".into(),
                style: String::new(),
                note: String::new(),
                text: "Water".into(),
                created: Utc::now() - chrono::Duration::days(1),
            });
        }
        lib.save().unwrap();
        let store = &o.store;
        store
            .index_book_text(
                dune,
                "The spice must flow through the desert of Arrakis. Water is life.",
            )
            .unwrap();
        store
            .index_book_text(cook, "Whisk the eggs gently.")
            .unwrap();
        store
            .index_book_text(locked, "You may not read this.")
            .unwrap();
        let c = store.create_collection("Favourites").unwrap();
        store.add_to_collection(c, dune).unwrap();
        Fixture {
            dir,
            root,
            dune,
            dune2,
            cook,
            dup,
            locked,
            big,
        }
    }

    pub fn config(&self) -> Config {
        let mut c = Config::new(&self.root);
        c.rate_limit = None;
        c
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// -- an in-memory pipe pair ---------------------------------------------------------------------

pub struct PipeReader {
    rx: Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
}

impl Read for PipeReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.pos >= self.buf.len() {
            match self.rx.recv() {
                Ok(b) => {
                    self.buf = b;
                    self.pos = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

pub struct PipeWriter(Sender<Vec<u8>>);

impl Write for PipeWriter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let _ = self.0.send(b.to_vec());
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub fn pipe() -> (PipeWriter, PipeReader) {
    let (tx, rx) = channel();
    (
        PipeWriter(tx),
        PipeReader {
            rx,
            buf: Vec::new(),
            pos: 0,
        },
    )
}

/// A scripted client: talks to a server on a thread through in-memory pipes.
pub struct Client {
    to_server: Option<PipeWriter>,
    from_server: BufReader<PipeReader>,
    next_id: i64,
    pub server: Arc<Server>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// notifications that arrived while waiting for an answer
    pub notes: Vec<Value>,
}

impl Client {
    pub fn start(cfg: Config) -> Client {
        let server = bookx_mcp::new_server(cfg);
        let (c2s_w, c2s_r) = pipe();
        let (s2c_w, s2c_r) = pipe();
        let s = server.clone();
        let thread = std::thread::spawn(move || {
            bookx_mcp::stdio::serve(s, BufReader::new(c2s_r), s2c_w).unwrap();
        });
        Client {
            to_server: Some(c2s_w),
            from_server: BufReader::new(s2c_r),
            next_id: 1,
            server,
            thread: Some(thread),
            notes: Vec::new(),
        }
    }

    /// A client that has done the initialize handshake.
    pub fn ready(cfg: Config) -> Client {
        let mut c = Client::start(cfg);
        let r = c.request(
            "initialize",
            json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test-client","version":"1.0"}}),
        );
        assert!(r.get("result").is_some(), "{r}");
        c.notify("notifications/initialized", json!({}));
        c
    }

    pub fn send_raw(&mut self, text: &str) {
        let w = self.to_server.as_mut().unwrap();
        w.write_all(text.as_bytes()).unwrap();
    }

    pub fn send(&mut self, v: &Value) {
        self.send_raw(&format!("{v}\n"));
    }

    pub fn notify(&mut self, method: &str, params: Value) {
        self.send(&json!({"jsonrpc":"2.0","method":method,"params":params}));
    }

    /// The next message from the server; `None` when it closed its output.
    pub fn recv(&mut self) -> Option<Value> {
        use std::io::BufRead;
        let mut line = String::new();
        if self.from_server.read_line(&mut line).ok()? == 0 {
            return None;
        }
        serde_json::from_str(line.trim()).ok()
    }

    /// Sends a request and waits for its answer; notifications on the way are kept in `notes`.
    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.request_with_id(json!(id), method, params)
    }

    pub fn request_with_id(&mut self, id: Value, method: &str, params: Value) -> Value {
        self.send(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        self.wait_for(&id)
    }

    pub fn wait_for(&mut self, id: &Value) -> Value {
        loop {
            let m = self.recv().expect("the server answered");
            if m.get("id") == Some(id) {
                return m;
            }
            self.notes.push(m);
        }
    }

    /// `tools/call`: the `result` object (panics on a protocol error).
    pub fn call(&mut self, name: &str, args: Value) -> Value {
        let r = self.request("tools/call", json!({"name":name,"arguments":args}));
        r.get("result")
            .cloned()
            .unwrap_or_else(|| panic!("{name}: {r}"))
    }

    /// The structured content of a successful call.
    pub fn ok(&mut self, name: &str, args: Value) -> Value {
        let r = self.call(name, args);
        assert_eq!(r["isError"], false, "{name}: {r}");
        r["structuredContent"].clone()
    }

    /// The error text of a failed call.
    pub fn err(&mut self, name: &str, args: Value) -> String {
        let r = self.call(name, args);
        assert_eq!(r["isError"], true, "{name} should fail: {r}");
        r["content"][0]["text"].as_str().unwrap().to_string()
    }

    pub fn close(mut self) {
        self.to_server = None;
        if let Some(t) = self.thread.take() {
            t.join().unwrap();
        }
    }
}
