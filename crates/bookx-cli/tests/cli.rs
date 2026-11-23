//! End-to-end tests of every `bookx-cli` command through `bookx_cli::run`.
//! Each test builds its own fixtures in its own temp dir, so they run in parallel.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bookx_cli::{Env, run};
use bookx_core::book::OpenBook;
use bookx_core::convert::{Chapter, create_epub};
use bookx_core::opf::Metadata;

struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        let dir = std::env::temp_dir().join(format!(
            "bookx-cli-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Sandbox { dir }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
    fn lib(&self) -> PathBuf {
        self.dir.join("lib")
    }
    /// Runs the CLI with `--library <sandbox>/lib` and returns (code, stdout, stderr).
    fn cli(&self, args: &[&str]) -> (i32, String, String) {
        let mut v = vec!["bookx-cli".to_string()];
        v.extend(args.iter().map(|s| s.to_string()));
        let env = Env {
            default_library: self.lib(),
            stop: Arc::new(AtomicBool::new(false)),
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&v, &env, &mut out, &mut err);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }
    fn epub(&self, name: &str, title: &str, author: &str, body: &str) -> PathBuf {
        let md = Metadata {
            title: title.into(),
            title_sort: title.into(),
            authors: vec![author.into()],
            language: "en".into(),
            ..Default::default()
        };
        let chapters = vec![
            Chapter {
                title: "One".into(),
                body: format!("<h1>One</h1><p>{body}</p>"),
            },
            Chapter {
                title: "Two".into(),
                body: "<h1>Two</h1><p>Second chapter text.</p>".into(),
            },
        ];
        let p = self.path(name);
        std::fs::write(&p, create_epub(&md, &chapters, None).to_bytes().unwrap()).unwrap();
        p
    }
    fn fb2(&self, name: &str) -> PathBuf {
        let p = self.path(name);
        std::fs::write(
            &p,
            r#"<?xml version="1.0" encoding="utf-8"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0">
<description><title-info><genre>sf</genre>
<author><first-name>Stan</first-name><last-name>Lem</last-name></author>
<book-title>Solaris</book-title><lang>pl</lang></title-info></description>
<body><section><title><p>Arrival</p></title><p>The ocean was alive.</p></section></body>
</FictionBook>"#,
        )
        .unwrap();
        p
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

// 1x1 transparent PNG
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xFF, 0xFF, 0x3F,
    0x00, 0x05, 0xFE, 0x02, 0xFE, 0xDC, 0xCC, 0x59, 0xE7, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
    0x44, 0xAE, 0x42, 0x60, 0x82,
];

// -- usage ----------------------------------------------------------------------------------

#[test]
fn no_arguments_is_a_usage_error() {
    let sb = Sandbox::new("usage");
    let (code, _, err) = sb.cli(&[]);
    assert_eq!(code, 2);
    assert!(err.contains("Usage"), "{err}");
}

#[test]
fn help_and_version_succeed() {
    let sb = Sandbox::new("help");
    let (code, out, _) = sb.cli(&["--help"]);
    assert_eq!(code, 0);
    for c in ["convert", "meta", "check", "library", "search", "serve"] {
        assert!(out.contains(c), "help should mention {c}");
    }
    let (code, out, _) = sb.cli(&["--version"]);
    assert_eq!(code, 0);
    assert!(out.contains("bookx-cli"));
}

#[test]
fn unknown_command_is_a_usage_error() {
    let sb = Sandbox::new("unknown");
    let (code, _, err) = sb.cli(&["frobnicate"]);
    assert_eq!(code, 2);
    assert!(!err.is_empty());
}

// -- formats / presets -------------------------------------------------------------------------

#[test]
fn formats_lists_inputs_and_outputs() {
    let sb = Sandbox::new("formats");
    let (code, out, _) = sb.cli(&["formats"]);
    assert_eq!(code, 0);
    for ext in ["epub", "fb2", "mobi", "docx", "cbz"] {
        assert!(out.contains(ext), "missing input {ext}: {out}");
    }
    assert!(out.contains("kepub.epub"));
    assert!(out.lines().any(|l| l.starts_with("Output:")));
}

#[test]
fn formats_json_is_valid() {
    let sb = Sandbox::new("formats-json");
    let (code, out, _) = sb.cli(&["formats", "--json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v["input"].as_array().unwrap().iter().any(|e| e == "epub"));
    assert!(
        v["output"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["id"] == "mobi")
    );
}

#[test]
fn presets_lists_builtins_in_both_forms() {
    let sb = Sandbox::new("presets");
    let (code, out, _) = sb.cli(&["presets"]);
    assert_eq!(code, 0);
    assert!(out.contains("Kindle (MOBI)"));
    let (code, out, _) = sb.cli(&["presets", "--json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(!v["presets"].as_array().unwrap().is_empty());
}

// -- convert -------------------------------------------------------------------------------------

#[test]
fn convert_epub_to_txt_and_markdown() {
    let sb = Sandbox::new("conv-txt");
    let input = sb.epub("b.epub", "Tale", "Ann Author", "Hello readers.");
    let out_txt = sb.path("b.txt");
    let (code, out, _) = sb.cli(&["convert", s(&input), s(&out_txt), "--quiet"]);
    assert_eq!(code, 0);
    assert!(out.contains("->"));
    let text = std::fs::read_to_string(&out_txt).unwrap();
    assert!(text.contains("Hello readers.") && text.contains("Second chapter text."));
    let out_md = sb.path("b.md");
    assert_eq!(sb.cli(&["convert", s(&input), s(&out_md), "-q"]).0, 0);
    assert!(std::fs::read_to_string(&out_md).unwrap().contains('#'));
}

#[test]
fn convert_report_prints_or_saves_what_was_lost() {
    let sb = Sandbox::new("conv-report");
    let input = sb.epub("b.epub", "Tale", "Ann Author", "Hello readers.");
    let (code, out, _) = sb.cli(&[
        "convert",
        s(&input),
        s(&sb.path("b.epub2.epub")),
        "-q",
        "--report",
    ]);
    assert_eq!(code, 0);
    assert!(
        out.contains("Conversion report") && out.contains("Nothing was lost"),
        "{out}"
    );
    let rep = sb.path("r.json");
    let (code, out, _) = sb.cli(&[
        "convert",
        s(&input),
        s(&sb.path("b.txt")),
        "-q",
        "--report",
        s(&rep),
    ]);
    assert_eq!(code, 0);
    assert!(out.contains("report saved"));
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&rep).unwrap()).unwrap();
    assert_eq!(v["output"], "txt");
    assert!(
        v["lost"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().unwrap().contains("table of contents"))
    );
}

#[test]
fn inspect_lists_the_contents_tree() {
    let sb = Sandbox::new("inspect");
    let input = sb.epub("b.epub", "Tale", "Ann Author", "Hello readers.");
    let (code, out, _) = sb.cli(&["inspect", s(&input)]);
    assert_eq!(code, 0);
    assert!(out.contains("Words:") && out.contains("Contents:"), "{out}");
    let (code, out, _) = sb.cli(&["inspect", s(&input), "--json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v["words"].as_u64().unwrap() > 3);
}

#[test]
fn convert_fb2_to_epub_keeps_metadata() {
    let sb = Sandbox::new("conv-fb2");
    let input = sb.fb2("s.fb2");
    let output = sb.path("s.epub");
    let (code, _, _) = sb.cli(&["convert", s(&input), s(&output), "-q"]);
    assert_eq!(code, 0);
    let book = OpenBook::open(&output).unwrap();
    assert_eq!(book.metadata().title, "Solaris");
    assert!(book.metadata().authors[0].contains("Lem"));
}

#[test]
fn convert_pdf_to_epub_is_flagged_experimental_and_keeps_the_text() {
    let sb = Sandbox::new("conv-pdf");
    let input = sb.path("Ada Reader - Foxes.pdf");
    std::fs::write(
        &input,
        bookx_pdf::testpdf::TestPdf::simple(&[&["The quick brown fox jumps over the lazy dog."]])
            .build(),
    )
    .unwrap();
    let output = sb.path("foxes.epub");
    let (code, _, err) = sb.cli(&["convert", s(&input), s(&output), "-q"]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("experimental"), "{err}");
    let book = OpenBook::open(&output).unwrap();
    let text = book.archive.get_string("OEBPS/chapter-1.xhtml").unwrap();
    assert!(text.contains("quick brown fox"));
}

#[test]
fn adding_a_pdf_keeps_the_pdf_as_a_format() {
    let sb = Sandbox::new("add-pdf");
    let input = sb.path("Ada Reader - Foxes.pdf");
    std::fs::write(
        &input,
        bookx_pdf::testpdf::TestPdf::simple(&[&["The quick brown fox jumps over the lazy dog."]])
            .build(),
    )
    .unwrap();
    let (code, _, err) = sb.cli(&["library", "add", s(&input)]);
    assert_eq!(code, 0, "{err}");
    let (_, dump, _) = sb.cli(&["library", "dump"]);
    assert!(dump.contains("Foxes") && dump.contains(".pdf"), "{dump}");
}

#[test]
fn convert_to_kepub_adds_kobo_spans() {
    let sb = Sandbox::new("conv-kepub");
    let input = sb.epub("b.epub", "Tale", "Ann", "Plain words here.");
    let output = sb.path("b.kepub.epub");
    assert_eq!(sb.cli(&["convert", s(&input), s(&output), "-q"]).0, 0);
    let mut z = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
    let mut found = false;
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let fname = f.name().map(|n| n.to_string()).unwrap_or_default();
        if fname.ends_with(".xhtml") || fname.ends_with(".html") {
            let mut t = String::new();
            if f.read_to_string(&mut t).is_ok() && t.contains("koboSpan") {
                found = true;
            }
        }
    }
    assert!(found, "no koboSpan in any chapter");
}

#[test]
fn convert_to_mobi_writes_a_palmdb() {
    let sb = Sandbox::new("conv-mobi");
    let input = sb.epub("b.epub", "Tale", "Ann", "Plain words here.");
    let output = sb.path("b.mobi");
    assert_eq!(sb.cli(&["convert", s(&input), s(&output), "-q"]).0, 0);
    let b = std::fs::read(&output).unwrap();
    assert_eq!(&b[60..68], b"BOOKMOBI");
}

#[test]
fn convert_applies_metadata_overrides() {
    let sb = Sandbox::new("conv-meta");
    let input = sb.epub("b.epub", "Old", "Ann", "Text.");
    let output = sb.path("n.epub");
    let (code, _, _) = sb.cli(&[
        "convert",
        s(&input),
        s(&output),
        "-q",
        "--title",
        "New Title",
        "--author",
        "Bob B",
        "--author",
        "Cy C",
        "--series",
        "Saga",
        "--series-index",
        "2",
        "--tag",
        "a",
        "--tag",
        "b",
        "--language",
        "de",
        "--publisher",
        "Pub",
    ]);
    assert_eq!(code, 0);
    let m = OpenBook::open(&output).unwrap().metadata().clone();
    assert_eq!(m.title, "New Title");
    assert_eq!(m.authors, vec!["Bob B", "Cy C"]);
    assert_eq!(m.series, "Saga");
    assert_eq!(m.series_index, 2.0);
    assert_eq!(m.tags, vec!["a", "b"]);
    assert_eq!(m.language, "de");
    assert_eq!(m.publisher, "Pub");
}

#[test]
fn convert_smarten_makes_quotes_curly() {
    let sb = Sandbox::new("conv-smart");
    let input = sb.epub("b.epub", "T", "A", "\"Hello,\" she said -- it's late...");
    let output = sb.path("o.epub");
    assert_eq!(
        sb.cli(&["convert", s(&input), s(&output), "-q", "--smarten"])
            .0,
        0
    );
    let book = OpenBook::open(&output).unwrap();
    let src = book.chapter_source(&book.spine_paths()[0]).unwrap();
    assert!(src.contains('\u{201C}') && src.contains('\u{2019}') && src.contains('\u{2026}'));
    assert!(!src.contains("&quot;Hello"));
}

#[test]
fn convert_with_preset_device_and_epub_version() {
    let sb = Sandbox::new("conv-preset");
    let input = sb.epub("b.epub", "T", "A", "Words.");
    let out1 = sb.path("p.mobi");
    assert_eq!(
        sb.cli(&[
            "convert",
            s(&input),
            s(&out1),
            "-q",
            "--preset",
            "Kindle (MOBI)"
        ])
        .0,
        0
    );
    let out2 = sb.path("d.epub");
    assert_eq!(
        sb.cli(&[
            "convert",
            s(&input),
            s(&out2),
            "-q",
            "--device",
            "kobo",
            "--epub-version",
            "2",
            "--auto-toc",
            "force",
            "--remove-unused-css",
            "--remove-blank-paragraphs",
            "--remove-fake-margins",
            "--unwrap-lines",
            "--split-large-html",
            "100000",
            "--jobs",
            "2",
        ])
        .0,
        0
    );
    assert!(OpenBook::open(&out2).is_ok());
}

#[test]
fn convert_with_a_preset_file() {
    let sb = Sandbox::new("conv-presetfile");
    let input = sb.epub("b.epub", "T", "A", "Words.");
    let mut store = bookx_convert::PresetStore::default();
    store.put(bookx_convert::Preset {
        name: "mine".into(),
        input_format: None,
        output_format: None,
        options: bookx_convert::ConvertOptions {
            smarten_punctuation: true,
            ..Default::default()
        },
    });
    let pf = sb.path("preset.json");
    store.save(&pf).unwrap();
    let output = sb.path("o.epub");
    let (code, _, err) = sb.cli(&["convert", s(&input), s(&output), "-q", "--preset", s(&pf)]);
    assert_eq!(code, 0, "{err}");
}

#[test]
fn convert_sets_a_cover() {
    let sb = Sandbox::new("conv-cover");
    let input = sb.epub("b.epub", "T", "A", "Words.");
    let img = sb.path("c.png");
    std::fs::write(&img, PNG).unwrap();
    let output = sb.path("o.epub");
    let (code, _, err) = sb.cli(&["convert", s(&input), s(&output), "-q", "--cover", s(&img)]);
    assert_eq!(code, 0, "{err}");
    assert!(OpenBook::open(&output).unwrap().cover_bytes().is_some());
}

#[test]
fn convert_prints_monotonic_progress_unless_quiet() {
    let sb = Sandbox::new("conv-progress");
    let input = sb.epub("b.epub", "T", "A", "Words.");
    let output = sb.path("o.txt");
    let (code, _, err) = sb.cli(&["convert", s(&input), s(&output)]);
    assert_eq!(code, 0);
    let pcts: Vec<u32> = err
        .lines()
        .filter_map(|l| l.split('%').next()?.trim().parse().ok())
        .collect();
    assert!(!pcts.is_empty(), "{err}");
    assert!(pcts.windows(2).all(|w| w[0] <= w[1]), "{pcts:?}");
    assert_eq!(*pcts.last().unwrap(), 100);
    let (_, _, err) = sb.cli(&["convert", s(&input), s(&sb.path("q.txt")), "-q"]);
    assert!(err.is_empty());
}

#[test]
fn convert_errors_are_reported_without_leaving_output() {
    let sb = Sandbox::new("conv-err");
    let input = sb.epub("b.epub", "T", "A", "Words.");
    let cases: Vec<(Vec<String>, &str)> = vec![
        (
            vec![
                "convert".into(),
                "nope.epub".into(),
                s(&sb.path("x.txt")).into(),
            ],
            "does not exist",
        ),
        (
            vec!["convert".into(), s(&input).into(), s(&input).into()],
            "same file",
        ),
        (
            vec![
                "convert".into(),
                s(&input).into(),
                s(&sb.path("x.xyz")).into(),
            ],
            "unsupported output format",
        ),
        (
            vec![
                "convert".into(),
                s(&input).into(),
                s(&sb.path("x.txt")).into(),
                "--device".into(),
                "toaster".into(),
            ],
            "unknown device",
        ),
        (
            vec![
                "convert".into(),
                s(&input).into(),
                s(&sb.path("x.epub")).into(),
                "--epub-version".into(),
                "5".into(),
            ],
            "must be 2 or 3",
        ),
        (
            vec![
                "convert".into(),
                s(&input).into(),
                s(&sb.path("x.epub")).into(),
                "--smarten".into(),
                "--unsmarten".into(),
            ],
            "cannot be combined",
        ),
        (
            vec![
                "convert".into(),
                s(&input).into(),
                s(&sb.path("x.epub")).into(),
                "--unwrap-lines".into(),
                "7".into(),
            ],
            "between 0 and 1",
        ),
        (
            vec![
                "convert".into(),
                s(&input).into(),
                s(&sb.path("x.epub")).into(),
                "--auto-toc".into(),
                "sometimes".into(),
            ],
            "if-missing",
        ),
        (
            vec![
                "convert".into(),
                s(&input).into(),
                s(&sb.path("x.epub")).into(),
                "--preset".into(),
                "no such preset".into(),
            ],
            "unknown preset",
        ),
    ];
    for (args, needle) in cases {
        let refs: Vec<&str> = args.iter().map(|a| a.as_str()).collect();
        let (code, _, err) = sb.cli(&refs);
        assert_eq!(code, 2, "{args:?}: {err}");
        assert!(
            err.contains(needle),
            "{args:?}: expected `{needle}` in `{err}`"
        );
    }
    for f in ["x.txt", "x.epub", "x.xyz"] {
        assert!(!sb.path(f).exists(), "{f} must not be created on failure");
    }
}

#[test]
fn convert_rejects_non_image_cover_and_garbage_input() {
    let sb = Sandbox::new("conv-bad");
    let input = sb.epub("b.epub", "T", "A", "Words.");
    let notimg = sb.path("c.png");
    std::fs::write(&notimg, b"not an image").unwrap();
    let (code, _, err) = sb.cli(&[
        "convert",
        s(&input),
        s(&sb.path("o.epub")),
        "-q",
        "--cover",
        s(&notimg),
    ]);
    assert_eq!(code, 2);
    assert!(err.contains("not a JPEG, PNG or GIF"), "{err}");
    let garbage = sb.path("g.epub");
    std::fs::write(&garbage, b"garbage").unwrap();
    let (code, _, _) = sb.cli(&["convert", s(&garbage), s(&sb.path("o.txt")), "-q"]);
    assert_eq!(code, 2);
    assert!(!sb.path("o.txt").exists());
}

// -- meta ------------------------------------------------------------------------------------------

#[test]
fn meta_show_text_and_json() {
    let sb = Sandbox::new("meta-show");
    let input = sb.epub("b.epub", "The Tale", "Ann Author", "Words.");
    let (code, out, _) = sb.cli(&["meta", "show", s(&input)]);
    assert_eq!(code, 0);
    assert!(out.contains("Title:       The Tale"));
    assert!(out.contains("Author(s):   Ann Author"));
    assert!(out.contains("Cover:       no"));
    let (code, out, _) = sb.cli(&["meta", "show", s(&input), "--json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["title"], "The Tale");
}

#[test]
fn meta_show_reads_other_formats() {
    let sb = Sandbox::new("meta-fb2");
    let (code, out, _) = sb.cli(&["meta", "show", s(&sb.fb2("s.fb2"))]);
    assert_eq!(code, 0);
    assert!(out.contains("Solaris"));
}

#[test]
fn meta_set_changes_every_field_and_persists() {
    let sb = Sandbox::new("meta-set");
    let input = sb.epub("b.epub", "Old", "Ann Author", "Words.");
    let img = sb.path("c.png");
    std::fs::write(&img, PNG).unwrap();
    let (code, out, err) = sb.cli(&[
        "meta",
        "set",
        s(&input),
        "--title",
        "The New One",
        "--author",
        "Zed Zeta",
        "--series",
        "Saga",
        "--series-index",
        "3.5",
        "--tag",
        "x",
        "--tag",
        "y",
        "--publisher",
        "P&Q",
        "--language",
        "fr",
        "--description",
        "A <b>desc</b>",
        "--rating",
        "4",
        "--identifier",
        "isbn:9781234567890",
        "--identifier",
        "doi:10.1/x",
        "--cover",
        s(&img),
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("updated"));
    let book = OpenBook::open(&input).unwrap();
    let m = book.metadata();
    assert_eq!(m.title, "The New One");
    assert_eq!(m.title_sort, "New One, The");
    assert_eq!(m.authors, vec!["Zed Zeta"]);
    assert_eq!(m.author_sort, "Zeta, Zed");
    assert_eq!(m.series, "Saga");
    assert_eq!(m.series_index, 3.5);
    assert_eq!(m.tags, vec!["x", "y"]);
    assert_eq!(m.publisher, "P&Q");
    assert_eq!(m.language, "fr");
    assert_eq!(m.description, "A <b>desc</b>");
    assert_eq!(m.rating, 8);
    assert_eq!(m.identifier("isbn"), Some("9781234567890"));
    assert_eq!(m.identifier("doi"), Some("10.1/x"));
    assert!(book.cover_bytes().is_some());
    // the chapters were not touched
    assert_eq!(book.spine_paths().len(), 2);
}

#[test]
fn meta_set_handles_non_ascii_titles() {
    // regression: title_sort used to panic on multi-byte characters
    let sb = Sandbox::new("meta-unicode");
    let input = sb.epub("b.epub", "Old", "Ann", "Words.");
    for t in ["Война и мир", "日本語の本", "The Ünïcode", "An Æther"] {
        let (code, _, err) = sb.cli(&["meta", "set", s(&input), "--title", t]);
        assert_eq!(code, 0, "{t}: {err}");
        assert!(OpenBook::open(&input).unwrap().metadata().title == t);
    }
}

#[test]
fn meta_set_validates_input() {
    let sb = Sandbox::new("meta-bad");
    let input = sb.epub("b.epub", "Old", "Ann", "Words.");
    let fb2 = sb.fb2("s.fb2");
    let missing = sb.path("missing.epub");
    let cases: Vec<(Vec<&str>, &str)> = vec![
        (
            vec!["meta", "set", s(&fb2), "--title", "x"],
            "only be written to .epub",
        ),
        (vec!["meta", "set", s(&input), "--rating", "6"], "--rating"),
        (
            vec!["meta", "set", s(&input), "--identifier", "nocolon"],
            "scheme:value",
        ),
        (vec!["meta", "set", s(&missing), "--title", "x"], ""),
    ];
    for (args, needle) in cases {
        let (code, _, err) = sb.cli(&args);
        assert_eq!(code, 2, "{args:?}");
        assert!(err.contains(needle), "{args:?}: {err}");
    }
    // nothing was modified by the failed attempts
    assert_eq!(OpenBook::open(&input).unwrap().metadata().title, "Old");
}

// -- check -----------------------------------------------------------------------------------------

fn break_epub(src: &Path, dst: &Path) {
    // drop one chapter file but keep the manifest entry for it
    let mut zin = zip::ZipArchive::new(std::fs::File::open(src).unwrap()).unwrap();
    let mut zout = zip::ZipWriter::new(std::fs::File::create(dst).unwrap());
    let mut dropped = false;
    for i in 0..zin.len() {
        let mut f = zin.by_index(i).unwrap();
        let name = f.name().map(|n| n.to_string()).unwrap_or_default();
        if !dropped && name.ends_with("chapter-1.xhtml") {
            dropped = true;
            continue;
        }
        let opts =
            zip::write::SimpleFileOptions::default().compression_method(if name == "mimetype" {
                zip::CompressionMethod::Stored
            } else {
                zip::CompressionMethod::Deflated
            });
        zout.start_file(&name, opts).unwrap();
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).unwrap();
        zout.write_all(&buf).unwrap();
    }
    zout.finish().unwrap();
    assert!(dropped, "fixture: no chapter file matched; names changed?");
}

fn chapter_names(p: &Path) -> Vec<String> {
    let z = zip::ZipArchive::new(std::fs::File::open(p).unwrap()).unwrap();
    z.file_names()
        .filter_map(|n| n.ok().map(|c| c.to_string()))
        .collect()
}

#[test]
fn check_passes_a_clean_book() {
    let sb = Sandbox::new("check-ok");
    let input = sb.epub("b.epub", "T", "A", "Words.");
    let (code, out, _) = sb.cli(&["check", s(&input)]);
    assert_eq!(code, 0, "{out}");
    let (code, out, _) = sb.cli(&["check", s(&input), "--json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["errors"], 0);
}

#[test]
fn check_reports_a_missing_file_with_exit_code_1() {
    let sb = Sandbox::new("check-bad");
    let good = sb.epub("b.epub", "T", "A", "Words.");
    let names = chapter_names(&good);
    assert!(
        names.iter().any(|n| n.ends_with("chapter-1.xhtml")),
        "{names:?}"
    );
    let bad = sb.path("bad.epub");
    break_epub(&good, &bad);
    let (code, out, _) = sb.cli(&["check", s(&bad)]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("error:"), "{out}");
    let (code, out, _) = sb.cli(&["check", s(&bad), "--json"]);
    assert_eq!(code, 1);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v["errors"].as_u64().unwrap() >= 1);
    assert_eq!(v["issues"][0]["severity"], "error");
}

#[test]
fn check_fails_cleanly_on_garbage_and_missing_files() {
    let sb = Sandbox::new("check-garbage");
    let g = sb.path("g.epub");
    std::fs::write(&g, b"nope").unwrap();
    assert_eq!(sb.cli(&["check", s(&g)]).0, 2);
    assert_eq!(sb.cli(&["check", s(&sb.path("none.epub"))]).0, 2);
}

// -- library ---------------------------------------------------------------------------------------

#[test]
fn library_add_list_info_export_remove() {
    let sb = Sandbox::new("lib-basic");
    let a = sb.epub("a.epub", "Alpha", "Ann Author", "First.");
    let b = sb.epub("b.epub", "Beta", "Bob Writer", "Second.");
    let (code, out, _) = sb.cli(&["library", "add", s(&a), s(&b)]);
    assert_eq!(code, 0);
    assert!(out.contains("2 added, 0 duplicate(s), 0 failed"), "{out}");

    let (code, out, _) = sb.cli(&["library", "list"]);
    assert_eq!(code, 0);
    assert!(out.contains("Alpha") && out.contains("Beta"));
    let (_, json, _) = sb.cli(&["library", "list", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);
    let id = v[0]["id"].as_u64().unwrap().to_string();

    let (code, out, _) = sb.cli(&["library", "info", &id]);
    assert_eq!(code, 0);
    assert!(out.contains("Title:") && out.contains("Status:      unread"));
    let (code, out, _) = sb.cli(&["library", "info", &id, "--json"]);
    assert_eq!(code, 0);
    assert!(serde_json::from_str::<serde_json::Value>(&out).is_ok());

    let dest = sb.path("out/copy.epub");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    assert_eq!(sb.cli(&["library", "export", &id, s(&dest)]).0, 0);
    assert!(OpenBook::open(&dest).is_ok());

    let (code, out, _) = sb.cli(&["library", "remove", &id]);
    assert_eq!(code, 0);
    assert!(out.contains("removed 1"));
    let (_, json, _) = sb.cli(&["library", "list", "--json"]);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&json)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn library_add_detects_duplicates_unless_allowed() {
    let sb = Sandbox::new("lib-dup");
    let a = sb.epub("a.epub", "Alpha", "Ann", "First.");
    assert_eq!(sb.cli(&["library", "add", s(&a)]).0, 0);
    let (code, out, _) = sb.cli(&["library", "add", s(&a)]);
    assert_eq!(code, 0);
    assert!(out.contains("duplicate of #"), "{out}");
    let (code, out, _) = sb.cli(&["library", "add", s(&a), "--allow-duplicates"]);
    assert_eq!(code, 0);
    assert!(out.contains("1 added"), "{out}");
}

#[test]
fn library_add_folders_recursively_and_converts_other_formats() {
    let sb = Sandbox::new("lib-folder");
    let books = sb.path("books");
    std::fs::create_dir_all(books.join("sub/deeper")).unwrap();
    std::fs::copy(
        sb.epub("x.epub", "Top", "A", "t.").as_path(),
        books.join("top.epub"),
    )
    .unwrap();
    std::fs::copy(
        sb.epub("y.epub", "Deep", "B", "d.").as_path(),
        books.join("sub/deeper/deep.epub"),
    )
    .unwrap();
    std::fs::copy(sb.fb2("s.fb2").as_path(), books.join("sub/solaris.fb2")).unwrap();
    // not recursive: only the top level
    let (_, out, _) = sb.cli(&["library", "add", s(&books)]);
    assert!(out.contains("1 added"), "{out}");
    let (code, out, _) = sb.cli(&["library", "add", s(&books), "-r"]);
    assert_eq!(code, 0);
    assert!(out.contains("2 added, 1 duplicate(s)"), "{out}");
    let (_, out, _) = sb.cli(&["library", "list"]);
    assert!(out.contains("Solaris") && out.contains("Deep") && out.contains("Top"));
}

#[test]
fn library_add_reports_failures_but_keeps_going() {
    let sb = Sandbox::new("lib-fail");
    let good = sb.epub("g.epub", "Good", "A", "ok.");
    let bad = sb.path("bad.epub");
    std::fs::write(&bad, b"garbage").unwrap();
    let (code, out, err) = sb.cli(&["library", "add", s(&bad), s(&good)]);
    assert_eq!(code, 1);
    assert!(err.contains("failed:"), "{err}");
    assert!(out.contains("1 added, 0 duplicate(s), 1 failed"), "{out}");
    assert!(sb.cli(&["library", "list"]).1.contains("Good"));
}

#[test]
fn library_errors() {
    let sb = Sandbox::new("lib-err");
    // nothing there yet
    let (code, _, err) = sb.cli(&["library", "list"]);
    assert_eq!(code, 2);
    assert!(err.contains("no library at"), "{err}");
    assert_eq!(sb.cli(&["library", "add"]).0, 2);
    assert_eq!(sb.cli(&["library", "add", "missing.epub"]).0, 2);
    let a = sb.epub("a.epub", "Alpha", "Ann", "First.");
    assert_eq!(sb.cli(&["library", "add", s(&a)]).0, 0);
    assert_eq!(sb.cli(&["library", "info", "999"]).0, 2);
    assert_eq!(sb.cli(&["library", "remove", "999"]).0, 2);
    assert_eq!(sb.cli(&["library", "remove"]).0, 2);
    assert_eq!(
        sb.cli(&["library", "export", "999", s(&sb.path("o.epub"))])
            .0,
        2
    );
}

#[test]
fn library_remove_can_delete_files() {
    let sb = Sandbox::new("lib-delete");
    let a = sb.epub("a.epub", "Alpha", "Ann", "First.");
    sb.cli(&["library", "add", s(&a)]);
    let (_, json, _) = sb.cli(&["library", "list", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let id = v[0]["id"].as_u64().unwrap().to_string();
    let file = sb.lib().join(v[0]["file"].as_str().unwrap());
    assert!(file.exists());
    assert_eq!(sb.cli(&["library", "remove", &id, "--delete-files"]).0, 0);
    assert!(!file.exists());
}

#[test]
fn default_library_is_used_when_flag_is_missing() {
    let sb = Sandbox::new("lib-default");
    let a = sb.epub("a.epub", "Alpha", "Ann", "First.");
    sb.cli(&["library", "add", s(&a)]);
    assert!(sb.lib().join("bookx-library.db").exists());
    assert!(
        !sb.lib().join("bookx-library.json").exists(),
        "no JSON catalogue any more"
    );
    // explicit flag wins
    let other = sb.path("other");
    let (code, _, _) = sb.cli(&["--library", s(&other), "library", "add", s(&a)]);
    assert_eq!(code, 0);
    assert!(other.join("bookx-library.db").exists());
}

// -- search ----------------------------------------------------------------------------------------

#[test]
fn search_filters_sorts_and_signals_no_hits() {
    let sb = Sandbox::new("search");
    for (f, t, a) in [
        ("1.epub", "Zebra", "Ann Author"),
        ("2.epub", "Apple", "Bob Writer"),
        ("3.epub", "Mango", "Ann Author"),
    ] {
        sb.cli(&["library", "add", s(&sb.epub(f, t, a, "text."))]);
    }
    let (code, out, _) = sb.cli(&["search", "author:ann"]);
    assert_eq!(code, 0);
    assert!(out.contains("Zebra") && out.contains("Mango") && !out.contains("Apple"));
    let (_, out, _) = sb.cli(&["search", "", "--sort", "title"]);
    let titles: Vec<&str> = ["Apple", "Mango", "Zebra"].to_vec();
    let pos: Vec<usize> = titles.iter().map(|t| out.find(t).unwrap()).collect();
    assert!(pos.windows(2).all(|w| w[0] < w[1]), "{out}");
    let (_, out, _) = sb.cli(&["search", "", "--sort", "title", "--desc"]);
    assert!(out.find("Zebra").unwrap() < out.find("Apple").unwrap());
    let (code, out, _) = sb.cli(&["search", "title:nothing-like-this"]);
    assert_eq!(code, 1);
    assert!(out.trim().is_empty());
    let (_, out, _) = sb.cli(&["search", "title:=apple", "--json"]);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&out)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let (code, _, err) = sb.cli(&["search", "x", "--sort", "weight"]);
    assert_eq!(code, 2);
    assert!(err.contains("unknown sort key"), "{err}");
}

// -- serve -----------------------------------------------------------------------------------------

#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Vec<u8>>>);
impl Write for Shared {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn http_get(addr: &str, path: &str, auth: Option<&str>) -> (u16, String) {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let a = auth
        .map(|a| format!("Authorization: Basic {a}\r\n"))
        .unwrap_or_default();
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: x\r\n{a}Connection: close\r\n\r\n"
    )
    .unwrap();
    let mut buf = String::new();
    let _ = s.read_to_string(&mut buf);
    let code = buf
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    (code, buf)
}

/// Starts `serve` in a thread; returns (base address, stop flag, join handle).
fn start_server(
    sb: &Sandbox,
    extra: &[&str],
) -> (String, Arc<AtomicBool>, std::thread::JoinHandle<i32>) {
    let stop = Arc::new(AtomicBool::new(false));
    let env = Env {
        default_library: sb.lib(),
        stop: stop.clone(),
    };
    let mut args: Vec<String> = ["bookx-cli", "serve", "--port", "0"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    args.extend(extra.iter().map(|s| s.to_string()));
    // port 0 is not accepted by the CLI default, but `--port 0` picks a free port
    let out = Shared::default();
    let out2 = out.clone();
    let h = std::thread::spawn(move || {
        let mut o = out2;
        let mut e = Vec::new();
        run(&args, &env, &mut o, &mut e)
    });
    let t = Instant::now();
    loop {
        let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
        if let Some(rest) = text
            .strip_prefix("serving on http://")
            .or_else(|| text.strip_prefix("serving on https://"))
        {
            return (rest.trim().to_string(), stop, h);
        }
        assert!(
            t.elapsed() < Duration::from_secs(10),
            "server did not start: {text}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn serve_exposes_opds_and_downloads_then_stops() {
    let sb = Sandbox::new("serve");
    sb.cli(&[
        "library",
        "add",
        s(&sb.epub("a.epub", "Served Book", "Ann", "text.")),
    ]);
    let (addr, stop, h) = start_server(&sb, &[]);
    let (code, body) = http_get(&addr, "/opds", None);
    assert_eq!(code, 200, "{body}");
    let (code, body) = http_get(&addr, "/", None);
    assert_eq!(code, 200);
    assert!(body.contains("Served Book"), "{body}");
    let (code, _) = http_get(&addr, "/nonexistent", None);
    assert_eq!(code, 404);
    stop.store(true, Ordering::SeqCst);
    assert_eq!(h.join().unwrap(), 0);
}

#[test]
fn serve_requires_credentials_when_users_are_given() {
    let sb = Sandbox::new("serve-auth");
    sb.cli(&[
        "library",
        "add",
        s(&sb.epub("a.epub", "Secret", "Ann", "text.")),
    ]);
    let (addr, stop, h) = start_server(&sb, &["--user", "ann:pw"]);
    assert_eq!(http_get(&addr, "/opds", None).0, 401);
    // base64("ann:pw") = YW5uOnB3
    assert_eq!(http_get(&addr, "/opds", Some("YW5uOnB3")).0, 200);
    // base64("ann:wrong") = YW5uOndyb25n
    assert_eq!(http_get(&addr, "/opds", Some("YW5uOndyb25n")).0, 401);
    stop.store(true, Ordering::SeqCst);
    assert_eq!(h.join().unwrap(), 0);
}

#[test]
fn serve_accounts_guest_prefix_and_web_client() {
    let sb = Sandbox::new("serve-accounts");
    sb.cli(&[
        "library",
        "add",
        s(&sb.epub("a.epub", "Hidden Title", "Ann", "text.")),
    ]);
    let (addr, stop, h) = start_server(
        &sb,
        &[
            "--account",
            "kid:pw:nothing",
            "--account",
            "mum:pw2",
            "--guest-collection",
            "nothing",
            "--public-prefix",
            "books/",
            "--trusted-proxy",
            "127.0.0.1",
        ],
    );
    // the child's collection holds no book; the account without collections sees all
    let (code, body) = http_get(&addr, "/books/opds/all", Some("a2lkOnB3"));
    assert_eq!(code, 200, "{body}");
    assert!(!body.contains("Hidden Title"), "{body}");
    let (code, body) = http_get(&addr, "/opds/all", Some("bXVtOnB3Mg=="));
    assert_eq!(code, 200);
    assert!(
        body.contains("Hidden Title") && body.contains("/books/download/"),
        "{body}"
    );
    // the guest sees an empty catalogue, and the web client files are served
    let (code, body) = http_get(&addr, "/opds/all", None);
    assert_eq!(code, 200);
    assert!(!body.contains("Hidden Title"));
    let (code, body) = http_get(&addr, "/books/app/manifest.webmanifest", None);
    assert_eq!(code, 200, "{body}");
    assert!(body.contains("Bookx"));
    stop.store(true, Ordering::SeqCst);
    assert_eq!(h.join().unwrap(), 0);
}

#[test]
fn serve_refuses_bad_proxy_prefix_and_account_arguments() {
    let sb = Sandbox::new("serve-bad-net");
    sb.cli(&["library", "add", s(&sb.epub("a.epub", "B", "Ann", "text."))]);
    for (flag, val, word) in [
        ("--trusted-proxy", "0.0.0.0/0", "trusted-proxy"),
        ("--trusted-proxy", "proxy.example.com", "trusted-proxy"),
        ("--public-prefix", "/a/../b", "public-prefix"),
        ("--public-prefix", "/bookx", "public-prefix"),
        ("--account", "nopassword", "account"),
    ] {
        let (code, _, err) = sb.cli(&["serve", "--port", "0", flag, val]);
        assert_eq!(code, 2, "{flag} {val}: {err}");
        assert!(err.contains(word), "{err}");
    }
}

#[test]
fn serve_kosync_endpoint_is_optional() {
    let sb = Sandbox::new("serve-kosync");
    sb.cli(&["library", "add", s(&sb.epub("a.epub", "B", "Ann", "text."))]);
    let kf = sb.path("kosync.json");
    let (addr, stop, h) = start_server(&sb, &["--kosync", s(&kf)]);
    assert_eq!(http_get(&addr, "/healthcheck", None).0, 200);
    assert_eq!(http_get(&addr, "/bookx/healthcheck", None).0, 200);
    assert_eq!(http_get(&addr, "/kosync/healthcheck", None).0, 404);
    stop.store(true, Ordering::SeqCst);
    assert_eq!(h.join().unwrap(), 0);
}

#[test]
fn serve_validates_arguments() {
    let sb = Sandbox::new("serve-bad");
    // no library yet
    let (code, _, err) = sb.cli(&["serve", "--port", "0"]);
    assert_eq!(code, 2);
    assert!(err.contains("no library at"), "{err}");
    sb.cli(&["library", "add", s(&sb.epub("a.epub", "B", "Ann", "text."))]);
    let (code, _, err) = sb.cli(&["serve", "--port", "0", "--user", "nopassword"]);
    assert_eq!(code, 2);
    assert!(err.contains("name:password"), "{err}");
    let (code, _, err) = sb.cli(&["serve", "--port", "0", "--user", ":pw"]);
    assert_eq!(code, 2, "{err}");
}

// -- the real binary ---------------------------------------------------------------------------------

#[test]
fn binary_exit_codes() {
    let bin = env!("CARGO_BIN_EXE_bookx-cli");
    let ok = std::process::Command::new(bin)
        .arg("formats")
        .output()
        .unwrap();
    assert_eq!(ok.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&ok.stdout).contains("Input:"));
    let bad = std::process::Command::new(bin)
        .args(["check", "/nonexistent/file.epub"])
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&bad.stderr).starts_with("error:"));
    let usage = std::process::Command::new(bin).output().unwrap();
    assert_eq!(usage.status.code(), Some(2));
}

#[test]
fn binary_honours_the_library_environment_variable() {
    let sb = Sandbox::new("bin-env");
    let a = sb.epub("a.epub", "EnvBook", "Ann", "x.");
    let bin = env!("CARGO_BIN_EXE_bookx-cli");
    let add = std::process::Command::new(bin)
        .env("BOOKX_LIBRARY", sb.lib())
        .args(["library", "add", s(&a)])
        .output()
        .unwrap();
    assert_eq!(add.status.code(), Some(0));
    let list = std::process::Command::new(bin)
        .env("BOOKX_LIBRARY", sb.lib())
        .args(["library", "list"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&list.stdout).contains("EnvBook"));
}

// -- the catalogue is a database ----------------------------------------------------------------

#[test]
fn dump_prints_the_whole_catalogue_as_json() {
    let sb = Sandbox::new("dump");
    sb.cli(&[
        "library",
        "add",
        s(&sb.epub("a.epub", "Alpha", "Ann Author", "First.")),
    ]);
    sb.cli(&[
        "library",
        "add",
        s(&sb.epub("b.epub", "Beta", "Bob Writer", "Second.")),
    ]);
    let (code, out, _) = sb.cli(&["library", "dump"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["books"].as_array().unwrap().len(), 2);
    assert!(v["next_id"].as_u64().unwrap() >= 3);
    let b = &v["books"][0];
    for key in [
        "id",
        "file",
        "meta",
        "added",
        "modified",
        "progress",
        "bookmarks",
        "highlights",
    ] {
        assert!(b.get(key).is_some(), "missing {key}");
    }
    assert_eq!(b["meta"]["title"], "Alpha");
    assert!(
        out.contains("\"file\":"),
        "pretty JSON with one file line per book"
    );
}

#[test]
fn dump_without_a_library_is_an_error() {
    let sb = Sandbox::new("dump-none");
    let (code, _, err) = sb.cli(&["library", "dump"]);
    assert_eq!(code, 2);
    assert!(err.contains("no library at"), "{err}");
}

#[test]
fn an_old_json_library_is_migrated_by_the_first_command_that_opens_it() {
    let sb = Sandbox::new("migrate");
    // build a catalogue, export it as the old JSON file in another library folder
    sb.cli(&[
        "library",
        "add",
        s(&sb.epub("a.epub", "Legacy", "Old Author", "Words.")),
    ]);
    let (_, json, _) = sb.cli(&["library", "dump"]);
    let old = sb.path("old-lib");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("bookx-library.json"), &json).unwrap();

    let (code, out, err) = sb.cli(&["--library", s(&old), "library", "list"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Legacy"), "{out}");
    assert!(err.contains("moved 1 book(s)"), "{err}");
    assert!(!old.join("bookx-library.json").exists());
    assert!(old.join("bookx-library.json.migrated").exists());
    assert!(old.join("bookx-library.db").exists());
    // the second time there is nothing to say
    let (_, out2, err2) = sb.cli(&["--library", s(&old), "library", "list"]);
    assert!(out2.contains("Legacy"));
    assert!(!err2.contains("moved"), "{err2}");
}

#[test]
fn a_damaged_old_catalogue_is_reported_with_its_name() {
    let sb = Sandbox::new("migrate-bad");
    let old = sb.path("old-lib");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("bookx-library.json"), "{ broken").unwrap();
    let (code, _, err) = sb.cli(&["--library", s(&old), "library", "list"]);
    assert_eq!(code, 2);
    assert!(err.contains("bookx-library.json"), "{err}");
    assert!(old.join("bookx-library.json").exists());
}

#[test]
fn changes_made_through_the_cli_are_durable_between_runs() {
    let sb = Sandbox::new("durable");
    sb.cli(&["library", "add", s(&sb.epub("a.epub", "Keep", "K", "x."))]);
    sb.cli(&["library", "add", s(&sb.epub("b.epub", "Drop", "D", "y."))]);
    let (_, json, _) = sb.cli(&["library", "list", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let drop_id = v
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["title"] == "Drop")
        .unwrap()["id"]
        .as_u64()
        .unwrap()
        .to_string();
    assert_eq!(sb.cli(&["library", "remove", &drop_id]).0, 0);
    let (_, out, _) = sb.cli(&["library", "list"]);
    assert!(out.contains("Keep") && !out.contains("Drop"));
    // a new book does not reuse the removed id
    sb.cli(&["library", "add", s(&sb.epub("c.epub", "Newer", "N", "z."))]);
    let (_, json, _) = sb.cli(&["library", "list", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let newer = v
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["title"] == "Newer")
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    assert!(newer > drop_id.parse::<u64>().unwrap());
}

#[test]
fn serve_over_https_when_a_certificate_is_given() {
    let sb = Sandbox::new("serve-tls");
    sb.cli(&[
        "library",
        "add",
        s(&sb.epub("a.epub", "Tls Book", "Ann", "text.")),
    ]);
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let (c, k) = (sb.path("cert.pem"), sb.path("key.pem"));
    std::fs::write(&c, cert.cert.pem()).unwrap();
    std::fs::write(&k, cert.signing_key.serialize_pem()).unwrap();
    let (c, k) = (
        c.to_str().unwrap().to_string(),
        k.to_str().unwrap().to_string(),
    );
    let (_addr, stop, h) = start_server(&sb, &["--tls-cert", &c, "--tls-key", &k]);
    stop.store(true, Ordering::SeqCst);
    assert_eq!(h.join().unwrap(), 0);
}

#[test]
fn a_certificate_without_a_key_is_refused_and_a_bad_file_is_reported() {
    let sb = Sandbox::new("serve-tls-bad");
    sb.cli(&[
        "library",
        "add",
        s(&sb.epub("a.epub", "Tls Book", "Ann", "text.")),
    ]);
    let (code, _out, err) = sb.cli(&["serve", "--port", "0", "--tls-cert", "x.pem"]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("tls-key"), "{err}");
    let (code, _out, err) = sb.cli(&[
        "serve",
        "--port",
        "0",
        "--tls-cert",
        "/no/such.pem",
        "--tls-key",
        "/no/such.key",
    ]);
    assert_ne!(code, 0);
    assert!(err.contains("/no/such.pem"), "{err}");
}
