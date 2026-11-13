//! Text handling for read aloud: sentences, chunks for models with a length limit, language.

/// One sentence of the chapter: its character range (canonical offsets, the same the canvas
/// uses for highlights) and what to say.
#[derive(Debug, Clone, PartialEq)]
pub struct Sentence {
    pub start: i32,
    pub end: i32,
    pub text: String,
}

const ABBREVIATIONS: [&str; 14] = [
    "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "vs", "etc", "e.g", "i.e", "no", "fig",
];

fn is_terminator(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '…' | '。' | '！' | '？' | '؟' | '।')
}

/// Splits chapter text into sentences. Line breaks end a sentence (the canvas joins paragraphs
/// with `\n`), abbreviations such as "Mr." and decimals such as "3.14" do not. Pieces without a
/// letter or digit ("* * *") are dropped.
pub fn split_sentences(text: &str) -> Vec<Sentence> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    let push = |a: usize, b: usize, out: &mut Vec<Sentence>| {
        let s: String = chars[a..b].iter().collect();
        let lead = s.chars().take_while(|c| c.is_whitespace()).count();
        let t = s.trim();
        if t.chars().any(|c| c.is_alphanumeric()) {
            out.push(Sentence {
                start: (a + lead) as i32,
                end: (a + lead + t.chars().count()) as i32,
                text: t.replace('\u{ad}', ""),
            });
        }
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            push(start, i, &mut out);
            start = i + 1;
        } else if is_terminator(c) {
            // swallow closing punctuation: ?!, .", .)
            let mut j = i + 1;
            while j < chars.len()
                && (is_terminator(chars[j])
                    || matches!(chars[j], '"' | '\'' | '”' | '’' | ')' | ']' | '»'))
            {
                j += 1;
            }
            let at_end = j >= chars.len() || chars[j].is_whitespace();
            if at_end && !(c == '.' && is_abbreviation(&chars[start..i], chars.get(j..))) {
                push(start, j, &mut out);
                start = j;
            }
            i = j;
            continue;
        }
        i += 1;
    }
    push(start, chars.len(), &mut out);
    out
}

/// Is the word before a period an abbreviation, or is the period inside a number?
fn is_abbreviation(before: &[char], after: Option<&[char]>) -> bool {
    let word: String = before
        .iter()
        .rev()
        .take_while(|c| !c.is_whitespace())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let w = word
        .trim_start_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    if ABBREVIATIONS.contains(&w.as_str()) {
        return true;
    }
    // a single capital letter: an initial ("J. R. R. Tolkien")
    if w.chars().count() == 1 && word.chars().last().is_some_and(|c| c.is_uppercase()) {
        return true;
    }
    // continues with a lower-case letter: "etc. and so on" is not certain, but "e.g. this" is
    let next = after.and_then(|a| a.iter().find(|c| !c.is_whitespace()));
    w.contains('.') && next.is_some_and(|c| c.is_lowercase())
}

/// Cuts `text` into pieces of at most `max` characters for models that take a limited number
/// of tokens: at clause punctuation when it can, at a space otherwise, in the middle of a word
/// only when there is no space at all. The pieces joined with a space are the text again
/// (apart from the white space at the cuts).
pub fn chunk_text(text: &str, max: usize) -> Vec<String> {
    let max = max.max(8);
    let mut out = Vec::new();
    let mut rest: Vec<char> = text.trim().chars().collect();
    while rest.len() > max {
        let window = &rest[..max];
        let cut = window
            .iter()
            .rposition(|c| matches!(c, ',' | ';' | ':' | '—' | '–' | '、' | '，' | '；'))
            .filter(|p| *p >= max / 3)
            .map(|p| p + 1)
            .or_else(|| {
                window
                    .iter()
                    .rposition(|c| c.is_whitespace())
                    .filter(|p| *p >= max / 3)
            })
            .unwrap_or(max);
        let piece: String = rest[..cut].iter().collect();
        let piece = piece.trim();
        if !piece.is_empty() {
            out.push(piece.to_string());
        }
        rest.drain(..cut);
        while rest.first().is_some_and(|c| c.is_whitespace()) {
            rest.remove(0);
        }
    }
    let last: String = rest.iter().collect();
    if !last.trim().is_empty() {
        out.push(last.trim().to_string());
    }
    out
}

// ---- language ----------------------------------------------------------------------------

