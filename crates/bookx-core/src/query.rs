//! Calibre-style search expressions.
//!
//! Examples: `tolkien`, `author:"le guin" tag:fantasy`, `rating:>=4 -format:pdf`,
//! `series:=Discworld OR status:reading`. `weak:` finds books with poor metadata (no author, a title
//! like a file name, no cover, no language), see [`super::quality`].

use super::library::Book;

#[derive(Debug, Clone, PartialEq)]
enum Op {
    Contains,
    Exact,
    Gt,
    Ge,
    Lt,
    Le,
}

#[derive(Debug, Clone, PartialEq)]
struct Term {
    field: Option<String>,
    op: Op,
    value: String,
    negate: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Query {
    groups: Vec<Vec<Term>>,
}

fn tokenize(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    for c in input.chars() {
        match c {
            '"' => in_q = !in_q,
            c if c.is_whitespace() && !in_q => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

impl Query {
    pub fn parse(input: &str) -> Query {
        let mut groups: Vec<Vec<Term>> = vec![Vec::new()];
        for tok in tokenize(input) {
            if tok == "OR" {
                groups.push(Vec::new());
                continue;
            }
            let (negate, tok) = match tok.strip_prefix('-') {
                Some(rest) if !rest.is_empty() => (true, rest.to_string()),
                _ => (false, tok),
            };
            let (field, rest) = match tok.split_once(':') {
                Some((f, r)) if is_field(f) => (Some(canonical_field(f)), r.to_string()),
                _ => (None, tok),
            };
            let (op, value) = if let Some(v) = rest.strip_prefix(">=") {
                (Op::Ge, v)
            } else if let Some(v) = rest.strip_prefix("<=") {
                (Op::Le, v)
            } else if let Some(v) = rest.strip_prefix('>') {
                (Op::Gt, v)
            } else if let Some(v) = rest.strip_prefix('<') {
                (Op::Lt, v)
            } else if let Some(v) = rest.strip_prefix('=') {
                (Op::Exact, v)
            } else {
                (Op::Contains, rest.as_str())
            };
            groups.last_mut().unwrap().push(Term {
                field,
                op,
                value: value.to_lowercase(),
                negate,
            });
        }
        groups.retain(|g| !g.is_empty());
        Query { groups }
    }

    pub fn matches(&self, b: &Book) -> bool {
        if self.groups.is_empty() {
            return true;
        }
        self.groups
            .iter()
            .any(|g| g.iter().all(|t| t.matches(b) != t.negate))
    }
}

fn is_field(f: &str) -> bool {
    matches!(
        f.to_ascii_lowercase().as_str(),
        "title"
            | "author"
            | "authors"
            | "tag"
            | "tags"
            | "series"
            | "publisher"
            | "language"
            | "lang"
            | "format"
            | "rating"
            | "identifier"
            | "identifiers"
            | "isbn"
            | "comments"
            | "status"
            | "progress"
            | "size"
            | "weak"
            | "drm"
            | "audiobook"
    )
}

fn canonical_field(f: &str) -> String {
    match f.to_ascii_lowercase().as_str() {
        "authors" => "author",
        "tags" => "tag",
        "lang" => "language",
        "identifiers" => "identifier",
        other => return other.to_string(),
    }
    .to_string()
}

fn text_match(hay: &str, term: &Term) -> bool {
    let h = hay.to_lowercase();
    match term.op {
        Op::Exact => h == term.value,
        _ => h.contains(&term.value),
    }
}

fn num_match(v: f64, term: &Term) -> bool {
    let Ok(t) = term.value.parse::<f64>() else {
        return false;
    };
    match term.op {
        Op::Gt => v > t,
        Op::Ge => v >= t,
        Op::Lt => v < t,
        Op::Le => v <= t,
        _ => (v - t).abs() < 1e-9,
    }
}

impl Term {
    fn matches(&self, b: &Book) -> bool {
        let m = &b.meta;
        match self.field.as_deref() {
            None => {
                text_match(&m.title, self)
                    || m.authors.iter().any(|a| text_match(a, self))
                    || m.tags.iter().any(|t| text_match(t, self))
                    || text_match(&m.series, self)
                    || text_match(&m.publisher, self)
            }
            Some("title") => text_match(&m.title, self),
            Some("author") => m.authors.iter().any(|a| text_match(a, self)),
            Some("tag") => {
                if self.value.is_empty() {
                    !m.tags.is_empty()
                } else {
                    m.tags.iter().any(|t| text_match(t, self))
                }
            }
            Some("series") => text_match(&m.series, self),
            Some("publisher") => text_match(&m.publisher, self),
            Some("language") => text_match(&m.language, self),
            Some("format") => {
                text_match(b.format(), self)
                    || b.all_formats().iter().any(|f| text_match(&f.format, self))
            }
            Some("comments") => text_match(&m.description, self),
            Some("identifier") | Some("isbn") => m.identifiers.iter().any(|i| {
                text_match(&i.value, self) || text_match(&format!("{}:{}", i.scheme, i.value), self)
            }),
            Some("rating") => num_match(m.rating as f64 / 2.0, self),
            Some("progress") => num_match(b.progress * 100.0, self),
            Some("size") => num_match(b.size as f64 / 1_048_576.0, self),
            Some("weak") => super::quality::is_weak(b, &self.value),
            // `drm:` finds protected books, `drm:kindle` those of one scheme
            Some("drm") => {
                if self.value.is_empty() {
                    !b.drm.is_empty()
                } else {
                    text_match(&b.drm, self)
                }
            }
            Some("audiobook") => b
                .resources
                .iter()
                .any(|r| r.kind == crate::formats::AUDIOBOOK),
            Some("status") => {
                let s = b.status();
                s == self.value || (self.op == Op::Contains && s.starts_with(&self.value))
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_terms() {
        let q = Query::parse(r#"author:"le guin" -tag:horror OR rating:>=4"#);
        assert_eq!(q.groups.len(), 2);
        assert_eq!(q.groups[0].len(), 2);
        assert!(q.groups[0][1].negate);
        assert_eq!(q.groups[1][0].op, Op::Ge);
    }

    #[test]
    fn weak_finds_books_with_poor_metadata() {
        let mut b: Book = serde_json::from_str(
            r#"{"id":1,"file":"a.epub","meta":{"title":"Dune","title_sort":"Dune","authors":["Frank Herbert"],"author_sort":"Herbert, Frank","publisher":"","language":"en","description":"","tags":[],"series":"","series_index":0.0,"identifiers":[],"published":"","rights":"","rating":0},"added":"2026-01-01T00:00:00Z","modified":"2026-01-01T00:00:00Z","size":1,"cover":"c.jpg"}"#,
        )
        .unwrap();
        assert!(!Query::parse("weak:").matches(&b));
        assert!(Query::parse("-weak:").matches(&b));
        b.cover = None;
        assert!(Query::parse("weak:").matches(&b));
        assert!(Query::parse("weak:cover").matches(&b));
        assert!(!Query::parse("weak:author").matches(&b));
    }

    #[test]
    fn rating_and_tag_terms_find_rated_and_tagged_books() {
        let mut b: Book = serde_json::from_str(
            r#"{"id":1,"file":"a.epub","meta":{"title":"Dune","title_sort":"Dune","authors":["F"],"author_sort":"F","publisher":"","language":"en","description":"","tags":["Sci-Fi","classic"],"series":"","series_index":0.0,"identifiers":[],"published":"","rights":"","rating":0},"added":"2026-01-01T00:00:00Z","modified":"2026-01-01T00:00:00Z","size":1}"#,
        )
        .unwrap();
        // 0-10 in the catalogue, stars in the search
        assert!(Query::parse("rating:0").matches(&b), "unrated");
        assert!(!Query::parse("rating:>=1").matches(&b));
        b.meta.rating = 10;
        assert!(Query::parse("rating:5").matches(&b));
        assert!(Query::parse("rating:>=4").matches(&b));
        assert!(!Query::parse("rating:0").matches(&b));
        b.meta.rating = 7; // three and a half stars
        assert!(Query::parse("rating:>=3 rating:<4").matches(&b));
        assert!(!Query::parse("rating:5").matches(&b));
        // tags: case does not matter, a leading minus excludes
        assert!(Query::parse("tag:sci-fi").matches(&b));
        assert!(Query::parse("tag:CLASSIC -tag:horror").matches(&b));
        assert!(!Query::parse("-tag:classic").matches(&b));
        assert!(Query::parse("tag:").matches(&b), "any tag");
    }

    #[test]
    fn format_finds_further_formats_of_a_book() {
        let mut b: Book = serde_json::from_str(
            r#"{"id":1,"file":"a.epub","meta":{"title":"Dune","title_sort":"Dune","authors":["F"],"author_sort":"F","publisher":"","language":"en","description":"","tags":[],"series":"","series_index":0.0,"identifiers":[],"published":"","rights":"","rating":0},"added":"2026-01-01T00:00:00Z","modified":"2026-01-01T00:00:00Z","size":1}"#,
        )
        .unwrap();
        assert!(!Query::parse("format:azw3").matches(&b));
        b.formats.push(crate::formats::FormatFile {
            file: "a.azw3".into(),
            format: "azw3".into(),
            size: 1,
        });
        assert!(Query::parse("format:azw3").matches(&b));
        assert!(Query::parse("format:epub").matches(&b));
        assert!(!Query::parse("-format:azw3").matches(&b));
    }

    #[test]
    fn drm_and_audiobook_find_protected_books_and_linked_audio() {
        let mut b: Book = serde_json::from_str(
            r#"{"id":1,"file":"a.epub","meta":{"title":"Dune","title_sort":"Dune","authors":["F"],"author_sort":"F","publisher":"","language":"en","description":"","tags":[],"series":"","series_index":0.0,"identifiers":[],"published":"","rights":"","rating":0},"added":"2026-01-01T00:00:00Z","modified":"2026-01-01T00:00:00Z","size":1}"#,
        )
        .unwrap();
        assert!(!Query::parse("drm:").matches(&b));
        assert!(!Query::parse("audiobook:").matches(&b));
        b.drm = "Kindle".into();
        b.resources.push(crate::formats::Resource {
            kind: crate::formats::AUDIOBOOK.into(),
            path: "/a.m4b".into(),
            label: "a.m4b".into(),
        });
        assert!(Query::parse("drm:").matches(&b));
        assert!(Query::parse("drm:kindle").matches(&b));
        assert!(!Query::parse("drm:kobo").matches(&b));
        assert!(Query::parse("audiobook:").matches(&b));
    }
}
