//! Citations and bibliographies from the library's metadata (PP-G-15 "plain-file export", PP-G-17).
//!
//! BibTeX, RIS, CSL JSON and APA 7 text for a list of books. No network, no GTK: the window is
//! `src/ui/citations.rs`, the command is `bookx-cli library cite`.

use bookx_core::library::Book;
use bookx_core::opf::Metadata;
use serde_json::json;

use crate::authors::{initials, parse_name, split_authors};
use crate::isbn;
use crate::text::fold;

/// The output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    BibTeX,
    Ris,
    CslJson,
    Apa,
}

impl Style {
    pub const ALL: [Style; 4] = [Style::BibTeX, Style::Ris, Style::CslJson, Style::Apa];

    pub fn parse(s: &str) -> Option<Style> {
        match s.trim().to_ascii_lowercase().as_str() {
            "bibtex" | "bib" => Some(Style::BibTeX),
            "ris" => Some(Style::Ris),
            "csl" | "csl-json" | "csljson" | "json" => Some(Style::CslJson),
            "apa" | "text" | "txt" => Some(Style::Apa),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Style::BibTeX => "BibTeX",
            Style::Ris => "RIS",
            Style::CslJson => "CSL JSON",
            Style::Apa => "APA 7 (text)",
        }
    }

    /// File extension for a saved bibliography.
    pub fn extension(self) -> &'static str {
        match self {
            Style::BibTeX => "bib",
            Style::Ris => "ris",
            Style::CslJson => "json",
            Style::Apa => "txt",
        }
    }
}

/// The year in a published date (`2021-03-14`, `2021`, `March 2021`), empty when there is none.
pub fn year_of(published: &str) -> String {
    let digits: Vec<char> = published.chars().collect();
    for w in digits.windows(4) {
        if w.iter().all(|c| c.is_ascii_digit()) {
            return w.iter().collect();
        }
    }
    String::new()
}

fn isbn_of(m: &Metadata) -> String {
    m.identifiers
        .iter()
        .filter(|i| i.scheme.eq_ignore_ascii_case("isbn"))
        .find_map(|i| isbn::canonical(&i.value))
        .unwrap_or_default()
}

fn doi_of(m: &Metadata) -> String {
    m.identifiers
        .iter()
        .find(|i| i.scheme.eq_ignore_ascii_case("doi"))
        .map(|i| i.value.trim().to_string())
        .unwrap_or_default()
}

/// The people of a metadata record, one entry per person.
fn people(m: &Metadata) -> Vec<String> {
    m.authors
        .iter()
        .flat_map(|a| split_authors(a))
        .filter(|a| !a.trim().is_empty())
        .collect()
}

fn bibtex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\textbackslash{}"),
            '&' | '%' | '$' | '#' | '_' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            '~' => out.push_str("\\textasciitilde{}"),
            '^' => out.push_str("\\textasciicircum{}"),
            '\n' | '\r' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// A BibTeX key such as `herbert1965dune`: last name of the first author, year, first title word.
pub fn cite_key(m: &Metadata) -> String {
    let first = people(m)
        .first()
        .map(|a| parse_name(a).last)
        .unwrap_or_default();
    let ascii = |s: &str| -> String {
        fold(s)
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
    };
    let word = m
        .title
        .split_whitespace()
        .map(ascii)
        .find(|w| w.len() > 3 && !matches!(w.as_str(), "the" | "with" | "from"))
        .or_else(|| {
            m.title
                .split_whitespace()
                .map(ascii)
                .find(|w| !w.is_empty())
        })
        .unwrap_or_default();
    let key = format!("{}{}{}", ascii(&first), year_of(&m.published), word);
    if key.is_empty() { "book".into() } else { key }
}

/// Keys that are unique within one list (`a`, `b`, … appended to repeats).
fn unique_keys(metas: &[&Metadata]) -> Vec<String> {
    let mut seen: std::collections::HashMap<String, usize> = Default::default();
    metas
        .iter()
        .map(|m| {
            let k = cite_key(m);
            let n = seen.entry(k.clone()).or_insert(0);
            *n += 1;
            if *n == 1 {
                k
            } else {
                format!("{k}{}", (b'a' + (*n as u8 - 2) % 26) as char)
            }
        })
        .collect()
}

