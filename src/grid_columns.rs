//! How many columns the cover grid may have (pure, no GTK).
//!
//! `GtkGridView` keeps `max_columns * 30 + 2 * max_columns + 1` tiles alive around its anchor, whatever the
//! window shows: at the old cap of 40 that is 1281 bound tiles for a screen that shows twenty, and building or
//! clearing them took most of a second at 2000 books. The cap now follows the width of the grid and the cover
//! size, with room to grow so that a resize does not change it every frame.

/// The cap before the grid has a width (about one 1080p screen of average covers).
pub const START_MAX: u32 = 12;
/// Most columns ever (the old fixed cap).
pub const LIMIT: u32 = 40;
/// Fewest.
pub const FLOOR: u32 = 4;
/// Columns kept in reserve above what fits now.
pub const SPARE: u32 = 3;
/// What a tile adds around its cover (the margins of the tile).
pub const TILE_EXTRA: i32 = 20;

/// The most columns a grid of `width` px can lay out with covers of `cover` px.
pub fn fits(width: i32, cover: i32) -> u32 {
    let tile = (cover + TILE_EXTRA).max(40);
    (width.max(0) / tile) as u32
}

/// The cap to use now, given the cap in force: it only moves when the grid needs more columns than it has
/// room for, or is more than twice the spare too generous, so a window resize is not a stream of changes.
pub fn max_for(width: i32, cover: i32, current: u32) -> u32 {
    if width <= 0 {
        return current;
    }
    let need = fits(width, cover);
    let target = (need + SPARE).clamp(FLOOR, LIMIT);
    if need + 1 > current || current > target + 2 * SPARE {
        target
    } else {
        current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normal_window_gets_a_small_cap() {
        // 1000 px of 150 px covers: 5 columns fit, the cap is 8 (257 tiles instead of 1281)
        assert_eq!(fits(1000, 150), 5);
        assert_eq!(max_for(1000, 150, START_MAX), START_MAX); // 12 is within the spare
        assert_eq!(max_for(1000, 150, 40), 8);
    }

    #[test]
    fn a_wide_window_or_small_covers_raise_the_cap() {
        assert_eq!(max_for(3840, 150, 8), 25);
        assert_eq!(max_for(1920, 80, 8), 22);
        // never above the old cap
        assert_eq!(max_for(30_000, 60, 8), LIMIT);
    }

    #[test]
    fn the_cap_never_leaves_fewer_columns_than_fit() {
        for width in (300..5000).step_by(37) {
            for cover in [60, 100, 150, 220, 320] {
                let mut cap = START_MAX;
                cap = max_for(width, cover, cap);
                assert!(
                    cap >= fits(width, cover).min(LIMIT),
                    "{width} {cover} {cap}"
                );
                assert!(cap >= FLOOR);
            }
        }
    }

    #[test]
    fn a_resize_does_not_change_the_cap_every_frame() {
        let mut cap = max_for(1000, 150, START_MAX);
        let mut changes = 0;
        for w in 900..1100 {
            let n = max_for(w, 150, cap);
            if n != cap {
                changes += 1;
                cap = n;
            }
        }
        assert!(changes <= 1, "{changes}");
    }

    #[test]
    fn no_width_yet_keeps_the_cap() {
        assert_eq!(max_for(0, 150, 12), 12);
        assert_eq!(max_for(-5, 150, 7), 7);
    }

    #[test]
    fn a_much_too_large_cap_comes_down() {
        assert_eq!(max_for(700, 150, 40), 7);
    }

    #[test]
    fn the_tiles_kept_alive_drop_with_the_cap() {
        let alive = |cols: u32| cols * 30 + 2 * cols + 1;
        assert_eq!(alive(40), 1281);
        assert!(alive(max_for(1000, 150, 40)) < 300);
    }
}
