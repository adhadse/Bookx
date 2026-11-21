//! A small, forgiving (X)HTML tokenizer used by the text-level transforms and writers.
//!
//! It splits markup into tags and text without building a tree, so a transform can rewrite text
//! nodes while every tag, attribute, comment and CDATA section is copied through byte for byte.

use regex::Regex;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tok<'a> {
    Text(&'a str),
    Tag(&'a str),
}

impl<'a> Tok<'a> {
    pub fn as_str(&self) -> &'a str {
        match self {
            Tok::Text(s) | Tok::Tag(s) => s,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagInfo {
    /// Lower-case local name without prefix handling (`svg:rect` stays as is).
    pub name: String,
    pub closing: bool,
    pub self_closing: bool,
}

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

pub const BLOCK: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "body",
    "dd",
    "details",
    "div",
    "dl",
    "dt",
    "figcaption",
    "figure",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "html",
    "li",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "ul",
    "br",
];

pub fn is_void(name: &str) -> bool {
    VOID.contains(&name)
}

pub fn is_block(name: &str) -> bool {
    BLOCK.contains(&name)
}

/// Elements whose text must never be rewritten.
pub fn is_verbatim(name: &str) -> bool {
    matches!(
        name,
        "code" | "pre" | "script" | "style" | "kbd" | "samp" | "tt"
    )
}

fn tag_end(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    let rest = &s[i..];
    if let Some(after) = rest.strip_prefix("<!--") {
        return Some(i + after.find("-->").map(|p| p + 7).unwrap_or(rest.len()));
    }
    if rest.starts_with("<![CDATA[") {
        return Some(i + rest.find("]]>").map(|p| p + 3).unwrap_or(rest.len()));
    }
    if rest.starts_with("<?") {
        return Some(i + rest.find("?>").map(|p| p + 2).unwrap_or(rest.len()));
    }
    let next = *b.get(i + 1)?;
    let starts_name = next.is_ascii_alphabetic()
        || next == b'!'
        || (next == b'/' && b.get(i + 2).is_some_and(|c| c.is_ascii_alphabetic()));
    if !starts_name {
        return None;
    }
    let mut j = i + 1;
    let mut quote: Option<u8> = None;
    while j < b.len() {
        let c = b[j];
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => {
                if c == b'"' || c == b'\'' {
                    quote = Some(c);
                } else if c == b'>' {
                    return Some(j + 1);
                }
            }
        }
        j += 1;
    }
    // unterminated tag: treat the rest as text
    None
}

pub fn tokenize(s: &str) -> Vec<Tok<'_>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut text_start = 0;
    while i < b.len() {
        if b[i] == b'<' {
            if let Some(e) = tag_end(s, i) {
                if text_start < i {
                    out.push(Tok::Text(&s[text_start..i]));
                }
                let tag = &s[i..e];
                out.push(Tok::Tag(tag));
                i = e;
                text_start = e;
                if let Some(info) = tag_info(tag) {
                    if !info.closing
                        && !info.self_closing
                        && matches!(info.name.as_str(), "script" | "style")
                    {
                        let close = format!("</{}", info.name);
                        let lower = s[i..].to_ascii_lowercase();
                        let end = lower.find(&close).map(|p| i + p).unwrap_or(s.len());
                        if end > i {
                            out.push(Tok::Text(&s[i..end]));
                        }
                        i = end;
                        text_start = end;
                    }
                }
                continue;
            }
        }
        i += 1;
    }
    if text_start < s.len() {
        out.push(Tok::Text(&s[text_start..]));
    }
    out
}

pub fn tag_info(tag: &str) -> Option<TagInfo> {
    let inner = tag.strip_prefix('<')?;
    if inner.starts_with('!') || inner.starts_with('?') {
        return None;
    }
    let (closing, inner) = match inner.strip_prefix('/') {
        Some(r) => (true, r),
        None => (false, inner),
    };
    let name: String = inner
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != '>' && *c != '/')
        .collect::<String>()
        .to_ascii_lowercase();
    if name.is_empty() {
        return None;
    }
    let self_closing =
        tag.trim_end_matches('>').trim_end().ends_with('/') || (!closing && is_void(&name));
    Some(TagInfo {
        name,
        closing,
        self_closing,
    })
}

