//! Vocabulary builder glue (roadmap R9/R39): words looked up in a dictionary are collected and
//! reviewed later with spaced repetition. No GTK in here, so it is unit tested.

use std::path::PathBuf;

use anyhow::Result;
use bookx_lookup::vocab::{VocabCounts, VocabStore};
use chrono::{DateTime, Utc};

/// Longest definition kept with a word, in characters.
const MAX_DEFINITION: usize = 300;

pub struct VocabHub {
    pub store: VocabStore,
    path: Option<PathBuf>,
}

impl VocabHub {
    /// Loads the store; a corrupt file is kept aside and an empty vocabulary is started.
    pub fn load(path: Option<PathBuf>) -> VocabHub {
        let store = match path.as_deref().map(VocabStore::load) {
            Some(Ok(s)) => s,
            Some(Err(_)) => {
                if let Some(p) = &path {
                    let _ = std::fs::rename(p, p.with_extension("json.corrupt"));
                }
                VocabStore::default()
            }
            None => VocabStore::default(),
        };
        VocabHub { store, path }
    }

    pub fn save(&self) {
        if let Some(p) = &self.path {
            let _ = self.store.save(p);
        }
    }

    /// Remembers a word that was just looked up. `selection` is what the reader had selected: it
    /// is kept as the sentence the word was met in when it is longer than the word itself.
    /// Returns `true` when the word is new.
    pub fn record_lookup(
        &mut self,
        word: &str,
        selection: &str,
        book_id: u64,
        definition: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let context = {
            let s = selection.trim();
            (s.chars().count() > word.trim().chars().count() + 1).then_some(s)
        };
        let new = self
            .store
            .add(word, context, Some(&book_id.to_string()), now)?;
        if let Some(def) = definition.map(str::trim).filter(|d| !d.is_empty()) {
            let has = self
                .store
                .find(word)
                .is_some_and(|e| e.definition.is_some());
            if !has {
                let cut: String = def.chars().take(MAX_DEFINITION).collect();
                self.store.set_definition(word, &cut)?;
            }
        }
        self.save();
        Ok(new)
    }

    pub fn counts(&self, now: DateTime<Utc>) -> VocabCounts {
        self.store.counts(now)
    }

    /// Grades the first due card. Returns its word.
    pub fn grade_first_due(&mut self, quality: u8, now: DateTime<Utc>) -> Result<Option<String>> {
        let word = self.store.due(now).first().map(|e| e.word.clone());
        if let Some(w) = &word {
            self.store.review(w, quality, now)?;
            self.save();
        }
        Ok(word)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn t(d: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 12, 9, 0, 0).unwrap() + Duration::days(d)
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "bookx-vocab-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d.join("vocab.json")
    }

    #[test]
    fn a_lookup_adds_the_word_with_its_definition_and_context() {
        let mut h = VocabHub::load(None);
        let new = h
            .record_lookup(
                "ephemeral",
                "a fleeting, ephemeral joy",
                7,
                Some("lasting a short time"),
                t(0),
            )
            .unwrap();
        assert!(new);
        let e = h.store.find("Ephemeral").unwrap();
        assert_eq!(e.definition.as_deref(), Some("lasting a short time"));
        assert_eq!(e.contexts.len(), 1);
        assert_eq!(e.contexts[0].sentence, "a fleeting, ephemeral joy");
        assert_eq!(e.contexts[0].book_id.as_deref(), Some("7"));
    }

    #[test]
    fn looking_up_a_known_word_counts_but_does_not_duplicate() {
        let mut h = VocabHub::load(None);
        h.record_lookup("word", "word", 1, Some("first"), t(0))
            .unwrap();
        let new = h
            .record_lookup("Word", "another word here", 1, Some("second"), t(1))
            .unwrap();
        assert!(!new);
        assert_eq!(h.store.len(), 1);
        let e = h.store.find("word").unwrap();
        assert_eq!(e.lookups, 2);
        // the first definition is kept
        assert_eq!(e.definition.as_deref(), Some("first"));
        assert_eq!(
            e.contexts.len(),
            1,
            "a selection equal to the word is no context"
        );
    }

    #[test]
    fn long_definitions_are_cut_and_empty_ones_ignored() {
        let mut h = VocabHub::load(None);
        h.record_lookup("big", "big", 1, Some(&"x".repeat(1000)), t(0))
            .unwrap();
        assert_eq!(
            h.store
                .find("big")
                .unwrap()
                .definition
                .as_ref()
                .unwrap()
                .chars()
                .count(),
            MAX_DEFINITION
        );
        h.record_lookup("none", "none", 1, Some("   "), t(0))
            .unwrap();
        assert!(h.store.find("none").unwrap().definition.is_none());
        h.record_lookup("nodef", "nodef", 1, None, t(0)).unwrap();
        assert!(h.store.find("nodef").unwrap().definition.is_none());
    }

    #[test]
    fn an_empty_word_is_an_error() {
        let mut h = VocabHub::load(None);
        assert!(h.record_lookup("  ", "x", 1, None, t(0)).is_err());
        assert!(h.store.is_empty());
    }

    #[test]
    fn new_words_are_due_and_grading_schedules_them_later() {
        let mut h = VocabHub::load(None);
        h.record_lookup("alpha", "alpha", 1, None, t(0)).unwrap();
        h.record_lookup("beta", "beta", 1, None, t(0)).unwrap();
        assert_eq!(h.counts(t(0)).due, 2);
        assert_eq!(h.counts(t(0)).total, 2);
        let w = h.grade_first_due(4, t(0)).unwrap().unwrap();
        assert_eq!(w, "alpha");
        assert_eq!(h.counts(t(0)).due, 1);
        // a day later alpha comes back
        assert_eq!(h.counts(t(1)).due, 2);
        assert_eq!(h.grade_first_due(4, t(0)).unwrap().as_deref(), Some("beta"));
        assert!(h.grade_first_due(4, t(0)).unwrap().is_none());
    }

    #[test]
    fn the_vocabulary_survives_a_restart() {
        let path = tmp("persist");
        let mut h = VocabHub::load(Some(path.clone()));
        h.record_lookup("keep", "keep this", 3, Some("hold"), t(0))
            .unwrap();
        let again = VocabHub::load(Some(path.clone()));
        assert_eq!(again.store.len(), 1);
        assert_eq!(
            again.store.find("keep").unwrap().definition.as_deref(),
            Some("hold")
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_corrupt_file_is_set_aside() {
        let path = tmp("corrupt");
        std::fs::write(&path, "{ nope").unwrap();
        let h = VocabHub::load(Some(path.clone()));
        assert!(h.store.is_empty());
        assert!(path.with_extension("json.corrupt").exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
