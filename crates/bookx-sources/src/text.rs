//! Text normalisation and string similarity shared by ranking, duplicate detection and filename guessing.

/// Maps one character to its unaccented ASCII-ish form (Latin only); other characters are returned as is.
fn fold_char(c: char, out: &mut String) {
    let s: &str = match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ā' | 'Ă' | 'Ą' => "a",
        'ç' | 'ć' | 'č' | 'ĉ' | 'ċ' | 'Ç' | 'Ć' | 'Č' => "c",
        'ď' | 'đ' | 'ð' | 'Ď' | 'Đ' | 'Ð' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' | 'È' | 'É' | 'Ê' | 'Ë' | 'Ē' | 'Ę' | 'Ě' => {
            "e"
        }
        'ğ' | 'ĝ' | 'Ğ' => "g",
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' | 'ı' | 'Ì' | 'Í' | 'Î' | 'Ï' | 'İ' => "i",
        'ł' | 'ľ' | 'ĺ' | 'Ł' | 'Ľ' => "l",
        'ñ' | 'ń' | 'ň' | 'Ñ' | 'Ń' | 'Ň' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' | 'Ő' => {
            "o"
        }
        'ŕ' | 'ř' | 'Ř' => "r",
        'ś' | 'š' | 'ş' | 'ŝ' | 'Ś' | 'Š' | 'Ş' => "s",
        'ť' | 'ţ' | 'Ť' | 'Ţ' => "t",
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' | 'ų' | 'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ū' | 'Ů' | 'Ű' => {
            "u"
        }
        'ý' | 'ÿ' | 'Ý' | 'Ÿ' => "y",
        'ź' | 'ż' | 'ž' | 'Ź' | 'Ż' | 'Ž' => "z",
        'ß' => "ss",
        'æ' | 'Æ' => "ae",
        'œ' | 'Œ' => "oe",
        'þ' | 'Þ' => "th",
        // typographic punctuation
        '\u{2018}' | '\u{2019}' | '\u{201B}' | '\u{2032}' => "'",
        '\u{201C}' | '\u{201D}' | '\u{201E}' => "\"",
        '\u{2010}'..='\u{2015}' | '\u{2212}' => "-",
        '\u{00A0}' | '\u{2009}' | '\u{202F}' | '\u{200A}' => " ",
        // combining marks (decomposed input) vanish
        '\u{0300}'..='\u{036F}' => "",
        _ => {
            out.extend(c.to_lowercase());
            return;
        }
    };
    out.push_str(s);
}

/// Unicode folding for comparisons: lower-cased, Latin diacritics removed, typographic
/// quotes/dashes flattened, `ß`→`ss`, `æ`→`ae`.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        fold_char(c, &mut out);
    }
    out
}

