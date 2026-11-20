//! Finding the saved vocabulary terms in the text of a page or chapter.
//!
//! The matcher is built once per book from the terms in force (an Aho-Corasick automaton over the
//! folded terms), then one pass over the text finds every occurrence. Rules:
//!
//! * case-insensitive (Unicode simple lower-casing); `’` and `'` are the same, and so are `-` and
//!   the non-breaking hyphen U+2011;
//! * whole words only: a match must not touch a letter or digit (or a combining mark) on either
//!   side, so `art` is not found in `part`; no inflections (`walk` is not found in `walked`);
//! * phrases (`New York`, `e-mail`, `C++`) match as written; any run of white space in the text is
//!   one space, but a paragraph break (`\n`) ends a phrase;
//! * soft hyphens, zero-width spaces and word joiners in the text are ignored, so a hyphenated
//!   word is still found;
//! * overlapping terms: the longest one wins, a shorter one is used where the longer one does not
//!   fit (`a b` and `a` in `a bc` finds `a`);
//! * offsets are in characters of the original text (what the reader uses for highlights).

use aho_corasick::{AhoCorasick, MatchKind};

/// A found term: char offsets `start..end` in the text searched, and the index of the term in the
/// list the matcher was built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub start: usize,
    pub end: usize,
    pub term: usize,
}

/// Characters that are not part of the text for matching purposes.
fn ignorable(c: char) -> bool {
    matches!(c, '\u{ad}' | '\u{200b}' | '\u{2060}' | '\u{feff}')
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || matches!(c, '\u{200c}' | '\u{200d}' | '\u{300}'..='\u{36f}')
}

fn fold_char(c: char) -> char {
    match c {
        '’' | '‘' | 'ʼ' => '\'',
        '\u{2010}' | '\u{2011}' => '-',
        c => {
            let mut l = c.to_lowercase();
            match (l.next(), l.next()) {
                (Some(x), None) => x,
                _ => c,
            }
        }
    }
}

/// Folded text with, for each folded char, where it came from.
struct Folded {
    text: String,
    chars: Vec<char>,
    /// Byte offset of every folded char in `text`.
    byte_at: Vec<usize>,
    /// Original char offset where the folded char starts and ends (exclusive).
    from: Vec<usize>,
    to: Vec<usize>,
}

fn fold(text: &str, keep_newlines: bool) -> Folded {
    let mut f = Folded {
        text: String::with_capacity(text.len()),
        chars: Vec::new(),
        byte_at: Vec::new(),
        from: Vec::new(),
        to: Vec::new(),
    };
    for (i, c) in text.chars().enumerate() {
        if ignorable(c) {
            continue;
        }
        let space = c.is_whitespace() && !(keep_newlines && c == '\n');
        let out = if space { ' ' } else { fold_char(c) };
        if space && f.chars.last() == Some(&' ') {
            if let Some(t) = f.to.last_mut() {
                *t = i + 1;
            }
            continue;
        }
        f.byte_at.push(f.text.len());
        f.text.push(out);
        f.chars.push(out);
        f.from.push(i);
        f.to.push(i + 1);
    }
    f
}

pub struct TermMatcher {
    ac: Option<AhoCorasick>,
    /// For each pattern of the automaton, the index of its term.
    term_of: Vec<usize>,
    /// First and last char of each pattern (word edges are checked).
    edges: Vec<(char, char)>,
}

impl TermMatcher {
    /// Builds the matcher for these terms. Empty terms are skipped (their index is never found).
    pub fn new<S: AsRef<str>>(terms: &[S]) -> Self {
        let mut patterns: Vec<String> = Vec::new();
        let mut term_of = Vec::new();
        let mut edges = Vec::new();
        for (i, t) in terms.iter().enumerate() {
            let f = fold(t.as_ref().trim(), false);
            let p = f.text.trim().to_string();
            let (Some(a), Some(b)) = (p.chars().next(), p.chars().last()) else {
                continue;
            };
            patterns.push(p);
            term_of.push(i);
            edges.push((a, b));
        }
        let ac = if patterns.is_empty() {
            None
        } else {
            AhoCorasick::builder()
                .match_kind(MatchKind::Standard)
                .build(&patterns)
                .ok()
        };
        TermMatcher { ac, term_of, edges }
    }

    pub fn is_empty(&self) -> bool {
        self.ac.is_none()
    }

