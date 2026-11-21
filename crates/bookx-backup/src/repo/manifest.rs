//! What a snapshot says (the manifest), the per-book notes state, and the checks every manifest
//! gets before a single path in it is used. A repository is untrusted input: a damaged or hostile
//! one must produce an error, never a write outside the target folder.

use anyhow::{bail, Result};
use bookx_core::library::{Bookmark, Highlight, ReadPos};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::seal::valid_id;

pub const FORMAT: u32 = 1;
/// Largest manifest accepted (plain or sealed).
pub const MAX_MANIFEST_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_FILES: usize = 1_000_000;
/// Longest relative path, in bytes.
pub const MAX_PATH_BYTES: usize = 1024;
pub const MAX_COMPONENT_BYTES: usize = 255;
pub const MAX_DEPTH: usize = 32;
/// A single file bigger than this (1 PiB) is nonsense.
pub const MAX_FILE_BYTES: u64 = 1 << 50;
/// All files together.
pub const MAX_TOTAL_BYTES: u64 = 1 << 52;
pub const MAX_STATE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_CATALOGUE_BYTES: u64 = 64 * 1024 * 1024 * 1024;
pub const MAX_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobRef {
    pub blob: String,
    pub size: u64,
}

/// What a file is, for the "include covers" switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    Book,
    Cover,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Path inside the library, `/` separated.
    pub path: String,
    pub blob: String,
    pub size: u64,
    /// Modification time, seconds since the epoch.
    pub mtime: i64,
    #[serde(default)]
    pub role: Role,
}

/// One snapshot. Stored as JSON (plain places) or sealed JSON (encrypted places).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    /// The repository this belongs to; a manifest copied in from another repository is refused.
    pub repo: String,
    /// 1, 2, 3… in the order snapshots were made; also in the file name.
    pub seq: u64,
    pub created: DateTime<Utc>,
    /// `auto` or `manual`.
    pub kind: String,
    pub app: String,
    pub books: usize,
    pub catalogue: BlobRef,
    /// The notes state ([`LibraryState`]) as of this snapshot.
    pub state: BlobRef,
    /// Journal records up to this number are contained in the catalogue of this snapshot.
    pub journal_seq: u64,
    pub include_books: bool,
    pub include_covers: bool,
    /// Files left out because they were too big for the place's size limit.
    #[serde(default)]
    pub too_big: Vec<String>,
    pub files: Vec<FileEntry>,
}

impl Manifest {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// The part of a manifest that says whether anything changed since the previous snapshot.
    pub fn same_content(&self, other: &Manifest) -> bool {
        self.catalogue.blob == other.catalogue.blob
            && self.state.blob == other.state.blob
            && self.include_books == other.include_books
            && self.include_covers == other.include_covers
            && self.files.len() == other.files.len()
            && self
                .files
                .iter()
                .zip(&other.files)
                .all(|(a, b)| a.path == b.path && a.blob == b.blob && a.size == b.size)
    }

    /// Everything a manifest must satisfy before use. `name` is the snapshot's file name, whose
    /// number and time have to agree with the content (an old manifest renamed to look new is
    /// refused here even in a place without encryption).
    pub fn validate(&self, repo: &str, name: &SnapName) -> Result<()> {
        if self.format != FORMAT {
            bail!(
                "this snapshot has format {}, which this Bookx cannot read",
                self.format
            );
        }
        if self.repo != repo {
            bail!("this snapshot belongs to a different backup repository");
        }
        if self.seq != name.seq || self.created.timestamp() != name.at.timestamp() {
            bail!("the snapshot does not match its name (an old snapshot under a new name?)");
        }
        if self.files.len() > MAX_FILES {
            bail!(
                "the snapshot lists {} files, more than the limit",
                self.files.len()
            );
        }
        for b in [&self.catalogue, &self.state] {
            if !valid_id(&b.blob) || b.size > MAX_CATALOGUE_BYTES {
                bail!("the snapshot names a bad object");
            }
        }
        let mut total: u64 = 0;
        let mut seen = std::collections::HashSet::with_capacity(self.files.len());
        for f in &self.files {
            validate_rel(&f.path)?;
            if !valid_id(&f.blob) {
                bail!("the snapshot names a bad object for {:?}", f.path);
            }
            if f.size > MAX_FILE_BYTES {
                bail!("the snapshot gives an impossible size for {:?}", f.path);
            }
            total = total.saturating_add(f.size);
            if total > MAX_TOTAL_BYTES {
                bail!("the snapshot is bigger than the limit");
            }
            if !seen.insert(f.path.as_str()) {
                bail!("the snapshot lists {:?} twice", f.path);
            }
        }
        Ok(())
    }
}

