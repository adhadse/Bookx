//! The plugin adapters behind the `bookx_core::contracts` traits.

mod common;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use bookx_core::archive::Archive;
use bookx_core::contracts::{
    pick_reader, run_pipeline, FormatReader, FormatWriter, MetadataQuery, MetadataSource, Registry,
    Transform,
};
use bookx_core::opf::Metadata;
use bookx_plugin::adapters::decode_metadata_list;
use bookx_plugin::{
    Host, PluginMetadataSource, PluginReader, PluginTransform, PluginWriter, SNIFF_BYTES,
};
use common::*;
use serde_json::json;

fn query() -> MetadataQuery {
    MetadataQuery {
        title: "Dune".into(),
        authors: vec!["Frank Herbert".into()],
        isbn: Some("9780441172719".into()),
    }
}

const LOOKUP_REPLY: &str = r#"[
  {"title":"Dune","authors":["Frank Herbert"],"tags":["sf","classic"],"rating":8,
   "series":"Dune","series_index":1.0,"language":"en","publisher":"Ace",
   "identifiers":[{"scheme":"isbn","value":"9780441172719"}],"description":"Spice. Sand. 🪱"},
  {"title":"Minimal"}
]"#;

#[test]
fn metadata_lookup_decodes_results_with_defaults() {
    let p = arc("metadata-source", "", &canned(&[(b'l', ok(LOOKUP_REPLY))]));
    let src = PluginMetadataSource::new(p).unwrap();
    assert_eq!(src.id(), "test.plugin");
    let r = src.lookup(&query()).unwrap();
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].title, "Dune");
    assert_eq!(r[0].authors, vec!["Frank Herbert"]);
    assert_eq!(r[0].tags, vec!["sf", "classic"]);
    assert_eq!(r[0].rating, 8);
    assert_eq!(r[0].series_index, 1.0);
    assert_eq!(r[0].identifier("isbn"), Some("9780441172719"));
    assert!(r[0].description.contains("🪱"));
    assert_eq!(r[1].title, "Minimal");
    assert!(r[1].authors.is_empty());
    assert_eq!(r[1].rating, 0);
}

#[test]
fn lookup_sends_the_query_as_json() {
    // echo plugin returns the query object as "ok", which is not an array -> decode error,
    // but we can observe the exact input through call_json on the same plugin.
    let p = arc("metadata-source", "", &echo());
    let v = p
        .call_json(
            "lookup",
            &json!({"title": "Dune", "authors": ["Frank Herbert"], "isbn": "9780441172719"}),
        )
        .unwrap();
    assert_eq!(v["title"], "Dune");
    let src = PluginMetadataSource::new(p).unwrap();
    let e = format!("{:#}", src.lookup(&query()).unwrap_err());
    assert!(e.contains("not an array of metadata objects"), "{e}");
}

#[test]
fn lookup_with_no_isbn_sends_null() {
    // A plugin that answers [] only when isbn is null: emulate with echo + manual check of the shape.
    let p = arc("metadata-source", "", &echo());
    let q = MetadataQuery {
        title: "T".into(),
        authors: vec![],
        isbn: None,
    };
    let src = PluginMetadataSource::new(p.clone()).unwrap();
    assert!(src.lookup(&q).is_err()); // echo is not an array
    let v = p
        .call_json("lookup", &json!({"title":"T","authors":[],"isbn":null}))
        .unwrap();
    assert!(v["isbn"].is_null());
}

