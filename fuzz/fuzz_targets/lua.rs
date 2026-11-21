//! KOReader sidecar (`metadata.*.lua`) reader.
#![cfg_attr(feature = "libfuzzer", no_main)]
use bookx_fuzz::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = bookx_serve::lua::parse(&String::from_utf8_lossy(data));
});
