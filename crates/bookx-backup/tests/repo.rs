//! End-to-end tests of the incremental repository: real libraries in temp folders, in-memory,
//! folder and WebDAV (in-process stand-in) places. No network beyond loopback.

mod common;

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bookx_backup::Cancelled;
use bookx_backup::crypto::Kdf;
use bookx_backup::repo::manifest::SnapName;
use bookx_backup::repo::{
    CatalogueAction, DavStore, FolderStore, MemStore, Phase, REPO_DIR, Report, Repository,
    RestoreMode, RestoreOptions, RestoreReport, RunOptions, Scope, Secrets,
};
use chrono::{Duration, TimeZone, Utc};
use common::{Lib, dav, tmp, tree};

fn no() -> AtomicBool {
    AtomicBool::new(false)
}

fn mem() -> Arc<MemStore> {
    Arc::new(MemStore::default())
}

fn open(m: &Arc<MemStore>, pass: Option<&str>) -> Repository {
    Repository::open_or_create(
        Box::new(m.clone()),
        &Secrets {
            passphrase: pass.map(str::to_string),
        },
        pass.is_some(),
        Kdf::TEST,
    )
    .unwrap()
}

fn run_with(repo: &Repository, lib: &Lib, f: impl FnOnce(&mut RunOptions)) -> Report {
    let mut o = RunOptions {
        jobs: 2,
        gc_grace: None,
        ..RunOptions::default()
    };
    f(&mut o);
    repo.run(&lib.source(), &o, &|_| {}, &no()).unwrap()
}

fn run(repo: &Repository, lib: &Lib) -> Report {
    run_with(repo, lib, |_| {})
}

fn restore(repo: &Repository, to: &Path, f: impl FnOnce(&mut RestoreOptions)) -> RestoreReport {
    let mut o = RestoreOptions::default();
    f(&mut o);
    repo.restore(None, to, &o, &|_| {}, &no()).unwrap()
}

fn blobs(m: &MemStore) -> Vec<String> {
    m.paths()
        .into_iter()
        .filter(|p| p.starts_with("blobs/"))
        .collect()
}

#[test]
fn the_first_run_copies_books_and_catalogue_and_the_second_has_nothing_to_do() {
    let mut lib = Lib::new("first");
    lib.add("One", 200);
    lib.add("Two", 300);
    lib.write("Ann Author/One/cover.jpg", b"jpeg-bytes");
    let m = mem();
    let repo = open(&m, None);
    let plan = repo
        .plan(&lib.source(), &Scope::default(), 2, &no(), &|_| {})
        .unwrap();
    assert!(plan.first_run && plan.estimated);
    assert_eq!(plan.new_files, 3);
    assert!(plan.enough_space);
    let r = run(&repo, &lib);
    assert_eq!(r.files, 3);
    assert_eq!(r.new_files, 3);
    assert!(r.catalogue_sent && r.verified && !r.up_to_date);
    assert!(r.snapshot.as_deref().unwrap().starts_with("s00000001-"));
    let puts = *m.puts.lock().unwrap();
    // nothing changed: no new snapshot, no uploads at all
    let again = run(&repo, &lib);
    assert!(again.up_to_date, "{again:?}");
    assert_eq!(again.new_files, 0);
    assert_eq!(
        *m.puts.lock().unwrap(),
        puts,
        "an unchanged library sends nothing"
    );
    assert_eq!(repo.list_snapshots().unwrap().len(), 1);
    // the plan now knows the place has everything
    let plan = repo
        .plan(&lib.source(), &Scope::default(), 2, &no(), &|_| {})
        .unwrap();
    assert_eq!(
        (plan.new_files, plan.reused_files, plan.first_run),
        (0, 3, false)
    );
}

#[test]
fn a_new_book_costs_only_its_own_blobs_and_a_rename_costs_nothing() {
    let mut lib = Lib::new("incr");
    let a = lib.add("Alpha", 400);
    lib.add("Beta", 400);
    let m = mem();
    let repo = open(&m, None);
    let first = run(&repo, &lib);
    let n_blobs = blobs(&m).len();
    lib.add("Gamma", 500);
    let second = run(&repo, &lib);
    assert_eq!(second.new_files, 1, "{second:?}");
    assert_eq!(second.reused_files, first.files);
    // the new blob, the changed catalogue and the changed notes state; never the old books
    assert!(
        blobs(&m).len() <= n_blobs + 3 && blobs(&m).len() >= n_blobs + 2,
        "{} vs {n_blobs}",
        blobs(&m).len()
    );
    assert!(second.new_bytes < first.new_bytes);
    // a rename: same content, new path
    let old = lib.book_file(a);
    let renamed = old.with_file_name("renamed.epub");
    std::fs::rename(&old, &renamed).unwrap();
    let before = blobs(&m).len();
    let third = run_with(&repo, &lib, |_| {});
    assert_eq!(third.new_files, 0, "{third:?}");
    assert_eq!(blobs(&m).len(), before, "a moved file adds no blob at all");
    assert!(
        !third.up_to_date && third.snapshot.is_some(),
        "the new path is a new snapshot"
    );
    let snaps = repo.list_snapshots().unwrap();
    let m3 = repo.manifest(&snaps[0].name).unwrap();
    assert!(m3.files.iter().any(|f| f.path.ends_with("renamed.epub")));
}

#[test]
fn restore_into_an_empty_folder_gives_the_same_books_notes_and_catalogue() {
    let mut lib = Lib::new("restore");
    let a = lib.add("Alpha", 300);
    lib.add("Beta", 300);
    lib.highlight(a, "remember this");
    let m = mem();
    let repo = open(&m, None);
    run(&repo, &lib);
    let to = tmp("restore-to").join("lib");
    let rep = restore(&repo, &to, |_| {});
    assert!(rep.failed.is_empty(), "{:?}", rep.failed);
    assert_eq!(rep.catalogue, CatalogueAction::Restored);
    assert_eq!(tree(&lib.root), tree(&to));
    let opened = bookx_store::open_library(&to).unwrap();
    assert_eq!(opened.library.books.len(), 2);
    let b = opened.library.get(a).unwrap();
    assert_eq!(b.highlights.len(), 1);
    assert_eq!(b.highlights[0].note, "remember this");
    // modification times come back
    let f = lib.book_file(a);
    let rel = f.strip_prefix(&lib.root).unwrap();
    let t0 = std::fs::metadata(&f).unwrap().modified().unwrap();
    let t1 = std::fs::metadata(to.join(rel)).unwrap().modified().unwrap();
    let d = t0
        .duration_since(t1)
        .or_else(|e| Ok::<_, ()>(e.duration()))
        .unwrap();
    assert!(d.as_secs() <= 1);
}

