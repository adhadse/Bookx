use crate::kit::*;

pub struct Book {
    pub title: &'static str,
    pub author: &'static str,
    pub series: &'static str,
    pub rating: usize,
    pub progress: f64,
    pub published: &'static str,
    pub size: &'static str,
    pub tags: &'static str,
}

pub const BOOKS: [Book; 12] = [
    Book {
        title: "Moby-Dick",
        author: "Herman Melville",
        series: "",
        rating: 4,
        progress: 0.05,
        published: "1851-10-18",
        size: "1.2 MB",
        tags: "classics, fiction",
    },
    Book {
        title: "Designing Data-Intensive Applications",
        author: "Martin Kleppmann",
        series: "",
        rating: 5,
        progress: 0.18,
        published: "2017-03-16",
        size: "11.2 MB",
        tags: "programming, databases",
    },
    Book {
        title: "Atomic Habits",
        author: "James Clear",
        series: "",
        rating: 4,
        progress: 1.0,
        published: "2018-10-16",
        size: "5.7 MB",
        tags: "self-help",
    },
    Book {
        title: "Dune",
        author: "Frank Herbert",
        series: "Dune [1]",
        rating: 5,
        progress: 0.0,
        published: "1965-08-01",
        size: "1.4 MB",
        tags: "sci-fi, classics",
    },
    Book {
        title: "The Left Hand of Darkness",
        author: "Ursula K. Le Guin",
        series: "Hainish [4]",
        rating: 4,
        progress: 0.0,
        published: "1969-03-01",
        size: "0.9 MB",
        tags: "sci-fi",
    },
    Book {
        title: "A Wizard of Earthsea",
        author: "Ursula K. Le Guin",
        series: "Earthsea [1]",
        rating: 4,
        progress: 0.34,
        published: "1968-11-01",
        size: "0.7 MB",
        tags: "fantasy",
    },
    Book {
        title: "Project Hail Mary",
        author: "Andy Weir",
        series: "",
        rating: 5,
        progress: 1.0,
        published: "2021-05-04",
        size: "2.2 MB",
        tags: "sci-fi",
    },
    Book {
        title: "Piranesi",
        author: "Susanna Clarke",
        series: "",
        rating: 0,
        progress: 0.0,
        published: "2020-09-15",
        size: "1.1 MB",
        tags: "fantasy",
    },
    Book {
        title: "Staff Engineer",
        author: "Will Larson",
        series: "",
        rating: 3,
        progress: 0.07,
        published: "2021-07-01",
        size: "4.0 MB",
        tags: "career",
    },
    Book {
        title: "The Pragmatic Programmer",
        author: "Hunt & Thomas",
        series: "",
        rating: 5,
        progress: 0.0,
        published: "2019-09-13",
        size: "6.3 MB",
        tags: "programming",
    },
    Book {
        title: "Gödel, Escher, Bach",
        author: "Douglas Hofstadter",
        series: "",
        rating: 0,
        progress: 0.0,
        published: "1979-02-05",
        size: "9.8 MB",
        tags: "maths, philosophy",
    },
    Book {
        title: "Thinking, Fast and Slow",
        author: "Daniel Kahneman",
        series: "",
        rating: 4,
        progress: 0.0,
        published: "2011-10-25",
        size: "4.4 MB",
        tags: "psychology",
    },
];

pub struct Opts {
    pub table: bool,
    /// The left sidebar (always shown on a wide window).
    pub sidebar: bool,
    pub details: Option<usize>,
    pub selected: Vec<usize>,
    pub search: &'static str,
    /// The search bar below the header is open.
    pub search_open: bool,
    pub bottom_bar: bool,
    pub banner: bool,
    pub filter_to: Option<Vec<usize>>,
    /// Header subtitle: the number of books.
    pub subtitle: &'static str,
    /// Toggle highlighted in the header (the main menu button is pressed).
    pub menu_open: bool,
    /// A device is connected: its name on the header-bar Device button.
    pub device: Option<&'static str>,
    /// The "device connected" banner under the header.
    pub device_banner: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            table: false,
            sidebar: true,
            details: None,
            selected: vec![],
            search: "",
            search_open: false,
            bottom_bar: false,
            banner: true,
            filter_to: None,
            subtitle: "12 books",
            menu_open: false,
            device: None,
            device_banner: false,
        }
    }
}

/// The header: add, title with the book count, search, menu (nothing else: progressive disclosure).
fn header(s: &mut Svg, x: f64, y: f64, w: f64, o: &Opts, t: &Theme) {
    if o.sidebar {
        // like Files: the sidebar has its own header bar (add, library, search, menu) and the
        // content its own (the name of the view, view options); the separator runs up through both
        let sw = 230.0;
        s.rect(x, y, sw, 48.0, 0.0, t.panel, None);
        s.line(x + sw, y, x + sw, y + 48.0, t.border, 1.0);
        icon_plus(s, x + 28.0, y + 25.0, t.fg);
        s.text(x + 100.0, y + 29.0, 13.0, true, t.fg, "middle", "lib  ▾");
        icon_search(s, x + 164.0, y + 24.0, t.fg);
        icon_menu(s, x + 202.0, y + 24.0, t.fg);
        let cw = w - sw;
        s.text(
            x + sw + cw / 2.0,
            y + 21.0,
            14.0,
            true,
            t.fg,
            "middle",
            "Home",
        );
        s.text(
            x + sw + cw / 2.0,
            y + 38.0,
            11.0,
            false,
            t.dim,
            "middle",
            o.subtitle,
        );
        icon_sidebar(s, x + w - 100.0, y + 24.0, t.fg, false);
        if let Some(name) = o.device {
            let pw = 44.0 + name.chars().count() as f64 * 7.4;
            let px = x + w - 120.0 - pw;
            s.rect(px, y + 9.0, pw, 30.0, 8.0, t.hover, None);
            s.rect(px + 10.0, y + 17.0, 12.0, 14.0, 2.0, "none", Some(t.fg));
            s.text(px + 30.0, y + 29.0, 12.0, false, t.fg, "start", name);
        }
        return;
    }
    s.rect(x + 12.0, y + 10.0, 34.0, 30.0, 8.0, t.hover, None);
    icon_plus(s, x + 29.0, y + 25.0, t.fg);
    // the two panel toggles stay in the header bar whether the panels are open or not
    s.rect(
        x + 52.0,
        y + 10.0,
        34.0,
        30.0,
        8.0,
        if o.sidebar { t.hover } else { "none" },
        None,
    );
    icon_sidebar(s, x + 69.0, y + 25.0, t.fg, false);
    s.text(x + w / 2.0, y + 21.0, 14.0, true, t.fg, "middle", "Bookx");
    s.text(
        x + w / 2.0,
        y + 38.0,
        11.0,
        false,
        t.dim,
        "middle",
        o.subtitle,
    );
    s.rect(
        x + w - 114.0,
        y + 9.0,
        34.0,
        30.0,
        8.0,
        if o.search_open { t.hover } else { "none" },
        None,
    );
    icon_search(s, x + w - 97.0, y + 24.0, t.fg);
    s.rect(
        x + w - 76.0,
        y + 9.0,
        34.0,
        30.0,
        8.0,
        if o.menu_open { t.hover } else { "none" },
        None,
    );
    icon_menu(s, x + w - 59.0, y + 24.0, t.fg);
    s.rect(
        x + w - 152.0,
        y + 9.0,
        34.0,
        30.0,
        8.0,
        if o.details.is_some() { t.hover } else { "none" },
        None,
    );
    icon_sidebar(s, x + w - 135.0, y + 24.0, t.fg, true);
    if let Some(name) = o.device {
        // appears only while a device is connected
        let pw = 44.0 + name.chars().count() as f64 * 7.4;
        let px = x + w - 164.0 - pw;
        s.rect(px, y + 9.0, pw, 30.0, 8.0, t.hover, None);
        s.rect(px + 10.0, y + 17.0, 12.0, 14.0, 2.0, "none", Some(t.fg));
        s.text(px + 30.0, y + 29.0, 12.0, false, t.fg, "start", name);
    }
}

