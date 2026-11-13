//! OPDS 1.2 (Atom) catalogue generation for a list of library books.
//!
//! Routes (relative to [`OpdsConfig::prefix`], default `/opds`):
//!
//! | Path | Feed |
//! |---|---|
//! | `/` | root navigation feed with search links |
//! | `/new`, `/all` | acquisition feed, newest first / by title |
//! | `/authors`, `/series`, `/tags` | navigation feeds (paginated) |
//! | `/authors/NAME`, `/series/NAME`, `/tags/NAME` | acquisition feeds (paginated) |
//! | `/search?q=...` | acquisition feed from a [`Query`] (`author:le guin tag:fantasy`) |
//! | `/opensearch.xml` | OpenSearch description |
//!
//! Links to files point at `/download/ID.epub`, `/cover/ID` and `/cover/ID/thumbnail` (served by
//! [`crate::server`]).

use std::collections::{BTreeMap, HashMap};

use bookx_core::library::Book;
use bookx_core::query::Query;
use chrono::{DateTime, NaiveDate, Utc};
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use uuid::Uuid;

use crate::http::mime_for_extension;
use crate::xml::{escape, strip_tags};

pub const NAV_TYPE: &str = "application/atom+xml;profile=opds-catalog;kind=navigation";
pub const ACQ_TYPE: &str = "application/atom+xml;profile=opds-catalog;kind=acquisition";
pub const OPENSEARCH_TYPE: &str = "application/opensearchdescription+xml";
pub const EPUB_MIME: &str = "application/epub+zip";

const REL_ACQ: &str = "http://opds-spec.org/acquisition";
const REL_IMAGE: &str = "http://opds-spec.org/image";
const REL_THUMB: &str = "http://opds-spec.org/image/thumbnail";

/// Fixed namespace so generated ids are stable across runs.
pub const DEFAULT_INSTANCE: Uuid = Uuid::from_u128(0x7b0f_6a57_9c3d_4e0b_8a52_626f_6f6b_7821);

const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

#[derive(Debug, Clone)]
pub struct OpdsConfig {
    pub title: String,
    /// URL prefix, no trailing slash.
    pub prefix: String,
    pub page_size: usize,
    /// Namespace for the stable `urn:uuid:` ids (one per library/server).
    pub instance_id: Uuid,
    /// Used as `<updated>` of feeds without books.
    pub now: DateTime<Utc>,
}

impl Default for OpdsConfig {
    fn default() -> Self {
        OpdsConfig {
            title: "Bookx Library".into(),
            prefix: "/opds".into(),
            page_size: 50,
            instance_id: DEFAULT_INSTANCE,
            now: Utc::now(),
        }
    }
}

/// A rendered document and its MIME type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpdsDoc {
    pub content_type: &'static str,
    pub body: String,
}

pub fn download_href(id: u64) -> String {
    format!("/download/{id}.epub")
}

pub fn cover_href(id: u64) -> String {
    format!("/cover/{id}")
}

pub fn thumbnail_href(id: u64) -> String {
    format!("/cover/{id}/thumbnail")
}

/// Percent-encodes one path segment.
pub fn encode_segment(s: &str) -> String {
    utf8_percent_encode(s, SEGMENT).to_string()
}

/// `2020` / `2020-05` / `2020-05-17` / RFC 3339 → RFC 3339 UTC, or `None` when unparseable.
pub fn published_to_rfc3339(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(
            dt.with_timezone(&Utc)
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string(),
        );
    }
    let digits = |p: &str, n: usize| p.len() == n && p.bytes().all(|b| b.is_ascii_digit());
    let parts: Vec<&str> = s.split('-').collect();
    let (y, m, d) = match parts.as_slice() {
        [y] if digits(y, 4) => (y.parse().ok()?, 1, 1),
        [y, m] if digits(y, 4) && digits(m, 2) => (y.parse().ok()?, m.parse().ok()?, 1),
        [y, m, d] if digits(y, 4) && digits(m, 2) && digits(d, 2) => {
            (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?)
        }
        _ => return None,
    };
    let date = NaiveDate::from_ymd_opt(y, m, d)?;
    Some(format!("{}T00:00:00Z", date.format("%Y-%m-%d")))
}

fn ts(dt: &DateTime<Utc>) -> String {
    dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn stable_id(cfg: &OpdsConfig, key: &str) -> String {
    format!(
        "urn:uuid:{}",
        Uuid::new_v5(&cfg.instance_id, key.as_bytes())
    )
}

/// Stable `urn:uuid:` id of a book entry.
pub fn book_id(cfg: &OpdsConfig, book: &Book) -> String {
    stable_id(cfg, &format!("book:{}", book.id))
}

struct Link<'a> {
    rel: &'a str,
    href: String,
    mime: &'a str,
    title: Option<&'a str>,
}

fn link_xml(l: &Link) -> String {
    let mut s = format!(
        "<link rel=\"{}\" href=\"{}\" type=\"{}\"",
        escape(l.rel),
        escape(&l.href),
        escape(l.mime)
    );
    if let Some(t) = l.title {
        s.push_str(&format!(" title=\"{}\"", escape(t)));
    }
    s.push_str("/>");
    s
}

struct Page {
    start: usize,
    end: usize,
    page: usize,
    last: usize,
    total: usize,
}

fn paginate(total: usize, page: usize, size: usize) -> Option<Page> {
    let size = size.max(1);
    let last = total.div_ceil(size).max(1);
    if page < 1 || page > last {
        return None;
    }
    let start = (page - 1) * size;
    Some(Page {
        start,
        end: (start + size).min(total),
        page,
        last,
        total,
    })
}

fn href_with(path: &str, params: &[(&str, String)]) -> String {
    if params.is_empty() {
        return path.to_string();
    }
    let q: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{k}={}", utf8_percent_encode(v, SEGMENT)))
        .collect();
    format!("{path}?{}", q.join("&"))
}

struct FeedCtx<'a> {
    cfg: &'a OpdsConfig,
    /// href of this feed without the page parameter
    path: String,
    /// extra query parameters (such as `q`) kept on pagination links
    params: Vec<(&'static str, String)>,
}

