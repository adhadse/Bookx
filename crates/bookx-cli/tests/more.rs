//! `catalog`, `polish`, `fonts`, `saved-search` and `plugin run`. Each test has its own sandbox.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use bookx_cli::{run, Env};
use bookx_core::archive::Archive;

struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new() -> Sandbox {
        let dir = std::env::temp_dir().join(format!("bookx-cli-more-{}", uuid::Uuid::new_v4()));
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
    fn add(&self, title: &str, author: &str) {
        let src = self.dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let p = src.join(format!("{title}.md"));
        std::fs::write(
            &p,
            format!(
                "---\ntitle: \"{title}\"\nauthor: \"{author}\"\n---\n# One\n\nText of {title}.\n"
            ),
        )
        .unwrap();
        let (code, out, err) = self.cli(&["library", "add", p.to_str().unwrap()]);
        assert_eq!(code, 0, "{out} {err}");
    }
    /// Path of the file of library book `id`.
    fn book_file(&self, id: u64) -> PathBuf {
        let (_, info, _) = self.cli(&["library", "info", &id.to_string(), "--json"]);
        let f = serde_json::from_str::<serde_json::Value>(&info).unwrap()["file"]
            .as_str()
            .unwrap()
            .to_string();
        self.lib().join(f)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn example_plugin() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins/trim-whitespace")
}

/// Adds trailing spaces to the first chapter of the book file.
fn dirty(file: &Path) -> String {
    let mut a = Archive::open(file).unwrap();
    let name = a
        .names()
        .iter()
        .find(|n| n.ends_with(".xhtml") && !n.contains("nav") && !n.contains("cover"))
        .unwrap()
        .clone();
    let t = a.get_string(&name).unwrap().replace("</p>", "</p>   ");
    a.set_string(&name, &t);
    a.save(file).unwrap();
    name
}

#[test]
fn catalog_prints_csv_and_json_with_chosen_columns() {
    let s = Sandbox::new();
    s.add("Dune", "Frank Herbert");
    s.add("=Evil", "A, B");
    let (code, out, err) = s.cli(&["catalog", "--columns", "id,title,authors"]);
    assert_eq!(code, 0, "{err}");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "id,title,authors");
    assert_eq!(lines.len(), 3, "{out}");
    assert!(out.contains("Dune") && out.contains("Frank Herbert"));
    assert!(out.contains("'=Evil"), "formula guard: {out}");
    let (_, out, _) = s.cli(&[
        "catalog",
        "--format",
        "json",
        "--columns",
        "title",
        "title:dune",
    ]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["title"], "Dune");
    let (code, _, err) = s.cli(&["catalog", "--columns", "title,nope"]);
    assert_eq!(code, 2);
    assert!(err.contains("unknown column"), "{err}");
    let (code, _, _) = s.cli(&["catalog", "title:zzzz"]);
    assert_eq!(code, 1, "no match is exit 1");
}

#[test]
fn polish_and_fonts_work_on_a_file() {
    let s = Sandbox::new();
    s.add("Dune", "Frank Herbert");
    let book = s.book_file(1);
    let out_file = s.dir.join("polished.epub");
    let (code, out, err) = s.cli(&["polish", book.to_str().unwrap(), out_file.to_str().unwrap()]);
    assert!(code == 0 || code == 1, "{out} {err}");
    assert!(out.contains("wrote"), "{out}");
    assert!(Archive::open(&out_file).is_ok());
    let (code, _, err) = s.cli(&["polish", "/nonexistent/x.epub", out_file.to_str().unwrap()]);
    assert_eq!(code, 2, "{err}");

    let (code, out, err) = s.cli(&["fonts", book.to_str().unwrap()]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("no embedded fonts"), "{out}");
    let (_, out, _) = s.cli(&["fonts", "--json", book.to_str().unwrap()]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v["fonts"].as_array().unwrap().is_empty());
}

