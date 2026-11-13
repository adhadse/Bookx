//! The list of libraries behind the switcher in the library header (L11), without GTK: names,
//! the folder of a new library, and what the registry in `bookx-store` shows as rows.

use crate::i18n::tr;
use std::path::{Path, PathBuf};

use bookx_store::LibraryRegistry;

/// One row of the switcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    pub path: PathBuf,
    pub current: bool,
    /// False when the folder is gone (a drive that is not plugged in): the row says so.
    pub available: bool,
}

/// A name for a library at `path` that no other entry has: the folder's name, then "name 2"…
pub fn unique_name(reg: &LibraryRegistry, wanted: &str) -> String {
    let base = wanted.trim();
    let base = if base.is_empty() { "Library" } else { base };
    if reg.get(base).is_none() {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|n| reg.get(n).is_none())
        .unwrap_or_else(|| base.to_string())
}

/// What a folder is called in the list.
pub fn name_for_folder(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "Library".into())
}

/// Makes sure the library in use is in the list and is the current one; returns its name.
pub fn note_current(reg: &mut LibraryRegistry, path: &Path) -> anyhow::Result<String> {
    let name = match reg.list().iter().find(|l| same_folder(&l.path, path)) {
        Some(l) => l.name.clone(),
        None => {
            let name = unique_name(reg, &name_for_folder(path));
            reg.add(&name, path)?;
            name
        }
    };
    if reg.current().map(|c| c.name.as_str()) != Some(name.as_str()) {
        reg.set_current(&name)?;
    }
    Ok(name)
}

fn same_folder(a: &Path, b: &Path) -> bool {
    a == b
        || match (a.canonicalize(), b.canonicalize()) {
            (Ok(x), Ok(y)) => x == y,
            _ => false,
        }
}

/// The rows of the switcher: the current library first, then the others by name.
pub fn rows(reg: &LibraryRegistry) -> Vec<Row> {
    let cur = reg.current().map(|c| c.name.clone());
    let mut v: Vec<Row> = reg
        .list()
        .iter()
        .map(|l| Row {
            name: l.name.clone(),
            path: l.path.clone(),
            current: cur.as_deref() == Some(l.name.as_str()),
            available: l.path.is_dir(),
        })
        .collect();
    v.sort_by(|a, b| {
        b.current
            .cmp(&a.current)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    v
}

/// The folder for a new library called `name` inside `parent` (characters a file name cannot
/// have are replaced).
pub fn new_library_folder(parent: &Path, name: &str) -> Option<PathBuf> {
    let clean: String = name
        .trim()
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '\0') {
                '-'
            } else {
                c
            }
        })
        .collect();
    let clean = clean.trim_matches('.').trim().to_string();
    if clean.is_empty() {
        None
    } else {
        Some(parent.join(clean))
    }
}

/// Checks a name typed for a new or renamed library; the cleaned name or what is wrong.
pub fn check_name(
    reg: &LibraryRegistry,
    name: &str,
    current: Option<&str>,
) -> Result<String, String> {
    let n = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if n.is_empty() {
        return Err(tr("Give the library a name."));
    }
    if reg.get(&n).is_some() && current != Some(n.as_str()) {
        return Err(format!("There is already a library called “{n}”."));
    }
    Ok(n)
}

/// The text under the name in a row.
pub fn row_detail(r: &Row) -> String {
    let p = r.path.display().to_string();
    if r.available {
        p
    } else {
        format!("{p} (not available)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(tag: &str) -> (LibraryRegistry, PathBuf) {
        let dir = std::env::temp_dir().join(format!("bookx-libs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (
            LibraryRegistry::open(&dir.join("libraries.json")).unwrap(),
            dir,
        )
    }

    #[test]
    fn names_are_made_unique() {
        let (mut r, dir) = reg("unique");
        assert_eq!(unique_name(&r, "Books"), "Books");
        r.add("Books", &dir.join("a")).unwrap();
        assert_eq!(unique_name(&r, "Books"), "Books 2");
        r.add("Books 2", &dir.join("b")).unwrap();
        assert_eq!(unique_name(&r, "Books"), "Books 3");
        assert_eq!(unique_name(&r, "  "), "Library");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folder_names() {
        assert_eq!(
            name_for_folder(Path::new("/home/me/Bookx Library")),
            "Bookx Library"
        );
        assert_eq!(name_for_folder(Path::new("/")), "Library");
    }

    #[test]
    fn the_library_in_use_is_added_once_and_made_current() {
        let (mut r, dir) = reg("current");
        let a = dir.join("Fiction");
        let b = dir.join("Work");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        assert_eq!(note_current(&mut r, &a).unwrap(), "Fiction");
        assert_eq!(note_current(&mut r, &a).unwrap(), "Fiction");
        assert_eq!(r.list().len(), 1);
        assert_eq!(note_current(&mut r, &b).unwrap(), "Work");
        assert_eq!(r.current().unwrap().name, "Work");
        // back to the first: it is current again, not added twice
        assert_eq!(note_current(&mut r, &a).unwrap(), "Fiction");
        assert_eq!(r.list().len(), 2);
        assert_eq!(r.current().unwrap().name, "Fiction");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_folders_with_one_name_get_two_entries() {
        let (mut r, dir) = reg("same");
        let a = dir.join("x").join("Books");
        let b = dir.join("y").join("Books");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        assert_eq!(note_current(&mut r, &a).unwrap(), "Books");
        assert_eq!(note_current(&mut r, &b).unwrap(), "Books 2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rows_put_the_current_library_first_and_flag_missing_folders() {
        let (mut r, dir) = reg("rows");
        let z = dir.join("Zed");
        let a = dir.join("alpha");
        std::fs::create_dir_all(&z).unwrap();
        r.add("Zed", &z).unwrap();
        r.add("alpha", &a).unwrap(); // the folder does not exist
        let rows = rows(&r);
        assert_eq!(rows[0].name, "Zed");
        assert!(rows[0].current && rows[0].available);
        assert_eq!(rows[1].name, "alpha");
        assert!(!rows[1].available);
        assert!(row_detail(&rows[1]).ends_with("(not available)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_library_folder_is_made_of_a_clean_name() {
        let p = Path::new("/data");
        assert_eq!(
            new_library_folder(p, " Comics "),
            Some(PathBuf::from("/data/Comics"))
        );
        assert_eq!(
            new_library_folder(p, "a/b"),
            Some(PathBuf::from("/data/a-b"))
        );
        assert_eq!(new_library_folder(p, ".."), None);
        assert_eq!(new_library_folder(p, "  "), None);
    }

    #[test]
    fn names_are_checked_for_new_and_renamed_libraries() {
        let (mut r, dir) = reg("check");
        r.add("One", &dir.join("1")).unwrap();
        assert_eq!(check_name(&r, "  Two  ", None).unwrap(), "Two");
        assert!(check_name(&r, "", None).is_err());
        assert!(check_name(&r, "One", None).unwrap_err().contains("already"));
        assert_eq!(check_name(&r, "One", Some("One")).unwrap(), "One");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn removing_never_touches_the_files_and_moves_current() {
        let (mut r, dir) = reg("remove");
        let a = dir.join("A");
        let b = dir.join("B");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        note_current(&mut r, &a).unwrap();
        note_current(&mut r, &b).unwrap();
        assert!(r.remove("B").unwrap());
        assert!(b.is_dir());
        assert_eq!(r.current().unwrap().name, "A");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
