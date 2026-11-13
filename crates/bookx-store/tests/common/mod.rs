#![allow(dead_code)]

use std::path::{Path, PathBuf};

use bookx_core::library::{Book, Bookmark, Highlight, ReadPos};
use bookx_core::opf::{Identifier, Metadata};
use chrono::{DateTime, TimeZone, Utc};

/// Unique temp directory removed on drop.
pub struct Tmp(pub PathBuf);

impl Tmp {
    pub fn new() -> Tmp {
        let p = std::env::temp_dir().join(format!("bookx-store-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
    pub fn join(&self, s: &str) -> PathBuf {
        self.0.join(s)
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn ts(secs: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(secs, 0).unwrap()
}

pub fn book(id: u64, title: &str, author: &str) -> Book {
    Book {
        id,
        file: format!("{author}/{title} ({id})/{title}.epub"),
        cover: None,
        meta: Metadata {
            title: title.into(),
            title_sort: title.into(),
            authors: vec![author.into()],
            author_sort: author.into(),
            ..Default::default()
        },
        added: ts(1_700_000_000 + id as i64),
        modified: ts(1_700_000_000 + id as i64),
        last_read: None,
        size: 1000,
        progress: 0.0,
        position: None,
        bookmarks: vec![],
        highlights: vec![],
        formats: Vec::new(),
        resources: Vec::new(),
        drm: String::new(),
    }
}

/// A book with every field set to something non-default.
pub fn full_book(id: u64) -> Book {
    let mut b = book(
        id,
        "Der Herr der Ringe: Die Gefährten ☃",
        "J. R. R. Tolkien",
    );
    b.cover = Some("cover.jpg".into());
    b.meta.authors.push("Christopher Tolkien".into());
    b.meta.publisher = "Klett-Cotta".into();
    b.meta.language = "de".into();
    b.meta.description = "<p>Ein \"Epos\" & mehr</p>".into();
    b.meta.tags = vec!["Fantasy".into(), "Klassiker".into()];
    b.meta.series = "Der Herr der Ringe".into();
    b.meta.series_index = 1.5;
    b.meta.identifiers = vec![
        Identifier {
            scheme: "isbn".into(),
            value: "9783608939811".into(),
            xml_id: Some("bookid".into()),
        },
        Identifier {
            scheme: "doi".into(),
            value: "10.1/x".into(),
            xml_id: None,
        },
    ];
    b.meta.published = "1954-07-29".into();
    b.meta.rights = "© Tolkien Estate".into();
    b.meta.rating = 9;
    b.last_read = Some(ts(1_710_000_000));
    b.size = 123_456_789;
    b.progress = 0.4321;
    b.position = Some(ReadPos {
        spine: 7,
        frac: 0.25,
    });
    b.bookmarks = vec![Bookmark {
        id: "bm1".into(),
        spine: 2,
        frac: 0.5,
        label: "Mitte".into(),
        created: ts(1_705_000_000),
    }];
    b.highlights = vec![Highlight {
        id: "h1".into(),
        spine: 3,
        start: -4,
        end: 99,
        color: "yellow".into(),
        style: "squiggly".into(),
        note: "Notiz \u{1F600}".into(),
        text: "Text".into(),
        created: ts(1_706_000_000),
    }];
    b
}

pub fn same(a: &Book, b: &Book) -> bool {
    serde_json::to_value(a).unwrap() == serde_json::to_value(b).unwrap()
}

/// Deterministic xorshift generator.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    pub fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}
