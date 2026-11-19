//! Decorations of the code view without GTK: invalid markup (CE-336), link targets and missing targets (CE-339),
//! bold/italic text (CE-333), CSS colour swatches (CE-356) and the text snippets shown next to anchors (CE-346).
//! All functions take `&str` and return byte ranges; the UI turns them into `GtkTextTag`s.

use std::ops::Range;
use std::sync::OnceLock;

use regex::Regex;

use crate::code_nav::{attr_spans, css_rules, resolve_link, LinkTarget};
use crate::markup::{scan, Event};
use crate::tags::pairs;
use crate::transform_css::{parse_color, Rgba};

/// Most marks of one kind a pass returns (a pathological file must not flood the buffer with tags).
pub const MAX_MARKS: usize = 5000;

const VOID: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

// ---------------------------------------------------------------------------------------------
// invalid markup (CE-336)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// `<p>` was opened and never closed.
    Unclosed(String),
    /// `</p>` without an open `<p>`.
    UnexpectedClose(String),
    /// A tag without its `>`.
    UnfinishedTag,
    DuplicateAttribute(String),
    UnquotedAttribute(String),
    /// `&` that does not start an entity.
    BareAmpersand,
    /// CSS: `}` without `{`.
    StrayBrace,
    /// CSS: `{` never closed.
    UnclosedBrace,
    /// A comment that never ends.
    UnclosedComment,
}

impl ErrorKind {
    /// English text for logs and dev hooks; the UI translates by kind.
    pub fn message(&self) -> String {
        match self {
            ErrorKind::Unclosed(n) => format!("<{n}> is never closed"),
            ErrorKind::UnexpectedClose(n) => format!("</{n}> has no matching start tag"),
            ErrorKind::UnfinishedTag => "this tag has no closing >".into(),
            ErrorKind::DuplicateAttribute(n) => format!("attribute {n} appears twice"),
            ErrorKind::UnquotedAttribute(n) => format!("the value of {n} needs quotes"),
            ErrorKind::BareAmpersand => "& must be written &amp;".into(),
            ErrorKind::StrayBrace => "} without a matching {".into(),
            ErrorKind::UnclosedBrace => "{ is never closed".into(),
            ErrorKind::UnclosedComment => "this comment is never closed".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkupError {
    pub range: Range<usize>,
    pub kind: ErrorKind,
}

fn entity_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"&(?:#[0-9]+|#[xX][0-9a-fA-F]+|[A-Za-z][A-Za-z0-9]*);").unwrap())
}

