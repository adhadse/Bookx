//! Book identifiers (L34): parse and format DOI, ASIN, Goodreads, Google Books, Open Library, Wikidata
//! and ISBN values, from bare ids, `scheme:value` strings and store URLs, plus URL builders.

use std::fmt;
use std::sync::OnceLock;

use bookx_core::opf::Identifier;
use bookx_core::util::percent_decode;
use regex::Regex;

use crate::isbn;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdKind {
    Isbn,
    Doi,
    Asin,
    Goodreads,
    GoogleBooks,
    OpenLibrary,
    Wikidata,
}

impl IdKind {
    /// The scheme string used in OPF identifiers and `scheme:value` text.
    pub fn scheme(self) -> &'static str {
        match self {
            IdKind::Isbn => "isbn",
            IdKind::Doi => "doi",
            IdKind::Asin => "asin",
            IdKind::Goodreads => "goodreads",
            IdKind::GoogleBooks => "google",
            IdKind::OpenLibrary => "olid",
            IdKind::Wikidata => "wikidata",
        }
    }

    pub fn from_scheme(s: &str) -> Option<IdKind> {
        Some(match s.trim().to_lowercase().as_str() {
            "isbn" | "isbn10" | "isbn13" | "isbn-10" | "isbn-13" => IdKind::Isbn,
            "doi" => IdKind::Doi,
            "asin" | "mobi-asin" | "amazon" | "kindle" => IdKind::Asin,
            "goodreads" | "gr" => IdKind::Goodreads,
            "google" | "googlebooks" | "google-books" | "gbooks" => IdKind::GoogleBooks,
            "olid" | "openlibrary" | "open-library" | "ol" => IdKind::OpenLibrary,
            "wikidata" | "qid" => IdKind::Wikidata,
            _ => return None,
        })
    }
}

/// A validated identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Id {
    pub kind: IdKind,
    pub value: String,
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.kind.scheme(), self.value)
    }
}

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).expect("static regex"))
}

fn doi_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"^10\.\d{4,9}(?:\.\d+)*/\S+$")
}

fn asin_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"^B0[A-Z0-9]{8}$")
}

fn google_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"^[A-Za-z0-9_-]{12}$")
}

fn ol_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"^OL\d+[MWA]$")
}

fn wikidata_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"^Q\d+$")
}

/// Validates and normalises `value` for `kind`.
pub fn validate(kind: IdKind, value: &str) -> Option<Id> {
    let v = value.trim();
    let value = match kind {
        IdKind::Isbn => {
            let n = isbn::normalize(v);
            if !isbn::is_valid(&n) {
                return None;
            }
            n
        }
        IdKind::Doi => {
            let mut v = v.trim_start_matches("doi:");
            loop {
                let t = v.trim_end_matches(['.', ',', ';']);
                let unbalanced = |open: char, close: char| {
                    t.ends_with(close) && t.matches(close).count() > t.matches(open).count()
                };
                if unbalanced('(', ')') || unbalanced('[', ']') || unbalanced('<', '>') {
                    v = &t[..t.len() - 1];
                } else if t.len() != v.len() {
                    v = t;
                } else {
                    break;
                }
            }
            if !doi_re().is_match(v) {
                return None;
            }
            v.to_string()
        }
        IdKind::Asin => {
            let u = v.to_uppercase();
            if asin_re().is_match(&u) || isbn::is_valid_isbn10(&u) {
                u
            } else {
                return None;
            }
        }
        IdKind::Goodreads => {
            if v.is_empty() || v.len() > 12 || !v.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            v.to_string()
        }
        IdKind::GoogleBooks => {
            if !google_re().is_match(v) {
                return None;
            }
            v.to_string()
        }
        IdKind::OpenLibrary => {
            let u = v.to_uppercase();
            if !ol_re().is_match(&u) {
                return None;
            }
            u
        }
        IdKind::Wikidata => {
            let u = v.to_uppercase();
            if !wikidata_re().is_match(&u) {
                return None;
            }
            u
        }
    };
    Some(Id { kind, value })
}

