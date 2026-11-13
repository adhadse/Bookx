//! Layouts, transactional moves, trash and undo of the library (roadmap PP-C-3, PP-C-7, PP-G-33).

use std::path::{Path, PathBuf};

use bookx_core::layout::{Layout, TemplateFn};
use bookx_core::library::{AddOutcome, Library};

fn tmp() -> PathBuf {
    let d = std::env::temp_dir().join(format!("bookx-files-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn md_book(dir: &Path, title: &str, author: &str) -> PathBuf {
    let p = dir.join(format!("{title}.md"));
    std::fs::write(
        &p,
        format!("---\ntitle: \"{title}\"\nauthor: \"{author}\"\n---\n# One\n\nHello {title}.\n"),
    )
    .unwrap();
    p
}

fn epub_book(dir: &Path, title: &str, author: &str) -> PathBuf {
    // import through a scratch library to get a real EPUB file
    let scratch = tmp();
    let mut l = Library::in_memory(&scratch.join("l")).unwrap();
    let AddOutcome::Added(id) = l.add_path(&md_book(&scratch, title, author), true).unwrap() else {
        panic!()
    };
    let out = dir.join(format!("{title}.epub"));
    l.export_copy(id, &out).unwrap();
    let _ = std::fs::remove_dir_all(scratch);
    out
}

fn add(lib: &mut Library, scratch: &Path, title: &str, author: &str) -> u64 {
    match lib
        .add_path(&md_book(scratch, title, author), false)
        .unwrap()
    {
        AddOutcome::Added(id) => id,
        AddOutcome::Duplicate(_) => panic!("duplicate"),
    }
}

fn render(src: &str, m: &bookx_core::opf::Metadata, ext: &str) -> Result<Vec<String>, String> {
    let s = src.replace("{title}", &m.title).replace(
        "{author}",
        m.authors.first().map(String::as_str).unwrap_or("x"),
    );
    let mut c: Vec<String> = s.split('/').map(str::to_string).collect();
    c.last_mut().unwrap().push_str(&format!(".{ext}"));
    Ok(c)
}

fn all_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p.strip_prefix(root).unwrap().to_string_lossy().to_string();
                if !rel.starts_with(".bookx/") {
                    out.push(rel); // the state folder (log, layout) is not book data
                }
            }
        }
    }
    out.sort();
    out
}

