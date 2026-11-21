//! The decisions behind the custom columns UI (roadmap L3/L25), without GTK: which values a form
//! changed, what a table cell shows, which columns the table shows, and whether a remembered sort
//! by a custom column still applies. The widgets are in `ui/custom_columns.rs`.

use std::collections::BTreeMap;

use crate::core::custom::{ColumnDef, ColumnKind};
use crate::core::library::SortKey;

/// What a choice row (yes/no, enumeration) offers: `""` first (no value), then the values.
pub fn choices(def: &ColumnDef) -> Vec<String> {
    let mut v = vec![String::new()];
    match def.kind {
        ColumnKind::Bool => {
            v.push("true".into());
            v.push("false".into());
        }
        ColumnKind::Enumeration => v.extend(def.values.iter().map(|s| s.trim().to_string())),
        // half stars: 0.5 to 5
        ColumnKind::Rating => v.extend((1..=10).map(|i| format!("{}", i as f64 / 2.0))),
        _ => {}
    }
    v
}

/// True for the columns edited with a choice row.
pub fn is_choice(def: &ColumnDef) -> bool {
    matches!(
        def.kind,
        ColumnKind::Bool | ColumnKind::Enumeration | ColumnKind::Rating
    )
}

/// The values of `entered` (label, text as typed) that differ from what the book has, in the form
/// [`bookx_core::library::Library::edit_custom`] takes. An entry that is invalid is an error
/// naming the column. Composite columns are never changed.
pub fn changed(
    defs: &[ColumnDef],
    original: &BTreeMap<String, String>,
    entered: &[(String, String)],
) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for (label, text) in entered {
        let Some(def) = defs.iter().find(|d| &d.label == label) else {
            continue;
        };
        if def.kind == ColumnKind::Composite {
            continue;
        }
        let norm = def
            .normalize(text)
            .map_err(|e| format!("{}: {e}", def.name))?;
        let before = original.get(label).map(String::as_str).unwrap_or("");
        if norm != before {
            out.push((label.clone(), norm));
        }
    }
    Ok(out)
}

/// For the bulk editor: only the columns that were filled in change; an empty field changes
/// nothing. Invalid values are errors.
pub fn bulk_changes(
    defs: &[ColumnDef],
    entered: &[(String, String)],
) -> Result<Vec<(String, String)>, String> {
    let filled: Vec<(String, String)> = entered
        .iter()
        .filter(|(_, v)| !v.trim().is_empty())
        .cloned()
        .collect();
    let mut out = Vec::new();
    for (label, text) in &filled {
        let Some(def) = defs.iter().find(|d| &d.label == label) else {
            continue;
        };
        if def.kind == ColumnKind::Composite {
            continue;
        }
        let n = def
            .normalize(text)
            .map_err(|e| format!("{}: {e}", def.name))?;
        out.push((label.clone(), n));
    }
    Ok(out)
}

/// The text of a table cell or details row. `yes` and `no` are the translated words.
pub fn cell_text(def: &ColumnDef, stored: &str, yes: &str, no: &str) -> String {
    if stored.is_empty() {
        return String::new();
    }
    match def.kind {
        ColumnKind::Bool => match stored {
            "true" => yes.to_string(),
            "false" => no.to_string(),
            other => other.to_string(),
        },
        _ => def.display(stored),
    }
}

/// The custom columns the table shows: those the user ticked that still exist, in the order of the
/// definitions.
pub fn shown_columns<'a>(defs: &'a [ColumnDef], ticked: &[String]) -> Vec<&'a ColumnDef> {
    defs.iter()
        .filter(|d| ticked.iter().any(|t| t == &d.label))
        .collect()
}

/// The ticked columns after switching one on or off.
pub fn toggle(ticked: &[String], defs: &[ColumnDef], label: &str, on: bool) -> Vec<String> {
    defs.iter()
        .filter(|d| {
            if d.label == label {
                on
            } else {
                ticked.contains(&d.label)
            }
        })
        .map(|d| d.label.clone())
        .collect()
}

/// A sort by a custom column, remembered with the built-in sort that was set when it was chosen.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CustomSort {
    pub label: String,
    pub ascending: bool,
    pub base_key: SortKey,
    pub base_ascending: bool,
}

/// Whether a remembered custom sort still applies: its column exists and the built-in sort was
/// not changed since (choosing another sort in the View options menu replaces it).
pub fn sort_applies(s: &CustomSort, defs: &[ColumnDef], key: SortKey, ascending: bool) -> bool {
    s.base_key == key && s.base_ascending == ascending && defs.iter().any(|d| d.label == s.label)
}

