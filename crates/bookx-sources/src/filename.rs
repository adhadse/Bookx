//! Guess title, author and series from file names (L15).

use std::sync::OnceLock;

use anyhow::{Context, Result, bail};
use bookx_core::opf::Metadata;
use regex::Regex;

use crate::authors::{split_authors, to_first_last};

/// What could be read from a file name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Guess {
    pub title: String,
    pub authors: Vec<String>,
    pub series: String,
    pub series_index: Option<f64>,
}

impl Guess {
    pub fn into_metadata(self) -> Metadata {
        Metadata {
            title: self.title,
            authors: self.authors,
            series: self.series,
            series_index: self.series_index.unwrap_or(0.0),
            ..Default::default()
        }
    }
}

/// A way of reading a file name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    /// Text with `{title}`, `{author}`, `{author_lf}` (`Last, First`), `{series}`, `{index}` and `{skip}`
    /// placeholders, e.g. `{author} - {title}`. A space in the template means "one or more spaces".
    Template(String),
    /// A regular expression with the named groups `title` (required), `author`, `series`, `index`.
    Regex(String),
}

/// The patterns tried by [`guess`] when the caller has no preference, most specific first.
pub fn default_patterns() -> Vec<Pattern> {
    [
        "{author} - {series} #{index} - {title}",
        "{series} #{index} - {title}",
        "{title} ({series} #{index})",
        "{author_lf} - {title}",
        "{title} ({author})",
        "{author} - {title}",
    ]
    .iter()
    .map(|s| Pattern::Template(s.to_string()))
    .collect()
}

struct Compiled {
    re: Regex,
    last_first: bool,
}

const TOKENS: &[&str] = &["title", "author", "author_lf", "series", "index", "skip"];

fn compile(p: &Pattern) -> Result<Compiled> {
    match p {
        Pattern::Regex(r) => {
            let re = Regex::new(r).with_context(|| format!("invalid pattern regex `{r}`"))?;
            if !re.capture_names().flatten().any(|n| n == "title") {
                bail!("pattern regex `{r}` needs a (?P<title>...) group");
            }
            Ok(Compiled {
                re,
                last_first: false,
            })
        }
        Pattern::Template(t) => {
            let mut rx = String::from("^");
            let mut seen: Vec<String> = Vec::new();
            let mut rest = t.as_str();
            while !rest.is_empty() {
                if let Some(start) = rest.find('{') {
                    push_literal(&mut rx, &rest[..start]);
                    let Some(len) = rest[start..].find('}') else {
                        bail!("unclosed `{{` in pattern `{t}`");
                    };
                    let name = &rest[start + 1..start + len];
                    if !TOKENS.contains(&name) {
                        bail!("unknown placeholder `{{{name}}}` in pattern `{t}`");
                    }
                    if name != "skip" && seen.iter().any(|s| s == name) {
                        bail!("placeholder `{{{name}}}` used twice in pattern `{t}`");
                    }
                    seen.push(name.to_string());
                    rx.push_str(match name {
                        "index" => r"(?P<index>\d+(?:\.\d+)?)",
                        "author_lf" => r"(?P<author_lf>[^,]+?,\s*.+?)",
                        "skip" => r".+?",
                        other => match other {
                            "title" => "(?P<title>.+?)",
                            "author" => "(?P<author>.+?)",
                            _ => "(?P<series>.+?)",
                        },
                    });
                    rest = &rest[start + len + 1..];
                } else {
                    push_literal(&mut rx, rest);
                    break;
                }
            }
            rx.push('$');
            if !seen.iter().any(|s| s == "title") {
                bail!("pattern `{t}` has no {{title}}");
            }
            Ok(Compiled {
                re: Regex::new(&rx).with_context(|| format!("pattern `{t}`"))?,
                last_first: seen.iter().any(|s| s == "author_lf"),
            })
        }
    }
}

fn push_literal(rx: &mut String, lit: &str) {
    let mut in_space = false;
    for c in lit.chars() {
        if c.is_whitespace() {
            if !in_space {
                rx.push_str(r"\s+");
            }
            in_space = true;
        } else {
            in_space = false;
            rx.push_str(&regex::escape(&c.to_string()));
        }
    }
}

impl Pattern {
    /// Checks that the pattern is usable (known placeholders, a title, a valid regex).
    pub fn validate(&self) -> Result<()> {
        compile(self).map(|_| ())
    }
}

