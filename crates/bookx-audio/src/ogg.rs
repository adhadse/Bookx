//! Ogg (Vorbis and Opus): stream parameters, the comment header and the duration from the last
//! page, read without decoding.

use std::io::{Read, Seek, SeekFrom};

use crate::limits::*;
use crate::vorbis::{Comments, parse_comments};

#[derive(Debug, Clone, Default)]
pub struct OggInfo {
    pub opus: bool,
    pub sample_rate: u32,
    pub channels: u8,
    pub duration_ms: u64,
    pub comments: Comments,
}

struct Page {
    packets: Vec<(Vec<u8>, bool)>, // (bytes, complete)
}

/// Reads one page at the reader's position. `None` at the end or on a damaged header.
fn read_page<R: Read>(r: &mut R) -> Option<Page> {
    let mut h = [0u8; 27];
    r.read_exact(&mut h).ok()?;
    if &h[..4] != b"OggS" {
        return None;
    }
    let n = h[26] as usize;
    let mut segs = vec![0u8; n];
    r.read_exact(&mut segs).ok()?;
    let total: usize = segs.iter().map(|&s| s as usize).sum();
    let mut data = vec![0u8; total];
    r.read_exact(&mut data).ok()?;
    let mut packets = Vec::new();
    let (mut cur, mut off) = (Vec::new(), 0usize);
    for (i, &s) in segs.iter().enumerate() {
        cur.extend_from_slice(&data[off..off + s as usize]);
        off += s as usize;
        if s < 255 {
            packets.push((std::mem::take(&mut cur), true));
        } else if i + 1 == segs.len() {
            packets.push((std::mem::take(&mut cur), false));
        }
    }
    Some(Page { packets })
}

pub fn read_info<R: Read + Seek>(r: &mut R) -> Option<OggInfo> {
    r.seek(SeekFrom::Start(0)).ok()?;
    let mut info = OggInfo::default();
    let mut packets: Vec<Vec<u8>> = Vec::new();
    let mut pending: Vec<u8> = Vec::new();
    let mut pre_skip = 0u64;
    'pages: for _ in 0..MAX_OGG_PAGES {
        let Some(page) = read_page(r) else { break };
        for (bytes, complete) in page.packets {
            pending.extend_from_slice(&bytes);
            if pending.len() > MAX_ID3_BYTES {
                break 'pages;
            }
            if complete {
                packets.push(std::mem::take(&mut pending));
                if packets.len() == 2 {
                    break 'pages;
                }
            }
        }
    }
    let first = packets.first()?;
    if first.starts_with(b"\x01vorbis") && first.len() >= 16 {
        info.channels = first[11];
        info.sample_rate = u32::from_le_bytes(first[12..16].try_into().ok()?);
    } else if first.starts_with(b"OpusHead") && first.len() >= 12 {
        info.opus = true;
        info.channels = first[9];
        pre_skip = u16::from_le_bytes([first[10], first[11]]) as u64;
        info.sample_rate = 48_000;
    } else {
        return None;
    }
    if let Some(c) = packets.get(1) {
        let body = if info.opus && c.starts_with(b"OpusTags") {
            &c[8..]
        } else if !info.opus && c.starts_with(b"\x03vorbis") {
            &c[7..]
        } else {
            &[][..]
        };
        info.comments = parse_comments(body);
    }
    // the duration is the granule position of the last page
    let len = r.seek(SeekFrom::End(0)).ok()?;
    let back = len.min(128 * 1024);
    r.seek(SeekFrom::Start(len - back)).ok()?;
    let mut tail = Vec::with_capacity(back as usize);
    r.take(back).read_to_end(&mut tail).ok()?;
    let mut best: Option<u64> = None;
    let mut i = 0;
    while i + 27 <= tail.len() {
        if &tail[i..i + 4] == b"OggS" && tail[i + 4] == 0 {
            let g = u64::from_le_bytes(tail[i + 6..i + 14].try_into().ok()?);
            if g != u64::MAX {
                best = Some(g);
            }
        }
        i += 1;
    }
    if let (Some(g), true) = (best, info.sample_rate > 0) {
        let samples = g.saturating_sub(pre_skip);
        info.duration_ms = samples.saturating_mul(1000) / info.sample_rate as u64;
    }
    Some(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgen::ogg_stream;
    use std::io::Cursor;

    #[test]
    fn vorbis_headers_and_duration_are_read() {
        let f = ogg_stream(false, 22050, 44100, &[("TITLE", "Song"), ("ARTIST", "Ann")]);
        let i = read_info(&mut Cursor::new(f)).unwrap();
        assert!(!i.opus);
        assert_eq!((i.sample_rate, i.channels, i.duration_ms), (22050, 1, 2000));
        assert_eq!(i.comments.tags.title, "Song");
    }

    #[test]
    fn opus_duration_uses_48k_and_pre_skip() {
        let f = ogg_stream(true, 48000, 48000 + 312, &[("TITLE", "O")]);
        let i = read_info(&mut Cursor::new(f)).unwrap();
        assert!(i.opus);
        assert_eq!(i.duration_ms, 1000);
        assert_eq!(i.comments.tags.title, "O");
    }

    #[test]
    fn a_real_ogg_and_opus_file() {
        let d = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let ogg = std::fs::read(d.join("tiny.ogg")).unwrap();
        let i = read_info(&mut Cursor::new(ogg)).unwrap();
        assert_eq!(i.comments.tags.title, "Tiny Ogg");
        assert!((1900..=2100).contains(&i.duration_ms), "{}", i.duration_ms);
        let opus = std::fs::read(d.join("tiny.opus")).unwrap();
        let i = read_info(&mut Cursor::new(opus)).unwrap();
        assert!(i.opus);
        assert!((1900..=2100).contains(&i.duration_ms), "{}", i.duration_ms);
    }

    #[test]
    fn damaged_streams_give_nothing_and_do_not_panic() {
        assert!(read_info(&mut Cursor::new(b"OggS".to_vec())).is_none());
        assert!(read_info(&mut Cursor::new(vec![0u8; 100])).is_none());
        let f = ogg_stream(false, 8000, 8000, &[("TITLE", "x")]);
        for n in (0..f.len()).step_by(7) {
            let _ = read_info(&mut Cursor::new(f[..n].to_vec()));
        }
        for i in (0..f.len()).step_by(3) {
            let mut c = f.clone();
            c[i] ^= 0xFF;
            let _ = read_info(&mut Cursor::new(c));
        }
    }
}
