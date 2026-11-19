//! Offline mode blocks the real HTTP client before it sends anything. Own test binary: the switch
//! is process-wide, so it must not share a process with tests that expect the network.

use bookx_sources::fetch::{is_offline, set_offline, FetchError, Fetcher, UreqFetcher};

#[test]
fn offline_mode_refuses_without_sending() {
    assert!(!is_offline());
    set_offline(true);
    assert!(is_offline());
    // port 9 on localhost: if a request were sent it would be refused with a different error
    let err = UreqFetcher::new()
        .get("http://127.0.0.1:9/never", &[])
        .unwrap_err();
    match FetchError::of(&err) {
        Some(FetchError::Other(m)) => assert!(m.contains("Offline mode"), "{m}"),
        other => panic!("unexpected {other:?}"),
    }
    set_offline(false);
    assert!(!is_offline());
    let err = UreqFetcher::new()
        .get("http://127.0.0.1:9/never", &[])
        .unwrap_err();
    match FetchError::of(&err) {
        Some(FetchError::Other(m)) => assert!(!m.contains("Offline mode"), "{m}"),
        other => panic!("unexpected {other:?}"),
    }
}
