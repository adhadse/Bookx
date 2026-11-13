//! E-ink mode (decision D8, after R13): what changes when reading on a slow, black-and-white
//! screen. No GTK in here, so it is unit tested; the app and the reader apply the effects.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::ReaderPrefs;
use crate::gestures::TapZones;
use crate::ui::canvas::Palette;

static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Called by the app whenever the setting changes.
pub fn set_active(on: bool) {
    ACTIVE.store(on, Ordering::SeqCst);
}

pub fn is_active() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}

/// What the mode changes, as plain values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Effects {
    /// GTK animations (sliding, fading) run.
    pub animations: bool,
    /// The header and bars fade away when idle (every fade is a flash on e-ink).
    pub chrome_fades: bool,
    /// Pure black text on pure white.
    pub high_contrast: bool,
}

pub fn effects(on: bool) -> Effects {
    Effects {
        animations: !on,
        chrome_fades: !on,
        high_contrast: on,
    }
}

/// Pure black on white, with links and the accent in black too (colours do not show anyway).
pub fn palette() -> Palette {
    Palette {
        bg: (1.0, 1.0, 1.0),
        fg: (0.0, 0.0, 0.0),
        link: (0.0, 0.0, 0.0),
        muted: (0.25, 0.25, 0.25),
        accent: (0.0, 0.0, 0.0),
    }
}

/// Reading preferences that suit the mode; applied once when it is switched on. Hyphenation
/// stays on (fewer ragged lines), the tap zones become wide strips (no bar toggle) and the layout
/// is a single column unless the user chose scrolling, which a page-refresh screen cannot do well.
pub fn tune_prefs(p: &mut ReaderPrefs) {
    p.tap_zones = TapZones::Wide;
    p.hyphenate = true;
    p.justify = true;
    if p.layout == crate::ui::canvas::LayoutMode::Scroll {
        p.layout = crate::ui::canvas::LayoutMode::SingleColumn;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::canvas::LayoutMode;

    #[test]
    fn the_switch_is_global_and_reversible() {
        // single test touching the global: no other test may depend on it
        set_active(true);
        assert!(is_active());
        set_active(false);
        assert!(!is_active());
    }

    #[test]
    fn effects_follow_the_mode() {
        let on = effects(true);
        assert!(!on.animations && !on.chrome_fades && on.high_contrast);
        let off = effects(false);
        assert!(off.animations && off.chrome_fades && !off.high_contrast);
    }

    #[test]
    fn the_palette_is_black_on_white() {
        let p = palette();
        assert_eq!(p.bg, (1.0, 1.0, 1.0));
        assert_eq!(p.fg, (0.0, 0.0, 0.0));
        // every colour is a grey: colours are meaningless on e-ink
        for c in [p.link, p.muted, p.accent, p.fg] {
            assert!(c.0 == c.1 && c.1 == c.2, "{c:?}");
        }
        // and the text is far darker than the muted text
        assert!(p.muted.0 > p.fg.0);
    }

    #[test]
    fn tuning_the_prefs() {
        let mut p = ReaderPrefs {
            tap_zones: TapZones::Edges,
            hyphenate: false,
            justify: false,
            layout: LayoutMode::Scroll,
            ..ReaderPrefs::default()
        };
        tune_prefs(&mut p);
        assert_eq!(p.tap_zones, TapZones::Wide);
        assert!(p.hyphenate && p.justify);
        assert_eq!(p.layout, LayoutMode::SingleColumn);
        // two columns are kept
        let mut q = ReaderPrefs {
            layout: LayoutMode::TwoColumns,
            ..ReaderPrefs::default()
        };
        tune_prefs(&mut q);
        assert_eq!(q.layout, LayoutMode::TwoColumns);
    }
}
