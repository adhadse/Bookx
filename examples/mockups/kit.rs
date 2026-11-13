//! Tiny SVG drawing kit used to author the Bookx design mockups in Rust.
#![allow(dead_code)]

use std::fmt::Write;

pub struct Svg {
    pub w: f64,
    pub h: f64,
    pub body: String,
    defs: String,
    uid: usize,
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub bg: &'static str,
    pub panel: &'static str,
    pub header: &'static str,
    pub card: &'static str,
    pub fg: &'static str,
    pub dim: &'static str,
    pub border: &'static str,
    pub accent: &'static str,
    pub hover: &'static str,
    pub shadow: f64,
}

pub const LIGHT: Theme = Theme {
    bg: "#fafafa",
    panel: "#f1f1f1",
    header: "#ebebeb",
    card: "#ffffff",
    fg: "#2e3436",
    dim: "#77767b",
    border: "#d6d6d6",
    accent: "#3584e4",
    hover: "#e4e4e4",
    shadow: 0.18,
};

pub const DARK: Theme = Theme {
    bg: "#242424",
    panel: "#2b2b2e",
    header: "#303033",
    card: "#383838",
    fg: "#f2f2f2",
    dim: "#9a9996",
    border: "#444448",
    accent: "#3584e4",
    hover: "#3d3d41",
    shadow: 0.5,
};

pub const SEPIA: Theme = Theme {
    bg: "#f4ecd8",
    panel: "#ebe1c8",
    header: "#ebe1c8",
    card: "#fbf5e6",
    fg: "#433522",
    dim: "#8a7a62",
    border: "#d9ccaa",
    accent: "#b8742a",
    hover: "#e3d7b9",
    shadow: 0.15,
};

/// E-ink mode: pure black on white, hard 2 px edges, no shadows, no colour.
pub const EINK: Theme = Theme {
    bg: "#ffffff",
    panel: "#ffffff",
    header: "#ffffff",
    card: "#ffffff",
    fg: "#000000",
    dim: "#333333",
    border: "#000000",
    accent: "#000000",
    hover: "#e6e6e6",
    shadow: 0.0,
};

pub const COVERS: [(&str, &str); 8] = [
    ("#3584e4", "#1c4f94"),
    ("#e66100", "#a34100"),
    ("#26a269", "#17633f"),
    ("#9141ac", "#5b2670"),
    ("#c64600", "#7a2b00"),
    ("#1a5fb4", "#0f3667"),
    ("#a51d2d", "#67121c"),
    ("#5e5c64", "#34333a"),
];

pub const HL: [&str; 5] = ["#ffd91f", "#b873f2", "#408cff", "#ff6152", "#33cc6b"];

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

impl Svg {
    pub fn new(w: f64, h: f64) -> Svg {
        Svg {
            w,
            h,
            body: String::new(),
            defs: String::new(),
            uid: 0,
        }
    }

    pub fn raw(&mut self, s: &str) {
        self.body.push_str(s);
        self.body.push('\n');
    }

    pub fn rect(
        &mut self,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        r: f64,
        fill: &str,
        stroke: Option<&str>,
    ) {
        let st = stroke
            .map(|s| format!(" stroke=\"{s}\" stroke-width=\"1.5\""))
            .unwrap_or_default();
        let _ = writeln!(self.body, "<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" rx=\"{r}\" fill=\"{fill}\"{st}/>");
    }

    pub fn rect_a(&mut self, x: f64, y: f64, w: f64, h: f64, r: f64, fill: &str, a: f64) {
        let _ = writeln!(self.body, "<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" rx=\"{r}\" fill=\"{fill}\" fill-opacity=\"{a}\"/>");
    }

    pub fn circle(&mut self, cx: f64, cy: f64, r: f64, fill: &str) {
        let _ = writeln!(
            self.body,
            "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"{r}\" fill=\"{fill}\"/>"
        );
    }

