//! The Live CSS panel (CE-260): for the element at a position of an XHTML file, which rules of the book's
//! style sheets apply, in cascade order, where they come from, what they inherit and what is overridden.
//!
//! Selector matching and specificity are the reader's own (`bookx_text::css`); the rules are read here as
//! text so that the file, the line and the property as written are kept. Values are the *declared* values
//! after the cascade (the winner per property); sizes in `em`/`%` are not resolved to pixels.
//! The user agent sheet and the reader's user style are not included.

use std::collections::HashSet;
use std::ops::Range;

use anyhow::{bail, Context, Result};
use bookx_core::archive::Archive;
use bookx_core::util::{parent_dir, resolve_href};
use bookx_text::css::{media_applies, parse_selector_list, MediaEnv};
use scraper::ElementRef;

use crate::markup::{scan, tag_attr, Event};

/// A declaration of a matched rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclInfo {
    pub name: String,
    pub value: String,
    pub important: bool,
    /// Another declaration of the cascade wins over this one (shown struck through).
    pub overridden: bool,
}

/// Where a rule is written: a file and a 1-based line, and the byte offset in that file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub file: String,
    pub line: usize,
    pub offset: usize,
}

impl Origin {
    pub fn label(&self) -> String {
        format!("{}:{}", self.file, self.line)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchedRule {
    /// The selector that matched, as written.
    pub selector: String,
    /// `ids << 16 | classes << 8 | types`.
    pub specificity: u32,
    pub origin: Origin,
    pub decls: Vec<DeclInfo>,
    /// For inherited rules: the ancestor they matched (`div.chapter`).
    pub from: Option<String>,
    /// The `style` attribute of the element itself.
    pub inline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Computed {
    pub name: String,
    pub value: String,
    pub inherited: bool,
    pub origin: Origin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveCss {
    /// `p.note#x`.
    pub element: String,
    /// Line of its start tag.
    pub line: usize,
    /// Position among the elements, like `html > body > div:2 > p:3`.
    pub path: String,
    /// Rules that apply to the element, strongest first.
    pub rules: Vec<MatchedRule>,
    /// Rules of ancestors that set inherited properties, nearest ancestor first.
    pub inherited: Vec<MatchedRule>,
    pub computed: Vec<Computed>,
}

/// Properties a child takes over from its parent.
const INHERITED: &[&str] = &[
    "color",
    "cursor",
    "direction",
    "font",
    "font-family",
    "font-size",
    "font-style",
    "font-variant",
    "font-weight",
    "hyphens",
    "letter-spacing",
    "line-height",
    "list-style",
    "list-style-type",
    "orphans",
    "quotes",
    "text-align",
    "text-indent",
    "text-transform",
    "visibility",
    "white-space",
    "widows",
    "word-spacing",
    "writing-mode",
    "text-orientation",
];

fn is_inherited(prop: &str) -> bool {
    INHERITED.contains(&prop)
}

/// 1-based line of byte `off`.
pub fn line_of(src: &str, off: usize) -> usize {
    src.as_bytes()[..off.min(src.len())]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
        + 1
}

/// A declaration as written.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RawDecl {
    name: String,
    value: String,
    important: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawRule {
    selector: String,
    /// Byte offset of the selector in the text.
    at: usize,
    decls: Vec<RawDecl>,
}

fn blank_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        let end = rest[i + 2..]
            .find("*/")
            .map(|e| i + 2 + e + 2)
            .unwrap_or(rest.len());
        out.extend(
            rest[i..end]
                .chars()
                .map(|c| if c == '\n' { '\n' } else { ' ' }),
        );
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Index of the `}` matching the `{` at `open` (or the end).
fn block_end(s: &str, open: usize) -> usize {
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    for (i, c) in s[open..].char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '{') => depth += 1,
            (None, '}') => {
                depth -= 1;
                if depth == 0 {
                    return open + i;
                }
            }
            _ => {}
        }
    }
    s.len()
}

fn split_decls(body: &str) -> Vec<RawDecl> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut paren = 0i32;
    let mut quote: Option<char> = None;
    let mut parts = Vec::new();
    for (i, c) in body.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => paren += 1,
            (None, ')') => paren -= 1,
            (None, ';') if paren <= 0 => {
                parts.push(&body[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&body[start..]);
    for p in parts {
        let Some((n, v)) = p.split_once(':') else {
            continue;
        };
        let name = n.trim().to_ascii_lowercase();
        if name.is_empty() || name.contains(char::is_whitespace) {
            continue;
        }
        let mut value = v.trim().to_string();
        let mut important = false;
        let lower = value.to_ascii_lowercase();
        if let Some(i) = lower.rfind("!important") {
            if lower[i + 10..].trim().is_empty() {
                important = true;
                value = value[..i].trim().to_string();
            }
        }
        out.push(RawDecl {
            name,
            value,
            important,
        });
    }
    out
}

fn raw_rules(css: &str) -> Vec<RawRule> {
    let clean = blank_comments(css);
    let mut out = Vec::new();
    collect_rules(&clean, 0, clean.len(), &mut out, 0);
    out
}

fn collect_rules(s: &str, mut i: usize, end: usize, out: &mut Vec<RawRule>, depth: usize) {
    while i < end {
        let rest = &s[i..end];
        let trimmed = rest.trim_start();
        i += rest.len() - trimmed.len();
        if i >= end {
            return;
        }
        let rest = &s[i..end];
        if rest.starts_with('@') {
            let stop = rest.find(['{', ';']).unwrap_or(rest.len());
            if rest[stop..].starts_with(';') || stop >= rest.len() {
                i += stop + 1;
                continue;
            }
            let prelude = &rest[1..stop];
            let open = i + stop;
            let close = block_end(s, open).min(end);
            let name = prelude.split_whitespace().next().unwrap_or("");
            if name == "media"
                && depth < 4
                && media_applies(
                    prelude[name.len()..].trim(),
                    MediaEnv {
                        viewport_width: 800.0,
                    },
                )
            {
                collect_rules(s, open + 1, close, out, depth + 1);
            }
            i = close + 1;
            continue;
        }
        let Some(stop) = rest.find('{') else { return };
        let open = i + stop;
        let close = block_end(s, open).min(end);
        out.push(RawRule {
            selector: rest[..stop].trim().to_string(),
            at: i,
            decls: split_decls(&s[open + 1..close]),
        });
        i = close + 1;
    }
}

/// One style sheet of the document: where its text lives.
struct Sheet {
    file: String,
    /// The text.
    css: String,
    /// Byte offset of the text in `file` (a `<style>` block inside the XHTML).
    base: usize,
    /// The file the lines are counted in.
    text_of_file: String,
}

fn sheets_of(archive: &Archive, html_path: &str, src: &str) -> Vec<Sheet> {
    let dir = parent_dir(html_path);
    let mut out = Vec::new();
    let mut in_style: Option<usize> = None;
    for ev in scan(src) {
        match ev {
            Event::Open { name, range, .. } if name == "link" => {
                let tag = &src[range];
                let rel = tag_attr(tag, "rel")
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if !rel.split_whitespace().any(|r| r == "stylesheet") {
                    continue;
                }
                if let Some(href) = tag_attr(tag, "href") {
                    let (p, _) = resolve_href(dir, &href);
                    if let Some(css) = archive.get_string(&p) {
                        out.push(Sheet {
                            file: p,
                            text_of_file: css.clone(),
                            css,
                            base: 0,
                        });
                    }
                }
            }
            Event::Open {
                name, self_closing, ..
            } if name == "style" && !self_closing => in_style = Some(0),
            Event::Text(r) if in_style.is_some() => {
                out.push(Sheet {
                    file: html_path.to_string(),
                    css: src[r.clone()].to_string(),
                    base: r.start,
                    text_of_file: src.to_string(),
                });
            }
            Event::Close { name, .. } if name == "style" => in_style = None,
            _ => {}
        }
    }
    out
}

/// An element's position: `(name, child index)` from `<html>` down, and its start tag.
type ElementPlace = (Vec<(String, usize)>, Range<usize>);

/// The element that holds `offset`: the innermost one whose start tag begins at or before it and that is not
/// closed before it, as a path of `(name, child index)` from `<html>` plus its start tag.
fn element_at(src: &str, offset: usize) -> Option<ElementPlace> {
    // (name, start tag, index among its siblings, element children seen so far)
    let mut stack: Vec<(String, Range<usize>, usize, usize)> = Vec::new();
    let mut roots = 0usize;
    let mut found: Option<ElementPlace> = None;
    for ev in scan(src) {
        match ev {
            Event::Open {
                name,
                range,
                self_closing,
            } => {
                if range.start > offset {
                    break;
                }
                let idx = match stack.last_mut() {
                    Some(top) => {
                        top.3 += 1;
                        top.3 - 1
                    }
                    None => {
                        roots += 1;
                        roots - 1
                    }
                };
                if is_void(&name) || self_closing {
                    if offset < range.end {
                        let mut path: Vec<(String, usize)> =
                            stack.iter().map(|s| (s.0.clone(), s.2)).collect();
                        path.push((name, idx));
                        found = Some((path, range));
                        break;
                    }
                    continue;
                }
                stack.push((name, range, idx, 0));
            }
            Event::Close { name, range } => {
                if range.start >= offset {
                    break;
                }
                if range.end <= offset {
                    if let Some(p) = stack.iter().rposition(|s| s.0 == name) {
                        stack.truncate(p);
                    }
                } else {
                    break;
                }
            }
            _ => {}
        }
    }
    if found.is_some() {
        return found;
    }
    let top = stack.last()?;
    let path = stack.iter().map(|s| (s.0.clone(), s.2)).collect();
    Some((path, top.1.clone()))
}

fn is_void(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "source"
            | "track"
            | "wbr"
            | "image"
            | "path"
            | "rect"
            | "circle"
    )
}

fn describe(el: ElementRef) -> String {
    let v = el.value();
    let mut s = v.name().to_string();
    if let Some(id) = v.attr("id") {
        s.push('#');
        s.push_str(id);
    }
    for c in v.attr("class").unwrap_or("").split_whitespace() {
        s.push('.');
        s.push_str(c);
    }
    s
}

fn find_by_path<'a>(root: ElementRef<'a>, path: &[(String, usize)]) -> Option<ElementRef<'a>> {
    // path[0] is `html` itself
    let mut cur = root;
    if path.first().is_none_or(|p| p.0 != cur.value().name()) {
        return None;
    }
    for (name, i) in &path[1..] {
        let kids: Vec<ElementRef> = cur.children().filter_map(ElementRef::wrap).collect();
        // html5ever may insert elements (tbody) the source does not have: look for the same name near `i`
        let pick = kids
            .get(*i)
            .filter(|k| k.value().name() == name)
            .or_else(|| kids.iter().find(|k| k.value().name() == name));
        cur = *pick?;
    }
    Some(cur)
}