#[test]
fn saved_search_lists_and_runs() {
    let s = Sandbox::new();
    s.add("Dune", "Frank Herbert");
    s.add("Emma", "Jane Austen");
    let cfg = s.dir.join("config.json");
    std::fs::write(
        &cfg,
        r#"{"saved_searches":[{"name":"Herbert","query":"author:herbert"},{"name":"Both","query":"search:Herbert OR author:austen"}]}"#,
    )
    .unwrap();
    let c = cfg.to_str().unwrap();
    let (code, out, _) = s.cli(&["saved-search", "list", "--config", c]);
    assert_eq!(code, 0);
    assert!(out.contains("Herbert\tauthor:herbert"), "{out}");
    let (code, out, err) = s.cli(&["saved-search", "run", "herbert", "--config", c, "--json"]);
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["title"], "Dune");
    let (code, _, err) = s.cli(&["saved-search", "run", "Both", "--config", c]);
    assert_eq!(code, 2);
    assert!(err.contains("refers to other saved searches"), "{err}");
    let (code, _, err) = s.cli(&["saved-search", "run", "Nope", "--config", c]);
    assert_eq!(code, 2);
    assert!(err.contains("no saved search"), "{err}");
}

#[test]
fn plugin_run_reports_first_and_writes_only_with_apply() {
    let s = Sandbox::new();
    s.add("Dune", "Frank Herbert");
    s.add("Emma", "Jane Austen");
    let file = s.book_file(1);
    let chapter = dirty(&file);
    let dir = example_plugin();
    let d = dir.to_str().unwrap();

    let (code, out, err) = s.cli(&["plugin", "run", d, "trim"]);
    assert_eq!(code, 0, "{out} {err}");
    assert!(out.contains("1 book(s) would change"), "{out}");
    assert!(out.contains("nothing written"), "{out}");
    assert!(
        Archive::open(&file)
            .unwrap()
            .get_string(&chapter)
            .unwrap()
            .contains("</p>   "),
        "a dry run must not touch the book"
    );

    let (code, out, err) = s.cli(&["plugin", "run", d, "trim", "--apply", "--json"]);
    assert_eq!(code, 0, "{out} {err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["applied"], true);
    assert_eq!(v["changed"], 1);
    assert!(!Archive::open(&file)
        .unwrap()
        .get_string(&chapter)
        .unwrap()
        .contains("</p>   "));

    // the query narrows the books
    dirty(&file);
    let (_, out, _) = s.cli(&["plugin", "run", d, "trim", "author:austen"]);
    assert!(out.contains("0 book(s) would change"), "{out}");

    let (code, _, err) = s.cli(&["plugin", "run", d, "nope"]);
    assert_eq!(code, 1);
    let _ = err;
}

#[test]
fn plugin_run_rejects_a_bad_plugin_folder() {
    let s = Sandbox::new();
    s.add("Dune", "Frank Herbert");
    let (code, _, err) = s.cli(&["plugin", "run", "/nonexistent-plugin", "x"]);
    assert_eq!(code, 2, "{err}");
}

#[test]
fn catalog_makes_an_epub_and_can_show_custom_columns() {
    let s = Sandbox::new();
    s.add("Dune", "Frank Herbert");
    s.add("Emma", "Jane Austen");
    let (code, _, err) = s.cli(&["catalog", "--format", "epub"]);
    assert_eq!(code, 2);
    assert!(err.contains("--output"), "{err}");
    let file = s.dir.join("catalog.epub");
    let (code, out, err) = s.cli(&[
        "catalog",
        "--format",
        "epub",
        "--output",
        file.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("2 book(s)"), "{out}");
    let bytes = std::fs::read(&file).unwrap();
    assert!(bytes.starts_with(b"PK"));
    let (code, ..) = s.cli(&["check", file.to_str().unwrap()]);
    assert!(code == 0 || code == 1, "the catalogue is a readable book");
    // custom columns can be listed
    s.cli(&["library", "columns", "add", "Pages", "--type", "integer"]);
    s.cli(&["library", "columns", "set", "1", "pages", "412"]);
    let (_, out, _) = s.cli(&["catalog", "--columns", "title,#pages"]);
    assert!(out.lines().any(|l| l == "Dune,412"), "{out}");
}