#[test]
fn the_default_layout_is_the_tree() {
    let d = tmp();
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    let id = add(&mut lib, &d, "Dune", "Frank Herbert");
    assert_eq!(
        lib.get(id).unwrap().file,
        format!("Frank Herbert/Dune ({id})/Dune - Frank Herbert.epub")
    );
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn flat_layout_imports_into_the_library_folder() {
    let d = tmp();
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    lib.set_layout(Layout::Flat).unwrap();
    let id = add(&mut lib, &d, "Dune", "Frank Herbert");
    assert_eq!(lib.get(id).unwrap().file, "Frank Herbert - Dune.epub");
    assert!(lib.book_path(lib.get(id).unwrap()).exists());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn changing_the_layout_previews_then_moves_everything_and_back() {
    let d = tmp();
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    let a = add(&mut lib, &d, "Dune", "Frank Herbert");
    let b = add(&mut lib, &d, "Emma", "Jane Austen");
    let before = all_files(&d.join("lib"));

    let plan = lib.plan_layout(&Layout::Flat);
    assert_eq!(plan.len(), 2);
    assert_eq!(plan[0].new_file, "Frank Herbert - Dune.epub");
    assert_eq!(
        all_files(&d.join("lib")),
        before,
        "a dry run changes nothing"
    );

    assert_eq!(lib.change_layout(Layout::Flat).unwrap(), 2);
    assert_eq!(lib.get(a).unwrap().file, "Frank Herbert - Dune.epub");
    assert_eq!(lib.get(b).unwrap().file, "Jane Austen - Emma.epub");
    assert!(
        !d.join("lib/Frank Herbert").exists(),
        "empty folders are removed"
    );
    assert!(lib.book_path(lib.get(a).unwrap()).exists());
    assert_eq!(lib.open_book(a).unwrap().metadata().title, "Dune");

    // and back to the tree
    assert_eq!(lib.change_layout(Layout::Tree).unwrap(), 2);
    assert_eq!(all_files(&d.join("lib")), before);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn template_layout_uses_the_renderer() {
    let d = tmp();
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    let f: TemplateFn = render;
    lib.set_template_renderer(f);
    let id = add(&mut lib, &d, "Dune", "Frank Herbert");
    lib.change_layout(Layout::Template("{author}/{title}".into()))
        .unwrap();
    assert_eq!(lib.get(id).unwrap().file, "Frank Herbert/Dune.epub");
    assert!(lib.book_path(lib.get(id).unwrap()).exists());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_failure_in_the_middle_of_a_layout_change_leaves_everything_as_it_was() {
    let d = tmp();
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    let a = add(&mut lib, &d, "Dune", "Frank Herbert");
    let b = add(&mut lib, &d, "Emma", "Jane Austen");
    let c = add(&mut lib, &d, "Ulysses", "James Joyce");
    let files_before = all_files(&d.join("lib"));
    let catalogue_before: Vec<String> = lib.books.iter().map(|b| b.file.clone()).collect();

    let plan = lib.plan_layout(&Layout::Flat);
    let mut calls = 0;
    let err = lib
        .apply_plan_with(&plan, &mut |from, to| {
            calls += 1;
            if calls == 3 {
                return Err(std::io::Error::other("disk gone"));
            }
            bookx_core::fsops::move_file(from, to)
        })
        .unwrap_err();
    assert!(format!("{err:#}").contains("disk gone"));
    assert_eq!(all_files(&d.join("lib")), files_before);
    let catalogue_after: Vec<String> = lib.books.iter().map(|b| b.file.clone()).collect();
    assert_eq!(catalogue_after, catalogue_before);
    for id in [a, b, c] {
        assert!(lib.book_path(lib.get(id).unwrap()).exists());
    }
    assert!(
        lib.history().iter().all(|o| o.label != "Moved 3 books"),
        "a failed move is not logged"
    );
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_metadata_edit_that_cannot_move_the_files_reports_the_error() {
    let d = tmp();
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    let id = add(&mut lib, &d, "Dune", "Frank Herbert");
    // a file squats on the place the book would move to
    let mut m = lib.get(id).unwrap().meta.clone();
    m.authors = vec!["Someone Else".into()];
    let squat = d.join(format!(
        "lib/Someone Else/Dune ({id})/Dune - Someone Else.epub"
    ));
    std::fs::create_dir_all(squat.parent().unwrap()).unwrap();
    std::fs::write(&squat, "x").unwrap();
    let old = lib.get(id).unwrap().file.clone();
    assert!(lib.update_metadata(id, m).is_err());
    assert_eq!(
        lib.get(id).unwrap().file,
        old,
        "the book is still where the catalogue says"
    );
    assert!(lib.book_path(lib.get(id).unwrap()).exists());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn linked_books_stay_where_they_are() {
    let d = tmp();
    let shelf = d.join("my books");
    std::fs::create_dir_all(&shelf).unwrap();
    let epub = epub_book(&shelf, "Linked One", "Some Author");
    let size = std::fs::metadata(&epub).unwrap().len();
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    lib.set_layout(Layout::Linked).unwrap();
    let AddOutcome::Added(id) = lib.add_path(&epub, false).unwrap() else {
        panic!()
    };
    assert_eq!(lib.get(id).unwrap().size, size);
    assert_eq!(
        lib.book_path(lib.get(id).unwrap()),
        epub.canonicalize().unwrap()
    );
    assert!(
        all_files(&d.join("lib"))
            .iter()
            .all(|f| f.starts_with(".bookx/")),
        "nothing was copied"
    );

    // editing writes the book where it is and never renames it
    let mut m = lib.get(id).unwrap().meta.clone();
    m.title = "Edited Title".into();
    lib.update_metadata(id, m).unwrap();
    assert!(epub.exists(), "the file keeps its name");
    assert_eq!(lib.open_book(id).unwrap().metadata().title, "Edited Title");

    // a layout change does not touch linked books
    assert!(lib.plan_layout(&Layout::Flat).is_empty());

    // deleting removes the catalogue entry only
    lib.remove(&[id], true).unwrap();
    assert!(lib.get(id).is_none());
    assert!(epub.exists(), "the user's file is never deleted");
    assert!(lib.restore_from_trash(&lib.trash()[0].name.clone()).is_ok());
    assert!(lib.get(id).is_some());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn deleting_moves_to_the_trash_and_restore_brings_it_back() {
    let d = tmp();
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    let id = add(&mut lib, &d, "Dune", "Frank Herbert");
    let path = lib.book_path(lib.get(id).unwrap());
    lib.remove(&[id], true).unwrap();
    assert!(lib.get(id).is_none());
    assert!(!path.exists());
    assert!(!d.join("lib/Frank Herbert").exists());
    let t = lib.trash();
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].book.meta.title, "Dune");
    assert!(d
        .join("lib/.bookx-trash")
        .join(&t[0].name)
        .join("files")
        .exists());

    let restored = lib.restore_from_trash(&t[0].name).unwrap();
    assert_eq!(restored, id);
    assert!(path.exists());
    assert!(lib.trash().is_empty());
    assert_eq!(lib.open_book(id).unwrap().metadata().title, "Dune");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn emptying_the_trash_erases_for_good() {
    let d = tmp();
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    let a = add(&mut lib, &d, "Dune", "Frank Herbert");
    let b = add(&mut lib, &d, "Emma", "Jane Austen");
    lib.remove(&[a, b], true).unwrap();
    assert_eq!(lib.trash().len(), 2);
    assert_eq!(lib.empty_trash().unwrap(), 2);
    assert!(lib.trash().is_empty());
    assert!(all_files(&d.join("lib"))
        .iter()
        .all(|f| f.starts_with(".bookx/")));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn restore_refuses_when_something_is_in_the_way_and_changes_nothing() {
    let d = tmp();
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    let id = add(&mut lib, &d, "Dune", "Frank Herbert");
    let path = lib.book_path(lib.get(id).unwrap());
    lib.remove(&[id], true).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "squatter").unwrap();
    let name = lib.trash()[0].name.clone();
    assert!(lib.restore_from_trash(&name).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "squatter");
    assert_eq!(lib.trash().len(), 1, "still in the trash");
    assert!(lib.get(id).is_none());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn undo_takes_back_delete_import_edit_and_layout_change() {
    let d = tmp();
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    let id = add(&mut lib, &d, "Dune", "Frank Herbert");
    assert_eq!(lib.history().len(), 1);

    // edit
    let mut m = lib.get(id).unwrap().meta.clone();
    m.title = "Dune Messiah".into();
    lib.update_metadata(id, m).unwrap();
    assert_eq!(lib.undo_last().unwrap().unwrap(), "Edited \"Dune Messiah\"");
    assert_eq!(lib.get(id).unwrap().meta.title, "Dune");
    assert_eq!(lib.open_book(id).unwrap().metadata().title, "Dune");
    assert!(lib.book_path(lib.get(id).unwrap()).exists());

    // layout change
    lib.change_layout(Layout::Flat).unwrap();
    assert_eq!(lib.get(id).unwrap().file, "Frank Herbert - Dune.epub");
    lib.undo_last().unwrap();
    assert!(lib.get(id).unwrap().file.starts_with("Frank Herbert/Dune"));
    assert!(lib.book_path(lib.get(id).unwrap()).exists());

    // delete
    lib.remove(&[id], true).unwrap();
    lib.undo_last().unwrap();
    assert!(lib.get(id).is_some() && lib.book_path(lib.get(id).unwrap()).exists());

    // import (the last undoable operation left is the import itself)
    assert_eq!(lib.undo_last().unwrap().unwrap(), "Imported \"Dune\"");
    assert!(lib.get(id).is_none());
    assert_eq!(lib.undo_last().unwrap(), None);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn the_history_survives_a_reopen() {
    let d = tmp();
    {
        let mut lib = Library::in_memory(&d.join("lib")).unwrap();
        add(&mut lib, &d, "Dune", "Frank Herbert");
    }
    let lib = Library::in_memory(&d.join("lib")).unwrap();
    assert_eq!(lib.history().len(), 1);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_read_only_library_refuses_changes() {
    let d = tmp();
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    let id = add(&mut lib, &d, "Dune", "Frank Herbert");
    lib.set_read_only(true);
    assert!(lib.is_read_only());
    assert!(lib.remove(&[id], true).is_err());
    assert!(lib.change_layout(Layout::Flat).is_err());
    assert!(lib
        .add_path(&md_book(&d, "Emma", "Jane Austen"), false)
        .is_err());
    assert!(lib
        .update_metadata(id, lib.get(id).unwrap().meta.clone())
        .is_err());
    assert!(lib.save().is_ok(), "saving is a no-op");
    assert!(lib.get(id).is_some());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_failed_import_leaves_no_files_behind() {
    let d = tmp();
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    lib.set_layout(Layout::Flat).unwrap();
    // the cover would go to ".covers/1.png", but ".covers" is a file
    std::fs::create_dir_all(d.join("lib")).unwrap();
    std::fs::write(d.join("lib/.covers"), "not a folder").unwrap();
    let p = bookx_core::library::Prepared {
        meta: bookx_core::opf::Metadata {
            title: "T".into(),
            authors: vec!["A".into()],
            ..Default::default()
        },
        cover: Some((vec![1, 2, 3], "png".into())),
        bytes: vec![0; 10],
        origin: None,
        drm: String::new(),
        source_file: None,
    };
    assert!(lib.commit_import(p, true).is_err());
    assert!(lib.books.is_empty());
    assert!(
        !d.join("lib/A - T.epub").exists(),
        "the book file is removed again"
    );
    let _ = std::fs::remove_dir_all(d);
}