#[test]
fn highlights_made_after_the_last_snapshot_come_back_from_the_journal() {
    let mut lib = Lib::new("journal");
    let a = lib.add("Alpha", 100);
    let m = mem();
    let repo = open(&m, None);
    run_with(&repo, &lib, |o| {
        o.now = Utc.with_ymd_and_hms(2026, 11, 1, 10, 0, 0).unwrap()
    });
    lib.highlight(a, "first note");
    let t1 = Utc.with_ymd_and_hms(2026, 11, 1, 10, 5, 0).unwrap();
    let j1 = repo.journal_append(&lib.source(), t1, &no()).unwrap();
    assert_eq!((j1.seq, j1.books), (Some(1), 1));
    // nothing changed: no new record
    assert_eq!(
        repo.journal_append(&lib.source(), t1, &no()).unwrap().seq,
        None
    );
    lib.highlight(a, "second note");
    let t2 = Utc.with_ymd_and_hms(2026, 11, 1, 10, 10, 0).unwrap();
    assert_eq!(
        repo.journal_append(&lib.source(), t2, &no()).unwrap().seq,
        Some(2)
    );
    assert_eq!(repo.list_snapshots().unwrap().len(), 1, "no new snapshot");
    // records are small
    let sizes: Vec<u64> = repo.list_journal().unwrap().iter().map(|j| j.2).collect();
    assert!(sizes.iter().all(|s| *s < 4096), "{sizes:?}");

    let notes = |to: &Path| -> Vec<String> {
        let o = bookx_store::open_library(to).unwrap();
        o.library
            .get(a)
            .unwrap()
            .highlights
            .iter()
            .map(|h| h.note.clone())
            .collect()
    };
    let to = tmp("jr-to").join("lib");
    let rep = restore(&repo, &to, |_| {});
    assert_eq!(rep.journal_records, 2);
    assert_eq!(notes(&to), vec!["first note", "second note"]);
    // a point in time between the two records
    let to2 = tmp("jr-to2").join("lib");
    let rep = restore(&repo, &to2, |o| o.until = Some(t1 + Duration::minutes(1)));
    assert_eq!(rep.journal_records, 1);
    assert_eq!(notes(&to2), vec!["first note"]);
    // without replay: the snapshot as it was
    let to3 = tmp("jr-to3").join("lib");
    restore(&repo, &to3, |o| o.replay_journal = false);
    assert!(notes(&to3).is_empty());
}

#[test]
fn a_journal_that_a_snapshot_contains_is_compacted_away() {
    let mut lib = Lib::new("compact");
    let a = lib.add("Alpha", 100);
    let m = mem();
    let repo = open(&m, None);
    lib.highlight(a, "n1");
    repo.journal_append(&lib.source(), Utc::now(), &no())
        .unwrap();
    assert_eq!(repo.list_journal().unwrap().len(), 1);
    run(&repo, &lib); // the snapshot contains the note; prune compacts
    assert!(repo.list_journal().unwrap().is_empty());
    let to = tmp("cp-to").join("lib");
    restore(&repo, &to, |_| {});
    let o = bookx_store::open_library(&to).unwrap();
    assert_eq!(o.library.get(a).unwrap().highlights.len(), 1);
}

#[test]
fn encrypted_repositories_hide_names_and_content_and_need_the_passphrase() {
    let mut lib = Lib::new("enc");
    let a = lib.add("Secret Title", 200);
    let m = mem();
    let repo = open(&m, Some("correct horse"));
    run(&repo, &lib);
    let book = std::fs::read(lib.book_file(a)).unwrap();
    for (p, data) in m.files.lock().unwrap().iter() {
        if p == "README.txt" || p == "bookx-repo.json" {
            continue;
        }
        assert!(
            !data.windows(8).any(|w| w == &book[..8]),
            "{p} holds plain content"
        );
        assert!(
            !data.windows(6).any(|w| w == b"Secret"),
            "{p} holds plain text"
        );
    }
    let id_plain = bookx_backup::repo::seal::Crypto::Plain.id_of(&book);
    assert!(
        !m.paths().iter().any(|p| p.contains(&id_plain)),
        "ids are keyed, not SHA-256 of the file"
    );
    // wrong, missing passphrase
    for p in [Some("nope"), None] {
        let e = Repository::open(
            Box::new(m.clone()),
            &Secrets {
                passphrase: p.map(str::to_string),
            },
        )
        .err()
        .unwrap()
        .to_string();
        assert!(e.contains("passphrase"), "{e}");
    }
    // another passphrase means other ids for the same file
    let m2 = mem();
    let repo2 = open(&m2, Some("another one"));
    run(&repo2, &lib);
    let ids = |m: &MemStore| {
        blobs(m)
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
    };
    assert!(ids(&m).is_disjoint(&ids(&m2)));
    // restore works, and is verified
    let again =
        Repository::open(Box::new(m.clone()), &Secrets::passphrase("correct horse")).unwrap();
    let to = tmp("enc-to").join("lib");
    let rep = restore(&again, &to, |_| {});
    assert!(rep.failed.is_empty());
    assert_eq!(tree(&lib.root), tree(&to));
    // the deep check reads and verifies everything
    let v = again.verify(true, 2, &no(), &|_, _| {}).unwrap();
    assert!(v.ok(), "{v:?}");
    assert!(v.bytes_read > 0);
}

