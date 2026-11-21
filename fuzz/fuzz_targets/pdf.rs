//! PDF files: open, page text, outline, links.
#![cfg_attr(feature = "libfuzzer", no_main)]
use bookx_fuzz::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = bookx_pdf::Document::open(data.to_vec(), None);
    let _ = bookx_formats::read_any("fuzz.pdf", data);
});
