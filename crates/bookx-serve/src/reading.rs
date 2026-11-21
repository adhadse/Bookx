//! Reading state found on a device, in a form that does not care where it came from.
//!
//! Sources: the Kobo database ([`crate::kobo`]) and KOReader's per-book sidecar folders
//! (`<book>.sdr/metadata.<ext>.lua`), which KOReader writes on Kobo, Kindle, PocketBook and
//! Android readers alike. The Kindle's own `.sdr` folders hold a binary format that is not read.

use std::path::{Path, PathBuf};

use crate::devfs::DeviceBook;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Unread,
    Reading,
    Finished,
}

impl Status {
    pub fn label(&self) -> &'static str {
        match self {
            Status::Unread => "Unread",
            Status::Reading => "Reading",
            Status::Finished => "Finished",
        }
    }
}

/// How far a book has been read on a device.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceState {
    /// Title the device knows the book by, when it says.
    pub title: Option<String>,
    /// 0.0 to 1.0.
    pub progress: f64,
    pub status: Status,
    pub last_read: Option<String>,
    /// Where the state was read from (shown to the user).
    pub source: &'static str,
}

/// `<dir>/<name without its last extension>.sdr`, KOReader's per-book folder.
pub fn sidecar_dir(book: &Path) -> PathBuf {
    let name = book
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(&name);
    book.with_file_name(format!("{stem}.sdr"))
}

/// `<book>.sdr/metadata.<ext>.lua`.
pub fn sidecar_file(book: &Path) -> PathBuf {
    let ext = book
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    sidecar_dir(book).join(format!("metadata.{ext}.lua"))
}

/// A highlight, note or bookmark found on a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceNote {
    /// The highlighted text (for a bookmark without text: its label).
    pub text: String,
    /// What the reader wrote about it; empty when nothing.
    pub note: String,
    pub chapter: Option<String>,
    /// `YYYY-MM-DD HH:MM:SS` or ISO 8601, whatever the device wrote.
    pub created: Option<String>,
    /// Last change, when the device keeps it apart from `created`.
    pub updated: Option<String>,
    /// The colour as a stored word (`bookx_core::devstyle`: a Bookx colour, or a device colour kept
    /// as written); `None` when the device does not say.
    pub color: Option<String>,
    /// The style as a Bookx style name; `None` when the device does not say.
    pub style: Option<String>,
    /// A page bookmark (KOReader's annotation without a highlighted range): its text is only a label
    /// such as `in Chapter 3`, not a quote.
    pub bookmark: bool,
}

/// Everything a KOReader sidecar says.
#[derive(Debug, Clone, PartialEq)]
pub struct Sidecar {
    pub state: Option<DeviceState>,
    pub notes: Vec<DeviceNote>,
}

/// Reads KOReader's sidecar of `book`, if there is one that says anything.
pub fn koreader_state(book: &Path) -> Option<DeviceState> {
    let text = std::fs::read_to_string(sidecar_file(book)).ok()?;
    parse_koreader(&text)
}

/// Reads progress and annotations from KOReader's sidecar of `book` (`None` without a file).
pub fn koreader_sidecar(book: &Path) -> Option<Sidecar> {
    let text = std::fs::read_to_string(sidecar_file(book)).ok()?;
    Some(parse_sidecar(&text))
}