fn attr_re(name: &str) -> Regex {
    static CACHE: OnceLock<Mutex<HashMap<String, Regex>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = cache.lock().unwrap_or_else(|e| e.into_inner());
    map.entry(name.to_string())
        .or_insert_with(|| {
            Regex::new(&format!(
                r#"(?is)(\s){}\s*=\s*("[^"]*"|'[^']*'|[^\s>"']*[^\s>"'/])"#,
                regex::escape(name)
            ))
            .expect("static attribute regex")
        })
        .clone()
}

fn unquote(v: &str) -> &str {
    let v = v.trim();
    if v.len() >= 2
        && (v.starts_with('"') && v.ends_with('"') || v.starts_with('\'') && v.ends_with('\''))
    {
        &v[1..v.len() - 1]
    } else {
        v
    }
}

/// Raw (still entity-escaped) attribute value of a tag.
pub fn get_attr(tag: &str, name: &str) -> Option<String> {
    attr_re(name)
        .captures(tag)
        .map(|c| unquote(&c[2]).to_string())
}

pub fn remove_attr(tag: &str, name: &str) -> String {
    attr_re(name).replace_all(tag, "").into_owned()
}

/// Sets or adds an attribute. `value` must already be escaped.
pub fn set_attr(tag: &str, name: &str, value: &str) -> String {
    let re = attr_re(name);
    if re.is_match(tag) {
        return re
            .replace(tag, |c: &regex::Captures| {
                format!("{}{}=\"{}\"", &c[1], name, value)
            })
            .into_owned();
    }
    let end = if tag.ends_with("/>") {
        tag.len() - 2
    } else {
        tag.len() - 1
    };
    let head = tag[..end].trim_end();
    let tail = &tag[end..];
    let sep = if tail.starts_with('/') { " " } else { "" };
    format!("{head} {name}=\"{value}\"{sep}{tail}")
}

pub fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn entity_table() -> &'static [(&'static str, &'static str)] {
    &[
        ("nbsp", "\u{a0}"),
        ("copy", "©"),
        ("reg", "®"),
        ("mdash", "—"),
        ("ndash", "–"),
        ("hellip", "…"),
        ("lsquo", "‘"),
        ("rsquo", "’"),
        ("ldquo", "“"),
        ("rdquo", "”"),
        ("laquo", "«"),
        ("raquo", "»"),
        ("bull", "•"),
        ("middot", "·"),
        ("times", "×"),
        ("deg", "°"),
        ("eacute", "é"),
        ("egrave", "è"),
        ("agrave", "à"),
        ("aacute", "á"),
        ("uuml", "ü"),
        ("ouml", "ö"),
        ("auml", "ä"),
        ("szlig", "ß"),
        ("ccedil", "ç"),
        ("ntilde", "ñ"),
        ("shy", "\u{ad}"),
        ("trade", "™"),
    ]
}

/// Decodes numeric entities, the five XML entities and a handful of common HTML ones.
/// Unknown entities are kept verbatim.
pub fn decode_entities(s: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"&(#[0-9]+|#[xX][0-9a-fA-F]+|[A-Za-z][A-Za-z0-9]*);").expect("static regex")
    });
    re.replace_all(s, |c: &regex::Captures| {
        let body = &c[1];
        if let Some(num) = body.strip_prefix('#') {
            let code = if let Some(h) = num.strip_prefix(['x', 'X']) {
                u32::from_str_radix(h, 16).ok()
            } else {
                num.parse::<u32>().ok()
            };
            return code
                .and_then(char::from_u32)
                .map(|ch| ch.to_string())
                .unwrap_or_else(|| c[0].to_string());
        }
        match body {
            "amp" => "&".into(),
            "lt" => "<".into(),
            "gt" => ">".into(),
            "quot" => "\"".into(),
            "apos" => "'".into(),
            other => entity_table()
                .iter()
                .find(|(n, _)| *n == other)
                .map(|(_, v)| v.to_string())
                .unwrap_or_else(|| c[0].to_string()),
        }
    })
    .into_owned()
}

/// Escapes text for element content.
pub fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Visible text of a fragment: tags removed, entities decoded, `<script>`/`<style>` dropped.
pub fn plain_text(html: &str) -> String {
    let mut out = String::new();
    let mut skip = 0usize;
    for t in tokenize(html) {
        match t {
            Tok::Tag(tag) => {
                if let Some(info) = tag_info(tag) {
                    if matches!(info.name.as_str(), "script" | "style") && !info.self_closing {
                        if info.closing {
                            skip = skip.saturating_sub(1);
                        } else {
                            skip += 1;
                        }
                    }
                }
            }
            Tok::Text(s) if skip == 0 => out.push_str(&decode_entities(s)),
            Tok::Text(_) => {}
        }
    }
    out
}

