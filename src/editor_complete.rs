//! Code completion of the book editor without GTK: what is being typed (`context`) and what to
//! offer (`candidates`). The completion provider in `ui/editor/completion.rs` only moves the text
//! in and out of the buffer. Covers tag names, attribute names, CSS properties, internal file
//! names in `href`/`src`/`url()`, anchors/ids after `#` (CE-345, CE-346, CE-354) and, as a
//! fallback, words of the open file.

use crate::core::util::{parent_dir, resolve_href};
use bookx_core::util::relative_href;

/// What the cursor is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Context {
    /// `<di|`: the name of a start tag.
    TagName { prefix: String },
    /// `<a hr|`: an attribute name of `tag`.
    AttrName { tag: String, prefix: String },
    /// `<a href="ch|`: the value of a link attribute (a file, `file#id` or `#id`).
    LinkValue { prefix: String },
    /// `<p class="no|`: the value of `class`.
    ClassValue { prefix: String },
    /// `col|` inside a CSS declaration block.
    CssProperty { prefix: String },
    /// `url(im|` in CSS.
    CssUrl { prefix: String },
    /// Plain text: complete from words of the file.
    Word { prefix: String },
    /// Nothing sensible to offer (inside a comment, in a selector, an unknown attribute value…).
    None,
}

/// One proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// What the list shows.
    pub label: String,
    /// What replaces the typed prefix.
    pub insert: String,
    /// Characters the cursor moves back after inserting (to sit inside `=""`).
    pub back: usize,
    pub kind: Kind,
    /// A short text shown next to the label (the start of the text an anchor points at).
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Tag,
    Attribute,
    Property,
    File,
    Anchor,
    Class,
    Word,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Tag => "tag",
            Kind::Attribute => "attribute",
            Kind::Property => "property",
            Kind::File => "file",
            Kind::Anchor => "anchor",
            Kind::Class => "class",
            Kind::Word => "word",
        }
    }
}

/// The book facts the proposals come from.
pub struct Data<'a> {
    /// The open file (archive path).
    pub current: &'a str,
    /// Every file of the book (archive paths).
    pub files: &'a [String],
    /// Ids defined in a file.
    pub ids_of: &'a dyn Fn(&str) -> Vec<String>,
    /// Class names defined in the stylesheets.
    pub classes: &'a [String],
    /// Words of the open file.
    pub words: &'a [String],
    /// The first words of the text at an anchor of a file (CE-346): `(file, id)`.
    pub snippet_of: &'a dyn Fn(&str, &str) -> Option<String>,
}

const MAX_PROPOSALS: usize = 60;

pub const TAGS: &[&str] = &[
    "a",
    "abbr",
    "address",
    "article",
    "aside",
    "audio",
    "b",
    "bdi",
    "bdo",
    "blockquote",
    "body",
    "br",
    "caption",
    "cite",
    "code",
    "col",
    "colgroup",
    "dd",
    "del",
    "details",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hr",
    "html",
    "i",
    "img",
    "ins",
    "kbd",
    "li",
    "link",
    "main",
    "mark",
    "math",
    "meta",
    "nav",
    "object",
    "ol",
    "p",
    "pre",
    "q",
    "rb",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "script",
    "section",
    "small",
    "source",
    "span",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "svg",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "u",
    "ul",
    "var",
    "video",
    "wbr",
];

const GLOBAL_ATTRS: &[&str] = &[
    "id",
    "class",
    "style",
    "title",
    "lang",
    "xml:lang",
    "dir",
    "epub:type",
    "role",
    "hidden",
    "aria-label",
    "aria-labelledby",
    "aria-describedby",
];

