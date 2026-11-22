//! Vorbis comments (FLAC, Ogg Vorbis, Opus) and the FLAC metadata blocks.

use crate::limits::*;
use crate::probe::{ChapterMark, Cover, Tags};
use crate::text::{base64_decode, decode_text};

/// Comment fields the readers understand.
#[derive(Debug, Default, Clone)]
pub struct Comments {
    pub tags: Tags,
    pub chapters: Vec<ChapterMark>,
    pub cover: Option<Cover>,
}

fn le32(b: &[u8]) -> usize {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize
}

fn be32(b: &[u8]) -> usize {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize
}

/// Parses a comment block: `vendor_len vendor count (len "KEY=value")*`.
pub fn parse_comments(b: &[u8]) -> Comments {
    let mut out = Comments::default();
    if b.len() < 4 {
        return out;
    }
    let vlen = le32(b);
    let mut pos = 4usize.saturating_add(vlen);
    if pos.saturating_add(4) > b.len() {
        return out;
    }
    let count = le32(&b[pos..]).min(MAX_ITEMS);
    pos += 4;
    let mut chapter_titles: Vec<(usize, String)> = Vec::new();
    let mut chapter_starts: Vec<(usize, u64)> = Vec::new();
    for _ in 0..count {
        if pos + 4 > b.len() {
            break;
        }
        let len = le32(&b[pos..]);
        pos += 4;
        let end = pos.saturating_add(len).min(b.len());
        let field = &b[pos..end];
        pos = end;
        let Some(eq) = field.iter().position(|&c| c == b'=') else {
            continue;
        };
        let key = String::from_utf8_lossy(&field[..eq]).to_ascii_uppercase();
        let raw = &field[eq + 1..];
        if key == "METADATA_BLOCK_PICTURE" {
            if out.cover.is_none() {
                let data = base64_decode(raw, MAX_COVER_BYTES + 4096);
                out.cover = parse_picture(&data);
            }
            continue;
        }
        let val = decode_text(3, raw);
        let t = &mut out.tags;
        match key.as_str() {
            "TITLE" => t.title = val,
            "ARTIST" | "AUTHOR" => t.authors.push(val),
            "ALBUMARTIST" | "ALBUM ARTIST" => {
                if t.authors.is_empty() {
                    t.authors.push(val)
                }
            }
            "ALBUM" => t.album = val,
            "COMPOSER" => t.composer = val,
            "NARRATOR" | "READER" | "PERFORMER" => {
                if t.narrator.is_empty() {
                    t.narrator = val
                }
            }
            "GENRE" => t.genre = val,
            "DATE" | "YEAR" => t.year = val.chars().take(4).collect(),
            "TRACKNUMBER" => t.track = val.split('/').next().and_then(|n| n.trim().parse().ok()),
            "DISCNUMBER" => t.disc = val.split('/').next().and_then(|n| n.trim().parse().ok()),
            "LANGUAGE" => t.language = val,
            "COMMENT" | "DESCRIPTION" => {
                if t.comment.is_empty() {
                    t.comment = val
                }
            }
            "SERIES" | "SERIES_NAME" | "GROUPING" => {
                if t.series.is_empty() {
                    t.series = val
                }
            }
            "SERIES_PART" | "SERIES_INDEX" | "SERIESNUMBER" => {
                t.series_index = val.trim().parse().ok()
            }
            k => {
                // Ogg chapters: CHAPTER001=00:01:02.500 and CHAPTER001NAME=Title
                if let Some(rest) = k.strip_prefix("CHAPTER") {
                    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                    let n: usize = digits.parse().unwrap_or(usize::MAX);
                    if n != usize::MAX
                        && chapter_starts.len() + chapter_titles.len() < 2 * MAX_CHAPTERS
                    {
                        if rest.len() == digits.len() {
                            if let Some(ms) = parse_timestamp(&val) {
                                chapter_starts.push((n, ms));
                            }
                        } else if &rest[digits.len()..] == "NAME" {
                            chapter_titles.push((n, val));
                        }
                    }
                }
            }
        }
    }
    chapter_starts.sort();
    for (n, ms) in chapter_starts.into_iter().take(MAX_CHAPTERS) {
        let title = chapter_titles
            .iter()
            .find(|(m, _)| *m == n)
            .map(|(_, t)| t.clone())
            .unwrap_or_default();
        out.chapters.push(ChapterMark {
            title,
            start_ms: ms,
        });
    }
    out
}

