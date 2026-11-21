//! One writer per repository (the lock file), collections in the journal, and big uploads to
//! WebDAV in pieces that continue after a break.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bookx_backup::crypto::Kdf;
use bookx_backup::repo::lock::{host_name, LockInfo, LOCK_FILE};
use bookx_backup::repo::{
    DavStore, LockLost, MemStore, RepoLocked, Report, Repository, RestoreOptions, RunOptions,
    Secrets,
};
use chrono::{Duration, Utc};
use common::{dav, tmp, tree, Lib};

fn no() -> AtomicBool {
    AtomicBool::new(false)
}

fn open(m: &Arc<MemStore>) -> Repository {
    Repository::open_or_create(Box::new(m.clone()), &Secrets::none(), false, Kdf::TEST).unwrap()
}

fn opts() -> RunOptions {
    RunOptions {
        jobs: 2,
        gc_grace: None,
        ..RunOptions::default()
    }
}

fn run(repo: &Repository, lib: &Lib) -> anyhow::Result<Report> {
    repo.run(&lib.source(), &opts(), &|_| {}, &no())
}

fn put_lock(m: &MemStore, host: &str, pid: u32, age_minutes: i64) {
    let at = Utc::now() - Duration::minutes(age_minutes);
    let l = LockInfo {
        format: 1,
        host: host.into(),
        pid,
        token: "someone-else".into(),
        since: at,
        beat: at,
        app: "Bookx test".into(),
    };
    m.files
        .lock()
        .unwrap()
        .insert(LOCK_FILE.into(), serde_json::to_vec(&l).unwrap());
}

fn has_lock(m: &MemStore) -> bool {
    m.files.lock().unwrap().contains_key(LOCK_FILE)
}

#[test]
fn a_second_computer_cannot_write_while_the_first_does_and_the_lock_goes_away_after() {
    let mut lib = Lib::new("lock-two");
    let alpha = lib.add("Alpha", 100);
    lib.highlight(alpha, "a note to journal");
    let m = Arc::new(MemStore::default());
    let a = open(&m);
    let b = open(&m);
    {
        let _held = a.lock().unwrap();
        assert!(has_lock(&m));
        let puts = *m.puts.lock().unwrap();
        for e in [
            run(&b, &lib).unwrap_err(),
            b.journal_append(&lib.source(), Utc::now(), &no())
                .unwrap_err(),
            b.gc(Duration::zero(), Utc::now(), &no()).unwrap_err(),
            b.prune(1, Utc::now()).unwrap_err(),
        ] {
            let locked = e.downcast_ref::<RepoLocked>().expect("a typed error");
            assert!(!locked.host.is_empty());
            assert!(e.to_string().contains("in use"), "{e}");
        }
        assert_eq!(*m.puts.lock().unwrap(), puts, "nothing was written");
        // the same handle may nest (run calls prune and gc)
        let _again = a.lock().unwrap();
    }
    assert!(!has_lock(&m), "released with the last guard");
    run(&b, &lib).unwrap();
    assert!(!has_lock(&m));
}

#[test]
fn a_failed_run_still_releases_the_lock() {
    let mut lib = Lib::new("lock-fail");
    lib.add("Alpha", 100);
    let m = Arc::new(MemStore::default());
    let repo = open(&m);
    *m.fail_after.lock().unwrap() = Some(1);
    assert!(run(&repo, &lib).is_err());
    assert!(!has_lock(&m));
}

#[test]
fn old_locks_and_locks_of_dead_processes_are_taken_without_asking() {
    let mut lib = Lib::new("lock-stale");
    let alpha = lib.add("Alpha", 100);
    let m = Arc::new(MemStore::default());
    let repo = open(&m);
    // nobody has written for twenty minutes
    put_lock(&m, "other-pc", 1, 20);
    run(&repo, &lib).unwrap();
    assert!(!has_lock(&m));
    // a crash of this computer's own process
    if cfg!(target_os = "linux") {
        put_lock(&m, &host_name(), u32::MAX - 1, 0);
        lib.highlight(alpha, "n");
        repo.journal_append(&lib.source(), Utc::now(), &no())
            .unwrap();
        assert!(!has_lock(&m));
    }
    // a fresh lock of another computer is respected
    put_lock(&m, "other-pc", 1, 2);
    assert!(run(&repo, &lib).unwrap_err().is::<RepoLocked>());
    assert!(has_lock(&m), "somebody else's lock is not removed");
}

