//! Shared low-level helpers: a forgiving markup scanner, entity decoding, reference rewriting and
//! path renaming with link repair across the whole book.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::OnceLock;

use bookx_core::archive::Archive;
use bookx_core::util::{extension, is_external, parent_dir, relative_href, resolve_href};
use regex::{Captures, Regex};

pub fn is_markup_path(path: &str) -> bool {
    matches!(extension(path).as_str(), "xhtml" | "html" | "htm" | "xht")
}

pub fn is_css_path(path: &str) -> bool {
    extension(path) == "css"
}

/// Files whose text can contain references to other files.
pub fn is_reference_text(path: &str) -> bool {
    matches!(
        extension(path).as_str(),
        "xhtml" | "html" | "htm" | "xht" | "svg" | "css" | "ncx" | "opf" | "xml"
    )
}

/// All XHTML/HTML files in the archive (by extension), in archive order.
pub fn markup_files(archive: &Archive) -> Vec<String> {
    archive
        .names()
        .iter()
        .filter(|n| is_markup_path(n))
        .cloned()
        .collect()
}

/// All CSS files in the archive, in archive order.
pub fn css_files(archive: &Archive) -> Vec<String> {
    archive
        .names()
        .iter()
        .filter(|n| is_css_path(n))
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// Scanner
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Text(Range<usize>),
    Open {
        /// Lower-cased local name (prefix removed).
        name: String,
        range: Range<usize>,
        self_closing: bool,
    },
    Close {
        name: String,
        range: Range<usize>,
    },
    /// Comment, doctype, processing instruction or CDATA.
    Other(Range<usize>),
}

fn local_lower(qname: &str) -> String {
    qname
        .rsplit(':')
        .next()
        .unwrap_or(qname)
        .to_ascii_lowercase()
}

/// Index of the `>` that ends the tag starting at `start` (quotes respected), or `None`.
fn tag_end(b: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    let mut quote = 0u8;
    while i < b.len() {
        let c = b[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'>' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Byte index just after the `>` of the start tag beginning at `start` (`src[start] == '<'`).
pub fn start_tag_end(src: &str, start: usize) -> usize {
    tag_end(src.as_bytes(), start)
        .map(|p| p + 1)
        .unwrap_or(src.len())
}

fn find_from(src: &str, from: usize, pat: &str) -> Option<usize> {
    src[from..].find(pat).map(|p| from + p)
}

/// Tokenises markup without requiring it to be well formed. The content of `<script>` and `<style>` is
/// reported as a single `Text` event between the open and close events.
pub fn scan(src: &str) -> Vec<Event> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut text_start = 0;
    macro_rules! flush {
        ($end:expr_2021) => {
            if $end > text_start {
                out.push(Event::Text(text_start..$end));
            }
        };
    }
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &src[i..];
        if rest.starts_with("<!--") {
            flush!(i);
            let end = find_from(src, i + 4, "-->")
                .map(|p| p + 3)
                .unwrap_or(src.len());
            out.push(Event::Other(i..end));
            i = end;
            text_start = i;
        } else if rest.starts_with("<![CDATA[") {
            flush!(i);
            let end = find_from(src, i + 9, "]]>")
                .map(|p| p + 3)
                .unwrap_or(src.len());
            out.push(Event::Other(i..end));
            i = end;
            text_start = i;
        } else if rest.starts_with("<!") || rest.starts_with("<?") {
            flush!(i);
            let end = tag_end(b, i).map(|p| p + 1).unwrap_or(src.len());
            out.push(Event::Other(i..end));
            i = end;
            text_start = i;
        } else if rest.starts_with("</")
            && b.get(i + 2)
                .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
        {
            flush!(i);
            let closed = tag_end(b, i);
            let end = closed.map(|p| p + 1).unwrap_or(src.len());
            let inner = src[i + 2..closed.unwrap_or(src.len())].trim();
            let qn = inner.split_whitespace().next().unwrap_or("");
            out.push(Event::Close {
                name: local_lower(qn),
                range: i..end,
            });
            i = end;
            text_start = i;
        } else if b
            .get(i + 1)
            .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
        {
            flush!(i);
            let closed = tag_end(b, i);
            let end = closed.map(|p| p + 1).unwrap_or(src.len());
            let body = &src[i + 1..closed.unwrap_or(src.len())];
            let qn_len = body
                .find(|c: char| c.is_whitespace() || c == '/')
                .unwrap_or(body.len());
            let name = local_lower(&body[..qn_len]);
            let self_closing = body.trim_end().ends_with('/');
            out.push(Event::Open {
                name: name.clone(),
                range: i..end,
                self_closing,
            });
            i = end;
            text_start = i;
            if !self_closing && (name == "script" || name == "style") {
                let close = format!("</{name}");
                let lower = src[i..].to_ascii_lowercase();
                let at = lower.find(&close).map(|p| i + p).unwrap_or(src.len());
                if at > i {
                    out.push(Event::Text(i..at));
                }
                i = at;
                text_start = i;
            }
        } else {
            i += 1;
        }
    }
    flush!(b.len());
    out
}

pub const BLOCK_TAGS: &[&str] = &[
    "p",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "ul",
    "ol",
    "br",
    "tr",
    "td",
    "th",
    "table",
    "blockquote",
    "section",
    "article",
    "aside",
    "header",
    "footer",
    "nav",
    "pre",
    "hr",
    "dt",
    "dd",
    "dl",
    "figure",
    "figcaption",
    "body",
    "title",
    "caption",
];

pub const DEFAULT_SKIP: &[&str] = &["script", "style", "head"];

/// Text runs (source ranges) outside the `skip` elements.
pub fn text_runs(src: &str, skip: &[&str]) -> Vec<Range<usize>> {
    let mut depth: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for ev in scan(src) {
        match ev {
            Event::Open {
                name, self_closing, ..
            } => {
                if !self_closing && skip.contains(&name.as_str()) {
                    depth.push(name);
                }
            }
            Event::Close { name, .. } => {
                if depth.last() == Some(&name) {
                    depth.pop();
                }
            }
            Event::Text(r) => {
                if depth.is_empty() {
                    out.push(r);
                }
            }
            Event::Other(_) => {}
        }
    }
    out
}

/// Plain text of a markup document: entities decoded, block elements separated by newlines, script/style/head skipped.
pub fn plain_text(src: &str) -> String {
    let mut depth: Vec<String> = Vec::new();
    let mut out = String::new();
    for ev in scan(src) {
        match ev {
            Event::Open {
                name, self_closing, ..
            } => {
                if !self_closing && DEFAULT_SKIP.contains(&name.as_str()) {
                    depth.push(name.clone());
                }
                if depth.is_empty() && BLOCK_TAGS.contains(&name.as_str()) {
                    out.push('\n');
                }
            }
            Event::Close { name, .. } => {
                if depth.last() == Some(&name) {
                    depth.pop();
                }
                if depth.is_empty() && BLOCK_TAGS.contains(&name.as_str()) {
                    out.push('\n');
                }
            }
            Event::Text(r) => {
                if depth.is_empty() {
                    out.push_str(&decode_entities(&src[r]));
                }
            }
            Event::Other(_) => {}
        }
    }
    out
}

/// `plain_text` with whitespace runs collapsed and the result trimmed (for equality checks).
pub fn normalized_text(src: &str) -> String {
    plain_text(src)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn named_entity(name: &str) -> Option<char> {
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "ndash" => '–',
        "mdash" => '—',
        "hellip" => '…',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "laquo" => '«',
        "raquo" => '»',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "eacute" => 'é',
        "egrave" => 'è',
        "agrave" => 'à',
        "uuml" => 'ü',
        "ouml" => 'ö',
        "auml" => 'ä',
        "szlig" => 'ß',
        "ccedil" => 'ç',
        "ntilde" => 'ñ',
        "shy" => '\u{ad}',
        "times" => '×',
        "deg" => '°',
        _ => return None,
    })
}

