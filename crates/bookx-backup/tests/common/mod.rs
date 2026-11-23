//! Helpers shared by the repository tests: a throw-away library, and a small in-process WebDAV
//! server (files in memory, `MOVE` optional) for the network route.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bookx_backup::repo::LibrarySource;
use bookx_core::convert::{Chapter, create_epub};
use bookx_core::library::Highlight;
use bookx_core::opf::Metadata;
use bookx_store::{OpenedLibrary, open_library};

pub fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("bookx-bkt-{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

pub struct Lib {
    pub root: PathBuf,
    pub opened: OpenedLibrary,
    work: PathBuf,
}

impl Lib {
    pub fn new(name: &str) -> Lib {
        let base = tmp(name);
        let root = base.join("library");
        let opened = open_library(&root).unwrap();
        Lib {
            root,
            opened,
            work: base,
        }
    }

    pub fn base(&self) -> &Path {
        &self.work
    }

    /// Adds a book of about `filler * 2` bytes of text; returns its id.
    pub fn add(&mut self, title: &str, filler: usize) -> u64 {
        let md = Metadata {
            title: title.into(),
            authors: vec!["Ann Author".into()],
            ..Metadata::default()
        };
        let body = format!("<p>{}</p>", "lorem ipsum ".repeat(filler));
        let a = create_epub(
            &md,
            &[Chapter {
                title: "One".into(),
                body,
            }],
            None,
        );
        let src = self.work.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let p = src.join(format!("{title}.epub"));
        a.save(&p).unwrap();
        match self.opened.library.add_path(&p, true).unwrap() {
            bookx_core::library::AddOutcome::Added(id) => id,
            _ => panic!("not added"),
        }
    }

    pub fn source(&self) -> LibrarySource {
        LibrarySource::with_store(&self.root, self.opened.store.clone())
    }

    pub fn highlight(&mut self, id: u64, note: &str) {
        let b = self.opened.library.get_mut(id).unwrap();
        b.highlights.push(Highlight {
            id: format!("h{}", b.highlights.len()),
            spine: 0,
            start: 0,
            end: 5,
            color: "yellow".into(),
            style: String::new(),
            note: note.into(),
            text: "lorem".into(),
            created: chrono::Utc::now(),
        });
        self.opened.library.save().unwrap();
    }

    pub fn book_file(&self, id: u64) -> PathBuf {
        let b = self.opened.library.get(id).unwrap();
        self.root.join(&b.file)
    }

    /// Makes a file look an hour old, so it counts as settled.
    pub fn age(&self, rel: &str) {
        let f = std::fs::File::options()
            .write(true)
            .open(self.root.join(rel))
            .unwrap();
        f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
            .unwrap();
    }

    pub fn write(&self, rel: &str, body: &[u8]) {
        let p = self.root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }
}

impl Drop for Lib {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.work);
    }
}

/// Every file under `root` (relative path, content), without the app's own state folders.
pub fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    fn walk(dir: &Path, rel: &str, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name.starts_with("bookx-library.") {
                continue;
            }
            let r = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            if e.file_type().unwrap().is_dir() {
                walk(&e.path(), &r, out);
            } else {
                out.insert(r, std::fs::read(e.path()).unwrap());
            }
        }
    }
    walk(root, "", &mut out);
    out
}

// -- WebDAV ----------------------------------------------------------------------------------------------

pub struct Dav {
    pub port: u16,
    pub files: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    pub dirs: Arc<Mutex<BTreeSet<String>>>,
    pub log: Arc<Mutex<Vec<String>>>,
    pub allow_move: Arc<AtomicBool>,
    pub puts: Arc<AtomicUsize>,
    pub knobs: Arc<Knobs>,
}

/// What the stand-in server does with `Content-Range` uploads.
pub struct Knobs {
    /// Honour `Content-Range` (write at the offset); off: the range is ignored, as many servers do.
    pub ranged: AtomicBool,
    /// Answer 503 to every ranged `PUT` from this number on (counting from 0).
    pub fail_range_from: AtomicUsize,
    pub range_puts: AtomicUsize,
    pub range_bytes: AtomicUsize,
}

const SEG: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

pub fn dav() -> Dav {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let d = Dav {
        port,
        files: Arc::default(),
        dirs: Arc::new(Mutex::new(BTreeSet::from(["/dav".to_string()]))),
        log: Arc::default(),
        allow_move: Arc::new(AtomicBool::new(true)),
        puts: Arc::default(),
        knobs: Arc::new(Knobs {
            ranged: AtomicBool::new(true),
            fail_range_from: AtomicUsize::new(usize::MAX),
            range_puts: AtomicUsize::new(0),
            range_bytes: AtomicUsize::new(0),
        }),
    };
    let (f, di, lg, mv, pu, kn) = (
        d.files.clone(),
        d.dirs.clone(),
        d.log.clone(),
        d.allow_move.clone(),
        d.puts.clone(),
        d.knobs.clone(),
    );
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let (f, di, lg, mv, pu, kn) = (
                f.clone(),
                di.clone(),
                lg.clone(),
                mv.clone(),
                pu.clone(),
                kn.clone(),
            );
            std::thread::spawn(move || serve(s, &f, &di, &lg, &mv, &pu, &kn));
        }
    });
    d
}

impl Dav {
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/dav/", self.port)
    }
}

