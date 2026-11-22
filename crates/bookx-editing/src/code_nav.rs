//! Navigation inside the code view without GTK (CE-342 Ctrl+click on file names and classes, CE-343 rename a
//! class across HTML and CSS, CE-344 jump to the definition of an id, CE-341 context help URLs, CE-355 sort CSS
//! rules). Everything works on `&str` and byte offsets and tolerates broken markup and CSS.

use std::ops::Range;
use std::sync::OnceLock;

use bookx_core::util::{is_external, parent_dir, resolve_href};
use regex::Regex;

use crate::markup::{Event, is_css_path, is_markup_path, scan};

// ---------------------------------------------------------------------------------------------
// attributes with positions
// ---------------------------------------------------------------------------------------------

/// One attribute of a start tag: the name and the byte range of its value (without the quotes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttrSpan {
    pub name: String,
    pub value: Range<usize>,
}

/// The attributes of the start tag occupying `tag` (a range of `src`).
pub fn attr_spans(src: &str, tag: Range<usize>) -> Vec<AttrSpan> {
    let b = src.as_bytes();
    let end = tag.end.min(b.len());
    let mut i = tag.start + 1;
    let mut out = Vec::new();
    // tag name
    while i < end && !b[i].is_ascii_whitespace() && b[i] != b'/' && b[i] != b'>' {
        i += 1;
    }
    while i < end {
        while i < end && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        if i >= end || b[i] == b'>' {
            break;
        }
        let ns = i;
        while i < end && !b[i].is_ascii_whitespace() && !matches!(b[i], b'=' | b'>' | b'/') {
            i += 1;
        }
        let name = src[ns..i].to_ascii_lowercase();
        while i < end && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < end && b[i] == b'=' {
            i += 1;
            while i < end && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < end && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                let vs = i + 1;
                let mut j = vs;
                while j < end && b[j] != q {
                    j += 1;
                }
                out.push(AttrSpan {
                    name,
                    value: vs..j.min(end),
                });
                i = (j + 1).min(end);
            } else {
                let vs = i;
                while i < end && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                    i += 1;
                }
                out.push(AttrSpan { name, value: vs..i });
            }
        } else if !name.is_empty() {
            out.push(AttrSpan { name, value: i..i });
        } else {
            i += 1;
        }
    }
    out
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '-' || c == '_' || !c.is_ascii()
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '-' || !c.is_ascii()
}

/// The whitespace separated word of `s` that contains byte `at` (relative), as a range.
fn word_at(s: &str, at: usize) -> Option<Range<usize>> {
    let at = at.min(s.len());
    let start = s[..at]
        .rfind(char::is_whitespace)
        .map(|p| p + s[p..].chars().next().map_or(1, char::len_utf8))
        .unwrap_or(0);
    let end = s[at..]
        .find(char::is_whitespace)
        .map(|p| at + p)
        .unwrap_or(s.len());
    (start < end).then_some(start..end)
}

// ---------------------------------------------------------------------------------------------
// CSS rules
// ---------------------------------------------------------------------------------------------

/// A rule or at-rule of a stylesheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssRule {
    /// From the first leading comment (or the selector) to just after the closing brace / semicolon.
    pub whole: Range<usize>,
    pub selector: Range<usize>,
    /// Inside the braces; empty for statements such as `@import`.
    pub body: Range<usize>,
    /// Name of the at-rule (`media`, `font-face`, ...), `None` for a style rule.
    pub at: Option<String>,
    /// Not nested inside `@media` and friends.
    pub top: bool,
}

