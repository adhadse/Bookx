//! Hyphenation (R2): language aware break points inside words.
//!
//! The patterns come from the `hypher` crate (TeX patterns compiled to tries). Offsets returned are byte
//! offsets into the word that always sit on `char` boundaries.

use hypher::Lang;

/// The soft hyphen character (U+00AD).
pub const SOFT_HYPHEN: char = '\u{ad}';

/// Finds hyphenation points in single words.
#[derive(Debug, Clone)]
pub struct Hyphenator {
    lang: Option<Lang>,
    min_word_len: usize,
    left_min: usize,
    right_min: usize,
}

/// Reduce a BCP-47 style tag (`en-US`, `pt_BR`, `DE`) to a two-letter lower-case code.
pub fn primary_subtag(tag: &str) -> Option<[u8; 2]> {
    let primary = tag.trim().split(['-', '_']).next()?;
    let b = primary.as_bytes();
    if b.len() != 2 || !b.iter().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some([b[0].to_ascii_lowercase(), b[1].to_ascii_lowercase()])
}

impl Hyphenator {
    /// A hyphenator for a language tag. Unknown languages give a hyphenator that never breaks
    /// (check [`Hyphenator::is_supported`]). Defaults: words of at least 5 characters, 2 characters
    /// left and 3 right of a break.
    pub fn for_language(tag: &str) -> Self {
        let lang = primary_subtag(tag).and_then(Lang::from_iso);
        Hyphenator {
            lang,
            min_word_len: 5,
            left_min: 2,
            right_min: 3,
        }
    }

    /// Whether patterns exist for the language.
    pub fn is_supported(&self) -> bool {
        self.lang.is_some()
    }

    /// Minimum word length (in characters) before any break is offered.
    pub fn with_min_word_len(mut self, n: usize) -> Self {
        self.min_word_len = n;
        self
    }

    /// Minimum characters kept on the left and right side of a break (clamped to at least 1).
    pub fn with_margins(mut self, left: usize, right: usize) -> Self {
        self.left_min = left.max(1);
        self.right_min = right.max(1);
        self
    }

    pub fn min_word_len(&self) -> usize {
        self.min_word_len
    }

    pub fn margins(&self) -> (usize, usize) {
        (self.left_min, self.right_min)
    }

    /// Byte offsets inside `word` where a hyphen may be inserted (the hyphen goes *at* the offset).
    ///
    /// Words with U+00AD already inside only report the offsets right after those soft hyphens. URLs,
    /// e-mail addresses and words containing digits are never broken.
    pub fn hyphenate(&self, word: &str) -> Vec<usize> {
        let Some(lang) = self.lang else {
            return Vec::new();
        };
        let total_chars = word.chars().count();
        if total_chars < self.min_word_len {
            return Vec::new();
        }
        if word.contains(SOFT_HYPHEN) {
            return self.explicit_soft_hyphens(word);
        }
        if is_unbreakable(word) {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut seg_start: Option<usize> = None;
        let mut chars_before_seg = 0usize;
        let mut chars_seen = 0usize;
        let flush = |start: usize, end: usize, chars_before: usize, out: &mut Vec<usize>| {
            let seg = &word[start..end];
            let mut offset = start;
            let mut chars = chars_before;
            let mut syllables =
                hypher::hyphenate_bounded(seg, lang, self.left_min, self.right_min).peekable();
            while let Some(s) = syllables.next() {
                offset += s.len();
                chars += s.chars().count();
                if syllables.peek().is_some()
                    && chars >= self.left_min
                    && total_chars - chars >= self.right_min
                {
                    out.push(offset);
                }
            }
        };
        for (i, c) in word.char_indices() {
            if c.is_alphabetic() {
                if seg_start.is_none() {
                    seg_start = Some(i);
                    chars_before_seg = chars_seen;
                }
            } else if let Some(s) = seg_start.take() {
                flush(s, i, chars_before_seg, &mut out);
            }
            chars_seen += 1;
        }
        if let Some(s) = seg_start {
            flush(s, word.len(), chars_before_seg, &mut out);
        }
        out
    }

    fn explicit_soft_hyphens(&self, word: &str) -> Vec<usize> {
        word.char_indices()
            .filter(|(_, c)| *c == SOFT_HYPHEN)
            .map(|(i, c)| i + c.len_utf8())
            .filter(|&o| o < word.len())
            .collect()
    }

    /// Insert U+00AD at every hyphenation point of every word in `text`. Everything else (spacing,
    /// punctuation, existing soft hyphens) is preserved.
    pub fn insert_soft_hyphens(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len() + text.len() / 4);
        let mut token_start: Option<usize> = None;
        let push_token = |tok: &str, out: &mut String| {
            if tok.contains(SOFT_HYPHEN) {
                out.push_str(tok);
                return;
            }
            let mut last = 0;
            for p in self.hyphenate(tok) {
                out.push_str(&tok[last..p]);
                out.push(SOFT_HYPHEN);
                last = p;
            }
            out.push_str(&tok[last..]);
        };
        for (i, c) in text.char_indices() {
            if c.is_whitespace() {
                if let Some(s) = token_start.take() {
                    push_token(&text[s..i], &mut out);
                }
                out.push(c);
            } else if token_start.is_none() {
                token_start = Some(i);
            }
        }
        if let Some(s) = token_start {
            push_token(&text[s..], &mut out);
        }
        out
    }
}