/// The banner that asks what to do when a device is connected.
fn device_banner(s: &mut Svg, x: f64, y: f64, w: f64, name: &str, t: &Theme) {
    s.rect_a(x, y, w, 46.0, 0.0, t.accent, 0.16);
    s.line(x, y + 46.0, x + w, y + 46.0, t.accent, 1.0);
    s.text(
        x + 16.0,
        y + 29.0,
        13.0,
        false,
        t.fg,
        "start",
        &format!("{name} connected — Sync reading progress and notes?"),
    );
    btn(
        s,
        x + w - 258.0,
        y + 8.0,
        62.0,
        30.0,
        "Sync",
        "suggested",
        t,
    );
    btn(s, x + w - 188.0, y + 8.0, 66.0, 30.0, "Always", "normal", t);
    btn(
        s,
        x + w - 114.0,
        y + 8.0,
        98.0,
        30.0,
        "Not now",
        "normal",
        t,
    );
}

/// The search bar under the header, with the Calibre-style placeholder.
fn search_bar(s: &mut Svg, x: f64, y: f64, w: f64, o: &Opts, t: &Theme) {
    s.rect(x, y, w, 56.0, 0.0, t.header, None);
    s.line(x, y + 56.0, x + w, y + 56.0, t.border, 1.0);
    let sw = (w * 0.6).min(620.0);
    let sx = x + w / 2.0 - sw / 2.0;
    s.rect(sx, y + 10.0, sw, 36.0, 9.0, t.hover, None);
    icon_search(s, sx + 22.0, y + 28.0, t.dim);
    if o.search.is_empty() {
        s.text(
            sx + 42.0,
            y + 34.0,
            13.0,
            false,
            t.dim,
            "start",
            "Search books — or try author:le guin tag:sci-fi text:\"inside the book\"",
        );
    } else {
        s.text(sx + 42.0, y + 34.0, 14.0, false, t.fg, "start", o.search);
        icon_close(s, sx + sw - 20.0, y + 28.0, t.dim);
    }
}

/// The yellow "Continue Reading" card: text on the left, the cover on the right.
fn continue_card(s: &mut Svg, x: f64, y: f64, w: f64, b: &Book, idx: usize) -> f64 {
    let h = 150.0;
    s.shadow(x + 6.0, y + 4.0, w - 12.0, h - 8.0, 18.0, 0.18);
    s.rect(x, y, w, h, 18.0, "#f6d34a", None);
    let ink = "#2e3436";
    s.text(
        x + 28.0,
        y + 36.0,
        11.0,
        true,
        "#2e3436b3",
        "start",
        "CONTINUE READING  →",
    );
    s.text(x + 28.0, y + 68.0, 24.0, true, ink, "start", b.title);
    s.text(
        x + 28.0,
        y + 90.0,
        14.0,
        false,
        "#2e3436c0",
        "start",
        b.author,
    );
    s.text(
        x + 28.0,
        y + 116.0,
        12.0,
        false,
        "#2e3436b3",
        "start",
        &format!("{:.0}% completed", b.progress * 100.0),
    );
    s.rect_a(x + 28.0, y + 126.0, 160.0, 3.0, 1.5, ink, 0.2);
    s.rect(
        x + 28.0,
        y + 126.0,
        160.0 * b.progress.max(0.04),
        3.0,
        1.5,
        ink,
        None,
    );
    cover(
        s,
        x + w - 28.0 - 80.0,
        y + 18.0,
        80.0,
        114.0,
        idx,
        b.title,
        "",
        None,
        false,
    );
    h
}

fn tag_browser(s: &mut Svg, x: f64, y: f64, w: f64, h: f64, t: &Theme, open: &str) {
    s.rect(x, y, w, h, 0.0, t.panel, None);
    s.line(x + w, y, x + w, y + h, t.border, 1.0);
    // Home, Recent, Favourites, Notes & Highlights, then the groups, Library Health at the bottom
    let mut yy = y + 12.0;
    let views: [(&str, &str); 4] = [
        ("Home", "12"),
        ("Recent", "12"),
        ("Favourites", "2"),
        ("Notes & Highlights", "41"),
    ];
    for (name, count) in views {
        if name == "Home" {
            s.rect(x + 8.0, yy - 4.0, w - 16.0, 36.0, 8.0, t.hover, None);
        }
        s.rect(x + 18.0, yy + 6.0, 14.0, 14.0, 3.0, t.dim, None);
        s.text(
            x + 44.0,
            yy + 19.0,
            14.0,
            name == "Home",
            t.fg,
            "start",
            name,
        );
        s.text(x + w - 20.0, yy + 19.0, 13.0, false, t.dim, "end", count);
        yy += 38.0;
    }
    s.line(x, yy + 4.0, x + w, yy + 4.0, t.border, 1.0);
    yy += 16.0;
    s.rect(x + 18.0, y + h - 36.0, 14.0, 14.0, 3.0, t.dim, None);
    s.text(
        x + 44.0,
        y + h - 24.0,
        14.0,
        false,
        t.fg,
        "start",
        "Library Health",
    );
    s.line(x, y + h - 48.0, x + w, y + h - 48.0, t.border, 1.0);
    let rows: [(&str, &str, &str); 7] = [
        ("Shelves", "3", "L"),
        ("Authors", "10", "A"),
        ("Series", "3", "S"),
        ("Tags", "14", "T"),
        ("Publishers", "7", "P"),
        ("Languages", "2", "G"),
        ("Reading status", "3", "R"),
    ];
    for (name, count, _) in rows {
        s.rect(x + 18.0, yy + 6.0, 14.0, 14.0, 3.0, t.dim, None);
        s.text(x + 44.0, yy + 19.0, 14.0, false, t.fg, "start", name);
        s.text(x + w - 40.0, yy + 19.0, 13.0, false, t.dim, "end", count);
        if name != "Shelves" {
            icon_chev(
                s,
                x + w - 22.0,
                yy + 14.0,
                t.dim,
                if name == open { 'u' } else { 'd' },
            );
        }
        yy += 40.0;
        if name == open {
            let items: &[(&str, &str)] = match open {
                "Tags" => &[
                    ("fantasy", "2"),
                    ("programming", "3"),
                    ("sci-fi", "3"),
                    ("self-help", "1"),
                    ("career", "1"),
                ],
                _ => &[
                    ("Ursula K. Le Guin", "2"),
                    ("James Clear", "1"),
                    ("Frank Herbert", "1"),
                    ("Martin Kleppmann", "1"),
                ],
            };
            for (i, (n, c)) in items.iter().enumerate() {
                if i == 2 {
                    s.rect(x + 8.0, yy - 3.0, w - 16.0, 30.0, 8.0, t.accent, None);
                    s.text(x + 44.0, yy + 17.0, 13.0, false, "#fff", "start", n);
                    s.text(x + w - 24.0, yy + 17.0, 12.0, false, "#ffffffcc", "end", c);
                } else {
                    s.text(x + 44.0, yy + 17.0, 13.0, false, t.fg, "start", n);
                    s.text(x + w - 24.0, yy + 17.0, 12.0, false, t.dim, "end", c);
                }
                yy += 30.0;
            }
            yy += 6.0;
        }
    }
}

