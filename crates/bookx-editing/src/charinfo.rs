//! Unicode facts for the "Insert special character" dialog (CE-300..CE-302) and the Characters report:
//! character names (the pure-Rust `unicode_names2` tables), the Unicode blocks, search by name, and the two
//! typing helpers (hex code followed by Alt+X, `&name;` followed by the final semicolon).

use crate::markup::decode_entities;

/// `(first, last, name)` of the Unicode blocks the dialog offers, in code point order. Private use, surrogates and
/// the huge CJK extension blocks are left out on purpose: they have no names worth browsing.
pub const BLOCKS: &[(u32, u32, &str)] = &[
    (0x0020, 0x007F, "Basic Latin"),
    (0x00A0, 0x00FF, "Latin-1 Supplement"),
    (0x0100, 0x017F, "Latin Extended-A"),
    (0x0180, 0x024F, "Latin Extended-B"),
    (0x0250, 0x02AF, "IPA Extensions"),
    (0x02B0, 0x02FF, "Spacing Modifier Letters"),
    (0x0300, 0x036F, "Combining Diacritical Marks"),
    (0x0370, 0x03FF, "Greek and Coptic"),
    (0x0400, 0x04FF, "Cyrillic"),
    (0x0500, 0x052F, "Cyrillic Supplement"),
    (0x0530, 0x058F, "Armenian"),
    (0x0590, 0x05FF, "Hebrew"),
    (0x0600, 0x06FF, "Arabic"),
    (0x0900, 0x097F, "Devanagari"),
    (0x0980, 0x09FF, "Bengali"),
    (0x0E00, 0x0E7F, "Thai"),
    (0x10A0, 0x10FF, "Georgian"),
    (0x1100, 0x11FF, "Hangul Jamo"),
    (0x1E00, 0x1EFF, "Latin Extended Additional"),
    (0x1F00, 0x1FFF, "Greek Extended"),
    (0x2000, 0x206F, "General Punctuation"),
    (0x2070, 0x209F, "Superscripts and Subscripts"),
    (0x20A0, 0x20CF, "Currency Symbols"),
    (0x20D0, 0x20FF, "Combining Marks for Symbols"),
    (0x2100, 0x214F, "Letterlike Symbols"),
    (0x2150, 0x218F, "Number Forms"),
    (0x2190, 0x21FF, "Arrows"),
    (0x2200, 0x22FF, "Mathematical Operators"),
    (0x2300, 0x23FF, "Miscellaneous Technical"),
    (0x2400, 0x243F, "Control Pictures"),
    (0x2460, 0x24FF, "Enclosed Alphanumerics"),
    (0x2500, 0x257F, "Box Drawing"),
    (0x2580, 0x259F, "Block Elements"),
    (0x25A0, 0x25FF, "Geometric Shapes"),
    (0x2600, 0x26FF, "Miscellaneous Symbols"),
    (0x2700, 0x27BF, "Dingbats"),
    (0x27C0, 0x27EF, "Miscellaneous Mathematical Symbols-A"),
    (0x2900, 0x297F, "Supplemental Arrows-B"),
    (0x2A00, 0x2AFF, "Supplemental Mathematical Operators"),
    (0x2B00, 0x2BFF, "Miscellaneous Symbols and Arrows"),
    (0x2E00, 0x2E7F, "Supplemental Punctuation"),
    (0x3000, 0x303F, "CJK Symbols and Punctuation"),
    (0x3040, 0x309F, "Hiragana"),
    (0x30A0, 0x30FF, "Katakana"),
    (0xFB00, 0xFB4F, "Alphabetic Presentation Forms"),
    (0xFE00, 0xFE0F, "Variation Selectors"),
    (0xFE30, 0xFE4F, "CJK Compatibility Forms"),
    (0xFF00, 0xFFEF, "Halfwidth and Fullwidth Forms"),
    (0xFFF0, 0xFFFF, "Specials"),
    (0x1D400, 0x1D7FF, "Mathematical Alphanumeric Symbols"),
    (0x1F300, 0x1F5FF, "Miscellaneous Symbols and Pictographs"),
    (0x1F600, 0x1F64F, "Emoticons"),
    (0x1F680, 0x1F6FF, "Transport and Map Symbols"),
    (0x1F900, 0x1F9FF, "Supplemental Symbols and Pictographs"),
];

/// Index into [`BLOCKS`] of the block that contains `c`.
pub fn block_of(c: char) -> Option<usize> {
    let v = c as u32;
    BLOCKS.iter().position(|(a, b, _)| *a <= v && v <= *b)
}

