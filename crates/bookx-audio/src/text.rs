//! Text helpers shared by the tag readers: encodings, base64, natural sort.

use crate::limits::MAX_TEXT_BYTES;

/// Decodes ID3-style text (`enc`: 0 Latin-1, 1 UTF-16 with BOM, 2 UTF-16BE, 3 UTF-8). Never fails:
/// bad bytes become U+FFFD. The result is cut at [`MAX_TEXT_BYTES`].
pub fn decode_text(enc: u8, b: &[u8]) -> String {
    let b = &b[..b.len().min(MAX_TEXT_BYTES)];
    let s = match enc {
        0 => b.iter().map(|&c| c as char).collect::<String>(),
        1 => {
            if b.starts_with(&[0xFF, 0xFE]) {
                utf16(&b[2..], true)
            } else if b.starts_with(&[0xFE, 0xFF]) {
                utf16(&b[2..], false)
            } else {
                utf16(b, true)
            }
        }
        2 => utf16(b, false),
        _ => String::from_utf8_lossy(b).into_owned(),
    };
    s.trim_matches(|c: char| c == '\0' || c.is_whitespace())
        .to_string()
}

fn utf16(b: &[u8], le: bool) -> String {
    let units: Vec<u16> = b
        .chunks_exact(2)
        .map(|c| {
            if le {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}

/// Standard base64 (what Vorbis comments use for pictures). Stops at the first bad byte;
/// the output is at most `max` bytes.
pub fn base64_decode(s: &[u8], max: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity((s.len() / 4 * 3).min(max));
    let (mut acc, mut bits) = (0u32, 0u32);
    for &c in s {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => break,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
            if out.len() >= max {
                break;
            }
        }
    }
    out
}

/// Sort key that puts `Chapter 2` before `Chapter 10` and ignores case.
pub fn natural_key(s: &str) -> Vec<(bool, String, u64)> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            let mut n = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() {
                    n.push(d);
                    chars.next();
                } else {
                    break;
                }
            }
            let v = n.trim_start_matches('0');
            let v = if v.len() > 18 { &v[..18] } else { v };
            out.push((true, String::new(), v.parse().unwrap_or(0)));
        } else {
            let mut t = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() {
                    break;
                }
                t.extend(d.to_lowercase());
                chars.next();
            }
            out.push((false, t, 0));
        }
    }
    out
}

/// A file name is not a title: `03 - The Road.mp3` becomes `The Road`.
pub fn title_from_file_name(stem: &str) -> String {
    let s = stem.replace('_', " ");
    let t = s.trim_start_matches(|c: char| c.is_ascii_digit() || " .-)(_".contains(c));
    let t = if t.is_empty() { s.trim() } else { t.trim() };
    t.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_encodings_decode() {
        assert_eq!(decode_text(0, b"Caf\xe9"), "Café");
        assert_eq!(decode_text(3, "Zoë\0".as_bytes()), "Zoë");
        assert_eq!(decode_text(1, &[0xFF, 0xFE, b'h', 0, b'i', 0]), "hi");
        assert_eq!(decode_text(1, &[0xFE, 0xFF, 0, b'h', 0, b'i']), "hi");
        assert_eq!(decode_text(2, &[0, b'o', 0, b'k']), "ok");
        assert_eq!(decode_text(3, &[0xFF, 0xFE, b'a']), "\u{FFFD}\u{FFFD}a");
        assert_eq!(decode_text(9, b" x "), "x");
    }

    #[test]
    fn base64_decodes_and_is_bounded() {
        assert_eq!(base64_decode(b"SGVsbG8=", 100), b"Hello");
        assert_eq!(base64_decode(b"SGVs\nbG8", 100), b"Hello");
        assert_eq!(base64_decode(b"SGVsbG8=", 3), b"Hel");
        assert_eq!(base64_decode(b"SG!sbG8", 100), b"H");
        assert!(base64_decode(b"", 5).is_empty());
    }

    #[test]
    fn natural_order_counts_numbers() {
        let mut v = vec!["Ch 10.mp3", "ch 2.mp3", "Ch 1.mp3", "Ch 02a.mp3"];
        v.sort_by_key(|s| natural_key(s));
        assert_eq!(v, vec!["Ch 1.mp3", "ch 2.mp3", "Ch 02a.mp3", "Ch 10.mp3"]);
        // absurdly long numbers must not overflow or panic
        let long = format!("a{}", "9".repeat(60));
        let _ = natural_key(&long);
    }

    #[test]
    fn file_names_become_titles() {
        assert_eq!(title_from_file_name("03 - The Road"), "The Road");
        assert_eq!(title_from_file_name("Part_One"), "Part One");
        assert_eq!(title_from_file_name("007"), "007");
    }
}
