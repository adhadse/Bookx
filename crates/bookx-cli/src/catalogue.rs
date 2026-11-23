//! `bookx-cli library …` commands for several formats per book and for author names:
//! `formats`, `add-format`, `remove-format`, `merge-formats`, `authors`, `rename-author`.
//!
//! Commands that change the library take the library lock like the other writers.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use bookx_sources::aliases;
use clap::Subcommand;

use crate::libtools;

#[derive(Subcommand, Debug)]
pub enum CatalogueCmd {
    /// List the format files of a book
    Formats { id: u64 },
    /// Add a file (AZW3, MOBI, PDF, ...) to a book as another format
    AddFormat { id: u64, file: PathBuf },
    /// Take one further format off a book (the file waits in the trash folder; `undo` brings it back)
    RemoveFormat {
        id: u64,
        /// The format, for example azw3
        format: String,
    },
    /// Merge books into the first one as formats of one book (`undo` takes it back)
    MergeFormats {
        /// The book that stays, then the books to fold into it
        ids: Vec<u64>,
        /// Only show what would happen
        #[arg(long)]
        dry_run: bool,
    },
    /// List the authors grouped by person (pen names and spellings merged)
    Authors {
        #[arg(long)]
        json: bool,
    },
    /// Give every book that lists one of these names the new name, and remember them as aliases
    RenameAuthor {
        /// The spellings to replace
        from: Vec<String>,
        /// The name they all become
        #[arg(long)]
        to: String,
        /// Only show which books would change
        #[arg(long)]
        dry_run: bool,
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

pub fn run(cmd: CatalogueCmd, root: &Path, out: &mut dyn Write) -> Result<i32> {
    match cmd {
        CatalogueCmd::Formats { id } => {
            let opened = open_existing(root)?;
            let b = opened
                .library
                .get(id)
                .ok_or_else(|| anyhow!("no book with id {id}"))?;
            for (i, f) in b.all_formats().iter().enumerate() {
                writeln!(
                    out,
                    "{}{}  {}  {} bytes",
                    f.format,
                    if i == 0 { " (main)" } else { "" },
                    f.file,
                    f.size
                )?;
            }
            Ok(0)
        }
        CatalogueCmd::AddFormat { id, file } => {
            let mut g = libtools::open_for_write(root)?;
            let f = g.opened.library.add_format(id, &file)?;
            g.opened.library.save()?;
            writeln!(out, "added {} to #{id}", f.format)?;
            Ok(0)
        }
        CatalogueCmd::RemoveFormat { id, format } => {
            let mut g = libtools::open_for_write(root)?;
            let file = g
                .opened
                .library
                .get(id)
                .ok_or_else(|| anyhow!("no book with id {id}"))?
                .formats
                .iter()
                .find(|f| f.format.eq_ignore_ascii_case(&format))
                .map(|f| f.file.clone())
                .ok_or_else(|| anyhow!("#{id} has no further {format} file"))?;
            g.opened.library.remove_format(id, &file)?;
            g.opened.library.save()?;
            writeln!(out, "removed {format} from #{id}")?;
            Ok(0)
        }
        CatalogueCmd::MergeFormats { ids, dry_run } => {
            let Some((&primary, others)) = ids.split_first() else {
                bail!("give the book that stays, then the books to merge into it");
            };
            if others.is_empty() {
                bail!("give at least one book to merge into #{primary}");
            }
            let mut g = libtools::open_for_write(root)?;
            let plan = g.opened.library.plan_merge(primary, others)?;
            writeln!(out, "result: {}", plan.summary())?;
            for i in &plan.items {
                match &i.blocked {
                    None => writeln!(out, "merge #{} ({})", i.id, i.formats.join(", "))?,
                    Some(why) => writeln!(out, "skip  #{}: {why}", i.id)?,
                }
            }
            if dry_run {
                return Ok(0);
            }
            let n = bookx_store::formats::merge(
                &mut g.opened.library,
                &g.opened.store,
                primary,
                others,
            )?;
            g.opened.library.save()?;
            writeln!(out, "merged {n} book(s) into #{primary}")?;
            Ok(0)
        }
        CatalogueCmd::Authors { json } => {
            let opened = open_existing(root)?;
            let map = aliases::AliasMap::from_pairs(&opened.store.author_aliases());
            let groups = aliases::group_authors(&opened.library.books, &map);
            if json {
                let v: Vec<_> = groups
                    .iter()
                    .map(|a| {
                        serde_json::json!({
                            "name": a.canonical,
                            "books": a.books,
                            "spellings": a.variants.iter().map(|(v, _)| v).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                writeln!(out, "{}", serde_json::to_string_pretty(&v)?)?;
            } else {
                for a in &groups {
                    writeln!(
                        out,
                        "{}  ({} book(s)){}",
                        a.canonical,
                        a.books,
                        if a.variants.len() > 1 {
                            format!(
                                "  also: {}",
                                a.variants
                                    .iter()
                                    .map(|(v, _)| v.as_str())
                                    .filter(|v| *v != a.canonical)
                                    .collect::<Vec<_>>()
                                    .join("; ")
                            )
                        } else {
                            String::new()
                        }
                    )?;
                }
            }
            Ok(0)
        }
        CatalogueCmd::RenameAuthor { from, to, dry_run } => {
            if from.is_empty() {
                bail!("give the spellings to replace");
            }
            let mut g = libtools::open_for_write(root)?;
            let plan = aliases::plan_rename(&g.opened.library.books, &from, &to);
            writeln!(out, "{}", plan.summary())?;
            for i in &plan.items {
                writeln!(
                    out,
                    "  #{} {}: {} -> {}",
                    i.id,
                    i.title,
                    i.before.join(" & "),
                    i.after.join(" & ")
                )?;
            }
            if dry_run {
                return Ok(0);
            }
            let n = g
                .opened
                .library
                .update_metadata_many(format!("Renamed author to \"{to}\""), plan.edits())?;
            for f in &from {
                if aliases::name_key(f) != aliases::name_key(&to) {
                    g.opened.store.set_author_alias(f, &to)?;
                }
            }
            g.opened.library.save()?;
            writeln!(out, "renamed on {n} book(s)")?;
            Ok(0)
        }
    }
}
