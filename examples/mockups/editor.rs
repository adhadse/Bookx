use crate::kit::*;
use crate::reader::{color_controls, Colors};

pub struct E {
    pub tab: &'static str,
    pub find: bool,
    pub panel: Option<&'static str>,
    pub image: bool,
}

const CODE: [&str; 18] = [
    "<?xml version=\"1.0\" encoding=\"utf-8\"?>",
    "<!DOCTYPE html>",
    "<html xmlns=\"http://www.w3.org/1999/xhtml\" lang=\"en\">",
    "  <head>",
    "    <title>Chapter 2</title>",
    "    <link rel=\"stylesheet\" href=\"../styles/book.css\"/>",
    "  </head>",
    "  <body>",
    "    <section id=\"ch02\" class=\"chapter\">",
    "      <h1>Reliable, Scalable, Maintainable</h1>",
    "      <p>Many applications today are <em>data-intensive</em>, as",
    "      opposed to compute-intensive. &amp; CPU power is rarely a",
    "      limiting factor for these applications.</p>",
    "      <!-- TODO: add figure 2-1 -->",
    "      <figure><img src=\"../images/fig2-1.png\" alt=\"Layers\"/></figure>",
    "      <p class=\"note\">Reliability means continuing to work",
    "      correctly, even when things go wrong.</p>",
    "    </section>",
];

/// The minimap at the right edge of the code view: every line of the file as a small bar, the
/// visible part tinted (GtkSourceView's `Map`; toggle in the Markup menu).
fn minimap(s: &mut Svg, x: f64, y: f64, w: f64, h: f64, lines: &[&str], visible: usize, t: &Theme) {
    s.rect_a(x, y, w, h, 0.0, t.dim, 0.07);
    s.rect_a(x, y, w, (visible as f64 * 5.0).min(h), 0.0, "#3584e4", 0.16);
    for (i, l) in lines.iter().enumerate() {
        let by = y + 6.0 + i as f64 * 5.0;
        if by > y + h - 6.0 {
            break;
        }
        let indent = (l.len() - l.trim_start().len()) as f64 * 1.3;
        let len = (l.trim().len() as f64 * 1.3).min(w - 12.0 - indent);
        s.rect_a(x + 6.0 + indent, by, len.max(2.0), 2.5, 1.0, t.dim, 0.55);
    }
}

fn syntax(s: &mut Svg, x: f64, y: f64, line: &str, t: &Theme, dark: bool) {
    // very small tokenizer: tags blue, attrs green, strings yellow, comments grey, entities orange
    let (ctag, cattr, cstr, ccom, cent) = if dark {
        ("#78aeed", "#8ff0a4", "#f9f06b", "#9a9996", "#ffbe6f")
    } else {
        ("#1a5fb4", "#26a269", "#986a00", "#77767b", "#c64600")
    };
    let cw = 8.4;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let col = |s: &mut Svg, start: usize, end: usize, c: &str| {
        let txt: String = chars[start..end].iter().collect();
        s.mono(x + start as f64 * cw, y, 14.0, c, &txt);
    };
    if line.trim_start().starts_with("<!--") {
        col(s, 0, chars.len(), ccom);
        return;
    }
    while i < chars.len() {
        if chars[i] == '<' {
            let st = i;
            while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '>' {
                i += 1;
            }
            col(s, st, i, ctag);
            while i < chars.len() && chars[i] != '>' {
                if chars[i] == '"' {
                    let a = i;
                    i += 1;
                    while i < chars.len() && chars[i] != '"' {
                        i += 1;
                    }
                    i = (i + 1).min(chars.len());
                    col(s, a, i, cstr);
                } else if chars[i].is_alphabetic() {
                    let a = i;
                    while i < chars.len()
                        && (chars[i].is_alphanumeric() || chars[i] == '-' || chars[i] == ':')
                    {
                        i += 1;
                    }
                    col(s, a, i, cattr);
                } else {
                    let a = i;
                    i += 1;
                    col(s, a, i, ctag);
                }
            }
            if i < chars.len() {
                col(s, i, i + 1, ctag);
                i += 1;
            }
        } else if chars[i] == '&' {
            let a = i;
            while i < chars.len() && chars[i] != ';' {
                i += 1;
            }
            i = (i + 1).min(chars.len());
            col(s, a, i, cent);
        } else {
            let a = i;
            while i < chars.len() && chars[i] != '<' && chars[i] != '&' {
                i += 1;
            }
            col(s, a, i, t.fg);
        }
    }
}

