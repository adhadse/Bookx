//! Hostile backups: `.bookxbackup` files, repository manifests and paths inside them. A restore
//! of an untrusted file is an error, never a write outside the target, never gigabytes of key
//! derivation. Fuzz twin: `fuzz/fuzz_targets/backup.rs`.

use std::time::{Duration, Instant};

use bookx_backup::crypto::{self, Kdf, Meta};
use bookx_backup::push;
use bookx_backup::repo::manifest::{validate_rel, Manifest};

fn meta() -> Meta {
    Meta {
        created: "2026-01-01T00:00:00Z".into(),
        books: 1,
        kind: "manual".into(),
        original_name: "x".into(),
    }
}

fn quick<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let r = f();
    assert!(
        t.elapsed() < Duration::from_secs(10),
        "{label}: {:?}",
        t.elapsed()
    );
    r
}

#[test]
fn hostile_encrypted_files_are_errors_and_quick() {
    let enc = crypto::encrypt_with(b"SQLite format 3\0 data", "pw", &meta(), Kdf::TEST).unwrap();
    let mut cases: Vec<(String, Vec<u8>)> = vec![
        ("empty".into(), vec![]),
        ("magic".into(), b"BOOKXBAK".to_vec()),
        ("short".into(), b"BOOKXBAK\x01".to_vec()),
        ("huge-header".into(), {
            let mut v = b"BOOKXBAK\x01".to_vec();
            v.extend(u32::MAX.to_be_bytes());
            v
        }),
        ("garbage-header".into(), {
            let mut v = b"BOOKXBAK\x01".to_vec();
            v.extend(5u32.to_be_bytes());
            v.extend(b"{{{{{");
            v
        }),
        ("zeros".into(), vec![0; 1 << 20]),
    ];
    for cut in (0..enc.len()).step_by(7) {
        cases.push((format!("cut-{cut}"), enc[..cut].to_vec()));
    }
    for i in (0..enc.len()).step_by(3) {
        let mut b = enc.clone();
        b[i] ^= 0x55;
        cases.push((format!("flip-{i}"), b));
    }
    // a header asking for absurd key derivation, in every field
    let hdr_len = u32::from_be_bytes(enc[9..13].try_into().unwrap()) as usize;
    let text = String::from_utf8(enc[13..13 + hdr_len].to_vec()).unwrap();
    for (from, to) in [
        ("\"memory_kib\":64", "\"memory_kib\":4294967295"),
        ("\"memory_kib\":64", "\"memory_kib\":524288"),
        ("\"passes\":1", "\"passes\":4294967295"),
        ("\"passes\":1", "\"passes\":11"),
        ("\"lanes\":1", "\"lanes\":0"),
        ("\"lanes\":1", "\"lanes\":9"),
        ("\"chunk\":65536", "\"chunk\":4294967296"),
    ] {
        let evil = text.replace(from, to);
        assert_ne!(evil, text, "{from} not found in {text}");
        let mut f = enc[..9].to_vec();
        f.extend((evil.len() as u32).to_be_bytes());
        f.extend(evil.as_bytes());
        f.extend(&enc[13 + hdr_len..]);
        cases.push((format!("kdf {to}"), f));
    }
    for (label, bytes) in &cases {
        let r = quick(label, || crypto::decrypt(bytes, "pw"));
        if label.starts_with("kdf ") || label.starts_with("cut-") || label.starts_with("flip-") {
            assert!(r.is_err(), "{label} must not decrypt");
        }
        let _ = quick(label, || push::open_bytes(bytes, Some("pw")));
        let _ = quick(label, || crypto::describe(bytes));
    }
    // a plain file that is not SQLite is not a backup
    assert!(push::open_bytes(b"not sqlite", None).is_err());
}

#[test]
fn hostile_manifest_paths_are_refused() {
    for bad in [
        "",
        "/etc/passwd",
        "../x",
        "a/../b",
        "a/./b",
        "a//b",
        "a/",
        "./a",
        "C:\\x",
        "C:/x",
        "a\\b",
        ".bookx/x",
        "a/.git/config",
        ".ssh/id_rsa",
        "a/.hidden",
        "a\0b",
        "a\nb",
        "CON",
        "dir/NUL.txt",
        "aux",
        "bookx-library.db",
        "bookx-library.json",
        &"a/".repeat(500),
        &"x".repeat(5000),
        &format!("a/{}", "y".repeat(1000)),
    ] {
        assert!(validate_rel(bad).is_err(), "{bad:?} must be refused");
    }
    for good in [
        "Author/Title (1)/book.epub",
        "a.epub",
        "ä/ö.epub",
        "dir/file name.pdf",
    ] {
        assert!(validate_rel(good).is_ok(), "{good:?}");
    }
}

#[test]
fn hostile_manifest_json_is_an_error() {
    let deep = format!("{}1{}", "[".repeat(100_000), "]".repeat(100_000));
    let h = "0".repeat(64);
    let files = |n: usize| {
        format!(
            r#"{{"format":1,"repo":"r","seq":1,"created":"2026-01-01T00:00:00Z","kind":"auto","app":"x","books":0,"catalogue":{{"blob":"{h}","size":1}},"state":{{"blob":"{h}","size":1}},"journal_seq":0,"include_books":true,"include_covers":true,"files":[{}]}}"#,
            (0..n)
                .map(|i| format!(
                    r#"{{"path":"b/{i}.epub","blob":"{h}","size":1,"mtime":0}}"#,
                    h = h
                ))
                .collect::<Vec<_>>()
                .join(","),
            h = h
        )
    };
    for (label, text) in [
        ("empty", String::new()),
        ("null", "null".into()),
        ("deep", deep),
        ("truncated", files(10)[..200].to_string()),
        (
            "huge-numbers",
            files(1).replace(
                "\"size\":1,\"mtime\":0",
                "\"size\":18446744073709551616,\"mtime\":-99999999999999999999",
            ),
        ),
        (
            "wrong-types",
            files(1).replace("\"seq\":1", "\"seq\":\"one\""),
        ),
    ] {
        let r = quick(label, || {
            serde_json::from_slice::<Manifest>(text.as_bytes())
        });
        assert!(r.is_err(), "{label} must not parse");
    }
    // a big but valid manifest parses in reasonable time and validates its paths one by one
    let big = files(200_000);
    let m: Manifest = quick("big", || serde_json::from_slice(big.as_bytes()).unwrap());
    assert_eq!(m.files.len(), 200_000);
    quick("validate", || {
        for f in &m.files {
            validate_rel(&f.path).unwrap();
        }
    });
}
