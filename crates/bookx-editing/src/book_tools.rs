//! Whole-book editing tools used by the editor UI (roadmap track T7/T9): spine reorder and non-linear marks, page roles,
//! replacing a file, smarten punctuation, remove unused CSS, unused media, beautify, and the accessibility tab helpers
//! (image descriptions, content language, accessibility metadata). Everything is text-level and keeps unrelated
//! markup byte-identical. No GUI.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Context, Result};
use bookx_convert::transforms::punctuation::{smarten_html_with, unsmarten_html, SmartenOptions};
use bookx_core::book::OpenBook;
use bookx_core::opf::SpineItem;
use bookx_core::util::{extension, file_name, parse_xml, xml_escape};

use crate::markup::*;
use crate::opf_edit::{append_to_section, edit_opf};
use crate::semantics::{set_semantic, Semantic};

// ---------------------------------------------------------------------------
// spine
// ---------------------------------------------------------------------------

/// Moves the element at `from` so that it ends up at index `to` (both clamped to the list). Returns whether
/// anything moved.
pub fn move_index<T>(v: &mut Vec<T>, from: usize, to: usize) -> bool {
    if v.is_empty() || from >= v.len() {
        return false;
    }
    let to = to.min(v.len() - 1);
    if from == to {
        return false;
    }
    let item = v.remove(from);
    v.insert(to, item);
    true
}

/// Where a dragged row lands: the index in the list *without* the dragged item for "drop before row `target`"
/// (`after` = false) or "after row `target`".
pub fn drop_index(from: usize, target: usize, after: bool) -> usize {
    let at = if after { target + 1 } else { target };
    if from < at {
        at - 1
    } else {
        at
    }
}

/// Puts the spine file `path` at position `to` of the reading order (0 based).
pub fn spine_move_to(book: &mut OpenBook, path: &str, to: usize) -> Result<bool> {
    let id = book.package.item_by_path(path).map(|i| i.id.clone());
    let mut order = book.package.spine.clone();
    let Some(from) = id.and_then(|id| order.iter().position(|s| s.idref == id)) else {
        bail!("Only files in the reading order can be moved");
    };
    if !move_index(&mut order, from, to) {
        return Ok(false);
    }
    write_spine(book, &order)?;
    Ok(true)
}

fn write_spine(book: &mut OpenBook, order: &[SpineItem]) -> Result<()> {
    let pkg = book.package.clone();
    pkg.set_spine(&mut book.archive, order)?;
    book.reload_package()
}

/// Marks a spine file as not part of the main reading order (`linear="no"`) or back. Returns whether it changed.
pub fn set_linear(book: &mut OpenBook, path: &str, linear: bool) -> Result<bool> {
    let id = book.package.item_by_path(path).map(|i| i.id.clone());
    let mut order = book.package.spine.clone();
    let mut changed = false;
    let mut found = false;
    for s in order.iter_mut().filter(|s| Some(&s.idref) == id.as_ref()) {
        found = true;
        if s.linear != linear {
            s.linear = linear;
            changed = true;
        }
    }
    if !found {
        bail!("Only files in the reading order can be marked");
    }
    if changed {
        write_spine(book, &order)?;
    }
    Ok(changed)
}

/// What the file browser says about a file besides its name (icons and tooltips, CE-022).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FileNote {
    Cover,
    TitlePage,
    Package,
    Ncx,
    Nav,
    NotInManifest,
    NonLinear,
    NotInSpine,
    TwiceInSpine,
}

impl FileNote {
    pub fn tooltip(self) -> &'static str {
        match self {
            FileNote::Cover => "Cover image",
            FileNote::TitlePage => "Title page",
            FileNote::Package => "Package document (metadata, file list and reading order)",
            FileNote::Ncx => "Table of contents (NCX)",
            FileNote::Nav => "Navigation document (table of contents)",
            FileNote::NotInManifest => "Not listed in the package: readers will not see this file",
            FileNote::NonLinear => "Not part of the main reading order",
            FileNote::NotInSpine => "A page that is not in the reading order",
            FileNote::TwiceInSpine => "Listed twice in the reading order",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            FileNote::Cover => "image-x-generic-symbolic",
            FileNote::TitlePage => "bookmark-new-symbolic",
            FileNote::Package => "document-properties-symbolic",
            FileNote::Ncx | FileNote::Nav => "view-list-symbolic",
            FileNote::NotInManifest => "dialog-warning-symbolic",
            FileNote::NonLinear => "go-jump-symbolic",
            FileNote::NotInSpine => "dialog-warning-symbolic",
            FileNote::TwiceInSpine => "dialog-warning-symbolic",
        }
    }
}

/// The notes for `path`, most important first.
pub fn file_notes(book: &OpenBook, path: &str) -> Vec<FileNote> {
    file_notes_with(book, path, &crate::semantics::read_semantics(book))
}

/// [`file_notes`] with the semantics read once by the caller (they parse the package and the navigation
/// document, which is slow for a book of many files: a list of 20 files read them 20 times).
pub fn file_notes_with(
    book: &OpenBook,
    path: &str,
    semantics: &[crate::semantics::SemanticEntry],
) -> Vec<FileNote> {
    let pkg = &book.package;
    let mut out = Vec::new();
    if path == pkg.opf_path {
        out.push(FileNote::Package);
    }
    let item = pkg.item_by_path(path);
    if item.is_none()
        && path != pkg.opf_path
        && !path.starts_with("META-INF/")
        && path != "mimetype"
    {
        out.push(FileNote::NotInManifest);
    }
    if pkg.cover_path().as_deref() == Some(path) {
        out.push(FileNote::Cover);
    }
    if pkg.ncx_item().map(|i| pkg.item_path(i)).as_deref() == Some(path) {
        out.push(FileNote::Ncx);
    }
    if pkg.nav_item().map(|i| pkg.item_path(i)).as_deref() == Some(path) {
        out.push(FileNote::Nav);
    }
    if semantics
        .iter()
        .any(|e| e.semantic == Semantic::TitlePage && e.href.split('#').next() == Some(path))
    {
        out.push(FileNote::TitlePage);
    }
    if let Some(item) = item {
        let uses: Vec<&SpineItem> = pkg.spine.iter().filter(|s| s.idref == item.id).collect();
        if item.media_type == "application/xhtml+xml" && uses.is_empty() {
            out.push(FileNote::NotInSpine);
        }
        if uses.len() > 1 {
            out.push(FileNote::TwiceInSpine);
        }
        if uses.iter().any(|s| !s.linear) {
            out.push(FileNote::NonLinear);
        }
    }
    out
}