/// `HH:MM:SS.mmm` (the Ogg chapter extension) to milliseconds.
pub fn parse_timestamp(s: &str) -> Option<u64> {
    let mut parts = s.trim().split(':');
    let h: u64 = parts.next()?.parse().ok()?;
    let m: u64 = parts.next()?.parse().ok()?;
    let sec = parts.next()?;
    if parts.next().is_some() || m > 59 && h > 0 {
        return None;
    }
    let (whole, frac) = sec.split_once('.').unwrap_or((sec, "0"));
    let sec_n: u64 = whole.parse().ok()?;
    let frac: String = frac.chars().take(3).collect();
    let ms: u64 = format!("{frac:0<3}").parse().ok()?;
    h.checked_mul(3_600_000)?
        .checked_add(m.checked_mul(60_000)?)?
        .checked_add(sec_n.checked_mul(1000)?)?
        .checked_add(ms)
}

/// A FLAC picture block (also the payload of `METADATA_BLOCK_PICTURE`).
pub fn parse_picture(b: &[u8]) -> Option<Cover> {
    if b.len() < 32 {
        return None;
    }
    let mut pos = 4; // picture type
    let mlen = be32(&b[pos..]);
    pos += 4;
    let mime_end = pos.checked_add(mlen)?;
    if mime_end + 4 > b.len() {
        return None;
    }
    let mime = String::from_utf8_lossy(&b[pos..mime_end]).to_ascii_lowercase();
    pos = mime_end;
    let dlen = be32(&b[pos..]);
    pos = pos.checked_add(4)?.checked_add(dlen)?;
    if pos.checked_add(20)? > b.len() {
        return None;
    }
    pos += 16; // width, height, depth, colours
    let n = be32(&b[pos..]);
    pos += 4;
    if n > MAX_COVER_BYTES || pos.checked_add(n)? > b.len() {
        return None;
    }
    Cover::from_bytes(&b[pos..pos + n], &mime)
}

/// Reads the metadata blocks at the start of a FLAC file (`data` starts with `fLaC`).
pub fn parse_flac(data: &[u8]) -> Comments {
    let mut out = Comments::default();
    if !data.starts_with(b"fLaC") {
        return out;
    }
    let mut pos = 4usize;
    for _ in 0..MAX_ITEMS {
        if pos + 4 > data.len() {
            break;
        }
        let last = data[pos] & 0x80 != 0;
        let kind = data[pos] & 0x7F;
        let len = ((data[pos + 1] as usize) << 16)
            | ((data[pos + 2] as usize) << 8)
            | data[pos + 3] as usize;
        pos += 4;
        let end = (pos + len).min(data.len());
        let body = &data[pos..end];
        match kind {
            4 => {
                let c = parse_comments(body);
                out.tags = c.tags;
                out.chapters = c.chapters;
                if out.cover.is_none() {
                    out.cover = c.cover;
                }
            }
            6 if out.cover.is_none() => {
                out.cover = parse_picture(body);
            }
            _ => {}
        }
        pos = end;
        if last || end >= data.len() {
            break;
        }
    }
    out
}

