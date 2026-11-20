//! ID3v2 (2.2, 2.3, 2.4) reader: title, artist, narrator, series, chapters (`CHAP`) and the cover
//! (`APIC`). Bounded by [`crate::limits`]; a damaged tag gives what was read before the damage.

use crate::limits::*;
use crate::probe::{ChapterMark, Cover, Tags};
use crate::text::decode_text;

/// What an ID3v2 tag holds.
#[derive(Debug, Default, Clone)]
pub struct Id3 {
    pub tags: Tags,
    pub chapters: Vec<ChapterMark>,
    pub cover: Option<Cover>,
}

/// The size of the whole tag (header included) when `head` starts with an ID3v2 header.
pub fn tag_size(head: &[u8]) -> Option<usize> {
    if head.len() < 10 || &head[..3] != b"ID3" {
        return None;
    }
    let sz = syncsafe(&head[6..10])?;
    let footer = if head[5] & 0x10 != 0 { 10 } else { 0 };
    Some(10 + sz as usize + footer)
}

fn syncsafe(b: &[u8]) -> Option<u32> {
    if b.iter().any(|x| x & 0x80 != 0) {
        return None;
    }
    Some(b.iter().fold(0u32, |a, &x| (a << 7) | x as u32))
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn unsync(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        out.push(b[i]);
        if b[i] == 0xFF && i + 1 < b.len() && b[i + 1] == 0 {
            i += 1;
        }
        i += 1;
    }
    out
}

/// Reads a tag from `data` (the tag bytes, header first).
pub fn parse(data: &[u8]) -> Id3 {
    let mut out = Id3::default();
    if data.len() < 10 || &data[..3] != b"ID3" {
        return out;
    }
    let major = data[3];
    if !(2..=4).contains(&major) {
        return out;
    }
    let flags = data[5];
    let Some(size) = syncsafe(&data[6..10]) else {
        return out;
    };
    let end = (10 + size as usize).min(data.len()).min(MAX_ID3_BYTES);
    let mut body: Vec<u8> = data[10..end].to_vec();
    if flags & 0x80 != 0 && major < 4 {
        body = unsync(&body);
    }
    let mut pos = 0usize;
    if flags & 0x40 != 0 && major >= 3 && body.len() >= 4 {
        let ext = if major == 4 {
            syncsafe(&body[..4]).unwrap_or(0) as usize
        } else {
            be32(&body) as usize + 4
        };
        pos = ext.min(body.len());
    }
    parse_frames(&body[pos..], major, &mut out, 0);
    out
}

fn parse_frames(body: &[u8], major: u8, out: &mut Id3, depth: usize) {
    let id_len = if major == 2 { 3 } else { 4 };
    let hdr = if major == 2 { 6 } else { 10 };
    let mut pos = 0usize;
    let mut items = 0usize;
    let mut extras: Vec<(String, String)> = Vec::new();
    while pos + hdr <= body.len() && items < MAX_ITEMS {
        items += 1;
        let id = &body[pos..pos + id_len];
        if id[0] == 0 {
            break; // padding
        }
        if !id
            .iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            break;
        }
        let id = String::from_utf8_lossy(id).into_owned();
        let size = match major {
            2 => {
                ((body[pos + 3] as usize) << 16)
                    | ((body[pos + 4] as usize) << 8)
                    | body[pos + 5] as usize
            }
            3 => be32(&body[pos + 4..]) as usize,
            _ => match syncsafe(&body[pos + 4..pos + 8]) {
                Some(s) => s as usize,
                None => be32(&body[pos + 4..]) as usize,
            },
        };
        let fflags = if major == 2 { 0 } else { body[pos + 9] };
        pos += hdr;
        let end = pos.saturating_add(size).min(body.len());
        let mut data: Vec<u8> = body[pos..end].to_vec();
        pos = end;
        if major == 4 {
            if fflags & 0x02 != 0 {
                data = unsync(&data);
            }
            if fflags & 0x01 != 0 && data.len() >= 4 {
                data.drain(..4); // data length indicator
            }
        }
        if major == 3 && fflags & 0x80 != 0 {
            continue; // compressed frames are not supported
        }
        frame(&id, &data, major, out, &mut extras, depth);
    }
    for (k, v) in extras {
        match k.to_ascii_lowercase().as_str() {
            "narrator" | "narrated by" | "reader" => out.tags.narrator = v,
            "series" | "series name" => out.tags.series = v,
            "series-part" | "series part" | "part" | "series_part" => {
                out.tags.series_index = v.trim().parse().ok()
            }
            "language" => out.tags.language = v,
            _ => {}
        }
    }
}