/// Well-formedness problems of an XHTML (`strict`) or HTML document, in document order, at most [`MAX_MARKS`].
/// Strict mode wants every element closed, quoted attributes and escaped ampersands; the lenient mode used for
/// `.html` only reports unfinished tags and duplicate attributes. Comments, CDATA, `<script>` and `<style>`
/// content are left alone.
pub fn markup_errors(src: &str, strict: bool) -> Vec<MarkupError> {
    let mut errs: Vec<MarkupError> = Vec::new();
    let mut stack: Vec<(String, Range<usize>)> = Vec::new();
    let mut raw_text = false;
    let b = src.as_bytes();
    for e in scan(src) {
        if errs.len() >= MAX_MARKS {
            break;
        }
        match e {
            Event::Open {
                name,
                range,
                self_closing,
            } => {
                raw_text = name == "script" || name == "style";
                if b.get(range.end.wrapping_sub(1)) != Some(&b'>') || range.end > src.len() {
                    errs.push(MarkupError {
                        range: range.start..(range.start + 1 + name.len()).min(src.len()),
                        kind: ErrorKind::UnfinishedTag,
                    });
                    continue;
                }
                let mut seen: Vec<String> = Vec::new();
                for a in attr_spans(src, range.clone()) {
                    if seen.contains(&a.name) {
                        errs.push(MarkupError {
                            range: attr_name_range(src, &a.name, &a.value, range.start),
                            kind: ErrorKind::DuplicateAttribute(a.name.clone()),
                        });
                    }
                    seen.push(a.name.clone());
                    let quoted = a.value.start > 0
                        && matches!(b[a.value.start - 1], b'"' | b'\'')
                        && a.value.start > range.start;
                    let bare_flag = a.value.is_empty() && !quoted;
                    if strict && !quoted && !bare_flag {
                        errs.push(MarkupError {
                            range: a.value.clone(),
                            kind: ErrorKind::UnquotedAttribute(a.name.clone()),
                        });
                    }
                }
                if strict && !self_closing && !VOID.contains(&name.as_str()) {
                    stack.push((name, range));
                }
            }
            Event::Close { name, range } => {
                raw_text = false;
                if !strict {
                    continue;
                }
                if let Some(d) = stack.iter().rposition(|(n, _)| *n == name) {
                    for (n, r) in stack.drain(d + 1..) {
                        errs.push(MarkupError {
                            range: r.start..(r.start + 1 + n.len()).min(src.len()),
                            kind: ErrorKind::Unclosed(n),
                        });
                    }
                    stack.pop();
                } else if !VOID.contains(&name.as_str()) {
                    errs.push(MarkupError {
                        range,
                        kind: ErrorKind::UnexpectedClose(name),
                    });
                }
            }
            Event::Text(r) => {
                if strict && !raw_text {
                    let t = &src[r.clone()];
                    for (i, _) in t.match_indices('&') {
                        if entity_re().find(&t[i..]).is_none_or(|m| m.start() != 0) {
                            errs.push(MarkupError {
                                range: r.start + i..r.start + i + 1,
                                kind: ErrorKind::BareAmpersand,
                            });
                        }
                    }
                }
            }
            Event::Other(r) => {
                raw_text = false;
                let t = &src[r.clone()];
                if (t.starts_with("<!--") && !t.ends_with("-->"))
                    || (t.starts_with("<![CDATA[") && !t.ends_with("]]>"))
                {
                    errs.push(MarkupError {
                        range: r.start..(r.start + 4).min(src.len()),
                        kind: ErrorKind::UnclosedComment,
                    });
                }
            }
        }
    }
    for (n, r) in stack {
        errs.push(MarkupError {
            range: r.start..(r.start + 1 + n.len()).min(src.len()),
            kind: ErrorKind::Unclosed(n),
        });
    }
    errs.sort_by_key(|e| e.range.start);
    errs.truncate(MAX_MARKS);
    errs
}

fn attr_name_range(src: &str, name: &str, value: &Range<usize>, tag_start: usize) -> Range<usize> {
    let from = tag_start.max(value.start.saturating_sub(name.len() + 3));
    let s = src[from..value.start.max(from)]
        .to_ascii_lowercase()
        .rfind(name)
        .map(|p| from + p)
        .unwrap_or(from);
    s..(s + name.len()).min(src.len())
}

/// Unbalanced braces and unfinished comments in a stylesheet.
pub fn css_errors(css: &str) -> Vec<MarkupError> {
    let b = css.as_bytes();
    let mut errs = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < b.len() && errs.len() < MAX_MARKS {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'*') => match css[i + 2..].find("*/") {
                Some(p) => {
                    i += p + 4;
                    continue;
                }
                None => {
                    errs.push(MarkupError {
                        range: i..i + 2,
                        kind: ErrorKind::UnclosedComment,
                    });
                    break;
                }
            },
            q @ (b'"' | b'\'') => {
                i += 1;
                while i < b.len() && b[i] != q && b[i] != b'\n' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'{' => open.push(i),
            b'}' => {
                if open.pop().is_none() {
                    errs.push(MarkupError {
                        range: i..i + 1,
                        kind: ErrorKind::StrayBrace,
                    });
                }
            }
            _ => {}
        }
        i += 1;
    }
    for o in open {
        errs.push(MarkupError {
            range: o..o + 1,
            kind: ErrorKind::UnclosedBrace,
        });
    }
    errs.sort_by_key(|e| e.range.start);
    errs
}

// ---------------------------------------------------------------------------------------------
// link targets (CE-339)
// ---------------------------------------------------------------------------------------------

/// A link value of the file and whether it leads somewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkMark {
    /// The value in the source (the part before `#` is the file name).
    pub range: Range<usize>,
    pub ok: bool,
}

