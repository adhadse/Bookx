//! Parity with Calibre: every corpus document is converted to every output format by both
//! Calibre's `ebook-convert` and Bookx, and the results are compared structurally (metadata, table
//! of contents, text as a normalised word stream, images, cover, internal links).
//!
//! Checks per cell (source S, target T):
//! * `W` writer: Calibre reads Bookx's T file; compared with Calibre reading its own T file.
//! * `R` reader: Bookx reads Calibre's T file; compared with Calibre reading it.
//! * `E` end to end: Bookx reads Bookx's T file; compared with Calibre reading Calibre's T file.
//! * `L` loss: text of Bookx's T file against Bookx's own EPUB conversion (words only).
//! * `V` validity of Bookx's EPUB/KEPUB output (repo validator and epubveri when installed).
//!
//! The test skips itself when `ebook-convert` is missing or `BOOKX_PARITY=0`; without
//! `BOOKX_PARITY=1` it runs only a small smoke subset (a few seconds). Known and accepted
//! differences live in `tests/parity_known.txt`; anything else fails the test. Environment:
//! `PARITY_SAMPLES=a,b` and `PARITY_TARGETS=epub,mobi` narrow the run, `JOBS=n` sets concurrency,
//! `BOOKX_PARITY_OUT=dir` keeps the matrix (`parity.md`, `parity.json`) and `BOOKX_PARITY_REPORT=1`
//! only reports instead of failing.

mod common;
mod corpus;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use bookx_convert::inspect::{compare, regressions, score, Digest, Score, Tolerance};
use bookx_convert::{convert, read_input, ConvertOptions};
use bookx_core::contracts::{FormatReader, Registry};

const TARGETS: &[&str] = &[
    "epub", "mobi", "azw3", "fb2", "htmlz", "txtz", "txt", "docx", "rtf", "pdb", "pmlz", "rb",
    "snb",
];

/// `BOOKX_PARITY=1` runs the whole matrix, unset runs a quick smoke subset, `0` skips.
fn full_matrix() -> bool {
    std::env::var("BOOKX_PARITY").is_ok_and(|v| v == "1")
}

fn calibre_ok() -> bool {
    std::env::var("BOOKX_PARITY").map_or(true, |v| v != "0")
        && Command::new("ebook-convert")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
}

fn readers() -> Registry<dyn FormatReader> {
    bookx_formats::default_readers()
}

fn calibre(input: &Path, output: &Path, work: &Path) -> Result<(), String> {
    calibre_args(input, output, work, &[])
}

fn calibre_args(input: &Path, output: &Path, work: &Path, extra: &[&str]) -> Result<(), String> {
    let cfg = work.join("calibre-config");
    let _ = std::fs::create_dir_all(&cfg);
    let out = Command::new("timeout")
        .arg("180")
        .arg("ebook-convert")
        .arg(input)
        .arg(output)
        .args(extra)
        .env("CALIBRE_CONFIG_DIRECTORY", &cfg)
        .env("CALIBRE_CACHE_DIRECTORY", work.join("calibre-cache"))
        .env("QT_QPA_PLATFORM", "offscreen")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() || !output.exists() {
        let text = String::from_utf8_lossy(&out.stdout);
        let tail: String = text.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
        let err = String::from_utf8_lossy(&out.stderr);
        let etail: String = err.lines().rev().take(2).collect::<Vec<_>>().join(" | ");
        return Err(format!("{tail} {etail}").trim().chars().take(300).collect());
    }
    Ok(())
}

fn bookx(input: &Path, ext: &str) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(input).map_err(|e| e.to_string())?;
    let name = input.file_name().unwrap().to_string_lossy().to_string();
    let ext = ext.to_string();
    let r = std::panic::catch_unwind(|| {
        convert(
            &readers(),
            &bytes,
            &name,
            &ext,
            &ConvertOptions::default(),
            &|_| {},
            &std::sync::atomic::AtomicBool::new(false),
        )
    });
    match r {
        Ok(Ok(c)) => Ok(c.bytes),
        Ok(Err(e)) => Err(format!("{e:#}").chars().take(300).collect()),
        Err(_) => Err("PANIC".into()),
    }
}

