//! Repair of the colour and style of highlights that are already in a library.
//!
//! Highlights imported or synced before the device looks were understood (see
//! `docs/highlight-sync-rca.md`) were all stored with Bookx's default look, yellow and plain. The
//! devices still know the truth: KOReader's `.sdr` sidecars, the Kobo database, the raw records the
//! built-in server received (`koreader-annotations.json`) and the ledgers Bookx kept. This module
//! reads those *sources* into [`Truth`] rows, matches them with the library's highlights **by text**
//! (a device says no position Bookx can use) and corrects only the look.
//!
//! Rules, so that running it twice, or on a library the reader has been colouring, is harmless:
//!
//! * Nothing is created and nothing is deleted. Only `color` and `style` of an existing highlight change.
//! * A highlight is corrected only while it still has Bookx's default look (the reader never chose
//!   it: every import used to give it) and only to what a device said. A look that is anything
//!   else is the reader's choice and stays; the device's ledger entry (`devstyle::DeviceLooks`)
//!   protects a look the reader changed after the device was last read.
//! * Words are brought to their canonical form (`lighten` is `background`, `Grey` is `gray`); a word Bookx
//!   does not know is left as it is.
//! * Two highlights with the same words are told apart by their creation time; when that is not
//!   enough and the device's rows disagree, they are counted as ambiguous and left alone.
//! * The whole change is one entry in the library's operation log (`Undo`).
//!
//! No GTK, no network. Reading a source is bounded (entries, depth, file sizes) and never follows a
//! symbolic link.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use bookx_core::devstyle::{self, DeviceLooks, Look};
use bookx_core::library::{Book, Highlight, Library};

use crate::syncplan::{self, Pairing, SideBook};

/// Name of the marker file in `<library>/.bookx/`; bump it when the rules change so every library is
/// looked at once more.
pub const PASS: &str = "highlight-looks-v1";

/// Most files and folders one scan of a device looks at.
const MAX_ENTRIES: usize = 100_000;
/// Deepest folder a scan goes into.
const MAX_DEPTH: usize = 8;
/// Largest sidecar or store file that is read.
const MAX_FILE: u64 = 64 * 1024 * 1024;

/// What one source says about the look of one highlight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Truth {
    /// Where it was read, for the report (`KOReader sidecar`, `Kobo database`, ...).
    pub source: String,
    /// The library book, when the source knows it (a ledger does).
    pub book_id: Option<u64>,
    pub title: String,
    pub authors: Vec<String>,
    /// The library highlight, when the source knows it (a ledger does).
    pub highlight: Option<String>,
    pub text: String,
    /// Creation time in seconds, as the device wrote it.
    pub created: Option<i64>,
    /// Canonical colour word; `None` when the device said nothing.
    pub color: Option<String>,
    /// Canonical style word; `None` when the device said nothing.
    pub style: Option<String>,
}

impl Truth {
    fn says_something(&self) -> bool {
        self.color.is_some() || self.style.is_some()
    }
}

fn split_authors(s: &str) -> Vec<String> {
    s.split(['\n', ';'])
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty())
        .collect()
}

fn small_file(p: &Path) -> Option<String> {
    let m = std::fs::symlink_metadata(p).ok()?;
    if !m.is_file() || m.len() > MAX_FILE {
        return None;
    }
    std::fs::read_to_string(p).ok()
}

// -- sources ------------------------------------------------------------------------------------

/// The highlights of one KOReader `metadata.*.lua`. `folder_name` is the name of the `.sdr` folder,
/// used as the title when the file has none.
pub fn truths_from_sidecar(text: &str, folder_name: &str, source: &str) -> Vec<Truth> {
    let side = crate::reading::parse_sidecar(text);
    let root = crate::lua::parse(text);
    let props = root.as_ref().and_then(|r| r.get("doc_props"));
    let title = props
        .and_then(|p| p.str("title"))
        .map(str::to_string)
        .filter(|t| !t.trim().is_empty())
        .or_else(|| folder_name.strip_suffix(".sdr").map(str::to_string))
        .unwrap_or_default();
    if title.is_empty() {
        return Vec::new();
    }
    let authors = props
        .and_then(|p| p.str("authors"))
        .map(split_authors)
        .unwrap_or_default();
    side.notes
        .into_iter()
        .filter(|n| !n.bookmark && !n.text.trim().is_empty())
        .map(|n| Truth {
            source: source.to_string(),
            book_id: None,
            title: title.clone(),
            authors: authors.clone(),
            highlight: None,
            text: n.text,
            created: n.created.as_deref().and_then(crate::reading::parse_time),
            color: n.color,
            style: n.style,
        })
        .collect()
}

