use crate::kit::*;

#[derive(Clone, Copy, PartialEq)]
pub enum Layout {
    Two,
    Single,
    Scroll,
}

/// How a highlight is drawn (`HighlightStyle` in the app).
#[derive(Clone, Copy, PartialEq)]
pub enum Hl {
    Background,
    Underline,
    Strike,
    Squiggly,
    Selection,
}

/// A highlight on the sample page: the first line that contains `find` gets it.
pub struct Mark {
    pub find: &'static str,
    pub style: Hl,
    /// Index into `HL` (yellow, purple, blue, red, green).
    pub color: usize,
    /// A highlight with a note keeps a solid 2 px line under it.
    pub note: bool,
}

pub const MARKS: [Mark; 5] = [
    Mark {
        find: "tery part of the world",
        style: Hl::Background,
        color: 0,
        note: false,
    },
    Mark {
        find: "It is a way I have",
        style: Hl::Underline,
        color: 2,
        note: false,
    },
    Mark {
        find: "damp, drizzly November",
        style: Hl::Background,
        color: 1,
        note: true,
    },
    Mark {
        find: "involuntarily",
        style: Hl::Squiggly,
        color: 4,
        note: false,
    },
    Mark {
        find: "hats off",
        style: Hl::Strike,
        color: 3,
        note: false,
    },
];

/// The text the user has selected on the sample page.
pub const SELECTED: &str = "spleen and regulating";

/// The sample page: Moby-Dick (public domain), wrapped by hand so that lines end in real
/// hyphenation points (wa-tery, driv-ing, step-ping, peo-ple's). Lines that end a paragraph
/// are not justified.
const PARAS: [&[&str]; 3] = [
    &[
        "Call me Ishmael. Some years ago—never mind how long",
        "precisely—having little or no money in my purse, and",
        "nothing particular to interest me on shore, I",
        "thought I would sail about a little and see the wa-",
        "tery part of the world. It is a way I have of driv-",
        "ing off the spleen and regulating the circulation.",
    ],
    &[
        "Whenever I find myself growing grim about the mouth;",
        "whenever it is a damp, drizzly November in my soul;",
        "whenever I find myself involuntarily pausing before",
        "coffin warehouses, and bringing up the rear of every",
        "funeral I meet; and especially whenever my hypos get",
        "such an upper hand of me, that it requires a strong",
        "moral principle to prevent me from deliberately step-",
        "ping into the street, and methodically knocking peo-",
        "ple's hats off—then, I account it high time to get to",
        "sea as soon as I can.",
    ],
    &[
        "This is my substitute for pistol and ball. With a",
        "philosophical flourish Cato throws himself upon his",
        "sword; I quietly take to the ship. There is nothing",
        "surprising in this. If they but knew it, almost all",
        "men in their degree, some time or other, cherish very",
        "nearly the same feelings towards the ocean with me.",
    ],
];

pub struct R {
    pub layout: Layout,
    pub sidebar: Option<&'static str>,
    pub chrome: bool,
    /// Draw the sample highlights (`MARKS`).
    pub highlights: bool,
    /// Draw a text selection on `SELECTED`.
    pub selection: bool,
    /// E-ink mode: no colour, hard black edges, bars that stay.
    pub eink: bool,
    /// Colour of the chapter-lead words, as a book's own CSS would set them (Original mode).
    pub lead: Option<&'static str>,
}

impl Default for R {
    fn default() -> Self {
        R {
            layout: Layout::Two,
            sidebar: None,
            chrome: true,
            highlights: true,
            selection: false,
            eink: false,
            lead: None,
        }
    }
}

/// Where interesting things ended up on the page, so sheets can point at them.
#[derive(Default, Clone, Copy)]
pub struct Anchors {
    /// Top centre of the selection.
    pub selection: Option<(f64, f64)>,
    /// End of the first line that is hyphenated.
    pub hyphen: Option<(f64, f64)>,
    /// Top centre of the first highlight that has a note.
    pub note: Option<(f64, f64)>,
}

/// Draws a highlight of `style` over x..x+w on the line whose baseline is `base`.
pub fn draw_mark(
    s: &mut Svg,
    x: f64,
    base: f64,
    w: f64,
    size: f64,
    m: &Mark,
    t: &Theme,
    eink: bool,
) {
    let dark = t.bg == DARK.bg;
    let col = if eink { "#000000" } else { HL[m.color] };
    let top = base - size * 1.05;
    let h = size * 1.4;
    match m.style {
        Hl::Background => {
            if eink {
                s.rect_a(x - 1.0, top, w + 2.0, h, 2.0, "#000000", 0.18);
            } else {
                s.rect_a(
                    x - 1.0,
                    top,
                    w + 2.0,
                    h,
                    2.0,
                    col,
                    if dark { 0.42 } else { 0.6 },
                );
            }
        }
        Hl::Selection => s.rect_a(
            x - 1.0,
            top,
            w + 2.0,
            h,
            2.0,
            t.accent,
            if dark { 0.45 } else { 0.32 },
        ),
        _ => {}
    }
    if m.note {
        s.rect(x - 1.0, top + h - 2.0, w + 2.0, 2.0, 0.0, col, None);
    }
}

/// The lines drawn over the glyphs: underline, strikethrough and squiggly.
pub fn draw_mark_line(s: &mut Svg, x: f64, base: f64, w: f64, size: f64, m: &Mark, eink: bool) {
    let col = if eink { "#000000" } else { HL[m.color] };
    match m.style {
        Hl::Underline => s.rect(x, base + size * 0.2, w, 2.0, 0.0, col, None),
        Hl::Strike => s.rect(x, base - size * 0.32, w, 2.0, 0.0, col, None),
        Hl::Squiggly => {
            let y0 = base + size * 0.28;
            let mut d = format!("M{x},{y0}");
            let mut xx = x;
            let mut up = true;
            while xx < x + w {
                xx = (xx + 3.0).min(x + w);
                d.push_str(&format!(" L{xx},{}", if up { y0 - 2.0 } else { y0 + 1.5 }));
                up = !up;
            }
            s.path(&d, "none", col, 2.0);
        }
        _ => {}
    }
}

/// Rough advance width of a serif glyph in em, to place highlights inside justified lines.
fn glyph_em(c: char) -> f64 {
    match c {
        'i' | 'l' | 'j' | '.' | ',' | ';' | ':' | '\'' | '’' | '!' => 0.27,
        'f' | 't' | 'r' | '-' | ' ' | '(' | ')' => 0.36,
        'm' | 'w' | 'M' | 'W' | '—' => 0.86,
        c if c.is_uppercase() => 0.68,
        _ => 0.54,
    }
}

