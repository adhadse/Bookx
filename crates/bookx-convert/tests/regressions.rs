//! Regression tests for bugs the Calibre parity run found (docs/convert-parity.md).

mod common;
mod corpus;

use std::sync::atomic::AtomicBool;

use bookx_convert::inspect::Digest;
use bookx_convert::{ConvertOptions, convert, read_input};

fn conv(input: &[u8], name: &str, ext: &str) -> Vec<u8> {
    convert(
        &bookx_formats::default_readers(),
        input,
        name,
        ext,
        &ConvertOptions::default(),
        &|_| {},
        &AtomicBool::new(false),
    )
    .unwrap_or_else(|e| panic!("{name} → {ext}: {e:#}"))
    .bytes
}

fn read(bytes: &[u8], name: &str) -> (bookx_core::book::OpenBook, Digest) {
    let b = read_input(&bookx_formats::default_readers(), bytes, name).unwrap();
    let d = Digest::of(&b);
    (b, d)
}

fn file(zip: &[u8], name: &str) -> String {
    let a = bookx_core::archive::Archive::from_bytes(zip.to_vec()).unwrap();
    a.get_string(name).unwrap_or_default()
}

#[test]
fn epub3_output_has_no_epub2_attributes_and_declares_svg() {
    let s = corpus::by_name("epub-svg-cover");
    let out = conv(&s.bytes, &s.file, "epub");
    let opf = file(&out, "OEBPS/content.opf");
    assert!(
        !opf.contains("opf:role") && !opf.contains("opf:file-as"),
        "{opf}"
    );
    assert!(
        opf.contains("property=\"role\""),
        "roles are refinements in EPUB 3"
    );
    assert!(
        opf.contains("properties=\"svg\""),
        "svg pages declare the svg property\n{opf}"
    );
    // EPUB 2 keeps the attribute form
    let two = convert(
        &bookx_formats::default_readers(),
        &s.bytes,
        &s.file,
        "epub",
        &ConvertOptions {
            epub_version: Some(bookx_convert::writers::EpubVersion::V2),
            ..Default::default()
        },
        &|_| {},
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(file(&two.bytes, "OEBPS/content.opf").contains("opf:role=\"aut\""));
}

#[test]
fn tag_soup_chapters_become_well_formed_xhtml() {
    let s = corpus::by_name("epub-broken-xhtml");
    let out = conv(&s.bytes, &s.file, "epub");
    let x = file(&out, "OEBPS/messy.xhtml");
    assert!(bookx_convert::markup::is_wellformed_xml(&x), "{x}");
    assert!(x.contains("Second paragraph") && x.contains("bare &amp; ampersand"));
    let (_, d) = read(&out, "x.epub");
    assert!(d.words.contains(&"messy".to_string()));
}

#[test]
fn navigation_points_with_one_target_share_a_play_order() {
    let xml = "<navMap><navPoint id=\"a\" playOrder=\"1\"><navLabel><text>A</text></navLabel><content src=\"c.xhtml\"/></navPoint><navPoint id=\"b\" playOrder=\"2\"><navLabel><text>B</text></navLabel><content src=\"c.xhtml\"/></navPoint><navPoint id=\"c\" playOrder=\"3\"><navLabel><text>C</text></navLabel><content src=\"d.xhtml\"/></navPoint></navMap>";
    let fixed = bookx_core::toc::fix_play_order(xml);
    assert_eq!(fixed.matches("playOrder=\"1\"").count(), 2);
    assert!(fixed.contains("playOrder=\"2\""));
    assert!(!fixed.contains("playOrder=\"3\""));
}

#[test]
fn legacy_encoded_text_is_not_mojibake() {
    let s = corpus::by_name("txt-cp1251");
    let (_, d) = read(&s.bytes, &s.file);
    assert!(
        d.words.contains(&"облонских".to_string()),
        "{:?}",
        &d.words[..6]
    );
    // the same through a writer and back
    let out = conv(&s.bytes, &s.file, "mobi");
    let (_, d) = read(&out, "x.mobi");
    assert!(d.words.contains(&"облонских".to_string()));
}

#[test]
fn untitled_inputs_take_the_file_name_as_title() {
    let s = corpus::by_name("txt-utf8");
    let out = conv(&s.bytes, &s.file, "epub");
    let (b, _) = read(&out, "x.epub");
    assert_eq!(b.metadata().title, "txt-utf8");
    let c = corpus::by_name("cbz");
    let out = conv(&c.bytes, &c.file, "epub");
    assert_eq!(read(&out, "x.epub").0.metadata().title, "cbz");
}

#[test]
fn single_html_keeps_anchors_author_and_splits_chapters() {
    let s = corpus::by_name("html-single");
    let out = conv(&s.bytes, &s.file, "epub");
    let (b, d) = read(&out, "x.epub");
    assert_eq!(b.metadata().authors, vec!["Web Author"]);
    assert_eq!(
        d.broken_links, 0,
        "the #end link must reach the final heading"
    );
    assert!(d.internal_links >= 1);
    assert!(d.toc.len() >= 2, "{:?}", d.toc);
}

#[test]
fn rtf_tables_keep_their_cells_apart() {
    let s = corpus::by_name("rtf-full");
    let (_, d) = read(&s.bytes, &s.file);
    for w in ["one", "two", "three", "four", "second"] {
        assert!(
            d.words.contains(&w.to_string()),
            "word {w} lost: {:?}",
            d.words
        );
    }
    assert!(
        !d.words.iter().any(|w| w.ends_with("cell") && w.len() > 4),
        "{:?}",
        d.words
    );
}

#[test]
fn authors_joined_with_ampersand_are_split() {
    let s = corpus::by_name("epub3-features");
    for ext in ["rb", "rtf", "snb", "pdb"] {
        let out = conv(&s.bytes, &s.file, ext);
        let (b, _) = read(&out, &format!("x.{ext}"));
        assert!(
            b.metadata().authors.len() <= 2
                && !b.metadata().authors.iter().any(|a| a.contains(" & ")),
            "{ext}: {:?}",
            b.metadata().authors
        );
    }
}

#[test]
fn htmlz_round_trip_keeps_chapters_and_contents() {
    let s = corpus::by_name("epub-long-toc");
    let out = conv(&s.bytes, &s.file, "htmlz");
    let (_, d) = read(&out, "x.htmlz");
    assert!(d.toc.len() >= 290, "{} entries", d.toc.len());
}

#[test]
fn nested_headings_become_nested_contents() {
    let s = corpus::by_name("markdown");
    let (_, d) = read(&s.bytes, &s.file);
    assert!(
        d.toc.iter().any(|t| t.0 == 1 && t.1 == "second level"),
        "{:?}",
        d.toc
    );
}

#[test]
fn txtz_keeps_the_cover() {
    let s = corpus::by_name("epub3-features");
    let out = conv(&s.bytes, &s.file, "txtz");
    let (b, _) = read(&out, "x.txtz");
    assert!(b.cover_bytes().is_some());
}

#[test]
fn docx_bookmarks_inside_runs_are_link_targets() {
    // Calibre writes <w:r><w:bookmarkStart/>…</w:r>
    use std::io::Write;
    let doc = "<?xml version=\"1.0\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body><w:p><w:hyperlink w:anchor=\"tgt\"><w:r><w:t>go</w:t></w:r></w:hyperlink></w:p><w:p><w:r><w:bookmarkStart w:id=\"1\" w:name=\"tgt\"/><w:t>target</w:t><w:bookmarkEnd w:id=\"1\"/></w:r></w:p></w:body></w:document>";
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default();
    z.start_file("[Content_Types].xml", o).unwrap();
    z.write_all(b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>").unwrap();
    z.start_file("word/document.xml", o).unwrap();
    z.write_all(doc.as_bytes()).unwrap();
    let bytes = z.finish().unwrap().into_inner();
    let (_, d) = read(&bytes, "bm.docx");
    assert_eq!((d.internal_links, d.broken_links), (1, 0));
}

#[test]
fn markdown_output_escapes_markup_characters() {
    let book = common::book_with(
        &["<p>AT&amp;T a&lt;b x&gt;y *star* _under_ [br] # not a heading</p>"],
        false,
    );
    let epub = book.archive.to_bytes().unwrap();
    let md = conv(&epub, "b.epub", "md");
    let (_, d) = read(&md, "x.md");
    for w in ["at", "a", "b", "x", "y", "star", "under", "br", "heading"] {
        assert!(
            d.words.contains(&w.to_string()),
            "{w} lost from {:?}",
            d.words
        );
    }
}

#[test]
fn oversized_declared_zip_members_are_refused() {
    // a member that lies about its size must not make us allocate the lie
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    use std::io::Write;
    z.start_file("a.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    z.write_all(b"hello").unwrap();
    let mut bytes = z.finish().unwrap().into_inner();
    // patch the uncompressed size in the central directory to ~4 GB
    if let Some(p) = bytes.windows(4).rposition(|w| w == b"PK\x01\x02") {
        bytes[p + 24..p + 28].copy_from_slice(&0xF000_0000u32.to_le_bytes());
    }
    let _ = bookx_core::archive::Archive::from_bytes(bytes); // no abort, error or ok
}
