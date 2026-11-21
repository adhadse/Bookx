//! Calibre-style search expressions.
//!
//! Examples: `tolkien`, `author:"le guin" tag:fantasy`, `rating:>=4 -format:pdf`,
//! `series:=Discworld OR status:reading`, `(tag:a OR tag:b) -tag:c`, `title:~^the\s`,
//! `added:>=2024-01-01`, `pubdate:<5daysago`, `#mood:happy`, `#pages:>300`, `marked:true`.
//! `weak:` finds books with poor metadata (no author, a title like a file name, no cover, no
//! language), see [`super::quality`].
//!
//! * `field:value` contains, `field:=value` is exact, `field:>=value` compares (numbers, dates).
//! * `field:~pattern` is a case-insensitive regular expression (linear time, size limited).
//! * `-term`, `NOT term` and `-( ... )` exclude; `OR` joins alternatives; `AND` is the default.
//! * Date fields (`date`/`added`, `modified`, `pubdate`/`published`, `lastread`) take `2024`,
//!   `2024-05`, `2024-05-17`, `today`, `yesterday`, `thismonth`, `thisyear` and `Ndaysago`.
//! * `#label:` searches a custom column (see [`super::custom`]); `empty` finds books without a value.

use chrono::NaiveDate;
use regex::{Regex, RegexBuilder};

use super::library::Book;

#[derive(Debug, Clone, PartialEq)]
enum Op {
    Contains,
    Exact,
    Gt,
    Ge,
    Lt,
    Le,
    Regex,
}

/// A compiled regular expression; two are equal when their patterns are.
#[derive(Debug, Clone)]
struct Rx {
    src: String,
    re: Option<Regex>,
}

impl PartialEq for Rx {
    fn eq(&self, o: &Rx) -> bool {
        self.src == o.src
    }
}

/// Longest pattern accepted; longer ones match nothing.
const MAX_PATTERN: usize = crate::limits::QUERY_PATTERN_CHARS;

impl Rx {
    fn new(src: &str) -> Rx {
        let re = if src.chars().count() > MAX_PATTERN {
            None
        } else {
            RegexBuilder::new(src)
                .case_insensitive(true)
                .size_limit(1 << 20)
                .build()
                .ok()
        };
        Rx {
            src: src.to_string(),
            re,
        }
    }

