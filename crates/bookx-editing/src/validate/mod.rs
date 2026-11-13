//! An EPUB 2 and EPUB 3 validator (E8), written from the specifications.
//!
//! [`validate`] checks an [`Archive`]; [`validate_bytes`] also checks the zip container itself (entry order,
//! compression, duplicates, directory entries) which an `Archive` no longer knows about. The result is a
//! [`Report`] of [`Message`]s with stable rule ids and a severity, renderable with [`render_text`] and
//! [`render_json`].
//!
//! # Rule ids
//!
//! Ids are modelled on the public epubveri message catalogue (`OPF-xxx`, `PKG-xxx`, `RSC-xxx`, `HTM-xxx`,
//! `CSS-xxx`, `NAV-xxx`, `NCX-xxx`, `ACC-xxx`) and written from knowledge of what the checks mean, not copied.
//! Where epubveri reports a generic schema error (`RSC-005`) for a problem, this validator gives each distinct
//! problem its own id in the 900 range of the family (for example `OPF-901`, missing `dc:identifier`) so that
//! every rule can be tested alone. The full list is [`RULES`].
//!
//! # What is checked
//!
//! * zip level: `mimetype` first, stored, without extra field and with the exact content, duplicate and
//!   case-colliding names, forbidden characters, trailing periods, spaces, non-ASCII names, very long paths,
//!   empty directory entries
//! * container: `META-INF/container.xml` rootfile, encryption algorithms
//! * package document: version, required metadata, unique identifier, `dcterms:modified`, manifest and spine
//!   consistency, media types versus content, fallbacks, property vocabularies and the properties content
//!   needs (`scripted`, `svg`, `mathml`, `switch`, `remote-resources`), cover references, guide
//! * XHTML content documents: well-formedness, root, head and title, duplicate ids, id references, `epub:type`
//!   vocabulary, elements forbidden in EPUB 2, `img` alt text, links to missing resources and fragments,
//!   remote resources
//! * navigation: the EPUB 3 nav document (`toc` nav, list structure, order) and the EPUB 2 NCX
//! * CSS: syntax, `@charset`, unknown at-rules, forbidden properties, `@import`/`url()`/`@font-face`
//!   references

mod content;
mod css;
pub mod entities;
mod extra;
mod package;
mod zipcheck;

pub use extra::{
    declared_encoding, is_utf8_label, jpeg_components, link_bad_char, scan_entities, EntityHit,
    Options, EXTRA_RULES,
};

use std::collections::{BTreeMap, HashMap, HashSet};

use bookx_core::archive::Archive;
use serde::{Deserialize, Serialize};

/// How serious a message is. Only `Error` makes a report fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Severity {
    Error,
    Warning,
    Info,
    Usage,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "ERROR",
            Severity::Warning => "WARNING",
            Severity::Info => "INFO",
            Severity::Usage => "USAGE",
        }
    }
}

/// A rule of the validator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Rule {
    pub id: &'static str,
    pub severity: Severity,
    pub title: &'static str,
}

const fn r(id: &'static str, severity: Severity, title: &'static str) -> Rule {
    Rule {
        id,
        severity,
        title,
    }
}

use Severity::{Error as E, Info as I, Usage as U, Warning as W};

