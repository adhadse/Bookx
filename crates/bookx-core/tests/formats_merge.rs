//! Several formats in one book, undo of cover replacements and grouped edits.

use std::path::{Path, PathBuf};

use bookx_core::formats::Target;
use bookx_core::history::CoverUndo;
use bookx_core::library::{AddOutcome, Library};

fn tmp() -> PathBuf {
    let d = std::env::temp_dir().join(format!("bookx-merge-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn add(lib: &mut Library, scratch: &Path, title: &str, author: &str) -> u64 {
    let p = scratch.join(format!("{title}.md"));
    std::fs::write(
        &p,
        format!("---\ntitle: \"{title}\"\nauthor: \"{author}\"\n---\n# One\n\nHello {title}.\n"),
    )
    .unwrap();
    match lib.add_path(&p, true).unwrap() {
        AddOutcome::Added(id) => id,
        AddOutcome::Duplicate(_) => panic!("duplicate"),
    }
}

fn fake(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

/// Two books where the second only has a non-EPUB file: a linked-style record we build by adding
/// a format then folding it.
fn two_formats(lib: &mut Library, d: &Path) -> (u64, u64) {
    let a = add(lib, d, "Dune", "Frank Herbert");
    let b = add(lib, d, "Dune Copy", "Frank Herbert");
    // turn the second book's file into an AZW3 so the formats differ
    let old = lib.get(b).unwrap().file.clone();
    let new = old.replace(".epub", ".azw3");
    std::fs::rename(lib.root.join(&old), lib.root.join(&new)).unwrap();
    lib.get_mut(b).unwrap().file = new;
    (a, b)
}

#[test]
fn merging_makes_one_book_with_two_formats_and_undo_restores_both() {
    let d = tmp();
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    let (a, b) = two_formats(&mut lib, &d);
    let plan = lib.plan_merge(a, &[b]).unwrap();
    assert_eq!(plan.mergeable().len(), 1);
    assert_eq!(plan.summary(), "Dune: epub + azw3");
    assert_eq!(lib.books.len(), 2, "planning changes nothing");

    assert_eq!(lib.merge_formats(a, &[b]).unwrap(), 1);
    assert_eq!(lib.books.len(), 1);
    let book = lib.get(a).unwrap().clone();
    assert!(book.file.ends_with(".epub"));
    assert_eq!(book.formats.len(), 1);
    assert_eq!(book.formats[0].format, "azw3");
    assert!(lib.root.join(&book.formats[0].file).exists());
    // the extra file moved into the primary's folder
    assert_eq!(
        Path::new(&book.formats[0].file).parent(),
        Path::new(&book.file).parent()
    );
    // each target gets its own file
    assert!(book.file_for(Target::Kindle).ends_with(".azw3"));
    assert!(book.file_for(Target::Reader).ends_with(".epub"));
    assert!(lib.open_book(a).is_ok());

    let label = lib.undo_last().unwrap().unwrap();
    assert!(label.starts_with("Merged"));
    assert_eq!(lib.books.len(), 2);
    assert!(lib.get(a).unwrap().formats.is_empty());
    let back = lib.get(b).unwrap();
    assert!(lib.root.join(&back.file).exists(), "{}", back.file);
    assert!(back.file.ends_with(".azw3"));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn two_books_with_the_same_format_are_not_merged() {
    let d = tmp();
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    let a = add(&mut lib, &d, "One", "A");
    let b = add(&mut lib, &d, "Two", "A");
    let plan = lib.plan_merge(a, &[b]).unwrap();
    assert!(plan.mergeable().is_empty());
    assert!(plan.items[0].blocked.as_ref().unwrap().contains("EPUB"));
    assert!(lib.merge_formats(a, &[b]).is_err());
    assert_eq!(lib.books.len(), 2);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn the_reader_gets_the_epub_even_when_the_primary_is_not_one() {
    let d = tmp();
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    let (a, b) = two_formats(&mut lib, &d);
    // merge the epub book INTO the azw3 one
    lib.merge_formats(b, &[a]).unwrap();
    let book = lib.get(b).unwrap();
    assert!(book.file.ends_with(".epub"), "{}", book.file);
    assert!(book.formats[0].file.ends_with(".azw3"));
    lib.undo_last().unwrap();
    assert!(lib.get(b).unwrap().file.ends_with(".azw3"));
    assert!(lib.get(a).is_some());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn deleting_a_merged_book_takes_all_its_format_files_to_the_trash() {
    let d = tmp();
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    let (a, b) = two_formats(&mut lib, &d);
    lib.merge_formats(a, &[b]).unwrap();
    let extra = lib.get(a).unwrap().formats[0].file.clone();
    lib.remove(&[a], true).unwrap();
    assert!(!lib.root.join(&extra).exists());
    lib.undo_last().unwrap();
    assert!(lib.root.join(&extra).exists());
    assert_eq!(lib.get(a).unwrap().formats.len(), 1);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn adding_and_removing_one_format_can_be_undone() {
    let d = tmp();
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    let a = add(&mut lib, &d, "Dune", "Frank Herbert");
    let src = fake(&d, "whatever.mobi", b"MOBI-DATA");
    let f = lib.add_format(a, &src).unwrap();
    assert_eq!(f.format, "mobi");
    assert!(lib.root.join(&f.file).exists());
    assert!(src.exists(), "the original is kept");
    assert!(lib.add_format(a, &src).is_err(), "no second MOBI");

    lib.remove_format(a, &f.file).unwrap();
    assert!(lib.get(a).unwrap().formats.is_empty());
    assert!(!lib.root.join(&f.file).exists());
    lib.undo_last().unwrap();
    assert_eq!(lib.get(a).unwrap().formats.len(), 1);
    assert!(lib.root.join(&f.file).exists());

    lib.undo_last().unwrap(); // the add
    assert!(lib.get(a).unwrap().formats.is_empty());
    assert!(!lib.root.join(&f.file).exists());
    // the main file cannot be removed that way
    let main = lib.get(a).unwrap().file.clone();
    assert!(lib.remove_format(a, &main).is_err());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn the_same_book_is_found_by_identifier_or_title() {
    let d = tmp();
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    let a = add(&mut lib, &d, "Dune", "Frank Herbert");
    let mut m = lib.get(a).unwrap().meta.clone();
    m.title = "Dune (Kindle edition)".into();
    m.identifiers = vec![];
    assert_eq!(lib.find_same_book(&m), None);
    m.title = "dune".into();
    assert_eq!(lib.find_same_book(&m), Some(a));
    let mut with_id = lib.get(a).unwrap().meta.clone();
    with_id.identifiers = vec![bookx_core::opf::Identifier {
        scheme: "isbn".into(),
        value: "978-0-441-17271-9".into(),
        xml_id: None,
    }];
    lib.update_metadata(a, with_id).unwrap();
    let mut other = m.clone();
    other.title = "Something else".into();
    other.identifiers = vec![bookx_core::opf::Identifier {
        scheme: "ISBN".into(),
        value: "9780441172719".into(),
        xml_id: None,
    }];
    assert_eq!(lib.find_same_book(&other), Some(a));
    let _ = std::fs::remove_dir_all(d);
}

fn png(n: u8) -> Vec<u8> {
    let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    v.extend(std::iter::repeat_n(n, 64));
    v
}

fn set_cover(lib: &mut Library, id: u64, data: Vec<u8>) {
    let path = lib.book_path(lib.get(id).unwrap());
    let mut ob = bookx_core::book::OpenBook::open(&path).unwrap();
    ob.set_cover(data, "png").unwrap();
    ob.archive.save(&path).unwrap();
    let m = lib.get(id).unwrap().meta.clone();
    lib.update_metadata(id, m).unwrap();
}

fn cover_bytes(lib: &Library, id: u64) -> Option<Vec<u8>> {
    lib.cover_path(lib.get(id).unwrap())
        .and_then(|p| std::fs::read(p).ok())
}

#[test]
fn undo_restores_a_replaced_cover() {
    let d = tmp();
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    let a = add(&mut lib, &d, "Dune", "Frank Herbert");
    let original = cover_bytes(&lib, a);
    set_cover(&mut lib, a, png(1));
    assert_eq!(cover_bytes(&lib, a).unwrap(), png(1));
    set_cover(&mut lib, a, png(2));
    assert_eq!(cover_bytes(&lib, a).unwrap(), png(2));
    lib.undo_last().unwrap();
    assert_eq!(cover_bytes(&lib, a).unwrap(), png(1), "catalogue cover");
    let ob = lib.open_book(a).unwrap();
    let inside = ob
        .archive
        .get(&ob.package.cover_path().unwrap())
        .unwrap()
        .to_vec();
    assert_eq!(inside, png(1), "cover inside the book file");
    // undoing the very first cover works too (a book file keeps an image named "cover" listed, so
    // the catalogue may still show it: see docs/metadata-notes.md)
    assert_eq!(original, None);
    lib.undo_last().unwrap();
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn cover_backups_respect_the_size_caps() {
    let d = tmp();
    assert_eq!(
        bookx_core::files::backup_cover_capped(&d, 1, &[0u8; 100], "jpg", 50, 1000),
        CoverUndo::TooBig
    );
    let mut names = Vec::new();
    for i in 0..5 {
        let CoverUndo::Saved(n) =
            bookx_core::files::backup_cover_capped(&d, i, &[1u8; 100], "jpg", 200, 250)
        else {
            panic!()
        };
        names.push(n);
    }
    let kept = std::fs::read_dir(d.join(".bookx/cover-backups"))
        .unwrap()
        .count();
    assert_eq!(kept, 2, "250 bytes hold two 100 byte covers");
    assert!(d.join(names.last().unwrap()).exists(), "the newest stays");
    assert!(!d.join(&names[0]).exists(), "the oldest went first");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn one_undo_takes_back_a_grouped_edit() {
    let d = tmp();
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    let a = add(&mut lib, &d, "One", "J. Smith");
    let b = add(&mut lib, &d, "Two", "J. Smith");
    let c = add(&mut lib, &d, "Three", "Other");
    let edits: Vec<_> = [a, b, c]
        .iter()
        .map(|id| {
            let mut m = lib.get(*id).unwrap().meta.clone();
            for au in &mut m.authors {
                if au == "J. Smith" {
                    *au = "John Smith".into();
                }
            }
            (*id, m)
        })
        .collect();
    assert_eq!(
        lib.update_metadata_many("Renamed author".into(), edits)
            .unwrap(),
        2
    );
    assert_eq!(lib.get(a).unwrap().meta.authors, vec!["John Smith"]);
    assert_eq!(
        lib.history().len(),
        1 + 1,
        "the imports join into one step, then one grouped edit"
    );
    lib.undo_last().unwrap();
    assert_eq!(lib.get(a).unwrap().meta.authors, vec!["J. Smith"]);
    assert_eq!(lib.get(b).unwrap().meta.authors, vec!["J. Smith"]);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn linked_resources_are_saved_and_undone() {
    use bookx_core::formats::{AUDIOBOOK, Resource};
    let d = tmp();
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    let a = add(&mut lib, &d, "Dune", "Frank Herbert");
    let r = Resource {
        kind: AUDIOBOOK.into(),
        path: "/audio/Dune.m4b".into(),
        label: "Dune.m4b".into(),
    };
    lib.set_resources(a, vec![r.clone()], "Linked audiobook".into())
        .unwrap();
    assert_eq!(lib.get(a).unwrap().resources, vec![r]);
    lib.undo_last().unwrap();
    assert!(lib.get(a).unwrap().resources.is_empty());
    let _ = std::fs::remove_dir_all(d);
}

fn protected_epub(d: &Path, extra: &[(&str, &str)]) -> PathBuf {
    let mut scratch = Library::in_memory(&d.join("scratch")).unwrap();
    let id = add(&mut scratch, d, "Locked Book", "Some Author");
    let src = scratch.root.join(&scratch.get(id).unwrap().file);
    let mut arc = bookx_core::archive::Archive::open(&src).unwrap();
    for (n, c) in extra {
        arc.set_string(n, c);
    }
    let out = d.join("locked.epub");
    arc.save(&out).unwrap();
    out
}

#[test]
fn a_protected_epub_is_imported_with_a_badge_and_never_decrypted() {
    let d = tmp();
    let aes = r#"<encryption><EncryptedData><EncryptionMethod Algorithm="http://www.w3.org/2001/04/xmlenc#aes128-cbc"/></EncryptedData></encryption>"#;
    let p = protected_epub(
        &d,
        &[
            ("META-INF/encryption.xml", aes),
            (
                "META-INF/rights.xml",
                r#"<rights xmlns="http://ns.adobe.com/adept"/>"#,
            ),
        ],
    );
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    let prepared = bookx_core::library::prepare_import(&p).unwrap();
    assert_eq!(prepared.drm, "Adobe ADEPT");
    let bytes_before = std::fs::read(&p).unwrap();
    let AddOutcome::Added(id) = lib.commit_import(prepared, true).unwrap() else {
        panic!()
    };
    assert_eq!(lib.get(id).unwrap().drm, "Adobe ADEPT");
    // the stored copy is byte-for-byte what was imported apart from the OPF round trip: the
    // encryption files are still there
    let stored =
        bookx_core::archive::Archive::open(&lib.root.join(&lib.get(id).unwrap().file)).unwrap();
    assert!(stored.contains("META-INF/encryption.xml"));
    assert_eq!(std::fs::read(&p).unwrap(), bytes_before);
    // a plain EPUB has no badge
    let plain = protected_epub(&d, &[]);
    assert_eq!(bookx_core::library::prepare_import(&plain).unwrap().drm, "");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_pdf_import_keeps_the_pdf_as_a_format_of_the_book() {
    let d = tmp();
    let src = fake(&d, "Paper.pdf", b"%PDF-1.4 stand-in");
    let md = d.join("Paper.md");
    std::fs::write(&md, "# One\n\nText of the reflowed copy.\n").unwrap();
    let mut prepared = bookx_core::library::prepare_import(&md).unwrap();
    prepared.source_file = Some(src.clone());
    let mut lib = Library::in_memory(&d.join("lib")).unwrap();
    let AddOutcome::Added(id) = lib.commit_import(prepared, true).unwrap() else {
        panic!()
    };
    let b = lib.get(id).unwrap();
    assert_eq!(
        b.all_formats()
            .iter()
            .map(|f| f.format.as_str())
            .collect::<Vec<_>>(),
        ["epub", "pdf"]
    );
    let pdf = lib.best_existing(b, Target::Convert).unwrap();
    assert!(pdf.exists());
    assert_eq!(
        std::fs::read(lib.root.join(&b.formats[0].file)).unwrap(),
        b"%PDF-1.4 stand-in"
    );
    // any other source file is not copied
    let mut p2 = bookx_core::library::prepare_import(&md).unwrap();
    p2.source_file = Some(fake(&d, "Other.txt", b"x"));
    let AddOutcome::Added(id2) = lib.commit_import(p2, true).unwrap() else {
        panic!()
    };
    assert!(lib.get(id2).unwrap().formats.is_empty());
    let _ = std::fs::remove_dir_all(d);
}
