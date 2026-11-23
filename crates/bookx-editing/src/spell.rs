//! Spell checking of XHTML text with Hunspell-compatible dictionaries (E5/E20).
//!
//! No dictionaries are bundled: callers load `.aff`/`.dic` files for the languages they have.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use bookx_core::book::OpenBook;
use spellbook::Dictionary;

use crate::markup::*;
use crate::reports::{book_text_files, primary_language, word_ranges};

/// Elements whose text is never spell checked.
pub const SKIP_ELEMENTS: &[&str] = &[
    "code", "pre", "kbd", "samp", "var", "script", "style", "head", "tt", "math",
];

/// A word found in the text content of a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordSpan {
    /// Byte offsets of the word in the source document.
    pub start: usize,
    pub end: usize,
    /// Decoded word.
    pub word: String,
    /// Language from the nearest `xml:lang`/`lang` attribute, if any.
    pub lang: Option<String>,
}

/// Extracts checkable words from the text nodes of `src` only (never attributes, code, scripts or styles).
/// Offsets point into `src`. Words containing digits are ignored.
pub fn extract_words(src: &str) -> Vec<WordSpan> {
    let mut out = Vec::new();
    let mut stack: Vec<(String, Option<String>)> = Vec::new();
    let mut skip_depth = 0usize;
    for ev in scan(src) {
        match ev {
            Event::Open {
                name,
                range,
                self_closing,
            } => {
                if self_closing {
                    continue;
                }
                let tag = &src[range];
                let lang = tag_attr(tag, "xml:lang")
                    .or_else(|| tag_attr(tag, "lang"))
                    .filter(|l| !l.trim().is_empty());
                let skip = SKIP_ELEMENTS.contains(&name.as_str())
                    || tag_attr(tag, "translate").is_some_and(|v| v == "no");
                if skip {
                    skip_depth += 1;
                }
                stack.push((name, lang));
                // remember whether this element incremented skip_depth
                if skip {
                    stack.last_mut().unwrap().0.push('\u{0}');
                }
            }
            Event::Close { name, .. } => {
                if let Some(pos) = stack
                    .iter()
                    .rposition(|(n, _)| n.trim_end_matches('\u{0}') == name)
                {
                    for (n, _) in stack.drain(pos..) {
                        if n.ends_with('\u{0}') {
                            skip_depth -= 1;
                        }
                    }
                }
            }
            Event::Text(r) => {
                if skip_depth > 0 {
                    continue;
                }
                let raw = &src[r.clone()];
                let (decoded, map) = decode_with_offsets(raw);
                let lang = stack.iter().rev().find_map(|(_, l)| l.clone());
                for w in word_ranges(&decoded) {
                    let word = &decoded[w.clone()];
                    if word.chars().any(|c| c.is_numeric()) {
                        continue;
                    }
                    out.push(WordSpan {
                        start: r.start + map[w.start],
                        end: r.start + map[w.end],
                        word: word.to_string(),
                        lang: lang.clone(),
                    });
                }
            }
            Event::Other(_) => {}
        }
    }
    out
}

/// Dictionaries per language plus the user's word list and ignore list.
#[derive(Default, Clone)]
pub struct SpellChecker {
    dicts: HashMap<String, Arc<Dictionary>>,
    user_words: HashSet<String>,
    ignored: HashSet<String>,
}

