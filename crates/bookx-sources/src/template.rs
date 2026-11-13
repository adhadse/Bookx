//! A small subset of the Calibre template language for save paths and display strings (L24).
//!
//! ```text
//! {title}                       a field
//! {title:lower|trunc(30)}       filters, applied left to right
//! {series?Series - : - }        prefix and suffix, only when the field is not empty
//! {if series}...{else}...{endif}   conditional on a non-empty field (`{if !series}` negates)
//! {{ and }}                     literal braces
//! /                             starts a new path component (only in path rendering)
//! ```
//!
//! Fields: `title`, `title_sort`, `authors`, `author` (first author), `author_sort`, `series`,
//! `series_index`, `publisher`, `language`, `published`, `year`, `tags`, `isbn`, `rating`.
//! Filters: `lower`, `upper`, `title`, `initial`, `sanitize`, `first_author`, `trunc(N)`, `pad(N)`.
//!
//! Path rendering never lets field values create path components or traverse directories.

use std::fmt;

use bookx_core::opf::{format_index, Metadata};

use crate::authors::author_sort;

pub const FIELDS: &[&str] = &[
    "title",
    "title_sort",
    "authors",
    "author",
    "author_sort",
    "series",
    "series_index",
    "publisher",
    "language",
    "published",
    "year",
    "tags",
    "isbn",
    "rating",
];

/// Longest path component, in characters (and 240 bytes, see [`MAX_COMPONENT_BYTES`]).
pub const MAX_COMPONENT_CHARS: usize = 120;
pub const MAX_COMPONENT_BYTES: usize = 240;

/// A template problem with its position (character offset into the template source).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateError {
    pub pos: usize,
    pub message: String,
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "template error at position {}: {}",
            self.pos, self.message
        )
    }
}

impl std::error::Error for TemplateError {}

#[derive(Debug, Clone, PartialEq)]
enum Filter {
    Lower,
    Upper,
    Title,
    Initial,
    Sanitize,
    FirstAuthor,
    Trunc(usize),
    Pad(usize),
}

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Lit(String),
    Field(String, Vec<Filter>),
    Opt {
        field: String,
        prefix: String,
        suffix: String,
    },
    If {
        field: String,
        negate: bool,
        then: Vec<Node>,
        otherwise: Vec<Node>,
    },
}

/// A parsed template.
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    nodes: Vec<Node>,
}

fn err<T>(pos: usize, message: impl Into<String>) -> Result<T, TemplateError> {
    Err(TemplateError {
        pos,
        message: message.into(),
    })
}

fn check_field(name: &str, pos: usize) -> Result<(), TemplateError> {
    if FIELDS.contains(&name) {
        Ok(())
    } else {
        err(pos, format!("unknown field `{name}`"))
    }
}

fn parse_filter(text: &str, pos: usize) -> Result<Filter, TemplateError> {
    let t = text.trim();
    let arg = |name: &str| -> Option<Result<usize, TemplateError>> {
        let inner = t.strip_prefix(name)?.strip_prefix('(')?.strip_suffix(')')?;
        Some(inner.trim().parse::<usize>().or_else(|_| {
            err(
                pos,
                format!("filter `{name}` needs a whole number, got `{inner}`"),
            )
        }))
    };
    Ok(match t {
        "lower" => Filter::Lower,
        "upper" => Filter::Upper,
        "title" => Filter::Title,
        "initial" => Filter::Initial,
        "sanitize" => Filter::Sanitize,
        "first_author" => Filter::FirstAuthor,
        _ => {
            if let Some(n) = arg("trunc") {
                Filter::Trunc(n?)
            } else if let Some(n) = arg("pad") {
                Filter::Pad(n?)
            } else {
                return err(pos, format!("unknown filter `{t}`"));
            }
        }
    })
}

enum Tok {
    Lit(String),
    Tag { text: String, pos: usize },
}

