//! Fetching book metadata from the internet (roadmap L6/S6) for the *Edit metadata* dialog, on
//! top of `bookx-sources`. No GTK in here, so it is unit tested; the network is behind the
//! `Fetcher` trait, and `$BOOKX_SOURCES_MOCK` swaps in canned answers for the UI tests.

use crate::i18n::trf;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use bookx_core::contracts::MetadataQuery;
use bookx_sources::cache::CachingFetcher;
use bookx_sources::fetch::{Fetcher, MockFetcher, MockReply, UreqFetcher};
use bookx_sources::health::{HealthBoard, HealthFetcher};
use bookx_sources::sources::SourceSet;

use crate::core::opf::{Identifier, Metadata};
use crate::core::util::title_sort;

/// What to do with values the book already has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Merge {
    /// Only fill in what is empty.
    FillEmpty,
    /// Take every value the sources found.
    Replace,
}

/// The query for a book: title, authors and an ISBN if one is recorded.
pub fn query_from(md: &Metadata) -> MetadataQuery {
    let isbn = md
        .identifiers
        .iter()
        .filter(|i| i.scheme.eq_ignore_ascii_case("isbn"))
        .find_map(|i| bookx_sources::isbn::canonical(&i.value));
    MetadataQuery {
        title: md.title.trim().to_string(),
        authors: md.authors.clone(),
        isbn,
    }
}

/// `current` with the values of `found` merged in.
pub fn apply(current: &Metadata, found: &Metadata, mode: Merge) -> Metadata {
    let mut m = current.clone();
    let take = |cur: &mut String, new: &str| {
        let new = new.trim();
        if !new.is_empty() && (mode == Merge::Replace || cur.trim().is_empty()) {
            *cur = new.to_string();
        }
    };
    let old_title = m.title.clone();
    take(&mut m.title, &found.title);
    take(&mut m.publisher, &found.publisher);
    take(&mut m.language, &found.language);
    take(&mut m.description, &found.description);
    take(&mut m.published, &found.published);
    if !found.authors.is_empty() && (mode == Merge::Replace || m.authors.is_empty()) {
        m.authors = found.authors.clone();
    }
    if !found.tags.is_empty() && (mode == Merge::Replace || m.tags.is_empty()) {
        m.tags = found.tags.clone();
    }
    if !found.series.trim().is_empty() && (mode == Merge::Replace || m.series.trim().is_empty()) {
        m.series = found.series.clone();
        m.series_index = found.series_index;
    }
    // identifiers are always added (a new scheme never hurts); Replace also updates known ones
    for id in &found.identifiers {
        match m
            .identifiers
            .iter_mut()
            .find(|i| i.scheme.eq_ignore_ascii_case(&id.scheme))
        {
            Some(cur) => {
                if mode == Merge::Replace {
                    cur.value = id.value.clone();
                }
            }
            None => m.identifiers.push(Identifier {
                scheme: id.scheme.clone(),
                value: id.value.clone(),
                xml_id: None,
            }),
        }
    }
    if m.title != old_title || m.title_sort.trim().is_empty() {
        m.title_sort = title_sort(&m.title);
    }
    if m.author_sort.trim().is_empty() || m.authors != current.authors {
        m.author_sort = crate::source_priority::author_sort_for(
            &m.authors,
            &m.language,
            crate::source_priority::current_name_order(),
        );
    }
    m
}

/// Names of the fields that differ, for the status line.
pub fn changed_fields(before: &Metadata, after: &Metadata) -> Vec<&'static str> {
    let mut v = Vec::new();
    let mut check = |name: &'static str, same: bool| {
        if !same {
            v.push(name);
        }
    };
    check("title", before.title == after.title);
    check("authors", before.authors == after.authors);
    check(
        "series",
        before.series == after.series && before.series_index == after.series_index,
    );
    check("publisher", before.publisher == after.publisher);
    check("published", before.published == after.published);
    check("language", before.language == after.language);
    check("tags", before.tags == after.tags);
    check("identifiers", before.identifiers == after.identifiers);
    check("description", before.description == after.description);
    check("rating", before.rating == after.rating);
    v
}

