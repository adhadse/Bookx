//! Hostile stylesheets and selectors: nesting, floods, unterminated tokens. The parser and the
//! cascade stay linear-ish and never panic or overflow the stack. Fuzz twin:
//! `fuzz/fuzz_targets/css.rs`.

use std::time::{Duration, Instant};

use bookx_text::css::{
    MediaEnv, StyleSheetSet, parse_declaration_text, parse_selector_list, parse_stylesheet,
};

fn quick<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let r = f();
    assert!(
        t.elapsed() < Duration::from_secs(15),
        "{label}: {:?}",
        t.elapsed()
    );
    r
}

fn cases() -> Vec<(&'static str, String)> {
    let n = 200_000;
    vec![
        ("open-braces", "{".repeat(n)),
        ("close-braces", "}".repeat(n)),
        ("nested-media", "@media screen{".repeat(50_000)),
        ("nested-blocks", "a{b{c{d{".repeat(30_000)),
        ("parens", "(".repeat(n)),
        ("brackets", "[".repeat(n)),
        (
            "calc-nest",
            format!(
                "p{{width:{}1{}}}",
                "calc(".repeat(50_000),
                ")".repeat(50_000)
            ),
        ),
        (
            "var-nest",
            format!(
                "p{{color:{}x{}}}",
                "var(--a,".repeat(50_000),
                ")".repeat(50_000)
            ),
        ),
        (
            "unterminated-string",
            format!("p{{content:'{}", "x".repeat(1 << 20)),
        ),
        ("unterminated-comment", format!("/*{}", "x".repeat(1 << 20))),
        (
            "unterminated-url",
            format!("p{{background:url({}", "x".repeat(1 << 20)),
        ),
        (
            "huge-selector",
            format!("{} {{color:red}}", "a ".repeat(100_000)),
        ),
        (
            "selector-list",
            format!("{}{{color:red}}", "a,".repeat(100_000)),
        ),
        ("many-rules", "p{color:red}".repeat(100_000)),
        (
            "many-decls",
            format!("p{{{}}}", "color:red;".repeat(200_000)),
        ),
        (
            "not-nest",
            format!("{}a{} {{x:y}}", ":not(".repeat(10_000), ")".repeat(10_000)),
        ),
        (
            "has-nest",
            format!("{}a{} {{x:y}}", ":is(".repeat(10_000), ")".repeat(10_000)),
        ),
        (
            "nth",
            "li:nth-child(99999999999999999999n+99999999999999999999){x:y}".into(),
        ),
        ("escapes", "\\".repeat(1 << 18)),
        ("nul-and-bom", "\u{feff}a\0{b:c\0}".repeat(10_000)),
        (
            "import-loop",
            "@import 'a.css'; @import url(a.css);".repeat(50_000),
        ),
        ("attr-selectors", "a[b=c]".repeat(50_000) + "{x:y}"),
        (
            "important",
            "p{a:b !important !important !important}".repeat(50_000),
        ),
        (
            "big-numbers",
            "p{width:1e999px;margin:-1e999em;font-size:99999999999999999999pt}".into(),
        ),
    ]
}

#[test]
fn stylesheets_selectors_and_declarations_survive_hostile_text() {
    for (label, css) in cases() {
        let (rules, _) = quick(label, || {
            parse_stylesheet(
                &css,
                MediaEnv {
                    viewport_width: 600.0,
                },
                0,
            )
        });
        assert!(rules.len() <= 1_000_000, "{label}");
        let _ = quick(label, || parse_selector_list(&css));
        let _ = quick(label, || parse_declaration_text(&css));
    }
}

#[test]
fn the_cascade_survives_hostile_sheets() {
    let sheet = |css: &str| {
        quick("cascade", || {
            StyleSheetSet::new().add_author(css).add_user(css)
        })
    };
    for (label, css) in cases() {
        let _ = quick(label, || sheet(&css));
    }
}