fn tokenize(src: &str) -> Result<Vec<Tok>, TemplateError> {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let mut lit = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '{' if chars.get(i + 1) == Some(&'{') => {
                lit.push('{');
                i += 2;
            }
            '}' if chars.get(i + 1) == Some(&'}') => {
                lit.push('}');
                i += 2;
            }
            '}' => return err(i, "unmatched `}` (write `}}` for a literal brace)"),
            '{' => {
                let start = i;
                let Some(len) = chars[i + 1..].iter().position(|c| *c == '}') else {
                    return err(start, "unclosed `{`");
                };
                let text: String = chars[i + 1..i + 1 + len].iter().collect();
                if text.contains('{') {
                    return err(start, "`{` inside a tag");
                }
                if !lit.is_empty() {
                    toks.push(Tok::Lit(std::mem::take(&mut lit)));
                }
                toks.push(Tok::Tag {
                    text,
                    pos: start + 1,
                });
                i += len + 2;
            }
            c => {
                lit.push(c);
                i += 1;
            }
        }
    }
    if !lit.is_empty() {
        toks.push(Tok::Lit(lit));
    }
    Ok(toks)
}

struct Frame {
    field: String,
    negate: bool,
    then: Vec<Node>,
    otherwise: Option<Vec<Node>>,
    pos: usize,
}

impl Template {
    /// Parses a template. Errors carry the character position of the problem.
    pub fn parse(src: &str) -> Result<Template, TemplateError> {
        let mut root: Vec<Node> = Vec::new();
        let mut stack: Vec<Frame> = Vec::new();
        fn current<'a>(root: &'a mut Vec<Node>, stack: &'a mut [Frame]) -> &'a mut Vec<Node> {
            match stack.last_mut() {
                None => root,
                Some(f) => f.otherwise.as_mut().unwrap_or(&mut f.then),
            }
        }
        for tok in tokenize(src)? {
            match tok {
                Tok::Lit(s) => current(&mut root, &mut stack).push(Node::Lit(s)),
                Tok::Tag { text, pos } => {
                    let t = text.trim();
                    if let Some(cond) = t.strip_prefix("if ").or_else(|| t.strip_prefix("if\t")) {
                        let cond = cond.trim();
                        let (negate, name) = match cond.strip_prefix('!') {
                            Some(n) => (true, n.trim()),
                            None => (false, cond),
                        };
                        let name_pos = pos + text.find(name).unwrap_or(0);
                        check_field(name, name_pos)?;
                        stack.push(Frame {
                            field: name.to_string(),
                            negate,
                            then: Vec::new(),
                            otherwise: None,
                            pos,
                        });
                    } else if t == "else" {
                        match stack.last_mut() {
                            None => return err(pos, "`{else}` without `{if}`"),
                            Some(f) if f.otherwise.is_some() => {
                                return err(pos, "second `{else}` in one `{if}`")
                            }
                            Some(f) => f.otherwise = Some(Vec::new()),
                        }
                    } else if t == "endif" {
                        let Some(f) = stack.pop() else {
                            return err(pos, "`{endif}` without `{if}`");
                        };
                        current(&mut root, &mut stack).push(Node::If {
                            field: f.field,
                            negate: f.negate,
                            then: f.then,
                            otherwise: f.otherwise.unwrap_or_default(),
                        });
                    } else if let Some((name, rest)) = text.split_once('?') {
                        let name = name.trim();
                        check_field(name, pos)?;
                        let (prefix, suffix) = rest.split_once(':').unwrap_or((rest, ""));
                        current(&mut root, &mut stack).push(Node::Opt {
                            field: name.to_string(),
                            prefix: prefix.to_string(),
                            suffix: suffix.to_string(),
                        });
                    } else {
                        let (name, filters) = match text.split_once(':') {
                            Some((n, f)) => (n, Some(f)),
                            None => (text.as_str(), None),
                        };
                        let name = name.trim();
                        let lead = text.len() - text.trim_start().len();
                        check_field(name, pos + lead)?;
                        let mut list = Vec::new();
                        if let Some(f) = filters {
                            let mut off = pos + text.find(':').unwrap_or(0) + 1;
                            for part in f.split('|') {
                                list.push(parse_filter(part, off)?);
                                off += part.chars().count() + 1;
                            }
                        }
                        current(&mut root, &mut stack).push(Node::Field(name.to_string(), list));
                    }
                }
            }
        }
        if let Some(f) = stack.last() {
            return err(f.pos.saturating_sub(1), "`{if}` without `{endif}`");
        }
        Ok(Template { nodes: root })
    }