fn details(s: &mut Svg, x: f64, y: f64, w: f64, h: f64, t: &Theme, idx: usize) {
    let b = &BOOKS[idx];
    s.rect(x, y, w, h, 0.0, t.panel, None);
    s.line(x, y, x, y + h, t.border, 1.0);
    s.text(x + w / 2.0, y + 30.0, 14.0, true, t.fg, "middle", "Details");
    cover(
        s,
        x + w / 2.0 - 70.0,
        y + 56.0,
        140.0,
        210.0,
        idx,
        b.title,
        b.author,
        None,
        false,
    );
    s.text(
        x + w / 2.0,
        y + 304.0,
        19.0,
        true,
        t.fg,
        "middle",
        b.title.split(',').next().unwrap_or(b.title),
    );
    s.text(
        x + w / 2.0,
        y + 328.0,
        14.0,
        false,
        t.dim,
        "middle",
        b.author,
    );
    if !b.series.is_empty() {
        s.text(
            x + w / 2.0,
            y + 348.0,
            12.0,
            false,
            t.fg,
            "middle",
            b.series,
        );
    }
    stars(s, x + w / 2.0 - 56.0, y + 360.0, b.rating, 18.0);
    btn(
        s,
        x + 24.0,
        y + 398.0,
        124.0,
        38.0,
        "Continue",
        "suggested",
        t,
    );
    btn(
        s,
        x + 160.0,
        y + 398.0,
        w - 184.0,
        38.0,
        "Edit Metadata",
        "normal",
        t,
    );
    let rows: [(&str, String); 5] = [
        ("Format", format!("EPUB · {}", b.size)),
        ("Publisher", "Penguin Publishing Group".into()),
        ("Published", b.published.into()),
        ("Tags", b.tags.into()),
        ("Identifiers", "isbn:9780735211292".into()),
    ];
    let mut yy = y + 456.0;
    s.rect(
        x + 18.0,
        yy,
        w - 36.0,
        5.0 * 52.0,
        10.0,
        t.card,
        Some(t.border),
    );
    for (k, v) in rows {
        s.text(x + 34.0, yy + 20.0, 11.0, false, t.dim, "start", k);
        s.text(x + 34.0, yy + 40.0, 14.0, false, t.fg, "start", &v);
        yy += 52.0;
        if yy < y + 456.0 + 5.0 * 52.0 {
            s.line(x + 18.0, yy, x + w - 18.0, yy, t.border, 1.0);
        }
    }
    s.text(x + 24.0, yy + 30.0, 14.0, true, t.fg, "start", "Comments");
    greek(s, x + 24.0, yy + 44.0, w - 48.0, 4, 18.0, t.border, idx);
}

pub fn lib_window(s: &mut Svg, x: f64, y: f64, w: f64, h: f64, t: &Theme, o: &Opts) {
    let top = window(s, x, y, w, h, t, true);
    header(s, x, y, w, o, t);
    let top = if o.search_open {
        search_bar(s, x, top, w, o, t);
        top + 56.0
    } else {
        top
    };
    let top = if o.device_banner {
        device_banner(s, x, top, w, o.device.unwrap_or("Device"), t);
        top + 46.0
    } else {
        top
    };
    let side_w = if o.sidebar { 230.0 } else { 0.0 };
    let det_w = if o.details.is_some() { 330.0 } else { 0.0 };
    let bottom = if o.bottom_bar { 54.0 } else { 0.0 };
    let body_h = y + h - bottom - top;
    if o.sidebar {
        tag_browser(s, x, top, side_w, body_h, t, "Tags");
    }
    if let Some(i) = o.details {
        details(s, x + w - det_w, top, det_w, body_h, t, i);
    }
    let cx = x + side_w;
    let cw = w - side_w - det_w;

    let idxs: Vec<usize> = o
        .filter_to
        .clone()
        .unwrap_or_else(|| (0..BOOKS.len()).collect());

    if o.table {
        let cols: [(&str, f64); 7] = [
            ("Title", 0.30),
            ("Author(s)", 0.17),
            ("Series", 0.12),
            ("Rating", 0.12),
            ("Read", 0.07),
            ("Published", 0.11),
            ("Size", 0.08),
        ];
        s.rect(cx, top, cw, 36.0, 0.0, t.panel, None);
        let mut xx = cx + 14.0;
        for (i, (n, f)) in cols.iter().enumerate() {
            s.text(xx, top + 24.0, 12.0, true, t.dim, "start", n);
            if i == 0 {
                icon_chev(s, xx + 38.0, top + 18.0, t.dim, 'u');
            }
            xx += cw * f;
            s.line(xx - 8.0, top + 8.0, xx - 8.0, top + 28.0, t.border, 1.0);
        }
        s.line(cx, top + 36.0, cx + cw, top + 36.0, t.border, 1.0);
        let mut yy = top + 36.0;
        for &i in &idxs {
            let b = &BOOKS[i];
            if yy + 40.0 > top + body_h {
                break;
            }
            let sel = o.selected.contains(&i);
            if sel {
                s.rect_a(cx, yy, cw, 40.0, 0.0, t.accent, 0.22);
            }
            s.line(cx, yy + 40.0, cx + cw, yy + 40.0, t.border, 1.0);
            let mut xx = cx + 14.0;
            let vals: [String; 7] = [
                b.title.into(),
                b.author.into(),
                b.series.into(),
                if b.rating > 0 {
                    "★".repeat(b.rating) + &"☆".repeat(5 - b.rating)
                } else {
                    String::new()
                },
                format!("{:.0}%", b.progress * 100.0),
                b.published.into(),
                b.size.into(),
            ];
            for (k, (_, f)) in cols.iter().enumerate() {
                let fill = if k == 3 { "#e5a50a" } else { t.fg };
                let maxc = (cw * f / 7.4) as usize;
                let v: String = if vals[k].chars().count() > maxc {
                    vals[k]
                        .chars()
                        .take(maxc.saturating_sub(1))
                        .collect::<String>()
                        + "…"
                } else {
                    vals[k].clone()
                };
                s.text(xx, yy + 25.0, 13.0, k == 0, fill, "start", &v);
                xx += cw * f;
            }
            yy += 40.0;
        }
    } else {
        let mut yy = top + 16.0;
        if o.banner {
            let bw = (cw - 48.0).min(900.0);
            let bx = cx + (cw - bw) / 2.0;
            yy += continue_card(s, bx, yy, bw, &BOOKS[0], 0) + 28.0;
        }
        let cwid = 142.0;
        let gap = 26.0;
        let cols = (((cw - 32.0) + gap) / (cwid + gap)).floor().max(1.0) as usize;
        let total = cols as f64 * (cwid + gap) - gap;
        let x0 = cx + (cw - total) / 2.0;
        for (n, &i) in idxs.iter().enumerate() {
            let b = &BOOKS[i];
            let col = n % cols;
            let row = n / cols;
            let bx = x0 + col as f64 * (cwid + gap);
            let by = yy + row as f64 * 304.0;
            if by + 290.0 > top + body_h {
                break;
            }
            if o.selected.contains(&i) {
                s.rect_a(
                    bx - 10.0,
                    by - 10.0,
                    cwid + 20.0,
                    300.0,
                    12.0,
                    t.accent,
                    0.22,
                );
            }
            cover(
                s,
                bx,
                by,
                cwid,
                213.0,
                i,
                b.title,
                "",
                None,
                b.progress >= 0.98,
            );
            if b.progress > 0.0 && b.progress < 0.98 {
                // reading progress sits under the cover: "5%  ▬▬▬▬"
                s.text(
                    bx + 2.0,
                    by + 236.0,
                    11.0,
                    false,
                    t.dim,
                    "start",
                    &format!("{:.0}%", b.progress * 100.0),
                );
                s.rect_a(bx + 36.0, by + 231.0, cwid - 36.0, 4.0, 2.0, t.fg, 0.15);
                s.rect(
                    bx + 36.0,
                    by + 231.0,
                    (cwid - 36.0) * b.progress,
                    4.0,
                    2.0,
                    t.accent,
                    None,
                );
            }
            let tt: String = if b.title.chars().count() > 20 {
                b.title.chars().take(19).collect::<String>() + "…"
            } else {
                b.title.into()
            };
            s.text(bx + cwid / 2.0, by + 262.0, 13.0, true, t.fg, "middle", &tt);
            s.text(
                bx + cwid / 2.0,
                by + 280.0,
                11.0,
                false,
                t.dim,
                "middle",
                b.author,
            );
        }
    }
    if o.bottom_bar {
        let by = y + h - bottom;
        s.rect(x, by, w, bottom, 0.0, t.header, None);
        s.line(x, by, x + w, by, t.border, 1.0);
        s.text(
            x + 20.0,
            by + 33.0,
            14.0,
            true,
            t.fg,
            "start",
            &format!("{} selected", o.selected.len()),
        );
        btn(
            s,
            x + w - 470.0,
            by + 8.0,
            100.0,
            38.0,
            "Read",
            "suggested",
            t,
        );
        btn(
            s,
            x + w - 360.0,
            by + 8.0,
            150.0,
            38.0,
            "Edit Metadata",
            "normal",
            t,
        );
        btn(
            s,
            x + w - 200.0,
            by + 8.0,
            100.0,
            38.0,
            "Remove",
            "destructive",
            t,
        );
        icon_close(s, x + w - 50.0, by + 27.0, t.dim);
    }
}

