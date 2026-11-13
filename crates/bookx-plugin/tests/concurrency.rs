//! Many threads on one plugin, and resource behaviour over many calls.

mod common;

use std::sync::Arc;
use std::thread;

use bookx_plugin::Host;
use common::*;
use serde_json::json;

#[test]
fn eight_threads_share_one_plugin() {
    let p = Arc::new(plugin("transform", "", &echo()));
    let handles: Vec<_> = (0..8)
        .map(|t| {
            let p = p.clone();
            thread::spawn(move || {
                for i in 0..100 {
                    let input = json!({"thread": t, "i": i, "text": format!("ünï-{t}-{i}")});
                    let out = p.call_json("echo", &input).unwrap();
                    assert_eq!(out, input);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
}

#[test]
fn failures_in_some_threads_do_not_affect_others() {
    let good = Arc::new(plugin("transform", "", &echo()));
    let bad = Arc::new(plugin("transform", "fuel = 100000", &infinite_loop()));
    let trap = Arc::new(plugin("transform", "", &trapper()));
    let mut hs = Vec::new();
    for t in 0..8 {
        let (good, bad, trap) = (good.clone(), bad.clone(), trap.clone());
        hs.push(thread::spawn(move || {
            for i in 0..40 {
                match (t + i) % 3 {
                    0 => assert!(bad.call_raw("x", b"").is_err()),
                    1 => assert!(trap.call_raw("x", b"").is_err()),
                    _ => {
                        let v = json!({"t": t, "i": i});
                        assert_eq!(good.call_json("e", &v).unwrap(), v);
                    }
                }
            }
        }));
    }
    for h in hs {
        h.join().unwrap();
    }
}

#[test]
fn threads_share_a_host_network_safely() {
    let net = MockNet::replying(br#"{"ok":1}"#);
    let host = Host::default().with_network(net.clone());
    let p = Arc::new(plugin_host(
        "transform",
        "permissions = [\"network:a.org\"]",
        &http_client(),
        host,
    ));
    let hs: Vec<_> = (0..8)
        .map(|t| {
            let p = p.clone();
            thread::spawn(move || {
                for i in 0..25 {
                    let url = format!("https://a.org/{t}/{i}");
                    let (raw, _) = p.call_raw("get", url.as_bytes()).unwrap();
                    assert_eq!(raw, br#"{"ok":1}"#);
                }
            })
        })
        .collect();
    for h in hs {
        h.join().unwrap();
    }
    assert_eq!(net.urls().len(), 200);
}

#[test]
fn plugin_is_send_and_sync() {
    fn assert_ss<T: Send + Sync>() {}
    assert_ss::<bookx_plugin::Plugin>();
    assert_ss::<bookx_plugin::PluginMetadataSource>();
    assert_ss::<bookx_plugin::PluginReader>();
    assert_ss::<bookx_plugin::PluginWriter>();
    assert_ss::<bookx_plugin::PluginTransform>();
}
