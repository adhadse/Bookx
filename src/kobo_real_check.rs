//! Accuracy check of the device import against real data (owner's Kobo backup and a sandbox copy of
//! the owner's library). Ignored by default: it needs `BOOKX_KOBO_DEV` (a copy of the device root)
//! and `BOOKX_KOBO_LIB` (a SANDBOX copy of a library) and writes a JSON report to
//! `BOOKX_KOBO_OUT`. Run: `BOOKX_KOBO_DEV=.. BOOKX_KOBO_LIB=.. BOOKX_KOBO_OUT=.. cargo test -p bookx
//! kobo_real_check -- --ignored --nocapture`.

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    #[test]
    #[ignore]
    fn kobo_real_check() {
        let dev = PathBuf::from(std::env::var("BOOKX_KOBO_DEV").unwrap());
        let root = PathBuf::from(std::env::var("BOOKX_KOBO_LIB").unwrap());
        let out = PathBuf::from(std::env::var("BOOKX_KOBO_OUT").unwrap());
        let mut lib = bookx_store::open_library(&root).unwrap().library;
        // what the app does when it opens a library
        let repaired = crate::highlight_repair::one_time_pass(&mut lib);
        eprintln!(
            "one-time repair: {:?}",
            repaired.map(|r| (r.moved, r.unanchored, r.fine))
        );
        let (items, problems) = crate::annotation_import::read_paths(std::slice::from_ref(&dev));
        eprintln!("read {} items, problems {problems:?}", items.len());
        let mut books = crate::device_sync::library_books(&lib.books);
        crate::device_sync::attach_looks(&mut books, &lib.root);
        let plan =
            crate::annotation_import::plan(&items, &books, &crate::device_sync::book_locator(&lib));
        eprintln!("PLAN:\n{}", plan.text());
        if std::env::var("BOOKX_KOBO_APPLY").is_ok() {
            crate::annotation_import::apply(&mut lib, &plan);
        }
        // dump what the library holds for each matched book, with the painted text
        let mut rep = Vec::new();
        for b in &lib.books {
            if b.highlights.is_empty() {
                continue;
            }
            let ob = lib.open_book(b.id).ok();
            let ch = ob.map(crate::core::anchor::BookChapters::new);
            for h in &b.highlights {
                let painted = ch.as_ref().and_then(|c| {
                    use crate::core::anchor::Chapters;
                    if h.start < 0 {
                        return None;
                    }
                    c.texts(h.spine).first().map(|t| {
                        t.iter()
                            .skip(h.start as usize)
                            .take((h.end - h.start).max(0) as usize)
                            .collect::<String>()
                    })
                });
                rep.push(serde_json::json!({
                    "book": b.meta.title, "id": h.id, "text": h.text, "note": h.note, "color": h.color,
                    "style": h.style, "created": h.created.to_rfc3339(), "spine": h.spine,
                    "start": h.start, "end": h.end, "painted": painted,
                }));
            }
        }
        let bookmarks: Vec<_> = lib
            .books
            .iter()
            .flat_map(|b| {
                b.bookmarks
                    .iter()
                    .map(|m| serde_json::json!({"book": b.meta.title, "label": m.label, "created": m.created.to_rfc3339()}))
            })
            .collect();
        std::fs::write(
            &out,
            serde_json::to_string_pretty(&serde_json::json!({"plan": plan.text(), "highlights": rep, "bookmarks": bookmarks})).unwrap(),
        )
        .unwrap();
        if std::env::var("BOOKX_KOBO_APPLY").is_ok() {
            lib.save().unwrap();
        }
    }

    /// The wireless path with the owner's real data: the plugin's Lua logic (host luajit, curl as the
    /// transport) replays the real sidecars to a Bookx server on a random port, Bookx merges them into the
    /// sandbox library, then a highlight made in Bookx goes back to the device inbox.
    #[test]
    #[ignore]
    fn kobo_real_wireless_check() {
        use bookx_serve::annotation_sync::AnnotationSync;
        use bookx_serve::kosync::{md5_hex, KoSync, MemoryStore};
        use bookx_serve::server::{LibrarySnapshot, Server, ServerConfig};
        use std::sync::Arc;
        let sdrs: Vec<String> = std::env::var("BOOKX_KOBO_SDRS")
            .unwrap()
            .split('|')
            .map(str::to_string)
            .collect();
        let root = PathBuf::from(std::env::var("BOOKX_KOBO_LIB").unwrap());
        let out = PathBuf::from(std::env::var("BOOKX_KOBO_OUT").unwrap());
        let replay = std::env::var("BOOKX_KOBO_REPLAY").unwrap();
        let mut lib = bookx_store::open_library(&root).unwrap().library;
        let _ = crate::highlight_repair::one_time_pass(&mut lib);
        let provider: bookx_serve::server::LibraryProvider = Arc::new(LibrarySnapshot::default);
        let mut cfg = ServerConfig::new(provider);
        cfg.kosync = Some(Arc::new(KoSync::new(Box::new(MemoryStore::new()), true)));
        let ann = Arc::new(AnnotationSync::memory());
        cfg.annotations = Some(ann.clone());
        let h = Server::start(cfg).unwrap();
        let key = md5_hex("pw");
        let r = bookx_serve::httpc::request(
            "POST",
            &format!("{}/bookx/users/create", h.base_url())
                .parse()
                .unwrap(),
            &[("Content-Type", "application/json")],
            serde_json::json!({"username": "owner", "password": key})
                .to_string()
                .as_bytes(),
            std::time::Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(r.status, 201);
        let run_lua = |label: &str| {
            let o = std::process::Command::new("luajit")
                .arg(&replay)
                .args(&sdrs)
                .env("BOOKX_URL", format!("{}/bookx", h.base_url()))
                .env("BOOKX_USER", "owner")
                .env("BOOKX_KEY", &key)
                .output()
                .unwrap();
            eprintln!(
                "{label}:\n{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
        };
        run_lua("plugin run 1");
        let mut ledger = crate::annotation_sync::Ledger::default();
        let rep = crate::annotation_sync_ui::run(&mut lib, &ann, &mut ledger);
        eprintln!("MERGE 1: {}", rep.text());
        let rep2 = crate::annotation_sync_ui::run(&mut lib, &ann, &mut ledger);
        eprintln!("MERGE 2 (nothing new): {}", rep2.text());
        // a highlight made in Bookx
        let id = lib
            .books
            .iter()
            .find(|b| b.meta.title == "Building Microservices")
            .unwrap()
            .id;
        let quote = "Independent deployability";
        let locator = crate::device_sync::book_locator(&lib);
        use crate::device_sync::Locator;
        let (spine, a, b) = locator.locate(id, quote).unwrap();
        lib.get_mut(id)
            .unwrap()
            .highlights
            .push(bookx_core::library::Highlight {
                id: "bookx-made-1".into(),
                spine,
                start: a,
                end: b,
                color: "blue".into(),
                style: "underline".into(),
                note: "made in Bookx".into(),
                text: quote.into(),
                created: chrono::Utc::now(),
            });
        let rep3 = crate::annotation_sync_ui::run(&mut lib, &ann, &mut ledger);
        eprintln!("MERGE 3 (Bookx highlight goes out): {}", rep3.text());
        run_lua("plugin run 2 (receives from Bookx)");
        let recs = ann.documents();
        let mut dump = Vec::new();
        for (user, doc, title, _) in &recs {
            for r in ann.records(user, doc, 0) {
                dump.push(serde_json::json!({"doc": doc, "title": title, "origin": r.origin, "device": r.device,
                    "id": r.annotation.id, "kind": r.annotation.kind, "text": r.annotation.text, "note": r.annotation.note,
                    "color": r.annotation.color, "drawer": r.annotation.drawer, "chapter": r.annotation.chapter,
                    "created": r.annotation.created, "page": r.annotation.page, "deleted": r.annotation.deleted}));
            }
        }
        let mut hl = Vec::new();
        for bk in &lib.books {
            let ob = lib.open_book(bk.id).ok();
            let ch = ob.map(crate::core::anchor::BookChapters::new);
            for hh in &bk.highlights {
                use crate::core::anchor::Chapters;
                let painted = ch.as_ref().and_then(|c| {
                    (hh.start >= 0)
                        .then(|| {
                            c.texts(hh.spine).first().map(|t| {
                                t.iter()
                                    .skip(hh.start as usize)
                                    .take((hh.end - hh.start) as usize)
                                    .collect::<String>()
                            })
                        })
                        .flatten()
                });
                hl.push(serde_json::json!({"book": bk.meta.title, "id": hh.id, "text": hh.text, "note": hh.note,
                    "color": hh.color, "style": hh.style, "created": hh.created.to_rfc3339(), "spine": hh.spine,
                    "start": hh.start, "end": hh.end, "painted": painted}));
            }
        }
        std::fs::write(
            &out,
            serde_json::to_string_pretty(&serde_json::json!({
            "merge1": rep.text(), "merge2": rep2.text(), "merge3": rep3.text(),
            "server": dump, "highlights": hl, "bookmarks": []}))
            .unwrap(),
        )
        .unwrap();
        drop(h);
    }
}
