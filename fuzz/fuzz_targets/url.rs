//! The URL gates: what may be opened, what a host check accepts, which addresses are blocked.
#![cfg_attr(feature = "libfuzzer", no_main)]
use bookx_fuzz::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let s = String::from_utf8_lossy(data);
    if let Some(u) = bookx_core::util::safe_external_url(&s) {
        assert!(u.starts_with(|c: char| c.is_ascii_alphabetic()));
        let l = u.to_ascii_lowercase();
        assert!(l.starts_with("http://") || l.starts_with("https://") || l.starts_with("mailto:"));
        assert!(!u.chars().any(|c| c.is_control() || c.is_whitespace() || c == '\\'));
    }
    let _ = bookx_serve::hostguard::host_allowed(&s, &[]);
    let _ = bookx_serve::hostguard::origin_allowed(&s, Some("localhost:8080"), &[]);
    let _ = bookx_sources::netguard::host_of(&s);
    if let Ok(ip) = s.trim().parse::<std::net::IpAddr>() {
        let _ = bookx_sources::netguard::is_blocked_ip(ip);
    }
});