fn tag_attrs(tag: &str) -> &'static [&'static str] {
    match tag {
        "a" => &["href", "rel", "type", "hreflang", "target"],
        "img" => &["src", "alt", "width", "height"],
        "link" => &["href", "rel", "type", "media"],
        "meta" => &["name", "content", "charset", "http-equiv"],
        "audio" | "video" => &["src", "controls", "poster", "width", "height"],
        "source" => &["src", "type", "media"],
        "script" => &["src", "type"],
        "object" => &["data", "type", "width", "height"],
        "td" | "th" => &["colspan", "rowspan", "headers", "scope"],
        "col" | "colgroup" => &["span"],
        "ol" => &["start", "reversed", "type"],
        "li" => &["value"],
        "svg" => &["viewBox", "width", "height", "xmlns"],
        "html" => &["xmlns", "xmlns:epub"],
        "q" | "blockquote" | "del" | "ins" => &["cite"],
        "bdo" => &["dir"],
        _ => &[],
    }
}

pub const CSS_PROPERTIES: &[&str] = &[
    "background",
    "background-color",
    "background-image",
    "background-position",
    "background-repeat",
    "background-size",
    "border",
    "border-bottom",
    "border-bottom-color",
    "border-bottom-style",
    "border-bottom-width",
    "border-collapse",
    "border-color",
    "border-left",
    "border-radius",
    "border-right",
    "border-spacing",
    "border-style",
    "border-top",
    "border-width",
    "box-sizing",
    "break-after",
    "break-before",
    "break-inside",
    "caption-side",
    "clear",
    "color",
    "content",
    "direction",
    "display",
    "float",
    "font",
    "font-family",
    "font-size",
    "font-style",
    "font-variant",
    "font-weight",
    "height",
    "hyphens",
    "letter-spacing",
    "line-height",
    "list-style",
    "list-style-position",
    "list-style-type",
    "margin",
    "margin-bottom",
    "margin-left",
    "margin-right",
    "margin-top",
    "max-height",
    "max-width",
    "min-height",
    "min-width",
    "object-fit",
    "orphans",
    "outline",
    "overflow",
    "padding",
    "padding-bottom",
    "padding-left",
    "padding-right",
    "padding-top",
    "page-break-after",
    "page-break-before",
    "page-break-inside",
    "position",
    "quotes",
    "src",
    "table-layout",
    "text-align",
    "text-align-last",
    "text-decoration",
    "text-indent",
    "text-transform",
    "vertical-align",
    "visibility",
    "white-space",
    "widows",
    "width",
    "word-break",
    "word-spacing",
    "writing-mode",
];

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '.')
}

/// The identifier characters directly before the end of `s`.
fn trailing_name(s: &str) -> &str {
    let start = s
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_name_char(*c))
        .last()
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    &s[start..]
}

fn trailing_word(s: &str) -> &str {
    let start = s
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || matches!(c, '\'' | '’' | '_'))
        .last()
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    &s[start..]
}

/// What is being typed at the end of `before` (the text from the start of the file to the cursor).
pub fn context(before: &str, css: bool) -> Context {
    // Only the tail matters; cut on a char boundary.
    let mut from = before.len().saturating_sub(6000);
    while !before.is_char_boundary(from) {
        from += 1;
    }
    let before = &before[from..];
    if css {
        css_context(before)
    } else {
        markup_context(before)
    }
}

