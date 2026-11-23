//! Shelves (collections) in the library window, without GTK (L5, L29): the `shelf:` search term,
//! names, the drag-and-drop payload, and the order the user gave shelves and books.
//!
//! The books of a shelf live in the catalogue (`bookx-store` collections). The order of the shelves
//! and of the books in each one is the user's own and is kept in `shelves.json` next to the config.

use crate::i18n::tr;
use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Prefix of the text a dragged book list carries.
const DRAG_PREFIX: &str = "bookx-books:";

/// The text carried by a drag of these books.
pub fn encode_drag(ids: &[u64]) -> String {
    let list: Vec<String> = ids.iter().map(u64::to_string).collect();
    format!("{DRAG_PREFIX}{}", list.join(","))
}

/// The books of a drag payload (empty when the text is something else).
pub fn decode_drag(text: &str) -> Vec<u64> {
    text.strip_prefix(DRAG_PREFIX)
        .map(|r| r.split(',').filter_map(|s| s.trim().parse().ok()).collect())
        .unwrap_or_default()
}

/// The search that shows a shelf.
pub fn reference_for(name: &str) -> String {
    format!("shelf:\"{}\"", name.replace('"', ""))
}

/// A `shelf:` term of a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShelfTerm {
    pub name: String,
    pub negated: bool,
}

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

/// Splits `shelf:name` / `-shelf:"a name"` terms from the rest of a query.
pub fn split_terms(query: &str) -> (String, Vec<ShelfTerm>) {
    let mut rest = Vec::new();
    let mut terms = Vec::new();
    for t in tokens(query) {
        let (negated, body) = match t.strip_prefix('-') {
            Some(b) => (true, b),
            None => (false, t.as_str()),
        };
        if body.len() > 6 && body[..6].eq_ignore_ascii_case("shelf:") {
            let name = body[6..].replace('"', "").trim().to_string();
            if !name.is_empty() {
                terms.push(ShelfTerm { name, negated });
                continue;
            }
        }
        rest.push(t);
    }
    (rest.join(" "), terms)
}

/// Keeps the ids that satisfy every term. `members(name)` gives the ids on a shelf (`None` when
/// there is no such shelf: it then matches nothing, or everything when negated).
pub fn filter_ids(
    ids: Vec<u64>,
    terms: &[ShelfTerm],
    members: impl Fn(&str) -> Option<Vec<u64>>,
) -> Vec<u64> {
    let sets: Vec<(bool, Option<std::collections::HashSet<u64>>)> = terms
        .iter()
        .map(|t| (t.negated, members(&t.name).map(|v| v.into_iter().collect())))
        .collect();
    ids.into_iter()
        .filter(|id| {
            sets.iter().all(|(neg, set)| {
                let inside = set.as_ref().is_some_and(|s| s.contains(id));
                inside != *neg
            })
        })
        .collect()
}

/// Checks a shelf name; the cleaned name, or what is wrong with it. `current` is the name being
/// renamed (it may keep its own name, or change its case).
pub fn check_name(
    name: &str,
    existing: &[String],
    current: Option<&str>,
) -> Result<String, String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err(tr("Give the shelf a name."));
    }
    if name.contains('"') {
        return Err(tr("Quotation marks cannot be used in a shelf name."));
    }
    let clash = existing.iter().any(|e| {
        e.eq_ignore_ascii_case(&name) && !current.is_some_and(|c| c.eq_ignore_ascii_case(e))
    });
    if clash {
        return Err(format!("A shelf called “{name}” already exists."));
    }
    Ok(name)
}

/// "3 books" for a shelf row.
pub fn count_text(n: usize) -> String {
    if n == 1 {
        "1 book".into()
    } else {
        format!("{n} books")
    }
}

/// What the toast says after adding books to a shelf: `added` new, `already` there before.
pub fn added_text(shelf: &str, added: usize, already: usize) -> String {
    match (added, already) {
        (0, 0) => format!("Nothing to add to “{shelf}”"),
        (0, _) => format!("Already on “{shelf}”"),
        (n, 0) => format!("Added {} to “{shelf}”", count_text(n)),
        (n, a) => format!("Added {} to “{shelf}” ({a} already there)", count_text(n)),
    }
}

