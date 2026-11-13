//! Fixtures shared by the integration tests (all generated in code).

#![allow(dead_code)]

use std::io::Cursor;
use std::path::PathBuf;

use bookx_core::archive::Archive;
use bookx_core::book::OpenBook;
use bookx_core::convert::{create_epub, Chapter};
use bookx_core::opf::{Identifier, Metadata};

pub fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([(x * 7 % 256) as u8, (y * 5 % 256) as u8, 120])
    });
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("png");
    out.into_inner()
}

pub fn jpeg(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([(x % 256) as u8, (y % 256) as u8, 40])
    });
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut out, image::ImageFormat::Jpeg)
        .expect("jpeg");
    out.into_inner()
}

pub fn metadata() -> Metadata {
    Metadata {
        title: "The Test Book: A Story".into(),
        authors: vec!["Ada Lovelace".into(), "Grace Hopper".into()],
        publisher: "Bookx Press".into(),
        language: "en".into(),
        description: "A <b>short</b> description & more.".into(),
        tags: vec!["testing".into(), "fiction".into()],
        series: "Saga".into(),
        series_index: 2.0,
        identifiers: vec![Identifier {
            scheme: "isbn".into(),
            value: "9781234567897".into(),
            xml_id: None,
        }],
        published: "2020-05-17".into(),
        ..Default::default()
    }
}

/// A small book: three chapters, an inline image, internal links and a cover.
pub fn sample_book() -> OpenBook {
    let chapters = vec![
        Chapter {
            title: "Chapter One".into(),
            body: "<h1 id=\"c1\">Chapter One</h1>\n<p>It was a \"dark\" night &amp; the rain fell. Another sentence follows here. See <a href=\"chapter-2.xhtml#c2\">chapter two</a>.</p>\n<p>Second <i>italic</i> and <b>bold</b> paragraph.</p>\n".into(),
        },
        Chapter {
            title: "Chapter Two".into(),
            body: "<h1 id=\"c2\">Chapter Two</h1>\n<p>Naïve café — 日本語. <img src=\"images/pic.png\" alt=\"a picture\"/></p>\n<p>Back to <a href=\"chapter-1.xhtml\">one</a> or <a href=\"https://example.org/x\">outside</a>.</p>\n".into(),
        },
        Chapter {
            title: "Chapter Three".into(),
            body: "<h1>Chapter Three</h1>\n<h2 id=\"c3a\">A section</h2>\n<p>The end of the story.</p>\n<pre>code  line</pre>\n".into(),
        },
    ];
    let cover = jpeg(300, 450);
    let mut arc: Archive = create_epub(&metadata(), &chapters, Some((cover, "jpg")));
    arc.set("OEBPS/images/pic.png", png(40, 30));
    let bytes = arc.to_bytes().expect("zip");
    let mut book = OpenBook::from_archive(Archive::from_bytes(bytes).expect("zip")).expect("epub");
    let pkg = book.package.clone();
    pkg.add_manifest_item(
        &mut book.archive,
        "pic",
        "OEBPS/images/pic.png",
        "image/png",
        "",
    )
    .expect("manifest");
    book.reload_package().expect("reload");
    book.set_metadata(metadata()).expect("metadata");
    book
}

/// Same book, but from a nested table of contents.
pub fn nested_toc_book() -> OpenBook {
    let mut book = sample_book();
    let pkg = book.package.clone();
    let toc = vec![
        bookx_core::toc::TocEntry {
            title: "Part I".into(),
            href: "OEBPS/chapter-1.xhtml#c1".into(),
            children: vec![bookx_core::toc::TocEntry {
                title: "Chapter Two".into(),
                href: "OEBPS/chapter-2.xhtml#c2".into(),
                children: vec![],
            }],
        },
        bookx_core::toc::TocEntry {
            title: "Part II".into(),
            href: "OEBPS/chapter-3.xhtml".into(),
            children: vec![bookx_core::toc::TocEntry {
                title: "A section".into(),
                href: "OEBPS/chapter-3.xhtml#c3a".into(),
                children: vec![],
            }],
        },
    ];
    bookx_core::toc::write_toc(&mut book.archive, &pkg, &toc).expect("toc");
    book
}

pub fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bookx-convert-{tag}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

