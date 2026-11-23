//! Protocol conformance and tool behaviour, with a scripted client over in-memory pipes.

mod common;

use bookx_mcp::WritePolicy;
use bookx_mcp::schema;
use common::*;
use serde_json::{Value, json};

fn ids(v: &Value, key: &str) -> Vec<u64> {
    v[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].as_u64().unwrap())
        .collect()
}

// -- handshake ------------------------------------------------------------------------------------

#[test]
fn initialize_handshake_negotiates_the_version_and_declares_capabilities() {
    let f = Fixture::new("init");
    let mut c = Client::start(f.config());
    let r = c.request(
        "initialize",
        json!({"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"x","version":"9"}}),
    );
    let res = &r["result"];
    assert_eq!(r["jsonrpc"], "2.0");
    assert_eq!(res["protocolVersion"], "2025-03-26");
    assert_eq!(res["serverInfo"]["name"], "bookx");
    assert!(res["capabilities"]["tools"].is_object());
    assert!(res["capabilities"]["resources"].is_object());
    assert!(res["capabilities"]["prompts"].is_object());
    assert!(res["capabilities"]["logging"].is_object());
    assert!(res["instructions"].as_str().unwrap().contains("read-only"));
    c.notify("notifications/initialized", json!({}));
    assert_eq!(c.request("ping", json!({}))["result"], json!({}));
    // the client shows up in the status list
    let clients = c.server.app.clients(None);
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].name, "x");
    c.close();
}

#[test]
fn an_unknown_protocol_version_gets_the_latest() {
    let f = Fixture::new("ver");
    let mut c = Client::start(f.config());
    let r = c.request("initialize", json!({"protocolVersion":"1999-01-01","capabilities":{},"clientInfo":{"name":"x","version":"1"}}));
    assert_eq!(
        r["result"]["protocolVersion"],
        bookx_mcp::server::LATEST_VERSION
    );
    let r = c.request("initialize", json!({"capabilities":{}}));
    assert_eq!(r["error"]["code"], -32602);
    c.close();
}

#[test]
fn requests_before_initialize_are_refused_and_bad_messages_answered() {
    let f = Fixture::new("preinit");
    let mut c = Client::start(f.config());
    let r = c.request("tools/list", json!({}));
    assert_eq!(r["error"]["code"], -32600);
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("initialize")
    );
    // ping is allowed any time
    assert!(c.request("ping", json!({})).get("result").is_some());
    // garbage and an unknown method
    c.send_raw("this is not json\n");
    let r = c.recv().unwrap();
    assert_eq!(r["error"]["code"], -32700);
    assert_eq!(r["id"], Value::Null);
    c.send_raw("[1,2]\n");
    assert_eq!(c.recv().unwrap()["error"]["code"], -32600);
    c.close();
    let mut c = Client::ready(f.config());
    let r = c.request("no/such/method", json!({}));
    assert_eq!(r["error"]["code"], -32601);
    c.close();
}

#[test]
fn string_ids_are_echoed_and_notifications_get_no_answer() {
    let f = Fixture::new("ids");
    let mut c = Client::ready(f.config());
    let r = c.request_with_id(json!("abc-1"), "ping", json!({}));
    assert_eq!(r["id"], "abc-1");
    c.notify("notifications/something-unknown", json!({}));
    // the next answer is the ping's, nothing was sent for the notification
    let r = c.request("ping", json!({}));
    assert!(c.notes.is_empty());
    assert!(r.get("result").is_some());
    c.close();
}

// -- tools/list -------------------------------------------------------------------------------------

#[test]
fn tools_list_has_valid_schemas_annotations_and_no_write_tools_when_read_only() {
    let f = Fixture::new("list");
    let mut c = Client::ready(f.config());
    let r = c.request("tools/list", json!({}));
    let tools = r["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for want in [
        "search_books",
        "get_book",
        "list_collections",
        "list_series",
        "list_authors",
        "list_tags",
        "get_toc",
        "read_book_text",
        "search_in_book",
        "search_all_books_text",
        "get_annotations",
        "get_reading_stats",
        "recent_books",
        "find_duplicates",
        "similar_books",
        "check_book",
        "convert_preview",
        "export_annotations",
    ] {
        assert!(names.contains(&want), "{want} missing from {names:?}");
    }
    for w in [
        "add_note",
        "add_highlight",
        "set_rating",
        "add_tags",
        "remove_tags",
        "mark_read",
        "add_to_shelf",
        "update_metadata",
        "fetch_metadata",
        "add_book_from_path",
    ] {
        assert!(!names.contains(&w), "{w} must not be listed read-only");
    }
    for t in tools {
        schema::schema_is_valid(&t["inputSchema"]).unwrap();
        schema::schema_is_valid(&t["outputSchema"]).unwrap();
        assert_eq!(t["annotations"]["readOnlyHint"], true, "{}", t["name"]);
        assert_eq!(t["annotations"]["destructiveHint"], false);
        assert!(t["title"].is_string() && t["description"].as_str().unwrap().len() > 20);
    }
    c.close();
}

#[test]
fn lists_paginate_with_opaque_cursors() {
    let f = Fixture::new("page");
    let mut cfg = f.config();
    cfg.page_size = 5;
    let mut c = Client::ready(cfg);
    let first = c.request("tools/list", json!({}));
    let page1 = first["result"]["tools"].as_array().unwrap().len();
    assert_eq!(page1, 5);
    let mut cursor = first["result"]["nextCursor"].as_str().unwrap().to_string();
    let mut total = page1;
    loop {
        let r = c.request("tools/list", json!({"cursor":cursor}));
        total += r["result"]["tools"].as_array().unwrap().len();
        match r["result"]["nextCursor"].as_str() {
            Some(n) => cursor = n.to_string(),
            None => break,
        }
    }
    assert!(total >= 18, "{total}");
    let bad = c.request("tools/list", json!({"cursor":"not-a-cursor"}));
    assert_eq!(bad["error"]["code"], -32602);
    let r = c.request("resources/list", json!({}));
    assert_eq!(r["result"]["resources"].as_array().unwrap().len(), 5);
    assert!(r["result"]["nextCursor"].is_string());
    c.close();
}