pub fn bibtex(metas: &[&Metadata]) -> String {
    let keys = unique_keys(metas);
    let mut out = String::new();
    for (m, key) in metas.iter().zip(keys) {
        out.push_str(&format!("@book{{{key},\n"));
        let authors: Vec<String> = people(m)
            .iter()
            .map(|a| parse_name(a).last_first())
            .collect();
        let field = |out: &mut String, name: &str, value: String| {
            if !value.trim().is_empty() {
                out.push_str(&format!(
                    "  {name} = {{{}}},\n",
                    bibtex_escape(value.trim())
                ));
            }
        };
        field(&mut out, "author", authors.join(" and "));
        // a second pair of braces keeps the capitals of the title
        if !m.title.trim().is_empty() {
            out.push_str(&format!(
                "  title = {{{{{}}}}},\n",
                bibtex_escape(m.title.trim())
            ));
        }
        field(&mut out, "publisher", m.publisher.clone());
        field(&mut out, "year", year_of(&m.published));
        field(&mut out, "isbn", isbn_of(m));
        field(&mut out, "doi", doi_of(m));
        field(&mut out, "language", m.language.clone());
        field(&mut out, "series", m.series.clone());
        if !m.series.is_empty() && m.series_index > 0.0 {
            field(&mut out, "number", format!("{}", m.series_index));
        }
        out.push_str("}\n\n");
    }
    out
}

pub fn ris(metas: &[&Metadata]) -> String {
    let mut out = String::new();
    for m in metas {
        let mut line = |tag: &str, value: &str| {
            if !value.trim().is_empty() {
                out.push_str(&format!(
                    "{tag}  - {}\r\n",
                    value.trim().replace(['\r', '\n'], " ")
                ));
            }
        };
        line("TY", "BOOK");
        for a in people(m) {
            line("AU", &parse_name(&a).last_first());
        }
        line("TI", &m.title);
        line("PB", &m.publisher);
        line("PY", &year_of(&m.published));
        line("SN", &isbn_of(m));
        line("DO", &doi_of(m));
        line("LA", &m.language);
        line("T2", &m.series);
        for t in &m.tags {
            line("KW", t);
        }
        out.push_str("ER  - \r\n\r\n");
    }
    out
}

pub fn csl_json(metas: &[&Metadata]) -> String {
    let keys = unique_keys(metas);
    let items: Vec<serde_json::Value> = metas
        .iter()
        .zip(keys)
        .map(|(m, key)| {
            let mut o = serde_json::Map::new();
            o.insert("id".into(), json!(key));
            o.insert("type".into(), json!("book"));
            o.insert("title".into(), json!(m.title));
            let authors: Vec<_> = people(m)
                .iter()
                .map(|a| {
                    let n = parse_name(a);
                    if n.first.is_empty() {
                        json!({ "literal": n.last })
                    } else {
                        json!({ "family": n.last, "given": n.first })
                    }
                })
                .collect();
            if !authors.is_empty() {
                o.insert("author".into(), json!(authors));
            }
            let mut put = |k: &str, v: String| {
                if !v.is_empty() {
                    o.insert(k.into(), json!(v));
                }
            };
            put("publisher", m.publisher.clone());
            put("ISBN", isbn_of(m));
            put("DOI", doi_of(m));
            put("language", m.language.clone());
            put("collection-title", m.series.clone());
            if !m.series.is_empty() && m.series_index > 0.0 {
                o.insert(
                    "collection-number".into(),
                    json!(format!("{}", m.series_index)),
                );
            }
            if let Ok(y) = year_of(&m.published).parse::<i32>() {
                o.insert("issued".into(), json!({ "date-parts": [[y]] }));
            }
            serde_json::Value::Object(o)
        })
        .collect();
    serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".into()) + "\n"
}

