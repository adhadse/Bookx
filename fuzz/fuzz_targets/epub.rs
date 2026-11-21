//! A zip as an EPUB: archive limits, container, OPF, TOC.
#![cfg_attr(feature = "libfuzzer", no_main)]
use bookx_fuzz::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(arc) = bookx_core::archive::Archive::from_bytes(data.to_vec()) {
        let _ = bookx_core::book::OpenBook::from_archive(arc);
    }
});