fn text_em(t: &str) -> f64 {
    t.chars().map(glyph_em).sum()
}

enum Item {
    Head,
    Line(usize, usize),
    Gap,
}

/// Flows the sample page into one or two columns inside x..x+cw, y..y+th.
fn flow(s: &mut Svg, x: f64, y: f64, cw: f64, th: f64, two: bool, t: &Theme, r: &R) -> Anchors {
    let mut anchors = Anchors::default();
    let gap_col = 56.0;
    let colw = if two {
        ((cw - gap_col) / 2.0).min(640.0)
    } else {
        cw.min(620.0)
    };
    let total = if two { colw * 2.0 + gap_col } else { colw };
    let x0 = x + (cw - total) / 2.0;
    let size = (colw / 27.0).clamp(10.0, 24.0);
    let lh = size * 1.66;
    let mut items = vec![Item::Head];
    for (p, para) in PARAS.iter().enumerate() {
        for l in 0..para.len() {
            items.push(Item::Line(p, l));
        }
        items.push(Item::Gap);
    }
    let mut col = 0;
    let mut cy = y;
    let eink = r.eink;
    for it in items {
        let h = match it {
            Item::Head => lh * 3.0,
            Item::Line(..) => lh,
            Item::Gap => lh * 0.45,
        };
        if cy + h > y + th {
            if two && col == 0 {
                col = 1;
                cy = y;
            } else {
                break;
            }
        }
        let cx = x0 + col as f64 * (colw + gap_col);
        match it {
            Item::Head => {
                s.text(
                    cx,
                    cy + size * 0.9,
                    size * 0.62,
                    true,
                    t.dim,
                    "start",
                    "CHAPTER 1",
                );
                s.serif(cx, cy + size * 2.5, size * 1.9, true, t.fg, "Loomings.");
            }
            Item::Gap => {}
            Item::Line(p, l) => {
                let text = PARAS[p][l];
                let last = l + 1 == PARAS[p].len();
                let base = cy + size * 1.05;
                let lw = if last { text_em(text) * size } else { colw };
                let mut marks: Vec<(&Mark, f64, f64)> = Vec::new();
                let mut all: Vec<&Mark> = MARKS.iter().filter(|_| r.highlights).collect();
                let sel = Mark {
                    find: SELECTED,
                    style: Hl::Selection,
                    color: 0,
                    note: false,
                };
                if r.selection {
                    all.push(&sel);
                }
                for m in all {
                    if let Some(b) = text.find(m.find) {
                        let all_em = text_em(text);
                        let from = text_em(&text[..b]);
                        let len = text_em(m.find);
                        marks.push((m, cx + lw * from / all_em, lw * len / all_em));
                    }
                }
                for (m, mx, mw) in &marks {
                    draw_mark(s, *mx, base, *mw, size, m, t, eink);
                    if m.style == Hl::Selection {
                        anchors.selection = Some((mx + mw / 2.0, base - size * 1.05));
                    }
                    if m.note && anchors.note.is_none() {
                        anchors.note = Some((mx + mw / 2.0, base - size * 1.05));
                    }
                }
                let lead_words = match (r.lead, p, l) {
                    (Some(c), 0, 0) => Some((c, "Call me Ishmael.")),
                    _ => None,
                };
                if let Some((c, words)) = lead_words {
                    let lw_lead = text_em(words) * size + size * 0.3;
                    s.serif(cx, base, size, true, c, words);
                    let rest = &text[words.len()..];
                    s.serif_fit(cx + lw_lead, base, size, t.fg, colw - lw_lead, rest);
                } else if last {
                    s.serif(cx, base, size, false, t.fg, text);
                } else {
                    s.serif_fit(cx, base, size, t.fg, colw, text);
                }
                for (m, mx, mw) in &marks {
                    draw_mark_line(s, *mx, base, *mw, size, m, eink);
                }
                if text.ends_with('-') && anchors.hyphen.is_none() {
                    anchors.hyphen = Some((cx + colw, base - size * 0.3));
                }
            }
        }
        cy += h;
    }
    if two {
        s.rect_a(
            x0 + colw + gap_col / 2.0,
            y + 4.0,
            1.0,
            th - 8.0,
            0.0,
            t.fg,
            0.18,
        );
    }
    anchors
}

