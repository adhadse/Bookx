//! Look-up settings that have a screen (Preferences → Dictionaries): the order the installed
//! dictionaries are asked in, your own web look-up sites, and the history of looked-up words. The
//! engines are `bookx_lookup::dictset` (`DictionarySet`, `WebSource`, `LookupHistory`); this file is
//! the validation and the storage around them. No GTK in here, so it is unit tested; the screen is
//! `ui/lookup_settings.rs`.
//!
//! * **Order**: kept in `priority.txt` inside the dictionaries folder (one folder name per line), so it
//!   travels with the dictionaries and needs no setting. [`crate::dictionaries::load`] applies it.
//!   Dictionaries not listed come after the listed ones, alphabetically.
//! * **Web sites**: `{word}` in the address stands for the selected text (percent-encoded). `{query}`
//!   is accepted as well. Only `http` and `https` pass, through the same check as every address
//!   that comes from outside ([`crate::core::util::safe_external_url`]).
//! * **History**: the words only, newest first, at most [`HISTORY_CAPACITY`], in `lookup-history.json`
//!   next to the settings; nothing leaves the computer.

use std::path::{Path, PathBuf};

use bookx_lookup::dictset::{LookupHistory, WebSource};
use serde::{Deserialize, Serialize};

use crate::i18n::{tr, trf};

pub const ORDER_FILE: &str = "priority.txt";
/// Most lines read from the order file.
const MAX_ORDER_LINES: usize = 500;
pub const HISTORY_CAPACITY: usize = 200;
pub const MAX_WEB_SOURCES: usize = 12;
pub const MAX_NAME: usize = 40;
pub const MAX_TEMPLATE: usize = 300;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LookupPrefs {
    /// Your own web look-up sites (the built-in ones are `WebSource::defaults`).
    pub web_sources: Vec<WebSource>,
    /// Keep the list of looked-up words.
    pub keep_history: bool,
}

impl Default for LookupPrefs {
    fn default() -> Self {
        LookupPrefs {
            web_sources: Vec::new(),
            keep_history: true,
        }
    }
}

// ---- dictionary order -------------------------------------------------------------------------

/// How the order file names a dictionary folder: its path below `root`, with `/`.
pub fn dict_id(root: &Path, dir: &Path) -> String {
    dir.strip_prefix(root)
        .unwrap_or(dir)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

/// The saved order (empty when there is none or it cannot be read).
pub fn read_order(root: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(root.join(ORDER_FILE)) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for l in text.lines().take(MAX_ORDER_LINES) {
        let l = l.trim();
        if l.is_empty() || l.len() > 300 || l.contains("..") || l.starts_with('/') {
            continue;
        }
        if !out.iter().any(|x| x == l) {
            out.push(l.to_string());
        }
    }
    out
}

/// Saves the order (a temporary file, then a rename, so a crash never leaves half a list).
pub fn write_order(root: &Path, ids: &[String]) -> std::io::Result<()> {
    std::fs::create_dir_all(root)?;
    let tmp = root.join(format!(".{ORDER_FILE}.tmp"));
    std::fs::write(&tmp, ids.join("\n") + "\n")?;
    std::fs::rename(&tmp, root.join(ORDER_FILE))
}

/// `dirs` (alphabetical) in the saved order: the listed ones first as listed, then the others.
pub fn apply_order(dirs: Vec<PathBuf>, root: &Path) -> Vec<PathBuf> {
    let order = read_order(root);
    if order.is_empty() {
        return dirs;
    }
    let mut rest = dirs;
    let mut out = Vec::with_capacity(rest.len());
    for id in &order {
        if let Some(i) = rest.iter().position(|d| dict_id(root, d) == *id) {
            out.push(rest.remove(i));
        }
    }
    out.extend(rest);
    out
}

/// Moves one item of a list (`false` when an index is out of range or nothing moves).
pub fn move_in<T>(list: &mut Vec<T>, from: usize, to: usize) -> bool {
    if from >= list.len() || to >= list.len() || from == to {
        return false;
    }
    let item = list.remove(from);
    list.insert(to, item);
    true
}

/// A dictionary of the folder as the screen lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictEntry {
    pub id: String,
    pub title: String,
}