    /// Names of the fields the template reads.
    pub fn fields_used(&self) -> Vec<String> {
        fn walk(nodes: &[Node], out: &mut Vec<String>) {
            for n in nodes {
                match n {
                    Node::Lit(_) => {}
                    Node::Field(f, _) | Node::Opt { field: f, .. } => {
                        if !out.contains(f) {
                            out.push(f.clone());
                        }
                    }
                    Node::If {
                        field,
                        then,
                        otherwise,
                        ..
                    } => {
                        if !out.contains(field) {
                            out.push(field.clone());
                        }
                        walk(then, out);
                        walk(otherwise, out);
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.nodes, &mut out);
        out
    }

    /// Renders for display: values are inserted as they are, `/` is an ordinary character.
    pub fn render(&self, md: &Metadata) -> String {
        let mut out = String::new();
        render_nodes(&self.nodes, md, false, &mut out);
        out
    }

    /// Renders a relative save path: components split at template `/`, every value and component
    /// made filesystem safe, `ext` (without dot, may be empty) appended to the last component.
    /// Never returns an empty list, empty components, `.` or `..`.
    pub fn render_path(&self, md: &Metadata, ext: &str) -> Vec<String> {
        let mut raw = String::new();
        render_nodes(&self.nodes, md, true, &mut raw);
        let mut comps: Vec<String> = raw
            .split('/')
            .map(|c| clean_component(c, 0))
            .filter(|c| !c.is_empty())
            .collect();
        if comps.is_empty() {
            comps.push("Unknown".into());
        }
        let ext = sanitize_value(ext).trim_matches('.').to_string();
        let ext_len = if ext.is_empty() { 0 } else { ext.len() + 1 };
        // re-clean the last one with room for the extension
        let last_raw = raw.rsplit('/').find(|c| !clean_component(c, 0).is_empty());
        if let Some(l) = last_raw {
            if let Some(slot) = comps.last_mut() {
                *slot = clean_component(l, ext_len);
            }
        }
        if !ext.is_empty() {
            if let Some(slot) = comps.last_mut() {
                slot.push('.');
                slot.push_str(&ext);
            }
        }
        comps
    }
}

fn field_value(md: &Metadata, name: &str) -> String {
    match name {
        "title" => md.title.trim().to_string(),
        "title_sort" => {
            if md.title_sort.trim().is_empty() {
                bookx_core::util::title_sort(&md.title)
            } else {
                md.title_sort.trim().to_string()
            }
        }
        "authors" => md
            .authors
            .iter()
            .map(|a| a.trim())
            .filter(|a| !a.is_empty())
            .collect::<Vec<_>>()
            .join(" & "),
        "author" => md
            .authors
            .iter()
            .map(|a| a.trim())
            .find(|a| !a.is_empty())
            .unwrap_or("")
            .to_string(),
        "author_sort" => {
            if md.author_sort.trim().is_empty() {
                author_sort(&md.authors)
            } else {
                md.author_sort.trim().to_string()
            }
        }
        "series" => md.series.trim().to_string(),
        "series_index" => {
            if md.series.trim().is_empty() || md.series_index <= 0.0 {
                String::new()
            } else {
                format_index(md.series_index)
            }
        }
        "publisher" => md.publisher.trim().to_string(),
        "language" => md.language.trim().to_string(),
        "published" => md.published.trim().to_string(),
        "year" => {
            let y: String = md.published.trim().chars().take(4).collect();
            if y.len() == 4 && y.chars().all(|c| c.is_ascii_digit()) {
                y
            } else {
                String::new()
            }
        }
        "tags" => md
            .tags
            .iter()
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        "isbn" => md
            .identifiers
            .iter()
            .find(|i| i.scheme.eq_ignore_ascii_case("isbn"))
            .map(|i| i.value.trim().to_string())
            .unwrap_or_default(),
        "rating" => {
            if md.rating == 0 {
                String::new()
            } else {
                // stored 0..=10 (two per star)
                format_index(md.rating as f64 / 2.0)
            }
        }
        _ => String::new(),
    }
}

fn apply_filter(f: &Filter, v: String) -> String {
    match f {
        Filter::Lower => v.to_lowercase(),
        Filter::Upper => v.to_uppercase(),
        Filter::Title => title_case(&v),
        Filter::Initial => match v.chars().find(|c| c.is_alphanumeric()) {
            Some(c) => c.to_uppercase().collect(),
            None if v.is_empty() => String::new(),
            None => "#".into(),
        },
        Filter::Sanitize => sanitize_value(&v),
        Filter::FirstAuthor => v.split(" & ").next().unwrap_or("").to_string(),
        Filter::Trunc(n) => truncate_chars(&v, *n),
        Filter::Pad(n) => {
            let digits = v.chars().take_while(|c| c.is_ascii_digit()).count();
            if digits == 0 || digits >= *n {
                v
            } else {
                format!("{}{}", "0".repeat(n - digits), v)
            }
        }
    }
}

fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut up = true;
    for c in s.chars() {
        if up {
            out.extend(c.to_uppercase());
        } else {
            out.extend(c.to_lowercase());
        }
        up = !c.is_alphanumeric() && c != '\'' && c != '\u{2019}';
    }
    out
}

fn is_combining(c: char) -> bool {
    matches!(c, '\u{0300}'..='\u{036F}' | '\u{200D}' | '\u{FE00}'..='\u{FE0F}' | '\u{1AB0}'..='\u{1AFF}' | '\u{20D0}'..='\u{20FF}')
}

/// First `n` chars without cutting a combining sequence in half, trailing spaces removed.
pub fn truncate_chars(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        // a combining mark right after the cut belongs to the last kept char: drop that char
        let next = s.chars().nth(n);
        if next.is_some_and(is_combining) {
            out.pop();
        }
        // dangling joiner / variation selector at the end
        while out.chars().last().is_some_and(|c| c == '\u{200D}') {
            out.pop();
        }
    }
    out.trim_end().to_string()
}

fn truncate_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut cut = s[..end].to_string();
    // do not leave a base char whose combining marks were cut off in the middle of a sequence
    if s[end..].chars().next().is_some_and(is_combining) {
        cut.pop();
    }
    cut.trim_end().to_string()
}

