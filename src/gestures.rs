//! Touch and tap behaviour of the reading view (roadmap R13): which part of the page turns it,
//! and what counts as a swipe. No GTK in here, so it is unit tested.

use serde::{Deserialize, Serialize};

/// What a tap on the page does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapAction {
    NextPage,
    PrevPage,
    /// Neither: show or hide the bars.
    Toggle,
}

/// Where the tap zones are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TapZones {
    /// 18 % at each edge turns the page, the middle toggles the bars (the long-time default).
    #[default]
    Edges,
    /// Thirds: left = back, right = forward, middle toggles the bars (tablets).
    Thirds,
    /// Like thirds but mirrored, for reading with the left hand.
    LeftHanded,
    /// The whole right two thirds go forward, the left third back; no bar toggle (e-ink).
    Wide,
    /// Taps never turn the page; only swipes and keys do.
    Off,
}

impl TapZones {
    pub const ALL: [TapZones; 5] = [
        TapZones::Edges,
        TapZones::Thirds,
        TapZones::LeftHanded,
        TapZones::Wide,
        TapZones::Off,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TapZones::Edges => "Edges (narrow strips)",
            TapZones::Thirds => "Thirds (tablet)",
            TapZones::LeftHanded => "Thirds, mirrored (left hand)",
            TapZones::Wide => "Wide (no bar toggle)",
            TapZones::Off => "Off (swipe or keys only)",
        }
    }

    /// The action for a tap at horizontal position `x` (0..1 across the page).
    pub fn action_at(self, x: f64) -> TapAction {
        let x = if x.is_finite() {
            x.clamp(0.0, 1.0)
        } else {
            0.5
        };
        match self {
            TapZones::Edges => {
                if x < 0.18 {
                    TapAction::PrevPage
                } else if x > 0.82 {
                    TapAction::NextPage
                } else {
                    TapAction::Toggle
                }
            }
            TapZones::Thirds => {
                if x < 1.0 / 3.0 {
                    TapAction::PrevPage
                } else if x > 2.0 / 3.0 {
                    TapAction::NextPage
                } else {
                    TapAction::Toggle
                }
            }
            TapZones::LeftHanded => {
                if x < 1.0 / 3.0 {
                    TapAction::NextPage
                } else if x > 2.0 / 3.0 {
                    TapAction::PrevPage
                } else {
                    TapAction::Toggle
                }
            }
            TapZones::Wide => {
                if x < 1.0 / 3.0 {
                    TapAction::PrevPage
                } else {
                    TapAction::NextPage
                }
            }
            TapZones::Off => TapAction::Toggle,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Swipe {
    /// Finger moved left: the next page.
    Left,
    /// Finger moved right: the previous page.
    Right,
}

/// Minimum horizontal travel of a swipe, in pixels.
pub const SWIPE_MIN_DISTANCE: f64 = 60.0;
/// A swipe must be this much more horizontal than vertical.
pub const SWIPE_DIRECTION_RATIO: f64 = 2.0;
/// A slower movement is a drag, not a swipe.
pub const SWIPE_MAX_MILLIS: u64 = 700;

/// Classifies a finger movement of `(dx, dy)` pixels that took `millis`.
pub fn classify_swipe(dx: f64, dy: f64, millis: u64) -> Option<Swipe> {
    if !dx.is_finite() || !dy.is_finite() || millis > SWIPE_MAX_MILLIS {
        return None;
    }
    if dx.abs() < SWIPE_MIN_DISTANCE || dx.abs() < dy.abs() * SWIPE_DIRECTION_RATIO {
        return None;
    }
    Some(if dx < 0.0 { Swipe::Left } else { Swipe::Right })
}

/// Whether a drag that started with a finger may select text. Selecting by dragging would make
/// swiping impossible, so on touch screens the selection starts with a long press instead.
pub fn drag_selects(is_touch: bool, long_pressed: bool) -> bool {
    !is_touch || long_pressed
}

/// Whether the key/swipe/tap that points to the right (`toward_right`) goes to the next page. In a
/// book that turns pages right to left (the spine's `page-progression-direction="rtl"`) it goes back.
pub fn right_is_forward(rtl: bool) -> bool {
    !rtl
}

/// A tap zone action in a right-to-left book: the left side goes forward.
pub fn mirror_tap(a: TapAction, rtl: bool) -> TapAction {
    match (rtl, a) {
        (true, TapAction::PrevPage) => TapAction::NextPage,
        (true, TapAction::NextPage) => TapAction::PrevPage,
        _ => a,
    }
}

/// Larger controls for fingers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TouchUi {
    /// On when the device has a touch screen.
    #[default]
    Auto,
    On,
    Off,
}

impl TouchUi {
    pub const ALL: [TouchUi; 3] = [TouchUi::Auto, TouchUi::On, TouchUi::Off];

    pub fn label(self) -> &'static str {
        match self {
            TouchUi::Auto => "Automatic (touch screens)",
            TouchUi::On => "Always on",
            TouchUi::Off => "Off",
        }
    }

    pub fn enabled(self, has_touch_screen: bool) -> bool {
        match self {
            TouchUi::Auto => has_touch_screen,
            TouchUi::On => true,
            TouchUi::Off => false,
        }
    }
}

#[cfg(test)]
mod rtl_tests {
    use super::*;

