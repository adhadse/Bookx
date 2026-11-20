//! Tiny generated audio files for tests (silence and sine, written by hand: no encoder, no
//! copyrighted audio). Also used by the app's own tests and by the UI scenarios' fixture helper.

/// A valid 1x1 PNG.
pub fn png_1x1() -> Vec<u8> {
    vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8,
        0xFF, 0xFF, 0x3F, 0x00, 0x05, 0xFE, 0x02, 0xFE, 0xA7, 0x35, 0x81, 0x84, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ]
}

/// A WAV file of 16-bit samples (`samples` interleaved).
pub fn wav_samples(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut v = Vec::with_capacity(44 + samples.len() * 2);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&channels.to_le_bytes());
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
    v.extend_from_slice(&(channels * 2).to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}

/// A mono sine wave of `ms` milliseconds, as WAV.
pub fn wav_tone(rate: u32, ms: u64, hz: f64) -> Vec<u8> {
    let n = (rate as u64 * ms / 1000) as usize;
    let s: Vec<i16> = (0..n)
        .map(|i| ((i as f64 / rate as f64 * hz * std::f64::consts::TAU).sin() * 8000.0) as i16)
        .collect();
    wav_samples(rate, 1, &s)
}

/// A mono WAV of silence.
pub fn wav_silence(rate: u32, ms: u64) -> Vec<u8> {
    wav_samples(rate, 1, &vec![0i16; (rate as u64 * ms / 1000) as usize])
}

/// A Vorbis comment block (vendor, count, entries).
pub fn vorbis_comments(fields: &[(&str, &str)]) -> Vec<u8> {
    let mut v = Vec::new();
    let vendor = b"bookx-test";
    v.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    v.extend_from_slice(vendor);
    v.extend_from_slice(&(fields.len() as u32).to_le_bytes());
    for (k, val) in fields {
        let e = format!("{k}={val}");
        v.extend_from_slice(&(e.len() as u32).to_le_bytes());
        v.extend_from_slice(e.as_bytes());
    }
    v
}

/// A FLAC picture block body.
pub fn flac_picture(data: &[u8], mime: &str) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&3u32.to_be_bytes());
    v.extend_from_slice(&(mime.len() as u32).to_be_bytes());
    v.extend_from_slice(mime.as_bytes());
    v.extend_from_slice(&0u32.to_be_bytes());
    v.extend_from_slice(&[0; 16]);
    v.extend_from_slice(&(data.len() as u32).to_be_bytes());
    v.extend_from_slice(data);
    v
}

fn flac_streaminfo_block(rate: u32, total: u64) -> Vec<u8> {
    let mut s = vec![0u8; 34];
    s[0..2].copy_from_slice(&4096u16.to_be_bytes());
    s[2..4].copy_from_slice(&4096u16.to_be_bytes());
    s[10] = (rate >> 12) as u8;
    s[11] = (rate >> 4) as u8;
    // mono (channels - 1 = 0); bits per sample - 1 = 15: its low four bits are s[13]'s high nibble
    s[12] = ((rate & 0xF) as u8) << 4;
    s[13] = 0xF0 | ((total >> 32) as u8 & 0x0F);
    s[14..18].copy_from_slice(&(total as u32).to_be_bytes());
    s
}

fn block_header(last: bool, kind: u8, len: usize) -> [u8; 4] {
    [
        (if last { 0x80 } else { 0 }) | kind,
        (len >> 16) as u8,
        (len >> 8) as u8,
        len as u8,
    ]
}

/// `fLaC` + STREAMINFO (8 kHz mono, 0 samples) + the given blocks. Not playable.
pub fn flac_with_blocks(blocks: &[(u8, Vec<u8>)]) -> Vec<u8> {
    let mut v = b"fLaC".to_vec();
    let si = flac_streaminfo_block(8000, 0);
    v.extend_from_slice(&block_header(blocks.is_empty(), 0, si.len()));
    v.extend_from_slice(&si);
    for (i, (k, b)) in blocks.iter().enumerate() {
        v.extend_from_slice(&block_header(i + 1 == blocks.len(), *k, b.len()));
        v.extend_from_slice(b);
    }
    v
}

fn crc8(data: &[u8]) -> u8 {
    let mut c = 0u8;
    for &b in data {
        c ^= b;
        for _ in 0..8 {
            c = if c & 0x80 != 0 {
                (c << 1) ^ 0x07
            } else {
                c << 1
            };
        }
    }
    c
}

