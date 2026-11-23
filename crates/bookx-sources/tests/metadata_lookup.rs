//! Metadata lookup end to end with mocked Open Library / Google Books / Wikidata responses.

use std::sync::Arc;

use bookx_core::archive::Archive;
use bookx_core::book::OpenBook;
use bookx_core::contracts::{MetadataQuery, MetadataSource};
use bookx_core::convert::{Chapter, create_epub};
use bookx_sources::cover::generate_cover;
use bookx_sources::fetch::{MockFetcher, MockReply};
use bookx_sources::sources::SourceSet;

const OL: &str = r#"{"numFound":1,"docs":[{"key":"/works/OL45804W","title":"Fantastic Mr Fox","author_name":["Roald Dahl"],
"first_publish_year":1970,"publisher":["Puffin"],"subject":["Foxes","Fiction"],"language":["eng"],"cover_i":6498519,"isbn":["0140328726"]}]}"#;

const GB: &str = r#"{"items":[{"id":"abcdefghijkl","volumeInfo":{"title":"Fantastic Mr Fox","authors":["Roald Dahl"],"publisher":"Penguin",
"publishedDate":"2007-04-05","description":"<p>A clever fox.</p>","categories":["Juvenile Fiction / Animals"],"language":"en",
"industryIdentifiers":[{"type":"ISBN_10","identifier":"0140328726"}],"imageLinks":{"thumbnail":"http://books.google.com/t.jpg&edge=curl"}}}]}"#;

fn query() -> MetadataQuery {
    MetadataQuery {
        title: "fantastic mr. fox".into(),
        authors: vec!["Dahl, Roald".into()],
        isbn: None,
    }
}

#[test]
fn lookup_merge_and_apply_to_a_book() {
    let cover = generate_cover("Fantastic Mr Fox", "Roald Dahl", "", 400, 600).unwrap();
    assert!(cover.len() > 1000);
    let f = MockFetcher::new()
        .on_prefix(
            "https://openlibrary.org/search.json",
            MockReply::Body(OL.into()),
        )
        .on_prefix(
            "https://www.googleapis.com/books/",
            MockReply::Body(GB.into()),
        )
        .on_prefix("https://query.wikidata.org/", MockReply::Status(429))
        .with_body("https://covers.openlibrary.org/b/id/6498519-L.jpg", &cover);
    let set = SourceSet::standard(Arc::new(f));
    let result = set.search(&query()).unwrap();

    assert_eq!(result.errors.len(), 1, "wikidata was rate limited");
    assert!(
        result.errors[0].message.contains("429"),
        "{:?}",
        result.errors
    );
    let best = result.best.expect("a confident match");
    assert!(best.confidence > 0.9, "{}", best.confidence);
    assert_eq!(best.sources, vec!["openlibrary", "google"]);
    assert_eq!(best.meta.title, "Fantastic Mr Fox");
    assert_eq!(best.meta.publisher, "Puffin");
    assert_eq!(best.meta.published, "1970");
    assert_eq!(best.meta.description, "A clever fox.");
    assert!(
        best.meta.tags.iter().any(|t| t == "Foxes")
            && best.meta.tags.iter().any(|t| t == "Animals")
    );
    assert_eq!(best.meta.identifier("isbn"), Some("0140328726"));

    // apply to a real EPUB: metadata and downloaded cover
    let chapters = [Chapter {
        title: "One".into(),
        body: "<p>text</p>".into(),
    }];
    let base = bookx_core::opf::Metadata {
        title: "fantastic mr. fox".into(),
        ..Default::default()
    };
    let archive = create_epub(&base, &chapters, None);
    let mut book =
        OpenBook::from_archive(Archive::from_bytes(archive.to_bytes().unwrap()).unwrap()).unwrap();
    let mut md = book.metadata().clone();
    md.title = best.meta.title.clone();
    md.authors = best.meta.authors.clone();
    md.tags = best.meta.tags.clone();
    md.description = best.meta.description.clone();
    md.identifiers = best.meta.identifiers.clone();
    book.set_metadata(md).unwrap();
    let bytes = set.fetch_cover(&best).unwrap().expect("cover downloaded");
    book.set_cover(bytes, "png").unwrap();
    assert!(
        book.cover_bytes()
            .is_some_and(|c| c.starts_with(b"\x89PNG"))
    );
    assert_eq!(book.metadata().authors, vec!["Roald Dahl"]);
    assert_eq!(book.metadata().identifier("isbn"), Some("0140328726"));
}

#[test]
fn every_source_failing_is_one_clear_error() {
    let f = MockFetcher::new()
        .on_prefix("https://openlibrary.org/", MockReply::Status(500))
        .on_prefix("https://www.googleapis.com/", MockReply::Timeout)
        .on_prefix(
            "https://query.wikidata.org/",
            MockReply::Body(b"<html>maintenance</html>".to_vec()),
        );
    let e = SourceSet::standard(Arc::new(f))
        .search(&query())
        .unwrap_err()
        .to_string();
    assert!(e.contains("openlibrary: HTTP error 500"), "{e}");
    assert!(e.contains("google: request timed out"), "{e}");
    assert!(
        e.contains("wikidata") && e.contains("not valid JSON"),
        "{e}"
    );
}

#[test]
fn trait_object_usage() {
    let f = MockFetcher::new()
        .on_prefix(
            "https://openlibrary.org/search.json",
            MockReply::Body(OL.into()),
        )
        .on_prefix("https://www.googleapis.com/", MockReply::Status(500))
        .on_prefix("https://query.wikidata.org/", MockReply::Status(500));
    let source: Box<dyn MetadataSource> = Box::new(SourceSet::standard(Arc::new(f)));
    let found = source.lookup(&query()).unwrap();
    assert_eq!(found[0].title, "Fantastic Mr Fox");
    assert_eq!(source.id(), "sources");
}

#[test]
fn isbn_lookup_through_open_library_editions() {
    let edition = r#"{"key":"/books/OL7353617M","title":"Fantastic Mr Fox","publishers":["Puffin"],"publish_date":"1988",
"isbn_13":["9780140328721"],"covers":[6498519],"authors":[{"key":"/authors/OL34184A"}]}"#;
    let f = MockFetcher::new()
        .with_body("https://openlibrary.org/isbn/9780140328721.json", edition)
        .with_body(
            "https://openlibrary.org/authors/OL34184A.json",
            r#"{"name":"Roald Dahl"}"#,
        )
        .on_prefix(
            "https://www.googleapis.com/",
            MockReply::Body(r#"{"totalItems":0}"#.into()),
        )
        .on_prefix(
            "https://query.wikidata.org/",
            MockReply::Body(r#"{"results":{"bindings":[]}}"#.into()),
        );
    let set = SourceSet::standard(Arc::new(f));
    let q = MetadataQuery {
        isbn: Some("978-0-14-032872-1".into()),
        ..Default::default()
    };
    let r = set.search(&q).unwrap();
    let best = r.best.unwrap();
    assert_eq!(best.confidence, 1.0);
    assert_eq!(best.meta.authors, vec!["Roald Dahl"]);
    assert_eq!(best.meta.published, "1988");
    assert!(best.cover_url.unwrap().ends_with("6498519-L.jpg"));
}