/// Decodes character references. Unknown named entities are kept verbatim.
pub fn decode_entities(raw: &str) -> String {
    decode_with_offsets(raw).0
}

/// Decodes character references and returns, for every byte of the output, the byte offset into `raw`
/// where the character that produced it starts. A final entry maps the end of the output to `raw.len()`.
pub fn decode_with_offsets(raw: &str) -> (String, Vec<usize>) {
    let mut out = String::with_capacity(raw.len());
    let mut map = Vec::with_capacity(raw.len() + 1);
    let b = raw.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'&' {
            if let Some(semi) = raw[i..].find(';').filter(|s| *s <= 10) {
                let name = &raw[i + 1..i + semi];
                let ch = if let Some(num) = name.strip_prefix('#') {
                    let v = if let Some(h) = num.strip_prefix(['x', 'X']) {
                        u32::from_str_radix(h, 16).ok()
                    } else {
                        num.parse::<u32>().ok()
                    };
                    v.and_then(char::from_u32)
                } else {
                    named_entity(name)
                };
                if let Some(ch) = ch {
                    let before = out.len();
                    out.push(ch);
                    for _ in before..out.len() {
                        map.push(i);
                    }
                    i += semi + 1;
                    continue;
                }
            }
        }
        let ch = raw[i..].chars().next().unwrap_or('\u{fffd}');
        let before = out.len();
        out.push(ch);
        for _ in before..out.len() {
            map.push(i);
        }
        i += ch.len_utf8();
    }
    map.push(raw.len());
    (out, map)
}

// ---------------------------------------------------------------------------
// Reference rewriting
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    /// XHTML, HTML, SVG, OPF, NCX and other XML: attributes plus `url()` in `style` attributes and `<style>` blocks.
    Markup,
    /// Stylesheets: `url()` and `@import`.
    Css,
}

pub fn ref_kind(path: &str) -> RefKind {
    if is_css_path(path) {
        RefKind::Css
    } else {
        RefKind::Markup
    }
}