pub fn editor_window(s: &mut Svg, x: f64, y: f64, w: f64, h: f64, t: &Theme, e: &E) {
    let dark = t.bg == DARK.bg;
    s.shadow(x, y, w, h, 14.0, t.shadow);
    s.rect(x, y, w, h, 14.0, t.bg, Some(t.border));
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
    s.line(x, y + 46.0, x + w, y + 46.0, t.border, 1.0);
    btn(s, x + 12.0, y + 8.0, 70.0, 32.0, "Save", "suggested", t);
    icon_undo(s, x + 112.0, y + 24.0, t.fg, false);
    icon_undo(s, x + 148.0, y + 24.0, t.dim, true);
    s.text(
        x + w / 2.0,
        y + 21.0,
        14.0,
        true,
        t.fg,
        "middle",
        "● Designing Data-Intensive Applications",
    );
    s.text(
        x + w / 2.0,
        y + 38.0,
        11.0,
        false,
        t.dim,
        "middle",
        "OEBPS/text/ch02.xhtml",
    );
    // right tools: find & replace · check book · reformat · tools menu · preview toggle
    s.rect(
        x + w - 312.0,
        y + 9.0,
        34.0,
        30.0,
        8.0,
        if e.find { t.hover } else { "none" },
        None,
    );
    icon_search(s, x + w - 295.0, y + 24.0, t.fg);
    icon_status(s, x + w - 252.0, y + 24.0, "warn");
    for (i, wd) in [14.0, 9.0, 12.0].iter().enumerate() {
        s.line(
            x + w - 219.0,
            y + 18.0 + i as f64 * 6.0,
            x + w - 219.0 + wd,
            y + 18.0 + i as f64 * 6.0,
            t.fg,
            1.8,
        );
    }
    icon_tools(s, x + w - 172.0, y + 24.0, t.fg);
    s.rect(x + w - 144.0, y + 9.0, 34.0, 30.0, 8.0, t.hover, None);
    icon_sidebar(s, x + w - 127.0, y + 24.0, t.fg, true);
    s.circle(x + w - 26.0, y + 23.0, 12.0, t.hover);
    icon_close(s, x + w - 26.0, y + 23.0, t.fg);

    let top = y + 46.0;
    let bottom = y + h - 28.0;
    // sidebar
    let sw = 290.0;
    s.rect(x, top, sw, bottom - top, 0.0, t.panel, None);
    s.line(x + sw, top, x + sw, bottom, t.border, 1.0);
    s.text(x + sw / 2.0, top + 30.0, 14.0, true, t.fg, "middle", "Book");
    s.line(x, top + 44.0, x + sw, top + 44.0, t.border, 1.0);
    match e.tab {
        "Files" => {
            let groups: [(&str, [&str; 4]); 4] = [
                (
                    "TEXT",
                    ["cover.xhtml", "ch01.xhtml", "ch02.xhtml", "ch03.xhtml"],
                ),
                ("STYLES", ["book.css", "", "", ""]),
                ("IMAGES", ["cover.png", "fig2-1.png", "fig2-2.png", ""]),
                ("PACKAGE", ["content.opf", "toc.ncx", "nav.xhtml", ""]),
            ];
            let mut yy = top + 66.0;
            for (g, files) in groups {
                s.text(x + 14.0, yy, 11.0, true, t.dim, "start", g);
                yy += 12.0;
                for (i, f) in files.iter().enumerate() {
                    if f.is_empty() {
                        continue;
                    }
                    if *f == "ch02.xhtml" {
                        s.rect(x + 6.0, yy, sw - 12.0, 32.0, 8.0, t.hover, None);
                        s.circle(x + sw - 62.0, yy + 16.0, 4.0, t.accent);
                    }
                    if g == "IMAGES" {
                        icon_img(s, x + 28.0, yy + 16.0, t.fg);
                    } else if g == "STYLES" {
                        icon_file(s, x + 28.0, yy + 16.0, "#c64600");
                    } else {
                        icon_file(s, x + 28.0, yy + 16.0, t.accent);
                    }
                    s.text(x + 50.0, yy + 21.0, 13.0, false, t.fg, "start", f);
                    if g == "TEXT" {
                        s.text(
                            x + sw - 24.0,
                            yy + 21.0,
                            12.0,
                            false,
                            t.dim,
                            "end",
                            &format!("{}", i + 1),
                        );
                    }
                    yy += 34.0;
                }
                yy += 14.0;
            }
            s.line(x, bottom - 100.0, x + sw, bottom - 100.0, t.border, 1.0);
            for (i, k) in ["+", "pen", "trash", "up", "down"].iter().enumerate() {
                let cx = x + 28.0 + i as f64 * 44.0;
                match *k {
                    "+" => icon_plus(s, cx, bottom - 76.0, t.fg),
                    "pen" => icon_pencil(s, cx, bottom - 76.0, t.fg),
                    "trash" => icon_trash(s, cx, bottom - 76.0, t.fg),
                    "up" => icon_chev(s, cx, bottom - 76.0, t.fg, 'u'),
                    _ => icon_chev(s, cx, bottom - 76.0, t.fg, 'd'),
                }
            }
        }
        "Contents" => {
            let toc: [(&str, usize); 9] = [
                ("Preface", 0),
                ("1. Reliable, Scalable…", 0),
                ("2. Data Models", 0),
                ("Relational vs Document", 1),
                ("Query Languages", 1),
                ("Graph-Like Models", 1),
                ("3. Storage and Retrieval", 0),
                ("Data Structures", 1),
                ("4. Encoding and Evolution", 0),
            ];
            let mut yy = top + 58.0;
            for (i, (n, d)) in toc.iter().enumerate() {
                if i == 3 {
                    s.rect(x + 6.0, yy, sw - 12.0, 36.0, 8.0, t.hover, None);
                }
                s.rect(
                    x + 12.0 + 18.0 * *d as f64,
                    yy + 3.0,
                    190.0 - 18.0 * *d as f64,
                    30.0,
                    6.0,
                    t.card,
                    Some(t.border),
                );
                s.text(
                    x + 22.0 + 18.0 * *d as f64,
                    yy + 23.0,
                    13.0,
                    false,
                    t.fg,
                    "start",
                    n,
                );
                s.text(
                    x + sw - 12.0,
                    yy + 22.0,
                    11.0,
                    false,
                    t.dim,
                    "end",
                    "ch0x.xhtml",
                );
                yy += 40.0;
            }
            s.line(x, bottom - 100.0, x + sw, bottom - 100.0, t.border, 1.0);
            icon_plus(s, x + 28.0, bottom - 76.0, t.fg);
            icon_trash(s, x + 70.0, bottom - 76.0, t.fg);
            icon_chev(s, x + 112.0, bottom - 76.0, t.fg, 'u');
            icon_chev(s, x + 154.0, bottom - 76.0, t.fg, 'd');
            s.text(
                x + 198.0,
                bottom - 71.0,
                13.0,
                true,
                t.fg,
                "start",
                "Generate",
            );
            s.text(
                x + 14.0,
                bottom - 52.0,
                11.0,
                false,
                t.dim,
                "start",
                "indent ⇥  outdent ⇤  link to current file",
            );
        }
        _ => {
            cover(
                s,
                x + 70.0,
                top + 66.0,
                150.0,
                225.0,
                1,
                "Designing Data",
                "Martin Kleppmann",
                None,
                false,
            );
            s.text(
                x + sw / 2.0,
                top + 330.0,
                17.0,
                true,
                t.fg,
                "middle",
                "Designing Data-Intensive",
            );
            s.text(
                x + sw / 2.0,
                top + 352.0,
                17.0,
                true,
                t.fg,
                "middle",
                "Applications",
            );
            s.text(
                x + sw / 2.0,
                top + 376.0,
                13.0,
                false,
                t.dim,
                "middle",
                "Martin Kleppmann",
            );
            btn(
                s,
                x + 60.0,
                top + 410.0,
                170.0,
                40.0,
                "Edit Metadata…",
                "suggested",
                t,
            );
            s.text(
                x + sw / 2.0,
                top + 480.0,
                11.0,
                false,
                t.dim,
                "middle",
                "Applied to the book being edited;",
            );
            s.text(
                x + sw / 2.0,
                top + 496.0,
                11.0,
                false,
                t.dim,
                "middle",
                "written to the file on Save.",
            );
        }
    }
    // switcher
    s.line(x, bottom - 52.0, x + sw, bottom - 52.0, t.border, 0.0);
    let tabs = ["Files", "Contents", "Metadata"];
    for (i, n) in tabs.iter().enumerate() {
        let tx = x + 10.0 + i as f64 * 90.0;
        if *n == e.tab {
            s.rect(tx, bottom - 48.0, 86.0, 44.0, 10.0, t.hover, None);
        }
        s.rect(tx + 36.0, bottom - 42.0, 14.0, 14.0, 3.0, t.fg, None);
        s.text(tx + 43.0, bottom - 14.0, 11.0, true, t.fg, "middle", n);
    }

    // center
    let cx = x + sw;
    let mut cy = top;
    if e.find {
        s.rect(cx, cy, w - sw, 110.0, 0.0, t.header, None);
        s.line(cx, cy + 110.0, x + w, cy + 110.0, t.border, 1.0);
        entry(
            s,
            cx + 14.0,
            cy + 10.0,
            380.0,
            38.0,
            "replication",
            false,
            t,
        );
        btn(s, cx + 404.0, cy + 10.0, 44.0, 38.0, ".*", "normal", t);
        btn(s, cx + 454.0, cy + 10.0, 44.0, 38.0, "Aa", "normal", t);
        btn(
            s,
            cx + 508.0,
            cy + 10.0,
            140.0,
            38.0,
            "All text files  ▾",
            "normal",
            t,
        );
        btn(
            s,
            cx + 658.0,
            cy + 10.0,
            90.0,
            38.0,
            "Find All",
            "normal",
            t,
        );
        entry(s, cx + 14.0, cy + 58.0, 380.0, 38.0, "replicas", false, t);
        btn(
            s,
            cx + 404.0,
            cy + 58.0,
            100.0,
            38.0,
            "Replace",
            "normal",
            t,
        );
        btn(
            s,
            cx + 514.0,
            cy + 58.0,
            120.0,
            38.0,
            "Replace All",
            "normal",
            t,
        );
        s.text(
            cx + 650.0,
            cy + 83.0,
            12.0,
            false,
            t.dim,
            "start",
            "2 of 37 in this file",
        );
        cy += 110.0;
    }
    let empty = e.panel == Some("empty");
    let panel_h = if e.panel.is_some() && !empty {
        270.0
    } else {
        0.0
    };
    let code_h = bottom - cy - panel_h;
    let prev_w = 430.0;
    if e.image {
        let iw = w - sw;
        cover(
            s,
            cx + iw / 2.0 - 190.0,
            cy + 30.0,
            300.0,
            400.0,
            1,
            "Designing Data",
            "Martin Kleppmann",
            None,
            false,
        );
        s.text(
            cx + iw / 2.0 - 40.0,
            cy + 470.0,
            13.0,
            false,
            t.dim,
            "middle",
            "cover.png · 1200×1574 px · 995.5 KB",
        );
        btn(
            s,
            cx + iw / 2.0 - 100.0,
            cy + 490.0,
            130.0,
            40.0,
            "Use as Cover",
            "normal",
            t,
        );
    } else if empty {
        // nothing selected yet: a status page
        let iw = w - sw;
        icon_pencil(s, cx + iw / 2.0, cy + code_h / 2.0 - 70.0, t.dim);
        s.text(
            cx + iw / 2.0,
            cy + code_h / 2.0 - 10.0,
            24.0,
            true,
            t.fg,
            "middle",
            "Select a File",
        );
        s.text(
            cx + iw / 2.0,
            cy + code_h / 2.0 + 22.0,
            14.0,
            false,
            t.dim,
            "middle",
            "Pick a chapter, stylesheet or image from the Files list to start editing",
        );
    } else {
        // gutter + code
        let gw = 54.0;
        let code_x = cx + gw + 10.0;
        s.rect(
            cx,
            cy,
            w - sw - prev_w,
            code_h,
            0.0,
            if dark { "#1e1e1e" } else { "#ffffff" },
            None,
        );
        for (i, l) in CODE.iter().enumerate() {
            let ly = cy + 30.0 + i as f64 * 22.0;
            if ly > cy + code_h - 10.0 {
                break;
            }
            if i == 11 {
                s.rect_a(
                    cx + gw,
                    ly - 16.0,
                    w - sw - prev_w - gw,
                    22.0,
                    0.0,
                    "#3584e4",
                    0.14,
                );
            }
            s.mono(
                cx + gw - 12.0 - 8.4 * ((i + 1).to_string().len() as f64),
                ly,
                13.0,
                t.dim,
                &(i + 1).to_string(),
            );
            syntax(s, code_x, ly, l, t, dark);
        }
        minimap(
            s,
            cx + (w - sw - prev_w) - 70.0,
            cy,
            70.0,
            code_h,
            &CODE,
            18,
            t,
        );
        if e.find {
            // find hit highlight
            s.rect_a(
                code_x + 8.4 * 11.0,
                cy + 30.0 + 8.0 * 22.0 - 16.0,
                8.4 * 11.0,
                22.0,
                3.0,
                "#ffd91f",
                0.55,
            );
        }
        // preview
        let px = x + w - prev_w;
        s.rect(px, cy, prev_w, code_h, 0.0, t.bg, None);
        s.line(px, cy, px, cy + code_h, t.border, 1.0);
        s.text(px + 14.0, cy + 22.0, 12.0, true, t.fg, "start", "Preview");
        icon_palette(s, px + 84.0, cy + 17.0, t.dim);
        s.line(px, cy + 32.0, px + prev_w, cy + 32.0, t.border, 1.0);
        s.serif(
            px + 24.0,
            cy + 90.0,
            26.0,
            true,
            t.fg,
            "Reliable, Scalable,",
        );
        s.serif(px + 24.0, cy + 122.0, 26.0, true, t.fg, "Maintainable");
        greek(s, px + 24.0, cy + 150.0, prev_w - 48.0, 3, 24.0, t.fg, 2);
        s.rect(
            px + 24.0,
            cy + 234.0,
            prev_w - 48.0,
            110.0,
            6.0,
            t.hover,
            None,
        );
        s.text(
            px + prev_w / 2.0,
            cy + 296.0,
            13.0,
            false,
            t.dim,
            "middle",
            "[Layers]",
        );
        greek(s, px + 24.0, cy + 370.0, prev_w - 48.0, 4, 24.0, t.fg, 5);
    }
    // bottom panel
    if let Some(p) = e.panel.filter(|p| *p != "empty") {
        let py = bottom - panel_h;
        s.rect(cx, py, w - sw, panel_h, 0.0, t.bg, None);
        s.line(cx, py, x + w, py, t.border, 1.0);
        let (title, rows): (&str, Vec<(&str, &str, &str)>) = if p == "check" {
            (
                "3 errors, 2 warnings, 1 note",
                vec![
                    (
                        "error",
                        "XML error: Opening and ending tag mismatch: p and em",
                        "ch07.xhtml:48",
                    ),
                    (
                        "error",
                        "[RSC-007] Referenced resource \"../images/fig7-9.png\" could not be found",
                        "ch07.xhtml:112",
                    ),
                    (
                        "error",
                        "[RSC-005] Element \"li\" not allowed here; expected element \"ul\" or \"ol\"",
                        "ch09.xhtml:23",
                    ),
                    (
                        "warn",
                        "[OPF-003] File \"images/old-cover.jpg\" is in the package but not listed in the manifest",
                        "images/old-cover.jpg",
                    ),
                    (
                        "warn",
                        "The book has no language (dc:language)",
                        "content.opf",
                    ),
                    ("info", "The book has no cover image", "content.opf"),
                ],
            )
        } else if p == "spelling" {
            (
                "5 possible misspellings",
                vec![
                    (
                        "warn",
                        "\u{201c}wrods\u{201d}: maybe words",
                        "ch03.xhtml:41",
                    ),
                    (
                        "warn",
                        "\u{201c}recieve\u{201d}: maybe receive",
                        "ch03.xhtml:57",
                    ),
                    (
                        "warn",
                        "\u{201c}replicaton\u{201d}: maybe replication",
                        "ch05.xhtml:12",
                    ),
                    ("warn", "\u{201c}teh\u{201d}: maybe the", "ch05.xhtml:208"),
                    (
                        "warn",
                        "\u{201c}quorams\u{201d}: maybe quorums",
                        "ch05.xhtml:341",
                    ),
                ],
            )
        } else {
            (
                "37 matches",
                vec![
                    (
                        "find",
                        "…single-leader replication works by…",
                        "ch05.xhtml:12",
                    ),
                    (
                        "find",
                        "…replication lag and read-after-write…",
                        "ch05.xhtml:208",
                    ),
                    (
                        "find",
                        "…leaderless replication, quorums…",
                        "ch05.xhtml:341",
                    ),
                    ("find", "…statement-based replication…", "ch05.xhtml:402"),
                ],
            )
        };
        s.text(cx + 16.0, py + 26.0, 14.0, true, t.fg, "start", title);
        icon_close(s, x + w - 26.0, py + 22.0, t.dim);
        let mut yy = py + 40.0;
        for (k, m, f) in rows {
            if yy + 34.0 > py + panel_h {
                break;
            }
            match k {
                "find" => icon_search(s, cx + 30.0, yy + 18.0, t.dim),
                other => icon_status(s, cx + 30.0, yy + 18.0, other),
            }
            s.text(cx + 54.0, yy + 15.0, 13.0, false, t.fg, "start", m);
            s.text(cx + 54.0, yy + 32.0, 11.0, false, t.dim, "start", f);
            yy += 38.0;
        }
    }
    // status
    s.line(x, bottom, x + w, bottom, t.border, 1.0);
    s.text(
        x + 14.0,
        bottom + 19.0,
        12.0,
        false,
        t.dim,
        "start",
        "OEBPS/text/ch02.xhtml   ·   Ln 12, Col 18   ·   255 lines",
    );
}

