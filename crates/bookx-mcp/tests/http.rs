//! The Streamable HTTP transport with a real client over loopback.

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use bookx_mcp::http::{start, HttpConfig};
use common::*;
use serde_json::{json, Value};

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn header(&self, n: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(n))
            .map(|(_, v)| v.as_str())
    }
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("{e}: {:?}", self.body))
    }
}

/// One request on its own connection.
fn http(addr: std::net::SocketAddr, method: &str, headers: &[(&str, &str)], body: &str) -> Reply {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut req = format!(
        "{method} /mcp HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = Vec::new();
    let _ = s.read_to_end(&mut raw);
    let text = String::from_utf8_lossy(&raw).to_string();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let mut lines = head.lines();
    let status = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    Reply {
        status,
        headers,
        body: body.to_string(),
    }
}

fn auth() -> (&'static str, String) {
    ("Authorization", format!("Bearer {TOKEN}"))
}

fn post(addr: std::net::SocketAddr, extra: &[(&str, &str)], v: &Value) -> Reply {
    let a = auth();
    let mut h = vec![
        (a.0, a.1.as_str()),
        ("Content-Type", "application/json"),
        ("Accept", "application/json, text/event-stream"),
    ];
    h.extend_from_slice(extra);
    http(addr, "POST", &h, &v.to_string())
}

fn server(f: &Fixture) -> bookx_mcp::http::HttpHandle {
    let s = bookx_mcp::new_server(f.config());
    start(s, HttpConfig::new(TOKEN)).unwrap()
}

fn init(addr: std::net::SocketAddr) -> String {
    let r = post(
        addr,
        &[],
        &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"http-test","version":"2"}}}),
    );
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.header("content-type"), Some("application/json"));
    assert_eq!(r.json()["result"]["serverInfo"]["name"], "bookx");
    r.header("mcp-session-id")
        .expect("a session id")
        .to_string()
}

#[test]
fn the_token_is_required_and_checked() {
    let f = Fixture::new("http-auth");
    let h = server(&f);
    let body = json!({"jsonrpc":"2.0","id":1,"method":"ping"}).to_string();
    // missing
    let r = http(
        h.addr(),
        "POST",
        &[("Content-Type", "application/json")],
        &body,
    );
    assert_eq!(r.status, 401);
    assert!(r.header("www-authenticate").unwrap().starts_with("Bearer"));
    assert!(!r.body.contains(TOKEN));
    // wrong, wrong scheme, wrong length, empty
    for bad in [
        "Bearer nope",
        "Bearer 0123456789abcdef0123456789abcdeX",
        &format!("Basic {TOKEN}"),
        "Bearer ",
        TOKEN,
    ] {
        let r = http(h.addr(), "POST", &[("Authorization", bad)], &body);
        assert_eq!(r.status, 401, "{bad}");
    }
    // right
    let r = post(
        h.addr(),
        &[],
        &json!({"jsonrpc":"2.0","id":1,"method":"ping"}),
    );
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["result"], json!({}));
    // the library is not reachable without the token either
    let r = http(h.addr(), "GET", &[], "");
    assert_eq!(r.status, 401);
    h.shutdown();
}

#[test]
fn a_session_lists_and_calls_tools() {
    let f = Fixture::new("http-session");
    let h = server(&f);
    let sid = init(h.addr());
    let ex = [
        ("Mcp-Session-Id", sid.as_str()),
        ("MCP-Protocol-Version", "2025-06-18"),
    ];
    // the initialized notification gets 202 and no body
    let r = post(
        h.addr(),
        &ex,
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    );
    assert_eq!((r.status, r.body.as_str()), (202, ""));
    let r = post(
        h.addr(),
        &ex,
        &json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    );
    assert!(r.json()["result"]["tools"].as_array().unwrap().len() >= 18);
    let r = post(
        h.addr(),
        &ex,
        &json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
        "params":{"name":"search_books","arguments":{"query":"tag:cooking"}}}),
    );
    let res = &r.json()["result"];
    assert_eq!(res["isError"], false);
    assert_eq!(res["structuredContent"]["total"], 1);
    // the app can show who is connected
    let clients = h.endpoint_clients();
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].name, "http-test");
    assert!(clients[0].calls >= 1);
    // unknown session: 404; ending a session
    let r = post(
        h.addr(),
        &[("Mcp-Session-Id", "nope")],
        &json!({"jsonrpc":"2.0","id":4,"method":"ping"}),
    );
    assert_eq!(r.status, 404);
    let a = auth();
    let r = http(
        h.addr(),
        "DELETE",
        &[(a.0, a.1.as_str()), ("Mcp-Session-Id", &sid)],
        "",
    );
    assert_eq!(r.status, 200);
    let r = post(
        h.addr(),
        &ex,
        &json!({"jsonrpc":"2.0","id":5,"method":"ping"}),
    );
    assert_eq!(r.status, 404);
    assert!(h.endpoint_clients().is_empty());
    h.shutdown();
}

#[test]
fn sessionless_clients_are_served_statelessly() {
    let f = Fixture::new("http-stateless");
    let h = server(&f);
    let r = post(
        h.addr(),
        &[],
        &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"recent_books","arguments":{}}}),
    );
    assert_eq!(r.json()["result"]["isError"], false);
    assert!(
        h.endpoint_clients().is_empty(),
        "no lingering session state"
    );
    h.shutdown();
}

