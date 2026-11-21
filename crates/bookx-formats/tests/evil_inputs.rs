//! Hostile files for every reader (docs/security.md, P1.2): zip bombs, path tricks, XML entity
//! bombs, nesting bombs, truncation, lying size fields. Each input must be answered with an error
//! or a book, never a panic, a hang or an unbounded allocation. Deterministic, no network; the
//! same entry point is fuzzed by `fuzz/fuzz_targets/formats.rs`.

mod common;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use bookx_formats::common::{read_zip_limited, zip_entry};
use bookx_formats::{read_any, supported_extensions};
use common::*;

/// Runs `read_any` on a thread and fails (instead of hanging) when it takes too long.
fn run(name: &str, bytes: Vec<u8>) -> bool {
    let (tx, rx) = mpsc::channel();
    let n = name.to_string();
    let len = bytes.len();
    std::thread::spawn(move || {
        let t = Instant::now();
        let ok = read_any(&n, &bytes).is_ok();
        let _ = tx.send((ok, t.elapsed()));
    });
    match rx.recv_timeout(Duration::from_secs(60)) {
        Ok((ok, took)) => {
            assert!(
                took < Duration::from_secs(30),
                "{name} ({len} bytes) took {took:?}"
            );
            ok
        }
        Err(_) => panic!("{name} ({len} bytes) did not finish in 60 s"),
    }
}

struct Rng(u64);
impl Rng {
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n)
            .map(|_| {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                (self.0 >> 24) as u8
            })
            .collect()
    }
}

fn with_prefix(prefix: &[u8], n: usize, seed: u64) -> Vec<u8> {
    let mut v = prefix.to_vec();
    v.extend(Rng(seed).bytes(n));
    v
}

/// Every claimed extension against generic hostile blobs.
#[test]
fn every_reader_survives_garbage_and_magic_prefixes() {
    let blobs: Vec<(&str, Vec<u8>)> = vec![
        ("empty", vec![]),
        ("one", vec![0]),
        ("zeros", vec![0; 4096]),
        ("ones", vec![0xff; 256 * 1024]),
        ("random", Rng(1).bytes(64 * 1024)),
        ("pk", with_prefix(b"PK\x03\x04", 4096, 2)),
        ("pk-eocd", {
            let mut v = b"PK\x05\x06".to_vec();
            v.extend([0xff; 18]);
            v
        }),
        ("pdf", with_prefix(b"%PDF-1.7\n", 8192, 3)),
        ("rtf", with_prefix(b"{\\rtf1", 8192, 4)),
        (
            "xml",
            with_prefix(b"<?xml version=\"1.0\"?><FictionBook>", 8192, 5),
        ),
        ("mobi", {
            let mut v = vec![0u8; 60];
            v.extend(b"BOOKMOBI");
            v.extend(Rng(6).bytes(8192));
            v
        }),
        ("palm", {
            let mut v = vec![0u8; 60];
            v.extend(b"TEXtREAd");
            v.extend(Rng(7).bytes(8192));
            v
        }),
        ("itss", with_prefix(b"ITSF\x03\0\0\0", 8192, 8)),
        ("lit", with_prefix(b"ITOLITLS", 8192, 9)),
        ("rar", with_prefix(b"Rar!\x1a\x07\x00", 8192, 10)),
        ("png", with_prefix(b"\x89PNG\r\n\x1a\n", 8192, 11)),
    ];
    let exts = supported_extensions();
    assert!(exts.len() > 15, "{exts:?}");
    for ext in &exts {
        for (label, blob) in &blobs {
            let name = format!("evil-{label}.{ext}");
            let _ = run(&name, blob.clone());
        }
    }
    // and with no usable extension, where the content decides
    for (label, blob) in &blobs {
        let _ = run(&format!("evil-{label}"), blob.clone());
    }
}

/// Every prefix of a valid file is a "truncated download".
#[test]
fn truncated_files_are_errors_not_panics() {
    let fb2 = br#"<?xml version="1.0"?><FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0"><description><title-info><book-title>T</book-title></title-info></description><body><section><p>Hello</p></section></body></FictionBook>"#.to_vec();
    let docx = make_zip(&[
        ("[Content_Types].xml", b"<Types/>"),
        (
            "word/document.xml",
            br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Hi</w:t></w:r></w:p></w:body></w:document>"#,
        ),
    ]);
    let rtf = br"{\rtf1\ansi{\info{\title T}}\pard Hello\par}".to_vec();
    let mobi = mobi_writer::write_palmdoc("Truncated", &"Hello world. ".repeat(200));
    for (name, full) in [
        ("t.fb2", fb2),
        ("t.docx", docx),
        ("t.rtf", rtf),
        ("t.mobi", mobi),
    ] {
        let step = (full.len() / 120).max(1);
        let mut n = 0;
        while n < full.len() {
            let _ = run(name, full[..n].to_vec());
            n += step;
        }
    }
}

