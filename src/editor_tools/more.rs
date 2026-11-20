//! More tools of the editor's Tools menu and file browser (roadmap E1-E25, gap analysis CE-020..CE-046 and
//! CE-130..CE-146): each function runs on an open book, says what it did as an [`Outcome`], and has no GTK in it.
//! The dialogs in `ui/editor/tools_ui.rs` call them for the preview (on a copy of the book) and for the real run.

use crate::i18n::tr;
use crate::i18n::trf;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Result};
use bookx_core::book::OpenBook;
use bookx_core::util::{extension, file_name};
use bookx_editing::book_tools::{self, AccessMeta, PageRole, Punctuation};
use bookx_editing::filter_styles::{self, FilterSpec};
use bookx_editing::fixes;
use bookx_editing::fonts::{self, FaceKey, SystemFont};
use bookx_editing::links::{self, LinkChecker, LinkStatus};
use bookx_editing::semantics::{self, Semantic};
use bookx_editing::transform_css::{self, CssTransform};
use bookx_sources::fetch::{FetchError, Fetcher, MockFetcher, MockReply, Throttle, UreqFetcher};

use super::{kib, outcome, plural, Outcome};

fn files_word(n: usize) -> String {
    plural(n, "file", "files")
}

fn changed_details(paths: &[String]) -> Vec<String> {
    paths.iter().map(|p| format!("{p} changed")).collect()
}

// ---------------------------------------------------------------------------
// text
// ---------------------------------------------------------------------------

/// Smarten punctuation (CE-132). `paths` empty = every page.
pub fn smarten(
    book: &mut OpenBook,
    paths: &[String],
    mode: Punctuation,
    style: book_tools::QuoteStyle,
) -> Result<Outcome> {
    if let Punctuation::Smarten(o) = mode {
        if !(o.quotes || o.dashes || o.ellipses) {
            bail!("Choose at least one kind of punctuation");
        }
    }
    let changed = book_tools::punctuation_styled(book, paths, mode, style);
    let what = match mode {
        Punctuation::Smarten(_) => "Smartened the punctuation in",
        Punctuation::Unsmarten => "Straightened the punctuation in",
    };
    Ok(if changed.is_empty() {
        outcome(false, tr("Nothing to change"), vec![])
    } else {
        outcome(
            true,
            format!("{what} {}", files_word(changed.len())),
            changed_details(&changed),
        )
    })
}

/// Fix HTML (CE-136): repairs the markup of the given pages (all when empty), with the HTML5 parser as the fallback.
pub fn fix_html(book: &mut OpenBook, paths: &[String], html5_only: bool) -> Result<Outcome> {
    let (fixes, failures) = fixes::fix_html_files(book, paths, html5_only);
    let mut details: Vec<String> = fixes
        .iter()
        .map(|f| format!("{}: {}", f.file, f.description))
        .collect();
    details.extend(failures.iter().map(|f| format!("could not repair: {f}")));
    let summary = if fixes.is_empty() {
        if failures.is_empty() {
            tr("The markup is already well-formed")
        } else {
            format!("{} could not be repaired", files_word(failures.len()))
        }
    } else {
        format!("Repaired {}", files_word(fixes.len()))
    };
    Ok(outcome(!fixes.is_empty(), summary, details))
}

/// Beautify (CE-137): tidies pages, stylesheets and XML files; `paths` empty = every text file of the book.
pub fn beautify(book: &mut OpenBook, paths: &[String]) -> Result<Outcome> {
    let targets: Vec<String> = if paths.is_empty() {
        book.archive
            .names()
            .iter()
            .filter(|n| {
                matches!(
                    extension(n).as_str(),
                    "xhtml" | "html" | "htm" | "xht" | "css" | "opf" | "ncx"
                )
            })
            .cloned()
            .collect()
    } else {
        paths.to_vec()
    };
    let before: HashMap<String, String> = targets
        .iter()
        .filter_map(|p| Some((p.clone(), book.archive.get_string(p)?)))
        .collect();
    let changed = book_tools::beautify_files(book, &targets);
    let details: Vec<String> = changed
        .iter()
        .map(|p| {
            let (a, d) = book_tools::line_diff_counts(
                before.get(p).map(String::as_str).unwrap_or(""),
                &book.archive.get_string(p).unwrap_or_default(),
            );
            format!("{p}: +{a} −{d} lines")
        })
        .collect();
    Ok(if changed.is_empty() {
        outcome(false, tr("Already nicely laid out"), vec![])
    } else {
        outcome(
            true,
            trf("Tidied {}", &[&files_word(changed.len())]),
            details,
        )
    })
}

// ---------------------------------------------------------------------------
// styles
// ---------------------------------------------------------------------------

/// Transform styles (CE-133).
pub fn transform_styles(book: &mut OpenBook, rules: &[CssTransform]) -> Result<Outcome> {
    if rules.is_empty() {
        bail!("Add at least one rule");
    }
    let r = transform_css::transform_book(book, rules);
    let details: Vec<String> = r
        .files
        .iter()
        .map(|(f, n)| format!("{f}: {}", plural(*n, "change", "changes")))
        .collect();
    Ok(if r.total_changes == 0 {
        outcome(false, tr("No style matched the rules"), vec![])
    } else {
        outcome(
            true,
            trf(
                "Changed {} in {}",
                &[
                    &plural(r.total_changes, "declaration", "declarations"),
                    &files_word(r.files.len()),
                ],
            ),
            details,
        )
    })
}

/// Remove unused CSS rules (CE-134).
pub fn remove_unused_css(book: &mut OpenBook) -> Result<Outcome> {
    let r = book_tools::remove_unused_css(book);
    let saved: usize = r.iter().map(|(_, a, b)| a - b).sum();
    let details = r
        .iter()
        .map(|(f, a, b)| format!("{f}: {a} → {b} bytes"))
        .collect();
    Ok(if r.is_empty() {
        outcome(false, tr("Every style rule is used"), vec![])
    } else {
        outcome(
            true,
            trf(
                "Removed unused rules from {} ({} smaller)",
                &[&files_word(r.len()), &kib(saved)],
            ),
            details,
        )
    })
}