impl FeedCtx<'_> {
    fn page_href(&self, page: usize) -> String {
        let mut p = self.params.clone();
        if page > 1 {
            p.push(("page", page.to_string()));
        }
        href_with(&self.path, &p)
    }
}

fn feed_start(
    cx: &FeedCtx,
    title: &str,
    updated: &DateTime<Utc>,
    kind: &str,
    page: Option<&Page>,
) -> String {
    let cfg = cx.cfg;
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str(concat!(
        "<feed xmlns=\"http://www.w3.org/2005/Atom\" ",
        "xmlns:dc=\"http://purl.org/dc/terms/\" ",
        "xmlns:opds=\"http://opds-spec.org/2010/catalog\" ",
        "xmlns:opensearch=\"http://a9.com/-/spec/opensearch/1.1/\" ",
        "xmlns:calibre=\"http://calibre.kovidgoyal.net/2009/metadata\">\n"
    ));
    let feed_key = href_with(&cx.path, &cx.params);
    s.push_str(&format!(
        "  <id>{}</id>\n",
        stable_id(cfg, &format!("feed:{feed_key}"))
    ));
    s.push_str(&format!("  <title>{}</title>\n", escape(title)));
    s.push_str(&format!("  <updated>{}</updated>\n", ts(updated)));
    s.push_str(&format!(
        "  <author><name>{}</name></author>\n",
        escape(&cfg.title)
    ));
    let me = page
        .map(|p| cx.page_href(p.page))
        .unwrap_or_else(|| href_with(&cx.path, &cx.params));
    let mut links = vec![
        Link {
            rel: "self",
            href: me,
            mime: kind,
            title: None,
        },
        Link {
            rel: "start",
            href: root_href(cfg),
            mime: NAV_TYPE,
            title: Some("Home"),
        },
        Link {
            rel: "search",
            href: format!("{}/opensearch.xml", cfg.prefix),
            mime: OPENSEARCH_TYPE,
            title: Some("Search"),
        },
        Link {
            rel: "search",
            href: format!("{}/search?q={{searchTerms}}", cfg.prefix),
            mime: ACQ_TYPE,
            title: Some("Search"),
        },
    ];
    if let Some(p) = page {
        if p.page > 1 {
            links.push(Link {
                rel: "first",
                href: cx.page_href(1),
                mime: kind,
                title: None,
            });
            links.push(Link {
                rel: "previous",
                href: cx.page_href(p.page - 1),
                mime: kind,
                title: None,
            });
        }
        if p.page < p.last {
            links.push(Link {
                rel: "next",
                href: cx.page_href(p.page + 1),
                mime: kind,
                title: None,
            });
            links.push(Link {
                rel: "last",
                href: cx.page_href(p.last),
                mime: kind,
                title: None,
            });
        }
    }
    for l in &links {
        s.push_str("  ");
        s.push_str(&link_xml(l));
        s.push('\n');
    }
    if let Some(p) = page {
        s.push_str(&format!(
            "  <opensearch:totalResults>{}</opensearch:totalResults>\n  <opensearch:itemsPerPage>{}</opensearch:itemsPerPage>\n  <opensearch:startIndex>{}</opensearch:startIndex>\n",
            p.total,
            cx.cfg.page_size.max(1),
            p.start + 1
        ));
    }
    s
}

fn root_href(cfg: &OpdsConfig) -> String {
    if cfg.prefix.is_empty() {
        "/".to_string()
    } else {
        cfg.prefix.clone()
    }
}

/// One acquisition entry.
pub fn book_entry(cfg: &OpdsConfig, b: &Book) -> String {
    let m = &b.meta;
    let mut s = String::new();
    s.push_str("  <entry>\n");
    let title = if m.title.trim().is_empty() {
        "Untitled"
    } else {
        &m.title
    };
    s.push_str(&format!("    <title>{}</title>\n", escape(title)));
    s.push_str(&format!("    <id>{}</id>\n", book_id(cfg, b)));
    s.push_str(&format!("    <updated>{}</updated>\n", ts(&b.modified)));
    if m.authors.is_empty() {
        s.push_str("    <author><name>Unknown</name></author>\n");
    }
    for a in &m.authors {
        s.push_str(&format!(
            "    <author><name>{}</name></author>\n",
            escape(a)
        ));
    }
    if !m.language.is_empty() {
        s.push_str(&format!(
            "    <dc:language>{}</dc:language>\n",
            escape(&m.language)
        ));
    }
    if !m.publisher.is_empty() {
        s.push_str(&format!(
            "    <dc:publisher>{}</dc:publisher>\n",
            escape(&m.publisher)
        ));
    }
    if let Some(p) = published_to_rfc3339(&m.published) {
        s.push_str(&format!("    <published>{p}</published>\n"));
        s.push_str(&format!("    <dc:issued>{p}</dc:issued>\n"));
    }
    for i in &m.identifiers {
        let v = if i.scheme.eq_ignore_ascii_case("isbn") {
            format!("urn:isbn:{}", i.value)
        } else if i.scheme.is_empty() {
            i.value.clone()
        } else {
            format!("{}:{}", i.scheme, i.value)
        };
        s.push_str(&format!(
            "    <dc:identifier>{}</dc:identifier>\n",
            escape(&v)
        ));
    }
    let summary = strip_tags(&m.description);
    if !summary.is_empty() {
        s.push_str(&format!(
            "    <summary type=\"text\">{}</summary>\n",
            escape(&summary)
        ));
    }
    for t in &m.tags {
        s.push_str(&format!(
            "    <category term=\"{0}\" label=\"{0}\"/>\n",
            escape(t)
        ));
    }
    if !m.series.is_empty() {
        s.push_str(&format!(
            "    <calibre:series>{}</calibre:series>\n",
            escape(&m.series)
        ));
        s.push_str(&format!(
            "    <calibre:series_index>{}</calibre:series_index>\n",
            m.series_index
        ));
    }
    if let Some(c) = &b.cover {
        let ext = c.rsplit_once('.').map(|(_, e)| e).unwrap_or("jpg");
        let mime = mime_for_extension(ext);
        let mime = if mime.starts_with("image/") {
            mime
        } else {
            "image/jpeg"
        };
        for (rel, href) in [
            (REL_IMAGE, cover_href(b.id)),
            (REL_THUMB, thumbnail_href(b.id)),
        ] {
            s.push_str("    ");
            s.push_str(&link_xml(&Link {
                rel,
                href,
                mime,
                title: None,
            }));
            s.push('\n');
        }
    }
    s.push_str(&format!(
        "    <link rel=\"{REL_ACQ}\" href=\"{}\" type=\"{EPUB_MIME}\" title=\"EPUB\" length=\"{}\"/>\n",
        escape(&download_href(b.id)),
        b.size
    ));
    s.push_str("  </entry>\n");
    s
}

