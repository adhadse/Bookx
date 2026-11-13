//! Ordered set of dictionaries with priority, enable/disable, word-form fallback,
//! lookup history and web lookup URL templates.

use crate::stardict::{Entry, StarDict};
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Result of looking a word up in one dictionary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictResult {
    pub dictionary: String,
    /// The candidate form that matched (may differ from the query: `run` for `running`).
    pub matched: String,
    pub entries: Vec<Entry>,
}

/// Outcome of [`DictionarySet::lookup_all`]: hits plus per-dictionary failures.
#[derive(Debug, Default)]
pub struct Lookup {
    pub results: Vec<DictResult>,
    /// `(dictionary name, error message)` for dictionaries that failed to answer.
    pub errors: Vec<(String, String)>,
}

struct Slot {
    dict: StarDict,
    enabled: bool,
}

/// Dictionaries in priority order (index 0 is the highest priority).
#[derive(Default)]
pub struct DictionarySet {
    slots: Vec<Slot>,
}

impl DictionarySet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a dictionary with the lowest priority; returns its index.
    pub fn add(&mut self, dict: StarDict) -> usize {
        self.slots.push(Slot {
            dict,
            enabled: true,
        });
        self.slots.len() - 1
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Names in priority order.
    pub fn names(&self) -> Vec<String> {
        self.slots
            .iter()
            .map(|s| s.dict.name().to_string())
            .collect()
    }

    fn slot(&self, i: usize) -> Result<&Slot> {
        self.slots
            .get(i)
            .ok_or_else(|| anyhow!("no dictionary at index {i}"))
    }

    pub fn is_enabled(&self, i: usize) -> Result<bool> {
        Ok(self.slot(i)?.enabled)
    }

    pub fn set_enabled(&mut self, i: usize, enabled: bool) -> Result<()> {
        self.slot(i)?;
        self.slots[i].enabled = enabled;
        Ok(())
    }

    /// Moves a dictionary to a new position (changes its priority).
    pub fn move_to(&mut self, from: usize, to: usize) -> Result<()> {
        self.slot(from)?;
        self.slot(to)?;
        let s = self.slots.remove(from);
        self.slots.insert(to, s);
        Ok(())
    }

    pub fn remove(&mut self, i: usize) -> Result<()> {
        self.slot(i)?;
        self.slots.remove(i);
        Ok(())
    }

    /// Looks `word` up in every enabled dictionary, trying [`word_forms`] in order and
    /// using the first form that has entries. Results are in priority order.
    pub fn lookup_all(&self, word: &str) -> Lookup {
        let forms = word_forms(word);
        let mut out = Lookup::default();
        for slot in self.slots.iter().filter(|s| s.enabled) {
            for form in &forms {
                match slot.dict.lookup(form) {
                    Ok(entries) if entries.is_empty() => continue,
                    Ok(entries) => {
                        out.results.push(DictResult {
                            dictionary: slot.dict.name().to_string(),
                            matched: form.clone(),
                            entries,
                        });
                        break;
                    }
                    Err(e) => {
                        out.errors
                            .push((slot.dict.name().to_string(), format!("{e:#}")));
                        break;
                    }
                }
            }
        }
        out
    }

    /// Prefix suggestions from all enabled dictionaries, deduplicated, priority order first.
    pub fn suggest(&self, prefix: &str, limit: usize) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for slot in self.slots.iter().filter(|s| s.enabled) {
            for w in slot.dict.suggest(prefix, limit) {
                if out.len() >= limit {
                    return out;
                }
                if !out.iter().any(|o| o.eq_ignore_ascii_case(&w)) {
                    out.push(w);
                }
            }
        }
        out
    }
}

const IRREGULAR: &[(&str, &str)] = &[
    ("better", "good"),
    ("best", "good"),
    ("went", "go"),
    ("gone", "go"),
    ("was", "be"),
    ("were", "be"),
    ("been", "be"),
    ("is", "be"),
    ("are", "be"),
    ("am", "be"),
    ("had", "have"),
    ("has", "have"),
    ("did", "do"),
    ("done", "do"),
    ("children", "child"),
    ("men", "man"),
    ("women", "woman"),
    ("feet", "foot"),
    ("teeth", "tooth"),
    ("mice", "mouse"),
    ("people", "person"),
    ("saw", "see"),
    ("seen", "see"),
    ("took", "take"),
    ("taken", "take"),
    ("came", "come"),
    ("made", "make"),
    ("said", "say"),
    ("knew", "know"),
    ("known", "know"),
    ("thought", "think"),
    ("got", "get"),
    ("gave", "give"),
    ("given", "give"),
    ("wrote", "write"),
    ("written", "write"),
];

