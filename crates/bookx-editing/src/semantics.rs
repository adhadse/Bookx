//! "Set semantics": the EPUB 2 `<guide>` and the EPUB 3 `landmarks` navigation (E18 / Calibre "Set semantics").

use anyhow::{Context, Result};
use bookx_core::book::OpenBook;
use bookx_core::util::{parent_dir, parse_xml, relative_href, resolve_href, xml_escape};
use serde::{Deserialize, Serialize};

use crate::markup::*;
use crate::opf_edit::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Semantic {
    Cover,
    TitlePage,
    Toc,
    /// Where the reader should start reading (`text` in the guide, `bodymatter` in landmarks).
    Bodymatter,
    Foreword,
    Preface,
    Copyright,
    Dedication,
    Epigraph,
    Acknowledgements,
    Index,
    Glossary,
    Bibliography,
    Colophon,
    Notes,
    ListOfIllustrations,
    ListOfTables,
}

pub const ALL_SEMANTICS: &[Semantic] = &[
    Semantic::Cover,
    Semantic::TitlePage,
    Semantic::Toc,
    Semantic::Bodymatter,
    Semantic::Foreword,
    Semantic::Preface,
    Semantic::Copyright,
    Semantic::Dedication,
    Semantic::Epigraph,
    Semantic::Acknowledgements,
    Semantic::Index,
    Semantic::Glossary,
    Semantic::Bibliography,
    Semantic::Colophon,
    Semantic::Notes,
    Semantic::ListOfIllustrations,
    Semantic::ListOfTables,
];