struct Cand {
    selector: String,
    specificity: u32,
    order: usize,
    origin: Origin,
    decls: Vec<RawDecl>,
    inline: bool,
}

fn matching(
    el: ElementRef,
    sheets: &[(Sheet, Vec<RawRule>)],
    inline_origin: Option<Origin>,
) -> Vec<Cand> {
    let mut out = Vec::new();
    let mut order = 0;
    for (sh, rules) in sheets {
        for r in rules {
            let Some(sels) = parse_selector_list(&r.selector) else {
                continue;
            };
            let best = sels
                .iter()
                .filter(|s| !s.first_letter && s.matches(el))
                .map(|s| s.specificity())
                .max();
            if let Some(spec) = best {
                let off = sh.base + r.at;
                out.push(Cand {
                    selector: r.selector.split_whitespace().collect::<Vec<_>>().join(" "),
                    specificity: spec,
                    order,
                    origin: Origin {
                        file: sh.file.clone(),
                        line: line_of(&sh.text_of_file, off),
                        offset: off,
                    },
                    decls: r.decls.clone(),
                    inline: false,
                });
            }
            order += 1;
        }
    }
    if let (Some(style), Some(origin)) = (el.value().attr("style"), inline_origin) {
        let decls = split_decls(style);
        if !decls.is_empty() {
            out.push(Cand {
                selector: "style attribute".into(),
                specificity: u32::MAX,
                order,
                origin,
                decls,
                inline: true,
            });
        }
    }
    out
}

