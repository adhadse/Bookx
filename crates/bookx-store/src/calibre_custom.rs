//! Custom columns between Bookx and Calibre's `metadata.db`.
//!
//! Mapping (Calibre datatype, Bookx type): `text` is text, `text` with several values is tags,
//! `series` is text (the series index is dropped), `comments` is long text, `enumeration` keeps its
//! values, `int`, `float`, `bool`, `rating` (Calibre stores 0 to 10) and `datetime` (kept as a date).
//! Calibre's `composite` columns are not imported: their templates are Calibre's template language,
//! which Bookx does not run. Bookx composite columns are exported as Calibre composites with their
//! template unchanged, which Calibre shows as a template error until the user rewrites them; the
//! values are not written.

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use bookx_core::custom::{ColumnDef, ColumnKind};
use bookx_core::library::Book;
use rusqlite::{params, types::Value, Connection};

use crate::calibre::parse_date;

/// What was read from a Calibre library.
#[derive(Debug, Default)]
pub(crate) struct CalibreCustom {
    pub defs: Vec<ColumnDef>,
    /// Calibre book id → label → value (already in the stored form).
    pub values: HashMap<i64, BTreeMap<String, String>>,
    /// Columns that were left out, with the reason.
    pub skipped: Vec<String>,
}

struct CalCol {
    id: i64,
    label: String,
    name: String,
    datatype: String,
    multiple: bool,
    display: serde_json::Value,
}

fn read_cols(cal: &Connection) -> Vec<CalCol> {
    let full = "SELECT id, label, name, datatype, is_multiple, display FROM custom_columns
                WHERE mark_for_delete = 0 ORDER BY id";
    let basic = "SELECT id, label, name, datatype, 0, '{}' FROM custom_columns ORDER BY id";
    for sql in [full, basic] {
        let Ok(mut st) = cal.prepare(sql) else {
            continue;
        };
        let Ok(rows) = st.query_map([], |r| {
            Ok(CalCol {
                id: r.get(0)?,
                label: r.get(1)?,
                name: r.get(2)?,
                datatype: r.get(3)?,
                multiple: r.get::<_, Option<i64>>(4)?.unwrap_or(0) != 0,
                display: serde_json::from_str(&r.get::<_, String>(5).unwrap_or_default())
                    .unwrap_or(serde_json::Value::Null),
            })
        }) else {
            continue;
        };
        return rows.flatten().collect();
    }
    Vec::new()
}

fn value_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Integer(i) => i.to_string(),
        Value::Real(f) => f.to_string(),
        Value::Text(t) => t.clone(),
        Value::Blob(_) => String::new(),
    }
}

/// Reads the definitions and every book's values; whatever cannot be read is left out.
pub(crate) fn read_calibre_custom(cal: &Connection) -> CalibreCustom {
    let mut out = CalibreCustom::default();
    for c in read_cols(cal) {
        let label = c.label.to_lowercase();
        let kind = match c.datatype.as_str() {
            "text" if c.multiple => ColumnKind::Tags,
            "text" | "series" => ColumnKind::Text,
            "comments" => ColumnKind::LongText,
            "enumeration" => ColumnKind::Enumeration,
            "int" => ColumnKind::Integer,
            "float" => ColumnKind::Float,
            "rating" => ColumnKind::Rating,
            "bool" => ColumnKind::Bool,
            "datetime" => ColumnKind::Date,
            other => {
                out.skipped
                    .push(format!("{} ({other} columns are not imported)", c.name));
                continue;
            }
        };
        let values = c
            .display
            .get("enum_values")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let def = ColumnDef {
            label: label.clone(),
            name: c.name.clone(),
            kind,
            values,
            template: String::new(),
        };
        if let Err(e) = def.validate() {
            out.skipped.push(format!("{} ({e})", c.name));
            continue;
        }
        let normalized = matches!(
            c.datatype.as_str(),
            "text" | "series" | "enumeration" | "rating"
        );
        let sql = if normalized {
            format!(
                "SELECT l.book, v.value FROM books_custom_column_{id}_link l
                 JOIN custom_column_{id} v ON v.id = l.value ORDER BY l.id",
                id = c.id
            )
        } else {
            format!("SELECT book, value FROM custom_column_{}", c.id)
        };
        let Ok(mut st) = cal.prepare(&sql) else {
            // the definition is kept even when the values cannot be read
            out.defs.push(def);
            continue;
        };
        let rows: Vec<(i64, Value)> = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|it| it.flatten().collect())
            .unwrap_or_default();
        let mut per_book: HashMap<i64, Vec<String>> = HashMap::new();
        for (book, v) in rows {
            let t = value_text(&v);
            let t = match c.datatype.as_str() {
                "rating" => t
                    .parse::<f64>()
                    .map(|r| (r / 2.0).to_string())
                    .unwrap_or_default(),
                "bool" => match t.as_str() {
                    "1" => "true".into(),
                    "0" => "false".into(),
                    _ => String::new(),
                },
                "datetime" => parse_date(&t)
                    .map(|d| d.format("%Y-%m-%d").to_string())
                    .unwrap_or_default(),
                _ => t,
            };
            if !t.is_empty() {
                per_book.entry(book).or_default().push(t);
            }
        }
        for (book, vals) in per_book {
            let joined = vals.join(if kind == ColumnKind::Tags { ", " } else { " " });
            if let Ok(n) = def.normalize(&joined) {
                if !n.is_empty() {
                    out.values.entry(book).or_default().insert(label.clone(), n);
                }
            }
        }
        out.defs.push(def);
    }
    out
}