/// First of `stops` outside strings, comments and parentheses, at or after `i` and before `to`.
fn find_delim(css: &str, mut i: usize, to: usize, stops: &[u8]) -> Option<usize> {
    let b = css.as_bytes();
    let mut paren = 0i32;
    while i < to {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = css[i + 2..].find("*/").map(|p| i + 2 + p + 2).unwrap_or(to);
                continue;
            }
            b'"' | b'\'' => {
                let q = b[i];
                i += 1;
                while i < to && b[i] != q {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'(' => paren += 1,
            b')' => paren = (paren - 1).max(0),
            c if paren == 0 && stops.contains(&c) => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Index of the `}` matching the `{` at `open`, or `to`.
fn matching_brace(css: &str, open: usize, to: usize) -> usize {
    let b = css.as_bytes();
    let mut depth = 0i32;
    let mut i = open;
    while i < to {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = css[i + 2..].find("*/").map(|p| i + 2 + p + 2).unwrap_or(to);
                continue;
            }
            b'"' | b'\'' => {
                let q = b[i];
                i += 1;
                while i < to && b[i] != q {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
        i += 1;
    }
    to
}

fn rules_in(css: &str, from: usize, to: usize, top: bool, out: &mut Vec<CssRule>) {
    let b = css.as_bytes();
    let mut i = from;
    let mut lead: Option<usize> = None;
    while i < to {
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            lead.get_or_insert(i);
            i = css[i + 2..].find("*/").map(|p| i + 2 + p + 2).unwrap_or(to);
            continue;
        }
        let start = i;
        let whole_start = lead.take().unwrap_or(start);
        if b[i] == b'@' {
            let name_end = css[i + 1..]
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .map(|p| i + 1 + p)
                .unwrap_or(to);
            let name = css[i + 1..name_end].to_ascii_lowercase();
            let Some(j) = find_delim(css, i, to, b";{") else {
                break;
            };
            if b[j] == b';' {
                out.push(CssRule {
                    whole: whole_start..j + 1,
                    selector: start..j,
                    body: j + 1..j + 1,
                    at: Some(name),
                    top,
                });
                i = j + 1;
            } else {
                let close = matching_brace(css, j, to);
                let end = (close + 1).min(to);
                out.push(CssRule {
                    whole: whole_start..end,
                    selector: start..j,
                    body: j + 1..close,
                    at: Some(name.clone()),
                    top,
                });
                if matches!(
                    name.as_str(),
                    "media" | "supports" | "layer" | "container" | "document"
                ) {
                    rules_in(css, j + 1, close, false, out);
                }
                i = end;
            }
        } else {
            let Some(j) = find_delim(css, i, to, b"{;}") else {
                break;
            };
            if b[j] != b'{' {
                // stray text such as `;` or `}`: skip it
                i = j + 1;
                continue;
            }
            let close = matching_brace(css, j, to);
            let end = (close + 1).min(to);
            out.push(CssRule {
                whole: whole_start..end,
                selector: start..j,
                body: j + 1..close,
                at: None,
                top,
            });
            i = end;
        }
    }
}

/// Every rule and at-rule of a stylesheet in document order (rules inside `@media` follow their at-rule).
pub fn css_rules(css: &str) -> Vec<CssRule> {
    let mut out = Vec::new();
    rules_in(css, 0, css.len(), true, &mut out);
    out
}

/// `.name` tokens (the byte range of the name) in the selector `sel` of `css`, outside `[...]` and strings.
fn selector_tokens(css: &str, sel: Range<usize>, marker: u8) -> Vec<Range<usize>> {
    let b = css.as_bytes();
    let mut out = Vec::new();
    let mut i = sel.start;
    let mut bracket = 0;
    while i < sel.end {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = css[i + 2..]
                    .find("*/")
                    .map(|p| i + 2 + p + 2)
                    .unwrap_or(sel.end);
                continue;
            }
            b'[' => bracket += 1,
            b']' => bracket = (bracket - 1).max(0),
            b'"' | b'\'' => {
                let q = b[i];
                i += 1;
                while i < sel.end && b[i] != q {
                    i += 1;
                }
            }
            c if c == marker && bracket == 0 => {
                let s = i + 1;
                let mut e = s;
                for ch in css[s..sel.end].chars() {
                    if is_ident(ch) {
                        e += ch.len_utf8();
                    } else {
                        break;
                    }
                }
                let first = css[s..e].chars().next();
                if e > s && first.is_some_and(is_ident_start) {
                    out.push(s..e);
                    i = e;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// Offset of the first rule of `css` whose selector names the class `name`.
pub fn find_class_rule(css: &str, name: &str) -> Option<usize> {
    if !css.contains(name) {
        return None;
    }
    css_rules(css)
        .into_iter()
        .filter(|r| r.at.is_none())
        .find_map(|r| {
            selector_tokens(css, r.selector, b'.')
                .into_iter()
                .find(|t| &css[t.clone()] == name)
                .map(|t| t.start)
        })
}

/// Offset of the first rule of `css` whose selector names the id `name`.
pub fn find_id_rule(css: &str, name: &str) -> Option<usize> {
    if !css.contains(name) {
        return None;
    }
    css_rules(css)
        .into_iter()
        .filter(|r| r.at.is_none())
        .find_map(|r| {
            selector_tokens(css, r.selector, b'#')
                .into_iter()
                .find(|t| &css[t.clone()] == name)
                .map(|t| t.start)
        })
}

// ---------------------------------------------------------------------------------------------
// hit test
// ---------------------------------------------------------------------------------------------

/// What is under the pointer or the caret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hit {
    /// A file name / link in `href`, `src`, ..., `url()` or `@import` (the whole value, `file#id` included).
    Link { value: String, range: Range<usize> },
    /// A class name: in a `class` attribute (`in_css == false`) or in a selector.
    Class {
        name: String,
        range: Range<usize>,
        in_css: bool,
    },
    /// An id: the value of an `id` attribute or a `#id` selector.
    Id {
        name: String,
        range: Range<usize>,
        in_css: bool,
    },
}

const LINK_ATTRS: &[&str] = &[
    "href",
    "src",
    "xlink:href",
    "poster",
    "data",
    "longdesc",
    "srcset",
];

fn url_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r#"(?:url\(\s*(?:"([^"]*)"|'([^']*)'|([^)\s"']+))\s*\))|(?:@import\s+(?:"([^"]*)"|'([^']*)'))"#,
        )
        .unwrap()
    })
}

fn css_hit(css: &str, pos: usize) -> Option<Hit> {
    let pos = pos.min(css.len());
    // url(...) and @import on the line of the caret
    let ls = css[..pos].rfind('\n').map_or(0, |p| p + 1);
    let le = css[pos..].find('\n').map_or(css.len(), |p| pos + p);
    for c in url_re().captures_iter(&css[ls..le]) {
        if let Some(g) = (1..=5).find_map(|i| c.get(i)) {
            let r = ls + g.start()..ls + g.end();
            if pos >= r.start && pos <= r.end && !r.is_empty() {
                return Some(Hit::Link {
                    value: css[r.clone()].to_string(),
                    range: r,
                });
            }
        }
    }
    for r in css_rules(css).into_iter().filter(|r| r.at.is_none()) {
        if pos < r.selector.start || pos > r.selector.end {
            continue;
        }
        for (marker, class) in [(b'.', true), (b'#', false)] {
            for t in selector_tokens(css, r.selector.clone(), marker) {
                if pos >= t.start.saturating_sub(1) && pos <= t.end {
                    let name = css[t.clone()].to_string();
                    return Some(if class {
                        Hit::Class {
                            name,
                            range: t,
                            in_css: true,
                        }
                    } else {
                        Hit::Id {
                            name,
                            range: t,
                            in_css: true,
                        }
                    });
                }
            }
        }
    }
    None
}

/// `pos` clamped to the text and moved back to a character boundary.
fn floor_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos.min(s.len());
    while !s.is_char_boundary(p) {
        p -= 1;
    }
    p
}

