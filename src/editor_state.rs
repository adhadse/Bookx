//! What the book editor remembers between sessions, in files of their own next to `editor.json`: window
//! preferences (line wrapping, full paths, image grid), the open tabs and cursor of each book (CE-015), the
//! user's new-file templates (CE-039) and saved style rule sets (CE-133). No GTK in here.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bookx_editing::transform_css::RuleSet;
use serde::{Deserialize, Serialize};

pub fn config_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx").map(|d| d.config_dir().to_path_buf())
}

fn read_json<T: for<'a> Deserialize<'a> + Default>(path: &Path) -> T {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into()),
    )
}

// ---------------------------------------------------------------------------
// window preferences
// ---------------------------------------------------------------------------

/// The title shown in the header: the book title, with a dot in front while there are unsaved changes.
pub fn header_title(title: &str, dirty: bool) -> String {
    if dirty {
        format!("● {title}")
    } else {
        title.to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowPrefs {
    /// Wrap long lines in the code view (CE-012).
    pub wrap_lines: bool,
    /// Show the folder of each file in the file list (CE-044).
    pub full_paths: bool,
    /// Show the images as a grid of thumbnails (CE-046).
    pub image_grid: bool,
    /// Reopen the tabs and cursor positions of a book (CE-015).
    pub restore_state: bool,
}

impl Default for WindowPrefs {
    fn default() -> Self {
        WindowPrefs {
            wrap_lines: false,
            full_paths: false,
            image_grid: false,
            restore_state: true,
        }
    }
}

impl WindowPrefs {
    pub fn load_from(path: &Path) -> WindowPrefs {
        read_json(path)
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        write_json(path, self)
    }

    pub fn load() -> WindowPrefs {
        config_dir()
            .map(|d| Self::load_from(&d.join("editor-window.json")))
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(d) = config_dir() {
            let _ = self.save_to(&d.join("editor-window.json"));
        }
    }
}

// ---------------------------------------------------------------------------
// per-book state
// ---------------------------------------------------------------------------

/// The open tabs and caret positions of one book.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BookState {
    pub tabs: Vec<String>,
    pub current: Option<String>,
    /// File -> (line, column), both 0 based.
    pub cursors: BTreeMap<String, (i32, i32)>,
}

impl BookState {
    /// Drops what refers to files that no longer exist.
    pub fn retain_existing(&mut self, exists: impl Fn(&str) -> bool) {
        self.tabs.retain(|t| exists(t));
        self.cursors.retain(|p, _| exists(p));
        if self.current.as_deref().is_some_and(|c| !exists(c)) {
            self.current = None;
        }
    }
}

const MAX_BOOKS: usize = 200;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StateStore {
    pub books: BTreeMap<String, BookState>,
    /// Oldest first; the least recently saved books are forgotten past [`MAX_BOOKS`].
    order: Vec<String>,
}

impl StateStore {
    pub fn load_from(path: &Path) -> StateStore {
        read_json(path)
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        write_json(path, self)
    }

    pub fn get(&self, key: &str) -> Option<&BookState> {
        self.books.get(key)
    }

    pub fn put(&mut self, key: &str, state: BookState) {
        self.order.retain(|k| k != key);
        self.order.push(key.to_string());
        self.books.insert(key.to_string(), state);
        while self.order.len() > MAX_BOOKS {
            let old = self.order.remove(0);
            self.books.remove(&old);
        }
    }

    pub fn load() -> StateStore {
        config_dir()
            .map(|d| Self::load_from(&d.join("editor-state.json")))
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(d) = config_dir() {
            let _ = self.save_to(&d.join("editor-state.json"));
        }
    }
}

// ---------------------------------------------------------------------------
// command line: bookx --edit FILE [name ...] [--select-text TEXT] (CE-018)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditRequest {
    pub file: PathBuf,
    /// Files inside the book to open in tabs (full path or just the file name).
    pub names: Vec<String>,
    /// Text to select in the last of them.
    pub select: Option<String>,
}

/// Reads `--edit FILE [inner-name ...] [--select-text TEXT]` from the command line (without the program name).
pub fn parse_edit_args(args: &[String]) -> Option<EditRequest> {
    let at = args.iter().position(|a| a == "--edit")?;
    let file = PathBuf::from(args.get(at + 1)?);
    let mut names = Vec::new();
    let mut select = None;
    let mut it = args[at + 2..].iter();
    while let Some(a) = it.next() {
        if a == "--select-text" {
            select = it.next().cloned();
        } else if let Some(t) = a.strip_prefix("--select-text=") {
            select = Some(t.to_string());
        } else if !a.starts_with("--") {
            names.push(a.clone());
        }
    }
    Some(EditRequest {
        file,
        names,
        select,
    })
}