#[test]
fn tampered_truncated_and_missing_blobs_are_caught_for_both_kinds_of_place() {
    for pass in [None, Some("pw")] {
        let mut lib = Lib::new("tamper");
        let a = lib.add("Alpha", 400);
        lib.add("Beta", 400);
        let m = mem();
        let repo = open(&m, pass);
        run(&repo, &lib);
        let target = {
            let snap = repo
                .manifest(&repo.list_snapshots().unwrap()[0].name)
                .unwrap();
            let big = snap.files.iter().max_by_key(|f| f.size).unwrap();
            format!("blobs/{}/{}", &big.blob[..2], big.blob)
        };
        let original = m.files.lock().unwrap()[&target].clone();
        let rel_of_target = {
            let snap = repo
                .manifest(&repo.list_snapshots().unwrap()[0].name)
                .unwrap();
            let id = target.rsplit('/').next().unwrap();
            snap.files
                .iter()
                .find(|f| f.blob == id)
                .map(|f| f.path.clone())
        };
        // 1. flipped byte
        let mut bad = original.clone();
        let mid = bad.len() / 2;
        bad[mid] ^= 1;
        m.files.lock().unwrap().insert(target.clone(), bad);
        let to = tmp("tamper-to").join("lib");
        let rep = restore(&repo, &to, |_| {});
        assert!(
            !rep.failed.is_empty(),
            "pass={pass:?}: tampered blob restored"
        );
        if let Some(rel) = &rel_of_target {
            assert!(
                !to.join(rel).exists(),
                "a file that failed its check is not left behind"
            );
            assert!(rep.failed.iter().any(|f| &f.0 == rel));
        }
        let leftovers: Vec<_> = walk_names(&to)
            .into_iter()
            .filter(|n| n.contains("bookx-restore"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        let v = repo.verify(true, 2, &no(), &|_, _| {}).unwrap();
        assert!(
            !v.ok() && v.corrupt.iter().any(|c| target.ends_with(&c.0)),
            "{v:?}"
        );
        // 2. truncated
        m.files
            .lock()
            .unwrap()
            .insert(target.clone(), original[..original.len() / 2].to_vec());
        let to = tmp("trunc-to").join("lib");
        let rep = restore(&repo, &to, |_| {});
        assert!(
            !rep.failed.is_empty(),
            "truncated blob restored (pass={pass:?})"
        );
        let v = repo.verify(false, 2, &no(), &|_, _| {}).unwrap();
        assert!(!v.corrupt.is_empty(), "shallow verify sees the wrong size");
        // 3. missing
        m.files.lock().unwrap().remove(&target);
        let v = repo.verify(false, 2, &no(), &|_, _| {}).unwrap();
        assert_eq!(v.missing.len(), 1);
        // and a run puts it back
        let r = run_with(&repo, &lib, |o| o.force = true);
        assert_eq!(r.new_files, 1, "{r:?}");
        assert!(repo.verify(true, 2, &no(), &|_, _| {}).unwrap().ok());
        let _ = a;
    }
}

fn noise(n: usize) -> Vec<u8> {
    let mut x = 0x9e3779b97f4a7c15u64;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

fn walk_names(root: &Path) -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            v.push(e.file_name().to_string_lossy().to_string());
            if e.file_type().unwrap().is_dir() {
                v.extend(walk_names(&e.path()));
            }
        }
    }
    v
}

#[test]
fn a_hostile_manifest_cannot_write_outside_the_target() {
    let mut lib = Lib::new("hostile");
    lib.add("Alpha", 50);
    let m = mem();
    let repo = open(&m, None);
    run(&repo, &lib);
    let snap = repo.list_snapshots().unwrap().remove(0);
    let path = snap.name.path();
    let mut doc: serde_json::Value =
        serde_json::from_slice(&m.files.lock().unwrap()[&path]).unwrap();
    let blob = doc["files"][0]["blob"].clone();
    for evil in [
        "../../escape.txt",
        "/tmp/escape.txt",
        "a/../../escape",
        ".bookx/plugins/x.wasm",
        "C:/escape",
        "nul.epub",
    ] {
        let mut d = doc.clone();
        d["files"][0]["path"] = evil.into();
        d["files"][0]["blob"] = blob.clone();
        m.files
            .lock()
            .unwrap()
            .insert(path.clone(), serde_json::to_vec(&d).unwrap());
        let parent = tmp("hostile-to");
        let to = parent.join("lib");
        let e = repo
            .restore(None, &to, &RestoreOptions::default(), &|_| {}, &no())
            .err()
            .unwrap_or_else(|| panic!("{evil:?} was accepted"));
        assert!(
            e.to_string().contains("not valid") || format!("{e:#}").contains("unsafe path"),
            "{e:#}"
        );
        assert!(!parent.join("escape.txt").exists() && !parent.join("escape").exists());
    }
    // sizes beyond the limits
    doc["files"][0]["size"] = u64::MAX.into();
    m.files
        .lock()
        .unwrap()
        .insert(path.clone(), serde_json::to_vec(&doc).unwrap());
    assert!(
        repo.restore(
            None,
            &tmp("h2").join("l"),
            &RestoreOptions::default(),
            &|_| {},
            &no()
        )
        .is_err()
    );
    // a manifest with a link in the way: the target has a symlink where a folder should be
    #[cfg(unix)]
    {
        m.files.lock().unwrap().insert(
            path.clone(),
            serde_json::to_vec(&{
                let mut d =
                    serde_json::from_slice::<serde_json::Value>(&serde_json::to_vec(&doc).unwrap())
                        .unwrap();
                d["files"][0]["size"] = 10.into();
                d
            })
            .unwrap(),
        );
    }
}

#[cfg(unix)]
#[test]
fn a_link_in_the_target_is_never_followed_on_restore() {
    let mut lib = Lib::new("link");
    let a = lib.add("Alpha", 50);
    let m = mem();
    let repo = open(&m, None);
    run(&repo, &lib);
    let rel = lib
        .book_file(a)
        .strip_prefix(&lib.root)
        .unwrap()
        .to_path_buf();
    let parent = tmp("link-to");
    let outside = parent.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let to = parent.join("lib");
    std::fs::create_dir_all(&to).unwrap();
    // `Ann Author` is a link to a folder outside the target
    let top = rel.components().next().unwrap();
    std::os::unix::fs::symlink(&outside, to.join(top)).unwrap();
    let rep = restore(&repo, &to, |o| o.restore_catalogue = false);
    assert_eq!(rep.failed.len(), 1, "{:?}", rep.failed);
    assert!(
        std::fs::read_dir(&outside).unwrap().next().is_none(),
        "nothing written through the link"
    );
}