/// A role a page of the book can be given (CE-032, CE-033).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageRole {
    Cover,
    TitlePage,
    Toc,
}

impl PageRole {
    pub fn semantic(self) -> Semantic {
        match self {
            PageRole::Cover => Semantic::Cover,
            PageRole::TitlePage => Semantic::TitlePage,
            PageRole::Toc => Semantic::Toc,
        }
    }
}

/// Gives the page `path` the role (guide entry and landmark). Returns a hint when the page cannot really play it.
pub fn mark_page(book: &mut OpenBook, path: &str, role: PageRole) -> Result<Option<String>> {
    if book.package.item_by_path(path).is_none() {
        bail!("{path} is not part of the book's file list");
    }
    set_semantic(book, role.semantic(), path, "")?;
    let hint = if role == PageRole::Toc {
        let text = book.archive.get_string(path).unwrap_or_default();
        (!text.contains("epub:type=\"toc\"") && !text.contains("epub:type='toc'")).then(|| {
            "This page has no navigation list marked as the table of contents (nav epub:type=\"toc\")".to_string()
        })
    } else {
        None
    };
    Ok(hint)
}

// ---------------------------------------------------------------------------
// replace a file
// ---------------------------------------------------------------------------

/// Replaces the content of `path` with `data`. When `new_name` differs from the old file name the file is renamed
/// first, so every link to it follows. Returns the final path.
pub fn replace_file(
    book: &mut OpenBook,
    path: &str,
    new_name: &str,
    data: Vec<u8>,
) -> Result<String> {
    if !book.archive.contains(path) {
        bail!("{path} is not in the book");
    }
    let mut target = path.to_string();
    if !new_name.is_empty() && new_name != file_name(path) {
        target = crate::files::renamed_in_place(path, new_name);
        let map = std::collections::HashMap::from([(path.to_string(), target.clone())]);
        crate::files::rename_files(book, &map)?;
    }
    book.archive.set(&target, data);
    Ok(target)
}

// ---------------------------------------------------------------------------
// punctuation, unused CSS
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Punctuation {
    Smarten(SmartenOptions),
    Unsmarten,
}

/// How double quotation marks look in a language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QuoteStyle {
    /// “like this”
    #[default]
    English,
    /// „wie hier“
    German,
    /// « comme ici »
    French,
    /// ”som här”
    Swedish,
}

impl QuoteStyle {
    pub const ALL: [QuoteStyle; 4] = [
        QuoteStyle::English,
        QuoteStyle::German,
        QuoteStyle::French,
        QuoteStyle::Swedish,
    ];

    pub fn label(self) -> &'static str {
        match self {
            QuoteStyle::English => "English “double”",
            QuoteStyle::German => "German „double“",
            QuoteStyle::French => "French « double »",
            QuoteStyle::Swedish => "Swedish ”double”",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            QuoteStyle::English => "en",
            QuoteStyle::German => "de",
            QuoteStyle::French => "fr",
            QuoteStyle::Swedish => "sv",
        }
    }

    pub fn from_id(id: &str) -> Option<QuoteStyle> {
        Self::ALL.into_iter().find(|s| s.id() == id)
    }

    /// The style for a language code (`de`, `fr-CA`, ...).
    pub fn for_language(lang: &str) -> QuoteStyle {
        match lang
            .split(['-', '_'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "de" | "cs" | "sk" | "sl" | "hr" => QuoteStyle::German,
            "fr" | "es" | "it" | "pt" | "ru" | "uk" | "el" => QuoteStyle::French,
            "sv" | "fi" => QuoteStyle::Swedish,
            _ => QuoteStyle::English,
        }
    }

    fn marks(self) -> Option<(&'static str, &'static str)> {
        match self {
            QuoteStyle::English => None,
            QuoteStyle::German => Some(("\u{201e}", "\u{201c}")),
            QuoteStyle::French => Some(("\u{ab}\u{a0}", "\u{a0}\u{bb}")),
            QuoteStyle::Swedish => Some(("\u{201d}", "\u{201d}")),
        }
    }
}

/// Rewrites the curly double quotes of the text (not of tags, scripts or styles) in the style of a language.
pub fn apply_quote_style(src: &str, style: QuoteStyle) -> String {
    let Some((open, close)) = style.marks() else {
        return src.to_string();
    };
    let mut out = String::with_capacity(src.len());
    let mut at = 0;
    for r in text_runs(src, DEFAULT_SKIP) {
        out.push_str(&src[at..r.start]);
        for c in src[r.clone()].chars() {
            match c {
                '\u{201c}' => out.push_str(open),
                '\u{201d}' => out.push_str(close),
                c => out.push(c),
            }
        }
        at = r.end;
    }
    out.push_str(&src[at..]);
    out
}

/// Smartens (or straightens) the text of the given pages (every page when `paths` is empty). Returns the changed
/// files.
pub fn punctuation(book: &mut OpenBook, paths: &[String], mode: Punctuation) -> Vec<String> {
    punctuation_styled(book, paths, mode, QuoteStyle::English)
}

/// [`punctuation`] with the double quotes in the style of a language.
pub fn punctuation_styled(
    book: &mut OpenBook,
    paths: &[String],
    mode: Punctuation,
    style: QuoteStyle,
) -> Vec<String> {
    let mut changed = Vec::new();
    for f in markup_files(&book.archive) {
        if !matches!(extension(&f).as_str(), "xhtml" | "xht" | "html" | "htm")
            || (!paths.is_empty() && !paths.contains(&f))
        {
            continue;
        }
        let Some(src) = book.archive.get_string(&f) else {
            continue;
        };
        let new = match mode {
            Punctuation::Smarten(o) => {
                let s = smarten_html_with(&src, o);
                if o.quotes {
                    apply_quote_style(&s, style)
                } else {
                    s
                }
            }
            Punctuation::Unsmarten => unsmarten_html(&src),
        };
        if new != src && parse_xml(&new).is_ok() {
            book.archive.set_string(&f, &new);
            changed.push(f);
        }
    }
    changed
}