fn parse_url(s: &str) -> Option<Id> {
    let u = url::Url::parse(s).ok()?;
    if !matches!(u.scheme(), "http" | "https") {
        return None;
    }
    let host = u.host_str()?.to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    let path = u.path();
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if host == "doi.org" || host == "dx.doi.org" {
        return validate(IdKind::Doi, &percent_decode(path.trim_start_matches('/')));
    }
    if host.starts_with("amazon.") || host.contains(".amazon.") {
        for (i, seg) in segs.iter().enumerate() {
            if matches!(*seg, "dp" | "product" | "ASIN") {
                if let Some(next) = segs.get(i + 1) {
                    return validate(IdKind::Asin, next);
                }
            }
        }
        return None;
    }
    if host == "goodreads.com" {
        if let Some(i) = segs
            .windows(2)
            .position(|w| w == ["book", "show"])
            .map(|i| i + 1)
        {
            let id: String = segs
                .get(i + 1)?
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            return validate(IdKind::Goodreads, &id);
        }
        return None;
    }
    if host.starts_with("books.google.") || host == "google.com" || host == "play.google.com" {
        let id = u
            .query_pairs()
            .find(|(k, _)| k == "id")
            .map(|(_, v)| v.into_owned())?;
        return validate(IdKind::GoogleBooks, &id);
    }
    if host == "openlibrary.org" {
        let i = segs
            .iter()
            .position(|s| matches!(*s, "works" | "books" | "authors"))?;
        return validate(IdKind::OpenLibrary, segs.get(i + 1)?);
    }
    if host == "wikidata.org" {
        let last = segs.last()?;
        return validate(IdKind::Wikidata, last);
    }
    None
}

/// Parses any of: store/library URLs, `urn:isbn:...`, `scheme:value`, or a bare value
/// (DOI, ISBN, `B0…` ASIN, `OL…M/W/A`).
pub fn parse(input: &str) -> Option<Id> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    if s.starts_with("http://") || s.starts_with("https://") {
        return parse_url(s);
    }
    let s = s.strip_prefix("urn:").unwrap_or(s);
    if let Some((scheme, rest)) = s.split_once(':') {
        if let Some(kind) = IdKind::from_scheme(scheme) {
            return validate(kind, rest);
        }
    }
    if let Some(id) = validate(IdKind::Doi, s) {
        return Some(id);
    }
    if isbn::is_valid(&isbn::normalize(s)) {
        return validate(IdKind::Isbn, s);
    }
    if asin_re().is_match(&s.to_uppercase()) {
        return validate(IdKind::Asin, s);
    }
    if ol_re().is_match(&s.to_uppercase()) {
        return validate(IdKind::OpenLibrary, s);
    }
    None
}

