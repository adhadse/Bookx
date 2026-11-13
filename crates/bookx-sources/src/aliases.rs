//! Author aliases and normalisation: pen names, `Last, First` against `First Last`, spelling
//! variants and transliterations that the user declared the same person.
//!
//! Pure functions over books and an [`AliasMap`] (the pairs live in the catalogue database,
//! `SqliteStore::author_aliases`). Renaming returns a [`RenamePlan`] to preview; applying it is
//! one `Library::update_metadata_many` call, which is one undo step.

use std::collections::{BTreeMap, HashMap};

use bookx_core::library::Book;
use bookx_core::opf::Metadata;

use crate::authors::{author_sort, parse_name, same_author, to_first_last};
use crate::text::normalize;

/// A key that is the same for `Tolkien, J.R.R.`-style and `First Last` spellings of one name
/// (case, accents and punctuation are ignored; word order of `Last, First` is normalised).
pub fn name_key(name: &str) -> String {
    normalize(&to_first_last(name.trim()))
}

/// Alias name to canonical name.
#[derive(Debug, Clone, Default)]
pub struct AliasMap {
    map: HashMap<String, String>,
}

impl AliasMap {
    pub fn from_pairs(pairs: &[(String, String)]) -> AliasMap {
        AliasMap {
            map: pairs
                .iter()
                .map(|(a, c)| (name_key(a), c.clone()))
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// The canonical name for `name`, or `None` when it has no alias entry.
    pub fn lookup(&self, name: &str) -> Option<&str> {
        self.map.get(&name_key(name)).map(String::as_str)
    }

    /// The canonical name, or `name` itself.
    pub fn canonical(&self, name: &str) -> String {
        self.lookup(name).unwrap_or(name).to_string()
    }

    /// Every spelling that stands for `canonical`.
    pub fn aliases_of(&self, canonical: &str) -> Vec<String> {
        let k = name_key(canonical);
        let mut v: Vec<String> = self
            .map
            .iter()
            .filter(|(_, c)| name_key(c) == k)
            .map(|(a, _)| a.clone())
            .collect();
        v.sort();
        v
    }
}

/// One person in the *Authors* browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorGroup {
    pub canonical: String,
    /// Spellings found in the library with their book counts, most used first (includes the canonical one).
    pub variants: Vec<(String, usize)>,
    /// Books by this person (a book with two spellings counts once).
    pub books: usize,
}

impl AuthorGroup {
    /// The library search for all the spellings: `author:="A" OR author:="B"`.
    pub fn search(&self) -> String {
        self.variants
            .iter()
            .map(|(v, _)| format!("author:=\"{v}\""))
            .collect::<Vec<_>>()
            .join(" OR ")
    }
}

/// The authors of a library grouped by person: declared aliases and plain spelling variants
/// (`Le Guin, Ursula K.` and `Ursula K. Le Guin`) share a group. Sorted by name.
pub fn group_authors(books: &[Book], aliases: &AliasMap) -> Vec<AuthorGroup> {
    // key -> spelling -> books
    let mut groups: BTreeMap<String, (String, HashMap<String, usize>, usize)> = BTreeMap::new();
    for b in books {
        let mut seen_keys: Vec<String> = Vec::new();
        for a in &b.meta.authors {
            if a.trim().is_empty() {
                continue;
            }
            let canon = aliases.lookup(a).map(str::to_string);
            let key = name_key(canon.as_deref().unwrap_or(a));
            let g = groups
                .entry(key.clone())
                .or_insert_with(|| (canon.clone().unwrap_or_default(), HashMap::new(), 0));
            if g.0.is_empty() {
                if let Some(c) = &canon {
                    g.0 = c.clone();
                }
            }
            *g.1.entry(a.clone()).or_default() += 1;
            if !seen_keys.contains(&key) {
                g.2 += 1;
                seen_keys.push(key);
            }
        }
    }
    let mut out: Vec<AuthorGroup> = groups
        .into_values()
        .map(|(canon, spellings, books)| {
            let mut variants: Vec<(String, usize)> = spellings.into_iter().collect();
            variants.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let canonical = if canon.is_empty() {
                // no alias entry: the most used "First Last" spelling
                variants
                    .iter()
                    .find(|(v, _)| !v.contains(','))
                    .or(variants.first())
                    .map(|(v, _)| v.clone())
                    .unwrap_or_default()
            } else {
                canon
            };
            if !variants.iter().any(|(v, _)| *v == canonical) {
                variants.push((canonical.clone(), 0));
            }
            AuthorGroup {
                canonical,
                variants,
                books,
            }
        })
        .collect();
    out.sort_by(|a, b| normalize(&a.canonical).cmp(&normalize(&b.canonical)));
    out
}

/// Pairs of spellings in the library that probably name the same person but are not the same key
/// (initials against full names: `J. R. R. Tolkien`, `John Ronald Reuel Tolkien`). Suggestions
/// only, nothing is merged by itself.
pub fn suggest_same_person(books: &[Book], aliases: &AliasMap) -> Vec<(String, String)> {
    let groups = group_authors(books, aliases);
    let mut out = Vec::new();
    for (i, a) in groups.iter().enumerate() {
        for b in groups.iter().skip(i + 1) {
            if parse_name(&a.canonical).last.is_empty() {
                continue;
            }
            if same_author(&a.canonical, &b.canonical) {
                // the spelling with more books is the one to keep
                if a.books >= b.books {
                    out.push((b.canonical.clone(), a.canonical.clone()));
                } else {
                    out.push((a.canonical.clone(), b.canonical.clone()));
                }
            }
        }
    }
    out
}

/// One book of a rename.
#[derive(Debug, Clone, PartialEq)]
pub struct RenameItem {
    pub id: u64,
    pub title: String,
    pub before: Vec<String>,
    pub after: Vec<String>,
    /// The new metadata, ready for `Library::update_metadata_many`.
    pub meta: Metadata,
}

/// What *Rename author everywhere* would change.
#[derive(Debug, Clone, PartialEq)]
pub struct RenamePlan {
    pub to: String,
    pub items: Vec<RenameItem>,
}

impl RenamePlan {
    pub fn summary(&self) -> String {
        format!(
            "{} book{} will list \"{}\"",
            self.items.len(),
            if self.items.len() == 1 { "" } else { "s" },
            self.to
        )
    }

