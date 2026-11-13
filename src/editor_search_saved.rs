//! Saved searches of the editor (CE-084): a named find/replace with its mode, scope and options, kept as JSON in
//! the config folder, importable and exportable, runnable one by one or several in sequence. The running
//! logic works on texts, so it is tested without GTK or a book.

use crate::i18n::tr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::editor_search::{self, Scope};
use crate::editor_search_fn::{order_files, Env, Function, Runner};
use crate::editor_search_modes::{Engine, Mode, RunCtx, Settings};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSearch {
    pub name: String,
    #[serde(flatten)]
    pub settings: Settings,
    #[serde(default = "this_file")]
    pub scope: Scope,
    /// Function mode: the name of the saved function to call.
    #[serde(default)]
    pub function: String,
}

fn this_file() -> Scope {
    Scope::ThisFile
}

impl SavedSearch {
    pub fn new(name: &str, settings: Settings, scope: Scope) -> SavedSearch {
        SavedSearch {
            name: name.into(),
            settings,
            scope,
            function: String::new(),
        }
    }

    /// A one-line description for the list.
    pub fn summary(&self) -> String {
        let mode = self.settings.mode.label();
        let extra = if self.settings.ignore_markup {
            ", ignoring tags"
        } else {
            ""
        };
        format!(
            "{}{extra} · {} → {} · {}",
            mode,
            clip(&self.settings.find),
            if self.settings.mode == Mode::Function {
                format!("function {}", self.function)
            } else {
                clip(&self.settings.replace)
            },
            self.scope.label()
        )
    }
}

fn clip(s: &str) -> String {
    let one: String = s.replace('\n', "⏎");
    if one.chars().count() > 32 {
        format!("{}…", one.chars().take(31).collect::<String>())
    } else {
        one
    }
}

/// `<config>/Bookx/saved-searches.json`.
pub fn path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.config_dir().join("saved-searches.json"))
}

pub fn parse_json(text: &str) -> Result<Vec<SavedSearch>, String> {
    serde_json::from_str(text).map_err(|e| e.to_string())
}

pub fn to_json(list: &[SavedSearch]) -> String {
    serde_json::to_string_pretty(list).unwrap_or_default()
}

pub fn load_from(path: &Path) -> Vec<SavedSearch> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| parse_json(&t).ok())
        .unwrap_or_default()
}

pub fn save_to(path: &Path, list: &[SavedSearch]) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, to_json(list))
}

/// A name that is not used yet: `Name`, `Name 2`, `Name 3` …
pub fn unique_name(list: &[SavedSearch], wanted: &str) -> String {
    let base = if wanted.trim().is_empty() {
        "Search"
    } else {
        wanted.trim()
    };
    if !list.iter().any(|s| s.name == base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|n| !list.iter().any(|s| &s.name == n))
        .unwrap_or_default()
}

/// Adds `s` (a same-named search is replaced in place); returns its index.
pub fn upsert(list: &mut Vec<SavedSearch>, s: SavedSearch) -> usize {
    match list.iter().position(|x| x.name == s.name) {
        Some(i) => {
            list[i] = s;
            i
        }
        None => {
            list.push(s);
            list.len() - 1
        }
    }
}

/// Appends imported searches, renaming those whose name is taken.
pub fn merge_imported(list: &mut Vec<SavedSearch>, imported: Vec<SavedSearch>) -> usize {
    let n = imported.len();
    for mut s in imported {
        s.name = unique_name(list, &s.name);
        list.push(s);
    }
    n
}

/// Moves item `i` one step up (`-1`) or down (`1`); returns its new index.
pub fn move_item(list: &mut [SavedSearch], i: usize, step: i32) -> usize {
    let j = i as i64 + step as i64;
    if i >= list.len() || j < 0 || j as usize >= list.len() {
        return i;
    }
    list.swap(i, j as usize);
    j as usize
}

// ---------------------------------------------------------------------------
// History of the Find and Replace boxes (CE-079) and special characters (CE-081)
// ---------------------------------------------------------------------------

pub const HISTORY_LEN: usize = 20;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct History {
    #[serde(default)]
    pub find: Vec<String>,
    #[serde(default)]
    pub replace: Vec<String>,
}

