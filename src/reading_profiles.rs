//! A book's own reading look (enhancement 19), without GTK.
//!
//! There are no named "reading profiles" any more: every change the reader makes to the look of a
//! book (font, size, spacing, margins, theme, columns, hyphenation …) is kept as that book's own
//! settings ([`BookLooks`]); a book that has none opens with the global defaults. The old named
//! profiles ([`Profiles`]) are only read to migrate them ([`migrate`]).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::{ColorMode, ReaderPrefs, ReaderTheme};
use crate::ui::canvas::LayoutMode;

/// The look of each book that has one, by book id.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BookLooks {
    pub books: BTreeMap<String, ReaderPrefs>,
}

impl BookLooks {
    pub fn get(&self, book: u64) -> Option<&ReaderPrefs> {
        self.books.get(&book.to_string())
    }

    /// Keeps `prefs` as the look of the book.
    pub fn set(&mut self, book: u64, prefs: &ReaderPrefs) {
        self.books.insert(book.to_string(), prefs.clone());
    }

    /// The book goes back to the defaults; true when it had a look of its own.
    pub fn reset(&mut self, book: u64) -> bool {
        self.books.remove(&book.to_string()).is_some()
    }

    /// What a book opens with: its own look, else the defaults.
    pub fn for_open(&self, book: u64, defaults: &ReaderPrefs) -> ReaderPrefs {
        self.get(book).cloned().unwrap_or_else(|| defaults.clone())
    }
}

/// A named bundle of settings of the old "Reading Profiles" feature (only read for migration).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    pub font_family: Option<String>,
    pub font_size: Option<f64>,
    pub line_spacing: Option<f64>,
    pub margin: Option<f64>,
    pub justify: Option<bool>,
    pub hyphenate: Option<bool>,
    pub theme: Option<ReaderTheme>,
    pub colors: Option<ColorMode>,
    pub custom_fg: Option<String>,
    pub custom_bg: Option<String>,
    pub layout: Option<LayoutMode>,
}

impl Profile {
    /// `prefs` with this profile's settings on top (sizes kept inside the ranges the reader allows).
    pub fn apply(&self, prefs: &ReaderPrefs) -> ReaderPrefs {
        let mut p = prefs.clone();
        if let Some(v) = &self.font_family {
            p.font_family = v.clone();
        }
        if let Some(v) = self.font_size {
            p.font_size = v.clamp(8.0, 40.0);
        }
        if let Some(v) = self.line_spacing {
            p.line_spacing = v.clamp(1.0, 2.5);
        }
        if let Some(v) = self.margin {
            p.margin = v.clamp(0.0, 200.0);
        }
        if let Some(v) = self.justify {
            p.justify = v;
        }
        if let Some(v) = self.hyphenate {
            p.hyphenate = v;
        }
        if let Some(v) = self.theme {
            p.theme = v;
        }
        if let Some(v) = self.colors {
            p.colors = v;
        }
        if let Some(v) = &self.custom_fg {
            p.custom_fg = v.clone();
        }
        if let Some(v) = &self.custom_bg {
            p.custom_bg = v.clone();
        }
        if let Some(v) = self.layout {
            p.layout = v;
        }
        p
    }
}

/// The old profile list and the books bound to a profile (only read for migration).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profiles {
    pub list: Vec<Profile>,
    /// Book id → name of the profile it opens with.
    pub books: BTreeMap<String, String>,
}

impl Profiles {
    pub fn is_empty(&self) -> bool {
        self.list.is_empty() && self.books.is_empty()
    }

    fn get(&self, name: &str) -> Option<&Profile> {
        self.list.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }
}

/// The result of [`migrate`].
#[derive(Debug, Default, PartialEq)]
pub struct Migration {
    /// A profile pinned to a book is now that book's own look.
    pub pinned: usize,
    /// JSON text of the profiles that were not pinned to any book (nothing is lost), or `None`
    /// when there were none.
    pub backup: Option<String>,
}

