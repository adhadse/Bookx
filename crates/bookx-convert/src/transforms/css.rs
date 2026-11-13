//! A forgiving CSS scanner and the `remove_unused_css` transform.

use std::collections::HashMap;

use anyhow::Result;
use bookx_core::book::OpenBook;
use bookx_core::contracts::{Transform, TransformReport};
use regex::Regex;
use scraper::{Html, Selector};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CssItem<'a> {
    /// `selectors` is the text before `{`, `block` includes both braces.
    Rule { selectors: &'a str, block: &'a str },
    /// An at-rule, statement or block, verbatim.
    At(&'a str),
    /// Whitespace, comments or stray text.
    Other(&'a str),
}

fn skip_quoted(b: &[u8], i: usize) -> usize {
    let q = b[i];
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            c if c == q => return j + 1,
            _ => j += 1,
        }
    }
    b.len()
}

fn skip_comment(css: &str, i: usize) -> usize {
    css[i + 2..]
        .find("*/")
        .map(|p| i + 2 + p + 2)
        .unwrap_or(css.len())
}

/// Index just after the `}` that closes the `{` at `open`.
fn match_brace(css: &str, open: usize) -> usize {
    let b = css.as_bytes();
    let mut depth = 0usize;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' => {
                i = skip_quoted(b, i);
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = skip_comment(css, i);
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    b.len()
}

pub fn parse_css(css: &str) -> Vec<CssItem<'_>> {
    let b = css.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let e = skip_comment(css, i);
            out.push(CssItem::Other(&css[i..e]));
            i = e;
        } else if c.is_ascii_whitespace() {
            let mut j = i;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            out.push(CssItem::Other(&css[i..j]));
            i = j;
        } else if c == b'}' {
            out.push(CssItem::Other(&css[i..i + 1]));
            i += 1;
        } else if c == b'@' {
            let mut j = i;
            let mut end = b.len();
            while j < b.len() {
                match b[j] {
                    b'"' | b'\'' => {
                        j = skip_quoted(b, j);
                        continue;
                    }
                    b';' => {
                        end = j + 1;
                        break;
                    }
                    b'{' => {
                        end = match_brace(css, j);
                        break;
                    }
                    _ => {}
                }
                j += 1;
            }
            out.push(CssItem::At(&css[i..end]));
            i = end;
        } else {
            let mut j = i;
            let mut bracket = 0i32;
            let mut found = None;
            while j < b.len() {
                match b[j] {
                    b'"' | b'\'' => {
                        j = skip_quoted(b, j);
                        continue;
                    }
                    b'/' if b.get(j + 1) == Some(&b'*') => {
                        j = skip_comment(css, j);
                        continue;
                    }
                    b'[' | b'(' => bracket += 1,
                    b']' | b')' => bracket -= 1,
                    b'{' if bracket <= 0 => {
                        found = Some(j);
                        break;
                    }
                    _ => {}
                }
                j += 1;
            }
            match found {
                Some(open) => {
                    let end = match_brace(css, open);
                    out.push(CssItem::Rule {
                        selectors: &css[i..open],
                        block: &css[open..end],
                    });
                    i = end;
                }
                None => {
                    out.push(CssItem::Other(&css[i..]));
                    i = b.len();
                }
            }
        }
    }
    out
}