fn url_ranges(css: &str) -> Vec<Range<usize>> {
    static R: OnceLock<Regex> = OnceLock::new();
    let re = R.get_or_init(|| {
        Regex::new(
            r#"(?:url\(\s*(?:"([^"]*)"|'([^']*)'|([^)\s"']+))\s*\))|(?:@import\s+(?:"([^"]*)"|'([^']*)'))"#,
        )
        .unwrap()
    });
    re.captures_iter(css)
        .filter_map(|c| (1..=5).find_map(|i| c.get(i)).map(|g| g.range()))
        .filter(|r| !r.is_empty())
        .collect()
}

/// Every internal link value of the file `from` with whether its file (and anchor, when `has_anchor` knows) exist.
/// External links (`http:`, `mailto:`, `data:`) are not reported.
pub fn link_marks(
    src: &str,
    from: &str,
    css: bool,
    file_exists: &dyn Fn(&str) -> bool,
    has_anchor: &dyn Fn(&str, &str) -> Option<bool>,
) -> Vec<LinkMark> {
    let mut values: Vec<Range<usize>> = Vec::new();
    if css {
        values = url_ranges(src);
    } else {
        let mut style = false;
        for e in scan(src) {
            match e {
                Event::Open { name, range, .. } => {
                    style = name == "style";
                    for a in attr_spans(src, range) {
                        if matches!(
                            a.name.as_str(),
                            "href" | "src" | "xlink:href" | "poster" | "data"
                        ) && !a.value.is_empty()
                        {
                            values.push(a.value);
                        }
                    }
                }
                Event::Text(r) if style => {
                    values.extend(
                        url_ranges(&src[r.clone()])
                            .into_iter()
                            .map(|v| v.start + r.start..v.end + r.start),
                    );
                }
                Event::Text(_) => {}
                _ => style = false,
            }
        }
    }
    values.truncate(MAX_MARKS);
    values
        .into_iter()
        .filter_map(|range| {
            let LinkTarget { file, fragment } = resolve_link(from, &src[range.clone()])?;
            let mut ok = file_exists(&file);
            if ok {
                if let Some(f) = &fragment {
                    ok = has_anchor(&file, f).unwrap_or(true);
                }
            }
            Some(LinkMark { range, ok })
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// bold and italic text (CE-333)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Emphasis {
    pub bold: bool,
    pub italic: bool,
}

/// The content of every `b`/`strong`/`i`/`em`/`h1`…`h6`/`th` element with how to draw it.
pub fn emphasis_spans(src: &str) -> Vec<(Range<usize>, Emphasis)> {
    let mut out = Vec::new();
    for p in pairs(src) {
        let Some(close) = p.close.clone() else {
            continue;
        };
        let e = match p.name.as_str() {
            "b" | "strong" | "th" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => Emphasis {
                bold: true,
                italic: false,
            },
            "i" | "em" | "cite" | "dfn" => Emphasis {
                bold: false,
                italic: true,
            },
            _ => continue,
        };
        if p.open.end < close.start {
            out.push((p.open.end..close.start, e));
        }
        if out.len() >= MAX_MARKS {
            break;
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// colours (CE-356)
// ---------------------------------------------------------------------------------------------

/// A colour literal in CSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorSpan {
    pub start: usize,
    pub end: usize,
    pub rgba: Rgba,
    /// Black or white, whichever reads better on the colour.
    pub text: (u8, u8, u8),
}

const MORE_NAMED: &[(&str, (u8, u8, u8))] = &[
    ("aliceblue", (240, 248, 255)),
    ("beige", (245, 245, 220)),
    ("cyan", (0, 255, 255)),
    ("magenta", (255, 0, 255)),
    ("crimson", (220, 20, 60)),
    ("coral", (255, 127, 80)),
    ("gold", (255, 215, 0)),
    ("indigo", (75, 0, 130)),
    ("ivory", (255, 255, 240)),
    ("khaki", (240, 230, 140)),
    ("lavender", (230, 230, 250)),
    ("lightblue", (173, 216, 230)),
    ("lightgray", (211, 211, 211)),
    ("lightgrey", (211, 211, 211)),
    ("lightgreen", (144, 238, 144)),
    ("darkgray", (169, 169, 169)),
    ("darkgrey", (169, 169, 169)),
    ("darkblue", (0, 0, 139)),
    ("darkgreen", (0, 100, 0)),
    ("darkred", (139, 0, 0)),
    ("salmon", (250, 128, 114)),
    ("tan", (210, 180, 140)),
    ("tomato", (255, 99, 71)),
    ("turquoise", (64, 224, 208)),
    ("violet", (238, 130, 238)),
    ("wheat", (245, 222, 179)),
    ("whitesmoke", (245, 245, 245)),
    ("slategray", (112, 128, 144)),
    ("skyblue", (135, 206, 235)),
    ("steelblue", (70, 130, 180)),
    ("sienna", (160, 82, 45)),
    ("orchid", (218, 112, 214)),
    ("plum", (221, 160, 221)),
    ("peru", (205, 133, 63)),
    ("linen", (250, 240, 230)),
    ("firebrick", (178, 34, 34)),
    ("forestgreen", (34, 139, 34)),
    ("dodgerblue", (30, 144, 255)),
    ("chocolate", (210, 105, 30)),
    ("cornsilk", (255, 248, 220)),
];

const COLOR_PROPS: &[&str] = &[
    "color",
    "background",
    "background-color",
    "border",
    "border-color",
    "border-top",
    "border-bottom",
    "border-left",
    "border-right",
    "border-top-color",
    "border-bottom-color",
    "border-left-color",
    "border-right-color",
    "outline",
    "outline-color",
    "text-decoration",
    "text-decoration-color",
    "box-shadow",
    "text-shadow",
    "fill",
    "stroke",
    "column-rule",
    "caret-color",
    "accent-color",
];

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let h = h.rem_euclid(360.0) / 360.0;
    let (s, l) = (s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let f = |mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    (f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0))
}

/// `#rgb`, `#rrggbb[aa]`, `rgb()`, `rgba()`, `hsl()`, `hsla()` and colour names. `None` for anything else.
pub fn parse_css_color(s: &str) -> Option<Rgba> {
    let t = s.trim().to_ascii_lowercase();
    if let Some(args) = t
        .strip_prefix("hsla(")
        .or_else(|| t.strip_prefix("hsl("))
        .and_then(|r| r.strip_suffix(')'))
    {
        let parts: Vec<&str> = args
            .split([',', ' ', '/'])
            .filter(|p| !p.is_empty())
            .collect();
        if parts.len() < 3 || parts.len() > 4 {
            return None;
        }
        let num = |p: &str| -> Option<f64> {
            p.trim_end_matches("deg")
                .trim_end_matches('%')
                .parse::<f64>()
                .ok()
        };
        let (r, g, b) = hsl_to_rgb(
            num(parts[0])?,
            num(parts[1])? / 100.0,
            num(parts[2])? / 100.0,
        );
        let a = match parts.get(3) {
            Some(p) => {
                let v = if let Some(pc) = p.strip_suffix('%') {
                    pc.parse::<f64>().ok()? / 100.0
                } else {
                    p.parse::<f64>().ok()?
                };
                (v.clamp(0.0, 1.0) * 255.0).round() as u8
            }
            None => 255,
        };
        return Some(Rgba(r, g, b, a));
    }
    parse_color(&t).or_else(|| {
        MORE_NAMED
            .iter()
            .find(|(n, _)| *n == t)
            .map(|(_, (r, g, b))| Rgba(*r, *g, *b, 255))
    })
}

fn lin(c: u8) -> f64 {
    let c = c as f64 / 255.0;
    if c <= 0.03928 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG relative luminance of the colour composited over white.
fn luminance(c: Rgba) -> f64 {
    let a = c.3 as f64 / 255.0;
    let mix = |v: u8| (v as f64 * a + 255.0 * (1.0 - a)).round() as u8;
    0.2126 * lin(mix(c.0)) + 0.7152 * lin(mix(c.1)) + 0.0722 * lin(mix(c.2))
}

/// Black or white, whichever has the higher contrast ratio on `c`.
pub fn readable_text(c: Rgba) -> (u8, u8, u8) {
    let l = luminance(c);
    let on_black = (l + 0.05) / 0.05;
    let on_white = 1.05 / (l + 0.05);
    if on_black >= on_white {
        (0, 0, 0)
    } else {
        (255, 255, 255)
    }
}

fn color_token_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?i)#[0-9a-f]{3,8}\b|\b(?:rgba?|hsla?)\([^()]*\)|\b[a-z]{3,20}\b").unwrap()
    })
}