fn tag_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r#"<[A-Za-z_][^>"']*(?:(?:"[^"]*"|'[^']*')[^>"']*)*>"#).unwrap())
}

fn attr_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r#"(\s)((?:xlink:)?href|src|poster|data|longdesc)(\s*=\s*)(?:"([^"]*)"|'([^']*)')"#,
        )
        .unwrap()
    })
}

fn url_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r#"(?i)url\(\s*(?:"([^"]*)"|'([^']*)'|([^)\s"']*))\s*\)|@import\s+(?:"([^"]*)"|'([^']*)')"#)
            .unwrap()
    })
}

fn style_block_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r#"(?is)(<style\b[^>]*>)(.*?)(</style)"#).unwrap())
}

fn unescape_attr(v: &str) -> String {
    v.replace("&amp;", "&")
}

fn escape_attr(v: &str, quote: char) -> String {
    let v = v.replace('&', "&amp;").replace('<', "&lt;");
    if quote == '"' {
        v.replace('"', "&quot;")
    } else {
        v.replace('\'', "&apos;")
    }
}

/// Rewrites `url(...)` and `@import` references in CSS text.
pub fn map_css_refs(css: &str, f: &mut dyn FnMut(&str) -> Option<String>) -> String {
    url_re()
        .replace_all(css, |c: &Captures| {
            let whole = c.get(0).unwrap().as_str();
            let (idx, val) = (1..=5)
                .find_map(|i| c.get(i).map(|m| (i, m)))
                .map(|(i, m)| (i, m.as_str()))
                .unwrap_or((3, ""));
            if val.is_empty() {
                return whole.to_string();
            }
            match f(val) {
                None => whole.to_string(),
                Some(new) => match idx {
                    1 => whole.replacen(&format!("\"{val}\""), &format!("\"{new}\""), 1),
                    2 => whole.replacen(&format!("'{val}'"), &format!("'{new}'"), 1),
                    4 => whole.replacen(&format!("\"{val}\""), &format!("\"{new}\""), 1),
                    5 => whole.replacen(&format!("'{val}'"), &format!("'{new}'"), 1),
                    _ => {
                        if new.contains([' ', ')', '(', '\'', '"']) {
                            format!("url(\"{new}\")")
                        } else {
                            format!("url({new})")
                        }
                    }
                },
            }
        })
        .into_owned()
}

/// Calls `f` for every file reference in `text` (attribute values are given with `&amp;` decoded) and replaces it
/// when `f` returns `Some(new)`. Text outside tags is never touched, so code samples are safe.
pub fn map_refs(text: &str, kind: RefKind, f: &mut dyn FnMut(&str) -> Option<String>) -> String {
    if kind == RefKind::Css {
        return map_css_refs(text, f);
    }
    let after_tags = tag_re().replace_all(text, |c: &Captures| {
        let tag = c.get(0).unwrap().as_str();
        let tag2 = attr_re().replace_all(tag, |a: &Captures| {
            let whole = a.get(0).unwrap().as_str();
            let (val, quote) = match (a.get(4), a.get(5)) {
                (Some(m), _) => (m.as_str(), '"'),
                (_, Some(m)) => (m.as_str(), '\''),
                _ => return whole.to_string(),
            };
            let decoded = unescape_attr(val);
            match f(&decoded) {
                Some(new) if new != decoded => format!(
                    "{}{}{}{q}{}{q}",
                    &a[1],
                    &a[2],
                    &a[3],
                    escape_attr(&new, quote),
                    q = quote
                ),
                _ => whole.to_string(),
            }
        });
        if tag2.contains("url(") || tag2.contains("URL(") {
            map_css_refs(&tag2, f)
        } else {
            tag2.into_owned()
        }
    });
    if !after_tags.contains("<style") && !after_tags.contains("<STYLE") {
        return after_tags.into_owned();
    }
    style_block_re()
        .replace_all(&after_tags, |c: &Captures| {
            format!("{}{}{}", &c[1], map_css_refs(&c[2], f), &c[3])
        })
        .into_owned()
}

fn any_attr_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r#"([A-Za-z_:][-A-Za-z0-9_:.]*)\s*=\s*(?:"([^"]*)"|'([^']*)')"#).unwrap()
    })
}

/// Attributes of a start tag as `(name, decoded value)` in source order. Names are kept as written.
pub fn tag_attrs(tag: &str) -> Vec<(String, String)> {
    let inner = tag
        .trim_start_matches('<')
        .split_once(|c: char| c.is_whitespace())
        .map(|(_, r)| r)
        .unwrap_or("");
    any_attr_re()
        .captures_iter(inner)
        .map(|c| {
            let v = c
                .get(2)
                .or_else(|| c.get(3))
                .map(|m| m.as_str())
                .unwrap_or("");
            (c[1].to_string(), decode_entities(v))
        })
        .collect()
}

