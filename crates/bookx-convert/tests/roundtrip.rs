//! Property style tests: random documents written by every writer must read back with their text,
//! and no reader may panic or hang on damaged input. Both run without Calibre.
//!
//! `BOOKX_FUZZ_CASES=n` changes the number of random documents (default 12) and
//! `BOOKX_FUZZ_SEED=n` the starting seed, so a failure can be replayed.

mod common;
mod corpus;

use std::io::Write;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use bookx_convert::inspect::{score, Digest};
use bookx_convert::{convert, read_input, ConvertOptions};
use bookx_core::archive::Archive;
use bookx_core::book::OpenBook;
use bookx_core::convert::{create_epub, Chapter};

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() as usize) % n.max(1)
    }
}

const WORDS: &[&str] = &[
    "river", "lantern", "autumn", "whisper", "copper", "harbour", "violet", "meadow", "thunder",
    "marble", "garden", "compass", "feather", "winter", "silver", "orchard", "candle", "shadow",
    "bridge", "falcon", "ember", "willow", "granite", "voyage", "mirror", "saddle", "anchor",
];
/// Words that stress escaping and encodings but fit every legacy code page.
const TRICKY_SAFE: &[&str] = &[
    "AT&T",
    "a<b",
    "x>y",
    "\"quoted\"",
    "it's",
    "café",
    "naïve",
    "über",
    "100%",
    "[1]",
    "(paren)",
    "{brace}",
    "back\\slash",
    "semi;colon",
    "§5",
    "2×3",
];
/// Beyond the Windows-1252 repertoire.
const TRICKY_WIDE: &[&str] = &[
    "日本語",
    "한국어",
    "Привет",
    "مرحبا",
    "😀",
    "ελληνικά",
    "—",
    "…",
];

fn random_chapters(rng: &mut Lcg, wide: bool) -> Vec<Chapter> {
    let n_chapters = 1 + rng.below(5);
    let mut out = Vec::new();
    for c in 0..n_chapters {
        let mut body = format!(
            "<h1 id=\"c{c}\">Chapter {} {}</h1>\n",
            c + 1,
            WORDS[rng.below(WORDS.len())]
        );
        for _ in 0..(2 + rng.below(6)) {
            let mut words = Vec::new();
            for _ in 0..(5 + rng.below(40)) {
                let r = rng.below(10);
                let w = if r == 0 {
                    TRICKY_SAFE[rng.below(TRICKY_SAFE.len())]
                } else if r == 1 && wide {
                    TRICKY_WIDE[rng.below(TRICKY_WIDE.len())]
                } else {
                    WORDS[rng.below(WORDS.len())]
                };
                words.push(w);
            }
            let text = words
                .join(" ")
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            match rng.below(8) {
                0 => body.push_str(&format!(
                    "<h2>Section {}</h2>\n<p>{text}</p>\n",
                    rng.below(100)
                )),
                1 => body.push_str(&format!(
                    "<ul><li>{text}</li><li>{} {}</li></ul>\n",
                    WORDS[rng.below(WORDS.len())],
                    WORDS[rng.below(WORDS.len())]
                )),
                2 => body.push_str(&format!("<blockquote><p>{text}</p></blockquote>\n")),
                _ => body.push_str(&format!("<p>{text}</p>\n")),
            }
        }
        out.push(Chapter {
            title: format!("Chapter {}", c + 1),
            body,
        });
    }
    out
}