    pub fn ring(&mut self, cx: f64, cy: f64, r: f64, stroke: &str, w: f64) {
        let _ = writeln!(self.body, "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"{r}\" fill=\"none\" stroke=\"{stroke}\" stroke-width=\"{w}\"/>");
    }

    pub fn line(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, stroke: &str, w: f64) {
        let _ = writeln!(self.body, "<line x1=\"{x1}\" y1=\"{y1}\" x2=\"{x2}\" y2=\"{y2}\" stroke=\"{stroke}\" stroke-width=\"{w}\" stroke-linecap=\"round\"/>");
    }

    pub fn path(&mut self, d: &str, fill: &str, stroke: &str, w: f64) {
        let _ = writeln!(self.body, "<path d=\"{d}\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"{w}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>");
    }

    pub fn text(
        &mut self,
        x: f64,
        y: f64,
        size: f64,
        bold: bool,
        fill: &str,
        anchor: &str,
        s: &str,
    ) {
        let wgt = if bold { "700" } else { "400" };
        let _ = writeln!(
            self.body,
            "<text x=\"{x}\" y=\"{y}\" font-size=\"{size}\" font-weight=\"{wgt}\" fill=\"{fill}\" text-anchor=\"{anchor}\" font-family=\"Cantarell, Inter, Noto Sans, sans-serif\">{}</text>",
            esc(s)
        );
    }

    pub fn mono(&mut self, x: f64, y: f64, size: f64, fill: &str, s: &str) {
        let _ = writeln!(
            self.body,
            "<text x=\"{x}\" y=\"{y}\" font-size=\"{size}\" fill=\"{fill}\" xml:space=\"preserve\" font-family=\"Noto Sans Mono, DejaVu Sans Mono, monospace\">{}</text>",
            esc(s)
        );
    }

    pub fn serif(&mut self, x: f64, y: f64, size: f64, bold: bool, fill: &str, s: &str) {
        let wgt = if bold { "700" } else { "400" };
        let _ = writeln!(
            self.body,
            "<text x=\"{x}\" y=\"{y}\" font-size=\"{size}\" font-weight=\"{wgt}\" fill=\"{fill}\" font-family=\"Noto Serif, Liberation Serif, serif\">{}</text>",
            esc(s)
        );
    }

    /// Serif text stretched to exactly `width` (a justified line of a book page).
    pub fn serif_fit(&mut self, x: f64, y: f64, size: f64, fill: &str, width: f64, s: &str) {
        let _ = writeln!(
            self.body,
            "<text x=\"{x}\" y=\"{y}\" font-size=\"{size}\" fill=\"{fill}\" textLength=\"{width}\" lengthAdjust=\"spacing\" xml:space=\"preserve\" font-family=\"Noto Serif, Liberation Serif, serif\">{}</text>",
            esc(s)
        );
    }

    /// Multi-line text, `lh` is the line height.
    pub fn lines(&mut self, x: f64, y: f64, size: f64, lh: f64, fill: &str, lines: &[&str]) {
        for (i, l) in lines.iter().enumerate() {
            self.text(x, y + lh * i as f64, size, false, fill, "start", l);
        }
    }

    pub fn gradient(&mut self, c1: &str, c2: &str) -> String {
        self.uid += 1;
        let id = format!("g{}", self.uid);
        let _ = writeln!(
            self.defs,
            "<linearGradient id=\"{id}\" x1=\"0\" y1=\"0\" x2=\"0.6\" y2=\"1\"><stop offset=\"0\" stop-color=\"{c1}\"/><stop offset=\"1\" stop-color=\"{c2}\"/></linearGradient>"
        );
        format!("url(#{id})")
    }

    pub fn shadow(&mut self, x: f64, y: f64, w: f64, h: f64, r: f64, a: f64) {
        for i in 0..6 {
            let g = i as f64 * 3.0;
            let _ = writeln!(
                self.body,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" fill=\"#000\" fill-opacity=\"{}\"/>",
                x - g,
                y - g + 8.0,
                w + 2.0 * g,
                h + 2.0 * g,
                r + g,
                a / 14.0
            );
        }
    }

    pub fn finish(self) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\n<defs>\n{defs}</defs>\n<rect width=\"{w}\" height=\"{h}\" fill=\"#ffffff\"/>\n{body}</svg>\n",
            w = self.w,
            h = self.h,
            defs = self.defs,
            body = self.body
        )
    }
}