const BOOK_EXTENSIONS: &[&str] = &[
    "epub", "mobi", "azw", "azw3", "azw4", "kfx", "pdf", "fb2", "fbz", "txt", "cbz", "cbr", "cbt",
    "docx", "doc", "odt", "rtf", "htmlz", "txtz", "kepub", "djvu", "lit", "prc", "lrf", "pdb",
    "md", "html", "htm", "zip",
];

fn junk_regexes() -> &'static Vec<Regex> {
    static R: OnceLock<Vec<Regex>> = OnceLock::new();
    R.get_or_init(|| {
        [
            r"(?i)\s*\((?:[a-z0-9-]+\.)+(?:org|com|net|io|is|li|rs|to|se|me|cc)\)",
            r"(?i)\s*\((?:z-?library|libgen|retail|true epub|epub|pdf|mobi|azw3|ocr|scan|scanned|ebook|repack)\)",
            r"\s*\[[^\]]*\]",
        ]
        .iter()
        .map(|p| Regex::new(p).expect("static regex"))
        .collect()
    })
}

fn year_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"(?:\s*\((?:1[5-9]\d\d|20[0-2]\d)\)|\s*[-,]\s*\(?(?:1[5-9]\d\d|20[0-2]\d)\)?)\s*$",
        )
        .expect("static regex")
    })
}

/// The file name without directories and without a known book extension.
fn raw_stem(file_name: &str) -> String {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    match base.rsplit_once('.') {
        Some((stem, ext)) if BOOK_EXTENSIONS.contains(&ext.to_lowercase().as_str()) => stem,
        _ => base,
    }
    .to_string()
}

/// Cleans a file name for guessing: drops the extension, `[...]` groups, site junk like
/// `(z-lib.org)`, a trailing year, turns `_` (and dotted words) into spaces.
pub fn clean_stem(file_name: &str) -> String {
    let mut s = raw_stem(file_name);
    s = s.replace('_', " ");
    for re in junk_regexes() {
        s = re.replace_all(&s, "").into_owned();
    }
    if !s.contains(' ') && s.matches('.').count() >= 2 {
        s = s.replace('.', " ");
    }
    s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let stripped = year_regex().replace(&s, "").trim().to_string();
    if !stripped.is_empty() {
        s = stripped;
    }
    s.trim_matches(|c: char| matches!(c, '-' | '.' | ',') || c.is_whitespace())
        .to_string()
}

fn authors_from(raw: &str, last_first: bool) -> Vec<String> {
    let mut out = Vec::new();
    for a in split_authors(raw) {
        let a = if last_first || a.matches(',').count() == 1 {
            to_first_last(&a)
        } else {
            a
        };
        let a = a.trim().to_string();
        if !a.is_empty() && !out.contains(&a) {
            out.push(a);
        }
    }
    out
}

fn plausible_author(raw: &str) -> bool {
    let words = raw.split_whitespace().count();
    raw.chars().any(|c| c.is_alphabetic()) && words <= 8
}

/// Applies one pattern to an already cleaned stem.
fn apply(c: &Compiled, stem: &str) -> Option<Guess> {
    let caps = c.re.captures(stem)?;
    let get = |n: &str| {
        caps.name(n)
            .map(|m| m.as_str().trim())
            .filter(|s| !s.is_empty())
    };
    let title = get("title")?;
    let mut g = Guess {
        title: title.to_string(),
        ..Default::default()
    };
    if let Some(a) = get("author_lf").or_else(|| get("author")) {
        if !plausible_author(a) {
            return None;
        }
        g.authors = authors_from(a, c.last_first);
    }
    if let Some(s) = get("series") {
        g.series = s.to_string();
    }
    g.series_index = get("index").and_then(|i| i.parse::<f64>().ok());
    Some(g)
}