/// The small palette button of the preview header (applications-graphics-symbolic).
fn icon_palette(s: &mut Svg, cx: f64, cy: f64, c: &str) {
    s.path(
        &format!(
            "M{},{} a8,8 0 1 1 3,15 c-3,0 -2,-3 -4,-3 c-3,0 -4,-1 -4,-4 a5,8 0 0 1 5,-8 z",
            cx - 2.0,
            cy - 8.0
        ),
        "none",
        c,
        1.5,
    );
    for (dx, dy) in [(-4.0, -2.0), (0.0, -5.0), (4.0, -2.0)] {
        s.circle(cx + dx, cy + dy, 1.5, c);
    }
}

/// The preview's colour popover: the same Theme / Original / Custom control as the reader menu.
fn preview_colors_popover(s: &mut Svg, x: f64, y: f64, mode: Colors, t: &Theme) {
    let w = 290.0;
    let h = 32.0 + if mode == Colors::Custom { 80.0 } else { 40.0 };
    popover_card(s, x, y, w, h, x + 70.0, true, t);
    color_controls(s, x + 16.0, y + 16.0, w - 32.0, mode, t);
}

/// The wrench of the Tools button.
fn icon_tools(s: &mut Svg, cx: f64, cy: f64, c: &str) {
    s.path(
        &format!(
            "M{},{} l7,-7 a4,4 0 1 1 3,3 l-7,7 a1.5,1.5 0 0 1 -3,-3 z",
            cx - 7.0,
            cy + 5.0
        ),
        "none",
        c,
        1.6,
    );
}

