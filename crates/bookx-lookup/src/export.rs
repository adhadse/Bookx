//! Annotation export: Markdown, plain text, HTML, JSON, Kindle `My Clippings.txt`
//! and Obsidian notes, plus filename templates and a Readwise payload builder.

use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{json, Value};
use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HighlightStyle {
    Highlight,
    Underline,
    Strikeout,
    Squiggly,
    Invert,
    Bookmark,
}

impl HighlightStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Highlight => "highlight",
            Self::Underline => "underline",
            Self::Strikeout => "strikeout",
            Self::Squiggly => "squiggly",
            Self::Invert => "invert",
            Self::Bookmark => "bookmark",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "highlight" | "lighten" => Some(Self::Highlight),
            "underline" | "underscore" => Some(Self::Underline),
            "strikeout" | "strikethrough" => Some(Self::Strikeout),
            "squiggly" => Some(Self::Squiggly),
            "invert" => Some(Self::Invert),
            "bookmark" => Some(Self::Bookmark),
            _ => None,
        }
    }
}

/// A format-neutral annotation.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationItem {
    pub book_title: String,
    pub book_author: Option<String>,
    pub chapter: Option<String>,
    pub text: String,
    pub note: Option<String>,
    pub style: HighlightStyle,
    pub color: Option<String>,
    pub created: DateTime<Utc>,
    /// Page or location label, free text (numeric labels are used by Readwise).
    pub location: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Markdown,
    Text,
    Html,
    Json,
    KindleClippings,
    Obsidian,
    /// A Markdown note with the front matter that Joplin's Markdown import reads.
    Joplin,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Markdown | Self::Obsidian | Self::Joplin => "md",
            Self::Text | Self::KindleClippings => "txt",
            Self::Html => "html",
            Self::Json => "json",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "md" | "markdown" => Some(Self::Markdown),
            "txt" | "text" => Some(Self::Text),
            "html" => Some(Self::Html),
            "json" => Some(Self::Json),
            "kindle" | "clippings" => Some(Self::KindleClippings),
            "obsidian" => Some(Self::Obsidian),
            "joplin" => Some(Self::Joplin),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExportOptions {
    /// Keep only these styles (`None` = all).
    pub styles: Option<Vec<HighlightStyle>>,
    pub group_by_chapter: bool,
    pub include_notes: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            styles: None,
            group_by_chapter: false,
            include_notes: true,
        }
    }
}

/// Items that pass the style filter, in input order.
pub fn filter_items<'a>(
    items: &'a [AnnotationItem],
    opts: &ExportOptions,
) -> Vec<&'a AnnotationItem> {
    items
        .iter()
        .filter(|i| opts.styles.as_ref().is_none_or(|s| s.contains(&i.style)))
        .collect()
}

type BookGroup<'a> = ((&'a str, Option<&'a str>), Vec<&'a AnnotationItem>);

/// Groups items by `(title, author)` in order of first appearance.
pub fn group_by_book<'a>(items: &[&'a AnnotationItem]) -> Vec<BookGroup<'a>> {
    let mut out: Vec<BookGroup<'a>> = Vec::new();
    for &it in items {
        let key = (it.book_title.as_str(), it.book_author.as_deref());
        match out.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => v.push(it),
            None => out.push((key, vec![it])),
        }
    }
    out
}

fn chapter_groups<'a>(
    items: &[&'a AnnotationItem],
    opts: &ExportOptions,
) -> Vec<(Option<&'a str>, Vec<&'a AnnotationItem>)> {
    if !opts.group_by_chapter {
        return vec![(None, items.to_vec())];
    }
    let mut out: Vec<(Option<&str>, Vec<&AnnotationItem>)> = Vec::new();
    for &it in items {
        let ch = it
            .chapter
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty());
        match out.iter_mut().find(|(k, _)| *k == ch) {
            Some((_, v)) => v.push(it),
            None => out.push((ch, vec![it])),
        }
    }
    out
}

fn note_of<'a>(item: &'a AnnotationItem, opts: &ExportOptions) -> Option<&'a str> {
    if !opts.include_notes {
        return None;
    }
    item.note
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn safe_color(c: &str) -> Option<&str> {
    let c = c.trim();
    let hex = c.strip_prefix('#').is_some_and(|h| {
        matches!(h.len(), 3 | 4 | 6 | 8) && h.chars().all(|x| x.is_ascii_hexdigit())
    });
    let name = !c.is_empty() && c.len() <= 20 && c.chars().all(|x| x.is_ascii_alphabetic());
    (hex || name).then_some(c)
}

fn yaml_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn title_line(title: &str, author: Option<&str>) -> String {
    match author.map(one_line).filter(|a| !a.is_empty()) {
        Some(a) => format!("{} by {a}", one_line(title)),
        None => one_line(title),
    }
}

fn quote_lines(text: &str, prefix: &str) -> String {
    let t = text.trim();
    let mut out = String::new();
    for line in t.lines() {
        out.push_str(prefix);
        out.push_str(line.trim_end());
        out.push('\n');
    }
    if t.is_empty() {
        out.push_str(prefix.trim_end());
        out.push('\n');
    }
    out
}