/// What is at byte `pos` of `src`: a link value, a class or an id. `css` tells a stylesheet from markup.
pub fn hit_at(src: &str, pos: usize, css: bool) -> Option<Hit> {
    let pos = floor_boundary(src, pos);
    if css {
        return css_hit(src, pos);
    }
    let mut style_open = false;
    for e in scan(src) {
        match e {
            Event::Open { name, range, .. } => {
                if range.start > pos {
                    break;
                }
                style_open = name == "style";
                if pos >= range.end {
                    continue;
                }
                for a in attr_spans(src, range) {
                    if pos < a.value.start || pos > a.value.end {
                        continue;
                    }
                    let value = &src[a.value.clone()];
                    if a.name == "class" {
                        let w = word_at(value, pos - a.value.start)?;
                        return Some(Hit::Class {
                            name: value[w.clone()].to_string(),
                            range: a.value.start + w.start..a.value.start + w.end,
                            in_css: false,
                        });
                    }
                    if a.name == "id" && !value.is_empty() {
                        return Some(Hit::Id {
                            name: value.to_string(),
                            range: a.value.clone(),
                            in_css: false,
                        });
                    }
                    if LINK_ATTRS.contains(&a.name.as_str()) && !value.trim().is_empty() {
                        return Some(Hit::Link {
                            value: value.to_string(),
                            range: a.value.clone(),
                        });
                    }
                    return None;
                }
                return None;
            }
            Event::Text(r) if style_open && pos >= r.start && pos <= r.end => {
                return css_hit(&src[r.clone()], pos - r.start).map(|h| shift(h, r.start));
            }
            Event::Text(_) => {}
            _ => style_open = false,
        }
    }
    None
}

fn shift(h: Hit, by: usize) -> Hit {
    let sh = |r: Range<usize>| r.start + by..r.end + by;
    match h {
        Hit::Link { value, range } => Hit::Link {
            value,
            range: sh(range),
        },
        Hit::Class {
            name,
            range,
            in_css,
        } => Hit::Class {
            name,
            range: sh(range),
            in_css,
        },
        Hit::Id {
            name,
            range,
            in_css,
        } => Hit::Id {
            name,
            range: sh(range),
            in_css,
        },
    }
}

// ---------------------------------------------------------------------------------------------
// definitions
// ---------------------------------------------------------------------------------------------

/// Offset of the start tag that defines the anchor `id` (`id="…"`, or `<a name="…">`).
pub fn find_id_def(src: &str, id: &str) -> Option<usize> {
    if !src.contains(id) {
        return None;
    }
    for e in scan(src) {
        if let Event::Open { name, range, .. } = e {
            for a in attr_spans(src, range.clone()) {
                if (a.name == "id" || (a.name == "name" && name == "a")) && &src[a.value] == id {
                    return Some(range.start);
                }
            }
        }
    }
    None
}

/// Offset of the first use of the class `name` in a `class` attribute (of the name itself).
pub fn find_class_use(src: &str, name: &str) -> Option<usize> {
    class_uses(src, name).into_iter().next().map(|r| r.start)
}

/// Offset of the first element with `id="name"`: where `#name` of a stylesheet points.
pub fn find_id_use(src: &str, id: &str) -> Option<usize> {
    if !src.contains(id) {
        return None;
    }
    for e in scan(src) {
        if let Event::Open { range, .. } = e {
            for a in attr_spans(src, range) {
                if a.name == "id" && &src[a.value.clone()] == id {
                    return Some(a.value.start);
                }
            }
        }
    }
    None
}

/// Byte ranges of every use of the class in `class` attributes.
pub fn class_uses(src: &str, name: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    if !src.contains(name) {
        return out;
    }
    for e in scan(src) {
        if let Event::Open { range, .. } = e {
            for a in attr_spans(src, range) {
                if a.name != "class" {
                    continue;
                }
                let v = &src[a.value.clone()];
                let mut at = 0;
                for w in v.split_whitespace() {
                    let s = v[at..].find(w).map_or(at, |p| at + p);
                    if w == name {
                        out.push(a.value.start + s..a.value.start + s + w.len());
                    }
                    at = s + w.len();
                }
            }
        }
    }
    out
}

/// Where a link value leads: another file (or the same one), maybe an anchor in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkTarget {
    pub file: String,
    pub fragment: Option<String>,
}

/// Resolves `value` (relative to the file `from`). `None` for external links (`http:`, `mailto:`, ...).
pub fn resolve_link(from: &str, value: &str) -> Option<LinkTarget> {
    let v = value.trim();
    if v.is_empty() || is_external(v) {
        return None;
    }
    let (full, frag) = resolve_href(parent_dir(from), v);
    let file = if full.is_empty() {
        from.to_string()
    } else {
        full
    };
    Some(LinkTarget {
        file,
        fragment: frag,
    })
}

/// Where Ctrl+click leads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Jump {
    /// Open the file and put the caret at the offset.
    To { path: String, offset: usize },
    /// Nothing to go to; the text says why (for a toast).
    Missing(String),
    /// An external link: the caller may offer to open it in the browser.
    External(String),
}

