//! PalmDOC (LZ77) compression of one text record.
//!
//! Format: `0x00` and `0x09..=0x7f` are literals; `0x01..=0x08` introduce that many literal bytes;
//! `0xc0..=0xff` stand for a space followed by `byte ^ 0x80`; `0x80..=0xbf` start a 2-byte
//! (distance, length) pair: `0x8000 | distance << 3 | (length - 3)` with distance 1..=2047 and
//! length 3..=10.

const NIL: u32 = u32::MAX;
const MAX_DIST: usize = 2047;
const MAX_LEN: usize = 10;
const MAX_CHAIN: usize = 48;

fn is_binary(b: u8) -> bool {
    (1..=8).contains(&b) || b >= 0x80
}

/// Compresses one record (at most 4096 bytes in a MOBI file).
pub fn compress(input: &[u8]) -> Vec<u8> {
    let n = input.len();
    let mut out = Vec::with_capacity(n / 2 + 16);
    let mut head = vec![NIL; 1 << 12];
    let mut prev = vec![NIL; n];
    let hash = |p: usize| {
        ((input[p] as usize) << 8 ^ (input[p + 1] as usize) << 4 ^ input[p + 2] as usize) & 0xfff
    };
    let insert = |p: usize, head: &mut Vec<u32>, prev: &mut Vec<u32>| {
        if p + 2 < n {
            let h = hash(p);
            prev[p] = head[h];
            head[h] = p as u32;
        }
    };

    let mut i = 0;
    while i < n {
        // longest match in the window
        let mut best_len = 0;
        let mut best_dist = 0;
        if i + 2 < n {
            let mut cand = head[hash(i)];
            let mut tries = 0;
            while cand != NIL && tries < MAX_CHAIN {
                let c = cand as usize;
                let dist = i - c;
                if dist > MAX_DIST {
                    break;
                }
                let limit = MAX_LEN.min(n - i);
                let mut l = 0;
                while l < limit && input[c + l] == input[i + l] {
                    l += 1;
                }
                if l > best_len {
                    best_len = l;
                    best_dist = dist;
                    if l == limit {
                        break;
                    }
                }
                cand = prev[c];
                tries += 1;
            }
        }
        if best_len >= 3 {
            let v: u16 = 0x8000 | ((best_dist as u16) << 3) | (best_len as u16 - 3);
            out.extend_from_slice(&v.to_be_bytes());
            for p in i..i + best_len {
                insert(p, &mut head, &mut prev);
            }
            i += best_len;
            continue;
        }
        let b = input[i];
        if b == 0x20 && i + 1 < n && (0x40..=0x7f).contains(&input[i + 1]) {
            out.push(input[i + 1] ^ 0x80);
            insert(i, &mut head, &mut prev);
            insert(i + 1, &mut head, &mut prev);
            i += 2;
        } else if is_binary(b) {
            let mut j = i;
            while j < n && j - i < 8 && is_binary(input[j]) {
                j += 1;
            }
            out.push((j - i) as u8);
            out.extend_from_slice(&input[i..j]);
            for p in i..j {
                insert(p, &mut head, &mut prev);
            }
            i = j;
        } else {
            out.push(b);
            insert(i, &mut head, &mut prev);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
pub(crate) fn reference_decompress(b: &[u8]) -> Vec<u8> {
    // Written from the format description above, independent of `compress`.
    let mut out: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        i += 1;
        match c {
            0 | 9..=0x7f => out.push(c),
            1..=8 => {
                out.extend_from_slice(&b[i..i + c as usize]);
                i += c as usize;
            }
            0xc0..=0xff => {
                out.push(b' ');
                out.push(c ^ 0x80);
            }
            _ => {
                let m = (((c as u16) << 8) | b[i] as u16) & 0x3fff;
                i += 1;
                let dist = (m >> 3) as usize;
                let len = (m & 7) as usize + 3;
                for _ in 0..len {
                    let v = out[out.len() - dist];
                    out.push(v);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(data: &[u8]) -> Vec<u8> {
        let c = compress(data);
        let d = reference_decompress(&c);
        assert_eq!(d, data, "roundtrip of {} bytes", data.len());
        c
    }

    #[test]
    fn empty_and_tiny_inputs() {
        assert!(compress(b"").is_empty());
        roundtrip(b"a");
        roundtrip(b"ab");
        roundtrip(b"abc");
        roundtrip(b" ");
    }

    #[test]
    fn space_plus_letter_is_one_byte() {
        let c = roundtrip(b"Hello, world");
        let mut want = b"Hello,".to_vec();
        // ' ' followed by 'w' (0x77) becomes 0x77 ^ 0x80
        want.push(0x77 ^ 0x80);
        want.extend_from_slice(b"orld");
        assert_eq!(c, want);
    }

    #[test]
    fn repeated_text_uses_back_references() {
        let data = "the quick brown fox jumps over the lazy dog. ".repeat(60);
        let c = roundtrip(data.as_bytes());
        assert!(c.len() < data.len() / 4, "{} -> {}", data.len(), c.len());
        // a pair starts with 0x80..=0xbf
        assert!(c.iter().any(|b| (0x80..=0xbf).contains(b)));
    }

    #[test]
    fn long_runs_overlap_correctly() {
        let c = roundtrip(&[b'a'; 4096]);
        assert!(c.len() < 900, "{}", c.len());
        roundtrip(&[0u8; 300]);
    }

    #[test]
    fn every_byte_value_roundtrips() {
        let all: Vec<u8> = (0..=255u8).collect();
        roundtrip(&all);
        let mut twice = all.clone();
        twice.extend_from_slice(&all);
        roundtrip(&twice);
        roundtrip(&[
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 0x80, 0x81, 0xff, 0xc0, 0x20, 0x20, 0x40,
        ]);
    }

    #[test]
    fn utf8_text_roundtrips() {
        let t = "Ünïcödé — “quotes” 日本語のテキスト ".repeat(40);
        roundtrip(t.as_bytes());
    }

    #[test]
    fn pseudo_random_data_roundtrips() {
        let mut x: u32 = 12345;
        let mut data = Vec::new();
        for _ in 0..4096 {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            data.push((x >> 24) as u8);
        }
        roundtrip(&data);
        // low-entropy variant with many matches
        let low: Vec<u8> = data.iter().map(|b| b & 3).collect();
        roundtrip(&low);
    }

    #[test]
    fn distance_limit_is_respected() {
        // a repeat 3000 bytes away cannot be referenced (max distance is 2047) but must still decode
        let mut data = b"UNIQUE-SEQUENCE".to_vec();
        data.extend((0..3000u32).map(|i| (i % 91 + 33) as u8));
        data.extend_from_slice(b"UNIQUE-SEQUENCE");
        roundtrip(&data);
    }

    #[test]
    fn match_pairs_encode_distance_and_length() {
        // "abcabc": 3 literals then a pair (distance 3, length 3)
        let c = compress(b"abcabcX");
        assert_eq!(&c[..3], b"abc");
        let v = u16::from_be_bytes([c[3], c[4]]);
        assert_eq!(v, 0x8000 | (3 << 3));
        assert_eq!(c[5], b'X');
    }
}
