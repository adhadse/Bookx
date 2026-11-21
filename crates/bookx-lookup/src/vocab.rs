//! Vocabulary builder: saved words with context, SM-2 spaced repetition,
//! flashcard sessions and Anki-compatible exports. All time is injected.

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Duration, Utc};
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs;
use std::path::Path;

/// Interval (days) from which a card counts as mastered.
pub const MASTERED_INTERVAL_DAYS: u32 = 21;
const MIN_EASE: f64 = 1.3;
/// Upper bound for an interval (100 years).
pub const MAX_INTERVAL_DAYS: u32 = 36_500;

/// Spaced-repetition state of one word (SM-2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Card {
    pub ease: f64,
    pub interval_days: u32,
    pub repetitions: u32,
    pub lapses: u32,
    pub due: DateTime<Utc>,
    pub last_reviewed: Option<DateTime<Utc>>,
}

impl Card {
    /// A new card, due immediately.
    pub fn new(now: DateTime<Utc>) -> Self {
        Self {
            ease: 2.5,
            interval_days: 0,
            repetitions: 0,
            lapses: 0,
            due: now,
            last_reviewed: None,
        }
    }

    pub fn is_due(&self, now: DateTime<Utc>) -> bool {
        self.due <= now
    }

    pub fn is_mastered(&self) -> bool {
        self.interval_days >= MASTERED_INTERVAL_DAYS
    }
}

/// Applies a review with `quality` 0..=5 (SM-2) and returns the next due time.
/// Quality below 3 is a lapse: the card restarts with a one day interval.
pub fn review(card: &mut Card, quality: u8, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
    if quality > 5 {
        bail!("quality must be 0..=5, got {quality}");
    }
    if quality < 3 {
        card.repetitions = 0;
        card.interval_days = 1;
        card.lapses += 1;
    } else {
        card.interval_days = match card.repetitions {
            0 => 1,
            1 => 6,
            _ => {
                let next = (card.interval_days as f64 * card.ease).round() as u32;
                next.max(card.interval_days.saturating_add(1))
                    .min(MAX_INTERVAL_DAYS)
            }
        };
        card.repetitions += 1;
        let q = (5 - quality) as f64;
        card.ease = (card.ease + 0.1 - q * (0.08 + q * 0.02)).max(MIN_EASE);
    }
    card.last_reviewed = Some(now);
    card.due = now + Duration::days(card.interval_days as i64);
    Ok(card.due)
}

/// A sentence in which a word was met.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WordContext {
    pub sentence: String,
    pub book_id: Option<String>,
    pub at: DateTime<Utc>,
    /// Where in the book it was met (fraction of the book), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<f64>,
}

/// Where a term with your own definition applies.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Scope {
    /// Every book.
    #[default]
    All,
    /// One book (its id).
    Book(String),
    /// The books of one series (its name).
    Series(String),
}

impl Scope {
    /// Whether a term of this scope is in force in the book `book_id` of series `series` (empty: none).
    pub fn applies(&self, book_id: &str, series: &str) -> bool {
        match self {
            Scope::All => true,
            Scope::Book(b) => b == book_id,
            Scope::Series(s) => {
                let s = s.trim();
                !s.is_empty() && s.to_lowercase() == series.trim().to_lowercase()
            }
        }
    }
}

/// What the user writes about a term of their own (see [`VocabStore::set_own`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OwnTerm {
    pub term: String,
    pub definition: String,
    pub note: String,
    pub tags: Vec<String>,
    pub language: String,
    pub scope: Scope,
    /// The sentence the term was met in, with the book and place.
    pub context: Option<String>,
    pub book_id: Option<String>,
    pub position: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VocabEntry {
    pub word: String,
    pub definition: Option<String>,
    pub contexts: Vec<WordContext>,
    pub lookups: u32,
    pub added: DateTime<Utc>,
    pub card: Card,
    /// The user's own definition (shown before any dictionary's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub language: String,
    #[serde(default)]
    pub scope: Scope,
}

impl VocabEntry {
    /// The definition to show: yours, else the one saved from a dictionary.
    pub fn shown_definition(&self) -> Option<&str> {
        self.own.as_deref().or(self.definition.as_deref())
    }
}

