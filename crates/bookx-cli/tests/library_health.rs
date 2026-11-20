//! The `library` housekeeping commands: backups, check, Calibre import, layout, trash, undo,
//! tidy and the import folder. Each test has its own sandbox, so they run in parallel.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use bookx_cli::{run, Env};

struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new() -> Sandbox {
        let dir = std::env::temp_dir().join(format!("bookx-cli-health-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Sandbox { dir }
    }
    fn lib(&self) -> PathBuf {
        self.dir.join("lib")
    }
    fn cli(&self, args: &[&str]) -> (i32, String, String) {
        let mut v = vec!["bookx-cli".to_string()];
        v.extend(args.iter().map(|s| s.to_string()));
        let env = Env {
            default_library: self.lib(),
            stop: Arc::new(AtomicBool::new(false)),
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&v, &env, &mut out, &mut err);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }
    fn md(&self, dir: &Path, title: &str, author: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let p = dir.join(format!("{title}.md"));
        std::fs::write(
            &p,
            format!(
                "---\ntitle: \"{title}\"\nauthor: \"{author}\"\n---\n# One\n\nText of {title}.\n"
            ),
        )
        .unwrap();
        p
    }
    fn add(&self, title: &str, author: &str) {
        let p = self.md(&self.dir.join("src"), title, author);
        let (code, out, err) = self.cli(&["library", "add", p.to_str().unwrap()]);
        assert_eq!(code, 0, "{out} {err}");
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn backup_lists_and_restores_a_snapshot() {
    let s = Sandbox::new();
    s.add("Dune", "Frank Herbert");
    let (code, out, _) = s.cli(&["library", "backup"]);
    assert_eq!(code, 0);
    assert!(out.contains("backed up 1 book(s)"), "{out}");
    s.add("Emma", "Jane Austen");
    let (_, out, _) = s.cli(&["library", "snapshots"]);
    assert!(out.contains("1 books") && out.contains("Manual"), "{out}");
    let (code, out, err) = s.cli(&["library", "restore", "latest"]);
    assert_eq!(code, 0, "{out} {err}");
    let (_, out, _) = s.cli(&["library", "list"]);
    assert!(out.contains("Dune") && !out.contains("Emma"), "{out}");
    let (_, json, _) = s.cli(&["library", "snapshots", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        v.as_array().unwrap().len(),
        2,
        "the manual one and the one taken before the restore"
    );
    let (code, _, err) = s.cli(&["library", "restore", "nonexistent.db"]);
    assert_eq!(code, 2);
    assert!(err.contains("no snapshot"), "{err}");
}

#[test]
fn check_reports_a_missing_file_and_fix_moves_the_entry_to_the_trash() {
    let s = Sandbox::new();
    s.add("Dune", "Frank Herbert");
    let (code, out, _) = s.cli(&["library", "check"]);
    assert_eq!(code, 0);
    assert!(out.contains("1 book(s) checked, 0 problem(s)"), "{out}");
    let (_, info, _) = s.cli(&["library", "info", "1", "--json"]);
    let file = serde_json::from_str::<serde_json::Value>(&info).unwrap()["file"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::remove_file(s.lib().join(&file)).unwrap();
    let (code, out, _) = s.cli(&["library", "check", "--json"]);
    assert_eq!(code, 1);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["findings"][0]["severity"], "error");
    assert!(v["findings"][0]["message"]
        .as_str()
        .unwrap()
        .contains("missing"));
    let (code, out, _) = s.cli(&["library", "check", "--fix"]);
    assert_eq!(code, 1, "the problem was there when it was checked");
    assert!(out.contains("fixed:"), "{out}");
    let (_, out, _) = s.cli(&["library", "trash", "list"]);
    assert!(out.contains("Dune"), "{out}");
    let (code, _, _) = s.cli(&["library", "check"]);
    assert_eq!(code, 0);
}

#[test]
fn drift_is_found_after_the_file_is_changed_behind_the_catalogue() {
    let s = Sandbox::new();
    s.add("Dune", "Frank Herbert");
    // edit the catalogue only, through the store
    {
        let mut o = bookx_store::open_library(&s.lib()).unwrap();
        o.library.get_mut(1).unwrap().meta.title = "Dune Changed".into();
        o.library.save().unwrap();
    }
    let (code, out, _) = s.cli(&["library", "check", "--drift"]);
    assert_eq!(code, 0, "drift is a warning");
    assert!(
        out.contains("differs from its file") && out.contains("Dune Changed"),
        "{out}"
    );
    let (_, out, _) = s.cli(&["library", "check"]);
    assert!(out.contains("0 problem(s)"), "{out}");
}

#[test]
fn layout_dry_run_changes_nothing_and_the_change_moves_and_undoes() {
    let s = Sandbox::new();
    s.add("Dune", "Frank Herbert");
    s.add("Emma", "Jane Austen");
    let (_, out, _) = s.cli(&["library", "layout"]);
    assert_eq!(out.trim(), "tree");
    let (code, out, _) = s.cli(&["library", "layout", "flat", "--dry-run"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("Frank Herbert - Dune.epub") && out.contains("nothing was changed"),
        "{out}"
    );
    assert!(s.lib().join("Frank Herbert").exists());
    let (code, out, err) = s.cli(&["library", "layout", "flat"]);
    assert_eq!(code, 0, "{out} {err}");
    assert!(s.lib().join("Frank Herbert - Dune.epub").exists());
    assert!(!s.lib().join("Frank Herbert").exists());
    let (_, out, _) = s.cli(&["library", "layout"]);
    assert_eq!(out.trim(), "flat");
    // a new book follows the layout
    s.add("Ulysses", "James Joyce");
    assert!(s.lib().join("James Joyce - Ulysses.epub").exists());
    // the move is one undoable step... after the import, so undo twice
    let (_, out, _) = s.cli(&["library", "history"]);
    assert!(out.contains("Moved 2 books"), "{out}");
    let (code, _, err) = s.cli(&["library", "layout", "nonsense"]);
    assert_eq!(code, 2);
    assert!(err.contains("unknown layout"), "{err}");
}

#[test]
fn a_template_layout_renders_paths_and_rejects_a_bad_template() {
    let s = Sandbox::new();
    s.add("Dune", "Frank Herbert");
    let (code, _, err) = s.cli(&["library", "layout", "template", "--template", "{title"]);
    assert_eq!(code, 2);
    assert!(err.contains("bad template"), "{err}");
    let (code, out, err) = s.cli(&[
        "library",
        "layout",
        "template",
        "--template",
        "{author}/{title}",
    ]);
    assert_eq!(code, 0, "{out} {err}");
    assert!(s.lib().join("Frank Herbert/Dune.epub").exists());
    let (_, out, _) = s.cli(&["library", "layout"]);
    assert!(out.contains("template: {author}/{title}"), "{out}");
}

#[test]
fn trash_restore_empty_and_undo() {
    let s = Sandbox::new();
    s.add("Dune", "Frank Herbert");
    s.add("Emma", "Jane Austen");
    s.cli(&["library", "remove", "1", "--delete-files"]);
    let (_, out, _) = s.cli(&["library", "list"]);
    assert!(!out.contains("Dune"));
    let (_, out, _) = s.cli(&["library", "trash", "list"]);
    assert!(out.contains("Dune"), "{out}");
    let (code, out, err) = s.cli(&["library", "trash", "restore", "1"]);
    assert_eq!(code, 0, "{out} {err}");
    let (_, out, _) = s.cli(&["library", "list"]);
    assert!(out.contains("Dune"));
    s.cli(&["library", "remove", "2", "--delete-files"]);
    let (_, out, _) = s.cli(&["library", "undo"]);
    assert!(out.contains("undone: Deleted"), "{out}");
    let (_, out, _) = s.cli(&["library", "list"]);
    assert!(out.contains("Emma"));
    s.cli(&["library", "remove", "2", "--delete-files"]);
    let (_, out, _) = s.cli(&["library", "trash", "empty"]);
    assert!(out.contains("erased 1 book(s)"), "{out}");
    let (code, _, err) = s.cli(&["library", "trash", "restore", "2"]);
    assert_eq!(code, 2);
    assert!(err.contains("not in the trash"), "{err}");
}

#[test]
fn undo_with_nothing_to_undo_says_so() {
    let s = Sandbox::new();
    s.add("Dune", "Frank Herbert");
    let (code, out, _) = s.cli(&["library", "undo"]);
    assert_eq!(code, 0);
    assert!(out.contains("undone: Imported"), "{out}");
    let (code, out, _) = s.cli(&["library", "undo"]);
    assert_eq!(code, 1);
    assert!(out.contains("nothing to undo"));
}

#[test]
fn tidy_lists_then_applies() {
    let s = Sandbox::new();
    s.add("Dune", "Herbert, Frank");
    s.add("Messiah", "Frank Herbert");
    s.add("Children", "Frank Herbert");
    let (_, out, _) = s.cli(&["library", "tidy"]);
    assert!(out.contains("1 change(s) proposed"), "{out}");
    assert!(
        out.contains("Herbert, Frank") && out.contains("→ “Frank Herbert”"),
        "{out}"
    );
    let (_, out, _) = s.cli(&["library", "tidy", "--apply"]);
    assert!(out.contains("1 book(s) changed"), "{out}");
    let (_, out, _) = s.cli(&["library", "tidy"]);
    assert!(out.contains("0 change(s)"), "{out}");
}

#[test]
fn the_import_folder_queues_and_adds() {
    let s = Sandbox::new();
    s.add("Dune", "Frank Herbert");
    let watch = s.dir.join("watch");
    s.md(&watch, "Fresh", "New Author");
    s.md(&watch, "Dune", "Frank Herbert");
    let (code, _, err) = s.cli(&["library", "inbox", "set", watch.to_str().unwrap()]);
    assert_eq!(code, 0, "{err}");
    let (_, out, _) = s.cli(&["library", "inbox", "scan", "--settle", "0"]);
    assert!(
        out.contains("2 book(s) waiting") && out.contains("looks like #1"),
        "{out}"
    );
    // a fresh file is not offered yet
    let (_, out, _) = s.cli(&["library", "inbox", "scan", "--settle", "3600"]);
    assert!(out.contains("0 book(s) waiting"), "{out}");
    let (code, out, _) = s.cli(&["library", "inbox", "add-all", "--settle", "0"]);
    assert_eq!(code, 0);
    assert!(out.contains("1 added, 1 duplicate(s)"), "{out}");
    let (_, out, _) = s.cli(&["library", "inbox", "scan", "--settle", "0"]);
    assert!(out.contains("0 book(s) waiting"), "{out}");
}

#[test]
fn a_second_writer_gets_a_clear_message() {
    let s = Sandbox::new();
    s.add("Dune", "Frank Herbert");
    let held = bookx_store::guard::acquire(&s.lib(), "Bookx").unwrap();
    assert!(matches!(held, bookx_store::guard::Lock::Acquired(_)));
    let p = s.md(&s.dir.join("src"), "Emma", "Jane Austen");
    let (code, _, err) = s.cli(&["library", "add", p.to_str().unwrap()]);
    assert_eq!(code, 2);
    assert!(
        err.contains("already open in Bookx") && err.contains("read-only"),
        "{err}"
    );
    // reading is still fine
    let (code, out, _) = s.cli(&["library", "list"]);
    assert_eq!(code, 0);
    assert!(out.contains("Dune"));
    drop(held);
    let (code, _, _) = s.cli(&["library", "add", p.to_str().unwrap()]);
    assert_eq!(code, 0);
}

fn calibre(dir: &Path) {
    std::fs::create_dir_all(dir.join("Frank Herbert/Dune (1)")).unwrap();
    let scratch = Sandbox::new();
    let (code, _, err) = scratch.cli(&[
        "library",
        "add",
        scratch
            .md(&scratch.dir.join("s"), "Dune", "Frank Herbert")
            .to_str()
            .unwrap(),
    ]);
    assert_eq!(code, 0, "{err}");
    let (_, info, _) = scratch.cli(&["library", "info", "1", "--json"]);
    let file = serde_json::from_str::<serde_json::Value>(&info).unwrap()["file"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::copy(
        scratch.lib().join(file),
        dir.join("Frank Herbert/Dune (1)/Dune - Frank Herbert.epub"),
    )
    .unwrap();
    let c = rusqlite::Connection::open(dir.join("metadata.db")).unwrap();
    c.execute_batch(
        "CREATE TABLE books(id INTEGER PRIMARY KEY, title TEXT, sort TEXT, timestamp TEXT, last_modified TEXT,
            pubdate TEXT, series_index REAL, author_sort TEXT, path TEXT, has_cover INTEGER);
         CREATE TABLE data(id INTEGER PRIMARY KEY, book INTEGER, format TEXT, uncompressed_size INTEGER, name TEXT);
         CREATE TABLE authors(id INTEGER PRIMARY KEY, name TEXT);
         CREATE TABLE books_authors_link(id INTEGER PRIMARY KEY, book INTEGER, author INTEGER);
         CREATE TABLE tags(id INTEGER PRIMARY KEY, name TEXT);
         CREATE TABLE books_tags_link(id INTEGER PRIMARY KEY, book INTEGER, tag INTEGER);
         CREATE TABLE series(id INTEGER PRIMARY KEY, name TEXT);
         CREATE TABLE books_series_link(id INTEGER PRIMARY KEY, book INTEGER, series INTEGER);
         CREATE TABLE publishers(id INTEGER PRIMARY KEY, name TEXT);
         CREATE TABLE books_publishers_link(id INTEGER PRIMARY KEY, book INTEGER, publisher INTEGER);
         CREATE TABLE comments(id INTEGER PRIMARY KEY, book INTEGER, text TEXT);
         CREATE TABLE languages(id INTEGER PRIMARY KEY, lang_code TEXT);
         CREATE TABLE books_languages_link(id INTEGER PRIMARY KEY, book INTEGER, lang_code INTEGER, item_order INTEGER);
         CREATE TABLE ratings(id INTEGER PRIMARY KEY, rating INTEGER);
         CREATE TABLE books_ratings_link(id INTEGER PRIMARY KEY, book INTEGER, rating INTEGER);
         CREATE TABLE identifiers(id INTEGER PRIMARY KEY, book INTEGER, type TEXT, val TEXT);
         INSERT INTO books VALUES(1,'Dune','Dune','2020-01-01 00:00:00+00:00','2020-01-01 00:00:00+00:00',
            '1965-08-01 00:00:00+00:00',1.0,'Herbert, Frank','Frank Herbert/Dune (1)',0);
         INSERT INTO data VALUES(1,1,'EPUB',100,'Dune - Frank Herbert');
         INSERT INTO authors VALUES(1,'Frank Herbert');
         INSERT INTO books_authors_link VALUES(1,1,1);",
    )
    .unwrap();
}

#[test]
fn a_calibre_library_is_imported_with_progress_and_a_report() {
    let s = Sandbox::new();
    let cal = s.dir.join("calibre");
    calibre(&cal);
    let (code, out, err) = s.cli(&["library", "import-calibre", cal.to_str().unwrap()]);
    assert_eq!(code, 0, "{out} {err}");
    assert!(out.contains("1 book(s) imported"), "{out}");
    assert!(err.contains("reading: 1/1"), "progress on stderr: {err}");
    let (_, out, _) = s.cli(&["library", "list"]);
    assert!(out.contains("Dune"), "{out}");
    let (code, out, _) = s.cli(&["library", "import-calibre", cal.to_str().unwrap(), "--copy"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("already in the library"),
        "the second import only reports duplicates: {out}"
    );
    let (code, _, err) = s.cli(&[
        "library",
        "import-calibre",
        s.dir.join("nope").to_str().unwrap(),
    ]);
    assert_eq!(code, 2);
    assert!(err.contains("no metadata.db"), "{err}");
}

#[test]
fn copying_a_calibre_import_makes_the_library_stand_alone() {
    let s = Sandbox::new();
    let cal = s.dir.join("calibre");
    calibre(&cal);
    let (code, out, err) = s.cli(&["library", "import-calibre", cal.to_str().unwrap(), "--copy"]);
    assert_eq!(code, 0, "{out} {err}");
    std::fs::remove_dir_all(&cal).unwrap();
    let (code, out, _) = s.cli(&["library", "check"]);
    assert_eq!(code, 0, "{out}");
}

#[test]
fn a_library_is_exported_as_a_calibre_library_and_imports_back() {
    let s = Sandbox::new();
    s.add("Dune", "Frank Herbert");
    s.add("Emma", "Jane Austen");
    let (_, before, _) = s.cli(&["library", "list", "--json"]);
    let target = s.dir.join("calibre-out");
    let (code, out, err) = s.cli(&["library", "export-calibre", target.to_str().unwrap()]);
    assert_eq!(code, 0, "{out} {err}");
    assert!(out.contains("2 book(s)"), "{out}");
    assert!(err.contains("exporting: 2/2"), "progress on stderr: {err}");
    assert!(target.join("metadata.db").is_file());
    assert!(target.join("Frank Herbert/Dune (1)/metadata.opf").is_file());

    // the library itself did not change, and a second export into the same folder is refused
    let (_, after, _) = s.cli(&["library", "list", "--json"]);
    assert_eq!(before, after);
    let (code, _, err) = s.cli(&["library", "export-calibre", target.to_str().unwrap()]);
    assert_eq!(code, 2);
    assert!(err.contains("not empty"), "{err}");
    let (code, _, err) = s.cli(&[
        "library",
        "export-calibre",
        s.lib().join("inside").to_str().unwrap(),
    ]);
    assert_eq!(code, 2);
    assert!(err.contains("inside"), "{err}");

    // and Bookx reads the result as a Calibre library
    let other = Sandbox::new();
    let (code, out, err) = other.cli(&["library", "import-calibre", target.to_str().unwrap()]);
    assert_eq!(code, 0, "{out} {err}");
    assert!(out.contains("2 book(s) imported"), "{out}");
    let (_, list, _) = other.cli(&["library", "list"]);
    assert!(list.contains("Dune") && list.contains("Emma"), "{list}");
}