/// (sample rate, channels, total samples) from the STREAMINFO block.
pub fn flac_streaminfo(data: &[u8]) -> Option<(u32, u8, u64)> {
    if data.len() < 8 + 34 || !data.starts_with(b"fLaC") || data[4] & 0x7F != 0 {
        return None;
    }
    let s = &data[8..42];
    let rate = ((s[10] as u32) << 12) | ((s[11] as u32) << 4) | (s[12] as u32 >> 4);
    let ch = ((s[12] >> 1) & 7) + 1;
    let total = (((s[13] & 0x0F) as u64) << 32)
        | ((s[14] as u64) << 24)
        | ((s[15] as u64) << 16)
        | ((s[16] as u64) << 8)
        | s[17] as u64;
    Some((rate, ch, total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgen::vorbis_comments;

    #[test]
    fn comments_are_read() {
        let b = vorbis_comments(&[
            ("TITLE", "Tiny"),
            ("artist", "Ann"),
            ("ARTIST", "Bo"),
            ("COMPOSER", "Nora"),
            ("NARRATOR", "Nan"),
            ("SERIES", "Saga"),
            ("SERIES_PART", "3"),
            ("TRACKNUMBER", "4/10"),
            ("DATE", "1999-01-02"),
        ]);
        let c = parse_comments(&b);
        assert_eq!(c.tags.title, "Tiny");
        assert_eq!(c.tags.authors, vec!["Ann", "Bo"]);
        assert_eq!(c.tags.narrator, "Nan");
        assert_eq!(c.tags.series, "Saga");
        assert_eq!(c.tags.series_index, Some(3.0));
        assert_eq!(c.tags.track, Some(4));
        assert_eq!(c.tags.year, "1999");
    }

    #[test]
    fn ogg_chapter_comments_are_sorted_with_names() {
        let b = vorbis_comments(&[
            ("CHAPTER002", "00:01:30.5"),
            ("CHAPTER002NAME", "Two"),
            ("CHAPTER001", "00:00:00.000"),
            ("CHAPTER001NAME", "One"),
            ("CHAPTER003", "garbage"),
        ]);
        let c = parse_comments(&b).chapters;
        assert_eq!(c.len(), 2);
        assert_eq!((c[0].title.as_str(), c[0].start_ms), ("One", 0));
        assert_eq!((c[1].title.as_str(), c[1].start_ms), ("Two", 90_500));
    }

    #[test]
    fn timestamps() {
        assert_eq!(parse_timestamp("01:02:03.004"), Some(3_723_004));
        assert_eq!(parse_timestamp("0:0:1"), Some(1000));
        assert_eq!(parse_timestamp("0:0:1.5"), Some(1500));
        assert_eq!(parse_timestamp("1:2"), None);
        assert_eq!(parse_timestamp("a:b:c"), None);
        assert_eq!(parse_timestamp("99999999999999999999:0:0"), None);
    }

    #[test]
    fn a_picture_block_gives_the_cover() {
        let png = crate::testgen::png_1x1();
        let block = crate::testgen::flac_picture(&png, "image/png");
        let c = parse_picture(&block).unwrap();
        assert_eq!(c.ext, "png");
        assert_eq!(c.data, png);
        assert!(parse_picture(&block[..20]).is_none());
        let f = crate::testgen::flac_with_blocks(&[(6, block)]);
        assert!(parse_flac(&f).cover.is_some());
    }

    #[test]
    fn streaminfo_of_a_generated_flac() {
        let f = crate::testgen::flac_silence(8000, 8000, &[("TITLE", "x")]);
        let (rate, ch, total) = flac_streaminfo(&f).unwrap();
        assert_eq!((rate, ch, total), (8000, 1, 8000));
        assert_eq!(parse_flac(&f).tags.title, "x");
    }

    #[test]
    fn hostile_input_does_not_panic() {
        let real = vorbis_comments(&[("TITLE", "t"), ("CHAPTER001", "0:0:1")]);
        for n in 0..real.len() {
            let _ = parse_comments(&real[..n]);
        }
        for i in 0..real.len() {
            let mut c = real.clone();
            c[i] = 0xFF;
            let _ = parse_comments(&c);
        }
        let mut huge = vec![0xFF; 8];
        huge.extend_from_slice(&[0xFF; 32]);
        let _ = parse_comments(&huge);
        let _ = parse_picture(&huge);
        let _ = parse_flac(b"fLaC\x84\xFF\xFF\xFF");
        assert!(flac_streaminfo(b"fLaC").is_none());
    }
}
