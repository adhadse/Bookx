//! Plain text, Markdown, HTML, HTMLZ and TXTZ.
//!
//! TXT, Markdown and HTML go through the converter of `bookx-core` (`import_to_epub`), so the
//! chapter heuristics stay identical to the ones the library uses when importing those formats.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use bookx_core::archive::Archive;
use bookx_core::contracts::FormatReader;
use bookx_core::convert::{import_to_epub, Chapter};
use bookx_core::opf::Metadata;

use crate::common::*;

/// `.txt` with encoding detection.
pub struct TxtReader;
/// `.md` / `.markdown`.
pub struct MarkdownReader;
/// `.html` / `.htm` / `.xhtml`.
pub struct HtmlReader;
/// Calibre's zipped HTML (`index.html`, optional `metadata.opf`, cover and images).
pub struct HtmlzReader;
/// Calibre's zipped text (`index.txt`, optional `metadata.opf`, cover and images).
pub struct TxtzReader;

/// The core importer titles a book after its file name; this stands in for it and is replaced.
const STEM_PLACEHOLDER: &str = "bookx-formats-file-stem";

/// Chapters and basic metadata produced by the core importer.
struct CoreImport {
    md: Metadata,
    chapters: Vec<Chapter>,
}

fn import_text_via_core(ext: &str, text: &str, stem: &str) -> Result<CoreImport> {
    if text.trim().is_empty() {
        bail!("the file is empty");
    }
    let dir: PathBuf = std::env::temp_dir().join(format!("bookx-formats-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).context("creating a scratch directory")?;
    let file = dir.join(format!("{STEM_PLACEHOLDER}.{ext}"));
    let result = std::fs::write(&file, text)
        .context("writing the scratch file")
        .and_then(|_| import_to_epub(&file));
    let _ = std::fs::remove_dir_all(&dir);
    let arc = result?;
    let opf = arc.get_string("OEBPS/content.opf").unwrap_or_default();
    let info = parse_opf_info(&opf).unwrap_or_default();
    let mut chapters = Vec::new();
    for n in 1.. {
        let Some(src) = arc.get_string(&format!("OEBPS/chapter-{n}.xhtml")) else {
            break;
        };
        let title = src
            .split_once("<title>")
            .and_then(|(_, r)| r.split_once("</title>"))
            .map(|(t, _)| xml_unescape(t))
            .unwrap_or_default();
        let body = src
            .split_once("<body>\n")
            .and_then(|(_, r)| r.rsplit_once("</body>"))
            .map(|(b, _)| b.to_string())
            .unwrap_or_default();
        chapters.push(Chapter { title, body });
    }
    let mut md = info.md;
    if md.title == STEM_PLACEHOLDER {
        md.title = stem.to_string();
    }
    Ok(CoreImport { md, chapters })
}

fn stem_of(file_name: &str) -> String {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    base.rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(base)
        .to_string()
}

/// Converts a text-like file. `ext` is `txt`, `md` or `html`; `file_name` supplies the title when
/// the content has none (pass `""` when unknown).
pub fn convert_plain(ext: &str, bytes: &[u8], file_name: &str) -> Result<Archive> {
    let text = if ext == "html" {
        decode_xml(bytes)
    } else {
        decode_text(bytes)
    };
    let stem = match stem_of(file_name) {
        s if s.is_empty() => "Untitled".to_string(),
        s => s,
    };
    if ext == "html" {
        return convert_html_single(&text, &stem);
    }
    let imp = import_text_via_core(ext, &text, &stem)?;
    build(Parts {
        md: imp.md,
        chapters: imp.chapters,
        ..Default::default()
    })
}

/// A single HTML document: kept as markup (ids, images with data URIs, links, tables, lists),
/// split into chapters at the top level headings; title, author and description come from the
/// `<title>` and `<meta>` elements.
fn convert_html_single(html: &str, stem: &str) -> Result<Archive> {
    if html.trim().is_empty() {
        bail!("the file is empty");
    }
    let doc = bookx_core::util::parse_html_document(html);
    let meta = |name: &str| -> Option<String> {
        let sel = scraper::Selector::parse(&format!("meta[name=\"{name}\" i]")).ok()?;
        doc.select(&sel)
            .next()
            .and_then(|m| m.value().attr("content"))
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
    };
    let mut md = Metadata {
        title: html_title(html).unwrap_or_else(|| stem.to_string()),
        language: "en".into(),
        ..Default::default()
    };
    if let Some(a) = meta("author") {
        md.authors = a
            .split([';', '&'])
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect();
    }
    if let Some(d) = meta("description") {
        md.description = d;
    }
    if let Some(l) = scraper::Selector::parse("html")
        .ok()
        .and_then(|sel| doc.select(&sel).next())
        .and_then(|h| h.value().attr("lang"))
        .filter(|l| !l.is_empty())
    {
        md.language = l.to_string();
    }
    // a lone file cannot bring its pictures along: keep the alt text instead of a dead image
    let dead_img = regex::Regex::new(r#"<img\b[^>]*/>"#).expect("static regex");
    let alt = regex::Regex::new(r#"\balt="([^"]*)""#).expect("static regex");
    let local_src = regex::Regex::new(r#"\bsrc="[^":]*""#).expect("static regex");
    let blocks: Vec<String> = html_blocks(html, &|_| None)
        .into_iter()
        .map(|b| {
            dead_img
                .replace_all(&b, |c: &regex::Captures| {
                    let tag = &c[0];
                    if local_src.is_match(tag) {
                        alt.captures(tag)
                            .map(|a| a[1].to_string())
                            .unwrap_or_default()
                    } else {
                        tag.to_string()
                    }
                })
                .into_owned()
        })
        .collect();
    let chapters = chapters_from_blocks(blocks, &md.title.clone());
    build(Parts {
        md,
        chapters,
        ..Default::default()
    })
}

macro_rules! plain_reader {
    ($ty:ty, $id:literal, $ext:literal, $exts:expr, $sniff:expr) => {
        impl FormatReader for $ty {
            fn id(&self) -> &'static str {
                $id
            }
            fn extensions(&self) -> &'static [&'static str] {
                $exts
            }
            fn sniff(&self, head: &[u8]) -> bool {
                let f: fn(&[u8]) -> bool = $sniff;
                f(head)
            }
            fn read(&self, bytes: &[u8]) -> Result<Archive> {
                convert_plain($ext, bytes, "")
            }
        }
    };
}

fn looks_like_text(head: &[u8]) -> bool {
    let n = head.len().min(2048);
    if n == 0 {
        return false;
    }
    if head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]) {
        return true;
    }
    !head[..n]
        .iter()
        .any(|&b| b == 0 || (b < 9) || (b > 13 && b < 32 && b != 27))
}

fn looks_like_html(head: &[u8]) -> bool {
    let n = head.len().min(2048);
    let s = String::from_utf8_lossy(&head[..n]).to_ascii_lowercase();
    let t = s.trim_start_matches('\u{feff}').trim_start();
    t.starts_with("<!doctype html")
        || t.starts_with("<html")
        || t.starts_with("<?xml") && s.contains("<html")
}

plain_reader!(
    TxtReader,
    "txt",
    "txt",
    &["txt", "text"],
    |h| looks_like_text(h) && !looks_like_html(h)
);
plain_reader!(
    MarkdownReader,
    "md",
    "md",
    &["md", "markdown"],
    |h| looks_like_text(h) && String::from_utf8_lossy(&h[..h.len().min(8)]).starts_with("---\n")
);
plain_reader!(
    HtmlReader,
    "html",
    "html",
    &["html", "htm", "xhtml", "xht"],
    |h| looks_like_html(h)
);

// ---------------------------------------------------------------------------
// HTMLZ / TXTZ
// ---------------------------------------------------------------------------

fn find_with_ext<'a>(
    zip: &'a ZipFiles,
    exts: &[&str],
    prefer: &str,
) -> Option<&'a (String, Vec<u8>)> {
    zip.files
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(prefer))
        .or_else(|| {
            zip.files.iter().find(|(n, _)| {
                let l = n.to_ascii_lowercase();
                exts.iter().any(|e| l.ends_with(e))
            })
        })
}

