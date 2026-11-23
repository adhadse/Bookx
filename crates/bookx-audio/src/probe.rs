//! Tags, chapters, cover and duration of one audio file, read without decoding it.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::format::{Format, detect};
use crate::limits::*;
use crate::text::decode_text;

/// What the tags say about the book (any field may be empty).
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tags {
    pub title: String,
    pub authors: Vec<String>,
    pub album: String,
    pub composer: String,
    pub conductor: String,
    pub narrator: String,
    pub series: String,
    pub series_index: Option<f64>,
    pub genre: String,
    pub year: String,
    pub track: Option<u32>,
    pub disc: Option<u32>,
    pub language: String,
    pub comment: String,
}

impl Tags {
    /// The narrator: the explicit field, else the composer field that Audible and iTunes use.
    pub fn narrator_or_composer(&self) -> &str {
        if !self.narrator.is_empty() {
            &self.narrator
        } else {
            &self.composer
        }
    }

    /// The book title: the album when there is one (parts of a book share it), else the title.
    pub fn book_title(&self) -> &str {
        if !self.album.is_empty() {
            &self.album
        } else {
            &self.title
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChapterMark {
    pub title: String,
    pub start_ms: u64,
}

/// A cover picture whose bytes really look like an image.
#[derive(Debug, Clone, PartialEq)]
pub struct Cover {
    /// `jpg`, `png`, `gif` or `webp`.
    pub ext: String,
    pub data: Vec<u8>,
}

impl Cover {
    /// `None` unless the bytes start like a JPEG, PNG, GIF or WebP (the declared `mime` is not trusted).
    pub fn from_bytes(data: &[u8], _mime: &str) -> Option<Cover> {
        let ext = if data.starts_with(&[0x89, b'P', b'N', b'G']) {
            "png"
        } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
            "jpg"
        } else if data.starts_with(b"GIF8") {
            "gif"
        } else if data.len() > 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
            "webp"
        } else {
            return None;
        };
        Some(Cover {
            ext: ext.into(),
            data: data.to_vec(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct FileInfo {
    pub format: Format,
    pub duration_ms: u64,
    pub sample_rate: u32,
    pub channels: u8,
    pub tags: Tags,
    pub chapters: Vec<ChapterMark>,
    pub cover: Option<Cover>,
}

/// Reads what `path` says about itself. Fails for files that are not audio.
pub fn probe_file(path: &Path) -> Result<FileInfo> {
    let mut f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut head = vec![0u8; 4096];
    let n = f.read(&mut head)?;
    head.truncate(n);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let Some(format) = detect(&head, &ext) else {
        bail!("{} is not an audio file Bookx knows", path.display());
    };
    probe_with(&mut f, format, path)
}

fn probe_with(f: &mut File, format: Format, path: &Path) -> Result<FileInfo> {
    let mut info = FileInfo {
        format,
        duration_ms: 0,
        sample_rate: 0,
        channels: 0,
        tags: Tags::default(),
        chapters: Vec::new(),
        cover: None,
    };
    match format {
        Format::Mp3 => {
            f.seek(SeekFrom::Start(0))?;
            let mut hdr = [0u8; 10];
            if f.read_exact(&mut hdr).is_ok() {
                if let Some(size) = crate::id3::tag_size(&hdr) {
                    let want = size.min(MAX_ID3_BYTES);
                    f.seek(SeekFrom::Start(0))?;
                    let mut buf = Vec::with_capacity(want);
                    f.take(want as u64).read_to_end(&mut buf)?;
                    let id3 = crate::id3::parse(&buf);
                    info.tags = id3.tags;
                    info.chapters = id3.chapters;
                    info.cover = id3.cover;
                }
            }
            if info.tags.title.is_empty() && info.tags.authors.is_empty() {
                read_id3v1(f, &mut info.tags);
            }
            if let Some(d) = crate::decode::stream_info(path) {
                info.duration_ms = d.duration_ms;
                info.sample_rate = d.rate;
                info.channels = d.channels;
            }
        }
        Format::M4a | Format::M4b => {
            let m = crate::mp4::read_info(f).context("not a readable MP4 file")?;
            info.duration_ms = m.duration_ms;
            info.tags = m.tags;
            info.chapters = m.chapters;
            info.cover = m.cover;
            if let Some(d) = crate::decode::stream_info(path) {
                info.sample_rate = d.rate;
                info.channels = d.channels;
            }
        }
        Format::Ogg | Format::Opus => {
            let o = crate::ogg::read_info(f).context("not a readable Ogg file")?;
            info.duration_ms = o.duration_ms;
            info.sample_rate = o.sample_rate;
            info.channels = o.channels;
            info.tags = o.comments.tags;
            info.chapters = o.comments.chapters;
            info.cover = o.comments.cover;
        }
        Format::Flac => {
            let meta = read_flac_metadata(f)?;
            if let Some((rate, ch, total)) = crate::vorbis::flac_streaminfo(&meta) {
                info.sample_rate = rate;
                info.channels = ch;
                if rate > 0 {
                    info.duration_ms = total.saturating_mul(1000) / rate as u64;
                }
            }
            let c = crate::vorbis::parse_flac(&meta);
            info.tags = c.tags;
            info.chapters = c.chapters;
            info.cover = c.cover;
        }
        Format::Wav => {
            f.seek(SeekFrom::Start(0))?;
            read_wav(f, &mut info)?;
        }
    }
    info.chapters.truncate(MAX_CHAPTERS);
    info.chapters.sort_by_key(|c| c.start_ms);
    // chapters that start after the end of the file are damage
    if info.duration_ms > 0 {
        let d = info.duration_ms;
        info.chapters.retain(|c| c.start_ms < d);
    }
    Ok(info)
}

fn read_id3v1(f: &mut File, t: &mut Tags) {
    let Ok(len) = f.seek(SeekFrom::End(0)) else {
        return;
    };
    if len < 128 || f.seek(SeekFrom::Start(len - 128)).is_err() {
        return;
    }
    let mut b = [0u8; 128];
    if f.read_exact(&mut b).is_err() || &b[..3] != b"TAG" {
        return;
    }
    t.title = decode_text(0, &b[3..33]);
    let a = decode_text(0, &b[33..63]);
    if !a.is_empty() {
        t.authors = vec![a];
    }
    t.album = decode_text(0, &b[63..93]);
}

/// The metadata blocks of a FLAC file, re-assembled after `fLaC` (the audio frames are skipped).
fn read_flac_metadata(f: &mut File) -> Result<Vec<u8>> {
    f.seek(SeekFrom::Start(0))?;
    let mut out = vec![0u8; 4];
    f.read_exact(&mut out)?;
    let mut total = 0usize;
    for _ in 0..MAX_ITEMS {
        let mut h = [0u8; 4];
        if f.read_exact(&mut h).is_err() {
            break;
        }
        let last = h[0] & 0x80 != 0;
        let kind = h[0] & 0x7F;
        let len = ((h[1] as usize) << 16) | ((h[2] as usize) << 8) | h[3] as usize;
        if matches!(kind, 0 | 4 | 6) && total + len <= MAX_ID3_BYTES {
            out.extend_from_slice(&h);
            let at = out.len();
            out.resize(at + len, 0);
            if f.read_exact(&mut out[at..]).is_err() {
                out.truncate(at);
                break;
            }
            total += len;
        } else {
            f.seek(SeekFrom::Current(len as i64))?;
        }
        if last {
            break;
        }
    }
    Ok(out)
}

fn read_wav(f: &mut File, info: &mut FileInfo) -> Result<()> {
    let mut riff = [0u8; 12];
    f.read_exact(&mut riff)?;
    let (mut rate, mut ch, mut bits) = (0u32, 0u16, 0u16);
    let mut data_len: Option<u64> = None;
    let mut align = 0u16;
    for _ in 0..1000 {
        let mut h = [0u8; 8];
        if f.read_exact(&mut h).is_err() {
            break;
        }
        let len = u32::from_le_bytes([h[4], h[5], h[6], h[7]]) as u64;
        let pad = len & 1;
        match &h[..4] {
            b"fmt " if len >= 16 => {
                let mut b = [0u8; 16];
                f.read_exact(&mut b)?;
                ch = u16::from_le_bytes([b[2], b[3]]);
                rate = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
                align = u16::from_le_bytes([b[12], b[13]]);
                bits = u16::from_le_bytes([b[14], b[15]]);
                f.seek(SeekFrom::Current((len - 16 + pad) as i64))?;
            }
            b"data" => {
                data_len = Some(len);
                let here = f.stream_position()?;
                let file_len = f.seek(SeekFrom::End(0))?;
                let avail = file_len.saturating_sub(here);
                // streamed files write 0 or 0xFFFFFFFF for the size
                data_len = data_len.map(|d| if d == 0 || d > avail { avail } else { d });
                f.seek(SeekFrom::Start(
                    here + data_len.unwrap_or(0).min(avail) + pad,
                ))?;
            }
            b"LIST" if (4..=65536).contains(&len) => {
                let mut b = vec![0u8; len as usize];
                f.read_exact(&mut b)?;
                if b.starts_with(b"INFO") {
                    read_riff_info(&b[4..], &mut info.tags);
                }
                f.seek(SeekFrom::Current(pad as i64))?;
            }
            _ => {
                f.seek(SeekFrom::Current((len + pad) as i64))?;
            }
        }
    }
    if rate == 0 || ch == 0 {
        bail!("a WAV file without a format chunk");
    }
    info.sample_rate = rate;
    info.channels = ch.min(255) as u8;
    let frame = if align > 0 {
        align as u64
    } else {
        (bits as u64 / 8).max(1) * ch as u64
    };
    if let Some(d) = data_len {
        info.duration_ms = d / frame.max(1) * 1000 / rate as u64;
    }
    Ok(())
}

fn read_riff_info(mut b: &[u8], t: &mut Tags) {
    while b.len() >= 8 {
        let len = u32::from_le_bytes([b[4], b[5], b[6], b[7]]) as usize;
        let end = (8 + len).min(b.len());
        let v = decode_text(3, &b[8..end]);
        match &b[..4] {
            b"INAM" => t.title = v,
            b"IART" => t.authors = vec![v],
            b"IPRD" => t.album = v,
            b"ICMT" => t.comment = v,
            b"IGNR" => t.genre = v,
            _ => {}
        }
        let next = (end + (len & 1)).min(b.len());
        b = &b[next..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgen::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-audio-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d.join(name)
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    #[test]
    fn wav_duration_and_info_tags() {
        let p = tmp("a.wav");
        std::fs::write(&p, wav_silence(8000, 1500)).unwrap();
        let i = probe_file(&p).unwrap();
        assert_eq!(i.format, Format::Wav);
        assert_eq!((i.duration_ms, i.sample_rate, i.channels), (1500, 8000, 1));
        // with a LIST INFO chunk before the data
        let mut w = wav_silence(8000, 1000);
        let mut info = b"INFO".to_vec();
        info.extend_from_slice(b"INAM\x05\0\0\0Tale\0\0");
        info.extend_from_slice(b"IART\x04\0\0\0Ann\0");
        let mut list = b"LIST".to_vec();
        list.extend_from_slice(&(info.len() as u32).to_le_bytes());
        list.extend_from_slice(&info);
        let at = 36;
        w.splice(at..at, list);
        std::fs::write(&p, &w).unwrap();
        let i = probe_file(&p).unwrap();
        assert_eq!(i.tags.title, "Tale");
        assert_eq!(i.tags.authors, vec!["Ann"]);
        assert_eq!(i.duration_ms, 1000);
    }

    #[test]
    fn generated_flac_is_probed() {
        let p = tmp("a.flac");
        std::fs::write(
            &p,
            flac_silence(8000, 20000, &[("TITLE", "Quiet"), ("COMPOSER", "Nan")]),
        )
        .unwrap();
        let i = probe_file(&p).unwrap();
        assert_eq!(i.format, Format::Flac);
        assert_eq!(i.duration_ms, 2500);
        assert_eq!(i.tags.title, "Quiet");
        assert_eq!(i.tags.narrator_or_composer(), "Nan");
    }

    #[test]
    fn real_files_from_ffmpeg() {
        let mp3 = probe_file(&fixture("tiny.mp3")).unwrap();
        assert_eq!(mp3.format, Format::Mp3);
        assert_eq!(mp3.tags.title, "Tiny Tale");
        assert_eq!(mp3.tags.authors, vec!["Ann Author"]);
        assert_eq!(mp3.tags.narrator_or_composer(), "Nora Narrator");
        assert!(
            (1900..=2200).contains(&mp3.duration_ms),
            "{}",
            mp3.duration_ms
        );
        assert_eq!(mp3.sample_rate, 8000);
        let c = probe_file(&fixture("tiny_cover.mp3")).unwrap();
        assert_eq!(c.cover.as_ref().map(|c| c.ext.as_str()), Some("png"));
        let m4b = probe_file(&fixture("tiny.m4b")).unwrap();
        assert_eq!(m4b.format, Format::M4b);
        assert_eq!(m4b.chapters.len(), 2);
        assert_eq!(m4b.tags.narrator_or_composer(), "Nora Narrator");
        let flac = probe_file(&fixture("tiny.flac")).unwrap();
        assert_eq!(flac.tags.title, "Tiny Flac");
        assert_eq!(flac.duration_ms, 2000);
        let ogg = probe_file(&fixture("tiny.ogg")).unwrap();
        assert_eq!(ogg.format, Format::Ogg);
        let opus = probe_file(&fixture("tiny.opus")).unwrap();
        assert_eq!(opus.format, Format::Opus);
        assert!(!opus.format.playable());
    }

    #[test]
    fn chapters_past_the_end_are_dropped_and_sorted() {
        let p = tmp("b.m4b");
        std::fs::write(
            &p,
            mp4_file(&Mp4Spec {
                duration_ms: 10_000,
                chapters: vec![("B".into(), 5000), ("A".into(), 0), ("Z".into(), 99_000)],
                ..Default::default()
            }),
        )
        .unwrap();
        let i = probe_file(&p).unwrap();
        let t: Vec<_> = i.chapters.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(t, vec!["A", "B"]);
    }

    #[test]
    fn not_audio_is_refused() {
        let p = tmp("x.mp3");
        std::fs::write(&p, b"this is a text file named like music").unwrap();
        assert!(probe_file(&p).is_err());
        assert!(probe_file(&p.with_file_name("missing.mp3")).is_err());
        let w = tmp("bad.wav");
        std::fs::write(&w, b"RIFF\0\0\0\0WAVEjunk").unwrap();
        assert!(probe_file(&w).is_err());
    }

    #[test]
    fn covers_must_be_images() {
        assert!(Cover::from_bytes(b"<html>", "image/png").is_none());
        assert_eq!(Cover::from_bytes(&png_1x1(), "").unwrap().ext, "png");
        assert_eq!(
            Cover::from_bytes(&[0xFF, 0xD8, 0xFF, 0xE0], "")
                .unwrap()
                .ext,
            "jpg"
        );
    }

    #[test]
    fn truncated_and_flipped_files_never_panic() {
        let base = tmp("f.flac");
        let good = flac_silence(8000, 9000, &[("TITLE", "t")]);
        for n in (0..good.len()).step_by(5) {
            std::fs::write(&base, &good[..n]).unwrap();
            let _ = probe_file(&base);
        }
        for i in (0..120.min(good.len())).step_by(1) {
            let mut c = good.clone();
            c[i] ^= 0xA5;
            std::fs::write(&base, &c).unwrap();
            let _ = probe_file(&base);
        }
        let w = tmp("g.wav");
        let good = wav_silence(8000, 100);
        for n in 0..good.len().min(80) {
            std::fs::write(&w, &good[..n]).unwrap();
            let _ = probe_file(&w);
        }
    }
}
