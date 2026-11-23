//! End to end tests of the public API: every format is converted through `read_any`, with the
//! right and with a wrong or missing file extension, and the result is validated independently.

mod common;

use bookx_formats::{default_readers, read_any, supported_extensions};
use common::fmt_writers::*;
use common::mobi_writer::*;
use common::*;

fn fb2() -> Vec<u8> {
    br#"<?xml version="1.0" encoding="utf-8"?><FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0"><description><title-info><genre>prose</genre><author><first-name>Fay</first-name><last-name>Bee</last-name></author><book-title>FB2 Book</book-title><lang>en</lang></title-info></description><body><section><title><p>One</p></title><p>Hello fb2.</p></section></body></FictionBook>"#.to_vec()
}

fn rtf() -> Vec<u8> {
    br"{\rtf1\ansi{\info{\title RTF Book}{\author Rita}}\pard Hello rtf.\par}".to_vec()
}

fn docx() -> Vec<u8> {
    let doc = br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Hello docx.</w:t></w:r></w:p></w:body></w:document>"#;
    let core = br#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>DOCX Book</dc:title><dc:creator>Dan</dc:creator></cp:coreProperties>"#;
    make_zip(&[
        ("[Content_Types].xml", b"<Types/>"),
        ("word/document.xml", doc),
        ("docProps/core.xml", core),
    ])
}

fn odt() -> Vec<u8> {
    let ns = "xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"";
    let content = format!(
        "<office:document-content {ns}><office:body><office:text><text:p>Hello odt.</text:p></office:text></office:body></office:document-content>"
    );
    let meta = format!(
        "<office:document-meta {ns}><office:meta><dc:title>ODT Book</dc:title><dc:creator>Odin</dc:creator></office:meta></office:document-meta>"
    );
    make_zip_stored(&[
        ("mimetype", b"application/vnd.oasis.opendocument.text"),
        ("content.xml", content.as_bytes()),
        ("meta.xml", meta.as_bytes()),
    ])
}

fn cbz() -> Vec<u8> {
    let mut a = JPG.to_vec();
    a.push(1);
    let mut b = PNG.to_vec();
    b.push(2);
    make_zip(&[
        ("ComicInfo.xml", b"<ComicInfo><Title>CBZ Book</Title><Writer>Cy</Writer><Manga>YesAndRightToLeft</Manga></ComicInfo>"),
        ("p2.png", &b),
        ("p1.jpg", &a),
    ])
}

fn mobi() -> Vec<u8> {
    write_mobi(&MobiSpec {
        title: "MOBI Book".into(),
        authors: vec!["Mo".into()],
        html: "<html><body><h1>One</h1><p>Hello mobi.</p></body></html>".into(),
        ..Default::default()
    })
}

fn htmlz() -> Vec<u8> {
    make_zip(&[(
        "index.html",
        b"<html><head><title>HTMLZ Book</title></head><body><p>Hello htmlz.</p></body></html>",
    )])
}

fn txtz() -> Vec<u8> {
    make_zip(&[("index.txt", b"Chapter 1\n\nHello txtz.\n"), ("metadata.opf", b"<package><metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>TXTZ Book</dc:title></metadata></package>")])
}

fn pdb() -> Vec<u8> {
    write_ereader(&EReaderSpec {
        version: 2,
        title: "PDB Book".into(),
        author: "Pat".into(),
        text: "\\x\\cOne\\c\\x\nHello pdb.\n".into(),
        ..Default::default()
    })
}

fn pml() -> Vec<u8> {
    b"\\vTITLE=\"PML Book\"\\v\n\\xOne\\x\nHello pml.\n".to_vec()
}

fn pmlz() -> Vec<u8> {
    make_zip(&[
        (
            "b.pml",
            b"\\vTITLE=\"PMLZ Book\"\\v\n\\xOne\\x\nHello pmlz.\n",
        ),
        ("b_img/x.png", PNG),
    ])
}

fn rb() -> Vec<u8> {
    write_rb(&RbSpec {
        info: "TITLE=RB Book\nAUTHOR=Rob\n".into(),
        pages: vec![(
            "1.html".into(),
            b"<html><body><h1>One</h1><p>Hello rb.</p></body></html>".to_vec(),
            true,
        )],
        ..Default::default()
    })
}