/// Winner key of one declaration: important beats normal, then inline, specificity, source order.
fn key(c: &Cand, d: &RawDecl) -> (bool, bool, u32, usize) {
    (d.important, c.inline, c.specificity, c.order)
}

/// Marks which declarations win per property among `cands`. Returns, per candidate, the `overridden` flags,
/// and the set of properties each candidate wins.
fn resolve(cands: &[Cand], skip: &HashSet<String>, only_inherited: bool) -> Vec<Vec<DeclInfo>> {
    use std::collections::HashMap;
    let mut best: HashMap<&str, (bool, bool, u32, usize)> = HashMap::new();
    for c in cands {
        for d in &c.decls {
            if only_inherited && !is_inherited(&d.name) {
                continue;
            }
            let k = key(c, d);
            let e = best.entry(d.name.as_str()).or_insert(k);
            if k > *e {
                *e = k;
            }
        }
    }
    cands
        .iter()
        .map(|c| {
            c.decls
                .iter()
                .filter(|d| !only_inherited || is_inherited(&d.name))
                .map(|d| DeclInfo {
                    name: d.name.clone(),
                    value: d.value.clone(),
                    important: d.important,
                    overridden: skip.contains(&d.name)
                        || best.get(d.name.as_str()) != Some(&key(c, d)),
                })
                .collect()
        })
        .collect()
}