pub fn dialog_frame(
    s: &mut Svg,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    title: &str,
    left: &str,
    right: &str,
    t: &Theme,
) {
    s.shadow(x, y, w, h, 18.0, 0.35);
    s.rect(x, y, w, h, 18.0, t.bg, Some(t.border));
    s.line(x, y + 54.0, x + w, y + 54.0, t.border, 1.0);
    btn(s, x + 14.0, y + 9.0, 92.0, 36.0, left, "normal", t);
    btn(s, x + w - 106.0, y + 9.0, 92.0, 36.0, right, "suggested", t);
    s.text(x + w / 2.0, y + 33.0, 16.0, true, t.fg, "middle", title);
}

pub fn field(s: &mut Svg, x: f64, y: f64, w: f64, title: &str, val: &str, t: &Theme) {
    s.rect(x, y, w, 56.0, 0.0, t.card, None);
    s.line(x, y + 56.0, x + w, y + 56.0, t.border, 1.0);
    s.text(x + 16.0, y + 22.0, 11.0, false, t.dim, "start", title);
    s.text(
        x + 16.0,
        y + 42.0,
        14.0,
        false,
        if val.is_empty() { t.dim } else { t.fg },
        "start",
        val,
    );
    icon_pencil(s, x + w - 28.0, y + 28.0, t.dim);
}

/// Edit Metadata: cover and *Fetch Metadata* on top, a status line with the outcome of the
/// look-up, then the Calibre fields. `fetched` shows the state after a successful look-up.
pub fn edit_metadata(s: &mut Svg, x: f64, y: f64, t: &Theme, fetched: bool) -> f64 {
    let (w, h) = (780.0, 1300.0);
    dialog_frame(s, x, y, w, h, "Edit Metadata", "Cancel", "Save", t);
    cover(
        s,
        x + 120.0,
        y + 82.0,
        130.0,
        195.0,
        3,
        "Dune",
        "Frank Herbert",
        None,
        false,
    );
    let bx = x + 290.0;
    btn(
        s,
        bx,
        y + 84.0,
        300.0,
        38.0,
        "Fetch Metadata",
        "suggested",
        t,
    );
    s.rect(bx, y + 136.0, 18.0, 18.0, 4.0, "none", Some(t.dim));
    s.text(
        bx + 28.0,
        y + 151.0,
        14.0,
        false,
        t.fg,
        "start",
        "Replace existing values",
    );
    btn(s, bx, y + 170.0, 300.0, 38.0, "Choose Cover", "normal", t);
    btn(
        s,
        bx,
        y + 220.0,
        300.0,
        38.0,
        "Remove Cover",
        "destructive",
        t,
    );
    s.text(
        bx,
        y + 276.0,
        12.0,
        false,
        t.dim,
        "start",
        "Looks the book up on Open Library, Google Books and Wikidata",
    );
    // status line (caption, dim)
    let st = if fetched {
        [
            "Found on Open Library, Wikidata (92% match). Updated: publisher, published,",
            "identifiers, tags. Cover added.",
        ]
    } else {
        ["Looking the book up…", ""]
    };
    let gx = x + 40.0;
    let gw = w - 80.0;
    s.rect_a(gx, y + 296.0, gw, 50.0, 10.0, t.accent, 0.1);
    s.text(gx + 16.0, y + 318.0, 12.0, false, t.dim, "start", st[0]);
    s.text(gx + 16.0, y + 336.0, 12.0, false, t.dim, "start", st[1]);

    s.text(gx, y + 388.0, 15.0, true, t.fg, "start", "Basics");
    let mut yy = y + 400.0;
    for (k, v) in [
        ("Title", "Dune"),
        ("Authors (separate with &)", "Frank Herbert"),
        ("Series", "Dune"),
    ] {
        field(s, gx, yy, gw, k, v, t);
        yy += 56.0;
    }
    s.rect(gx, yy, gw, 56.0, 0.0, t.card, None);
    s.line(gx, yy + 56.0, gx + gw, yy + 56.0, t.border, 1.0);
    s.text(
        gx + 16.0,
        yy + 34.0,
        14.0,
        false,
        t.fg,
        "start",
        "Series number",
    );
    s.text(gx + gw - 110.0, yy + 34.0, 14.0, false, t.fg, "end", "1.0");
    s.circle(gx + gw - 82.0, yy + 28.0, 15.0, t.hover);
    s.line(
        gx + gw - 89.0,
        yy + 28.0,
        gx + gw - 75.0,
        yy + 28.0,
        t.fg,
        2.0,
    );
    s.circle(gx + gw - 40.0, yy + 28.0, 15.0, t.hover);
    icon_plus(s, gx + gw - 40.0, yy + 28.0, t.fg);
    yy += 56.0;
    s.rect(gx, yy, gw, 56.0, 0.0, t.card, None);
    s.text(gx + 16.0, yy + 34.0, 14.0, false, t.fg, "start", "Rating");
    stars(s, gx + gw - 170.0, yy + 16.0, 5, 24.0);
    yy += 76.0;
    s.text(gx, yy, 15.0, true, t.fg, "start", "Details");
    yy += 12.0;
    for (k, v) in [
        ("Tags (comma separated)", "sci-fi, classics"),
        ("Publisher", "Chilton Books"),
        ("Published (YYYY-MM-DD)", "1965-08-01"),
        ("Language", "en"),
        ("Identifiers (isbn:…, doi:…)", "isbn:9780441172719"),
    ] {
        field(s, gx, yy, gw, k, v, t);
        yy += 56.0;
    }
    yy += 20.0;
    s.text(gx, yy, 15.0, true, t.fg, "start", "Sorting");
    s.text(
        gx + 70.0,
        yy,
        12.0,
        false,
        t.dim,
        "start",
        "Leave empty to generate automatically",
    );
    yy += 12.0;
    field(s, gx, yy, gw, "Title sort", "", t);
    field(s, gx, yy + 56.0, gw, "Author sort", "Herbert, Frank", t);
    yy += 56.0 * 2.0 + 28.0;
    s.text(gx, yy, 15.0, true, t.fg, "start", "Comments");
    s.rect(gx, yy + 10.0, gw, 100.0, 10.0, t.card, Some(t.border));
    greek(s, gx + 14.0, yy + 26.0, gw - 28.0, 3, 22.0, t.border, 4);
    y + h
}