/// Every rule the validator can report.
pub const RULES: &[Rule] = &[
    r("PKG-004", E, "the file is not a readable zip archive"),
    r("PKG-005", E, "the mimetype entry has an extra field"),
    r(
        "PKG-006",
        E,
        "the mimetype entry is missing or not the first entry",
    ),
    r(
        "PKG-007",
        E,
        "the mimetype entry must contain exactly application/epub+zip",
    ),
    r("PKG-022", E, "the mimetype entry must not be compressed"),
    r(
        "PKG-009",
        E,
        "file name contains characters that are not allowed",
    ),
    r("PKG-010", W, "file name contains spaces"),
    r("PKG-011", E, "file name ends with a period"),
    r("PKG-012", U, "file name contains non-ASCII characters"),
    r("PKG-014", W, "the zip contains an empty directory entry"),
    r("PKG-024", E, "file names differ only by case"),
    r("PKG-028", W, "the path is longer than 255 bytes"),
    r("OPF-060", E, "duplicate entry in the zip archive"),
    r("RSC-002", E, "META-INF/container.xml is missing"),
    r("RSC-003", E, "container.xml names no package document"),
    r("RSC-004", E, "an encryption algorithm is not supported"),
    r("OPF-002", E, "the package document is missing"),
    r("RSC-016", E, "the XML document is not well-formed"),
    r(
        "OPF-001",
        E,
        "the package version is missing or unsupported, or the root is not a package",
    ),
    r("OPF-901", E, "the package has no dc:identifier"),
    r("OPF-902", E, "the package has no dc:title"),
    r("OPF-903", E, "the package has no dc:language"),
    r("OPF-055", W, "dc:title is empty"),
    r("OPF-092", E, "dc:language is not a valid language tag"),
    r(
        "OPF-048",
        E,
        "the package has no unique-identifier attribute",
    ),
    r(
        "OPF-030",
        E,
        "unique-identifier does not refer to a dc:identifier",
    ),
    r("OPF-904", E, "EPUB 3 packages need a dcterms:modified meta"),
    r(
        "OPF-905",
        E,
        "dcterms:modified must be CCYY-MM-DDThh:mm:ssZ",
    ),
    r("OPF-906", E, "duplicate id in the manifest"),
    r("OPF-907", E, "a manifest item lacks id, href or media-type"),
    r(
        "OPF-074",
        E,
        "one resource is declared twice in the manifest",
    ),
    r("OPF-091", E, "a manifest href must not have a fragment"),
    r(
        "OPF-099",
        E,
        "the manifest lists the package document itself",
    ),
    r(
        "RSC-001",
        E,
        "a manifest resource is missing from the container",
    ),
    r(
        "OPF-003",
        W,
        "a file in the container is not declared in the manifest",
    ),
    r(
        "OPF-029",
        E,
        "the file content does not match its declared media type",
    ),
    r(
        "OPF-035",
        W,
        "text/html is not appropriate for XHTML content",
    ),
    r("OPF-033", E, "the spine has no linear itemref"),
    r(
        "OPF-049",
        E,
        "a spine itemref does not refer to a manifest item",
    ),
    r("OPF-034", E, "a spine itemref appears twice"),
    r("OPF-917", E, "linear must be yes or no"),
    r("OPF-918", I, "non-linear spine item"),
    r("OPF-910", E, "an EPUB 2 spine needs a toc attribute"),
    r(
        "OPF-050",
        E,
        "the spine toc attribute must refer to the NCX",
    ),
    r("OPF-912", E, "an EPUB 3 manifest needs one nav item"),
    r(
        "OPF-913",
        E,
        "more than one manifest item has the nav property",
    ),
    r(
        "OPF-012",
        E,
        "a property is used on an item of the wrong media type",
    ),
    r(
        "OPF-915",
        E,
        "the cover meta does not refer to an image item",
    ),
    r("OPF-027", E, "undefined property"),
    r("OPF-026", E, "malformed property value"),
    r("OPF-028", E, "undeclared prefix"),
    r(
        "OPF-014",
        E,
        "the item uses a feature whose property is not declared",
    ),
    r(
        "OPF-015",
        E,
        "the scripted property is declared but the item has no script",
    ),
    r(
        "OPF-018",
        W,
        "remote-resources is declared but the item uses no remote resource",
    ),
    r(
        "OPF-040",
        E,
        "the fallback does not refer to a manifest item",
    ),
    r(
        "OPF-042",
        E,
        "the spine item has a media type that is not allowed and no fallback",
    ),
    r("OPF-045", E, "the fallback chain is circular"),
    r(
        "OPF-031",
        E,
        "a guide reference points to a resource that is not in the manifest",
    ),
    r("OPF-088", U, "unknown epub:type value"),
    r("HTM-901", E, "the root element is not xhtml:html"),
    r("HTM-902", E, "the document has no title"),
    r("HTM-903", E, "the document has no head"),
    r("HTM-905", E, "duplicate id"),
    r("HTM-906", E, "the element is not allowed in EPUB 2"),
    r("HTM-907", E, "img has no alt attribute"),
    r("HTM-909", E, "an id reference points to no element"),
    r("RSC-006", E, "remote scripts are not allowed"),
    r("RSC-007", E, "the referenced resource does not exist"),
    r(
        "RSC-008",
        E,
        "the referenced resource is not declared in the manifest",
    ),
    r(
        "RSC-012",
        E,
        "the fragment identifier does not exist in the target",
    ),
    r("RSC-026", E, "the URL points outside the container"),
    r("ACC-004", U, "link without text"),
    r("ACC-007", U, "the document declares no language"),
    r("NAV-001", E, "malformed navigation list"),
    r("NAV-901", E, "the nav document has no nav of type toc"),
    r(
        "NAV-011",
        W,
        "the table of contents is not in reading order",
    ),
    r(
        "NCX-001",
        E,
        "dtb:uid does not match the package identifier",
    ),
    r("NCX-002", E, "the NCX has no dtb:uid"),
    r("NCX-901", E, "the NCX has no navMap"),
    r("NCX-902", E, "a navPoint has no playOrder"),
    r(
        "NCX-903",
        E,
        "the same playOrder is used for different targets",
    ),
    r("NCX-904", E, "a navPoint has no label text or content src"),
    r(
        "CSS-001",
        E,
        "this property must not be used in an EPUB style sheet",
    ),
    r("CSS-003", E, "the style sheet is not UTF-8"),
    r("CSS-008", E, "CSS syntax error"),
    r("CSS-902", W, "unknown at-rule"),
];

