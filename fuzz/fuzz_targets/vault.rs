//! The encrypted password store file (`secrets.vault`): header, KDF settings, item table, MAC.
#![cfg_attr(feature = "libfuzzer", no_main)]
use bookx_fuzz::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // a hostile file must be refused cheaply: bounded sizes, bounded key derivation, no panic
    let _ = bookx_secrets::probe(data, "fuzz fuzz fuzz");
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = bookx_secrets::check_passphrase(s);
    }
});