// ---------------------------------------------------------------------------
// Widgets
// ---------------------------------------------------------------------------

/// A window with an (optional) headerbar strip. Returns the y coordinate below the header.
pub fn window(s: &mut Svg, x: f64, y: f64, w: f64, h: f64, t: &Theme, header: bool) -> f64 {
    s.shadow(x, y, w, h, 14.0, t.shadow);
    s.rect(x, y, w, h, 14.0, t.bg, Some(t.border));
    if header {
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
        // window close
        s.circle(x + w - 26.0, y + 23.0, 12.0, t.hover);
        icon_close(s, x + w - 26.0, y + 23.0, t.fg);
        y + 46.0
    } else {
        y
    }
}

pub fn btn(s: &mut Svg, x: f64, y: f64, w: f64, h: f64, label: &str, kind: &str, t: &Theme) {
    let (fill, fg) = match kind {
        "suggested" => (t.accent, "#ffffff"),
        "destructive" => ("#e01b24", "#ffffff"),
        "flat" => ("none", t.fg),
        _ => (t.hover, t.fg),
    };
    let r = if kind == "pill" { h / 2.0 } else { 8.0 };
    s.rect(x, y, w, h, r, fill, None);
    s.text(
        x + w / 2.0,
        y + h / 2.0 + 5.0,
        14.0,
        kind == "suggested",
        fg,
        "middle",
        label,
    );
}

pub fn entry(
    s: &mut Svg,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    text: &str,
    placeholder: bool,
    t: &Theme,
) {
    s.rect(x, y, w, h, 8.0, t.hover, None);
    s.text(
        x + 12.0,
        y + h / 2.0 + 5.0,
        14.0,
        false,
        if placeholder { t.dim } else { t.fg },
        "start",
        text,
    );
}

pub fn switch(s: &mut Svg, x: f64, y: f64, on: bool, t: &Theme) {
    s.rect(
        x,
        y,
        46.0,
        26.0,
        13.0,
        if on { t.accent } else { t.border },
        None,
    );
    s.circle(
        if on { x + 33.0 } else { x + 13.0 },
        y + 13.0,
        10.0,
        "#ffffff",
    );
}

pub fn slider(s: &mut Svg, x: f64, y: f64, w: f64, frac: f64, t: &Theme) {
    s.rect(x, y - 2.0, w, 4.0, 2.0, t.border, None);
    s.rect(x, y - 2.0, w * frac, 4.0, 2.0, t.accent, None);
    s.circle(x + w * frac, y, 8.0, "#ffffff");
    s.ring(x + w * frac, y, 8.0, t.border, 1.0);
}

pub fn stars(s: &mut Svg, x: f64, y: f64, filled: usize, size: f64) {
    for i in 0..5 {
        let cx = x + i as f64 * (size + 4.0) + size / 2.0;
        let fill = if i < filled { "#f5c211" } else { "none" };
        let stroke = if i < filled { "#f5c211" } else { "#9a9996" };
        let r = size / 2.0;
        let mut d = String::new();
        for k in 0..10 {
            let ang = -std::f64::consts::FRAC_PI_2 + k as f64 * std::f64::consts::PI / 5.0;
            let rr = if k % 2 == 0 { r } else { r * 0.45 };
            let _ = write!(
                d,
                "{}{:.1},{:.1}",
                if k == 0 { "M" } else { "L" },
                cx + rr * ang.cos(),
                y + r + rr * ang.sin()
            );
        }
        d.push('Z');
        s.path(&d, fill, stroke, 1.3);
    }
}

