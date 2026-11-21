//! Hostile page images for the comic reader: bombs declared in the header, truncated and corrupt
//! files, wrong magic. The decoder answers with an error before it allocates, in bounded time.
//! Fuzz twin: `fuzz/fuzz_targets/comic.rs`.

use std::time::{Duration, Instant};

use bookx_comic::decode::{decode_page, dimensions, is_bomb, Opts};

fn png_header(w: u32, h: u32) -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    v.extend(w.to_be_bytes());
    v.extend(h.to_be_bytes());
    v.extend(b"\x08\x06\0\0\0\0\0\0\0");
    v
}

fn gif_header(w: u16, h: u16) -> Vec<u8> {
    let mut v = b"GIF89a".to_vec();
    v.extend(w.to_le_bytes());
    v.extend(h.to_le_bytes());
    v.extend([0x00, 0, 0, b';']);
    v
}

fn rng(seed: u64, n: usize) -> Vec<u8> {
    let mut x = seed | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

#[test]
fn declared_giant_images_are_refused_before_allocation() {
    for (w, h) in [
        (60_000, 60_000),
        (u32::MAX, 1),
        (1, u32::MAX),
        (40_001, 1),
        (30_000, 30_000),
        (0, 0),
    ] {
        let p = png_header(w, h);
        let t = Instant::now();
        let r = decode_page(&p, &Opts::default());
        assert!(r.is_err(), "{w}x{h}");
        assert!(
            t.elapsed() < Duration::from_secs(5),
            "{w}x{h}: {:?}",
            t.elapsed()
        );
        if w > 40_000 || h > 40_000 || (w as u64) * (h as u64) > 160_000_000 {
            assert!(
                is_bomb(&p) || dimensions(&p).is_err(),
                "{w}x{h} counts as a bomb"
            );
        }
    }
    let g = gif_header(65_535, 65_535);
    assert!(decode_page(&g, &Opts::default()).is_err());
}

#[test]
fn corrupt_images_are_errors_in_bounded_time() {
    let mut cases: Vec<Vec<u8>> = vec![
        vec![],
        vec![0xff, 0xd8],
        vec![0xff, 0xd8, 0xff, 0xe0, 0xff, 0xff],
        b"RIFF\xff\xff\xff\xffWEBPVP8 ".to_vec(),
        b"BM\xff\xff\xff\xff".to_vec(),
        b"II*\0\xff\xff\xff\xff".to_vec(),
        png_header(100, 100),
        gif_header(10, 10),
    ];
    for seed in 1..40 {
        let mut p = png_header(64, 64);
        p.extend(rng(seed, 4096));
        cases.push(p);
        let mut j = vec![0xff, 0xd8, 0xff, 0xe0];
        j.extend(rng(seed + 100, 4096));
        cases.push(j);
        cases.push(rng(seed + 200, 8192));
    }
    for (i, c) in cases.iter().enumerate() {
        let t = Instant::now();
        let _ = decode_page(c, &Opts::default());
        let _ = dimensions(c);
        let _ = is_bomb(c);
        assert!(
            t.elapsed() < Duration::from_secs(10),
            "case {i}: {:?}",
            t.elapsed()
        );
    }
}
