//! Fixtures and an independent EPUB validator for the integration tests (everything generated in code).

#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};
use std::path::PathBuf;

use bookx_core::archive::Archive;
use bookx_core::book::OpenBook;
use bookx_core::convert::{create_epub, Chapter};
use bookx_core::opf::Metadata;

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

pub fn unique_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(
        format!(
            "bookx-editing-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        )
        .replace(['(', ')', ' '], ""),
    );
    std::fs::create_dir_all(&d).unwrap();
    d
}

pub fn metadata() -> Metadata {
    Metadata {
        title: "Integration".into(),
        authors: vec!["Jane Doe".into()],
        language: "en".into(),
        ..Default::default()
    }
}

pub fn book_from_bodies(bodies: &[String]) -> OpenBook {
    let chapters: Vec<Chapter> = bodies
        .iter()
        .enumerate()
        .map(|(i, b)| Chapter {
            title: format!("Chapter {}", i + 1),
            body: format!("{b}\n"),
        })
        .collect();
    OpenBook::from_archive(create_epub(&metadata(), &chapters, None)).unwrap()
}

/// A hand-written, deliberately broken EPUB 2 (as zip bytes).
pub fn messy_epub() -> Vec<u8> {
    let mut a = Archive::default();
    a.set("mimetype", b"application/epub+zip\n".to_vec());
    a.set_string(
        "META-INF/container.xml",
        "<?xml version=\"1.0\"?><container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>",
    );
    a.set_string(
        "OEBPS/content.opf",
        r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:title>Messy Book</dc:title>
    <dc:creator opf:role="aut" opf:file-as="Doe, Jane">Jane Doe</dc:creator>
    <dc:identifier id="uid" opf:scheme="UUID">0f0e0d0c-0b0a-0908-0706-050403020100</dc:identifier>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="c1" href="c1.html" media-type="application/xhtml+xml"/>
    <item id="c2" href="text/c2.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="text/c3.xhtml" media-type="application/xhtml+xml"/>
    <item id="css" href="style.css" media-type="text/css"/>
    <item id="ghost" href="ghost.xhtml" media-type="application/xhtml+xml"/>
    <item id="pic" href="img/pic.png" media-type="image/png"/>
  </manifest>
  <spine toc="ncx"><itemref idref="c1"/><itemref idref="c2"/><itemref idref="ghost"/></spine>
  <guide><reference type="text" title="Start" href="c1.html"/></guide>
</package>
"#,
    );
    a.set_string(
        "OEBPS/toc.ncx",
        r#"<?xml version="1.0"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1"><head><meta name="dtb:uid" content="x"/></head><docTitle><text>Messy Book</text></docTitle>
<navMap>
<navPoint id="n1" playOrder="1"><navLabel><text>First</text></navLabel><content src="c1.html"/></navPoint>
<navPoint id="n2" playOrder="2"><navLabel><text>Second</text></navLabel><content src="text/c2.xhtml#sec"/></navPoint>
</navMap></ncx>"#,
    );
    a.set_string(
        "OEBPS/c1.html",
        "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.1//EN\" \"http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd\">\n<HTML><HEAD><TITLE>One</TITLE><link rel=stylesheet href=style.css></HEAD>\n<BODY><H1>First</H1>\n<P>Tom &amp; Jerry &nbsp;meet<BR>again. <A HREF=\"text/c2.xhtml#sec\">Next</A>\n<IMG SRC=img/pic.png>\n<P class=note>A second paragraph &mdash; with an entity.</BODY></HTML>",
    );
    a.set_string(
        "OEBPS/text/c2.xhtml",
        "<?xml version=\"1.0\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>Two</title><link rel=\"stylesheet\" href=\"../style.css\"/></head><body><h2 id=\"sec\">Second</h2><p id=\"dup\">Back to <a href=\"../c1.html\">one</a>.</p><p id=\"dup\">Dup id.</p></body></html>",
    );
    a.set_string(
        "OEBPS/text/c3.xhtml",
        "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>Three</title></head><body><p>Orphan chapter.</p></body></html>",
    );
    a.set_string(
        "OEBPS/style.css",
        "body { font-family: serif }\n.note { color: red }\n.unused { color: blue }\n",
    );
    a.set("OEBPS/img/pic.png", png(6, 4));
    a.set("OEBPS/extra/stray.png", png(2, 2));
    a.to_bytes().unwrap()
}

