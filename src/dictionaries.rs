//! Offline dictionary lookup for the reader (roadmap R8/R38), on top of `bookx-lookup`.
//!
//! Dictionaries are StarDict folders (a directory containing `*.ifo`, `*.idx`, `*.dict[.dz]`) kept in
//! `~/.local/share/bookx/dictionaries/<name>/`, or in the folder named by `$BOOKX_DICTIONARIES`.
//! No GTK in here, so everything is unit tested.

use crate::i18n::trf;
use std::path::{Path, PathBuf};

use bookx_lookup::dictset::{DictionarySet, Lookup};
use bookx_lookup::stardict::StarDict;

/// Where dictionaries are looked for.
pub fn dictionary_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("BOOKX_DICTIONARIES") {
        return PathBuf::from(d);
    }
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.data_dir().join("dictionaries"))
        .unwrap_or_else(|| PathBuf::from("dictionaries"))
}

/// The dictionaries found in a folder plus the ones that could not be opened.
pub struct Loaded {
    pub set: DictionarySet,
    /// `(folder, error)`
    pub failed: Vec<(String, String)>,
}

/// Opens every StarDict dictionary below `root` (`root` itself, its sub-folders and their
/// sub-folders). Order is alphabetical, so priority is predictable.
pub fn load(root: &Path) -> Loaded {
    // alphabetical, then the order chosen in Preferences → Dictionaries (`priority.txt`)
    let dirs = crate::lookup_prefs::apply_order(dictionary_dirs(root), root);
    let mut set = DictionarySet::new();
    let mut failed = Vec::new();
    for d in dirs {
        match StarDict::open(&d) {
            Ok(dict) => {
                set.add(dict);
            }
            Err(e) => failed.push((d.display().to_string(), format!("{e:#}"))),
        }
    }
    Loaded { set, failed }
}

/// The folders below `root` that hold a StarDict dictionary, in alphabetical order. Folders whose name
/// starts with a dot (downloads in progress) are not looked into.
pub fn dictionary_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    find_dictionaries(root, 0, &mut dirs);
    dirs.sort();
    dirs
}

fn find_dictionaries(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut has_ifo = false;
    let mut subs = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
            {
                continue;
            }
            subs.push(p);
        } else if p.extension().is_some_and(|x| x == "ifo") {
            has_ifo = true;
        }
    }
    if has_ifo {
        out.push(dir.to_path_buf());
    }
    if depth < 2 {
        for s in subs {
            find_dictionaries(&s, depth + 1, out);
        }
    }
}

/// What to look up for a selection: a phrase of up to two words as is, otherwise the first word.
/// Punctuation around the words is dropped. `None` when nothing word-like is selected.
pub fn term_for_selection(selection: &str) -> Option<String> {
    let words: Vec<String> = selection
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’')
                .to_string()
        })
        .filter(|w| !w.is_empty())
        .collect();
    match words.len() {
        0 => None,
        1 | 2 => Some(words.join(" ")),
        _ => Some(words[0].clone()),
    }
}

/// Longest article shown in the popover, in characters.
const MAX_ARTICLE_CHARS: usize = 900;

