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

/// The release channel from the pre-release part of the version (`0.3.0-beta.1` is `beta`): the same
/// rule as `src/beta.rs`, which a test checks against this constant.
fn channel_of(pre: &str) -> &'static str {
    let p = pre.to_ascii_lowercase();
    if p.is_empty() {
        "stable"
    } else if p.starts_with("alpha") {
        "alpha"
    } else if p.starts_with("beta") {
        "beta"
    } else if p.starts_with("rc") {
        "rc"
    } else {
        "prerelease"
    }
}

/// The short git commit of the build, from `BOOKX_GIT_COMMIT` (packagers without a `.git` set it) or git.
fn git_commit() -> String {
    if let Ok(c) = env::var("BOOKX_GIT_COMMIT") {
        return c.trim().chars().take(12).collect();
    }
    std::process::Command::new("git")
        .args(["rev-parse", "--short=9", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|c| c.chars().all(|ch| ch.is_ascii_hexdigit()))
        .unwrap_or_default()
}

fn main() {
    println!("cargo:rerun-if-env-changed=BOOKX_GIT_COMMIT");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!(
        "cargo:rustc-env=BOOKX_CHANNEL={}",
        channel_of(&env::var("CARGO_PKG_VERSION_PRE").unwrap_or_default())
    );
    println!("cargo:rustc-env=BOOKX_GIT_COMMIT={}", git_commit());
    println!(
        "cargo:rustc-env=BOOKX_PROFILE={}",
        env::var("PROFILE").unwrap_or_default()
    );
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
