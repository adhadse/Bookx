//! Structural digest of a book, used to compare conversions (against Calibre's output, between
//! formats, after a round trip). A [`Digest`] keeps what a user would notice losing: metadata, the
//! table of contents, the text as a normalised word stream, images, the cover and whether the
//! internal links still resolve.

use std::collections::{HashMap, HashSet};

use bookx_core::book::OpenBook;
use bookx_core::toc::{flatten, TocEntry};
use bookx_core::util::{is_external, parent_dir, resolve_href};
use scraper::{Html, Node, Selector};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Digest {
    pub title: String,
    pub authors: Vec<String>,
    pub language: String,
    pub identifiers: Vec<(String, String)>,
    pub series: String,
    pub series_index: f64,
    pub has_cover: bool,
    /// `(depth, title)` in document order.
    pub toc: Vec<(usize, String)>,
    pub toc_unresolved: usize,
    pub words: Vec<String>,
    pub images: usize,
    pub internal_links: usize,
    pub broken_links: usize,
}

/// Lowercases and folds typographic variants so that smart quotes, ligatures and non-breaking
/// spaces do not count as differences.
pub fn fold_char(c: char) -> Option<char> {
    Some(match c {
        '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{2032}' => '\'',
        '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{2033}' => '"',
        '\u{00A0}' | '\u{2009}' | '\u{202F}' | '\u{200A}' | '\u{2002}' | '\u{2003}' => ' ',
        '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}' | '\u{00AD}' | '\u{2060}' => return None,
        '\u{FB00}' => return Some('f'),
        _ => c,
    })
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF)
}

/// Splits text into comparable tokens: lowercase alphanumeric runs, one token per CJK character.
pub fn tokenize(text: &str, out: &mut Vec<String>) {
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        if !cur.is_empty() {
            out.push(std::mem::take(cur));
        }
    };
    for c in text.chars() {
        let Some(c) = fold_char(c) else { continue };
        if is_cjk(c) {
            flush(&mut cur, out);
            out.push(c.to_string());
        } else if c.is_alphanumeric() {
            for l in c.to_lowercase() {
                cur.push(l);
            }
        } else if c == '\'' && !cur.is_empty() {
            // "don't" and "dont" are the same word for our purposes
        } else {
            flush(&mut cur, out);
        }
    }
    flush(&mut cur, out);
}

const BLOCKS: &[&str] = &[
    "p", "div", "br", "li", "tr", "td", "th", "h1", "h2", "h3", "h4", "h5", "h6",
];

fn walk(el: scraper::ElementRef, out: &mut String) {
    let name = el.value().name();
    if matches!(name, "script" | "style" | "head" | "title" | "rt" | "rp") {
        return;
    }
    let block = BLOCKS.contains(&name);
    if block {
        out.push(' ');
    }
    for c in el.children() {
        match c.value() {
            Node::Text(t) => out.push_str(t),
            Node::Element(_) => {
                if let Some(e) = scraper::ElementRef::wrap(c) {
                    walk(e, out);
                }
            }
            _ => {}
        }
    }
    if block {
        out.push(' ');
    }
}

fn body_text(doc: &Html) -> String {
    let mut out = String::new();
    walk(doc.root_element(), &mut out);
    out
}

fn toc_titles(entries: &[TocEntry]) -> Vec<(usize, String)> {
    flatten(entries)
        .into_iter()
        .map(|f| (f.depth, norm_title(&f.title)))
        .collect()
}

fn norm_title(t: &str) -> String {
    let mut w = Vec::new();
    tokenize(t, &mut w);
    w.join(" ")
}

