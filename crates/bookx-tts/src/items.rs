//! What the settings page lists for download: one entry per thing the user thinks of
//! ("Kokoro", a Piper voice, the speech runtime), made of one or more packs.

use crate::factory::language_name;
use crate::models::{
    catalog, install_plan, kokoro_model_pack, kokoro_paths, Pack, PackKind, Store,
};
use crate::prefs::TtsPrefs;

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// Stable key (`runtime`, `kokoro`, `piper-de_DE-thorsten-medium`).
    pub key: String,
    pub title: String,
    pub summary: String,
    pub languages: Vec<String>,
    pub license_name: String,
    pub license_url: String,
    pub license_note: String,
    pub installed: bool,
    /// Pack ids to install (requirements first), only those missing.
    pub plan: Vec<String>,
    /// Bytes still to download.
    pub download_bytes: u64,
    /// Bytes on disk when installed.
    pub size_bytes: u64,
    /// Pack ids that go when the item is removed (the runtime stays).
    pub remove: Vec<String>,
}

pub fn format_size(bytes: u64) -> String {
    const MB: f64 = 1_000_000.0;
    let b = bytes as f64;
    if b >= 1000.0 * MB {
        format!("{:.1} GB", b / (1000.0 * MB))
    } else if b >= MB {
        format!("{:.0} MB", b / MB)
    } else {
        format!("{:.0} KB", (b / 1000.0).max(1.0))
    }
}

fn languages_text(langs: &[String]) -> String {
    langs
        .iter()
        .map(|l| language_name(l))
        .collect::<Vec<_>>()
        .join(", ")
}

fn make(
    all: &[Pack],
    store: &Store,
    key: &str,
    pack_id: &str,
    remove: Vec<String>,
    title: &str,
) -> Option<Item> {
    let pack = all.iter().find(|p| p.id == pack_id)?;
    let plan = install_plan(all, store, pack_id);
    let download_bytes = plan.iter().map(|p| p.download_size()).sum();
    // the whole chain, to show what the item weighs when installed
    let mut chain = vec![pack_id.to_string()];
    chain.extend(pack.requires.clone());
    let size_bytes: u64 = chain
        .iter()
        .filter_map(|id| all.iter().find(|p| &p.id == id))
        .map(|p| p.download_size())
        .sum();
    Some(Item {
        key: key.to_string(),
        title: title.to_string(),
        summary: pack.summary.clone(),
        languages: pack.languages.clone(),
        license_name: pack.license_name.clone(),
        license_url: pack.license_url.clone(),
        license_note: pack.license_note.clone(),
        installed: plan.is_empty(),
        plan: plan.into_iter().map(|p| p.id).collect(),
        download_bytes,
        size_bytes,
        remove,
    })
}

/// The entries for the settings page, in the order shown.
pub fn items(prefs: &TtsPrefs, store: &Store) -> Vec<Item> {
    let all = catalog();
    let mut out = Vec::new();
    out.extend(make(
        &all,
        store,
        "runtime",
        "onnxruntime",
        vec!["onnxruntime".into()],
        "Speech runtime (ONNX Runtime)",
    ));
    let model = kokoro_model_pack(&prefs.kokoro_model);
    if let Some(mut k) = make(
        &all,
        store,
        "kokoro",
        model,
        vec![
            "kokoro-int8".into(),
            "kokoro-fp32".into(),
            "kokoro-voices".into(),
        ],
        "Kokoro, natural voices (recommended)",
    ) {
        k.installed =
            kokoro_paths(store, &prefs.kokoro_model).is_some() && store.runtime_lib().is_some();
        // the runtime is part of the download but not of what Kokoro owns
        out.push(k);
    }
    for p in all.iter().filter(|p| p.kind == PackKind::PiperVoice) {
        let title = format!("Piper: {}", p.title);
        out.extend(make(&all, store, &p.id, &p.id, vec![p.id.clone()], &title));
    }
    out
}

