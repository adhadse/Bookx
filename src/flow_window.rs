//! Estimates for laying out a long chapter around a position (X18).
//!
//! Opening a book at 60 % of a 1,500 paragraph chapter must not lay out the 900 paragraphs before it
//! first. The canvas picks the paragraph where the target should be from these estimates, lays out a
//! window from there and shows it, and lays the whole chapter out in the background; when that is done
//! the exact numbers replace the estimates. Pure data in, pure data out (no GTK), unit tested.

/// What the estimator needs to know about one block of the chapter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockEst {
    /// Characters of text in the block (0 for a picture or an empty block).
    pub chars: usize,
    /// Font scale of the block (a heading is larger than the body text).
    pub scale: f64,
    /// Space above and below the block, in ems of the body text.
    pub gap_em: f64,
}

/// Measured on the real font and column.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// Characters per line of body text.
    pub chars_per_line: f64,
    /// Height of one line of body text, in pixels.
    pub line_h: f64,
    /// One em of body text, in pixels.
    pub em: f64,
    /// Height assumed for a picture.
    pub image_h: f64,
}

/// The height of a block in pixels.
pub fn block_height(b: &BlockEst, m: &Metrics) -> f64 {
    if b.chars == 0 {
        return m.image_h * 0.25 + b.gap_em * m.em;
    }
    let scale = b.scale.max(0.5);
    let per_line = (m.chars_per_line / scale).max(8.0);
    let lines = (b.chars as f64 / per_line).ceil().max(1.0);
    lines * m.line_h * scale + b.gap_em * m.em
}

/// `y` at the start of every block, and the total as the last entry (`len() == blocks.len() + 1`).
pub fn cumulative(blocks: &[BlockEst], m: &Metrics) -> Vec<f64> {
    let mut out = Vec::with_capacity(blocks.len() + 1);
    let mut y = 0.0;
    out.push(y);
    for b in blocks {
        y += block_height(b, m);
        out.push(y);
    }
    out
}

/// The block to start a window at so that `target_y` lies at least `lead` pixels into it: the last
/// block that starts at or before `target_y - lead`.
pub fn window_start(cum: &[f64], target_y: f64, lead: f64) -> usize {
    let n = cum.len().saturating_sub(1);
    let at = target_y - lead;
    cum[..n.max(1).min(cum.len())]
        .partition_point(|&y| y <= at)
        .saturating_sub(1)
        .min(n.saturating_sub(1))
}

/// Characters before each block (the offset a block's first character has in the chapter), for the
/// block lengths `chars` including the one separating character between blocks that have text.
pub fn chars_before(chars: &[usize]) -> Vec<usize> {
    let mut out = Vec::with_capacity(chars.len() + 1);
    let mut n = 0;
    out.push(n);
    for &c in chars {
        n += c + usize::from(c > 0);
        out.push(n);
    }
    out
}

/// Chapters with this many blocks are laid out in slices (`BOOKX_SLICE_FROM` lowers it for the tests, which
/// then run every reader scenario through the sliced layout).
pub const SLICE_FROM: usize = 120;

/// Chapters with this many blocks are shown from a window when the position is far in (`BOOKX_WINDOW_FROM`).
pub const WINDOW_FROM: usize = 240;

/// Whether a chapter is worth laying out in slices: short ones are done in one go, as always.
pub fn worth_slicing(blocks: usize, from: usize) -> bool {
    blocks >= from
}

/// Whether a saved position is far enough into the chapter to start from a window.
pub fn worth_window(frac: f64, blocks: usize, from: usize) -> bool {
    blocks >= from && frac > 0.03
}

