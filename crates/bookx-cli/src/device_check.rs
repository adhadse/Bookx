//! `bookx-cli device-check <mount>`: validates the folder layout of a mounted e-reader and the books
//! on it. Pure Rust and strictly read-only: it never writes to the device (the Kobo database is
//! read from a private copy), so it is safe to run on a reader somebody is using. Tested with fake
//! mounts.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use bookx_serve::devices::{DeviceKind, classify, kobo_db_path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warn,
    Fail,
}

impl Level {
    fn label(self) -> &'static str {
        match self {
            Level::Ok => "ok",
            Level::Warn => "warn",
            Level::Fail => "FAIL",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub level: Level,
    /// Path relative to the mount (or `.` for the mount itself)
    pub path: String,
    pub message: String,
}

#[derive(Debug)]
pub struct Report {
    pub kind: Option<DeviceKind>,
    pub books_checked: usize,
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn count(&self, level: Level) -> usize {
        self.findings.iter().filter(|f| f.level == level).count()
    }
    pub fn passed(&self) -> bool {
        self.count(Level::Fail) == 0
    }
}

const FAT32_MAX: u64 = 4 * 1024 * 1024 * 1024 - 1;
const BOOK_EXTS: &[&str] = &[
    "epub", "kepub", "azw3", "azw", "mobi", "prc", "pdf", "fb2", "txt", "cbz", "djvu", "doc",
    "docx", "rtf",
];

struct Ctx {
    root: PathBuf,
    findings: Vec<Finding>,
}

impl Ctx {
    fn add(&mut self, level: Level, path: &Path, msg: impl Into<String>) {
        let rel = path.strip_prefix(&self.root).unwrap_or(path);
        let rel = rel.to_string_lossy();
        self.findings.push(Finding {
            level,
            path: if rel.is_empty() {
                ".".into()
            } else {
                rel.into_owned()
            },
            message: msg.into(),
        });
    }
}

/// Checks the device mounted at `mount`. `kind` overrides the layout-based detection. `only` limits the
/// book check to one file (a path relative to the mount, or a bare file name searched in the books folder).
/// `all` checks every book on the device instead of the folder Bookx sends to.
pub fn check_device_with(
    mount: &Path,
    kind: Option<DeviceKind>,
    only: Option<&str>,
    all: bool,
) -> Result<Report> {
    if !mount.is_dir() {
        bail!("{} is not a directory", mount.display());
    }
    let mut cx = Ctx {
        root: mount.to_path_buf(),
        findings: vec![],
    };
    let detected = classify(mount);
    let Some(kind) = kind.or(detected) else {
        cx.add(
            Level::Fail,
            mount,
            "no e-reader layout recognised (expected .kobo, documents+system, system/config or a Books folder)",
        );
        return Ok(Report {
            kind: None,
            books_checked: 0,
            findings: cx.findings,
        });
    };
    if detected != Some(kind) {
        cx.add(
            Level::Warn,
            mount,
            format!(
                "layout looks like {}, checking as {}",
                detected.map_or("nothing known", |k| k.label()),
                kind.label()
            ),
        );
    }
    cx.add(Level::Ok, mount, format!("device kind: {}", kind.label()));
    layout(&mut cx, mount, kind);
    if kind == DeviceKind::Kobo {
        kobo_facts(&mut cx, mount);
    }

    let books_dir = mount.join(kind.target_subdir(mount));
    let mut files = vec![];
    if let Some(name) = only {
        let direct = mount.join(name);
        let in_dir = books_dir.join(name);
        match [direct, in_dir].into_iter().find(|p| p.is_file()) {
            Some(p) => files.push(p),
            None => cx.add(
                Level::Fail,
                Path::new(name),
                "the sent book is not on the device",
            ),
        }
    } else if all {
        files = bookx_serve::devfs::list_books(mount)
            .into_iter()
            .map(|b| b.path)
            .collect();
        if files.is_empty() {
            cx.add(Level::Warn, mount, "no books on the device");
        }
    } else if books_dir.is_dir() {
        collect_books(&books_dir, 0, &mut files);
        files.sort();
        if files.is_empty() {
            cx.add(Level::Warn, &books_dir, "no books in the books folder yet");
        }
    } else {
        cx.add(
            Level::Warn,
            &books_dir,
            "books folder does not exist yet (created on first send)",
        );
    }
    let mut lower_names = std::collections::HashMap::new();
    for f in &files {
        if let Some(prev) = lower_names.insert(f.to_string_lossy().to_lowercase(), f.clone()) {
            cx.add(
                Level::Fail,
                f,
                format!(
                    "differs from {} only by case (FAT cannot hold both)",
                    prev.display()
                ),
            );
        }
        check_book(&mut cx, f, kind);
    }
    Ok(Report {
        kind: Some(kind),
        books_checked: files.len(),
        findings: cx.findings,
    })
}

/// Model, firmware, free space and what the library database holds. Read-only.
fn kobo_facts(cx: &mut Ctx, mount: &Path) {
    let vfile = mount.join(".kobo/version");
    match std::fs::read_to_string(&vfile)
        .ok()
        .and_then(|t| bookx_serve::kobo::parse_version(&t))
    {
        Some(v) => cx.add(Level::Ok, &vfile, v.describe()),
        None if vfile.is_file() => cx.add(
            Level::Warn,
            &vfile,
            "version file is not in the usual format",
        ),
        None => {}
    }
    if let Some(sp) = bookx_serve::devfs::free_space(mount) {
        cx.add(Level::Ok, mount, sp.describe());
    }
    let Some(db) = kobo_db_path(mount) else {
        return;
    };
    let rel = mount.join(".kobo/KoboReader.sqlite");
    match bookx_serve::kobo::db_info(&db) {
        Ok(i) => {
            cx.add(
                Level::Ok,
                &rel,
                format!(
                    "schema version {}, {} journal, {} book row(s) (+{} files below .adds), {} shelf(ves), {} highlight(s)",
                    i.schema_version
                        .map_or("unknown".to_string(), |v| v.to_string()),
                    if i.wal { "WAL" } else { "rollback" },
                    i.book_rows,
                    i.other_rows,
                    i.shelves,
                    i.highlights
                ),
            );
            let files = bookx_serve::devfs::list_books(mount);
            let known: std::collections::HashSet<_> = bookx_serve::kobo::read_states(&db)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|(id, _)| {
                    bookx_serve::kobo::rel_of_content_id(&id).map(|r| mount.join(r))
                })
                .collect();
            let unknown = files.iter().filter(|b| !known.contains(&b.path)).count();
            if unknown > 0 {
                cx.add(
                    Level::Warn,
                    mount,
                    format!(
                        "{unknown} of {} book file(s) are not in the Kobo database (never imported by the firmware); their reading state, if any, is in KOReader sidecars",
                        files.len()
                    ),
                );
            }
        }
        Err(e) => cx.add(
            Level::Warn,
            &rel,
            format!("library database could not be read: {e:#}"),
        ),
    }
}

