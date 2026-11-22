//! MP4 audio (`.m4a`, `.m4b`): duration, iTunes tags, the cover and chapters, from the Nero `chpl`
//! atom or from a QuickTime chapter text track. Only `moov` is read into memory (bounded); the audio
//! data is never touched here.

use std::io::{Read, Seek, SeekFrom};

use crate::limits::*;
use crate::probe::{ChapterMark, Cover, Tags};
use crate::text::decode_text;

#[derive(Debug, Clone, Default)]
pub struct Mp4Info {
    pub duration_ms: u64,
    pub tags: Tags,
    pub chapters: Vec<ChapterMark>,
    pub cover: Option<Cover>,
    pub brand: [u8; 4],
}

type Atom<'a> = ([u8; 4], &'a [u8]);

/// The child boxes of `data`, with sizes clamped to what is there. Stops at the first nonsense.
fn boxes(data: &[u8]) -> Vec<Atom<'_>> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + 8 <= data.len() && out.len() < MAX_ITEMS {
        let size =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as u64;
        let kind = [data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]];
        let (hdr, size) = match size {
            0 => (8u64, (data.len() - pos) as u64),
            1 => {
                if pos + 16 > data.len() {
                    break;
                }
                let big = u64::from_be_bytes(data[pos + 8..pos + 16].try_into().unwrap());
                (16, big)
            }
            n => (8, n),
        };
        if size < hdr {
            break;
        }
        let end = (pos as u64).saturating_add(size).min(data.len() as u64) as usize;
        out.push((kind, &data[pos + hdr as usize..end]));
        pos = end;
    }
    out
}