/// The rule for `id`.
pub fn rule(id: &str) -> Option<&'static Rule> {
    RULES.iter().chain(EXTRA_RULES.iter()).find(|r| r.id == id)
}

/// One finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub severity: Severity,
    /// Path inside the container (empty for archive-wide findings).
    pub file: String,
    pub line: Option<u32>,
    pub col: Option<u32>,
    pub text: String,
}

/// Counts per severity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
    pub usages: usize,
}

/// The result of a validation run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub messages: Vec<Message>,
    pub summary: Summary,
    /// `true` when there is no `ERROR`.
    pub passed: bool,
    /// The detected package version, e.g. `2.0` or `3.0`, when the package document could be read.
    pub version: Option<String>,
}

impl Report {
    fn finish(mut self) -> Self {
        let mut s = Summary::default();
        for m in &self.messages {
            match m.severity {
                Severity::Error => s.errors += 1,
                Severity::Warning => s.warnings += 1,
                Severity::Info => s.infos += 1,
                Severity::Usage => s.usages += 1,
            }
        }
        self.summary = s;
        self.passed = s.errors == 0;
        self
    }

    /// The distinct rule ids that were reported, sorted.
    pub fn ids(&self) -> Vec<String> {
        let mut v: Vec<String> = self.messages.iter().map(|m| m.id.clone()).collect();
        v.sort();
        v.dedup();
        v
    }

    pub fn has(&self, id: &str) -> bool {
        self.messages.iter().any(|m| m.id == id)
    }

    /// Messages of one rule.
    pub fn of(&self, id: &str) -> Vec<&Message> {
        self.messages.iter().filter(|m| m.id == id).collect()
    }

    /// Distinct ids of messages with a given severity.
    pub fn ids_with(&self, severity: Severity) -> Vec<String> {
        let mut v: Vec<String> = self
            .messages
            .iter()
            .filter(|m| m.severity == severity)
            .map(|m| m.id.clone())
            .collect();
        v.sort();
        v.dedup();
        v
    }
}

