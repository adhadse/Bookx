//! News recipes end to end: TOML recipe + mocked feeds/pages → EPUB, validated independently.

use std::collections::HashSet;
use std::io::Read;

use bookx_core::archive::Archive;
use bookx_core::book::OpenBook;
use bookx_sources::fetch::MockFetcher;
use bookx_sources::recipe::{build_news_epub, is_due, next_run, Recipe, Schedule};
use chrono::{TimeZone, Utc};

const RECIPE: &str = r#"
title = "Evening Brief"
oldest_days = 3
max_articles = 5
use_full_article = true
exclude = ["(?i)quiz"]
remove_selectors = [".related"]
title_template = "{title} - {date:%A %d %B %Y}"

[[feeds]]
url = "https://one.example/rss"
title = "One"

[[feeds]]
url = "https://two.example/atom.xml"

[schedule]
kind = "daily"
hour = 17
minute = 30
"#;

fn rss() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0"><channel><title>One Feed</title><link>https://one.example/</link>
<item><title>Big story</title><link>https://one.example/big</link><pubDate>Wed, 13 Mar 2024 15:00:00 GMT</pubDate><description>teaser</description></item>
<item><title>Daily quiz</title><link>https://one.example/quiz</link><pubDate>Wed, 13 Mar 2024 16:00:00 GMT</pubDate><description>quiz</description></item>
<item><title>Last week</title><link>https://one.example/old</link><pubDate>Mon, 04 Mar 2024 16:00:00 GMT</pubDate><description>old</description></item>
</channel></rss>"#
}

fn atom() -> &'static str {
    r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>Two Feed</title>
<entry><title>Second opinion</title><link href="https://two.example/op"/><id>1</id><updated>2024-03-13T10:00:00Z</updated><summary>Just the summary &amp; more</summary></entry>
</feed>"#
}

fn fetcher() -> MockFetcher {
    MockFetcher::new()
        .with_body("https://one.example/rss", rss())
        .with_body("https://two.example/atom.xml", atom())
        .with_body(
            "https://two.example/op",
            "<html><body><article><p>Second opinion in full, with commas, and many more words than the summary.</p></article></body></html>",
        )
        .with_body(
            "https://one.example/big",
            "<html><body><article><p>The big story in full, with commas, clauses, and length.</p><div class=\"related\"><p>Related links, many, many words, here.</p></div></article></body></html>",
        )
}

fn read(z: &mut zip::ZipArchive<std::io::Cursor<Vec<u8>>>, name: &str) -> String {
    let mut s = String::new();
    z.by_name(name)
        .unwrap_or_else(|_| panic!("{name}"))
        .read_to_string(&mut s)
        .unwrap();
    s
}

