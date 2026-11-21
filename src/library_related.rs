//! Quickview (roadmap L19): the other books that share an author, the series or a tag with one
//! book. No GTK.

use crate::core::library::Book;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// `author`, `series` or `tag`.
    pub kind: &'static str,
    pub value: String,
    /// A library search that finds exactly these books and the book itself.
    pub query: String,
    /// The other books: id and title, in title order.
    pub books: Vec<(u64, String)>,
}

fn quote(v: &str) -> String {
    format!("\"{}\"", v.replace('"', ""))
}

/// The groups of `id` that have at least one other book, authors first, then the series, then the
/// tags. At most `max_tags` tag groups are listed (the most shared first).
pub fn related(books: &[Book], id: u64, max_tags: usize) -> Vec<Group> {
    let Some(me) = books.iter().find(|b| b.id == id) else {
        return Vec::new();
    };
    let others = |pred: &dyn Fn(&Book) -> bool| -> Vec<(u64, String)> {
        let mut v: Vec<(u64, String)> = books
            .iter()
            .filter(|b| b.id != id && pred(b))
            .map(|b| (b.id, b.meta.title.clone()))
            .collect();
        v.sort_by(|a, b| {
            a.1.to_lowercase()
                .cmp(&b.1.to_lowercase())
                .then(a.0.cmp(&b.0))
        });
        v
    };
    let mut out = Vec::new();
    for a in &me.meta.authors {
        let list = others(&|b| b.meta.authors.iter().any(|x| x.eq_ignore_ascii_case(a)));
        if !list.is_empty() {
            out.push(Group {
                kind: "author",
                value: a.clone(),
                query: format!("author:={}", quote(a)),
                books: list,
            });
        }
    }
    if !me.meta.series.is_empty() {
        let s = &me.meta.series;
        let list = others(&|b| b.meta.series.eq_ignore_ascii_case(s));
        if !list.is_empty() {
            out.push(Group {
                kind: "series",
                value: s.clone(),
                query: format!("series:={}", quote(s)),
                books: list,
            });
        }
    }
    let mut tags: Vec<Group> = Vec::new();
    for t in &me.meta.tags {
        let list = others(&|b| b.meta.tags.iter().any(|x| x.eq_ignore_ascii_case(t)));
        if !list.is_empty() {
            tags.push(Group {
                kind: "tag",
                value: t.clone(),
                query: format!("tag:={}", quote(t)),
                books: list,
            });
        }
    }
    tags.sort_by(|a, b| {
        b.books
            .len()
            .cmp(&a.books.len())
            .then(a.value.cmp(&b.value))
    });
    out.extend(tags.into_iter().take(max_tags));
    out
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
        b.meta.tags = tags.iter().map(|s| s.to_string()).collect();
        b
    }

    #[test]
    fn shared_authors_series_and_tags_are_listed_without_the_book_itself() {
        let books = vec![
            book(1, "Dune", &["Frank Herbert"], "Dune", &["scifi", "classic"]),
            book(2, "Messiah", &["frank herbert"], "Dune", &["scifi"]),
            book(3, "Emma", &["Jane Austen"], "", &["classic", "scifi"]),
            book(4, "Alone", &["Nobody"], "", &[]),
        ];
        let g = related(&books, 1, 5);
        let kinds: Vec<(&str, &str)> = g.iter().map(|g| (g.kind, g.value.as_str())).collect();
        assert_eq!(
            kinds,
            vec![
                ("author", "Frank Herbert"),
                ("series", "Dune"),
                ("tag", "scifi"),
                ("tag", "classic")
            ]
        );
        assert_eq!(g[0].books, vec![(2, "Messiah".to_string())]);
        assert_eq!(g[2].books.len(), 2, "the most shared tag first");
        assert_eq!(g[0].query, "author:=\"Frank Herbert\"");
        assert!(related(&books, 4, 5).is_empty());
        assert!(related(&books, 99, 5).is_empty());
        assert_eq!(related(&books, 1, 1).len(), 3, "tag groups are capped");
    }

    #[test]
    fn a_query_with_quotes_in_a_name_stays_one_search() {
        let books = vec![
            book(1, "A", &["O\"Brien"], "", &[]),
            book(2, "B", &["O\"Brien"], "", &[]),
        ];
        let g = related(&books, 1, 5);
        assert_eq!(g[0].query, "author:=\"OBrien\"");
    }
}