/// Text of an XHTML document with all markup removed (whitespace collapsed).
pub fn xml_text(xml: &str) -> String {
    let doc = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .expect("well-formed xml");
    let body = doc
        .descendants()
        .find(|n| n.has_tag_name("body"))
        .unwrap_or_else(|| doc.root_element());
    body.descendants()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A book of the given XHTML bodies (`chapter-N.xhtml`), with `pic.png` (40x30), `pic.jpg` (64x48)
/// and `anim.gif` (16x16) next to the chapters, and optionally a JPEG cover.
pub fn book_with(bodies: &[&str], cover: bool) -> OpenBook {
    let chapters: Vec<Chapter> = bodies
        .iter()
        .enumerate()
        .map(|(i, b)| Chapter {
            title: format!("Chapter {}", i + 1),
            body: (*b).to_string(),
        })
        .collect();
    let cover = cover.then(|| (jpeg(200, 300), "jpg"));
    let mut arc: Archive = create_epub(&metadata(), &chapters, cover);
    arc.set("OEBPS/pic.png", png(40, 30));
    arc.set("OEBPS/pic.jpg", jpeg(64, 48));
    arc.set("OEBPS/anim.gif", gif(16, 16));
    OpenBook::from_archive(Archive::from_bytes(arc.to_bytes().expect("zip")).expect("zip"))
        .expect("epub")
}

pub fn gif(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 200, 30, 255]));
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut out, image::ImageFormat::Gif)
        .expect("gif");
    out.into_inner()
}

/// All files of a zip, in order.
pub fn unzip(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    use std::io::Read;
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).expect("a zip file");
    (0..z.len())
        .map(|i| {
            let mut f = z.by_index(i).expect("entry");
            let mut data = Vec::new();
            f.read_to_end(&mut data).expect("read");
            (f.name().expect("name").to_string(), data)
        })
        .collect()
}

pub fn part<'a>(files: &'a [(String, Vec<u8>)], name: &str) -> &'a [u8] {
    files
        .iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| {
            panic!(
                "missing part {name}; have {:?}",
                files.iter().map(|f| &f.0).collect::<Vec<_>>()
            )
        })
        .1
        .as_slice()
}

pub fn part_str(files: &[(String, Vec<u8>)], name: &str) -> String {
    String::from_utf8(part(files, name).to_vec()).expect("utf-8 part")
}

/// Whitespace-normalised text of every spine document of an EPUB (any bytes), through the core
/// HTML reader (independent of the writers' own model).
pub fn epub_text(archive: Archive) -> String {
    let book = OpenBook::from_archive(archive).expect("epub");
    let mut parts = Vec::new();
    for p in book.spine_paths() {
        if let Some(d) = book.chapter_doc(&p) {
            parts.push(
                d.blocks
                    .iter()
                    .map(|b| b.plain_text())
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
    }
    parts
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The sentences every conversion of [`sample_book`] must still contain.
pub const SENTENCES: &[&str] = &[
    "It was a \"dark\" night & the rain fell.",
    "Another sentence follows here.",
    "Second italic and bold paragraph.",
    "Naïve café",
    "Back to one or outside.",
    "The end of the story.",
    "code line",
];

/// True when the Calibre oracle may be used (`ebook-convert` installed, `BOOKX_NO_ORACLE` unset).
pub fn oracle_available() -> bool {
    if std::env::var_os("BOOKX_NO_ORACLE").is_some() {
        return false;
    }
    std::process::Command::new("ebook-convert")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Runs `ebook-convert` on `input` (written as `in_name`) producing `out_name`; `None` when the
/// oracle is unavailable. Panics (with Calibre's output) when Calibre fails.
pub fn calibre(input: &[u8], in_name: &str, out_name: &str, extra: &[&str]) -> Option<Vec<u8>> {
    if !oracle_available() {
        return None;
    }
    let dir = temp_dir("oracle");
    let inp = dir.join(in_name);
    let out = dir.join(out_name);
    std::fs::write(&inp, input).expect("write input");
    let res = std::process::Command::new("ebook-convert")
        .arg(&inp)
        .arg(&out)
        .args(extra)
        .output()
        .expect("run ebook-convert");
    if !res.status.success() {
        let msg = format!(
            "ebook-convert {in_name} -> {out_name} failed:\n{}\n{}",
            String::from_utf8_lossy(&res.stdout),
            String::from_utf8_lossy(&res.stderr)
        );
        let _ = std::fs::remove_dir_all(&dir);
        panic!("{msg}");
    }
    let data = std::fs::read(&out).expect("calibre output");
    let _ = std::fs::remove_dir_all(&dir);
    Some(data)
}

/// Text of a file converted to EPUB by Calibre.
pub fn calibre_text(input: &[u8], in_name: &str) -> Option<String> {
    let epub = calibre(input, in_name, "back.epub", &[])?;
    Some(epub_text(Archive::from_bytes(epub).expect("calibre epub")))
}

/// Asserts that every sentence of [`SENTENCES`] occurs in `text`.
pub fn assert_sentences(text: &str, what: &str) {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    for s in SENTENCES {
        assert!(text.contains(s), "{what}: missing `{s}` in:\n{text}");
    }
}
