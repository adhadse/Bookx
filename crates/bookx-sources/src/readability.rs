//! Article extraction for read-later (S1): finds the main content of a web page, cleans it to
//! XHTML, resolves URLs, lists the images to fetch and packs the result into an EPUB.
//!
//! The extractor is a text-density heuristic: `<article>`/`<main>`/`itemprop=articleBody` win when
//! they hold enough text, otherwise the container whose paragraphs score best (length, commas, low
//! link density) is chosen. Navigation, ads, forms, scripts and the like are dropped while the
//! cleaned tree is written out, so the output only ever contains a small whitelist of tags.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use anyhow::{bail, Result};
use bookx_core::archive::Archive;
use bookx_core::convert::{create_epub, Chapter};
use bookx_core::opf::{Identifier, Metadata, Package};
use ego_tree::{NodeId, NodeRef};
use regex::Regex;
use scraper::{ElementRef, Html, Node, Selector};
use serde_json::Value;

use crate::fetch::Fetcher;
use crate::sources::normalize_date;
use crate::text::strip_html;

/// An extracted article.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Article {
    pub url: String,
    pub title: String,
    pub author: String,
    pub site: String,
    /// `YYYY[-MM[-DD]]`, empty when unknown.
    pub published: String,
    pub description: String,
    pub language: String,
    /// Absolute URL of the lead image (Open Graph / schema.org), if any.
    pub lead_image: Option<String>,
    /// Clean XHTML fragment (block elements only at top level).
    pub content: String,
    /// Absolute URLs of the images used in `content`, in order, without duplicates.
    pub images: Vec<String>,
    /// Characters of visible text in `content`.
    pub text_len: usize,
}

impl Article {
    /// Lead image first, then the content images.
    pub fn all_image_urls(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for u in self.lead_image.iter().chain(self.images.iter()) {
            if !v.contains(u) {
                v.push(u.clone());
            }
        }
        v
    }
}

/// Options for [`extract_with`].
#[derive(Debug, Clone)]
pub struct ExtractOptions {
    /// CSS selectors; when given, only these elements form the article (instead of the detection).
    pub keep: Vec<String>,
    /// CSS selectors removed before anything else.
    pub remove: Vec<String>,
    /// Fewer visible characters than this is an error ("nothing readable").
    pub min_text_len: usize,
    /// Upper bound of images collected per article.
    pub max_images: usize,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        ExtractOptions {
            keep: Vec::new(),
            remove: Vec::new(),
            min_text_len: 1,
            max_images: 100,
        }
    }
}

/// Extracts the article of `html`; `base_url` resolves relative links and images.
pub fn extract(html: &str, base_url: &str) -> Result<Article> {
    extract_with(html, base_url, &ExtractOptions::default())
}

// ---------------------------------------------------------------------------------------------
// URL handling
// ---------------------------------------------------------------------------------------------

/// Resolves `href` against `base` (absolute http(s) result only). Handles `//host`, `/path`,
/// `../x`, `?q`. Returns `None` for fragments, `javascript:`, `data:`, `mailto:` and invalid input.
pub fn resolve_url(base: &str, href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() || href.starts_with('#') {
        return None;
    }
    let lower = href.to_ascii_lowercase();
    if lower.starts_with("javascript:")
        || lower.starts_with("data:")
        || lower.starts_with("mailto:")
        || lower.starts_with("tel:")
    {
        return None;
    }
    let joined = match url::Url::parse(base) {
        Ok(b) => b.join(href).ok()?,
        Err(_) => url::Url::parse(href).ok()?,
    };
    if matches!(joined.scheme(), "http" | "https") {
        Some(joined.to_string())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------------------------
// tree helpers
// ---------------------------------------------------------------------------------------------

const JUNK_TAGS: &[&str] = &[
    "script", "style", "noscript", "nav", "aside", "footer", "form", "iframe", "button", "select",
    "input", "textarea", "svg", "canvas", "template", "object", "embed", "link", "meta", "dialog",
    "menu", "head", "title", "audio", "video", "map", "applet", "option", "optgroup", "datalist",
];

fn junk_class_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"(?:^|[\s_-])(?:ad|ads|advert|advertisement|adsbygoogle|banner|breadcrumb|breadcrumbs|comment|comments|cookie|cookies|disqus|footer|masthead|menu|modal|navbar|nav|newsletter|outbrain|popup|promo|related|share|sharing|shoutbox|sidebar|skip-link|social|sponsor|sponsored|subscribe|taboola|toolbar|widget|paywall)(?:[\s_-]|$)",
        )
        .expect("static regex")
    })
}

fn el_name<'a>(n: NodeRef<'a, Node>) -> Option<&'a str> {
    n.value().as_element().map(|e| e.name())
}

fn attr<'a>(n: NodeRef<'a, Node>, name: &str) -> Option<&'a str> {
    n.value().as_element().and_then(|e| e.attr(name))
}

fn is_body_like(name: &str) -> bool {
    matches!(name, "html" | "body" | "main" | "article")
}

/// Per-page state: junk and protected node sets.
struct Page<'a> {
    doc: &'a Html,
    junk: HashSet<NodeId>,
}

impl<'a> Page<'a> {
    fn new(doc: &'a Html, remove_selectors: &[String]) -> Result<Self> {
        // elements that contain the real content must never be dropped for their class names
        let mut protected: HashSet<NodeId> = HashSet::new();
        for sel in ["article", "main", "[role=main]", "[itemprop=articleBody]"] {
            if let Ok(s) = Selector::parse(sel) {
                for el in doc.select(&s) {
                    let mut cur = Some(*el);
                    while let Some(n) = cur {
                        protected.insert(n.id());
                        cur = n.parent();
                    }
                }
            }
        }
        let mut junk = HashSet::new();
        for node in doc.tree.nodes() {
            let Some(e) = node.value().as_element() else {
                continue;
            };
            let name = e.name();
            let mut bad = JUNK_TAGS.contains(&name);
            if !bad && name == "header" {
                // a <header> inside an <article> carries title and byline
                bad = !node.ancestors().any(|a| el_name(a) == Some("article"));
            }
            if !bad && e.attr("hidden").is_some() {
                bad = true;
            }
            if !bad && e.attr("aria-hidden") == Some("true") {
                bad = true;
            }
            if !bad {
                if let Some(st) = e.attr("style") {
                    let st = st.replace(' ', "").to_lowercase();
                    bad = st.contains("display:none") || st.contains("visibility:hidden");
                }
            }
            if !bad && !is_body_like(name) && !protected.contains(&node.id()) {
                let re = junk_class_re();
                bad = e
                    .attr("class")
                    .is_some_and(|c| re.is_match(&c.to_lowercase()))
                    || e.attr("id").is_some_and(|c| re.is_match(&c.to_lowercase()))
                    || e.attr("role").is_some_and(|r| {
                        matches!(
                            r,
                            "navigation" | "banner" | "contentinfo" | "complementary" | "search"
                        )
                    });
            }
            if bad {
                junk.insert(node.id());
            }
        }
        for sel in remove_selectors {
            let s = Selector::parse(sel)
                .map_err(|e| anyhow::anyhow!("invalid remove selector `{sel}`: {e}"))?;
            for el in doc.select(&s) {
                junk.insert(el.id());
            }
        }
        Ok(Page { doc, junk })
    }

    fn is_junk(&self, n: NodeRef<Node>) -> bool {
        self.junk.contains(&n.id())
    }

    fn in_junk(&self, n: NodeRef<Node>) -> bool {
        self.is_junk(n) || n.ancestors().any(|a| self.is_junk(a))
    }

    /// Visible text length below `n`, junk excluded, whitespace collapsed.
    fn text_len(&self, n: NodeRef<Node>) -> usize {
        let mut total: usize = 0;
        let mut last_space = true;
        self.walk_text(n, &mut |s| {
            for c in s.chars() {
                if c.is_whitespace() {
                    if !last_space {
                        total += 1;
                        last_space = true;
                    }
                } else {
                    total += 1;
                    last_space = false;
                }
            }
        });
        total.saturating_sub(usize::from(last_space && total > 0))
    }

    fn link_text_len(&self, n: NodeRef<Node>) -> usize {
        n.descendants()
            .filter(|d| el_name(*d) == Some("a") && !self.in_junk(*d))
            .map(|a| self.text_len(a))
            .sum()
    }

    fn walk_text(&self, n: NodeRef<Node>, f: &mut dyn FnMut(&str)) {
        for c in n.children() {
            match c.value() {
                Node::Text(t) => f(t),
                Node::Element(_) if !self.is_junk(c) => self.walk_text(c, f),
                _ => {}
            }
        }
    }

