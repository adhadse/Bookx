//! Vocabulary builder glue (roadmap R9/R39): words looked up in a dictionary are collected and
//! reviewed later with spaced repetition. No GTK in here, so it is unit tested.

use std::path::PathBuf;

use anyhow::Result;
use bookx_lookup::vocab::{OwnTerm, VocabCounts, VocabStore};
use chrono::{DateTime, Utc};

/// Longest definition kept with a word, in characters.
const MAX_DEFINITION: usize = 300;

pub struct VocabHub {
    pub store: VocabStore,
    path: Option<PathBuf>,
    /// Counts every change of the vocabulary, so the page can tell when its marks are out of date.
    rev: std::cell::Cell<u64>,
    marks: std::cell::RefCell<crate::vocab_marks::MarkCache>,
}

/// Whether a selection is a word or a short phrase worth a visible "Add to Vocabulary" button
/// (up to three words, no sentence ending inside it).
pub fn is_short_phrase(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() || t.chars().count() > 40 || t.split_whitespace().count() > 3 {
        return false;
    }
    !t.chars()
        .rev()
        .skip(1)
        .any(|c| matches!(c, '.' | '!' | '?' | ';' | '\n'))
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
        VocabHub {
            store,
            path,
            rev: std::cell::Cell::new(1),
            marks: Default::default(),
        }
    }

    /// Changes whenever the vocabulary is saved.
    pub fn revision(&self) -> u64 {
        self.rev.get()
    }

    /// The saved terms of this book found in `text`, as char ranges.
    pub fn marks_for(&self, book_id: u64, series: &str, text: &str) -> Vec<(i32, i32)> {
        let set =
            self.marks
                .borrow_mut()
                .get(self.rev.get(), &self.store, &book_id.to_string(), series);
        set.marks(text)
    }

    /// Whether any saved term is in force in this book (cheap: nothing is marked when none is).
    pub fn has_terms_for(&self, book_id: u64, series: &str) -> bool {
        !self
            .store
            .own_for_book(&book_id.to_string(), series)
            .is_empty()
    }

    /// The word of the saved term whose text is `text` (what a click on a mark found).
    pub fn word_for_mark(&self, book_id: u64, series: &str, text: &str) -> Option<String> {
        let set =
            self.marks
                .borrow_mut()
                .get(self.rev.get(), &self.store, &book_id.to_string(), series);
        set.word_at(text).map(str::to_string)
    }

    /// Saves a term with the user's own definition. `true` when the term is new.
    pub fn save_own(&mut self, t: &OwnTerm, now: DateTime<Utc>) -> Result<bool> {
        let new = self.store.set_own(t, now)?;
        self.save();
        Ok(new)
    }

    /// Takes the user's own definition of a term away.
    pub fn remove_own(&mut self, term: &str) -> bool {
        let r = self.store.clear_own(term);
        self.save();
        r
    }

    /// Removes a word from the vocabulary altogether.
    pub fn remove_word(&mut self, term: &str) -> bool {
        let r = self.store.remove(term);
        self.save();
        r
    }

    /// Brings back an entry that was removed (Undo of a removal).
    pub fn restore_entry(&mut self, entry: bookx_lookup::vocab::VocabEntry) {
        self.store.restore_entry(entry);
        self.save();
    }

    pub fn save(&self) {
        self.rev.set(self.rev.get() + 1);
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
    #[test]
    fn short_phrases_get_the_visible_button() {
        use super::is_short_phrase;
        assert!(is_short_phrase("serendipity"));
        assert!(is_short_phrase("  rolling ball "));
        assert!(is_short_phrase("à la carte"));
        assert!(is_short_phrase("end."));
        assert!(!is_short_phrase(""));
        assert!(!is_short_phrase("one two three four"));
        assert!(!is_short_phrase("It ended. Then"));
        assert!(!is_short_phrase(&"x".repeat(41)));
    }

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

#[cfg(test)]
mod own_tests {
    use super::*;
    use bookx_lookup::vocab::Scope;

    fn own(term: &str) -> OwnTerm {
        OwnTerm {
            term: term.into(),
            definition: "a made-up thing".into(),
            ..Default::default()
        }
    }

    #[test]
    fn saving_a_term_persists_and_changes_the_revision() {
        let d = std::env::temp_dir().join(format!("bookx-vocab-own-{}", uuid::Uuid::new_v4()));
        let path = d.join("vocabulary.json");
        let mut h = VocabHub::load(Some(path.clone()));
        let r0 = h.revision();
        assert!(h.save_own(&own("Zorb"), Utc::now()).unwrap());
        assert!(h.revision() > r0);
        let again = VocabHub::load(Some(path));
        assert_eq!(
            again.store.find("zorb").unwrap().own.as_deref(),
            Some("a made-up thing")
        );
        assert!(h.remove_own("Zorb"));
        assert!(h.store.find("zorb").is_none());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn marks_come_from_the_terms_of_the_book_and_follow_edits() {
        let mut h = VocabHub::load(None);
        h.save_own(&own("Zorb"), Utc::now()).unwrap();
        let mut b = own("Quux");
        b.scope = Scope::Book("2".into());
        h.save_own(&b, Utc::now()).unwrap();
        let text = "a zorb and a Quux";
        assert_eq!(h.marks_for(1, "", text), [(2, 6)]);
        assert_eq!(h.marks_for(2, "", text), [(2, 6), (13, 17)]);
        h.remove_own("zorb");
        assert_eq!(h.marks_for(2, "", text), [(13, 17)]);
    }
}