fn crc16(data: &[u8]) -> u16 {
    let mut c = 0u16;
    for &b in data {
        c ^= (b as u16) << 8;
        for _ in 0..8 {
            c = if c & 0x8000 != 0 {
                (c << 1) ^ 0x8005
            } else {
                c << 1
            };
        }
    }
    c
}

fn utf8_number(n: u32) -> Vec<u8> {
    if n < 0x80 {
        return vec![n as u8];
    }
    let mut bytes = 2;
    while n >= 1u32 << (5 * bytes + 1) {
        bytes += 1;
    }
    let mut out = vec![0u8; bytes as usize];
    let mut v = n;
    for i in (1..bytes as usize).rev() {
        out[i] = 0x80 | (v & 0x3F) as u8;
        v >>= 6;
    }
    out[0] = (0xFF00u16 >> bytes) as u8 | v as u8;
    out
}

/// A complete, decodable mono 16-bit FLAC of silence (constant subframes), `samples` long, with
/// the given Vorbis comments.
pub fn flac_silence(rate: u32, samples: u64, comments: &[(&str, &str)]) -> Vec<u8> {
    let mut v = b"fLaC".to_vec();
    let si = flac_streaminfo_block(rate, samples);
    v.extend_from_slice(&block_header(comments.is_empty(), 0, si.len()));
    v.extend_from_slice(&si);
    if !comments.is_empty() {
        let c = vorbis_comments(comments);
        v.extend_from_slice(&block_header(true, 4, c.len()));
        v.extend_from_slice(&c);
    }
    let mut left = samples;
    let mut frame_no = 0u32;
    while left > 0 {
        let n = left.min(4096);
        let mut f = vec![0xFF, 0xF8];
        let full = n == 4096;
        f.push(if full { 0xC0 } else { 0x70 }); // block size code, rate from STREAMINFO
        f.push(0x00); // mono, size from STREAMINFO
        f.extend_from_slice(&utf8_number(frame_no));
        if !full {
            f.extend_from_slice(&((n - 1) as u16).to_be_bytes());
        }
        f.push(crc8(&f));
        f.push(0x00); // constant subframe, no wasted bits
        f.extend_from_slice(&[0, 0]);
        let c = crc16(&f);
        f.extend_from_slice(&c.to_be_bytes());
        v.extend_from_slice(&f);
        left -= n;
        frame_no += 1;
    }
    v
}

fn ogg_page(header_type: u8, granule: u64, serial: u32, seq: u32, packets: &[&[u8]]) -> Vec<u8> {
    let mut segs: Vec<u8> = Vec::new();
    for p in packets {
        let mut n = p.len();
        while n >= 255 {
            segs.push(255);
            n -= 255;
        }
        segs.push(n as u8);
    }
    let mut v = b"OggS".to_vec();
    v.push(0);
    v.push(header_type);
    v.extend_from_slice(&granule.to_le_bytes());
    v.extend_from_slice(&serial.to_le_bytes());
    v.extend_from_slice(&seq.to_le_bytes());
    v.extend_from_slice(&[0; 4]);
    v.push(segs.len() as u8);
    v.extend_from_slice(&segs);
    for p in packets {
        v.extend_from_slice(p);
    }
    v
}

/// The skeleton of an Ogg Vorbis or Opus stream (identification, comments, one data page ending at
/// `end_granule`). Enough for tags and duration; not decodable.
pub fn ogg_stream(opus: bool, rate: u32, end_granule: u64, fields: &[(&str, &str)]) -> Vec<u8> {
    let comments = vorbis_comments(fields);
    let (ident, tags) = if opus {
        let mut i = b"OpusHead".to_vec();
        i.extend_from_slice(&[1, 1, 0x38, 0x01]);
        i.extend_from_slice(&rate.to_le_bytes());
        i.extend_from_slice(&[0, 0, 0]);
        let mut t = b"OpusTags".to_vec();
        t.extend_from_slice(&comments);
        (i, t)
    } else {
        let mut i = b"\x01vorbis".to_vec();
        i.extend_from_slice(&0u32.to_le_bytes());
        i.push(1);
        i.extend_from_slice(&rate.to_le_bytes());
        i.extend_from_slice(&[0; 14]);
        let mut t = b"\x03vorbis".to_vec();
        t.extend_from_slice(&comments);
        t.push(1);
        (i, t)
    };
    let mut v = ogg_page(2, 0, 7, 0, &[&ident]);
    v.extend_from_slice(&ogg_page(0, 0, 7, 1, &[&tags]));
    v.extend_from_slice(&ogg_page(4, end_granule, 7, 2, &[&[0u8; 10]]));
    v
}