fn markup_context(before: &str) -> Context {
    if let (Some(c), e) = (before.rfind("<!--"), before.rfind("-->")) {
        if e.is_none_or(|e| e < c) {
            return Context::None;
        }
    }
    let lt = before.rfind('<');
    let gt = before.rfind('>');
    match (lt, gt) {
        (Some(l), g) if g.is_none_or(|g| g < l) => {
            let tag = &before[l + 1..];
            if tag.starts_with('/') || tag.starts_with('!') || tag.starts_with('?') {
                return Context::None;
            }
            if !tag.contains(char::is_whitespace) {
                return Context::TagName {
                    prefix: tag.to_string(),
                };
            }
            let name = tag
                .split(char::is_whitespace)
                .next()
                .unwrap_or("")
                .to_string();
            // inside a quoted value?
            let mut quote: Option<(char, usize)> = None;
            for (i, c) in tag.char_indices() {
                match quote {
                    Some((q, _)) if c == q => quote = None,
                    None if c == '"' || c == '\'' => quote = Some((c, i)),
                    _ => {}
                }
            }
            match quote {
                Some((_, qi)) => {
                    let value = &tag[qi + 1..];
                    let attr = tag[..qi].trim_end().strip_suffix('=').map(|h| {
                        let h = h.trim_end();
                        trailing_name(h).to_ascii_lowercase()
                    });
                    match attr.as_deref() {
                        Some("href" | "src" | "xlink:href" | "poster" | "data" | "cite") => {
                            Context::LinkValue {
                                prefix: value.to_string(),
                            }
                        }
                        Some("class") => Context::ClassValue {
                            prefix: trailing_name(value).to_string(),
                        },
                        _ => Context::None,
                    }
                }
                None => {
                    if tag.ends_with(char::is_whitespace) || trailing_name(tag).is_empty() {
                        Context::AttrName {
                            tag: name,
                            prefix: String::new(),
                        }
                    } else if tag.ends_with('=') {
                        Context::None
                    } else {
                        Context::AttrName {
                            tag: name,
                            prefix: trailing_name(tag).to_string(),
                        }
                    }
                }
            }
        }
        _ => Context::Word {
            prefix: trailing_word(before).to_string(),
        },
    }
}

fn css_context(before: &str) -> Context {
    if let (Some(c), e) = (before.rfind("/*"), before.rfind("*/")) {
        if e.is_none_or(|e| e < c) {
            return Context::None;
        }
    }
    let open = before.rfind('{');
    let close = before.rfind('}');
    let Some(o) = open.filter(|o| close.is_none_or(|c| c < *o)) else {
        return Context::None;
    };
    let block = &before[o + 1..];
    let decl_start = block.rfind(';').map(|i| i + 1).unwrap_or(0);
    let decl = &block[decl_start..];
    if let Some(u) = decl.rfind("url(") {
        if !decl[u..].contains(')') {
            let v = decl[u + 4..].trim_start_matches(['"', '\'']);
            return Context::CssUrl {
                prefix: v.to_string(),
            };
        }
    }
    if decl.contains(':') {
        return Context::None;
    }
    Context::CssProperty {
        prefix: trailing_name(decl.trim_start()).to_string(),
    }
}

fn starts_ci(label: &str, prefix: &str) -> bool {
    label.to_lowercase().starts_with(&prefix.to_lowercase())
}

fn plain(label: &str, kind: Kind) -> Candidate {
    Candidate {
        label: label.to_string(),
        insert: label.to_string(),
        back: 0,
        kind,
        detail: None,
    }
}

/// How many characters of `context` the proposals replace: the typed prefix.
pub fn prefix_len(ctx: &Context) -> usize {
    match ctx {
        Context::TagName { prefix }
        | Context::AttrName { prefix, .. }
        | Context::LinkValue { prefix }
        | Context::ClassValue { prefix }
        | Context::CssProperty { prefix }
        | Context::CssUrl { prefix }
        | Context::Word { prefix } => prefix_replaced(ctx, prefix).chars().count(),
        Context::None => 0,
    }
}

/// For link values only the part after the last `#` is replaced while completing anchors, else the
/// whole value; this returns the replaced part of `prefix`.
fn prefix_replaced<'a>(ctx: &Context, prefix: &'a str) -> &'a str {
    match ctx {
        Context::LinkValue { .. } | Context::CssUrl { .. } => match prefix.rfind('#') {
            Some(i) => &prefix[i + 1..],
            None => prefix,
        },
        _ => prefix,
    }
}