/// A table row's value of a column, for sorting: `None` when the book has none.
pub fn row_value<'a>(custom: &'a BTreeMap<String, String>, label: &str) -> Option<&'a str> {
    custom
        .get(label)
        .map(String::as_str)
        .filter(|s| !s.is_empty())
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

    fn defs() -> Vec<ColumnDef> {
        let mut mood = def("mood", ColumnKind::Enumeration);
        mood.values = vec!["Calm".into(), "Tense".into()];
        vec![
            def("pages", ColumnKind::Integer),
            mood,
            def("done", ColumnKind::Bool),
            def("both", ColumnKind::Composite),
        ]
    }

    #[test]
    fn choice_rows_offer_no_value_first() {
        let d = defs();
        assert_eq!(choices(&d[1]), vec!["", "Calm", "Tense"]);
        assert_eq!(choices(&d[2]), vec!["", "true", "false"]);
        assert!(is_choice(&d[1]) && is_choice(&d[2]) && !is_choice(&d[0]));
        let r = choices(&def("score", ColumnKind::Rating));
        assert_eq!((r.len(), r[1].as_str(), r[10].as_str()), (11, "0.5", "5"));
    }

    #[test]
    fn a_form_reports_only_what_changed() {
        let d = defs();
        let mut orig = BTreeMap::new();
        orig.insert("pages".to_string(), "300".to_string());
        orig.insert("done".to_string(), "true".to_string());
        let entered = vec![
            ("pages".to_string(), " 300 ".to_string()),
            ("mood".to_string(), "tense".to_string()),
            ("done".to_string(), String::new()),
            ("both".to_string(), "ignored".to_string()),
        ];
        let c = changed(&d, &orig, &entered).unwrap();
        assert_eq!(
            c,
            vec![
                ("mood".to_string(), "Tense".to_string()),
                ("done".to_string(), String::new())
            ],
            "300 is the same, clearing is a change, composites are skipped"
        );
        let bad = vec![("pages".to_string(), "lots".to_string())];
        let e = changed(&d, &orig, &bad).unwrap_err();
        assert!(e.starts_with("PAGES:"), "{e}");
    }

    #[test]
    fn the_bulk_editor_changes_only_filled_fields() {
        let d = defs();
        let entered = vec![
            ("pages".to_string(), String::new()),
            ("mood".to_string(), "Calm".to_string()),
        ];
        assert_eq!(
            bulk_changes(&d, &entered).unwrap(),
            vec![("mood".to_string(), "Calm".to_string())]
        );
        assert!(bulk_changes(&d, &[("mood".to_string(), "sleepy".to_string())]).is_err());
        assert!(bulk_changes(&d, &[]).unwrap().is_empty());
    }

    #[test]
    fn cells_show_words_and_stars() {
        let d = defs();
        assert_eq!(cell_text(&d[2], "true", "Yes", "No"), "Yes");
        assert_eq!(cell_text(&d[2], "false", "Yes", "No"), "No");
        assert_eq!(cell_text(&d[2], "", "Yes", "No"), "");
        assert_eq!(cell_text(&d[0], "300", "Yes", "No"), "300");
        let r = def("score", ColumnKind::Rating);
        assert_eq!(cell_text(&r, "4.5", "Yes", "No"), "★★★★½");
    }

    #[test]
    fn the_table_shows_the_ticked_columns_that_exist() {
        let d = defs();
        let ticked = vec!["mood".to_string(), "gone".to_string(), "pages".to_string()];
        let shown: Vec<&str> = shown_columns(&d, &ticked)
            .iter()
            .map(|c| c.label.as_str())
            .collect();
        assert_eq!(
            shown,
            vec!["pages", "mood"],
            "in the order of the definitions"
        );
        assert_eq!(
            toggle(&ticked, &d, "done", true),
            vec!["pages", "mood", "done"]
        );
        assert_eq!(toggle(&ticked, &d, "pages", false), vec!["mood"]);
    }

    #[test]
    fn a_custom_sort_ends_when_the_builtin_sort_changes() {
        let d = defs();
        let s = CustomSort {
            label: "pages".into(),
            ascending: false,
            base_key: SortKey::Title,
            base_ascending: true,
        };
        assert!(sort_applies(&s, &d, SortKey::Title, true));
        assert!(!sort_applies(&s, &d, SortKey::Author, true));
        assert!(!sort_applies(&s, &d, SortKey::Title, false));
        assert!(
            !sort_applies(&s, &d[1..2], SortKey::Title, true),
            "column removed"
        );
        let mut m = BTreeMap::new();
        m.insert("a".to_string(), String::new());
        assert_eq!(row_value(&m, "a"), None);
        assert_eq!(row_value(&m, "b"), None);
    }
}