fn nested(open: &str, close: &str, depth: usize, inner: &str) -> String {
    let mut s = String::with_capacity(depth * (open.len() + close.len()) + inner.len());
    for _ in 0..depth {
        s.push_str(open);
    }
    s.push_str(inner);
    for _ in 0..depth {
        s.push_str(close);
    }
    s
}

const LAUGHS: &str = r#"<!DOCTYPE lolz [
 <!ENTITY lol "lol">
 <!ENTITY lol1 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;">
 <!ENTITY lol2 "&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;">
 <!ENTITY lol3 "&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;">
 <!ENTITY lol4 "&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;">
 <!ENTITY lol5 "&lol4;&lol4;&lol4;&lol4;&lol4;&lol4;&lol4;&lol4;&lol4;&lol4;">
 <!ENTITY lol6 "&lol5;&lol5;&lol5;&lol5;&lol5;&lol5;&lol5;&lol5;&lol5;&lol5;">
 <!ENTITY lol7 "&lol6;&lol6;&lol6;&lol6;&lol6;&lol6;&lol6;&lol6;&lol6;&lol6;">
 <!ENTITY lol8 "&lol7;&lol7;&lol7;&lol7;&lol7;&lol7;&lol7;&lol7;&lol7;&lol7;">
 <!ENTITY lol9 "&lol8;&lol8;&lol8;&lol8;&lol8;&lol8;&lol8;&lol8;&lol8;&lol8;">
]>"#;

const XXE: &str = r#"<!DOCTYPE x [<!ENTITY xxe SYSTEM "file:///etc/passwd"><!ENTITY rem SYSTEM "http://169.254.169.254/latest/">]>"#;

