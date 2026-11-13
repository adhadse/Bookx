//! What the reader remembers about the two floating cards (read aloud and speed reading) of a book:
//! which are open, where they are, and the exact place they stopped. Pure data with its own
//! (de)serialisation rules; `Config` keeps one [`BookCards`] per book and the reader fills and reads
//! it. Nothing here knows GTK.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Cards a book may remember at most (older books are forgotten first), so the config stays small.
pub const MAX_BOOKS: usize = 300;
/// Longest sentence text kept for display.
const MAX_TEXT: usize = 160;

/// Where a card sits, as fractions (0..=1) of the free room inside the reader window: `x` 0 is the
/// left edge and 1 the right edge, `y` 0 the top and 1 the bottom. Fractions survive resizing.
pub type Place = (f64, f64);

/// Default place of the read-aloud card: centred at the bottom.
pub const TTS_PLACE: Place = (0.5, 1.0);
/// Default place of the speed-reading card: centred, a little higher.
pub const RSVP_PLACE: Place = (0.5, 0.8);

fn fix_place(p: Place, default: Place) -> Place {
    let f = |v: f64, d: f64| if v.is_finite() { v.clamp(0.0, 1.0) } else { d };
    (f(p.0, default.0), f(p.1, default.1))
}

/// The read-aloud card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TtsCard {
    pub x: f64,
    pub y: f64,
    /// The small player (the default); `false` is the expanded one.
    pub compact: bool,
    /// Spine index and path of the chapter being read; the path finds the chapter again when the
    /// book changed.
    pub chapter: usize,
    pub chapter_path: String,
    /// Sentence number, number of sentences and char offset of the sentence start in the chapter.
    pub sentence: usize,
    pub sentences: usize,
    pub offset: i32,
    /// The sentence, for showing it while the card waits.
    pub text: String,
    pub speed: f64,
    pub voice: String,
}

impl Default for TtsCard {
    fn default() -> Self {
        TtsCard {
            x: TTS_PLACE.0,
            y: TTS_PLACE.1,
            compact: true,
            chapter: 0,
            chapter_path: String::new(),
            sentence: 0,
            sentences: 0,
            offset: 0,
            text: String::new(),
            speed: 1.0,
            voice: String::new(),
        }
    }
}

/// The speed-reading card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RsvpCard {
    pub x: f64,
    pub y: f64,
    pub chapter: usize,
    pub chapter_path: String,
    /// Index of the word in the chapter, number of words, and char offset of the word.
    pub word: usize,
    pub words: usize,
    pub offset: i32,
    /// The word, for showing it while the card waits.
    pub text: String,
    pub wpm: u32,
}

impl Default for RsvpCard {
    fn default() -> Self {
        RsvpCard {
            x: RSVP_PLACE.0,
            y: RSVP_PLACE.1,
            chapter: 0,
            chapter_path: String::new(),
            word: 0,
            words: 0,
            offset: 0,
            text: String::new(),
            wpm: 300,
        }
    }
}

/// Which card is drawn on top.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Top {
    #[default]
    Tts,
    Rsvp,
}

/// The cards of one book. A card the user closed is `None` and is not restored.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BookCards {
    pub tts: Option<TtsCard>,
    pub rsvp: Option<RsvpCard>,
    pub top: Top,
}

impl BookCards {
    pub fn is_empty(&self) -> bool {
        self.tts.is_none() && self.rsvp.is_none()
    }

    /// The same cards with every value made sane (fractions inside 0..=1, text short, speeds in range).
    pub fn clamped(&self) -> BookCards {
        let mut c = self.clone();
        if let Some(t) = c.tts.as_mut() {
            (t.x, t.y) = fix_place((t.x, t.y), TTS_PLACE);
            t.speed = if t.speed.is_finite() {
                t.speed.clamp(0.5, 2.5)
            } else {
                1.0
            };
            t.offset = t.offset.max(0);
            t.text = short(&t.text);
            if t.sentences > 0 {
                t.sentence = t.sentence.min(t.sentences - 1);
            }
        }
        if let Some(r) = c.rsvp.as_mut() {
            (r.x, r.y) = fix_place((r.x, r.y), RSVP_PLACE);
            r.wpm = r.wpm.clamp(50, 1000);
            r.offset = r.offset.max(0);
            r.text = short(&r.text);
            if r.words > 0 {
                r.word = r.word.min(r.words - 1);
            }
        }
        c
    }
}

