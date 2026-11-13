//! Tag editing without GTK: how a typed text becomes tags, duplicates, and what a bulk edit of
//! several books shows. Tags keep the capitalisation the user typed first; two tags that differ only
//! in case, spaces around them or inner runs of spaces are the same tag.

/// One tag from a typed piece of text: trimmed, inner white space collapsed, no commas or
/// semicolons; `None` when nothing is left.
pub fn normalize(text: &str) -> Option<String> {
    let t = text
        .replace([',', ';'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!t.is_empty()).then_some(t)
}

/// The tags in a text typed with commas (or semicolons) between them, without duplicates.
pub fn parse_entry(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for piece in text.split([',', ';']) {
        if let Some(t) = normalize(piece) {
            add(&mut out, &t);
        }
    }
    out
}

fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// Whether the list has this tag (case does not matter).
pub fn has(tags: &[String], tag: &str) -> bool {
    tags.iter().any(|t| same(t, tag))
}

/// Adds a tag unless it is already there (whatever its case). True when it was added.
pub fn add(tags: &mut Vec<String>, tag: &str) -> bool {
    match normalize(tag) {
        Some(t) if !has(tags, &t) => {
            tags.push(t);
            true
        }
        _ => false,
    }
}

/// Removes a tag (whatever its case). True when it was there.
pub fn remove(tags: &mut Vec<String>, tag: &str) -> bool {
    let n = tags.len();
    tags.retain(|t| !same(t, tag));
    tags.len() != n
}

/// The tags of several books with how many of them have each, most common first, then by name.
pub fn union_counts(books: &[&[String]]) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = Vec::new();
    for tags in books {
        let mut seen: Vec<&String> = Vec::new();
        for t in *tags {
            if seen.iter().any(|s| same(s, t)) {
                continue;
            }
            seen.push(t);
            match out.iter_mut().find(|(n, _)| same(n, t)) {
                Some(e) => e.1 += 1,
                None => out.push((t.clone(), 1)),
            }
        }
    }
    out.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then(a.0.to_lowercase().cmp(&b.0.to_lowercase()))
    });
    out
}

/// The existing tags that start with (or else contain) what was typed, for the suggestions;
/// tags the book has already are left out.
pub fn suggestions(all: &[String], typed: &str, have: &[String], limit: usize) -> Vec<String> {
    let q = typed.trim().to_lowercase();
    let mut starts: Vec<String> = Vec::new();
    let mut contains: Vec<String> = Vec::new();
    for t in all {
        if has(have, t) || (starts.iter().chain(&contains)).any(|s| same(s, t)) {
            continue;
        }
        let l = t.to_lowercase();
        if q.is_empty() || l.starts_with(&q) {
            starts.push(t.clone());
        } else if l.contains(&q) {
            contains.push(t.clone());
        }
    }
    starts.sort_by_key(|t| t.to_lowercase());
    contains.sort_by_key(|t| t.to_lowercase());
    starts.extend(contains);
    starts.truncate(limit);
    starts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(t: &[&str]) -> Vec<String> {
        t.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_typed_tag_is_trimmed_and_cleaned() {
        assert_eq!(normalize("  sci   fi  "), Some("sci fi".into()));
        assert_eq!(normalize(" ,; "), None);
        assert_eq!(normalize("Fantasy,"), Some("Fantasy".into()));
    }

    #[test]
    fn a_comma_separated_entry_gives_unique_tags() {
        assert_eq!(parse_entry("a, b ,A,, c;b"), v(&["a", "b", "c"]));
        assert!(parse_entry("  ,").is_empty());
    }

    #[test]
    fn tags_are_added_and_removed_whatever_their_case() {
        let mut t = v(&["Fantasy"]);
        assert!(!add(&mut t, " fantasy "));
        assert!(add(&mut t, "Sci-Fi"));
        assert_eq!(t, v(&["Fantasy", "Sci-Fi"]));
        assert!(remove(&mut t, "FANTASY"));
        assert!(!remove(&mut t, "fantasy"));
        assert_eq!(t, v(&["Sci-Fi"]));
    }

    #[test]
    fn a_bulk_edit_shows_how_many_books_have_each_tag() {
        let a = v(&["x", "Y"]);
        let b = v(&["y", "z"]);
        let c = v(&["Y", "Y"]);
        let u = union_counts(&[&a, &b, &c]);
        let want: Vec<(String, usize)> = vec![("Y".into(), 3), ("x".into(), 1), ("z".into(), 1)];
        assert_eq!(u, want);
    }

    #[test]
    fn suggestions_start_with_the_typed_text_and_skip_what_the_book_has() {
        let all = v(&["Fantasy", "fan fiction", "Sci-Fi", "Urban fantasy"]);
        assert_eq!(
            suggestions(&all, "fan", &v(&["fan fiction"]), 5),
            v(&["Fantasy", "Urban fantasy"])
        );
        assert_eq!(suggestions(&all, "", &[], 2).len(), 2);
        assert!(suggestions(&all, "zzz", &[], 5).is_empty());
    }
}
