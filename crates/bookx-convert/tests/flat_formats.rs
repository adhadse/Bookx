//! FB2, HTMLZ, TXTZ, TXT, Markdown and HTML output, validated independently.

mod common;

use std::io::{Cursor, Read};

use base64::Engine;
use bookx_convert::writers::{
    Fb2Writer, HtmlWriter, HtmlzWriter, MarkdownWriter, TxtWriter, TxtzWriter,
};
use bookx_core::contracts::FormatWriter;
use roxmltree::Document;
use zip::ZipArchive;

fn unzip(bytes: &[u8]) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut z = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut m = std::collections::BTreeMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let mut b = Vec::new();
        f.read_to_end(&mut b).unwrap();
        m.insert(f.name().unwrap().to_string(), b);
    }
    m
}

const FB2: &str = "http://www.gribuser.ru/xml/fictionbook/2.0";
const XLINK: &str = "http://www.w3.org/1999/xlink";

fn fb2_doc(bytes: &[u8]) -> Document<'static> {
    let text: &'static str = Box::leak(String::from_utf8(bytes.to_vec()).unwrap().into_boxed_str());
    Document::parse(text).expect("FB2 must be well-formed XML")
}

fn el<'a>(n: roxmltree::Node<'a, 'a>, name: &str) -> Option<roxmltree::Node<'a, 'a>> {
    n.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
}