/// The two-letter code of a language tag: "en-GB" and "eng" give "en"; "" and "und" give "".
pub fn primary_language(tag: &str) -> String {
    let t = tag.trim().to_lowercase();
    let first = t.split(['-', '_']).next().unwrap_or("");
    let code = match first {
        "eng" => "en",
        "deu" | "ger" => "de",
        "fra" | "fre" => "fr",
        "spa" => "es",
        "ita" => "it",
        "por" => "pt",
        "nld" | "dut" => "nl",
        "rus" => "ru",
        "jpn" => "ja",
        "zho" | "chi" => "zh",
        "kor" => "ko",
        "hin" => "hi",
        "ara" => "ar",
        "pol" => "pl",
        "swe" => "sv",
        "tur" => "tr",
        "ukr" => "uk",
        "ces" | "cze" => "cs",
        "und" | "mul" | "zxx" => "",
        other => other,
    };
    if (2..=3).contains(&code.len()) && code.chars().all(|c| c.is_ascii_lowercase()) {
        code.to_string()
    } else {
        String::new()
    }
}

const STOPWORDS: [(&str, &[&str]); 8] = [
    (
        "en",
        &[
            "the", "and", "of", "to", "was", "that", "with", "his", "her", "for", "had", "not",
            "you", "this", "but",
        ],
    ),
    (
        "de",
        &[
            "der", "die", "und", "das", "nicht", "ist", "ein", "ich", "mit", "den", "sie", "zu",
            "auf", "dem", "von",
        ],
    ),
    (
        "fr",
        &[
            "le", "la", "les", "des", "est", "une", "que", "pas", "pour", "dans", "qui", "il",
            "elle", "avec", "vous",
        ],
    ),
    (
        "es",
        &[
            "el", "los", "las", "que", "una", "por", "con", "para", "del", "como", "pero", "sus",
            "más", "está", "era",
        ],
    ),
    (
        "it",
        &[
            "il", "che", "non", "una", "per", "con", "della", "sono", "del", "gli", "come",
            "anche", "più", "era", "nel",
        ],
    ),
    (
        "pt",
        &[
            "que", "não", "uma", "para", "com", "dos", "são", "mais", "como", "mas", "foi", "ele",
            "ela", "seu", "pelo",
        ],
    ),
    (
        "nl",
        &[
            "de", "het", "een", "van", "niet", "dat", "met", "zijn", "ook", "voor", "maar", "hij",
            "ze", "was", "op",
        ],
    ),
    (
        "sv",
        &[
            "och", "att", "det", "som", "en", "på", "är", "för", "med", "inte", "den", "har",
            "var", "han", "av",
        ],
    ),
];

/// Guesses the language of a sample: by script for non-Latin text, by common words for
/// Latin text. `None` when it is not clear.
pub fn detect_language(sample: &str) -> Option<String> {
    let mut counts = [0usize; 9];
    for c in sample.chars().take(4000) {
        let u = c as u32;
        let slot = match u {
            0x3040..=0x30ff => 1, // kana
            0xac00..=0xd7af | 0x1100..=0x11ff => 2,
            0x4e00..=0x9fff => 3,
            0x0400..=0x04ff => 4,
            0x0600..=0x06ff => 5,
            0x0900..=0x097f => 6,
            0x0370..=0x03ff => 7,
            _ if c.is_alphabetic() => 0,
            _ => continue,
        };
        counts[slot] += 1;
    }
    let total: usize = counts.iter().sum();
    if total < 12 {
        return None;
    }
    // kana inside Han text means Japanese
    if counts[1] * 10 >= total {
        return Some("ja".into());
    }
    let scripts = [
        (2, "ko"),
        (3, "zh"),
        (4, "ru"),
        (5, "ar"),
        (6, "hi"),
        (7, "el"),
    ];
    for (slot, lang) in scripts {
        if counts[slot] * 2 >= total {
            return Some(lang.into());
        }
    }
    if counts[0] * 2 < total {
        return None;
    }
    let lower = sample.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .take(600)
        .collect();
    let mut scored: Vec<(usize, &str)> = STOPWORDS
        .iter()
        .map(|(lang, list)| (words.iter().filter(|w| list.contains(w)).count(), *lang))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    let (best, second) = (scored[0].0, scored[1].0);
    (best >= 3 && best * 10 >= second * 13).then(|| scored[0].1.to_string())
}