/// The highlights of a Kobo database (read from a private copy).
pub fn truths_from_kobo(db: &Path) -> Vec<Truth> {
    let Ok(rows) = crate::kobo::read_book_notes(db) else {
        return Vec::new();
    };
    rows.into_iter()
        .filter(|b| !b.note.text.trim().is_empty())
        .map(|b| Truth {
            source: "Kobo database".into(),
            book_id: None,
            title: b.title,
            authors: b.author.as_deref().map(split_authors).unwrap_or_default(),
            highlight: None,
            text: b.note.text,
            created: b
                .note
                .created
                .as_deref()
                .and_then(crate::reading::parse_time),
            color: b.note.color,
            style: b.note.style,
        })
        .collect()
}

/// Everything a device folder says: its Kobo database and every KOReader sidecar below it. Bounded
/// and read only; symbolic links are not followed.
pub fn truths_from_folder(dir: &Path) -> Vec<Truth> {
    let mut out = Vec::new();
    if let Some(db) = crate::devices::kobo_db_path(dir) {
        out.extend(truths_from_kobo(&db));
    }
    let mut budget = MAX_ENTRIES;
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            if budget == 0 {
                return out;
            }
            budget -= 1;
            let Ok(t) = e.file_type() else { continue };
            if t.is_symlink() {
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if t.is_dir() {
                if depth < MAX_DEPTH && name != ".kobo" {
                    stack.push((e.path(), depth + 1));
                }
            } else if name.starts_with("metadata.") && name.ends_with(".lua") {
                if let Some(text) = small_file(&e.path()) {
                    let folder = d
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    out.extend(truths_from_sidecar(&text, &folder, "KOReader sidecar"));
                }
            }
        }
    }
    out
}

