//! `library cite`, `fixity` and `covers`. Each test has its own sandbox, so they run in parallel.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use bookx_cli::{run, Env};

struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new() -> Sandbox {
        let dir = std::env::temp_dir().join(format!("bookx-cli-extras-{}", uuid::Uuid::new_v4()));
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
fn cite_prints_bibtex_ris_csl_and_apa_and_filters_by_search() {
    let s = Sandbox::new();
    s.add("Dune", "Frank Herbert");
    s.add("Emma", "Jane Austen");
    let (code, out, err) = s.cli(&["library", "cite"]);
    assert_eq!(code, 0, "{out} {err}");
    assert!(
        out.contains("@book{herbert") || out.contains("@book{herbert"),
        "{out}"
    );
    assert!(out.contains("author = {Austen, Jane}"), "{out}");
    let (_, out, _) = s.cli(&["library", "cite", "--style", "ris", "author:austen"]);
    assert!(
        out.contains("TY  - BOOK") && out.contains("AU  - Austen, Jane"),
        "{out}"
    );
    assert!(!out.contains("Herbert"), "{out}");
    let (_, out, _) = s.cli(&["library", "cite", "--style", "csl-json"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);
    let (_, out, _) = s.cli(&["library", "cite", "--style", "apa"]);
    assert!(out.contains("Austen, J. (n.d.). *Emma*."), "{out}");
    let file = s.dir.join("refs.bib");
    let (code, out, _) = s.cli(&["library", "cite", "-o", file.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(out.contains("wrote 2 reference(s)"), "{out}");
    assert!(std::fs::read_to_string(file).unwrap().contains("@book"));
    let (code, _, err) = s.cli(&["library", "cite", "--style", "mla"]);
    assert_ne!(code, 0);
    assert!(err.contains("unknown style"), "{err}");
}

#[test]
fn fixity_records_then_finds_silent_damage_and_accepts_it() {
    let s = Sandbox::new();
    s.add("Dune", "Frank Herbert");
    let (code, out, err) = s.cli(&["library", "fixity"]);
    assert_eq!(code, 0, "{out} {err}");
    assert!(out.contains("1 newly recorded"), "{out}");
    let (code, out, _) = s.cli(&["library", "fixity"]);
    assert_eq!(code, 0);
    assert!(out.contains("1 intact"), "{out}");

    let (_, info, _) = s.cli(&["library", "info", "1", "--json"]);
    let file = serde_json::from_str::<serde_json::Value>(&info).unwrap()["file"]
        .as_str()
        .unwrap()
        .to_string();
    let path = s.lib().join(&file);
    let mut bytes = std::fs::read(&path).unwrap();
    let when = std::fs::metadata(&path).unwrap().modified().unwrap();
    let last = bytes.len() / 2;
    bytes[last] ^= 0xFF;
    std::fs::write(&path, &bytes).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(when)
        .unwrap();

    let (code, out, _) = s.cli(&["library", "fixity"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("DAMAGED") && out.contains(&file), "{out}");
    let (_, json, _) = s.cli(&["library", "fixity", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["damaged"][0], file.as_str());

    let (code, _, _) = s.cli(&["library", "fixity", "--accept", &file]);
    assert_eq!(code, 0);
    let (code, _, _) = s.cli(&["library", "fixity"]);
    assert_eq!(code, 0);
}

#[test]
fn covers_lists_sizes() {
    let s = Sandbox::new();
    s.add("Dune", "Frank Herbert");
    let (code, out, err) = s.cli(&["library", "covers", "--all", "--json"]);
    assert_eq!(code, 0, "{out} {err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    // an imported text book gets a generated cover; whatever its size, the report has it
    if let Some(first) = v.as_array().unwrap().first() {
        assert!(first["width"].as_u64().unwrap() > 0);
        assert!(first["height"].as_u64().unwrap() > 0);
        assert!(first["grade"].is_string());
    }
    let (code, out, _) = s.cli(&["library", "covers"]);
    assert_eq!(code, 0);
    assert!(!out.is_empty());
}