fn element_label(el: ElementRef) -> String {
    describe(el)
}

/// What applies to the element at byte `offset` of `html_path` (whose current text is `src`).
pub fn inspect(archive: &Archive, html_path: &str, src: &str, offset: usize) -> Result<LiveCss> {
    let offset = offset.min(src.len());
    let (path, tag) = element_at(src, offset).context("no element at the cursor")?;
    let doc = bookx_core::util::parse_html_document(src);
    let root = doc.root_element();
    let el =
        find_by_path(root, &path).context("the element could not be found in the parsed page")?;

    let sheets: Vec<(Sheet, Vec<RawRule>)> = sheets_of(archive, html_path, src)
        .into_iter()
        .map(|s| {
            let r = raw_rules(&s.css);
            (s, r)
        })
        .collect();
    let inline_origin = Origin {
        file: html_path.to_string(),
        line: line_of(src, tag.start),
        offset: tag.start,
    };
    let cands = matching(el, &sheets, Some(inline_origin.clone()));
    let resolved = resolve(&cands, &HashSet::new(), false);
    let mut order: Vec<usize> = (0..cands.len()).collect();
    // strongest first: inline, specificity, later source order
    order.sort_by_key(|i| {
        std::cmp::Reverse((cands[*i].inline, cands[*i].specificity, cands[*i].order))
    });
    let rules: Vec<MatchedRule> = order
        .iter()
        .map(|i| MatchedRule {
            selector: cands[*i].selector.clone(),
            specificity: if cands[*i].inline {
                0
            } else {
                cands[*i].specificity
            },
            origin: cands[*i].origin.clone(),
            decls: resolved[*i].clone(),
            from: None,
            inline: cands[*i].inline,
        })
        .collect();

    let mut computed: Vec<Computed> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for r in &rules {
        for d in r.decls.iter().filter(|d| !d.overridden) {
            if seen.insert(d.name.clone()) {
                computed.push(Computed {
                    name: d.name.clone(),
                    value: d.value.clone(),
                    inherited: false,
                    origin: r.origin.clone(),
                });
            }
        }
    }

    let mut inherited = Vec::new();
    let mut parent = el.parent().and_then(ElementRef::wrap);
    while let Some(p) = parent {
        let pc = matching(p, &sheets, None);
        let res = resolve(&pc, &seen, true);
        let mut idx: Vec<usize> = (0..pc.len()).collect();
        idx.sort_by_key(|i| std::cmp::Reverse((pc[*i].specificity, pc[*i].order)));
        let label = element_label(p);
        for i in idx {
            if res[i].is_empty() {
                continue;
            }
            inherited.push(MatchedRule {
                selector: pc[i].selector.clone(),
                specificity: pc[i].specificity,
                origin: pc[i].origin.clone(),
                decls: res[i].clone(),
                from: Some(label.clone()),
                inline: false,
            });
            for d in res[i].iter().filter(|d| !d.overridden) {
                if seen.insert(d.name.clone()) {
                    computed.push(Computed {
                        name: d.name.clone(),
                        value: d.value.clone(),
                        inherited: true,
                        origin: pc[i].origin.clone(),
                    });
                }
            }
        }
        parent = p.parent().and_then(ElementRef::wrap);
    }
    computed.sort_by(|a, b| a.name.cmp(&b.name));

    let path_text = path
        .iter()
        .map(|(n, i)| {
            if *i == 0 {
                n.clone()
            } else {
                format!("{n}:{}", i + 1)
            }
        })
        .collect::<Vec<_>>()
        .join(" > ");
    Ok(LiveCss {
        element: element_label(el),
        line: line_of(src, tag.start),
        path: path_text,
        rules,
        inherited,
        computed,
    })
}

