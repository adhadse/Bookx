//! Fixity checksums: notice when a book file has silently changed on disk (roadmap PP-G-31, PP-C-2).
//!
//! [`verify`] hashes the files of a library and compares them with what was recorded the last time
//! (`<library>/.bookx/fixity.json`: SHA-256, size and modification time). A file whose content
//! differs while its size and time are the same has been damaged by the disk or a sync tool: that is
//! the only case reported as [`Verdict::Damaged`]. A file whose time or size moved was edited, so the
//! new checksum is recorded and the file counts as fine. Files that are not recorded yet are recorded.
//!
//! No GTK. Reading is streamed, so large books do not fill memory. `stop` lets the caller cancel.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::history::STATE_DIR;

/// What was recorded about one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub sha256: String,
    pub size: u64,
    /// Modification time, seconds since the epoch.
    pub mtime: i64,
    /// When it was checked last (seconds since the epoch).
    pub checked: i64,
}

/// All recorded files, by path relative to the library root.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub files: BTreeMap<String, Entry>,
}

pub fn manifest_path(root: &Path) -> PathBuf {
    root.join(STATE_DIR).join("fixity.json")
}

impl Manifest {
    /// Reads the manifest; missing or damaged means empty.
    pub fn load(root: &Path) -> Manifest {
        std::fs::read(manifest_path(root))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let p = manifest_path(root);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)
            .with_context(|| format!("cannot write {}", tmp.display()))?;
        std::fs::rename(&tmp, &p)?;
        Ok(())
    }
}

/// The result for one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Same checksum as recorded.
    Intact,
    /// Not recorded before; recorded now.
    Recorded,
    /// Size or time changed (an edit); the new checksum is recorded.
    Edited,
    /// Content changed although size and time did not: bit rot or a bad sync.
    Damaged,
    /// Recorded before, gone now.
    Missing,
    /// Could not be read.
    Unreadable(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub files: Vec<(String, Verdict)>,
    /// The run was cancelled before every file was looked at.
    pub cancelled: bool,
}

impl Report {
    fn count(&self, f: impl Fn(&Verdict) -> bool) -> usize {
        self.files.iter().filter(|(_, v)| f(v)).count()
    }

    pub fn intact(&self) -> usize {
        self.count(|v| matches!(v, Verdict::Intact))
    }
    pub fn recorded(&self) -> usize {
        self.count(|v| matches!(v, Verdict::Recorded))
    }
    pub fn edited(&self) -> usize {
        self.count(|v| matches!(v, Verdict::Edited))
    }
    pub fn damaged(&self) -> Vec<&str> {
        self.files
            .iter()
            .filter(|(_, v)| matches!(v, Verdict::Damaged))
            .map(|(p, _)| p.as_str())
            .collect()
    }
    pub fn missing(&self) -> Vec<&str> {
        self.files
            .iter()
            .filter(|(_, v)| matches!(v, Verdict::Missing))
            .map(|(p, _)| p.as_str())
            .collect()
    }
    pub fn unreadable(&self) -> usize {
        self.count(|v| matches!(v, Verdict::Unreadable(_)))
    }

    /// True when nothing is damaged, missing or unreadable.
    pub fn is_clean(&self) -> bool {
        self.damaged().is_empty() && self.missing().is_empty() && self.unreadable() == 0
    }

    /// One line for a toast or the command line.
    pub fn summary(&self) -> String {
        let total = self.files.len();
        let mut s = format!("{total} file(s) checked: {} intact", self.intact());
        for (n, what) in [
            (self.recorded(), "newly recorded"),
            (self.edited(), "edited since last time"),
            (self.damaged().len(), "DAMAGED"),
            (self.missing().len(), "missing"),
            (self.unreadable(), "unreadable"),
        ] {
            if n > 0 {
                s.push_str(&format!(", {n} {what}"));
            }
        }
        if self.cancelled {
            s.push_str(" (stopped early)");
        }
        s
    }
}

/// SHA-256 of a file, streamed.
pub fn hash_file(path: &Path, stop: &AtomicBool) -> Result<Option<String>> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(Some(
        h.finalize().iter().map(|b| format!("{b:02x}")).collect(),
    ))
}

