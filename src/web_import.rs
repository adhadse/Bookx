//! Adding things from the web to the library (roadmap S1, S4): a web page becomes a book (read it
//! later), an RSS/Atom/JSON feed becomes a news book. Built on `bookx-sources`; the network is
//! behind its `Fetcher` trait, so everything here is tested without it.

use crate::i18n::trf;
use std::path::Path;

use anyhow::{bail, Context, Result};
use bookx_sources::feed::{decode_text, detect};
use bookx_sources::fetch::Fetcher;
use bookx_sources::readability::{
    article_to_epub, extract_with, fetch_images, ExtractOptions, ImageLimits,
};
use bookx_sources::recipe::{build_news_epub, Recipe};
use chrono::{DateTime, Utc};

use crate::core::library::{prepare_from_archive, Prepared};

/// Fewer visible characters than this is not an article.
const MIN_ARTICLE_CHARS: usize = 200;

/// The fetcher: the real network, or canned pages from the folder in `$BOOKX_WEB_MOCK` for the UI
/// tests (`page.html` at `https://mock.example/post`, `feed.xml` at `https://mock.example/feed.xml`,
/// everything else answers 404).
pub fn fetcher() -> std::sync::Arc<dyn Fetcher> {
    use bookx_sources::fetch::{MockFetcher, MockReply, UreqFetcher};
    match std::env::var_os("BOOKX_WEB_MOCK") {
        Some(dir) => {
            let dir = std::path::PathBuf::from(dir);
            let file = |n: &str| {
                std::fs::read(dir.join(n))
                    .map(MockReply::Body)
                    .unwrap_or(MockReply::Status(404))
            };
            std::sync::Arc::new(
                MockFetcher::new()
                    .on("https://mock.example/post", file("page.html"))
                    .on("https://mock.example/feed.xml", file("feed.xml"))
                    .on(
                        "https://mock.example/wallabag/oauth/v2/token",
                        file("wallabag-token.json"),
                    )
                    .on_prefix(
                        "https://mock.example/wallabag/api/entries.json",
                        file("wallabag-entries.json"),
                    )
                    .on_prefix("https://mock.example/", MockReply::Status(404)),
            )
        }
        None => std::sync::Arc::new(UreqFetcher::new()),
    }
}

/// What the address turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Article,
    Feed,
    /// One or more Wikipedia articles as chapters of one book.
    Wikipedia,
}

/// The result of an import: the book to add, what it was, and a line for the user.
pub struct Imported {
    pub prepared: Prepared,
    pub kind: Kind,
    pub summary: String,
}

/// Cleans up what the user typed: `example.com/post` becomes `https://example.com/post`.
pub fn normalize_url(input: &str) -> Result<String> {
    let t = input.trim();
    if t.is_empty() {
        bail!("Enter the address of a web page or a feed.");
    }
    if t.contains(char::is_whitespace) {
        bail!("The address must not contain spaces.");
    }
    let with_scheme = if t.contains("://") {
        t.to_string()
    } else {
        format!("https://{t}")
    };
    let u =
        url::Url::parse(&with_scheme).with_context(|| format!("`{t}` is not a valid address"))?;
    if !matches!(u.scheme(), "http" | "https") {
        bail!("Only http and https addresses can be added.");
    }
    if u.host_str().is_none_or(|h| h.is_empty()) {
        bail!("`{t}` has no host name.");
    }
    Ok(u.to_string())
}

/// Downloads `input` and turns it into a book: a feed becomes a news book with its latest
/// articles, anything else is read as an article.
pub fn import(input: &str, fetcher: &dyn Fetcher, now: DateTime<Utc>) -> Result<Imported> {
    if let Some(spec) = bookx_sources::wikipedia::parse_spec(input)? {
        return import_wikipedia(&spec, fetcher);
    }
    let url = normalize_url(input)?;
    // the user typed this address, so its host may be local; links found inside it may not
    bookx_sources::netguard::trust_url(&url);
    let body = fetcher
        .get(&url, &[])
        .with_context(|| trf("Could not download {}", &[&url]))?;
    if body.is_empty() {
        bail!("{url} answered with an empty page.");
    }
    if detect(&body).is_some() {
        return import_feed(&url, fetcher, now);
    }
    import_article(&url, &decode_text(&body), fetcher)
}

