//! `bookx-cli library replace`: search and replace in one metadata field of many books (roadmap L16).

use std::io::Write;
use std::path::Path;

use anyhow::{Result, anyhow};
use bookx_core::query::Query;
use bookx_core::replace::{self, Rule, Target};

#[derive(clap::Args, Debug)]
pub struct ReplaceArgs {
    /// The field: title, authors, series, publisher, language, tags, comments or #label
    #[arg(long)]
    pub field: String,
    /// The text to search for
    #[arg(long)]
    pub find: String,
    /// The replacement (with --regex, `$1` is the first group)
    #[arg(long = "with", default_value = "")]
    pub with: String,
    /// The search text is a regular expression
    #[arg(long)]
    pub regex: bool,
    /// Match upper and lower case as written
    #[arg(long)]
    pub case: bool,
    /// Show what would change and change nothing
    #[arg(long)]
    pub dry_run: bool,
    /// Only the books a library search finds (default: every book)
    pub query: Vec<String>,
}

pub fn run(a: ReplaceArgs, root: &Path, out: &mut dyn Write) -> Result<i32> {
    let target = Target::parse(&a.field).ok_or_else(|| {
        anyhow!("unknown field {:?}: use title, authors, series, publisher, language, tags, comments or #label", a.field)
    })?;
    let rule = Rule {
        target,
        find: a.find,
        replace: a.with,
        regex: a.regex,
        case_sensitive: a.case,
    };
    if !bookx_store::has_library(root) {
        anyhow::bail!("no library at {} (add books first)", root.display());
    }
    let ids_of = |lib: &bookx_core::library::Library| -> Vec<u64> {
        let q = Query::parse(&a.query.join(" "));
        lib.books
            .iter()
            .filter(|b| q.matches(b))
            .map(|b| b.id)
            .collect()
    };
    if a.dry_run {
        let lib = bookx_store::open_library(root)?.library;
        let ids = ids_of(&lib);
        let p = replace::preview(&lib, &ids, &rule, usize::MAX)?;
        for c in &p.changes {
            let note = c
                .problem
                .as_deref()
                .map(|m| format!("  [skipped: {m}]"))
                .unwrap_or_default();
            writeln!(out, "{:>5}  {:?} -> {:?}{note}", c.id, c.before, c.after)?;
        }
        writeln!(out, "{} book(s) would change", p.total - p.problems)?;
        return Ok(0);
    }
    let mut guarded = crate::libtools::open_for_write(root)?;
    let lib = &mut guarded.opened.library;
    let ids = ids_of(lib);
    let (n, skipped) = replace::apply(lib, &ids, &rule)?;
    write!(out, "{n} book(s) changed")?;
    if skipped > 0 {
        write!(out, ", {skipped} skipped because the result is not allowed")?;
    }
    writeln!(out)?;
    Ok(0)
}
