//! File-level operations on a mounted device: what is on it, how much room is left, removing
//! books, which storage (main memory or SD card) a volume is, and the save-path templates that
//! decide in which folder a sent book lands.
//!
//! Everything works on a directory, so it serves USB mass storage and gvfs-mounted MTP alike, and
//! it is tested with fake device trees.

use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::devices::{sanitize_file_name, BOOK_EXTENSIONS};

// -- storage ------------------------------------------------------------------------------------

/// Capacity of the volume a path is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Space {
    pub total: u64,
    pub free: u64,
}

impl Space {
    /// "12.3 GB free of 32.0 GB".
    pub fn describe(&self) -> String {
        format!(
            "{} free of {}",
            human_size(self.free),
            human_size(self.total)
        )
    }
}

/// Free and total bytes of the file system holding `path`; `None` if it cannot be asked.
#[cfg(unix)]
pub fn free_space(path: &Path) -> Option<Space> {
    let st = nix::sys::statvfs::statvfs(path).ok()?;
    let unit = st.fragment_size() as u64;
    let total = (st.blocks() as u64).saturating_mul(unit);
    let free = (st.blocks_available() as u64).saturating_mul(unit);
    (total > 0).then_some(Space {
        total,
        free: free.min(total),
    })
}

/// Windows: not implemented yet (needs `GetDiskFreeSpaceExW`, which is `unsafe` FFI), so no free-space line.
#[cfg(not(unix))]
pub fn free_space(_path: &Path) -> Option<Space> {
    None
}

/// "512 B", "1.5 MB", "12.3 GB" (decimal units, like the devices themselves report).
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1000.0 && i < UNITS.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// Which memory of the reader a volume is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Storage {
    Main,
    Card,
}

impl Storage {
    pub fn label(&self) -> &'static str {
        match self {
            Storage::Main => "Main memory",
            Storage::Card => "SD card",
        }
    }
}

/// Main memory or SD card, judged by the volume's name (`SDCARD`, `sd card`, `SD_Card`, `EXT_SD`,
/// `microSD`…); the readers do not say it any other way over USB. A device's own volume is main
/// memory, a card is a second volume.
pub fn storage_of(mount: &Path) -> Storage {
    let name = mount
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let words: Vec<&str> = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let is_card = name.contains("card")
        || name.contains("microsd")
        || words.iter().any(|w| matches!(*w, "sd" | "sdcard" | "sdhc"));
    if is_card {
        Storage::Card
    } else {
        Storage::Main
    }
}

// -- listing and removing -----------------------------------------------------------------------

/// A book file found on a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceBook {
    pub path: PathBuf,
    /// Path below the device root, with `/`.
    pub rel: String,
    pub size: u64,
    /// Last modification, seconds since the Unix epoch.
    pub modified: Option<i64>,
}

impl DeviceBook {
    /// File name without the extension (`.kepub.epub` counts as one extension).
    pub fn stem(&self) -> String {
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        file_stem(&name).to_string()
    }

    /// Extension, lower case (`epub`, `kepub.epub`, `azw3`…).
    pub fn format(&self) -> String {
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if name.ends_with(".kepub.epub") {
            "kepub.epub".into()
        } else {
            name.rsplit_once('.')
                .map(|(_, e)| e.to_string())
                .unwrap_or_default()
        }
    }
}

/// `name` without its extension; `.kepub.epub` is removed as a whole.
pub fn file_stem(name: &str) -> &str {
    if name.to_ascii_lowercase().ends_with(".kepub.epub") {
        return &name[..name.len() - ".kepub.epub".len()];
    }
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => name,
    }
}

const SKIPPED_DIRS: &[&str] = &["system", "android", "lost.dir", "trash", ".trash"];
const MAX_DEPTH: usize = 6;
const MAX_BOOKS: usize = 20_000;

/// Every book file below `mount`, sorted by path. Hidden folders (the Kobo's `.kobo`, KOReader's
/// `.sdr`), system folders and symbolic links are skipped, depth and count are bounded.
pub fn list_books(mount: &Path) -> Vec<DeviceBook> {
    let mut out = Vec::new();
    walk(mount, mount, 0, &mut out);
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<DeviceBook>) {
    if depth > MAX_DEPTH || out.len() >= MAX_BOOKS {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        if ft.is_dir() {
            let lower = name.to_ascii_lowercase();
            if name.starts_with('.') || SKIPPED_DIRS.contains(&lower.as_str()) {
                continue;
            }
            walk(root, &path, depth + 1, out);
        } else if ft.is_file() && is_book_file(&name) {
            let meta = entry.metadata().ok();
            let modified = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64);
            let rel = path
                .strip_prefix(root)
                .map(|p| {
                    p.components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join("/")
                })
                .unwrap_or_else(|_| name.clone());
            out.push(DeviceBook {
                path,
                rel,
                size: meta.map(|m| m.len()).unwrap_or(0),
                modified,
            });
        }
    }
}

