//! The *Problems* list of the editor's *Check Book* (Calibre editor gap analysis section 2.6): everything the
//! quick checks and the validator find, as one list with stable ids, plain-language help ([`help`]), automatic
//! fixes where they exist ([`fix`]) and the bookkeeping the panel needs (grouping, ignored kinds, next/previous,
//! keeping the selection across re-runs). No GUI in here.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use bookx_core::archive::Archive;

use crate::validate::{Options, Severity, validate_with};

pub mod fix;
pub mod help;

pub use fix::{FixOutcome, Scope, apply_fix, fix_label};
pub use help::{Help, help};

/// Ids for the quick checks that have no validator counterpart: (id, severity, title).
pub const CHECK_RULES: &[(&str, Severity, &str)] = &[
    ("CHK-001", Severity::Error, "the package could not be read"),
    ("CHK-002", Severity::Info, "the book has no author"),
    ("CHK-003", Severity::Info, "the book has no cover image"),
];

/// What to check.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CheckOptions {
    /// "Also check accessibility" (CE-164).
    pub accessibility: bool,
}

/// One entry of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub id: String,
    pub severity: Severity,
    pub file: String,
    pub line: Option<u32>,
    pub col: Option<u32>,
    pub text: String,
}

impl Problem {
    /// Identifies the problem across re-runs (line numbers move when the text is edited).
    pub fn key(&self) -> String {
        format!("{}\u{1}{}\u{1}{}", self.id, self.file, self.text)
    }
}

/// The validator id a quick-check message of `bookx_core::check` stands for.
fn core_id(message: &str) -> Option<&'static str> {
    const MAP: &[(&str, &str)] = &[
        ("Cannot read package", "CHK-001"),
        ("Missing or incorrect mimetype", "PKG-007"),
        ("XML error", "RSC-016"),
        ("The book has no title", "OPF-902"),
        ("The book has no language", "OPF-903"),
        ("The book has no author", "CHK-002"),
        ("The book has no identifier", "OPF-901"),
        ("unique-identifier", "OPF-030"),
        ("The book has no cover", "CHK-003"),
        ("Manifest item", "RSC-001"),
        ("Duplicate manifest id", "OPF-906"),
        ("Spine references unknown", "OPF-049"),
        ("The spine is empty", "OPF-033"),
        ("File is not listed in the manifest", "OPF-003"),
        ("Broken link", "RSC-007"),
        ("Link target #", "RSC-012"),
    ];
    MAP.iter()
        .find(|(p, _)| message.starts_with(p))
        .map(|m| m.1)
}

fn severity_of_core(s: bookx_core::check::Severity) -> Severity {
    match s {
        bookx_core::check::Severity::Error => Severity::Error,
        bookx_core::check::Severity::Warning => Severity::Warning,
        bookx_core::check::Severity::Info => Severity::Info,
    }
}

/// Runs every check on the book: the validator with the extended rules, plus the quick checks of the core for
/// what the validator does not say. Errors come first, then by file and line.
pub fn collect(archive: &Archive, opts: &CheckOptions) -> Vec<Problem> {
    let report = validate_with(
        archive,
        &Options {
            extended: true,
            accessibility: opts.accessibility,
        },
    );
    let mut out: Vec<Problem> = Vec::new();
    for m in report.messages {
        if m.severity == Severity::Usage && !opts.accessibility {
            continue;
        }
        out.push(Problem {
            id: m.id,
            severity: m.severity,
            file: m.file,
            line: m.line,
            col: m.col,
            text: m.text,
        });
    }
    let seen: HashSet<(String, String)> =
        out.iter().map(|p| (p.id.clone(), p.file.clone())).collect();
    for i in bookx_core::check::check_book(archive) {
        let Some(id) = core_id(&i.message) else {
            continue;
        };
        let duplicate = seen.contains(&(id.to_string(), i.file.clone()))
            || (id.starts_with("OPF-9") && out.iter().any(|p| p.id == id))
            || (id == "PKG-007" && out.iter().any(|p| p.id.starts_with("PKG-00")));
        if duplicate {
            continue;
        }
        out.push(Problem {
            id: id.to_string(),
            severity: severity_of_core(i.severity),
            file: i.file,
            line: i.line,
            col: None,
            text: i.message,
        });
    }
    out.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.id.cmp(&b.id))
    });
    out
}

/// Kinds that cannot be skipped: when the book is not readable the other findings mean little (CE-162).
pub fn can_ignore(id: &str) -> bool {
    !matches!(
        id,
        "RSC-016" | "RSC-002" | "RSC-003" | "OPF-002" | "PKG-004" | "CHK-001" | "BKX-027"
    )
}

/// Title of a check id (for the list of ignored kinds).
pub fn title(id: &str) -> String {
    crate::validate::rule(id)
        .map(|r| r.title.to_string())
        .or_else(|| {
            CHECK_RULES
                .iter()
                .find(|r| r.0 == id)
                .map(|r| r.2.to_string())
        })
        .unwrap_or_else(|| id.to_string())
}

/// Removes ignored kinds; returns the rest and how many were hidden.
pub fn without_ignored(
    problems: Vec<Problem>,
    ignored: &BTreeSet<String>,
) -> (Vec<Problem>, usize) {
    let total = problems.len();
    let kept: Vec<Problem> = problems
        .into_iter()
        .filter(|p| !can_ignore(&p.id) || !ignored.contains(&p.id))
        .collect();
    let hidden = total - kept.len();
    (kept, hidden)
}

/// Problem counts by importance: (errors, warnings, notes).
pub fn counts(problems: &[Problem]) -> (usize, usize, usize) {
    let mut c = (0, 0, 0);
    for p in problems {
        match p.severity {
            Severity::Error => c.0 += 1,
            Severity::Warning => c.1 += 1,
            _ => c.2 += 1,
        }
    }
    c
}

/// Indexes of the problems grouped by file, files in the order of their most severe problem.
pub fn group_by_file(problems: &[Problem]) -> Vec<(String, Vec<usize>)> {
    let mut groups: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, p) in problems.iter().enumerate() {
        groups.entry(p.file.as_str()).or_default().push(i);
    }
    let mut out: Vec<(String, Vec<usize>)> = groups
        .into_iter()
        .map(|(f, v)| (f.to_string(), v))
        .collect();
    out.sort_by_key(|(f, v)| (v.iter().map(|i| problems[*i].severity).min(), f.clone()));
    out
}

/// The index to go to for F8 / Shift+F8, wrapping around.
pub fn step(len: usize, current: Option<usize>, forward: bool) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some(match (current, forward) {
        (None, true) => 0,
        (None, false) => len - 1,
        (Some(i), true) => (i + 1) % len,
        (Some(i), false) => (i + len - 1) % len,
    })
}

/// After a re-run: where the previously selected problem is now (same id, file and text); when it is gone, the
/// first problem of the same kind in the same file, else nothing (CE-165).
pub fn reselect(old: &Problem, new: &[Problem]) -> Option<usize> {
    new.iter().position(|p| p.key() == old.key()).or_else(|| {
        new.iter()
            .position(|p| p.id == old.id && p.file == old.file)
    })
}

/// Keys of the problems in `file`: they may be out of date after the file was edited.
pub fn stale_keys(problems: &[Problem], file: &str) -> Vec<String> {
    problems
        .iter()
        .filter(|p| p.file == file)
        .map(Problem::key)
        .collect()
}

#[cfg(test)]
mod tests;
