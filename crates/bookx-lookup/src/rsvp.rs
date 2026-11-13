//! RSVP speed-reading schedule and the back/forward jump history.

use anyhow::{bail, Result};

pub const MIN_WPM: u32 = 50;
pub const MAX_WPM: u32 = 2000;

#[derive(Debug, Clone, PartialEq)]
pub struct RsvpConfig {
    pub wpm: u32,
    /// Multiplier after `, ; : — –`.
    pub comma_factor: f64,
    /// Multiplier after `. ! ? …`.
    pub sentence_factor: f64,
    /// Multiplier for the last word of a paragraph (replaces the sentence factor if larger).
    pub paragraph_factor: f64,
    /// Words longer than this (letters/digits) get extra time.
    pub long_word_chars: usize,
    /// Extra time fraction per character beyond `long_word_chars`.
    pub long_word_step: f64,
    pub max_long_factor: f64,
    pub min_ms: u32,
}

impl Default for RsvpConfig {
    fn default() -> Self {
        Self {
            wpm: 300,
            comma_factor: 1.5,
            sentence_factor: 2.2,
            paragraph_factor: 3.0,
            long_word_chars: 8,
            long_word_step: 0.08,
            max_long_factor: 2.0,
            min_ms: 40,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub text: String,
    /// A blank line follows this token.
    pub paragraph_end: bool,
}

/// Splits text into whitespace separated tokens, noting paragraph ends (a gap with two
/// or more line breaks).
pub fn tokenize(text: &str) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::new();
    let mut gap_newlines = 0usize;
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<Token>| {
        if !cur.is_empty() {
            out.push(Token {
                text: std::mem::take(cur),
                paragraph_end: false,
            });
        }
    };
    for c in text.chars() {
        if c.is_whitespace() {
            flush(&mut cur, &mut out);
            if c == '\n' {
                gap_newlines += 1;
                if gap_newlines == 2 {
                    if let Some(last) = out.last_mut() {
                        last.paragraph_end = true;
                    }
                }
            }
        } else {
            gap_newlines = 0;
            cur.push(c);
        }
    }
    flush(&mut cur, &mut out);
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct RsvpWord {
    pub text: String,
    /// Character index (not byte) of the optimal recognition point.
    pub orp: usize,
    pub duration_ms: u32,
    pub sentence_end: bool,
    pub paragraph_end: bool,
}

/// Optimal recognition point: character index at which the eye should rest.
/// Punctuation around the word is skipped, the point sits about a third into the letters.
pub fn orp_index(word: &str) -> usize {
    let chars: Vec<char> = word.chars().collect();
    let Some(first) = chars.iter().position(|c| c.is_alphanumeric()) else {
        return 0;
    };
    let last = chars
        .iter()
        .rposition(|c| c.is_alphanumeric())
        .unwrap_or(first);
    let n = last - first + 1;
    let inside = match n {
        0..=1 => 0,
        2..=5 => 1,
        6..=9 => 2,
        10..=13 => 3,
        _ => 4,
    };
    first + inside
}

const ABBREVIATIONS: &[&str] = &[
    "mr", "mrs", "ms", "dr", "prof", "st", "vs", "etc", "e.g", "i.e", "no", "jr", "sr",
];

fn strip_closers(w: &str) -> &str {
    w.trim_end_matches(['"', '\'', '”', '’', ')', ']', '}', '»', '›'])
}

fn ends_sentence(word: &str) -> bool {
    let w = strip_closers(word);
    if w.ends_with(['!', '?', '…']) {
        return true;
    }
    if let Some(stem) = w.strip_suffix('.') {
        if w.ends_with("..") {
            return true;
        }
        let stem_l = stem
            .trim_start_matches(['"', '\'', '“', '‘', '(', '['])
            .to_lowercase();
        return !ABBREVIATIONS.contains(&stem_l.as_str());
    }
    false
}

fn ends_clause(word: &str) -> bool {
    strip_closers(word).ends_with([',', ';', ':', '—', '–'])
}

fn core_len(word: &str) -> usize {
    let chars: Vec<char> = word.chars().collect();
    match (
        chars.iter().position(|c| c.is_alphanumeric()),
        chars.iter().rposition(|c| c.is_alphanumeric()),
    ) {
        (Some(a), Some(b)) => b - a + 1,
        _ => 0,
    }
}

/// Builds the word schedule for `text`.
pub fn schedule(text: &str, cfg: &RsvpConfig) -> Result<Vec<RsvpWord>> {
    if !(MIN_WPM..=MAX_WPM).contains(&cfg.wpm) {
        bail!(
            "wpm must be between {MIN_WPM} and {MAX_WPM}, got {}",
            cfg.wpm
        );
    }
    let base = 60_000.0 / cfg.wpm as f64;
    let words = tokenize(text)
        .into_iter()
        .map(|t| {
            let mut factor = 1.0;
            let n = core_len(&t.text);
            if n > cfg.long_word_chars {
                factor *= (1.0 + cfg.long_word_step * (n - cfg.long_word_chars) as f64)
                    .min(cfg.max_long_factor);
            }
            let sentence_end = ends_sentence(&t.text);
            let mut pause = 1.0;
            if sentence_end {
                pause = cfg.sentence_factor;
            } else if ends_clause(&t.text) {
                pause = cfg.comma_factor;
            }
            if t.paragraph_end {
                pause = pause.max(cfg.paragraph_factor);
            }
            factor *= pause;
            let ms = (base * factor).round().max(cfg.min_ms as f64);
            RsvpWord {
                orp: orp_index(&t.text),
                duration_ms: ms as u32,
                sentence_end,
                paragraph_end: t.paragraph_end,
                text: t.text,
            }
        })
        .collect();
    Ok(words)
}

/// The `ahead` words following `index` (the preview shown next to the focus word).
pub fn preview_window(words: &[RsvpWord], index: usize, ahead: usize) -> &[RsvpWord] {
    let start = index.saturating_add(1).min(words.len());
    let end = start.saturating_add(ahead).min(words.len());
    &words[start..end]
}

pub fn total_duration_ms(words: &[RsvpWord]) -> u64 {
    words.iter().map(|w| w.duration_ms as u64).sum()
}

/// Index of the word being shown `ms` milliseconds after the start (clamped to the last word).
pub fn index_at_ms(words: &[RsvpWord], ms: u64) -> Option<usize> {
    if words.is_empty() {
        return None;
    }
    let mut acc = 0u64;
    for (i, w) in words.iter().enumerate() {
        acc += w.duration_ms as u64;
        if ms < acc {
            return Some(i);
        }
    }
    Some(words.len() - 1)
}

/// Back/forward stack of reading positions ("go to previous location").
#[derive(Debug, Clone, PartialEq)]
pub struct JumpHistory<P> {
    entries: Vec<P>,
    cursor: usize,
    capacity: usize,
}

impl<P: Clone + PartialEq> JumpHistory<P> {
    /// `capacity` is the maximum number of remembered positions (at least 2).
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Vec::new(),
            cursor: 0,
            capacity: capacity.max(2),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn current(&self) -> Option<&P> {
        self.entries.get(self.cursor)
    }

    pub fn can_back(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_forward(&self) -> bool {
        self.cursor + 1 < self.entries.len()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.cursor = 0;
    }

    /// Records a jump from `from` to `to`. Forward history is discarded, adjacent
    /// duplicates are collapsed and the oldest entries fall off at capacity.
    pub fn jump(&mut self, from: P, to: P) {
        if self.entries.is_empty() {
            self.entries.push(from.clone());
            self.cursor = 0;
        } else {
            self.entries.truncate(self.cursor + 1);
            self.entries[self.cursor] = from.clone();
            if self.cursor > 0 && self.entries[self.cursor - 1] == from {
                self.entries.pop();
            }
        }
        if self.entries.last() != Some(&to) {
            self.entries.push(to);
        }
        while self.entries.len() > self.capacity {
            self.entries.remove(0);
        }
        self.cursor = self.entries.len() - 1;
    }

    /// Goes back; `here` is where the reader is now (so `forward` can return to it).
    pub fn back(&mut self, here: P) -> Option<&P> {
        if self.cursor == 0 || self.entries.is_empty() {
            return None;
        }
        self.entries[self.cursor] = here;
        self.cursor -= 1;
        self.entries.get(self.cursor)
    }

    pub fn forward(&mut self, here: P) -> Option<&P> {
        if !self.can_forward() {
            return None;
        }
        self.entries[self.cursor] = here;
        self.cursor += 1;
        self.entries.get(self.cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(t: &str) -> Vec<String> {
        tokenize(t).into_iter().map(|t| t.text).collect()
    }

    #[test]
    fn tokenizer_basics() {
        assert_eq!(
            texts("  hello \t world\n\nnext  "),
            vec!["hello", "world", "next"]
        );
        assert!(tokenize("").is_empty());
        assert!(tokenize(" \n\t ").is_empty());
        assert_eq!(texts("naïve 日本語 🦀"), vec!["naïve", "日本語", "🦀"]);
        assert_eq!(texts("well-known —dash"), vec!["well-known", "—dash"]);
    }

    #[test]
    fn tokenizer_paragraph_ends() {
        let t = tokenize("one two\n\nthree\nfour\r\n\r\nfive\n\n\n");
        let flags: Vec<(&str, bool)> = t
            .iter()
            .map(|t| (t.text.as_str(), t.paragraph_end))
            .collect();
        assert_eq!(
            flags,
            vec![
                ("one", false),
                ("two", true),
                ("three", false),
                ("four", true),
                ("five", true)
            ]
        );
        // a single newline is not a paragraph break
        assert!(!tokenize("a\nb")[0].paragraph_end);
        // leading blank lines do not mark anything
        assert!(!tokenize("\n\n\na b")[0].paragraph_end);
    }

    #[test]
    fn orp_positions() {
        for (w, i) in [
            ("a", 0),
            ("I", 0),
            ("of", 1),
            ("the", 1),
            ("hello", 1),
            ("reader", 2),
            ("elephant", 2),
            ("beautiful", 2),
            ("wonderfully", 3),
            ("international", 3),
            ("internationally", 4),
            ("extraordinarily", 4),
        ] {
            assert_eq!(orp_index(w), i, "{w}");
        }
    }

    #[test]
    fn orp_skips_punctuation_and_handles_unicode() {
        assert_eq!(orp_index("\"Hello,\""), 2);
        assert_eq!(orp_index("(a)"), 1);
        assert_eq!(orp_index("—"), 0);
        assert_eq!(orp_index("..."), 0);
        assert_eq!(orp_index(""), 0);
        assert_eq!(orp_index("¿Qué?"), 2);
        assert_eq!(orp_index("日本語です"), 1);
        // result is always a valid char index
        for w in ["x", "ab", "“quoted”", "naïveté", "...word"] {
            assert!(orp_index(w) < w.chars().count().max(1), "{w}");
        }
    }

    #[test]
    fn duration_from_wpm() {
        let cfg = RsvpConfig {
            wpm: 300,
            ..Default::default()
        };
        let s = schedule("one two three", &cfg).unwrap();
        assert_eq!(
            s.iter().map(|w| w.duration_ms).collect::<Vec<_>>(),
            vec![200, 200, 200]
        );
        let fast = schedule(
            "one",
            &RsvpConfig {
                wpm: 600,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(fast[0].duration_ms, 100);
        assert_eq!(total_duration_ms(&s), 600);
    }

    #[test]
    fn punctuation_pauses() {
        let cfg = RsvpConfig::default();
        let s = schedule("word, word; word: word. word! word? word… word", &cfg).unwrap();
        let d: Vec<u32> = s.iter().map(|w| w.duration_ms).collect();
        assert_eq!(d, vec![300, 300, 300, 440, 440, 440, 440, 200]);
        assert!(s[3].sentence_end && !s[0].sentence_end);
    }

    #[test]
    fn punctuation_with_closing_quotes_and_abbreviations() {
        let cfg = RsvpConfig::default();
        let s = schedule("said.\" fine.) Mr. Smith e.g. x end.", &cfg).unwrap();
        let d: Vec<u32> = s.iter().map(|w| w.duration_ms).collect();
        assert_eq!(
            d[0], 440,
            "period before a closing quote still ends the sentence"
        );
        assert_eq!(d[1], 440);
        assert_eq!(d[2], 200, "Mr. is not a sentence end");
        assert_eq!(d[4], 200, "e.g. is not a sentence end");
        assert_eq!(d[6], 440);
        let s = schedule("wait... what", &cfg).unwrap();
        assert!(s[0].sentence_end);
    }

    #[test]
    fn long_words_take_longer_with_cap() {
        let cfg = RsvpConfig::default();
        let s = schedule(
            "eight888 internationalization supercalifragilisticexpialidocious",
            &cfg,
        )
        .unwrap();
        assert_eq!(s[0].duration_ms, 200, "8 letters is not long");
        // 20 letters: 1 + 0.08*12 = 1.96
        assert_eq!(s[1].duration_ms, 392);
        assert_eq!(s[2].duration_ms, 400, "factor capped at 2.0");
    }

    #[test]
    fn paragraph_end_pause_and_min_duration() {
        let cfg = RsvpConfig::default();
        let s = schedule("end\n\nnext", &cfg).unwrap();
        assert_eq!(s[0].duration_ms, 600);
        assert!(s[0].paragraph_end);
        let s = schedule("done.\n\nnext", &cfg).unwrap();
        assert_eq!(
            s[0].duration_ms, 600,
            "paragraph factor wins over sentence factor"
        );
        let fast = RsvpConfig {
            wpm: 2000,
            min_ms: 50,
            ..Default::default()
        };
        assert_eq!(schedule("a", &fast).unwrap()[0].duration_ms, 50);
    }

    #[test]
    fn invalid_wpm_and_empty_text() {
        assert!(schedule(
            "x",
            &RsvpConfig {
                wpm: 49,
                ..Default::default()
            }
        )
        .unwrap_err()
        .to_string()
        .contains("wpm"));
        assert!(schedule(
            "x",
            &RsvpConfig {
                wpm: 2001,
                ..Default::default()
            }
        )
        .is_err());
        assert!(schedule(
            "x",
            &RsvpConfig {
                wpm: 0,
                ..Default::default()
            }
        )
        .is_err());
        assert!(schedule(
            "x",
            &RsvpConfig {
                wpm: 50,
                ..Default::default()
            }
        )
        .is_ok());
        assert!(schedule("   ", &RsvpConfig::default()).unwrap().is_empty());
    }

    #[test]
    fn preview_window_bounds() {
        let s = schedule("a b c d e", &RsvpConfig::default()).unwrap();
        let t = |w: &[RsvpWord]| w.iter().map(|x| x.text.clone()).collect::<Vec<_>>();
        assert_eq!(t(preview_window(&s, 0, 2)), vec!["b", "c"]);
        assert_eq!(t(preview_window(&s, 3, 5)), vec!["e"]);
        assert!(preview_window(&s, 4, 3).is_empty());
        assert!(preview_window(&s, 99, 3).is_empty());
        assert!(preview_window(&s, 0, 0).is_empty());
        assert!(preview_window(&s, usize::MAX, usize::MAX).is_empty());
        assert!(preview_window(&[], 0, 3).is_empty());
    }

    #[test]
    fn index_at_time() {
        let s = schedule("a b, c", &RsvpConfig::default()).unwrap(); // 200, 300, 200
        assert_eq!(index_at_ms(&s, 0), Some(0));
        assert_eq!(index_at_ms(&s, 199), Some(0));
        assert_eq!(index_at_ms(&s, 200), Some(1));
        assert_eq!(index_at_ms(&s, 499), Some(1));
        assert_eq!(index_at_ms(&s, 500), Some(2));
        assert_eq!(index_at_ms(&s, 10_000), Some(2));
        assert_eq!(index_at_ms(&[], 5), None);
        assert_eq!(total_duration_ms(&[]), 0);
    }

    #[test]
    fn schedule_property_durations_bounded() {
        let text = "The quick, brown fox — jumped over (the lazy) dog. “Really?” she said…\n\nIndeed! Internationalization matters.";
        for wpm in [50, 120, 300, 700, 2000] {
            let cfg = RsvpConfig {
                wpm,
                ..Default::default()
            };
            let s = schedule(text, &cfg).unwrap();
            let base = 60_000.0 / wpm as f64;
            for w in &s {
                assert!(w.duration_ms >= cfg.min_ms);
                assert!(
                    w.duration_ms as f64 <= base * 2.0 * 3.0 + 1.0,
                    "{} at {wpm}",
                    w.text
                );
                assert!(w.orp < w.text.chars().count());
            }
        }
    }

    // ---- jump history

    #[test]
    fn jump_history_back_and_forward() {
        let mut h: JumpHistory<u32> = JumpHistory::new(10);
        assert!(h.is_empty());
        assert!(h.current().is_none());
        assert!(h.back(0).is_none());
        assert!(h.forward(0).is_none());
        h.jump(10, 50); // user was at 10, jumped to 50
        h.jump(52, 90); // read on to 52, jumped to 90
        assert_eq!(h.len(), 3);
        assert_eq!(h.current(), Some(&90));
        assert_eq!(h.back(95), Some(&52));
        assert_eq!(h.back(52), Some(&10));
        assert!(h.back(10).is_none());
        assert!(!h.can_back());
        assert_eq!(h.forward(10), Some(&52));
        assert_eq!(
            h.forward(53),
            Some(&95),
            "forward returns to where the reader left"
        );
        assert!(h.forward(95).is_none());
        assert!(!h.can_forward());
        assert!(h.can_back());
    }

    #[test]
    fn jump_clears_forward_history() {
        let mut h = JumpHistory::new(10);
        h.jump(1, 2);
        h.jump(2, 3);
        h.jump(3, 4);
        h.back(4);
        h.back(3);
        assert!(h.can_forward());
        h.jump(2, 99);
        assert!(!h.can_forward());
        assert_eq!(h.current(), Some(&99));
        assert_eq!(h.back(99), Some(&2));
        assert_eq!(h.back(2), Some(&1));
    }

    #[test]
    fn jump_dedups_adjacent_entries() {
        let mut h = JumpHistory::new(10);
        h.jump(5, 5);
        assert_eq!(h.len(), 1);
        h.jump(5, 7);
        h.jump(7, 7);
        assert_eq!(h.len(), 2);
        h.jump(7, 5);
        h.jump(5, 7);
        // 5,7,5,7 are not adjacent duplicates
        assert_eq!(h.len(), 4);
        let mut h2 = JumpHistory::new(10);
        h2.jump(1, 2);
        h2.jump(1, 3); // from equals the entry before the current one
        assert_eq!(h2.len(), 2);
        assert_eq!(h2.current(), Some(&3));
    }

    #[test]
    fn jump_history_is_bounded() {
        let mut h = JumpHistory::new(3);
        for i in 0..20u32 {
            h.jump(i * 10, i * 10 + 5);
        }
        assert_eq!(h.len(), 3);
        assert_eq!(h.current(), Some(&195));
        let mut back = Vec::new();
        let mut here = 195;
        while let Some(p) = h.back(here).cloned() {
            back.push(p);
            here = p;
        }
        assert_eq!(back, vec![190, 180]);
        let tiny: JumpHistory<u8> = JumpHistory::new(0);
        assert_eq!(tiny.capacity, 2);
    }

    #[test]
    fn jump_history_clear_and_generic_positions() {
        let mut h: JumpHistory<(usize, f32)> = JumpHistory::new(5);
        h.jump((0, 0.1), (3, 0.5));
        assert_eq!(h.back((3, 0.9)), Some(&(0, 0.1)));
        h.clear();
        assert!(h.is_empty());
        assert!(h.current().is_none());
        assert!(!h.can_back() && !h.can_forward());
        h.jump((1, 0.0), (2, 0.0));
        assert_eq!(h.len(), 2);
    }
}
