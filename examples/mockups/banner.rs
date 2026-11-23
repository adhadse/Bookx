//! The Continue Reading banner: three design variants for the states of the landing page and the
//! resize ladder of the chosen one (variant A). See docs/archive/ux-gaps-notes.md for the decision.

use crate::kit::Svg;

const PAPER: &str = "#fafafa";
const FG: &str = "#1d1d20";
const DIM: &str = "#5e5c64";
const COVER: &str = "#c9a47a";

fn cover(s: &mut Svg, x: f64, y: f64, h: f64) {
    let w = h * 2.0 / 3.0;
    s.rect(x, y, w, h, 4.0, COVER, None);
    s.rect(
        x + w * 0.12,
        y + h * 0.12,
        w * 0.76,
        h * 0.5,
        2.0,
        "#e9d3a8",
        None,
    );
    s.rect(
        x + w * 0.2,
        y + h * 0.72,
        w * 0.6,
        h * 0.05,
        2.0,
        "#6b4f2a",
        None,
    );
}

fn window(s: &mut Svg, x: f64, y: f64, w: f64, h: f64, caption: &str) {
    s.rect(x, y, w, h, 14.0, PAPER, Some("#d0d0d0"));
    s.text(x + 4.0, y - 12.0, 18.0, true, FG, "start", caption);
    // two covers of the grid under the banner
    let top = y + h * 0.58;
    for i in 0..4 {
        s.rect(
            x + 24.0 + i as f64 * 120.0,
            top,
            90.0,
            120.0,
            6.0,
            "#e6e0d6",
            None,
        );
    }
}

fn card(s: &mut Svg, x: f64, y: f64, w: f64, h: f64) {
    s.rect(x, y, w, h, 16.0, "#d9c3ab", None);
    s.rect_a(x, y, w, h, 16.0, "#ffffff", 0.18);
}

fn button(s: &mut Svg, x: f64, y: f64, w: f64, label: &str, primary: bool) {
    s.rect(
        x,
        y,
        w,
        30.0,
        15.0,
        if primary { FG } else { "#c5ad93" },
        None,
    );
    s.text(
        x + w / 2.0,
        y + 20.0,
        13.0,
        true,
        if primary { "#fff" } else { FG },
        "middle",
        label,
    );
}

/// Variant A (chosen): hero = [big cover | text and actions | also reading]; strip = one row.
fn hero_a(s: &mut Svg, x: f64, y: f64, w: f64, h: f64) {
    card(s, x, y, w, h);
    cover(s, x + 18.0, y + 14.0, h - 28.0);
    let tx = x + 18.0 + (h - 28.0) * 2.0 / 3.0 + 28.0;
    s.text(tx, y + 44.0, 12.0, true, DIM, "start", "CONTINUE READING");
    s.text(
        tx,
        y + 82.0,
        30.0,
        true,
        FG,
        "start",
        "Alice's Adventures in Wonderland",
    );
    s.text(tx, y + 110.0, 18.0, true, DIM, "start", "Lewis Carroll");
    s.text(
        tx,
        y + 136.0,
        13.0,
        false,
        DIM,
        "start",
        "Chapter 5 · about 2 h 30 min left · 40% completed",
    );
    s.rect(tx, y + 148.0, 300.0, 5.0, 2.5, "#00000033", None);
    s.rect(tx, y + 148.0, 120.0, 5.0, 2.5, FG, None);
    button(s, tx, y + 170.0, 100.0, "Continue", true);
    button(s, tx + 110.0, y + 170.0, 80.0, "Details", false);
    button(s, tx + 200.0, y + 170.0, 120.0, "Open from start", false);
    button(s, tx + 330.0, y + 170.0, 110.0, "Mark finished", false);
    // also reading
    s.text(
        x + w - 190.0,
        y + 60.0,
        11.0,
        true,
        DIM,
        "start",
        "ALSO READING",
    );
    for i in 0..3 {
        cover(s, x + w - 190.0 + i as f64 * 56.0, y + 72.0, 72.0);
    }
}

fn strip_a(s: &mut Svg, x: f64, y: f64, w: f64, narrow: bool) {
    let h = 64.0;
    card(s, x, y, w, h);
    cover(s, x + 12.0, y + 6.0, 52.0);
    if !narrow {
        s.text(
            x + 82.0,
            y + 26.0,
            15.0,
            true,
            FG,
            "start",
            "Alice's Adventures in Wonderland",
        );
        s.text(
            x + 82.0,
            y + 43.0,
            12.0,
            false,
            DIM,
            "start",
            "Lewis Carroll · 40%",
        );
        s.rect(x + 82.0, y + 51.0, w - 250.0, 4.0, 2.0, "#00000033", None);
        s.rect(x + 82.0, y + 51.0, (w - 250.0) * 0.4, 4.0, 2.0, FG, None);
        button(s, x + w - 112.0, y + 17.0, 100.0, "Continue", true);
    } else {
        s.text(
            x + 82.0,
            y + 38.0,
            14.0,
            true,
            FG,
            "start",
            "Alice's Adventures…",
        );
        s.circle(x + w - 34.0, y + 32.0, 17.0, FG);
        s.raw(&format!(
            "<path d=\"M{} {} l12 7 l-12 7 z\" fill=\"#fff\"/>",
            x + w - 39.0,
            y + 25.0
        ));
    }
}

