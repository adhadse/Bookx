//! Settings import and export (X10), without GTK: Preferences → General → Export Settings / Import Settings.
//!
//! The file is JSON: `{"format": "bookx-settings", "version": 1, "app_version": "…", "settings": {…}}`. It holds
//! the settings of `config.json` **without secrets** (passwords, tokens, passphrases, API keys are blanked, and an
//! import never takes them from the file) and without what belongs to one machine (library folder, window size,
//! favourites by catalogue id). An import is read with a size limit and checked completely before anything
//! changes; the dialog shows which settings would change, and applying keeps the current secrets.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Context, Result};
use serde_json::{Map, Value};

use crate::config::Config;

pub const FORMAT: &str = "bookx-settings";
pub const VERSION: u64 = 1;
/// A settings file larger than this is refused unread.
pub const MAX_BYTES: u64 = 1 << 20;
/// Most settings (top-level keys) a file may carry; the app has about a hundred.
pub const MAX_KEYS: usize = 400;

/// Top-level settings that describe this machine or this catalogue and so are neither exported nor imported.
pub const MACHINE_KEYS: &[&str] = &[
    "library_dir",
    "window_width",
    "window_height",
    "maximized",
    "favourites",
    "sidebar_view",
    "sidebar_open",
];

/// The secrets of `cfg` by their vault key.
fn secrets_of(cfg: &Config) -> BTreeMap<String, String> {
    let mut copy = cfg.clone();
    copy.secret_fields()
        .into_iter()
        .map(|(k, v)| (k, v.clone()))
        .collect()
}

/// `cfg` with every secret blanked.
fn without_secrets(cfg: &Config) -> Config {
    let mut copy = cfg.clone();
    for (_, v) in copy.secret_fields() {
        v.clear();
    }
    copy
}

fn settings_map(cfg: &Config) -> Map<String, Value> {
    let mut m = match serde_json::to_value(without_secrets(cfg)) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    };
    for k in MACHINE_KEYS {
        m.remove(*k);
    }
    m
}

/// The text of an export file.
pub fn export(cfg: &Config) -> String {
    let doc = serde_json::json!({
        "format": FORMAT,
        "version": VERSION,
        "app_version": crate::config::VERSION,
        "settings": Value::Object(settings_map(cfg)),
    });
    serde_json::to_string_pretty(&doc).unwrap_or_default()
}

/// A checked settings file.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub app_version: String,
    pub settings: Map<String, Value>,
}

/// Reads and checks a settings file; nothing is applied. `len` is the file size in bytes (checked before
/// the text is read, by the caller, with [`MAX_BYTES`]; checked again here).
pub fn parse(text: &str) -> Result<Imported> {
    if text.len() as u64 > MAX_BYTES {
        bail!("the file is larger than {} KiB", MAX_BYTES / 1024);
    }
    let v: Value = serde_json::from_str(text).context("the file is not valid JSON")?;
    let Value::Object(mut doc) = v else {
        bail!("the file is not a Bookx settings file");
    };
    if doc.get("format").and_then(Value::as_str) != Some(FORMAT) {
        bail!("the file is not a Bookx settings file");
    }
    match doc.get("version").and_then(Value::as_u64) {
        Some(v) if v <= VERSION => {}
        Some(_) => bail!("the file was made by a newer Bookx; update Bookx to import it"),
        None => bail!("the file has no version"),
    }
    let Some(Value::Object(settings)) = doc.remove("settings") else {
        bail!("the file has no settings");
    };
    if settings.len() > MAX_KEYS {
        bail!("the file has too many settings");
    }
    // the whole must be a valid configuration, or nothing is imported
    serde_json::from_value::<Config>(Value::Object(settings.clone()))
        .context("some settings have a value Bookx does not understand")?;
    Ok(Imported {
        app_version: doc
            .get("app_version")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .take(40)
            .collect(),
        settings,
    })
}

/// What an import would do, for the preview.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preview {
    /// Settings whose value would change.
    pub changed: Vec<String>,
    /// Settings in the file that this Bookx does not know (kept out).
    pub unknown: Vec<String>,
    /// Settings in the file that are never imported (this machine's, or secrets).
    pub skipped: Vec<String>,
}

pub fn preview(current: &Config, imp: &Imported) -> Preview {
    let cur = settings_map(current);
    let known: BTreeSet<String> = match serde_json::to_value(current) {
        Ok(Value::Object(m)) => m.keys().cloned().collect(),
        _ => BTreeSet::new(),
    };
    // what applying would give, compared setting by setting (secrets are blanked on both sides)
    let after = apply(current, imp)
        .map(|c| settings_map(&c))
        .unwrap_or_else(|_| cur.clone());
    let mut p = Preview::default();
    for k in imp.settings.keys() {
        if MACHINE_KEYS.contains(&k.as_str()) {
            p.skipped.push(k.clone());
        } else if !known.contains(k) {
            p.unknown.push(k.clone());
        } else if cur.get(k) != after.get(k) {
            p.changed.push(k.clone());
        }
    }
    p
}