pub fn cover(
    s: &mut Svg,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    seed: usize,
    title: &str,
    author: &str,
    progress: Option<f64>,
    done: bool,
) {
    let (c1, c2) = COVERS[seed % 8];
    s.shadow(x + 2.0, y + 1.0, w - 4.0, h - 4.0, 5.0, 0.25);
    let g = s.gradient(c1, c2);
    s.rect(x, y, w, h, 5.0, &g, None);
    s.rect_a(x + 8.0, y + 8.0, 6.0, h - 16.0, 2.0, "#ffffff", 0.12);
    let words: Vec<&str> = title.split(' ').collect();
    let mut yy = y + h * 0.3;
    for wd in words {
        s.text(
            x + w / 2.0,
            yy,
            (w / 7.5).min(17.0),
            true,
            "#ffffff",
            "middle",
            wd,
        );
        yy += (w / 7.5).min(17.0) + 4.0;
    }
    s.text(
        x + w / 2.0,
        y + h - 14.0,
        10.0,
        false,
        "#ffffffcc",
        "middle",
        author,
    );
    if let Some(p) = progress {
        s.rect_a(x, y + h - 4.0, w, 4.0, 0.0, "#000000", 0.4);
        s.rect(x, y + h - 4.0, w * p, 4.0, 0.0, "#3584e4", None);
    }
    if done {
        s.circle(x + w - 14.0, y + 14.0, 9.0, "#3584e4");
        s.path(
            &format!("M{},{} l3,3 l6,-6", x + w - 18.0, y + 14.0),
            "none",
            "#fff",
            2.0,
        );
    }
}

/// Greeked text: `n` lines of grey bars.
pub fn greek(s: &mut Svg, x: f64, y: f64, w: f64, n: usize, lh: f64, fill: &str, seed: usize) {
    for i in 0..n {
        let last = i == n - 1;
        let k = ((i * 7 + seed * 13) % 11) as f64 / 40.0;
        let lw = if last {
            w * (0.35 + k)
        } else {
            w * (0.96 + (k / 6.0)).min(1.0)
        };
        s.rect_a(x, y + i as f64 * lh, lw, lh * 0.34, 2.0, fill, 0.42);
    }
}

pub fn label(s: &mut Svg, x: f64, y: f64, text: &str) {
    s.text(x, y, 17.0, true, "#2e3436", "start", text);
}

pub fn note(s: &mut Svg, x: f64, y: f64, lines: &[&str]) {
    s.lines(x, y, 14.0, 20.0, "#5e5c64", lines);
}

pub fn menu(s: &mut Svg, x: f64, y: f64, w: f64, items: &[&str], t: &Theme) -> f64 {
    let mut h = 12.0;
    for it in items {
        h += if *it == "-" { 12.0 } else { 34.0 };
    }
    s.shadow(x, y, w, h, 12.0, 0.25);
    s.rect(x, y, w, h, 12.0, t.card, Some(t.border));
    let mut yy = y + 6.0;
    for it in items {
        if *it == "-" {
            s.line(x + 8.0, yy + 6.0, x + w - 8.0, yy + 6.0, t.border, 1.0);
            yy += 12.0;
            continue;
        }
        let danger = it.starts_with('!');
        let txt = it.trim_start_matches('!');
        s.text(
            x + 16.0,
            yy + 22.0,
            14.0,
            false,
            if danger { "#e01b24" } else { t.fg },
            "start",
            txt,
        );
        if let Some(i) = txt.find('▸') {
            let _ = i;
        }
        yy += 34.0;
    }
    h
}

pub fn tip(s: &mut Svg, x: f64, y: f64, text: &str) {
    let w = text.chars().count() as f64 * 7.6 + 24.0;
    s.rect(x, y, w, 30.0, 8.0, "#2e3436", None);
    s.text(x + 12.0, y + 20.0, 13.0, false, "#ffffff", "start", text);
}

