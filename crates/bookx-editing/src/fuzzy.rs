//! Subsequence matching for file-name completion (CE-345): the typed letters must appear in order in the
//! candidate; word starts (after `/`, `-`, `_`, `.`, a digit/letter change or a lower to upper change) and the
//! start of the base name score highest, so `c2` finds `chapter-2.xhtml` and `ss` finds `text/style/s.css`.

/// Score of `query` in `candidate` (higher is better), or `None` when it is not a subsequence. Case-insensitive.
/// An empty query matches everything with score 0.
pub fn score(query: &str, candidate: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let cand: Vec<char> = candidate.chars().collect();
    let q: Vec<char> = query.chars().collect();
    if q.len() > cand.len() {
        return None;
    }
    let base_start = candidate
        .rfind('/')
        .map(|b| candidate[..b + 1].chars().count())
        .unwrap_or(0);
    let lower = |c: char| c.to_lowercase().next().unwrap_or(c);
    // best[j] = best score matching q[..=i] with q[i] at cand[j] (dynamic programming, small inputs)
    let n = cand.len();
    let mut prev: Vec<Option<i32>> = vec![None; n];
    for (i, qc) in q.iter().enumerate() {
        let mut cur: Vec<Option<i32>> = vec![None; n];
        for j in 0..n {
            if lower(cand[j]) != lower(*qc) {
                continue;
            }
            let start_bonus = if is_boundary(&cand, j) { 8 } else { 0 };
            let base_bonus = if j == base_start { 6 } else { 0 };
            let here = 1 + start_bonus + base_bonus + if cand[j] == *qc { 1 } else { 0 };
            if i == 0 {
                // leading gap costs a little
                cur[j] = Some(here - (j as i32).min(10) / 2);
            } else {
                let mut best: Option<i32> = None;
                for (k, pk) in prev.iter().enumerate().take(j) {
                    if let Some(p) = *pk {
                        let adj = if k + 1 == j {
                            5
                        } else {
                            -((j - k - 1) as i32).min(6)
                        };
                        let v = p + here + adj;
                        if best.is_none_or(|b| v > b) {
                            best = Some(v);
                        }
                    }
                }
                cur[j] = best;
            }
        }
        prev = cur;
    }
    let best = prev.into_iter().flatten().max()?;
    // shorter candidates and base-name hits win ties
    Some(best * 4 - (n as i32 / 8).min(20))
}

fn is_boundary(cand: &[char], j: usize) -> bool {
    if j == 0 {
        return true;
    }
    let (p, c) = (cand[j - 1], cand[j]);
    matches!(p, '/' | '-' | '_' | '.' | ' ' | '#')
        || (p.is_ascii_digit() != c.is_ascii_digit() && c.is_alphanumeric())
        || (p.is_lowercase() && c.is_uppercase())
}

/// The candidates that match `query`, best first (ties in the original order), at most `limit`.
pub fn rank<'a>(query: &str, items: &'a [String], limit: usize) -> Vec<&'a String> {
    let mut v: Vec<(i32, usize, &String)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, s)| score(query, s).map(|sc| (sc, i, s)))
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    v.into_iter().take(limit).map(|x| x.2).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        [
            "text/chapter-1.xhtml",
            "text/chapter-2.xhtml",
            "text/cover.xhtml",
            "styles/style.css",
            "images/cover-large.jpg",
            "fonts/Charis-Bold.ttf",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    #[test]
    fn subsequence_required() {
        assert!(score("xyz", "chapter").is_none());
        assert!(score("cpt", "chapter").is_some());
        assert!(score("tpc", "chapter").is_none());
        assert_eq!(score("", "anything"), Some(0));
        assert!(score("toolongquery", "short").is_none());
    }

    #[test]
    fn word_starts_beat_scattered_letters() {
        let n = names();
        let r = rank("c2", &n, 5);
        assert_eq!(r[0], "text/chapter-2.xhtml");
        let r = rank("cl", &n, 5);
        assert_eq!(r[0], "images/cover-large.jpg", "c and l both start words");
        let r = rank("cb", &n, 5);
        assert_eq!(r[0], "fonts/Charis-Bold.ttf");
    }

    #[test]
    fn base_name_start_wins_over_directory_hit() {
        let n = names();
        let r = rank("st", &n, 5);
        assert_eq!(r[0], "styles/style.css");
        let items: Vec<String> = vec![
            "a/xcover.css".into(),
            "cover/a.css".into(),
            "b/cover.css".into(),
        ];
        let r = rank("cov", &items, 3);
        assert_eq!(
            r[0], "b/cover.css",
            "match at the start of the base name first"
        );
    }

    #[test]
    fn case_unicode_and_ties() {
        assert!(score("ÉCOLE", "école.xhtml").is_some());
        assert!(score("ü", "über.xhtml").is_some());
        let items: Vec<String> = vec!["b.css".into(), "a.css".into()];
        let r = rank("css", &items, 2);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0], "b.css", "equal scores keep the original order");
        assert_eq!(rank("a", &items, 1).len(), 1);
    }
}
