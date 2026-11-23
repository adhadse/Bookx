//! Which metadata source is preferred for which field (Preferences → Library → Metadata sources).
//! Unset fields use the built-in order; the choice only decides which answer wins when several
//! sources know a field.

use std::collections::BTreeMap;
use std::sync::Mutex;

use bookx_sources::authors::{NameOrder, keep_as_written};
use bookx_sources::sources::{Field, FieldPriority, SOURCE_IDS, SourceSet};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SourcePriority {
    /// Field key → source ids, preferred first.
    pub order: BTreeMap<String, Vec<String>>,
    /// `auto`, `last-first` or `as-written` (see `bookx_sources::authors::NameOrder`).
    pub name_order: String,
}

impl SourcePriority {
    /// Prefers `source` for `field`; `None` goes back to the built-in order. Unknown names are refused.
    pub fn set(&mut self, field: Field, source: Option<&str>) -> bool {
        match source {
            None => {
                self.order.remove(field.key());
                true
            }
            Some(s) if SOURCE_IDS.contains(&s) => {
                self.order.insert(field.key().into(), vec![s.to_string()]);
                true
            }
            Some(_) => false,
        }
    }

    pub fn preferred(&self, field: Field) -> Option<&str> {
        self.order
            .get(field.key())
            .and_then(|l| l.first())
            .map(String::as_str)
            .filter(|s| SOURCE_IDS.contains(s))
    }

    /// The order the lookup uses: built-in defaults with these choices on top.
    pub fn effective(&self) -> FieldPriority {
        SourceSet::standard_priority().overlay(&self.order)
    }
}

impl SourcePriority {
    pub fn name_order(&self) -> NameOrder {
        NameOrder::from_key(&self.name_order)
    }
}

/// The author sort string of `authors` of a book in `lang`: `Last, First` per author unless the
/// name order setting (or the script of the name) says the family name already comes first.
pub fn author_sort_for(authors: &[String], lang: &str, order: NameOrder) -> String {
    authors
        .iter()
        .map(|a| {
            if keep_as_written(a, lang, order) {
                a.trim().to_string()
            } else {
                crate::core::util::author_sort(a)
            }
        })
        .collect::<Vec<_>>()
        .join(" & ")
}

/// The name order lookups use now.
pub fn current_name_order() -> NameOrder {
    CURRENT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(SourcePriority::name_order)
        .unwrap_or_default()
}

static CURRENT: Mutex<Option<SourcePriority>> = Mutex::new(None);

/// Makes `p` the order every lookup uses (called at start-up and when the setting changes).
pub fn install(p: &SourcePriority) {
    *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(p.clone());
}

/// The order lookups use now.
pub fn current() -> FieldPriority {
    CURRENT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(SourcePriority::effective)
        .unwrap_or_else(SourceSet::standard_priority)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_are_set_cleared_and_checked() {
        let mut p = SourcePriority::default();
        assert!(p.set(Field::Description, Some("openlibrary")));
        assert_eq!(p.preferred(Field::Description), Some("openlibrary"));
        assert!(!p.set(Field::Tags, Some("bing")));
        assert_eq!(p.preferred(Field::Tags), None);
        assert!(p.set(Field::Description, None));
        assert!(p.order.is_empty());
        // a hand-edited file with a bad id never reaches the lookup
        p.order.insert("series".into(), vec!["bing".into()]);
        assert_eq!(p.preferred(Field::Series), None);
        assert_eq!(p.effective().preferred(Field::Series), Some("wikidata"));
    }

    #[test]
    fn author_sort_follows_the_name_order() {
        let a = vec!["Ada Lovelace".to_string(), "村上 春樹".to_string()];
        assert_eq!(
            author_sort_for(&a, "en", NameOrder::Auto),
            "Lovelace, Ada & 村上 春樹"
        );
        assert_eq!(
            author_sort_for(&["Murakami Haruki".into()], "ja", NameOrder::Auto),
            "Murakami Haruki"
        );
        assert_eq!(
            author_sort_for(&["Murakami Haruki".into()], "ja", NameOrder::LastFirst),
            "Haruki, Murakami"
        );
        assert_eq!(
            author_sort_for(&["Ada Lovelace".into()], "en", NameOrder::AsWritten),
            "Ada Lovelace"
        );
        let mut p = SourcePriority::default();
        assert_eq!(p.name_order(), NameOrder::Auto);
        p.name_order = "as-written".into();
        assert_eq!(p.name_order(), NameOrder::AsWritten);
    }

    #[test]
    fn old_settings_load_without_the_field() {
        let p: SourcePriority = serde_json::from_str("{}").unwrap();
        assert!(p.order.is_empty());
    }
}
