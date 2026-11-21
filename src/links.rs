//! `bookx://` links (X20), without GTK: parsing, and finding the book a link names.
//!
//! * `bookx://open?id=42` opens the book with that library id (ids belong to one library).
//! * `bookx://book/isbn:9780441172719` (or `…/9780441172719`, or any other identifier such as
//!   `doi:…` or `uuid:…`) opens the book that carries the identifier, in any library.
//!
//! The desktop file registers the scheme (`x-scheme-handler/bookx`); a second `bookx` process
//! hands the link to the running one through the application action `app.open-uri`.

use crate::core::library::Library;

pub const SCHEME: &str = "bookx";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// A library id.
    Id(u64),
    /// An identifier as written in the link: `isbn:9780…` or just the value.
    Identifier(String),
}

/// Reads a `bookx://` link; `None` when it is not one of ours or is malformed.
pub fn parse(uri: &str) -> Option<Link> {
    let uri = uri.trim();
    let rest = strip_scheme(uri)?;
    let (path, query) = match rest.split_once('?') {
        Some((p, q)) => (p, q),
        None => (rest, ""),
    };
    let path = path.trim_matches('/');
    if path == "open" {
        let id = query
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .find(|(k, _)| *k == "id")
            .and_then(|(_, v)| v.parse::<u64>().ok())?;
        return Some(Link::Id(id));
    }
    if let Some(ident) = path.strip_prefix("book/") {
        let ident = percent_decode(ident)?;
        let ident = ident.trim();
        if ident.is_empty() || ident.chars().any(char::is_control) {
            return None;
        }
        return Some(Link::Identifier(ident.to_string()));
    }
    None
}

fn strip_scheme(uri: &str) -> Option<&str> {
    let (scheme, rest) = uri.split_once("://")?;
    scheme.eq_ignore_ascii_case(SCHEME).then_some(rest)
}

/// Whether an argument or URI is a `bookx://` link.
pub fn is_link(arg: &str) -> bool {
    strip_scheme(arg.trim()).is_some()
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The id of the book a link names in this library.
pub fn resolve(link: &Link, lib: &Library) -> Option<u64> {
    match link {
        Link::Id(id) => lib.get(*id).map(|b| b.id),
        Link::Identifier(want) => {
            let want = want.to_ascii_lowercase();
            lib.books.iter().find_map(|b| {
                b.meta
                    .identifiers
                    .iter()
                    .any(|i| {
                        let value = i.value.to_ascii_lowercase();
                        let full = format!("{}:{}", i.scheme.to_ascii_lowercase(), value);
                        full == want || value == want
                    })
                    .then_some(b.id)
            })
        }
    }
}

/// The links among command line arguments (`bookx bookx://open?id=3`).
#[cfg(test)]
pub fn from_args(args: &[String]) -> Vec<String> {
    args.iter().filter(|a| is_link(a)).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_links_carry_an_id() {
        assert_eq!(parse("bookx://open?id=42"), Some(Link::Id(42)));
        assert_eq!(parse("BOOKX://open?x=1&id=7"), Some(Link::Id(7)));
        assert_eq!(parse("bookx://open?id=abc"), None);
        assert_eq!(parse("bookx://open"), None);
        assert_eq!(parse("bookx://open?id=-1"), None);
    }

    #[test]
    fn book_links_carry_an_identifier() {
        assert_eq!(
            parse("bookx://book/isbn:9780441172719"),
            Some(Link::Identifier("isbn:9780441172719".into()))
        );
        assert_eq!(
            parse("bookx://book/doi%3A10.1000%2F182"),
            Some(Link::Identifier("doi:10.1000/182".into()))
        );
        assert_eq!(parse("bookx://book/"), None);
        assert_eq!(parse("bookx://book/%zz"), None);
        assert_eq!(parse("bookx://book/a%00b"), None);
    }

    #[test]
    fn other_schemes_and_paths_are_not_ours() {
        assert_eq!(parse("https://open?id=1"), None);
        assert_eq!(parse("file:///tmp/a.epub"), None);
        assert_eq!(parse("bookx://settings"), None);
        assert_eq!(parse("not a url"), None);
        assert!(is_link(" bookx://open?id=1 "));
        assert!(!is_link("/tmp/book.epub"));
        assert_eq!(
            from_args(&["a.epub".into(), "bookx://open?id=1".into()]),
            vec!["bookx://open?id=1".to_string()]
        );
    }

    fn book(id: u64, ident: &str) -> crate::core::library::Book {
        let json = format!(
            r#"{{"id":{id},"file":"a.epub","meta":{{"title":"T","title_sort":"T","authors":["A"],"author_sort":"A","publisher":"","language":"en","description":"","tags":[],"series":"","series_index":0.0,"identifiers":[{ident}],"published":"","rights":"","rating":0}},"added":"2026-01-01T00:00:00Z","modified":"2026-01-01T00:00:00Z","size":1}}"#
        );
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn links_find_books_by_id_and_identifier() {
        let root = std::env::temp_dir().join(format!("bookx-links-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let mut lib = Library::in_memory(&root).unwrap();
        lib.books
            .push(book(3, r#"{"scheme":"isbn","value":"9780441172719"}"#));
        lib.books
            .push(book(9, r#"{"scheme":"doi","value":"10.1000/182"}"#));
        assert_eq!(resolve(&Link::Id(9), &lib), Some(9));
        assert_eq!(resolve(&Link::Id(4), &lib), None);
        for ident in ["isbn:9780441172719", "9780441172719", "ISBN:9780441172719"] {
            assert_eq!(resolve(&Link::Identifier(ident.into()), &lib), Some(3));
        }
        assert_eq!(
            resolve(&Link::Identifier("doi:10.1000/182".into()), &lib),
            Some(9)
        );
        assert_eq!(resolve(&Link::Identifier("isbn:000".into()), &lib), None);
        let _ = std::fs::remove_dir_all(root);
    }
}