#[test]
fn an_old_snapshot_renamed_to_look_new_is_refused() {
    for pass in [None, Some("pw")] {
        let mut lib = Lib::new("replay");
        lib.add("Alpha", 50);
        let m = mem();
        let repo = open(&m, pass);
        run_with(&repo, &lib, |o| {
            o.now = Utc.with_ymd_and_hms(2026, 11, 1, 10, 0, 0).unwrap()
        });
        let old = repo.list_snapshots().unwrap().remove(0);
        lib.add("Beta", 50);
        run_with(&repo, &lib, |o| {
            o.now = Utc.with_ymd_and_hms(2026, 11, 2, 10, 0, 0).unwrap()
        });
        let new = repo.list_snapshots().unwrap().remove(0);
        assert_eq!(new.name.seq, 2);
        // the attacker puts the old manifest where the new one was
        let old_bytes = m.files.lock().unwrap()[&old.name.path()].clone();
        m.files.lock().unwrap().insert(new.name.path(), old_bytes);
        let e = repo.manifest(&new.name).unwrap_err();
        let text = format!("{e:#}");
        assert!(
            if pass.is_some() {
                text.contains("damaged")
            } else {
                text.contains("does not match its name")
            },
            "{text}"
        );
        let r = repo.restore(
            None,
            &tmp("replay-to").join("l"),
            &RestoreOptions::default(),
            &|_| {},
            &no(),
        );
        assert!(r.is_err(), "pass={pass:?}");
        // deleting the newest snapshot instead is a rollback: the next run notices
        m.files.lock().unwrap().remove(&new.name.path());
        let e = repo
            .run(
                &lib.source(),
                &RunOptions {
                    gc_grace: None,
                    ..RunOptions::default()
                },
                &|_| {},
                &no(),
            )
            .unwrap_err();
        assert!(
            e.downcast_ref::<bookx_backup::repo::RolledBack>().is_some(),
            "{e:#}"
        );
        let r = run_with(&repo, &lib, |o| o.accept_rollback = true);
        assert_eq!(r.seq, 3, "numbers are never reused: {r:?}");
        // and a restore that insists on at least the newest seen refuses the older one
        let e = repo
            .restore(
                None,
                &tmp("rb").join("l"),
                &RestoreOptions {
                    min_seq: Some(9),
                    ..RestoreOptions::default()
                },
                &|_| {},
                &no(),
            )
            .unwrap_err();
        assert!(e.downcast_ref::<bookx_backup::repo::RolledBack>().is_some());
    }
}

#[test]
fn cancel_leaves_no_snapshot_and_no_half_written_files() {
    let mut lib = Lib::new("cancel");
    for i in 0..6 {
        lib.add(&format!("Book {i}"), 4000);
    }
    let base = tmp("cancel-place");
    let place = base.join(REPO_DIR);
    let repo = Repository::open_or_create(
        Box::new(FolderStore::new(&place)),
        &Secrets::none(),
        false,
        Kdf::TEST,
    )
    .unwrap();
    let cancel = no();
    let hits = Mutex::new(0);
    let r = repo.run(
        &lib.source(),
        &RunOptions {
            jobs: 1,
            gc_grace: None,
            ..RunOptions::default()
        },
        &|p| {
            if p.phase == Phase::Uploading && p.bytes_done > 0 {
                *hits.lock().unwrap() += 1;
                cancel.store(true, Ordering::Relaxed);
            }
        },
        &cancel,
    );
    let e = r.unwrap_err();
    assert!(e.downcast_ref::<Cancelled>().is_some(), "{e:#}");
    assert!(repo.list_snapshots().unwrap().is_empty());
    let names = walk_names(&place);
    assert!(!names.iter().any(|n| n.contains(".part-")), "{names:?}");
    // every blob that is there is whole: a deep check of what exists finds nothing wrong
    for e in walk_names(&place.join("blobs")) {
        if e.len() == 64 {
            let p = place.join("blobs").join(&e[..2]).join(&e);
            assert_eq!(
                bookx_backup::repo::seal::Crypto::Plain.id_of(&std::fs::read(p).unwrap()),
                e
            );
        }
    }
    // resumable: the next run completes and reuses what was uploaded
    let r = run(&repo, &lib);
    assert_eq!(r.files, 6);
    assert!(r.reused_files >= 1, "{r:?}");
    assert!(repo.verify(true, 2, &no(), &|_, _| {}).unwrap().ok());
}

#[test]
fn a_failing_place_stops_the_run_with_no_snapshot_and_a_retry_resumes() {
    let mut lib = Lib::new("fail");
    for i in 0..5 {
        lib.add(&format!("Book {i}"), 100 + i * 10);
    }
    let m = mem();
    let repo = open(&m, None);
    *m.fail_after.lock().unwrap() = Some(*m.puts.lock().unwrap() + 3);
    let e = repo
        .run(
            &lib.source(),
            &RunOptions {
                jobs: 1,
                gc_grace: None,
                ..RunOptions::default()
            },
            &|_| {},
            &no(),
        )
        .unwrap_err();
    assert!(e.to_string().contains("refused"), "{e:#}");
    assert!(
        repo.list_snapshots().unwrap().is_empty(),
        "the manifest is written last"
    );
    *m.fail_after.lock().unwrap() = None;
    let r = run(&repo, &lib);
    assert_eq!(r.files, 5);
    assert!(
        r.new_files < 5,
        "what was sent before the failure is reused: {r:?}"
    );
    // a place that is unreachable fails with its own words and is isolated
    m.set_down(Some("the NAS is switched off"));
    let e = repo
        .run(&lib.source(), &RunOptions::default(), &|_| {}, &no())
        .unwrap_err();
    assert!(e.to_string().contains("NAS"));
}

