//! Split an XHTML file into several, or merge spine files into one (E9/E15), repairing manifest, spine, TOC and
//! every inbound link.

use std::collections::{HashMap, HashSet};

use anyhow::{bail, Context, Result};
use bookx_convert::transforms::css::{parse_css, parse_decls, split_selectors, CssItem};
use bookx_core::book::OpenBook;
use bookx_core::opf::SpineItem;
use bookx_core::util::{extension, file_name, parent_dir, parse_xml, relative_href, resolve_href};
use regex::Regex;

use crate::markup::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitSpec {
    /// Start a new file at each element with one of these ids (or `<a name>`).
    AtIds(Vec<String>),
    /// Start a new file at every `h1`..`hN`.
    Headings(u8),
    /// Start a new file at every CSS page break (inline styles and the rules passed in `PageBreakRules`).
    PageBreaks,
    /// Start a new file at these byte offsets of the source (the editor cursor, a position picked in the
    /// preview). An offset inside a tag moves to the edge of that tag, see [`snap_offset`].
    AtOffsets(Vec<usize>),
    /// Start a new file at every element with this class (and this tag name, when given).
    ByClass { tag: Option<String>, class: String },
    /// Start a new file at every element a small XPath-like selector picks, see [`Selector`].
    Selector(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageBreakRules {
    pub before_classes: HashSet<String>,
    pub after_classes: HashSet<String>,
    pub before_tags: HashSet<String>,
    pub after_tags: HashSet<String>,
}

fn is_break_value(v: &str) -> bool {
    matches!(
        v.trim()
            .to_ascii_lowercase()
            .trim_end_matches("!important")
            .trim(),
        "always" | "page" | "left" | "right" | "recto" | "verso"
    )
}

/// `(before, after)` flags of a declaration list.
fn break_flags(decls: &[(String, String)]) -> (bool, bool) {
    let mut before = false;
    let mut after = false;
    for (n, v) in decls {
        match n.as_str() {
            "page-break-before" | "break-before" if is_break_value(v) => before = true,
            "page-break-after" | "break-after" if is_break_value(v) => after = true,
            _ => {}
        }
    }
    (before, after)
}

impl PageBreakRules {
    /// Extracts the simple selectors (`.cls`, `tag`, `tag.cls`) that force a page break.
    pub fn from_css(css: &str) -> Self {
        let mut r = Self::default();
        for item in parse_css(css) {
            let CssItem::Rule { selectors, block } = item else {
                continue;
            };
            let decls: Vec<(String, String)> = parse_decls(block)
                .into_iter()
                .map(|(n, v, _)| (n, v))
                .collect();
            let (before, after) = break_flags(&decls);
            if !before && !after {
                continue;
            }
            for sel in split_selectors(selectors) {
                let (tag, class) = match sel.split_once('.') {
                    Some((t, c)) => (t.trim(), Some(c.trim())),
                    None => (sel.trim(), None),
                };
                let simple = |s: &str| {
                    s.chars()
                        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
                };
                if !simple(tag) || class.is_some_and(|c| !simple(c) || c.is_empty()) {
                    continue;
                }
                match class {
                    Some(c) if tag.is_empty() => {
                        if before {
                            r.before_classes.insert(c.to_string());
                        }
                        if after {
                            r.after_classes.insert(c.to_string());
                        }
                    }
                    Some(_) => {}
                    None if !tag.is_empty() => {
                        if before {
                            r.before_tags.insert(tag.to_ascii_lowercase());
                        }
                        if after {
                            r.after_tags.insert(tag.to_ascii_lowercase());
                        }
                    }
                    None => {}
                }
            }
        }
        r
    }

    pub fn merge(&mut self, other: PageBreakRules) {
        self.before_classes.extend(other.before_classes);
        self.after_classes.extend(other.after_classes);
        self.before_tags.extend(other.before_tags);
        self.after_tags.extend(other.after_tags);
    }
}

fn has_content(fragment: &str) -> bool {
    !plain_text(fragment).trim().is_empty()
        || fragment.contains("<img")
        || fragment.contains("<svg")
        || fragment.contains("<image")
}

/// Byte offsets (in `src`) at which new files should start for `spec`. Offsets are element start tags (or element
/// ends for `break-after`), sorted and free of splits that would create an empty part.
pub fn split_points(src: &str, spec: &SplitSpec, rules: &PageBreakRules) -> Result<Vec<usize>> {
    let (body_start, body_end) = body_bounds(src).context("the file has no <body> to split")?;
    let doc = parse_xml(src).context("only well-formed XHTML can be split")?;
    let mut points: Vec<usize> = Vec::new();
    let inside_body = |n: &roxmltree::Node| {
        let r = n.range();
        r.start >= body_start && r.end <= body_end
    };
    match spec {
        SplitSpec::AtIds(ids) => {
            for id in ids {
                let found = doc.descendants().find(|n| {
                    n.is_element()
                        && inside_body(n)
                        && (n.attribute("id") == Some(id.as_str())
                            || (n.tag_name().name() == "a"
                                && n.attribute("name") == Some(id.as_str())))
                });
                match found {
                    Some(n) => points.push(n.range().start),
                    None => bail!("no element with id \"{id}\" in the file"),
                }
            }
        }
        SplitSpec::Headings(max) => {
            let max = (*max).clamp(1, 6);
            for n in doc
                .descendants()
                .filter(|n| n.is_element() && inside_body(n))
            {
                let name = n.tag_name().name();
                if let Some(level) = name
                    .strip_prefix('h')
                    .and_then(|d| d.parse::<u8>().ok())
                    .filter(|l| (1..=6).contains(l))
                {
                    if level <= max {
                        points.push(n.range().start);
                    }
                }
            }
        }
        SplitSpec::AtOffsets(offs) => {
            points.extend(offs.iter().map(|o| snap_offset(src, *o)));
        }
        SplitSpec::ByClass { tag, class } => {
            for n in doc
                .descendants()
                .filter(|n| n.is_element() && inside_body(n))
            {
                let tag_ok = tag
                    .as_ref()
                    .is_none_or(|t| t == "*" || n.tag_name().name().eq_ignore_ascii_case(t.trim()));
                let has = n
                    .attribute("class")
                    .is_some_and(|c| c.split_whitespace().any(|w| w == class.trim()));
                if tag_ok && has {
                    points.push(n.range().start);
                }
            }
        }
        SplitSpec::Selector(text) => {
            let sel = Selector::parse(text)?;
            for n in doc
                .descendants()
                .filter(|n| n.is_element() && inside_body(n))
            {
                if sel.matches(n) {
                    points.push(n.range().start);
                }
            }
        }
        SplitSpec::PageBreaks => {
            for n in doc
                .descendants()
                .filter(|n| n.is_element() && inside_body(n))
            {
                let mut before = false;
                let mut after = false;
                if let Some(style) = n.attribute("style") {
                    let decls: Vec<(String, String)> = style
                        .split(';')
                        .filter_map(|d| d.split_once(':'))
                        .map(|(a, b)| (a.trim().to_ascii_lowercase(), b.trim().to_string()))
                        .collect();
                    let (b, a) = break_flags(&decls);
                    before |= b;
                    after |= a;
                }
                let tag = n.tag_name().name().to_ascii_lowercase();
                before |= rules.before_tags.contains(&tag);
                after |= rules.after_tags.contains(&tag);
                if let Some(cls) = n.attribute("class") {
                    for c in cls.split_whitespace() {
                        before |= rules.before_classes.contains(c);
                        after |= rules.after_classes.contains(c);
                    }
                }
                if before {
                    points.push(n.range().start);
                }
                if after {
                    points.push(n.range().end);
                }
            }
        }
    }
    points.retain(|p| *p > body_start && *p < body_end);
    points.sort_unstable();
    points.dedup();
    // drop splits that would leave an empty part at either end or in the middle
    let mut kept: Vec<usize> = Vec::new();
    let mut prev = body_start;
    for p in points {
        if has_content(&src[prev..p]) {
            kept.push(p);
            prev = p;
        }
    }
    while let Some(&last) = kept.last() {
        if has_content(&src[last..body_end]) {
            break;
        }
        kept.pop();
    }
    Ok(kept)
}

/// Moves a split position to where cutting makes sense: out of a tag, comment or processing instruction (to its
/// start, or to the end of an end tag), and out of the edge of an element (right after a start tag, right before
/// an end tag: only blanks in between), so no empty `<p></p>` is left behind. Positions in the middle of text are
/// kept (the open elements are closed and re-opened by [`split_document`]). An offset past the end is clamped.
pub fn snap_offset(src: &str, off: usize) -> usize {
    let mut off = off.min(src.len());
    while !src.is_char_boundary(off) {
        off -= 1;
    }
    let events = scan(src);
    for ev in &events {
        match ev {
            Event::Open { range, .. } | Event::Other(range)
                if range.start < off && off < range.end =>
            {
                off = range.start;
                break;
            }
            Event::Close { range, .. } if range.start < off && off < range.end => {
                off = range.end;
                break;
            }
            Event::Open { range, .. } | Event::Close { range, .. } | Event::Other(range)
                if range.start >= off =>
            {
                break
            }
            _ => {}
        }
    }
    // the edge of an element: step out of it
    loop {
        let blank_before = src[..off].trim_end().len();
        let blank_after = off + (src.len() - off - src[off..].trim_start().len());
        let open = events.iter().find_map(|e| match e {
            Event::Open {
                range,
                self_closing: false,
                name,
            } if range.end == blank_before && !is_void_tag(name) && !is_root_tag(name) => {
                Some(range.start)
            }
            _ => None,
        });
        if let Some(s) = open {
            off = s;
            continue;
        }
        let close = events.iter().find_map(|e| match e {
            Event::Close { range, name } if range.start == blank_after && !is_root_tag(name) => {
                Some(range.end)
            }
            _ => None,
        });
        match close {
            Some(e) => off = e,
            None => break,
        }
    }
    off
}

fn is_root_tag(name: &str) -> bool {
    matches!(name, "body" | "html" | "head")
}

fn is_void_tag(name: &str) -> bool {
    matches!(
        name,
        "br" | "hr"
            | "img"
            | "meta"
            | "link"
            | "input"
            | "col"
            | "area"
            | "base"
            | "wbr"
            | "source"
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Pred {
    Has(String),
    Eq(String, String),
    Contains(String, String),
    Word(String, String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Step {
    /// `//` before the step (any depth) instead of `/` (direct child).
    deep: bool,
    name: Option<String>,
    preds: Vec<Pred>,
}

/// A small XPath-like selector for choosing split points: `//h2`, `//div[@class='chapter']`,
/// `//*[contains(@class,'part')]`, `//section/h1`, plus the CSS-style shorthands `h2.part`, `.part`, `#id`.
/// Steps are joined by `/` (child) or `//` (any depth).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    steps: Vec<Step>,
}

impl Selector {
    pub fn parse(text: &str) -> Result<Selector> {
        let t = text.trim();
        if t.is_empty() {
            bail!("the selector is empty");
        }
        let css_like = !t.starts_with('/') && !t.contains('[') && !t.contains('/');
        if css_like {
            return Self::parse_css(t);
        }
        let mut steps = Vec::new();
        let mut rest = t;
        let mut first = true;
        while !rest.is_empty() {
            let deep = if let Some(r) = rest.strip_prefix("//") {
                rest = r;
                true
            } else if let Some(r) = rest.strip_prefix('/') {
                rest = r;
                false
            } else if first {
                true
            } else {
                bail!("expected / in \"{text}\"");
            };
            first = false;
            let end = step_end(rest);
            let (step, r) = rest.split_at(end);
            rest = r;
            steps.push(parse_step(step, deep, text)?);
        }
        if steps.is_empty() {
            bail!("the selector \"{text}\" has no step");
        }
        Ok(Selector { steps })
    }

    fn parse_css(t: &str) -> Result<Selector> {
        let mut name = String::new();
        let mut preds = Vec::new();
        let mut cur = String::new();
        let mut kind = ' ';
        let flush = |kind: char, cur: &mut String, name: &mut String, preds: &mut Vec<Pred>| {
            if cur.is_empty() {
                return;
            }
            match kind {
                '.' => preds.push(Pred::Word("class".into(), cur.clone())),
                '#' => preds.push(Pred::Eq("id".into(), cur.clone())),
                _ => *name = cur.clone(),
            }
            cur.clear();
        };
        for ch in t.chars() {
            if ch == '.' || ch == '#' {
                flush(kind, &mut cur, &mut name, &mut preds);
                kind = ch;
            } else if ch.is_alphanumeric() || ch == '-' || ch == '_' || ch == '*' {
                cur.push(ch);
            } else {
                bail!("cannot read the selector \"{t}\"");
            }
        }
        flush(kind, &mut cur, &mut name, &mut preds);
        if name.is_empty() && preds.is_empty() {
            bail!("cannot read the selector \"{t}\"");
        }
        Ok(Selector {
            steps: vec![Step {
                deep: true,
                name: (!name.is_empty() && name != "*").then_some(name.to_ascii_lowercase()),
                preds,
            }],
        })
    }

    pub fn matches(&self, n: roxmltree::Node) -> bool {
        self.match_from(self.steps.len() - 1, n)
    }

    fn match_from(&self, idx: usize, n: roxmltree::Node) -> bool {
        let st = &self.steps[idx];
        if !step_matches(st, n) {
            return false;
        }
        if idx == 0 {
            // a leading single `/` means the step is the document element's child
            return true;
        }
        let prev = idx - 1;
        if st.deep {
            n.ancestors()
                .skip(1)
                .any(|a| a.is_element() && self.match_from(prev, a))
        } else {
            n.parent()
                .is_some_and(|p| p.is_element() && self.match_from(prev, p))
        }
    }
}

fn step_end(s: &str) -> usize {
    let mut depth = 0;
    let mut quote = None;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, '[' | '(') => depth += 1,
            (None, ']' | ')') => depth -= 1,
            (None, '/') if depth == 0 => return i,
            _ => {}
        }
    }
    s.len()
}

fn parse_step(s: &str, deep: bool, whole: &str) -> Result<Step> {
    let (name, mut rest) = match s.find('[') {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    };
    let name = name.trim();
    if name.is_empty() {
        bail!("a step of \"{whole}\" has no element name (use * for any)");
    }
    let mut preds = Vec::new();
    while let Some(r) = rest.strip_prefix('[') {
        let end = r
            .rfind(']')
            .with_context(|| format!("missing ] in \"{whole}\""))?;
        // the predicate ends at the first ] outside quotes
        let mut q = None;
        let mut close = None;
        for (i, c) in r.char_indices() {
            match (q, c) {
                (Some(x), c) if c == x => q = None,
                (Some(_), _) => {}
                (None, '\'' | '"') => q = Some(c),
                (None, ']') => {
                    close = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let close = close.unwrap_or(end);
        preds.push(parse_pred(r[..close].trim(), whole)?);
        rest = &r[close + 1..];
    }
    if !rest.trim().is_empty() {
        bail!("cannot read \"{whole}\" near \"{rest}\"");
    }
    Ok(Step {
        deep,
        name: (name != "*").then(|| name.to_ascii_lowercase()),
        preds,
    })
}

fn unquote(v: &str) -> String {
    v.trim().trim_matches(|c| c == '\'' || c == '"').to_string()
}

fn parse_pred(p: &str, whole: &str) -> Result<Pred> {
    if let Some(inner) = p
        .strip_prefix("contains(")
        .and_then(|r| r.strip_suffix(')'))
    {
        let (a, v) = inner
            .split_once(',')
            .with_context(|| format!("contains() needs two arguments in \"{whole}\""))?;
        let a = a
            .trim()
            .strip_prefix('@')
            .with_context(|| format!("contains() works on an attribute (@name) in \"{whole}\""))?;
        return Ok(Pred::Contains(a.to_string(), unquote(v)));
    }
    let a = p
        .strip_prefix('@')
        .with_context(|| format!("cannot read the condition [{p}] in \"{whole}\""))?;
    match a.split_once('=') {
        Some((n, v)) => Ok(Pred::Eq(n.trim().to_string(), unquote(v))),
        None => Ok(Pred::Has(a.trim().to_string())),
    }
}

fn step_matches(st: &Step, n: roxmltree::Node) -> bool {
    if !n.is_element() {
        return false;
    }
    if let Some(name) = &st.name {
        if !n.tag_name().name().eq_ignore_ascii_case(name) {
            return false;
        }
    }
    st.preds.iter().all(|p| match p {
        Pred::Has(a) => n.attribute(a.as_str()).is_some(),
        Pred::Eq(a, v) => n.attribute(a.as_str()) == Some(v.as_str()),
        Pred::Contains(a, v) => n
            .attribute(a.as_str())
            .is_some_and(|x| x.contains(v.as_str())),
        Pred::Word(a, v) => n
            .attribute(a.as_str())
            .is_some_and(|x| x.split_whitespace().any(|w| w == v)),
    })
}

/// One file a split would produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartPreview {
    /// First heading, or the start of the text.
    pub title: String,
    /// Characters of text in the part.
    pub chars: usize,
}

/// What a split at `offsets` would give, without changing anything (CE-249 preview).
pub fn preview_parts(src: &str, offsets: &[usize]) -> Vec<PartPreview> {
    let Some((bs, be)) = body_bounds(src) else {
        return Vec::new();
    };
    let mut bounds = vec![bs];
    bounds.extend(offsets.iter().copied().filter(|o| *o > bs && *o < be));
    bounds.push(be);
    bounds
        .windows(2)
        .map(|w| {
            let frag = &src[w[0]..w[1]];
            let text = normalized_text(frag);
            let heading = heading_text(frag);
            let title = heading.unwrap_or_else(|| text.chars().take(48).collect());
            PartPreview {
                title,
                chars: text.chars().count(),
            }
        })
        .collect()
}

fn heading_text(frag: &str) -> Option<String> {
    let mut open: Option<usize> = None;
    for ev in scan(frag) {
        match ev {
            Event::Open { name, range, .. }
                if name.len() == 2
                    && name.starts_with('h')
                    && name.as_bytes()[1].is_ascii_digit() =>
            {
                open = Some(range.end)
            }
            Event::Close { name, range }
                if name.len() == 2 && name.starts_with('h') && open.is_some() =>
            {
                let t = normalized_text(&frag[open?..range.start]);
                if !t.is_empty() {
                    return Some(t);
                }
                open = None;
            }
            _ => {}
        }
    }
    None
}

fn strip_id(tag: &str) -> String {
    static_re_id().replace(tag, "").into_owned()
}

fn static_re_id() -> &'static Regex {
    use std::sync::OnceLock;
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r#"\s(?:xml:)?id\s*=\s*(?:"[^"]*"|'[^']*')"#).unwrap())
}

/// Cuts `src` at the given offsets (from [`split_points`]) into `offsets.len() + 1` well-formed documents. Open
/// ancestors are closed at the end of a part and re-opened (without their `id`) at the start of the next one.
pub fn split_document(src: &str, offsets: &[usize]) -> Result<Vec<String>> {
    let (body_start, body_end) = body_bounds(src).context("the file has no <body> to split")?;
    let doc = parse_xml(src).context("only well-formed XHTML can be split")?;
    let body = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "body")
        .context("no body element")?;
    let mut offs: Vec<usize> = offsets.to_vec();
    offs.sort_unstable();
    offs.dedup();
    if offs.iter().any(|o| *o <= body_start || *o >= body_end) {
        bail!("split offset outside of the body");
    }
    if offs.is_empty() {
        return Ok(vec![src.to_string()]);
    }
    let chain = |off: usize| -> Vec<(String, String)> {
        doc.descendants()
            .filter(|n| {
                n.is_element()
                    && *n != body
                    && n.ancestors().any(|a| a == body)
                    && n.range().start < off
                    && off < n.range().end
            })
            .map(|n| {
                let start = n.range().start;
                let open = &src[start..start_tag_end(src, start)];
                (strip_id(open), format!("</{}>", tag_qname(open)))
            })
            .collect()
    };
    let body_open_range = {
        let mut r = None;
        for ev in scan(src) {
            if let Event::Open { name, range, .. } = ev {
                if name == "body" {
                    r = Some(range);
                    break;
                }
            }
        }
        r.context("no body start tag")?
    };
    let head = &src[..body_start];
    let head_no_id = format!(
        "{}{}{}",
        &src[..body_open_range.start],
        strip_id(&src[body_open_range.clone()]),
        &src[body_open_range.end..body_start]
    );
    let tail = &src[body_end..];
    let mut bounds = vec![body_start];
    bounds.extend(&offs);
    bounds.push(body_end);
    let mut parts = Vec::new();
    for k in 0..bounds.len() - 1 {
        let opener = if k == 0 {
            String::new()
        } else {
            chain(bounds[k]).iter().map(|(o, _)| o.as_str()).collect()
        };
        let closer = if k + 1 == bounds.len() - 1 {
            String::new()
        } else {
            chain(bounds[k + 1])
                .iter()
                .rev()
                .map(|(_, c)| c.as_str())
                .collect()
        };
        parts.push(format!(
            "{}{}{}{}{}",
            if k == 0 { head } else { &head_no_id },
            opener,
            &src[bounds[k]..bounds[k + 1]],
            closer,
            tail
        ));
    }
    for p in &parts {
        parse_xml(p).context("internal error: a split part is not well-formed")?;
    }
    Ok(parts)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitResult {
    /// All resulting files in reading order; the first is the original path.
    pub parts: Vec<String>,
}

fn unique_id(book: &OpenBook, base: &str) -> String {
    let mut id = base.to_string();
    let mut n = 1;
    while book.package.item_by_id(&id).is_some() {
        n += 1;
        id = format!("{base}-{n}");
    }
    id
}

/// Page break rules from the style sheets and `<style>` blocks `src` (at `path`) uses.
pub fn page_break_rules_for(book: &OpenBook, src: &str, path: &str) -> PageBreakRules {
    let mut rules = PageBreakRules::default();
    let dir = parent_dir(path);
    for ev in scan(src) {
        match ev {
            Event::Open { name, range, .. } if name == "link" => {
                let tag = &src[range];
                if let Some(href) = tag_attr(tag, "href") {
                    let (p, _) = resolve_href(dir, &href);
                    if let Some(css) = book.archive.get_string(&p) {
                        rules.merge(PageBreakRules::from_css(&css));
                    }
                }
            }
            Event::Text(r) => {
                let t = &src[r];
                if t.contains("break") {
                    rules.merge(PageBreakRules::from_css(t));
                }
            }
            _ => {}
        }
    }
    rules
}

/// Splits `path` according to `spec`. The original keeps the first part; others are named `<stem>_split_NNN.<ext>`,
/// added to the manifest and the spine right after the original, and every link into the book (TOC, NCX, nav,
/// other chapters) is re-pointed at the part that now holds its target.
pub fn split_file(book: &mut OpenBook, path: &str, spec: &SplitSpec) -> Result<SplitResult> {
    let src = book
        .archive
        .get_string(path)
        .with_context(|| format!("{path} not found"))?;
    if !is_markup_path(path) {
        bail!("{path} is not an XHTML/HTML file");
    }
    let item = book
        .package
        .item_by_path(path)
        .cloned()
        .with_context(|| format!("{path} is not in the manifest"))?;
    let rules = page_break_rules_for(book, &src, path);
    let offsets = split_points(&src, spec, &rules)?;
    if offsets.is_empty() {
        bail!("nothing to split: no usable split points in {path}");
    }
    let parts = split_document(&src, &offsets)?;

    let dir = parent_dir(path).to_string();
    let ext = extension(path);
    let stem = file_name(path)
        .rsplit_once('.')
        .map(|(s, _)| s.to_string())
        .unwrap_or_else(|| file_name(path).to_string());
    let mut paths = vec![path.to_string()];
    let mut n = 0;
    while paths.len() < parts.len() {
        n += 1;
        let cand = bookx_core::util::join_path(&dir, &format!("{stem}_split_{n:03}.{ext}"));
        if !book.archive.contains(&cand) {
            paths.push(cand);
        }
    }

    // id -> part index
    let doc = parse_xml(&src)?;
    let mut id_part: HashMap<String, usize> = HashMap::new();
    for el in doc.descendants().filter(|n| n.is_element()) {
        let part = offsets.iter().filter(|o| **o <= el.range().start).count();
        for v in [
            el.attribute("id"),
            (el.tag_name().name() == "a")
                .then(|| el.attribute("name"))
                .flatten(),
        ]
        .into_iter()
        .flatten()
        {
            id_part.entry(v.to_string()).or_insert(part);
        }
    }
    drop(doc);

    for (p, text) in paths.iter().zip(&parts) {
        book.archive.set_string(p, text);
    }
    let pkg = book.package.clone();
    let mut new_ids = vec![item.id.clone()];
    for p in &paths[1..] {
        let id = unique_id(book, &format!("{}_split_{}", item.id, new_ids.len()));
        pkg.add_manifest_item(
            &mut book.archive,
            &id,
            p,
            &item.media_type,
            &item.properties,
        )?;
        book.reload_package()?;
        new_ids.push(id);
    }
    if let Some(pos) = book.package.spine.iter().position(|s| s.idref == item.id) {
        let mut spine: Vec<SpineItem> = book.package.spine.clone();
        let linear = spine[pos].linear;
        for (k, id) in new_ids.iter().enumerate().skip(1) {
            spine.insert(
                pos + k,
                SpineItem {
                    idref: id.clone(),
                    linear,
                },
            );
        }
        let pkg = book.package.clone();
        pkg.set_spine(&mut book.archive, &spine)?;
    }

    let part_set: HashSet<&str> = paths.iter().map(|s| s.as_str()).collect();
    retarget_links(&mut book.archive, |from, target, frag| {
        let from_is_part = part_set.contains(from);
        let refers_orig = target == path || (from_is_part && target == from);
        if !refers_orig {
            return None;
        }
        let frag = frag?;
        let idx = *id_part.get(frag)?;
        let dest = &paths[idx];
        if dest == from || (!from_is_part && dest == path && target == path) {
            return None;
        }
        Some(format!("{dest}#{frag}"))
    });
    book.reload_package()?;
    Ok(SplitResult { parts: paths })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeResult {
    pub target: String,
    pub removed: Vec<String>,
    /// `(file, old id, new id)` for ids renamed to resolve conflicts.
    pub renamed_ids: Vec<(String, String, String)>,
}

/// Merges `paths[1..]` into `paths[0]` (content appended in the given order). Conflicting ids are renamed,
/// relative resource links are rebased, stylesheets are carried over and every link to a merged file (TOC, NCX,
/// nav, other chapters) now points into the target. The merged files leave the manifest, spine and archive.
pub fn merge_files(book: &mut OpenBook, paths: &[String]) -> Result<MergeResult> {
    if paths.len() < 2 {
        bail!("select at least two files to merge");
    }
    let uniq: HashSet<&String> = paths.iter().collect();
    if uniq.len() != paths.len() {
        bail!("the same file was selected twice");
    }
    let nav = book.package.nav_item().map(|i| book.package.item_path(i));
    for p in paths {
        if !is_markup_path(p) {
            bail!("{p} is not an XHTML/HTML file");
        }
        if book.package.item_by_path(p).is_none() {
            bail!("{p} is not in the manifest");
        }
        if !book.archive.contains(p) {
            bail!("{p} is missing from the archive");
        }
        if nav.as_deref() == Some(p.as_str()) {
            bail!("the navigation document cannot be merged");
        }
    }
    let target = paths[0].clone();
    let others: Vec<String> = paths[1..].to_vec();
    let tdir = parent_dir(&target).to_string();
    let mut tsrc = book.archive.get_string(&target).unwrap_or_default();
    parse_xml(&tsrc).with_context(|| format!("{target} is not well-formed XHTML"))?;
    let (_, t_ce) = body_bounds(&tsrc).with_context(|| format!("{target} has no <body>"))?;

    // files that are linked without a fragment need a marker anchor
    let mut plain_targets: HashSet<String> = HashSet::new();
    for name in book.archive.names() {
        if !is_reference_text(name) {
            continue;
        }
        let Some(text) = book.archive.get_string(name) else {
            continue;
        };
        for r in collect_refs(&text, ref_kind(name)) {
            if is_local_ref(&r) {
                let (p, frag) = resolve_href(parent_dir(name), &r);
                if frag.is_none() && others.contains(&p) {
                    plain_targets.insert(p);
                }
            }
        }
    }

    let mut taken: HashSet<String> = defined_ids(&tsrc).into_iter().collect();
    let mut all_ids: HashSet<String> = taken.clone();
    let mut sources: Vec<(String, String)> = Vec::new();
    for o in &others {
        let s = book.archive.get_string(o).unwrap_or_default();
        parse_xml(&s).with_context(|| format!("{o} is not well-formed XHTML"))?;
        all_ids.extend(defined_ids(&s));
        sources.push((o.clone(), s));
    }

    let mut appended = String::new();
    let mut head_add = String::new();
    let mut result = MergeResult {
        target: target.clone(),
        removed: others.clone(),
        renamed_ids: Vec::new(),
    };
    let mut id_map: HashMap<(String, String), String> = HashMap::new();
    let mut markers: HashMap<String, String> = HashMap::new();
    let linked: HashSet<String> = collect_head_links(&tsrc, &tdir);

    for (o, osrc) in &sources {
        let odir = parent_dir(o);
        let (bs, be) = body_bounds(osrc).with_context(|| format!("{o} has no <body>"))?;
        let mut inner = osrc[bs..be].to_string();
        let ids_o = defined_ids(osrc);
        let mut rename: HashMap<String, String> = HashMap::new();
        for id in &ids_o {
            if taken.contains(id) && !rename.contains_key(id) {
                let mut n = 2;
                let mut cand = format!("{id}-{n}");
                while all_ids.contains(&cand) || taken.contains(&cand) {
                    n += 1;
                    cand = format!("{id}-{n}");
                }
                all_ids.insert(cand.clone());
                rename.insert(id.clone(), cand.clone());
                id_map.insert((o.clone(), id.clone()), cand.clone());
                result.renamed_ids.push((o.clone(), id.clone(), cand));
            }
        }
        if !rename.is_empty() {
            inner = rewrite_all_tags(&inner, &mut |tag, attr, val| match attr {
                "id" | "xml:id" => rename.get(val).cloned(),
                "name" if tag == "a" => rename.get(val).cloned(),
                "href" | "xlink:href" => val
                    .strip_prefix('#')
                    .and_then(|f| rename.get(f))
                    .map(|n| format!("#{n}")),
                _ => None,
            });
        }
        if odir != tdir {
            inner = map_refs(&inner, RefKind::Markup, &mut |href| {
                if !is_local_ref(href) {
                    return None;
                }
                let (p, frag) = resolve_href(odir, href);
                if p.is_empty() {
                    return None;
                }
                let to = match frag {
                    Some(f) => format!("{p}#{f}"),
                    None => p,
                };
                Some(relative_href(&target, &to))
            });
        }
        // body attributes that style the content
        let body_tag = {
            let mut t = String::new();
            for ev in scan(osrc) {
                if let Event::Open { name, range, .. } = ev {
                    if name == "body" {
                        t = osrc[range].to_string();
                        break;
                    }
                }
            }
            t
        };
        let class = tag_attr(&body_tag, "class");
        let style = tag_attr(&body_tag, "style");
        let mut marker = String::new();
        if plain_targets.contains(o) {
            let stem = file_name(o)
                .rsplit_once('.')
                .map(|(s, _)| s)
                .unwrap_or(file_name(o));
            let mut m = stem.to_string();
            let mut n = 1;
            while all_ids.contains(&m) || taken.contains(&m) {
                n += 1;
                m = format!("{stem}-{n}");
            }
            all_ids.insert(m.clone());
            markers.insert(o.clone(), m.clone());
            marker = format!("<div id=\"{}\"/>", xml_attr(&m));
            taken.insert(m);
        }
        for id in ids_o {
            taken.insert(rename.get(&id).cloned().unwrap_or(id));
        }
        appended.push('\n');
        appended.push_str(&marker);
        if class.is_some() || style.is_some() {
            let mut attrs = String::new();
            if let Some(c) = class {
                attrs.push_str(&format!(" class=\"{}\"", xml_attr(&c)));
            }
            if let Some(s) = style {
                attrs.push_str(&format!(" style=\"{}\"", xml_attr(&s)));
            }
            appended.push_str(&format!("<div{attrs}>{inner}</div>"));
        } else {
            appended.push_str(&inner);
        }
        for ev in scan(osrc) {
            if let Event::Open { name, range, .. } = ev {
                if name == "link" {
                    let tag = &osrc[range];
                    let rel = tag_attr(tag, "rel").unwrap_or_default().to_lowercase();
                    if let (true, Some(href)) = (rel.contains("stylesheet"), tag_attr(tag, "href"))
                    {
                        let (p, _) = resolve_href(odir, &href);
                        if !p.is_empty()
                            && !linked.contains(&p)
                            && !head_add.contains(&format!("\"{}\"", relative_href(&target, &p)))
                        {
                            head_add.push_str(&format!(
                                "<link rel=\"stylesheet\" type=\"text/css\" href=\"{}\"/>\n",
                                xml_attr(&relative_href(&target, &p))
                            ));
                        }
                    }
                }
            }
        }
    }

    tsrc.insert_str(t_ce, &format!("{appended}\n"));
    if !head_add.is_empty() {
        if let Some(h) = tsrc.rfind("</head>") {
            tsrc.insert_str(h, &head_add);
        }
    }
    parse_xml(&tsrc).context("internal error: merged document is not well-formed")?;
    book.archive.set_string(&target, &tsrc);

    for o in &others {
        if let Some(item) = book.package.item_by_path(o).cloned() {
            let pkg = book.package.clone();
            pkg.remove_manifest_item(&mut book.archive, &item.id)?;
            book.reload_package()?;
        }
        book.archive.remove(o);
    }
    retarget_links(&mut book.archive, |_, path, frag| {
        if !others.contains(&path.to_string()) {
            return None;
        }
        match frag {
            Some(f) => {
                let f = id_map
                    .get(&(path.to_string(), f.to_string()))
                    .map(|s| s.as_str())
                    .unwrap_or(f);
                Some(format!("{target}#{f}"))
            }
            None => Some(match markers.get(path) {
                Some(m) => format!("{target}#{m}"),
                None => target.clone(),
            }),
        }
    });
    book.reload_package()?;
    Ok(result)
}

fn xml_attr(s: &str) -> String {
    bookx_core::util::xml_escape(s)
}

fn collect_head_links(src: &str, dir: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    for ev in scan(src) {
        if let Event::Open { name, range, .. } = ev {
            if name == "link" {
                if let Some(h) = tag_attr(&src[range], "href") {
                    out.insert(resolve_href(dir, &h).0);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::make_book;
    use bookx_core::check::{check_book, Severity};

    fn errors(book: &OpenBook) -> Vec<String> {
        check_book(&book.archive)
            .into_iter()
            .filter(|i| {
                i.severity == Severity::Error
                    || i.message.contains("Link target")
                    || i.message.contains("Broken")
            })
            .map(|i| format!("{}: {}", i.file, i.message))
            .collect()
    }

    fn all_text(book: &OpenBook) -> String {
        book.spine_paths()
            .iter()
            .map(|p| normalized_text(&book.archive.get_string(p).unwrap()))
            .collect::<Vec<_>>()
            .join(" ")
    }

    const SAMPLE: &str = "<h1 id=\"a\">Alpha</h1><p>one</p><div class=\"w\" id=\"wrap\"><p id=\"p1\">two</p><h2 id=\"b\">Beta</h2><p>three <a href=\"#a\">back</a></p></div><h1 id=\"c\">Gamma</h1><p>four <a href=\"#b\">to beta</a></p>";

    #[test]
    fn page_break_rules_from_css() {
        let r = PageBreakRules::from_css(
            ".chap { page-break-before: always } h1 { break-after: page } .no { page-break-before: avoid } div.x { break-before: page } a, .two { page-break-after: always !important }",
        );
        assert!(r.before_classes.contains("chap"));
        assert!(!r.before_classes.contains("no"));
        assert!(r.after_tags.contains("h1"));
        assert!(r.after_tags.contains("a"));
        assert!(r.after_classes.contains("two"));
        assert!(r.before_classes.len() == 1 && r.before_tags.is_empty());
        let mut m = PageBreakRules::default();
        m.merge(r);
        assert_eq!(m.before_classes.len(), 1);
    }

    #[test]
    fn split_points_by_heading_level_and_ids() {
        let src = format!("<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>t</title></head><body>{SAMPLE}</body></html>");
        let p1 = split_points(&src, &SplitSpec::Headings(1), &PageBreakRules::default()).unwrap();
        assert_eq!(
            p1.len(),
            1,
            "first h1 is at the start and does not split: {p1:?}"
        );
        assert!(src[p1[0]..].starts_with("<h1 id=\"c\">"));
        let p2 = split_points(&src, &SplitSpec::Headings(2), &PageBreakRules::default()).unwrap();
        assert_eq!(p2.len(), 2);
        let ids = split_points(
            &src,
            &SplitSpec::AtIds(vec!["p1".into(), "c".into()]),
            &PageBreakRules::default(),
        )
        .unwrap();
        assert_eq!(ids.len(), 2);
        let e = split_points(
            &src,
            &SplitSpec::AtIds(vec!["zz".into()]),
            &PageBreakRules::default(),
        )
        .unwrap_err();
        assert_eq!(e.to_string(), "no element with id \"zz\" in the file");
    }

    #[test]
    fn split_points_page_breaks() {
        let src = "<html><body><p>a</p><p style=\"page-break-before: always\">b</p><div class=\"x\">c</div><p>d</p><p class=\"y\">e</p></body></html>";
        let mut rules = PageBreakRules::default();
        rules.after_classes.insert("x".into());
        rules.before_classes.insert("y".into());
        let pts = split_points(src, &SplitSpec::PageBreaks, &rules).unwrap();
        assert_eq!(pts.len(), 3);
        let parts = split_document(src, &pts).unwrap();
        assert_eq!(parts.len(), 4);
        assert!(parts[1].contains(
            "<p style=\"page-break-before: always\">b</p><div class=\"x\">c</div></body>"
        ));
        // break-after on the very last element does not create an empty part
        let src2 = "<html><body><p>a</p><p class=\"x\">b</p></body></html>";
        assert!(split_points(src2, &SplitSpec::PageBreaks, &rules)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn split_document_reopens_ancestors() {
        let src = "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>t</title></head><body id=\"bd\"><div class=\"w\" id=\"wrap\"><p>one</p><p id=\"two\">two</p></div></body></html>";
        let off = src.find("<p id=\"two\">").unwrap();
        let parts = split_document(src, &[off]).unwrap();
        assert_eq!(parts.len(), 2);
        assert!(parts[0].contains("<p>one</p></div></body></html>"));
        assert!(parts[0].contains("id=\"wrap\""));
        assert!(
            parts[1].contains("<body><div class=\"w\"><p id=\"two\">two</p></div></body>"),
            "{}",
            parts[1]
        );
        assert!(!parts[1].contains("wrap"));
        assert!(!parts[1].contains("bd"));
        assert_eq!(split_document(src, &[]).unwrap(), vec![src.to_string()]);
        assert!(split_document(src, &[1]).is_err());
        assert!(split_document("<html><head/></html>", &[3]).is_err());
        assert!(split_document("<html><body><p>x</body></html>", &[15]).is_err());
    }

    #[test]
    fn split_file_updates_manifest_spine_links_toc() {
        let mut book = make_book(&[SAMPLE, "<p id=\"x\">other <a href=\"chapter-1.xhtml#c\">to gamma</a> <a href=\"chapter-1.xhtml\">whole</a></p>"]);
        // make the TOC point into chapter 1
        let toc = vec![bookx_core::toc::TocEntry {
            title: "Gamma".into(),
            href: "OEBPS/chapter-1.xhtml#c".into(),
            children: vec![],
        }];
        bookx_core::toc::write_toc(&mut book.archive, &book.package, &toc).unwrap();
        let before = all_text(&book);
        assert!(errors(&book).is_empty(), "{:?}", errors(&book));
        let res = split_file(&mut book, "OEBPS/chapter-1.xhtml", &SplitSpec::Headings(2)).unwrap();
        assert_eq!(res.parts.len(), 3);
        assert_eq!(res.parts[0], "OEBPS/chapter-1.xhtml");
        assert_eq!(res.parts[1], "OEBPS/chapter-1_split_001.xhtml");
        assert_eq!(
            book.spine_paths()[..4],
            [
                "OEBPS/chapter-1.xhtml",
                "OEBPS/chapter-1_split_001.xhtml",
                "OEBPS/chapter-1_split_002.xhtml",
                "OEBPS/chapter-2.xhtml"
            ]
        );
        assert_eq!(all_text(&book), before);
        assert!(errors(&book).is_empty(), "{:?}", errors(&book));
        // links
        let c2 = book.archive.get_string("OEBPS/chapter-2.xhtml").unwrap();
        assert!(c2.contains("href=\"chapter-1_split_002.xhtml#c\""), "{c2}");
        assert!(
            c2.contains("href=\"chapter-1.xhtml\""),
            "plain links keep pointing at the first part"
        );
        // same-file fragment links turned cross-file, and unaffected ones left alone
        let p1 = book
            .archive
            .get_string("OEBPS/chapter-1_split_001.xhtml")
            .unwrap();
        assert!(p1.contains("href=\"chapter-1.xhtml#a\""), "{p1}");
        let p2 = book
            .archive
            .get_string("OEBPS/chapter-1_split_002.xhtml")
            .unwrap();
        assert!(p2.contains("href=\"chapter-1_split_001.xhtml#b\""), "{p2}");
        // toc
        let t = book.toc();
        assert_eq!(
            t.iter().find(|e| e.title == "Gamma").unwrap().href,
            "OEBPS/chapter-1_split_002.xhtml#c"
        );
        // manifest
        let item = book
            .package
            .item_by_path("OEBPS/chapter-1_split_001.xhtml")
            .unwrap();
        assert_eq!(item.media_type, "application/xhtml+xml");
        assert_eq!(item.id, "ch1_split_1");
        // each part is well-formed
        for p in &res.parts {
            parse_xml(&book.archive.get_string(p).unwrap()).unwrap();
        }
    }

    #[test]
    fn split_file_errors() {
        let mut book = make_book(&["<p>just one</p>"]);
        assert!(
            split_file(&mut book, "OEBPS/chapter-1.xhtml", &SplitSpec::Headings(3))
                .unwrap_err()
                .to_string()
                .starts_with("nothing to split")
        );
        assert!(split_file(&mut book, "OEBPS/none.xhtml", &SplitSpec::Headings(3)).is_err());
        assert!(split_file(&mut book, "OEBPS/style.css", &SplitSpec::Headings(3)).is_err());
        book.archive.set_string(
            "OEBPS/bad.xhtml",
            "<html><body><h1>a</h1><h1>b</h1><p></body></html>",
        );
        assert!(split_file(&mut book, "OEBPS/bad.xhtml", &SplitSpec::Headings(1)).is_err());
        book.archive.set_string(
            "OEBPS/loose.xhtml",
            "<html><body><h1>a</h1><p>x</p><h1>b</h1></body></html>",
        );
        assert!(
            split_file(&mut book, "OEBPS/loose.xhtml", &SplitSpec::Headings(1))
                .unwrap_err()
                .to_string()
                .contains("not in the manifest")
        );
    }

    #[test]
    fn merge_resolves_id_conflicts_and_links() {
        let mut book = make_book(&[
            "<h1 id=\"top\">One</h1><p id=\"dup\">first <a href=\"chapter-2.xhtml#dup\">to two dup</a></p>",
            "<h1 id=\"top2\">Two</h1><p id=\"dup\">second <a href=\"#dup\">self</a> <a href=\"chapter-1.xhtml#dup\">to one</a></p>",
            "<p>three <a href=\"chapter-2.xhtml\">to two</a> <a href=\"chapter-2.xhtml#dup\">to two dup</a></p>",
        ]);
        let toc = vec![
            bookx_core::toc::TocEntry {
                title: "One".into(),
                href: "OEBPS/chapter-1.xhtml".into(),
                children: vec![],
            },
            bookx_core::toc::TocEntry {
                title: "Two".into(),
                href: "OEBPS/chapter-2.xhtml".into(),
                children: vec![],
            },
        ];
        bookx_core::toc::write_toc(&mut book.archive, &book.package, &toc).unwrap();
        let before = all_text(&book);
        let res = merge_files(
            &mut book,
            &[
                "OEBPS/chapter-1.xhtml".into(),
                "OEBPS/chapter-2.xhtml".into(),
            ],
        )
        .unwrap();
        assert_eq!(res.removed, vec!["OEBPS/chapter-2.xhtml"]);
        assert_eq!(
            res.renamed_ids,
            vec![(
                "OEBPS/chapter-2.xhtml".to_string(),
                "dup".to_string(),
                "dup-2".to_string()
            )]
        );
        assert!(!book.archive.contains("OEBPS/chapter-2.xhtml"));
        assert_eq!(
            book.spine_paths(),
            vec!["OEBPS/chapter-1.xhtml", "OEBPS/chapter-3.xhtml"]
        );
        assert!(book.package.item_by_id("ch2").is_none());
        let merged = book.archive.get_string("OEBPS/chapter-1.xhtml").unwrap();
        parse_xml(&merged).unwrap();
        assert!(merged.contains("id=\"dup-2\""));
        assert!(
            merged.contains("href=\"#dup-2\""),
            "self link follows the renamed id: {merged}"
        );
        assert!(
            merged.contains("href=\"chapter-1.xhtml#dup\"")
                || merged.contains("href=\"#dup\"")
                || merged.contains("chapter-1.xhtml#dup")
        );
        assert!(merged.contains("<div id=\"chapter-2\"/>"));
        let c3 = book.archive.get_string("OEBPS/chapter-3.xhtml").unwrap();
        assert!(c3.contains("href=\"chapter-1.xhtml#chapter-2\""), "{c3}");
        assert!(c3.contains("href=\"chapter-1.xhtml#dup-2\""), "{c3}");
        let t = book.toc();
        assert_eq!(t[1].href, "OEBPS/chapter-1.xhtml#chapter-2");
        assert_eq!(all_text(&book), before);
        assert!(errors(&book).is_empty(), "{:?}", errors(&book));
    }

    #[test]
    fn merge_rebases_resources_and_styles_across_directories() {
        let mut book = make_book(&["<p>a</p>", "<p>b</p>"]);
        let png = crate::testutil::png(8, 8);
        book.archive.set("OEBPS/img/pic.png", png);
        book.archive.set_string("OEBPS/sub/x.css", "p{color:red}");
        let src = "<?xml version=\"1.0\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>t</title><link rel=\"stylesheet\" href=\"x.css\"/><link rel=\"stylesheet\" href=\"../style.css\"/></head><body class=\"k\"><p>b <img src=\"../img/pic.png\" alt=\"\"/></p></body></html>";
        book.archive.set_string("OEBPS/sub/c2.xhtml", src);
        let pkg = book.package.clone();
        pkg.add_manifest_item(
            &mut book.archive,
            "c2",
            "OEBPS/sub/c2.xhtml",
            "application/xhtml+xml",
            "",
        )
        .unwrap();
        book.reload_package().unwrap();
        merge_files(
            &mut book,
            &["OEBPS/chapter-1.xhtml".into(), "OEBPS/sub/c2.xhtml".into()],
        )
        .unwrap();
        let m = book.archive.get_string("OEBPS/chapter-1.xhtml").unwrap();
        assert!(m.contains("src=\"img/pic.png\""), "{m}");
        assert!(m.contains("<div class=\"k\">"));
        assert!(m.contains("href=\"sub/x.css\""), "{m}");
        assert_eq!(
            m.matches("style.css").count(),
            1,
            "already-linked stylesheets are not duplicated"
        );
        assert!(errors(&book).is_empty(), "{:?}", errors(&book));
    }

    #[test]
    fn merge_errors() {
        let mut book = make_book(&["<p>a</p>", "<p>b</p>"]);
        let a = "OEBPS/chapter-1.xhtml".to_string();
        let b = "OEBPS/chapter-2.xhtml".to_string();
        assert!(merge_files(&mut book, std::slice::from_ref(&a))
            .unwrap_err()
            .to_string()
            .contains("at least two"));
        assert!(merge_files(&mut book, &[a.clone(), a.clone()])
            .unwrap_err()
            .to_string()
            .contains("twice"));
        assert!(
            merge_files(&mut book, &[a.clone(), "OEBPS/style.css".into()])
                .unwrap_err()
                .to_string()
                .contains("not an XHTML")
        );
        assert!(
            merge_files(&mut book, &[a.clone(), "OEBPS/nav.xhtml".into()])
                .unwrap_err()
                .to_string()
                .contains("navigation")
        );
        book.archive
            .set_string("OEBPS/loose.xhtml", "<html><body/></html>");
        assert!(
            merge_files(&mut book, &[a.clone(), "OEBPS/loose.xhtml".into()])
                .unwrap_err()
                .to_string()
                .contains("not in the manifest")
        );
        book.archive
            .set_string(&b, "<html><body><p>unclosed</body></html>");
        assert!(merge_files(&mut book, &[a, b]).is_err());
    }

    /// Deterministic generator for the round-trip property test.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self, n: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) % n
        }
    }

    fn random_body(r: &mut Lcg, chapter: usize, n_ids: &mut usize) -> String {
        let mut s = String::new();
        let blocks = 4 + r.next(8) as usize;
        let mut depth_open = 0;
        for i in 0..blocks {
            match r.next(6) {
                0 => {
                    *n_ids += 1;
                    s.push_str(&format!(
                        "<h1 id=\"h{chapter}_{n_ids}\">Head {chapter}.{i}</h1>"
                    ));
                }
                1 => {
                    *n_ids += 1;
                    s.push_str(&format!(
                        "<h2 id=\"s{chapter}_{n_ids}\">Sub {chapter}.{i}</h2>"
                    ));
                }
                2 if depth_open < 2 => {
                    depth_open += 1;
                    s.push_str(&format!("<div class=\"d\" id=\"d{chapter}_{i}\">"));
                }
                3 if depth_open > 0 => {
                    depth_open -= 1;
                    s.push_str("</div>");
                }
                _ => {
                    *n_ids += 1;
                    s.push_str(&format!(
                        "<p id=\"p{chapter}_{n_ids}\">word{chapter}x{i} text &amp; more</p>"
                    ));
                }
            }
        }
        for _ in 0..depth_open {
            s.push_str("</div>");
        }
        s
    }

    fn add_links(bodies: &mut [String], r: &mut Lcg) {
        let ids: Vec<(usize, String)> = bodies
            .iter()
            .enumerate()
            .flat_map(|(c, b)| defined_ids(b).into_iter().map(move |id| (c, id)))
            .collect();
        for (c, body) in bodies.iter_mut().enumerate() {
            for _ in 0..3 {
                let (tc, id) = &ids[r.next(ids.len() as u64) as usize];
                let href = if *tc == c {
                    format!("#{id}")
                } else {
                    format!("chapter-{}.xhtml#{id}", tc + 1)
                };
                body.push_str(&format!("<p>link <a href=\"{href}\">to {id}</a></p>"));
            }
            if c > 0 && r.next(2) == 0 {
                body.push_str(&format!("<p><a href=\"chapter-{c}.xhtml\">prev</a></p>"));
            }
        }
    }

    #[test]
    fn split_then_merge_roundtrip_property() {
        let mut splits = 0;
        for seed in 1..=80u64 {
            let mut r = Lcg(seed * 7919);
            let mut counter = 0;
            let mut bodies: Vec<String> = (1..=3)
                .map(|c| random_body(&mut r, c, &mut counter))
                .collect();
            add_links(&mut bodies, &mut r);
            let refs: Vec<&str> = bodies.iter().map(|s| s.as_str()).collect();
            let mut book = make_book(&refs);
            assert!(errors(&book).is_empty(), "seed {seed}: {:?}", errors(&book));
            let text0 = all_text(&book);
            let level = 1 + r.next(2) as u8;
            let target = "OEBPS/chapter-2.xhtml";
            let split = split_file(&mut book, target, &SplitSpec::Headings(level));
            let Ok(split) = split else {
                continue; // nothing to split for this random chapter
            };
            splits += 1;
            assert_eq!(
                all_text(&book),
                text0,
                "seed {seed}: text preserved by split"
            );
            assert!(
                errors(&book).is_empty(),
                "seed {seed} after split: {:?}",
                errors(&book)
            );
            // spine order: parts are consecutive
            let spine = book.spine_paths();
            let pos = spine.iter().position(|p| p == target).unwrap();
            assert_eq!(&spine[pos..pos + split.parts.len()], &split.parts[..]);
            merge_files(&mut book, &split.parts).unwrap();
            assert_eq!(
                all_text(&book),
                text0,
                "seed {seed}: text preserved by merge"
            );
            assert!(
                errors(&book).is_empty(),
                "seed {seed} after merge: {:?}",
                errors(&book)
            );
            assert_eq!(book.spine_paths().len(), 3);
            let merged = book.archive.get_string(target).unwrap();
            parse_xml(&merged).unwrap();
            // every original id still exists exactly once
            let ids = defined_ids(&merged);
            let uniq: HashSet<&String> = ids.iter().collect();
            assert_eq!(
                ids.len(),
                uniq.len(),
                "seed {seed}: duplicate ids after merge"
            );
        }
        assert!(splits >= 30, "only {splits} seeds produced a split");
    }

    fn wrapped(body: &str) -> String {
        format!("<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>t</title></head><body>{body}</body></html>")
    }

    #[test]
    fn snap_moves_offsets_out_of_tags_only() {
        let src = wrapped("<p class=\"x\">hello <b>w</b></p><!-- note --><p>z</p>");
        let tag = src.find("class").unwrap();
        assert_eq!(snap_offset(&src, tag), src.find("<p class").unwrap());
        let close = src.find("</b>").unwrap() + 2;
        // out of the end tag, and since `</p>` follows directly, out of the paragraph too
        assert_eq!(snap_offset(&src, close), src.find("</p><!--").unwrap() + 4);
        let comment = src.find("note").unwrap();
        assert_eq!(snap_offset(&src, comment), src.find("<!--").unwrap());
        let text = src.find("ello").unwrap();
        assert_eq!(snap_offset(&src, text), text);
        assert_eq!(snap_offset(&src, 10_000), src.len());
    }

    #[test]
    fn snap_steps_out_of_the_edges_of_elements() {
        let src = wrapped("<p>one</p>\n<p>two <b>x</b></p><p>three</p>");
        // right after a start tag: split in front of the element
        let at = src.find("two").unwrap();
        assert_eq!(snap_offset(&src, at), src.find("<p>two").unwrap());
        // right before an end tag: split behind the element
        let end = src.find("</p><p>three").unwrap();
        assert_eq!(snap_offset(&src, end), end + 4);
        // blanks between the cursor and the tag do not matter
        let src2 = wrapped("<p>one</p><p>\n  two</p>");
        let at = src2.find("two").unwrap();
        assert_eq!(snap_offset(&src2, at), src2.find("<p>\n  two").unwrap());
        // nested starts all step out
        let src3 = wrapped("<p>a</p><div><p>b</p></div>");
        let at = src3.find("b</p>").unwrap();
        assert_eq!(snap_offset(&src3, at), src3.find("<div>").unwrap());
        // after a void element the text is not at an edge
        let src4 = wrapped("<p>a<br/>b</p>");
        let at = src4.find("b</p>").unwrap();
        assert_eq!(snap_offset(&src4, at), at);
    }

    #[test]
    fn splitting_inside_text_closes_and_reopens_the_open_tags() {
        let src = wrapped("<div id=\"w\"><p id=\"p\">hello <b>big</b> world</p></div>");
        let at = src.find("ig</b>").unwrap();
        let parts = split_document(&src, &[at]).unwrap();
        assert_eq!(parts.len(), 2);
        assert!(
            parts[0].contains("<b>b</b></p></div></body>"),
            "{}",
            parts[0]
        );
        // ids are not duplicated on the re-opened elements
        assert!(
            parts[1].contains("<div><p><b>ig</b> world</p></div>"),
            "{}",
            parts[1]
        );
        assert_eq!(
            normalized_text(&parts[0]).replace(' ', "")
                + &normalized_text(&parts[1]).replace(' ', ""),
            normalized_text(&src).replace(' ', "")
        );
    }

    #[test]
    fn at_offsets_snaps_and_drops_empty_parts() {
        let src = wrapped("<p>one</p><p>two</p>");
        let mid = src.find("<p>two").unwrap() + 2;
        let pts = split_points(
            &src,
            &SplitSpec::AtOffsets(vec![mid]),
            &PageBreakRules::default(),
        )
        .unwrap();
        assert_eq!(pts, [src.find("<p>two").unwrap()]);
        // at the very start of the body nothing would be left in the first part
        let start = body_bounds(&src).unwrap().0;
        let pts = split_points(
            &src,
            &SplitSpec::AtOffsets(vec![start]),
            &PageBreakRules::default(),
        )
        .unwrap();
        assert!(pts.is_empty());
    }

    #[test]
    fn split_file_at_the_cursor_migrates_links_spine_and_toc() {
        let mut book = make_book(&[
            "<h1 id=\"a\">Alpha</h1><p id=\"one\">one two three</p><p id=\"two\">four <a href=\"#one\">back</a></p>",
        ]);
        let path = "OEBPS/chapter-1.xhtml";
        let src = book.archive.get_string(path).unwrap();
        let toc = vec![
            bookx_core::toc::TocEntry {
                title: "Alpha".into(),
                href: format!("{path}#a"),
                children: vec![],
            },
            bookx_core::toc::TocEntry {
                title: "Two".into(),
                href: format!("{path}#two"),
                children: vec![],
            },
        ];
        bookx_core::toc::write_toc(&mut book.archive, &book.package, &toc).unwrap();
        let at = src.find("three").unwrap();
        let before = all_text(&book);
        let res = split_file(&mut book, path, &SplitSpec::AtOffsets(vec![at])).unwrap();
        assert_eq!(res.parts.len(), 2);
        assert_eq!(book.spine_paths().len(), 2);
        let second = book.archive.get_string(&res.parts[1]).unwrap();
        assert!(second.contains("<p>three</p>"), "{second}");
        assert!(second.contains("id=\"two\""));
        // the link to #one stays valid: the target is in the first part
        assert!(second.contains("href=\"chapter-1.xhtml#one\""), "{second}");
        let t = book.toc();
        assert_eq!(
            t.iter().find(|e| e.title == "Two").unwrap().href,
            format!("{}#two", res.parts[1])
        );
        assert_eq!(all_text(&book).replace(' ', ""), before.replace(' ', ""));
        assert!(errors(&book).is_empty(), "{:?}", errors(&book));
    }

    #[test]
    fn selectors_pick_elements_like_xpath() {
        let src = wrapped(
            "<p>intro</p><section class=\"part big\"><h2>A</h2><p>x</p></section><div class=\"part\"><h2 lang=\"en\">B</h2><p>y</p></div><p class=\"note\">n</p>",
        );
        let n = |sel: &str| {
            split_points(
                &src,
                &SplitSpec::Selector(sel.into()),
                &PageBreakRules::default(),
            )
            .map(|p| p.len())
        };
        assert_eq!(n("//h2").unwrap(), 2);
        assert_eq!(n("//section/h2").unwrap(), 1);
        assert_eq!(n("//div//h2").unwrap(), 1);
        assert_eq!(n("//*[@class='part']").unwrap(), 1);
        assert_eq!(n("//*[contains(@class,'part')]").unwrap(), 2);
        assert_eq!(n("//h2[@lang]").unwrap(), 1);
        assert_eq!(n("//p[@class=\"note\"]").unwrap(), 1);
        assert_eq!(n("div.part").unwrap(), 1);
        assert_eq!(n(".part").unwrap(), 2);
        assert!(n("//h2[").is_err());
        assert!(n("").is_err());
        assert!(n("//p[position()=1]").is_err());
    }

    #[test]
    fn by_class_splits_at_each_element_with_the_class() {
        let src = wrapped("<p>intro</p><div class=\"chap a\"><p>1</p></div><div class=\"chap\"><p>2</p></div><p class=\"chap\">3</p>");
        let pts = |tag: Option<&str>| {
            split_points(
                &src,
                &SplitSpec::ByClass {
                    tag: tag.map(str::to_string),
                    class: "chap".into(),
                },
                &PageBreakRules::default(),
            )
            .unwrap()
            .len()
        };
        assert_eq!(pts(None), 3);
        assert_eq!(pts(Some("div")), 2);
        assert_eq!(pts(Some("p")), 1);
    }

    #[test]
    fn the_preview_lists_the_parts_with_titles() {
        let src = wrapped("<h1>Alpha</h1><p>one</p><h1>Beta</h1><p>two words</p><p>tail only</p>");
        let pts = split_points(&src, &SplitSpec::Headings(1), &PageBreakRules::default()).unwrap();
        let parts = preview_parts(&src, &pts);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].title, "Alpha");
        assert_eq!(parts[1].title, "Beta");
        assert!(parts[1].chars > parts[0].chars);
        let mid = src.find("<p>tail").unwrap() + 3;
        let parts = preview_parts(&src, &[mid]);
        assert_eq!(parts.len(), 2);
        assert!(parts[1].title.starts_with("tail"), "{:?}", parts[1]);
    }
}