#[test]
fn fb2_structure_and_metadata() {
    let book = common::sample_book();
    let bytes = Fb2Writer.write(&book).unwrap();
    assert!(bytes.starts_with(b"<?xml"));
    let doc = fb2_doc(&bytes);
    let root = doc.root_element();
    assert_eq!(root.tag_name().name(), "FictionBook");
    assert_eq!(root.tag_name().namespace(), Some(FB2));
    let desc = el(root, "description").unwrap();
    let ti = el(desc, "title-info").unwrap();
    assert_eq!(
        el(ti, "book-title").unwrap().text(),
        Some("The Test Book: A Story")
    );
    assert_eq!(el(ti, "lang").unwrap().text(), Some("en"));
    let authors: Vec<(String, String)> = ti
        .children()
        .filter(|c| c.has_tag_name((FB2, "author")))
        .map(|a| {
            (
                el(a, "first-name")
                    .and_then(|n| n.text())
                    .unwrap_or("")
                    .to_string(),
                el(a, "last-name")
                    .and_then(|n| n.text())
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .collect();
    assert_eq!(
        authors,
        vec![
            ("Ada".into(), "Lovelace".into()),
            ("Grace".into(), "Hopper".into())
        ]
    );
    assert_eq!(el(ti, "keywords").unwrap().text(), Some("testing, fiction"));
    assert_eq!(el(ti, "date").unwrap().text(), Some("2020-05-17"));
    let seq = el(ti, "sequence").unwrap();
    assert_eq!(seq.attribute("name"), Some("Saga"));
    assert_eq!(seq.attribute("number"), Some("2"));
    let annotation: String = el(ti, "annotation")
        .unwrap()
        .descendants()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect();
    assert_eq!(annotation, "A short description & more.");
    // element order required by the schema
    let order: Vec<&str> = ti
        .children()
        .filter(|c| c.is_element())
        .map(|c| c.tag_name().name())
        .collect();
    let expected = [
        "genre",
        "author",
        "author",
        "book-title",
        "annotation",
        "keywords",
        "date",
        "coverpage",
        "lang",
        "sequence",
    ];
    assert_eq!(order, expected);
    let pi = el(desc, "publish-info").unwrap();
    assert_eq!(el(pi, "publisher").unwrap().text(), Some("Bookx Press"));
    assert_eq!(el(pi, "isbn").unwrap().text(), Some("9781234567897"));
    let di = el(desc, "document-info").unwrap();
    assert!(el(di, "id").is_some() && el(di, "version").is_some() && el(di, "date").is_some());
}

#[test]
fn fb2_body_sections_and_text() {
    let book = common::sample_book();
    let doc = fb2_doc(&Fb2Writer.write(&book).unwrap());
    let body = el(doc.root_element(), "body").unwrap();
    let sections: Vec<_> = body
        .children()
        .filter(|c| c.has_tag_name((FB2, "section")))
        .collect();
    assert_eq!(sections.len(), 3);
    let titles: Vec<String> = sections
        .iter()
        .map(|s| {
            el(*s, "title")
                .unwrap()
                .descendants()
                .filter(|n| n.is_text())
                .filter_map(|n| n.text())
                .collect::<String>()
        })
        .collect();
    assert_eq!(titles, vec!["Chapter One", "Chapter Two", "Chapter Three"]);
    let all: String = body
        .descendants()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(all.contains("It was a \"dark\" night & the rain fell."));
    assert!(all.contains("Naïve café — 日本語."));
    assert!(all.contains("The end of the story."));
    // inline markup became FB2 elements
    let p_with_em = sections[0]
        .descendants()
        .find(|n| n.has_tag_name((FB2, "emphasis")))
        .unwrap();
    assert_eq!(p_with_em.text(), Some("italic"));
    assert!(
        sections[0]
            .descendants()
            .any(|n| n.has_tag_name((FB2, "strong")) && n.text() == Some("bold"))
    );
    // external link kept as <a l:href>, internal link flattened to text
    let link = sections[1]
        .descendants()
        .find(|n| n.has_tag_name((FB2, "a")))
        .unwrap();
    assert_eq!(
        link.attribute((XLINK, "href")),
        Some("https://example.org/x")
    );
    assert_eq!(
        sections[1]
            .descendants()
            .filter(|n| n.has_tag_name((FB2, "a")))
            .count(),
        1
    );
}

#[test]
fn fb2_binaries_are_base64_of_the_original_images() {
    let book = common::sample_book();
    let doc = fb2_doc(&Fb2Writer.write(&book).unwrap());
    let root = doc.root_element();
    let bins: Vec<_> = root
        .children()
        .filter(|c| c.has_tag_name((FB2, "binary")))
        .collect();
    assert_eq!(bins.len(), 2, "cover + inline picture");
    let engine = base64::engine::general_purpose::STANDARD;
    let mut decoded = Vec::new();
    for b in &bins {
        let raw: String = b.text().unwrap().split_whitespace().collect();
        decoded.push((
            b.attribute("id").unwrap().to_string(),
            b.attribute("content-type").unwrap().to_string(),
            engine.decode(raw).unwrap(),
        ));
    }
    assert!(
        decoded
            .iter()
            .any(|(_, ct, d)| ct == "image/jpeg" && d == book.cover_bytes().unwrap())
    );
    assert!(
        decoded.iter().any(|(_, ct, d)| ct == "image/png"
            && d == book.archive.get("OEBPS/images/pic.png").unwrap())
    );
    // coverpage and inline <image> point at existing binaries
    let ids: Vec<&str> = bins.iter().map(|b| b.attribute("id").unwrap()).collect();
    for img in doc.descendants().filter(|n| n.has_tag_name((FB2, "image"))) {
        let href = img.attribute((XLINK, "href")).unwrap();
        assert!(ids.contains(&href.trim_start_matches('#')), "{href}");
    }
    let cover = el(
        el(el(root, "description").unwrap(), "title-info").unwrap(),
        "coverpage",
    )
    .unwrap();
    assert!(el(cover, "image").is_some());
    // base64 lines are wrapped
    let raw = bins[0].text().unwrap();
    assert!(raw.lines().all(|l| l.len() <= 76));
}

#[test]
fn fb2_minimal_book_is_still_valid() {
    let mut book = common::sample_book();
    let mut md = book.metadata().clone();
    md.authors.clear();
    md.title.clear();
    md.tags.clear();
    md.series.clear();
    md.description.clear();
    md.publisher.clear();
    book.set_metadata(md).unwrap();
    book.remove_cover().unwrap();
    let doc = fb2_doc(&Fb2Writer.write(&book).unwrap());
    let ti = el(el(doc.root_element(), "description").unwrap(), "title-info").unwrap();
    assert_eq!(el(ti, "book-title").unwrap().text(), Some("Untitled"));
    assert!(el(ti, "author").and_then(|a| el(a, "nickname")).is_some());
    assert!(el(ti, "coverpage").is_none());
}

#[test]
fn fb2_author_names() {
    use bookx_convert::writers::fb2::author_xml;
    assert_eq!(
        author_xml("Madonna"),
        "<author><last-name>Madonna</last-name></author>"
    );
    assert_eq!(
        author_xml("Ursula K. Le Guin"),
        "<author><first-name>Ursula</first-name><middle-name>K. Le</middle-name><last-name>Guin</last-name></author>"
    );
    assert_eq!(
        author_xml("  "),
        "<author><nickname>Unknown</nickname></author>"
    );
    assert!(author_xml("A & B").contains("&amp;"));
}

#[test]
fn htmlz_contains_index_cover_metadata_and_images() {
    let book = common::sample_book();
    let files = unzip(&HtmlzWriter.write(&book).unwrap());
    for f in ["index.html", "style.css", "metadata.opf"] {
        assert!(files.contains_key(f), "missing {f}");
    }
    let cover = files
        .iter()
        .find(|(n, _)| n.starts_with("cover."))
        .expect("cover file");
    assert_eq!(cover.1.as_slice(), book.cover_bytes().unwrap());

    // metadata.opf via roxmltree
    let opf = String::from_utf8(files["metadata.opf"].clone()).unwrap();
    let od = Document::parse(&opf).unwrap();
    let t = |name: &str| -> Vec<String> {
        od.descendants()
            .filter(|n| n.has_tag_name(name))
            .map(|n| n.text().unwrap_or("").to_string())
            .collect()
    };
    assert_eq!(t("title"), vec!["The Test Book: A Story"]);
    assert_eq!(t("creator"), vec!["Ada Lovelace", "Grace Hopper"]);
    assert_eq!(t("publisher"), vec!["Bookx Press"]);
    let cover_meta = od
        .descendants()
        .find(|n| n.has_tag_name("meta") && n.attribute("name") == Some("cover"))
        .unwrap();
    assert_eq!(cover_meta.attribute("content"), Some("cover"));
    // manifest hrefs exist
    for it in od.descendants().filter(|n| n.has_tag_name("item")) {
        assert!(files.contains_key(it.attribute("href").unwrap()));
    }

    // index.html via a real HTML parser
    let html = String::from_utf8(files["index.html"].clone()).unwrap();
    let h = scraper::Html::parse_document(&html);
    let sel = |s: &str| scraper::Selector::parse(s).unwrap();
    assert_eq!(
        h.select(&sel("title"))
            .next()
            .unwrap()
            .text()
            .collect::<String>(),
        "The Test Book: A Story"
    );
    assert!(
        h.select(&sel("link[rel=stylesheet]"))
            .any(|l| l.value().attr("href") == Some("style.css"))
    );
    let body_text: String = h.select(&sel("body")).next().unwrap().text().collect();
    for want in [
        "Chapter One",
        "Chapter Two",
        "Chapter Three",
        "The end of the story.",
    ] {
        assert!(body_text.contains(want), "{want}");
    }
    // images were copied and rewritten
    let imgs: Vec<&str> = h
        .select(&sel("img"))
        .filter_map(|i| i.value().attr("src"))
        .collect();
    assert_eq!(imgs.len(), 1);
    assert!(imgs[0].starts_with("images/"));
    assert_eq!(
        files[imgs[0]],
        book.archive.get("OEBPS/images/pic.png").unwrap()
    );
    // internal links now point at anchors of the single file; external ones are untouched
    let ids: std::collections::HashSet<String> = h
        .select(&sel("[id]"))
        .filter_map(|e| e.value().attr("id").map(String::from))
        .collect();
    let mut internal = 0;
    for a in h.select(&sel("a")) {
        let href = a.value().attr("href").unwrap();
        if href.starts_with("http") {
            assert_eq!(href, "https://example.org/x");
        } else {
            internal += 1;
            assert!(href.starts_with('#'), "{href}");
            assert!(ids.contains(&href[1..]), "{href} has no target");
        }
    }
    assert_eq!(internal, 2);
    // stylesheet is the concatenation of the book's css
    let css = String::from_utf8(files["style.css"].clone()).unwrap();
    assert!(css.contains("line-height"));
}

#[test]
fn htmlz_without_cover() {
    let mut book = common::sample_book();
    book.remove_cover().unwrap();
    let files = unzip(&HtmlzWriter.write(&book).unwrap());
    assert!(!files.keys().any(|n| n.starts_with("cover.")));
    let opf = String::from_utf8(files["metadata.opf"].clone()).unwrap();
    assert!(!opf.contains("name=\"cover\""));
}

#[test]
fn txtz_has_index_txt_and_metadata() {
    let book = common::sample_book();
    let files = unzip(&TxtzWriter.write(&book).unwrap());
    let txt = String::from_utf8(files["index.txt"].clone()).unwrap();
    assert!(txt.starts_with("The Test Book: A Story\nAda Lovelace & Grace Hopper"));
    assert!(txt.contains("It was a \"dark\" night & the rain fell."));
    assert!(txt.contains("CHAPTER ONE"));
    let opf = String::from_utf8(files["metadata.opf"].clone()).unwrap();
    let d = Document::parse(&opf).unwrap();
    assert!(
        d.descendants()
            .any(|n| n.has_tag_name("title") && n.text() == Some("The Test Book: A Story"))
    );
}

#[test]
fn plain_writers_wrap_the_core_exporters() {
    let book = common::sample_book();
    let txt = String::from_utf8(TxtWriter.write(&book).unwrap()).unwrap();
    assert_eq!(
        txt,
        bookx_core::convert::export_book(&book, bookx_core::convert::ExportFormat::Txt)
    );
    let md = String::from_utf8(MarkdownWriter.write(&book).unwrap()).unwrap();
    assert!(md.starts_with("---\ntitle: \"The Test Book: A Story\""));
    assert!(md.contains("# Chapter One"));
    assert!(md.contains("*italic*") && md.contains("**bold**"));
    let html = String::from_utf8(HtmlWriter.write(&book).unwrap()).unwrap();
    assert!(html.starts_with("<!DOCTYPE html>"));
    let parsed = scraper::Html::parse_document(&html);
    assert!(
        parsed
            .root_element()
            .text()
            .collect::<String>()
            .contains("The end of the story.")
    );
    let ids: Vec<(&str, &str)> = vec![
        (TxtWriter.id(), TxtWriter.extension()),
        (MarkdownWriter.id(), MarkdownWriter.extension()),
        (HtmlWriter.id(), HtmlWriter.extension()),
        (TxtzWriter.id(), TxtzWriter.extension()),
        (HtmlzWriter.id(), HtmlzWriter.extension()),
        (Fb2Writer.id(), Fb2Writer.extension()),
    ];
    assert_eq!(
        ids,
        vec![
            ("txt", "txt"),
            ("md", "md"),
            ("html", "html"),
            ("txtz", "txtz"),
            ("htmlz", "htmlz"),
            ("fb2", "fb2")
        ]
    );
}

#[test]
fn registry_lists_writers_with_unique_ids() {
    let r = bookx_convert::default_writers();
    let ids: Vec<&str> = r.iter().map(|w| w.id()).collect();
    for want in [
        "epub", "kepub", "fb2", "htmlz", "txtz", "txt", "md", "html", "mobi", "azw3",
    ] {
        assert!(ids.contains(&want), "missing {want}");
    }
    let mut s = ids.clone();
    s.sort();
    s.dedup();
    assert_eq!(s.len(), ids.len());
}