    pub fn edits(&self) -> Vec<(u64, Metadata)> {
        self.items.iter().map(|i| (i.id, i.meta.clone())).collect()
    }
}

/// Plans replacing every spelling in `from` (compared by [`name_key`]) with `to`. A book that
/// already lists `to` ends up with it once. Books that would not change are left out.
pub fn plan_rename(books: &[Book], from: &[String], to: &str) -> RenamePlan {
    let to = to.trim();
    let keys: Vec<String> = from.iter().map(|f| name_key(f)).collect();
    let to_key = name_key(to);
    let mut items = Vec::new();
    if to.is_empty() {
        return RenamePlan {
            to: to.into(),
            items,
        };
    }
    for b in books {
        if !b
            .meta
            .authors
            .iter()
            .any(|a| keys.contains(&name_key(a)) && a != to)
        {
            continue;
        }
        let mut after: Vec<String> = Vec::new();
        for a in &b.meta.authors {
            let n = if keys.contains(&name_key(a)) {
                to.to_string()
            } else {
                a.clone()
            };
            let again = name_key(&n) == to_key && after.iter().any(|x| name_key(x) == to_key);
            if !again && !after.contains(&n) {
                after.push(n);
            }
        }
        let mut meta = b.meta.clone();
        meta.authors = after.clone();
        meta.author_sort = author_sort(&after);
        items.push(RenameItem {
            id: b.id,
            title: b.meta.title.clone(),
            before: b.meta.authors.clone(),
            after,
            meta,
        });
    }
    RenamePlan {
        to: to.into(),
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(id: u64, authors: &[&str]) -> Book {
        let mut b: Book = serde_json::from_str(
            r#"{"id":1,"file":"a.epub","meta":{"title":"T","title_sort":"T","authors":[],"author_sort":"","publisher":"","language":"en","description":"","tags":[],"series":"","series_index":0.0,"identifiers":[],"published":"","rights":"","rating":0},"added":"2026-01-01T00:00:00Z","modified":"2026-01-01T00:00:00Z","size":1}"#,
        )
        .unwrap();
        b.id = id;
        b.meta.title = format!("Book {id}");
        b.meta.authors = authors.iter().map(|s| s.to_string()).collect();
        b
    }

    fn pairs(p: &[(&str, &str)]) -> Vec<(String, String)> {
        p.iter()
            .map(|(a, c)| (a.to_string(), c.to_string()))
            .collect()
    }

    #[test]
    fn name_order_and_case_do_not_matter_for_the_key() {
        assert_eq!(
            name_key("Le Guin, Ursula K."),
            name_key("ursula k. le guin")
        );
        assert_ne!(name_key("Ursula Le Guin"), name_key("Ursula Wolf"));
    }

    #[test]
    fn aliases_resolve_to_the_canonical_name_in_any_spelling() {
        let m = AliasMap::from_pairs(&pairs(&[("Clemens, Samuel", "Mark Twain")]));
        assert_eq!(m.canonical("Samuel Clemens"), "Mark Twain");
        assert_eq!(m.canonical("Someone"), "Someone");
        assert_eq!(m.aliases_of("mark twain").len(), 1);
    }

    #[test]
    fn the_authors_browser_groups_by_person() {
        let books = vec![
            book(1, &["Mark Twain"]),
            book(2, &["Samuel Clemens"]),
            book(3, &["Twain, Mark"]),
            book(4, &["Mark Twain", "Samuel Clemens"]),
            book(5, &["Ursula K. Le Guin"]),
        ];
        let m = AliasMap::from_pairs(&pairs(&[("Samuel Clemens", "Mark Twain")]));
        let g = group_authors(&books, &m);
        assert_eq!(g.len(), 2);
        let twain = g.iter().find(|x| x.canonical == "Mark Twain").unwrap();
        assert_eq!(twain.books, 4, "a book with two spellings counts once");
        assert_eq!(twain.variants.len(), 3);
        assert!(twain.search().contains("author:=\"Samuel Clemens\" OR"));
        // without the alias the spelling variants of Twain still fold, Clemens stays apart
        let g2 = group_authors(&books, &AliasMap::default());
        assert_eq!(g2.len(), 3);
        let t2 = g2.iter().find(|x| x.canonical == "Mark Twain").unwrap();
        assert_eq!(t2.books, 3);
    }

    #[test]
    fn initials_are_suggested_as_the_same_person() {
        let books = vec![
            book(1, &["J. R. R. Tolkien"]),
            book(2, &["John Ronald Reuel Tolkien"]),
            book(3, &["John Ronald Reuel Tolkien"]),
            book(4, &["Mark Twain"]),
        ];
        let s = suggest_same_person(&books, &AliasMap::default());
        assert_eq!(
            s,
            vec![(
                "J. R. R. Tolkien".to_string(),
                "John Ronald Reuel Tolkien".to_string()
            )]
        );
    }

    #[test]
    fn renaming_previews_every_spelling_and_keeps_other_authors() {
        let books = vec![
            book(1, &["Mark Twain"]),
            book(2, &["Clemens, Samuel", "Co Author"]),
            book(3, &["Mark Twain", "Samuel Clemens"]),
            book(4, &["Other"]),
        ];
        let p = plan_rename(
            &books,
            &["Mark Twain".to_string(), "Samuel Clemens".to_string()],
            "Samuel Langhorne Clemens",
        );
        assert_eq!(p.items.len(), 3);
        assert_eq!(
            p.items[1].after,
            vec!["Samuel Langhorne Clemens", "Co Author"]
        );
        assert_eq!(
            p.items[2].after,
            vec!["Samuel Langhorne Clemens"],
            "listed once"
        );
        assert_eq!(p.items[0].meta.author_sort, "Clemens, Samuel Langhorne");
        assert!(p.summary().starts_with("3 books"));
        // renaming to the current spelling changes nothing
        assert!(plan_rename(&books, &["Other".to_string()], "Other")
            .items
            .is_empty());
        assert!(plan_rename(&books, &["Other".to_string()], "  ")
            .items
            .is_empty());
    }
}