fn import_wikipedia(
    spec: &bookx_sources::wikipedia::Spec,
    fetcher: &dyn Fetcher,
) -> Result<Imported> {
    let (archive, problems) = bookx_sources::wikipedia::build_book(spec, fetcher)?;
    let prepared = prepare_from_archive(archive, Path::new(&spec.titles[0]))?;
    let mut summary = trf("Made “{}” from Wikipedia", &[&prepared.meta.title]);
    if !problems.is_empty() {
        summary.push_str(&format!(", {} article(s) skipped", problems.len()));
    }
    Ok(Imported {
        prepared,
        kind: Kind::Wikipedia,
        summary,
    })
}

fn import_article(url: &str, html: &str, fetcher: &dyn Fetcher) -> Result<Imported> {
    // a page with almost no text is usually an error page or needs JavaScript to show anything
    let opts = ExtractOptions {
        min_text_len: MIN_ARTICLE_CHARS,
        ..ExtractOptions::default()
    };
    let article = extract_with(html, url, &opts).with_context(|| {
        trf(
            "Nothing readable found at {} (the page has too little text, or needs JavaScript)",
            &[&url],
        )
    })?;
    let mut wanted = article.images.clone();
    if let Some(l) = &article.lead_image {
        if !wanted.contains(l) {
            wanted.push(l.clone());
        }
    }
    let images = fetch_images(fetcher, &wanted, &ImageLimits::default());
    let archive = article_to_epub(&article, &images)?;
    let prepared = prepare_from_archive(archive, Path::new(&article.title))?;
    Ok(Imported {
        summary: trf(
            "Saved “{}” ({} images)",
            &[&prepared.meta.title, &images.len()],
        ),
        prepared,
        kind: Kind::Article,
    })
}

fn import_feed(url: &str, fetcher: &dyn Fetcher, now: DateTime<Utc>) -> Result<Imported> {
    let recipe = Recipe::new("News", &[url]);
    let news = build_news_epub(&recipe, fetcher, now)?;
    let prepared = prepare_from_archive(news.archive, Path::new(&news.title))?;
    let mut summary = trf(
        "Made “{}” from {} article(s)",
        &[&prepared.meta.title, &news.stats.articles],
    );
    if !news.errors.is_empty() {
        summary.push_str(&format!(", {} problem(s) skipped", news.errors.len()));
    }
    Ok(Imported {
        prepared,
        kind: Kind::Feed,
        summary,
    })
}

/// What a Wallabag import produced: the books with the Wallabag id each came from, and what failed.
pub struct WallabagRun {
    pub books: Vec<(u64, Prepared)>,
    pub problems: Vec<String>,
}