/// The Tools menu: one-click operations, each snapshotted first.
fn tools_menu(s: &mut Svg, x: f64, y: f64, t: &Theme) {
    popover_card(s, x, y, 290.0, 585.0, x + 290.0 - 34.0, true, t);
    let items: [&str; 21] = [
        "Fix Problems Automatically",
        "Upgrade to EPUB 3",
        "-",
        "Insert Contents Page",
        "Remove Contents Page",
        "Improve Accessibility",
        "-",
        "Compress Images",
        "Shrink Embedded Fonts",
        "Arrange Files in Folders",
        "-",
        "Split File at Chapter Headings",
        "Merge with Next File",
        "-",
        "Check Spelling",
        "Book Report…",
        "-",
        "Create Checkpoint",
        "Restore Checkpoint      ▸",
        "Compare with Checkpoint      ▸",
        "",
    ];
    let mut yy = y + 12.0;
    for it in items {
        match it {
            "-" => {
                s.line(x + 10.0, yy + 5.0, x + 280.0, yy + 5.0, t.border, 1.0);
                yy += 11.0;
            }
            "" => {}
            _ => {
                s.text(x + 18.0, yy + 22.0, 14.0, false, t.fg, "start", it);
                yy += 34.0;
            }
        }
    }
}

/// The Book Report dialog.
fn book_report(s: &mut Svg, x: f64, y: f64, t: &Theme) {
    let (w, h) = (500.0, 520.0);
    let top = adw_dialog(s, x, y, w, h, "Book Report", t);
    let rows = [
        ("Files", "42 (1.3 MiB)"),
        ("Words", "98,412 (9,350 different)"),
        ("Characters", "561,908 (96 different)"),
        ("Links", "36 inside the book, 4 to the web, 0 broken"),
        ("Images", "12 (880 KiB, about 210 KiB could be saved)"),
        ("Style rules", "120 (14 unused)"),
        ("Classes without a style", "3"),
    ];
    let gx = x + 20.0;
    let gw = w - 40.0;
    let rh = 50.0;
    s.rect(
        gx,
        top + 10.0,
        gw,
        rows.len() as f64 * rh,
        12.0,
        t.card,
        Some(t.border),
    );
    for (i, (k, v)) in rows.iter().enumerate() {
        let yy = top + 10.0 + i as f64 * rh;
        if i > 0 {
            s.line(gx, yy, gx + gw, yy, t.border, 1.0);
        }
        s.text(gx + 16.0, yy + 30.0, 14.0, false, t.fg, "start", k);
        s.text(gx + gw - 16.0, yy + 30.0, 12.0, false, t.dim, "end", v);
    }
}