/// Folded, lower-cased, punctuation turned into spaces, whitespace collapsed.
pub fn normalize(s: &str) -> String {
    let folded = fold(s);
    let mut out = String::with_capacity(folded.len());
    let mut space = true;
    for c in folded.chars().flat_map(|c| c.to_lowercase()) {
        if c.is_alphanumeric() {
            out.push(c);
            space = false;
        } else if c == '\'' {
            // "don't" -> "dont"
        } else if !space {
            out.push(' ');
            space = true;
        }
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Levenshtein edit distance in chars.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// `1 - distance / max_len`; two empty strings are identical (1.0).
pub fn levenshtein_ratio(a: &str, b: &str) -> f64 {
    let max = a.chars().count().max(b.chars().count());
    if max == 0 {
        return 1.0;
    }
    1.0 - levenshtein(a, b) as f64 / max as f64
}

/// Jaro similarity in 0..=1.
pub fn jaro(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let window = (a.len().max(b.len()) / 2).saturating_sub(1);
    let mut a_match = vec![false; a.len()];
    let mut b_match = vec![false; b.len()];
    let mut matches = 0usize;
    for (i, ca) in a.iter().enumerate() {
        let lo = i.saturating_sub(window);
        let hi = (i + window + 1).min(b.len());
        for j in lo..hi {
            if !b_match[j] && *ca == b[j] {
                a_match[i] = true;
                b_match[j] = true;
                matches += 1;
                break;
            }
        }
    }
    if matches == 0 {
        return 0.0;
    }
    let mut transpositions = 0usize;
    let mut k = 0;
    for i in 0..a.len() {
        if a_match[i] {
            while !b_match[k] {
                k += 1;
            }
            if a[i] != b[k] {
                transpositions += 1;
            }
            k += 1;
        }
    }
    let m = matches as f64;
    (m / a.len() as f64 + m / b.len() as f64 + (m - (transpositions / 2) as f64) / m) / 3.0
}

/// Jaro-Winkler similarity (prefix scale 0.1, prefix up to 4 chars) in 0..=1.
pub fn jaro_winkler(a: &str, b: &str) -> f64 {
    let j = jaro(a, b);
    let prefix = a
        .chars()
        .zip(b.chars())
        .take(4)
        .take_while(|(x, y)| x == y)
        .count();
    (j + prefix as f64 * 0.1 * (1.0 - j)).min(1.0)
}

/// Similarity of two titles after [`normalize`]: the best of Levenshtein ratio and Jaro-Winkler.
pub fn title_similarity(a: &str, b: &str) -> f64 {
    let (na, nb) = (normalize(a), normalize(b));
    if na.is_empty() && nb.is_empty() {
        return if a.trim() == b.trim() { 1.0 } else { 0.0 };
    }
    let jw = jaro_winkler(&na, &nb);
    // Jaro-Winkler is generous with unrelated long strings: only trust it when it is high
    let jw = if jw >= 0.85 { jw * 0.98 } else { jw * 0.6 };
    levenshtein_ratio(&na, &nb).max(jw)
}

/// Splits a string into normalised words.
pub fn words(s: &str) -> Vec<String> {
    normalize(s)
        .split(' ')
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

/// Removes HTML tags and decodes the most common entities; whitespace is collapsed.
pub fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut tag = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' if !in_tag => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let name: String = tag
                    .trim_start_matches('/')
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .collect::<String>()
                    .to_ascii_lowercase();
                // inline tags vanish, block-level ones separate words
                if !matches!(
                    name.as_str(),
                    "b" | "i"
                        | "em"
                        | "strong"
                        | "u"
                        | "span"
                        | "a"
                        | "small"
                        | "sup"
                        | "sub"
                        | "font"
                        | "code"
                ) {
                    out.push(' ');
                }
            }
            _ if in_tag => tag.push(c),
            _ => out.push(c),
        }
    }
    if in_tag {
        out.push(' ');
    }
    let out = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_removes_accents_and_ligatures() {
        assert_eq!(fold("Éloïse Ångström"), "eloise angstrom");
        assert_eq!(fold("Straße æon Œuvre"), "strasse aeon oeuvre");
        assert_eq!(fold("Łódź"), "lodz");
        assert_eq!(fold("日本語"), "日本語");
        assert_eq!(fold("E\u{0301}"), "e");
        assert_eq!(fold(""), "");
    }

    #[test]
    fn fold_flattens_typographic_punctuation() {
        assert_eq!(
            fold("it\u{2019}s \u{2014} \u{201C}ok\u{201D}"),
            "it's - \"ok\""
        );
        assert_eq!(fold("a\u{00A0}b"), "a b");
    }

    #[test]
    fn normalize_cases() {
        assert_eq!(
            normalize("  The  Hobbit: There & Back Again! "),
            "the hobbit there back again"
        );
        assert_eq!(normalize("Don't"), "dont");
        assert_eq!(normalize("Ünïcödé-Tëst"), "unicode test");
        assert_eq!(normalize("!!!"), "");
        assert_eq!(normalize("日本 語"), "日本 語");
        assert_eq!(normalize("ΑΒΓ"), "αβγ");
    }

    #[test]
    fn levenshtein_known_values() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("same", "same"), 0);
        assert_eq!(levenshtein("flaw", "lawn"), 2);
        assert_eq!(levenshtein("über", "uber"), 1);
    }

    #[test]
    fn levenshtein_ratio_bounds() {
        assert_eq!(levenshtein_ratio("", ""), 1.0);
        assert_eq!(levenshtein_ratio("abc", "abc"), 1.0);
        assert_eq!(levenshtein_ratio("abc", "xyz"), 0.0);
        assert!((levenshtein_ratio("kitten", "sitting") - (1.0 - 3.0 / 7.0)).abs() < 1e-12);
    }

    #[test]
    fn jaro_known_values() {
        assert!((jaro("MARTHA", "MARHTA") - 0.944_444).abs() < 1e-5);
        assert!((jaro("DIXON", "DICKSONX") - 0.766_667).abs() < 1e-5);
        assert_eq!(jaro("", ""), 1.0);
        assert_eq!(jaro("a", ""), 0.0);
        assert_eq!(jaro("abc", "xyz"), 0.0);
        assert_eq!(jaro("abc", "abc"), 1.0);
    }

    #[test]
    fn jaro_winkler_known_values() {
        assert!((jaro_winkler("MARTHA", "MARHTA") - 0.961_111).abs() < 1e-5);
        assert!((jaro_winkler("DIXON", "DICKSONX") - 0.813_333).abs() < 1e-5);
        assert_eq!(jaro_winkler("same", "same"), 1.0);
        assert!(jaro_winkler("abcdef", "abcxyz") > jaro("abcdef", "abcxyz"));
    }

    #[test]
    fn title_similarity_ignores_case_punctuation_accents() {
        assert_eq!(title_similarity("The Hobbit", "the hobbit!"), 1.0);
        assert_eq!(title_similarity("Les Misérables", "les miserables"), 1.0);
        assert!(title_similarity("Dune", "Dune Messiah") > 0.4);
        assert!(title_similarity("Dune", "Neuromancer") < 0.6);
        assert_eq!(title_similarity("!!!", "!!!"), 1.0);
        assert_eq!(title_similarity("!!!", "???"), 0.0);
    }

    #[test]
    fn similarities_are_symmetric_and_reflexive_on_samples() {
        let samples = [
            "",
            "a",
            "Dune",
            "dune messiah",
            "Ünï",
            "日本語の本",
            "The Lord of the Rings",
        ];
        for a in samples {
            for b in samples {
                assert!((levenshtein_ratio(a, b) - levenshtein_ratio(b, a)).abs() < 1e-12);
                assert!((jaro_winkler(a, b) - jaro_winkler(b, a)).abs() < 1e-12);
                assert!((title_similarity(a, b) - title_similarity(b, a)).abs() < 1e-12);
            }
            assert_eq!(levenshtein_ratio(a, a), 1.0);
            assert_eq!(jaro_winkler(a, a), 1.0);
            assert_eq!(title_similarity(a, a), 1.0);
        }
    }

    #[test]
    fn words_splits_normalised() {
        assert_eq!(words("Jean-Paul  Sartre"), vec!["jean", "paul", "sartre"]);
        assert!(words("  ").is_empty());
    }

    #[test]
    fn strip_html_removes_tags_and_entities() {
        assert_eq!(
            strip_html("<p>Hello <b>world</b> &amp; co&nbsp;&lt;3</p>"),
            "Hello world & co <3"
        );
        assert_eq!(strip_html("plain"), "plain");
        assert_eq!(strip_html("a<br/>b"), "a b");
        assert_eq!(strip_html("<i>x</i>y"), "xy");
        assert_eq!(strip_html("unclosed <tag"), "unclosed");
        assert_eq!(strip_html(""), "");
    }
}