/// `BOOKX_SLICE_FROM` / `BOOKX_WINDOW_FROM`, or `default`.
pub fn threshold(var: &str, default: usize) -> usize {
    std::env::var(var)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m() -> Metrics {
        Metrics {
            chars_per_line: 60.0,
            line_h: 24.0,
            em: 16.0,
            image_h: 300.0,
        }
    }

    fn para(chars: usize) -> BlockEst {
        BlockEst {
            chars,
            scale: 1.0,
            gap_em: 0.65,
        }
    }

    #[test]
    fn a_longer_block_is_taller() {
        let m = m();
        assert!(block_height(&para(600), &m) > block_height(&para(60), &m));
        assert_eq!(
            block_height(&para(60), &m),
            24.0 + 0.65 * 16.0,
            "60 characters are one line"
        );
        assert_eq!(block_height(&para(61), &m), 48.0 + 0.65 * 16.0);
    }

    #[test]
    fn a_heading_takes_more_room_than_a_paragraph_of_the_same_text() {
        let mut h = para(30);
        h.scale = 2.0;
        assert!(block_height(&h, &m()) > block_height(&para(30), &m()));
    }

    #[test]
    fn cumulative_heights_grow_and_end_with_the_total() {
        let blocks: Vec<_> = (1..=50).map(|i| para(i * 20)).collect();
        let cum = cumulative(&blocks, &m());
        assert_eq!(cum.len(), 51);
        assert_eq!(cum[0], 0.0);
        assert!(cum.windows(2).all(|w| w[1] > w[0]));
    }

    #[test]
    fn the_window_starts_a_lead_before_the_target() {
        let blocks: Vec<_> = (0..1000).map(|_| para(300)).collect();
        let cum = cumulative(&blocks, &m());
        let total = *cum.last().unwrap();
        let target = total * 0.6;
        let k = window_start(&cum, target, 800.0);
        assert!(cum[k] <= target - 800.0);
        assert!(cum[k + 1] > target - 800.0);
        assert_eq!(window_start(&cum, 0.0, 800.0), 0);
        assert_eq!(window_start(&cum, total * 2.0, 0.0), 999);
        assert_eq!(window_start(&[0.0], 10.0, 0.0), 0);
    }

    #[test]
    fn characters_before_a_block_count_a_separator_per_block_with_text() {
        assert_eq!(chars_before(&[10, 0, 5]), vec![0, 11, 11, 17]);
    }

    #[test]
    fn short_chapters_and_early_positions_are_laid_out_whole() {
        assert!(!worth_slicing(30, SLICE_FROM));
        assert!(worth_slicing(1500, SLICE_FROM));
        assert!(!worth_window(0.0, 1500, WINDOW_FROM));
        assert!(!worth_window(0.6, 100, WINDOW_FROM));
        assert!(worth_window(0.6, 1500, WINDOW_FROM));
        assert_eq!(threshold("BOOKX_NO_SUCH_THRESHOLD", 7), 7);
    }
}

#[cfg(test)]
mod perf {
    use super::*;
    use std::time::{Duration, Instant};

    fn budget(release_ms: u64) -> Duration {
        let debug = if cfg!(debug_assertions) { 30.0 } else { 1.0 };
        let scale: f64 = std::env::var("BOOKX_PERF_SCALE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0);
        Duration::from_secs_f64(release_ms as f64 / 1000.0 * debug * scale)
    }

    #[test]
    fn estimating_a_chapter_of_fifty_thousand_blocks_is_instant() {
        // the estimate runs on the main loop before the first screen of a chapter opened in the middle
        let m = Metrics {
            chars_per_line: 70.0,
            line_h: 24.0,
            em: 16.0,
            image_h: 300.0,
        };
        let blocks: Vec<BlockEst> = (0..50_000)
            .map(|i| BlockEst {
                chars: 40 + (i * 37) % 900,
                scale: if i % 97 == 0 { 1.6 } else { 1.0 },
                gap_em: 0.65,
            })
            .collect();
        let t = Instant::now();
        let cum = cumulative(&blocks, &m);
        let chars: Vec<usize> = blocks.iter().map(|b| b.chars).collect();
        let before = chars_before(&chars);
        let k = window_start(&cum, cum[cum.len() - 1] * 0.6, 800.0);
        let took = t.elapsed();
        assert!(k > 25_000 && k < 35_000 && before[k] > 0);
        assert!(took <= budget(10), "estimate took {took:?}");
    }
}