#[test]
fn a_damaged_or_huge_lock_file_does_not_block_forever() {
    let mut lib = Lib::new("lock-junk");
    let alpha = lib.add("Alpha", 100);
    let m = Arc::new(MemStore::default());
    let repo = open(&m);
    for junk in [b"not json".to_vec(), vec![b'x'; 1 << 20], b"{}".to_vec()] {
        m.files.lock().unwrap().insert(LOCK_FILE.into(), junk);
        lib.highlight(alpha, "n");
        repo.journal_append(&lib.source(), Utc::now(), &no())
            .unwrap();
        assert!(!has_lock(&m));
    }
}

#[test]
fn taking_over_stops_the_old_writer_before_it_writes_its_snapshot() {
    let mut lib = Lib::new("lock-over");
    lib.add("Alpha", 100);
    let m = Arc::new(MemStore::default());
    let a = open(&m);
    let b = open(&m);
    let held = a.lock().unwrap();
    let old = b.take_over().unwrap().expect("there was a lock");
    assert_eq!(old.host, host_name());
    // b is now the only writer
    run(&b, &lib).unwrap();
    // a's lock is gone (b took over and released): a notices before it writes its snapshot
    assert!(a.lock_held().unwrap_err().is::<LockLost>());
    drop(held);
    // dropping a's guard must not remove a lock that is not its own
    let _b = b.lock().unwrap();
    drop(a);
    assert!(has_lock(&m));
}

#[test]
fn shelves_made_between_snapshots_come_back_from_the_journal() {
    let mut lib = Lib::new("jr-cols");
    let a = lib.add("Alpha", 100);
    let b = lib.add("Beta", 100);
    let m = Arc::new(MemStore::default());
    let repo = open(&m);
    run(&repo, &lib).unwrap();
    let st = lib.opened.store.clone();
    let shelf = st.create_collection("Sci-Fi").unwrap();
    st.add_to_collection(shelf, a).unwrap();
    st.add_to_collection(shelf, b).unwrap();
    let j = repo
        .journal_append(&lib.source(), Utc::now(), &no())
        .unwrap();
    assert!(j.collections && j.books == 0 && j.seq.is_some(), "{j:?}");
    // unchanged: nothing more
    assert!(repo
        .journal_append(&lib.source(), Utc::now(), &no())
        .unwrap()
        .seq
        .is_none());
    // a change inside a shelf is a new record
    st.remove_from_collection(shelf, b).unwrap();
    assert!(
        repo.journal_append(&lib.source(), Utc::now(), &no())
            .unwrap()
            .collections
    );

    let to = tmp("jr-cols-to").join("lib");
    let rep = repo
        .restore(None, &to, &RestoreOptions::default(), &|_| {}, &no())
        .unwrap();
    assert_eq!(rep.journal_records, 2);
    assert_eq!(rep.journal_collections, 1);
    let o = bookx_store::open_library(&to).unwrap();
    let cols = o.store.list_collections().unwrap();
    assert_eq!(cols.len(), 1);
    assert_eq!(cols[0].name, "Sci-Fi");
    assert_eq!(o.store.collection_books(cols[0].id), vec![a]);
    // a book that is not in the catalogue is skipped, not an error
    assert!(rep.journal_errors.is_empty(), "{:?}", rep.journal_errors);
}

// -- WebDAV in pieces ------------------------------------------------------------------------------------