/// Guesses metadata from a file name: tries `patterns` in order and uses the first that matches;
/// when none does, the cleaned name becomes the title. Invalid patterns are skipped
/// (use [`Pattern::validate`] to report them).
pub fn guess(file_name: &str, patterns: &[Pattern]) -> Guess {
    let stem = clean_stem(file_name);
    let raw = raw_stem(file_name);
    for p in patterns {
        let Ok(c) = compile(p) else { continue };
        // custom regexes see the untouched name first (they may rely on `_` or brackets)
        if matches!(p, Pattern::Regex(_)) {
            if let Some(g) = apply(&c, &raw) {
                return g;
            }
        }
        if let Some(g) = apply(&c, &stem) {
            return g;
        }
    }
    Guess {
        title: stem,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(name: &str) -> Guess {
        guess(name, &default_patterns())
    }

    fn t(p: &str) -> Pattern {
        Pattern::Template(p.into())
    }

    #[test]
    fn author_dash_title() {
        let r = g("J. R. R. Tolkien - The Hobbit.epub");
        assert_eq!(r.title, "The Hobbit");
        assert_eq!(r.authors, vec!["J. R. R. Tolkien"]);
        assert_eq!(r.series, "");
        assert_eq!(r.series_index, None);
    }

    #[test]
    fn title_paren_author() {
        let r = g("The Hobbit (J. R. R. Tolkien).mobi");
        assert_eq!(r.title, "The Hobbit");
        assert_eq!(r.authors, vec!["J. R. R. Tolkien"]);
    }

    #[test]
    fn series_index_title() {
        let r = g("Discworld #12 - The Wee Free Men.epub");
        assert_eq!(r.series, "Discworld");
        assert_eq!(r.series_index, Some(12.0));
        assert_eq!(r.title, "The Wee Free Men");
        assert!(r.authors.is_empty());
    }

    #[test]
    fn fractional_series_index() {
        let r = g("Cosmere #1.5 - Edgedancer.epub");
        assert_eq!(r.series_index, Some(1.5));
    }

    #[test]
    fn author_series_index_title() {
        let r = g("Terry Pratchett - Discworld #12 - The Wee Free Men.epub");
        assert_eq!(r.authors, vec!["Terry Pratchett"]);
        assert_eq!(r.series, "Discworld");
        assert_eq!(r.series_index, Some(12.0));
        assert_eq!(r.title, "The Wee Free Men");
    }

    #[test]
    fn title_with_series_in_parens() {
        let r = g("Dune (Dune Chronicles #1).epub");
        assert_eq!(r.title, "Dune");
        assert_eq!(r.series, "Dune Chronicles");
        assert_eq!(r.series_index, Some(1.0));
    }

    #[test]
    fn last_first_dash_title() {
        let r = g("Tolkien, J. R. R. - The Hobbit.epub");
        assert_eq!(r.authors, vec!["J. R. R. Tolkien"]);
        assert_eq!(r.title, "The Hobbit");
        let r = g("Pratchett, Terry & Gaiman, Neil - Good Omens.epub");
        assert_eq!(r.authors, vec!["Terry Pratchett", "Neil Gaiman"]);
    }

    #[test]
    fn multiple_authors() {
        let r = g("Neil Gaiman & Terry Pratchett - Good Omens.epub");
        assert_eq!(r.authors, vec!["Neil Gaiman", "Terry Pratchett"]);
    }

    #[test]
    fn strips_brackets_underscores_year_and_site_junk() {
        let r = g("[Fantasy] Ursula_K._Le_Guin_-_A_Wizard_of_Earthsea_(1968)_(z-lib.org).epub");
        assert_eq!(r.title, "A Wizard of Earthsea");
        assert_eq!(r.authors, vec!["Ursula K. Le Guin"]);
        let r = g("Isaac Asimov - Foundation - 1951.pdf");
        assert_eq!(r.title, "Foundation");
        let r = g("Author - Title (Z-Library).epub");
        assert_eq!(r.title, "Title");
        let r = g("Author - Title (libgen.li).epub");
        assert_eq!(r.title, "Title");
    }

    #[test]
    fn no_pattern_matches_uses_whole_name_as_title() {
        let r = g("Neuromancer.epub");
        assert_eq!(r.title, "Neuromancer");
        assert!(r.authors.is_empty());
    }

    #[test]
    fn bare_numbers_and_years_are_kept_as_titles() {
        assert_eq!(g("1984.epub").title, "1984");
        assert_eq!(g("Fahrenheit 451.epub").title, "Fahrenheit 451");
        assert_eq!(g("Blade Runner 2049.epub").title, "Blade Runner 2049");
        assert_eq!(g("(1999).epub").title, "(1999)");
    }

    #[test]
    fn track_number_is_not_an_author() {
        let r = g("01 - Introduction.epub");
        assert_eq!(r.title, "01 - Introduction");
        assert!(r.authors.is_empty());
    }

    #[test]
    fn hyphenated_names_need_spaced_dash() {
        let r = g("Jean-Paul Sartre - Nausea.epub");
        assert_eq!(r.authors, vec!["Jean-Paul Sartre"]);
        assert_eq!(r.title, "Nausea");
    }

    #[test]
    fn unicode_names() {
        let r = g("Gabriel García Márquez - Cien años de soledad.epub");
        assert_eq!(r.authors, vec!["Gabriel García Márquez"]);
        assert_eq!(r.title, "Cien años de soledad");
        let r = g("村上春樹 - ノルウェイの森.epub");
        assert_eq!(r.authors, vec!["村上春樹"]);
        assert_eq!(r.title, "ノルウェイの森");
    }

    #[test]
    fn path_components_are_ignored() {
        assert_eq!(g("/home/u/Books/A B - Some Title.epub").title, "Some Title");
        assert_eq!(g("C:\\Books\\A B - Some Title.epub").title, "Some Title");
    }

    #[test]
    fn unknown_extension_is_not_stripped() {
        assert_eq!(clean_stem("Mr. Smith"), "Mr. Smith");
        assert_eq!(clean_stem("Title.EPUB"), "Title");
    }

    #[test]
    fn dotted_names() {
        assert_eq!(
            clean_stem("Some.Author.-.Some.Title.epub"),
            "Some Author - Some Title"
        );
    }

    #[test]
    fn custom_template_order_matters() {
        let pats = [t("{title} - {author}")];
        let r = guess("The Hobbit - Tolkien.epub", &pats);
        assert_eq!(r.title, "The Hobbit");
        assert_eq!(r.authors, vec!["Tolkien"]);
        let r = guess(
            "A - B.epub",
            &[t("{author} - {title}"), t("{title} - {author}")],
        );
        assert_eq!((r.title.as_str(), r.authors[0].as_str()), ("B", "A"));
    }

    #[test]
    fn custom_regex_with_named_groups() {
        let p = Pattern::Regex(r"^(?P<series>[A-Za-z ]+?) (?P<index>\d+) (?P<title>.+)$".into());
        let r = guess("Discworld 12 Wee Free Men.epub", &[p]);
        assert_eq!(r.series, "Discworld");
        assert_eq!(r.series_index, Some(12.0));
        assert_eq!(r.title, "Wee Free Men");
    }

    #[test]
    fn custom_regex_author_group() {
        let p = Pattern::Regex(r"^(?P<author>[^_]+)__(?P<title>.+)$".into());
        let r = guess("Gaiman, Neil__Coraline.epub", &[p]);
        assert_eq!(r.authors, vec!["Neil Gaiman"]);
        assert_eq!(r.title, "Coraline");
    }

    #[test]
    fn invalid_patterns_are_skipped_and_reported_by_validate() {
        let bad = [
            t("{author} - {nope}"),
            t("{author}"),
            t("{title} {title}"),
            t("{title"),
            Pattern::Regex("(".into()),
            Pattern::Regex("^(?P<author>.+)$".into()),
        ];
        for p in &bad {
            assert!(p.validate().is_err(), "{p:?}");
        }
        let r = guess(
            "A - B.epub",
            &[bad[0].clone(), bad[4].clone(), t("{author} - {title}")],
        );
        assert_eq!(r.title, "B");
        assert!(t("{author} - {title}").validate().is_ok());
    }

    #[test]
    fn validate_error_messages_name_the_problem() {
        let e = t("{x} {title}").validate().unwrap_err().to_string();
        assert!(e.contains("{x}"), "{e}");
        let e = t("{author}").validate().unwrap_err().to_string();
        assert!(e.contains("no {title}"), "{e}");
        let e = Pattern::Regex("^x$".into())
            .validate()
            .unwrap_err()
            .to_string();
        assert!(e.contains("title"), "{e}");
    }

    #[test]
    fn skip_placeholder_ignores_part() {
        let r = guess(
            "2023 - Author - Title.epub",
            &[t("{skip} - {author} - {title}")],
        );
        assert_eq!(r.authors, vec!["Author"]);
        assert_eq!(r.title, "Title");
    }

    #[test]
    fn into_metadata_maps_fields() {
        let md = g("Terry Pratchett - Discworld #12 - The Wee Free Men.epub").into_metadata();
        assert_eq!(md.title, "The Wee Free Men");
        assert_eq!(md.authors, vec!["Terry Pratchett"]);
        assert_eq!(md.series, "Discworld");
        assert_eq!(md.series_index, 12.0);
        assert_eq!(g("X.epub").into_metadata().series_index, 0.0);
    }

    #[test]
    fn empty_and_weird_input_do_not_panic() {
        assert_eq!(g("").title, "");
        assert_eq!(g(".epub").title, "");
        assert_eq!(g(" - .epub").title, "");
        let long = "a".repeat(5000);
        assert_eq!(g(&long).title.len(), 5000);
        let _ = g(&format!("{} - {}.epub", "x ".repeat(2000), "y"));
    }
}