fn split_nul(b: &[u8], enc: u8) -> (&[u8], &[u8]) {
    let wide = enc == 1 || enc == 2;
    if wide {
        let mut i = 0;
        while i + 1 < b.len() {
            if b[i] == 0 && b[i + 1] == 0 {
                return (&b[..i], &b[i + 2..]);
            }
            i += 2;
        }
    } else if let Some(i) = b.iter().position(|&c| c == 0) {
        return (&b[..i], &b[i + 1..]);
    }
    (b, &[])
}

fn texts(enc: u8, b: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = b;
    for _ in 0..16 {
        let (a, r) = split_nul(rest, enc);
        let t = decode_text(enc, a);
        if !t.is_empty() {
            out.push(t);
        }
        if r.is_empty() {
            break;
        }
        rest = r;
    }
    out
}

fn frame(
    id: &str,
    data: &[u8],
    major: u8,
    out: &mut Id3,
    extras: &mut Vec<(String, String)>,
    depth: usize,
) {
    let tag = if major == 2 { v22_to_v23(id) } else { id };
    let t = &mut out.tags;
    match tag {
        "TIT2" | "TPE1" | "TALB" | "TCOM" | "TPE2" | "TCON" | "TYER" | "TDRC" | "TRCK" | "TPOS"
        | "TLAN" | "TIT1" | "GRP1" | "MVNM" | "MVIN" | "TPE3" => {
            let Some((&enc, rest)) = data.split_first() else {
                return;
            };
            let v = texts(enc, rest);
            let first = v.first().cloned().unwrap_or_default();
            match tag {
                "TIT2" => t.title = first,
                "TPE1" => t.authors = v,
                "TPE2" => {
                    if t.authors.is_empty() {
                        t.authors = v;
                    }
                }
                "TALB" => t.album = first,
                // iTunes and Audible keep the narrator as composer
                "TCOM" => t.composer = first,
                "TPE3" => t.conductor = first,
                "TCON" => t.genre = first,
                "TYER" | "TDRC" => t.year = first.chars().take(4).collect(),
                "TRCK" => t.track = first.split('/').next().and_then(|n| n.trim().parse().ok()),
                "TPOS" => t.disc = first.split('/').next().and_then(|n| n.trim().parse().ok()),
                "TLAN" => t.language = first,
                "TIT1" | "GRP1" | "MVNM" => {
                    if t.series.is_empty() {
                        t.series = first;
                    }
                }
                "MVIN" => t.series_index = first.trim().parse().ok(),
                _ => {}
            }
        }
        "TXXX" => {
            let Some((&enc, rest)) = data.split_first() else {
                return;
            };
            let (k, v) = split_nul(rest, enc);
            let (k, v) = (decode_text(enc, k), decode_text(enc, v));
            if !v.is_empty() {
                extras.push((k, v));
            }
        }
        "COMM" | "USLT" => {
            if data.len() < 5 {
                return;
            }
            let enc = data[0];
            let (_, text) = split_nul(&data[4..], enc);
            let c = decode_text(enc, text);
            if t.comment.is_empty() {
                t.comment = c;
            }
        }
        "APIC" => {
            if out.cover.is_some() || data.len() < 4 {
                return;
            }
            let enc = data[0];
            let rest = &data[1..];
            let (mime, rest) = if major == 2 {
                if rest.len() < 3 {
                    return;
                }
                (
                    String::from_utf8_lossy(&rest[..3]).to_ascii_lowercase(),
                    &rest[3..],
                )
            } else {
                let (m, r) = split_nul(rest, 0);
                (String::from_utf8_lossy(m).to_ascii_lowercase(), r)
            };
            if rest.is_empty() {
                return;
            }
            let (_, pic) = split_nul(&rest[1..], enc);
            if pic.is_empty() || pic.len() > MAX_COVER_BYTES {
                return;
            }
            if let Some(c) = Cover::from_bytes(pic, &mime) {
                out.cover = Some(c);
            }
        }
        "CHAP" => {
            if depth > 0 || out.chapters.len() >= MAX_CHAPTERS {
                return;
            }
            let (_, rest) = split_nul(data, 0);
            if rest.len() < 16 {
                return;
            }
            let start = be32(rest) as u64;
            let mut sub = Id3::default();
            parse_frames(&rest[16..], major, &mut sub, depth + 1);
            out.chapters.push(ChapterMark {
                title: sub.tags.title,
                start_ms: start,
            });
        }
        _ => {}
    }
}