impl SpellChecker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers the dictionary for a language tag (`en`, `en-US`, `de_DE`). The full tag is the key, so `en-US` and
    /// `en-GB` can both be loaded; a later dictionary for the same tag replaces the earlier one. Lookups of a tag
    /// without its own dictionary fall back to the bare language, then to any variant (see [`lookup_key`]).
    pub fn add_dictionary(&mut self, lang: &str, aff: &str, dic: &str) -> Result<()> {
        let key = lang_key(lang);
        if primary_language(lang).is_empty() {
            bail!("empty language tag");
        }
        let d = Dictionary::new(aff, dic)
            .map_err(|e| anyhow::anyhow!("invalid dictionary for {lang}: {e}"))?;
        self.dicts.insert(key, Arc::new(d));
        Ok(())
    }

    /// Loads `.aff` and `.dic` files (UTF-8).
    pub fn add_dictionary_files(&mut self, lang: &str, aff: &Path, dic: &Path) -> Result<()> {
        let a = std::fs::read_to_string(aff)
            .with_context(|| format!("reading {} (UTF-8 expected)", aff.display()))?;
        let d = std::fs::read_to_string(dic)
            .with_context(|| format!("reading {} (UTF-8 expected)", dic.display()))?;
        self.add_dictionary(lang, &a, &d)
    }

    pub fn has_language(&self, lang: &str) -> bool {
        self.lookup(lang).is_some()
    }

    /// The key of the dictionary that serves `lang`: the exact tag, else the bare language, else a variant (`en-us`
    /// first for English, otherwise the first in alphabetical order).
    pub fn lookup_key(&self, lang: &str) -> Option<String> {
        let key = lang_key(lang);
        if self.dicts.contains_key(&key) {
            return Some(key);
        }
        let primary = primary_language(lang);
        if self.dicts.contains_key(&primary) {
            return Some(primary);
        }
        let preferred = format!("{primary}-{primary}");
        let mut variants: Vec<&String> = self
            .dicts
            .keys()
            .filter(|k| k.split('-').next() == Some(primary.as_str()))
            .collect();
        variants.sort();
        if primary == "en" && self.dicts.contains_key("en-us") {
            return Some("en-us".into());
        }
        if self.dicts.contains_key(&preferred) {
            return Some(preferred);
        }
        variants.first().map(|k| (*k).clone())
    }

    #[cfg(test)]
    fn remove_dictionary_for_test(&mut self, key: &str) {
        self.dicts.remove(key);
    }

    fn lookup(&self, lang: &str) -> Option<&Dictionary> {
        self.lookup_key(lang)
            .and_then(|k| self.dicts.get(&k))
            .map(|d| d.as_ref())
    }

    pub fn languages(&self) -> Vec<String> {
        let mut v: Vec<String> = self.dicts.keys().cloned().collect();
        v.sort();
        v
    }

    /// Adds a word to the personal dictionary (stored lower-cased and as written).
    pub fn add_user_word(&mut self, word: &str) {
        let w = normalize(word);
        if !w.is_empty() {
            self.user_words.insert(w);
        }
    }

    pub fn remove_user_word(&mut self, word: &str) -> bool {
        self.user_words.remove(&normalize(word))
    }

    /// Replaces the personal dictionary with the lines of `text` (blank lines and `#` comments ignored).
    pub fn load_user_words(&mut self, text: &str) {
        self.user_words.clear();
        for l in text.lines() {
            let l = l.trim();
            if !l.is_empty() && !l.starts_with('#') {
                self.add_user_word(l);
            }
        }
    }

    /// Personal dictionary as sorted lines.
    pub fn user_words_text(&self) -> String {
        let mut v: Vec<&String> = self.user_words.iter().collect();
        v.sort();
        v.iter().map(|w| format!("{w}\n")).collect()
    }

    /// Words ignored for this session (not persisted by the checker).
    pub fn ignore_word(&mut self, word: &str) {
        self.ignored.insert(normalize(word));
    }

    /// Takes a word back off the session ignore list (the undo of [`SpellChecker::ignore_word`]). True when it was there.
    pub fn unignore_word(&mut self, word: &str) -> bool {
        self.ignored.remove(&normalize(word))
    }

    pub fn clear_ignored(&mut self) {
        self.ignored.clear();
    }

    /// `None` when there is no dictionary for `lang`.
    pub fn check_word(&self, lang: &str, word: &str) -> Option<bool> {
        let d = self.lookup(lang)?;
        let w = normalize(word);
        if self.user_words.contains(&w) || self.ignored.contains(&w) {
            return Some(true);
        }
        if d.check(&w) {
            return Some(true);
        }
        if w.contains('-') {
            let parts: Vec<&str> = w.split('-').filter(|p| !p.is_empty()).collect();
            if parts.len() > 1
                && parts
                    .iter()
                    .all(|p| d.check(p) || self.user_words.contains(&p.to_lowercase()))
            {
                return Some(true);
            }
        }
        Some(false)
    }

    /// Suggestions for a misspelled word (empty if no dictionary).
    pub fn suggest(&self, lang: &str, word: &str) -> Vec<String> {
        let Some(d) = self.lookup(lang) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        d.suggest(&normalize(word), &mut out);
        out
    }
}

/// `en_GB` and `en-gb` both become `en-gb`.
pub fn lang_key(tag: &str) -> String {
    tag.trim().replace('_', "-").to_ascii_lowercase()
}

