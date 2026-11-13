//! A watched import folder with a review queue (roadmap PP-G-32).
//!
//! Books dropped into the folder are not added blindly: [`scan`] lists them with the title and
//! author found in the file and a note when the library already has the book, and the user adds or
//! dismisses each one (or all). Files still being copied are left alone until they stop changing.
//! What was added or dismissed is remembered in `<library>/.bookx/inbox.json`, so a book comes back
//! only when its file changes.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::history::STATE_DIR;
use crate::library::{collect_book_files, AddOutcome, Library, Prepared};

/// How long a file must be unchanged before it counts as fully copied.
pub const SETTLE: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxState {
    /// The watched folder.
    #[serde(default)]
    pub folder: Option<PathBuf>,
    /// Keys of files that were added or dismissed.
    #[serde(default)]
    pub done: BTreeSet<String>,
}

pub fn state_path(root: &Path) -> PathBuf {
    root.join(STATE_DIR).join("inbox.json")
}

impl InboxState {
    /// Reads the state; missing or damaged means empty.
    pub fn load(root: &Path) -> InboxState {
        std::fs::read(state_path(root))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let p = state_path(root);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, &p).with_context(|| format!("cannot write {}", p.display()))
    }
}

/// Identifies one version of a file: moving it keeps the key only if size and time stay.
pub fn file_key(path: &Path, size: u64, mtime_secs: i64) -> String {
    format!("{}|{size}|{mtime_secs}", path.display())
}

/// Whether a file last changed at `mtime` is old enough to be read.
pub fn is_settled(mtime: SystemTime, now: SystemTime, settle: Duration) -> bool {
    now.duration_since(mtime)
        .map(|d| d >= settle)
        .unwrap_or(false)
}

/// A file waiting in the folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub path: PathBuf,
    pub key: String,
    pub size: u64,
    pub title: String,
    pub authors: Vec<String>,
    /// The library book it looks like, when there is one.
    pub duplicate_of: Option<u64>,
    /// Why the file cannot be added (unreadable, unsupported).
    pub error: Option<String>,
}

fn mtime_of(m: &std::fs::Metadata) -> (SystemTime, i64) {
    let t = m.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let s = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    (t, s)
}

/// Lists the new books in the watched folder. `prepare` reads a file the way the importer does.
/// Files that are still changing are skipped; the next scan sees them.
pub fn scan(
    state: &InboxState,
    lib: &Library,
    now: SystemTime,
    settle: Duration,
    prepare: &dyn Fn(&Path) -> Result<Prepared>,
) -> Vec<Candidate> {
    let Some(folder) = &state.folder else {
        return Vec::new();
    };
    let mut files = Vec::new();
    collect_book_files(folder, true, &mut files);
    let mut out = Vec::new();
    for path in files {
        let Ok(m) = std::fs::metadata(&path) else {
            continue;
        };
        let (mtime, secs) = mtime_of(&m);
        if !is_settled(mtime, now, settle) {
            continue;
        }
        let key = file_key(&path, m.len(), secs);
        if state.done.contains(&key) {
            continue;
        }
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let c = match prepare(&path) {
            Ok(p) => Candidate {
                duplicate_of: lib.find_duplicate(&p.meta),
                title: p.meta.title,
                authors: p.meta.authors,
                path,
                key,
                size: m.len(),
                error: None,
            },
            Err(e) => Candidate {
                path,
                key,
                size: m.len(),
                title: name,
                authors: Vec::new(),
                duplicate_of: None,
                error: Some(format!("{e:#}")),
            },
        };
        out.push(c);
    }
    out
}

/// Adds the book to the library and remembers it. A book that is already in the library counts
/// as handled too (the outcome says so).
pub fn accept(
    state: &mut InboxState,
    lib: &mut Library,
    c: &Candidate,
    allow_duplicate: bool,
    prepare: &dyn Fn(&Path) -> Result<Prepared>,
) -> Result<AddOutcome> {
    let p = prepare(&c.path).with_context(|| format!("cannot read {}", c.path.display()))?;
    let out = lib.commit_import(p, allow_duplicate)?;
    lib.save()?;
    state.done.insert(c.key.clone());
    Ok(out)
}