/// `s` with comments, strings and `url(...)` blanked out (same length, only ASCII spaces) so token searches
/// cannot hit them.
fn mask(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for k in from..to.min(out.len()) {
            if out[k] != b'\n' {
                out[k] = b' ';
            }
        }
    };
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let e = s[i + 2..].find("*/").map_or(b.len(), |p| i + 2 + p + 2);
                blank(&mut out, i, e);
                i = e;
                continue;
            }
            q @ (b'"' | b'\'') => {
                let mut e = i + 1;
                while e < b.len() && b[e] != q {
                    if b[e] == b'\\' {
                        e += 1;
                    }
                    e += 1;
                }
                blank(&mut out, i, (e + 1).min(b.len()));
                i = e + 1;
                continue;
            }
            b'u' | b'U' if s[i..].len() >= 4 && s[i..i + 4].eq_ignore_ascii_case("url(") => {
                let e = s[i..].find(')').map_or(b.len(), |p| i + p + 1);
                blank(&mut out, i, e);
                i = e;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    // multi-byte characters were never blanked partially: strings are blanked whole, others kept
    String::from_utf8(out).unwrap_or_else(|_| " ".repeat(s.len()))
}

fn declarations_in(src: &str, region: Range<usize>, out: &mut Vec<ColorSpan>) {
    let text = &src[region.clone()];
    let masked = mask(text);
    let mut at = 0;
    for decl in masked.split(';') {
        let d_start = at;
        at += decl.len() + 1;
        let Some(colon) = decl.find(':') else {
            continue;
        };
        let prop = decl[..colon].trim().to_ascii_lowercase();
        let colour_prop = COLOR_PROPS.contains(&prop.as_str());
        let val_start = d_start + colon + 1;
        let value = &masked[val_start..(d_start + decl.len()).min(masked.len())];
        for m in color_token_re().find_iter(value) {
            let tok = m.as_str();
            let first = tok.as_bytes()[0];
            let is_name = first.is_ascii_alphabetic() && !tok.contains('(');
            if is_name && !colour_prop {
                continue;
            }
            if is_name {
                // not part of a longer word such as `red-ish`
                let before = value[..m.start()].chars().next_back();
                let after = value[m.end()..].chars().next();
                if before.is_some_and(|c| c == '-' || c == '_' || c == '.')
                    || after.is_some_and(|c| c == '-' || c == '_' || c == '(')
                {
                    continue;
                }
            }
            let Some(rgba) = parse_css_color(tok) else {
                continue;
            };
            let s = region.start + val_start + m.start();
            out.push(ColorSpan {
                start: s,
                end: s + tok.len(),
                rgba,
                text: readable_text(rgba),
            });
        }
    }
}