/// Decides where a [`Hit`] in the file `from` (text `from_text`) leads. `names` are the files of the book, `text`
/// returns the text of a text file.
pub fn plan_jump(
    hit: &Hit,
    from: &str,
    from_text: &str,
    names: &[String],
    text: &dyn Fn(&str) -> Option<String>,
) -> Jump {
    match hit {
        Hit::Link { value, .. } => {
            let Some(t) = resolve_link(from, value) else {
                return Jump::External(value.clone());
            };
            if !names.contains(&t.file) {
                return Jump::Missing(t.file);
            }
            let Some(frag) = t.fragment else {
                return Jump::To {
                    path: t.file,
                    offset: 0,
                };
            };
            let body = if t.file == from {
                Some(from_text.to_string())
            } else {
                text(&t.file)
            };
            match body.and_then(|b| find_id_def(&b, &frag)) {
                Some(offset) => Jump::To {
                    path: t.file,
                    offset,
                },
                None => Jump::Missing(format!("#{frag}")),
            }
        }
        Hit::Class {
            name,
            in_css: false,
            ..
        }
        | Hit::Id {
            name,
            in_css: false,
            ..
        } => {
            let is_class = matches!(hit, Hit::Class { .. });
            // stylesheets the file links to first
            let mut sheets: Vec<&String> = names.iter().filter(|n| is_css_path(n)).collect();
            sheets.sort_by_key(|n| !from_text.contains(n.rsplit('/').next().unwrap_or(n)));
            for s in sheets {
                let Some(t) = text(s) else { continue };
                let found = if is_class {
                    find_class_rule(&t, name)
                } else {
                    find_id_rule(&t, name)
                };
                if let Some(offset) = found {
                    return Jump::To {
                        path: s.clone(),
                        offset,
                    };
                }
            }
            Jump::Missing(format!("{}{name}", if is_class { "." } else { "#" }))
        }
        Hit::Class {
            name, in_css: true, ..
        }
        | Hit::Id {
            name, in_css: true, ..
        } => {
            let is_class = matches!(hit, Hit::Class { .. });
            let base = from.rsplit('/').next().unwrap_or(from);
            let mut pages: Vec<(bool, &String)> = names
                .iter()
                .filter(|n| is_markup_path(n))
                .map(|n| (true, n))
                .collect();
            let texts: Vec<Option<String>> = pages.iter().map(|(_, n)| text(n)).collect();
            for (p, t) in pages.iter_mut().zip(&texts) {
                p.0 = !t.as_deref().is_some_and(|t| t.contains(base));
            }
            let mut order: Vec<usize> = (0..pages.len()).collect();
            order.sort_by_key(|i| pages[*i].0);
            for i in order {
                let Some(t) = &texts[i] else { continue };
                let found = if is_class {
                    find_class_use(t, name)
                } else {
                    find_id_use(t, name)
                };
                if let Some(offset) = found {
                    return Jump::To {
                        path: pages[i].1.clone(),
                        offset,
                    };
                }
            }
            Jump::Missing(format!("{}{name}", if is_class { "." } else { "#" }))
        }
    }
}

// ---------------------------------------------------------------------------------------------
// rename a class (CE-343)
// ---------------------------------------------------------------------------------------------

/// Whether `name` can be a CSS class name without escaping.
pub fn valid_class_name(name: &str) -> bool {
    let mut cs = name.chars();
    cs.next().is_some_and(is_ident_start) && name.chars().all(is_ident) && !name.starts_with("--")
}

/// One line that changes, for the preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineChange {
    /// 1-based line number in the file.
    pub line: usize,
    pub before: String,
    pub after: String,
}

/// The new text of one file and what changes in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub new_text: String,
    pub count: usize,
    pub lines: Vec<LineChange>,
}

/// The effect of renaming a class across the book.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClassRename {
    pub changes: Vec<FileChange>,
    pub total: usize,
    /// The new name is used already (the two classes merge).
    pub collides: bool,
}

/// Renames every `.old` selector and every `class="… old …"` use over the given text files (`(path, text)`;
/// only HTML and CSS files are looked at, `<style>` blocks included). Nothing is written.
pub fn rename_class(files: &[(String, String)], old: &str, new: &str) -> Option<ClassRename> {
    if !valid_class_name(old) || !valid_class_name(new) || old == new {
        return None;
    }
    let mut res = ClassRename::default();
    for (path, text) in files {
        let mut edits: Vec<(Range<usize>, &str)> = Vec::new();
        if is_css_path(path) {
            for t in css_class_tokens(text, 0..text.len()) {
                if &text[t.clone()] == old {
                    edits.push((t, new));
                } else if &text[t.clone()] == new {
                    res.collides = true;
                }
            }
        } else if is_markup_path(path) {
            let mut style_open = false;
            for e in scan(text) {
                match e {
                    Event::Open { name, range, .. } => {
                        style_open = name == "style";
                        for a in attr_spans(text, range) {
                            if a.name != "class" {
                                continue;
                            }
                            let v = &text[a.value.clone()];
                            let mut at = 0;
                            for w in v.split_whitespace() {
                                let s = v[at..].find(w).map_or(at, |p| at + p);
                                let r = a.value.start + s..a.value.start + s + w.len();
                                if w == old {
                                    edits.push((r, new));
                                } else if w == new {
                                    res.collides = true;
                                }
                                at = s + w.len();
                            }
                        }
                    }
                    Event::Text(r) if style_open => {
                        for t in css_class_tokens(&text[r.clone()], 0..r.len()) {
                            let t = t.start + r.start..t.end + r.start;
                            if &text[t.clone()] == old {
                                edits.push((t, new));
                            } else if &text[t.clone()] == new {
                                res.collides = true;
                            }
                        }
                    }
                    Event::Text(_) => {}
                    _ => style_open = false,
                }
            }
        } else {
            continue;
        }
        if edits.is_empty() {
            continue;
        }
        edits.sort_by_key(|e| e.0.start);
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        let mut lines = Vec::new();
        for (r, with) in &edits {
            out.push_str(&text[last..r.start]);
            out.push_str(with);
            last = r.end;
        }
        out.push_str(&text[last..]);
        let mut seen = std::collections::BTreeSet::new();
        for (r, _) in &edits {
            let line = text[..r.start].matches('\n').count();
            if seen.insert(line) {
                let after_line = out.lines().nth(line).unwrap_or("").to_string();
                lines.push(LineChange {
                    line: line + 1,
                    before: text.lines().nth(line).unwrap_or("").trim().to_string(),
                    after: after_line.trim().to_string(),
                });
            }
        }
        res.total += edits.len();
        res.changes.push(FileChange {
            path: path.clone(),
            new_text: out,
            count: edits.len(),
            lines,
        });
    }
    Some(res)
}

fn css_class_tokens(css: &str, within: Range<usize>) -> Vec<Range<usize>> {
    let part = &css[within.clone()];
    let mut out = Vec::new();
    for r in css_rules(part).into_iter().filter(|r| r.at.is_none()) {
        for t in selector_tokens(part, r.selector, b'.') {
            out.push(t.start + within.start..t.end + within.start);
        }
    }
    out
}