/// The `bookname` of a StarDict folder, read from its `.ifo` without opening the dictionary.
pub fn bookname(dir: &Path) -> Option<String> {
    let rd = std::fs::read_dir(dir).ok()?;
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "ifo") {
            use std::io::Read;
            let mut buf = String::new();
            std::fs::File::open(&p)
                .ok()?
                .take(8192)
                .read_to_string(&mut buf)
                .ok()?;
            return buf
                .lines()
                .find_map(|l| l.strip_prefix("bookname="))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
        }
    }
    None
}

/// The dictionaries of `root` in the order they are asked in.
pub fn entries(root: &Path) -> Vec<DictEntry> {
    apply_order(crate::dictionaries::dictionary_dirs(root), root)
        .iter()
        .map(|d| {
            let id = dict_id(root, d);
            let title = bookname(d).unwrap_or_else(|| id.clone());
            DictEntry { id, title }
        })
        .collect()
}

/// Moves a dictionary and saves the result. Returns the new order of ids.
pub fn move_dictionary(root: &Path, id: &str, delta: i32) -> Result<Vec<String>, String> {
    let mut ids: Vec<String> = entries(root).into_iter().map(|e| e.id).collect();
    let Some(from) = ids.iter().position(|i| i == id) else {
        return Err(tr("This dictionary is gone."));
    };
    let to = (from as i64 + delta as i64).clamp(0, ids.len() as i64 - 1) as usize;
    if move_in(&mut ids, from, to) {
        write_order(root, &ids).map_err(|e| trf("Could not save the order: {}", &[&e]))?;
    }
    Ok(ids)
}

// ---- web sites --------------------------------------------------------------------------------

/// Checks a name and an address template and makes the source. `editing` is the index of the one
/// being edited (it may keep its name).
pub fn make_source(
    name: &str,
    template: &str,
    existing: &[WebSource],
    editing: Option<usize>,
) -> Result<WebSource, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(tr("Give the site a name."));
    }
    if name.chars().count() > MAX_NAME || name.chars().any(|c| c.is_control()) {
        return Err(trf(
            "The name can have at most {} characters.",
            &[&MAX_NAME],
        ));
    }
    let template = template.trim().replace("{query}", "{word}");
    if template.len() > MAX_TEMPLATE {
        return Err(tr("The address is too long."));
    }
    if !template.contains("{word}") {
        return Err(tr("The address must contain {word} where the text goes."));
    }
    let lower = template.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return Err(tr("The address must start with https:// or http://."));
    }
    // the site (everything before the first slash after the scheme) is fixed text: selected text
    // never chooses where the browser goes
    let after = template.split_once("://").map_or("", |x| x.1);
    let authority = after.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains("{word}") || authority.contains('@') || authority.is_empty() {
        return Err(tr(
            "Put {word} after the site name, for example https://example.com/search?q={word}.",
        ));
    }
    let src = WebSource::new(name, &template).map_err(|_| tr("This address cannot be used."))?;
    if crate::core::util::safe_external_url(&src.url("sample").map_err(|e| e.to_string())?)
        .is_none()
    {
        return Err(tr("This address cannot be used."));
    }
    let clash = existing
        .iter()
        .enumerate()
        .any(|(i, s)| s.name.eq_ignore_ascii_case(name) && Some(i) != editing);
    if clash {
        return Err(tr("There is a site with this name already."));
    }
    if editing.is_none() && existing.len() >= MAX_WEB_SOURCES {
        return Err(trf("At most {} sites can be added.", &[&MAX_WEB_SOURCES]));
    }
    Ok(src)
}

/// The address to open for `term`, or why there is none. It passes the check for outside addresses
/// once more, because the settings file may have been edited by hand.
pub fn expand(src: &WebSource, term: &str) -> Result<String, String> {
    let url = src.url(term).map_err(|_| tr("Nothing to look up."))?;
    crate::core::util::safe_external_url(&url).ok_or_else(|| tr("This address cannot be used."))
}

/// The sites offered when looking up: the built-in ones and yours.
pub fn all_sources(prefs: &LookupPrefs) -> Vec<WebSource> {
    let mut v = WebSource::defaults();
    for s in &prefs.web_sources {
        if !v.iter().any(|x| x.name.eq_ignore_ascii_case(&s.name)) {
            v.push(s.clone());
        }
    }
    v
}

// ---- history ----------------------------------------------------------------------------------

