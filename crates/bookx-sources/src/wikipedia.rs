//! Wikipedia as a book (S8): one or more articles become the chapters of an EPUB.
//!
//! The text comes from the public MediaWiki API (`action=parse`), is cleaned with the same
//! whitelist as every web page (no scripts, no styles, no edit links) and carries its licence:
//! Wikipedia text is CC BY-SA 4.0, so the book names the source and the licence and every chapter
//! links its article. Only `*.wikipedia.org` is ever asked, whatever the user typed.

use std::sync::OnceLock;

use anyhow::{anyhow, bail, Context, Result};
use bookx_core::archive::Archive;
use bookx_core::convert::{create_epub, Chapter};
use bookx_core::opf::{Identifier, Metadata};
use bookx_core::util::xml_escape;
use regex::Regex;
use serde_json::Value;

use crate::fetch::{url_encode, Fetcher};
use crate::readability::clean_fragment;

/// Most articles in one book.
pub const MAX_TITLES: usize = 20;

pub const LICENCE: &str = "CC BY-SA 4.0 (https://creativecommons.org/licenses/by-sa/4.0/)";

/// What the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub lang: String,
    pub titles: Vec<String>,
}

fn lang_ok(l: &str) -> bool {
    (2..=12).contains(&l.len())
        && l.split('-').all(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
        && l.as_bytes()[0].is_ascii_lowercase()
}

/// `wikipedia:Title`, `wikipedia:de:Berlin|Hamburg` (several titles separated by `|`), or the
/// address of an article (`https://en.wikipedia.org/wiki/Rust_(programming_language)`). `Ok(None)`
/// when the input is neither.
pub fn parse_spec(input: &str) -> Result<Option<Spec>> {
    let t = input.trim();
    let lower = t.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("wikipedia:") {
        let rest = &t[t.len() - rest.len()..];
        let (lang, titles) = match rest.split_once(':') {
            Some((l, r)) if lang_ok(&l.to_ascii_lowercase()) && !l.is_empty() => {
                (l.to_ascii_lowercase(), r)
            }
            _ => ("en".to_string(), rest),
        };
        let titles: Vec<String> = titles
            .split('|')
            .map(|s| s.trim().replace('_', " "))
            .filter(|s| !s.is_empty())
            .collect();
        if titles.is_empty() {
            bail!("Name at least one Wikipedia article, such as wikipedia:Ada Lovelace");
        }
        if titles.len() > MAX_TITLES {
            bail!("At most {MAX_TITLES} articles fit in one book");
        }
        for s in &titles {
            if s.chars().count() > 200 || s.chars().any(char::is_control) {
                bail!("`{s}` is not an article title");
            }
        }
        return Ok(Some(Spec { lang, titles }));
    }
    if let Ok(u) = url::Url::parse(t) {
        let host = u.host_str().unwrap_or("").to_ascii_lowercase();
        if let Some(lang) = host.strip_suffix(".wikipedia.org") {
            if matches!(u.scheme(), "http" | "https") && lang_ok(lang) {
                if let Some(title) = u.path().strip_prefix("/wiki/") {
                    let title = percent_decode(title).replace('_', " ");
                    if !title.is_empty() && !title.contains(['\u{0}', '\n']) {
                        return Ok(Some(Spec {
                            lang: lang.to_string(),
                            titles: vec![title],
                        }));
                    }
                }
            }
        }
    }
    Ok(None)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The API address for one article.
pub fn api_url(lang: &str, title: &str) -> String {
    format!(
        "https://{lang}.wikipedia.org/w/api.php?action=parse&format=json&formatversion=2&prop=text&redirects=1&disableeditsection=1&disabletoc=1&page={}",
        url_encode(title)
    )
}

/// The article address shown in the book.
pub fn article_url(lang: &str, title: &str) -> String {
    format!(
        "https://{lang}.wikipedia.org/wiki/{}",
        url_encode(&title.replace(' ', "_"))
    )
}

/// `(title as Wikipedia spells it, HTML)` from the answer of the API.
pub fn parse_response(body: &[u8]) -> Result<(String, String)> {
    let v: Value = serde_json::from_slice(body).context("Wikipedia did not answer with JSON")?;
    if let Some(e) = v.pointer("/error/info").and_then(Value::as_str) {
        bail!("Wikipedia: {e}");
    }
    let title = v
        .pointer("/parse/title")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Wikipedia sent no article"))?;
    let html = v
        .pointer("/parse/text")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Wikipedia sent no text"))?;
    Ok((title.to_string(), html.to_string()))
}

fn noise() -> &'static [Regex; 4] {
    static R: OnceLock<[Regex; 4]> = OnceLock::new();
    R.get_or_init(|| {
        [
            Regex::new(r#"(?s)<sup[^>]*class="[^"]*reference[^"]*"[^>]*>.*?</sup>"#).expect("static"),
            Regex::new(r#"(?s)<(div|table)[^>]*class="[^"]*(navbox|metadata|noprint|hatnote|sidebar)[^"]*"[^>]*>.*?</(div|table)>"#).expect("static"),
            Regex::new(r#"(?s)<style.*?</style>"#).expect("static"),
            Regex::new(r#"(?s)<span[^>]*class="[^"]*mw-editsection[^"]*"[^>]*>.*?</span>"#).expect("static"),
        ]
    })
}

/// Cleaned XHTML of one article: reference marks, navigation boxes and edit links are gone.
pub fn clean_article(html: &str, lang: &str, title: &str) -> String {
    let mut h = html.to_string();
    for r in noise() {
        h = r.replace_all(&h, "").into_owned();
    }
    clean_fragment(&h, &article_url(lang, title)).content
}

/// Downloads the articles and builds the book. Articles that cannot be read are listed in the
/// second value; it is an error only when none could be read.
pub fn build_book(spec: &Spec, fetcher: &dyn Fetcher) -> Result<(Archive, Vec<String>)> {
    let mut chapters = Vec::new();
    let mut titles = Vec::new();
    let mut problems = Vec::new();
    for want in &spec.titles {
        let got = fetcher
            .get(&api_url(&spec.lang, want), &[])
            .and_then(|b| parse_response(&b));
        match got {
            Ok((title, html)) => {
                let content = clean_article(&html, &spec.lang, &title);
                if crate::text::strip_html(&content).trim().chars().count() < 20 {
                    problems.push(format!("{want}: no text"));
                    continue;
                }
                let url = article_url(&spec.lang, &title);
                let body = format!(
                    "<h1>{}</h1>\n{}<p class=\"source\"><a href=\"{}\">{}</a> · {}</p>\n",
                    xml_escape(&title),
                    content,
                    xml_escape(&url),
                    xml_escape(&url),
                    xml_escape("CC BY-SA 4.0")
                );
                chapters.push(Chapter {
                    title: title.clone(),
                    body,
                });
                titles.push(title);
            }
            Err(e) => problems.push(format!("{want}: {e:#}")),
        }
    }
    if chapters.is_empty() {
        bail!(
            "No article could be read from Wikipedia{}",
            problems
                .first()
                .map(|p| format!(" ({p})"))
                .unwrap_or_default()
        );
    }
    let title = if titles.len() == 1 {
        titles[0].clone()
    } else {
        format!("Wikipedia: {}", titles.join(", "))
    };
    let mut md = Metadata {
        title,
        authors: vec!["Wikipedia contributors".into()],
        publisher: "Wikipedia".into(),
        language: spec.lang.clone(),
        rights: format!("Text from Wikipedia, {LICENCE}"),
        tags: vec!["Wikipedia".into()],
        ..Default::default()
    };
    md.identifiers.push(Identifier {
        scheme: "url".into(),
        value: article_url(&spec.lang, &titles[0]),
        xml_id: None,
    });
    let mut archive = create_epub(&md, &chapters, None);
    crate::readability::write_full_metadata(&mut archive, &md)?;
    Ok((archive, problems))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::{MockFetcher, MockReply};

    fn answer(title: &str, html: &str) -> String {
        serde_json::json!({"parse": {"title": title, "pageid": 1, "text": html}}).to_string()
    }

    const HTML: &str = r##"<div class="mw-parser-output"><div class="hatnote">For other uses see X</div><p>Ada Lovelace was an English mathematician known for her work on the Analytical Engine.<sup class="reference"><a href="#cite_note-1">[1]</a></sup></p><h2>Life<span class="mw-editsection"><a href="/edit">edit</a></span></h2><p>She was born in London in 1815, the daughter of the poet Lord Byron.</p><script>alert(1)</script><style>.x{}</style></div>"##;

    #[test]
    fn specs_come_from_a_prefix_or_an_address() {
        let s = parse_spec("wikipedia:Ada Lovelace").unwrap().unwrap();
        assert_eq!(
            (s.lang.as_str(), s.titles.clone()),
            ("en", vec!["Ada Lovelace".to_string()])
        );
        let s = parse_spec(" Wikipedia:de:Berlin | Hamburg_Hafen ")
            .unwrap()
            .unwrap();
        assert_eq!(s.lang, "de");
        assert_eq!(s.titles, vec!["Berlin", "Hamburg Hafen"]);
        // a title with a colon that is not a language
        let s = parse_spec("wikipedia:Star Wars: A New Hope")
            .unwrap()
            .unwrap();
        assert_eq!(s.lang, "en");
        assert_eq!(s.titles, vec!["Star Wars: A New Hope"]);
        let s = parse_spec("https://fr.wikipedia.org/wiki/Caf%C3%A9_(boisson)")
            .unwrap()
            .unwrap();
        assert_eq!(
            (s.lang.as_str(), s.titles[0].as_str()),
            ("fr", "Café (boisson)")
        );
        // anything else is not for this module
        for other in [
            "https://example.org/wiki/X",
            "https://evilwikipedia.org/wiki/X",
            "https://wikipedia.org.evil.com/wiki/X",
            "example.com",
            "ftp://en.wikipedia.org/wiki/X",
        ] {
            assert_eq!(parse_spec(other).unwrap(), None, "{other}");
        }
        assert!(parse_spec("wikipedia:").is_err());
        assert!(parse_spec(&format!(
            "wikipedia:{}",
            vec!["a"; MAX_TITLES + 1].join("|")
        ))
        .is_err());
    }

    #[test]
    fn requests_only_go_to_wikipedia() {
        let u = api_url("en", "A&b=c#d");
        assert!(u.starts_with("https://en.wikipedia.org/w/api.php?"));
        assert!(u.ends_with("page=A%26b%3Dc%23d"), "{u}");
        // a hostile language code cannot change the host
        assert_eq!(
            parse_spec("wikipedia:evil.com/x:Title")
                .unwrap()
                .unwrap()
                .lang,
            "en"
        );
    }

    #[test]
    fn an_article_is_cleaned() {
        let c = clean_article(HTML, "en", "Ada Lovelace");
        assert!(
            c.contains("English mathematician") && c.contains("born in London"),
            "{c}"
        );
        for gone in [
            "[1]",
            "For other uses",
            "alert",
            "edit",
            "<style",
            "<script",
        ] {
            assert!(!c.contains(gone), "{gone} in {c}");
        }
    }

    #[test]
    fn articles_become_chapters_with_the_licence() {
        let f = MockFetcher::new()
            .with_body(&api_url("en", "Ada Lovelace"), answer("Ada Lovelace", HTML))
            .with_body(&api_url("en", "Missing"), r#"{"error":{"code":"missingtitle","info":"The page you specified doesn't exist."}}"#);
        let spec = Spec {
            lang: "en".into(),
            titles: vec!["Ada Lovelace".into(), "Missing".into()],
        };
        let (a, problems) = build_book(&spec, &f).unwrap();
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("doesn't exist"), "{problems:?}");
        let prepared =
            bookx_core::library::prepare_from_archive(a, std::path::Path::new("x")).unwrap();
        assert_eq!(prepared.meta.title, "Ada Lovelace");
        assert!(prepared.meta.rights.contains("CC BY-SA"));
        assert_eq!(prepared.meta.publisher, "Wikipedia");
        // nothing readable at all is an error
        let none = MockFetcher::new().on(&api_url("en", "Ada Lovelace"), MockReply::Status(500));
        let e = build_book(
            &Spec {
                lang: "en".into(),
                titles: vec!["Ada Lovelace".into()],
            },
            &none,
        )
        .unwrap_err();
        assert!(format!("{e:#}").contains("No article"), "{e:#}");
    }

    #[test]
    fn several_articles_make_one_titled_book() {
        let f = MockFetcher::new()
            .with_body(&api_url("en", "A"), answer("Alpha", HTML))
            .with_body(&api_url("en", "B"), answer("Beta", HTML));
        let (a, p) = build_book(
            &Spec {
                lang: "en".into(),
                titles: vec!["A".into(), "B".into()],
            },
            &f,
        )
        .unwrap();
        assert!(p.is_empty());
        let prepared =
            bookx_core::library::prepare_from_archive(a, std::path::Path::new("x")).unwrap();
        assert_eq!(prepared.meta.title, "Wikipedia: Alpha, Beta");
    }
}