fn snb() -> Vec<u8> {
    write_snb(&[
        (
            "book.snbf",
            b"<book-snbf><head><name>SNB Book</name><author>Sue</author></head></book-snbf>"
                .to_vec(),
            true,
        ),
        (
            "toc.snbf",
            b"<toc-snbf><body><chapter src=\"c.snbc\">One</chapter></body></toc-snbf>".to_vec(),
            true,
        ),
        (
            "c.snbc",
            b"<snbc><body><text>Hello snb.</text></body></snbc>".to_vec(),
            true,
        ),
    ])
}

fn tcr() -> Vec<u8> {
    tcr_compress(b"Chapter 1\n\nHello tcr. Hello tcr.\n")
}

fn chm() -> Vec<u8> {
    write_chm(&ChmSpec {
        title: "CHM Book".into(),
        files: vec![(
            "/a.htm".into(),
            b"<html><body><h1>One</h1><p>Hello chm.</p></body></html>".to_vec(),
        )],
        compressed: true,
        ..Default::default()
    })
}

fn cbc() -> Vec<u8> {
    make_zip(&[
        ("comics.txt", b"one.cbz:Issue One\n"),
        ("one.cbz", &cbz()),
        ("metadata.opf", br#"<package><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>CBC Book</dc:title></metadata></package>"#),
    ])
}

fn ztxt() -> Vec<u8> {
    write_ztxt("zTXT Book", "Hello ztxt.\n\nSecond paragraph.\n", false)
}

/// Samples of the second wave of readers: `(file name, bytes, title, text)`.
fn samples2() -> Vec<(&'static str, Vec<u8>, &'static str, &'static str)> {
    vec![
        ("a.pdb", pdb(), "PDB Book", "Hello pdb."),
        ("a.pdb", ztxt(), "zTXT Book", "Hello ztxt."),
        ("a.pml", pml(), "PML Book", "Hello pml."),
        ("a.pmlz", pmlz(), "PMLZ Book", "Hello pmlz."),
        ("a.rb", rb(), "RB Book", "Hello rb."),
        ("a.snb", snb(), "SNB Book", "Hello snb."),
        ("a.tcr", tcr(), "a", "Hello tcr."),
        ("a.chm", chm(), "CHM Book", "Hello chm."),
        ("a.cbc", cbc(), "CBC Book", ""),
    ]
}

/// `(file name, bytes, id of the reader that must win by content)` for every sample whose bytes
/// identify the format.
fn identifiable() -> Vec<(&'static str, Vec<u8>, &'static str)> {
    let mut v: Vec<(&'static str, Vec<u8>, &'static str)> = vec![
        ("a.fb2", fb2(), "fb2"),
        ("a.fbz", make_zip(&[("a.fb2", &fb2())]), "fbz"),
        ("a.rtf", rtf(), "rtf"),
        ("a.docx", docx(), "docx"),
        ("a.odt", odt(), "odt"),
        ("a.htmlz", htmlz(), "htmlz"),
        ("a.txtz", txtz(), "txtz"),
        ("a.mobi", mobi(), "mobi"),
        (
            "a.prc",
            write_palmdoc("Doc", "Plain palmdoc text.\n"),
            "mobi",
        ),
        ("a.cbz", cbz(), "cbz"),
    ];
    v.extend([
        ("a.pdb", pdb(), "pdb"),
        ("a.pdb", ztxt(), "pdb"),
        ("a.pmlz", pmlz(), "pmlz"),
        ("a.rb", rb(), "rb"),
        ("a.snb", snb(), "snb"),
        ("a.tcr", tcr(), "tcr"),
        ("a.chm", chm(), "chm"),
        ("a.lit", write_lit(false), "lit"),
        ("a.cbc", cbc(), "cbc"),
    ]);
    v
}

/// `(file name, bytes, expected title, expected text)`
fn samples() -> Vec<(&'static str, Vec<u8>, &'static str, &'static str)> {
    vec![
        ("a.fb2", fb2(), "FB2 Book", "Hello fb2."),
        (
            "a.fbz",
            make_zip(&[("a.fb2", &fb2())]),
            "FB2 Book",
            "Hello fb2.",
        ),
        ("a.rtf", rtf(), "RTF Book", "Hello rtf."),
        ("a.docx", docx(), "DOCX Book", "Hello docx."),
        ("a.odt", odt(), "ODT Book", "Hello odt."),
        ("a.htmlz", htmlz(), "HTMLZ Book", "Hello htmlz."),
        ("a.txtz", txtz(), "TXTZ Book", "Hello txtz."),
        ("a.mobi", mobi(), "MOBI Book", "Hello mobi."),
        ("a.azw3", mobi(), "MOBI Book", "Hello mobi."),
        ("a.prc", mobi(), "MOBI Book", "Hello mobi."),
        ("a.azw", mobi(), "MOBI Book", "Hello mobi."),
        (
            "a.html",
            b"<html><head><title>HTML Book</title></head><body><p>Hello html.</p></body></html>"
                .to_vec(),
            "HTML Book",
            "Hello html.",
        ),
        ("a.pdb", pdb(), "PDB Book", "Hello pdb."),
        ("a.pml", pml(), "PML Book", "Hello pml."),
        ("a.pmlz", pmlz(), "PMLZ Book", "Hello pmlz."),
        ("a.rb", rb(), "RB Book", "Hello rb."),
        ("a.snb", snb(), "SNB Book", "Hello snb."),
        ("a.tcr", tcr(), "a", "Hello tcr."),
        ("a.chm", chm(), "CHM Book", "Hello chm."),
        ("a.cbc", cbc(), "CBC Book", ""),
        (
            "Plain Title.txt",
            b"Hello txt.\n".to_vec(),
            "Plain Title",
            "Hello txt.",
        ),
        (
            "a.md",
            b"---\ntitle: MD Book\n---\n# H\n\nHello md.\n".to_vec(),
            "MD Book",
            "Hello md.",
        ),
    ]
}

#[test]
fn every_format_converts_by_extension() {
    for (name, bytes, title, text) in samples() {
        let arc = read_any(name, &bytes).unwrap_or_else(|e| panic!("{name}: {e:#}"));
        let book = validate(&arc);
        assert_eq!(book.metadata().title, title, "{name}");
        assert!(chapter_texts(&book).join(" ").contains(text), "{name}");
        assert!(!book.spine_paths().is_empty(), "{name}");
        assert!(!book.toc().is_empty(), "{name}: toc");
    }
}

#[test]
fn comics_convert_with_cover_pages_and_direction() {
    let arc = read_any("x.cbz", &cbz()).unwrap();
    let book = validate(&arc);
    assert_eq!(book.metadata().title, "CBZ Book");
    assert_eq!(book.metadata().authors, vec!["Cy"]);
    assert_eq!(book.spine_paths().len(), 2);
    assert!(book.cover_bytes().is_some());
    assert!(
        arc.get_string("OEBPS/content.opf")
            .unwrap()
            .contains("page-progression-direction=\"rtl\"")
    );
}

#[test]
fn content_sniffing_works_with_missing_or_wrong_extensions() {
    for name_for in ["book", "book.bin", "book.xml", "book.dat", "BOOK.BIN"] {
        for (orig, bytes, title, _) in samples() {
            // Markdown and plain text have no distinguishing content; HTML is only sniffed
            // when it looks like HTML.
            if orig.ends_with(".md") || orig.ends_with(".txt") || orig.ends_with(".pml") {
                continue;
            }
            // TCR has no title of its own: it comes from the file name.
            if orig.ends_with(".tcr") {
                let arc = read_any(name_for, &bytes).unwrap();
                assert!(
                    chapter_texts(&validate(&arc))
                        .join(" ")
                        .contains("Hello tcr.")
                );
                continue;
            }
            let arc = read_any(name_for, &bytes)
                .unwrap_or_else(|e| panic!("{name_for} <- {orig}: {e:#}"));
            assert_eq!(
                validate(&arc).metadata().title,
                title,
                "{name_for} <- {orig}"
            );
        }
        let c = read_any(name_for, &cbz()).unwrap();
        assert_eq!(validate(&c).metadata().title, "CBZ Book");
        // plain text falls through to the TXT reader
        let t = read_any(name_for, b"Just some words.\n\nAnd more.\n").unwrap();
        assert!(chapter_texts(&validate(&t)).join(" ").contains("And more."));
    }
}

#[test]
fn extension_takes_precedence_but_unknown_extensions_fall_back_to_content() {
    // A file called .txt is text even if the bytes could be something else.
    let arc = read_any("looks.txt", &fb2()).unwrap();
    assert!(
        chapter_texts(&validate(&arc))
            .join(" ")
            .contains("Hello fb2.")
    );
    assert_eq!(validate(&arc).metadata().title, "looks");
    // Totally unknown binary content has no reader.
    let err = read_any("x.zzz", &[0u8, 1, 2, 3, 0, 0, 0, 0, 9, 9])
        .unwrap_err()
        .to_string();
    assert!(err.contains("no reader"), "{err}");
    // An EPUB is not an input format of this crate.
    let epub = bookx_core::convert::create_epub(&bookx_core::opf::Metadata::default(), &[], None);
    assert!(read_any("a.epub", &epub.to_bytes().unwrap()).is_err());
}

#[test]
fn errors_are_contextual() {
    let err = read_any("broken.mobi", b"\0\0\0").unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("broken.mobi") && msg.contains("mobi"), "{msg}");
    let err = read_any("empty.txt", b"").unwrap_err();
    assert!(format!("{err:#}").contains("empty"));
}