/// Filter style information (CE-140).
pub fn filter_styles(book: &mut OpenBook, spec: &FilterSpec) -> Result<Outcome> {
    if spec.properties.is_empty() {
        bail!("Choose at least one property to remove");
    }
    let r = filter_styles::filter_styles(book, spec);
    let mut details: Vec<String> = r
        .files
        .iter()
        .map(|(f, n)| format!("{f}: {} removed", plural(*n, "declaration", "declarations")))
        .collect();
    if r.empty_rules_removed > 0 {
        details.push(format!(
            "{} left empty and removed",
            plural(r.empty_rules_removed, "rule", "rules")
        ));
    }
    Ok(if r.declarations_removed == 0 {
        outcome(false, tr("None of those properties is used"), vec![])
    } else {
        outcome(
            true,
            trf(
                "Removed {} from {}",
                &[
                    &plural(r.declarations_removed, "declaration", "declarations"),
                    &files_word(r.files.len()),
                ],
            ),
            details,
        )
    })
}

// ---------------------------------------------------------------------------
// media
// ---------------------------------------------------------------------------

/// Delete unused media (CE-135): removes exactly the listed files.
pub fn delete_unused_media(book: &mut OpenBook, paths: &[String]) -> Result<Outcome> {
    if paths.is_empty() {
        return Ok(outcome(false, tr("No unused media to delete"), vec![]));
    }
    let size: usize = paths.iter().map(|p| book.archive.size_of(p)).sum();
    let gone = book_tools::remove_media(book, paths)?;
    Ok(outcome(
        !gone.is_empty(),
        trf("Deleted {} ({})", &[&files_word(gone.len()), &kib(size)]),
        gone.iter().map(|p| format!("{p} deleted")).collect(),
    ))
}

/// What embedding a font would do, for the dialog.
#[derive(Debug, Clone, PartialEq)]
pub struct FontCandidate {
    pub key: FaceKey,
    pub chars: usize,
    pub font: Option<SystemFont>,
}

/// The faces the book uses but does not carry, each with the installed font that would be embedded.
pub fn font_candidates(book: &OpenBook, installed: &[SystemFont]) -> Vec<FontCandidate> {
    fonts::missing_faces(book)
        .into_iter()
        .map(|m| FontCandidate {
            font: fonts::best_system_font(installed, &m.key.family, m.key.weight, m.key.italic)
                .cloned(),
            chars: m.chars,
            key: m.key,
        })
        .collect()
}

/// Embed referenced fonts (CE-130): only the families in `chosen` (lower case), and never a font whose licence
/// forbids embedding.
pub fn embed_fonts(
    book: &mut OpenBook,
    installed: &[SystemFont],
    chosen: &HashSet<String>,
    subset: bool,
) -> Result<Outcome> {
    if chosen.is_empty() {
        bail!("Choose at least one font to embed");
    }
    let lookup = |family: &str, weight: u16, italic: bool| -> Option<Vec<u8>> {
        if !chosen.contains(&family.to_ascii_lowercase()) {
            return None;
        }
        let f = fonts::best_system_font(installed, family, weight, italic)?;
        if f.embedding == fonts::Embedding::Restricted {
            return None;
        }
        std::fs::read(&f.path).ok()
    };
    let r = fonts::embed_referenced_fonts(book, &lookup, subset)?;
    let bytes: usize = r.iter().map(|e| e.bytes).sum();
    Ok(if r.is_empty() {
        outcome(false, tr("No font could be embedded"), vec![])
    } else {
        outcome(
            true,
            trf(
                "Embedded {} ({})",
                &[&plural(r.len(), "font", "fonts"), &kib(bytes)],
            ),
            r.iter()
                .map(|e| {
                    format!(
                        "{} {}{} → {}",
                        e.family,
                        e.weight,
                        if e.italic { " italic" } else { "" },
                        e.path
                    )
                })
                .collect(),
        )
    })
}

// ---------------------------------------------------------------------------
// semantics, roles, spine
// ---------------------------------------------------------------------------

/// Set semantics (CE-138): `wanted` gives the target (`file#anchor`, an archive path) of each kind, or `None` to
/// clear it. Kinds not mentioned stay as they are.
pub fn set_semantics(
    book: &mut OpenBook,
    wanted: &[(Semantic, Option<String>)],
) -> Result<Outcome> {
    let mut details = Vec::new();
    for (sem, target) in wanted {
        let current = semantics::read_semantics(book)
            .into_iter()
            .find(|e| e.semantic == *sem)
            .map(|e| e.href);
        match target {
            Some(href) => {
                let path = href.split('#').next().unwrap_or("");
                if !book.archive.contains(path) {
                    bail!("{path} is not in the book");
                }
                if current.as_deref() == Some(href.as_str()) {
                    continue;
                }
                semantics::set_semantic(book, *sem, href, "")?;
                details.push(format!("{} → {href}", sem.label()));
            }
            None if current.is_some() => {
                semantics::clear_semantic(book, *sem)?;
                details.push(format!("{} cleared", sem.label()));
            }
            None => {}
        }
    }
    Ok(if details.is_empty() {
        outcome(false, tr("The semantics are unchanged"), vec![])
    } else {
        outcome(
            true,
            trf(
                "Updated {}",
                &[&plural(details.len(), "semantic", "semantics")],
            ),
            details,
        )
    })
}

/// Mark a page as cover page, title page or table of contents (CE-032, CE-033).
pub fn mark_role(book: &mut OpenBook, path: &str, role: PageRole) -> Result<Outcome> {
    let hint = book_tools::mark_page(book, path, role)?;
    let name = role.semantic().label();
    let mut details = vec![format!("{path} is now the {name}")];
    details.extend(hint);
    Ok(outcome(
        true,
        trf("{} is the {}", &[&file_name(path), &name]),
        details,
    ))
}