/// `(start, end)` byte range of the content between `<body ...>` and `</body>`.
pub fn body_range(html: &str) -> Option<(usize, usize)> {
    let mut start = None;
    let mut pos = 0;
    for t in tokenize(html) {
        let len = t.as_str().len();
        if let Tok::Tag(tag) = t {
            if let Some(info) = tag_info(tag) {
                if info.name == "body" {
                    if info.closing {
                        return start.map(|s| (s, pos));
                    } else if start.is_none() {
                        start = Some(pos + len);
                    }
                }
            }
        }
        pos += len;
    }
    start.map(|s| (s, html.len()))
}

fn is_wellformed(src: &str) -> bool {
    roxmltree::Document::parse_with_options(
        src,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .is_ok()
}

/// Whether `src` is a well-formed XML document (DTD allowed, as XHTML 1.1 files carry one).
pub fn is_wellformed_xml(src: &str) -> bool {
    is_wellformed(src)
}

fn write_children(el: scraper::ElementRef<'_>, out: &mut String) {
    use scraper::Node;
    for c in el.children() {
        match c.value() {
            Node::Text(t) => out.push_str(&escape_text(t)),
            Node::Comment(cm) => {
                let body = cm.replace("--", "- -");
                out.push_str(&format!("<!--{}-->", body.trim_end_matches('-')));
            }
            Node::Element(_) => {
                if let Some(e) = scraper::ElementRef::wrap(c) {
                    write_xhtml(e, out);
                }
            }
            _ => {}
        }
    }
}

fn write_xhtml(el: scraper::ElementRef<'_>, out: &mut String) {
    let e = el.value();
    let name = e.name.local.to_string();
    out.push('<');
    out.push_str(&name);
    for (k, v) in e.attrs.iter() {
        let key = match &k.prefix {
            Some(p) if !p.is_empty() => format!("{p}:{}", k.local),
            _ => k.local.to_string(),
        };
        if key == "xmlns" {
            continue;
        }
        out.push_str(&format!(" {key}=\"{}\"", escape_attr(v)));
    }
    if is_void(&name) && el.children().next().is_none() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    write_children(el, out);
    out.push_str(&format!("</{name}>"));
}

/// Rebuilds sloppy HTML as well-formed XHTML by parsing it the way a browser does (unclosed
/// tags, bare ampersands, undeclared entities, mismatched nesting). Returns `None` when even
/// the rebuilt text is not well-formed XML.
pub fn repair_to_xhtml(src: &str) -> Option<String> {
    let doc = bookx_core::util::parse_html_document(src);
    let mut body = String::new();
    let mut head = String::new();
    for child in doc.root_element().children() {
        let Some(el) = scraper::ElementRef::wrap(child) else {
            continue;
        };
        match &*el.value().name.local {
            "head" => write_children(el, &mut head),
            "body" => {
                let attrs: String = el
                    .value()
                    .attrs
                    .iter()
                    .map(|(k, v)| format!(" {}=\"{}\"", k.local, escape_attr(v)))
                    .collect();
                body.push_str(&format!("<body{attrs}>"));
                write_children(el, &mut body);
                body.push_str("</body>");
            }
            _ => {}
        }
    }
    let epub = if src.contains("epub:") {
        " xmlns:epub=\"http://www.idpf.org/2007/ops\""
    } else {
        ""
    };
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"{epub}><head>{head}</head>{body}</html>\n"
    );
    if !head.contains("<title") {
        let xml = xml.replace("<head>", "<head><title></title>");
        return is_wellformed(&xml).then_some(xml);
    }
    is_wellformed(&xml).then_some(xml)
}

#[cfg(test)]
mod tests {
    #[test]
    fn tag_soup_is_repaired_to_xhtml() {
        let fixed = repair_to_xhtml(
            "<p>one & two&nbsp;<b>bold <i>both</b> end\n<p>next<br>line<img src=a.png alt=x>",
        )
        .unwrap();
        assert!(is_wellformed_xml(&fixed), "{fixed}");
        assert!(fixed.contains("one &amp; two"));
        assert!(fixed.contains("<br/>") && fixed.contains("<img"));
        assert!(!is_wellformed_xml("<p>a & b"));
    }

    use super::*;

    #[test]
    fn tokenizes_tags_and_text() {
        let t = tokenize("<p class=\"a>b\">Hi <b>there</b></p>");
        assert_eq!(
            t,
            vec![
                Tok::Tag("<p class=\"a>b\">"),
                Tok::Text("Hi "),
                Tok::Tag("<b>"),
                Tok::Text("there"),
                Tok::Tag("</b>"),
                Tok::Tag("</p>")
            ]
        );
    }