// ---------------------------------------------------------------------------
// Independent validation: zip crate + roxmltree only, no code from bookx_core's package model.
// ---------------------------------------------------------------------------

fn parse(text: &str) -> roxmltree::Document<'_> {
    roxmltree::Document::parse_with_options(
        text.trim_start_matches('\u{feff}'),
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .unwrap_or_else(|e| panic!("not well-formed XML: {e}\n{text}"))
}

fn resolve(base_dir: &str, href: &str) -> String {
    let href = href.split('?').next().unwrap();
    let mut parts: Vec<&str> = if href.starts_with('/') {
        vec![]
    } else {
        base_dir.split('/').filter(|s| !s.is_empty()).collect()
    };
    for seg in href.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Everything an EPUB reader needs, checked from the zip bytes alone. Returns the file map for further asserts.
pub fn validate_epub(bytes: &[u8]) -> HashMap<String, Vec<u8>> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("a zip file");
    // mimetype first, stored, exact
    {
        let first = zip.by_index(0).unwrap();
        assert_eq!(
            first.name().unwrap(),
            "mimetype",
            "mimetype must be the first entry"
        );
        assert_eq!(
            first.compression(),
            zip::CompressionMethod::Stored,
            "mimetype must be stored"
        );
    }
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).unwrap();
        let name = f.name().unwrap().to_string();
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).unwrap();
        files.insert(name, buf);
    }
    assert_eq!(files["mimetype"], b"application/epub+zip");
    let container = String::from_utf8(files["META-INF/container.xml"].clone()).unwrap();
    let cdoc = parse(&container);
    let opf_path = cdoc
        .descendants()
        .find(|n| n.has_tag_name("rootfile"))
        .and_then(|n| n.attribute("full-path"))
        .expect("rootfile")
        .to_string();
    let opf_text =
        String::from_utf8(files.get(&opf_path).expect("package document").clone()).unwrap();
    let opf = parse(&opf_text);
    let opf_dir = opf_path.rsplit_once('/').map(|x| x.0).unwrap_or("");
    let pkg = opf.root_element();
    let version = pkg.attribute("version").expect("package version");
    assert!(matches!(version, "2.0" | "3.0"), "version {version}");
    // metadata
    let md = pkg
        .children()
        .find(|n| n.has_tag_name("metadata"))
        .expect("metadata");
    let dc = |name: &str| {
        md.children()
            .filter(|n| n.tag_name().name() == name)
            .count()
    };
    assert!(dc("title") >= 1, "dc:title");
    assert!(dc("language") >= 1, "dc:language");
    let ids: Vec<(Option<&str>, String)> = md
        .children()
        .filter(|n| n.tag_name().name() == "identifier")
        .map(|n| (n.attribute("id"), n.text().unwrap_or("").to_string()))
        .collect();
    assert!(!ids.is_empty(), "dc:identifier");
    let uid = pkg
        .attribute("unique-identifier")
        .expect("unique-identifier");
    assert!(
        ids.iter().any(|(id, _)| *id == Some(uid)),
        "unique-identifier points at an identifier"
    );
    // manifest
    let manifest = pkg
        .children()
        .find(|n| n.has_tag_name("manifest"))
        .expect("manifest");
    let mut manifest_ids = HashSet::new();
    let mut manifest_paths = HashSet::new();
    let mut media: HashMap<String, String> = HashMap::new();
    for item in manifest.children().filter(|n| n.is_element()) {
        let id = item.attribute("id").unwrap();
        assert!(
            manifest_ids.insert(id.to_string()),
            "duplicate manifest id {id}"
        );
        let path = resolve(opf_dir, &decode(item.attribute("href").unwrap()));
        assert!(
            files.contains_key(&path),
            "manifest item {id} points to missing {path}"
        );
        manifest_paths.insert(path.clone());
        media.insert(path, item.attribute("media-type").unwrap().to_string());
    }
    let spine = pkg
        .children()
        .find(|n| n.has_tag_name("spine"))
        .expect("spine");
    assert!(spine.children().any(|n| n.is_element()), "spine not empty");
    for r in spine.children().filter(|n| n.is_element()) {
        assert!(
            manifest_ids.contains(r.attribute("idref").unwrap()),
            "spine idref"
        );
    }
    if let Some(t) = spine.attribute("toc") {
        assert!(manifest_ids.contains(t));
    }
    for name in files.keys() {
        if name == "mimetype" || name.starts_with("META-INF/") || *name == opf_path {
            continue;
        }
        assert!(
            manifest_paths.contains(name),
            "{name} is not in the manifest"
        );
    }
    // every markup/ncx/css document is well formed and its links resolve
    let mut ids_per_file: HashMap<String, HashSet<String>> = HashMap::new();
    let mut docs: Vec<(String, String)> = Vec::new();
    for (name, data) in &files {
        let ext = name.rsplit('.').next().unwrap_or("");
        if matches!(ext, "xhtml" | "html" | "htm" | "ncx" | "opf" | "svg") {
            let text =
                String::from_utf8(data.clone()).unwrap_or_else(|_| panic!("{name} is not UTF-8"));
            let d = parse(&text);
            if matches!(ext, "xhtml" | "html" | "htm") {
                ids_per_file.insert(
                    name.clone(),
                    d.descendants()
                        .filter(|n| n.is_element())
                        .filter_map(|n| n.attribute("id"))
                        .map(|s| s.to_string())
                        .collect(),
                );
                docs.push((name.clone(), text));
            }
        }
    }
    for (name, text) in &docs {
        let d = parse(text);
        let dir = name.rsplit_once('/').map(|x| x.0).unwrap_or("");
        for n in d.descendants().filter(|n| n.is_element()) {
            for attr in ["href", "src"] {
                let Some(v) = n.attribute(attr) else { continue };
                let lower = v.to_ascii_lowercase();
                if v.is_empty()
                    || lower.starts_with("http:")
                    || lower.starts_with("https:")
                    || lower.starts_with("mailto:")
                    || lower.starts_with("data:")
                {
                    continue;
                }
                let (path, frag) = match v.split_once('#') {
                    Some((p, f)) => (p, Some(f)),
                    None => (v, None),
                };
                let target = if path.is_empty() {
                    name.clone()
                } else {
                    resolve(dir, &decode(path))
                };
                assert!(
                    files.contains_key(&target),
                    "{name}: {attr}=\"{v}\" -> missing {target}"
                );
                if let (Some(f), Some(ids)) = (frag, ids_per_file.get(&target)) {
                    assert!(
                        ids.contains(&decode(f)),
                        "{name}: {attr}=\"{v}\" -> no id {f} in {target}"
                    );
                }
            }
        }
    }
    // NCX targets
    for (name, data) in &files {
        if name.ends_with(".ncx") {
            let text = String::from_utf8(data.clone()).unwrap();
            let d = parse(&text);
            let dir = name.rsplit_once('/').map(|x| x.0).unwrap_or("");
            for n in d.descendants().filter(|n| n.has_tag_name("content")) {
                let v = n.attribute("src").unwrap();
                let path = v.split('#').next().unwrap();
                assert!(
                    files.contains_key(&resolve(dir, &decode(path))),
                    "NCX target {v}"
                );
            }
        }
    }
    files
}

pub fn spine_text(book: &OpenBook) -> Vec<String> {
    book.spine_paths()
        .iter()
        .map(|p| bookx_editing::markup::normalized_text(&book.archive.get_string(p).unwrap()))
        .collect()
}

pub fn reopen(book: &OpenBook) -> OpenBook {
    OpenBook::from_archive(Archive::from_bytes(book.archive.to_bytes().unwrap()).unwrap()).unwrap()
}