/// Value of the attribute `name` (case-insensitive, prefix-sensitive) in a start tag.
pub fn tag_attr(tag: &str, name: &str) -> Option<String> {
    tag_attrs(tag)
        .into_iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v)
}

/// Rewrites attribute values of one start tag. `f(name, decoded_value)` returns the new value to substitute.
/// Attributes are matched by the name as written; quotes are preserved.
pub fn rewrite_tag_attrs(tag: &str, f: &mut dyn FnMut(&str, &str) -> Option<String>) -> String {
    any_attr_re()
        .replace_all(tag, |c: &Captures| {
            let whole = c.get(0).unwrap().as_str();
            let (m, quote) = match (c.get(2), c.get(3)) {
                (Some(m), _) => (m, '"'),
                (_, Some(m)) => (m, '\''),
                _ => return whole.to_string(),
            };
            let decoded = decode_entities(m.as_str());
            match f(&c[1], &decoded) {
                Some(new) if new != decoded => format!(
                    "{}{}{}",
                    &whole[..m.start() - c.get(0).unwrap().start()],
                    escape_attr(&new, quote),
                    &whole[m.end() - c.get(0).unwrap().start()..]
                ),
                _ => whole.to_string(),
            }
        })
        .into_owned()
}

/// Applies [`rewrite_tag_attrs`] to every start tag in `text`.
pub fn rewrite_all_tags(
    text: &str,
    f: &mut dyn FnMut(&str, &str, &str) -> Option<String>,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for ev in scan(text) {
        if let Event::Open { name, range, .. } = ev {
            out.push_str(&text[last..range.start]);
            let tag = &text[range.clone()];
            out.push_str(&rewrite_tag_attrs(tag, &mut |a, v| f(&name, a, v)));
            last = range.end;
        }
    }
    out.push_str(&text[last..]);
    out
}

/// Sets (or adds) an attribute in a start tag text, keeping everything else byte-identical.
pub fn set_tag_attr(tag: &str, name: &str, value: &str) -> String {
    let esc = escape_attr(value, '"');
    let re = Regex::new(&format!(
        r#"(\s{}\s*=\s*)(?:"[^"]*"|'[^']*')"#,
        regex::escape(name)
    ))
    .unwrap();
    if re.is_match(tag) {
        return re
            .replace(tag, |c: &Captures| format!("{}\"{}\"", &c[1], esc))
            .into_owned();
    }
    let trimmed_end = tag.trim_end_matches('>');
    let (head, tail) = if let Some(h) = trimmed_end.strip_suffix('/') {
        (h.trim_end().to_string(), "/>")
    } else {
        (trimmed_end.to_string(), ">")
    };
    format!("{head} {name}=\"{esc}\"{tail}")
}

/// Removes an attribute from a start tag text.
pub fn remove_tag_attr(tag: &str, name: &str) -> String {
    let re = Regex::new(&format!(
        r#"\s{}\s*=\s*(?:"[^"]*"|'[^']*')"#,
        regex::escape(name)
    ))
    .unwrap();
    re.replace(tag, "").into_owned()
}

/// Qualified element name of a start or end tag text (`<h:p class="x">` -> `h:p`).
pub fn tag_qname(tag: &str) -> &str {
    let t = tag.trim_start_matches('<').trim_start_matches('/');
    let end = t
        .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
        .unwrap_or(t.len());
    &t[..end]
}

/// Body content range `(start, end)` of an XHTML source: just after `<body ...>` and just before `</body>`.
pub fn body_bounds(src: &str) -> Option<(usize, usize)> {
    let mut start = None;
    let mut end = None;
    for ev in scan(src) {
        match ev {
            Event::Open {
                name,
                range,
                self_closing: false,
            } if name == "body" && start.is_none() => start = Some(range.end),
            Event::Close { name, range } if name == "body" => end = Some(range.start),
            _ => {}
        }
    }
    match (start, end) {
        (Some(s), Some(e)) if s <= e => Some((s, e)),
        _ => None,
    }
}

/// Every `id` / `name` anchor defined in a markup document.
pub fn defined_ids(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for ev in scan(src) {
        if let Event::Open { range, name, .. } = ev {
            let tag = &src[range];
            if let Some(id) = tag_attr(tag, "id") {
                out.push(id);
            }
            if name == "a" {
                if let Some(n) = tag_attr(tag, "name") {
                    out.push(n);
                }
            }
        }
    }
    out
}

/// Applies [`map_css_refs`] to the content of every `<style>` block in a markup document.
pub fn map_style_blocks(text: &str, f: &mut dyn FnMut(&str) -> Option<String>) -> String {
    if !text.contains("<style") && !text.contains("<STYLE") {
        return text.to_string();
    }
    style_block_re()
        .replace_all(text, |c: &Captures| {
            format!("{}{}{}", &c[1], map_css_refs(&c[2], f), &c[3])
        })
        .into_owned()
}

