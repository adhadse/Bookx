//! Text-level editing helpers for the package document that keep unrelated markup byte-identical.

use anyhow::{Context, Result, bail};
use bookx_core::book::OpenBook;
use bookx_core::opf::{NS_DC, NS_OPF};
use bookx_core::util::{parse_xml, xml_escape};
use regex::Regex;

use crate::markup::*;

/// Applies `f` to the package document text; the result must be well-formed XML. Returns whether the text changed.
/// The package is reloaded afterwards.
pub fn edit_opf(book: &mut OpenBook, f: impl FnOnce(&str) -> Result<String>) -> Result<bool> {
    let path = book.package.opf_path.clone();
    let text = book
        .archive
        .get_string(&path)
        .with_context(|| format!("missing {path}"))?;
    let new = f(&text)?;
    if new == text {
        return Ok(false);
    }
    parse_xml(&new).context("edited package document is not well-formed")?;
    book.archive.set_string(&path, &new);
    book.reload_package()?;
    Ok(true)
}

/// Sets an attribute on the start tag of the first element called `element` (local name).
pub fn set_element_attr(text: &str, element: &str, attr: &str, value: &str) -> String {
    let mut done = false;
    let mut out = String::with_capacity(text.len() + 32);
    let mut last = 0;
    for ev in scan(text) {
        if let Event::Open { name, range, .. } = ev {
            if !done && name == element {
                out.push_str(&text[last..range.start]);
                out.push_str(&set_tag_attr(&text[range.clone()], attr, value));
                last = range.end;
                done = true;
            }
        }
    }
    out.push_str(&text[last..]);
    out
}

/// Prefix bound to the Dublin Core namespace in the package (`dc` if undeclared) and whether it is declared.
pub fn dc_prefix(text: &str) -> (String, bool) {
    if let Ok(doc) = parse_xml(text) {
        for n in doc.descendants().filter(|n| n.is_element()) {
            for ns in n.namespaces() {
                if ns.uri() == NS_DC {
                    if let Some(p) = ns.name() {
                        return (p.to_string(), true);
                    }
                }
            }
        }
    }
    ("dc".into(), false)
}

/// Prefix bound to the OPF namespace for attributes (`opf` if undeclared).
pub fn opf_attr_prefix(text: &str) -> Option<String> {
    let doc = parse_xml(text).ok()?;
    for n in doc.descendants().filter(|n| n.is_element()) {
        for ns in n.namespaces() {
            if ns.uri() == NS_OPF && ns.name().is_some() {
                return ns.name().map(|s| s.to_string());
            }
        }
    }
    None
}

/// Inserts `xml` as the last child of the top-level `<section>` element (e.g. `metadata`, `manifest`, `guide`).
pub fn append_to_section(text: &str, section: &str, xml: &str) -> Result<String> {
    let re = Regex::new(&format!(
        r"</(?:[A-Za-z0-9_]+:)?{}\s*>",
        regex::escape(section)
    ))?;
    let m = re
        .find_iter(text)
        .last()
        .with_context(|| format!("package has no <{section}> element"))?;
    let at = m.start();
    let line_start = text[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let close_indent = &text[line_start..at];
    if close_indent.chars().all(|c| c == ' ' || c == '\t') && line_start > 0 {
        // put the new element on its own line, indented one level deeper than the closing tag
        let indent = format!("{close_indent}  ");
        Ok(format!(
            "{}{}{}\n{}{}",
            &text[..line_start],
            indent,
            xml,
            close_indent,
            &text[at..]
        ))
    } else {
        Ok(format!("{}{}{}", &text[..at], xml, &text[at..]))
    }
}

/// Adds a `<dc:name>` element (prefix and namespace declaration handled).
pub fn add_dc_element(text: &str, name: &str, attrs: &str, value: &str) -> Result<String> {
    let (p, declared) = dc_prefix(text);
    let decl = if declared {
        String::new()
    } else {
        format!(" xmlns:{p}=\"{NS_DC}\"")
    };
    append_to_section(
        text,
        "metadata",
        &format!(
            "<{p}:{name}{decl}{attrs}>{}</{p}:{name}>",
            xml_escape(value)
        ),
    )
}

/// Changes the `properties` attribute of the manifest item `id` with `f` (old value -> new value, empty removes it).
pub fn edit_item_properties(text: &str, id: &str, f: impl Fn(&str) -> String) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut last = 0;
    for ev in scan(text) {
        if let Event::Open { name, range, .. } = ev {
            if name == "item" {
                let tag = &text[range.clone()];
                if tag_attr(tag, "id").as_deref() == Some(id) {
                    let old = tag_attr(tag, "properties").unwrap_or_default();
                    let new = f(&old);
                    if new != old {
                        out.push_str(&text[last..range.start]);
                        out.push_str(&if new.is_empty() {
                            remove_tag_attr(tag, "properties")
                        } else {
                            set_tag_attr(tag, "properties", &new)
                        });
                        last = range.end;
                    }
                }
            }
        }
    }
    out.push_str(&text[last..]);
    out
}