/// A snapshot file name: `s00000012-20261121T101500Z-manual.snap`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapName {
    pub seq: u64,
    pub at: DateTime<Utc>,
    pub manual: bool,
}

impl SnapName {
    pub fn file_name(&self) -> String {
        format!(
            "s{:08}-{}-{}.snap",
            self.seq,
            self.at.format("%Y%m%dT%H%M%SZ"),
            if self.manual { "manual" } else { "auto" }
        )
    }

    pub fn path(&self) -> String {
        format!("snapshots/{}", self.file_name())
    }

    pub fn parse(name: &str) -> Option<SnapName> {
        let stem = name.strip_suffix(".snap")?;
        let rest = stem.strip_prefix('s')?;
        let mut it = rest.splitn(3, '-');
        let seq_text = it.next()?;
        if seq_text.len() != 8 || !seq_text.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let seq = seq_text.parse().ok()?;
        let at = chrono::NaiveDateTime::parse_from_str(it.next()?, "%Y%m%dT%H%M%SZ").ok()?;
        let manual = match it.next()? {
            "manual" => true,
            "auto" => false,
            _ => return None,
        };
        Some(SnapName {
            seq,
            at: DateTime::from_naive_utc_and_offset(at, Utc),
            manual,
        })
    }
}

/// A journal record file name: `j0000000042-20261121T101500Z.jrn`.
pub fn journal_name(seq: u64, at: DateTime<Utc>) -> String {
    format!("j{seq:010}-{}.jrn", at.format("%Y%m%dT%H%M%SZ"))
}

pub fn parse_journal_name(name: &str) -> Option<(u64, DateTime<Utc>)> {
    let stem = name.strip_suffix(".jrn")?.strip_prefix('j')?;
    let (seq, at) = stem.split_once('-')?;
    if seq.len() != 10 || !seq.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let at = chrono::NaiveDateTime::parse_from_str(at, "%Y%m%dT%H%M%SZ").ok()?;
    Some((
        seq.parse().ok()?,
        DateTime::from_naive_utc_and_offset(at, Utc),
    ))
}

const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A path from a manifest, checked: relative, `/` separated, no `.`/`..`, no empty parts, no hidden
/// parts (so never `.bookx`, `.bookx-backups`, `.git`, `.ssh`…), no control characters, no
/// backslash or drive letter, no reserved Windows device names, and bounded in length and depth. The catalogue file name is reserved for the catalogue.
pub fn validate_rel(path: &str) -> Result<()> {
    let bad = |why: &str| -> Result<()> { bail!("unsafe path in the snapshot ({why}): {path:?}") };
    if path.is_empty() || path.len() > MAX_PATH_BYTES {
        return bad("length");
    }
    let b = path.as_bytes();
    if path.starts_with('/')
        || path.contains('\\')
        || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
    {
        return bad("absolute or drive path");
    }
    if path.chars().any(|c| c.is_control()) {
        return bad("control character");
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() > MAX_DEPTH {
        return bad("too deep");
    }
    for p in &parts {
        if p.is_empty() || *p == "." || *p == ".." {
            return bad("empty or dot component");
        }
        if p.len() > MAX_COMPONENT_BYTES {
            return bad("component too long");
        }
        if p.starts_with('.') {
            return bad("hidden component");
        }
        let stem = p.split('.').next().unwrap_or("").to_ascii_uppercase();
        if RESERVED.contains(&stem.as_str()) {
            return bad("reserved name");
        }
    }
    let last = parts.last().unwrap().to_ascii_lowercase();
    if parts.len() == 1
        && (last.starts_with("bookx-library.db") || last.starts_with("bookx-library.json"))
    {
        return bad("the catalogue is restored separately");
    }
    Ok(())
}

// -- the notes state ---------------------------------------------------------------------------------

/// Everything Bookx manages for one book that is not the book file: what the journal records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BookNotes {
    pub id: u64,
    /// The book's file in the library, to find it again after an id change.
    pub file: String,
    pub title: String,
    #[serde(default)]
    pub highlights: Vec<Highlight>,
    #[serde(default)]
    pub bookmarks: Vec<Bookmark>,
    #[serde(default)]
    pub position: Option<ReadPos>,
    #[serde(default)]
    pub progress: f64,
    #[serde(default)]
    pub last_read: Option<DateTime<Utc>>,
    /// `new`, `reading`, `on_hold`, `finished`.
    #[serde(default)]
    pub status: Option<String>,
}