/// The colour literals of a stylesheet (`css`) or of the `style` attributes and `<style>` blocks of a markup
/// file, at most [`MAX_MARKS`]. Colour names count only in colour properties.
pub fn css_colors(src: &str, css: bool) -> Vec<ColorSpan> {
    let mut out = Vec::new();
    if css {
        for r in css_rules(src) {
            // `at` rules with a body of declarations (`@font-face`, `@page`) and style rules alike
            if r.at.as_deref().is_some_and(|a| {
                matches!(a, "media" | "supports" | "layer" | "container" | "document")
            }) {
                continue;
            }
            declarations_in(src, r.body, &mut out);
            if out.len() >= MAX_MARKS {
                break;
            }
        }
    } else {
        let mut style = false;
        for e in scan(src) {
            match e {
                Event::Open { name, range, .. } => {
                    style = name == "style";
                    for a in attr_spans(src, range) {
                        if a.name == "style" && !a.value.is_empty() {
                            declarations_in(src, a.value, &mut out);
                        }
                    }
                }
                Event::Text(r) if style => {
                    let part = &src[r.clone()];
                    for rule in css_rules(part) {
                        if rule.at.as_deref().is_some_and(|a| a == "media") {
                            continue;
                        }
                        declarations_in(
                            src,
                            rule.body.start + r.start..rule.body.end + r.start,
                            &mut out,
                        );
                    }
                }
                Event::Text(_) => {}
                _ => style = false,
            }
            if out.len() >= MAX_MARKS {
                break;
            }
        }
    }
    out.truncate(MAX_MARKS);
    out
}

