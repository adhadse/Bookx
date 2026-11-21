//! Hostile plugins: manifests, WebAssembly modules and URLs a guest passes to `http_get`. A bad
//! plugin is an error at install or call time, quickly, and never a panic or a larger permission
//! than the manifest asked for. Fuzz twin: `fuzz/fuzz_targets/plugin.rs`.

mod common;

use std::time::{Duration, Instant};

use bookx_plugin::runtime::url_host;
use bookx_plugin::{Host, Manifest, Plugin};
use common::*;

fn timed<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let r = f();
    assert!(
        t.elapsed() < Duration::from_secs(10),
        "{label}: {:?}",
        t.elapsed()
    );
    r
}

#[test]
fn hostile_manifests_are_refused_or_harmless() {
    let ok = manifest_toml("evil.one", "transform", "");
    let deep = format!("a = {}1{}", "[".repeat(5000), "]".repeat(5000));
    let many_perms = format!(
        "permissions = [{}]",
        (0..50_000)
            .map(|i| format!("\"network:h{i}.example\""))
            .collect::<Vec<_>>()
            .join(",")
    );
    let cases: Vec<(&str, String)> = vec![
        ("empty", String::new()),
        ("garbage", "\u{0}\u{1}\u{2}".into()),
        ("bom", format!("\u{feff}{ok}")),
        ("deep", format!("{ok}{deep}")),
        ("many-permissions", format!("{ok}{many_perms}")),
        ("huge-name", ok.replace("Test evil.one", &"N".repeat(1 << 20))),
        ("path-id", ok.replace("evil.one", "../../etc/passwd")),
        ("slash-id", ok.replace("evil.one", "a/b")),
        ("nul-id", ok.replace("evil.one", "a\\u0000b")),
        ("huge-version", ok.replace("1.0.0", &"1.".repeat(100_000))),
        ("negative-api", ok.replace("api_version = 1", "api_version = -1")),
        ("huge-api", ok.replace("api_version = 1", "api_version = 18446744073709551616")),
        ("duplicate-keys", format!("{ok}{ok}")),
        ("wildcard-host", format!("{ok}permissions = [\"network:*\"]")),
        ("localhost-host", format!("{ok}permissions = [\"network:localhost\"]")),
        ("ip-host", format!("{ok}permissions = [\"network:169.254.169.254\"]")),
        ("userinfo-host", format!("{ok}permissions = [\"network:a.com@evil.com\"]")),
        ("scheme-host", format!("{ok}permissions = [\"network:https://a.com\"]")),
        ("path-host", format!("{ok}permissions = [\"network:a.com/../x\"]")),
        ("unknown-permission", format!("{ok}permissions = [\"fs.write:/\", \"exec:sh\", \"editor.write\"]")),
        ("wrong-types", format!("{ok}permissions = \"network:a.com\"\nlimits = 5")),
        ("huge-limits", format!("{ok}[limits]\nfuel = 18446744073709551615\nmemory_pages = 4294967295\ntimeout_ms = 18446744073709551615")),
        ("negative-limits", format!("{ok}[limits]\nfuel = -1\nmemory_pages = -5")),
        ("inline-table-bomb", format!("{ok}x = {}", "{a=".repeat(2000) + "1" + &"}".repeat(2000))),
    ];
    for (label, text) in &cases {
        let r = timed(label, || Manifest::parse(text));
        if let Ok(m) = r {
            // whatever is accepted may not carry more than it was granted to ask for
            assert!(m.allowed_hosts().len() <= 1000, "{label}");
            for h in m.allowed_hosts() {
                assert!(
                    h.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_'),
                    "{label}: host {h:?}"
                );
                assert!(
                    !h.contains('*') && !h.contains('@') && !h.contains('/'),
                    "{label}: {h:?}"
                );
            }
            let lim = m.limits;
            assert!(
                lim.memory_pages <= 65_536,
                "{label}: {} pages",
                lim.memory_pages
            );
        }
    }
    for must_fail in [
        "empty",
        "garbage",
        "path-id",
        "slash-id",
        "negative-api",
        "huge-api",
        "duplicate-keys",
        "wildcard-host",
        "scheme-host",
        "path-host",
        "userinfo-host",
        "unknown-permission",
        "wrong-types",
    ] {
        let text = &cases.iter().find(|(l, _)| *l == must_fail).unwrap().1;
        assert!(
            Manifest::parse(text).is_err(),
            "{must_fail} must be refused"
        );
    }
}

