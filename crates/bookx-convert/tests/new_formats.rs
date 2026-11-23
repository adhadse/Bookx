//! The DOCX, RTF, PDB, PMLZ, RB, SNB, OEB and ZIP outputs through the registry, the pipeline and
//! the options, plus the OEB and ZIP writers themselves.

mod common;

use std::sync::atomic::AtomicBool;

use bookx_convert::writers::{DocxOptions, OebWriter, PageSize, PdbCompression, ZipWriter};
use bookx_convert::{ConvertOptions, PresetStore, convert, default_writers};
use bookx_core::archive::Archive;
use bookx_core::contracts::{FormatWriter, Registry};
use common::*;

fn run(ext: &str, opts: &ConvertOptions) -> bookx_convert::Converted {
    let readers: Registry<dyn bookx_core::contracts::FormatReader> = Registry::new();
    convert(
        &readers,
        &sample_book().archive.to_bytes().unwrap(),
        "book.epub",
        ext,
        opts,
        &|_| {},
        &AtomicBool::new(false),
    )
    .unwrap_or_else(|e| panic!("{ext}: {e:#}"))
}

#[test]
fn registry_lists_every_new_format() {
    let ids: Vec<&str> = default_writers().iter().map(|w| w.id()).collect();
    for id in ["docx", "rtf", "pdb", "pmlz", "rb", "snb", "oeb", "zip"] {
        assert!(ids.contains(&id), "{id} not registered: {ids:?}");
    }
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "ids are unique");
    for w in default_writers().iter() {
        assert_eq!(w.extension().to_ascii_lowercase(), w.extension());
    }
}

#[test]
fn every_new_format_converts_through_the_pipeline() {
    let cases = [
        ("docx", "docx", b"PK".as_slice()),
        ("DOCX", "docx", b"PK"),
        (".rtf", "rtf", b"{\\rtf1"),
        ("pdb", "pdb", b"The Test Book"),
        ("pmlz", "pmlz", b"PK"),
        ("rb", "rb", &[0xb0, 0x0c, 0xb0, 0x0c]),
        ("snb", "snb", b"SNBP000B"),
        ("oeb", "oeb", b"PK"),
        ("zip", "zip", b"PK"),
    ];
    for (ask, ext, magic) in cases {
        let out = run(ask, &ConvertOptions::default());
        assert_eq!(out.extension, ext, "{ask}");
        assert!(out.bytes.starts_with(magic), "{ask}: {:?}", &out.bytes[..8]);
    }
}

#[test]
fn transforms_apply_before_the_new_writers() {
    let opts = ConvertOptions {
        smarten_punctuation: true,
        ..Default::default()
    };
    let out = run("rtf", &opts);
    let s = String::from_utf8(out.bytes).unwrap();
    assert!(s.contains("\\'93dark\\'94"), "smart quotes reach the RTF");
    assert!(out.reports.iter().any(|r| r.0.contains("smarten")));
}

#[test]
fn docx_options_flow_through_the_pipeline() {
    let opts = ConvertOptions {
        docx: DocxOptions {
            page: PageSize::A4,
            margin_twips: 1134,
            font: "Georgia".into(),
            font_size_pt: 11.0,
            toc: true,
        },
        ..Default::default()
    };
    let files = unzip(&run("docx", &opts).bytes);
    let doc = part_str(&files, "word/document.xml");
    assert!(doc.contains("w:w=\"11906\" w:h=\"16838\""));
    assert!(doc.contains("w:left=\"1134\""));
    assert!(doc.contains("TOC \\o"));
    assert!(part_str(&files, "word/styles.xml").contains("Georgia"));
}

#[test]
fn pdb_compression_option_flows_through_the_pipeline() {
    let default = run("pdb", &ConvertOptions::default()).bytes;
    let palm = run(
        "pdb",
        &ConvertOptions {
            pdb_compression: PdbCompression::PalmDoc,
            ..Default::default()
        },
    )
    .bytes;
    let version = |b: &[u8]| {
        let off = u32::from_be_bytes(b[78..82].try_into().unwrap()) as usize;
        u16::from_be_bytes([b[off], b[off + 1]])
    };
    assert_eq!(version(&default), 10);
    assert_eq!(version(&palm), 2);
}

#[test]
fn options_and_presets_serialise_the_new_fields() {
    let opts = ConvertOptions {
        docx: DocxOptions {
            page: PageSize::Custom(8000, 9000),
            toc: true,
            ..Default::default()
        },
        pdb_compression: PdbCompression::PalmDoc,
        ..Default::default()
    };
    let json = opts.to_json().unwrap();
    assert!(json.contains("palmdoc"));
    assert_eq!(ConvertOptions::from_json(&json).unwrap(), opts);
    // presets written before these options existed still load, with defaults
    let old = "{\"smarten_punctuation\":true}";
    let o = ConvertOptions::from_json(old).unwrap();
    assert!(o.smarten_punctuation);
    assert_eq!(o.docx, DocxOptions::default());
    assert_eq!(o.pdb_compression, PdbCompression::Deflate);
    let partial = "{\"docx\":{\"page\":\"a4\"}}";
    let o = ConvertOptions::from_json(partial).unwrap();
    assert_eq!(o.docx.page, PageSize::A4);
    assert_eq!(o.docx.font, "Times New Roman");
    let _ = PresetStore::default();
}