/// Pango markup for the lookup result.
pub fn render(term: &str, lookup: &Lookup) -> String {
    use std::fmt::Write;
    let esc = |s: &str| relm4::gtk::glib::markup_escape_text(s).to_string();
    let mut out = String::new();
    if lookup.results.is_empty() {
        out.push_str(&trf("No definition found for <b>{}</b>", &[&esc(term)]));
    }
    for r in &lookup.results {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        let _ = write!(out, "<small>{}</small>", esc(&r.dictionary));
        for e in &r.entries {
            let text = e.plain_text();
            let text = truncate(&text, MAX_ARTICLE_CHARS);
            let _ = write!(out, "\n<b>{}</b>\n{}", esc(&e.word), esc(&text));
        }
    }
    for (name, err) in &lookup.errors {
        let _ = write!(
            out,
            "\n\n<small>{} could not be read: {}</small>",
            esc(name),
            esc(err)
        );
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

/// Text shown when no dictionary is installed.
pub fn empty_help(dir: &Path) -> String {
    trf("No dictionaries installed.\n\nCopy StarDict dictionaries (folders with .ifo, .idx and .dict files) into:\n{}", &[&dir.display()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "bookx-dict-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// Writes a tiny StarDict dictionary (sorted words) into `dir`.
    fn write_dict(dir: &Path, name: &str, entries: &[(&str, &str)]) {
        fs::create_dir_all(dir).unwrap();
        let mut idx = Vec::new();
        let mut dict = Vec::new();
        for (w, d) in entries {
            idx.extend_from_slice(w.as_bytes());
            idx.push(0);
            idx.extend_from_slice(&(dict.len() as u32).to_be_bytes());
            idx.extend_from_slice(&(d.len() as u32).to_be_bytes());
            dict.extend_from_slice(d.as_bytes());
        }
        fs::write(
            dir.join("d.ifo"),
            format!(
                "StarDict's dict ifo file\nversion=2.4.2\nwordcount={}\nidxfilesize={}\nbookname={name}\nsametypesequence=m\n",
                entries.len(),
                idx.len()
            ),
        )
        .unwrap();
        fs::write(dir.join("d.idx"), idx).unwrap();
        fs::write(dir.join("d.dict"), dict).unwrap();
    }

    #[test]
    fn selection_terms() {
        assert_eq!(term_for_selection("word"), Some("word".into()));
        assert_eq!(term_for_selection("  “Hello,” "), Some("Hello".into()));
        assert_eq!(term_for_selection("ice cream"), Some("ice cream".into()));
        assert_eq!(term_for_selection("one two three four"), Some("one".into()));
        assert_eq!(term_for_selection("don’t"), Some("don’t".into()));
        assert_eq!(term_for_selection("... !!!"), None);
        assert_eq!(term_for_selection(""), None);
        assert_eq!(term_for_selection("\n\t "), None);
    }

    #[test]
    fn finds_dictionaries_in_nested_folders_in_order() {
        let root = tmp("nested");
        write_dict(&root.join("b-dict"), "Second", &[("beta", "b")]);
        write_dict(&root.join("a-dict"), "First", &[("alpha", "a")]);
        write_dict(&root.join("deep/er/c-dict"), "TooDeep", &[("gamma", "g")]);
        write_dict(&root.join("deep/c-dict"), "Third", &[("gamma", "g")]);
        let loaded = load(&root);
        assert!(loaded.failed.is_empty(), "{:?}", loaded.failed);
        assert_eq!(loaded.set.names(), vec!["First", "Second", "Third"]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_saved_priority_decides_which_dictionary_answers_first() {
        let root = tmp("priority");
        write_dict(&root.join("a-dict"), "First", &[("word", "from a")]);
        write_dict(&root.join("b-dict"), "Second", &[("word", "from b")]);
        let first_answer = |r: &Path| {
            let l = load(r);
            (
                l.set.names(),
                l.set.lookup_all("word").results[0].dictionary.clone(),
            )
        };
        assert_eq!(first_answer(&root).1, "First");
        crate::lookup_prefs::move_dictionary(&root, "b-dict", -1).unwrap();
        let (names, answer) = first_answer(&root);
        assert_eq!(names, vec!["Second", "First"]);
        assert_eq!(answer, "Second", "the dictionary moved up answers first");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn an_empty_or_missing_folder_has_no_dictionaries() {
        let root = tmp("empty");
        assert!(load(&root).set.names().is_empty());
        assert!(load(&root.join("nope")).set.names().is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_broken_dictionary_is_reported_and_the_others_still_load() {
        let root = tmp("broken");
        write_dict(&root.join("good"), "Good", &[("word", "meaning")]);
        fs::create_dir_all(root.join("bad")).unwrap();
        fs::write(root.join("bad/x.ifo"), "garbage").unwrap();
        let loaded = load(&root);
        assert_eq!(loaded.set.names(), vec!["Good"]);
        assert_eq!(loaded.failed.len(), 1);
        assert!(loaded.failed[0].0.ends_with("bad"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn lookup_and_render_an_article() {
        let root = tmp("render");
        write_dict(
            &root.join("d"),
            "Mini <Dict>",
            &[("library", "a place with books & shelves")],
        );
        let loaded = load(&root);
        let r = loaded.set.lookup_all("libraries"); // word forms: libraries -> library
        let text = render("libraries", &r);
        assert!(text.contains("Mini &lt;Dict&gt;"), "{text}");
        assert!(text.contains("<b>library</b>"), "{text}");
        assert!(text.contains("books &amp; shelves"), "{text}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn render_reports_a_missing_word() {
        let t = render("zzz<", &Lookup::default());
        assert_eq!(t, "No definition found for <b>zzz&lt;</b>");
    }

    #[test]
    fn long_articles_are_cut() {
        let root = tmp("long");
        let long = "word ".repeat(1000);
        write_dict(&root.join("d"), "Big", &[("big", &long)]);
        let loaded = load(&root);
        let text = render("big", &loaded.set.lookup_all("big"));
        assert!(text.contains('…'));
        assert!(text.chars().count() < 1200, "{}", text.chars().count());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn truncate_keeps_short_text_and_trims() {
        assert_eq!(truncate("  short ", 10), "short");
        assert_eq!(truncate("abcdef", 3), "abc…");
        assert_eq!(truncate("héllo wörld", 5), "héllo…");
    }

    #[test]
    fn help_names_the_folder() {
        let t = empty_help(Path::new("/x/dictionaries"));
        assert!(t.contains("/x/dictionaries") && t.contains("StarDict"));
    }

    #[test]
    fn the_folder_can_be_overridden_by_the_environment() {
        // not set here: falls back to a data folder that ends with `dictionaries`
        if std::env::var_os("BOOKX_DICTIONARIES").is_none() {
            assert!(dictionary_dir().ends_with("dictionaries"));
        }
    }
}
