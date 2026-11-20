//! Which files the library accepts and how they become EPUB books: everything `bookx-formats`
//! can read, plus what the engine imports itself (roadmap C2/C17).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use bookx_formats::{read_any, supported_extensions};

use crate::core::library::{self, Prepared};

/// Lower-case extensions (without the dot) that can be added to the library.
pub fn extensions() -> Vec<String> {
    let mut v: Vec<String> = supported_extensions()
        .into_iter()
        .map(|e| e.to_ascii_lowercase())
        .collect();
    for e in ["epub", "txt", "md", "markdown", "html", "htm", "xhtml"] {
        v.push(e.to_string());
    }
    // file formats added by enabled plugins
    v.extend(crate::plugins::reader_extensions(&crate::plugins::current()));
    v.sort();
    v.dedup();
    v
}

pub fn is_supported(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .is_some_and(|e| extensions().contains(&e))
}

/// Book files in `dir` (sorted), optionally including sub-folders. Hidden entries are skipped.
pub fn collect(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        let hidden = p
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with('.'));
        if hidden {
            continue;
        }
        if p.is_dir() {
            if recursive {
                collect(&p, true, out);
            }
        } else if is_supported(&p) {
            out.push(p);
        }
    }
}

/// Reads any supported file into a ready-to-add book. Safe to call from worker threads.
pub fn prepare(path: &Path) -> Result<Prepared> {
    // audio (a file, a folder, a zip of parts) is an audiobook, not a book to convert
    if !is_supported(path) && crate::audiobook::is_audio_source(path) {
        let data = crate::audiobook::data_dir().context("no settings folder")?;
        return crate::audiobook::prepare(path, &data);
    }
    let mut raw = prepare_unpatched(path)?;
    raw.source_file = Some(path.to_path_buf());
    let before = raw.meta.clone();
    let (prepared, patches) =
        crate::plugins::patch_prepared_noted(&crate::plugins::current_patches(), raw, path)?;
    if !patches.is_empty() {
        // the text of the book changed, and possibly what was read from it
        let mut fields = crate::metadata_fetch::changed_fields(&before, &prepared.meta);
        fields.push("content");
        note_patched(&prepared, fields, &patches);
    }
    Ok(prepared)
}