// ---------------------------------------------------------------------------------------------
// anchor snippets (CE-346)
// ---------------------------------------------------------------------------------------------

/// Characters of text shown next to an anchor in the completion list.
pub const SNIPPET_CHARS: usize = 48;

/// `(id, snippet)` for every anchor of the file: the first words of the element's text, or of the text that
/// follows an empty anchor.
pub fn anchor_snippets(src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let all = pairs(src);
    for p in &all {
        let spans = attr_spans(src, p.open.clone());
        let Some(id) = spans
            .iter()
            .find(|a| a.name == "id" || (a.name == "name" && p.name == "a"))
            .map(|a| src[a.value.clone()].to_string())
        else {
            continue;
        };
        let content_end = p.close.as_ref().map_or(p.open.end, |c| c.start);
        let mut end = if content_end > p.open.end {
            content_end
        } else {
            // empty anchor: the text after it
            src.len()
        };
        end = end.min(p.open.end + 600);
        while !src.is_char_boundary(end) {
            end -= 1;
        }
        let text = crate::markup::plain_text(&src[p.open.end..end]);
        let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let snippet: String = if flat.chars().count() > SNIPPET_CHARS {
            let cut: String = flat.chars().take(SNIPPET_CHARS).collect();
            format!("{cut}…")
        } else {
            flat
        };
        out.push((id, snippet));
        if out.len() >= 4000 {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str, strict: bool) -> Vec<ErrorKind> {
        markup_errors(src, strict)
            .into_iter()
            .map(|e| e.kind)
            .collect()
    }

    #[test]
    fn well_formed_documents_have_no_errors() {
        let ok = "<?xml version=\"1.0\"?><html xmlns=\"x\"><head><title>t &amp; u</title></head><body><p class=\"a\">x<br/><img src=\"a.png\" alt=\"\"/></p><!-- c --><![CDATA[ & < ]]></body></html>";
        assert_eq!(kinds(ok, true), vec![]);
        assert_eq!(
            kinds(
                "<script>if (a < b && c) {}</script><style>a>b{}</style>",
                true
            ),
            vec![]
        );
        assert_eq!(kinds("<p>&#169; &#xA9; &nbsp;</p>", true), vec![]);
    }

    #[test]
    fn mismatched_unclosed_and_stray_tags() {
        let e = markup_errors("<div><p>x</div></p>", true);
        assert_eq!(e[0].kind, ErrorKind::Unclosed("p".into()));
        assert_eq!(&"<div><p>x</div></p>"[e[0].range.clone()], "<p");
        assert_eq!(e[1].kind, ErrorKind::UnexpectedClose("p".into()));
        assert_eq!(
            kinds("<ul><li>a<li>b</ul>", true),
            vec![
                ErrorKind::Unclosed("li".into()),
                ErrorKind::Unclosed("li".into())
            ]
        );
        assert_eq!(
            kinds("<p>never", true),
            vec![ErrorKind::Unclosed("p".into())]
        );
        assert_eq!(
            kinds("</p>", true),
            vec![ErrorKind::UnexpectedClose("p".into())]
        );
        assert_eq!(
            kinds("<a:b>x</a:b>", true),
            vec![],
            "prefixes are ignored when pairing"
        );
    }

    #[test]
    fn nested_comments_cdata_and_malformed_input() {
        assert_eq!(
            kinds("<p><!-- <b> --></p>", true),
            vec![],
            "tags inside comments do not count"
        );
        assert_eq!(kinds("<p><![CDATA[</q>]]></p>", true), vec![]);
        assert_eq!(kinds("<p><!-- open", true).len(), 2);
        assert!(kinds("<p><!-- open", true).contains(&ErrorKind::UnclosedComment));
        assert_eq!(kinds("<p class=\"a\"", true)[0], ErrorKind::UnfinishedTag);
        for s in [
            "<",
            "<<>>",
            "</",
            "<a b='",
            "&",
            "<p>&",
            "<!--",
            "<![CDATA[",
            "<a =>",
            "\u{0}<\u{1f600}>",
        ] {
            let _ = markup_errors(s, true);
            let _ = markup_errors(s, false);
        }
    }

    #[test]
    fn attributes_and_ampersands() {
        assert_eq!(
            kinds("<p a=\"1\" a=\"2\">x</p>", true),
            vec![ErrorKind::DuplicateAttribute("a".into())]
        );
        assert_eq!(
            kinds("<p a=1>x</p>", true),
            vec![ErrorKind::UnquotedAttribute("a".into())]
        );
        assert_eq!(
            kinds("<p a=1>x</p>", false),
            vec![],
            "lenient mode accepts bare values"
        );
        let e = markup_errors("<p>a & b &amp; c &x</p>", true);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].range, 5..6);
        assert_eq!(
            kinds("<input disabled/>", true),
            vec![],
            "valueless attributes are fine in this check"
        );
        assert_eq!(
            kinds("<p>é & ü</p>", true),
            vec![ErrorKind::BareAmpersand],
            "unicode text keeps offsets valid"
        );
    }

    #[test]
    fn lenient_mode_ignores_unclosed_elements() {
        assert_eq!(kinds("<ul><li>a<li>b</ul><p>x", false), vec![]);
        assert_eq!(kinds("<p", false), vec![ErrorKind::UnfinishedTag]);
    }

    #[test]
    fn css_brace_errors() {
        assert!(css_errors("a { b: c } d { e: f }").is_empty());
        assert!(css_errors("a { content: \"}\" } /* } */").is_empty());
        let e = css_errors("a { b: c } }");
        assert_eq!(e[0].kind, ErrorKind::StrayBrace);
        assert_eq!(e[0].range, 11..12);
        assert_eq!(css_errors("a { b: c")[0].kind, ErrorKind::UnclosedBrace);
        assert_eq!(css_errors("/* never")[0].kind, ErrorKind::UnclosedComment);
        assert!(css_errors("@media x { a { b: c } }").is_empty());
    }

    #[test]
    fn link_marks_found_and_missing() {
        let html = "<a href=\"c2.xhtml#t\">a</a><a href=\"gone.xhtml\">b</a><a href=\"https://x.org\">c</a><img src=\"../im/a.png\"/><a href=\"#zz\">d</a><a href=\"#ok\">e</a>";
        let exists = |p: &str| matches!(p, "T/c2.xhtml" | "im/a.png" | "T/c1.xhtml");
        let anchor = |f: &str, a: &str| Some(f == "T/c2.xhtml" && a == "t" || a == "ok");
        let m = link_marks(html, "T/c1.xhtml", false, &exists, &anchor);
        let seen: Vec<(&str, bool)> = m.iter().map(|m| (&html[m.range.clone()], m.ok)).collect();
        assert_eq!(
            seen,
            vec![
                ("c2.xhtml#t", true),
                ("gone.xhtml", false),
                ("../im/a.png", true),
                ("#zz", false),
                ("#ok", true)
            ],
            "external links are skipped"
        );
        let css = "a { background: url(\"../im/a.png\") } @import 'nope.css'; p { background: url(data:image/png;base64,AA) }";
        let m = link_marks(css, "T/s.css", true, &|p| p == "im/a.png", &|_, _| None);
        assert_eq!(m.len(), 2);
        assert!(m[0].ok && !m[1].ok);
        let m = link_marks(
            "<style>a{background:url(x.png)}</style>",
            "a.xhtml",
            false,
            &|_| false,
            &|_, _| None,
        );
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn emphasis_ranges() {
        let s = "<h1>T</h1><p><b>a <i>b</i></b> <em>c</em> <strong/><span>d</span></p>";
        let e = emphasis_spans(s);
        let got: Vec<(&str, bool, bool)> = e
            .iter()
            .map(|(r, e)| (&s[r.clone()], e.bold, e.italic))
            .collect();
        assert_eq!(
            got,
            vec![
                ("T", true, false),
                ("a <i>b</i>", true, false),
                ("b", false, true),
                ("c", false, true)
            ]
        );
        assert!(emphasis_spans("<b>unclosed").is_empty());
    }

    #[test]
    fn colour_parsing() {
        assert_eq!(parse_css_color("#f00"), Some(Rgba(255, 0, 0, 255)));
        assert_eq!(parse_css_color("#00ff0080"), Some(Rgba(0, 255, 0, 128)));
        assert_eq!(parse_css_color("rgb(1, 2, 3)"), Some(Rgba(1, 2, 3, 255)));
        assert_eq!(
            parse_css_color("hsl(0, 100%, 50%)"),
            Some(Rgba(255, 0, 0, 255))
        );
        assert_eq!(
            parse_css_color("hsl(120deg 100% 25%)"),
            Some(Rgba(0, 128, 0, 255))
        );
        assert_eq!(
            parse_css_color("hsla(240, 100%, 50%, 0.5)"),
            Some(Rgba(0, 0, 255, 128))
        );
        assert_eq!(parse_css_color("Crimson"), Some(Rgba(220, 20, 60, 255)));
        assert_eq!(parse_css_color("rebeccapurplish"), None);
        assert_eq!(parse_css_color("#ggg"), None);
        assert_eq!(parse_css_color("hsl(1,2)"), None);
    }

    #[test]
    fn contrast_safe_text() {
        assert_eq!(readable_text(Rgba(255, 255, 0, 255)), (0, 0, 0));
        assert_eq!(readable_text(Rgba(0, 0, 128, 255)), (255, 255, 255));
        assert_eq!(readable_text(Rgba(0, 0, 0, 255)), (255, 255, 255));
        assert_eq!(readable_text(Rgba(255, 255, 255, 255)), (0, 0, 0));
        assert_eq!(
            readable_text(Rgba(0, 0, 0, 0)),
            (0, 0, 0),
            "a transparent colour is white paper"
        );
        for r in [0u8, 64, 128, 192, 255] {
            for g in [0u8, 100, 200] {
                for b in [0u8, 255] {
                    let c = Rgba(r, g, b, 255);
                    let t = readable_text(c);
                    let (lt, lc) = (luminance(Rgba(t.0, t.1, t.2, 255)), luminance(c));
                    let ratio = (lt.max(lc) + 0.05) / (lt.min(lc) + 0.05);
                    assert!(ratio >= 4.5, "{c:?} -> {t:?} ratio {ratio}");
                }
            }
        }
    }

    #[test]
    fn colour_spans_in_css_style_attributes_and_blocks() {
        let css = "/* #123456 */ a.red:hover { color: red; background: #fff url(#abc.png); border: 1px solid rgb(0,0,255); font-family: orange; content: \"#f00\" }\n@media print { p { color: hsl(0,0%,0%) } }\n#fff:hover { x: y }";
        let c = css_colors(css, true);
        let got: Vec<&str> = c.iter().map(|c| &css[c.start..c.end]).collect();
        assert_eq!(
            got,
            vec!["red", "#fff", "rgb(0,0,255)", "hsl(0,0%,0%)"],
            "comments, strings, url(), selectors and font-family are skipped"
        );
        let html = "<style>p { color: navy }</style><p style=\"color:#0f0;margin:0\">x</p><p style=\"font-family: red\">y</p>";
        let c = css_colors(html, false);
        let got: Vec<&str> = c.iter().map(|c| &html[c.start..c.end]).collect();
        assert_eq!(got, vec!["navy", "#0f0"]);
        assert_eq!(c[0].text, (255, 255, 255));
        assert!(css_colors("p { color: #zzz; background: #12 }", true).is_empty());
        let _ = css_colors("p { color: \"é", true);
        let _ = css_colors("é { color: red; ü: ñ }", true);
    }

    #[test]
    fn anchor_snippets_show_text() {
        let s = "<h1 id=\"top\">The <i>Great</i> Beginning of Everything and Then Some More Words to Overflow</h1><a id=\"x\"/> after the anchor <p id=\"p2\"></p><a name=\"old\">legacy</a>";
        let a = anchor_snippets(s);
        assert_eq!(a[0].0, "top");
        assert!(
            a[0].1.starts_with("The Great Beginning of Everything") && a[0].1.ends_with('…'),
            "{:?}",
            a[0]
        );
        assert_eq!(
            a[1],
            ("x".to_string(), "after the anchor legacy".to_string())
        );
        assert!(a[2].0 == "p2");
        assert_eq!(a[3], ("old".to_string(), "legacy".to_string()));
        let _ = anchor_snippets(
            "<p id=\"é\">éééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééé",
        );
    }
}