    fn node_text(&self, n: NodeRef<Node>) -> String {
        let mut s = String::new();
        self.walk_text(n, &mut |t| s.push_str(t));
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// The container to turn into the article.
    fn pick_container(&self) -> Option<NodeId> {
        let mut best: Option<(usize, NodeId)> = None;
        for sel in ["article", "[itemprop=articleBody]", "main", "[role=main]"] {
            let Ok(s) = Selector::parse(sel) else {
                continue;
            };
            best = None;
            for el in self.doc.select(&s) {
                if self.in_junk(*el) {
                    continue;
                }
                let len = self.text_len(*el);
                if len >= 200 && best.is_none_or(|(l, _)| len > l) {
                    best = Some((len, el.id()));
                }
            }
            if best.is_some() {
                break;
            }
        }
        if let Some((_, id)) = best {
            return Some(id);
        }
        self.score_container()
    }

    fn score_container(&self) -> Option<NodeId> {
        let mut scores: HashMap<NodeId, f64> = HashMap::new();
        for node in self.doc.tree.nodes() {
            let Some(name) = el_name(node) else { continue };
            if !matches!(name, "p" | "pre" | "blockquote" | "td") || self.in_junk(node) {
                continue;
            }
            let len = self.text_len(node);
            if len < 25 {
                continue;
            }
            let commas = self
                .node_text(node)
                .chars()
                .filter(|c| matches!(c, ',' | '，' | '、'))
                .count();
            let links = self.link_text_len(node);
            let density = (links as f64 / len as f64).min(1.0);
            let score = (1.0 + commas as f64 + (len as f64 / 100.0).min(3.0)) * (1.0 - density);
            if let Some(p) = node.parent() {
                *scores.entry(p.id()).or_default() += score;
                if let Some(gp) = p.parent() {
                    *scores.entry(gp.id()).or_default() += score / 2.0;
                }
            }
        }
        let (mut best_id, mut best_score) = scores
            .iter()
            .max_by(|a, b| a.1.total_cmp(b.1).then_with(|| b.0.cmp(a.0)))
            .map(|(id, s)| (*id, *s))?;
        // content split over sibling blocks: the parent collects half of every child's score
        while let Some(node) = self.doc.tree.get(best_id) {
            let Some(parent) = node.parent() else { break };
            if matches!(el_name(parent), None | Some("html") | Some("body")) {
                break;
            }
            match scores.get(&parent.id()) {
                Some(ps) if *ps > best_score * 0.75 => {
                    best_id = parent.id();
                    best_score = *ps;
                }
                _ => break,
            }
        }
        Some(best_id)
    }
}

// ---------------------------------------------------------------------------------------------
// metadata
// ---------------------------------------------------------------------------------------------

fn meta_content(doc: &Html, keys: &[&str]) -> String {
    for key in keys {
        for attr_name in ["property", "name", "itemprop"] {
            let Ok(sel) = Selector::parse(&format!("meta[{attr_name}=\"{key}\"]")) else {
                continue;
            };
            for el in doc.select(&sel) {
                if let Some(c) = el.value().attr("content") {
                    let c = c.trim();
                    if !c.is_empty() {
                        return c.to_string();
                    }
                }
            }
        }
    }
    String::new()
}

#[derive(Default)]
struct LdInfo {
    headline: String,
    author: String,
    published: String,
    publisher: String,
    image: String,
    description: String,
}

fn ld_name(v: &Value) -> String {
    match v {
        Value::String(s) => s.trim().to_string(),
        Value::Object(o) => o.get("name").map(ld_name).unwrap_or_default(),
        Value::Array(a) => a
            .iter()
            .map(ld_name)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    }
}

fn ld_image(v: &Value) -> String {
    match v {
        Value::String(s) => s.trim().to_string(),
        Value::Object(o) => o.get("url").map(ld_image).unwrap_or_default(),
        Value::Array(a) => a
            .iter()
            .map(ld_image)
            .find(|s| !s.is_empty())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn ld_collect(v: &Value, out: &mut LdInfo) {
    match v {
        Value::Array(a) => a.iter().for_each(|x| ld_collect(x, out)),
        Value::Object(o) => {
            if let Some(g) = o.get("@graph") {
                ld_collect(g, out);
            }
            let ty = match o.get("@type") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Array(a)) => a
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(","),
                _ => String::new(),
            };
            if ty.contains("Article")
                || ty.contains("BlogPosting")
                || ty.contains("Report")
                || ty.contains("NewsArticle")
            {
                let set = |slot: &mut String, val: String| {
                    if slot.is_empty() {
                        *slot = val;
                    }
                };
                set(
                    &mut out.headline,
                    o.get("headline").map(ld_name).unwrap_or_default(),
                );
                set(
                    &mut out.author,
                    o.get("author").map(ld_name).unwrap_or_default(),
                );
                set(
                    &mut out.published,
                    o.get("datePublished").map(ld_name).unwrap_or_default(),
                );
                set(
                    &mut out.publisher,
                    o.get("publisher").map(ld_name).unwrap_or_default(),
                );
                set(
                    &mut out.image,
                    o.get("image").map(ld_image).unwrap_or_default(),
                );
                set(
                    &mut out.description,
                    o.get("description").map(ld_name).unwrap_or_default(),
                );
            }
        }
        _ => {}
    }
}

fn ld_info(doc: &Html) -> LdInfo {
    let mut info = LdInfo::default();
    if let Ok(sel) = Selector::parse("script[type=\"application/ld+json\"]") {
        for el in doc.select(&sel) {
            let raw: String = el.text().collect();
            if let Ok(v) = serde_json::from_str::<Value>(raw.trim()) {
                ld_collect(&v, &mut info);
            }
        }
    }
    info
}

fn first_text(doc: &Html, css: &str) -> String {
    Selector::parse(css)
        .ok()
        .and_then(|s| doc.select(&s).next().map(|e| e.text().collect::<String>()))
        .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
}

fn clean_title(raw: &str, site: &str) -> String {
    let t = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    for sep in [" | ", " - ", " – ", " — ", " · ", " :: "] {
        if let Some((head, tail)) = t.rsplit_once(sep) {
            let tail_n = crate::text::normalize(tail);
            let site_n = crate::text::normalize(site);
            let looks_like_site = !site_n.is_empty()
                && (tail_n == site_n || site_n.contains(&tail_n) || tail_n.contains(&site_n));
            if looks_like_site && !head.trim().is_empty() {
                return head.trim().to_string();
            }
        }
    }
    t
}

// ---------------------------------------------------------------------------------------------
// output
// ---------------------------------------------------------------------------------------------

fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{0}'..='\u{8}'
            | '\u{B}'
            | '\u{C}'
            | '\u{E}'..='\u{1F}'
            | '\u{FFFE}'
            | '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
    out
}

fn escape_attr(s: &str) -> String {
    escape_text(s).replace('"', "&quot;")
}

fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    out
}

fn srcset_best(srcset: &str) -> Option<String> {
    let mut best: Option<(f64, String)> = None;
    for part in srcset.split(',') {
        let mut it = part.split_whitespace();
        let Some(u) = it.next() else { continue };
        let w = it
            .next()
            .and_then(|d| d.trim_end_matches(['w', 'x']).parse::<f64>().ok())
            .unwrap_or(1.0);
        if best.as_ref().is_none_or(|(bw, _)| w > *bw) {
            best = Some((w, u.to_string()));
        }
    }
    best.map(|(_, u)| u)
}

struct Emitter<'a> {
    page: &'a Page<'a>,
    base: &'a str,
    title_norm: String,
    skipped_title: bool,
    images: Vec<String>,
    max_images: usize,
}

const INLINE_TAGS: &[&str] = &[
    "a", "abbr", "b", "strong", "i", "em", "code", "kbd", "mark", "q", "s", "small", "span", "sub",
    "sup", "u", "cite", "time", "br", "img", "wbr", "del", "ins", "dfn", "font", "label", "bdi",
    "bdo", "data", "var", "samp", "big", "tt", "strike", "picture", "source",
];