pub fn toast(s: &mut Svg, cx: f64, y: f64, text: &str, action: Option<&str>) {
    let w = text.chars().count() as f64 * 8.2
        + 60.0
        + action.map(|a| a.len() as f64 * 9.0 + 20.0).unwrap_or(0.0);
    let x = cx - w / 2.0;
    s.rect(x, y, w, 44.0, 22.0, "#3d3d3d", None);
    s.text(x + 22.0, y + 28.0, 14.0, true, "#ffffff", "start", text);
    if let Some(a) = action {
        s.text(
            x + w - 54.0 - a.len() as f64 * 8.0,
            y + 28.0,
            14.0,
            true,
            "#99c1f1",
            "start",
            a,
        );
    }
    icon_close(s, x + w - 24.0, y + 22.0, "#ffffff");
}

pub fn alert(
    s: &mut Svg,
    x: f64,
    y: f64,
    w: f64,
    title: &str,
    body: &[&str],
    cancel: &str,
    ok: &str,
    destructive: bool,
    t: &Theme,
) {
    let h = 130.0 + body.len() as f64 * 20.0;
    s.shadow(x, y, w, h, 16.0, 0.3);
    s.rect(x, y, w, h, 16.0, t.card, Some(t.border));
    s.text(x + w / 2.0, y + 44.0, 20.0, true, t.fg, "middle", title);
    for (i, l) in body.iter().enumerate() {
        s.text(
            x + w / 2.0,
            y + 74.0 + 20.0 * i as f64,
            14.0,
            false,
            t.dim,
            "middle",
            l,
        );
    }
    let by = y + h - 50.0;
    s.line(x, by - 8.0, x + w, by - 8.0, t.border, 1.0);
    s.line(x + w / 2.0, by - 8.0, x + w / 2.0, y + h, t.border, 1.0);
    s.text(x + w / 4.0, by + 24.0, 15.0, false, t.fg, "middle", cancel);
    s.text(
        x + 3.0 * w / 4.0,
        by + 24.0,
        15.0,
        true,
        if destructive { "#e01b24" } else { t.accent },
        "middle",
        ok,
    );
}

pub fn pref_row(s: &mut Svg, x: f64, y: f64, w: f64, h: f64, left: &str, right: &str, t: &Theme) {
    s.rect(x, y, w, h, 0.0, t.card, None);
    s.line(x, y + h, x + w, y + h, t.border, 1.0);
    s.text(
        x + 16.0,
        y + h / 2.0 + 5.0,
        14.0,
        false,
        t.fg,
        "start",
        left,
    );
    s.text(
        x + w - 16.0,
        y + h / 2.0 + 5.0,
        14.0,
        false,
        t.dim,
        "end",
        right,
    );
}

pub fn row(s: &mut Svg, x: f64, y: f64, w: f64, h: f64, title: &str, sub: Option<&str>, t: &Theme) {
    s.rect(x, y, w, h, 0.0, t.card, None);
    s.line(x, y + h, x + w, y + h, t.border, 1.0);
    if let Some(sub) = sub {
        s.text(x + 16.0, y + 20.0, 11.0, false, t.dim, "start", title);
        s.text(x + 16.0, y + 40.0, 14.0, false, t.fg, "start", sub);
    } else {
        s.text(
            x + 16.0,
            y + h / 2.0 + 5.0,
            14.0,
            false,
            t.fg,
            "start",
            title,
        );
    }
}

// ---------------------------------------------------------------------------
// Icons (centered on cx, cy, ~16 px)
// ---------------------------------------------------------------------------