/// Drops the CSS rules no page uses. Returns `(file, bytes before, bytes after)` for the sheets that shrank.
pub fn remove_unused_css(book: &mut OpenBook) -> Vec<(String, usize, usize)> {
    use bookx_convert::transforms::css::{css_paths, parse_documents, prune_css, Usage};
    let mut usage = Usage::new(parse_documents(book));
    let mut out = Vec::new();
    for p in css_paths(book) {
        let Some(css) = book.archive.get_string(&p) else {
            continue;
        };
        let new = prune_css(&css, &mut usage);
        if new != css {
            out.push((p.clone(), css.len(), new.len()));
            book.archive.set_string(&p, &new);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// unused media
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MediaKind {
    Image,
    Font,
    Audio,
    Video,
}

impl MediaKind {
    pub fn label(self) -> &'static str {
        match self {
            MediaKind::Image => "Image",
            MediaKind::Font => "Font",
            MediaKind::Audio => "Audio",
            MediaKind::Video => "Video",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnusedMedia {
    pub path: String,
    pub kind: MediaKind,
    pub bytes: usize,
}

fn media_kind(media_type: &str, path: &str) -> Option<MediaKind> {
    let mt = media_type.to_ascii_lowercase();
    if mt.starts_with("image/") {
        Some(MediaKind::Image)
    } else if mt.starts_with("audio/") {
        Some(MediaKind::Audio)
    } else if mt.starts_with("video/") {
        Some(MediaKind::Video)
    } else if mt.contains("font") || crate::fonts::is_font_path(path) {
        Some(MediaKind::Font)
    } else {
        None
    }
}

/// Images, fonts, audio and video that no page, stylesheet, SVG or navigation file mentions, other than the cover.
/// A file counts as used when its name appears in any of those texts, so the answer errs on keeping files.
pub fn unused_media(book: &OpenBook) -> Vec<UnusedMedia> {
    let pkg = &book.package;
    let nav = pkg.nav_item().map(|i| pkg.item_path(i));
    let texts: Vec<String> = book
        .archive
        .names()
        .iter()
        .filter(|n| **n != pkg.opf_path || nav.as_deref() == Some(n.as_str()))
        .filter(|n| {
            matches!(
                extension(n).as_str(),
                "xhtml" | "html" | "htm" | "xht" | "css" | "svg" | "ncx" | "smil" | "js"
            )
        })
        .filter_map(|n| book.archive.get_string(n))
        .collect();
    let cover = pkg.cover_path();
    let mut out = Vec::new();
    for item in &pkg.manifest {
        let path = pkg.item_path(item);
        let Some(kind) = media_kind(&item.media_type, &path) else {
            continue;
        };
        if cover.as_deref() == Some(path.as_str()) || item.properties.contains("cover-image") {
            continue;
        }
        let name = file_name(&path);
        if texts.iter().any(|t| t.contains(name)) {
            continue;
        }
        out.push(UnusedMedia {
            bytes: book.archive.size_of(&path),
            path,
            kind,
        });
    }
    out.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.path.cmp(&b.path)));
    out
}

/// Deletes the given media files from the book (manifest entries included). Returns the removed paths.
pub fn remove_media(book: &mut OpenBook, paths: &[String]) -> Result<Vec<String>> {
    crate::files::delete_files(book, paths)
}

// ---------------------------------------------------------------------------
// beautify
// ---------------------------------------------------------------------------

const BLOCK: &[&str] = &[
    "html",
    "head",
    "body",
    "title",
    "meta",
    "link",
    "style",
    "script",
    "div",
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "li",
    "dl",
    "dt",
    "dd",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "td",
    "th",
    "caption",
    "blockquote",
    "section",
    "article",
    "aside",
    "nav",
    "header",
    "footer",
    "main",
    "figure",
    "figcaption",
    "hr",
    "pre",
    "form",
    "fieldset",
    "address",
    "details",
    "summary",
];

fn is_block_name(n: &str) -> bool {
    BLOCK.contains(&n)
}

const VERBATIM: &[&str] = &["pre", "script", "style", "textarea"];

/// Tidies the white space of an XHTML page: between two block-level tags (or comments, the doctype) that only
/// have white space between them, the gap becomes a line break plus indentation by block depth. White space inside
/// text, around inline tags and in `pre`/`script`/`style` is never touched, so the page looks the same. A page that
/// does not parse is returned as it was.
pub fn beautify_markup(src: &str) -> String {
    if parse_xml(src).is_err() {
        return src.to_string();
    }
    #[derive(Clone, Copy, PartialEq)]
    enum Edge {
        Open,
        Close,
        Other,
        Inline,
    }
    let structural = |e: Edge| e != Edge::Inline;
    let mut stack: Vec<String> = Vec::new();
    let mut verbatim = 0usize;
    // the last tag seen, where it ended, and the white space text after it (if that is all there was)
    let mut prev: Option<(Edge, usize)> = None;
    let mut prev_pushed = false;
    let mut gap: Option<std::ops::Range<usize>> = None;
    let mut edits: Vec<(std::ops::Range<usize>, usize)> = Vec::new();
    for ev in scan(src) {
        let prev_open_pushed = std::mem::replace(&mut prev_pushed, false);
        let (edge, range, depth) = match ev {
            Event::Text(r) => {
                prev_pushed = prev_open_pushed;
                gap = match (&gap, prev) {
                    (None, Some((_, end)))
                        if end == r.start && src[r.clone()].trim().is_empty() =>
                    {
                        Some(r)
                    }
                    _ => {
                        // real text: no tidying around it
                        prev = prev.map(|(_, end)| (Edge::Inline, end));
                        None
                    }
                };
                continue;
            }
            Event::Open {
                name,
                range,
                self_closing,
            } => {
                let edge = if is_block_name(&name) {
                    Edge::Open
                } else {
                    Edge::Inline
                };
                let depth = stack.len();
                let verbatim_before = verbatim;
                prev_pushed = !self_closing;
                if !self_closing {
                    stack.push(name.clone());
                    if VERBATIM.contains(&name.as_str()) {
                        verbatim += 1;
                    }
                }
                if verbatim_before > 0 {
                    prev = Some((Edge::Inline, range.end));
                    gap = None;
                    continue;
                }
                (edge, range, depth)
            }
            Event::Close { name, range } => {
                let verbatim_before = verbatim;
                if let Some(pos) = stack.iter().rposition(|n| *n == name) {
                    stack.truncate(pos);
                }
                if VERBATIM.contains(&name.as_str()) {
                    verbatim = verbatim.saturating_sub(1);
                }
                if verbatim_before > 0 {
                    prev = Some((Edge::Inline, range.end));
                    gap = None;
                    continue;
                }
                let edge = if is_block_name(&name) {
                    Edge::Close
                } else {
                    Edge::Inline
                };
                (edge, range, stack.len())
            }
            Event::Other(r) => {
                let d = stack.len();
                (
                    if verbatim > 0 {
                        Edge::Inline
                    } else {
                        Edge::Other
                    },
                    r,
                    d,
                )
            }
        };
        if let Some((pe, pend)) = prev {
            let g = gap.take().unwrap_or(pend..range.start);
            let empty_element = pe == Edge::Open && prev_open_pushed && edge == Edge::Close;
            if structural(pe)
                && structural(edge)
                && !empty_element
                && g.start <= g.end
                && g.end == range.start
                && src[g.clone()].trim().is_empty()
            {
                edits.push((g, depth));
            }
        }
        gap = None;
        prev = Some((edge, range.end));
    }
    let mut out = String::with_capacity(src.len() + src.len() / 8);
    let mut at = 0;
    for (r, depth) in edits {
        out.push_str(&src[at..r.start]);
        out.push('\n');
        for _ in 0..depth {
            out.push_str("  ");
        }
        at = r.end;
    }
    out.push_str(&src[at..]);
    if parse_xml(&out).is_ok() {
        out
    } else {
        src.to_string()
    }
}

/// Lays out a stylesheet one declaration per line, rules separated by a blank line. Comments stay where they are.
pub fn beautify_css(css: &str) -> String {
    use bookx_convert::transforms::css::{parse_css, CssItem};
    fn block(inner: &str, indent: &str) -> String {
        let mut o = String::new();
        for seg in crate::transform_css::split_declarations(inner) {
            let t = seg.trim();
            if t.is_empty() {
                continue;
            }
            let body = t.strip_suffix(';').unwrap_or(t);
            match body.split_once(':') {
                Some((n, v)) if !n.trim_start().starts_with("/*") => {
                    o.push_str(&format!("{indent}  {}: {};\n", n.trim(), v.trim()));
                }
                _ => o.push_str(&format!("{indent}  {t}\n")),
            }
        }
        o
    }
    fn items(css: &str, indent: &str, out: &mut String) {
        for item in parse_css(css) {
            match item {
                CssItem::Rule {
                    selectors,
                    block: b,
                } => {
                    let sel = selectors.split_whitespace().collect::<Vec<_>>().join(" ");
                    let inner = b
                        .trim()
                        .strip_prefix('{')
                        .and_then(|x| x.strip_suffix('}'))
                        .unwrap_or("");
                    out.push_str(&format!(
                        "{indent}{sel} {{\n{}{indent}}}\n\n",
                        block(inner, indent)
                    ));
                }
                CssItem::At(a) => {
                    let a = a.trim();
                    let lower = a.to_ascii_lowercase();
                    match (a.find('{'), a.rfind('}')) {
                        (Some(o), Some(c)) if c > o => {
                            let head = a[..o].split_whitespace().collect::<Vec<_>>().join(" ");
                            let inner = &a[o + 1..c];
                            if lower.starts_with("@media") || lower.starts_with("@supports") {
                                let mut nested = String::new();
                                items(inner, &format!("{indent}  "), &mut nested);
                                out.push_str(&format!(
                                    "{indent}{head} {{\n{}{indent}}}\n\n",
                                    nested.trim_end_matches('\n').to_string() + "\n"
                                ));
                            } else {
                                out.push_str(&format!(
                                    "{indent}{head} {{\n{}{indent}}}\n\n",
                                    block(inner, indent)
                                ));
                            }
                        }
                        _ => out.push_str(&format!("{indent}{a}\n")),
                    }
                }
                CssItem::Other(o) => {
                    let t = o.trim();
                    if !t.is_empty() {
                        out.push_str(&format!("{indent}{t}\n\n"));
                    }
                }
            }
        }
    }
    let mut out = String::new();
    items(css, "", &mut out);
    format!("{}\n", out.trim_end())
}

/// Beautifies `paths`: pages with [`beautify_markup`], stylesheets with [`beautify_css`], package and navigation XML
/// with the XML pretty printer. Returns the changed files.
pub fn beautify_files(book: &mut OpenBook, paths: &[String]) -> Vec<String> {
    let mut changed = Vec::new();
    for p in paths {
        let Some(src) = book.archive.get_string(p) else {
            continue;
        };
        let new = match extension(p).as_str() {
            "xhtml" | "xht" | "html" | "htm" => beautify_markup(&src),
            "css" => beautify_css(&src),
            "opf" | "ncx" | "xml" | "smil" => {
                bookx_core::format::pretty_print_xml(&src).unwrap_or_else(|_| src.clone())
            }
            _ => continue,
        };
        if new != src {
            book.archive.set_string(p, &new);
            changed.push(p.clone());
        }
    }
    if changed.contains(&book.package.opf_path) {
        let _ = book.reload_package();
    }
    changed
}

/// Up to `max` changed lines as `(before, after)`, shortened for display.
pub fn diff_samples(old: &str, new: &str, max: usize) -> Vec<(String, String)> {
    let short = |s: &str| {
        let t = s.trim();
        if t.chars().count() > 110 {
            format!("{}…", t.chars().take(110).collect::<String>())
        } else {
            t.to_string()
        }
    };
    let d = similar::TextDiff::from_lines(old, new);
    let mut out = Vec::new();
    for op in d.ops() {
        if let similar::DiffOp::Replace {
            old_index,
            old_len,
            new_index,
            new_len,
        } = *op
        {
            for k in 0..old_len.min(new_len) {
                let a = d.old_slices()[old_index + k];
                let b = d.new_slices()[new_index + k];
                out.push((short(a), short(b)));
                if out.len() >= max {
                    return out;
                }
            }
        }
    }
    out
}

/// Lines added and removed between two texts.
pub fn line_diff_counts(old: &str, new: &str) -> (usize, usize) {
    let d = similar::TextDiff::from_lines(old, new);
    let (mut add, mut del) = (0, 0);
    for c in d.iter_all_changes() {
        match c.tag() {
            similar::ChangeTag::Insert => add += 1,
            similar::ChangeTag::Delete => del += 1,
            similar::ChangeTag::Equal => {}
        }
    }
    (add, del)
}

// ---------------------------------------------------------------------------
// accessibility: image descriptions, language, metadata
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageUse {
    pub file: String,
    /// Index of the `<img>` among the images of the file.
    pub index: usize,
    pub src: String,
    /// `None` = no `alt` attribute at all, `Some("")` = decorative.
    pub alt: Option<String>,
}

fn img_tags(src: &str) -> Vec<std::ops::Range<usize>> {
    scan(src)
        .into_iter()
        .filter_map(|e| match e {
            Event::Open { name, range, .. } if name == "img" => Some(range),
            _ => None,
        })
        .collect()
}

/// Every `<img>` of the book in reading order with its description.
pub fn image_uses(book: &OpenBook) -> Vec<ImageUse> {
    let mut files = book.spine_paths();
    for f in markup_files(&book.archive) {
        if !files.contains(&f) && matches!(extension(&f).as_str(), "xhtml" | "xht" | "html" | "htm")
        {
            files.push(f);
        }
    }
    let mut out = Vec::new();
    for f in files {
        let Some(src) = book.archive.get_string(&f) else {
            continue;
        };
        for (index, r) in img_tags(&src).into_iter().enumerate() {
            let tag = &src[r];
            out.push(ImageUse {
                file: f.clone(),
                index,
                src: decode_entities(&tag_attr(tag, "src").unwrap_or_default()),
                alt: tag_attr(tag, "alt").map(|a| decode_entities(&a)),
            });
        }
    }
    out
}

/// Sets the description of one image; an empty text marks it decorative (`alt=""`).
pub fn set_alt_text(book: &mut OpenBook, file: &str, index: usize, alt: &str) -> Result<bool> {
    let src = book
        .archive
        .get_string(file)
        .with_context(|| format!("{file} is not in the book"))?;
    let tags = img_tags(&src);
    let r = tags
        .get(index)
        .with_context(|| format!("{file} has no image number {}", index + 1))?
        .clone();
    let new_tag = set_tag_attr(&src[r.clone()], "alt", &xml_escape(alt));
    if new_tag == src[r.clone()] {
        return Ok(false);
    }
    let new = format!("{}{}{}", &src[..r.start], new_tag, &src[r.end..]);
    book.archive.set_string(file, &new);
    Ok(true)
}

/// Gives every image that uses the same file the same description. Returns how many images changed.
pub fn set_alt_for_source(book: &mut OpenBook, target: &ImageUse, alt: &str) -> Result<usize> {
    let abs = |u: &ImageUse| {
        bookx_core::util::resolve_href(bookx_core::util::parent_dir(&u.file), &u.src).0
    };
    let want = abs(target);
    let mut n = 0;
    for u in image_uses(book) {
        if abs(&u) == want && set_alt_text(book, &u.file, u.index, alt)? {
            n += 1;
        }
    }
    Ok(n)
}

/// Sets the language of the book: `dc:language` and `lang`/`xml:lang` on every page. Returns the pages changed.
pub fn set_content_language(book: &mut OpenBook, lang: &str) -> Result<usize> {
    let lang = lang.trim();
    if lang.is_empty() {
        bail!("Give a language code such as en or fr-CA");
    }
    let mut md = book.metadata().clone();
    if md.language != lang {
        md.language = lang.to_string();
        book.set_metadata(md)?;
    }
    let mut n = 0;
    for f in markup_files(&book.archive) {
        if !matches!(extension(&f).as_str(), "xhtml" | "xht" | "html" | "htm") {
            continue;
        }
        let Some(src) = book.archive.get_string(&f) else {
            continue;
        };
        let root = scan(&src).into_iter().find_map(|e| match e {
            Event::Open { name, range, .. } if name == "html" => Some(range),
            _ => None,
        });
        let Some(r) = root else { continue };
        let mut tag = set_tag_attr(&src[r.clone()], "xml:lang", lang);
        tag = set_tag_attr(&tag, "lang", lang);
        if tag != src[r.clone()] {
            book.archive
                .set_string(&f, &format!("{}{}{}", &src[..r.start], tag, &src[r.end..]));
            n += 1;
        }
    }
    Ok(n)
}

/// The schema.org accessibility metadata an EPUB can carry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccessMeta {
    pub summary: String,
    pub access_modes: BTreeSet<String>,
    pub sufficient_modes: Vec<String>,
    pub features: BTreeSet<String>,
    pub hazards: BTreeSet<String>,
    pub conformance: String,
}

pub const ACCESS_MODES: &[&str] = &["textual", "visual", "auditory", "tactile"];
pub const ACCESS_FEATURES: &[&str] = &[
    "alternativeText",
    "longDescription",
    "tableOfContents",
    "readingOrder",
    "structuralNavigation",
    "displayTransformability",
    "ARIA",
    "pageBreakMarkers",
    "printPageNumbers",
    "index",
    "MathML",
    "none",
];
pub const ACCESS_HAZARDS: &[&str] = &[
    "none",
    "flashing",
    "noFlashingHazard",
    "motionSimulation",
    "noMotionSimulationHazard",
    "sound",
    "noSoundHazard",
    "unknown",
];

const ACCESS_PROPS: &[&str] = &[
    "schema:accessibilitySummary",
    "schema:accessMode",
    "schema:accessModeSufficient",
    "schema:accessibilityFeature",
    "schema:accessibilityHazard",
    "dcterms:conformsTo",
];

/// Reads the accessibility metadata from the package document.
pub fn read_access_meta(book: &OpenBook) -> AccessMeta {
    let mut m = AccessMeta::default();
    let Some(text) = book.archive.get_string(&book.package.opf_path) else {
        return m;
    };
    let Ok(doc) = parse_xml(&text) else { return m };
    for n in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "meta")
    {
        let Some(prop) = n.attribute("property") else {
            continue;
        };
        let v = n.text().unwrap_or("").trim().to_string();
        match prop {
            "schema:accessibilitySummary" => m.summary = v,
            "schema:accessMode" => {
                m.access_modes.insert(v);
            }
            "schema:accessModeSufficient" => m.sufficient_modes.push(v),
            "schema:accessibilityFeature" => {
                m.features.insert(v);
            }
            "schema:accessibilityHazard" => {
                m.hazards.insert(v);
            }
            "dcterms:conformsTo" => m.conformance = v,
            _ => {}
        }
    }
    m
}

/// Replaces the accessibility metadata of the package with `m` (empty fields are not written). EPUB 3 only.
pub fn write_access_meta(book: &mut OpenBook, m: &AccessMeta) -> Result<bool> {
    crate::opf_edit::require_epub3(book)?;
    let mut lines: Vec<String> = Vec::new();
    let mut add = |prop: &str, v: &str| {
        if !v.trim().is_empty() {
            lines.push(format!(
                "<meta property=\"{prop}\">{}</meta>",
                xml_escape(v.trim())
            ));
        }
    };
    add("schema:accessibilitySummary", &m.summary);
    for v in &m.access_modes {
        add("schema:accessMode", v);
    }
    for v in &m.sufficient_modes {
        add("schema:accessModeSufficient", v);
    }
    for v in &m.features {
        add("schema:accessibilityFeature", v);
    }
    for v in &m.hazards {
        add("schema:accessibilityHazard", v);
    }
    add("dcterms:conformsTo", &m.conformance);
    edit_opf(book, |text| {
        let mut t = text.to_string();
        for p in ACCESS_PROPS {
            let re = regex::Regex::new(&format!(
                r#"[ \t]*<meta\s[^>]*property\s*=\s*["']{}["'][^>]*?(?:/>|>[^<]*</meta>)[ \t]*\r?\n?"#,
                regex::escape(p)
            ))?;
            t = re.replace_all(&t, "").into_owned();
        }
        for l in &lines {
            t = append_to_section(&t, "metadata", l)?;
        }
        Ok(t)
    })
}

/// Counts of files per folder level used by tests and the report: files by extension.
pub fn extension_counts(book: &OpenBook) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for n in book.archive.names() {
        *m.entry(extension(n)).or_insert(0) += 1;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::make_book;

    #[test]
    fn move_index_and_drop_index() {
        let mut v = vec!['a', 'b', 'c', 'd'];
        assert!(move_index(&mut v, 0, 2));
        assert_eq!(v, vec!['b', 'c', 'a', 'd']);
        assert!(move_index(&mut v, 3, 0));
        assert_eq!(v, vec!['d', 'b', 'c', 'a']);
        assert!(!move_index(&mut v, 1, 1));
        assert!(!move_index(&mut v, 9, 0));
        assert!(move_index(&mut v, 0, 99));
        assert_eq!(v.last(), Some(&'d'));
        // dragging row 0 onto the lower half of row 2 puts it after row 2
        assert_eq!(drop_index(0, 2, true), 2);
        assert_eq!(drop_index(0, 2, false), 1);
        assert_eq!(drop_index(3, 1, false), 1);
        assert_eq!(drop_index(3, 1, true), 2);
        assert_eq!(drop_index(2, 2, false), 2);
    }

    #[test]
    fn spine_reorder_and_linear() {
        let mut b = make_book(&["<p>1</p>", "<p>2</p>", "<p>3</p>"]);
        let s = b.spine_paths();
        assert!(spine_move_to(&mut b, &s[0], 2).unwrap());
        assert_eq!(
            b.spine_paths(),
            vec![s[1].clone(), s[2].clone(), s[0].clone()]
        );
        assert!(!spine_move_to(&mut b, &s[0], 2).unwrap());
        assert!(spine_move_to(&mut b, "OEBPS/style.css", 0).is_err());
        assert!(set_linear(&mut b, &s[1], false).unwrap());
        assert!(!set_linear(&mut b, &s[1], false).unwrap());
        assert!(file_notes(&b, &s[1]).contains(&FileNote::NonLinear));
        assert!(set_linear(&mut b, &s[1], true).unwrap());
        assert!(!file_notes(&b, &s[1]).contains(&FileNote::NonLinear));
    }

    #[test]
    fn notes_for_special_files() {
        let mut b = make_book(&["<p>1</p>"]);
        let opf = b.package.opf_path.clone();
        assert!(file_notes(&b, &opf).contains(&FileNote::Package));
        b.archive.set_string("OEBPS/stray.xhtml", "<html/>");
        assert!(file_notes(&b, "OEBPS/stray.xhtml").contains(&FileNote::NotInManifest));
        if let Some(n) = b.package.nav_item().map(|i| b.package.item_path(i)) {
            assert!(file_notes(&b, &n).contains(&FileNote::Nav));
        }
        let first = b.spine_paths()[0].clone();
        assert_eq!(file_notes(&b, &first), vec![]);
        mark_page(&mut b, &first, PageRole::TitlePage).unwrap();
        assert!(file_notes(&b, &first).contains(&FileNote::TitlePage));
    }

    #[test]
    fn notes_with_given_semantics_equal_notes_that_read_them() {
        let mut b = make_book(&["<p>1</p>", "<p>2</p>", "<p>3</p>"]);
        let first = b.spine_paths()[0].clone();
        mark_page(&mut b, &first, PageRole::TitlePage).unwrap();
        let sem = crate::semantics::read_semantics(&b);
        assert!(!sem.is_empty());
        for f in b.file_list() {
            assert_eq!(file_notes_with(&b, &f, &sem), file_notes(&b, &f), "{f}");
        }
        assert!(file_notes_with(&b, &first, &sem).contains(&FileNote::TitlePage));
        // without the semantics the title page note is not there: the caller's list is what counts
        assert!(!file_notes_with(&b, &first, &[]).contains(&FileNote::TitlePage));
    }

    #[test]
    fn mark_toc_warns_when_no_nav_list() {
        let mut b = make_book(&["<p>1</p>"]);
        let first = b.spine_paths()[0].clone();
        let hint = mark_page(&mut b, &first, PageRole::Toc).unwrap();
        assert!(hint.is_some());
        assert!(mark_page(&mut b, "nope.xhtml", PageRole::Toc).is_err());
    }

    #[test]
    fn replace_with_new_name_updates_links() {
        let mut b = make_book(&["<p>1</p>", "<p>see <a href=\"chapter-1.xhtml\">one</a></p>"]);
        let s = b.spine_paths();
        let two = s[1].clone();
        let one = s[0].clone();
        let name = file_name(&one).to_string();
        let ref_text = b.archive.get_string(&two).unwrap();
        if !ref_text.contains(&name) {
            b.archive
                .set_string(&two, &ref_text.replace("chapter-1.xhtml", &name));
        }
        let new = replace_file(
            &mut b,
            &one,
            "intro.xhtml",
            b"<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>NEW</p></body></html>".to_vec(),
        )
        .unwrap();
        assert!(new.ends_with("intro.xhtml"));
        assert!(!b.archive.contains(&one));
        assert!(b.archive.get_string(&new).unwrap().contains("NEW"));
        assert!(b.archive.get_string(&two).unwrap().contains("intro.xhtml"));
        // same name: content only
        let same = replace_file(&mut b, &new, "intro.xhtml", b"x".to_vec()).unwrap();
        assert_eq!(same, new);
        assert!(replace_file(&mut b, "missing", "a", vec![]).is_err());
    }

    #[test]
    fn punctuation_scope_and_modes() {
        let mut b = make_book(&["<p>\"Hi\" -- ok...</p>", "<p>\"Hi\"</p>"]);
        let s = b.spine_paths();
        let ch = punctuation(
            &mut b,
            &[s[0].clone()],
            Punctuation::Smarten(SmartenOptions::default()),
        );
        assert_eq!(ch, vec![s[0].clone()]);
        let t = b.archive.get_string(&s[0]).unwrap();
        assert!(t.contains("“Hi” – ok…"), "{t}");
        assert!(b.archive.get_string(&s[1]).unwrap().contains("\"Hi\""));
        assert!(
            punctuation(&mut b, &[], Punctuation::Smarten(SmartenOptions::default())).len() <= 1
        );
        let back = punctuation(&mut b, &[], Punctuation::Unsmarten);
        assert!(!back.is_empty());
        assert!(b
            .archive
            .get_string(&s[0])
            .unwrap()
            .contains("\"Hi\" -- ok..."));
    }

    #[test]
    fn quote_styles_by_language() {
        let src = "<p title=\"“keep”\">“Hi,” she said.</p><style>a:before{content:\"“\"}</style>";
        assert_eq!(apply_quote_style(src, QuoteStyle::English), src);
        let de = apply_quote_style(src, QuoteStyle::German);
        assert!(de.contains("„Hi,“ she said."), "{de}");
        assert!(de.contains("title=\"“keep”\""), "attributes are left alone");
        assert!(de.contains("content:\"“\""), "styles are left alone");
        let fr = apply_quote_style(src, QuoteStyle::French);
        assert!(fr.contains("«\u{a0}Hi,\u{a0}» she said."), "{fr}");
        assert_eq!(QuoteStyle::for_language("de-AT"), QuoteStyle::German);
        assert_eq!(QuoteStyle::for_language("en-GB"), QuoteStyle::English);
        assert_eq!(QuoteStyle::for_language("sv"), QuoteStyle::Swedish);
        assert_eq!(QuoteStyle::from_id("fr"), Some(QuoteStyle::French));
        let mut b = make_book(&["<p>\"Hallo\" sagte er.</p>"]);
        let o = SmartenOptions::default();
        punctuation_styled(&mut b, &[], Punctuation::Smarten(o), QuoteStyle::German);
        let t = b.archive.get_string(&b.spine_paths()[0]).unwrap();
        assert!(t.contains("„Hallo“"), "{t}");
    }

    #[test]
    fn diff_samples_pair_changed_lines() {
        let s = diff_samples("a\n\"x\" y\nc\n", "a\n“x” y\nc\n", 3);
        assert_eq!(s, vec![("\"x\" y".to_string(), "“x” y".to_string())]);
        assert!(diff_samples("a\n", "a\n", 3).is_empty());
        assert_eq!(diff_samples("1\n2\n3\n", "a\nb\nc\n", 2).len(), 2);
    }

    #[test]
    fn unused_css_is_removed_and_reported() {
        let mut b = make_book(&["<p class=\"used\">x</p>"]);
        let css = css_path(&b);
        b.archive
            .set_string(&css, ".used { color: red }\n.unused { color: blue }\n");
        let r = remove_unused_css(&mut b);
        assert_eq!(r.len(), 1);
        assert!(r[0].2 < r[0].1);
        let t = b.archive.get_string(&css).unwrap();
        assert!(t.contains(".used") && !t.contains(".unused"));
        assert!(remove_unused_css(&mut b).is_empty());
    }

    fn css_path(b: &OpenBook) -> String {
        css_files(&b.archive).into_iter().next().expect("css")
    }

    #[test]
    fn unused_media_lists_only_unreferenced_files() {
        let mut b = make_book(&["<p><img src=\"used.png\" alt=\"\"/></p>"]);
        let base = b.package.base_dir().to_string();
        for n in ["used.png", "orphan.png", "orphan.ttf"] {
            crate::files::add_file(&mut b, n, crate::testutil::png(4, 4), None).unwrap();
        }
        let _ = base;
        let un = unused_media(&b);
        let names: Vec<&str> = un.iter().map(|u| file_name(&u.path)).collect();
        assert!(names.contains(&"orphan.png"), "{names:?}");
        assert!(!names.contains(&"used.png"));
        assert!(un.iter().all(|u| u.bytes > 0));
        let paths: Vec<String> = un.iter().map(|u| u.path.clone()).collect();
        let gone = remove_media(&mut b, &paths).unwrap();
        assert_eq!(gone.len(), paths.len());
        assert!(unused_media(&b).is_empty());
    }

    #[test]
    fn beautify_markup_is_safe_and_idempotent() {
        let src = "<?xml version=\"1.0\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>T</title></head><body><div><p>a <b>b</b>  <i>c</i></p><p>two</p></div>\n\n\n<pre>  keep\n   this </pre></body></html>";
        let out = beautify_markup(src);
        assert!(out.contains("<body>\n    <div>\n      <p>a <b>b</b>  <i>c</i></p>\n      <p>two</p>\n    </div>"), "{out}");
        assert!(out.contains("<pre>  keep\n   this </pre>"));
        assert!(out.contains("<b>b</b>  <i>c</i>"));
        assert_eq!(beautify_markup(&out), out);
        // text content is the same
        assert_eq!(
            plain_text(src).split_whitespace().collect::<Vec<_>>(),
            plain_text(&out).split_whitespace().collect::<Vec<_>>()
        );
        // broken input is left alone
        assert_eq!(beautify_markup("<p>x"), "<p>x");
        // a self-closing tag before a closing tag is not an empty element
        assert_eq!(
            beautify_markup("<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p> </p></body></html>"),
            "<html xmlns=\"http://www.w3.org/1999/xhtml\">\n  <body>\n    <p> </p>\n  </body>\n</html>"
        );
        let head = "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><meta charset=\"utf-8\"/></head><body><p></p></body></html>";
        assert_eq!(
            beautify_markup(head),
            "<html xmlns=\"http://www.w3.org/1999/xhtml\">\n  <head>\n    <meta charset=\"utf-8\"/>\n  </head>\n  <body>\n    <p></p>\n  </body>\n</html>"
        );
    }

    #[test]
    fn beautify_css_layout() {
        let out = beautify_css("p{color:red;margin:0}\n\n\n@media print{ p{display:none} }\n@font-face{font-family:X;src:url(a.ttf)}");
        assert_eq!(
            out,
            "p {\n  color: red;\n  margin: 0;\n}\n\n@media print {\n  p {\n    display: none;\n  }\n}\n\n@font-face {\n  font-family: X;\n  src: url(a.ttf);\n}\n"
        );
        assert_eq!(beautify_css(&out), out);
    }

    #[test]
    fn beautify_files_by_kind_and_diff_counts() {
        let mut b = make_book(&["<p>x</p>"]);
        let css = css_path(&b);
        b.archive.set_string(&css, "p{color:red}");
        let page = b.spine_paths()[0].clone();
        let ch = beautify_files(&mut b, &[css.clone(), page.clone(), "nope".into()]);
        assert!(ch.contains(&css));
        let (a, d) = line_diff_counts("a\nb\n", "a\nc\nd\n");
        assert_eq!((a, d), (2, 1));
    }

    #[test]
    fn alt_text_editing() {
        let mut b = make_book(&[
            "<p><img src=\"a.png\"/><img src=\"b.png\" alt=\"old\"/></p>",
            "<p><img src=\"../OEBPS/a.png\"/></p>",
        ]);
        let imgs = image_uses(&b);
        assert!(imgs.len() >= 3);
        assert_eq!(imgs[0].alt, None);
        assert_eq!(imgs[1].alt.as_deref(), Some("old"));
        assert!(set_alt_text(&mut b, &imgs[0].file, 0, "A picture & more").unwrap());
        assert!(!set_alt_text(&mut b, &imgs[0].file, 0, "A picture & more").unwrap());
        assert_eq!(image_uses(&b)[0].alt.as_deref(), Some("A picture & more"));
        assert!(set_alt_text(&mut b, &imgs[0].file, 9, "x").is_err());
        let first = image_uses(&b)[0].clone();
        let n = set_alt_for_source(&mut b, &first, "same").unwrap();
        assert_eq!(n, 2);
        assert!(set_alt_text(&mut b, &imgs[0].file, 0, "").unwrap());
        assert_eq!(image_uses(&b)[0].alt.as_deref(), Some(""));
        for f in b.spine_paths() {
            assert!(parse_xml(&b.archive.get_string(&f).unwrap()).is_ok());
        }
    }

    #[test]
    fn language_and_access_metadata() {
        let mut b = make_book(&["<p>x</p>"]);
        let n = set_content_language(&mut b, "fr-CA").unwrap();
        assert!(n >= 1);
        assert_eq!(b.metadata().language, "fr-CA");
        let page = b.spine_paths()[0].clone();
        let t = b.archive.get_string(&page).unwrap();
        assert!(t.contains("lang=\"fr-CA\""), "{t}");
        assert_eq!(set_content_language(&mut b, "fr-CA").unwrap(), 0);
        assert!(set_content_language(&mut b, " ").is_err());

        let mut m = AccessMeta {
            summary: "Fully <accessible> & tagged".into(),
            conformance: "EPUB Accessibility 1.1 - WCAG 2.1 Level AA".into(),
            ..Default::default()
        };
        m.access_modes.insert("textual".into());
        m.access_modes.insert("visual".into());
        m.sufficient_modes.push("textual".into());
        m.features.insert("alternativeText".into());
        m.hazards.insert("none".into());
        assert!(write_access_meta(&mut b, &m).unwrap());
        assert_eq!(read_access_meta(&b), m);
        assert!(
            !write_access_meta(&mut b, &m).unwrap(),
            "second write changes nothing"
        );
        m.access_modes.remove("visual");
        m.summary.clear();
        write_access_meta(&mut b, &m).unwrap();
        assert_eq!(read_access_meta(&b), m);
        assert!(extension_counts(&b).get("xhtml").copied().unwrap_or(0) >= 1);
    }
}
