//! Compare two books, two files or two package documents (E4/E25).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use bookx_core::archive::Archive;
use bookx_core::book::OpenBook;
use bookx_core::format::pretty_print_xml;
use bookx_core::opf::{Metadata, Package};
use bookx_core::util::extension;
use serde::{Deserialize, Serialize};
use similar::{Algorithm, ChangeTag, TextDiff};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Granularity {
    Line,
    Word,
    Char,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpKind {
    Equal,
    Insert,
    Delete,
    /// Consecutive delete + insert merged into one change.
    Replace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffOp {
    pub kind: OpKind,
    pub old: String,
    pub new: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffStats {
    pub equal: usize,
    pub added: usize,
    pub removed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChanges {
    pub ops: Vec<DiffOp>,
    pub stats: DiffStats,
}

impl TextChanges {
    pub fn is_identical(&self) -> bool {
        self.stats.added == 0 && self.stats.removed == 0
    }
}

fn differ<'a>(
    old: &'a str,
    new: &'a str,
    g: Granularity,
    patience: bool,
) -> TextDiff<'a, 'a, 'a, str> {
    let mut cfg = TextDiff::configure();
    cfg.algorithm(if patience {
        Algorithm::Patience
    } else {
        Algorithm::Myers
    });
    match g {
        Granularity::Line => cfg.diff_lines(old, new),
        Granularity::Word => cfg.diff_words(old, new),
        Granularity::Char => cfg.diff_chars(old, new),
    }
}

/// Diffs two texts at line, word or character level. Adjacent deletions and insertions are merged into `Replace`.
/// With `patience`, the patience algorithm is used (usually nicer for code), otherwise Myers.
pub fn diff_text(old: &str, new: &str, g: Granularity, patience: bool) -> TextChanges {
    let d = differ(old, new, g, patience);
    let mut ops: Vec<DiffOp> = Vec::new();
    let mut stats = DiffStats::default();
    for c in d.iter_all_changes() {
        let v = c.value();
        let (kind, o, n) = match c.tag() {
            ChangeTag::Equal => {
                stats.equal += 1;
                (OpKind::Equal, v, v)
            }
            ChangeTag::Delete => {
                stats.removed += 1;
                (OpKind::Delete, v, "")
            }
            ChangeTag::Insert => {
                stats.added += 1;
                (OpKind::Insert, "", v)
            }
        };
        match ops.last_mut() {
            Some(last) if last.kind == kind => {
                last.old.push_str(o);
                last.new.push_str(n);
            }
            Some(last)
                if matches!(kind, OpKind::Insert | OpKind::Delete)
                    && matches!(last.kind, OpKind::Insert | OpKind::Delete | OpKind::Replace) =>
            {
                last.kind = OpKind::Replace;
                last.old.push_str(o);
                last.new.push_str(n);
            }
            _ => ops.push(DiffOp {
                kind,
                old: o.to_string(),
                new: n.to_string(),
            }),
        }
    }
    TextChanges { ops, stats }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineChangeKind {
    /// The line is new.
    Added,
    /// The line replaces another one.
    Modified,
    /// Lines were deleted just before this one.
    Removed,
}

/// One marked line of the new text (0-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineChange {
    pub line: usize,
    pub kind: LineChangeKind,
}

/// Which lines of `new` differ from `old`, for the editor's change bar. A replaced block marks its
/// common length as modified and the rest as added; deleted lines mark the line that follows them
/// (the last line when they were at the end). Sorted, at most one entry per line.
pub fn changed_lines(old: &str, new: &str) -> Vec<LineChange> {
    use similar::DiffOp as Op;
    if old == new {
        return Vec::new();
    }
    let d = TextDiff::configure()
        .algorithm(Algorithm::Patience)
        .diff_lines(old, new);
    let total = new.lines().count().max(1);
    let mut out: Vec<LineChange> = Vec::new();
    for op in d.ops() {
        match *op {
            Op::Equal { .. } => {}
            Op::Insert {
                new_index, new_len, ..
            } => {
                for l in new_index..new_index + new_len {
                    out.push(LineChange {
                        line: l,
                        kind: LineChangeKind::Added,
                    });
                }
            }
            Op::Delete { new_index, .. } => out.push(LineChange {
                line: new_index.min(total - 1),
                kind: LineChangeKind::Removed,
            }),
            Op::Replace {
                old_len,
                new_index,
                new_len,
                ..
            } => {
                for k in 0..new_len {
                    out.push(LineChange {
                        line: new_index + k,
                        kind: if k < old_len {
                            LineChangeKind::Modified
                        } else {
                            LineChangeKind::Added
                        },
                    });
                }
                if new_len < old_len {
                    out.push(LineChange {
                        line: (new_index + new_len).min(total - 1),
                        kind: LineChangeKind::Removed,
                    });
                }
            }
        }
    }
    out.sort_by_key(|c| c.line);
    out.dedup_by_key(|c| c.line);
    out
}

/// Unified diff (`--- a`, `+++ b`, `@@` hunks) at line level. Empty when the texts are equal.
pub fn unified_diff(
    old_name: &str,
    new_name: &str,
    old: &str,
    new: &str,
    context: usize,
) -> String {
    if old == new {
        return String::new();
    }
    TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(context)
        .header(old_name, new_name)
        .to_string()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Word-level diff rendered as HTML: deletions in `<del>`, insertions in `<ins>`, everything escaped.
pub fn html_diff(old: &str, new: &str) -> String {
    let mut out = String::from("<div class=\"diff\">");
    for op in diff_text(old, new, Granularity::Word, false).ops {
        match op.kind {
            OpKind::Equal => out.push_str(&html_escape(&op.new)),
            OpKind::Insert => out.push_str(&format!("<ins>{}</ins>", html_escape(&op.new))),
            OpKind::Delete => out.push_str(&format!("<del>{}</del>", html_escape(&op.old))),
            OpKind::Replace => out.push_str(&format!(
                "<del>{}</del><ins>{}</ins>",
                html_escape(&op.old),
                html_escape(&op.new)
            )),
        }
    }
    out.push_str("</div>");
    out
}

// ---------------------------------------------------------------------------
// Archives
// ---------------------------------------------------------------------------

pub fn is_text_path(path: &str) -> bool {
    matches!(
        extension(path).as_str(),
        "xhtml"
            | "html"
            | "htm"
            | "xht"
            | "css"
            | "opf"
            | "ncx"
            | "xml"
            | "svg"
            | "txt"
            | "js"
            | "smil"
    ) || path == "mimetype"
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub text: bool,
    pub old_size: usize,
    pub new_size: usize,
    /// Line-level statistics for text files.
    pub stats: Option<DiffStats>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<FileChange>,
    /// Files whose bytes are identical but whose path differs (only reported for files that were added+removed).
    pub renamed: Vec<(String, String)>,
    pub unchanged: usize,
}

impl ArchiveDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.changed.is_empty()
            && self.renamed.is_empty()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CompareOptions {
    /// Pretty-print XML-ish files before comparing so formatting differences do not show up.
    pub normalize_xml: bool,
}

fn comparable_text(path: &str, data: &[u8], opts: CompareOptions) -> String {
    let t = String::from_utf8_lossy(data)
        .trim_start_matches('\u{feff}')
        .to_string();
    if opts.normalize_xml
        && matches!(
            extension(path).as_str(),
            "xhtml" | "xht" | "opf" | "ncx" | "xml" | "svg"
        )
    {
        pretty_print_xml(&t).unwrap_or(t)
    } else {
        t
    }
}

pub fn compare_archives(a: &Archive, b: &Archive, opts: CompareOptions) -> ArchiveDiff {
    let mut d = ArchiveDiff::default();
    let mut names: BTreeSet<&String> = BTreeSet::new();
    names.extend(a.names());
    names.extend(b.names());
    for n in names {
        match (a.get(n), b.get(n)) {
            (Some(x), Some(y)) => {
                let text = is_text_path(n);
                let (same, stats) = if text {
                    let (tx, ty) = (comparable_text(n, x, opts), comparable_text(n, y, opts));
                    if tx == ty {
                        (true, None)
                    } else {
                        (
                            false,
                            Some(diff_text(&tx, &ty, Granularity::Line, false).stats),
                        )
                    }
                } else {
                    (x == y, None)
                };
                if same {
                    d.unchanged += 1;
                } else {
                    d.changed.push(FileChange {
                        path: n.clone(),
                        text,
                        old_size: x.len(),
                        new_size: y.len(),
                        stats,
                    });
                }
            }
            (Some(_), None) => d.removed.push(n.clone()),
            (None, Some(_)) => d.added.push(n.clone()),
            (None, None) => {}
        }
    }
    let mut by_content: HashMap<&[u8], &String> = HashMap::new();
    for r in &d.removed {
        if let Some(data) = a.get(r) {
            by_content.entry(data).or_insert(r);
        }
    }
    let mut renamed = Vec::new();
    for ad in &d.added {
        if let Some(data) = b.get(ad) {
            if let Some(old) = by_content.get(data) {
                renamed.push(((*old).clone(), ad.clone()));
            }
        }
    }
    d.added.retain(|x| !renamed.iter().any(|(_, n)| n == x));
    d.removed.retain(|x| !renamed.iter().any(|(o, _)| o == x));
    renamed.sort();
    d.renamed = renamed;
    d
}

/// Archive-level comparison of two books.
pub fn compare_books(a: &OpenBook, b: &OpenBook, opts: CompareOptions) -> ArchiveDiff {
    compare_archives(&a.archive, &b.archive, opts)
}

/// Unified diff of one file present in both archives (`None` if absent in either or identical binary).
pub fn file_unified_diff(
    a: &Archive,
    b: &Archive,
    path: &str,
    opts: CompareOptions,
) -> Option<String> {
    let (x, y) = (a.get(path)?, b.get(path)?);
    if !is_text_path(path) {
        return None;
    }
    Some(unified_diff(
        &format!("a/{path}"),
        &format!("b/{path}"),
        &comparable_text(path, x, opts),
        &comparable_text(path, y, opts),
        3,
    ))
}

/// Unified diff of every changed text file, concatenated.
pub fn archive_unified_diff(a: &Archive, b: &Archive, opts: CompareOptions) -> String {
    let d = compare_archives(a, b, opts);
    let mut out = String::new();
    for c in d.changed.iter().filter(|c| c.text) {
        if let Some(u) = file_unified_diff(a, b, &c.path, opts) {
            out.push_str(&u);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Side by side (CE-224..CE-227)
// ---------------------------------------------------------------------------

/// How a row of the side-by-side view looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Equal,
    /// Only on the new side.
    Added,
    /// Only on the old side.
    Removed,
    /// A line on both sides that differs; the words that differ are in [`Cell::marks`].
    Changed,
}

/// One line on one side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// 1-based line number in that side's text.
    pub line: usize,
    pub text: String,
    /// Byte ranges of `text` that differ (words), for `Changed` rows only.
    pub marks: Vec<std::ops::Range<usize>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Line {
        kind: RowKind,
        old: Option<Cell>,
        new: Option<Cell>,
    },
    /// Equal lines that are not shown.
    Skip(usize),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SideBySide {
    pub rows: Vec<Row>,
    /// Number of rows that are not `Equal`.
    pub changed_rows: usize,
}

impl SideBySide {
    pub fn is_identical(&self) -> bool {
        self.changed_rows == 0
    }

    /// Indices of the first row of each run of changes (for next/previous change).
    pub fn change_starts(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut in_run = false;
        for (i, r) in self.rows.iter().enumerate() {
            let changed = matches!(r, Row::Line { kind, .. } if *kind != RowKind::Equal);
            if changed && !in_run {
                out.push(i);
            }
            in_run = changed;
        }
        out
    }

    /// Rows whose old or new text contains `needle`.
    pub fn search(&self, needle: &str, case_sensitive: bool) -> Vec<usize> {
        if needle.is_empty() {
            return Vec::new();
        }
        let n = if case_sensitive {
            needle.to_string()
        } else {
            needle.to_lowercase()
        };
        let hit = |c: &Option<Cell>| {
            c.as_ref().is_some_and(|c| {
                if case_sensitive {
                    c.text.contains(&n)
                } else {
                    c.text.to_lowercase().contains(&n)
                }
            })
        };
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r, Row::Line { old, new, .. } if hit(old) || hit(new)))
            .map(|(i, _)| i)
            .collect()
    }

    /// Row showing line `line` of the new (or, with `old_side`, the old) text, or the nearest one before it.
    pub fn row_of_line(&self, line: usize, old_side: bool) -> Option<usize> {
        let mut best = None;
        for (i, r) in self.rows.iter().enumerate() {
            if let Row::Line { old, new, .. } = r {
                let c = if old_side { old } else { new };
                if let Some(c) = c {
                    if c.line <= line {
                        best = Some(i);
                    }
                }
            }
        }
        best
    }
}

/// Word-level changes inside two lines: byte ranges that differ on each side.
pub fn word_marks(
    old: &str,
    new: &str,
) -> (Vec<std::ops::Range<usize>>, Vec<std::ops::Range<usize>>) {
    let (mut mo, mut mn) = (Vec::new(), Vec::new());
    let (mut po, mut pn) = (0usize, 0usize);
    for op in diff_text(old, new, Granularity::Word, false).ops {
        match op.kind {
            OpKind::Equal => {
                po += op.old.len();
                pn += op.new.len();
            }
            OpKind::Delete => {
                mo.push(po..po + op.old.len());
                po += op.old.len();
            }
            OpKind::Insert => {
                mn.push(pn..pn + op.new.len());
                pn += op.new.len();
            }
            OpKind::Replace => {
                mo.push(po..po + op.old.len());
                mn.push(pn..pn + op.new.len());
                po += op.old.len();
                pn += op.new.len();
            }
        }
    }
    (mo, mn)
}

fn cell(lines: &[&str], i: usize, marks: Vec<std::ops::Range<usize>>) -> Cell {
    Cell {
        line: i + 1,
        text: lines[i].trim_end_matches(['\n', '\r']).to_string(),
        marks,
    }
}

/// Side-by-side diff of two texts: one row per line pair, only changes and `context` lines around them (the
/// rest is folded into `Skip` rows). `context` = `usize::MAX` shows everything.
pub fn side_by_side(old: &str, new: &str, context: usize) -> SideBySide {
    use similar::DiffOp as Op;
    let (ol, nl): (Vec<&str>, Vec<&str>) = (
        old.split_inclusive('\n').collect(),
        new.split_inclusive('\n').collect(),
    );
    let d = TextDiff::configure()
        .algorithm(Algorithm::Patience)
        .diff_lines(old, new);
    let mut all: Vec<Row> = Vec::new();
    for op in d.ops() {
        match *op {
            Op::Equal {
                old_index,
                new_index,
                len,
            } => {
                for k in 0..len {
                    all.push(Row::Line {
                        kind: RowKind::Equal,
                        old: Some(cell(&ol, old_index + k, Vec::new())),
                        new: Some(cell(&nl, new_index + k, Vec::new())),
                    });
                }
            }
            Op::Delete {
                old_index, old_len, ..
            } => {
                for k in 0..old_len {
                    all.push(Row::Line {
                        kind: RowKind::Removed,
                        old: Some(cell(&ol, old_index + k, Vec::new())),
                        new: None,
                    });
                }
            }
            Op::Insert {
                new_index, new_len, ..
            } => {
                for k in 0..new_len {
                    all.push(Row::Line {
                        kind: RowKind::Added,
                        old: None,
                        new: Some(cell(&nl, new_index + k, Vec::new())),
                    });
                }
            }
            Op::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                for k in 0..old_len.max(new_len) {
                    match (k < old_len, k < new_len) {
                        (true, true) => {
                            let (a, b) = (ol[old_index + k], nl[new_index + k]);
                            let (ma, mb) = word_marks(
                                a.trim_end_matches(['\n', '\r']),
                                b.trim_end_matches(['\n', '\r']),
                            );
                            all.push(Row::Line {
                                kind: RowKind::Changed,
                                old: Some(cell(&ol, old_index + k, ma)),
                                new: Some(cell(&nl, new_index + k, mb)),
                            });
                        }
                        (true, false) => all.push(Row::Line {
                            kind: RowKind::Removed,
                            old: Some(cell(&ol, old_index + k, Vec::new())),
                            new: None,
                        }),
                        _ => all.push(Row::Line {
                            kind: RowKind::Added,
                            old: None,
                            new: Some(cell(&nl, new_index + k, Vec::new())),
                        }),
                    }
                }
            }
        }
    }
    let is_change = |r: &Row| matches!(r, Row::Line { kind, .. } if *kind != RowKind::Equal);
    let changed_rows = all.iter().filter(|r| is_change(r)).count();
    if context == usize::MAX {
        return SideBySide {
            rows: all,
            changed_rows,
        };
    }
    let mut keep = vec![false; all.len()];
    for (i, r) in all.iter().enumerate() {
        if is_change(r) {
            let lo = i.saturating_sub(context);
            let hi = (i + context).min(all.len() - 1);
            for k in keep.iter_mut().take(hi + 1).skip(lo) {
                *k = true;
            }
        }
    }
    let mut rows = Vec::new();
    let mut skipped = 0;
    for (r, k) in all.into_iter().zip(keep) {
        if k {
            if skipped > 0 {
                rows.push(Row::Skip(skipped));
                skipped = 0;
            }
            rows.push(r);
        } else {
            skipped += 1;
        }
    }
    if skipped > 0 && !rows.is_empty() {
        rows.push(Row::Skip(skipped));
    }
    SideBySide { rows, changed_rows }
}

/// The text of `path` in `archive` as it is compared (BOM removed; XML pretty-printed with `beautify`).
pub fn text_for_compare(archive: &Archive, path: &str, beautify: bool) -> Option<String> {
    let data = archive.get(path)?;
    if !is_text_path(path) {
        return None;
    }
    Some(comparable_text(
        path,
        data,
        CompareOptions {
            normalize_xml: beautify,
        },
    ))
}

/// Side-by-side diff of one file in two archives. A file missing on one side is compared with an empty text.
pub fn file_side_by_side(
    a: &Archive,
    b: &Archive,
    path: &str,
    beautify: bool,
    context: usize,
) -> Option<SideBySide> {
    if !is_text_path(path) {
        return None;
    }
    let x = text_for_compare(a, path, beautify).unwrap_or_default();
    let y = text_for_compare(b, path, beautify).unwrap_or_default();
    Some(side_by_side(&x, &y, context))
}

// ---------------------------------------------------------------------------
// OPF
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldChange {
    pub field: String,
    pub old: String,
    pub new: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpfDiff {
    pub metadata: Vec<FieldChange>,
    pub manifest_added: Vec<String>,
    pub manifest_removed: Vec<String>,
    /// Same href, different media type or properties.
    pub manifest_changed: Vec<FieldChange>,
    /// Spine as ordered hrefs, only filled when it differs.
    pub spine_old: Vec<String>,
    pub spine_new: Vec<String>,
    pub version_changed: Option<(String, String)>,
}

impl OpfDiff {
    pub fn is_empty(&self) -> bool {
        self.metadata.is_empty()
            && self.manifest_added.is_empty()
            && self.manifest_removed.is_empty()
            && self.manifest_changed.is_empty()
            && self.spine_old == self.spine_new
            && self.version_changed.is_none()
    }
}

fn sorted(v: &[String]) -> String {
    let mut v: Vec<&String> = v.iter().collect();
    v.sort();
    v.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" | ")
}

fn metadata_fields(m: &Metadata) -> BTreeMap<&'static str, String> {
    let mut f = BTreeMap::new();
    f.insert("title", m.title.clone());
    f.insert("title_sort", m.title_sort.clone());
    f.insert("authors", sorted(&m.authors));
    f.insert("author_sort", m.author_sort.clone());
    f.insert("publisher", m.publisher.clone());
    f.insert("language", m.language.clone());
    f.insert("description", m.description.clone());
    f.insert("tags", sorted(&m.tags));
    f.insert("series", m.series.clone());
    f.insert("series_index", m.series_index.to_string());
    f.insert(
        "identifiers",
        sorted(
            &m.identifiers
                .iter()
                .map(|i| format!("{}:{}", i.scheme.to_lowercase(), i.value))
                .collect::<Vec<_>>(),
        ),
    );
    f.insert("published", m.published.clone());
    f.insert("rights", m.rights.clone());
    f.insert("rating", m.rating.to_string());
    f
}

/// Compares the metadata of two packages field by field. Order of multi-valued fields is ignored.
pub fn compare_metadata(a: &Metadata, b: &Metadata) -> Vec<FieldChange> {
    let (fa, fb) = (metadata_fields(a), metadata_fields(b));
    let same_authors = fa["authors"] == fb["authors"];
    fa.iter()
        .filter(|(k, v)| fb[*k] != **v && !(same_authors && **k == "author_sort"))
        .map(|(k, v)| FieldChange {
            field: (*k).to_string(),
            old: v.clone(),
            new: fb[k].clone(),
        })
        .collect()
}

/// Compares two packages ignoring manifest order, item ids and attribute order. Hrefs are compared as
/// archive-absolute paths, so moving the OPF file does not matter either.
pub fn compare_packages(a: &Package, b: &Package) -> OpfDiff {
    let keyed = |p: &Package| -> BTreeMap<String, (String, String)> {
        p.manifest
            .iter()
            .map(|i| {
                let mut props: Vec<&str> = i.properties.split_whitespace().collect();
                props.sort();
                (p.item_path(i), (i.media_type.clone(), props.join(" ")))
            })
            .collect()
    };
    let (ma, mb) = (keyed(a), keyed(b));
    let mut d = OpfDiff {
        metadata: compare_metadata(&a.metadata, &b.metadata),
        ..Default::default()
    };
    for (k, v) in &ma {
        match mb.get(k) {
            None => d.manifest_removed.push(k.clone()),
            Some(w) if w != v => d.manifest_changed.push(FieldChange {
                field: k.clone(),
                old: format!("{} [{}]", v.0, v.1),
                new: format!("{} [{}]", w.0, w.1),
            }),
            _ => {}
        }
    }
    d.manifest_added = mb
        .keys()
        .filter(|k| !ma.contains_key(*k))
        .cloned()
        .collect();
    let (sa, sb) = (a.spine_paths(), b.spine_paths());
    if sa != sb {
        d.spine_old = sa;
        d.spine_new = sb;
    }
    if a.version != b.version {
        d.version_changed = Some((a.version.clone(), b.version.clone()));
    }
    d
}

/// Parses two OPF documents and compares them with [`compare_packages`].
pub fn compare_opf(a_path: &str, a: &str, b_path: &str, b: &str) -> anyhow::Result<OpfDiff> {
    Ok(compare_packages(
        &Package::parse_text(a_path, a)?,
        &Package::parse_text(b_path, b)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_diff_basic() {
        let d = diff_text("a\nb\nc\n", "a\nB\nc\nd\n", Granularity::Line, false);
        assert_eq!(
            d.stats,
            DiffStats {
                equal: 2,
                added: 2,
                removed: 1
            }
        );
        assert_eq!(d.ops[0].kind, OpKind::Equal);
        assert_eq!(d.ops[1].kind, OpKind::Replace);
        assert_eq!(d.ops[1].old, "b\n");
        assert_eq!(d.ops[1].new, "B\n");
        assert_eq!(d.ops[3].kind, OpKind::Insert);
        assert!(!d.is_identical());
    }

    #[test]
    fn identical_and_empty() {
        assert!(diff_text("x", "x", Granularity::Word, false).is_identical());
        assert!(diff_text("", "", Granularity::Line, true).ops.is_empty());
        let d = diff_text("", "new\n", Granularity::Line, false);
        assert_eq!(d.ops.len(), 1);
        assert_eq!(d.ops[0].kind, OpKind::Insert);
        let d = diff_text("old\n", "", Granularity::Line, false);
        assert_eq!(d.ops[0].kind, OpKind::Delete);
    }

    #[test]
    fn word_and_char_levels() {
        let w = diff_text("the quick fox", "the slow fox", Granularity::Word, false);
        assert!(
            w.ops
                .iter()
                .any(|o| o.kind == OpKind::Replace && o.old == "quick" && o.new == "slow")
        );
        let c = diff_text("cat", "cut", Granularity::Char, false);
        assert_eq!(c.stats.added, 1);
        assert_eq!(c.stats.removed, 1);
    }

    #[test]
    fn ops_reconstruct_both_sides() {
        let (a, b) = ("one two three four five\n", "one 2 three five six\n");
        for g in [Granularity::Line, Granularity::Word, Granularity::Char] {
            for patience in [false, true] {
                let d = diff_text(a, b, g, patience);
                let old: String = d.ops.iter().map(|o| o.old.as_str()).collect();
                let new: String = d.ops.iter().map(|o| o.new.as_str()).collect();
                assert_eq!(old, a, "{g:?}");
                assert_eq!(new, b, "{g:?}");
            }
        }
    }

    #[test]
    fn unified_output() {
        let u = unified_diff("a.txt", "b.txt", "1\n2\n3\n", "1\nX\n3\n", 1);
        assert!(u.starts_with("--- a.txt\n+++ b.txt\n@@ -1,3 +1,3 @@"));
        assert!(u.contains("-2\n"));
        assert!(u.contains("+X\n"));
        assert!(u.contains(" 1\n"));
        assert_eq!(unified_diff("a", "b", "same", "same", 3), "");
    }

    #[test]
    fn html_output_escapes() {
        let h = html_diff("a <b> c", "a <i> c & d");
        assert!(h.starts_with("<div class=\"diff\">"));
        assert!(h.contains("<del>&lt;b&gt;</del><ins>&lt;i&gt;</ins>"));
        assert!(h.contains("&amp;"));
        assert!(!h.contains("<b>"));
        assert_eq!(html_diff("x", "x"), "<div class=\"diff\">x</div>");
    }

    fn arc(files: &[(&str, &[u8])]) -> Archive {
        let mut a = Archive::default();
        for (n, d) in files {
            a.set(n, d.to_vec());
        }
        a
    }

    #[test]
    fn archive_summary() {
        let a = arc(&[
            ("same.xhtml", b"<p>x</p>"),
            ("ch.xhtml", b"<p>a</p>\n<p>b</p>\n"),
            ("gone.css", b"p{}"),
            ("img.png", &[1, 2, 3]),
            ("old/pic.jpg", &[9, 9, 9, 9]),
        ]);
        let b = arc(&[
            ("same.xhtml", b"<p>x</p>"),
            ("ch.xhtml", b"<p>a</p>\n<p>c</p>\n"),
            ("new.css", b"a{}"),
            ("img.png", &[1, 2, 4]),
            ("new/pic.jpg", &[9, 9, 9, 9]),
        ]);
        let d = compare_archives(&a, &b, CompareOptions::default());
        assert_eq!(d.unchanged, 1);
        assert_eq!(d.added, vec!["new.css"]);
        assert_eq!(d.removed, vec!["gone.css"]);
        assert_eq!(
            d.renamed,
            vec![("old/pic.jpg".to_string(), "new/pic.jpg".to_string())]
        );
        assert_eq!(d.changed.len(), 2);
        let ch = d.changed.iter().find(|c| c.path == "ch.xhtml").unwrap();
        assert!(ch.text);
        assert_eq!(ch.stats.unwrap().added, 1);
        let img = d.changed.iter().find(|c| c.path == "img.png").unwrap();
        assert!(!img.text);
        assert_eq!(img.stats, None);
        assert!(!d.is_empty());
        assert!(compare_archives(&a, &a, CompareOptions::default()).is_empty());
    }

    #[test]
    fn archive_normalize_xml_ignores_formatting() {
        let a = arc(&[("a.xhtml", b"<html><body><p>x</p></body></html>")]);
        let b = arc(&[(
            "a.xhtml",
            b"<html>\n  <body>\n    <p>x</p>\n  </body>\n</html>\n",
        )]);
        assert_eq!(
            compare_archives(&a, &b, CompareOptions::default())
                .changed
                .len(),
            1
        );
        assert!(
            compare_archives(
                &a,
                &b,
                CompareOptions {
                    normalize_xml: true
                }
            )
            .is_empty()
        );
    }

    #[test]
    fn archive_unified_diff_only_text() {
        let a = arc(&[("a.css", b"p{}\n"), ("b.bin", &[1])]);
        let b = arc(&[("a.css", b"p{color:red}\n"), ("b.bin", &[2])]);
        let u = archive_unified_diff(&a, &b, CompareOptions::default());
        assert!(u.contains("--- a/a.css"));
        assert!(u.contains("+p{color:red}"));
        assert!(!u.contains("b.bin"));
        assert!(file_unified_diff(&a, &b, "b.bin", CompareOptions::default()).is_none());
        assert!(file_unified_diff(&a, &b, "nope.css", CompareOptions::default()).is_none());
    }

    const OPF_A: &str = r#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
<dc:identifier id="id">urn:uuid:1</dc:identifier><dc:title>T</dc:title><dc:language>en</dc:language>
<dc:creator>A</dc:creator><dc:creator>B</dc:creator><dc:subject>x</dc:subject><dc:subject>y</dc:subject></metadata>
<manifest><item id="a" href="a.xhtml" media-type="application/xhtml+xml"/><item id="b" href="b.xhtml" media-type="application/xhtml+xml"/><item id="n" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav scripted"/></manifest>
<spine><itemref idref="a"/><itemref idref="b"/></spine></package>"#;

    const OPF_B_REORDERED: &str = r#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
<dc:language>en</dc:language><dc:title>T</dc:title><dc:creator>B</dc:creator><dc:creator>A</dc:creator>
<dc:subject>y</dc:subject><dc:subject>x</dc:subject><dc:identifier id="uid">urn:uuid:1</dc:identifier></metadata>
<manifest><item id="zz" href="nav.xhtml" media-type="application/xhtml+xml" properties="scripted nav"/><item id="q2" href="b.xhtml" media-type="application/xhtml+xml"/><item id="q1" href="a.xhtml" media-type="application/xhtml+xml"/></manifest>
<spine><itemref idref="q1"/><itemref idref="q2"/></spine></package>"#;

    #[test]
    fn opf_ignores_order_and_ids() {
        let d = compare_opf("c.opf", OPF_A, "c.opf", OPF_B_REORDERED).unwrap();
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn opf_detects_real_differences() {
        let b = OPF_B_REORDERED
            .replace("<dc:title>T</dc:title>", "<dc:title>U</dc:title>")
            .replace(
                "<item id=\"q2\" href=\"b.xhtml\"",
                "<item id=\"q2\" href=\"c.xhtml\"",
            )
            .replace("version=\"3.0\"", "version=\"2.0\"");
        let d = compare_opf("c.opf", OPF_A, "other/c.opf", &b).unwrap();
        assert_eq!(d.metadata.len(), 2, "title and the derived title_sort");
        assert_eq!(d.metadata[0].field, "title");
        assert_eq!(d.metadata[0].new, "U");
        assert_eq!(d.version_changed, Some(("3.0".into(), "2.0".into())));
        // other/c.opf changes the absolute paths of every manifest item
        assert_eq!(d.manifest_added.len(), 3);
        assert_eq!(d.manifest_removed.len(), 3);
        assert_ne!(d.spine_old, d.spine_new);
        let same_dir = compare_opf("c.opf", OPF_A, "c.opf", &b).unwrap();
        assert_eq!(same_dir.manifest_added, vec!["c.xhtml"]);
        assert_eq!(same_dir.manifest_removed, vec!["b.xhtml"]);
    }

    #[test]
    fn opf_manifest_property_change() {
        let b = OPF_A.replace("properties=\"nav scripted\"", "properties=\"nav\"");
        let d = compare_opf("c.opf", OPF_A, "c.opf", &b).unwrap();
        assert_eq!(d.manifest_changed.len(), 1);
        assert_eq!(d.manifest_changed[0].field, "nav.xhtml");
        assert!(d.manifest_changed[0].old.contains("nav scripted"));
    }

    #[test]
    fn opf_parse_error_reported() {
        assert!(compare_opf("a.opf", "<package", "b.opf", OPF_A).is_err());
    }

    #[test]
    fn changed_lines_mark_added_modified_and_removed() {
        use LineChangeKind::*;
        let kinds = |a: &str, b: &str| {
            changed_lines(a, b)
                .into_iter()
                .map(|c| (c.line, c.kind))
                .collect::<Vec<_>>()
        };
        assert!(kinds("a\nb\n", "a\nb\n").is_empty());
        assert_eq!(kinds("a\nb\nc\n", "a\nB\nc\n"), [(1, Modified)]);
        assert_eq!(kinds("a\nc\n", "a\nb\nc\n"), [(1, Added)]);
        assert_eq!(
            kinds("a\nb\nc\n", "a\nc\n"),
            [(1, Removed)],
            "the line after the deletion carries the mark"
        );
        assert_eq!(
            kinds("a\nb\nc\n", "a\nb\n"),
            [(1, Removed)],
            "a deletion at the end marks the last line"
        );
        assert_eq!(
            kinds("a\nb\n", "a\nX\nY\nZ\n"),
            [(1, Modified), (2, Added), (3, Added)]
        );
        assert_eq!(kinds("", "a\nb\n"), [(0, Added), (1, Added)]);
        assert_eq!(kinds("a\n", ""), [(0, Removed)]);
    }

    fn lines(n: usize) -> String {
        (1..=n).map(|i| format!("line {i}\n")).collect()
    }

    #[test]
    fn side_by_side_folds_equal_lines_into_skips() {
        let old = lines(30);
        let new = old.replace("line 15\n", "line 15 changed\n");
        let d = side_by_side(&old, &new, 2);
        assert_eq!(d.changed_rows, 1);
        // 2 + 1 + 2 rows shown, the rest folded
        let shown = d
            .rows
            .iter()
            .filter(|r| matches!(r, Row::Line { .. }))
            .count();
        assert_eq!(shown, 5);
        assert!(matches!(d.rows.first(), Some(Row::Skip(12))));
        assert!(matches!(d.rows.last(), Some(Row::Skip(13))));
        let full = side_by_side(&old, &new, usize::MAX);
        assert_eq!(full.rows.len(), 30);
        assert!(side_by_side(&old, &old, 3).is_identical());
        assert!(side_by_side(&old, &old, 3).rows.is_empty());
    }

    #[test]
    fn side_by_side_pairs_changed_lines_and_marks_the_words() {
        let d = side_by_side("a\nthe quick fox\nz\n", "a\nthe slow fox\nz\n", 1);
        let Row::Line { kind, old, new } = &d.rows[1] else {
            panic!("{d:?}")
        };
        assert_eq!(*kind, RowKind::Changed);
        let (o, n) = (old.as_ref().unwrap(), new.as_ref().unwrap());
        assert_eq!((o.line, n.line), (2, 2));
        assert_eq!(&o.text[o.marks[0].clone()], "quick");
        assert_eq!(&n.text[n.marks[0].clone()], "slow");
    }

    #[test]
    fn side_by_side_adds_and_removes_with_line_numbers_of_each_side() {
        let d = side_by_side("a\nb\nc\n", "a\nc\nd\ne\n", usize::MAX);
        let kinds: Vec<RowKind> = d
            .rows
            .iter()
            .map(|r| match r {
                Row::Line { kind, .. } => *kind,
                Row::Skip(_) => unreachable!(),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                RowKind::Equal,
                RowKind::Removed,
                RowKind::Equal,
                RowKind::Added,
                RowKind::Added
            ]
        );
        assert_eq!(d.change_starts(), [1, 3]);
        // the added "d" is line 3 of the new text, nothing on the old side
        let Row::Line { old, new, .. } = &d.rows[3] else {
            panic!()
        };
        assert!(old.is_none());
        assert_eq!(new.as_ref().unwrap().line, 3);
    }

    #[test]
    fn side_by_side_search_and_line_lookup() {
        let d = side_by_side(&lines(10), &lines(10).replace("line 7", "Seven"), 1);
        assert_eq!(d.search("seven", false).len(), 1);
        assert!(d.search("seven", true).is_empty());
        assert!(d.search("", false).is_empty());
        assert_eq!(d.search("line 7", true).len(), 1); // only the old side has it
        let r = d.row_of_line(7, false).unwrap();
        assert!(matches!(&d.rows[r], Row::Line { new: Some(c), .. } if c.line == 7));
    }

    #[test]
    fn beautify_hides_formatting_only_changes() {
        let mut a = Archive::default();
        let mut b = Archive::default();
        a.set_string("x.xhtml", "<html><body><p>hi</p></body></html>");
        b.set_string(
            "x.xhtml",
            "<html>\n  <body>\n    <p>hi</p>\n  </body>\n</html>",
        );
        let plain = file_side_by_side(&a, &b, "x.xhtml", false, 3).unwrap();
        assert!(!plain.is_identical());
        let pretty = file_side_by_side(&a, &b, "x.xhtml", true, 3).unwrap();
        assert!(pretty.is_identical());
        assert!(file_side_by_side(&a, &b, "pic.png", false, 3).is_none());
    }

    #[test]
    fn a_big_file_diffs_fast() {
        let old = lines(20_000);
        let new = old
            .replace("line 10000\n", "x\n")
            .replace("line 5\n", "y\n");
        let t = std::time::Instant::now();
        let d = side_by_side(&old, &new, 3);
        assert_eq!(d.changed_rows, 2);
        let budget = if cfg!(debug_assertions) { 6000 } else { 200 };
        assert!(t.elapsed().as_millis() < budget);
    }
}