/// XML bodies that attack the parsers behind FB2, DOCX, ODT and the OPF.
fn xml_attacks() -> Vec<(&'static str, String)> {
    vec![
        ("billion-laughs", format!("{LAUGHS}<r>&lol9;</r>")),
        ("xxe", format!("{XXE}<r>&xxe;&rem;</r>")),
        ("deep", nested("<a>", "</a>", 200_000, "x")),
        ("deep-ns", nested("<a xmlns:b='u'>", "</a>", 50_000, "x")),
        (
            "many-attrs",
            format!(
                "<r {}/>",
                (0..400_000)
                    .map(|i| format!("a{i}='1'"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        ),
        ("unterminated", "<r><a><b".repeat(10_000)),
        ("huge-text", format!("<r>{}</r>", "A".repeat(8 << 20))),
        (
            "char-refs",
            format!("<r>{}</r>", "&#x110000;&#0;&#xD800;".repeat(50_000)),
        ),
    ]
}

#[test]
fn xml_entity_and_nesting_bombs_in_every_xml_format() {
    for (label, xml) in xml_attacks() {
        let b = xml.as_bytes();
        // FB2: the XML is the file
        let fb2 = format!(
            "<?xml version=\"1.0\"?>{}",
            xml.replace("<r", "<FictionBook")
                .replace("</r>", "</FictionBook>")
        );
        let _ = run(&format!("{label}.fb2"), fb2.into_bytes());
        let _ = run(&format!("{label}.html"), b.to_vec());
        // DOCX / ODT: the XML is the main part
        let docx = make_zip(&[
            ("[Content_Types].xml", b"<Types/>"),
            ("word/document.xml", b),
        ]);
        let _ = run(&format!("{label}.docx"), docx);
        let odt = make_zip(&[
            ("mimetype", b"application/vnd.oasis.opendocument.text"),
            ("content.xml", b),
        ]);
        let _ = run(&format!("{label}.odt"), odt);
        // FBZ: an fb2 inside a zip
        let fbz = make_zip(&[("a.fb2", b)]);
        let _ = run(&format!("{label}.fbz"), fbz);
    }
}

#[test]
fn xml_bombs_in_the_opf_and_container_of_an_epub() {
    for (label, xml) in xml_attacks() {
        let epub = make_zip(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", xml.as_bytes()),
        ]);
        assert!(
            bookx_core::archive::Archive::from_bytes(epub.clone()).is_ok(),
            "{label}: the zip itself is valid"
        );
        let started = Instant::now();
        let arc = bookx_core::archive::Archive::from_bytes(epub).unwrap();
        let _ = bookx_core::book::OpenBook::from_archive(arc);
        assert!(started.elapsed() < Duration::from_secs(30), "{label}");
        // an OPF named by a valid container
        let container = br#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="o.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
        let epub = make_zip(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", container),
            ("o.opf", xml.as_bytes()),
        ]);
        let arc = bookx_core::archive::Archive::from_bytes(epub).unwrap();
        let _ = bookx_core::book::OpenBook::from_archive(arc);
    }
}

#[test]
fn nested_rtf_groups_and_control_word_floods() {
    for (label, rtf) in [
        ("braces", format!("{{\\rtf1{}", "{".repeat(500_000))),
        ("closing", format!("{{\\rtf1{}", "}".repeat(500_000))),
        (
            "balanced",
            format!("{{\\rtf1{}x{}}}", "{".repeat(300_000), "}".repeat(300_000)),
        ),
        ("words", format!("{{\\rtf1 {}}}", "\\b0".repeat(300_000))),
        (
            "fonttbl",
            format!("{{\\rtf1{{\\fonttbl{}}}}}", "{\\f0 x;}".repeat(100_000)),
        ),
        (
            "huge-param",
            "{\\rtf1\\u99999999999999999999 \\li-2147483648 \\fs999999999 x}".to_string(),
        ),
        (
            "pict",
            format!("{{\\rtf1{{\\pict\\pngblip {}}}}}", "ff".repeat(1 << 20)),
        ),
        ("binary", "{\\rtf1\\bin2147483647 xx}".to_string()),
    ] {
        let _ = run(&format!("{label}.rtf"), rtf.into_bytes());
    }
}

#[test]
fn zip_names_and_structure_attacks() {
    let names = [
        "../../../../etc/cron.d/x",
        "/etc/passwd",
        "..\\..\\windows\\x",
        "C:\\x",
        "a/../../b",
        "a\0b.xhtml",
        "./././a",
        "//server/share/x",
        "",
        "a//b///c",
        "\u{202e}fdp.xhtml",
        &"d/".repeat(5000),
    ];
    let long = "n".repeat(60_000);
    for ext in ["fbz", "docx", "odt", "htmlz", "txtz", "cbz", "epub", "pmlz"] {
        let mut files: Vec<(&str, &[u8])> =
            names.iter().map(|n| (*n, b"<a/>".as_slice())).collect();
        files.push((long.as_str(), b"x"));
        files.push(("a.png", b"\x89PNG\r\n\x1a\n"));
        let z = make_zip_stored(&files);
        let _ = run(&format!("names.{ext}"), z);
    }
    // the shared reader never hands `..` entries on
    let z = make_zip(&[("../x", b"a"), ("ok/../../y", b"b"), ("ok.txt", b"c")]);
    let f = read_zip_limited(&z, 1 << 20, 1 << 20).unwrap();
    assert!(f.names().all(|n| !n.split('/').any(|s| s == "..")));
    assert!(f.names().any(|n| n == "ok.txt"));
}

#[test]
fn zip_bombs_and_huge_entry_counts_hit_the_budgets() {
    use std::io::Write;
    // 64 MiB of zeros deflates to about 64 KiB: it has to stop at a 1 MiB entry budget
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    w.start_file("bomb.txt", opts).unwrap();
    let chunk = vec![0u8; 1 << 20];
    for _ in 0..64 {
        w.write_all(&chunk).unwrap();
    }
    let bomb = w.finish().unwrap().into_inner();
    assert!(bomb.len() < 1 << 20, "the bomb is {} bytes", bomb.len());
    let t = Instant::now();
    assert!(
        read_zip_limited(&bomb, 1 << 20, 1 << 30).is_err(),
        "per-entry budget"
    );
    assert!(
        read_zip_limited(&bomb, 1 << 30, 1 << 20).is_err(),
        "total budget"
    );
    assert!(t.elapsed() < Duration::from_secs(20));
    // many members that together exceed the total budget
    let small = make_zip(
        &(0..50)
            .map(|i| (format!("f{i}"), vec![b'x'; 40_000]))
            .collect::<Vec<_>>()
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect::<Vec<_>>(),
    );
    assert!(read_zip_limited(&small, 1 << 20, 1 << 20).is_err());
    // a zip that claims more entries than the limit
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for i in 0..bookx_formats::common::MAX_ENTRIES + 10 {
        w.start_file(format!("{i}"), stored).unwrap();
    }
    let many = w.finish().unwrap().into_inner();
    let t = Instant::now();
    assert!(read_zip_limited(&many, 1 << 20, 1 << 20).is_err());
    assert!(t.elapsed() < Duration::from_secs(20));
    assert!(zip_entry(&bomb, "missing").is_none());
    assert!(zip_entry(&bomb, "bomb.txt").is_some_and(|b| b.len() <= 256 << 20));
}

#[test]
fn lying_zip_directories_are_errors() {
    let good = make_zip_stored(&[("a.txt", b"hello"), ("b.txt", b"world")]);
    // corrupt the end-of-central-directory record: entry counts, directory size and offset
    let eocd = good.windows(4).rposition(|w| w == b"PK\x05\x06").unwrap();
    for (off, len) in [(8usize, 2usize), (10, 2), (12, 4), (16, 4)] {
        for v in [0xffu8, 0x7f, 0x01] {
            let mut z = good.clone();
            for b in &mut z[eocd + off..eocd + off + len] {
                *b = v;
            }
            let _ = run("lying.docx", z.clone());
            let _ = run("lying.cbz", z.clone());
            let _ = bookx_core::archive::Archive::from_bytes(z);
        }
    }
    // an entry whose declared size is 4 GiB but whose data is five bytes
    let mut z = good.clone();
    let lfh = z.windows(4).position(|w| w == b"PK\x03\x04").unwrap();
    z[lfh + 22..lfh + 26].copy_from_slice(&u32::MAX.to_le_bytes());
    let cd = z.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    z[cd + 24..cd + 28].copy_from_slice(&u32::MAX.to_le_bytes());
    let _ = run("lying-size.docx", z.clone());
    let _ = bookx_core::archive::Archive::from_bytes(z);
    // overlapping entries: two central directory records pointing at the same local header
    let mut z = good.clone();
    let cds: Vec<usize> = z
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == b"PK\x01\x02")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(cds.len(), 2);
    let first_off: [u8; 4] = z[cds[0] + 42..cds[0] + 46].try_into().unwrap();
    z[cds[1] + 42..cds[1] + 46].copy_from_slice(&first_off);
    let _ = run("overlap.docx", z.clone());
    let _ = bookx_core::archive::Archive::from_bytes(z);
}

