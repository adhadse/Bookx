//! Changing the passphrase of an encrypted repository: re-encryption in place, crashes in the
//! middle, resuming, and what must be refused.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bookx_backup::crypto::Kdf;
use bookx_backup::repo::{
    MemStore, RekeyOptions, RekeyPending, RepoLocked, Repository, RestoreOptions, RunOptions,
    Secrets,
};
use bookx_backup::Cancelled;
use common::{tmp, tree, Lib};

fn no() -> AtomicBool {
    AtomicBool::new(false)
}

fn opts() -> RekeyOptions {
    RekeyOptions {
        kdf: Kdf::TEST,
        verify: true,
        jobs: 2,
    }
}

fn open(m: &Arc<MemStore>, pass: &str) -> anyhow::Result<Repository> {
    Repository::open(Box::new(m.clone()), &Secrets::passphrase(pass))
}

fn change(
    m: &Arc<MemStore>,
    old: &str,
    new: &str,
    cancel: &AtomicBool,
) -> anyhow::Result<bookx_backup::repo::RekeyReport> {
    Repository::change_passphrase(Box::new(m.clone()), old, new, &opts(), cancel, &|_| {})
}

fn run(repo: &Repository, lib: &Lib) -> bookx_backup::repo::Report {
    repo.run(
        &lib.source(),
        &RunOptions {
            jobs: 2,
            gc_grace: None,
            ..RunOptions::default()
        },
        &|_| {},
        &no(),
    )
    .unwrap()
}

/// A library with three books, a note, backed up twice (two snapshots, a journal record).
fn setup(name: &str) -> (Lib, Arc<MemStore>) {
    let mut lib = Lib::new(name);
    let a = lib.add("Alpha", 300);
    lib.add("Beta", 400);
    lib.add("Gamma", 500);
    let m = Arc::new(MemStore::default());
    let repo = Repository::open_or_create(
        Box::new(m.clone()),
        &Secrets::passphrase("old pass"),
        true,
        Kdf::TEST,
    )
    .unwrap();
    run(&repo, &lib);
    lib.highlight(a, "a note");
    repo.journal_append(&lib.source(), chrono::Utc::now(), &no())
        .unwrap();
    lib.write(
        "Ann Author/Extra/extra.bin",
        b"more bytes in a later snapshot",
    );
    repo.run(
        &lib.source(),
        &RunOptions {
            manual: true,
            jobs: 2,
            gc_grace: None,
            ..RunOptions::default()
        },
        &|_| {},
        &no(),
    )
    .unwrap();
    (lib, m)
}

fn object_paths(m: &MemStore) -> Vec<String> {
    m.paths()
        .into_iter()
        .filter(|p| {
            p.starts_with("blobs/") || p.starts_with("snapshots/") || p.starts_with("journal/")
        })
        .collect()
}

fn restores_equal(repo: &Repository, lib: &Lib, name: &str) {
    let to = tmp(name).join("lib");
    repo.restore(None, &to, &RestoreOptions::default(), &|_| {}, &no())
        .unwrap();
    assert_eq!(tree(&lib.root), tree(&to));
}

#[test]
fn a_changed_passphrase_opens_everything_and_the_old_one_nothing() {
    let (lib, m) = setup("rk-basic");
    let before = object_paths(&m);
    let bytes_before: Vec<Vec<u8>> = {
        let f = m.files.lock().unwrap();
        before.iter().map(|p| f[p].clone()).collect()
    };
    let r = change(&m, "old pass", "new pass", &no()).unwrap();
    assert_eq!(r.objects, before.len());
    assert_eq!(r.converted, before.len());
    assert!(!r.resumed && !r.was_done);
    // same names, other bytes
    assert_eq!(object_paths(&m), before, "no object was renamed");
    {
        let f = m.files.lock().unwrap();
        assert!(before
            .iter()
            .zip(&bytes_before)
            .all(|(p, old)| &f[p] != old && f[p].len() == old.len()));
        assert!(!f.contains_key("rekey.json"), "the marker is removed");
        assert!(!f.contains_key("lock.json"));
    }
    let e = open(&m, "old pass").err().unwrap().to_string();
    assert!(e.contains("wrong passphrase"), "{e}");
    let repo = open(&m, "new pass").unwrap();
    assert!(repo.verify(true, 2, &no(), &|_, _| {}).unwrap().ok());
    restores_equal(&repo, &lib, "rk-basic-to");
    // the journal record still replays
    let to = tmp("rk-basic-j").join("lib");
    let rep = repo
        .restore(None, &to, &RestoreOptions::default(), &|_| {}, &no())
        .unwrap();
    assert!(rep.failed.is_empty());
    // the same change again (a second place of the same account, or a retry) has nothing to do
    let again = change(&m, "old pass", "new pass", &no()).unwrap();
    assert!(again.was_done && again.converted == 0, "{again:?}");
    // the library keeps working: nothing is sent again (the ids are unchanged), new data goes in
    let again = run(&repo, &lib);
    assert!(again.up_to_date || again.new_files == 0, "{again:?}");
    let mut lib = lib;
    lib.add("Delta", 200);
    let r = run(&repo, &lib);
    assert_eq!(r.new_files, 1);
    restores_equal(&repo, &lib, "rk-basic-to2");
    // and the passphrase can be changed again
    change(&m, "new pass", "third pass", &no()).unwrap();
    restores_equal(&open(&m, "third pass").unwrap(), &lib, "rk-basic-to3");
}

