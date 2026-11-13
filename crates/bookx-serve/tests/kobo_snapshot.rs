//! Runs the Kobo database code against a COPY of a real device's `KoboReader.sqlite`.
//!
//! Skipped unless `BOOKX_KOBO_SNAPSHOT` names such a copy (a file, never the device itself): the
//! snapshot holds the owner's data and is not part of the repository. Everything happens on a
//! second copy in a temporary folder, so the snapshot stays byte-identical.

use std::path::PathBuf;

use bookx_serve::kobo;

fn snapshot() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("BOOKX_KOBO_SNAPSHOT")?);
    p.is_file().then_some(p)
}

#[test]
fn real_snapshot_reads_and_a_shelf_write_on_the_copy() {
    let Some(src) = snapshot() else {
        eprintln!("BOOKX_KOBO_SNAPSHOT not set: skipped");
        return;
    };
    assert!(
        !src.starts_with("/run/media"),
        "give a copy of the database, not the device"
    );
    let dir = std::env::temp_dir().join(format!("bookx-kobo-snap-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("KoboReader.sqlite");
    std::fs::copy(&src, &db).unwrap();
    let wal = src.with_file_name("KoboReader.sqlite-wal");
    if wal.is_file() {
        std::fs::copy(&wal, dir.join("KoboReader.sqlite-wal")).unwrap();
    }
    let before_bytes = std::fs::read(&src).unwrap();

    let info = kobo::db_info(&db).unwrap();
    assert!(info.schema_version.is_some());
    let states = kobo::read_states(&db).unwrap();
    assert_eq!(states.len(), info.book_rows);
    assert!(
        states
            .iter()
            .all(|(_, s)| (0.0..=1.0).contains(&s.progress)),
        "progress is a fraction"
    );
    let shelves_before = kobo::read_shelves(&db).unwrap();

    let ids = vec![
        kobo::content_id("BookxSandbox/Test One.kepub.epub"),
        kobo::content_id("BookxSandbox/Test Two.kepub.epub"),
    ];
    let n = kobo::add_to_shelf(&db, "Bookx test", &ids, "2026-11-12T12:00:00Z").unwrap();
    assert_eq!(n, 2);
    assert_eq!(
        kobo::add_to_shelf(&db, "Bookx test", &ids, "2026-11-12T12:01:00Z").unwrap(),
        0
    );
    let after = kobo::read_shelves(&db).unwrap();
    assert_eq!(after.len(), shelves_before.len() + 1);
    for s in &shelves_before {
        assert!(after.contains(s), "existing shelf {} unchanged", s.name);
    }
    // reading state of every book is as it was
    let states_after = kobo::read_states(&db).unwrap();
    assert_eq!(states.len(), states_after.len());
    for ((ia, a), (ib, b)) in states.iter().zip(states_after.iter()) {
        assert_eq!(ia, ib);
        assert_eq!(
            (a.progress, a.status, &a.last_read),
            (b.progress, b.status, &b.last_read)
        );
    }
    let c = rusqlite::Connection::open(&db).unwrap();
    let ok: String = c
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ok, "ok");
    drop(c);
    // the backup is the database as it was, and the snapshot itself was not touched
    let bak = dir.join("KoboReader.sqlite.bookx-backup");
    assert_eq!(kobo::read_states(&bak).unwrap().len(), states.len());
    assert_eq!(
        kobo::read_shelves(&bak).unwrap().len(),
        shelves_before.len()
    );
    assert_eq!(std::fs::read(&src).unwrap(), before_bytes);
    std::fs::remove_dir_all(&dir).ok();
}

/// Parses every `*.sdr/metadata.*.lua` below the folder `BOOKX_KOBO_SIDECARS` (copies taken from a
/// real device) and checks the result against an independent count in the raw text.
#[test]
fn real_koreader_sidecars_are_parsed_completely() {
    let Some(dir) = std::env::var_os("BOOKX_KOBO_SIDECARS").map(PathBuf::from) else {
        return;
    };
    let mut seen = 0;
    for d in std::fs::read_dir(&dir).unwrap().flatten() {
        let Ok(rd) = std::fs::read_dir(d.path()) else {
            continue;
        };
        for f in rd.flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            if !(name.starts_with("metadata.") && name.ends_with(".lua")) {
                continue;
            }
            let text = std::fs::read_to_string(f.path()).unwrap();
            let sc = bookx_serve::reading::parse_sidecar(&text);
            // each annotation of the file has a line `[1] = {` at eight spaces of indent
            let expected = text
                .lines()
                .filter(|l| l.starts_with("        [") && l.trim_end().ends_with("] = {"))
                .count();
            let top_level_percent = text
                .lines()
                .find_map(|l| l.strip_prefix("    [\"percent_finished\"] = "))
                .map(|v| v.trim_end_matches(',').parse::<f64>().unwrap());
            let in_annotations = text.contains("[\"annotations\"] = {}");
            let st = sc.state.expect("state");
            assert_eq!(
                Some(st.progress),
                top_level_percent.map(|p| p.clamp(0.0, 1.0))
            );
            assert_eq!(st.source, "KOReader");
            assert!(st.last_read.is_some(), "summary.modified is read");
            if !in_annotations {
                assert!(sc.notes.len() <= expected && !sc.notes.is_empty());
            }
            seen += 1;
        }
    }
    assert!(seen > 0, "no sidecars in {}", dir.display());
}