/// `3 errors, 1 warning, 0 infos, 2 usages: FAILED` (or `PASSED`).
pub fn summary_line(report: &Report) -> String {
    fn n(count: usize, one: &str, many: &str) -> String {
        format!("{count} {}", if count == 1 { one } else { many })
    }
    let s = &report.summary;
    format!(
        "{}, {}, {}, {}: {}",
        n(s.errors, "error", "errors"),
        n(s.warnings, "warning", "warnings"),
        n(s.infos, "info", "infos"),
        n(s.usages, "usage", "usages"),
        if report.passed { "PASSED" } else { "FAILED" }
    )
}

/// One line per message (`ERROR(OPF-030) path:line:col: text`) and the [`summary_line`].
pub fn render_text(report: &Report) -> String {
    let mut out = String::new();
    for m in &report.messages {
        out.push_str(m.severity.label());
        out.push('(');
        out.push_str(&m.id);
        out.push(')');
        if !m.file.is_empty() {
            out.push(' ');
            out.push_str(&m.file);
            if let Some(l) = m.line {
                out.push_str(&format!(":{l}"));
                if let Some(c) = m.col {
                    out.push_str(&format!(":{c}"));
                }
            }
        }
        out.push_str(": ");
        out.push_str(&m.text);
        out.push('\n');
    }
    out.push_str(&summary_line(report));
    out.push('\n');
    out
}

/// The report as pretty-printed JSON.
pub fn render_json(report: &Report) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|_| "{}".into())
}

// ---------------------------------------------------------------------------
// entry points
// ---------------------------------------------------------------------------

/// Validates an open container. Zip-level problems that an [`Archive`] cannot show (entry order, compression,
/// duplicates, directory entries) are checked by [`validate_bytes`] instead.
pub fn validate(archive: &Archive) -> Report {
    validate_with(archive, &Options::default())
}

/// Like [`validate`], with the extra checks of [`Options`] switched on.
pub fn validate_with(archive: &Archive, opts: &Options) -> Report {
    let mut cx = Cx::new(archive);
    cx.opts = *opts;
    if opts.extended {
        cx.index = extra::NameIndex::new(archive.names());
    }
    zipcheck::check_names(&mut cx);
    zipcheck::check_mimetype_content(&mut cx);
    package::run(&mut cx);
    if opts.extended {
        extra::run(&mut cx);
    }
    cx.into_report()
}

/// Validates the bytes of an EPUB file: the zip structure first, then everything [`validate`] checks.
pub fn validate_bytes(data: &[u8]) -> Report {
    let (raw, readable) = zipcheck::scan_zip(data);
    if !readable {
        return Report {
            messages: raw,
            ..Default::default()
        }
        .finish();
    }
    let Ok(archive) = Archive::from_bytes(data.to_vec()) else {
        let mut cx = Cx::empty();
        cx.emit("PKG-004", "", None, "the zip archive cannot be read");
        return cx.into_report();
    };
    let mut report = validate(&archive);
    let mut all = raw;
    all.append(&mut report.messages);
    report.messages = all;
    report.finish()
}

// ---------------------------------------------------------------------------
// shared state
// ---------------------------------------------------------------------------

pub(crate) type Pos = Option<(u32, u32)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefKind {
    /// `a`, `area`: navigation links
    Link,
    /// `img`, `audio`, `video`, `object`, `iframe` ...
    Resource,
    /// style sheets and what they pull in
    Style,
    Script,
}

#[derive(Debug, Clone)]
pub(crate) struct Ref {
    pub from: String,
    pub pos: Pos,
    /// Resolved container path, empty for a fragment-only reference to the document itself.
    pub path: String,
    pub fragment: Option<String>,
    pub kind: RefKind,
}

