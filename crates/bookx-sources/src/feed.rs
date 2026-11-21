//! Feed parsing (S4/S5): RSS 2.0 / RSS 1.0 (RDF), Atom and JSON Feed into one [`Feed`] model.
//!
//! The parser is forgiving: HTML entities that XML does not know (`&nbsp;`), bare ampersands,
//! stray control characters, byte-order marks, Latin-1 bytes and unparsable dates do not make a
//! feed fail; a feed fails only when it is not XML/JSON at all or not a feed.

use std::sync::OnceLock;

use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use regex::Regex;
use roxmltree::{Document, Node, ParsingOptions};
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Enclosure {
    pub url: String,
    pub mime: String,
    pub length: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub link: String,
    pub date: Option<DateTime<Utc>>,
    pub author: String,
    /// Full content as HTML (`content:encoded`, Atom `content`, JSON `content_html`), may be empty.
    pub content: String,
    /// Short description as HTML, may be empty.
    pub summary: String,
    pub enclosures: Vec<Enclosure>,
    pub categories: Vec<String>,
}

impl Entry {
    /// The best available body: full content, else the summary.
    pub fn body(&self) -> &str {
        if self.content.trim().is_empty() {
            &self.summary
        } else {
            &self.content
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Feed {
    pub title: String,
    pub link: String,
    pub description: String,
    pub entries: Vec<Entry>,
}

/// Which syntax a body looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedKind {
    Rss,
    Atom,
    Json,
}

// ---------------------------------------------------------------------------------------------
// dates
// ---------------------------------------------------------------------------------------------

fn zone_offset(abbr: &str) -> Option<&'static str> {
    Some(match abbr {
        "UT" | "UTC" | "GMT" | "Z" => "+0000",
        "EST" => "-0500",
        "EDT" => "-0400",
        "CST" => "-0600",
        "CDT" => "-0500",
        "MST" => "-0700",
        "MDT" => "-0600",
        "PST" => "-0800",
        "PDT" => "-0700",
        "BST" | "CET" if abbr == "BST" => "+0100",
        "CET" => "+0100",
        "CEST" | "EET" => "+0200",
        "EEST" => "+0300",
        "JST" => "+0900",
        _ => return None,
    })
}

/// Parses the date formats found in feeds (RFC 2822, RFC 3339, and common sloppy variants)
/// into UTC. Returns `None` when nothing fits.
pub fn parse_date(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let ok = |d: DateTime<Utc>| {
        let y = chrono::Datelike::year(&d);
        (1000..=9999).contains(&y).then_some(d)
    };
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return ok(d.with_timezone(&Utc));
    }
    if let Ok(d) = DateTime::parse_from_rfc2822(s) {
        return ok(d.with_timezone(&Utc));
    }
    // numeric offset for a trailing zone abbreviation
    let mut t = s.to_string();
    if let Some((head, last)) = s.rsplit_once(' ') {
        if let Some(off) = zone_offset(last) {
            t = format!("{head} {off}");
        }
    }
    // weekday names are redundant (and sometimes wrong)
    let no_weekday = match t.split_once(',') {
        Some((wd, rest))
            if wd.trim().len() <= 9 && wd.trim().chars().all(|c| c.is_alphabetic()) =>
        {
            rest.trim().to_string()
        }
        _ => t.clone(),
    };
    for cand in [&t, &no_weekday] {
        for f in [
            "%d %b %Y %H:%M:%S %z",
            "%d %b %Y %H:%M %z",
            "%d %B %Y %H:%M:%S %z",
            "%Y-%m-%dT%H:%M:%S%.f%z",
            "%Y-%m-%d %H:%M:%S%.f %z",
            "%Y-%m-%d %H:%M:%S%z",
        ] {
            if let Ok(d) = DateTime::parse_from_str(cand, f) {
                return ok(d.with_timezone(&Utc));
            }
        }
        for f in [
            "%d %b %Y %H:%M:%S",
            "%d %b %Y %H:%M",
            "%d %B %Y %H:%M:%S",
            "%Y-%m-%dT%H:%M:%S%.f",
            "%Y-%m-%d %H:%M:%S%.f",
            "%Y-%m-%dT%H:%M",
            "%Y-%m-%d %H:%M",
            "%a %b %e %H:%M:%S %Y",
            "%b %e %Y %H:%M:%S",
        ] {
            if let Ok(d) = NaiveDateTime::parse_from_str(cand, f) {
                return ok(d.and_utc());
            }
        }
        for f in [
            "%d %b %Y",
            "%d %B %Y",
            "%Y-%m-%d",
            "%Y/%m/%d",
            "%B %d, %Y",
            "%b %d, %Y",
            "%d.%m.%Y",
        ] {
            if let Ok(d) = NaiveDate::parse_from_str(cand, f) {
                return d.and_hms_opt(0, 0, 0).and_then(|d| ok(d.and_utc()));
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------------------------
// text helpers
// ---------------------------------------------------------------------------------------------

/// Plain text of a title that may carry markup or entities (`Tom &amp; Jerry`, `<b>x</b>`).
pub fn plain_text(s: &str) -> String {
    let s = s.trim();
    let t = if s.contains('&') || s.contains('<') {
        bookx_core::util::parse_html_fragment(s)
            .root_element()
            .text()
            .collect::<String>()
    } else {
        s.to_string()
    };
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn named_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "nbsp" => "\u{a0}",
        "mdash" => "\u{2014}",
        "ndash" => "\u{2013}",
        "hellip" => "\u{2026}",
        "copy" => "\u{a9}",
        "reg" => "\u{ae}",
        "trade" => "\u{2122}",
        "lsquo" => "\u{2018}",
        "rsquo" => "\u{2019}",
        "ldquo" => "\u{201c}",
        "rdquo" => "\u{201d}",
        "laquo" => "\u{ab}",
        "raquo" => "\u{bb}",
        "bull" => "\u{2022}",
        "middot" => "\u{b7}",
        "times" => "\u{d7}",
        "deg" => "\u{b0}",
        "euro" => "\u{20ac}",
        "pound" => "\u{a3}",
        "yen" => "\u{a5}",
        "cent" => "\u{a2}",
        "sect" => "\u{a7}",
        "para" => "\u{b6}",
        "eacute" => "\u{e9}",
        "egrave" => "\u{e8}",
        "agrave" => "\u{e0}",
        "aacute" => "\u{e1}",
        "uuml" => "\u{fc}",
        "ouml" => "\u{f6}",
        "auml" => "\u{e4}",
        "Uuml" => "\u{dc}",
        "Ouml" => "\u{d6}",
        "Auml" => "\u{c4}",
        "szlig" => "\u{df}",
        "ntilde" => "\u{f1}",
        "ccedil" => "\u{e7}",
        "iexcl" => "\u{a1}",
        "iquest" => "\u{bf}",
        _ => return None,
    })
}

/// Makes HTML-flavoured XML parseable: unknown named entities are resolved or kept as text, bare
/// ampersands escaped, invalid characters dropped. CDATA sections are left alone.
fn sanitize_xml(text: &str) -> String {
    static R: OnceLock<[Regex; 2]> = OnceLock::new();
    let [cdata, amp] = R.get_or_init(|| {
        [
            Regex::new(r"(?s)<!\[CDATA\[.*?\]\]>").expect("static regex"),
            Regex::new(r"&(#[0-9]+;|#[xX][0-9a-fA-F]+;|[A-Za-z][A-Za-z0-9]*;)?")
                .expect("static regex"),
        ]
    });
    let fix = |seg: &str, out: &mut String| {
        let mut last = 0;
        for m in amp.captures_iter(seg) {
            let whole = m.get(0).map(|w| w.range()).unwrap_or(0..0);
            out.push_str(&seg[last..whole.start]);
            last = whole.end;
            match m.get(1).map(|g| g.as_str()) {
                None => out.push_str("&amp;"),
                Some(ent) => {
                    let name = ent.trim_end_matches(';');
                    if ent.starts_with('#') || matches!(name, "amp" | "lt" | "gt" | "quot" | "apos")
                    {
                        out.push('&');
                        out.push_str(ent);
                    } else if let Some(r) = named_entity(name) {
                        out.push_str(r);
                    } else {
                        out.push_str("&amp;");
                        out.push_str(ent);
                    }
                }
            }
        }
        out.push_str(&seg[last..]);
    };
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    for m in cdata.find_iter(text) {
        fix(&text[pos..m.start()], &mut out);
        out.push_str(m.as_str());
        pos = m.end();
    }
    fix(&text[pos..], &mut out);
    out.chars()
        .filter(|c| !matches!(c, '\u{0}'..='\u{8}' | '\u{B}' | '\u{C}' | '\u{E}'..='\u{1F}' | '\u{FFFE}' | '\u{FFFF}'))
        .collect()
}

/// Decodes a response body: UTF-8 (BOM stripped), UTF-16 with BOM, else Latin-1.
pub fn decode_text(body: &[u8]) -> String {
    let body = body.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(body);
    if body.starts_with(&[0xFF, 0xFE]) || body.starts_with(&[0xFE, 0xFF]) {
        let le = body[0] == 0xFF;
        let units: Vec<u16> = body[2..]
            .chunks_exact(2)
            .map(|c| {
                if le {
                    u16::from_le_bytes([c[0], c[1]])
                } else {
                    u16::from_be_bytes([c[0], c[1]])
                }
            })
            .collect();
        return String::from_utf16_lossy(&units);
    }
    match std::str::from_utf8(body) {
        Ok(s) => s.to_string(),
        // Latin-1 / Windows-1252 are by far the most common legacy encodings in old feeds
        Err(_) => body.iter().map(|&b| b as char).collect(),
    }
}

/// Which syntax `body` has, judging by its first bytes.
pub fn detect(body: &[u8]) -> Option<FeedKind> {
    let head = String::from_utf8_lossy(&body[..body.len().min(4096)]).to_lowercase();
    let t = head.trim_start_matches('\u{feff}').trim_start();
    if t.starts_with('{') {
        return Some(FeedKind::Json);
    }
    if t.contains("<rss") || t.contains("<rdf:rdf") || t.contains("<channel") {
        return Some(FeedKind::Rss);
    }
    if t.contains("<feed") {
        return Some(FeedKind::Atom);
    }
    None
}

/// Parses a feed of any supported kind.
pub fn parse_feed(body: &[u8]) -> Result<Feed> {
    let text = decode_text(body);
    let trimmed = text.trim_start();
    if trimmed.is_empty() {
        bail!("feed is empty");
    }
    if trimmed.starts_with('{') {
        return parse_json_feed(trimmed);
    }
    let xml = sanitize_xml(trimmed);
    let opts = ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let doc = Document::parse_with_options(&xml, opts)
        .map_err(|e| anyhow!("feed is not well-formed XML: {e}"))?;
    let root = doc.root_element();
    match root.tag_name().name() {
        "rss" | "RDF" => parse_rss(root),
        "feed" => parse_atom(root, &xml),
        other => bail!("not a feed (root element `{other}`)"),
    }
}

fn kids<'a, 'b>(n: Node<'a, 'b>, local: &'a str) -> impl Iterator<Item = Node<'a, 'b>> {
    n.children()
        .filter(move |c| c.is_element() && c.tag_name().name() == local)
}

fn text_of(n: Node) -> String {
    n.descendants()
        .filter(|d| d.is_text())
        .filter_map(|d| d.text())
        .collect::<String>()
        .trim()
        .to_string()
}

fn child_text(n: Node, local: &str) -> String {
    kids(n, local)
        .map(text_of)
        .find(|t| !t.is_empty())
        .unwrap_or_default()
}

fn child_text_ns(n: Node, local: &str, ns_part: &str) -> String {
    n.children()
        .filter(|c| c.is_element() && c.tag_name().name() == local)
        .filter(|c| {
            c.tag_name()
                .namespace()
                .is_some_and(|ns| ns.contains(ns_part))
        })
        .map(text_of)
        .find(|t| !t.is_empty())
        .unwrap_or_default()
}

/// RSS `author` is often `mail@host (Name)` or `Name <mail@host>`.
fn author_name(raw: &str) -> String {
    let raw = raw.trim();
    if let (Some(a), Some(b)) = (raw.find('('), raw.rfind(')')) {
        if a < b && raw.contains('@') {
            return raw[a + 1..b].trim().to_string();
        }
    }
    if let Some(a) = raw.find('<') {
        if raw.contains('@') && a > 0 {
            return raw[..a].trim().trim_matches('"').to_string();
        }
    }
    if raw.contains('@') && !raw.contains(' ') {
        return String::new();
    }
    raw.to_string()
}

fn parse_rss(root: Node) -> Result<Feed> {
    let channel = if root.tag_name().name() == "rss" {
        kids(root, "channel")
            .next()
            .ok_or_else(|| anyhow!("RSS feed has no <channel>"))?
    } else {
        kids(root, "channel").next().unwrap_or(root)
    };
    let mut feed = Feed {
        title: plain_text(&child_text(channel, "title")),
        link: child_text(channel, "link"),
        description: plain_text(&child_text(channel, "description")),
        entries: Vec::new(),
    };
    let channel_author = {
        let a = child_text_ns(channel, "creator", "purl.org/dc");
        if a.is_empty() {
            author_name(&child_text(channel, "managingEditor"))
        } else {
            a
        }
    };
    // RSS 1.0 has items next to the channel
    let items: Vec<Node> = kids(channel, "item").chain(kids(root, "item")).collect();
    for it in items {
        let guid_node = kids(it, "guid").next();
        let guid = guid_node.map(text_of).unwrap_or_default();
        let mut link = child_text(it, "link");
        if link.is_empty() {
            // Atom link inside RSS
            link = it
                .children()
                .filter(|c| c.is_element() && c.tag_name().name() == "link")
                .find_map(|c| c.attribute("href"))
                .unwrap_or("")
                .to_string();
        }
        if link.is_empty() {
            let permalink = guid_node.and_then(|g| g.attribute("isPermaLink")) != Some("false");
            if permalink && (guid.starts_with("http://") || guid.starts_with("https://")) {
                link = guid.clone();
            }
        }
        let date_raw = [
            child_text(it, "pubDate"),
            child_text_ns(it, "date", "purl.org/dc"),
            child_text(it, "published"),
            child_text(it, "updated"),
        ]
        .into_iter()
        .find(|d| !d.is_empty())
        .unwrap_or_default();
        let author = {
            let dc = child_text_ns(it, "creator", "purl.org/dc");
            let a = if dc.is_empty() {
                author_name(&child_text(it, "author"))
            } else {
                dc
            };
            if a.is_empty() {
                channel_author.clone()
            } else {
                a
            }
        };
        let mut enclosures: Vec<Enclosure> = Vec::new();
        for e in kids(it, "enclosure") {
            if let Some(url) = e.attribute("url") {
                enclosures.push(Enclosure {
                    url: url.to_string(),
                    mime: e.attribute("type").unwrap_or("").to_string(),
                    length: e.attribute("length").and_then(|l| l.trim().parse().ok()),
                });
            }
        }
        for m in it.descendants().filter(|d| {
            d.is_element()
                && matches!(d.tag_name().name(), "content" | "thumbnail")
                && d.tag_name()
                    .namespace()
                    .is_some_and(|ns| ns.contains("search.yahoo.com/mrss"))
        }) {
            if let Some(url) = m.attribute("url") {
                if !enclosures.iter().any(|e| e.url == url) {
                    enclosures.push(Enclosure {
                        url: url.to_string(),
                        mime: m.attribute("type").unwrap_or("").to_string(),
                        length: m.attribute("fileSize").and_then(|l| l.trim().parse().ok()),
                    });
                }
            }
        }
        let categories = kids(it, "category")
            .map(text_of)
            .filter(|c| !c.is_empty())
            .collect();
        let entry = Entry {
            id: if guid.is_empty() { link.clone() } else { guid },
            title: plain_text(&child_text(it, "title")),
            link: link.trim().to_string(),
            date: parse_date(&date_raw),
            author,
            content: child_text_ns(it, "encoded", "purl.org/rss/1.0/modules/content"),
            summary: child_text(it, "description"),
            enclosures,
            categories,
        };
        feed.entries.push(entry);
    }
    Ok(feed)
}

fn atom_text(n: Node, xml: &str) -> String {
    match n.attribute("type") {
        Some("xhtml") => {
            // the payload is a <div> in the XHTML namespace: keep its inner markup
            let div = n.children().find(|c| c.is_element());
            match div {
                Some(d) => {
                    let inner: Vec<_> = d.children().collect();
                    match (inner.first(), inner.last()) {
                        (Some(a), Some(b)) => {
                            xml[a.range().start..b.range().end].trim().to_string()
                        }
                        _ => String::new(),
                    }
                }
                None => text_of(n),
            }
        }
        Some("text") | None => {
            // plain text may still contain markup in sloppy feeds; keep it as it is
            text_of(n)
        }
        _ => text_of(n),
    }
}

fn parse_atom(root: Node, xml: &str) -> Result<Feed> {
    let link_of = |n: Node| -> String {
        let mut alt = None;
        let mut any = None;
        for l in kids(n, "link") {
            let Some(h) = l.attribute("href") else {
                continue;
            };
            match l.attribute("rel") {
                None | Some("alternate") => {
                    if alt.is_none() {
                        alt = Some(h)
                    }
                }
                Some("self") | Some("enclosure") | Some("replies") | Some("edit") => {}
                _ => {
                    if any.is_none() {
                        any = Some(h)
                    }
                }
            }
        }
        alt.or(any).unwrap_or("").trim().to_string()
    };
    let feed_author = kids(root, "author")
        .map(|a| child_text(a, "name"))
        .find(|n| !n.is_empty())
        .unwrap_or_default();
    let mut feed = Feed {
        title: plain_text(
            &kids(root, "title")
                .next()
                .map(|t| atom_text(t, xml))
                .unwrap_or_default(),
        ),
        link: link_of(root),
        description: plain_text(&child_text(root, "subtitle")),
        entries: Vec::new(),
    };
    for e in kids(root, "entry") {
        let date_raw = ["published", "updated", "created", "issued", "modified"]
            .iter()
            .map(|n| child_text(e, n))
            .find(|d| !d.is_empty())
            .unwrap_or_default();
        let author = kids(e, "author")
            .map(|a| child_text(a, "name"))
            .find(|n| !n.is_empty())
            .unwrap_or_else(|| feed_author.clone());
        let enclosures = kids(e, "link")
            .filter(|l| l.attribute("rel") == Some("enclosure"))
            .filter_map(|l| {
                Some(Enclosure {
                    url: l.attribute("href")?.to_string(),
                    mime: l.attribute("type").unwrap_or("").to_string(),
                    length: l.attribute("length").and_then(|v| v.trim().parse().ok()),
                })
            })
            .collect();
        let link = link_of(e);
        feed.entries.push(Entry {
            id: {
                let id = child_text(e, "id");
                if id.is_empty() {
                    link.clone()
                } else {
                    id
                }
            },
            title: plain_text(
                &kids(e, "title")
                    .next()
                    .map(|t| atom_text(t, xml))
                    .unwrap_or_default(),
            ),
            link,
            date: parse_date(&date_raw),
            author,
            content: kids(e, "content")
                .next()
                .map(|c| atom_text(c, xml))
                .unwrap_or_default(),
            summary: kids(e, "summary")
                .next()
                .map(|c| atom_text(c, xml))
                .unwrap_or_default(),
            enclosures,
            categories: kids(e, "category")
                .filter_map(|c| {
                    c.attribute("term")
                        .or_else(|| c.attribute("label"))
                        .map(String::from)
                })
                .filter(|c| !c.is_empty())
                .collect(),
        });
    }
    Ok(feed)
}

fn json_str(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

fn parse_json_feed(text: &str) -> Result<Feed> {
    let v: Value =
        serde_json::from_str(text).map_err(|e| anyhow!("feed is not valid JSON: {e}"))?;
    let version = json_str(&v, "version");
    if !version.contains("jsonfeed.org") && v.get("items").is_none() {
        bail!("not a JSON Feed (no `version` or `items`)");
    }
    let items = v
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("JSON Feed has no `items` array"))?;
    let feed_author = v
        .get("authors")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .or_else(|| v.get("author"))
        .map(|a| json_str(a, "name"))
        .unwrap_or_default();
    let mut feed = Feed {
        title: plain_text(&json_str(&v, "title")),
        link: json_str(&v, "home_page_url"),
        description: plain_text(&json_str(&v, "description")),
        entries: Vec::new(),
    };
    for it in items {
        if !it.is_object() {
            continue;
        }
        let author = it
            .get("authors")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .or_else(|| it.get("author"))
            .map(|a| json_str(a, "name"))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| feed_author.clone());
        let content = {
            let h = json_str(it, "content_html");
            if h.is_empty() {
                let t = json_str(it, "content_text");
                if t.is_empty() {
                    String::new()
                } else {
                    format!(
                        "<p>{}</p>",
                        t.replace('&', "&amp;")
                            .replace('<', "&lt;")
                            .replace('>', "&gt;")
                            .replace("\n\n", "</p><p>")
                    )
                }
            } else {
                h
            }
        };
        let link = {
            let u = json_str(it, "url");
            if u.is_empty() {
                json_str(it, "external_url")
            } else {
                u
            }
        };
        feed.entries.push(Entry {
            id: {
                let id = it
                    .get("id")
                    .map(|i| {
                        i.as_str()
                            .map(String::from)
                            .unwrap_or_else(|| i.to_string())
                    })
                    .unwrap_or_default();
                if id.is_empty() {
                    link.clone()
                } else {
                    id
                }
            },
            title: plain_text(&json_str(it, "title")),
            link,
            date: parse_date(&json_str(it, "date_published"))
                .or_else(|| parse_date(&json_str(it, "date_modified"))),
            author,
            content,
            summary: json_str(it, "summary"),
            enclosures: it
                .get("attachments")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| {
                            Some(Enclosure {
                                url: x.get("url")?.as_str()?.to_string(),
                                mime: json_str(x, "mime_type"),
                                length: x.get("size_in_bytes").and_then(Value::as_u64),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
            categories: it
                .get("tags")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default(),
        });
    }
    Ok(feed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, mi, s).unwrap()
    }

    const RSS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:atom="http://www.w3.org/2005/Atom" xmlns:media="http://search.yahoo.com/mrss/">
<channel>
  <title>Example &amp; Co News</title>
  <link>https://example.com/</link>
  <description><![CDATA[All the <b>news</b>]]></description>
  <atom:link href="https://example.com/feed.xml" rel="self" type="application/rss+xml"/>
  <managingEditor>editor@example.com (Chief Editor)</managingEditor>
  <item>
    <title><![CDATA[Tom &amp; Jerry: <i>the</i> return]]></title>
    <link>https://example.com/a/1?x=1&amp;y=2</link>
    <guid isPermaLink="false">post-1</guid>
    <pubDate>Tue, 05 Mar 2024 10:00:00 GMT</pubDate>
    <dc:creator><![CDATA[Ada Lovelace]]></dc:creator>
    <category>Tech</category><category>News</category>
    <description>Short &lt;p&gt;summary&lt;/p&gt; with &nbsp;nbsp &mdash; and AT&T</description>
    <content:encoded><![CDATA[<p>Full <b>content</b> here &amp; there</p><img src="/i.png">]]></content:encoded>
    <enclosure url="https://example.com/a.mp3" length="12345" type="audio/mpeg"/>
    <media:content url="https://example.com/m.jpg" type="image/jpeg" fileSize="999"/>
  </item>
  <item>
    <title>Second</title>
    <guid>https://example.com/a/2</guid>
    <pubDate>not a date</pubDate>
    <author>bob@example.com (Bob)</author>
    <description>Only summary</description>
  </item>
  <item>
    <title>Third  with
    newline</title>
    <link>https://example.com/a/3</link>
    <pubDate>Wed, 06 Mar 2024 11:30 +0100</pubDate>
  </item>
</channel></rss>"#;

    #[test]
    fn rss_channel_and_items() {
        let f = parse_feed(RSS.as_bytes()).unwrap();
        assert_eq!(f.title, "Example & Co News");
        assert_eq!(f.link, "https://example.com/");
        assert_eq!(f.description, "All the news");
        assert_eq!(f.entries.len(), 3);
        let e = &f.entries[0];
        assert_eq!(e.title, "Tom & Jerry: the return");
        assert_eq!(e.link, "https://example.com/a/1?x=1&y=2");
        assert_eq!(e.id, "post-1");
        assert_eq!(e.date, Some(utc(2024, 3, 5, 10, 0, 0)));
        assert_eq!(e.author, "Ada Lovelace");
        assert_eq!(e.categories, vec!["Tech", "News"]);
        assert_eq!(
            e.content,
            "<p>Full <b>content</b> here &amp; there</p><img src=\"/i.png\">"
        );
        assert_eq!(e.body(), e.content);
        assert!(
            e.summary.contains("<p>summary</p>")
                && e.summary.contains("\u{a0}nbsp \u{2014} and AT&T"),
            "{}",
            e.summary
        );
        assert_eq!(e.enclosures.len(), 2);
        assert_eq!(
            e.enclosures[0],
            Enclosure {
                url: "https://example.com/a.mp3".into(),
                mime: "audio/mpeg".into(),
                length: Some(12345)
            }
        );
        assert_eq!(e.enclosures[1].url, "https://example.com/m.jpg");
        assert_eq!(e.enclosures[1].length, Some(999));
    }

    #[test]
    fn rss_fallbacks_and_bad_dates() {
        let f = parse_feed(RSS.as_bytes()).unwrap();
        let e = &f.entries[1];
        assert_eq!(
            e.link, "https://example.com/a/2",
            "permalink guid replaces missing link"
        );
        assert_eq!(e.id, "https://example.com/a/2");
        assert_eq!(e.date, None, "bad date does not fail the feed");
        assert_eq!(e.author, "Bob");
        assert_eq!(e.body(), "Only summary");
        let e = &f.entries[2];
        assert_eq!(e.title, "Third with newline");
        assert_eq!(
            e.date,
            Some(utc(2024, 3, 6, 10, 30, 0)),
            "seconds optional, offset applied"
        );
        assert_eq!(e.author, "Chief Editor", "channel editor as fallback");
        assert_eq!(e.id, "https://example.com/a/3", "link doubles as id");
    }

    #[test]
    fn rss_non_permalink_guid_is_not_a_link() {
        let x = r#"<rss version="2.0"><channel><title>t</title><item><title>a</title><guid isPermaLink="false">http://not.a/link</guid></item></channel></rss>"#;
        let f = parse_feed(x.as_bytes()).unwrap();
        assert_eq!(f.entries[0].link, "");
        assert_eq!(f.entries[0].id, "http://not.a/link");
    }

    #[test]
    fn rss_1_0_rdf() {
        let x = r#"<?xml version="1.0"?>
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns="http://purl.org/rss/1.0/" xmlns:dc="http://purl.org/dc/elements/1.1/">
<channel rdf:about="http://x.org/"><title>RDF feed</title><link>http://x.org/</link></channel>
<item rdf:about="http://x.org/1"><title>One</title><link>http://x.org/1</link><dc:date>2024-01-02T03:04:05Z</dc:date><dc:creator>Cy</dc:creator></item>
</rdf:RDF>"#;
        let f = parse_feed(x.as_bytes()).unwrap();
        assert_eq!(f.title, "RDF feed");
        assert_eq!(f.entries.len(), 1);
        assert_eq!(f.entries[0].date, Some(utc(2024, 1, 2, 3, 4, 5)));
        assert_eq!(f.entries[0].author, "Cy");
    }

    const ATOM: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:xhtml="http://www.w3.org/1999/xhtml">
  <title type="html">Blog &amp;amp; Stuff</title>
  <subtitle>Things</subtitle>
  <link rel="self" href="https://b.example/atom.xml"/>
  <link rel="alternate" type="text/html" href="https://b.example/"/>
  <author><name>Feed Author</name></author>
  <entry>
    <title>Hello &lt;em&gt;World&lt;/em&gt;</title>
    <link rel="replies" href="https://b.example/1#comments"/>
    <link href="https://b.example/1"/>
    <link rel="enclosure" type="audio/ogg" length="77" href="https://b.example/1.ogg"/>
    <id>tag:b.example,2024:1</id>
    <published>2024-03-05T10:00:00+02:00</published>
    <updated>2024-03-06T10:00:00Z</updated>
    <author><name>Entry Author</name></author>
    <category term="rust"/><category term="atom" label="Atom"/>
    <summary type="text">Plain &amp; simple</summary>
    <content type="xhtml"><div xmlns="http://www.w3.org/1999/xhtml"><p>Para <b>one</b></p><p>Para two &amp; more</p></div></content>
  </entry>
  <entry>
    <title>Second</title>
    <link rel="alternate" href="https://b.example/2"/>
    <updated>2024-03-07</updated>
    <content type="html">&lt;p&gt;Escaped &lt;b&gt;html&lt;/b&gt;&lt;/p&gt;</content>
  </entry>
</feed>"#;

    #[test]
    fn atom_feed() {
        let f = parse_feed(ATOM.as_bytes()).unwrap();
        assert_eq!(f.title, "Blog & Stuff");
        assert_eq!(f.link, "https://b.example/");
        assert_eq!(f.description, "Things");
        assert_eq!(f.entries.len(), 2);
        let e = &f.entries[0];
        assert_eq!(e.title, "Hello World");
        assert_eq!(
            e.link, "https://b.example/1",
            "rel=replies skipped, plain link used"
        );
        assert_eq!(e.id, "tag:b.example,2024:1");
        assert_eq!(
            e.date,
            Some(utc(2024, 3, 5, 8, 0, 0)),
            "published wins over updated"
        );
        assert_eq!(e.author, "Entry Author");
        assert_eq!(e.categories, vec!["rust", "atom"]);
        assert_eq!(e.summary, "Plain & simple");
        assert_eq!(
            e.content,
            "<p>Para <b>one</b></p><p>Para two &amp; more</p>"
        );
        assert_eq!(
            e.enclosures,
            vec![Enclosure {
                url: "https://b.example/1.ogg".into(),
                mime: "audio/ogg".into(),
                length: Some(77)
            }]
        );
        let e = &f.entries[1];
        assert_eq!(e.author, "Feed Author", "feed author as fallback");
        assert_eq!(e.date, Some(utc(2024, 3, 7, 0, 0, 0)));
        assert_eq!(e.content, "<p>Escaped <b>html</b></p>");
        assert_eq!(e.id, "https://b.example/2");
    }

    const JSON: &str = r#"{
      "version": "https://jsonfeed.org/version/1.1",
      "title": "JSON &amp; Co", "home_page_url": "https://j.example/",
      "authors": [{"name": "Feed Person"}],
      "items": [
        {"id": "1", "url": "https://j.example/1", "title": "One", "content_html": "<p>Hi</p>",
         "date_published": "2024-03-05T10:00:00-05:00", "authors": [{"name": "Item Person"}],
         "tags": ["a", "b"], "attachments": [{"url": "https://j.example/f.mp3", "mime_type": "audio/mpeg", "size_in_bytes": 5}]},
        {"id": 2, "external_url": "https://other/2", "title": "Two", "content_text": "Line one\n\nLine <two> & more", "date_modified": "2024-03-06T00:00:00Z"},
        {"title": "No id"},
        "garbage",
        42
      ]
    }"#;

    #[test]
    fn json_feed() {
        let f = parse_feed(JSON.as_bytes()).unwrap();
        assert_eq!(f.title, "JSON & Co");
        assert_eq!(f.link, "https://j.example/");
        assert_eq!(f.entries.len(), 3, "non-object items are skipped");
        let e = &f.entries[0];
        assert_eq!(
            (e.id.as_str(), e.title.as_str(), e.author.as_str()),
            ("1", "One", "Item Person")
        );
        assert_eq!(e.date, Some(utc(2024, 3, 5, 15, 0, 0)));
        assert_eq!(e.content, "<p>Hi</p>");
        assert_eq!(e.categories, vec!["a", "b"]);
        assert_eq!(e.enclosures[0].length, Some(5));
        let e = &f.entries[1];
        assert_eq!(e.id, "2", "numeric ids are accepted");
        assert_eq!(e.link, "https://other/2");
        assert_eq!(e.author, "Feed Person");
        assert_eq!(e.date, Some(utc(2024, 3, 6, 0, 0, 0)));
        assert_eq!(
            e.content,
            "<p>Line one</p><p>Line &lt;two&gt; &amp; more</p>"
        );
        assert_eq!(f.entries[2].title, "No id");
    }

    #[test]
    fn json_feed_errors() {
        assert!(parse_feed(br#"{"hello": 1}"#)
            .unwrap_err()
            .to_string()
            .contains("not a JSON Feed"));
        assert!(
            parse_feed(br#"{"version":"https://jsonfeed.org/version/1","items":5}"#)
                .unwrap_err()
                .to_string()
                .contains("items")
        );
        assert!(parse_feed(b"{ nope")
            .unwrap_err()
            .to_string()
            .contains("not valid JSON"));
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        for (body, msg) in [
            (&b""[..], "empty"),
            (b"   \n ", "empty"),
            (b"<html><body>hi</body></html>", "not a feed"),
            (b"<rss version=\"2.0\"></rss>", "no <channel>"),
            (b"<rss><channel><title>x</title></rss>", "not well-formed"),
            (b"plain text", "not well-formed"),
            (b"\x00\x01\x02", "not well-formed"),
        ] {
            let e = parse_feed(body).unwrap_err().to_string();
            assert!(e.contains(msg), "{:?}: {e}", String::from_utf8_lossy(body));
        }
    }

    #[test]
    fn empty_feeds_parse() {
        let f = parse_feed(b"<rss version=\"2.0\"><channel><title>Empty</title></channel></rss>")
            .unwrap();
        assert_eq!(f.title, "Empty");
        assert!(f.entries.is_empty());
        let f = parse_feed(b"<feed xmlns=\"http://www.w3.org/2005/Atom\"><title>A</title></feed>")
            .unwrap();
        assert!(f.entries.is_empty());
    }

    #[test]
    fn bom_leading_whitespace_and_control_chars() {
        let mut body = b"\xEF\xBB\xBF\n\n  ".to_vec();
        body.extend_from_slice(b"<?xml version=\"1.0\"?><rss version=\"2.0\"><channel><title>T\x0b\x01</title><item><title>A\x08B</title></item></channel></rss>");
        let f = parse_feed(&body).unwrap();
        assert_eq!(f.title, "T");
        assert_eq!(f.entries[0].title, "AB");
    }

    #[test]
    fn latin1_and_utf16_bodies() {
        let mut latin = b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><rss version=\"2.0\"><channel><title>Caf".to_vec();
        latin.push(0xE9);
        latin.extend_from_slice(b"</title></channel></rss>");
        assert_eq!(parse_feed(&latin).unwrap().title, "Caf\u{e9}");
        let xml =
            "<rss version=\"2.0\"><channel><title>Wide \u{65E5}\u{672C}</title></channel></rss>";
        let mut u16le = vec![0xFF, 0xFE];
        for u in xml.encode_utf16() {
            u16le.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(parse_feed(&u16le).unwrap().title, "Wide \u{65E5}\u{672C}");
        let mut u16be = vec![0xFE, 0xFF];
        for u in xml.encode_utf16() {
            u16be.extend_from_slice(&u.to_be_bytes());
        }
        assert_eq!(parse_feed(&u16be).unwrap().title, "Wide \u{65E5}\u{672C}");
    }

    #[test]
    fn unknown_entities_and_bare_ampersands() {
        let x = "<rss version=\"2.0\"><channel><title>A &foo; B &amp; C &#169; &#xA9; AT&T x&y</title></channel></rss>";
        let f = parse_feed(x.as_bytes()).unwrap();
        assert_eq!(f.title, "A &foo; B & C \u{a9} \u{a9} AT&T x&y");
    }

    #[test]
    fn cdata_is_not_touched() {
        let x = "<rss version=\"2.0\"><channel><title>t</title><item><link><![CDATA[https://x.org/?a=1&b=2]]></link></item></channel></rss>";
        assert_eq!(
            parse_feed(x.as_bytes()).unwrap().entries[0].link,
            "https://x.org/?a=1&b=2"
        );
    }

    #[test]
    fn doctype_is_accepted_and_entity_bombs_are_bounded() {
        let x = "<!DOCTYPE rss [<!ENTITY a \"aaaaaaaaaa\"><!ENTITY b \"&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;\"><!ENTITY c \"&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;\"><!ENTITY d \"&c;&c;&c;&c;&c;&c;&c;&c;&c;&c;\"><!ENTITY e \"&d;&d;&d;&d;&d;&d;&d;&d;&d;&d;\"><!ENTITY f \"&e;&e;&e;&e;&e;&e;&e;&e;&e;&e;\">]><rss version=\"2.0\"><channel><title>&f;&f;&f;&f;&f;&f;&f;&f;&f;&f;</title></channel></rss>";
        // either parsed or rejected, but never hangs or panics
        let _ = parse_feed(x.as_bytes());
        let ok = "<!DOCTYPE rss><rss version=\"2.0\"><channel><title>fine</title></channel></rss>";
        assert_eq!(parse_feed(ok.as_bytes()).unwrap().title, "fine");
    }

    #[test]
    fn huge_feed() {
        let mut x = String::from("<rss version=\"2.0\"><channel><title>Big</title>");
        for i in 0..5000 {
            x.push_str(&format!("<item><title>Item {i}</title><link>https://x.org/{i}</link><pubDate>Tue, 05 Mar 2024 10:00:00 GMT</pubDate><description>d{i}</description></item>"));
        }
        x.push_str("</channel></rss>");
        let f = parse_feed(x.as_bytes()).unwrap();
        assert_eq!(f.entries.len(), 5000);
        assert_eq!(f.entries[4999].title, "Item 4999");
    }

    #[test]
    fn detect_kinds() {
        assert_eq!(detect(RSS.as_bytes()), Some(FeedKind::Rss));
        assert_eq!(detect(ATOM.as_bytes()), Some(FeedKind::Atom));
        assert_eq!(detect(JSON.as_bytes()), Some(FeedKind::Json));
        assert_eq!(detect(b"\xEF\xBB\xBF  {\"a\":1}"), Some(FeedKind::Json));
        assert_eq!(detect(b"<html></html>"), None);
        assert_eq!(detect(b""), None);
    }

    #[test]
    fn date_formats() {
        let want = Some(utc(2024, 3, 5, 10, 0, 0));
        for s in [
            "Tue, 05 Mar 2024 10:00:00 GMT",
            "Tue, 05 Mar 2024 10:00:00 +0000",
            "Tue, 05 Mar 2024 10:00:00 UT",
            "Tue, 05 Mar 2024 05:00:00 EST",
            "Tue, 05 Mar 2024 02:00:00 PST",
            "Tue, 05 Mar 2024 12:00:00 CEST",
            "Mon, 05 Mar 2024 10:00:00 GMT",
            "05 Mar 2024 10:00:00 GMT",
            "05 Mar 2024 10:00:00",
            "2024-03-05T10:00:00Z",
            "2024-03-05T11:00:00+01:00",
            "2024-03-05T10:00:00",
            "2024-03-05T10:00:00.000Z",
            "2024-03-05 10:00:00",
            "2024-03-05 10:00",
            "  2024-03-05T10:00:00Z  ",
            "Tue Mar  5 10:00:00 2024",
        ] {
            assert_eq!(parse_date(s), want, "{s:?}");
        }
        assert_eq!(parse_date("2024-03-05"), Some(utc(2024, 3, 5, 0, 0, 0)));
        assert_eq!(parse_date("05 March 2024"), Some(utc(2024, 3, 5, 0, 0, 0)));
        assert_eq!(parse_date("March 5, 2024"), Some(utc(2024, 3, 5, 0, 0, 0)));
        assert_eq!(parse_date("2024/03/05"), Some(utc(2024, 3, 5, 0, 0, 0)));
        assert_eq!(parse_date("05.03.2024"), Some(utc(2024, 3, 5, 0, 0, 0)));
    }

    #[test]
    fn bad_dates() {
        for s in [
            "",
            "   ",
            "yesterday",
            "32 Mar 2024",
            "2024-13-45",
            "0000-00-00",
            "Tue, 05 Mar 0001 10:00:00 GMT",
            "💥",
        ] {
            assert_eq!(parse_date(s), None, "{s:?}");
        }
    }

    #[test]
    fn plain_text_cases() {
        assert_eq!(plain_text("Tom &amp; Jerry"), "Tom & Jerry");
        assert_eq!(plain_text("<b>bold</b>  and\n text"), "bold and text");
        assert_eq!(plain_text("  plain "), "plain");
        assert_eq!(plain_text("&lt;3"), "<3");
        assert_eq!(plain_text(""), "");
    }

    #[test]
    fn author_name_forms() {
        assert_eq!(author_name("joe@x.com (Joe Smith)"), "Joe Smith");
        assert_eq!(author_name("Joe Smith <joe@x.com>"), "Joe Smith");
        assert_eq!(author_name("\"Joe Smith\" <joe@x.com>"), "Joe Smith");
        assert_eq!(author_name("joe@x.com"), "");
        assert_eq!(author_name("Joe Smith"), "Joe Smith");
    }

    #[test]
    fn entry_body_prefers_content() {
        let e = Entry {
            content: " ".into(),
            summary: "s".into(),
            ..Default::default()
        };
        assert_eq!(e.body(), "s");
        let e = Entry {
            content: "c".into(),
            summary: "s".into(),
            ..Default::default()
        };
        assert_eq!(e.body(), "c");
    }
}
