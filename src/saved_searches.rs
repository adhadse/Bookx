//! Saved searches and virtual libraries (L4, L22, L23), without GTK.
//!
//! A saved search is a name and a query in the search language. In a query, `search:name` (or
//! `vl:name`) stands for the saved query, so saved searches combine with each other and with
//! other terms: `search:"to read" tag:fantasy`, `-vl:Finished`, `search:a OR search:b`.
//! A virtual library is a saved search the user keeps as a restriction: everything in the library
//! (grid, table, browse panel searches) is narrowed by it.
//!
//! The search language has OR and AND but no parentheses, so references are expanded by
//! working with the query in disjunctive normal form (a list of OR-groups of AND-terms).

use crate::i18n::tr;
use crate::i18n::trf;
use serde::{Deserialize, Serialize};

const MAX_DEPTH: usize = 6;
const MAX_GROUPS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSearch {
    pub name: String,
    pub query: String,
}

type Dnf = Vec<Vec<String>>;

/// Splits on spaces outside quotes, keeping the quotes (the search parser removes them later).
fn tokens(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    for c in input.chars() {
        match c {
            '"' => {
                in_q = !in_q;
                cur.push(c);
            }
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

/// `(negated, name)` when the token is `search:name` or `vl:name` (optionally with a leading `-`).
fn reference(tok: &str) -> Option<(bool, String)> {
    let (neg, rest) = match tok.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, tok),
    };
    let lower = rest.to_ascii_lowercase();
    let name = if lower.starts_with("search:") {
        &rest[7..]
    } else if lower.starts_with("vl:") {
        &rest[3..]
    } else {
        return None;
    };
    let name = name.replace('"', "");
    if name.trim().is_empty() {
        return None;
    }
    Some((neg, name.trim().to_string()))
}

fn negate_token(t: &str) -> String {
    match t.strip_prefix('-') {
        Some(r) if !r.is_empty() => r.to_string(),
        _ => format!("-{t}"),
    }
}

fn cross(a: Dnf, b: &Dnf) -> Result<Dnf, String> {
    let mut out = Vec::new();
    for x in &a {
        for y in b {
            let mut g = x.clone();
            g.extend(y.iter().cloned());
            out.push(g);
        }
    }
    if out.len() > MAX_GROUPS {
        return Err(tr("This search is too complicated"));
    }
    Ok(out)
}

/// NOT (g1 OR g2 …) = every group has at least one term that does not hold.
fn negate(d: &Dnf) -> Result<Dnf, String> {
    let mut acc: Dnf = vec![Vec::new()];
    for g in d {
        let options: Dnf = g.iter().map(|t| vec![negate_token(t)]).collect();
        if options.is_empty() {
            return Err(tr(
                "A saved search that matches everything cannot be excluded",
            ));
        }
        acc = cross(acc, &options)?;
    }
    if d.is_empty() {
        return Err(tr(
            "A saved search that matches everything cannot be excluded",
        ));
    }
    Ok(acc)
}

fn find<'a>(items: &'a [SavedSearch], name: &str) -> Option<&'a SavedSearch> {
    items.iter().find(|s| s.name.eq_ignore_ascii_case(name))
}

fn dnf(query: &str, items: &[SavedSearch], depth: usize) -> Result<Dnf, String> {
    if depth > MAX_DEPTH {
        return Err(tr("Saved searches refer to each other in a circle"));
    }
    let mut result: Dnf = Vec::new();
    let mut group: Dnf = vec![Vec::new()];
    let mut any = false;
    let flush = |group: &mut Dnf, result: &mut Dnf, any: &mut bool| -> Result<(), String> {
        if *any {
            result.extend(std::mem::replace(group, vec![Vec::new()]));
            if result.len() > MAX_GROUPS {
                return Err(tr("This search is too complicated"));
            }
        }
        *any = false;
        Ok(())
    };
    for tok in tokens(query) {
        if tok == "OR" {
            flush(&mut group, &mut result, &mut any)?;
            continue;
        }
        any = true;
        let options: Dnf = match reference(&tok) {
            Some((neg, name)) => {
                let saved = find(items, &name)
                    .ok_or_else(|| trf("There is no saved search called “{}”", &[&name]))?;
                let d = dnf(&saved.query, items, depth + 1)?;
                if neg {
                    negate(&d)?
                } else {
                    d
                }
            }
            None => vec![vec![tok]],
        };
        group = cross(group, &options)?;
    }
    flush(&mut group, &mut result, &mut any)?;
    Ok(result)
}