fn layout(cx: &mut Ctx, mount: &Path, kind: DeviceKind) {
    let need_dir = |cx: &mut Ctx, rel: &str, level: Level| {
        let p = mount.join(rel);
        if p.is_dir() {
            cx.add(Level::Ok, &p, "folder present");
        } else {
            cx.add(level, &p, "folder missing");
        }
    };
    match kind {
        DeviceKind::Kobo => {
            need_dir(cx, ".kobo", Level::Fail);
            if kobo_db_path(mount).is_some() {
                cx.add(
                    Level::Ok,
                    &mount.join(".kobo/KoboReader.sqlite"),
                    "library database present",
                );
            } else {
                cx.add(
                    Level::Warn,
                    &mount.join(".kobo/KoboReader.sqlite"),
                    "no library database: the Kobo has not finished its first import",
                );
            }
            if !mount.join(".kobo/version").is_file() {
                cx.add(
                    Level::Warn,
                    &mount.join(".kobo/version"),
                    "firmware version file missing",
                );
            }
        }
        DeviceKind::Kindle => {
            need_dir(cx, "documents", Level::Fail);
            need_dir(cx, "system", Level::Fail);
        }
        DeviceKind::PocketBook => {
            if mount.join("system/config").exists() {
                cx.add(Level::Ok, &mount.join("system/config"), "present");
            } else {
                cx.add(Level::Fail, &mount.join("system/config"), "missing");
            }
        }
        DeviceKind::Generic => {
            let books = kind.target_subdir(mount);
            if mount.join(&books).is_dir() {
                cx.add(Level::Ok, &mount.join(&books), "folder present");
            } else {
                cx.add(Level::Fail, &mount.join(&books), "Books folder missing");
            }
        }
    }
}

