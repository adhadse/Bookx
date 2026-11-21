//! OPDS client side: parses Atom catalogue feeds (navigation and acquisition) into plain structs.
//!
//! No networking here: callers fetch the XML and pass it with the URL it came from, so relative
//! links can be resolved. Works with Project Gutenberg, Standard Ebooks, Calibre and Bookx feeds.

use anyhow::{bail, Context, Result};
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use roxmltree::{Document, Node};
use url::Url;

use crate::xml::strip_tags;

const ATOM_NS: &str = "http://www.w3.org/2005/Atom";
const OPENSEARCH_NS: &str = "http://a9.com/-/spec/opensearch/1.1/";
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
const REL_ACQ_PREFIX: &str = "http://opds-spec.org/acquisition";

const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Link {
    pub rel: String,
    /// Absolute when the base URL was absolute.
    pub href: String,
    pub mime: String,
    pub title: String,
    pub length: Option<u64>,
}

impl Link {
    pub fn is_acquisition(&self) -> bool {
        self.rel.starts_with(REL_ACQ_PREFIX)
    }

    fn is_catalog(&self) -> bool {
        let m = self.mime.to_ascii_lowercase();
        m.contains("application/atom+xml") || m.contains("opds-catalog")
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub updated: String,
    pub authors: Vec<String>,
    /// `<summary>`, or the text of `<content>` when there is no summary.
    pub summary: String,
    pub categories: Vec<String>,
    pub language: String,
    pub publisher: String,
    pub published: String,
    pub series: Option<String>,
    pub series_index: Option<f64>,
    pub links: Vec<Link>,
}

impl Entry {
    pub fn acquisition_links(&self) -> Vec<&Link> {
        self.links.iter().filter(|l| l.is_acquisition()).collect()
    }

    pub fn is_acquisition(&self) -> bool {
        self.links.iter().any(|l| l.is_acquisition())
    }

    pub fn cover(&self) -> Option<&Link> {
        self.links
            .iter()
            .find(|l| l.rel == "http://opds-spec.org/image" || l.rel == "x-stanza-cover-image")
    }

    pub fn thumbnail(&self) -> Option<&Link> {
        self.links.iter().find(|l| {
            l.rel == "http://opds-spec.org/image/thumbnail"
                || l.rel == "x-stanza-cover-image-thumbnail"
        })
    }

    /// The link to follow for a navigation entry (a sub-catalogue).
    pub fn navigation_link(&self) -> Option<&Link> {
        if self.is_acquisition() {
            return None;
        }
        self.links
            .iter()
            .find(|l| {
                l.rel == "subsection"
                    || l.rel == "http://opds-spec.org/sort/new"
                    || l.rel == "http://opds-spec.org/sort/popular"
                    || l.rel == "http://opds-spec.org/featured"
                    || l.rel == "http://opds-spec.org/recommended"
            })
            .or_else(|| {
                self.links
                    .iter()
                    .find(|l| l.is_catalog() && l.rel != "self" && !l.href.is_empty())
            })
    }

    /// First acquisition link matching the preference list, in preference order. Preferences are
    /// extensions (`epub`, `kepub`, `mobi`, `azw3`, `pdf`) or MIME types.
    pub fn best_acquisition(&self, prefs: &[&str]) -> Option<&Link> {
        let acq = self.acquisition_links();
        prefs.iter().find_map(|p| {
            let want = mime_for_pref(p);
            acq.iter().copied().find(|l| mime_matches(&l.mime, &want))
        })
    }
}

fn mime_for_pref(p: &str) -> Vec<String> {
    let p = p.trim().to_ascii_lowercase();
    let v: &[&str] = match p.as_str() {
        "epub" => &["application/epub+zip"],
        "kepub" => &["application/kepub+zip"],
        "mobi" => &["application/x-mobipocket-ebook"],
        "azw3" | "azw" => &["application/x-mobi8-ebook", "application/vnd.amazon.ebook"],
        "pdf" => &["application/pdf"],
        "fb2" => &["application/x-fictionbook+xml", "application/fb2+zip"],
        "cbz" => &["application/vnd.comicbook+zip", "application/x-cbz"],
        _ => return vec![p],
    };
    v.iter().map(|s| s.to_string()).collect()
}

fn mime_matches(link_mime: &str, wanted: &[String]) -> bool {
    let base = link_mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    wanted.contains(&base)
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Feed {
    pub id: String,
    pub title: String,
    pub updated: String,
    /// URL the feed was fetched from.
    pub url: String,
    pub links: Vec<Link>,
    pub entries: Vec<Entry>,
}

/// Where a catalogue's search lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchLink {
    /// URL template with `{searchTerms}`.
    Template(String),
    /// URL of an OpenSearch description document to fetch and parse first.
    Description(String),
}

impl Feed {
    fn link_with_rel(&self, rel: &str) -> Option<&Link> {
        self.links.iter().find(|l| l.rel == rel)
    }

