//! Browsing OPDS catalogues (P18, S3), without GTK: the catalogue list, loading a feed, search,
//! and picking and downloading a format. Parsing is `bookx_serve::opds_client`; the network is the
//! `Fetcher` of `bookx-sources`, so tests use canned pages.
//!
//! `$BOOKX_OPDS_MOCK` (a folder) makes `https://mock.example/opds/<file>` answer with that file,
//! for the UI tests.

use crate::i18n::tr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use bookx_serve::opds_client::{self as opds, Entry, Feed, Link, SearchLink};
use bookx_sources::fetch::Fetcher;
use serde::{Deserialize, Serialize};

/// Formats in the order Bookx prefers them when a book is offered in several.
pub const PREFERRED: [&str; 7] = ["epub", "kepub", "azw3", "mobi", "fb2", "cbz", "pdf"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Catalog {
    pub id: String,
    pub name: String,
    pub url: String,
    pub user: String,
    /// Kept in the keyring like the other passwords (see `Config::secret_fields`).
    pub password: String,
}

/// Catalogues that are always listed.
pub fn presets() -> Vec<Catalog> {
    vec![
        Catalog {
            id: "gutenberg".into(),
            name: tr("Project Gutenberg"),
            url: "https://www.gutenberg.org/ebooks.opds/".into(),
            ..Catalog::default()
        },
        Catalog {
            id: "standardebooks".into(),
            name: tr("Standard Ebooks"),
            url: "https://standardebooks.org/feeds/opds".into(),
            ..Catalog::default()
        },
    ]
}

pub fn password_key(id: &str) -> String {
    format!("opds-{id}-password")
}

/// The always-listed catalogues with the sign-in the user saved for each (`logins` is keyed by id;
/// only its user name and password are used).
pub fn with_logins(presets: Vec<Catalog>, logins: &[Catalog]) -> Vec<Catalog> {
    presets
        .into_iter()
        .map(|mut p| {
            if let Some(l) = logins.iter().find(|l| l.id == p.id) {
                p.user = l.user.clone();
                p.password = l.password.clone();
            }
            p
        })
        .collect()
}

/// Saves (or, with an empty user name, forgets) the sign-in of the catalogue `id`.
pub fn set_login(logins: &mut Vec<Catalog>, id: &str, user: &str, password: &str) {
    logins.retain(|l| l.id != id);
    if !user.trim().is_empty() {
        logins.push(Catalog {
            id: id.to_string(),
            user: user.trim().to_string(),
            password: password.to_string(),
            ..Catalog::default()
        });
    }
}

/// A failed request in words a reader can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub message: String,
    /// A sign-in would help: the server asked for one, or did not accept the one given.
    pub sign_in: bool,
}

/// Explains why loading from catalogue `c` failed. The raw `HTTP error 401` helps nobody.
pub fn explain(c: &Catalog, err: &anyhow::Error) -> Problem {
    use bookx_sources::fetch::FetchError;
    let plain = |message: String| Problem {
        message,
        sign_in: false,
    };
    let Some(fe) = FetchError::of(err) else {
        return plain(format!("{err:#}"));
    };
    match fe {
        FetchError::Http(401 | 403) if c.user.is_empty() => Problem {
            message: if c.id == "standardebooks" {
                tr(
                    "Standard Ebooks gives its catalogue to Patrons Circle members. Press Sign In, enter the email address of your membership as the user name and leave the password empty.",
                )
            } else {
                tr("This catalogue asks for a user name and password. Press Sign In to enter them.")
            },
            sign_in: true,
        },
        FetchError::Http(401 | 403) => Problem {
            message: tr(
                "The catalogue did not accept the user name or password. Press Sign In to change them.",
            ),
            sign_in: true,
        },
        FetchError::Http(404 | 410) => plain(tr(
            "The catalogue has no page at this address (HTTP 404). Check the address.",
        )),
        FetchError::Http(c @ (429 | 500..=599)) => plain(crate::i18n::trf(
            "The catalogue's server is busy or down for maintenance (HTTP {}). That is on its side: try again later.",
            &[c],
        )),
        FetchError::Http(c) => plain(crate::i18n::trf(
            "The catalogue answered with an error (HTTP {}).",
            &[c],
        )),
        FetchError::Timeout => plain(tr(
            "The catalogue did not answer in time. Check the connection and try again.",
        )),
        FetchError::Other(m) => plain(crate::i18n::trf("Could not reach the catalogue: {}", &[m])),
    }
}