fn noise(n: usize) -> Vec<u8> {
    let mut x = 0x2545f4914f6cdd1du64;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

const PIECE: u64 = 256 * 1024;
/// Two one-byte pieces at `create` and two more at the `check` of the run.
const PROBES: usize = 4;

fn store(d: &common::Dav) -> DavStore {
    DavStore::new(&d.url(), "u", "p", "backups")
        .unwrap()
        .with_chunking(1024 * 1024, PIECE)
}

fn repo_on(d: &common::Dav, pass: Option<&str>) -> Repository {
    Repository::open_or_create(
        Box::new(store(d)),
        &Secrets {
            passphrase: pass.map(str::to_string),
        },
        pass.is_some(),
        Kdf::TEST,
    )
    .unwrap()
}

fn partials(d: &common::Dav) -> Vec<(String, usize)> {
    d.files
        .lock()
        .unwrap()
        .iter()
        .filter(|(k, _)| k.contains(".part-"))
        .map(|(k, v)| (k.clone(), v.len()))
        .collect()
}

#[test]
fn a_big_upload_goes_in_pieces_and_continues_where_it_stopped() {
    let d = dav();
    let lib = Lib::new("dav-pieces");
    let big = noise(3 * 1024 * 1024 + 12345);
    lib.write("Ann Author/Huge/huge.bin", &big);
    let repo = repo_on(&d, None);

    // the connection breaks after four pieces of the big file
    d.knobs.fail_range_from.store(PROBES + 4, Ordering::Relaxed);
    assert!(run(&repo, &lib).is_err(), "the break fails the run");
    let held = partials(&d);
    assert_eq!(held.len(), 1, "{held:?}");
    assert!(held[0].0.ends_with(".part-resume"));
    assert_eq!(held[0].1 as u64, 4 * PIECE, "four whole pieces are kept");
    assert!(!has_lock_dav(&d), "the lock is released after a failure");

    // later: the same run sends only the rest
    d.knobs.fail_range_from.store(usize::MAX, Ordering::Relaxed);
    d.knobs.range_bytes.store(0, Ordering::Relaxed);
    let repo = repo_on(&d, None);
    let r = run(&repo, &lib).unwrap();
    assert!(r.verified && r.files == 1, "{r:?}");
    let sent = d.knobs.range_bytes.load(Ordering::Relaxed);
    let rest = big.len() - 4 * PIECE as usize;
    assert!(
        sent >= rest && sent < rest + 64 * 1024,
        "{sent} bytes sent, {rest} were missing"
    );
    assert!(partials(&d).is_empty(), "moved into place");
    // and the copy is right
    let to = tmp("dav-pieces-to").join("lib");
    repo.restore(None, &to, &RestoreOptions::default(), &|_| {}, &no())
        .unwrap();
    assert_eq!(tree(&lib.root), tree(&to));
    assert!(repo.verify(true, 2, &no(), &|_, _| {}).unwrap().ok());
}

fn has_lock_dav(d: &common::Dav) -> bool {
    d.files
        .lock()
        .unwrap()
        .keys()
        .any(|k| k.ends_with("lock.json"))
}

#[test]
fn a_server_that_ignores_ranges_gets_whole_uploads() {
    let d = dav();
    d.knobs.ranged.store(false, Ordering::Relaxed);
    let lib = Lib::new("dav-noranges");
    lib.write("Ann Author/Huge/huge.bin", &noise(2 * 1024 * 1024));
    let repo = repo_on(&d, None);
    let r = run(&repo, &lib).unwrap();
    assert_eq!(r.files, 1);
    // only the probe used ranges (and failed to prove anything)
    assert!(d.knobs.range_puts.load(Ordering::Relaxed) <= 2);
    let to = tmp("dav-noranges-to").join("lib");
    repo.restore(None, &to, &RestoreOptions::default(), &|_| {}, &no())
        .unwrap();
    assert_eq!(tree(&lib.root), tree(&to));
}

#[test]
fn an_encrypted_big_upload_starts_over_instead_of_mixing_two_encryptions() {
    let d = dav();
    let lib = Lib::new("dav-enc-pieces");
    let big = noise(2 * 1024 * 1024);
    lib.write("Ann Author/Huge/huge.bin", &big);
    let repo = repo_on(&d, Some("pw"));
    d.knobs.fail_range_from.store(PROBES + 3, Ordering::Relaxed);
    assert!(run(&repo, &lib).is_err());
    assert_eq!(partials(&d).len(), 1);
    d.knobs.fail_range_from.store(usize::MAX, Ordering::Relaxed);
    d.knobs.range_bytes.store(0, Ordering::Relaxed);
    let r = run(&repo, &lib).unwrap();
    assert!(r.verified);
    // a fresh sealing of everything: not a continuation of the other bytes
    assert!(d.knobs.range_bytes.load(Ordering::Relaxed) >= big.len());
    let to = tmp("dav-enc-pieces-to").join("lib");
    let again = Repository::open(Box::new(store(&d)), &Secrets::passphrase("pw")).unwrap();
    again
        .restore(None, &to, &RestoreOptions::default(), &|_| {}, &no())
        .unwrap();
    assert_eq!(tree(&lib.root), tree(&to));
}