fn serve(
    mut s: TcpStream,
    files: &Mutex<BTreeMap<String, Vec<u8>>>,
    dirs: &Mutex<BTreeSet<String>>,
    log: &Mutex<Vec<String>>,
    allow_move: &AtomicBool,
    puts: &AtomicUsize,
    knobs: &Knobs,
) {
    s.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let mut buf = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        match s.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut it = head.lines().next().unwrap_or("").split_whitespace();
    let (method, target) = (
        it.next().unwrap_or("").to_string(),
        it.next().unwrap_or("").to_string(),
    );
    let header = |n: &str| {
        head.lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.eq_ignore_ascii_case(n).then(|| v.trim().to_string())
            })
            .unwrap_or_default()
    };
    let len: usize = header("content-length").parse().unwrap_or(0);
    while buf.len() < head_end + len {
        match s.read(&mut chunk) {
            Ok(0) | Err(_) => return, // an upload that was cut off is never stored
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let body = buf[head_end..head_end + len].to_vec();
    let key = percent_encoding::percent_decode_str(&target)
        .decode_utf8_lossy()
        .trim_end_matches('/')
        .to_string();
    log.lock().unwrap().push(format!("{method} {key}"));
    let reply = |s: &mut TcpStream, code: u16, ctype: &str, body: &[u8]| {
        let _ = write!(
            s,
            "HTTP/1.1 {code} X\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = s.write_all(body);
    };
    let parent = key
        .rsplit_once('/')
        .map(|(p, _)| p.to_string())
        .unwrap_or_default();
    let mut files = files.lock().unwrap();
    let mut dirs = dirs.lock().unwrap();
    match method.as_str() {
        "MKCOL" => {
            if dirs.contains(&key) {
                reply(&mut s, 405, "text/plain", b"");
            } else if !dirs.contains(&parent) {
                reply(&mut s, 409, "text/plain", b"");
            } else {
                dirs.insert(key);
                reply(&mut s, 201, "text/plain", b"");
            }
        }
        "PUT" => {
            let range = header("content-range");
            if !dirs.contains(&parent) {
                reply(&mut s, 409, "text/plain", b"");
            } else if !range.is_empty() && knobs.ranged.load(Ordering::Relaxed) {
                // "bytes a-b/total": write at the offset, which must be the end (or the start)
                let from: usize = range
                    .trim_start_matches("bytes ")
                    .split('-')
                    .next()
                    .and_then(|x| x.parse().ok())
                    .unwrap_or(0);
                let n = knobs.range_puts.fetch_add(1, Ordering::Relaxed);
                if n >= knobs.fail_range_from.load(Ordering::Relaxed) {
                    reply(&mut s, 503, "text/plain", b"");
                    return;
                }
                knobs.range_bytes.fetch_add(body.len(), Ordering::Relaxed);
                let cur = files.entry(key).or_default();
                if from == 0 {
                    cur.clear();
                }
                if from != cur.len() {
                    reply(&mut s, 416, "text/plain", b"");
                    return;
                }
                cur.extend_from_slice(&body);
                reply(&mut s, 201, "text/plain", b"");
            } else {
                puts.fetch_add(1, Ordering::Relaxed);
                files.insert(key, body);
                reply(&mut s, 201, "text/plain", b"");
            }
        }
        "MOVE" => {
            if !allow_move.load(Ordering::Relaxed) {
                reply(&mut s, 405, "text/plain", b"");
                return;
            }
            let dest = header("destination");
            let path = dest
                .split_once("://")
                .and_then(|(_, r)| r.find('/').map(|i| r[i..].to_string()))
                .unwrap_or(dest);
            let dkey = percent_encoding::percent_decode_str(&path)
                .decode_utf8_lossy()
                .trim_end_matches('/')
                .to_string();
            match files.remove(&key) {
                Some(v) => {
                    files.insert(dkey, v);
                    reply(&mut s, 201, "text/plain", b"");
                }
                None => reply(&mut s, 404, "text/plain", b""),
            }
        }
        "GET" => match files.get(&key) {
            Some(b) => reply(&mut s, 200, "application/octet-stream", b),
            None => reply(&mut s, 404, "text/plain", b""),
        },
        "DELETE" => {
            let had = files.remove(&key).is_some();
            reply(&mut s, if had { 204 } else { 404 }, "text/plain", b"");
        }
        "PROPFIND" => {
            if !dirs.contains(&key) {
                reply(&mut s, 404, "text/plain", b"");
                return;
            }
            let enc = |p: &str| {
                p.split('/')
                    .map(|x| percent_encoding::utf8_percent_encode(x, SEG).to_string())
                    .collect::<Vec<_>>()
                    .join("/")
            };
            let mut x = String::from(r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:">"#);
            x.push_str(&format!("<d:response><d:href>{}/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>", enc(&key)));
            if header("depth") != "0" {
                for d in dirs
                    .iter()
                    .filter(|d| d.rsplit_once('/').map(|(p, _)| p) == Some(&key))
                {
                    x.push_str(&format!("<d:response><d:href>{}/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>", enc(d)));
                }
                for (f, b) in files
                    .iter()
                    .filter(|(f, _)| f.rsplit_once('/').map(|(p, _)| p) == Some(&key))
                {
                    x.push_str(&format!("<d:response><d:href>{}</d:href><d:propstat><d:prop><d:resourcetype/><d:getcontentlength>{}</d:getcontentlength></d:prop></d:propstat></d:response>", enc(f), b.len()));
                }
            }
            x.push_str("</d:multistatus>");
            reply(&mut s, 207, "application/xml", x.as_bytes());
        }
        _ => reply(&mut s, 405, "text/plain", b""),
    }
}