/// Fields (`content` and metadata fields) that import patches changed in a prepared book: (title, size, fields, patches).
type PatchNote = (String, usize, Vec<&'static str>, Vec<String>);

static PATCH_NOTES: std::sync::Mutex<Vec<PatchNote>> = std::sync::Mutex::new(Vec::new());

fn note_patched(p: &Prepared, fields: Vec<&'static str>, patches: &[String]) {
    PATCH_NOTES.lock().unwrap_or_else(|e| e.into_inner()).push((
        p.meta.title.clone(),
        p.bytes.len(),
        fields,
        patches.to_vec(),
    ));
}

/// The fields an import patch changed in `p` and the source to record for them
/// (`patch: id, id`), once; `None` when no patch touched the metadata.
pub fn take_patch_note(p: &Prepared) -> Option<(Vec<&'static str>, String)> {
    let mut notes = PATCH_NOTES.lock().unwrap_or_else(|e| e.into_inner());
    let i = notes
        .iter()
        .position(|(t, size, _, _)| *t == p.meta.title && *size == p.bytes.len())?;
    let (_, _, fields, patches) = notes.remove(i);
    Some((fields, format!("patch: {}", patches.join(", "))))
}

fn prepare_unpatched(path: &Path) -> Result<Prepared> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    // a protected file that is not an EPUB cannot be converted; say why instead of failing oddly.
    // (A protected EPUB is added with a badge so that it is listed; Bookx never removes DRM.)
    if ext != "epub" {
        if let Some(d) = crate::core::drm::detect(path) {
            anyhow::bail!("{}: {}", path.display(), d.message());
        }
    }
    // the engine's own importers keep their behaviour (EPUB, TXT, Markdown, HTML)
    if matches!(
        ext.as_str(),
        "epub" | "txt" | "md" | "markdown" | "html" | "htm" | "xhtml"
    ) {
        return library::prepare_import(path);
    }
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("book");
    let plugins = crate::plugins::current();
    let by_plugin = plugins
        .readers
        .iter()
        .any(|r| r.extensions().iter().any(|e| e.eq_ignore_ascii_case(&ext)));
    let archive = if by_plugin {
        crate::plugins::read(&plugins, name, &bytes)
    } else {
        read_any(name, &bytes)
    }
    .with_context(|| format!("importing {}", path.display()))?;
    library::prepare_from_archive(archive, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn a_patch_note_is_handed_out_once_for_the_book_it_belongs_to() {
        let p = Prepared {
            meta: crate::core::opf::Metadata {
                title: "Patched Note Book".into(),
                ..Default::default()
            },
            cover: None,
            bytes: vec![1, 2, 3],
            origin: None,
            drm: String::new(),
            source_file: None,
        };
        assert!(take_patch_note(&p).is_none());
        note_patched(
            &p,
            vec!["title"],
            &["fix-titles".to_string(), "b".to_string()],
        );
        let (fields, source) = take_patch_note(&p).unwrap();
        assert_eq!(fields, vec!["title"]);
        assert_eq!(source, "patch: fix-titles, b");
        assert!(take_patch_note(&p).is_none(), "taken once");
        let other = Prepared {
            bytes: vec![1],
            ..p
        };
        assert!(take_patch_note(&other).is_none());
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "bookx-import-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    const FB2: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0"><description><title-info>
<author><first-name>Stan</first-name><last-name>Lem</last-name></author>
<book-title>Solaris</book-title><lang>en</lang></title-info></description>
<body><section><title><p>One</p></title><p>The ocean was alive.</p></section></body></FictionBook>"#;

    #[test]
    fn accepts_the_formats_the_readers_know() {
        for f in [
            "a.epub", "a.EPUB", "a.fb2", "a.fbz", "a.docx", "a.odt", "a.rtf", "a.mobi", "a.azw3",
            "a.cbz", "a.txt", "a.md", "a.html", "a.txtz", "a.htmlz", "a.lrf", "a.pdf", "a.PDF",
        ] {
            assert!(is_supported(Path::new(f)), "{f}");
        }
        for f in ["a.jpg", "a", "a.exe", ".epub-not"] {
            assert!(!is_supported(Path::new(f)), "{f}");
        }
    }

    #[test]
    fn extension_list_is_sorted_unique_and_lowercase() {
        let e = extensions();
        let mut s = e.clone();
        s.sort();
        s.dedup();
        assert_eq!(e, s);
        assert!(e.iter().all(|x| *x == x.to_ascii_lowercase()));
        assert!(e.contains(&"fb2".to_string()) && e.contains(&"epub".to_string()));
    }

    #[test]
    fn collects_books_recursively_in_order_and_skips_hidden_and_other_files() {
        let d = tmp("collect");
        fs::create_dir_all(d.join("sub/deeper")).unwrap();
        fs::create_dir_all(d.join(".hidden")).unwrap();
        for f in [
            "b.fb2",
            "a.epub",
            "notes.jpg",
            ".dot.epub",
            "sub/c.docx",
            "sub/deeper/d.txt",
            ".hidden/e.epub",
        ] {
            fs::write(d.join(f), "x").unwrap();
        }
        let mut flat = Vec::new();
        collect(&d, false, &mut flat);
        let names = |v: &[PathBuf]| -> Vec<String> {
            v.iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().into())
                .collect()
        };
        assert_eq!(names(&flat), vec!["a.epub", "b.fb2"]);
        let mut deep = Vec::new();
        collect(&d, true, &mut deep);
        assert_eq!(names(&deep), vec!["a.epub", "b.fb2", "c.docx", "d.txt"]);
        let mut none = Vec::new();
        collect(&d.join("missing"), true, &mut none);
        assert!(none.is_empty());
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn prepares_an_fb2_book_with_metadata() {
        let d = tmp("fb2");
        let p = d.join("solaris.fb2");
        fs::write(&p, FB2).unwrap();
        let prepared = prepare(&p).unwrap();
        assert_eq!(prepared.meta.title, "Solaris");
        assert!(prepared.meta.authors[0].contains("Lem"));
        assert!(prepared.bytes.starts_with(b"PK"));
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn prepares_text_through_the_engine_importer() {
        let d = tmp("txt");
        let p = d.join("note.txt");
        fs::write(&p, "A title\n\nSome words.").unwrap();
        let prepared = prepare(&p).unwrap();
        assert!(!prepared.meta.title.is_empty());
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn a_broken_file_is_an_error_naming_the_file() {
        let d = tmp("broken");
        let p = d.join("bad.fb2");
        fs::write(&p, "not xml at all").unwrap();
        let err = prepare(&p).unwrap_err();
        assert!(format!("{err:#}").contains("bad.fb2"), "{err:#}");
        let missing = prepare(&d.join("nope.fb2")).unwrap_err();
        assert!(format!("{missing:#}").contains("nope.fb2"));
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn a_book_without_a_title_gets_its_file_name() {
        let d = tmp("untitled");
        let p = d.join("Some File Name.fb2");
        fs::write(
            &p,
            r#"<?xml version="1.0"?><FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0"><description><title-info><book-title></book-title></title-info></description><body><section><p>text here</p></section></body></FictionBook>"#,
        )
        .unwrap();
        let prepared = prepare(&p).unwrap();
        assert!(!prepared.meta.title.trim().is_empty());
        assert_eq!(prepared.meta.authors, vec!["Unknown"]);
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn a_pdf_is_imported_with_its_metadata_text_and_kept_as_a_format() {
        use bookx_pdf::testpdf::TestPdf;
        let d = tmp("pdf");
        let pdf = TestPdf::simple(&[&["The quick brown fox jumps over the lazy dog."]])
            .title("Foxes")
            .author("Ann Author")
            .build();
        let path = d.join("whatever.pdf");
        fs::write(&path, pdf).unwrap();
        let p = prepare(&path).unwrap();
        assert_eq!(p.meta.title, "Foxes");
        assert_eq!(p.meta.authors, vec!["Ann Author".to_string()]);
        assert!(p.cover.is_some(), "page 1 is the cover");
        assert_eq!(p.source_file.as_deref(), Some(path.as_path()));
        let lib_dir = d.join("lib");
        let mut lib = crate::core::library::Library::in_memory(&lib_dir).unwrap();
        let crate::core::library::AddOutcome::Added(id) = lib.commit_import(p, false).unwrap()
        else {
            panic!("not added")
        };
        let b = lib.get(id).unwrap();
        let formats: Vec<&str> = b
            .all_formats()
            .iter()
            .map(|f| f.format.clone())
            .map(|f| if f == "pdf" { "pdf" } else { "other" })
            .collect();
        assert_eq!(formats, vec!["other", "pdf"]);
        let text = crate::core::book::OpenBook::open(&lib.book_path(b)).unwrap();
        let first = text.archive.get_string("OEBPS/chapter-1.xhtml").unwrap();
        assert!(
            first.contains("quick brown fox"),
            "the EPUB holds the text for search: {first}"
        );
    }
}
