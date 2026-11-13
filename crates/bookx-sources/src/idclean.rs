//! Identifier cleanup suggestions for imported books (roadmap PP-G-1, import quality).
//!
//! `ISBN: 978-0-441-17271-9`, `urn:isbn:…`, a Goodreads URL or an ISBN-10 are all fine data, but
//! stored in many shapes. [`suggest`] lists what could be tidied and [`apply`] does the ones the
//! user accepted. A suggestion never invents data: a checksum that is wrong is only reported.

use bookx_core::opf::{Identifier, Metadata};

use crate::identifiers::{self, Id, IdKind};
use crate::isbn;

/// What a suggestion does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Replace the identifier at `index` by `scheme:value`.
    Rewrite {
        index: usize,
        scheme: String,
        value: String,
    },
    /// Drop the identifier at `index` (the same one is listed twice).
    Remove { index: usize },
    /// Nothing can be done automatically; the user should look.
    Report,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub action: Action,
    pub text: String,
}

impl Suggestion {
    pub fn fixable(&self) -> bool {
        self.action != Action::Report
    }
}

fn shown(i: &Identifier) -> String {
    if i.scheme.is_empty() {
        i.value.clone()
    } else {
        format!("{}:{}", i.scheme, i.value)
    }
}

/// The ways the identifiers of `m` could be tidied.
pub fn suggest(m: &Metadata) -> Vec<Suggestion> {
    let mut out = Vec::new();
    let mut seen: Vec<Id> = Vec::new();
    for (index, i) in m.identifiers.iter().enumerate() {
        let id = Id::from_identifier(i).or_else(|| identifiers::parse(&i.value));
        let Some(id) = id else {
            if IdKind::from_scheme(&i.scheme) == Some(IdKind::Isbn) {
                out.push(Suggestion {
                    action: Action::Report,
                    text: format!(
                        "{} is not a valid ISBN (wrong length or check digit)",
                        shown(i)
                    ),
                });
            }
            continue;
        };
        if seen.contains(&id) {
            out.push(Suggestion {
                action: Action::Remove { index },
                text: format!("{} is listed twice", shown(i)),
            });
            continue;
        }
        let mut target = id.clone();
        if target.kind == IdKind::Isbn && isbn::is_valid_isbn10(&target.value) {
            if let Some(v13) = isbn::to_isbn13(&target.value) {
                target.value = v13;
            }
        }
        seen.push(id);
        seen.push(target.clone());
        let wanted_scheme = target.kind.scheme();
        if i.scheme != wanted_scheme || i.value != target.value {
            out.push(Suggestion {
                text: format!("{} → {}:{}", shown(i), wanted_scheme, target.value),
                action: Action::Rewrite {
                    index,
                    scheme: wanted_scheme.to_string(),
                    value: target.value,
                },
            });
        }
    }
    out
}

/// `m` with the fixable `suggestions` applied.
pub fn apply(m: &Metadata, suggestions: &[Suggestion]) -> Metadata {
    let mut out = m.clone();
    let mut remove: Vec<usize> = Vec::new();
    for s in suggestions {
        match &s.action {
            Action::Rewrite {
                index,
                scheme,
                value,
            } => {
                if let Some(i) = out.identifiers.get_mut(*index) {
                    i.scheme = scheme.clone();
                    i.value = value.clone();
                }
            }
            Action::Remove { index } => remove.push(*index),
            Action::Report => {}
        }
    }
    remove.sort_unstable_by(|a, b| b.cmp(a));
    for i in remove {
        if i < out.identifiers.len() {
            out.identifiers.remove(i);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(ids: &[(&str, &str)]) -> Metadata {
        Metadata {
            identifiers: ids
                .iter()
                .map(|(s, v)| Identifier {
                    scheme: s.to_string(),
                    value: v.to_string(),
                    xml_id: None,
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn clean_identifiers_need_nothing() {
        assert!(suggest(&md(&[("isbn", "9780441172719"), ("goodreads", "234225")])).is_empty());
        assert!(suggest(&md(&[])).is_empty());
    }

    #[test]
    fn hyphens_and_odd_schemes_are_normalised() {
        let m = md(&[
            ("ISBN", "978-0-441-17271-9"),
            ("", "urn:isbn:9780441172719"),
        ]);
        let s = suggest(&m);
        assert_eq!(s.len(), 2, "{s:?}");
        assert_eq!(
            s[0].action,
            Action::Rewrite {
                index: 0,
                scheme: "isbn".into(),
                value: "9780441172719".into()
            }
        );
        assert!(s[1].text.contains("listed twice"));
        let fixed = apply(&m, &s);
        assert_eq!(fixed.identifiers.len(), 1);
        assert_eq!(fixed.identifiers[0].scheme, "isbn");
        assert_eq!(fixed.identifiers[0].value, "9780441172719");
        assert!(suggest(&fixed).is_empty(), "idempotent");
    }

    #[test]
    fn an_isbn_10_becomes_an_isbn_13() {
        let m = md(&[("isbn", "0-441-17271-7")]);
        let s = suggest(&m);
        assert_eq!(s.len(), 1);
        let fixed = apply(&m, &s);
        assert_eq!(fixed.identifiers[0].value, "9780441172719");
    }

    #[test]
    fn a_bad_check_digit_is_only_reported() {
        let s = suggest(&md(&[("isbn", "9780441172718")]));
        assert_eq!(s.len(), 1);
        assert!(!s[0].fixable());
        assert!(s[0].text.contains("not a valid ISBN"));
        let m = md(&[("isbn", "9780441172718")]);
        assert_eq!(apply(&m, &s), m, "nothing changes");
    }

    #[test]
    fn a_store_url_in_the_value_becomes_the_id() {
        let m = md(&[("", "https://www.goodreads.com/book/show/234225.Dune")]);
        let s = suggest(&m);
        assert_eq!(s.len(), 1, "{s:?}");
        let fixed = apply(&m, &s);
        assert_eq!(
            (
                fixed.identifiers[0].scheme.as_str(),
                fixed.identifiers[0].value.as_str()
            ),
            ("goodreads", "234225")
        );
    }
}