/// ID3v2.3 frames: `(id, bytes)` with the text frames built by [`id3_text`].
pub fn id3_tag(frames: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = frames.concat();
    let n = body.len() as u32;
    let mut v = b"ID3\x03\0\0".to_vec();
    v.extend_from_slice(&[
        (n >> 21) as u8 & 0x7f,
        (n >> 14) as u8 & 0x7f,
        (n >> 7) as u8 & 0x7f,
        n as u8 & 0x7f,
    ]);
    v.extend_from_slice(&body);
    v
}

/// One UTF-8 text frame.
pub fn id3_text(id: &str, text: &str) -> Vec<u8> {
    let mut d = vec![3];
    d.extend_from_slice(text.as_bytes());
    id3_frame(id, &d)
}

pub fn id3_frame(id: &str, data: &[u8]) -> Vec<u8> {
    let mut v = id.as_bytes().to_vec();
    v.extend_from_slice(&(data.len() as u32).to_be_bytes());
    v.extend_from_slice(&[0, 0]);
    v.extend_from_slice(data);
    v
}

/// What [`mp4_file`] writes.
#[derive(Debug, Clone, Default)]
pub struct Mp4Spec {
    pub duration_ms: u64,
    /// Nero `chpl` chapters: (title, start ms).
    pub chapters: Vec<(String, u64)>,
    /// QuickTime chapter text track: (title, start ms).
    pub text_chapters: Vec<(String, u64)>,
    pub title: String,
    pub artist: String,
    pub narrator: String,
    pub series: String,
    pub cover: Option<Vec<u8>>,
    /// `M4A ` instead of `M4B `.
    pub m4a: bool,
}

fn bx(kind: &[u8], body: &[u8]) -> Vec<u8> {
    let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    v.extend_from_slice(kind);
    v.extend_from_slice(body);
    v
}

fn full(kind: &[u8], body: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 4];
    b.extend_from_slice(body);
    bx(kind, &b)
}

fn ilst_text(kind: &[u8], s: &str) -> Vec<u8> {
    let mut d = vec![0, 0, 0, 1, 0, 0, 0, 0];
    d.extend_from_slice(s.as_bytes());
    bx(kind, &bx(b"data", &d))
}