pub fn subtitle(i: &Item) -> String {
    let mut parts = Vec::new();
    parts.push(if i.installed {
        format!("Installed · {}", format_size(i.size_bytes))
    } else {
        format!("{} to download", format_size(i.download_bytes))
    });
    parts.push(i.license_name.clone());
    if !i.languages.is_empty() {
        parts.push(languages_text(&i.languages));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-items-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn sizes_are_short_and_human() {
        assert_eq!(format_size(500), "1 KB");
        assert_eq!(format_size(92_361_116), "92 MB");
        assert_eq!(format_size(1_500_000_000), "1.5 GB");
        assert_eq!(format_size(11_413_655), "11 MB");
    }

    #[test]
    fn the_list_starts_with_the_runtime_and_kokoro_and_follows_the_model_choice() {
        let root = tmp();
        let store = Store::new(&root);
        let mut prefs = TtsPrefs::default();
        let list = items(&prefs, &store);
        assert_eq!(list[0].key, "runtime");
        assert_eq!(list[1].key, "kokoro");
        assert!(list.len() > 10, "Piper voices follow");
        let k = &list[1];
        assert!(!k.installed);
        assert_eq!(k.plan, ["onnxruntime", "kokoro-voices", "kokoro-int8"]);
        assert!(
            (100_000_000..150_000_000).contains(&k.download_bytes),
            "{}",
            k.download_bytes
        );
        assert_eq!(k.license_name, "Apache-2.0");
        assert!(
            k.remove.contains(&"kokoro-voices".to_string())
                && !k.remove.contains(&"onnxruntime".to_string())
        );
        prefs.kokoro_model = "fp32".into();
        let big = &items(&prefs, &store)[1];
        assert_eq!(big.plan.last().unwrap(), "kokoro-fp32");
        assert!(big.download_bytes > 300_000_000);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn subtitles_tell_size_licence_and_languages() {
        let root = tmp();
        let list = items(&TtsPrefs::default(), &Store::new(&root));
        let s = subtitle(&list[1]);
        assert!(
            s.contains("MB to download") && s.contains("Apache-2.0") && s.contains("English"),
            "{s}"
        );
        let de = list
            .iter()
            .find(|i| i.key == "piper-de_DE-thorsten-medium")
            .unwrap();
        let s = subtitle(de);
        assert!(s.contains("CC0") && s.contains("German"), "{s}");
        // a Piper voice needs the runtime too
        assert_eq!(de.plan, ["onnxruntime", "piper-de_DE-thorsten-medium"]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_installed_item_says_so_and_has_nothing_to_download() {
        let root = tmp();
        let store = Store::new(&root);
        let all = catalog();
        let p = all
            .iter()
            .find(|p| p.id == "piper-de_DE-thorsten-medium")
            .unwrap();
        std::fs::create_dir_all(store.dir_of(p)).unwrap();
        for f in &p.files {
            std::fs::write(store.dir_of(p).join(&f.rel), vec![0u8; f.size as usize]).unwrap();
        }
        store.mark_installed(p).unwrap();
        let rt = all.iter().find(|p| p.id == "onnxruntime").unwrap();
        std::fs::create_dir_all(store.dir_of(rt)).unwrap();
        std::fs::File::create(store.dir_of(rt).join("libonnxruntime.so"))
            .unwrap()
            .set_len(rt.files[0].installed_size())
            .unwrap();
        store.mark_installed(rt).unwrap();
        let list = items(&TtsPrefs::default(), &store);
        let de = list.iter().find(|i| i.key == p.id).unwrap();
        assert!(de.installed && de.plan.is_empty() && de.download_bytes == 0);
        assert!(subtitle(de).starts_with("Installed"));
        assert!(list[0].installed, "the runtime is there");
        assert!(!list[1].installed, "Kokoro is not");
        std::fs::remove_dir_all(&root).ok();
    }
}
