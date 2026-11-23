//! Design rule (issue #39): the sync server moves reading position, highlights, notes and
//! bookmarks, never book files, and nothing a device does removes a book or a library file. A
//! device that deletes highlights, reports 0 %, or never comes back leaves the library as it was.
//! (The same rule is tested for the app's own sync code in `src/sync_rule.rs`.)

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use bookx_serve::server::LibrarySnapshot;
use common::zoo::{self, Bench, Peer, ZOO};

const DOC: &str = "0123456789abcdef0123456789abcdef";

struct Tmp(PathBuf);
impl Drop for Tmp {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn tree(root: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((
                    p.strip_prefix(root).unwrap().display().to_string(),
                    std::fs::read(&p).unwrap(),
                ));
            }
        }
    }
    out.sort();
    out
}

/// Strips the test module: what is left is the code that runs.
fn production(src: &str) -> &str {
    src.split("#[cfg(test)]").next().unwrap()
}

#[test]
fn a_device_that_deletes_wipes_or_goes_away_leaves_the_library_files_alone() {
    if std::env::var("NET").is_ok_and(|v| v == "0") {
        return;
    }
    let root = std::env::temp_dir().join(format!("bookx-rule-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("Author/Dune")).unwrap();
    std::fs::write(root.join("Author/Dune/Dune.epub"), b"PK the book itself").unwrap();
    std::fs::write(root.join("Author/Dune/cover.jpg"), b"jpeg").unwrap();
    let _guard = Tmp(root.clone());
    let before = tree(&root);
    let snapshot_root = root.clone();
    let bench = Bench::start_with(Arc::new(move || LibrarySnapshot {
        root: snapshot_root.clone(),
        books: Vec::new(),
    }));
    for reader in ZOO {
        let peer = Peer::plugin(&bench, reader);
        // the reader sends a highlight, deletes it, and reports that it is back at the start
        assert_eq!(peer.put_progress(DOC, 0.8), 200);
        assert_eq!(
            peer.put_highlight(DOC, "h1", "A line.", 1_700_000_000).0,
            200
        );
        assert_eq!(peer.put_progress(DOC, 0.0), 200, "{}", reader.slug);
    }
    // the highlight of the last reader is deleted by it
    let last = Peer::plugin(&bench, zoo::by_slug("boox-leaf2-issue-39"));
    let _ = last.put_highlight(DOC, "h1", "A line.", 1_700_000_100);
    assert_eq!(
        tree(&root),
        before,
        "no library file was added, changed or removed"
    );
    // there is no route that takes or gives a book file through the sync endpoints
    let base = bench.base();
    for (method, path) in [
        ("GET", "/bookx/books/Dune.epub"),
        ("PUT", "/bookx/books/Dune.epub"),
        ("DELETE", "/bookx/books/Dune.epub"),
        ("DELETE", "/bookx/syncs/progress"),
        ("DELETE", "/bookx/library"),
    ] {
        let r = bookx_serve::httpc::request(
            method,
            &format!("{base}{path}").parse().unwrap(),
            &[
                ("x-auth-user", bench.user),
                ("x-auth-key", bench.key.as_str()),
            ],
            b"x",
            std::time::Duration::from_secs(10),
        )
        .unwrap();
        assert!(
            r.status == 404 || r.status == 405,
            "{method} {path} answered {}: the sync server has no book route",
            r.status
        );
    }
    assert_eq!(tree(&root), before);
}

#[test]
fn the_sync_server_code_never_removes_a_library_file() {
    // kosync's own store writes through a temporary file it then renames; that is its only delete
    let files: &[(&str, &str, usize)] = &[
        ("kosync.rs", include_str!("../src/kosync.rs"), 1),
        (
            "annotation_sync.rs",
            include_str!("../src/annotation_sync.rs"),
            0,
        ),
        ("activity.rs", include_str!("../src/activity.rs"), 0),
        ("syncplan.rs", include_str!("../src/syncplan.rs"), 0),
        ("reading.rs", include_str!("../src/reading.rs"), 0),
        (
            "kosync_client.rs",
            include_str!("../src/kosync_client.rs"),
            0,
        ),
        // the private temporary copy of the Kobo database is removed when it is dropped
        ("kobo.rs", include_str!("../src/kobo.rs"), 1),
    ];
    for (name, src, allowed) in files {
        let code = production(src);
        for bad in [
            "remove_file(",
            "remove_dir_all(",
            "remove_dir(",
            "delete_book(",
        ] {
            assert!(
                code.matches(bad).count() <= *allowed,
                "{name} calls `{bad}`: sync code must not delete files"
            );
        }
    }
}