pub fn reader_window(s: &mut Svg, x: f64, y: f64, w: f64, h: f64, t: &Theme, r: &R) -> Anchors {
    if r.eink {
        s.rect(x, y, w, h, 14.0, t.bg, Some("#000000"));
        s.rect(
            x - 1.0,
            y - 1.0,
            w + 2.0,
            h + 2.0,
            15.0,
            "none",
            Some("#000000"),
        );
    } else {
        s.shadow(x, y, w, h, 14.0, t.shadow);
        s.rect(x, y, w, h, 14.0, t.bg, Some(t.border));
    }
    let mut top = y;
    let bottom;
    if r.chrome {
        s.path(
            &format!(
                "M{x},{} a14,14 0 0 1 14,-14 h{} a14,14 0 0 1 14,14 v{} h-{w} z",
                y + 14.0,
                w - 28.0,
                32.0
            ),
            t.header,
            "none",
            0.0,
        );
        s.line(
            x,
            y + 46.0,
            x + w,
            y + 46.0,
            t.border,
            if r.eink { 2.0 } else { 1.0 },
        );
        icon_chev(s, x + 28.0, y + 23.0, t.fg, 'l');
        s.rect(
            x + 54.0,
            y + 9.0,
            34.0,
            30.0,
            8.0,
            if r.sidebar.is_some() { t.hover } else { "none" },
            None,
        );
        icon_sidebar(s, x + 71.0, y + 24.0, t.fg, false);
        icon_bookmark(s, x + 112.0, y + 24.0, t.fg, false);
        if w < 700.0 {
            s.text(x + 136.0, y + 28.0, 13.0, true, t.fg, "start", "Moby-Dick");
        } else {
            s.text(
                x + w / 2.0,
                y + 21.0,
                14.0,
                true,
                t.fg,
                "middle",
                "Moby-Dick; or, The Whale",
            );
            s.text(
                x + w / 2.0,
                y + 38.0,
                11.0,
                false,
                t.dim,
                "middle",
                "Chapter 1",
            );
        }
        icon_search(s, x + w - 232.0, y + 24.0, t.fg);
        icon_aa(s, x + w - 192.0, y + 24.0, t.fg);
        icon_full(s, x + w - 152.0, y + 24.0, t.fg);
        icon_menu(s, x + w - 112.0, y + 24.0, t.fg);
        s.circle(x + w - 26.0, y + 23.0, 12.0, t.hover);
        icon_close(s, x + w - 26.0, y + 23.0, t.fg);
        top = y + 46.0;
        bottom = y + h - 78.0;
    } else {
        bottom = y + h - 56.0;
    }
    let mut left = x;
    if let Some(tab) = r.sidebar {
        let sw = 280.0;
        let sh = if r.chrome { bottom - top } else { y + h - top };
        s.rect(left, top, sw, sh, 0.0, t.panel, None);
        s.line(left + sw, top, left + sw, top + sh, t.border, 1.0);
        sidebar(s, left, top, sw, sh, t, tab);
        left += sw;
    }
    let cw = x + w - left - 2.0 * 44.0;
    let lead = if r.chrome { 28.0 } else { 46.0 };
    let th = bottom - top - 40.0 - (lead - 28.0);
    let anchors = flow(
        s,
        left + 44.0,
        top + lead,
        cw,
        th,
        r.layout == Layout::Two,
        t,
        r,
    );
    if r.layout == Layout::Scroll {
        s.rect(x + w - 16.0, top + 60.0, 6.0, 120.0, 3.0, t.dim, None);
    }
    // bottom: previous / slider / next, then "page of book" and "chapter · time to complete"
    if r.chrome {
        let by = y + h - 78.0;
        s.line(x, by, x + w, by, t.border, 1.0);
        icon_chev(s, x + 36.0, by + 26.0, t.fg, 'l');
        icon_chev(s, x + w - 36.0, by + 26.0, t.fg, 'r');
        slider(s, x + 64.0, by + 26.0, w - 128.0, 0.05, t);
        s.text(x + 30.0, by + 64.0, 12.0, false, t.dim, "start", "9 of 180");
        s.text(
            x + w - 30.0,
            by + 64.0,
            12.0,
            false,
            t.dim,
            "end",
            "Loomings  ·  3 minutes to complete",
        );
    } else {
        s.text(
            x + 60.0,
            y + h - 24.0,
            12.0,
            false,
            t.dim,
            "start",
            "9 of 180",
        );
        s.text(
            x + w / 2.0,
            y + 26.0,
            12.0,
            false,
            t.dim,
            "middle",
            "Moby-Dick; or, The Whale",
        );
        s.text(
            x + w - 60.0,
            y + h - 24.0,
            12.0,
            false,
            t.dim,
            "end",
            "Loomings  ·  3 minutes to complete",
        );
    }
    anchors
}

fn tabs(s: &mut Svg, x: f64, y: f64, w: f64, t: &Theme, active: &str) {
    let names = ["Contents", "Bookmarks", "Notes", "Search"];
    let tw = w / 4.0;
    s.line(x, y, x + w, y, t.border, 1.0);
    for (i, n) in names.iter().enumerate() {
        let cx = x + tw * i as f64 + tw / 2.0;
        if *n == active {
            s.rect(
                x + tw * i as f64 + 6.0,
                y + 6.0,
                tw - 12.0,
                46.0,
                10.0,
                t.hover,
                None,
            );
        }
        s.rect(cx - 7.0, y + 12.0, 14.0, 14.0, 3.0, t.fg, None);
        s.text(cx, y + 44.0, 11.0, true, t.fg, "middle", n);
    }
}

fn sidebar(s: &mut Svg, x: f64, y: f64, w: f64, h: f64, t: &Theme, tab: &str) {
    s.text(
        x + 18.0,
        y + 30.0,
        14.0,
        true,
        t.fg,
        "start",
        "Moby-Dick; or, The Whale",
    );
    s.line(x, y + 44.0, x + w, y + 44.0, t.border, 1.0);
    tabs(s, x, y + h - 56.0, w, t, tab);
    let mut yy = y + 76.0;
    match tab {
        "Contents" => {
            let toc: [(&str, usize, bool); 12] = [
                ("Etymology", 0, false),
                ("Extracts", 0, false),
                ("1. Loomings", 0, true),
                ("2. The Carpet-Bag", 0, false),
                ("3. The Spouter-Inn", 0, false),
                ("4. The Counterpane", 0, false),
                ("5. Breakfast", 0, false),
                ("6. The Street", 0, false),
                ("7. The Chapel", 0, false),
                ("8. The Pulpit", 0, false),
                ("9. The Sermon", 0, false),
                ("10. A Bosom Friend", 0, false),
            ];
            for (n, d, cur) in toc {
                if cur {
                    s.rect(x + 8.0, yy - 20.0, w - 16.0, 32.0, 8.0, t.hover, None);
                }
                let c: String = n.chars().take(30).collect();
                s.text(
                    x + 18.0 + 16.0 * d as f64,
                    yy,
                    13.0,
                    d == 0,
                    t.fg,
                    "start",
                    &c,
                );
                yy += 36.0;
            }
        }
        "Bookmarks" => {
            for (c, sub) in [
                ("1. Loomings", "Page 9 · 12 Nov 2026"),
                ("5. Breakfast", "Page 41 · 10 Nov 2026"),
                ("Extracts", "Page 3 · 3 Nov 2026"),
            ] {
                s.text(x + 18.0, yy, 14.0, true, t.fg, "start", c);
                s.text(x + 18.0, yy + 20.0, 12.0, false, t.dim, "start", sub);
                icon_trash(s, x + w - 30.0, yy + 8.0, t.dim);
                s.line(x, yy + 34.0, x + w, yy + 34.0, t.border, 1.0);
                yy += 62.0;
            }
        }
        "Notes" => {
            for (i, (q, n)) in [
                (
                    "whenever it is a damp, drizzly November in my soul",
                    "Compare with the opening of chapter 2",
                ),
                (
                    "tery part of the world. It is a way I have of driving off the spleen",
                    "",
                ),
                (
                    "involuntarily pausing before coffin warehouses",
                    "Use as a quote",
                ),
            ]
            .iter()
            .enumerate()
            {
                s.rect(x + 14.0, yy - 14.0, 5.0, 54.0, 2.0, HL[[0, 2, 1][i]], None);
                let cut = q
                    .char_indices()
                    .filter(|(i, c)| *c == ' ' && *i <= 30)
                    .map(|(i, _)| i)
                    .next_back()
                    .unwrap_or(q.len());
                s.text(x + 30.0, yy, 13.0, false, t.fg, "start", &q[..cut]);
                s.text(
                    x + 30.0,
                    yy + 18.0,
                    13.0,
                    false,
                    t.fg,
                    "start",
                    q[cut..].trim(),
                );
                if !n.is_empty() {
                    s.text(x + 30.0, yy + 38.0, 12.0, false, t.accent, "start", n);
                }
                icon_trash(s, x + w - 26.0, yy - 4.0, t.dim);
                yy += 80.0;
            }
        }
        _ => {
            s.rect(x + 12.0, yy - 24.0, w - 24.0, 36.0, 9.0, t.hover, None);
            icon_search(s, x + 32.0, yy - 6.0, t.dim);
            s.text(x + 50.0, yy, 13.0, false, t.fg, "start", "whale");
            s.text(
                x + w / 2.0,
                yy + 30.0,
                12.0,
                false,
                t.dim,
                "middle",
                "37 results",
            );
            yy += 54.0;
            for (a, b) in [
                ("…the whale, the Whale itself, was…", "Etymology"),
                ("…a whale ship was my proper…", "1. Loomings"),
                ("…the great whale in his…", "2. The Carpet-Bag"),
                ("…of whales and whaling…", "3. The Spouter-Inn"),
            ] {
                s.text(x + 18.0, yy, 13.0, false, t.fg, "start", a);
                s.text(x + 18.0, yy + 18.0, 12.0, false, t.dim, "start", b);
                s.line(x, yy + 30.0, x + w, yy + 30.0, t.border, 1.0);
                yy += 56.0;
            }
        }
    }
}