/// Replaces the content of every `<style>` block with `f(content)`.
pub fn map_style_contents(text: &str, f: &mut dyn FnMut(&str) -> String) -> String {
    if !text.contains("<style") && !text.contains("<STYLE") {
        return text.to_string();
    }
    style_block_re()
        .replace_all(text, |c: &Captures| {
            format!("{}{}{}", &c[1], f(&c[2]), &c[3])
        })
        .into_owned()
}

/// Collects every file reference (decoded) in `text`.
pub fn collect_refs(text: &str, kind: RefKind) -> Vec<String> {
    let mut v = Vec::new();
    map_refs(text, kind, &mut |r| {
        v.push(r.to_string());
        None
    });
    v
}

/// Whether the reference points to a local file (not external, not a pure fragment, no query).
pub fn is_local_ref(href: &str) -> bool {
    let h = href.trim();
    !h.is_empty() && !h.starts_with('#') && !is_external(h) && !has_scheme(h)
}

fn has_scheme(h: &str) -> bool {
    match h.find(':') {
        Some(p) => {
            p > 0
                && h[..p]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
                && !h[..p].contains('/')
        }
        None => false,
    }
}

/// Moves files inside the archive and repairs every reference to them (and every relative reference *from*
/// them) in markup, CSS, OPF, NCX and SVG. `map` is `old path -> new path`. References that do not resolve to an
/// existing file are left untouched. Returns the number of files whose text changed.
pub fn rename_paths(archive: &mut Archive, map: &HashMap<String, String>) -> usize {
    let map: HashMap<&String, &String> = map.iter().filter(|(a, b)| a != b).collect();
    if map.is_empty() {
        return 0;
    }
    let names: Vec<String> = archive.names().to_vec();
    let existing: std::collections::HashSet<&str> = names.iter().map(|s| s.as_str()).collect();
    let mut updates: Vec<(String, String)> = Vec::new();
    for name in &names {
        if !is_reference_text(name) {
            continue;
        }
        let Some(text) = archive.get_string(name) else {
            continue;
        };
        let new_name = map.get(name).map(|s| s.as_str()).unwrap_or(name.as_str());
        let dir = parent_dir(name);
        let new = map_refs(&text, ref_kind(name), &mut |href| {
            if !is_local_ref(href) || href.contains('?') {
                return None;
            }
            let (path, frag) = resolve_href(dir, href);
            if path.is_empty() {
                return None;
            }
            let target_new = match map.get(&path) {
                Some(t) => t.as_str(),
                None if existing.contains(path.as_str()) => path.as_str(),
                None => return None,
            };
            if target_new == path && new_name == name {
                return None;
            }
            let mut to = target_new.to_string();
            if let Some(f) = frag {
                to.push('#');
                to.push_str(&f);
            }
            Some(relative_href(new_name, &to))
        });
        if new != text {
            updates.push((name.clone(), new));
        }
    }
    let changed = updates.len();
    for (name, text) in updates {
        archive.set_string(&name, &text);
    }
    let mut moved: Vec<(String, Vec<u8>)> = Vec::new();
    for old in map.keys() {
        if let Some(data) = archive.get(old).map(|d| d.to_vec()) {
            archive.remove(old);
            moved.push((map[old].clone(), data));
        }
    }
    moved.sort_by(|a, b| a.0.cmp(&b.0));
    for (n, d) in moved {
        archive.set(&n, d);
    }
    changed
}

