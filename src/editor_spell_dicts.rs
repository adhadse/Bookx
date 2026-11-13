//! Where the spell checker finds and keeps things: the dictionary folders (system and the user's own), the user
//! dictionaries in the configuration folder, and what is remembered per book (ignored words, excluded files).
//! No GTK in here.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use bookx_editing::spell_user::UserDicts;
use serde::{Deserialize, Serialize};

fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
}

/// The configuration folder (`~/.config/Bookx`).
pub fn config_dir() -> Option<PathBuf> {
    project_dirs().map(|d| d.config_dir().to_path_buf())
}

/// Where imported dictionaries are kept (`BOOKX_HUNSPELL_USER` overrides it, for tests).
pub fn user_dictionary_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("BOOKX_HUNSPELL_USER") {
        return Some(d.into());
    }
    project_dirs().map(|d| d.data_dir().join("hunspell"))
}

/// The system folders plus the user's own: where [`crate::editor_tools::find_dictionary`] looks.
pub fn search_dirs() -> Vec<PathBuf> {
    with_user_dir(
        crate::editor_tools::dictionary_dirs(),
        user_dictionary_dir(),
    )
}

fn with_user_dir(mut v: Vec<PathBuf>, user: Option<PathBuf>) -> Vec<PathBuf> {
    if let Some(u) = user {
        v.retain(|d| *d != u);
        v.insert(0, u);
    }
    v
}

/// The named user dictionaries.
pub fn user_dicts() -> Option<UserDicts> {
    config_dir().map(UserDicts::new)
}

/// What is remembered for one book.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BookPrefs {
    /// Words the user chose to ignore in this book.
    pub ignored: BTreeSet<String>,
    /// Files that are not spell checked.
    pub excluded: BTreeSet<String>,
}

fn prefs_file(root: &Path, book_id: u64) -> PathBuf {
    root.join("spelling-books").join(format!("{book_id}.json"))
}

impl BookPrefs {
    pub fn load_from(root: &Path, book_id: u64) -> BookPrefs {
        std::fs::read_to_string(prefs_file(root, book_id))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save_to(&self, root: &Path, book_id: u64) -> std::io::Result<()> {
        let f = prefs_file(root, book_id);
        if self.ignored.is_empty() && self.excluded.is_empty() {
            let _ = std::fs::remove_file(&f);
            return Ok(());
        }
        if let Some(d) = f.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(
            f,
            serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into()),
        )
    }

    pub fn load(book_id: u64) -> BookPrefs {
        config_dir()
            .map(|r| Self::load_from(&r, book_id))
            .unwrap_or_default()
    }

    pub fn save(&self, book_id: u64) {
        if let Some(r) = config_dir() {
            let _ = self.save_to(&r, book_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn book_prefs_round_trip_and_tolerate_damage() {
        let root = std::env::temp_dir().join(format!("bookx-bprefs-{}", uuid::Uuid::new_v4()));
        assert_eq!(BookPrefs::load_from(&root, 7), BookPrefs::default());
        let mut p = BookPrefs::default();
        p.ignored.insert("Gandalf".into());
        p.excluded.insert("OEBPS/notes.xhtml".into());
        p.save_to(&root, 7).unwrap();
        assert_eq!(BookPrefs::load_from(&root, 7), p);
        assert_eq!(
            BookPrefs::load_from(&root, 8),
            BookPrefs::default(),
            "per book"
        );
        std::fs::write(prefs_file(&root, 7), "{ nope").unwrap();
        assert_eq!(BookPrefs::load_from(&root, 7), BookPrefs::default());
        std::fs::write(prefs_file(&root, 7), r#"{"ignored":["x"]}"#).unwrap();
        assert_eq!(BookPrefs::load_from(&root, 7).ignored.len(), 1);
        BookPrefs::default().save_to(&root, 7).unwrap();
        assert!(
            !prefs_file(&root, 7).exists(),
            "an empty list leaves no file"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_users_folder_is_searched_first_once() {
        let sys = vec![PathBuf::from("/usr/share/hunspell"), PathBuf::from("/u")];
        let v = with_user_dir(sys.clone(), Some(PathBuf::from("/u")));
        assert_eq!(
            v,
            [PathBuf::from("/u"), PathBuf::from("/usr/share/hunspell")]
        );
        assert_eq!(with_user_dir(sys.clone(), None), sys);
    }
}