impl Emitter<'_> {
    fn image_tag(&mut self, n: NodeRef<Node>) -> String {
        let width: Option<u32> =
            attr(n, "width").and_then(|w| w.trim().trim_end_matches("px").parse().ok());
        let height: Option<u32> =
            attr(n, "height").and_then(|w| w.trim().trim_end_matches("px").parse().ok());
        if width.is_some_and(|w| w <= 3) || height.is_some_and(|h| h <= 3) {
            return String::new();
        }
        let mut candidates: Vec<String> = Vec::new();
        if let Some(s) = attr(n, "src") {
            candidates.push(s.to_string());
        }
        for a in [
            "data-src",
            "data-original",
            "data-lazy-src",
            "data-orig-file",
        ] {
            if let Some(s) = attr(n, a) {
                candidates.push(s.to_string());
            }
        }
        for a in ["srcset", "data-srcset"] {
            if let Some(s) = attr(n, a).and_then(srcset_best) {
                candidates.push(s);
            }
        }
        let Some(url) = candidates.iter().find_map(|c| resolve_url(self.base, c)) else {
            return String::new();
        };
        if self.images.len() >= self.max_images && !self.images.contains(&url) {
            return String::new();
        }
        if !self.images.contains(&url) {
            self.images.push(url.clone());
        }
        format!(
            "<img src=\"{}\" alt=\"{}\"/>",
            escape_attr(&url),
            escape_attr(attr(n, "alt").unwrap_or("").trim())
        )
    }

    fn inline_element(&mut self, n: NodeRef<Node>, name: &str) -> String {
        match name {
            "br" => "<br/>".into(),
            "img" => self.image_tag(n),
            "picture" | "source" => {
                let mut s = String::new();
                for c in n.children() {
                    if el_name(c) == Some("img") {
                        s.push_str(&self.image_tag(c));
                    }
                }
                s
            }
            "a" => {
                let inner = self.inline_children(n);
                let href = attr(n, "href").and_then(|h| resolve_url(self.base, h));
                match href {
                    Some(h) if has_content(&inner) => {
                        format!("<a href=\"{}\">{inner}</a>", escape_attr(&h))
                    }
                    _ => inner,
                }
            }
            "b" | "strong" => wrap("strong", self.inline_children(n)),
            "i" | "em" | "cite" | "dfn" | "var" => wrap("em", self.inline_children(n)),
            "code" | "kbd" | "samp" | "tt" => wrap("code", self.inline_children(n)),
            "sub" => wrap("sub", self.inline_children(n)),
            "sup" => wrap("sup", self.inline_children(n)),
            "del" | "s" | "strike" => wrap("del", self.inline_children(n)),
            "q" => format!("\u{201C}{}\u{201D}", self.inline_children(n)),
            _ => self.inline_children(n),
        }
    }

    /// Phrasing content of `parent`; nested block elements are flattened with spaces.
    fn inline_children(&mut self, parent: NodeRef<Node>) -> String {
        let mut out = String::new();
        for c in parent.children() {
            match c.value() {
                Node::Text(t) => out.push_str(&escape_text(&collapse(t))),
                Node::Element(e) => {
                    if self.page.is_junk(c) {
                        continue;
                    }
                    let name = e.name();
                    if INLINE_TAGS.contains(&name) {
                        let s = self.inline_element(c, name);
                        out.push_str(&s);
                    } else {
                        out.push(' ');
                        let s = self.inline_children(c);
                        out.push_str(&s);
                        out.push(' ');
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Content of a list item or table cell: plain phrasing content when there are no blocks inside.
    fn cell_content(&mut self, n: NodeRef<Node>) -> String {
        let has_block = n.children().any(|c| {
            el_name(c).is_some_and(|name| !INLINE_TAGS.contains(&name)) && !self.page.is_junk(c)
        });
        if has_block {
            let mut inner = String::new();
            self.blocks(n, &mut inner);
            inner.trim_end().to_string()
        } else {
            self.inline_children(n).trim().to_string()
        }
    }

    fn flush(pending: &mut String, out: &mut String) {
        let t = pending.trim();
        if has_content(t) {
            out.push_str("<p>");
            out.push_str(t);
            out.push_str("</p>\n");
        }
        pending.clear();
    }

    fn blocks(&mut self, parent: NodeRef<Node>, out: &mut String) {
        let mut pending = String::new();
        for c in parent.children() {
            match c.value() {
                Node::Text(t) => pending.push_str(&escape_text(&collapse(t))),
                Node::Element(e) => {
                    if self.page.is_junk(c) {
                        continue;
                    }
                    let name = e.name();
                    if INLINE_TAGS.contains(&name) {
                        let s = self.inline_element(c, name);
                        pending.push_str(&s);
                        continue;
                    }
                    Self::flush(&mut pending, out);
                    self.block_element(c, name, out);
                }
                _ => {}
            }
        }
        Self::flush(&mut pending, out);
    }

    fn block_element(&mut self, n: NodeRef<Node>, name: &str, out: &mut String) {
        match name {
            "p" => {
                let inner = self.inline_children(n);
                let t = inner.trim();
                if has_content(t) {
                    out.push_str(&format!("<p>{t}</p>\n"));
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let inner = self.inline_children(n);
                let t = inner.trim();
                if !has_content(t) {
                    return;
                }
                if !self.skipped_title
                    && !self.title_norm.is_empty()
                    && crate::text::normalize(&strip_html(t)) == self.title_norm
                {
                    self.skipped_title = true;
                    return;
                }
                // the chapter title is the only <h1>
                let level = if name == "h1" { "h2" } else { name };
                out.push_str(&format!("<{level}>{t}</{level}>\n"));
            }
            "ul" | "ol" => {
                let mut items = String::new();
                for li in n
                    .children()
                    .filter(|c| el_name(*c) == Some("li") && !self.page.is_junk(*c))
                {
                    let inner = self.cell_content(li);
                    if has_content(&inner) {
                        items.push_str(&format!("<li>{inner}</li>\n"));
                    }
                }
                if !items.is_empty() {
                    out.push_str(&format!("<{name}>\n{items}</{name}>\n"));
                }
            }
            "li" | "dd" | "dt" => {
                // stray list item outside a list: keep the text
                self.blocks(n, out);
            }
            "dl" => {
                let mut items = String::new();
                for c in n.children() {
                    match el_name(c) {
                        Some(t @ ("dt" | "dd")) if !self.page.is_junk(c) => {
                            let inner = self.cell_content(c);
                            if has_content(&inner) {
                                items.push_str(&format!("<{t}>{inner}</{t}>\n"));
                            }
                        }
                        _ => {}
                    }
                }
                if !items.is_empty() {
                    out.push_str(&format!("<dl>\n{items}</dl>\n"));
                }
            }
            "blockquote" => {
                let mut inner = String::new();
                self.blocks(n, &mut inner);
                if has_content(&inner) {
                    out.push_str(&format!("<blockquote>\n{inner}</blockquote>\n"));
                }
            }
            "pre" => {
                let raw: String = {
                    let mut s = String::new();
                    self.page.walk_text(n, &mut |t| s.push_str(t));
                    s
                };
                if !raw.trim().is_empty() {
                    out.push_str(&format!(
                        "<pre>{}</pre>\n",
                        escape_text(raw.trim_matches('\n'))
                    ));
                }
            }
            "figure" => {
                let mut inner = String::new();
                self.blocks(n, &mut inner);
                if has_content(&inner) {
                    out.push_str(&format!("<figure>\n{inner}</figure>\n"));
                }
            }
            "figcaption" | "caption" => {
                let inner = self.inline_children(n);
                if has_content(inner.trim()) {
                    out.push_str(&format!("<p><em>{}</em></p>\n", inner.trim()));
                }
            }
            "hr" => out.push_str("<hr/>\n"),
            "table" => self.table(n, out),
            "address" | "center" | "div" | "section" | "article" | "main" | "header" | "body"
            | "html" | "details" | "summary" | "fieldset" | "legend" | "tbody" | "thead"
            | "tfoot" | "tr" | "td" | "th" | "colgroup" | "col" | "hgroup" | "span"
            | "figure-group" | "body-text" => self.blocks(n, out),
            _ => self.blocks(n, out),
        }
    }

    fn table(&mut self, n: NodeRef<Node>, out: &mut String) {
        let cells: Vec<NodeRef<Node>> = n
            .descendants()
            .filter(|d| matches!(el_name(*d), Some("td" | "th")) && !self.page.in_junk(*d))
            .collect();
        let rows: Vec<NodeRef<Node>> = n
            .descendants()
            .filter(|d| el_name(*d) == Some("tr") && !self.page.in_junk(*d))
            .collect();
        // layout tables (one cell) are just containers
        if cells.len() <= 1 {
            self.blocks(n, out);
            return;
        }
        let mut body = String::new();
        for r in rows {
            let mut row = String::new();
            for c in r
                .children()
                .filter(|c| matches!(el_name(*c), Some("td" | "th")))
            {
                let tag = el_name(c).unwrap_or("td");
                let inner = self.cell_content(c);
                row.push_str(&format!("<{tag}>{inner}</{tag}>"));
            }
            if !row.is_empty() {
                body.push_str(&format!("<tr>{row}</tr>\n"));
            }
        }
        if !body.is_empty() {
            out.push_str(&format!("<table>\n{body}</table>\n"));
        }
    }
}

fn wrap(tag: &str, inner: String) -> String {
    if has_content(&inner) {
        format!("<{tag}>{inner}</{tag}>")
    } else {
        inner
    }
}

/// Whether an XHTML snippet shows anything: text or an image.
fn has_content(s: &str) -> bool {
    s.contains("<img")
        || s.chars().any(|c| !c.is_whitespace() && !c.is_control()) && {
            // strip tags and look for visible characters
            let mut in_tag = false;
            let mut seen = false;
            for c in s.chars() {
                match c {
                    '<' => in_tag = true,
                    '>' => in_tag = false,
                    c if !in_tag && !c.is_whitespace() => {
                        seen = true;
                        break;
                    }
                    _ => {}
                }
            }
            seen
        }
}

fn visible_len(xhtml: &str) -> usize {
    strip_html(xhtml).chars().count()
}

/// Cleaned XHTML of a markup fragment (feed entry bodies etc.) plus the images it uses.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fragment {
    pub content: String,
    pub images: Vec<String>,
    pub text_len: usize,
}

/// Cleans an HTML fragment with the same whitelist as [`extract`], without searching for a container.
pub fn clean_fragment(html: &str, base_url: &str) -> Fragment {
    let doc = bookx_core::util::parse_html_fragment(html);
    let page = match Page::new(&doc, &[]) {
        Ok(p) => p,
        Err(_) => return Fragment::default(),
    };
    let mut em = Emitter {
        page: &page,
        base: base_url,
        title_norm: String::new(),
        skipped_title: true,
        images: Vec::new(),
        max_images: 100,
    };
    let mut out = String::new();
    em.blocks(doc.tree.root(), &mut out);
    Fragment {
        text_len: visible_len(&out),
        images: em.images,
        content: out,
    }
}

/// Like [`extract`] with selectors and limits.
pub fn extract_with(html: &str, base_url: &str, opts: &ExtractOptions) -> Result<Article> {
    let doc = bookx_core::util::parse_html_document(html);
    let page = Page::new(&doc, &opts.remove)?;

    let ld = ld_info(&doc);
    let site = {
        let s = meta_content(&doc, &["og:site_name", "application-name"]);
        if s.is_empty() {
            ld.publisher.clone()
        } else {
            s
        }
    };
    let raw_title = [
        meta_content(&doc, &["og:title", "twitter:title"]),
        ld.headline.clone(),
        first_text(&doc, "head > title, title"),
        first_text(&doc, "h1"),
    ]
    .into_iter()
    .find(|t| !t.trim().is_empty())
    .unwrap_or_default();
    let title = clean_title(&raw_title, &site);
    let author = {
        let a = meta_content(
            &doc,
            &[
                "author",
                "article:author",
                "twitter:creator",
                "parsely-author",
                "dc.creator",
            ],
        );
        let a = if a.is_empty() { ld.author.clone() } else { a };
        // URLs and @handles are not names
        if a.starts_with("http") || a.starts_with('@') {
            ld.author.clone()
        } else {
            a
        }
    };
    let published = {
        let p = meta_content(
            &doc,
            &[
                "article:published_time",
                "datePublished",
                "og:article:published_time",
                "date",
                "dc.date",
                "pubdate",
            ],
        );
        let p = if p.is_empty() {
            ld.published.clone()
        } else {
            p
        };
        let p = if p.is_empty() {
            Selector::parse("time[datetime]")
                .ok()
                .and_then(|s| {
                    doc.select(&s)
                        .next()
                        .and_then(|e| e.value().attr("datetime").map(String::from))
                })
                .unwrap_or_default()
        } else {
            p
        };
        normalize_date(&p)
    };
    let description = {
        let d = meta_content(
            &doc,
            &["og:description", "description", "twitter:description"],
        );
        if d.is_empty() {
            ld.description.clone()
        } else {
            d
        }
    };
    let lead_image = {
        let raw = meta_content(
            &doc,
            &[
                "og:image",
                "og:image:url",
                "twitter:image",
                "twitter:image:src",
            ],
        );
        let raw = if raw.is_empty() {
            ld.image.clone()
        } else {
            raw
        };
        let raw = if raw.is_empty() {
            Selector::parse("link[rel=image_src]")
                .ok()
                .and_then(|s| {
                    doc.select(&s)
                        .next()
                        .and_then(|e| e.value().attr("href").map(String::from))
                })
                .unwrap_or_default()
        } else {
            raw
        };
        resolve_url(base_url, &raw)
    };
    let language = Selector::parse("html")
        .ok()
        .and_then(|s| {
            doc.select(&s)
                .next()
                .and_then(|e| e.value().attr("lang").map(String::from))
        })
        .map(|l| l.split(['-', '_']).next().unwrap_or("").to_lowercase())
        .unwrap_or_default();

    let mut em = Emitter {
        page: &page,
        base: base_url,
        title_norm: crate::text::normalize(&title),
        skipped_title: false,
        images: Vec::new(),
        max_images: opts.max_images,
    };
    let mut content = String::new();
    if !opts.keep.is_empty() {
        let mut found: Vec<ElementRef> = Vec::new();
        for sel in &opts.keep {
            let s = Selector::parse(sel)
                .map_err(|e| anyhow::anyhow!("invalid keep selector `{sel}`: {e}"))?;
            for el in doc.select(&s) {
                if !page.in_junk(*el) && !found.iter().any(|f| f.id() == el.id()) {
                    found.push(el);
                }
            }
        }
        // document order
        let order: HashMap<NodeId, usize> = doc
            .tree
            .nodes()
            .enumerate()
            .map(|(i, n)| (n.id(), i))
            .collect();
        found.sort_by_key(|e| order.get(&e.id()).copied().unwrap_or(0));
        // an element inside another kept element would be emitted twice
        let ids: Vec<NodeId> = found.iter().map(|e| e.id()).collect();
        for el in &found {
            if el.ancestors().any(|a| ids.contains(&a.id())) {
                continue;
            }
            em.block_element(**el, el.value().name(), &mut content);
        }
    } else {
        let container = page
            .pick_container()
            .or_else(|| {
                Selector::parse("body")
                    .ok()
                    .and_then(|s| doc.select(&s).next().map(|e| e.id()))
            })
            .and_then(|id| doc.tree.get(id));
        if let Some(c) = container {
            em.blocks(c, &mut content);
        }
    }
    let text_len = visible_len(&content);
    if text_len < opts.min_text_len.max(1) {
        bail!("no readable content found on {base_url}");
    }
    Ok(Article {
        url: base_url.to_string(),
        title,
        author,
        site,
        published,
        description,
        language,
        lead_image,
        content,
        images: em.images,
        text_len,
    })
}

// ---------------------------------------------------------------------------------------------
// images and EPUB
// ---------------------------------------------------------------------------------------------

/// An image downloaded for an article.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedImage {
    pub url: String,
    pub data: Vec<u8>,
}

/// Limits for [`fetch_images`].
#[derive(Debug, Clone, Copy)]
pub struct ImageLimits {
    pub max_bytes_each: usize,
    pub max_count: usize,
}

impl Default for ImageLimits {
    fn default() -> Self {
        ImageLimits {
            max_bytes_each: 2 * 1024 * 1024,
            max_count: 60,
        }
    }
}

/// `(extension, media type)` of image data by magic bytes: JPEG, PNG, GIF, WebP, SVG.
pub fn sniff_image(data: &[u8]) -> Option<(&'static str, &'static str)> {
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(("jpg", "image/jpeg"))
    } else if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("png", "image/png"))
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        Some(("gif", "image/gif"))
    } else if data.len() > 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some(("webp", "image/webp"))
    } else if data.len() > 5 && {
        let head = String::from_utf8_lossy(&data[..data.len().min(512)]).to_lowercase();
        head.contains("<svg")
    } {
        Some(("svg", "image/svg+xml"))
    } else {
        None
    }
}