/// Replaces `old` with `new` as href/src target while ignoring fragments: used to retarget links in all files.
pub fn retarget_links(
    archive: &mut Archive,
    mut f: impl FnMut(&str, &str, Option<&str>) -> Option<String>,
) -> usize {
    let names: Vec<String> = archive.names().to_vec();
    let mut changed = 0;
    for name in &names {
        if !is_reference_text(name) {
            continue;
        }
        let Some(text) = archive.get_string(name) else {
            continue;
        };
        let dir = parent_dir(name).to_string();
        let new = map_refs(&text, ref_kind(name), &mut |href| {
            if href.trim().is_empty() || is_external(href) || has_scheme(href) {
                return None;
            }
            let (path, frag) = resolve_href(&dir, href);
            let path = if path.is_empty() { name.clone() } else { path };
            let new_target = f(name, &path, frag.as_deref())?;
            Some(relative_href(name, &new_target))
        });
        if new != text {
            archive.set_string(name, &new);
            changed += 1;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_basic_events() {
        let ev = scan("<p class=\"a>b\">Hi<br/></p><!-- c --><?x?>");
        assert!(matches!(&ev[0], Event::Open{name, ..} if name == "p"));
        assert!(matches!(&ev[1], Event::Text(r) if *r == (15..17)));
        assert!(matches!(&ev[2], Event::Open{name, self_closing: true, ..} if name == "br"));
        assert!(matches!(&ev[3], Event::Close{name, ..} if name == "p"));
        assert!(matches!(&ev[4], Event::Other(_)));
        assert!(matches!(&ev[5], Event::Other(_)));
    }

    #[test]
    fn scan_script_content_is_one_text() {
        let src = "<script>if (a<b) { x('</p>') }</script><p>t</p>";
        let ev = scan(src);
        assert!(matches!(&ev[1], Event::Text(r) if src[r.clone()].starts_with("if (a<b)")));
        let runs = text_runs(src, DEFAULT_SKIP);
        assert_eq!(runs.len(), 1);
        assert_eq!(&src[runs[0].clone()], "t");
    }

    #[test]
    fn scan_unterminated_and_stray_lt() {
        let ev = scan("a < b <p");
        assert!(!ev.is_empty());
        let _ = scan("<!-- never closed");
        let _ = scan("<![CDATA[ x");
        let _ = scan("</");
        let _ = scan("<");
        assert_eq!(plain_text("1 < 2"), "1 < 2");
    }

    #[test]
    fn scan_unterminated_tags_with_multibyte_text_regression() {
        // used to slice inside a multi-byte character
        for src in [
            "</a \u{feff}",
            "<a \u{feff}",
            "<p>日本</p",
            "<b 😀",
            "</é",
            "<é>",
        ] {
            let ev = scan(src);
            let covered: usize = ev
                .iter()
                .map(|e| match e {
                    Event::Text(r) | Event::Other(r) => r.len(),
                    Event::Open { range, .. } | Event::Close { range, .. } => range.len(),
                })
                .sum();
            assert_eq!(covered, src.len(), "events tile the input: {src:?}");
        }
    }

    #[test]
    fn path_kind_helpers() {
        assert!(is_markup_path("a/B.XHTML") && is_markup_path("x.htm") && !is_markup_path("x.css"));
        assert!(is_css_path("a.CSS") && !is_css_path("a.xhtml"));
        assert!(
            is_reference_text("a.opf") && is_reference_text("a.ncx") && is_reference_text("a.svg")
        );
        assert!(!is_reference_text("a.png"));
        assert_eq!(ref_kind("a.css"), RefKind::Css);
        assert_eq!(ref_kind("a.xhtml"), RefKind::Markup);
        let mut a = Archive::default();
        a.set_string("b.xhtml", "");
        a.set_string("a.css", "");
        a.set_string("c.png", "");
        a.set_string("d.html", "");
        assert_eq!(markup_files(&a), vec!["b.xhtml", "d.html"]);
        assert_eq!(css_files(&a), vec!["a.css"]);
    }

    #[test]
    fn style_block_mapping() {
        let src = "<p style=\"color:red\">x</p><style>a{b:url(x.png)}</style><STYLE type=\"text/css\">c{d:url(y.png)}</STYLE>";
        let mut seen = Vec::new();
        let out = map_style_blocks(src, &mut |r| {
            seen.push(r.to_string());
            Some(format!("n/{r}"))
        });
        assert_eq!(seen, vec!["x.png", "y.png"]);
        assert!(out.contains("<style>a{b:url(n/x.png)}</style>"));
        assert!(out.contains("url(n/y.png)}</STYLE>"));
        let upper = map_style_contents(src, &mut |c| c.to_uppercase());
        assert!(upper.contains("<style>A{B:URL(X.PNG)}</style>"));
        assert!(upper.contains("style=\"color:red\""));
        assert_eq!(map_style_contents("<p/>", &mut |_| unreachable!()), "<p/>");
        assert_eq!(map_style_blocks("<p/>", &mut |_| unreachable!()), "<p/>");
    }

    #[test]
    fn plain_text_blocks_and_skips() {
        let src = "<html><head><title>T</title><style>p{}</style></head><body><p>one<b>two</b></p><p>x &amp; y&#233;&#x41;&nbsp;</p><pre>q</pre></body></html>";
        let t = plain_text(src);
        assert!(t.contains("onetwo"));
        assert!(!t.contains("p{}"));
        assert!(!t.contains('T') || !t.starts_with('T'));
        assert!(t.contains("x & yéA\u{a0}"));
        assert_eq!(normalized_text(src), "onetwo x & yéA q");
    }

    #[test]
    fn entity_decoding_and_offsets() {
        assert_eq!(
            decode_entities("a&amp;b&unknown;c&#xZZ;"),
            "a&b&unknown;c&#xZZ;"
        );
        assert_eq!(decode_entities("&lt;&gt;&quot;&apos;"), "<>\"'");
        let (s, m) = decode_with_offsets("a&amp;é");
        assert_eq!(s, "a&é");
        assert_eq!(m[0], 0);
        assert_eq!(m[1], 1);
        assert_eq!(m[2], 6);
        assert_eq!(*m.last().unwrap(), "a&amp;é".len());
        assert_eq!(m.len(), s.len() + 1);
    }

    #[test]
    fn map_refs_attributes_and_style() {
        let src = "<a href=\"x.xhtml#a\">t</a> text href=\"no.png\" <img src='i.png' alt=\"a\"/><div style=\"background:url(b.png)\"/><style>@import 'c.css'; p{background:url(\"d.png\")}</style><a data-href=\"z\"/>";
        let mut seen = Vec::new();
        let out = map_refs(src, RefKind::Markup, &mut |r| {
            seen.push(r.to_string());
            Some(format!("N/{r}"))
        });
        assert_eq!(seen, vec!["x.xhtml#a", "i.png", "b.png", "c.css", "d.png"]);
        assert!(out.contains("href=\"N/x.xhtml#a\""));
        assert!(out.contains("text href=\"no.png\""));
        assert!(out.contains("src='N/i.png'"));
        assert!(out.contains("url(N/b.png)"));
        assert!(out.contains("@import 'N/c.css'"));
        assert!(out.contains("url(\"N/d.png\")"));
        assert!(out.contains("data-href=\"z\""));
    }

    #[test]
    fn map_refs_entities_roundtrip() {
        let out = map_refs(
            "<a href=\"a.html?x=1&amp;y=2\">",
            RefKind::Markup,
            &mut |r| {
                assert_eq!(r, "a.html?x=1&y=2");
                Some("b.html?x=1&y=2".into())
            },
        );
        assert_eq!(out, "<a href=\"b.html?x=1&amp;y=2\">");
    }

    #[test]
    fn map_css_refs_forms() {
        let css = "a{background:url( x.png )} @import \"y.css\"; b{src:url('f.ttf')}";
        let out = map_css_refs(css, &mut |r| Some(format!("../{r}")));
        assert!(out.contains("url(../x.png)"));
        assert!(out.contains("@import \"../y.css\""));
        assert!(out.contains("url('../f.ttf')"));
        // unquoted new value with a space gets quoted
        let out = map_css_refs("a{b:url(x.png)}", &mut |_| Some("my file.png".into()));
        assert!(out.contains("url(\"my file.png\")"));
        assert_eq!(
            map_css_refs("a{b:url()}", &mut |_| Some("x".into())),
            "a{b:url()}"
        );
    }

    #[test]
    fn tag_attrs_parsing() {
        let t = "<a href=\"x.html?a=1&amp;b=2\" Class='k' data-x = \"y\" disabled>";
        let v = tag_attrs(t);
        assert_eq!(v.len(), 3);
        assert_eq!(tag_attr(t, "HREF").as_deref(), Some("x.html?a=1&b=2"));
        assert_eq!(tag_attr(t, "class").as_deref(), Some("k"));
        assert_eq!(tag_attr(t, "data-x").as_deref(), Some("y"));
        assert_eq!(tag_attr(t, "id"), None);
        assert!(tag_attrs("<br/>").is_empty());
    }

    #[test]
    fn set_and_remove_tag_attr() {
        assert_eq!(
            set_tag_attr("<img src=\"a\"/>", "alt", ""),
            "<img src=\"a\" alt=\"\"/>"
        );
        assert_eq!(
            set_tag_attr("<img src=\"a\" />", "alt", "x"),
            "<img src=\"a\" alt=\"x\"/>"
        );
        assert_eq!(
            set_tag_attr("<p class=\"c\">", "lang", "en"),
            "<p class=\"c\" lang=\"en\">"
        );
        assert_eq!(
            set_tag_attr("<p lang='fr'>", "lang", "en"),
            "<p lang=\"en\">"
        );
        assert_eq!(
            set_tag_attr("<p>", "title", "a\"&b"),
            "<p title=\"a&quot;&amp;b\">"
        );
        assert_eq!(
            remove_tag_attr("<p class=\"c\" id='x'>", "id"),
            "<p class=\"c\">"
        );
        assert_eq!(remove_tag_attr("<p>", "id"), "<p>");
    }

    #[test]
    fn rewrite_attrs_and_bounds() {
        let t = "<a href='#o' id=\"o\" title=\"a &amp; b\">";
        let out = rewrite_tag_attrs(t, &mut |n, v| {
            (n == "id" || (n == "href" && v == "#o")).then(|| {
                if n == "id" {
                    "n".to_string()
                } else {
                    "#n".to_string()
                }
            })
        });
        assert_eq!(out, "<a href='#n' id=\"n\" title=\"a &amp; b\">");
        let doc = "<p id=\"x\">t</p><a id=\"x\"/>";
        let out = rewrite_all_tags(doc, &mut |tag, a, v| {
            (tag == "a" && a == "id").then(|| format!("{v}2"))
        });
        assert_eq!(out, "<p id=\"x\">t</p><a id=\"x2\"/>");
        assert_eq!(tag_qname("<h:p class=\"x\">"), "h:p");
        assert_eq!(tag_qname("</div>"), "div");
        assert_eq!(tag_qname("<br/>"), "br");
        let s = "<html><body class=\"c\"><p/></body></html>";
        let (a, b) = body_bounds(s).unwrap();
        assert_eq!(&s[a..b], "<p/>");
        assert_eq!(body_bounds("<html/>"), None);
        assert_eq!(body_bounds("<body/>"), None);
    }

    #[test]
    fn defined_ids_lists_id_and_anchor_names() {
        let ids = defined_ids("<p id=\"a\"><a name=\"b\"/><span name=\"c\"/><div id='d'/></p>");
        assert_eq!(ids, vec!["a", "b", "d"]);
    }

    #[test]
    fn collect_refs_lists_all() {
        let v = collect_refs(
            "<link href=\"a.css\"/><img src=\"b.png\"/>",
            RefKind::Markup,
        );
        assert_eq!(v, vec!["a.css", "b.png"]);
        assert_eq!(collect_refs("a{b:url(c.png)}", RefKind::Css), vec!["c.png"]);
    }

    #[test]
    fn local_ref_classification() {
        assert!(is_local_ref("a/b.png"));
        assert!(is_local_ref("../a.xhtml#x"));
        assert!(!is_local_ref("#x"));
        assert!(!is_local_ref(""));
        assert!(!is_local_ref("http://a/b"));
        assert!(!is_local_ref("mailto:a@b"));
        assert!(!is_local_ref("tel:123"));
        assert!(!is_local_ref("data:image/png;base64,AA"));
    }

    #[test]
    fn rename_paths_updates_everything() {
        let mut a = Archive::default();
        a.set_string("OEBPS/c.opf", "<package><manifest><item id=\"c\" href=\"ch/one.xhtml\"/><item id=\"i\" href=\"pic.png\"/><item id=\"s\" href=\"s.css\"/></manifest></package>");
        a.set_string("OEBPS/ch/one.xhtml", "<html><head><link href=\"../s.css\"/></head><body><img src=\"../pic.png\"/><a href=\"two.xhtml#t\">x</a><a href=\"#loc\">y</a><a href=\"http://e.com/pic.png\">z</a></body></html>");
        a.set_string("OEBPS/ch/two.xhtml", "<html><body id=\"t\"/></html>");
        a.set_string("OEBPS/s.css", "a{background:url(pic.png)}");
        a.set("OEBPS/pic.png", vec![1, 2, 3]);
        let mut m = HashMap::new();
        m.insert("OEBPS/pic.png".to_string(), "OEBPS/img/pic.png".to_string());
        m.insert(
            "OEBPS/ch/one.xhtml".to_string(),
            "OEBPS/Text/one.xhtml".to_string(),
        );
        let changed = rename_paths(&mut a, &m);
        assert!(changed >= 3);
        assert!(a.contains("OEBPS/img/pic.png"));
        assert!(!a.contains("OEBPS/pic.png"));
        assert!(a.contains("OEBPS/Text/one.xhtml"));
        let opf = a.get_string("OEBPS/c.opf").unwrap();
        assert!(opf.contains("href=\"Text/one.xhtml\""));
        assert!(opf.contains("href=\"img/pic.png\""));
        let one = a.get_string("OEBPS/Text/one.xhtml").unwrap();
        assert!(one.contains("href=\"../s.css\""));
        assert!(one.contains("src=\"../img/pic.png\""));
        assert!(one.contains("href=\"../ch/two.xhtml#t\""));
        assert!(one.contains("href=\"#loc\""));
        assert!(one.contains("http://e.com/pic.png"));
        assert_eq!(
            a.get_string("OEBPS/s.css").unwrap(),
            "a{background:url(img/pic.png)}"
        );
        assert_eq!(rename_paths(&mut a, &HashMap::new()), 0);
    }

    #[test]
    fn rename_paths_leaves_broken_refs_alone() {
        let mut a = Archive::default();
        a.set_string(
            "a.xhtml",
            "<a href=\"missing.xhtml\"/><a href=\"b.xhtml?q=1\"/>",
        );
        a.set_string("b.xhtml", "<p/>");
        let mut m = HashMap::new();
        m.insert("b.xhtml".to_string(), "x/b.xhtml".to_string());
        rename_paths(&mut a, &m);
        assert_eq!(
            a.get_string("a.xhtml").unwrap(),
            "<a href=\"missing.xhtml\"/><a href=\"b.xhtml?q=1\"/>"
        );
    }

    #[test]
    fn retarget_links_rewrites_selected_targets() {
        let mut a = Archive::default();
        a.set_string(
            "d/a.xhtml",
            "<a href=\"b.xhtml#x\"/><a href=\"#y\"/><a href=\"http://x\"/>",
        );
        a.set_string("d/b.xhtml", "<p/>");
        let n = retarget_links(&mut a, |from, path, frag| {
            (path == "d/b.xhtml" && frag == Some("x"))
                .then(|| "d/c.xhtml#x".to_string())
                .or_else(|| {
                    (from == "d/a.xhtml" && path == "d/a.xhtml" && frag == Some("y"))
                        .then(|| "d/a.xhtml#z".to_string())
                })
        });
        assert_eq!(n, 1);
        let t = a.get_string("d/a.xhtml").unwrap();
        assert!(t.contains("href=\"c.xhtml#x\""));
        assert!(t.contains("href=\"a.xhtml#z\""));
    }
}
