//! Soak test in its own process (its own file) so `/proc/self` counts are not disturbed by other
//! tests running in parallel.

mod common;

use std::time::Duration;

use common::*;

fn fd_count() -> Option<usize> {
    Some(std::fs::read_dir("/proc/self/fd").ok()?.count())
}

fn thread_count() -> Option<usize> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    s.lines()
        .find_map(|l| l.strip_prefix("Threads:"))
        .and_then(|v| v.trim().parse().ok())
}

#[test]
fn two_thousand_requests_do_not_leak_threads_or_descriptors() {
    let (Some(fd0), Some(th0)) = (fd_count(), thread_count()) else {
        return;
    };
    let srv = start_fast(|l| l.max_connections_per_ip = 64);
    let addr = srv.addr();
    let auth = auth_header();
    let mut mid = None;
    let mut keep = Client::connect(addr);
    for i in 0..2000 {
        match i % 8 {
            0..=2 => {
                let r = once(
                    addr,
                    format!("GET /opds HTTP/1.1\r\nHost: x\r\n{auth}Connection: close\r\n\r\n")
                        .as_bytes(),
                )
                .expect("response");
                assert_eq!(r.status, 200, "request {i}");
            }
            3..=4 => {
                // keep-alive: reuse one connection
                assert_eq!(keep.get("/healthcheck", "").status, 200, "request {i}");
            }
            5 => {
                let r = once(addr, b"GET /\xff HTTP/1.1\r\n\r\n").expect("response");
                assert_eq!(r.status, 400);
            }
            6 => {
                // abandoned partial request
                let mut c = Client::connect(addr);
                c.send(b"GET /opds HTTP/1.1\r\nHo").unwrap();
            }
            _ => {
                let r = once(
                    addr,
                    format!("GET /{} HTTP/1.1\r\n\r\n", "a".repeat(9000)).as_bytes(),
                )
                .expect("response");
                assert_eq!(r.status, 414);
            }
        }
        if i == 1000 {
            assert!(eventually(Duration::from_secs(5), || srv
                .open_connections()
                <= 1));
            mid = Some((fd_count().unwrap(), thread_count().unwrap()));
        }
    }
    drop(keep);
    assert!(
        eventually(Duration::from_secs(5), || srv.open_connections() == 0),
        "open: {}",
        srv.open_connections()
    );
    let (fd_mid, th_mid) = mid.unwrap();
    let fd_end = fd_count().unwrap();
    let th_end = thread_count().unwrap();
    // nothing accumulates between request 1000 and 2000 (tolerance for the harness)
    assert!(fd_end <= fd_mid + 2, "fds grew: mid {fd_mid}, end {fd_end}");
    assert!(
        th_end <= th_mid + 1,
        "threads grew: mid {th_mid}, end {th_end}"
    );

    srv.shutdown();
    let fd1 = fd_count().unwrap();
    let th1 = thread_count().unwrap();
    assert!(fd1 <= fd0 + 1, "fds before {fd0}, after shutdown {fd1}");
    assert!(th1 <= th0 + 1, "threads before {th0}, after shutdown {th1}");
}