/// The target layout of docs/editor-ui-design.md (GNOME Builder is the inspiration, nothing is copied):
/// tabs, sidebar panels, search bar with a live count, gutter marks, spelling underline, bottom panel,
/// status bar, quick open.
fn design_window(s: &mut Svg, x: f64, y: f64, w: f64, h: f64, t: &Theme) {
    s.shadow(x, y, w, h, 14.0, t.shadow);
    s.rect(x, y, w, h, 14.0, t.bg, Some(t.border));
    s.rect(x, y, w, 46.0, 14.0, t.header, None);
    s.line(x, y + 46.0, x + w, y + 46.0, t.border, 1.0);
    btn(s, x + 12.0, y + 8.0, 70.0, 32.0, "Save", "suggested", t);
    icon_undo(s, x + 112.0, y + 24.0, t.fg, false);
    icon_undo(s, x + 148.0, y + 24.0, t.dim, true);
    s.text(
        x + w / 2.0,
        y + 21.0,
        14.0,
        true,
        t.fg,
        "middle",
        "Designing Data-Intensive Applications",
    );
    s.text(
        x + w / 2.0,
        y + 38.0,
        11.0,
        false,
        t.dim,
        "middle",
        "OEBPS/text/ch02.xhtml",
    );
    icon_search(s, x + w - 300.0, y + 24.0, t.fg);
    icon_status(s, x + w - 260.0, y + 24.0, "warn");
    s.text(x + w - 232.0, y + 29.0, 15.0, true, t.fg, "middle", "<>");
    icon_tools(s, x + w - 190.0, y + 24.0, t.fg);
    icon_sidebar(s, x + w - 150.0, y + 24.0, t.fg, true);

    let top = y + 46.0;
    let bottom = y + h - 28.0;
    let sw = 270.0;
    s.rect(x, top, sw, bottom - top, 0.0, t.panel, None);
    s.line(x + sw, top, x + sw, bottom, t.border, 1.0);
    for (i, (name, on)) in [
        ("Files", true),
        ("Outline", false),
        ("Search", false),
        ("Meta", false),
    ]
    .iter()
    .enumerate()
    {
        let tx = x + 12.0 + i as f64 * 62.0;
        if *on {
            s.rect(tx - 4.0, top + 8.0, 58.0, 28.0, 8.0, t.hover, None);
        }
        s.text(
            tx + 25.0,
            top + 27.0,
            13.0,
            *on,
            if *on { t.fg } else { t.dim },
            "middle",
            name,
        );
    }
    s.line(x, top + 44.0, x + sw, top + 44.0, t.border, 1.0);
    let mut yy = top + 62.0;
    for (grp, files) in [
        (
            "TEXT",
            vec![
                ("ch01.xhtml", false),
                ("ch02.xhtml", true),
                ("ch03.xhtml", true),
            ],
        ),
        ("STYLES", vec![("book.css", false)]),
        ("IMAGES", vec![("fig2-1.png", false), ("fig2-2.png", false)]),
    ] {
        s.text(x + 14.0, yy, 11.0, true, t.dim, "start", grp);
        yy += 8.0;
        for (f, sel) in files {
            if sel {
                s.rect(x + 6.0, yy, sw - 12.0, 30.0, 8.0, t.accent, None);
            }
            icon_file(s, x + 26.0, yy + 15.0, if sel { "#ffffff" } else { t.dim });
            s.text(
                x + 46.0,
                yy + 20.0,
                14.0,
                false,
                if sel { "#ffffff" } else { t.fg },
                "start",
                f,
            );
            yy += 32.0;
        }
        yy += 14.0;
    }
    s.text(
        x + 14.0,
        bottom - 14.0,
        12.0,
        false,
        t.dim,
        "start",
        "2 files selected · Ctrl+click, Shift+click",
    );

    // tab bar
    let cx = x + sw;
    let cw = w - sw;
    s.rect(cx, top, cw, 34.0, 0.0, t.header, None);
    s.line(cx, top + 34.0, cx + cw, top + 34.0, t.border, 1.0);
    for (i, (name, on)) in [
        ("ch02.xhtml", true),
        ("book.css", false),
        ("fig2-1.png", false),
    ]
    .iter()
    .enumerate()
    {
        let tx = cx + 8.0 + i as f64 * 150.0;
        s.rect(
            tx,
            top + 4.0,
            142.0,
            30.0,
            8.0,
            if *on { t.bg } else { "none" },
            None,
        );
        s.text(
            tx + 14.0,
            top + 24.0,
            13.0,
            *on,
            if *on { t.fg } else { t.dim },
            "start",
            name,
        );
        icon_close(s, tx + 126.0, top + 19.0, t.dim);
    }
    // search bar
    let sy = top + 35.0;
    s.rect(cx, sy, cw, 76.0, 0.0, t.panel, None);
    s.line(cx, sy + 76.0, cx + cw, sy + 76.0, t.border, 1.0);
    entry(s, cx + 12.0, sy + 6.0, 330.0, 30.0, "wrods", false, t);
    entry(s, cx + 12.0, sy + 40.0, 330.0, 30.0, "words", false, t);
    for (i, l) in [".*", "Aa", "\\b"].iter().enumerate() {
        btn(
            s,
            cx + 352.0 + i as f64 * 44.0,
            sy + 6.0,
            38.0,
            30.0,
            l,
            "flat",
            t,
        );
    }
    btn(
        s,
        cx + 490.0,
        sy + 6.0,
        130.0,
        30.0,
        "This file ▾",
        "flat",
        t,
    );
    btn(s, cx + 352.0, sy + 40.0, 90.0, 30.0, "Replace", "flat", t);
    btn(
        s,
        cx + 450.0,
        sy + 40.0,
        110.0,
        30.0,
        "Replace All",
        "flat",
        t,
    );
    s.text(
        cx + 640.0,
        sy + 27.0,
        13.0,
        true,
        t.accent,
        "start",
        "3 of 12 in this file · 41 in the book",
    );

    // code with gutter marks and a spelling underline
    let code_top = sy + 76.0;
    let pw = 300.0;
    let code_w = cw - pw;
    let code_bottom = bottom - 150.0;
    s.rect(
        cx,
        code_top,
        54.0,
        code_bottom - code_top,
        0.0,
        t.panel,
        None,
    );
    for (i, line) in CODE.iter().enumerate().take(14) {
        let ly = code_top + 26.0 + i as f64 * 22.0;
        s.text(
            cx + 44.0,
            ly,
            12.0,
            false,
            t.dim,
            "end",
            &format!("{}", i + 1),
        );
        s.mono(cx + 66.0, ly, 13.0, t.fg, line);
        if i == 9 || i == 10 {
            s.rect(cx + 2.0, ly - 14.0, 4.0, 20.0, 1.0, "#26a269", None);
        }
        if i == 13 {
            s.rect(cx + 2.0, ly - 14.0, 4.0, 20.0, 1.0, "#e5a50a", None);
            s.circle(cx + 24.0, ly - 4.0, 5.0, "#e5a50a");
        }
    }
    minimap(
        s,
        cx + code_w - 70.0,
        code_top,
        70.0,
        code_bottom - code_top,
        &CODE,
        14,
        t,
    );
    let uy = code_top + 26.0 + 10.0 * 22.0 + 4.0;
    s.path(
        &format!(
            "M{},{} l3,3 l3,-3 l3,3 l3,-3 l3,3 l3,-3 l3,3 l3,-3 l3,3 l3,-3",
            cx + 160.0,
            uy
        ),
        "none",
        "#e01b24",
        1.4,
    );
    popover_card(s, cx + 150.0, uy + 12.0, 220.0, 112.0, cx + 190.0, true, t);
    for (i, l) in [
        "Replace with “words”",
        "Add to dictionary",
        "Ignore all",
        "Change all…",
    ]
    .iter()
    .enumerate()
    {
        s.text(
            cx + 168.0,
            uy + 40.0 + i as f64 * 24.0,
            13.0,
            i == 0,
            t.fg,
            "start",
            l,
        );
    }
    // preview pane following the cursor
    let px = cx + code_w;
    s.line(px, code_top, px, code_bottom, t.border, 1.0);
    s.rect(
        px + 1.0,
        code_top,
        pw - 1.0,
        code_bottom - code_top,
        0.0,
        t.card,
        None,
    );
    s.text(
        px + 16.0,
        code_top + 22.0,
        11.0,
        false,
        t.dim,
        "start",
        "Preview · follows the cursor",
    );
    s.serif(
        px + 20.0,
        code_top + 70.0,
        22.0,
        true,
        t.fg,
        "Reliable, Scalable, Maintainable",
    );
    greek(
        s,
        px + 20.0,
        code_top + 100.0,
        pw - 40.0,
        6,
        16.0,
        t.border,
        3,
    );
    s.rect_a(
        px + 14.0,
        code_top + 96.0,
        pw - 28.0,
        36.0,
        4.0,
        t.accent,
        0.15,
    );

    // bottom panel
    let by = code_bottom;
    s.rect(cx, by, cw, bottom - by, 0.0, t.panel, None);
    s.line(cx, by, cx + cw, by, t.border, 1.0);
    for (i, (name, on)) in [
        ("Check results", true),
        ("Reports", false),
        ("Search results", false),
    ]
    .iter()
    .enumerate()
    {
        s.text(
            cx + 16.0 + i as f64 * 130.0,
            by + 22.0,
            13.0,
            *on,
            if *on { t.fg } else { t.dim },
            "start",
            name,
        );
    }
    for (i, (code, msg)) in [
        (
            "[RSC-007]",
            "OEBPS/text/ch02.xhtml:14  Referenced resource “fig2-9.png” could not be found",
        ),
        (
            "[OPF-003]",
            "OEBPS/styles/stray.css  File is not listed in the manifest",
        ),
    ]
    .iter()
    .enumerate()
    {
        let ry = by + 40.0 + i as f64 * 34.0;
        icon_status(s, cx + 24.0, ry + 14.0, "warn");
        s.text(cx + 46.0, ry + 19.0, 13.0, true, t.fg, "start", code);
        s.text(cx + 130.0, ry + 19.0, 13.0, false, t.fg, "start", msg);
        btn(s, cx + cw - 190.0, ry + 2.0, 60.0, 28.0, "Fix", "flat", t);
        btn(
            s,
            cx + cw - 124.0,
            ry + 2.0,
            108.0,
            28.0,
            "Skip this kind",
            "flat",
            t,
        );
    }
    // status bar
    s.line(x, bottom, x + w, bottom, t.border, 1.0);
    s.text(
        x + 14.0,
        bottom + 19.0,
        12.0,
        false,
        t.dim,
        "start",
        "ch02.xhtml · Ln 11, Col 21 · XHTML · en-US · 1 misspelling · last checkpoint 4 min ago",
    );
    // quick open
    popover_card(
        s,
        x + w / 2.0 - 200.0,
        y + 60.0,
        400.0,
        150.0,
        x + w / 2.0,
        true,
        t,
    );
    entry(s, x + w / 2.0 - 184.0, y + 76.0, 368.0, 32.0, "c2", true, t);
    for (i, l) in ["ch02.xhtml", "ch02-notes.xhtml", "cover.css"]
        .iter()
        .enumerate()
    {
        s.text(
            x + w / 2.0 - 170.0,
            y + 134.0 + i as f64 * 24.0,
            13.0,
            i == 0,
            t.fg,
            "start",
            l,
        );
    }
    s.text(
        x + w / 2.0 + 180.0,
        y + 134.0,
        11.0,
        false,
        t.dim,
        "end",
        "Ctrl+P",
    );
}