/// The language to read a book in: what its metadata says, else what the text looks like.
pub fn book_language(metadata: &str, sample: &str) -> String {
    let m = primary_language(metadata);
    if !m.is_empty() {
        return m;
    }
    detect_language(sample).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(s: &str) -> Vec<String> {
        split_sentences(s).into_iter().map(|s| s.text).collect()
    }

    #[test]
    fn sentences_end_at_terminators_and_line_breaks() {
        assert_eq!(
            texts("One. Two! Three? Four"),
            vec!["One.", "Two!", "Three?", "Four"]
        );
        assert_eq!(texts("Title\nBody text."), vec!["Title", "Body text."]);
        assert_eq!(
            texts("He said “Go.” Then left."),
            vec!["He said “Go.”", "Then left."]
        );
        assert_eq!(texts("Wait… what?!"), vec!["Wait…", "what?!"]);
    }

    #[test]
    fn abbreviations_decimals_and_initials_do_not_end_sentences() {
        assert_eq!(
            texts("Mr. Smith met Dr. Jones. They talked."),
            vec!["Mr. Smith met Dr. Jones.", "They talked."]
        );
        assert_eq!(texts("Pi is 3.14 exactly."), vec!["Pi is 3.14 exactly."]);
        assert_eq!(
            texts("J. R. R. Tolkien wrote it. Yes."),
            vec!["J. R. R. Tolkien wrote it.", "Yes."]
        );
    }

    #[test]
    fn ranges_are_character_offsets_of_the_text() {
        let text = "Ünï one. Two\nThree.";
        let s = split_sentences(text);
        let chars: Vec<char> = text.chars().collect();
        for x in &s {
            let slice: String = chars[x.start as usize..x.end as usize].iter().collect();
            assert_eq!(slice, x.text);
        }
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn pieces_without_words_are_skipped_and_soft_hyphens_removed() {
        assert_eq!(texts("* * *\nReal text."), vec!["Real text."]);
        assert_eq!(texts("hy\u{ad}phen word."), vec!["hyphen word."]);
        assert!(texts("").is_empty());
        assert!(texts("...\n\n").is_empty());
    }

    #[test]
    fn chunks_respect_the_limit_and_keep_the_words() {
        let long = "word ".repeat(100);
        let c = chunk_text(&long, 60);
        assert!(c.len() >= 8);
        assert!(c.iter().all(|p| p.chars().count() <= 60), "{c:?}");
        assert_eq!(c.join(" "), long.trim());
        assert_eq!(chunk_text("short.", 60), vec!["short."]);
        assert!(chunk_text("  ", 60).is_empty());
    }

    #[test]
    fn chunks_prefer_clause_punctuation() {
        let t = "A fairly long opening clause that goes on, and a second clause that also goes on for a while";
        let c = chunk_text(t, 60);
        assert_eq!(c[0], "A fairly long opening clause that goes on,");
        assert_eq!(c.join(" "), t);
    }

    #[test]
    fn chunks_cut_words_without_spaces() {
        let c = chunk_text(&"x".repeat(50), 20);
        assert_eq!(c.iter().map(|p| p.len()).collect::<Vec<_>>(), [20, 20, 10]);
    }

    #[test]
    fn language_tags_are_reduced_to_two_letters() {
        assert_eq!(primary_language("en-GB"), "en");
        assert_eq!(primary_language("deu"), "de");
        assert_eq!(primary_language("fre"), "fr");
        assert_eq!(primary_language("und"), "");
        assert_eq!(primary_language(""), "");
        assert_eq!(primary_language("12"), "");
    }

    #[test]
    fn the_language_is_detected_from_script_and_words() {
        let en = "It was the best of times, it was the worst of times, and the people of the town did not know that it was the end.";
        let de = "Es war einmal ein Mann, der nicht wusste, was er mit dem Leben anfangen sollte, und die Leute sagten, dass er ein Narr ist.";
        let fr = "Il était une fois une femme qui ne savait pas ce que le monde pouvait faire pour elle, et les gens disaient qu'elle est folle.";
        assert_eq!(detect_language(en).as_deref(), Some("en"));
        assert_eq!(detect_language(de).as_deref(), Some("de"));
        assert_eq!(detect_language(fr).as_deref(), Some("fr"));
        assert_eq!(
            detect_language("Привет, мир! Это довольно длинное предложение.").as_deref(),
            Some("ru")
        );
        assert_eq!(
            detect_language("吾輩は猫である。名前はまだ無い。どこで生れたかとんと見当がつかぬ。")
                .as_deref(),
            Some("ja")
        );
        assert_eq!(
            detect_language("我是一只猫。我还没有名字。我不知道自己出生在哪里。").as_deref(),
            Some("zh")
        );
        assert_eq!(detect_language("Hi."), None);
        assert_eq!(detect_language("zzz qqq xxx vvv www kkk jjj"), None);
    }

    #[test]
    fn metadata_wins_and_the_text_fills_the_gap() {
        assert_eq!(book_language("de-AT", "the and of to was that"), "de");
        let en = "It was the best of times, it was the worst of times, and the people of the town did not know that.";
        assert_eq!(book_language("", en), "en");
        assert_eq!(book_language("und", en), "en");
        assert_eq!(book_language("", "?"), "");
    }
}
