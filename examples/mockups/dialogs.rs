//! Sheet 3: the dialogs of the app (statistics, sharing, vocabulary, metadata, preferences).

use crate::kit::*;
use crate::library;

/// One row of a preferences group.
#[derive(Clone, Copy)]
enum Row {
    /// Title, subtitle, value on the right.
    Value(&'static str, Option<&'static str>, &'static str),
    Combo(&'static str, Option<&'static str>, &'static str),
    Switch(&'static str, Option<&'static str>, bool),
    /// Title and value of a spin row.
    Spin(&'static str, &'static str),
    /// A password / entry row: small title, then the text.
    Entry(&'static str, &'static str),
    /// A row with a button on the right.
    #[allow(dead_code)]
    Button(&'static str, Option<&'static str>, &'static str),
}

fn row_height(r: &Row) -> f64 {
    match r {
        Row::Value(_, Some(_), _) | Row::Combo(_, Some(_), _) | Row::Switch(_, Some(_), _) => 62.0,
        Row::Button(_, Some(_), _) => 62.0,
        Row::Entry(..) => 58.0,
        _ => 52.0,
    }
}

/// An `AdwPreferencesGroup` with an optional title and description. Returns the y below it.
fn pref_group(
    s: &mut Svg,
    x: f64,
    y: f64,
    w: f64,
    title: Option<&str>,
    desc: &[&str],
    rows: &[Row],
    t: &Theme,
) -> f64 {
    let mut y = y;
    if let Some(title) = title {
        s.text(x + 4.0, y + 14.0, 14.0, true, t.fg, "start", title);
        y += 24.0;
    }
    for d in desc {
        s.text(x + 4.0, y + 14.0, 12.0, false, t.dim, "start", d);
        y += 17.0;
    }
    if title.is_some() || !desc.is_empty() {
        y += 6.0;
    }
    let h: f64 = rows.iter().map(row_height).sum();
    s.rect(x, y, w, h, 12.0, t.card, Some(t.border));
    let mut yy = y;
    for (i, r) in rows.iter().enumerate() {
        let rh = row_height(r);
        if i > 0 {
            row_sep(s, x, yy, w, t);
        }
        match *r {
            Row::Value(a, sub, v) => {
                card_row(s, x, yy, rh, a, sub, t);
                s.text(
                    x + w - 16.0,
                    yy + rh / 2.0 + 5.0,
                    14.0,
                    false,
                    t.dim,
                    "end",
                    v,
                );
            }
            Row::Combo(a, sub, v) => combo_row(s, x, yy, w, rh, a, sub, v, t),
            Row::Switch(a, sub, on) => switch_row(s, x, yy, w, rh, a, sub, on, t),
            Row::Spin(a, v) => {
                card_row(s, x, yy, rh, a, None, t);
                s.text(
                    x + w - 112.0,
                    yy + rh / 2.0 + 5.0,
                    14.0,
                    false,
                    t.fg,
                    "end",
                    v,
                );
                s.circle(x + w - 84.0, yy + rh / 2.0, 14.0, t.hover);
                s.line(
                    x + w - 91.0,
                    yy + rh / 2.0,
                    x + w - 77.0,
                    yy + rh / 2.0,
                    t.fg,
                    2.0,
                );
                s.circle(x + w - 40.0, yy + rh / 2.0, 14.0, t.hover);
                icon_plus(s, x + w - 40.0, yy + rh / 2.0, t.fg);
            }
            Row::Entry(a, v) => {
                s.text(x + 16.0, yy + 22.0, 11.0, false, t.dim, "start", a);
                s.text(x + 16.0, yy + 44.0, 14.0, false, t.fg, "start", v);
            }
            Row::Button(a, sub, label) => {
                card_row(s, x, yy, rh, a, sub, t);
                btn(
                    s,
                    x + w - 116.0,
                    yy + rh / 2.0 - 17.0,
                    100.0,
                    34.0,
                    label,
                    "normal",
                    t,
                );
            }
        }
        yy += rh;
    }
    y + h
}

/// Reading Statistics: totals, the last seven days and the most-read books.
fn stats(s: &mut Svg, x: f64, y: f64, t: &Theme) {
    let (w, h) = (500.0, 820.0);
    let top = adw_dialog(s, x, y, w, h, "Reading Statistics", t);
    let gx = x + 20.0;
    let gw = w - 40.0;
    let yy = pref_group(
        s,
        gx,
        top + 6.0,
        gw,
        None,
        &[],
        &[
            Row::Value("Today", None, "42 min"),
            Row::Value("Last 7 days", None, "3 h 40 min"),
            Row::Value("Last 30 days", None, "14 h 05 min"),
            Row::Value("All time", None, "62 h 20 min"),
            Row::Value("Streak", None, "5 days (longest 12)"),
            Row::Value("Books finished this year", None, "7"),
            Row::Value("Reading speed", None, "238 words per minute"),
        ],
        t,
    );
    // the bar chart: one bar per day, today on the right
    let cy = group_card(s, gx, yy + 18.0, gw, 150.0, Some("Last 7 days"), t);
    let vals = [
        ("Fri", 0.55),
        ("Sat", 0.9),
        ("Sun", 1.0),
        ("Mon", 0.2),
        ("Tue", 0.0),
        ("Wed", 0.45),
        ("Thu", 0.3),
    ];
    let slot = gw / 7.0;
    let chart_h = 150.0 - 36.0;
    for (i, (d, f)) in vals.iter().enumerate() {
        let bw = slot * 0.55;
        let bx = gx + i as f64 * slot + (slot - bw) / 2.0;
        let bh = if *f > 0.0 { (chart_h - 20.0) * f } else { 2.0 };
        let top_y = cy - 28.0 - bh;
        s.rect_a(
            bx,
            top_y,
            bw,
            bh,
            2.0,
            t.fg,
            if *f > 0.0 { 0.75 } else { 0.12 },
        );
        s.text(bx + bw / 2.0, cy - 10.0, 11.0, false, t.dim, "middle", d);
    }
    pref_group(
        s,
        gx,
        cy + 18.0,
        gw,
        Some("Most read"),
        &[],
        &[
            Row::Value("Moby-Dick", None, "5 h 10 min"),
            Row::Value("Designing Data-Intensive Applications", None, "3 h 25 min"),
            Row::Value("A Wizard of Earthsea", None, "1 h 50 min"),
        ],
        t,
    );
}

/// Share Library: OPDS, web page and KOReader sync.
fn share(s: &mut Svg, x: f64, y: f64, t: &Theme) {
    let (w, h) = (500.0, 590.0);
    let top = adw_dialog(s, x, y, w, h, "Share Library", t);
    let gx = x + 20.0;
    let gw = w - 40.0;
    let yy = pref_group(
        s,
        gx,
        top + 8.0,
        gw,
        None,
        &[
            "Other apps and e-readers can browse and download your books",
            "over OPDS: KOReader, Moon+ Reader, Librera, Kybook and more.",
        ],
        &[
            Row::Switch("Share this library", None, true),
            Row::Switch(
                "Allow other devices on my network",
                Some("Needs a password. Only on networks you trust."),
                true,
            ),
            Row::Entry("Password (user name: bookx)", "••••••••••"),
            Row::Spin("Port", "8080"),
            Row::Switch(
                "KOReader progress sync",
                Some("Lets KOReader devices keep their reading position here"),
                true,
            ),
        ],
        t,
    );
    let sy = yy + 18.0;
    s.rect(gx, sy, gw, 130.0, 12.0, t.card, Some(t.border));
    s.text(
        gx + 16.0,
        sy + 28.0,
        14.0,
        false,
        t.fg,
        "start",
        "Sharing at:",
    );
    s.mono(
        gx + 16.0,
        sy + 52.0,
        13.0,
        t.accent,
        "http://192.168.1.20:8080/",
    );
    s.mono(
        gx + 16.0,
        sy + 74.0,
        13.0,
        t.accent,
        "http://localhost:8080/",
    );
    btn(
        s,
        gx + 12.0,
        sy + 88.0,
        130.0,
        32.0,
        "Copy address",
        "flat",
        t,
    );
}

/// The flashcard dialog. `state`: 0 question, 1 answer, 2 finished.
fn vocab(s: &mut Svg, x: f64, y: f64, h: f64, state: usize, t: &Theme) {
    let w = 500.0;
    let top = adw_dialog(s, x, y, w, h, "Vocabulary", t);
    s.text(
        x + 20.0,
        top + 30.0,
        13.0,
        false,
        t.dim,
        "start",
        "24 words · 6 due · 14 learning · 4 mastered",
    );
    let mid = top + (h - 56.0) / 2.0 - 30.0;
    match state {
        0 => {
            s.text(
                x + w / 2.0,
                mid + 16.0,
                38.0,
                true,
                t.fg,
                "middle",
                "ephemeral",
            );
            // the suggested pill button
            s.rect(
                x + w / 2.0 - 100.0,
                y + h - 76.0,
                200.0,
                44.0,
                22.0,
                t.accent,
                None,
            );
            s.text(
                x + w / 2.0,
                y + h - 48.0,
                15.0,
                true,
                "#ffffff",
                "middle",
                "Show Answer",
            );
        }
        1 => {
            s.text(
                x + w / 2.0,
                mid - 30.0,
                38.0,
                true,
                t.fg,
                "middle",
                "ephemeral",
            );
            s.lines(
                x + 40.0,
                mid + 16.0,
                14.0,
                22.0,
                t.fg,
                &["lasting for a very short time"],
            );
            s.lines(
                x + 40.0,
                mid + 60.0,
                14.0,
                22.0,
                t.dim,
                &[
                    "“Fame, like the ephemeral beauty of cherry",
                    "blossoms, is gone almost as soon as it arrives.”",
                ],
            );
            let gw = (w - 40.0 - 24.0) / 4.0;
            for (i, l) in ["Again", "Hard", "Good", "Easy"].iter().enumerate() {
                btn(
                    s,
                    x + 20.0 + i as f64 * (gw + 8.0),
                    y + h - 70.0,
                    gw,
                    40.0,
                    l,
                    "normal",
                    t,
                );
            }
        }
        _ => {
            s.text(
                x + w / 2.0,
                mid + 10.0,
                38.0,
                true,
                t.fg,
                "middle",
                "All done!",
            );
            s.text(
                x + w / 2.0,
                mid + 48.0,
                14.0,
                false,
                t.dim,
                "middle",
                "Reviewed 6 cards, 5 remembered.",
            );
        }
    }
}

/// Find Duplicates: groups of books that look like the same one; pick the copy to keep, then Merge.
fn duplicates(s: &mut Svg, x: f64, y: f64, t: &Theme) {
    let (w, h) = (560.0, 600.0);
    let top = adw_dialog(s, x, y, w, h, "Duplicate Books", t);
    let gx = x + 20.0;
    let gw = w - 40.0;
    let mut yy = top + 10.0;
    for (title, n, rows) in [
        (
            "Moby-Dick",
            2,
            [
                (
                    "Moby-Dick",
                    "Herman Melville · EPUB · 1.2 MB · 5% read",
                    true,
                ),
                (
                    "Moby-Dick; or, The Whale",
                    "Herman Melville · EPUB · 1.4 MB",
                    false,
                ),
            ],
        ),
        (
            "Dune",
            2,
            [
                (
                    "Dune",
                    "Frank Herbert · EPUB · 1.4 MB · isbn:9780441172719",
                    true,
                ),
                (
                    "Dune (Chilton)",
                    "Frank Herbert · EPUB · 0.9 MB · isbn:9780441172719",
                    false,
                ),
            ],
        ),
    ] {
        s.text(gx + 4.0, yy + 14.0, 14.0, true, t.fg, "start", title);
        s.text(
            gx + 4.0,
            yy + 34.0,
            12.0,
            false,
            t.dim,
            "start",
            &format!("{n} copies. Pick the one to keep."),
        );
        btn(
            s,
            gx + gw - 90.0,
            yy + 4.0,
            90.0,
            34.0,
            "Merge",
            "suggested",
            t,
        );
        yy += 50.0;
        s.rect(gx, yy, gw, 124.0, 12.0, t.card, Some(t.border));
        for (i, (a, b, on)) in rows.iter().enumerate() {
            let ry = yy + i as f64 * 62.0;
            if i > 0 {
                row_sep(s, gx, ry, gw, t);
            }
            s.ring(gx + 28.0, ry + 31.0, 9.0, t.dim, 1.5);
            if *on {
                s.circle(gx + 28.0, ry + 31.0, 5.0, t.accent);
            }
            s.text(gx + 52.0, ry + 28.0, 14.0, false, t.fg, "start", a);
            s.text(gx + 52.0, ry + 47.0, 12.0, false, t.dim, "start", b);
        }
        yy += 124.0 + 24.0;
    }
    s.text(
        x + w / 2.0,
        yy + 40.0,
        12.0,
        false,
        t.dim,
        "middle",
        "Merging unites metadata, notes and reading progress in the copy you keep.",
    );
}

/// The five tap-zone layouts as a small diagram each.
fn tap_zones(s: &mut Svg, x: f64, y: f64, t: &Theme) {
    let names = [
        "Edges (narrow strips)",
        "Thirds (tablet)",
        "Thirds, mirrored (left hand)",
        "Wide (no bar toggle)",
        "Off (swipe or keys only)",
    ];
    // (x0, x1, kind) in 0..1; kind: 'p' previous, 'n' next, 'b' bars
    let zones: [&[(f64, f64, char)]; 5] = [
        &[(0.0, 0.18, 'p'), (0.18, 0.82, 'b'), (0.82, 1.0, 'n')],
        &[
            (0.0, 1.0 / 3.0, 'p'),
            (1.0 / 3.0, 2.0 / 3.0, 'b'),
            (2.0 / 3.0, 1.0, 'n'),
        ],
        &[
            (0.0, 1.0 / 3.0, 'n'),
            (1.0 / 3.0, 2.0 / 3.0, 'b'),
            (2.0 / 3.0, 1.0, 'p'),
        ],
        &[(0.0, 1.0 / 3.0, 'p'), (1.0 / 3.0, 1.0, 'n')],
        &[],
    ];
    let (pw, ph) = (130.0, 180.0);
    for (i, name) in names.iter().enumerate() {
        let px = x + i as f64 * (pw + 40.0);
        s.rect(px, y, pw, ph, 8.0, t.card, Some(t.border));
        for (a, b, k) in zones[i] {
            let (fill, label) = match k {
                'p' => ("#3584e4", "‹"),
                'n' => ("#26a269", "›"),
                _ => ("#9a9996", "bars"),
            };
            s.rect_a(
                px + pw * a + 1.0,
                y + 1.0,
                pw * (b - a) - 2.0,
                ph - 2.0,
                6.0,
                fill,
                0.28,
            );
            s.text(
                px + pw * (a + b) / 2.0,
                y + ph / 2.0 + 8.0,
                if *k == 'b' { 12.0 } else { 26.0 },
                true,
                t.fg,
                "middle",
                label,
            );
        }
        if zones[i].is_empty() {
            s.text(
                px + pw / 2.0,
                y + ph / 2.0 + 5.0,
                12.0,
                false,
                t.dim,
                "middle",
                "swipe · keys",
            );
        }
        s.text(
            px + pw / 2.0,
            y + ph + 22.0,
            12.0,
            false,
            "#2e3436",
            "middle",
            name,
        );
    }
}

pub fn run(s: &mut Svg) {
    let t = &LIGHT;
    s.text(90.0, 130.0, 74.0, true, "#2e3436", "start", "Bookx Dialogs");
    s.lines(
        90.0,
        180.0,
        17.0,
        24.0,
        "#5e5c64",
        &[
            "The windows behind the main menu and the book dialogs: reading statistics, sharing, vocabulary flashcards, Edit Metadata with Fetch Metadata, and Preferences.",
            "All are libadwaita dialogs (rounded card, title in the header, close button at the right) that sit on top of the library or the reader.",
        ],
    );

    // ---------------------------------------------------------------- 3.a .. 3.b
    label(s, 90.0, 300.0, "3.a Reading Statistics");
    stats(s, 90.0, 330.0, t);
    note(
        s,
        90.0,
        1170.0,
        &[
            "Time read today, in the last 7 and 30 days and overall, the reading streak,",
            "books finished this year and the measured reading speed. The chart shows",
            "the last seven days (today on the right); Most read lists the top books.",
        ],
    );

    label(s, 660.0, 300.0, "3.b Share Library");
    share(s, 660.0, 330.0, t);
    note(
        s,
        660.0,
        960.0,
        &[
            "Serves the library as an OPDS catalogue and a web page. Reachable from",
            "other devices only when “Allow other devices on my network” is on, and",
            "then always behind a password (user name “bookx”). KOReader can also keep",
            "its reading position here.",
        ],
    );

    // ---------------------------------------------------------------- 3.c .. 3.e vocabulary
    label(s, 90.0, 1260.0, "3.c Vocabulary — flashcard, question");
    vocab(s, 90.0, 1290.0, 440.0, 0, t);
    label(s, 660.0, 1260.0, "3.d Vocabulary — answer, grade yourself");
    vocab(s, 660.0, 1290.0, 520.0, 1, t);
    label(s, 90.0, 1790.0, "3.e Vocabulary — done for today");
    vocab(s, 90.0, 1820.0, 330.0, 2, t);
    note(
        s,
        660.0,
        1855.0,
        &[
            "Words you look up in the reader’s dictionary are collected here",
            "with the sentence they came from. Again · Hard · Good · Easy",
            "schedule the next review (spaced repetition). When the list is",
            "empty the card explains how to collect words.",
        ],
    );

    // ---------------------------------------------------------------- 3.j Duplicates
    label(s, 660.0, 1990.0, "3.j Find Duplicates (main menu)");
    duplicates(s, 660.0, 2020.0, t);
    note(
        s,
        660.0,
        2650.0,
        &[
            "Books with the same title and author, or the same ISBN, are grouped. When there are none, a status page says so.",
        ],
    );

    // ---------------------------------------------------------------- 3.f Preferences
    label(
        s,
        1230.0,
        300.0,
        "3.f Preferences — a sidebar of sections, a search over all of them",
    );
    {
        let (x, y) = (1230.0, 330.0);
        let (w, h) = (860.0, 1000.0);
        let side = 250.0;
        let top = adw_dialog(s, x + side, y, w - side, h, "Reading", t);
        // the sidebar: its own column with the window title, the search entry and the sections
        s.rect(x, y, side, h, 18.0, t.panel, Some(t.border));
        s.rect(x + side - 20.0, y, 20.0, h, 0.0, t.panel, None);
        s.line(x + side, y, x + side, y + h, t.border, 1.0);
        s.text(
            x + side / 2.0,
            y + 33.0,
            16.0,
            true,
            t.fg,
            "middle",
            "Preferences",
        );
        s.rect(x + 12.0, y + 56.0, side - 24.0, 36.0, 8.0, t.hover, None);
        icon_search(s, x + 32.0, y + 74.0, t.dim);
        s.text(
            x + 50.0,
            y + 79.0,
            14.0,
            false,
            t.dim,
            "start",
            "Search settings",
        );
        let sections = [
            "General",
            "Library",
            "Reading",
            "Devices and Sharing",
            "E-mail",
            "Sync",
            "Backup",
            "Extensions",
            "Patches",
            "Text to Speech",
        ];
        for (i, name) in sections.iter().enumerate() {
            let ry = y + 108.0 + i as f64 * 44.0;
            if *name == "Reading" {
                s.rect(x + 8.0, ry, side - 16.0, 38.0, 8.0, t.hover, None);
            }
            icon_dot(s, x + 30.0, ry + 19.0, t.dim);
            s.text(x + 50.0, ry + 25.0, 14.0, false, t.fg, "start", name);
        }
        let gx = x + side + 30.0;
        let gw = w - side - 60.0;
        pref_group(
            s,
            gx,
            top + 12.0,
            gw,
            Some("Reader defaults"),
            &["These can also be changed while reading from the Appearance menu"],
            &[
                Row::Combo("Theme", None, "Sepia"),
                Row::Combo("Layout", None, "Two columns"),
                Row::Spin("Font size (pt)", "14"),
                Row::Switch("Justify text", None, true),
                Row::Switch(
                    "Hyphenate words",
                    Some("Break long words at line ends, in the language of the book"),
                    true,
                ),
                Row::Switch(
                    "Use the book's own styling",
                    Some(
                        "Italic and bold classes, alignment, hidden text. Takes effect when a chapter loads.",
                    ),
                    true,
                ),
                Row::Switch(
                    "E-ink mode",
                    Some("Black on white, no animations, bars that stay, wide tap zones"),
                    false,
                ),
                Row::Combo(
                    "Tap zones",
                    Some("Which part of the page turns it when tapped or clicked"),
                    "Edges (narrow strips)",
                ),
                Row::Combo("Large touch controls", None, "Automatic (touch screens)"),
            ],
            t,
        );
    }
    note(
        s,
        1230.0,
        1370.0,
        &[
            "Typing in the search entry (Ctrl+F) filters the settings of every section and the sidebar lists the sections that match.",
            "In a narrow window the sidebar becomes an overlay that a button in the header opens.",
            "New reader options: Hyphenate words, Use the book's own styling (the book’s CSS classes for italic, bold,",
            "alignment and hidden text), E-ink mode, Tap zones and Large touch controls. Turning on E-ink mode also",
            "switches to wide tap zones, hyphenation and justified text (see the e-ink sheet).",
        ],
    );

    label(
        s,
        1230.0,
        1530.0,
        "3.g Tap zones — the choices of the Tap zones row",
    );
    tap_zones(s, 1230.0, 1560.0, t);
    note(
        s,
        1230.0,
        1810.0,
        &[
            "‹ previous page   › next page   bars: show or hide the header and the navigation row.",
            "Wide is for e-ink: no bar toggle, so a tap never flashes the screen for nothing.",
        ],
    );

    // ---------------------------------------------------------------- 3.h Edit Metadata
    label(
        s,
        2130.0,
        300.0,
        "3.h Edit Metadata — with Fetch Metadata and its status line",
    );
    library::edit_metadata(s, 2130.0, 330.0, t, true);
    note(
        s,
        2130.0,
        1670.0,
        &[
            "Fetch Metadata looks the book up on Open Library, Google Books and Wikidata and fills",
            "the empty fields; tick “Replace existing values” to overwrite filled ones too.",
            "The status line says where it was found, how good the match is and what changed",
            "(“Looking the book up…” while it works, “No match found…” when nothing fits).",
        ],
    );

    label(s, 2130.0, 1830.0, "3.i Bulk edit (several books)");
    library::bulk_edit(s, 2130.0, 1860.0, t);

    s.text(
        1600.0,
        2720.0,
        12.0,
        false,
        "#77767b",
        "middle",
        "Mockups are design illustrations. Book titles are used for illustration purposes only.",
    );
}
