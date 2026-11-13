//! Test support shared by unit and integration tests. Validation here is deliberately independent
//! of the converters: it reopens the produced bytes with the `zip` and `roxmltree` crates.
#![allow(dead_code)]

pub mod fmt_writers;
pub mod mobi_writer;

use std::io::{Cursor, Read, Write};

use bookx_core::archive::Archive;
use bookx_core::book::OpenBook;
use bookx_core::contracts::FormatReader;

pub const PNG: &[u8] =
    b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89";
pub const JPG: &[u8] = b"\xFF\xD8\xFF\xE0\0\x10JFIF\0\x01\x01\0\0\x01\0\x01\0\0\xFF\xD9";

/// Builds a zip file from `(name, bytes)` pairs (deflated).
pub fn make_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut out);
        for (n, d) in files {
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            z.start_file(*n, opts).unwrap();
            z.write_all(d).unwrap();
        }
        z.finish().unwrap();
    }
    out.into_inner()
}

/// Like [`make_zip`] but the entries are stored uncompressed (the ODT `mimetype` convention).
pub fn make_zip_stored(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut out);
        for (n, d) in files {
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            z.start_file(*n, opts).unwrap();
            z.write_all(d).unwrap();
        }
        z.finish().unwrap();
    }
    out.into_inner()
}

/// Validates the EPUB produced by a reader without using the converter's own code and returns
/// the book opened through `bookx-core`.
pub fn validate(arc: &Archive) -> OpenBook {
    let bytes = arc.to_bytes().expect("archive serialises");
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes.clone())).expect("valid zip");
    {
        let first = zip.by_index(0).unwrap();
        assert_eq!(first.name().unwrap(), "mimetype", "mimetype must be first");
        assert_eq!(first.compression(), zip::CompressionMethod::Stored);
    }
    let mut mt = String::new();
    zip.by_name("mimetype")
        .unwrap()
        .read_to_string(&mut mt)
        .unwrap();
    assert_eq!(mt, "application/epub+zip");
    let mut container = String::new();
    zip.by_name("META-INF/container.xml")
        .unwrap()
        .read_to_string(&mut container)
        .unwrap();
    let cdoc = roxmltree::Document::parse(&container).expect("container.xml is XML");
    let opf_path = cdoc
        .descendants()
        .find(|n| n.has_tag_name("rootfile"))
        .and_then(|n| n.attribute("full-path"))
        .expect("rootfile")
        .to_string();
    let mut opf = String::new();
    zip.by_name(&opf_path)
        .unwrap()
        .read_to_string(&mut opf)
        .unwrap();
    let odoc = roxmltree::Document::parse(&opf).expect("OPF is XML");
    let base = opf_path.rsplit_once('/').map(|x| x.0).unwrap_or("");
    let mut ids = std::collections::HashMap::new();
    for item in odoc.descendants().filter(|n| n.has_tag_name("item")) {
        let href = item.attribute("href").unwrap();
        let path = if base.is_empty() {
            href.to_string()
        } else {
            format!("{base}/{href}")
        };
        assert!(
            zip.by_name(&path).is_ok(),
            "manifest item {path} missing from zip"
        );
        ids.insert(item.attribute("id").unwrap().to_string(), path.clone());
        if path.ends_with(".xhtml") {
            let mut s = String::new();
            zip.by_name(&path).unwrap().read_to_string(&mut s).unwrap();
            roxmltree::Document::parse_with_options(
                &s,
                roxmltree::ParsingOptions {
                    allow_dtd: true,
                    ..Default::default()
                },
            )
            .unwrap_or_else(|e| panic!("{path} is not well-formed XML: {e}\n{s}"));
        }
    }
    for ir in odoc.descendants().filter(|n| n.has_tag_name("itemref")) {
        assert!(ids.contains_key(ir.attribute("idref").unwrap()));
    }
    OpenBook::from_archive(Archive::from_bytes(bytes).unwrap()).expect("OpenBook opens it")
}

/// Visible text of every spine document, one string per chapter.
pub fn chapter_texts(book: &OpenBook) -> Vec<String> {
    book.spine_paths()
        .iter()
        .map(|p| {
            let src = book.chapter_source(p).unwrap();
            let doc = roxmltree::Document::parse_with_options(
                &src,
                roxmltree::ParsingOptions {
                    allow_dtd: true,
                    ..Default::default()
                },
            )
            .unwrap();
            let body = doc.descendants().find(|n| n.has_tag_name("body")).unwrap();
            body.descendants()
                .filter(|n| n.is_text())
                .map(|n| n.text().unwrap())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

pub fn chapter_xml(book: &OpenBook, index: usize) -> String {
    book.chapter_source(&book.spine_paths()[index]).unwrap()
}

pub fn toc_titles(book: &OpenBook) -> Vec<String> {
    bookx_core::toc::flatten(&book.toc())
        .into_iter()
        .map(|t| t.title)
        .collect()
}

/// Small deterministic PRNG (xorshift64*).
pub struct Prng(pub u64);

impl Prng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Feeds 200 truncated or mutated variants of `sample` to the reader and asserts it never panics.
/// Returns how many variants were rejected with an error.
pub fn fuzz_no_panic(reader: &dyn FormatReader, sample: &[u8]) -> usize {
    let mut rng = Prng(0x9E3779B97F4A7C15 ^ sample.len() as u64);
    let mut errors = 0;
    for i in 0..200 {
        let mut v = sample.to_vec();
        match i % 4 {
            0 => v.truncate(rng.below(v.len() + 1)),
            1 => {
                for _ in 0..1 + rng.below(8) {
                    if !v.is_empty() {
                        let at = rng.below(v.len());
                        v[at] = rng.next() as u8;
                    }
                }
            }
            2 => {
                if !v.is_empty() {
                    let at = rng.below(v.len());
                    let len = rng.below(64).min(v.len() - at);
                    v.drain(at..at + len);
                }
            }
            _ => {
                let at = rng.below(v.len() + 1);
                let junk: Vec<u8> = (0..rng.below(32)).map(|_| rng.next() as u8).collect();
                v.splice(at..at, junk);
                v.truncate(rng.below(v.len() + 1).max(v.len() / 2));
            }
        }
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = reader.sniff(&v);
            reader.read(&v)
        }));
        match r {
            Err(_) => panic!("{} panicked on variant {i}", reader.id()),
            Ok(Err(_)) => errors += 1,
            Ok(Ok(arc)) => {
                // Whatever is returned must still be a loadable book.
                let _ = arc.to_bytes().expect("archive serialises");
            }
        }
    }
    errors
}

/// Asserts the reader rejects degenerate input with an error (never a panic).
pub fn assert_rejects_garbage(reader: &dyn FormatReader) {
    let huge_noise: Vec<u8> = (0..1_000_000u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
        .collect();
    for input in [
        &b""[..],
        &b"\0"[..],
        &b"   \n"[..],
        &b"PK\x03\x04"[..],
        &huge_noise[..],
    ] {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| reader.read(input)));
        assert!(r.is_ok(), "{} panicked", reader.id());
    }
    assert!(
        reader.read(b"").is_err(),
        "{} accepted empty input",
        reader.id()
    );
}