    #[test]
    fn right_to_left_books_turn_the_other_way() {
        assert!(right_is_forward(false));
        assert!(!right_is_forward(true));
        assert_eq!(mirror_tap(TapAction::NextPage, true), TapAction::PrevPage);
        assert_eq!(mirror_tap(TapAction::PrevPage, true), TapAction::NextPage);
        assert_eq!(mirror_tap(TapAction::Toggle, true), TapAction::Toggle);
        assert_eq!(mirror_tap(TapAction::NextPage, false), TapAction::NextPage);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TapAction::*;

    fn row(z: TapZones) -> Vec<TapAction> {
        [
            0.0, 0.1, 0.17, 0.19, 0.3, 0.34, 0.5, 0.66, 0.7, 0.81, 0.83, 0.9, 1.0,
        ]
        .iter()
        .map(|x| z.action_at(*x))
        .collect()
    }

    #[test]
    fn edge_zones() {
        let r = row(TapZones::Edges);
        assert_eq!(
            r,
            vec![
                PrevPage, PrevPage, PrevPage, Toggle, Toggle, Toggle, Toggle, Toggle, Toggle,
                Toggle, NextPage, NextPage, NextPage
            ]
        );
    }

    #[test]
    fn third_zones() {
        assert_eq!(
            row(TapZones::Thirds),
            vec![
                PrevPage, PrevPage, PrevPage, PrevPage, PrevPage, Toggle, Toggle, Toggle, NextPage,
                NextPage, NextPage, NextPage, NextPage
            ]
        );
    }

    #[test]
    fn left_handed_mirrors_the_thirds() {
        let a = row(TapZones::Thirds);
        let b = row(TapZones::LeftHanded);
        for (x, y) in a.iter().zip(&b) {
            let mirrored = match x {
                PrevPage => NextPage,
                NextPage => PrevPage,
                Toggle => Toggle,
            };
            assert_eq!(*y, mirrored);
        }
    }

    #[test]
    fn wide_zones_never_toggle_and_off_always_does() {
        assert!(row(TapZones::Wide).iter().all(|a| *a != Toggle));
        assert_eq!(TapZones::Wide.action_at(0.2), PrevPage);
        assert_eq!(TapZones::Wide.action_at(0.5), NextPage);
        assert!(row(TapZones::Off).iter().all(|a| *a == Toggle));
    }

    #[test]
    fn nonsense_positions_do_not_panic() {
        for z in TapZones::ALL {
            for x in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -5.0, 7.0] {
                let _ = z.action_at(x);
            }
        }
        assert_eq!(TapZones::Edges.action_at(-1.0), PrevPage);
        assert_eq!(TapZones::Edges.action_at(2.0), NextPage);
        assert_eq!(TapZones::Edges.action_at(f64::NAN), Toggle);
    }

    #[test]
    fn swipes() {
        assert_eq!(classify_swipe(-120.0, 10.0, 200), Some(Swipe::Left));
        assert_eq!(classify_swipe(120.0, -10.0, 200), Some(Swipe::Right));
        // too short
        assert_eq!(classify_swipe(-59.0, 0.0, 100), None);
        assert_eq!(classify_swipe(-60.0, 0.0, 100), Some(Swipe::Left));
        // too vertical
        assert_eq!(classify_swipe(-100.0, 80.0, 100), None);
        assert_eq!(classify_swipe(-100.0, 50.0, 100), Some(Swipe::Left));
        assert_eq!(classify_swipe(0.0, 300.0, 100), None);
        // too slow
        assert_eq!(classify_swipe(-200.0, 0.0, 701), None);
        assert_eq!(classify_swipe(-200.0, 0.0, 700), Some(Swipe::Left));
        // not a number
        assert_eq!(classify_swipe(f64::NAN, 0.0, 100), None);
        assert_eq!(classify_swipe(-100.0, f64::INFINITY, 100), None);
    }

    #[test]
    fn dragging_selects_with_a_mouse_but_needs_a_long_press_on_touch() {
        assert!(drag_selects(false, false));
        assert!(!drag_selects(true, false));
        assert!(drag_selects(true, true));
        assert!(drag_selects(false, true));
    }

    #[test]
    fn touch_ui_modes() {
        assert!(TouchUi::Auto.enabled(true));
        assert!(!TouchUi::Auto.enabled(false));
        assert!(TouchUi::On.enabled(false));
        assert!(!TouchUi::Off.enabled(true));
        assert_eq!(TouchUi::default(), TouchUi::Auto);
        assert_eq!(TapZones::default(), TapZones::Edges);
    }

    #[test]
    fn settings_round_trip_through_json() {
        for z in TapZones::ALL {
            let s = serde_json::to_string(&z).unwrap();
            assert_eq!(serde_json::from_str::<TapZones>(&s).unwrap(), z);
        }
        for t in TouchUi::ALL {
            let s = serde_json::to_string(&t).unwrap();
            assert_eq!(serde_json::from_str::<TouchUi>(&s).unwrap(), t);
        }
        assert_eq!(
            serde_json::to_string(&TapZones::LeftHanded).unwrap(),
            "\"left_handed\""
        );
    }

    #[test]
    fn every_option_has_a_label() {
        assert!(TapZones::ALL.iter().all(|z| !z.label().is_empty()));
        assert!(TouchUi::ALL.iter().all(|t| !t.label().is_empty()));
    }
}