/// Bulk editor: only the fields that are filled in are changed.
pub fn bulk_edit(s: &mut Svg, x: f64, y: f64, t: &Theme) {
    dialog_frame(s, x, y, 780.0, 520.0, "Edit 3 Books", "Cancel", "Apply", t);
    s.text(
        x + 40.0,
        y + 94.0,
        13.0,
        false,
        t.dim,
        "start",
        "Only the fields you fill in are changed",
    );
    let mut yy = y + 110.0;
    for (k, v) in [
        ("Add tags (comma separated)", "to-read"),
        ("Remove tags (comma separated)", ""),
        ("Set authors", ""),
        ("Set series", "Earthsea"),
    ] {
        field(s, x + 40.0, yy, 700.0, k, v, t);
        yy += 56.0;
    }
    s.rect(x + 40.0, yy, 700.0, 56.0, 0.0, t.card, None);
    s.text(
        x + 56.0,
        yy + 34.0,
        14.0,
        false,
        t.fg,
        "start",
        "Set rating (click to choose)",
    );
    stars(s, x + 560.0, yy + 16.0, 4, 24.0);
}

/// The main-menu popover: view, sort, cover size, the two optional panels, then the pages.
fn main_menu(s: &mut Svg, x: f64, y: f64, t: &Theme) {
    let w = 300.0;
    let h = 672.0;
    popover_card(s, x, y, w, h, x + w - 36.0, true, t);
    // grid / table toggle (linked buttons)
    s.rect(x + 90.0, y + 14.0, 60.0, 34.0, 8.0, t.accent, None);
    icon_grid(s, x + 120.0, y + 31.0, "#ffffff");
    s.rect(x + 152.0, y + 14.0, 60.0, 34.0, 8.0, t.hover, None);
    icon_list(s, x + 182.0, y + 31.0, t.fg);
    // sort by
    s.text(x + 16.0, y + 88.0, 14.0, false, t.fg, "start", "Sort by");
    s.rect(x + 100.0, y + 68.0, 130.0, 34.0, 8.0, t.hover, None);
    s.text(x + 112.0, y + 90.0, 13.0, false, t.fg, "start", "Title");
    icon_chev(s, x + 214.0, y + 84.0, t.fg, 'd');
    s.rect(x + 240.0, y + 68.0, 44.0, 34.0, 8.0, t.hover, None);
    s.path(
        &format!("M{},{} v14 m-5,-9 l5,-5 l5,5", x + 262.0, y + 78.0),
        "none",
        t.fg,
        1.8,
    );
    // cover size
    s.text(
        x + 16.0,
        y + 134.0,
        14.0,
        false,
        t.fg,
        "start",
        "Cover size",
    );
    slider(s, x + 20.0, y + 162.0, w - 40.0, 0.35, t);
    row_sep(s, x + 12.0, y + 190.0, w - 24.0, t);
    let mut yy = y + 202.0;
    for it in [
        "Open Library Folder",
        "Saved Searches",
        "Online Catalogues",
        "Reading Statistics",
        "Vocabulary",
        "Import Calibre Library",
        "Share Library",
        "Library Health",
        "Preferences",
        "Keyboard Shortcuts",
        "About Bookx",
    ] {
        s.text(x + 22.0, yy + 24.0, 14.0, false, t.fg, "start", it);
        yy += 40.0;
    }
}

fn search_cheat_sheet(s: &mut Svg, x: f64, y: f64, t: &Theme) {
    s.shadow(x, y, 1000.0, 140.0, 16.0, 0.2);
    s.rect(x, y, 1000.0, 56.0, 16.0, t.header, Some(t.border));
    s.rect(x + 160.0, y + 10.0, 680.0, 36.0, 10.0, t.hover, None);
    icon_search(s, x + 184.0, y + 28.0, t.dim);
    s.text(
        x + 206.0,
        y + 34.0,
        14.0,
        false,
        t.fg,
        "start",
        "author:\"le guin\" tag:fantasy -rating:<3 OR series:=Dune",
    );
    s.shadow(x + 160.0, y + 66.0, 680.0, 520.0, 14.0, 0.25);
    s.rect(
        x + 160.0,
        y + 66.0,
        680.0,
        520.0,
        14.0,
        t.card,
        Some(t.border),
    );
    s.text(
        x + 184.0,
        y + 102.0,
        16.0,
        true,
        t.fg,
        "start",
        "Search syntax",
    );
    let rows: [(&str, &str); 12] = [
        ("tolkien", "any of title, author, tag, series, publisher"),
        ("title:  author:  tag:  series:", "restrict to one field"),
        ("author:\"le guin\"", "quote phrases"),
        ("tag:=fantasy", "= matches exactly"),
        ("rating:>=4", "> >= < <= for ratings, size, progress"),
        ("status:unread|reading|finished", "reading status"),
        ("language:en  publisher:penguin", "language, publisher"),
        ("identifier:isbn  comments:wizard", "identifiers, comments"),
        (
            "text:\"inside the book\"",
            "search the contents of the books",
        ),
        ("-tag:horror", "exclude"),
        ("a OR b", "either term"),
        ("format:epub  size:>10", "format / size in MB"),
    ];
    let mut yy = y + 136.0;
    for (a, b) in rows {
        s.mono(x + 184.0, yy, 12.5, t.accent, a);
        s.text(x + 500.0, yy, 12.5, false, t.dim, "start", b);
        yy += 32.0;
    }
    s.text(
        x + 184.0,
        yy + 18.0,
        12.0,
        false,
        t.dim,
        "start",
        "Hover the search field for this cheat sheet. The sidebar fills the search for you.",
    );
}

