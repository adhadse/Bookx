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
    // the name of a file chooser filter
    "set_name(Some(",
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

/// Files whose dialogs are not made with `dialog_toast::new_dialog()` yet (X9: Ctrl+W closes a
/// dialog made with it). Remove an entry when its file is converted.
const OWN_DIALOGS: &[&str] = &["dialog_toast.rs", "i18n_lint.rs"];

#[test]
fn lint_dialogs_are_made_with_new_dialog() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui");
    let mut all = Vec::new();
    files(&root, &mut all);
    let mut bad = Vec::new();
    for f in all {
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        if OWN_DIALOGS.contains(&name.as_str()) {
            continue;
        }
        let text = std::fs::read_to_string(&f).unwrap();
        if production(&text).contains("adw::Dialog::new()") {
            bad.push(f.strip_prefix(&root).unwrap().display().to_string());
        }
    }
    assert!(
        bad.is_empty(),
        "use dialog_toast::new_dialog() (Ctrl+W closes it): {bad:?}"
    );
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

// ---- XC-5: header capitalisation of buttons, menu entries, tooltips and alert buttons -----------------

/// Words that stay in lower case inside a header-caps label (HIG "Writing Style").
const SMALL_WORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "of", "in", "on", "to", "for", "with", "from", "as", "at", "by",
    "vs",
];

/// What is wrong with a button, tooltip or menu label: a word that should start with a capital
/// (header caps), or an ending in an ellipsis (D14). `None` when it is fine.
pub fn header_caps_problem(label: &str) -> Option<String> {
    if label.ends_with('…') || label.ends_with("...") {
        return Some("ends in an ellipsis".into());
    }
    for (i, w) in label.split_whitespace().enumerate() {
        let w = w.trim_matches(|c: char| !c.is_alphanumeric());
        let Some(first) = w.chars().next() else {
            continue;
        };
        if !first.is_alphabetic() || w.chars().any(|c| c.is_uppercase()) && !first.is_uppercase() {
            continue;
        }
        if first.is_lowercase() && (i == 0 || !SMALL_WORDS.contains(&w)) {
            return Some(format!("“{w}” should start with a capital"));
        }
    }
    None
}

/// The labels of buttons, tooltips and alert buttons written as string literals passed to `tr` in `src`.
fn control_labels(src: &str) -> Vec<String> {
    const OPENERS: &[&str] = &[
        "Button::with_label(&tr(\"",
        "set_tooltip_text(Some(&tr(\"",
        "set_button_label(Some(&tr(\"",
        "icon_button(\"edit-copy-symbolic\", &tr(\"",
        "icon_button(\"view-refresh-symbolic\", &tr(\"",
    ];
    let src = production(src);
    let mut out = Vec::new();
    for o in OPENERS {
        let mut from = 0;
        while let Some(i) = src[from..].find(o) {
            let start = from + i + o.len();
            from = start;
            if let Some(end) = src[start..].find('"') {
                out.push(src[start..start + end].to_string());
            }
        }
    }
    // alert buttons: `("id", &tr("Label"))`
    let mut from = 0;
    while let Some(i) = src[from..].find(", &tr(\"") {
        let start = from + i + 7;
        from = start;
        let line_start = src[..start].rfind('\n').map_or(0, |n| n + 1);
        if src[line_start..start].contains("(\"") && src[line_start..start].contains("add_respons")
            || src[line_start..start].trim_start().starts_with("(\"")
        {
            if let Some(end) = src[start..].find('"') {
                out.push(src[start..start + end].to_string());
            }
        }
    }
    out
}

#[test]
fn header_caps_rule() {
    assert_eq!(header_caps_problem("Open from Start"), None);
    assert_eq!(header_caps_problem("Accept and Download"), None);
    assert_eq!(header_caps_problem("Mark as Finished"), None);
    assert!(header_caps_problem("Open folder").is_some());
    assert!(header_caps_problem("Copy install command").is_some());
    assert!(header_caps_problem("Choose…").is_some());
    assert_eq!(header_caps_problem("Play"), None);
    assert_eq!(header_caps_problem("Send {} Books"), None);
}

#[test]
fn lint_buttons_and_tooltips_of_the_settings_files_use_header_caps() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui");
    let mut bad = Vec::new();
    let mut seen = 0;
    for f in [
        "preferences.rs",
        "mcp_settings.rs",
        "dict_settings.rs",
        "reading_ruler.rs",
        "tts_settings.rs",
        "library_banner.rs",
    ] {
        let text = std::fs::read_to_string(root.join(f)).unwrap();
        for l in control_labels(&text) {
            seen += 1;
            if let Some(p) = header_caps_problem(&l) {
                bad.push(format!("{f}: {l:?}: {p}"));
            }
        }
    }
    assert!(seen > 10, "the scan found only {seen} labels");
    assert!(bad.is_empty(), "labels without header caps: {bad:#?}");
}

#[test]
fn lint_row_titles_of_the_settings_files_are_short_and_neutral() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui");
    let mut bad = Vec::new();
    let mut seen = 0;
    for f in [
        "preferences.rs",
        "mcp_settings.rs",
        "dict_settings.rs",
        "reading_ruler.rs",
        "tts_settings.rs",
        "library_banner.rs",
    ] {
        let text = std::fs::read_to_string(root.join(f)).unwrap();
        let src = production(&text);
        for o in [".title(tr(\"", "set_title(&tr(\""] {
            let mut from = 0;
            while let Some(i) = src[from..].find(o) {
                let start = from + i + o.len();
                from = start;
                let Some(end) = src[start..].find('"') else {
                    continue;
                };
                let t = &src[start..start + end];
                seen += 1;
                if let Some(p) = super::preferences::title_problem(t) {
                    bad.push(format!("{f}: {t:?}: {p}"));
                }
            }
        }
    }
    assert!(seen > 20, "the scan found only {seen} titles");
    assert!(bad.is_empty(), "titles to fix: {bad:#?}");
}