    pub fn next(&self) -> Option<&str> {
        self.link_with_rel("next").map(|l| l.href.as_str())
    }

    pub fn previous(&self) -> Option<&str> {
        self.link_with_rel("previous")
            .or_else(|| self.link_with_rel("prev"))
            .map(|l| l.href.as_str())
    }

    pub fn start(&self) -> Option<&str> {
        self.link_with_rel("start").map(|l| l.href.as_str())
    }

    /// A direct template link wins over an OpenSearch description.
    pub fn search(&self) -> Option<SearchLink> {
        let searches: Vec<&Link> = self.links.iter().filter(|l| l.rel == "search").collect();
        if let Some(l) = searches.iter().find(|l| l.href.contains("{searchTerms")) {
            return Some(SearchLink::Template(l.href.clone()));
        }
        searches
            .iter()
            .find(|l| l.mime.contains("opensearchdescription"))
            .map(|l| SearchLink::Description(l.href.clone()))
    }

    /// True if the feed lists books (any entry has an acquisition link).
    pub fn is_acquisition(&self) -> bool {
        if let Some(l) = self.link_with_rel("self") {
            let m = l.mime.to_ascii_lowercase();
            if m.contains("kind=acquisition") {
                return true;
            }
            if m.contains("kind=navigation") {
                return false;
            }
        }
        self.entries.iter().any(|e| e.is_acquisition())
    }
}

/// Resolves `href` against `base` (RFC 3986). Falls back to `href` itself if `base` is not an
/// absolute URL or the result is not valid. `{template}` braces are preserved.
pub fn resolve_url(base: &str, href: &str) -> String {
    let href = href.trim();
    let joined = Url::parse(base).ok().and_then(|b| b.join(href).ok());
    match joined {
        Some(u) => {
            let s = u.to_string();
            if href.contains('{') {
                s.replace("%7B", "{").replace("%7D", "}")
            } else {
                s
            }
        }
        None => href.to_string(),
    }
}

fn is_atom(n: &Node, name: &str) -> bool {
    n.is_element() && n.tag_name().name() == name && n.tag_name().namespace() == Some(ATOM_NS)
}

fn text_of(n: &Node) -> String {
    let kind = n.attribute("type").unwrap_or("text").to_ascii_lowercase();
    match kind.as_str() {
        "xhtml" | "application/xhtml+xml" => {
            let t: String = n
                .descendants()
                .filter(|d| d.is_text())
                .filter_map(|d| d.text())
                .collect();
            t.split_whitespace().collect::<Vec<_>>().join(" ")
        }
        "html" | "text/html" => strip_tags(n.text().unwrap_or("")),
        _ => n
            .text()
            .unwrap_or("")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn base_of(n: &Node, parent: &str) -> String {
    match n.attribute((XML_NS, "base")) {
        Some(b) => resolve_url(parent, b),
        None => parent.to_string(),
    }
}

fn parse_link(n: &Node, base: &str) -> Link {
    Link {
        rel: n.attribute("rel").unwrap_or("alternate").trim().to_string(),
        href: resolve_url(base, n.attribute("href").unwrap_or("")),
        mime: n.attribute("type").unwrap_or("").trim().to_string(),
        title: n.attribute("title").unwrap_or("").trim().to_string(),
        length: n.attribute("length").and_then(|l| l.trim().parse().ok()),
    }
}

fn is_dc(n: &Node, name: &str) -> bool {
    n.is_element()
        && n.tag_name().name() == name
        && matches!(
            n.tag_name().namespace(),
            Some("http://purl.org/dc/terms/") | Some("http://purl.org/dc/elements/1.1/")
        )
}

fn parse_entry(n: &Node, base: &str) -> Entry {
    let base = base_of(n, base);
    let mut e = Entry::default();
    let mut content = String::new();
    for c in n.children().filter(|c| c.is_element()) {
        if is_atom(&c, "id") {
            e.id = c.text().unwrap_or("").trim().to_string();
        } else if is_atom(&c, "title") {
            e.title = text_of(&c);
        } else if is_atom(&c, "updated") {
            e.updated = c.text().unwrap_or("").trim().to_string();
        } else if is_atom(&c, "published") {
            e.published = c.text().unwrap_or("").trim().to_string();
        } else if is_atom(&c, "author") {
            if let Some(name) = c.children().find(|x| is_atom(x, "name")) {
                let name = text_of(&name);
                if !name.is_empty() {
                    e.authors.push(name);
                }
            }
        } else if is_atom(&c, "summary") {
            e.summary = text_of(&c);
        } else if is_atom(&c, "content") {
            content = text_of(&c);
        } else if is_atom(&c, "category") {
            if let Some(t) = c.attribute("label").or_else(|| c.attribute("term")) {
                e.categories.push(t.to_string());
            }
        } else if is_atom(&c, "link") {
            e.links.push(parse_link(&c, &base));
        } else if is_dc(&c, "language") {
            e.language = c.text().unwrap_or("").trim().to_string();
        } else if is_dc(&c, "publisher") {
            e.publisher = c.text().unwrap_or("").trim().to_string();
        } else if is_dc(&c, "issued") && e.published.is_empty() {
            e.published = c.text().unwrap_or("").trim().to_string();
        } else if c.tag_name().namespace() == Some("http://calibre.kovidgoyal.net/2009/metadata") {
            match c.tag_name().name() {
                "series" => e.series = c.text().map(|s| s.trim().to_string()),
                "series_index" => e.series_index = c.text().and_then(|s| s.trim().parse().ok()),
                _ => {}
            }
        }
    }
    if e.summary.is_empty() {
        e.summary = content;
    }
    e
}

/// Parses an OPDS Atom feed fetched from `url`.
pub fn parse_feed(xml: &str, url: &str) -> Result<Feed> {
    let doc = Document::parse_with_options(
        xml.trim_start_matches('\u{feff}'),
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .context("not a valid XML document")?;
    let root = doc.root_element();
    if !is_atom(&root, "feed") {
        bail!(
            "not an Atom feed (root element is <{}>)",
            root.tag_name().name()
        );
    }
    let base = base_of(&root, url);
    let mut feed = Feed {
        url: url.to_string(),
        ..Default::default()
    };
    for c in root.children().filter(|c| c.is_element()) {
        if is_atom(&c, "id") {
            feed.id = c.text().unwrap_or("").trim().to_string();
        } else if is_atom(&c, "title") {
            feed.title = text_of(&c);
        } else if is_atom(&c, "updated") {
            feed.updated = c.text().unwrap_or("").trim().to_string();
        } else if is_atom(&c, "link") {
            feed.links.push(parse_link(&c, &base));
        } else if is_atom(&c, "entry") {
            feed.entries.push(parse_entry(&c, &base));
        }
    }
    Ok(feed)
}

/// Extracts the Atom search URL template from an OpenSearch description document.
pub fn parse_opensearch_description(xml: &str, url: &str) -> Result<Option<String>> {
    let doc = Document::parse(xml).context("not a valid XML document")?;
    let root = doc.root_element();
    if root.tag_name().name() != "OpenSearchDescription" {
        bail!("not an OpenSearch description");
    }
    let urls: Vec<Node> = root
        .children()
        .filter(|n| {
            n.is_element()
                && n.tag_name().name() == "Url"
                && (n.tag_name().namespace() == Some(OPENSEARCH_NS)
                    || n.tag_name().namespace().is_none())
        })
        .collect();
    let pick = urls
        .iter()
        .find(|n| n.attribute("type").is_some_and(|t| t.contains("atom+xml")))
        .or_else(|| urls.first());
    Ok(pick
        .and_then(|n| n.attribute("template"))
        .map(|t| resolve_url(url, t)))
}

/// Fills a search template: `{searchTerms}` gets the percent-encoded terms, other parameters
/// (`{startPage?}`, `{count}`, ...) are removed.
pub fn expand_search_template(template: &str, terms: &str) -> String {
    let encoded = utf8_percent_encode(terms, UNRESERVED).to_string();
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let name = after[..close].trim_end_matches('?');
                if name == "searchTerms" || name.ends_with(":searchTerms") {
                    out.push_str(&encoded);
                }
                rest = &after[close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUTENBERG_ROOT: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns:dcterms="http://purl.org/dc/terms/" xmlns:opds="http://opds-spec.org/2010/catalog" xmlns:opensearch="http://a9.com/-/spec/opensearch/1.1/" xmlns="http://www.w3.org/2005/Atom">
  <id>https://www.gutenberg.org/ebooks.opds/</id>
  <updated>2024-03-01T04:05:06+00:00</updated>
  <title>Project Gutenberg</title>
  <subtitle>Free ebooks since 1971.</subtitle>
  <author><name>Marcello Perathoner</name><uri>https://www.gutenberg.org</uri></author>
  <link rel="search" type="application/opensearchdescription+xml" title="Project Gutenberg Catalog Search" href="/catalog/osd-books.xml"/>
  <link rel="self" title="This Page" type="application/atom+xml;profile=opds-catalog;kind=navigation" href="/ebooks.opds/"/>
  <link rel="start" title="Start Page" type="application/atom+xml;profile=opds-catalog;kind=navigation" href="/ebooks.opds/"/>
  <opensearch:itemsPerPage>25</opensearch:itemsPerPage>
  <entry>
    <updated>2024-03-01T04:05:06+00:00</updated>
    <title>Popular</title>
    <content type="text">Our most popular books.</content>
    <id>https://www.gutenberg.org/ebooks/search.opds/?sort_order=downloads</id>
    <link type="application/atom+xml;profile=opds-catalog;kind=acquisition" rel="subsection" href="/ebooks/search.opds/?sort_order=downloads"/>
  </entry>
  <entry>
    <updated>2024-03-01T04:05:06+00:00</updated>
    <title>Authors</title>
    <content type="text">Authors A-Z.</content>
    <id>https://www.gutenberg.org/ebooks/authors/search.opds/</id>
    <link type="application/atom+xml;profile=opds-catalog;kind=navigation" rel="subsection" href="/ebooks/authors/search.opds/"/>
  </entry>
</feed>"#;

    const GUTENBERG_BOOKS: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns:dcterms="http://purl.org/dc/terms/" xmlns:opds="http://opds-spec.org/2010/catalog" xmlns="http://www.w3.org/2005/Atom">
  <id>https://www.gutenberg.org/ebooks/search.opds/?query=dickens</id>
  <title>Books: dickens</title>
  <updated>2024-03-01T04:05:06+00:00</updated>
  <link rel="self" href="/ebooks/search.opds/?query=dickens" type="application/atom+xml;profile=opds-catalog;kind=acquisition"/>
  <link rel="next" title="Next Page" type="application/atom+xml;profile=opds-catalog;kind=acquisition" href="/ebooks/search.opds/?query=dickens&amp;start_index=26"/>
  <entry>
    <updated>2023-12-01T00:00:00+00:00</updated>
    <id>urn:gutenberg:1400:3</id>
    <title>Great Expectations</title>
    <content type="xhtml"><div xmlns="http://www.w3.org/1999/xhtml">Charles Dickens &amp; friends.<br/> Downloaded 1000 times.</div></content>
    <author><name>Dickens, Charles</name></author>
    <category term="Bildungsromans" label="Bildungsromans"/>
    <dcterms:language>en</dcterms:language>
    <link type="application/epub+zip" rel="http://opds-spec.org/acquisition" href="/ebooks/1400.epub3.images" title="EPUB3 (E-readers incl. Send-to-Kindle)" length="1234567"/>
    <link type="application/x-mobipocket-ebook" rel="http://opds-spec.org/acquisition" href="/ebooks/1400.kf8.images" title="Kindle"/>
    <link type="image/jpeg" rel="http://opds-spec.org/image" href="https://www.gutenberg.org/cache/epub/1400/pg1400.cover.medium.jpg"/>
    <link type="image/jpeg" rel="http://opds-spec.org/image/thumbnail" href="/cache/epub/1400/pg1400.cover.small.jpg"/>
    <link type="application/atom+xml" rel="related" href="/ebooks/1400.opds" title="Related"/>
  </entry>
</feed>"#;

    const STANDARD_EBOOKS: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:schema="http://schema.org/" xml:base="https://standardebooks.org">
  <id>https://standardebooks.org/feeds/opds/new-releases</id>
  <title>Standard Ebooks - Newest 15 Ebooks</title>
  <updated>2024-02-02T00:00:00Z</updated>
  <link href="/feeds/opds/new-releases" rel="self" type="application/atom+xml;profile=opds-catalog;kind=acquisition"/>
  <link href="/feeds/opds" rel="start" type="application/atom+xml;profile=opds-catalog;kind=navigation"/>
  <link href="/feeds/opds/all?query={searchTerms}" rel="search" type="application/atom+xml"/>
  <entry>
    <id>https://standardebooks.org/ebooks/jane-austen/pride-and-prejudice</id>
    <title>Pride and Prejudice</title>
    <updated>2024-01-30T10:00:00Z</updated>
    <published>2013-05-01T00:00:00Z</published>
    <author><name>Jane Austen</name><uri>https://standardebooks.org/ebooks/jane-austen</uri></author>
    <dc:language>en-GB</dc:language>
    <dc:publisher>Standard Ebooks</dc:publisher>
    <summary type="text">A novel of manners.</summary>
    <category scheme="http://purl.org/dc/terms/LCSH" term="Fiction"/>
    <category scheme="http://purl.org/dc/terms/LCSH" term="Courtship -- Fiction"/>
    <link href="/ebooks/jane-austen/pride-and-prejudice/dist/cover.jpg" rel="http://opds-spec.org/image" type="image/jpeg"/>
    <link href="/ebooks/jane-austen/pride-and-prejudice/dist/cover-thumbnail.jpg" rel="http://opds-spec.org/image/thumbnail" type="image/jpeg"/>
    <link href="/ebooks/jane-austen_pride-and-prejudice.epub?source=download" rel="http://opds-spec.org/acquisition" title="Recommended compatible epub" type="application/epub+zip"/>
    <link href="/ebooks/jane-austen_pride-and-prejudice_advanced.epub?source=download" rel="http://opds-spec.org/acquisition" title="Advanced epub" type="application/epub+zip"/>
    <link href="/ebooks/jane-austen_pride-and-prejudice.kepub.epub?source=download" rel="http://opds-spec.org/acquisition" title="Kobo Kepub epub" type="application/kepub+zip"/>
    <link href="/ebooks/jane-austen_pride-and-prejudice.azw3?source=download" rel="http://opds-spec.org/acquisition" title="Amazon Kindle azw3" type="application/x-mobi8-ebook"/>
  </entry>
</feed>"#;

    const CALIBRE: &str = r#"<?xml version='1.0' encoding='UTF-8'?>
<feed xmlns:dc="http://purl.org/dc/terms/" xmlns="http://www.w3.org/2005/Atom" xmlns:calibre="http://calibre.kovidgoyal.net/2009/metadata">
  <title>calibre Library</title>
  <id>urn:uuid:abc</id>
  <updated>2024-01-01T00:00:00+00:00</updated>
  <link href="/opds/navcatalog/4f6e6577657374?library_id=Lib" type="application/atom+xml;profile=opds-catalog;kind=acquisition" rel="self"/>
  <link href="../search/{searchTerms}" type="application/atom+xml" rel="search"/>
  <link href="navcatalog/page2" rel="next" type="application/atom+xml"/>
  <entry>
    <title>The Hobbit</title>
    <id>urn:uuid:1</id>
    <updated>2023-01-01T00:00:00+00:00</updated>
    <author><name>J.R.R. Tolkien</name></author>
    <calibre:series>Middle-earth</calibre:series>
    <calibre:series_index>0.5</calibre:series_index>
    <dc:publisher>Allen &amp; Unwin</dc:publisher>
    <content type="html">&lt;div&gt;&lt;p&gt;There &amp;amp; back.&lt;/p&gt;&lt;/div&gt;</content>
    <link type="application/epub+zip" href="/get/epub/1/Lib" rel="http://opds-spec.org/acquisition"/>
    <link type="image/jpeg" href="/get/cover/1/Lib" rel="x-stanza-cover-image"/>
    <link type="image/jpeg" href="/get/thumb/1/Lib" rel="x-stanza-cover-image-thumbnail"/>
  </entry>
</feed>"#;

    const BASE: &str = "https://www.gutenberg.org/ebooks.opds/";

    #[test]
    fn resolves_relative_links() {
        assert_eq!(resolve_url("https://a.org/x/y/", "/z"), "https://a.org/z");
        assert_eq!(
            resolve_url("https://a.org/x/y/", "z"),
            "https://a.org/x/y/z"
        );
        assert_eq!(
            resolve_url("https://a.org/x/y/", "../z"),
            "https://a.org/x/z"
        );
        assert_eq!(resolve_url("https://a.org/x/y", "z"), "https://a.org/x/z");
        assert_eq!(
            resolve_url("https://a.org/x", "//b.org/q"),
            "https://b.org/q"
        );
        assert_eq!(
            resolve_url("https://a.org/x", "https://c.org/"),
            "https://c.org/"
        );
        assert_eq!(
            resolve_url("https://a.org/x?y=1", "?p=2"),
            "https://a.org/x?p=2"
        );
        assert_eq!(
            resolve_url("http://a.org/x", "#frag"),
            "http://a.org/x#frag"
        );
        assert_eq!(resolve_url("relative/base", "z"), "z");
        assert_eq!(resolve_url("", "z"), "z");
        assert_eq!(
            resolve_url("https://a.org/", "  /trim  "),
            "https://a.org/trim"
        );
    }

    #[test]
    fn template_braces_survive_resolution() {
        assert_eq!(
            resolve_url("https://a.org/opds/", "../search/{searchTerms}"),
            "https://a.org/search/{searchTerms}"
        );
        assert_eq!(
            resolve_url("https://a.org/opds/", "/s?q={searchTerms}"),
            "https://a.org/s?q={searchTerms}"
        );
    }

    #[test]
    fn gutenberg_root_navigation() {
        let f = parse_feed(GUTENBERG_ROOT, BASE).unwrap();
        assert_eq!(f.title, "Project Gutenberg");
        assert_eq!(f.id, "https://www.gutenberg.org/ebooks.opds/");
        assert_eq!(f.updated, "2024-03-01T04:05:06+00:00");
        assert_eq!(f.entries.len(), 2);
        assert!(!f.is_acquisition());
        assert_eq!(f.start(), Some("https://www.gutenberg.org/ebooks.opds/"));
        assert_eq!(f.next(), None);
        assert_eq!(f.previous(), None);
        let nav = f.entries[1].navigation_link().unwrap();
        assert_eq!(
            nav.href,
            "https://www.gutenberg.org/ebooks/authors/search.opds/"
        );
        assert_eq!(f.entries[1].summary, "Authors A-Z.");
        assert!(!f.entries[1].is_acquisition());
        assert_eq!(
            f.search(),
            Some(SearchLink::Description(
                "https://www.gutenberg.org/catalog/osd-books.xml".into()
            ))
        );
    }

    #[test]
    fn gutenberg_acquisition_feed() {
        let url = "https://www.gutenberg.org/ebooks/search.opds/?query=dickens";
        let f = parse_feed(GUTENBERG_BOOKS, url).unwrap();
        assert!(f.is_acquisition());
        assert_eq!(
            f.next(),
            Some("https://www.gutenberg.org/ebooks/search.opds/?query=dickens&start_index=26")
        );
        let e = &f.entries[0];
        assert_eq!(e.title, "Great Expectations");
        assert_eq!(e.id, "urn:gutenberg:1400:3");
        assert_eq!(e.authors, ["Dickens, Charles"]);
        assert_eq!(e.categories, ["Bildungsromans"]);
        assert_eq!(e.language, "en");
        // xhtml content flattened to text
        assert_eq!(
            e.summary,
            "Charles Dickens & friends. Downloaded 1000 times."
        );
        assert!(e.is_acquisition());
        let acq = e.acquisition_links();
        assert_eq!(acq.len(), 2);
        assert_eq!(
            acq[0].href,
            "https://www.gutenberg.org/ebooks/1400.epub3.images"
        );
        assert_eq!(acq[0].length, Some(1234567));
        assert_eq!(acq[0].title, "EPUB3 (E-readers incl. Send-to-Kindle)");
        assert_eq!(acq[1].length, None);
        assert_eq!(
            e.cover().unwrap().href,
            "https://www.gutenberg.org/cache/epub/1400/pg1400.cover.medium.jpg"
        );
        assert_eq!(
            e.thumbnail().unwrap().href,
            "https://www.gutenberg.org/cache/epub/1400/pg1400.cover.small.jpg"
        );
        assert!(e.navigation_link().is_none());
    }

    #[test]
    fn standard_ebooks_uses_xml_base_and_search_template() {
        let f = parse_feed(
            STANDARD_EBOOKS,
            "https://standardebooks.org/feeds/opds/new-releases",
        )
        .unwrap();
        assert_eq!(f.title, "Standard Ebooks - Newest 15 Ebooks");
        assert_eq!(
            f.search(),
            Some(SearchLink::Template(
                "https://standardebooks.org/feeds/opds/all?query={searchTerms}".into()
            ))
        );
        let e = &f.entries[0];
        assert_eq!(e.language, "en-GB");
        assert_eq!(e.publisher, "Standard Ebooks");
        assert_eq!(e.published, "2013-05-01T00:00:00Z");
        assert_eq!(e.categories, ["Fiction", "Courtship -- Fiction"]);
        assert_eq!(e.summary, "A novel of manners.");
        assert_eq!(e.acquisition_links().len(), 4);
        assert_eq!(
            e.cover().unwrap().href,
            "https://standardebooks.org/ebooks/jane-austen/pride-and-prejudice/dist/cover.jpg"
        );
    }

    #[test]
    fn best_acquisition_follows_preferences() {
        let f = parse_feed(STANDARD_EBOOKS, "https://standardebooks.org/").unwrap();
        let e = &f.entries[0];
        assert!(e
            .best_acquisition(&["kepub", "epub"])
            .unwrap()
            .href
            .contains("kepub.epub"));
        assert!(e
            .best_acquisition(&["azw3", "epub"])
            .unwrap()
            .href
            .contains(".azw3"));
        // first matching link of the type (the "recommended" epub, not the advanced one)
        assert_eq!(
            e.best_acquisition(&["epub"]).unwrap().title,
            "Recommended compatible epub"
        );
        // raw mime types work too, with parameters on the link ignored
        assert!(e.best_acquisition(&["application/x-mobi8-ebook"]).is_some());
        assert!(e.best_acquisition(&["pdf", "mobi"]).is_none());
        assert!(e.best_acquisition(&[]).is_none());
        let g = parse_feed(GUTENBERG_BOOKS, BASE).unwrap();
        assert!(g.entries[0]
            .best_acquisition(&["mobi"])
            .unwrap()
            .href
            .contains("kf8"));
        assert!(g.entries[0].best_acquisition(&["kepub"]).is_none());
    }

    #[test]
    fn mime_parameters_are_ignored_when_matching() {
        assert!(mime_matches(
            "application/epub+zip; charset=binary",
            &mime_for_pref("EPUB")
        ));
        assert!(!mime_matches("application/pdf", &mime_for_pref("epub")));
        assert_eq!(mime_for_pref("image/png"), vec!["image/png".to_string()]);
    }

    #[test]
    fn calibre_feed_relative_links_series_and_html_content() {
        let f = parse_feed(
            CALIBRE,
            "http://192.168.1.5:8080/opds/navcatalog/4f6e6577657374",
        )
        .unwrap();
        assert_eq!(
            f.search(),
            Some(SearchLink::Template(
                "http://192.168.1.5:8080/opds/search/{searchTerms}".into()
            ))
        );
        assert_eq!(
            f.next(),
            Some("http://192.168.1.5:8080/opds/navcatalog/navcatalog/page2")
        );
        let e = &f.entries[0];
        assert_eq!(e.series.as_deref(), Some("Middle-earth"));
        assert_eq!(e.series_index, Some(0.5));
        assert_eq!(e.publisher, "Allen & Unwin");
        assert_eq!(e.summary, "There & back.");
        assert_eq!(
            e.cover().unwrap().href,
            "http://192.168.1.5:8080/get/cover/1/Lib"
        );
        assert_eq!(
            e.thumbnail().unwrap().href,
            "http://192.168.1.5:8080/get/thumb/1/Lib"
        );
        assert_eq!(
            e.acquisition_links()[0].href,
            "http://192.168.1.5:8080/get/epub/1/Lib"
        );
    }

    #[test]
    fn feed_without_self_kind_is_classified_by_entries() {
        let f = parse_feed(CALIBRE, "http://x/").unwrap();
        // self link says kind=acquisition
        assert!(f.is_acquisition());
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>t</title><entry><title>e</title><link rel="http://opds-spec.org/acquisition/open-access" href="a.epub" type="application/epub+zip"/></entry></feed>"#;
        let f = parse_feed(xml, "http://x/dir/").unwrap();
        assert!(f.is_acquisition());
        assert_eq!(
            f.entries[0].acquisition_links()[0].href,
            "http://x/dir/a.epub"
        );
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>t</title><entry><title>e</title><link rel="subsection" href="x" type="application/atom+xml"/></entry></feed>"#;
        assert!(!parse_feed(xml, "http://x/").unwrap().is_acquisition());
    }

    #[test]
    fn empty_feed_and_defaults() {
        let f = parse_feed(
            r#"<feed xmlns="http://www.w3.org/2005/Atom"/>"#,
            "http://x/",
        )
        .unwrap();
        assert!(f.entries.is_empty());
        assert!(f.links.is_empty());
        assert_eq!(f.title, "");
        assert_eq!(f.next(), None);
        assert_eq!(f.search(), None);
        assert!(!f.is_acquisition());
    }

    #[test]
    fn link_without_rel_is_alternate_and_catalog_links_are_navigation() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom"><entry><title>Sub</title><link href="/c" type="application/atom+xml;profile=opds-catalog"/></entry></feed>"#;
        let f = parse_feed(xml, "http://x/a").unwrap();
        let e = &f.entries[0];
        assert_eq!(e.links[0].rel, "alternate");
        assert_eq!(e.navigation_link().unwrap().href, "http://x/c");
    }

    #[test]
    fn entry_with_only_self_link_has_no_navigation() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom"><entry><title>X</title><link rel="self" href="/c" type="application/atom+xml"/></entry></feed>"#;
        let f = parse_feed(xml, "http://x/a").unwrap();
        assert!(f.entries[0].navigation_link().is_none());
    }

    #[test]
    fn previous_link_variants() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom"><link rel="prev" href="/p1"/><link rel="next" href="/p3"/></feed>"#;
        let f = parse_feed(xml, "http://x/p2").unwrap();
        assert_eq!(f.previous(), Some("http://x/p1"));
        assert_eq!(f.next(), Some("http://x/p3"));
    }

    #[test]
    fn rejects_non_feeds() {
        let e = parse_feed("<html><body/></html>", "http://x/").unwrap_err();
        assert!(format!("{e}").contains("not an Atom feed (root element is <html>)"));
        let e = parse_feed("<feed><entry/></feed>", "http://x/").unwrap_err();
        assert!(format!("{e}").contains("not an Atom feed"), "no namespace");
        let e =
            parse_feed("<feed xmlns=\"http://www.w3.org/2005/Atom\">", "http://x/").unwrap_err();
        assert!(format!("{e:#}").contains("not a valid XML"));
        assert!(parse_feed("", "http://x/").is_err());
        assert!(parse_feed("\u{0}\u{1}garbage", "http://x/").is_err());
    }

    #[test]
    fn accepts_bom_and_unknown_elements() {
        let xml = "\u{feff}<feed xmlns=\"http://www.w3.org/2005/Atom\" xmlns:x=\"urn:x\"><x:weird a=\"1\"><deep/></x:weird><title>T</title></feed>";
        assert_eq!(parse_feed(xml, "http://x/").unwrap().title, "T");
    }

    #[test]
    fn entities_and_unicode_in_text() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom"><entry><title>Tom &amp; Jerry — 日本語 &#x1F642;</title><author><name>Üñí</name></author><author><name>  </name></author></entry></feed>"#;
        let f = parse_feed(xml, "http://x/").unwrap();
        assert_eq!(f.entries[0].title, "Tom & Jerry — 日本語 🙂");
        assert_eq!(f.entries[0].authors, ["Üñí"]);
    }

    #[test]
    fn opensearch_description_parsing() {
        let xml = r#"<?xml version="1.0"?>
<OpenSearchDescription xmlns="http://a9.com/-/spec/opensearch/1.1/">
 <ShortName>x</ShortName>
 <Url type="text/html" template="/html?q={searchTerms}"/>
 <Url type="application/atom+xml" template="/atom?q={searchTerms}&amp;p={startPage?}"/>
</OpenSearchDescription>"#;
        assert_eq!(
            parse_opensearch_description(xml, "https://a.org/osd.xml").unwrap(),
            Some("https://a.org/atom?q={searchTerms}&p={startPage?}".into())
        );
        let only_html = r#"<OpenSearchDescription xmlns="http://a9.com/-/spec/opensearch/1.1/"><Url type="text/html" template="/h?q={searchTerms}"/></OpenSearchDescription>"#;
        assert_eq!(
            parse_opensearch_description(only_html, "https://a.org/osd.xml").unwrap(),
            Some("https://a.org/h?q={searchTerms}".into())
        );
        let none = r#"<OpenSearchDescription xmlns="http://a9.com/-/spec/opensearch/1.1/"/>"#;
        assert_eq!(
            parse_opensearch_description(none, "https://a.org/").unwrap(),
            None
        );
        assert!(parse_opensearch_description("<feed/>", "https://a.org/").is_err());
        assert!(parse_opensearch_description("nope", "https://a.org/").is_err());
    }

    #[test]
    fn search_template_expansion() {
        assert_eq!(
            expand_search_template("https://a.org/s?q={searchTerms}", "war & peace"),
            "https://a.org/s?q=war%20%26%20peace"
        );
        assert_eq!(
            expand_search_template("/s?q={searchTerms}&p={startPage?}&c={count}", "x"),
            "/s?q=x&p=&c="
        );
        assert_eq!(
            expand_search_template("/s/{searchTerms?}", "日本"),
            "/s/%E6%97%A5%E6%9C%AC"
        );
        assert_eq!(expand_search_template("/s?{os:searchTerms}", "a"), "/s?a");
        assert_eq!(expand_search_template("/plain", "x"), "/plain");
        assert_eq!(
            expand_search_template("/broken{searchTerms", "x"),
            "/broken{searchTerms"
        );
        assert_eq!(expand_search_template("", "x"), "");
        assert_eq!(
            expand_search_template("/{searchTerms}/{searchTerms}", "a b"),
            "/a%20b/a%20b"
        );
    }

    #[test]
    fn roundtrip_with_our_own_server_feed() {
        use bookx_core::library::Book;
        use bookx_core::opf::Metadata;
        let now = chrono::Utc::now();
        let b = Book {
            id: 3,
            file: "x.epub".into(),
            cover: Some("c.jpg".into()),
            meta: Metadata {
                title: "Q & A".into(),
                authors: vec!["Ann".into()],
                series: "S".into(),
                series_index: 2.0,
                tags: vec!["t1".into()],
                language: "de".into(),
                ..Default::default()
            },
            added: now,
            modified: now,
            last_read: None,
            size: 99,
            progress: 0.0,
            position: None,
            bookmarks: vec![],
            highlights: vec![],
            formats: vec![],
            resources: vec![],
            drm: String::new(),
            custom: Default::default(),
            marked: false,
        };
        let cfg = crate::opds::OpdsConfig::default();
        let doc =
            crate::opds::route(&cfg, &[b], &["all".to_string()], &Default::default()).unwrap();
        let f = parse_feed(&doc.body, "http://h:1/opds/all").unwrap();
        let e = &f.entries[0];
        assert_eq!(e.title, "Q & A");
        assert_eq!(e.authors, ["Ann"]);
        assert_eq!(e.series.as_deref(), Some("S"));
        assert_eq!(e.series_index, Some(2.0));
        assert_eq!(e.categories, ["t1"]);
        assert_eq!(e.language, "de");
        assert_eq!(e.acquisition_links()[0].href, "http://h:1/download/3.epub");
        assert_eq!(e.acquisition_links()[0].length, Some(99));
        assert_eq!(e.cover().unwrap().href, "http://h:1/cover/3");
        assert_eq!(
            f.search(),
            Some(SearchLink::Template(
                "http://h:1/opds/search?q={searchTerms}".into()
            ))
        );
        assert_eq!(f.start(), Some("http://h:1/opds"));
    }
}
