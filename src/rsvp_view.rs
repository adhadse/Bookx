//! The speed-reading player (R48), without GTK: which word is on screen, how it is split around the
//! recognition point, pausing, stepping and changing speed. Timing comes from
//! `bookx_lookup::rsvp::schedule`.

use bookx_lookup::rsvp::{schedule, RsvpConfig, RsvpWord, MAX_WPM, MIN_WPM};

pub const DEFAULT_WPM: u32 = 300;
pub const STEP_WPM: u32 = 25;

pub fn clamp_wpm(w: u32) -> u32 {
    w.clamp(MIN_WPM, 1000.min(MAX_WPM))
}

/// Char offset in `full` where the words of `page` begin (0 when they cannot be found), so that
/// speed reading starts on the page that is open.
pub fn start_offset(full: &str, page: &str) -> usize {
    let probe: String = page
        .split_whitespace()
        .take(6)
        .collect::<Vec<_>>()
        .join(" ");
    if probe.is_empty() {
        return 0;
    }
    // canonical text may break lines differently: compare with collapsed white space
    let collapsed: String = full.split_whitespace().collect::<Vec<_>>().join(" ");
    match collapsed.find(&probe) {
        Some(byte) => {
            // map back to an offset in `full` by counting words before the match
            let words_before = collapsed[..byte].split_whitespace().count();
            let mut count = 0;
            let mut in_word = false;
            for (i, c) in full.chars().enumerate() {
                if c.is_whitespace() {
                    in_word = false;
                } else if !in_word {
                    in_word = true;
                    if count == words_before {
                        return i;
                    }
                    count += 1;
                }
            }
            0
        }
        None => 0,
    }
}