/// Downloads images, skipping failures, non-images and oversized files.
pub fn fetch_images(
    fetcher: &dyn Fetcher,
    urls: &[String],
    limits: &ImageLimits,
) -> Vec<FetchedImage> {
    let mut out = Vec::new();
    for u in urls {
        if out.len() >= limits.max_count {
            break;
        }
        if let Ok(data) = fetcher.get(u, &[]) {
            if data.len() <= limits.max_bytes_each && sniff_image(&data).is_some() {
                out.push(FetchedImage {
                    url: u.clone(),
                    data,
                });
            }
        }
    }
    out
}

/// Replaces `<img src="URL" …/>` produced by this module: fetched images get `images/<name>`
/// from `local` (url → relative path), the others are removed.
pub fn rewrite_images(content: &str, local: &HashMap<String, String>) -> String {
    static R: OnceLock<Regex> = OnceLock::new();
    let re = R
        .get_or_init(|| Regex::new(r#"<img src="([^"]*)" alt="([^"]*)"/>"#).expect("static regex"));
    re.replace_all(content, |c: &regex::Captures| {
        let url = c[1].replace("&amp;", "&");
        match local.get(&url) {
            Some(path) => format!("<img src=\"{}\" alt=\"{}\"/>", escape_attr(path), &c[2]),
            None => String::new(),
        }
    })
    .into_owned()
}

/// Adds image files below `OEBPS/images/` and registers them in the manifest; returns url → relative path.
pub fn add_images_to_epub(
    archive: &mut Archive,
    images: &[FetchedImage],
    prefix: &str,
) -> Result<HashMap<String, String>> {
    let pkg = Package::parse(archive)?;
    let dir = pkg.base_dir().to_string();
    let mut map = HashMap::new();
    for (i, img) in images.iter().enumerate() {
        let Some((ext, media)) = sniff_image(&img.data) else {
            continue;
        };
        if map.contains_key(&img.url) {
            continue;
        }
        let name = format!("{prefix}{}.{ext}", i + 1);
        let rel = format!("images/{name}");
        let abs = if dir.is_empty() {
            rel.clone()
        } else {
            format!("{dir}/{rel}")
        };
        archive.set(&abs, img.data.clone());
        pkg.add_manifest_item(archive, &format!("{prefix}{}", i + 1), &abs, media, "")?;
        map.insert(img.url.clone(), rel);
    }
    Ok(map)
}

/// `create_epub` writes only the basic fields; this stores the complete metadata (date, tags, …).
pub(crate) fn write_full_metadata(archive: &mut Archive, md: &Metadata) -> Result<()> {
    let mut pkg = Package::parse(archive)?;
    pkg.metadata.published = md.published.clone();
    pkg.metadata.description = md.description.clone();
    pkg.metadata.tags = md.tags.clone();
    pkg.metadata.series = md.series.clone();
    pkg.metadata.series_index = md.series_index;
    pkg.metadata.publisher = md.publisher.clone();
    pkg.metadata.language = md.language.clone();
    pkg.metadata.rights = md.rights.clone();
    pkg.write_metadata(archive)
}

fn byline(a: &Article) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !a.author.is_empty() {
        parts.push(format!("By {}", escape_text(&a.author)));
    }
    if !a.site.is_empty() {
        parts.push(escape_text(&a.site));
    }
    if !a.published.is_empty() {
        parts.push(escape_text(&a.published));
    }
    parts.join(" \u{00B7} ")
}

/// Packs an article as a one-chapter EPUB. `images` are the downloaded files (see [`fetch_images`]);
/// images that were not fetched disappear from the text. The lead image becomes the cover when
/// it is a JPEG or PNG.
pub fn article_to_epub(article: &Article, images: &[FetchedImage]) -> Result<Archive> {
    let lead = article
        .lead_image
        .as_ref()
        .and_then(|u| images.iter().find(|i| &i.url == u))
        .and_then(|i| match sniff_image(&i.data) {
            Some((ext @ ("jpg" | "png"), _)) => Some((i.data.clone(), ext)),
            _ => None,
        });
    let mut md = Metadata {
        title: if article.title.is_empty() {
            article.url.clone()
        } else {
            article.title.clone()
        },
        authors: if !article.author.is_empty() {
            vec![article.author.clone()]
        } else if !article.site.is_empty() {
            vec![article.site.clone()]
        } else {
            vec![]
        },
        publisher: article.site.clone(),
        published: article.published.clone(),
        description: article.description.clone(),
        language: article.language.clone(),
        tags: vec!["Article".into()],
        ..Default::default()
    };
    if !article.url.is_empty() {
        md.identifiers.push(Identifier {
            scheme: "url".into(),
            value: article.url.clone(),
            xml_id: None,
        });
    }
    // placeholder body first: image paths depend on the manifest naming
    let chapter = Chapter {
        title: md.title.clone(),
        body: String::new(),
    };
    let mut archive = create_epub(&md, &[chapter], lead.as_ref().map(|(d, e)| (d.clone(), *e)));
    let content_images: Vec<FetchedImage> = images
        .iter()
        .filter(|i| article.images.contains(&i.url))
        .cloned()
        .collect();
    write_full_metadata(&mut archive, &md)?;
    let local = add_images_to_epub(&mut archive, &content_images, "img")?;
    let mut body = format!("<h1>{}</h1>\n", escape_text(&md.title));
    let by = byline(article);
    if !by.is_empty() {
        body.push_str(&format!("<p class=\"byline\"><em>{by}</em></p>\n"));
    }
    if !article.url.is_empty() {
        body.push_str(&format!(
            "<p class=\"source\"><a href=\"{}\">{}</a></p>\n",
            escape_attr(&article.url),
            escape_text(&article.url)
        ));
    }
    body.push_str(&rewrite_images(&article.content, &local));
    let pkg = Package::parse(&archive)?;
    let path = format!("{}/chapter-1.xhtml", pkg.base_dir());
    let old = archive
        .get_string(&path)
        .ok_or_else(|| anyhow::anyhow!("chapter file missing in generated EPUB"))?;
    let new = old.replacen("<body>\n</body>", &format!("<body>\n{body}</body>"), 1);
    if new == old {
        bail!("could not place the article text into the chapter");
    }
    archive.set_string(&path, &new);
    Ok(archive)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::{MockFetcher, MockReply};
    use bookx_core::archive::Archive as Arc2;
    use bookx_core::book::OpenBook;

    fn parse_xml(s: &str) -> roxmltree::Document<'_> {
        let opts = roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        };
        roxmltree::Document::parse_with_options(s, opts).unwrap_or_else(|e| panic!("{e}\n{s}"))
    }

    const BASE: &str = "https://blog.example.com/2024/03/post/";

    fn blog_page() -> String {
        r##"<!DOCTYPE html>
<html lang="en-GB"><head>
<meta charset="utf-8">
<title>Why Rust is Fun | Example Blog</title>
<meta property="og:title" content="Why Rust is Fun">
<meta property="og:site_name" content="Example Blog">
<meta property="og:image" content="/img/lead.jpg">
<meta property="og:description" content="A short love letter.">
<meta name="author" content="Ada Lovelace">
<meta property="article:published_time" content="2024-03-05T10:00:00+00:00">
<script>var tracking = "<p>not content</p>";</script>
<style>.x{display:none}</style>
</head><body class="has-sidebar home">
<nav class="navbar"><a href="/">Home</a><a href="/about">About</a></nav>
<div id="cookie-banner">We use cookies. <button>OK</button></div>
<div class="layout"><article class="post">
<header><h1>Why Rust is Fun</h1><p class="meta">posted by Ada</p></header>
<p>Rust is a systems language, fast, safe, and friendly. It gives you control, without giving up safety, which is rare.</p>
<p>Here is a <a href="../other/">relative link</a> and an <a href="https://example.org/abs?x=1&y=2">absolute one</a>, plus <b>bold</b> and <i>italic</i> text, and some code: <code>let x = 5;</code>.</p>
<figure><img src="images/a.png" alt="Diagram &amp; more" width="600"><figcaption>A diagram</figcaption></figure>
<img src="//cdn.example.com/b.jpg" alt="">
<img src="data:image/gif;base64,R0lGOD" alt="inline">
<img src="/pixel.gif" width="1" height="1" alt="">
<h2>Details</h2>
<ul><li>First point is long enough to count as text</li><li>Second <a href="#note">note</a></li></ul>
<pre>fn main() {
    println!("hi &amp; bye");
}</pre>
<blockquote><p>Quoted wisdom lives here, for everyone to read.</p></blockquote>
<form action="/subscribe"><input name="email"><button>Subscribe</button></form>
<div class="share-buttons"><a href="https://twitter.com">Tweet</a></div>
<div class="comments"><p>First! This is a comment, with commas, and text, and more text to be long.</p></div>
</article>
<aside class="sidebar"><h3>Popular</h3><ul><li><a href="/p1">Post one</a></li></ul></aside>
</div>
<footer><p>Copyright 2024 Example Blog. All rights reserved, forever and ever.</p></footer>
<script src="/app.js"></script>
</body></html>"##
            .to_string()
    }

    #[test]
    fn resolve_url_cases() {
        let b = "https://a.com/x/y/page.html?q=1";
        assert_eq!(
            resolve_url(b, "z.png").as_deref(),
            Some("https://a.com/x/y/z.png")
        );
        assert_eq!(
            resolve_url(b, "../z.png").as_deref(),
            Some("https://a.com/x/z.png")
        );
        assert_eq!(
            resolve_url(b, "/z.png").as_deref(),
            Some("https://a.com/z.png")
        );
        assert_eq!(
            resolve_url(b, "//cdn.com/z.png").as_deref(),
            Some("https://cdn.com/z.png")
        );
        assert_eq!(
            resolve_url(b, "http://b.org/p").as_deref(),
            Some("http://b.org/p")
        );
        assert_eq!(
            resolve_url(b, "?page=2").as_deref(),
            Some("https://a.com/x/y/page.html?page=2")
        );
        assert_eq!(
            resolve_url(b, "  spaced.png ").as_deref(),
            Some("https://a.com/x/y/spaced.png")
        );
        for bad in [
            "",
            "#top",
            "javascript:void(0)",
            "data:image/png;base64,xx",
            "mailto:a@b.c",
            "tel:123",
            "ftp://x/y",
        ] {
            assert_eq!(resolve_url(b, bad), None, "{bad}");
        }
        assert_eq!(
            resolve_url("not a base", "https://ok.com/a").as_deref(),
            Some("https://ok.com/a")
        );
        assert_eq!(resolve_url("not a base", "relative"), None);
    }

    #[test]
    fn blog_metadata() {
        let a = extract(&blog_page(), BASE).unwrap();
        assert_eq!(a.title, "Why Rust is Fun");
        assert_eq!(a.author, "Ada Lovelace");
        assert_eq!(a.site, "Example Blog");
        assert_eq!(a.published, "2024-03-05");
        assert_eq!(a.description, "A short love letter.");
        assert_eq!(a.language, "en");
        assert_eq!(
            a.lead_image.as_deref(),
            Some("https://blog.example.com/img/lead.jpg")
        );
        assert_eq!(a.url, BASE);
    }

    #[test]
    fn blog_content_is_clean() {
        let a = extract(&blog_page(), BASE).unwrap();
        let c = &a.content;
        assert!(c.contains("Rust is a systems language"));
        assert!(c.contains("<h2>Details</h2>"));
        assert!(c.contains("<li>First point"));
        assert!(c.contains("<blockquote>"));
        assert!(
            c.contains("<pre>fn main() {\n    println!(\"hi &amp; bye\");\n}</pre>"),
            "{c}"
        );
        assert!(
            c.contains("<strong>bold</strong>")
                && c.contains("<em>italic</em>")
                && c.contains("<code>let x = 5;</code>")
        );
        for bad in [
            "Subscribe",
            "Tweet",
            "First!",
            "Popular",
            "Copyright",
            "cookies",
            "not content",
            "Home",
        ] {
            assert!(!c.contains(bad), "`{bad}` leaked into:\n{c}");
        }
        assert!(!c.contains("<h1>"), "title heading removed: {c}");
        assert!(
            !c.contains("<script")
                && !c.contains("<form")
                && !c.contains("<nav")
                && !c.contains("<div")
        );
    }

    #[test]
    fn blog_links_and_images_are_resolved() {
        let a = extract(&blog_page(), BASE).unwrap();
        assert!(
            a.content
                .contains("<a href=\"https://blog.example.com/2024/03/other/\">relative link</a>"),
            "{}",
            a.content
        );
        assert!(a
            .content
            .contains("<a href=\"https://example.org/abs?x=1&amp;y=2\">absolute one</a>"));
        assert!(
            a.content.contains("Second note"),
            "fragment links are unwrapped"
        );
        assert_eq!(
            a.images,
            vec![
                "https://blog.example.com/2024/03/post/images/a.png",
                "https://cdn.example.com/b.jpg"
            ]
        );
        assert!(a.content.contains("alt=\"Diagram &amp; more\""));
        assert!(!a.content.contains("pixel.gif") && !a.content.contains("data:"));
        assert_eq!(
            a.all_image_urls()[0],
            "https://blog.example.com/img/lead.jpg"
        );
        assert_eq!(a.all_image_urls().len(), 3);
    }

    #[test]
    fn output_is_well_formed_xhtml() {
        let a = extract(&blog_page(), BASE).unwrap();
        let doc = format!("<root>{}</root>", a.content);
        roxmltree::Document::parse(&doc).unwrap_or_else(|e| panic!("{e}\n{doc}"));
        assert_eq!(a.text_len, visible_len(&a.content));
        assert!(a.text_len > 300);
    }

    #[test]
    fn news_page_with_schema_org_metadata() {
        let html = r#"<html><head>
<title>City council approves budget - The Daily News</title>
<script type="application/ld+json">
{"@context":"https://schema.org","@graph":[{"@type":"WebSite","name":"x"},
 {"@type":["NewsArticle"],"headline":"City council approves budget","author":[{"@type":"Person","name":"Jo Reporter"},{"name":"Sam Editor"}],
  "datePublished":"2024-02-29T08:30:00Z","publisher":{"@type":"Organization","name":"The Daily News"},
  "image":["https://news.example.com/hero.jpg"],"description":"Budget vote."}]}
</script></head><body>
<div class="top-menu"><a href="/">Home</a></div>
<main><div itemprop="articleBody">
<p>The city council voted 7-2 on Tuesday night to approve the annual budget, ending weeks of debate, protests, and procedural delays.</p>
<p>Mayor Smith said the plan, which raises spending on transit and parks, reflects the priorities of residents, businesses, and workers alike.</p>
</div></main></body></html>"#;
        let a = extract(html, "https://news.example.com/a/1").unwrap();
        assert_eq!(a.title, "City council approves budget");
        assert_eq!(a.author, "Jo Reporter, Sam Editor");
        assert_eq!(a.site, "The Daily News");
        assert_eq!(a.published, "2024-02-29");
        assert_eq!(
            a.lead_image.as_deref(),
            Some("https://news.example.com/hero.jpg")
        );
        assert!(a.content.contains("voted 7-2"));
        assert!(!a.content.contains("Home"));
    }

    #[test]
    fn title_suffix_removed_only_for_site_name() {
        let html = "<html><head><title>Rust - The Book | Example Site</title><meta property=\"og:site_name\" content=\"Example Site\"></head><body><article><p>".to_string() + &"text here, ".repeat(40) + "</p></article></body></html>";
        let a = extract(&html, "https://x.com/").unwrap();
        assert_eq!(a.title, "Rust - The Book");
        assert_eq!(clean_title("A - B", ""), "A - B");
        assert_eq!(clean_title("A | Site", "Site"), "A");
        assert_eq!(clean_title("  spaced   out  ", ""), "spaced out");
    }

    #[test]
    fn wiki_like_page_without_article_tag() {
        let html = r#"<html><head><title>Ferris (mascot) - Wikipedia</title></head><body>
<div id="mw-navigation"><div class="portal"><ul><li><a href="/w/main">Main page</a></li><li><a href="/w/random">Random</a></li></ul></div></div>
<div id="content"><h1 id="firstHeading">Ferris (mascot)</h1>
<div id="bodyContent"><div id="mw-content-text">
<p>Ferris is the unofficial mascot of the Rust programming language, a crab, orange, and friendly.</p>
<p>The name is a pun on the word ferrous, relating to iron and rust, which is the language's namesake, as many fans note.</p>
<h2>History</h2>
<p>The mascot appeared in the early community, drawn by Karen Rustad Tölva, and was released into the public domain, widely.</p>
<table class="infobox"><tr><th>Created by</th><td>Karen</td></tr><tr><th>Year</th><td>2015</td></tr></table>
</div></div></div>
<div id="footer"><ul><li>This page was last edited on 1 March 2024.</li></ul></div>
</body></html>"#;
        let a = extract(html, "https://en.wikipedia.org/wiki/Ferris").unwrap();
        assert_eq!(
            a.title, "Ferris (mascot) - Wikipedia",
            "no site name known, nothing is cut"
        );
        assert!(a.content.contains("unofficial mascot"));
        assert!(a.content.contains("<h2>History</h2>"));
        assert!(
            a.content.contains("<table>")
                && a.content.contains("<th>Created by</th>")
                && a.content.contains("<td>2015</td>")
        );
        assert!(!a.content.contains("Main page") && !a.content.contains("last edited"));
    }

    #[test]
    fn page_without_semantic_tags_uses_text_density() {
        let long = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore, et dolore magna aliqua.";
        let html = format!(
            "<html><body><div id=\"wrap\"><div id=\"links\"><a href=\"/1\">One</a> <a href=\"/2\">Two</a> <a href=\"/3\">Three</a> <a href=\"/4\">Four link text here to be somewhat long enough</a></div>\
             <div id=\"text\"><p>{long}</p><p>{long}</p><p>{long}</p></div>\
             <div id=\"related-posts\"><p>{long}</p></div></div></body></html>"
        );
        let a = extract(&html, "https://x.org/").unwrap();
        assert_eq!(
            a.content.matches("Lorem ipsum").count(),
            3,
            "related block dropped: {}",
            a.content
        );
        assert!(!a.content.contains("Four link"));
    }

    #[test]
    fn content_split_over_sibling_blocks_is_joined() {
        let p = "A reasonably long sentence, with commas, that counts as real article text for scoring.";
        let html = format!(
            "<html><body><div id=\"story\"><div class=\"part\"><p>{p}</p><p>{p}</p></div><div class=\"part\"><p>{p}</p><p>{p}</p></div><div class=\"part\"><p>{p}</p><p>{p}</p></div></div></body></html>"
        );
        let a = extract(&html, "https://x.org/").unwrap();
        assert_eq!(
            a.content.matches("reasonably long").count(),
            6,
            "{}",
            a.content
        );
    }

    #[test]
    fn link_heavy_blocks_lose_against_prose() {
        let prose = "This paragraph is real prose, written for people, with commas, clauses, and a decent length to score.";
        let links: String = (0..8).map(|i| format!("<p><a href=\"/l{i}\">A link list entry that is long enough to pass the length filter {i}</a></p>")).collect();
        let html = format!("<html><body><div id=\"list\">{links}</div><div id=\"prose\"><p>{prose}</p><p>{prose}</p></div></body></html>");
        let a = extract(&html, "https://x.org/").unwrap();
        assert!(
            a.content.contains("real prose") && !a.content.contains("link list entry"),
            "{}",
            a.content
        );
    }

    #[test]
    fn plain_body_fallback_when_no_paragraphs() {
        let a = extract(
            "<html><body>Just some text without any markup at all.</body></html>",
            "https://x.org/",
        )
        .unwrap();
        assert!(a.content.contains("Just some text"));
        assert!(a.content.starts_with("<p>"));
    }

    #[test]
    fn nothing_readable_is_an_error() {
        for html in [
            "",
            "<html><body></body></html>",
            "<html><body><script>x()</script><nav>menu</nav></body></html>",
            "<html><body>  \n </body></html>",
        ] {
            let e = extract(html, "https://x.org/").unwrap_err();
            assert!(e.to_string().contains("no readable content"), "{html}: {e}");
        }
        let opts = ExtractOptions {
            min_text_len: 1000,
            ..Default::default()
        };
        assert!(extract_with(
            "<html><body><p>short text here but fine</p></body></html>",
            "https://x.org/",
            &opts
        )
        .is_err());
    }

    #[test]
    fn malformed_html_still_extracts() {
        let html = "<html><body><div><p>Unclosed paragraph one, with text <b>bold <i>nested never closed\n<p>Second paragraph, also unclosed, with enough text to be taken seriously\n<ul><li>item one<li>item two</ul><table><tr><td>cell<tr><td>another</table></div>";
        let a = extract(html, "https://x.org/").unwrap();
        assert!(a.content.contains("Unclosed paragraph one"));
        assert!(a.content.contains("Second paragraph"));
        assert!(
            a.content.contains("<li>")
                && a.content.contains("item one")
                && a.content.contains("item two")
        );
        parse_xml(&format!("<root>{}</root>", a.content));
    }

    #[test]
    fn hostile_content_is_neutralised() {
        let html = r#"<html><body><article><p onclick="evil()">Safe text, with commas, and <script>alert(1)</script>more text in the paragraph to be long enough.</p>
<p><a href="javascript:alert(1)">bad link text here</a> and <img src="javascript:alert(1)" alt="x"> <iframe src="http://evil"></iframe> <object data="x"></object> end of paragraph text.</p>
<p style="display:none">hidden text in a paragraph that should vanish entirely from the output</p>
<div hidden>also hidden text that should vanish entirely from the output please</div>
<p>Ünïcödé 日本語 &lt;tag&gt; &amp; \u{0007} control chars, handled; fine, with enough text to count as an article.</p></article></body></html>"#;
        let a = extract(html, "https://x.org/").unwrap();
        for bad in [
            "onclick",
            "alert",
            "javascript",
            "iframe",
            "object",
            "hidden text",
            "evil",
        ] {
            assert!(!a.content.contains(bad), "`{bad}` in {}", a.content);
        }
        assert!(a.content.contains("Ünïcödé 日本語 &lt;tag&gt; &amp;"));
        assert!(a.content.contains("bad link text here"));
        parse_xml(&format!("<root>{}</root>", a.content));
    }

    #[test]
    fn huge_page_is_handled() {
        let mut html = String::from("<html><head><title>Huge</title></head><body>");
        for i in 0..4000 {
            html.push_str(&format!(
                "<div class=\"row\"><p>Paragraph number {i}, filled with filler words, commas, and more filler words to reach a sensible length.</p></div>"
            ));
        }
        html.push_str("</body></html>");
        assert!(html.len() > 500_000);
        let a = extract(&html, "https://x.org/").unwrap();
        assert!(a.content.contains("Paragraph number 3999"));
        assert!(a.content.contains("Paragraph number 0,"));
        assert!(a.text_len > 400_000);
    }

    #[test]
    fn deeply_nested_page_does_not_overflow() {
        let depth = 400;
        let html = format!("<html><body>{}<p>Deep text, with commas, that is long enough to matter for scoring purposes.</p>{}</body></html>", "<div>".repeat(depth), "</div>".repeat(depth));
        let a = extract(&html, "https://x.org/").unwrap();
        assert!(a.content.contains("Deep text"));
    }

    #[test]
    fn keep_and_remove_selectors() {
        let html = r#"<html><body><div id="a"><p>Alpha paragraph, long enough to matter, with commas, and words.</p></div>
<div id="b"><p>Beta paragraph, long enough to matter, with commas, and words.</p><p class="ad-slot-x ad">Buy now! sponsored text paragraph that is long enough to see.</p><span class="junkme">REMOVEME</span></div>
<div id="c"><p>Gamma paragraph, long enough to matter, with commas, and words.</p></div></body></html>"#;
        let keep = ExtractOptions {
            keep: vec!["#c".into(), "#a".into()],
            ..Default::default()
        };
        let a = extract_with(html, "https://x.org/", &keep).unwrap();
        let (ia, ic) = (
            a.content.find("Alpha").unwrap(),
            a.content.find("Gamma").unwrap(),
        );
        assert!(ia < ic, "document order, not selector order");
        assert!(!a.content.contains("Beta"));
        let rm = ExtractOptions {
            remove: vec![".junkme".into(), "#a".into()],
            ..Default::default()
        };
        let b = extract_with(html, "https://x.org/", &rm).unwrap();
        assert!(!b.content.contains("REMOVEME") && !b.content.contains("Alpha"));
        let nested = ExtractOptions {
            keep: vec!["#b".into(), "#b p".into()],
            ..Default::default()
        };
        let c = extract_with(html, "https://x.org/", &nested).unwrap();
        assert_eq!(
            c.content.matches("Beta paragraph").count(),
            1,
            "no double emission"
        );
    }

    #[test]
    fn invalid_selectors_are_errors() {
        let keep = ExtractOptions {
            keep: vec!["div[".into()],
            ..Default::default()
        };
        assert!(extract_with("<p>x</p>", "https://x.org/", &keep)
            .unwrap_err()
            .to_string()
            .contains("invalid keep selector"));
        let rm = ExtractOptions {
            remove: vec!["[[".into()],
            ..Default::default()
        };
        assert!(extract_with("<p>x</p>", "https://x.org/", &rm)
            .unwrap_err()
            .to_string()
            .contains("invalid remove selector"));
    }

    #[test]
    fn max_images_is_respected() {
        let imgs: String = (0..10)
            .map(|i| format!("<img src=\"/i{i}.jpg\" alt=\"\">"))
            .collect();
        let html = format!("<html><body><article><p>Some text, long enough to be here for the article test case.</p>{imgs}</article></body></html>");
        let opts = ExtractOptions {
            max_images: 3,
            ..Default::default()
        };
        let a = extract_with(&html, "https://x.org/", &opts).unwrap();
        assert_eq!(a.images.len(), 3);
        assert_eq!(a.content.matches("<img").count(), 3);
    }

    #[test]
    fn lazy_loaded_and_srcset_images() {
        let html = r#"<html><body><article><p>Some text, long enough to be here for the article test case.</p>
<img data-src="/lazy.jpg" src="data:image/gif;base64,AAAA" alt="lazy">
<img srcset="/small.jpg 300w, /big.jpg 1200w, /mid.jpg 600w" alt="s">
<picture><source srcset="/x.webp"><img src="/pic.jpg" alt="p"></picture></article></body></html>"#;
        let a = extract(html, "https://x.org/").unwrap();
        assert_eq!(
            a.images,
            vec![
                "https://x.org/lazy.jpg",
                "https://x.org/big.jpg",
                "https://x.org/pic.jpg"
            ]
        );
    }

    #[test]
    fn clean_fragment_basics() {
        let f = clean_fragment("<p>Hello <b>world</b></p><script>x()</script><img src=\"/a.png\" alt=\"A\"><div>loose</div>", "https://x.org/post/");
        assert!(f.content.contains("<p>Hello <strong>world</strong></p>"));
        assert!(f.content.contains("<p>loose</p>"));
        assert!(!f.content.contains("script"));
        assert_eq!(f.images, vec!["https://x.org/a.png"]);
        assert_eq!(f.text_len, "Hello world loose".len(), "{}", f.content);
        assert_eq!(clean_fragment("", "https://x.org/"), Fragment::default());
    }

    #[test]
    fn sniffing() {
        assert_eq!(
            sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some(("jpg", "image/jpeg"))
        );
        assert_eq!(
            sniff_image(b"\x89PNG\r\n\x1a\nrest"),
            Some(("png", "image/png"))
        );
        assert_eq!(sniff_image(b"GIF89a.."), Some(("gif", "image/gif")));
        assert_eq!(
            sniff_image(b"RIFF\x00\x00\x00\x00WEBPVP8 "),
            Some(("webp", "image/webp"))
        );
        assert_eq!(
            sniff_image(b"<?xml version='1.0'?><svg xmlns=''/>"),
            Some(("svg", "image/svg+xml"))
        );
        assert_eq!(sniff_image(b"<html>"), None);
        assert_eq!(sniff_image(b""), None);
    }

    fn png_bytes() -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend_from_slice(&[0; 64]);
        v
    }

    #[test]
    fn fetch_images_filters_and_limits() {
        let f = MockFetcher::new()
            .with_body("http://i/ok.png", png_bytes())
            .with_body("http://i/html", "<html>nope</html>")
            .on("http://i/down", MockReply::Status(500))
            .with_body("http://i/big.png", {
                let mut b = png_bytes();
                b.resize(5000, 0);
                b
            })
            .with_body("http://i/ok2.png", png_bytes());
        let urls: Vec<String> = ["ok.png", "html", "down", "big.png", "ok2.png"]
            .iter()
            .map(|s| format!("http://i/{s}"))
            .collect();
        let got = fetch_images(
            &f,
            &urls,
            &ImageLimits {
                max_bytes_each: 1000,
                max_count: 10,
            },
        );
        assert_eq!(
            got.iter().map(|i| i.url.as_str()).collect::<Vec<_>>(),
            vec!["http://i/ok.png", "http://i/ok2.png"]
        );
        let one = fetch_images(
            &f,
            &urls,
            &ImageLimits {
                max_bytes_each: 1000,
                max_count: 1,
            },
        );
        assert_eq!(one.len(), 1);
        assert_eq!(
            f.urls().iter().filter(|u| *u == "http://i/ok2.png").count(),
            1,
            "stops fetching at the limit"
        );
    }

    #[test]
    fn rewrite_images_maps_or_removes() {
        let mut local = HashMap::new();
        local.insert(
            "https://x.org/a.png?x=1&y=2".to_string(),
            "images/img1.png".to_string(),
        );
        let c = "<p><img src=\"https://x.org/a.png?x=1&amp;y=2\" alt=\"A &amp; B\"/> and <img src=\"https://x.org/gone.png\" alt=\"\"/></p>";
        assert_eq!(
            rewrite_images(c, &local),
            "<p><img src=\"images/img1.png\" alt=\"A &amp; B\"/> and </p>"
        );
    }

    #[test]
    fn article_to_epub_roundtrip() {
        let a = extract(&blog_page(), BASE).unwrap();
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
        jpeg.resize(100, 1);
        let fetched = vec![
            FetchedImage {
                url: "https://blog.example.com/img/lead.jpg".into(),
                data: jpeg.clone(),
            },
            FetchedImage {
                url: "https://blog.example.com/2024/03/post/images/a.png".into(),
                data: png_bytes(),
            },
        ];
        let arc = article_to_epub(&a, &fetched).unwrap();
        // independent checks: zip layout and XML well-formedness
        let bytes = arc.to_bytes().unwrap();
        let mut zip = zip_reader(&bytes);
        assert_eq!(zip.first_name(), "mimetype");
        let reopened = Arc2::from_bytes(bytes).unwrap();
        for name in reopened.names() {
            if name.ends_with(".xhtml") || name.ends_with(".opf") || name.ends_with(".ncx") {
                parse_xml(&reopened.get_string(name).unwrap());
            }
        }
        let book = OpenBook::from_archive(reopened).unwrap();
        let md = book.metadata();
        assert_eq!(md.title, "Why Rust is Fun");
        assert_eq!(md.authors, vec!["Ada Lovelace"]);
        assert_eq!(md.publisher, "Example Blog");
        assert_eq!(md.published, "2024-03-05");
        assert_eq!(md.language, "en");
        assert_eq!(md.identifier("url"), Some(BASE));
        assert!(book.cover_bytes().is_some(), "lead image became the cover");
        let spine = book.spine_paths();
        assert_eq!(spine.len(), 1);
        let ch = book.chapter_source(&spine[0]).unwrap();
        assert!(ch.contains("<h1>Why Rust is Fun</h1>"));
        assert!(ch.contains("By Ada Lovelace"));
        assert!(ch.contains("src=\"images/img1.png\""), "{ch}");
        assert!(!ch.contains("cdn.example.com"), "unfetched image removed");
        assert!(book
            .file_list()
            .iter()
            .any(|f| f.ends_with("images/img1.png")));
        assert_eq!(book.toc().len(), 1);
    }

    struct ZipPeek(Vec<String>);
    impl ZipPeek {
        fn first_name(&mut self) -> String {
            self.0.first().cloned().unwrap_or_default()
        }
    }
    fn zip_reader(bytes: &[u8]) -> ZipPeek {
        // local file header: name length at 26..28, name at 30
        let n = u16::from_le_bytes([bytes[26], bytes[27]]) as usize;
        let method = u16::from_le_bytes([bytes[8], bytes[9]]);
        assert_eq!(method, 0, "mimetype must be stored uncompressed");
        ZipPeek(vec![
            String::from_utf8_lossy(&bytes[30..30 + n]).into_owned()
        ])
    }

    #[test]
    fn article_to_epub_without_images_or_metadata() {
        let a = Article {
            content: "<p>Only text here, nothing else.</p>\n".into(),
            ..Default::default()
        };
        let arc = article_to_epub(&a, &[]).unwrap();
        let book =
            OpenBook::from_archive(Arc2::from_bytes(arc.to_bytes().unwrap()).unwrap()).unwrap();
        assert_eq!(book.metadata().title, "");
        assert!(book.cover_bytes().is_none());
        let ch = book.chapter_source(&book.spine_paths()[0]).unwrap();
        assert!(ch.contains("Only text here"));
    }

    #[test]
    fn article_to_epub_escapes_hostile_titles() {
        let a = Article {
            title: "<script>alert(1)</script> & \"quotes\"".into(),
            author: "A <b>".into(),
            url: "https://x.org/?a=1&b=2".into(),
            content: "<p>Body text.</p>\n".into(),
            ..Default::default()
        };
        let arc = article_to_epub(&a, &[]).unwrap();
        let book =
            OpenBook::from_archive(Arc2::from_bytes(arc.to_bytes().unwrap()).unwrap()).unwrap();
        assert_eq!(
            book.metadata().title,
            "<script>alert(1)</script> & \"quotes\""
        );
        let ch = book.chapter_source(&book.spine_paths()[0]).unwrap();
        parse_xml(&ch);
        assert!(!ch.contains("<script>"));
    }

    #[test]
    fn png_lead_image_and_unusable_lead_image() {
        let mut a = extract(&blog_page(), BASE).unwrap();
        a.lead_image = Some("https://l/x.png".into());
        let arc = article_to_epub(
            &a,
            &[FetchedImage {
                url: "https://l/x.png".into(),
                data: png_bytes(),
            }],
        )
        .unwrap();
        let book =
            OpenBook::from_archive(Arc2::from_bytes(arc.to_bytes().unwrap()).unwrap()).unwrap();
        assert!(book.cover_bytes().is_some());
        a.lead_image = Some("https://l/x.gif".into());
        let arc = article_to_epub(
            &a,
            &[FetchedImage {
                url: "https://l/x.gif".into(),
                data: b"GIF89a....".to_vec(),
            }],
        )
        .unwrap();
        let book =
            OpenBook::from_archive(Arc2::from_bytes(arc.to_bytes().unwrap()).unwrap()).unwrap();
        assert!(book.cover_bytes().is_none(), "GIF is not used as cover");
    }
}
