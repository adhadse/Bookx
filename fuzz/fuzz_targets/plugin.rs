//! Plugin manifests and WebAssembly modules. The first byte chooses which.
#![cfg_attr(feature = "libfuzzer", no_main)]
use bookx_fuzz::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&k, rest)) = data.split_first() else { return };
    if k & 1 == 0 {
        let _ = bookx_plugin::Manifest::parse(&String::from_utf8_lossy(rest));
        let _ = bookx_plugin::runtime::url_host(&String::from_utf8_lossy(rest));
    } else if let Ok(m) = bookx_plugin::Manifest::parse(
        "id = \"fuzz.p\"\nname = \"F\"\nversion = \"1.0.0\"\napi_version = 1\nkind = \"transform\"\n",
    ) {
        if let Ok(p) = bookx_plugin::Plugin::from_bytes(m, rest, bookx_plugin::Host::default()) {
            let _ = p.call_raw("x", b"{}");
        }
    }
});