#[test]
fn a_copy_that_reads_back_wrong_is_removed_and_the_run_fails() {
    let mut lib = Lib::new("corrupt");
    lib.add("Alpha", 100);
    let m = mem();
    let repo = open(&m, None);
    *m.corrupt.lock().unwrap() = true;
    let e = repo
        .run(
            &lib.source(),
            &RunOptions {
                jobs: 1,
                gc_grace: None,
                ..RunOptions::default()
            },
            &|_| {},
            &no(),
        )
        .unwrap_err();
    assert!(e.to_string().contains("does not match"), "{e:#}");
    assert!(blobs(&m).is_empty(), "the bad copy is deleted");
    assert!(repo.list_snapshots().unwrap().is_empty());
}

#[test]
fn not_enough_space_is_reported_before_anything_is_sent() {
    let mut lib = Lib::new("space");
    lib.add("Alpha", 20000);
    let m = mem();
    let repo = open(&m, None);
    run(&repo, &lib); // some blobs exist, so the plan counts rather than estimates
    lib.add("Beta", 20000);
    *m.free.lock().unwrap() = Some(1000);
    let plan = repo
        .plan(&lib.source(), &Scope::default(), 2, &no(), &|_| {})
        .unwrap();
    assert!(
        !plan.enough_space && plan.new_files == 1 && plan.upload_bytes > 1000,
        "{plan:?}"
    );
    let before = *m.puts.lock().unwrap();
    let e = repo
        .run(&lib.source(), &RunOptions::default(), &|_| {}, &no())
        .unwrap_err();
    assert!(e.to_string().contains("not enough space"), "{e:#}");
    assert_eq!(*m.puts.lock().unwrap(), before, "nothing was sent");
}

#[test]
fn scope_switches_and_the_size_cap() {
    let mut lib = Lib::new("scope");
    lib.add("Alpha", 100);
    lib.add("Big", 30000);
    lib.write("Ann Author/Alpha/cover.jpg", b"jpeg");
    lib.write(".bookx-trash/gone.epub", b"trash");
    lib.write("Ann Author/Alpha/half.epub.part", b"partial");
    let m = mem();
    let repo = open(&m, None);
    let r = run(&repo, &lib);
    assert_eq!(r.files, 3, "two books and a cover, no trash or partials");
    let m2 = mem();
    let repo2 = open(&m2, None);
    let r = run_with(&repo2, &lib, |o| o.scope.include_covers = false);
    assert_eq!(r.files, 2);
    let m3 = mem();
    let repo3 = open(&m3, None);
    let cap = std::fs::metadata(lib.book_file(2)).unwrap().len() - 1;
    let r = run_with(&repo3, &lib, |o| o.scope.max_file_size = Some(cap));
    assert_eq!(r.too_big.len(), 1, "{r:?}");
    assert_eq!(r.files, 2);
    let m4 = mem();
    let repo4 = open(&m4, None);
    let r = run_with(&repo4, &lib, |o| {
        o.scope.include_books = false;
        o.scope.include_covers = false;
    });
    assert_eq!(r.files, 0, "catalogue only");
    assert!(r.snapshot.is_some());
    let to = tmp("scope-to").join("lib");
    let rep = restore(&repo4, &to, |_| {});
    assert_eq!(rep.catalogue, CatalogueAction::Restored);
    assert!(tree(&to).is_empty());
}

#[test]
fn restore_modes_never_overwrite_silently_and_a_dry_run_changes_nothing() {
    let mut lib = Lib::new("modes");
    let a = lib.add("Alpha", 100);
    let b = lib.add("Beta", 100);
    let m = mem();
    let repo = open(&m, None);
    run(&repo, &lib);
    let to = tmp("modes-to").join("lib");
    restore(&repo, &to, |_| {});
    let fa = lib
        .book_file(a)
        .strip_prefix(&lib.root)
        .unwrap()
        .to_path_buf();
    let fb = lib
        .book_file(b)
        .strip_prefix(&lib.root)
        .unwrap()
        .to_path_buf();
    let orig_a = std::fs::read(to.join(&fa)).unwrap();
    std::fs::write(to.join(&fa), b"my newer edit").unwrap();
    std::fs::File::options()
        .write(true)
        .open(to.join(&fa))
        .unwrap()
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(3600))
        .unwrap();
    std::fs::remove_file(to.join(&fb)).unwrap();
    let before = tree(&to);
    // dry run, both modes
    for mode in [RestoreMode::MissingOnly, RestoreMode::ReplaceAll] {
        let rep = restore(&repo, &to, |o| {
            o.mode = mode;
            o.dry_run = true;
        });
        assert!(rep.dry_run && rep.files_written == 0);
        assert_eq!(
            rep.would_write,
            if mode == RestoreMode::MissingOnly {
                1
            } else {
                2
            }
        );
        assert_eq!(tree(&to), before, "dry run changed something");
    }
    // missing only: the edited file stays, the missing one comes back
    let rep = restore(&repo, &to, |_| {});
    assert_eq!((rep.files_written, rep.skipped_existing), (1, 1));
    assert_eq!(rep.catalogue, CatalogueAction::KeptExisting);
    assert_eq!(std::fs::read(to.join(&fa)).unwrap(), b"my newer edit");
    // replace all: replaced, and said to be newer
    let rep = restore(&repo, &to, |o| o.mode = RestoreMode::ReplaceAll);
    assert_eq!(rep.replaced_newer, vec![fa.to_string_lossy().to_string()]);
    assert_eq!(std::fs::read(to.join(&fa)).unwrap(), orig_a);
    assert_eq!(rep.skipped_same, 1);
    // the old catalogue was snapshotted first
    assert!(
        bookx_store::health::list_snapshots(&to)
            .iter()
            .any(|s| s.kind == bookx_store::health::SnapshotKind::PreRestore)
    );
}

