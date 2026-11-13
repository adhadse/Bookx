//! Optional oracle test: the validator's verdict (passed / failed) must agree with `epubveri` (<https://github.com/veripublica/epubveri>, a Rust binary) on a corpus
//! of EPUBs generated in code (valid books and books with exactly one injected fault). The comparison runs only
//! when the binary is found (`$BOOKX_EPUBVERI`, `epubveri` on PATH, or `~/.cache/epubveri/bin/epubveri`); set `BOOKX_ORACLE=0` to skip it explicitly. Without
//! epubveri the test still checks the validator against the verdicts recorded in the fixtures.

use std::process::{Command, Stdio};

use bookx_editing::testepub::*;
use bookx_editing::validate::validate_bytes;

struct Case {
    name: String,
    bytes: Vec<u8>,
    /// epubveri is expected to report at least one error
    expect_error: bool,
}

/// Faults that epubveri only reports as warnings, usage messages or not at all.
const VALID_WITH_NOTES: &[&str] = &["OPF-003", "PKG-010", "PKG-012", "OPF-918", "OPF-088"];

/// Cases where Bookx and epubveri deliberately differ. Each is explained in docs/validator-notes.md; the comparison skips them.
const DIFFERENCES_FROM_EPUBVERI: &[&str] = &["fault-HTM-902", "fault-NCX-902", "fault-NCX-904"];

fn corpus() -> Vec<Case> {
    let mut out = vec![
        Case {
            name: "clean-epub3".into(),
            bytes: to_bytes(&clean_epub3()),
            expect_error: false,
        },
        Case {
            name: "clean-epub2".into(),
            bytes: to_bytes(&clean_epub2()),
            expect_error: false,
        },
    ];
    for fault in faults() {
        let valid_note = VALID_WITH_NOTES.contains(&fault.id);
        if !(fault.oracle_error || valid_note) {
            continue;
        }
        let mut files = if fault.version == 2 {
            clean_epub2()
        } else {
            clean_epub3()
        };
        (fault.apply)(&mut files);
        out.push(Case {
            name: format!("{}-{}", if valid_note { "note" } else { "fault" }, fault.id),
            bytes: to_bytes(&files),
            expect_error: !valid_note,
        });
    }
    for (id, bytes) in zip_faults() {
        if ["PKG-005", "PKG-006", "PKG-022"].contains(&id) {
            out.push(Case {
                name: format!("fault-zip-{id}"),
                bytes,
                expect_error: true,
            });
        }
    }
    out
}

/// The validator binary: `$BOOKX_EPUBVERI`, else `epubveri` on `PATH`, else `~/.cache/epubveri/bin/epubveri`.
fn epubveri_binary() -> Option<std::path::PathBuf> {
    if std::env::var("BOOKX_ORACLE").as_deref() == Ok("0") {
        return None;
    }
    let mut candidates = Vec::new();
    if let Some(p) = std::env::var_os("BOOKX_EPUBVERI") {
        candidates.push(std::path::PathBuf::from(p));
    }
    candidates.push("epubveri".into());
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(std::path::Path::new(&home).join(".cache/epubveri/bin/epubveri"));
    }
    candidates.into_iter().find(|c| {
        Command::new(c)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

#[test]
fn the_corpus_has_valid_books_and_single_fault_books() {
    let c = corpus();
    assert!(c.len() >= 30, "{} cases", c.len());
    assert!(c.iter().filter(|c| !c.expect_error).count() >= 6);
    let mut names: Vec<&str> = c.iter().map(|c| c.name.as_str()).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), c.len(), "unique names");
}

#[test]
fn the_validator_agrees_with_the_recorded_verdicts() {
    let mut wrong = Vec::new();
    for case in corpus() {
        let r = validate_bytes(&case.bytes);
        if r.passed == case.expect_error {
            wrong.push(case.name);
        }
    }
    assert!(
        wrong.is_empty(),
        "verdict differs from the fixture for {wrong:?}"
    );
}

#[test]
fn the_validator_agrees_with_epubveri() {
    let Some(bin) = epubveri_binary() else {
        eprintln!("epubveri is not installed (or BOOKX_ORACLE=0), skipping the oracle comparison");
        return;
    };
    let dir = std::env::temp_dir().join(format!("bookx-oracle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cases = corpus();
    let workers = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(8);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let disagreements = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some(case) = cases.get(i) else { break };
                if DIFFERENCES_FROM_EPUBVERI.contains(&case.name.as_str()) {
                    continue;
                }
                let path = dir.join(format!("{}.epub", case.name));
                std::fs::write(&path, &case.bytes).unwrap();
                let out = Command::new(&bin)
                    .arg("-i")
                    .arg(&path)
                    .output()
                    .expect("running epubveri");
                let epubveri_failed = !out.status.success();
                let ours_failed = !validate_bytes(&case.bytes).passed;
                if epubveri_failed != ours_failed {
                    let msg = String::from_utf8_lossy(&out.stdout);
                    let first = msg.lines().next().unwrap_or("");
                    disagreements.lock().unwrap().push(format!(
                        "{}: epubveri {}, bookx {} ({first})",
                        case.name,
                        if epubveri_failed { "fails" } else { "passes" },
                        if ours_failed { "fails" } else { "passes" }
                    ));
                }
            });
        }
    });
    let mut disagreements = disagreements.into_inner().unwrap();
    disagreements.sort();
    if std::env::var_os("BOOKX_ORACLE_KEEP").is_none() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    assert!(
        disagreements.is_empty(),
        "verdicts differ:\n{}",
        disagreements.join("\n")
    );
}