/// The place refuses every write after `k` more.
fn fail_in(m: &MemStore, k: usize) {
    let now = *m.puts.lock().unwrap();
    *m.fail_after.lock().unwrap() = Some(now + k);
}

#[test]
fn a_break_in_the_middle_is_finished_by_running_it_again() {
    for (i, frac) in [0usize, 1, 2, 3].into_iter().enumerate() {
        let (lib, m) = setup(&format!("rk-crash-{i}"));
        let n = object_paths(&m).len();
        // the marker is write 1, then the objects
        let fail_at = [2, 1 + n / 2, n, 1][frac];
        fail_in(&m, fail_at);
        let e = change(&m, "old pass", "new pass", &no()).unwrap_err();
        assert!(!e.is::<Cancelled>(), "{e:#}");
        *m.fail_after.lock().unwrap() = None;
        for p in ["old pass", "new pass"] {
            let e = open(&m, p).err().unwrap();
            assert!(e.is::<RekeyPending>(), "{p}: {e:#}");
            assert!(e.to_string().contains("interrupted"));
        }
        assert!(!m.files.lock().unwrap().contains_key("lock.json"));
        let r = change(&m, "old pass", "new pass", &no()).unwrap();
        assert!(r.resumed, "{r:?}");
        assert_eq!(r.objects, n);
        assert_eq!(r.already + r.converted, n);
        if fail_at > 1 {
            assert!(r.already >= 1, "{r:?}");
        }
        let repo = open(&m, "new pass").unwrap();
        assert!(repo.verify(true, 2, &no(), &|_, _| {}).unwrap().ok());
        assert!(open(&m, "old pass").is_err());
        restores_equal(&repo, &lib, &format!("rk-crash-to-{i}"));
    }
}

#[test]
fn a_break_just_before_the_commit_only_needs_the_commit() {
    let (lib, m) = setup("rk-commit");
    let n = object_paths(&m).len();
    // the marker and every object get through; the new settings do not
    fail_in(&m, 1 + n);
    assert!(change(&m, "old pass", "new pass", &no()).is_err());
    *m.fail_after.lock().unwrap() = None;
    assert!(m.files.lock().unwrap().contains_key("rekey.json"));
    let r = change(&m, "old pass", "new pass", &no()).unwrap();
    assert_eq!((r.converted, r.already), (0, n), "{r:?}");
    restores_equal(&open(&m, "new pass").unwrap(), &lib, "rk-commit-to");
}

#[test]
fn a_marker_left_after_the_commit_is_harmless() {
    let (lib, m) = setup("rk-marker");
    // keep a copy of the marker while the change runs: the progress callback sees it exist
    let marker: Arc<std::sync::Mutex<Option<Vec<u8>>>> = Arc::default();
    let (mm, mk) = (m.clone(), marker.clone());
    Repository::change_passphrase(
        Box::new(m.clone()),
        "old pass",
        "new pass",
        &opts(),
        &no(),
        &move |_| {
            if let Some(b) = mm.files.lock().unwrap().get("rekey.json") {
                *mk.lock().unwrap() = Some(b.clone());
            }
        },
    )
    .unwrap();
    let saved = marker.lock().unwrap().clone().expect("the marker existed");
    m.files.lock().unwrap().insert("rekey.json".into(), saved);
    // the repository opens (the marker belongs to the settings in place) and works
    let repo = open(&m, "new pass").unwrap();
    restores_equal(&repo, &lib, "rk-marker-to");
    // running the change again just tidies up
    let r = change(&m, "old pass", "new pass", &no()).unwrap();
    assert!(r.was_done);
    assert!(!m.files.lock().unwrap().contains_key("rekey.json"));
}

#[test]
fn cancel_stops_cleanly_and_resumes() {
    let (lib, m) = setup("rk-cancel");
    let cancel = no();
    let seen = std::sync::atomic::AtomicUsize::new(0);
    let e = Repository::change_passphrase(
        Box::new(m.clone()),
        "old pass",
        "new pass",
        &RekeyOptions { jobs: 1, ..opts() },
        &cancel,
        &|p| {
            if p.objects_done >= 3 {
                seen.fetch_add(1, Ordering::Relaxed);
                cancel.store(true, Ordering::Relaxed);
            }
        },
    )
    .unwrap_err();
    assert!(e.is::<Cancelled>(), "{e:#}");
    assert!(open(&m, "old pass").err().unwrap().is::<RekeyPending>());
    let r = change(&m, "old pass", "new pass", &no()).unwrap();
    assert!(r.resumed && r.already >= 3, "{r:?}");
    restores_equal(&open(&m, "new pass").unwrap(), &lib, "rk-cancel-to");
}

