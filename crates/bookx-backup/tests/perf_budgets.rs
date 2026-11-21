//! Performance budgets (release builds; debug gets 30x room, `BOOKX_PERF_SCALE` scales them).

mod common;

use std::io::{Read, Write};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bookx_backup::crypto::Kdf;
use bookx_backup::repo::manifest::{BlobRef, FileEntry, Manifest, Role, SnapName, FORMAT};
use bookx_backup::repo::seal::{Crypto, Keys};
use bookx_backup::repo::{MemStore, Repository, RunOptions, Secrets};
use chrono::{TimeZone, Utc};

fn budget(release_secs: f64) -> Duration {
    let scale: f64 = std::env::var("BOOKX_PERF_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let debug = if cfg!(debug_assertions) { 30.0 } else { 1.0 };
    Duration::from_secs_f64(release_secs * scale * debug)
}

fn no() -> AtomicBool {
    AtomicBool::new(false)
}

#[test]
fn sealing_and_opening_32_mib_streams_fast_and_in_constant_memory() {
    let c = Crypto::Keyed(Keys::from_master(&[3u8; 32]));
    let data = vec![0x5au8; 32 << 20];
    let t = Instant::now();
    let mut r = c
        .seal_reader("blobs/aa/x", std::io::Cursor::new(&data), data.len() as u64)
        .unwrap();
    let mut sealed = Vec::with_capacity(data.len() + (1 << 20));
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = r.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        sealed.extend_from_slice(&buf[..n]);
    }
    let mut out = std::io::sink();
    let mut w = c.open_writer("blobs/aa/x", &mut out, data.len() as u64);
    for ch in sealed.chunks(256 * 1024) {
        w.write_all(ch).unwrap();
    }
    assert_eq!(w.finish().unwrap(), data.len() as u64);
    let took = t.elapsed();
    assert!(took < budget(1.5), "seal + open of 32 MiB took {took:?}");
}

#[test]
fn an_unchanged_library_of_4000_files_is_a_quick_no_op() {
    let lib = common::Lib::new("perf-noop");
    for i in 0..4000 {
        let rel = format!("Author {}/Book {i}/book-{i}.epub", i % 50);
        lib.write(&rel, format!("content {i}").as_bytes());
        lib.age(&rel);
    }
    let m = Arc::new(MemStore::default());
    let repo = Repository::open_or_create(Box::new(m.clone()), &Secrets::none(), false, Kdf::TEST)
        .unwrap();
    let opts = RunOptions {
        jobs: 4,
        gc_grace: None,
        ..RunOptions::default()
    };
    let t = Instant::now();
    let first = repo.run(&lib.source(), &opts, &|_| {}, &no()).unwrap();
    let first_took = t.elapsed();
    assert_eq!(first.files, 4000);
    let t = Instant::now();
    let second = repo.run(&lib.source(), &opts, &|_| {}, &no()).unwrap();
    let took = t.elapsed();
    assert!(second.up_to_date);
    assert!(
        took < budget(1.5),
        "the no-op run took {took:?} (the first took {first_took:?})"
    );
    assert!(
        first_took < budget(8.0),
        "the first run of 4000 files took {first_took:?}"
    );
}

#[test]
fn a_manifest_of_200000_files_validates_and_round_trips() {
    let id = "ab".repeat(32);
    let at = Utc.with_ymd_and_hms(2026, 11, 21, 10, 0, 0).unwrap();
    let files: Vec<FileEntry> = (0..200_000)
        .map(|i| FileEntry {
            path: format!("Author {}/Book {i}/book {i}.epub", i % 500),
            blob: id.clone(),
            size: 1000 + i as u64,
            mtime: 1_700_000_000,
            role: Role::Book,
        })
        .collect();
    let m = Manifest {
        format: FORMAT,
        repo: "r".into(),
        seq: 1,
        created: at,
        kind: "auto".into(),
        app: "t".into(),
        books: 200_000,
        catalogue: BlobRef {
            blob: id.clone(),
            size: 1,
        },
        state: BlobRef { blob: id, size: 1 },
        journal_seq: 0,
        include_books: true,
        include_covers: true,
        too_big: vec![],
        files,
    };
    let name = SnapName {
        seq: 1,
        at,
        manual: false,
    };
    let t = Instant::now();
    let bytes = serde_json::to_vec(&m).unwrap();
    let back: Manifest = serde_json::from_slice(&bytes).unwrap();
    back.validate("r", &name).unwrap();
    let took = t.elapsed();
    assert!(took < budget(2.5), "a 200000-file manifest took {took:?}");
}

#[test]
fn a_passphrase_change_of_a_few_hundred_objects_and_big_files_is_quick() {
    let lib = common::Lib::new("perf-rekey");
    for i in 0..300 {
        let rel = format!("Author {}/Book {i}/book-{i}.epub", i % 20);
        lib.write(&rel, format!("content {i}").repeat(50).as_bytes());
    }
    for i in 0..4 {
        // not compressible, several chunks each
        let mut x = 0x9e3779b97f4a7c15u64 ^ i;
        let big: Vec<u8> = (0..8 << 20)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect();
        lib.write(&format!("Big/Big {i}/big-{i}.epub"), &big);
    }
    let m = Arc::new(MemStore::default());
    let repo = Repository::open_or_create(
        Box::new(m.clone()),
        &Secrets::passphrase("a"),
        true,
        Kdf::TEST,
    )
    .unwrap();
    repo.run(
        &lib.source(),
        &RunOptions {
            jobs: 4,
            gc_grace: None,
            ..RunOptions::default()
        },
        &|_| {},
        &no(),
    )
    .unwrap();
    drop(repo);
    let t = Instant::now();
    let r = Repository::change_passphrase(
        Box::new(m.clone()),
        "a",
        "b",
        &bookx_backup::repo::RekeyOptions {
            kdf: Kdf::TEST,
            verify: true,
            jobs: 4,
        },
        &no(),
        &|_| {},
    )
    .unwrap();
    let took = t.elapsed();
    assert!(r.converted > 300, "{r:?}");
    assert!(
        took < budget(3.0),
        "re-encrypting {} objects (32 MiB of big files) took {took:?}",
        r.converted
    );
}