/// Picks the cover: the OPF reference, else a file called `cover.*`.
fn zip_cover(zip: &ZipFiles, href: Option<&str>) -> Option<(Vec<u8>, String)> {
    let by_ref = href.and_then(|h| {
        let (p, _) = bookx_core::util::resolve_href("", h);
        zip.get(&p)
    });
    let data = by_ref.or_else(|| {
        zip.files
            .iter()
            .find(|(n, _)| {
                let l = n.to_ascii_lowercase();
                l.starts_with("cover.") || l.ends_with("/cover.jpg") || l.ends_with("/cover.png")
            })
            .map(|(_, d)| d.as_slice())
    })?;
    let ext = image_ext(data)?;
    Some((data.to_vec(), ext.to_string()))
}

fn read_opf(zip: &ZipFiles) -> OpfInfo {
    zip.get_ci("metadata.opf")
        .or_else(|| {
            zip.files
                .iter()
                .find(|(n, _)| n.to_ascii_lowercase().ends_with(".opf"))
                .map(|(_, d)| d.as_slice())
        })
        .and_then(|d| parse_opf_info(&decode_xml(d)))
        .unwrap_or_default()
}

/// Converts an HTMLZ archive.
pub fn convert_htmlz(bytes: &[u8]) -> Result<Archive> {
    let zip = read_zip(bytes)?;
    let Some((html_name, html_bytes)) =
        find_with_ext(&zip, &[".html", ".htm", ".xhtml"], "index.html")
    else {
        bail!("the HTMLZ archive has no HTML file");
    };
    let html = decode_xml(html_bytes);
    let info = read_opf(&zip);
    let mut md = info.md.clone();
    if md.title.is_empty() {
        md.title = html_title(&html).unwrap_or_default();
    }
    let dir = bookx_core::util::parent_dir(html_name).to_string();
    let mut media = MediaSet::default();
    for (name, data) in &zip.files {
        if image_ext(data).is_some() && !name.to_ascii_lowercase().starts_with("cover.") {
            media.add(name, data.clone());
        }
    }
    let rewrite = |src: &str| -> Option<String> {
        if bookx_core::util::is_external(src) {
            return None;
        }
        let (path, _) = bookx_core::util::resolve_href(&dir, src);
        media.path_of(&path).map(str::to_string)
    };
    let blocks = html_blocks(&html, &rewrite);
    let default_title = if md.title.is_empty() {
        "Beginning".to_string()
    } else {
        md.title.clone()
    };
    let chapters = chapters_from_blocks(blocks, &default_title);
    let used: Vec<Media> = media
        .items
        .iter()
        .filter(|m| chapters.iter().any(|c| c.body.contains(&m.path)))
        .cloned()
        .collect();
    build(Parts {
        md,
        chapters,
        cover: zip_cover(&zip, info.cover_href.as_deref()),
        media: used,
        ..Default::default()
    })
}