/// Strips non-alphanumeric characters from both ends (keeps inner `'` and `-`).
pub fn clean_word(word: &str) -> String {
    word.trim()
        .trim_matches(|c: char| !c.is_alphanumeric())
        .replace('\u{2019}', "'")
}

fn push_unique(out: &mut Vec<String>, s: String) {
    if s.chars().count() >= 2 && !out.contains(&s) {
        out.push(s);
    }
}

fn drop_suffix(w: &str, n: usize) -> Option<&str> {
    let count = w.chars().count();
    if count <= n {
        return None;
    }
    let idx = w.char_indices().nth(count - n).map(|(i, _)| i)?;
    Some(&w[..idx])
}

/// Stem candidates for a lowercase English word (best guesses first).
fn english_stems(w: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some((_, base)) = IRREGULAR.iter().find(|(f, _)| *f == w) {
        push_unique(&mut out, base.to_string());
    }
    let len = w.chars().count();
    let is_vowel = |c: char| "aeiou".contains(c);
    let undouble = |stem: &str| -> Option<String> {
        let mut it = stem.chars().rev();
        let (a, b) = (it.next()?, it.next()?);
        (a == b && a.is_alphabetic() && !is_vowel(a))
            .then(|| stem[..stem.len() - a.len_utf8()].to_string())
    };
    let verbal = |stem: &str, out: &mut Vec<String>| {
        if let Some(u) = undouble(stem) {
            push_unique(out, u);
        }
        push_unique(out, format!("{stem}e"));
        push_unique(out, stem.to_string());
    };
    if w.ends_with("ies") && len > 4 {
        push_unique(&mut out, format!("{}y", drop_suffix(w, 3).unwrap_or("")));
    }
    if w.ends_with("ied") && len > 4 {
        push_unique(&mut out, format!("{}y", drop_suffix(w, 3).unwrap_or("")));
    }
    if w.ends_with("ier") && len > 4 {
        push_unique(&mut out, format!("{}y", drop_suffix(w, 3).unwrap_or("")));
    }
    if w.ends_with("iest") && len > 5 {
        push_unique(&mut out, format!("{}y", drop_suffix(w, 4).unwrap_or("")));
    }
    if w.ends_with("ily") && len > 4 {
        push_unique(&mut out, format!("{}y", drop_suffix(w, 3).unwrap_or("")));
    }
    if w.ends_with("ves") && len > 4 {
        let stem = drop_suffix(w, 3).unwrap_or("");
        push_unique(&mut out, format!("{stem}f"));
        push_unique(&mut out, format!("{stem}fe"));
    }
    if w.ends_with('s') && !w.ends_with("ss") && len > 3 {
        let sibilant = ["xes", "ses", "zes", "ches", "shes", "oes"]
            .iter()
            .any(|s| w.ends_with(s));
        if sibilant {
            push_unique(&mut out, drop_suffix(w, 2).unwrap_or("").to_string());
        }
        push_unique(&mut out, drop_suffix(w, 1).unwrap_or("").to_string());
    }
    if w.ends_with("ing") && len > 5 {
        verbal(drop_suffix(w, 3).unwrap_or(""), &mut out);
    }
    if w.ends_with("ed") && len > 4 {
        verbal(drop_suffix(w, 2).unwrap_or(""), &mut out);
    }
    if w.ends_with("est") && len > 4 {
        verbal(drop_suffix(w, 3).unwrap_or(""), &mut out);
    }
    if w.ends_with("er") && len > 4 {
        verbal(drop_suffix(w, 2).unwrap_or(""), &mut out);
    }
    if w.ends_with("ly") && len > 4 {
        push_unique(&mut out, drop_suffix(w, 2).unwrap_or("").to_string());
    }
    out
}

/// Candidate dictionary forms for a word as selected in a book, in the order to try:
/// the text as given, punctuation-stripped, lowercase, possessive removed, then
/// English word-form fallbacks (`running`→`run`, `libraries`→`library`, `better`→`good`).
pub fn word_forms(word: &str) -> Vec<String> {
    let mut out = Vec::new();
    let raw = word.trim();
    if raw.is_empty() {
        return out;
    }
    let mut push = |s: String| {
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    };
    push(raw.to_string());
    let cleaned = clean_word(raw);
    push(cleaned.clone());
    let lower = cleaned.to_lowercase();
    push(lower.clone());
    let mut base = lower;
    for suffix in ["'s", "'"] {
        if let Some(stripped) = base.strip_suffix(suffix) {
            if !stripped.is_empty() {
                base = stripped.to_string();
                push(base.clone());
                break;
            }
        }
    }
    for s in english_stems(&base) {
        push(s);
    }
    out
}