/// Variant B: a left-aligned hero without a cover column; the cover leans on the right edge.
fn hero_b(s: &mut Svg, x: f64, y: f64, w: f64, h: f64) {
    card(s, x, y, w, h);
    s.text(
        x + 28.0,
        y + 50.0,
        12.0,
        true,
        DIM,
        "start",
        "CONTINUE READING",
    );
    s.text(
        x + 28.0,
        y + 96.0,
        34.0,
        true,
        FG,
        "start",
        "Alice's Adventures in Wonderland",
    );
    s.text(
        x + 28.0,
        y + 126.0,
        18.0,
        false,
        DIM,
        "start",
        "Lewis Carroll · 40% · about 2 h 30 min left",
    );
    button(s, x + 28.0, y + h - 56.0, 120.0, "Continue", true);
    cover(s, x + w - 150.0, y - 18.0, h + 10.0);
}

/// Variant C: a progress ring around the cover, one primary action, the others in a menu.
fn hero_c(s: &mut Svg, x: f64, y: f64, w: f64, h: f64) {
    card(s, x, y, w, h);
    s.ring(x + 90.0, y + h / 2.0, 62.0, "#00000022", 8.0);
    s.ring(x + 90.0, y + h / 2.0, 62.0, FG, 8.0);
    cover(s, x + 90.0 - 34.0, y + h / 2.0 - 50.0, 100.0);
    s.text(
        x + 190.0,
        y + 70.0,
        28.0,
        true,
        FG,
        "start",
        "Alice's Adventures in Wonderland",
    );
    s.text(
        x + 190.0,
        y + 98.0,
        16.0,
        false,
        DIM,
        "start",
        "Lewis Carroll · 40%",
    );
    button(s, x + 190.0, y + h - 60.0, 120.0, "Continue", true);
    button(s, x + 320.0, y + h - 60.0, 40.0, "…", false);
}

pub fn run(s: &mut Svg) {
    s.text(
        90.0,
        100.0,
        54.0,
        true,
        FG,
        "start",
        "Continue Reading banner",
    );
    s.text(90.0, 140.0, 18.0, false, DIM, "start", "Three variants for the cold-start hero, A is chosen: it carries every action and the other books being read. After the first visit Home shows the slim strip.");

    // ---- 1. cold start: the three hero variants
    s.text(
        90.0,
        210.0,
        26.0,
        true,
        FG,
        "start",
        "1  Cold start: the hero (viewport 1250 px high: 32 % of it)",
    );
    window(
        s,
        90.0,
        270.0,
        1400.0,
        520.0,
        "A  text | big cover | also reading (chosen)",
    );
    hero_a(s, 114.0, 294.0, 1352.0, 260.0);
    window(
        s,
        1560.0,
        270.0,
        1400.0,
        520.0,
        "B  text, cover leaning out",
    );
    hero_b(s, 1584.0, 294.0, 1352.0, 260.0);
    window(
        s,
        90.0,
        900.0,
        1400.0,
        520.0,
        "C  progress ring, one action and a menu",
    );
    hero_c(s, 114.0, 924.0, 1352.0, 260.0);

    // ---- 2. later visits: the strip
    s.text(
        90.0,
        1530.0,
        26.0,
        true,
        FG,
        "start",
        "2  Later visits to Home in the same session: the strip (64 px card, 88 px with margins)",
    );
    window(s, 90.0, 1590.0, 1400.0, 320.0, "A  strip");
    strip_a(s, 114.0, 1614.0, 1352.0, false);

    // ---- 3. the resize ladder of A
    s.text(90.0, 2010.0, 26.0, true, FG, "start", "3  Resize ladder of variant A: the layout follows the width of the book area, the height follows the viewport");
    let rungs: [(&str, f64, u8); 5] = [
        ("wide ≥ 900 px (+ also reading from 1150)", 1000.0, 0),
        ("medium 640-900 px: smaller cover, one More menu", 760.0, 1),
        (
            "strip 420-640 px, or a viewport under 928 px high",
            560.0,
            2,
        ),
        ("mini < 420 px: title and play button", 330.0, 3),
        (
            "scrolled: it shrinks to the strip, then slides off",
            760.0,
            4,
        ),
    ];
    let mut y = 2070.0;
    for (label, w, kind) in rungs {
        s.text(90.0, y, 16.0, true, DIM, "start", label);
        y += 14.0;
        match kind {
            0 => hero_a(s, 90.0, y, w.min(1100.0), 230.0),
            1 => {
                card(s, 90.0, y, w, 200.0);
                cover(s, 108.0, y + 14.0, 172.0);
                s.text(
                    250.0,
                    y + 60.0,
                    22.0,
                    true,
                    FG,
                    "start",
                    "Alice's Adventures in Wonderland",
                );
                s.text(
                    250.0,
                    y + 86.0,
                    14.0,
                    false,
                    DIM,
                    "start",
                    "Lewis Carroll · 40%",
                );
                button(s, 250.0, y + 130.0, 100.0, "Continue", true);
                button(s, 360.0, y + 130.0, 40.0, "…", false);
            }
            2 => strip_a(s, 90.0, y, w, false),
            3 => strip_a(s, 90.0, y, w, true),
            _ => {
                strip_a(s, 90.0, y, w, false);
                s.text(
                    w + 110.0,
                    y + 38.0,
                    14.0,
                    false,
                    DIM,
                    "start",
                    "← shrinks from the hero first (no jump of the books), then slides off",
                );
            }
        }
        y += match kind {
            0 => 280.0,
            1 => 250.0,
            _ => 110.0,
        };
    }
    s.text(90.0, y + 30.0, 14.0, false, DIM, "start", "Colours: the dominant colour of the cover, saturation capped, pastel in light mode and deep in dark mode; the text colour has at least 4.5:1 contrast on both ends of the gradient.");
}