/// Converts a TXTZ archive.
pub fn convert_txtz(bytes: &[u8]) -> Result<Archive> {
    let zip = read_zip(bytes)?;
    let Some((name, data)) =
        find_with_ext(&zip, &[".txt", ".md", ".markdown", ".text"], "index.txt")
    else {
        bail!("the TXTZ archive has no text file");
    };
    let l = name.to_ascii_lowercase();
    let ext = if l.ends_with(".md") || l.ends_with(".markdown") {
        "md"
    } else {
        "txt"
    };
    let text = decode_text(data);
    let info = read_opf(&zip);
    let imp = import_text_via_core(ext, &text, &stem_of(name))?;
    let mut md = imp.md;
    let m = info.md.clone();
    if !m.title.is_empty() {
        md.title = m.title;
    }
    if !m.authors.is_empty() {
        md.authors = m.authors;
    }
    for (dst, src) in [
        (&mut md.publisher, m.publisher),
        (&mut md.language, m.language),
        (&mut md.description, m.description),
        (&mut md.published, m.published),
        (&mut md.series, m.series),
    ] {
        if !src.is_empty() {
            *dst = src;
        }
    }
    if m.series_index > 0.0 {
        md.series_index = m.series_index;
    }
    md.tags = m.tags;
    md.identifiers = m.identifiers;
    build(Parts {
        md,
        chapters: imp.chapters,
        cover: zip_cover(&zip, info.cover_href.as_deref()),
        ..Default::default()
    })
}

