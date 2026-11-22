//! The book-wide side of the interactive spell checker (CE-270..CE-278): every word of the book with its language,
//! count and first places, which words are misspelled, "change everywhere", and the files that are left out
//! (by the user or by a `<!-- bookx-no-spell-check -->` comment).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{Result, bail};
use bookx_core::book::OpenBook;

use crate::markup::{Event, scan, tag_attr};
use crate::reports::{book_text_files, document_language};
use crate::spell::{SpellChecker, extract_words, lang_key, replace_all_in_source};

/// Comments that switch the spell check off for the file that contains them. The second one is accepted so books
/// prepared with Calibre keep working.
pub const NO_SPELL_MARKERS: &[&str] = &["bookx-no-spell-check", "calibre-no-spell-check"];

/// Whether `src` contains a `<!-- bookx-no-spell-check -->` style comment.
pub fn has_no_spell_marker(src: &str) -> bool {
    scan(src).into_iter().any(|e| match e {
        Event::Other(r) => {
            let t = &src[r];
            t.starts_with("<!--") && NO_SPELL_MARKERS.iter().any(|m| t.contains(m))
        }
        _ => false,
    })
}

/// Where a word occurs: an index into [`BookWords::files`] and a byte range of that file's source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Occurrence {
    pub file: usize,
    pub start: usize,
    pub end: usize,
}

/// One distinct word (as written) in one language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordEntry {
    pub word: String,
    /// Normalised language tag the word was checked in (`en-gb`).
    pub lang: String,
    pub count: usize,
    pub misspelled: bool,
    /// `false` when no dictionary for `lang` is loaded (the word was not checked).
    pub checked: bool,
}

#[derive(Debug, Clone, Default)]
pub struct BookWords {
    /// The files that were checked, in reading order.
    pub files: Vec<String>,
    /// Files that were left out, with the reason.
    pub skipped: Vec<(String, SkipReason)>,
    pub entries: Vec<WordEntry>,
    occurrences: HashMap<(String, String), Vec<Occurrence>>,
    /// Languages found with no dictionary loaded.
    pub unavailable: BTreeSet<String>,
    pub words_checked: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// Excluded by the user.
    Excluded,
    /// The file carries a `bookx-no-spell-check` comment.
    Marker,
}

impl BookWords {
    pub fn misspelled(&self) -> impl Iterator<Item = &WordEntry> {
        self.entries.iter().filter(|e| e.misspelled)
    }