/// The answer of a lookup.
#[derive(Debug, Clone)]
pub struct Fetched {
    pub meta: Metadata,
    pub cover: Option<Vec<u8>>,
    pub confidence: f64,
    pub sources: Vec<String>,
    /// Which source gave which field (`publisher` -> `openlibrary`); fields not listed come from
    /// all of `sources`.
    pub field_sources: std::collections::BTreeMap<String, String>,
    /// Sources that failed while others answered.
    pub warnings: Vec<String>,
}

/// Looks the book up. `Ok(None)`: nothing matched well enough. The enabled plugins are asked too
/// and fill in what the built-in sources left empty.
pub fn fetch(md: &Metadata, fetcher: Arc<dyn Fetcher>) -> Result<Option<Fetched>> {
    fetch_with(md, fetcher, &crate::plugins::current())
}

/// [`fetch`] with an explicit set of plugins.
pub fn fetch_with(
    md: &Metadata,
    fetcher: Arc<dyn Fetcher>,
    plugins: &crate::plugins::Loaded,
) -> Result<Option<Fetched>> {
    let q = query_from(md);
    if q.title.is_empty() && q.isbn.is_none() {
        anyhow::bail!("Enter a title or an ISBN first.");
    }
    let set = SourceSet::standard(fetcher).with_priority(crate::source_priority::current());
    let built_in = set.search(&q);
    let hits = crate::plugins::lookup_all(plugins, &q);
    let mut found = match built_in {
        Ok(res) => {
            let warnings: Vec<String> = res
                .errors
                .iter()
                .map(|e| format!("{}: {}", e.source, e.message))
                .collect();
            res.best.map(|best| {
                // a missing or broken cover must not lose the metadata
                let cover = set.fetch_cover(&best).ok().flatten();
                Fetched {
                    meta: best.meta,
                    cover,
                    confidence: best.confidence,
                    sources: best.sources,
                    field_sources: best.field_sources,
                    warnings,
                }
            })
        }
        // every built-in source failed: only an error when no plugin has an answer either
        Err(e) if hits.is_empty() => return Err(e),
        Err(e) => {
            let _ = e;
            None
        }
    };
    for (id, hit) in hits {
        match &mut found {
            Some(f) => {
                let merged = apply(&f.meta, &hit, Merge::FillEmpty);
                for field in changed_fields(&f.meta, &merged) {
                    f.field_sources
                        .insert(field.to_string(), format!("plugin {id}"));
                }
                f.meta = merged;
                f.sources.push(format!("plugin {id}"));
            }
            None => {
                found = Some(Fetched {
                    meta: apply(md, &hit, Merge::FillEmpty),
                    cover: None,
                    confidence: 0.5,
                    sources: vec![format!("plugin {id}")],
                    field_sources: Default::default(),
                    warnings: Vec::new(),
                });
            }
        }
    }
    Ok(found)
}

/// The fetcher: the real network, or canned answers from the folder in `$BOOKX_SOURCES_MOCK`
/// (`openlibrary.json`, `google.json`, `cover.jpg`; anything missing answers 404; an
/// `openlibrary.status` or `google.status` file with a number makes that source answer with it).
///
/// Every request goes through the health board ([`health_board`]), so a source that keeps failing
/// is left alone for a while and the review window can show how each one is doing. Answers are
/// kept on disk for a short time ([`cache_dir`]) so a repeated lookup does not hit the network;
/// the UI tests that use canned answers skip the cache unless `$BOOKX_SOURCES_CACHE=1`.
pub fn fetcher() -> Arc<dyn Fetcher> {
    cached().clone()
}