/// The usual spelling of a tag for file names: `en-gb` and `en_GB` become `en-GB`, `zh-hant-tw` becomes
/// `zh-Hant-TW` (language lower case, a 4 letter script capitalised, a 2 letter region upper case).
pub fn canonical_tag(tag: &str) -> String {
    tag.trim()
        .replace('_', "-")
        .split('-')
        .enumerate()
        .map(|(i, part)| match (i, part.len()) {
            (0, _) => part.to_ascii_lowercase(),
            (_, 4) => {
                let mut c = part.to_ascii_lowercase();
                if let Some(f) = c.get_mut(..1) {
                    f.make_ascii_uppercase();
                }
                c
            }
            (_, 2) => part.to_ascii_uppercase(),
            _ => part.to_string(),
        })
        .collect::<Vec<_>>()
        .join("-")
}

fn normalize(word: &str) -> String {
    word.trim().replace('’', "'")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Misspelling {
    pub file: String,
    pub start: usize,
    pub end: usize,
    pub word: String,
    pub lang: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpellReport {
    pub misspellings: Vec<Misspelling>,
    pub words_checked: usize,
    /// Languages found in the text for which no dictionary is loaded (those words were not checked).
    pub unavailable_languages: BTreeSet<String>,
}

impl SpellReport {
    pub fn by_file(&self) -> BTreeMap<&str, Vec<&Misspelling>> {
        let mut m: BTreeMap<&str, Vec<&Misspelling>> = BTreeMap::new();
        for x in &self.misspellings {
            m.entry(x.file.as_str()).or_default().push(x);
        }
        m
    }

    /// Unique misspelled words with their number of occurrences, most frequent first.
    pub fn unique_words(&self) -> Vec<(String, usize)> {
        let mut c: HashMap<&str, usize> = HashMap::new();
        for m in &self.misspellings {
            *c.entry(m.word.as_str()).or_default() += 1;
        }
        let mut v: Vec<(String, usize)> = c.into_iter().map(|(w, n)| (w.to_string(), n)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }
}

/// Spell checks one document. `default_lang` applies where the text has no `lang` of its own.
pub fn check_source(
    checker: &SpellChecker,
    file: &str,
    src: &str,
    default_lang: &str,
    report: &mut SpellReport,
    cache: &mut HashMap<(String, String), Option<bool>>,
) {
    let doc_lang = crate::reports::document_language(src);
    for w in extract_words(src) {
        let lang = w
            .lang
            .clone()
            .or_else(|| doc_lang.clone())
            .unwrap_or_else(|| default_lang.to_string());
        let key = lang_key(&lang);
        let res = *cache
            .entry((key.clone(), w.word.clone()))
            .or_insert_with(|| checker.check_word(&key, &w.word));
        match res {
            None => {
                report.unavailable_languages.insert(key);
            }
            Some(ok) => {
                report.words_checked += 1;
                if !ok {
                    report.misspellings.push(Misspelling {
                        file: file.to_string(),
                        start: w.start,
                        end: w.end,
                        word: w.word,
                        lang: key,
                    });
                }
            }
        }
    }
}

/// Spell checks all text files of the book (spine order first). The language of the package is the default.
pub fn check_book_spelling(book: &OpenBook, checker: &SpellChecker) -> SpellReport {
    let mut report = SpellReport::default();
    let mut cache = HashMap::new();
    let default_lang = book.package.metadata.language.clone();
    for f in book_text_files(book) {
        if let Some(src) = book.archive.get_string(&f) {
            check_source(checker, &f, &src, &default_lang, &mut report, &mut cache);
        }
    }
    report
}

/// Replaces one misspelling by editing only that text range. Fails if the text no longer matches (stale offsets).
pub fn apply_replacement(src: &str, m: &Misspelling, replacement: &str) -> Result<String> {
    let raw = src
        .get(m.start..m.end)
        .context("the text has changed since it was checked")?;
    if decode_entities(raw) != m.word {
        bail!("the text has changed since it was checked");
    }
    if replacement.is_empty() {
        bail!("empty replacement");
    }
    let mut out = String::with_capacity(src.len() + replacement.len());
    out.push_str(&src[..m.start]);
    out.push_str(&escape_text(replacement));
    out.push_str(&src[m.end..]);
    Ok(out)
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Replaces every occurrence of the misspelled `word` (same language) in `src`, working from the end so offsets
/// stay valid. Returns the new source and the number of replacements.
pub fn replace_all_in_source(src: &str, word: &str, replacement: &str) -> Result<(String, usize)> {
    if replacement.is_empty() {
        bail!("empty replacement");
    }
    let mut spans: Vec<WordSpan> = extract_words(src)
        .into_iter()
        .filter(|w| w.word == word)
        .collect();
    spans.sort_by_key(|s| std::cmp::Reverse(s.start));
    let mut out = src.to_string();
    let n = spans.len();
    let rep = escape_text(replacement);
    for s in spans {
        out.replace_range(s.start..s.end, &rep);
    }
    Ok((out, n))
}

/// Applies a replacement to the file in the book (all occurrences of the misspelled word). Returns the count.
pub fn replace_in_book(
    book: &mut OpenBook,
    file: &str,
    word: &str,
    replacement: &str,
) -> Result<usize> {
    let src = book
        .archive
        .get_string(file)
        .with_context(|| format!("{file} not found"))?;
    let (new, n) = replace_all_in_source(&src, word, replacement)?;
    if n > 0 {
        book.archive.set_string(file, &new);
    }
    Ok(n)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::testutil::make_book;

    pub(crate) const AFF: &str = "SET UTF-8\nTRY esianrtolcdugmphbyfvkwz\nSFX S Y 1\nSFX S 0 s .\n";
    pub(crate) const DIC: &str =
        "12\nhello\nworld/S\nthe\ncat/S\ndog/S\nquick\nbrown\nfox/S\ntext\nmore\nand\nwell\n";
    const DE_AFF: &str = "SET UTF-8\n";
    const DE_DIC: &str = "3\nder\nhund\nkatze\n";

    pub(crate) fn checker() -> SpellChecker {
        let mut c = SpellChecker::new();
        c.add_dictionary("en-US", AFF, DIC).unwrap();
        c
    }

    #[test]
    fn extract_only_text_nodes() {
        let src = "<html lang=\"en\"><head><title>Titel</title><style>.zzqq{}</style></head><body><p class=\"qqzz\" title=\"attrword\">Hello <code>codeword</code> wörld <b>fox</b>es 42 abc123 &amp; caf&#233;</p><pre>preword</pre><script>var scriptword;</script><p lang=\"de\">Der <span>Hund</span></p><p>after</p></body></html>";
        let w = extract_words(src);
        let words: Vec<&str> = w.iter().map(|w| w.word.as_str()).collect();
        assert_eq!(
            words,
            vec![
                "Hello", "wörld", "fox", "es", "café", "Der", "Hund", "after"
            ]
        );
        let de: Vec<&str> = w
            .iter()
            .filter(|w| w.lang.as_deref() == Some("de"))
            .map(|w| w.word.as_str())
            .collect();
        assert_eq!(de, vec!["Der", "Hund"]);
        assert_eq!(
            w.last().unwrap().lang.as_deref(),
            Some("en"),
            "inherited from <html>"
        );
        assert_eq!(extract_words("<p>x</p>")[0].lang, None);
        for s in &w {
            let raw = &src[s.start..s.end];
            assert_eq!(decode_entities(raw), s.word, "offsets point at the word");
        }
        let cafe = w.iter().find(|s| s.word == "café").unwrap();
        assert_eq!(&src[cafe.start..cafe.end], "caf&#233;");
    }

    #[test]
    fn extract_translate_no_and_unbalanced_markup() {
        let w = extract_words("<body><p translate=\"no\">Skipme</p><p>keep <i>this</p> end</body>");
        let words: Vec<&str> = w.iter().map(|w| w.word.as_str()).collect();
        assert_eq!(words, vec!["keep", "this", "end"]);
        assert!(extract_words("").is_empty());
        assert!(extract_words("<p><code>unclosed code word").is_empty());
    }

    #[test]
    fn tags_get_their_usual_spelling() {
        assert_eq!(canonical_tag("en-gb"), "en-GB");
        assert_eq!(canonical_tag("EN_us"), "en-US");
        assert_eq!(canonical_tag("zh-hant-tw"), "zh-Hant-TW");
        assert_eq!(canonical_tag(" de "), "de");
        assert_eq!(canonical_tag("es-419"), "es-419");
        assert_eq!(canonical_tag(""), "");
        assert_eq!(lang_key("en_GB"), "en-gb");
    }

    #[test]
    fn dictionary_loading_and_errors() {
        let mut c = SpellChecker::new();
        assert!(!c.has_language("en"));
        c.add_dictionary("en_GB", AFF, DIC).unwrap();
        assert!(
            c.has_language("en-US"),
            "a variant serves the other variants"
        );
        assert!(c.has_language("en"));
        assert!(!c.has_language("de"));
        assert_eq!(c.languages(), vec!["en-gb"]);
        c.add_dictionary("en-US", AFF, DIC).unwrap();
        assert_eq!(c.languages(), vec!["en-gb", "en-us"]);
        assert_eq!(c.lookup_key("en-US").as_deref(), Some("en-us"));
        assert_eq!(
            c.lookup_key("en_AU").as_deref(),
            Some("en-us"),
            "English falls back to en-US"
        );
        assert_eq!(c.lookup_key("en-GB").as_deref(), Some("en-gb"));
        c.add_dictionary("fr-CA", AFF, DIC).unwrap();
        c.add_dictionary("fr-FR", AFF, DIC).unwrap();
        assert_eq!(
            c.lookup_key("fr").as_deref(),
            Some("fr-fr"),
            "else the main variant"
        );
        c.remove_dictionary_for_test("fr-fr");
        assert_eq!(
            c.lookup_key("fr").as_deref(),
            Some("fr-ca"),
            "else the first variant"
        );
        c.add_dictionary("fr", AFF, DIC).unwrap();
        assert_eq!(
            c.lookup_key("fr-BE").as_deref(),
            Some("fr"),
            "the bare language beats a variant"
        );
        assert!(c.add_dictionary("", AFF, DIC).is_err());
        let e = c
            .add_dictionary("xx", "SFX garbage\n", "notanumber\n")
            .unwrap_err();
        assert!(
            e.to_string().starts_with("invalid dictionary for xx"),
            "{e}"
        );
        let dir = std::env::temp_dir().join(format!("bookx-spell-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("de.aff"), DE_AFF).unwrap();
        std::fs::write(dir.join("de.dic"), DE_DIC).unwrap();
        c.add_dictionary_files("de", &dir.join("de.aff"), &dir.join("de.dic"))
            .unwrap();
        assert_eq!(c.check_word("de", "hund"), Some(true));
        assert!(
            c.add_dictionary_files("fr", &dir.join("nope.aff"), &dir.join("de.dic"))
                .is_err()
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn check_words_with_affixes_case_and_hyphens() {
        let c = checker();
        assert_eq!(c.check_word("en", "hello"), Some(true));
        assert_eq!(c.check_word("en", "Hello"), Some(true));
        assert_eq!(c.check_word("en", "worlds"), Some(true));
        assert_eq!(c.check_word("en", "wrold"), Some(false));
        assert_eq!(c.check_word("en", "quick-brown"), Some(true));
        assert_eq!(c.check_word("en", "quick-zzz"), Some(false));
        assert_eq!(c.check_word("fr", "bonjour"), None);
        assert_eq!(c.check_word("en", "the’s"), Some(false));
    }

    #[test]
    fn user_words_and_ignore_list() {
        let mut c = checker();
        assert_eq!(c.check_word("en", "Bookx"), Some(false));
        c.add_user_word("Bookx");
        assert_eq!(c.check_word("en", "Bookx"), Some(true));
        assert!(c.remove_user_word("Bookx"));
        assert!(!c.remove_user_word("Bookx"));
        c.ignore_word("zork");
        assert_eq!(c.check_word("en", "zork"), Some(true));
        assert!(c.unignore_word("zork"));
        assert!(!c.unignore_word("zork"));
        assert_eq!(c.check_word("en", "zork"), Some(false));
        c.ignore_word("zork");
        c.clear_ignored();
        assert_eq!(c.check_word("en", "zork"), Some(false));
        c.load_user_words("# comment\nalpha\n\n beta \n");
        assert_eq!(c.user_words_text(), "alpha\nbeta\n");
        assert_eq!(c.check_word("en", "beta"), Some(true));
        c.load_user_words("gamma");
        assert_eq!(c.check_word("en", "beta"), Some(false));
        c.add_user_word("   ");
        assert_eq!(c.user_words_text(), "gamma\n");
    }

    #[test]
    fn suggestions() {
        let c = checker();
        assert!(c.suggest("en", "wrold").contains(&"world".to_string()));
        assert!(c.suggest("en", "helo").contains(&"hello".to_string()));
        assert!(c.suggest("zz", "helo").is_empty());
    }

    #[test]
    fn check_source_report() {
        let mut c = checker();
        c.add_dictionary("de", DE_AFF, DE_DIC).unwrap();
        let src = "<html lang=\"en\"><body><p>The quikc cat and Der <span lang=\"de\">Hund und katze</span> <span lang=\"fr\">bonjour</span></p></body></html>";
        let mut r = SpellReport::default();
        check_source(&c, "a.xhtml", src, "en", &mut r, &mut HashMap::new());
        let words: Vec<&str> = r.misspellings.iter().map(|m| m.word.as_str()).collect();
        assert_eq!(words, vec!["quikc", "Der", "und"]);
        assert_eq!(r.misspellings[2].lang, "de");
        assert_eq!(
            r.unavailable_languages.iter().collect::<Vec<_>>(),
            vec!["fr"]
        );
        assert_eq!(r.words_checked, 8);
        let m = &r.misspellings[0];
        assert_eq!(&src[m.start..m.end], "quikc");
    }

    #[test]
    fn check_book_per_file_and_unique_words() {
        let c = checker();
        let book = make_book(&["<p>the cat tehe</p><p>tehe wrold</p>", "<p>hello tehe</p>"]);
        let r = check_book_spelling(&book, &c);
        let by = r.by_file();
        assert_eq!(by["OEBPS/chapter-1.xhtml"].len(), 3);
        assert_eq!(by["OEBPS/chapter-2.xhtml"].len(), 1);
        let u = r.unique_words();
        assert_eq!(u[0], ("tehe".to_string(), 3));
        assert!(u.contains(&("wrold".to_string(), 1)));
        // the nav document ("Contents", "Ch") is checked too and flagged: the dictionary is tiny
        assert!(by.contains_key("OEBPS/nav.xhtml"));
        assert!(r.words_checked > 8);
        let none = check_book_spelling(&book, &SpellChecker::new());
        assert!(none.misspellings.is_empty());
        assert!(none.unavailable_languages.contains("en"));
    }

    #[test]
    fn apply_replacement_edits_only_the_word() {
        let src = "<p class=\"wrold\">the wrold &amp; <b>wrold</b></p>";
        let ms: Vec<Misspelling> = {
            let mut r = SpellReport::default();
            check_source(&checker(), "f", src, "en", &mut r, &mut HashMap::new());
            r.misspellings
        };
        assert_eq!(ms.len(), 2);
        let out = apply_replacement(src, &ms[0], "world").unwrap();
        assert_eq!(out, "<p class=\"wrold\">the world &amp; <b>wrold</b></p>");
        let out = apply_replacement(src, &ms[1], "a<b & c").unwrap();
        assert!(out.ends_with("<b>a&lt;b &amp; c</b></p>"));
        roxmltree::Document::parse(&out).unwrap();
        // stale offsets
        let changed = src.replace("the", "that");
        assert!(apply_replacement(&changed, &ms[1], "world").is_err());
        assert!(apply_replacement("<p>x</p>", &ms[1], "world").is_err());
        assert_eq!(
            apply_replacement(src, &ms[0], "").unwrap_err().to_string(),
            "empty replacement"
        );
    }

    #[test]
    fn replace_all_and_in_book() {
        let (out, n) =
            replace_all_in_source("<p>tehe, tehe <i>tehe</i> and tehex</p>", "tehe", "the")
                .unwrap();
        assert_eq!(n, 3);
        assert_eq!(out, "<p>the, the <i>the</i> and tehex</p>");
        assert!(replace_all_in_source("<p>x</p>", "x", "").is_err());
        let mut book = make_book(&["<p>tehe tehe</p>"]);
        assert_eq!(
            replace_in_book(&mut book, "OEBPS/chapter-1.xhtml", "tehe", "the").unwrap(),
            2
        );
        assert!(
            book.archive
                .get_string("OEBPS/chapter-1.xhtml")
                .unwrap()
                .contains("<p>the the</p>")
        );
        assert_eq!(
            replace_in_book(&mut book, "OEBPS/chapter-1.xhtml", "zzz", "the").unwrap(),
            0
        );
        assert!(replace_in_book(&mut book, "nope", "a", "b").is_err());
    }
}