/// The proposals for `ctx`, best first, at most 60.
pub fn candidates(ctx: &Context, data: &Data) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = match ctx {
        Context::TagName { prefix } => TAGS
            .iter()
            .filter(|t| starts_ci(t, prefix))
            .map(|t| plain(t, Kind::Tag))
            .collect(),
        Context::AttrName { tag, prefix } => {
            let mut v: Vec<&str> = tag_attrs(&tag.to_ascii_lowercase()).to_vec();
            v.extend(GLOBAL_ATTRS);
            v.into_iter()
                .filter(|a| starts_ci(a, prefix))
                .map(|a| Candidate {
                    label: a.to_string(),
                    insert: format!("{a}=\"\""),
                    back: 1,
                    kind: Kind::Attribute,
                    detail: None,
                })
                .collect()
        }
        Context::CssProperty { prefix } => CSS_PROPERTIES
            .iter()
            .filter(|p| starts_ci(p, prefix))
            .map(|p| Candidate {
                label: p.to_string(),
                insert: format!("{p}: "),
                back: 0,
                kind: Kind::Property,
                detail: None,
            })
            .collect(),
        Context::ClassValue { prefix } => data
            .classes
            .iter()
            .filter(|c| starts_ci(c, prefix) && *c != prefix)
            .map(|c| plain(c, Kind::Class))
            .collect(),
        Context::LinkValue { prefix } | Context::CssUrl { prefix } => link_candidates(prefix, data),
        Context::Word { prefix } => {
            if prefix.chars().count() < 3 {
                Vec::new()
            } else {
                data.words
                    .iter()
                    .filter(|w| starts_ci(w, prefix) && *w != prefix)
                    .map(|w| plain(w, Kind::Word))
                    .collect()
            }
        }
        Context::None => Vec::new(),
    };
    out.dedup_by(|a, b| a.label == b.label && a.kind == b.kind);
    out.truncate(MAX_PROPOSALS);
    out
}

fn link_candidates(prefix: &str, data: &Data) -> Vec<Candidate> {
    if let Some(hash) = prefix.find('#') {
        let (file_part, id_prefix) = (&prefix[..hash], &prefix[hash + 1..]);
        let target = if file_part.is_empty() {
            data.current.to_string()
        } else {
            resolve_href(parent_dir(data.current), file_part).0
        };
        return (data.ids_of)(&target)
            .into_iter()
            .filter(|id| starts_ci(id, id_prefix))
            .map(|id| Candidate {
                detail: (data.snippet_of)(&target, &id).filter(|t| !t.is_empty()),
                ..plain(&id, Kind::Anchor)
            })
            .collect();
    }
    if prefix.contains("://") || prefix.starts_with("mailto:") {
        return Vec::new();
    }
    // subsequence matching: word starts and the start of the base name count most (CE-345)
    let rels: Vec<String> = data
        .files
        .iter()
        .filter(|f| *f != data.current)
        .map(|f| relative_href(data.current, f))
        .collect();
    let mut scored: Vec<(i32, &String)> = rels
        .iter()
        .filter_map(|rel| {
            let base = rel.rsplit('/').next().unwrap_or(rel);
            let exact =
                starts_ci(rel, prefix) || (!prefix.contains('/') && starts_ci(base, prefix));
            bookx_editing::fuzzy::score(prefix, rel)
                .map(|s| (s + if exact { 10_000 } else { 0 }, rel))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
    scored
        .into_iter()
        .map(|(_, r)| plain(r, Kind::File))
        .collect()
}

/// Distinct words of at least 4 letters, most frequent first (the fallback proposals).
pub fn words_of(text: &str) -> Vec<String> {
    use std::collections::HashMap;
    // only text between tags counts
    let mut plain = String::with_capacity(text.len() / 2);
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => {
                in_tag = true;
                plain.push(' ');
            }
            '>' => in_tag = false,
            _ if !in_tag => plain.push(c),
            _ => {}
        }
    }
    let mut count: HashMap<String, usize> = HashMap::new();
    for w in plain.split(|c: char| !(c.is_alphabetic() || c == '\'' || c == '’')) {
        if w.chars().count() >= 4 {
            *count.entry(w.to_string()).or_default() += 1;
        }
    }
    let mut v: Vec<(String, usize)> = count.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.into_iter().map(|(w, _)| w).take(4000).collect()
}