fn render_markdown(
    title: &str,
    author: Option<&str>,
    items: &[&AnnotationItem],
    opts: &ExportOptions,
) -> String {
    let mut out = format!("# {}\n", one_line(title));
    if let Some(a) = author.map(one_line).filter(|a| !a.is_empty()) {
        let _ = writeln!(out, "*{a}*");
    }
    out.push('\n');
    for (chapter, group) in chapter_groups(items, opts) {
        if let Some(c) = chapter {
            let _ = write!(out, "## {}\n\n", one_line(c));
        }
        for it in group {
            if it.style == HighlightStyle::Bookmark && it.text.trim().is_empty() {
                out.push_str("> (bookmark)\n\n");
            } else {
                out.push_str(&quote_lines(&it.text, "> "));
                out.push('\n');
            }
            if let Some(n) = note_of(it, opts) {
                let _ = write!(out, "**Note:** {n}\n\n");
            }
        }
    }
    out
}

fn render_text(
    title: &str,
    author: Option<&str>,
    items: &[&AnnotationItem],
    opts: &ExportOptions,
) -> String {
    let head = title_line(title, author);
    let mut out = format!("{head}\n{}\n\n", "=".repeat(head.chars().count()));
    for (chapter, group) in chapter_groups(items, opts) {
        if let Some(c) = chapter {
            let _ = write!(out, "[{}]\n\n", one_line(c));
        }
        for it in group {
            let body = if it.text.trim().is_empty() {
                "(bookmark)"
            } else {
                it.text.trim()
            };
            for (i, line) in body.lines().enumerate() {
                out.push_str(if i == 0 { "- " } else { "  " });
                out.push_str(line.trim_end());
                out.push('\n');
            }
            if let Some(n) = note_of(it, opts) {
                for (i, line) in n.lines().enumerate() {
                    out.push_str(if i == 0 { "  Note: " } else { "  " });
                    out.push_str(line.trim_end());
                    out.push('\n');
                }
            }
        }
        out.push('\n');
    }
    out
}

fn render_html_section(
    title: &str,
    author: Option<&str>,
    items: &[&AnnotationItem],
    opts: &ExportOptions,
) -> String {
    let mut out = String::from("<section class=\"book\">\n");
    let _ = writeln!(out, "<h1>{}</h1>", html_escape(&one_line(title)));
    if let Some(a) = author.map(one_line).filter(|a| !a.is_empty()) {
        let _ = writeln!(out, "<p class=\"author\">{}</p>", html_escape(&a));
    }
    for (chapter, group) in chapter_groups(items, opts) {
        if let Some(c) = chapter {
            let _ = writeln!(out, "<h2>{}</h2>", html_escape(&one_line(c)));
        }
        for it in group {
            let style_attr = it
                .color
                .as_deref()
                .and_then(safe_color)
                .map(|c| format!(" style=\"border-color:{c}\""))
                .unwrap_or_default();
            let body = html_escape(it.text.trim()).replace('\n', "<br>\n");
            let _ = writeln!(
                out,
                "<blockquote class=\"{}\"{style_attr}>{body}</blockquote>",
                it.style.as_str()
            );
            if let Some(n) = note_of(it, opts) {
                let _ = writeln!(
                    out,
                    "<p class=\"note\">{}</p>",
                    html_escape(n).replace('\n', "<br>\n")
                );
            }
        }
    }
    out.push_str("</section>\n");
    out
}