    /// Every occurrence of a term in `text`, in order, not overlapping.
    pub fn find(&self, text: &str) -> Vec<Match> {
        let Some(ac) = &self.ac else {
            return Vec::new();
        };
        let f = fold(text, true);
        let mut found: Vec<Match> = Vec::new();
        for m in ac.find_overlapping_iter(&f.text) {
            // byte offsets back to folded chars
            let (Ok(a) | Err(a)) = f.byte_at.binary_search(&m.start());
            let b = match f.byte_at.binary_search(&m.end()) {
                Ok(b) | Err(b) => b,
            };
            if b <= a || a >= f.chars.len() {
                continue;
            }
            let (first, last) = self.edges[m.pattern().as_usize()];
            if is_word(first) && a > 0 && is_word(f.chars[a - 1]) {
                continue;
            }
            if is_word(last) && b < f.chars.len() && is_word(f.chars[b]) {
                continue;
            }
            found.push(Match {
                start: f.from[a],
                end: f.to[b - 1],
                term: self.term_of[m.pattern().as_usize()],
            });
        }
        found.sort_by(|x, y| x.start.cmp(&y.start).then(y.end.cmp(&x.end)));
        let mut out: Vec<Match> = Vec::with_capacity(found.len());
        let mut last_end = 0;
        for m in found {
            if m.start >= last_end || out.is_empty() {
                last_end = m.end;
                out.push(m);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(terms: &[&str], text: &str) -> Vec<(usize, usize, usize)> {
        TermMatcher::new(terms)
            .find(text)
            .iter()
            .map(|m| (m.start, m.end, m.term))
            .collect()
    }

    fn words(terms: &[&str], text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        TermMatcher::new(terms)
            .find(text)
            .iter()
            .map(|m| chars[m.start..m.end].iter().collect())
            .collect()
    }

    #[test]
    fn finds_whole_words_case_insensitively() {
        assert_eq!(
            words(&["flux"], "Flux and FLUX, a flux."),
            ["Flux", "FLUX", "flux"]
        );
        assert_eq!(spans(&["flux"], "a flux"), [(2, 6, 0)]);
        assert!(words(&["art"], "part of the heart, smart cart").is_empty());
        assert!(words(&["walk"], "he walked and walks").is_empty());
    }

    #[test]
    fn unicode_words_and_case() {
        assert_eq!(words(&["élan"], "Un ÉLAN vital"), ["ÉLAN"]);
        assert_eq!(words(&["straße"], "Die Straße. Die STRASSE"), ["Straße"]);
        assert_eq!(words(&["ǆ"], "ǅ"), ["ǅ"]);
        assert_eq!(
            words(&["münchen"], "in München, nicht Munchen"),
            ["München"]
        );
        assert!(words(&["日本"], "xx").is_empty());
        // CJK has no spaces: the edges are letters, so an embedded match is not a word
        assert!(words(&["日本"], "大日本帝国").is_empty());
        assert_eq!(words(&["日本"], "大 日本 帝国"), ["日本"]);
    }

    #[test]
    fn offsets_are_in_characters_not_bytes() {
        // 'é' and '日' take several bytes
        assert_eq!(spans(&["gap"], "éé 日 gap"), [(5, 8, 0)]);
    }

    #[test]
    fn phrases_and_whitespace() {
        assert_eq!(
            words(&["New York"], "in New  York, and new\u{a0}york"),
            ["New  York", "new\u{a0}york"]
        );
        // a paragraph break ends a phrase
        assert!(words(&["New York"], "New\nYork").is_empty());
        assert_eq!(
            words(&["e-mail"], "an e-mail, an e\u{2011}mail"),
            ["e-mail", "e\u{2011}mail"]
        );
    }

    #[test]
    fn punctuation_around_the_term_is_not_part_of_it() {
        assert_eq!(
            words(&["flux"], "“flux,” (flux) flux. flux! flux? flux;"),
            ["flux"; 6]
        );
        assert_eq!(words(&["can't"], "He can’t. CAN'T!"), ["can’t", "CAN'T"]);
    }

    #[test]
    fn terms_with_symbols_at_the_edge() {
        assert_eq!(words(&["C++"], "I like C++ and C++."), ["C++", "C++"]);
        assert_eq!(words(&["U.S.A."], "the U.S.A. is"), ["U.S.A."]);
        assert!(words(&["C++"], "ABC++").is_empty());
    }

    #[test]
    fn overlapping_terms_prefer_the_longest() {
        let t = ["New York", "York", "New"];
        assert_eq!(spans(&t, "New York is new"), [(0, 8, 0), (12, 15, 2)]);
        assert_eq!(spans(&t, "old York"), [(4, 8, 1)]);
        // the longer term does not fit, the shorter one does
        assert_eq!(spans(&["a b", "a"], "a bc"), [(0, 1, 1)]);
        // partly overlapping terms: the first wins, the second only where it is free
        assert_eq!(
            words(&["ice cream", "cream cheese"], "ice cream cheese"),
            ["ice cream"]
        );
    }

    #[test]
    fn soft_hyphens_and_zero_width_characters_are_ignored() {
        // "remembered" (with a soft hyphen and a zero-width space inside) is another word
        let text = "the re\u{ad}mem\u{ad}ber\u{200b}ed and remem\u{ad}ber, re\u{ad}mem\u{ad}ber.";
        assert_eq!(
            words(&["remember"], text),
            ["remem\u{ad}ber", "re\u{ad}mem\u{ad}ber"]
        );
        let ms = TermMatcher::new(&["remember"]).find(text);
        assert_eq!((ms[0].start, ms[0].end), (22, 31));
        // and a term typed with a soft hyphen is the same term
        assert_eq!(words(&["re\u{ad}member"], "remember"), ["remember"]);
    }

    #[test]
    fn right_to_left_text() {
        let text = "זהו שלום עולם, ומילה אחרת: كتاب جميل والكتاب";
        assert_eq!(words(&["שלום עולם"], text), ["שלום עולם"]);
        assert_eq!(words(&["كتاب"], text), ["كتاب"]);
        // not inside another Arabic word
        assert!(words(&["كتاب"], "والكتاب").is_empty());
        // Arabic with vowel marks
        assert_eq!(words(&["كِتَاب"], "هذا كِتَاب جميل"), ["كِتَاب"]);
        // Hindi: a vowel sign is part of the word
        assert!(words(&["कि"], "किताब").is_empty());
        assert_eq!(words(&["किताब"], "यह किताब है"), ["किताब"]);
    }

    #[test]
    fn nothing_to_find() {
        let none: [&str; 0] = [];
        assert!(TermMatcher::new(&none).find("text").is_empty());
        assert!(TermMatcher::new(&["", "  "]).is_empty());
        assert!(TermMatcher::new(&["x"]).find("").is_empty());
    }

    #[test]
    fn the_index_of_the_term_is_reported() {
        assert_eq!(
            spans(&["", "beta", "alpha"], "alpha beta"),
            [(0, 5, 2), (6, 10, 1)]
        );
    }
}
