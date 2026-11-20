//! Adversarial tests for the HTTP transport (`bookx_serve::net`) over real sockets on port 0.
//! Timeouts are shortened through `Limits` so every test finishes in about a second.

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;

const MS: fn(u64) -> Duration = Duration::from_millis;

fn alive(addr: std::net::SocketAddr) -> bool {
    once(
        addr,
        b"GET /healthcheck HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    )
    .is_some_and(|r| r.status == 200)
}

// ------------------------------------------------------------------------------------- limits

#[test]
fn request_line_limit_is_exact() {
    let srv = start_fast(|_| {});
    let auth = auth_header();
    let line = |n: usize| {
        format!(
            "GET /{} HTTP/1.1\r\nHost: x\r\n{auth}Connection: close\r\n\r\n",
            "a".repeat(n)
        )
    };
    // "GET /" + n + " HTTP/1.1" == 8192 bytes
    let r = once(srv.addr(), line(8192 - 5 - 9).as_bytes()).unwrap();
    assert_eq!(r.status, 404);
    let r = once(srv.addr(), line(8192 - 5 - 9 + 1).as_bytes()).unwrap();
    assert_eq!(r.status, 414);
    assert_eq!(r.header("connection"), Some("close"));
    assert!(alive(srv.addr()));
}

#[test]
fn request_line_without_any_newline_is_cut_off_at_the_limit() {
    let srv = start_fast(|l| l.header_timeout = Duration::from_secs(5));
    let mut c = Client::connect(srv.addr());
    let started = Instant::now();
    // never a CRLF: the server must answer 414 as soon as the limit is crossed, not wait
    c.send(format!("GET /{}", "a".repeat(9000)).as_bytes())
        .unwrap();
    let r = c.read_response(false).expect("414");
    assert_eq!(r.status, 414);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn header_block_limit() {
    let srv = start_fast(|_| {});
    let auth = auth_header();
    // 12 x ~1 KiB headers: under 16 KiB
    let mut raw = format!("GET /opds HTTP/1.1\r\nHost: x\r\n{auth}");
    for i in 0..12 {
        raw.push_str(&format!("X-{i}: {}\r\n", "v".repeat(1000)));
    }
    raw.push_str("Connection: close\r\n\r\n");
    assert_eq!(once(srv.addr(), raw.as_bytes()).unwrap().status, 200);

    // 20 of them: over 16 KiB
    let mut raw = format!("GET /opds HTTP/1.1\r\nHost: x\r\n{auth}");
    for i in 0..20 {
        raw.push_str(&format!("X-{i}: {}\r\n", "v".repeat(1000)));
    }
    raw.push_str("Connection: close\r\n\r\n");
    let r = once(srv.addr(), raw.as_bytes()).unwrap();
    assert_eq!(r.status, 431);

    // one single huge header
    let raw = format!(
        "GET /opds HTTP/1.1\r\nHost: x\r\nX-Big: {}\r\n\r\n",
        "A".repeat(1024 * 1024)
    );
    let mut c = Client::connect(srv.addr());
    let _ = c.send(raw.as_bytes());
    let r = c.read_response(false).expect("431");
    assert_eq!(r.status, 431);
    assert!(alive(srv.addr()));
}

#[test]
fn header_block_limit_boundary() {
    // head (incl. the blank line) of exactly max_header_bytes passes, one more byte fails
    let srv = start_fast(|_| {});
    let auth = auth_header();
    let build = |pad: usize| {
        format!(
            "GET /opds HTTP/1.1\r\nHost: x\r\n{auth}X-Pad: {}\r\nConnection: close\r\n\r\n",
            "p".repeat(pad)
        )
    };
    let base = build(0).len();
    let r = once(srv.addr(), build(16384 - base).as_bytes()).unwrap();
    assert_eq!(r.status, 200);
    let r = once(srv.addr(), build(16384 - base + 1).as_bytes()).unwrap();
    assert_eq!(r.status, 431);
}

#[test]
fn header_count_limit() {
    let srv = start_fast(|_| {});
    let auth = auth_header();
    // the auth line and Connection are headers too: Host + Authorization + Connection + n
    let build = |n: usize| {
        let mut raw = format!("GET /opds HTTP/1.1\r\nHost: x\r\n{auth}Connection: close\r\n");
        for i in 0..n {
            raw.push_str(&format!("X{i}: v\r\n"));
        }
        raw.push_str("\r\n");
        raw
    };
    assert_eq!(once(srv.addr(), build(97).as_bytes()).unwrap().status, 200); // 100 in total
    assert_eq!(once(srv.addr(), build(98).as_bytes()).unwrap().status, 431); // 101
    assert_eq!(
        once(srv.addr(), build(3000).as_bytes()).unwrap().status,
        431
    );
}

#[test]
fn limits_are_configurable() {
    let srv = start_fast(|l| {
        l.max_request_line = 40;
        l.max_headers = 2;
    });
    let r = once(
        srv.addr(),
        b"GET /healthcheck HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    )
    .unwrap();
    assert_eq!(r.status, 200);
    let r = once(
        srv.addr(),
        format!("GET /{} HTTP/1.1\r\n\r\n", "a".repeat(50)).as_bytes(),
    )
    .unwrap();
    assert_eq!(r.status, 414);
    let r = once(
        srv.addr(),
        b"GET /healthcheck HTTP/1.1\r\nA: 1\r\nB: 2\r\nC: 3\r\n\r\n",
    )
    .unwrap();
    assert_eq!(r.status, 431);
}

// ---------------------------------------------------------------------------------- timeouts

#[test]
fn slowloris_header_drip_times_out_and_frees_the_worker() {
    let srv = start_fast(|_| {});
    let mut c = Client::connect(srv.addr());
    c.set_timeout(MS(60));
    c.send(b"GET /opds HTTP/1.1\r\n").unwrap();
    let started = Instant::now();
    let mut reply = None;
    // one byte every ~60 ms, a header that never ends
    for i in 0..60 {
        if c.send(if i == 0 { b"X" } else { b"a" }).is_err() {
            break;
        }
        if let Some(r) = c.read_response(false) {
            reply = Some(r);
            break;
        }
    }
    let r = reply.expect("408 before the drip ends");
    assert_eq!(r.status, 408);
    assert!(
        started.elapsed() < Duration::from_millis(2000),
        "took {:?}",
        started.elapsed()
    );
    assert!(c.is_closed(Duration::from_secs(3)));
    assert!(
        eventually(Duration::from_secs(3), || srv.open_connections() == 0),
        "the connection slot must be released"
    );
    assert!(alive(srv.addr()));
}

#[test]
fn silent_connection_is_closed_without_a_reply() {
    let srv = start_fast(|_| {});
    let mut c = Client::connect(srv.addr());
    assert!(c.is_closed(Duration::from_secs(3)));
    assert!(eventually(Duration::from_secs(2), || srv
        .open_connections()
        == 0));
}

#[test]
fn stalled_body_gets_408() {
    let srv = start_fast(|_| {});
    let mut c = Client::connect(srv.addr());
    c.send(b"POST /bookx/users/create HTTP/1.1\r\nHost: x\r\nContent-Length: 1000\r\n\r\n{\"user")
        .unwrap();
    let started = Instant::now();
    let r = c.read_response(false).expect("408");
    assert_eq!(r.status, 408);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(c.is_closed(Duration::from_secs(3)));
    assert!(eventually(Duration::from_secs(3), || srv
        .open_connections()
        == 0));
}

#[test]
fn dripped_never_ending_body_hits_the_total_deadline() {
    let srv = start_fast(|_| {});
    let mut c = Client::connect(srv.addr());
    c.set_timeout(MS(50));
    c.send(b"PUT /bookx/syncs/progress HTTP/1.1\r\nHost: x\r\nContent-Length: 1000000\r\n\r\n")
        .unwrap();
    let started = Instant::now();
    let mut reply = None;
    for _ in 0..100 {
        // a byte well inside each per-read timeout: only a total deadline can stop this
        if c.send(b"x").is_err() {
            break;
        }
        if let Some(r) = c.read_response(false) {
            reply = Some(r);
            break;
        }
    }
    assert_eq!(reply.expect("408").status, 408);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(eventually(Duration::from_secs(3), || srv
        .open_connections()
        == 0));
}

#[test]
fn endless_flood_of_body_bytes_is_cut_at_content_length() {
    // the client claims 10 bytes and then keeps sending: the extra bytes are not a body, they are
    // garbage after the request, and must get a 400, not be buffered forever
    let srv = start_fast(|_| {});
    let mut c = Client::connect(srv.addr());
    let mut raw =
        b"POST /bookx/users/create HTTP/1.1\r\nHost: x\r\nContent-Length: 10\r\n\r\n".to_vec();
    raw.extend_from_slice(b"0123456789");
    raw.extend_from_slice(&vec![b'z'; 2 * 1024 * 1024]);
    let _ = c.send(&raw);
    let first = c.read_response(false).expect("answer to the first request");
    assert!(first.status == 400 || first.status == 402 || first.status == 201);
    // whatever follows, the connection is eventually closed and the server survives
    assert!(c.is_closed(Duration::from_secs(5)));
    assert!(alive(srv.addr()));
    assert!(eventually(Duration::from_secs(3), || srv
        .open_connections()
        == 0));
}

#[test]
fn partial_request_then_close_frees_the_slot() {
    let srv = start_fast(|l| {
        l.header_timeout = Duration::from_secs(30);
        l.body_timeout = Duration::from_secs(30);
    });
    let mut held = Vec::new();
    for partial in [
        &b"GET /heal"[..],
        b"GET /healthcheck HTTP/1.1\r\nHost:",
        b"POST /bookx/users/create HTTP/1.1\r\nContent-Length: 50\r\n\r\nabc",
        b"\r\n",
    ] {
        let mut c = Client::connect(srv.addr());
        c.send(partial).unwrap();
        held.push(c);
    }
    assert!(eventually(Duration::from_secs(2), || srv
        .open_connections()
        == 4));
    drop(held);
    assert!(
        eventually(Duration::from_secs(3), || srv.open_connections() == 0),
        "open: {}",
        srv.open_connections()
    );
    assert!(alive(srv.addr()));
}

#[test]
fn idle_keep_alive_connection_is_closed_after_the_idle_timeout() {
    let srv = start_fast(|l| l.idle_timeout = MS(300));
    let mut c = Client::connect(srv.addr());
    assert_eq!(c.get("/healthcheck", "").status, 200);
    std::thread::sleep(MS(100));
    assert_eq!(c.get("/healthcheck", "").status, 200, "still open");
    let started = Instant::now();
    assert!(c.is_closed(Duration::from_secs(3)));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(eventually(Duration::from_secs(2), || srv
        .open_connections()
        == 0));
}

#[test]
fn a_client_that_never_reads_the_response_is_dropped() {
    // write deadline: 4 MiB of response towards a peer that reads nothing
    let dir = std::env::temp_dir().join(format!("bookx-hardening-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("1.epub"), vec![7u8; 48 * 1024 * 1024]).unwrap();
    let snap = bookx_serve::server::LibrarySnapshot {
        root: dir.clone(),
        books: vec![book(1, "1.epub")],
    };
    let srv = start_with(
        |l| {
            l.write_timeout = MS(400);
            l.write_min_rate = 16 * 1024 * 1024;
        },
        |c| {
            c.users.clear();
            c.library = Arc::new(move || snap.clone());
        },
    );
    let mut c = Client::connect(srv.addr());
    c.send(b"GET /download/1.epub HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    assert!(eventually(Duration::from_secs(2), || srv
        .open_connections()
        == 1));
    // do not read; the server must give up after its (size-scaled) write deadline
    assert!(eventually(Duration::from_secs(20), || srv
        .open_connections()
        == 0));
    drop(c);
    srv.shutdown();
    std::fs::remove_dir_all(dir).ok();
}

fn book(id: u64, file: &str) -> bookx_core::library::Book {
    use chrono::Utc;
    bookx_core::library::Book {
        id,
        file: file.into(),
        cover: None,
        meta: bookx_core::opf::Metadata {
            title: "T".into(),
            authors: vec!["A".into()],
            ..Default::default()
        },
        added: Utc::now(),
        modified: Utc::now(),
        last_read: None,
        size: 0,
        progress: 0.0,
        position: None,
        bookmarks: vec![],
        highlights: vec![],
        formats: vec![],
        resources: vec![],
        drm: String::new(),
    }
}

// ----------------------------------------------------------------------- connection handling

#[test]
fn pipelined_requests_are_answered_in_order() {
    let srv = start_fast(|_| {});
    let mut c = Client::connect(srv.addr());
    let auth = auth_header();
    let mut raw = Vec::new();
    raw.extend_from_slice(b"GET /healthcheck HTTP/1.1\r\nHost: x\r\n\r\n");
    raw.extend_from_slice(format!("GET /nothing HTTP/1.1\r\nHost: x\r\n{auth}\r\n").as_bytes());
    raw.extend_from_slice(
        b"POST /bookx/users/create HTTP/1.1\r\nHost: x\r\nContent-Length: 31\r\n\r\n{\"username\":\"p\",\"password\":\"q\"}",
    );
    raw.extend_from_slice(b"HEAD /healthcheck HTTP/1.1\r\nHost: x\r\n\r\n");
    raw.extend_from_slice(b"GET /healthcheck HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    c.send(&raw).unwrap();
    assert_eq!(c.read_response(false).unwrap().status, 200);
    assert_eq!(c.read_response(false).unwrap().status, 404);
    assert_eq!(c.read_response(false).unwrap().status, 201);
    let head = c.read_response(true).unwrap();
    assert_eq!(head.status, 200);
    assert!(head.header("content-length").is_some());
    let last = c.read_response(false).unwrap();
    assert_eq!(last.status, 200);
    assert_eq!(last.header("connection"), Some("close"));
    assert!(c.is_closed(Duration::from_secs(2)));
}

#[test]
fn pipelined_flood_in_one_write_is_all_served() {
    let srv = start_fast(|_| {});
    let mut c = Client::connect(srv.addr());
    let one = b"GET /healthcheck HTTP/1.1\r\nHost: x\r\n\r\n";
    let n = 200;
    c.send(&one.repeat(n)).unwrap();
    for i in 0..n {
        assert_eq!(
            c.read_response(false)
                .unwrap_or_else(|| panic!("#{i}"))
                .status,
            200
        );
    }
}

#[test]
fn keep_alive_connection_is_reused() {
    let srv = start_fast(|_| {});
    let mut c = Client::connect(srv.addr());
    for _ in 0..25 {
        let r = c.get("/healthcheck", "");
        assert_eq!(r.status, 200);
        assert_eq!(r.header("connection"), Some("keep-alive"));
    }
    assert_eq!(srv.open_connections(), 1);
}

#[test]
fn head_keeps_the_connection_framed() {
    let srv = start_fast(|_| {});
    let mut c = Client::connect(srv.addr());
    c.send(b"HEAD /healthcheck HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    let h = c.read_response(true).unwrap();
    assert_eq!(h.status, 200);
    assert_eq!(h.header("content-length"), Some("14"));
    assert_eq!(c.get("/healthcheck", "").text(), r#"{"state":"OK"}"#);
}

#[test]
fn connection_close_is_honoured() {
    let srv = start_fast(|_| {});
    let mut c = Client::connect(srv.addr());
    let r = c.get("/healthcheck", "Connection: close\r\n");
    assert_eq!(r.header("connection"), Some("close"));
    assert!(c.is_closed(Duration::from_secs(2)));
}

#[test]
fn http_1_0_is_answered_and_closed() {
    let srv = start_fast(|_| {});
    // without keep-alive the connection is closed after the answer
    let mut c = Client::connect(srv.addr());
    c.send(b"GET /healthcheck HTTP/1.0\r\n\r\n").unwrap();
    assert_eq!(c.read_response(false).unwrap().status, 200);
    assert!(c.is_closed(Duration::from_secs(2)));
    // an explicit HTTP/1.0 keep-alive is honoured by hyper (the old transport always closed)
    let mut c = Client::connect(srv.addr());
    c.send(b"GET /healthcheck HTTP/1.0\r\nConnection: keep-alive\r\n\r\n")
        .unwrap();
    let r = c.read_response(false).unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(r.header("connection"), Some("keep-alive"));
    c.send(b"GET /healthcheck HTTP/1.0\r\n\r\n").unwrap();
    assert_eq!(c.read_response(false).unwrap().status, 200);
    assert!(c.is_closed(Duration::from_secs(2)));
}

#[test]
fn unsupported_http_versions() {
    let srv = start_fast(|_| {});
    // hyper answers every unknown version with 400 (the old transport said 505)
    for v in ["HTTP/2.0", "HTTP/1.2", "HTTP/0.9"] {
        let r = once(srv.addr(), format!("GET / {v}\r\n\r\n").as_bytes()).unwrap();
        assert!(matches!(r.status, 400 | 505), "{v}: {}", r.status);
    }
    assert_eq!(
        once(srv.addr(), b"GET / SPDY/3\r\n\r\n").unwrap().status,
        400
    );
}

#[test]
fn max_requests_per_connection_closes_it() {
    let srv = start_fast(|l| l.max_requests_per_connection = 3);
    let mut c = Client::connect(srv.addr());
    assert_eq!(
        c.get("/healthcheck", "").header("connection"),
        Some("keep-alive")
    );
    assert_eq!(
        c.get("/healthcheck", "").header("connection"),
        Some("keep-alive")
    );
    assert_eq!(
        c.get("/healthcheck", "").header("connection"),
        Some("close")
    );
    assert!(c.is_closed(Duration::from_secs(2)));
}

#[test]
fn leading_blank_lines_before_the_request_are_ignored() {
    let srv = start_fast(|_| {});
    let r = once(srv.addr(), b"\r\n\r\nGET /healthcheck HTTP/1.1\r\n\r\n").unwrap();
    assert_eq!(r.status, 200);
}

// ---------------------------------------------------------------------------- connection caps

#[test]
fn connection_cap_gives_503_and_recovers() {
    let srv = start_fast(|l| {
        l.max_connections = 6;
        l.max_connections_per_ip = 6;
        l.idle_timeout = Duration::from_secs(30);
        l.header_timeout = Duration::from_secs(30);
    });
    let mut idle: Vec<Client> = (0..6).map(|_| Client::connect(srv.addr())).collect();
    assert!(eventually(Duration::from_secs(2), || srv
        .open_connections()
        == 6));

    let auth = auth_header();
    let mut extra = Client::connect(srv.addr());
    extra
        .send(format!("GET /opds HTTP/1.1\r\nHost: x\r\n{auth}\r\n").as_bytes())
        .unwrap();
    let r = extra.read_response(false).expect("503");
    assert_eq!(r.status, 503);
    assert_eq!(r.header("retry-after"), Some("1"));
    assert!(extra.is_closed(Duration::from_secs(3)));

    // free one slot: the authenticated request goes through
    drop(idle.pop());
    assert!(eventually(Duration::from_secs(3), || srv
        .open_connections()
        == 5));
    let mut c = Client::connect(srv.addr());
    assert_eq!(c.get("/opds", &auth).status, 200);
    drop(c);
    drop(idle);
    assert!(eventually(Duration::from_secs(3), || srv
        .open_connections()
        == 0));
}

#[test]
fn sixty_four_idle_connections_do_not_block_an_authenticated_request() {
    // default cap is 64 connections: 60 idle ones leave room, 64 do not
    let srv = start_with(
        |l| {
            l.max_connections_per_ip = 100;
            l.idle_timeout = Duration::from_secs(30);
            l.header_timeout = Duration::from_secs(30);
        },
        |c| c.threads = 2,
    );
    let auth = auth_header();
    let mut idle: Vec<Client> = (0..60).map(|_| Client::connect(srv.addr())).collect();
    assert!(eventually(Duration::from_secs(3), || srv
        .open_connections()
        == 60));
    // only 2 handler threads, 60 idle sockets: requests still run
    let mut c = Client::connect(srv.addr());
    let started = Instant::now();
    assert_eq!(c.get("/opds", &auth).status, 200);
    assert!(started.elapsed() < Duration::from_secs(2));

    // fill the table completely (59 + the open one = 60; add 3 more idle = 63 + c = 64)
    idle.extend((0..3).map(|_| Client::connect(srv.addr())));
    assert!(eventually(Duration::from_secs(3), || srv
        .open_connections()
        == 64));
    let mut over = Client::connect(srv.addr());
    over.send(format!("GET /opds HTTP/1.1\r\nHost: x\r\n{auth}\r\n").as_bytes())
        .unwrap();
    assert_eq!(over.read_response(false).expect("503").status, 503);
    // the request on the established connection is still served
    assert_eq!(c.get("/opds", &auth).status, 200);
    // and once the idle ones go away, new clients are fine again
    idle.clear();
    assert!(eventually(Duration::from_secs(3), || srv
        .open_connections()
        == 1));
    let mut d = Client::connect(srv.addr());
    assert_eq!(d.get("/opds", &auth).status, 200);
}

#[test]
fn default_limits_cap_at_64_connections() {
    let srv = start_with(|l| l.max_connections_per_ip = 1000, |_| {});
    let held: Vec<Client> = (0..64).map(|_| Client::connect(srv.addr())).collect();
    assert!(eventually(Duration::from_secs(3), || srv
        .open_connections()
        == 64));
    let mut over = Client::connect(srv.addr());
    over.send(b"GET /healthcheck HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    assert_eq!(over.read_response(false).unwrap().status, 503);
    drop(held);
}

#[test]
fn per_address_cap_leaves_room_for_others() {
    let srv = start_fast(|l| {
        l.max_connections = 50;
        l.max_connections_per_ip = 4;
        l.idle_timeout = Duration::from_secs(30);
        l.header_timeout = Duration::from_secs(30);
    });
    let held: Vec<Client> = (0..4).map(|_| Client::connect(srv.addr())).collect();
    assert!(eventually(Duration::from_secs(2), || srv
        .open_connections()
        == 4));
    let mut fifth = Client::connect(srv.addr());
    fifth
        .send(b"GET /healthcheck HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    assert_eq!(fifth.read_response(false).unwrap().status, 503);
    drop(held);
    assert!(eventually(Duration::from_secs(2), || srv
        .open_connections()
        == 0));
    assert!(alive(srv.addr()));
}

#[test]
fn slowloris_flood_is_released_by_the_timeouts() {
    let srv = start_fast(|l| {
        l.max_connections = 12;
        l.max_connections_per_ip = 12;
    });
    let mut slow: Vec<Client> = (0..12).map(|_| Client::connect(srv.addr())).collect();
    for c in &mut slow {
        c.send(b"GET /opds HTTP/1.1\r\nX-Slow: ").unwrap();
    }
    assert!(eventually(Duration::from_secs(2), || srv
        .open_connections()
        == 12));
    // table full: a legitimate client is turned away for now ...
    let r = once(srv.addr(), b"GET /healthcheck HTTP/1.1\r\n\r\n").unwrap();
    assert_eq!(r.status, 503);
    // ... and served once the 500 ms header deadline has freed the slots
    assert!(eventually(Duration::from_secs(5), || alive(srv.addr())));
    for c in &mut slow {
        assert!(c.is_closed(Duration::from_secs(3)));
    }
}

#[test]
fn handler_concurrency_is_bounded_by_threads() {
    let current = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (cur, pk) = (current.clone(), peak.clone());
    let provider = Arc::new(move || {
        let now = cur.fetch_add(1, Ordering::SeqCst) + 1;
        pk.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(MS(150));
        cur.fetch_sub(1, Ordering::SeqCst);
        bookx_serve::server::LibrarySnapshot::default()
    });
    let srv = start_with(
        |_| {},
        |c| {
            c.users.clear();
            c.threads = 2;
            c.library = provider;
        },
    );
    let addr = srv.addr();
    let started = Instant::now();
    let hs: Vec<_> = (0..6)
        .map(|_| {
            std::thread::spawn(move || {
                once(
                    addr,
                    b"GET /opds HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
                )
                .unwrap()
                .status
            })
        })
        .collect();
    for h in hs {
        assert_eq!(h.join().unwrap(), 200);
    }
    assert!(
        peak.load(Ordering::SeqCst) <= 2,
        "peak {}",
        peak.load(Ordering::SeqCst)
    );
    assert!(started.elapsed() >= MS(440), "three rounds of 150 ms");
}

// ------------------------------------------------------------------------------ malformed input

#[test]
fn invalid_utf8_requests_get_400() {
    let srv = start_fast(|_| {});
    for raw in [
        &b"GET /\xff\xfe HTTP/1.1\r\n\r\n"[..],
        b"GET / HTTP/1.1\r\nX: \xc3\x28\r\n\r\n",
        b"\xff\xfe\xfd / HTTP/1.1\r\n\r\n",
        b"GET /%FF%FE HTTP/1.1\r\nAuthorization: Basic cmVhZGVyOnMzY3JldA==\r\n\r\n",
    ] {
        let r = once(srv.addr(), raw).unwrap();
        assert_eq!(r.status, 400, "{raw:?}");
    }
    assert!(alive(srv.addr()));
}

#[test]
fn bad_percent_encoding_never_crashes() {
    let srv = start_fast(|_| {});
    let auth = auth_header();
    for (path, ok) in [
        ("/%", [400, 404]),
        ("/%zz", [400, 404]),
        ("/%2", [400, 404]),
        ("/download/%%%", [400, 404]),
        ("/?q=%", [200, 200]),
        ("/?q=%zz%FF", [200, 200]),
        ("/opds/search?q=%E6%97", [200, 200]),
        ("/%C0%AE%C0%AE/", [400, 404]),
    ] {
        let r = once(
            srv.addr(),
            format!("GET {path} HTTP/1.1\r\nHost: x\r\n{auth}Connection: close\r\n\r\n").as_bytes(),
        )
        .unwrap_or_else(|| panic!("{path}"));
        assert!(ok.contains(&r.status), "{path} -> {}", r.status);
    }
    assert!(alive(srv.addr()));
}

#[test]
fn garbage_and_malformed_request_lines() {
    let srv = start_fast(|_| {});
    for raw in [
        &b"\x00\x01\x02 not http \xff\xfe\r\n\r\n"[..],
        b"GET\r\n\r\n",
        b"GET /\r\n\r\n",
        b"GET  /  HTTP/1.1\r\n\r\n",
        b"get / HTTP/1.1\r\n\r\n",
        b"GET / HTTP/1.1 junk\r\n\r\n",
        b"GET /a b HTTP/1.1\r\n\r\n",
        b"GET /\x01 HTTP/1.1\r\n\r\n",
        b"GET /\0 HTTP/1.1\r\n\r\n",
        b"GET / HTTP/1.1\r\nNoColon\r\n\r\n",
        b"GET / HTTP/1.1\r\n: nameless\r\n\r\n",
        b"GET / HTTP/1.1\r\nBad Name: v\r\n\r\n",
        b"GET / HTTP/1.1\r\nName : v\r\n\r\n",
        b"GET / HTTP/1.1\r\nA: b\r\n folded\r\n\r\n",
        b"GET * HTTP/1.1\r\n\r\n",
        b"CONNECT example.com:443 HTTP/1.1\r\n\r\n",
    ] {
        let r = once(srv.addr(), raw).unwrap_or_else(|| panic!("no reply to {raw:?}"));
        assert_eq!(r.status, 400, "{raw:?}");
        assert_eq!(r.header("connection"), Some("close"));
    }
    // a fragment is silently dropped by hyper's URI parser (the old transport answered 400); the
    // handler never sees it
    let r = once(
        srv.addr(),
        b"GET /healthcheck#frag HTTP/1.1\r\nConnection: close\r\n\r\n",
    )
    .unwrap();
    assert_eq!(r.status, 200);
    assert!(alive(srv.addr()));
}

#[test]
fn bare_lf_line_endings_are_never_interpreted() {
    let srv = start_fast(|_| {});
    // no CRLFCRLF anywhere: nothing may be served
    // (hyper would accept bare LF as a line ending; the guard in front of it refuses it at once)
    let mut c = Client::connect(srv.addr());
    c.send(b"GET /healthcheck HTTP/1.1\nHost: x\n\n").unwrap();
    let r = c.read_response(false).expect("400");
    assert_eq!(r.status, 400);
    assert!(c.is_closed(Duration::from_secs(3)));

    // a bare LF smuggled inside a header must not start a second request line
    let raw = b"GET /healthcheck HTTP/1.1\r\nX: a\nGET /opds HTTP/1.1\r\n\r\n";
    let mut c = Client::connect(srv.addr());
    c.send(raw).unwrap();
    assert_eq!(c.read_response(false).unwrap().status, 400);
    assert!(c.read_response(false).is_none(), "exactly one answer");

    // a bare CR too
    let raw = b"GET /healthcheck HTTP/1.1\r\nX: a\rGET /opds HTTP/1.1\r\n\r\n";
    assert_eq!(once(srv.addr(), raw).unwrap().status, 400);
}

#[test]
fn absolute_form_targets() {
    let srv = start_fast(|_| {});
    let auth = auth_header();
    let r = once(
        srv.addr(),
        format!("GET http://evil.example:81/opds?x=1 HTTP/1.1\r\nHost: other\r\n{auth}Connection: close\r\n\r\n")
            .as_bytes(),
    )
    .unwrap();
    assert_eq!(r.status, 200);
    let r = once(
        srv.addr(),
        b"GET http://evil.example HTTP/1.1\r\nHost: other\r\nConnection: close\r\n\r\n",
    )
    .unwrap();
    assert_eq!(r.status, 401, "reduced to / which needs auth");
    for bad in [
        "GET https://h/opds HTTP/1.1",
        "GET ftp://h/ HTTP/1.1",
        "GET http:///opds HTTP/1.1",
        "GET http://u:p@h/opds HTTP/1.1",
        "GET //h/opds HTTP/1.1x",
    ] {
        let r = once(srv.addr(), format!("{bad}\r\n\r\n").as_bytes()).unwrap();
        assert!(r.status == 400 || r.status == 505, "{bad}");
    }
    // traversal inside an absolute-form target
    let r = once(
        srv.addr(),
        format!(
            "GET http://h/download/../../etc/passwd HTTP/1.1\r\n{auth}Connection: close\r\n\r\n"
        )
        .as_bytes(),
    )
    .unwrap();
    assert!(matches!(r.status, 400 | 404));
}

#[test]
fn unknown_and_unusual_methods() {
    let srv = start_fast(|_| {});
    let auth = auth_header();
    for m in ["PROPFIND", "DELETE", "PATCH", "OPTIONS", "TRACE"] {
        let r = once(
            srv.addr(),
            format!("{m} /opds HTTP/1.1\r\nHost: x\r\n{auth}Connection: close\r\n\r\n").as_bytes(),
        )
        .unwrap();
        assert_eq!(r.status, 405, "{m}");
    }
}

// ----------------------------------------------------------------------- bodies and framing

#[test]
fn expect_100_continue_flow() {
    let srv = start_fast(|_| {});
    let body = br#"{"username":"e1","password":"pw"}"#;
    let mut c = Client::connect(srv.addr());
    c.send(
        format!(
            "POST /bookx/users/create HTTP/1.1\r\nHost: x\r\nExpect: 100-continue\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .as_bytes(),
    )
    .unwrap();
    // the interim response arrives before the body is sent
    let interim = c.read_response(false).expect("100");
    assert_eq!(interim.status, 100);
    c.send(body).unwrap();
    assert_eq!(c.read_response(false).unwrap().status, 201);
    // the connection stays usable
    assert_eq!(c.get("/healthcheck", "").status, 200);
}

#[test]
fn expect_continue_is_not_sent_for_refused_requests() {
    let srv = start_fast(|_| {});
    let mut c = Client::connect(srv.addr());
    c.send(
        b"POST /bookx/users/create HTTP/1.1\r\nHost: x\r\nExpect: 100-continue\r\nContent-Length: 99999999\r\n\r\n",
    )
    .unwrap();
    let r = c.read_response(false).unwrap();
    assert_eq!(r.status, 413, "straight to the error, no 100 first");
    assert!(c.is_closed(Duration::from_secs(3)));

    let r = once(
        srv.addr(),
        b"POST /bookx/users/create HTTP/1.1\r\nExpect: banana\r\nContent-Length: 2\r\n\r\n{}",
    )
    .unwrap();
    assert_eq!(r.status, 417);
}

#[test]
fn expect_without_a_body_needs_no_interim_response() {
    let srv = start_fast(|_| {});
    let r = once(
        srv.addr(),
        b"GET /healthcheck HTTP/1.1\r\nExpect: 100-continue\r\nConnection: close\r\n\r\n",
    )
    .unwrap();
    assert_eq!(r.status, 200);
}

#[test]
fn body_limit_over_http() {
    let srv = start_fast(|l| l.max_body = 100);
    let ok = format!(r#"{{"username":"u","password":"{}"}}"#, "p".repeat(50));
    let r = once(
        srv.addr(),
        format!(
            "POST /bookx/users/create HTTP/1.1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{ok}",
            ok.len()
        )
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(r.status, 201);
    let big = "x".repeat(101);
    let mut c = Client::connect(srv.addr());
    c.send(
        format!(
            "POST /bookx/users/create HTTP/1.1\r\nContent-Length: {}\r\n\r\n{big}",
            big.len()
        )
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(c.read_response(false).unwrap().status, 413);
    assert!(c.is_closed(Duration::from_secs(3)));
}

#[test]
fn body_arriving_in_tiny_pieces_is_assembled() {
    let srv = start_fast(|l| l.body_timeout = Duration::from_secs(5));
    let body = br#"{"username":"slow","password":"pw"}"#;
    let mut c = Client::connect(srv.addr());
    c.send(
        format!(
            "POST /bookx/users/create HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .as_bytes(),
    )
    .unwrap();
    for b in body {
        c.send(&[*b]).unwrap();
    }
    assert_eq!(c.read_response(false).unwrap().status, 201);
}

#[test]
fn one_mebibyte_body_is_accepted() {
    let srv = start_fast(|l| l.body_timeout = Duration::from_secs(10));
    let mut body = br#"{"username":"big","password":"x","pad":""#.to_vec();
    body.resize(1024 * 1024 - 2, b'a');
    body.extend_from_slice(br#""}"#);
    assert_eq!(body.len(), 1024 * 1024);
    let mut c = Client::connect(srv.addr());
    c.send(
        format!(
            "POST /bookx/users/create HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .as_bytes(),
    )
    .unwrap();
    c.send(&body).unwrap();
    let r = c.read_response(false).unwrap();
    assert!(matches!(r.status, 201 | 400), "{}", r.status); // JSON accepted or politely refused
}

#[test]
fn chunked_bodies_are_cleanly_refused() {
    let srv = start_fast(|_| {});
    let chunked = b"POST /bookx/users/create HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
    let r = once(srv.addr(), chunked).unwrap();
    assert_eq!(r.status, 501);
    assert_eq!(r.header("connection"), Some("close"));
    for te in [
        "Transfer-Encoding: identity",
        "Transfer-Encoding: gzip, chunked",
        "Transfer-Encoding:\tchunked",
        "transfer-encoding: CHUNKED",
        "Transfer-Encoding: xchunked",
    ] {
        let r = once(
            srv.addr(),
            format!("PUT /bookx/syncs/progress HTTP/1.1\r\n{te}\r\n\r\n0\r\n\r\n").as_bytes(),
        )
        .unwrap();
        // hyper itself refuses a Transfer-Encoding that does not end in `chunked` with 400
        assert!(matches!(r.status, 400 | 501), "{te}: {}", r.status);
    }
    // "Transfer-Encoding : chunked" and "Transfer-Encoding: chunked" with obfuscated name
    for te in [
        "Transfer-Encoding : chunked",
        " Transfer-Encoding: chunked",
        "Transfer_Encoding: chunked",
    ] {
        let r = once(
            srv.addr(),
            format!("PUT /bookx/syncs/progress HTTP/1.1\r\n{te}\r\n\r\n0\r\n\r\n").as_bytes(),
        );
        // either rejected outright, or (for the unknown header name) ignored and answered normally
        let r = r.unwrap();
        assert!(matches!(r.status, 400 | 401 | 404), "{te} -> {}", r.status);
    }
}

#[test]
fn request_smuggling_attempts_are_rejected() {
    let srv = start_fast(|_| {});
    let smuggled = "GET /healthcheck HTTP/1.1\r\nHost: x\r\n\r\n";
    // CL + TE in both orders
    for headers in [
        "Content-Length: 4\r\nTransfer-Encoding: chunked\r\n",
        "Transfer-Encoding: chunked\r\nContent-Length: 4\r\n",
        "Content-Length: 0\r\nTransfer-Encoding: chunked\r\n",
    ] {
        let mut c = Client::connect(srv.addr());
        c.send(
            format!(
                "POST /bookx/users/create HTTP/1.1\r\nHost: x\r\n{headers}\r\n0\r\n\r\n{smuggled}"
            )
            .as_bytes(),
        )
        .unwrap();
        let r = c.read_response(false).unwrap();
        // hyper drops the Content-Length that comes with a Transfer-Encoding, so we see (and
        // refuse) the Transfer-Encoding alone: 501 instead of the former 400
        assert!(matches!(r.status, 400 | 501), "{headers:?}: {}", r.status);
        assert!(
            c.read_response(false).is_none(),
            "the smuggled request ran: {headers:?}"
        );
    }
    // duplicate / conflicting Content-Length
    for headers in [
        "Content-Length: 5\r\nContent-Length: 40\r\n",
        "Content-Length: 5\r\ncontent-length: 6\r\n",
    ] {
        let mut c = Client::connect(srv.addr());
        c.send(
            format!("POST /bookx/users/create HTTP/1.1\r\n{headers}\r\n12345{smuggled}").as_bytes(),
        )
        .unwrap();
        assert_eq!(c.read_response(false).unwrap().status, 400, "{headers:?}");
        assert!(c.read_response(false).is_none(), "{headers:?}");
    }
    // identical duplicates are collapsed by hyper (RFC 9112 6.3: same framing for everybody), so
    // the five body bytes are consumed and the next request is a normal pipelined one
    let mut c = Client::connect(srv.addr());
    c.send(
        format!(
            "POST /bookx/users/create HTTP/1.1\r\nContent-Length: 5\r\nContent-Length: 5\r\n\r\n12345{smuggled}"
        )
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(c.read_response(false).unwrap().status, 400);
    assert_eq!(c.read_response(false).unwrap().status, 200);
    // malformed Content-Length values
    for v in ["+5", "-5", "5 5", "0x5", "5,5", "", "abc", "1e3"] {
        let r = once(
            srv.addr(),
            format!("POST /bookx/users/create HTTP/1.1\r\nContent-Length: {v}\r\n\r\n12345")
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(r.status, 400, "{v:?}");
    }
    // a body longer than declared is not a second request unless it parses as one, and then it is
    // answered separately and in order (normal pipelining), never merged into the first
    let mut c = Client::connect(srv.addr());
    c.send(
        format!("POST /bookx/users/create HTTP/1.1\r\nContent-Length: 2\r\n\r\n{{}}{smuggled}")
            .as_bytes(),
    )
    .unwrap();
    let first = c.read_response(false).unwrap();
    assert!(matches!(first.status, 400 | 402 | 201), "{}", first.status);
    assert_eq!(c.read_response(false).unwrap().status, 200);
    assert!(alive(srv.addr()));
}

#[test]
fn response_splitting_through_the_request_is_not_possible() {
    let srv = start_fast(|_| {});
    let auth = auth_header();
    // CRLF encoded in the URL reaches the handler only as %0d%0a text
    let r = once(
        srv.addr(),
        format!("GET /?q=a%0d%0aSet-Cookie:%20x=1 HTTP/1.1\r\n{auth}Connection: close\r\n\r\n")
            .as_bytes(),
    )
    .unwrap();
    assert_eq!(r.status, 200);
    assert!(r.header("set-cookie").is_none());
}

// ------------------------------------------------------------------------------ path traversal

#[test]
fn more_path_traversal_attempts() {
    let srv = start_fast(|_| {});
    let auth = auth_header();
    let attempts = [
        "/download/..%2f..%2f..%2fetc%2fpasswd",
        "/download/%2e%2e%2f%2e%2e%2fetc%2fpasswd",
        "/download/%252e%252e%252fetc%252fpasswd",
        "/download/....//....//etc/passwd",
        "/download/..%c0%af..%c0%afetc/passwd",
        "/download/..%5c..%5cwindows%5cwin.ini",
        "/download/..\\..\\etc\\passwd",
        "/cover/../../../etc/passwd",
        "/cover/%2e%2e/%2e%2e/etc/passwd",
        "/cover/1/../../../etc/passwd",
        "/cover/1%00.jpg",
        "/opds/%2e%2e/%2e%2e/etc/passwd",
        "/opds/..;/..;/etc/passwd",
        "/./././etc/passwd",
        "/kosync/../../etc/passwd",
        "/kosync/%2e%2e/etc/passwd",
        "/bookx/../../etc/passwd",
        "/bookx/%2e%2e/etc/passwd",
        "/download/1.epub/../../../../etc/passwd",
        "/download/file:///etc/passwd",
        "/download//etc/passwd",
        "/etc/passwd",
        "/proc/self/environ",
    ];
    for a in attempts {
        let raw = format!("GET {a} HTTP/1.1\r\nHost: x\r\n{auth}Connection: close\r\n\r\n");
        let Some(r) = once(srv.addr(), raw.as_bytes()) else {
            continue;
        };
        let text = r.text();
        assert!(
            !text.contains("root:x:") && !text.contains("PATH="),
            "leak via {a}"
        );
        assert!(matches!(r.status, 400 | 404), "{a} -> {}", r.status);
    }
    assert!(alive(srv.addr()));
}

// ------------------------------------------------------------------------------ auth throttle

#[test]
fn failed_logins_are_throttled_per_address() {
    let srv = start_fast(|l| {
        l.auth_max_failures = 3;
        l.auth_block = MS(600);
    });
    let bad = format!("Authorization: {}\r\n", basic(USER, "wrong"));
    let good = auth_header();
    // requests without credentials (browser first visit) are not failures
    for _ in 0..10 {
        assert_eq!(
            once(
                srv.addr(),
                b"GET /opds HTTP/1.1\r\nConnection: close\r\n\r\n"
            )
            .unwrap()
            .status,
            401
        );
    }
    assert_eq!(
        once(
            srv.addr(),
            format!("GET /opds HTTP/1.1\r\n{good}Connection: close\r\n\r\n").as_bytes()
        )
        .unwrap()
        .status,
        200
    );
    for _ in 0..3 {
        let r = once(
            srv.addr(),
            format!("GET /opds HTTP/1.1\r\n{bad}Connection: close\r\n\r\n").as_bytes(),
        )
        .unwrap();
        assert_eq!(r.status, 401);
    }
    // now blocked: even the right password and the health check get 429
    let started = Instant::now();
    let r = once(
        srv.addr(),
        format!("GET /opds HTTP/1.1\r\n{good}Connection: close\r\n\r\n").as_bytes(),
    )
    .unwrap();
    assert_eq!(r.status, 429);
    assert!(r.header("retry-after").unwrap().parse::<u64>().unwrap() >= 1);
    assert_eq!(r.header("connection"), Some("close"));
    assert_eq!(
        once(
            srv.addr(),
            b"GET /healthcheck HTTP/1.1\r\nConnection: close\r\n\r\n"
        )
        .unwrap()
        .status,
        429
    );
    // keep-alive connections are cut as well
    let mut c = Client::connect(srv.addr());
    assert_eq!(c.get("/opds", &good).status, 429);
    assert!(c.is_closed(Duration::from_secs(2)));
    // the block ends
    std::thread::sleep(MS(650).saturating_sub(started.elapsed()));
    assert_eq!(
        once(
            srv.addr(),
            format!("GET /opds HTTP/1.1\r\n{good}Connection: close\r\n\r\n").as_bytes()
        )
        .unwrap()
        .status,
        200
    );
}

#[test]
fn failed_kosync_logins_count_too() {
    let srv = start_fast(|l| l.auth_max_failures = 4);
    for _ in 0..4 {
        let r = once(
            srv.addr(),
            b"GET /bookx/users/auth HTTP/1.1\r\nx-auth-user: nobody\r\nx-auth-key: 00\r\nConnection: close\r\n\r\n",
        )
        .unwrap();
        assert_eq!(r.status, 401);
    }
    let r = once(
        srv.addr(),
        b"GET /bookx/users/auth HTTP/1.1\r\nx-auth-user: nobody\r\nx-auth-key: 00\r\nConnection: close\r\n\r\n",
    )
    .unwrap();
    assert_eq!(r.status, 429);
}

#[test]
fn a_success_does_not_reset_the_failure_count() {
    let srv = start_fast(|l| l.auth_max_failures = 3);
    let bad = format!("Authorization: {}\r\n", basic(USER, "wrong"));
    let good = auth_header();
    for _ in 0..2 {
        once(
            srv.addr(),
            format!("GET /opds HTTP/1.1\r\n{bad}Connection: close\r\n\r\n").as_bytes(),
        )
        .unwrap();
        assert_eq!(
            once(
                srv.addr(),
                format!("GET /opds HTTP/1.1\r\n{good}Connection: close\r\n\r\n").as_bytes()
            )
            .unwrap()
            .status,
            200
        );
    }
    // the 3rd failure still trips it
    let r = once(
        srv.addr(),
        format!("GET /opds HTTP/1.1\r\n{bad}Connection: close\r\n\r\n").as_bytes(),
    )
    .unwrap();
    assert_eq!(r.status, 401);
    let r = once(
        srv.addr(),
        format!("GET /opds HTTP/1.1\r\n{good}Connection: close\r\n\r\n").as_bytes(),
    )
    .unwrap();
    assert_eq!(r.status, 429);
}

#[test]
fn throttle_can_be_switched_off() {
    let srv = start_fast(|l| l.auth_max_failures = 0);
    let bad = format!("Authorization: {}\r\n", basic(USER, "wrong"));
    for _ in 0..30 {
        let r = once(
            srv.addr(),
            format!("GET /opds HTTP/1.1\r\n{bad}Connection: close\r\n\r\n").as_bytes(),
        )
        .unwrap();
        assert_eq!(r.status, 401);
    }
}

// ------------------------------------------------------------------------------ response headers

#[test]
fn security_headers_on_responses() {
    let srv = start_fast(|_| {});
    let auth = auth_header();
    let mut c = Client::connect(srv.addr());
    for (path, extra) in [
        ("/opds", auth.as_str()),
        ("/", auth.as_str()),
        ("/nothing", auth.as_str()),
    ] {
        let r = c.get(path, extra);
        assert_eq!(r.header("cache-control"), Some("no-store"), "{path}");
        assert_eq!(
            r.header("x-content-type-options"),
            Some("nosniff"),
            "{path}"
        );
    }
    let r = c.get("/opds", "");
    assert_eq!(r.status, 401);
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(r.header("cache-control"), Some("no-store"));
    // errors produced by our transport layer carry nosniff as well (hyper's own bodyless 400s do not)
    let r = once(
        srv.addr(),
        b"PUT / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
    )
    .unwrap();
    assert_eq!(r.status, 501);
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    let r = once(
        srv.addr(),
        format!("GET /{} HTTP/1.1\r\n\r\n", "a".repeat(9000)).as_bytes(),
    )
    .unwrap();
    assert_eq!(r.status, 414);
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    // the health check stays cacheable-agnostic
    let r = c.get("/healthcheck", "");
    assert_eq!(r.header("cache-control"), None);
    // without users nothing needs no-store
    let open = start_with(|_| {}, |c| c.users.clear());
    let mut c = Client::connect(open.addr());
    let r = c.get("/opds", "");
    assert_eq!(r.status, 200);
    assert_eq!(r.header("cache-control"), None);
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    assert!(r.header("date").unwrap().ends_with(" GMT"));
}

// ------------------------------------------------------------------------------------ shutdown

#[test]
fn shutdown_with_open_connections_is_prompt_and_closes_them() {
    let srv = start_with(
        |l| {
            l.idle_timeout = Duration::from_secs(60);
            l.header_timeout = Duration::from_secs(60);
            l.body_timeout = Duration::from_secs(60);
        },
        |_| {},
    );
    let addr = srv.addr();
    let mut idle: Vec<Client> = (0..5).map(|_| Client::connect(addr)).collect();
    let mut kept = Client::connect(addr);
    assert_eq!(kept.get("/healthcheck", "").status, 200);
    let mut partial = Client::connect(addr);
    partial
        .send(b"GET /opds HTTP/1.1\r\nHost: x\r\nX-Half")
        .unwrap();
    let mut body = Client::connect(addr);
    body.send(b"POST /bookx/users/create HTTP/1.1\r\nContent-Length: 100\r\n\r\nabc")
        .unwrap();
    assert!(eventually(Duration::from_secs(2), || srv
        .open_connections()
        == 8));

    let started = Instant::now();
    srv.shutdown();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "shutdown took {:?}",
        started.elapsed()
    );
    for c in idle.iter_mut().chain([&mut kept, &mut partial, &mut body]) {
        assert!(c.is_closed(Duration::from_secs(2)));
    }
    assert!(
        TcpStream::connect_timeout(&addr, MS(300)).is_err(),
        "listener closed"
    );
}

#[test]
fn shutdown_lets_a_response_in_flight_finish() {
    let dir = std::env::temp_dir().join(format!("bookx-hardening-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let data: Vec<u8> = (0..16 * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(dir.join("1.epub"), &data).unwrap();
    let snap = bookx_serve::server::LibrarySnapshot {
        root: dir.clone(),
        books: vec![book(1, "1.epub")],
    };
    let srv = start_with(
        |_| {},
        |c| {
            c.users.clear();
            c.library = Arc::new(move || snap.clone());
        },
    );
    let mut c = Client::connect(srv.addr());
    c.send(b"GET /download/1.epub HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    let reader = std::thread::spawn(move || {
        // start reading late so the server is still writing when shutdown begins
        std::thread::sleep(MS(400));
        c.read_response(false)
    });
    std::thread::sleep(MS(150));
    srv.shutdown();
    let r = reader.join().unwrap().expect("full response");
    assert_eq!(r.status, 200);
    assert_eq!(r.body.len(), data.len());
    assert!(r.body == data);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn shutdown_twice_in_a_row_via_drop_is_safe() {
    let srv = start_fast(|_| {});
    let addr = srv.addr();
    drop(srv);
    assert!(TcpStream::connect_timeout(&addr, MS(300)).is_err());
}

#[test]
fn server_survives_connection_resets_midway() {
    let srv = start_fast(|_| {});
    for i in 0..30 {
        let mut s = TcpStream::connect(srv.addr()).unwrap();
        let _ = s.write_all(b"GET /healthcheck HTTP/1.1\r\nHost: x\r\n\r\n");
        if i % 2 == 0 {
            // close with unread data pending: sends RST on Linux
            let _ = s.set_read_timeout(Some(MS(1)));
            let mut b = [0u8; 1];
            let _ = s.read(&mut b);
        }
        drop(s);
    }
    assert!(eventually(Duration::from_secs(3), || srv
        .open_connections()
        == 0));
    assert!(alive(srv.addr()));
}

#[test]
fn many_parallel_clients_with_mixed_abuse() {
    let srv = start_fast(|l| l.max_connections_per_ip = 64);
    let addr = srv.addr();
    let auth = auth_header();
    let hs: Vec<_> = (0..16)
        .map(|t| {
            let auth = auth.clone();
            std::thread::spawn(move || {
                for i in 0..20 {
                    match (t + i) % 5 {
                        0 => {
                            let r = once(
                                addr,
                                format!("GET /opds HTTP/1.1\r\n{auth}Connection: close\r\n\r\n")
                                    .as_bytes(),
                            );
                            assert_eq!(r.map(|r| r.status), Some(200));
                        }
                        1 => {
                            let r = once(addr, b"GARBAGE\r\n\r\n");
                            assert_eq!(r.map(|r| r.status), Some(400));
                        }
                        2 => {
                            let mut c = Client::connect(addr);
                            c.send(b"GET /opds HT").unwrap();
                        }
                        3 => {
                            let r = once(
                                addr,
                                format!("GET /{} HTTP/1.1\r\n\r\n", "a".repeat(9000)).as_bytes(),
                            );
                            assert_eq!(r.map(|r| r.status), Some(414));
                        }
                        _ => {
                            let r = once(
                                addr,
                                b"GET /healthcheck HTTP/1.1\r\nConnection: close\r\n\r\n",
                            );
                            assert_eq!(r.map(|r| r.status), Some(200));
                        }
                    }
                }
            })
        })
        .collect();
    for h in hs {
        h.join().unwrap();
    }
    assert!(eventually(Duration::from_secs(5), || srv
        .open_connections()
        == 0));
    assert!(alive(srv.addr()));
}
