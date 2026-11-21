//! The tag browser as a tree (roadmap L18): tags written with a separator (`Fiction.Fantasy.Epic`)
//! show as nested entries; a parent entry stands for every book under it. No GTK.

use std::collections::BTreeMap;

use crate::core::library::Book;

/// The separator between the levels of a tag.
pub const SEPARATOR: char = '.';

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The last level (`Epic`).
    pub label: String,
    /// The whole path (`Fiction.Fantasy.Epic`).
    pub path: String,
    /// 0 for a top level entry.
    pub indent: usize,
    /// Books that have this tag or one below it.
    pub count: usize,
    /// A library search for exactly these books.
    pub query: String,
    pub has_children: bool,
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', ""))
}

/// The search for one entry: a leaf is the tag itself, a parent also takes the tags below it.
pub fn query_for(path: &str, has_children: bool) -> String {
    if has_children {
        let esc = regex::escape(path);
        format!("tag:{}", quote(&format!("~^{esc}(\\{SEPARATOR}|$)")))
    } else {
        format!("tag:={}", quote(path))
    }
}

/// The rows of the tree, in path order (case does not matter), with the number of books under each.
pub fn rows(books: &[Book]) -> Vec<Row> {
    // path (lower case) -> (display path, books)
    let mut nodes: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for b in books {
        let mut seen: Vec<String> = Vec::new();
        for tag in &b.meta.tags {
            let parts: Vec<&str> = tag
                .split(SEPARATOR)
                .filter(|p| !p.trim().is_empty())
                .collect();
            for i in 1..=parts.len() {
                let path = parts[..i].join(&SEPARATOR.to_string());
                let key = path.to_lowercase();
                if !seen.contains(&key) {
                    seen.push(key.clone());
                    let e = nodes.entry(key).or_insert((path, 0));
                    e.1 += 1;
                }
            }
        }
    }
    let keys: Vec<String> = nodes.keys().cloned().collect();
    keys.iter()
        .map(|k| {
            let (path, count) = nodes[k].clone();
            let prefix = format!("{k}{SEPARATOR}");
            let has_children = keys.iter().any(|o| o.starts_with(&prefix));
            Row {
                label: path.rsplit(SEPARATOR).next().unwrap_or(&path).to_string(),
                indent: path.matches(SEPARATOR).count(),
                query: query_for(&path, has_children),
                path,
                count,
                has_children,
            }
        })
        .collect()
}

/// True when some tag has more than one level (a flat tag list needs no tree).
pub fn any_nested(books: &[Book]) -> bool {
    books
        .iter()
        .flat_map(|b| b.meta.tags.iter())
        .any(|t| t.trim_matches(SEPARATOR).contains(SEPARATOR))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::query::Query;

    fn book(id: u64, tags: &[&str]) -> Book {
        let mut b: Book = serde_json::from_str(&format!(
            r#"{{"id":{id},"file":"a.epub","meta":{{"title":"T{id}","title_sort":"T","authors":[],"author_sort":"","publisher":"","language":"en","description":"","tags":[],"series":"","series_index":0.0,"identifiers":[],"published":"","rights":"","rating":0}},"added":"2026-01-01T00:00:00Z","modified":"2026-01-01T00:00:00Z","size":1}}"#
        ))
        .unwrap();
        b.meta.tags = tags.iter().map(|s| s.to_string()).collect();
        b
    }

    #[test]
    fn dotted_tags_nest_and_parents_count_their_books_once() {
        let books = vec![
            book(1, &["Fiction.Fantasy.Epic", "Fiction.Fantasy"]),
            book(2, &["fiction.fantasy"]),
            book(3, &["Fiction.Crime", "Plain"]),
        ];
        let r = rows(&books);
        let sum: Vec<(String, usize, usize)> = r
            .iter()
            .map(|x| (x.path.clone(), x.indent, x.count))
            .collect();
        assert_eq!(
            sum,
            vec![
                ("Fiction".into(), 0, 3),
                ("Fiction.Crime".into(), 1, 1),
                ("Fiction.Fantasy".into(), 1, 2),
                ("Fiction.Fantasy.Epic".into(), 2, 1),
                ("Plain".into(), 0, 1),
            ]
        );
        assert!(r[0].has_children && !r[1].has_children);
        assert_eq!(r[3].label, "Epic");
        assert!(any_nested(&books));
        assert!(!any_nested(&[book(1, &["a", "b"])]));
    }

    #[test]
    fn the_query_of_an_entry_finds_its_books_and_the_ones_below() {
        let books = vec![
            book(1, &["Fiction.Fantasy.Epic"]),
            book(2, &["Fiction.Crime"]),
            book(3, &["Fictional"]),
            book(4, &["Fiction"]),
        ];
        let find = |q: &str| -> Vec<u64> {
            let q = Query::parse(q);
            books
                .iter()
                .filter(|b| q.matches(b))
                .map(|b| b.id)
                .collect()
        };
        let r = rows(&books);
        let by = |p: &str| r.iter().find(|x| x.path == p).unwrap();
        assert_eq!(find(&by("Fiction").query), vec![1, 2, 4], "not Fictional");
        assert_eq!(find(&by("Fiction.Crime").query), vec![2]);
        assert_eq!(find(&by("Fiction.Fantasy").query), vec![1]);
        assert_eq!(find(&by("Fictional").query), vec![3]);
        // a name with regex characters is searched as written
        let odd = vec![book(1, &["C++.Tips"]), book(2, &["CXX.Tips"])];
        let r = rows(&odd);
        let q = Query::parse(&r.iter().find(|x| x.path == "C++").unwrap().query);
        assert!(q.matches(&odd[0]) && !q.matches(&odd[1]));
    }
}