/// Makes one value safe to place inside a path component: separators and characters that are
/// invalid on common filesystems become `_`, control characters are dropped.
pub fn sanitize_value(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect()
}

const RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Final cleanup of one component: invalid characters, edge dots and spaces, reserved names and
/// the length limits (`reserve` bytes/chars are kept free for an extension).
fn clean_component(c: &str, reserve: usize) -> String {
    let s = sanitize_value(c);
    let s = s.trim_matches(|ch: char| ch == '.' || ch.is_whitespace());
    let mut s = truncate_chars(s, MAX_COMPONENT_CHARS.saturating_sub(reserve));
    s = truncate_bytes(&s, MAX_COMPONENT_BYTES.saturating_sub(reserve));
    let s = s.trim_matches(|ch: char| ch == '.' || ch.is_whitespace());
    let stem = s.split('.').next().unwrap_or("").to_lowercase();
    if RESERVED.contains(&stem.as_str()) {
        format!("_{s}")
    } else {
        s.to_string()
    }
}

fn render_nodes(nodes: &[Node], md: &Metadata, path: bool, out: &mut String) {
    let val = |name: &str, filters: &[Filter]| -> String {
        let mut v = field_value(md, name);
        for f in filters {
            v = apply_filter(f, v);
        }
        if path {
            sanitize_value(&v)
        } else {
            v
        }
    };
    for n in nodes {
        match n {
            Node::Lit(s) => out.push_str(s),
            Node::Field(name, filters) => out.push_str(&val(name, filters)),
            Node::Opt {
                field,
                prefix,
                suffix,
            } => {
                let v = val(field, &[]);
                if !v.trim().is_empty() {
                    out.push_str(prefix);
                    out.push_str(&v);
                    out.push_str(suffix);
                }
            }
            Node::If {
                field,
                negate,
                then,
                otherwise,
            } => {
                let truthy = !field_value(md, field).trim().is_empty();
                if truthy != *negate {
                    render_nodes(then, md, path, out);
                } else {
                    render_nodes(otherwise, md, path, out);
                }
            }
        }
    }
}

