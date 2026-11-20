//! Glue between the app and the library health tools of `bookx-store` / `bookx-core`: opening the
//! library with its guards, the automatic snapshots and how things are worded for the user.
//! No GTK in here, so it is unit tested; the dialog is `ui/health.rs`.

use crate::i18n::tr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bookx_store::guard::{
    open_library_guarded, FsWarning, GuardedLibrary, LibraryLock, LockHolder,
};
use bookx_store::health::{self, Finding, Severity, Snapshot};
use bookx_store::SqliteStore;
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};

/// Settings of the automatic snapshots and the one-time notes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HealthPrefs {
    /// Take a snapshot of the catalogue on start-up and on exit.
    pub auto_backup: bool,
    /// How many automatic snapshots are kept (plus one per day for a week).
    pub keep: usize,
    /// Libraries (by path) whose network/sync-folder note was shown already.
    pub warned: Vec<String>,
    /// Where else backups are copied to: drives, shares, cloud folders, WebDAV.
    pub backup: bookx_backup::BackupSettings,
}

impl Default for HealthPrefs {
    fn default() -> Self {
        HealthPrefs {
            auto_backup: true,
            keep: 5,
            warned: Vec::new(),
            backup: bookx_backup::BackupSettings::default(),
        }
    }
}

/// What the app holds on to for the open library besides the books.
pub struct LibraryGuard {
    pub store: Arc<SqliteStore>,
    /// Released on exit or when another library is opened.
    pub lock: Option<LibraryLock>,
    /// Who else has the library open (then it is read-only).
    pub busy: Option<LockHolder>,
    pub warnings: Vec<FsWarning>,
}

/// Opens the library in `root`: takes the lock (or opens read-only when somebody else has it) and
/// looks at the file system. `BOOKX_READ_ONLY=1` forces read-only (the UI tests use it).
pub fn open(
    root: &Path,
) -> anyhow::Result<(bookx_core::library::Library, LibraryGuard, Option<usize>)> {
    bookx_core::layout::register_template_renderer(bookx_sources::template::render_library_path);
    let force = std::env::var("BOOKX_READ_ONLY").is_ok_and(|v| v == "1");
    let g: GuardedLibrary = open_library_guarded(root, "Bookx", force)?;
    let migrated = g.opened.migrated;
    Ok((
        g.opened.library,
        LibraryGuard {
            store: g.opened.store,
            lock: g.lock,
            busy: g.busy,
            warnings: g.warnings,
        },
        migrated,
    ))
}

/// The sentence shown when the library is read-only because somebody else has it.
pub fn read_only_note(g: &LibraryGuard) -> Option<String> {
    g.busy.as_ref().map(|h| h.describe())
}

/// The warnings not shown before for this library; marks them as shown. Empty after the first time.
pub fn take_new_warnings(
    prefs: &mut HealthPrefs,
    root: &Path,
    warnings: &[FsWarning],
) -> Vec<String> {
    let key = root.display().to_string();
    if warnings.is_empty() || prefs.warned.contains(&key) {
        return Vec::new();
    }
    prefs.warned.push(key);
    warnings.iter().map(|w| w.message.clone()).collect()
}

/// Takes the automatic snapshot if enabled and the catalogue changed; prunes old ones.
pub fn auto_snapshot(store: &SqliteStore, root: &Path, prefs: &HealthPrefs) -> Option<Snapshot> {
    if !prefs.auto_backup {
        return None;
    }
    health::auto_snapshot(store, root, prefs.keep.max(1), Utc::now())
        .ok()
        .flatten()
}

pub fn plural(n: usize, word: &str) -> String {
    use crate::i18n::ntr;
    let n64 = n as u64;
    match word {
        "book" => ntr("{} book", "{} books", n64),
        "new book" => ntr("{} new book", "{} new books", n64),
        "place" => ntr("{} place", "{} places", n64),
        "file" => ntr("{} file", "{} files", n64),
        "problem" => ntr("{} problem", "{} problems", n64),
        "warning" => ntr("{} warning", "{} warnings", n64),
        "name" => ntr("{} name", "{} names", n64),
        "backup" => ntr("{} backup", "{} backups", n64),
        "edition" => ntr("{} edition", "{} editions", n64),
        _ => {
            if n == 1 {
                format!("1 {word}")
            } else {
                format!("{n} {word}s")
            }
        }
    }
}

pub fn size_text(bytes: u64) -> String {
    if bytes < 1024 * 1024 {
        format!("{:.0} KB", (bytes as f64 / 1024.0).max(1.0))
    } else {
        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
    }
}