// ---------------------------------------------------------------------------
// closing several tabs (CE-011)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseMode {
    /// Every tab but the one kept.
    Others,
    /// The tabs to the right of the one kept.
    Right,
}

/// The tabs to close when `keep` stays open.
pub fn tabs_to_close(tabs: &[String], keep: &str, mode: CloseMode) -> Vec<String> {
    let Some(at) = tabs.iter().position(|t| t == keep) else {
        return Vec::new();
    };
    tabs.iter()
        .enumerate()
        .filter(|(i, _)| match mode {
            CloseMode::Others => *i != at,
            CloseMode::Right => *i > at,
        })
        .map(|(_, t)| t.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// revert to before / after (CE-008)
// ---------------------------------------------------------------------------

/// Checkpoints made by "Revert to Before" hold the state that was left; "Revert to After" goes back to them.
pub const REDO_PREFIX: &str = "After: ";
/// The snapshot taken before a redo, so that a redo can be reverted too.
pub const REDO_GUARD: &str = "Before: Redo";

/// The checkpoint "Revert to Before" restores: the newest one that is not a redo point.
pub fn undo_target(list: &[(u64, String)]) -> Option<u64> {
    list.iter()
        .rev()
        .find(|(_, n)| !n.starts_with(REDO_PREFIX))
        .map(|(id, _)| *id)
}

/// Whether the checkpoint an Undo toast points to is still there (the list is capped, and the user may have deleted it).
pub fn snapshot_exists(list: &[(u64, String)], id: u64) -> bool {
    list.iter().any(|(i, _)| *i == id)
}

/// Whether "Revert to Before the Last Change" and "Revert to After (Redo)" have something to do: from the checkpoint
/// list (oldest first) and the redo points made by reverts (newest last).
pub fn revert_enabled(list: &[(u64, String)], redo: &[u64]) -> (bool, bool) {
    (
        undo_target(list).is_some(),
        redo.last().is_some_and(|r| redo_still_valid(list, *r)),
    )
}

/// The items whose id is not in `hidden` (checkpoints that were deleted with an Undo toast still pending).
pub fn without_hidden<T>(items: Vec<T>, id_of: impl Fn(&T) -> u64, hidden: &[u64]) -> Vec<T> {
    items
        .into_iter()
        .filter(|i| !hidden.contains(&id_of(i)))
        .collect()
}

/// A redo point is still usable when nothing but redo guards was recorded after it (any other change in between
/// starts a new history).
pub fn redo_still_valid(list: &[(u64, String)], redo_id: u64) -> bool {
    match list.iter().position(|(id, _)| *id == redo_id) {
        None => false,
        Some(i) => list[i + 1..].iter().all(|(_, n)| n == REDO_GUARD),
    }
}

// ---------------------------------------------------------------------------
// user templates for new files
// ---------------------------------------------------------------------------

/// A new-file template the user keeps in the `editor-templates` folder of the configuration: any `.xhtml`, `.html`
/// or `.css` file; `{title}` in it is replaced by the title of the new file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserTemplate {
    pub label: String,
    pub extension: String,
    pub body: String,
}

pub fn templates_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("editor-templates"))
}

pub fn list_templates_in(dir: &Path) -> Vec<UserTemplate> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<UserTemplate> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let ext = p.extension()?.to_str()?.to_ascii_lowercase();
            if !matches!(ext.as_str(), "xhtml" | "html" | "css") {
                return None;
            }
            Some(UserTemplate {
                label: p.file_stem()?.to_str()?.to_string(),
                extension: if ext == "html" { "xhtml".into() } else { ext },
                body: std::fs::read_to_string(&p).ok()?,
            })
        })
        .collect();
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

pub fn user_templates() -> Vec<UserTemplate> {
    templates_dir()
        .map(|d| list_templates_in(&d))
        .unwrap_or_default()
}

impl UserTemplate {
    /// The file the template produces for the typed name: the extension of the template is added when missing.
    pub fn file_name(&self, typed: &str) -> Option<String> {
        let t = typed.trim();
        if t.is_empty() || t.contains('/') {
            return None;
        }
        let ext = format!(".{}", self.extension);
        Some(if t.to_ascii_lowercase().ends_with(&ext) {
            t.to_string()
        } else {
            format!("{t}{ext}")
        })
    }

    pub fn render(&self, title: &str) -> String {
        self.body.replace("{title}", title)
    }
}

