//! Author name tools (L35): splitting lists, `Last, First` conversion, sort keys, initials, matching.

use crate::text::fold;

const PARTICLES: &[&str] = &[
    "van", "von", "de", "del", "della", "di", "da", "du", "dos", "das", "der", "den", "la", "le",
    "ter", "ten", "bin", "ibn", "al", "el", "st", "st.", "y", "af", "av",
];
const SUFFIXES: &[&str] = &[
    "jr", "jr.", "sr", "sr.", "ii", "iii", "iv", "phd", "ph.d.", "md", "m.d.",
];
const ROLE_WORDS: &[&str] = &[
    "author",
    "editor",
    "translator",
    "illustrator",
    "narrator",
    "foreword",
    "introduction",
    "ed.",
    "eds.",
    "trans.",
    "contributor",
    "adaptation",
];

/// A parsed person name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Name {
    pub first: String,
    pub last: String,
    pub suffix: String,
}

fn is_suffix(w: &str) -> bool {
    SUFFIXES.contains(&w.trim_matches(',').to_lowercase().as_str())
}

/// Parses `First Middle Last`, `Last, First` and `Last, First, Jr.` (also `First Last, Jr.`).
pub fn parse_name(name: &str) -> Name {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Name::default();
    }
    let parts: Vec<&str> = name
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() >= 2 {
        let rest = &parts[1..];
        let is_suf_only = rest.iter().all(|p| is_suffix(p));
        if is_suf_only {
            // "John Smith, Jr."
            let mut n = parse_name(parts[0]);
            n.suffix = rest.join(" ");
            return n;
        }
        let mut suffix = String::new();
        let mut first_parts = Vec::new();
        for p in rest {
            if is_suffix(p) {
                suffix = p.to_string();
            } else {
                first_parts.push(*p);
            }
        }
        return Name {
            first: first_parts.join(" "),
            last: parts[0].to_string(),
            suffix,
        };
    }
    let mut words: Vec<&str> = name.split(' ').collect();
    let mut suffix = String::new();
    if words.len() > 1 && words.last().is_some_and(|w| is_suffix(w)) {
        suffix = words.pop().unwrap_or_default().to_string();
    }
    if words.len() == 1 {
        return Name {
            first: String::new(),
            last: words[0].to_string(),
            suffix,
        };
    }
    let mut split = words.len() - 1;
    while split > 1 && PARTICLES.contains(&words[split - 1].to_lowercase().as_str()) {
        split -= 1;
    }
    Name {
        first: words[..split].join(" "),
        last: words[split..].join(" "),
        suffix,
    }
}

impl Name {
    /// `First Last[ Suffix]`
    pub fn first_last(&self) -> String {
        let mut s = String::new();
        for p in [&self.first, &self.last, &self.suffix] {
            if !p.is_empty() {
                if !s.is_empty() {
                    s.push(' ');
                }
                s.push_str(p);
            }
        }
        s
    }

    /// `Last, First[ Suffix]`; single names stay as they are.
    pub fn last_first(&self) -> String {
        if self.first.is_empty() {
            return if self.suffix.is_empty() {
                self.last.clone()
            } else {
                format!("{} {}", self.last, self.suffix)
            };
        }
        let mut s = format!("{}, {}", self.last, self.first);
        if !self.suffix.is_empty() {
            s.push(' ');
            s.push_str(&self.suffix);
        }
        s
    }
}

/// `Ada Lovelace` → `Lovelace, Ada`; already sorted names are left alone.
pub fn to_last_first(name: &str) -> String {
    parse_name(name).last_first()
}

/// `Lovelace, Ada` → `Ada Lovelace`; other names are only whitespace-normalised.
pub fn to_first_last(name: &str) -> String {
    parse_name(name).first_last()
}

/// How author sort strings are made from names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NameOrder {
    /// `Last, First` for Western names; names in Chinese, Japanese or Korean script, and Latin
    /// names of books in those languages, are kept as written (the family name comes first there).
    #[default]
    Auto,
    /// Always `Last, First`.
    LastFirst,
    /// Never rearranged.
    AsWritten,
}

impl NameOrder {
    pub fn key(self) -> &'static str {
        match self {
            NameOrder::Auto => "auto",
            NameOrder::LastFirst => "last-first",
            NameOrder::AsWritten => "as-written",
        }
    }

    pub fn from_key(k: &str) -> NameOrder {
        match k {
            "last-first" => NameOrder::LastFirst,
            "as-written" => NameOrder::AsWritten,
            _ => NameOrder::Auto,
        }
    }
}

fn is_family_first_char(c: char) -> bool {
    matches!(c as u32,
        0x2E80..=0x2FDF | 0x3000..=0x30FF | 0x31F0..=0x31FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xAC00..=0xD7AF | 0x1100..=0x11FF | 0x3130..=0x318F | 0xF900..=0xFAFF | 0xFF66..=0xFF9F)
}