fn random_book(seed: u64, wide: bool) -> OpenBook {
    let mut rng = Lcg(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    let chapters = random_chapters(&mut rng, wide);
    let mut md = common::metadata();
    md.title = format!("Random {seed} {}", WORDS[rng.below(WORDS.len())]);
    let cover = (rng.below(2) == 0).then(|| (common::jpeg(120, 180), "jpg"));
    let arc: Archive = create_epub(&md, &chapters, cover);
    OpenBook::from_archive(Archive::from_bytes(arc.to_bytes().unwrap()).unwrap()).unwrap()
}

/// (extension, readable name for the reader, may hold characters beyond Windows-1252)
const FORMATS: &[(&str, bool)] = &[
    ("epub", true),
    ("kepub", true),
    ("mobi", true),
    ("azw3", true),
    ("fb2", true),
    ("htmlz", true),
    ("txtz", true),
    ("txt", true),
    ("html", true),
    ("md", true),
    ("docx", true),
    ("rtf", true),
    ("pdb", false),
    ("pmlz", false),
    ("rb", false),
    ("snb", true),
];

fn cases() -> u64 {
    std::env::var("BOOKX_FUZZ_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(12)
}

fn seed0() -> u64 {
    std::env::var("BOOKX_FUZZ_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
}

fn min_recall(ext: &str) -> f64 {
    match ext {
        // RTF and the PML family have no real list/heading structure to glue words back
        "rtf" | "pdb" | "pmlz" | "rb" => 0.97,
        "txt" | "txtz" | "md" => 0.98,
        _ => 0.995,
    }
}

#[test]
fn random_documents_survive_every_writer_and_reader() {
    let readers = bookx_formats::default_readers();
    let mut failures = Vec::new();
    for i in 0..cases() {
        let seed = seed0() + i;
        for &(ext, wide) in FORMATS {
            let book = random_book(seed, wide);
            let truth = Digest::of(&book);
            let epub = book.archive.to_bytes().unwrap();
            let out = match convert(
                &readers,
                &epub,
                "random.epub",
                ext,
                &ConvertOptions::default(),
                &|_| {},
                &AtomicBool::new(false),
            ) {
                Ok(o) => o,
                Err(e) => {
                    failures.push(format!("seed {seed} → {ext}: write failed: {e:#}"));
                    continue;
                }
            };
            let name = format!("random.{}", out.extension);
            let back = match read_input(&readers, &out.bytes, &name) {
                Ok(b) => b,
                Err(e) => {
                    failures.push(format!("seed {seed} → {ext}: read back failed: {e:#}"));
                    continue;
                }
            };
            let d = Digest::of(&back);
            let s = score(&truth, &d);
            if s.recall < min_recall(ext) {
                let (missing, _) = bookx_convert::inspect::word_delta(&truth.words, &d.words);
                failures.push(format!(
                    "seed {seed} → {ext}: recall {:.4}, missing {:?}",
                    s.recall,
                    missing.iter().take(6).collect::<Vec<_>>()
                ));
            }
            if s.order < 0.98 {
                failures.push(format!("seed {seed} → {ext}: order {:.3}", s.order));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// -- damaged input ------------------------------------------------------------------------------

fn run_with_limit<T: Send + 'static>(
    what: String,
    limit: Duration,
    f: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)));
    });
    match rx.recv_timeout(limit) {
        Ok(Ok(v)) => v,
        Ok(Err(_)) => panic!("{what}: panicked"),
        Err(_) => panic!("{what}: still running after {limit:?}"),
    }
}

fn mutations(data: &[u8], rng: &mut Lcg) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let n = data.len();
    if n == 0 {
        return out;
    }
    for frac in [0, 1, 7, 33, 50, 90, 99] {
        out.push((
            format!("truncated to {frac}%"),
            data[..n * frac / 100].to_vec(),
        ));
    }
    for k in 0..6 {
        let mut d = data.to_vec();
        for _ in 0..(1 + n / 200).min(40) {
            let at = rng.below(n);
            d[at] ^= 1 << rng.below(8);
        }
        out.push((format!("bit flips #{k}"), d));
    }
    let mut d = data.to_vec();
    let at = rng.below(n);
    let len = (n - at).min(64 + rng.below(512));
    d[at..at + len].fill(0);
    out.push((format!("zeroed {len} bytes at {at}"), d));
    let mut d = data.to_vec();
    let at = rng.below(n);
    d[at..].fill(0xFF);
    out.push((format!("0xFF from {at}"), d));
    out
}

fn fixture_files() -> Vec<(String, Vec<u8>)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/calibre-made");
    let mut v: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    std::fs::read(e.path()).ok().map(|b| (name, b))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

#[test]
fn damaged_files_never_panic_or_hang() {
    let mut files: Vec<(String, Vec<u8>)> = corpus::all()
        .into_iter()
        .map(|s| (s.file, s.bytes))
        .collect();
    files.extend(fixture_files());
    // writer output too: the formats Bookx writes must also survive damage
    let readers = bookx_formats::default_readers();
    let book = random_book(7, false);
    let epub = book.archive.to_bytes().unwrap();
    for ext in [
        "mobi", "azw3", "fb2", "docx", "rtf", "pdb", "pmlz", "rb", "snb", "htmlz", "txtz",
    ] {
        if let Ok(o) = convert(
            &readers,
            &epub,
            "w.epub",
            ext,
            &ConvertOptions::default(),
            &|_| {},
            &AtomicBool::new(false),
        ) {
            files.push((format!("bookx-writer.{}", o.extension), o.bytes));
        }
    }
    let mut rng = Lcg(seed0());
    let started = Instant::now();
    let mut cases = 0;
    for (name, data) in files {
        // big files are mutated less to keep the run short
        let data = if data.len() > 400_000 {
            data[..400_000].to_vec()
        } else {
            data
        };
        for (what, bytes) in mutations(&data, &mut rng) {
            let label = format!("{name}: {what}");
            let nm = name.clone();
            run_with_limit(label, Duration::from_secs(60), move || {
                let readers = bookx_formats::default_readers();
                if let Ok(book) = read_input(&readers, &bytes, &nm) {
                    // anything that was readable must also be convertible
                    let _ = Digest::of(&book);
                    let _ = convert(
                        &readers,
                        &bytes,
                        &nm,
                        "azw3",
                        &ConvertOptions::default(),
                        &|_| {},
                        &AtomicBool::new(false),
                    );
                }
            });
            cases += 1;
        }
    }
    assert!(cases > 300, "only {cases} damaged inputs were tried");
    assert!(
        started.elapsed() < Duration::from_secs(600),
        "the damaged input run took {:?}",
        started.elapsed()
    );
}

