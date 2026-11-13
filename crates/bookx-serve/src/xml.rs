//! Minimal XML text helpers (escaping for element text and attribute values).

/// True for characters allowed in XML 1.0 documents.
pub fn is_xml_char(c: char) -> bool {
    matches!(c,
        '\u{9}' | '\u{A}' | '\u{D}'
        | '\u{20}'..='\u{D7FF}'
        | '\u{E000}'..='\u{FFFD}'
        | '\u{10000}'..='\u{10FFFF}')
}

/// Escapes `& < > " '` and drops characters XML cannot represent (control characters).
/// Safe for both element text and attribute values.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if is_xml_char(c) => out.push(c),
            _ => {}
        }
    }
    out
}

/// Removes HTML/XML tags from a fragment and collapses whitespace, for plain-text summaries.
pub fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' if in_tag => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_the_five_characters() {
        assert_eq!(escape("a&b<c>d\"e'f"), "a&amp;b&lt;c&gt;d&quot;e&apos;f");
    }

    #[test]
    fn keeps_unicode() {
        assert_eq!(escape("Ünïcödé 日本語 🙂"), "Ünïcödé 日本語 🙂");
    }

    #[test]
    fn drops_control_characters_but_keeps_whitespace() {
        assert_eq!(escape("a\u{0}b\u{1}c\td\ne\r"), "abc\td\ne\r");
        assert_eq!(escape("\u{FFFE}x\u{FFFF}"), "x");
    }

    #[test]
    fn escape_is_not_double_applied_to_entities() {
        assert_eq!(escape("&amp;"), "&amp;amp;");
    }

    #[test]
    fn empty_string() {
        assert_eq!(escape(""), "");
    }

    #[test]
    fn xml_char_ranges() {
        assert!(is_xml_char('a'));
        assert!(is_xml_char('\u{10000}'));
        assert!(!is_xml_char('\u{8}'));
        assert!(!is_xml_char('\u{FFFE}'));
    }

    #[test]
    fn strip_tags_removes_markup_and_decodes_entities() {
        assert_eq!(
            strip_tags("<p>Hello <b>big</b>&nbsp;world &amp; co</p>\n<p>Next</p>"),
            "Hello big world & co Next"
        );
        assert_eq!(strip_tags(""), "");
        assert_eq!(strip_tags("no tags"), "no tags");
        assert_eq!(strip_tags("a > b"), "a > b");
    }
}