/// `Herbert, F.` for APA.
fn apa_person(a: &str) -> String {
    let n = parse_name(a);
    if n.first.is_empty() {
        return n.last;
    }
    let ini: Vec<String> = n
        .first
        .split(|c: char| c.is_whitespace())
        .filter(|w| !w.trim_matches('.').is_empty())
        .flat_map(|w| {
            let w = w.trim_matches('.');
            if w.contains('-') {
                w.split('-')
                    .filter_map(|p| p.chars().next())
                    .map(|c| format!("{}.", c.to_uppercase()))
                    .collect::<Vec<_>>()
            } else if (2..=3).contains(&w.chars().count()) && w.chars().all(|c| c.is_uppercase()) {
                w.chars().map(|c| format!("{c}.")).collect()
            } else if w.contains('.') {
                initials(w)
                    .into_iter()
                    .map(|c| format!("{}.", c.to_uppercase()))
                    .collect()
            } else {
                w.chars()
                    .next()
                    .map(|c| vec![format!("{}.", c.to_uppercase())])
                    .unwrap_or_default()
            }
        })
        .collect();
    let mut s = format!("{}, {}", n.last, ini.join(" "));
    if !n.suffix.is_empty() {
        s.push(' ');
        s.push_str(&n.suffix);
    }
    s
}

/// One APA 7 reference; the title is wrapped in `*` for italics.
pub fn apa_one(m: &Metadata) -> String {
    let names: Vec<String> = people(m).iter().map(|a| apa_person(a)).collect();
    let who = match names.len() {
        0 => String::new(),
        1 => names[0].clone(),
        2 => format!("{}, & {}", names[0], names[1]),
        n if n <= 20 => format!("{}, & {}", names[..n - 1].join(", "), names[n - 1]),
        _ => format!("{}, ... {}", names[..19].join(", "), names[names.len() - 1]),
    };
    let year = year_of(&m.published);
    let when = if year.is_empty() {
        "n.d.".to_string()
    } else {
        year
    };
    let mut s = String::new();
    if !who.is_empty() {
        s.push_str(&who);
        if !who.ends_with('.') {
            s.push('.');
        }
        s.push(' ');
    }
    s.push_str(&format!("({when}). "));
    let title = m.title.trim();
    s.push_str(&format!("*{title}*"));
    if !title.ends_with(['.', '?', '!']) {
        s.push('.');
    }
    if !m.publisher.trim().is_empty() {
        s.push_str(&format!(" {}.", m.publisher.trim()));
    }
    let doi = doi_of(m);
    if !doi.is_empty() {
        let url = if doi.starts_with("http") {
            doi
        } else {
            format!("https://doi.org/{doi}")
        };
        s.push(' ');
        s.push_str(&url);
    }
    s
}