/// Puts `entry` first; an equal older entry is dropped; at most [`HISTORY_LEN`] are kept. Empty text is ignored.
pub fn remember(list: &mut Vec<String>, entry: &str) {
    if entry.is_empty() {
        return;
    }
    list.retain(|e| e != entry);
    list.insert(0, entry.to_string());
    list.truncate(HISTORY_LEN);
}

/// `<config>/Bookx/search-history.json`.
pub fn history_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.config_dir().join("search-history.json"))
}

pub fn load_history(path: &Path) -> History {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_history(path: &Path, h: &History) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, serde_json::to_string(h).unwrap_or_default())
}

/// Characters and escapes the search fields offer: `(label, text to insert, only useful in regex mode)`.
pub const SPECIALS: &[(&str, &str, bool)] = &[
    ("No-break space", "\u{a0}", false),
    ("Em dash —", "\u{2014}", false),
    ("En dash –", "\u{2013}", false),
    ("Ellipsis …", "\u{2026}", false),
    ("Soft hyphen", "\u{ad}", false),
    ("Left quote “", "\u{201c}", false),
    ("Right quote ”", "\u{201d}", false),
    ("Line break (\\n)", "\\n", true),
    ("Tab (\\t)", "\\t", true),
    ("Any space (\\s)", "\\s", true),
    ("Any digit (\\d)", "\\d", true),
    ("Word character (\\w)", "\\w", true),
    ("Word boundary (\\b)", "\\b", true),
];

/// What a special entry inserts in the current mode: in regex mode an invisible character is written as
/// `\x{..}` so it stays visible in the box; otherwise the character (or escape) itself.
pub fn special_text(insert: &str, regex_mode: bool) -> String {
    let mut it = insert.chars();
    match (it.next(), it.next()) {
        (Some(c), None) if regex_mode && (c.is_whitespace() || c == '\u{ad}') => {
            format!("\\x{{{:x}}}", c as u32)
        }
        _ => insert.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Count the matches, change nothing.
    Count,
    ReplaceAll,
}

/// What one search of a run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StepReport {
    pub name: String,
    pub matches: usize,
    pub files_with_matches: usize,
    pub replaced: usize,
    pub files_changed: usize,
    pub error: Option<String>,
    pub output: Vec<String>,
    pub final_output: Option<String>,
}

/// The book as far as a run needs it.
pub struct Book<'a> {
    /// Every text file with its current text, in the book's file order. Replace All edits these.
    pub texts: &'a mut Vec<(String, String)>,
    pub current: Option<&'a str>,
    pub selected: &'a [String],
    pub open: &'a [String],
    pub spine: &'a [String],
}

/// Runs `searches` one after the other on the texts of `book` (each search sees the result of the one before).
/// A search with an error is reported and skipped; the others still run.
pub fn run_sequence(
    searches: &[SavedSearch],
    action: Action,
    book: &mut Book,
    functions: &[Function],
    env: &Env,
) -> Vec<StepReport> {
    searches
        .iter()
        .map(|s| run_one(s, action, book, functions, env))
        .collect()
}

pub fn run_one(
    s: &SavedSearch,
    action: Action,
    book: &mut Book,
    functions: &[Function],
    env: &Env,
) -> StepReport {
    let mut rep = StepReport {
        name: s.name.clone(),
        ..Default::default()
    };
    if s.scope == Scope::MarkedText {
        rep.error = Some(tr("The scope “Marked text” can't be run from the list"));
        return rep;
    }
    let engine = match Engine::new(&s.settings) {
        Ok(e) => e,
        Err(e) => {
            rep.error = Some(e);
            return rep;
        }
    };
    let mut runner = if engine.is_function() && action == Action::ReplaceAll {
        let Some(f) = functions.iter().find(|f| f.name == s.function) else {
            rep.error = Some(format!("The function “{}” does not exist", s.function));
            return rep;
        };
        match Runner::new(f, env) {
            Ok(r) => Some(r),
            Err(e) => {
                rep.error = Some(e.to_string());
                return rep;
            }
        }
    } else {
        None
    };
    let all: Vec<String> = book.texts.iter().map(|(p, _)| p.clone()).collect();
    let mut files =
        editor_search::scope_files(s.scope, book.current, &all, book.selected, book.open);
    if let Some(r) = &runner {
        files = order_files(&files, book.spine, r.func.file_order);
    }
    for path in files {
        let Some(slot) = book.texts.iter_mut().find(|(p, _)| *p == path) else {
            continue;
        };
        let hits = engine.find_all(&slot.1, None).len();
        if hits > 0 {
            rep.matches += hits;
            rep.files_with_matches += 1;
        }
        if action == Action::Count || hits == 0 {
            continue;
        }
        let mut ctx = RunCtx {
            runner: runner.as_mut(),
            file: &path,
        };
        match engine.replace_all(&slot.1, &s.settings.replace, None, &mut ctx) {
            Ok((new, n)) => {
                if n > 0 && new != slot.1 {
                    slot.1 = new;
                    rep.replaced += n;
                    rep.files_changed += 1;
                } else {
                    rep.replaced += n;
                }
            }
            Err(e) => {
                rep.error = Some(format!("{}: {e}", editor_search_file_name(&path)));
                break;
            }
        }
    }
    if let Some(r) = runner.as_mut() {
        if rep.error.is_none() {
            match r.finish("") {
                Ok(f) => rep.final_output = f,
                Err(e) => rep.error = Some(e.to_string()),
            }
        }
        rep.output = r.output();
    }
    rep
}

