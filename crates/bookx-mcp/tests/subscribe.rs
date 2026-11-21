//! `resources/subscribe` and the list_changed / updated notifications (stdio only).

mod common;

use std::time::Duration;

use common::*;
use serde_json::json;

fn config(f: &Fixture) -> bookx_mcp::Config {
    let mut c = f.config();
    c.watch_interval = Duration::from_millis(60);
    c
}

/// The next notification with this method (answers and other messages are skipped).
fn wait_note(c: &mut Client, method: &str) -> serde_json::Value {
    for _ in 0..200 {
        if let Some(i) = c.notes.iter().position(|n| n["method"] == method) {
            return c.notes.remove(i);
        }
        let m = c.recv().expect("the server is still there");
        c.notes.push(m);
    }
    panic!("no {method}");
}

fn edit_title(f: &Fixture, id: u64, title: &str) {
    let mut o = bookx_store::open_library(&f.root).unwrap();
    let mut m = o.library.get(id).unwrap().meta.clone();
    m.title = title.into();
    o.library.update_metadata(id, m).unwrap();
}

#[test]
fn capabilities_say_subscribe_and_list_changed_on_stdio() {
    let f = Fixture::new("sub-caps");
    let mut c = Client::start(config(&f));
    let r = c.request(
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"x","version":"1"}}),
    );
    assert_eq!(r["result"]["capabilities"]["resources"]["subscribe"], true);
    assert_eq!(
        r["result"]["capabilities"]["resources"]["listChanged"],
        true
    );
    c.close();
}

#[test]
fn a_subscribed_book_is_reported_when_it_changes_and_others_are_not() {
    let f = Fixture::new("sub-updated");
    let mut c = Client::ready(config(&f));
    let uri = format!("bookx://book/{}", f.dune);
    let r = c.request("resources/subscribe", json!({"uri": uri}));
    assert_eq!(r["result"], json!({}), "{r}");
    edit_title(&f, f.dune2, "Changed elsewhere");
    edit_title(&f, f.dune, "Renamed Dune");
    let n = wait_note(&mut c, "notifications/resources/updated");
    assert_eq!(n["params"]["uri"], uri);
    // nothing was said about the other book: the only update is for the subscribed one
    std::thread::sleep(Duration::from_millis(250));
    c.request("ping", json!({}));
    assert!(
        !c.notes
            .iter()
            .any(|n| n["method"] == "notifications/resources/updated"),
        "{:?}",
        c.notes
    );
    // after unsubscribing, further changes are silent
    assert_eq!(
        c.request("resources/unsubscribe", json!({"uri": uri}))["result"],
        json!({})
    );
    edit_title(&f, f.dune, "Renamed again");
    let r = c.request("ping", json!({}));
    assert_eq!(r["result"], json!({}));
    std::thread::sleep(Duration::from_millis(250));
    let r = c.request("ping", json!({}));
    assert_eq!(r["result"], json!({}));
    assert!(!c
        .notes
        .iter()
        .any(|n| n["method"] == "notifications/resources/updated"));
    c.close();
}

#[test]
fn a_removed_book_changes_the_list() {
    let f = Fixture::new("sub-list");
    let mut c = Client::ready(config(&f));
    {
        let mut o = bookx_store::open_library(&f.root).unwrap();
        o.library.remove(&[f.dune2], false).unwrap();
    }
    wait_note(&mut c, "notifications/resources/list_changed");
    c.close();
}

#[test]
fn bad_subscriptions_are_errors() {
    let f = Fixture::new("sub-bad");
    let mut c = Client::ready(config(&f));
    let r = c.request(
        "resources/subscribe",
        json!({"uri": "http://example.com/x"}),
    );
    assert_eq!(r["error"]["code"], -32002, "{r}");
    let r = c.request("resources/subscribe", json!({}));
    assert_eq!(r["error"]["code"], -32602, "{r}");
    for i in 0..bookx_mcp::server::MAX_SUBSCRIPTIONS {
        let r = c.request(
            "resources/subscribe",
            json!({"uri": format!("bookx://book/{i}")}),
        );
        assert_eq!(r["result"], json!({}));
    }
    let r = c.request("resources/subscribe", json!({"uri": "bookx://book/99999"}));
    assert_eq!(r["error"]["code"], -32602, "{r}");
    // an existing subscription can be repeated at the limit
    let r = c.request("resources/subscribe", json!({"uri": "bookx://book/0"}));
    assert_eq!(r["result"], json!({}));
    c.close();
}