fn find<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(data).into_iter().find(|b| &b.0 == kind).map(|b| b.1)
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn be64(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// Reads the file's `ftyp` brand and `moov` box.
fn read_moov<R: Read + Seek>(r: &mut R) -> Option<(Vec<u8>, [u8; 4])> {
    let len = r.seek(SeekFrom::End(0)).ok()?;
    let mut pos = 0u64;
    let mut brand = *b"    ";
    for _ in 0..4096 {
        if pos + 8 > len {
            break;
        }
        r.seek(SeekFrom::Start(pos)).ok()?;
        let mut h = [0u8; 16];
        let n = r.read(&mut h[..8]).ok()?;
        if n < 8 {
            break;
        }
        let size32 = u32::from_be_bytes(h[..4].try_into().ok()?) as u64;
        let kind: [u8; 4] = h[4..8].try_into().ok()?;
        let (hdr, size) = match size32 {
            0 => (8, len - pos),
            1 => {
                r.read_exact(&mut h[8..16]).ok()?;
                (16, u64::from_be_bytes(h[8..16].try_into().ok()?))
            }
            n => (8, n),
        };
        if size < hdr {
            break;
        }
        if &kind == b"ftyp" && size >= 12 {
            let mut b = [0u8; 4];
            r.seek(SeekFrom::Start(pos + hdr)).ok()?;
            r.read_exact(&mut b).ok()?;
            brand = b;
        }
        if &kind == b"moov" {
            let body = size.min(len - pos) - hdr;
            if body > MAX_MOOV_BYTES {
                return None;
            }
            r.seek(SeekFrom::Start(pos + hdr)).ok()?;
            let mut v = Vec::with_capacity(body as usize);
            r.take(body).read_to_end(&mut v).ok()?;
            return Some((v, brand));
        }
        pos = pos.checked_add(size)?;
    }
    None
}

pub fn read_info<R: Read + Seek>(r: &mut R) -> Option<Mp4Info> {
    let (moov, brand) = read_moov(r)?;
    let mut info = Mp4Info {
        brand,
        ..Default::default()
    };
    {
        let mvhd = find(&moov, b"mvhd")?;
        let v = *mvhd.first()?;
        let (ts, dur) = if v == 1 {
            (be32(mvhd, 20)? as u64, be64(mvhd, 24)?)
        } else {
            (be32(mvhd, 12)? as u64, be32(mvhd, 16)? as u64)
        };
        if let Some(ms) = dur.saturating_mul(1000).checked_div(ts) {
            info.duration_ms = ms;
        }
    }
    let mut chpl: Vec<ChapterMark> = Vec::new();
    for (kind, body) in boxes(&moov) {
        match &kind {
            b"udta" => {
                for (k, b) in boxes(body) {
                    match &k {
                        b"chpl" => chpl = parse_chpl(b),
                        b"meta" => read_meta(b, &mut info),
                        _ => {}
                    }
                }
            }
            b"meta" => read_meta(body, &mut info),
            _ => {}
        }
    }
    info.chapters = chpl;
    if info.chapters.is_empty() {
        info.chapters = text_track_chapters(&moov, r).unwrap_or_default();
    }
    Some(info)
}

/// The Nero chapter list: `version/flags, [4 reserved if version 1], count, (start in 100 ns,
/// title length, title)*`.
pub fn parse_chpl(b: &[u8]) -> Vec<ChapterMark> {
    let mut out = Vec::new();
    let Some(&version) = b.first() else {
        return out;
    };
    let mut pos = 4 + if version == 1 { 4 } else { 0 };
    let Some(&count) = b.get(pos) else {
        return out;
    };
    pos += 1;
    for _ in 0..count {
        let (Some(start), Some(&len)) = (be64(b, pos), b.get(pos + 8)) else {
            break;
        };
        let t0 = pos + 9;
        let t1 = (t0 + len as usize).min(b.len());
        out.push(ChapterMark {
            title: decode_text(3, &b[t0..t1]),
            start_ms: start / 10_000,
        });
        pos = t1;
    }
    out.truncate(MAX_CHAPTERS);
    out
}

fn read_meta(b: &[u8], info: &mut Mp4Info) {
    // a full box (4 bytes of version and flags) in MP4, a plain box in old QuickTime files
    let inner = if b.len() >= 8 && &b[4..8] == b"hdlr" {
        b
    } else if b.len() >= 4 {
        &b[4..]
    } else {
        return;
    };
    if let Some(ilst) = find(inner, b"ilst") {
        read_ilst(ilst, info);
    }
}

fn data_atom(item: &[u8]) -> Option<(u32, &[u8])> {
    let d = find(item, b"data")?;
    let flags = be32(d, 0)? & 0x00FF_FFFF;
    Some((flags, d.get(8..)?))
}

fn read_ilst(ilst: &[u8], info: &mut Mp4Info) {
    for (kind, item) in boxes(ilst) {
        let t = &mut info.tags;
        if &kind == b"covr" {
            if info.cover.is_none() {
                if let Some((flags, p)) = data_atom(item) {
                    let mime = match flags {
                        13 => "image/jpeg",
                        14 => "image/png",
                        _ => "",
                    };
                    if p.len() <= MAX_COVER_BYTES {
                        info.cover = Cover::from_bytes(p, mime);
                    }
                }
            }
            continue;
        }
        if &kind == b"----" {
            let mean = find(item, b"mean").and_then(|m| m.get(4..)).unwrap_or(&[]);
            let name = find(item, b"name").and_then(|m| m.get(4..)).unwrap_or(&[]);
            let _ = mean;
            let key = decode_text(3, name).to_ascii_lowercase();
            if let Some((_, p)) = data_atom(item) {
                let v = decode_text(3, p);
                match key.as_str() {
                    "narrator" | "reader" => t.narrator = v,
                    "series" => t.series = v,
                    "series-part" | "series_part" | "part" => {
                        t.series_index = v.trim().parse().ok()
                    }
                    "language" => t.language = v,
                    _ => {}
                }
            }
            continue;
        }
        let Some((flags, p)) = data_atom(item) else {
            continue;
        };
        if &kind == b"trkn" || &kind == b"disk" {
            if p.len() >= 4 {
                let n = u16::from_be_bytes([p[2], p[3]]) as u32;
                if &kind == b"trkn" {
                    t.track = Some(n);
                } else {
                    t.disc = Some(n);
                }
            }
            continue;
        }
        if flags != 1 && flags != 0 {
            continue;
        }
        let v = decode_text(3, p);
        match &kind {
            b"\xa9nam" => t.title = v,
            b"\xa9ART" => t.authors = vec![v],
            b"aART" => {
                if t.authors.is_empty() {
                    t.authors = vec![v]
                }
            }
            b"\xa9alb" => t.album = v,
            b"\xa9nrt" => t.narrator = v,
            b"\xa9wrt" => t.composer = v,
            b"\xa9gen" => t.genre = v,
            b"\xa9day" => t.year = v.chars().take(4).collect(),
            b"\xa9cmt" | b"desc" | b"ldes" => {
                if t.comment.is_empty() {
                    t.comment = v
                }
            }
            b"\xa9grp" | b"tvsh" | b"\xa9mvn" => {
                if t.series.is_empty() {
                    t.series = v
                }
            }
            b"\xa9mvi" | b"tves" => t.series_index = v.trim().parse().ok(),
            _ => {}
        }
    }
}

struct Track<'a> {
    id: u32,
    timescale: u32,
    handler: [u8; 4],
    chap_refs: Vec<u32>,
    stbl: Option<&'a [u8]>,
}

