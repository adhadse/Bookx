//! Fuzzy search of the editor (CE-424): finds text that is *almost* the query, tolerating a few
//! typing mistakes (a missing, extra or different letter). It is the "approximate substring" kind of
//! match (Sellers' edit distance): every stretch of the text whose edit distance to the query is
//! small enough is a match, the best one of each cluster. No GTK in here.

/// How many edits a query of `len` characters may be off by: none for very short queries, one for up
/// to six characters, then one more for every four.
pub fn allowed_edits(len: usize) -> usize {
    match len {
        0..=3 => 0,
        4..=6 => 1,
        n => 1 + (n - 3) / 4,
    }
}

fn same(a: char, b: char, case_sensitive: bool) -> bool {
    if case_sensitive {
        a == b
    } else {
        a == b || a.to_lowercase().eq(b.to_lowercase())
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Byte ranges of the approximate matches of `query` in `text`, in order and not overlapping.
/// `max_edits` of `None` uses [`allowed_edits`]. With `whole_word` a match must start and end at
/// word boundaries.
pub fn find_all(
    text: &str,
    query: &str,
    max_edits: Option<usize>,
    case_sensitive: bool,
    whole_word: bool,
) -> Vec<(usize, usize)> {
    let q: Vec<char> = query.chars().collect();
    if q.is_empty() {
        return Vec::new();
    }
    let k = max_edits.unwrap_or_else(|| allowed_edits(q.len()));
    let t: Vec<(usize, char)> = text.char_indices().collect();
    let m = q.len();
    // column of the DP: (cost, start index in `t`) for every prefix of the query
    let mut old: Vec<(usize, usize)> = (0..=m).map(|i| (i, 0)).collect();
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut best: Option<(usize, usize, usize)> = None; // (cost, start, end) in char indices
    let mut floor = 0usize; // matches may not start before the end of the previous one
    let flush = |best: &mut Option<(usize, usize, usize)>,
                 out: &mut Vec<(usize, usize)>,
                 floor: &mut usize| {
        if let Some((_, s, e)) = best.take() {
            let b = |i: usize| t.get(i).map(|x| x.0).unwrap_or(text.len());
            let (bs, be) = (b(s), b(e));
            let ok = !whole_word
                || (text[..bs].chars().next_back().is_none_or(|c| !is_word(c))
                    && text[be..].chars().next().is_none_or(|c| !is_word(c)));
            if bs < be && ok {
                out.push((bs, be));
                *floor = e;
            }
        }
    };
    for (j, &(_, ch)) in t.iter().enumerate() {
        let mut new: Vec<(usize, usize)> = Vec::with_capacity(m + 1);
        new.push((0, j + 1));
        for i in 1..=m {
            let sub = old[i - 1].0 + usize::from(!same(q[i - 1], ch, case_sensitive));
            let mut cell = (sub, old[i - 1].1);
            let del_text = old[i].0 + 1;
            if del_text < cell.0 {
                cell = (del_text, old[i].1);
            }
            let del_query = new[i - 1].0 + 1;
            if del_query < cell.0 {
                cell = (del_query, new[i - 1].1);
            }
            new.push(cell);
        }
        let (cost, start) = new[m];
        if cost <= k && start >= floor {
            // a candidate that starts where the pending one ended is another match
            if best.is_some_and(|b| start >= b.2) {
                flush(&mut best, &mut out, &mut floor);
            }
            if best.is_none_or(|b| cost <= b.0) {
                best = Some((cost, start, j + 1));
            }
        }
        old = new;
    }
    flush(&mut best, &mut out, &mut floor);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str, q: &str) -> Vec<String> {
        find_all(text, q, None, false, false)
            .into_iter()
            .map(|(a, b)| text[a..b].to_string())
            .collect()
    }

    #[test]
    fn exact_matches_are_found() {
        assert_eq!(found("the cat sat", "cat"), vec!["cat"]);
        assert_eq!(found("Hello hello", "hello"), vec!["Hello", "hello"]);
    }

    #[test]
    fn typing_mistakes_are_tolerated() {
        assert_eq!(found("He said recieve it", "receive"), vec!["recieve"]);
        assert_eq!(
            found("a colour, a color", "colour"),
            vec!["colour", "color"]
        );
        assert_eq!(found("Mississipi", "mississippi"), vec!["Mississipi"]);
        assert_eq!(found("hallo world", "hello"), vec!["hallo"]);
    }

    #[test]
    fn short_queries_stay_exact_and_unrelated_text_is_ignored() {
        assert!(
            found("cut cot cat", "cat").len() == 1,
            "no edits for 3 letters"
        );
        assert!(found("completely different", "banana").is_empty());
        assert_eq!(allowed_edits(3), 0);
        assert_eq!(allowed_edits(5), 1);
        assert_eq!(allowed_edits(7), 2);
        assert_eq!(allowed_edits(11), 3);
        assert!(find_all("x", "", None, false, false).is_empty());
    }

    #[test]
    fn case_whole_word_and_explicit_edits() {
        assert!(find_all("Hello", "hello", None, true, false).len() == 1);
        assert!(find_all("Hallo", "hello", Some(0), true, false).is_empty());
        assert_eq!(
            find_all("Hallo", "hello", Some(1), false, false),
            vec![(0, 5)]
        );
        let t = "bookshelf shelf";
        let w = find_all(t, "shelf", Some(0), false, true);
        assert_eq!(
            w,
            vec![(10, 15)],
            "whole word skips the one inside bookshelf"
        );
    }

    #[test]
    fn offsets_are_bytes_and_matches_do_not_overlap() {
        let t = "é café cafe";
        let m = find_all(t, "cafe", Some(1), false, false);
        assert!(!m.is_empty());
        for w in m.windows(2) {
            assert!(w[0].1 <= w[1].0, "{m:?}");
        }
        for (a, b) in &m {
            assert!(t.is_char_boundary(*a) && t.is_char_boundary(*b));
        }
        assert_eq!(&t[m[0].0..m[0].1], "café");
    }
}
