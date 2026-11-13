//! How complete the metadata of an imported book is (roadmap PP-G-1, "weak metadata").
//!
//! A book is *weak* when it has no author, a title that looks like a file name, no cover or no
//! language. The library search knows this as `weak:` (any reason) and `weak:author`, `weak:title`,
//! `weak:cover`, `weak:language`; the *Needs attention* entry is that search.

use super::library::Book;

/// One reason a book needs a look.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Weak {
    NoAuthor,
    FilenameTitle,
    NoCover,
    NoLanguage,
}

impl Weak {
    pub const ALL: [Weak; 4] = [
        Weak::NoAuthor,
        Weak::FilenameTitle,
        Weak::NoCover,
        Weak::NoLanguage,
    ];

    /// The word after `weak:` in the search.
    pub fn key(self) -> &'static str {
        match self {
            Weak::NoAuthor => "author",
            Weak::FilenameTitle => "title",
            Weak::NoCover => "cover",
            Weak::NoLanguage => "language",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Weak::NoAuthor => "no author",
            Weak::FilenameTitle => "title looks like a file name",
            Weak::NoCover => "no cover",
            Weak::NoLanguage => "no language",
        }
    }
}

const EXTENSIONS: &[&str] = &[
    "epub", "pdf", "mobi", "azw", "azw3", "fb2", "txt", "docx", "doc", "rtf", "cbz", "cbr", "djvu",
    "htm", "html", "md", "kepub", "lit", "odt",
];

const PLACEHOLDERS: &[&str] = &["unknown", "untitled", "no title", "document"];

/// Does `title` read like a file name (`my_book_final.epub`, `the-hobbit-1937`, `Untitled`)?
pub fn looks_like_filename(title: &str) -> bool {
    let t = title.trim();
    if t.is_empty() {
        return true;
    }
    let lower = t.to_lowercase();
    if PLACEHOLDERS.contains(&lower.as_str()) || lower.starts_with("microsoft word - ") {
        return true;
    }
    if let Some((_, ext)) = lower.rsplit_once('.') {
        if EXTENSIONS.contains(&ext) {
            return true;
        }
    }
    if t.contains(char::is_whitespace) {
        return false;
    }
    // one word with separators: `the_hobbit`, `the-hobbit-1937`, `scan0042`
    let separators = t.chars().filter(|c| matches!(c, '_' | '-')).count();
    if t.contains('_') && separators >= 1 {
        return true;
    }
    let slug = t
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    slug && t.contains('-')
}

fn unknown_author(a: &str) -> bool {
    let a = a.trim().to_lowercase();
    a.is_empty() || a == "unknown" || a == "unknown author" || a == "anonymous"
}

/// Everything that is weak about `b`, in a fixed order.
pub fn weak_reasons(b: &Book) -> Vec<Weak> {
    let mut v = Vec::new();
    if b.meta.authors.iter().all(|a| unknown_author(a)) {
        v.push(Weak::NoAuthor);
    }
    if looks_like_filename(&b.meta.title) {
        v.push(Weak::FilenameTitle);
    }
    if b.cover.as_deref().is_none_or(|c| c.trim().is_empty()) {
        v.push(Weak::NoCover);
    }
    if b.meta.language.trim().is_empty() {
        v.push(Weak::NoLanguage);
    }
    v
}

/// Does `b` have the reason named `key` (`""` = any)?
pub fn is_weak(b: &Book, key: &str) -> bool {
    let reasons = weak_reasons(b);
    if key.trim().is_empty() {
        return !reasons.is_empty();
    }
    reasons.iter().any(|r| r.key().starts_with(key.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(title: &str, authors: &[&str], cover: bool, lang: &str) -> Book {
        let mut b: Book = serde_json::from_str(
            r#"{"id":1,"file":"a.epub","meta":{"title":"T","title_sort":"T","authors":["A"],"author_sort":"A","publisher":"","language":"","description":"","tags":[],"series":"","series_index":0.0,"identifiers":[],"published":"","rights":"","rating":0},"added":"2026-01-01T00:00:00Z","modified":"2026-01-01T00:00:00Z","size":1}"#,
        )
        .unwrap();
        b.meta.title = title.into();
        b.meta.authors = authors.iter().map(|s| s.to_string()).collect();
        b.meta.language = lang.into();
        b.cover = cover.then(|| "c.jpg".to_string());
        b
    }

    #[test]
    fn file_name_like_titles_are_found() {
        for t in [
            "my_book_final",
            "Dune.epub",
            "the-hobbit-1937",
            "Untitled",
            "",
            "Microsoft Word - report",
            "scan_0042",
        ] {
            assert!(looks_like_filename(t), "{t:?}");
        }
        for t in [
            "Dune",
            "The Hobbit, or There and Back Again",
            "Catch-22",
            "Spider-Man",
            "Mr. Palomar",
            "1984",
        ] {
            assert!(!looks_like_filename(t), "{t:?}");
        }
    }

    #[test]
    fn a_complete_book_is_not_weak() {
        let b = book("Dune", &["Frank Herbert"], true, "en");
        assert!(weak_reasons(&b).is_empty());
        assert!(!is_weak(&b, ""));
    }

    #[test]
    fn every_reason_is_reported_in_order() {
        let b = book("scan_1", &["Unknown"], false, "");
        assert_eq!(weak_reasons(&b), Weak::ALL.to_vec());
        for k in ["author", "title", "cover", "language", ""] {
            assert!(is_weak(&b, k), "{k}");
        }
        let b = book("Dune", &["Frank Herbert"], false, "en");
        assert_eq!(weak_reasons(&b), vec![Weak::NoCover]);
        assert!(is_weak(&b, "cov") && !is_weak(&b, "author"));
    }
}