fn short(s: &str) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    one.chars().take(MAX_TEXT).collect()
}

/// Cards of every book, by catalogue id (a string, so it is a JSON object).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CardsStore {
    books: BTreeMap<String, BookCards>,
}

impl CardsStore {
    /// The cards to restore for `book`, sanitised; `None` when there are none.
    pub fn get(&self, book: u64) -> Option<BookCards> {
        self.books
            .get(&book.to_string())
            .map(BookCards::clamped)
            .filter(|c| !c.is_empty())
    }

    /// Remembers `cards` for `book`; no open card forgets the book.
    pub fn set(&mut self, book: u64, cards: BookCards) {
        let key = book.to_string();
        if cards.is_empty() {
            self.books.remove(&key);
            return;
        }
        self.books.insert(key.clone(), cards.clamped());
        while self.books.len() > MAX_BOOKS {
            // ids grow with time: the smallest (oldest) number goes first
            let oldest = self
                .books
                .keys()
                .min_by_key(|k| k.parse::<u64>().unwrap_or(0))
                .cloned();
            match oldest {
                Some(k) if k != key => {
                    self.books.remove(&k);
                }
                _ => break,
            }
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.books.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.books.is_empty()
    }
}

/// Where a saved chapter is in the book as it is now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved {
    pub chapter: usize,
    /// The saved place inside the chapter still means something (the chapter was found by its path
    /// or the structure is unchanged). When false, start at the top of `chapter`.
    pub exact: bool,
}

/// Finds the chapter a card was in. The path wins; when the path is gone the saved index is used if
/// it exists (the nearest chapter start, from the top); an index beyond the end falls back to the
/// last chapter. `None` when the book has no chapters.
pub fn resolve_chapter(saved_index: usize, saved_path: &str, spine: &[String]) -> Option<Resolved> {
    if spine.is_empty() {
        return None;
    }
    if !saved_path.is_empty() {
        if let Some(i) = spine.iter().position(|p| p == saved_path) {
            return Some(Resolved {
                chapter: i,
                exact: true,
            });
        }
        return Some(Resolved {
            chapter: saved_index.min(spine.len() - 1),
            exact: false,
        });
    }
    // an older save without a path: trust the index only when it still exists
    Some(Resolved {
        chapter: saved_index.min(spine.len() - 1),
        exact: saved_index < spine.len(),
    })
}

