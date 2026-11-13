//! Live spell checking of the open file in the book editor: the Hunspell engine of
//! `bookx_editing::spell` with the book's dictionary, a personal word list and a per-session ignore
//! list. The editor underlines the misspelled words of the visible part. No GTK in here.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use bookx_editing::spell::{check_source, SpellChecker, SpellReport};
use bookx_editing::spell_session::has_no_spell_marker;
use bookx_editing::spell_user::UserDicts;

/// A misspelled word; offsets are bytes into the checked text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spot {
    pub start: usize,
    pub end: usize,
    pub word: String,
    pub lang: String,
}

pub struct SpellState {
    checker: SpellChecker,
    lang: String,
    ready: bool,
    note: Option<String>,
    user_file: Option<PathBuf>,
    /// The named user dictionaries, when the personal list lives in the configuration folder.
    dicts: Option<UserDicts>,
    /// Dictionary files already loaded (a tag without its own dictionary reuses the one that serves it).
    loaded: HashSet<PathBuf>,
    missing: Vec<(String, String)>,
    cache: RefCell<HashMap<(String, String), Option<bool>>>,
}

/// `<config>/Bookx/spelling-words.txt`, the personal dictionary.
pub fn user_words_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.config_dir().join("spelling-words.txt"))
}

impl SpellState {
    /// Loads the dictionary for `lang` (empty means English) from `dirs`.
    pub fn new(lang: &str, dirs: &[PathBuf], user_file: Option<PathBuf>) -> SpellState {
        let lang = if lang.trim().is_empty() { "en" } else { lang }.to_string();
        let mut checker = SpellChecker::new();
        let mut note = None;
        let mut ready = false;
        let mut loaded = HashSet::new();
        match crate::editor_tools::find_dictionary(&lang, dirs) {
            Some((aff, dic)) => match checker.add_dictionary_files(&lang, &aff, &dic) {
                Ok(()) => {
                    ready = true;
                    loaded.insert(aff);
                }
                Err(e) => note = Some(format!("Could not load the dictionary: {e:#}")),
            },
            None => note = Some(format!("No spelling dictionary for “{lang}” is installed")),
        }
        let dicts = user_file
            .as_ref()
            .filter(|f| f.file_name().is_some_and(|n| n == "spelling-words.txt"))
            .and_then(|f| f.parent())
            .map(UserDicts::new);
        match &dicts {
            Some(d) => {
                for w in d.enabled_words() {
                    checker.add_user_word(&w);
                }
            }
            None => {
                if let Some(Ok(t)) = user_file.as_ref().map(std::fs::read_to_string) {
                    checker.load_user_words(&t);
                }
            }
        }
        SpellState {
            checker,
            lang,
            ready,
            note,
            user_file,
            dicts,
            loaded,
            missing: Vec::new(),
            cache: RefCell::new(HashMap::new()),
        }
    }

    /// Loads the dictionaries of the other languages the book uses (CE-271), so `lang="en-GB"` or `lang="de"`
    /// text is checked with its own dictionary. Tags that need none that is not loaded yet are skipped.
    pub fn load_languages(&mut self, langs: &BTreeSet<String>, dirs: &[PathBuf]) {
        let (loaded, missing) = (&mut self.loaded, &mut self.missing);
        for l in langs {
            if self.checker.lookup_key(l).is_some_and(|k| k == *l) {
                continue;
            }
            match crate::editor_tools::find_dictionary(
                &bookx_editing::spell::canonical_tag(l),
                dirs,
            ) {
                Some((aff, dic)) => {
                    if loaded.contains(&aff) && self.checker.has_language(l) {
                        continue;
                    }
                    match self.checker.add_dictionary_files(l, &aff, &dic) {
                        Ok(()) => {
                            loaded.insert(aff);
                            self.ready = true;
                        }
                        Err(e) => missing.push((l.clone(), format!("{e:#}"))),
                    }
                }
                None => missing.push((l.clone(), "no dictionary installed".to_string())),
            }
        }
        self.cache.borrow_mut().clear();
    }

    /// Languages the book uses that have no usable dictionary, with the reason.
    pub fn missing_languages(&self) -> &[(String, String)] {
        &self.missing
    }