/// Adds a catalogue (checking the address) and returns its id.
pub fn add(
    list: &mut Vec<Catalog>,
    name: &str,
    url: &str,
    user: &str,
    password: &str,
) -> Result<String> {
    let name = name.trim();
    let url = url.trim();
    if name.is_empty() {
        bail!("Give the catalogue a name");
    }
    let parsed = url::Url::parse(url).map_err(|_| anyhow!("That is not a web address"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        bail!("The address must start with http:// or https://");
    }
    if list
        .iter()
        .chain(presets().iter())
        .any(|c| c.url == url || c.name.eq_ignore_ascii_case(name))
    {
        bail!("That catalogue is already in the list");
    }
    let mut n = 1;
    let base: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let mut id = format!("{base}-{n}");
    while list.iter().any(|c| c.id == id) {
        n += 1;
        id = format!("{base}-{n}");
    }
    list.push(Catalog {
        id: id.clone(),
        name: name.to_string(),
        url: url.to_string(),
        user: user.trim().to_string(),
        password: password.to_string(),
    });
    Ok(id)
}

/// The fetcher: the real network, or the canned pages of `$BOOKX_OPDS_MOCK`.
pub fn fetcher() -> Arc<dyn Fetcher> {
    match std::env::var_os("BOOKX_OPDS_MOCK") {
        Some(dir) => Arc::new(MockFiles(PathBuf::from(dir))),
        None => crate::diag_fetch::traced(
            Arc::new(bookx_sources::fetch::UreqFetcher::with_limits(
                std::time::Duration::from_secs(30),
                200 * 1024 * 1024,
            )),
            crate::trace::Area::Opds,
        ),
    }
}

struct MockFiles(PathBuf);

impl Fetcher for MockFiles {
    fn get(&self, url: &str, headers: &[(String, String)]) -> Result<Vec<u8>> {
        let rest = url
            .strip_prefix("https://mock.example/opds/")
            .ok_or_else(|| anyhow!("HTTP 404"))?;
        let name = rest.split(['?', '#']).next().unwrap_or("");
        if name.contains('/') || name.contains("..") {
            bail!("HTTP 404");
        }
        // like Standard Ebooks: a page called `private-…` answers 401 until a login is sent
        if name.starts_with("private-") && !headers.iter().any(|(k, _)| k == "Authorization") {
            return Err(bookx_sources::fetch::FetchError::Http(401).into());
        }
        std::fs::read(self.0.join(name)).map_err(|_| anyhow!("HTTP 404"))
    }
}

/// The user added this catalogue, so its own host may be on the LAN. Any other host a feed points
/// to still goes through the private-address guard (`bookx_sources::netguard`).
fn trust_catalogue(c: &Catalog) {
    bookx_sources::netguard::trust_url(&c.url);
}

/// `Authorization: Basic …` for a catalogue with a user name.
pub fn auth_headers(c: &Catalog) -> Vec<(String, String)> {
    if c.user.is_empty() {
        return Vec::new();
    }
    let token =
        base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", c.user, c.password));
    vec![("Authorization".into(), format!("Basic {token}"))]
}

/// Downloads and parses one feed. Credentials are sent only to the host of the catalogue.
pub fn load(fetcher: &dyn Fetcher, c: &Catalog, url: &str) -> Result<Feed> {
    trust_catalogue(c);
    let headers = if same_host(&c.url, url) {
        auth_headers(c)
    } else {
        Vec::new()
    };
    let body = fetcher
        .get(url, &headers)
        .with_context(|| format!("Could not load {url}"))?;
    let xml = String::from_utf8_lossy(&body);
    opds::parse_feed(&xml, url).with_context(|| "That address is not an OPDS catalogue")
}

fn same_host(a: &str, b: &str) -> bool {
    match (url::Url::parse(a), url::Url::parse(b)) {
        (Ok(a), Ok(b)) => a.host_str() == b.host_str() && a.scheme() == b.scheme(),
        _ => false,
    }
}

/// The address to load for a search in `feed`; follows an OpenSearch description if needed.
pub fn search_url(fetcher: &dyn Fetcher, c: &Catalog, feed: &Feed, terms: &str) -> Result<String> {
    trust_catalogue(c);
    let terms = terms.trim();
    if terms.is_empty() {
        bail!("Type something to search for");
    }
    let template = match feed.search() {
        Some(SearchLink::Template(t)) => t,
        Some(SearchLink::Description(u)) => {
            let headers = if same_host(&c.url, &u) {
                auth_headers(c)
            } else {
                Vec::new()
            };
            let body = fetcher
                .get(&u, &headers)
                .context("Could not load the search description")?;
            opds::parse_opensearch_description(&String::from_utf8_lossy(&body), &u)?
                .ok_or_else(|| anyhow!("This catalogue has no search"))?
        }
        None => bail!("This catalogue has no search"),
    };
    Ok(opds::expand_search_template(&template, terms))
}

/// The link to download: the best format by [`PREFERRED`], else the first acquisition link.
pub fn pick_link(e: &Entry) -> Option<&Link> {
    e.best_acquisition(&PREFERRED)
        .or_else(|| e.acquisition_links().into_iter().next())
}

/// The file extension for a download.
pub fn extension(link: &Link) -> String {
    let m = link
        .mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let by_mime = match m.as_str() {
        "application/epub+zip" => Some("epub"),
        "application/kepub+zip" => Some("kepub.epub"),
        "application/x-mobipocket-ebook" => Some("mobi"),
        "application/x-mobi8-ebook" | "application/vnd.amazon.ebook" => Some("azw3"),
        "application/pdf" => Some("pdf"),
        "application/x-fictionbook+xml" => Some("fb2"),
        "application/fb2+zip" => Some("fb2.zip"),
        "application/vnd.comicbook+zip" | "application/x-cbz" => Some("cbz"),
        "text/plain" => Some("txt"),
        _ => None,
    };
    if let Some(e) = by_mime {
        return e.to_string();
    }
    url::Url::parse(&link.href)
        .ok()
        .and_then(|u| {
            Path::new(u.path())
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
        })
        .filter(|e| !e.is_empty() && e.len() <= 8)
        .unwrap_or_else(|| "epub".into())
}

fn file_stem(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s: String = s.trim_matches('.').chars().take(100).collect();
    if s.is_empty() { "book".into() } else { s }
}

/// Downloads the entry's best format into `dir` and returns the path. Never overwrites.
pub fn download(fetcher: &dyn Fetcher, c: &Catalog, e: &Entry, dir: &Path) -> Result<PathBuf> {
    trust_catalogue(c);
    let link = pick_link(e).ok_or_else(|| anyhow!("“{}” has nothing to download", e.title))?;
    let headers = if same_host(&c.url, &link.href) {
        auth_headers(c)
    } else {
        Vec::new()
    };
    let body = fetcher
        .get(&link.href, &headers)
        .with_context(|| format!("Could not download “{}”", e.title))?;
    if body.is_empty() {
        bail!("The download of “{}” was empty", e.title);
    }
    std::fs::create_dir_all(dir)?;
    let stem = file_stem(&e.title);
    let ext = extension(link);
    let mut path = dir.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem} ({n}).{ext}"));
        n += 1;
    }
    std::fs::write(&path, body)?;
    Ok(path)
}

