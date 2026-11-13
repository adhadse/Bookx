//! Small view settings of the book editor (minimap, hard-to-see characters), kept in their own
//! file so the main configuration stays untouched. No GTK in here.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorPrefs {
    /// The overview of the whole file at the right edge of the code view.
    pub minimap: bool,
    /// Scrolling the preview also scrolls the code view (the other way, code to preview, is always on).
    pub scroll_both: bool,
    /// Draw spaces, tabs, line breaks and invisible characters (CE-338).
    pub show_whitespace: bool,
    /// Typing `;` after `&name` turns the entity into its character (CE-302). Off by default.
    pub convert_entities: bool,
    /// Kinds of problems hidden in the Problems list, per book (library id) (CE-162).
    pub ignored_checks: BTreeMap<String, BTreeSet<String>>,
    /// "Also check accessibility" in the Problems panel (CE-164).
    pub check_accessibility: bool,
}

impl Default for EditorPrefs {
    fn default() -> Self {
        EditorPrefs {
            minimap: true,
            scroll_both: false,
            show_whitespace: false,
            convert_entities: false,
            ignored_checks: BTreeMap::new(),
            check_accessibility: false,
        }
    }
}

pub fn path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.config_dir().join("editor.json"))
}

impl EditorPrefs {
    pub fn load_from(path: &Path) -> EditorPrefs {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(
            path,
            serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into()),
        )
    }

    pub fn load() -> EditorPrefs {
        path().map(|p| Self::load_from(&p)).unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(p) = path() {
            let _ = self.save_to(&p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_and_tolerate_damage() {
        let dir = std::env::temp_dir().join(format!("bookx-eprefs-{}", uuid::Uuid::new_v4()));
        let f = dir.join("editor.json");
        assert_eq!(EditorPrefs::load_from(&f), EditorPrefs::default());
        let p = EditorPrefs {
            minimap: false,
            scroll_both: true,
            show_whitespace: true,
            convert_entities: true,
            ignored_checks: BTreeMap::from([(
                "7".to_string(),
                BTreeSet::from(["OPF-003".to_string()]),
            )]),
            check_accessibility: true,
        };
        p.save_to(&f).unwrap();
        assert_eq!(EditorPrefs::load_from(&f), p);
        std::fs::write(&f, "{\"minimap\": false}").unwrap();
        let q = EditorPrefs::load_from(&f);
        assert!(
            !q.minimap && !q.show_whitespace && !q.convert_entities,
            "missing keys use defaults (entity conversion is off unless asked for)"
        );
        std::fs::write(&f, "garbage").unwrap();
        assert_eq!(EditorPrefs::load_from(&f), EditorPrefs::default());
        assert!(q.ignored_checks.is_empty() && !q.check_accessibility);
        let _ = std::fs::remove_dir_all(dir);
    }
}