fn encode_path(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'~'
            | b'/'
            | b':'
            | b'('
            | b')'
            | b';'
            | b',' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

impl Id {
    /// Page of this identifier on the web.
    pub fn url(&self) -> String {
        let v = &self.value;
        match self.kind {
            IdKind::Isbn => format!("https://openlibrary.org/isbn/{v}"),
            IdKind::Doi => format!("https://doi.org/{}", encode_path(v)),
            IdKind::Asin => format!("https://www.amazon.com/dp/{v}"),
            IdKind::Goodreads => format!("https://www.goodreads.com/book/show/{v}"),
            IdKind::GoogleBooks => format!("https://books.google.com/books?id={v}"),
            IdKind::OpenLibrary => {
                let dir = match v.chars().last() {
                    Some('W') => "works",
                    Some('A') => "authors",
                    _ => "books",
                };
                format!("https://openlibrary.org/{dir}/{v}")
            }
            IdKind::Wikidata => format!("https://www.wikidata.org/wiki/{v}"),
        }
    }

    pub fn to_identifier(&self) -> Identifier {
        Identifier {
            scheme: self.kind.scheme().to_string(),
            value: self.value.clone(),
            xml_id: None,
        }
    }

    /// Validates an OPF identifier (scheme + value). `None` for unknown schemes or invalid values.
    pub fn from_identifier(i: &Identifier) -> Option<Id> {
        match IdKind::from_scheme(&i.scheme) {
            Some(kind) => validate(kind, &i.value),
            None if i.scheme.is_empty() => parse(&i.value),
            None => None,
        }
    }
}

/// All recognisable identifiers of a metadata record.
pub fn ids_of(identifiers: &[Identifier]) -> Vec<Id> {
    let mut out: Vec<Id> = Vec::new();
    for i in identifiers {
        if let Some(id) = Id::from_identifier(i) {
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Id {
        parse(s).unwrap_or_else(|| panic!("`{s}` should parse"))
    }

    #[test]
    fn schemes_roundtrip() {
        for k in [
            IdKind::Isbn,
            IdKind::Doi,
            IdKind::Asin,
            IdKind::Goodreads,
            IdKind::GoogleBooks,
            IdKind::OpenLibrary,
            IdKind::Wikidata,
        ] {
            assert_eq!(IdKind::from_scheme(k.scheme()), Some(k));
        }
        assert_eq!(IdKind::from_scheme("MOBI-ASIN"), Some(IdKind::Asin));
        assert_eq!(IdKind::from_scheme("nonsense"), None);
    }

    #[test]
    fn parses_bare_values() {
        assert_eq!(
            p("10.1000/xyz123"),
            Id {
                kind: IdKind::Doi,
                value: "10.1000/xyz123".into()
            }
        );
        assert_eq!(p("978-0-306-40615-7").kind, IdKind::Isbn);
        assert_eq!(p("0-306-40615-2").value, "0306406152");
        assert_eq!(p("b00jxqzuxy").kind, IdKind::Asin);
        assert_eq!(p("B00JXQZUXY").value, "B00JXQZUXY");
        assert_eq!(p("OL7353617M").kind, IdKind::OpenLibrary);
        assert_eq!(p("ol45883w").value, "OL45883W");
    }

    #[test]
    fn parses_scheme_prefixed() {
        assert_eq!(p("doi:10.1000/abc").kind, IdKind::Doi);
        assert_eq!(p("urn:isbn:9780306406157").kind, IdKind::Isbn);
        assert_eq!(p("isbn: 978 0 306 40615 7").value, "9780306406157");
        assert_eq!(p("goodreads:12345").value, "12345");
        assert_eq!(p("mobi-asin:B00JXQZUXY").kind, IdKind::Asin);
        assert_eq!(p("google:zyTCAlFPjgYC").kind, IdKind::GoogleBooks);
        assert_eq!(p("wikidata:q42").value, "Q42");
        assert_eq!(p("olid:OL1M").kind, IdKind::OpenLibrary);
    }

    #[test]
    fn parses_urls() {
        assert_eq!(
            p("https://doi.org/10.1000/xyz%2F123").value,
            "10.1000/xyz/123"
        );
        assert_eq!(
            p("http://dx.doi.org/10.1038/nature12373").value,
            "10.1038/nature12373"
        );
        assert_eq!(
            p("https://www.amazon.com/Some-Title/dp/B00JXQZUXY/ref=sr_1_1?x=1").value,
            "B00JXQZUXY"
        );
        assert_eq!(
            p("https://www.amazon.de/gp/product/0306406152").value,
            "0306406152"
        );
        assert_eq!(
            p("https://www.goodreads.com/book/show/5907.The_Hobbit").value,
            "5907"
        );
        assert_eq!(
            p("https://www.goodreads.com/book/show/2767052-the-hunger-games").value,
            "2767052"
        );
        assert_eq!(
            p("https://books.google.com/books?id=zyTCAlFPjgYC&hl=en").value,
            "zyTCAlFPjgYC"
        );
        assert_eq!(
            p("https://openlibrary.org/works/OL45883W/The_Hobbit").value,
            "OL45883W"
        );
        assert_eq!(
            p("https://openlibrary.org/books/OL7353617M").kind,
            IdKind::OpenLibrary
        );
        assert_eq!(p("https://www.wikidata.org/wiki/Q42").value, "Q42");
    }

    #[test]
    fn rejects_invalid() {
        for bad in [
            "",
            "   ",
            "hello",
            "10.12/short",
            "isbn:0306406153",
            "goodreads:abc",
            "goodreads:",
            "google:tooShort",
            "asin:123",
            "olid:OL1X",
            "wikidata:42",
            "https://example.com/x",
            "https://www.goodreads.com/author/show/1",
            "https://www.amazon.com/",
            "https://books.google.com/books",
            "https://openlibrary.org/",
            "ftp://doi.org/10.1000/abc",
            "unknown:thing",
        ] {
            assert!(parse(bad).is_none(), "`{bad}` must not parse");
        }
    }

    #[test]
    fn doi_trailing_punctuation_is_trimmed() {
        assert_eq!(p("doi:10.1000/abc.").value, "10.1000/abc");
        assert_eq!(p("10.1000/abc),").value, "10.1000/abc");
    }

    #[test]
    fn urls_for_each_kind() {
        assert_eq!(
            p("0306406152").url(),
            "https://openlibrary.org/isbn/0306406152"
        );
        assert_eq!(
            p("10.1000/a<b>c").url(),
            "https://doi.org/10.1000/a%3Cb%3Ec"
        );
        assert_eq!(p("10.1000/abc(1)").url(), "https://doi.org/10.1000/abc(1)");
        assert_eq!(
            p("B00JXQZUXY").url(),
            "https://www.amazon.com/dp/B00JXQZUXY"
        );
        assert_eq!(
            p("goodreads:5907").url(),
            "https://www.goodreads.com/book/show/5907"
        );
        assert_eq!(
            p("google:zyTCAlFPjgYC").url(),
            "https://books.google.com/books?id=zyTCAlFPjgYC"
        );
        assert_eq!(
            p("OL45883W").url(),
            "https://openlibrary.org/works/OL45883W"
        );
        assert_eq!(
            p("OL7353617M").url(),
            "https://openlibrary.org/books/OL7353617M"
        );
        assert_eq!(
            p("OL26320A").url(),
            "https://openlibrary.org/authors/OL26320A"
        );
        assert_eq!(p("wikidata:Q42").url(), "https://www.wikidata.org/wiki/Q42");
    }

    #[test]
    fn url_roundtrips_through_parse() {
        for s in [
            "0306406152",
            "10.1000/xyz",
            "B00JXQZUXY",
            "goodreads:5907",
            "google:zyTCAlFPjgYC",
            "OL45883W",
            "wikidata:Q42",
        ] {
            let id = p(s);
            if id.kind != IdKind::Isbn {
                assert_eq!(parse(&id.url()), Some(id.clone()), "{s}");
            }
        }
    }

    #[test]
    fn display_and_opf_conversion() {
        let id = p("978-0-306-40615-7");
        assert_eq!(id.to_string(), "isbn:9780306406157");
        let opf = id.to_identifier();
        assert_eq!(
            (opf.scheme.as_str(), opf.value.as_str()),
            ("isbn", "9780306406157")
        );
        assert_eq!(Id::from_identifier(&opf), Some(id));
        // legacy scheme names and unscoped values
        let legacy = Identifier {
            scheme: "MOBI-ASIN".into(),
            value: "B00JXQZUXY".into(),
            xml_id: None,
        };
        assert_eq!(
            Id::from_identifier(&legacy).map(|i| i.kind),
            Some(IdKind::Asin)
        );
        let bare = Identifier {
            scheme: String::new(),
            value: "10.1000/abc".into(),
            xml_id: None,
        };
        assert_eq!(
            Id::from_identifier(&bare).map(|i| i.kind),
            Some(IdKind::Doi)
        );
        let unknown = Identifier {
            scheme: "uuid".into(),
            value: "x".into(),
            xml_id: None,
        };
        assert_eq!(Id::from_identifier(&unknown), None);
        let invalid = Identifier {
            scheme: "isbn".into(),
            value: "123".into(),
            xml_id: None,
        };
        assert_eq!(Id::from_identifier(&invalid), None);
    }

    #[test]
    fn ids_of_dedupes_and_skips_unknown() {
        let list = vec![
            Identifier {
                scheme: "isbn".into(),
                value: "0-306-40615-2".into(),
                xml_id: None,
            },
            Identifier {
                scheme: "isbn".into(),
                value: "0306406152".into(),
                xml_id: None,
            },
            Identifier {
                scheme: "uuid".into(),
                value: "abc".into(),
                xml_id: Some("bookid".into()),
            },
            Identifier {
                scheme: "doi".into(),
                value: "10.1000/x".into(),
                xml_id: None,
            },
        ];
        let ids = ids_of(&list);
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0].kind, IdKind::Isbn);
        assert_eq!(ids[1].kind, IdKind::Doi);
    }

    #[test]
    fn validate_directly() {
        assert!(
            validate(IdKind::Goodreads, "1234567890123").is_none(),
            "too long"
        );
        assert!(
            validate(IdKind::Asin, "0306406152").is_some(),
            "ISBN-10 is a valid ASIN"
        );
        assert!(validate(IdKind::Isbn, "x").is_none());
    }
}
