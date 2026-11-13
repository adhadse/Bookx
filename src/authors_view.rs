//! Author aliases and normalisation (roadmap PP-G-31) on top of `bookx_sources::aliases`: the
//! *Authors* browser grouped by person, merging two spellings, and renaming an author everywhere
//! with a preview. A rename is one entry of the operation log, so one undo takes all books back.
//!
//! No GTK in here, so it is unit tested; the window is `ui/authors.rs`.

use crate::i18n::trf;
use anyhow::Result;
use bookx_sources::aliases::{self, AliasMap, AuthorGroup, RenamePlan};
use bookx_store::SqliteStore;

use crate::core::library::{Book, Library};

/// The aliases the user declared.
pub fn alias_map(store: &SqliteStore) -> AliasMap {
    AliasMap::from_pairs(&store.author_aliases())
}

/// The people of the library, aliases applied.
pub fn groups(books: &[Book], store: &SqliteStore) -> Vec<AuthorGroup> {
    aliases::group_authors(books, &alias_map(store))
}

/// Pairs of names that probably are the same person and are not declared yet.
pub fn suggestions(books: &[Book], store: &SqliteStore) -> Vec<(String, String)> {
    aliases::suggest_same_person(books, &alias_map(store))
}

/// What changes if every spelling in `from` becomes `to`.
pub fn preview(books: &[Book], from: &[String], to: &str) -> RenamePlan {
    aliases::plan_rename(books, from, to)
}

/// Writes the plan: one undo step. `remember` also records every old spelling as an alias of the
/// new name, so books imported later with the old spelling still group under it.
pub fn apply(
    lib: &mut Library,
    store: &SqliteStore,
    plan: &RenamePlan,
    from: &[String],
    remember: bool,
) -> Result<usize> {
    if plan.items.is_empty() && !remember {
        return Ok(0);
    }
    let n = lib.update_metadata_many(trf("Renamed author to \"{}\"", &[&plan.to]), plan.edits())?;
    if remember {
        for f in from {
            if aliases::name_key(f) != aliases::name_key(&plan.to) {
                store.set_author_alias(f, &plan.to)?;
            }
        }
    }
    Ok(n)
}

/// Merges several spellings into one name: a rename plus the aliases.
pub fn merge(
    lib: &mut Library,
    store: &SqliteStore,
    spellings: &[String],
    canonical: &str,
) -> Result<usize> {
    let plan = preview(&lib.books, spellings, canonical);
    apply(lib, store, &plan, spellings, true)
}

/// Declares an alias without touching any book (a pen name that should group with the real name
/// in the browser but stay as it is on the books).
pub fn declare_alias(store: &SqliteStore, alias: &str, canonical: &str) -> Result<()> {
    store.set_author_alias(alias, canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::library::AddOutcome;

    fn setup(authors: &[(&str, &str)]) -> (Library, SqliteStore, std::path::PathBuf) {
        let d = std::env::temp_dir().join(format!("bookx-av-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        let mut lib = Library::in_memory(&d.join("lib")).unwrap();
        for (t, a) in authors {
            let p = d.join(format!("{t}.md"));
            std::fs::write(
                &p,
                format!("---\ntitle: \"{t}\"\nauthor: \"{a}\"\n---\n# One\n\nHi.\n"),
            )
            .unwrap();
            let AddOutcome::Added(_) = lib.add_path(&p, true).unwrap() else {
                panic!()
            };
        }
        let store = SqliteStore::open_in_memory().unwrap();
        (lib, store, d)
    }

    #[test]
    fn merging_spellings_renames_everywhere_remembers_aliases_and_undoes_in_one_step() {
        let (mut lib, store, d) = setup(&[
            ("One", "Mark Twain"),
            ("Two", "Samuel Clemens"),
            ("Three", "Clemens, Samuel L."),
            ("Four", "Someone Else"),
        ]);
        let before: Vec<_> = lib.books.iter().map(|b| b.meta.authors.clone()).collect();
        let from = vec![
            "Samuel Clemens".to_string(),
            "Clemens, Samuel L.".to_string(),
        ];
        let plan = preview(&lib.books, &from, "Mark Twain");
        assert_eq!(plan.items.len(), 2, "preview lists the books that change");
        assert_eq!(
            lib.books[1].meta.authors,
            vec!["Samuel Clemens"],
            "preview changes nothing"
        );

        let history = lib.history().len();
        assert_eq!(merge(&mut lib, &store, &from, "Mark Twain").unwrap(), 2);
        assert_eq!(lib.history().len(), history + 1, "one operation");
        for b in &lib.books[..3] {
            assert_eq!(b.meta.authors, vec!["Mark Twain"]);
        }
        let g = groups(&lib.books, &store);
        assert_eq!(g.len(), 2);
        assert_eq!(
            g.iter()
                .find(|x| x.canonical == "Mark Twain")
                .unwrap()
                .books,
            3
        );
        assert_eq!(store.author_aliases().len(), 2);

        lib.undo_last().unwrap();
        let after: Vec<_> = lib.books.iter().map(|b| b.meta.authors.clone()).collect();
        assert_eq!(after, before);
        // the remembered aliases keep grouping the old spellings under the new name
        let g = groups(&lib.books, &store);
        assert_eq!(g.len(), 2);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn an_alias_alone_groups_without_changing_books() {
        let (lib, store, d) = setup(&[("One", "Richard Bachman"), ("Two", "Stephen King")]);
        assert_eq!(groups(&lib.books, &store).len(), 2);
        declare_alias(&store, "Richard Bachman", "Stephen King").unwrap();
        let g = groups(&lib.books, &store);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].canonical, "Stephen King");
        assert_eq!(g[0].books, 2);
        assert!(g[0].search().contains("Richard Bachman"));
        assert_eq!(lib.books[0].meta.authors, vec!["Richard Bachman"]);
        assert!(declare_alias(&store, "Stephen King", "stephen king").is_err());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn suggestions_name_initials_and_full_names() {
        let (lib, store, d) = setup(&[
            ("One", "J. R. R. Tolkien"),
            ("Two", "John Ronald Reuel Tolkien"),
            ("Three", "John Ronald Reuel Tolkien"),
        ]);
        let s = suggestions(&lib.books, &store);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].1, "John Ronald Reuel Tolkien");
        let _ = std::fs::remove_dir_all(d);
    }
}