fn editor_search_file_name(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(find: &str, rep: &str, mode: Mode) -> Settings {
        Settings {
            find: find.into(),
            replace: rep.into(),
            mode,
            case_sensitive: true,
            ..Default::default()
        }
    }

    fn texts() -> Vec<(String, String)> {
        vec![
            ("OEBPS/c1.xhtml".into(), "<p>one one</p>".into()),
            ("OEBPS/c2.xhtml".into(), "<p>two one</p>".into()),
            ("OEBPS/s.css".into(), "p { color: one }".into()),
        ]
    }

    fn book<'a>(t: &'a mut Vec<(String, String)>, spine: &'a [String]) -> Book<'a> {
        Book {
            texts: t,
            current: Some("OEBPS/c1.xhtml"),
            selected: &[],
            open: &[],
            spine,
        }
    }

    #[test]
    fn json_round_trip_keeps_every_field() {
        let mut s = SavedSearch::new("Dashes", st("--", "–", Mode::Normal), Scope::TextFiles);
        s.settings.whole_word = true;
        s.settings.ignore_markup = true;
        s.settings.dot_all = true;
        let mut f = SavedSearch::new("Fn", st("x", "", Mode::Function), Scope::AllFiles);
        f.function = "Mine".into();
        let json = to_json(&[s.clone(), f.clone()]);
        assert!(json.contains("\"scope\": \"text_files\""));
        assert!(json.contains("\"mode\": \"function\""));
        let back = parse_json(&json).unwrap();
        assert_eq!(back, vec![s, f]);
        assert!(parse_json("[{\"nonsense\":1}]").is_err());
    }

    #[test]
    fn older_files_with_missing_fields_still_load() {
        let v = parse_json("[{\"name\":\"a\",\"find\":\"x\"}]").unwrap();
        assert_eq!(v[0].scope, Scope::ThisFile);
        assert_eq!(v[0].settings.mode, Mode::Normal);
    }

    #[test]
    fn file_store_round_trip() {
        let dir = std::env::temp_dir().join(format!("bookx-ss-{}", std::process::id()));
        let f = dir.join("x/saved.json");
        let l = vec![SavedSearch::new(
            "a",
            st("x", "y", Mode::Regex),
            Scope::StyleFiles,
        )];
        save_to(&f, &l).unwrap();
        assert_eq!(load_from(&f), l);
        assert!(load_from(&dir.join("nope.json")).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn list_editing() {
        let mut l = vec![];
        let a = SavedSearch::new("A", st("a", "", Mode::Normal), Scope::ThisFile);
        assert_eq!(upsert(&mut l, a.clone()), 0);
        let mut a2 = a.clone();
        a2.settings.find = "changed".into();
        assert_eq!(upsert(&mut l, a2), 0);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].settings.find, "changed");
        assert_eq!(unique_name(&l, "A"), "A 2");
        assert_eq!(unique_name(&l, "  "), "Search");
        assert_eq!(unique_name(&l, "B"), "B");
        let n = merge_imported(&mut l, vec![a.clone(), a.clone()]);
        assert_eq!(n, 2);
        assert_eq!(
            l.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["A", "A 2", "A 3"]
        );
        assert_eq!(move_item(&mut l, 0, 1), 1);
        assert_eq!(l[1].name, "A");
        assert_eq!(move_item(&mut l, 0, -1), 0);
        assert_eq!(move_item(&mut l, 2, 1), 2);
        assert_eq!(move_item(&mut l, 9, 1), 9);
    }

    #[test]
    fn history_keeps_the_newest_first_without_duplicates() {
        let mut h = vec![];
        for w in ["a", "b", "a", "", "c"] {
            remember(&mut h, w);
        }
        assert_eq!(h, ["c", "a", "b"]);
        for i in 0..30 {
            remember(&mut h, &format!("w{i}"));
        }
        assert_eq!(h.len(), HISTORY_LEN);
        assert_eq!(h[0], "w29");
    }

    #[test]
    fn history_persists() {
        let dir = std::env::temp_dir().join(format!("bookx-hist-{}", std::process::id()));
        let f = dir.join("h.json");
        let h = History {
            find: vec!["x".into()],
            replace: vec!["y".into()],
        };
        save_history(&f, &h).unwrap();
        assert_eq!(load_history(&f), h);
        assert_eq!(load_history(&dir.join("none.json")), History::default());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn special_characters_adapt_to_the_mode() {
        assert_eq!(special_text("\u{2014}", false), "\u{2014}");
        assert_eq!(special_text("\u{2014}", true), "\u{2014}");
        assert_eq!(special_text("\u{a0}", true), "\\x{a0}");
        assert_eq!(special_text("\u{a0}", false), "\u{a0}");
        assert_eq!(special_text("\\n", true), "\\n");
        for (_, ins, _) in SPECIALS {
            assert!(!special_text(ins, true).is_empty());
        }
    }

    #[test]
    fn summary_is_short() {
        let s = SavedSearch::new(
            "A",
            st(&"x".repeat(80), "y\nz", Mode::Regex),
            Scope::AllFiles,
        );
        let t = s.summary();
        assert!(t.contains("Regex") && t.contains("All files") && t.contains("y⏎z"));
        assert!(t.chars().count() < 100);
    }

    #[test]
    fn count_changes_nothing_and_reports_per_scope() {
        let mut t = texts();
        let spine = vec![];
        let s = SavedSearch::new("one", st("one", "1", Mode::Normal), Scope::TextFiles);
        let r = run_one(
            &s,
            Action::Count,
            &mut book(&mut t, &spine),
            &[],
            &Env::default(),
        );
        assert_eq!((r.matches, r.files_with_matches, r.replaced), (3, 2, 0));
        assert_eq!(t, texts());
        let s = SavedSearch::new("one", st("one", "1", Mode::Normal), Scope::AllFiles);
        assert_eq!(
            run_one(
                &s,
                Action::Count,
                &mut book(&mut t, &spine),
                &[],
                &Env::default()
            )
            .matches,
            4
        );
        let s = SavedSearch::new("one", st("one", "1", Mode::Normal), Scope::StyleFiles);
        assert_eq!(
            run_one(
                &s,
                Action::Count,
                &mut book(&mut t, &spine),
                &[],
                &Env::default()
            )
            .matches,
            1
        );
        let s = SavedSearch::new("one", st("one", "1", Mode::Normal), Scope::ThisFile);
        assert_eq!(
            run_one(
                &s,
                Action::Count,
                &mut book(&mut t, &spine),
                &[],
                &Env::default()
            )
            .matches,
            2
        );
    }

    #[test]
    fn replace_all_in_scope() {
        let mut t = texts();
        let spine = vec![];
        let s = SavedSearch::new("one", st("one", "1", Mode::Normal), Scope::TextFiles);
        let r = run_one(
            &s,
            Action::ReplaceAll,
            &mut book(&mut t, &spine),
            &[],
            &Env::default(),
        );
        assert_eq!((r.replaced, r.files_changed, r.error), (3, 2, None));
        assert_eq!(t[0].1, "<p>1 1</p>");
        assert_eq!(t[1].1, "<p>two 1</p>");
        assert_eq!(t[2].1, "p { color: one }");
    }

    #[test]
    fn several_searches_run_in_order_and_see_each_other() {
        let mut t = texts();
        let spine = vec![];
        let list = vec![
            SavedSearch::new("a", st("one", "two", Mode::Normal), Scope::TextFiles),
            SavedSearch::new("b", st("two", "three", Mode::Normal), Scope::TextFiles),
            SavedSearch::new("bad", st("(", "", Mode::Regex), Scope::TextFiles),
            SavedSearch::new("marked", st("x", "", Mode::Normal), Scope::MarkedText),
        ];
        let rs = run_sequence(
            &list,
            Action::ReplaceAll,
            &mut book(&mut t, &spine),
            &[],
            &Env::default(),
        );
        assert_eq!(rs.len(), 4);
        assert_eq!(rs[1].replaced, 4, "the second sees the first one's output");
        assert_eq!(t[0].1, "<p>three three</p>");
        assert!(rs[2]
            .error
            .as_deref()
            .unwrap()
            .contains("regular expression"));
        assert!(rs[3].error.as_deref().unwrap().contains("Marked text"));
    }

    #[test]
    fn markup_and_tag_modes_run_from_the_list() {
        let mut t = vec![(
            "a.xhtml".to_string(),
            "<p>so on <i>a</i> word <img src=\"x\"/></p>".to_string(),
        )];
        let spine = vec![];
        let mut s1 = SavedSearch::new("m", st("on a word", "X", Mode::Normal), Scope::TextFiles);
        s1.settings.ignore_markup = true;
        let s2 = SavedSearch::new(
            "t",
            st("img[!alt]", "alt=none", Mode::Tag),
            Scope::TextFiles,
        );
        let rs = run_sequence(
            &[s1, s2],
            Action::ReplaceAll,
            &mut Book {
                texts: &mut t,
                current: None,
                selected: &[],
                open: &[],
                spine: &spine,
            },
            &[],
            &Env::default(),
        );
        assert_eq!((rs[0].replaced, rs[1].replaced), (1, 1));
        assert_eq!(t[0].1, "<p>so X<i></i> <img src=\"x\" alt=\"none\"/></p>");
    }

    #[test]
    fn function_searches_use_the_named_function_and_report_output() {
        let mut t = texts();
        let spine = vec!["OEBPS/c2.xhtml".to_string(), "OEBPS/c1.xhtml".to_string()];
        let mut f = Function::new(
            "Num",
            "",
            "// @file_order spine\n// @call_after_last_match\nfn replace(m, number, file, meta, data) { if m == () { return \"done \" + (number - 1); } print(file); number }",
        );
        f.name = "Num".into();
        let mut s = SavedSearch::new("fn", st("one", "", Mode::Function), Scope::TextFiles);
        s.function = "Num".into();
        let r = run_one(
            &s,
            Action::ReplaceAll,
            &mut book(&mut t, &spine),
            std::slice::from_ref(&f),
            &Env::default(),
        );
        assert_eq!(r.error, None);
        assert_eq!(t[1].1, "<p>two 1</p>", "the spine order visits c2 first");
        assert_eq!(t[0].1, "<p>2 3</p>");
        assert_eq!(r.final_output.as_deref(), Some("done 3"));
        assert_eq!(r.output.len(), 3);
        let mut missing = s.clone();
        missing.function = "Nope".into();
        let r = run_one(
            &missing,
            Action::ReplaceAll,
            &mut book(&mut t, &spine),
            &[f],
            &Env::default(),
        );
        assert!(r.error.unwrap().contains("does not exist"));
        // counting never needs the function
        let r = run_one(
            &missing,
            Action::Count,
            &mut book(&mut t, &spine),
            &[],
            &Env::default(),
        );
        assert_eq!(r.error, None);
    }

    #[test]
    fn a_function_error_stops_that_search_and_names_the_file() {
        let mut t = texts();
        let spine = vec![];
        let f = Function::new(
            "Bad",
            "",
            "fn replace(m, number, file, meta, data) { nope() }",
        );
        let mut s = SavedSearch::new("fn", st("one", "", Mode::Function), Scope::TextFiles);
        s.function = "Bad".into();
        let r = run_one(
            &s,
            Action::ReplaceAll,
            &mut book(&mut t, &spine),
            &[f],
            &Env::default(),
        );
        assert!(r.error.unwrap().starts_with("c1.xhtml: line 1"));
        assert_eq!(t, texts());
    }
}
