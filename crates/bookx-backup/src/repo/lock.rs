//! One writer at a time: the lock file in a repository.
//!
//! Two computers backing up into the same folder would reuse snapshot numbers and could remove
//! each other's unused blobs, so a writer first puts `lock.json` in the repository (who, since
//! when, last sign of life). Another writer that finds a fresh lock stops with [`RepoLocked`]
//! and the UI offers *Take Over*. A lock whose last sign of life is older than [`STALE_AFTER`],
//! or that this very computer left behind when its process is gone, is taken without asking.
//!
//! The lock is advisory and best effort: a folder, a share or a WebDAV server offers no atomic
//! "create if missing" that all of them honour, so after writing the lock the file is read back
//! and the writer whose token is there wins. The window for two simultaneous starts is the
//! round trip of one small write; the rollback check and the per-run listing of snapshots stay
//! as the second line of defence. Nothing here is a security boundary: whoever can write the
//! place can also delete the lock.

use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::Repository;

/// How the message about a busy place starts (the app looks for it in error texts).
pub const IN_USE: &str = "this backup place is in use";

/// The file in the repository folder.
pub const LOCK_FILE: &str = "lock.json";
/// A lock with no sign of life for this long is taken without asking.
pub const STALE_AFTER: Duration = Duration::from_secs(15 * 60);
/// How often a running writer renews its lock.
const BEAT_EVERY: Duration = Duration::from_secs(60);
const MAX_LOCK_BYTES: u64 = 4096;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LockInfo {
    pub format: u32,
    /// What the computer calls itself.
    pub host: String,
    /// Process number on that computer (only used to see whether it is still there).
    pub pid: u32,
    /// Random, one per acquisition: tells two writers on the same computer apart.
    pub token: String,
    pub since: DateTime<Utc>,
    pub beat: DateTime<Utc>,
    pub app: String,
}

/// Somebody else is writing to this repository.
#[derive(Debug, Clone)]
pub struct RepoLocked {
    pub host: String,
    pub since: DateTime<Utc>,
    pub beat: DateTime<Utc>,
}

impl std::fmt::Display for RepoLocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{IN_USE}: {} has been writing to it since {} (last sign of life {}). Two computers must not write one backup at once. Wait until it is done, or take over if you know it stopped",
            self.host,
            self.since.format("%Y-%m-%d %H:%M UTC"),
            self.beat.format("%H:%M UTC"),
        )
    }
}
impl std::error::Error for RepoLocked {}

/// This writer's lock was removed or replaced while it was working.
#[derive(Debug)]
pub struct LockLost;

impl std::fmt::Display for LockLost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("another computer took over this backup place while it was being written; nothing more was written")
    }
}
impl std::error::Error for LockLost {}

/// What a repository handle remembers about its own lock (re-entrant: `run` calls `prune`).
#[derive(Default)]
pub(crate) struct LockState {
    token: String,
    depth: usize,
    last_beat: Option<Instant>,
    since: Option<DateTime<Utc>>,
}

/// What this computer calls itself, for the lock file and the message on the other one.
pub fn host_name() -> String {
    for p in ["/proc/sys/kernel/hostname", "/etc/hostname"] {
        if let Ok(t) = std::fs::read_to_string(p) {
            if let Some(h) = clean_host(&t) {
                return h;
            }
        }
    }
    for v in ["HOSTNAME", "COMPUTERNAME", "HOST"] {
        if let Some(h) = std::env::var(v).ok().and_then(|t| clean_host(&t)) {
            return h;
        }
    }
    "another computer".into()
}

fn clean_host(t: &str) -> Option<String> {
    let h: String = t
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect();
    (!h.is_empty()).then_some(h)
}

/// Whether a lock found in the place can be taken without asking.
pub fn is_stale(l: &LockInfo, now: DateTime<Utc>, my_host: &str) -> bool {
    if l.format != super::manifest::FORMAT {
        return false;
    }
    let idle = now.signed_duration_since(l.beat);
    if idle.to_std().is_ok_and(|d| d > STALE_AFTER) {
        return true;
    }
    // this computer, process gone: a crash left it behind
    l.host == my_host && !process_alive(l.pid)
}

fn process_alive(pid: u32) -> bool {
    if cfg!(target_os = "linux") {
        std::path::Path::new("/proc").join(pid.to_string()).exists()
    } else {
        true
    }
}

/// The lock, while held. Dropping it releases the lock when it is still this writer's.
pub struct LockGuard<'a> {
    repo: &'a Repository,
}

impl Repository {
    /// Reads the lock file, `None` when the place has none (or it cannot be read as a lock).
    pub fn lock_info(&self) -> Result<Option<LockInfo>> {
        if self.store.stat(LOCK_FILE)?.is_none() {
            return Ok(None);
        }
        let bytes = self.store.get_bytes(LOCK_FILE, MAX_LOCK_BYTES)?;
        Ok(serde_json::from_slice(&bytes).ok())
    }