impl Digest {
    pub fn of(book: &OpenBook) -> Digest {
        let m = book.metadata();
        let mut d = Digest {
            title: norm_title(&m.title),
            authors: m
                .authors
                .iter()
                .map(|a| norm_title(a))
                .filter(|a| a != "unknown" && !a.is_empty())
                .collect(),
            language: m
                .language
                .split('-')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase(),
            identifiers: m
                .identifiers
                .iter()
                .filter(|i| !matches!(i.scheme.to_ascii_lowercase().as_str(), "uuid" | "calibre"))
                .map(|i| (i.scheme.to_ascii_lowercase(), i.value.clone()))
                .collect(),
            series: norm_title(&m.series),
            series_index: m.series_index,
            has_cover: book.cover_bytes().is_some(),
            ..Default::default()
        };
        let archive = &book.archive;
        let toc = book.toc();
        d.toc = toc_titles(&toc);
        for f in flatten(&toc) {
            let (path, _) = resolve_href("", &f.href);
            if !path.is_empty() && !archive.contains(&path) {
                d.toc_unresolved += 1;
            }
        }
        d.images = book
            .package
            .manifest
            .iter()
            .filter(|i| i.media_type.starts_with("image/"))
            .filter(|i| {
                // the cover is counted through `has_cover`
                !i.properties.contains("cover-image")
                    && book.package.cover_id.as_deref() != Some(i.id.as_str())
            })
            .count();

        let mut ids: HashMap<String, HashSet<String>> = HashMap::new();
        let spine = book.spine_paths();
        let mut docs = Vec::new();
        for path in &spine {
            let Some(src) = archive.get_string(path) else {
                continue;
            };
            let html = Html::parse_document(&src);
            let set = ids.entry(path.clone()).or_default();
            for n in html.tree.nodes() {
                if let Node::Element(e) = n.value() {
                    if let Some(id) = e.attr("id") {
                        set.insert(id.to_string());
                    }
                    if e.name() == "a" {
                        if let Some(name) = e.attr("name") {
                            set.insert(name.to_string());
                        }
                    }
                }
            }
            tokenize(&body_text(&html), &mut d.words);
            docs.push((path.clone(), html));
        }
        let a = Selector::parse("a[href]").expect("selector");
        for (path, html) in &docs {
            for el in html.select(&a) {
                let href = el.value().attr("href").unwrap_or("").trim();
                if href.is_empty() || is_external(href) || href.contains(':') {
                    continue;
                }
                d.internal_links += 1;
                let (target, frag) = resolve_href(parent_dir(path), href);
                let target = if target.is_empty() {
                    path.clone()
                } else {
                    target
                };
                let ok = archive.contains(&target)
                    && frag
                        .as_ref()
                        .is_none_or(|f| ids.get(&target).is_none_or(|s| s.contains(f)));
                if !ok {
                    d.broken_links += 1;
                }
            }
        }
        d
    }
}

/// One difference between two digests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diff {
    pub category: &'static str,
    pub detail: String,
}

/// How strictly to compare; formats that cannot carry something are told so by the caller.
#[derive(Debug, Clone, Copy)]
pub struct Tolerance {
    /// Fraction of reference words that may be missing from the output.
    pub missing: f64,
    /// Fraction of output words that may not be in the reference (generated pages, labels).
    pub extra: f64,
}

impl Default for Tolerance {
    fn default() -> Self {
        Tolerance {
            missing: 0.002,
            extra: 0.02,
        }
    }
}

