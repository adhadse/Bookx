//! End-to-end behaviour of `convert` / `convert_file`: readers, transforms, writers, progress and
//! cancellation.

mod common;

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::bail;
use bookx_convert::transforms::{AutoTocMode, MetadataOverride, ProfileKind};
use bookx_convert::{
    builtin_presets, convert, convert_file, read_input, Cancelled, ConvertOptions, Progress,
};
use bookx_core::archive::Archive;
use bookx_core::book::OpenBook;
use bookx_core::contracts::{FormatReader, Registry};

fn no_readers() -> Registry<dyn FormatReader> {
    Registry::new()
}

fn epub_bytes() -> Vec<u8> {
    common::sample_book().archive.to_bytes().unwrap()
}

fn run(
    input: &[u8],
    name: &str,
    ext: &str,
    opts: &ConvertOptions,
) -> anyhow::Result<(bookx_convert::Converted, Vec<Progress>)> {
    let log = RefCell::new(Vec::new());
    let cancel = AtomicBool::new(false);
    let out = convert(
        &no_readers(),
        input,
        name,
        ext,
        opts,
        &|p| log.borrow_mut().push(p),
        &cancel,
    )?;
    Ok((out, log.into_inner()))
}

fn reopen(bytes: &[u8]) -> OpenBook {
    OpenBook::from_archive(Archive::from_bytes(bytes.to_vec()).unwrap()).unwrap()
}

#[test]
fn converts_epub_to_every_supported_format() {
    let input = epub_bytes();
    let cases = [
        ("epub", "epub"),
        (".EPUB", "epub"),
        ("kepub", "kepub.epub"),
        ("kepub.epub", "kepub.epub"),
        ("fb2", "fb2"),
        ("htmlz", "htmlz"),
        ("txtz", "txtz"),
        ("txt", "txt"),
        ("md", "md"),
        ("html", "html"),
        ("mobi", "mobi"),
        ("azw3", "azw3"),
        ("KF8", "azw3"),
    ];
    for (ask, got) in cases {
        let (out, _) = run(&input, "book.epub", ask, &ConvertOptions::default())
            .unwrap_or_else(|e| panic!("{ask}: {e:#}"));
        assert_eq!(out.extension, got, "{ask}");
        assert!(out.bytes.len() > 200, "{ask}");
    }
}

#[test]
fn unsupported_outputs_have_helpful_errors() {
    let input = epub_bytes();
    let e = run(&input, "b.epub", "pdf", &ConvertOptions::default()).unwrap_err();
    assert!(
        e.to_string().contains("unsupported output format `pdf`"),
        "{e}"
    );
    for known in [
        "mobi", "docx", "rtf", "pdb", "pmlz", "rb", "snb", "oeb", "zip",
    ] {
        assert!(e.to_string().contains(known), "{known} missing from: {e}");
    }
}

#[test]
fn plain_text_input_goes_through_the_builtin_importer() {
    let txt = "Chapter 1\n\nIt was a \"dark\" night.\n\nChapter 2\n\nMorning came -- slowly...\n";
    let opts = ConvertOptions {
        smarten_punctuation: true,
        ..Default::default()
    };
    let (out, _) = run(txt.as_bytes(), "My Story.txt", "epub", &opts).unwrap();
    let book = reopen(&out.bytes);
    assert_eq!(book.metadata().title, "My Story");
    assert_eq!(book.spine_paths().len(), 2);
    let all: String = book
        .spine_paths()
        .iter()
        .map(|p| book.archive.get_string(p).unwrap())
        .collect();
    assert!(all.contains("“dark”"), "smart quotes applied");
    assert!(all.contains("–") && all.contains("…"));
}

#[test]
fn markdown_and_html_inputs() {
    let (out, _) = run(
        b"---\ntitle: \"Doc\"\n---\n# One\n\nHello *world*\n",
        "d.md",
        "txt",
        &ConvertOptions::default(),
    )
    .unwrap();
    assert!(String::from_utf8(out.bytes)
        .unwrap()
        .contains("Hello world"));
    let (out, _) = run(
        b"<html><head><title>T</title></head><body><p>Hi</p></body></html>",
        "page.html",
        "md",
        &ConvertOptions::default(),
    )
    .unwrap();
    assert!(String::from_utf8(out.bytes).unwrap().contains("Hi"));
}

struct StubReader;

impl FormatReader for StubReader {
    fn id(&self) -> &'static str {
        "stub"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["stub"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        head.starts_with(b"STUBFILE")
    }
    fn read(&self, bytes: &[u8]) -> anyhow::Result<Archive> {
        if bytes.ends_with(b"BROKEN") {
            bail!("corrupt stub file");
        }
        Ok(common::sample_book().archive)
    }
}

