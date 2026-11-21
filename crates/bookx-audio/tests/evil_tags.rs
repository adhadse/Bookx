//! Hostile tag and chapter data: nothing may panic, hang or allocate what the file claims.

use std::io::Cursor;
use std::time::{Duration, Instant};

use bookx_audio::{id3, mp4, vorbis};

/// Deterministic noise (xorshift), so a failure is reproducible.
fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

fn within(limit: Duration, f: impl FnOnce()) {
    let t = Instant::now();
    f();
    assert!(t.elapsed() < limit, "took {:?}", t.elapsed());
}

#[test]
fn random_bytes_never_panic_any_parser() {
    for seed in 1..60u64 {
        let b = noise(seed, 1 + (seed as usize * 37) % 5000);
        let _ = id3::tag_size(&b);
        let _ = id3::parse(&b);
        let _ = mp4::parse_chpl(&b);
        let _ = mp4::read_info(&mut Cursor::new(b.clone()));
        let _ = vorbis::parse_comments(&b);
        let _ = vorbis::parse_picture(&b);
        let _ = vorbis::parse_flac(&b);
        let _ = vorbis::flac_streaminfo(&b);
    }
}

#[test]
fn an_id3_header_that_claims_a_gigabyte_is_not_allocated() {
    // "ID3", v2.4, no flags, synchsafe size 0x7f7f7f7f (about 256 MiB) but only a few bytes follow
    let mut b = b"ID3\x04\x00\x00\x7f\x7f\x7f\x7f".to_vec();
    b.extend_from_slice(b"TIT2\x7f\x7f\x7f\x7f\x00\x00\x03abc");
    within(Duration::from_secs(2), || {
        let t = id3::parse(&b);
        let _ = t;
    });
}

#[test]
fn id3_frames_with_sizes_past_the_end_or_zero_do_not_loop() {
    let mut b = b"ID3\x04\x00\x00\x00\x00\x01\x00".to_vec();
    // 128 frames of size zero: a loop that does not advance would hang
    for _ in 0..128 {
        b.extend_from_slice(b"TIT2\x00\x00\x00\x00\x00\x00");
    }
    within(Duration::from_secs(2), || {
        let _ = id3::parse(&b);
    });
}

#[test]
fn a_chapter_list_that_claims_millions_of_entries_stops_at_the_data() {
    // chpl: version, flags, reserved, count = u32::MAX, then nothing
    let mut b = vec![1, 0, 0, 0, 0, 0, 0, 0];
    b.extend_from_slice(&u32::MAX.to_be_bytes());
    within(Duration::from_secs(2), || {
        assert!(mp4::parse_chpl(&b).len() < 10);
    });
}

#[test]
fn vorbis_comments_with_huge_counts_and_lengths_are_cut_short() {
    // vendor length u32::MAX
    let mut b = u32::MAX.to_le_bytes().to_vec();
    b.extend_from_slice(b"short");
    within(Duration::from_secs(2), || {
        let _ = vorbis::parse_comments(&b);
    });
    // vendor empty, 2^32-1 comments, each claiming 2^32-1 bytes
    let mut b = 0u32.to_le_bytes().to_vec();
    b.extend_from_slice(&u32::MAX.to_le_bytes());
    b.extend_from_slice(&u32::MAX.to_le_bytes());
    within(Duration::from_secs(2), || {
        let _ = vorbis::parse_comments(&b);
    });
}

#[test]
fn a_flac_picture_with_absurd_lengths_is_refused() {
    let mut b = Vec::new();
    for v in [3u32, 4, 0xffff_fff0, 0, 0, 0, 0, 0xffff_fff0] {
        b.extend_from_slice(&v.to_be_bytes());
    }
    within(Duration::from_secs(2), || {
        assert!(vorbis::parse_picture(&b).is_none());
    });
}

#[test]
fn an_mp4_with_a_box_that_nests_into_itself_ends() {
    // a `moov` box whose size is 8: its own header only; repeated
    let mut b = Vec::new();
    for _ in 0..1000 {
        b.extend_from_slice(&8u32.to_be_bytes());
        b.extend_from_slice(b"moov");
    }
    within(Duration::from_secs(2), || {
        let _ = mp4::read_info(&mut Cursor::new(b.clone()));
    });
    // a box with size 0 ("to the end") inside itself, and one with a 64-bit size of zero
    let mut c = Vec::new();
    c.extend_from_slice(&1u32.to_be_bytes());
    c.extend_from_slice(b"moov");
    c.extend_from_slice(&0u64.to_be_bytes());
    within(Duration::from_secs(2), || {
        let _ = mp4::read_info(&mut Cursor::new(c.clone()));
    });
}
