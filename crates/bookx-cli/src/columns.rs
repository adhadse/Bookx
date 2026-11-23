//! `bookx-cli library columns …`: the custom columns of the library (roadmap L3/L25).

use std::io::Write;
use std::path::Path;

use anyhow::{Result, anyhow, bail};
use bookx_core::custom::{ColumnDef, ColumnKind, label_from_name};
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum ColumnsCmd {
    /// Show the custom columns, or with --values the values one column has in the library
    List {
        #[arg(long)]
        json: bool,
        /// Show the values of this column (its label) with how many books carry each
        #[arg(long)]
        values: Option<String>,
    },
    /// Add a custom column
    Add {
        /// The name the column is shown with; its label (`#label` in searches) is made from it
        name: String,
        /// text, long_text, integer, float, rating, bool, date, tags, enumeration or composite
        #[arg(long = "type", default_value = "text")]
        kind: String,
        /// The label used in searches (default: made from the name)
        #[arg(long)]
        label: Option<String>,
        /// The allowed values of an enumeration, separated by commas
        #[arg(long)]
        values: Option<String>,
        /// The template of a composite column, such as "{series} {series_index}"
        #[arg(long)]
        template: Option<String>,
    },
    /// Remove a custom column and its values from every book
    Remove { label: String },
    /// Set (or with an empty value clear) a value: `columns set 12 mood Calm`
    Set {
        /// The id of the book
        id: u64,
        label: String,
        value: String,
    },
}

pub fn run(cmd: ColumnsCmd, root: &Path, out: &mut dyn Write) -> Result<i32> {
    match cmd {
        ColumnsCmd::List { json, values } => {
            if !bookx_store::has_library(root) {
                bail!("no library at {} (add books first)", root.display());
            }
            let lib = bookx_store::open_library(root)?.library;
            if let Some(label) = values {
                let label = label.trim_start_matches('#');
                if lib.column(label).is_none() {
                    bail!("there is no custom column `{label}`");
                }
                let vals = lib.custom_values(label);
                if json {
                    let v: Vec<_> = vals
                        .iter()
                        .map(|(v, n)| serde_json::json!({"value": v, "books": n}))
                        .collect();
                    writeln!(out, "{}", serde_json::to_string_pretty(&v)?)?;
                } else {
                    for (v, n) in vals {
                        writeln!(out, "{n:>5}  {v}")?;
                    }
                }
                return Ok(0);
            }
            if json {
                writeln!(out, "{}", serde_json::to_string_pretty(lib.columns())?)?;
            } else if lib.columns().is_empty() {
                writeln!(
                    out,
                    "no custom columns (add one with `library columns add`)"
                )?;
            } else {
                for c in lib.columns() {
                    let extra = match c.kind {
                        ColumnKind::Enumeration => format!(" [{}]", c.values.join(", ")),
                        ColumnKind::Composite => format!(" = {}", c.template),
                        _ => String::new(),
                    };
                    writeln!(
                        out,
                        "#{:<16} {:<12} {}{extra}",
                        c.label,
                        c.kind.key(),
                        c.name
                    )?;
                }
            }
            Ok(0)
        }
        ColumnsCmd::Add {
            name,
            kind,
            label,
            values,
            template,
        } => {
            let kind = ColumnKind::parse(&kind).ok_or_else(|| {
                anyhow!(
                    "unknown type {kind:?}: use {}",
                    ColumnKind::ALL.map(|k| k.key()).join(", ")
                )
            })?;
            let mut guarded = crate::libtools::open_for_write(root)?;
            let lib = &mut guarded.opened.library;
            let label = label
                .map(|l| l.trim_start_matches('#').to_string())
                .unwrap_or_else(|| label_from_name(&name));
            let mut defs = lib.columns().to_vec();
            defs.push(ColumnDef {
                label: label.clone(),
                name,
                kind,
                values: values
                    .map(|v| {
                        v.split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect()
                    })
                    .unwrap_or_default(),
                template: template.unwrap_or_default(),
            });
            lib.set_columns(defs)?;
            writeln!(out, "added the column #{label}")?;
            Ok(0)
        }
        ColumnsCmd::Remove { label } => {
            let label = label.trim_start_matches('#').to_string();
            let mut guarded = crate::libtools::open_for_write(root)?;
            let lib = &mut guarded.opened.library;
            if lib.column(&label).is_none() {
                bail!("there is no custom column `{label}`");
            }
            let defs: Vec<ColumnDef> = lib
                .columns()
                .iter()
                .filter(|c| c.label != label)
                .cloned()
                .collect();
            let had = lib
                .books
                .iter()
                .filter(|b| !b.custom_value(&label).is_empty())
                .count();
            lib.set_columns(defs)?;
            writeln!(out, "removed #{label} and its values from {had} book(s)")?;
            Ok(0)
        }
        ColumnsCmd::Set { id, label, value } => {
            let label = label.trim_start_matches('#').to_string();
            let mut guarded = crate::libtools::open_for_write(root)?;
            let lib = &mut guarded.opened.library;
            lib.set_custom(id, &label, &value)?;
            writeln!(
                out,
                "#{label} of book {id} is now {}",
                match lib.get(id).map(|b| b.custom_value(&label)) {
                    Some(v) if !v.is_empty() => v.to_string(),
                    _ => "empty".to_string(),
                }
            )?;
            Ok(0)
        }
    }
}