/// Parses and renders for display in one go.
pub fn render_template(src: &str, md: &Metadata) -> Result<String, TemplateError> {
    Ok(Template::parse(src)?.render(md))
}

/// Parses and renders a save path in one go; see [`Template::render_path`].
pub fn render_save_path(src: &str, md: &Metadata, ext: &str) -> Result<Vec<String>, TemplateError> {
    Ok(Template::parse(src)?.render_path(md, ext))
}

/// [`render_save_path`] with a `String` error: the shape `bookx_core::layout::TemplateFn` wants,
/// so a library can name its files with this template language.
pub fn render_library_path(src: &str, md: &Metadata, ext: &str) -> Result<Vec<String>, String> {
    render_save_path(src, md, ext).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_core::opf::Identifier;

    fn md() -> Metadata {
        Metadata {
            title: "The Wee Free Men".into(),
            authors: vec!["Terry Pratchett".into(), "Neil Gaiman".into()],
            series: "Discworld".into(),
            series_index: 30.0,
            publisher: "Doubleday".into(),
            language: "en".into(),
            published: "2003-05-01".into(),
            tags: vec!["Fantasy".into(), "Humour".into()],
            rating: 8,
            identifiers: vec![Identifier {
                scheme: "isbn".into(),
                value: "9780385032261".into(),
                xml_id: None,
            }],
            ..Default::default()
        }
    }

    fn r(t: &str) -> String {
        render_template(t, &md()).unwrap()
    }

    fn rp(t: &str, m: &Metadata, ext: &str) -> Vec<String> {
        render_save_path(t, m, ext).unwrap()
    }

    fn perr(t: &str) -> TemplateError {
        Template::parse(t).unwrap_err()
    }

    #[test]
    fn library_paths_render_for_the_layout_setting() {
        let md = Metadata {
            title: "Dune".into(),
            authors: vec!["Frank Herbert".into()],
            ..Default::default()
        };
        let f: bookx_core::layout::TemplateFn = render_library_path;
        assert_eq!(
            f("{author}/{title}", &md, "epub").unwrap(),
            vec!["Frank Herbert", "Dune.epub"]
        );
        assert!(f("{title", &md, "epub").is_err());
    }

    #[test]
    fn plain_fields() {
        assert_eq!(r("{title}"), "The Wee Free Men");
        assert_eq!(r("{authors}"), "Terry Pratchett & Neil Gaiman");
        assert_eq!(r("{author}"), "Terry Pratchett");
        assert_eq!(r("{author_sort}"), "Pratchett, Terry & Gaiman, Neil");
        assert_eq!(r("{title_sort}"), "Wee Free Men, The");
        assert_eq!(r("{series} {series_index}"), "Discworld 30");
        assert_eq!(
            r("{publisher}/{language}/{published}/{year}"),
            "Doubleday/en/2003-05-01/2003"
        );
        assert_eq!(r("{tags}"), "Fantasy, Humour");
        assert_eq!(r("{isbn}"), "9780385032261");
        assert_eq!(r("{rating}"), "4");
    }

    #[test]
    fn stored_sort_fields_win() {
        let mut m = md();
        m.title_sort = "Wee, The".into();
        m.author_sort = "Custom".into();
        assert_eq!(
            render_template("{title_sort}|{author_sort}", &m).unwrap(),
            "Wee, The|Custom"
        );
    }

    #[test]
    fn fractional_index_and_empty_fields() {
        let mut m = md();
        m.series_index = 1.5;
        assert_eq!(render_template("{series_index}", &m).unwrap(), "1.5");
        m.series.clear();
        assert_eq!(render_template("[{series_index}]", &m).unwrap(), "[]");
        let e = Metadata::default();
        assert_eq!(
            render_template("{title}|{authors}|{year}|{isbn}|{rating}|{tags}", &e).unwrap(),
            "|||||"
        );
        let weird = Metadata {
            published: "c. 1850".into(),
            ..Default::default()
        };
        assert_eq!(render_template("{year}", &weird).unwrap(), "");
    }

    #[test]
    fn filters() {
        assert_eq!(r("{title:lower}"), "the wee free men");
        assert_eq!(r("{title:upper}"), "THE WEE FREE MEN");
        assert_eq!(r("{title:lower|title}"), "The Wee Free Men");
        assert_eq!(r("{title:initial}"), "T");
        assert_eq!(r("{authors:first_author}"), "Terry Pratchett");
        assert_eq!(r("{title:trunc(7)}"), "The Wee");
        assert_eq!(r("{series_index:pad(3)}"), "030");
        assert_eq!(r("{title:lower|trunc(3)|upper}"), "THE");
        assert_eq!(r("{ title : lower }"), "the wee free men");
    }

    #[test]
    fn filter_edge_cases() {
        let mut m = md();
        m.title = "élan vital".into();
        assert_eq!(render_template("{title:initial}", &m).unwrap(), "É");
        m.title = "42 things".into();
        assert_eq!(render_template("{title:initial}", &m).unwrap(), "4");
        m.title = "...".into();
        assert_eq!(render_template("{title:initial}", &m).unwrap(), "#");
        m.title = String::new();
        assert_eq!(render_template("{title:initial}", &m).unwrap(), "");
        m.title = "o'brien's day".into();
        assert_eq!(
            render_template("{title:title}", &m).unwrap(),
            "O'brien's Day"
        );
        m.series_index = 1234.0;
        assert_eq!(
            render_template("{series_index:pad(3)}", &m).unwrap(),
            "1234"
        );
        m.series_index = 1.5;
        assert_eq!(
            render_template("{series_index:pad(3)}", &m).unwrap(),
            "001.5"
        );
        assert_eq!(render_template("{series:pad(5)}", &m).unwrap(), "Discworld");
        assert_eq!(render_template("{title:trunc(0)}", &m).unwrap(), "");
    }

    #[test]
    fn sanitize_filter_replaces_separators() {
        let mut m = md();
        m.title = "AC/DC: Live?".into();
        assert_eq!(
            render_template("{title:sanitize}", &m).unwrap(),
            "AC_DC_ Live_"
        );
        assert_eq!(
            render_template("{title}", &m).unwrap(),
            "AC/DC: Live?",
            "display keeps text"
        );
    }

    #[test]
    fn trunc_is_unicode_safe() {
        assert_eq!(truncate_chars("日本語のタイトル", 3), "日本語");
        assert_eq!(truncate_chars("héllo", 2), "hé");
        // a combining accent after the cut must not be separated from its base letter
        let s = "ab\u{0065}\u{0301}cd";
        assert_eq!(truncate_chars(s, 3), "ab");
        assert_eq!(truncate_chars(s, 4), "ab\u{0065}\u{0301}");
        assert_eq!(truncate_chars("abc", 10), "abc");
        assert_eq!(truncate_chars("ab  cd", 4), "ab");
        assert_eq!(truncate_chars("👨\u{200D}👩", 2), "👨");
    }

    #[test]
    fn optional_prefix_suffix() {
        assert_eq!(r("{series?(: )}"), "(Discworld )");
        assert_eq!(r("{series?Series - }"), "Series - Discworld");
        assert_eq!(r("{series?[:]}"), "[Discworld]");
        let mut m = md();
        m.series.clear();
        assert_eq!(render_template("a{series?[:]}b", &m).unwrap(), "ab");
        m.series = "   ".into();
        assert_eq!(
            render_template("a{series?[:]}b", &m).unwrap(),
            "ab",
            "blank counts as empty"
        );
    }

    #[test]
    fn conditionals() {
        assert_eq!(r("{if series}S{endif}"), "S");
        assert_eq!(r("{if !series}none{else}has{endif}"), "has");
        let mut m = md();
        m.series.clear();
        assert_eq!(
            render_template("{if series}S{else}no series{endif}", &m).unwrap(),
            "no series"
        );
        assert_eq!(render_template("{if !series}x{endif}", &m).unwrap(), "x");
        assert_eq!(
            render_template(
                "{if series}{if publisher}both{else}pub-less{endif}{endif}",
                &md()
            )
            .unwrap(),
            "both"
        );
        let mut m2 = md();
        m2.publisher.clear();
        assert_eq!(
            render_template(
                "{if series}{if publisher}both{else}pub-less{endif}{endif}",
                &m2
            )
            .unwrap(),
            "pub-less"
        );
    }

    #[test]
    fn literal_braces() {
        assert_eq!(r("{{title}}"), "{title}");
        assert_eq!(r("a}}b{{c"), "a}b{c");
    }

    #[test]
    fn errors_have_positions() {
        let e = perr("abc {nope}");
        assert_eq!(e.pos, 5);
        assert!(e.message.contains("unknown field `nope`"), "{e}");
        assert_eq!(perr("{title:wrong}").pos, 7);
        assert!(perr("{title:wrong}")
            .message
            .contains("unknown filter `wrong`"));
        assert_eq!(perr("{title:lower|bad}").pos, 13);
        let e = perr("x {title");
        assert_eq!((e.pos, e.message.as_str()), (2, "unclosed `{`"));
        assert_eq!(perr("a}").pos, 1);
        assert!(perr("{if series}x").message.contains("without `{endif}`"));
        assert_eq!(perr("{if series}x").pos, 0);
        assert!(perr("{endif}").message.contains("without `{if}`"));
        assert!(perr("{else}").message.contains("without `{if}`"));
        assert!(perr("{if series}{else}{else}{endif}")
            .message
            .contains("second"));
        assert!(perr("{if nothing}{endif}")
            .message
            .contains("unknown field `nothing`"));
        assert!(perr("{series?a:b").message.contains("unclosed"));
        assert!(perr("{ti{tle}}").message.contains("inside a tag"));
        assert!(perr("{trunc:trunc(x)}").message.contains("unknown field"));
        assert!(perr("{title:trunc(x)}").message.contains("whole number"));
        assert!(perr("{title:trunc(-1)}").message.contains("whole number"));
        assert!(perr("{nope?a:b}").message.contains("unknown field"));
    }

    #[test]
    fn error_position_counts_chars_not_bytes() {
        let e = perr("日本{nope}");
        assert_eq!(e.pos, 3);
        assert_eq!(
            e.to_string(),
            "template error at position 3: unknown field `nope`"
        );
    }

    #[test]
    fn fields_used_lists_each_once() {
        let t =
            Template::parse("{author_sort}/{series?a:b}/{if series}{title}{endif}{title}").unwrap();
        assert_eq!(t.fields_used(), vec!["author_sort", "series", "title"]);
    }

    #[test]
    fn path_basic() {
        assert_eq!(
            rp(
                "{author_sort:first_author}/{series?:}/{title}",
                &md(),
                "epub"
            ),
            vec!["Pratchett, Terry", "Discworld", "The Wee Free Men.epub"]
        );
    }

    #[test]
    fn path_skips_empty_components() {
        let mut m = md();
        m.series.clear();
        assert_eq!(
            rp("{authors:first_author}/{series}/{title}", &m, "epub"),
            vec!["Terry Pratchett", "The Wee Free Men.epub"]
        );
        assert_eq!(
            rp("{series}/{publisher}", &Metadata::default(), "epub"),
            vec!["Unknown.epub"]
        );
        assert_eq!(rp("", &md(), ""), vec!["Unknown"]);
        assert_eq!(rp("///", &md(), "epub"), vec!["Unknown.epub"]);
    }

    #[test]
    fn path_values_cannot_create_components() {
        let mut m = md();
        m.title = "AC/DC\\Live".into();
        assert_eq!(rp("{title}", &m, "epub"), vec!["AC_DC_Live.epub"]);
        m.title = "a/b".into();
        assert_eq!(rp("x/{title}/y", &m, ""), vec!["x", "a_b", "y"]);
    }

    #[test]
    fn path_traversal_is_neutralised() {
        for evil in [
            "..",
            "../..",
            "../../etc/passwd",
            "/etc/passwd",
            "..\\..\\windows",
            ".",
            "...",
            " .. ",
            "a/../../b",
        ] {
            let mut m = md();
            m.title = evil.into();
            m.authors = vec![evil.into()];
            let comps = rp("{authors}/{title}", &m, "epub");
            assert!(!comps.is_empty());
            for c in &comps {
                assert!(
                    !c.is_empty() && c != "." && c != "..",
                    "{evil:?} -> {comps:?}"
                );
                assert!(
                    !c.contains('/') && !c.contains('\\'),
                    "{evil:?} -> {comps:?}"
                );
                assert!(!c.starts_with('.'), "{evil:?} -> {comps:?}");
            }
        }
    }

    #[test]
    fn dotdot_in_template_literal_is_dropped() {
        assert_eq!(
            rp("../../{title}", &md(), "epub"),
            vec!["The Wee Free Men.epub"]
        );
        assert_eq!(rp("a/./b", &md(), ""), vec!["a", "b"]);
    }

    #[test]
    fn path_removes_invalid_characters_and_control_chars() {
        let mut m = md();
        m.title = "What? <Is> \"this\" | *really*: \u{0007}ok\n".into();
        assert_eq!(
            rp("{title}", &m, ""),
            vec!["What_ _Is_ _this_ _ _really__ ok"]
        );
    }

    #[test]
    fn reserved_windows_names_are_prefixed() {
        let mut m = md();
        m.title = "CON".into();
        assert_eq!(rp("{title}", &m, "epub"), vec!["_CON.epub"]);
        m.title = "nul".into();
        assert_eq!(rp("{title}", &m, ""), vec!["_nul"]);
        m.title = "Console".into();
        assert_eq!(rp("{title}", &m, ""), vec!["Console"]);
    }

    #[test]
    fn components_are_limited_in_chars_and_bytes() {
        let mut m = md();
        m.title = "x".repeat(500);
        let c = &rp("{title}", &m, "epub")[0];
        assert_eq!(c.chars().count(), 120);
        assert!(c.ends_with("x.epub") && c.len() <= 120);
        m.title = "日".repeat(500);
        let c = &rp("{title}", &m, "epub")[0];
        assert!(c.len() <= MAX_COMPONENT_BYTES, "{} bytes", c.len());
        assert!(c.ends_with("日.epub"));
        assert!(c.chars().count() <= 120);
        m.title = "é".repeat(500);
        let c = &rp("{title}", &m, "")[0];
        assert!(c.len() <= MAX_COMPONENT_BYTES && c.chars().count() <= 120);
    }

    #[test]
    fn truncated_component_does_not_end_in_dot_or_space() {
        let mut m = md();
        m.title = format!("{}. tail", "a".repeat(119));
        let c = &rp("{title}", &m, "")[0];
        assert!(!c.ends_with('.') && !c.ends_with(' '), "{c:?}");
    }

    #[test]
    fn extension_is_sanitised() {
        assert_eq!(rp("{title}", &md(), "../epub")[0], "The Wee Free Men._epub");
        assert_eq!(rp("{title}", &md(), ".mobi")[0], "The Wee Free Men.mobi");
    }

    #[test]
    fn unicode_values_survive() {
        let mut m = md();
        m.title = "Cien años de soledad".into();
        m.authors = vec!["Gabriel García Márquez".into()];
        assert_eq!(
            rp("{author}/{title}", &m, "epub"),
            vec!["Gabriel García Márquez", "Cien años de soledad.epub"]
        );
    }

    #[test]
    fn render_path_is_deterministic_and_never_panics_on_odd_input() {
        let mut m = Metadata::default();
        for t in [
            "\u{0}",
            "\u{202E}evil",
            "😀",
            "\u{0301}",
            "  ",
            "a\tb",
            "\u{FEFF}x",
        ] {
            m.title = t.into();
            m.authors = vec![t.into()];
            let a = rp("{authors}/{title:trunc(2)}", &m, "epub");
            let b = rp("{authors}/{title:trunc(2)}", &m, "epub");
            assert_eq!(a, b);
            assert!(a.iter().all(|c| !c.is_empty()));
        }
    }
}