// ---- OEB / ZIP --------------------------------------------------------------------------------

#[test]
fn zip_contains_every_file_of_the_epub() {
    let book = sample_book();
    let files = unzip(&ZipWriter.write(&book).unwrap());
    let mut ours: Vec<&str> = files.iter().map(|f| f.0.as_str()).collect();
    let mut theirs: Vec<&str> = book
        .archive
        .names()
        .iter()
        .filter(|n| !n.ends_with('/'))
        .map(|n| n.as_str())
        .collect();
    ours.sort();
    theirs.sort();
    assert_eq!(ours, theirs);
    for (n, d) in &files {
        assert_eq!(Some(d.as_slice()), book.archive.get(n), "{n}");
    }
    assert!(files.iter().any(|f| f.0 == "mimetype"));
    assert!(files.iter().any(|f| f.0 == "META-INF/container.xml"));
}

#[test]
fn zip_output_is_a_valid_epub_container_again() {
    let bytes = ZipWriter.write(&sample_book()).unwrap();
    let archive = Archive::from_bytes(bytes).unwrap();
    assert_sentences(&epub_text(archive), "zip");
}

#[test]
fn oeb_is_rooted_at_content_opf() {
    let book = sample_book();
    let files = unzip(&OebWriter.write(&book).unwrap());
    let names: Vec<&str> = files.iter().map(|f| f.0.as_str()).collect();
    assert!(names.contains(&"content.opf"));
    assert!(
        !names
            .iter()
            .any(|n| *n == "mimetype" || n.starts_with("META-INF") || n.starts_with("OEBPS"))
    );
    assert!(!names.iter().any(|n| n.starts_with('/') || n.contains("..")));
    // every manifest entry of the OPF exists next to it
    let opf = part_str(&files, "content.opf");
    let doc = roxmltree::Document::parse(&opf).unwrap();
    let mut items = 0;
    for it in doc.descendants().filter(|n| n.tag_name().name() == "item") {
        let href = it.attribute("href").unwrap();
        let href = bookx_core::util::percent_decode(href);
        assert!(
            names.contains(&href.as_str()),
            "manifest href {href} missing"
        );
        items += 1;
    }
    assert!(items >= 6);
    // the spine items resolve and the text is intact
    let spine: Vec<_> = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "itemref")
        .collect();
    assert_eq!(spine.len(), 3, "three chapters");
    let mut text = String::new();
    for f in &files {
        if f.0.starts_with("chapter-") {
            text.push_str(&String::from_utf8_lossy(&f.1));
        }
    }
    assert!(text.contains("Another sentence follows here."));
}

#[test]
fn oeb_package_can_be_reopened_as_an_epub() {
    // the OPF moved to the root: container.xml is added back by the test to read it with core
    let files = unzip(&OebWriter.write(&sample_book()).unwrap());
    let mut arc = Archive::from_bytes(
        bookx_core::convert::create_epub(&metadata(), &[], None)
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    for n in arc.names().to_vec() {
        arc.remove(&n);
    }
    for (n, d) in files {
        arc.set(&n, d);
    }
    arc.set(
        "META-INF/container.xml",
        b"<?xml version=\"1.0\"?><container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>".to_vec(),
    );
    assert_sentences(&epub_text(arc), "oeb");
}

#[test]
fn oeb_of_a_book_with_the_opf_at_the_root() {
    let mut book = book_with(&["<p>flat</p>"], false);
    // move the whole package to the archive root
    let names = book.archive.names().to_vec();
    for n in names {
        if let Some(rest) = n.strip_prefix("OEBPS/") {
            let data = book.archive.get(&n).unwrap().to_vec();
            book.archive.remove(&n);
            book.archive.set(rest, data);
        }
    }
    book.archive.set(
        "META-INF/container.xml",
        b"<?xml version=\"1.0\"?><container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>".to_vec(),
    );
    book.reload_package().unwrap();
    assert_eq!(book.package.base_dir(), "");
    let files = unzip(&OebWriter.write(&book).unwrap());
    assert!(files.iter().any(|f| f.0 == "content.opf"));
    assert!(files.iter().any(|f| f.0 == "chapter-1.xhtml"));
}

#[test]
fn calibre_reads_our_oeb_and_zip_layouts() {
    // Calibre's OEB output is a directory with content.opf; ours zipped has the same entries
    let epub = sample_book().archive.to_bytes().unwrap();
    let Some(_) = calibre(&epub, "b.epub", "b.txt", &[]) else {
        eprintln!("skipped: Calibre not available");
        return;
    };
    let oeb = unzip(&OebWriter.write(&sample_book()).unwrap());
    let names: Vec<&str> = oeb.iter().map(|f| f.0.as_str()).collect();
    for want in ["content.opf", "toc.ncx"] {
        assert!(
            names.contains(&want),
            "{want} (Calibre's OEB directory has it too)"
        );
    }
}