fn cached() -> &'static Arc<CachingFetcher> {
    static F: std::sync::OnceLock<Arc<CachingFetcher>> = std::sync::OnceLock::new();
    F.get_or_init(|| {
        let raw: Arc<dyn Fetcher> = match std::env::var_os("BOOKX_SOURCES_MOCK") {
            Some(dir) => Arc::new(mock_from(Path::new(&dir))),
            None => Arc::new(UreqFetcher::new()),
        };
        let healthy: Arc<dyn Fetcher> = Arc::new(HealthFetcher::new(raw, health_board()));
        let dir = cache_dir();
        if !cache_enabled() {
            return Arc::new(CachingFetcher::disabled(healthy));
        }
        let c = CachingFetcher::new(healthy, &dir.join("responses"));
        c.trim();
        Arc::new(c)
    })
}

fn cache_enabled() -> bool {
    match std::env::var("BOOKX_SOURCES_CACHE").as_deref() {
        Ok("0") => false,
        Ok("1") => true,
        _ => std::env::var_os("BOOKX_SOURCES_MOCK").is_none(),
    }
}

/// Where the source cache and the saved source health live.
pub fn cache_dir() -> std::path::PathBuf {
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.cache_dir().join("sources"))
        .unwrap_or_else(|| std::env::temp_dir().join("bookx-sources"))
}

/// Requests answered from the cache and sent out since the app started.
pub fn cache_stats() -> bookx_sources::cache::CacheStats {
    cached().stats()
}