pub fn icon_close(s: &mut Svg, cx: f64, cy: f64, c: &str) {
    s.path(
        &format!("M{},{} l8,8 m0,-8 l-8,8", cx - 4.0, cy - 4.0),
        "none",
        c,
        1.8,
    );
}
pub fn icon_menu(s: &mut Svg, cx: f64, cy: f64, c: &str) {
    for d in [-5.0, 0.0, 5.0] {
        s.line(cx - 7.0, cy + d, cx + 7.0, cy + d, c, 1.8);
    }
}
pub fn icon_plus(s: &mut Svg, cx: f64, cy: f64, c: &str) {
    s.line(cx - 7.0, cy, cx + 7.0, cy, c, 2.0);
    s.line(cx, cy - 7.0, cx, cy + 7.0, c, 2.0);
}
pub fn icon_search(s: &mut Svg, cx: f64, cy: f64, c: &str) {
    s.ring(cx - 1.5, cy - 1.5, 5.5, c, 1.8);
    s.line(cx + 2.5, cy + 2.5, cx + 7.0, cy + 7.0, c, 1.8);
}
pub fn icon_sidebar(s: &mut Svg, cx: f64, cy: f64, c: &str, right: bool) {
    s.rect(cx - 8.0, cy - 7.0, 16.0, 14.0, 3.0, "none", Some(c));
    let x = if right { cx + 2.0 } else { cx - 3.0 };
    s.line(x, cy - 7.0, x, cy + 7.0, c, 1.6);
}
pub fn icon_grid(s: &mut Svg, cx: f64, cy: f64, c: &str) {
    for (dx, dy) in [(-7.0, -7.0), (1.0, -7.0), (-7.0, 1.0), (1.0, 1.0)] {
        s.rect(cx + dx, cy + dy, 6.0, 6.0, 1.5, c, None);
    }
}
pub fn icon_list(s: &mut Svg, cx: f64, cy: f64, c: &str) {
    for d in [-5.0, 0.0, 5.0] {
        s.circle(cx - 6.0, cy + d, 1.3, c);
        s.line(cx - 2.0, cy + d, cx + 7.0, cy + d, c, 1.8);
    }
}
pub fn icon_dots(s: &mut Svg, cx: f64, cy: f64, c: &str) {
    for d in [-6.0, 0.0, 6.0] {
        s.circle(cx + d, cy, 1.8, c);
    }
}
pub fn icon_chev(s: &mut Svg, cx: f64, cy: f64, c: &str, dir: char) {
    let d = match dir {
        'r' => "m-2,-5 l5,5 l-5,5",
        'l' => "m2,-5 l-5,5 l5,5",
        'd' => "m-5,-2 l5,5 l5,-5",
        _ => "m-5,2 l5,-5 l5,5",
    };
    s.path(&format!("M{cx},{cy} {d}"), "none", c, 1.8);
}
pub fn icon_bookmark(s: &mut Svg, cx: f64, cy: f64, c: &str, filled: bool) {
    s.path(
        &format!("M{},{} h10 v14 l-5,-4 l-5,4 z", cx - 5.0, cy - 7.0),
        if filled { c } else { "none" },
        c,
        1.6,
    );
}
pub fn icon_aa(s: &mut Svg, cx: f64, cy: f64, c: &str) {
    s.text(cx, cy + 6.0, 17.0, true, c, "middle", "A");
}
pub fn icon_full(s: &mut Svg, cx: f64, cy: f64, c: &str) {
    s.path(
        &format!(
            "M{},{} h-6 v6 M{},{} h6 v-6",
            cx - 1.0,
            cy - 7.0,
            cx + 1.0,
            cy + 7.0
        ),
        "none",
        c,
        1.6,
    );
    s.path(
        &format!(
            "M{},{} h6 v6 M{},{} h-6 v-6",
            cx + 1.0,
            cy - 7.0,
            cx - 1.0,
            cy + 7.0
        ),
        "none",
        c,
        1.6,
    );
}
pub fn icon_undo(s: &mut Svg, cx: f64, cy: f64, c: &str, flip: bool) {
    let sgn = if flip { -1.0 } else { 1.0 };
    s.path(
        &format!(
            "M{},{} h{} a5,5 0 0 1 0,10 h-6 M{},{} l{},-4 m{},4 l{},4",
            cx - 6.0 * sgn,
            cy - 2.0,
            10.0 * sgn,
            cx - 6.0 * sgn,
            cy - 2.0,
            4.0 * sgn,
            -4.0 * sgn,
            4.0 * sgn
        ),
        "none",
        c,
        1.6,
    );
}
pub fn icon_dot(s: &mut Svg, cx: f64, cy: f64, c: &str) {
    s.circle(cx, cy, 5.0, c);
}
pub fn icon_file(s: &mut Svg, cx: f64, cy: f64, c: &str) {
    s.path(
        &format!("M{},{} h7 l4,4 v11 h-11 z", cx - 5.5, cy - 8.0),
        "none",
        c,
        1.5,
    );
}
pub fn icon_folder(s: &mut Svg, cx: f64, cy: f64, c: &str) {
    s.path(
        &format!("M{},{} h5 l2,2 h6 v9 h-13 z", cx - 7.0, cy - 6.0),
        "none",
        c,
        1.5,
    );
}
pub fn icon_img(s: &mut Svg, cx: f64, cy: f64, c: &str) {
    s.rect(cx - 7.0, cy - 6.0, 14.0, 12.0, 2.0, "none", Some(c));
    s.path(
        &format!("M{},{} l4,-4 l3,3 l2,-2 l3,3", cx - 7.0, cy + 5.0),
        "none",
        c,
        1.4,
    );
}
pub fn icon_trash(s: &mut Svg, cx: f64, cy: f64, c: &str) {
    s.path(
        &format!(
            "M{},{} h12 m-10,0 v10 h8 v-10 m-6,-3 h4",
            cx - 6.0,
            cy - 4.0
        ),
        "none",
        c,
        1.5,
    );
}
pub fn icon_pencil(s: &mut Svg, cx: f64, cy: f64, c: &str) {
    s.path(
        &format!("M{},{} l9,-9 l3,3 l-9,9 h-3 z", cx - 6.0, cy + 6.0),
        "none",
        c,
        1.5,
    );
}
pub fn icon_status(s: &mut Svg, cx: f64, cy: f64, kind: &str) {
    match kind {
        "error" => {
            s.circle(cx, cy, 8.0, "#e01b24");
            icon_close(s, cx, cy, "#fff");
        }
        "warn" => {
            s.path(
                &format!("M{},{} l8,14 h-16 z", cx, cy - 8.0),
                "#f5c211",
                "#f5c211",
                1.0,
            );
            s.text(cx, cy + 5.0, 11.0, true, "#2e3436", "middle", "!");
        }
        _ => {
            s.circle(cx, cy, 8.0, "#3584e4");
            s.text(cx, cy + 4.5, 12.0, true, "#fff", "middle", "i");
        }
    }
}
pub fn icon_user(s: &mut Svg, cx: f64, cy: f64, c: &str) {
    s.circle(cx, cy - 3.0, 3.5, c);
    s.path(
        &format!("M{},{} a6,5 0 0 1 12,0 z", cx - 6.0, cy + 7.0),
        c,
        c,
        1.0,
    );
}
pub fn icon_play(s: &mut Svg, cx: f64, cy: f64, c: &str) {
    s.path(
        &format!("M{},{} l10,6 l-10,6 z", cx - 4.0, cy - 6.0),
        c,
        c,
        1.0,
    );
}