    /// Takes the lock for writing (re-entrant for the same handle). Fails with [`RepoLocked`]
    /// (downcast the error) when another writer holds a fresh one.
    pub fn lock(&self) -> Result<LockGuard<'_>> {
        let now = Utc::now();
        let mut st = self.lock_state.lock().unwrap();
        if st.depth > 0 {
            st.depth += 1;
            return Ok(LockGuard { repo: self });
        }
        let me = host_name();
        if self.store.stat(LOCK_FILE)?.is_some() {
            let found = self
                .store
                .get_bytes(LOCK_FILE, MAX_LOCK_BYTES)
                .ok()
                .and_then(|b| serde_json::from_slice::<LockInfo>(&b).ok());
            if let Some(l) = found {
                if !is_stale(&l, now, &me) {
                    return Err(anyhow::Error::new(RepoLocked {
                        host: l.host,
                        since: l.since,
                        beat: l.beat,
                    }));
                }
            }
            // stale, or not a lock at all: replaced below
        }
        let token = super::seal::random_hex(8)?;
        let info = LockInfo {
            format: super::manifest::FORMAT,
            host: me,
            pid: std::process::id(),
            token: token.clone(),
            since: now,
            beat: now,
            app: format!("Bookx {}", env!("CARGO_PKG_VERSION")),
        };
        self.write_lock(&info)?;
        // two starts at the same moment: whoever's token is in the file now won
        let back = self.lock_info()?;
        let Some(l) = back else {
            self.store.delete(LOCK_FILE).ok();
            anyhow::bail!("the backup place does not keep the lock file correctly");
        };
        if l.token != token {
            return Err(anyhow::Error::new(RepoLocked {
                host: l.host,
                since: l.since,
                beat: l.beat,
            }));
        }
        st.token = token;
        st.depth = 1;
        st.last_beat = Some(Instant::now());
        st.since = Some(now);
        Ok(LockGuard { repo: self })
    }

    fn write_lock(&self, info: &LockInfo) -> Result<()> {
        self.store
            .put_bytes(
                LOCK_FILE,
                &serde_json::to_vec(info)?,
                &AtomicBool::new(false),
            )
            .context("writing the backup lock")
    }

    /// Removes the lock whatever it says: the user confirmed that the other writer stopped.
    pub fn take_over(&self) -> Result<Option<LockInfo>> {
        Repository::take_over_lock(self.store.as_ref())
    }

    /// [`Repository::take_over`] for a place whose repository is not opened (no passphrase needed).
    pub fn take_over_lock(store: &dyn super::store::RepoStore) -> Result<Option<LockInfo>> {
        let old = store
            .stat(LOCK_FILE)?
            .and_then(|_| store.get_bytes(LOCK_FILE, MAX_LOCK_BYTES).ok())
            .and_then(|b| serde_json::from_slice::<LockInfo>(&b).ok());
        store.delete(LOCK_FILE)?;
        Ok(old)
    }

    /// Whether this handle still holds the lock. Writers call it before the step that must not
    /// happen without it (the snapshot manifest).
    pub fn lock_held(&self) -> Result<()> {
        let st = self.lock_state.lock().unwrap();
        if st.depth == 0 {
            return Ok(());
        }
        let token = st.token.clone();
        drop(st);
        match self.lock_info() {
            Ok(Some(l)) if l.token == token => Ok(()),
            Ok(_) => Err(anyhow::Error::new(LockLost)),
            // a place that cannot be read will fail the next write by itself
            Err(_) => Ok(()),
        }
    }

    /// Renews the lock at most once a minute. `false` when it was taken over.
    pub fn lock_beat(&self) -> bool {
        let mut st = self.lock_state.lock().unwrap();
        if st.depth == 0 || st.last_beat.is_some_and(|t| t.elapsed() < BEAT_EVERY) {
            return true;
        }
        st.last_beat = Some(Instant::now());
        let token = st.token.clone();
        let since = st.since.unwrap_or_else(Utc::now);
        drop(st);
        match self.lock_info() {
            Ok(Some(mut l)) if l.token == token => {
                l.beat = Utc::now();
                l.since = since;
                let _ = self.write_lock(&l);
                true
            }
            Ok(_) => false,
            Err(_) => true,
        }
    }
}

impl Drop for LockGuard<'_> {
    fn drop(&mut self) {
        let mut st = self.repo.lock_state.lock().unwrap();
        st.depth = st.depth.saturating_sub(1);
        if st.depth > 0 {
            return;
        }
        let token = std::mem::take(&mut st.token);
        drop(st);
        // only remove a lock that is still ours
        if let Ok(Some(l)) = self.repo.lock_info() {
            if l.token == token {
                let _ = self.repo.store.delete(LOCK_FILE);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(host: &str, pid: u32, age_min: i64) -> LockInfo {
        let now = Utc::now();
        LockInfo {
            format: super::super::manifest::FORMAT,
            host: host.into(),
            pid,
            token: "t".into(),
            since: now - chrono::Duration::minutes(age_min),
            beat: now - chrono::Duration::minutes(age_min),
            app: "x".into(),
        }
    }

    #[test]
    fn a_fresh_lock_of_another_computer_is_respected_an_old_one_is_not() {
        let now = Utc::now();
        assert!(!is_stale(&info("other", 1, 1), now, "me"));
        assert!(is_stale(&info("other", 1, 16), now, "me"));
        assert!(is_stale(&info("other", 1, 600), now, "me"));
    }

    #[test]
    fn a_lock_of_this_computers_dead_process_is_stale() {
        let now = Utc::now();
        // no process has this number
        assert!(is_stale(&info("me", u32::MAX - 1, 0), now, "me") || !cfg!(target_os = "linux"));
        assert!(!is_stale(&info("me", std::process::id(), 0), now, "me"));
    }

    #[test]
    fn host_names_are_clean() {
        assert_eq!(clean_host("  box\n").as_deref(), Some("box"));
        assert_eq!(clean_host("\n"), None);
        assert!(clean_host(&"a".repeat(500)).unwrap().len() <= 64);
    }
}