impl FormatReader for HtmlzReader {
    fn id(&self) -> &'static str {
        "htmlz"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["htmlz"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        zip_names(head).is_some_and(|n| n.iter().any(|f| f.eq_ignore_ascii_case("index.html")))
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        convert_htmlz(bytes)
    }
}

impl FormatReader for TxtzReader {
    fn id(&self) -> &'static str {
        "txtz"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["txtz"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        zip_names(head).is_some_and(|n| n.iter().any(|f| f.eq_ignore_ascii_case("index.txt")))
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        convert_txtz(bytes)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::testutil::*;

    pub const OPF: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
<dc:title>Zipped Title</dc:title><dc:creator>Zed Author</dc:creator><dc:language>de</dc:language>
<dc:subject>t1</dc:subject><dc:publisher>Pub</dc:publisher><meta name="cover" content="c"/></metadata>
<manifest><item id="c" href="cover.png" media-type="image/png"/></manifest></package>"#;

    pub fn htmlz_sample() -> Vec<u8> {
        let html = br#"<html><head><title>HTML Title</title></head><body>
<p>Intro text.</p><h1>First</h1><p>One <i>it</i> <img src="images/pic one.jpg"/></p><h1>Second</h1><p>Two</p></body></html>"#;
        make_zip(&[
            ("index.html", html),
            ("metadata.opf", OPF.as_bytes()),
            ("cover.png", PNG),
            ("images/pic one.jpg", JPG),
            ("images/unused.png", PNG),
        ])
    }

    #[test]
    fn txt_chapters_title_and_paragraphs() {
        let txt = "Intro line\n\nChapter 1\n\nFirst para\nwraps here.\n\nChapter 2\n\nSecond.\n";
        let arc = convert_plain("txt", txt.as_bytes(), "/books/My Novel.txt").unwrap();
        let book = validate(&arc);
        assert_eq!(book.metadata().title, "My Novel");
        assert_eq!(
            toc_titles(&book),
            vec!["Beginning", "Chapter 1", "Chapter 2"]
        );
        assert!(chapter_texts(&book)[1].contains("First para wraps here."));
        let r = TxtReader.read(txt.as_bytes()).unwrap();
        assert_eq!(validate(&r).metadata().title, "Untitled");
    }

    #[test]
    fn txt_encodings_are_detected() {
        let want = "Café Ünïcode — “q”";
        let utf8 = convert_plain("txt", format!("\u{feff}{want}\n").as_bytes(), "a.txt").unwrap();
        assert!(chapter_texts(&validate(&utf8))[0].contains(want));
        let utf16: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain(want.encode_utf16().flat_map(|u| u.to_le_bytes()))
            .collect();
        let a = convert_plain("txt", &utf16, "a.txt").unwrap();
        assert!(chapter_texts(&validate(&a))[0].contains(want));
        let (latin, _, _) = encoding_rs::WINDOWS_1252.encode("Caf\u{e9} na\u{ef}ve");
        let a = convert_plain("txt", &latin, "a.txt").unwrap();
        assert!(chapter_texts(&validate(&a))[0].contains("Café naïve"));
    }

    #[test]
    fn empty_and_blank_text_is_an_error() {
        for ext in ["txt", "md", "html"] {
            assert!(convert_plain(ext, b"", "x")
                .unwrap_err()
                .to_string()
                .contains("empty"));
            assert!(convert_plain(ext, b" \n\t\n", "x").is_err());
        }
    }

    #[test]
    fn non_ascii_titles_do_not_crash_the_pipeline() {
        let arc = convert_plain("txt", "Привет мир\n".as_bytes(), "Война и мир.txt").unwrap();
        let book = validate(&arc);
        assert_eq!(book.metadata().title, "Война и мир");
    }

    #[test]
    fn markdown_front_matter_and_chapters() {
        let md = "---\ntitle: \"The Story\"\nauthor: \"Ada\"\nlang: fr\n---\n# One\n\nHello *there*\n\n# Two\n\n- a\n- b\n";
        let arc = MarkdownReader.read(md.as_bytes()).unwrap();
        let book = validate(&arc);
        assert_eq!(book.metadata().title, "The Story");
        assert_eq!(book.metadata().authors, vec!["Ada"]);
        assert_eq!(book.metadata().language, "fr");
        assert_eq!(toc_titles(&book), vec!["One", "Two"]);
        assert!(chapter_xml(&book, 0).contains("<em>there</em>"));
        assert!(chapter_xml(&book, 1).contains("<li>a</li>"));
    }

    #[test]
    fn html_title_and_body() {
        let h = b"<!DOCTYPE html><html><head><title>Page &amp; Co</title></head><body><h1>Hi</h1><p>Body text</p></body></html>";
        let book = validate(&HtmlReader.read(h).unwrap());
        assert_eq!(book.metadata().title, "Page & Co");
        assert!(chapter_texts(&book)[0].contains("Body text"));
        let l1 = b"<html><head><meta charset='windows-1251'><title>x</title></head><body><p>\xcf\xf0\xe8\xe2\xe5\xf2</p></body></html>";
        let book = validate(&convert_plain("html", l1, "a.html").unwrap());
        assert!(chapter_texts(&book)[0].contains("Привет"));
    }

    #[test]
    fn htmlz_metadata_cover_images_and_chapters() {
        let arc = convert_htmlz(&htmlz_sample()).unwrap();
        let book = validate(&arc);
        let md = book.metadata();
        assert_eq!(md.title, "Zipped Title");
        assert_eq!(md.authors, vec!["Zed Author"]);
        assert_eq!(md.language, "de");
        assert_eq!(md.publisher, "Pub");
        assert_eq!(md.tags, vec!["t1"]);
        assert_eq!(book.cover_bytes(), Some(PNG));
        assert_eq!(toc_titles(&book), vec!["Zipped Title", "First", "Second"]);
        assert!(chapter_xml(&book, 1).contains("<img src=\"images/pic_one.jpg\" alt=\"\"/>"));
        assert_eq!(arc.get("OEBPS/images/pic_one.jpg"), Some(JPG));
        assert!(
            !arc.contains("OEBPS/images/unused.png"),
            "unreferenced images are dropped"
        );
        assert!(chapter_texts(&book)[0].contains("Intro text."));
    }

    #[test]
    fn htmlz_without_opf_uses_html_title_and_cover_file() {
        let z = make_zip(&[
            (
                "index.html",
                b"<html><head><title>Only HTML</title></head><body><p>x</p></body></html>",
            ),
            ("cover.jpg", JPG),
        ]);
        let book = validate(&convert_htmlz(&z).unwrap());
        assert_eq!(book.metadata().title, "Only HTML");
        assert_eq!(book.cover_bytes(), Some(JPG));
    }

    #[test]
    fn htmlz_errors() {
        assert!(convert_htmlz(&make_zip(&[("a.png", PNG)]))
            .unwrap_err()
            .to_string()
            .contains("no HTML"));
        assert!(convert_htmlz(b"nope").is_err());
        let empty = make_zip(&[("index.html", b"<html><body></body></html>")]);
        assert!(convert_htmlz(&empty)
            .unwrap_err()
            .to_string()
            .contains("no readable text"));
    }

    #[test]
    fn txtz_reads_text_metadata_and_cover() {
        let z = make_zip(&[
            ("index.txt", "Chapter 1\n\nHello zipped world.\n".as_bytes()),
            ("metadata.opf", OPF.as_bytes()),
            ("cover.png", PNG),
        ]);
        let book = validate(&convert_txtz(&z).unwrap());
        assert_eq!(book.metadata().title, "Zipped Title");
        assert_eq!(book.metadata().authors, vec!["Zed Author"]);
        assert_eq!(book.cover_bytes(), Some(PNG));
        assert!(chapter_texts(&book)
            .iter()
            .any(|t| t.contains("Hello zipped world.")));
        // markdown inside a txtz
        let z = make_zip(&[("index.md", b"# Head\n\nBody *em*\n")]);
        let book = validate(&convert_txtz(&z).unwrap());
        assert!(chapter_xml(&book, 0).contains("<em>em</em>"));
        // no metadata: title from the text file name
        let z = make_zip(&[("story.txt", b"Just text.\n")]);
        assert_eq!(
            validate(&convert_txtz(&z).unwrap()).metadata().title,
            "story"
        );
        assert!(convert_txtz(&make_zip(&[("a.png", PNG)]))
            .unwrap_err()
            .to_string()
            .contains("no text file"));
    }

    #[test]
    fn sniffing_and_ids() {
        assert!(HtmlzReader.sniff(&htmlz_sample()));
        assert!(!HtmlzReader.sniff(&make_zip(&[("x.txt", b"a")])));
        assert!(TxtzReader.sniff(&make_zip(&[("index.txt", b"a")])));
        assert!(!TxtzReader.sniff(&htmlz_sample()));
        assert!(HtmlReader.sniff(b"<!DOCTYPE html><html>"));
        assert!(HtmlReader.sniff(b"  <HTML><body>"));
        assert!(!HtmlReader.sniff(b"plain"));
        assert!(TxtReader.sniff(b"Just some text\nlines"));
        assert!(!TxtReader.sniff(b"<html><body>"));
        assert!(!TxtReader.sniff(b"\0\x01\x02binary"));
        assert!(!TxtReader.sniff(b""));
        assert!(TxtReader.sniff(&[0xFF, 0xFE, b'a', 0]));
        assert!(MarkdownReader.sniff(b"---\ntitle: x\n---\n"));
        assert!(!MarkdownReader.sniff(b"hello"));
        let ids: Vec<_> = [
            TxtReader.id(),
            MarkdownReader.id(),
            HtmlReader.id(),
            HtmlzReader.id(),
            TxtzReader.id(),
        ]
        .into();
        assert_eq!(ids, vec!["txt", "md", "html", "htmlz", "txtz"]);
        assert!(TxtReader.extensions().contains(&"txt"));
        assert!(MarkdownReader.extensions().contains(&"markdown"));
        assert!(HtmlReader.extensions().contains(&"htm"));
        assert_eq!(HtmlzReader.extensions(), &["htmlz"]);
        assert_eq!(TxtzReader.extensions(), &["txtz"]);
    }

    #[test]
    fn robust_against_garbage_and_mutations() {
        assert_rejects_garbage(&TxtReader);
        assert_rejects_garbage(&MarkdownReader);
        assert_rejects_garbage(&HtmlReader);
        assert_rejects_garbage(&HtmlzReader);
        assert_rejects_garbage(&TxtzReader);
        fuzz_no_panic(&HtmlzReader, &htmlz_sample());
        fuzz_no_panic(
            &TxtzReader,
            &make_zip(&[
                ("index.txt", b"Chapter 1\n\nHi\n"),
                ("metadata.opf", OPF.as_bytes()),
            ]),
        );
        fuzz_no_panic(
            &TxtReader,
            b"Chapter 1\n\nHello there\n\nChapter 2\n\nMore\n",
        );
        fuzz_no_panic(&MarkdownReader, b"---\ntitle: x\n---\n# A\n\ntext\n");
        fuzz_no_panic(
            &HtmlReader,
            b"<html><head><title>x</title></head><body><h1>A</h1><p>t</p></body></html>",
        );
    }

    #[test]
    fn scratch_directories_are_cleaned_up() {
        let before = std::fs::read_dir(std::env::temp_dir())
            .map(|d| {
                d.flatten()
                    .filter(|e| {
                        e.file_name()
                            .to_string_lossy()
                            .starts_with("bookx-formats-")
                    })
                    .count()
            })
            .unwrap_or(0);
        let _ = convert_plain("txt", b"hello\n", "a.txt").unwrap();
        let _ = convert_plain("md", b"", "a.md");
        let after = std::fs::read_dir(std::env::temp_dir())
            .map(|d| {
                d.flatten()
                    .filter(|e| {
                        e.file_name()
                            .to_string_lossy()
                            .starts_with("bookx-formats-")
                    })
                    .count()
            })
            .unwrap_or(0);
        // other tests may run in parallel and hold a directory briefly; ours must not leak one
        assert!(after <= before + 8, "{before} -> {after}");
    }
}