/// The distinct class names used in `class` attributes of a markup file, sorted.
pub fn classes_used(src: &str) -> Vec<String> {
    let mut v = Vec::new();
    for e in scan(src) {
        if let Event::Open { range, .. } = e {
            for a in attr_spans(src, range) {
                if a.name == "class" {
                    v.extend(src[a.value].split_whitespace().map(str::to_string));
                }
            }
        }
    }
    v.sort();
    v.dedup();
    v
}

// ---------------------------------------------------------------------------------------------
// sort CSS rules (CE-355)
// ---------------------------------------------------------------------------------------------

/// Sorts the plain style rules of a stylesheet by selector, also the ones inside `@media`, `@supports`, `@layer`
/// and `@container` blocks (each block on its own). Runs of rules between at-rules (`@import`, `@media`,
/// `@font-face`, ...) are sorted separately so the at-rules keep their place; a comment directly in front of a
/// rule moves with it. Returns `None` when the order does not change.
pub fn sort_css_rules(css: &str) -> Option<String> {
    let rules = css_rules(css);
    let out = sort_block(css, 0..css.len(), &rules);
    (out != css).then_some(out)
}

fn is_block_container(name: &str) -> bool {
    matches!(
        name,
        "media" | "supports" | "layer" | "container" | "document"
    )
}

/// `css[range]` with the plain rules directly in it sorted and the blocks of container at-rules sorted inside.
fn sort_block(css: &str, range: Range<usize>, rules: &[CssRule]) -> String {
    let mut kids: Vec<&CssRule> = Vec::new();
    for r in rules {
        if r.whole.start >= range.start
            && r.whole.end <= range.end
            && kids.last().is_none_or(|k| r.whole.start >= k.whole.end)
        {
            kids.push(r);
        }
    }
    let mut out = String::with_capacity(range.len());
    let mut last = range.start;
    let mut i = 0;
    while i < kids.len() {
        let k = kids[i];
        if let Some(name) = &k.at {
            out.push_str(&css[last..k.whole.start]);
            if is_block_container(name) && k.body.end < k.whole.end {
                out.push_str(&css[k.whole.start..k.body.start]);
                out.push_str(&sort_block(css, k.body.clone(), rules));
                out.push_str(&css[k.body.end..k.whole.end]);
            } else {
                out.push_str(&css[k.whole.clone()]);
            }
            last = k.whole.end;
            i += 1;
            continue;
        }
        let mut j = i;
        while j < kids.len() && kids[j].at.is_none() {
            j += 1;
        }
        let run = &kids[i..j];
        let mut order: Vec<usize> = (0..run.len()).collect();
        let key = |r: &CssRule| {
            css[r.selector.clone()]
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase()
        };
        order.sort_by(|a, b| key(run[*a]).cmp(&key(run[*b])));
        // keep the separators (whitespace between the rules) in place, swap only the rule texts
        out.push_str(&css[last..run[0].whole.start]);
        for (n, o) in order.iter().enumerate() {
            out.push_str(&css[run[*o].whole.clone()]);
            if n + 1 < run.len() {
                out.push_str(&css[run[n].whole.end..run[n + 1].whole.start]);
            }
        }
        last = run[run.len() - 1].whole.end;
        i = j;
    }
    out.push_str(&css[last..range.end]);
    out
}

// ---------------------------------------------------------------------------------------------
// context help (CE-341)
// ---------------------------------------------------------------------------------------------

/// What the user asks help for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelpTopic {
    Element(String),
    Attribute(String),
    CssProperty(String),
}

impl HelpTopic {
    pub fn label(&self) -> String {
        match self {
            HelpTopic::Element(n) => format!("<{n}>"),
            HelpTopic::Attribute(n) | HelpTopic::CssProperty(n) => n.clone(),
        }
    }
}

fn safe_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 40
        && n.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The MDN page of a topic. The name is restricted to `[a-z0-9-]`, so the URL cannot be bent.
pub fn mdn_url(topic: &HelpTopic) -> Option<String> {
    let base = "https://developer.mozilla.org/en-US/docs/Web";
    match topic {
        HelpTopic::Element(n) if safe_name(n) => Some(format!("{base}/HTML/Element/{n}")),
        HelpTopic::Attribute(n) if safe_name(n) => Some(format!("{base}/HTML/Attributes/{n}")),
        HelpTopic::CssProperty(n) if safe_name(n) => Some(format!("{base}/CSS/{n}")),
        _ => None,
    }
}