fn digest_bookx(path: &Path) -> Result<Digest, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let r = std::panic::catch_unwind(|| read_input(&readers(), &bytes, &name));
    match r {
        Ok(Ok(book)) => Ok(Digest::of(&book)),
        Ok(Err(e)) => Err(format!("{e:#}").chars().take(300).collect()),
        Err(_) => Err("PANIC".into()),
    }
}

fn digest_calibre(path: &Path, work: &Path, tag: &str) -> Result<Digest, String> {
    let out = work.join(format!("{tag}.via-calibre.epub"));
    calibre_args(path, &out, work, &["--no-default-epub-cover"])?;
    digest_bookx(&out)
}

#[derive(Debug, Clone, serde::Serialize)]
struct Cell {
    sample: String,
    target: String,
    /// `W`/`R`/`E`/`L`/`V` -> list of "category: detail"
    diffs: BTreeMap<String, Vec<(String, String)>>,
    /// Hard failures (conversion errors on either side).
    notes: Vec<String>,
}

fn epubveri_errors(path: &Path) -> Option<Vec<String>> {
    let home = std::env::var("HOME").unwrap_or_default();
    let bin = std::env::var("BOOKX_EPUBVERI")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            Some(PathBuf::from(format!(
                "{home}/.cache/epubveri/bin/epubveri"
            )))
        })
        .filter(|p| p.exists())?;
    let out = Command::new(bin)
        .args(["-i"])
        .arg(path)
        .args(["--format", "json"])
        .output()
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let items = v["inputs"][0]["items"].as_array()?;
    Some(
        items
            .iter()
            .filter(|i| matches!(i["severity"].as_str(), Some("error") | Some("fatal")))
            .map(|i| {
                format!(
                    "{} {}",
                    i["id"].as_str().unwrap_or("?"),
                    i["message"].as_str().unwrap_or("")
                )
            })
            .collect(),
    )
}

fn run_cell(
    sample: &corpus::Sample,
    src: &Path,
    truth: &Digest,
    target: &str,
    work: &Path,
) -> Cell {
    let mut cell = Cell {
        sample: sample.name.to_string(),
        target: target.to_string(),
        diffs: BTreeMap::new(),
        notes: Vec::new(),
    };
    let tag = format!("{}.{}", sample.name, target);
    let cal_out = work.join(format!("{tag}.calibre.{target}"));
    let bx_out = work.join(format!("{tag}.bookx.{target}"));
    let cal = calibre(src, &cal_out, work);
    let bx = bookx(src, target);
    if let Err(e) = &cal {
        cell.notes.push(format!("calibre-fails: {e}"));
    }
    if let Err(e) = &bx {
        cell.notes.push(format!("bookx-fails: {e}"));
    }
    if let Ok(b) = &bx {
        let _ = std::fs::write(&bx_out, b);
    }
    let put = |cell: &mut Cell, check: &str, theirs: &Score, ours: &Score| {
        let d = regressions(theirs, ours);
        if !d.is_empty() {
            cell.diffs.insert(
                check.to_string(),
                d.into_iter()
                    .map(|x| (x.category.to_string(), x.detail))
                    .collect(),
            );
        }
    };
    // Calibre reading Calibre's own output is the yardstick
    let cal_cal = if cal.is_ok() {
        match digest_calibre(&cal_out, work, &format!("{tag}.c")) {
            Ok(d) => Some(score(truth, &d)),
            Err(e) => {
                cell.notes
                    .push(format!("calibre-rereads-own-output-fails: {e}"));
                None
            }
        }
    } else {
        None
    };
    if let Some(yard) = &cal_cal {
        if bx.is_ok() {
            match digest_calibre(&bx_out, work, &format!("{tag}.b")) {
                Ok(d) => put(&mut cell, "W", yard, &score(truth, &d)),
                Err(e) => cell
                    .notes
                    .push(format!("calibre-rejects-bookx-output: {e}")),
            }
        }
        match digest_bookx(&cal_out) {
            Ok(d) => put(&mut cell, "R", yard, &score(truth, &d)),
            Err(e) => cell
                .notes
                .push(format!("bookx-rejects-calibre-output: {e}")),
        }
    }
    if bx.is_ok() {
        match digest_bookx(&bx_out) {
            Ok(own) => {
                if let Some(yard) = &cal_cal {
                    put(&mut cell, "E", yard, &score(truth, &own));
                }
                // loss relative to Bookx's own EPUB conversion of the same source
                if let Ok(epub) = bookx(src, "epub") {
                    let p = work.join(format!("{tag}.ref.epub"));
                    let _ = std::fs::write(&p, epub);
                    if let Ok(r) = digest_bookx(&p) {
                        let mut d = compare(&r, &own, Tolerance::default());
                        d.retain(|x| x.category == "text-missing");
                        if !d.is_empty() {
                            cell.diffs.insert(
                                "L".into(),
                                d.into_iter()
                                    .map(|x| (x.category.to_string(), x.detail))
                                    .collect(),
                            );
                        }
                    }
                }
            }
            Err(e) => cell.notes.push(format!("bookx-rejects-own-output: {e}")),
        }
        if target == "epub" {
            let mut v = Vec::new();
            if let Ok(book) = bookx_core::book::OpenBook::open(&bx_out) {
                for i in bookx_core::check::check_book(&book.archive) {
                    if matches!(i.severity, bookx_core::check::Severity::Error) {
                        v.push((
                            "validator".to_string(),
                            format!("{}: {}", i.file, i.message),
                        ));
                    }
                }
            }
            if let Some(errs) = epubveri_errors(&bx_out) {
                for e in errs {
                    v.push(("epubveri".to_string(), e));
                }
            }
            if !v.is_empty() {
                cell.diffs.insert("V".into(), v);
            }
        }
    }
    cell
}