pub fn apa(metas: &[&Metadata]) -> String {
    let mut lines: Vec<String> = metas.iter().map(|m| apa_one(m)).collect();
    // APA lists references alphabetically
    lines.sort_by_key(|l| fold(l));
    let mut out = lines.join("\n\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// The bibliography of `metas` in `style`.
pub fn render(metas: &[&Metadata], style: Style) -> String {
    match style {
        Style::BibTeX => bibtex(metas),
        Style::Ris => ris(metas),
        Style::CslJson => csl_json(metas),
        Style::Apa => apa(metas),
    }
}

/// The bibliography of library books.
pub fn render_books(books: &[&Book], style: Style) -> String {
    let metas: Vec<&Metadata> = books.iter().map(|b| &b.meta).collect();
    render(&metas, style)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_core::opf::Identifier;

    fn dune() -> Metadata {
        Metadata {
            title: "Dune".into(),
            authors: vec!["Frank Herbert".into()],
            publisher: "Chilton Books".into(),
            published: "1965-08-01".into(),
            language: "en".into(),
            series: "Dune Chronicles".into(),
            series_index: 1.0,
            identifiers: vec![Identifier {
                scheme: "isbn".into(),
                value: "978-0-441-17271-9".into(),
                xml_id: None,
            }],
            tags: vec!["sci-fi".into(), "classic".into()],
            ..Default::default()
        }
    }

    fn two_authors() -> Metadata {
        Metadata {
            title: "Good Omens & the Rest".into(),
            authors: vec!["Neil Gaiman".into(), "Terry Pratchett".into()],
            published: "1990".into(),
            ..Default::default()
        }
    }

    #[test]
    fn year_is_found_in_any_date_shape() {
        assert_eq!(year_of("1965-08-01"), "1965");
        assert_eq!(year_of("March 2021"), "2021");
        assert_eq!(year_of("n/a"), "");
        assert_eq!(year_of("123"), "");
    }

    #[test]
    fn bibtex_has_key_authors_and_escapes() {
        let (a, b) = (dune(), two_authors());
        let t = bibtex(&[&a, &b]);
        assert!(t.contains("@book{herbert1965dune,"));
        assert!(t.contains("author = {Herbert, Frank}"));
        assert!(t.contains("isbn = {9780441172719}"));
        assert!(t.contains("series = {Dune Chronicles}"));
        assert!(t.contains("number = {1}"));
        assert!(t.contains("author = {Gaiman, Neil and Pratchett, Terry}"));
        assert!(t.contains("title = {{Good Omens \\& the Rest}}"), "{t}");
    }

    #[test]
    fn repeated_keys_get_a_letter() {
        let a = dune();
        let t = bibtex(&[&a, &a, &a]);
        assert!(t.contains("@book{herbert1965dune,"));
        assert!(t.contains("@book{herbert1965dunea,"));
        assert!(t.contains("@book{herbert1965duneb,"));
    }

    #[test]
    fn key_is_ascii_without_stop_words() {
        let m = Metadata {
            title: "The Éclair Book".into(),
            authors: vec!["José de la Cruz".into()],
            published: "2001".into(),
            ..Default::default()
        };
        assert_eq!(cite_key(&m), "delacruz2001eclair");
        assert_eq!(cite_key(&Metadata::default()), "book");
    }

    #[test]
    fn ris_uses_one_tag_per_line() {
        let a = dune();
        let t = ris(&[&a]);
        assert!(t.starts_with("TY  - BOOK\r\n"));
        assert!(t.contains("AU  - Herbert, Frank\r\n"));
        assert!(t.contains("PY  - 1965\r\n"));
        assert!(t.contains("KW  - classic\r\n"));
        assert!(t.contains("ER  - \r\n"));
    }

    #[test]
    fn csl_json_parses_back() {
        let a = dune();
        let b = Metadata {
            title: "Plato".into(),
            authors: vec!["Plato".into()],
            ..Default::default()
        };
        let v: serde_json::Value = serde_json::from_str(&csl_json(&[&a, &b])).unwrap();
        assert_eq!(v[0]["type"], "book");
        assert_eq!(v[0]["author"][0]["family"], "Herbert");
        assert_eq!(v[0]["author"][0]["given"], "Frank");
        assert_eq!(v[0]["issued"]["date-parts"][0][0], 1965);
        assert_eq!(v[0]["collection-number"], "1");
        assert_eq!(v[1]["author"][0]["literal"], "Plato");
        assert!(v[1].get("issued").is_none());
    }

    #[test]
    fn apa_formats_authors_year_and_title() {
        assert_eq!(
            apa_one(&dune()),
            "Herbert, F. (1965). *Dune*. Chilton Books."
        );
        assert_eq!(
            apa_one(&two_authors()),
            "Gaiman, N., & Pratchett, T. (1990). *Good Omens & the Rest*."
        );
        let anon = Metadata {
            title: "Beowulf".into(),
            ..Default::default()
        };
        assert_eq!(apa_one(&anon), "(n.d.). *Beowulf*.");
        let initials = Metadata {
            title: "Hobbit".into(),
            authors: vec!["J. R. R. Tolkien".into()],
            published: "1937".into(),
            ..Default::default()
        };
        assert_eq!(apa_one(&initials), "Tolkien, J. R. R. (1937). *Hobbit*.");
    }

    #[test]
    fn apa_list_is_alphabetical() {
        let (a, b) = (dune(), two_authors());
        let t = apa(&[&b, &a]);
        assert!(t.find("Gaiman").unwrap() < t.find("Herbert").unwrap());
    }

    #[test]
    fn styles_parse_by_name() {
        assert_eq!(Style::parse("BibTeX"), Some(Style::BibTeX));
        assert_eq!(Style::parse("csl-json"), Some(Style::CslJson));
        assert_eq!(Style::parse("nope"), None);
        assert_eq!(Style::parse("ris"), Some(Style::Ris));
        assert_eq!(Style::Apa.extension(), "txt");
    }

    #[test]
    fn empty_list_gives_empty_output() {
        assert_eq!(render(&[], Style::BibTeX), "");
        assert_eq!(render(&[], Style::Apa), "");
        assert_eq!(render(&[], Style::CslJson).trim(), "[]");
    }
}