/// The icon for a finding.
pub fn finding_icon(f: &Finding) -> &'static str {
    match f.severity {
        Severity::Error => "dialog-error-symbolic",
        Severity::Warning => "dialog-warning-symbolic",
        Severity::Info => "dialog-information-symbolic",
    }
}

/// The result line under the check button.
pub fn check_summary(books: usize, findings: &[Finding]) -> String {
    if findings.is_empty() {
        return format!("{} checked, no problems found", plural(books, "book"));
    }
    let errors = findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();
    format!(
        "{} checked: {} found ({} serious)",
        plural(books, "book"),
        plural(findings.len(), "problem"),
        errors
    )
}

/// What an operation of the history looks like in a list.
pub fn describe_op(op: &bookx_core::history::Op) -> String {
    format!(
        "{} · {}{}",
        op.at.with_timezone(&Local).format("%d %b %H:%M"),
        op.label,
        if op.undone { " (undone)" } else { "" }
    )
}

/// `Deleted 12 Nov 10:15`
pub fn describe_trash(e: &bookx_core::history::TrashEntry) -> String {
    let files = if e.files.is_empty() {
        "catalogue entry only".to_string()
    } else {
        plural(e.files.len(), "file")
    };
    format!(
        "{} · deleted {} · {}",
        e.book.meta.authors_display(),
        e.at.with_timezone(&Local).format("%d %b %H:%M"),
        files
    )
}

/// What the Preferences layout choices are called, with the template text where one applies.
pub fn layout_choices() -> [(&'static str, &'static str); 4] {
    [
        ("tree", crate::i18n::N_("Author / Title folders")),
        ("flat", crate::i18n::N_("One flat folder")),
        ("template", crate::i18n::N_("Custom template")),
        ("linked", crate::i18n::N_("Link in place (do not copy)")),
    ]
}

/// Index into [`layout_choices`] of the library's layout.
pub fn layout_index(l: &bookx_core::layout::Layout) -> u32 {
    use bookx_core::layout::Layout;
    match l {
        Layout::Tree => 0,
        Layout::Flat => 1,
        Layout::Template(_) => 2,
        Layout::Linked => 3,
    }
}

/// What a layout change would do, in words, for the preview before it is applied.
pub fn layout_preview(plans: &[bookx_core::files::MovePlan]) -> String {
    if plans.is_empty() {
        return tr("No files need to move.");
    }
    let mut s = format!("{} will be moved:\n", plural(plans.len(), "book"));
    for p in plans.iter().take(6) {
        s += &format!("  {} → {}\n", p.old_file, p.new_file);
    }
    if plans.len() > 6 {
        s += &format!("  … and {} more\n", plans.len() - 6);
    }
    s += "If anything fails, every file goes back where it was.";
    s
}

/// Records where the fields changed by an edit came from: `fetched` has the fields an online lookup
/// filled (with the source names), every other changed field was typed by the user.
pub fn record_edit(
    store: &SqliteStore,
    book: u64,
    before: &bookx_core::opf::Metadata,
    after: &bookx_core::opf::Metadata,
    fetched: &[(String, String)],
) {
    record_edit_as(
        store,
        book,
        before,
        after,
        fetched,
        YOU,
        Utc::now().timestamp(),
    );
}

/// [`record_edit`] at a given time.
pub fn record_edit_at(
    store: &SqliteStore,
    book: u64,
    before: &bookx_core::opf::Metadata,
    after: &bookx_core::opf::Metadata,
    fetched: &[(String, String)],
    now: i64,
) {
    record_edit_as(store, book, before, after, fetched, YOU, now);
}

/// [`record_edit`] where the fields not named in `fetched` come from `default_source` (for
/// example `bulk edit`, `tidy names`, `editor`) instead of the user's own typing.
pub fn record_edit_as(
    store: &SqliteStore,
    book: u64,
    before: &bookx_core::opf::Metadata,
    after: &bookx_core::opf::Metadata,
    fetched: &[(String, String)],
    default_source: &str,
    now: i64,
) {
    let mut by_source: std::collections::BTreeMap<String, Vec<&'static str>> = Default::default();
    for field in crate::metadata_fetch::changed_fields(before, after) {
        let source = fetched
            .iter()
            .find(|(f, _)| f == field)
            .map(|(_, s)| s.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| default_source.to_string());
        by_source.entry(source).or_default().push(field);
    }
    for (source, fields) in by_source {
        let _ = store.set_provenance(book, &fields, &source, now);
    }
}

