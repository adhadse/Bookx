//! `.bookxbackup` files (header, KDF settings, chunks) and repository manifests.
#![cfg_attr(feature = "libfuzzer", no_main)]
use bookx_fuzz::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = bookx_backup::crypto::describe(data);
    let _ = bookx_backup::push::open_bytes(data, None);
    if let Ok(m) = serde_json::from_slice::<bookx_backup::repo::manifest::Manifest>(data) {
        for f in &m.files {
            let _ = bookx_backup::repo::manifest::validate_rel(&f.path);
        }
    }
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = bookx_backup::repo::manifest::validate_rel(s);
    }
});