/// Names of the control characters people actually meet in text (they have no entry in the name tables).
fn control_name(c: char) -> Option<&'static str> {
    Some(match c {
        '\t' => "CHARACTER TABULATION",
        '\n' => "LINE FEED",
        '\r' => "CARRIAGE RETURN",
        '\u{0b}' => "LINE TABULATION",
        '\u{0c}' => "FORM FEED",
        '\u{85}' => "NEXT LINE",
        _ => return None,
    })
}

/// The Unicode name of `c` (`LATIN SMALL LETTER E WITH ACUTE`), if it has one.
pub fn name(c: char) -> Option<String> {
    if let Some(n) = control_name(c) {
        return Some(n.to_string());
    }
    unicode_names2::name(c).map(|n| n.to_string())
}

/// `U+00E9` style label.
pub fn code_label(c: char) -> String {
    format!("U+{:04X}", c as u32)
}

/// Whether `c` is worth showing as a button: it has a name and is not a control character.
pub fn is_listable(c: char) -> bool {
    !c.is_control() && unicode_names2::name(c).is_some()
}

/// The listable characters of block `index` of [`BLOCKS`].
pub fn chars_in_block(index: usize) -> Vec<char> {
    let Some((a, b, _)) = BLOCKS.get(index) else {
        return Vec::new();
    };
    (*a..=*b)
        .filter_map(char::from_u32)
        .filter(|c| is_listable(*c))
        .collect()
}

/// Characters whose name contains every word of `query` (case-insensitive). A query that is a single character,
/// `U+00E9`, or a hex code of four to six digits also finds that character first. At most `limit` results.
pub fn search(query: &str, limit: usize) -> Vec<char> {
    let q = query.trim();
    if q.is_empty() || limit == 0 {
        return Vec::new();
    }
    let mut out: Vec<char> = Vec::new();
    let push = |c: char, out: &mut Vec<char>| {
        if !out.contains(&c) {
            out.push(c);
        }
    };
    let mut it = q.chars();
    if let (Some(c), None) = (it.next(), it.next()) {
        if is_listable(c) {
            push(c, &mut out);
        }
    }
    let hex = q
        .strip_prefix("U+")
        .or_else(|| q.strip_prefix("u+"))
        .unwrap_or(q);
    if (hex.len() != q.len() || (4..=6).contains(&hex.len()))
        && hex.chars().all(|c| c.is_ascii_hexdigit())
    {
        if let Some(c) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
            if is_listable(c) {
                push(c, &mut out);
            }
        }
    }
    if let Some(c) = unicode_names2::character(q) {
        push(c, &mut out);
    }
    let words: Vec<String> = q.split_whitespace().map(|w| w.to_uppercase()).collect();
    'blocks: for (a, b, _) in BLOCKS {
        for v in *a..=*b {
            let Some(c) = char::from_u32(v) else { continue };
            if c.is_control() {
                continue;
            }
            let Some(n) = unicode_names2::name(c) else {
                continue;
            };
            let n = n.to_string();
            if words.iter().all(|w| n.contains(w.as_str())) {
                push(c, &mut out);
                if out.len() >= limit {
                    break 'blocks;
                }
            }
        }
    }
    out.truncate(limit);
    out
}

/// Alt+X: the hex code that ends `before` (the text up to the caret) and the character it stands for.
/// Returns `(number of characters to replace, character)`. A `U+` prefix is replaced too. Without the prefix
/// the longest run of up to six hex digits that makes a valid character is used.
pub fn alt_x(before: &str) -> Option<(usize, char)> {
    let chars: Vec<char> = before.chars().collect();
    let mut digits = 0;
    while digits < 6 && digits < chars.len() && chars[chars.len() - 1 - digits].is_ascii_hexdigit()
    {
        digits += 1;
    }
    if digits == 0 {
        return None;
    }
    let start = chars.len() - digits;
    let has_prefix = start >= 2 && chars[start - 1] == '+' && matches!(chars[start - 2], 'U' | 'u');
    let valid = |n: usize| -> Option<char> {
        let s: String = chars[chars.len() - n..].iter().collect();
        u32::from_str_radix(&s, 16).ok().and_then(char::from_u32)
    };
    if has_prefix {
        let c = valid(digits)?;
        return Some((digits + 2, c));
    }
    (1..=digits).rev().find_map(|n| valid(n).map(|c| (n, c)))
}