/// The configuration after the import: `current` with the file's settings laid over it, machine settings and
/// secrets kept as they are.
pub fn apply(current: &Config, imp: &Imported) -> Result<Config> {
    let mut m = match serde_json::to_value(current)? {
        Value::Object(m) => m,
        _ => bail!("internal: settings are not an object"),
    };
    for (k, v) in &imp.settings {
        if MACHINE_KEYS.contains(&k.as_str()) || !m.contains_key(k) {
            continue;
        }
        m.insert(k.clone(), v.clone());
    }
    let mut out: Config =
        serde_json::from_value(Value::Object(m)).context("the settings cannot be applied")?;
    // secrets never come from a file: put the current ones back (a new entry has none)
    let keep = secrets_of(current);
    for (k, v) in out.secret_fields() {
        *v = keep.get(&k).cloned().unwrap_or_default();
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        let mut c = Config::default();
        c.share.password = "share-secret-1".into();
        c.readwise_token = "rw-token-abcdef".into();
        c.tts.server.api_key = "sk-very-secret".into();
        c.eink = true;
        c.window_width = 777;
        c.library_dir = "/home/alice/My Books".into();
        c.favourites = vec![1, 2, 3];
        c
    }

    #[test]
    fn an_export_has_no_secrets_and_nothing_of_this_machine() {
        let text = export(&cfg());
        for bad in [
            "share-secret-1",
            "rw-token-abcdef",
            "sk-very-secret",
            "/home/alice",
            "My Books",
        ] {
            assert!(!text.contains(bad), "{bad} leaked");
        }
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["format"], FORMAT);
        assert_eq!(v["settings"]["eink"], true);
        for k in MACHINE_KEYS {
            assert!(v["settings"].get(k).is_none(), "{k}");
        }
    }

    #[test]
    fn export_then_import_round_trips_into_a_fresh_config() {
        let imp = parse(&export(&cfg())).unwrap();
        let fresh = Config::default();
        let p = preview(&fresh, &imp);
        assert!(p.changed.contains(&"eink".to_string()), "{p:?}");
        assert!(p.unknown.is_empty());
        let out = apply(&fresh, &imp).unwrap();
        assert!(out.eink);
        // this machine's settings stay as they were
        assert_eq!(out.window_width, fresh.window_width);
        assert_eq!(out.library_dir, fresh.library_dir);
        assert!(out.favourites.is_empty());
        // and no secret arrived
        assert!(out.share.password.is_empty() && out.readwise_token.is_empty());
    }

    #[test]
    fn an_import_keeps_the_current_secrets_even_when_the_file_carries_some() {
        let mut doc: Value = serde_json::from_str(&export(&Config::default())).unwrap();
        doc["settings"]["readwise_token"] = "stolen-token".into();
        doc["settings"]["share"]["password"] = "stolen-pass".into();
        let imp = parse(&doc.to_string()).unwrap();
        let current = cfg();
        let out = apply(&current, &imp).unwrap();
        assert_eq!(out.readwise_token, "rw-token-abcdef");
        assert_eq!(out.share.password, "share-secret-1");
        let ch = preview(&current, &imp).changed;
        assert!(
            !ch.contains(&"readwise_token".to_string()) && !ch.contains(&"share".to_string()),
            "{ch:?}"
        );
    }

    #[test]
    fn identical_settings_show_no_changes() {
        let c = cfg();
        let imp = parse(&export(&c)).unwrap();
        let p = preview(&c, &imp);
        assert!(p.changed.is_empty(), "{p:?}");
    }

    #[test]
    fn machine_settings_and_unknown_ones_are_listed_not_applied() {
        let mut doc: Value = serde_json::from_str(&export(&Config::default())).unwrap();
        doc["settings"]["library_dir"] = "/elsewhere".into();
        doc["settings"]["future_option"] = true.into();
        let imp = parse(&doc.to_string()).unwrap();
        let cur = Config::default();
        let p = preview(&cur, &imp);
        assert_eq!(p.skipped, vec!["library_dir"]);
        assert_eq!(p.unknown, vec!["future_option"]);
        let out = apply(&cur, &imp).unwrap();
        assert_eq!(out.library_dir, cur.library_dir);
    }

    #[test]
    fn bad_files_are_refused_with_a_reason() {
        let err = |t: &str| format!("{:#}", parse(t).unwrap_err());
        assert!(err("not json").contains("not valid JSON"));
        assert!(err("[1,2]").contains("not a Bookx settings file"));
        assert!(err(r#"{"format":"other","version":1,"settings":{}}"#).contains("not a Bookx"));
        assert!(err(r#"{"format":"bookx-settings","settings":{}}"#).contains("no version"));
        assert!(err(r#"{"format":"bookx-settings","version":99,"settings":{}}"#).contains("newer"));
        assert!(err(r#"{"format":"bookx-settings","version":1}"#).contains("no settings"));
        assert!(
            err(r#"{"format":"bookx-settings","version":1,"settings":{"eink":"yes please"}}"#)
                .contains("does not understand")
        );
        let huge = format!(
            r#"{{"format":"bookx-settings","version":1,"settings":{{"x":"{}"}}}}"#,
            "a".repeat(MAX_BYTES as usize)
        );
        assert!(err(&huge).contains("larger"));
        let many: String = (0..=MAX_KEYS)
            .map(|i| format!("\"k{i}\":1"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(err(&format!(
            r#"{{"format":"bookx-settings","version":1,"settings":{{{many}}}}}"#
        ))
        .contains("too many"));
    }

    #[test]
    fn deeply_nested_json_is_an_error_not_a_crash() {
        let deep = format!(
            r#"{{"format":"bookx-settings","version":1,"settings":{}1{}}}"#,
            "[".repeat(5000),
            "]".repeat(5000)
        );
        assert!(parse(&deep).is_err());
    }

    #[test]
    fn a_partial_file_changes_only_what_it_names() {
        let imp =
            parse(r#"{"format":"bookx-settings","version":1,"settings":{"eink":true}}"#).unwrap();
        let mut cur = Config::default();
        cur.copy_on_add = !cur.copy_on_add;
        let out = apply(&cur, &imp).unwrap();
        assert!(out.eink);
        assert_eq!(out.copy_on_add, cur.copy_on_add);
    }
}
