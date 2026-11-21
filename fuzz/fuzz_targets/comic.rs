//! Page images of a comic: header checks and decoding with limits.
#![cfg_attr(feature = "libfuzzer", no_main)]
use bookx_fuzz::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = bookx_comic::decode::dimensions(data);
    let _ = bookx_comic::decode::is_bomb(data);
    let _ = bookx_comic::decode::decode_page(data, &bookx_comic::decode::Opts::default());
});