/// Char ranges `(start, end)` of the whitespace separated words of `text`, shifted by `base`.
/// They match `bookx_lookup::rsvp::tokenize`, so range `i` belongs to word `i`.
pub fn word_ranges(text: &str, base: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut n = 0;
    for (i, c) in text.chars().enumerate() {
        n = i + 1;
        if c.is_whitespace() {
            if let Some(a) = start.take() {
                out.push((base + a, base + i));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(a) = start {
        out.push((base + a, base + n));
    }
    out
}

pub struct Player {
    text: String,
    ranges: Vec<(usize, usize)>,
    /// Words of the whole text that come before the first one of this player.
    first_word: usize,
    words: Vec<RsvpWord>,
    pub index: usize,
    wpm: u32,
}

impl Player {
    /// `text` split into words, starting at the word that begins at or after char offset `from`.
    pub fn new(full_text: &str, from_char: usize, wpm: u32) -> Player {
        let text = full_text;
        let wpm = clamp_wpm(wpm);
        let skipped: String = text.chars().skip(from_char).collect();
        let lead = skipped.chars().take_while(|c| c.is_whitespace()).count();
        let text = skipped.trim_start().to_string();
        let ranges = word_ranges(&text, from_char + lead);
        let first_word = word_ranges(
            &full_text.chars().take(from_char + lead).collect::<String>(),
            0,
        )
        .iter()
        .filter(|r| r.1 <= from_char + lead)
        .count();
        let words = schedule(
            &text,
            &RsvpConfig {
                wpm,
                ..RsvpConfig::default()
            },
        )
        .unwrap_or_default();
        Player {
            text,
            ranges,
            first_word,
            words,
            index: 0,
            wpm,
        }
    }

    /// Number of the word on screen counted from the start of the whole text.
    pub fn absolute_index(&self) -> usize {
        self.first_word + self.index
    }

    /// Number of words of the whole text (from the start of the chapter to the end).
    pub fn absolute_len(&self) -> usize {
        self.first_word + self.words.len()
    }

    /// Char range, in the text the player was made from, of the word on screen.
    pub fn word_range(&self) -> Option<(i32, i32)> {
        self.ranges
            .get(self.index)
            .map(|r| (r.0 as i32, r.1 as i32))
    }

    /// Char range of the single letter to rest the eye on: the recognition point of the word.
    pub fn focus_range(&self) -> Option<(i32, i32)> {
        let (a, b) = self.word_range()?;
        let at = (a + self.current()?.orp as i32).min(b - 1);
        Some((at, at + 1))
    }

    pub fn wpm(&self) -> u32 {
        self.wpm
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.words.len()
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    pub fn finished(&self) -> bool {
        self.index + 1 >= self.words.len()
    }

    pub fn current(&self) -> Option<&RsvpWord> {
        self.words.get(self.index)
    }

    /// The next few words, dimmed beside the focus word.
    pub fn upcoming(&self, n: usize) -> String {
        bookx_lookup::rsvp::preview_window(&self.words, self.index, n)
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Milliseconds the current word stays on screen.
    pub fn delay_ms(&self) -> u32 {
        self.current().map_or(1000, |w| w.duration_ms)
    }

    /// Moves to the next word; false at the end.
    pub fn advance(&mut self) -> bool {
        if self.finished() {
            return false;
        }
        self.index += 1;
        true
    }

    /// Jumps by `delta` words (negative goes back), staying inside the text.
    pub fn step(&mut self, delta: i64) {
        let last = self.words.len().saturating_sub(1) as i64;
        self.index = (self.index as i64 + delta).clamp(0, last) as usize;
    }

    /// Back to the start of the sentence on screen (or the one before when already at its start).
    pub fn sentence_back(&mut self) {
        if self.words.is_empty() {
            return;
        }
        let mut i = self.index;
        if i > 0 && self.words[i - 1].sentence_end {
            i -= 1;
        }
        while i > 0 && !self.words[i - 1].sentence_end {
            i -= 1;
        }
        self.index = i;
    }

    /// On to the first word of the next sentence.
    pub fn sentence_forward(&mut self) {
        let last = self.words.len().saturating_sub(1);
        let mut i = self.index;
        while i < last && !self.words[i].sentence_end {
            i += 1;
        }
        self.index = (i + 1).min(last);
    }

    /// Changes the speed, keeping the word on screen.
    pub fn set_wpm(&mut self, wpm: u32) {
        let wpm = clamp_wpm(wpm);
        if wpm == self.wpm {
            return;
        }
        let words = schedule(
            &self.text,
            &RsvpConfig {
                wpm,
                ..RsvpConfig::default()
            },
        )
        .unwrap_or_default();
        if words.len() == self.words.len() {
            self.words = words;
        }
        self.wpm = wpm;
    }

    /// Fraction of the text shown so far.
    pub fn fraction(&self) -> f64 {
        if self.words.len() <= 1 {
            return 1.0;
        }
        self.index as f64 / (self.words.len() - 1) as f64
    }

    /// Minutes left at the current speed.
    pub fn minutes_left(&self) -> f64 {
        self.words[self.index.min(self.words.len().saturating_sub(1))..]
            .iter()
            .map(|w| w.duration_ms as f64)
            .sum::<f64>()
            / 60_000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str =
        "The quick brown fox jumps. Over the lazy dog, again and again. A new sentence ends here.";

    #[test]
    fn words_come_in_order_and_the_player_stops_at_the_end() {
        let mut p = Player::new(TEXT, 0, 300);
        assert_eq!(p.current().unwrap().text, "The");
        let mut n = 1;
        while p.advance() {
            n += 1;
        }
        assert_eq!(n, p.len());
        assert!(p.finished());
        assert_eq!(p.current().unwrap().text, "here.");
        assert!(!p.advance());
    }

    #[test]
    fn starting_in_the_middle_skips_what_came_before() {
        let from = TEXT.find("Over").unwrap();
        let p = Player::new(TEXT, from, 300);
        assert_eq!(p.current().unwrap().text, "Over");
        let p = Player::new("", 0, 300);
        assert!(p.is_empty());
        assert!(p.current().is_none());
        assert_eq!(p.delay_ms(), 1000);
    }

    #[test]
    fn the_start_is_found_from_the_text_of_the_page() {
        let full = "Chapter one.\n\nThe quick brown fox\njumps over the lazy dog and runs.";
        let page = "jumps over the lazy dog and runs.";
        let at = start_offset(full, page);
        assert_eq!(&full.chars().skip(at).collect::<String>(), page);
        assert_eq!(start_offset(full, "not in the book at all"), 0);
        assert_eq!(start_offset(full, ""), 0);
        assert_eq!(start_offset(full, "Chapter one."), 0);
    }

    #[test]
    fn word_ranges_follow_the_words_of_the_text() {
        assert_eq!(
            word_ranges("ab  cd\nef", 10),
            vec![(10, 12), (14, 16), (17, 19)]
        );
        assert!(word_ranges("  ", 0).is_empty());
        assert_eq!(word_ranges("é日本 x", 0), vec![(0, 3), (4, 5)]);
        // as many ranges as the tokenizer finds words
        let t = "The quick, brown fox — jumped.\n\nOver (the) lazy dog.";
        assert_eq!(
            word_ranges(t, 0).len(),
            bookx_lookup::rsvp::tokenize(t).len()
        );
    }

    #[test]
    fn the_player_knows_where_the_word_and_its_focus_letter_are_in_the_book() {
        let from = TEXT.find("Over").unwrap();
        let mut p = Player::new(TEXT, from, 300);
        let chars: Vec<char> = TEXT.chars().collect();
        let slice = |(a, b): (i32, i32)| chars[a as usize..b as usize].iter().collect::<String>();
        assert_eq!(slice(p.word_range().unwrap()), "Over");
        assert_eq!(slice(p.focus_range().unwrap()), "v");
        p.step(3);
        assert_eq!(slice(p.word_range().unwrap()), "dog,");
        assert_eq!(slice(p.focus_range().unwrap()), "o");
        // leading white space and a start in the middle of the text
        let q = Player::new("  \n hello world", 0, 300);
        assert_eq!(q.word_range(), Some((4, 9)));
        assert_eq!(q.focus_range(), Some((5, 6)));
        // punctuation around the word never takes the focus
        let r = Player::new("\"Hi,\"", 0, 300);
        assert_eq!(r.focus_range(), Some((2, 3)));
        assert!(Player::new("", 0, 300).word_range().is_none());
    }

    #[test]
    fn word_numbers_count_from_the_start_of_the_chapter() {
        let from = TEXT.find("Over").unwrap();
        let mut p = Player::new(TEXT, from, 300);
        let before = bookx_lookup::rsvp::tokenize(&TEXT[..from]).len();
        assert_eq!(p.absolute_index(), before);
        p.step(3);
        assert_eq!(p.absolute_index(), before + 3);
        assert_eq!(p.absolute_len(), bookx_lookup::rsvp::tokenize(TEXT).len());
        assert_eq!(Player::new(TEXT, 0, 300).absolute_index(), 0);
    }

    #[test]
    fn next_sentence_jumps_over_the_end_of_the_sentence() {
        let mut p = Player::new(TEXT, 0, 300);
        p.sentence_forward();
        assert_eq!(p.current().unwrap().text, "Over");
        p.step(2);
        p.sentence_forward();
        assert_eq!(p.current().unwrap().text, "A");
        p.sentence_forward();
        assert!(p.finished(), "the last sentence stays at the last word");
        p.sentence_forward();
        assert!(p.finished());
    }

    #[test]
    fn stepping_and_sentence_back() {
        let mut p = Player::new(TEXT, 0, 300);
        p.step(7);
        assert_eq!(p.current().unwrap().text, "lazy");
        p.sentence_back();
        assert_eq!(p.current().unwrap().text, "Over");
        p.sentence_back();
        assert_eq!(p.current().unwrap().text, "The");
        p.step(-5);
        assert_eq!(p.index, 0);
        p.step(10_000);
        assert!(p.finished());
    }

    #[test]
    fn speed_changes_keep_the_word_and_change_the_delay() {
        let mut p = Player::new(TEXT, 0, 300);
        p.step(3);
        let slow = p.delay_ms();
        p.set_wpm(600);
        assert_eq!(p.index, 3);
        assert_eq!(p.wpm(), 600);
        assert!(p.delay_ms() < slow);
        p.set_wpm(5);
        assert_eq!(p.wpm(), bookx_lookup::rsvp::MIN_WPM);
        p.set_wpm(99_999);
        assert_eq!(p.wpm(), 1000);
    }

    #[test]
    fn progress_and_time_left() {
        let mut p = Player::new(TEXT, 0, 300);
        assert_eq!(p.fraction(), 0.0);
        let full = p.minutes_left();
        assert!(full > 0.0 && full < 1.0);
        p.step(10_000);
        assert_eq!(p.fraction(), 1.0);
        assert!(p.minutes_left() < full);
        assert!(p.upcoming(3).is_empty());
        let mut q = Player::new(TEXT, 0, 300);
        assert_eq!(q.upcoming(3), "quick brown fox");
        q.step(1);
        assert_eq!(q.upcoming(2), "brown fox");
    }
}
