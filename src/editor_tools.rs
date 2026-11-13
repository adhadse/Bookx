//! The editor's *Tools* menu (roadmap E1, E2, E3, E13, E17-E19): each tool runs one operation of
//! `bookx-editing` on the open book and says what it did. No GTK in here, so it is unit tested.

use crate::i18n::tr;
use crate::i18n::trf;
use crate::i18n::{ntr, N_};
use anyhow::Result;
use bookx_editing::accessibility::{make_accessible, AccessibilityOptions};
use bookx_editing::fixes::{fix_book, FixOptions};
use bookx_editing::fonts::subset_embedded_fonts;
use bookx_editing::images::{optimize_images, ImageOptions};
use bookx_editing::inline_toc::{insert_inline_toc, remove_inline_toc, InlineTocOptions};
use bookx_editing::links::{arrange_files, ArrangeOptions};
use bookx_editing::reports::{build_report, ReportOptions};
use bookx_editing::upgrade::{upgrade_to_epub3, UpgradeOptions};

mod more;
pub mod rules;
pub use more::*;

#[cfg(test)]
use crate::core::archive::Archive;
use crate::core::book::OpenBook;
#[cfg(test)]
use crate::core::check::check_book;
use crate::core::check::{Issue, Severity};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    FixProblems,
    UpgradeToEpub3,
    AddInlineToc,
    RemoveInlineToc,
    MakeAccessible,
    CompressImages,
    SubsetFonts,
    ArrangeFiles,
}

impl Tool {
    pub const ALL: [Tool; 8] = [
        Tool::FixProblems,
        Tool::UpgradeToEpub3,
        Tool::AddInlineToc,
        Tool::RemoveInlineToc,
        Tool::MakeAccessible,
        Tool::CompressImages,
        Tool::SubsetFonts,
        Tool::ArrangeFiles,
    ];

    /// Stable name (menu action target, dev commands).
    pub fn id(self) -> &'static str {
        match self {
            Tool::FixProblems => "fix",
            Tool::UpgradeToEpub3 => "upgrade",
            Tool::AddInlineToc => "toc-add",
            Tool::RemoveInlineToc => "toc-remove",
            Tool::MakeAccessible => "accessible",
            Tool::CompressImages => "images",
            Tool::SubsetFonts => "fonts",
            Tool::ArrangeFiles => "arrange",
        }
    }

    pub fn from_id(id: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::FixProblems => N_("Fix Problems Automatically"),
            Tool::UpgradeToEpub3 => N_("Upgrade to EPUB 3"),
            Tool::AddInlineToc => N_("Insert Contents Page"),
            Tool::RemoveInlineToc => N_("Remove Contents Page"),
            Tool::MakeAccessible => N_("Improve Accessibility"),
            Tool::CompressImages => N_("Compress Images"),
            Tool::SubsetFonts => N_("Shrink Embedded Fonts"),
            Tool::ArrangeFiles => N_("Arrange Files in Folders"),
        }
    }
}

/// What a tool did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub changed: bool,
    /// One line for the toast.
    pub summary: String,
    /// What exactly changed (shown in the results list).
    pub details: Vec<String>,
}

