//! Favourite and recently used characters of the "Insert special character" dialog (CE-300), kept in
//! `special-chars.json` in the configuration folder. No GTK in here.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How many recently used characters are remembered.
pub const RECENT_MAX: usize = 32;

/// What a fresh installation offers as favourites: the characters books need most.
const STARTER: &[char] = &[
    '\u{a0}', '—', '–', '…', '“', '”', '‘', '’', '«', '»', '©', '§', '°', '×',
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CharPrefs {
    pub favourites: Vec<char>,
    pub recent: Vec<char>,
}

impl Default for CharPrefs {
    fn default() -> Self {
        CharPrefs {
            favourites: STARTER.to_vec(),
            recent: Vec::new(),
        }
    }
}

pub fn path() -> Option<PathBuf> {
    crate::editor_spell_dicts::config_dir().map(|d| d.join("special-chars.json"))
}

impl CharPrefs {
    pub fn is_favourite(&self, c: char) -> bool {
        self.favourites.contains(&c)
    }

    /// Adds `c` to the favourites, or removes it when it is one. Returns whether it is a favourite now.
    pub fn toggle_favourite(&mut self, c: char) -> bool {
        if let Some(i) = self.favourites.iter().position(|x| *x == c) {
            self.favourites.remove(i);
            false
        } else {
            self.favourites.push(c);
            true
        }
    }

    /// Moves a favourite by `delta` places (negative = towards the start). Returns whether it moved.
    pub fn move_favourite(&mut self, c: char, delta: i32) -> bool {
        let Some(i) = self.favourites.iter().position(|x| *x == c) else {
            return false;
        };
        let j = (i as i32 + delta).clamp(0, self.favourites.len() as i32 - 1) as usize;
        if i == j {
            return false;
        }
        let x = self.favourites.remove(i);
        self.favourites.insert(j, x);
        true
    }

    /// Notes that `c` was inserted: it becomes the first of the recent characters.
    pub fn used(&mut self, c: char) {
        self.recent.retain(|x| *x != c);
        self.recent.insert(0, c);
        self.recent.truncate(RECENT_MAX);
    }

    pub fn load_from(path: &Path) -> CharPrefs {
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

    pub fn load() -> CharPrefs {
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

    fn few() -> CharPrefs {
        CharPrefs {
            favourites: vec!['a', 'b', 'c'],
            recent: vec![],
        }
    }

    #[test]
    fn favourites_are_added_removed_and_reordered() {
        let mut p = few();
        assert!(p.toggle_favourite('d'));
        assert!(p.is_favourite('d'));
        assert!(!p.toggle_favourite('a'));
        assert_eq!(p.favourites, ['b', 'c', 'd']);
        assert!(p.move_favourite('d', -1));
        assert_eq!(p.favourites, ['b', 'd', 'c']);
        assert!(!p.move_favourite('b', -1), "already first");
        assert!(!p.move_favourite('c', 5), "already last");
        assert!(p.move_favourite('b', 9), "clamped to the end");
        assert_eq!(p.favourites, ['d', 'c', 'b']);
        assert!(!p.move_favourite('z', 1));
    }

    #[test]
    fn recent_characters_are_newest_first_without_duplicates() {
        let mut p = few();
        for c in ['x', 'y', 'x', 'z'] {
            p.used(c);
        }
        assert_eq!(p.recent, ['z', 'x', 'y']);
        for i in 0..100u32 {
            p.used(char::from_u32(0x100 + i).unwrap());
        }
        assert_eq!(p.recent.len(), RECENT_MAX);
        assert_eq!(p.recent[0], char::from_u32(0x100 + 99).unwrap());
    }

    #[test]
    fn saved_and_starter_sets() {
        let d = std::env::temp_dir().join(format!("bookx-chars-{}", uuid::Uuid::new_v4()));
        let f = d.join("sub/special-chars.json");
        let fresh = CharPrefs::load_from(&f);
        assert!(
            fresh.is_favourite('—') && fresh.is_favourite('\u{a0}'),
            "starter set"
        );
        let mut p = few();
        p.used('é');
        p.save_to(&f).unwrap();
        assert_eq!(CharPrefs::load_from(&f), p);
        let mut empty = few();
        empty.favourites.clear();
        empty.save_to(&f).unwrap();
        assert!(
            CharPrefs::load_from(&f).favourites.is_empty(),
            "an emptied list stays empty"
        );
        std::fs::write(&f, "garbage").unwrap();
        assert_eq!(CharPrefs::load_from(&f), CharPrefs::default());
        std::fs::write(&f, r#"{"recent":["é"]}"#).unwrap();
        let q = CharPrefs::load_from(&f);
        assert_eq!(q.recent, ['é']);
        let _ = std::fs::remove_dir_all(d);
    }
}