fn is_book_file(name: &str) -> bool {
    !name.starts_with('.')
        && name
            .rsplit_once('.')
            .is_some_and(|(_, e)| BOOK_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// What [`remove_book`] deleted.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Removed {
    pub files: Vec<PathBuf>,
}

/// Deletes one book from the device, plus its reading-state folder (`<name>.sdr`) next to it.
/// Refuses anything that is not a book file inside `mount` (also through `..` or symbolic links).
pub fn remove_book(mount: &Path, book: &Path) -> Result<Removed> {
    let real_mount = std::fs::canonicalize(mount)
        .with_context(|| format!("device is not mounted: {}", mount.display()))?;
    let real = std::fs::canonicalize(book).with_context(|| format!("no such file: {book:?}"))?;
    if !real.starts_with(&real_mount) || real == real_mount {
        bail!("{} is not on the device", book.display());
    }
    let meta = std::fs::symlink_metadata(&real)?;
    let name = real
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !meta.is_file() || !is_book_file(&name) {
        bail!("{} is not a book file", book.display());
    }
    std::fs::remove_file(&real).with_context(|| format!("removing {}", real.display()))?;
    let mut removed = Removed { files: vec![real] };
    if let Some(parent) = book.parent() {
        let sdr = crate::reading::sidecar_dir(&parent.join(&name));
        if sdr.is_dir() && std::fs::remove_dir_all(&sdr).is_ok() {
            removed.files.push(sdr);
        }
    }
    Ok(removed)
}

// -- save-path templates ------------------------------------------------------------------------

/// What a template can use.
#[derive(Debug, Clone, Default)]
pub struct BookFields {
    pub title: String,
    pub authors: Vec<String>,
    pub series: Option<String>,
    pub series_index: Option<f64>,
    pub language: String,
}

/// Fields a template may mention.
pub const TEMPLATE_FIELDS: &[&str] = &[
    "title",
    "author",
    "authors",
    "author_sort",
    "series",
    "series_index",
    "language",
];

const MAX_COMPONENT: usize = 80;

/// A rendered template: folders below the target folder, and the file name without extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavePath {
    pub dirs: Vec<String>,
    pub stem: String,
}

/// Why `template` cannot be used (unknown field, unbalanced braces, no file name), if so.
pub fn template_problem(template: &str) -> Option<String> {
    let sample = BookFields {
        title: "Title".into(),
        authors: vec!["Author".into()],
        series: Some("Series".into()),
        series_index: Some(1.0),
        language: "en".into(),
    };
    render_template(template, &sample)
        .err()
        .map(|e| format!("{e:#}"))
}

/// Expands `{title}`, `{author}`, `{authors}`, `{author_sort}`, `{series}`, `{series_index}` and
/// `{language}` in a `/`-separated template such as `{author}/{series}/{series_index} - {title}`.
/// Parts that come out empty are dropped (a book without a series simply has no series folder),
/// every part is made safe for FAT/ext4 and `..` can never survive. An empty template means the
/// file goes straight into the target folder, named after the title.
pub fn render_template(template: &str, f: &BookFields) -> Result<SavePath> {
    let template = if template.trim().is_empty() {
        "{title}"
    } else {
        template
    };
    let mut parts: Vec<String> = Vec::new();
    for raw in template.split('/') {
        let text = expand(raw, f)?;
        let text = text
            .trim()
            .trim_start_matches("- ")
            .trim_end_matches(" -")
            .trim();
        if text.is_empty() || text == "-" {
            continue;
        }
        let safe = limit(&sanitize_file_name(text), MAX_COMPONENT);
        if safe == ".." || safe == "." || safe.is_empty() {
            continue;
        }
        parts.push(safe);
    }
    let stem = match parts.pop() {
        Some(s) => s,
        None => {
            let t = sanitize_file_name(&f.title);
            limit(&t, MAX_COMPONENT)
        }
    };
    if parts.len() > 4 {
        bail!("the template has too many folders (at most 4)");
    }
    Ok(SavePath { dirs: parts, stem })
}