/// Class names written as `.name` in stylesheet text.
pub fn classes_of(css: &str) -> Vec<String> {
    let mut v = Vec::new();
    let b = css.as_bytes();
    let mut i = 0;
    let mut depth = 0i32;
    while i < b.len() {
        match b[i] {
            b'{' => depth += 1,
            b'}' => depth = (depth - 1).max(0),
            b'.' if depth == 0 => {
                let s = i + 1;
                let mut e = s;
                while e < b.len() && (b[e].is_ascii_alphanumeric() || b[e] == b'-' || b[e] == b'_')
                {
                    e += 1;
                }
                if e > s && !b[s].is_ascii_digit() {
                    v.push(css[s..e].to_string());
                }
                i = e;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    v.sort();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data<'a>(
        files: &'a [String],
        words: &'a [String],
        classes: &'a [String],
        ids: &'a dyn Fn(&str) -> Vec<String>,
    ) -> Data<'a> {
        Data {
            current: "OEBPS/ch1.xhtml",
            files,
            ids_of: ids,
            classes,
            words,
            snippet_of: &|_, id| (id == "zzmark").then(|| "The mark".to_string()),
        }
    }

    #[test]
    fn contexts_in_markup() {
        assert_eq!(
            context("<p>x</p><di", false),
            Context::TagName {
                prefix: "di".into()
            }
        );
        assert_eq!(
            context("<a hr", false),
            Context::AttrName {
                tag: "a".into(),
                prefix: "hr".into()
            }
        );
        assert_eq!(
            context("<a ", false),
            Context::AttrName {
                tag: "a".into(),
                prefix: String::new()
            }
        );
        assert_eq!(
            context("<a href=\"ch2.xh", false),
            Context::LinkValue {
                prefix: "ch2.xh".into()
            }
        );
        assert_eq!(
            context("<img alt='x' src='../Images/c", false),
            Context::LinkValue {
                prefix: "../Images/c".into()
            }
        );
        assert_eq!(
            context("<p class=\"a no", false),
            Context::ClassValue {
                prefix: "no".into()
            }
        );
        assert_eq!(context("<p title=\"x", false), Context::None);
        assert_eq!(context("<p>text <!-- not", false), Context::None);
        assert_eq!(context("</", false), Context::None);
        assert_eq!(
            context("<p>The quick bro", false),
            Context::Word {
                prefix: "bro".into()
            }
        );
        // closed quotes are values no more
        assert_eq!(
            context("<a href=\"x.html\" re", false),
            Context::AttrName {
                tag: "a".into(),
                prefix: "re".into()
            }
        );
    }

    #[test]
    fn contexts_in_css() {
        assert_eq!(
            context("p { col", true),
            Context::CssProperty {
                prefix: "col".into()
            }
        );
        assert_eq!(
            context("p { color: red; mar", true),
            Context::CssProperty {
                prefix: "mar".into()
            }
        );
        assert_eq!(context("p { color: re", true), Context::None);
        assert_eq!(context("p.no", true), Context::None);
        assert_eq!(context("p { a: b } .x", true), Context::None);
        assert_eq!(
            context("@font-face { src: url('../Fonts/b", true),
            Context::CssUrl {
                prefix: "../Fonts/b".into()
            }
        );
        assert_eq!(context("/* mar", true), Context::None);
    }

    #[test]
    fn proposals_for_tags_attributes_and_properties() {
        let (f, w, c) = (vec![], vec![], vec![]);
        let ids = |_: &str| Vec::new();
        let d = data(&f, &w, &c, &ids);
        let tags = candidates(
            &Context::TagName {
                prefix: "bl".into(),
            },
            &d,
        );
        assert_eq!(tags[0].label, "blockquote");
        assert!(
            candidates(&Context::TagName { prefix: "H".into() }, &d)
                .iter()
                .any(|t| t.label == "h1")
        );
        let attrs = candidates(
            &Context::AttrName {
                tag: "img".into(),
                prefix: "s".into(),
            },
            &d,
        );
        let src = attrs.iter().find(|a| a.label == "src").unwrap();
        assert_eq!(src.insert, "src=\"\"");
        assert_eq!(src.back, 1);
        assert!(attrs.iter().any(|a| a.label == "style"));
        assert!(!attrs.iter().any(|a| a.label == "href"));
        let props = candidates(
            &Context::CssProperty {
                prefix: "font-w".into(),
            },
            &d,
        );
        assert_eq!(props.len(), 1);
        assert_eq!(props[0].insert, "font-weight: ");
    }

    #[test]
    fn proposals_for_files_and_anchors() {
        let files: Vec<String> = [
            "OEBPS/ch1.xhtml",
            "OEBPS/ch2.xhtml",
            "OEBPS/Images/cover.jpg",
            "OEBPS/style.css",
        ]
        .map(String::from)
        .to_vec();
        let (w, c) = (vec![], vec![]);
        let ids = |p: &str| match p {
            "OEBPS/ch2.xhtml" => vec!["intro".to_string(), "end".to_string()],
            "OEBPS/ch1.xhtml" => vec!["top".to_string()],
            _ => vec![],
        };
        let d = data(&files, &w, &c, &ids);
        let l = candidates(
            &Context::LinkValue {
                prefix: "ch".into(),
            },
            &d,
        );
        assert_eq!(
            l.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
            ["ch2.xhtml"],
            "the open file itself is not offered"
        );
        let img = candidates(
            &Context::LinkValue {
                prefix: "Images/".into(),
            },
            &d,
        );
        assert_eq!(img[0].label, "Images/cover.jpg");
        let a = candidates(
            &Context::LinkValue {
                prefix: "ch2.xhtml#in".into(),
            },
            &d,
        );
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].label, "intro");
        assert_eq!(a[0].kind, Kind::Anchor);
        let own = candidates(&Context::LinkValue { prefix: "#".into() }, &d);
        assert_eq!(own[0].label, "top");
        assert_eq!(
            prefix_len(&Context::LinkValue {
                prefix: "ch2.xhtml#in".into()
            }),
            2,
            "only the id part is replaced"
        );
        assert_eq!(
            prefix_len(&Context::LinkValue {
                prefix: "ch".into()
            }),
            2
        );
        assert!(
            candidates(
                &Context::LinkValue {
                    prefix: "https://x".into()
                },
                &d
            )
            .is_empty()
        );
    }

    #[test]
    fn words_and_classes() {
        let w = words_of("<p class=\"zzzz\">Alpha beta alpha Gamma alpha beta</p>");
        assert_eq!(w[0], "alpha");
        assert!(w.contains(&"Alpha".to_string()));
        assert!(
            !w.contains(&"zzzz".to_string()),
            "attribute values are not words"
        );
        let c = classes_of(
            ".note { color: red } p.big, .small > b { x: y } @media { .inner { a: b } }",
        );
        assert!(c.contains(&"note".to_string()));
        assert!(c.contains(&"big".to_string()));
        assert!(c.contains(&"small".to_string()));
        let (f, ci) = (vec![], c.clone());
        let ids = |_: &str| Vec::new();
        let ww = words_of("Quickly quickest quickly");
        let d = data(&f, &ww, &ci, &ids);
        assert_eq!(
            candidates(
                &Context::ClassValue {
                    prefix: "sm".into()
                },
                &d
            )[0]
            .label,
            "small"
        );
        let cand = candidates(
            &Context::Word {
                prefix: "quic".into(),
            },
            &d,
        );
        assert_eq!(cand[0].label.to_lowercase(), "quickly");
        assert!(
            candidates(
                &Context::Word {
                    prefix: "qu".into()
                },
                &d
            )
            .is_empty()
        );
    }
}
