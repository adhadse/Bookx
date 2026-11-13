//! `library formats / add-format / remove-format / merge-formats / authors / rename-author`.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use bookx_cli::{run, Env};

struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new() -> Sandbox {
        let dir = std::env::temp_dir().join(format!("bookx-cli-cat-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Sandbox { dir }
    }
    fn cli(&self, args: &[&str]) -> (i32, String, String) {
        let mut v = vec!["bookx-cli".to_string()];
        v.extend(args.iter().map(|s| s.to_string()));
        let env = Env {
            default_library: self.dir.join("lib"),
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
    fn add(&self, title: &str, author: &str) {
        let src = self.dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let p = src.join(format!("{title}.md"));
        std::fs::write(
            &p,
            format!("---\ntitle: \"{title}\"\nauthor: \"{author}\"\n---\n# One\n\nText.\n"),
        )
        .unwrap();
        let (code, out, err) =
            self.cli(&["library", "add", "--allow-duplicates", p.to_str().unwrap()]);
        assert_eq!(code, 0, "{out} {err}");
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn adding_listing_and_removing_a_format_and_undoing_each() {
    let s = Sandbox::new();
    s.add("Dune", "Frank Herbert");
    let azw = s.dir.join("Dune.azw3");
    std::fs::write(&azw, b"AZW3").unwrap();
    let (c, out, err) = s.cli(&["library", "add-format", "1", azw.to_str().unwrap()]);
    assert_eq!(c, 0, "{err}");
    assert!(out.contains("added azw3"));
    let (_, out, _) = s.cli(&["library", "formats", "1"]);
    assert!(out.contains("epub (main)") && out.contains("azw3"), "{out}");
    let (_, info, _) = s.cli(&["library", "info", "1"]);
    assert!(info.contains("Formats:     epub, azw3"), "{info}");
    let (_, json, _) = s.cli(&["library", "list", "--json"]);
    assert!(json.contains("\"azw3\""));
    let (c, _, err) = s.cli(&["library", "remove-format", "1", "azw3"]);
    assert_eq!(c, 0, "{err}");
    assert!(!s.cli(&["library", "formats", "1"]).1.contains("azw3"));
    assert_eq!(s.cli(&["library", "undo"]).0, 0);
    assert!(
        s.cli(&["library", "formats", "1"]).1.contains("azw3"),
        "undo puts it back"
    );
    let (c, _, err) = s.cli(&["library", "remove-format", "1", "mobi"]);
    assert_eq!(c, 2);
    assert!(err.contains("no further mobi"));
}

#[test]
fn merge_formats_previews_merges_and_undoes() {
    let s = Sandbox::new();
    s.add("Dune", "Frank Herbert");
    s.add("Dune Two", "Frank Herbert");
    // same format twice: refused with the reason
    let (c, out, err) = s.cli(&["library", "merge-formats", "1", "2"]);
    assert_ne!(c, 0, "{out}");
    assert!(out.contains("skip  #2") && err.contains("nothing to merge"));
    // make the second book an AZW3 file
    let (_, dump, _) = s.cli(&["library", "dump"]);
    let v: serde_json::Value = serde_json::from_str(&dump).unwrap();
    let file = v["books"][1]["file"].as_str().unwrap().to_string();
    let lib = s.dir.join("lib");
    let new = file.replace(".epub", ".azw3");
    std::fs::rename(lib.join(&file), lib.join(&new)).unwrap();
    let db = lib.join("bookx-library.db");
    let c = rusqlite::Connection::open(&db).unwrap();
    let data: String = c
        .query_row("SELECT data FROM books WHERE id = 2", [], |r| r.get(0))
        .unwrap();
    let fixed = data
        .replace(&file.replace('/', "\\/"), &new)
        .replace(&file, &new);
    c.execute("UPDATE books SET data = ?1 WHERE id = 2", [fixed])
        .unwrap();
    drop(c);

    let (c, out, err) = s.cli(&["library", "merge-formats", "1", "2", "--dry-run"]);
    assert_eq!(c, 0, "{err}");
    assert!(out.contains("result: Dune: epub + azw3"), "{out}");
    assert!(
        s.cli(&["library", "list"]).1.contains("Dune Two"),
        "a dry run changes nothing"
    );
    let (c, out, err) = s.cli(&["library", "merge-formats", "1", "2"]);
    assert_eq!(c, 0, "{err}");
    assert!(out.contains("merged 1 book(s) into #1"));
    assert!(!s.cli(&["library", "list"]).1.contains("Dune Two"));
    assert!(s.cli(&["library", "formats", "1"]).1.contains("azw3"));
    assert!(s
        .cli(&["library", "history"])
        .1
        .contains("Merged 1 books as formats"));
    assert_eq!(s.cli(&["library", "undo"]).0, 0);
    assert!(s.cli(&["library", "list"]).1.contains("Dune Two"));
}

#[test]
fn authors_are_grouped_and_renamed_with_a_preview_and_one_undo() {
    let s = Sandbox::new();
    s.add("One", "Mark Twain");
    s.add("Two", "Samuel Clemens");
    s.add("Three", "Someone Else");
    let (c, out, _) = s.cli(&[
        "library",
        "rename-author",
        "Samuel Clemens",
        "--to",
        "Mark Twain",
        "--dry-run",
    ]);
    assert_eq!(c, 0);
    assert!(out.contains("1 book will list \"Mark Twain\""), "{out}");
    assert!(s.cli(&["library", "authors"]).1.contains("Samuel Clemens"));
    let (c, out, err) = s.cli(&[
        "library",
        "rename-author",
        "Samuel Clemens",
        "--to",
        "Mark Twain",
    ]);
    assert_eq!(c, 0, "{err}");
    assert!(out.contains("renamed on 1 book(s)"));
    let (_, authors, _) = s.cli(&["library", "authors"]);
    assert!(authors.contains("Mark Twain  (2 book(s))"), "{authors}");
    assert!(!authors.contains("Samuel Clemens"));
    assert_eq!(s.cli(&["library", "undo"]).0, 0);
    let (_, authors, _) = s.cli(&["library", "authors", "--json"]);
    // the alias is remembered, so the old spelling still groups under the name
    assert!(authors.contains("Mark Twain"));
    let (_, list, _) = s.cli(&["library", "list"]);
    assert!(
        list.contains("Samuel Clemens"),
        "the books are back to the old name: {list}"
    );
}