// ---- the user's order ----

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryShelves {
    /// Shelf names in the order the user wants; new shelves come after these, by name.
    #[serde(default)]
    pub order: Vec<String>,
    /// Books of a shelf in the user's order (only for shelves the user ordered).
    #[serde(default)]
    pub books: BTreeMap<String, Vec<u64>>,
}

/// Order for every library (by its folder), in `shelves.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShelfPrefs {
    #[serde(default)]
    pub libraries: BTreeMap<String, LibraryShelves>,
}

impl ShelfPrefs {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).unwrap_or_default())
    }

    pub fn of(&self, library: &str) -> LibraryShelves {
        self.libraries.get(library).cloned().unwrap_or_default()
    }

    pub fn of_mut(&mut self, library: &str) -> &mut LibraryShelves {
        self.libraries.entry(library.to_string()).or_default()
    }
}

impl LibraryShelves {
    /// The names in the user's order: the ordered ones first (those that still exist), then the rest by name.
    pub fn sort_names(&self, names: &[String]) -> Vec<String> {
        let mut out: Vec<String> = self
            .order
            .iter()
            .filter_map(|o| names.iter().find(|n| n.eq_ignore_ascii_case(o)).cloned())
            .collect();
        let mut rest: Vec<String> = names
            .iter()
            .filter(|n| !out.iter().any(|o| o == *n))
            .cloned()
            .collect();
        rest.sort_by_key(|n| n.to_lowercase());
        out.extend(rest);
        out
    }

    /// Moves a shelf one place up (`-1`) or down (`1`) in the list `names` (as shown).
    pub fn move_shelf(&mut self, names: &[String], name: &str, delta: i32) -> bool {
        let mut list = self.sort_names(names);
        let Some(i) = list.iter().position(|n| n == name) else {
            return false;
        };
        let j = i as i32 + delta;
        if j < 0 || j as usize >= list.len() {
            return false;
        }
        list.swap(i, j as usize);
        self.order = list;
        true
    }

    pub fn rename(&mut self, old: &str, new: &str) {
        for o in self
            .order
            .iter_mut()
            .filter(|o| o.eq_ignore_ascii_case(old))
        {
            *o = new.to_string();
        }
        if let Some(b) = self.books.remove(old) {
            self.books.insert(new.to_string(), b);
        }
    }

    /// Where the shelf stands in the user's order (`None` when it was never ordered).
    pub fn position(&self, name: &str) -> Option<usize> {
        self.order.iter().position(|o| o.eq_ignore_ascii_case(name))
    }

    /// Undo of `forget`: the shelf goes back to its place and its books to their order.
    pub fn restore(&mut self, name: &str, position: Option<usize>, books: Option<Vec<u64>>) {
        if let Some(p) = position {
            if self.position(name).is_none() {
                self.order.insert(p.min(self.order.len()), name.to_string());
            }
        }
        if let Some(b) = books {
            self.books.insert(name.to_string(), b);
        }
    }

    pub fn forget(&mut self, name: &str) {
        self.order.retain(|o| !o.eq_ignore_ascii_case(name));
        self.books.remove(name);
    }

    /// Puts `ids` (the shelf's books) in the user's order; books never ordered go last, in the given order.
    pub fn sort_books(&self, shelf: &str, ids: Vec<u64>) -> Vec<u64> {
        let Some(saved) = self.books.get(shelf) else {
            return ids;
        };
        let mut out: Vec<u64> = saved.iter().copied().filter(|s| ids.contains(s)).collect();
        let rest: Vec<u64> = ids.iter().copied().filter(|i| !out.contains(i)).collect();
        out.extend(rest);
        out
    }

