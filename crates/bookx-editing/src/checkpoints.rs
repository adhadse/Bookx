//! Named snapshots of an [`Archive`] (E13), sharing unchanged file contents between snapshots.

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use bookx_core::archive::Archive;
use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::compare::{compare_archives, ArchiveDiff, CompareOptions};

#[derive(Debug, Clone)]
struct Snapshot {
    id: u64,
    name: String,
    created: i64,
    pinned: bool,
    /// Files in archive order.
    files: Vec<(String, Arc<[u8]>)>,
}

/// Public description of a checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointInfo {
    pub id: u64,
    pub name: String,
    /// Unix seconds.
    pub created: i64,
    pub pinned: bool,
    pub file_count: usize,
    pub bytes: usize,
}

#[derive(Debug, Clone)]
pub struct Checkpoints {
    snapshots: Vec<Snapshot>,
    max: usize,
    next_id: u64,
}

impl Default for Checkpoints {
    fn default() -> Self {
        Self::new(20)
    }
}

#[derive(Serialize, Deserialize)]
struct StoredFile {
    name: String,
    blob: usize,
}

#[derive(Serialize, Deserialize)]
struct StoredSnapshot {
    id: u64,
    name: String,
    created: i64,
    pinned: bool,
    files: Vec<StoredFile>,
}

#[derive(Serialize, Deserialize)]
struct StoredIndex {
    version: u32,
    max: usize,
    next_id: u64,
    snapshots: Vec<StoredSnapshot>,
}

impl Checkpoints {
    /// `max` is the number of checkpoints kept (at least 1); the oldest unpinned ones are dropped beyond that.
    pub fn new(max: usize) -> Self {
        Self {
            snapshots: Vec::new(),
            max: max.max(1),
            next_id: 1,
        }
    }

    pub fn max(&self) -> usize {
        self.max
    }

    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    /// Takes a snapshot named `name` (an empty name becomes `Checkpoint N`) and returns its id. Files equal to the
    /// ones in the newest snapshot share their allocation with it. Prunes beyond the configured maximum.
    pub fn create(&mut self, name: &str, archive: &Archive) -> u64 {
        self.create_at(name, archive, chrono::Utc::now().timestamp())
    }