fn known_list() -> Vec<(String, String, String, String)> {
    let text = include_str!("parity_known.txt");
    text.lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            (p.len() == 4).then(|| (p[0].into(), p[1].into(), p[2].into(), p[3].into()))
        })
        .collect()
}

fn matches_known(
    known: &[(String, String, String, String)],
    c: &Cell,
    check: &str,
    cat: &str,
) -> bool {
    known.iter().any(|(s, t, ch, ca)| {
        (s == "*" || s == &c.sample)
            && (t == "*" || t == &c.target)
            && (ch == "*" || ch == check)
            && (ca == "*" || ca == cat)
    })
}

/// Calibre-made documents used as sources, to exercise the binary readers on real writer output.
fn calibre_made(work: &Path) -> Vec<(corpus::Sample, PathBuf)> {
    let base = [
        (
            "epub3-features",
            [
                "mobi", "azw3", "pdb", "pmlz", "rb", "snb", "fb2", "docx", "rtf",
            ]
            .as_slice(),
        ),
        ("epub-long-toc", ["mobi", "azw3"].as_slice()),
        ("epub-rtl", ["mobi", "azw3"].as_slice()),
        ("epub-ruby-cjk", ["azw3"].as_slice()),
    ];
    let mut out = Vec::new();
    for (name, fmts) in base {
        let s = corpus::by_name(name);
        let src = work.join(&s.file);
        std::fs::write(&src, &s.bytes).unwrap();
        for f in fmts {
            let p = work.join(format!("cal-{name}.{f}"));
            if calibre(&src, &p, work).is_ok() {
                let bytes = std::fs::read(&p).unwrap();
                let label: &'static str = Box::leak(format!("cal-{name}-{f}").into_boxed_str());
                out.push((
                    corpus::Sample {
                        name: label,
                        file: format!("cal-{name}.{f}"),
                        bytes,
                    },
                    p,
                ));
            }
        }
    }
    out
}

