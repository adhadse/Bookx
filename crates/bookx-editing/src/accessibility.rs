//! "Make accessible" (E19): language attributes, DPUB-ARIA roles, landmarks, table header hints, an alt-text report
//! and schema.org accessibility metadata.

use std::collections::HashSet;

use anyhow::Result;
use bookx_core::book::OpenBook;
use bookx_core::util::{extension, file_name, parse_xml, xml_escape};
use serde::{Deserialize, Serialize};

use crate::markup::*;
use crate::opf_edit::*;
use crate::semantics::{read_semantics, write_semantics, Semantic, SemanticEntry, SemanticsTarget};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AltKind {
    Missing,
    Empty,
    LooksLikeFilename,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AltIssue {
    pub file: String,
    pub src: String,
    pub kind: AltKind,
}

#[derive(Debug, Clone)]
pub struct AccessibilityOptions {
    pub summary: Option<String>,
    pub add_metadata: bool,
    pub add_landmarks: bool,
    /// Give images without `alt` an empty one (decorative) instead of only reporting them.
    pub mark_missing_alt_decorative: bool,
}

impl Default for AccessibilityOptions {
    fn default() -> Self {
        Self {
            summary: None,
            add_metadata: true,
            add_landmarks: true,
            mark_missing_alt_decorative: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccessibilityReport {
    pub changes: Vec<String>,
    pub alt_issues: Vec<AltIssue>,
    pub table_issues: Vec<String>,
}

/// DPUB-ARIA role for an `epub:type` token.
pub fn epub_type_to_role(t: &str) -> Option<&'static str> {
    Some(match t {
        "abstract" => "doc-abstract",
        "acknowledgements" => "doc-acknowledgments",
        "afterword" => "doc-afterword",
        "appendix" => "doc-appendix",
        "backlink" => "doc-backlink",
        "biblioentry" => "doc-biblioentry",
        "bibliography" => "doc-bibliography",
        "biblioref" => "doc-biblioref",
        "chapter" => "doc-chapter",
        "colophon" => "doc-colophon",
        "conclusion" => "doc-conclusion",
        "cover" => "doc-cover",
        "credit" => "doc-credit",
        "credits" => "doc-credits",
        "dedication" => "doc-dedication",
        "endnote" => "doc-endnote",
        "endnotes" => "doc-endnotes",
        "epigraph" => "doc-epigraph",
        "epilogue" => "doc-epilogue",
        "errata" => "doc-errata",
        "example" => "doc-example",
        "footnote" => "doc-footnote",
        "foreword" => "doc-foreword",
        "glossary" => "doc-glossary",
        "glossref" => "doc-glossref",
        "index" => "doc-index",
        "introduction" => "doc-introduction",
        "noteref" => "doc-noteref",
        "notice" => "doc-notice",
        "pagebreak" => "doc-pagebreak",
        "page-list" => "doc-pagelist",
        "part" => "doc-part",
        "preface" => "doc-preface",
        "prologue" => "doc-prologue",
        "pullquote" => "doc-pullquote",
        "qna" => "doc-qna",
        "subtitle" => "doc-subtitle",
        "tip" => "doc-tip",
        "toc" => "doc-toc",
        _ => return None,
    })
}

fn looks_like_filename(alt: &str, src: &str) -> bool {
    let a = alt.trim().to_ascii_lowercase();
    if a.is_empty() {
        return false;
    }
    let f = file_name(src).to_ascii_lowercase();
    let stem = f.rsplit_once('.').map(|(s, _)| s).unwrap_or(&f);
    a == f
        || a == stem
        || matches!(
            extension(&a).as_str(),
            "jpg" | "jpeg" | "png" | "gif" | "svg" | "webp"
        )
}

/// Images whose alternative text is missing, empty or just the file name. Empty alt is reported as `Empty`
/// (it may be intentional for decorative images, the editor decides).
pub fn alt_text_report(book: &OpenBook) -> Vec<AltIssue> {
    let mut out = Vec::new();
    for file in markup_files(&book.archive) {
        let Some(src) = book.archive.get_string(&file) else {
            continue;
        };
        for ev in scan(&src) {
            if let Event::Open { name, range, .. } = ev {
                if name != "img" {
                    continue;
                }
                let tag = &src[range];
                let img_src = tag_attr(tag, "src").unwrap_or_default();
                let kind = match tag_attr(tag, "alt") {
                    None => Some(AltKind::Missing),
                    Some(a) if a.trim().is_empty() => Some(AltKind::Empty),
                    Some(a) if looks_like_filename(&a, &img_src) => {
                        Some(AltKind::LooksLikeFilename)
                    }
                    _ => None,
                };
                if let Some(kind) = kind {
                    out.push(AltIssue {
                        file: file.clone(),
                        src: img_src,
                        kind,
                    });
                }
            }
        }
    }
    out
}

struct TableState {
    rows: usize,
    in_thead: bool,
    has_th: bool,
    has_caption: bool,
}

/// Processes one document; returns the new source and counters.
fn process_document(
    src: &str,
    lang: &str,
    opts: &AccessibilityOptions,
    tables_report: &mut Vec<String>,
    file: &str,
) -> (String, Vec<String>) {
    let mut out = String::with_capacity(src.len() + 256);
    let mut last = 0;
    let mut changes = Vec::new();
    let mut tables: Vec<TableState> = Vec::new();
    let (mut roles, mut scopes, mut alts, mut langs) = (0, 0, 0, 0);
    let mut seen_root = false;
    for ev in scan(src) {
        match ev {
            Event::Open {
                name,
                range,
                self_closing,
            } => {
                let tag = &src[range.clone()];
                let mut new_tag = tag.to_string();
                if name == "html" && !seen_root {
                    seen_root = true;
                    if !lang.is_empty() {
                        if tag_attr(&new_tag, "lang").is_none() {
                            new_tag = set_tag_attr(&new_tag, "lang", lang);
                            langs += 1;
                        }
                        if tag_attr(&new_tag, "xml:lang").is_none() {
                            new_tag = set_tag_attr(&new_tag, "xml:lang", lang);
                            langs += 1;
                        }
                    }
                }
                if let Some(et) = tag_attr(&new_tag, "epub:type") {
                    if tag_attr(&new_tag, "role").is_none() {
                        if let Some(role) = et.split_whitespace().find_map(epub_type_to_role) {
                            new_tag = set_tag_attr(&new_tag, "role", role);
                            roles += 1;
                        }
                    }
                }
                match name.as_str() {
                    "table" => {
                        tables.push(TableState {
                            rows: 0,
                            in_thead: false,
                            has_th: false,
                            has_caption: false,
                        });
                        if self_closing {
                            tables.pop();
                        }
                    }
                    "thead" => {
                        if let Some(t) = tables.last_mut() {
                            t.in_thead = true;
                        }
                    }
                    "tr" => {
                        if let Some(t) = tables.last_mut() {
                            t.rows += 1;
                        }
                    }
                    "caption" => {
                        if let Some(t) = tables.last_mut() {
                            t.has_caption = true;
                        }
                    }
                    "th" => {
                        if let Some(t) = tables.last_mut() {
                            t.has_th = true;
                            if tag_attr(&new_tag, "scope").is_none()
                                && tag_attr(&new_tag, "id").is_none()
                            {
                                let scope = if t.in_thead || t.rows <= 1 {
                                    "col"
                                } else {
                                    "row"
                                };
                                new_tag = set_tag_attr(&new_tag, "scope", scope);
                                scopes += 1;
                            }
                        }
                    }
                    "img" => {
                        if opts.mark_missing_alt_decorative && tag_attr(&new_tag, "alt").is_none() {
                            new_tag = set_tag_attr(&new_tag, "alt", "");
                            alts += 1;
                        }
                    }
                    _ => {}
                }
                if new_tag != tag {
                    out.push_str(&src[last..range.start]);
                    out.push_str(&new_tag);
                    last = range.end;
                }
            }
            Event::Close { name, .. } => match name.as_str() {
                "thead" => {
                    if let Some(t) = tables.last_mut() {
                        t.in_thead = false;
                    }
                }
                "table" => {
                    if let Some(t) = tables.pop() {
                        if !t.has_th {
                            tables_report.push(format!(
                                "{file}: table without header cells (<th>){}",
                                if t.has_caption {
                                    ""
                                } else {
                                    " and without caption"
                                }
                            ));
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    out.push_str(&src[last..]);
    if langs > 0 {
        changes.push(format!(
            "{file}: added {langs} language attribute(s) on <html>"
        ));
    }
    if roles > 0 {
        changes.push(format!("{file}: added {roles} DPUB-ARIA role(s)"));
    }
    if scopes > 0 {
        changes.push(format!(
            "{file}: added scope to {scopes} table header cell(s)"
        ));
    }
    if alts > 0 {
        changes.push(format!(
            "{file}: marked {alts} image(s) as decorative (alt=\"\")"
        ));
    }
    (out, changes)
}

fn has_meta(opf: &str, property: &str, value: Option<&str>) -> bool {
    parse_xml(opf)
        .map(|d| {
            d.descendants().any(|n| {
                n.is_element()
                    && n.tag_name().name() == "meta"
                    && n.attribute("property") == Some(property)
                    && value.is_none_or(|v| n.text().map(str::trim) == Some(v))
            })
        })
        .unwrap_or(false)
}

/// Improves the accessibility of the book. Safe to run repeatedly.
pub fn make_accessible(
    book: &mut OpenBook,
    opts: &AccessibilityOptions,
) -> Result<AccessibilityReport> {
    let mut rep = AccessibilityReport::default();
    let lang = book.package.metadata.language.clone();
    let xhtml: Vec<String> = book
        .package
        .manifest
        .iter()
        .filter(|i| i.media_type == "application/xhtml+xml")
        .map(|i| book.package.item_path(i))
        .collect();
    if opts.add_landmarks && book.package.is_epub3() && book.package.nav_item().is_some() {
        let mut entries = read_semantics(book);
        let have: HashSet<Semantic> = entries.iter().map(|e| e.semantic).collect();
        let nav_path = book.package.nav_item().map(|i| book.package.item_path(i));
        let spine = book.spine_paths();
        let looks_cover = |p: &str| file_name(p).to_ascii_lowercase().contains("cover");
        let mut added: Vec<&str> = Vec::new();
        if !have.contains(&Semantic::Toc) {
            if let Some(n) = &nav_path {
                entries.push(SemanticEntry {
                    semantic: Semantic::Toc,
                    href: n.clone(),
                    title: String::new(),
                });
                added.push("toc");
            }
        }
        if !have.contains(&Semantic::Cover) {
            if let Some(c) = spine.iter().find(|p| looks_cover(p)) {
                entries.push(SemanticEntry {
                    semantic: Semantic::Cover,
                    href: c.clone(),
                    title: String::new(),
                });
                added.push("cover");
            }
        }
        if !have.contains(&Semantic::Bodymatter) {
            if let Some(s) = spine.iter().find(|p| !looks_cover(p)) {
                entries.push(SemanticEntry {
                    semantic: Semantic::Bodymatter,
                    href: s.clone(),
                    title: String::new(),
                });
                added.push("bodymatter");
            }
        }
        if !added.is_empty() {
            write_semantics(
                book,
                &entries,
                SemanticsTarget {
                    guide: false,
                    landmarks: true,
                },
            )?;
            rep.changes
                .push(format!("Added landmarks: {}", added.join(", ")));
        }
    }

    for file in &xhtml {
        let Some(src) = book.archive.get_string(file) else {
            continue;
        };
        let (new, changes) = process_document(&src, &lang, opts, &mut rep.table_issues, file);
        if new != src && (parse_xml(&new).is_ok() || parse_xml(&src).is_err()) {
            book.archive.set_string(file, &new);
            rep.changes.extend(changes);
        }
    }
    rep.alt_issues = alt_text_report(book);

    if opts.add_metadata && book.package.is_epub3() {
        let mut props: Vec<(&str, String)> = Vec::new();
        let has_images = book.package.manifest.iter().any(|i| {
            i.media_type.starts_with("image/")
                && book.package.cover_id.as_deref().is_none_or(|c| c != i.id)
        }) || xhtml.iter().any(|f| {
            book.archive
                .get_string(f)
                .is_some_and(|s| s.contains("<img") || s.contains("<svg"))
        });
        props.push(("schema:accessMode", "textual".into()));
        if has_images {
            props.push(("schema:accessMode", "visual".into()));
        }
        props.push(("schema:accessModeSufficient", "textual".into()));
        props.push(("schema:accessibilityFeature", "readingOrder".into()));
        if book.package.nav_item().is_some() {
            props.push(("schema:accessibilityFeature", "tableOfContents".into()));
            props.push(("schema:accessibilityFeature", "structuralNavigation".into()));
        }
        if has_images && rep.alt_issues.iter().all(|i| i.kind == AltKind::Empty) {
            props.push(("schema:accessibilityFeature", "alternativeText".into()));
        }
        props.push(("schema:accessibilityHazard", "unknown".into()));
        let mut added = Vec::new();
        edit_opf(book, |t| {
            let mut t = t.to_string();
            for (p, v) in &props {
                if !has_meta(&t, p, Some(v)) {
                    t = append_to_section(
                        &t,
                        "metadata",
                        &format!("<meta property=\"{p}\">{}</meta>", xml_escape(v)),
                    )?;
                    added.push(format!("{p}={v}"));
                }
            }
            if !has_meta(&t, "schema:accessibilitySummary", None) {
                let s = opts.summary.clone().unwrap_or_else(|| {
                    "This publication has a navigable table of contents and a logical reading order.".to_string()
                });
                t = append_to_section(
                    &t,
                    "metadata",
                    &format!(
                        "<meta property=\"schema:accessibilitySummary\">{}</meta>",
                        xml_escape(&s)
                    ),
                )?;
                added.push("schema:accessibilitySummary".into());
            }
            Ok(t)
        })?;
        if !added.is_empty() {
            rep.changes.push(format!(
                "Added accessibility metadata: {}",
                added.join(", ")
            ));
        }
    }
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::make_book;
    use bookx_core::check::{check_book, Severity};

    fn book_with_content() -> OpenBook {
        let mut b = make_book(&[
            "<h1>One</h1><p epub:type=\"noteref\">n</p><section epub:type=\"chapter bodymatter\"><p>x</p></section><table><tr><th>A</th><th id=\"x\">B</th></tr><tr><th>r</th><td>1</td></tr></table><table><caption>c</caption><thead><tr><th>H</th></tr></thead><tr><td>1</td></tr></table><table><tr><td>plain</td></tr></table><img src=\"a.png\"/><img src=\"b.png\" alt=\"\"/><img src=\"c.png\" alt=\"c.png\"/><img src=\"d.png\" alt=\"A cat\"/>",
            "<p>two</p>",
        ]);
        let t = b
            .archive
            .get_string("OEBPS/chapter-1.xhtml")
            .unwrap()
            .replace(
                "<html ",
                "<html xmlns:epub=\"http://www.idpf.org/2007/ops\" ",
            )
            .replace(" lang=\"en\"", "");
        b.archive.set_string("OEBPS/chapter-1.xhtml", &t);
        b
    }

    #[test]
    fn role_mapping() {
        assert_eq!(epub_type_to_role("noteref"), Some("doc-noteref"));
        assert_eq!(
            epub_type_to_role("acknowledgements"),
            Some("doc-acknowledgments")
        );
        assert_eq!(epub_type_to_role("page-list"), Some("doc-pagelist"));
        assert_eq!(epub_type_to_role("bodymatter"), None);
    }

    #[test]
    fn alt_report_kinds() {
        let b = book_with_content();
        let r = alt_text_report(&b);
        let kinds: Vec<(&str, AltKind)> = r.iter().map(|i| (i.src.as_str(), i.kind)).collect();
        assert_eq!(
            kinds,
            vec![
                ("a.png", AltKind::Missing),
                ("b.png", AltKind::Empty),
                ("c.png", AltKind::LooksLikeFilename)
            ]
        );
        assert!(looks_like_filename("IMG_0001", "x/IMG_0001.jpg"));
        assert!(looks_like_filename("photo.JPG", "x/other.png"));
        assert!(!looks_like_filename("A photo of a dog", "dog.jpg"));
        assert!(!looks_like_filename("", "dog.jpg"));
    }

    #[test]
    fn document_changes_languages_roles_scopes() {
        let mut b = book_with_content();
        let rep = make_accessible(&mut b, &AccessibilityOptions::default()).unwrap();
        let t = b.archive.get_string("OEBPS/chapter-1.xhtml").unwrap();
        let doc = roxmltree::Document::parse_with_options(
            &t,
            roxmltree::ParsingOptions {
                allow_dtd: true,
                ..Default::default()
            },
        )
        .unwrap();
        let root = doc.root_element();
        assert_eq!(root.attribute("lang"), Some("en"));
        assert_eq!(
            root.attribute(("http://www.w3.org/XML/1998/namespace", "lang")),
            Some("en")
        );
        let role_of = |ty: &str| {
            doc.descendants()
                .find(|n| {
                    n.attributes().any(|a| {
                        a.name() == "type" && a.value().split_whitespace().any(|v| v == ty)
                    })
                })
                .and_then(|n| n.attribute("role"))
        };
        assert_eq!(role_of("noteref"), Some("doc-noteref"));
        assert_eq!(role_of("chapter"), Some("doc-chapter"));
        let scopes: Vec<(String, Option<&str>)> = doc
            .descendants()
            .filter(|n| n.has_tag_name("th"))
            .map(|n| (n.text().unwrap().to_string(), n.attribute("scope")))
            .collect();
        assert_eq!(
            scopes,
            vec![
                ("A".into(), Some("col")),
                ("B".into(), None),
                ("r".into(), Some("row")),
                ("H".into(), Some("col"))
            ]
        );
        assert_eq!(rep.table_issues.len(), 1);
        assert!(
            rep.table_issues[0].contains("without header cells")
                && rep.table_issues[0].contains("without caption")
        );
        assert_eq!(rep.alt_issues.len(), 3);
        assert!(rep.changes.iter().any(|c| c.contains("DPUB-ARIA")));
        assert!(rep.changes.iter().any(|c| c.contains("scope")));
        let c2: Vec<&String> = rep
            .changes
            .iter()
            .filter(|c| c.contains("chapter-2"))
            .collect();
        assert_eq!(c2.len(), 1);
        assert!(
            c2[0].contains("language"),
            "chapter 2 only needs xml:lang: {c2:?}"
        );
        assert!(check_book(&b.archive)
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .all(|i| i.message.contains("Broken link")));
    }

    #[test]
    fn metadata_and_landmarks_added_once() {
        let mut b = book_with_content();
        let opts = AccessibilityOptions {
            summary: Some("Tagged & navigable".into()),
            ..Default::default()
        };
        let rep = make_accessible(&mut b, &opts).unwrap();
        assert!(rep
            .changes
            .iter()
            .any(|c| c.starts_with("Added accessibility metadata")));
        assert!(rep
            .changes
            .iter()
            .any(|c| c == "Added landmarks: toc, bodymatter"));
        let opf = b.archive.get_string(&b.package.opf_path).unwrap();
        let doc = roxmltree::Document::parse(&opf).unwrap();
        let metas: Vec<(String, String)> = doc
            .descendants()
            .filter(|n| n.has_tag_name("meta"))
            .filter_map(|n| Some((n.attribute("property")?.to_string(), n.text()?.to_string())))
            .collect();
        for want in [
            ("schema:accessMode", "textual"),
            ("schema:accessMode", "visual"),
            ("schema:accessModeSufficient", "textual"),
            ("schema:accessibilityFeature", "structuralNavigation"),
            ("schema:accessibilityFeature", "tableOfContents"),
            ("schema:accessibilityFeature", "readingOrder"),
            ("schema:accessibilityHazard", "unknown"),
            ("schema:accessibilitySummary", "Tagged & navigable"),
        ] {
            assert!(
                metas.contains(&(want.0.to_string(), want.1.to_string())),
                "{want:?} in {metas:?}"
            );
        }
        assert!(
            !metas.contains(&(
                "schema:accessibilityFeature".into(),
                "alternativeText".into()
            )),
            "images lack alt text"
        );
        let nav = b.archive.get_string("OEBPS/nav.xhtml").unwrap();
        assert!(nav.contains("epub:type=\"landmarks\""));
        // idempotent
        let snapshot = b.archive.clone();
        let again = make_accessible(&mut b, &opts).unwrap();
        assert!(again.changes.is_empty(), "{:?}", again.changes);
        for n in snapshot.names() {
            assert_eq!(snapshot.get(n), b.archive.get(n), "{n}");
        }
    }

    #[test]
    fn decorative_marking_and_alternative_text_feature() {
        let mut b = make_book(&["<p><img src=\"a.png\"/><img src=\"b.png\" alt=\"Bee\"/></p>"]);
        let rep = make_accessible(
            &mut b,
            &AccessibilityOptions {
                mark_missing_alt_decorative: true,
                add_landmarks: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(rep
            .changes
            .iter()
            .any(|c| c.contains("marked 1 image(s) as decorative")));
        assert!(b
            .archive
            .get_string("OEBPS/chapter-1.xhtml")
            .unwrap()
            .contains("<img src=\"a.png\" alt=\"\"/>"));
        assert_eq!(rep.alt_issues.len(), 1);
        assert_eq!(rep.alt_issues[0].kind, AltKind::Empty);
        let opf = b.archive.get_string(&b.package.opf_path).unwrap();
        assert!(opf.contains(">alternativeText<"));
        assert!(!b
            .archive
            .get_string("OEBPS/nav.xhtml")
            .unwrap()
            .contains("landmarks"));
    }

    #[test]
    fn epub2_gets_no_epub3_metadata_or_landmarks() {
        let mut b = make_book(&["<p>x</p>"]);
        let opf = b.package.opf_path.clone();
        let t = b
            .archive
            .get_string(&opf)
            .unwrap()
            .replace("version=\"3.0\"", "version=\"2.0\"");
        b.archive.set_string(&opf, &t);
        b.reload_package().unwrap();
        let rep = make_accessible(&mut b, &AccessibilityOptions::default()).unwrap();
        assert!(!b.archive.get_string(&opf).unwrap().contains("schema:"));
        assert!(!rep.changes.iter().any(|c| c.contains("landmarks")));
    }

    #[test]
    fn cover_landmark_detected() {
        let mut b = make_book(&["<p>c</p>", "<p>t</p>"]);
        b.archive
            .rename("OEBPS/chapter-1.xhtml", "OEBPS/cover.xhtml");
        let opf = b.package.opf_path.clone();
        let t = b
            .archive
            .get_string(&opf)
            .unwrap()
            .replace("chapter-1.xhtml", "cover.xhtml");
        b.archive.set_string(&opf, &t);
        b.reload_package().unwrap();
        let rep = make_accessible(
            &mut b,
            &AccessibilityOptions {
                add_metadata: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            rep.changes
                .iter()
                .any(|c| c == "Added landmarks: toc, cover, bodymatter"),
            "{:?}",
            rep.changes
        );
        let sem = read_semantics(&b);
        assert_eq!(
            sem.iter()
                .find(|e| e.semantic == Semantic::Bodymatter)
                .unwrap()
                .href,
            "OEBPS/chapter-2.xhtml"
        );
    }
}