fn mtime_of(md: &std::fs::Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Pure decision for one file whose current hash is known.
pub fn judge(old: Option<&Entry>, hash: &str, size: u64, mtime: i64) -> Verdict {
    match old {
        None => Verdict::Recorded,
        Some(e) if e.sha256 == hash => Verdict::Intact,
        Some(e) if e.size != size || e.mtime != mtime => Verdict::Edited,
        Some(_) => Verdict::Damaged,
    }
}

/// Checks `files` (relative to `root`) against the manifest and updates the manifest on disk
/// (damaged files keep their old checksum, so they stay reported until fixed or accepted).
/// `now` is seconds since the epoch.
pub fn verify(root: &Path, files: &[String], now: i64, stop: &AtomicBool) -> Result<Report> {
    let mut manifest = Manifest::load(root);
    let mut report = Report::default();
    for rel in files {
        if stop.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        let path = root.join(rel);
        let md = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if manifest.files.contains_key(rel) {
                    report.files.push((rel.clone(), Verdict::Missing));
                }
                continue;
            }
            Err(e) => {
                report
                    .files
                    .push((rel.clone(), Verdict::Unreadable(e.to_string())));
                continue;
            }
        };
        let hash = match hash_file(&path, stop) {
            Ok(Some(h)) => h,
            Ok(None) => {
                report.cancelled = true;
                break;
            }
            Err(e) => {
                report
                    .files
                    .push((rel.clone(), Verdict::Unreadable(e.to_string())));
                continue;
            }
        };
        let (size, mtime) = (md.len(), mtime_of(&md));
        let verdict = judge(manifest.files.get(rel), &hash, size, mtime);
        if verdict != Verdict::Damaged {
            manifest.files.insert(
                rel.clone(),
                Entry {
                    sha256: hash,
                    size,
                    mtime,
                    checked: now,
                },
            );
        }
        report.files.push((rel.clone(), verdict));
    }
    // forget files that were removed on purpose (not listed any more)
    let listed: std::collections::HashSet<&String> = files.iter().collect();
    if !report.cancelled {
        manifest.files.retain(|k, _| listed.contains(k));
    }
    manifest.save(root)?;
    Ok(report)
}

/// Accepts the current content of a damaged file as the new truth.
pub fn accept(root: &Path, rel: &str, now: i64) -> Result<()> {
    let mut m = Manifest::load(root);
    let path = root.join(rel);
    let md = std::fs::metadata(&path)?;
    let hash = hash_file(&path, &AtomicBool::new(false))?.unwrap_or_default();
    m.files.insert(
        rel.to_string(),
        Entry {
            sha256: hash,
            size: md.len(),
            mtime: mtime_of(&md),
            checked: now,
        },
    );
    m.save(root)
}

