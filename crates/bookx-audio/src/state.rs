//! What Bookx remembers of one audiobook between sessions: where you stopped, the speed, the
//! bookmarks with their notes. One small JSON file per book (written atomically).

use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Bookmarks per book (a damaged file cannot make the list grow without end).
pub const MAX_BOOKMARKS: usize = 1000;
/// Longest note kept.
pub const MAX_NOTE: usize = 2000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    pub id: String,
    pub position_ms: u64,
    #[serde(default)]
    pub note: String,
    /// Unix seconds.
    #[serde(default)]
    pub created: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub position_ms: u64,
    pub speed: f32,
    #[serde(default)]
    pub bookmarks: Vec<Bookmark>,
    /// Unix seconds of the last change.
    #[serde(default)]
    pub updated: i64,
    /// The book was played to its end at least once.
    #[serde(default)]
    pub finished: bool,
}

impl Default for State {
    fn default() -> Self {
        State {
            position_ms: 0,
            speed: 1.0,
            bookmarks: Vec::new(),
            updated: 0,
            finished: false,
        }
    }
}

impl State {
    /// The saved state, or the default when there is none or it is damaged. Values are clamped.
    pub fn load(path: &Path) -> State {
        let md_ok = std::fs::metadata(path)
            .map(|m| m.len() < 8 * 1024 * 1024)
            .unwrap_or(false);
        let mut s: State = if md_ok {
            std::fs::read(path)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default()
        } else {
            State::default()
        };
        s.speed = crate::stretch::clamp_speed(s.speed);
        s.bookmarks.truncate(MAX_BOOKMARKS);
        for b in &mut s.bookmarks {
            if b.note.len() > MAX_NOTE {
                b.note = b.note.chars().take(MAX_NOTE).collect();
            }
        }
        s
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Adds a bookmark at `position_ms`; returns its id. `None` when the list is full.
    pub fn add_bookmark(&mut self, position_ms: u64, note: &str, now: i64) -> Option<String> {
        if self.bookmarks.len() >= MAX_BOOKMARKS {
            return None;
        }
        let mut note: String = note.trim().chars().take(MAX_NOTE).collect();
        note.shrink_to_fit();
        let mut n = self.bookmarks.len() + 1;
        let id = loop {
            let id = format!("b{n}");
            if !self.bookmarks.iter().any(|b| b.id == id) {
                break id;
            }
            n += 1;
        };
        self.bookmarks.push(Bookmark {
            id: id.clone(),
            position_ms,
            note,
            created: now,
        });
        self.bookmarks.sort_by_key(|b| b.position_ms);
        Some(id)
    }

    pub fn remove_bookmark(&mut self, id: &str) -> bool {
        let n = self.bookmarks.len();
        self.bookmarks.retain(|b| b.id != id);
        self.bookmarks.len() != n
    }

    pub fn set_note(&mut self, id: &str, note: &str) -> bool {
        match self.bookmarks.iter_mut().find(|b| b.id == id) {
            Some(b) => {
                b.note = note.trim().chars().take(MAX_NOTE).collect();
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-audio-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d.join("state.json")
    }

    #[test]
    fn state_round_trips() {
        let p = tmp();
        let mut s = State {
            position_ms: 123_456,
            speed: 1.5,
            ..Default::default()
        };
        s.add_bookmark(9000, "  the heist  ", 100).unwrap();
        s.add_bookmark(1000, "", 101).unwrap();
        s.save(&p).unwrap();
        let back = State::load(&p);
        assert_eq!(back, s);
        assert_eq!(back.bookmarks[0].position_ms, 1000, "sorted by position");
        assert_eq!(back.bookmarks[1].note, "the heist");
    }

    #[test]
    fn missing_or_damaged_files_give_the_default() {
        let p = tmp();
        assert_eq!(State::load(&p), State::default());
        std::fs::write(&p, b"{not json").unwrap();
        assert_eq!(State::load(&p), State::default());
        std::fs::write(&p, br#"{"position_ms": 5, "speed": 99.0}"#).unwrap();
        let s = State::load(&p);
        assert_eq!(
            (s.position_ms, s.speed),
            (5, 3.0),
            "an old or hand-made file loads, speed is clamped"
        );
    }

    #[test]
    fn bookmark_ids_are_unique_and_removal_works() {
        let mut s = State::default();
        let a = s.add_bookmark(10, "a", 0).unwrap();
        let b = s.add_bookmark(5, "b", 0).unwrap();
        assert_ne!(a, b);
        assert!(s.set_note(&a, "changed"));
        assert!(!s.set_note("nope", "x"));
        assert!(s.remove_bookmark(&a));
        assert!(!s.remove_bookmark(&a));
        let c = s.add_bookmark(1, "c", 0).unwrap();
        assert_ne!(c, b, "an id is not reused while it exists");
        assert_eq!(s.bookmarks.len(), 2);
    }

    #[test]
    fn the_list_and_notes_are_bounded() {
        let mut s = State::default();
        for i in 0..MAX_BOOKMARKS {
            assert!(s.add_bookmark(i as u64, "x", 0).is_some());
        }
        assert!(s.add_bookmark(1, "overflow", 0).is_none());
        let mut s = State::default();
        let id = s.add_bookmark(1, &"é".repeat(MAX_NOTE * 2), 0).unwrap();
        assert_eq!(
            s.bookmarks
                .iter()
                .find(|b| b.id == id)
                .unwrap()
                .note
                .chars()
                .count(),
            MAX_NOTE
        );
        // a hostile file with a huge list and a huge note
        let p = tmp();
        let many: Vec<String> = (0..1500)
            .map(|i| {
                format!(
                    r#"{{"id":"x{i}","position_ms":{i},"note":"{}"}}"#,
                    "é".repeat(2000)
                )
            })
            .collect();
        std::fs::write(
            &p,
            format!(
                r#"{{"position_ms":1,"speed":1.0,"bookmarks":[{}]}}"#,
                many.join(",")
            ),
        )
        .unwrap();
        let s = State::load(&p);
        assert_eq!(s.bookmarks.len(), MAX_BOOKMARKS);
        assert!(s
            .bookmarks
            .iter()
            .all(|b| b.note.chars().count() <= MAX_NOTE));
    }
}