#[test]
fn retention_never_prunes_manual_and_gc_is_two_phase() {
    let mut lib = Lib::new("gc");
    lib.add("Alpha", 100);
    let m = mem();
    let repo = open(&m, None);
    let t = |d: u32| Utc.with_ymd_and_hms(2026, 1, d, 10, 0, 0).unwrap();
    run_with(&repo, &lib, |o| {
        o.manual = true;
        o.now = t(1);
    });
    for d in 2..=5 {
        lib.add(&format!("B{d}"), 50 + d as usize);
        run_with(&repo, &lib, |o| {
            o.now = t(d);
            o.keep = 1;
        });
    }
    let far = Utc.with_ymd_and_hms(2027, 6, 1, 0, 0, 0).unwrap();
    let removed = repo.prune(1, far).unwrap();
    let snaps = repo.list_snapshots().unwrap();
    assert!(snaps.iter().any(|s| s.name.manual), "manual snapshot kept");
    assert_eq!(
        snaps.len(),
        2,
        "manual + the newest automatic; removed {removed}"
    );
    // blobs only older snapshots used are now unused: first pass notes them, nothing is deleted
    let before = blobs(&m).len();
    let cancel = no();
    let g1 = repo.gc(Duration::hours(24), far, &cancel).unwrap();
    assert_eq!(g1.deleted_blobs, 0);
    assert!(g1.newly_marked > 0, "{g1:?}");
    assert_eq!(blobs(&m).len(), before);
    // too early: still not
    let g2 = repo
        .gc(Duration::hours(24), far + Duration::hours(1), &cancel)
        .unwrap();
    assert_eq!(g2.deleted_blobs, 0);
    // after the grace period: deleted, and what the kept snapshots need is untouched
    let g3 = repo
        .gc(Duration::hours(24), far + Duration::hours(25), &cancel)
        .unwrap();
    assert!(g3.deleted_blobs > 0 && g3.deleted_bytes > 0, "{g3:?}");
    assert!(blobs(&m).len() < before);
    assert!(repo.verify(true, 2, &no(), &|_, _| {}).unwrap().ok());
    let to = tmp("gc-to").join("lib");
    let rep = restore(&repo, &to, |_| {});
    assert!(rep.failed.is_empty(), "{:?}", rep.failed);
}

#[test]
fn gc_refuses_to_delete_anything_when_a_snapshot_cannot_be_read() {
    let mut lib = Lib::new("gcsafe");
    lib.add("Alpha", 100);
    let m = mem();
    let repo = open(&m, None);
    run(&repo, &lib);
    lib.add("Beta", 100);
    run(&repo, &lib);
    let snaps = repo.list_snapshots().unwrap();
    m.files
        .lock()
        .unwrap()
        .insert(snaps[1].name.path(), b"{ not json".to_vec());
    // an orphan blob exists too
    m.files
        .lock()
        .unwrap()
        .insert(format!("blobs/ab/{}", "ab".repeat(32)), b"x".to_vec());
    let n = m.paths().len();
    let far = Utc::now() + Duration::days(30);
    for _ in 0..2 {
        assert!(repo.gc(Duration::zero(), far, &no()).is_err());
    }
    assert_eq!(m.paths().len(), n);
}

#[test]
fn leftover_temporary_objects_are_cleaned_by_gc() {
    let mut lib = Lib::new("tmpclean");
    lib.add("Alpha", 100);
    let m = mem();
    let repo = open(&m, None);
    run(&repo, &lib);
    m.files
        .lock()
        .unwrap()
        .insert("blobs/ab/.xyz.part-1234".into(), b"half".to_vec());
    let g = repo.gc(Duration::hours(1), Utc::now(), &no()).unwrap();
    assert_eq!(g.temp_removed, 1);
    assert!(!m.paths().iter().any(|p| p.contains(".part-")));
}

#[test]
fn the_old_whole_catalogue_backups_still_list_and_restore_next_to_the_repository() {
    use bookx_backup::{FolderTarget, list_backups};
    let mut lib = Lib::new("legacy");
    lib.add("Alpha", 100);
    let place = tmp("legacy-place");
    // an old-style backup of the catalogue
    let snap = bookx_store::health::take_snapshot(
        &lib.opened.store,
        &lib.root,
        bookx_store::health::SnapshotKind::Manual,
        Utc::now(),
    )
    .unwrap();
    let t = FolderTarget { dir: place.clone() };
    let opts = bookx_backup::PushOptions {
        passphrase: None,
        keep: 5,
        now: Utc::now(),
        kdf: Kdf::TEST,
    };
    bookx_backup::push(&t, &snap, &opts, &no(), &|_| {}).unwrap();
    // the repository in the same place
    let repo = Repository::open_or_create(
        Box::new(FolderStore::new(place.join(REPO_DIR))),
        &Secrets::none(),
        false,
        Kdf::TEST,
    )
    .unwrap();
    run(&repo, &lib);
    let old = list_backups(&t).unwrap();
    assert_eq!(
        old.len(),
        1,
        "the repository folder is not mistaken for a backup file"
    );
    let out = bookx_backup::fetch_to_file(&t, &old[0].0.name, None, &tmp("legacy-out")).unwrap();
    assert!(out.exists());
    assert_eq!(repo.list_snapshots().unwrap().len(), 1);
    assert!(Repository::exists(&FolderStore::new(place.join(REPO_DIR))).unwrap());
}

#[test]
fn a_repository_in_a_real_folder_has_a_readme_plain_blobs_and_readable_manifests() {
    let mut lib = Lib::new("folder");
    let a = lib.add("Alpha", 100);
    let base = tmp("folder-place");
    let place = base.join(REPO_DIR);
    let repo = Repository::open_or_create(
        Box::new(FolderStore::new(&place)),
        &Secrets::none(),
        false,
        Kdf::TEST,
    )
    .unwrap();
    run(&repo, &lib);
    let readme = std::fs::read_to_string(place.join("README.txt")).unwrap();
    assert!(readme.contains("NOT encrypted") && readme.contains("blobs/"));
    // a plain blob is the original file
    let book = std::fs::read(lib.book_file(a)).unwrap();
    let id = bookx_backup::repo::seal::Crypto::Plain.id_of(&book);
    assert_eq!(
        std::fs::read(place.join("blobs").join(&id[..2]).join(&id)).unwrap(),
        book
    );
    let snap = repo.list_snapshots().unwrap().remove(0);
    let text = std::fs::read_to_string(place.join("snapshots").join(snap.file_name())).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["files"][0]["blob"], id);
    // reopen from disk
    let again = Repository::open(Box::new(FolderStore::new(&place)), &Secrets::none()).unwrap();
    assert_eq!(again.id(), repo.id());
    // an encrypted request against a plain repository is an error, not a silent downgrade
    let e = Repository::open_or_create(
        Box::new(FolderStore::new(&place)),
        &Secrets::passphrase("x"),
        true,
        Kdf::TEST,
    )
    .err()
    .unwrap();
    assert!(e.to_string().contains("unencrypted"), "{e}");
    let _ = SnapName::parse("x");
}