/// When the files were last verified (the newest `checked`), if ever.
pub fn last_checked(root: &Path) -> Option<i64> {
    Manifest::load(root).files.values().map(|e| e.checked).max()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);
    impl Dir {
        fn new() -> Dir {
            let p = std::env::temp_dir().join(format!("bookx-fixity-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&p).unwrap();
            Dir(p)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn no_stop() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn hash_matches_known_value() {
        let d = Dir::new();
        let p = d.0.join("a.txt");
        std::fs::write(&p, "abc").unwrap();
        assert_eq!(
            hash_file(&p, &no_stop()).unwrap().unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn judging_is_pure() {
        let e = Entry {
            sha256: "aa".into(),
            size: 3,
            mtime: 10,
            checked: 0,
        };
        assert_eq!(judge(None, "aa", 3, 10), Verdict::Recorded);
        assert_eq!(judge(Some(&e), "aa", 3, 10), Verdict::Intact);
        assert_eq!(judge(Some(&e), "bb", 3, 11), Verdict::Edited);
        assert_eq!(judge(Some(&e), "bb", 4, 10), Verdict::Edited);
        assert_eq!(judge(Some(&e), "bb", 3, 10), Verdict::Damaged);
    }

    #[test]
    fn first_run_records_then_second_is_intact() {
        let d = Dir::new();
        std::fs::write(d.0.join("a.epub"), "one").unwrap();
        std::fs::write(d.0.join("b.epub"), "two").unwrap();
        let files = vec!["a.epub".to_string(), "b.epub".to_string()];
        let r = verify(&d.0, &files, 100, &no_stop()).unwrap();
        assert_eq!(r.recorded(), 2);
        assert!(r.is_clean());
        let r = verify(&d.0, &files, 200, &no_stop()).unwrap();
        assert_eq!(r.intact(), 2);
        assert_eq!(last_checked(&d.0), Some(200));
        assert!(r.summary().starts_with("2 file(s) checked: 2 intact"));
    }

    #[test]
    fn silent_change_is_damage_but_an_edit_is_not() {
        let d = Dir::new();
        let p = d.0.join("a.epub");
        std::fs::write(&p, "one").unwrap();
        let files = vec!["a.epub".to_string()];
        verify(&d.0, &files, 1, &no_stop()).unwrap();

        // same size, same time, different bytes
        let before = std::fs::metadata(&p).unwrap().modified().unwrap();
        std::fs::write(&p, "ONE").unwrap();
        let f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
        f.set_modified(before).unwrap();
        drop(f);
        let r = verify(&d.0, &files, 2, &no_stop()).unwrap();
        assert_eq!(r.damaged(), vec!["a.epub"]);
        assert!(!r.is_clean());
        // still reported on the next run
        assert_eq!(
            verify(&d.0, &files, 3, &no_stop()).unwrap().damaged().len(),
            1
        );
        // accepting makes it the truth
        accept(&d.0, "a.epub", 4).unwrap();
        assert!(verify(&d.0, &files, 5, &no_stop()).unwrap().is_clean());

        // a real edit changes the size: fine, and re-recorded
        std::fs::write(&p, "a longer text").unwrap();
        let r = verify(&d.0, &files, 6, &no_stop()).unwrap();
        assert_eq!(r.edited(), 1);
        assert!(r.is_clean());
        assert_eq!(verify(&d.0, &files, 7, &no_stop()).unwrap().intact(), 1);
    }

    #[test]
    fn missing_files_are_reported_only_when_recorded() {
        let d = Dir::new();
        std::fs::write(d.0.join("a.epub"), "x").unwrap();
        let both = vec!["a.epub".to_string(), "never.epub".to_string()];
        let r = verify(&d.0, &both, 1, &no_stop()).unwrap();
        assert!(r.missing().is_empty(), "a file never seen is not 'missing'");
        std::fs::remove_file(d.0.join("a.epub")).unwrap();
        let r = verify(&d.0, &both, 2, &no_stop()).unwrap();
        assert_eq!(r.missing(), vec!["a.epub"]);
    }

    #[test]
    fn unlisted_files_leave_the_manifest() {
        let d = Dir::new();
        std::fs::write(d.0.join("a.epub"), "x").unwrap();
        std::fs::write(d.0.join("b.epub"), "y").unwrap();
        verify(&d.0, &["a.epub".into(), "b.epub".into()], 1, &no_stop()).unwrap();
        verify(&d.0, &["a.epub".into()], 2, &no_stop()).unwrap();
        let m = Manifest::load(&d.0);
        assert_eq!(m.files.keys().collect::<Vec<_>>(), vec!["a.epub"]);
    }

    #[test]
    fn cancelling_stops_early_and_keeps_the_manifest() {
        let d = Dir::new();
        std::fs::write(d.0.join("a.epub"), "x").unwrap();
        let stop = AtomicBool::new(true);
        let r = verify(&d.0, &["a.epub".into()], 1, &stop).unwrap();
        assert!(r.cancelled);
        assert!(r.files.is_empty());
        assert!(r.summary().ends_with("(stopped early)"));
    }

    #[test]
    fn a_damaged_manifest_is_ignored() {
        let d = Dir::new();
        std::fs::create_dir_all(d.0.join(STATE_DIR)).unwrap();
        std::fs::write(manifest_path(&d.0), "{not json").unwrap();
        assert_eq!(Manifest::load(&d.0), Manifest::default());
    }
}