fn collect_books(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let hidden = e.file_name().to_string_lossy().starts_with('.');
        if p.is_dir() {
            if depth < 4 && !hidden {
                collect_books(&p, depth + 1, out);
            }
        } else if !hidden && is_book(&p) {
            out.push(p);
        }
    }
}

fn is_book(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| BOOK_EXTS.contains(&e.to_lowercase().as_str()))
}

/// Characters FAT/exFAT and Windows hosts reject in file names.
fn bad_name_chars(name: &str) -> Vec<char> {
    name.chars()
        .filter(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\') || c.is_control())
        .collect()
}

fn check_book(cx: &mut Ctx, path: &Path, kind: DeviceKind) {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let lower = name.to_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("").to_string();
    let bad = bad_name_chars(&name);
    if !bad.is_empty() {
        cx.add(
            Level::Fail,
            path,
            format!("file name has characters FAT cannot store: {bad:?}"),
        );
    }
    if name.len() > 255 {
        cx.add(Level::Fail, path, "file name is longer than 255 bytes");
    }
    if name.ends_with('.') || name.ends_with(' ') {
        cx.add(
            Level::Fail,
            path,
            "file name ends with a dot or space (FAT drops it)",
        );
    }
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        cx.add(Level::Fail, path, "file is empty (an interrupted copy?)");
        return;
    }
    if size > FAT32_MAX {
        cx.add(
            Level::Fail,
            path,
            "file is 4 GiB or larger and cannot live on FAT32",
        );
    }
    let Ok(mut f) = std::fs::File::open(path) else {
        cx.add(Level::Fail, path, "cannot be read");
        return;
    };
    let mut head = vec![0u8; 4096.min(size as usize)];
    if f.read_exact(&mut head).is_err() {
        cx.add(Level::Fail, path, "cannot be read");
        return;
    }
    let is_kepub = lower.ends_with(".kepub.epub") || ext == "kepub";
    let supported: &[&str] = match kind {
        DeviceKind::Kobo => &["epub", "kepub", "pdf", "mobi", "txt", "cbz", "rtf"],
        DeviceKind::Kindle => &["azw3", "azw", "mobi", "prc", "pdf", "txt", "epub"],
        DeviceKind::PocketBook => &[
            "epub", "pdf", "fb2", "mobi", "txt", "djvu", "doc", "docx", "rtf", "cbz",
        ],
        DeviceKind::Generic => BOOK_EXTS,
    };
    if !supported.contains(&ext.as_str()) {
        cx.add(
            Level::Warn,
            path,
            format!(".{ext} is not a format a {} opens", kind.label()),
        );
    }
    match ext.as_str() {
        "epub" | "kepub" => {
            let epub = check_epub(&head, path);
            match epub {
                Ok(info) => {
                    cx.add(Level::Ok, path, "valid EPUB container");
                    if kind == DeviceKind::Kobo {
                        if info.has_kobo_spans && !is_kepub {
                            cx.add(Level::Warn, path, "contains Kobo spans but is not named .kepub.epub; Kobo will treat it as plain EPUB");
                        } else if is_kepub && !info.has_kobo_spans {
                            cx.add(
                                Level::Fail,
                                path,
                                "named .kepub.epub but has no koboSpan markup: not a real KEPUB",
                            );
                        } else if !is_kepub {
                            cx.add(Level::Warn, path, "plain EPUB works on Kobo, but send KEPUB for reading stats and fonts");
                        } else {
                            cx.add(
                                Level::Ok,
                                path,
                                format!("KEPUB markup present ({} spans)", info.spans),
                            );
                            for m in &info.kepub_fail {
                                cx.add(Level::Fail, path, m.clone());
                            }
                            for m in info.kepub_warn.iter().take(5) {
                                cx.add(Level::Warn, path, m.clone());
                            }
                        }
                    } else if is_kepub {
                        cx.add(
                            Level::Warn,
                            path,
                            "KEPUB is for Kobo only; other readers show it as EPUB",
                        );
                    }
                    if kind == DeviceKind::Kindle {
                        cx.add(Level::Warn, path, "Kindle opens EPUB only on recent firmware or after Send-to-Kindle; AZW3 is safer");
                    }
                }
                Err(e) => cx.add(Level::Fail, path, e),
            }
        }
        "mobi" | "azw" | "azw3" | "prc" => match check_mobi(path, &head) {
            Ok(version) => {
                cx.add(
                    Level::Ok,
                    path,
                    format!("valid PalmDB/MOBI container (MOBI header version {version})"),
                );
                if ext == "azw3" && version != 8 && !contains(path, b"BOUNDARY") {
                    cx.add(
                        Level::Fail,
                        path,
                        "named .azw3 but has no KF8 (version 8) header",
                    );
                }
                if ext == "mobi" && version == 8 {
                    cx.add(Level::Warn, path, "KF8 content in a .mobi file; use .azw3");
                }
            }
            Err(e) => cx.add(Level::Fail, path, e),
        },
        "pdf" => {
            if head.starts_with(b"%PDF-") {
                cx.add(Level::Ok, path, "PDF header present");
            } else {
                cx.add(Level::Fail, path, "does not start with %PDF-");
            }
        }
        "cbz" => {
            if head.starts_with(b"PK") {
                cx.add(Level::Ok, path, "ZIP header present");
            } else {
                cx.add(Level::Fail, path, "not a ZIP file");
            }
        }
        _ => {}
    }
}