// ---------------------------------------------------------------------------
// saved style rule sets
// ---------------------------------------------------------------------------

pub fn rules_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("editor-css-rules"))
}

fn rule_file_name(name: &str) -> String {
    let safe: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{}.json", safe.trim())
}

pub fn list_rule_sets_in(dir: &Path) -> Vec<RuleSet> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<RuleSet> = rd
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .filter_map(|e| RuleSet::from_json(&std::fs::read_to_string(e.path()).ok()?))
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn save_rule_set_in(dir: &Path, set: &RuleSet) -> std::io::Result<()> {
    if set.name.trim().is_empty() {
        return Err(std::io::Error::other("a rule set needs a name"));
    }
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(rule_file_name(&set.name)), set.to_json())
}

pub fn delete_rule_set_in(dir: &Path, name: &str) {
    let _ = std::fs::remove_file(dir.join(rule_file_name(name)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_editing::transform_css::CssTransform;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-estate-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn hidden_checkpoints_are_left_out() {
        let l = vec![(1u64, "a"), (2, "b"), (3, "c")];
        assert_eq!(
            without_hidden(l.clone(), |i| i.0, &[2]),
            vec![(1, "a"), (3, "c")]
        );
        assert_eq!(without_hidden(l.clone(), |i| i.0, &[]).len(), 3);
        assert!(without_hidden(l, |i| i.0, &[1, 2, 3]).is_empty());
    }

    #[test]
    fn undo_toast_needs_its_checkpoint() {
        let l = vec![(3, "Before: Reformat".to_string()), (4, "x".to_string())];
        assert!(snapshot_exists(&l, 3));
        assert!(!snapshot_exists(&l, 9));
        assert!(!snapshot_exists(&[], 3));
    }

    #[test]
    fn window_prefs_defaults_roundtrip_and_damage() {
        let d = tmp("prefs");
        let f = d.join("w.json");
        assert_eq!(WindowPrefs::load_from(&f), WindowPrefs::default());
        assert!(WindowPrefs::default().restore_state);
        let p = WindowPrefs {
            wrap_lines: true,
            full_paths: true,
            image_grid: true,
            restore_state: false,
        };
        p.save_to(&f).unwrap();
        assert_eq!(WindowPrefs::load_from(&f), p);
        std::fs::write(&f, "{\"wrap_lines\": true").unwrap();
        assert_eq!(WindowPrefs::load_from(&f), WindowPrefs::default());
        std::fs::write(&f, "{\"wrap_lines\": true}").unwrap();
        let q = WindowPrefs::load_from(&f);
        assert!(q.wrap_lines && q.restore_state && !q.full_paths);
    }

    #[test]
    fn book_state_is_kept_per_book_and_pruned() {
        let d = tmp("state");
        let f = d.join("s.json");
        let mut s = StateStore::default();
        let mut a = BookState {
            tabs: vec!["a.xhtml".into(), "gone.xhtml".into()],
            current: Some("gone.xhtml".into()),
            ..Default::default()
        };
        a.cursors.insert("a.xhtml".into(), (3, 4));
        a.cursors.insert("gone.xhtml".into(), (1, 1));
        s.put("1", a);
        s.save_to(&f).unwrap();
        let mut back = StateStore::load_from(&f);
        assert_eq!(back, s);
        let mut st = back.get("1").unwrap().clone();
        st.retain_existing(|p| p == "a.xhtml");
        assert_eq!(st.tabs, vec!["a.xhtml"]);
        assert_eq!(st.current, None);
        assert_eq!(st.cursors.len(), 1);
        assert!(back.get("2").is_none());
        for i in 0..MAX_BOOKS + 5 {
            back.put(&format!("b{i}"), BookState::default());
        }
        assert_eq!(back.books.len(), MAX_BOOKS);
        assert!(back.get("1").is_none(), "the oldest books are forgotten");
        assert!(back.get(&format!("b{}", MAX_BOOKS + 4)).is_some());
    }

    #[test]
    fn user_templates_are_read_from_a_folder() {
        let d = tmp("tpl");
        std::fs::write(
            d.join("Poem.xhtml"),
            "<html><body><h1>{title}</h1></body></html>",
        )
        .unwrap();
        std::fs::write(d.join("Plain.css"), "p { margin: 0 }").unwrap();
        std::fs::write(d.join("notes.txt"), "ignored").unwrap();
        std::fs::write(d.join("Old.html"), "<p>x</p>").unwrap();
        let t = list_templates_in(&d);
        let labels: Vec<&str> = t.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, vec!["Old", "Plain", "Poem"]);
        let poem = &t[2];
        assert_eq!(poem.file_name("verse").as_deref(), Some("verse.xhtml"));
        assert_eq!(
            poem.file_name("verse.XHTML").as_deref(),
            Some("verse.XHTML")
        );
        assert_eq!(poem.file_name("a/b"), None);
        assert_eq!(poem.file_name("  "), None);
        assert!(poem.render("Ode").contains("<h1>Ode</h1>"));
        assert_eq!(t[0].extension, "xhtml", "html templates make xhtml files");
        assert!(list_templates_in(&d.join("none")).is_empty());
    }

    #[test]
    fn edit_command_line() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(parse_edit_args(&a(&["book.epub"])), None);
        assert_eq!(parse_edit_args(&a(&["--edit"])), None);
        let r = parse_edit_args(&a(&[
            "--edit",
            "b.epub",
            "ch1.xhtml",
            "style.css",
            "--select-text",
            "some words",
        ]))
        .unwrap();
        assert_eq!(r.file, PathBuf::from("b.epub"));
        assert_eq!(r.names, vec!["ch1.xhtml", "style.css"]);
        assert_eq!(r.select.as_deref(), Some("some words"));
        let r = parse_edit_args(&a(&["--edit", "b.epub", "--select-text=x y"])).unwrap();
        assert!(r.names.is_empty());
        assert_eq!(r.select.as_deref(), Some("x y"));
    }

    #[test]
    fn tabs_to_close_by_mode() {
        let t: Vec<String> = ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect();
        assert_eq!(
            tabs_to_close(&t, "b", CloseMode::Others),
            vec!["a", "c", "d"]
        );
        assert_eq!(tabs_to_close(&t, "b", CloseMode::Right), vec!["c", "d"]);
        assert!(tabs_to_close(&t, "d", CloseMode::Right).is_empty());
        assert!(tabs_to_close(&t, "zz", CloseMode::Others).is_empty());
    }

    #[test]
    fn revert_before_and_after() {
        let l = |v: &[(u64, &str)]| -> Vec<(u64, String)> {
            v.iter().map(|(i, n)| (*i, n.to_string())).collect()
        };
        assert_eq!(undo_target(&[]), None);
        let list = l(&[(1, "Before: A"), (2, "Before: B"), (3, "After: B")]);
        assert_eq!(undo_target(&list), Some(2), "redo points are skipped");
        assert_eq!(undo_target(&l(&[(5, "After: x")])), None);
        // a redo point stays valid while only guards follow
        assert!(redo_still_valid(
            &l(&[(1, "Before: A"), (3, "After: B")]),
            3
        ));
        assert!(redo_still_valid(&l(&[(3, "After: B"), (4, REDO_GUARD)]), 3));
        assert!(!redo_still_valid(
            &l(&[(3, "After: B"), (4, "Before: Smarten")]),
            3
        ));
        assert!(!redo_still_valid(&l(&[(1, "x")]), 3));
    }

    #[test]
    fn rule_sets_save_list_delete() {
        let d = tmp("rules").join("sets");
        let set = RuleSet {
            name: "Bigger / bolder".into(),
            rules: vec![CssTransform::ScaleFontSizes { factor: 1.1 }],
        };
        save_rule_set_in(&d, &set).unwrap();
        save_rule_set_in(
            &d,
            &RuleSet {
                name: "A".into(),
                rules: vec![],
            },
        )
        .unwrap();
        let all = list_rule_sets_in(&d);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "A");
        assert_eq!(all[1], set);
        assert!(save_rule_set_in(&d, &RuleSet::default()).is_err());
        delete_rule_set_in(&d, "Bigger / bolder");
        assert_eq!(list_rule_sets_in(&d).len(), 1);
    }

    #[test]
    fn the_header_title_marks_unsaved_changes() {
        assert_eq!(header_title("Dune", false), "Dune");
        assert_eq!(header_title("Dune", true), "● Dune");
    }

    #[test]
    fn the_revert_items_follow_the_checkpoint_list() {
        let none: Vec<(u64, String)> = vec![];
        assert_eq!(revert_enabled(&none, &[]), (false, false));
        let l = vec![(1, "Before: Fix".to_string())];
        assert_eq!(revert_enabled(&l, &[]), (true, false));
        let l = vec![(1, format!("{}Fix", REDO_PREFIX))];
        assert_eq!(revert_enabled(&l, &[1]), (false, true));
        assert_eq!(revert_enabled(&l, &[9]), (false, false));
    }
}