/// The Edit (markup) menu and the file-list menu.
fn markup_menus(s: &mut Svg, x: f64, y: f64, t: &Theme) {
    popover_card(s, x, y, 330.0, 360.0, x + 300.0, true, t);
    let items = [
        "Select to Matching Tag            Ctrl+Alt+K",
        "Jump to Matching Tag              Ctrl+Alt+J",
        "Close the Open Tag                Ctrl+Alt+.",
        "-",
        "Wrap Selection in Tag…            Ctrl+Alt+W",
        "Remove Surrounding Tag            Ctrl+Alt+U",
        "Rename Tag…                       Ctrl+Alt+R",
        "-",
        "Change Block to              ▸    Ctrl+1…6",
        "Change Case                  ▸",
    ];
    let mut yy = y + 12.0;
    for it in items {
        if it == "-" {
            s.line(x + 10.0, yy + 5.0, x + 320.0, yy + 5.0, t.border, 1.0);
            yy += 11.0;
        } else {
            s.text(x + 18.0, yy + 22.0, 13.0, false, t.fg, "start", it);
            yy += 34.0;
        }
    }
}

fn files_menu(s: &mut Svg, x: f64, y: f64, t: &Theme) {
    popover_card(s, x, y, 250.0, 330.0, x + 40.0, false, t);
    let items = [
        "New File…                 Ctrl+N",
        "Add Files…",
        "-",
        "Merge Selected…",
        "Bulk Rename…",
        "Link Stylesheets…",
        "Export Selected…",
        "-",
        "Rename…                         F2",
        "Delete Selected            Delete",
    ];
    let mut yy = y + 12.0;
    for it in items {
        if it == "-" {
            s.line(x + 10.0, yy + 5.0, x + 240.0, yy + 5.0, t.border, 1.0);
            yy += 11.0;
        } else {
            s.text(x + 18.0, yy + 22.0, 13.0, false, t.fg, "start", it);
            yy += 30.0;
        }
    }
}