fn tracks(moov: &[u8]) -> Vec<Track<'_>> {
    let mut out = Vec::new();
    for (kind, trak) in boxes(moov) {
        if &kind != b"trak" {
            continue;
        }
        let ver_off = |b: &[u8]| if b.first() == Some(&1) { 20 } else { 12 };
        let id = find(trak, b"tkhd")
            .and_then(|b| be32(b, ver_off(b)))
            .unwrap_or(0);
        let mdia = find(trak, b"mdia").unwrap_or(&[]);
        let timescale = find(mdia, b"mdhd")
            .and_then(|b| be32(b, ver_off(b)))
            .unwrap_or(0);
        let handler = find(mdia, b"hdlr")
            .and_then(|b| b.get(8..12))
            .map(|h| [h[0], h[1], h[2], h[3]])
            .unwrap_or([0; 4]);
        let mut chap_refs = Vec::new();
        if let Some(tref) = find(trak, b"tref") {
            if let Some(chap) = find(tref, b"chap") {
                chap_refs = chap
                    .chunks_exact(4)
                    .take(16)
                    .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
            }
        }
        let stbl = find(mdia, b"minf").and_then(|m| find(m, b"stbl"));
        out.push(Track {
            id,
            timescale,
            handler,
            chap_refs,
            stbl,
        });
    }
    out
}