/// An MP4 container without audio samples that has the atoms Bookx reads (no decodable audio).
pub fn mp4_file(s: &Mp4Spec) -> Vec<u8> {
    let mut ftyp = if s.m4a {
        b"M4A ".to_vec()
    } else {
        b"M4B ".to_vec()
    };
    ftyp.extend_from_slice(&[0, 0, 2, 0]);
    ftyp.extend_from_slice(b"isom");
    let mut out = bx(b"ftyp", &ftyp);
    // mdat: the chapter text samples
    let mut mdat = Vec::new();
    let mut sample_offsets = Vec::new();
    let mdat_start = out.len() + 8;
    for (t, _) in &s.text_chapters {
        sample_offsets.push((mdat_start + mdat.len()) as u32);
        let mut smp = (t.len() as u16).to_be_bytes().to_vec();
        smp.extend_from_slice(t.as_bytes());
        mdat.extend_from_slice(&smp);
    }
    out.extend_from_slice(&bx(b"mdat", &mdat));

    let mut mvhd = [0u8; 100];
    mvhd[12..16].copy_from_slice(&1000u32.to_be_bytes());
    mvhd[16..20].copy_from_slice(&(s.duration_ms as u32).to_be_bytes());
    let mut moov = full(b"mvhd", &mvhd[4..]);

    if !s.text_chapters.is_empty() {
        let tkhd = |id: u32| {
            let mut b = [0u8; 84];
            b[12..16].copy_from_slice(&id.to_be_bytes());
            full(b"tkhd", &b[4..])
        };
        let mdhd = {
            let mut b = [0u8; 24];
            b[12..16].copy_from_slice(&1000u32.to_be_bytes());
            full(b"mdhd", &b[4..])
        };
        let hdlr = |h: &[u8]| {
            let mut b = vec![0u8; 4];
            b.extend_from_slice(h);
            b.extend_from_slice(&[0; 13]);
            full(b"hdlr", &b)
        };
        let audio_trak = bx(
            b"trak",
            &[
                tkhd(1),
                bx(b"tref", &bx(b"chap", &2u32.to_be_bytes())),
                bx(b"mdia", &[mdhd.clone(), hdlr(b"soun")].concat()),
            ]
            .concat(),
        );
        // time table: durations between the starts
        let mut stts = Vec::new();
        let n = s.text_chapters.len();
        stts.extend_from_slice(&(n as u32).to_be_bytes());
        for (i, (_, st)) in s.text_chapters.iter().enumerate() {
            let next = s
                .text_chapters
                .get(i + 1)
                .map(|c| c.1)
                .unwrap_or(s.duration_ms.max(*st));
            stts.extend_from_slice(&1u32.to_be_bytes());
            stts.extend_from_slice(&((next - st) as u32).to_be_bytes());
        }
        let mut stsz = vec![0, 0, 0, 0];
        stsz.extend_from_slice(&(n as u32).to_be_bytes());
        for (t, _) in &s.text_chapters {
            stsz.extend_from_slice(&((t.len() + 2) as u32).to_be_bytes());
        }
        // one chunk per sample
        let mut stsc = Vec::new();
        stsc.extend_from_slice(&1u32.to_be_bytes());
        stsc.extend_from_slice(&1u32.to_be_bytes());
        stsc.extend_from_slice(&1u32.to_be_bytes());
        stsc.extend_from_slice(&1u32.to_be_bytes());
        let mut stco = (n as u32).to_be_bytes().to_vec();
        for o in &sample_offsets {
            stco.extend_from_slice(&o.to_be_bytes());
        }
        let stbl = bx(
            b"stbl",
            &[
                full(b"stts", &stts),
                full(b"stsz", &stsz),
                full(b"stsc", &stsc),
                full(b"stco", &stco),
            ]
            .concat(),
        );
        let text_trak = bx(
            b"trak",
            &[
                tkhd(2),
                bx(b"mdia", &[mdhd, hdlr(b"text"), bx(b"minf", &stbl)].concat()),
            ]
            .concat(),
        );
        moov.extend_from_slice(&audio_trak);
        moov.extend_from_slice(&text_trak);
    }

    let mut udta = Vec::new();
    if !s.chapters.is_empty() {
        let mut c = vec![1, 0, 0, 0, 0, 0, 0, 0, s.chapters.len() as u8];
        for (t, ms) in &s.chapters {
            c.extend_from_slice(&(ms * 10_000).to_be_bytes());
            c.push(t.len().min(255) as u8);
            c.extend_from_slice(&t.as_bytes()[..t.len().min(255)]);
        }
        udta.extend_from_slice(&bx(b"chpl", &c));
    }
    let mut ilst = Vec::new();
    if !s.title.is_empty() {
        ilst.extend_from_slice(&ilst_text(b"\xa9nam", &s.title));
    }
    if !s.artist.is_empty() {
        ilst.extend_from_slice(&ilst_text(b"\xa9ART", &s.artist));
    }
    if !s.narrator.is_empty() {
        ilst.extend_from_slice(&ilst_text(b"\xa9nrt", &s.narrator));
    }
    if !s.series.is_empty() {
        ilst.extend_from_slice(&ilst_text(b"\xa9grp", &s.series));
    }
    if let Some(c) = &s.cover {
        let mut d = vec![0, 0, 0, 14, 0, 0, 0, 0];
        d.extend_from_slice(c);
        ilst.extend_from_slice(&bx(b"covr", &bx(b"data", &d)));
    }
    if !ilst.is_empty() {
        let mut meta = vec![0, 0, 0, 0];
        meta.extend_from_slice(&bx(b"ilst", &ilst));
        udta.extend_from_slice(&bx(b"meta", &meta));
    }
    if !udta.is_empty() {
        moov.extend_from_slice(&bx(b"udta", &udta));
    }
    out.extend_from_slice(&bx(b"moov", &moov));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_frame_numbers_match_char_encoding() {
        for n in [
            0u32, 1, 127, 128, 2047, 2048, 40000, 0xFFFF, 0x10000, 0x1FFFFF,
        ] {
            if let Some(c) = char::from_u32(n) {
                let mut b = [0u8; 4];
                assert_eq!(utf8_number(n), c.encode_utf8(&mut b).as_bytes(), "{n}");
            }
        }
    }

    #[test]
    fn crcs_match_known_values() {
        assert_eq!(crc8(b"123456789"), 0xF4);
        assert_eq!(crc16(b"123456789"), 0xFEE8);
    }
}