#[test]
fn registry_is_consistent() {
    let readers = default_readers();
    let mut ids = Vec::new();
    let mut exts = Vec::new();
    for r in readers.iter() {
        ids.push(r.id());
        for e in r.extensions() {
            assert_eq!(*e, e.to_ascii_lowercase());
            assert!(!e.starts_with('.'));
            exts.push(*e);
        }
    }
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), n, "reader ids are unique");
    let n = exts.len();
    exts.sort_unstable();
    exts.dedup();
    assert_eq!(exts.len(), n, "no extension is claimed twice");
    assert_eq!(
        ids,
        vec![
            "azw4", "cbc", "cbr", "cbt", "cbz", "chm", "djvu", "docx", "fb2", "fbz", "html",
            "htmlz", "lit", "lrf", "md", "mobi", "odt", "pdb", "pdf", "pml", "pmlz", "rb", "rtf",
            "snb", "tcr", "txt", "txtz"
        ]
    );
    let all = supported_extensions();
    for e in [
        "fb2", "fbz", "htmlz", "txtz", "txt", "md", "html", "rtf", "docx", "odt", "cbz", "cbt",
        "cbr", "mobi", "azw", "azw3", "prc", "pdb", "pml", "pmlz", "rb", "snb", "tcr", "cbc",
        "chm", "lit", "lrf", "pdf", "azw4", "djvu", "djv",
    ] {
        assert!(all.contains(&e), "{e}");
    }
    assert!(all.windows(2).all(|w| w[0] < w[1]), "sorted and unique");
}

