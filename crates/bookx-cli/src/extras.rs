//! `bookx-cli library …` commands for citations, file checksums and cover quality:
//! `cite`, `fixity`, `covers`. They only read the library (`fixity` also writes its own
//! `.bookx/fixity.json`), so they do not take the library lock.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use anyhow::{Result, anyhow, bail};
use bookx_core::fixity;
use bookx_core::query::Query;
use bookx_sources::citation::{self, Style};
use bookx_sources::cover_quality as cq;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum ExtrasCmd {
    /// Print a bibliography of the library (or of the books a search finds)
    Cite {
        /// bibtex, ris, csl-json or apa
        #[arg(long, default_value = "bibtex")]
        style: String,
        /// Write to this file instead of the screen
        #[arg(long, short)]
        output: Option<PathBuf>,
        /// A library search, for example `author:herbert` (default: every book)
        query: Vec<String>,
    },
    /// Check the book files against their recorded SHA-256 checksums (the first run records them);
    /// exits with 1 when a file is damaged, missing or unreadable
    Fixity {
        #[arg(long)]
        json: bool,
        /// Accept the current content of this file (path relative to the library) as correct
        #[arg(long)]
        accept: Option<String>,
    },
    /// List books whose cover is small or oddly shaped
    Covers {
        #[arg(long)]
        json: bool,
        /// List every cover with its size, not only the poor ones
        #[arg(long)]
        all: bool,
    },
}

fn open_existing(root: &Path) -> Result<bookx_store::OpenedLibrary> {
    if !bookx_store::has_library(root) {
        bail!(
            "no library at {} (add books first, or pass --library)",
            root.display()
        );
    }
    bookx_store::open_library(root)
}

pub fn run(cmd: ExtrasCmd, root: &Path, out: &mut dyn Write) -> Result<i32> {
    match cmd {
        ExtrasCmd::Cite {
            style,
            output,
            query,
        } => {
            let style = Style::parse(&style).ok_or_else(|| {
                anyhow!("unknown style {style:?}: use bibtex, ris, csl-json or apa")
            })?;
            let opened = open_existing(root)?;
            let q = Query::parse(&query.join(" "));
            let books: Vec<&bookx_core::library::Book> = opened
                .library
                .books
                .iter()
                .filter(|b| q.matches(b))
                .collect();
            let text = citation::render_books(&books, style);
            match output {
                Some(p) => {
                    std::fs::write(&p, &text)?;
                    writeln!(out, "wrote {} reference(s) to {}", books.len(), p.display())?;
                }
                None => out.write_all(text.as_bytes())?,
            }
            Ok(0)
        }
        ExtrasCmd::Fixity { json, accept } => {
            let opened = open_existing(root)?;
            let now = chrono::Utc::now().timestamp();
            if let Some(rel) = accept {
                fixity::accept(&opened.library.root, &rel, now)?;
                writeln!(out, "accepted {rel}")?;
                return Ok(0);
            }
            let files: Vec<String> = opened
                .library
                .books
                .iter()
                .flat_map(|b| b.all_formats().into_iter().map(|f| f.file))
                .collect();
            let report =
                fixity::verify(&opened.library.root, &files, now, &AtomicBool::new(false))?;
            if json {
                let v = serde_json::json!({
                    "checked": report.files.len(),
                    "intact": report.intact(),
                    "recorded": report.recorded(),
                    "edited": report.edited(),
                    "damaged": report.damaged(),
                    "missing": report.missing(),
                    "unreadable": report.unreadable(),
                });
                writeln!(out, "{}", serde_json::to_string_pretty(&v)?)?;
            } else {
                writeln!(out, "{}", report.summary())?;
                for p in report.damaged() {
                    writeln!(
                        out,
                        "DAMAGED  {p}  (content changed, date and size did not)"
                    )?;
                }
                for p in report.missing() {
                    writeln!(out, "MISSING  {p}")?;
                }
            }
            Ok(if report.is_clean() { 0 } else { 1 })
        }
        ExtrasCmd::Covers { json, all } => {
            let opened = open_existing(root)?;
            let mut rows = Vec::new();
            for b in &opened.library.books {
                let Some(path) = opened.library.cover_path(b) else {
                    continue;
                };
                let size = std::fs::read(&path).ok().and_then(|d| cq::size_of(&d));
                let Some(size) = size else {
                    continue;
                };
                if all || cq::grade(size) < cq::Grade::Good {
                    rows.push((b.id, b.meta.title.clone(), size));
                }
            }
            if json {
                let v: Vec<_> = rows
                    .iter()
                    .map(|(id, title, s)| {
                        serde_json::json!({
                            "id": id, "title": title, "width": s.0, "height": s.1,
                            "grade": cq::grade(*s).label(), "odd_shape": cq::odd_shape(*s),
                        })
                    })
                    .collect();
                writeln!(out, "{}", serde_json::to_string_pretty(&v)?)?;
            } else if rows.is_empty() {
                writeln!(out, "every cover is good")?;
            } else {
                for (id, title, s) in rows {
                    writeln!(out, "{id:>6}  {title}  {}", cq::describe_quality(s))?;
                }
            }
            Ok(0)
        }
    }
}