fn render(
    cells: &[Cell],
    samples: &[String],
    targets: &[String],
    known: &[(String, String, String, String)],
) -> String {
    let mut s = String::new();
    s.push_str("| source \\ target |");
    for t in targets {
        s.push_str(&format!(" {t} |"));
    }
    s.push_str("\n|---|");
    for _ in targets {
        s.push_str("---|");
    }
    s.push('\n');
    for name in samples {
        s.push_str(&format!("| {name} |"));
        for t in targets {
            let c = cells.iter().find(|c| &c.sample == name && &c.target == t);
            let txt = match c {
                None => "·".to_string(),
                Some(c) => {
                    let mut parts = Vec::new();
                    for n in &c.notes {
                        let short = n.split(':').next().unwrap_or(n);
                        parts.push(format!("**{short}**"));
                    }
                    for (check, ds) in &c.diffs {
                        let mut cats: Vec<String> = Vec::new();
                        for (cat, _) in ds {
                            let mark = if matches_known(known, c, check, cat) {
                                ""
                            } else {
                                "!"
                            };
                            let e = format!("{mark}{cat}");
                            if !cats.contains(&e) {
                                cats.push(e);
                            }
                        }
                        parts.push(format!("{check}:{}", cats.join("+")));
                    }
                    if parts.is_empty() {
                        "ok".to_string()
                    } else {
                        parts.join("<br>")
                    }
                }
            };
            s.push_str(&format!(" {txt} |"));
        }
        s.push('\n');
    }
    s
}