/// The "Text appearance" popover (Aa): font, line spacing, margins, justify.
pub fn appearance_popover(s: &mut Svg, x: f64, y: f64, t: &Theme) {
    let (w, h) = (320.0, 270.0);
    popover_card(s, x, y, w, h, x + w - 40.0, true, t);
    s.text(x + 20.0, y + 42.0, 14.0, false, t.fg, "start", "Font");
    s.rect(x + w - 190.0, y + 20.0, 170.0, 34.0, 8.0, t.hover, None);
    s.text(
        x + w - 178.0,
        y + 42.0,
        13.0,
        false,
        t.fg,
        "start",
        "Noto Serif",
    );
    icon_chev(s, x + w - 38.0, y + 36.0, t.fg, 'd');
    s.text(
        x + 20.0,
        y + 92.0,
        14.0,
        false,
        t.fg,
        "start",
        "Line spacing",
    );
    slider(s, x + 22.0, y + 116.0, w - 44.0, 0.45, t);
    s.text(x + 20.0, y + 158.0, 14.0, false, t.fg, "start", "Margins");
    slider(s, x + 22.0, y + 182.0, w - 44.0, 0.3, t);
    s.text(
        x + 20.0,
        y + 238.0,
        14.0,
        false,
        t.fg,
        "start",
        "Justify text",
    );
    switch(s, x + w - 70.0, y + 222.0, true, t);
}

/// The selection popover. Returns its height. `note` shows the note of an existing highlight
/// at the top (touch screens have no hover); `style` is 0 background, 1 underline,
/// 2 strikethrough, 3 squiggly; `color` indexes `HL` (the ring marks the colour of an existing highlight).
pub fn selection_popover(
    s: &mut Svg,
    x: f64,
    y: f64,
    t: &Theme,
    note: Option<&str>,
    style: usize,
    color: Option<usize>,
    eink: bool,
) -> f64 {
    let w = 440.0;
    let note_h = if note.is_some() { 52.0 } else { 0.0 };
    let h = 12.0 + note_h + 44.0 + 8.0 + 38.0 + 10.0;
    s.shadow(x, y, w, h, 14.0, if eink { 0.0 } else { 0.3 });
    s.rect(x, y, w, h, 14.0, t.card, Some(t.border));
    // arrow below the card, pointing at the selection
    s.path(
        &format!("M{},{} l10,10 l10,-10", x + w / 2.0 - 10.0, y + h),
        t.card,
        t.border,
        1.0,
    );
    s.rect(x + w / 2.0 - 9.0, y + h - 1.5, 18.0, 3.0, 0.0, t.card, None);
    let mut yy = y + 10.0;
    if let Some(n) = note {
        s.rect_a(x + 8.0, yy, w - 16.0, 44.0, 8.0, t.fg, 0.06);
        s.text(x + 20.0, yy + 27.0, 13.0, false, t.fg, "start", n);
        yy += note_h;
    }
    // colours, then note · copy · dictionary · Wikipedia · translate · remove
    for (i, c) in HL.iter().enumerate() {
        let cx = x + 28.0 + i as f64 * 34.0;
        let fill = if eink { "#808080" } else { c };
        s.circle(cx, yy + 22.0, 12.0, fill);
        if color == Some(i) {
            s.ring(cx, yy + 22.0, 15.0, t.fg, 2.0);
        }
    }
    s.line(x + 188.0, yy + 8.0, x + 188.0, yy + 36.0, t.border, 1.0);
    for (i, k) in ["note", "copy", "dict", "wiki", "translate", "trash"]
        .iter()
        .enumerate()
    {
        let cx = x + 214.0 + i as f64 * 38.0;
        let cy = yy + 22.0;
        match *k {
            "note" => icon_pencil(s, cx, cy, t.fg),
            "copy" => {
                s.rect(cx - 6.0, cy - 7.0, 10.0, 12.0, 2.0, "none", Some(t.fg));
                s.rect(cx - 2.0, cy - 3.0, 10.0, 12.0, 2.0, t.card, Some(t.fg));
            }
            "dict" => {
                s.rect(cx - 6.0, cy - 8.0, 12.0, 16.0, 2.0, "none", Some(t.fg));
                s.line(cx - 3.0, cy - 3.0, cx + 3.0, cy - 3.0, t.fg, 1.4);
                s.line(cx - 3.0, cy + 1.0, cx + 3.0, cy + 1.0, t.fg, 1.4);
            }
            "wiki" => s.text(cx, cy + 6.0, 17.0, true, t.fg, "middle", "W"),
            "translate" => s.text(cx, cy + 5.0, 14.0, true, t.fg, "middle", "文A"),
            _ => icon_trash(s, cx, cy, if eink { t.fg } else { "#e01b24" }),
        }
    }
    yy += 44.0 + 8.0;
    // highlight style: background · underline · strikethrough · squiggly
    for i in 0..4 {
        let bx = x + 14.0 + i as f64 * 40.0;
        s.rect(
            bx,
            yy,
            36.0,
            34.0,
            8.0,
            if i == style { t.hover } else { "none" },
            None,
        );
        let cx = bx + 18.0;
        match i {
            0 => {
                s.rect(
                    cx - 10.0,
                    yy + 7.0,
                    20.0,
                    20.0,
                    3.0,
                    if eink { "#cccccc" } else { "#f6d32d" },
                    None,
                );
                s.text(
                    cx,
                    yy + 23.0,
                    14.0,
                    true,
                    if eink { "#000000" } else { "#241f31" },
                    "middle",
                    "A",
                );
            }
            1 => {
                s.text(cx, yy + 22.0, 14.0, true, t.fg, "middle", "A");
                s.line(cx - 7.0, yy + 27.0, cx + 7.0, yy + 27.0, t.fg, 1.6);
            }
            2 => {
                s.text(cx, yy + 23.0, 14.0, true, t.fg, "middle", "A");
                s.line(cx - 8.0, yy + 17.0, cx + 8.0, yy + 17.0, t.fg, 1.6);
            }
            _ => {
                s.text(cx, yy + 22.0, 14.0, true, t.fg, "middle", "A");
                s.path(
                    &format!(
                        "M{},{} l2,-2 l2,2 l2,-2 l2,2 l2,-2 l2,2 l2,-2",
                        cx - 8.0,
                        yy + 27.0
                    ),
                    "none",
                    if eink { t.fg } else { "#e01b24" },
                    1.4,
                );
            }
        }
    }
    s.text(
        x + 14.0 + 4.0 * 40.0 + 12.0,
        yy + 22.0,
        12.0,
        false,
        t.dim,
        "start",
        "How the highlight is drawn",
    );
    h
}

