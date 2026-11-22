//! Terms saved with your own definition, as the reader needs them: what a selection becomes as a term,
//! the sentence it was met in, and the places in a chapter where saved terms stand. No GTK in here, so
//! it is unit tested.

use std::rc::Rc;

use bookx_lookup::termmatch::TermMatcher;
use bookx_lookup::vocab::{MAX_TERM_CHARS, VocabStore};

/// What a selection is saved as: the words as they stand, white space made single, punctuation around
/// them dropped (`“Ned Land,”` is `Ned Land`; `C++` keeps its signs). `None` when nothing word-like.
pub fn term_from_selection(selection: &str) -> Option<String> {
    let joined = selection.split_whitespace().collect::<Vec<_>>().join(" ");
    let t = joined.trim_matches(|c: char| !c.is_alphanumeric() && !"+#$%&".contains(c));
    let t = if t.chars().any(char::is_alphanumeric) {
        t
    } else {
        return None;
    };
    let t: String = t.chars().take(MAX_TERM_CHARS).collect();
    let t = t.replace('\u{ad}', "");
    (!t.is_empty()).then_some(t)
}

/// The sentence around chars `start..end` of `text` (char offsets), at most about 300 characters.
pub fn sentence_around(text: &str, start: usize, end: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return String::new();
    }
    let start = start.min(chars.len());
    let end = end.clamp(start, chars.len());
    let stop = |c: char| matches!(c, '.' | '!' | '?' | '…' | '。' | '！' | '？' | '\n');
    let mut a = start;
    while a > 0 && start - a < 200 && !stop(chars[a - 1]) {
        a -= 1;
    }
    let mut b = end;
    while b < chars.len() && b - end < 200 && !stop(chars[b]) {
        b += 1;
    }
    // the closing mark of the sentence belongs to it
    if b < chars.len() && chars[b] != '\n' {
        b += 1;
        if b < chars.len() && ")]”’\"'»›".contains(chars[b]) {
            b += 1;
        }
    }
    let s: String = chars[a..b].iter().collect();
    // what closes the sentence before (`.)` `.”`) does not start this one
    let s = s.trim_start_matches(|c: char| c.is_whitespace() || ")]}”’\"'»›".contains(c));
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Most terms marked in one book (a huge imported glossary must not make a chapter slow to open).
pub const MAX_MARK_TERMS: usize = 20_000;

/// The terms in force in one book, ready to be looked for in its text.
pub struct MarkSet {
    matcher: TermMatcher,
    /// The word of each term.
    pub words: Vec<String>,
}

impl MarkSet {
    pub fn new(store: &VocabStore, book_id: &str, series: &str) -> MarkSet {
        let words: Vec<String> = store
            .own_for_book(book_id, series)
            .iter()
            .take(MAX_MARK_TERMS)
            .map(|e| e.word.clone())
            .collect();
        MarkSet {
            matcher: TermMatcher::new(&words),
            words,
        }
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.matcher.is_empty()
    }

    /// The word of the term found in `text` (the text of a mark), if any.
    pub fn word_at(&self, text: &str) -> Option<&str> {
        self.matcher
            .find(text)
            .first()
            .and_then(|m| self.words.get(m.term))
            .map(String::as_str)
    }

    /// Char ranges of the saved terms in `text`.
    pub fn marks(&self, text: &str) -> Vec<(i32, i32)> {
        self.matcher
            .find(text)
            .iter()
            .map(|m| (m.start as i32, m.end as i32))
            .collect()
    }
}

/// Keeps the [`MarkSet`] of the book being read until the vocabulary or the book changes.
#[derive(Default)]
pub struct MarkCache {
    key: Option<(u64, String, String)>,
    set: Option<Rc<MarkSet>>,
}