/// One manifest item.
#[derive(Debug, Clone, Default)]
pub(crate) struct Item {
    pub id: String,
    pub href: String,
    pub path: String,
    pub media_type: String,
    pub properties: Vec<String>,
    pub fallback: Option<String>,
    pub pos: Pos,
}

impl Item {
    pub fn has_prop(&self, p: &str) -> bool {
        self.properties.iter().any(|x| x == p)
    }
}

pub(crate) struct Cx<'a> {
    pub archive: Option<&'a Archive>,
    pub messages: Vec<Message>,
    pub version: Option<String>,
    /// 2 or 3 once known
    pub major: u8,
    pub opf_path: String,
    pub items: Vec<Item>,
    pub manifest_paths: HashSet<String>,
    pub refs: Vec<Ref>,
    /// ids per parsed document path
    pub ids: HashMap<String, HashSet<String>>,
    /// package identifier text (the one `unique-identifier` points to)
    pub identifier: Option<String>,
    /// files whose content is encrypted (not readable)
    pub encrypted: HashSet<String>,
    /// per item path: remote resources used / script / svg ... found in content
    pub features: BTreeMap<String, Features>,
    pub spine_paths: Vec<String>,
    pub opts: Options,
    pub index: extra::NameIndex,
    /// every container path some reference points at
    pub referenced: HashSet<String>,
    /// the manifest id the EPUB 2 cover meta points to
    pub cover_meta: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct Features {
    pub scripted: bool,
    pub svg: bool,
    pub mathml: bool,
    pub switch: bool,
    pub remote: bool,
}

impl<'a> Cx<'a> {
    fn new(archive: &'a Archive) -> Self {
        let mut c = Self::empty();
        c.archive = Some(archive);
        c
    }

    fn empty() -> Self {
        Cx {
            archive: None,
            messages: Vec::new(),
            version: None,
            major: 3,
            opf_path: String::new(),
            items: Vec::new(),
            manifest_paths: HashSet::new(),
            refs: Vec::new(),
            ids: HashMap::new(),
            identifier: None,
            encrypted: HashSet::new(),
            features: BTreeMap::new(),
            spine_paths: Vec::new(),
            opts: Options::default(),
            index: extra::NameIndex::default(),
            referenced: HashSet::new(),
            cover_meta: None,
        }
    }

    pub fn emit(&mut self, id: &'static str, file: &str, pos: Pos, text: impl Into<String>) {
        let severity = rule(id).map(|r| r.severity).unwrap_or(Severity::Error);
        self.messages.push(Message {
            id: id.to_string(),
            severity,
            file: file.to_string(),
            line: pos.map(|p| p.0),
            col: pos.map(|p| p.1),
            text: text.into(),
        });
    }

    pub fn emit_as(
        &mut self,
        id: &'static str,
        severity: Severity,
        file: &str,
        pos: Pos,
        text: impl Into<String>,
    ) {
        self.emit(id, file, pos, text);
        if let Some(m) = self.messages.last_mut() {
            m.severity = severity;
        }
    }

    pub fn get(&self, path: &str) -> Option<&'a [u8]> {
        self.archive.and_then(|a| a.get(path))
    }

    pub fn text(&self, path: &str) -> Option<String> {
        self.archive.and_then(|a| a.get_string(path))
    }

    pub fn contains(&self, path: &str) -> bool {
        self.archive.is_some_and(|a| a.contains(path))
    }

    fn into_report(self) -> Report {
        Report {
            messages: self.messages,
            version: self.version,
            ..Default::default()
        }
        .finish()
    }
}

/// How a reference resolves.
pub(crate) enum Target {
    /// path in the container and fragment
    Local(String, Option<String>),
    Remote,
    /// mailto:, data:, urn:, javascript: ...
    Ignored,
    /// `..` climbs out of the container
    Leaks,
}