/// Put a spine file at a position (drag and drop, CE-029).
pub fn spine_drop(book: &mut OpenBook, path: &str, to: usize) -> Result<Outcome> {
    Ok(if book_tools::spine_move_to(book, path, to)? {
        outcome(
            true,
            trf(
                "{} is now number {} in the reading order",
                &[&file_name(path), &(to + 1)],
            ),
            vec![],
        )
    } else {
        outcome(false, tr("The reading order is unchanged"), vec![])
    })
}

/// Flip a file between the main reading order and "not part of the main reading order" (CE-030).
pub fn toggle_nonlinear(book: &mut OpenBook, path: &str) -> Result<Outcome> {
    let linear_now = book
        .package
        .item_by_path(path)
        .map(|i| {
            book.package
                .spine
                .iter()
                .filter(|s| s.idref == i.id)
                .all(|s| s.linear)
        })
        .unwrap_or(true);
    book_tools::set_linear(book, path, !linear_now)?;
    Ok(outcome(
        true,
        if linear_now {
            format!(
                "{} is no longer part of the main reading order",
                file_name(path)
            )
        } else {
            format!(
                "{} is part of the main reading order again",
                file_name(path)
            )
        },
        vec![],
    ))
}

/// Change the extension of the given files (CE-025); references follow.
pub fn change_extension(book: &mut OpenBook, paths: &[String], ext: &str) -> Result<Outcome> {
    let ext = ext.trim().trim_start_matches('.');
    if ext.is_empty() || ext.contains(['/', ' ']) {
        bail!("Enter an extension such as xhtml");
    }
    let plan = bookx_editing::files::change_extension_plan(paths, ext);
    if plan.is_empty() {
        return Ok(outcome(false, tr("The extension is unchanged"), vec![]));
    }
    let n = bookx_editing::files::rename_files(book, &plan)?;
    let mut details: Vec<String> = plan.iter().map(|(a, b)| format!("{a} → {b}")).collect();
    details.sort();
    Ok(outcome(
        true,
        trf(
            "Renamed {}; {} updated",
            &[&files_word(plan.len()), &files_word(n)],
        ),
        details,
    ))
}

/// Replace a file with other content, renaming it (and its links) when the name differs (CE-036).
pub fn replace_with(
    book: &mut OpenBook,
    path: &str,
    new_name: &str,
    data: Vec<u8>,
) -> Result<Outcome> {
    let bytes = data.len();
    let final_path = book_tools::replace_file(book, path, new_name, data)?;
    Ok(outcome(
        true,
        trf("Replaced {} ({})", &[&file_name(path), &kib(bytes)]),
        vec![if final_path == path {
            format!("{path} replaced")
        } else {
            format!("{path} replaced and renamed to {final_path}")
        }],
    ))
}

/// Move a file to another folder, with link update (CE-146). `folder` is relative to the package folder.
pub fn move_to_folder(book: &mut OpenBook, path: &str, folder: &str) -> Result<Outcome> {
    let to = links::destination_in_folder(book, path, folder);
    if to == path {
        return Ok(outcome(false, tr("The file is already there"), vec![]));
    }
    let n = links::move_file(book, path, &to)?;
    Ok(outcome(
        true,
        trf(
            "Moved {} to {}",
            &[&file_name(path), &bookx_core::util::parent_dir(&to)],
        ),
        vec![format!("{path} → {to} ({} updated)", files_word(n))],
    ))
}

/// Repair broken internal links (CE-145).
pub fn repair_links(book: &mut OpenBook) -> Result<Outcome> {
    let (repairs, left) = links::repair_broken_links(book);
    let mut details: Vec<String> = repairs
        .iter()
        .map(|r| format!("{}: {} → {} ({})", r.file, r.old, r.new, r.how))
        .collect();
    details.extend(
        left.iter()
            .map(|b| format!("still broken: {} in {} ({})", b.href, b.file, b.reason)),
    );
    let summary = match (repairs.len(), left.len()) {
        (0, 0) => tr("No broken links"),
        (0, n) => format!(
            "{} cannot be repaired automatically",
            plural(n, "link", "links")
        ),
        (r, 0) => format!("Repaired {}", plural(r, "link", "links")),
        (r, n) => format!("Repaired {}; {n} left", plural(r, "link", "links")),
    };
    Ok(outcome(!repairs.is_empty(), summary, details))
}

// ---------------------------------------------------------------------------
// accessibility
// ---------------------------------------------------------------------------

/// Everything the accessibility dialog can change at once.
#[derive(Debug, Clone, Default)]
pub struct AccessEdits {
    pub language: Option<String>,
    /// `(file, image index, text)`; empty text = decorative.
    pub alts: Vec<(String, usize, String)>,
    /// Give every use of the same image file the text (instead of only that one).
    pub all_uses: bool,
    pub meta: Option<AccessMeta>,
}

pub fn accessibility(book: &mut OpenBook, e: &AccessEdits) -> Result<Outcome> {
    let mut details = Vec::new();
    if let Some(lang) = &e.language {
        if !lang.trim().is_empty() && lang.trim() != book.metadata().language {
            let n = book_tools::set_content_language(book, lang)?;
            details.push(format!(
                "language set to {} ({})",
                lang.trim(),
                plural(n, "page", "pages")
            ));
        }
    }
    let mut alts = 0;
    for (file, index, text) in &e.alts {
        let uses = book_tools::image_uses(book);
        let Some(u) = uses
            .iter()
            .find(|u| &u.file == file && u.index == *index)
            .cloned()
        else {
            continue;
        };
        if u.alt.as_deref() == Some(text.as_str()) {
            continue;
        }
        alts += if e.all_uses {
            book_tools::set_alt_for_source(book, &u, text)?
        } else {
            usize::from(book_tools::set_alt_text(book, file, *index, text)?)
        };
    }
    if alts > 0 {
        details.push(format!("{} described", plural(alts, "image", "images")));
    }
    if let Some(m) = &e.meta {
        if book_tools::read_access_meta(book) != *m && book_tools::write_access_meta(book, m)? {
            details.push("accessibility metadata updated".to_string());
        }
    }
    Ok(if details.is_empty() {
        outcome(false, tr("Nothing changed"), vec![])
    } else {
        outcome(true, tr("Accessibility updated"), details)
    })
}