/// The char offset to start from in a chapter of `len` chars: the saved one, or the top when the
/// place no longer means anything or lies beyond the end.
pub fn resolve_offset(saved: i32, exact: bool, len: i32) -> i32 {
    if !exact || saved < 0 || saved >= len {
        0
    } else {
        saved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spine() -> Vec<String> {
        ["c1.xhtml", "c2.xhtml", "c3.xhtml"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn both() -> BookCards {
        BookCards {
            tts: Some(TtsCard {
                x: 0.2,
                y: 0.9,
                compact: false,
                chapter: 1,
                chapter_path: "c2.xhtml".into(),
                sentence: 12,
                sentences: 40,
                offset: 1530,
                text: "It was a dark night.".into(),
                speed: 1.25,
                voice: "af_heart".into(),
            }),
            rsvp: Some(RsvpCard {
                x: 0.7,
                y: 0.1,
                chapter: 2,
                chapter_path: "c3.xhtml".into(),
                word: 311,
                words: 900,
                offset: 2100,
                text: "midnight".into(),
                wpm: 450,
            }),
            top: Top::Rsvp,
        }
    }

    #[test]
    fn cards_survive_a_round_trip_through_json() {
        let mut s = CardsStore::default();
        s.set(42, both());
        let json = serde_json::to_string(&s).unwrap();
        let back: CardsStore = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.get(42).unwrap(), both());
        assert!(back.get(43).is_none());
    }

    #[test]
    fn old_and_partial_files_still_load() {
        // a card from before some fields existed
        let s: CardsStore =
            serde_json::from_str(r#"{"7":{"tts":{"chapter":3,"sentence":5}}}"#).unwrap();
        let c = s.get(7).unwrap();
        let t = c.tts.unwrap();
        assert_eq!((t.chapter, t.sentence), (3, 5));
        assert!(t.compact, "compact is the default");
        assert_eq!((t.x, t.y), TTS_PLACE);
        assert_eq!(t.speed, 1.0);
        assert!(c.rsvp.is_none());
        assert_eq!(c.top, Top::Tts);
        // an empty config key and unknown fields
        let s: CardsStore = serde_json::from_str("{}").unwrap();
        assert!(s.is_empty());
        let s: CardsStore =
            serde_json::from_str(r#"{"9":{"rsvp":{"wpm":300,"future":true},"extra":1}}"#).unwrap();
        assert_eq!(s.get(9).unwrap().rsvp.unwrap().wpm, 300);
    }

    #[test]
    fn values_are_clamped_when_read() {
        let mut c = both();
        let t = c.tts.as_mut().unwrap();
        t.x = -3.0;
        t.y = f64::NAN;
        t.speed = 99.0;
        t.sentence = 400;
        t.text = "word ".repeat(200);
        let r = c.rsvp.as_mut().unwrap();
        r.x = 5.0;
        r.wpm = 1;
        r.word = 5000;
        let mut s = CardsStore::default();
        s.set(1, c);
        let c = s.get(1).unwrap();
        let t = c.tts.unwrap();
        assert_eq!((t.x, t.y), (0.0, TTS_PLACE.1));
        assert_eq!(t.speed, 2.5);
        assert_eq!(t.sentence, 39, "the last sentence");
        assert!(t.text.chars().count() <= 160);
        let r = c.rsvp.unwrap();
        assert_eq!(r.x, 1.0);
        assert_eq!(r.wpm, 50);
        assert_eq!(r.word, 899);
    }

    #[test]
    fn a_book_without_open_cards_is_forgotten() {
        let mut s = CardsStore::default();
        s.set(5, both());
        assert_eq!(s.len(), 1);
        s.set(5, BookCards::default());
        assert!(s.is_empty());
        // only one card closed: the other stays
        let mut c = both();
        c.tts = None;
        s.set(5, c);
        let c = s.get(5).unwrap();
        assert!(c.tts.is_none() && c.rsvp.is_some());
    }

    #[test]
    fn the_store_stays_small_by_forgetting_the_oldest_books() {
        let mut s = CardsStore::default();
        for id in 1..=(MAX_BOOKS as u64 + 20) {
            s.set(id, both());
        }
        assert_eq!(s.len(), MAX_BOOKS);
        assert!(s.get(1).is_none());
        assert!(s.get(MAX_BOOKS as u64 + 20).is_some());
    }

    #[test]
    fn chapters_are_found_by_path_then_by_index() {
        let sp = spine();
        assert_eq!(
            resolve_chapter(0, "c3.xhtml", &sp),
            Some(Resolved {
                chapter: 2,
                exact: true
            }),
            "the path wins over a stale index"
        );
        // the chapter is gone: the same index, from the top
        assert_eq!(
            resolve_chapter(1, "gone.xhtml", &sp),
            Some(Resolved {
                chapter: 1,
                exact: false
            })
        );
        // the book got shorter: the last chapter, from the top
        assert_eq!(
            resolve_chapter(9, "gone.xhtml", &sp),
            Some(Resolved {
                chapter: 2,
                exact: false
            })
        );
        // an old save without a path
        assert_eq!(
            resolve_chapter(1, "", &sp),
            Some(Resolved {
                chapter: 1,
                exact: true
            })
        );
        assert!(!resolve_chapter(7, "", &sp).unwrap().exact);
        assert_eq!(resolve_chapter(0, "c1.xhtml", &[]), None);
    }

    #[test]
    fn offsets_beyond_the_end_or_in_a_changed_chapter_start_at_the_top() {
        assert_eq!(resolve_offset(500, true, 1000), 500);
        assert_eq!(resolve_offset(500, false, 1000), 0);
        assert_eq!(resolve_offset(1000, true, 1000), 0);
        assert_eq!(resolve_offset(5000, true, 1000), 0);
        assert_eq!(resolve_offset(-4, true, 1000), 0);
        assert_eq!(resolve_offset(0, true, 0), 0);
    }
}
