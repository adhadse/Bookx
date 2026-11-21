//! A catalogue of books as an EPUB (roadmap L28): a title page, then the books by title, by author,
//! by series and by tag, each with its author and series. Reading it on a device lets one browse the
//! library without the books. The EPUB can be converted to other formats like any other.

use std::collections::BTreeMap;

use anyhow::Result;

use super::convert::{create_epub, Chapter};
use super::library::Book;
use super::opf::Metadata;
use super::util::xml_escape;

fn key(s: &str) -> String {
    s.to_lowercase()
}

fn line(b: &Book) -> String {
    let mut s = format!("<li><b>{}</b>", xml_escape(&b.meta.title));
    if !b.meta.authors.is_empty() {
        s += &format!(" — {}", xml_escape(&b.meta.authors.join(" & ")));
    }
    if !b.meta.series.is_empty() {
        s += &format!(
            " <i>({} #{})</i>",
            xml_escape(&b.meta.series),
            super::opf::format_index(b.meta.series_index)
        );
    }
    s + "</li>"
}

fn sorted<'a>(books: &[&'a Book]) -> Vec<&'a Book> {
    let mut v = books.to_vec();
    v.sort_by(|a, b| {
        key(&a.meta.title_sort)
            .cmp(&key(&b.meta.title_sort))
            .then(a.id.cmp(&b.id))
    });
    v
}

fn grouped(books: &[&Book], names: impl Fn(&Book) -> Vec<String>, empty: &str) -> String {
    let mut map: BTreeMap<String, Vec<&Book>> = BTreeMap::new();
    for b in sorted(books) {
        let n = names(b);
        if n.is_empty() {
            map.entry(empty.to_string()).or_default().push(b);
        }
        for name in n {
            map.entry(name).or_default().push(b);
        }
    }
    let mut out = String::new();
    let mut entries: Vec<_> = map.into_iter().collect();
    entries.sort_by_key(|(k, _)| key(k));
    for (name, list) in entries {
        out += &format!("<h2>{}</h2><ul>", xml_escape(&name));
        for b in list {
            out += &line(b);
        }
        out += "</ul>";
    }
    out
}

/// The catalogue of `books` as EPUB bytes. `labels` are the (translated) section names:
/// `[by title, by author, by series, by tag, no author, no series, no tag]`.
pub fn build(title: &str, books: &[&Book], labels: &[&str; 7]) -> Result<Vec<u8>> {
    let mut chapters = Vec::new();
    let list: String = sorted(books).into_iter().map(line).collect();
    chapters.push(Chapter {
        title: labels[0].to_string(),
        body: format!("<h1>{}</h1><ul>{list}</ul>", xml_escape(labels[0])),
    });
    chapters.push(Chapter {
        title: labels[1].to_string(),
        body: format!(
            "<h1>{}</h1>{}",
            xml_escape(labels[1]),
            grouped(books, |b| b.meta.authors.clone(), labels[4])
        ),
    });
    chapters.push(Chapter {
        title: labels[2].to_string(),
        body: format!(
            "<h1>{}</h1>{}",
            xml_escape(labels[2]),
            grouped(
                books,
                |b| if b.meta.series.is_empty() {
                    Vec::new()
                } else {
                    vec![b.meta.series.clone()]
                },
                labels[5]
            )
        ),
    });
    chapters.push(Chapter {
        title: labels[3].to_string(),
        body: format!(
            "<h1>{}</h1>{}",
            xml_escape(labels[3]),
            grouped(books, |b| b.meta.tags.clone(), labels[6])
        ),
    });
    let md = Metadata {
        title: title.to_string(),
        authors: vec!["Bookx".into()],
        language: "en".into(),
        ..Default::default()
    };
    create_epub(&md, &chapters, None).to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(id: u64, title: &str, authors: &[&str], series: &str, tags: &[&str]) -> Book {
        let mut b: Book = serde_json::from_str(&format!(
            r#"{{"id":{id},"file":"a.epub","meta":{{"title":"{title}","title_sort":"{title}","authors":[],"author_sort":"","publisher":"","language":"en","description":"","tags":[],"series":"","series_index":0.0,"identifiers":[],"published":"","rights":"","rating":0}},"added":"2026-01-01T00:00:00Z","modified":"2026-01-01T00:00:00Z","size":1}}"#
        ))
        .unwrap();
        b.meta.authors = authors.iter().map(|s| s.to_string()).collect();
        b.meta.series = series.into();
        b.meta.series_index = 2.0;
        b.meta.tags = tags.iter().map(|s| s.to_string()).collect();
        b
    }

    const L: [&str; 7] = [
        "By Title",
        "By Author",
        "By Series",
        "By Tag",
        "No author",
        "No series",
        "No tag",
    ];

    #[test]
    fn the_catalogue_is_an_epub_with_the_four_listings() {
        let a = book(1, "Zeta <&> Book", &["Ann"], "Saga", &["x", "y"]);
        let b = book(2, "Alpha", &[], "", &[]);
        let bytes = build("My Library", &[&a, &b], &L).unwrap();
        let open = crate::book::OpenBook::from_archive(
            crate::archive::Archive::from_bytes(bytes).unwrap(),
        )
        .unwrap();
        assert_eq!(open.metadata().title, "My Library");
        let text: String = open
            .archive
            .names()
            .iter()
            .filter(|n| n.ends_with("html"))
            .filter_map(|n| open.archive.get_string(n))
            .collect();
        assert!(text.contains("Zeta &lt;&amp;&gt; Book"), "escaped");
        assert!(text.contains("Saga #2"));
        for l in ["By Author", "No author", "No series", "No tag"] {
            assert!(text.contains(l), "{l}");
        }
        assert!(
            text.find("Alpha").unwrap() < text.find("Zeta").unwrap(),
            "sorted by title"
        );
    }

    #[test]
    fn an_empty_library_still_makes_a_valid_catalogue() {
        assert!(build("Empty", &[], &L).is_ok());
    }
}