/// Typing `;` after `&name` (or `&#233` / `&#xE9`): the character the entity stands for. `before` is the text up to
/// the caret, without the `;`. Returns `(characters to replace, character)`. `&amp;`, `&lt;`, `&gt;`, `&quot;` and `&apos;` are kept as
/// they are, because the markup needs them escaped (a quote would end an attribute value).
pub fn entity_before_semicolon(before: &str) -> Option<(usize, char)> {
    let amp = before.rfind('&')?;
    let name = &before[amp + 1..];
    if name.is_empty()
        || name.len() > 10
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '#')
    {
        return None;
    }
    if matches!(name, "amp" | "lt" | "gt" | "quot" | "apos") {
        return None;
    }
    let decoded = decode_entities(&format!("&{name};"));
    let mut it = decoded.chars();
    let c = it.next()?;
    if it.next().is_some() || matches!(c, '&' | '<' | '>' | '"' | '\'') {
        return None;
    }
    Some((before[amp..].chars().count(), c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_labels() {
        assert_eq!(
            name('é').as_deref(),
            Some("LATIN SMALL LETTER E WITH ACUTE")
        );
        assert_eq!(name('\u{a0}').as_deref(), Some("NO-BREAK SPACE"));
        assert_eq!(name('\n').as_deref(), Some("LINE FEED"));
        assert_eq!(name('\u{e000}'), None);
        assert_eq!(code_label('é'), "U+00E9");
        assert_eq!(code_label('😀'), "U+1F600");
    }

    #[test]
    fn blocks_cover_their_characters() {
        let latin1 = block_of('é').unwrap();
        assert_eq!(BLOCKS[latin1].2, "Latin-1 Supplement");
        assert_eq!(BLOCKS[block_of('€').unwrap()].2, "Currency Symbols");
        assert_eq!(block_of('\u{e000}'), None);
        let list = chars_in_block(latin1);
        assert!(list.contains(&'é') && list.contains(&'\u{a0}'));
        assert!(!list.contains(&'\u{80}'), "controls are not listed");
        assert!(chars_in_block(9999).is_empty());
        for w in BLOCKS.windows(2) {
            assert!(w[0].1 < w[1].0, "{} overlaps {}", w[0].2, w[1].2);
        }
    }

    #[test]
    fn search_by_name_code_and_character() {
        let r = search("e acute", 50);
        assert!(r.contains(&'é') && r.contains(&'É'));
        assert!(r.iter().all(|c| name(*c).unwrap().contains("ACUTE")));
        assert_eq!(search("ACUTE e", 50).len(), r.len(), "word order is free");
        assert_eq!(search("U+00E9", 5)[0], 'é');
        assert_eq!(search("00e9", 5)[0], 'é');
        assert_eq!(search("é", 5)[0], 'é');
        assert_eq!(search("EURO SIGN", 3)[0], '€');
        assert_eq!(search("no-break space", 3)[0], '\u{a0}');
        assert!(search("zzzzqq", 5).is_empty());
        assert!(search("", 5).is_empty());
        assert_eq!(search("letter", 7).len(), 7, "the limit holds");
    }

    #[test]
    fn alt_x_converts_the_hex_before_the_caret() {
        assert_eq!(alt_x("x 00e9"), Some((4, 'é')));
        assert_eq!(alt_x("U+00E9"), Some((6, 'é')));
        assert_eq!(alt_x("a U+2014"), Some((6, '—')));
        assert_eq!(alt_x("1F600"), Some((5, '😀')));
        assert_eq!(alt_x("A9"), Some((2, '©')));
        assert_eq!(alt_x("zz"), None);
        assert_eq!(alt_x(""), None);
        // six digits that are not a valid code point fall back to the longest valid run
        assert_eq!(alt_x("110000"), Some((5, '\u{10000}')));
        assert_eq!(alt_x("x1234567"), Some((5, '\u{34567}')));
        // a surrogate is no character; the shorter run wins
        assert_eq!(alt_x("D800"), Some((3, '\u{800}')));
        assert_eq!(alt_x("U+D800"), None);
    }

    #[test]
    fn entity_typing_replaces_named_and_numeric_references() {
        assert_eq!(entity_before_semicolon("a &nbsp"), Some((5, '\u{a0}')));
        assert_eq!(entity_before_semicolon("&eacute"), Some((7, 'é')));
        assert_eq!(entity_before_semicolon("x&#233"), Some((5, 'é')));
        assert_eq!(entity_before_semicolon("x&#x2014"), Some((7, '—')));
        assert_eq!(
            entity_before_semicolon("a &amp"),
            None,
            "markup stays escaped"
        );
        assert_eq!(entity_before_semicolon("&lt"), None);
        assert_eq!(
            entity_before_semicolon("&quot"),
            None,
            "a quote could end an attribute"
        );
        assert_eq!(entity_before_semicolon("&apos"), None);
        assert_eq!(entity_before_semicolon("&#60"), None);
        assert_eq!(entity_before_semicolon("&nosuchthing"), None);
        assert_eq!(entity_before_semicolon("AT&T"), None);
        assert_eq!(entity_before_semicolon("no ampersand"), None);
        assert_eq!(entity_before_semicolon("&nbsp more"), None);
    }
}