/// One line for a book entry: authors and the formats on offer.
pub fn describe(e: &Entry) -> String {
    let formats: Vec<String> = e
        .acquisition_links()
        .iter()
        .map(|l| extension(l).to_uppercase())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut parts = Vec::new();
    if !e.authors.is_empty() {
        parts.push(e.authors.join(", "));
    }
    if !formats.is_empty() {
        parts.push(formats.join(" · "));
    }
    parts.join(" — ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_sources::fetch::{MockFetcher, MockReply};

    const ROOT: &str = r#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:opds="http://opds-spec.org/2010/catalog">
 <id>root</id><title>Root</title>
 <link rel="self" href="/opds/root.xml" type="application/atom+xml;profile=opds-catalog;kind=navigation"/>
 <link rel="search" href="/opds/search?q={searchTerms}" type="application/atom+xml"/>
 <entry><id>1</id><title>New</title>
  <link rel="subsection" href="/opds/new.xml" type="application/atom+xml;profile=opds-catalog;kind=acquisition"/></entry>
</feed>"#;

    const BOOKS: &str = r#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom">
 <id>b</id><title>Books</title>
 <link rel="next" href="/opds/page2.xml" type="application/atom+xml"/>
 <entry><id>b1</id><title>A: Tale / Part</title><author><name>Ann Writer</name></author>
  <link rel="http://opds-spec.org/acquisition" href="/dl/a.mobi" type="application/x-mobipocket-ebook"/>
  <link rel="http://opds-spec.org/acquisition" href="/dl/a.epub" type="application/epub+zip"/>
 </entry>
</feed>"#;

    fn tmp() -> PathBuf {
        std::env::temp_dir().join(format!("bookx-opds-{}", uuid::Uuid::new_v4()))
    }

    fn cat() -> Catalog {
        Catalog {
            id: "x".into(),
            name: "X".into(),
            url: "https://cat.example/opds/root.xml".into(),
            user: "me".into(),
            password: "pw".into(),
        }
    }

    #[test]
    fn loads_navigation_and_acquisition_feeds() {
        let f = MockFetcher::new()
            .on(
                "https://cat.example/opds/root.xml",
                MockReply::Body(ROOT.as_bytes().to_vec()),
            )
            .on(
                "https://cat.example/opds/new.xml",
                MockReply::Body(BOOKS.as_bytes().to_vec()),
            );
        let root = load(&f, &cat(), "https://cat.example/opds/root.xml").unwrap();
        assert_eq!(root.entries.len(), 1);
        let nav = root.entries[0].navigation_link().unwrap();
        assert_eq!(nav.href, "https://cat.example/opds/new.xml");
        let books = load(&f, &cat(), &nav.href).unwrap();
        assert_eq!(books.next(), Some("https://cat.example/opds/page2.xml"));
        let e = &books.entries[0];
        assert_eq!(pick_link(e).unwrap().href, "https://cat.example/dl/a.epub");
        assert_eq!(describe(e), "Ann Writer — EPUB · MOBI");
    }

    #[test]
    fn credentials_go_only_to_the_catalogue_host() {
        let f = MockFetcher::new()
            .on(
                "https://cat.example/opds/root.xml",
                MockReply::Body(ROOT.as_bytes().to_vec()),
            )
            .on(
                "https://other.example/x.xml",
                MockReply::Body(ROOT.as_bytes().to_vec()),
            );
        load(&f, &cat(), "https://cat.example/opds/root.xml").unwrap();
        load(&f, &cat(), "https://other.example/x.xml").unwrap();
        let log = f.requests();
        assert_eq!(log[0].headers[0].0, "Authorization");
        assert!(log[0].headers[0].1.starts_with("Basic "));
        assert!(log[1].headers.is_empty());
    }

    #[test]
    fn a_page_that_is_not_a_catalogue_is_explained() {
        let f = MockFetcher::new().on(
            "https://a.example/",
            MockReply::Body(b"<html></html>".to_vec()),
        );
        let err = load(&f, &Catalog::default(), "https://a.example/").unwrap_err();
        assert!(format!("{err:#}").contains("not an OPDS catalogue"));
    }

    #[test]
    fn search_expands_the_template_and_follows_descriptions() {
        let f = MockFetcher::new().on(
            "https://cat.example/opds/root.xml",
            MockReply::Body(ROOT.as_bytes().to_vec()),
        );
        let root = load(&f, &cat(), "https://cat.example/opds/root.xml").unwrap();
        assert_eq!(
            search_url(&f, &cat(), &root, "war & peace").unwrap(),
            "https://cat.example/opds/search?q=war%20%26%20peace"
        );
        assert!(search_url(&f, &cat(), &root, "  ").is_err());

        let desc = r#"<OpenSearchDescription xmlns="http://a9.com/-/spec/opensearch/1.1/">
            <Url type="application/atom+xml" template="/s?query={searchTerms}"/></OpenSearchDescription>"#;
        let feed = Feed {
            links: vec![Link {
                rel: "search".into(),
                href: "https://cat.example/os.xml".into(),
                mime: "application/opensearchdescription+xml".into(),
                ..Link::default()
            }],
            ..Feed::default()
        };
        let f = MockFetcher::new().on(
            "https://cat.example/os.xml",
            MockReply::Body(desc.as_bytes().to_vec()),
        );
        assert_eq!(
            search_url(&f, &cat(), &feed, "dune").unwrap(),
            "https://cat.example/s?query=dune"
        );
        let none = Feed::default();
        assert!(search_url(&f, &cat(), &none, "x").is_err());
    }

    #[test]
    fn downloads_the_preferred_format_without_overwriting() {
        let f = MockFetcher::new()
            .on(
                "https://cat.example/opds/new.xml",
                MockReply::Body(BOOKS.as_bytes().to_vec()),
            )
            .on(
                "https://cat.example/dl/a.epub",
                MockReply::Body(b"PK-data".to_vec()),
            );
        let books = load(&f, &cat(), "https://cat.example/opds/new.xml").unwrap();
        let dir = tmp();
        let p1 = download(&f, &cat(), &books.entries[0], &dir).unwrap();
        let p2 = download(&f, &cat(), &books.entries[0], &dir).unwrap();
        assert_eq!(p1.file_name().unwrap(), "A_ Tale _ Part.epub");
        assert_eq!(p2.file_name().unwrap(), "A_ Tale _ Part (2).epub");
        assert_eq!(std::fs::read(p1).unwrap(), b"PK-data");
    }

    #[test]
    fn a_failed_download_is_an_error_and_leaves_no_file() {
        let f = MockFetcher::new().on(
            "https://cat.example/opds/new.xml",
            MockReply::Body(BOOKS.as_bytes().to_vec()),
        );
        let books = load(&f, &cat(), "https://cat.example/opds/new.xml").unwrap();
        let dir = tmp();
        assert!(download(&f, &cat(), &books.entries[0], &dir).is_err());
        assert!(!dir.exists() || std::fs::read_dir(&dir).unwrap().count() == 0);
        assert!(download(&f, &cat(), &Entry::default(), &dir).is_err());
    }

    #[test]
    fn extensions_come_from_the_type_then_the_address() {
        let l = |mime: &str, href: &str| Link {
            mime: mime.into(),
            href: href.into(),
            ..Link::default()
        };
        assert_eq!(
            extension(&l("application/epub+zip; charset=x", "https://a/b")),
            "epub"
        );
        assert_eq!(
            extension(&l("application/octet-stream", "https://a/b/c.AZW3?x=1")),
            "azw3"
        );
        assert_eq!(extension(&l("", "https://a/b")), "epub");
    }

    fn failure(c: &Catalog, reply: MockReply) -> Problem {
        let f = MockFetcher::new().on("https://cat.example/x", reply);
        explain(c, &load(&f, c, "https://cat.example/x").unwrap_err())
    }

    #[test]
    fn a_401_without_a_login_asks_to_sign_in_and_names_standard_ebooks() {
        let p = failure(&Catalog::default(), MockReply::Status(401));
        assert!(p.sign_in);
        assert!(!p.message.contains("HTTP error"), "{}", p.message);
        let se = Catalog {
            id: "standardebooks".into(),
            ..Catalog::default()
        };
        let p = failure(&se, MockReply::Status(401));
        assert!(p.sign_in);
        assert!(p.message.contains("Patrons Circle"), "{}", p.message);
    }

    #[test]
    fn a_refused_login_says_so_and_a_server_that_is_down_is_not_blamed_on_the_user() {
        let p = failure(&cat(), MockReply::Status(403));
        assert!(p.sign_in);
        assert!(p.message.contains("did not accept"), "{}", p.message);
        let p = failure(&cat(), MockReply::Status(503));
        assert!(!p.sign_in);
        assert!(p.message.contains("503") && p.message.contains("later"));
        assert!(!failure(&cat(), MockReply::Status(404)).sign_in);
        assert!(
            failure(&cat(), MockReply::Timeout)
                .message
                .contains("in time")
        );
        assert!(
            failure(&cat(), MockReply::Failure("dns".into()))
                .message
                .contains("dns")
        );
    }

    #[test]
    fn a_saved_login_reaches_the_always_listed_catalogue_and_is_sent_to_its_host_only() {
        let mut logins = Vec::new();
        set_login(&mut logins, "standardebooks", " me@example.org ", "");
        let listed = with_logins(presets(), &logins);
        let se = listed.iter().find(|c| c.id == "standardebooks").unwrap();
        assert_eq!(se.user, "me@example.org");
        assert_eq!(se.url, "https://standardebooks.org/feeds/opds");
        assert!(
            listed
                .iter()
                .find(|c| c.id == "gutenberg")
                .unwrap()
                .user
                .is_empty()
        );

        let f = MockFetcher::new().on(&se.url, MockReply::Body(ROOT.as_bytes().to_vec()));
        load(&f, se, &se.url).unwrap();
        assert!(f.requests()[0].headers[0].1.starts_with("Basic "));

        set_login(&mut logins, "standardebooks", "", "");
        assert!(logins.is_empty(), "an empty user name forgets the login");
    }

    #[test]
    fn the_real_gutenberg_front_page_parses() {
        // the start of https://www.gutenberg.org/ebooks.opds/ (August 2025), thumbnails left out
        let xml = include_str!("../crates/bookx-serve/tests/fixtures/gutenberg-root.opds");
        let f = MockFetcher::new().on(
            "https://www.gutenberg.org/ebooks.opds/",
            MockReply::Body(xml.as_bytes().to_vec()),
        );
        let g = presets().into_iter().find(|c| c.id == "gutenberg").unwrap();
        assert_eq!(g.url, "https://www.gutenberg.org/ebooks.opds/");
        let feed = load(&f, &g, &g.url).unwrap();
        assert_eq!(feed.entries.len(), 3);
        assert_eq!(
            feed.entries[0].navigation_link().unwrap().href,
            "https://www.gutenberg.org/ebooks/search.opds/?sort_order=downloads"
        );
        assert!(
            feed.search().is_some(),
            "search is an OpenSearch description"
        );
    }

    #[test]
    fn catalogues_are_checked_when_added() {
        let mut v = Vec::new();
        let id = add(
            &mut v,
            "My Calibre",
            "http://nas.local:8080/opds",
            "me",
            "pw",
        )
        .unwrap();
        assert_eq!(id, "my-calibre-1");
        assert!(add(&mut v, "Again", "http://nas.local:8080/opds", "", "").is_err());
        assert!(add(&mut v, "", "http://a", "", "").is_err());
        assert!(add(&mut v, "Ftp", "ftp://a/b", "", "").is_err());
        assert!(add(&mut v, "Junk", "not a url", "", "").is_err());
        assert!(add(&mut v, "project gutenberg", "http://x.example", "", "").is_err());
        let id2 = add(&mut v, "My Calibre!", "http://other.example", "", "").unwrap();
        assert_ne!(id, id2);
        assert_eq!(presets().len(), 2);
    }
}
