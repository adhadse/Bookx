//! Windows-1252 helpers for the legacy formats (PML, RB, RTF).

/// The Windows-1252 byte of `c`, if it has one (ASCII maps to itself).
pub fn cp1252_byte(c: char) -> Option<u8> {
    if c.is_ascii() {
        return Some(c as u8);
    }
    let mut buf = [0u8; 4];
    let s = c.encode_utf8(&mut buf);
    let (bytes, _, had_errors) = encoding_rs::WINDOWS_1252.encode(s);
    if had_errors || bytes.len() != 1 {
        None
    } else {
        Some(bytes[0])
    }
}

/// `s` as Windows-1252 bytes; characters it cannot represent become `?` (names and metadata
/// fields of binary formats).
pub fn to_cp1252_lossy(s: &str) -> Vec<u8> {
    s.chars().map(|c| cp1252_byte(c).unwrap_or(b'?')).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_ascii_latin1_and_the_0x80_block() {
        assert_eq!(cp1252_byte('a'), Some(b'a'));
        assert_eq!(cp1252_byte('é'), Some(0xe9));
        assert_eq!(cp1252_byte('“'), Some(0x93));
        assert_eq!(cp1252_byte('€'), Some(0x80));
        assert_eq!(cp1252_byte('\u{a0}'), Some(0xa0));
    }

    #[test]
    fn unmappable_characters_have_no_byte() {
        assert_eq!(cp1252_byte('日'), None);
        assert_eq!(cp1252_byte('😀'), None);
        assert_eq!(cp1252_byte('Ω'), None);
    }

    #[test]
    fn lossy_conversion_uses_question_marks() {
        assert_eq!(to_cp1252_lossy("aé日"), vec![b'a', 0xe9, b'?']);
        assert!(to_cp1252_lossy("").is_empty());
    }
}