/// Imports the newest articles of the Wallabag server in `acc` that were not imported before.
/// The access token is the pasted one, or comes from signing in with the API client.
pub fn import_wallabag(
    acc: &crate::web_accounts::WebAccounts,
    fetcher: &dyn Fetcher,
) -> Result<WallabagRun> {
    use bookx_sources::wallabag as wb;
    let base = wb::normalize_base(&acc.wallabag_url)?;
    let token = match acc.wallabag_token.trim() {
        "" => wb::sign_in(fetcher, &base, &acc.wallabag_credentials())?,
        t => t.to_string(),
    };
    let seen: std::collections::HashSet<u64> = acc.wallabag_seen.iter().copied().collect();
    let entries = wb::fetch_entries(fetcher, &base, &token, &acc.wallabag_query(), &|id| {
        seen.contains(&id)
    })?;
    let mut run = WallabagRun {
        books: Vec::new(),
        problems: Vec::new(),
    };
    for e in entries {
        let article = wb::entry_article(&e);
        if article.text_len < 20 {
            run.problems.push(trf("“{}” has no text", &[&e.title]));
            continue;
        }
        let mut wanted = article.images.clone();
        if let Some(l) = &article.lead_image {
            if !wanted.contains(l) {
                wanted.push(l.clone());
            }
        }
        let images = fetch_images(fetcher, &wanted, &ImageLimits::default());
        let made = article_to_epub(&article, &images)
            .and_then(|a| prepare_from_archive(a, Path::new(&article.title)));
        match made {
            Ok(p) => run.books.push((e.id, p)),
            Err(err) => run.problems.push(format!("{}: {err:#}", e.title)),
        }
    }
    Ok(run)
}