#[test]
fn no_reader_panics_on_any_input_of_any_other_reader() {
    let inputs: Vec<Vec<u8>> = samples()
        .into_iter()
        .map(|s| s.1)
        .chain([
            cbz(),
            vec![],
            vec![0xFF; 100],
            b"PK\x03\x04".to_vec(),
            b"{\\rtf".to_vec(),
        ])
        .collect();
    let readers = default_readers();
    for r in readers.iter() {
        for input in &inputs {
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = r.sniff(input);
                r.read(input)
            }));
            assert!(res.is_ok(), "{} panicked", r.id());
        }
    }
}

#[test]
fn every_reader_survives_200_mutations_of_its_sample() {
    let readers = default_readers();
    for (name, bytes, _, _) in samples().into_iter().chain([("a.cbz", cbz(), "", "")]) {
        let ext = name.rsplit('.').next().unwrap();
        let r = readers
            .iter()
            .find(|r| r.extensions().contains(&ext))
            .unwrap_or_else(|| panic!("no reader for {ext}"));
        fuzz_no_panic(r, &bytes);
    }
}

#[test]
fn truncations_of_valid_files_never_panic() {
    for (name, bytes, _, _) in samples() {
        for cut in (0..bytes.len()).step_by((bytes.len() / 60).max(1)) {
            let r = std::panic::catch_unwind(|| read_any(name, &bytes[..cut]));
            assert!(r.is_ok(), "{name} cut at {cut} panicked");
        }
    }
}

#[test]
fn large_text_is_converted_quickly_enough() {
    let line = "The quick brown fox jumps over the lazy dog, again and again. ";
    let para = line.repeat(8);
    let mut text = String::new();
    for i in 0..400 {
        text.push_str(&format!("Chapter {}\n\n", i + 1));
        for _ in 0..12 {
            text.push_str(&para);
            text.push_str("\n\n");
        }
    }
    assert!(text.len() > 2_000_000);
    let t = std::time::Instant::now();
    let arc = read_any("big.txt", text.as_bytes()).unwrap();
    let book = validate(&arc);
    assert_eq!(book.spine_paths().len(), 400);
    assert!(t.elapsed().as_secs() < 60, "took {:?}", t.elapsed());
}