/// The note editor page of the selection popover.
pub fn note_popover(s: &mut Svg, x: f64, y: f64, t: &Theme) {
    popover_card(s, x, y, 320.0, 170.0, x + 160.0, false, t);
    s.rect(x + 14.0, y + 14.0, 292.0, 92.0, 8.0, t.bg, Some(t.border));
    s.text(
        x + 26.0,
        y + 40.0,
        14.0,
        false,
        t.fg,
        "start",
        "Compare with the opening of chapter 2 —",
    );
    s.text(
        x + 26.0,
        y + 62.0,
        14.0,
        false,
        t.fg,
        "start",
        "the same gloom, a different harbour.",
    );
    icon_chev(s, x + 28.0, y + 138.0, t.fg, 'l');
    s.rect(x + 60.0, y + 120.0, 246.0, 36.0, 8.0, t.accent, None);
    s.text(
        x + 183.0,
        y + 144.0,
        14.0,
        true,
        "#ffffff",
        "middle",
        "Save Note",
    );
}

/// The bubble that shows a note instantly while the pointer is over its highlight.
pub fn hover_note(s: &mut Svg, cx: f64, y: f64, t: &Theme, lines: &[&str]) {
    let w = 300.0;
    let h = 20.0 + lines.len() as f64 * 20.0;
    let x = cx - w / 2.0;
    let y = y - h - 12.0;
    popover_card(s, x, y, w, h, cx, false, t);
    for (i, l) in lines.iter().enumerate() {
        s.text(
            x + 16.0,
            y + 24.0 + i as f64 * 20.0,
            13.0,
            false,
            t.fg,
            "start",
            l,
        );
    }
}

/// Colour mode shown in the main menu and the editor's preview popover.
#[derive(Clone, Copy, PartialEq)]
pub enum Colors {
    Theme,
    Original,
    Custom,
}

/// The linked `Theme | Original | Custom` toggle row, and, for Custom, the two colour buttons.
/// Returns the height used.
pub fn color_controls(s: &mut Svg, x: f64, y: f64, w: f64, mode: Colors, t: &Theme) -> f64 {
    let bw = w / 3.0;
    for (i, (m, name)) in [
        (Colors::Theme, "Theme"),
        (Colors::Original, "Original"),
        (Colors::Custom, "Custom"),
    ]
    .iter()
    .enumerate()
    {
        let bx = x + i as f64 * bw;
        let on = *m == mode;
        let r = if i == 0 || i == 2 { 8.0 } else { 0.0 };
        s.rect(
            bx,
            y,
            bw,
            30.0,
            r,
            if on { t.border } else { t.hover },
            Some(t.border),
        );
        s.text(bx + bw / 2.0, y + 20.0, 13.0, on, t.fg, "middle", name);
    }
    if mode != Colors::Custom {
        return 30.0;
    }
    let ry = y + 40.0;
    s.text(x + 4.0, ry + 19.0, 13.0, false, t.fg, "start", "Text");
    s.rect(x + 44.0, ry, 64.0, 28.0, 8.0, t.hover, Some(t.border));
    s.rect(
        x + 50.0,
        ry + 6.0,
        52.0,
        16.0,
        4.0,
        "#ffcc00",
        Some(t.border),
    );
    s.text(x + 132.0, ry + 19.0, 13.0, false, t.fg, "start", "Page");
    s.rect(x + 176.0, ry, 64.0, 28.0, 8.0, t.hover, Some(t.border));
    s.rect(
        x + 182.0,
        ry + 6.0,
        52.0,
        16.0,
        4.0,
        "#101010",
        Some(t.border),
    );
    s.text(x + 248.0, ry + 19.0, 10.0, false, t.dim, "start", "");
    70.0
}