/// A path that is a KOReader sidecar, a Kobo database or a device folder.
pub fn truths_from_path(path: &Path) -> Vec<Truth> {
    let Ok(m) = std::fs::symlink_metadata(path) else {
        return Vec::new();
    };
    if m.is_dir() {
        return truths_from_folder(path);
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.ends_with(".lua") {
        let folder = path
            .parent()
            .and_then(|d| d.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        return small_file(path)
            .map(|t| truths_from_sidecar(&t, &folder, "KOReader sidecar"))
            .unwrap_or_default();
    }
    if name.ends_with(".sqlite") || name.ends_with(".db") {
        return truths_from_kobo(path);
    }
    Vec::new()
}

/// The raw records the built-in server received from the KOReader plugin
/// (`koreader-annotations.json`): the colour and drawer exactly as the plugin sent them.
pub fn truths_from_annotation_store(path: &Path) -> Vec<Truth> {
    let Ok(m) = std::fs::symlink_metadata(path) else {
        return Vec::new();
    };
    if !m.is_file() || m.len() > MAX_FILE {
        return Vec::new();
    }
    let Ok(store) = crate::annotation_sync::AnnotationSync::open(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (user, doc, title, authors) in store.documents() {
        for r in store.records(&user, &doc, 0) {
            let a = &r.annotation;
            if r.origin != crate::annotation_sync::ORIGIN_KOREADER
                || a.deleted
                || a.kind != crate::annotation_sync::KIND_HIGHLIGHT
                || a.text.trim().is_empty()
            {
                continue;
            }
            let (color, style) = devstyle::look_of(
                Some(a.color.as_str()).filter(|c| !c.trim().is_empty()),
                Some(a.drawer.as_str()).filter(|c| !c.trim().is_empty()),
            );
            out.push(Truth {
                source: "Bookx sync records".into(),
                book_id: None,
                title: title.clone(),
                authors: split_authors(&authors),
                highlight: None,
                text: a.text.clone(),
                created: Some(a.created).filter(|c| *c > 0),
                color,
                style,
            });
        }
    }
    out
}

/// What Bookx itself remembered about the device's look: `annotation-sync.json` (the look the
/// device had at the last wireless sync, per highlight) in `config_dir`, and `.bookx/device-looks.json`
/// (the look at the last USB read) in the library.
pub fn truths_from_ledgers(config_dir: Option<&Path>, library: &Path) -> Vec<Truth> {
    let mut out = Vec::new();
    if let Some(v) = config_dir
        .and_then(|c| small_file(&c.join("annotation-sync.json")))
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
    {
        if let Some(map) = v.get("incoming").and_then(|m| m.as_object()) {
            for e in map.values() {
                let (Some(book), Some(h)) = (
                    e.get("book_id").and_then(|b| b.as_u64()),
                    e.get("highlight").and_then(|h| h.as_str()),
                ) else {
                    continue;
                };
                if e.get("state").and_then(|s| s.as_str()) != Some("linked") {
                    continue;
                }
                let look = e.get("device_look");
                let word = |k: &str| {
                    look.and_then(|l| l.get(k))
                        .and_then(|w| w.as_str())
                        .filter(|w| !w.trim().is_empty())
                };
                let (color, style) = devstyle::look_of(word("color"), word("style"));
                out.push(Truth {
                    source: "Bookx sync ledger".into(),
                    book_id: Some(book),
                    title: String::new(),
                    authors: Vec::new(),
                    highlight: Some(h.to_string()),
                    text: String::new(),
                    created: None,
                    color,
                    style,
                });
            }
        }
    }
    for (key, look) in DeviceLooks::load(library).looks {
        let Some((book, id)) = key.split_once('/') else {
            continue;
        };
        let Ok(book) = book.parse::<u64>() else {
            continue;
        };
        let (color, style) = devstyle::look_of(
            Some(look.color.as_str()).filter(|c| !c.is_empty()),
            Some(look.style.as_str()).filter(|c| !c.is_empty()),
        );
        out.push(Truth {
            source: "Bookx USB sync ledger".into(),
            book_id: Some(book),
            title: String::new(),
            authors: Vec::new(),
            highlight: Some(id.to_string()),
            text: String::new(),
            created: None,
            color,
            style,
        });
    }
    out
}

// -- the plan -----------------------------------------------------------------------------------

/// One highlight whose look is to change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fix {
    pub book: u64,
    pub book_title: String,
    pub highlight: String,
    pub text: String,
    pub from: Look,
    pub to: Look,
    /// Where the new look came from (empty for a spelling brought to its canonical form).
    pub source: String,
}

/// Counts of what the repair found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Rows the sources gave.
    pub rows: usize,
    /// Highlights whose look was taken from a device.
    pub fixed: usize,
    /// Highlights whose words were only brought to their canonical form.
    pub normalised: usize,
    /// Highlights the device agrees with already.
    pub right: usize,
    /// Highlights with another look than the device's that the reader chose: left alone.
    pub kept: usize,
    /// Highlights that several device rows could be (same words, different looks): left alone.
    pub ambiguous: usize,
    /// Rows of a device for which the library has no highlight (nothing is created).
    pub no_highlight: usize,
    /// Titles of device books the library does not have (or has more than once).
    pub unmatched_books: Vec<String>,
    /// The sources that gave rows, with how many.
    pub sources: BTreeMap<String, usize>,
}

impl Report {
    /// Highlights that change.
    pub fn changed(&self) -> usize {
        self.fixed + self.normalised
    }

    pub fn text(&self) -> String {
        format!(
            "{} highlight(s) corrected from a device, {} spelling(s) normalised, {} already right, {} left because you chose their look, {} ambiguous, {} device highlight(s) not in the library, {} book(s) not matched",
            self.fixed,
            self.normalised,
            self.right,
            self.kept,
            self.ambiguous,
            self.no_highlight,
            self.unmatched_books.len()
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub fixes: Vec<Fix>,
    pub report: Report,
}

fn side_books(books: &[Book]) -> Vec<SideBook> {
    books
        .iter()
        .map(|b| SideBook {
            id: b.id.to_string(),
            title: b.meta.title.clone(),
            authors: b.meta.authors.clone(),
            progress: 0.0,
            status: crate::reading::Status::Reading,
            last_read: None,
            notes: Vec::new(),
        })
        .collect()
}

fn is_pdf(h: &Highlight) -> bool {
    h.style.starts_with("pdf|")
}

fn look_of(h: &Highlight) -> Look {
    devstyle::full_look(&h.color, &h.style)
}

/// What a truth asks of a highlight: the new look (the parts it does not say stay).
fn target(h: &Highlight, t: &Truth) -> Look {
    let own = look_of(h);
    Look {
        color: t.color.clone().unwrap_or(own.color),
        style: t.style.clone().unwrap_or(own.style),
    }
}

/// A fix decided for the highlight `(book index, highlight index)`.
type Decided = (usize, usize, Look, String);

struct Ctx<'a> {
    books: &'a [Book],
    ledger: &'a DeviceLooks,
    decided: HashSet<(usize, usize)>,
    fixes: Vec<Decided>,
    report: Report,
}

impl Ctx<'_> {
    /// Decides what a truth does to one highlight.
    fn consider(&mut self, bi: usize, hi: usize, t: &Truth) {
        if !self.decided.insert((bi, hi)) {
            return;
        }
        let book = &self.books[bi];
        let h = &book.highlights[hi];
        let to = target(h, t);
        let own = look_of(h);
        if to == own {
            self.report.right += 1;
            return;
        }
        let reader_changed = self
            .ledger
            .looks
            .get(&DeviceLooks::key(book.id, &h.id))
            .is_some_and(|l| devstyle::full_look(&l.color, &l.style) != own);
        if !devstyle::is_default_look(&h.color, &h.style) || reader_changed {
            self.report.kept += 1;
            return;
        }
        self.fixes.push((bi, hi, to, t.source.clone()));
    }
}

/// Works out what the repair would change. Changes nothing.
pub fn plan(books: &[Book], ledger: &DeviceLooks, truths: &[Truth]) -> Plan {
    let mut cx = Ctx {
        books,
        ledger,
        decided: HashSet::new(),
        fixes: Vec::new(),
        report: Report {
            rows: truths.len(),
            ..Report::default()
        },
    };
    for t in truths {
        *cx.report.sources.entry(t.source.clone()).or_default() += 1;
    }
    let sides = side_books(books);
    let mut unmatched: BTreeSet<String> = BTreeSet::new();
    // book index and normalised text -> the rows that say something about those words
    let mut groups: BTreeMap<(usize, String), Vec<&Truth>> = BTreeMap::new();
    // rows a ledger knows the highlight of
    let mut named: Vec<(usize, &Truth)> = Vec::new();

    for t in truths.iter().filter(|t| t.says_something()) {
        let bi = match t.book_id {
            Some(id) => books.iter().position(|b| b.id == id),
            None => match syncplan::pair(&sides, &t.title, &t.authors) {
                Pairing::One(i) => Some(i),
                _ => {
                    unmatched.insert(t.title.clone());
                    continue;
                }
            },
        };
        let Some(bi) = bi else { continue };
        if t.highlight.is_some() {
            named.push((bi, t));
        } else if !syncplan::norm(&t.text).is_empty() {
            groups
                .entry((bi, syncplan::norm(&t.text)))
                .or_default()
                .push(t);
        }
    }

    // what a device says now beats what a ledger remembers from an earlier read, so the rows
    // that carry text are decided first and the ledgers only fill in what is left
    for ((bi, key), rows) in groups {
        let book = &books[bi];
        let same_words: Vec<usize> = book
            .highlights
            .iter()
            .enumerate()
            .filter(|(_, h)| !is_pdf(h) && syncplan::norm(&h.text) == key)
            .map(|(i, _)| i)
            .collect();
        if same_words.is_empty() {
            cx.report.no_highlight += rows.len();
            continue;
        }
        let mut libs = same_words;
        libs.retain(|i| !cx.decided.contains(&(bi, *i)));
        if libs.is_empty() {
            continue;
        }
        let mut rest: Vec<&Truth> = Vec::new();
        // first by the time the highlight was made
        for t in rows {
            let hit = t.created.and_then(|c| {
                libs.iter()
                    .position(|&i| book.highlights[i].created.timestamp() == c)
            });
            match hit {
                Some(p) => {
                    let hi = libs.remove(p);
                    cx.consider(bi, hi, t);
                }
                None => rest.push(t),
            }
        }
        if rest.is_empty() {
            continue;
        }
        if libs.is_empty() {
            cx.report.no_highlight += rest.len();
            continue;
        }
        // the rest: one to one, or every row says the same
        let same_look = rest
            .windows(2)
            .all(|w| w[0].color == w[1].color && w[0].style == w[1].style);
        if (libs.len() == 1 && rest.len() == 1) || (same_look && libs.len() <= rest.len()) {
            for hi in libs {
                cx.consider(bi, hi, rest[0]);
            }
        } else {
            cx.report.ambiguous += libs.len();
        }
    }
    for (bi, t) in named {
        let hid = t.highlight.as_deref().unwrap_or_default();
        match books[bi].highlights.iter().position(|h| h.id == hid) {
            Some(hi) => cx.consider(bi, hi, t),
            None => cx.report.no_highlight += 1,
        }
    }

    let mut out = Plan::default();
    // spellings: `lighten`, `Grey`, `underscore` stored as such become the canonical word
    for (bi, book) in books.iter().enumerate() {
        for (hi, h) in book.highlights.iter().enumerate() {
            if is_pdf(h) || cx.decided.contains(&(bi, hi)) {
                continue;
            }
            let color = devstyle::canonical_color(&h.color)
                .filter(|c| *c != h.color && !h.color.trim().is_empty());
            let style = devstyle::canonical_style(&h.style)
                .filter(|s| *s != h.style && !h.style.trim().is_empty());
            if color.is_none() && style.is_none() {
                continue;
            }
            out.fixes.push(Fix {
                book: book.id,
                book_title: book.meta.title.clone(),
                highlight: h.id.clone(),
                text: h.text.clone(),
                from: Look {
                    color: h.color.clone(),
                    style: h.style.clone(),
                },
                to: Look {
                    color: color.unwrap_or_else(|| h.color.clone()),
                    style: style.unwrap_or_else(|| h.style.clone()),
                },
                source: String::new(),
            });
            cx.report.normalised += 1;
        }
    }
    for (bi, hi, to, source) in std::mem::take(&mut cx.fixes) {
        let b = &books[bi];
        let h = &b.highlights[hi];
        out.fixes.push(Fix {
            book: b.id,
            book_title: b.meta.title.clone(),
            highlight: h.id.clone(),
            text: h.text.clone(),
            from: Look {
                color: h.color.clone(),
                style: h.style.clone(),
            },
            to,
            source,
        });
        cx.report.fixed += 1;
    }
    cx.report.unmatched_books = unmatched.into_iter().collect();
    out.report = cx.report;
    out
}

/// Applies a plan: changes `color` and `style` of the planned highlights, saves, and logs one
/// undoable step. The device ledger is not written: a corrected highlight no longer has the default
/// look, which already protects it, and an undo leaves nothing behind that would stop a second run.
/// Returns how many highlights changed.
pub fn apply(lib: &mut Library, plan: &Plan) -> Result<usize> {
    if plan.fixes.is_empty() {
        return Ok(0);
    }
    let mut before: BTreeMap<u64, Vec<Highlight>> = BTreeMap::new();
    let mut changed = 0;
    for f in &plan.fixes {
        let Some(book) = lib.get_mut(f.book) else {
            continue;
        };
        let snapshot = book.highlights.clone();
        let Some(h) = book.highlights.iter_mut().find(|h| h.id == f.highlight) else {
            continue;
        };
        if h.color == f.to.color && h.style == f.to.style {
            continue;
        }
        h.color = f.to.color.clone();
        h.style = f.to.style.clone();
        before.entry(f.book).or_insert(snapshot);
        changed += 1;
    }
    if changed > 0 {
        lib.save()?;
        lib.record_highlight_edit(
            format!("Corrected the colour of {changed} highlights from the device"),
            before.into_iter().collect(),
        );
    }
    Ok(changed)
}

// -- the one-time pass --------------------------------------------------------------------------

/// The marker file of the one-time pass.
pub fn marker(root: &Path) -> PathBuf {
    root.join(bookx_core::history::STATE_DIR).join(PASS)
}

/// The pass that runs once when a library is opened: `truths` are what the available sources said.
/// Without any source the spellings are still normalised but the marker is **not** written, so the
/// pass is tried again when a device or a ledger is there; once a source was read the marker is
/// written and the library is not touched again by this pass (the CLI repairs on request, and
/// the sync keeps looks right from then on). `None` when it was done before, the library is
/// read-only, or nothing changed.
pub fn one_time(lib: &mut Library, truths: &[Truth]) -> Option<Report> {
    if lib.is_read_only() || marker(&lib.root).exists() {
        return None;
    }
    let ledger = DeviceLooks::load(&lib.root);
    let p = plan(&lib.books, &ledger, truths);
    apply(lib, &p).ok()?;
    if truths.iter().any(Truth::says_something) {
        let m = marker(&lib.root);
        if let Some(d) = m.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(
            m,
            format!(
                "rows {} fixed {} normalised {} right {}\n",
                p.report.rows, p.report.fixed, p.report.normalised, p.report.right
            ),
        );
    }
    (p.report.changed() > 0).then_some(p.report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn book(id: u64, title: &str, author: &str, hs: Vec<Highlight>) -> Book {
        serde_json::from_value::<Book>(serde_json::json!({
            "id": id, "file": "a.epub",
            "meta": {"title": title, "title_sort": "", "authors": [author], "author_sort": "", "publisher": "", "language": "", "description": "", "tags": [], "series": "", "series_index": 0.0, "identifiers": [], "published": "", "rights": "", "rating": 0},
            "added": Utc::now(), "modified": Utc::now(), "size": 1,
            "highlights": hs,
        }))
        .unwrap()
    }

    fn hl(id: &str, text: &str, color: &str, style: &str) -> Highlight {
        Highlight {
            id: id.into(),
            spine: 0,
            start: 0,
            end: 1,
            color: color.into(),
            style: style.into(),
            note: String::new(),
            text: text.into(),
            created: Utc.with_ymd_and_hms(2026, 8, 17, 11, 22, 15).unwrap(),
        }
    }

    fn truth(text: &str, color: &str, style: &str) -> Truth {
        Truth {
            source: "KOReader sidecar".into(),
            book_id: None,
            title: "Dune".into(),
            authors: vec!["Frank Herbert".into()],
            highlight: None,
            text: text.into(),
            created: None,
            color: Some(color.to_string()).filter(|c| !c.is_empty()),
            style: Some(style.to_string()).filter(|c| !c.is_empty()),
        }
    }

    fn lib_with(hs: Vec<Highlight>) -> Vec<Book> {
        vec![book(1, "Dune", "Frank Herbert", hs)]
    }

    const FEAR: &str = "Fear is the mind-killer, said the old woman.";

    #[test]
    fn a_default_look_takes_what_the_device_said_and_nothing_is_created() {
        let books = lib_with(vec![
            hl("a", FEAR, "yellow", ""),
            hl("b", "The spice must flow through everything.", "yellow", ""),
        ]);
        let truths = [
            truth(FEAR, "purple", "background"),
            truth(
                "The spice must flow through everything.",
                "olive",
                "underline",
            ),
            truth(
                "A quote the library has never had in it.",
                "red",
                "background",
            ),
        ];
        let p = plan(&books, &DeviceLooks::default(), &truths);
        assert_eq!(p.report.fixed, 2);
        assert_eq!(p.report.no_highlight, 1, "nothing is created for it");
        assert_eq!(p.fixes[0].to.color, "purple");
        assert_eq!(
            (p.fixes[1].to.color.as_str(), p.fixes[1].to.style.as_str()),
            ("olive", "underline")
        );
    }

    #[test]
    fn a_look_the_reader_chose_is_left_alone() {
        let books = lib_with(vec![hl("a", FEAR, "blue", "underline")]);
        let p = plan(
            &books,
            &DeviceLooks::default(),
            &[truth(FEAR, "purple", "background")],
        );
        assert!(p.fixes.is_empty());
        assert_eq!(p.report.kept, 1);
    }

    #[test]
    fn what_the_device_agrees_with_is_not_a_change() {
        let books = lib_with(vec![hl("a", FEAR, "yellow", "")]);
        let p = plan(
            &books,
            &DeviceLooks::default(),
            &[truth(FEAR, "yellow", "background")],
        );
        assert!(p.fixes.is_empty());
        assert_eq!(p.report.right, 1);
    }

    #[test]
    fn the_same_words_twice_are_told_apart_by_their_time_or_left_alone() {
        let words = "The same sentence marked twice in this book.";
        let mut second = hl("b", words, "yellow", "");
        second.created = Utc.with_ymd_and_hms(2026, 8, 18, 9, 0, 0).unwrap();
        let books = lib_with(vec![hl("a", words, "yellow", ""), second]);
        let mut t1 = truth(words, "red", "background");
        t1.created = Some(
            Utc.with_ymd_and_hms(2026, 8, 17, 11, 22, 15)
                .unwrap()
                .timestamp(),
        );
        let mut t2 = truth(words, "green", "background");
        t2.created = Some(
            Utc.with_ymd_and_hms(2026, 8, 18, 9, 0, 0)
                .unwrap()
                .timestamp(),
        );
        let p = plan(&books, &DeviceLooks::default(), &[t1, t2]);
        let to = |id: &str| {
            p.fixes
                .iter()
                .find(|f| f.highlight == id)
                .unwrap()
                .to
                .color
                .clone()
        };
        assert_eq!((to("a").as_str(), to("b").as_str()), ("red", "green"));
        // without times, two rows that disagree cannot be assigned
        let p = plan(
            &books,
            &DeviceLooks::default(),
            &[
                truth(words, "red", "background"),
                truth(words, "green", "background"),
            ],
        );
        assert!(p.fixes.is_empty());
        assert_eq!(p.report.ambiguous, 2);
        // rows that all say the same can
        let p = plan(
            &books,
            &DeviceLooks::default(),
            &[
                truth(words, "red", "background"),
                truth(words, "red", "background"),
            ],
        );
        assert_eq!(p.fixes.len(), 2);
    }

    #[test]
    fn a_ledger_names_the_highlight_and_protects_a_look_changed_after_it() {
        let books = lib_with(vec![
            hl("a", "First quote of the book here.", "yellow", ""),
            hl("b", "Second quote of the book here.", "pink", "underline"),
        ]);
        let mk = |id: &str, c: &str| Truth {
            source: "Bookx sync ledger".into(),
            book_id: Some(1),
            title: String::new(),
            authors: vec![],
            highlight: Some(id.into()),
            text: String::new(),
            created: None,
            color: Some(c.into()),
            style: Some("background".into()),
        };
        let p = plan(
            &books,
            &DeviceLooks::default(),
            &[mk("a", "cyan"), mk("b", "cyan")],
        );
        assert_eq!(p.fixes.len(), 1);
        assert_eq!(p.fixes[0].highlight, "a");
        assert_eq!(p.report.kept, 1);
        // the ledger says the device had green, and the library still has yellow plain: the reader
        // changed it back after the device was last read
        let mut ledger = DeviceLooks::default();
        ledger.looks.insert(
            DeviceLooks::key(1, "a"),
            Look {
                color: "green".into(),
                style: "background".into(),
            },
        );
        let books2 = lib_with(vec![hl("a", "First quote of the book here.", "yellow", "")]);
        let p = plan(&books2, &ledger, &[mk("a", "cyan")]);
        assert!(p.fixes.is_empty(), "the ledger says the reader changed it");
    }

    #[test]
    fn spellings_are_normalised_and_unknown_words_stay() {
        let books = lib_with(vec![
            hl(
                "a",
                "Quote with a spelling that is not canonical.",
                "Grey",
                "underscore",
            ),
            hl(
                "b",
                "Quote with a word nobody knows about yet.",
                "teal",
                "wavy",
            ),
        ]);
        let p = plan(&books, &DeviceLooks::default(), &[]);
        assert_eq!(p.report.normalised, 1);
        assert_eq!(p.fixes[0].to.color, "gray");
        assert_eq!(p.fixes[0].to.style, "underline");
    }

    #[test]
    fn books_are_matched_by_title_and_unmatched_ones_are_reported() {
        let books = lib_with(vec![hl("a", FEAR, "yellow", "")]);
        let mut other = truth(FEAR, "red", "background");
        other.title = "Some Other Book".into();
        let p = plan(&books, &DeviceLooks::default(), &[other]);
        assert!(p.fixes.is_empty());
        assert_eq!(
            p.report.unmatched_books,
            vec!["Some Other Book".to_string()]
        );
    }

    #[test]
    fn a_sidecar_text_gives_one_row_per_highlight_with_its_look() {
        let lua = include_str!("../tests/fixtures/koreader_all_looks.lua");
        let rows = truths_from_sidecar(lua, "Looks.sdr", "KOReader sidecar");
        assert_eq!(rows.len(), 36);
        assert_eq!(rows[0].title, "Looks of Highlights");
        assert_eq!(rows[0].authors, vec!["Ann Author".to_string()]);
        assert_eq!(rows[0].color.as_deref(), Some("red"));
        assert!(rows[0].created.is_some());
    }
}
