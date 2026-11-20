//! Synthetic libraries for scale measurements (PP-C-11, PP-G-12): pure functions of the book id, so
//! any size is reproducible and no copyrighted text is involved. Titles, names and descriptions are
//! built from syllables; the shape (many authors with a few books, a quarter of the books in a series,
//! a few hundred tags, some books with a second format, a planted share of near-duplicates) follows
//! what real collections look like. Shared with other crates' tests through `#[path]`.
#![allow(dead_code)]

use bookx_core::library::Book;
use bookx_core::opf::{Identifier, Metadata};
use chrono::{TimeZone, Utc};

/// splitmix64 over `(id, salt)`: independent, reproducible numbers per book and purpose.
pub fn mix(id: u64, salt: u64) -> u64 {
    let mut z = id
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(salt.wrapping_mul(0xD1B5_4A32_D192_ED03))
        .wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

const SYL: &[&str] = &[
    "ba", "ko", "ri", "mu", "sen", "tal", "dor", "vi", "lan", "pe", "ru", "mar", "ni", "sol", "ka",
    "the", "gro", "fan", "wel", "zi", "or", "an", "el", "is", "um", "qua", "bre", "cho", "dra",
    "hel",
];

/// A pronounceable made-up word of `n` syllables from `seed`.
pub fn word(seed: u64, n: usize) -> String {
    let mut s = String::new();
    for k in 0..n {
        s += SYL[(mix(seed, k as u64 + 1) % SYL.len() as u64) as usize];
    }
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

/// Which author writes book `i` (about 8 books per author, uneven).
pub fn author_no(i: u64, n: u64) -> u64 {
    let authors = (n / 8).max(1);
    // a squared draw makes some authors much more productive than others
    let u = (mix(i, 1) % 10_000) as f64 / 10_000.0;
    ((u * u) * authors as f64) as u64
}

pub fn author_name(no: u64) -> String {
    // surnames come from a pool of 2500, so the same surname is shared by several authors
    format!("{} {}", word(no, 2), word(1_000_000 + no % 2500, 3))
}

/// Book `i` (1-based) of a library of `n` books. Every 500th book is a near-duplicate of the book
/// before it (same author and title, different spelling and file).
pub fn synth_book(i: u64, n: u64) -> Book {
    if i > 1 && i % 500 == 0 {
        let mut d = synth_book(i - 1, n);
        d.id = i;
        d.meta.title = format!("The {}!", d.meta.title);
        d.file = format!("books/{:05}/b{i}.epub", i / 1000);
        return d;
    }
    let title = format!(
        "{} {} {}",
        word(mix(i, 2), 2),
        word(mix(i, 3), 3),
        word(mix(i, 4), 2)
    );
    let author = author_name(author_no(i, n));
    let by_comma = {
        let (a, b) = author.split_once(' ').unwrap_or((&author, ""));
        format!("{b}, {a}")
    };
    let in_series = mix(i, 5) % 4 == 0;
    let series_no = mix(i, 6) % (n / 10).max(1);
    let tags: Vec<String> = (0..1 + mix(i, 7) % 3)
        .map(|k| format!("tag-{}", mix(i, 10 + k) % 400))
        .collect();
    let added = 1_500_000_000 + (mix(i, 8) % 200_000_000) as i64;
    let words: Vec<String> = (0..24).map(|k| word(mix(i, 100 + k), 2)).collect();
    let mut b = Book {
        id: i,
        file: format!("books/{:05}/b{i}.epub", i / 1000),
        cover: (mix(i, 9) % 10 != 0).then(|| format!("books/{:05}/c{i}.jpg", i / 1000)),
        meta: Metadata {
            title_sort: title.to_lowercase(),
            title,
            authors: vec![author],
            author_sort: by_comma,
            publisher: format!("{} Press", word(mix(i, 11) % 300, 2)),
            language: ["en", "de", "fr", "es"][(mix(i, 12) % 4) as usize].into(),
            description: words.join(" "),
            tags,
            series: if in_series {
                format!("{} Cycle", word(series_no, 3))
            } else {
                String::new()
            },
            series_index: if in_series {
                (mix(i, 13) % 9) as f64 + 1.0
            } else {
                0.0
            },
            identifiers: vec![Identifier {
                scheme: "isbn".into(),
                value: format!("97{:011}", i),
                xml_id: None,
            }],
            published: format!("{}", 1900 + mix(i, 14) % 125),
            rights: String::new(),
            rating: (mix(i, 15) % 11) as u8,
        },
        added: Utc.timestamp_opt(added, 0).unwrap(),
        modified: Utc.timestamp_opt(added + 60, 0).unwrap(),
        last_read: (mix(i, 16) % 5 == 0).then(|| Utc.timestamp_opt(added + 86_400, 0).unwrap()),
        size: 200_000 + mix(i, 17) % 3_000_000,
        progress: (mix(i, 18) % 101) as f64 / 100.0,
        position: None,
        bookmarks: vec![],
        highlights: vec![],
        formats: vec![],
        resources: vec![],
        drm: String::new(),
    };
    if mix(i, 19) % 6 == 0 {
        b.formats = vec![bookx_core::formats::FormatFile {
            file: format!("books/{:05}/b{i}.mobi", i / 1000),
            format: "mobi".into(),
            size: b.size / 2,
        }];
    }
    b
}

/// The books `1..=n`.
pub fn synth_library(n: u64) -> Vec<Book> {
    (1..=n).map(|i| synth_book(i, n)).collect()
}

/// Resident memory of this process in kilobytes (Linux), 0 elsewhere.
pub fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reproducible_and_shaped_like_a_library() {
        let n = 20_000;
        let a = synth_library(n);
        let b = synth_library(n);
        assert_eq!(a[777].meta.title, b[777].meta.title);
        let authors: std::collections::HashSet<_> =
            a.iter().map(|b| b.meta.authors[0].clone()).collect();
        assert!(
            authors.len() > 500 && authors.len() < 5000,
            "{}",
            authors.len()
        );
        let series = a.iter().filter(|b| !b.meta.series.is_empty()).count();
        assert!((3500..6500).contains(&series), "{series}");
        assert_eq!(
            a.iter()
                .map(|b| b.id)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            n as usize
        );
        // the planted near-duplicate
        assert_eq!(a[499].meta.authors, a[498].meta.authors);
        assert_ne!(a[499].meta.title, a[498].meta.title);
    }
}