/// Splits `a, b > c, d:not(.x, .y)` on top-level commas.
pub fn split_selectors(list: &str) -> Vec<String> {
    let b = list.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' => {
                i = skip_quoted(b, i);
                continue;
            }
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b',' if depth <= 0 => {
                out.push(list[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(list[start..].trim().to_string());
    out.into_iter().filter(|s| !s.is_empty()).collect()
}

/// `(name, value, original_text)` of each declaration in a `{ ... }` block.
pub fn parse_decls(block: &str) -> Vec<(String, String, String)> {
    let inner = block.trim().trim_start_matches('{').trim_end_matches('}');
    let b = inner.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    let mut i = 0;
    let mut push = |s: &str| {
        if let Some((n, v)) = s.split_once(':') {
            if !n.trim().is_empty() {
                out.push((
                    n.trim().to_ascii_lowercase(),
                    v.trim().to_string(),
                    s.to_string(),
                ));
            }
        }
    };
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' => {
                i = skip_quoted(b, i);
                continue;
            }
            b'(' => depth += 1,
            b')' => depth -= 1,
            b';' if depth <= 0 => {
                push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < inner.len() {
        push(&inner[start..]);
    }
    out
}

/// Removes the pseudo-classes and pseudo-elements that cannot be decided from a static document.
fn strip_dynamic(sel: &str) -> String {
    let re = Regex::new(
        r"(?i)::?(hover|focus|focus-within|focus-visible|active|visited|link|any-link|target|before|after|first-line|first-letter|selection|checked|disabled|enabled|placeholder|marker|backdrop|-[a-z-]+)(\([^)]*\))?|::[a-z-]+(\([^)]*\))?",
    )
    .expect("static regex");
    let s = re.replace_all(sel, "").trim().to_string();
    if s.is_empty() || s.ends_with(['>', '+', '~', ' ']) {
        format!("{s}*")
    } else {
        s
    }
}

pub struct Usage {
    docs: Vec<Html>,
    cache: HashMap<String, bool>,
}

impl Usage {
    pub fn new(docs: Vec<Html>) -> Self {
        Usage {
            docs,
            cache: HashMap::new(),
        }
    }

    /// True when `selector` matches an element of any document, or when it cannot be evaluated
    /// (unknown syntax is always kept).
    pub fn is_used(&mut self, selector: &str) -> bool {
        if let Some(v) = self.cache.get(selector) {
            return *v;
        }
        let stripped = strip_dynamic(selector);
        let used = match Selector::parse(&stripped) {
            Ok(sel) => self.docs.iter().any(|d| d.select(&sel).next().is_some()),
            Err(_) => true,
        };
        self.cache.insert(selector.to_string(), used);
        used
    }
}

/// Parsed documents for every markup file in the manifest.
pub fn parse_documents(book: &OpenBook) -> Vec<Html> {
    book.package
        .manifest
        .iter()
        .filter(|i| i.media_type == "application/xhtml+xml" || i.media_type == "text/html")
        .filter_map(|i| book.archive.get_string(&book.package.item_path(i)))
        .map(|t| Html::parse_document(&t))
        .collect()
}

pub fn css_paths(book: &OpenBook) -> Vec<String> {
    book.package
        .manifest
        .iter()
        .filter(|i| i.media_type == "text/css")
        .map(|i| book.package.item_path(i))
        .collect()
}

pub fn prune_css(css: &str, usage: &mut Usage) -> String {
    let mut out = String::with_capacity(css.len());
    let mut removed_any = false;
    for item in parse_css(css) {
        match item {
            CssItem::Rule { selectors, block } => {
                let list = split_selectors(selectors);
                let kept: Vec<&String> = list.iter().filter(|s| usage.is_used(s)).collect();
                if kept.len() == list.len() {
                    out.push_str(selectors);
                    out.push_str(block);
                } else {
                    removed_any = true;
                    if !kept.is_empty() {
                        let joined = kept
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        out.push_str(&format!("{joined} {block}"));
                    }
                }
            }
            CssItem::At(s) | CssItem::Other(s) => out.push_str(s),
        }
    }
    if removed_any {
        out
    } else {
        css.to_string()
    }
}

/// Drops CSS rules whose selectors match no element of any document of the book.
/// `@font-face`, `@media`, `@page` and other at-rules are always kept.
#[derive(Debug, Clone, Default)]
pub struct RemoveUnusedCss;

impl Transform for RemoveUnusedCss {
    fn id(&self) -> &'static str {
        "remove_unused_css"
    }
    fn apply(&self, book: &mut OpenBook) -> Result<TransformReport> {
        let mut usage = Usage::new(parse_documents(book));
        let mut changed = 0;
        let mut notes = Vec::new();
        for path in css_paths(book) {
            let Some(css) = book.archive.get_string(&path) else {
                continue;
            };
            let new = prune_css(&css, &mut usage);
            if new != css {
                notes.push(format!("{path}: {} -> {} bytes", css.len(), new.len()));
                book.archive.set_string(&path, &new);
                changed += 1;
            }
        }
        Ok(TransformReport {
            files_changed: changed,
            notes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;

    fn usage(html: &str) -> Usage {
        Usage::new(vec![Html::parse_document(html)])
    }

    #[test]
    fn parses_rules_and_at_rules() {
        let css = "@charset \"utf-8\";\n/* c */ p, .a { color: red }\n@media print { p { x: y } }\n@font-face { font-family: F; src: url(a.ttf) }";
        let items = parse_css(css);
        let rules = items
            .iter()
            .filter(|i| matches!(i, CssItem::Rule { .. }))
            .count();
        let ats = items.iter().filter(|i| matches!(i, CssItem::At(_))).count();
        assert_eq!(rules, 1);
        assert_eq!(ats, 3);
        let joined: String = items
            .iter()
            .map(|i| match i {
                CssItem::Rule { selectors, block } => format!("{selectors}{block}"),
                CssItem::At(s) | CssItem::Other(s) => s.to_string(),
            })
            .collect();
        assert_eq!(joined, css);
    }

    #[test]
    fn braces_inside_strings_and_comments_do_not_confuse() {
        let css = "a[title=\"}{\"] { content: \"}\"; } /* } */ b { x: y }";
        let rules: Vec<_> = parse_css(css)
            .into_iter()
            .filter_map(|i| match i {
                CssItem::Rule { selectors, .. } => Some(selectors.trim().to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(rules, vec!["a[title=\"}{\"]", "b"]);
    }

    #[test]
    fn unterminated_css_does_not_panic() {
        let _ = parse_css("p { color: red");
        let _ = parse_css("p");
        let _ = parse_css("@media print { p {");
        let _ = parse_css("}}}");
    }

    #[test]
    fn selector_lists_split_at_top_level() {
        assert_eq!(
            split_selectors("a, b > c, d:not(.x, .y), [t=\"a,b\"]"),
            vec!["a", "b > c", "d:not(.x, .y)", "[t=\"a,b\"]"]
        );
    }

    #[test]
    fn declarations_keep_original_text() {
        let d = parse_decls(
            "{ margin-left: 5%; background: url(data:image/png;base64,AAA); COLOR:red }",
        );
        assert_eq!(d.len(), 3);
        assert_eq!(d[0].0, "margin-left");
        assert_eq!(d[1].1, "url(data:image/png;base64,AAA)");
        assert_eq!(d[2].0, "color");
    }

    #[test]
    fn removes_unused_keeps_used() {
        let mut u = usage("<html><body><p class=\"a\">x</p></body></html>");
        let css = ".a { color: red }\n.b { color: blue }\n#nothing { x: y }\np { margin: 0 }";
        let out = prune_css(css, &mut u);
        assert!(out.contains(".a {"));
        assert!(!out.contains(".b"));
        assert!(!out.contains("#nothing"));
        assert!(out.contains("p { margin: 0 }"));
    }

    #[test]
    fn selector_lists_keep_only_used_members() {
        let mut u = usage("<p class=\"a\">x</p>");
        let out = prune_css(".a, .b, .c > i { color: red }", &mut u);
        assert_eq!(out.trim(), ".a { color: red }");
        // fully used lists are left byte for byte
        let css = ".a ,  p {x:y}";
        assert_eq!(prune_css(css, &mut u), css);
    }

    #[test]
    fn at_rules_are_always_kept() {
        let mut u = usage("<p>x</p>");
        let css = "@font-face { font-family: F; src: url(f.ttf) }\n@media print { .zz { x: y } }\n@page { margin: 1em }\n@import url(a.css);";
        assert_eq!(prune_css(css, &mut u), css);
    }

    #[test]
    fn pseudo_classes_are_handled_safely() {
        let mut u = usage("<p class=\"a\">x</p><a href=\"#\">l</a>");
        let css = "a:hover { x: y }\n.a::first-letter { x: y }\n.z:hover { x: y }\np:first-child { x: y }\n.a:not(.q) { x: y }\n.weird:made-up(1) { x: y }\n::selection { x: y }\na:link, a:visited { x: y }";
        let out = prune_css(css, &mut u);
        assert!(out.contains("a:hover"));
        assert!(out.contains(".a::first-letter"));
        assert!(!out.contains(".z:hover"));
        assert!(out.contains("p:first-child"));
        assert!(out.contains(".a:not(.q)"));
        assert!(
            out.contains(".weird:made-up(1)"),
            "unparsable selectors are kept"
        );
        assert!(out.contains("::selection"));
        assert!(out.contains("a:link, a:visited"));
    }

    #[test]
    fn descendant_and_attribute_selectors() {
        let mut u = usage("<div class=\"c\"><span id=\"s\" lang=\"en\">x</span></div>");
        let out = prune_css(
            ".c span { a: b }\n.c > p { a: b }\nspan[lang=\"en\"] { a: b }\nspan[lang=\"fr\"] { a: b }\n#s { a: b }\n#t { a: b }",
            &mut u,
        );
        assert!(out.contains(".c span"));
        assert!(!out.contains(".c > p"));
        assert!(out.contains("[lang=\"en\"]"));
        assert!(!out.contains("[lang=\"fr\"]"));
        assert!(out.contains("#s "));
        assert!(!out.contains("#t "));
    }

    #[test]
    fn prune_is_idempotent() {
        let mut u = usage("<p class=\"a\">x</p>");
        let once = prune_css(
            ".a, .b { x: y }\n.c { x: y }\n@media print { .q { x: y } }",
            &mut u,
        );
        assert_eq!(prune_css(&once, &mut u), once);
    }

    #[test]
    fn transform_end_to_end() {
        let mut b = book_with(&[("A", "<p class=\"used\">x</p>"), ("B", "<p>y</p>")]);
        b.archive.set_string(
            "OEBPS/style.css",
            ".used { color: red }\n.unused { color: blue }\n",
        );
        let t = RemoveUnusedCss;
        let r = t.apply(&mut b).unwrap();
        assert_eq!(r.files_changed, 1);
        let css = b.archive.get_string("OEBPS/style.css").unwrap();
        assert!(css.contains(".used"));
        assert!(!css.contains(".unused"));
        let snap = snapshot(&b);
        assert_eq!(t.apply(&mut b).unwrap().files_changed, 0);
        assert_eq!(snapshot(&b), snap);
        assert_eq!(t.id(), "remove_unused_css");
    }

    #[test]
    fn classes_used_in_any_file_are_kept() {
        let mut b = book_with(&[("A", "<p>x</p>"), ("B", "<p class=\"late\">y</p>")]);
        b.archive
            .set_string("OEBPS/style.css", ".late { x: y }\n.never { x: y }\n");
        RemoveUnusedCss.apply(&mut b).unwrap();
        let css = b.archive.get_string("OEBPS/style.css").unwrap();
        assert!(css.contains(".late") && !css.contains(".never"));
    }
}