/// Bounded most-recent-first list of looked-up words (case-insensitive dedup).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LookupHistory {
    capacity: usize,
    words: VecDeque<String>,
}

impl LookupHistory {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            words: VecDeque::new(),
        }
    }

    /// Records a lookup; blank words are ignored, repeats move to the front.
    pub fn record(&mut self, word: &str) {
        let w = word.trim();
        if w.is_empty() || self.capacity == 0 {
            return;
        }
        let lw = w.to_lowercase();
        self.words.retain(|x| x.to_lowercase() != lw);
        self.words.push_front(w.to_string());
        self.words.truncate(self.capacity);
    }

    pub fn entries(&self) -> Vec<String> {
        self.words.iter().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.words.len()
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    pub fn clear(&mut self) {
        self.words.clear();
    }

    /// Removes one word (case-insensitive); returns whether something was removed.
    pub fn remove(&mut self, word: &str) -> bool {
        let lw = word.trim().to_lowercase();
        let before = self.words.len();
        self.words.retain(|x| x.to_lowercase() != lw);
        before != self.words.len()
    }
}

/// Percent-encodes UTF-8 bytes (everything except RFC 3986 unreserved characters).
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A custom lookup website described by a URL template containing `{word}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebSource {
    pub name: String,
    pub template: String,
}

impl WebSource {
    pub fn new(name: &str, template: &str) -> Result<Self> {
        if name.trim().is_empty() {
            bail!("web source needs a name");
        }
        if !(template.starts_with("https://") || template.starts_with("http://")) {
            bail!("web source URL must start with http:// or https://");
        }
        if !template.contains("{word}") {
            bail!("web source URL must contain {{word}}");
        }
        Ok(Self {
            name: name.trim().to_string(),
            template: template.to_string(),
        })
    }

    /// Builds the lookup URL; the word is trimmed and percent-encoded.
    pub fn url(&self, word: &str) -> Result<String> {
        let w = word.trim();
        if w.is_empty() {
            bail!("nothing to look up");
        }
        Ok(self.template.replace("{word}", &percent_encode(w)))
    }

    /// Sensible built-in sources.
    pub fn defaults() -> Vec<WebSource> {
        [
            ("Wiktionary", "https://en.wiktionary.org/wiki/{word}"),
            (
                "Google define",
                "https://www.google.com/search?q=define+{word}",
            ),
            ("Wikipedia", "https://en.wikipedia.org/wiki/{word}"),
        ]
        .iter()
        .filter_map(|(n, t)| WebSource::new(n, t).ok())
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{write_dict, DictSpec, TmpDir};

    fn open(t: &TmpDir, name: &str, words: &[(&str, &str)]) -> StarDict {
        let spec = DictSpec::new(
            name,
            words
                .iter()
                .map(|(w, d)| (*w, d.as_bytes().to_vec()))
                .collect(),
        );
        StarDict::open(write_dict(t.path(), &spec)).unwrap()
    }

    fn forms(w: &str) -> Vec<String> {
        word_forms(w)
    }

    #[test]
    fn stems_cover_the_documented_examples() {
        assert!(forms("running").contains(&"run".to_string()));
        assert!(forms("libraries").contains(&"library".to_string()));
        assert!(forms("walked").contains(&"walk".to_string()));
        assert!(forms("better").contains(&"good".to_string()));
        assert!(forms("making").contains(&"make".to_string()));
        assert!(forms("boxes").contains(&"box".to_string()));
        assert!(forms("hopped").contains(&"hop".to_string()));
        assert!(forms("happier").contains(&"happy".to_string()));
        assert!(forms("quickly").contains(&"quick".to_string()));
        assert!(forms("happily").contains(&"happy".to_string()));
        assert!(forms("knives").contains(&"knife".to_string()));
        assert!(forms("studied").contains(&"study".to_string()));
        assert!(forms("cats").contains(&"cat".to_string()));
        assert!(forms("biggest").contains(&"big".to_string()));
        assert!(forms("children").contains(&"child".to_string()));
        assert!(forms("went").contains(&"go".to_string()));
    }

    #[test]
    fn forms_order_raw_cleaned_lower_then_stems() {
        let f = forms("  “Running,” ");
        assert_eq!(f[0], "“Running,”");
        assert_eq!(f[1], "Running");
        assert_eq!(f[2], "running");
        assert_eq!(f[3], "run");
        assert_eq!(forms("cat"), vec!["cat".to_string()]);
    }

    #[test]
    fn forms_possessives_and_apostrophes() {
        assert!(forms("dog’s").contains(&"dog".to_string()));
        assert!(forms("dog's").contains(&"dog".to_string()));
        assert!(forms("dogs'").contains(&"dog".to_string()));
        assert!(forms("don’t").contains(&"don't".to_string()));
        assert_eq!(forms("'s")[0], "'s");
    }

    #[test]
    fn forms_degenerate_inputs() {
        assert!(forms("").is_empty());
        assert!(forms("   ").is_empty());
        assert_eq!(forms("!!!"), vec!["!!!".to_string()]);
        for w in [
            "s",
            "ed",
            "ing",
            "ies",
            "es",
            "ss",
            "ly",
            "é",
            "ïes",
            "日本語",
            "i",
        ] {
            let _ = forms(w); // must not panic
        }
        assert!(!forms("ing").contains(&"".to_string()));
        assert!(!forms("ss").iter().any(|f| f == "s"));
    }

    #[test]
    fn forms_unicode_stems_do_not_split_chars() {
        assert!(forms("naïves").iter().all(|f| !f.is_empty()));
        assert!(forms("cafés").contains(&"café".to_string()));
        assert!(forms("Überlaufed").iter().any(|f| f == "überlauf"));
    }

    #[test]
    fn forms_are_unique_and_deterministic() {
        for w in ["running", "libraries", "bosses", "Better", "stopped", "x"] {
            let f = forms(w);
            let mut d = f.clone();
            d.sort();
            d.dedup();
            assert_eq!(f.len(), d.len(), "{w}");
            assert_eq!(f, forms(w));
        }
    }

    #[test]
    fn clean_word_trims_punctuation_only_at_edges() {
        assert_eq!(clean_word("(well-known)."), "well-known");
        assert_eq!(clean_word("“don’t”"), "don't");
        assert_eq!(clean_word("...."), "");
        assert_eq!(clean_word("  ok "), "ok");
    }

    #[test]
    fn set_priority_enable_and_lookup_all() {
        let t = TmpDir::new("set");
        let a = open(
            &t,
            "alpha",
            &[("run", "to move fast"), ("library", "books")],
        );
        let b = open(&t, "beta", &[("run", "operate"), ("walk", "go on foot")]);
        let mut set = DictionarySet::new();
        assert!(set.is_empty());
        assert_eq!(set.add(a), 0);
        assert_eq!(set.add(b), 1);
        assert_eq!(set.len(), 2);
        assert_eq!(set.names(), vec!["alpha", "beta"]);

        let l = set.lookup_all("Running");
        assert!(l.errors.is_empty());
        assert_eq!(l.results.len(), 2);
        assert_eq!(l.results[0].dictionary, "alpha");
        assert_eq!(l.results[0].matched, "run");
        assert_eq!(l.results[1].entries[0].plain_text(), "operate");

        set.move_to(1, 0).unwrap();
        assert_eq!(set.lookup_all("run").results[0].dictionary, "beta");

        set.set_enabled(0, false).unwrap();
        assert!(!set.is_enabled(0).unwrap());
        let l = set.lookup_all("run");
        assert_eq!(l.results.len(), 1);
        assert_eq!(l.results[0].dictionary, "alpha");

        assert_eq!(set.lookup_all("walked").results.len(), 0);
        set.set_enabled(0, true).unwrap();
        assert_eq!(set.lookup_all("walked").results[0].matched, "walk");
        assert!(set.lookup_all("").results.is_empty());
        assert!(set.lookup_all("zzzz").results.is_empty());

        set.remove(0).unwrap();
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn set_index_errors() {
        let mut set = DictionarySet::new();
        assert!(set.is_enabled(0).is_err());
        assert!(set.set_enabled(0, true).is_err());
        assert!(set.move_to(0, 0).is_err());
        assert!(set.remove(3).is_err());
        let t = TmpDir::new("seterr");
        set.add(open(&t, "a", &[("x", "y")]));
        assert!(set.move_to(0, 1).is_err());
        assert!(set.move_to(2, 0).is_err());
        assert!(set.move_to(0, 0).is_ok());
    }

    #[test]
    fn broken_dictionary_is_reported_but_others_answer() {
        let t = TmpDir::new("broken");
        let good = open(&t, "good", &[("word", "fine")]);
        let t2 = TmpDir::new("broken2");
        let spec = DictSpec::new("bad", vec![("word", b"fine".to_vec())]);
        let ifo = write_dict(t2.path(), &spec);
        std::fs::write(t2.path().join("bad.dict"), b"x").unwrap(); // too short
        let bad = StarDict::open(ifo).unwrap();
        let mut set = DictionarySet::new();
        set.add(bad);
        set.add(good);
        let l = set.lookup_all("word");
        assert_eq!(l.results.len(), 1);
        assert_eq!(l.results[0].dictionary, "good");
        assert_eq!(l.errors.len(), 1);
        assert_eq!(l.errors[0].0, "bad");
        assert!(l.errors[0].1.contains("beyond"));
    }

    #[test]
    fn suggest_merges_dedups_and_limits() {
        let t = TmpDir::new("sugg");
        let a = open(&t, "a", &[("apple", "1"), ("apply", "2")]);
        let b = open(&t, "b", &[("Apple", "3"), ("apricot", "4")]);
        let mut set = DictionarySet::new();
        set.add(a);
        set.add(b);
        assert_eq!(set.suggest("ap", 10), vec!["apple", "apply", "apricot"]);
        assert_eq!(set.suggest("ap", 2), vec!["apple", "apply"]);
        assert!(set.suggest("ap", 0).is_empty());
        set.set_enabled(1, false).unwrap();
        assert_eq!(set.suggest("ap", 10), vec!["apple", "apply"]);
    }

    #[test]
    fn history_bounded_dedup_recent_first() {
        let mut h = LookupHistory::new(3);
        assert!(h.is_empty());
        for w in ["a", "b", "c", "d"] {
            h.record(w);
        }
        assert_eq!(h.entries(), vec!["d", "c", "b"]);
        h.record("C");
        assert_eq!(h.entries(), vec!["C", "d", "b"]);
        h.record("  ");
        h.record("");
        assert_eq!(h.len(), 3);
        assert!(h.remove("D"));
        assert!(!h.remove("zzz"));
        assert_eq!(h.entries(), vec!["C", "b"]);
        h.clear();
        assert!(h.is_empty());
    }

    #[test]
    fn history_zero_capacity_and_serde() {
        let mut h = LookupHistory::new(0);
        h.record("a");
        assert!(h.is_empty());
        let mut h = LookupHistory::new(2);
        h.record("é");
        h.record("日本");
        let json = serde_json::to_string(&h).unwrap();
        let back: LookupHistory = serde_json::from_str(&json).unwrap();
        assert_eq!(back, h);
    }

    #[test]
    fn percent_encoding_golden() {
        assert_eq!(percent_encode("abc-._~XYZ09"), "abc-._~XYZ09");
        assert_eq!(percent_encode("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
        assert_eq!(percent_encode("日本"), "%E6%97%A5%E6%9C%AC");
        assert_eq!(percent_encode(""), "");
        assert_eq!(percent_encode("%"), "%25");
    }

    #[test]
    fn web_source_builds_urls() {
        let s = WebSource::new("Example", "https://example.com/?q={word}").unwrap();
        assert_eq!(
            s.url(" naïve café ").unwrap(),
            "https://example.com/?q=na%C3%AFve%20caf%C3%A9"
        );
        let twice = WebSource::new("t", "http://x.org/{word}/{word}").unwrap();
        assert_eq!(twice.url("a b").unwrap(), "http://x.org/a%20b/a%20b");
        assert!(s.url("  ").is_err());
        // injection characters are encoded, not interpreted
        assert_eq!(
            s.url("a&b=c#d").unwrap(),
            "https://example.com/?q=a%26b%3Dc%23d"
        );
    }

    #[test]
    fn web_source_validation_and_defaults() {
        assert!(WebSource::new("", "https://x/{word}").is_err());
        assert!(WebSource::new("n", "ftp://x/{word}").is_err());
        assert!(WebSource::new("n", "javascript:alert({word})").is_err());
        assert!(WebSource::new("n", "https://x/").is_err());
        let d = WebSource::defaults();
        assert_eq!(d.len(), 3);
        assert!(d.iter().all(|s| s.url("test").unwrap().contains("test")));
        let json = serde_json::to_string(&d[0]).unwrap();
        assert_eq!(serde_json::from_str::<WebSource>(&json).unwrap(), d[0]);
    }
}
