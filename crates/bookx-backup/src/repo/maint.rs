//! Looking after a repository: checking it (`verify`) and removing what nothing uses (`gc`).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::manifest::Manifest;
use super::run::parallel_for;
use super::seal::valid_id;
use super::{blob_path, Repository};
use crate::push::Cancelled;

const PENDING: &str = "gc/pending.json";
const MAX_PENDING: u64 = 256 * 1024 * 1024;

/// What [`Repository::verify`] found.
#[derive(Debug, Clone, Default)]
pub struct VerifyReport {
    pub deep: bool,
    pub snapshots_checked: usize,
    /// Snapshots that could not be read or are not valid: (file name, why).
    pub snapshots_bad: Vec<(String, String)>,
    pub blobs_checked: usize,
    pub bytes_read: u64,
    /// Blobs a snapshot needs and the place does not have.
    pub missing: Vec<String>,
    /// Blobs that are there but wrong: (id, why). Wrong size when shallow, wrong content when deep.
    pub corrupt: Vec<(String, String)>,
    pub journal_checked: usize,
    pub journal_bad: Vec<(u64, String)>,
}

impl VerifyReport {
    pub fn ok(&self) -> bool {
        self.snapshots_bad.is_empty()
            && self.missing.is_empty()
            && self.corrupt.is_empty()
            && self.journal_bad.is_empty()
    }
}

/// What [`Repository::gc`] did.
#[derive(Debug, Clone, Default)]
pub struct GcReport {
    pub referenced: usize,
    pub blobs_in_place: usize,
    /// Blobs no snapshot uses that were noted this time (removed on a later run, after the grace period).
    pub newly_marked: usize,
    /// Blobs that were noted before, are still unused and old enough: removed.
    pub deleted_blobs: usize,
    pub deleted_bytes: u64,
    /// Leftovers of interrupted uploads.
    pub temp_removed: usize,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Pending {
    #[serde(default)]
    marked: BTreeMap<String, i64>,
}

fn referenced(m: &Manifest, into: &mut HashMap<String, u64>) {
    into.entry(m.catalogue.blob.clone())
        .or_insert(m.catalogue.size);
    into.entry(m.state.blob.clone()).or_insert(m.state.size);
    for f in &m.files {
        into.entry(f.blob.clone()).or_insert(f.size);
    }
}

impl Repository {
    /// Every blob (id and content size) the snapshots in the place need. Fails when any snapshot
    /// cannot be read: what it needs is then unknown.
    pub fn referenced_blobs(&self) -> Result<HashMap<String, u64>> {
        let mut into = HashMap::new();
        for s in self.list_snapshots()? {
            let m = self
                .manifest(&s.name)
                .map_err(|e| anyhow!("snapshot {} cannot be read: {e:#}", s.name.file_name()))?;
            referenced(&m, &mut into);
        }
        Ok(into)
    }

    /// Checks the repository: every snapshot opens and is valid, every blob it needs is there
    /// with the right size and, when `deep`, with the right content (each blob is read back and
    /// checked against its id; this reads everything).
    pub fn verify(
        &self,
        deep: bool,
        jobs: usize,
        cancel: &AtomicBool,
        progress: &(dyn Fn(u64, u64) + Sync),
    ) -> Result<VerifyReport> {
        let mut rep = VerifyReport {
            deep,
            ..VerifyReport::default()
        };
        let mut need: HashMap<String, u64> = HashMap::new();
        for s in self.list_snapshots()? {
            rep.snapshots_checked += 1;
            match self.manifest(&s.name) {
                Ok(m) => referenced(&m, &mut need),
                Err(e) => rep
                    .snapshots_bad
                    .push((s.name.file_name(), format!("{e:#}"))),
            }
        }
        // sizes in the place
        let mut present: HashMap<String, u64> = HashMap::new();
        for d in self.store.list("blobs")? {
            if d.is_dir && d.name.len() == 2 {
                for e in self.store.list(&format!("blobs/{}", d.name))? {
                    if !e.is_dir {
                        present.insert(e.name, e.size);
                    }
                }
            }
        }
        let mut check: Vec<(String, u64)> = Vec::new();
        for (id, size) in &need {
            match present.get(id) {
                None => rep.missing.push(id.clone()),
                Some(have) if *have != self.crypto.stored_len(*size) => rep.corrupt.push((
                    id.clone(),
                    format!(
                        "stored size {have} does not match the expected {}",
                        self.crypto.stored_len(*size)
                    ),
                )),
                Some(_) => check.push((id.clone(), *size)),
            }
        }
        rep.blobs_checked = need.len();
        if deep {
            let total: u64 = check.iter().map(|c| c.1).sum();
            let done = std::sync::atomic::AtomicU64::new(0);
            let bad: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
            parallel_for(check.len(), jobs, |i| {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let (id, size) = &check[i];
                let mut sink = std::io::sink();
                if let Err(e) = self.read_blob(id, *size, &mut sink) {
                    bad.lock().unwrap().push((id.clone(), format!("{e:#}")));
                }
                let d = done.fetch_add(*size, Ordering::Relaxed) + size;
                progress(d, total);
            });
            if cancel.load(Ordering::Relaxed) {
                return Err(anyhow::Error::new(Cancelled));
            }
            rep.bytes_read = done.load(Ordering::Relaxed);
            rep.corrupt.extend(bad.into_inner().unwrap());
        }
        for (seq, at, _) in self.list_journal()? {
            rep.journal_checked += 1;
            if deep {
                if let Err(e) = self.read_journal(seq, at) {
                    rep.journal_bad.push((seq, format!("{e:#}")));
                }
            }
        }
        rep.missing.sort();
        rep.corrupt.sort();
        Ok(rep)
    }