/// Adds a space-separated token to `properties` (no-op if present).
pub fn add_property(old: &str, prop: &str) -> String {
    if old.split_whitespace().any(|p| p == prop) {
        old.to_string()
    } else if old.trim().is_empty() {
        prop.to_string()
    } else {
        format!("{} {prop}", old.trim())
    }
}

/// Removes the `<guide>` element entirely.
pub fn remove_section(text: &str, section: &str) -> String {
    let re = Regex::new(&format!(
        r"(?s)[ \t]*<(?:[A-Za-z0-9_]+:)?{s}\b[^>]*?(?:/>|>.*?</(?:[A-Za-z0-9_]+:)?{s}\s*>)[ \t]*\r?\n?",
        s = regex::escape(section)
    ))
    .unwrap();
    re.replace(text, "").into_owned()
}

/// Escapes text for element content.
pub fn esc(s: &str) -> String {
    xml_escape(s)
}

/// Fails with a clear message unless the book has an EPUB 3 package.
pub fn require_epub3(book: &OpenBook) -> Result<()> {
    if !book.package.is_epub3() {
        bail!(
            "this operation needs an EPUB 3 package (version {})",
            book.package.version
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPF: &str = "<?xml version=\"1.0\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"2.0\" unique-identifier=\"id\">\n  <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n    <dc:title>T</dc:title>\n  </metadata>\n  <manifest>\n    <item id=\"a\" href=\"a.xhtml\" media-type=\"application/xhtml+xml\"/>\n  </manifest>\n  <spine toc=\"ncx\"><itemref idref=\"a\"/></spine>\n  <guide>\n    <reference type=\"text\" href=\"a.xhtml\"/>\n  </guide>\n</package>\n";

    #[test]
    fn set_element_attr_first_only() {
        let t = set_element_attr(OPF, "package", "version", "3.0");
        assert!(t.contains("version=\"3.0\" unique-identifier"));
        let t = set_element_attr(OPF, "spine", "page-progression-direction", "rtl");
        assert!(t.contains("<spine toc=\"ncx\" page-progression-direction=\"rtl\">"));
        assert_eq!(set_element_attr(OPF, "nothing", "a", "b"), OPF);
    }

    #[test]
    fn prefixes() {
        assert_eq!(dc_prefix(OPF), ("dc".to_string(), true));
        assert_eq!(
            dc_prefix("<package><metadata/></package>"),
            ("dc".to_string(), false)
        );
        assert_eq!(
            dc_prefix(
                "<package xmlns:d=\"http://purl.org/dc/elements/1.1/\"><metadata/></package>"
            )
            .0,
            "d"
        );
        assert_eq!(opf_attr_prefix(OPF), None);
        assert_eq!(
            opf_attr_prefix("<package xmlns:o=\"http://www.idpf.org/2007/opf\"/>").as_deref(),
            Some("o")
        );
    }

    #[test]
    fn append_and_dc_element() {
        let t = add_dc_element(OPF, "language", "", "en & de").unwrap();
        assert!(t.contains("    <dc:title>T</dc:title>\n    <dc:language>en &amp; de</dc:language>\n  </metadata>"), "{t}");
        parse_xml(&t).unwrap();
        let t2 = add_dc_element(
            "<package><metadata></metadata></package>",
            "title",
            " id=\"x\"",
            "A",
        )
        .unwrap();
        assert_eq!(
            t2,
            "<package><metadata><dc:title xmlns:dc=\"http://purl.org/dc/elements/1.1/\" id=\"x\">A</dc:title></metadata></package>"
        );
        assert!(append_to_section(OPF, "nonexistent", "<x/>").is_err());
        let g =
            append_to_section(OPF, "guide", "<reference type=\"toc\" href=\"t.xhtml\"/>").unwrap();
        assert!(g.contains(
            "    <reference type=\"text\" href=\"a.xhtml\"/>\n    <reference type=\"toc\""
        ));
    }

    #[test]
    fn item_properties() {
        let t = edit_item_properties(OPF, "a", |o| add_property(o, "scripted"));
        assert!(t.contains("<item id=\"a\" href=\"a.xhtml\" media-type=\"application/xhtml+xml\" properties=\"scripted\"/>"));
        let t2 = edit_item_properties(&t, "a", |o| add_property(o, "svg"));
        assert!(t2.contains("properties=\"scripted svg\""));
        assert_eq!(
            edit_item_properties(&t2, "a", |o| add_property(o, "svg")),
            t2
        );
        let t3 = edit_item_properties(&t2, "a", |_| String::new());
        assert_eq!(t3, OPF);
        assert_eq!(edit_item_properties(OPF, "zz", |_| "x".into()), OPF);
        assert_eq!(add_property("nav", "nav"), "nav");
    }

    #[test]
    fn remove_section_variants() {
        let t = remove_section(OPF, "guide");
        assert!(!t.contains("guide"));
        parse_xml(&t).unwrap();
        assert!(t.contains("</spine>\n</package>"));
        assert_eq!(
            remove_section("<package><guide/></package>", "guide"),
            "<package></package>"
        );
        assert_eq!(remove_section(OPF, "none"), OPF);
    }

    #[test]
    fn edit_opf_validates_and_reports_changes() {
        let mut b = crate::testutil::make_book(&["<p>x</p>"]);
        let before = b.archive.get_string(&b.package.opf_path).unwrap();
        assert!(
            !edit_opf(&mut b, |t| Ok(t.to_string())).unwrap(),
            "unchanged text reports false"
        );
        assert!(
            edit_opf(&mut b, |t| Ok(set_element_attr(
                t,
                "spine",
                "page-progression-direction",
                "rtl"
            )))
            .unwrap()
        );
        assert!(
            b.archive
                .get_string(&b.package.opf_path)
                .unwrap()
                .contains("page-progression-direction=\"rtl\"")
        );
        let now = b.archive.get_string(&b.package.opf_path).unwrap();
        let e = edit_opf(&mut b, |t| Ok(t.replace("</package>", ""))).unwrap_err();
        assert!(e.to_string().contains("not well-formed"), "{e}");
        assert_eq!(
            b.archive.get_string(&b.package.opf_path).unwrap(),
            now,
            "a broken edit is never stored"
        );
        let e = edit_opf(&mut b, |_| anyhow::bail!("nope")).unwrap_err();
        assert_eq!(e.to_string(), "nope");
        assert_ne!(before, now);
    }

    #[test]
    fn require_epub3_message() {
        let mut b = crate::testutil::make_book(&["<p>x</p>"]);
        assert!(require_epub3(&b).is_ok());
        let opf = b.package.opf_path.clone();
        let t = b
            .archive
            .get_string(&opf)
            .unwrap()
            .replace("version=\"3.0\"", "version=\"2.0\"");
        b.archive.set_string(&opf, &t);
        b.reload_package().unwrap();
        assert_eq!(
            require_epub3(&b).unwrap_err().to_string(),
            "this operation needs an EPUB 3 package (version 2.0)"
        );
        assert_eq!(esc("a<b"), "a&lt;b");
    }
}