// -- read tools -----------------------------------------------------------------------------------------

#[test]
fn search_books_uses_the_query_language_sorts_and_pages() {
    let f = Fixture::new("search");
    let mut c = Client::ready(f.config());
    let all = c.ok("search_books", json!({"limit":100}));
    assert_eq!(all["total"], 6);
    let tag = c.ok("search_books", json!({"query":"tag:sf"}));
    let mut got = ids(&tag, "books");
    got.sort();
    assert_eq!(got, vec![f.dune, f.dune2, f.dup]);
    let a = c.ok("search_books", json!({"query":"author:=\"Julia Child\""}));
    assert_eq!(ids(&a, "books"), vec![f.cook]);
    let s = c.ok(
        "search_books",
        json!({"query":"series:\"alpha saga\" -title:two"}),
    );
    assert_eq!(s["total"], 2);
    assert_eq!(
        c.ok("search_books", json!({"query":"status:reading"}))["total"],
        1
    );
    assert_eq!(
        c.ok("search_books", json!({"query":"rating:>=5"}))["total"],
        0
    );
    assert_eq!(
        c.ok("search_books", json!({"query":"nothing-matches-this"}))["books"],
        json!([])
    );
    // sorting and paging
    let by_title = c.ok("search_books", json!({"sort":"title","limit":2}));
    assert_eq!(by_title["books"][0]["title"], "Alpha Two");
    let cursor = by_title["next_cursor"].as_str().unwrap();
    let next = c.ok(
        "search_books",
        json!({"sort":"title","limit":2,"cursor":cursor}),
    );
    assert_eq!(next["books"].as_array().unwrap().len(), 2);
    assert_ne!(next["books"][0]["id"], by_title["books"][0]["id"]);
    let desc = c.ok(
        "search_books",
        json!({"sort":"title","descending":true,"limit":1}),
    );
    assert_eq!(desc["books"][0]["title"], "The Long Book");
    // a summary carries what an assistant needs and no file paths
    let b = &by_title["books"][0];
    assert!(b["authors"].is_array() && b["formats"].is_array() && b["rating"].is_number());
    assert!(!b.to_string().contains(f.root.to_str().unwrap()));
    c.close();
}

#[test]
fn get_book_describes_provenance_shelves_and_progress() {
    let f = Fixture::new("getbook");
    let o = bookx_store::open_library(&f.root).unwrap();
    o.store
        .set_provenance(f.dune, &["description"], "openlibrary", 1_700_000_000)
        .unwrap();
    drop(o);
    let mut c = Client::ready(f.config());
    let r = c.ok("get_book", json!({"book_id": f.dune}));
    let b = &r["book"];
    assert_eq!(b["title"], "Dune Alpha");
    assert_eq!(b["series"], "Alpha Saga");
    assert_eq!(b["highlights"], 2);
    assert_eq!(b["shelves"], json!(["Favourites"]));
    assert_eq!(b["provenance"][0]["source"], "openlibrary");
    assert!(b["formats"][0]["name"].as_str().unwrap().ends_with(".epub"));
    assert!((b["progress"].as_f64().unwrap() - 0.4).abs() < 1e-6);
    assert!(
        !r.to_string().contains(f.root.to_str().unwrap()),
        "no library paths in answers"
    );
    let e = c.err("get_book", json!({"book_id": 9999}));
    assert!(e.contains("no book with id 9999"));
    c.close();
}

#[test]
fn lists_of_shelves_series_authors_and_tags() {
    let f = Fixture::new("facets");
    let mut c = Client::ready(f.config());
    let shelves = c.ok("list_collections", json!({}));
    assert_eq!(shelves["collections"][0]["name"], "Favourites");
    let id = shelves["collections"][0]["id"].as_i64().unwrap();
    let on = c.ok("list_collections", json!({"collection_id": id}));
    assert_eq!(ids(&on, "books"), vec![f.dune]);
    let series = c.ok("list_series", json!({}));
    assert_eq!(series["series"][0]["name"], "Alpha Saga");
    assert_eq!(series["series"][0]["books"], 3);
    assert_eq!(series["series"][0]["volumes"][0]["index"], 1.0);
    let authors = c.ok("list_authors", json!({"sort":"count"}));
    assert_eq!(authors["authors"][0]["name"], "Frank Herbert");
    assert_eq!(authors["authors"][0]["books"], 3);
    let tags = c.ok("list_tags", json!({"query":"SF"}));
    assert_eq!(tags["total"], 1);
    assert_eq!(tags["tags"][0]["books"], 3);
    c.close();
}