fn main_menu(s: &mut Svg, x: f64, y: f64, t: &Theme, mode: Colors) {
    let w = 300.0;
    let extra = if mode == Colors::Custom { 114.0 } else { 44.0 };
    let h = 420.0 + extra;
    s.shadow(x, y, w, h, 16.0, 0.3);
    s.rect(x, y, w, h, 16.0, t.card, Some(t.border));
    for (i, c) in ["#ffffff", "#ffffff", "#f4ecd8", "#1e1e21", "#000000"]
        .iter()
        .enumerate()
    {
        let cx = x + 38.0 + i as f64 * 56.0;
        let sel = mode == Colors::Theme && i == 3;
        s.circle(cx, y + 36.0, 18.0, c);
        s.ring(
            cx,
            y + 36.0,
            18.0,
            if sel { t.accent } else { t.border },
            if sel { 3.0 } else { 1.5 },
        );
    }
    let used = color_controls(s, x + 16.0, y + 68.0, w - 32.0, mode, t);
    let y = y + used + 14.0;
    s.circle(x + 40.0, y + 92.0, 16.0, t.hover);
    s.line(x + 33.0, y + 92.0, x + 47.0, y + 92.0, t.fg, 2.0);
    s.text(
        x + w / 2.0 - 20.0,
        y + 98.0,
        16.0,
        true,
        t.fg,
        "middle",
        "100%",
    );
    s.circle(x + w - 90.0, y + 92.0, 16.0, t.hover);
    icon_plus(s, x + w - 90.0, y + 92.0, t.fg);
    s.circle(x + w - 40.0, y + 92.0, 16.0, t.hover);
    s.text(x + w - 40.0, y + 98.0, 12.0, true, t.fg, "middle", "1:1");
    let rows: [(&str, &str); 7] = [
        ("Open Book…", ""),
        ("Reload", "Ctrl+R"),
        ("Properties", ""),
        ("Layout", "›"),
        ("Preferences", "Ctrl+,"),
        ("Keyboard Shortcuts", "Ctrl+?"),
        ("About Bookx", ""),
    ];
    let mut yy = y + 126.0;
    for (i, (a, b)) in rows.iter().enumerate() {
        if i == 0 || i == 3 || i == 4 {
            s.line(x + 12.0, yy, x + w - 12.0, yy, t.border, 1.0);
            yy += 8.0;
        }
        s.text(x + 22.0, yy + 22.0, 14.0, false, t.fg, "start", a);
        s.text(x + w - 22.0, yy + 22.0, 12.0, false, t.dim, "end", b);
        yy += 38.0;
    }
}

fn layout_menu(s: &mut Svg, x: f64, y: f64, t: &Theme) {
    s.shadow(x, y, 240.0, 170.0, 14.0, 0.3);
    s.rect(x, y, 240.0, 170.0, 14.0, t.card, Some(t.border));
    icon_chev(s, x + 24.0, y + 28.0, t.fg, 'l');
    s.text(x + 48.0, y + 33.0, 15.0, true, t.fg, "start", "Layout");
    for (i, n) in ["Two Columns", "Single Column", "Scroll"]
        .iter()
        .enumerate()
    {
        let yy = y + 70.0 + i as f64 * 34.0;
        s.ring(x + 28.0, yy, 8.0, t.dim, 1.5);
        if i == 0 {
            s.circle(x + 28.0, yy, 4.5, t.accent);
        }
        s.text(x + 48.0, yy + 5.0, 14.0, false, t.fg, "start", n);
    }
}

fn lookup_popover(s: &mut Svg, x: f64, y: f64, kind: &str, t: &Theme) {
    let (w, h) = (360.0, 190.0);
    s.shadow(x, y, w, h, 14.0, 0.3);
    s.rect(x, y, w, h, 14.0, t.card, Some(t.border));
    icon_chev(s, x + 24.0, y + 28.0, t.fg, 'l');
    match kind {
        "dict" => {
            s.text(x + 48.0, y + 33.0, 14.0, true, t.fg, "start", "Dictionary");
            s.text(x + 22.0, y + 70.0, 16.0, true, t.fg, "start", "chapter");
            s.text(
                x + 22.0,
                y + 88.0,
                11.0,
                false,
                t.dim,
                "start",
                "Mini Test Dict",
            );
            s.lines(
                x + 22.0,
                y + 114.0,
                13.0,
                20.0,
                t.fg,
                &[
                    "A main division of a book.",
                    "Words you look up are saved to Vocabulary.",
                ],
            );
        }
        "wiki" => {
            s.text(x + 48.0, y + 33.0, 14.0, true, t.fg, "start", "Wikipedia");
            s.text(x + 22.0, y + 70.0, 14.0, true, t.fg, "start", "Moby-Dick");
            s.lines(
                x + 22.0,
                y + 94.0,
                13.0,
                20.0,
                t.fg,
                &[
                    "Moby-Dick; or, The Whale is an 1851 novel",
                    "by American writer Herman Melville.",
                ],
            );
            s.text(
                x + w - 22.0,
                y + h - 16.0,
                12.0,
                false,
                t.accent,
                "end",
                "Open Wikipedia ↗",
            );
        }
        _ => {
            s.rect(x + w - 150.0, y + 12.0, 130.0, 32.0, 8.0, t.hover, None);
            s.text(x + w - 138.0, y + 33.0, 13.0, false, t.fg, "start", "Hindi");
            icon_chev(s, x + w - 34.0, y + 28.0, t.fg, 'd');
            s.text(x + 22.0, y + 90.0, 26.0, false, t.fg, "start", "दुनिया");
            s.text(
                x + w - 22.0,
                y + h - 16.0,
                12.0,
                false,
                t.dim,
                "end",
                "Google Translate",
            );
        }
    }
}

/// One cell of the highlight-style table: a phrase drawn in `style`.
fn style_cell(
    s: &mut Svg,
    x: f64,
    y: f64,
    w: f64,
    t: &Theme,
    style: Hl,
    color: usize,
    note: bool,
    eink: bool,
) {
    s.rect(x, y, w, 52.0, 10.0, t.bg, Some(t.border));
    let size = 20.0;
    let phrase = "a damp, drizzly November";
    let tw = text_em(phrase) * size;
    let tx = x + 22.0;
    let base = y + 34.0;
    let m = Mark {
        find: "",
        style,
        color,
        note,
    };
    draw_mark(s, tx, base, tw, size, &m, t, eink);
    s.serif(tx, base, size, false, t.fg, phrase);
    draw_mark_line(s, tx, base, tw, size, &m, eink);
}