fn render(d: &Dnf) -> String {
    d.iter()
        .map(|g| g.join(" "))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// Replaces every `search:` / `vl:` reference of `query` by the saved query.
pub fn expand(query: &str, items: &[SavedSearch]) -> Result<String, String> {
    Ok(render(&dnf(query, items, 0)?))
}

/// The query to run: `query` narrowed by the active virtual library, references expanded.
pub fn resolve(
    query: &str,
    library: Option<&str>,
    items: &[SavedSearch],
) -> Result<String, String> {
    let user = dnf(query, items, 0)?;
    let Some(name) = library else {
        return Ok(render(&user));
    };
    let Some(vl) = find(items, name) else {
        return Ok(render(&user));
    };
    let vl = dnf(&vl.query, items, 1)?;
    let user = if user.is_empty() {
        vec![Vec::new()]
    } else {
        user
    };
    if vl.is_empty() {
        return Ok(render(&user));
    }
    let mut out = Vec::new();
    for u in &user {
        for v in &vl {
            let mut g = v.clone();
            g.extend(u.iter().cloned());
            out.push(g);
        }
    }
    if out.len() > MAX_GROUPS {
        return Err(tr("This search is too complicated"));
    }
    Ok(render(&out))
}

/// The `search:` reference for a name, quoted when needed.
pub fn reference_for(name: &str) -> String {
    if name.chars().any(char::is_whitespace) {
        format!("search:\"{name}\"")
    } else {
        format!("search:{name}")
    }
}

/// Adds or replaces (same name, ignoring case) a saved search after checking it.
pub fn upsert(items: &mut Vec<SavedSearch>, name: &str, query: &str) -> Result<(), String> {
    let name = name.trim();
    let query = query.trim();
    if name.is_empty() {
        return Err(tr("Give the search a name"));
    }
    if name.contains('"') || name.contains(':') {
        return Err(tr("The name cannot contain quotes or colons"));
    }
    if query.is_empty() {
        return Err(tr("The search is empty"));
    }
    let mut trial = items.clone();
    trial.retain(|s| !s.name.eq_ignore_ascii_case(name));
    trial.push(SavedSearch {
        name: name.to_string(),
        query: query.to_string(),
    });
    expand(query, &trial)?;
    match items.iter_mut().find(|s| s.name.eq_ignore_ascii_case(name)) {
        Some(s) => {
            s.name = name.to_string();
            s.query = query.to_string();
        }
        None => items.push(SavedSearch {
            name: name.to_string(),
            query: query.to_string(),
        }),
    }
    Ok(())
}

/// Removes a saved search; other searches that refer to it stop working, which `expand` reports.
pub fn remove(items: &mut Vec<SavedSearch>, name: &str) -> bool {
    let before = items.len();
    items.retain(|s| !s.name.eq_ignore_ascii_case(name));
    items.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(name: &str, query: &str) -> SavedSearch {
        SavedSearch {
            name: name.into(),
            query: query.into(),
        }
    }

    #[test]
    fn plain_queries_pass_through() {
        assert_eq!(expand("tag:a OR tag:b", &[]).unwrap(), "tag:a OR tag:b");
        assert_eq!(expand("", &[]).unwrap(), "");
        assert_eq!(
            expand("author:\"le guin\"", &[]).unwrap(),
            "author:\"le guin\""
        );
    }

    #[test]
    fn a_reference_becomes_the_saved_query() {
        let items = [s("Fantasy", "tag:fantasy rating:>=4")];
        assert_eq!(
            expand("search:fantasy dune", &items).unwrap(),
            "tag:fantasy rating:>=4 dune"
        );
        assert_eq!(
            expand("vl:FANTASY", &items).unwrap(),
            "tag:fantasy rating:>=4"
        );
    }

    #[test]
    fn quoted_names_with_spaces() {
        let items = [s("to read", "status:unread")];
        assert_eq!(
            expand("search:\"to read\"", &items).unwrap(),
            "status:unread"
        );
        assert_eq!(reference_for("to read"), "search:\"to read\"");
        assert_eq!(reference_for("x"), "search:x");
    }

    #[test]
    fn or_inside_a_saved_search_distributes() {
        let items = [s("sf", "tag:scifi OR tag:space")];
        assert_eq!(
            expand("search:sf author:herbert", &items).unwrap(),
            "tag:scifi author:herbert OR tag:space author:herbert"
        );
    }

    #[test]
    fn negation_follows_de_morgan() {
        let items = [s("a", "tag:x tag:y"), s("b", "tag:x OR tag:y")];
        assert_eq!(expand("-search:a", &items).unwrap(), "-tag:x OR -tag:y");
        assert_eq!(expand("-search:b", &items).unwrap(), "-tag:x -tag:y");
        let items = [s("c", "-tag:x")];
        assert_eq!(expand("-vl:c", &items).unwrap(), "tag:x");
    }

    #[test]
    fn saved_searches_can_use_each_other() {
        let items = [s("a", "tag:x"), s("b", "search:a rating:5")];
        assert_eq!(expand("search:b", &items).unwrap(), "tag:x rating:5");
    }

    #[test]
    fn cycles_and_unknown_names_are_errors() {
        let items = [s("a", "search:b"), s("b", "search:a")];
        assert!(expand("search:a", &items).unwrap_err().contains("circle"));
        assert!(expand("search:nope", &[])
            .unwrap_err()
            .contains("no saved search"));
    }

    #[test]
    fn the_virtual_library_narrows_every_group() {
        let items = [s("English", "language:en")];
        assert_eq!(
            resolve("a OR b", Some("english"), &items).unwrap(),
            "language:en a OR language:en b"
        );
        assert_eq!(resolve("", Some("English"), &items).unwrap(), "language:en");
        assert_eq!(resolve("a", None, &items).unwrap(), "a");
        // a library that was deleted no longer restricts
        assert_eq!(resolve("a", Some("gone"), &items).unwrap(), "a");
    }

    #[test]
    fn upsert_checks_names_and_circles() {
        let mut v = Vec::new();
        upsert(&mut v, " Fantasy ", "tag:fantasy").unwrap();
        upsert(&mut v, "fantasy", "tag:fantasy rating:5").unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].query, "tag:fantasy rating:5");
        assert!(upsert(&mut v, "", "x").is_err());
        assert!(upsert(&mut v, "a:b", "x").is_err());
        assert!(upsert(&mut v, "x", "  ").is_err());
        assert!(upsert(&mut v, "loop", "search:loop").is_err());
        upsert(&mut v, "two", "search:fantasy").unwrap();
        assert!(upsert(&mut v, "Fantasy", "search:two").is_err());
        assert!(remove(&mut v, "TWO"));
        assert!(!remove(&mut v, "two"));
    }

    #[test]
    fn explosions_are_refused() {
        let items = [s("a", "x OR y OR z OR w")];
        let q = "search:a search:a search:a search:a";
        assert!(expand(q, &items).is_err());
    }
}