/// Progress, status and annotations from the text of a KOReader `metadata.*.lua`. Uses a real
/// parse of the file (so highlight texts that mention `percent_finished` cannot confuse it) and
/// falls back to scanning for the keys when the file is not valid Lua data.
pub fn parse_sidecar(text: &str) -> Sidecar {
    let Some(root) = crate::lua::parse(text) else {
        return Sidecar {
            state: parse_koreader_scan(text),
            notes: Vec::new(),
        };
    };
    let percent = root.num("percent_finished");
    let summary = root.get("summary");
    let status = summary.and_then(|s| s.str("status"));
    let state = if percent.is_none() && status.is_none() {
        None
    } else {
        let st = status_of(status, percent.unwrap_or(0.0).clamp(0.0, 1.0));
        Some(DeviceState {
            title: root
                .get("doc_props")
                .and_then(|d| d.str("title"))
                .map(str::to_string)
                .filter(|t| !t.trim().is_empty()),
            progress: progress_of(st, percent),
            status: st,
            last_read: summary
                .and_then(|s| s.str("modified"))
                .filter(|m| !m.is_empty())
                .map(str::to_string),
            source: "KOReader",
        })
    };
    let mut notes = Vec::new();
    // KOReader before 2024 kept the look of a highlight (`drawer`, `color`) in `highlight` (page ->
    // list of highlights) and only the text and note in `bookmarks`; the two belong together by
    // their `datetime`
    let mut legacy: std::collections::HashMap<&str, (Option<&str>, Option<&str>)> =
        std::collections::HashMap::new();
    if let Some(pages) = root.get("highlight") {
        for page in pages.items() {
            for h in page.items() {
                if let Some(when) = h.str("datetime") {
                    legacy
                        .entry(when)
                        .or_insert((h.str("color"), h.str("drawer")));
                }
            }
        }
    }
    // current format (`annotations`), and the one older KOReader and Bookx itself write (`bookmarks`)
    for key in ["annotations", "bookmarks"] {
        if let Some(list) = root.get(key) {
            for a in list.items() {
                let text = a.str("text").or_else(|| a.str("notes")).unwrap_or("");
                // `note` is the reader's own words; legacy bookmarks repeat the text in `notes`
                let note = a.str("note").unwrap_or("");
                if text.trim().is_empty() && note.trim().is_empty() {
                    continue;
                }
                // KOReader's current format: a highlight has a range (`pos1`) and a `drawer`; a page
                // bookmark has neither. The legacy `bookmarks` list does not say it that way.
                let bookmark = key == "annotations"
                    && a.get("pos1").is_none()
                    && a.str("drawer").is_none()
                    && a.get("page").is_some();
                let old = a.str("datetime").and_then(|d| legacy.get(d)).copied();
                notes.push(DeviceNote {
                    bookmark,
                    text: text.to_string(),
                    note: note.to_string(),
                    chapter: a.str("chapter").map(str::to_string),
                    created: a.str("datetime").map(str::to_string),
                    updated: a.str("datetime_updated").map(str::to_string),
                    color: a
                        .str("color")
                        .or(old.and_then(|o| o.0))
                        .and_then(bookx_core::devstyle::koreader_color),
                    style: a
                        .str("drawer")
                        .or(old.and_then(|o| o.1))
                        .and_then(bookx_core::devstyle::canonical_style),
                });
            }
            break;
        }
    }
    Sidecar { state, notes }
}

fn status_of(status: Option<&str>, progress: f64) -> Status {
    match status {
        Some("complete") => Status::Finished,
        Some("reading") => Status::Reading,
        _ if progress > 0.0 => Status::Reading,
        _ => Status::Unread,
    }
}

fn progress_of(status: Status, percent: Option<f64>) -> f64 {
    if status == Status::Finished {
        1.0
    } else {
        percent.unwrap_or(0.0).clamp(0.0, 1.0)
    }
}

/// Extracts progress and status from the text of a KOReader `metadata.*.lua`.
pub fn parse_koreader(text: &str) -> Option<DeviceState> {
    parse_sidecar(text).state
}

fn parse_koreader_scan(text: &str) -> Option<DeviceState> {
    let percent = lua_number(text, "percent_finished");
    let status = lua_string(text, "status");
    if percent.is_none() && status.is_none() {
        return None;
    }
    let st = status_of(status.as_deref(), percent.unwrap_or(0.0).clamp(0.0, 1.0));
    Some(DeviceState {
        title: lua_string(text, "title"),
        progress: progress_of(st, percent),
        status: st,
        last_read: None,
        source: "KOReader",
    })
}

fn after_key<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("[\"{key}\"]");
    let i = text.find(&pat)?;
    let rest = text[i + pat.len()..].trim_start();
    Some(rest.strip_prefix('=')?.trim_start())
}

fn lua_number(text: &str, key: &str) -> Option<f64> {
    let rest = after_key(text, key)?;
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn lua_string(text: &str, key: &str) -> Option<String> {
    let rest = after_key(text, key)?.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => out.push(chars.next()?),
            c => out.push(c),
        }
    }
    None
}

/// Everything the device says about one book file.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceBookData {
    pub path: PathBuf,
    pub state: Option<DeviceState>,
    pub notes: Vec<DeviceNote>,
}

