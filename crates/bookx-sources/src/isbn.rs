//! ISBN-10 / ISBN-13 handling (L34): normalising, validating, converting, extracting from text.

use std::sync::OnceLock;

use regex::Regex;

/// Strips hyphens, spaces and an `ISBN` prefix; `x` becomes `X`. Does not validate.
pub fn normalize(s: &str) -> String {
    let t = s.trim();
    let t = match t.get(..4) {
        Some(h) if h.eq_ignore_ascii_case("isbn") => {
            let mut rest = &t[4..];
            for tag in ["-10", "-13"] {
                if let Some(r) = rest.strip_prefix(tag) {
                    if r.chars().next().is_some_and(|c| !c.is_ascii_digit()) {
                        rest = r;
                    }
                }
            }
            rest.trim_start_matches([':', ' ', '-'])
        }
        _ => t,
    };
    t.chars()
        .filter(|c| !matches!(c, '-' | ' ' | '\u{2010}'..='\u{2015}' | '\u{00A0}'))
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn digit_value(c: char) -> Option<u32> {
    c.to_digit(10)
}

/// Check digit for the first nine digits of an ISBN-10 (`'X'` for 10). `None` if the input is not nine digits.
pub fn check_digit10(first9: &str) -> Option<char> {
    let digits: Vec<u32> = first9.chars().map(digit_value).collect::<Option<_>>()?;
    if digits.len() != 9 {
        return None;
    }
    let sum: u32 = digits
        .iter()
        .enumerate()
        .map(|(i, d)| d * (10 - i as u32))
        .sum();
    let check = (11 - sum % 11) % 11;
    Some(if check == 10 {
        'X'
    } else {
        char::from_digit(check, 10)?
    })
}

/// Check digit for the first twelve digits of an ISBN-13. `None` if the input is not twelve digits.
pub fn check_digit13(first12: &str) -> Option<char> {
    let digits: Vec<u32> = first12.chars().map(digit_value).collect::<Option<_>>()?;
    if digits.len() != 12 {
        return None;
    }
    let sum: u32 = digits
        .iter()
        .enumerate()
        .map(|(i, d)| if i % 2 == 0 { *d } else { d * 3 })
        .sum();
    char::from_digit((10 - sum % 10) % 10, 10)
}

/// Whether the (possibly hyphenated) string is a valid ISBN-10.
pub fn is_valid_isbn10(s: &str) -> bool {
    let n = normalize(s);
    if n.chars().count() != 10 || !n.is_ascii() {
        return false;
    }
    let (head, last) = n.split_at(9);
    check_digit10(head).is_some_and(|c| last.starts_with(c))
}

/// Whether the (possibly hyphenated) string is a valid ISBN-13 (prefix 978 or 979).
pub fn is_valid_isbn13(s: &str) -> bool {
    let n = normalize(s);
    if n.chars().count() != 13 || !n.is_ascii() || !(n.starts_with("978") || n.starts_with("979")) {
        return false;
    }
    let (head, last) = n.split_at(12);
    check_digit13(head).is_some_and(|c| last.starts_with(c))
}

pub fn is_valid(s: &str) -> bool {
    is_valid_isbn10(s) || is_valid_isbn13(s)
}

/// ISBN-10 → ISBN-13 (978 prefix). `None` when the input is not a valid ISBN-10.
pub fn to_isbn13(isbn10: &str) -> Option<String> {
    if !is_valid_isbn10(isbn10) {
        return None;
    }
    let n = normalize(isbn10);
    let body = format!("978{}", &n[..9]);
    let c = check_digit13(&body)?;
    Some(format!("{body}{c}"))
}

/// ISBN-13 → ISBN-10; only 978-prefixed ISBNs have a 10-digit form.
pub fn to_isbn10(isbn13: &str) -> Option<String> {
    if !is_valid_isbn13(isbn13) {
        return None;
    }
    let n = normalize(isbn13);
    let body = n.strip_prefix("978")?;
    let body = &body[..9];
    let c = check_digit10(body)?;
    Some(format!("{body}{c}"))
}

/// Canonical form for comparisons: the valid ISBN-13 (converting ISBN-10), or `None` if invalid.
pub fn canonical(s: &str) -> Option<String> {
    if is_valid_isbn13(s) {
        Some(normalize(s))
    } else {
        to_isbn13(s)
    }
}

/// Same book edition identity: both valid and equal after conversion to ISBN-13.
pub fn same_isbn(a: &str, b: &str) -> bool {
    match (canonical(a), canonical(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

fn candidate_regexes() -> &'static (Regex, Regex) {
    static RE: OnceLock<(Regex, Regex)> = OnceLock::new();
    RE.get_or_init(|| {
        (
            // digits with hyphens only, e.g. 978-0-306-40615-7
            Regex::new(r"\d[\d\-\u{2010}]{8,20}[\dXx]").expect("static regex"),
            // after an ISBN label spaces are allowed too
            Regex::new(r"(?i)isbn(?:-?1[03])?\s*:?\s*(\d[\d\- \u{2010}]{8,24}[\dXx])")
                .expect("static regex"),
        )
    })
}

/// First valid ISBN-13 or ISBN-10 that is a prefix of `digits`.
fn valid_prefix(digits: &str) -> Option<String> {
    for len in [13, 10] {
        if digits.len() >= len && digits.is_char_boundary(len) {
            let cand = &digits[..len];
            if (len == 13 && is_valid_isbn13(cand)) || (len == 10 && is_valid_isbn10(cand)) {
                return Some(cand.to_string());
            }
        }
    }
    None
}

/// Finds valid ISBNs in free text, in order of appearance, normalised, without duplicates.
pub fn extract_isbns(text: &str) -> Vec<String> {
    let (plain, labelled) = candidate_regexes();
    let mut found: Vec<(usize, String)> = Vec::new();
    for m in plain.find_iter(text) {
        if let Some(v) = valid_prefix(&normalize(m.as_str())) {
            found.push((m.start(), v));
        }
    }
    for c in labelled.captures_iter(text) {
        if let Some(g) = c.get(1) {
            if let Some(v) = valid_prefix(&normalize(g.as_str())) {
                found.push((g.start(), v));
            }
        }
    }
    found.sort_by_key(|(pos, _)| *pos);
    let mut out: Vec<String> = Vec::new();
    for (_, v) in found {
        if !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_separators_and_prefix() {
        assert_eq!(normalize("978-0-306-40615-7"), "9780306406157");
        assert_eq!(normalize(" ISBN 0-306-40615-2 "), "0306406152");
        assert_eq!(normalize("ISBN-13: 978 0 306 40615 7"), "9780306406157");
        assert_eq!(normalize("ISBN:0-8044-2957-x"), "080442957X");
        assert_eq!(normalize("0‐306‐40615‐2"), "0306406152");
        assert_eq!(normalize(""), "");
    }

    #[test]
    fn check_digits() {
        assert_eq!(check_digit10("030640615"), Some('2'));
        assert_eq!(check_digit10("080442957"), Some('X'));
        assert_eq!(check_digit13("978030640615"), Some('7'));
        assert_eq!(check_digit13("978316148410"), Some('0'));
        assert_eq!(check_digit10("12345678"), None);
        assert_eq!(check_digit10("12345678a"), None);
        assert_eq!(check_digit13("12345"), None);
        assert_eq!(check_digit13("97803064061x"), None);
    }

    #[test]
    fn validates_isbn10() {
        assert!(is_valid_isbn10("0-306-40615-2"));
        assert!(is_valid_isbn10("080442957X"));
        assert!(is_valid_isbn10("0-8044-2957-x"));
        assert!(!is_valid_isbn10("0306406153"));
        assert!(!is_valid_isbn10("030640615"));
        assert!(!is_valid_isbn10("03064061522"));
        assert!(!is_valid_isbn10("abcdefghij"));
        assert!(!is_valid_isbn10("0306406é52"));
        assert!(!is_valid_isbn10(""));
    }

    #[test]
    fn validates_isbn13() {
        assert!(is_valid_isbn13("978-0-306-40615-7"));
        assert!(is_valid_isbn13("9783161484100"));
        assert!(is_valid_isbn13("979-10-90636-07-1"));
        assert!(!is_valid_isbn13("9780306406158"));
        assert!(!is_valid_isbn13("9770306406157"), "prefix must be 978/979");
        assert!(!is_valid_isbn13("978030640615"));
        assert!(!is_valid_isbn13(""));
        assert!(is_valid("0306406152") && is_valid("9780306406157"));
        assert!(!is_valid("12345"));
    }

    #[test]
    fn converts_10_to_13_and_back() {
        assert_eq!(to_isbn13("0-306-40615-2").as_deref(), Some("9780306406157"));
        assert_eq!(
            to_isbn10("978-0-306-40615-7").as_deref(),
            Some("0306406152")
        );
        assert_eq!(to_isbn13("080442957X").as_deref(), Some("9780804429573"));
        assert_eq!(to_isbn10("9780804429573").as_deref(), Some("080442957X"));
        assert_eq!(to_isbn13("0306406153"), None);
        assert_eq!(to_isbn10("9791090636071"), None, "979 has no ISBN-10");
        assert_eq!(to_isbn10("9780306406158"), None);
    }

    #[test]
    fn canonical_and_same() {
        assert_eq!(canonical("0-306-40615-2").as_deref(), Some("9780306406157"));
        assert_eq!(
            canonical("978-0-306-40615-7").as_deref(),
            Some("9780306406157")
        );
        assert_eq!(canonical("junk"), None);
        assert!(same_isbn("0306406152", "978-0-306-40615-7"));
        assert!(!same_isbn("0306406152", "9783161484100"));
        assert!(!same_isbn("bad", "bad"), "invalid ISBNs never match");
    }

    #[test]
    fn extracts_from_free_text() {
        let t = "Printed in 2008. ISBN 978-0-306-40615-7 (hardcover), also ISBN-10: 0-8044-2957-X. \
                 Order 1234567890123 is not one. Repeat 9780306406157.";
        assert_eq!(extract_isbns(t), vec!["9780306406157", "080442957X"]);
    }

    #[test]
    fn extracts_labelled_with_spaces_without_swallowing_following_numbers() {
        let t = "ISBN 0 306 40615 2 published 1999";
        assert_eq!(extract_isbns(t), vec!["0306406152"]);
        let t2 = "ISBN: 9780306406157 2008";
        assert_eq!(extract_isbns(t2), vec!["9780306406157"]);
    }

    #[test]
    fn extract_ignores_invalid_and_empty() {
        assert!(extract_isbns("").is_empty());
        assert!(extract_isbns("phone 555-123-4567 id 123456789").is_empty());
        assert!(extract_isbns("ISBN 0-306-40615-3").is_empty());
    }

    #[test]
    fn extract_handles_unicode_text() {
        let t = "Büchér — ISBN 978‐0‐306‐40615‐7 — fin";
        assert_eq!(extract_isbns(t), vec!["9780306406157"]);
    }
}