/// Insert soft hyphens with default settings for a language tag.
pub fn insert_soft_hyphens(text: &str, lang: &str) -> String {
    Hyphenator::for_language(lang).insert_soft_hyphens(text)
}

fn is_unbreakable(word: &str) -> bool {
    word.chars().any(|c| c.is_numeric())
        || word.contains("://")
        || word.contains('@')
        || word.contains('/')
        || word.contains('\\')
        || word.to_ascii_lowercase().starts_with("www.")
        || word.contains(".com")
        || word.contains(".org")
        || word.contains(".net")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(word: &str, pts: &[usize]) -> String {
        let mut s = String::new();
        let mut last = 0;
        for &p in pts {
            s.push_str(&word[last..p]);
            s.push('-');
            last = p;
        }
        s.push_str(&word[last..]);
        s
    }

    #[test]
    fn english_known_words() {
        let h = Hyphenator::for_language("en-US");
        assert!(h.is_supported());
        assert_eq!(apply("extensive", &h.hyphenate("extensive")), "ex-ten-sive");
        assert_eq!(apply("wonderful", &h.hyphenate("wonderful")), "won-der-ful");
    }

    #[test]
    fn german_french_spanish_italian_portuguese_dutch_russian_supported() {
        for tag in [
            "de", "fr", "es", "it", "pt", "nl", "ru", "pt-BR", "de_CH", "DE",
        ] {
            let h = Hyphenator::for_language(tag);
            assert!(h.is_supported(), "{tag}");
        }
        let de = Hyphenator::for_language("de");
        assert!(!de.hyphenate("Donaudampfschifffahrt").is_empty());
        let fr = Hyphenator::for_language("fr");
        assert!(!fr.hyphenate("extraordinairement").is_empty());
        let ru = Hyphenator::for_language("ru");
        assert!(!ru.hyphenate("программирование").is_empty());
        let es = Hyphenator::for_language("es");
        assert!(!es.hyphenate("extraordinariamente").is_empty());
        let it = Hyphenator::for_language("it");
        assert!(!it.hyphenate("straordinariamente").is_empty());
        let nl = Hyphenator::for_language("nl");
        assert!(!nl.hyphenate("ontwikkelingssamenwerking").is_empty());
        let pt = Hyphenator::for_language("pt");
        assert!(!pt.hyphenate("extraordinariamente").is_empty());
    }

    #[test]
    fn unknown_language_never_breaks() {
        for tag in ["xx", "", "tlh", "e", "12"] {
            let h = Hyphenator::for_language(tag);
            assert!(!h.is_supported(), "{tag}");
            assert!(h.hyphenate("extraordinarily").is_empty());
        }
    }

    #[test]
    fn primary_subtag_parsing() {
        assert_eq!(primary_subtag("en-US"), Some(*b"en"));
        assert_eq!(primary_subtag(" PT_br "), Some(*b"pt"));
        assert_eq!(primary_subtag("deu"), None);
        assert_eq!(primary_subtag(""), None);
        assert_eq!(primary_subtag("é"), None);
    }

    #[test]
    fn minimum_word_length() {
        let h = Hyphenator::for_language("en");
        assert!(h.hyphenate("ab").is_empty());
        assert!(h.hyphenate("abcd").is_empty());
        let long = Hyphenator::for_language("en").with_min_word_len(20);
        assert!(long.hyphenate("extensive").is_empty());
        assert_eq!(long.min_word_len(), 20);
        let none = Hyphenator::for_language("en").with_min_word_len(0);
        assert_eq!(none.min_word_len(), 0);
    }

    #[test]
    fn left_right_margins_respected() {
        let word = "extensively";
        let base = Hyphenator::for_language("en");
        let pts = base.hyphenate(word);
        assert!(!pts.is_empty());
        let wide = Hyphenator::for_language("en").with_margins(4, 5);
        assert_eq!(wide.margins(), (4, 5));
        for p in wide.hyphenate(word) {
            assert!(word[..p].chars().count() >= 4);
            assert!(word[p..].chars().count() >= 5);
        }
        assert!(wide.hyphenate(word).len() <= pts.len());
        let huge = Hyphenator::for_language("en").with_margins(50, 50);
        assert!(huge.hyphenate(word).is_empty());
        assert_eq!(
            Hyphenator::for_language("en").with_margins(0, 0).margins(),
            (1, 1)
        );
    }

    #[test]
    fn default_margins_two_and_three() {
        let h = Hyphenator::for_language("en");
        assert_eq!(h.margins(), (2, 3));
        for w in [
            "international",
            "hyphenation",
            "representation",
            "congratulations",
        ] {
            for p in h.hyphenate(w) {
                assert!(w[..p].chars().count() >= 2, "{w} {p}");
                assert!(w[p..].chars().count() >= 3, "{w} {p}");
            }
        }
    }

    #[test]
    fn urls_emails_and_numbers_not_broken() {
        let h = Hyphenator::for_language("en");
        for w in [
            "https://example.com/internationalization",
            "www.internationalization.org",
            "internationalization@example.com",
            "wonderful123",
            "1234567890",
            "path/to/internationalization",
            "internationalization.com",
        ] {
            assert!(h.hyphenate(w).is_empty(), "{w}");
        }
    }

    #[test]
    fn existing_soft_hyphens_are_respected() {
        let h = Hyphenator::for_language("en");
        let w = "won\u{ad}derful";
        let pts = h.hyphenate(w);
        assert_eq!(pts, vec![5]);
        assert_eq!(&w[..pts[0]], "won\u{ad}");
        // trailing soft hyphen is not a break inside the word
        assert!(h.hyphenate("wonderful\u{ad}").is_empty());
        // unsupported language still respects explicit ones? no: nothing is offered
        assert!(Hyphenator::for_language("xx").hyphenate(w).is_empty());
    }

    #[test]
    fn uppercase_and_unicode() {
        let h = Hyphenator::for_language("en");
        let lower = h.hyphenate("extensive");
        assert_eq!(h.hyphenate("EXTENSIVE"), lower);
        assert_eq!(h.hyphenate("Extensive"), lower);
        let de = Hyphenator::for_language("de");
        let w = "Überraschungsmoment";
        let pts = de.hyphenate(w);
        assert!(!pts.is_empty());
        for p in pts {
            assert!(w.is_char_boundary(p));
        }
    }

    #[test]
    fn apostrophes_never_adjacent_to_break() {
        let h = Hyphenator::for_language("fr");
        let w = "l'extraordinairement";
        let pts = h.hyphenate(w);
        assert!(!pts.is_empty());
        for p in &pts {
            assert!(w.is_char_boundary(*p));
            let before = w[..*p].chars().last().unwrap();
            let after = w[*p..].chars().next().unwrap();
            assert!(before != '\'' && after != '\'');
            assert!(*p > 2, "no break inside the elided article");
        }
        let typo = "l\u{2019}extraordinairement";
        for p in h.hyphenate(typo) {
            assert!(typo.is_char_boundary(p));
        }
        let en = Hyphenator::for_language("en");
        let w = "don't'wonderful";
        for p in en.hyphenate(w) {
            assert!(!w[..p].ends_with('\'') && !w[p..].starts_with('\''));
        }
    }

    #[test]
    fn punctuation_around_words() {
        let h = Hyphenator::for_language("en");
        let w = "\u{201c}wonderful,\u{201d}";
        let pts = h.hyphenate(w);
        assert!(!pts.is_empty());
        for p in pts {
            assert!(w.is_char_boundary(p));
            assert!(w[p..].chars().count() >= 3);
        }
    }

    #[test]
    fn empty_and_punctuation_only() {
        let h = Hyphenator::for_language("en");
        assert!(h.hyphenate("").is_empty());
        assert!(h.hyphenate("!!!!!!!!").is_empty());
        assert!(h.hyphenate("--------").is_empty());
    }

    #[test]
    fn hyphenated_compound_segments_each_processed() {
        let h = Hyphenator::for_language("en");
        let w = "extensive-wonderful";
        let pts = h.hyphenate(w);
        assert!(pts.iter().any(|&p| p < 9));
        assert!(pts.iter().any(|&p| p > 10));
        assert!(!pts.contains(&9) && !pts.contains(&10));
    }

    #[test]
    fn insert_soft_hyphens_basic() {
        let out = insert_soft_hyphens("A wonderful extensive book", "en");
        assert_eq!(out, "A won\u{ad}der\u{ad}ful ex\u{ad}ten\u{ad}sive book");
        assert_eq!(out.replace(SOFT_HYPHEN, ""), "A wonderful extensive book");
    }

    #[test]
    fn insert_soft_hyphens_preserves_whitespace_and_idempotent() {
        let src = "  wonderful \n\t extensive  ";
        let once = insert_soft_hyphens(src, "en");
        assert_eq!(once.replace(SOFT_HYPHEN, ""), src);
        let twice = insert_soft_hyphens(&once, "en");
        assert_eq!(once, twice);
    }

    #[test]
    fn insert_soft_hyphens_skips_urls_unknown_lang_and_empty() {
        assert_eq!(
            insert_soft_hyphens("see https://example.com/internationalization", "en")
                .matches(SOFT_HYPHEN)
                .count(),
            0
        );
        assert_eq!(insert_soft_hyphens("wonderful", "xx"), "wonderful");
        assert_eq!(insert_soft_hyphens("", "en"), "");
        assert_eq!(insert_soft_hyphens("   ", "en"), "   ");
    }

    #[test]
    fn long_word_over_inline_buffer() {
        let h = Hyphenator::for_language("en");
        let w = "supercalifragilisticexpialidocious".repeat(4);
        let pts = h.hyphenate(&w);
        assert!(!pts.is_empty());
        for p in pts {
            assert!(w.is_char_boundary(p));
        }
    }

    #[test]
    fn property_points_sorted_on_boundaries_and_text_preserved() {
        let mut seed = 0x1234_5678_9abc_def0u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let alphabet: Vec<char> = "abcdefghijklmnopqrstuvwxyzÄÖÜéèñçßABCЖдф'’-1.@/\u{ad} "
            .chars()
            .collect();
        for lang in ["en", "de", "fr", "ru", "es"] {
            let h = Hyphenator::for_language(lang);
            for _ in 0..400 {
                let len = (rnd() % 30) as usize;
                let word: String = (0..len)
                    .map(|_| alphabet[(rnd() as usize) % alphabet.len()])
                    .collect();
                let pts = h.hyphenate(&word);
                let mut prev = 0;
                for &p in &pts {
                    assert!(word.is_char_boundary(p), "{word:?} {p}");
                    assert!(p > prev && p < word.len() + 1);
                    prev = p;
                }
                let inserted = h.insert_soft_hyphens(&word);
                assert_eq!(
                    inserted.replace(SOFT_HYPHEN, ""),
                    word.replace(SOFT_HYPHEN, "")
                );
            }
        }
    }
}