#[test]
fn webdav_runs_with_move_and_without_and_cancels_mid_file() {
    for allow_move in [true, false] {
        let d = dav();
        d.allow_move.store(allow_move, Ordering::Relaxed);
        let mut lib = Lib::new("dav");
        let a = lib.add("Alpha", 3000);
        lib.add("Beta", 3000);
        let store = || DavStore::new(&d.url(), "u", "p", "backups").unwrap();
        let repo = Repository::open_or_create(
            Box::new(store()),
            &Secrets::passphrase("pw"),
            true,
            Kdf::TEST,
        )
        .unwrap();
        let r = run(&repo, &lib);
        assert_eq!(r.files, 2);
        let log = d.log.lock().unwrap().clone();
        assert_eq!(
            log.iter()
                .any(|l| l.starts_with("MOVE ") && l.contains("/blobs/")),
            allow_move,
            "{allow_move}"
        );
        assert!(
            !d.files.lock().unwrap().keys().any(|k| k.contains(".part-")),
            "no temporary names left"
        );
        // nothing the server holds is readable
        let book = std::fs::read(lib.book_file(a)).unwrap();
        assert!(
            d.files
                .lock()
                .unwrap()
                .values()
                .all(|v| !v.windows(8).any(|w| w == &book[..8]))
        );
        // second run: no uploads
        let blob_puts = |d: &common::Dav| {
            d.log
                .lock()
                .unwrap()
                .iter()
                .filter(|l| l.starts_with("PUT ") && l.contains("/blobs/"))
                .count()
        };
        let puts = blob_puts(&d);
        assert!(run(&repo, &lib).up_to_date);
        assert_eq!(blob_puts(&d), puts);
        // restore through a fresh connection
        let again = Repository::open(Box::new(store()), &Secrets::passphrase("pw")).unwrap();
        let to = tmp("dav-to").join("lib");
        let rep = restore(&again, &to, |_| {});
        assert!(rep.failed.is_empty(), "{:?}", rep.failed);
        assert_eq!(tree(&lib.root), tree(&to));
        assert!(again.verify(true, 2, &no(), &|_, _| {}).unwrap().ok());
        // cancel in the middle of a new big file
        lib.write("Ann Author/Huge/huge.bin", &noise(4 << 20));
        let cancel = no();
        let files_before = d.files.lock().unwrap().len();
        let r = repo.run(
            &lib.source(),
            &RunOptions {
                jobs: 1,
                gc_grace: None,
                ..RunOptions::default()
            },
            &|p| {
                if p.phase == Phase::Uploading && p.bytes_done > 100_000 {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
            &cancel,
        );
        assert!(r.unwrap_err().downcast_ref::<Cancelled>().is_some());
        let keys: Vec<String> = d.files.lock().unwrap().keys().cloned().collect();
        assert!(!keys.iter().any(|k| k.contains(".part-")), "{keys:?}");
        assert_eq!(repo.list_snapshots().unwrap().len(), 1);
        assert!(d.files.lock().unwrap().len() <= files_before + 1);
        // wrong passphrase through the network
        assert!(Repository::open(Box::new(store()), &Secrets::passphrase("bad")).is_err());
    }
}

#[test]
fn hostile_repository_settings_are_refused_without_doing_the_work() {
    let m = mem();
    let lib = Lib::new("cfg");
    drop(lib);
    let repo = open(&m, Some("pw"));
    let mut cfg: serde_json::Value =
        serde_json::from_slice(&m.files.lock().unwrap()["bookx-repo.json"]).unwrap();
    for (k, v) in [
        ("memory_kib", 4_000_000_000u64),
        ("passes", 1_000_000),
        ("lanes", 0),
    ] {
        let mut c = cfg.clone();
        c["kdf"][k] = v.into();
        m.files
            .lock()
            .unwrap()
            .insert("bookx-repo.json".into(), serde_json::to_vec(&c).unwrap());
        let t = std::time::Instant::now();
        let e = Repository::open(Box::new(m.clone()), &Secrets::passphrase("pw"))
            .err()
            .unwrap()
            .to_string();
        assert!(e.contains("unreasonable"), "{k}: {e}");
        assert!(t.elapsed() < std::time::Duration::from_secs(2));
    }
    // a repository from the future, and nonsense
    cfg["format"] = 99.into();
    m.files
        .lock()
        .unwrap()
        .insert("bookx-repo.json".into(), serde_json::to_vec(&cfg).unwrap());
    assert!(
        Repository::open(Box::new(m.clone()), &Secrets::passphrase("pw"))
            .err()
            .unwrap()
            .to_string()
            .contains("cannot read")
    );
    m.files
        .lock()
        .unwrap()
        .insert("bookx-repo.json".into(), b"not json".to_vec());
    assert!(Repository::open(Box::new(m.clone()), &Secrets::passphrase("pw")).is_err());
    // an oversized settings file is not read into memory
    m.files
        .lock()
        .unwrap()
        .insert("bookx-repo.json".into(), vec![b' '; 200_000]);
    assert!(Repository::open(Box::new(m.clone()), &Secrets::passphrase("pw")).is_err());
    drop(repo);
    // no repository at all
    let e = Repository::open(Box::new(mem()), &Secrets::none())
        .err()
        .unwrap();
    assert!(
        e.downcast_ref::<bookx_backup::repo::NoRepository>()
            .is_some()
    );
}

#[test]
fn a_blob_bigger_than_the_snapshot_says_is_not_read_to_the_end() {
    for pass in [None, Some("pw")] {
        let mut lib = Lib::new("oversize");
        lib.add("Alpha", 100);
        let m = mem();
        let repo = open(&m, pass);
        run(&repo, &lib);
        let snap = repo
            .manifest(&repo.list_snapshots().unwrap()[0].name)
            .unwrap();
        let big = &snap.files[0];
        let path = format!("blobs/{}/{}", &big.blob[..2], big.blob);
        m.files.lock().unwrap().insert(path, vec![7u8; 5 << 20]);
        let rep = restore(&repo, &tmp("oversize-to").join("lib"), |o| {
            o.restore_catalogue = false
        });
        assert_eq!(rep.failed.len(), 1, "{pass:?}");
        assert!(
            rep.failed[0].1.contains("bigger")
                || rep.failed[0].1.contains("damaged")
                || rep.failed[0].1.contains("checksum"),
            "{:?}",
            rep.failed
        );
    }
}

#[test]
fn a_damaged_catalogue_stops_the_restore_and_leaves_the_target_without_one() {
    let mut lib = Lib::new("badcat");
    lib.add("Alpha", 100);
    let m = mem();
    let repo = open(&m, None);
    run(&repo, &lib);
    let snap = repo
        .manifest(&repo.list_snapshots().unwrap()[0].name)
        .unwrap();
    let id = &snap.catalogue.blob;
    let path = format!("blobs/{}/{}", &id[..2], id);
    let mut bad = m.files.lock().unwrap()[&path].clone();
    bad[100] ^= 0xff;
    m.files.lock().unwrap().insert(path, bad);
    let to = tmp("badcat-to").join("lib");
    let e = repo
        .restore(None, &to, &RestoreOptions::default(), &|_| {}, &no())
        .unwrap_err();
    assert!(
        format!("{e:#}").contains("checksum") || format!("{e:#}").contains("damaged"),
        "{e:#}"
    );
    assert!(!to.join("bookx-library.db").exists());
}

#[test]
fn restore_limits_are_enforced_before_writing() {
    let mut lib = Lib::new("limits");
    lib.add("Alpha", 100);
    lib.add("Beta", 100);
    let m = mem();
    let repo = open(&m, None);
    run(&repo, &lib);
    let to = tmp("limits-to").join("lib");
    let e = repo
        .restore(
            None,
            &to,
            &RestoreOptions {
                max_files: 1,
                ..RestoreOptions::default()
            },
            &|_| {},
            &no(),
        )
        .unwrap_err();
    assert!(e.to_string().contains("limits"));
    let e = repo
        .restore(
            None,
            &to,
            &RestoreOptions {
                max_bytes: 10,
                ..RestoreOptions::default()
            },
            &|_| {},
            &no(),
        )
        .unwrap_err();
    assert!(e.to_string().contains("limits"));
    assert!(!to.exists());
}

#[test]
fn a_book_that_changes_while_it_is_copied_is_left_out_not_stored_wrong() {
    let mut lib = Lib::new("changing");
    let a = lib.add("Alpha", 100);
    let m = mem();
    let repo = open(&m, None);
    // the file is hashed, then replaced by a different one of another size before the upload
    let src = lib.source();
    let f = lib.book_file(a);
    let hashed = std::sync::atomic::AtomicBool::new(false);
    let r = repo
        .run(
            &src,
            &RunOptions {
                jobs: 1,
                gc_grace: None,
                ..RunOptions::default()
            },
            &|p| {
                if p.phase == Phase::Uploading && !hashed.swap(true, Ordering::Relaxed) {
                    std::fs::write(&f, b"something else entirely").unwrap();
                }
            },
            &no(),
        )
        .unwrap();
    assert_eq!(r.skipped.len(), 1, "{r:?}");
    assert!(r.skipped[0].reason.contains("changed"), "{:?}", r.skipped);
    assert_eq!(r.files, 0);
    assert!(repo.verify(true, 1, &no(), &|_, _| {}).unwrap().ok());
}

#[test]
fn a_backup_place_inside_the_library_is_not_backed_up_into_itself() {
    let mut lib = Lib::new("inside");
    lib.add("Alpha", 100);
    std::fs::create_dir_all(lib.root.join("My Backups")).unwrap();
    let place = lib.root.join("My Backups").join(REPO_DIR);
    let repo = Repository::open_or_create(
        Box::new(FolderStore::new(&place)),
        &Secrets::none(),
        false,
        Kdf::TEST,
    )
    .unwrap();
    let r = run(&repo, &lib);
    assert_eq!(r.files, 1, "{r:?}");
    let again = run(&repo, &lib);
    assert!(
        again.up_to_date,
        "the repository growing does not make the library look changed: {again:?}"
    );
}

#[cfg(unix)]
#[test]
fn unchanged_settled_files_are_not_read_again() {
    use std::os::unix::fs::PermissionsExt;
    let lib = Lib::new("cache");
    lib.write("Ann/One/one.epub", b"one one one");
    lib.age("Ann/One/one.epub");
    lib.write("Ann/Two/two.epub", b"two"); // brand new: not trusted by the cache yet
    let m = mem();
    let repo = open(&m, None);
    run(&repo, &lib);
    let p = lib.root.join("Ann/One/one.epub");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::File::open(&p).is_ok() {
        return; // running as root: permissions do not stop us, nothing to prove
    }
    let r = run(&repo, &lib);
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(r.up_to_date, "the settled file came from the cache: {r:?}");
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
}

#[cfg(unix)]
#[test]
fn an_unreadable_file_is_left_out_and_reported_not_fatal() {
    use std::os::unix::fs::PermissionsExt;
    let lib = Lib::new("unreadable");
    lib.write("Ann/One/one.epub", b"readable");
    lib.write("Ann/Two/two.epub", b"secret");
    let p = lib.root.join("Ann/Two/two.epub");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::File::open(&p).is_ok() {
        return;
    }
    let m = mem();
    let repo = open(&m, None);
    let r = run(&repo, &lib);
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(r.files, 1);
    assert_eq!(r.skipped.len(), 1, "{r:?}");
    assert!(r.skipped[0].path.ends_with("two.epub") && r.skipped[0].reason.contains("cannot read"));
    assert!(r.snapshot.is_some());
}