/// A zip whose only member (`name`) claims to be `size` bytes of zeros (a few hundred KB packed).
fn zip_bomb(name: &str, size: usize) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    z.start_file(
        name,
        zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .large_file(true),
    )
    .unwrap();
    let chunk = vec![0u8; 1 << 20];
    for _ in 0..size / chunk.len() {
        z.write_all(&chunk).unwrap();
    }
    z.finish().unwrap().into_inner()
}

/// The same compressed member under another name, without compressing again.
fn rename_member(bomb: &[u8], name: &str) -> Vec<u8> {
    let mut src = zip::ZipArchive::new(std::io::Cursor::new(bomb.to_vec())).unwrap();
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let f = src.by_index_raw(0).unwrap();
    z.raw_copy_file_rename(f, name).unwrap();
    z.finish().unwrap().into_inner()
}

#[test]
fn zip_bombs_are_refused_without_exhausting_memory() {
    // just over the 256 MB a member of a converted format may expand to, and over the 512 MB of EPUB
    let over_format = zip_bomb("x", 260 << 20);
    let over_epub = zip_bomb("x", 520 << 20);
    for (file, member, bomb) in [
        ("bomb.epub", "OEBPS/chapter.xhtml", &over_epub),
        ("bomb.docx", "word/document.xml", &over_format),
        ("bomb.odt", "content.xml", &over_format),
        ("bomb.fbz", "book.fb2", &over_format),
        ("bomb.htmlz", "index.html", &over_format),
        ("bomb.txtz", "index.txt", &over_format),
        ("bomb.cbz", "page001.png", &over_format),
        ("bomb.pmlz", "index.pml", &over_format),
    ] {
        let bytes = rename_member(bomb, member);
        assert!(bytes.len() < 4 << 20, "{file} is {} bytes", bytes.len());
        let started = Instant::now();
        let name = file.to_string();
        let ok = run_with_limit(
            format!("zip bomb {file}"),
            Duration::from_secs(120),
            move || read_input(&bookx_formats::default_readers(), &bytes, &name).is_ok(),
        );
        // a refusal is right; a truncated read would mean the whole expansion was held
        assert!(!ok, "{file}: the bomb was accepted as a book");
        assert!(started.elapsed() < Duration::from_secs(120), "{file}");
    }
}

fn png_header(w: u32, h: u32) -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    v.extend(13u32.to_be_bytes());
    v.extend(b"IHDR");
    v.extend(&ihdr);
    v.extend(0u32.to_be_bytes()); // wrong CRC on purpose
    v.extend(0u32.to_be_bytes());
    v.extend(b"IEND");
    v.extend(0u32.to_be_bytes());
    v
}

#[test]
fn images_with_absurd_dimensions_do_not_blow_up() {
    let bomb = png_header(60_000, 60_000);
    // as the cover and as an inline picture of an EPUB, and as a comic page
    let mut book = random_book(3, false);
    let _ = book.set_cover(bomb.clone(), "png");
    let epub = book.archive.to_bytes().unwrap();
    let cbz = {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for i in 0..3 {
            z.start_file(
                format!("p{i}.png"),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            z.write_all(&bomb).unwrap();
        }
        z.finish().unwrap().into_inner()
    };
    for (input, name) in [(epub, "huge-cover.epub"), (cbz, "huge-pages.cbz")] {
        for ext in [
            "epub", "mobi", "azw3", "kepub", "fb2", "docx", "rtf", "snb", "pdb", "rb", "htmlz",
        ] {
            let input = input.clone();
            let label = format!("{name} → {ext}");
            let ext = ext.to_string();
            let name = name.to_string();
            let started = Instant::now();
            run_with_limit(label.clone(), Duration::from_secs(60), move || {
                let _ = convert(
                    &bookx_formats::default_readers(),
                    &input,
                    &name,
                    &ext,
                    &ConvertOptions::default(),
                    &|_| {},
                    &AtomicBool::new(false),
                );
            });
            assert!(started.elapsed() < Duration::from_secs(60), "{label}");
        }
    }
}

#[test]
fn deeply_nested_markup_does_not_overflow_the_stack() {
    let depth = 5000;
    let html = format!(
        "<html><body>{}text{}</body></html>",
        "<div>".repeat(depth),
        "</div>".repeat(depth)
    );
    let xml = format!(
        "<?xml version=\"1.0\"?><FictionBook><body>{}<p>x</p>{}</body></FictionBook>",
        "<section>".repeat(depth),
        "</section>".repeat(depth)
    );
    for (name, bytes) in [
        ("deep.html", html.into_bytes()),
        ("deep.fb2", xml.into_bytes()),
    ] {
        run_with_limit(name.to_string(), Duration::from_secs(60), move || {
            let _ = read_input(&bookx_formats::default_readers(), &bytes, name);
        });
    }
}