    /// A cheap copy of the checker (dictionaries are shared) to run a scan on another thread.
    pub fn snapshot(&self) -> SpellChecker {
        self.checker.clone()
    }

    /// The user dictionaries behind "Add to Dictionary".
    pub fn user_dicts(&self) -> Option<&UserDicts> {
        self.dicts.as_ref()
    }

    /// Reads the user dictionaries again (after one was created, enabled, disabled or edited).
    pub fn reload_user_words(&mut self) {
        if let Some(d) = &self.dicts {
            let words = d.enabled_words();
            let mut fresh = self.checker.clone();
            fresh.load_user_words("");
            for w in words {
                fresh.add_user_word(&w);
            }
            self.checker = fresh;
            self.cache.borrow_mut().clear();
        }
    }

    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// Why nothing is underlined, when that is so.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// The misspelled words of an XHTML document, in order.
    pub fn check(&self, text: &str) -> Vec<Spot> {
        if !self.ready || has_no_spell_marker(text) {
            return Vec::new();
        }
        let mut report = SpellReport::default();
        check_source(
            &self.checker,
            "",
            text,
            &self.lang,
            &mut report,
            &mut self.cache.borrow_mut(),
        );
        report
            .misspellings
            .into_iter()
            .map(|m| Spot {
                start: m.start,
                end: m.end,
                word: m.word,
                lang: m.lang,
            })
            .collect()
    }

    /// Is `word` in the dictionary? Without a dictionary every word counts as known.
    pub fn recognized(&self, word: &str) -> bool {
        !self.ready || self.checker.check_word(&self.lang, word).unwrap_or(true)
    }

    pub fn suggest(&self, spot: &Spot) -> Vec<String> {
        self.checker.suggest(&spot.lang, &spot.word)
    }

    /// Ignores a word until the editor is closed.
    pub fn ignore(&mut self, word: &str) {
        self.checker.ignore_word(word);
        self.cache.borrow_mut().clear();
    }

    /// Ignores several words (the ones remembered for the book).
    pub fn ignore_all<'a>(&mut self, words: impl IntoIterator<Item = &'a String>) {
        for w in words {
            self.checker.ignore_word(w);
        }
        self.cache.borrow_mut().clear();
    }

    /// Adds a word to the named user dictionary and saves it.
    pub fn add_to_named(&mut self, dict: &str, word: &str) -> anyhow::Result<()> {
        let Some(d) = &self.dicts else {
            return Ok(self.add_to_dictionary(word)?);
        };
        d.add_word(dict, word)?;
        self.checker.add_user_word(word);
        self.cache.borrow_mut().clear();
        Ok(())
    }

    /// Adds a word to the personal dictionary and saves it.
    pub fn add_to_dictionary(&mut self, word: &str) -> std::io::Result<()> {
        if self.dicts.is_some() {
            return self
                .add_to_named(bookx_editing::spell_user::PERSONAL, word)
                .map_err(std::io::Error::other);
        }
        self.checker.add_user_word(word);
        self.cache.borrow_mut().clear();
        match &self.user_file {
            Some(p) => save_words(p, &self.checker.user_words_text()),
            None => Ok(()),
        }
    }
}

fn save_words(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, text)
}

/// The spots that overlap `start..end`.
pub fn in_range(spots: &[Spot], start: usize, end: usize) -> &[Spot] {
    let a = spots.partition_point(|s| s.end <= start);
    let b = spots.partition_point(|s| s.start < end);
    &spots[a..b.max(a)]
}

/// The spot under byte offset `at` (an offset just after the word counts).
pub fn spot_at(spots: &[Spot], at: usize) -> Option<&Spot> {
    spots.iter().find(|s| s.start <= at && at <= s.end)
}

/// F8 / Shift+F8: the index of the next (or previous) spot relative to the caret at byte `caret`, wrapping around.
/// The spot the caret is in or at the start of counts as "here" and is skipped when moving forward.
pub fn pick_spot(spots: &[Spot], caret: usize, forward: bool) -> Option<usize> {
    if spots.is_empty() {
        return None;
    }
    if forward {
        spots.iter().position(|s| s.start > caret).or(Some(0))
    } else {
        spots
            .iter()
            .rposition(|s| s.start < caret)
            .or(Some(spots.len() - 1))
    }
}