// ---------------------------------------------------------------------------
// Adwaita-style composite widgets shared by the dialog sheets
// ---------------------------------------------------------------------------

/// A libadwaita dialog: rounded card with a header (title centred, close button at the right).
/// Returns the y coordinate where the content starts.
pub fn adw_dialog(s: &mut Svg, x: f64, y: f64, w: f64, h: f64, title: &str, t: &Theme) -> f64 {
    s.shadow(x, y, w, h, 18.0, 0.35);
    s.rect(x, y, w, h, 18.0, t.bg, Some(t.border));
    s.text(x + w / 2.0, y + 33.0, 16.0, true, t.fg, "middle", title);
    s.circle(x + w - 28.0, y + 28.0, 12.0, t.hover);
    icon_close(s, x + w - 28.0, y + 28.0, t.fg);
    y + 56.0
}

/// A group of rows in a card (`AdwPreferencesGroup`), with an optional title above it.
/// Returns the y coordinate below the group.
pub fn group_card(
    s: &mut Svg,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    title: Option<&str>,
    t: &Theme,
) -> f64 {
    let mut y = y;
    if let Some(title) = title {
        s.text(x + 4.0, y + 14.0, 14.0, true, t.fg, "start", title);
        y += 26.0;
    }
    s.rect(x, y, w, h, 12.0, t.card, Some(t.border));
    y + h
}