/// Deletes every cached response. Returns how many there were.
pub fn clear_cache() -> usize {
    cached().clear()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

const HEALTH_FILE: &str = "source-health.json";

/// Reads the health saved by an earlier session into `board`. Returns the number of sources.
pub fn load_health(board: &HealthBoard, dir: &Path, now: u64) -> usize {
    std::fs::read_to_string(dir.join(HEALTH_FILE))
        .map(|t| board.load_json(&t, now))
        .unwrap_or(0)
}

/// Saves the health of `board` for the next session (best effort).
pub fn save_health(board: &HealthBoard, dir: &Path, now: u64) {
    if std::fs::create_dir_all(dir).is_ok() {
        let _ = std::fs::write(dir.join(HEALTH_FILE), board.to_json(now));
    }
}

/// Saves the health of the global board; call after a lookup or a batch.
pub fn persist_health() {
    save_health(&health_board(), &cache_dir(), unix_now());
}

/// How the metadata sources have been doing, this session and the earlier ones.
pub fn health_board() -> Arc<HealthBoard> {
    static BOARD: std::sync::OnceLock<Arc<HealthBoard>> = std::sync::OnceLock::new();
    BOARD
        .get_or_init(|| {
            let b = Arc::new(HealthBoard::new());
            load_health(&b, &cache_dir(), unix_now());
            b
        })
        .clone()
}

pub fn mock_from(dir: &Path) -> MockFetcher {
    let file = |name: &str| {
        std::fs::read(dir.join(name))
            .map(MockReply::Body)
            .unwrap_or(MockReply::Status(404))
    };
    // `<name>.status` holding a number makes that source answer with the status (rate limits and
    // outages in the UI tests)
    let answer = |source: &str, body: &str| {
        std::fs::read_to_string(dir.join(format!("{source}.status")))
            .ok()
            .and_then(|t| t.trim().parse::<u16>().ok())
            .map(MockReply::Status)
            .unwrap_or_else(|| file(body))
    };
    MockFetcher::new()
        .on_prefix(
            "https://openlibrary.org/",
            answer("openlibrary", "openlibrary.json"),
        )
        .on_prefix("https://covers.openlibrary.org/", file("cover.jpg"))
        .on_prefix(
            "https://www.googleapis.com/",
            answer("google", "google.json"),
        )
        .on_prefix("https://query.wikidata.org/", MockReply::Status(404))
        .on_prefix("https://www.wikidata.org/", MockReply::Status(404))
}

/// One-line summary of a lookup for the status label.
pub fn summary(f: &Fetched, changed: &[&str]) -> String {
    let who = f.sources.join(", ");
    let pct = (f.confidence * 100.0).round() as u32;
    if changed.is_empty() {
        trf(
            "Found on {} ({}% match), but nothing new to add.",
            &[&who, &pct],
        )
    } else {
        trf(
            "Found on {} ({}% match). Updated: {}.",
            &[&who, &pct, &changed.join(", ")],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md() -> Metadata {
        Metadata {
            title: "Dune".into(),
            authors: vec!["Frank Herbert".into()],
            ..Default::default()
        }
    }

    fn found() -> Metadata {
        Metadata {
            title: "Dune".into(),
            authors: vec!["Frank Herbert".into()],
            publisher: "Chilton Books".into(),
            published: "1965-08-01".into(),
            language: "en".into(),
            description: "A desert planet.".into(),
            tags: vec!["science fiction".into()],
            series: "Dune Chronicles".into(),
            series_index: 1.0,
            identifiers: vec![Identifier {
                scheme: "isbn".into(),
                value: "9780441172719".into(),
                xml_id: None,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn query_uses_title_authors_and_a_valid_isbn() {
        let mut m = md();
        m.identifiers.push(Identifier {
            scheme: "isbn".into(),
            value: "978-0-441-17271-9".into(),
            xml_id: None,
        });
        let q = query_from(&m);
        assert_eq!(q.title, "Dune");
        assert_eq!(q.authors, vec!["Frank Herbert"]);
        assert_eq!(q.isbn.as_deref(), Some("9780441172719"));
        // an invalid ISBN is not sent
        m.identifiers[0].value = "123".into();
        assert_eq!(query_from(&m).isbn, None);
        // other schemes are ignored
        m.identifiers[0] = Identifier {
            scheme: "doi".into(),
            value: "10.1/x".into(),
            xml_id: None,
        };
        assert_eq!(query_from(&m).isbn, None);
    }

    #[test]
    fn fill_empty_keeps_what_the_book_has() {
        let mut cur = md();
        cur.publisher = "My Publisher".into();
        let out = apply(&cur, &found(), Merge::FillEmpty);
        assert_eq!(out.publisher, "My Publisher", "kept");
        assert_eq!(out.published, "1965-08-01");
        assert_eq!(out.language, "en");
        assert_eq!(out.series, "Dune Chronicles");
        assert_eq!(out.series_index, 1.0);
        assert_eq!(out.tags, vec!["science fiction"]);
        assert_eq!(out.identifiers.len(), 1);
        assert_eq!(
            changed_fields(&cur, &out),
            vec![
                "series",
                "published",
                "language",
                "tags",
                "identifiers",
                "description"
            ]
        );
    }

    #[test]
    fn replace_takes_the_found_values() {
        let mut cur = md();
        cur.publisher = "My Publisher".into();
        cur.series = "Old".into();
        cur.series_index = 9.0;
        let out = apply(&cur, &found(), Merge::Replace);
        assert_eq!(out.publisher, "Chilton Books");
        assert_eq!(out.series, "Dune Chronicles");
        assert_eq!(out.series_index, 1.0);
    }

    #[test]
    fn empty_found_values_never_erase() {
        let mut cur = md();
        cur.publisher = "Keep".into();
        cur.tags = vec!["mine".into()];
        let nothing = Metadata::default();
        for mode in [Merge::FillEmpty, Merge::Replace] {
            let out = apply(&cur, &nothing, mode);
            assert_eq!(out.publisher, "Keep");
            assert_eq!(out.tags, vec!["mine"]);
            assert_eq!(out.title, "Dune");
            assert_eq!(out.authors, vec!["Frank Herbert"]);
        }
    }

    #[test]
    fn identifiers_are_added_and_only_replaced_in_replace_mode() {
        let mut cur = md();
        cur.identifiers.push(Identifier {
            scheme: "isbn".into(),
            value: "OLD".into(),
            xml_id: None,
        });
        cur.identifiers.push(Identifier {
            scheme: "doi".into(),
            value: "10.1/x".into(),
            xml_id: None,
        });
        let fill = apply(&cur, &found(), Merge::FillEmpty);
        assert_eq!(fill.identifier("isbn"), Some("OLD"));
        assert_eq!(fill.identifier("doi"), Some("10.1/x"));
        let mut f2 = found();
        f2.identifiers.push(Identifier {
            scheme: "goodreads".into(),
            value: "234225".into(),
            xml_id: None,
        });
        let rep = apply(&cur, &f2, Merge::Replace);
        assert_eq!(rep.identifier("isbn"), Some("9780441172719"));
        assert_eq!(rep.identifier("goodreads"), Some("234225"));
        assert_eq!(rep.identifier("doi"), Some("10.1/x"));
        assert_eq!(fill.identifier("goodreads"), None);
    }

    #[test]
    fn sort_keys_follow_a_changed_title_or_author() {
        let mut cur = md();
        cur.title_sort = "Dune".into();
        cur.author_sort = "Herbert, Frank".into();
        let mut f = found();
        f.title = "The Dune Book".into();
        f.authors = vec!["Ann Writer".into()];
        let out = apply(&cur, &f, Merge::Replace);
        assert_eq!(out.title_sort, "Dune Book, The");
        assert_eq!(out.author_sort, "Writer, Ann");
        // unchanged title and authors keep their sort keys
        let same = apply(&cur, &found(), Merge::FillEmpty);
        assert_eq!(same.title_sort, "Dune");
        assert_eq!(same.author_sort, "Herbert, Frank");
    }

    #[test]
    fn the_summary_names_sources_confidence_and_changes() {
        let f = Fetched {
            meta: found(),
            cover: None,
            confidence: 0.874,
            sources: vec!["openlibrary".into(), "googlebooks".into()],
            field_sources: Default::default(),
            warnings: vec![],
        };
        let s = summary(&f, &["series", "tags"]);
        assert!(
            s.contains("openlibrary, googlebooks") && s.contains("87%"),
            "{s}"
        );
        assert!(s.contains("series, tags"));
        assert!(summary(&f, &[]).contains("nothing new"));
    }

    fn tmp() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "bookx-mockmeta-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const OL: &str = r#"{"numFound":1,"docs":[{"key":"/works/OL893415W","title":"Dune","author_name":["Frank Herbert"],"first_publish_year":1965,"publisher":["Chilton Books"],"isbn":["9780441172719"],"cover_i":8231856,"subject":["Science fiction"],"language":["eng"]}]}"#;

    #[test]
    fn a_canned_open_library_answer_fills_the_book() {
        let d = tmp();
        std::fs::write(d.join("openlibrary.json"), OL).unwrap();
        std::fs::write(d.join("cover.jpg"), [0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3]).unwrap();
        let f = fetch(&md(), Arc::new(mock_from(&d))).unwrap().unwrap();
        assert_eq!(f.meta.title, "Dune");
        assert!(
            f.sources.iter().any(|s| s.contains("open")),
            "{:?}",
            f.sources
        );
        assert!(f.confidence > 0.5);
        let merged = apply(&md(), &f.meta, Merge::FillEmpty);
        assert_eq!(merged.publisher, "Chilton Books");
        assert_eq!(merged.identifier("isbn"), Some("9780441172719"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn nothing_found_and_all_failing_are_different() {
        let d = tmp();
        // every source answers 404: all failed
        assert!(fetch(&md(), Arc::new(mock_from(&d))).is_err());
        // a valid but empty answer: no match
        std::fs::write(d.join("openlibrary.json"), r#"{"numFound":0,"docs":[]}"#).unwrap();
        assert!(fetch(&md(), Arc::new(mock_from(&d))).unwrap().is_none());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn an_empty_query_is_refused_before_any_request() {
        let blank = Metadata::default();
        let err = fetch(&blank, Arc::new(MockFetcher::new())).unwrap_err();
        assert!(format!("{err}").contains("title or an ISBN"));
    }

    #[test]
    fn a_missing_cover_does_not_lose_the_metadata() {
        let d = tmp();
        std::fs::write(d.join("openlibrary.json"), OL).unwrap();
        let f = fetch(&md(), Arc::new(mock_from(&d))).unwrap().unwrap();
        assert!(f.cover.is_none());
        assert_eq!(f.meta.title, "Dune");
        let _ = std::fs::remove_dir_all(d);
    }

    fn plugin_loaded(reply: &str) -> crate::plugins::Loaded {
        // a plugin source built from WAT, registered the way the app does it
        let packed = ((1024i64) << 32) | reply.len() as i64;
        let esc: String = reply.bytes().map(|x| format!("\\{x:02x}")).collect();
        let wasm = wat::parse_str(format!(
            r#"(module (memory (export "memory") 2) (global $h (mut i32) (i32.const 4096))
              (func (export "alloc") (param $n i32) (result i32) (local $p i32)
                (local.set $p (global.get $h))
                (global.set $h (i32.add (local.get $p) (i32.and (i32.add (local.get $n) (i32.const 7)) (i32.const -8))))
                (local.get $p))
              (func (export "dealloc") (param i32 i32))
              (data (i32.const 1024) "{esc}")
              (func (export "bookx_call") (param $op i32) (param $ol i32) (param $ip i32) (param $il i32) (result i64)
                (if (i32.eq (i32.load8_u (local.get $op)) (i32.const 108)) (then (return (i64.const {packed}))))
                (i64.const -2)))"#
        ))
        .unwrap();
        let dir = std::env::temp_dir().join(format!("bookx-mf-plug-{}", uuid::Uuid::new_v4()));
        let p = dir.join("p");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            p.join("plugin.toml"),
            "id = \"p.source\"\nname = \"P\"\nversion = \"1.0.0\"\napi_version = 1\nkind = \"metadata-source\"\n",
        )
        .unwrap();
        std::fs::write(p.join("plugin.wasm"), wasm).unwrap();
        let mut m = bookx_plugin::PluginManager::new(
            &dir,
            dir.join("s.json"),
            bookx_plugin::Host::default(),
        );
        m.scan();
        m.set_enabled("p.source", true).unwrap();
        let (loaded, errs) = crate::plugins::build(&mut m);
        assert!(errs.is_empty(), "{errs:?}");
        let _ = std::fs::remove_dir_all(dir);
        loaded
    }

    #[test]
    fn a_plugin_fills_what_the_builtin_sources_left_empty() {
        let d = tmp();
        std::fs::write(d.join("openlibrary.json"), OL).unwrap();
        let plugin = plugin_loaded(
            r#"{"ok":[{"publisher":"Ignored Press","series":"Plugin Saga","tags":["plugin tag"]}]}"#,
        );
        let f = fetch_with(&md(), Arc::new(mock_from(&d)), &plugin)
            .unwrap()
            .unwrap();
        assert_eq!(f.meta.publisher, "Chilton Books", "the built-in value wins");
        assert_eq!(f.meta.series, "Plugin Saga", "the plugin fills the gap");
        assert!(
            f.sources.iter().any(|s| s == "plugin p.source"),
            "{:?}",
            f.sources
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_plugin_alone_is_enough_when_every_builtin_source_fails() {
        let d = tmp(); // nothing in it: every built-in source answers 404
        let plugin = plugin_loaded(r#"{"ok":[{"publisher":"Only Plugin","language":"fr"}]}"#);
        let f = fetch_with(&md(), Arc::new(mock_from(&d)), &plugin)
            .unwrap()
            .unwrap();
        assert_eq!(f.meta.publisher, "Only Plugin");
        assert_eq!(f.meta.title, "Dune", "the book's own values stay");
        assert_eq!(f.sources, vec!["plugin p.source"]);
        // without the plugin the same situation is an error
        assert!(fetch_with(
            &md(),
            Arc::new(mock_from(&d)),
            &crate::plugins::Loaded::default()
        )
        .is_err());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_plugin_that_knows_nothing_changes_nothing() {
        let d = tmp();
        std::fs::write(d.join("openlibrary.json"), OL).unwrap();
        let plugin = plugin_loaded(r#"{"ok":[]}"#);
        let with = fetch_with(&md(), Arc::new(mock_from(&d)), &plugin)
            .unwrap()
            .unwrap();
        let without = fetch_with(
            &md(),
            Arc::new(mock_from(&d)),
            &crate::plugins::Loaded::default(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(with.meta, without.meta);
        assert_eq!(with.sources, without.sources);
        let _ = std::fs::remove_dir_all(d);
    }
}