/// The tag, attribute or CSS property at `pos`.
pub fn help_topic_at(src: &str, pos: usize, css: bool) -> Option<HelpTopic> {
    let pos = floor_boundary(src, pos);
    let word = |s: &str, at: usize| -> Option<Range<usize>> {
        let at = at.min(s.len());
        let st = s[..at]
            .char_indices()
            .rev()
            .take_while(|(_, c)| c.is_ascii_alphanumeric() || *c == '-')
            .last()
            .map_or(at, |(i, _)| i);
        let en = s[at..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .map_or(s.len(), |p| at + p);
        (st < en).then_some(st..en)
    };
    let css_prop = |s: &str, at: usize| -> Option<HelpTopic> {
        let w = word(s, at)?;
        let after = s[w.end..].trim_start();
        let before = s[..w.start].trim_end();
        // `name:` inside a declaration block
        (after.starts_with(':')
            && (before.ends_with('{') || before.ends_with(';') || before.is_empty()))
        .then(|| HelpTopic::CssProperty(s[w].to_ascii_lowercase()))
    };
    if css {
        return css_prop(src, pos);
    }
    let mut style_open = false;
    for e in scan(src) {
        match e {
            Event::Open { name, range, .. } => {
                if range.start > pos {
                    break;
                }
                style_open = name == "style";
                if pos >= range.end {
                    continue;
                }
                // the tag name (possibly with a prefix)
                if pos
                    <= range.start
                        + 1
                        + src[range.start + 1..range.end]
                            .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
                            .unwrap_or(0)
                {
                    return Some(HelpTopic::Element(name));
                }
                for a in attr_spans(src, range) {
                    let name_start = src[..a.value.start].rfind(&a.name).unwrap_or(a.value.start);
                    if pos >= name_start && pos <= name_start + a.name.len() {
                        return Some(HelpTopic::Attribute(a.name));
                    }
                    if a.name == "style" && pos >= a.value.start && pos <= a.value.end {
                        let v = &src[a.value.clone()];
                        return css_prop(&format!("{{{v}"), pos - a.value.start + 1);
                    }
                }
                return None;
            }
            Event::Close { name, range } => {
                if pos >= range.start && pos < range.end {
                    return Some(HelpTopic::Element(name));
                }
                style_open = false;
            }
            Event::Text(r) if style_open && pos >= r.start && pos <= r.end => {
                return css_prop(&src[r.clone()], pos - r.start);
            }
            Event::Text(_) => {}
            _ => style_open = false,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const HTML: &str = "<html><head><link href=\"../Styles/s.css\" rel=\"stylesheet\"/><style>.note { color: red; }</style></head>\n<body><p class=\"a note\" id=\"p1\">x <a href=\"ch2.xhtml#top\">go</a> <img src='im/a.png' alt=\"\"/></p></body></html>";

    fn at(s: &str, needle: &str, plus: usize) -> usize {
        s.find(needle).unwrap() + plus
    }

    #[test]
    fn attribute_spans_with_quotes_and_bare_values() {
        let s = "<a href=\"x y\" data-k=v hidden class='q'/>";
        let spans = attr_spans(s, 0..s.len());
        let got: Vec<(String, &str)> = spans
            .iter()
            .map(|a| (a.name.clone(), &s[a.value.clone()]))
            .collect();
        assert_eq!(
            got,
            vec![
                ("href".into(), "x y"),
                ("data-k".into(), "v"),
                ("hidden".into(), ""),
                ("class".into(), "q")
            ]
        );
    }

    #[test]
    fn hit_link_class_id_and_style_block() {
        let p = at(HTML, "ch2.xhtml", 3);
        assert!(
            matches!(hit_at(HTML, p, false), Some(Hit::Link { value, .. }) if value == "ch2.xhtml#top")
        );
        let p = at(HTML, "im/a.png", 2);
        assert!(
            matches!(hit_at(HTML, p, false), Some(Hit::Link { value, .. }) if value == "im/a.png")
        );
        let p = at(HTML, "a note", 3);
        match hit_at(HTML, p, false) {
            Some(Hit::Class {
                name,
                in_css,
                range,
            }) => {
                assert_eq!(name, "note");
                assert!(!in_css);
                assert_eq!(&HTML[range], "note");
            }
            h => panic!("{h:?}"),
        }
        let p = at(HTML, "p1", 1);
        assert!(matches!(hit_at(HTML, p, false), Some(Hit::Id { name, .. }) if name == "p1"));
        let p = at(HTML, ".note", 2);
        match hit_at(HTML, p, false) {
            Some(Hit::Class {
                name,
                in_css,
                range,
            }) => {
                assert!(in_css);
                assert_eq!(name, "note");
                assert_eq!(&HTML[range], "note", "ranges map back into the document");
            }
            h => panic!("{h:?}"),
        }
        assert_eq!(hit_at(HTML, at(HTML, "x <a", 0), false), None);
        assert_eq!(hit_at(HTML, 0, false), None);
    }

    #[test]
    fn hit_in_css_url_import_selectors_and_not_values() {
        let css = "@import \"base.css\";\n.a .b:hover, #t > p { background: url('../im/x.png'); margin: 0.5em }\n@media print { .c { color: red } }";
        assert!(
            matches!(hit_at(css, at(css, "base", 1), true), Some(Hit::Link { value, .. }) if value == "base.css")
        );
        assert!(
            matches!(hit_at(css, at(css, "x.png", 1), true), Some(Hit::Link { value, .. }) if value == "../im/x.png")
        );
        assert!(
            matches!(hit_at(css, at(css, ".b", 2), true), Some(Hit::Class { name, .. }) if name == "b")
        );
        assert!(
            matches!(hit_at(css, at(css, "#t", 1), true), Some(Hit::Id { name, .. }) if name == "t")
        );
        assert!(
            matches!(hit_at(css, at(css, "{ .c", 4), true), Some(Hit::Class { name, .. }) if name == "c"),
            "inside @media"
        );
        assert_eq!(
            hit_at(css, at(css, "0.5em", 2), true),
            None,
            "a decimal in a value is not a class"
        );
        assert_eq!(hit_at(css, at(css, "margin", 2), true), None);
    }

    #[test]
    fn css_rules_handle_comments_strings_media_and_braces_in_strings() {
        let css = "/* head */ a { content: \"}\"; }\n/* c */ @media screen { b { x: y } }\n@font-face { src: url(a.ttf) }\n@charset \"utf-8\";\nc, d {}";
        let r = css_rules(css);
        let sels: Vec<String> = r
            .iter()
            .map(|r| css[r.selector.clone()].trim().to_string())
            .collect();
        assert_eq!(
            sels,
            vec![
                "a",
                "@media screen",
                "b",
                "@font-face",
                "@charset \"utf-8\"",
                "c, d"
            ]
        );
        assert!(r[0].top && !r[2].top);
        assert!(
            css[r[0].whole.clone()].starts_with("/* head */"),
            "leading comment belongs to the rule"
        );
        assert_eq!(r[1].at.as_deref(), Some("media"));
        // malformed input never panics
        for s in [
            "a {",
            "}} a { b",
            "@media {",
            "/* open",
            "a { content: \"x",
            "",
        ] {
            let _ = css_rules(s);
        }
    }

    #[test]
    fn find_definitions() {
        let css = "p { x: y }\n.note, .other { a: b }\n#top { c: d }";
        assert_eq!(
            find_class_rule(css, "other"),
            Some(css.find("other").unwrap())
        );
        assert_eq!(find_class_rule(css, "nope"), None);
        assert_eq!(
            find_class_rule("p { margin: 1.note }", "note"),
            None,
            "not in a declaration"
        );
        assert_eq!(find_id_rule(css, "top"), Some(css.find("top").unwrap()));
        assert_eq!(find_id_def(HTML, "p1"), Some(HTML.find("<p ").unwrap()));
        assert_eq!(find_id_def("<a name=\"x\">", "x"), Some(0));
        assert_eq!(find_id_def("<p title=\"x\">", "x"), None);
        assert_eq!(
            find_class_use(HTML, "note"),
            Some(HTML.find("note\"").unwrap())
        );
        assert_eq!(
            find_class_use("<p class=\"notes\">", "note"),
            None,
            "whole words only"
        );
    }

    #[test]
    fn jump_plans() {
        let names: Vec<String> = [
            "OEBPS/Text/c1.xhtml",
            "OEBPS/Text/c2.xhtml",
            "OEBPS/Styles/s.css",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let texts = |p: &str| match p {
            "OEBPS/Text/c1.xhtml" => Some("<p class=\"hl\" id=\"a\">x</p>".to_string()),
            "OEBPS/Text/c2.xhtml" => Some("<h1 id=\"top\">T</h1>".to_string()),
            "OEBPS/Styles/s.css" => Some("p {}\n.hl { x: y }".to_string()),
            _ => None,
        };
        let from = "OEBPS/Text/c1.xhtml";
        let ft = texts(from).unwrap();
        let link = |v: &str| Hit::Link {
            value: v.into(),
            range: 0..0,
        };
        assert_eq!(
            plan_jump(&link("c2.xhtml#top"), from, &ft, &names, &texts),
            Jump::To {
                path: "OEBPS/Text/c2.xhtml".into(),
                offset: 0
            }
        );
        assert_eq!(
            plan_jump(&link("c2.xhtml"), from, &ft, &names, &texts),
            Jump::To {
                path: "OEBPS/Text/c2.xhtml".into(),
                offset: 0
            }
        );
        assert_eq!(
            plan_jump(&link("#a"), from, &ft, &names, &texts),
            Jump::To {
                path: from.into(),
                offset: 0
            }
        );
        assert_eq!(
            plan_jump(&link("c2.xhtml#zz"), from, &ft, &names, &texts),
            Jump::Missing("#zz".into())
        );
        assert_eq!(
            plan_jump(&link("gone.xhtml"), from, &ft, &names, &texts),
            Jump::Missing("OEBPS/Text/gone.xhtml".into())
        );
        assert_eq!(
            plan_jump(&link("https://x.org/"), from, &ft, &names, &texts),
            Jump::External("https://x.org/".into())
        );
        let class = Hit::Class {
            name: "hl".into(),
            range: 0..0,
            in_css: false,
        };
        assert_eq!(
            plan_jump(&class, from, &ft, &names, &texts),
            Jump::To {
                path: "OEBPS/Styles/s.css".into(),
                offset: 6
            }
        );
        let back = Hit::Class {
            name: "hl".into(),
            range: 0..0,
            in_css: true,
        };
        assert_eq!(
            plan_jump(
                &back,
                "OEBPS/Styles/s.css",
                "p {}\n.hl { x: y }",
                &names,
                &texts
            ),
            Jump::To {
                path: from.into(),
                offset: 10
            }
        );
        let missing = Hit::Class {
            name: "zz".into(),
            range: 0..0,
            in_css: false,
        };
        assert_eq!(
            plan_jump(&missing, from, &ft, &names, &texts),
            Jump::Missing(".zz".into())
        );
    }

    #[test]
    fn rename_class_across_html_and_css_with_previews() {
        let files = vec![
            ("a.xhtml".to_string(), "<style>.old{x:y}</style>\n<p class=\"old big\">1</p>\n<p class=\"older old\">2</p>".to_string()),
            ("s.css".to_string(), "/* .old in a comment is a selector-free comment */\n.old, p.old:hover { c: d }\n@media print { .old { e: f } }\n.older { g: h }\na[class=\".old\"] {}".to_string()),
            ("n.ncx".to_string(), "<x class=\"old\"/>".to_string()),
            ("b.xhtml".to_string(), "<p class=\"x\">none</p>".to_string()),
        ];
        let r = rename_class(&files, "old", "fresh").unwrap();
        assert_eq!(r.total, 6, "{r:#?}");
        assert!(!r.collides);
        assert_eq!(
            r.changes.len(),
            2,
            "unrelated and non-markup files are left alone"
        );
        let a = &r.changes[0];
        assert_eq!(a.count, 3);
        assert!(a.new_text.contains("<style>.fresh{x:y}</style>"));
        assert!(a.new_text.contains("class=\"fresh big\""));
        assert!(
            a.new_text.contains("class=\"older fresh\""),
            "whole words only"
        );
        assert_eq!(a.lines[1].line, 2);
        assert_eq!(a.lines[1].after, "<p class=\"fresh big\">1</p>");
        let c = &r.changes[1];
        assert!(c.new_text.contains(".fresh, p.fresh:hover"));
        assert!(c.new_text.contains("@media print { .fresh"));
        assert!(
            c.new_text.contains(".older {"),
            "a longer name is not renamed"
        );
        assert!(
            c.new_text.contains("a[class=\".old\"]"),
            "attribute selector strings are untouched"
        );
        let again = rename_class(&files, "old", "big").unwrap();
        assert!(again.collides);
    }

    #[test]
    fn rename_class_validation_and_unicode() {
        let f = vec![(
            "a.xhtml".to_string(),
            "<p class=\"héros\">é</p>".to_string(),
        )];
        assert!(rename_class(&f, "héros", "x y").is_none());
        assert!(rename_class(&f, "héros", "").is_none());
        assert!(rename_class(&f, "héros", "1a").is_none());
        assert!(rename_class(&f, "héros", "héros").is_none());
        let r = rename_class(&f, "héros", "épée").unwrap();
        assert_eq!(r.changes[0].new_text, "<p class=\"épée\">é</p>");
        assert!(valid_class_name("a-b_c") && !valid_class_name("a b") && !valid_class_name("9"));
    }

    #[test]
    fn sort_rules_sorts_inside_media_blocks_too() {
        let css = "b { x: y }\na { x: y }\n@media (max-width: 600px) {\n  z { c: d }\n  /* m */\n  m { c: d }\n  a { c: d }\n}\n@import url(x);\nd { e: f }\nc { e: f }\n";
        let s = sort_css_rules(css).unwrap();
        assert_eq!(
            s,
            "a { x: y }\nb { x: y }\n@media (max-width: 600px) {\n  a { c: d }\n  /* m */\n  m { c: d }\n  z { c: d }\n}\n@import url(x);\nc { e: f }\nd { e: f }\n"
        );
        assert_eq!(sort_css_rules(&s), None);
        // only the block is out of order; the outer rules stay
        let only = "a { x: y }\n@supports (display: grid) {\n  q { a: b }\n  p { a: b }\n}\n";
        assert_eq!(
            sort_css_rules(only).unwrap(),
            "a { x: y }\n@supports (display: grid) {\n  p { a: b }\n  q { a: b }\n}\n"
        );
        // nested at-rules that are not containers (@font-face inside @media) keep their place; an open block is safe
        let odd = "@media print { b { x: y } a { x: y }";
        let _ = sort_css_rules(odd);
        assert_eq!(sort_css_rules("@media print { }"), None);
    }

    #[test]
    fn sort_rules_keeps_at_rules_and_comments() {
        let css = "@charset \"utf-8\";\n/* z */\nz { a: b }\na { c: d }\n@font-face { src: url(x) }\nm { e: f }\nb { g: h }\n";
        let s = sort_css_rules(css).unwrap();
        assert_eq!(
            s,
            "@charset \"utf-8\";\na { c: d }\n/* z */\nz { a: b }\n@font-face { src: url(x) }\nb { g: h }\nm { e: f }\n"
        );
        assert_eq!(sort_css_rules(&s), None, "sorting twice changes nothing");
        assert_eq!(sort_css_rules("a{}"), None);
        assert_eq!(sort_css_rules(""), None);
        let u = "b { x: y }\nÉA { x: y }\n";
        assert_eq!(
            sort_css_rules(u),
            None,
            "unicode selectors sort without panicking"
        );
    }

    #[test]
    fn help_topics_and_urls() {
        let h = "<div class=\"x\" style=\"color: red; margin: 0\">t</div>";
        assert_eq!(
            help_topic_at(h, 3, false),
            Some(HelpTopic::Element("div".into()))
        );
        assert_eq!(
            help_topic_at(h, 7, false),
            Some(HelpTopic::Attribute("class".into()))
        );
        assert_eq!(
            help_topic_at(h, h.find("margin").unwrap() + 2, false),
            Some(HelpTopic::CssProperty("margin".into()))
        );
        assert_eq!(
            help_topic_at(h, h.find("red").unwrap(), false),
            None,
            "a value is not a property"
        );
        assert_eq!(
            help_topic_at(h, h.find("</div").unwrap() + 3, false),
            Some(HelpTopic::Element("div".into()))
        );
        let css = "p {\n  font-size: 1em;\n}";
        assert_eq!(
            help_topic_at(css, css.find("font").unwrap() + 3, true),
            Some(HelpTopic::CssProperty("font-size".into()))
        );
        assert_eq!(help_topic_at(css, 0, true), None);
        assert_eq!(
            mdn_url(&HelpTopic::Element("blockquote".into())).as_deref(),
            Some("https://developer.mozilla.org/en-US/docs/Web/HTML/Element/blockquote")
        );
        assert_eq!(
            mdn_url(&HelpTopic::CssProperty("font-size".into())).as_deref(),
            Some("https://developer.mozilla.org/en-US/docs/Web/CSS/font-size")
        );
        assert_eq!(
            mdn_url(&HelpTopic::Attribute("href".into())).as_deref(),
            Some("https://developer.mozilla.org/en-US/docs/Web/HTML/Attributes/href")
        );
        assert_eq!(
            mdn_url(&HelpTopic::Element("../../x?y".into())),
            None,
            "names cannot bend the URL"
        );
        assert_eq!(mdn_url(&HelpTopic::Element("A".into())), None);
    }

    #[test]
    fn no_offset_in_unicode_or_broken_text_panics() {
        let docs = [
            "<p class=\"héros ünï\" id=\"é\">日本語 <a href=\"ch2.xhtml#ü\">é</a></p><style>.é { color: red; background: url(é.png) }</style>",
            ".é, #ü > p:not(.ñ) { margin: 0 } @media print { .日本 { x: y } } /* é */ a[href=\"é\"] {",
            "<p class=\"a b\" <b id=",
            "<a href=\"",
            "<style>.a{",
        ];
        for d in docs {
            for pos in 0..=d.len() + 2 {
                for css in [false, true] {
                    let _ = hit_at(d, pos, css);
                    let _ = help_topic_at(d, pos, css);
                }
            }
            let _ = sort_css_rules(d);
            let _ = rename_class(
                &[("a.xhtml".into(), d.into()), ("s.css".into(), d.into())],
                "é",
                "e",
            );
            let _ = classes_used(d);
        }
    }

    #[test]
    fn classes_used_lists_distinct_names() {
        assert_eq!(
            classes_used("<p class=\"b a\"><i class=\"a\"/></p>"),
            vec!["a", "b"]
        );
    }
}
