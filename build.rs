//! Writes the table of embedded interface languages (`src/i18n.rs` includes it).
//!
//! Normal builds embed every `po/<code>.po`. With the feature `reviewed-locales-only` (the Flathub build:
//! its policy refuses machine translations) only English and the codes in `po/reviewed-locales.txt` are
//! embedded; the other `.po` files are not read, so none of their text ends up in the binary.

use std::{env, fs, path::Path};

/// `(code, name in its own language)` of every language that has a `po/<code>.po`.
const ALL: [(&str, &str); 4] = [
    ("es", "Español"),
    ("de", "Deutsch"),
    ("fr", "Français"),
    ("hi", "हिन्दी"),
];

fn main() {
    let root = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let list = Path::new(&root).join("po/reviewed-locales.txt");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=po/reviewed-locales.txt");
    println!("cargo:rerun-if-changed=po");

    let gate = env::var_os("CARGO_FEATURE_REVIEWED_LOCALES_ONLY").is_some();
    let reviewed: Vec<String> = fs::read_to_string(&list)
        .unwrap_or_default()
        .lines()
        .map(|l| l.split('#').next().unwrap_or("").trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    for code in &reviewed {
        assert!(
            ALL.iter().any(|(c, _)| c == code),
            "po/reviewed-locales.txt lists `{code}`, which build.rs does not know"
        );
    }

    // only the tests read the gate (the table below is already filtered), so the app build sees it unused
    let mut out = String::from("#[allow(dead_code)]\npub const REVIEWED_ONLY: bool = ");
    out.push_str(if gate { "true;\n" } else { "false;\n" });
    out.push_str("pub const LANGUAGES: &[(&str, &str, Option<&str>)] = &[\n");
    out.push_str("    (\"en\", \"English\", None),\n");
    for (code, name) in ALL {
        if gate && !reviewed.iter().any(|r| r == code) {
            continue;
        }
        out.push_str(&format!(
            "    (\"{code}\", \"{name}\", Some(include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/po/{code}.po\")))),\n"
        ));
    }
    out.push_str("];\n");
    let dest = Path::new(&env::var("OUT_DIR").expect("OUT_DIR")).join("languages.rs");
    fs::write(dest, out).expect("write languages.rs");
}