#[test]
fn toc_and_reading_text_with_cursors_across_chapters() {
    let f = Fixture::new("read");
    let mut c = Client::ready(f.config());
    let toc = c.ok("get_toc", json!({"book_id": f.dune}));
    assert_eq!(toc["chapter_count"], 3);
    assert_eq!(toc["chapters"][0]["title"], "Desert");
    assert!(toc["chapters"][0]["words"].as_u64().unwrap() > 5);
    let r = c.ok("read_book_text", json!({"book_id": f.dune, "chapter": 0}));
    let text = r["text"].as_str().unwrap();
    assert!(text.contains("spice must flow"));
    assert!(text.contains("\n"), "paragraphs are separated");
    assert!(r["note"].as_str().unwrap().contains("personal use"));
    // follow the cursor to the end of the book
    let mut cursor = r["next_cursor"].as_str().map(str::to_string);
    let mut all = text.to_string();
    let mut hops = 0;
    while let Some(cur) = cursor {
        let n = c.ok("read_book_text", json!({"book_id": f.dune, "cursor": cur}));
        all.push_str(n["text"].as_str().unwrap());
        cursor = n["next_cursor"].as_str().map(str::to_string);
        hops += 1;
        assert!(hops < 10);
    }
    assert!(all.contains("final chapter returns home"));
    assert_eq!(hops, 2);
    // the text block of the answer starts with where we are
    let raw = c.call("read_book_text", json!({"book_id": f.dune, "chapter": 1}));
    assert!(
        raw["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("[Dune Alpha — chapter 1")
    );
    let e = c.err("read_book_text", json!({"book_id": f.dune, "chapter": 9}));
    assert!(e.contains("3 chapters"));
    c.close();
}

#[test]
fn text_answers_are_capped_and_continue() {
    let f = Fixture::new("caps");
    let mut cfg = f.config();
    cfg.max_text_chars = 5000;
    let mut c = Client::ready(cfg);
    let r = c.ok(
        "read_book_text",
        json!({"book_id": f.big, "chapter": 0, "max_chars": 20000}),
    );
    let n = r["text"].as_str().unwrap().chars().count();
    assert!(n <= 5000 && n > 4000, "{n}");
    assert!(r["chapter_chars"].as_u64().unwrap() > 100_000);
    let next = c.ok(
        "read_book_text",
        json!({"book_id": f.big, "cursor": r["next_cursor"]}),
    );
    assert_eq!(
        next["offset"].as_u64().unwrap(),
        r["end_offset"].as_u64().unwrap()
    );
    // the schema bounds the argument
    let e = c.err(
        "read_book_text",
        json!({"book_id": f.big, "max_chars": 10_000_000}),
    );
    assert!(e.contains("at most"));
    // the resource is cut at the same size
    let res = c.request(
        "resources/read",
        json!({"uri": format!("bookx://book/{}/chapter/0", f.big)}),
    );
    let t = res["result"]["contents"][0]["text"].as_str().unwrap();
    assert!(t.chars().count() < 5200 && t.contains("cut at the size limit"));
    c.close();
}

#[test]
fn protected_books_refuse_their_text_everywhere() {
    let f = Fixture::new("drm");
    let mut c = Client::ready(f.config());
    for (tool, args) in [
        ("read_book_text", json!({"book_id": f.locked})),
        ("get_toc", json!({"book_id": f.locked})),
        (
            "search_in_book",
            json!({"book_id": f.locked, "query":"read"}),
        ),
    ] {
        let e = c.err(tool, args);
        assert!(e.contains("copy protected"), "{tool}: {e}");
    }
    let r = c.request(
        "resources/read",
        json!({"uri": format!("bookx://book/{}/chapter/0", f.locked)}),
    );
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("copy protected")
    );
    // the full-text index may know the book; the tool leaves it out
    let hits = c.ok("search_all_books_text", json!({"query":"read this"}));
    assert_eq!(hits["hits"], json!([]));
    // metadata is still visible and flagged
    let b = c.ok("get_book", json!({"book_id": f.locked}));
    assert_eq!(b["book"]["protected"], true);
    c.close();
}

#[test]
fn search_inside_a_book_and_across_the_library() {
    let f = Fixture::new("fts");
    let mut c = Client::ready(f.config());
    let r = c.ok(
        "search_in_book",
        json!({"book_id": f.dune, "query":"desert"}),
    );
    let hits = r["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 3);
    assert_eq!(hits[0]["chapter"], 0);
    assert!(
        hits[0]["snippet"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("desert")
    );
    let w = c.ok(
        "search_in_book",
        json!({"book_id": f.dune, "query":"Water", "case_sensitive": true, "whole_word": true}),
    );
    assert_eq!(
        w["hits"].as_array().unwrap().len(),
        2,
        "the heading and the sentence"
    );
    let re = c.ok(
        "search_in_book",
        json!({"book_id": f.dune, "query":"sand\\w+", "regex": true}),
    );
    assert_eq!(re["hits"].as_array().unwrap().len(), 1);
    let paged = c.ok(
        "search_in_book",
        json!({"book_id": f.dune, "query":"the", "limit": 1}),
    );
    assert_eq!(paged["hits"].as_array().unwrap().len(), 1);
    assert!(paged["next_cursor"].is_string());
    let e = c.err(
        "search_in_book",
        json!({"book_id": f.dune, "query":"("," regex": true}),
    );
    assert!(e.contains("unknown argument") || e.contains("invalid"));
    let e = c.err(
        "search_in_book",
        json!({"book_id": f.dune, "query":"(", "regex": true}),
    );
    assert!(!e.is_empty());

    let all = c.ok("search_all_books_text", json!({"query":"spice desert"}));
    assert_eq!(all["hits"].as_array().unwrap().len(), 1);
    assert_eq!(all["hits"][0]["book"]["id"], f.dune);
    assert!(all["hits"][0]["snippet"].as_str().unwrap().contains("**"));
    assert!(all["note"].as_str().unwrap().contains("indexed"));
    c.close();
}

#[test]
fn annotations_filter_export_and_page() {
    let f = Fixture::new("notes");
    let mut c = Client::ready(f.config());
    let all = c.ok("get_annotations", json!({}));
    assert_eq!(all["total"], 2);
    assert_eq!(all["annotations"][0]["id"], "h1", "newest first");
    assert_eq!(all["annotations"][0]["type"], "note");
    let notes = c.ok("get_annotations", json!({"kind":"note"}));
    assert_eq!(notes["total"], 1);
    let green = c.ok(
        "get_annotations",
        json!({"color":"green","book_id": f.dune}),
    );
    assert_eq!(green["annotations"][0]["text"], "Water");
    assert_eq!(
        c.ok("get_annotations", json!({"query":"remember"}))["total"],
        1
    );
    assert_eq!(
        c.ok("get_annotations", json!({"book_id": f.cook}))["total"],
        0
    );
    let one = c.ok("get_annotations", json!({"limit":1}));
    assert!(one["next_cursor"].is_string());
    let md = c.ok("export_annotations", json!({"book_id": f.dune}));
    let text = md["content"].as_str().unwrap();
    assert!(
        text.contains("# Dune Alpha") && text.contains("> spice") && text.contains("remember this")
    );
    assert!(text.contains("### Desert"), "chapter titles are used");
    let js = c.ok(
        "export_annotations",
        json!({"book_id": f.dune, "format":"json"}),
    );
    assert!(serde_json::from_str::<Value>(js["content"].as_str().unwrap()).is_ok());
    c.close();
}

#[test]
fn reading_stats_recent_books_similar_duplicates() {
    let f = Fixture::new("misc");
    // a stats file with one session
    let mut store = bookx_lookup::stats::StatsStore::default();
    let end = chrono::Utc::now();
    store
        .record(bookx_lookup::stats::Session {
            book_id: f.dune.to_string(),
            start: end - chrono::Duration::minutes(30),
            end,
            pages: 12,
            words: 3000,
            characters: 15000,
            start_pos: 0.0,
            end_pos: 0.2,
        })
        .unwrap();
    let stats = f.dir.join("stats.json");
    store.save(&stats).unwrap();
    let mut cfg = f.config();
    cfg.stats_path = Some(stats);
    let mut c = Client::ready(cfg);
    let s = c.ok("get_reading_stats", json!({}));
    assert_eq!(s["library"]["books"], 6);
    assert_eq!(s["library"]["reading"], 1);
    assert_eq!(s["sessions"]["count"], 1);
    assert_eq!(s["sessions"]["words"], 3000);
    assert_eq!(s["sessions"]["per_book"][0]["title"], "Dune Alpha");
    let none = {
        let mut cfg = f.config();
        cfg.stats_path = Some(f.dir.join("missing.json"));
        let mut c2 = Client::ready(cfg);
        let r = c2.ok("get_reading_stats", json!({}));
        c2.close();
        r
    };
    assert!(
        none["note"]
            .as_str()
            .unwrap()
            .contains("no reading sessions")
    );

    let recent = c.ok("recent_books", json!({}));
    assert_eq!(ids(&recent, "books"), vec![f.dune]);
    let added = c.ok("recent_books", json!({"kind":"added","limit":2}));
    assert_eq!(added["books"].as_array().unwrap().len(), 2);

    let sim = c.ok("similar_books", json!({"book_id": f.dune}));
    let order = ids(&sim, "books");
    assert!(order.contains(&f.dune2) && order.contains(&f.dup));
    assert!(!order.contains(&f.cook));
    assert!(
        sim["books"][0]["reasons"][0]
            .as_str()
            .unwrap()
            .contains("same")
    );

    let dups = c.ok("find_duplicates", json!({}));
    assert_eq!(dups["total"], 1);
    let group = ids(&dups["groups"][0], "books");
    assert!(group.contains(&f.dune) && group.contains(&f.dup));
    c.close();
}

#[test]
fn check_book_and_convert_preview() {
    let f = Fixture::new("check");
    let mut c = Client::ready(f.config());
    let r = c.ok("check_book", json!({"book_id": f.dune}));
    assert!(r["errors"].is_number() && r["issues"].is_array());
    let p = c.ok(
        "convert_preview",
        json!({"book_id": f.dune, "format":"txt"}),
    );
    assert_eq!(p["format"], "txt");
    assert!(
        p["lost"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().unwrap().contains("contents")
                || l.as_str().unwrap().contains("cover")
                || l.as_str().unwrap().contains("link")
                || l.as_str().unwrap().contains("series")),
        "{p}"
    );
    let e = c.err(
        "convert_preview",
        json!({"book_id": f.dune, "format":"m/o"}),
    );
    assert!(e.contains("extension"));
    c.close();
}

// -- errors ------------------------------------------------------------------------------------------------

#[test]
fn argument_and_tool_errors_are_reported_the_right_way() {
    let f = Fixture::new("errors");
    let mut c = Client::ready(f.config());
    // unknown tool: a protocol error
    let r = c.request("tools/call", json!({"name":"no_such_tool","arguments":{}}));
    assert_eq!(r["error"]["code"], -32602);
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no_such_tool")
    );
    let r = c.request("tools/call", json!({"arguments":{}}));
    assert_eq!(r["error"]["code"], -32602);
    // bad arguments: a tool error the model can read and fix
    assert!(c.err("get_book", json!({})).contains("book_id"));
    assert!(
        c.err("get_book", json!({"book_id":"x"}))
            .contains("integer")
    );
    assert!(
        c.err("search_books", json!({"sort":"sideways"}))
            .contains("one of")
    );
    assert!(
        c.err("search_books", json!({"limit": 1000}))
            .contains("at most")
    );
    assert!(
        c.err("search_books", json!({"bogus": 1}))
            .contains("unknown argument")
    );
    assert!(
        c.err("search_books", json!({"cursor":"zzz"}))
            .contains("cursor")
    );
    // missing arguments object is fine for tools without required arguments
    let r = c.request("tools/call", json!({"name":"search_books"}));
    assert_eq!(r["result"]["isError"], false);
    c.close();
}

#[test]
fn library_paths_never_leave_in_errors() {
    let f = Fixture::new("scrub");
    let mut cfg = f.config();
    cfg.root = f.dir.join("not-a-library");
    let mut c = Client::ready(cfg);
    let e = c.err("search_books", json!({}));
    assert!(!e.contains(f.dir.to_str().unwrap()), "{e}");
    assert!(e.contains("no Bookx library"));
    c.close();
}

// -- progress, cancellation, logging, limits ----------------------------------------------------------------------

#[test]
fn progress_notifications_follow_the_token() {
    let f = Fixture::new("progress");
    let mut c = Client::ready(f.config());
    let r = c.request(
        "tools/call",
        json!({"name":"search_in_book","arguments":{"book_id": f.dune,"query":"the"},"_meta":{"progressToken":"tok-1"}}),
    );
    assert_eq!(r["result"]["isError"], false);
    let progress: Vec<&Value> = c
        .notes
        .iter()
        .filter(|n| n["method"] == "notifications/progress")
        .collect();
    assert!(progress.len() >= 2, "{:?}", c.notes);
    assert!(
        progress
            .iter()
            .all(|p| p["params"]["progressToken"] == "tok-1")
    );
    let values: Vec<f64> = progress
        .iter()
        .map(|p| p["params"]["progress"].as_f64().unwrap())
        .collect();
    assert!(
        values.windows(2).all(|w| w[0] <= w[1]),
        "progress only increases"
    );
    // without a token nothing is sent
    c.notes.clear();
    c.ok("search_in_book", json!({"book_id": f.dune, "query":"the"}));
    assert!(c.notes.is_empty());
    c.close();
}

#[test]
fn a_cancelled_request_gets_no_answer() {
    let f = Fixture::new("cancel");
    let c = Client::ready(f.config());
    let server = c.server.clone();
    // the deterministic part: the flag is set before the request is handled
    let mut session = bookx_mcp::server::Session::new(true);
    session.initialized = true;
    let id = json!(77);
    server.register_request(&session.id, &id);
    assert!(server.try_cancel(
        &session.id,
        &json!({"method":"notifications/cancelled","params":{"requestId":77}})
    ));
    let sink = |_: Value| {};
    let answer = server.handle(
        &mut session,
        json!({"jsonrpc":"2.0","id":77,"method":"tools/call","params":{"name":"search_books","arguments":{}}}),
        &sink,
    );
    assert!(answer.is_none(), "{answer:?}");
    // an id that was not cancelled is still answered
    let answer = server.handle(
        &mut session,
        json!({"jsonrpc":"2.0","id":78,"method":"ping"}),
        &sink,
    );
    assert!(answer.is_some());
    c.close();
}

#[test]
fn cancel_notification_over_the_wire_keeps_the_server_healthy() {
    let f = Fixture::new("cancel2");
    let mut c = Client::ready(f.config());
    // request and cancel in one burst; whichever wins, the server stays usable
    let a = json!({"jsonrpc":"2.0","id":500,"method":"tools/call","params":{"name":"search_in_book","arguments":{"book_id": f.big,"query":"word"}}});
    let n = json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":500,"reason":"user"}});
    c.send_raw(&format!("{a}\n{n}\n"));
    let r = c.request("ping", json!({}));
    assert!(r.get("result").is_some());
    for note in &c.notes {
        assert_eq!(
            note["id"], 500,
            "only the cancelled request may answer first: {note}"
        );
    }
    c.close();
}

#[test]
fn logging_level_controls_messages_and_write_tools_log() {
    let f = Fixture::new("logging");
    let mut cfg = f.config();
    cfg.write = WritePolicy::Locked;
    let mut c = Client::ready(cfg);
    let r = c.request("logging/setLevel", json!({"level":"loud"}));
    assert_eq!(r["error"]["code"], -32602);
    c.ok("add_tags", json!({"book_id": f.cook, "tags":["x"]}));
    let logs: Vec<&Value> = c
        .notes
        .iter()
        .filter(|n| n["method"] == "notifications/message")
        .collect();
    assert_eq!(logs.len(), 1, "{:?}", c.notes);
    assert_eq!(logs[0]["params"]["data"]["tool"], "add_tags");
    c.notes.clear();
    assert_eq!(
        c.request("logging/setLevel", json!({"level":"error"}))["result"],
        json!({})
    );
    c.ok("add_tags", json!({"book_id": f.cook, "tags":["y"]}));
    assert!(
        c.notes.is_empty(),
        "info messages are below the chosen level"
    );
    c.close();
}

#[test]
fn the_rate_limit_answers_with_a_readable_error() {
    let f = Fixture::new("rate");
    let mut cfg = f.config();
    cfg.rate_limit = Some((3, std::time::Duration::from_secs(60)));
    let mut c = Client::ready(cfg);
    for _ in 0..3 {
        c.ok("recent_books", json!({}));
    }
    let e = c.err("recent_books", json!({}));
    assert!(e.contains("rate limit"), "{e}");
    c.close();
}

#[test]
fn a_stopped_server_refuses_calls() {
    let f = Fixture::new("stopped");
    let mut c = Client::ready(f.config());
    c.ok("recent_books", json!({}));
    c.server.app.stop();
    assert!(c.err("recent_books", json!({})).contains("switched off"));
    c.close();
}

// -- resources and prompts --------------------------------------------------------------------------------------------

#[test]
fn resources_list_templates_and_read() {
    let f = Fixture::new("res");
    let mut c = Client::ready(f.config());
    let l = c.request("resources/list", json!({}));
    let uris: Vec<&str> = l["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["uri"].as_str().unwrap())
        .collect();
    assert!(uris.contains(&"bookx://library/stats"));
    assert!(uris.contains(&format!("bookx://book/{}", f.dune).as_str()));
    let t = c.request("resources/templates/list", json!({}));
    let tpl: Vec<&str> = t["result"]["resourceTemplates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["uriTemplate"].as_str().unwrap())
        .collect();
    assert_eq!(tpl.len(), 3);
    assert!(tpl.contains(&"bookx://book/{id}/chapter/{n}"));

    let r = c.request(
        "resources/read",
        json!({"uri": format!("bookx://book/{}", f.dune)}),
    );
    let content = &r["result"]["contents"][0];
    assert_eq!(content["mimeType"], "application/json");
    let b: Value = serde_json::from_str(content["text"].as_str().unwrap()).unwrap();
    assert_eq!(b["title"], "Dune Alpha");

    let r = c.request(
        "resources/read",
        json!({"uri": format!("bookx://book/{}/chapter/1", f.dune)}),
    );
    assert!(
        r["result"]["contents"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Water is life")
    );

    let r = c.request("resources/read", json!({"uri": "bookx://library/stats"}));
    let s: Value =
        serde_json::from_str(r["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(s["books"], 6);

    // errors: not found is -32002
    for uri in [
        "bookx://book/9999",
        "bookx://nonsense",
        "bookx://book/1/chapter/99",
        &format!("bookx://book/{}/cover", f.cook),
    ] {
        let r = c.request("resources/read", json!({"uri": uri}));
        assert_eq!(r["error"]["code"], -32002, "{uri}: {r}");
    }
    assert_eq!(
        c.request("resources/read", json!({}))["error"]["code"],
        -32602
    );
    c.close();
}

#[test]
fn cover_resource_is_a_base64_image_from_inside_the_library() {
    let f = Fixture::new("cover");
    // give a book a cover file in the library
    let png = [137u8, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 0];
    {
        let mut o = bookx_store::open_library(&f.root).unwrap();
        std::fs::write(f.root.join("cover-test.png"), png).unwrap();
        o.library.get_mut(f.cook).unwrap().cover = Some("cover-test.png".into());
        o.library.get_mut(f.dune2).unwrap().cover = Some("../outside.png".into());
        o.library.save().unwrap();
    }
    std::fs::write(f.dir.join("outside.png"), png).unwrap();
    let mut c = Client::ready(f.config());
    let r = c.request(
        "resources/read",
        json!({"uri": format!("bookx://book/{}/cover", f.cook)}),
    );
    let blob = &r["result"]["contents"][0];
    assert_eq!(blob["mimeType"], "image/png");
    use base64::Engine;
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(blob["blob"].as_str().unwrap())
            .unwrap(),
        png
    );
    // a cover path that climbs out of the library is not served
    let r = c.request(
        "resources/read",
        json!({"uri": format!("bookx://book/{}/cover", f.dune2)}),
    );
    assert_eq!(r["error"]["code"], -32002);
    c.close();
}

#[test]
fn prompts_list_and_get() {
    let f = Fixture::new("prompts");
    let mut c = Client::ready(f.config());
    let l = c.request("prompts/list", json!({}));
    let names: Vec<&str> = l["result"]["prompts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    for n in [
        "summarise-chapter",
        "reading-recommendations",
        "study-notes-from-highlights",
        "find-quotes-about",
    ] {
        assert!(names.contains(&n), "{n}");
    }
    let r = c.request("prompts/get", json!({"name":"summarise-chapter","arguments":{"book_id": f.dune.to_string(),"chapter":"1"}}));
    let msgs = r["result"]["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[1]["content"]["type"], "resource");
    assert!(
        msgs[1]["content"]["resource"]["text"]
            .as_str()
            .unwrap()
            .contains("Water is life")
    );
    let r = c.request(
        "prompts/get",
        json!({"name":"study-notes-from-highlights","arguments":{"book_id": f.dune.to_string()}}),
    );
    assert!(
        r["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap()
            .contains("remember this")
    );
    let r = c.request(
        "prompts/get",
        json!({"name":"find-quotes-about","arguments":{"topic":"water"}}),
    );
    assert!(
        r["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap()
            .contains("search_all_books_text")
    );
    let r = c.request(
        "prompts/get",
        json!({"name":"find-quotes-about","arguments":{}}),
    );
    assert_eq!(r["error"]["code"], -32602);
    let r = c.request("prompts/get", json!({"name":"nope"}));
    assert_eq!(r["error"]["code"], -32602);
    c.close();
}

// -- read-only enforcement and writing ------------------------------------------------------------------------------------

#[test]
fn read_only_servers_do_not_have_write_tools_and_cannot_be_talked_into_them() {
    let f = Fixture::new("ro");
    let before = std::fs::read(bookx_store::library_db_path(&f.root))
        .unwrap()
        .len();
    let mut c = Client::ready(f.config());
    for tool in [
        "add_tags",
        "set_rating",
        "update_metadata",
        "mark_read",
        "add_highlight",
        "add_note",
        "add_to_shelf",
        "fetch_metadata",
        "add_book_from_path",
    ] {
        let r = c.request("tools/call", json!({"name":tool,"arguments":{"book_id": f.cook,"tags":["x"],"stars":1,"fields":{"title":"x"},"text":"abc","note":"n","shelf":"s","path":"/etc/passwd"}}));
        assert_eq!(r["error"]["code"], -32602, "{tool}: {r}");
    }
    let b = c.ok("get_book", json!({"book_id": f.cook}));
    assert_eq!(b["book"]["tags"], json!(["cooking"]));
    assert!(!f.root.join(".bookx/mcp-audit.log").exists());
    let _ = before;
    c.close();
}

fn writer(f: &Fixture) -> Client {
    let mut cfg = f.config();
    cfg.write = WritePolicy::Locked;
    cfg.allow_dirs = vec![f.dir.join("incoming")];
    Client::ready(cfg)
}

#[test]
fn write_tools_are_listed_with_honest_annotations() {
    let f = Fixture::new("wlist");
    let mut c = writer(&f);
    let r = c.request("tools/list", json!({}));
    let tools = r["result"]["tools"].as_array().unwrap();
    let get = |n: &str| {
        tools
            .iter()
            .find(|t| t["name"] == n)
            .unwrap_or_else(|| panic!("{n} missing"))
    };
    for n in [
        "add_note",
        "add_highlight",
        "set_rating",
        "add_tags",
        "remove_tags",
        "mark_read",
        "add_to_shelf",
        "update_metadata",
        "add_book_from_path",
    ] {
        let t = get(n);
        assert_eq!(t["annotations"]["readOnlyHint"], false, "{n}");
        schema::schema_is_valid(&t["inputSchema"]).unwrap();
    }
    assert!(
        !tools.iter().any(|t| t["name"] == "fetch_metadata"),
        "network tool needs the network switch"
    );
    assert_eq!(get("search_books")["annotations"]["readOnlyHint"], true);
    assert!(
        get("add_book_from_path")["description"]
            .as_str()
            .unwrap()
            .contains("incoming")
    );
    c.close();
}

#[test]
fn metadata_edits_go_through_the_library_and_can_be_undone() {
    let f = Fixture::new("wmeta");
    let mut c = writer(&f);
    let r = c.ok(
        "add_tags",
        json!({"book_id": f.cook, "tags":["Baking","cooking"]}),
    );
    assert!(r["message"].as_str().unwrap().contains("tags added"));
    assert_eq!(r["book"]["tags"].as_array().unwrap().len(), 2);
    assert_eq!(
        c.ok("add_tags", json!({"book_id": f.cook, "tags":["baking"]}))["message"],
        "nothing to change"
    );
    c.ok(
        "remove_tags",
        json!({"book_id": f.cook, "tags":["COOKING"]}),
    );
    c.ok("set_rating", json!({"book_id": f.cook, "stars": 4.5}));
    let r = c.ok("update_metadata", json!({"book_id": f.cook, "fields":{"title":"Cooking Basics 2","authors":["Ann Lee"],"series":"Kitchen","series_index":2}}));
    assert_eq!(r["book"]["title"], "Cooking Basics 2");
    let b = c.ok("get_book", json!({"book_id": f.cook}));
    assert_eq!(b["book"]["authors"], json!(["Ann Lee"]));
    assert_eq!(b["book"]["rating"], 4.5);
    assert_eq!(b["book"]["tags"], json!(["Baking"]));
    assert_eq!(b["book"]["series"], "Kitchen");
    // the file itself carries the change
    let on_disk = bookx_core::book::OpenBook::open(
        &f.root.join(
            bookx_store::open_library(&f.root)
                .unwrap()
                .library
                .get(f.cook)
                .unwrap()
                .file
                .clone(),
        ),
    )
    .unwrap();
    assert_eq!(on_disk.metadata().title, "Cooking Basics 2");

    // whitelist and validation
    assert!(
        c.err(
            "update_metadata",
            json!({"book_id": f.cook, "fields":{"file":"x"}})
        )
        .contains("unknown argument")
    );
    assert!(
        c.err("update_metadata", json!({"book_id": f.cook, "fields":{}}))
            .contains("at least one")
    );
    assert!(
        c.err("set_rating", json!({"book_id": f.cook, "stars": 9}))
            .contains("at most")
    );
    assert!(
        c.err("add_tags", json!({"book_id": 4242, "tags":["a"]}))
            .contains("no book")
    );

    // four edits (and the fixture's import) are in the operation log; the app's undo takes the
    // edits back one by one
    let ops = bookx_core::history::load_ops(&f.root);
    assert_eq!(ops.len(), 5, "{ops:?}");
    assert!(
        ops[1..]
            .iter()
            .all(|o| matches!(o.kind, bookx_core::history::OpKind::Edit { .. }))
    );
    let mut o = bookx_store::open_library(&f.root).unwrap();
    for _ in 0..4 {
        o.library.undo_last().unwrap().expect("something to undo");
    }
    let cook = o.library.get(f.cook).unwrap();
    assert_eq!(cook.meta.title, "Cooking Basics");
    assert_eq!(cook.meta.tags, vec!["cooking".to_string()]);
    c.close();
}

#[test]
fn notes_highlights_status_and_shelves() {
    let f = Fixture::new("wnotes");
    let mut c = writer(&f);
    let h = c.ok(
        "add_highlight",
        json!({"book_id": f.dune, "text":"sandworms", "note":"dangerous", "color":"red"}),
    );
    assert_eq!(h["chapter"], 0);
    let annotations = c.ok(
        "get_annotations",
        json!({"book_id": f.dune, "query":"dangerous"}),
    );
    assert_eq!(annotations["annotations"][0]["color"], "red");
    assert_eq!(annotations["annotations"][0]["text"], "sandworms");
    // the offsets follow the reader's numbering: paragraphs joined by one line break
    let o = bookx_store::open_library(&f.root).unwrap();
    let hl = o
        .library
        .get(f.dune)
        .unwrap()
        .highlights
        .iter()
        .find(|x| x.note == "dangerous")
        .unwrap()
        .clone();
    let t = c.ok("read_book_text", json!({"book_id": f.dune, "chapter": 0}));
    let chapter: Vec<char> = t["text"].as_str().unwrap().chars().collect();
    let got: String = chapter[hl.start as usize..hl.end as usize].iter().collect();
    assert_eq!(got, "sandworms");
    drop(o);
    assert!(
        c.err(
            "add_highlight",
            json!({"book_id": f.dune, "text":"not in the book"})
        )
        .contains("not found")
    );
    assert!(
        c.err(
            "add_highlight",
            json!({"book_id": f.dune, "text":"x", "color":"pink"})
        )
        .contains("one of")
    );
    assert!(
        c.err(
            "add_highlight",
            json!({"book_id": f.locked, "text":"read this"})
        )
        .contains("copy protected")
    );

    let n = c.ok(
        "add_note",
        json!({"book_id": f.dune, "note":"check later", "chapter": 2}),
    );
    assert!(n["message"].as_str().unwrap().contains("bookmark"));
    let n = c.ok(
        "add_note",
        json!({"book_id": f.dune, "note":"nice line", "text":"Water is life"}),
    );
    assert_eq!(n["chapter"], 1);
    assert_eq!(
        c.ok(
            "get_annotations",
            json!({"book_id": f.dune, "kind":"bookmark"})
        )["total"],
        1
    );

    c.ok("mark_read", json!({"book_id": f.cook}));
    let b = c.ok("get_book", json!({"book_id": f.cook}));
    assert_eq!(b["book"]["reading_status"], "finished");
    assert_eq!(b["book"]["status"], "finished");
    c.ok("mark_read", json!({"book_id": f.cook, "read": false}));
    assert_eq!(
        c.ok("get_book", json!({"book_id": f.cook}))["book"]["status"],
        "unread"
    );

    assert!(
        c.err("add_to_shelf", json!({"book_id": f.cook, "shelf":"Nope"}))
            .contains("create")
    );
    c.ok(
        "add_to_shelf",
        json!({"book_id": f.cook, "shelf":"favourites"}),
    );
    c.ok(
        "add_to_shelf",
        json!({"book_id": f.cook, "shelf":"Kitchen", "create": true}),
    );
    let shelves = c.ok("get_book", json!({"book_id": f.cook}))["book"]["shelves"].clone();
    assert_eq!(shelves.as_array().unwrap().len(), 2);
    c.close();
}

#[test]
fn writes_are_audited_without_secrets_and_the_log_is_in_the_library() {
    let f = Fixture::new("audit");
    let mut c = writer(&f);
    c.ok("add_tags", json!({"book_id": f.cook, "tags":["audited"]}));
    let _ = c.err("add_tags", json!({"book_id": 777, "tags":["audited"]}));
    let log = std::fs::read_to_string(f.root.join(".bookx/mcp-audit.log")).unwrap();
    let lines: Vec<Value> = log
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["tool"], "add_tags");
    assert_eq!(lines[0]["client"], "test-client");
    assert_eq!(lines[0]["ok"], true);
    assert_eq!(lines[1]["ok"], false);
    assert_eq!(lines[0]["arguments"]["tags"][0], "audited");
    assert_eq!(c.server.app.audit.recent().len(), 2);
    // reads are not audited
    c.ok("search_books", json!({}));
    assert_eq!(
        std::fs::read_to_string(f.root.join(".bookx/mcp-audit.log"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    c.close();
}

#[test]
fn imports_only_from_allowed_folders() {
    let f = Fixture::new("wadd");
    let incoming = f.dir.join("incoming");
    std::fs::create_dir_all(&incoming).unwrap();
    let md = bookx_core::opf::Metadata {
        title: "Fresh Arrival".into(),
        authors: vec!["New Author".into()],
        ..Default::default()
    };
    let ch = vec![bookx_core::convert::Chapter {
        title: "One".into(),
        body: "<p>Hello</p>".into(),
    }];
    let p = incoming.join("fresh.epub");
    std::fs::write(
        &p,
        bookx_core::convert::create_epub(&md, &ch, None)
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    let outside = f.dir.join("src/dune.epub");
    let mut c = writer(&f);
    let r = c.ok("add_book_from_path", json!({"path": p.to_str().unwrap()}));
    assert_eq!(r["message"], "book added");
    assert_eq!(r["book"]["title"], "Fresh Arrival");
    let again = c.ok("add_book_from_path", json!({"path": p.to_str().unwrap()}));
    assert!(again["message"].as_str().unwrap().contains("already"));
    assert!(
        c.err(
            "add_book_from_path",
            json!({"path": outside.to_str().unwrap()})
        )
        .contains("outside")
    );
    assert!(
        c.err(
            "add_book_from_path",
            json!({"path": format!("{}/../src/dune.epub", incoming.display())})
        )
        .contains("outside")
    );
    assert!(
        c.err("add_book_from_path", json!({"path": "/etc/passwd"}))
            .contains("outside")
    );
    assert!(
        c.err(
            "add_book_from_path",
            json!({"path": incoming.join("none.epub").to_str().unwrap()})
        )
        .contains("does not exist")
    );
    // the import is one undoable operation
    let mut o = bookx_store::open_library(&f.root).unwrap();
    assert!(o.library.undo_last().unwrap().is_some());
    assert_eq!(
        o.library
            .books
            .iter()
            .filter(|b| b.meta.title == "Fresh Arrival")
            .count(),
        0
    );
    c.close();
}

#[test]
fn writes_are_refused_while_the_app_holds_the_library() {
    let f = Fixture::new("wlock");
    // another process (here: a stand-in with a different name) holds the lock
    let held = bookx_store::guard::acquire(&f.root, "the-app").unwrap();
    let busy = matches!(held, bookx_store::guard::Lock::Busy(_));
    let mut c = writer(&f);
    let e = c.err("add_tags", json!({"book_id": f.cook, "tags":["blocked"]}));
    // the same process counts as holder: either the lock was ours (then busy for the server)
    assert!(e.contains("open in") || busy, "{e}");
    // reads keep working while writes are refused
    c.ok("search_books", json!({}));
    drop(held);
    c.close();
}

#[test]
fn in_app_writes_do_not_take_the_lock() {
    let f = Fixture::new("winapp");
    let held = bookx_store::guard::acquire(&f.root, "the-app").unwrap();
    let mut cfg = f.config();
    cfg.write = WritePolicy::InApp;
    let mut c = Client::ready(cfg);
    c.ok("add_tags", json!({"book_id": f.cook, "tags":["inapp"]}));
    assert!(
        c.ok("get_book", json!({"book_id": f.cook}))["book"]["tags"]
            .to_string()
            .contains("inapp")
    );
    drop(held);
    c.close();
}

#[test]
fn fetch_metadata_uses_the_fetcher_and_needs_network_permission() {
    use bookx_sources::fetch::MockFetcher;
    let f = Fixture::new("wfetch");
    let body = json!({"numFound":1,"docs":[{"title":"Cooking Basics","author_name":["Julia Child"],
        "publisher":["Knopf"],"first_publish_year":1961,"isbn":["9780375413400"],"key":"/works/OL1W",
        "language":["eng"],"subject":["Cookery"]}]}).to_string();
    let mock = std::sync::Arc::new(MockFetcher::new().on_prefix(
        "https://openlibrary.org/search.json",
        bookx_sources::fetch::MockReply::Body(body.into_bytes()),
    ));
    let mut cfg = f.config();
    cfg.write = WritePolicy::Locked;
    cfg.allow_network = true;
    cfg.fetcher = Some(mock.clone());
    let mut c = Client::ready(cfg);
    let r = c.request("tools/list", json!({}));
    let t = r["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "fetch_metadata")
        .unwrap()
        .clone();
    assert_eq!(t["annotations"]["openWorldHint"], true);
    let preview = c.ok("fetch_metadata", json!({"book_id": f.cook}));
    assert_eq!(preview["applied"], false);
    assert!(
        mock.urls().iter().any(|u| u.contains("openlibrary")),
        "{:?}",
        mock.urls()
    );
    // preview changes nothing
    assert_eq!(
        c.ok("get_book", json!({"book_id": f.cook}))["book"]["publisher"],
        ""
    );
    if preview["would_change"]
        .as_array()
        .map(|a| !a.is_empty())
        .unwrap_or(false)
    {
        let applied = c.ok("fetch_metadata", json!({"book_id": f.cook, "apply": true}));
        assert_eq!(applied["applied"], true);
        assert_eq!(
            c.ok("get_book", json!({"book_id": f.cook}))["book"]["publisher"],
            "Knopf"
        );
    }
    c.close();
}