#[test]
fn urls_cannot_smuggle_another_host_past_the_permission_check() {
    for (url, expect) in [
        ("https://a.com/x", Some("a.com")),
        ("HTTPS://a.com/x", None), // scheme match is exact: not accepted, never widened
        ("https://a.com:8443/x", Some("a.com")),
        ("https://A.COM/x", Some("a.com")),
        ("https://a.com?x=@evil.com", Some("a.com")),
        ("https://a.com#@evil.com", Some("a.com")),
    ] {
        assert_eq!(url_host(url).as_deref(), expect, "{url}");
    }
    for url in [
        "https://a.com@evil.com/",
        "https://a.com:pw@evil.com/",
        "https://evil.com\\@a.com/",
        "https://a.com\\.evil.com/",
        "https://a.com%2eevil.com/",
        "https://a.com\u{ff0e}evil.com/",
        "https://[::1]/",
        "https://a.com:99999999999/",
        "https://a.com:/",
        "https://a.com:80:80/",
        "https:///etc/passwd",
        "https://",
        "https://a com/",
        "https://a.com\n.evil.com/",
        "https://a.com\0.evil.com/",
        "https://\u{202e}moc.a/",
        "file:///etc/passwd",
        "ftp://a.com/",
        "data:text/plain,a.com",
        "javascript:a.com",
        "//a.com/x",
        " https://a.com/",
        "",
    ] {
        assert_eq!(url_host(url), None, "{url:?} must not yield a host");
    }
}

fn wasm_cases() -> Vec<(&'static str, Vec<u8>)> {
    let head = b"\0asm\x01\0\0\0".to_vec();
    let with = |tail: &[u8]| {
        let mut v = head.clone();
        v.extend_from_slice(tail);
        v
    };
    let valid = wat::parse_str(r#"(module (memory (export "memory") 1) (func (export "alloc") (param i32) (result i32) i32.const 0))"#).unwrap();
    let mut cases = vec![
        ("empty", vec![]),
        ("magic-only", b"\0asm".to_vec()),
        ("bad-version", b"\0asm\xff\xff\xff\xff".to_vec()),
        ("text", b"(module)".to_vec()),
        ("huge-section", with(&[1, 0xff, 0xff, 0xff, 0xff, 0x0f])),
        (
            "huge-custom",
            with(&[0, 0xff, 0xff, 0xff, 0xff, 0x0f, 1, b'x']),
        ),
        ("memory-4gb", with(&[5, 3, 1, 0, 0xff, 0xff, 0x03])),
        ("memory-max", with(&[5, 5, 1, 1, 0, 0xff, 0xff, 0x03])),
        ("many-types", {
            let mut s = vec![1u8];
            let mut body = vec![0xff, 0xff, 0x03];
            body.extend(std::iter::repeat_n([0x60u8, 0, 0], 50).flatten());
            s.push(body.len() as u8);
            s.extend(body);
            with(&s)
        }),
        ("random", {
            let mut x = 0x2545f4914f6cdd1du64;
            (0..65_536)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    (x >> 24) as u8
                })
                .collect()
        }),
        ("zeros", vec![0; 1 << 20]),
    ];
    for cut in (0..valid.len()).step_by(3) {
        cases.push(("truncated", valid[..cut].to_vec()));
    }
    for i in (8..valid.len()).step_by(2) {
        let mut b = valid.clone();
        b[i] ^= 0xff;
        cases.push(("flipped", b));
    }
    cases
}

#[test]
fn hostile_wasm_modules_are_install_errors_not_panics() {
    for (label, bytes) in wasm_cases() {
        let r = timed(label, || {
            Plugin::from_bytes(
                manifest("evil.wasm", "transform", ""),
                &bytes,
                Host::default(),
            )
        });
        if let Ok(p) = r {
            // a module that does load is still metered
            let _ = timed(label, || p.call_raw("x", b"{}"));
        }
    }
}

#[test]
fn a_manifest_with_huge_limits_cannot_lift_the_host_caps() {
    let text = format!(
        "{}[limits]\nfuel = 18446744073709551615\nmemory_pages = 4294967295\ntimeout_ms = 18446744073709551615\n",
        manifest_toml("evil.limits", "transform", "")
    );
    match Manifest::parse(&text) {
        Err(_) => {}
        Ok(m) => {
            assert!(
                m.limits.memory_pages <= 65_536,
                "{} pages",
                m.limits.memory_pages
            );
            assert!(
                m.limits.timeout_ms <= 10 * 60 * 1000,
                "{} ms",
                m.limits.timeout_ms
            );
        }
    }
}
