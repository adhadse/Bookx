//! Decisions behind adding books (roadmap L14, L15): which fields a file name can fill in, how a
//! typed ISBN becomes a record, how the patterns the user writes are read. No GTK.

use bookx_sources::filename::{self, Pattern};
use bookx_sources::isbn;

use crate::core::library::Prepared;
use crate::core::opf::Metadata;
use crate::core::util::{author_sort, title_sort};

/// The patterns written as text: one per `;`; an entry starting with `re:` is a regular expression
/// (named groups `title`, `author`, `series`, `index`), anything else a template such as
/// `{author} - {title}`. Empty text means the built-in patterns.
pub fn parse_patterns(text: &str) -> Vec<Pattern> {
    let mut out: Vec<Pattern> = text
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| match s.strip_prefix("re:") {
            Some(re) => Pattern::Regex(re.trim().to_string()),
            None => Pattern::Template(s.to_string()),
        })
        .collect();
    out.retain(|p| p.validate().is_ok());
    if out.is_empty() {
        filename::default_patterns()
    } else {
        out
    }
}

/// The first problem of the patterns text (for the settings row), if any.
pub fn patterns_error(text: &str) -> Option<String> {
    text.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .find_map(|s| {
            let p = match s.strip_prefix("re:") {
                Some(re) => Pattern::Regex(re.trim().to_string()),
                None => Pattern::Template(s.to_string()),
            };
            p.validate().err().map(|e| format!("{s}: {e}"))
        })
}

fn missing_author(m: &Metadata) -> bool {
    m.authors.is_empty()
        || (m.authors.len() == 1 && m.authors[0].trim().eq_ignore_ascii_case("unknown"))
}

fn stem(file_name: &str) -> String {
    std::path::Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string()
}

/// Fills in what a book read from a file lacks, using the file name: the authors when it has none,
/// the title when it is only the file name, the series when it has none. Returns the changed
/// fields; a book with real metadata is left alone.
pub fn guess_into(meta: &mut Metadata, file_name: &str, patterns: &[Pattern]) -> Vec<&'static str> {
    let g = filename::guess(file_name, patterns);
    let mut changed = Vec::new();
    let title_is_file = {
        let s = stem(file_name);
        let t = meta.title.trim();
        t.is_empty() || t == s || t == filename::clean_stem(file_name)
    };
    if missing_author(meta) && !g.authors.is_empty() {
        meta.authors = g.authors.clone();
        meta.author_sort = g
            .authors
            .iter()
            .map(|a| author_sort(a))
            .collect::<Vec<_>>()
            .join(" & ");
        changed.push("authors");
    }
    if title_is_file && !g.title.is_empty() && meta.title != g.title {
        meta.title = g.title.clone();
        meta.title_sort = title_sort(&g.title);
        changed.push("title");
    }
    if meta.series.trim().is_empty() && !g.series.is_empty() {
        meta.series = g.series.clone();
        meta.series_index = g.series_index.unwrap_or(0.0);
        changed.push("series");
    }
    changed
}

/// Applies [`guess_into`] to a book that is about to be added (also in the EPUB inside, so the file
/// and the catalogue agree). `Prepared::source_file` or `origin` names the file.
pub fn apply_guess(p: &mut Prepared, on: bool, patterns_text: &str) -> Vec<&'static str> {
    if !on {
        return Vec::new();
    }
    let Some(name) = p
        .source_file
        .as_ref()
        .or(p.origin.as_ref())
        .and_then(|f| f.file_name())
        .and_then(|n| n.to_str())
        .map(str::to_string)
    else {
        return Vec::new();
    };
    let mut meta = p.meta.clone();
    let changed = guess_into(&mut meta, &name, &parse_patterns(patterns_text));
    if changed.is_empty() {
        return changed;
    }
    // the file inside gets the same metadata; when it cannot be rewritten nothing changes
    let rewritten = crate::core::archive::Archive::from_bytes(p.bytes.clone())
        .and_then(crate::core::book::OpenBook::from_archive)
        .and_then(|mut b| {
            b.set_metadata(meta.clone())?;
            b.archive.to_bytes()
        });
    match rewritten {
        Ok(bytes) => {
            p.bytes = bytes;
            p.meta = meta;
            changed
        }
        Err(_) => Vec::new(),
    }
}

/// Why this text cannot be an ISBN, or the ISBN-13 it stands for.
pub fn isbn_from_input(text: &str) -> Result<String, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err(crate::i18n::N_("Type an ISBN").into());
    }
    match isbn::canonical(t) {
        Some(c) => Ok(c),
        None => {
            Err(crate::i18n::N_("That is not a valid ISBN (the check digit does not fit)").into())
        }
    }
}

