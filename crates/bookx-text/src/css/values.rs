//! Property values: tokenising with `cssparser`, parsing of the supported property set into typed declarations.

use cssparser::{ParseError, Parser, Token};

/// A colour with alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }

    pub const TRANSPARENT: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };

    /// Lower-case `#rrggbb`, or `#rrggbbaa` when not opaque.
    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
    Oblique,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    /// Not specified: the reader decides (usually justify or left).
    #[default]
    Start,
    Left,
    Right,
    Center,
    Justify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextDecoration {
    pub underline: bool,
    pub line_through: bool,
    pub overline: bool,
}

impl TextDecoration {
    pub fn is_none(&self) -> bool {
        !self.underline && !self.line_through && !self.overline
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Display {
    #[default]
    Inline,
    Block,
    None,
    ListItem,
    InlineBlock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageBreak {
    #[default]
    Auto,
    Always,
    Avoid,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hyphens {
    #[default]
    Manual,
    None,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Float {
    #[default]
    None,
    Left,
    Right,
}

/// CSS `writing-mode`: the direction lines run and the way they stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WritingMode {
    #[default]
    HorizontalTb,
    /// Lines run top to bottom, columns stack from right to left (Japanese, Chinese).
    VerticalRl,
    /// Lines run top to bottom, columns stack from left to right (Mongolian).
    VerticalLr,
}

impl WritingMode {
    pub fn is_vertical(self) -> bool {
        self != WritingMode::HorizontalTb
    }
}

/// CSS `text-orientation`: how glyphs stand in vertical lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextOrientation {
    /// Ideographs upright, Latin rotated a quarter turn.
    #[default]
    Mixed,
    /// Every glyph upright.
    Upright,
    /// Every glyph rotated, as in horizontal text turned a quarter turn.
    Sideways,
}

/// CSS `text-combine-upright` (tate-chu-yoko): several characters set side by side in one em.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextCombine {
    #[default]
    None,
    /// The whole element.
    All,
    /// Runs of up to this many ASCII digits (2 to 4).
    Digits(u8),
}

/// Computed line height.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum LineHeight {
    #[default]
    Normal,
    /// Multiplier of the element's font size (inherited as a number).
    Number(f64),
    /// Absolute length in px.
    Px(f64),
}

/// A computed width: absolute or relative to the container.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Size {
    Px(f64),
    Percent(f64),
}

/// Four sides in px.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Edges {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

/// A specified length before resolution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Len {
    Px(f64),
    Em(f64),
    Rem(f64),
    Pct(f64),
    Auto,
}

