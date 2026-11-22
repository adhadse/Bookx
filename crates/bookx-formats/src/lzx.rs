//! LZX frame decoding for CHM .
//!
//! The LZX bit stream itself is decoded by the `lzxd` crate; this module adds what CHM needs
//! around it: the window size taken from the `LZXC` control data, frames of 32 KiB with
//! byte-aligned compressed starts from the reset table, decoder resets at the reset interval,
//! and hard output limits.

use anyhow::{Context, Result, bail};
use lzxd::{Lzxd, WindowSize};

/// Uncompressed size of one LZX frame.
pub const FRAME: usize = 0x8000;

/// Maps a window size in bytes to the decoder's window enum (powers of two, 32 KiB to 32 MiB).
pub fn window_size(bytes: u32) -> Result<WindowSize> {
    Ok(match bytes {
        0x8000 => WindowSize::KB32,
        0x1_0000 => WindowSize::KB64,
        0x2_0000 => WindowSize::KB128,
        0x4_0000 => WindowSize::KB256,
        0x8_0000 => WindowSize::KB512,
        0x10_0000 => WindowSize::MB1,
        0x20_0000 => WindowSize::MB2,
        0x40_0000 => WindowSize::MB4,
        0x80_0000 => WindowSize::MB8,
        0x100_0000 => WindowSize::MB16,
        0x200_0000 => WindowSize::MB32,
        other => bail!("unsupported LZX window size {other}"),
    })
}

/// Decompresses `frames`, each given as its compressed bytes. All frames except the last
/// produce [`FRAME`] bytes; the last produces what remains of `total`. The decoder state is
/// reset before every frame whose index is a multiple of `reset_every` (0 means never).
pub fn decompress_frames(
    window: WindowSize,
    frames: &[&[u8]],
    total: usize,
    reset_every: usize,
    max_out: usize,
) -> Result<Vec<u8>> {
    if total > max_out {
        bail!("LZX content of {total} bytes exceeds the limit of {max_out}");
    }
    let expected_frames = total.div_ceil(FRAME);
    if frames.len() < expected_frames {
        bail!(
            "LZX data has {} frames but {expected_frames} are needed",
            frames.len()
        );
    }
    let mut dec = Lzxd::new(window);
    let mut out = Vec::with_capacity(total.min(max_out));
    for (i, frame) in frames.iter().take(expected_frames).enumerate() {
        if reset_every > 0 && i > 0 && i % reset_every == 0 {
            dec.reset();
        }
        let want = (total - out.len()).min(FRAME);
        let part = dec
            .decompress_next(frame, want)
            .map_err(|e| anyhow::anyhow!("LZX decoding failed in frame {i}: {e}"))
            .with_context(|| "corrupt LZX data")?;
        out.extend_from_slice(part);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fmt_writers::*;

    #[test]
    fn window_sizes_map_and_reject() {
        assert!(window_size(0x8000).is_ok());
        assert!(window_size(0x200_0000).is_ok());
        for bad in [0, 1, 0x9000, 0x400_0000] {
            let e = window_size(bad).unwrap_err();
            assert!(format!("{e}").contains("unsupported LZX window"), "{bad}");
        }
    }

    #[test]
    fn hand_assembled_uncompressed_block() {
        // Hand computed: E8 flag 0, block type 3 (uncompressed), size 3 -> words 0x3000 and
        // 0x0030 (little endian 16 bit words, bits read MSB first), then R0..R2 = 1,1,1 and
        // the raw bytes padded to an even length.
        let data = [
            0x00, 0x30, 0x30, 0x00, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, b'a', b'b', b'c', 0,
        ];
        let out = decompress_frames(WindowSize::KB32, &[&data], 3, 0, 100).unwrap();
        assert_eq!(out, b"abc");
        assert_eq!(lzx_uncompressed_stream(b"abc"), data, "test writer agrees");
    }

    #[test]
    fn hand_assembled_verbatim_block() {
        // Verbatim block (type 1) with a main tree where only 'a' and 'b' have length 1.
        let stream = lzx_verbatim_ab_stream(&[0, 1, 1, 0]);
        let out = decompress_frames(WindowSize::KB32, &[&stream], 4, 0, 100).unwrap();
        assert_eq!(out, b"abba");
    }

    #[test]
    fn two_frames_with_reset_and_without() {
        let a = vec![b'x'; FRAME];
        let f1 = lzx_uncompressed_stream(&a);
        let f2 = lzx_uncompressed_stream(b"tail");
        // Without a reset the second frame is a continuation: its own header says E8 flag
        // bit is read only once, so an independent stream needs a reset to decode.
        let out = decompress_frames(WindowSize::KB32, &[&f1, &f2], FRAME + 4, 1, 1 << 20).unwrap();
        assert_eq!(out.len(), FRAME + 4);
        assert_eq!(&out[FRAME..], b"tail");
        assert!(out[..FRAME].iter().all(|b| *b == b'x'));
    }

    #[test]
    fn too_few_frames_and_limits_are_errors() {
        let f = lzx_uncompressed_stream(b"abc");
        let e = decompress_frames(WindowSize::KB32, &[&f], FRAME + 1, 0, 1 << 20).unwrap_err();
        assert!(format!("{e}").contains("frames"));
        let e = decompress_frames(WindowSize::KB32, &[&f], 3, 0, 2).unwrap_err();
        assert!(format!("{e}").contains("exceeds the limit"));
    }

    #[test]
    fn corrupt_data_is_an_error_not_a_panic() {
        let junk = [0xFFu8; 40];
        assert!(decompress_frames(WindowSize::KB32, &[&junk], 100, 0, 1000).is_err());
        assert!(decompress_frames(WindowSize::KB32, &[&[]], 3, 0, 1000).is_err());
    }

    #[test]
    fn zero_length_output_needs_no_frames() {
        assert_eq!(
            decompress_frames(WindowSize::KB32, &[], 0, 0, 10).unwrap(),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn encoder_roundtrip_with_matches() {
        // Independent encoder (dev-dependency lzxc) produces real compressed frames.
        let text: Vec<u8> = b"the quick brown fox jumps over the lazy dog. "
            .iter()
            .copied()
            .cycle()
            .take(100_000)
            .collect();
        let frames = lzxc_encode(&text);
        assert!(frames.len() >= 4);
        let refs: Vec<&[u8]> = frames.iter().map(|f| f.as_slice()).collect();
        let out = decompress_frames(WindowSize::KB64, &refs, text.len(), 0, 1 << 20).unwrap();
        assert_eq!(out, text);
    }
}