/// Reading state and notes of the given device books: the Kobo database for books it knows,
/// KOReader sidecars for the others (a sidecar further along wins when both exist; notes of both
/// are kept). Only books with something to say are returned, sorted by path.
pub fn collect_data(mount: &Path, books: &[DeviceBook]) -> Vec<DeviceBookData> {
    let mut out: Vec<DeviceBookData> = Vec::new();
    let slot = |out: &mut Vec<DeviceBookData>, path: PathBuf| -> usize {
        match out.iter().position(|d| d.path == path) {
            Some(i) => i,
            None => {
                out.push(DeviceBookData {
                    path,
                    state: None,
                    notes: Vec::new(),
                });
                out.len() - 1
            }
        }
    };
    if let Some(db) = crate::devices::kobo_db_path(mount) {
        if let Ok(rows) = crate::kobo::read_states(&db) {
            for (id, st) in rows {
                if let Some(rel) = crate::kobo::rel_of_content_id(&id) {
                    let path = mount.join(rel);
                    if books.iter().any(|b| b.path == path) {
                        let i = slot(&mut out, path);
                        out[i].state = Some(st);
                    }
                }
            }
        }
        if let Ok(rows) = crate::kobo::read_notes(&db) {
            for (id, note) in rows {
                if let Some(rel) = crate::kobo::rel_of_content_id(&id) {
                    let path = mount.join(rel);
                    if books.iter().any(|b| b.path == path) {
                        let i = slot(&mut out, path);
                        out[i].notes.push(note);
                    }
                }
            }
        }
    }
    for b in books {
        let Some(side) = koreader_sidecar(&b.path) else {
            continue;
        };
        if side.state.is_none() && side.notes.is_empty() {
            continue;
        }
        let i = slot(&mut out, b.path.clone());
        let d = &mut out[i];
        if let Some(st) = side.state {
            match &d.state {
                Some(have) if st.progress <= have.progress => {}
                _ => d.state = Some(st),
            }
        }
        d.notes.extend(side.notes);
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Reading state of the given device books (see [`collect_data`]).
pub fn collect_states(mount: &Path, books: &[DeviceBook]) -> Vec<(PathBuf, DeviceState)> {
    collect_data(mount, books)
        .into_iter()
        .filter_map(|d| d.state.map(|s| (d.path, s)))
        .collect()
}

/// Seconds since the epoch of a time stamp as the devices write them: `2026-08-11T13:22:26Z`,
/// `2026-08-11T13:22:26.000`, `2026-08-11 13:22:26` or just `2026-08-11`. Devices that write no
/// zone are taken as UTC.
pub fn parse_time(s: &str) -> Option<i64> {
    use chrono::{NaiveDate, NaiveDateTime};
    let s = s.trim();
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(t.timestamp());
    }
    for f in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"] {
        if let Ok(t) = NaiveDateTime::parse_from_str(s.trim_end_matches('Z'), f) {
            return Some(t.and_utc().timestamp());
        }
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|t| t.and_utc().timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LUA: &str = r#"-- we can read Lua syntax here!
return {
    ["doc_path"] = "/mnt/onboard/Bookx/A.epub",
    ["percent_finished"] = 0.4375,
    ["summary"] = {
        ["status"] = "reading",
        ["modified"] = "2026-11-01",
    },
    ["doc_props"] = {
        ["title"] = "Alpha \"One\"",
    },
}
"#;

    /// The real KOReader (v2026.07) sidecar of the owner's Kobo: highlights have `pos0`/`pos1`/`drawer`,
    /// a page bookmark has only `page`, `pageno` and the label `in <chapter>` as its text.
    #[test]
    fn a_page_bookmark_is_no_highlight() {
        let lua = r#"return {
    ["annotations"] = {
        [1] = { ["chapter"] = "Domain Coupling", ["datetime"] = "2026-08-23 07:09:10", ["page"] = "/body/DocFragment[7]/body/section/div/section[3]/div/section[1]/div/figure/div/img.0", ["pageno"] = 77, ["text"] = "in Domain Coupling" },
        [2] = { ["chapter"] = "Ch", ["color"] = "cyan", ["datetime"] = "2026-08-17 11:22:15", ["datetime_updated"] = "2026-08-17 11:30:00", ["drawer"] = "lighten", ["page"] = "/body/a/text().0", ["pageno"] = 22, ["pos0"] = "/body/a/text().0", ["pos1"] = "/body/a/text().9", ["text"] = "A quote.", ["note"] = "mine" },
    },
    ["doc_props"] = { ["title"] = "T" },
}"#;
        let n = parse_sidecar(lua).notes;
        assert_eq!(n.len(), 2);
        assert!(n[0].bookmark && n[0].text == "in Domain Coupling");
        assert!(!n[1].bookmark);
        assert_eq!(n[1].created.as_deref(), Some("2026-08-17 11:22:15"));
        assert_eq!(n[1].updated.as_deref(), Some("2026-08-17 11:30:00"));
        assert_eq!(n[1].color.as_deref(), Some("cyan"));
    }

    #[test]
    fn koreader_progress_and_status_are_parsed() {
        let s = parse_koreader(LUA).unwrap();
        assert_eq!(s.status, Status::Reading);
        assert!((s.progress - 0.4375).abs() < 1e-9);
        assert_eq!(s.title.as_deref(), Some("Alpha \"One\""));
        assert_eq!(s.source, "KOReader");
    }

    #[test]
    fn complete_means_finished_even_without_a_percentage() {
        let s = parse_koreader(r#"return { ["summary"] = { ["status"] = "complete" } }"#).unwrap();
        assert_eq!(s.status, Status::Finished);
        assert_eq!(s.progress, 1.0);
    }

    #[test]
    fn nonsense_and_empty_files_say_nothing() {
        assert!(parse_koreader("").is_none());
        assert!(parse_koreader("return { [\"other\"] = 1 }").is_none());
        let s = parse_koreader("[\"percent_finished\"] = 7.5").unwrap();
        assert_eq!(s.progress, 1.0, "clamped");
        let s = parse_koreader("[\"percent_finished\"] = \"x\"");
        assert!(s.is_none());
    }

    #[test]
    fn sidecar_paths_follow_koreader() {
        assert_eq!(
            sidecar_file(Path::new("/m/Books/a.epub")),
            PathBuf::from("/m/Books/a.sdr/metadata.epub.lua")
        );
        assert_eq!(
            sidecar_file(Path::new("/m/Books/a.kepub.epub")),
            PathBuf::from("/m/Books/a.kepub.sdr/metadata.epub.lua")
        );
    }

    #[test]
    fn states_merge_the_kobo_database_and_sidecars() {
        let root = std::env::temp_dir().join(format!("bookx-rd-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join(".kobo")).unwrap();
        std::fs::create_dir_all(root.join("Bookx/b.sdr")).unwrap();
        std::fs::write(root.join("Bookx/a.kepub.epub"), b"x").unwrap();
        std::fs::write(root.join("Bookx/b.epub"), b"x").unwrap();
        std::fs::write(root.join("Bookx/c.epub"), b"x").unwrap();
        std::fs::write(
            root.join("Bookx/b.sdr/metadata.epub.lua"),
            "return { [\"percent_finished\"] = 0.9 }",
        )
        .unwrap();
        let db = root.join(".kobo/KoboReader.sqlite");
        crate::kobo::create_test_database(&db).unwrap();
        rusqlite::Connection::open(&db)
            .unwrap()
            .execute_batch(
                "INSERT INTO content VALUES ('file:///mnt/onboard/Bookx/a.kepub.epub','6','A','x',50,1,NULL);
                 INSERT INTO content VALUES ('file:///mnt/onboard/Bookx/b.epub','6','B','x',10,1,NULL);",
            )
            .unwrap();
        let books = crate::devfs::list_books(&root);
        let st = collect_states(&root, &books);
        assert_eq!(st.len(), 2, "c.epub has no state anywhere");
        let a = &st
            .iter()
            .find(|(p, _)| p.ends_with("a.kepub.epub"))
            .unwrap()
            .1;
        assert_eq!((a.source, a.progress), ("Kobo database", 0.5));
        let b = &st.iter().find(|(p, _)| p.ends_with("b.epub")).unwrap().1;
        assert_eq!(
            (b.source, b.progress),
            ("KOReader", 0.9),
            "the further one wins"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// Shape of a real sidecar: annotations first (their text may mention the keys), then the rest.
    const REAL_SIDECAR: &str = "-- /mnt/onboard/A.sdr/metadata.epub.lua\nreturn {\n    [\"annotations\"] = {\n        [1] = {\n            [\"chapter\"] = \"One\",\n            [\"color\"] = \"yellow\",\n            [\"datetime\"] = \"2026-08-17 11:22:15\",\n            [\"page\"] = \"/body/DocFragment[6]/body/p[1]/text().0\",\n            [\"text\"] = \"the key [\\\"percent_finished\\\"] = 0.99 and [\\\"status\\\"] = \\\"complete\\\" in a quote\",\n        },\n        [2] = {\n            [\"chapter\"] = \"Two\",\n            [\"datetime\"] = \"2026-08-17 16:39:33\",\n            [\"datetime_updated\"] = \"2026-08-17 16:55:40\",\n            [\"note\"] = \"worth rereading\",\n            [\"text\"] = \"second highlight\",\n        },\n    },\n    [\"doc_props\"] = {\n        [\"authors\"] = \"Ann\\\nBob\",\n        [\"title\"] = \"Real Title\",\n    },\n    [\"percent_finished\"] = 0.23828920570265,\n    [\"stats\"] = {\n        [\"title\"] = \"Other\",\n    },\n    [\"summary\"] = {\n        [\"modified\"] = \"2026-08-12\",\n        [\"status\"] = \"reading\",\n    },\n}\n";

    #[test]
    fn a_real_shaped_sidecar_is_read_by_structure_not_by_searching_for_keys() {
        let sc = parse_sidecar(REAL_SIDECAR);
        let st = sc.state.unwrap();
        assert_eq!(
            st.status,
            Status::Reading,
            "a quoted highlight cannot say `complete`"
        );
        assert!((st.progress - 0.23828920570265).abs() < 1e-12);
        assert_eq!(st.title.as_deref(), Some("Real Title"));
        assert_eq!(st.last_read.as_deref(), Some("2026-08-12"));
        assert_eq!(sc.notes.len(), 2);
        assert_eq!(sc.notes[0].chapter.as_deref(), Some("One"));
        assert!(sc.notes[0].note.is_empty());
        assert_eq!(sc.notes[1].note, "worth rereading");
        assert_eq!(sc.notes[1].updated.as_deref(), Some("2026-08-17 16:55:40"));
    }

    #[test]
    fn the_old_bookmarks_format_bookx_writes_is_read_too() {
        let lua = "return {\n [\"doc_props\"] = { [\"title\"] = \"T\" },\n [\"bookmarks\"] = {\n  [1] = { [\"chapter\"] = \"C\", [\"datetime\"] = \"2026-05-06 07:08:09\", [\"notes\"] = \"my note\", [\"page\"] = \"/body/DocFragment[2]\", [\"text\"] = \"highlighted\" },\n },\n}\n";
        let sc = parse_sidecar(lua);
        assert!(sc.state.is_none(), "no progress in it");
        assert_eq!(sc.notes.len(), 1);
        assert_eq!(sc.notes[0].text, "highlighted");
    }

    #[test]
    fn time_stamps_of_every_device_style_are_understood() {
        assert_eq!(parse_time("2026-08-11T13:22:26Z"), Some(1786454546));
        assert_eq!(parse_time("2026-08-11T13:22:26.000"), Some(1786454546));
        assert_eq!(parse_time("2026-08-11 13:22:26"), Some(1786454546));
        assert_eq!(parse_time("2026-08-11"), Some(1786406400));
        assert_eq!(parse_time("0000-00-00T00:00:00.000"), None);
        assert_eq!(parse_time(""), None);
    }

    #[test]
    fn the_snapshot_of_a_real_device_layout_is_read_without_touching_it() {
        let root = std::env::temp_dir().join(format!("bookx-rd-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join(".kobo")).unwrap();
        std::fs::create_dir_all(root.join("Mine.sdr")).unwrap();
        std::fs::write(root.join("Mine.epub"), b"x").unwrap();
        std::fs::write(root.join("Mine.sdr/metadata.epub.lua"), REAL_SIDECAR).unwrap();
        let db = root.join(".kobo/KoboReader.sqlite");
        crate::kobo::create_real_schema_database(&db).unwrap();
        let listing = |r: &Path| -> Vec<(String, u64)> {
            let mut v = Vec::new();
            fn walk(d: &Path, root: &Path, v: &mut Vec<(String, u64)>) {
                for e in std::fs::read_dir(d).unwrap().flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p, root, v);
                    } else {
                        v.push((
                            p.strip_prefix(root).unwrap().display().to_string(),
                            e.metadata().unwrap().len(),
                        ));
                    }
                }
            }
            walk(r, r, &mut v);
            v.sort();
            v
        };
        let before = listing(&root);
        let books = crate::devfs::list_books(&root);
        let data = collect_data(&root, &books);
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].state.as_ref().unwrap().source, "KOReader");
        assert_eq!(data[0].notes.len(), 2);
        assert_eq!(listing(&root), before);
        let _ = std::fs::remove_dir_all(root);
    }
}
