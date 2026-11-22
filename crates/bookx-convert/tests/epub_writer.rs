//! EPUB writer, validated by unzipping and parsing the output with `zip` and `roxmltree` only.

mod common;

use std::collections::HashSet;
use std::io::{Cursor, Read};

use bookx_convert::writers::{EpubVersion, EpubWriter};
use bookx_core::archive::Archive;
use bookx_core::book::OpenBook;
use bookx_core::contracts::FormatWriter;
use roxmltree::Document;
use zip::{CompressionMethod, ZipArchive};

struct Unzipped {
    names: Vec<String>,
    files: std::collections::HashMap<String, Vec<u8>>,
    first_method: CompressionMethod,
}

fn unzip(bytes: &[u8]) -> Unzipped {
    let mut z = ZipArchive::new(Cursor::new(bytes)).expect("valid zip");
    let mut names = Vec::new();
    let mut files = std::collections::HashMap::new();
    let mut first_method = CompressionMethod::Deflated;
    for i in 0..z.len() {
        let mut f = z.by_index(i).expect("entry");
        if i == 0 {
            first_method = f.compression();
        }
        let name = f.name().expect("name").to_string();
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).expect("read");
        names.push(name.clone());
        files.insert(name, buf);
    }
    Unzipped {
        names,
        files,
        first_method,
    }
}

fn parse(bytes: &[u8]) -> Document<'static> {
    let text: &'static str = Box::leak(
        String::from_utf8(bytes.to_vec())
            .expect("utf8")
            .into_boxed_str(),
    );
    Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .expect("well-formed XML")
}

fn opf_path(z: &Unzipped) -> String {
    let c = parse(&z.files["META-INF/container.xml"]);
    c.descendants()
        .find(|n| n.has_tag_name("rootfile"))
        .and_then(|n| n.attribute("full-path"))
        .expect("rootfile")
        .to_string()
}