    /// Removes the blobs a verify reported as corrupt, so the next run sends them again.
    pub fn remove_blobs(&self, ids: &[String]) -> Result<usize> {
        let mut n = 0;
        for id in ids {
            if !valid_id(id) {
                bail!("not an object id: {id:?}");
            }
            self.store.delete(&blob_path(id))?;
            n += 1;
        }
        Ok(n)
    }

    /// Removes blobs no snapshot uses, in two steps: a blob found unused is only *noted*; a later
    /// call removes it when it has been unused for `grace` (so a blob that a snapshot being written
    /// right now is about to use is never taken). Nothing is removed when any snapshot cannot be
    /// read. Leftovers of interrupted uploads (`.part-` files) are removed at once.
    pub fn gc(
        &self,
        grace: chrono::Duration,
        now: DateTime<Utc>,
        cancel: &AtomicBool,
    ) -> Result<GcReport> {
        let refs = self.referenced_blobs()?;
        let mut rep = GcReport {
            referenced: refs.len(),
            ..GcReport::default()
        };
        let mut pending: Pending = match self.store.stat(PENDING)? {
            Some(_) => self
                .store
                .get_bytes(PENDING, self.crypto.stored_len(MAX_PENDING))
                .ok()
                .and_then(|b| self.crypto.open_bytes(PENDING, &b, MAX_PENDING).ok())
                .and_then(|p| serde_json::from_slice(&p).ok())
                .unwrap_or_default(),
            None => Pending::default(),
        };
        let mut unused: HashMap<String, u64> = HashMap::new();
        for d in self.store.list("blobs")? {
            if !d.is_dir || d.name.len() != 2 {
                continue;
            }
            for e in self.store.list(&format!("blobs/{}", d.name))? {
                if e.is_dir {
                    continue;
                }
                if e.name.starts_with('.') && e.name.contains(".part-") {
                    self.store.delete(&format!("blobs/{}/{}", d.name, e.name))?;
                    rep.temp_removed += 1;
                } else if valid_id(&e.name) {
                    rep.blobs_in_place += 1;
                    if !refs.contains_key(&e.name) {
                        unused.insert(e.name, e.size);
                    }
                }
            }
        }
        let mut next = Pending::default();
        let mut remaining: HashSet<&String> = HashSet::new();
        for (id, size) in &unused {
            if cancel.load(Ordering::Relaxed) {
                return Err(anyhow::Error::new(Cancelled));
            }
            match pending.marked.get(id) {
                Some(&t) if now.timestamp() - t >= grace.num_seconds() => {
                    self.store.delete(&blob_path(id))?;
                    rep.deleted_blobs += 1;
                    rep.deleted_bytes += size;
                }
                Some(&t) => {
                    next.marked.insert(id.clone(), t);
                    remaining.insert(id);
                }
                None => {
                    next.marked.insert(id.clone(), now.timestamp());
                    rep.newly_marked += 1;
                }
            }
        }
        pending.marked.clear();
        if next.marked.is_empty() && self.store.stat(PENDING)?.is_none() {
            return Ok(rep);
        }
        self.put_checked(PENDING, &serde_json::to_vec(&next)?, cancel)?;
        Ok(rep)
    }
}