    pub fn occurrences(&self, e: &WordEntry) -> &[Occurrence] {
        self.occurrences
            .get(&(e.word.clone(), e.lang.clone()))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// The next occurrence of `e` after `from` (file path and byte offset; the caret), wrapping to the first.
    pub fn next_occurrence(
        &self,
        e: &WordEntry,
        from: Option<(&str, usize)>,
    ) -> Option<Occurrence> {
        let occ = self.occurrences(e);
        let first = *occ.first()?;
        let Some((path, at)) = from else {
            return Some(first);
        };
        let Some(fi) = self.files.iter().position(|f| f == path) else {
            return Some(first);
        };
        occ.iter()
            .find(|o| (o.file, o.start) > (fi, at))
            .copied()
            .or(Some(first))
    }

    pub fn file_of(&self, o: &Occurrence) -> &str {
        &self.files[o.file]
    }
}

/// Every language tag the text mentions: the package language, the `lang` of each document and of each
/// element. Tags are normalised (`en-gb`). Excluded files are ignored.
pub fn languages_in_book(book: &OpenBook, excluded: &HashSet<String>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let pkg = lang_key(&book.package.metadata.language);
    if !pkg.is_empty() {
        out.insert(pkg);
    }
    for f in book_text_files(book) {
        if excluded.contains(&f) {
            continue;
        }
        let Some(src) = book.archive.get_string(&f) else {
            continue;
        };
        out.extend(languages_in_source(&src));
    }
    out
}

/// The `lang` / `xml:lang` values in one document (normalised), none when the file carries the no-spell-check
/// comment. Cheap: only start tags are looked at.
pub fn languages_in_source(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if !src.contains("lang") || has_no_spell_marker(src) {
        return out;
    }
    for ev in scan(src) {
        if let Event::Open { range, .. } = ev {
            let tag = &src[range];
            if let Some(l) = tag_attr(tag, "xml:lang").or_else(|| tag_attr(tag, "lang")) {
                let k = lang_key(&l);
                if !k.is_empty() {
                    out.insert(k);
                }
            }
        }
    }
    out
}

/// Loads a dictionary for each of `langs` with `find` (which returns the `.aff` and `.dic` of a tag). Returns the
/// tags that loaded and the tags without a usable dictionary (with the reason).
pub fn load_languages(
    checker: &mut SpellChecker,
    langs: &BTreeSet<String>,
    find: &dyn Fn(&str) -> Option<(PathBuf, PathBuf)>,
) -> (Vec<String>, Vec<(String, String)>) {
    let mut loaded = Vec::new();
    let mut missing = Vec::new();
    for l in langs {
        match find(l) {
            Some((aff, dic)) => match checker.add_dictionary_files(l, &aff, &dic) {
                Ok(()) => loaded.push(l.clone()),
                Err(e) => missing.push((l.clone(), format!("{e:#}"))),
            },
            None => missing.push((l.clone(), "no dictionary installed".to_string())),
        }
    }
    (loaded, missing)
}

/// The text files of the book (reading order) with their sources, and the default language: what [`scan_sources`]
/// needs. Owned, so a scan can run on another thread.
pub fn collect_sources(book: &OpenBook) -> (String, Vec<(String, String)>) {
    let default_lang = lang_key(&book.package.metadata.language);
    let default_lang = if default_lang.is_empty() {
        "en".to_string()
    } else {
        default_lang
    };
    let files = book_text_files(book)
        .into_iter()
        .filter_map(|p| book.archive.get_string(&p).map(|t| (p, t)))
        .collect();
    (default_lang, files)
}

/// Reads every word of the book. A word's language is its own `lang`, else the document's, else the book's.
pub fn scan_book(book: &OpenBook, checker: &SpellChecker, excluded: &HashSet<String>) -> BookWords {
    let (lang, files) = collect_sources(book);
    scan_sources(&lang, &files, checker, excluded)
}

pub fn scan_sources(
    default_lang: &str,
    sources: &[(String, String)],
    checker: &SpellChecker,
    excluded: &HashSet<String>,
) -> BookWords {
    let mut out = BookWords::default();
    let mut index: HashMap<(String, String), usize> = HashMap::new();
    let mut status: HashMap<(String, String), Option<bool>> = HashMap::new();
    for (path, src) in sources {
        if excluded.contains(path) {
            out.skipped.push((path.clone(), SkipReason::Excluded));
            continue;
        }
        if has_no_spell_marker(src) {
            out.skipped.push((path.clone(), SkipReason::Marker));
            continue;
        }
        let doc_lang = document_language(src).map(|l| lang_key(&l));
        let fi = out.files.len();
        out.files.push(path.clone());
        for w in extract_words(src) {
            let lang = w
                .lang
                .as_deref()
                .map(lang_key)
                .or_else(|| doc_lang.clone())
                .unwrap_or_else(|| default_lang.to_string());
            let key = (w.word.clone(), lang.clone());
            let ok = *status
                .entry(key.clone())
                .or_insert_with(|| checker.check_word(&lang, &w.word));
            let i = *index.entry(key.clone()).or_insert_with(|| {
                out.entries.push(WordEntry {
                    word: w.word.clone(),
                    lang: lang.clone(),
                    count: 0,
                    misspelled: ok == Some(false),
                    checked: ok.is_some(),
                });
                if ok.is_none() {
                    out.unavailable.insert(lang.clone());
                }
                out.entries.len() - 1
            });
            out.entries[i].count += 1;
            if ok.is_some() {
                out.words_checked += 1;
            }
            out.occurrences.entry(key).or_default().push(Occurrence {
                file: fi,
                start: w.start,
                end: w.end,
            });
        }
    }
    out
}

/// Display order: by count (most first) or alphabetically, ignoring case; ties by word, then language.
pub fn compare_entries(a: &WordEntry, b: &WordEntry, by_count: bool) -> std::cmp::Ordering {
    let alpha = a
        .word
        .to_lowercase()
        .cmp(&b.word.to_lowercase())
        .then(a.lang.cmp(&b.lang));
    if by_count {
        b.count.cmp(&a.count).then(alpha)
    } else {
        alpha
    }
}

/// Sorts entries for display, see [`compare_entries`].
pub fn sort_entries(entries: &mut [WordEntry], by_count: bool) {
    entries.sort_by(|a, b| compare_entries(a, b, by_count));
}

/// Changes every occurrence of `word` in the text files of the book (except `excluded` ones) to `replacement`.
/// Returns the number of files changed and the number of replacements. The caller takes a checkpoint first.
pub fn change_everywhere(
    book: &mut OpenBook,
    word: &str,
    replacement: &str,
    excluded: &HashSet<String>,
) -> Result<(usize, usize)> {
    if replacement.trim().is_empty() {
        bail!("The replacement is empty");
    }
    let mut files = 0;
    let mut total = 0;
    for path in book_text_files(book) {
        if excluded.contains(&path) {
            continue;
        }
        let Some(src) = book.archive.get_string(&path) else {
            continue;
        };
        if has_no_spell_marker(&src) {
            continue;
        }
        let (new, n) = replace_all_in_source(&src, word, replacement)?;
        if n > 0 {
            book.archive.set_string(&path, &new);
            files += 1;
            total += n;
        }
    }
    Ok((files, total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spell::tests::{AFF, DIC, checker};
    use crate::testutil::make_book;

    fn book_with(bodies: &[&str]) -> OpenBook {
        make_book(bodies)
    }

    #[test]
    fn marker_comment_is_found_only_in_comments() {
        assert!(has_no_spell_marker(
            "<html><!-- bookx-no-spell-check --><p>x</p>"
        ));
        assert!(has_no_spell_marker("<!--calibre-no-spell-check--><p>x</p>"));
        assert!(!has_no_spell_marker("<p>bookx-no-spell-check</p>"));
        assert!(!has_no_spell_marker("<!-- something else --><p>x</p>"));
    }

    #[test]
    fn words_get_counts_languages_and_places() {
        let b = book_with(&[
            "<p>The quikc fox and the quikc dog</p><p lang=\"de\">Der Hund</p><p>The <code>zzz</code></p>",
        ]);
        let c = checker();
        let w = scan_book(&b, &c, &HashSet::new());
        let q = w.entries.iter().find(|e| e.word == "quikc").unwrap();
        assert_eq!((q.count, q.misspelled, q.checked), (2, true, true));
        assert_eq!(w.occurrences(q).len(), 2);
        let the: Vec<_> = w
            .entries
            .iter()
            .filter(|e| e.word.eq_ignore_ascii_case("the"))
            .collect();
        assert_eq!(the.len(), 2, "The and the are separate words as written");
        assert!(!the.iter().any(|e| e.misspelled));
        let der = w.entries.iter().find(|e| e.word == "Der").unwrap();
        assert_eq!(der.lang, "de");
        assert!(!der.checked, "no German dictionary is loaded");
        assert!(w.unavailable.contains("de"));
        assert!(w.entries.iter().all(|e| e.word != "zzz"), "code is skipped");
        assert_eq!(
            w.misspelled().count(),
            w.entries.iter().filter(|e| e.misspelled).count()
        );
        let src = b
            .archive
            .get_string(w.file_of(&w.occurrences(q)[0]))
            .unwrap();
        let o = w.occurrences(q)[0];
        assert_eq!(&src[o.start..o.end], "quikc");
    }

    #[test]
    fn two_english_dictionaries_check_each_variant_on_its_own() {
        let mut c = SpellChecker::new();
        c.add_dictionary("en-US", AFF, DIC).unwrap();
        c.add_dictionary(
            "en-GB",
            AFF,
            &DIC.replace(
                "12
",
                "13
colour
",
            ),
        )
        .unwrap();
        let b = book_with(&[
            "<p lang=\"en-GB\">The colour fox</p><p lang=\"en-US\">The colour fox</p><p>The colour</p>",
        ]);
        let w = scan_book(&b, &c, &HashSet::new());
        let gb = w
            .entries
            .iter()
            .find(|e| e.word == "colour" && e.lang == "en-gb")
            .unwrap();
        let us = w
            .entries
            .iter()
            .find(|e| e.word == "colour" && e.lang == "en-us")
            .unwrap();
        assert!(!gb.misspelled, "en-GB knows colour");
        assert!(us.misspelled, "en-US does not");
        let plain = w
            .entries
            .iter()
            .find(|e| e.word == "colour" && e.lang == "en")
            .unwrap();
        assert!(
            plain.misspelled,
            "the bare book language uses the en-US dictionary (the English default)"
        );
        assert_eq!(
            c.suggest("en-GB", "colur").first().map(String::as_str),
            Some("colour")
        );
    }

    #[test]
    fn languages_are_collected_from_elements() {
        let b = book_with(&["<p lang=\"fr-CA\">Bonjour</p><p xml:lang=\"de\">Hallo</p>"]);
        let l = languages_in_book(&b, &HashSet::new());
        assert!(l.contains("en") && l.contains("fr-ca") && l.contains("de"));
        let spine = b.spine_paths();
        let none = languages_in_book(&b, &spine.iter().cloned().collect());
        assert!(!none.contains("de"), "excluded files do not count");
    }

    #[test]
    fn excluded_and_marked_files_are_skipped() {
        let mut b = book_with(&["<p>one quikc</p>", "<p>two wrods</p>"]);
        let spine = b.spine_paths();
        let c = checker();
        let ex: HashSet<String> = [spine[0].clone()].into();
        let w = scan_book(&b, &c, &ex);
        assert!(w.entries.iter().all(|e| e.word != "quikc"));
        assert_eq!(w.skipped, vec![(spine[0].clone(), SkipReason::Excluded)]);
        let src = b
            .archive
            .get_string(&spine[1])
            .unwrap()
            .replace("<body", "<!-- bookx-no-spell-check --><body");
        b.archive.set_string(&spine[1], &src);
        let w = scan_book(&b, &c, &HashSet::new());
        assert!(w.entries.iter().all(|e| e.word != "wrods"));
        assert!(w.skipped.contains(&(spine[1].clone(), SkipReason::Marker)));
    }

    #[test]
    fn next_occurrence_walks_files_and_wraps() {
        let b = book_with(&["<p>quikc a quikc</p>", "<p>quikc</p>"]);
        let c = checker();
        let w = scan_book(&b, &c, &HashSet::new());
        let e = w.entries.iter().find(|e| e.word == "quikc").unwrap();
        let first = w.next_occurrence(e, None).unwrap();
        assert_eq!(w.file_of(&first), b.spine_paths()[0]);
        let second = w
            .next_occurrence(e, Some((w.file_of(&first), first.start)))
            .unwrap();
        assert!(second.start > first.start && second.file == first.file);
        let third = w
            .next_occurrence(e, Some((w.file_of(&second), second.start)))
            .unwrap();
        assert_eq!(w.file_of(&third), b.spine_paths()[1]);
        let wrapped = w
            .next_occurrence(e, Some((w.file_of(&third), third.start)))
            .unwrap();
        assert_eq!(wrapped, first);
        assert_eq!(
            w.next_occurrence(e, Some(("nowhere.xhtml", 0))),
            Some(first)
        );
    }

    #[test]
    fn change_everywhere_edits_all_files_and_escapes() {
        let mut b = book_with(&["<p>quikc and quikc</p>", "<p>quikc</p><code>quikc</code>"]);
        let (files, n) = change_everywhere(&mut b, "quikc", "quick", &HashSet::new()).unwrap();
        assert_eq!((files, n), (2, 3));
        let spine = b.spine_paths();
        let s1 = b.archive.get_string(&spine[1]).unwrap();
        assert!(
            s1.contains("<p>quick</p><code>quikc</code>"),
            "code is left alone"
        );
        let (_, n) = change_everywhere(&mut b, "quick", "a&b", &HashSet::new()).unwrap();
        assert_eq!(n, 3);
        assert!(
            b.archive
                .get_string(&spine[0])
                .unwrap()
                .contains("a&amp;b and a&amp;b")
        );
        assert!(change_everywhere(&mut b, "a", " ", &HashSet::new()).is_err());
        let ex: HashSet<String> = [spine[0].clone()].into();
        let (files, _) = change_everywhere(&mut b, "and", "or", &ex).unwrap();
        assert_eq!(files, 0, "excluded files are not touched");
    }

    #[test]
    fn sorting_by_count_and_alphabet() {
        let e = |w: &str, n| WordEntry {
            word: w.into(),
            lang: "en".into(),
            count: n,
            misspelled: true,
            checked: true,
        };
        let mut v = vec![e("beta", 1), e("Alpha", 1), e("gamma", 5)];
        sort_entries(&mut v, true);
        assert_eq!(
            v.iter().map(|x| x.word.as_str()).collect::<Vec<_>>(),
            ["gamma", "Alpha", "beta"]
        );
        sort_entries(&mut v, false);
        assert_eq!(
            v.iter().map(|x| x.word.as_str()).collect::<Vec<_>>(),
            ["Alpha", "beta", "gamma"]
        );
    }
}
