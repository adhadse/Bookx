//! Style tweaks of the reader (roadmap R21): the reader's own layer on top of the book's styling.
//!
//! Every field is an override: `None` leaves the preference (or the book) alone. Tweaks exist
//! for all books and, on top of that, per book; a book with tweaks of its own ignores the
//! general ones. No GTK in here, so everything is unit tested.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ui::canvas::CanvasStyle;

/// Longest user style sheet kept.
pub const MAX_CSS_CHARS: usize = 20_000;

/// Hyphenation language choices of the tweaks dialog: `(value, label)`. An empty value means
/// the language of the book, `off` switches hyphenation off.
pub const HYPHEN_LANGUAGES: [(&str, &str); 10] = [
    ("", "Language of the book"),
    ("off", "Off"),
    ("en-US", "English"),
    ("de", "German"),
    ("fr", "French"),
    ("es", "Spanish"),
    ("it", "Italian"),
    ("pt", "Portuguese"),
    ("nl", "Dutch"),
    ("ru", "Russian"),
];

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleTweaks {
    /// Line height as a multiple of the font size.
    pub line_height: Option<f64>,
    /// Page margin in pixels.
    pub margin: Option<f64>,
    /// Space after a paragraph, in em.
    pub paragraph_spacing: Option<f64>,
    /// Font family used instead of the one chosen in the text menu.
    pub font: Option<String>,
    /// `Some(true)` justifies every paragraph, `Some(false)` sets them ragged.
    pub justify: Option<bool>,
    /// Knuth-Plass line breaking on or off.
    pub optimal_breaks: Option<bool>,
    /// `None` = the language of the book, `Some("off")` = no hyphenation, else a language tag.
    pub hyphenation: Option<String>,
    /// The reader's own style sheet, applied after the book's.
    pub user_css: String,
}

impl StyleTweaks {
    pub fn is_empty(&self) -> bool {
        *self == StyleTweaks::default()
    }

    /// The same tweaks with every value inside its sane range and the style sheet capped.
    pub fn clamped(&self) -> StyleTweaks {
        let mut t = self.clone();
        t.line_height = t
            .line_height
            .filter(|v| v.is_finite())
            .map(|v| v.clamp(1.0, 3.0));
        t.margin = t
            .margin
            .filter(|v| v.is_finite())
            .map(|v| v.clamp(0.0, 200.0));
        t.paragraph_spacing = t
            .paragraph_spacing
            .filter(|v| v.is_finite())
            .map(|v| v.clamp(0.0, 3.0));
        t.font = t
            .font
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty());
        t.hyphenation = t
            .hyphenation
            .map(|h| h.trim().to_string())
            .filter(|h| !h.is_empty());
        if t.user_css.chars().count() > MAX_CSS_CHARS {
            t.user_css = t.user_css.chars().take(MAX_CSS_CHARS).collect();
        }
        t
    }

    /// Lays the tweaks over the style made from the preferences.
    pub fn apply(&self, style: &mut CanvasStyle) {
        let t = self.clamped();
        if let Some(v) = t.line_height {
            style.line_spacing = v;
        }
        if let Some(v) = t.margin {
            style.margin = v;
        }
        if let Some(v) = t.paragraph_spacing {
            style.paragraph_spacing = Some(v);
        }
        if let Some(f) = t.font {
            style.font_family = f;
        }
        if let Some(j) = t.justify {
            style.justify = j;
        }
        if let Some(o) = t.optimal_breaks {
            style.optimal_breaks = o;
        }
        if t.hyphenation.as_deref() == Some("off") {
            style.hyphenate = false;
        } else if t.hyphenation.is_some() {
            style.hyphenate = true;
        }
    }

    /// The language the hyphenation patterns are picked for.
    pub fn hyphenation_language(&self, book_language: &str) -> String {
        match self.hyphenation.as_deref() {
            Some(l) if l != "off" && !l.is_empty() => l.to_string(),
            _ => book_language.to_string(),
        }
    }

    /// The style sheet to apply to chapters (trimmed, capped).
    pub fn css(&self) -> String {
        self.clamped().user_css.trim().to_string()
    }
}

/// The general tweaks and the ones of single books.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TweakStore {
    pub general: StyleTweaks,
    /// Keyed by the id of the book in the library.
    pub books: BTreeMap<String, StyleTweaks>,
}

impl TweakStore {
    /// What applies to `book`: its own tweaks when it has any, else the general ones.
    pub fn effective(&self, book: u64) -> StyleTweaks {
        self.books
            .get(&book.to_string())
            .unwrap_or(&self.general)
            .clamped()
    }

    #[cfg(test)]
    pub fn has_own(&self, book: u64) -> bool {
        self.books.contains_key(&book.to_string())
    }

    /// Stores `tweaks` for one book (`per_book`) or for all of them (the default of books that have
    /// none of their own).
    pub fn set(&mut self, book: u64, per_book: bool, tweaks: StyleTweaks) {
        let tweaks = tweaks.clamped();
        if per_book {
            // an empty set is only "nothing of its own" when the general tweaks are empty too;
            // otherwise the book keeps its (empty) choice and does not fall back to them
            if tweaks.is_empty() && self.general.is_empty() {
                self.books.remove(&book.to_string());
            } else {
                self.books.insert(book.to_string(), tweaks);
            }
        } else {
            self.general = tweaks;
        }
    }

    /// The book forgets its own tweaks; the general ones apply again. True when it had some.
    pub fn reset_book(&mut self, book: u64) -> bool {
        self.books.remove(&book.to_string()).is_some()
    }

