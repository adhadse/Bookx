//! Calibre-style "Reports" (E3): files, words, characters, CSS, links and images.

use std::collections::{BTreeMap, HashMap, HashSet};

use bookx_convert::transforms::css::{parse_css, parse_decls, split_selectors, CssItem};
use bookx_core::book::OpenBook;
use bookx_core::util::{extension, is_external, parent_dir, resolve_href};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

use crate::markup::*;

#[derive(Debug, Clone)]
pub struct ReportOptions {
    pub top_words: usize,
    pub top_chars: usize,
}

impl Default for ReportOptions {
    fn default() -> Self {
        Self {
            top_words: 100,
            top_chars: 100,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    pub kind: String,
    pub media_type: String,
    pub size: u64,
    pub in_manifest: bool,
    pub in_spine: bool,
    /// Words in the text of a markup file (0 for every other kind of file).
    #[serde(default)]
    pub words: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TypeTotals {
    pub count: usize,
    pub bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FilesReport {
    pub files: Vec<FileEntry>,
    pub by_type: BTreeMap<String, TypeTotals>,
    pub total_files: usize,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WordStats {
    pub total: usize,
    pub unique: usize,
    /// Words that occur exactly once.
    pub hapax: usize,
    /// Most frequent words (case folded), by count descending then alphabetically.
    pub top: Vec<(String, usize)>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WordsReport {
    pub overall: WordStats,
    pub by_language: BTreeMap<String, WordStats>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum CharCategory {
    LetterLower,
    LetterUpper,
    LetterOther,
    Number,
    Whitespace,
    Punctuation,
    Symbol,
    Control,
}

pub fn char_category(c: char) -> CharCategory {
    if c.is_lowercase() {
        CharCategory::LetterLower
    } else if c.is_uppercase() {
        CharCategory::LetterUpper
    } else if c.is_alphabetic() {
        CharCategory::LetterOther
    } else if c.is_numeric() {
        CharCategory::Number
    } else if c.is_whitespace() {
        CharCategory::Whitespace
    } else if c.is_control() {
        CharCategory::Control
    } else if (c.is_ascii_punctuation() && !"$+<=>^`|~".contains(c))
        || matches!(c as u32, 0xA1 | 0xA7 | 0xAB | 0xB6 | 0xB7 | 0xBB | 0xBF | 0x2010..=0x2027 | 0x2030..=0x205E | 0x3001..=0x3003 | 0x3008..=0x3011)
    {
        CharCategory::Punctuation
    } else {
        CharCategory::Symbol
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CharEntry {
    pub ch: char,
    pub code: u32,
    pub count: usize,
    pub category: CharCategory,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CharsReport {
    pub total: usize,
    pub unique: usize,
    pub top: Vec<CharEntry>,
    pub by_category: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuleUse {
    pub selector: String,
    /// Number of elements matched across all documents; `None` if the selector cannot be evaluated statically
    /// (pseudo-classes of dynamic state, unknown syntax). Such rules count as used.
    pub matches: Option<usize>,
    pub used: bool,
    pub declarations: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CssFileReport {
    pub path: String,
    pub size: u64,
    pub rules: Vec<RuleUse>,
    pub used_rules: usize,
    pub unused_rules: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CssReport {
    pub files: Vec<CssFileReport>,
    pub total_rules: usize,
    pub unused_rules: usize,
    /// Classes used in markup but defined by no selector.
    pub undefined_classes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LinkEntry {
    pub file: String,
    pub href: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LinksReport {
    pub internal: usize,
    pub external: usize,
    pub mail: usize,
    pub broken: Vec<LinkEntry>,
    pub external_links: Vec<LinkEntry>,
    /// Resources (images, stylesheets, fonts) referenced but absent from the archive.
    pub missing_resources: Vec<LinkEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageEntry {
    pub path: String,
    pub format: String,
    pub bytes: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Number of references from markup/CSS/OPF.
    pub references: usize,
    /// Rough estimate of bytes that recompression/resizing could save (see [`estimate_image_potential`]).
    pub potential_savings: u64,
    pub oversized: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImagesReport {
    pub images: Vec<ImageEntry>,
    pub total_bytes: u64,
    pub total_potential_savings: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Report {
    pub files: FilesReport,
    pub words: WordsReport,
    pub characters: CharsReport,
    pub css: CssReport,
    pub links: LinksReport,
    pub images: ImagesReport,
}

impl Report {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn from_json(s: &str) -> anyhow::Result<Report> {
        Ok(serde_json::from_str(s)?)
    }
}

pub fn build_report(book: &OpenBook, opts: &ReportOptions) -> Report {
    Report {
        files: files_report(book),
        words: words_report(book, opts.top_words),
        characters: chars_report(book, opts.top_chars),
        css: css_report(book),
        links: links_report(book),
        images: images_report(book),
    }
}

// ---------------------------------------------------------------------------
// files
// ---------------------------------------------------------------------------

pub fn file_kind(path: &str, media_type: &str) -> &'static str {
    if media_type == "application/xhtml+xml" || media_type == "text/html" || is_markup_path(path) {
        "text"
    } else if media_type == "text/css" || extension(path) == "css" {
        "style"
    } else if media_type.starts_with("image/") || bookx_core::util::is_image_path(path) {
        "image"
    } else if media_type.starts_with("font/")
        || media_type.contains("font")
        || matches!(extension(path).as_str(), "ttf" | "otf" | "woff" | "woff2")
    {
        "font"
    } else if media_type.starts_with("audio/") {
        "audio"
    } else if media_type.starts_with("video/") {
        "video"
    } else if matches!(extension(path).as_str(), "opf" | "ncx") || path.starts_with("META-INF/") {
        "package"
    } else {
        "other"
    }
}

pub fn files_report(book: &OpenBook) -> FilesReport {
    let spine: HashSet<String> = book.spine_paths().into_iter().collect();
    let mut rep = FilesReport::default();
    for name in book.archive.names() {
        if name == "mimetype" {
            // still reported, like any other file
        }
        let item = book.package.item_by_path(name);
        let media = item.map(|i| i.media_type.clone()).unwrap_or_default();
        let kind = file_kind(name, &media).to_string();
        let size = book.archive.size_of(name) as u64;
        let words = if kind == "text" {
            book.archive
                .get_string(name)
                .map(|t| tokenize_words(&plain_text(&t)).len())
                .unwrap_or(0)
        } else {
            0
        };
        let e = rep.by_type.entry(kind.clone()).or_default();
        e.count += 1;
        e.bytes += size;
        rep.total_bytes += size;
        rep.files.push(FileEntry {
            path: name.clone(),
            kind,
            media_type: media,
            size,
            in_manifest: item.is_some(),
            in_spine: spine.contains(name),
            words,
        });
    }
    rep.total_files = rep.files.len();
    rep.files
        .sort_by(|a, b| b.size.cmp(&a.size).then(a.path.cmp(&b.path)));
    rep
}

// ---------------------------------------------------------------------------
// words and characters
// ---------------------------------------------------------------------------

/// Byte ranges of the words in `text`: runs of letters/digits, allowing inner apostrophes and hyphens
/// ("don't", "well-known").
pub fn word_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut last_alnum_end = 0;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (k, &(i, c)) in chars.iter().enumerate() {
        let alnum = c.is_alphanumeric();
        let joiner = matches!(c, '\'' | '’' | '-' | '\u{2010}')
            && start.is_some()
            && chars.get(k + 1).is_some_and(|(_, n)| n.is_alphanumeric());
        if alnum {
            if start.is_none() {
                start = Some(i);
            }
            last_alnum_end = i + c.len_utf8();
        } else if !joiner {
            if let Some(s) = start.take() {
                out.push(s..last_alnum_end);
            }
        }
    }
    if let Some(s) = start {
        out.push(s..last_alnum_end);
    }
    out
}

/// Splits text into words, see [`word_ranges`].
pub fn tokenize_words(text: &str) -> Vec<&str> {
    word_ranges(text).into_iter().map(|r| &text[r]).collect()
}

/// Language of a markup document: `lang` / `xml:lang` of the root element.
pub fn document_language(src: &str) -> Option<String> {
    for ev in scan(src) {
        if let Event::Open { name, range, .. } = ev {
            if name == "html" || name == "svg" {
                let tag = &src[range];
                return tag_attr(tag, "xml:lang")
                    .or_else(|| tag_attr(tag, "lang"))
                    .filter(|l| !l.trim().is_empty());
            }
        }
    }
    None
}

/// Primary language subtag, lower-cased (`en-US` -> `en`).
pub fn primary_language(tag: &str) -> String {
    tag.split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn make_stats(counts: &HashMap<String, usize>, top_n: usize) -> WordStats {
    let mut v: Vec<(&String, &usize)> = counts.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    WordStats {
        total: counts.values().sum(),
        unique: counts.len(),
        hapax: counts.values().filter(|c| **c == 1).count(),
        top: v
            .into_iter()
            .take(top_n)
            .map(|(w, c)| (w.clone(), *c))
            .collect(),
    }
}

pub fn words_report(book: &OpenBook, top_n: usize) -> WordsReport {
    let book_lang = primary_language(&book.package.metadata.language);
    let mut overall: HashMap<String, usize> = HashMap::new();
    let mut per: HashMap<String, HashMap<String, usize>> = HashMap::new();
    for path in book_text_files(book) {
        let Some(src) = book.archive.get_string(&path) else {
            continue;
        };
        let lang = document_language(&src)
            .map(|l| primary_language(&l))
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| book_lang.clone());
        let lang = if lang.is_empty() {
            "und".to_string()
        } else {
            lang
        };
        let text = plain_text(&src);
        let lang_map = per.entry(lang).or_default();
        for w in tokenize_words(&text) {
            let w = w.to_lowercase();
            *overall.entry(w.clone()).or_default() += 1;
            *lang_map.entry(w).or_default() += 1;
        }
    }
    WordsReport {
        overall: make_stats(&overall, top_n),
        by_language: per
            .iter()
            .map(|(l, m)| (l.clone(), make_stats(m, top_n)))
            .collect(),
    }
}

/// Markup files in the spine order first, then the other markup files of the archive.
pub fn book_text_files(book: &OpenBook) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for p in book.spine_paths() {
        if book.archive.contains(&p) && seen.insert(p.clone()) {
            out.push(p);
        }
    }
    for p in markup_files(&book.archive) {
        if seen.insert(p.clone()) {
            out.push(p);
        }
    }
    out
}

pub fn chars_report(book: &OpenBook, top_n: usize) -> CharsReport {
    let mut counts: HashMap<char, usize> = HashMap::new();
    for path in book_text_files(book) {
        let Some(src) = book.archive.get_string(&path) else {
            continue;
        };
        for c in plain_text(&src).chars() {
            *counts.entry(c).or_default() += 1;
        }
    }
    let mut by_category: BTreeMap<String, usize> = BTreeMap::new();
    let mut entries: Vec<CharEntry> = counts
        .into_iter()
        .map(|(ch, count)| CharEntry {
            ch,
            code: ch as u32,
            count,
            category: char_category(ch),
        })
        .collect();
    for e in &entries {
        *by_category.entry(format!("{:?}", e.category)).or_default() += e.count;
    }
    entries.sort_by(|a, b| b.count.cmp(&a.count).then(a.code.cmp(&b.code)));
    CharsReport {
        total: entries.iter().map(|e| e.count).sum(),
        unique: entries.len(),
        top: entries.into_iter().take(top_n).collect(),
        by_category,
    }
}

// ---------------------------------------------------------------------------
// css
// ---------------------------------------------------------------------------

/// Selector text with the parts that cannot be decided from a static document removed. `None` if nothing
/// evaluable remains.
pub fn static_selector(sel: &str) -> Option<String> {
    let mut out = String::new();
    let chars: Vec<char> = sel.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ':' {
            let mut j = i + 1;
            if chars.get(j) == Some(&':') {
                j += 1;
            }
            let start = j;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '-') {
                j += 1;
            }
            let name: String = chars[start..j].iter().collect::<String>().to_lowercase();
            let mut end = j;
            if chars.get(j) == Some(&'(') {
                let mut depth = 0;
                while end < chars.len() {
                    if chars[end] == '(' {
                        depth += 1;
                    } else if chars[end] == ')' {
                        depth -= 1;
                        if depth == 0 {
                            end += 1;
                            break;
                        }
                    }
                    end += 1;
                }
            }
            let structural = matches!(
                name.as_str(),
                "first-child"
                    | "last-child"
                    | "only-child"
                    | "nth-child"
                    | "nth-last-child"
                    | "first-of-type"
                    | "last-of-type"
                    | "only-of-type"
                    | "nth-of-type"
                    | "nth-last-of-type"
                    | "not"
                    | "root"
                    | "empty"
                    | "lang"
            );
            if structural && chars[i + 1] != ':' {
                out.extend(&chars[i..end]);
            }
            i = end;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    let out = out.trim().to_string();
    let out = if out.is_empty() || out.ends_with(['>', '+', '~']) {
        format!("{out}*")
    } else {
        out
    };
    Some(out)
}

fn parse_docs(book: &OpenBook) -> Vec<Html> {
    book_text_files(book)
        .iter()
        .filter_map(|p| book.archive.get_string(p))
        .map(|t| Html::parse_document(&t))
        .collect()
}

pub fn css_report(book: &OpenBook) -> CssReport {
    let docs = parse_docs(book);
    let mut rep = CssReport::default();
    let mut cache: HashMap<String, Option<usize>> = HashMap::new();
    let mut defined: HashSet<String> = HashSet::new();
    for path in css_files(&book.archive) {
        let Some(css) = book.archive.get_string(&path) else {
            continue;
        };
        let mut file = CssFileReport {
            path: path.clone(),
            size: css.len() as u64,
            rules: Vec::new(),
            used_rules: 0,
            unused_rules: 0,
        };
        for item in parse_css(&css) {
            if let CssItem::Rule { selectors, block } = item {
                let ndecl = parse_decls(block).len();
                for s in split_selectors(selectors) {
                    collect_classes(&s, &mut defined);
                    let matches = *cache.entry(s.clone()).or_insert_with(|| {
                        let stripped = static_selector(&s)?;
                        let sel = Selector::parse(&stripped).ok()?;
                        Some(docs.iter().map(|d| d.select(&sel).count()).sum())
                    });
                    let used = matches.map(|m| m > 0).unwrap_or(true);
                    if used {
                        file.used_rules += 1;
                    } else {
                        file.unused_rules += 1;
                    }
                    file.rules.push(RuleUse {
                        selector: s,
                        matches,
                        used,
                        declarations: ndecl,
                    });
                }
            }
        }
        rep.total_rules += file.rules.len();
        rep.unused_rules += file.unused_rules;
        rep.files.push(file);
    }
    let mut used_classes: HashSet<String> = HashSet::new();
    let class_sel = Selector::parse("[class]").ok();
    if let Some(cs) = class_sel {
        for d in &docs {
            for el in d.select(&cs) {
                if let Some(c) = el.value().attr("class") {
                    used_classes.extend(c.split_whitespace().map(|s| s.to_string()));
                }
            }
        }
    }
    let mut undefined: Vec<String> = used_classes.difference(&defined).cloned().collect();
    undefined.sort();
    rep.undefined_classes = undefined;
    rep
}

pub(crate) fn collect_classes(selector: &str, out: &mut HashSet<String>) {
    let mut chars = selector.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '.' {
            let mut name = String::new();
            while let Some(&n) = chars.peek() {
                if n.is_alphanumeric() || n == '-' || n == '_' {
                    name.push(n);
                    chars.next();
                } else {
                    break;
                }
            }
            if !name.is_empty() {
                out.insert(name);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// links
// ---------------------------------------------------------------------------

pub fn links_report(book: &OpenBook) -> LinksReport {
    let mut rep = LinksReport::default();
    let files = markup_files(&book.archive);
    let mut ids: HashMap<String, HashSet<String>> = HashMap::new();
    let sources: Vec<(String, String)> = files
        .iter()
        .filter_map(|f| book.archive.get_string(f).map(|t| (f.clone(), t)))
        .collect();
    for (f, t) in &sources {
        ids.insert(f.clone(), defined_ids(t).into_iter().collect());
    }
    for (file, src) in &sources {
        let dir = parent_dir(file);
        let events = scan(src);
        for (k, ev) in events.iter().enumerate() {
            let Event::Open { name, range, .. } = ev else {
                continue;
            };
            let tag = &src[range.clone()];
            if name == "a" {
                let Some(href) = tag_attr(tag, "href") else {
                    continue;
                };
                let text = events[k + 1..]
                    .iter()
                    .take_while(|e| !matches!(e, Event::Close{name, ..} if name == "a"))
                    .filter_map(|e| match e {
                        Event::Text(r) => Some(decode_entities(&src[r.clone()])),
                        _ => None,
                    })
                    .collect::<String>();
                let entry = LinkEntry {
                    file: file.clone(),
                    href: href.clone(),
                    text: text.trim().to_string(),
                };
                let h = href.trim();
                if h.to_ascii_lowercase().starts_with("mailto:") {
                    rep.mail += 1;
                } else if is_external(h) {
                    rep.external += 1;
                    rep.external_links.push(entry);
                } else if h.is_empty() {
                    rep.broken.push(entry);
                } else {
                    rep.internal += 1;
                    let (path, frag) = resolve_href(dir, h);
                    let target = if path.is_empty() { file.clone() } else { path };
                    let ok = if !book.archive.contains(&target) {
                        false
                    } else if let (Some(f), Some(set)) = (frag.as_ref(), ids.get(&target)) {
                        set.contains(f)
                    } else {
                        true
                    };
                    if !ok {
                        rep.broken.push(entry);
                    }
                }
            } else {
                for attr in ["src", "poster"] {
                    if let Some(v) = tag_attr(tag, attr) {
                        check_resource(book, file, &v, &mut rep);
                    }
                }
                if name == "link" {
                    if let Some(v) = tag_attr(tag, "href") {
                        check_resource(book, file, &v, &mut rep);
                    }
                }
            }
        }
    }
    rep
}

fn check_resource(book: &OpenBook, file: &str, href: &str, rep: &mut LinksReport) {
    if !is_local_ref(href) {
        return;
    }
    let (path, _) = resolve_href(parent_dir(file), href);
    if !path.is_empty() && !book.archive.contains(&path) {
        rep.missing_resources.push(LinkEntry {
            file: file.to_string(),
            href: href.to_string(),
            text: String::new(),
        });
    }
}

// ---------------------------------------------------------------------------
// images
// ---------------------------------------------------------------------------

/// Header-only dimensions and format name of a raster image.
pub fn image_info(data: &[u8]) -> Option<(String, u32, u32)> {
    let reader = image::ImageReader::new(std::io::Cursor::new(data))
        .with_guessed_format()
        .ok()?;
    let fmt = reader.format()?;
    let (w, h) = reader.into_dimensions().ok()?;
    Some((format!("{fmt:?}").to_lowercase(), w, h))
}

/// Estimated bytes recompression could save. JPEG: everything above 0.5 byte per pixel; PNG: 12% of the size when
/// above 0.25 byte per pixel; other formats: 0. This is a heuristic, the real figure comes from `images::optimize`.
pub fn estimate_image_potential(format: &str, bytes: u64, width: u32, height: u32) -> u64 {
    let pixels = width as u64 * height as u64;
    match format {
        "jpeg" if pixels > 0 => bytes.saturating_sub(pixels / 2),
        "png" if pixels > 0 && bytes * 4 > pixels => bytes * 12 / 100,
        _ => 0,
    }
}

pub fn images_report(book: &OpenBook) -> ImagesReport {
    let mut refs: HashMap<String, usize> = HashMap::new();
    for name in book.archive.names() {
        if !is_reference_text(name) {
            continue;
        }
        let Some(text) = book.archive.get_string(name) else {
            continue;
        };
        let dir = parent_dir(name);
        for r in collect_refs(&text, ref_kind(name)) {
            if is_local_ref(&r) {
                let (p, _) = resolve_href(dir, &r);
                *refs.entry(p).or_default() += 1;
            }
        }
    }
    let mut rep = ImagesReport::default();
    for name in book.archive.names() {
        if !bookx_core::util::is_image_path(name) {
            continue;
        }
        let data = book.archive.get(name).unwrap_or(&[]);
        let bytes = data.len() as u64;
        let (format, w, h) = match image_info(data) {
            Some((f, w, h)) => (f, Some(w), Some(h)),
            None => (extension(name), None, None),
        };
        let potential = match (w, h) {
            (Some(w), Some(h)) => estimate_image_potential(&format, bytes, w, h),
            _ => 0,
        };
        rep.total_bytes += bytes;
        rep.total_potential_savings += potential;
        rep.images.push(ImageEntry {
            path: name.clone(),
            format,
            bytes,
            width: w,
            height: h,
            references: refs.get(name).copied().unwrap_or(0),
            potential_savings: potential,
            oversized: matches!((w, h), (Some(w), Some(h)) if w.max(h) > 3000),
        });
    }
    rep.images
        .sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.path.cmp(&b.path)));
    rep
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_core::archive::Archive;
    use bookx_core::convert::{create_epub, Chapter};
    use bookx_core::opf::Metadata;

    use crate::testutil::png;

    fn book() -> OpenBook {
        let md = Metadata {
            title: "T".into(),
            authors: vec!["A".into()],
            language: "en".into(),
            ..Default::default()
        };
        let chapters = vec![
            Chapter {
                title: "One".into(),
                body: "<h1>One</h1><p class=\"used\">The cat and the dog. The CAT's friend don't stop; well-known é.</p><p><a href=\"chapter-2.xhtml#x\">next</a> <a href=\"chapter-2.xhtml#nope\">bad</a> <a href=\"gone.xhtml\">gone</a> <a href=\"http://example.com/\">web</a> <a href=\"mailto:a@b.c\">mail</a> <img src=\"images/missing.png\"/><img src=\"images/p.png\"/></p>\n".into(),
            },
            Chapter {
                title: "Two".into(),
                body: "<p id=\"x\">Der Hund und die Katze</p>\n".into(),
            },
        ];
        let mut a = create_epub(&md, &chapters, None);
        a.set("OEBPS/images/p.png", png(40, 30));
        let c2 = a
            .get_string("OEBPS/chapter-2.xhtml")
            .unwrap()
            .replace("lang=\"en\"", "lang=\"de\"");
        a.set_string("OEBPS/chapter-2.xhtml", &c2);
        a.set_string(
            "OEBPS/style.css",
            "body { margin: 0 }\n.used { color: red }\n.unused { color: blue }\np:hover { color: green }\nh1, h9 { x: y }\n.zzz:nth-child(2) { a: b }\n",
        );
        OpenBook::from_archive(a).unwrap()
    }

    #[test]
    fn tokenizer_handles_apostrophes_hyphens_unicode() {
        assert_eq!(
            tokenize_words("Don't stop -- well-known 'quoted' naïve 42x, end-"),
            vec![
                "Don't",
                "stop",
                "well-known",
                "quoted",
                "naïve",
                "42x",
                "end"
            ]
        );
        assert!(tokenize_words("").is_empty());
        assert!(tokenize_words("... --- ''").is_empty());
        assert_eq!(tokenize_words("日本語 テスト"), vec!["日本語", "テスト"]);
    }

    #[test]
    fn categories() {
        assert_eq!(char_category('a'), CharCategory::LetterLower);
        assert_eq!(char_category('É'), CharCategory::LetterUpper);
        assert_eq!(char_category('日'), CharCategory::LetterOther);
        assert_eq!(char_category('5'), CharCategory::Number);
        assert_eq!(char_category(' '), CharCategory::Whitespace);
        assert_eq!(char_category('\u{7}'), CharCategory::Control);
        assert_eq!(char_category('!'), CharCategory::Punctuation);
        assert_eq!(char_category('—'), CharCategory::Punctuation);
        assert_eq!(char_category('«'), CharCategory::Punctuation);
        assert_eq!(char_category('+'), CharCategory::Symbol);
        assert_eq!(char_category('€'), CharCategory::Symbol);
    }

    #[test]
    fn files_by_type() {
        let b = book();
        let r = files_report(&b);
        assert_eq!(r.total_files, b.archive.names().len());
        assert_eq!(r.by_type["text"].count, 3); // 2 chapters + nav
        assert_eq!(r.by_type["style"].count, 1);
        assert_eq!(r.by_type["image"].count, 1);
        assert_eq!(r.by_type["package"].count, 3); // opf, ncx, container.xml
        assert_eq!(r.total_bytes, r.files.iter().map(|f| f.size).sum::<u64>());
        assert!(r.files.windows(2).all(|w| w[0].size >= w[1].size));
        let ch = r
            .files
            .iter()
            .find(|f| f.path.ends_with("chapter-1.xhtml"))
            .unwrap();
        assert!(ch.in_manifest && ch.in_spine);
        let nav = r
            .files
            .iter()
            .find(|f| f.path.ends_with("nav.xhtml"))
            .unwrap();
        assert!(nav.in_manifest && !nav.in_spine);
        assert!(r
            .files
            .iter()
            .any(|f| f.path == "mimetype" && !f.in_manifest));
    }

    #[test]
    fn file_kind_cases() {
        assert_eq!(file_kind("a.ttf", ""), "font");
        assert_eq!(file_kind("a.bin", "audio/mpeg"), "audio");
        assert_eq!(file_kind("a.bin", "video/mp4"), "video");
        assert_eq!(file_kind("a.xyz", ""), "other");
        assert_eq!(file_kind("a.svg", "image/svg+xml"), "image");
    }

    #[test]
    fn words_case_folded_per_language() {
        let b = book();
        let w = words_report(&b, 100);
        let top: HashMap<_, _> = w.by_language["en"].top.iter().cloned().collect();
        assert_eq!(top["the"], 3);
        assert_eq!(top["cat"], 1);
        assert_eq!(top["cat's"], 1);
        let de = &w.by_language["de"];
        assert_eq!(de.total, 5);
        assert_eq!(de.hapax, 5);
        assert!(words_report(&b, 5).overall.top.len() == 5);
        assert_eq!(w.overall.top[0].0, "the");
        assert_eq!(
            w.overall.total,
            w.by_language.values().map(|s| s.total).sum::<usize>()
        );
        assert!(w.overall.unique <= w.overall.total);
        // sorted: count desc then alphabetical
        assert!(w
            .overall
            .top
            .windows(2)
            .all(|p| p[0].1 > p[1].1 || (p[0].1 == p[1].1 && p[0].0 < p[1].0)));
    }

    #[test]
    fn characters_counted_with_categories() {
        let b = book();
        let c = chars_report(&b, 3);
        assert!(c.total > 50);
        assert_eq!(c.top.len(), 3);
        assert!(c.top[0].count >= c.top[1].count);
        assert!(c.by_category.contains_key("LetterLower"));
        assert!(c.by_category.contains_key("Punctuation"));
        let all = chars_report(&b, 1000);
        let e = all.top.iter().find(|e| e.ch == 'é').unwrap();
        assert_eq!(e.code, 0xe9);
        assert_eq!(all.total, all.top.iter().map(|e| e.count).sum::<usize>());
    }

    #[test]
    fn css_usage_with_counts() {
        let b = book();
        let c = css_report(&b);
        let f = c
            .files
            .iter()
            .find(|f| f.path.ends_with("style.css"))
            .unwrap();
        let get = |s: &str| f.rules.iter().find(|r| r.selector == s).unwrap().clone();
        assert_eq!(get(".used").matches, Some(1));
        assert!(get(".used").used);
        assert_eq!(get(".unused").matches, Some(0));
        assert!(!get(".unused").used);
        assert_eq!(get("h1").matches, Some(2)); // chapter 1 + nav
        assert!(!get("h9").used);
        assert!(get("p:hover").used, "dynamic pseudo-classes are stripped");
        assert!(get("p:hover").matches.unwrap() >= 2);
        assert!(!get(".zzz:nth-child(2)").used);
        assert_eq!(f.used_rules + f.unused_rules, f.rules.len());
        assert_eq!(c.unused_rules, 3);
        assert_eq!(get("body").declarations, 1);
    }

    #[test]
    fn css_undefined_classes() {
        let mut b = book();
        b.archive.set_string(
            "OEBPS/extra.xhtml",
            "<html><body><p class=\"ghost used\">x</p></body></html>",
        );
        let c = css_report(&b);
        assert_eq!(c.undefined_classes, vec!["ghost"]);
    }

    #[test]
    fn static_selector_rules() {
        assert_eq!(static_selector("a:hover").unwrap(), "a");
        assert_eq!(static_selector("p::first-line").unwrap(), "p");
        assert_eq!(
            static_selector("li:nth-child(2)").unwrap(),
            "li:nth-child(2)"
        );
        assert_eq!(static_selector("div >").unwrap(), "div >*");
        assert_eq!(static_selector(":hover").unwrap(), "*");
        assert_eq!(static_selector("a:not(.x)").unwrap(), "a:not(.x)");
    }

    #[test]
    fn links_classified() {
        let b = book();
        let l = links_report(&b);
        assert_eq!(l.internal, 5); // 3 in chapter 1 + 2 in nav
        assert_eq!(l.external, 1);
        assert_eq!(l.mail, 1);
        assert_eq!(l.broken.len(), 2);
        assert!(l
            .broken
            .iter()
            .any(|e| e.href == "chapter-2.xhtml#nope" && e.text == "bad"));
        assert!(l.broken.iter().any(|e| e.href == "gone.xhtml"));
        assert_eq!(l.external_links[0].href, "http://example.com/");
        assert_eq!(l.missing_resources.len(), 1);
        assert_eq!(l.missing_resources[0].href, "images/missing.png");
    }

    #[test]
    fn images_dimensions_and_references() {
        let b = book();
        let r = images_report(&b);
        assert_eq!(r.images.len(), 1);
        let i = &r.images[0];
        assert_eq!(i.format, "png");
        assert_eq!((i.width, i.height), (Some(40), Some(30)));
        assert_eq!(i.references, 1);
        assert_eq!(i.bytes, b.archive.size_of("OEBPS/images/p.png") as u64);
        assert!(!i.oversized);
        assert_eq!(r.total_bytes, i.bytes);
    }

    #[test]
    fn images_garbage_and_jpeg_estimates() {
        let mut b = book();
        b.archive
            .set("OEBPS/images/bad.png", b"not an image".to_vec());
        let r = images_report(&b);
        let bad = r
            .images
            .iter()
            .find(|i| i.path.ends_with("bad.png"))
            .unwrap();
        assert_eq!(bad.width, None);
        assert_eq!(bad.potential_savings, 0);
        assert_eq!(estimate_image_potential("jpeg", 100_000, 100, 100), 95_000);
        assert_eq!(estimate_image_potential("jpeg", 4_000, 100, 100), 0);
        assert_eq!(estimate_image_potential("png", 10_000, 100, 100), 1_200);
        assert_eq!(estimate_image_potential("png", 100, 100, 100), 0);
        assert_eq!(estimate_image_potential("gif", 100, 1, 1), 0);
        assert_eq!(image_info(b"xx"), None);
    }

    #[test]
    fn full_report_json_roundtrip() {
        let b = book();
        let r = build_report(&b, &ReportOptions::default());
        let json = r.to_json();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        for k in ["files", "words", "characters", "css", "links", "images"] {
            assert!(v.get(k).is_some(), "{k}");
        }
        assert_eq!(Report::from_json(&json).unwrap(), r);
        assert!(Report::from_json("{").is_err());
    }

    #[test]
    fn empty_archive_report_does_not_panic() {
        let mut a = Archive::default();
        a.set_string("x.txt", "hi");
        let b = OpenBook {
            archive: a,
            package: bookx_core::opf::Package::parse_text(
                "c.opf",
                "<package version=\"2.0\"><metadata/><manifest/><spine/></package>",
            )
            .unwrap(),
        };
        let r = build_report(&b, &ReportOptions::default());
        assert_eq!(r.words.overall.total, 0);
        assert_eq!(r.files.total_files, 1);
    }

    #[test]
    fn document_language_variants() {
        assert_eq!(
            document_language("<html xml:lang=\"fr-CA\" lang=\"en\">").as_deref(),
            Some("fr-CA")
        );
        assert_eq!(
            document_language("<html lang=\"de\">").as_deref(),
            Some("de")
        );
        assert_eq!(document_language("<html lang=\"\">"), None);
        assert_eq!(document_language("<p>x</p>"), None);
        assert_eq!(primary_language("pt_BR"), "pt");
    }
}
