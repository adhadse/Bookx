//! What Bookx reads back from helper programs it runs on untrusted files: the RAR listings of
//! `unrar` and 7-Zip, the page pictures (PNM) of `ddjvu`, and the cutting of the unpacked stream.
#![cfg_attr(feature = "libfuzzer", no_main)]
use bookx_fuzz::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&which, rest)) = data.split_first() else { return };
    let text = String::from_utf8_lossy(rest);
    match which % 4 {
        0 => {
            let _ = bookx_formats::rar::parse_unrar_list(&text);
        }
        1 => {
            let _ = bookx_formats::rar::parse_7z_list(&text);
        }
        2 => {
            let _ = bookx_formats::djvu::parse_pnm(rest);
        }
        _ => {
            let listed = bookx_formats::rar::parse_7z_list(&text);
            let _ = bookx_formats::rar::check_listing(&listed);
            let _ = bookx_formats::rar::split_stream(&listed, rest);
        }
    }
});