fn updated_of(cfg: &OpdsConfig, books: &[&Book]) -> DateTime<Utc> {
    books.iter().map(|b| b.modified).max().unwrap_or(cfg.now)
}

/// Acquisition feed of `books` (already filtered and sorted), one page of it.
fn acquisition_feed(cx: &FeedCtx, title: &str, books: &[&Book], page: usize) -> Option<OpdsDoc> {
    let pg = paginate(books.len(), page, cx.cfg.page_size)?;
    let mut s = feed_start(cx, title, &updated_of(cx.cfg, books), ACQ_TYPE, Some(&pg));
    for b in &books[pg.start..pg.end] {
        s.push_str(&book_entry(cx.cfg, b));
    }
    s.push_str("</feed>\n");
    Some(OpdsDoc {
        content_type: ACQ_TYPE,
        body: s,
    })
}

struct NavItem {
    title: String,
    href: String,
    content: String,
    kind: &'static str,
}

fn nav_entry(cfg: &OpdsConfig, it: &NavItem, updated: &DateTime<Utc>) -> String {
    format!(
        "  <entry>\n    <title>{}</title>\n    <id>{}</id>\n    <updated>{}</updated>\n    <content type=\"text\">{}</content>\n    <link rel=\"subsection\" href=\"{}\" type=\"{}\"/>\n  </entry>\n",
        escape(&it.title),
        stable_id(cfg, &format!("nav:{}", it.href)),
        ts(updated),
        escape(&it.content),
        escape(&it.href),
        it.kind
    )
}

fn nav_feed(
    cx: &FeedCtx,
    title: &str,
    updated: &DateTime<Utc>,
    items: &[NavItem],
    page: usize,
) -> Option<OpdsDoc> {
    let pg = paginate(items.len(), page, cx.cfg.page_size)?;
    let mut s = feed_start(cx, title, updated, NAV_TYPE, Some(&pg));
    for it in &items[pg.start..pg.end] {
        s.push_str(&nav_entry(cx.cfg, it, updated));
    }
    s.push_str("</feed>\n");
    Some(OpdsDoc {
        content_type: NAV_TYPE,
        body: s,
    })
}

fn count_label(n: usize) -> String {
    if n == 1 {
        "1 book".into()
    } else {
        format!("{n} books")
    }
}

fn root_feed(cfg: &OpdsConfig, books: &[Book]) -> OpdsDoc {
    let refs: Vec<&Book> = books.iter().collect();
    let updated = updated_of(cfg, &refs);
    let p = &cfg.prefix;
    let items = vec![
        NavItem {
            title: "Newest".into(),
            href: format!("{p}/new"),
            content: format!("Recently added books ({})", count_label(books.len())),
            kind: ACQ_TYPE,
        },
        NavItem {
            title: "All books".into(),
            href: format!("{p}/all"),
            content: "Sorted by title".into(),
            kind: ACQ_TYPE,
        },
        NavItem {
            title: "By author".into(),
            href: format!("{p}/authors"),
            content: "Browse by author".into(),
            kind: NAV_TYPE,
        },
        NavItem {
            title: "By series".into(),
            href: format!("{p}/series"),
            content: "Browse by series".into(),
            kind: NAV_TYPE,
        },
        NavItem {
            title: "By tag".into(),
            href: format!("{p}/tags"),
            content: "Browse by tag".into(),
            kind: NAV_TYPE,
        },
    ];
    let cx = FeedCtx {
        cfg,
        path: root_href(cfg),
        params: vec![],
    };
    let mut s = feed_start(&cx, &cfg.title, &updated, NAV_TYPE, None);
    for it in &items {
        s.push_str(&nav_entry(cfg, it, &updated));
    }
    s.push_str("</feed>\n");
    OpdsDoc {
        content_type: NAV_TYPE,
        body: s,
    }
}

/// The OpenSearch description document.
pub fn opensearch_description(cfg: &OpdsConfig) -> OpdsDoc {
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<OpenSearchDescription xmlns=\"http://a9.com/-/spec/opensearch/1.1/\">\n  <ShortName>{}</ShortName>\n  <Description>Search the library</Description>\n  <InputEncoding>UTF-8</InputEncoding>\n  <OutputEncoding>UTF-8</OutputEncoding>\n  <Url type=\"{}\" template=\"{}\"/>\n</OpenSearchDescription>\n",
        escape(&cfg.title),
        escape(ACQ_TYPE),
        escape(&format!("{}/search?q={{searchTerms}}", cfg.prefix))
    );
    OpdsDoc {
        content_type: OPENSEARCH_TYPE,
        body,
    }
}

fn by_title(a: &&Book, b: &&Book) -> std::cmp::Ordering {
    a.meta
        .title
        .to_lowercase()
        .cmp(&b.meta.title.to_lowercase())
        .then(a.id.cmp(&b.id))
}

fn group_counts<'a>(
    books: &'a [Book],
    keys: impl Fn(&'a Book) -> Vec<&'a str>,
) -> Vec<(String, usize)> {
    let mut map: BTreeMap<(String, String), usize> = BTreeMap::new();
    for b in books {
        for k in keys(b) {
            if k.trim().is_empty() {
                continue;
            }
            *map.entry((k.to_lowercase(), k.to_string())).or_default() += 1;
        }
    }
    map.into_iter().map(|((_, name), n)| (name, n)).collect()
}

