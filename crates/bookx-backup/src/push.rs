//! Writing a snapshot to a [`Target`], checking it, pruning old ones, and getting one back.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, anyhow, bail};
use bookx_store::health::{self, Snapshot};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use crate::crypto::{self, Kdf, Meta};
use crate::target::{RemoteFile, RemoteName, Target, parse_remote_name, remote_name};

/// The error of a backup the user cancelled.
#[derive(Debug)]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("cancelled")
    }
}
impl std::error::Error for Cancelled {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushStage {
    Reading,
    Encrypting,
    Uploading,
    Verifying,
    Pruning,
    Done,
}

impl PushStage {
    pub fn label(&self) -> &'static str {
        match self {
            PushStage::Reading => "Reading the snapshot",
            PushStage::Encrypting => "Encrypting",
            PushStage::Uploading => "Copying",
            PushStage::Verifying => "Checking the copy",
            PushStage::Pruning => "Removing old backups",
            PushStage::Done => "Done",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub stage: PushStage,
    /// 0.0 to 1.0 over the whole push.
    pub fraction: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushReport {
    pub name: String,
    pub bytes: u64,
    pub encrypted: bool,
    /// The file read back from the place has the hash of what was written.
    pub verified: bool,
    /// Old backups removed by the retention rule.
    pub pruned: usize,
}

fn sha(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// How a push is done.
#[derive(Debug, Clone, Copy)]
pub struct PushOptions<'a> {
    /// Encrypt with this passphrase; `None` writes the plain snapshot.
    pub passphrase: Option<&'a str>,
    /// How many automatic backups the place keeps.
    pub keep: usize,
    pub now: DateTime<Utc>,
    pub kdf: Kdf,
}

/// Copies `snapshot` to `target`.
///
/// 1. reads the snapshot file, encrypts it when `passphrase` is given;
/// 2. writes it under a name that says when/what it is;
/// 3. reads it back and compares the SHA-256 (verify-after-write): a copy that differs is deleted
///    and the push fails;
/// 4. applies the retention rule `keep` to the place (see [`health::retention`]: manual backups are
///    never removed, only files with Bookx's own names are ever touched).
///
/// `cancel` is looked at between the steps; a cancelled push removes the file it wrote.
pub fn push(
    target: &dyn Target,
    snapshot: &Snapshot,
    opts: &PushOptions,
    cancel: &AtomicBool,
    progress: &dyn Fn(Progress),
) -> Result<PushReport> {
    let PushOptions {
        passphrase,
        keep,
        now,
        kdf,
    } = *opts;
    let step = |stage, fraction| {
        progress(Progress { stage, fraction });
        if cancel.load(Ordering::Relaxed) {
            Err(anyhow::Error::new(Cancelled))
        } else {
            Ok(())
        }
    };
    step(PushStage::Reading, 0.02)?;
    let plain = std::fs::read(&snapshot.path)
        .with_context(|| format!("cannot read {}", snapshot.path.display()))?;
    let file_name = snapshot
        .path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .ok_or_else(|| anyhow!("the snapshot has no file name"))?;
    let encrypted = passphrase.is_some();
    let name = remote_name(&file_name, encrypted);
    let data = match passphrase {
        Some(p) => {
            step(PushStage::Encrypting, 0.1)?;
            crypto::encrypt_with(
                &plain,
                p,
                &Meta {
                    created: snapshot.at.to_rfc3339(),
                    books: snapshot.books,
                    kind: format!("{:?}", snapshot.kind).to_lowercase(),
                    original_name: file_name.clone(),
                },
                kdf,
            )?
        }
        None => plain,
    };
    step(PushStage::Uploading, 0.3)?;
    target.check()?;
    target.put(&name, &data)?;
    let cleanup = |e: anyhow::Error| {
        let _ = target.delete(&name);
        e
    };
    step(PushStage::Verifying, 0.7).map_err(cleanup)?;
    let back = target
        .get(&name)
        .map_err(|e| cleanup(anyhow!("could not read the copy back: {e:#}")))?;
    if sha(&back) != sha(&data) {
        let _ = target.delete(&name);
        bail!(
            "the copy does not match the original (it was removed); the place may be full or unreliable"
        );
    }
    step(PushStage::Pruning, 0.9)?;
    let pruned = prune(target, keep, now)?;
    progress(Progress {
        stage: PushStage::Done,
        fraction: 1.0,
    });
    Ok(PushReport {
        name,
        bytes: data.len() as u64,
        encrypted,
        verified: true,
        pruned,
    })
}

/// Applies the retention rule to the backups in `target`. Returns how many were deleted.
pub fn prune(target: &dyn Target, keep: usize, now: DateTime<Utc>) -> Result<usize> {
    let files = list_backups(target)?;
    let input: Vec<_> = files.iter().map(|(_, n)| (n.at, n.kind)).collect();
    let mut n = 0;
    for i in health::retention(&input, keep.max(1), now) {
        target.delete(&files[i].0.name)?;
        n += 1;
    }
    Ok(n)
}

/// The backups in a place, newest first.
pub fn list_backups(target: &dyn Target) -> Result<Vec<(RemoteFile, RemoteName)>> {
    let mut v: Vec<_> = target
        .list()?
        .into_iter()
        .filter_map(|f| {
            let n = parse_remote_name(&f.name)?;
            Some((f, n))
        })
        .collect();
    v.sort_by(|a, b| b.1.at.cmp(&a.1.at).then_with(|| b.0.name.cmp(&a.0.name)));
    Ok(v)
}

/// What a place holds, for the "where do backups go" display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub count: usize,
    pub bytes: u64,
    pub newest: Option<DateTime<Utc>>,
    pub free: Option<u64>,
}

pub fn summarize(target: &dyn Target) -> Result<Summary> {
    let l = list_backups(target)?;
    Ok(Summary {
        count: l.len(),
        bytes: l.iter().map(|(f, _)| f.size).sum(),
        newest: l.first().map(|(_, n)| n.at),
        free: target.free_space(),
    })
}

/// Downloads backup `name` from `target` into `out_dir` as a plain SQLite file, decrypting it
/// when needed, and returns its path. The file is checked to be a SQLite database; restoring it
/// (with the integrity check and the "before a restore" snapshot) is `health::restore_snapshot`.
pub fn fetch_to_file(
    target: &dyn Target,
    name: &str,
    passphrase: Option<&str>,
    out_dir: &Path,
) -> Result<PathBuf> {
    let data = target.get(name)?;
    let plain = open_bytes(&data, passphrase)?;
    write_plain(&plain, name, out_dir)
}

/// The plain SQLite bytes of a backup file's content: decrypts `.bookxbackup` (judged by the
/// content, not the name).
pub fn open_bytes(data: &[u8], passphrase: Option<&str>) -> Result<Vec<u8>> {
    let plain = if crypto::is_encrypted(data) {
        let p = passphrase.filter(|p| !p.is_empty()).ok_or_else(|| {
            anyhow!("this backup is encrypted: enter the backup passphrase to restore it")
        })?;
        crypto::decrypt(data, p)?
    } else {
        data.to_vec()
    };
    if !plain.starts_with(b"SQLite format 3\0") {
        bail!("that file is not a Bookx backup");
    }
    Ok(plain)
}

fn write_plain(plain: &[u8], name: &str, out_dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(out_dir)?;
    let stem = name
        .strip_suffix(".bookxbackup")
        .or_else(|| name.strip_suffix(".db"))
        .unwrap_or(name);
    let stem: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let path = out_dir.join(format!("restore-{stem}.db"));
    std::fs::write(&path, plain)?;
    Ok(path)
}

/// Reads a backup file from disk (a `.db` or a `.bookxbackup` the user picked) into a plain file in `out_dir`.
pub fn open_file(path: &Path, passphrase: Option<&str>, out_dir: &Path) -> Result<PathBuf> {
    let data = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let plain = open_bytes(&data, passphrase)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    write_plain(&plain, &name, out_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::MemTarget;
    use bookx_store::health::SnapshotKind;
    use chrono::TimeZone;

    fn snap(dir: &Path, at: DateTime<Utc>, kind: SnapshotKind) -> Snapshot {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(health::snapshot_file_name(at, 2, kind));
        let mut body = b"SQLite format 3\0".to_vec();
        body.extend(std::iter::repeat_n(7u8, 5000));
        std::fs::write(&path, &body).unwrap();
        Snapshot {
            path,
            at,
            books: 2,
            kind,
            size: body.len() as u64,
        }
    }

    fn tmp() -> PathBuf {
        std::env::temp_dir().join(format!("bookx-push-{}", uuid::Uuid::new_v4()))
    }

    fn at(d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, d, 10, 0, 0).unwrap()
    }

    fn run(
        t: &dyn Target,
        s: &Snapshot,
        pass: Option<&str>,
        keep: usize,
        now: DateTime<Utc>,
    ) -> Result<PushReport> {
        let opts = PushOptions {
            passphrase: pass,
            keep,
            now,
            kdf: Kdf::TEST,
        };
        push(t, s, &opts, &AtomicBool::new(false), &|_| {})
    }

    #[test]
    fn a_plain_push_is_verified_and_restorable() {
        let d = tmp();
        let t = MemTarget::default();
        let s = snap(&d, at(1), SnapshotKind::Manual);
        let r = run(&t, &s, None, 5, at(1)).unwrap();
        assert!(r.verified && !r.encrypted);
        assert_eq!(r.name, "bookx-20261101T100000Z-n2-manual.db");
        let out = fetch_to_file(&t, &r.name, None, &d.join("out")).unwrap();
        assert_eq!(std::fs::read(out).unwrap(), std::fs::read(&s.path).unwrap());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn an_encrypted_push_needs_the_passphrase_to_come_back() {
        let d = tmp();
        let t = MemTarget::default();
        let s = snap(&d, at(2), SnapshotKind::Auto);
        let r = run(&t, &s, Some("hunter22"), 5, at(2)).unwrap();
        assert!(r.encrypted && r.name.ends_with(".bookxbackup"));
        let stored = t.files.lock().unwrap()[&r.name].clone();
        assert!(
            !stored.windows(15).any(|w| w == b"SQLite format 3"),
            "ciphertext only"
        );
        let e = fetch_to_file(&t, &r.name, None, &d.join("o1")).unwrap_err();
        assert!(e.to_string().contains("passphrase"));
        let e = fetch_to_file(&t, &r.name, Some("nope"), &d.join("o2")).unwrap_err();
        assert!(e.to_string().contains("wrong passphrase"));
        let out = fetch_to_file(&t, &r.name, Some("hunter22"), &d.join("o3")).unwrap();
        assert_eq!(std::fs::read(out).unwrap(), std::fs::read(&s.path).unwrap());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_copy_that_reads_back_differently_is_removed_and_reported() {
        let d = tmp();
        let t = MemTarget::default();
        *t.corrupt.lock().unwrap() = true;
        let s = snap(&d, at(3), SnapshotKind::Manual);
        let e = run(&t, &s, None, 5, at(3)).unwrap_err().to_string();
        assert!(e.contains("does not match"), "{e}");
        assert!(
            t.files.lock().unwrap().is_empty(),
            "the bad copy is deleted"
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn an_unreachable_place_fails_with_its_reason_and_writes_nothing() {
        let d = tmp();
        let t = MemTarget::default();
        t.set_down(Some("the NAS is switched off"));
        let s = snap(&d, at(4), SnapshotKind::Auto);
        let e = run(&t, &s, None, 5, at(4)).unwrap_err().to_string();
        assert!(e.contains("NAS is switched off"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn retention_runs_per_place_and_never_deletes_manual_or_foreign_files() {
        let d = tmp();
        let t = MemTarget::default();
        t.files
            .lock()
            .unwrap()
            .insert("bookx-20260101T100000Z-n1-manual.db".into(), b"m".to_vec());
        for day in 1..=6 {
            let s = snap(&d, at(day), SnapshotKind::Auto);
            // a long time later, so the "one per day for a week" rule does not apply
            run(
                &t,
                &s,
                None,
                2,
                Utc.with_ymd_and_hms(2027, 3, 1, 0, 0, 0).unwrap(),
            )
            .unwrap();
        }
        let names: Vec<String> = t.files.lock().unwrap().keys().cloned().collect();
        assert_eq!(
            names.iter().filter(|n| n.contains("auto")).count(),
            2,
            "{names:?}"
        );
        assert!(names.iter().any(|n| n.contains("manual")), "manual kept");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn cancelling_removes_what_was_written() {
        let d = tmp();
        let t = MemTarget::default();
        let s = snap(&d, at(5), SnapshotKind::Manual);
        let cancel = AtomicBool::new(false);
        let opts = PushOptions {
            passphrase: None,
            keep: 5,
            now: at(5),
            kdf: Kdf::TEST,
        };
        let r = push(&t, &s, &opts, &cancel, &|p| {
            if p.stage == PushStage::Verifying {
                cancel.store(true, Ordering::Relaxed);
            }
        });
        let e = r.unwrap_err();
        assert!(e.downcast_ref::<Cancelled>().is_some());
        assert!(t.files.lock().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn summary_counts_files_and_finds_the_newest() {
        let d = tmp();
        let t = MemTarget::default();
        for day in [3, 9] {
            let s = snap(&d, at(day), SnapshotKind::Manual);
            run(&t, &s, None, 5, at(day)).unwrap();
        }
        let sum = summarize(&t).unwrap();
        assert_eq!(sum.count, 2);
        assert_eq!(sum.newest, Some(at(9)));
        assert!(sum.bytes > 10_000);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_file_that_is_no_backup_is_refused() {
        let d = tmp();
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("x.db");
        std::fs::write(&f, b"hello").unwrap();
        assert!(
            open_file(&f, None, &d.join("o"))
                .unwrap_err()
                .to_string()
                .contains("not a Bookx backup")
        );
        let _ = std::fs::remove_dir_all(d);
    }
}
