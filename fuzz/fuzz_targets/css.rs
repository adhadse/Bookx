//! Stylesheets, declaration lists and selectors.
#![cfg_attr(feature = "libfuzzer", no_main)]
use bookx_fuzz::fuzz_target;
use bookx_text::css::{parse_declaration_text, parse_selector_list, parse_stylesheet, MediaEnv, StyleSheetSet};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = parse_stylesheet(&text, MediaEnv { viewport_width: 600.0 }, 0);
    let _ = parse_declaration_text(&text);
    let _ = parse_selector_list(&text);
    let _ = StyleSheetSet::new().add_author(&text);
});
