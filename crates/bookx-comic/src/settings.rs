//! Reading modes and what the reader remembers: global defaults plus a per-book override for the
//! things that depend on the book (mode, fit, reading direction).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// How pages are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// One page at a time.
    #[default]
    Single,
    /// Two pages side by side like an open book.
    Spread,
    /// One long vertical strip (webtoon).
    Webtoon,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Single => "single",
            Mode::Spread => "spread",
            Mode::Webtoon => "webtoon",
        }
    }
    pub fn from_name(s: &str) -> Option<Mode> {
        match s {
            "single" => Some(Mode::Single),
            "spread" => Some(Mode::Spread),
            "webtoon" => Some(Mode::Webtoon),
            _ => None,
        }
    }
}

/// How the page is scaled to the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    Width,
    Height,
    #[default]
    Page,
    Original,
}

impl Fit {
    pub fn name(self) -> &'static str {
        match self {
            Fit::Width => "width",
            Fit::Height => "height",
            Fit::Page => "page",
            Fit::Original => "original",
        }
    }
    pub fn from_name(s: &str) -> Option<Fit> {
        match s {
            "width" => Some(Fit::Width),
            "height" => Some(Fit::Height),
            "page" => Some(Fit::Page),
            "original" => Some(Fit::Original),
            _ => None,
        }
    }
}

/// Settings that apply to every comic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Global {
    pub mode: Mode,
    pub fit: Fit,
    /// Two-page spreads: the cover stays on a page of its own.
    pub cover_alone: bool,
    /// Two-page spreads: a wide page (a double page already) stands alone.
    pub wide_alone: bool,
    /// Guided view: Space steps through the panels.
    pub guided: bool,
    /// Show the pages in grey (e-ink screens, saves toner in print).
    pub grey: bool,
    /// Animate the move to a panel (reduced motion and e-ink switch it off anyway).
    pub animate: bool,
    /// The thumbnail strip is open.
    pub thumbs: bool,
}

impl Default for Global {
    fn default() -> Self {
        Global {
            mode: Mode::Single,
            fit: Fit::Page,
            cover_alone: true,
            wide_alone: true,
            guided: false,
            grey: false,
            animate: true,
            thumbs: false,
        }
    }
}

/// What is remembered for one book.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BookView {
    pub mode: Option<Mode>,
    pub fit: Option<Fit>,
    /// `Some(true)` right to left (manga), `Some(false)` left to right; `None` follows the book.
    pub rtl: Option<bool>,
    pub guided: Option<bool>,
}

/// The settings in force for the book that is open.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Effective {
    pub mode: Mode,
    pub fit: Fit,
    pub rtl: bool,
    pub guided: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
    pub global: Global,
    /// Keyed by the book id.
    pub books: BTreeMap<String, BookView>,
}

impl Store {
    pub fn from_json(text: &str) -> Store {
        serde_json::from_str(text).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// `book_rtl` is what the book itself says (ComicInfo `Manga`, the page progression).
    pub fn effective(&self, id: u64, book_rtl: bool) -> Effective {
        let b = self.books.get(&id.to_string()).copied().unwrap_or_default();
        Effective {
            mode: b.mode.unwrap_or(self.global.mode),
            fit: b.fit.unwrap_or(self.global.fit),
            rtl: b.rtl.unwrap_or(book_rtl),
            guided: b.guided.unwrap_or(self.global.guided),
        }
    }

    pub fn book_mut(&mut self, id: u64) -> &mut BookView {
        self.books.entry(id.to_string()).or_default()
    }

    /// Whether the book has choices of its own (what "Use the defaults for this book" would forget).
    pub fn has_own(&self, id: u64) -> bool {
        self.books
            .get(&id.to_string())
            .is_some_and(|b| *b != BookView::default())
    }

    /// Forget the per-book choices of a book.
    pub fn forget(&mut self, id: u64) {
        self.books.remove(&id.to_string());
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_book_has_own_choices_only_after_one_is_made() {
        let mut s = super::Store::default();
        assert!(!s.has_own(7));
        s.book_mut(7);
        assert!(!s.has_own(7), "an empty entry is the defaults");
        s.book_mut(7).guided = Some(true);
        assert!(s.has_own(7));
        s.forget(7);
        assert!(!s.has_own(7));
    }

    use super::*;

    #[test]
    fn book_choices_beat_the_defaults_and_the_book() {
        let mut s = Store::default();
        let e = s.effective(7, true);
        assert!(e.rtl);
        assert_eq!(e.mode, Mode::Single);
        s.book_mut(7).rtl = Some(false);
        s.book_mut(7).mode = Some(Mode::Webtoon);
        let e = s.effective(7, true);
        assert!(!e.rtl, "the user's toggle wins over ComicInfo");
        assert_eq!(e.mode, Mode::Webtoon);
        assert!(s.effective(8, true).rtl, "other books are not affected");
        s.forget(7);
        assert!(s.effective(7, true).rtl);
    }

    #[test]
    fn json_round_trip_and_garbage() {
        let mut s = Store::default();
        s.book_mut(3).fit = Some(Fit::Width);
        s.global.grey = true;
        let back = Store::from_json(&s.to_json());
        assert_eq!(back, s);
        assert_eq!(Store::from_json("{ nonsense"), Store::default());
        assert_eq!(
            Store::from_json("{\"global\":{\"mode\":\"spread\"}}")
                .global
                .mode,
            Mode::Spread
        );
    }

    #[test]
    fn names_round_trip() {
        for m in [Mode::Single, Mode::Spread, Mode::Webtoon] {
            assert_eq!(Mode::from_name(m.name()), Some(m));
        }
        for f in [Fit::Width, Fit::Height, Fit::Page, Fit::Original] {
            assert_eq!(Fit::from_name(f.name()), Some(f));
        }
        assert_eq!(Mode::from_name("x"), None);
    }
}