/// Must `name` (of a book in language `lang`) stay as written in an author sort string?
pub fn keep_as_written(name: &str, lang: &str, order: NameOrder) -> bool {
    match order {
        NameOrder::AsWritten => true,
        NameOrder::LastFirst => false,
        NameOrder::Auto => {
            let primary = lang
                .split(['-', '_'])
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            name.chars().any(is_family_first_char) || matches!(primary.as_str(), "ja" | "zh" | "ko")
        }
    }
}

/// Author sort string for a list: `Last, First & Last, First`.
pub fn author_sort(authors: &[String]) -> String {
    authors
        .iter()
        .map(|a| to_last_first(a))
        .filter(|a| !a.is_empty())
        .collect::<Vec<_>>()
        .join(" & ")
}

/// Splits a combined author field on `;`, `&`, `+` and ` and `/` und `/` et `.
/// Commas are not separators (they are ambiguous with `Last, First`), except in lists of three or
/// more comma parts where no part is a suffix and every part has at least two words.
pub fn split_authors(s: &str) -> Vec<String> {
    let mut pieces = vec![s.to_string()];
    for sep in [";", " & ", "&", " + ", " and ", " und ", " et ", " AND "] {
        pieces = pieces
            .into_iter()
            .flat_map(|p| p.split(sep).map(String::from).collect::<Vec<_>>())
            .collect();
    }
    let mut out = Vec::new();
    for p in pieces {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        let comma: Vec<&str> = p.split(',').map(str::trim).collect();
        if comma.len() >= 3
            && comma
                .iter()
                .all(|c| c.split_whitespace().count() >= 2 && !is_suffix(c))
        {
            out.extend(comma.iter().map(|c| c.to_string()));
        } else {
            out.push(p.to_string());
        }
    }
    out
}