/// The metadata of a record made from an ISBN: a placeholder title and the identifier.
pub fn isbn_record(text: &str) -> Result<Metadata, String> {
    isbn_from_input(text).map(|i| crate::core::empty::isbn_metadata(&i))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(title: &str, authors: &[&str]) -> Metadata {
        Metadata {
            title: title.into(),
            authors: authors.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_book_without_an_author_learns_it_from_the_file_name() {
        let mut m = meta("Tolkien - The Hobbit", &["Unknown"]);
        let c = guess_into(
            &mut m,
            "Tolkien - The Hobbit.epub",
            &filename::default_patterns(),
        );
        assert_eq!(c, vec!["authors", "title"]);
        assert_eq!(m.authors, vec!["Tolkien"]);
        assert_eq!(m.title, "The Hobbit");
        assert_eq!(m.author_sort, "Tolkien");
    }

    #[test]
    fn real_metadata_is_never_touched() {
        let mut m = meta("The Hobbit", &["J. R. R. Tolkien"]);
        let c = guess_into(
            &mut m,
            "Somebody - Other.epub",
            &filename::default_patterns(),
        );
        assert!(c.is_empty());
        assert_eq!(m.title, "The Hobbit");
        // a good title with a missing author keeps the title
        let mut m = meta("Real Title", &[]);
        let c = guess_into(
            &mut m,
            "Jane Doe - Other Name.epub",
            &filename::default_patterns(),
        );
        assert_eq!(c, vec!["authors"]);
        assert_eq!(m.title, "Real Title");
    }

    #[test]
    fn the_series_comes_from_the_name_when_there_is_none() {
        let mut m = meta("Discworld #12 - The Wee Free Men", &[]);
        let c = guess_into(
            &mut m,
            "Discworld #12 - The Wee Free Men.epub",
            &filename::default_patterns(),
        );
        assert!(c.contains(&"series"));
        assert_eq!((m.series.as_str(), m.series_index), ("Discworld", 12.0));
    }

    #[test]
    fn patterns_are_read_from_text() {
        assert_eq!(parse_patterns("").len(), filename::default_patterns().len());
        let p = parse_patterns("{title} by {author}; re:^(?P<title>.+)$");
        assert_eq!(p.len(), 2);
        assert!(matches!(p[1], Pattern::Regex(_)));
        // a broken one is dropped in use and named for the settings row
        assert_eq!(
            parse_patterns("re:(").len(),
            filename::default_patterns().len()
        );
        assert!(patterns_error("re:(").unwrap().contains("re:("));
        assert!(patterns_error("{title} - {author}").is_none());
        let mut m = meta("by Jane - Name", &[]);
        guess_into(
            &mut m,
            "Name by Jane Doe.epub",
            &parse_patterns("{title} by {author}"),
        );
        assert_eq!(m.authors, vec!["Jane Doe"]);
    }

    #[test]
    fn isbns_are_checked_and_made_canonical() {
        assert_eq!(
            isbn_from_input(" 978-0-441-01359-3 ").unwrap(),
            "9780441013593"
        );
        assert_eq!(isbn_from_input("0441013597").unwrap(), "9780441013593");
        assert!(isbn_from_input("9780441013594").is_err());
        assert!(isbn_from_input("").is_err());
        let m = isbn_record("0441013597").unwrap();
        assert_eq!(m.identifier("isbn"), Some("9780441013593"));
        assert!(m.title.starts_with("ISBN "));
    }

    #[test]
    fn a_prepared_book_is_rewritten_with_the_guess() {
        use crate::core::convert::{Chapter, create_epub};
        let md = meta("Tolkien - The Hobbit", &["Unknown"]);
        let ch = Chapter {
            title: "One".into(),
            body: "<p>x</p>".into(),
        };
        let bytes = create_epub(&md, &[ch], None).to_bytes().unwrap();
        let mut p = Prepared {
            meta: md,
            cover: None,
            bytes,
            origin: None,
            drm: String::new(),
            source_file: Some("/tmp/Tolkien - The Hobbit.epub".into()),
            source_id: None,
        };
        assert!(
            apply_guess(&mut p, false, "").is_empty(),
            "off by default for the caller's choice"
        );
        let c = apply_guess(&mut p, true, "");
        assert_eq!(c, vec!["authors", "title"]);
        let inner = crate::core::book::OpenBook::from_archive(
            crate::core::archive::Archive::from_bytes(p.bytes.clone()).unwrap(),
        )
        .unwrap();
        assert_eq!(inner.metadata().title, "The Hobbit");
        assert_eq!(inner.metadata().authors, vec!["Tolkien"]);
    }
}
