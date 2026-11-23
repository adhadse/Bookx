//! Custom columns in the SQLite catalogue: persistence, schema upgrade, snapshots, JSON export.

use std::path::{Path, PathBuf};

use bookx_core::convert::{Chapter, create_epub};
use bookx_core::custom::{ColumnDef, ColumnKind};
use bookx_core::library::AddOutcome;
use bookx_core::opf::Metadata;
use bookx_store::health::{SnapshotKind, restore_snapshot, take_snapshot};
use bookx_store::{SCHEMA_VERSION, SqliteStore, open_library};
use chrono::Utc;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bookx-custom-{tag}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn epub(dir: &Path, title: &str) -> PathBuf {
    let md = Metadata {
        title: title.into(),
        authors: vec!["A".into()],
        ..Default::default()
    };
    let ch = Chapter {
        title: "One".into(),
        body: format!("<p>Text of {title}.</p>"),
    };
    let p = dir.join(format!("{title}.epub"));
    std::fs::write(&p, create_epub(&md, &[ch], None).to_bytes().unwrap()).unwrap();
    p
}

fn col(label: &str, kind: ColumnKind) -> ColumnDef {
    ColumnDef {
        label: label.into(),
        name: label.to_uppercase(),
        kind,
        ..Default::default()
    }
}

#[test]
fn columns_and_values_survive_reopening_and_snapshots() {
    let dir = tmp("persist");
    let root = dir.join("lib");
    let id = {
        let mut o = open_library(&root).unwrap();
        let AddOutcome::Added(id) = o.library.add_path(&epub(&dir, "One"), false).unwrap() else {
            panic!()
        };
        o.library
            .set_columns(vec![
                col("pages", ColumnKind::Integer),
                col("note", ColumnKind::Text),
            ])
            .unwrap();
        o.library.set_custom(id, "pages", "412").unwrap();
        o.library.set_custom(id, "note", "lent to Sam").unwrap();
        // a snapshot taken now has both
        take_snapshot(&o.store, &root, SnapshotKind::Manual, Utc::now()).unwrap();
        // ... and one value changes afterwards
        o.library.set_custom(id, "pages", "1").unwrap();
        id
    };

    let o = open_library(&root).unwrap();
    assert_eq!(o.library.columns().len(), 2);
    assert_eq!(o.library.columns()[0].label, "pages", "the order is kept");
    assert_eq!(o.library.get(id).unwrap().custom_value("pages"), "1");
    assert_eq!(
        o.store.search_ids("#pages:<5"),
        vec![id],
        "the store searches them too"
    );
    assert_eq!(o.store.search_ids("#note:lent"), vec![id]);

    // restoring the snapshot brings the older value back (it is part of the catalogue)
    let snap = bookx_store::health::list_snapshots(&root).remove(0);
    restore_snapshot(&o.store, &root, &snap.path, Utc::now()).unwrap();
    drop(o.library);
    let o2 = open_library(&root).unwrap();
    assert_eq!(o2.library.get(id).unwrap().custom_value("pages"), "412");
    assert_eq!(o2.library.columns().len(), 2);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_version_7_database_gains_the_table_and_keeps_its_books() {
    let dir = tmp("upgrade");
    let db = dir.join("v7.db");
    {
        let s = SqliteStore::open(&db).unwrap();
        assert_eq!(s.schema_version(), SCHEMA_VERSION);
    }
    {
        let c = rusqlite::Connection::open(&db).unwrap();
        c.execute_batch(
            "DROP TABLE custom_columns; DROP TABLE item_notes; PRAGMA user_version = 7;",
        )
        .unwrap();
    }
    let s = SqliteStore::open(&db).unwrap();
    assert_eq!(s.schema_version(), SCHEMA_VERSION);
    assert!(s.custom_columns().is_empty());
    s.set_custom_columns(&[col("a", ColumnKind::Bool)]).unwrap();
    assert_eq!(s.custom_columns().len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_json_export_carries_the_definitions_and_the_values() {
    let dir = tmp("json");
    let root = dir.join("lib");
    let mut o = open_library(&root).unwrap();
    let AddOutcome::Added(id) = o.library.add_path(&epub(&dir, "Two"), false).unwrap() else {
        panic!()
    };
    let mut mood = col("mood", ColumnKind::Enumeration);
    mood.values = vec!["Calm".into(), "Tense".into()];
    o.library.set_columns(vec![mood]).unwrap();
    o.library.set_custom(id, "mood", "calm").unwrap();
    let json = o.store.export_catalog_json().unwrap();
    assert!(json.contains("custom_columns") && json.contains("\"mood\": \"Calm\""));

    // importing that file into an empty store brings the definitions along
    let file = dir.join("cat.json");
    std::fs::write(&file, &json).unwrap();
    let fresh = SqliteStore::open_in_memory().unwrap();
    let report = fresh.import_json_catalog(&file).unwrap();
    assert_eq!(report.imported, 1);
    assert_eq!(fresh.custom_columns().len(), 1);
    assert_eq!(fresh.custom_columns()[0].values, vec!["Calm", "Tense"]);
    assert_eq!(fresh.search_ids("#mood:calm"), vec![id]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn clearing_the_text_index_forgets_every_book_so_it_can_be_rebuilt() {
    let dir = tmp("index");
    let mut o = open_library(&dir.join("lib")).unwrap();
    let AddOutcome::Added(a) = o.library.add_path(&epub(&dir, "Idx"), false).unwrap() else {
        panic!()
    };
    o.store
        .index_book_text_stamped(a, 7, "the white whale")
        .unwrap();
    assert_eq!(o.store.indexed_text_count(), 1);
    assert_eq!(o.store.search_text("whale", 5).unwrap().len(), 1);
    assert_eq!(o.store.text_stamps().len(), 1);
    assert_eq!(o.store.clear_text_index().unwrap(), 1);
    assert_eq!(o.store.indexed_text_count(), 0);
    assert!(o.store.search_text("whale", 5).unwrap().is_empty());
    assert!(
        o.store.text_stamps().is_empty(),
        "every book is pending again"
    );
    assert_eq!(o.store.clear_text_index().unwrap(), 0);
    let _ = std::fs::remove_dir_all(dir);
}