#[test]
fn mobi_headers_that_lie_about_sizes() {
    let valid = mobi_writer::write_palmdoc("Lies", &"Some text for the record. ".repeat(300));
    let mut rng = Rng(99);
    // overwrite each 4-byte window of the first 400 bytes with extreme values
    for at in (0..400.min(valid.len().saturating_sub(4))).step_by(2) {
        for v in [0u32, 1, 0x7fff_ffff, 0xffff_ffff, 0x0100_0000] {
            let mut b = valid.clone();
            b[at..at + 4].copy_from_slice(&v.to_be_bytes());
            let _ = run("lie.mobi", b);
        }
    }
    for _ in 0..200 {
        let mut b = valid.clone();
        for _ in 0..8 {
            let i = (rng.bytes(2)[0] as usize * 7 + rng.bytes(1)[0] as usize) % b.len();
            b[i] = rng.bytes(1)[0];
        }
        let _ = run("flip.azw3", b);
    }
}

#[test]
fn an_image_bomb_inside_a_comic_does_not_allocate() {
    // a PNG header that claims 60000 x 60000 pixels, then nothing
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png.extend(60_000u32.to_be_bytes());
    png.extend(60_000u32.to_be_bytes());
    png.extend(b"\x08\x06\0\0\0\0\0\0\0");
    let z = make_zip(&[("001.png", &png), ("002.png", &png)]);
    let t = Instant::now();
    let _ = run("bomb.cbz", z);
    assert!(t.elapsed() < Duration::from_secs(20));
}

#[test]
fn pdf_structure_attacks() {
    let loops = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[2 0 R]/Count 1>>endobj\ntrailer<</Root 1 0 R/Size 3>>\n%%EOF".to_vec();
    let deep = format!(
        "%PDF-1.4\n1 0 obj{}endobj\ntrailer<</Root 1 0 R>>\n%%EOF",
        "[".repeat(200_000)
    );
    let dict = format!(
        "%PDF-1.4\n1 0 obj{}endobj\ntrailer<</Root 1 0 R>>\n%%EOF",
        "<<".repeat(100_000)
    );
    let xref = b"%PDF-1.4\nxref\n0 99999999999\n0000000000 65535 f \ntrailer<</Size 99999999999/Root 1 0 R>>\nstartxref\n9\n%%EOF".to_vec();
    let stream = b"%PDF-1.4\n1 0 obj<</Length 99999999999>>stream\nabc\nendstream endobj\ntrailer<</Root 1 0 R>>\n%%EOF".to_vec();
    for (label, b) in [
        ("loop", loops),
        ("deep-array", deep.into_bytes()),
        ("deep-dict", dict.into_bytes()),
        ("xref", xref),
        ("stream", stream),
    ] {
        let _ = run(&format!("{label}.pdf"), b);
    }
}