/// Where the history is kept: next to the settings file.
pub fn history_path() -> Option<PathBuf> {
    crate::config::stats_path().and_then(|p| p.parent().map(|d| d.join("lookup-history.json")))
}

pub fn load_history(path: &Path) -> LookupHistory {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<LookupHistory>(&t).ok())
        .map(|h| {
            // a file written by hand may hold more than the limit
            let mut fresh = LookupHistory::new(HISTORY_CAPACITY);
            for w in h.entries().iter().rev().take(HISTORY_CAPACITY) {
                fresh.record(w);
            }
            fresh
        })
        .unwrap_or_else(|| LookupHistory::new(HISTORY_CAPACITY))
}

pub fn save_history(path: &Path, h: &LookupHistory) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string(h).unwrap_or_default())?;
    std::fs::rename(&tmp, path)
}

/// Notes a looked-up word (nothing when the history is off).
pub fn record(path: &Path, keep: bool, word: &str) {
    if !keep {
        return;
    }
    let mut h = load_history(path);
    h.record(word);
    let _ = save_history(path, &h);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-lp-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn dict(root: &Path, name: &str, book: &str) {
        let d = root.join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("x.ifo"),
            format!("StarDict's dict ifo file\nversion=3.0.0\nbookname={book}\nwordcount=0\n"),
        )
        .unwrap();
    }

    #[test]
    fn dictionaries_are_listed_in_the_saved_order_and_new_ones_come_last() {
        let root = tmp("order");
        for (n, b) in [("a", "Alpha"), ("b", "Beta"), ("c", "Gamma")] {
            dict(&root, n, b);
        }
        let titles = |r: &Path| entries(r).into_iter().map(|e| e.title).collect::<Vec<_>>();
        assert_eq!(titles(&root), vec!["Alpha", "Beta", "Gamma"]);
        assert_eq!(
            move_dictionary(&root, "c", -2).unwrap(),
            vec!["c", "a", "b"]
        );
        assert_eq!(titles(&root), vec!["Gamma", "Alpha", "Beta"]);
        // a dictionary installed later is asked last; a removed one disappears from the order
        dict(&root, "d", "Delta");
        std::fs::remove_dir_all(root.join("a")).unwrap();
        assert_eq!(titles(&root), vec!["Gamma", "Beta", "Delta"]);
        // moving past the ends stays inside the list
        assert_eq!(
            move_dictionary(&root, "c", -5).unwrap(),
            vec!["c", "b", "d"]
        );
        assert_eq!(move_dictionary(&root, "d", 5).unwrap(), vec!["c", "b", "d"]);
        assert!(move_dictionary(&root, "zzz", 1).is_err());
    }

    #[test]
    fn the_order_decides_which_dictionary_answers_first() {
        let root = tmp("load");
        for (n, b) in [("a", "Alpha"), ("b", "Beta")] {
            dict(&root, n, b);
        }
        let dirs = |r: &Path| {
            apply_order(crate::dictionaries::dictionary_dirs(r), r)
                .iter()
                .map(|d| dict_id(r, d))
                .collect::<Vec<_>>()
        };
        assert_eq!(dirs(&root), vec!["a", "b"]);
        write_order(&root, &["b".into(), "a".into()]).unwrap();
        assert_eq!(dirs(&root), vec!["b", "a"]);
    }

    #[test]
    fn a_hand_written_order_file_cannot_name_other_places() {
        let root = tmp("evil");
        dict(&root, "a", "Alpha");
        std::fs::write(
            root.join(ORDER_FILE),
            format!("../../etc\n/etc\n\na\na\n{}", "x".repeat(400)),
        )
        .unwrap();
        assert_eq!(read_order(&root), vec!["a"]);
        std::fs::write(root.join(ORDER_FILE), "x\n".repeat(2000)).unwrap();
        assert!(read_order(&root).len() <= MAX_ORDER_LINES);
    }

    #[test]
    fn move_in_stays_inside_the_list() {
        let mut v = vec![1, 2, 3];
        assert!(move_in(&mut v, 0, 2));
        assert_eq!(v, vec![2, 3, 1]);
        assert!(!move_in(&mut v, 0, 3) && !move_in(&mut v, 5, 0) && !move_in(&mut v, 1, 1));
    }

    #[test]
    fn web_sites_need_a_name_a_fixed_site_and_the_placeholder() {
        let ok = make_source(
            "Reverso",
            "https://dictionary.reverso.net/english/{word}",
            &[],
            None,
        )
        .unwrap();
        assert_eq!(ok.name, "Reverso");
        // {query} is the same thing
        let q = make_source("Q", "https://example.com/?q={query}", &[], None).unwrap();
        assert_eq!(q.template, "https://example.com/?q={word}");
        for (n, t) in [
            ("", "https://example.com/{word}"),
            ("N", "https://example.com/"),
            ("N", "ftp://example.com/{word}"),
            ("N", "javascript:alert({word})"),
            ("N", "file:///etc/{word}"),
            ("N", "https://{word}.example.com/"),
            ("N", "https://user@example.com/{word}"),
            ("N", "https:///{word}"),
            ("N", "example.com/{word}"),
            ("N", "https://example.com/ {word}"),
        ] {
            assert!(make_source(n, t, &[], None).is_err(), "{n:?} {t:?}");
        }
        assert!(make_source(
            "N",
            &format!("https://example.com/{}", "a".repeat(400)),
            &[],
            None
        )
        .is_err());
        assert!(make_source(&"n".repeat(MAX_NAME + 1), "https://e.com/{word}", &[], None).is_err());
    }

    #[test]
    fn web_site_names_are_unique_and_limited() {
        let a = make_source("Site", "https://a.example/{word}", &[], None).unwrap();
        let list = vec![a];
        assert!(make_source("SITE", "https://b.example/{word}", &list, None).is_err());
        assert!(make_source("SITE", "https://b.example/{word}", &list, Some(0)).is_ok());
        let many: Vec<WebSource> = (0..MAX_WEB_SOURCES)
            .map(|i| make_source(&format!("s{i}"), "https://a.example/{word}", &[], None).unwrap())
            .collect();
        assert!(make_source("more", "https://a.example/{word}", &many, None).is_err());
    }

    #[test]
    fn the_selected_text_is_encoded_and_cannot_change_the_site() {
        let s = make_source("S", "https://example.com/define?q={word}", &[], None).unwrap();
        let u = expand(&s, "a b&c=d/../?#@evil.com").unwrap();
        assert!(u.starts_with("https://example.com/define?q="), "{u}");
        assert!(
            !u.contains(' ') && !u.contains('#') && !u.contains('@'),
            "{u}"
        );
        assert_eq!(u.matches('?').count(), 1);
        assert!(expand(&s, "  ").is_err());
        // a settings file edited by hand with a bad template is still refused when used
        let bad = WebSource {
            name: "Bad".into(),
            template: "javascript:alert('{word}')".into(),
        };
        assert!(expand(&bad, "x").is_err());
    }

    #[test]
    fn built_in_and_own_sites_are_listed_once() {
        let mut p = LookupPrefs::default();
        p.web_sources
            .push(make_source("Mine", "https://m.example/{word}", &[], None).unwrap());
        p.web_sources.push(WebSource {
            name: "wiktionary".into(),
            template: "https://x.example/{word}".into(),
        });
        let all = all_sources(&p);
        assert_eq!(
            all.iter()
                .filter(|s| s.name.eq_ignore_ascii_case("wiktionary"))
                .count(),
            1
        );
        assert!(all.iter().any(|s| s.name == "Mine"));
    }

    #[test]
    fn history_is_recent_first_bounded_and_can_be_switched_off() {
        let d = tmp("hist");
        let p = d.join("sub").join("lookup-history.json");
        record(&p, true, "alpha");
        record(&p, true, "beta");
        record(&p, true, "Alpha");
        record(&p, false, "ignored");
        assert_eq!(load_history(&p).entries(), vec!["Alpha", "beta"]);
        for i in 0..HISTORY_CAPACITY + 20 {
            record(&p, true, &format!("w{i}"));
        }
        let h = load_history(&p);
        assert_eq!(h.len(), HISTORY_CAPACITY);
        assert_eq!(h.entries()[0], format!("w{}", HISTORY_CAPACITY + 19));
        // a broken file is an empty history
        std::fs::write(&p, "not json").unwrap();
        assert!(load_history(&p).is_empty());
    }

    #[test]
    fn old_settings_without_the_block_load() {
        let p: LookupPrefs = serde_json::from_str("{}").unwrap();
        assert!(p.keep_history && p.web_sources.is_empty());
    }
}
