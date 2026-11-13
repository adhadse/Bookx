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
    let url = normalize_url(input)?;
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
}