/// Remembers that the user does not want this file.
pub fn dismiss(state: &mut InboxState, c: &Candidate) {
    state.done.insert(c.key.clone());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-inbox-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn md(dir: &Path, title: &str) -> PathBuf {
        let p = dir.join(format!("{title}.md"));
        std::fs::write(
            &p,
            format!(
                "---\ntitle: \"{title}\"\nauthor: \"An Author\"\n---\n# One\n\nText of {title}.\n"
            ),
        )
        .unwrap();
        p
    }

    fn prepare(p: &Path) -> Result<Prepared> {
        crate::library::prepare_import(p)
    }

    fn later() -> SystemTime {
        SystemTime::now() + Duration::from_secs(60)
    }

    #[test]
    fn a_file_must_stop_changing_before_it_is_offered() {
        let now = SystemTime::now();
        assert!(!is_settled(now, now, SETTLE));
        assert!(is_settled(now - Duration::from_secs(10), now, SETTLE));
        assert!(
            !is_settled(now + Duration::from_secs(10), now, SETTLE),
            "a time in the future"
        );
    }

    #[test]
    fn new_files_are_listed_with_duplicates_flagged() {
        let d = tmp();
        let mut lib = Library::in_memory(&d.join("lib")).unwrap();
        let watch = d.join("watch");
        std::fs::create_dir_all(watch.join("sub")).unwrap();
        md(&watch, "Fresh");
        md(&watch.join("sub"), "Known");
        std::fs::write(watch.join("notes.pdf.txt.bak"), "x").unwrap();
        lib.add_path(&md(&d, "Known"), false).unwrap();
        let st = InboxState {
            folder: Some(watch.clone()),
            ..Default::default()
        };
        // not settled yet
        assert!(scan(
            &st,
            &lib,
            SystemTime::now() - Duration::from_secs(30),
            SETTLE,
            &prepare
        )
        .is_empty());
        let found = scan(&st, &lib, later(), SETTLE, &prepare);
        assert_eq!(found.len(), 2);
        let known = found.iter().find(|c| c.title == "Known").unwrap();
        assert!(known.duplicate_of.is_some());
        let fresh = found.iter().find(|c| c.title == "Fresh").unwrap();
        assert!(fresh.duplicate_of.is_none());
        assert_eq!(fresh.authors, vec!["An Author"]);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn accepting_adds_and_a_handled_file_does_not_come_back() {
        let d = tmp();
        let mut lib = Library::in_memory(&d.join("lib")).unwrap();
        let watch = d.join("watch");
        std::fs::create_dir_all(&watch).unwrap();
        md(&watch, "One");
        md(&watch, "Two");
        let mut st = InboxState {
            folder: Some(watch),
            ..Default::default()
        };
        let found = scan(&st, &lib, later(), SETTLE, &prepare);
        assert_eq!(found.len(), 2);
        let one = found.iter().find(|c| c.title == "One").unwrap();
        let two = found.iter().find(|c| c.title == "Two").unwrap();
        assert!(matches!(
            accept(&mut st, &mut lib, one, false, &prepare).unwrap(),
            AddOutcome::Added(_)
        ));
        dismiss(&mut st, two);
        assert_eq!(lib.books.len(), 1);
        assert!(scan(&st, &lib, later(), SETTLE, &prepare).is_empty());

        // the state survives a restart
        st.save(&d.join("lib")).unwrap();
        assert_eq!(InboxState::load(&d.join("lib")), st);

        // a changed file (new size) is offered again
        let p = st.folder.clone().unwrap().join("Two.md");
        std::fs::write(&p, std::fs::read_to_string(&p).unwrap() + "\nMore text.\n").unwrap();
        assert_eq!(scan(&st, &lib, later(), SETTLE, &prepare).len(), 1);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn an_unreadable_file_is_listed_with_its_error() {
        let d = tmp();
        let lib = Library::in_memory(&d.join("lib")).unwrap();
        let watch = d.join("watch");
        std::fs::create_dir_all(&watch).unwrap();
        std::fs::write(watch.join("broken.epub"), "not an epub").unwrap();
        let st = InboxState {
            folder: Some(watch),
            ..Default::default()
        };
        let found = scan(&st, &lib, later(), SETTLE, &prepare);
        assert_eq!(found.len(), 1);
        assert!(found[0].error.is_some());
        assert_eq!(found[0].title, "broken");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn no_folder_means_nothing_to_scan() {
        let d = tmp();
        let lib = Library::in_memory(&d.join("lib")).unwrap();
        assert!(scan(&InboxState::default(), &lib, later(), SETTLE, &prepare).is_empty());
        assert_eq!(InboxState::load(&d), InboxState::default());
        let _ = std::fs::remove_dir_all(d);
    }
}