/// Separator line between two rows of a group card.
pub fn row_sep(s: &mut Svg, x: f64, y: f64, w: f64, t: &Theme) {
    s.line(x, y, x + w, y, t.border, 1.0);
}

/// An `AdwActionRow`-like row inside a group card: title (and optional subtitle) on the left.
pub fn card_row(s: &mut Svg, x: f64, y: f64, h: f64, title: &str, sub: Option<&str>, t: &Theme) {
    match sub {
        Some(sub) => {
            s.text(
                x + 16.0,
                y + h / 2.0 - 3.0,
                14.0,
                false,
                t.fg,
                "start",
                title,
            );
            s.text(
                x + 16.0,
                y + h / 2.0 + 15.0,
                12.0,
                false,
                t.dim,
                "start",
                sub,
            );
        }
        None => s.text(
            x + 16.0,
            y + h / 2.0 + 5.0,
            14.0,
            false,
            t.fg,
            "start",
            title,
        ),
    }
}

/// A value on the right of a row (dim text).
pub fn row_value(s: &mut Svg, x: f64, y: f64, w: f64, h: f64, v: &str, t: &Theme) {
    s.text(
        x + w - 16.0,
        y + h / 2.0 + 5.0,
        14.0,
        false,
        t.dim,
        "end",
        v,
    );
}

pub fn switch_row(
    s: &mut Svg,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    title: &str,
    sub: Option<&str>,
    on: bool,
    t: &Theme,
) {
    card_row(s, x, y, h, title, sub, t);
    switch(s, x + w - 62.0, y + h / 2.0 - 13.0, on, t);
}

/// An `AdwComboRow`: the current choice with a small chevron.
pub fn combo_row(
    s: &mut Svg,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    title: &str,
    sub: Option<&str>,
    value: &str,
    t: &Theme,
) {
    card_row(s, x, y, h, title, sub, t);
    s.text(
        x + w - 40.0,
        y + h / 2.0 + 5.0,
        14.0,
        false,
        t.dim,
        "end",
        value,
    );
    icon_chev(s, x + w - 24.0, y + h / 2.0 - 1.0, t.dim, 'd');
}

/// Check mark inside a ring (selected state of a list).
pub fn icon_check(s: &mut Svg, cx: f64, cy: f64, c: &str) {
    s.path(&format!("M{},{} l4,4 l8,-8", cx - 6.0, cy), "none", c, 2.2);
}

/// Greyed-out small caption under a sheet item.
pub fn caption(s: &mut Svg, x: f64, y: f64, text: &str) {
    s.text(x, y, 13.0, true, "#5e5c64", "middle", text);
}

/// A popover card with an arrow pointing up (`up = true`) or down at `ax` (x of the arrow tip).
pub fn popover_card(s: &mut Svg, x: f64, y: f64, w: f64, h: f64, ax: f64, up: bool, t: &Theme) {
    s.shadow(x, y, w, h, 14.0, if t.shadow == 0.0 { 0.0 } else { 0.3 });
    s.rect(x, y, w, h, 14.0, t.card, Some(t.border));
    let d = if up {
        format!("M{},{} l10,-10 l10,10 z", ax - 10.0, y)
    } else {
        format!("M{},{} l10,10 l10,-10 z", ax - 10.0, y + h)
    };
    s.path(&d, t.card, t.border, 1.0);
    // hide the card border under the arrow
    let (ay, ah) = if up {
        (y - 0.5, 2.5)
    } else {
        (y + h - 1.5, 2.5)
    };
    s.rect(ax - 9.0, ay, 18.0, ah, 0.0, t.card, None);
}