impl Semantic {
    /// `type` value of an OPF `<reference>`.
    pub fn guide_type(self) -> &'static str {
        match self {
            Semantic::Cover => "cover",
            Semantic::TitlePage => "title-page",
            Semantic::Toc => "toc",
            Semantic::Bodymatter => "text",
            Semantic::Foreword => "foreword",
            Semantic::Preface => "preface",
            Semantic::Copyright => "copyright-page",
            Semantic::Dedication => "dedication",
            Semantic::Epigraph => "epigraph",
            Semantic::Acknowledgements => "acknowledgements",
            Semantic::Index => "index",
            Semantic::Glossary => "glossary",
            Semantic::Bibliography => "bibliography",
            Semantic::Colophon => "colophon",
            Semantic::Notes => "notes",
            Semantic::ListOfIllustrations => "loi",
            Semantic::ListOfTables => "lot",
        }
    }

    /// `epub:type` value of a landmark.
    pub fn epub_type(self) -> &'static str {
        match self {
            Semantic::Cover => "cover",
            Semantic::TitlePage => "titlepage",
            Semantic::Toc => "toc",
            Semantic::Bodymatter => "bodymatter",
            Semantic::Foreword => "foreword",
            Semantic::Preface => "preface",
            Semantic::Copyright => "copyright-page",
            Semantic::Dedication => "dedication",
            Semantic::Epigraph => "epigraph",
            Semantic::Acknowledgements => "acknowledgements",
            Semantic::Index => "index",
            Semantic::Glossary => "glossary",
            Semantic::Bibliography => "bibliography",
            Semantic::Colophon => "colophon",
            Semantic::Notes => "endnotes",
            Semantic::ListOfIllustrations => "loi",
            Semantic::ListOfTables => "lot",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Semantic::Cover => "Cover",
            Semantic::TitlePage => "Title Page",
            Semantic::Toc => "Table of Contents",
            Semantic::Bodymatter => "Start of Content",
            Semantic::Foreword => "Foreword",
            Semantic::Preface => "Preface",
            Semantic::Copyright => "Copyright",
            Semantic::Dedication => "Dedication",
            Semantic::Epigraph => "Epigraph",
            Semantic::Acknowledgements => "Acknowledgements",
            Semantic::Index => "Index",
            Semantic::Glossary => "Glossary",
            Semantic::Bibliography => "Bibliography",
            Semantic::Colophon => "Colophon",
            Semantic::Notes => "Notes",
            Semantic::ListOfIllustrations => "List of Illustrations",
            Semantic::ListOfTables => "List of Tables",
        }
    }

    /// Parses a guide `type`; the legacy synonyms `start`, `title`, `copyright` and `other.*` are understood.
    pub fn from_guide_type(t: &str) -> Option<Semantic> {
        let t = t.trim().to_ascii_lowercase();
        ALL_SEMANTICS
            .iter()
            .copied()
            .find(|s| s.guide_type() == t)
            .or(match t.as_str() {
                "start" | "bodymatter" => Some(Semantic::Bodymatter),
                "titlepage" | "title" => Some(Semantic::TitlePage),
                "copyright" => Some(Semantic::Copyright),
                _ => None,
            })
    }

    pub fn from_epub_type(t: &str) -> Option<Semantic> {
        let t = t.trim().to_ascii_lowercase();
        ALL_SEMANTICS
            .iter()
            .copied()
            .find(|s| s.epub_type() == t)
            .or(if t == "notes" {
                Some(Semantic::Notes)
            } else {
                None
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticEntry {
    pub semantic: Semantic,
    /// Absolute archive path, optionally followed by `#fragment`.
    pub href: String,
    pub title: String,
}

/// Reads the guide references and the landmarks of the book; entries present in both appear once.
pub fn read_semantics(book: &OpenBook) -> Vec<SemanticEntry> {
    let mut out: Vec<SemanticEntry> = Vec::new();
    let mut push = |e: SemanticEntry| {
        if !out
            .iter()
            .any(|x| x.semantic == e.semantic && x.href == e.href)
        {
            out.push(e);
        }
    };
    let opf_dir = book.package.base_dir().to_string();
    if let Some(text) = book.archive.get_string(&book.package.opf_path) {
        if let Ok(doc) = parse_xml(&text) {
            for r in doc
                .descendants()
                .filter(|n| n.is_element() && n.tag_name().name() == "reference")
            {
                let (Some(t), Some(h)) = (r.attribute("type"), r.attribute("href")) else {
                    continue;
                };
                if let Some(semantic) = Semantic::from_guide_type(t) {
                    let (p, f) = resolve_href(&opf_dir, h);
                    push(SemanticEntry {
                        semantic,
                        href: with_frag(p, f),
                        title: r.attribute("title").unwrap_or("").to_string(),
                    });
                }
            }
        }
    }
    if let Some(nav) = book.package.nav_item() {
        let nav_path = book.package.item_path(nav);
        if let Some(src) = book.archive.get_string(&nav_path) {
            if let Some((s, e)) = landmarks_range(&src) {
                let part = &src[s..e];
                let events = scan(part);
                for (k, ev) in events.iter().enumerate() {
                    if let Event::Open { name, range, .. } = ev {
                        if name != "a" {
                            continue;
                        }
                        let tag = &part[range.clone()];
                        let (Some(t), Some(h)) =
                            (tag_attr(tag, "epub:type"), tag_attr(tag, "href"))
                        else {
                            continue;
                        };
                        let title: String = events[k + 1..]
                            .iter()
                            .take_while(|e| !matches!(e, Event::Close { name, .. } if name == "a"))
                            .filter_map(|e| match e {
                                Event::Text(r) => Some(decode_entities(&part[r.clone()])),
                                _ => None,
                            })
                            .collect();
                        if let Some(semantic) = Semantic::from_epub_type(&t) {
                            let (p, f) = resolve_href(parent_dir(&nav_path), &h);
                            push(SemanticEntry {
                                semantic,
                                href: with_frag(p, f),
                                title: title.trim().to_string(),
                            });
                        }
                    }
                }
            }
        }
    }
    out
}

fn with_frag(p: String, f: Option<String>) -> String {
    match f {
        Some(f) => format!("{p}#{f}"),
        None => p,
    }
}

/// Byte range of the `<nav epub:type="landmarks">` element in a navigation document.
fn landmarks_range(src: &str) -> Option<(usize, usize)> {
    let doc = parse_xml(src).ok()?;
    doc.descendants()
        .find(|n| {
            n.is_element()
                && n.tag_name().name() == "nav"
                && n.attributes().any(|a| {
                    a.name() == "type" && a.value().split_whitespace().any(|v| v == "landmarks")
                })
        })
        .map(|n| (n.range().start, n.range().end))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticsTarget {
    pub guide: bool,
    pub landmarks: bool,
}

impl SemanticsTarget {
    /// Guide always, landmarks when the book is EPUB 3 with a navigation document.
    pub fn for_book(book: &OpenBook) -> Self {
        Self {
            guide: true,
            landmarks: book.package.is_epub3() && book.package.nav_item().is_some(),
        }
    }
}

/// Replaces the guide and/or landmarks with `entries` (an empty list removes them). Hrefs are archive paths and
/// must point at existing files.
pub fn write_semantics(
    book: &mut OpenBook,
    entries: &[SemanticEntry],
    target: SemanticsTarget,
) -> Result<()> {
    for e in entries {
        let (p, _) = resolve_href("", &e.href);
        if !book.archive.contains(&p) {
            anyhow::bail!("semantic target {} does not exist", e.href);
        }
    }
    if target.guide {
        let opf = book.package.opf_path.clone();
        edit_opf(book, |t| {
            let t = remove_section(t, "guide");
            if entries.is_empty() {
                return Ok(t);
            }
            let mut g = String::from("  <guide>\n");
            for e in entries {
                g.push_str(&format!(
                    "    <reference type=\"{}\" title=\"{}\" href=\"{}\"/>\n",
                    e.semantic.guide_type(),
                    xml_escape(if e.title.is_empty() {
                        e.semantic.label()
                    } else {
                        &e.title
                    }),
                    xml_escape(&relative_href(&opf, &e.href)),
                ));
            }
            g.push_str("  </guide>\n");
            let at = t
                .rfind("</package>")
                .context("package has no closing tag")?;
            Ok(format!("{}{}{}", &t[..at], g, &t[at..]))
        })?;
    }
    if target.landmarks {
        let nav = book
            .package
            .nav_item()
            .map(|i| book.package.item_path(i))
            .context("the book has no navigation document for landmarks")?;
        let src = book.archive.get_string(&nav).unwrap_or_default();
        let mut base = src.clone();
        if let Some((s, e)) = landmarks_range(&src) {
            let (mut s2, mut e2) = (s, e);
            // swallow the line break and indentation around the old element
            while s2 > 0 && matches!(base.as_bytes()[s2 - 1], b' ' | b'\t') {
                s2 -= 1;
            }
            if base[e2..].starts_with('\n') {
                e2 += 1;
            }
            base.replace_range(s2..e2, "");
        }
        if !entries.is_empty() {
            let mut nav_xml = String::from("  <nav epub:type=\"landmarks\" id=\"landmarks\" hidden=\"hidden\">\n    <h2>Guide</h2>\n    <ol>\n");
            for e in entries {
                nav_xml.push_str(&format!(
                    "      <li><a epub:type=\"{}\" href=\"{}\">{}</a></li>\n",
                    e.semantic.epub_type(),
                    xml_escape(&relative_href(&nav, &e.href)),
                    xml_escape(if e.title.is_empty() {
                        e.semantic.label()
                    } else {
                        &e.title
                    }),
                ));
            }
            nav_xml.push_str("    </ol>\n  </nav>\n");
            let (_, body_end) = body_bounds(&base).context("navigation document has no <body>")?;
            let line_start = base[..body_end]
                .rfind('\n')
                .map(|i| i + 1)
                .unwrap_or(body_end);
            let at = if base[line_start..body_end].trim().is_empty() {
                line_start
            } else {
                body_end
            };
            base.insert_str(
                at,
                &if at == body_end {
                    format!("\n{nav_xml}")
                } else {
                    nav_xml
                },
            );
            if tag_attr_of_root(&base, "xmlns:epub").is_none() {
                base = set_root_attr(&base, "xmlns:epub", "http://www.idpf.org/2007/ops");
            }
        }
        parse_xml(&base).context("internal error: landmarks produced malformed XHTML")?;
        book.archive.set_string(&nav, &base);
    }
    Ok(())
}

fn tag_attr_of_root(src: &str, name: &str) -> Option<String> {
    scan(src).into_iter().find_map(|e| match e {
        Event::Open { name: n, range, .. } if n == "html" => tag_attr(&src[range], name),
        _ => None,
    })
}

fn set_root_attr(src: &str, name: &str, value: &str) -> String {
    set_element_attr(src, "html", name, value)
}

/// Sets (or replaces) the entry for one semantic, keeping all the others. Writes guide and landmarks as the book
/// supports.
pub fn set_semantic(
    book: &mut OpenBook,
    semantic: Semantic,
    href: &str,
    title: &str,
) -> Result<()> {
    let mut entries = read_semantics(book);
    entries.retain(|e| e.semantic != semantic);
    entries.push(SemanticEntry {
        semantic,
        href: href.to_string(),
        title: title.to_string(),
    });
    let target = SemanticsTarget::for_book(book);
    write_semantics(book, &entries, target)
}

/// Removes every entry of `semantic`.
pub fn clear_semantic(book: &mut OpenBook, semantic: Semantic) -> Result<()> {
    let mut entries = read_semantics(book);
    let before = entries.len();
    entries.retain(|e| e.semantic != semantic);
    if entries.len() == before {
        return Ok(());
    }
    let target = SemanticsTarget::for_book(book);
    write_semantics(book, &entries, target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::make_book;

    #[test]
    fn mapping_roundtrips() {
        for s in ALL_SEMANTICS {
            assert_eq!(Semantic::from_guide_type(s.guide_type()), Some(*s), "{s:?}");
            assert_eq!(Semantic::from_epub_type(s.epub_type()), Some(*s), "{s:?}");
            assert!(!s.label().is_empty());
        }
        assert_eq!(
            Semantic::from_guide_type("START"),
            Some(Semantic::Bodymatter)
        );
        assert_eq!(
            Semantic::from_guide_type("title"),
            Some(Semantic::TitlePage)
        );
        assert_eq!(
            Semantic::from_guide_type("copyright"),
            Some(Semantic::Copyright)
        );
        assert_eq!(Semantic::from_guide_type("other.unknown"), None);
        assert_eq!(Semantic::from_epub_type("notes"), Some(Semantic::Notes));
        assert_eq!(Semantic::from_epub_type("weird"), None);
    }

    #[test]
    fn set_and_read_guide_and_landmarks() {
        let mut b = make_book(&["<p>cover</p>", "<p>text</p>"]);
        assert!(read_semantics(&b).is_empty());
        set_semantic(&mut b, Semantic::Cover, "OEBPS/chapter-1.xhtml", "").unwrap();
        set_semantic(
            &mut b,
            Semantic::Bodymatter,
            "OEBPS/chapter-2.xhtml#start",
            "Begin & read",
        )
        .unwrap();
        let sem = read_semantics(&b);
        assert_eq!(sem.len(), 2);
        assert_eq!(sem[0].semantic, Semantic::Cover);
        assert_eq!(sem[0].title, "Cover");
        assert_eq!(sem[1].href, "OEBPS/chapter-2.xhtml#start");
        assert_eq!(sem[1].title, "Begin & read");

        // independent checks of both outputs
        let opf = b.archive.get_string("OEBPS/content.opf").unwrap();
        let doc = roxmltree::Document::parse(&opf).unwrap();
        let refs: Vec<(&str, &str)> = doc
            .descendants()
            .filter(|n| n.has_tag_name("reference"))
            .map(|n| (n.attribute("type").unwrap(), n.attribute("href").unwrap()))
            .collect();
        assert_eq!(
            refs,
            vec![
                ("cover", "chapter-1.xhtml"),
                ("text", "chapter-2.xhtml#start")
            ]
        );
        let nav = b.archive.get_string("OEBPS/nav.xhtml").unwrap();
        let ndoc = crate::testutil::xml(&nav);
        let lm = ndoc
            .descendants()
            .find(|n| {
                n.has_tag_name("nav")
                    && n.attributes()
                        .any(|a| a.name() == "type" && a.value() == "landmarks")
            })
            .unwrap();
        let types: Vec<&str> = lm
            .descendants()
            .filter(|n| n.has_tag_name("a"))
            .map(|n| n.attributes().find(|a| a.name() == "type").unwrap().value())
            .collect();
        assert_eq!(types, vec!["cover", "bodymatter"]);
        // the TOC nav is untouched
        assert!(nav.contains("<nav epub:type=\"toc\" id=\"toc\">"));
        assert!(bookx_core::check::check_book(&b.archive)
            .iter()
            .all(|i| i.severity != bookx_core::check::Severity::Error));
    }

    #[test]
    fn set_replaces_and_clear_removes() {
        let mut b = make_book(&["<p>a</p>", "<p>b</p>"]);
        set_semantic(&mut b, Semantic::Cover, "OEBPS/chapter-1.xhtml", "").unwrap();
        set_semantic(&mut b, Semantic::Cover, "OEBPS/chapter-2.xhtml", "C").unwrap();
        let sem = read_semantics(&b);
        assert_eq!(sem.len(), 1);
        assert_eq!(sem[0].href, "OEBPS/chapter-2.xhtml");
        assert_eq!(
            b.archive
                .get_string("OEBPS/nav.xhtml")
                .unwrap()
                .matches("epub:type=\"landmarks\"")
                .count(),
            1
        );
        // idempotent
        let before = b.archive.get_string("OEBPS/content.opf").unwrap();
        set_semantic(&mut b, Semantic::Cover, "OEBPS/chapter-2.xhtml", "C").unwrap();
        assert_eq!(b.archive.get_string("OEBPS/content.opf").unwrap(), before);
        clear_semantic(&mut b, Semantic::Cover).unwrap();
        assert!(read_semantics(&b).is_empty());
        assert!(!b
            .archive
            .get_string("OEBPS/content.opf")
            .unwrap()
            .contains("<guide"));
        assert!(!b
            .archive
            .get_string("OEBPS/nav.xhtml")
            .unwrap()
            .contains("landmarks"));
        clear_semantic(&mut b, Semantic::Cover).unwrap();
    }

    #[test]
    fn epub2_has_guide_only_and_reads_legacy_types() {
        let mut b = make_book(&["<p>a</p>"]);
        let opf = b.package.opf_path.clone();
        let t = b.archive.get_string(&opf).unwrap().replace("version=\"3.0\"", "version=\"2.0\"").replace(" properties=\"nav\"", "").replace("</package>", "<guide><reference type=\"start\" title=\"S\" href=\"chapter-1.xhtml#x\"/><reference type=\"other.x\" href=\"chapter-1.xhtml\"/></guide></package>");
        b.archive.set_string(&opf, &t);
        b.reload_package().unwrap();
        let sem = read_semantics(&b);
        assert_eq!(sem.len(), 1);
        assert_eq!(sem[0].semantic, Semantic::Bodymatter);
        assert_eq!(sem[0].href, "OEBPS/chapter-1.xhtml#x");
        assert_eq!(
            SemanticsTarget::for_book(&b),
            SemanticsTarget {
                guide: true,
                landmarks: false
            }
        );
        set_semantic(&mut b, Semantic::TitlePage, "OEBPS/chapter-1.xhtml", "").unwrap();
        assert_eq!(read_semantics(&b).len(), 2);
        assert!(!b
            .archive
            .get_string("OEBPS/nav.xhtml")
            .unwrap()
            .contains("landmarks"));
    }

    #[test]
    fn errors() {
        let mut b = make_book(&["<p>a</p>"]);
        let e = set_semantic(&mut b, Semantic::Cover, "OEBPS/missing.xhtml", "").unwrap_err();
        assert_eq!(
            e.to_string(),
            "semantic target OEBPS/missing.xhtml does not exist"
        );
        let e = write_semantics(
            &mut b,
            &[],
            SemanticsTarget {
                guide: false,
                landmarks: true,
            },
        );
        assert!(e.is_ok());
        b.archive.set_string(
            "OEBPS/nav.xhtml",
            "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head/></html>",
        );
        let e = write_semantics(
            &mut b,
            &[SemanticEntry {
                semantic: Semantic::Cover,
                href: "OEBPS/chapter-1.xhtml".into(),
                title: "".into(),
            }],
            SemanticsTarget {
                guide: false,
                landmarks: true,
            },
        )
        .unwrap_err();
        assert!(e.to_string().contains("no <body>"), "{e}");
    }

    #[test]
    fn landmarks_declare_epub_namespace_when_missing() {
        let mut b = make_book(&["<p>a</p>"]);
        b.archive.set_string("OEBPS/nav.xhtml", "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>n</title></head><body><nav id=\"toc\"><ol><li><a href=\"chapter-1.xhtml\">One</a></li></ol></nav></body></html>");
        set_semantic(&mut b, Semantic::Cover, "OEBPS/chapter-1.xhtml", "").unwrap();
        let nav = b.archive.get_string("OEBPS/nav.xhtml").unwrap();
        assert!(nav.contains("xmlns:epub=\"http://www.idpf.org/2007/ops\""));
        crate::testutil::xml(&nav);
        assert_eq!(read_semantics(&b).len(), 1);
    }
}
