//! `bookx-cli highlights repair` on a library whose highlights were all stored yellow and plain.
//! Each test has its own sandbox, so they run in parallel.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use bookx_cli::{Env, run};
use bookx_core::library::Highlight;
use chrono::{TimeZone, Utc};

const SIDECAR: &str = include_str!("../../bookx-serve/tests/fixtures/koreader_all_looks.lua");

struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new() -> Sandbox {
        let dir = std::env::temp_dir().join(format!("bookx-cli-hl-{}", uuid::Uuid::new_v4()));
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
    /// A library with one book whose 36 highlights are all yellow and plain, and a device folder.
    fn seeded(&self) -> PathBuf {
        let src = self.dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let p = src.join("Looks.md");
        std::fs::write(
            &p,
            "---\ntitle: \"Looks of Highlights\"\nauthor: \"Ann Author\"\n---\n# One\n\nText.\n",
        )
        .unwrap();
        let (code, out, err) = self.cli(&["library", "add", p.to_str().unwrap()]);
        assert_eq!(code, 0, "{out} {err}");
        let mut opened = bookx_store::open_library(&self.lib()).unwrap();
        let id = opened.library.books[0].id;
        let rows =
            bookx_serve::look_repair::truths_from_sidecar(SIDECAR, "Looks.sdr", "KOReader sidecar");
        let book = opened.library.get_mut(id).unwrap();
        for (i, r) in rows.iter().enumerate() {
            book.highlights.push(Highlight {
                id: format!("bx-{i}"),
                spine: 0,
                start: i as i32,
                end: i as i32 + 3,
                color: "yellow".into(),
                style: String::new(),
                note: String::new(),
                text: r.text.clone(),
                created: Utc.timestamp_opt(r.created.unwrap(), 0).unwrap(),
            });
        }
        opened.library.save().unwrap();
        let dev = self.dir.join("device");
        std::fs::create_dir_all(dev.join("Looks.sdr")).unwrap();
        std::fs::write(dev.join("Looks.sdr/metadata.epub.lua"), SIDECAR).unwrap();
        dev
    }
    fn colours(&self) -> Vec<(String, String)> {
        let opened = bookx_store::open_library(&self.lib()).unwrap();
        opened.library.books[0]
            .highlights
            .iter()
            .map(|h| (h.color.clone(), h.style.clone()))
            .collect()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn a_dry_run_reports_and_changes_nothing_and_apply_fixes_once() {
    let s = Sandbox::new();
    let dev = s.seeded();
    let cfg = s.dir.join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    let args = |extra: &[&'static str]| {
        let mut v: Vec<String> = ["highlights", "repair", "--config-dir"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        v.push(cfg.display().to_string());
        v.push("--from".into());
        v.push(dev.display().to_string());
        v.extend(extra.iter().map(|s| s.to_string()));
        v
    };
    let run_cli = |extra: &[&'static str]| {
        let a = args(extra);
        let refs: Vec<&str> = a.iter().map(String::as_str).collect();
        s.cli(&refs)
    };

    let (code, out, err) = run_cli(&[]);
    assert_eq!(code, 0, "{out} {err}");
    assert!(out.contains("35 highlight(s) corrected"), "{out}");
    assert!(out.contains("nothing was changed"), "{out}");
    assert!(
        s.colours()
            .iter()
            .all(|c| c == &("yellow".into(), "".into())),
        "a dry run changes nothing"
    );

    let (code, out, err) = run_cli(&["--apply"]);
    assert_eq!(code, 0, "{out} {err}");
    assert!(out.contains("35 highlight(s) changed"), "{out}");
    let got = s.colours();
    assert_eq!(got[0], ("red".to_string(), "background".to_string()));
    assert_eq!(got[1], ("red".to_string(), "underline".to_string()));
    assert_eq!(got[3], ("red".to_string(), "invert".to_string()));
    assert_eq!(got[35], ("gray".to_string(), "invert".to_string()));

    // again: nothing left to do
    let (code, out, _) = run_cli(&["--apply"]);
    assert_eq!(code, 0);
    assert!(out.contains("0 highlight(s) changed"), "{out}");

    // JSON for scripts
    let (code, out, _) = run_cli(&["--json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["applied"], false);
    assert_eq!(v["report"]["fixed"], 0);
}

#[test]
fn without_a_source_it_says_so() {
    let s = Sandbox::new();
    s.seeded();
    let cfg = s.dir.join("empty-cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    let (code, out, err) = s.cli(&[
        "highlights",
        "repair",
        "--config-dir",
        cfg.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{out} {err}");
    assert!(out.contains("no source"), "{out}");
}

#[test]
fn apply_and_dry_run_together_are_refused() {
    let s = Sandbox::new();
    s.seeded();
    let (code, _, err) = s.cli(&["highlights", "repair", "--apply", "--dry-run"]);
    assert_eq!(code, 2, "{err}");
}