/// Cleans one author string: whitespace, role suffixes like `(Editor)`, shouting case.
pub fn clean_author(name: &str) -> String {
    let mut s = name.split_whitespace().collect::<Vec<_>>().join(" ");
    loop {
        let before = s.clone();
        if let Some(i) = s.rfind('(') {
            if s.ends_with(')') {
                let inner = s[i + 1..s.len() - 1].trim().to_lowercase();
                if ROLE_WORDS.contains(&inner.as_str()) {
                    s = s[..i].trim().to_string();
                }
            }
        }
        if let Some(stripped) = s.strip_suffix(" - Goodreads Author") {
            s = stripped.trim().to_string();
        }
        if s == before {
            break;
        }
    }
    let letters: Vec<char> = s.chars().filter(|c| c.is_alphabetic()).collect();
    let all_upper = letters.len() > 3 && letters.iter().all(|c| c.is_uppercase());
    let all_lower = letters.len() > 3 && letters.iter().all(|c| c.is_lowercase());
    if all_upper || all_lower {
        s = title_case_name(&s);
    }
    s.trim_matches(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .to_string()
}

fn title_case_name(s: &str) -> String {
    s.split(' ')
        .enumerate()
        .map(|(i, w)| {
            if i > 0 && PARTICLES.contains(&w.to_lowercase().as_str()) && w.len() > 1 {
                return w.to_lowercase();
            }
            // capitalise after hyphens and apostrophes too: "o'brien" -> "O'Brien"
            let mut out = String::with_capacity(w.len());
            let mut up = true;
            for c in w.chars() {
                if up {
                    out.extend(c.to_uppercase());
                } else {
                    out.extend(c.to_lowercase());
                }
                up = matches!(c, '-' | '\'' | '\u{2019}' | '.');
            }
            out
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Initials of a first-name string: `J. R. R.`, `JRR`, `J.R.R.` and `John Ronald Reuel` → `jrr`.
pub fn initials(first: &str) -> Vec<char> {
    let mut out = Vec::new();
    for word in first.split(|c: char| c.is_whitespace() || c == '-') {
        let w = word.trim_matches('.');
        if w.is_empty() {
            continue;
        }
        let dots: Vec<&str> = w.split('.').filter(|x| !x.is_empty()).collect();
        if dots.len() > 1 {
            out.extend(dots.iter().filter_map(|d| fold(d).chars().next()));
        } else if (2..=3).contains(&w.chars().count()) && w.chars().all(|c| c.is_uppercase()) {
            // "JRR" written without dots
            out.extend(fold(w).chars());
        } else if let Some(c) = fold(w).chars().next() {
            out.push(c);
        }
    }
    out
}

fn last_key(n: &Name) -> String {
    crate::text::normalize(&n.last)
}

/// Whether two first-name strings can belong to the same person: equal, or one is the
/// initials/prefix form of the other, or one is empty.
fn first_compatible(a: &str, b: &str) -> bool {
    if a.trim().is_empty() || b.trim().is_empty() {
        return true;
    }
    let (ia, ib) = (initials(a), initials(b));
    let n = ia.len().min(ib.len());
    if n == 0 || ia[..n] != ib[..n] {
        return false;
    }
    // Where both sides spell a first name out, the spelled names must be equal.
    let wa: Vec<String> = crate::text::words(a);
    let wb: Vec<String> = crate::text::words(b);
    for (x, y) in wa.iter().zip(wb.iter()) {
        if x.chars().count() > 1
            && y.chars().count() > 1
            && x != y
            && !x.starts_with(y.as_str())
            && !y.starts_with(x.as_str())
        {
            return false;
        }
    }
    true
}

/// Whether two author strings name the same person (`Tolkien, J.R.R.` = `John Ronald Reuel Tolkien`).
pub fn same_author(a: &str, b: &str) -> bool {
    let (na, nb) = (parse_name(a), parse_name(b));
    if na.last.is_empty() || nb.last.is_empty() {
        return false;
    }
    last_key(&na) == last_key(&nb) && first_compatible(&na.first, &nb.first)
}

/// Whether two author lists describe the same authors: every author of the shorter list has a
/// match in the other. Empty lists never match anything.
pub fn same_authors(a: &[String], b: &[String]) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let forward = short.iter().all(|x| long.iter().any(|y| same_author(x, y)));
    // lists of equal length must agree both ways so the relation stays symmetric
    forward
        && (short.len() != long.len()
            || long.iter().all(|y| short.iter().any(|x| same_author(x, y))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_simple_names() {
        let n = parse_name("Ada Lovelace");
        assert_eq!((n.first.as_str(), n.last.as_str()), ("Ada", "Lovelace"));
        let n = parse_name("J. R. R. Tolkien");
        assert_eq!((n.first.as_str(), n.last.as_str()), ("J. R. R.", "Tolkien"));
        let n = parse_name("Plato");
        assert_eq!((n.first.as_str(), n.last.as_str()), ("", "Plato"));
        assert_eq!(parse_name("   "), Name::default());
    }

    #[test]
    fn parse_last_first_and_suffix() {
        let n = parse_name("Tolkien, John Ronald Reuel");
        assert_eq!(
            (n.first.as_str(), n.last.as_str()),
            ("John Ronald Reuel", "Tolkien")
        );
        let n = parse_name("King, Martin Luther, Jr.");
        assert_eq!(n.suffix, "Jr.");
        assert_eq!(n.first, "Martin Luther");
        let n = parse_name("Martin Luther King, Jr.");
        assert_eq!(
            (n.first.as_str(), n.last.as_str(), n.suffix.as_str()),
            ("Martin Luther", "King", "Jr.")
        );
        let n = parse_name("Sammy Davis Jr");
        assert_eq!(n.suffix, "Jr");
    }

    #[test]
    fn parse_particles() {
        let n = parse_name("Ludwig van Beethoven");
        assert_eq!(
            (n.first.as_str(), n.last.as_str()),
            ("Ludwig", "van Beethoven")
        );
        let n = parse_name("Charles de la Cruz");
        assert_eq!(n.last, "de la Cruz");
        let n = parse_name("Van Morrison");
        assert_eq!((n.first.as_str(), n.last.as_str()), ("Van", "Morrison"));
    }

    #[test]
    fn conversions() {
        assert_eq!(to_last_first("Ada Lovelace"), "Lovelace, Ada");
        assert_eq!(to_last_first("Lovelace, Ada"), "Lovelace, Ada");
        assert_eq!(to_first_last("Lovelace, Ada"), "Ada Lovelace");
        assert_eq!(to_first_last("  Ada   Lovelace "), "Ada Lovelace");
        assert_eq!(to_last_first("Plato"), "Plato");
        assert_eq!(
            to_last_first("Martin Luther King, Jr."),
            "King, Martin Luther Jr."
        );
        assert_eq!(
            to_first_last("King, Martin Luther, Jr."),
            "Martin Luther King Jr."
        );
        assert_eq!(to_last_first(""), "");
        assert_eq!(
            to_last_first("Ludwig van Beethoven"),
            "van Beethoven, Ludwig"
        );
    }

    #[test]
    fn author_sort_joins() {
        assert_eq!(
            author_sort(&v(&["Neil Gaiman", "Terry Pratchett", ""])),
            "Gaiman, Neil & Pratchett, Terry"
        );
        assert_eq!(author_sort(&[]), "");
    }

    #[test]
    fn splits_author_lists() {
        assert_eq!(
            split_authors("Neil Gaiman & Terry Pratchett"),
            v(&["Neil Gaiman", "Terry Pratchett"])
        );
        assert_eq!(split_authors("A B and C D"), v(&["A B", "C D"]));
        assert_eq!(
            split_authors("Gaiman, Neil; Pratchett, Terry"),
            v(&["Gaiman, Neil", "Pratchett, Terry"])
        );
        assert_eq!(split_authors("Gaiman, Neil"), v(&["Gaiman, Neil"]));
        assert_eq!(
            split_authors("Ann Lee, Bob Ray, Cy Dee"),
            v(&["Ann Lee", "Bob Ray", "Cy Dee"])
        );
        assert_eq!(
            split_authors("Martin Luther King, Jr."),
            v(&["Martin Luther King, Jr."])
        );
        assert_eq!(split_authors(" ; & "), Vec::<String>::new());
        assert_eq!(
            split_authors("Anderson"),
            v(&["Anderson"]),
            "'and' inside a word is not a separator"
        );
    }

    #[test]
    fn cleans_authors() {
        assert_eq!(clean_author("  Ada   Lovelace "), "Ada Lovelace");
        assert_eq!(clean_author("Ada Lovelace (Author)"), "Ada Lovelace");
        assert_eq!(clean_author("Jane Roe (Editor) (Translator)"), "Jane Roe");
        assert_eq!(clean_author("Jane Roe - Goodreads Author"), "Jane Roe");
        assert_eq!(clean_author("ADA LOVELACE"), "Ada Lovelace");
        assert_eq!(clean_author("ada lovelace"), "Ada Lovelace");
        assert_eq!(clean_author("PATRICK O'BRIEN"), "Patrick O'Brien");
        assert_eq!(clean_author("LUDWIG VAN BEETHOVEN"), "Ludwig van Beethoven");
        assert_eq!(
            clean_author("Jane Roe (Illustrator Extraordinaire)"),
            "Jane Roe (Illustrator Extraordinaire)"
        );
        assert_eq!(clean_author("Smith, John,"), "Smith, John");
        assert_eq!(
            clean_author("JK Rowling"),
            "JK Rowling",
            "short all-caps tokens in mixed names stay"
        );
        assert_eq!(clean_author(""), "");
    }

    #[test]
    fn initials_forms() {
        assert_eq!(initials("J. R. R."), vec!['j', 'r', 'r']);
        assert_eq!(initials("J.R.R."), vec!['j', 'r', 'r']);
        assert_eq!(initials("JRR"), vec!['j', 'r', 'r']);
        assert_eq!(initials("John Ronald Reuel"), vec!['j', 'r', 'r']);
        assert_eq!(initials("Jean-Paul"), vec!['j', 'p']);
        assert_eq!(initials("Émile"), vec!['e']);
        assert!(initials("").is_empty());
    }

    #[test]
    fn same_author_variants() {
        assert!(same_author("J.R.R. Tolkien", "Tolkien, John Ronald Reuel"));
        assert!(same_author("Tolkien, J. R. R.", "John R. R. Tolkien"));
        assert!(same_author("Ursula K. Le Guin", "Ursula K. Le Guin"));
        assert!(same_author(
            "Gabriel García Márquez",
            "Gabriel Garcia Marquez"
        ));
        assert!(
            same_author("Tolkien", "J. R. R. Tolkien"),
            "missing first name is compatible"
        );
        assert!(!same_author("John Smith", "Jane Smith"));
        assert!(!same_author("John Smith", "John Smyth"));
        assert!(!same_author("J. Smith", "K. Smith"));
        assert!(!same_author("", "Smith"));
    }

    #[test]
    fn same_authors_lists() {
        assert!(same_authors(
            &v(&["Neil Gaiman"]),
            &v(&["Gaiman, Neil", "Terry Pratchett"])
        ));
        assert!(same_authors(&v(&["A B", "C D"]), &v(&["C D", "A B"])));
        assert!(!same_authors(&v(&["A B", "E F"]), &v(&["A B", "C D"])));
        assert!(!same_authors(&[], &[]));
        assert!(!same_authors(&v(&["A B"]), &[]));
    }

    #[test]
    fn name_order_keeps_family_first_names_as_written() {
        use NameOrder::*;
        assert!(keep_as_written("村上 春樹", "ja", Auto));
        assert!(keep_as_written("村上春樹", "en", Auto), "script decides");
        assert!(keep_as_written("김영하", "", Auto));
        assert!(
            keep_as_written("Murakami Haruki", "ja-JP", Auto),
            "language decides for Latin names"
        );
        assert!(!keep_as_written("Ada Lovelace", "en", Auto));
        assert!(!keep_as_written("Ada Lovelace", "", Auto));
        assert!(!keep_as_written("村上 春樹", "ja", LastFirst));
        assert!(keep_as_written("Ada Lovelace", "en", AsWritten));
        for o in [Auto, LastFirst, AsWritten] {
            assert_eq!(NameOrder::from_key(o.key()), o);
        }
        assert_eq!(NameOrder::from_key("junk"), Auto);
    }
}