#[test]
fn readers_registry_is_consulted_by_extension_then_content() {
    let mut readers: Registry<dyn FormatReader> = Registry::new();
    readers.register(Box::new(StubReader));
    let cancel = AtomicBool::new(false);
    let opts = ConvertOptions::default();
    let out = convert(
        &readers,
        b"STUBFILE ok",
        "x.stub",
        "txt",
        &opts,
        &|_| {},
        &cancel,
    )
    .unwrap();
    assert!(String::from_utf8(out.bytes)
        .unwrap()
        .contains("The end of the story."));
    // wrong extension, right magic
    let out = convert(
        &readers,
        b"STUBFILE ok",
        "x.bin",
        "txt",
        &opts,
        &|_| {},
        &cancel,
    )
    .unwrap();
    assert!(!out.bytes.is_empty());
    // the reader's own error is wrapped with the file name
    let e = convert(
        &readers,
        b"STUBFILE BROKEN",
        "x.stub",
        "txt",
        &opts,
        &|_| {},
        &cancel,
    )
    .unwrap_err();
    assert!(format!("{e:#}").contains("reading x.stub as stub"), "{e:#}");
    assert!(format!("{e:#}").contains("corrupt stub file"));
    // nothing claims it
    let e = read_input(&readers, b"hello", "mystery.xyz").err().unwrap();
    assert!(e.to_string().contains("no reader for `mystery.xyz`"));
}

#[test]
fn epub_with_unknown_extension_is_recognised_by_content() {
    let (out, _) = run(
        &epub_bytes(),
        "download.bin",
        "txt",
        &ConvertOptions::default(),
    )
    .unwrap();
    assert!(!out.bytes.is_empty());
}

#[test]
fn progress_is_monotonic_and_ends_at_100() {
    let opts = ConvertOptions {
        smarten_punctuation: true,
        remove_unused_css: true,
        remove_blank_paragraphs: true,
        device: Some(ProfileKind::Kindle),
        auto_toc: Some(AutoTocMode::Force),
        ..Default::default()
    };
    let (out, log) = run(&epub_bytes(), "b.epub", "mobi", &opts).unwrap();
    assert!(log.len() >= 8, "{log:?}");
    assert_eq!(log.first().unwrap().percent, 0);
    assert_eq!(
        log.last().unwrap(),
        &Progress {
            percent: 100,
            stage: "done".into()
        }
    );
    for w in log.windows(2) {
        assert!(
            w[0].percent <= w[1].percent,
            "progress went backwards: {log:?}"
        );
    }
    assert!(log.iter().all(|p| p.percent <= 100));
    // every transform shows up as a stage and reports
    let ids: Vec<&str> = out.reports.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "remove_blank_paragraphs",
            "smarten_punctuation",
            "remove_unused_css",
            "device_profile",
            "auto_toc"
        ]
    );
    for (id, _) in &out.reports {
        assert!(log.iter().any(|p| p.stage == *id), "{id}");
    }
}

#[test]
fn progress_without_transforms_is_still_complete() {
    let (_, log) = run(&epub_bytes(), "b.epub", "txt", &ConvertOptions::default()).unwrap();
    assert_eq!(log.last().unwrap().percent, 100);
    assert!(log.iter().any(|p| p.stage == "writing"));
}

#[test]
fn cancel_before_start_does_nothing() {
    let cancel = AtomicBool::new(true);
    let calls = RefCell::new(0);
    let e = convert(
        &no_readers(),
        &epub_bytes(),
        "b.epub",
        "txt",
        &ConvertOptions::default(),
        &|_| *calls.borrow_mut() += 1,
        &cancel,
    )
    .unwrap_err();
    assert!(e.downcast_ref::<Cancelled>().is_some());
    assert_eq!(e.to_string(), "conversion cancelled");
    assert_eq!(*calls.borrow(), 0);
}