fn render_json(books: &[BookGroup<'_>], opts: &ExportOptions, now: DateTime<Utc>) -> String {
    let books: Vec<Value> = books
        .iter()
        .map(|((title, author), items)| {
            let anns: Vec<Value> = items
                .iter()
                .map(|it| {
                    json!({
                        "id": crate::ids::item_id(it),
                        "chapter": it.chapter,
                        "text": it.text,
                        "note": note_of(it, opts),
                        "style": it.style.as_str(),
                        "color": it.color,
                        "created": it.created.to_rfc3339_opts(SecondsFormat::Secs, true),
                        "location": it.location,
                    })
                })
                .collect();
            json!({ "title": title, "author": author, "annotations": anns })
        })
        .collect();
    let doc = json!({
        "version": 1,
        "exported": now.to_rfc3339_opts(SecondsFormat::Secs, true),
        "books": books,
    });
    serde_json::to_string_pretty(&doc).unwrap_or_default()
}

fn kindle_date(t: DateTime<Utc>) -> String {
    t.format("%A, %B %-d, %Y %-I:%M:%S %p").to_string()
}

fn render_kindle(
    title: &str,
    author: Option<&str>,
    items: &[&AnnotationItem],
    opts: &ExportOptions,
) -> String {
    let head = match author.map(one_line).filter(|a| !a.is_empty()) {
        Some(a) => format!("{} ({a})", one_line(title)),
        None => one_line(title),
    };
    let mut out = String::new();
    let mut clip = |kind: &str, it: &AnnotationItem, body: &str| {
        let loc = it
            .location
            .as_deref()
            .map(one_line)
            .filter(|l| !l.is_empty())
            .map(|l| format!(" at location {l}"))
            .unwrap_or_default();
        let _ = write!(
            out,
            "{head}\n- Your {kind}{loc} | Added on {}\n\n{}\n==========\n",
            kindle_date(it.created),
            one_line(body)
        );
    };
    for it in items {
        if it.style == HighlightStyle::Bookmark {
            clip("Bookmark", it, "");
            continue;
        }
        if !it.text.trim().is_empty() {
            clip("Highlight", it, &it.text);
        }
        if let Some(n) = note_of(it, opts) {
            clip("Note", it, n);
        }
    }
    out
}

fn render_joplin(
    title: &str,
    author: Option<&str>,
    items: &[&AnnotationItem],
    opts: &ExportOptions,
    now: DateTime<Utc>,
) -> String {
    // Joplin reads `title`, `created`, `updated` and a `tags` list from the front matter
    let mut out = String::from("---\n");
    let _ = writeln!(out, "title: {}", yaml_quote(&one_line(title)));
    let _ = writeln!(
        out,
        "created: {}",
        now.to_rfc3339_opts(SecondsFormat::Secs, true)
    );
    out.push_str("tags:\n  - bookx\n  - highlights\n---\n\n");
    let _ = write!(out, "# {}\n\n", one_line(title));
    if let Some(a) = author.map(one_line).filter(|a| !a.is_empty()) {
        let _ = write!(out, "*{a}*\n\n");
    }
    for (chapter, group) in chapter_groups(items, opts) {
        if let Some(c) = chapter {
            let _ = write!(out, "## {}\n\n", one_line(c));
        }
        for it in group {
            if !it.text.trim().is_empty() {
                out.push_str(&quote_lines(&it.text, "> "));
                out.push('\n');
            }
            if let Some(n) = note_of(it, opts) {
                let _ = write!(out, "**Note:** {}\n\n", one_line(n));
            }
        }
    }
    out
}

fn render_obsidian(
    title: &str,
    author: Option<&str>,
    items: &[&AnnotationItem],
    opts: &ExportOptions,
    now: DateTime<Utc>,
) -> String {
    let mut out = String::from("---\n");
    let _ = writeln!(out, "title: {}", yaml_quote(&one_line(title)));
    if let Some(a) = author.map(one_line).filter(|a| !a.is_empty()) {
        let _ = writeln!(out, "author: {}", yaml_quote(&a));
    }
    out.push_str("tags: [bookx, highlights]\n");
    let _ = writeln!(
        out,
        "exported: {}",
        now.to_rfc3339_opts(SecondsFormat::Secs, true)
    );
    let _ = writeln!(out, "highlights: {}", items.len());
    out.push_str("---\n\n");
    let _ = write!(out, "# {}\n\n", one_line(title));
    for (chapter, group) in chapter_groups(items, opts) {
        if let Some(c) = chapter {
            let _ = write!(out, "## {}\n\n", one_line(c));
        }
        for it in group {
            // the style and, unless it is the usual yellow, the colour
            let mut label = match it.style {
                HighlightStyle::Highlight => String::new(),
                s => format!(" {}", s.as_str()),
            };
            if let Some(c) = it
                .color
                .as_deref()
                .and_then(safe_color)
                .filter(|c| !c.eq_ignore_ascii_case("yellow"))
            {
                label.push(' ');
                label.push_str(c);
            }
            let _ = writeln!(out, "> [!quote]{label}");
            if it.text.trim().is_empty() {
                out.push_str(">\n");
            } else {
                out.push_str(&quote_lines(&it.text, "> "));
            }
            out.push('\n');
            if let Some(n) = note_of(it, opts) {
                out.push_str("> [!note]\n");
                out.push_str(&quote_lines(n, "> "));
                out.push('\n');
            }
        }
    }
    out
}

/// Renders all (filtered) annotations in one string. With several books the
/// per-book documents are concatenated (HTML and JSON stay a single valid document).
pub fn export(
    items: &[AnnotationItem],
    format: ExportFormat,
    opts: &ExportOptions,
    now: DateTime<Utc>,
) -> String {
    let filtered = filter_items(items, opts);
    let books = group_by_book(&filtered);
    match format {
        ExportFormat::Json => render_json(&books, opts, now),
        ExportFormat::Html => {
            let title = match books.as_slice() {
                [((t, _), _)] => one_line(t),
                _ => "Annotations".to_string(),
            };
            let mut out = format!(
                "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>{}</title>\n</head>\n<body>\n",
                html_escape(&title)
            );
            for ((t, a), its) in &books {
                out.push_str(&render_html_section(t, *a, its, opts));
            }
            out.push_str("</body>\n</html>\n");
            out
        }
        _ => {
            let mut parts = Vec::new();
            for ((t, a), its) in &books {
                parts.push(match format {
                    ExportFormat::Markdown => render_markdown(t, *a, its, opts),
                    ExportFormat::Text => render_text(t, *a, its, opts),
                    ExportFormat::KindleClippings => render_kindle(t, *a, its, opts),
                    ExportFormat::Joplin => render_joplin(t, *a, its, opts, now),
                    _ => render_obsidian(t, *a, its, opts, now),
                });
            }
            parts.concat()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportFile {
    pub name: String,
    pub content: String,
}

/// Expands a filename template. Placeholders: `{title}`, `{author}`, `{date}`
/// (`%Y-%m-%d`) and `{date:<chrono format>}`; `{{` / `}}` give literal braces.
/// The result is sanitised for use as a file name (no extension added).
pub fn expand_filename(
    template: &str,
    title: &str,
    author: Option<&str>,
    now: DateTime<Utc>,
) -> Result<String> {
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '}' => bail!("unmatched '}}' in filename template"),
            '{' => {
                let mut name = String::new();
                let mut closed = false;
                for n in chars.by_ref() {
                    if n == '}' {
                        closed = true;
                        break;
                    }
                    name.push(n);
                }
                if !closed {
                    bail!("unclosed '{{' in filename template");
                }
                let (key, arg) = match name.split_once(':') {
                    Some((k, a)) => (k, Some(a)),
                    None => (name.as_str(), None),
                };
                match (key, arg) {
                    ("title", None) => out.push_str(title),
                    ("author", None) => out.push_str(author.unwrap_or("Unknown")),
                    ("date", fmt) => {
                        let fmt = fmt.unwrap_or("%Y-%m-%d");
                        write!(out, "{}", now.format(fmt)).map_err(|_| {
                            anyhow!("invalid date format {fmt:?} in filename template")
                        })?;
                    }
                    _ => bail!("unknown placeholder {{{name}}} in filename template"),
                }
            }
            c => out.push(c),
        }
    }
    Ok(sanitize_filename(&out))
}

/// Replaces characters that are unsafe in file names; never returns an empty string.
pub fn sanitize_filename(s: &str) -> String {
    let mapped: String = s
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let collapsed = one_line(&mapped);
    let trimmed: String = collapsed
        .trim_matches(|c: char| c == '.' || c.is_whitespace())
        .chars()
        .take(120)
        .collect();
    let trimmed = trimmed
        .trim_end_matches(|c: char| c == '.' || c.is_whitespace())
        .to_string();
    if trimmed.is_empty() {
        "annotations".to_string()
    } else {
        trimmed
    }
}

/// One file per book, named from `template` plus the format's extension. Name
/// clashes get ` (2)`, ` (3)`, … suffixes.
pub fn export_files(
    items: &[AnnotationItem],
    format: ExportFormat,
    opts: &ExportOptions,
    template: &str,
    now: DateTime<Utc>,
) -> Result<Vec<ExportFile>> {
    let filtered = filter_items(items, opts);
    let mut files: Vec<ExportFile> = Vec::new();
    for ((title, author), its) in group_by_book(&filtered) {
        let base = expand_filename(template, title, author, now)?;
        let mut name = format!("{base}.{}", format.extension());
        let mut n = 2;
        while files.iter().any(|f| f.name == name) {
            name = format!("{base} ({n}).{}", format.extension());
            n += 1;
        }
        let subset: Vec<AnnotationItem> = its.into_iter().cloned().collect();
        files.push(ExportFile {
            name,
            content: export(&subset, format, opts, now),
        });
    }
    Ok(files)
}

fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Body for Readwise's `POST /api/v2/highlights/` (no network access here).
/// Bookmarks and empty highlights are skipped; fields are cut to Readwise's limits.
pub fn readwise_payload(items: &[AnnotationItem], opts: &ExportOptions) -> Value {
    let highlights: Vec<Value> = filter_items(items, opts)
        .into_iter()
        .filter(|i| i.style != HighlightStyle::Bookmark && !i.text.trim().is_empty())
        .map(|i| {
            let mut h = json!({
                "text": truncate_chars(i.text.trim(), 8191),
                "title": truncate_chars(&one_line(&i.book_title), 511),
                "source_type": "bookx",
                "category": "books",
                "highlighted_at": i.created.to_rfc3339_opts(SecondsFormat::Secs, true),
            });
            if let Some(a) = i
                .book_author
                .as_deref()
                .map(one_line)
                .filter(|a| !a.is_empty())
            {
                h["author"] = json!(truncate_chars(&a, 1024));
            }
            if let Some(n) = note_of(i, opts) {
                h["note"] = json!(truncate_chars(n, 8191));
            }
            if let Some(loc) = i
                .location
                .as_deref()
                .and_then(|l| l.trim().parse::<u64>().ok())
            {
                h["location"] = json!(loc);
                h["location_type"] = json!("page");
            }
            h
        })
        .collect();
    json!({ "highlights": highlights })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(d: u32, h: u32, m: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 1, d, h, m, s).unwrap()
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 12, 8, 30, 0).unwrap()
    }

    fn a() -> AnnotationItem {
        AnnotationItem {
            book_title: "Dune & Co".into(),
            book_author: Some("Frank <Herbert>".into()),
            chapter: Some("Part 1".into()),
            text: "Fear is the “mind-killer”.\nI *must* not fear. <b>&</b>".into(),
            note: Some("Litany: `a` & b".into()),
            style: HighlightStyle::Highlight,
            color: Some("#ffee00".into()),
            created: at(2, 15, 4, 5),
            location: Some("42".into()),
        }
    }

    fn b() -> AnnotationItem {
        AnnotationItem {
            chapter: Some("Part 2".into()),
            text: "日本語 text".into(),
            note: None,
            style: HighlightStyle::Underline,
            color: None,
            created: at(3, 9, 5, 0),
            location: None,
            ..a()
        }
    }

    fn c() -> AnnotationItem {
        AnnotationItem {
            text: "second in part 1".into(),
            note: None,
            created: at(4, 0, 0, 0),
            location: None,
            ..a()
        }
    }

    fn opts() -> ExportOptions {
        ExportOptions::default()
    }

    #[test]
    fn markdown_flat_golden() {
        let out = export(&[a(), b()], ExportFormat::Markdown, &opts(), now());
        let expected = "# Dune & Co\n*Frank <Herbert>*\n\n\
> Fear is the “mind-killer”.\n> I *must* not fear. <b>&</b>\n\n\
**Note:** Litany: `a` & b\n\n\
> 日本語 text\n\n";
        assert_eq!(out, expected);
    }

    #[test]
    fn markdown_grouped_by_chapter_golden() {
        let o = ExportOptions {
            group_by_chapter: true,
            ..opts()
        };
        let out = export(&[a(), b(), c()], ExportFormat::Markdown, &o, now());
        let expected = "# Dune & Co\n*Frank <Herbert>*\n\n\
## Part 1\n\n\
> Fear is the “mind-killer”.\n> I *must* not fear. <b>&</b>\n\n\
**Note:** Litany: `a` & b\n\n\
> second in part 1\n\n\
## Part 2\n\n\
> 日本語 text\n\n";
        assert_eq!(out, expected);
    }

    #[test]
    fn markdown_without_author_notes_off_and_bookmarks() {
        let mut x = a();
        x.book_author = None;
        let mut bm = b();
        bm.style = HighlightStyle::Bookmark;
        bm.text = String::new();
        bm.book_author = None;
        let o = ExportOptions {
            include_notes: false,
            ..opts()
        };
        let out = export(&[x, bm], ExportFormat::Markdown, &o, now());
        assert_eq!(
            out,
            "# Dune & Co\n\n> Fear is the “mind-killer”.\n> I *must* not fear. <b>&</b>\n\n> (bookmark)\n\n"
        );
    }

    #[test]
    fn text_golden_flat_and_grouped() {
        let out = export(&[a(), b()], ExportFormat::Text, &opts(), now());
        let expected = format!(
            "Dune & Co by Frank <Herbert>\n{}\n\n- Fear is the “mind-killer”.\n  I *must* not fear. <b>&</b>\n  Note: Litany: `a` & b\n- 日本語 text\n\n",
            "=".repeat(28)
        );
        assert_eq!(out, expected);
        let o = ExportOptions {
            group_by_chapter: true,
            ..opts()
        };
        let out = export(&[b(), a()], ExportFormat::Text, &o, now());
        assert!(
            out.contains("[Part 2]\n\n- 日本語 text\n\n[Part 1]\n\n- Fear"),
            "{out}"
        );
    }

    #[test]
    fn html_golden_and_escaping() {
        let x = AnnotationItem {
            book_title: "T<1>".into(),
            book_author: Some("A&B".into()),
            chapter: None,
            text: "q\"uote'\nline".into(),
            note: None,
            style: HighlightStyle::Highlight,
            color: Some("red".into()),
            created: at(1, 0, 0, 0),
            location: None,
        };
        let out = export(&[x], ExportFormat::Html, &opts(), now());
        let expected = "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>T&lt;1&gt;</title>\n</head>\n<body>\n\
<section class=\"book\">\n<h1>T&lt;1&gt;</h1>\n<p class=\"author\">A&amp;B</p>\n\
<blockquote class=\"highlight\" style=\"border-color:red\">q&quot;uote&#39;<br>\nline</blockquote>\n\
</section>\n</body>\n</html>\n";
        assert_eq!(out, expected);
    }

    #[test]
    fn html_notes_chapters_and_hostile_content() {
        let mut x = a();
        x.color = Some("red;background:url(javascript:alert(1))".into());
        x.text = "<script>alert('x')</script>".into();
        x.note = Some("<img src=x onerror=1>\nsecond".into());
        let o = ExportOptions {
            group_by_chapter: true,
            ..opts()
        };
        let out = export(&[x], ExportFormat::Html, &o, now());
        assert!(!out.contains("<script>"));
        assert!(!out.contains("<img"));
        assert!(
            !out.contains("javascript"),
            "unsafe colour must be dropped: {out}"
        );
        assert!(out.contains("<h2>Part 1</h2>"));
        assert!(out.contains("&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt;"));
        assert!(out.contains("<p class=\"note\">&lt;img src=x onerror=1&gt;<br>\nsecond</p>"));
    }

    #[test]
    fn html_multiple_books_is_one_document() {
        let mut other = b();
        other.book_title = "Other".into();
        let out = export(&[a(), other], ExportFormat::Html, &opts(), now());
        assert_eq!(out.matches("<!DOCTYPE html>").count(), 1);
        assert_eq!(out.matches("<section class=\"book\">").count(), 2);
        assert!(out.contains("<title>Annotations</title>"));
    }

    #[test]
    fn json_is_valid_and_complete() {
        let out = export(&[a(), b()], ExportFormat::Json, &opts(), now());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["version"], 1);
        assert_eq!(v["exported"], "2026-11-12T08:30:00Z");
        assert_eq!(v["books"].as_array().unwrap().len(), 1);
        let book = &v["books"][0];
        assert_eq!(book["title"], "Dune & Co");
        assert_eq!(book["author"], "Frank <Herbert>");
        let anns = book["annotations"].as_array().unwrap();
        assert_eq!(anns.len(), 2);
        assert_eq!(
            anns[0]["text"],
            "Fear is the “mind-killer”.\nI *must* not fear. <b>&</b>"
        );
        assert_eq!(anns[0]["note"], "Litany: `a` & b");
        assert_eq!(anns[0]["style"], "highlight");
        assert_eq!(anns[0]["color"], "#ffee00");
        assert_eq!(anns[0]["created"], "2024-01-02T15:04:05Z");
        assert_eq!(anns[0]["location"], "42");
        assert_eq!(anns[0]["id"], crate::ids::item_id(&a()));
        assert_ne!(anns[0]["id"], anns[1]["id"]);
        let again = export(&[b(), a()], ExportFormat::Json, &opts(), now());
        let w: Value = serde_json::from_str(&again).unwrap();
        assert_eq!(
            w["books"][0]["annotations"][1]["id"], anns[0]["id"],
            "ids are stable between exports"
        );
        assert_eq!(anns[1]["text"], "日本語 text");
        assert!(anns[1]["note"].is_null());
        assert_eq!(anns[1]["style"], "underline");
        assert_eq!(anns[1]["chapter"], "Part 2");
    }

    #[test]
    fn json_empty_and_notes_off() {
        let out = export(&[], ExportFormat::Json, &opts(), now());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert!(v["books"].as_array().unwrap().is_empty());
        let o = ExportOptions {
            include_notes: false,
            ..opts()
        };
        let v: Value =
            serde_json::from_str(&export(&[a()], ExportFormat::Json, &o, now())).unwrap();
        assert!(v["books"][0]["annotations"][0]["note"].is_null());
    }

    #[test]
    fn kindle_clippings_golden() {
        let out = export(&[a(), b()], ExportFormat::KindleClippings, &opts(), now());
        let expected = "Dune & Co (Frank <Herbert>)\n\
- Your Highlight at location 42 | Added on Tuesday, January 2, 2024 3:04:05 PM\n\n\
Fear is the “mind-killer”. I *must* not fear. <b>&</b>\n\
==========\n\
Dune & Co (Frank <Herbert>)\n\
- Your Note at location 42 | Added on Tuesday, January 2, 2024 3:04:05 PM\n\n\
Litany: `a` & b\n\
==========\n\
Dune & Co (Frank <Herbert>)\n\
- Your Highlight | Added on Wednesday, January 3, 2024 9:05:00 AM\n\n\
日本語 text\n\
==========\n";
        assert_eq!(out, expected);
    }

    #[test]
    fn kindle_bookmarks_note_only_and_midnight() {
        let mut bm = b();
        bm.style = HighlightStyle::Bookmark;
        bm.text = String::new();
        bm.created = at(5, 0, 7, 9);
        bm.book_author = None;
        let mut note_only = c();
        note_only.text = String::new();
        note_only.note = Some("just a note".into());
        note_only.created = at(5, 12, 0, 0);
        note_only.book_author = None;
        let out = export(
            &[bm, note_only],
            ExportFormat::KindleClippings,
            &opts(),
            now(),
        );
        let expected = "Dune & Co\n- Your Bookmark | Added on Friday, January 5, 2024 12:07:09 AM\n\n\n==========\n\
Dune & Co\n- Your Note | Added on Friday, January 5, 2024 12:00:00 PM\n\njust a note\n==========\n";
        assert_eq!(out, expected);
    }

    #[test]
    fn obsidian_golden() {
        let o = ExportOptions {
            group_by_chapter: true,
            ..opts()
        };
        let out = export(&[a(), b()], ExportFormat::Obsidian, &o, now());
        let expected = "---\ntitle: \"Dune & Co\"\nauthor: \"Frank <Herbert>\"\ntags: [bookx, highlights]\nexported: 2026-11-12T08:30:00Z\nhighlights: 2\n---\n\n\
# Dune & Co\n\n\
## Part 1\n\n\
> [!quote] #ffee00\n> Fear is the “mind-killer”.\n> I *must* not fear. <b>&</b>\n\n\
> [!note]\n> Litany: `a` & b\n\n\
## Part 2\n\n\
> [!quote] underline\n> 日本語 text\n\n";
        assert_eq!(out, expected);
    }

    #[test]
    fn joplin_note_has_front_matter_tags_and_plain_quotes() {
        let o = ExportOptions {
            group_by_chapter: true,
            ..opts()
        };
        let out = export(&[a(), b()], ExportFormat::Joplin, &o, now());
        assert!(
            out.starts_with(
                "---\ntitle: \"Dune & Co\"\ncreated: 2026-11-12T08:30:00Z\ntags:\n  - bookx\n  - highlights\n---\n\n# Dune & Co\n\n*Frank <Herbert>*\n\n## Part 1\n\n"
            ),
            "{out}"
        );
        assert!(out.contains("> Fear is the “mind-killer”.\n"), "{out}");
        assert!(out.contains("**Note:** Litany: `a` & b\n"), "{out}");
        assert!(
            !out.contains("[!quote]"),
            "no Obsidian callouts in a Joplin note"
        );
        assert_eq!(
            out.matches("\n---\n").count(),
            1,
            "front matter closes once"
        );
        assert_eq!(
            ExportFormat::from_name("Joplin"),
            Some(ExportFormat::Joplin)
        );
        assert_eq!(ExportFormat::Joplin.extension(), "md");
        assert_eq!(export(&[], ExportFormat::Joplin, &opts(), now()), "");
    }

    #[test]
    fn obsidian_front_matter_is_escaped() {
        let mut x = a();
        x.book_title = "Say \"hi\"\\ now\nline".into();
        x.book_author = None;
        let out = export(&[x], ExportFormat::Obsidian, &opts(), now());
        assert!(
            out.starts_with("---\ntitle: \"Say \\\"hi\\\"\\\\ now line\"\ntags:"),
            "{out}"
        );
        // front matter is closed exactly once
        assert_eq!(out.matches("\n---\n").count(), 1);
    }

    #[test]
    fn multiple_books_concatenate_and_keep_order() {
        let mut other = b();
        other.book_title = "Zed".into();
        other.book_author = None;
        let out = export(
            &[other.clone(), a(), c()],
            ExportFormat::Markdown,
            &opts(),
            now(),
        );
        let zed = out.find("# Zed").unwrap();
        let dune = out.find("# Dune & Co").unwrap();
        assert!(zed < dune);
        assert_eq!(
            out.matches("# Dune & Co").count(),
            1,
            "same book is grouped"
        );
    }

    #[test]
    fn style_filter_applies_to_every_format() {
        let o = ExportOptions {
            styles: Some(vec![HighlightStyle::Underline]),
            ..opts()
        };
        for f in [
            ExportFormat::Markdown,
            ExportFormat::Text,
            ExportFormat::Html,
            ExportFormat::Json,
            ExportFormat::KindleClippings,
            ExportFormat::Obsidian,
        ] {
            let out = export(&[a(), b()], f, &o, now());
            assert!(!out.contains("mind-killer"), "{f:?}");
            assert!(out.contains("日本語"), "{f:?}");
        }
        let none = ExportOptions {
            styles: Some(vec![]),
            ..opts()
        };
        assert!(export(&[a()], ExportFormat::Markdown, &none, now()).is_empty());
        assert_eq!(filter_items(&[a(), b()], &o).len(), 1);
    }

    #[test]
    fn empty_input_is_handled_per_format() {
        assert_eq!(export(&[], ExportFormat::Markdown, &opts(), now()), "");
        assert_eq!(export(&[], ExportFormat::Text, &opts(), now()), "");
        assert_eq!(
            export(&[], ExportFormat::KindleClippings, &opts(), now()),
            ""
        );
        assert_eq!(export(&[], ExportFormat::Obsidian, &opts(), now()), "");
        assert!(export(&[], ExportFormat::Html, &opts(), now()).contains("<body>\n</body>"));
    }

    #[test]
    fn group_by_book_and_chapter_without_chapters() {
        let mut x = a();
        x.chapter = None;
        let mut y = b();
        y.chapter = Some("   ".into());
        let o = ExportOptions {
            group_by_chapter: true,
            ..opts()
        };
        let out = export(&[x, y], ExportFormat::Markdown, &o, now());
        assert!(!out.contains("## "), "{out}");
        let all = [a(), b()];
        let f = filter_items(&all, &opts());
        let g = group_by_book(&f);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].0, ("Dune & Co", Some("Frank <Herbert>")));
        assert_eq!(g[0].1.len(), 2);
    }

    #[test]
    fn same_title_different_author_are_different_books() {
        let mut y = b();
        y.book_author = Some("Other".into());
        let all = [a(), y];
        let f = filter_items(&all, &opts());
        assert_eq!(group_by_book(&f).len(), 2);
    }

    #[test]
    fn style_parse_and_format_names() {
        for s in [
            HighlightStyle::Highlight,
            HighlightStyle::Underline,
            HighlightStyle::Strikeout,
            HighlightStyle::Squiggly,
            HighlightStyle::Bookmark,
        ] {
            assert_eq!(HighlightStyle::parse(s.as_str()), Some(s));
        }
        assert_eq!(
            HighlightStyle::parse(" LIGHTEN "),
            Some(HighlightStyle::Highlight)
        );
        assert_eq!(
            HighlightStyle::parse("strikethrough"),
            Some(HighlightStyle::Strikeout)
        );
        assert_eq!(HighlightStyle::parse("nope"), None);
        assert_eq!(ExportFormat::from_name("MD"), Some(ExportFormat::Markdown));
        assert_eq!(
            ExportFormat::from_name("kindle"),
            Some(ExportFormat::KindleClippings)
        );
        assert_eq!(ExportFormat::from_name("pdf"), None);
        let ext: Vec<_> = [
            ExportFormat::Markdown,
            ExportFormat::Text,
            ExportFormat::Html,
            ExportFormat::Json,
            ExportFormat::KindleClippings,
            ExportFormat::Obsidian,
        ]
        .iter()
        .map(|f| f.extension())
        .collect();
        assert_eq!(ext, vec!["md", "txt", "html", "json", "txt", "md"]);
    }

    #[test]
    fn filename_templates() {
        let n = now();
        assert_eq!(
            expand_filename("{title}-{date:%Y%m%d}", "Dune", None, n).unwrap(),
            "Dune-20261112"
        );
        assert_eq!(
            expand_filename("{author} - {title}", "Dune", Some("F. Herbert"), n).unwrap(),
            "F. Herbert - Dune"
        );
        assert_eq!(
            expand_filename("{author}", "Dune", None, n).unwrap(),
            "Unknown"
        );
        assert_eq!(
            expand_filename("{date}", "x", None, n).unwrap(),
            "2026-11-12"
        );
        assert_eq!(
            expand_filename("{{title}} {title}", "T", None, n).unwrap(),
            "{title} T"
        );
        assert_eq!(expand_filename("a}}b", "T", None, n).unwrap(), "a}b");
        assert_eq!(
            expand_filename("{date:%H-%M}", "T", None, n).unwrap(),
            "08-30"
        );
        assert_eq!(expand_filename("plain", "T", None, n).unwrap(), "plain");
    }

    #[test]
    fn filename_template_errors() {
        let n = now();
        let e = |t: &str| expand_filename(t, "T", None, n).unwrap_err().to_string();
        assert!(e("{nope}").contains("unknown placeholder {nope}"));
        assert!(e("{title:x}").contains("unknown placeholder"));
        assert!(e("{title").contains("unclosed"));
        assert!(e("a}b").contains("unmatched"));
        assert!(e("{date:%Q%!}").contains("invalid date format"));
    }

    #[test]
    fn filename_sanitising() {
        let n = now();
        assert_eq!(
            expand_filename("{title}", "a/b\\c:d*e?f\"g<h>i|j", None, n).unwrap(),
            "a_b_c_d_e_f_g_h_i_j"
        );
        assert_eq!(
            expand_filename("{title}", "../../etc/passwd", None, n).unwrap(),
            "_.._etc_passwd"
        );
        assert_eq!(
            expand_filename("{title}", "  ...  ", None, n).unwrap(),
            "annotations"
        );
        assert_eq!(
            expand_filename("{title}", "", None, n).unwrap(),
            "annotations"
        );
        assert_eq!(
            expand_filename("{title}", "a\nb\tc", None, n).unwrap(),
            "a b c"
        );
        let long = "é".repeat(300);
        assert_eq!(
            expand_filename("{title}", &long, None, n)
                .unwrap()
                .chars()
                .count(),
            120
        );
        assert_eq!(sanitize_filename("Café ☕"), "Café ☕");
        assert_eq!(sanitize_filename("trailing."), "trailing");
    }

    #[test]
    fn export_files_one_per_book_with_unique_names() {
        let mut other = b();
        other.book_title = "Dune: Co".into(); // sanitises to "Dune_ Co"
        let mut third = b();
        third.book_title = "Dune? Co".into(); // also "Dune_ Co"
        let files = export_files(
            &[a(), other, third],
            ExportFormat::Markdown,
            &opts(),
            "{title}-{date:%Y%m%d}",
            now(),
        )
        .unwrap();
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "Dune & Co-20261112.md",
                "Dune_ Co-20261112.md",
                "Dune_ Co-20261112 (2).md"
            ]
        );
        assert!(files[0].content.starts_with("# Dune & Co\n"));
        assert!(!files[0].content.contains("日本語"));
        assert!(files[1].content.contains("日本語"));
        let clash = export_files(&[a(), a()], ExportFormat::Json, &opts(), "x", now()).unwrap();
        assert_eq!(clash.len(), 1, "same book is one file");
        assert_eq!(clash[0].name, "x.json");
        assert!(export_files(&[a()], ExportFormat::Json, &opts(), "{bad}", now()).is_err());
        assert!(export_files(&[], ExportFormat::Json, &opts(), "x", now())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn readwise_payload_shape_and_limits() {
        let mut long = c();
        long.text = "x".repeat(9000);
        long.book_title = "T".repeat(600);
        let mut bm = b();
        bm.style = HighlightStyle::Bookmark;
        let mut empty = b();
        empty.text = "  ".into();
        let v = readwise_payload(&[a(), long, bm, empty, b()], &opts());
        let hs = v["highlights"].as_array().unwrap();
        assert_eq!(hs.len(), 3, "bookmark and empty text are skipped");
        assert_eq!(
            hs[0]["text"],
            "Fear is the “mind-killer”.\nI *must* not fear. <b>&</b>"
        );
        assert_eq!(hs[0]["title"], "Dune & Co");
        assert_eq!(hs[0]["author"], "Frank <Herbert>");
        assert_eq!(hs[0]["note"], "Litany: `a` & b");
        assert_eq!(hs[0]["source_type"], "bookx");
        assert_eq!(hs[0]["category"], "books");
        assert_eq!(hs[0]["location"], 42);
        assert_eq!(hs[0]["location_type"], "page");
        assert_eq!(hs[0]["highlighted_at"], "2024-01-02T15:04:05Z");
        assert_eq!(hs[1]["text"].as_str().unwrap().chars().count(), 8191);
        assert_eq!(hs[1]["title"].as_str().unwrap().chars().count(), 511);
        assert!(hs[1].get("location").is_none());
        assert!(hs[1].get("note").is_none());
        assert_eq!(hs[2]["text"], "日本語 text");
    }

    #[test]
    fn readwise_filters_and_non_numeric_locations() {
        let mut x = a();
        x.location = Some("chapter 3".into());
        x.book_author = None;
        let o = ExportOptions {
            styles: Some(vec![HighlightStyle::Highlight]),
            include_notes: false,
            ..opts()
        };
        let v = readwise_payload(&[x, b()], &o);
        let hs = v["highlights"].as_array().unwrap();
        assert_eq!(hs.len(), 1);
        assert!(hs[0].get("location").is_none());
        assert!(hs[0].get("author").is_none());
        assert!(hs[0].get("note").is_none());
        assert_eq!(readwise_payload(&[], &opts()), json!({"highlights": []}));
    }

    #[test]
    fn unicode_and_markup_notes_survive_every_text_format() {
        let mut x = a();
        x.note = Some("日本語 ☕ **bold** [link](http://x) | pipe # hash".into());
        for f in [
            ExportFormat::Markdown,
            ExportFormat::Text,
            ExportFormat::KindleClippings,
            ExportFormat::Obsidian,
        ] {
            let out = export(&[x.clone()], f, &opts(), now());
            assert!(
                out.contains("日本語 ☕ **bold** [link](http://x) | pipe # hash"),
                "{f:?}"
            );
        }
        let v: Value =
            serde_json::from_str(&export(&[x], ExportFormat::Json, &opts(), now())).unwrap();
        assert_eq!(
            v["books"][0]["annotations"][0]["note"],
            "日本語 ☕ **bold** [link](http://x) | pipe # hash"
        );
    }

    #[test]
    fn export_is_deterministic() {
        for f in [
            ExportFormat::Markdown,
            ExportFormat::Json,
            ExportFormat::Obsidian,
            ExportFormat::Html,
        ] {
            assert_eq!(
                export(&[a(), b()], f, &opts(), now()),
                export(&[a(), b()], f, &opts(), now())
            );
        }
    }

    #[test]
    fn safe_color_rules() {
        assert_eq!(safe_color("#fff"), Some("#fff"));
        assert_eq!(safe_color("#FFEE00AA"), Some("#FFEE00AA"));
        assert_eq!(safe_color(" yellow "), Some("yellow"));
        assert_eq!(safe_color("#ggg"), None);
        assert_eq!(safe_color("#ff"), None);
        assert_eq!(safe_color("red;x"), None);
        assert_eq!(safe_color(""), None);
        assert_eq!(safe_color("rgb(1,2,3)"), None);
    }
}
