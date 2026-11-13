//! Conservative XHTML pretty printer ("beautify") for the book editor.
//!
//! Only block containers that hold no text of their own are re-indented; anything
//! with mixed content is kept verbatim so reading text is never altered.

use anyhow::Result;
use roxmltree::Node;

use super::util::parse_xml;

const CONTAINERS: &[&str] = &[
    "html",
    "head",
    "body",
    "div",
    "section",
    "article",
    "aside",
    "nav",
    "header",
    "footer",
    "main",
    "ul",
    "ol",
    "dl",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "blockquote",
    "figure",
    "package",
    "metadata",
    "manifest",
    "spine",
    "guide",
    "ncx",
    "navMap",
    "navPoint",
    "svg",
    "container",
    "rootfiles",
];

fn is_container(n: Node) -> bool {
    CONTAINERS.contains(&n.tag_name().name())
        && !n
            .children()
            .any(|c| c.is_text() && !c.text().unwrap_or("").trim().is_empty())
}

fn emit(n: Node, text: &str, depth: usize, indent: &str, out: &mut String) {
    let pad = indent.repeat(depth);
    if !n.is_element() {
        if n.is_comment() || n.is_pi() {
            out.push_str(&pad);
            out.push_str(text[n.range()].trim());
            out.push('\n');
        }
        return;
    }
    let r = n.range();
    if !is_container(n) {
        out.push_str(&pad);
        out.push_str(text[r].trim());
        out.push('\n');
        return;
    }
    let kids: Vec<Node> = n
        .children()
        .filter(|c| c.is_element() || c.is_comment() || c.is_pi())
        .collect();
    if kids.is_empty() {
        out.push_str(&pad);
        out.push_str(text[r].trim());
        out.push('\n');
        return;
    }
    let first = kids.first().unwrap().range().start;
    let last = kids.last().unwrap().range().end;
    out.push_str(&pad);
    out.push_str(text[r.start..first].trim());
    out.push('\n');
    for k in kids {
        emit(k, text, depth + 1, indent, out);
    }
    out.push_str(&pad);
    out.push_str(text[last..r.end].trim());
    out.push('\n');
}

pub fn pretty_print_xml(text: &str) -> Result<String> {
    let doc = parse_xml(text)?;
    let root = doc.root_element();
    let r = root.range();
    let mut out = String::new();
    let prefix = text[..r.start].trim_end();
    if !prefix.is_empty() {
        out.push_str(prefix.trim_start_matches('\u{feff}'));
        out.push('\n');
    }
    emit(root, text, 0, "  ", &mut out);
    let suffix = text[r.end..].trim();
    if !suffix.is_empty() {
        out.push_str(suffix);
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indents_containers_only() {
        let src = "<?xml version=\"1.0\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>T</title></head><body><div><p>Hello <b>world</b></p></div></body></html>";
        let out = pretty_print_xml(src).unwrap();
        assert!(out.contains("\n  <head>\n    <title>T</title>\n  </head>\n"));
        assert!(out.contains("      <p>Hello <b>world</b></p>\n"));
        assert!(parse_xml(&out).is_ok());
    }
}