/// Length of the longest common subsequence of two token slices, computed on the most frequent
/// alignment-safe way (patience-style anchoring keeps this fast for long books).
pub fn lcs_len(a: &[String], b: &[String]) -> usize {
    // trim common prefix and suffix
    let mut s = 0;
    while s < a.len() && s < b.len() && a[s] == b[s] {
        s += 1;
    }
    let (mut ea, mut eb) = (a.len(), b.len());
    while ea > s && eb > s && a[ea - 1] == b[eb - 1] {
        ea -= 1;
        eb -= 1;
    }
    let (x, y) = (&a[s..ea], &b[s..eb]);
    let common = s + (a.len() - ea);
    if x.is_empty() || y.is_empty() {
        return common;
    }
    if x.len() as u64 * y.len() as u64 > 40_000_000 {
        // too large for the quadratic table: fall back to the multiset overlap (an upper bound)
        let mut counts: HashMap<&str, i64> = HashMap::new();
        for w in x {
            *counts.entry(w.as_str()).or_default() += 1;
        }
        let mut n = 0;
        for w in y {
            if let Some(c) = counts.get_mut(w.as_str()) {
                if *c > 0 {
                    *c -= 1;
                    n += 1;
                }
            }
        }
        return common + n;
    }
    let mut prev = vec![0u32; y.len() + 1];
    let mut cur = vec![0u32; y.len() + 1];
    for xi in x {
        for (j, yj) in y.iter().enumerate() {
            cur[j + 1] = if xi == yj {
                prev[j] + 1
            } else {
                prev[j + 1].max(cur[j])
            };
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    common + prev[y.len()] as usize
}

/// Words of `reference` that are missing from `got` and words of `got` that are not in
/// `reference` (multiset difference).
pub fn word_delta(reference: &[String], got: &[String]) -> (Vec<String>, Vec<String>) {
    let mut counts: HashMap<&str, i64> = HashMap::new();
    for w in reference {
        *counts.entry(w.as_str()).or_default() += 1;
    }
    let mut extra = Vec::new();
    for w in got {
        match counts.get_mut(w.as_str()) {
            Some(c) if *c > 0 => *c -= 1,
            _ => extra.push(w.clone()),
        }
    }
    let mut missing = Vec::new();
    let mut left: HashMap<&str, i64> = counts;
    for w in reference {
        if let Some(c) = left.get_mut(w.as_str()) {
            if *c > 0 {
                *c -= 1;
                missing.push(w.clone());
            }
        }
    }
    (missing, extra)
}

fn sample(words: &[String]) -> String {
    words.iter().take(8).cloned().collect::<Vec<_>>().join(" ")
}

/// Compares `got` against `reference` and lists the differences a user would care about.
pub fn compare(reference: &Digest, got: &Digest, tol: Tolerance) -> Vec<Diff> {
    let mut out = Vec::new();
    let mut push = |category: &'static str, detail: String| out.push(Diff { category, detail });
    if reference.title != got.title {
        push("title", format!("`{}` vs `{}`", reference.title, got.title));
    }
    if reference.authors != got.authors {
        push(
            "authors",
            format!("{:?} vs {:?}", reference.authors, got.authors),
        );
    }
    if reference.language != got.language && !reference.language.is_empty() {
        push(
            "language",
            format!("`{}` vs `{}`", reference.language, got.language),
        );
    }
    for id in &reference.identifiers {
        if !got.identifiers.contains(id) {
            push("identifier", format!("{}:{} lost", id.0, id.1));
        }
    }
    if reference.series != got.series {
        push(
            "series",
            format!("`{}` vs `{}`", reference.series, got.series),
        );
    } else if !reference.series.is_empty()
        && (reference.series_index - got.series_index).abs() > 1e-6
    {
        push(
            "series",
            format!("index {} vs {}", reference.series_index, got.series_index),
        );
    }
    if reference.has_cover != got.has_cover {
        push(
            "cover",
            format!("{} vs {}", reference.has_cover, got.has_cover),
        );
    }
    if reference.toc != got.toc {
        let depth_only = reference.toc.len() == got.toc.len()
            && reference.toc.iter().zip(&got.toc).all(|(a, b)| a.1 == b.1);
        let titles = |d: &Digest| d.toc.iter().map(|t| t.1.clone()).collect::<Vec<_>>();
        let (m, e) = word_delta(&titles(reference), &titles(got));
        push(
            if depth_only { "toc-depth" } else { "toc" },
            format!(
                "{} vs {} entries; missing {:?}; extra {:?}",
                reference.toc.len(),
                got.toc.len(),
                m.iter().take(4).collect::<Vec<_>>(),
                e.iter().take(4).collect::<Vec<_>>()
            ),
        );
    }
    if got.toc_unresolved > 0 {
        push(
            "toc-links",
            format!("{} entries point nowhere", got.toc_unresolved),
        );
    }
    if reference.images != got.images {
        push("images", format!("{} vs {}", reference.images, got.images));
    }
    let (missing, extra) = word_delta(&reference.words, &got.words);
    let rl = reference.words.len().max(1) as f64;
    let gl = got.words.len().max(1) as f64;
    if missing.len() as f64 / rl > tol.missing {
        push(
            "text-missing",
            format!(
                "{} of {} words, e.g. {}",
                missing.len(),
                reference.words.len(),
                sample(&missing)
            ),
        );
    }
    if extra.len() as f64 / gl > tol.extra {
        push(
            "text-extra",
            format!(
                "{} of {} words, e.g. {}",
                extra.len(),
                got.words.len(),
                sample(&extra)
            ),
        );
    }
    if missing.is_empty() && extra.is_empty() && reference.words != got.words {
        push("text-order", "same words in a different order".into());
    } else if reference.words.len().min(got.words.len()) < 20_000 {
        let l = lcs_len(&reference.words, &got.words);
        let common = reference.words.len() - missing.len();
        if common > 0 && (common - l.min(common)) as f64 / common as f64 > 0.01 {
            push(
                "text-order",
                format!("{} common words out of sequence", common - l.min(common)),
            );
        }
    }
    if reference.internal_links > 0 && got.broken_links > reference.broken_links {
        push(
            "links",
            format!(
                "{} broken of {} (reference {} of {})",
                got.broken_links,
                got.internal_links,
                reference.broken_links,
                reference.internal_links
            ),
        );
    }
    out
}

/// How much of the truth (what the source document held) survived in `got`; every field is a
/// fraction in 0..=1 where 1 means "all of it".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Score {
    pub title: f64,
    pub authors: f64,
    pub language: f64,
    pub identifiers: f64,
    pub series: f64,
    pub cover: f64,
    pub toc: f64,
    pub toc_depth: f64,
    pub recall: f64,
    /// Fraction of the output words that are not in the truth (generated pages, labels, junk).
    pub extra: f64,
    pub order: f64,
    pub images: f64,
    pub broken_links: f64,
}

fn lang_key(l: &str) -> &str {
    match l {
        "eng" => "en",
        "rus" => "ru",
        "deu" | "ger" => "de",
        "fra" | "fre" => "fr",
        "spa" => "es",
        "jpn" => "ja",
        "ara" => "ar",
        "zho" | "chi" => "zh",
        "ita" => "it",
        "por" => "pt",
        "heb" => "he",
        "kor" => "ko",
        "nld" | "dut" => "nl",
        "pol" => "pl",
        "tur" => "tr",
        other => other,
    }
}

fn frac(found: usize, total: usize) -> f64 {
    if total == 0 {
        1.0
    } else {
        found as f64 / total as f64
    }
}

/// Scores `got` against `truth`. Facets the truth does not have score 1.
pub fn score(truth: &Digest, got: &Digest) -> Score {
    let mut s = Score {
        title: (truth.title.is_empty() || truth.title == got.title) as u8 as f64,
        language: (truth.language.is_empty()
            || lang_key(&truth.language) == lang_key(&got.language)) as u8 as f64,
        series: (truth.series.is_empty()
            || (truth.series == got.series && (truth.series_index - got.series_index).abs() < 1e-6))
            as u8 as f64,
        cover: (!truth.has_cover || got.has_cover) as u8 as f64,
        ..Default::default()
    };
    s.authors = frac(
        truth
            .authors
            .iter()
            .filter(|a| got.authors.contains(a))
            .count(),
        truth.authors.len(),
    );
    s.identifiers = frac(
        truth
            .identifiers
            .iter()
            .filter(|i| !matches!(i.0.as_str(), "mobi-asin" | "amazon"))
            .filter(|i| got.identifiers.contains(i))
            .count(),
        truth
            .identifiers
            .iter()
            .filter(|i| !matches!(i.0.as_str(), "mobi-asin" | "amazon"))
            .count(),
    );
    let mut have: HashMap<&str, i64> = HashMap::new();
    for (_, t) in &got.toc {
        *have.entry(t.as_str()).or_default() += 1;
    }
    let mut found = 0;
    let mut depth_ok = 0;
    for (d, t) in &truth.toc {
        if let Some(c) = have.get_mut(t.as_str()) {
            if *c > 0 {
                *c -= 1;
                found += 1;
                if got.toc.iter().any(|g| &g.1 == t && g.0 == *d) {
                    depth_ok += 1;
                }
            }
        }
    }
    s.toc = frac(found, truth.toc.len());
    s.toc_depth = frac(depth_ok, found);
    let (missing, extra) = word_delta(&truth.words, &got.words);
    s.recall = 1.0 - frac(missing.len(), truth.words.len());
    s.extra = if got.words.is_empty() {
        0.0
    } else {
        extra.len() as f64 / got.words.len() as f64
    };
    let common = truth.words.len() - missing.len();
    s.order = if truth.words.len() > 20_000 || got.words.len() > 20_000 {
        1.0
    } else {
        frac(lcs_len(&truth.words, &got.words).min(common), common)
    };
    s.images = if truth.images == 0 {
        1.0
    } else {
        (got.images as f64 / truth.images as f64).min(1.0)
    };
    s.broken_links = got.broken_links as f64;
    s
}

/// Facets where `ours` is clearly worse than `theirs` (both scored against the same truth).
pub fn regressions(theirs: &Score, ours: &Score) -> Vec<Diff> {
    let mut out = Vec::new();
    let mut chk = |category: &'static str, t: f64, o: f64, eps: f64| {
        if o + eps < t {
            out.push(Diff {
                category,
                detail: format!("{o:.3} vs reference {t:.3}"),
            });
        }
    };
    chk("title", theirs.title, ours.title, 0.5);
    chk("authors", theirs.authors, ours.authors, 0.01);
    chk("language", theirs.language, ours.language, 0.5);
    chk("identifier", theirs.identifiers, ours.identifiers, 0.01);
    chk("series", theirs.series, ours.series, 0.5);
    chk("cover", theirs.cover, ours.cover, 0.5);
    chk("toc", theirs.toc, ours.toc, 0.05);
    chk("toc-depth", theirs.toc_depth, ours.toc_depth, 0.05);
    chk("text-missing", theirs.recall, ours.recall, 0.003);
    chk("text-extra", 1.0 - theirs.extra, 1.0 - ours.extra, 0.03);
    chk("text-order", theirs.order, ours.order, 0.01);
    chk("images", theirs.images, ours.images, 0.01);
    if ours.broken_links > theirs.broken_links {
        out.push(Diff {
            category: "links",
            detail: format!(
                "{} broken vs reference {}",
                ours.broken_links, theirs.broken_links
            ),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<String> {
        let mut v = Vec::new();
        tokenize(s, &mut v);
        v
    }

    #[test]
    fn tokens_fold_typography() {
        assert_eq!(
            words("Don’t “stop” — now\u{a0}ok"),
            words("dont \"stop\" now ok")
        );
        assert_eq!(words("日本語 text"), vec!["日", "本", "語", "text"]);
    }

    #[test]
    fn delta_and_lcs() {
        let a = words("one two three four");
        let b = words("one three two four five");
        let (m, e) = word_delta(&a, &b);
        assert!(m.is_empty());
        assert_eq!(e, vec!["five"]);
        assert_eq!(lcs_len(&a, &b), 3);
    }

    #[test]
    fn compare_flags_loss_and_order() {
        let mut r = Digest {
            words: words("a b c d e f g h i j"),
            ..Default::default()
        };
        let mut g = r.clone();
        assert!(compare(&r, &g, Tolerance::default()).is_empty());
        g.words.pop();
        let d = compare(&r, &g, Tolerance::default());
        assert!(d.iter().any(|x| x.category == "text-missing"));
        g = r.clone();
        g.words.swap(1, 8);
        let d = compare(&r, &g, Tolerance::default());
        assert!(d.iter().any(|x| x.category == "text-order"));
        r.has_cover = true;
        let d = compare(&r, &r.clone(), Tolerance::default());
        assert!(d.is_empty());
    }
}
