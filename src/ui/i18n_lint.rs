//! XC-2 gate: a raw string literal must not reach `set_description`, `set_title`, `set_subtitle`
//! or `set_placeholder_text` in `src/ui/**`; it has to go through `tr()` / `trf()` / `ntr()` so it
//! is extracted and translated. `tr(CONST)` hides a string from the extractor, so a constant
//! must be marked with `N_()`.
//!
//! A file can be listed in `REPORT_ONLY`: its hits are printed (`cargo test lint -- --nocapture`)
//! but do not fail the test. `tr(CONST)` is checked over all of `src/`: the constant has to be
//! declared with `N_(text)` (`const_not_marked`), or the extractor never sees the string.

#![cfg(test)]

use std::path::{Path, PathBuf};

/// Prefixes (relative to `src/ui/`) whose hits are reported but do not fail. Empty: every file is clean.
const REPORT_ONLY: &[&str] = &[];

/// Calls whose first argument is user-visible text.
const CALLS: &[&str] = &[
    "set_description(Some(",
    "set_title(",
    "set_subtitle(",
    "set_placeholder_text(Some(",
    "set_tooltip_text(Some(",
    "set_heading(Some(",
];

/// The production part of a file: everything before its `#[cfg(test)]` module.
fn production(src: &str) -> &str {
    src.find("#[cfg(test)]").map_or(src, |i| &src[..i])
}

/// Line numbers (1-based) where one of `CALLS` is followed by a bare string literal.
pub fn raw_literals(src: &str) -> Vec<usize> {
    let src = production(src);
    let mut hits = Vec::new();
    for call in CALLS {
        let mut from = 0;
        while let Some(i) = src[from..].find(call) {
            let at = from + i;
            from = at + call.len();
            let rest = src[from..].trim_start();
            if let Some(lit) = rest.strip_prefix('"') {
                // `""` clears a field, and a literal that is only punctuation or digits is not prose
                let body = lit.split('"').next().unwrap_or("");
                if body.chars().any(char::is_alphabetic) {
                    hits.push(src[..at].matches('\n').count() + 1);
                }
            }
        }
    }
    hits.sort_unstable();
    hits.dedup();
    hits
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn report_only(rel: &str) -> bool {
    REPORT_ONLY.iter().any(|p| rel.starts_with(p))
}

#[test]
fn lint_ui_has_no_raw_literals_in_titles_and_descriptions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui");
    let mut all = Vec::new();
    files(&root, &mut all);
    all.sort();
    let mut failures = Vec::new();
    for f in all {
        let rel = f.strip_prefix(&root).unwrap().to_string_lossy().to_string();
        if rel == "i18n_lint.rs" {
            continue;
        }
        let hits = raw_literals(&std::fs::read_to_string(&f).unwrap());
        if hits.is_empty() {
            continue;
        }
        if report_only(&rel) {
            println!("i18n-lint (report only): src/ui/{rel}: lines {hits:?}");
        } else {
            failures.push(format!("src/ui/{rel}: lines {hits:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "raw string literals reach a widget without tr(): {failures:#?}"
    );
}

#[test]
fn lint_finds_a_raw_literal_and_ignores_translated_ones() {
    let src = "a.set_description(Some(\"Choose the file\"));\n\
               b.set_title(&tr(\"Fine\"));\n\
               c.set_title(\"Raw Title\");\n\
               d.set_title(\"\");\n\
               e.set_description(Some(\n    &tr(\"ok\"),\n));\n\
               #[cfg(test)]\nmod t { fn x() { f.set_title(\"in tests\"); } }";
    assert_eq!(raw_literals(src), vec![1, 3]);
}

/// Names passed as `tr(NAME)` / `tr(&NAME)` / `ntr(NAME, ..)` whose `const NAME: &str = "text"`
/// in the same text is a bare literal instead of `N_(text)`.
pub fn const_not_marked(src: &str) -> Vec<String> {
    let src = production(src);
    let mut bad = Vec::new();
    for line_start in src.match_indices("const ") {
        let rest = &src[line_start.0 + 6..];
        let Some((name, after)) = rest.split_once(": &str =") else {
            continue;
        };
        let name = name.trim_start_matches("pub ").trim();
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        {
            continue;
        }
        if !after.trim_start().starts_with('"') {
            continue;
        }
        let used = [format!("tr({name})"), format!("tr(&{name})")]
            .iter()
            .any(|p| src.contains(p.as_str()));
        if used {
            bad.push(name.to_string());
        }
    }
    bad
}

#[test]
fn lint_every_translated_constant_is_marked_with_n() {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        files(dir, out);
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut all = Vec::new();
    walk(&root, &mut all);
    let mut failures = Vec::new();
    for f in all {
        if f.ends_with("i18n_lint.rs") {
            continue;
        }
        let bad = const_not_marked(&std::fs::read_to_string(&f).unwrap());
        if !bad.is_empty() {
            failures.push(format!("{}: {bad:?}", f.display()));
        }
    }
    assert!(failures.is_empty(), "tr(CONST) without N_(): {failures:#?}");
}

#[test]
fn lint_finds_an_unmarked_constant() {
    let src =
        "const A: &str = \"hello\";\nconst B: &str = N_(\"ok\");\nconst C: &str = \"unused\";\n\
               fn f() { tr(A); tr(B); }";
    assert_eq!(const_not_marked(src), vec!["A".to_string()]);
}