/// The metadata of `ids` as they are now (all books when `ids` is empty), to be compared by
/// [`record_changes`] after a bulk operation.
pub fn snapshot_meta(
    lib: &bookx_core::library::Library,
    ids: &[u64],
) -> Vec<(u64, bookx_core::opf::Metadata)> {
    lib.books
        .iter()
        .filter(|b| ids.is_empty() || ids.contains(&b.id))
        .map(|b| (b.id, b.meta.clone()))
        .collect()
}

/// Records `source` for every field that differs from the `before` snapshot. Returns the number of
/// books that changed.
pub fn record_changes(
    store: &SqliteStore,
    lib: &bookx_core::library::Library,
    before: &[(u64, bookx_core::opf::Metadata)],
    source: &str,
    now: i64,
) -> usize {
    let mut n = 0;
    for (id, old) in before {
        if let Some(b) = lib.get(*id) {
            if crate::metadata_fetch::changed_fields(old, &b.meta).is_empty() {
                continue;
            }
            record_edit_as(store, *id, old, &b.meta, &[], source, now);
            n += 1;
        }
    }
    n
}

/// How a field typed by the user is named in the record.
pub const YOU: &str = "you";
/// Fields changed for several books at once with *Edit Metadata in Bulk*.
pub const BULK_EDIT: &str = "bulk edit";
/// Fields changed by *Tidy names*.
pub const TIDY: &str = "tidy names";
/// Fields changed in the metadata tab of the book editor.
pub const EDITOR: &str = "editor";

