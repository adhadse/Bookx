//! The `/bookx` wire JSON: annotation requests and the KOSync routes through the real handler.
#![cfg_attr(feature = "libfuzzer", no_main)]
use bookx_fuzz::fuzz_target;
use bookx_serve::http::Request;
use bookx_serve::server::{handle, LibrarySnapshot, ServerConfig};
use std::sync::Arc;

fuzz_target!(|data: &[u8]| {
    let _ = bookx_serve::annotation_sync::parse_request(data, 1_800_000_000);
    let mut cfg = ServerConfig::new(Arc::new(LibrarySnapshot::default));
    cfg.kosync = Some(Arc::new(bookx_serve::kosync::KoSync::new(
        Box::new(bookx_serve::kosync::MemoryStore::new()),
        true,
    )));
    for path in ["/bookx/users/create", "/bookx/syncs/progress"] {
        let _ = handle(&cfg, &Request::new("POST", path).with_body(data));
    }
    // the first line, if text, as a request target and Host header
    if let Ok(s) = std::str::from_utf8(data) {
        let line = s.lines().next().unwrap_or("");
        let _ = bookx_serve::hostguard::host_allowed(line, &[]);
        let _ = handle(&cfg, &Request::new("GET", line).with_header("Host", line));
    }
});