/// Renders the document for `segments` (the path below the prefix), or `None` for 404.
pub fn route(
    cfg: &OpdsConfig,
    books: &[Book],
    segments: &[String],
    query: &HashMap<String, String>,
) -> Option<OpdsDoc> {
    let page = match query.get("page") {
        None => 1,
        Some(p) => p.parse::<usize>().ok().filter(|p| *p >= 1)?,
    };
    let p = &cfg.prefix;
    let segs: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();
    let updated = books.iter().map(|b| b.modified).max().unwrap_or(cfg.now);
    match segs.as_slice() {
        [] => (page == 1).then(|| root_feed(cfg, books)),
        ["opensearch.xml"] => Some(opensearch_description(cfg)),
        ["new"] => {
            let mut v: Vec<&Book> = books.iter().collect();
            v.sort_by(|a, b| b.added.cmp(&a.added).then(b.id.cmp(&a.id)));
            let cx = FeedCtx {
                cfg,
                path: format!("{p}/new"),
                params: vec![],
            };
            acquisition_feed(&cx, "Newest books", &v, page)
        }
        ["all"] => {
            let mut v: Vec<&Book> = books.iter().collect();
            v.sort_by(by_title);
            let cx = FeedCtx {
                cfg,
                path: format!("{p}/all"),
                params: vec![],
            };
            acquisition_feed(&cx, "All books", &v, page)
        }
        ["search"] => {
            let q = query.get("q").map(|s| s.as_str()).unwrap_or("");
            let parsed = Query::parse(q);
            let mut v: Vec<&Book> = books.iter().filter(|b| parsed.matches(b)).collect();
            v.sort_by(by_title);
            let cx = FeedCtx {
                cfg,
                path: format!("{p}/search"),
                params: vec![("q", q.to_string())],
            };
            acquisition_feed(&cx, &format!("Search: {q}"), &v, page)
        }
        [kind @ ("authors" | "series" | "tags")] => {
            let counts = match *kind {
                "authors" => group_counts(books, |b| {
                    b.meta.authors.iter().map(|s| s.as_str()).collect()
                }),
                "series" => group_counts(books, |b| vec![b.meta.series.as_str()]),
                _ => group_counts(books, |b| b.meta.tags.iter().map(|s| s.as_str()).collect()),
            };
            let items: Vec<NavItem> = counts
                .into_iter()
                .map(|(name, n)| NavItem {
                    href: format!("{p}/{kind}/{}", encode_segment(&name)),
                    title: name,
                    content: count_label(n),
                    kind: ACQ_TYPE,
                })
                .collect();
            let title = match *kind {
                "authors" => "Authors",
                "series" => "Series",
                _ => "Tags",
            };
            let cx = FeedCtx {
                cfg,
                path: format!("{p}/{kind}"),
                params: vec![],
            };
            nav_feed(&cx, title, &updated, &items, page)
        }
        [kind @ ("authors" | "series" | "tags"), name] => {
            let mut v: Vec<&Book> = books
                .iter()
                .filter(|b| match *kind {
                    "authors" => b.meta.authors.iter().any(|a| a == name),
                    "series" => b.meta.series == *name,
                    _ => b.meta.tags.iter().any(|t| t == name),
                })
                .collect();
            if v.is_empty() {
                return None;
            }
            if *kind == "series" {
                v.sort_by(|a, b| {
                    a.meta
                        .series_index
                        .partial_cmp(&b.meta.series_index)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| by_title(a, b))
                });
            } else {
                v.sort_by(by_title);
            }
            let cx = FeedCtx {
                cfg,
                path: format!("{p}/{kind}/{}", encode_segment(name)),
                params: vec![],
            };
            acquisition_feed(&cx, name, &v, page)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_core::opf::{Identifier, Metadata};
    use chrono::TimeZone;

    fn book(id: u64, title: &str, authors: &[&str]) -> Book {
        let t =
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap() + chrono::Duration::days(id as i64);
        Book {
            id,
            file: format!("a/{id}.epub"),
            cover: None,
            meta: Metadata {
                title: title.into(),
                authors: authors.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            },
            added: t,
            modified: t,
            last_read: None,
            size: 1000 + id,
            progress: 0.0,
            position: None,
            bookmarks: vec![],
            highlights: vec![],
            formats: vec![],
            resources: vec![],
            drm: String::new(),
        }
    }

    fn cfg() -> OpdsConfig {
        OpdsConfig {
            now: Utc.with_ymd_and_hms(2025, 5, 5, 0, 0, 0).unwrap(),
            page_size: 3,
            ..Default::default()
        }
    }

    fn segs(p: &str) -> Vec<String> {
        crate::http::split_path(p).unwrap()
    }

    fn get(books: &[Book], path: &str, q: &[(&str, &str)]) -> Option<OpdsDoc> {
        let query = q
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        route(&cfg(), books, &segs(path), &query)
    }

    const ATOM: &str = "http://www.w3.org/2005/Atom";

    fn parse(doc: &OpdsDoc) -> roxmltree::Document<'_> {
        roxmltree::Document::parse(&doc.body).unwrap_or_else(|e| panic!("{e}\n{}", doc.body))
    }

    fn entries<'a>(d: &'a roxmltree::Document) -> Vec<roxmltree::Node<'a, 'a>> {
        d.root_element()
            .children()
            .filter(|n| n.has_tag_name((ATOM, "entry")))
            .collect()
    }

    fn child_text(n: roxmltree::Node, name: &str) -> String {
        n.children()
            .find(|c| c.has_tag_name((ATOM, name)))
            .and_then(|c| c.text())
            .unwrap_or("")
            .to_string()
    }

    fn feed_links(d: &roxmltree::Document) -> Vec<(String, String, String)> {
        d.root_element()
            .children()
            .filter(|n| n.has_tag_name((ATOM, "link")))
            .map(|n| {
                (
                    n.attribute("rel").unwrap_or("").to_string(),
                    n.attribute("href").unwrap_or("").to_string(),
                    n.attribute("type").unwrap_or("").to_string(),
                )
            })
            .collect()
    }

    fn rich_book() -> Book {
        let mut b = book(
            7,
            "Tom & Jerry's <Big> \"Day\"",
            &["Ann & Bob", "Üñí Çødé 日本"],
        );
        b.meta.language = "en".into();
        b.meta.publisher = "Pub <lisher>".into();
        b.meta.published = "2020-05-17".into();
        b.meta.description = "<p>Hello <b>world</b> &amp; more</p>".into();
        b.meta.tags = vec!["sci-fi".into(), "a&b".into()];
        b.meta.series = "Saga \"One\"".into();
        b.meta.series_index = 2.5;
        b.meta.identifiers = vec![
            Identifier {
                scheme: "isbn".into(),
                value: "9781234567890".into(),
                xml_id: None,
            },
            Identifier {
                scheme: "doi".into(),
                value: "10.1/x".into(),
                xml_id: None,
            },
        ];
        b.cover = Some("a/7.png".into());
        b
    }

    #[test]
    fn root_is_valid_navigation_feed_with_search() {
        let books = vec![book(1, "A", &["X"])];
        let doc = get(&books, "/", &[]).unwrap();
        assert_eq!(doc.content_type, NAV_TYPE);
        let d = parse(&doc);
        let root = d.root_element();
        assert!(root.has_tag_name((ATOM, "feed")));
        let es = entries(&d);
        assert_eq!(es.len(), 5);
        let titles: Vec<_> = es.iter().map(|e| child_text(*e, "title")).collect();
        assert_eq!(
            titles,
            ["Newest", "All books", "By author", "By series", "By tag"]
        );
        let links = feed_links(&d);
        assert!(links.contains(&("self".into(), "/opds".into(), NAV_TYPE.into())));
        assert!(links.contains(&("start".into(), "/opds".into(), NAV_TYPE.into())));
        assert!(links.contains(&(
            "search".into(),
            "/opds/opensearch.xml".into(),
            OPENSEARCH_TYPE.into()
        )));
        assert!(links.contains(&(
            "search".into(),
            "/opds/search?q={searchTerms}".into(),
            ACQ_TYPE.into()
        )));
        // every nav entry links to a subsection
        for e in es {
            let l = e
                .children()
                .find(|c| c.has_tag_name((ATOM, "link")))
                .unwrap();
            assert_eq!(l.attribute("rel"), Some("subsection"));
            assert!(l.attribute("href").unwrap().starts_with("/opds/"));
        }
        // required Atom feed children
        for tag in ["id", "title", "updated"] {
            assert!(
                root.children().any(|c| c.has_tag_name((ATOM, tag))),
                "{tag}"
            );
        }
    }

    #[test]
    fn root_works_with_empty_library_and_uses_now() {
        let doc = get(&[], "/", &[]).unwrap();
        let d = parse(&doc);
        assert_eq!(
            child_text(d.root_element(), "updated"),
            "2025-05-05T00:00:00Z"
        );
    }

    #[test]
    fn root_page_two_is_404() {
        assert!(get(&[], "/", &[("page", "2")]).is_none());
    }

    #[test]
    fn book_entry_has_all_metadata() {
        let b = rich_book();
        let doc = get(std::slice::from_ref(&b), "/all", &[]).unwrap();
        assert_eq!(doc.content_type, ACQ_TYPE);
        let d = parse(&doc);
        let es = entries(&d);
        assert_eq!(es.len(), 1);
        let e = es[0];
        assert_eq!(child_text(e, "title"), "Tom & Jerry's <Big> \"Day\"");
        let authors: Vec<String> = e
            .children()
            .filter(|c| c.has_tag_name((ATOM, "author")))
            .map(|a| child_text(a, "name"))
            .collect();
        assert_eq!(authors, ["Ann & Bob", "Üñí Çødé 日本"]);
        assert_eq!(child_text(e, "summary"), "Hello world & more");
        assert_eq!(child_text(e, "published"), "2020-05-17T00:00:00Z");
        assert_eq!(child_text(e, "updated"), "2024-01-08T00:00:00Z");
        let dc = "http://purl.org/dc/terms/";
        let txt = |ns: &str, name: &str| -> String {
            e.children()
                .find(|c| c.has_tag_name((ns, name)))
                .and_then(|c| c.text())
                .unwrap_or("")
                .to_string()
        };
        assert_eq!(txt(dc, "language"), "en");
        assert_eq!(txt(dc, "publisher"), "Pub <lisher>");
        assert_eq!(txt(dc, "issued"), "2020-05-17T00:00:00Z");
        let cal = "http://calibre.kovidgoyal.net/2009/metadata";
        assert_eq!(txt(cal, "series"), "Saga \"One\"");
        assert_eq!(txt(cal, "series_index"), "2.5");
        let ids: Vec<String> = e
            .children()
            .filter(|c| c.has_tag_name((dc, "identifier")))
            .map(|c| c.text().unwrap().to_string())
            .collect();
        assert_eq!(ids, ["urn:isbn:9781234567890", "doi:10.1/x"]);
        let cats: Vec<_> = e
            .children()
            .filter(|c| c.has_tag_name((ATOM, "category")))
            .map(|c| (c.attribute("term").unwrap(), c.attribute("label").unwrap()))
            .collect();
        assert_eq!(cats, [("sci-fi", "sci-fi"), ("a&b", "a&b")]);
        let links: Vec<_> = e
            .children()
            .filter(|c| c.has_tag_name((ATOM, "link")))
            .map(|c| {
                (
                    c.attribute("rel").unwrap().to_string(),
                    c.attribute("href").unwrap().to_string(),
                    c.attribute("type").unwrap().to_string(),
                    c.attribute("length").map(|s| s.to_string()),
                )
            })
            .collect();
        assert!(links.contains(&(
            "http://opds-spec.org/image".into(),
            "/cover/7".into(),
            "image/png".into(),
            None
        )));
        assert!(links.contains(&(
            "http://opds-spec.org/image/thumbnail".into(),
            "/cover/7/thumbnail".into(),
            "image/png".into(),
            None
        )));
        assert!(links.contains(&(
            "http://opds-spec.org/acquisition".into(),
            "/download/7.epub".into(),
            "application/epub+zip".into(),
            Some("1007".into())
        )));
    }

    #[test]
    fn entry_without_optional_fields_is_minimal_but_valid() {
        let mut b = book(1, "  ", &[]);
        b.meta.series_index = 0.0;
        let doc = get(&[b], "/new", &[]).unwrap();
        let d = parse(&doc);
        let e = entries(&d)[0];
        assert_eq!(child_text(e, "title"), "Untitled");
        let a = e
            .children()
            .find(|c| c.has_tag_name((ATOM, "author")))
            .unwrap();
        assert_eq!(child_text(a, "name"), "Unknown");
        assert!(!doc.body.contains("<summary"));
        assert!(!doc.body.contains("<published"));
        assert!(!doc.body.contains("calibre:series"));
        assert!(!doc.body.contains("opds-spec.org/image"));
        assert!(!doc.body.contains("dc:language"));
    }

    #[test]
    fn xml_special_and_control_characters_never_break_the_document() {
        let mut b = book(1, "x\u{0}y\u{1}z & \u{FFFE}", &["<a>"]);
        b.meta.description = "bad \u{8} char <script>alert('x')</script>".into();
        b.meta.tags = vec!["\"quoted\"".into(), "it's".into()];
        let doc = get(&[b], "/all", &[]).unwrap();
        let d = parse(&doc);
        let e = entries(&d)[0];
        assert_eq!(child_text(e, "title"), "xyz & ");
        assert!(!doc.body.contains("<script"));
    }

    #[test]
    fn ids_are_stable_unique_and_valid_uuids() {
        let a = book(1, "A", &["x"]);
        let b = book(2, "B", &["x"]);
        let c = cfg();
        assert_eq!(book_id(&c, &a), book_id(&c, &a.clone()));
        assert_ne!(book_id(&c, &a), book_id(&c, &b));
        let id = book_id(&c, &a);
        let u = id.strip_prefix("urn:uuid:").unwrap();
        let parsed = Uuid::parse_str(u).unwrap();
        assert_eq!(parsed.get_version_num(), 5);
        // different instance, different ids
        let other = OpdsConfig {
            instance_id: Uuid::from_u128(1),
            ..cfg()
        };
        assert_ne!(book_id(&other, &a), id);
        // same output twice
        let books = vec![a, b];
        assert_eq!(get(&books, "/all", &[]), get(&books, "/all", &[]));
    }

    #[test]
    fn feed_ids_differ_between_feeds() {
        let books = vec![book(1, "A", &["x"])];
        let id = |p: &str| {
            let doc = get(&books, p, &[]).unwrap();
            let d = parse(&doc);
            child_text(d.root_element(), "id")
        };
        assert_ne!(id("/new"), id("/all"));
        assert_ne!(id("/"), id("/new"));
        assert!(id("/").starts_with("urn:uuid:"));
    }

    #[test]
    fn new_is_newest_first_and_all_is_by_title() {
        let books = vec![
            book(1, "Zeta", &["x"]),
            book(2, "alpha", &["x"]),
            book(3, "Mid", &["x"]),
        ];
        let titles = |p: &str| {
            let doc = get(&books, p, &[]).unwrap();
            let d = parse(&doc);
            entries(&d)
                .iter()
                .map(|e| child_text(*e, "title"))
                .collect::<Vec<_>>()
        };
        assert_eq!(titles("/new"), ["Mid", "alpha", "Zeta"]);
        assert_eq!(titles("/all"), ["alpha", "Mid", "Zeta"]);
    }

    #[test]
    fn pagination_has_next_previous_first_last() {
        let books: Vec<Book> = (1..=8).map(|i| book(i, &format!("B{i}"), &["x"])).collect();
        let page = |n: &str| {
            let doc = get(&books, "/all", &[("page", n)]).unwrap();
            let d = parse(&doc);
            (
                entries(&d).len(),
                feed_links(&d),
                d.root_element()
                    .descendants()
                    .find(|x| x.tag_name().name() == "totalResults")
                    .and_then(|x| x.text().map(String::from)),
            )
        };
        let (n, links, total) = page("1");
        assert_eq!(n, 3);
        assert_eq!(total.as_deref(), Some("8"));
        let rels: Vec<&str> = links.iter().map(|l| l.0.as_str()).collect();
        assert!(rels.contains(&"next"));
        assert!(!rels.contains(&"previous"));
        assert!(links.contains(&("next".into(), "/opds/all?page=2".into(), ACQ_TYPE.into())));
        assert!(links.contains(&("last".into(), "/opds/all?page=3".into(), ACQ_TYPE.into())));

        let (n, links, _) = page("2");
        assert_eq!(n, 3);
        assert!(links.contains(&("previous".into(), "/opds/all".into(), ACQ_TYPE.into())));
        assert!(links.contains(&("first".into(), "/opds/all".into(), ACQ_TYPE.into())));
        assert!(links.contains(&("self".into(), "/opds/all?page=2".into(), ACQ_TYPE.into())));
        assert!(links.contains(&("next".into(), "/opds/all?page=3".into(), ACQ_TYPE.into())));

        let (n, links, _) = page("3");
        assert_eq!(n, 2);
        assert!(!links.iter().any(|l| l.0 == "next"));
        assert!(links.iter().any(|l| l.0 == "previous"));
    }

    #[test]
    fn pages_cover_every_book_exactly_once() {
        let books: Vec<Book> = (1..=8).map(|i| book(i, &format!("B{i}"), &["x"])).collect();
        let mut seen = Vec::new();
        for p in 1..=3 {
            let doc = get(&books, "/all", &[("page", &p.to_string())]).unwrap();
            let d = parse(&doc);
            seen.extend(entries(&d).iter().map(|e| child_text(*e, "title")));
        }
        assert_eq!(seen, ["B1", "B2", "B3", "B4", "B5", "B6", "B7", "B8"]);
    }

    #[test]
    fn invalid_pages_are_404() {
        let books: Vec<Book> = (1..=4).map(|i| book(i, "t", &["x"])).collect();
        assert!(get(&books, "/all", &[("page", "3")]).is_none());
        assert!(get(&books, "/all", &[("page", "0")]).is_none());
        assert!(get(&books, "/all", &[("page", "-1")]).is_none());
        assert!(get(&books, "/all", &[("page", "x")]).is_none());
        assert!(get(&books, "/all", &[("page", "99999999999999999999")]).is_none());
        // an empty library still has page 1
        assert!(get(&[], "/all", &[("page", "1")]).is_some());
        assert!(get(&[], "/all", &[("page", "2")]).is_none());
    }

    #[test]
    fn authors_navigation_and_acquisition() {
        let books = vec![
            book(1, "One", &["Le Guin"]),
            book(2, "Two", &["le guin", "Pratchett"]),
            book(3, "Three", &["Le Guin"]),
            book(4, "Four", &["A/B & C"]),
        ];
        let doc = get(&books, "/authors", &[]).unwrap();
        assert_eq!(doc.content_type, NAV_TYPE);
        let d = parse(&doc);
        let es = entries(&d);
        let names: Vec<_> = es.iter().map(|e| child_text(*e, "title")).collect();
        assert_eq!(names, ["A/B & C", "Le Guin", "le guin"]);
        let first = es[0];
        assert_eq!(child_text(first, "content"), "1 book");
        let link = first
            .children()
            .find(|c| c.has_tag_name((ATOM, "link")))
            .unwrap();
        assert_eq!(
            link.attribute("href"),
            Some("/opds/authors/A%2FB%20%26%20C")
        );
        assert_eq!(link.attribute("type"), Some(ACQ_TYPE));
        assert_eq!(child_text(es[1], "content"), "2 books");

        // following that link works, including the encoded slash (segments arrive decoded)
        let doc = route(
            &cfg(),
            &books,
            &["authors".into(), "A/B & C".into()],
            &HashMap::new(),
        )
        .unwrap();
        let d = parse(&doc);
        assert_eq!(entries(&d).len(), 1);
        assert_eq!(child_text(d.root_element(), "title"), "A/B & C");

        let doc = get(&books, "/authors/Le%20Guin", &[]).unwrap();
        let d = parse(&doc);
        let titles: Vec<_> = entries(&d)
            .iter()
            .map(|e| child_text(*e, "title"))
            .collect();
        assert_eq!(titles, ["One", "Three"]);
        assert!(get(&books, "/authors/Nobody", &[]).is_none());
    }

    #[test]
    fn navigation_feeds_are_paginated_too() {
        let books: Vec<Book> = (1..=7)
            .map(|i| book(i, "T", &[&format!("Author {i}")]))
            .collect();
        let doc = get(&books, "/authors", &[]).unwrap();
        let d = parse(&doc);
        assert_eq!(entries(&d).len(), 3);
        assert!(feed_links(&d).contains(&(
            "next".into(),
            "/opds/authors?page=2".into(),
            NAV_TYPE.into()
        )));
        let doc = get(&books, "/authors", &[("page", "3")]).unwrap();
        let d = parse(&doc);
        let names: Vec<_> = entries(&d)
            .iter()
            .map(|e| child_text(*e, "title"))
            .collect();
        assert_eq!(names, ["Author 7"]);
        assert!(get(&books, "/authors", &[("page", "4")]).is_none());
    }

    #[test]
    fn series_are_ordered_by_index() {
        let mut a = book(1, "Third", &["x"]);
        let mut b = book(2, "First", &["x"]);
        let mut c = book(3, "Second", &["x"]);
        let d_ = book(4, "No series", &["x"]);
        for (bk, i) in [(&mut a, 3.0), (&mut b, 1.0), (&mut c, 2.0)] {
            bk.meta.series = "Saga".into();
            bk.meta.series_index = i;
        }
        let books = vec![a, b, c, d_];
        let doc = get(&books, "/series", &[]).unwrap();
        let d = parse(&doc);
        assert_eq!(entries(&d).len(), 1);
        assert_eq!(child_text(entries(&d)[0], "content"), "3 books");
        let doc = get(&books, "/series/Saga", &[]).unwrap();
        let d = parse(&doc);
        let titles: Vec<_> = entries(&d)
            .iter()
            .map(|e| child_text(*e, "title"))
            .collect();
        assert_eq!(titles, ["First", "Second", "Third"]);
    }

    #[test]
    fn tags_feed() {
        let mut a = book(1, "A", &["x"]);
        a.meta.tags = vec!["fantasy".into(), "epic".into()];
        let mut b = book(2, "B", &["x"]);
        b.meta.tags = vec!["fantasy".into()];
        let books = vec![a, b];
        let doc = get(&books, "/tags", &[]).unwrap();
        let d = parse(&doc);
        let names: Vec<_> = entries(&d)
            .iter()
            .map(|e| child_text(*e, "title"))
            .collect();
        assert_eq!(names, ["epic", "fantasy"]);
        let doc = get(&books, "/tags/fantasy", &[]).unwrap();
        assert_eq!(entries(&parse(&doc)).len(), 2);
    }

    #[test]
    fn empty_groups_give_an_empty_navigation_feed() {
        let doc = get(&[book(1, "A", &["x"])], "/series", &[]).unwrap();
        assert_eq!(entries(&parse(&doc)).len(), 0);
    }

    #[test]
    fn search_uses_the_query_language() {
        let mut a = book(1, "Left Hand of Darkness", &["Ursula Le Guin"]);
        a.meta.tags = vec!["scifi".into()];
        let b = book(2, "Dune", &["Frank Herbert"]);
        let books = vec![a, b];
        let titles = |q: &str| {
            let doc = get(&books, "/search", &[("q", q)]).unwrap();
            let d = parse(&doc);
            entries(&d)
                .iter()
                .map(|e| child_text(*e, "title"))
                .collect::<Vec<_>>()
        };
        assert_eq!(titles("dune"), ["Dune"]);
        assert_eq!(titles("author:\"le guin\""), ["Left Hand of Darkness"]);
        assert_eq!(
            titles("tag:scifi OR dune"),
            ["Dune", "Left Hand of Darkness"]
        );
        assert_eq!(titles("-dune"), ["Left Hand of Darkness"]);
        assert_eq!(titles("nothing-matches-this"), Vec::<String>::new());
        assert_eq!(titles("").len(), 2);
    }

    #[test]
    fn search_pagination_keeps_the_query() {
        let books: Vec<Book> = (1..=7)
            .map(|i| book(i, &format!("Match {i}"), &["x"]))
            .collect();
        let doc = get(&books, "/search", &[("q", "title:match"), ("page", "2")]).unwrap();
        let d = parse(&doc);
        let links = feed_links(&d);
        assert!(links.contains(&(
            "next".into(),
            "/opds/search?q=title%3Amatch&page=3".into(),
            ACQ_TYPE.into()
        )));
        assert!(links.contains(&(
            "previous".into(),
            "/opds/search?q=title%3Amatch".into(),
            ACQ_TYPE.into()
        )));
        assert_eq!(child_text(d.root_element(), "title"), "Search: title:match");
    }

    #[test]
    fn opensearch_description_is_valid_and_has_template() {
        let doc = get(&[], "/opensearch.xml", &[]).unwrap();
        assert_eq!(doc.content_type, OPENSEARCH_TYPE);
        let d = parse(&doc);
        let ns = "http://a9.com/-/spec/opensearch/1.1/";
        let root = d.root_element();
        assert!(root.has_tag_name((ns, "OpenSearchDescription")));
        let url = root
            .children()
            .find(|c| c.has_tag_name((ns, "Url")))
            .unwrap();
        assert_eq!(
            url.attribute("template"),
            Some("/opds/search?q={searchTerms}")
        );
        assert_eq!(url.attribute("type"), Some(ACQ_TYPE));
        let name = root
            .children()
            .find(|c| c.has_tag_name((ns, "ShortName")))
            .unwrap();
        assert_eq!(name.text(), Some("Bookx Library"));
    }

    #[test]
    fn title_with_markup_is_escaped_in_opensearch() {
        let c = OpdsConfig {
            title: "A & <B>".into(),
            ..cfg()
        };
        let doc = opensearch_description(&c);
        let d = parse(&doc);
        assert!(d
            .root_element()
            .descendants()
            .any(|n| n.text() == Some("A & <B>")));
    }

    #[test]
    fn unknown_routes_are_404() {
        let books = vec![book(1, "A", &["x"])];
        for p in [
            "/nope",
            "/new/extra",
            "/authors/a/b",
            "/search/x",
            "/opensearch.xml/x",
        ] {
            assert!(get(&books, p, &[]).is_none(), "{p}");
        }
    }

    #[test]
    fn custom_prefix_is_used_in_links() {
        let c = OpdsConfig {
            prefix: "/lib/opds".into(),
            ..cfg()
        };
        let doc = route(&c, &[], &[], &HashMap::new()).unwrap();
        assert!(doc.body.contains("href=\"/lib/opds/new\""));
        assert!(doc.body.contains("href=\"/lib/opds/opensearch.xml\""));
        let c = OpdsConfig {
            prefix: "".into(),
            ..cfg()
        };
        let doc = route(&c, &[], &[], &HashMap::new()).unwrap();
        assert!(doc.body.contains("rel=\"start\" href=\"/\""));
    }

    #[test]
    fn published_dates() {
        assert_eq!(
            published_to_rfc3339("2020"),
            Some("2020-01-01T00:00:00Z".into())
        );
        assert_eq!(
            published_to_rfc3339("2020-05"),
            Some("2020-05-01T00:00:00Z".into())
        );
        assert_eq!(
            published_to_rfc3339("2020-05-17"),
            Some("2020-05-17T00:00:00Z".into())
        );
        assert_eq!(
            published_to_rfc3339("2020-05-17T10:00:00+02:00"),
            Some("2020-05-17T08:00:00Z".into())
        );
        assert_eq!(
            published_to_rfc3339(" 1999 "),
            Some("1999-01-01T00:00:00Z".into())
        );
        for bad in [
            "",
            "   ",
            "May 2020",
            "2020-13",
            "2020-02-30",
            "20-05-17",
            "2020-5-1",
            "abcd",
        ] {
            assert_eq!(published_to_rfc3339(bad), None, "{bad}");
        }
    }

    #[test]
    fn cover_mime_follows_extension_and_falls_back() {
        for (file, mime) in [
            ("c.jpg", "image/jpeg"),
            ("c.jpeg", "image/jpeg"),
            ("c.gif", "image/gif"),
            ("c.webp", "image/webp"),
            ("c.bin", "image/jpeg"),
            ("c", "image/jpeg"),
        ] {
            let mut b = book(1, "A", &["x"]);
            b.cover = Some(file.into());
            let e = book_entry(&cfg(), &b);
            assert!(e.contains(&format!("type=\"{mime}\"")), "{file}: {e}");
        }
    }

    #[test]
    fn segment_encoding() {
        assert_eq!(encode_segment("a b/c&d"), "a%20b%2Fc%26d");
        assert_eq!(encode_segment("safe-._~"), "safe-._~");
        assert_eq!(encode_segment("日"), "%E6%97%A5");
    }

    #[test]
    fn hrefs_helpers() {
        assert_eq!(download_href(5), "/download/5.epub");
        assert_eq!(cover_href(5), "/cover/5");
        assert_eq!(thumbnail_href(5), "/cover/5/thumbnail");
    }

    #[test]
    fn updated_is_the_newest_modification() {
        let books = vec![
            book(1, "A", &["x"]),
            book(9, "B", &["x"]),
            book(3, "C", &["x"]),
        ];
        let doc = get(&books, "/all", &[]).unwrap();
        let d = parse(&doc);
        assert_eq!(
            child_text(d.root_element(), "updated"),
            "2024-01-10T00:00:00Z"
        );
    }

    #[test]
    fn large_library_page_is_bounded() {
        let books: Vec<Book> = (1..=2000)
            .map(|i| book(i, &format!("Book {i}"), &["x"]))
            .collect();
        let c = OpdsConfig {
            page_size: 50,
            ..cfg()
        };
        let doc = route(&c, &books, &segs("/all"), &HashMap::new()).unwrap();
        let d = parse(&doc);
        assert_eq!(entries(&d).len(), 50);
    }
}