pub fn run(s: &mut Svg) {
    let l = &LIGHT;
    s.text(90.0, 130.0, 74.0, true, "#2e3436", "start", "Bookx Reader");
    s.lines(90.0, 180.0, 17.0, 24.0, "#5e5c64", &[
        "A reader built on Bookx’s own Rust text engine (Pango + cairo): real pagination, two columns, hyphenation, selection, highlights and notes — no web view, no JavaScript.",
        "It opens in the same window as the library; Esc or the back button returns to the library. Reading position is saved automatically.",
        "Sample text: Moby-Dick by Herman Melville (public domain).",
    ]);

    // ------------------------------------------------------------ 2.a / 2.b
    label(
        s,
        90.0,
        300.0,
        "2.a Reader — two columns, hyphenation, highlights (light theme)",
    );
    let a = reader_window(s, 90.0, 320.0, 1450.0, 660.0, l, &R::default());
    if let Some((hx, hy)) = a.hyphen {
        s.ring(hx - 4.0, hy, 14.0, "#e01b24", 2.0);
        s.text(
            hx - 4.0,
            hy - 22.0,
            12.0,
            true,
            "#e01b24",
            "middle",
            "wa-tery",
        );
    }
    note(s, 90.0, 1016.0, &[
        "Header: back · contents/notes sidebar · add bookmark · title and chapter · search · text appearance (Aa) · fullscreen · main menu.",
        "Footer: previous / next page and the progress slider; below it “page of book” on the left and “chapter · minutes to complete” on the right.",
        "Hyphenation (ringed) breaks long words at line ends in the language of the book, so justified lines stay evenly spaced. Preferences → Hyphenate words.",
    ]);

    label(s, 1600.0, 300.0, "2.b Sidebar — Contents (sepia theme)");
    reader_window(
        s,
        1600.0,
        320.0,
        1450.0,
        660.0,
        &SEPIA,
        &R {
            sidebar: Some("Contents"),
            layout: Layout::Single,
            ..Default::default()
        },
    );

    // ------------------------------------------------------------ 2.c / 2.d / 2.e
    label(s, 90.0, 1140.0, "2.c Selecting text — colours, note, copy, dictionary, Wikipedia, translate, highlight style");
    let a = reader_window(
        s,
        90.0,
        1160.0,
        1000.0,
        700.0,
        l,
        &R {
            layout: Layout::Single,
            chrome: false,
            highlights: false,
            selection: true,
            ..Default::default()
        },
    );
    if let Some((px, py)) = a.selection {
        let h = 12.0 + 44.0 + 8.0 + 38.0 + 10.0;
        selection_popover(s, px - 220.0, py - h - 14.0, l, None, 0, None, false);
    }
    label(
        s,
        1130.0,
        1140.0,
        "2.d An existing highlight — its note shows in the popover (dark theme)",
    );
    let a = reader_window(
        s,
        1130.0,
        1160.0,
        1000.0,
        700.0,
        &DARK,
        &R {
            layout: Layout::Single,
            chrome: false,
            highlights: true,
            ..Default::default()
        },
    );
    if let Some((px, py)) = a.note {
        let h = 12.0 + 52.0 + 44.0 + 8.0 + 38.0 + 10.0;
        selection_popover(
            s,
            px - 220.0,
            py - h - 14.0,
            &DARK,
            Some("Compare with the opening of chapter 2"),
            0,
            Some(1),
            false,
        );
    }
    label(s, 2170.0, 1140.0, "2.e Hover a note — it appears at once");
    let a = reader_window(
        s,
        2170.0,
        1160.0,
        930.0,
        700.0,
        l,
        &R {
            layout: Layout::Single,
            chrome: false,
            highlights: true,
            ..Default::default()
        },
    );
    if let Some((px, py)) = a.note {
        hover_note(s, px, py, l, &["Compare with the opening of", "chapter 2"]);
    }
    note(s, 90.0, 1896.0, &[
        "Selecting text opens the popover: five highlight colours, then Note, Copy, Dictionary, Wikipedia, Translate (and Remove on an existing highlight).",
        "The bottom row chooses how the highlight is drawn: background, underline, strikethrough or squiggly. Clicking a highlight edits it; its note is shown in the popover",
        "(touch screens have no hover) and, with a mouse, in a bubble as soon as the pointer is over the highlight. Dictionary look-ups are saved to Vocabulary.",
    ]);
    note(s, 90.0, 1972.0, &["Notes: a highlight with a note keeps a solid 2 px line in its colour under the text (2.d, 2.e)."]);

    // ------------------------------------------------------------ 2.f styles
    label(
        s,
        90.0,
        2080.0,
        "2.f Highlight styles — light · dark · e-ink",
    );
    {
        let (x, y) = (90.0, 2100.0);
        let cols = [
            ("Light", &LIGHT, false),
            ("Dark", &DARK, false),
            ("E-ink", &EINK, true),
        ];
        let rows: [(&str, Hl, usize, bool); 5] = [
            ("Background", Hl::Background, 0, false),
            ("Underline", Hl::Underline, 2, false),
            ("Strikethrough", Hl::Strike, 3, false),
            ("Squiggly", Hl::Squiggly, 4, false),
            ("Background + note", Hl::Background, 1, true),
        ];
        for (ci, (name, th, _)) in cols.iter().enumerate() {
            s.text(
                x + 190.0 + ci as f64 * 370.0 + 150.0,
                y + 14.0,
                13.0,
                true,
                "#5e5c64",
                "middle",
                name,
            );
            s.rect(
                x + 190.0 + ci as f64 * 370.0 - 6.0,
                y + 26.0,
                352.0,
                rows.len() as f64 * 62.0 + 8.0,
                12.0,
                th.bg,
                None,
            );
        }
        for (ri, (name, st, c, nt)) in rows.iter().enumerate() {
            let yy = y + 30.0 + ri as f64 * 62.0;
            s.text(x, yy + 32.0, 14.0, false, "#2e3436", "start", name);
            for (ci, (_, th, eink)) in cols.iter().enumerate() {
                style_cell(
                    s,
                    x + 190.0 + ci as f64 * 370.0,
                    yy,
                    340.0,
                    th,
                    *st,
                    *c,
                    *nt,
                    *eink,
                );
            }
        }
    }
    note(s, 90.0, 2500.0, &[
        "Dark pages use a gentler tint so the text stays readable. In e-ink mode colours turn into grey and black lines.",
    ]);

    // sidebar tabs
    label(
        s,
        1500.0,
        2080.0,
        "2.g Sidebar tabs — Bookmarks · Notes · Search",
    );
    for (i, tab) in ["Bookmarks", "Notes", "Search"].iter().enumerate() {
        let x = 1500.0 + i as f64 * 330.0;
        s.shadow(x, 2100.0, 300.0, 640.0, 12.0, 0.2);
        s.rect(x, 2100.0, 300.0, 640.0, 12.0, l.panel, Some(l.border));
        sidebar(s, x, 2100.0, 300.0, 640.0, l, tab);
    }

    // ------------------------------------------------------------ 2.h .. 2.j popovers
    label(s, 90.0, 2840.0, "2.h Text appearance popover (Aa)");
    reader_window(
        s,
        90.0,
        2860.0,
        760.0,
        560.0,
        l,
        &R {
            layout: Layout::Single,
            highlights: false,
            ..Default::default()
        },
    );
    appearance_popover(s, 378.0, 2912.0, l);

    label(s, 900.0, 2840.0, "2.i Main menu · Layout");
    reader_window(
        s,
        900.0,
        2860.0,
        760.0,
        560.0,
        l,
        &R {
            layout: Layout::Single,
            highlights: false,
            ..Default::default()
        },
    );
    main_menu(s, 1300.0, 2908.0, l, Colors::Theme);
    layout_menu(s, 1620.0, 3000.0, l);

    label(
        s,
        1960.0,
        2840.0,
        "2.j Dictionary · Wikipedia · Translate (inline)",
    );
    lookup_popover(s, 1960.0, 2870.0, "dict", l);
    lookup_popover(s, 2350.0, 2870.0, "wiki", l);
    lookup_popover(s, 1960.0, 3090.0, "tr", l);
    note_popover(s, 2350.0, 3140.0, l);
    s.text(
        2350.0,
        3340.0,
        12.0,
        false,
        "#5e5c64",
        "start",
        "The note page of the selection popover",
    );
    note(
        s,
        1960.0,
        3330.0,
        &[
            "Dictionary uses your StarDict files.",
            "Wikipedia shows the article summary;",
            "Translate offers a language picker.",
            "Both fall back to the browser offline.",
        ],
    );

    // ------------------------------------------------------------ 2.k layouts
    label(
        s,
        90.0,
        3520.0,
        "2.k Layouts — two columns · single column · continuous scroll",
    );
    for (i, (lay, w)) in [
        (Layout::Two, 640.0),
        (Layout::Single, 640.0),
        (Layout::Scroll, 600.0),
    ]
    .iter()
    .enumerate()
    {
        reader_window(
            s,
            90.0 + i as f64 * 670.0,
            3540.0,
            *w,
            460.0,
            l,
            &R {
                layout: *lay,
                highlights: false,
                ..Default::default()
            },
        );
    }
    note(s, 90.0, 4036.0, &[
        "Two columns pages like a printed book; single column suits narrow windows; scroll keeps one long chapter.",
        "At the end of a chapter the reader carries on with the next one — in scroll mode, just keep scrolling.",
    ]);

    // ------------------------------------------------------------ 2.l themes
    label(
        s,
        90.0,
        4140.0,
        "2.l Themes — Light · Sepia · Dark · Black (or follow the system)",
    );
    for (i, (th, name)) in [
        (&LIGHT, "Light"),
        (&SEPIA, "Sepia"),
        (&DARK, "Dark"),
        (&DARK, "Black"),
    ]
    .iter()
    .enumerate()
    {
        let x = 90.0 + i as f64 * 485.0;
        let mut tt = **th;
        if i == 3 {
            tt.bg = "#000000";
            tt.header = "#000000";
            tt.panel = "#0a0a0a";
        }
        reader_window(
            s,
            x,
            4160.0,
            460.0,
            320.0,
            &tt,
            &R {
                layout: Layout::Single,
                ..Default::default()
            },
        );
        s.text(x + 230.0, 4510.0, 13.0, true, "#5e5c64", "middle", name);
    }

    // ------------------------------------------------------------ 2.m focus
    label(
        s,
        90.0,
        4600.0,
        "2.m Focus mode (F11) and idle chrome — the bars fade, only a quiet status line stays",
    );
    reader_window(
        s,
        90.0,
        4620.0,
        960.0,
        520.0,
        l,
        &R {
            chrome: false,
            highlights: false,
            ..Default::default()
        },
    );
    reader_window(
        s,
        1120.0,
        4620.0,
        960.0,
        520.0,
        &DARK,
        &R {
            chrome: false,
            highlights: false,
            layout: Layout::Single,
            ..Default::default()
        },
    );
    note(
        s,
        2160.0,
        4650.0,
        &[
            "Move the pointer or press a key: header",
            "buttons and the navigation row fade back in.",
            "Tap the centre of the page, or the top and",
            "bottom edge, to bring the bars back.",
            "Esc leaves fullscreen first, then returns to",
            "the library. In e-ink mode the bars never",
            "fade (see the e-ink sheet).",
        ],
    );
    // ------------------------------------------------------------ 2.n colours
    label(
        s,
        90.0,
        5260.0,
        "2.n Colours — Theme · Original · Custom (main menu; the editor's preview has the same control)",
    );
    main_menu(s, 90.0, 5290.0, l, Colors::Original);
    main_menu(s, 440.0, 5290.0, l, Colors::Custom);
    let mut original = LIGHT;
    original.bg = "#fdf6e3";
    reader_window(
        s,
        800.0,
        5290.0,
        1100.0,
        560.0,
        &original,
        &R {
            layout: Layout::Single,
            highlights: false,
            lead: Some("#cc0000"),
            ..Default::default()
        },
    );
    let mut custom = DARK;
    custom.bg = "#101010";
    custom.header = "#1a1a1a";
    custom.panel = "#161616";
    custom.fg = "#ffcc00";
    custom.dim = "#b38f00";
    reader_window(
        s,
        1950.0,
        5290.0,
        1100.0,
        560.0,
        &custom,
        &R {
            layout: Layout::Single,
            highlights: false,
            ..Default::default()
        },
    );
    s.text(
        1350.0,
        5876.0,
        13.0,
        true,
        "#5e5c64",
        "middle",
        "Original — the book’s own CSS: cream page #fdf6e3, red chapter lead",
    );
    s.text(
        2500.0,
        5876.0,
        13.0,
        true,
        "#5e5c64",
        "middle",
        "Custom — text #ffcc00 on page #101010",
    );
    note(s, 90.0, 5880.0, &[
        "Below the five theme swatches a linked toggle chooses the colours: Theme (the swatch), Original (what the book’s CSS says,",
        "on a light page like a browser) or Custom. Custom adds Text and Page colour buttons.",
    ]);
    s.text(
        1600.0,
        6010.0,
        12.0,
        false,
        "#77767b",
        "middle",
        "Mockups are design illustrations. Sample text: Moby-Dick (public domain).",
    );
}