fn contains(path: &Path, needle: &[u8]) -> bool {
    std::fs::read(path).is_ok_and(|b| b.windows(needle.len()).any(|w| w == needle))
}

struct EpubInfo {
    has_kobo_spans: bool,
    /// `koboSpan` markup checks: (fail, warn) messages.
    kepub_fail: Vec<String>,
    kepub_warn: Vec<String>,
    spans: usize,
}

/// Structure of the Kobo spans in one document: ids are `kobo.<paragraph>.<sentence>`, unique, the
/// text sits inside the `book-columns`/`book-inner` wrapper, and the document is well-formed XML.
fn kepub_doc_problems(name: &str, text: &str) -> (usize, Vec<String>, Vec<String>) {
    let (mut fail, mut warn) = (Vec::new(), Vec::new());
    let doc = match roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    ) {
        Ok(d) => d,
        Err(e) => {
            fail.push(format!("{name} is not well-formed XML: {e}"));
            return (0, fail, warn);
        }
    };
    let mut seen = std::collections::HashSet::new();
    let mut spans = 0;
    for n in doc.descendants().filter(|n| {
        n.is_element()
            && n.tag_name().name() == "span"
            && n.attribute("class")
                .is_some_and(|c| c.split_whitespace().any(|w| w == "koboSpan"))
    }) {
        spans += 1;
        let id = n.attribute("id").unwrap_or("");
        let ok = id
            .strip_prefix("kobo.")
            .and_then(|r| r.split_once('.'))
            .is_some_and(|(p, s)| {
                !p.is_empty()
                    && !s.is_empty()
                    && p.chars().all(|c| c.is_ascii_digit())
                    && s.chars().all(|c| c.is_ascii_digit())
            });
        if !ok {
            warn.push(format!(
                "{name}: span id {id:?} is not kobo.<paragraph>.<sentence>"
            ));
        }
        if !seen.insert(id.to_string()) {
            fail.push(format!("{name}: span id {id:?} appears twice"));
        }
    }
    if spans > 0 {
        let has = |id: &str| doc.descendants().any(|n| n.attribute("id") == Some(id));
        if !(has("book-columns") && has("book-inner")) {
            warn.push(format!(
                "{name}: no book-columns/book-inner wrapper around the text"
            ));
        }
    }
    (spans, fail, warn)
}