#[test]
fn many_chapter_fb2_is_fast_enough() {
    let mut x = String::from(
        "<FictionBook><description><title-info><book-title>Big</book-title></title-info></description><body>",
    );
    for i in 0..1500 {
        x.push_str(&format!(
            "<section><title><p>Ch {i}</p></title><p>Some text for chapter {i}.</p></section>"
        ));
    }
    x.push_str("</body></FictionBook>");
    let t = std::time::Instant::now();
    let arc = read_any("big.fb2", x.as_bytes()).unwrap();
    assert_eq!(validate(&arc).spine_paths().len(), 1500);
    assert!(t.elapsed().as_secs() < 60, "took {:?}", t.elapsed());
}

#[test]
fn formats_are_independent_of_each_other_and_deterministic_in_content() {
    // Converting twice yields the same chapters (only the book uuid and timestamp differ).
    for (name, bytes, _, _) in samples() {
        let a = read_any(name, &bytes).unwrap();
        let b = read_any(name, &bytes).unwrap();
        let (a, b) = (validate(&a), validate(&b));
        assert_eq!(chapter_texts(&a), chapter_texts(&b), "{name}");
        assert_eq!(a.metadata().title, b.metadata().title);
    }
}

// ---------------------------------------------------------------------------------------------
// second wave: PDB, PML/PMLZ, RB, SNB, TCR, CBC, CHM, LIT, AZW4
// ---------------------------------------------------------------------------------------------

#[test]
fn second_wave_formats_convert_with_right_metadata_and_text() {
    for (name, bytes, title, text) in samples2() {
        let arc = read_any(name, &bytes).unwrap_or_else(|e| panic!("{name}: {e:#}"));
        let book = validate(&arc);
        assert_eq!(book.metadata().title, title, "{name}");
        assert!(chapter_texts(&book).join(" ").contains(text), "{name}");
        assert!(!book.toc().is_empty(), "{name}: toc");
    }
}

#[test]
fn every_identifiable_file_is_claimed_by_its_own_reader_first() {
    // pick_reader tries the extension first; an unknown extension forces content sniffing, which
    // walks the registry in order: no reader may shadow another one's files.
    let readers = default_readers();
    for (name, bytes, id) in identifiable() {
        let got = bookx_core::contracts::pick_reader(&readers, "unknown.zzz", &bytes)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(got.id(), id, "{name} sniffed as {}", got.id());
        let by_ext = bookx_core::contracts::pick_reader(&readers, name, &bytes).unwrap();
        assert_eq!(by_ext.id(), id, "{name} by extension");
    }
}

#[test]
fn old_formats_still_win_for_their_files_and_new_sniffers_reject_them() {
    let new_ids = [
        "pdb", "azw4", "rb", "snb", "tcr", "chm", "lit", "pmlz", "cbc", "pml",
    ];
    let readers = default_readers();
    for (name, bytes, id) in identifiable() {
        for r in readers.iter().filter(|r| new_ids.contains(&r.id())) {
            if r.id() == id {
                continue;
            }
            assert!(!r.sniff(&bytes), "{} claims {name} ({id})", r.id());
        }
    }
}

#[test]
fn new_readers_reject_other_formats_samples_with_errors_not_panics() {
    let readers = default_readers();
    for (name, bytes, id) in identifiable() {
        for r in readers.iter().filter(|r| r.id() != id) {
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| r.read(&bytes)));
            assert!(res.is_ok(), "{} panicked on {name}", r.id());
        }
    }
}

#[test]
fn new_readers_survive_200_mutations_and_truncations() {
    let readers = default_readers();
    for (name, bytes, _) in identifiable() {
        let ext = name.rsplit('.').next().unwrap();
        let r = readers
            .iter()
            .find(|r| r.extensions().contains(&ext))
            .unwrap();
        fuzz_no_panic(r, &bytes);
        for cut in (0..bytes.len()).step_by((bytes.len() / 80).max(1)) {
            let res = std::panic::catch_unwind(|| read_any(name, &bytes[..cut]));
            assert!(res.is_ok(), "{name} cut at {cut} panicked");
        }
    }
}

