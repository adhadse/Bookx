//! `bookx-cli highlights repair`: corrects the colour and style of highlights that are already in the
//! library from what the e-readers say (see `bookx_serve::look_repair` and `docs/highlight-sync-rca.md`).
//!
//! Without `--apply` it only reports. With `--apply` it changes the library under the library lock
//! and writes one entry in the library's history, so `undo` takes it back.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use bookx_serve::look_repair as repair;
use clap::Subcommand;

use crate::libtools;

#[derive(Subcommand, Debug)]
pub enum HighlightsCmd {
    /// Correct the colour and style of highlights already in the library from what the devices say.
    /// Only the look of existing highlights changes: nothing is added or deleted, and a look you chose
    /// in Bookx stays. Without --apply nothing is changed.
    Repair(RepairArgs),
}

#[derive(clap::Args, Debug)]
pub struct RepairArgs {
    /// A mounted e-reader or a copy of its folders (KOReader `.sdr` sidecars, `.kobo/KoboReader.sqlite`), one
    /// KOReader `metadata.*.lua`, or a `KoboReader.sqlite`. Repeatable. The sync records and ledgers of
    /// Bookx's own settings folder are always used when they exist.
    #[arg(long = "from")]
    from: Vec<PathBuf>,
    /// Bookx's settings folder with the sync records (default: the app's)
    #[arg(long)]
    config_dir: Option<PathBuf>,
    /// Report what would change and change nothing (this is the default)
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    /// Make the changes
    #[arg(long)]
    apply: bool,
    #[arg(long)]
    json: bool,
}

fn default_config_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx").map(|d| d.config_dir().to_path_buf())
}

/// Every row the sources give.
fn gather(from: &[PathBuf], config: Option<&Path>, library: &Path) -> Vec<repair::Truth> {
    let mut rows = Vec::new();
    for p in from {
        rows.extend(repair::truths_from_path(p));
    }
    if let Some(c) = config {
        rows.extend(repair::truths_from_annotation_store(
            &c.join("koreader-annotations.json"),
        ));
    }
    rows.extend(repair::truths_from_ledgers(config, library));
    rows
}

pub fn run(cmd: HighlightsCmd, root: &Path, out: &mut dyn Write) -> Result<i32> {
    let HighlightsCmd::Repair(a) = cmd;
    if !bookx_store::has_library(root) {
        bail!(
            "no library at {} (add books first, or pass --library)",
            root.display()
        );
    }
    let config = a.config_dir.clone().or_else(default_config_dir);
    let rows = gather(&a.from, config.as_deref(), root);
    let mut g = if a.apply {
        libtools::open_for_write(root)?
    } else {
        // a report does not need the lock
        bookx_store::guard::open_library_guarded(root, "Bookx CLI", true)?
    };
    let ledger = bookx_core::devstyle::DeviceLooks::load(&g.opened.library.root);
    let plan = repair::plan(&g.opened.library.books, &ledger, &rows);
    let changed = if a.apply {
        repair::apply(&mut g.opened.library, &plan)?
    } else {
        0
    };
    if a.json {
        let fixes: Vec<_> = plan
            .fixes
            .iter()
            .map(|f| {
                serde_json::json!({
                    "book": f.book, "title": f.book_title, "highlight": f.highlight, "text": f.text,
                    "from": {"color": f.from.color, "style": f.from.style},
                    "to": {"color": f.to.color, "style": f.to.style}, "source": f.source,
                })
            })
            .collect();
        let r = &plan.report;
        writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "applied": a.apply, "changed": changed,
                "report": {
                    "rows": r.rows, "fixed": r.fixed, "normalised": r.normalised, "right": r.right,
                    "kept": r.kept, "ambiguous": r.ambiguous, "no_highlight": r.no_highlight,
                    "unmatched_books": r.unmatched_books, "sources": r.sources,
                },
                "fixes": fixes,
            }))?
        )?;
        return Ok(0);
    }
    if plan.report.rows == 0 {
        writeln!(
            out,
            "no source to learn the looks from: pass --from <device folder, sidecar or KoboReader.sqlite>"
        )?;
    }
    for (s, n) in &plan.report.sources {
        writeln!(out, "source: {s}: {n} highlight(s)")?;
    }
    for f in plan.fixes.iter().take(40) {
        writeln!(
            out,
            "{} #{} {:?}: {}/{} -> {}/{}",
            if a.apply { "fixed" } else { "would fix" },
            f.book,
            f.text.chars().take(50).collect::<String>(),
            f.from.color,
            f.from.style,
            f.to.color,
            f.to.style
        )?;
    }
    if plan.fixes.len() > 40 {
        writeln!(out, "... and {} more", plan.fixes.len() - 40)?;
    }
    for t in &plan.report.unmatched_books {
        writeln!(out, "book not in the library: {t}")?;
    }
    writeln!(out, "{}", plan.report.text())?;
    if a.apply {
        writeln!(
            out,
            "{changed} highlight(s) changed (one undo step in the library's history)"
        )?;
    } else if plan.report.changed() > 0 {
        writeln!(out, "nothing was changed: run again with --apply")?;
    }
    Ok(0)
}
