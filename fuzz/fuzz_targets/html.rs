//! Chapter markup through the reader's parser (flood and nesting guards included).
#![cfg_attr(feature = "libfuzzer", no_main)]
use bookx_fuzz::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = bookx_core::html::parse_html(&text, "OEBPS/c.xhtml");
    let _ = bookx_core::util::parse_xml(&text);
});