#[test]
fn calibre_parity_matrix() {
    if !calibre_ok() {
        eprintln!("skipped: ebook-convert not available (or BOOKX_PARITY=0)");
        return;
    }
    let work = common::temp_dir("parity");
    let smoke = !full_matrix();
    let want: Option<Vec<String>> = std::env::var("PARITY_SAMPLES")
        .ok()
        .map(|v| v.split(',').map(str::to_string).collect())
        .or_else(|| smoke.then(|| vec!["epub3-features".into(), "txt-utf8".into()]));
    let targets: Vec<String> = std::env::var("PARITY_TARGETS")
        .ok()
        .map(|v| v.split(',').map(str::to_string).collect())
        .unwrap_or_else(|| {
            if smoke {
                vec!["epub".into(), "mobi".into(), "azw3".into()]
            } else {
                TARGETS.iter().map(|s| s.to_string()).collect()
            }
        });

    let mut sources: Vec<(corpus::Sample, PathBuf)> = Vec::new();
    for s in corpus::all() {
        let p = work.join(&s.file);
        std::fs::write(&p, &s.bytes).unwrap();
        sources.push((s, p));
    }
    if !smoke
        || want
            .as_ref()
            .is_some_and(|w| w.iter().any(|n| n.starts_with("cal-")))
    {
        sources.extend(calibre_made(&work));
    }
    if let Some(w) = &want {
        sources.retain(|(s, _)| {
            w.iter()
                .any(|n| s.name == n || s.name.starts_with(n.as_str()))
        });
    }
    let jobs: Vec<(usize, String)> = sources
        .iter()
        .enumerate()
        .flat_map(|(i, _)| targets.iter().map(move |t| (i, t.clone())))
        .collect();
    let n_threads = std::env::var("JOBS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
                .min(12)
        });
    let truths: Vec<Option<Digest>> = std::thread::scope(|sc| {
        let hs: Vec<_> = sources
            .iter()
            .map(|(s, p)| {
                let work = &work;
                sc.spawn(move || {
                    let mut d = digest_calibre(p, work, &format!("{}.truth", s.name)).ok()?;
                    // Calibre invents contents entries for sources that have none ("Start"),
                    // so only formats with a real table of contents are a yardstick for it
                    let ext = s.file.rsplit('.').next().unwrap_or("");
                    if !matches!(ext, "epub" | "fb2" | "docx" | "mobi" | "azw3") {
                        d.toc.clear();
                    }
                    Some(d)
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let next = AtomicUsize::new(0);
    let cells = Mutex::new(Vec::<Cell>::new());
    std::thread::scope(|sc| {
        for _ in 0..n_threads {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some((si, t)) = jobs.get(i) else { break };
                let (s, p) = &sources[*si];
                let Some(truth) = &truths[*si] else {
                    cells.lock().unwrap().push(Cell {
                        sample: s.name.to_string(),
                        target: t.clone(),
                        diffs: BTreeMap::new(),
                        notes: vec!["calibre-cannot-read-source: no yardstick".into()],
                    });
                    continue;
                };
                let cell = run_cell(s, p, truth, t, &work);
                cells.lock().unwrap().push(cell);
            });
        }
    });
    let cells = cells.into_inner().unwrap();
    let known = known_list();
    let sample_names: Vec<String> = sources.iter().map(|(s, _)| s.name.to_string()).collect();
    let md = render(&cells, &sample_names, &targets, &known);

    let mut unknown = Vec::new();
    let mut details = String::new();
    let mut sorted = cells.clone();
    sorted.sort_by(|a, b| (&a.sample, &a.target).cmp(&(&b.sample, &b.target)));
    for c in &sorted {
        for n in &c.notes {
            let cat = n.split(':').next().unwrap_or(n);
            details.push_str(&format!("- {} → {}: {}\n", c.sample, c.target, n));
            if !matches_known(&known, c, "N", cat) {
                unknown.push(format!("{} {} N {cat}  ({n})", c.sample, c.target));
            }
        }
        for (check, ds) in &c.diffs {
            for (cat, detail) in ds {
                details.push_str(&format!(
                    "- {} → {} [{check}] {cat}: {detail}\n",
                    c.sample, c.target
                ));
                if !matches_known(&known, c, check, cat) {
                    unknown.push(format!(
                        "{} {} {check} {cat}  ({detail})",
                        c.sample, c.target
                    ));
                }
            }
        }
    }
    if let Ok(dir) = std::env::var("BOOKX_PARITY_OUT") {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("parity.md"), format!("{md}\n\n{details}")).unwrap();
        std::fs::write(
            dir.join("parity.json"),
            serde_json::to_string_pretty(&sorted).unwrap(),
        )
        .unwrap();
        std::fs::write(dir.join("unknown.txt"), unknown.join("\n")).unwrap();
    }
    if std::env::var("BOOKX_PARITY_KEEP").is_err() {
        let _ = std::fs::remove_dir_all(&work);
    }
    eprintln!("{md}");
    if std::env::var("BOOKX_PARITY_REPORT").is_ok() {
        eprintln!("{} unlisted differences", unknown.len());
        return;
    }
    assert!(
        unknown.is_empty(),
        "differences from Calibre not listed in tests/parity_known.txt:\n{}",
        unknown.join("\n")
    );
}

/// `PARITY_EXPLAIN=fb2-full cargo test --test parity explain -- --ignored --nocapture` lists the
/// words and contents entries that differ between Calibre reading its own EPUB of the source and
/// Calibre reading Bookx's file (`PARITY_TARGET`, default epub) for one sample.
#[test]
#[ignore]
fn explain() {
    let Ok(name) = std::env::var("PARITY_EXPLAIN") else {
        return;
    };
    let s = corpus::by_name(&name);
    let work = common::temp_dir("explain");
    let src = work.join(&s.file);
    std::fs::write(&src, &s.bytes).unwrap();
    let truth = digest_calibre(&src, &work, "truth").expect("calibre reads the source");
    let target = std::env::var("PARITY_TARGET").unwrap_or_else(|_| "epub".into());
    let bx = bookx(&src, &target).expect("bookx converts");
    let p = work.join(format!("bx.{target}"));
    std::fs::write(&p, bx).unwrap();
    // what a user of the other program would see: Calibre reading Bookx's file
    let ours = digest_calibre(&p, &work, "ours").expect("calibre reads bookx's output");
    let (missing, extra) = bookx_convert::inspect::word_delta(&truth.words, &ours.words);
    eprintln!("missing: {missing:?}\nextra: {extra:?}");
    eprintln!("toc calibre: {:?}\ntoc bookx:   {:?}", truth.toc, ours.toc);
    eprintln!(
        "title {:?}/{:?} authors {:?}/{:?} lang {:?}/{:?} cover {}/{} images {}/{} ids {:?}/{:?}",
        truth.title,
        ours.title,
        truth.authors,
        ours.authors,
        truth.language,
        ours.language,
        truth.has_cover,
        ours.has_cover,
        truth.images,
        ours.images,
        truth.identifiers,
        ours.identifiers
    );
    let _ = std::fs::remove_dir_all(&work);
}