    fn is_match(&self, hay: &str) -> bool {
        self.re.as_ref().is_some_and(|r| r.is_match(hay))
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Term {
    field: Option<String>,
    op: Op,
    value: String,
    rx: Option<Rx>,
}

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Term(Term),
    Not(Box<Node>),
    And(Vec<Node>),
    Or(Vec<Node>),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Query {
    root: Option<Node>,
}

#[derive(Debug, PartialEq)]
enum Tok {
    Open,
    Close,
    Or,
    And,
    Not,
    Word(String),
}

fn flush(cur: &mut String, out: &mut Vec<Tok>) {
    if cur.is_empty() {
        return;
    }
    let w = std::mem::take(cur);
    out.push(match w.as_str() {
        "OR" => Tok::Or,
        "AND" => Tok::And,
        "NOT" => Tok::Not,
        _ => Tok::Word(w),
    });
}

fn tokenize(input: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    let mut depth = 0usize;
    for c in input.chars() {
        match c {
            '"' => in_q = !in_q,
            c if c.is_whitespace() && !in_q => flush(&mut cur, &mut out),
            // a parenthesis opens a group at the start of a word (or after a minus), and closes
            // one at the end of a word; anywhere else it is an ordinary character
            '(' if !in_q && (cur.is_empty() || cur == "-") => {
                if cur == "-" {
                    out.push(Tok::Not);
                    cur.clear();
                }
                out.push(Tok::Open);
                depth += 1;
            }
            ')' if !in_q && depth > 0 => {
                flush(&mut cur, &mut out);
                out.push(Tok::Close);
                depth -= 1;
            }
            c => cur.push(c),
        }
    }
    flush(&mut cur, &mut out);
    out
}

struct Parser {
    toks: std::vec::IntoIter<Tok>,
    peeked: Option<Tok>,
}

impl Parser {
    fn peek(&mut self) -> Option<&Tok> {
        if self.peeked.is_none() {
            self.peeked = self.toks.next();
        }
        self.peeked.as_ref()
    }

    fn next(&mut self) -> Option<Tok> {
        self.peek();
        self.peeked.take()
    }

    fn or(&mut self, depth: usize) -> Option<Node> {
        let mut alts: Vec<Node> = Vec::new();
        loop {
            if let Some(n) = self.and(depth) {
                alts.push(n);
            }
            if matches!(self.peek(), Some(Tok::Or)) {
                self.next();
                continue;
            }
            break;
        }
        match alts.len() {
            0 => None,
            1 => alts.pop(),
            _ => Some(Node::Or(alts)),
        }
    }

    fn and(&mut self, depth: usize) -> Option<Node> {
        let mut parts: Vec<Node> = Vec::new();
        loop {
            match self.peek() {
                None | Some(Tok::Or) | Some(Tok::Close) => break,
                Some(Tok::And) => {
                    self.next();
                }
                _ => {
                    if let Some(n) = self.unary(depth) {
                        parts.push(n);
                    }
                }
            }
        }
        match parts.len() {
            0 => None,
            1 => parts.pop(),
            _ => Some(Node::And(parts)),
        }
    }

    fn unary(&mut self, depth: usize) -> Option<Node> {
        match self.next()? {
            Tok::Not => {
                if depth >= 64 {
                    return None;
                }
                self.unary(depth + 1).map(|n| Node::Not(Box::new(n)))
            }
            Tok::Open => {
                // nesting is limited so a hostile query cannot exhaust the stack
                let inner = if depth < 32 { self.or(depth + 1) } else { None };
                if matches!(self.peek(), Some(Tok::Close)) {
                    self.next();
                }
                inner
            }
            Tok::Word(w) => Some(parse_word(&w)),
            Tok::Close | Tok::Or | Tok::And => None,
        }
    }
}

fn parse_word(tok: &str) -> Node {
    let (negate, tok) = match tok.strip_prefix('-') {
        Some(rest) if !rest.is_empty() => (true, rest),
        _ => (false, tok),
    };
    let (field, rest) = match tok.split_once(':') {
        Some((f, r)) if is_field(f) => (Some(canonical_field(f)), r),
        _ => (None, tok),
    };
    let mut rx = None;
    let (op, value) = if let Some(v) = rest.strip_prefix('~') {
        rx = Some(Rx::new(v));
        (Op::Regex, v)
    } else if let Some(v) = rest.strip_prefix(">=") {
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
        (Op::Contains, rest)
    };
    let t = Node::Term(Term {
        field,
        op,
        value: value.to_lowercase(),
        rx,
    });
    if negate {
        Node::Not(Box::new(t))
    } else {
        t
    }
}

impl Query {
    pub fn parse(input: &str) -> Query {
        let mut p = Parser {
            toks: tokenize(input).into_iter(),
            peeked: None,
        };
        let mut parts = Vec::new();
        // a stray `)` cannot occur (it is only a token inside a group), but be safe
        while let Some(n) = p.or(0) {
            parts.push(n);
            if p.next().is_none() {
                break;
            }
        }
        let root = match parts.len() {
            0 => None,
            1 => parts.pop(),
            _ => Some(Node::And(parts)),
        };
        Query { root }
    }

    pub fn matches(&self, b: &Book) -> bool {
        match &self.root {
            None => true,
            Some(n) => n.matches(b, &Today::new()),
        }
    }
}

/// "Today" for relative dates, read once per search call.
struct Today(NaiveDate);

impl Today {
    fn new() -> Today {
        Today(chrono::Local::now().date_naive())
    }
}

impl Node {
    fn matches(&self, b: &Book, today: &Today) -> bool {
        match self {
            Node::Term(t) => t.matches(b, today),
            Node::Not(n) => !n.matches(b, today),
            Node::And(v) => v.iter().all(|n| n.matches(b, today)),
            Node::Or(v) => v.iter().any(|n| n.matches(b, today)),
        }
    }
}

fn is_field(f: &str) -> bool {
    if let Some(label) = f.strip_prefix('#') {
        return super::custom::is_label_shape(label);
    }
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
            | "marked"
            | "id"
            | "date"
            | "added"
            | "modified"
            | "pubdate"
            | "published"
            | "lastread"
    )
}

fn canonical_field(f: &str) -> String {
    match f.to_ascii_lowercase().as_str() {
        "authors" => "author",
        "tags" => "tag",
        "lang" => "language",
        "identifiers" => "identifier",
        "date" => "added",
        "published" => "pubdate",
        other => return other.to_string(),
    }
    .to_string()
}

fn text_match(hay: &str, term: &Term) -> bool {
    if term.op == Op::Regex {
        return term.rx.as_ref().is_some_and(|r| r.is_match(hay));
    }
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

/// The ISO date a date term stands for: `2024`, `2024-05`, `2024-05-17` as written, or a relative
/// word (`today`, `yesterday`, `thismonth`, `thisyear`, `5daysago`, `2weeksago`).
pub fn resolve_date(value: &str, today: NaiveDate) -> Option<String> {
    let v = value.trim().to_lowercase();
    let day = |d: NaiveDate| Some(d.format("%Y-%m-%d").to_string());
    match v.as_str() {
        "today" => return day(today),
        "yesterday" => return day(today.pred_opt()?),
        "thismonth" => return Some(today.format("%Y-%m").to_string()),
        "thisyear" => return Some(today.format("%Y").to_string()),
        _ => {}
    }
    for (suffix, per) in [("daysago", 1i64), ("weeksago", 7), ("monthsago", 30)] {
        if let Some(n) = v.strip_suffix(suffix) {
            let n: i64 = n.parse().ok()?;
            if !(0..=36_500).contains(&n) {
                return None;
            }
            return day(today.checked_sub_signed(chrono::Duration::days(n * per))?);
        }
    }
    let ok_digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    let mut parts = v.split('-');
    let year = parts.next()?;
    if !ok_digits(year, 4) {
        return None;
    }
    let month = parts.next();
    let dom = parts.next();
    if parts.next().is_some() {
        return None;
    }
    if let Some(m) = month {
        if !ok_digits(m, 2) || !(1..=12).contains(&m.parse::<u32>().ok()?) {
            return None;
        }
    }
    if dom.is_some() {
        return day(NaiveDate::parse_from_str(&v, "%Y-%m-%d").ok()?);
    }
    Some(v)
}

/// A date value (partial dates and timestamps allowed) against a date term.
fn date_match(hay: &str, term: &Term, today: &Today) -> bool {
    let hay: String = hay.chars().take(10).collect();
    if hay.is_empty() {
        return term.value == "empty";
    }
    if term.value.is_empty() {
        return true;
    }
    if term.value == "empty" {
        return false;
    }
    let Some(want) = resolve_date(&term.value, today.0) else {
        return false;
    };
    // compare at the precision of the term: `2024-05` covers the month
    let h: String = hay.chars().take(want.len()).collect();
    match term.op {
        Op::Gt => h > want,
        Op::Ge => h >= want,
        Op::Lt => h < want,
        Op::Le => h <= want,
        _ => h == want,
    }
}

/// A custom column value against a term. The value is compared by what it looks like: numbers
/// numerically, `true`/`false` as yes/no, dates as ISO dates, a list (`a, b`) item by item.
fn custom_match(stored: Option<&str>, term: &Term, today: &Today) -> bool {
    let v = term.value.as_str();
    let stored = stored.map(str::trim).filter(|s| !s.is_empty());
    if term.op == Op::Regex {
        return stored.is_some_and(|s| term.rx.as_ref().is_some_and(|r| r.is_match(s)));
    }
    if matches!(term.op, Op::Contains | Op::Exact) {
        match v {
            "" => return stored.is_some(),
            "empty" | "blank" | "unset" => return stored.is_none(),
            "true" | "yes" => {
                return stored.is_some_and(|s| matches!(s.to_lowercase().as_str(), "true" | "yes"))
            }
            "false" | "no" => {
                return stored.is_some_and(|s| matches!(s.to_lowercase().as_str(), "false" | "no"))
            }
            _ => {}
        }
    }
    let Some(s) = stored else {
        return false;
    };
    if let (Ok(a), Ok(_)) = (s.parse::<f64>(), v.parse::<f64>()) {
        return num_match(a, term);
    }
    let looks_date = |x: &str| x.len() >= 4 && x.as_bytes()[..4].iter().all(|b| b.is_ascii_digit());
    if matches!(term.op, Op::Gt | Op::Ge | Op::Lt | Op::Le)
        && looks_date(s)
        && resolve_date(v, today.0).is_some()
    {
        return date_match(s, term, today);
    }
    if matches!(term.op, Op::Contains | Op::Exact)
        && looks_date(s)
        && s.contains('-')
        && resolve_date(v, today.0).is_some()
    {
        return date_match(s, term, today);
    }
    match term.op {
        Op::Gt => s.to_lowercase().as_str() > v,
        Op::Ge => s.to_lowercase().as_str() >= v,
        Op::Lt => s.to_lowercase().as_str() < v,
        Op::Le => s.to_lowercase().as_str() <= v,
        Op::Exact => s.split(',').any(|i| i.trim().to_lowercase() == v) || s.to_lowercase() == v,
        _ => s.to_lowercase().contains(v),
    }
}

impl Term {
    fn matches(&self, b: &Book, today: &Today) -> bool {
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
            Some("id") => num_match(b.id as f64, self),
            Some("added") => date_match(&b.added.format("%Y-%m-%d").to_string(), self, today),
            Some("modified") => date_match(&b.modified.format("%Y-%m-%d").to_string(), self, today),
            Some("lastread") => date_match(
                &b.last_read
                    .map(|d| d.format("%Y-%m-%d").to_string())
                    .unwrap_or_default(),
                self,
                today,
            ),
            Some("pubdate") => date_match(&m.published, self, today),
            Some("marked") => b.marked != matches!(self.value.as_str(), "false" | "no"),
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
            Some(f) if f.starts_with('#') => {
                custom_match(b.custom.get(&f[1..]).map(String::as_str), self, today)
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(tags: &[&str]) -> Book {
        let tags = serde_json::to_string(tags).unwrap();
        serde_json::from_str(&format!(
            r#"{{"id":7,"file":"a.epub","meta":{{"title":"Dune","title_sort":"Dune","authors":["Frank Herbert"],"author_sort":"Herbert, Frank","publisher":"Ace","language":"en","description":"","tags":{tags},"series":"","series_index":0.0,"identifiers":[],"published":"1965-08-01","rights":"","rating":0}},"added":"2026-01-15T00:00:00Z","modified":"2026-01-01T00:00:00Z","size":1}}"#
        ))
        .unwrap()
    }

    #[test]
    fn parses_terms() {
        let q = Query::parse(r#"author:"le guin" -tag:horror OR rating:>=4"#);
        let Some(Node::Or(alts)) = &q.root else {
            panic!("an OR of two alternatives: {q:?}");
        };
        assert_eq!(alts.len(), 2);
        let Node::And(first) = &alts[0] else {
            panic!("two terms")
        };
        assert!(matches!(first[1], Node::Not(_)));
        let Node::Term(t) = &alts[1] else { panic!() };
        assert_eq!(t.op, Op::Ge);
        assert!(Query::parse("").matches(&book(&[])));
        assert!(Query::parse("   ").matches(&book(&[])));
    }

    #[test]
    fn parentheses_group_and_negate() {
        let b = book(&["fantasy", "classic"]);
        assert!(Query::parse("(tag:horror OR tag:fantasy) tag:classic").matches(&b));
        assert!(!Query::parse("(tag:horror OR tag:scifi) tag:classic").matches(&b));
        assert!(Query::parse("tag:horror OR (tag:fantasy tag:classic)").matches(&b));
        assert!(!Query::parse("-(tag:fantasy OR tag:horror)").matches(&b));
        assert!(Query::parse("-(tag:horror OR tag:scifi)").matches(&b));
        assert!(Query::parse("NOT tag:horror AND tag:fantasy").matches(&b));
        assert!(Query::parse("((tag:fantasy))").matches(&b));
        // an unclosed group still works, a parenthesis inside a word is text
        assert!(Query::parse("(tag:fantasy").matches(&b));
        assert!(!Query::parse("title:dune(x)").matches(&b));
        assert!(
            !Query::parse("tag:fantasy)").matches(&b),
            "a stray ) is text"
        );
        // hostile nesting does not overflow the stack
        let deep = format!("{}tag:x{}", "(".repeat(5000), ")".repeat(5000));
        let _ = Query::parse(&deep).matches(&b);
        let nots = "NOT ".repeat(20000) + "tag:x";
        let _ = Query::parse(&nots).matches(&b);
    }

    #[test]
    fn regular_expressions_use_a_tilde() {
        let b = book(&["sci-fi"]);
        assert!(Query::parse("title:~^du").matches(&b));
        assert!(Query::parse(r#"author:"~h.rb""#).matches(&b));
        assert!(Query::parse("tag:~^sci-(fi|fy)$").matches(&b));
        assert!(!Query::parse("title:~^x").matches(&b));
        assert!(Query::parse("-title:~^x").matches(&b));
        assert!(Query::parse("~^dune$").matches(&b), "any field");
        // an invalid or oversized pattern matches nothing instead of failing
        assert!(!Query::parse("title:~(").matches(&b));
        assert!(!Query::parse(&format!("title:~{}", "a".repeat(400))).matches(&b));
        assert!(!Query::parse("title:~(a*)*b(((((a{1000}){1000}){1000}){1000})").matches(&b));
    }

    #[test]
    fn dates_compare_at_the_precision_written() {
        let b = book(&[]);
        assert!(Query::parse("added:2026").matches(&b));
        assert!(Query::parse("added:2026-01").matches(&b));
        assert!(Query::parse("added:=2026-01-15").matches(&b));
        assert!(!Query::parse("added:2026-02").matches(&b));
        assert!(Query::parse("added:>=2026-01-01 added:<=2026-01-31").matches(&b));
        assert!(Query::parse("date:>2025-12").matches(&b));
        assert!(!Query::parse("added:>2026-01").matches(&b));
        assert!(Query::parse("pubdate:<1970").matches(&b));
        assert!(Query::parse("published:1965-08").matches(&b));
        assert!(Query::parse("modified:<2026-02-01").matches(&b));
        assert!(!Query::parse("lastread:2026").matches(&b), "never read");
        assert!(Query::parse("lastread:empty").matches(&b));
        assert!(!Query::parse("added:2026-02-30").matches(&b), "not a date");
        assert!(Query::parse("added:<today").matches(&b));
        assert!(Query::parse("added:>=1000daysago").matches(&b));
    }

    #[test]
    fn relative_dates_resolve_against_today() {
        let today = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        assert_eq!(resolve_date("today", today).as_deref(), Some("2026-03-01"));
        assert_eq!(
            resolve_date("yesterday", today).as_deref(),
            Some("2026-02-28")
        );
        assert_eq!(resolve_date("thismonth", today).as_deref(), Some("2026-03"));
        assert_eq!(resolve_date("thisyear", today).as_deref(), Some("2026"));
        assert_eq!(
            resolve_date("10daysago", today).as_deref(),
            Some("2026-02-19")
        );
        assert_eq!(
            resolve_date("2weeksago", today).as_deref(),
            Some("2026-02-15")
        );
        assert_eq!(resolve_date("2024-05", today).as_deref(), Some("2024-05"));
        assert_eq!(resolve_date("2024-5", today), None);
        assert_eq!(resolve_date("xdaysago", today), None);
        assert_eq!(resolve_date("99999999daysago", today), None);
    }

    #[test]
    fn marked_and_id_terms() {
        let mut b = book(&[]);
        assert!(!Query::parse("marked:true").matches(&b));
        assert!(Query::parse("marked:false").matches(&b));
        assert!(Query::parse("-marked:true").matches(&b));
        b.marked = true;
        assert!(Query::parse("marked:true").matches(&b));
        assert!(Query::parse("marked:").matches(&b));
        assert!(Query::parse("id:7").matches(&b));
        assert!(Query::parse("id:>=7 id:<8").matches(&b));
        assert!(!Query::parse("id:8").matches(&b));
    }

    #[test]
    fn custom_columns_are_searched_by_label() {
        let mut b = book(&[]);
        for (k, v) in [
            ("mood", "Happy"),
            ("pages", "320"),
            ("price", "9.5"),
            ("read", "true"),
            ("finished", "2025-06-30"),
            ("labels", "alpha, beta"),
            ("score", "4.5"),
        ] {
            b.custom.insert(k.into(), v.into());
        }
        let m = |q: &str| Query::parse(q).matches(&b);
        assert!(m("#mood:happy") && m("#MOOD:=HAPPY") && m("#mood:app"));
        assert!(!m("#mood:=app") && !m("#mood:sad"));
        assert!(m("#pages:>300") && m("#pages:<=320") && m("#pages:320") && !m("#pages:>320"));
        assert!(m("#pages:>=9") && !m("#pages:<10"), "numbers, not text");
        assert!(m("#price:9.5") && m("#price:>9"));
        assert!(m("#read:true") && m("#read:yes") && !m("#read:false") && !m("#read:no"));
        assert!(m("#finished:2025") && m("#finished:>=2025-06") && m("#finished:<2026"));
        assert!(!m("#finished:>=1daysago"));
        assert!(m("#labels:=beta") && m("#labels:alp") && !m("#labels:=bet"));
        assert!(m("#score:>=4"));
        assert!(m("#mood:") && !m("#unset:") && m("#unset:empty") && m("-#unset:"));
        assert!(!m("#mood:empty"));
        assert!(m("#mood:~^h.p+y$") && !m("#mood:~^x"));
        assert!(m("#mood:happy #pages:>100") && !m("#mood:happy #pages:>400"));
        // a missing value never matches a comparison
        assert!(!m("#unset:>1") && !m("#unset:<1"));
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