/// Turns the old profiles into per-book looks on top of `base` (the global defaults). Profiles
/// that no book uses are returned as the text of a backup file.
pub fn migrate(old: &Profiles, base: &ReaderPrefs, looks: &mut BookLooks) -> Migration {
    let mut used: Vec<String> = Vec::new();
    let mut pinned = 0;
    for (book, name) in &old.books {
        let Some(p) = old.get(name) else { continue };
        if book.parse::<u64>().is_err() || looks.books.contains_key(book) {
            continue;
        }
        looks.books.insert(book.clone(), p.apply(base));
        used.push(p.name.to_ascii_lowercase());
        pinned += 1;
    }
    let spare: Vec<&Profile> = old
        .list
        .iter()
        .filter(|p| !used.contains(&p.name.to_ascii_lowercase()))
        .collect();
    let backup =
        (!spare.is_empty()).then(|| serde_json::to_string_pretty(&spare).unwrap_or_default());
    Migration { pinned, backup }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs() -> ReaderPrefs {
        ReaderPrefs {
            font_family: "Literata".into(),
            font_size: 18.0,
            theme: ReaderTheme::Sepia,
            layout: LayoutMode::Scroll,
            ..Default::default()
        }
    }

    fn profile(name: &str, size: f64) -> Profile {
        Profile {
            name: name.into(),
            font_size: Some(size),
            ..Default::default()
        }
    }

    #[test]
    fn a_new_book_opens_with_the_defaults_and_keeps_its_own_changes() {
        let mut looks = BookLooks::default();
        let defaults = ReaderPrefs::default();
        assert_eq!(looks.for_open(7, &defaults), defaults);
        looks.set(7, &prefs());
        assert_eq!(looks.for_open(7, &defaults).font_size, 18.0);
        assert_eq!(looks.for_open(8, &defaults), defaults);
        assert!(looks.reset(7));
        assert!(!looks.reset(7));
        assert_eq!(looks.for_open(7, &defaults), defaults);
    }

    #[test]
    fn looks_round_trip_through_json_and_old_configs_read_fine() {
        let mut looks = BookLooks::default();
        looks.set(3, &prefs());
        let text = serde_json::to_string(&looks).unwrap();
        let back: BookLooks = serde_json::from_str(&text).unwrap();
        assert_eq!(back, looks);
        let none: BookLooks = serde_json::from_str("{}").unwrap();
        assert!(none.books.is_empty());
    }

    #[test]
    fn a_pinned_profile_becomes_the_books_own_look_and_the_rest_is_backed_up() {
        let old = Profiles {
            list: vec![profile("Big", 24.0), profile("Small", 10.0)],
            books: BTreeMap::from([("5".to_string(), "big".to_string())]),
        };
        let base = ReaderPrefs::default();
        let mut looks = BookLooks::default();
        let m = migrate(&old, &base, &mut looks);
        assert_eq!(m.pinned, 1);
        assert_eq!(looks.get(5).unwrap().font_size, 24.0);
        assert_eq!(looks.get(5).unwrap().margin, base.margin);
        let backup = m.backup.unwrap();
        assert!(backup.contains("Small") && !backup.contains("Big"));
    }

    #[test]
    fn migration_never_overwrites_a_look_and_ignores_broken_pins() {
        let old = Profiles {
            list: vec![profile("A", 20.0)],
            books: BTreeMap::from([
                ("1".to_string(), "A".to_string()),
                ("x".to_string(), "A".to_string()),
                ("2".to_string(), "Gone".to_string()),
            ]),
        };
        let mut looks = BookLooks::default();
        looks.set(1, &prefs());
        let m = migrate(&old, &ReaderPrefs::default(), &mut looks);
        assert_eq!(m.pinned, 0);
        assert_eq!(looks.get(1).unwrap().font_size, 18.0);
        assert!(looks.get(2).is_none());
        // "A" was not used (book 1 kept its own look), so it is in the backup
        assert!(m.backup.unwrap().contains("\"A\""));
        assert_eq!(
            migrate(&Profiles::default(), &ReaderPrefs::default(), &mut looks),
            Migration::default()
        );
    }

    #[test]
    fn old_profiles_still_deserialize() {
        let p: Profiles = serde_json::from_str("{}").unwrap();
        assert!(p.is_empty());
        let p: Profiles =
            serde_json::from_str(r#"{"list":[{"name":"x","font_size":500.0}],"books":{"1":"x"}}"#)
                .unwrap();
        assert_eq!(p.list[0].apply(&ReaderPrefs::default()).font_size, 40.0);
    }
}