fn dir_of(p: &str) -> &str {
    p.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

fn join(dir: &str, href: &str) -> String {
    let mut parts: Vec<&str> = if dir.is_empty() {
        vec![]
    } else {
        dir.split('/').collect()
    };
    for seg in href.split('/') {
        match seg {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// Checks every rule of the OCF/OPF containers we can verify without epubveri.
fn assert_valid_epub(bytes: &[u8], expect_version: &str) -> Unzipped {
    let z = unzip(bytes);
    // OCF: mimetype first, stored, exact content
    assert_eq!(z.names[0], "mimetype");
    assert_eq!(z.first_method, CompressionMethod::Stored);
    assert_eq!(z.files["mimetype"], b"application/epub+zip");
    assert!(z.files.contains_key("META-INF/container.xml"));
    // no duplicate entries
    let set: HashSet<&String> = z.names.iter().collect();
    assert_eq!(set.len(), z.names.len(), "duplicate zip entries");

    let opf = opf_path(&z);
    let doc = parse(&z.files[&opf]);
    let root = doc.root_element();
    assert_eq!(root.tag_name().name(), "package");
    assert_eq!(root.attribute("version"), Some(expect_version));
    let uid = root
        .attribute("unique-identifier")
        .expect("unique-identifier");
    let ident_ids: Vec<&str> = doc
        .descendants()
        .filter(|n| n.has_tag_name("identifier"))
        .filter_map(|n| n.attribute("id"))
        .collect();
    assert!(
        ident_ids.contains(&uid),
        "unique-identifier {uid} must name a dc:identifier"
    );
    for req in ["title", "language", "identifier"] {
        assert!(
            doc.descendants().any(|n| n.has_tag_name(req)),
            "dc:{req} missing"
        );
    }

    // manifest
    let base = dir_of(&opf).to_string();
    let items: Vec<_> = doc
        .descendants()
        .filter(|n| {
            n.has_tag_name("item") && n.parent().is_some_and(|p| p.has_tag_name("manifest"))
        })
        .collect();
    let mut ids = HashSet::new();
    let mut hrefs = HashSet::new();
    for it in &items {
        let id = it.attribute("id").expect("item id");
        assert!(ids.insert(id), "duplicate manifest id {id}");
        let href = it.attribute("href").expect("href");
        assert!(hrefs.insert(href), "duplicate href {href}");
        let path = join(&base, href);
        assert!(
            z.files.contains_key(&path),
            "manifest points at missing file {path}"
        );
        assert!(
            !it.attribute("media-type").unwrap_or("").is_empty(),
            "media-type of {id}"
        );
    }
    // every content file is in the manifest (except container/opf/mimetype)
    for name in &z.names {
        if name == "mimetype" || name.starts_with("META-INF/") || *name == opf {
            continue;
        }
        assert!(
            items
                .iter()
                .any(|it| join(&base, it.attribute("href").unwrap()) == *name),
            "{name} is not in the manifest"
        );
    }
    // spine
    let spine = root
        .children()
        .find(|n| n.has_tag_name("spine"))
        .expect("spine");
    let refs: Vec<_> = spine
        .children()
        .filter(|n| n.has_tag_name("itemref"))
        .collect();
    assert!(!refs.is_empty());
    for r in refs {
        assert!(
            ids.contains(r.attribute("idref").unwrap()),
            "dangling itemref"
        );
    }
    if let Some(toc) = spine.attribute("toc") {
        assert!(ids.contains(toc));
    }
    // every XHTML file parses
    for it in items
        .iter()
        .filter(|i| i.attribute("media-type") == Some("application/xhtml+xml"))
    {
        parse(&z.files[&join(&base, it.attribute("href").unwrap())]);
    }
    z
}

#[test]
fn epub3_has_nav_cover_and_valid_package() {
    let book = common::sample_book();
    let bytes = EpubWriter::default().write(&book).unwrap();
    let z = assert_valid_epub(&bytes, "3.0");
    let opf = opf_path(&z);
    let doc = parse(&z.files[&opf]);
    let base = dir_of(&opf);

    let nav_items: Vec<_> = doc
        .descendants()
        .filter(|n| {
            n.has_tag_name("item")
                && n.attribute("properties")
                    .is_some_and(|p| p.split(' ').any(|x| x == "nav"))
        })
        .collect();
    assert_eq!(nav_items.len(), 1);
    let nav = parse(&z.files[&join(base, nav_items[0].attribute("href").unwrap())]);
    let toc_nav = nav
        .descendants()
        .find(|n| {
            n.has_tag_name("nav")
                && n.attributes()
                    .any(|a| a.name() == "type" && a.value() == "toc")
        })
        .expect("toc nav");
    let links: Vec<String> = toc_nav
        .descendants()
        .filter(|n| n.has_tag_name("a"))
        .map(|a| a.text().unwrap_or("").to_string())
        .collect();
    assert_eq!(links, vec!["Chapter One", "Chapter Two", "Chapter Three"]);
    for a in toc_nav.descendants().filter(|n| n.has_tag_name("a")) {
        let href = a.attribute("href").unwrap();
        assert!(
            z.files
                .contains_key(&join(base, href.split('#').next().unwrap()))
        );
    }

    assert!(
        doc.descendants()
            .any(|n| n.has_tag_name("meta") && n.attribute("property") == Some("dcterms:modified"))
    );
    // cover
    let cover_meta = doc
        .descendants()
        .find(|n| n.has_tag_name("meta") && n.attribute("name") == Some("cover"))
        .and_then(|n| n.attribute("content"))
        .expect("cover meta");
    let cover_item = doc
        .descendants()
        .find(|n| n.has_tag_name("item") && n.attribute("id") == Some(cover_meta))
        .expect("cover item");
    assert!(
        cover_item
            .attribute("properties")
            .unwrap_or("")
            .contains("cover-image")
    );
    assert_eq!(cover_item.attribute("media-type"), Some("image/jpeg"));
    assert_eq!(
        z.files[&join(base, cover_item.attribute("href").unwrap())],
        book.cover_bytes().unwrap()
    );
    // metadata
    let text = |name: &str| -> Vec<String> {
        doc.descendants()
            .filter(|n| n.has_tag_name(name))
            .map(|n| n.text().unwrap_or("").to_string())
            .collect()
    };
    assert_eq!(text("title"), vec!["The Test Book: A Story"]);
    assert_eq!(text("creator"), vec!["Ada Lovelace", "Grace Hopper"]);
    assert_eq!(text("language"), vec!["en"]);
    assert_eq!(text("publisher"), vec!["Bookx Press"]);
    assert_eq!(text("subject"), vec!["testing", "fiction"]);
    assert_eq!(
        text("description"),
        vec!["A <b>short</b> description & more."]
    );
    assert!(
        text("identifier")
            .iter()
            .any(|i| i == "urn:isbn:9781234567897")
    );
}

#[test]
fn epub2_has_ncx_and_no_epub3_features() {
    let book = common::sample_book();
    let bytes = EpubWriter {
        version: EpubVersion::V2,
    }
    .write(&book)
    .unwrap();
    let z = assert_valid_epub(&bytes, "2.0");
    let opf = opf_path(&z);
    let doc = parse(&z.files[&opf]);
    assert!(
        !doc.descendants()
            .any(|n| n.has_tag_name("item") && n.attribute("properties").is_some()),
        "EPUB 2 has no manifest properties"
    );
    assert!(
        !doc.descendants().any(|n| n.attribute("property").is_some()),
        "no EPUB 3 meta properties"
    );
    assert!(
        !z.names.iter().any(|n| n.ends_with("nav.xhtml")),
        "no nav document in EPUB 2"
    );
    let ncx_item = doc
        .descendants()
        .find(|n| {
            n.has_tag_name("item") && n.attribute("media-type") == Some("application/x-dtbncx+xml")
        })
        .expect("ncx item");
    let spine_toc = doc
        .descendants()
        .find(|n| n.has_tag_name("spine"))
        .unwrap()
        .attribute("toc");
    assert_eq!(spine_toc, ncx_item.attribute("id"));
    let ncx = parse(&z.files[&join(dir_of(&opf), ncx_item.attribute("href").unwrap())]);
    let titles: Vec<&str> = ncx
        .descendants()
        .filter(|n| n.has_tag_name("navPoint"))
        .map(|n| {
            n.descendants()
                .find(|c| c.has_tag_name("text"))
                .and_then(|t| t.text())
                .unwrap_or("")
        })
        .collect();
    assert_eq!(titles, vec!["Chapter One", "Chapter Two", "Chapter Three"]);
    let orders: Vec<u32> = ncx
        .descendants()
        .filter(|n| n.has_tag_name("navPoint"))
        .map(|n| n.attribute("playOrder").unwrap().parse().unwrap())
        .collect();
    assert_eq!(orders, vec![1, 2, 3]);
    // the cover is still announced the EPUB 2 way
    assert!(
        doc.descendants()
            .any(|n| n.has_tag_name("meta") && n.attribute("name") == Some("cover"))
    );
}

#[test]
fn metadata_pages_and_cover_roundtrip_through_our_own_reader() {
    let book = common::nested_toc_book();
    for v in [EpubVersion::V2, EpubVersion::V3] {
        let bytes = EpubWriter { version: v }.write(&book).unwrap();
        let back = OpenBook::from_archive(Archive::from_bytes(bytes).unwrap()).unwrap();
        let (a, b) = (book.metadata(), back.metadata());
        assert_eq!(a.title, b.title);
        assert_eq!(a.authors, b.authors);
        assert_eq!(a.publisher, b.publisher);
        assert_eq!(a.language, b.language);
        assert_eq!(a.description, b.description);
        assert_eq!(a.tags, b.tags);
        assert_eq!(a.series, b.series);
        assert_eq!(a.series_index, b.series_index);
        assert_eq!(a.published, b.published);
        assert_eq!(b.identifier("isbn"), Some("9781234567897"));
        assert_eq!(back.spine_paths(), book.spine_paths());
        assert_eq!(back.cover_bytes(), book.cover_bytes());
        let toc = back.toc();
        assert_eq!(toc.len(), 2, "{v:?}");
        assert_eq!(toc[0].title, "Part I");
        assert_eq!(toc[0].children[0].title, "Chapter Two");
        assert_eq!(toc[1].children[0].href, "OEBPS/chapter-3.xhtml#c3a");
    }
}

#[test]
fn repack_repairs_duplicate_ids_missing_files_and_orphans() {
    let mut book = common::sample_book();
    // duplicate manifest id, a manifest entry for a file that does not exist, an orphan image
    let opf = book.package.opf_path.clone();
    let text = book.archive.get_string(&opf).unwrap();
    let text = text.replace("id=\"ch3\"", "id=\"ch2\"");
    let text = text.replace(
        "</manifest>",
        "<item id=\"ghost\" href=\"ghost.xhtml\" media-type=\"application/xhtml+xml\"/></manifest>",
    );
    book.archive.set_string(&opf, &text);
    book.archive
        .set("OEBPS/images/orphan.png", common::png(8, 8));
    book.reload_package().unwrap();

    let bytes = EpubWriter::default().write(&book).unwrap();
    let z = assert_valid_epub(&bytes, "3.0");
    assert!(!z.files.keys().any(|k| k.contains("ghost")));
    assert!(
        z.files.contains_key("OEBPS/images/orphan.png"),
        "orphans are added to the manifest"
    );
}

#[test]
fn book_without_any_toc_gets_a_generated_one() {
    let mut book = common::sample_book();
    let pkg = book.package.clone();
    bookx_core::toc::write_toc(&mut book.archive, &pkg, &[]).unwrap();
    assert!(book.toc().is_empty());
    let bytes = EpubWriter::default().write(&book).unwrap();
    let z = assert_valid_epub(&bytes, "3.0");
    let back = OpenBook::from_archive(Archive::from_bytes(bytes).unwrap()).unwrap();
    let toc = back.toc();
    assert_eq!(toc.len(), 3, "one entry per spine file");
    assert!(z.names.iter().any(|n| n.ends_with("nav.xhtml")));
}

#[test]
fn identity_and_extension() {
    assert_eq!(EpubWriter::default().id(), "epub");
    assert_eq!(
        EpubWriter {
            version: EpubVersion::V2
        }
        .id(),
        "epub2"
    );
    assert_eq!(EpubWriter::default().extension(), "epub");
    assert_eq!(EpubVersion::V2.as_str(), "2.0");
    assert_eq!(EpubVersion::default(), EpubVersion::V3);
    let j = serde_json::to_string(&EpubVersion::V2).unwrap();
    assert_eq!(j, "\"2\"");
    assert_eq!(
        serde_json::from_str::<EpubVersion>("\"3\"").unwrap(),
        EpubVersion::V3
    );
}

#[test]
fn output_is_stable_apart_from_timestamps() {
    let book = common::sample_book();
    let a = unzip(&EpubWriter::default().write(&book).unwrap());
    let b = unzip(&EpubWriter::default().write(&book).unwrap());
    assert_eq!(a.names, b.names);
    for n in &a.names {
        if n.ends_with(".opf") {
            continue; // contains dcterms:modified
        }
        assert_eq!(a.files[n], b.files[n], "{n}");
    }
}

#[test]
fn empty_metadata_still_produces_a_valid_package() {
    let mut book = common::sample_book();
    let mut md = book.metadata().clone();
    md.title.clear();
    md.language.clear();
    md.identifiers.clear();
    md.authors.clear();
    book.set_metadata(md).unwrap();
    let bytes = EpubWriter::default().write(&book).unwrap();
    assert_valid_epub(&bytes, "3.0");
}
