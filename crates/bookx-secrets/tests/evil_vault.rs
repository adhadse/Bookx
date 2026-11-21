//! Hostile `secrets.vault` files: every one is refused quickly and without a panic.

use std::time::{Duration, Instant};

use bookx_secrets::{probe, FileStore, Kdf, OpenError};

fn real_file() -> Vec<u8> {
    let d = std::env::temp_dir().join(format!(
        "bookx-evil-vault-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    let s = FileStore::with_kdf(d.join("secrets.vault"), Kdf::TEST);
    s.create("a long master password").unwrap();
    bookx_secrets::SecretStore::set(&s, "k", "value").unwrap();
    let raw = std::fs::read(s.path()).unwrap();
    let _ = std::fs::remove_dir_all(&d);
    raw
}

#[test]
fn garbage_prefixes_and_oversized_input_are_refused_fast() {
    let start = Instant::now();
    let raw = real_file();
    assert_eq!(probe(&raw, "a long master password"), Ok(1));
    for n in 0..raw.len() {
        assert!(probe(&raw[..n], "a long master password").is_err());
    }
    assert!(
        probe(&vec![0xAB; 4 << 20], "x").is_err(),
        "4 MiB is over the limit"
    );
    assert!(probe(&[b'B'; 100], "x").is_err());
    assert!(start.elapsed() < Duration::from_secs(30));
}

#[test]
fn key_derivation_settings_cannot_be_abused() {
    let raw = real_file();
    let started = Instant::now();
    for (at, v) in [
        (9usize, u32::MAX),
        (13, u32::MAX),
        (17, u32::MAX),
        (9, 0),
        (13, 0),
        (17, 0),
    ] {
        let mut x = raw.clone();
        x[at..at + 4].copy_from_slice(&v.to_be_bytes());
        assert!(
            matches!(probe(&x, "x"), Err(OpenError::Unreadable(_))),
            "{at} {v}"
        );
    }
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "no key derivation was run"
    );
}

#[test]
fn a_count_that_lies_is_refused() {
    let raw = real_file();
    for count in [0u32, 2, 513, u32::MAX] {
        let mut x = raw.clone();
        x[45..49].copy_from_slice(&count.to_be_bytes());
        assert!(probe(&x, "a long master password").is_err(), "{count}");
    }
}