pub fn run(out: &mut Svg) {
    let t = &LIGHT;
    let s = out;
    s.text(90.0, 130.0, 74.0, true, "#2e3436", "start", "Bookx");
    s.lines(90.0, 180.0, 17.0, 24.0, "#5e5c64", &[
        "Library — a Calibre-class library manager, built natively for GNOME with GTK4 + libadwaita, written entirely in Rust.",
        "The first screen is just your books: a yellow Continue Reading card, covers with progress, and a header with add, search and the menu.",
        "A sidebar like Files (Home, Recent, Favourites, Notes, shelves) sits on the left; the details of a selected book slide in on the right.",
    ]);

    // ---------------------------------------------------------------- 1.a
    label(s, 90.0, 300.0, "1.a Library — the clean first screen");
    lib_window(s, 90.0, 320.0, 1440.0, 860.0, t, &Opts::default());
    note(
        s,
        90.0,
        1216.0,
        &[
            "Header: add books (+) · title with the number of books · search · main menu. No panels, no toolbars.",
            "Continue Reading: a yellow card for the book you read last (cover, title, author, % completed). Click it to carry on.",
            "Under each cover: % read and a thin progress bar, then title and author. A check badge marks finished books.",
        ],
    );

    // ---------------------------------------------------------------- 1.b
    label(
        s,
        1660.0,
        300.0,
        "1.b Search bar open — Calibre syntax, results update as you type",
    );
    lib_window(
        s,
        1660.0,
        320.0,
        1440.0,
        860.0,
        t,
        &Opts {
            search_open: true,
            search: "tag:sci-fi",
            banner: false,
            filter_to: Some(vec![3, 4, 6]),
            subtitle: "3 of 12 books",
            ..Default::default()
        },
    );
    note(
        s,
        1660.0,
        1216.0,
        &[
            "Ctrl+F, the search button, or just start typing. Closing the bar clears the filter. `text:` searches inside the books.",
            "The title shows “3 of 12 books” while a filter is active.",
        ],
    );

    // ---------------------------------------------------------------- 1.c
    label(
        s,
        90.0,
        1340.0,
        "1.c Several books selected — the action bar",
    );
    lib_window(
        s,
        90.0,
        1360.0,
        1440.0,
        800.0,
        t,
        &Opts {
            selected: vec![1, 2, 5],
            bottom_bar: true,
            ..Default::default()
        },
    );
    note(
        s,
        90.0,
        2196.0,
        &[
            "Ctrl/Shift-click or drag a rubber band. The bar at the bottom shows “3 selected” with Read, Edit Metadata (bulk), Remove, and a clear button.",
            "Double-click or Enter opens a book in the reader; Delete asks before removing.",
        ],
    );

    // ---------------------------------------------------------------- 1.d
    label(
        s,
        1660.0,
        1340.0,
        "1.d Context menu — only about the selected book",
    );
    lib_window(
        s,
        1660.0,
        1360.0,
        1440.0,
        800.0,
        t,
        &Opts {
            selected: vec![3],
            banner: false,
            ..Default::default()
        },
    );
    menu(
        s,
        1990.0,
        1500.0,
        270.0,
        &[
            "Read",
            "-",
            "Edit Metadata…",
            "Edit Book…",
            "-",
            "Mark as Read",
            "Mark as Unread",
            "-",
            "Send to Device…",
            "Export Annotations      ▸",
            "Convert To      ▸",
            "Save a Copy…",
            "Show in Files",
            "-",
            "!Remove…",
        ],
        t,
    );
    s.rect_a(1996.0, 1792.0, 258.0, 34.0, 8.0, t.accent, 0.18);
    menu(
        s,
        2276.0,
        1786.0,
        260.0,
        &[
            "EPUB (cleaned up)",
            "KEPUB (Kobo)",
            "AZW3 (Kindle)",
            "MOBI (older Kindles)",
            "FictionBook (FB2)",
            "HTML archive (HTMLZ)",
            "Text archive (TXTZ)",
            "-",
            "Plain text (TXT)",
            "Markdown (MD)",
            "HTML",
        ],
        t,
    );
    note(
        s,
        1660.0,
        2196.0,
        &[
            "Convert To: device formats first (EPUB, KEPUB, AZW3, MOBI, FB2, HTMLZ, TXTZ), then plain text, Markdown and HTML.",
            "One device entry: Send to Device… opens the Device window (1.n) with the selected books ready (disabled with “No device connected” otherwise). Export Annotations → JSON / Markdown / plain text.",
        ],
    );

    // ---------------------------------------------------------------- 1.e
    label(
        s,
        90.0,
        2320.0,
        "1.e Main menu — view options, panels and the other windows",
    );
    lib_window(
        s,
        90.0,
        2340.0,
        760.0,
        900.0,
        t,
        &Opts {
            banner: false,
            menu_open: true,
            ..Default::default()
        },
    );
    main_menu(s, 540.0, 2394.0, t);
    note(
        s,
        90.0,
        3276.0,
        &[
            "Grid / table, sort key and order, cover size, and the two optional panels.",
            "Reading Statistics, Vocabulary and Share Library open their own dialogs (sheet “Dialogs”).",
        ],
    );

    // ---------------------------------------------------------------- 1.f
    label(
        s,
        940.0,
        2320.0,
        "1.f Optional panels — Browse (left) and Details (right), switched on in the menu",
    );
    lib_window(
        s,
        940.0,
        2340.0,
        2160.0,
        900.0,
        t,
        &Opts {
            sidebar: true,
            details: Some(3),
            selected: vec![3],
            ..Default::default()
        },
    );
    note(
        s,
        940.0,
        3276.0,
        &[
            "Browse filters by Authors, Series, Tags, Publishers, Languages, Ratings and Reading status (click a name to fill the search).",
            "Details shows the selected book: cover, rating, format, publisher, identifiers, last read, annotations, comments. Both panels remember their state.",
        ],
    );

    // ---------------------------------------------------------------- 1.g
    label(
        s,
        90.0,
        3420.0,
        "1.g Table view — sortable columns (Calibre style), multi-select",
    );
    lib_window(
        s,
        90.0,
        3440.0,
        1440.0,
        700.0,
        t,
        &Opts {
            table: true,
            selected: vec![3, 4, 5],
            bottom_bar: true,
            banner: false,
            ..Default::default()
        },
    );
    note(
        s,
        90.0,
        4176.0,
        &["Click a column header to sort. Switch between grid and table in the main menu."],
    );

    label(s, 1660.0, 3420.0, "1.h Search — the built-in cheat sheet");
    search_cheat_sheet(s, 1660.0, 3440.0, t);

    // ---------------------------------------------------------------- 1.i add / states
    label(s, 90.0, 4320.0, "1.i Add books");
    menu(
        s,
        90.0,
        4340.0,
        320.0,
        &[
            "Add Books…         Ctrl+O",
            "Add from the Web…",
            "Add Folder…",
            "Add Folder Recursively…",
        ],
        t,
    );
    note(
        s,
        90.0,
        4500.0,
        &[
            "Drag & drop files or folders anywhere",
            "on the library. Duplicates are skipped",
            "(same title + first author).",
            "TXT / MD / HTML are converted to EPUB.",
        ],
    );
    toast(s, 250.0, 4620.0, "Added 4 books", None);
    {
        // Add from the Web: an article to read later, or a news feed made into a book
        let (x, y, w) = (90.0, 4700.0, 560.0);
        s.shadow(x, y, w, 270.0, 16.0, 0.3);
        s.rect(x, y, w, 270.0, 16.0, t.card, Some(t.border));
        s.text(
            x + w / 2.0,
            y + 44.0,
            20.0,
            true,
            t.fg,
            "middle",
            "Add from the Web",
        );
        s.text(
            x + w / 2.0,
            y + 72.0,
            13.0,
            false,
            t.dim,
            "middle",
            "Paste the address of an article to read later, or of a news feed",
        );
        s.text(
            x + w / 2.0,
            y + 90.0,
            13.0,
            false,
            t.dim,
            "middle",
            "(RSS, Atom, JSON Feed) to make a book of its latest articles.",
        );
        entry(
            s,
            x + 30.0,
            y + 110.0,
            w - 60.0,
            40.0,
            "https://example.com/article",
            true,
            t,
        );
        let by = y + 210.0;
        s.line(x, by - 8.0, x + w, by - 8.0, t.border, 1.0);
        s.line(x + w / 2.0, by - 8.0, x + w / 2.0, y + 270.0, t.border, 1.0);
        s.text(
            x + w / 4.0,
            by + 24.0,
            15.0,
            false,
            t.fg,
            "middle",
            "Cancel",
        );
        s.text(
            x + 3.0 * w / 4.0,
            by + 24.0,
            15.0,
            true,
            t.accent,
            "middle",
            "Add",
        );
    }

    label(s, 500.0, 4320.0, "0. Welcome / empty library");
    {
        let (x, y) = (500.0, 4340.0);
        window(s, x, y, 780.0, 520.0, t, true);
        s.rect(x + 12.0, y + 10.0, 34.0, 30.0, 8.0, t.hover, None);
        icon_plus(s, x + 29.0, y + 25.0, t.fg);
        s.text(x + 390.0, y + 21.0, 14.0, true, t.fg, "middle", "Bookx");
        cover(s, x + 350.0, y + 120.0, 80.0, 110.0, 0, "", "", None, false);
        s.text(
            x + 390.0,
            y + 290.0,
            26.0,
            true,
            t.fg,
            "middle",
            "Welcome to Bookx",
        );
        s.text(
            x + 390.0,
            y + 322.0,
            14.0,
            false,
            t.dim,
            "middle",
            "Add some books to start your library.",
        );
        s.text(
            x + 390.0,
            y + 342.0,
            14.0,
            false,
            t.dim,
            "middle",
            "EPUB, TXT, Markdown and HTML are supported — drop them here.",
        );
        btn(
            s,
            x + 176.0,
            y + 380.0,
            130.0,
            42.0,
            "Add Books…",
            "suggested",
            t,
        );
        btn(
            s,
            x + 318.0,
            y + 380.0,
            130.0,
            42.0,
            "Add Folder…",
            "normal",
            t,
        );
        btn(
            s,
            x + 460.0,
            y + 380.0,
            170.0,
            42.0,
            "News Subscriptions…",
            "normal",
            t,
        );
    }
    label(s, 1380.0, 4320.0, "1.j Importing · no results");
    {
        let (x, y) = (1380.0, 4340.0);
        window(s, x, y, 640.0, 250.0, t, true);
        s.ring(x + 60.0, y + 25.0, 8.0, t.accent, 2.5);
        s.text(
            x + 320.0,
            y + 150.0,
            18.0,
            true,
            t.fg,
            "middle",
            "Importing 38 books…",
        );
        s.text(
            x + 320.0,
            y + 176.0,
            13.0,
            false,
            t.dim,
            "middle",
            "Parsing metadata and extracting covers in the background",
        );
        window(s, x, y + 280.0, 640.0, 250.0, t, true);
        icon_search(s, x + 320.0, y + 380.0, t.dim);
        s.text(
            x + 320.0,
            y + 440.0,
            18.0,
            true,
            t.fg,
            "middle",
            "No Results",
        );
        s.text(
            x + 320.0,
            y + 466.0,
            13.0,
            false,
            t.dim,
            "middle",
            "No book matches your search",
        );
    }
    label(s, 2150.0, 4320.0, "1.k Remove confirmation · toast");
    alert(
        s,
        2150.0,
        4350.0,
        520.0,
        "Remove Book?",
        &[
            "“Dune” and its bookmarks and highlights will be",
            "removed from the library.",
        ],
        "Cancel",
        "Remove",
        true,
        t,
    );
    alert(
        s,
        2150.0,
        4560.0,
        520.0,
        "Remove Books?",
        &[
            "3 books with their bookmarks and highlights",
            "will be removed from the library.",
        ],
        "Cancel",
        "Remove",
        true,
        t,
    );
    toast(s, 2410.0, 4790.0, "3 books removed", None);
    note(
        s,
        2150.0,
        4880.0,
        &[
            "Libraries are portable folders:",
            "  Author/Title (id)/Title - Author.epub",
            "  bookx-library.db  (catalogue, progress, notes)",
            "Cover art here is abstract; real covers come",
            "from the EPUB files.",
        ],
    );
    // ---------------------------------------------------------------- 1.l device
    label(
        s,
        90.0,
        5200.0,
        "1.l A device is connected — the Device button and the banner",
    );
    lib_window(
        s,
        90.0,
        5220.0,
        1440.0,
        900.0,
        t,
        &Opts {
            device: Some("Kobo Libra 2"),
            device_banner: true,
            details: Some(2),
            selected: vec![2],
            banner: false,
            ..Default::default()
        },
    );
    note(
        s,
        90.0,
        6156.0,
        &[
            "The Device button appears in the header bar only while a device is connected, with its name; it opens the Device window (1.n).",
            "The banner offers Sync, Always (turns on automatic sync in Preferences) or Not now. Nothing happens before you answer unless automatic sync is on.",
            "The details panel opened by itself when the book was clicked; its toggle, like the browse toggle, stays in the header bar and the window buttons never move.",
        ],
    );

    // ---------------------------------------------------------------- 1.m sync summary
    label(
        s,
        1660.0,
        5200.0,
        "1.m Sync summary — what will happen, in which direction",
    );
    alert(
        s,
        1660.0,
        5240.0,
        620.0,
        "Sync with Kobo Libra 2?",
        &[
            "3 books: progress moves forward from the device",
            "  (Dune 42%→67%, Emma 0%→100%, Piranesi 10%→18%)",
            "5 highlights go device→Bookx",
            "2 highlights go Bookx→device (1 with notes)",
            "Nothing is deleted on either side.",
        ],
        "Cancel",
        "Sync",
        false,
        t,
    );
    note(
        s,
        1660.0,
        5510.0,
        &[
            "Always progressive: progress only moves forward, highlights and notes are united by identity, a richer note wins over an empty one,",
            "and nothing is ever deleted or lowered because the other side lacks it.",
        ],
    );

    // ---------------------------------------------------------------- 1.n device window
    label(
        s,
        90.0,
        6280.0,
        "1.n The Device window — books to send, books on the device",
    );
    {
        let (x, y, w, h) = (90.0, 6300.0, 860.0, 1120.0);
        s.shadow(x, y, w, h, 18.0, 0.35);
        s.rect(x, y, w, h, 18.0, t.bg, Some(t.border));
        s.line(x, y + 54.0, x + w, y + 54.0, t.border, 1.0);
        s.rect(x + 14.0, y + 11.0, 40.0, 32.0, 8.0, t.hover, None);
        icon_grid(s, x + 34.0, y + 27.0, t.fg);
        s.rect(x + 56.0, y + 11.0, 40.0, 32.0, 8.0, t.accent, None);
        icon_list(s, x + 76.0, y + 27.0, "#ffffff");
        s.text(x + w / 2.0, y + 33.0, 16.0, true, t.fg, "middle", "Device");
        s.rect(
            x + 20.0,
            y + 70.0,
            w - 40.0,
            62.0,
            12.0,
            t.card,
            Some(t.border),
        );
        s.text(
            x + 38.0,
            y + 96.0,
            14.0,
            true,
            t.fg,
            "start",
            "Kobo Libra 2",
        );
        s.text(
            x + 38.0,
            y + 116.0,
            12.0,
            false,
            t.dim,
            "start",
            "Kobo · Main memory · 11.4 GB free of 32.0 GB",
        );

        // ---- Books to send: check box far left, fixed cover slot, title, details, warning
        s.text(
            x + 20.0,
            y + 166.0,
            15.0,
            true,
            t.fg,
            "start",
            "Books to send",
        );
        s.text(
            x + w - 20.0,
            y + 166.0,
            12.0,
            false,
            t.dim,
            "end",
            "2 of 3 selected   Select All   None",
        );
        let rows: [(usize, &str, &str, &str, &str, bool); 3] = [
            (
                4,
                "EPUB → KEPUB (converted)",
                "+ 3 notes added as a last section",
                "4.0 MB",
                "",
                true,
            ),
            (3, "EPUB → KEPUB (converted)", "", "0.7 MB", "", true),
            (
                0,
                "EPUB → KEPUB (converted)",
                "",
                "1.1 MB",
                "⚠ Already on the device; sending adds a second copy (skipped unless you tick it).",
                false,
            ),
        ];
        let mut yy = y + 180.0;
        for (i, (bi, fmt, notes, size, warn, on)) in rows.iter().enumerate() {
            let b = &BOOKS[*bi];
            let rh = if warn.is_empty() { 84.0 } else { 104.0 };
            s.rect(
                x + 20.0,
                yy,
                w - 40.0,
                rh,
                0.0,
                if *on { "#e4eefb" } else { t.card },
                Some(t.border),
            );
            s.rect(
                x + 34.0,
                yy + rh / 2.0 - 11.0,
                22.0,
                22.0,
                5.0,
                if *on { t.accent } else { "#ffffff" },
                Some(t.border),
            );
            if *on {
                icon_check(s, x + 45.0, yy + rh / 2.0, "#ffffff");
            }
            cover(s, x + 74.0, yy + 12.0, 40.0, 60.0, *bi, "", "", None, false);
            s.text(x + 130.0, yy + 26.0, 14.0, true, t.fg, "start", b.title);
            s.text(
                x + 130.0,
                yy + 44.0,
                12.0,
                false,
                t.dim,
                "start",
                &format!("{} · {}", b.author, size),
            );
            s.text(x + 130.0, yy + 60.0, 12.0, false, t.dim, "start", fmt);
            if !notes.is_empty() {
                s.text(x + 330.0, yy + 60.0, 12.0, false, t.dim, "start", notes);
            }
            if !warn.is_empty() {
                s.text(x + 130.0, yy + 80.0, 12.0, false, "#c64600", "start", warn);
            }
            let _ = i;
            yy += rh;
        }
        btn(
            s,
            x + w - 190.0,
            yy + 12.0,
            170.0,
            40.0,
            "Send 2 books",
            "suggested",
            t,
        );

        // ---- Books on the device: same columns, tiles click to toggle
        let by = yy + 90.0;
        s.text(
            x + 20.0,
            by,
            15.0,
            true,
            t.fg,
            "start",
            "Books on the device",
        );
        btn(
            s,
            x + w - 330.0,
            by - 24.0,
            150.0,
            34.0,
            "Sync Reading State",
            "normal",
            t,
        );
        btn(
            s,
            x + w - 170.0,
            by - 24.0,
            150.0,
            34.0,
            "Remove 2 Selected…",
            "destructive",
            t,
        );
        s.text(
            x + 20.0,
            by + 24.0,
            12.0,
            false,
            t.dim,
            "start",
            "Tiles: click anywhere on a tile to tick it. List view uses the same columns as above.",
        );
        for col in 0..5 {
            let b = &BOOKS[(col * 3 + 1) % BOOKS.len()];
            let cx = x + 28.0 + col as f64 * 160.0;
            let cy = by + 40.0;
            let on = col == 1 || col == 3;
            if on {
                s.rect_a(cx - 8.0, cy - 8.0, 144.0, 226.0, 10.0, t.accent, 0.22);
            }
            cover(
                s,
                cx,
                cy,
                128.0,
                180.0,
                col * 3 + 1,
                b.title,
                b.author,
                None,
                false,
            );
            s.rect(
                cx + 6.0,
                cy + 6.0,
                22.0,
                22.0,
                5.0,
                if on { t.accent } else { "#ffffff" },
                Some(t.border),
            );
            if on {
                icon_check(s, cx + 17.0, cy + 17.0, "#ffffff");
            }
            let name: String = b.title.chars().take(16).collect();
            s.text(cx + 64.0, cy + 202.0, 12.0, false, t.fg, "middle", &name);
        }
        let oy = by + 300.0;
        s.text(x + 20.0, oy, 15.0, true, t.fg, "start", "Sending options");
        switch_row(
            s,
            x + 20.0,
            oy + 14.0,
            w - 40.0,
            78.0,
            "Add my notes to the copy sent to the device",
            Some(
                "An extra “My notes” section at the end of the copy on the device; your library copy is not changed",
            ),
            true,
            t,
        );
        switch_row(
            s,
            x + 20.0,
            oy + 100.0,
            w - 40.0,
            78.0,
            "Sync automatically when a device connects",
            Some(
                "Cable-connected readers · progress only moves forward, nothing is deleted      (?)",
            ),
            false,
            t,
        );
    }
    note(
        s,
        990.0,
        6340.0,
        &[
            "Books to send: exactly the books selected in the library (right-click → Send to Device…), each with the format it will have,",
            "whether it is converted, the notes that are added, its size and any warning. Untick a book to leave it out; a warned book starts unticked.",
            "Every row and tile has its check box at the far left / top-left and toggles when you click anywhere on it; Shift-click ticks a range.",
            "Covers sit in a fixed 40×60 slot so the check boxes and titles line up whatever the cover.",
            "“Send N books” sends only the ticked ones, with the same format decisions the preview shows.",
        ],
    );

    s.text(
        1600.0,
        7480.0,
        12.0,
        false,
        "#77767b",
        "middle",
        "Mockups are design illustrations. Book titles are used for illustration purposes only.",
    );
}