    /// The tweaks stored for the scope: the book's own (`per_book`) or the general ones.
    #[cfg(test)]
    pub fn of(&self, book: u64, per_book: bool) -> StyleTweaks {
        if per_book && self.has_own(book) {
            self.books[&book.to_string()].clone()
        } else if per_book {
            StyleTweaks::default()
        } else {
            self.general.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_set_changes_nothing() {
        let mut s = CanvasStyle::default();
        let before = format!("{s:?}");
        StyleTweaks::default().apply(&mut s);
        assert_eq!(format!("{s:?}"), before);
        assert!(StyleTweaks::default().is_empty());
    }

    #[test]
    fn tweaks_override_the_preferences() {
        let mut s = CanvasStyle::default();
        StyleTweaks {
            line_height: Some(1.8),
            margin: Some(100.0),
            paragraph_spacing: Some(1.0),
            font: Some("Sans".into()),
            justify: Some(false),
            optimal_breaks: Some(false),
            hyphenation: Some("off".into()),
            user_css: String::new(),
        }
        .apply(&mut s);
        assert_eq!(s.line_spacing, 1.8);
        assert_eq!(s.margin, 100.0);
        assert_eq!(s.paragraph_spacing, Some(1.0));
        assert_eq!(s.font_family, "Sans");
        assert!(!s.justify && !s.optimal_breaks && !s.hyphenate);
    }

    #[test]
    fn values_are_clamped() {
        let t = StyleTweaks {
            line_height: Some(99.0),
            margin: Some(-5.0),
            paragraph_spacing: Some(f64::NAN),
            font: Some("  ".into()),
            hyphenation: Some(" ".into()),
            ..Default::default()
        }
        .clamped();
        assert_eq!(t.line_height, Some(3.0));
        assert_eq!(t.margin, Some(0.0));
        assert_eq!(t.paragraph_spacing, None);
        assert_eq!(t.font, None);
        assert_eq!(t.hyphenation, None);
    }

    #[test]
    fn the_style_sheet_is_capped() {
        let t = StyleTweaks {
            user_css: "a".repeat(MAX_CSS_CHARS + 50),
            ..Default::default()
        };
        assert_eq!(t.css().chars().count(), MAX_CSS_CHARS);
        let t = StyleTweaks {
            user_css: "  p { color: red }\n".into(),
            ..Default::default()
        };
        assert_eq!(t.css(), "p { color: red }");
    }

    #[test]
    fn hyphenation_language_follows_the_book_unless_chosen() {
        let mut t = StyleTweaks::default();
        assert_eq!(t.hyphenation_language("fr"), "fr");
        t.hyphenation = Some("de".into());
        assert_eq!(t.hyphenation_language("fr"), "de");
        t.hyphenation = Some("off".into());
        assert_eq!(t.hyphenation_language("fr"), "fr");
        let mut s = CanvasStyle::default();
        t.apply(&mut s);
        assert!(!s.hyphenate);
        t.hyphenation = Some("de".into());
        t.apply(&mut s);
        assert!(s.hyphenate, "choosing a language switches hyphenation on");
    }

    #[test]
    fn a_book_with_tweaks_of_its_own_ignores_the_general_ones() {
        let mut store = TweakStore::default();
        store.set(
            0,
            false,
            StyleTweaks {
                margin: Some(10.0),
                ..Default::default()
            },
        );
        assert_eq!(store.effective(7).margin, Some(10.0));
        store.set(
            7,
            true,
            StyleTweaks {
                line_height: Some(2.0),
                ..Default::default()
            },
        );
        assert!(store.has_own(7));
        assert_eq!(store.effective(7).margin, None);
        assert_eq!(store.effective(7).line_height, Some(2.0));
        assert_eq!(store.effective(8).margin, Some(10.0));
        // resetting the book brings the general tweaks back
        assert!(store.reset_book(7));
        assert!(!store.has_own(7));
        assert_eq!(store.effective(7).margin, Some(10.0));
    }

    #[test]
    fn the_store_survives_a_round_trip_and_old_files() {
        let mut store = TweakStore::default();
        store.set(
            3,
            true,
            StyleTweaks {
                user_css: "p { text-indent: 2em }".into(),
                justify: Some(true),
                ..Default::default()
            },
        );
        let json = serde_json::to_string(&store).unwrap();
        assert_eq!(serde_json::from_str::<TweakStore>(&json).unwrap(), store);
        let old: TweakStore = serde_json::from_str("{}").unwrap();
        assert!(old.general.is_empty() && old.books.is_empty());
        let partial: StyleTweaks = serde_json::from_str(r#"{"margin": 12.0}"#).unwrap();
        assert_eq!(partial.margin, Some(12.0));
    }

    #[test]
    fn the_scope_picks_what_is_stored() {
        let mut store = TweakStore::default();
        let t = StyleTweaks {
            margin: Some(30.0),
            ..Default::default()
        };
        store.set(5, false, t.clone());
        assert_eq!(store.of(5, false), t);
        assert!(store.of(5, true).is_empty());
        store.set(
            5,
            true,
            StyleTweaks {
                margin: Some(40.0),
                ..Default::default()
            },
        );
        assert_eq!(store.of(5, true).margin, Some(40.0));
        assert_eq!(store.of(5, false), t);
    }

    #[test]
    fn a_book_may_switch_everything_off_without_falling_back_to_the_general_tweaks() {
        let mut st = TweakStore::default();
        st.general.margin = Some(80.0);
        st.set(7, true, StyleTweaks::default());
        assert!(st.has_own(7));
        assert_eq!(st.effective(7).margin, None);
        assert!(st.reset_book(7));
        assert_eq!(st.effective(7).margin, Some(80.0));
        assert!(!st.reset_book(7));
        // with no general tweaks an empty set is just "nothing of its own"
        let mut plain = TweakStore::default();
        plain.set(7, true, StyleTweaks::default());
        assert!(!plain.has_own(7));
    }
}