    #[test]
    fn tokenize_roundtrips_exactly() {
        let src = "<?xml version='1.0'?><!DOCTYPE html><!-- c <b> --><p>a &lt; b < c</p><![CDATA[x<y]]><br/>tail";
        let joined: String = tokenize(src).iter().map(|t| t.as_str()).collect();
        assert_eq!(joined, src);
    }

    #[test]
    fn comments_doctype_and_cdata_are_tags() {
        let t = tokenize("<!-- a --><!DOCTYPE html><![CDATA[<x>]]>");
        assert!(t.iter().all(|t| matches!(t, Tok::Tag(_))));
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn stray_less_than_is_text() {
        let t = tokenize("1 < 2 and 3 <4");
        assert_eq!(t.len(), 1);
        assert!(matches!(t[0], Tok::Text(_)));
    }

    #[test]
    fn unterminated_tag_is_text() {
        let t = tokenize("a <b c=\"x");
        assert!(t.iter().all(|t| matches!(t, Tok::Text(_))));
    }

    #[test]
    fn script_and_style_bodies_are_single_text() {
        let t = tokenize("<style>p > a { }</style><script>if (a<b) {}</script>");
        assert_eq!(
            t,
            vec![
                Tok::Tag("<style>"),
                Tok::Text("p > a { }"),
                Tok::Tag("</style>"),
                Tok::Tag("<script>"),
                Tok::Text("if (a<b) {}"),
                Tok::Tag("</script>")
            ]
        );
    }

    #[test]
    fn tag_info_variants() {
        let i = tag_info("<P class='x'>").unwrap();
        assert_eq!(i.name, "p");
        assert!(!i.closing && !i.self_closing);
        assert!(tag_info("</div>").unwrap().closing);
        assert!(tag_info("<br>").unwrap().self_closing);
        assert!(tag_info("<img src='a'/>").unwrap().self_closing);
        assert!(tag_info("<!-- x -->").is_none());
        assert!(tag_info("<?xml?>").is_none());
    }

    #[test]
    fn attribute_get_set_remove() {
        let t = "<img src=\"a.png\" data-src='b' alt=x/>";
        assert_eq!(get_attr(t, "src").as_deref(), Some("a.png"));
        assert_eq!(get_attr(t, "alt").as_deref(), Some("x"));
        assert_eq!(get_attr(t, "src2"), None);
        // data-src must not be matched by `src`
        assert_eq!(get_attr("<a data-src=\"z\">", "src"), None);
        assert_eq!(
            set_attr(t, "src", "b.png"),
            "<img src=\"b.png\" data-src='b' alt=x/>"
        );
        assert_eq!(set_attr("<p>", "id", "x"), "<p id=\"x\">");
        assert_eq!(set_attr("<br/>", "id", "x"), "<br id=\"x\" />");
        assert_eq!(
            remove_attr("<p id=\"a\" class=\"b\">", "id"),
            "<p class=\"b\">"
        );
    }

    #[test]
    fn entities_decode() {
        assert_eq!(
            decode_entities("a &amp; b &lt; &#233; &#x41; &mdash; &bogus;"),
            "a & b < é A — &bogus;"
        );
        assert_eq!(decode_entities("&#99999999999;"), "&#99999999999;");
    }

    #[test]
    fn escapes() {
        assert_eq!(escape_text("a<b&c>"), "a&lt;b&amp;c&gt;");
        assert_eq!(escape_attr("\"a\"&"), "&quot;a&quot;&amp;");
    }

    #[test]
    fn plain_text_drops_tags_and_scripts() {
        assert_eq!(
            plain_text("<p>Hi <b>x</b>&amp;<script>var a;</script></p>"),
            "Hi x&"
        );
    }

    #[test]
    fn body_range_finds_content() {
        let h = "<html><head><title>t</title></head><body class='x'><p>a</p></body></html>";
        let (s, e) = body_range(h).unwrap();
        assert_eq!(&h[s..e], "<p>a</p>");
        assert!(body_range("<p>no body</p>").is_none());
    }

    #[test]
    fn verbatim_and_block_sets() {
        assert!(is_verbatim("pre") && is_verbatim("code") && !is_verbatim("p"));
        assert!(is_block("p") && !is_block("span"));
        assert!(is_void("br") && !is_void("p"));
    }
}