    /// Moves a book one place earlier (`-1`) or later (`1`) on a shelf whose books are `current`.
    pub fn move_book(&mut self, shelf: &str, current: Vec<u64>, book: u64, delta: i32) -> bool {
        let mut list = self.sort_books(shelf, current);
        let Some(i) = list.iter().position(|b| *b == book) else {
            return false;
        };
        let j = i as i32 + delta;
        if j < 0 || j as usize >= list.len() {
            return false;
        }
        list.swap(i, j as usize);
        self.books.insert(shelf.to_string(), list);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn drag_payload_round_trips() {
        let t = encode_drag(&[3, 14, 15]);
        assert_eq!(t, "bookx-books:3,14,15");
        assert_eq!(decode_drag(&t), vec![3, 14, 15]);
        assert!(decode_drag("some other text").is_empty());
        assert_eq!(decode_drag("bookx-books:1,x,2"), vec![1, 2]);
        assert!(decode_drag("bookx-books:").is_empty());
    }

    #[test]
    fn shelf_terms_are_split_from_the_query() {
        let (rest, t) = split_terms("tag:fantasy shelf:\"To read\" -shelf:Done author:x");
        assert_eq!(rest, "tag:fantasy author:x");
        assert_eq!(
            t,
            vec![
                ShelfTerm {
                    name: "To read".into(),
                    negated: false
                },
                ShelfTerm {
                    name: "Done".into(),
                    negated: true
                }
            ]
        );
        let (rest, t) = split_terms("plain words");
        assert_eq!(rest, "plain words");
        assert!(t.is_empty());
        // an empty name or a similar word is not a shelf term
        let (rest, t) = split_terms("shelf: shelfish:x");
        assert_eq!(rest, "shelf: shelfish:x");
        assert!(t.is_empty());
        assert_eq!(split_terms(&reference_for("A B")).1[0].name, "A B");
    }

    #[test]
    fn the_filter_keeps_books_on_the_shelf_and_drops_negated() {
        let members = |n: &str| match n {
            "A" => Some(vec![1, 2, 3]),
            "B" => Some(vec![2]),
            _ => None,
        };
        let t = |n: &str, neg| ShelfTerm {
            name: n.into(),
            negated: neg,
        };
        assert_eq!(
            filter_ids(vec![1, 2, 3, 4], &[t("A", false)], members),
            vec![1, 2, 3]
        );
        assert_eq!(
            filter_ids(vec![1, 2, 3, 4], &[t("A", false), t("B", true)], members),
            vec![1, 3]
        );
        assert!(filter_ids(vec![1], &[t("Gone", false)], members).is_empty());
        assert_eq!(filter_ids(vec![1], &[t("Gone", true)], members), vec![1]);
        assert_eq!(filter_ids(vec![1, 2], &[], members), vec![1, 2]);
    }

    #[test]
    fn names_are_checked() {
        let ex = names(&["To read", "Done"]);
        assert_eq!(
            check_name("  Favourites  ", &ex, None).unwrap(),
            "Favourites"
        );
        assert_eq!(check_name("a   b", &ex, None).unwrap(), "a b");
        assert!(check_name("  ", &ex, None).is_err());
        assert!(
            check_name("to READ", &ex, None)
                .unwrap_err()
                .contains("already exists")
        );
        // renaming to the same name in another case is fine
        assert_eq!(
            check_name("to read", &ex, Some("To read")).unwrap(),
            "to read"
        );
        assert!(check_name("Done", &ex, Some("To read")).is_err());
        assert!(check_name("a\"b", &ex, None).is_err());
    }

    #[test]
    fn texts() {
        assert_eq!(count_text(1), "1 book");
        assert_eq!(count_text(0), "0 books");
        assert_eq!(added_text("S", 2, 0), "Added 2 books to “S”");
        assert_eq!(
            added_text("S", 1, 2),
            "Added 1 book to “S” (2 already there)"
        );
        assert_eq!(added_text("S", 0, 3), "Already on “S”");
        assert_eq!(added_text("S", 0, 0), "Nothing to add to “S”");
    }

    #[test]
    fn shelves_follow_the_users_order_then_the_alphabet() {
        let mut l = LibraryShelves::default();
        let all = names(&["Zeta", "alpha", "Beta"]);
        assert_eq!(l.sort_names(&all), names(&["alpha", "Beta", "Zeta"]));
        assert!(l.move_shelf(&all, "Zeta", -1));
        assert_eq!(l.sort_names(&all), names(&["alpha", "Zeta", "Beta"]));
        assert!(l.move_shelf(&all, "Zeta", -1));
        assert_eq!(l.sort_names(&all), names(&["Zeta", "alpha", "Beta"]));
        // at the top already
        assert!(!l.move_shelf(&all, "Zeta", -1));
        // a new shelf goes after the ordered ones; a deleted one disappears
        let more = names(&["Zeta", "alpha", "Beta", "Aardvark"]);
        assert_eq!(
            l.sort_names(&more),
            names(&["Zeta", "alpha", "Beta", "Aardvark"])
        );
        let fewer = names(&["alpha", "Beta"]);
        assert_eq!(l.sort_names(&fewer), names(&["alpha", "Beta"]));
        assert!(!l.move_shelf(&all, "Missing", 1));
    }

    #[test]
    fn books_follow_the_users_order_then_the_given_one() {
        let mut l = LibraryShelves::default();
        assert_eq!(l.sort_books("S", vec![1, 2, 3]), vec![1, 2, 3]);
        assert!(l.move_book("S", vec![1, 2, 3], 3, -1));
        assert_eq!(l.sort_books("S", vec![1, 2, 3]), vec![1, 3, 2]);
        // a book removed from the shelf drops out; a new one goes last
        assert_eq!(l.sort_books("S", vec![1, 2, 9]), vec![1, 2, 9]);
        assert_eq!(l.sort_books("S", vec![2, 3, 9]), vec![3, 2, 9]);
        assert!(!l.move_book("S", vec![1, 2, 3], 1, -1));
        assert!(!l.move_book("S", vec![1, 2, 3], 7, 1));
    }

    #[test]
    fn a_deleted_shelf_comes_back_to_its_place_with_its_book_order() {
        let mut l = LibraryShelves::default();
        let all = names(&["A", "B", "C"]);
        l.move_shelf(&all, "C", -2);
        l.move_book("B", vec![1, 2], 2, -1);
        let (pos, books) = (l.position("B"), l.books.get("B").cloned());
        l.forget("B");
        assert_eq!(l.position("B"), None);
        l.restore("B", pos, books);
        assert_eq!(l.position("B"), pos);
        assert_eq!(l.sort_books("B", vec![1, 2]), vec![2, 1]);
        // restoring twice does not duplicate the entry
        l.restore("B", pos, None);
        assert_eq!(l.order.iter().filter(|o| *o == "B").count(), 1);
    }

    #[test]
    fn rename_and_delete_keep_the_order_tidy() {
        let mut l = LibraryShelves::default();
        let all = names(&["A", "B"]);
        l.move_shelf(&all, "B", -1);
        l.move_book("B", vec![1, 2], 2, -1);
        l.rename("B", "Bee");
        assert_eq!(l.sort_names(&names(&["A", "Bee"])), names(&["Bee", "A"]));
        assert_eq!(l.sort_books("Bee", vec![1, 2]), vec![2, 1]);
        l.forget("Bee");
        assert!(l.books.is_empty());
        assert_eq!(l.sort_names(&names(&["A"])), names(&["A"]));
    }

    #[test]
    fn prefs_are_per_library_and_saved() {
        let dir = std::env::temp_dir().join(format!("bookx-shelves-{}", std::process::id()));
        let path = dir.join("shelves.json");
        let mut p = ShelfPrefs::default();
        p.of_mut("/lib/a").order = names(&["X"]);
        p.save(&path).unwrap();
        let back = ShelfPrefs::load(&path);
        assert_eq!(back.of("/lib/a").order, names(&["X"]));
        assert!(back.of("/lib/b").order.is_empty());
        std::fs::write(&path, "{{{").unwrap();
        assert_eq!(ShelfPrefs::load(&path), ShelfPrefs::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