fn datatype(k: ColumnKind) -> &'static str {
    match k {
        ColumnKind::Text | ColumnKind::Tags => "text",
        ColumnKind::LongText => "comments",
        ColumnKind::Integer => "int",
        ColumnKind::Float => "float",
        ColumnKind::Rating => "rating",
        ColumnKind::Bool => "bool",
        ColumnKind::Date => "datetime",
        ColumnKind::Enumeration => "enumeration",
        ColumnKind::Composite => "composite",
    }
}

/// Creates the custom column tables in a Calibre `metadata.db` and writes every book's values.
pub(crate) fn write_calibre_custom(
    tx: &Connection,
    defs: &[ColumnDef],
    books: &[Book],
) -> Result<()> {
    for (i, d) in defs.iter().enumerate() {
        let id = i as i64 + 1;
        let dt = datatype(d.kind);
        let multiple = d.kind == ColumnKind::Tags;
        let normalized = matches!(dt, "text" | "enumeration" | "rating");
        let display = match d.kind {
            ColumnKind::Enumeration => serde_json::json!({
                "enum_values": d.values, "enum_colors": [], "use_decorations": false,
                "description": ""
            }),
            ColumnKind::Composite => serde_json::json!({
                "composite_template": d.template, "composite_sort": "text",
                "make_category": false, "contains_html": false, "description": ""
            }),
            ColumnKind::Tags => serde_json::json!({
                "is_names": false, "description": "",
                "cc_icon_rules": [], "use_decorations": false
            }),
            _ => serde_json::json!({"description": ""}),
        };
        tx.execute(
            "INSERT INTO custom_columns(id, label, name, datatype, mark_for_delete, editable,
                                        display, is_multiple, normalized)
             VALUES(?1, ?2, ?3, ?4, 0, 1, ?5, ?6, ?7)",
            params![
                id,
                d.label,
                d.name,
                dt,
                display.to_string(),
                multiple as i64,
                // composites have no tables, so they are not "normalized"
                normalized as i64
            ],
        )?;
        if d.kind == ColumnKind::Composite {
            continue;
        }
        let sqltype = match dt {
            "int" | "rating" => "INT",
            "float" => "REAL",
            "bool" => "BOOL",
            "datetime" => "timestamp",
            _ => "TEXT",
        };
        let collate = if matches!(dt, "text" | "enumeration") {
            "COLLATE NOCASE"
        } else {
            ""
        };
        if normalized {
            tx.execute_batch(&format!(
                "CREATE TABLE custom_column_{id}(
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     value {sqltype} NOT NULL {collate},
                     link TEXT NOT NULL DEFAULT '',
                     UNIQUE(value));
                 CREATE INDEX custom_column_{id}_idx ON custom_column_{id} (value {collate});
                 CREATE TABLE books_custom_column_{id}_link(
                     id INTEGER PRIMARY KEY,
                     book INTEGER NOT NULL,
                     value INTEGER NOT NULL,
                     UNIQUE(book, value));
                 CREATE INDEX books_custom_column_{id}_link_aidx ON books_custom_column_{id}_link (value);
                 CREATE INDEX books_custom_column_{id}_link_bidx ON books_custom_column_{id}_link (book);"
            ))?;
        } else {
            tx.execute_batch(&format!(
                "CREATE TABLE custom_column_{id}(
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     book INTEGER NOT NULL,
                     value {sqltype} NOT NULL {collate},
                     UNIQUE(book));
                 CREATE INDEX custom_column_{id}_idx ON custom_column_{id} (book);"
            ))?;
        }
        for b in books {
            let v = b.custom_value(&d.label);
            if v.is_empty() {
                continue;
            }
            if normalized {
                let items: Vec<&str> = if multiple {
                    v.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .collect()
                } else {
                    vec![v]
                };
                for item in items {
                    let stored: Value = if dt == "rating" {
                        Value::Integer((v.parse::<f64>().unwrap_or(0.0) * 2.0).round() as i64)
                    } else {
                        Value::Text(item.to_string())
                    };
                    tx.execute(
                        &format!("INSERT OR IGNORE INTO custom_column_{id}(value) VALUES(?1)"),
                        params![stored],
                    )?;
                    let vid: i64 = tx.query_row(
                        &format!("SELECT id FROM custom_column_{id} WHERE value = ?1"),
                        params![stored],
                        |r| r.get(0),
                    )?;
                    tx.execute(
                        &format!(
                            "INSERT OR IGNORE INTO books_custom_column_{id}_link(book, value) VALUES(?1, ?2)"
                        ),
                        params![b.id as i64, vid],
                    )?;
                }
            } else {
                let stored: Value = match dt {
                    "int" => Value::Integer(v.parse().unwrap_or(0)),
                    "float" => Value::Real(v.parse().unwrap_or(0.0)),
                    "bool" => Value::Integer((v == "true") as i64),
                    "datetime" => Value::Text(crate::calibre_export::calibre_pubdate(v)),
                    _ => Value::Text(v.to_string()),
                };
                tx.execute(
                    &format!("INSERT INTO custom_column_{id}(book, value) VALUES(?1, ?2)"),
                    params![b.id as i64, stored],
                )?;
            }
        }
    }
    Ok(())
}