#[test]
fn malformed_requests_and_wrong_methods() {
    let f = Fixture::new("http-bad");
    let h = server(&f);
    let a = auth();
    let hdr = [(a.0, a.1.as_str())];
    let r = http(h.addr(), "POST", &hdr, "{not json");
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["error"]["code"], -32700);
    let r = http(
        h.addr(),
        "POST",
        &hdr,
        "[{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}]",
    );
    assert_eq!(
        r.status, 400,
        "batches are not part of the current protocol"
    );
    let r = http(h.addr(), "GET", &hdr, "");
    assert_eq!(r.status, 405);
    assert!(r.header("allow").unwrap().contains("POST"));
    let r = post(
        h.addr(),
        &[("MCP-Protocol-Version", "1900-01-01")],
        &json!({"jsonrpc":"2.0","id":1,"method":"ping"}),
    );
    assert_eq!(r.status, 400);
    let r = post(
        h.addr(),
        &[],
        &json!({"jsonrpc":"2.0","id":1,"method":"nope"}),
    );
    assert_eq!(r.status, 200, "protocol errors travel in the body");
    assert_eq!(r.json()["error"]["code"], -32601);
    h.shutdown();
}

#[test]
fn foreign_origins_are_refused_even_with_the_token() {
    let f = Fixture::new("http-origin");
    let h = server(&f);
    let ping = json!({"jsonrpc":"2.0","id":1,"method":"ping"});
    let r = post(h.addr(), &[("Origin", "https://evil.example")], &ping);
    assert_eq!(r.status, 403);
    let r = post(h.addr(), &[("Origin", "http://localhost:5173")], &ping);
    assert_eq!(r.status, 200);
    h.shutdown();
}

#[test]
fn a_dns_rebinding_host_is_refused_even_with_the_token() {
    let f = Fixture::new("http-host");
    let h = server(&f);
    let ping = json!({"jsonrpc":"2.0","id":1,"method":"ping"}).to_string();
    let raw = |host: &str| {
        let mut s = TcpStream::connect(h.addr()).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let req = format!(
            "POST /mcp HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{ping}",
            ping.len()
        );
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out.split(' ').nth(1).unwrap_or("").to_string()
    };
    assert_eq!(raw("evil.example:8765"), "403");
    assert_eq!(raw("localhost.evil.example"), "403");
    assert_eq!(raw("127.0.0.1:8765"), "200");
    assert_eq!(raw("localhost"), "200");
    h.shutdown();
}

#[test]
fn the_endpoint_needs_a_long_token_and_listens_on_loopback_by_default() {
    let f = Fixture::new("http-token");
    let s = bookx_mcp::new_server(f.config());
    assert!(start(s.clone(), HttpConfig::new("short")).is_err());
    assert!(start(s.clone(), HttpConfig::new("")).is_err());
    let h = start(s, HttpConfig::new(TOKEN)).unwrap();
    assert!(h.addr().ip().is_loopback());
    assert!(h.url().starts_with("http://127.0.0.1:"));
    assert_eq!(bookx_mcp::generate_token().len(), 64);
    h.shutdown();
}

#[test]
fn many_clients_at_once_get_their_own_answers() {
    let f = Fixture::new("http-parallel");
    let h = server(&f);
    let addr = h.addr();
    let threads: Vec<_> = (0..8)
        .map(|i| {
            std::thread::spawn(move || {
                let r = post(
                    addr,
                    &[],
                    &json!({"jsonrpc":"2.0","id":i,"method":"tools/call",
                    "params":{"name":"search_books","arguments":{"limit":3}}}),
                );
                let v = r.json();
                assert_eq!(v["id"], i);
                assert_eq!(
                    v["result"]["structuredContent"]["books"]
                        .as_array()
                        .unwrap()
                        .len(),
                    3
                );
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    h.shutdown();
}

#[test]
fn tls_serves_the_same_endpoint_with_a_self_signed_certificate() {
    let f = Fixture::new("http-tls");
    let (cert, key) = bookx_serve::cert::write_self_signed(
        &f.dir.join("tls"),
        &["localhost".into(), "127.0.0.1".into()],
    )
    .unwrap();
    let mut cfg = HttpConfig::new(TOKEN);
    cfg.tls = Some(bookx_serve::net::tls_config(&cert, &key).unwrap());
    let h = start(bookx_mcp::new_server(f.config()), cfg).unwrap();
    assert!(h.url().starts_with("https://"));
    let pem = std::fs::read(&cert).unwrap();
    let url = url::Url::parse(&h.url()).unwrap();
    let body = json!({"jsonrpc":"2.0","id":1,"method":"ping"}).to_string();
    let bearer = format!("Bearer {TOKEN}");
    let r = bookx_serve::httpc::request_trusting(
        "POST",
        &url,
        &[
            ("Authorization", bearer.as_str()),
            ("Content-Type", "application/json"),
        ],
        body.as_bytes(),
        Duration::from_secs(10),
        Some(&pem),
    )
    .unwrap();
    assert_eq!(r.status, 200);
    let r = bookx_serve::httpc::request_trusting(
        "POST",
        &url,
        &[],
        body.as_bytes(),
        Duration::from_secs(10),
        Some(&pem),
    )
    .unwrap();
    assert_eq!(r.status, 401);
    h.shutdown();
}