fn generate_dialog(s: &mut Svg, x: f64, y: f64, t: &Theme) {
    s.shadow(x, y, 520.0, 360.0, 16.0, 0.3);
    s.rect(x, y, 520.0, 360.0, 16.0, t.card, Some(t.border));
    s.text(
        x + 260.0,
        y + 44.0,
        20.0,
        true,
        t.fg,
        "middle",
        "Generate Contents",
    );
    s.text(
        x + 260.0,
        y + 68.0,
        13.0,
        false,
        t.dim,
        "middle",
        "Headings, the links of a contents page, or one entry per file.",
    );
    for (i, (l, v)) in [
        ("Source", "From headings ▾"),
        ("Deepest heading level", "3"),
        ("Page whose links are used", "contents.xhtml ▾"),
        ("When a contents exists", "Replace ▾"),
    ]
    .iter()
    .enumerate()
    {
        let ry = y + 96.0 + i as f64 * 44.0;
        s.text(x + 30.0, ry + 24.0, 14.0, false, t.fg, "start", l);
        entry(s, x + 290.0, ry, 200.0, 34.0, v, false, t);
    }
    s.line(x, y + 296.0, x + 520.0, y + 296.0, t.border, 1.0);
    s.line(x + 260.0, y + 296.0, x + 260.0, y + 360.0, t.border, 1.0);
    s.text(x + 130.0, y + 334.0, 15.0, false, t.fg, "middle", "Cancel");
    s.text(
        x + 390.0,
        y + 334.0,
        15.0,
        true,
        t.accent,
        "middle",
        "Generate",
    );
}

fn new_file_dialog(s: &mut Svg, x: f64, y: f64, t: &Theme) {
    s.shadow(x, y, 520.0, 300.0, 16.0, 0.3);
    s.rect(x, y, 520.0, 300.0, 16.0, t.card, Some(t.border));
    s.text(x + 260.0, y + 44.0, 20.0, true, t.fg, "middle", "New File");
    s.text(
        x + 260.0,
        y + 68.0,
        13.0,
        false,
        t.dim,
        "middle",
        "A page is added to the reading order after the open file.",
    );
    entry(s, x + 30.0, y + 90.0, 460.0, 34.0, "interlude", true, t);
    entry(
        s,
        x + 30.0,
        y + 134.0,
        460.0,
        34.0,
        "Chapter with a heading ▾",
        false,
        t,
    );
    s.rect(x + 30.0, y + 184.0, 20.0, 20.0, 5.0, t.accent, None);
    s.text(
        x + 62.0,
        y + 199.0,
        14.0,
        false,
        t.fg,
        "start",
        "Link the book’s stylesheets",
    );
    s.line(x, y + 236.0, x + 520.0, y + 236.0, t.border, 1.0);
    s.line(x + 260.0, y + 236.0, x + 260.0, y + 300.0, t.border, 1.0);
    s.text(x + 130.0, y + 274.0, 15.0, false, t.fg, "middle", "Cancel");
    s.text(
        x + 390.0,
        y + 274.0,
        15.0,
        true,
        t.accent,
        "middle",
        "Create",
    );
}

