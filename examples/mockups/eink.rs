//! Sheet 4: the e-ink look of the reader (Preferences → E-ink mode) and touch controls.

use crate::kit::*;
use crate::reader::{Layout, R, reader_window, selection_popover};

/// A header strip with three buttons of `size` pixels, to compare the normal and the large touch targets.
fn touch_strip(s: &mut Svg, x: f64, y: f64, w: f64, size: f64, t: &Theme, caption: &str) {
    let h = size + 20.0;
    s.rect(x, y, w, h, 12.0, t.header, Some(t.border));
    for i in 0..4 {
        let bx = x + 14.0 + i as f64 * (size + 14.0);
        s.rect(bx, y + 10.0, size, size, 8.0, t.hover, Some(t.border));
        let (cx, cy) = (bx + size / 2.0, y + 10.0 + size / 2.0);
        match i {
            0 => icon_chev(s, cx, cy, t.fg, 'l'),
            1 => icon_sidebar(s, cx, cy, t.fg, false),
            2 => icon_bookmark(s, cx, cy, t.fg, false),
            _ => icon_search(s, cx, cy, t.fg),
        }
    }
    let sx = x + 14.0 + 4.0 * (size + 14.0) + 20.0;
    slider(s, sx, y + h / 2.0, w - (sx - x) - 40.0, 0.4, t);
    // a bigger thumb for fingers
    if size > 36.0 {
        s.circle(
            sx + (w - (sx - x) - 40.0) * 0.4,
            y + h / 2.0,
            14.0,
            "#ffffff",
        );
        s.ring(
            sx + (w - (sx - x) - 40.0) * 0.4,
            y + h / 2.0,
            14.0,
            t.border,
            1.5,
        );
    }
    s.text(
        x + w / 2.0,
        y + h + 24.0,
        13.0,
        true,
        "#5e5c64",
        "middle",
        caption,
    );
}

pub fn run(s: &mut Svg) {
    let t = &EINK;
    s.text(90.0, 130.0, 74.0, true, "#2e3436", "start", "Bookx E-ink");
    s.lines(
        90.0,
        180.0,
        17.0,
        24.0,
        "#5e5c64",
        &[
            "E-ink mode (Preferences → Reader defaults → E-ink mode) is for slow black-and-white screens: pure black on white, no animations,",
            "bars that stay put, hard 2 px edges and wide tap zones. Large touch controls make every button at least 44 px.",
        ],
    );

    // ---------------------------------------------------------------- 4.a
    label(s, 90.0, 300.0, "4.a The reader in e-ink mode");
    let r = R {
        layout: Layout::Single,
        eink: true,
        ..Default::default()
    };
    reader_window(s, 90.0, 320.0, 760.0, 1000.0, t, &r);
    note(
        s,
        90.0,
        1356.0,
        &[
            "Black on white, hyphenated and justified. Highlights become grey or black",
            "lines (underline, strikethrough, squiggly); the bars never fade.",
        ],
    );

    // ---------------------------------------------------------------- 4.b
    label(s, 930.0, 300.0, "4.b Selecting text");
    let a = reader_window(
        s,
        930.0,
        320.0,
        760.0,
        1000.0,
        t,
        &R {
            layout: Layout::Single,
            eink: true,
            highlights: false,
            selection: true,
            ..Default::default()
        },
    );
    if let Some((px, py)) = a.selection {
        let h = 12.0 + 44.0 + 8.0 + 38.0 + 10.0;
        selection_popover(s, px - 220.0, py - h - 14.0, t, None, 0, None, true);
    }
    note(
        s,
        930.0,
        1356.0,
        &[
            "The selection is a grey band; the popover has hard black edges",
            "and grey swatches. Dictionary, Wikipedia and Translate are as usual.",
        ],
    );

    // ---------------------------------------------------------------- 4.c
    label(s, 1770.0, 300.0, "4.c Wide tap zones (the e-ink default)");
    reader_window(
        s,
        1770.0,
        320.0,
        760.0,
        1000.0,
        t,
        &R {
            layout: Layout::Single,
            eink: true,
            highlights: false,
            ..Default::default()
        },
    );
    let (zx, zy, zw, zh) = (1770.0, 366.0, 760.0, 1000.0 - 46.0 - 78.0);
    s.rect_a(zx, zy, zw / 3.0, zh, 0.0, "#000000", 0.08);
    s.rect_a(zx + zw / 3.0, zy, zw * 2.0 / 3.0, zh, 0.0, "#000000", 0.16);
    s.line(zx + zw / 3.0, zy, zx + zw / 3.0, zy + zh, "#000000", 2.0);
    for (cx, dir, name) in [
        (zx + zw / 6.0, 'l', "previous"),
        (zx + zw * 2.0 / 3.0, 'r', "next"),
    ] {
        let cy = zy + zh / 2.0;
        s.circle(cx, cy, 34.0, "#ffffff");
        s.ring(cx, cy, 34.0, "#000000", 2.0);
        icon_chev(s, cx, cy, "#000000", dir);
        s.rect(
            cx - 36.0,
            cy + 44.0,
            72.0,
            22.0,
            4.0,
            "#ffffff",
            Some("#000000"),
        );
        s.text(cx, cy + 60.0, 13.0, true, "#000000", "middle", name);
    }
    note(
        s,
        1770.0,
        1356.0,
        &[
            "Left third: previous page. Right two thirds: next page. No tap toggles",
            "the bars, so nothing flashes. Other choices: Edges, Thirds, mirrored Thirds, Off.",
        ],
    );

    // ---------------------------------------------------------------- what changes
    label(s, 2610.0, 300.0, "What E-ink mode changes");
    let items = [
        ("No animations", "sliding and fading are switched off"),
        ("Bars stay", "header and navigation row never fade"),
        ("Pure black on white", "text, links and accents are black"),
        (
            "Hyphenation + justify",
            "fewer ragged lines on a narrow page",
        ),
        ("Wide tap zones", "no bar toggle on tap"),
        (
            "One column",
            "unless two columns were chosen; scrolling becomes single column",
        ),
        (
            "Hard edges",
            "2 px header line, 1 px button borders, no shadows",
        ),
    ];
    for (i, (a, b)) in items.iter().enumerate() {
        let y = 330.0 + i as f64 * 76.0;
        s.rect(2610.0, y, 490.0, 62.0, 10.0, "#ffffff", Some("#000000"));
        s.text(2630.0, y + 26.0, 15.0, true, "#000000", "start", a);
        s.text(2630.0, y + 48.0, 12.0, false, "#333333", "start", b);
    }

    // ---------------------------------------------------------------- 4.d touch
    label(
        s,
        90.0,
        1500.0,
        "4.d Large touch controls — Preferences → Large touch controls (Automatic · Always on · Off)",
    );
    touch_strip(
        s,
        90.0,
        1530.0,
        700.0,
        34.0,
        &LIGHT,
        "Off — normal controls",
    );
    touch_strip(
        s,
        930.0,
        1530.0,
        700.0,
        44.0,
        &LIGHT,
        "On — every button and row at least 44 px, bigger slider thumb",
    );
    note(
        s,
        90.0,
        1680.0,
        &[
            "Automatic turns them on when the screen has a touch device. They combine with E-ink mode, but are independent of it.",
            "Highlight swatches grow from 26 to 38 px; list rows are at least 48 px tall.",
        ],
    );
    s.text(
        1600.0,
        1820.0,
        12.0,
        false,
        "#77767b",
        "middle",
        "Mockups are design illustrations. Sample text: Moby-Dick (public domain).",
    );
}