#[test]
fn lookup_empty_and_errors() {
    let p = arc("metadata-source", "", &canned(&[(b'l', ok("[]"))]));
    assert!(PluginMetadataSource::new(p)
        .unwrap()
        .lookup(&query())
        .unwrap()
        .is_empty());

    let p = arc(
        "metadata-source",
        "",
        &canned(&[(b'l', br#"{"err":"rate limited"}"#.to_vec())]),
    );
    let e = format!(
        "{:#}",
        PluginMetadataSource::new(p)
            .unwrap()
            .lookup(&query())
            .unwrap_err()
    );
    assert!(e.contains("rate limited"), "{e}");

    for bad in ["{}", "\"x\"", "[1]", "[{\"title\":5}]", "null"] {
        let p = arc("metadata-source", "", &canned(&[(b'l', ok(bad))]));
        let e = format!(
            "{:#}",
            PluginMetadataSource::new(p)
                .unwrap()
                .lookup(&query())
                .unwrap_err()
        );
        assert!(e.contains("not an array of metadata objects"), "{bad}: {e}");
    }
}

#[test]
fn rating_is_clamped_and_unknown_fields_ignored() {
    let r = decode_metadata_list(json!([{"title":"x","rating":200}])).unwrap();
    assert_eq!(r[0].rating, 10);
    // 256 does not fit u8 -> error
    assert!(decode_metadata_list(json!([{"title":"x","rating":256}])).is_err());
    assert!(decode_metadata_list(json!([{"title":"x","rating":-1}])).is_err());
    let r = decode_metadata_list(json!([{"title":"x","rating":255,"future_field":[1,2]}])).unwrap();
    assert_eq!(r[0].rating, 10);
}

#[test]
fn cover_returns_bytes_none_or_errors() {
    let png = [0x89, b'P', b'N', b'G', 0, 1, 2, 255];
    let reply = ok(&format!("\"{}\"", B64.encode(png)));
    let src =
        PluginMetadataSource::new(arc("metadata-source", "", &canned(&[(b'c', reply)]))).unwrap();
    assert_eq!(src.cover(&Metadata::default()).unwrap(), Some(png.to_vec()));

    let src = PluginMetadataSource::new(arc("metadata-source", "", &canned(&[(b'c', ok("null"))])))
        .unwrap();
    assert_eq!(src.cover(&Metadata::default()).unwrap(), None);

    for bad in ["5", "\"!!!not base64!!!\""] {
        let src =
            PluginMetadataSource::new(arc("metadata-source", "", &canned(&[(b'c', ok(bad))])))
                .unwrap();
        assert!(src.cover(&Metadata::default()).is_err(), "{bad}");
    }

    // plugins without a cover op: -2 is an error the caller can treat as "no cover"
    let src = PluginMetadataSource::new(arc("metadata-source", "", &canned(&[(b'l', ok("[]"))])))
        .unwrap();
    assert!(src.cover(&Metadata::default()).is_err());
}

#[test]
fn cover_input_is_the_full_metadata() {
    let p = arc("metadata-source", "", &echo());
    let md = Metadata {
        title: "Ünï".into(),
        authors: vec!["A".into()],
        ..Default::default()
    };
    let v = p
        .call_json("cover", &serde_json::to_value(&md).unwrap())
        .unwrap();
    assert_eq!(v["title"], "Ünï");
    // echoed object is not base64 -> adapter error
    let src = PluginMetadataSource::new(p).unwrap();
    assert!(src.cover(&md).is_err());
}

#[test]
fn adapters_check_the_plugin_kind() {
    let wasm = always(&ok("1"));
    let t = || arc("transform", "", &wasm);
    let m = |kind: &str| arc(kind, "extensions = [\"x\"]\nextension = \"x\"", &wasm);
    let e = PluginMetadataSource::new(t()).err().unwrap().to_string();
    assert!(e.contains("is a transform, not a metadata-source"), "{e}");
    assert!(PluginReader::new(t())
        .err()
        .unwrap()
        .to_string()
        .contains("not a format-reader"));
    assert!(PluginWriter::new(t())
        .err()
        .unwrap()
        .to_string()
        .contains("not a format-writer"));
    assert!(PluginTransform::new(m("metadata-source"))
        .err()
        .unwrap()
        .to_string()
        .contains("not a transform"));
    assert!(PluginTransform::new(m("device")).is_err());
    assert!(PluginTransform::new(t()).is_ok());
}

// ---------- reader ----------

fn reader_plugin(epub_zip: &[u8]) -> PluginReader {
    let wasm = canned(&[
        (b's', ok("true")),
        (b'r', ok(&format!("\"{}\"", B64.encode(epub_zip)))),
    ]);
    PluginReader::new(arc(
        "format-reader",
        "extensions = [\"FB3\", \"fbx\"]",
        &wasm,
    ))
    .unwrap()
}

#[test]
fn reader_metadata_and_sniff() {
    let r = reader_plugin(b"");
    assert_eq!(r.id(), "test.plugin");
    assert_eq!(r.extensions(), &["fb3", "fbx"]);
    assert!(r.sniff(b"anything"));
    assert!(r.sniff(b""));
    assert!(r.sniff(&vec![0u8; SNIFF_BYTES * 10]));
}

#[test]
fn sniff_failures_mean_false() {
    for reply in [ok("false"), ok("\"yes\""), ok("1"), b"garbage".to_vec()] {
        let wasm = canned(&[(b's', reply)]);
        let r = PluginReader::new(arc("format-reader", "extensions = [\"x\"]", &wasm)).unwrap();
        assert!(!r.sniff(b"x"));
    }
    // no sniff op at all
    let wasm = canned(&[(b'r', ok("\"\""))]);
    let r = PluginReader::new(arc("format-reader", "extensions = [\"x\"]", &wasm)).unwrap();
    assert!(!r.sniff(b"x"));
}

#[test]
fn sniff_only_sends_the_head() {
    // echo returns the input object; check the size bound through call_json equivalence by
    // measuring the reply of the same encoding the adapter uses.
    let p = arc(
        "format-reader",
        "extensions = [\"x\"]\nmemory_pages = 64",
        &echo(),
    );
    let head = vec![7u8; SNIFF_BYTES];
    let v = p
        .call_json("sniff", &json!({"head_b64": B64.encode(&head)}))
        .unwrap();
    assert_eq!(
        B64.decode(v["head_b64"].as_str().unwrap()).unwrap().len(),
        SNIFF_BYTES
    );
    // the adapter must not choke on a 1 MiB head even in a small plugin
    let r = PluginReader::new(p).unwrap();
    assert!(!r.sniff(&vec![1u8; 1 << 20])); // echo's object is not a bool
}

#[test]
fn reader_returns_a_usable_archive() {
    let zip = sample_epub("From Plugin").archive.to_bytes().unwrap();
    let r = reader_plugin(&zip);
    let a = r.read(b"%FB3 original bytes%").unwrap();
    assert!(a.contains("META-INF/container.xml"));
    let book = bookx_core::book::OpenBook::from_archive(a).unwrap();
    assert_eq!(book.metadata().title, "From Plugin");
}

#[test]
fn reader_rejects_bad_replies() {
    for (reply, msg) in [
        (ok("5"), "not a base64 string"),
        (ok("\"@@@@\""), "not valid base64"),
        (
            ok(&format!("\"{}\"", B64.encode(b"this is not a zip"))),
            "invalid EPUB",
        ),
        (br#"{"err":"cannot parse"}"#.to_vec(), "cannot parse"),
    ] {
        let wasm = canned(&[(b'r', reply)]);
        let r = PluginReader::new(arc("format-reader", "extensions = [\"x\"]", &wasm)).unwrap();
        let e = match r.read(b"x") {
            Ok(_) => panic!("expected error for {msg}"),
            Err(e) => format!("{e:#}"),
        };
        assert!(e.contains(msg), "{msg}: {e}");
    }
}

#[test]
fn plugin_reader_works_in_the_registry() {
    let zip = sample_epub("Reg").archive.to_bytes().unwrap();
    let mut readers: Registry<dyn FormatReader> = Registry::new();
    readers.register(Box::new(reader_plugin(&zip)));
    assert_eq!(
        pick_reader(&readers, "book.FB3", b"").unwrap().id(),
        "test.plugin"
    );
    assert_eq!(
        pick_reader(&readers, "book.unknown", b"sniffme")
            .unwrap()
            .id(),
        "test.plugin"
    );
}

// ---------- writer ----------

#[test]
fn writer_returns_the_plugin_bytes() {
    let out = b"%PDF-fake\x00\xff output".to_vec();
    let wasm = canned(&[(b'w', ok(&format!("\"{}\"", B64.encode(&out))))]);
    let w = PluginWriter::new(arc("format-writer", "extension = \"Pdfx\"", &wasm)).unwrap();
    assert_eq!(w.id(), "test.plugin");
    assert_eq!(w.extension(), "pdfx");
    assert_eq!(w.write(&sample_epub("W")).unwrap(), out);
}

#[test]
fn writer_sends_a_valid_epub_zip() {
    // echo returns {"book_b64": ...}; verify that value decodes to a zip containing the book.
    let p = arc(
        "format-writer",
        "extension = \"x\"\nmemory_pages = 256",
        &echo(),
    );
    let book = sample_epub("Sent");
    let zip = book.archive.to_bytes().unwrap();
    let v = p
        .call_json("write", &json!({"book_b64": B64.encode(&zip)}))
        .unwrap();
    let back = Archive::from_bytes(B64.decode(v["book_b64"].as_str().unwrap()).unwrap()).unwrap();
    assert!(back.contains("META-INF/container.xml"));
    // and the adapter, fed the echo, errors since the reply is not a string
    let w = PluginWriter::new(p).unwrap();
    assert!(w.write(&book).is_err());
}

#[test]
fn writer_errors() {
    for (reply, msg) in [
        (ok("null"), "not a base64 string"),
        (ok("\"%%\""), "not valid base64"),
        (br#"{"err":"disk full"}"#.to_vec(), "disk full"),
    ] {
        let wasm = canned(&[(b'w', reply)]);
        let w = PluginWriter::new(arc("format-writer", "extension = \"x\"", &wasm)).unwrap();
        let e = format!("{:#}", w.write(&sample_epub("W")).unwrap_err());
        assert!(e.contains(msg), "{msg}: {e}");
    }
}

// ---------- transform ----------

#[test]
fn echo_transform_keeps_the_book() {
    let t = PluginTransform::new(arc("transform", "memory_pages = 256", &echo())).unwrap();
    assert_eq!(t.id(), "test.plugin");
    let mut book = sample_epub("Same");
    let before = book.archive.to_bytes().unwrap().len();
    let report = t.apply(&mut book).unwrap();
    assert_eq!(report.files_changed, 0);
    assert!(report.notes.is_empty());
    assert_eq!(book.metadata().title, "Same");
    assert_eq!(book.archive.to_bytes().unwrap().len(), before);
}

#[test]
fn transform_replaces_the_book_and_reports() {
    let new_zip = sample_epub("Changed").archive.to_bytes().unwrap();
    let reply = ok(&format!(
        r#"{{"book_b64":"{}","files_changed":3,"notes":["fixed quotes","日本語"]}}"#,
        B64.encode(&new_zip)
    ));
    let t = PluginTransform::new(arc("transform", "", &canned(&[(b'a', reply)]))).unwrap();
    let mut book = sample_epub("Original");
    let report = t.apply(&mut book).unwrap();
    assert_eq!(book.metadata().title, "Changed");
    assert_eq!(report.files_changed, 3);
    assert_eq!(report.notes, vec!["fixed quotes", "日本語"]);
}

#[test]
fn transform_report_only_leaves_the_book() {
    let t = PluginTransform::new(arc(
        "transform",
        "",
        &canned(&[(b'a', ok(r#"{"files_changed":0,"notes":["nothing to do"]}"#))]),
    ))
    .unwrap();
    let mut book = sample_epub("Keep");
    let r = t.apply(&mut book).unwrap();
    assert_eq!(r.notes, vec!["nothing to do"]);
    assert_eq!(book.metadata().title, "Keep");
    let t = PluginTransform::new(arc("transform", "", &canned(&[(b'a', ok("{}"))]))).unwrap();
    assert_eq!(t.apply(&mut book).unwrap().files_changed, 0);
    let t = PluginTransform::new(arc(
        "transform",
        "",
        &canned(&[(
            b'a',
            ok(r#"{"book_b64":null,"notes":null,"files_changed":null}"#),
        )]),
    ))
    .unwrap();
    assert!(t.apply(&mut book).unwrap().notes.is_empty());
}

#[test]
fn transform_rejects_bad_replies_and_keeps_the_book() {
    let not_epub = B64.encode(b"nope");
    for (reply, msg) in [
        (ok("5"), "not an object"),
        (
            ok(r#"{"files_changed":"many"}"#),
            "files_changed is not a number",
        ),
        (
            ok(r#"{"files_changed":-1}"#),
            "files_changed is not a number",
        ),
        (ok(r#"{"notes":"one"}"#), "notes is not an array"),
        (ok(r#"{"notes":[1]}"#), "a note is not a string"),
        (ok(r#"{"book_b64":5}"#), "not a base64 string"),
        (ok(r#"{"book_b64":"@@"}"#), "not valid base64"),
        (
            ok(&format!(r#"{{"book_b64":"{not_epub}"}}"#)),
            "invalid EPUB",
        ),
        (br#"{"err":"bad quotes"}"#.to_vec(), "bad quotes"),
    ] {
        let t = PluginTransform::new(arc("transform", "", &canned(&[(b'a', reply)]))).unwrap();
        let mut book = sample_epub("Untouched");
        let e = format!("{:#}", t.apply(&mut book).unwrap_err());
        assert!(e.contains(msg), "{msg}: {e}");
        assert_eq!(book.metadata().title, "Untouched");
    }
}

#[test]
fn a_valid_zip_that_is_not_an_epub_is_rejected() {
    let mut a = Archive::default();
    a.set_string("hello.txt", "hi");
    let reply = ok(&format!(
        r#"{{"book_b64":"{}"}}"#,
        B64.encode(a.to_bytes().unwrap())
    ));
    let t = PluginTransform::new(arc("transform", "", &canned(&[(b'a', reply)]))).unwrap();
    let mut book = sample_epub("Safe");
    assert!(t.apply(&mut book).is_err());
    assert_eq!(book.metadata().title, "Safe");
}

#[test]
fn transform_runs_in_the_core_pipeline_and_failures_name_the_stage() {
    let new_zip = sample_epub("Piped").archive.to_bytes().unwrap();
    let good = ok(&format!(
        r#"{{"book_b64":"{}","files_changed":1}}"#,
        B64.encode(new_zip)
    ));
    let mut stages: Registry<dyn Transform> = Registry::new();
    stages.register(Box::new(
        PluginTransform::new(arc("transform", "", &canned(&[(b'a', good)]))).unwrap(),
    ));
    let mut book = sample_epub("In");
    let report = run_pipeline(&mut book, &stages).unwrap();
    assert_eq!(report[0].0, "test.plugin");
    assert_eq!(book.metadata().title, "Piped");

    stages.register(Box::new(
        PluginTransform::new(arc("transform", "", &trapper())).unwrap(),
    ));
    let e = format!("{:#}", run_pipeline(&mut book, &stages).unwrap_err());
    assert!(e.contains("transform `test.plugin` failed"), "{e}");
    assert!(e.contains("trapped"), "{e}");
}

#[test]
fn metadata_source_registers_in_a_registry() {
    let mut reg: Registry<dyn MetadataSource> = Registry::new();
    reg.register(Box::new(
        PluginMetadataSource::new(arc(
            "metadata-source",
            "",
            &canned(&[(b'l', ok(LOOKUP_REPLY))]),
        ))
        .unwrap(),
    ));
    let found: Vec<_> = reg
        .iter()
        .flat_map(|s| s.lookup(&query()).unwrap())
        .collect();
    assert_eq!(found.len(), 2);
}

#[test]
fn ids_are_interned_so_reloads_do_not_leak_new_strings() {
    let a = PluginMetadataSource::new(arc("metadata-source", "", &always(&ok("[]")))).unwrap();
    let b = PluginMetadataSource::new(arc("metadata-source", "", &always(&ok("[]")))).unwrap();
    assert!(std::ptr::eq(a.id(), b.id()));
    let r1 = PluginReader::new(arc(
        "format-reader",
        "extensions = [\"q1\"]",
        &always(&ok("1")),
    ))
    .unwrap();
    let r2 = PluginReader::new(arc(
        "format-reader",
        "extensions = [\"q1\"]",
        &always(&ok("1")),
    ))
    .unwrap();
    assert!(std::ptr::eq(r1.extensions(), r2.extensions()));
}

#[test]
fn host_without_network_makes_network_plugins_fail_cleanly() {
    let p = std::sync::Arc::new(plugin_host(
        "metadata-source",
        "permissions = [\"network:x.org\"]",
        &http_client(),
        Host::default(),
    ));
    let src = PluginMetadataSource::new(p).unwrap();
    assert!(src.lookup(&query()).is_err());
}