    /// Like [`Checkpoints::create`] with an explicit creation time (unix seconds).
    pub fn create_at(&mut self, name: &str, archive: &Archive, created: i64) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let prev: HashMap<&str, &Arc<[u8]>> = self
            .snapshots
            .last()
            .map(|s| s.files.iter().map(|(n, d)| (n.as_str(), d)).collect())
            .unwrap_or_default();
        let files: Vec<(String, Arc<[u8]>)> = archive
            .names()
            .iter()
            .filter_map(|n| {
                let data = archive.get(n)?;
                let shared = match prev.get(n.as_str()) {
                    Some(p) if p.as_ref() == data => Arc::clone(p),
                    _ => Arc::from(data),
                };
                Some((n.clone(), shared))
            })
            .collect();
        let name = if name.trim().is_empty() {
            format!("Checkpoint {id}")
        } else {
            name.trim().to_string()
        };
        self.snapshots.push(Snapshot {
            id,
            name,
            created,
            pinned: false,
            files,
        });
        self.prune();
        id
    }

    fn prune(&mut self) {
        while self.snapshots.len() > self.max {
            // never drop the newest one
            let newest = self.snapshots.len() - 1;
            match self
                .snapshots
                .iter()
                .enumerate()
                .position(|(i, s)| !s.pinned && i != newest)
            {
                Some(i) => {
                    self.snapshots.remove(i);
                }
                None => break,
            }
        }
    }

    /// Oldest first.
    pub fn list(&self) -> Vec<CheckpointInfo> {
        self.snapshots
            .iter()
            .map(|s| CheckpointInfo {
                id: s.id,
                name: s.name.clone(),
                created: s.created,
                pinned: s.pinned,
                file_count: s.files.len(),
                bytes: s.files.iter().map(|(_, d)| d.len()).sum(),
            })
            .collect()
    }

    fn find(&self, id: u64) -> Result<&Snapshot> {
        self.snapshots
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| anyhow!("no checkpoint with id {id}"))
    }

    /// Rebuilds the archive as it was when the checkpoint was taken.
    pub fn restore(&self, id: u64) -> Result<Archive> {
        let s = self.find(id)?;
        let mut a = Archive::default();
        for (n, d) in &s.files {
            a.set(n, d.to_vec());
        }
        Ok(a)
    }

    /// Replaces the content of `archive` with the checkpoint's.
    pub fn restore_into(&self, id: u64, archive: &mut Archive) -> Result<()> {
        *archive = self.restore(id)?;
        Ok(())
    }

    /// What changed between the checkpoint (old) and `current` (new).
    pub fn diff_against(
        &self,
        id: u64,
        current: &Archive,
        opts: CompareOptions,
    ) -> Result<ArchiveDiff> {
        Ok(compare_archives(&self.restore(id)?, current, opts))
    }

    pub fn delete(&mut self, id: u64) -> Result<()> {
        let before = self.snapshots.len();
        self.snapshots.retain(|s| s.id != id);
        if self.snapshots.len() == before {
            bail!("no checkpoint with id {id}");
        }
        Ok(())
    }

    pub fn rename(&mut self, id: u64, name: &str) -> Result<()> {
        let s = self
            .snapshots
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| anyhow!("no checkpoint with id {id}"))?;
        s.name = name.trim().to_string();
        Ok(())
    }

    /// Pinned checkpoints survive pruning.
    pub fn set_pinned(&mut self, id: u64, pinned: bool) -> Result<()> {
        let s = self
            .snapshots
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| anyhow!("no checkpoint with id {id}"))?;
        s.pinned = pinned;
        Ok(())
    }

    /// Number of distinct file allocations held (shared files counted once) and their total size.
    pub fn memory_usage(&self) -> (usize, usize) {
        let mut seen: HashMap<*const u8, usize> = HashMap::new();
        for s in &self.snapshots {
            for (_, d) in &s.files {
                seen.insert(d.as_ptr(), d.len());
            }
        }
        (seen.len(), seen.values().sum())
    }

    /// Whether `file` is stored once for both checkpoints.
    pub fn shares_file(&self, a: u64, b: u64, file: &str) -> bool {
        let get = |id: u64| {
            self.find(id).ok().and_then(|s| {
                s.files
                    .iter()
                    .find(|(n, _)| n == file)
                    .map(|(_, d)| d.clone())
            })
        };
        match (get(a), get(b)) {
            (Some(x), Some(y)) => Arc::ptr_eq(&x, &y),
            _ => false,
        }
    }

    /// Serialises every checkpoint into one zip (`index.json` + deduplicated `blobs/N`).
    pub fn to_zip(&self) -> Result<Vec<u8>> {
        let mut blobs: Vec<&Arc<[u8]>> = Vec::new();
        let mut index: HashMap<*const u8, usize> = HashMap::new();
        let mut snapshots = Vec::new();
        for s in &self.snapshots {
            let mut files = Vec::new();
            for (n, d) in &s.files {
                let blob = *index.entry(d.as_ptr()).or_insert_with(|| {
                    blobs.push(d);
                    blobs.len() - 1
                });
                files.push(StoredFile {
                    name: n.clone(),
                    blob,
                });
            }
            snapshots.push(StoredSnapshot {
                id: s.id,
                name: s.name.clone(),
                created: s.created,
                pinned: s.pinned,
                files,
            });
        }
        let idx = StoredIndex {
            version: 1,
            max: self.max,
            next_id: self.next_id,
            snapshots,
        };
        let mut out = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut out);
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            zip.start_file("index.json", opts)?;
            zip.write_all(&serde_json::to_vec(&idx)?)?;
            for (i, b) in blobs.iter().enumerate() {
                zip.start_file(format!("blobs/{i}"), opts)?;
                zip.write_all(b)?;
            }
            zip.finish()?;
        }
        Ok(out.into_inner())
    }

    pub fn from_zip(data: &[u8]) -> Result<Self> {
        let mut zip = ZipArchive::new(Cursor::new(data)).context("not a checkpoint archive")?;
        let idx: StoredIndex = {
            let mut f = zip
                .by_name("index.json")
                .context("checkpoint archive has no index.json")?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;
            serde_json::from_slice(&buf).context("corrupt checkpoint index")?
        };
        if idx.version != 1 {
            bail!("unsupported checkpoint archive version {}", idx.version);
        }
        let mut blobs: HashMap<usize, Arc<[u8]>> = HashMap::new();
        let mut snapshots = Vec::new();
        for s in idx.snapshots {
            let mut files = Vec::new();
            for f in s.files {
                let data = match blobs.get(&f.blob) {
                    Some(d) => Arc::clone(d),
                    None => {
                        let mut e = zip
                            .by_name(&format!("blobs/{}", f.blob))
                            .with_context(|| format!("missing blob {}", f.blob))?;
                        let mut buf = Vec::new();
                        e.read_to_end(&mut buf)?;
                        let arc: Arc<[u8]> = Arc::from(buf);
                        blobs.insert(f.blob, Arc::clone(&arc));
                        arc
                    }
                };
                files.push((f.name, data));
            }
            snapshots.push(Snapshot {
                id: s.id,
                name: s.name,
                created: s.created,
                pinned: s.pinned,
                files,
            });
        }
        Ok(Self {
            snapshots,
            max: idx.max.max(1),
            next_id: idx.next_id.max(1),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arc(files: &[(&str, &str)]) -> Archive {
        let mut a = Archive::default();
        for (n, d) in files {
            a.set_string(n, d);
        }
        a
    }

    #[test]
    fn create_list_restore() {
        let mut c = Checkpoints::new(5);
        assert!(c.is_empty());
        let mut a = arc(&[("a.xhtml", "one"), ("b.css", "x")]);
        let id1 = c.create_at("before", &a, 100);
        a.set_string("a.xhtml", "two");
        a.set_string("c.png", "bin");
        let id2 = c.create_at("", &a, 200);
        assert_ne!(id1, id2);
        let l = c.list();
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].name, "before");
        assert_eq!(l[0].created, 100);
        assert_eq!(l[0].file_count, 2);
        assert_eq!(l[0].bytes, 4);
        assert_eq!(l[1].name, format!("Checkpoint {id2}"));
        let r = c.restore(id1).unwrap();
        assert_eq!(r.get_string("a.xhtml").unwrap(), "one");
        assert!(!r.contains("c.png"));
        assert_eq!(r.names(), ["a.xhtml", "b.css"]);
        let mut cur = arc(&[("z", "z")]);
        c.restore_into(id2, &mut cur).unwrap();
        assert_eq!(cur.get_string("a.xhtml").unwrap(), "two");
        assert!(!cur.contains("z"));
    }

    #[test]
    fn unchanged_files_are_shared() {
        let mut c = Checkpoints::new(5);
        let mut a = arc(&[("big", "0123456789"), ("small", "s")]);
        let i1 = c.create("1", &a);
        a.set_string("small", "t");
        let i2 = c.create("2", &a);
        assert!(c.shares_file(i1, i2, "big"));
        assert!(!c.shares_file(i1, i2, "small"));
        assert!(!c.shares_file(i1, i2, "nope"));
        assert!(!c.shares_file(i1, 99, "big"));
        let (allocs, bytes) = c.memory_usage();
        assert_eq!(allocs, 3);
        assert_eq!(bytes, 12);
    }

    #[test]
    fn bounded_with_pruning_and_pins() {
        let mut c = Checkpoints::new(3);
        let a = arc(&[("f", "x")]);
        let ids: Vec<u64> = (0..3).map(|i| c.create(&format!("c{i}"), &a)).collect();
        c.set_pinned(ids[0], true).unwrap();
        let id3 = c.create("c3", &a);
        let names: Vec<String> = c.list().into_iter().map(|i| i.name).collect();
        assert_eq!(
            names,
            vec!["c0", "c2", "c3"],
            "oldest unpinned (c1) dropped"
        );
        assert!(c.restore(ids[1]).is_err());
        c.set_pinned(ids[2], true).unwrap();
        c.set_pinned(id3, true).unwrap();
        let id4 = c.create("c4", &a);
        // everything older is pinned: the bound is exceeded rather than losing pinned data, newest is never dropped
        assert_eq!(c.len(), 4);
        assert!(c.restore(id4).is_ok());
        assert_eq!(c.max(), 3);
        assert_eq!(Checkpoints::new(0).max(), 1);
    }

    #[test]
    fn diff_against_current() {
        let mut c = Checkpoints::new(5);
        let mut a = arc(&[("a.xhtml", "l1\nl2\n"), ("old.css", "x"), ("keep", "k")]);
        let id = c.create("s", &a);
        a.set_string("a.xhtml", "l1\nl2 changed\n");
        a.remove("old.css");
        a.set_string("new.css", "y");
        let d = c.diff_against(id, &a, CompareOptions::default()).unwrap();
        assert_eq!(d.added, vec!["new.css"]);
        assert_eq!(d.removed, vec!["old.css"]);
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.unchanged, 1);
        assert!(c.diff_against(77, &a, CompareOptions::default()).is_err());
    }

    #[test]
    fn delete_and_rename_errors() {
        let mut c = Checkpoints::new(5);
        let id = c.create("a", &arc(&[("f", "x")]));
        c.rename(id, "  renamed ").unwrap();
        assert_eq!(c.list()[0].name, "renamed");
        assert_eq!(
            c.delete(42).unwrap_err().to_string(),
            "no checkpoint with id 42"
        );
        assert!(c.rename(42, "x").is_err());
        assert!(c.set_pinned(42, true).is_err());
        c.delete(id).unwrap();
        assert!(c.is_empty());
        assert_eq!(
            c.restore(id).unwrap_err().to_string(),
            format!("no checkpoint with id {id}")
        );
    }

    #[test]
    fn zip_roundtrip_keeps_everything_and_sharing() {
        let mut c = Checkpoints::new(7);
        let mut a = arc(&[("a", "shared"), ("b", "v1")]);
        a.set("bin", vec![0, 1, 2, 255]);
        let i1 = c.create_at("one", &a, 11);
        a.set_string("b", "v2");
        let i2 = c.create_at("two", &a, 22);
        c.set_pinned(i2, true).unwrap();
        let bytes = c.to_zip().unwrap();

        // independent check: it is a zip with index.json and fewer blobs than file entries
        let mut z = ZipArchive::new(Cursor::new(&bytes)).unwrap();
        let names: Vec<String> = (0..z.len())
            .map(|i| z.by_index(i).unwrap().name().unwrap().to_string())
            .collect();
        assert!(names.contains(&"index.json".to_string()));
        assert_eq!(names.iter().filter(|n| n.starts_with("blobs/")).count(), 4);

        let back = Checkpoints::from_zip(&bytes).unwrap();
        assert_eq!(back.list(), c.list());
        assert_eq!(back.max(), 7);
        assert!(back.shares_file(i1, i2, "a"));
        assert!(!back.shares_file(i1, i2, "b"));
        let r = back.restore(i1).unwrap();
        assert_eq!(r.get("bin").unwrap(), &[0, 1, 2, 255]);
        assert_eq!(r.get_string("b").unwrap(), "v1");
        let mut back = back;
        let i3 = back.create("three", &a);
        assert!(i3 > i2, "ids keep increasing after a reload");
    }

    #[test]
    fn from_zip_rejects_garbage() {
        assert!(Checkpoints::from_zip(b"nope").is_err());
        let mut out = Cursor::new(Vec::new());
        {
            let mut z = ZipWriter::new(&mut out);
            z.start_file("other", SimpleFileOptions::default()).unwrap();
            z.write_all(b"x").unwrap();
            z.finish().unwrap();
        }
        let e = Checkpoints::from_zip(&out.into_inner()).unwrap_err();
        assert!(e.to_string().contains("no index.json"), "{e}");

        let mut out = Cursor::new(Vec::new());
        {
            let mut z = ZipWriter::new(&mut out);
            z.start_file("index.json", SimpleFileOptions::default())
                .unwrap();
            z.write_all(br#"{"version":9,"max":1,"next_id":1,"snapshots":[]}"#)
                .unwrap();
            z.finish().unwrap();
        }
        let e = Checkpoints::from_zip(&out.into_inner()).unwrap_err();
        assert!(e.to_string().contains("unsupported"), "{e}");

        let mut out = Cursor::new(Vec::new());
        {
            let mut z = ZipWriter::new(&mut out);
            z.start_file("index.json", SimpleFileOptions::default())
                .unwrap();
            z.write_all(br#"{"version":1,"max":1,"next_id":1,"snapshots":[{"id":1,"name":"x","created":0,"pinned":false,"files":[{"name":"a","blob":3}]}]}"#).unwrap();
            z.finish().unwrap();
        }
        let e = Checkpoints::from_zip(&out.into_inner()).unwrap_err();
        assert!(e.to_string().contains("missing blob 3"), "{e}");
    }

    #[test]
    fn empty_archive_snapshot() {
        let mut c = Checkpoints::default();
        let id = c.create("empty", &Archive::default());
        assert_eq!(c.restore(id).unwrap().names().len(), 0);
        let z = c.to_zip().unwrap();
        assert_eq!(Checkpoints::from_zip(&z).unwrap().len(), 1);
    }
}
