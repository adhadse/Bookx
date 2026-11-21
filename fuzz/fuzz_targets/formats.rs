//! Every input reader (FB2/FBZ, RTF, DOCX, ODT, HTMLZ, TXTZ, MOBI/AZW3, CBZ/CBT, PDF, ...). The
//! first byte picks the file name (extension), so the corpus reaches every reader.
#![cfg_attr(feature = "libfuzzer", no_main)]
use bookx_fuzz::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let exts = bookx_formats::supported_extensions();
    let Some((&first, rest)) = data.split_first() else { return };
    let ext = exts[first as usize % exts.len()];
    let _ = bookx_formats::read_any(&format!("fuzz.{ext}"), rest);
});