fn outcome(changed: bool, summary: impl Into<String>, details: Vec<String>) -> Outcome {
    Outcome {
        changed,
        summary: summary.into(),
        details,
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn kib(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

/// Runs `tool` on `book`. The caller saves a checkpoint first and reloads its views afterwards.
pub fn run(tool: Tool, book: &mut OpenBook) -> Result<Outcome> {
    match tool {
        Tool::FixProblems => {
            let r = fix_book(book, &FixOptions::default())?;
            let details: Vec<String> = r
                .fixes
                .iter()
                .map(|f| format!("{}: {}", f.file, f.description))
                .chain(r.failures.iter().map(|f| format!("could not repair: {f}")))
                .collect();
            let summary = if r.fixes.is_empty() {
                tr("Nothing to fix")
            } else {
                format!("Fixed {}", plural(r.fixes.len(), "problem", "problems"))
            };
            let summary = if r.remaining_errors.is_empty() {
                summary
            } else {
                format!(
                    "{summary}; {} left that need a manual fix",
                    plural(r.remaining_errors.len(), "error", "errors")
                )
            };
            let mut details = details;
            details.extend(
                r.remaining_errors
                    .iter()
                    .map(|e| format!("still wrong: {e}")),
            );
            Ok(outcome(!r.fixes.is_empty(), summary, details))
        }
        Tool::UpgradeToEpub3 => {
            let r = upgrade_to_epub3(book, &UpgradeOptions::default())?;
            Ok(if r.changes.is_empty() {
                outcome(false, tr("Already EPUB 3"), vec![])
            } else {
                outcome(true, tr("Upgraded to EPUB 3"), r.changes)
            })
        }
        Tool::AddInlineToc => {
            let file = insert_inline_toc(book, &InlineTocOptions::default())?;
            Ok(outcome(
                true,
                tr("Contents page added"),
                vec![format!("written to {file}")],
            ))
        }
        Tool::RemoveInlineToc => {
            let removed = remove_inline_toc(book)?;
            Ok(if removed {
                outcome(true, tr("Contents page removed"), vec![])
            } else {
                outcome(false, tr("There is no contents page to remove"), vec![])
            })
        }
        Tool::MakeAccessible => {
            let r = make_accessible(book, &AccessibilityOptions::default())?;
            let mut details = r.changes.clone();
            details.extend(r.alt_issues.iter().map(|a| format!("{a:?}")));
            details.extend(r.table_issues.iter().cloned());
            let changed = !r.changes.is_empty();
            let summary = match (changed, r.alt_issues.len()) {
                (false, 0) => tr("Nothing to improve"),
                (false, n) => format!("{} still need a description", plural(n, "image", "images")),
                (true, 0) => tr("Accessibility improved"),
                (true, n) => format!(
                    "Accessibility improved; {} still need a description",
                    plural(n, "image", "images")
                ),
            };
            Ok(outcome(changed, summary, details))
        }
        Tool::CompressImages => {
            let r = optimize_images(book, &ImageOptions::default())?;
            let saved = r.bytes_before().saturating_sub(r.bytes_after());
            let details: Vec<String> = r
                .changes
                .iter()
                .map(|c| format!("{}: {} → {}", c.path, kib(c.before), kib(c.after)))
                .collect();
            Ok(if r.changes.is_empty() {
                outcome(
                    false,
                    tr("The images are already as small as they can get"),
                    details,
                )
            } else {
                outcome(
                    true,
                    trf(
                        "Optimised {}, saved {}",
                        &[&plural(r.changes.len(), "image", "images"), &kib(saved)],
                    ),
                    details,
                )
            })
        }
        Tool::SubsetFonts => {
            let r = subset_embedded_fonts(book);
            let saved: usize = r.iter().map(|s| s.before.saturating_sub(s.after)).sum();
            let details: Vec<String> = r
                .iter()
                .map(|s| {
                    format!(
                        "{}: {} → {} ({} characters)",
                        s.path,
                        kib(s.before),
                        kib(s.after),
                        s.chars
                    )
                })
                .collect();
            Ok(if r.is_empty() {
                outcome(false, tr("No embedded fonts to shrink"), details)
            } else {
                outcome(
                    true,
                    trf(
                        "Shrank {}, saved {}",
                        &[&plural(r.len(), "font", "fonts"), &kib(saved)],
                    ),
                    details,
                )
            })
        }
        Tool::ArrangeFiles => {
            let r = arrange_files(book, &ArrangeOptions::default())?;
            let details: Vec<String> = r.moved.iter().map(|(a, b)| format!("{a} → {b}")).collect();
            Ok(if r.moved.is_empty() {
                outcome(false, tr("The files are already arranged"), details)
            } else {
                outcome(
                    true,
                    trf("Moved {}", &[&plural(r.moved.len(), "file", "files")]),
                    details,
                )
            })
        }
    }
}

/// The quick checks of the editor plus the EPUB validator, in one list. Usage hints of the
/// validator are left out; its ids are put in front of the text (`[OPF-003] ...`).
#[cfg(test)]
pub fn check_all(archive: &Archive) -> Vec<Issue> {
    use bookx_editing::validate::{validate, Severity as V};
    let mut out = check_book(archive);
    for m in validate(archive).messages {
        let severity = match m.severity {
            V::Error => Severity::Error,
            V::Warning => Severity::Warning,
            V::Info => Severity::Info,
            V::Usage => continue,
        };
        out.push(Issue {
            severity,
            file: m.file,
            line: m.line,
            message: format!("[{}] {}", m.id, m.text),
        });
    }
    out
}

/// Splits `path` into one file per heading of level `1..=level`.
pub fn split_at_headings(book: &mut OpenBook, path: &str, level: u8) -> Result<Outcome> {
    use bookx_editing::split_merge::{split_file, SplitSpec};
    let r = match split_file(book, path, &SplitSpec::Headings(level.clamp(1, 6))) {
        Ok(r) => r,
        Err(e) if e.to_string().starts_with("nothing to split") => {
            return Ok(outcome(
                false,
                tr("There is nothing to split at those headings"),
                vec![],
            ));
        }
        Err(e) => return Err(e),
    };
    Ok(if r.parts.len() <= 1 {
        outcome(
            false,
            tr("There is nothing to split at those headings"),
            vec![],
        )
    } else {
        outcome(
            true,
            trf("Split into {}", &[&plural(r.parts.len(), "file", "files")]),
            r.parts,
        )
    })
}

/// Merges `path` with the file that follows it in the reading order.
pub fn merge_with_next(book: &mut OpenBook, path: &str) -> Result<Outcome> {
    use bookx_editing::split_merge::merge_files;
    let spine = book.spine_paths();
    let Some(i) = spine.iter().position(|p| p == path) else {
        anyhow::bail!("Only files in the reading order can be merged");
    };
    let Some(next) = spine.get(i + 1) else {
        return Ok(outcome(
            false,
            tr("This is the last file; there is nothing to merge it with"),
            vec![],
        ));
    };
    let r = merge_files(book, &[path.to_string(), next.clone()])?;
    let mut details: Vec<String> = r
        .removed
        .iter()
        .map(|f| format!("{f} merged into {}", r.target))
        .collect();
    details.extend(
        r.renamed_ids
            .iter()
            .map(|(f, a, b)| format!("{f}: id {a} became {b}")),
    );
    Ok(outcome(true, tr("Merged with the next file"), details))
}

/// Renames one file (new name in the same folder) and repairs every reference to it (CE-023).
pub fn rename_one(book: &mut OpenBook, old: &str, new_name: &str) -> Result<Outcome> {
    use bookx_editing::files::{rename_files, renamed_in_place};
    let new_name = new_name.trim();
    if new_name.is_empty() || new_name.contains('/') {
        anyhow::bail!("Enter a file name without a folder");
    }
    if new_name == bookx_core::util::file_name(old) {
        return Ok(outcome(false, tr("The name is unchanged"), vec![]));
    }
    let new = renamed_in_place(old, new_name);
    let map = std::collections::HashMap::from([(old.to_string(), new.clone())]);
    let n = rename_files(book, &map)?;
    Ok(outcome(
        true,
        tr("Renamed and updated references"),
        vec![format!("{old} is now {new} ({n} files updated)")],
    ))
}

/// Renames several files in one go (bulk rename, change extension); references follow.
pub fn rename_many(
    book: &mut OpenBook,
    map: &std::collections::HashMap<String, String>,
) -> Result<Outcome> {
    let n = bookx_editing::files::rename_files(book, map)?;
    let mut details: Vec<String> = map
        .iter()
        .filter(|(a, b)| a != b)
        .map(|(a, b)| format!("{a} is now {b}"))
        .collect();
    details.sort();
    Ok(outcome(
        !details.is_empty(),
        trf(
            "Renamed {} ({} updated)",
            &[&ntr("{} file", "{} files", details.len()), &n],
        ),
        details,
    ))
}

/// Deletes files from the book (CE-034). Protected package files are skipped and said so.
pub fn delete_paths(book: &mut OpenBook, paths: &[String]) -> Result<Outcome> {
    let done = bookx_editing::files::delete_files(book, paths)?;
    if done.is_empty() {
        return Ok(outcome(
            false,
            tr("These files are required and cannot be deleted"),
            vec![],
        ));
    }
    let broken = bookx_editing::links::find_broken_links(book).len();
    let mut summary = trf("Deleted {}", &[&ntr("{} file", "{} files", done.len())]);
    if broken > 0 {
        summary.push_str(&format!("; {broken} links now point nowhere (run Check)"));
    }
    let details = done.iter().map(|d| format!("{d} deleted")).collect();
    Ok(outcome(true, summary, details))
}

/// Merges the markup files into the first one (CE-027).
pub fn merge_many(book: &mut OpenBook, paths: &[String]) -> Result<Outcome> {
    let r = bookx_editing::split_merge::merge_files(book, paths)?;
    let mut details: Vec<String> = r
        .removed
        .iter()
        .map(|f| format!("{f} merged into {}", r.target))
        .collect();
    details.extend(
        r.renamed_ids
            .iter()
            .map(|(f, a, b)| format!("{f}: id {a} became {b}")),
    );
    Ok(outcome(
        true,
        trf("Merged {} files into {}", &[&paths.len(), &r.target]),
        details,
    ))
}

/// Puts the files read from disk into the book (CE-040).
pub fn add_files(
    book: &mut OpenBook,
    files: Vec<(String, Vec<u8>)>,
    after: Option<&str>,
) -> Result<Outcome> {
    let mut details = Vec::new();
    for (name, data) in files {
        let at = bookx_editing::files::add_file(book, &name, data, after)?;
        details.push(format!("{at} added"));
    }
    Ok(outcome(
        !details.is_empty(),
        trf("Added {}", &[&ntr("{} file", "{} files", details.len())]),
        details,
    ))
}

/// Creates a file from a template (CE-039).
pub fn new_file(
    book: &mut OpenBook,
    name: &str,
    template: bookx_editing::files::Template,
    after: Option<&str>,
    link_styles: bool,
) -> Result<Outcome> {
    let p = bookx_editing::files::create_from_template(book, name, template, after, link_styles)?;
    Ok(outcome(
        true,
        trf("Created {}", &[&p]),
        vec![format!("{p} created")],
    ))
}

/// Links stylesheets into the chosen pages (CE-041).
pub fn link_styles(
    book: &mut OpenBook,
    html: &[String],
    css: &[String],
    remove_existing: bool,
) -> Result<Outcome> {
    let n = bookx_editing::files::link_stylesheets(book, html, css, remove_existing)?;
    Ok(outcome(
        n > 0,
        if n > 0 {
            trf(
                "Stylesheets linked in {}",
                &[&ntr("{} file", "{} files", n)],
            )
        } else {
            tr("Those pages already use these stylesheets")
        },
        html.iter().map(|h| format!("{h} linked")).collect(),
    ))
}

/// Moves a file one place up or down in the reading order (CE-029).
pub fn move_in_spine(book: &mut OpenBook, path: &str, up: bool) -> Result<Outcome> {
    let id = book.package.item_by_path(path).map(|i| i.id.clone());
    let mut order = book.package.spine.clone();
    let Some(i) = id.and_then(|id| order.iter().position(|s| s.idref == id)) else {
        anyhow::bail!("Only files in the reading order can be moved");
    };
    let j = if up {
        i.checked_sub(1)
    } else {
        Some(i + 1).filter(|j| *j < order.len())
    };
    let Some(j) = j else {
        return Ok(outcome(false, tr("The file is already at the end"), vec![]));
    };
    order.swap(i, j);
    let pkg = book.package.clone();
    pkg.set_spine(&mut book.archive, &order)?;
    book.reload_package()?;
    Ok(outcome(
        true,
        trf(
            "{} is now number {} in the reading order",
            &[&path, &(j + 1)],
        ),
        vec![],
    ))
}

/// Reading-order position of every path (files outside the spine sort last, by name).
pub fn sort_by_reading_order(book: &OpenBook, paths: &mut [String]) {
    let spine = book.spine_paths();
    paths.sort_by_key(|p| {
        (
            spine.iter().position(|s| s == p).unwrap_or(usize::MAX),
            p.clone(),
        )
    });
}

/// Folders searched for Hunspell dictionaries: `$BOOKX_HUNSPELL`, else the usual system ones.
pub fn dictionary_dirs() -> Vec<std::path::PathBuf> {
    if let Some(d) = std::env::var_os("BOOKX_HUNSPELL") {
        return vec![d.into()];
    }
    let mut v = vec![
        "/usr/share/hunspell".into(),
        "/usr/share/myspell".into(),
        "/usr/share/myspell/dicts".into(),
    ];
    if let Some(h) = std::env::var_os("HOME") {
        v.push(std::path::Path::new(&h).join(".local/share/hunspell"));
    }
    v
}

/// The Hunspell `(aff, dic)` pair for a language tag: the exact tag (`en-GB` → `en_GB`), else the
/// language's main variant (`en_US`), else any variant, else the bare language.
pub fn find_dictionary(
    lang: &str,
    dirs: &[std::path::PathBuf],
) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let tag = lang.trim().replace('-', "_");
    let primary = tag.split('_').next().unwrap_or("").to_ascii_lowercase();
    if primary.is_empty() {
        return None;
    }
    let mut names = vec![
        tag.clone(),
        format!("{primary}_{}", primary.to_ascii_uppercase()),
        if primary == "en" {
            "en_US".into()
        } else {
            primary.clone()
        },
        primary.clone(),
    ];
    names.dedup();
    for dir in dirs {
        let mut candidates = names.clone();
        if let Ok(rd) = std::fs::read_dir(dir) {
            let mut others: Vec<String> = rd
                .flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .filter_map(|n| n.strip_suffix(".aff").map(str::to_string))
                .filter(|n| n.to_ascii_lowercase().starts_with(&format!("{primary}_")))
                .collect();
            others.sort();
            candidates.extend(others);
        }
        for n in candidates {
            let (aff, dic) = (dir.join(format!("{n}.aff")), dir.join(format!("{n}.dic")));
            if aff.is_file() && dic.is_file() {
                return Some((aff, dic));
            }
        }
    }
    None
}

/// Words the book's dictionary does not know, as issues (at most [`MAX_SPELLING`]). The second
/// value is a note when nothing could be checked or the list was cut.
pub fn spelling_issues(
    book: &OpenBook,
    dirs: &[std::path::PathBuf],
) -> (Vec<Issue>, Option<String>) {
    use bookx_editing::spell::{check_book_spelling, SpellChecker};
    let lang = book.package.metadata.language.clone();
    let lang = if lang.trim().is_empty() {
        "en".to_string()
    } else {
        lang
    };
    let Some((aff, dic)) = find_dictionary(&lang, dirs) else {
        return (
            vec![],
            Some(format!("No spelling dictionary for “{lang}” is installed")),
        );
    };
    let mut checker = SpellChecker::new();
    if let Err(e) = checker.add_dictionary_files(&lang, &aff, &dic) {
        return (
            vec![],
            Some(format!("Could not load the dictionary: {e:#}")),
        );
    }
    let report = check_book_spelling(book, &checker);
    let total = report.misspellings.len();
    let issues = report
        .misspellings
        .iter()
        .take(MAX_SPELLING)
        .map(|m| {
            let src = book.archive.get_string(&m.file).unwrap_or_default();
            let line = src
                .get(..m.start)
                .map(|h| h.matches('\n').count() as u32 + 1);
            let sug = checker.suggest(&m.lang, &m.word);
            let message = if sug.is_empty() {
                format!("“{}” is not in the dictionary", m.word)
            } else {
                format!(
                    "“{}”: maybe {}",
                    m.word,
                    sug.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
                )
            };
            Issue {
                severity: Severity::Warning,
                file: m.file.clone(),
                line,
                message,
            }
        })
        .collect();
    let note =
        (total > MAX_SPELLING).then(|| format!("Showing the first {MAX_SPELLING} of {total}"));
    (issues, note)
}

/// Longest list of misspellings shown.
pub const MAX_SPELLING: usize = 300;

/// The numbers of the *Book Report* dialog as `(heading, value)` rows.
pub fn report_rows(book: &OpenBook) -> Vec<(String, String)> {
    let r = build_report(book, &ReportOptions::default());
    let mut rows = vec![
        (
            "Files".to_string(),
            format!(
                "{} ({})",
                r.files.total_files,
                kib(r.files.total_bytes as usize)
            ),
        ),
        (
            "Words".to_string(),
            format!(
                "{} ({} different)",
                r.words.overall.total, r.words.overall.unique
            ),
        ),
        (
            "Characters".to_string(),
            format!("{} ({} different)", r.characters.total, r.characters.unique),
        ),
        (
            "Links".to_string(),
            format!(
                "{} inside the book, {} to the web, {} broken",
                r.links.internal,
                r.links.external,
                r.links.broken.len()
            ),
        ),
        (
            "Images".to_string(),
            format!(
                "{} ({}, about {} could be saved)",
                r.images.images.len(),
                kib(r.images.total_bytes as usize),
                kib(r.images.total_potential_savings as usize)
            ),
        ),
        (
            tr("Style rules"),
            format!("{} ({} unused)", r.css.total_rules, r.css.unused_rules),
        ),
    ];
    if !r.links.missing_resources.is_empty() {
        rows.push((
            tr("Missing resources"),
            r.links.missing_resources.len().to_string(),
        ));
    }
    if !r.css.undefined_classes.is_empty() {
        rows.push((
            tr("Classes without a style"),
            r.css.undefined_classes.len().to_string(),
        ));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::convert::{create_epub, Chapter};
    use crate::core::opf::Metadata;

    fn book() -> OpenBook {
        let md = Metadata {
            title: "Tool Book".into(),
            authors: vec!["A".into()],
            language: "en".into(),
            ..Default::default()
        };
        let ch = |t: &str, b: &str| Chapter {
            title: t.into(),
            body: b.into(),
        };
        let a = create_epub(
            &md,
            &[
                ch("One", "<h1>One</h1><p>First words here.</p>"),
                ch(
                    "Two",
                    "<h1>Two</h1><p>More words <a href=\"chapter-1.xhtml\">back</a>.</p>",
                ),
            ],
            None,
        );
        OpenBook::from_archive(a).unwrap()
    }

    fn add_png(book: &mut OpenBook, with_alt: bool) {
        let png = bookx_sources::cover::generate_cover("T", "A", "", 300, 400).unwrap();
        book.archive.set("OEBPS/pic.png", png);
        let pkg = book.package.clone();
        pkg.add_manifest_item(&mut book.archive, "pic", "OEBPS/pic.png", "image/png", "")
            .unwrap();
        book.reload_package().unwrap();
        let ch = book.spine_paths()[0].clone();
        let src = book.archive.get_string(&ch).unwrap();
        let alt = if with_alt { " alt=\"a picture\"" } else { "" };
        book.archive.set_string(
            &ch,
            &src.replace(
                "</body>",
                &format!("<p><img src=\"pic.png\"{alt}/></p></body>"),
            ),
        );
    }

    #[test]
    fn every_tool_has_a_unique_id_label_and_round_trips() {
        let mut ids: Vec<_> = Tool::ALL.iter().map(|t| t.id()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), Tool::ALL.len());
        for t in Tool::ALL {
            assert_eq!(Tool::from_id(t.id()), Some(t));
            assert!(!t.label().is_empty());
        }
        assert_eq!(Tool::from_id("nope"), None);
    }

    #[test]
    fn fixing_a_clean_book_changes_nothing_and_a_broken_one_is_repaired_once() {
        let mut b = book();
        let o = run(Tool::FixProblems, &mut b).unwrap();
        assert!(!o.changed, "{o:?}");
        assert_eq!(o.summary, "Nothing to fix");
        // an image without alt text and a file that is not in the manifest
        add_png(&mut b, false);
        b.archive.set("OEBPS/stray.css", b"p{}".to_vec());
        let o = run(Tool::FixProblems, &mut b).unwrap();
        assert!(o.changed);
        assert!(o.summary.starts_with("Fixed "), "{}", o.summary);
        assert!(!o.details.is_empty());
        assert!(b.package.item_by_path("OEBPS/stray.css").is_some());
        // running it again finds nothing more
        let again = run(Tool::FixProblems, &mut b).unwrap();
        assert!(!again.changed, "{again:?}");
    }

    #[test]
    fn upgrading_is_reported_once() {
        let mut b = book();
        let first = run(Tool::UpgradeToEpub3, &mut b).unwrap();
        assert!(b.package.is_epub3());
        let second = run(Tool::UpgradeToEpub3, &mut b).unwrap();
        assert!(!second.changed, "{second:?} (first: {first:?})");
        assert_eq!(second.summary, "Already EPUB 3");
    }

    #[test]
    fn the_contents_page_can_be_added_regenerated_and_removed() {
        let mut b = book();
        let o = run(Tool::AddInlineToc, &mut b).unwrap();
        assert!(o.changed && o.details[0].contains("written to"), "{o:?}");
        let toc_file = b
            .file_list()
            .into_iter()
            .find(|f| f.contains("toc") && f.ends_with("xhtml") && !f.contains("nav"));
        assert!(toc_file.is_some(), "{:?}", b.file_list());
        // inserting again does not duplicate the page
        run(Tool::AddInlineToc, &mut b).unwrap();
        let pages = b
            .file_list()
            .into_iter()
            .filter(|f| f.contains("inline") || f.ends_with("contents.xhtml"))
            .count();
        assert!(pages <= 1, "{pages} contents pages");
        let removed = run(Tool::RemoveInlineToc, &mut b).unwrap();
        assert!(removed.changed);
        let none = run(Tool::RemoveInlineToc, &mut b).unwrap();
        assert!(!none.changed);
        assert_eq!(none.summary, "There is no contents page to remove");
    }

    #[test]
    fn accessibility_adds_metadata_and_reports_missing_descriptions() {
        let mut b = book();
        add_png(&mut b, false);
        let o = run(Tool::MakeAccessible, &mut b).unwrap();
        assert!(o.changed, "{o:?}");
        assert!(
            o.summary.contains("still need a description"),
            "{}",
            o.summary
        );
        let opf = b.archive.get_string(&b.package.opf_path).unwrap();
        assert!(opf.contains("accessib"), "no accessibility metadata");
        // a second run changes nothing
        let again = run(Tool::MakeAccessible, &mut b).unwrap();
        assert!(!again.changed, "{again:?}");
    }

    #[test]
    fn images_are_optimised_without_losing_them() {
        let mut b = book();
        add_png(&mut b, true);
        let before = b.archive.get("OEBPS/pic.png").unwrap().len();
        let o = run(Tool::CompressImages, &mut b).unwrap();
        let after = b.archive.get("OEBPS/pic.png").unwrap().len();
        assert!(after <= before, "{after} > {before}");
        if o.changed {
            assert!(o.summary.starts_with("Optimised"), "{}", o.summary);
            assert!(o.details[0].contains("pic.png"));
        } else {
            assert_eq!(after, before);
        }
        // the picture still decodes
        assert!(bookx_editing::images::decode(b.archive.get("OEBPS/pic.png").unwrap()).is_ok());
    }

    #[test]
    fn a_book_without_fonts_has_nothing_to_shrink() {
        let mut b = book();
        let o = run(Tool::SubsetFonts, &mut b).unwrap();
        assert!(!o.changed);
        assert_eq!(o.summary, "No embedded fonts to shrink");
    }

    #[test]
    fn files_are_arranged_into_folders_and_stay_valid() {
        let mut b = book();
        add_png(&mut b, true);
        let text_before = b.chapter_doc(&b.spine_paths()[0]).unwrap().to_text();
        let o = run(Tool::ArrangeFiles, &mut b).unwrap();
        assert!(o.changed, "{o:?}");
        assert!(
            b.file_list()
                .iter()
                .any(|f| f.contains("Images/") && f.ends_with("pic.png")),
            "{:?}",
            b.file_list()
        );
        // the text and the picture reference survive
        let ch = b.spine_paths()[0].clone();
        assert_eq!(b.chapter_doc(&ch).unwrap().to_text(), text_before);
        let src = b.archive.get_string(&ch).unwrap();
        assert!(src.contains("Images/pic.png"), "{src}");
        assert!(crate::core::check::check_book(&b.archive)
            .iter()
            .all(|i| i.severity != crate::core::check::Severity::Error));
        let again = run(Tool::ArrangeFiles, &mut b).unwrap();
        assert!(!again.changed, "{again:?}");
    }

    #[test]
    fn the_report_rows_describe_the_book() {
        let mut b = book();
        add_png(&mut b, true);
        let rows = report_rows(&b);
        let get = |k: &str| rows.iter().find(|(h, _)| h == k).map(|(_, v)| v.clone());
        assert!(get("Files").unwrap().contains("files") || get("Files").unwrap().contains('('));
        assert!(get("Words").unwrap().contains("different"));
        assert!(
            get("Images").unwrap().starts_with("1 "),
            "{:?}",
            get("Images")
        );
        assert!(
            get("Links").unwrap().contains("0 broken"),
            "{:?}",
            get("Links")
        );
        for k in ["Characters", "Style rules"] {
            assert!(get(k).is_some(), "{k}");
        }
    }

    #[test]
    fn a_broken_link_shows_up_in_the_report() {
        let mut b = book();
        let ch = b.spine_paths()[1].clone();
        let src = b.archive.get_string(&ch).unwrap();
        b.archive
            .set_string(&ch, &src.replace("chapter-1.xhtml", "gone.xhtml"));
        let links = report_rows(&b)
            .into_iter()
            .find(|(h, _)| h == "Links")
            .unwrap()
            .1;
        assert!(links.contains("1 broken"), "{links}");
    }

    #[test]
    fn a_fresh_book_has_no_findings_at_all() {
        let b = book();
        let i = check_all(&b.archive);
        // only the note that there is no cover; nothing from the validator
        assert!(
            i.iter()
                .all(|x| x.severity == Severity::Info && !x.message.starts_with('[')),
            "{i:?}"
        );
    }

    #[test]
    fn check_all_adds_validator_findings_with_their_ids() {
        let mut b = book();
        assert!(check_all(&b.archive)
            .iter()
            .all(|i| i.severity != Severity::Error));
        b.archive.set("mimetype", b"wrong".to_vec());
        let issues = check_all(&b.archive);
        assert!(
            issues.iter().any(|i| i.message.starts_with('[')),
            "{issues:?}"
        );
        assert!(issues.iter().any(|i| i.severity == Severity::Error));
    }

    fn dict_dir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-spell-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("en_US.aff"), "SET UTF-8\n").unwrap();
        std::fs::write(
            d.join("en_US.dic"),
            "6\nfirst\nwords\nhere\nMore\nback\nTool\n",
        )
        .unwrap();
        std::fs::write(d.join("en_GB.aff"), "SET UTF-8\n").unwrap();
        std::fs::write(d.join("en_GB.dic"), "1\nfirst\n").unwrap();
        d
    }

    #[test]
    fn dictionaries_are_found_by_exact_tag_then_main_variant_then_any() {
        let d = dict_dir();
        let dirs = vec![d.clone()];
        let name = |l: &str| {
            find_dictionary(l, &dirs)
                .map(|(a, _)| a.file_name().unwrap().to_str().unwrap().to_string())
        };
        assert_eq!(name("en-GB").as_deref(), Some("en_GB.aff"));
        assert_eq!(name("en").as_deref(), Some("en_US.aff"));
        assert_eq!(name("en-AU").as_deref(), Some("en_US.aff"));
        assert_eq!(name("fr"), None);
        assert_eq!(name(""), None);
        std::fs::remove_file(d.join("en_US.dic")).unwrap();
        assert_eq!(
            name("en").as_deref(),
            Some("en_GB.aff"),
            "a variant without .dic is skipped"
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn spelling_reports_unknown_words_with_lines_and_suggestions() {
        let d = dict_dir();
        let mut b = book();
        let ch = b.spine_paths()[0].clone();
        let src = b.archive.get_string(&ch).unwrap();
        b.archive
            .set_string(&ch, &src.replace("First words here.", "First wrods here."));
        let (issues, note) = spelling_issues(&b, std::slice::from_ref(&d));
        assert!(note.is_none(), "{note:?}");
        let hit = issues
            .iter()
            .find(|i| i.message.contains("wrods"))
            .expect("wrods");
        assert_eq!(hit.file, ch);
        assert!(hit.line.unwrap() >= 1);
        assert!(!issues.iter().any(|i| i.message.contains("“here”")));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn spelling_without_a_dictionary_says_so() {
        let b = book();
        let (issues, note) = spelling_issues(&b, &[std::env::temp_dir().join("bookx-no-such-dir")]);
        assert!(issues.is_empty());
        assert!(note.unwrap().contains("No spelling dictionary"));
    }

    #[test]
    fn a_file_is_split_at_headings_and_merged_again() {
        let mut b = book();
        let ch = b.spine_paths()[0].clone();
        let src = b.archive.get_string(&ch).unwrap();
        b.archive.set_string(
            &ch,
            &src.replace(
                "</body>",
                "<h1>Part Two</h1><p>Second part text.</p></body>",
            ),
        );
        let before = b.spine_paths().len();
        let o = split_at_headings(&mut b, &ch, 1).unwrap();
        assert!(o.changed, "{o:?}");
        assert_eq!(b.spine_paths().len(), before + 1);
        let words: String = b
            .spine_paths()
            .iter()
            .filter_map(|p| b.chapter_doc(p))
            .map(|d| d.to_text())
            .collect();
        assert!(words.contains("Second part text") && words.contains("First words"));
        let first = b.spine_paths()[0].clone();
        assert!(!split_at_headings(&mut b, &first, 1).unwrap().changed);
        let m = merge_with_next(&mut b, &first).unwrap();
        assert!(m.changed, "{m:?}");
        assert_eq!(b.spine_paths().len(), before);
        let words2: String = b
            .spine_paths()
            .iter()
            .filter_map(|p| b.chapter_doc(p))
            .map(|d| d.to_text())
            .collect();
        assert!(words2.contains("Second part text"));
    }

    #[test]
    fn merging_the_last_file_or_a_file_outside_the_spine_is_refused_politely() {
        let mut b = book();
        let last = b.spine_paths().last().unwrap().clone();
        let o = merge_with_next(&mut b, &last).unwrap();
        assert!(!o.changed && o.summary.contains("last file"));
        assert!(merge_with_next(&mut b, "OEBPS/style.css").is_err());
    }

    #[test]
    fn renaming_through_the_tool_keeps_longer_names_and_urls_intact() {
        let mut b = book();
        add_png(&mut b, true);
        b.archive.set("OEBPS/data.png", vec![1, 2, 3]);
        let pkg = b.package.clone();
        pkg.add_manifest_item(&mut b.archive, "data", "OEBPS/data.png", "image/png", "")
            .unwrap();
        b.reload_package().unwrap();
        b.archive.set_string(
            "OEBPS/style.css",
            "p { background: url(pic.png) } q { background: url(data.png) }",
        );
        let ch = b.spine_paths()[0].clone();
        let src = b.archive.get_string(&ch).unwrap();
        b.archive.set_string(
            &ch,
            &src.replace("</body>", "<img src=\"data.png\"/></body>"),
        );
        let o = rename_one(&mut b, "OEBPS/pic.png", "cover-art.png").unwrap();
        assert!(o.changed);
        let src = b.archive.get_string(&ch).unwrap();
        assert!(src.contains("src=\"cover-art.png\"") && src.contains("src=\"data.png\""));
        assert!(b
            .archive
            .get_string("OEBPS/style.css")
            .unwrap()
            .contains("url(cover-art.png)"));
        assert!(b
            .archive
            .get_string("OEBPS/style.css")
            .unwrap()
            .contains("url(data.png)"));
        assert!(rename_one(&mut b, "OEBPS/data.png", "a/b.png").is_err());
        assert!(
            !rename_one(&mut b, "OEBPS/data.png", "data.png")
                .unwrap()
                .changed
        );
        assert!(rename_one(&mut b, "OEBPS/data.png", "cover-art.png").is_err());
    }

    #[test]
    fn many_files_can_be_merged_renamed_deleted_added_and_created() {
        let mut b = book();
        let spine = b.spine_paths();
        let o = merge_many(&mut b, &spine).unwrap();
        assert!(o.changed && o.summary.starts_with("Merged 2 files"));
        assert_eq!(b.spine_paths().len(), 1);
        let only = b.spine_paths();
        assert!(merge_many(&mut b, &only).is_err());

        let o = add_files(
            &mut b,
            vec![
                ("one.png".into(), vec![1]),
                ("t.css".into(), b"p{}".to_vec()),
            ],
            None,
        )
        .unwrap();
        assert!(o.summary.contains("Added 2 files"));
        assert!(b
            .package
            .manifest
            .iter()
            .any(|i| i.href.ends_with("one.png")));

        let o = new_file(
            &mut b,
            "fresh",
            bookx_editing::files::Template::Chapter,
            None,
            true,
        )
        .unwrap();
        assert!(o.changed);
        assert_eq!(b.spine_paths().len(), 2);
        let fresh = b.spine_paths()[1].clone();
        assert!(b.archive.get_string(&fresh).unwrap().contains("t.css"));

        let first_only = b.spine_paths()[..1].to_vec();
        let css = b
            .archive
            .names()
            .iter()
            .find(|n| n.ends_with("t.css"))
            .cloned()
            .unwrap();
        let o = link_styles(&mut b, &first_only, std::slice::from_ref(&css), false).unwrap();
        assert!(o.changed);
        assert!(
            !link_styles(&mut b, &first_only, &[css], false)
                .unwrap()
                .changed
        );

        let plan = bookx_editing::files::bulk_rename_plan(&b.spine_paths(), "p", 1);
        assert!(rename_many(&mut b, &plan).unwrap().changed);
        assert!(b.spine_paths()[0].ends_with("p1.xhtml"));

        let png = b
            .archive
            .names()
            .iter()
            .find(|n| n.ends_with("one.png"))
            .cloned()
            .unwrap();
        let o = delete_paths(&mut b, std::slice::from_ref(&png)).unwrap();
        assert!(o.changed && !b.archive.contains(&png));
        let opf = b.package.opf_path.clone();
        assert!(!delete_paths(&mut b, &[opf]).unwrap().changed);
    }

    #[test]
    fn files_move_in_the_reading_order_and_sort_by_it() {
        let mut b = book();
        let (a, c) = (b.spine_paths()[0].clone(), b.spine_paths()[1].clone());
        assert!(!move_in_spine(&mut b, &a, true).unwrap().changed);
        assert!(move_in_spine(&mut b, &a, false).unwrap().changed);
        assert_eq!(b.spine_paths(), vec![c.clone(), a.clone()]);
        assert!(!move_in_spine(&mut b, &a, false).unwrap().changed);
        assert!(move_in_spine(&mut b, "OEBPS/style.css", true).is_err());
        let mut v = vec!["z.css".to_string(), a.clone(), c.clone()];
        sort_by_reading_order(&b, &mut v);
        assert_eq!(v, vec![c, a, "z.css".to_string()]);
    }

    #[test]
    fn kib_formats_sizes() {
        assert_eq!(kib(0), "0 KB");
        assert_eq!(kib(1), "1 KB");
        assert_eq!(kib(1024), "1 KB");
        assert_eq!(kib(1025), "2 KB");
        assert_eq!(kib(3 * 1024 * 1024), "3.0 MB");
    }
}