/// One line for the user after an import.
pub fn wallabag_summary(books: usize, problems: usize) -> String {
    let mut s = crate::i18n::ntr(
        "Added {} article from Wallabag",
        "Added {} articles from Wallabag",
        books as u32,
    );
    if problems > 0 {
        s.push_str(&format!(", {problems} skipped"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_sources::fetch::{MockFetcher, MockReply};
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 12, 9, 0, 0).unwrap()
    }

    const PAGE: &str = r#"<html><head><title>A Fine Post | Example Blog</title>
        <meta property="og:title" content="A Fine Post"/><meta name="author" content="Pat Writer"/></head>
        <body><nav>Home About Contact</nav><article><h1>A Fine Post</h1>
        <p>This is the first paragraph of a reasonably long article about gardening and the many things one learns when tending vegetables through a whole year.</p>
        <p>The second paragraph continues the story with more words, so that the extraction has enough readable text to be convinced that this is the content.</p>
        <p>A third paragraph closes the post with a thought about patience and compost.</p></article>
        <footer>Copyright</footer></body></html>"#;

    const FEED: &str = r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Garden News</title>
        <link>https://news.example/</link>
        <item><title>Spring planting</title><link>https://news.example/spring</link>
          <description>&lt;p&gt;Plant the early peas now while the soil is still cool and moist enough.&lt;/p&gt;</description>
          <pubDate>Wed, 11 Nov 2026 08:00:00 GMT</pubDate></item>
        <item><title>Compost tips</title><link>https://news.example/compost</link>
          <description>&lt;p&gt;Turn the heap weekly and keep it damp but never soggy at all.&lt;/p&gt;</description>
          <pubDate>Tue, 10 Nov 2026 08:00:00 GMT</pubDate></item></channel></rss>"#;

    #[test]
    fn addresses_are_normalised_and_checked() {
        assert_eq!(
            normalize_url("example.com/post").unwrap(),
            "https://example.com/post"
        );
        assert_eq!(
            normalize_url("  http://a.org/x?y=1 ").unwrap(),
            "http://a.org/x?y=1"
        );
        assert_eq!(
            normalize_url("https://example.com").unwrap(),
            "https://example.com/"
        );
        for bad in [
            "",
            "   ",
            "two words.com",
            "ftp://x.org/f",
            "file:///etc/passwd",
            "https://",
            "javascript:alert(1)",
        ] {
            assert!(normalize_url(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn a_web_page_becomes_an_article_book() {
        let f = MockFetcher::new().with_body("https://blog.example/post", PAGE);
        let got = import("blog.example/post", &f, now()).unwrap();
        assert_eq!(got.kind, Kind::Article);
        assert_eq!(got.prepared.meta.title, "A Fine Post");
        assert!(got.summary.contains("A Fine Post"), "{}", got.summary);
        assert!(got.prepared.bytes.starts_with(b"PK"));
        // the book opens and holds the text but not the page furniture
        let book = crate::core::book::OpenBook::from_archive(
            crate::core::archive::Archive::from_bytes(got.prepared.bytes).unwrap(),
        )
        .unwrap();
        let text: String = book
            .spine_paths()
            .iter()
            .filter_map(|p| book.chapter_doc(p))
            .map(|d| d.to_text())
            .collect();
        assert!(text.contains("tending vegetables"), "{text}");
        assert!(
            !text.contains("Copyright") && !text.contains("Home About"),
            "{text}"
        );
    }

    #[test]
    fn wikipedia_articles_become_one_book_through_the_same_door() {
        use bookx_sources::wikipedia::api_url;
        let page = serde_json::json!({"parse": {"title": "Ada Lovelace", "text":
            "<p>Ada Lovelace was an English mathematician and writer, chiefly known for her work.</p>"}})
        .to_string();
        let f = MockFetcher::new().with_body(&api_url("en", "Ada Lovelace"), page);
        let got = import("wikipedia:Ada Lovelace", &f, now()).unwrap();
        assert_eq!(got.kind, Kind::Wikipedia);
        assert_eq!(got.prepared.meta.title, "Ada Lovelace");
        assert!(got.summary.contains("Ada Lovelace"), "{}", got.summary);
        assert!(f
            .urls()
            .iter()
            .all(|u| u.starts_with("https://en.wikipedia.org/")));
        // an article address of Wikipedia takes the same route
        let again = import("https://en.wikipedia.org/wiki/Ada_Lovelace", &f, now()).unwrap();
        assert_eq!(again.kind, Kind::Wikipedia);
    }

    #[test]
    fn a_feed_becomes_a_news_book() {
        let f = MockFetcher::new().with_body("https://news.example/feed.xml", FEED);
        let got = import("https://news.example/feed.xml", &f, now()).unwrap();
        assert_eq!(got.kind, Kind::Feed);
        assert!(got.summary.contains("2 article(s)"), "{}", got.summary);
        let book = crate::core::book::OpenBook::from_archive(
            crate::core::archive::Archive::from_bytes(got.prepared.bytes).unwrap(),
        )
        .unwrap();
        let text: String = book
            .spine_paths()
            .iter()
            .filter_map(|p| book.chapter_doc(p))
            .map(|d| d.to_text())
            .collect();
        assert!(
            text.contains("early peas") && text.contains("Turn the heap"),
            "{text}"
        );
    }

    #[test]
    fn downloads_that_fail_say_which_address() {
        let f = MockFetcher::new().on("https://down.example/x", MockReply::Status(500));
        let err = import("down.example/x", &f, now()).err().unwrap();
        assert!(format!("{err:#}").contains("down.example"), "{err:#}");
        let t = MockFetcher::new().on("https://slow.example/x", MockReply::Timeout);
        assert!(import("slow.example/x", &t, now()).is_err());
    }

    #[test]
    fn a_page_with_nothing_to_read_is_an_error_not_an_empty_book() {
        let f = MockFetcher::new().with_body(
            "https://empty.example/",
            "<html><body><nav>menu</nav><p>hi</p></body></html>",
        );
        let err = import("empty.example", &f, now()).err().unwrap();
        assert!(format!("{err:#}").contains("too little text"), "{err:#}");
        let g = MockFetcher::new().with_body("https://blank.example/", "");
        let err = import("blank.example", &g, now()).err().unwrap();
        assert!(format!("{err:#}").contains("empty"), "{err:#}");
    }

    #[test]
    fn a_feed_whose_articles_all_fail_is_an_error() {
        let f = MockFetcher::new().with_body(
            "https://bad.example/feed",
            r#"<rss version="2.0"><channel><title>Empty</title></channel></rss>"#,
        );
        assert!(import("https://bad.example/feed", &f, now()).is_err());
    }

    #[test]
    fn images_of_the_article_are_fetched_when_available_and_dropped_when_not() {
        let html = PAGE.replace(
            "<p>A third paragraph",
            "<p><img src=\"/pic.png\" alt=\"x\"/></p><p>A third paragraph",
        );
        let png = bookx_sources::cover::generate_cover("T", "A", "", 200, 300).unwrap();
        let with = MockFetcher::new()
            .with_body("https://blog.example/post", &html)
            .with_body("https://blog.example/pic.png", &png);
        let got = import("https://blog.example/post", &with, now()).unwrap();
        assert!(got.summary.contains("(1 images)"), "{}", got.summary);
        // the same page when the image cannot be downloaded
        let without = MockFetcher::new()
            .with_body("https://blog.example/post", &html)
            .on("https://blog.example/pic.png", MockReply::Status(404));
        let got = import("https://blog.example/post", &without, now()).unwrap();
        assert!(got.summary.contains("(0 images)"), "{}", got.summary);
    }

    fn wb_page(ids: &[u64]) -> String {
        let items: Vec<String> = ids
            .iter()
            .map(|id| {
                format!(
                    r#"{{"id":{id},"url":"https://blog.example/p{id}","title":"Saved {id}","content":"<p>{}</p>","language":"en","published_by":["Pat"],"created_at":"2024-05-01T10:00:00+0000","is_archived":0}}"#,
                    "A saved article with enough words to count as text. ".repeat(3)
                )
            })
            .collect();
        format!(
            r#"{{"pages":1,"_embedded":{{"items":[{}]}}}}"#,
            items.join(",")
        )
    }

    #[test]
    fn wallabag_articles_become_books_once() {
        use bookx_sources::wallabag::{entries_url, Query};
        let mut acc = crate::web_accounts::WebAccounts {
            wallabag_url: "wallabag.example.org".into(),
            wallabag_token: "TOK".into(),
            ..Default::default()
        };
        let q: Query = acc.wallabag_query();
        let f = MockFetcher::new().with_body(
            &entries_url("https://wallabag.example.org", &q, 1),
            wb_page(&[3, 2, 1]),
        );
        let run = import_wallabag(&acc, &f).unwrap();
        assert_eq!(run.books.len(), 3);
        assert_eq!(run.books[0].0, 3);
        assert_eq!(run.books[0].1.meta.title, "Saved 3");
        assert!(f.posts().is_empty(), "a pasted token needs no sign-in");
        // the second time only the new article is taken
        acc.remember_wallabag(&[3, 2, 1]);
        let f = MockFetcher::new().with_body(
            &entries_url("https://wallabag.example.org", &q, 1),
            wb_page(&[4, 3, 2, 1]),
        );
        let run = import_wallabag(&acc, &f).unwrap();
        assert_eq!(run.books.iter().map(|b| b.0).collect::<Vec<_>>(), vec![4]);
        assert!(wallabag_summary(1, 0).contains('1'));
    }

    #[test]
    fn wallabag_signs_in_when_there_is_no_token() {
        use bookx_sources::wallabag::{entries_url, Query};
        let mut acc = crate::web_accounts::WebAccounts {
            wallabag_url: "https://w.example.org".into(),
            ..Default::default()
        };
        assert!(import_wallabag(&acc, &MockFetcher::new()).is_err());
        acc.wallabag_client_id = "i".into();
        acc.wallabag_client_secret = "s".into();
        acc.wallabag_user = "u".into();
        acc.wallabag_password = "p".into();
        let f = MockFetcher::new()
            .with_body(
                "https://w.example.org/oauth/v2/token",
                r#"{"access_token":"T2"}"#,
            )
            .with_body(
                &entries_url("https://w.example.org", &Query::default(), 1),
                wb_page(&[9]),
            );
        let run = import_wallabag(&acc, &f).unwrap();
        assert_eq!(run.books.len(), 1);
        assert_eq!(f.posts().len(), 1);
        assert_eq!(
            f.requests()[0].header_value("authorization"),
            Some("Bearer T2")
        );
    }
}