fn check_epub(head: &[u8], path: &Path) -> Result<EpubInfo, String> {
    if !head.starts_with(b"PK\x03\x04") {
        return Err("not a ZIP file (truncated or wrong format)".into());
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| format!("broken ZIP (incomplete copy?): {e}"))?;
    {
        let first = zip.by_index(0).map_err(|e| e.to_string())?;
        if first.name().map_or(true, |n| n != "mimetype") {
            return Err("first ZIP entry is not `mimetype`".into());
        }
        if first.compression() != zip::CompressionMethod::Stored {
            return Err("`mimetype` is compressed".into());
        }
    }
    let mut mt = String::new();
    zip.by_name("mimetype")
        .map_err(|e| e.to_string())?
        .read_to_string(&mut mt)
        .map_err(|e| e.to_string())?;
    if mt != "application/epub+zip" {
        return Err(format!("`mimetype` says {mt:?}"));
    }
    let mut container = String::new();
    zip.by_name("META-INF/container.xml")
        .map_err(|_| "META-INF/container.xml missing".to_string())?
        .read_to_string(&mut container)
        .map_err(|e| e.to_string())?;
    let opf = container
        .split("full-path=\"")
        .nth(1)
        .and_then(|r| r.split('"').next())
        .ok_or("container.xml names no package document")?
        .to_string();
    if zip.by_name(&opf).is_err() {
        return Err(format!("package document {opf} missing"));
    }
    let names: Vec<String> = zip
        .file_names()
        .filter_map(|n| n.ok().map(|c| c.into_owned()))
        .collect();
    let mut info = EpubInfo {
        has_kobo_spans: false,
        kepub_fail: Vec::new(),
        kepub_warn: Vec::new(),
        spans: 0,
    };
    for n in names
        .iter()
        .filter(|n| n.ends_with(".xhtml") || n.ends_with(".html"))
    {
        let mut s = String::new();
        if let Ok(mut e) = zip.by_name(n) {
            if e.read_to_string(&mut s).is_ok() && s.contains("koboSpan") {
                info.has_kobo_spans = true;
                let (spans, fail, warn) = kepub_doc_problems(n, &s);
                info.spans += spans;
                info.kepub_fail.extend(fail);
                info.kepub_warn.extend(warn);
            }
        }
    }
    Ok(info)
}

fn check_mobi(path: &Path, head: &[u8]) -> Result<u32, String> {
    if head.len() < 82 || &head[60..68] != b"BOOKMOBI" {
        return Err("PalmDB header does not say BOOKMOBI".into());
    }
    let rec0 = u32::from_be_bytes([head[78], head[79], head[80], head[81]]) as usize;
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if rec0 + 40 > bytes.len() {
        return Err("first record lies past the end of the file (truncated)".into());
    }
    if &bytes[rec0 + 16..rec0 + 20] != b"MOBI" {
        return Err("no MOBI header in the first record".into());
    }
    let v = u32::from_be_bytes([
        bytes[rec0 + 36],
        bytes[rec0 + 37],
        bytes[rec0 + 38],
        bytes[rec0 + 39],
    ]);
    Ok(v)
}

pub fn write_report(r: &Report, json: bool, out: &mut dyn Write) -> Result<()> {
    if json {
        let list: Vec<_> = r
            .findings
            .iter()
            .map(|f| serde_json::json!({"level": f.level.label(), "path": f.path, "message": f.message}))
            .collect();
        writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "device": r.kind.map(|k| k.slug()),
                "books_checked": r.books_checked,
                "failures": r.count(Level::Fail),
                "warnings": r.count(Level::Warn),
                "findings": list,
            }))?
        )?;
    } else {
        for f in &r.findings {
            writeln!(out, "{:<5} {}: {}", f.level.label(), f.path, f.message)?;
        }
        writeln!(
            out,
            "{} book(s) checked, {} failure(s), {} warning(s): {}",
            r.books_checked,
            r.count(Level::Fail),
            r.count(Level::Warn),
            if r.passed() { "PASS" } else { "FAIL" }
        )?;
    }
    Ok(())
}