impl BookNotes {
    /// Nothing worth keeping: no notes, marks, position, progress or status.
    pub fn is_empty(&self) -> bool {
        self.highlights.is_empty()
            && self.bookmarks.is_empty()
            && self.position.is_none()
            && self.progress == 0.0
            && self.last_read.is_none()
            && self.status.as_deref().is_none_or(|s| s == "new")
    }

    pub fn fingerprint(&self) -> String {
        use sha2::{Digest, Sha256};
        super::seal::hex(&Sha256::digest(
            serde_json::to_vec(self).unwrap_or_default(),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectionState {
    pub name: String,
    pub books: Vec<u64>,
}

/// The notes of the whole library at the time of a snapshot (a readable JSON for people who look).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryState {
    pub format: u32,
    pub books: Vec<BookNotes>,
    pub collections: Vec<CollectionState>,
}

/// One journal record: the notes of the books that changed since the previous record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalRecord {
    pub format: u32,
    pub repo: String,
    pub seq: u64,
    pub at: DateTime<Utc>,
    pub books: Vec<BookNotes>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn hostile_paths_are_refused() {
        for ok in [
            "a.epub",
            "Author/Title (3)/book.epub",
            "Ünïcode/ñ.epub",
            "Smith, J./Title: Sub/b.epub",
            "A/B/cover.jpg",
        ] {
            validate_rel(ok).unwrap_or_else(|e| panic!("{ok}: {e}"));
        }
        let long = "a".repeat(300);
        let deep = vec!["d"; 40].join("/");
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "a/../../x",
            "a/./b",
            "a//b",
            "a/",
            "C:/x",
            "C:\\x",
            "a\\b",
            ".bookx/plugins/evil.wasm",
            ".bookx-backups/x.db",
            "a/.ssh/id",
            "con",
            "a/NUL.txt",
            "lpt1.epub",
            "bad\nname",
            "bad\0name",
            "bookx-library.db",
            "bookx-library.db-wal",
            long.as_str(),
            deep.as_str(),
        ] {
            assert!(validate_rel(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn names_round_trip() {
        let n = SnapName {
            seq: 12,
            at: Utc.with_ymd_and_hms(2026, 11, 21, 10, 15, 0).unwrap(),
            manual: true,
        };
        assert_eq!(n.file_name(), "s00000012-20261121T101500Z-manual.snap");
        assert_eq!(SnapName::parse(&n.file_name()), Some(n));
        assert!(SnapName::parse("s12-20261121T101500Z-manual.snap").is_none());
        assert!(SnapName::parse("../s00000012-20261121T101500Z-manual.snap").is_none());
        let at = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
        assert_eq!(parse_journal_name(&journal_name(7, at)), Some((7, at)));
    }

    fn manifest() -> Manifest {
        let id = "a".repeat(64);
        Manifest {
            format: FORMAT,
            repo: "r".into(),
            seq: 1,
            created: Utc.with_ymd_and_hms(2026, 11, 21, 10, 15, 0).unwrap(),
            kind: "auto".into(),
            app: "t".into(),
            books: 1,
            catalogue: BlobRef {
                blob: id.clone(),
                size: 1,
            },
            state: BlobRef {
                blob: id.clone(),
                size: 1,
            },
            journal_seq: 0,
            include_books: true,
            include_covers: true,
            too_big: vec![],
            files: vec![FileEntry {
                path: "a.epub".into(),
                blob: id,
                size: 3,
                mtime: 0,
                role: Role::Book,
            }],
        }
    }

    #[test]
    fn a_manifest_must_agree_with_its_name_and_repository() {
        let m = manifest();
        let n = SnapName {
            seq: 1,
            at: m.created,
            manual: false,
        };
        m.validate("r", &n).unwrap();
        assert!(m.validate("other", &n).is_err());
        assert!(
            m.validate(
                "r",
                &SnapName {
                    seq: 2,
                    ..n.clone()
                }
            )
            .is_err(),
            "renamed old snapshot"
        );
        let mut dup = m.clone();
        dup.files.push(dup.files[0].clone());
        assert!(dup.validate("r", &n).is_err());
        let mut bad = m.clone();
        bad.files[0].blob = "../../x".into();
        assert!(bad.validate("r", &n).is_err());
        let mut huge = m.clone();
        huge.files[0].size = u64::MAX;
        assert!(huge.validate("r", &n).is_err());
        let mut path = m;
        path.files[0].path = "../escape".into();
        assert!(path.validate("r", &n).is_err());
    }

    #[test]
    fn empty_notes_are_not_kept() {
        let mut b = BookNotes::default();
        assert!(b.is_empty());
        b.status = Some("new".into());
        assert!(b.is_empty());
        b.progress = 0.3;
        assert!(!b.is_empty());
    }
}