/// Which entries a list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    #[default]
    All,
    /// Entries with the user's own definition.
    Mine,
    /// Entries collected from dictionary look-ups only.
    Dictionary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VocabCounts {
    pub total: usize,
    pub due: usize,
    pub mastered: usize,
    pub learning: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VocabStore {
    entries: Vec<VocabEntry>,
}

fn key(w: &str) -> String {
    w.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Longest term kept, in characters.
pub const MAX_TERM_CHARS: usize = 200;
/// Longest own definition kept, in characters.
pub const MAX_OWN_CHARS: usize = 4000;

impl VocabStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[VocabEntry] {
        &self.entries
    }

    pub fn find(&self, word: &str) -> Option<&VocabEntry> {
        let k = key(word);
        self.entries.iter().find(|e| key(&e.word) == k)
    }

    fn find_mut(&mut self, word: &str) -> Option<&mut VocabEntry> {
        let k = key(word);
        self.entries.iter_mut().find(|e| key(&e.word) == k)
    }

    /// Adds a word (or bumps its lookup count and context list when known).
    /// Returns `true` when the word is new.
    pub fn add(
        &mut self,
        word: &str,
        context: Option<&str>,
        book_id: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let w = word.trim();
        if w.is_empty() {
            bail!("cannot add an empty word");
        }
        let ctx = context
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(|c| WordContext {
                sentence: c.to_string(),
                book_id: book_id.map(str::to_string),
                at: now,
                position: None,
            });
        if let Some(e) = self.find_mut(w) {
            e.lookups += 1;
            if let Some(c) = ctx {
                if !e.contexts.iter().any(|x| x.sentence == c.sentence) {
                    e.contexts.push(c);
                }
            }
            return Ok(false);
        }
        self.entries.push(VocabEntry {
            word: w.to_string(),
            definition: None,
            contexts: ctx.into_iter().collect(),
            lookups: 1,
            added: now,
            card: Card::new(now),
            own: None,
            note: String::new(),
            tags: Vec::new(),
            language: String::new(),
            scope: Scope::All,
        });
        Ok(true)
    }

    /// Saves a term with the user's own definition, new or already collected. The term does not
    /// have to be in any dictionary. Returns `true` when the term is new to the vocabulary.
    pub fn set_own(&mut self, t: &OwnTerm, now: DateTime<Utc>) -> Result<bool> {
        let term: String = t.term.split_whitespace().collect::<Vec<_>>().join(" ");
        let term: String = term.chars().take(MAX_TERM_CHARS).collect();
        if term.is_empty() {
            bail!("a term is needed");
        }
        let def: String = t.definition.trim().chars().take(MAX_OWN_CHARS).collect();
        if def.is_empty() {
            bail!("a definition is needed");
        }
        let new = self.find(&term).is_none();
        if new {
            self.entries.push(VocabEntry {
                word: term.clone(),
                definition: None,
                contexts: Vec::new(),
                lookups: 0,
                added: now,
                card: Card::new(now),
                own: None,
                note: String::new(),
                tags: Vec::new(),
                language: String::new(),
                scope: Scope::All,
            });
        }
        let ctx = t
            .context
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(|c| WordContext {
                sentence: c.chars().take(600).collect(),
                book_id: t.book_id.clone(),
                at: now,
                position: t.position,
            });
        let e = self.find_mut(&term).expect("just added");
        e.word = term;
        e.own = Some(def);
        e.note = t.note.trim().to_string();
        e.tags = t
            .tags
            .iter()
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect();
        e.language = t.language.trim().to_string();
        e.scope = t.scope.clone();
        if let Some(c) = ctx {
            if !e.contexts.iter().any(|x| x.sentence == c.sentence) {
                e.contexts.push(c);
            }
        }
        Ok(new)
    }

    /// Takes the user's own definition away. An entry that has nothing else (never looked up) goes
    /// away with it. Returns `true` when there was one.
    pub fn clear_own(&mut self, word: &str) -> bool {
        let Some(e) = self.find_mut(word) else {
            return false;
        };
        if e.own.is_none() {
            return false;
        }
        e.own = None;
        e.note.clear();
        e.tags.clear();
        e.scope = Scope::All;
        if e.lookups == 0 && e.definition.is_none() {
            self.remove(word);
        }
        true
    }

    /// Entries that have the user's own definition.
    pub fn own_entries(&self) -> Vec<&VocabEntry> {
        self.entries.iter().filter(|e| e.own.is_some()).collect()
    }

    /// The user's own entries in force in a book (`series` is empty when it has none).
    pub fn own_for_book(&self, book_id: &str, series: &str) -> Vec<&VocabEntry> {
        self.entries
            .iter()
            .filter(|e| e.own.is_some() && e.scope.applies(book_id, series))
            .collect()
    }

    /// The user's own definition of `term` when it is in force in the book.
    pub fn own_definition(&self, term: &str, book_id: &str, series: &str) -> Option<&VocabEntry> {
        self.find(term)
            .filter(|e| e.own.is_some() && e.scope.applies(book_id, series))
    }

    /// Entries for a list: filtered by where they come from and by a search over the word, the
    /// definitions, the note and the tags (case-insensitive). Sorted by word.
    pub fn list(&self, source: Source, query: &str) -> Vec<&VocabEntry> {
        let q = query.trim().to_lowercase();
        let mut v: Vec<&VocabEntry> = self
            .entries
            .iter()
            .filter(|e| match source {
                Source::All => true,
                Source::Mine => e.own.is_some(),
                Source::Dictionary => e.own.is_none(),
            })
            .filter(|e| {
                q.is_empty()
                    || e.word.to_lowercase().contains(&q)
                    || e.own
                        .iter()
                        .chain(e.definition.iter())
                        .any(|d| d.to_lowercase().contains(&q))
                    || e.note.to_lowercase().contains(&q)
                    || e.tags.iter().any(|t| t.to_lowercase().contains(&q))
            })
            .collect();
        v.sort_by_key(|e| e.word.to_lowercase());
        v
    }

    /// A store with only the entries that have the user's own definition.
    pub fn only_own(&self) -> VocabStore {
        VocabStore {
            entries: self
                .entries
                .iter()
                .filter(|e| e.own.is_some())
                .cloned()
                .collect(),
        }
    }

    pub fn set_definition(&mut self, word: &str, definition: &str) -> Result<()> {
        let e = self
            .find_mut(word)
            .ok_or_else(|| anyhow!("{word:?} is not in the vocabulary"))?;
        let d = definition.trim();
        e.definition = (!d.is_empty()).then(|| d.to_string());
        Ok(())
    }

    /// Puts back an entry that was removed (Undo): it replaces an entry of the same word.
    pub fn restore_entry(&mut self, entry: VocabEntry) {
        let k = key(&entry.word);
        self.entries.retain(|e| key(&e.word) != k);
        self.entries.push(entry);
    }

    pub fn remove(&mut self, word: &str) -> bool {
        let k = key(word);
        let before = self.entries.len();
        self.entries.retain(|e| key(&e.word) != k);
        before != self.entries.len()
    }

    /// Words met in a given book.
    pub fn by_book(&self, book_id: &str) -> Vec<&VocabEntry> {
        self.entries
            .iter()
            .filter(|e| {
                e.contexts
                    .iter()
                    .any(|c| c.book_id.as_deref() == Some(book_id))
            })
            .collect()
    }

    /// Entries due at `now`, most overdue first (ties by word).
    pub fn due(&self, now: DateTime<Utc>) -> Vec<&VocabEntry> {
        let mut v: Vec<&VocabEntry> = self.entries.iter().filter(|e| e.card.is_due(now)).collect();
        v.sort_by(|a, b| {
            a.card
                .due
                .cmp(&b.card.due)
                .then_with(|| a.word.cmp(&b.word))
        });
        v
    }

    pub fn mastered(&self) -> Vec<&VocabEntry> {
        self.entries
            .iter()
            .filter(|e| e.card.is_mastered())
            .collect()
    }

    pub fn counts(&self, now: DateTime<Utc>) -> VocabCounts {
        let mastered = self.entries.iter().filter(|e| e.card.is_mastered()).count();
        VocabCounts {
            total: self.entries.len(),
            due: self.entries.iter().filter(|e| e.card.is_due(now)).count(),
            mastered,
            learning: self.entries.len() - mastered,
        }
    }

    /// Reviews one word; returns its next due time.
    pub fn review(&mut self, word: &str, quality: u8, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
        let e = self
            .find_mut(word)
            .ok_or_else(|| anyhow!("{word:?} is not in the vocabulary"))?;
        review(&mut e.card, quality, now)
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s).context("parsing vocabulary JSON")
    }

    /// Loads a store; a missing file yields an empty store.
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(s) => Self::from_json(&s).with_context(|| format!("loading {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Saves atomically (write to a temporary file, then rename).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, self.to_json()?)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// A flashcard run over the cards due now. Failed cards (quality < 3) come back
/// at the end of the queue.
#[derive(Debug, Clone)]
pub struct FlashcardSession {
    queue: VecDeque<String>,
    total: usize,
    answered: usize,
    correct: usize,
}

impl FlashcardSession {
    /// Starts a session with at most `limit` due cards.
    pub fn new(store: &VocabStore, now: DateTime<Utc>, limit: usize) -> Self {
        let queue: VecDeque<String> = store
            .due(now)
            .into_iter()
            .take(limit)
            .map(|e| e.word.clone())
            .collect();
        Self {
            total: queue.len(),
            queue,
            answered: 0,
            correct: 0,
        }
    }

    pub fn current(&self) -> Option<&str> {
        self.queue.front().map(String::as_str)
    }

    pub fn remaining(&self) -> usize {
        self.queue.len()
    }

    pub fn is_finished(&self) -> bool {
        self.queue.is_empty()
    }

    /// `(answered, correct, total cards at start)`.
    pub fn progress(&self) -> (usize, usize, usize) {
        (self.answered, self.correct, self.total)
    }

    /// Grades the current card.
    pub fn answer(
        &mut self,
        store: &mut VocabStore,
        quality: u8,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let word = self
            .queue
            .front()
            .cloned()
            .ok_or_else(|| anyhow!("the session is finished"))?;
        store.review(&word, quality, now)?;
        self.queue.pop_front();
        self.answered += 1;
        if quality >= 3 {
            self.correct += 1;
        } else {
            self.queue.push_back(word);
        }
        Ok(())
    }
}

/// Options for [`export_anki_tsv`].
#[derive(Debug, Clone)]
pub struct AnkiOptions {
    /// Emit Anki's `#separator:tab` / `#html:true` / `#tags column:3` header lines.
    pub headers: bool,
    /// Tag added to every note.
    pub tag: String,
}

impl Default for AnkiOptions {
    fn default() -> Self {
        Self {
            headers: true,
            tag: "bookx".into(),
        }
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn tsv_field(s: &str) -> String {
    s.replace('\t', " ")
        .replace("\r\n", "<br>")
        .replace(['\n', '\r'], "<br>")
}

fn tag_safe(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_whitespace() { '_' } else { c })
        .collect()
}

fn bold_word(sentence: &str, word: &str) -> String {
    let escaped = html_escape(sentence);
    let w = html_escape(word);
    match RegexBuilder::new(&regex::escape(&w))
        .case_insensitive(true)
        .build()
    {
        Ok(re) => re
            .replace(&escaped, |c: &regex::Captures| format!("<b>{}</b>", &c[0]))
            .into_owned(),
        Err(_) => escaped,
    }
}

/// Anki-importable `front<TAB>back<TAB>tags` lines. The front is the word, the back
/// holds the definition and the first context sentence with the word in bold.
pub fn export_anki_tsv(store: &VocabStore, opts: &AnkiOptions) -> String {
    let mut out = String::new();
    if opts.headers {
        out.push_str("#separator:tab\n#html:true\n#tags column:3\n");
    }
    for e in store.entries() {
        let mut back = String::new();
        if let Some(d) = e.shown_definition() {
            back.push_str(&html_escape(d));
        }
        if !e.note.is_empty() {
            if !back.is_empty() {
                back.push_str("<br>");
            }
            back.push_str(&html_escape(&e.note));
        }
        if let Some(c) = e.contexts.first() {
            if !back.is_empty() {
                back.push_str("<br>");
            }
            back.push_str("<i>");
            back.push_str(&bold_word(&c.sentence, &e.word));
            back.push_str("</i>");
        }
        let mut tags = vec![tag_safe(&opts.tag)];
        if let Some(b) = e.contexts.first().and_then(|c| c.book_id.as_deref()) {
            tags.push(tag_safe(b));
        }
        tags.extend(e.tags.iter().map(|t| tag_safe(t)));
        tags.retain(|t| !t.is_empty());
        out.push_str(&format!(
            "{}\t{}\t{}\n",
            tsv_field(&html_escape(&e.word)),
            tsv_field(&back),
            tags.join(" ")
        ));
    }
    out
}

/// Plain text export: `word — definition` followed by indented context sentences.
pub fn export_text(store: &VocabStore) -> String {
    let mut out = String::new();
    for e in store.entries() {
        match e.shown_definition() {
            Some(d) => out.push_str(&format!("{} — {}\n", e.word, d.replace('\n', " "))),
            None => out.push_str(&format!("{}\n", e.word)),
        }
        for c in &e.contexts {
            out.push_str(&format!("  > {}\n", c.sentence.replace('\n', " ")));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TmpDir;
    use chrono::TimeZone;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap()
    }

    #[test]
    fn a_removed_entry_comes_back_with_its_definition() {
        let mut v = VocabStore::new();
        v.add("zorb", None, None, t0()).unwrap();
        v.set_definition("zorb", "a rolling ball").unwrap();
        let e = v.find("zorb").unwrap().clone();
        assert!(v.remove("zorb"));
        assert!(v.find("zorb").is_none());
        v.restore_entry(e);
        assert_eq!(
            v.find("zorb").and_then(|e| e.definition.as_deref()),
            Some("a rolling ball")
        );
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn sm2_progression_golden() {
        let mut c = Card::new(t0());
        assert!(c.is_due(t0()));
        let mut now = t0();
        let mut intervals = Vec::new();
        for _ in 0..5 {
            let due = review(&mut c, 4, now).unwrap();
            assert_eq!(due, now + Duration::days(c.interval_days as i64));
            intervals.push(c.interval_days);
            now = due;
        }
        assert_eq!(intervals, vec![1, 6, 15, 38, 95]);
        assert!((c.ease - 2.5).abs() < 1e-9, "q=4 keeps ease");
        assert_eq!(c.repetitions, 5);
        assert!(c.is_mastered());
        assert_eq!(c.last_reviewed, Some(now - Duration::days(95)));
    }

    #[test]
    fn sm2_ease_adjustments() {
        let mut c = Card::new(t0());
        review(&mut c, 5, t0()).unwrap();
        assert!((c.ease - 2.6).abs() < 1e-9);
        review(&mut c, 3, t0()).unwrap();
        assert!((c.ease - 2.46).abs() < 1e-9);
        let mut c = Card::new(t0());
        for _ in 0..40 {
            review(&mut c, 3, t0()).unwrap();
        }
        assert!(
            (c.ease - MIN_EASE).abs() < 1e-9,
            "ease never drops below 1.3"
        );
    }

    #[test]
    fn sm2_lapse_resets() {
        let mut c = Card::new(t0());
        for _ in 0..4 {
            review(&mut c, 5, t0()).unwrap();
        }
        let ease = c.ease;
        assert!(c.interval_days > 6);
        let due = review(&mut c, 2, t0()).unwrap();
        assert_eq!((c.repetitions, c.interval_days, c.lapses), (0, 1, 1));
        assert_eq!(due, t0() + Duration::days(1));
        assert_eq!(c.ease, ease, "lapse leaves ease alone");
        review(&mut c, 0, t0()).unwrap();
        assert_eq!(c.lapses, 2);
        assert!(!c.is_mastered());
    }

    #[test]
    fn sm2_interval_always_grows_on_success_and_rejects_bad_quality() {
        let mut c = Card::new(t0());
        let mut last = 0;
        for _ in 0..12 {
            review(&mut c, 3, t0()).unwrap();
            assert!(c.interval_days >= last);
            last = c.interval_days;
        }
        assert!(review(&mut c, 6, t0()).is_err());
        assert!(review(&mut c, 255, t0())
            .unwrap_err()
            .to_string()
            .contains("0..=5"));
        assert_eq!(c.lapses, 0, "rejected review changes nothing");
    }

    #[test]
    fn sm2_property_loop_all_quality_sequences() {
        // deterministic LCG
        let mut s = 12345u64;
        for _ in 0..200 {
            let mut c = Card::new(t0());
            let mut now = t0();
            for _ in 0..30 {
                s = s
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let q = ((s >> 33) % 6) as u8;
                let due = review(&mut c, q, now).unwrap();
                assert!(due > now);
                assert!(c.ease >= MIN_EASE && c.ease < 6.0);
                assert!((1..=MAX_INTERVAL_DAYS).contains(&c.interval_days));
                now = due;
            }
        }
    }

    #[test]
    fn add_words_contexts_and_duplicates() {
        let mut s = VocabStore::new();
        assert!(s.is_empty());
        assert!(s
            .add(" Ephemeral ", Some("An ephemeral joy."), Some("b1"), t0())
            .unwrap());
        assert!(!s
            .add(
                "ephemeral",
                Some("Another ephemeral thing."),
                Some("b2"),
                t0()
            )
            .unwrap());
        assert!(!s
            .add("EPHEMERAL", Some("An ephemeral joy."), Some("b1"), t0())
            .unwrap());
        assert!(!s.add("ephemeral", Some("   "), None, t0()).unwrap());
        assert_eq!(s.len(), 1);
        let e = s.find("EPHEMERAL").unwrap();
        assert_eq!(e.word, "Ephemeral");
        assert_eq!(e.lookups, 4);
        assert_eq!(e.contexts.len(), 2);
        assert_eq!(e.contexts[1].book_id.as_deref(), Some("b2"));
        assert_eq!(s.by_book("b2").len(), 1);
        assert!(s.by_book("zzz").is_empty());
        assert!(s.add("", None, None, t0()).is_err());
        assert!(s.add("   ", None, None, t0()).is_err());
        assert!(s.find("nope").is_none());
    }

    #[test]
    fn definition_and_remove() {
        let mut s = VocabStore::new();
        s.add("a", None, None, t0()).unwrap();
        s.set_definition("A", "  first letter ").unwrap();
        assert_eq!(
            s.find("a").unwrap().definition.as_deref(),
            Some("first letter")
        );
        s.set_definition("a", " ").unwrap();
        assert!(s.find("a").unwrap().definition.is_none());
        assert!(s
            .set_definition("zz", "x")
            .unwrap_err()
            .to_string()
            .contains("not in"));
        assert!(s.remove("A"));
        assert!(!s.remove("a"));
        assert!(s.is_empty());
    }

    #[test]
    fn due_list_counts_and_mastery() {
        let mut s = VocabStore::new();
        for (i, w) in ["b", "a", "c"].iter().enumerate() {
            s.add(w, None, None, t0() + Duration::minutes(i as i64))
                .unwrap();
        }
        let now = t0() + Duration::hours(1);
        let due: Vec<&str> = s.due(now).iter().map(|e| e.word.as_str()).collect();
        assert_eq!(due, vec!["b", "a", "c"], "most overdue first");
        s.review("b", 4, now).unwrap();
        assert_eq!(s.due(now).len(), 2);
        assert_eq!(s.due(now + Duration::days(1)).len(), 3);
        let c = s.counts(now);
        assert_eq!((c.total, c.due, c.mastered, c.learning), (3, 2, 0, 3));
        for _ in 0..6 {
            s.review("a", 5, now).unwrap();
        }
        assert_eq!(s.mastered().len(), 1);
        assert_eq!(s.counts(now).mastered, 1);
        assert!(s.review("zzz", 4, now).is_err());
        assert!(s.review("a", 9, now).is_err());
    }

    #[test]
    fn due_ties_sort_by_word() {
        let mut s = VocabStore::new();
        s.add("zeta", None, None, t0()).unwrap();
        s.add("alpha", None, None, t0()).unwrap();
        let due: Vec<&str> = s.due(t0()).iter().map(|e| e.word.as_str()).collect();
        assert_eq!(due, vec!["alpha", "zeta"]);
    }

    #[test]
    fn flashcard_session_flow() {
        let mut s = VocabStore::new();
        for w in ["one", "two", "three"] {
            s.add(w, None, None, t0()).unwrap();
        }
        let now = t0() + Duration::minutes(5);
        let mut f = FlashcardSession::new(&s, now, 2);
        assert_eq!(f.progress(), (0, 0, 2));
        assert_eq!(f.remaining(), 2);
        assert_eq!(f.current(), Some("one"));
        f.answer(&mut s, 1, now).unwrap(); // fail: re-queued
        assert_eq!(f.current(), Some("three"));
        assert_eq!(f.remaining(), 2);
        f.answer(&mut s, 5, now).unwrap();
        assert_eq!(f.current(), Some("one"));
        f.answer(&mut s, 3, now).unwrap();
        assert!(f.is_finished());
        assert_eq!(f.current(), None);
        assert_eq!(f.progress(), (3, 2, 2));
        assert!(f
            .answer(&mut s, 3, now)
            .unwrap_err()
            .to_string()
            .contains("finished"));
        assert_eq!(s.find("one").unwrap().card.lapses, 1);
        assert_eq!(s.find("two").unwrap().card.repetitions, 0, "outside limit");
    }

    #[test]
    fn flashcard_session_empty_and_invalid_quality() {
        let s = VocabStore::new();
        let f = FlashcardSession::new(&s, t0(), 10);
        assert!(f.is_finished());
        let mut s = VocabStore::new();
        s.add("x", None, None, t0()).unwrap();
        let mut f = FlashcardSession::new(&s, t0(), 10);
        assert!(f.answer(&mut s, 7, t0()).is_err());
        assert_eq!(f.remaining(), 1, "invalid answer keeps the card");
        let f0 = FlashcardSession::new(&s, t0(), 0);
        assert!(f0.is_finished());
    }

    #[test]
    fn json_round_trip_and_files() {
        let mut s = VocabStore::new();
        s.add("naïve", Some("A naïve “plan”."), Some("book/1"), t0())
            .unwrap();
        s.set_definition("naïve", "lacking experience").unwrap();
        s.review("naïve", 4, t0()).unwrap();
        let back = VocabStore::from_json(&s.to_json().unwrap()).unwrap();
        assert_eq!(back, s);

        let t = TmpDir::new("vocab");
        let p = t.path().join("sub/dir/vocab.json");
        s.save(&p).unwrap();
        assert!(!p.with_extension("json.tmp").exists());
        assert_eq!(VocabStore::load(&p).unwrap(), s);
        assert!(VocabStore::load(&t.path().join("none.json"))
            .unwrap()
            .is_empty());
        std::fs::write(&p, "{ not json").unwrap();
        assert!(VocabStore::load(&p).is_err());
        assert!(VocabStore::from_json("[]").is_err());
        // loading a directory is a read error, not an empty store
        assert!(VocabStore::load(t.path()).is_err());
    }

    #[test]
    fn anki_tsv_golden() {
        let mut s = VocabStore::new();
        s.add(
            "run",
            Some("They run <fast>\nand far."),
            Some("my book"),
            t0(),
        )
        .unwrap();
        s.set_definition("run", "to move\tquickly & well").unwrap();
        s.add("bare", None, None, t0()).unwrap();
        let tsv = export_anki_tsv(&s, &AnkiOptions::default());
        let expected = "#separator:tab\n#html:true\n#tags column:3\n\
run\tto move quickly &amp; well<br><i>They <b>run</b> &lt;fast&gt;<br>and far.</i>\tbookx my_book\n\
bare\t\tbookx\n";
        assert_eq!(tsv, expected);
        let plain = export_anki_tsv(
            &s,
            &AnkiOptions {
                headers: false,
                tag: "".into(),
            },
        );
        assert!(!plain.contains('#'));
        assert!(plain.ends_with("bare\t\t\n"));
        for line in plain.lines() {
            assert_eq!(line.matches('\t').count(), 2, "{line:?}");
        }
    }

    #[test]
    fn anki_tsv_empty_store_and_word_with_regex_chars() {
        let s = VocabStore::new();
        assert_eq!(
            export_anki_tsv(&s, &AnkiOptions::default()).lines().count(),
            3
        );
        let mut s = VocabStore::new();
        s.add("c++", Some("I like C++ a lot"), None, t0()).unwrap();
        let tsv = export_anki_tsv(
            &s,
            &AnkiOptions {
                headers: false,
                tag: "t".into(),
            },
        );
        assert!(tsv.contains("<b>C++</b>"), "{tsv}");
    }

    #[test]
    fn text_export_golden() {
        let mut s = VocabStore::new();
        s.add("run", Some("They run\nfast."), None, t0()).unwrap();
        s.set_definition("run", "move").unwrap();
        s.add("bare", None, None, t0()).unwrap();
        assert_eq!(
            export_text(&s),
            "run — move\n  > They run fast.\n\nbare\n\n"
        );
        assert_eq!(export_text(&VocabStore::new()), "");
    }
}

#[cfg(test)]
mod own_tests {
    use super::*;
    use chrono::TimeZone;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 20, 12, 0, 0).unwrap()
    }

    fn own(term: &str, def: &str) -> OwnTerm {
        OwnTerm {
            term: term.into(),
            definition: def.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_term_that_is_in_no_dictionary_can_be_saved_with_a_definition() {
        let mut s = VocabStore::new();
        let mut t = own("  Ned   Land ", " the harpooner of the Nautilus ");
        t.context = Some("Ned Land spoke.".into());
        t.book_id = Some("7".into());
        t.position = Some(0.25);
        t.tags = vec!["name".into(), " ".into()];
        assert!(s.set_own(&t, t0()).unwrap());
        let e = s.find("ned land").unwrap();
        assert_eq!(e.word, "Ned Land");
        assert_eq!(e.own.as_deref(), Some("the harpooner of the Nautilus"));
        assert_eq!(e.shown_definition(), Some("the harpooner of the Nautilus"));
        assert_eq!(e.lookups, 0);
        assert_eq!(e.tags, ["name"]);
        assert_eq!(e.contexts[0].position, Some(0.25));
        assert_eq!(e.contexts[0].book_id.as_deref(), Some("7"));
        // saving again edits it (no second entry, the context is not repeated)
        assert!(!s.set_own(&t, t0()).unwrap());
        assert_eq!(s.len(), 1);
        assert_eq!(s.find("Ned Land").unwrap().contexts.len(), 1);
    }

    #[test]
    fn a_term_and_a_definition_are_needed() {
        let mut s = VocabStore::new();
        assert!(s.set_own(&own("  ", "x"), t0()).is_err());
        assert!(s.set_own(&own("x", "  \n"), t0()).is_err());
        assert!(s.is_empty());
    }

    #[test]
    fn your_definition_comes_before_the_dictionarys() {
        let mut s = VocabStore::new();
        s.add("flux", Some("in flux"), None, t0()).unwrap();
        s.set_definition("flux", "a flow").unwrap();
        assert_eq!(s.find("flux").unwrap().shown_definition(), Some("a flow"));
        assert!(!s.set_own(&own("Flux", "my meaning"), t0()).unwrap());
        let e = s.find("flux").unwrap();
        assert_eq!(e.shown_definition(), Some("my meaning"));
        assert_eq!(e.definition.as_deref(), Some("a flow"));
        let tsv = export_anki_tsv(&s, &AnkiOptions::default());
        assert!(tsv.contains("my meaning"), "{tsv}");
        assert!(export_text(&s).contains("Flux — my meaning"));
    }

    #[test]
    fn removing_your_definition_keeps_a_word_that_was_looked_up() {
        let mut s = VocabStore::new();
        s.add("flux", None, None, t0()).unwrap();
        s.set_own(&own("flux", "mine"), t0()).unwrap();
        s.set_own(&own("zorb", "mine"), t0()).unwrap();
        assert!(s.clear_own("flux"));
        assert_eq!(s.find("flux").unwrap().own, None);
        assert!(s.clear_own("zorb"));
        assert!(s.find("zorb").is_none(), "an entry with nothing else goes");
        assert!(!s.clear_own("flux"));
        assert!(!s.clear_own("nothing"));
    }

    #[test]
    fn scopes_decide_where_a_term_is_in_force() {
        let mut s = VocabStore::new();
        let mut a = own("a", "all");
        s.set_own(&a, t0()).unwrap();
        a.term = "b".into();
        a.scope = Scope::Book("7".into());
        s.set_own(&a, t0()).unwrap();
        a.term = "c".into();
        a.scope = Scope::Series("The Expanse".into());
        s.set_own(&a, t0()).unwrap();
        let words = |book: &str, series: &str| -> Vec<String> {
            s.own_for_book(book, series)
                .iter()
                .map(|e| e.word.clone())
                .collect()
        };
        assert_eq!(words("7", ""), ["a", "b"]);
        assert_eq!(words("8", "the expanse"), ["a", "c"]);
        assert_eq!(words("7", "The Expanse "), ["a", "b", "c"]);
        assert_eq!(words("9", ""), ["a"]);
        assert!(s.own_definition("b", "8", "").is_none());
        assert!(s.own_definition("B", "7", "").is_some());
        assert!(!Scope::Series("".into()).applies("1", ""));
    }

    #[test]
    fn lists_filter_by_source_and_search() {
        let mut s = VocabStore::new();
        s.add("zebra", None, None, t0()).unwrap();
        s.set_definition("zebra", "striped horse").unwrap();
        let mut t = own("Nautilus", "the submarine");
        t.note = "ch 4".into();
        t.tags = vec!["ship".into()];
        s.set_own(&t, t0()).unwrap();
        let names = |v: Vec<&VocabEntry>| v.iter().map(|e| e.word.clone()).collect::<Vec<_>>();
        assert_eq!(names(s.list(Source::All, "")), ["Nautilus", "zebra"]);
        assert_eq!(names(s.list(Source::Mine, "")), ["Nautilus"]);
        assert_eq!(names(s.list(Source::Dictionary, "")), ["zebra"]);
        assert_eq!(names(s.list(Source::All, "SUBMA")), ["Nautilus"]);
        assert_eq!(names(s.list(Source::All, "ship")), ["Nautilus"]);
        assert_eq!(names(s.list(Source::All, "ch 4")), ["Nautilus"]);
        assert_eq!(names(s.list(Source::All, "horse")), ["zebra"]);
        assert!(s.list(Source::Mine, "horse").is_empty());
        assert_eq!(s.only_own().len(), 1);
    }

    #[test]
    fn a_vocabulary_file_from_before_own_definitions_still_loads() {
        let old = r#"{"entries":[{"word":"ephemeral","definition":"short","contexts":[{"sentence":"s","book_id":"1","at":"2026-01-01T00:00:00Z"}],"lookups":2,"added":"2026-01-01T00:00:00Z","card":{"ease":2.5,"interval_days":1,"repetitions":1,"lapses":0,"due":"2026-01-02T00:00:00Z","last_reviewed":null}}]}"#;
        let s = VocabStore::from_json(old).unwrap();
        let e = s.find("ephemeral").unwrap();
        assert_eq!(e.own, None);
        assert_eq!(e.scope, Scope::All);
        assert_eq!(e.contexts[0].position, None);
        // and a new file does not carry empty fields for old entries
        let json = s.to_json().unwrap();
        assert!(!json.contains("\"own\""), "{json}");
        // own entries survive a round trip
        let mut s = s;
        let mut t = own("Zorb", "def");
        t.scope = Scope::Series("S".into());
        t.position = Some(0.5);
        t.context = Some("c".into());
        s.set_own(&t, t0()).unwrap();
        let back = VocabStore::from_json(&s.to_json().unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