/// `Open Library: publisher, tags · you: title` for the details panel; empty without records.
pub fn provenance_summary(p: &[bookx_store::provenance::FieldSource]) -> String {
    let mut by_source: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
    for f in p {
        by_source
            .entry(f.source.as_str())
            .or_default()
            .push(f.field.as_str());
    }
    by_source
        .into_iter()
        .map(|(s, f)| format!("{s}: {}", f.join(", ")))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The folder the Calibre chooser starts in: an existing Calibre library next to the documents.
pub fn calibre_hint() -> Option<PathBuf> {
    let home = directories::UserDirs::new()?;
    let docs = home.document_dir()?.to_path_buf();
    ["Calibre Library", "Calibre-Bibliothek", "calibre"]
        .iter()
        .map(|n| docs.join(n))
        .find(|p| p.join("metadata.db").is_file())
}

pub fn when(t: DateTime<Utc>) -> String {
    crate::i18n::fmt_when(&t.with_timezone(&Local))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_store::guard::FsWarningKind;
    use bookx_store::health::SnapshotKind;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-health-glue-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn defaults_keep_a_few_snapshots_and_old_configs_load() {
        let p = HealthPrefs::default();
        assert!(p.auto_backup && p.keep == 5);
        let loaded: HealthPrefs = serde_json::from_str("{\"keep\": 2}").unwrap();
        assert!(loaded.auto_backup && loaded.keep == 2);
    }

    #[test]
    fn network_notes_are_shown_once_per_library() {
        let mut p = HealthPrefs::default();
        let w = [FsWarning {
            kind: FsWarningKind::Network,
            message: "careful".into(),
        }];
        let root = Path::new("/mnt/nas/lib");
        assert_eq!(
            take_new_warnings(&mut p, root, &w),
            vec!["careful".to_string()]
        );
        assert!(take_new_warnings(&mut p, root, &w).is_empty());
        assert_eq!(take_new_warnings(&mut p, Path::new("/other"), &w).len(), 1);
        assert!(take_new_warnings(&mut p, Path::new("/third"), &[]).is_empty());
    }

    #[test]
    fn the_auto_snapshot_respects_the_switch_and_the_keep_count() {
        let d = tmp();
        let (lib, g, _) = open(&d.join("lib")).unwrap();
        let root = lib.root.clone();
        let mut prefs = HealthPrefs {
            auto_backup: false,
            ..Default::default()
        };
        assert!(auto_snapshot(&g.store, &root, &prefs).is_none());
        assert!(health::list_snapshots(&root).is_empty());
        prefs.auto_backup = true;
        let s = auto_snapshot(&g.store, &root, &prefs).unwrap();
        assert_eq!(s.kind, SnapshotKind::Auto);
        assert!(
            auto_snapshot(&g.store, &root, &prefs).is_none(),
            "unchanged since"
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_second_open_is_read_only_and_says_who_has_the_library() {
        let d = tmp();
        let (_lib, first, _) = open(&d.join("lib")).unwrap();
        assert!(first.lock.is_some() && read_only_note(&first).is_none());
        let (lib2, second, _) = open(&d.join("lib")).unwrap();
        assert!(lib2.is_read_only());
        let note = read_only_note(&second).unwrap();
        assert!(note.contains("already open in Bookx"), "{note}");
        drop(first);
        let (lib3, _third, _) = open(&d.join("lib")).unwrap();
        assert!(!lib3.is_read_only());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn texts_are_plain_and_singular_where_they_should_be() {
        assert_eq!(plural(1, "book"), "1 book");
        assert_eq!(plural(0, "book"), "0 books");
        assert_eq!(size_text(100), "1 KB");
        assert_eq!(size_text(5 * 1024 * 1024 + 512 * 1024), "5.5 MB");
        assert_eq!(check_summary(3, &[]), "3 books checked, no problems found");
        let f = Finding {
            severity: Severity::Error,
            kind: health::FindingKind::MissingFile,
            book: Some(1),
            message: "x".into(),
            fixes: vec![],
        };
        assert_eq!(
            check_summary(1, std::slice::from_ref(&f)),
            "1 book checked: 1 problem found (1 serious)"
        );
        assert_eq!(finding_icon(&f), "dialog-error-symbolic");
    }

    #[test]
    fn edits_record_where_each_changed_field_came_from() {
        let d = tmp();
        let (mut lib, g, _) = open(&d.join("lib")).unwrap();
        std::fs::write(
            d.join("a.md"),
            "---\ntitle: \"T\"\nauthor: \"A\"\n---\n# a\n\ntext\n",
        )
        .unwrap();
        lib.add_path(&d.join("a.md"), false).unwrap();
        let before = lib.books[0].meta.clone();
        let mut after = before.clone();
        after.publisher = "Mock Press".into();
        after.title = "Typed Title".into();
        record_edit(
            &g.store,
            1,
            &before,
            &after,
            &[("publisher".to_string(), "Open Library".to_string())],
        );
        let p = g.store.provenance(1);
        assert_eq!(
            provenance_summary(&p),
            "Open Library: publisher · you: title"
        );
        // an edit that changes nothing records nothing
        record_edit(&g.store, 1, &after, &after, &[]);
        assert_eq!(g.store.provenance(1).len(), 2);
        assert_eq!(provenance_summary(&[]), "");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn bulk_changes_record_their_source_for_the_books_that_changed() {
        let d = tmp();
        let (mut lib, g, _) = open(&d.join("lib")).unwrap();
        for i in 0..2 {
            std::fs::write(
                d.join(format!("{i}.md")),
                format!("---\ntitle: \"T{i}\"\nauthor: \"A\"\n---\n# a\n\ntext\n"),
            )
            .unwrap();
            lib.add_path(&d.join(format!("{i}.md")), false).unwrap();
        }
        let before = snapshot_meta(&lib, &[]);
        assert_eq!(before.len(), 2);
        let first = lib.books[0].id;
        lib.bulk_edit(&[first], |m| m.publisher = "P".into())
            .unwrap();
        assert_eq!(record_changes(&g.store, &lib, &before, BULK_EDIT, 7), 1);
        assert_eq!(
            provenance_summary(&g.store.provenance(first)),
            "bulk edit: publisher"
        );
        let other = lib.books[1].id;
        assert!(
            g.store.provenance(other).is_empty(),
            "untouched books record nothing"
        );
        assert_eq!(snapshot_meta(&lib, &[other]).len(), 1);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn layout_choices_line_up_with_their_index() {
        use bookx_core::layout::Layout;
        let c = layout_choices();
        for (i, l) in [
            Layout::Tree,
            Layout::Flat,
            Layout::Template("x".into()),
            Layout::Linked,
        ]
        .iter()
        .enumerate()
        {
            assert_eq!(layout_index(l) as usize, i);
            assert_eq!(Layout::parse(c[i].0, "x").unwrap().label(), l.label());
        }
    }

    #[test]
    fn the_layout_preview_lists_a_few_moves_and_the_promise() {
        use bookx_core::files::MovePlan;
        assert_eq!(layout_preview(&[]), "No files need to move.");
        let plans: Vec<MovePlan> = (0..9)
            .map(|i| MovePlan {
                id: i,
                title: format!("t{i}"),
                old_file: format!("a/{i}.epub"),
                new_file: format!("{i}.epub"),
                old_cover: None,
                new_cover: None,
                files: vec![],
            })
            .collect();
        let p = layout_preview(&plans);
        assert!(
            p.contains("9 books will be moved")
                && p.contains("and 3 more")
                && p.contains("goes back"),
            "{p}"
        );
    }
}