/// A specified value: CSS-wide keyword or a concrete value.
#[derive(Debug, Clone, PartialEq)]
pub enum Val<T> {
    Inherit,
    Initial,
    Unset,
    Set(T),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WeightSpec {
    Abs(u16),
    Bolder,
    Lighter,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineHeightSpec {
    Normal,
    Number(f64),
    Len(Len),
}

/// One parsed declaration (shorthands are already expanded).
#[derive(Debug, Clone, PartialEq)]
pub enum Decl {
    FontFamily(Val<Vec<String>>),
    FontSize(Val<Len>),
    FontWeight(Val<WeightSpec>),
    FontStyle(Val<FontStyle>),
    Color(Val<Color>),
    BackgroundColor(Val<Color>),
    TextAlign(Val<TextAlign>),
    TextIndent(Val<Len>),
    Margin(Side, Val<Len>),
    Padding(Side, Val<Len>),
    LineHeight(Val<LineHeightSpec>),
    TextDecoration(Val<TextDecoration>),
    Display(Val<Display>),
    PageBreakBefore(Val<PageBreak>),
    PageBreakAfter(Val<PageBreak>),
    Hyphens(Val<Hyphens>),
    SmallCaps(Val<bool>),
    Float(Val<Float>),
    Width(Val<Option<Len>>),
    MaxWidth(Val<Option<Len>>),
    WritingMode(Val<WritingMode>),
    TextOrientation(Val<TextOrientation>),
    TextCombine(Val<TextCombine>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

/// Owned token used for value parsing.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Tok {
    Ident(String),
    Num(f64),
    Dim(f64, String),
    Pct(f64),
    Str(String),
    Hash(String),
    Comma,
    Slash,
    Bang,
    Func(String, Vec<Tok>),
    Other,
}

fn f(v: f32) -> f64 {
    v.to_string().parse().unwrap_or(v as f64)
}

fn collect(p: &mut Parser<'_>) -> Vec<Tok> {
    let mut out = Vec::new();
    while let Ok(t) = p.next().cloned() {
        out.push(match t {
            Token::Ident(s) => Tok::Ident(s.to_string()),
            Token::Number { value, .. } => Tok::Num(f(value)),
            Token::Dimension { value, unit, .. } => Tok::Dim(f(value), unit.to_ascii_lowercase()),
            Token::Percentage { unit_value, .. } => {
                Tok::Pct((f(unit_value) * 100.0 * 1e6).round() / 1e6)
            }
            Token::QuotedString(s) => Tok::Str(s.to_string()),
            Token::Hash(s) | Token::IDHash(s) => Tok::Hash(s.to_string()),
            Token::Comma => Tok::Comma,
            Token::Delim('/') => Tok::Slash,
            Token::Delim('!') => Tok::Bang,
            Token::Function(name) => {
                let name = name.to_ascii_lowercase();
                let inner = p
                    .parse_nested_block(|p| Ok::<_, ParseError<()>>(collect(p)))
                    .unwrap_or_default();
                Tok::Func(name, inner)
            }
            _ => Tok::Other,
        });
    }
    out
}

pub(crate) fn tokenize(s: &str) -> Vec<Tok> {
    let mut p = Parser::new(s);
    collect(&mut p)
}

/// Split off a trailing `!important`.
pub(crate) fn split_important(mut toks: Vec<Tok>) -> (Vec<Tok>, bool) {
    if toks.len() >= 3 {
        if let (Some(Tok::Ident(i)), Some(Tok::Bang)) = (toks.last(), toks.get(toks.len() - 2)) {
            if i.eq_ignore_ascii_case("important") {
                toks.truncate(toks.len() - 2);
                return (toks, true);
            }
        }
    }
    (toks, false)
}

fn ident(t: &Tok) -> Option<&str> {
    if let Tok::Ident(s) = t { Some(s) } else { None }
}

fn kw(toks: &[Tok]) -> Option<String> {
    match toks {
        [Tok::Ident(s)] => Some(s.to_ascii_lowercase()),
        _ => None,
    }
}

pub(crate) fn parse_len(t: &Tok) -> Option<Len> {
    match t {
        Tok::Num(n) if *n == 0.0 => Some(Len::Px(0.0)),
        Tok::Pct(p) => Some(Len::Pct(*p)),
        Tok::Dim(v, u) => Some(match u.as_str() {
            "px" => Len::Px(*v),
            "pt" => Len::Px(v * 4.0 / 3.0),
            "pc" => Len::Px(v * 16.0),
            "in" => Len::Px(v * 96.0),
            "cm" => Len::Px(v * 96.0 / 2.54),
            "mm" => Len::Px(v * 96.0 / 25.4),
            "q" => Len::Px(v * 96.0 / 101.6),
            "em" => Len::Em(*v),
            "rem" => Len::Rem(*v),
            "ex" | "ch" => Len::Em(v * 0.5),
            _ => return None,
        }),
        Tok::Ident(s) if s.eq_ignore_ascii_case("auto") => Some(Len::Auto),
        _ => None,
    }
}

fn no_auto(l: Len) -> Option<Len> {
    (l != Len::Auto).then_some(l)
}

fn val<T>(toks: &[Tok], parse: impl Fn(&[Tok]) -> Option<T>) -> Option<Val<T>> {
    if let Some(k) = kw(toks) {
        match k.as_str() {
            "inherit" => return Some(Val::Inherit),
            "initial" => return Some(Val::Initial),
            "unset" | "revert" | "revert-layer" => return Some(Val::Unset),
            _ => {}
        }
    }
    parse(toks).map(Val::Set)
}

const NAMED_COLORS: &[(&str, Color)] = &[
    ("black", Color::rgb(0, 0, 0)),
    ("silver", Color::rgb(192, 192, 192)),
    ("gray", Color::rgb(128, 128, 128)),
    ("grey", Color::rgb(128, 128, 128)),
    ("white", Color::rgb(255, 255, 255)),
    ("maroon", Color::rgb(128, 0, 0)),
    ("red", Color::rgb(255, 0, 0)),
    ("purple", Color::rgb(128, 0, 128)),
    ("fuchsia", Color::rgb(255, 0, 255)),
    ("magenta", Color::rgb(255, 0, 255)),
    ("green", Color::rgb(0, 128, 0)),
    ("lime", Color::rgb(0, 255, 0)),
    ("olive", Color::rgb(128, 128, 0)),
    ("yellow", Color::rgb(255, 255, 0)),
    ("navy", Color::rgb(0, 0, 128)),
    ("blue", Color::rgb(0, 0, 255)),
    ("teal", Color::rgb(0, 128, 128)),
    ("aqua", Color::rgb(0, 255, 255)),
    ("cyan", Color::rgb(0, 255, 255)),
    ("orange", Color::rgb(255, 165, 0)),
    ("brown", Color::rgb(165, 42, 42)),
    ("pink", Color::rgb(255, 192, 203)),
    ("gold", Color::rgb(255, 215, 0)),
    ("darkgray", Color::rgb(169, 169, 169)),
    ("darkgrey", Color::rgb(169, 169, 169)),
    ("lightgray", Color::rgb(211, 211, 211)),
    ("lightgrey", Color::rgb(211, 211, 211)),
    ("darkred", Color::rgb(139, 0, 0)),
    ("darkblue", Color::rgb(0, 0, 139)),
    ("darkgreen", Color::rgb(0, 100, 0)),
    ("transparent", Color::TRANSPARENT),
];

fn hex_color(h: &str) -> Option<Color> {
    if !h.is_ascii() {
        return None;
    }
    let d = |i: usize| u8::from_str_radix(&h[i..i + 1], 16).ok().map(|v| v * 17);
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    match h.len() {
        3 => Some(Color::rgb(d(0)?, d(1)?, d(2)?)),
        4 => Some(Color {
            r: d(0)?,
            g: d(1)?,
            b: d(2)?,
            a: d(3)?,
        }),
        6 => Some(Color::rgb(p(0)?, p(2)?, p(4)?)),
        8 => Some(Color {
            r: p(0)?,
            g: p(2)?,
            b: p(4)?,
            a: p(6)?,
        }),
        _ => None,
    }
}

fn channel(t: &Tok) -> Option<u8> {
    match t {
        Tok::Num(n) => Some(n.round().clamp(0.0, 255.0) as u8),
        Tok::Pct(p) => Some((p / 100.0 * 255.0).round().clamp(0.0, 255.0) as u8),
        _ => None,
    }
}

/// Parse a colour value (named, hex, `rgb()`, `rgba()`).
pub(crate) fn parse_color(toks: &[Tok]) -> Option<Color> {
    match toks {
        [Tok::Ident(s)] => {
            let l = s.to_ascii_lowercase();
            NAMED_COLORS.iter().find(|(n, _)| *n == l).map(|(_, c)| *c)
        }
        [Tok::Hash(h)] => hex_color(h),
        [Tok::Func(name, args)] if name == "rgb" || name == "rgba" => {
            let parts: Vec<&Tok> = args
                .iter()
                .filter(|t| !matches!(t, Tok::Comma | Tok::Slash))
                .collect();
            if parts.len() != 3 && parts.len() != 4 {
                return None;
            }
            let a = match parts.get(3) {
                None => 255,
                Some(Tok::Num(n)) => (n.clamp(0.0, 1.0) * 255.0).round() as u8,
                Some(Tok::Pct(p)) => (p.clamp(0.0, 100.0) / 100.0 * 255.0).round() as u8,
                Some(_) => return None,
            };
            Some(Color {
                r: channel(parts[0])?,
                g: channel(parts[1])?,
                b: channel(parts[2])?,
                a,
            })
        }
        _ => None,
    }
}

fn parse_family(toks: &[Tok]) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for seg in toks.split(|t| *t == Tok::Comma) {
        match seg {
            [Tok::Str(s)] => out.push(s.clone()),
            [] => return None,
            idents => {
                let words: Option<Vec<&str>> = idents.iter().map(ident).collect();
                let w = words?;
                if w.iter().any(|x| {
                    matches!(
                        x.to_ascii_lowercase().as_str(),
                        "inherit" | "initial" | "unset"
                    )
                }) && w.len() == 1
                {
                    return None;
                }
                out.push(w.join(" "));
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

fn parse_font_size(toks: &[Tok]) -> Option<Len> {
    match toks {
        [Tok::Ident(s)] => Some(match s.to_ascii_lowercase().as_str() {
            "xx-small" => Len::Rem(0.5625),
            "x-small" => Len::Rem(0.625),
            "small" => Len::Rem(0.8125),
            "medium" => Len::Rem(1.0),
            "large" => Len::Rem(1.125),
            "x-large" => Len::Rem(1.5),
            "xx-large" => Len::Rem(2.0),
            "xxx-large" => Len::Rem(3.0),
            "smaller" => Len::Em(1.0 / 1.2),
            "larger" => Len::Em(1.2),
            _ => return None,
        }),
        [t] => {
            let l = no_auto(parse_len(t)?)?;
            let neg = match l {
                Len::Px(v) | Len::Em(v) | Len::Rem(v) | Len::Pct(v) => v < 0.0,
                Len::Auto => true,
            };
            (!neg).then_some(l)
        }
        _ => None,
    }
}

fn parse_weight(t: &Tok) -> Option<WeightSpec> {
    match t {
        Tok::Ident(s) => match s.to_ascii_lowercase().as_str() {
            "normal" => Some(WeightSpec::Abs(400)),
            "bold" => Some(WeightSpec::Abs(700)),
            "bolder" => Some(WeightSpec::Bolder),
            "lighter" => Some(WeightSpec::Lighter),
            _ => None,
        },
        Tok::Num(n) if (1.0..=1000.0).contains(n) => Some(WeightSpec::Abs(n.round() as u16)),
        _ => None,
    }
}

fn parse_style(s: &str) -> Option<FontStyle> {
    match s.to_ascii_lowercase().as_str() {
        "normal" => Some(FontStyle::Normal),
        "italic" => Some(FontStyle::Italic),
        "oblique" => Some(FontStyle::Oblique),
        _ => None,
    }
}

fn parse_line_height_tok(t: &Tok) -> Option<LineHeightSpec> {
    match t {
        Tok::Ident(s) if s.eq_ignore_ascii_case("normal") => Some(LineHeightSpec::Normal),
        Tok::Num(n) if *n >= 0.0 => Some(LineHeightSpec::Number(*n)),
        t => {
            let l = no_auto(parse_len(t)?)?;
            match l {
                Len::Px(v) | Len::Em(v) | Len::Rem(v) | Len::Pct(v) if v < 0.0 => None,
                l => Some(LineHeightSpec::Len(l)),
            }
        }
    }
}

fn parse_decoration(toks: &[Tok]) -> Option<TextDecoration> {
    let mut d = TextDecoration::default();
    let mut any = false;
    for t in toks {
        match t {
            Tok::Ident(s) => match s.to_ascii_lowercase().as_str() {
                "none" => any = true,
                "underline" => {
                    d.underline = true;
                    any = true
                }
                "line-through" => {
                    d.line_through = true;
                    any = true
                }
                "overline" => {
                    d.overline = true;
                    any = true
                }
                "blink" | "solid" | "double" | "dotted" | "dashed" | "wavy" | "currentcolor" => {}
                other => {
                    if NAMED_COLORS.iter().all(|(n, _)| *n != other) {
                        return None;
                    }
                }
            },
            Tok::Hash(_) | Tok::Func(..) | Tok::Dim(..) | Tok::Pct(_) | Tok::Num(_) => {}
            _ => return None,
        }
    }
    any.then_some(d)
}

fn parse_display(toks: &[Tok]) -> Option<Display> {
    Some(match kw(toks)?.as_str() {
        "none" => Display::None,
        "inline" => Display::Inline,
        "block" | "flex" | "grid" | "table" | "table-row" | "table-cell" | "table-row-group"
        | "table-header-group" | "table-footer-group" | "table-caption" | "flow-root"
        | "run-in" => Display::Block,
        "list-item" => Display::ListItem,
        "inline-block" | "inline-flex" | "inline-grid" | "inline-table" => Display::InlineBlock,
        "contents" => Display::Inline,
        _ => return None,
    })
}

fn parse_break(toks: &[Tok]) -> Option<PageBreak> {
    Some(match kw(toks)?.as_str() {
        "auto" => PageBreak::Auto,
        "always" | "page" => PageBreak::Always,
        "left" | "verso" => PageBreak::Left,
        "right" | "recto" => PageBreak::Right,
        "avoid" | "avoid-page" | "avoid-column" => PageBreak::Avoid,
        _ => return None,
    })
}

fn parse_hyphens(toks: &[Tok]) -> Option<Hyphens> {
    Some(match kw(toks)?.as_str() {
        "none" => Hyphens::None,
        "manual" => Hyphens::Manual,
        "auto" => Hyphens::Auto,
        _ => return None,
    })
}

fn parse_writing_mode(toks: &[Tok]) -> Option<WritingMode> {
    Some(match kw(toks)?.as_str() {
        "horizontal-tb" | "lr" | "lr-tb" | "rl" | "rl-tb" => WritingMode::HorizontalTb,
        "vertical-rl" | "tb" | "tb-rl" | "sideways-rl" => WritingMode::VerticalRl,
        "vertical-lr" | "tb-lr" | "sideways-lr" => WritingMode::VerticalLr,
        _ => return None,
    })
}

fn parse_text_orientation(toks: &[Tok]) -> Option<TextOrientation> {
    Some(match kw(toks)?.as_str() {
        "mixed" | "vertical-right" => TextOrientation::Mixed,
        "upright" => TextOrientation::Upright,
        "sideways" | "sideways-right" | "use-glyph-orientation" => TextOrientation::Sideways,
        _ => return None,
    })
}

/// `none | all | digits <2-4>?`, and the legacy `horizontal` of `-epub-text-combine`.
fn parse_text_combine(toks: &[Tok]) -> Option<TextCombine> {
    match toks {
        [Tok::Ident(k)] => match k.to_ascii_lowercase().as_str() {
            "none" => Some(TextCombine::None),
            "all" | "horizontal" => Some(TextCombine::All),
            "digits" => Some(TextCombine::Digits(2)),
            _ => None,
        },
        [Tok::Ident(k), Tok::Num(n)] if k.eq_ignore_ascii_case("digits") => {
            let n = *n as i64;
            (2..=4).contains(&n).then_some(TextCombine::Digits(n as u8))
        }
        _ => None,
    }
}

fn parse_small_caps(toks: &[Tok]) -> Option<bool> {
    let mut caps = false;
    let mut any = false;
    for t in toks {
        let Tok::Ident(s) = t else { return None };
        any = true;
        if matches!(
            s.to_ascii_lowercase().as_str(),
            "small-caps" | "all-small-caps"
        ) {
            caps = true;
        }
    }
    any.then_some(caps)
}

fn parse_size_or_none(toks: &[Tok]) -> Option<Option<Len>> {
    match toks {
        [Tok::Ident(s)] if matches!(s.to_ascii_lowercase().as_str(), "auto" | "none") => Some(None),
        [t] => {
            let l = no_auto(parse_len(t)?)?;
            match l {
                Len::Px(v) | Len::Em(v) | Len::Rem(v) | Len::Pct(v) if v < 0.0 => None,
                Len::Em(v) | Len::Rem(v) if v.is_nan() => None,
                l => Some(Some(l)),
            }
        }
        _ => None,
    }
}

fn expand4(toks: &[Tok], allow_auto: bool, allow_neg: bool) -> Option<[Len; 4]> {
    if toks.is_empty() || toks.len() > 4 {
        return None;
    }
    let mut v = Vec::new();
    for t in toks {
        let l = parse_len(t)?;
        if l == Len::Auto && !allow_auto {
            return None;
        }
        if !allow_neg {
            if let Len::Px(x) | Len::Em(x) | Len::Rem(x) | Len::Pct(x) = l {
                if x < 0.0 {
                    return None;
                }
            }
        }
        v.push(l);
    }
    Some(match v.len() {
        1 => [v[0], v[0], v[0], v[0]],
        2 => [v[0], v[1], v[0], v[1]],
        3 => [v[0], v[1], v[2], v[1]],
        _ => [v[0], v[1], v[2], v[3]],
    })
}

fn font_shorthand(toks: &[Tok]) -> Option<Vec<Decl>> {
    let mut style = FontStyle::Normal;
    let mut caps = false;
    let mut weight = WeightSpec::Abs(400);
    let mut i = 0;
    let mut size = None;
    while i < toks.len() {
        let t = &toks[i];
        if let Tok::Ident(s) = t {
            let l = s.to_ascii_lowercase();
            if l == "normal" {
                i += 1;
                continue;
            }
            if let Some(st) = parse_style(&l) {
                style = st;
                i += 1;
                continue;
            }
            if l == "small-caps" {
                caps = true;
                i += 1;
                continue;
            }
            if let Some(w) = parse_weight(t) {
                weight = w;
                i += 1;
                continue;
            }
        } else if let Some(w @ WeightSpec::Abs(_)) = parse_weight(t) {
            weight = w;
            i += 1;
            continue;
        }
        size = Some(parse_font_size(std::slice::from_ref(t))?);
        i += 1;
        break;
    }
    let size = size?;
    let mut lh = LineHeightSpec::Normal;
    if toks.get(i) == Some(&Tok::Slash) {
        lh = parse_line_height_tok(toks.get(i + 1)?)?;
        i += 2;
    }
    let family = parse_family(&toks[i..])?;
    Some(vec![
        Decl::FontStyle(Val::Set(style)),
        Decl::SmallCaps(Val::Set(caps)),
        Decl::FontWeight(Val::Set(weight)),
        Decl::FontSize(Val::Set(size)),
        Decl::LineHeight(Val::Set(lh)),
        Decl::FontFamily(Val::Set(family)),
    ])
}

const SIDES: [Side; 4] = [Side::Top, Side::Right, Side::Bottom, Side::Left];

/// Parse one declaration to typed declarations. Unknown properties and invalid values give an empty list.
pub(crate) fn parse_declaration(name: &str, toks: &[Tok]) -> Vec<Decl> {
    let name = name.trim().to_ascii_lowercase();
    let one = |d: Option<Decl>| d.into_iter().collect::<Vec<_>>();
    // CSS-wide keywords on shorthands expand to every longhand
    let wide = kw(toks).and_then(|k| match k.as_str() {
        "inherit" => Some(0),
        "initial" => Some(1),
        "unset" | "revert" | "revert-layer" => Some(2),
        _ => None,
    });
    fn wide_val<T>(w: i32) -> Val<T> {
        match w {
            0 => Val::Inherit,
            1 => Val::Initial,
            _ => Val::Unset,
        }
    }
    match name.as_str() {
        "font-family" => one(val(toks, parse_family).map(Decl::FontFamily)),
        "font-size" => one(val(toks, parse_font_size).map(Decl::FontSize)),
        "font-weight" => one(
            val(toks, |t| if let [x] = t { parse_weight(x) } else { None }).map(Decl::FontWeight),
        ),
        "font-style" => {
            one(val(toks, |t| kw(t).and_then(|k| parse_style(&k))).map(Decl::FontStyle))
        }
        "font-variant" | "font-variant-caps" => {
            one(val(toks, parse_small_caps).map(Decl::SmallCaps))
        }
        "font" => {
            if let Some(w) = wide {
                return vec![
                    Decl::FontStyle(wide_val(w)),
                    Decl::SmallCaps(wide_val(w)),
                    Decl::FontWeight(wide_val(w)),
                    Decl::FontSize(wide_val(w)),
                    Decl::LineHeight(wide_val(w)),
                    Decl::FontFamily(wide_val(w)),
                ];
            }
            font_shorthand(toks).unwrap_or_default()
        }
        "color" => {
            if kw(toks).is_some_and(|k| k == "currentcolor") {
                return vec![Decl::Color(Val::Inherit)];
            }
            one(val(toks, parse_color).map(Decl::Color))
        }
        "background-color" => {
            if kw(toks).is_some_and(|k| k == "currentcolor") {
                return vec![Decl::BackgroundColor(Val::Inherit)];
            }
            one(val(toks, parse_color).map(Decl::BackgroundColor))
        }
        // only the colour of the shorthand is used (images and gradients are not drawn)
        "background" => {
            if let Some(k) = kw(toks) {
                match k.as_str() {
                    "inherit" => return vec![Decl::BackgroundColor(Val::Inherit)],
                    "initial" | "none" | "transparent" => {
                        return vec![Decl::BackgroundColor(Val::Initial)];
                    }
                    "unset" | "revert" => return vec![Decl::BackgroundColor(Val::Unset)],
                    _ => {}
                }
            }
            toks.iter()
                .find_map(|t| parse_color(std::slice::from_ref(t)))
                .map(|c| vec![Decl::BackgroundColor(Val::Set(c))])
                .unwrap_or_default()
        }
        "text-align" => one(val(toks, |t| {
            Some(match kw(t)?.as_str() {
                "left" | "start" | "-webkit-left" | "-moz-left" => TextAlign::Left,
                "right" | "end" | "-webkit-right" | "-moz-right" => TextAlign::Right,
                "center" | "-webkit-center" | "-moz-center" => TextAlign::Center,
                "justify" | "justify-all" => TextAlign::Justify,
                _ => return None,
            })
        })
        .map(Decl::TextAlign)),
        "text-indent" => one(val(toks, |t| {
            let l = no_auto(parse_len(t.first()?)?)?;
            Some(l)
        })
        .map(Decl::TextIndent)),
        "line-height" => one(val(toks, |t| {
            if let [x] = t {
                parse_line_height_tok(x)
            } else {
                None
            }
        })
        .map(Decl::LineHeight)),
        "text-decoration" | "text-decoration-line" => {
            one(val(toks, parse_decoration).map(Decl::TextDecoration))
        }
        "display" => one(val(toks, parse_display).map(Decl::Display)),
        "page-break-before" | "break-before" => {
            one(val(toks, parse_break).map(Decl::PageBreakBefore))
        }
        "page-break-after" | "break-after" => one(val(toks, parse_break).map(Decl::PageBreakAfter)),
        "hyphens" | "-webkit-hyphens" | "-epub-hyphens" | "-moz-hyphens" => {
            one(val(toks, parse_hyphens).map(Decl::Hyphens))
        }
        "writing-mode" | "-epub-writing-mode" | "-webkit-writing-mode" => {
            one(val(toks, parse_writing_mode).map(Decl::WritingMode))
        }
        "text-orientation" | "-epub-text-orientation" | "-webkit-text-orientation" => {
            one(val(toks, parse_text_orientation).map(Decl::TextOrientation))
        }
        "text-combine-upright" | "-epub-text-combine" | "-webkit-text-combine" => {
            one(val(toks, parse_text_combine).map(Decl::TextCombine))
        }
        "float" => one(val(toks, |t| {
            Some(match kw(t)?.as_str() {
                "left" => Float::Left,
                "right" => Float::Right,
                "none" | "inline-start" | "inline-end" => Float::None,
                _ => return None,
            })
        })
        .map(Decl::Float)),
        "width" => one(val(toks, parse_size_or_none).map(Decl::Width)),
        "max-width" => one(val(toks, parse_size_or_none).map(Decl::MaxWidth)),
        "margin" | "padding" => {
            let is_margin = name == "margin";
            let mk = |s: Side, v: Val<Len>| {
                if is_margin {
                    Decl::Margin(s, v)
                } else {
                    Decl::Padding(s, v)
                }
            };
            if let Some(w) = wide {
                return SIDES.iter().map(|s| mk(*s, wide_val(w))).collect();
            }
            match expand4(toks, is_margin, is_margin) {
                Some(v) => SIDES
                    .iter()
                    .zip(v)
                    .map(|(s, l)| mk(*s, Val::Set(l)))
                    .collect(),
                None => Vec::new(),
            }
        }
        n if n.starts_with("margin-") || n.starts_with("padding-") => {
            let is_margin = n.starts_with("margin-");
            let side = match n.split_once('-').map(|x| x.1).unwrap_or("") {
                "top" | "block-start" => Side::Top,
                "right" => Side::Right,
                "bottom" | "block-end" => Side::Bottom,
                "left" | "inline-start" => Side::Left,
                "inline-end" => Side::Right,
                _ => return Vec::new(),
            };
            let v = val(toks, |t| {
                let [x] = t else { return None };
                let l = parse_len(x)?;
                if l == Len::Auto && !is_margin {
                    return None;
                }
                if !is_margin {
                    if let Len::Px(v) | Len::Em(v) | Len::Rem(v) | Len::Pct(v) = l {
                        if v < 0.0 {
                            return None;
                        }
                    }
                }
                Some(l)
            });
            one(v.map(|v| {
                if is_margin {
                    Decl::Margin(side, v)
                } else {
                    Decl::Padding(side, v)
                }
            }))
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(name: &str, value: &str) -> Vec<Decl> {
        let (t, _) = split_important(tokenize(value));
        parse_declaration(name, &t)
    }

    fn first(name: &str, value: &str) -> Decl {
        one(name, value)
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("{name}: {value} did not parse"))
    }

    #[test]
    fn tokenize_basic_kinds() {
        let t = tokenize("1.5em 12px 50% #fff 'a b', bold / rgb(1,2,3) !important");
        assert_eq!(t[0], Tok::Dim(1.5, "em".into()));
        assert_eq!(t[1], Tok::Dim(12.0, "px".into()));
        assert_eq!(t[2], Tok::Pct(50.0));
        assert_eq!(t[3], Tok::Hash("fff".into()));
        assert_eq!(t[4], Tok::Str("a b".into()));
        assert_eq!(t[5], Tok::Comma);
        assert_eq!(t[6], Tok::Ident("bold".into()));
        assert_eq!(t[7], Tok::Slash);
        assert_eq!(
            t[8],
            Tok::Func(
                "rgb".into(),
                vec![
                    Tok::Num(1.0),
                    Tok::Comma,
                    Tok::Num(2.0),
                    Tok::Comma,
                    Tok::Num(3.0)
                ]
            )
        );
        assert_eq!(t[9], Tok::Bang);
        assert_eq!(t[10], Tok::Ident("important".into()));
        assert_eq!(tokenize("1.4"), vec![Tok::Num(1.4)], "no f32 noise");
        assert!(tokenize("").is_empty());
    }

    #[test]
    fn important_splitting() {
        let (t, imp) = split_important(tokenize("red ! important"));
        assert!(imp);
        assert_eq!(t, vec![Tok::Ident("red".into())]);
        let (t, imp) = split_important(tokenize("red"));
        assert!(!imp);
        assert_eq!(t.len(), 1);
        let (_, imp) = split_important(tokenize("!IMPORTANT"));
        assert!(!imp, "needs a value before it");
        let (t, imp) = split_important(tokenize("a !foo"));
        assert!(!imp);
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn length_units() {
        let l = |s: &str| parse_len(&tokenize(s)[0]);
        assert_eq!(l("10px"), Some(Len::Px(10.0)));
        assert_eq!(l("12pt"), Some(Len::Px(16.0)));
        assert_eq!(l("1pc"), Some(Len::Px(16.0)));
        assert_eq!(l("1in"), Some(Len::Px(96.0)));
        assert!(
            (match l("2.54cm") {
                Some(Len::Px(v)) => v,
                _ => 0.0,
            } - 96.0)
                .abs()
                < 1e-9
        );
        assert!(
            (match l("25.4mm") {
                Some(Len::Px(v)) => v,
                _ => 0.0,
            } - 96.0)
                .abs()
                < 1e-9
        );
        assert!(
            (match l("101.6Q") {
                Some(Len::Px(v)) => v,
                _ => 0.0,
            } - 96.0)
                .abs()
                < 1e-9
        );
        assert_eq!(l("1.5em"), Some(Len::Em(1.5)));
        assert_eq!(l("2rem"), Some(Len::Rem(2.0)));
        assert_eq!(l("4ex"), Some(Len::Em(2.0)));
        assert_eq!(l("10EM"), Some(Len::Em(10.0)));
        assert_eq!(l("50%"), Some(Len::Pct(50.0)));
        assert_eq!(l("0"), Some(Len::Px(0.0)));
        assert_eq!(l("auto"), Some(Len::Auto));
        assert_eq!(l("5"), None, "unitless non-zero is invalid");
        assert_eq!(l("3vw"), None);
        assert_eq!(l("foo"), None);
    }

    #[test]
    fn colours() {
        let c = |s: &str| parse_color(&tokenize(s));
        assert_eq!(c("red"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(c("RED"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(c("#f00"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(c("#FF0000"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(
            c("#ff000080"),
            Some(Color {
                r: 255,
                g: 0,
                b: 0,
                a: 128
            })
        );
        assert_eq!(
            c("#f008"),
            Some(Color {
                r: 255,
                g: 0,
                b: 0,
                a: 136
            })
        );
        assert_eq!(c("rgb(10, 20, 30)"), Some(Color::rgb(10, 20, 30)));
        assert_eq!(c("rgb(10 20 30)"), Some(Color::rgb(10, 20, 30)));
        assert_eq!(
            c("rgba(10,20,30,0.5)"),
            Some(Color {
                r: 10,
                g: 20,
                b: 30,
                a: 128
            })
        );
        assert_eq!(c("rgb(100%, 0%, 50%)"), Some(Color::rgb(255, 0, 128)));
        assert_eq!(c("rgb(300, -5, 0)"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(c("transparent"), Some(Color::TRANSPARENT));
        for bad in [
            "#ff",
            "#fffff",
            "#ggg",
            "rgb(1,2)",
            "rgb(a,b,c)",
            "notacolor",
            "rgb(1,2,3,x)",
            "",
        ] {
            assert_eq!(c(bad), None, "{bad}");
        }
        assert_eq!(Color::rgb(1, 2, 3).to_hex(), "#010203");
        assert_eq!(
            Color {
                r: 1,
                g: 2,
                b: 3,
                a: 4
            }
            .to_hex(),
            "#01020304"
        );
    }

    #[test]
    fn font_family_lists() {
        assert_eq!(
            first("font-family", "Georgia, 'Times New Roman', serif"),
            Decl::FontFamily(Val::Set(vec![
                "Georgia".into(),
                "Times New Roman".into(),
                "serif".into()
            ]))
        );
        assert_eq!(
            first("font-family", "Times New Roman, serif"),
            Decl::FontFamily(Val::Set(vec!["Times New Roman".into(), "serif".into()]))
        );
        assert_eq!(
            first("font-family", "inherit"),
            Decl::FontFamily(Val::Inherit)
        );
        assert!(one("font-family", "Georgia,").is_empty());
        assert!(one("font-family", "12px").is_empty());
        assert!(one("font-family", "").is_empty());
    }

    #[test]
    fn font_size_forms() {
        assert_eq!(
            first("font-size", "12px"),
            Decl::FontSize(Val::Set(Len::Px(12.0)))
        );
        assert_eq!(
            first("font-size", "12pt"),
            Decl::FontSize(Val::Set(Len::Px(16.0)))
        );
        assert_eq!(
            first("font-size", "1.2em"),
            Decl::FontSize(Val::Set(Len::Em(1.2)))
        );
        assert_eq!(
            first("font-size", "150%"),
            Decl::FontSize(Val::Set(Len::Pct(150.0)))
        );
        assert_eq!(
            first("font-size", "2rem"),
            Decl::FontSize(Val::Set(Len::Rem(2.0)))
        );
        assert_eq!(
            first("font-size", "large"),
            Decl::FontSize(Val::Set(Len::Rem(1.125)))
        );
        assert_eq!(
            first("font-size", "larger"),
            Decl::FontSize(Val::Set(Len::Em(1.2)))
        );
        assert!(one("font-size", "-3px").is_empty());
        assert!(one("font-size", "auto").is_empty());
        assert!(one("font-size", "huge").is_empty());
        assert!(one("font-size", "1em 2em").is_empty());
    }

    #[test]
    fn font_weight_style_variant() {
        assert_eq!(
            first("font-weight", "bold"),
            Decl::FontWeight(Val::Set(WeightSpec::Abs(700)))
        );
        assert_eq!(
            first("font-weight", "normal"),
            Decl::FontWeight(Val::Set(WeightSpec::Abs(400)))
        );
        assert_eq!(
            first("font-weight", "600"),
            Decl::FontWeight(Val::Set(WeightSpec::Abs(600)))
        );
        assert_eq!(
            first("font-weight", "bolder"),
            Decl::FontWeight(Val::Set(WeightSpec::Bolder))
        );
        assert_eq!(
            first("font-weight", "lighter"),
            Decl::FontWeight(Val::Set(WeightSpec::Lighter))
        );
        assert!(one("font-weight", "heavy").is_empty());
        assert!(one("font-weight", "0").is_empty());
        assert_eq!(
            first("font-style", "italic"),
            Decl::FontStyle(Val::Set(FontStyle::Italic))
        );
        assert_eq!(
            first("font-style", "oblique"),
            Decl::FontStyle(Val::Set(FontStyle::Oblique))
        );
        assert_eq!(
            first("font-style", "NORMAL"),
            Decl::FontStyle(Val::Set(FontStyle::Normal))
        );
        assert!(one("font-style", "slanted").is_empty());
        assert_eq!(
            first("font-variant", "small-caps"),
            Decl::SmallCaps(Val::Set(true))
        );
        assert_eq!(
            first("font-variant-caps", "all-small-caps"),
            Decl::SmallCaps(Val::Set(true))
        );
        assert_eq!(
            first("font-variant", "normal"),
            Decl::SmallCaps(Val::Set(false))
        );
        assert_eq!(
            first("font-variant", "oldstyle-nums small-caps"),
            Decl::SmallCaps(Val::Set(true))
        );
        assert!(one("font-variant", "12px").is_empty());
    }

    #[test]
    fn font_shorthand_full_and_errors() {
        let d = one("font", "italic small-caps bold 14px/1.5 Georgia, serif");
        assert_eq!(
            d,
            vec![
                Decl::FontStyle(Val::Set(FontStyle::Italic)),
                Decl::SmallCaps(Val::Set(true)),
                Decl::FontWeight(Val::Set(WeightSpec::Abs(700))),
                Decl::FontSize(Val::Set(Len::Px(14.0))),
                Decl::LineHeight(Val::Set(LineHeightSpec::Number(1.5))),
                Decl::FontFamily(Val::Set(vec!["Georgia".into(), "serif".into()])),
            ]
        );
        let d = one("font", "12px serif");
        assert_eq!(d[0], Decl::FontStyle(Val::Set(FontStyle::Normal)));
        assert_eq!(d[2], Decl::FontWeight(Val::Set(WeightSpec::Abs(400))));
        assert_eq!(d[4], Decl::LineHeight(Val::Set(LineHeightSpec::Normal)));
        let d = one("font", "600 1em/120% 'A B'");
        assert_eq!(d[2], Decl::FontWeight(Val::Set(WeightSpec::Abs(600))));
        assert_eq!(
            d[4],
            Decl::LineHeight(Val::Set(LineHeightSpec::Len(Len::Pct(120.0))))
        );
        assert!(one("font", "bold").is_empty(), "no size");
        assert!(one("font", "12px").is_empty(), "no family");
        assert!(one("font", "caption").is_empty());
        let d = one("font", "inherit");
        assert_eq!(d.len(), 6);
        assert!(d.iter().all(|x| matches!(
            x,
            Decl::FontStyle(Val::Inherit)
                | Decl::SmallCaps(Val::Inherit)
                | Decl::FontWeight(Val::Inherit)
                | Decl::FontSize(Val::Inherit)
                | Decl::LineHeight(Val::Inherit)
                | Decl::FontFamily(Val::Inherit)
        )));
    }

    #[test]
    fn margin_padding_shorthands() {
        let m = |v: &str| {
            one("margin", v)
                .into_iter()
                .map(|d| match d {
                    Decl::Margin(s, Val::Set(l)) => (s, l),
                    other => panic!("{other:?}"),
                })
                .collect::<Vec<_>>()
        };
        let px = |a: f64, b: f64, c: f64, d: f64| {
            vec![
                (Side::Top, Len::Px(a)),
                (Side::Right, Len::Px(b)),
                (Side::Bottom, Len::Px(c)),
                (Side::Left, Len::Px(d)),
            ]
        };
        assert_eq!(m("1px"), px(1.0, 1.0, 1.0, 1.0));
        assert_eq!(m("1px 2px"), px(1.0, 2.0, 1.0, 2.0));
        assert_eq!(m("1px 2px 3px"), px(1.0, 2.0, 3.0, 2.0));
        assert_eq!(m("1px 2px 3px 4px"), px(1.0, 2.0, 3.0, 4.0));
        assert_eq!(m("0")[0].1, Len::Px(0.0));
        assert_eq!(
            m("-5px 0")[0].1,
            Len::Px(-5.0),
            "negative margins are allowed"
        );
        assert_eq!(m("0 auto")[1].1, Len::Auto);
        assert!(one("margin", "1px 2px 3px 4px 5px").is_empty());
        assert!(one("margin", "").is_empty());
        assert!(one("margin", "1px red").is_empty());
        assert!(
            one("padding", "-1px").is_empty(),
            "negative padding is invalid"
        );
        assert!(one("padding", "auto").is_empty());
        assert_eq!(one("padding", "2px 4px").len(), 4);
        assert_eq!(
            first("padding", "2px 4px"),
            Decl::Padding(Side::Top, Val::Set(Len::Px(2.0)))
        );
        let w = one("margin", "inherit");
        assert_eq!(w.len(), 4);
        assert!(w.iter().all(|d| matches!(d, Decl::Margin(_, Val::Inherit))));
        let w = one("padding", "initial");
        assert!(
            w.iter()
                .all(|d| matches!(d, Decl::Padding(_, Val::Initial)))
        );
    }

    #[test]
    fn margin_padding_longhands_and_logical() {
        assert_eq!(
            first("margin-top", "1em"),
            Decl::Margin(Side::Top, Val::Set(Len::Em(1.0)))
        );
        assert_eq!(
            first("margin-left", "auto"),
            Decl::Margin(Side::Left, Val::Set(Len::Auto))
        );
        assert_eq!(
            first("padding-right", "3px"),
            Decl::Padding(Side::Right, Val::Set(Len::Px(3.0)))
        );
        assert_eq!(
            first("margin-block-start", "1px"),
            Decl::Margin(Side::Top, Val::Set(Len::Px(1.0)))
        );
        assert_eq!(
            first("margin-inline-end", "1px"),
            Decl::Margin(Side::Right, Val::Set(Len::Px(1.0)))
        );
        assert_eq!(
            first("padding-block-end", "1px"),
            Decl::Padding(Side::Bottom, Val::Set(Len::Px(1.0)))
        );
        assert_eq!(
            first("padding-inline-start", "1px"),
            Decl::Padding(Side::Left, Val::Set(Len::Px(1.0)))
        );
        assert!(one("padding-left", "auto").is_empty());
        assert!(one("padding-left", "-2px").is_empty());
        assert!(one("margin-top", "1px 2px").is_empty());
        assert!(one("margin-bogus", "1px").is_empty());
        assert_eq!(
            first("margin-top", "inherit"),
            Decl::Margin(Side::Top, Val::Inherit)
        );
    }

    #[test]
    fn text_properties() {
        assert_eq!(
            first("text-align", "justify"),
            Decl::TextAlign(Val::Set(TextAlign::Justify))
        );
        assert_eq!(
            first("text-align", "-webkit-center"),
            Decl::TextAlign(Val::Set(TextAlign::Center))
        );
        assert_eq!(
            first("text-align", "start"),
            Decl::TextAlign(Val::Set(TextAlign::Left))
        );
        assert_eq!(
            first("text-align", "end"),
            Decl::TextAlign(Val::Set(TextAlign::Right))
        );
        assert!(one("text-align", "diagonal").is_empty());
        assert_eq!(
            first("text-indent", "1.5em"),
            Decl::TextIndent(Val::Set(Len::Em(1.5)))
        );
        assert_eq!(
            first("text-indent", "-1em"),
            Decl::TextIndent(Val::Set(Len::Em(-1.0)))
        );
        assert_eq!(
            first("text-indent", "2em hanging"),
            Decl::TextIndent(Val::Set(Len::Em(2.0)))
        );
        assert!(one("text-indent", "auto").is_empty());
        assert_eq!(
            first("line-height", "1.4"),
            Decl::LineHeight(Val::Set(LineHeightSpec::Number(1.4)))
        );
        assert_eq!(
            first("line-height", "normal"),
            Decl::LineHeight(Val::Set(LineHeightSpec::Normal))
        );
        assert_eq!(
            first("line-height", "20px"),
            Decl::LineHeight(Val::Set(LineHeightSpec::Len(Len::Px(20.0))))
        );
        assert_eq!(
            first("line-height", "150%"),
            Decl::LineHeight(Val::Set(LineHeightSpec::Len(Len::Pct(150.0))))
        );
        assert!(one("line-height", "-1").is_empty());
        assert!(one("line-height", "tall").is_empty());
    }

    #[test]
    fn decoration_display_breaks_misc() {
        let d = |u, l, o| TextDecoration {
            underline: u,
            line_through: l,
            overline: o,
        };
        assert_eq!(
            first("text-decoration", "underline"),
            Decl::TextDecoration(Val::Set(d(true, false, false)))
        );
        assert_eq!(
            first("text-decoration", "underline line-through"),
            Decl::TextDecoration(Val::Set(d(true, true, false)))
        );
        assert_eq!(
            first("text-decoration", "none"),
            Decl::TextDecoration(Val::Set(d(false, false, false)))
        );
        assert_eq!(
            first("text-decoration", "underline wavy red"),
            Decl::TextDecoration(Val::Set(d(true, false, false)))
        );
        assert_eq!(
            first("text-decoration-line", "overline"),
            Decl::TextDecoration(Val::Set(d(false, false, true)))
        );
        assert!(one("text-decoration", "squiggle").is_empty());
        assert!(one("text-decoration", "").is_empty());
        assert!(TextDecoration::default().is_none());
        assert!(!d(true, false, false).is_none());

        assert_eq!(
            first("display", "block"),
            Decl::Display(Val::Set(Display::Block))
        );
        assert_eq!(
            first("display", "none"),
            Decl::Display(Val::Set(Display::None))
        );
        assert_eq!(
            first("display", "inline"),
            Decl::Display(Val::Set(Display::Inline))
        );
        assert_eq!(
            first("display", "list-item"),
            Decl::Display(Val::Set(Display::ListItem))
        );
        assert_eq!(
            first("display", "inline-block"),
            Decl::Display(Val::Set(Display::InlineBlock))
        );
        assert_eq!(
            first("display", "flex"),
            Decl::Display(Val::Set(Display::Block))
        );
        assert!(one("display", "wobbly").is_empty());

        assert_eq!(
            first("page-break-before", "always"),
            Decl::PageBreakBefore(Val::Set(PageBreak::Always))
        );
        assert_eq!(
            first("page-break-after", "avoid"),
            Decl::PageBreakAfter(Val::Set(PageBreak::Avoid))
        );
        assert_eq!(
            first("break-before", "page"),
            Decl::PageBreakBefore(Val::Set(PageBreak::Always))
        );
        assert_eq!(
            first("break-after", "left"),
            Decl::PageBreakAfter(Val::Set(PageBreak::Left))
        );
        assert_eq!(
            first("page-break-before", "recto"),
            Decl::PageBreakBefore(Val::Set(PageBreak::Right))
        );
        assert_eq!(
            first("page-break-after", "auto"),
            Decl::PageBreakAfter(Val::Set(PageBreak::Auto))
        );
        assert!(one("page-break-before", "sometimes").is_empty());

        for name in [
            "hyphens",
            "-webkit-hyphens",
            "-epub-hyphens",
            "-moz-hyphens",
        ] {
            assert_eq!(first(name, "none"), Decl::Hyphens(Val::Set(Hyphens::None)));
        }
        assert_eq!(
            first("hyphens", "auto"),
            Decl::Hyphens(Val::Set(Hyphens::Auto))
        );
        assert_eq!(
            first("hyphens", "manual"),
            Decl::Hyphens(Val::Set(Hyphens::Manual))
        );
        assert!(one("hyphens", "always").is_empty());

        assert_eq!(first("float", "left"), Decl::Float(Val::Set(Float::Left)));
        assert_eq!(first("float", "right"), Decl::Float(Val::Set(Float::Right)));
        assert_eq!(first("float", "none"), Decl::Float(Val::Set(Float::None)));
        assert!(one("float", "center").is_empty());
    }

    #[test]
    fn vertical_writing_properties() {
        use WritingMode::*;
        let wm = |n: &str, v: &str| first(n, v);
        assert_eq!(
            wm("writing-mode", "vertical-rl"),
            Decl::WritingMode(Val::Set(VerticalRl))
        );
        assert_eq!(
            wm("writing-mode", "vertical-lr"),
            Decl::WritingMode(Val::Set(VerticalLr))
        );
        assert_eq!(
            wm("writing-mode", "horizontal-tb"),
            Decl::WritingMode(Val::Set(HorizontalTb))
        );
        // prefixed and legacy spellings of old EPUB 3 books
        assert_eq!(
            wm("-epub-writing-mode", "vertical-rl"),
            Decl::WritingMode(Val::Set(VerticalRl))
        );
        assert_eq!(
            wm("-webkit-writing-mode", "vertical-rl"),
            Decl::WritingMode(Val::Set(VerticalRl))
        );
        assert_eq!(
            wm("writing-mode", "tb-rl"),
            Decl::WritingMode(Val::Set(VerticalRl))
        );
        assert_eq!(
            wm("writing-mode", "tb-lr"),
            Decl::WritingMode(Val::Set(VerticalLr))
        );
        assert_eq!(
            wm("writing-mode", "lr-tb"),
            Decl::WritingMode(Val::Set(HorizontalTb))
        );
        assert_eq!(
            wm("writing-mode", "inherit"),
            Decl::WritingMode(Val::Inherit)
        );
        assert!(one("writing-mode", "diagonal").is_empty());
        assert_eq!(
            first("text-orientation", "upright"),
            Decl::TextOrientation(Val::Set(TextOrientation::Upright))
        );
        assert_eq!(
            first("text-orientation", "mixed"),
            Decl::TextOrientation(Val::Set(TextOrientation::Mixed))
        );
        assert_eq!(
            first("-epub-text-orientation", "sideways"),
            Decl::TextOrientation(Val::Set(TextOrientation::Sideways))
        );
        assert!(one("text-orientation", "slanted").is_empty());
    }

    #[test]
    fn text_combine_upright_values() {
        let tc = |n: &str, v: &str| first(n, v);
        assert_eq!(
            tc("text-combine-upright", "all"),
            Decl::TextCombine(Val::Set(TextCombine::All))
        );
        assert_eq!(
            tc("text-combine-upright", "none"),
            Decl::TextCombine(Val::Set(TextCombine::None))
        );
        assert_eq!(
            tc("text-combine-upright", "digits"),
            Decl::TextCombine(Val::Set(TextCombine::Digits(2)))
        );
        assert_eq!(
            tc("text-combine-upright", "digits 4"),
            Decl::TextCombine(Val::Set(TextCombine::Digits(4)))
        );
        assert_eq!(
            tc("-epub-text-combine", "horizontal"),
            Decl::TextCombine(Val::Set(TextCombine::All))
        );
        assert_eq!(
            tc("-webkit-text-combine", "horizontal"),
            Decl::TextCombine(Val::Set(TextCombine::All))
        );
        assert!(one("text-combine-upright", "digits 9").is_empty());
        assert!(one("text-combine-upright", "sideways").is_empty());

        assert_eq!(
            first("width", "50%"),
            Decl::Width(Val::Set(Some(Len::Pct(50.0))))
        );
        assert_eq!(first("width", "auto"), Decl::Width(Val::Set(None)));
        assert_eq!(
            first("max-width", "100%"),
            Decl::MaxWidth(Val::Set(Some(Len::Pct(100.0))))
        );
        assert_eq!(first("max-width", "none"), Decl::MaxWidth(Val::Set(None)));
        assert_eq!(
            first("width", "10em"),
            Decl::Width(Val::Set(Some(Len::Em(10.0))))
        );
        assert!(one("width", "-5px").is_empty());
        assert!(one("width", "calc(1px + 2px)").is_empty());
    }

    #[test]
    fn color_property_and_global_keywords() {
        assert_eq!(
            first("color", "#336699"),
            Decl::Color(Val::Set(Color::rgb(0x33, 0x66, 0x99)))
        );
        assert_eq!(first("color", "currentColor"), Decl::Color(Val::Inherit));
        assert_eq!(first("color", "inherit"), Decl::Color(Val::Inherit));
        assert_eq!(first("color", "initial"), Decl::Color(Val::Initial));
        assert_eq!(first("color", "unset"), Decl::Color(Val::Unset));
        assert_eq!(first("color", "revert"), Decl::Color(Val::Unset));
        assert!(one("color", "glorp").is_empty());
    }

    #[test]
    fn unknown_properties_ignored_and_case_insensitive_names() {
        assert!(one("glorp-prop", "left").is_empty());
        assert!(one("", "red").is_empty());
        assert_eq!(
            first(" COLOR ", "red"),
            Decl::Color(Val::Set(Color::rgb(255, 0, 0)))
        );
    }

    #[test]
    fn background_colour_longhand_and_shorthand() {
        assert_eq!(
            first("background-color", "#ffeecc"),
            Decl::BackgroundColor(Val::Set(Color::rgb(255, 238, 204)))
        );
        assert_eq!(
            first("background", "url(x.png) no-repeat #102030 top"),
            Decl::BackgroundColor(Val::Set(Color::rgb(16, 32, 48)))
        );
        assert_eq!(
            first("background", "rgb(1, 2, 3)"),
            Decl::BackgroundColor(Val::Set(Color::rgb(1, 2, 3)))
        );
        assert_eq!(
            first("background", "none"),
            Decl::BackgroundColor(Val::Initial)
        );
        assert_eq!(
            first("background", "transparent"),
            Decl::BackgroundColor(Val::Initial)
        );
        assert_eq!(
            first("background-color", "inherit"),
            Decl::BackgroundColor(Val::Inherit)
        );
        assert!(one("background", "url(x.png) repeat-x").is_empty());
        assert!(one("background-color", "glorp").is_empty());
    }
}