#[test]
fn lit_and_azw4_give_the_documented_errors_through_read_any() {
    let e = read_any("book.lit", &write_lit(false)).unwrap_err();
    assert!(format!("{e:#}").contains("discontinued format"));
    let e = read_any("book.lit", &write_lit(true)).unwrap_err();
    assert!(format!("{e:#}").contains("discontinued format"));
    let replica = write_mobi(&MobiSpec {
        title: "Replica".into(),
        authors: vec!["Ann".into()],
        html: "<html><body><p>%PDF-1.4</p></body></html>".into(),
        ..Default::default()
    });
    let e = read_any("book.azw4", &replica).unwrap_err();
    let m = format!("{e:#}");
    assert!(
        m.contains("AZW4 is a PDF wrapper") && m.contains("Replica") && m.contains("Ann"),
        "{m}"
    );
}

#[test]
fn pdb_extension_routes_by_database_type() {
    let palm = write_palmdoc("Palm Doc", "Hello from palmdoc.\n");
    let book = validate(&read_any("x.pdb", &palm).unwrap());
    assert!(
        chapter_texts(&book)
            .join(" ")
            .contains("Hello from palmdoc.")
    );
    let e = read_any("x.pdb", &pdb_assemble("n", b"Plkr", b"Data", &[vec![1]])).unwrap_err();
    assert!(format!("{e:#}").contains("unsupported Palm database type"));
}

#[test]
fn drm_protected_inputs_have_clear_errors() {
    let e = read_any(
        "x.pdb",
        &write_ereader(&EReaderSpec {
            version: 272,
            text: "x".into(),
            ..Default::default()
        }),
    )
    .unwrap_err();
    assert!(format!("{e:#}").contains("DRM protected"));
}

#[test]
fn second_wave_big_inputs_stay_fast() {
    let mut text = String::new();
    for i in 0..300 {
        text.push_str(&format!("\\xChapter {i}\\x\n"));
        for _ in 0..30 {
            text.push_str("The quick brown fox jumps over the lazy \\idog\\i again and again.\n");
        }
    }
    let f = write_ereader(&EReaderSpec {
        version: 10,
        title: "Big".into(),
        text,
        ..Default::default()
    });
    let t = std::time::Instant::now();
    let book = validate(&read_any("big.pdb", &f).unwrap());
    assert_eq!(book.spine_paths().len(), 300);
    assert!(t.elapsed().as_secs() < 60, "took {:?}", t.elapsed());
}

/// Cross-check of the CHM test writer (and thereby of the reader's assumptions) with the
/// independent `extract_chmLib` tool from chmlib. Skipped when the tool is not installed.
#[test]
fn chm_fixture_is_accepted_by_chmlib_when_available() {
    let tool = std::process::Command::new("extract_chmLib").output();
    if tool.is_err() {
        eprintln!("skipped: extract_chmLib is not installed");
        return;
    }
    let big: Vec<u8> = (0..120_000u32)
        .map(|i| b"<p>lorem ipsum dolor sit amet</p>\n"[(i % 34) as usize])
        .collect();
    for (compressed, reset) in [(false, 1), (true, 1), (true, 2)] {
        let files = vec![
            (
                "/index.htm".to_string(),
                b"<html><body><p>Hello chmlib</p></body></html>".to_vec(),
            ),
            ("/dir/big.htm".to_string(), big.clone()),
        ];
        let f = write_chm(&ChmSpec {
            title: "X".into(),
            files: files.clone(),
            compressed,
            reset_frames: reset,
            ..Default::default()
        });
        let dir = std::env::temp_dir().join(format!(
            "bookx-chm-{}-{compressed}-{reset}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.chm");
        std::fs::write(&path, &f).unwrap();
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let status = std::process::Command::new("extract_chmLib")
            .arg(&path)
            .arg(&out)
            .output()
            .unwrap();
        assert!(status.status.success(), "extract_chmLib failed: {status:?}");
        for (p, data) in &files {
            let got = std::fs::read(out.join(p.trim_start_matches('/'))).unwrap_or_else(|e| {
                panic!("{p} not extracted (compressed={compressed}, reset={reset}): {e}")
            });
            let first_diff = got.iter().zip(data.iter()).position(|(a, b)| a != b);
            assert!(
                got.len() == data.len() && first_diff.is_none(),
                "{p} compressed={compressed} reset={reset}: lengths {} vs {}, first difference at {first_diff:?}",
                got.len(),
                data.len()
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