/// Convenience for callers that only have a path: reads the file from the archive.
pub fn inspect_file(archive: &Archive, html_path: &str, offset: usize) -> Result<LiveCss> {
    let Some(src) = archive.get_string(html_path) else {
        bail!("{html_path} not found");
    };
    inspect(archive, html_path, &src, offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSS: &str = "/* base */\nbody { font-family: serif; color: #222 }\n\np { margin: 0; text-indent: 1em }\n.note { color: red; margin: 1em !important }\np.note { text-indent: 0 }\n@media screen { div p { color: blue } }\n@media print { p { color: green } }\n@font-face { font-family: X; src: url(a.ttf) }\n";

    const HTML: &str = "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>t</title><link rel=\"stylesheet\" href=\"s.css\"/><style>\nh1 { color: purple }\n</style></head>\n<body>\n<div class=\"chapter\"><p class=\"note\" style=\"margin: 3em; color: black\">Hello <i>there</i></p></div>\n<h1>Title</h1>\n</body></html>";

    fn book() -> Archive {
        let mut a = Archive::default();
        a.set_string("OEBPS/s.css", CSS);
        a.set_string("OEBPS/c.xhtml", HTML);
        a
    }

    fn at(needle: &str) -> usize {
        HTML.find(needle).unwrap()
    }

    #[test]
    fn raw_rules_keep_lines_media_and_skip_other_at_rules() {
        let r = raw_rules(CSS);
        let sels: Vec<&str> = r.iter().map(|r| r.selector.as_str()).collect();
        assert_eq!(sels, ["body", "p", ".note", "p.note", "div p"]);
        assert!(r[2].decls[1].important);
        assert_eq!(r[2].decls[1].name, "margin");
        assert_eq!(line_of(CSS, r[1].at), 4);
    }

    #[test]
    fn decl_splitting_survives_semicolons_in_urls_and_strings() {
        let d = split_decls(
            "background: url(data:image/png;base64,AAA); content: \"a;b\"; color: red !IMPORTANT",
        );
        assert_eq!(d.len(), 3);
        assert!(d[0].value.starts_with("url(data:image/png;base64"));
        assert_eq!(d[1].value, "\"a;b\"");
        assert!(d[2].important && d[2].value == "red");
    }

    #[test]
    fn matching_rules_come_in_cascade_order_with_origins() {
        let l = inspect(&book(), "OEBPS/c.xhtml", HTML, at("Hello")).unwrap();
        assert_eq!(l.element, "p.note");
        assert_eq!(l.line, 5);
        let sels: Vec<&str> = l.rules.iter().map(|r| r.selector.as_str()).collect();
        assert_eq!(sels, ["style attribute", "p.note", ".note", "div p", "p"]);
        let note = l.rules.iter().find(|r| r.selector == ".note").unwrap();
        assert_eq!(note.origin.label(), "OEBPS/s.css:5");
        assert_eq!(l.rules[0].origin.file, "OEBPS/c.xhtml");
        assert_eq!(l.rules[0].origin.line, 5);
        // the print rule did not apply, the screen one did
        assert!(!l
            .rules
            .iter()
            .any(|r| r.decls.iter().any(|d| d.value == "green")));
    }

    #[test]
    fn overridden_properties_are_marked_and_important_wins() {
        let l = inspect(&book(), "OEBPS/c.xhtml", HTML, at("Hello")).unwrap();
        let get = |sel: &str, name: &str| {
            l.rules
                .iter()
                .find(|r| r.selector == sel)
                .unwrap()
                .decls
                .iter()
                .find(|d| d.name == name)
                .unwrap()
                .clone()
        };
        // !important in .note beats the inline margin
        assert!(!get(".note", "margin").overridden);
        assert!(get("style attribute", "margin").overridden);
        assert!(get("p", "margin").overridden);
        // inline colour beats the screen rule and .note
        assert!(!get("style attribute", "color").overridden);
        assert!(get("div p", "color").overridden);
        assert!(get(".note", "color").overridden);
        // p.note (0,2,1... higher specificity) beats p for text-indent
        assert!(!get("p.note", "text-indent").overridden);
        assert!(get("p", "text-indent").overridden);
    }

    #[test]
    fn computed_values_are_the_winners_and_inheritance_is_listed() {
        let l = inspect(&book(), "OEBPS/c.xhtml", HTML, at("Hello")).unwrap();
        let c = |n: &str| l.computed.iter().find(|c| c.name == n).cloned();
        assert_eq!(c("margin").unwrap().value, "1em");
        assert_eq!(c("color").unwrap().value, "black");
        assert!(!c("color").unwrap().inherited);
        let ff = c("font-family").unwrap();
        assert_eq!(ff.value, "serif");
        assert!(ff.inherited);
        // the body colour is inherited but overridden by the element's own colour
        let body = l.inherited.iter().find(|r| r.selector == "body").unwrap();
        assert_eq!(body.from.as_deref(), Some("body"));
        let col = body.decls.iter().find(|d| d.name == "color").unwrap();
        assert!(col.overridden);
        assert!(
            !body
                .decls
                .iter()
                .find(|d| d.name == "font-family")
                .unwrap()
                .overridden
        );
        // margin is not inherited
        assert!(body.decls.iter().all(|d| d.name != "margin"));
    }

    #[test]
    fn style_blocks_in_the_page_are_found_with_page_lines() {
        let l = inspect(&book(), "OEBPS/c.xhtml", HTML, at("Title")).unwrap();
        assert_eq!(l.element, "h1");
        let r = &l.rules[0];
        assert_eq!(r.selector, "h1");
        assert_eq!(r.origin.file, "OEBPS/c.xhtml");
        assert_eq!(r.origin.line, 2);
        assert_eq!(&HTML[r.origin.offset..r.origin.offset + 2], "h1");
    }

    #[test]
    fn the_cursor_may_be_in_a_tag_in_text_or_inside_an_inline_element() {
        let in_tag = inspect(&book(), "OEBPS/c.xhtml", HTML, at("class=\"note\"")).unwrap();
        assert_eq!(in_tag.element, "p.note");
        let in_i = inspect(&book(), "OEBPS/c.xhtml", HTML, at("there")).unwrap();
        assert_eq!(in_i.element, "i");
        // `i` has no rules of its own but inherits the paragraph's
        assert!(in_i.rules.is_empty());
        assert!(in_i
            .inherited
            .iter()
            .any(|r| r.from.as_deref() == Some("p.note")));
        let after = inspect(&book(), "OEBPS/c.xhtml", HTML, at("</p>") + 4).unwrap();
        assert_eq!(after.element, "div.chapter");
        assert!(after.path.starts_with("html > body"), "{}", after.path);
    }

    #[test]
    fn broken_input_is_an_error_not_a_panic() {
        assert!(inspect(&book(), "OEBPS/c.xhtml", "", 0).is_err());
        assert!(inspect_file(&book(), "OEBPS/none.xhtml", 0).is_err());
        let a = Archive::default();
        let l = inspect(
            &a,
            "x.xhtml",
            "<html><body><p class=\"a\">t</p></body></html>",
            20,
        )
        .unwrap();
        assert!(l.rules.is_empty());
        // a fragment without <html> cannot be placed in the parsed page
        assert!(inspect(&a, "x.xhtml", "<p>no html root</p>", 4).is_err());
    }
}