// ---------------------------------------------------------------------------
// the network: external links and resources
// ---------------------------------------------------------------------------

/// The fetcher for the editor's network tools: the real network, or canned answers from the folder in
/// `$BOOKX_EDITOR_NET_MOCK` for the UI tests (`https://mock.example/<name>` answers with the file `<name>` of that
/// folder, everything else 404; `https://down.example/…` times out).
pub fn net_fetcher() -> Arc<dyn Fetcher> {
    match std::env::var_os("BOOKX_EDITOR_NET_MOCK") {
        Some(dir) => {
            let dir = std::path::PathBuf::from(dir);
            let mut m = MockFetcher::new();
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.flatten() {
                    if let (Some(name), Ok(body)) =
                        (e.file_name().to_str(), std::fs::read(e.path()))
                    {
                        m = m.on(
                            &format!("https://mock.example/{name}"),
                            MockReply::Body(body),
                        );
                    }
                }
            }
            Arc::new(
                m.on_prefix("https://down.example/", MockReply::Timeout)
                    .on_prefix("https://mock.example/", MockReply::Status(404)),
            )
        }
        None => Arc::new(UreqFetcher::with_limits(
            Duration::from_secs(15),
            20 * 1024 * 1024,
        )),
    }
}

/// How a fetch result reads as a link status.
pub fn status_of(result: &Result<Vec<u8>>) -> LinkStatus {
    match result {
        Ok(_) => LinkStatus::Ok,
        Err(e) => match FetchError::of(e) {
            Some(FetchError::Http(404 | 410)) => LinkStatus::NotFound,
            Some(FetchError::Http(c)) => LinkStatus::Error(format!("HTTP error {c}")),
            Some(FetchError::Timeout) => LinkStatus::Error("timed out".into()),
            _ => LinkStatus::Error(format!("{e}")),
        },
    }
}

/// Checks each URL once, at most one request per `interval_ms`. `sleep` and `now` are injected so the pacing is
/// testable; `progress(done, total)` is called after each URL; `cancelled` stops early.
pub fn check_urls(
    urls: &[String],
    fetcher: &dyn Fetcher,
    interval_ms: u64,
    now: &mut dyn FnMut() -> u64,
    sleep: &mut dyn FnMut(u64),
    progress: &mut dyn FnMut(usize, usize),
    cancelled: &dyn Fn() -> bool,
) -> Vec<(String, LinkStatus)> {
    let mut throttle = Throttle::new(interval_ms);
    let mut out = Vec::new();
    for (i, u) in urls.iter().enumerate() {
        if cancelled() {
            break;
        }
        let wait = throttle.wait_needed(now());
        if wait > 0 {
            sleep(wait);
        }
        throttle.record(now());
        out.push((u.clone(), status_of(&fetcher.get(u, &[]))));
        progress(i + 1, urls.len());
    }
    out
}

/// A [`LinkChecker`] that answers from results gathered earlier (on a worker thread).
pub struct Gathered(pub HashMap<String, LinkStatus>);

impl LinkChecker for Gathered {
    fn check(&self, url: &str) -> LinkStatus {
        self.0
            .get(url)
            .cloned()
            .unwrap_or(LinkStatus::Error("not checked".into()))
    }
}

/// Summary of a link check: `(broken, ok)` and one detail line per broken or invalid link.
pub fn link_report(results: &[links::LinkCheckResult]) -> (usize, usize, Vec<String>) {
    let mut broken = 0;
    let mut ok = 0;
    let mut lines = Vec::new();
    for r in results {
        match &r.status {
            LinkStatus::Ok | LinkStatus::Skipped => ok += 1,
            LinkStatus::Redirect(to) => {
                ok += 1;
                lines.push(format!("{}: {} moved to {to}", r.link.file, r.link.url));
            }
            LinkStatus::NotFound => {
                broken += 1;
                lines.push(format!("{}: {} was not found", r.link.file, r.link.url));
            }
            LinkStatus::Error(e) => {
                broken += 1;
                lines.push(format!("{}: {} ({e})", r.link.file, r.link.url));
            }
            LinkStatus::Invalid(e) => {
                broken += 1;
                lines.push(format!("{}: {} is invalid ({e})", r.link.file, r.link.url));
            }
        }
    }
    (broken, ok, lines)
}

/// Downloads every URL (and, for stylesheets, the resources they name) into a canned fetcher, up to `max_bytes`
/// each and `max_rounds` of stylesheet nesting. Runs on a worker thread.
pub fn prefetch(
    urls: &[String],
    fetcher: &dyn Fetcher,
    interval_ms: u64,
    max_bytes: usize,
    cancelled: &dyn Fn() -> bool,
) -> links::MockFetcher {
    let mut mock = links::MockFetcher::default();
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: Vec<String> = urls.to_vec();
    let mut throttle = Throttle::new(interval_ms);
    let start = std::time::Instant::now();
    let mut rounds = 0;
    while !queue.is_empty() && rounds < 3 && !cancelled() {
        rounds += 1;
        let mut next = Vec::new();
        for u in queue {
            if !seen.insert(u.clone()) || cancelled() {
                continue;
            }
            let now = start.elapsed().as_millis() as u64;
            let wait = throttle.wait_needed(now);
            if wait > 0 {
                std::thread::sleep(Duration::from_millis(wait));
            }
            throttle.record(start.elapsed().as_millis() as u64);
            match fetcher.get(&u, &[]) {
                Ok(data) if data.len() <= max_bytes => {
                    let ct = guess_content_type(&u);
                    if ct == Some("text/css") {
                        next.extend(links::remote_resources_in_css(&String::from_utf8_lossy(
                            &data,
                        )));
                    }
                    mock = mock.with(&u, &data, ct);
                }
                Ok(_) => mock = mock.failing(&u, "larger than the size limit"),
                Err(e) => mock = mock.failing(&u, &format!("{e}")),
            }
        }
        queue = next;
    }
    mock
}