#[test]
fn recipe_to_nested_epub() {
    let recipe = Recipe::from_toml(RECIPE).unwrap();
    let now = Utc.with_ymd_and_hms(2024, 3, 14, 9, 0, 0).unwrap();
    let news = build_news_epub(&recipe, &fetcher(), now).unwrap();
    assert_eq!(news.title, "Evening Brief - Thursday 14 March 2024");
    assert_eq!(news.stats.articles, 2);
    assert_eq!(news.stats.full_articles, 2);
    assert_eq!(
        (news.stats.skipped_old, news.stats.skipped_filtered),
        (1, 1)
    );
    assert!(news.errors.is_empty(), "{:?}", news.errors);

    let bytes = news.archive.to_bytes().unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.clone())).unwrap();
    assert_eq!(z.by_index(0).unwrap().name().unwrap(), "mimetype");
    assert_eq!(
        z.by_index(0).unwrap().compression(),
        zip::CompressionMethod::Stored
    );

    // nested navigation, parsed with a separate XML parser
    let nav = read(&mut z, "OEBPS/nav.xhtml");
    let o = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let nav_doc = roxmltree::Document::parse_with_options(&nav, o).unwrap();
    let top_ol = nav_doc
        .descendants()
        .find(|n| n.tag_name().name() == "ol")
        .unwrap();
    let sections: Vec<_> = top_ol
        .children()
        .filter(|n| n.tag_name().name() == "li")
        .collect();
    assert_eq!(sections.len(), 2);
    let kid_titles: Vec<Vec<String>> = sections
        .iter()
        .map(|li| {
            li.descendants()
                .filter(|n| n.tag_name().name() == "ol")
                .flat_map(|ol| ol.children().filter(|c| c.tag_name().name() == "li"))
                .map(|c| {
                    c.descendants()
                        .filter(|d| d.is_text())
                        .filter_map(|d| d.text())
                        .collect::<String>()
                })
                .collect()
        })
        .collect();
    assert_eq!(
        kid_titles,
        vec![
            vec!["Big story".to_string()],
            vec!["Second opinion".to_string()]
        ]
    );

    // NCX depth and play order are consistent
    let ncx = read(&mut z, "OEBPS/toc.ncx");
    let ncx_doc = roxmltree::Document::parse(&ncx).unwrap();
    let orders: Vec<u32> = ncx_doc
        .descendants()
        .filter(|n| n.tag_name().name() == "navPoint")
        .map(|n| n.attribute("playOrder").unwrap().parse().unwrap())
        .collect();
    assert_eq!(orders, vec![1, 2, 3, 4]);
    let depth = ncx_doc
        .descendants()
        .find(|n| n.attribute("name") == Some("dtb:depth"))
        .unwrap()
        .attribute("content")
        .unwrap();
    assert_eq!(depth, "2");

    // every link in the nav resolves to a file
    let names: HashSet<String> = (0..z.len())
        .map(|i| z.by_index(i).unwrap().name().unwrap().to_string())
        .collect();
    for a in nav_doc.descendants().filter(|n| n.tag_name().name() == "a") {
        let href = a.attribute("href").unwrap();
        assert!(names.contains(&format!("OEBPS/{href}")), "{href}");
    }

    let book = OpenBook::from_archive(Archive::from_bytes(bytes).unwrap()).unwrap();
    let text: String = book
        .spine_paths()
        .iter()
        .filter_map(|p| book.chapter_source(p))
        .collect();
    assert!(text.contains("The big story in full"));
    assert!(!text.contains("Related links"));
    assert!(text.contains("Second opinion in full"));
    assert!(!text.contains("Daily quiz") && !text.contains("Last week"));
    assert!(book
        .cover_bytes()
        .is_some_and(|c| c.starts_with(b"\x89PNG")));
}

#[test]
fn schedule_from_the_recipe_drives_the_next_run() {
    let recipe = Recipe::from_toml(RECIPE).unwrap();
    let schedule = recipe.schedule.clone().unwrap();
    assert_eq!(
        schedule,
        Schedule::Daily {
            hour: 17,
            minute: 30
        }
    );
    let ran = Utc.with_ymd_and_hms(2024, 3, 13, 17, 31, 5).unwrap();
    let now = Utc.with_ymd_and_hms(2024, 3, 14, 9, 0, 0).unwrap();
    assert_eq!(
        next_run(&schedule, Some(ran), now).unwrap(),
        Utc.with_ymd_and_hms(2024, 3, 14, 17, 30, 0).unwrap()
    );
    assert!(!is_due(&schedule, Some(ran), now).unwrap());
    let evening = Utc.with_ymd_and_hms(2024, 3, 14, 17, 30, 0).unwrap();
    assert!(is_due(&schedule, Some(ran), evening).unwrap());
}

#[test]
fn bad_recipe_documents_give_helpful_errors() {
    let e =
        Recipe::from_toml("title = \"x\"\ninclude = 5\n[[feeds]]\nurl = \"https://a.example/f\"\n");
    assert!(e.is_err());
    let e = Recipe::from_toml(
        "title = \"x\"\ninclude = [\"(\"]\n[[feeds]]\nurl = \"https://a.example/f\"\n",
    )
    .unwrap_err();
    assert!(format!("{e:#}").contains("invalid regex"), "{e:#}");
}