pub fn run(s: &mut Svg) {
    let l = &LIGHT;
    let d = &DARK;
    s.text(90.0, 130.0, 74.0, true, "#2e3436", "start", "Bookx Editor");
    s.lines(90.0, 180.0, 17.0, 24.0, "#5e5c64", &[
        "A Calibre-class EPUB editor, rewritten in Rust: browse every file in the book, edit XHTML / CSS / OPF / NCX with syntax highlighting and live preview,",
        "find & replace across the whole book, check it for problems, edit the table of contents, spine order and metadata — then save straight back into the library.",
    ]);

    label(
        s,
        90.0,
        300.0,
        "3.a Editor — Files · code editor with line numbers and minimap · live preview",
    );
    editor_window(
        s,
        90.0,
        320.0,
        1780.0,
        960.0,
        d,
        &E {
            tab: "Files",
            find: false,
            panel: None,
            image: false,
        },
    );
    note(s, 90.0, 1316.0, &[
        "Opens in its own window from the library (Edit Book…, or Ctrl+E on a selection). Changes stay in memory until Save (Ctrl+S); closing with unsaved changes asks first.",
        "Files: grouped by type, with the reading-order number, an accent dot on edited files, add / rename (updates links) / delete / move up-down in the spine.",
    ]);

    label(s, 1930.0, 300.0, "3.b Contents — table-of-contents editor");
    editor_window(
        s,
        1930.0,
        320.0,
        1180.0,
        960.0,
        l,
        &E {
            tab: "Contents",
            find: false,
            panel: None,
            image: false,
        },
    );
    note(
        s,
        1930.0,
        1316.0,
        &[
            "Edit titles inline; indent / outdent / reorder; point an entry at the open file;",
            "or Generate the whole TOC from h1–h3 headings. Writes both toc.ncx and nav.xhtml.",
        ],
    );

    label(
        s,
        90.0,
        1470.0,
        "3.c Find & Replace — regular expressions, this file or all text files",
    );
    editor_window(
        s,
        90.0,
        1490.0,
        1780.0,
        900.0,
        d,
        &E {
            tab: "Files",
            find: true,
            panel: Some("find"),
            image: false,
        },
    );
    note(s, 90.0, 2426.0, &[
        "Ctrl+H / Ctrl+F toggles the bar. F3 / Shift+F3 step through matches. Replace, Replace All and Find All; Find All lists every hit and jumps to it.",
        "Replacements are single undo steps (Ctrl+Z). In regex mode $1, $2 … refer to capture groups.",
    ]);

    label(
        s,
        1930.0,
        1470.0,
        "3.d Check book (F7) — structural problems, with jump-to-line",
    );
    editor_window(
        s,
        1930.0,
        1490.0,
        1180.0,
        900.0,
        l,
        &E {
            tab: "Files",
            find: false,
            panel: Some("check"),
            image: false,
        },
    );
    note(
        s,
        1930.0,
        2426.0,
        &[
            "XML well-formedness, missing or unmanifested files, duplicate ids, spine errors,",
            "broken links and anchors, missing title / language / identifier / cover.",
        ],
    );

    label(s, 90.0, 2570.0, "3.e Images and metadata");
    editor_window(
        s,
        90.0,
        2590.0,
        1480.0,
        760.0,
        d,
        &E {
            tab: "Metadata",
            find: false,
            panel: None,
            image: true,
        },
    );
    note(
        s,
        90.0,
        3386.0,
        &[
            "Select an image to see it; “Use as Cover” rewrites the cover entry in the package.",
            "The Metadata tab opens the same Edit Metadata dialog as the library.",
        ],
    );

    label(s, 1640.0, 2570.0, "3.f Dialogs");
    alert(
        s,
        1640.0,
        2610.0,
        560.0,
        "Save Changes?",
        &[
            "Your changes to this book will be lost",
            "if you don’t save them.",
        ],
        "Cancel",
        "Save",
        false,
        l,
    );
    s.text(
        1640.0 + 140.0,
        2810.0,
        13.0,
        false,
        "#5e5c64",
        "middle",
        "Cancel · Discard · Save",
    );
    alert(
        s,
        2260.0,
        2610.0,
        560.0,
        "Delete File?",
        &[
            "“fig2-9.png” will be removed from the book.",
            "Links to it are not updated.",
        ],
        "Cancel",
        "Delete",
        true,
        l,
    );
    {
        let (x, y) = (1640.0, 2870.0);
        s.shadow(x, y, 560.0, 230.0, 16.0, 0.3);
        s.rect(x, y, 560.0, 230.0, 16.0, l.card, Some(l.border));
        s.text(
            x + 280.0,
            y + 44.0,
            20.0,
            true,
            l.fg,
            "middle",
            "Rename File",
        );
        s.text(
            x + 280.0,
            y + 70.0,
            13.0,
            false,
            l.dim,
            "middle",
            "References to the file in other chapters are updated.",
        );
        entry(s, x + 30.0, y + 92.0, 500.0, 40.0, "fig2-1.png", false, l);
        s.line(x, y + 168.0, x + 560.0, y + 168.0, l.border, 1.0);
        s.line(x + 280.0, y + 168.0, x + 280.0, y + 230.0, l.border, 1.0);
        s.text(x + 140.0, y + 206.0, 15.0, false, l.fg, "middle", "Cancel");
        s.text(
            x + 420.0,
            y + 206.0,
            15.0,
            true,
            l.accent,
            "middle",
            "Rename",
        );
    }
    note(
        s,
        2260.0,
        2890.0,
        &[
            "Add files… copies images, fonts, CSS or",
            "chapters into the right folder and",
            "registers them in the manifest/spine.",
            "",
            "Reformat (¶) pretty-prints XHTML / OPF /",
            "NCX without touching reading text.",
        ],
    );
    label(
        s,
        90.0,
        3560.0,
        "3.g Tools menu — one-click operations, each preceded by an automatic checkpoint",
    );
    editor_window(
        s,
        90.0,
        3580.0,
        1300.0,
        720.0,
        d,
        &E {
            tab: "Files",
            find: false,
            panel: None,
            image: false,
        },
    );
    tools_menu(s, 90.0 + 1300.0 - 172.0 + 17.0 - 290.0, 3580.0 + 52.0, d);
    note(
        s,
        90.0,
        4336.0,
        &[
            "Fix problems, upgrade to EPUB 3, add or remove an inline contents page, improve accessibility, compress images, shrink fonts and",
            "arrange files in folders, split a file at its chapter headings or merge it with the next. Check Spelling lists likely typos; Book Report shows the numbers; checkpoints can be restored or compared with the open book.",
        ],
    );
    label(s, 1450.0, 3560.0, "3.h Book Report");
    book_report(s, 1450.0, 3590.0, l);
    label(s, 2030.0, 3560.0, "3.i Nothing open yet");
    editor_window(
        s,
        2030.0,
        3580.0,
        1080.0,
        720.0,
        l,
        &E {
            tab: "Files",
            find: false,
            panel: Some("empty"),
            image: false,
        },
    );
    note(
        s,
        2030.0,
        4336.0,
        &["The editor opens on a status page; Save stays disabled until something changed."],
    );
    label(
        s,
        90.0,
        4440.0,
        "3.j Check Spelling (Tools menu) — misspellings with a suggestion and file:line",
    );
    editor_window(
        s,
        90.0,
        4460.0,
        1500.0,
        760.0,
        l,
        &E {
            tab: "Files",
            find: false,
            panel: Some("spelling"),
            image: false,
        },
    );
    note(
        s,
        90.0,
        5256.0,
        &[
            "Uses the system Hunspell dictionary for the book’s language. Click a result to jump to the word.",
        ],
    );
    label(
        s,
        90.0,
        5290.0,
        "3.k Preview colours — palette button next to “Preview”: Theme · Original · Custom",
    );
    for (i, mode) in [Colors::Original, Colors::Custom].iter().enumerate() {
        let (wx, wy, ww) = (90.0 + i as f64 * 1560.0, 5310.0, 1480.0);
        editor_window(
            s,
            wx,
            wy,
            ww,
            720.0,
            l,
            &E {
                tab: "Files",
                find: false,
                panel: None,
                image: false,
            },
        );
        let px = wx + ww - 430.0;
        s.rect_a(px + 72.0, wy + 46.0 + 4.0, 24.0, 24.0, 6.0, l.border, 0.8);
        icon_palette(s, px + 84.0, wy + 46.0 + 17.0, l.fg);
        preview_colors_popover(s, px + 14.0, wy + 46.0 + 36.0, *mode, l);
    }
    note(
        s,
        90.0,
        6066.0,
        &[
            "The preview pane shows the chapter the way the reader will: the book’s own colours (Original) or your own text and page colours (Custom).",
        ],
    );
    label(
        s,
        90.0,
        6150.0,
        "3.l Target layout — tabs · sidebar panels · search bar with live count · gutter marks · spelling fix menu · bottom panel · status bar · Ctrl+P",
    );
    design_window(s, 90.0, 6170.0, 2020.0, 1120.0, l);
    note(
        s,
        90.0,
        7326.0,
        &[
            "Inspired by GNOME Builder’s layout; no Builder code is used (docs/editor-ui-design.md). Green gutter bars: changed since the last checkpoint; amber: a Check finding.",
            "Files, Outline (contents), Search and Metadata share the sidebar; Check results, Reports and Search results share the bottom panel. Implemented so far: multi-select files, menus, dialogs on the right.",
        ],
    );
    label(s, 2180.0, 6150.0, "3.m Markup menu · file-list menu");
    markup_menus(s, 2180.0, 6190.0, l);
    files_menu(s, 2180.0, 6590.0, l);
    label(s, 2560.0, 6150.0, "3.n Generate Contents · New File");
    generate_dialog(s, 2560.0, 6190.0, l);
    new_file_dialog(s, 2560.0, 6580.0, l);
    note(
        s,
        2180.0,
        7000.0,
        &[
            "Right-click in the file list opens the same menu as the ⋯ button.",
            "Every whole-book action first takes a checkpoint named “Before: …”.",
        ],
    );
    s.text(
        1600.0,
        7560.0,
        12.0,
        false,
        "#77767b",
        "middle",
        "Mockups are design illustrations.",
    );
}