#[test]
fn a_wrong_passphrase_or_another_new_one_changes_nothing() {
    let (_lib, m) = setup("rk-wrong");
    let before = m.files.lock().unwrap().clone();
    assert!(change(&m, "not it", "new pass", &no()).is_err());
    assert!(change(&m, "old pass", "", &no()).is_err());
    assert!(change(&m, "old pass", "old pass", &no()).is_err());
    assert_eq!(*m.files.lock().unwrap(), before, "nothing was written");
    // an unfinished change to one new passphrase cannot be continued with another
    fail_in(&m, 3);
    assert!(change(&m, "old pass", "new pass", &no()).is_err());
    *m.fail_after.lock().unwrap() = None;
    let e = change(&m, "old pass", "something else", &no())
        .unwrap_err()
        .to_string();
    assert!(e.contains("not finished"), "{e}");
    assert!(change(&m, "old pass", "new pass", &no()).is_ok());
}

#[test]
fn a_damaged_object_stops_the_change_before_the_commit() {
    let (_lib, m) = setup("rk-damaged");
    let victim = object_paths(&m)
        .into_iter()
        .find(|p| p.starts_with("blobs/"))
        .unwrap();
    {
        let mut f = m.files.lock().unwrap();
        let v = f.get_mut(&victim).unwrap();
        let mid = v.len() / 2;
        v[mid] ^= 0x55;
    }
    let e = change(&m, "old pass", "new pass", &no()).unwrap_err();
    assert!(format!("{e:#}").contains(victim.rsplit('/').next().unwrap()) || !e.is::<Cancelled>());
    // the settings are still the old ones
    assert!(m.files.lock().unwrap().contains_key("rekey.json"));
    let cfg = String::from_utf8(m.files.lock().unwrap()["bookx-repo.json"].clone()).unwrap();
    assert!(!cfg.contains("rekeyed"));
}

#[test]
fn an_unencrypted_repository_has_no_passphrase_to_change() {
    let m = Arc::new(MemStore::default());
    Repository::create(Box::new(m.clone()), &Secrets::none(), false, Kdf::TEST).unwrap();
    let e = change(&m, "a", "b", &no()).unwrap_err().to_string();
    assert!(e.contains("not encrypted"), "{e}");
}

#[test]
fn a_hostile_marker_cannot_make_the_change_do_huge_work() {
    let (_lib, m) = setup("rk-hostile");
    fail_in(&m, 2);
    assert!(change(&m, "old pass", "new pass", &no()).is_err());
    *m.fail_after.lock().unwrap() = None;
    {
        let mut f = m.files.lock().unwrap();
        let mut v: serde_json::Value = serde_json::from_slice(&f["rekey.json"]).unwrap();
        v["kdf"]["memory_kib"] = 4_000_000_000u64.into();
        f.insert("rekey.json".into(), serde_json::to_vec(&v).unwrap());
    }
    let t = std::time::Instant::now();
    let e = change(&m, "old pass", "new pass", &no())
        .unwrap_err()
        .to_string();
    assert!(e.contains("unreasonable"), "{e}");
    assert!(t.elapsed().as_secs() < 5);
}

#[test]
fn another_writer_keeps_the_change_out() {
    let (_lib, m) = setup("rk-locked");
    let other = open(&m, "old pass").unwrap();
    let _held = other.lock().unwrap();
    let e = change(&m, "old pass", "new pass", &no()).unwrap_err();
    assert!(e.is::<RepoLocked>(), "{e:#}");
    assert!(!m.files.lock().unwrap().contains_key("rekey.json"));
}

#[test]
fn big_objects_are_rewritten_through_the_pipe_and_a_cut_leaves_both_kinds() {
    let mut lib = Lib::new("rk-big");
    lib.add("Alpha", 100);
    // several 64 KiB chunks, not compressible
    let mut x = 0x1234_5678_9abc_def1u64;
    let big: Vec<u8> = (0..700_000)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect();
    lib.write("Big Author/Big/big.epub", &big);
    lib.write("Big Author/Big2/big2.epub", &big[1000..]);
    let m = Arc::new(MemStore::default());
    let repo = Repository::open_or_create(
        Box::new(m.clone()),
        &Secrets::passphrase("old pass"),
        true,
        Kdf::TEST,
    )
    .unwrap();
    run(&repo, &lib);
    drop(repo);
    let n = object_paths(&m).len();
    // stops after the marker and about half of the objects
    fail_in(&m, 1 + n / 2);
    assert!(change(&m, "old pass", "new pass", &no()).is_err());
    *m.fail_after.lock().unwrap() = None;
    let r = change(&m, "old pass", "new pass", &no()).unwrap();
    assert!(r.resumed && r.already > 0 && r.converted > 0, "{r:?}");
    let repo = open(&m, "new pass").unwrap();
    assert!(repo.verify(true, 2, &no(), &|_, _| {}).unwrap().ok());
    restores_equal(&repo, &lib, "rk-big-to");
    lib.add("Later", 10);
}