#[test]
fn cancel_midway_returns_an_error_and_stops_the_remaining_stages() {
    let opts = ConvertOptions {
        smarten_punctuation: true,
        remove_unused_css: true,
        auto_toc: Some(AutoTocMode::Force),
        ..Default::default()
    };
    let cancel = AtomicBool::new(false);
    let stages = RefCell::new(Vec::new());
    let e = convert(
        &no_readers(),
        &epub_bytes(),
        "b.epub",
        "epub",
        &opts,
        &|p| {
            stages.borrow_mut().push(p.stage.clone());
            if p.stage == "smarten_punctuation" {
                cancel.store(true, Ordering::SeqCst);
            }
        },
        &cancel,
    )
    .unwrap_err();
    assert!(e.downcast_ref::<Cancelled>().is_some());
    let stages = stages.into_inner();
    assert!(stages.contains(&"smarten_punctuation".to_string()));
    assert!(
        !stages.contains(&"remove_unused_css".to_string()),
        "later stages must not run: {stages:?}"
    );
    assert!(!stages.contains(&"writing".to_string()));
    assert!(!stages.contains(&"done".to_string()));
}

#[test]
fn cancelled_file_conversion_leaves_nothing_behind() {
    let dir = common::temp_dir("cancel");
    let input = dir.join("in.epub");
    std::fs::write(&input, epub_bytes()).unwrap();
    for stage in ["read", "writing"] {
        let output = dir.join("out").join("book.mobi");
        let cancel = AtomicBool::new(false);
        let e = convert_file(
            &no_readers(),
            &input,
            &output,
            &ConvertOptions::default(),
            &|p| {
                if p.stage == stage {
                    cancel.store(true, Ordering::SeqCst);
                }
            },
            &cancel,
        )
        .unwrap_err();
        assert!(e.downcast_ref::<Cancelled>().is_some(), "{stage}");
        assert!(!output.exists(), "{stage}: no partial output");
        if let Ok(rd) = std::fs::read_dir(dir.join("out")) {
            assert_eq!(rd.count(), 0, "{stage}: no temp files");
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn failed_file_conversion_leaves_nothing_behind() {
    let dir = common::temp_dir("fail");
    let input = dir.join("in.epub");
    std::fs::write(&input, b"this is not an epub at all").unwrap();
    let output = dir.join("out.epub");
    let cancel = AtomicBool::new(false);
    let e = convert_file(
        &no_readers(),
        &input,
        &output,
        &ConvertOptions::default(),
        &|_| {},
        &cancel,
    )
    .unwrap_err();
    assert!(!format!("{e:#}").is_empty());
    assert!(!output.exists());
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        1,
        "only the input remains"
    );
    let missing = convert_file(
        &no_readers(),
        &dir.join("nope.epub"),
        &output,
        &ConvertOptions::default(),
        &|_| {},
        &cancel,
    )
    .unwrap_err();
    assert!(format!("{missing:#}").contains("nope.epub"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn file_conversion_writes_atomically_and_picks_the_format_from_the_name() {
    let dir = common::temp_dir("file");
    let input = dir.join("in.epub");
    std::fs::write(&input, epub_bytes()).unwrap();
    let cancel = AtomicBool::new(false);
    for (name, ext) in [
        ("a.mobi", "mobi"),
        ("a2.azw3", "azw3"),
        ("b.kepub.epub", "kepub.epub"),
        ("c.fb2", "fb2"),
        ("d.txt", "txt"),
    ] {
        let out = dir.join("nested").join(name);
        let r = convert_file(
            &no_readers(),
            &input,
            &out,
            &ConvertOptions::default(),
            &|_| {},
            &cancel,
        )
        .unwrap();
        assert_eq!(r.extension, ext, "{name}");
        assert_eq!(std::fs::read(&out).unwrap(), r.bytes);
    }
    let leftovers: Vec<_> = std::fs::read_dir(dir.join("nested"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        leftovers.iter().all(|n| !n.contains("bookx-tmp")),
        "{leftovers:?}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_failing_transform_names_itself() {
    let opts = ConvertOptions {
        metadata: MetadataOverride {
            cover: Some(b"not an image".to_vec()),
            ..Default::default()
        },
        ..Default::default()
    };
    let e = run(&epub_bytes(), "b.epub", "epub", &opts).err().unwrap();
    let msg = format!("{e:#}");
    assert!(
        msg.contains("transform `set_metadata_override` failed"),
        "{msg}"
    );
    assert!(msg.contains("not a JPEG, PNG or GIF"), "{msg}");
}

#[test]
fn metadata_override_reaches_the_output() {
    let opts = ConvertOptions {
        metadata: MetadataOverride {
            title: Some("Overridden".into()),
            authors: Some(vec!["Someone Else".into()]),
            ..Default::default()
        },
        ..Default::default()
    };
    let (out, _) = run(&epub_bytes(), "b.epub", "mobi", &opts).unwrap();
    let text = String::from_utf8_lossy(&out.bytes);
    assert!(text.contains("Overridden") && text.contains("Someone Else"));
}

#[test]
fn threads_do_not_change_the_result() {
    let mut chapters = Vec::new();
    for i in 0..12 {
        chapters.push(bookx_core::convert::Chapter {
            title: format!("C{i}"),
            body: format!("<h1>C{i}</h1><p>\"Quote {i}\" -- it's 'fine'...</p><p>&nbsp;</p><p>* * *</p><p>More text {i}.</p>"),
        });
    }
    let arc = bookx_core::convert::create_epub(&common::metadata(), &chapters, None);
    let input = arc.to_bytes().unwrap();
    let base = ConvertOptions {
        smarten_punctuation: true,
        remove_blank_paragraphs: true,
        replace_scene_breaks: Some("<hr/>".into()),
        unwrap_lines: Some(0.7),
        ..Default::default()
    };
    let mut seq = base.clone();
    seq.jobs = 1;
    let mut par = base;
    par.jobs = 8;
    let a = reopen(&run(&input, "b.epub", "epub", &seq).unwrap().0.bytes);
    let b = reopen(&run(&input, "b.epub", "epub", &par).unwrap().0.bytes);
    assert_eq!(a.spine_paths(), b.spine_paths());
    for p in a.spine_paths() {
        assert_eq!(a.archive.get(&p), b.archive.get(&p), "{p}");
    }
    let ch = a.archive.get_string(&a.spine_paths()[3]).unwrap();
    assert!(ch.contains("“Quote 3”") && ch.contains("<hr/>") && !ch.contains("&nbsp;"));
}

#[test]
fn builtin_presets_run_end_to_end() {
    let store = builtin_presets();
    for (name, ext) in [
        ("Kindle (MOBI)", "mobi"),
        ("Kindle (AZW3)", "azw3"),
        ("Kobo (KEPUB)", "kepub"),
        ("Generic e-ink (EPUB 2)", "epub"),
    ] {
        let preset = store.find(name, "epub").unwrap();
        let (out, log) = run(&epub_bytes(), "b.epub", ext, &preset.options)
            .unwrap_or_else(|e| panic!("{name}: {e:#}"));
        assert_eq!(log.last().unwrap().percent, 100);
        assert!(!out.reports.is_empty());
    }
    // the EPUB 2 preset really produces EPUB 2
    let preset = store.find("Generic e-ink (EPUB 2)", "epub").unwrap();
    let (out, _) = run(&epub_bytes(), "b.epub", "epub", &preset.options).unwrap();
    let book = reopen(&out.bytes);
    assert_eq!(book.package.version, "2.0");
    // and the e-ink profile stylesheet is part of it
    assert!(book
        .archive
        .names()
        .iter()
        .any(|n| n.ends_with("bookx-device.css")));
}

#[test]
fn epub_version_option_selects_the_packaging() {
    let mut o = ConvertOptions {
        epub_version: Some(bookx_convert::writers::EpubVersion::V2),
        ..Default::default()
    };
    let (out, _) = run(&epub_bytes(), "b.epub", "epub", &o).unwrap();
    assert_eq!(reopen(&out.bytes).package.version, "2.0");
    o.epub_version = Some(bookx_convert::writers::EpubVersion::V3);
    let (out, _) = run(&epub_bytes(), "b.epub", "epub", &o).unwrap();
    assert_eq!(reopen(&out.bytes).package.version, "3.0");
    // KEPUB honours it too
    let (out, _) = run(&epub_bytes(), "b.epub", "kepub", &o).unwrap();
    assert_eq!(reopen(&out.bytes).package.version, "3.0");
    let (out, _) = run(&epub_bytes(), "b.epub", "kepub", &ConvertOptions::default()).unwrap();
    assert_eq!(reopen(&out.bytes).package.version, "2.0");
}

#[test]
fn split_large_html_option_with_mobi_output() {
    let body: String = (0..400)
        .map(|i| format!("<p id=\"p{i}\">Paragraph {i} of a long chapter with filler text.</p>\n"))
        .collect();
    let chapters = vec![bookx_core::convert::Chapter {
        title: "Long".into(),
        body: format!("<h1>Long</h1>\n{body}"),
    }];
    let arc = bookx_core::convert::create_epub(&common::metadata(), &chapters, None);
    let opts = ConvertOptions {
        split_large_html: Some(5000),
        ..Default::default()
    };
    let (out, _) = run(&arc.to_bytes().unwrap(), "b.epub", "epub", &opts).unwrap();
    let book = reopen(&out.bytes);
    assert!(book.spine_paths().len() > 4);
    for p in book.spine_paths() {
        assert!(book.archive.get(&p).unwrap().len() <= 5000);
    }
}