fn expand(raw: &str, f: &BookFields) -> Result<String> {
    let mut out = String::new();
    let mut rest = raw;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after
            .find('}')
            .context("an opening brace in the template has no closing brace")?;
        let name = after[..close].trim();
        out.push_str(&field(name, f)?);
        rest = &after[close + 1..];
    }
    if rest.contains('}') {
        bail!("a closing brace in the template has no opening brace");
    }
    out.push_str(rest);
    Ok(out)
}

fn field(name: &str, f: &BookFields) -> Result<String> {
    Ok(match name {
        "title" => f.title.clone(),
        "author" => f.authors.first().cloned().unwrap_or_default(),
        "authors" => f.authors.join(", "),
        "author_sort" => f
            .authors
            .first()
            .map(|a| author_sort(a))
            .unwrap_or_default(),
        "series" => f.series.clone().unwrap_or_default(),
        "series_index" => match f.series_index {
            Some(i) if i.fract() == 0.0 => format!("{:02}", i as i64),
            Some(i) => format!("{i}"),
            None => String::new(),
        },
        "language" => f.language.clone(),
        other => bail!(
            "unknown field {{{other}}}; use one of {}",
            TEMPLATE_FIELDS
                .iter()
                .map(|n| format!("{{{n}}}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    })
}

/// "Ursula K. Le Guin" becomes "Guin, Ursula K. Le": last word first (good enough for a folder).
fn author_sort(name: &str) -> String {
    if name.contains(',') {
        return name.to_string();
    }
    let words: Vec<&str> = name.split_whitespace().collect();
    match words.split_last() {
        Some((last, first)) if !first.is_empty() => format!("{last}, {}", first.join(" ")),
        _ => name.to_string(),
    }
}

fn limit(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    s[..cut].trim_end().to_string()
}

/// True if `rel` is a plain relative path without `..` (used by tests and callers that join).
pub fn is_plain_relative(rel: &Path) -> bool {
    rel.components().all(|c| matches!(c, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new() -> Tmp {
            let p = std::env::temp_dir().join(format!("bookx-devfs-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
        fn file(&self, rel: &str, bytes: &[u8]) -> PathBuf {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, bytes).unwrap();
            p
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    fn fields() -> BookFields {
        BookFields {
            title: "A Wizard of Earthsea".into(),
            authors: vec!["Ursula K. Le Guin".into(), "Other".into()],
            series: Some("Earthsea".into()),
            series_index: Some(1.0),
            language: "en".into(),
        }
    }

    #[test]
    fn sizes_read_like_a_device_reports_them() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(999), "999 B");
        assert_eq!(human_size(1500), "1.5 kB");
        assert_eq!(human_size(12_300_000_000), "12.3 GB");
    }

    #[test]
    fn free_space_of_a_real_directory() {
        let t = Tmp::new();
        let s = free_space(&t.0).expect("statvfs works on the temp dir");
        assert!(s.total >= s.free && s.total > 0);
        assert!(s.describe().contains("free of"));
        assert_eq!(free_space(&t.0.join("nope")), None);
    }

    #[test]
    fn cards_are_told_from_main_memory_by_name() {
        for card in [
            "SDCARD", "sd card", "SD_Card", "EXT_SD", "microSD", "SD", "My Card",
        ] {
            assert_eq!(
                storage_of(Path::new("/run/media/u").join(card).as_path()),
                Storage::Card,
                "{card}"
            );
        }
        for main in [
            "KOBOeReader",
            "Kindle",
            "Boox",
            "SANSA",
            "PB632",
            "Standards",
        ] {
            assert_eq!(
                storage_of(Path::new("/run/media/u").join(main).as_path()),
                Storage::Main,
                "{main}"
            );
        }
        assert_eq!(Storage::Card.label(), "SD card");
    }

    #[test]
    fn listing_finds_books_and_skips_the_rest() {
        let t = Tmp::new();
        t.file("Books/a.epub", b"12345");
        t.file("Books/Sub/b.kepub.epub", b"1");
        t.file("documents/c.azw3", b"123");
        t.file(".kobo/hidden.epub", b"x");
        t.file("Books/.hidden.epub", b"x");
        t.file("Books/a.sdr/metadata.epub.lua", b"x");
        t.file("system/s.epub", b"x");
        t.file("Books/notes.txt.bak", b"x");
        t.file("Books/readme.jpg", b"x");
        let l = list_books(&t.0);
        let rels: Vec<&str> = l.iter().map(|b| b.rel.as_str()).collect();
        assert_eq!(
            rels,
            vec!["Books/Sub/b.kepub.epub", "Books/a.epub", "documents/c.azw3"]
        );
        assert_eq!(l[1].size, 5);
        assert!(l[1].modified.is_some());
        assert_eq!(l[0].stem(), "b");
        assert_eq!(l[0].format(), "kepub.epub");
        assert_eq!(l[2].format(), "azw3");
    }

    #[test]
    fn listing_does_not_follow_symlinks_out_of_the_device() {
        let t = Tmp::new();
        let outside = Tmp::new();
        outside.file("secret.epub", b"x");
        std::os::unix::fs::symlink(&outside.0, t.0.join("Books")).unwrap();
        assert!(list_books(&t.0).is_empty());
    }

    #[test]
    fn removing_deletes_the_book_and_its_sidecar_only() {
        let t = Tmp::new();
        let book = t.file("Books/a.epub", b"1");
        t.file("Books/a.sdr/metadata.epub.lua", b"x");
        let other = t.file("Books/b.epub", b"2");
        let r = remove_book(&t.0, &book).unwrap();
        assert_eq!(r.files.len(), 2);
        assert!(!book.exists() && !t.0.join("Books/a.sdr").exists());
        assert!(other.exists());
    }

    #[test]
    fn removing_refuses_files_outside_or_not_books() {
        let t = Tmp::new();
        let outside = Tmp::new();
        let evil = outside.file("x.epub", b"1");
        assert!(remove_book(&t.0, &evil).is_err());
        assert!(evil.exists());
        let sneaky = t.0.join("../").join(evil.strip_prefix("/").unwrap());
        assert!(remove_book(&t.0, &sneaky).is_err());
        let text = t.file("Books/notes.txt.old", b"1");
        assert!(remove_book(&t.0, &text).is_err());
        assert!(remove_book(&t.0, &t.0).is_err());
        assert!(remove_book(&t.0, &t.0.join("Books/missing.epub")).is_err());
        std::os::unix::fs::symlink(&evil, t.0.join("link.epub")).unwrap();
        assert!(remove_book(&t.0, &t.0.join("link.epub")).is_err());
        assert!(evil.exists());
    }

    #[test]
    fn templates_expand_and_drop_empty_parts() {
        let p = render_template("{author}/{series}/{series_index} - {title}", &fields()).unwrap();
        assert_eq!(p.dirs, vec!["Ursula K. Le Guin", "Earthsea"]);
        assert_eq!(p.stem, "01 - A Wizard of Earthsea");
        let mut f = fields();
        f.series = None;
        f.series_index = None;
        let p = render_template("{author}/{series}/{series_index} - {title}", &f).unwrap();
        assert_eq!(p.dirs, vec!["Ursula K. Le Guin"]);
        assert_eq!(p.stem, "A Wizard of Earthsea");
        let p = render_template("{author_sort}/{title}", &fields()).unwrap();
        assert_eq!(p.dirs, vec!["Guin, Ursula K. Le"]);
        let p = render_template("{authors} {language}", &fields()).unwrap();
        assert_eq!(p.stem, "Ursula K. Le Guin, Other en");
        f.series_index = Some(2.5);
        assert_eq!(render_template("{series_index}", &f).unwrap().stem, "2.5");
    }

    #[test]
    fn an_empty_template_is_the_plain_title() {
        let p = render_template("  ", &fields()).unwrap();
        assert!(p.dirs.is_empty());
        assert_eq!(p.stem, "A Wizard of Earthsea");
    }

    #[test]
    fn bad_templates_are_explained() {
        assert!(template_problem("{nope}/{title}")
            .unwrap()
            .contains("unknown field"));
        assert!(template_problem("{title").unwrap().contains("closing"));
        assert!(template_problem("title}").unwrap().contains("opening"));
        assert!(template_problem("a/b/c/d/e/{title}")
            .unwrap()
            .contains("too many"));
        assert_eq!(template_problem("{author}/{title}"), None);
        assert_eq!(template_problem(""), None);
    }

    #[test]
    fn hostile_metadata_cannot_escape_the_target_folder() {
        let mut f = fields();
        f.title = "../../etc/passwd".into();
        f.authors = vec!["..".into()];
        let p = render_template("{author}/{title}", &f).unwrap();
        for d in &p.dirs {
            assert!(is_plain_relative(Path::new(d)), "{d}");
        }
        assert!(!p.stem.contains('/'));
        let p = render_template("../{title}", &f).unwrap();
        assert!(p.dirs.iter().all(|d| d != ".."));
        let mut f = fields();
        f.title = "x".repeat(500);
        assert!(render_template("{title}", &f).unwrap().stem.len() <= MAX_COMPONENT);
        f.title = String::new();
        assert_eq!(render_template("", &f).unwrap().stem, "book");
    }
}
