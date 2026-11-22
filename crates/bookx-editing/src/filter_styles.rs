//! "Filter style information": remove chosen CSS properties from the whole book.

use std::collections::BTreeMap;

use bookx_convert::transforms::css::{CssItem, parse_css};
use bookx_core::book::OpenBook;
use serde::{Deserialize, Serialize};

use crate::markup::*;
use crate::transform_css::{
    CssTransform, split_declarations, transform_markup_styles, transform_stylesheet,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterSpec {
    /// Property names to remove. A trailing `*` matches a prefix (`-webkit-*`, `margin*`).
    pub properties: Vec<String>,
    /// Drop rules that have no declarations left.
    pub remove_empty_rules: bool,
    /// Drop `style=""` attributes that have no declarations left.
    pub remove_empty_style_attrs: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterReport {
    pub declarations_removed: usize,
    pub files: BTreeMap<String, usize>,
    pub empty_rules_removed: usize,
    pub empty_style_attrs_removed: usize,
}

fn declaration_names(inner: &str, out: &mut BTreeMap<String, usize>) {
    for seg in split_declarations(inner) {
        let body = seg.strip_suffix(';').unwrap_or(seg);
        if let Some((n, _)) = body.split_once(':') {
            let n = n.trim().to_ascii_lowercase();
            if !n.is_empty() && !n.contains(['{', '/', ' ']) {
                *out.entry(n).or_default() += 1;
            }
        }
    }
}

fn collect_css(css: &str, out: &mut BTreeMap<String, usize>) {
    for item in parse_css(css) {
        match item {
            CssItem::Rule { block, .. } => declaration_names(
                block.trim().trim_start_matches('{').trim_end_matches('}'),
                out,
            ),
            CssItem::At(s) => {
                if let (Some(o), Some(c)) = (s.find('{'), s.rfind('}')) {
                    let inner = &s[o + 1..c];
                    let lower = s.to_ascii_lowercase();
                    if lower.starts_with("@media") || lower.starts_with("@supports") {
                        collect_css(inner, out);
                    } else if lower.starts_with("@font-face") || lower.starts_with("@page") {
                        declaration_names(inner, out);
                    }
                }
            }
            CssItem::Other(_) => {}
        }
    }
}

/// Every CSS property used in the book (stylesheets, `<style>` blocks, `style` attributes) with its number of
/// declarations, for choosing what to filter.
pub fn list_properties(book: &OpenBook) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for name in book.archive.names() {
        let Some(text) = book.archive.get_string(name) else {
            continue;
        };
        if is_css_path(name) {
            collect_css(&text, &mut out);
        } else if is_markup_path(name) {
            map_style_contents(&text, &mut |css| {
                collect_css(css, &mut out);
                css.to_string()
            });
            rewrite_all_tags(&text, &mut |_, attr, val| {
                if attr == "style" {
                    declaration_names(val, &mut out);
                }
                None
            });
        }
    }
    out
}

fn drop_empty_rules(css: &str) -> (String, usize) {
    let mut out = String::with_capacity(css.len());
    let mut removed = 0;
    let mut skip_ws = false;
    for item in parse_css(css) {
        match item {
            CssItem::Rule { selectors, block } => {
                let inner = block.trim().trim_start_matches('{').trim_end_matches('}');
                if inner.trim().is_empty() {
                    removed += 1;
                    skip_ws = true;
                } else {
                    out.push_str(selectors);
                    out.push_str(block);
                    skip_ws = false;
                }
            }
            CssItem::At(s) => {
                skip_ws = false;
                let lower = s.to_ascii_lowercase();
                match (s.find('{'), s.rfind('}')) {
                    (Some(o), Some(c))
                        if lower.starts_with("@media") || lower.starts_with("@supports") =>
                    {
                        let (inner, n) = drop_empty_rules(&s[o + 1..c]);
                        removed += n;
                        out.push_str(&s[..=o]);
                        out.push_str(&inner);
                        out.push_str(&s[c..]);
                    }
                    _ => out.push_str(s),
                }
            }
            CssItem::Other(s) => {
                if skip_ws && s.trim().is_empty() {
                    continue;
                }
                skip_ws = false;
                out.push_str(s);
            }
        }
    }
    (out, removed)
}

/// Removes the listed properties from every stylesheet, `<style>` block and inline style.
pub fn filter_styles(book: &mut OpenBook, spec: &FilterSpec) -> FilterReport {
    let mut rep = FilterReport::default();
    let ts: Vec<CssTransform> = spec
        .properties
        .iter()
        .filter(|p| !p.trim().is_empty())
        .map(|p| CssTransform::RemoveProperty { name: p.clone() })
        .collect();
    for name in book.archive.names().to_vec() {
        let Some(text) = book.archive.get_string(&name) else {
            continue;
        };
        let is_css = is_css_path(&name);
        if !is_css && !is_markup_path(&name) {
            continue;
        }
        let (mut new, n) = if ts.is_empty() {
            (text.clone(), 0)
        } else if is_css {
            transform_stylesheet(&text, &ts)
        } else {
            transform_markup_styles(&text, &ts)
        };
        if spec.remove_empty_rules {
            if is_css {
                let (n2, r) = drop_empty_rules(&new);
                new = n2;
                rep.empty_rules_removed += r;
            } else {
                new = map_style_contents(&new, &mut |css| {
                    let (n2, r) = drop_empty_rules(css);
                    rep.empty_rules_removed += r;
                    n2
                });
            }
        }
        if spec.remove_empty_style_attrs && !is_css {
            let mut removed_here = 0;
            let mut out = String::with_capacity(new.len());
            let mut last = 0;
            for ev in scan(&new) {
                if let Event::Open { range, .. } = ev {
                    let tag = &new[range.clone()];
                    if tag_attr(tag, "style").is_some_and(|s| s.trim().is_empty()) {
                        out.push_str(&new[last..range.start]);
                        out.push_str(&remove_tag_attr(tag, "style"));
                        last = range.end;
                        removed_here += 1;
                    }
                }
            }
            out.push_str(&new[last..]);
            new = out;
            rep.empty_style_attrs_removed += removed_here;
        }
        if new != text {
            book.archive.set_string(&name, &new);
            rep.declarations_removed += n;
            rep.files.insert(name, n);
        }
    }
    rep
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::make_book;

    fn styled_book() -> OpenBook {
        let mut b = make_book(&[
            "<p style=\"-webkit-hyphens: auto; color: red\">a</p><p style=\"-webkit-hyphens: auto\">b</p><p style=\"margin: 0\">c</p>",
        ]);
        b.archive.set_string(
            "OEBPS/style.css",
            "body { -webkit-hyphens: auto; color: black }\n.x { -webkit-hyphens: none }\n\n@media print {\n  .y { -webkit-text-size-adjust: 100% }\n  .z { color: red }\n}\n@font-face { font-family: F; -webkit-font-smoothing: antialiased }\np { margin: 0 }\n",
        );
        let t = b
            .archive
            .get_string("OEBPS/chapter-1.xhtml")
            .unwrap()
            .replace(
                "</head>",
                "<style>.w { -webkit-hyphens: manual }</style></head>",
            );
        b.archive.set_string("OEBPS/chapter-1.xhtml", &t);
        b
    }

    #[test]
    fn property_inventory() {
        let b = styled_book();
        let l = list_properties(&b);
        assert_eq!(l["-webkit-hyphens"], 5);
        assert_eq!(l["color"], 3);
        assert_eq!(l["margin"], 2);
        assert_eq!(l["-webkit-text-size-adjust"], 1);
        assert_eq!(l["-webkit-font-smoothing"], 1);
        assert_eq!(l["font-family"], 1);
        assert!(!l.contains_key(""));
    }

    #[test]
    fn filter_with_wildcard_and_cleanup() {
        let mut b = styled_book();
        let spec = FilterSpec {
            properties: vec!["-webkit-*".into()],
            remove_empty_rules: true,
            remove_empty_style_attrs: true,
        };
        let r = filter_styles(&mut b, &spec);
        assert_eq!(r.declarations_removed, 7);
        assert_eq!(
            r.empty_rules_removed, 3,
            ".x, .y (in @media) and .w (in <style>) become empty; body keeps color"
        );
        assert_eq!(r.empty_style_attrs_removed, 1);
        let css = b.archive.get_string("OEBPS/style.css").unwrap();
        assert_eq!(
            css,
            "body { color: black }\n@media print {\n  .z { color: red }\n}\n@font-face { font-family: F; }\np { margin: 0 }\n"
        );
        let ch = b.archive.get_string("OEBPS/chapter-1.xhtml").unwrap();
        assert!(
            ch.contains("<p style=\"color: red\">a</p><p>b</p><p style=\"margin: 0\">c</p>"),
            "{ch}"
        );
        assert!(!ch.contains(".w {"));
        let again = filter_styles(&mut b, &spec);
        assert_eq!(again, FilterReport::default());
        assert_eq!(
            list_properties(&b)
                .keys()
                .filter(|k| k.starts_with("-webkit"))
                .count(),
            0
        );
    }

    #[test]
    fn filter_keeps_empty_things_when_not_asked() {
        let mut b = styled_book();
        let r = filter_styles(
            &mut b,
            &FilterSpec {
                properties: vec!["margin".into(), "  ".into()],
                ..Default::default()
            },
        );
        assert_eq!(r.declarations_removed, 2);
        assert_eq!(r.empty_rules_removed, 0);
        assert!(
            b.archive
                .get_string("OEBPS/style.css")
                .unwrap()
                .contains("p { }")
        );
        assert!(
            b.archive
                .get_string("OEBPS/chapter-1.xhtml")
                .unwrap()
                .contains("<p style=\"\">c</p>")
        );
    }

    #[test]
    fn empty_spec_is_a_noop() {
        let mut b = styled_book();
        let before = b.archive.get_string("OEBPS/style.css").unwrap();
        assert_eq!(
            filter_styles(&mut b, &FilterSpec::default()),
            FilterReport::default()
        );
        assert_eq!(b.archive.get_string("OEBPS/style.css").unwrap(), before);
    }

    #[test]
    fn empty_rule_cleanup_alone() {
        let mut b = make_book(&["<p>x</p>"]);
        b.archive
            .set_string("OEBPS/style.css", "a { }\nb { /* c */ }\nc { x: y }\n");
        let r = filter_styles(
            &mut b,
            &FilterSpec {
                remove_empty_rules: true,
                ..Default::default()
            },
        );
        assert_eq!(
            r.empty_rules_removed, 1,
            "a rule holding only a comment is kept"
        );
        assert_eq!(
            b.archive.get_string("OEBPS/style.css").unwrap(),
            "b { /* c */ }\nc { x: y }\n"
        );
    }
}
