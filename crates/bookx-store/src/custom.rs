//! The definitions of the custom columns (roadmap L3/L25). The values live in each book's record
//! ([`bookx_core::library::Book::custom`]), so they travel with every snapshot, backup and export of
//! the catalogue; this table only says which columns exist, in which order.

use anyhow::Result;
use bookx_core::custom::{validate_columns, ColumnDef, ColumnKind};
use rusqlite::{params, Connection};

use crate::store::SqliteStore;

pub(crate) fn migrate_v8(tx: &Connection) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS custom_columns(
             label TEXT PRIMARY KEY,
             position INTEGER NOT NULL,
             name TEXT NOT NULL,
             kind TEXT NOT NULL,
             enum_values TEXT NOT NULL DEFAULT '[]',
             template TEXT NOT NULL DEFAULT ''
         ) WITHOUT ROWID;",
    )?;
    Ok(())
}

pub(crate) fn load_columns(conn: &Connection) -> Vec<ColumnDef> {
    let Ok(mut st) = conn.prepare(
        "SELECT label, name, kind, enum_values, template FROM custom_columns ORDER BY position, label",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = st.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
        ))
    }) else {
        return Vec::new();
    };
    rows.flatten()
        .filter_map(|(label, name, kind, values, template)| {
            Some(ColumnDef {
                label,
                name,
                kind: ColumnKind::parse(&kind)?,
                values: serde_json::from_str(&values).unwrap_or_default(),
                template,
            })
        })
        .collect()
}

pub(crate) fn store_columns(conn: &Connection, defs: &[ColumnDef]) -> Result<()> {
    validate_columns(defs)?;
    conn.execute("DELETE FROM custom_columns", [])?;
    let mut ins = conn.prepare(
        "INSERT INTO custom_columns(label, position, name, kind, enum_values, template)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for (i, d) in defs.iter().enumerate() {
        ins.execute(params![
            d.label,
            i as i64,
            d.name,
            d.kind.key(),
            serde_json::to_string(&d.values)?,
            d.template
        ])?;
    }
    Ok(())
}

impl SqliteStore {
    /// The custom column definitions, in the order the user arranged them.
    pub fn custom_columns(&self) -> Vec<ColumnDef> {
        load_columns(&self.lock())
    }

    /// Replaces the custom column definitions (validated first; nothing changes on error). The
    /// values of removed columns stay in the books' records until the library removes them (the
    /// library does that in [`bookx_core::library::Library::set_columns`]).
    pub fn set_custom_columns(&self, defs: &[ColumnDef]) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        store_columns(&tx, defs)?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(label: &str, kind: ColumnKind) -> ColumnDef {
        ColumnDef {
            label: label.into(),
            name: label.to_uppercase(),
            kind,
            ..Default::default()
        }
    }

    #[test]
    fn definitions_round_trip_in_order() {
        let s = SqliteStore::open_in_memory().unwrap();
        assert!(s.custom_columns().is_empty());
        let mut e = def("mood", ColumnKind::Enumeration);
        e.values = vec!["Calm".into(), "Tense".into()];
        let mut c = def("both", ColumnKind::Composite);
        c.template = "{title} {#mood}".into();
        let defs = vec![def("zeta", ColumnKind::Integer), e, c];
        s.set_custom_columns(&defs).unwrap();
        assert_eq!(s.custom_columns(), defs, "same columns in the same order");
        // replacing drops what is not listed
        s.set_custom_columns(&defs[..1]).unwrap();
        assert_eq!(s.custom_columns().len(), 1);
    }

    #[test]
    fn invalid_definitions_change_nothing() {
        let s = SqliteStore::open_in_memory().unwrap();
        s.set_custom_columns(&[def("a", ColumnKind::Text)]).unwrap();
        let bad = [def("title", ColumnKind::Text)];
        assert!(s.set_custom_columns(&bad).is_err());
        assert_eq!(s.custom_columns().len(), 1);
        assert_eq!(s.custom_columns()[0].label, "a");
    }
}