/// The spots overlapping the byte range `from..to`, as `(start, end)` character offsets, for the
/// underline tags. Walks the text once instead of counting from the start for each word.
pub fn char_spans(text: &str, spots: &[Spot], from: usize, to: usize) -> Vec<(usize, usize)> {
    let hits = in_range(spots, from, to);
    let Some(first) = hits.first() else {
        return Vec::new();
    };
    let mut byte = first.start.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    let mut chars = text[..byte].chars().count();
    let mut out = Vec::with_capacity(hits.len());
    for s in hits {
        let (a, b) = (s.start.min(text.len()), s.end.min(text.len()));
        if a < byte || !text.is_char_boundary(a) || !text.is_char_boundary(b) {
            continue;
        }
        chars += text[byte..a].chars().count();
        let len = text[a..b].chars().count();
        out.push((chars, chars + len));
        chars += len;
        byte = b;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-espell-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("en_US.aff"), "SET UTF-8\n").unwrap();
        std::fs::write(d.join("en_US.dic"), "5\nthe\nquick\nbrown\nfox\nwords\n").unwrap();
        d
    }

    const DOC: &str =
        "<html><body><p>The quikc brown fox</p><p>wrods and <code>zzqx</code></p></body></html>";

    #[test]
    fn misspelled_words_are_found_with_offsets_and_suggestions() {
        let d = dir();
        let s = SpellState::new("en", std::slice::from_ref(&d), None);
        assert!(s.is_ready());
        let spots = s.check(DOC);
        let words: Vec<&str> = spots.iter().map(|s| s.word.as_str()).collect();
        assert_eq!(words, ["quikc", "wrods", "and"]);
        assert_eq!(&DOC[spots[0].start..spots[0].end], "quikc");
        assert!(!words.contains(&"zzqx"), "code is not checked");
        assert!(s.suggest(&spots[1]).contains(&"words".to_string()));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn ignore_and_personal_dictionary() {
        let d = dir();
        let user = d.join("mine.txt");
        let mut s = SpellState::new("en", std::slice::from_ref(&d), Some(user.clone()));
        s.ignore("quikc");
        let words: Vec<String> = s.check(DOC).into_iter().map(|s| s.word).collect();
        assert!(!words.contains(&"quikc".to_string()));
        s.add_to_dictionary("wrods").unwrap();
        assert!(std::fs::read_to_string(&user).unwrap().contains("wrods"));
        let again = SpellState::new("en", std::slice::from_ref(&d), Some(user));
        let words: Vec<String> = again.check(DOC).into_iter().map(|s| s.word).collect();
        assert!(
            !words.contains(&"wrods".to_string()),
            "saved words load again"
        );
        assert!(
            words.contains(&"quikc".to_string()),
            "ignored words do not persist"
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn no_dictionary_means_no_underlines_and_a_reason() {
        let none = SpellState::new("xx", &[std::env::temp_dir().join("bookx-nodict")], None);
        assert!(!none.is_ready());
        assert!(none.note().unwrap().contains("No spelling dictionary"));
        assert!(none.check(DOC).is_empty());
    }

    #[test]
    fn character_offsets_follow_multibyte_text() {
        let text = "é <p>héllo wrld</p> ünï";
        let b0 = text.find("wrld").unwrap();
        let b1 = text.find("ünï").unwrap();
        let sp = |s: usize, e: usize| Spot {
            start: s,
            end: e,
            word: "x".into(),
            lang: "en".into(),
        };
        let spots = vec![sp(b0, b0 + 4), sp(b1, b1 + "ünï".len())];
        let all = char_spans(text, &spots, 0, text.len());
        let chars: Vec<char> = text.chars().collect();
        for (a, b) in &all {
            let w: String = chars[*a..*b].iter().collect();
            assert!(w == "wrld" || w == "ünï", "{w}");
        }
        assert_eq!(all.len(), 2);
        let only_second = char_spans(text, &spots, b1, text.len());
        assert_eq!(only_second.len(), 1);
        assert_eq!(only_second[0], all[1]);
        assert!(char_spans(text, &spots, 0, 2).is_empty());
    }

    #[test]
    fn range_and_position_lookups() {
        let sp = |s, e| Spot {
            start: s,
            end: e,
            word: "x".into(),
            lang: "en".into(),
        };
        let v = vec![sp(0, 3), sp(10, 15), sp(20, 22)];
        assert_eq!(in_range(&v, 4, 12).len(), 1);
        assert_eq!(in_range(&v, 0, 100).len(), 3);
        assert_eq!(in_range(&v, 3, 10).len(), 0);
        assert_eq!(in_range(&v, 50, 60).len(), 0);
        assert_eq!(spot_at(&v, 12).unwrap().start, 10);
        assert_eq!(
            spot_at(&v, 15).unwrap().start,
            10,
            "just after the word counts"
        );
        assert!(spot_at(&v, 17).is_none());
    }

    #[test]
    fn variants_get_their_own_dictionaries_and_missing_ones_are_reported() {
        let d = dir();
        std::fs::write(d.join("en_GB.aff"), "SET UTF-8\n").unwrap();
        std::fs::write(d.join("en_GB.dic"), "3\nthe\ncolour\nfox\n").unwrap();
        let mut s = SpellState::new("en", std::slice::from_ref(&d), None);
        let langs: BTreeSet<String> = ["en", "en-gb", "en-us", "de"].map(String::from).into();
        s.load_languages(&langs, std::slice::from_ref(&d));
        let doc = "<p lang=\"en-GB\">the colour</p><p lang=\"en-US\">the colour</p>";
        let words: Vec<(String, String)> =
            s.check(doc).into_iter().map(|s| (s.word, s.lang)).collect();
        assert_eq!(
            words,
            [("colour".to_string(), "en-us".to_string())],
            "only en-US rejects colour"
        );
        let missing: Vec<&str> = s
            .missing_languages()
            .iter()
            .map(|(l, _)| l.as_str())
            .collect();
        assert_eq!(
            missing,
            ["de"],
            "en-us reuses the en dictionary, de has none"
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn named_user_dictionaries_feed_the_checker() {
        let d = dir();
        let cfg = d.join("cfg");
        let user = cfg.join("spelling-words.txt");
        let mut s = SpellState::new("en", std::slice::from_ref(&d), Some(user));
        let dicts = s.user_dicts().unwrap().clone();
        dicts.create("Fantasy").unwrap();
        s.add_to_named("Fantasy", "quikc").unwrap();
        assert!(s.check(DOC).iter().all(|x| x.word != "quikc"));
        assert!(cfg.join("spelling-dictionaries/Fantasy.txt").is_file());
        dicts.set_enabled("Fantasy", false).unwrap();
        s.reload_user_words();
        assert!(
            s.check(DOC).iter().any(|x| x.word == "quikc"),
            "a disabled dictionary is not used"
        );
        dicts.set_enabled("Fantasy", true).unwrap();
        s.reload_user_words();
        assert!(s.check(DOC).iter().all(|x| x.word != "quikc"));
        s.add_to_dictionary("wrods").unwrap();
        assert!(dicts.read("Personal").words.contains(&"wrods".to_string()));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_no_spell_check_comment_silences_a_file() {
        let d = dir();
        let s = SpellState::new("en", std::slice::from_ref(&d), None);
        assert!(!s.check(DOC).is_empty());
        let marked = DOC.replace("<html>", "<html><!-- bookx-no-spell-check -->");
        assert!(s.check(&marked).is_empty());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn f8_moves_between_spots_and_wraps() {
        let sp = |s, e| Spot {
            start: s,
            end: e,
            word: "x".into(),
            lang: "en".into(),
        };
        let v = vec![sp(10, 15), sp(20, 25), sp(30, 35)];
        assert_eq!(pick_spot(&v, 0, true), Some(0));
        assert_eq!(
            pick_spot(&v, 10, true),
            Some(1),
            "the spot at the caret is skipped"
        );
        assert_eq!(pick_spot(&v, 22, true), Some(2));
        assert_eq!(pick_spot(&v, 30, true), Some(0), "wraps to the first");
        assert_eq!(pick_spot(&v, 100, false), Some(2));
        assert_eq!(pick_spot(&v, 20, false), Some(0));
        assert_eq!(pick_spot(&v, 10, false), Some(2), "wraps to the last");
        assert_eq!(pick_spot(&[], 5, true), None);
    }
}