fn v22_to_v23(id: &str) -> &str {
    match id {
        "TT2" => "TIT2",
        "TP1" => "TPE1",
        "TP2" => "TPE2",
        "TP3" => "TPE3",
        "TAL" => "TALB",
        "TCM" => "TCOM",
        "TCO" => "TCON",
        "TYE" => "TYER",
        "TRK" => "TRCK",
        "TPA" => "TPOS",
        "TLA" => "TLAN",
        "TT1" => "TIT1",
        "TXX" => "TXXX",
        "COM" => "COMM",
        "ULT" => "USLT",
        "PIC" => "APIC",
        "GP1" => "GRP1",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame23(id: &str, data: &[u8]) -> Vec<u8> {
        let mut v = id.as_bytes().to_vec();
        v.extend_from_slice(&(data.len() as u32).to_be_bytes());
        v.extend_from_slice(&[0, 0]);
        v.extend_from_slice(data);
        v
    }

    fn tag(major: u8, frames: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = frames.concat();
        let n = body.len() as u32;
        let mut v = b"ID3".to_vec();
        v.extend_from_slice(&[major, 0, 0]);
        v.extend_from_slice(&[
            (n >> 21) as u8 & 0x7f,
            (n >> 14) as u8 & 0x7f,
            (n >> 7) as u8 & 0x7f,
            n as u8 & 0x7f,
        ]);
        v.extend_from_slice(&body);
        v
    }

    fn text(id: &str, s: &str) -> Vec<u8> {
        let mut d = vec![3];
        d.extend_from_slice(s.as_bytes());
        frame23(id, &d)
    }

    #[test]
    fn titles_authors_narrator_and_series_are_read() {
        let mut txxx = vec![3];
        txxx.extend_from_slice(b"SERIES\0Wayfarers");
        let mut part = vec![3];
        part.extend_from_slice(b"SERIES-PART\0 2.5");
        let t = tag(
            3,
            &[
                text("TIT2", "The Long Way"),
                text("TPE1", "Becky Chambers"),
                text("TALB", "The Long Way (Audio)"),
                text("TCOM", "Rachel Dulude"),
                text("TRCK", "3/12"),
                text("TDRC", "2014-07-29"),
                frame23("TXXX", &txxx),
                frame23("TXXX", &part),
            ],
        );
        let id3 = parse(&t);
        assert_eq!(id3.tags.title, "The Long Way");
        assert_eq!(id3.tags.authors, vec!["Becky Chambers"]);
        assert_eq!(id3.tags.composer, "Rachel Dulude");
        assert_eq!(id3.tags.track, Some(3));
        assert_eq!(id3.tags.year, "2014");
        assert_eq!(id3.tags.series, "Wayfarers");
        assert_eq!(id3.tags.series_index, Some(2.5));
        assert_eq!(tag_size(&t), Some(t.len()));
    }

    #[test]
    fn utf16_and_several_values() {
        let mut d = vec![1, 0xFF, 0xFE];
        for u in "Ann\u{0}Bo".encode_utf16() {
            d.extend_from_slice(&u.to_le_bytes());
        }
        let t = tag(3, &[frame23("TPE1", &d)]);
        // the NUL is inside the BOM string: wide strings end at a 16-bit NUL
        assert_eq!(parse(&t).tags.authors, vec!["Ann", "Bo"]);
    }

    #[test]
    fn chapters_come_with_their_titles() {
        let sub = text("TIT2", "Chapter One");
        let mut chap = b"ch0\0".to_vec();
        chap.extend_from_slice(&0u32.to_be_bytes());
        chap.extend_from_slice(&5000u32.to_be_bytes());
        chap.extend_from_slice(&[0xFF; 8]);
        chap.extend_from_slice(&sub);
        let sub2 = text("TIT2", "Two");
        let mut chap2 = b"ch1\0".to_vec();
        chap2.extend_from_slice(&5000u32.to_be_bytes());
        chap2.extend_from_slice(&9000u32.to_be_bytes());
        chap2.extend_from_slice(&[0xFF; 8]);
        chap2.extend_from_slice(&sub2);
        let t = tag(3, &[frame23("CHAP", &chap), frame23("CHAP", &chap2)]);
        let c = parse(&t).chapters;
        assert_eq!(c.len(), 2);
        assert_eq!((c[0].title.as_str(), c[0].start_ms), ("Chapter One", 0));
        assert_eq!((c[1].title.as_str(), c[1].start_ms), ("Two", 5000));
    }

    #[test]
    fn the_cover_is_found_only_when_it_is_a_picture() {
        let png = crate::testgen::png_1x1();
        let mut apic = vec![0];
        apic.extend_from_slice(b"image/png\0");
        apic.push(3);
        apic.extend_from_slice(b"front\0");
        apic.extend_from_slice(&png);
        let t = tag(3, &[frame23("APIC", &apic)]);
        let c = parse(&t).cover.expect("cover");
        assert_eq!(c.ext, "png");
        assert_eq!(c.data, png);
        let mut bad = vec![0];
        bad.extend_from_slice(b"image/png\0\x03x\0not a picture");
        assert!(parse(&tag(3, &[frame23("APIC", &bad)])).cover.is_none());
    }

    #[test]
    fn version_2_2_frames_are_read() {
        let mut d = vec![0];
        d.extend_from_slice(b"Old Title");
        let mut f = b"TT2".to_vec();
        f.extend_from_slice(&[0, 0, d.len() as u8]);
        f.extend_from_slice(&d);
        assert_eq!(parse(&tag(2, &[f])).tags.title, "Old Title");
    }

    #[test]
    fn hostile_tags_do_not_panic_or_allocate_wildly() {
        // a frame that claims 4 GiB
        let mut f = b"TIT2".to_vec();
        f.extend_from_slice(&u32::MAX.to_be_bytes());
        f.extend_from_slice(&[0, 0, 3, b'a']);
        let t = tag(3, &[f]);
        let _ = parse(&t);
        // header says huge, data is short
        let mut h = b"ID3\x04\0\0\x7f\x7f\x7f\x7f".to_vec();
        h.extend_from_slice(b"TIT2");
        let _ = parse(&h);
        // every prefix of a real tag, and every single byte flipped
        let real = tag(
            3,
            &[text("TIT2", "x"), text("TPE1", "y"), {
                let mut c = b"c\0".to_vec();
                c.extend_from_slice(&[0; 16]);
                frame23("CHAP", &c)
            }],
        );
        for n in 0..real.len() {
            let _ = parse(&real[..n]);
        }
        for i in 0..real.len() {
            for v in [0u8, 0x80, 0xFF] {
                let mut c = real.clone();
                c[i] = v;
                let _ = parse(&c);
            }
        }
        // nested chapters do not recurse
        let mut inner = b"a\0".to_vec();
        inner.extend_from_slice(&[0; 16]);
        let deep = frame23("CHAP", &inner);
        let mut outer = b"b\0".to_vec();
        outer.extend_from_slice(&[0; 16]);
        outer.extend_from_slice(&deep);
        assert_eq!(parse(&tag(3, &[frame23("CHAP", &outer)])).chapters.len(), 1);
    }

    #[test]
    fn unsynchronised_tags_are_repaired() {
        assert_eq!(unsync(&[1, 0xFF, 0, 2]), vec![1, 0xFF, 2]);
        assert!(syncsafe(&[0x80, 0, 0, 0]).is_none());
        assert!(parse(b"ID3\x09\0\0\0\0\0\0").tags.title.is_empty());
    }
}