fn guess_content_type(url: &str) -> Option<&'static str> {
    let path = url.split(['?', '#']).next().unwrap_or("");
    match extension(path).as_str() {
        "css" => Some("text/css"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "svg" => Some("image/svg+xml"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// Download external resources (CE-144) from a canned fetcher filled by [`prefetch`].
pub fn download_resources(book: &mut OpenBook, fetched: &links::MockFetcher) -> Result<Outcome> {
    let r = links::download_external_resources(book, fetched, &links::DownloadOptions::default())?;
    let mut details: Vec<String> = r
        .downloaded
        .iter()
        .map(|(u, p)| format!("{u} → {p}"))
        .collect();
    details.extend(
        r.failed
            .iter()
            .map(|(u, why)| format!("could not get {u}: {why}")),
    );
    Ok(if r.downloaded.is_empty() {
        outcome(
            false,
            if r.failed.is_empty() {
                tr("The book uses no external resources")
            } else {
                format!(
                    "{} could not be downloaded",
                    plural(r.failed.len(), "resource", "resources")
                )
            },
            details,
        )
    } else {
        outcome(
            true,
            trf(
                "Downloaded {}{}",
                &[
                    &plural(r.downloaded.len(), "resource", "resources"),
                    &(if r.failed.is_empty() {
                        String::new()
                    } else {
                        format!("; {} failed", r.failed.len())
                    }),
                ],
            ),
            details,
        )
    })
}

fn is_text(name: &str) -> bool {
    matches!(
        extension(name).as_str(),
        "xhtml" | "html" | "htm" | "xht" | "css" | "xml" | "opf" | "ncx" | "svg" | "txt" | "smil"
    )
}

/// What a tool would do, for the preview of a dialog: the summary, then for each text file that differs the
/// number of lines added and removed with a few before/after lines; or the detail lines when no text changed.
pub fn preview_report(
    before: &bookx_core::archive::Archive,
    after: &bookx_core::archive::Archive,
    outcome: &Result<Outcome>,
) -> String {
    let o = match outcome {
        Ok(o) => o,
        Err(e) => return format!("{e:#}"),
    };
    let mut out = vec![o.summary.clone()];
    let mut diffs = Vec::new();
    for name in after.names() {
        if !is_text(name) {
            continue;
        }
        let (Some(a), Some(b)) = (before.get_string(name), after.get_string(name)) else {
            continue;
        };
        if a != b {
            diffs.push((name.clone(), a, b));
        }
    }
    if diffs.is_empty() {
        out.extend(o.details.iter().take(10).cloned());
        if o.details.len() > 10 {
            out.push(format!("… and {} more", o.details.len() - 10));
        }
        return out.join("\n");
    }
    out.push(String::new());
    // lines about files that are not shown as a text difference (deleted, moved ...)
    let extra: Vec<&String> = o
        .details
        .iter()
        .filter(|d| !diffs.iter().any(|(n, ..)| d.starts_with(n.as_str())))
        .collect();
    for d in extra.iter().take(6) {
        out.push((*d).clone());
    }
    if extra.len() > 6 {
        out.push(format!("… and {} more", extra.len() - 6));
    }
    for (i, (name, a, b)) in diffs.iter().enumerate() {
        if i >= 8 {
            out.push(format!("… and {} more files", diffs.len() - 8));
            break;
        }
        let (add, del) = book_tools::line_diff_counts(a, b);
        out.push(format!("{name}: +{add} −{del} lines"));
        if i < 3 {
            for (x, y) in book_tools::diff_samples(a, b, 2) {
                out.push(format!("   − {x}"));
                out.push(format!("   + {y}"));
            }
        }
    }
    out.join("\n")
}

/// Files of the book by category in the order the file browser shows them (CE-020): text in reading order, the
/// others alphabetical. The category names are the group titles.
pub fn browser_order(
    book: &OpenBook,
    files: &[String],
    group_of: impl Fn(&str) -> &'static str,
) -> Vec<(&'static str, Vec<String>)> {
    let spine = book.spine_paths();
    let mut groups: BTreeMap<usize, (&'static str, Vec<String>)> = BTreeMap::new();
    for f in files {
        let g = group_of(f);
        let rank = ["Text", "Styles", "Images", "Fonts", "Package", "Other"]
            .iter()
            .position(|x| *x == g)
            .unwrap_or(5);
        groups
            .entry(rank)
            .or_insert((g, Vec::new()))
            .1
            .push(f.clone());
    }
    groups
        .into_values()
        .map(|(g, mut v)| {
            if g == "Text" {
                v.sort_by_key(|p| {
                    (
                        spine.iter().position(|s| s == p).unwrap_or(usize::MAX),
                        p.to_lowercase(),
                    )
                });
            } else {
                v.sort_by_key(|p| p.to_lowercase());
            }
            (g, v)
        })
        .collect()
}

/// The text of the tooltip of a file row: path, size, type and what is special about it (CE-021, CE-022).
pub fn file_tooltip(book: &OpenBook, path: &str) -> String {
    file_tooltip_with(book, path, &book_tools::file_notes(book, path))
}

/// [`file_tooltip`] for notes the caller already has.
pub fn file_tooltip_with(book: &OpenBook, path: &str, notes: &[book_tools::FileNote]) -> String {
    let mut lines = vec![path.to_string()];
    let size = book.archive.size_of(path);
    let mt = book
        .package
        .item_by_path(path)
        .map(|i| i.media_type.clone())
        .unwrap_or_else(|| bookx_core::util::media_type_for(path).to_string());
    lines.push(format!("{} · {}", kib(size), mt));
    for n in notes {
        lines.push(n.tooltip().to_string());
    }
    lines.join("\n")
}

/// An unzipped EPUB folder as an archive (CE-013): every file under `dir`, paths relative to it. A missing
/// `mimetype` is added; the folder must hold `META-INF/container.xml` and parse as a book.
pub fn archive_from_folder(dir: &std::path::Path) -> Result<bookx_core::archive::Archive> {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(dir, &mut files);
    files.sort();
    let mut a = bookx_core::archive::Archive::default();
    a.set("mimetype", b"application/epub+zip".to_vec());
    for f in files {
        let rel = f
            .strip_prefix(dir)?
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("/");
        if rel == "mimetype" {
            continue;
        }
        a.set(&rel, std::fs::read(&f)?);
    }
    if !a.contains("META-INF/container.xml") {
        bail!("That folder is not an unzipped EPUB: it has no META-INF/container.xml");
    }
    OpenBook::from_archive(a.clone())?;
    Ok(a)
}

/// Whether the network tools should be offered: `NET=0` (tests) or `BOOKX_OFFLINE` turns them off unless the canned
/// network is set.
pub fn network_allowed() -> bool {
    std::env::var_os("BOOKX_EDITOR_NET_MOCK").is_some()
        || (std::env::var("NET").map(|v| v != "0").unwrap_or(true) && !crate::privacy::is_offline())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_convert::transforms::punctuation::SmartenOptions;
    use bookx_core::convert::{create_epub, Chapter};
    use bookx_core::opf::Metadata;
    use bookx_editing::links::{ExternalLink, LinkCheckResult, LinkKind};

    fn book(bodies: &[&str]) -> OpenBook {
        let md = Metadata {
            title: "More".into(),
            authors: vec!["A".into()],
            language: "en".into(),
            ..Default::default()
        };
        let ch: Vec<Chapter> = bodies
            .iter()
            .enumerate()
            .map(|(i, b)| Chapter {
                title: format!("C{}", i + 1),
                body: (*b).into(),
            })
            .collect();
        OpenBook::from_archive(create_epub(&md, &ch, None)).unwrap()
    }

    #[test]
    fn smarten_reports_and_validates() {
        let mut b = book(&["<p>\"Hi\" -- there...</p>"]);
        assert!(smarten(
            &mut b,
            &[],
            Punctuation::Smarten(SmartenOptions {
                quotes: false,
                dashes: false,
                ellipses: false
            }),
            Default::default()
        )
        .is_err());
        let o = smarten(
            &mut b,
            &[],
            Punctuation::Smarten(SmartenOptions::default()),
            Default::default(),
        )
        .unwrap();
        assert!(o.changed && o.summary.starts_with("Smartened the punctuation in 1 file"));
        assert!(
            !smarten(
                &mut b,
                &[],
                Punctuation::Smarten(SmartenOptions::default()),
                Default::default()
            )
            .unwrap()
            .changed
        );
        let o = smarten(&mut b, &[], Punctuation::Unsmarten, Default::default()).unwrap();
        assert!(o.changed && o.summary.starts_with("Straightened"));
    }

    #[test]
    fn transform_styles_summary() {
        let mut b = book(&["<p>x</p>"]);
        assert!(transform_styles(&mut b, &[]).is_err());
        let css = bookx_editing::markup::css_files(&b.archive).remove(0);
        b.archive
            .set_string(&css, "p { color: red; font-size: 10px }");
        let o = transform_styles(&mut b, &[CssTransform::ScaleFontSizes { factor: 2.0 }]).unwrap();
        assert!(o.changed);
        assert_eq!(o.summary, "Changed 1 declaration in 1 file");
        assert!(b.archive.get_string(&css).unwrap().contains("20px"));
        let o = transform_styles(
            &mut b,
            &[CssTransform::RemoveProperty {
                name: "margin".into(),
            }],
        )
        .unwrap();
        assert!(!o.changed);
    }

    #[test]
    fn fix_html_and_beautify_and_css() {
        let mut b = book(&["<p>ok</p>", "<p>fine</p>"]);
        let s = b.spine_paths();
        b.archive
            .set_string(&s[1], "<html><body><p>Open <b>tags<p>more</body></html>");
        let o = fix_html(&mut b, &[s[1].clone()], false).unwrap();
        assert!(o.changed, "{o:?}");
        assert!(!fix_html(&mut b, &[], false).unwrap().changed);
        let o = beautify(&mut b, &[]).unwrap();
        assert!(o.changed);
        assert!(o.details.iter().any(|d| d.contains('+')));
        assert!(!beautify(&mut b, &[]).unwrap().changed);
        let css = bookx_editing::markup::css_files(&b.archive).remove(0);
        b.archive.set_string(&css, ".gone{color:red}");
        let o = remove_unused_css(&mut b).unwrap();
        assert!(o.changed);
        assert!(!remove_unused_css(&mut b).unwrap().changed);
    }

    #[test]
    fn filter_and_unused_media() {
        let mut b = book(&["<p style=\"color: red; margin: 0\">x</p>"]);
        assert!(filter_styles(&mut b, &FilterSpec::default()).is_err());
        let o = filter_styles(
            &mut b,
            &FilterSpec {
                properties: vec!["color".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            o.changed && o.summary.starts_with("Removed 1 declaration"),
            "{o:?}"
        );
        assert!(!delete_unused_media(&mut b, &[]).unwrap().changed);
        bookx_editing::files::add_file(&mut b, "orphan.png", vec![1, 2, 3], None).unwrap();
        let un = book_tools::unused_media(&b);
        assert_eq!(un.len(), 1);
        let o = delete_unused_media(&mut b, &[un[0].path.clone()]).unwrap();
        assert!(o.changed && o.summary.starts_with("Deleted 1 file"));
    }

    #[test]
    fn semantics_roles_and_spine() {
        let mut b = book(&["<p>1</p>", "<p>2</p>"]);
        let s = b.spine_paths();
        let o = set_semantics(&mut b, &[(Semantic::Foreword, Some(s[1].clone()))]).unwrap();
        assert!(o.changed, "{o:?}");
        assert!(
            !set_semantics(&mut b, &[(Semantic::Foreword, Some(s[1].clone()))])
                .unwrap()
                .changed
        );
        assert!(set_semantics(&mut b, &[(Semantic::Preface, Some("nope.xhtml".into()))]).is_err());
        assert!(
            set_semantics(&mut b, &[(Semantic::Foreword, None)])
                .unwrap()
                .changed
        );
        assert!(
            !set_semantics(&mut b, &[(Semantic::Foreword, None)])
                .unwrap()
                .changed
        );
        let o = mark_role(&mut b, &s[0], PageRole::TitlePage).unwrap();
        assert!(o.summary.contains("Title"), "{}", o.summary);
        assert!(spine_drop(&mut b, &s[0], 1).unwrap().changed);
        assert!(!spine_drop(&mut b, &s[0], 1).unwrap().changed);
        assert!(toggle_nonlinear(&mut b, &s[0])
            .unwrap()
            .summary
            .contains("no longer"));
        assert!(toggle_nonlinear(&mut b, &s[0])
            .unwrap()
            .summary
            .contains("again"));
    }

    #[test]
    fn extension_replace_and_move() {
        let mut b = book(&["<p>1</p>", "<p><a href=\"chapter-1.xhtml\">a</a></p>"]);
        assert!(change_extension(&mut b, &[], "").is_err());
        let s = b.spine_paths();
        let o = change_extension(&mut b, &[s[0].clone()], "html").unwrap();
        assert!(o.changed, "{o:?}");
        assert!(b
            .archive
            .get_string(&s[1])
            .unwrap()
            .contains("chapter-1.html"));
        assert!(!change_extension(&mut b, &[], "html").unwrap().changed);
        let new0 = b.spine_paths()[0].clone();
        let o = move_to_folder(&mut b, &new0, "Text").unwrap();
        assert!(o.changed, "{o:?}");
        assert!(b
            .archive
            .get_string(&s[1])
            .unwrap()
            .contains("Text/chapter-1.html"));
        let moved = b.spine_paths()[0].clone();
        assert!(!move_to_folder(&mut b, &moved, "Text").unwrap().changed);
        let o = replace_with(
            &mut b,
            &moved,
            "front.html",
            b"<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>NEW</p></body></html>".to_vec(),
        )
        .unwrap();
        assert!(o.details[0].contains("renamed"));
        assert!(b.archive.get_string(&s[1]).unwrap().contains("front.html"));
    }

    #[test]
    fn repair_links_summary() {
        let mut b = book(&[
            "<p>1</p>",
            "<p><a href=\"Chapter-1.xhtml\">a</a><a href=\"nothing.xhtml\">b</a></p>",
        ]);
        let o = repair_links(&mut b).unwrap();
        assert!(
            o.details.iter().any(|d| d.starts_with("still broken")),
            "{o:?}"
        );
        assert!(
            o.summary.contains("left") || o.summary.contains("cannot"),
            "{}",
            o.summary
        );
        let mut c = book(&["<p>1</p>"]);
        assert_eq!(repair_links(&mut c).unwrap().summary, "No broken links");
    }

    #[test]
    fn font_candidates_and_embedding_rules() {
        let mut b = book(&[
            "<p style=\"font-family: Zapf\">abc</p><p style=\"font-family: Open Sans\">x</p>",
        ]);
        let dir = std::env::temp_dir().join(format!("bookx-more-fonts-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("t.ttf"), bookx_editing::testfont::tiny_ttf(0)).unwrap();
        let mut installed = fonts::scan_system_fonts(std::slice::from_ref(&dir));
        installed[0].family = "zapf".into();
        let cands = font_candidates(&b, &installed);
        assert_eq!(cands.len(), 2);
        assert!(cands
            .iter()
            .find(|c| c.key.family == "zapf")
            .unwrap()
            .font
            .is_some());
        assert!(cands
            .iter()
            .find(|c| c.key.family == "open sans")
            .unwrap()
            .font
            .is_none());
        assert!(embed_fonts(&mut b, &installed, &HashSet::new(), false).is_err());
        // a restricted font is never embedded
        installed[0].embedding = fonts::Embedding::Restricted;
        let chosen: HashSet<String> = ["zapf".to_string()].into();
        assert!(
            !embed_fonts(&mut b, &installed, &chosen, false)
                .unwrap()
                .changed
        );
        installed[0].embedding = fonts::Embedding::Installable;
        let o = embed_fonts(&mut b, &installed, &chosen, true).unwrap();
        assert!(
            o.changed && o.summary.starts_with("Embedded 1 font"),
            "{o:?}"
        );
        assert_eq!(font_candidates(&b, &installed).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn accessibility_edits() {
        let mut b = book(&["<p><img src=\"a.png\"/></p>"]);
        let imgs = book_tools::image_uses(&b);
        let o = accessibility(
            &mut b,
            &AccessEdits {
                language: Some("de".into()),
                alts: vec![(imgs[0].file.clone(), 0, "A drawing".into())],
                all_uses: false,
                meta: Some(AccessMeta {
                    summary: "Tagged".into(),
                    ..Default::default()
                }),
            },
        )
        .unwrap();
        assert!(o.changed);
        assert_eq!(o.details.len(), 3, "{:?}", o.details);
        assert!(
            !accessibility(&mut b, &AccessEdits::default())
                .unwrap()
                .changed
        );
        let again = AccessEdits {
            language: Some("de".into()),
            alts: vec![(imgs[0].file.clone(), 0, "A drawing".into())],
            ..Default::default()
        };
        assert!(!accessibility(&mut b, &again).unwrap().changed);
    }

    #[test]
    fn link_checking_is_paced_and_reports() {
        let f = MockFetcher::new()
            .with_body("https://a/ok", "x")
            .on("https://a/gone", MockReply::Status(404))
            .on("https://a/boom", MockReply::Status(500))
            .on("https://a/slow", MockReply::Timeout);
        let urls: Vec<String> = ["ok", "gone", "boom", "slow", "unknown"]
            .iter()
            .map(|u| format!("https://a/{u}"))
            .collect();
        let clock = std::cell::Cell::new(0u64);
        let slept = std::cell::RefCell::new(Vec::new());
        let seen = std::cell::RefCell::new(Vec::new());
        let r = check_urls(
            &urls,
            &f,
            500,
            &mut || clock.get(),
            &mut |ms| {
                slept.borrow_mut().push(ms);
                clock.set(clock.get() + ms);
            },
            &mut |d, t| seen.borrow_mut().push((d, t)),
            &|| false,
        );
        assert_eq!(r.len(), 5);
        assert_eq!(r[0].1, LinkStatus::Ok);
        assert_eq!(r[1].1, LinkStatus::NotFound);
        assert!(matches!(&r[2].1, LinkStatus::Error(e) if e.contains("500")));
        assert!(matches!(&r[3].1, LinkStatus::Error(e) if e.contains("timed out")));
        assert!(matches!(
            r[4].1,
            LinkStatus::NotFound | LinkStatus::Error(_)
        ));
        assert_eq!(
            *slept.borrow(),
            vec![500, 500, 500, 500],
            "one request per interval"
        );
        assert_eq!(seen.borrow().last(), Some(&(5, 5)));
        // cancelling stops at once
        let n = std::cell::Cell::new(0);
        let r = check_urls(
            &urls,
            &f,
            0,
            &mut || 0,
            &mut |_| {},
            &mut |_, _| n.set(n.get() + 1),
            &|| n.get() >= 2,
        );
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn link_report_counts() {
        let l = |s| LinkCheckResult {
            link: ExternalLink {
                file: "c.xhtml".into(),
                url: "https://x".into(),
                kind: LinkKind::Https,
            },
            status: s,
        };
        let (broken, ok, lines) = link_report(&[
            l(LinkStatus::Ok),
            l(LinkStatus::NotFound),
            l(LinkStatus::Redirect("https://y".into())),
            l(LinkStatus::Error("x".into())),
            l(LinkStatus::Skipped),
        ]);
        assert_eq!((broken, ok, lines.len()), (2, 3, 3));
    }

    #[test]
    fn download_through_a_prefetch() {
        let mut b = book(&["<p><img src=\"https://x.test/p.png\" alt=\"\"/><img src=\"https://x.test/gone.png\" alt=\"\"/></p>"]);
        let net = MockFetcher::new()
            .with_body("https://x.test/p.png", bookx_editing_png())
            .on("https://x.test/gone.png", MockReply::Status(404));
        let urls = links::remote_resources(&b);
        assert_eq!(urls.len(), 2);
        let fetched = prefetch(&urls, &net, 0, 1 << 20, &|| false);
        let o = download_resources(&mut b, &fetched).unwrap();
        assert!(
            o.changed && o.summary.contains("1 resource") && o.summary.contains("1 failed"),
            "{o:?}"
        );
        assert!(links::remote_resources(&b)
            .iter()
            .all(|u| u.contains("gone")));
        // too large
        let b2 = book(&["<p><img src=\"https://x.test/p.png\" alt=\"\"/></p>"]);
        let big = prefetch(&links::remote_resources(&b2), &net, 0, 4, &|| false);
        assert!(big.responses.values().all(|r| r.is_err()));
    }

    fn bookx_editing_png() -> Vec<u8> {
        bookx_sources::cover::generate_cover("T", "A", "", 120, 160).unwrap()
    }

    #[test]
    fn preview_shows_diffs_or_details() {
        let mut b = book(&["<p>\"Hi\" there</p>"]);
        let before = b.archive.clone();
        let o = smarten(
            &mut b,
            &[],
            Punctuation::Smarten(SmartenOptions::default()),
            Default::default(),
        );
        let text = preview_report(&before, &b.archive, &o);
        assert!(
            text.starts_with("Smartened the punctuation in 1 file"),
            "{text}"
        );
        assert!(
            text.contains("+1 −1 lines") || text.contains("lines"),
            "{text}"
        );
        assert!(text.contains("   − ") && text.contains("   + "), "{text}");
        // a deletion has no text diff: the details are listed
        bookx_editing::files::add_file(&mut b, "orphan.png", vec![1], None).unwrap();
        let un = book_tools::unused_media(&b);
        let before = b.archive.clone();
        let o = delete_unused_media(&mut b, &[un[0].path.clone()]);
        let text = preview_report(&before, &b.archive, &o);
        assert!(text.contains("orphan.png deleted"), "{text}");
        let err: Result<Outcome> = Err(anyhow::anyhow!("Add at least one rule"));
        assert_eq!(
            preview_report(&before, &b.archive, &err),
            "Add at least one rule"
        );
    }

    #[test]
    fn unzipped_folder_becomes_an_archive() {
        let b = book(&["<p>x</p>"]);
        let dir = std::env::temp_dir().join(format!("bookx-folder-{}", uuid::Uuid::new_v4()));
        for n in b.archive.names() {
            if n == "mimetype" {
                continue;
            }
            let p = dir.join(n);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b.archive.get(n).unwrap()).unwrap();
        }
        let a = archive_from_folder(&dir).unwrap();
        assert_eq!(
            a.get_string("mimetype").as_deref(),
            Some("application/epub+zip")
        );
        assert_eq!(a.names().len(), b.archive.names().len());
        assert!(OpenBook::from_archive(a).is_ok());
        std::fs::remove_file(dir.join("META-INF/container.xml")).unwrap();
        assert!(archive_from_folder(&dir)
            .unwrap_err()
            .to_string()
            .contains("container.xml"));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(archive_from_folder(&dir).is_err());
    }

    #[test]
    fn browser_order_and_tooltips() {
        let b = book(&["<p>1</p>", "<p>2</p>"]);
        let mut files: Vec<String> = b.file_list();
        files.reverse();
        let g = |p: &str| match extension(p).as_str() {
            "xhtml" => "Text",
            "css" => "Styles",
            _ => "Other",
        };
        let groups = browser_order(&b, &files, g);
        assert_eq!(groups[0].0, "Text");
        let spine = b.spine_paths();
        let text: Vec<&String> = groups[0].1.iter().filter(|p| spine.contains(p)).collect();
        assert_eq!(
            text,
            spine.iter().collect::<Vec<_>>(),
            "text in reading order"
        );
        let t = file_tooltip(&b, &spine[0]);
        assert!(
            t.starts_with(&spine[0]) && t.contains("application/xhtml+xml") && t.contains("KB")
        );
        let opf = file_tooltip(&b, &b.package.opf_path);
        assert!(opf.contains("Package document"));
    }
}