/// Resolves `href` found in the document `from`.
pub(crate) fn resolve(from: &str, href: &str) -> Target {
    let href = href.trim();
    let bytes = href.as_bytes();
    let scheme_len = bytes
        .iter()
        .position(|b| !(b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.')))
        .unwrap_or(bytes.len());
    if scheme_len > 0
        && bytes.get(scheme_len) == Some(&b':')
        && bytes[0].is_ascii_alphabetic()
        && scheme_len > 1
    {
        let scheme = href[..scheme_len].to_ascii_lowercase();
        return if scheme == "http" || scheme == "https" {
            Target::Remote
        } else {
            Target::Ignored
        };
    }
    if href.starts_with("//") {
        return Target::Remote;
    }
    let (path_part, frag) = match href.split_once('#') {
        Some((p, f)) => (p, Some(bookx_core::util::percent_decode(f))),
        None => (href, None),
    };
    let path_part = path_part.split('?').next().unwrap_or("");
    let frag = frag.filter(|f| !f.is_empty());
    if path_part.is_empty() {
        return Target::Local(String::new(), frag);
    }
    let decoded = bookx_core::util::percent_decode(path_part);
    if decoded.starts_with('/') {
        return Target::Leaks;
    }
    let dir = bookx_core::util::parent_dir(from);
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in decoded.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Target::Leaks;
                }
            }
            s => parts.push(s),
        }
    }
    Target::Local(parts.join("/"), frag)
}

/// Adds a reference found in `from` to the list resolved at the end. Remote targets are returned as `true`.
pub(crate) fn add_ref(cx: &mut Cx, from: &str, pos: Pos, href: &str, kind: RefKind) -> bool {
    match resolve(from, href) {
        Target::Local(path, fragment) => {
            let path = if path.is_empty() {
                from.to_string()
            } else {
                path
            };
            cx.refs.push(Ref {
                from: from.to_string(),
                pos,
                path,
                fragment,
                kind,
            });
            false
        }
        Target::Remote => true,
        Target::Ignored => false,
        Target::Leaks => {
            cx.emit(
                "RSC-026",
                from,
                pos,
                format!("the URL \"{href}\" points outside the container"),
            );
            false
        }
    }
}

/// Checks every collected reference against the container, the manifest and the ids of the target.
pub(crate) fn resolve_refs(cx: &mut Cx) {
    let refs = std::mem::take(&mut cx.refs);
    for r in &refs {
        cx.referenced.insert(r.path.clone());
        if !cx.contains(&r.path) {
            cx.emit(
                "RSC-007",
                &r.from,
                r.pos,
                format!(
                    "the referenced resource \"{}\" ({:?} reference) could not be found",
                    r.path, r.kind
                ),
            );
            continue;
        }
        if !cx.manifest_paths.contains(&r.path) && r.path != cx.opf_path {
            cx.emit(
                "RSC-008",
                &r.from,
                r.pos,
                format!(
                    "the referenced resource \"{}\" is not declared in the manifest",
                    r.path
                ),
            );
            continue;
        }
        if let Some(frag) = &r.fragment {
            if frag.starts_with("epubcfi(") || frag.contains('(') {
                continue;
            }
            if let Some(ids) = cx.ids.get(&r.path) {
                if !ids.contains(frag) {
                    cx.emit(
                        "RSC-012",
                        &r.from,
                        r.pos,
                        format!("the fragment \"#{frag}\" does not exist in \"{}\"", r.path),
                    );
                }
            }
        }
    }
}

/// Position of a byte offset in `src` (1-based line and column in characters).
pub(crate) fn offset_pos(src: &str, offset: usize) -> Pos {
    let offset = offset.min(src.len());
    let mut end = offset;
    while end > 0 && !src.is_char_boundary(end) {
        end -= 1;
    }
    let before = &src[..end];
    let line = before.matches('\n').count() as u32 + 1;
    let col = match before.rfind('\n') {
        Some(i) => before[i + 1..].chars().count() as u32 + 1,
        None => before.chars().count() as u32 + 1,
    };
    Some((line, col))
}

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) mod tests_extra;