fn text_track_chapters<R: Read + Seek>(moov: &[u8], r: &mut R) -> Option<Vec<ChapterMark>> {
    let all = tracks(moov);
    let want = all
        .iter()
        .find(|t| !t.chap_refs.is_empty())?
        .chap_refs
        .clone();
    let t = all.iter().find(|t| {
        want.contains(&t.id) && t.timescale > 0 && matches!(&t.handler, b"text" | b"sbtl")
    })?;
    let stbl = t.stbl?;
    let stsz = find(stbl, b"stsz")?;
    let fixed = be32(stsz, 4)?;
    let count = (be32(stsz, 8)? as usize).min(MAX_CHAPTERS);
    let size_of = |i: usize| -> Option<u32> {
        if fixed != 0 {
            Some(fixed)
        } else {
            be32(stsz, 12 + 4 * i)
        }
    };
    // time of each sample
    let stts = find(stbl, b"stts")?;
    let n_stts = (be32(stts, 4)? as usize).min(MAX_ITEMS);
    let mut starts: Vec<u64> = Vec::with_capacity(count);
    let mut t_acc = 0u64;
    'outer: for e in 0..n_stts {
        let c = be32(stts, 8 + 8 * e)? as u64;
        let d = be32(stts, 12 + 8 * e)? as u64;
        for _ in 0..c {
            if starts.len() >= count {
                break 'outer;
            }
            starts.push(t_acc);
            t_acc = t_acc.saturating_add(d);
        }
    }
    // file offset of each sample: chunks and the samples-per-chunk table
    let offsets: Vec<u64> = if let Some(co) = find(stbl, b"stco") {
        let n = (be32(co, 4)? as usize).min(MAX_ITEMS);
        (0..n)
            .filter_map(|i| be32(co, 8 + 4 * i).map(|v| v as u64))
            .collect()
    } else {
        let co = find(stbl, b"co64")?;
        let n = (be32(co, 4)? as usize).min(MAX_ITEMS);
        (0..n).filter_map(|i| be64(co, 8 + 8 * i)).collect()
    };
    let stsc = find(stbl, b"stsc")?;
    let n_stsc = (be32(stsc, 4)? as usize).min(MAX_ITEMS);
    let mut runs: Vec<(usize, usize)> = Vec::new(); // (first chunk, samples per chunk)
    for e in 0..n_stsc {
        runs.push((
            be32(stsc, 8 + 12 * e)? as usize,
            be32(stsc, 12 + 12 * e)? as usize,
        ));
    }
    let mut sample_off: Vec<(u64, u32)> = Vec::new();
    let mut idx = 0usize;
    for (ci, &base) in offsets.iter().enumerate() {
        let chunk_no = ci + 1;
        let per = runs
            .iter()
            .rev()
            .find(|(first, _)| *first <= chunk_no)
            .map(|r| r.1)
            .unwrap_or(1);
        let mut off = base;
        for _ in 0..per {
            if idx >= count {
                break;
            }
            let sz = size_of(idx)?;
            sample_off.push((off, sz));
            off = off.saturating_add(sz as u64);
            idx += 1;
        }
        if idx >= count {
            break;
        }
    }
    let mut out = Vec::new();
    for (i, (off, sz)) in sample_off.iter().enumerate() {
        if *sz < 2 || *sz > 4096 || i >= starts.len() {
            continue;
        }
        r.seek(SeekFrom::Start(*off)).ok()?;
        let mut buf = vec![0u8; *sz as usize];
        if r.read_exact(&mut buf).is_err() {
            break;
        }
        let len = u16::from_be_bytes([buf[0], buf[1]]) as usize;
        let body = &buf[2..(2 + len).min(buf.len())];
        let title = if body.starts_with(&[0xFE, 0xFF]) {
            decode_text(2, &body[2..])
        } else {
            decode_text(3, body)
        };
        out.push(ChapterMark {
            title,
            start_ms: starts[i].saturating_mul(1000) / t.timescale as u64,
        });
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgen::{Mp4Spec, mp4_file};
    use std::io::Cursor;

    #[test]
    fn chpl_chapters_tags_and_duration() {
        let f = mp4_file(&Mp4Spec {
            duration_ms: 90_000,
            chapters: vec![
                ("One".into(), 0),
                ("Two".into(), 30_000),
                ("Three".into(), 61_500),
            ],
            title: "The Book".into(),
            artist: "Ann".into(),
            narrator: "Nora".into(),
            series: "Saga".into(),
            cover: Some(crate::testgen::png_1x1()),
            ..Default::default()
        });
        let i = read_info(&mut Cursor::new(f)).unwrap();
        assert_eq!(i.duration_ms, 90_000);
        assert_eq!(i.tags.title, "The Book");
        assert_eq!(i.tags.authors, vec!["Ann"]);
        assert_eq!(i.tags.narrator, "Nora");
        assert_eq!(i.tags.series, "Saga");
        assert_eq!(i.cover.unwrap().ext, "png");
        assert_eq!(i.brand, *b"M4B ");
        let c: Vec<_> = i
            .chapters
            .iter()
            .map(|c| (c.title.as_str(), c.start_ms))
            .collect();
        assert_eq!(c, vec![("One", 0), ("Two", 30_000), ("Three", 61_500)]);
    }

    #[test]
    fn text_track_chapters_are_read_when_there_is_no_chpl() {
        let f = mp4_file(&Mp4Spec {
            duration_ms: 20_000,
            text_chapters: vec![
                ("Intro".into(), 0),
                ("Main".into(), 4_000),
                ("Ünï".into(), 12_000),
            ],
            ..Default::default()
        });
        let i = read_info(&mut Cursor::new(f)).unwrap();
        let c: Vec<_> = i
            .chapters
            .iter()
            .map(|c| (c.title.as_str(), c.start_ms))
            .collect();
        assert_eq!(c, vec![("Intro", 0), ("Main", 4_000), ("Ünï", 12_000)]);
    }

    #[test]
    fn a_real_ffmpeg_file_gives_both_kinds() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tiny.m4b");
        let i = read_info(&mut std::fs::File::open(p).unwrap()).unwrap();
        assert_eq!(i.tags.title, "Tiny Tale");
        assert_eq!(i.chapters.len(), 2);
        assert_eq!(i.chapters[1].title, "Second Part");
        assert!((i.chapters[1].start_ms as i64 - 1000).abs() < 5);
        assert!((2400..=2700).contains(&i.duration_ms), "{}", i.duration_ms);
    }

    #[test]
    fn chpl_parsing_is_bounded() {
        assert!(parse_chpl(&[]).is_empty());
        assert!(parse_chpl(&[1, 0, 0, 0]).is_empty());
        // claims 255 chapters but has bytes for none
        assert!(parse_chpl(&[0, 0, 0, 0, 255, 1, 2, 3]).is_empty());
        // a title longer than the data
        let mut b = vec![0, 0, 0, 0, 1];
        b.extend_from_slice(&10_000_000u64.to_be_bytes());
        b.push(200);
        b.extend_from_slice(b"short");
        let c = parse_chpl(&b);
        assert_eq!((c[0].title.as_str(), c[0].start_ms), ("short", 1000));
    }

    #[test]
    fn hostile_files_do_not_panic() {
        let good = mp4_file(&Mp4Spec {
            duration_ms: 5000,
            chapters: vec![("A".into(), 0), ("B".into(), 1000)],
            text_chapters: vec![("X".into(), 0)],
            title: "T".into(),
            cover: Some(crate::testgen::png_1x1()),
            ..Default::default()
        });
        for n in (0..good.len()).step_by(3) {
            let _ = read_info(&mut Cursor::new(good[..n].to_vec()));
        }
        for i in 0..good.len() {
            for v in [0u8, 0xFF, 0x7F] {
                let mut c = good.clone();
                c[i] = v;
                let _ = read_info(&mut Cursor::new(c));
            }
        }
        // a box that claims 2^64 - 1 bytes, and a moov that claims more than the limit
        let mut huge = vec![0, 0, 0, 1, b'm', b'o', b'o', b'v'];
        huge.extend_from_slice(&u64::MAX.to_be_bytes());
        assert!(read_info(&mut Cursor::new(huge)).is_none());
        let mut big = (u32::MAX).to_be_bytes().to_vec();
        big.extend_from_slice(b"moov");
        assert!(read_info(&mut Cursor::new(big)).is_none());
        // boxes nested into themselves forever (a box whose size is its header only)
        let loopy: Vec<u8> = std::iter::repeat_n([0, 0, 0, 8, b'm', b'o', b'o', b'v'], 5000)
            .flatten()
            .collect();
        let _ = read_info(&mut Cursor::new(loopy));
    }
}