impl MarkCache {
    /// The set for this book; built again only when `revision` (of the vocabulary), the book or its
    /// series is not the one the cached set was built for.
    pub fn get(
        &mut self,
        revision: u64,
        store: &VocabStore,
        book_id: &str,
        series: &str,
    ) -> Rc<MarkSet> {
        let key = (revision, book_id.to_string(), series.to_string());
        if self.key.as_ref() != Some(&key) || self.set.is_none() {
            self.set = Some(Rc::new(MarkSet::new(store, book_id, series)));
            self.key = Some(key);
        }
        self.set.clone().expect("just set")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_lookup::vocab::{OwnTerm, Scope};
    use chrono::Utc;

    fn own(term: &str, scope: Scope) -> OwnTerm {
        OwnTerm {
            term: term.into(),
            definition: "d".into(),
            scope,
            ..Default::default()
        }
    }

    #[test]
    fn a_selection_becomes_a_term() {
        let t = |s| term_from_selection(s);
        assert_eq!(t("flux"), Some("flux".into()));
        assert_eq!(t("  “Ned \n Land,” "), Some("Ned Land".into()));
        assert_eq!(t("C++"), Some("C++".into()));
        assert_eq!(t("(e-mail)"), Some("e-mail".into()));
        assert_eq!(t("re\u{ad}member"), Some("remember".into()));
        assert_eq!(t("…!"), None);
        assert_eq!(t("   "), None);
        let long = "word ".repeat(100);
        assert!(t(&long).unwrap().chars().count() <= 200);
    }

    #[test]
    fn the_sentence_around_a_selection() {
        let text = "First one. The harpooner Ned Land spoke up! Then silence.\nNext paragraph.";
        let start = text.find("Ned").unwrap();
        assert_eq!(
            sentence_around(text, start, start + 8),
            "The harpooner Ned Land spoke up!"
        );
        assert_eq!(sentence_around(text, 0, 5), "First one.");
        let n = text.chars().count();
        assert_eq!(sentence_around(text, n - 5, n), "Next paragraph.");
        assert_eq!(sentence_around("", 0, 0), "");
        // chars, not bytes
        let t = "Él dijo «élan». Luego calló.";
        let s = t.chars().position(|c| c == 'é').unwrap();
        assert_eq!(sentence_around(t, s, s + 4), "Él dijo «élan».");
    }

    #[test]
    fn closing_marks_of_the_sentence_before_are_not_kept() {
        let t = "(Dinah was the cat.) “I hope they remember her saucer of milk.” Then more.";
        let s = t.find("saucer").unwrap();
        assert_eq!(
            sentence_around(t, s, s + 6),
            "“I hope they remember her saucer of milk.”"
        );
        let t = "He left (quietly.) The saucer broke.";
        let s = t.find("saucer").unwrap();
        assert_eq!(sentence_around(t, s, s + 6), "The saucer broke.");
    }

    #[test]
    fn long_text_without_a_stop_is_cut() {
        let t = "word ".repeat(500);
        assert!(sentence_around(&t, 1000, 1004).chars().count() <= 420);
    }

    #[test]
    fn the_marks_follow_the_scopes_of_the_book() {
        let mut s = VocabStore::new();
        let now = Utc::now();
        s.set_own(&own("Nautilus", Scope::All), now).unwrap();
        s.set_own(&own("Ned Land", Scope::Book("7".into())), now)
            .unwrap();
        s.set_own(&own("Aronnax", Scope::Book("8".into())), now)
            .unwrap();
        s.add("plain", None, None, now).unwrap();
        let text = "Ned Land boarded the Nautilus; Aronnax stayed. plain";
        let m = MarkSet::new(&s, "7", "");
        assert_eq!(m.marks(text), [(0, 8), (21, 29)]);
        let m8 = MarkSet::new(&s, "8", "");
        assert_eq!(m8.marks(text), [(21, 29), (31, 38)]);
        assert!(MarkSet::new(&VocabStore::new(), "1", "").is_empty());
    }

    #[test]
    fn a_huge_glossary_is_cut_to_a_sane_number_of_marks() {
        // set_own looks the term up linearly, so 20 000 calls are quadratic (minutes when instrumented):
        // build the store from serialised copies of one entry instead
        let mut one = VocabStore::new();
        one.set_own(&own("term", Scope::All), Utc::now()).unwrap();
        let mut json = serde_json::to_value(&one).unwrap();
        let entry = json["entries"][0].clone();
        json["entries"] = (0..MAX_MARK_TERMS + 50)
            .map(|i| {
                let mut e = entry.clone();
                e["word"] = format!("term{i}").into();
                e
            })
            .collect();
        let s: VocabStore = serde_json::from_value(json).unwrap();
        let m = MarkSet::new(&s, "1", "");
        assert_eq!(m.words.len(), MAX_MARK_TERMS);
    }

    #[test]
    fn the_cache_is_rebuilt_when_something_changes() {
        let mut s = VocabStore::new();
        let mut c = MarkCache::default();
        let a = c.get(1, &s, "1", "");
        assert!(a.is_empty());
        assert!(Rc::ptr_eq(&a, &c.get(1, &s, "1", "")));
        s.set_own(&own("x", Scope::All), Utc::now()).unwrap();
        // same revision: the old set (the owner bumps the revision when it saves)
        assert!(Rc::ptr_eq(&a, &c.get(1, &s, "1", "")));
        let b = c.get(2, &s, "1", "");
        assert!(!b.is_empty());
        assert!(!Rc::ptr_eq(&a, &b));
        assert!(!Rc::ptr_eq(&b, &c.get(2, &s, "2", "")));
    }
}
