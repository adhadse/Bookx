//! "Transform CSS": rule-based property transforms over every stylesheet, `<style>` block and inline `style`
//! attribute of the book.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use bookx_convert::transforms::css::{CssItem, parse_css};
use bookx_core::book::OpenBook;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::markup::*;

/// A colour as RGBA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

const NAMED: &[(&str, (u8, u8, u8))] = &[
    ("black", (0, 0, 0)),
    ("silver", (192, 192, 192)),
    ("gray", (128, 128, 128)),
    ("grey", (128, 128, 128)),
    ("white", (255, 255, 255)),
    ("maroon", (128, 0, 0)),
    ("red", (255, 0, 0)),
    ("purple", (128, 0, 128)),
    ("fuchsia", (255, 0, 255)),
    ("green", (0, 128, 0)),
    ("lime", (0, 255, 0)),
    ("olive", (128, 128, 0)),
    ("yellow", (255, 255, 0)),
    ("navy", (0, 0, 128)),
    ("blue", (0, 0, 255)),
    ("teal", (0, 128, 128)),
    ("aqua", (0, 255, 255)),
    ("orange", (255, 165, 0)),
    ("brown", (165, 42, 42)),
    ("pink", (255, 192, 203)),
];

/// Parses `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()`/`rgba()` and the basic colour names.
pub fn parse_color(s: &str) -> Option<Rgba> {
    let s = s.trim().to_ascii_lowercase();
    if let Some(h) = s.strip_prefix('#') {
        let d = |i: usize| {
            u8::from_str_radix(h.get(i..i + 1)?, 16)
                .ok()
                .map(|v| v * 17)
        };
        let p = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
        return match h.len() {
            3 => Some(Rgba(d(0)?, d(1)?, d(2)?, 255)),
            4 => Some(Rgba(d(0)?, d(1)?, d(2)?, d(3)?)),
            6 => Some(Rgba(p(0)?, p(2)?, p(4)?, 255)),
            8 => Some(Rgba(p(0)?, p(2)?, p(4)?, p(6)?)),
            _ => None,
        };
    }
    if let Some(args) = s
        .strip_prefix("rgba(")
        .or_else(|| s.strip_prefix("rgb("))
        .and_then(|r| r.strip_suffix(')'))
    {
        let parts: Vec<&str> = args
            .split([',', ' ', '/'])
            .filter(|p| !p.is_empty())
            .collect();
        if parts.len() < 3 || parts.len() > 4 {
            return None;
        }
        let ch = |p: &str| -> Option<u8> {
            if let Some(pc) = p.strip_suffix('%') {
                pc.parse::<f64>()
                    .ok()
                    .map(|v| (v * 2.55).round().clamp(0.0, 255.0) as u8)
            } else {
                p.parse::<f64>()
                    .ok()
                    .map(|v| v.round().clamp(0.0, 255.0) as u8)
            }
        };
        let a = match parts.get(3) {
            Some(p) => {
                let v = if let Some(pc) = p.strip_suffix('%') {
                    pc.parse::<f64>().ok()? / 100.0
                } else {
                    p.parse::<f64>().ok()?
                };
                (v * 255.0).round().clamp(0.0, 255.0) as u8
            }
            None => 255,
        };
        return Some(Rgba(ch(parts[0])?, ch(parts[1])?, ch(parts[2])?, a));
    }
    NAMED
        .iter()
        .find(|(n, _)| *n == s)
        .map(|(_, (r, g, b))| Rgba(*r, *g, *b, 255))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unit {
    Px,
    Pt,
    Em,
    Rem,
    In,
    Cm,
    Mm,
    Pc,
}

impl Unit {
    pub fn name(self) -> &'static str {
        match self {
            Unit::Px => "px",
            Unit::Pt => "pt",
            Unit::Em => "em",
            Unit::Rem => "rem",
            Unit::In => "in",
            Unit::Cm => "cm",
            Unit::Mm => "mm",
            Unit::Pc => "pc",
        }
    }

    pub fn parse(s: &str) -> Option<Unit> {
        Some(match s.to_ascii_lowercase().as_str() {
            "px" => Unit::Px,
            "pt" => Unit::Pt,
            "em" => Unit::Em,
            "rem" => Unit::Rem,
            "in" => Unit::In,
            "cm" => Unit::Cm,
            "mm" => Unit::Mm,
            "pc" => Unit::Pc,
            _ => return None,
        })
    }

    fn to_px(self, v: f64, base: f64) -> f64 {
        match self {
            Unit::Px => v,
            Unit::Pt => v * 4.0 / 3.0,
            Unit::Pc => v * 16.0,
            Unit::In => v * 96.0,
            Unit::Cm => v * 96.0 / 2.54,
            Unit::Mm => v * 96.0 / 25.4,
            Unit::Em | Unit::Rem => v * base,
        }
    }

    fn px_to_unit(self, px: f64, base: f64) -> f64 {
        match self {
            Unit::Px => px,
            Unit::Pt => px * 3.0 / 4.0,
            Unit::Pc => px / 16.0,
            Unit::In => px / 96.0,
            Unit::Cm => px * 2.54 / 96.0,
            Unit::Mm => px * 25.4 / 96.0,
            Unit::Em | Unit::Rem => px / base,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CssTransform {
    /// Delete every declaration of this property. A trailing `*` matches a prefix (`margin*`).
    RemoveProperty { name: String },
    /// Delete the declaration of `name` only where its value is exactly `value` (case-insensitive).
    RemovePropertyWithValue { name: String, value: String },
    /// Replace one colour by another in colour-bearing properties. `to` is written verbatim.
    ReplaceColor { from: String, to: String },
    /// Convert lengths from one unit to another. `base_px` is the size of 1em. `properties` limits the
    /// properties (all when empty).
    ConvertUnits {
        from: Unit,
        to: Unit,
        base_px: f64,
        properties: Vec<String>,
    },
    /// Rename a family in `font-family` and the `font` shorthand (also inside `@font-face`).
    RenameFontFamily { from: String, to: String },
    /// Multiply every `font-size` by `factor`. Not idempotent.
    ScaleFontSizes { factor: f64 },
    /// Replace a text in the value of a property (`property` empty = any).
    ReplaceValue {
        property: String,
        from: String,
        to: String,
    },
    /// "If property `name` is `from`, change it to `to`": the whole value must match (case-insensitive).
    ChangeValue {
        name: String,
        from: String,
        to: String,
    },
    /// Give every declaration of `name` the value `value`, whatever it was.
    SetProperty { name: String, value: String },
    /// In every block where `when_property` contains `when_contains` (case-insensitive), set `name` to `value`:
    /// added when missing, replaced when different. "Make family X italic" is
    /// [`CssTransform::make_family_italic`].
    AddProperty {
        name: String,
        value: String,
        when_property: String,
        when_contains: String,
    },
}

impl CssTransform {
    /// Every block that uses `family` in `font-family` gets `font-style: italic`.
    pub fn make_family_italic(family: &str) -> CssTransform {
        CssTransform::AddProperty {
            name: "font-style".into(),
            value: "italic".into(),
            when_property: "font-family".into(),
            when_contains: family.into(),
        }
    }

    /// One line for the rule list and the preview.
    pub fn describe(&self) -> String {
        match self {
            CssTransform::RemoveProperty { name } => format!("Remove the property {name}"),
            CssTransform::RemovePropertyWithValue { name, value } => {
                format!("Remove {name} where it is {value}")
            }
            CssTransform::ReplaceColor { from, to } => format!("Change the colour {from} to {to}"),
            CssTransform::ConvertUnits { from, to, .. } => {
                format!("Convert {} lengths to {}", from.name(), to.name())
            }
            CssTransform::RenameFontFamily { from, to } => {
                format!("Rename the font family {from} to {to}")
            }
            CssTransform::ScaleFontSizes { factor } => {
                format!("Multiply every font-size by {}", fmt_num(*factor, 4))
            }
            CssTransform::ReplaceValue { property, from, to } => {
                if property.is_empty() {
                    format!("Replace the text “{from}” by “{to}” in any value")
                } else {
                    format!("Replace the text “{from}” by “{to}” in {property}")
                }
            }
            CssTransform::ChangeValue { name, from, to } => {
                format!("If {name} is {from}, change it to {to}")
            }
            CssTransform::SetProperty { name, value } => format!("Set {name} to {value}"),
            CssTransform::AddProperty {
                name,
                value,
                when_property,
                when_contains,
            } => {
                format!("Where {when_property} has {when_contains}, set {name} to {value}")
            }
        }
    }
}

/// A named list of rules the user keeps (JSON on disk).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RuleSet {
    pub name: String,
    pub rules: Vec<CssTransform>,
}

impl RuleSet {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Option<RuleSet> {
        serde_json::from_str(text).ok()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransformReport {
    pub files: BTreeMap<String, usize>,
    pub total_changes: usize,
}

const COLOR_PROPS: &[&str] = &[
    "color",
    "background",
    "background-color",
    "border",
    "border-color",
    "border-top",
    "border-right",
    "border-bottom",
    "border-left",
    "border-top-color",
    "border-right-color",
    "border-bottom-color",
    "border-left-color",
    "outline",
    "outline-color",
    "text-decoration",
    "text-decoration-color",
    "text-shadow",
    "box-shadow",
    "fill",
    "stroke",
    "column-rule",
    "column-rule-color",
    "caret-color",
];

fn prop_matches(pattern: &str, name: &str) -> bool {
    let p = pattern.trim().to_ascii_lowercase();
    match p.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => p == name,
    }
}

fn url_split_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r#"(?i)url\([^)]*\)|"[^"]*"|'[^']*'"#).unwrap())
}

/// Applies `f` to the parts of a value that are outside `url(...)` and quoted strings.
fn map_outside_literals(value: &str, f: &dyn Fn(&str) -> String) -> String {
    let mut out = String::new();
    let mut last = 0;
    for m in url_split_re().find_iter(value) {
        out.push_str(&f(&value[last..m.start()]));
        out.push_str(m.as_str());
        last = m.end();
    }
    out.push_str(&f(&value[last..]));
    out
}

fn color_token_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)#[0-9a-f]{3,8}\b|rgba?\([^)]*\)|\b[a-z]+\b").unwrap())
}

fn replace_colors(value: &str, from: Rgba, to: &str) -> String {
    map_outside_literals(value, &|part| {
        color_token_re()
            .replace_all(part, |c: &regex::Captures| {
                let tok = &c[0];
                if parse_color(tok) == Some(from) {
                    to.to_string()
                } else {
                    tok.to_string()
                }
            })
            .into_owned()
    })
}

fn fmt_num(v: f64, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    };
    if s == "-0" { "0".into() } else { s }
}

fn length_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?i)(^|[^\w.#-])([+-]?(?:\d+\.?\d*|\.\d+))(px|pt|em|rem|in|cm|mm|pc)\b")
            .unwrap()
    })
}

fn convert_units(value: &str, from: Unit, to: Unit, base: f64) -> String {
    map_outside_literals(value, &|part| {
        length_re()
            .replace_all(part, |c: &regex::Captures| {
                let unit = Unit::parse(&c[3]);
                match (unit, c[2].parse::<f64>()) {
                    (Some(u), Ok(n)) if u == from => {
                        let px = from.to_px(n, base);
                        let out = to.px_to_unit(px, base);
                        let dec = if matches!(to, Unit::Em | Unit::Rem | Unit::In | Unit::Cm) {
                            3
                        } else {
                            4
                        };
                        format!("{}{}{}", &c[1], fmt_num(out, dec), to.name())
                    }
                    _ => c[0].to_string(),
                }
            })
            .into_owned()
    })
}

fn unquote_family(f: &str) -> (&str, Option<char>) {
    let t = f.trim();
    for q in ['"', '\''] {
        if t.len() >= 2 && t.starts_with(q) && t.ends_with(q) {
            return (&t[1..t.len() - 1], Some(q));
        }
    }
    (t, None)
}

fn split_commas(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut quote: Option<char> = None;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"') | (None, '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            (None, ',') if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

fn rename_family_list(list: &str, from: &str, to: &str) -> String {
    let lead: String = list.chars().take_while(|c| c.is_whitespace()).collect();
    if !split_commas(list.trim())
        .into_iter()
        .any(|p| unquote_family(p).0.eq_ignore_ascii_case(from))
    {
        return list.to_string();
    }
    let parts: Vec<String> = split_commas(list.trim())
        .into_iter()
        .map(|p| {
            let (name, q) = unquote_family(p);
            if name.eq_ignore_ascii_case(from) {
                let quote = q.unwrap_or('"');
                if q.is_some() || to.contains(char::is_whitespace) {
                    format!("{quote}{to}{quote}")
                } else {
                    to.to_string()
                }
            } else {
                p.trim().to_string()
            }
        })
        .collect();
    format!("{lead}{}", parts.join(", "))
}

fn size_token_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"(?i)(^|\s)([+-]?(?:\d+\.?\d*|\.\d+))(px|pt|em|rem|%|in|cm|mm|pc)(/[^\s,]+)?(\s)",
        )
        .unwrap()
    })
}

fn rename_family_in_font_shorthand(value: &str, from: &str, to: &str) -> String {
    match size_token_re().find(&format!("{value} ")) {
        Some(m) => {
            let padded = format!("{value} ");
            let head = &padded[..m.end()];
            let tail = padded[m.end()..].trim_end();
            format!("{head}{}", rename_family_list(tail, from, to))
        }
        None => value.to_string(),
    }
}

fn scale_size(value: &str, factor: f64) -> String {
    let lead: String = value.chars().take_while(|c| c.is_whitespace()).collect();
    let v = value.trim();
    static R: OnceLock<Regex> = OnceLock::new();
    let re = R.get_or_init(|| {
        Regex::new(r"(?i)^([+-]?(?:\d+\.?\d*|\.\d+))(px|pt|em|rem|%|in|cm|mm|pc)(\s*!important)?$")
            .unwrap()
    });
    match re.captures(v) {
        Some(c) => {
            let n: f64 = c[1].parse().unwrap_or(0.0);
            format!(
                "{lead}{}{}{}",
                fmt_num(n * factor, 4),
                &c[2],
                c.get(3).map(|m| m.as_str()).unwrap_or("")
            )
        }
        None => value.to_string(),
    }
}

fn scale_font_shorthand(value: &str, factor: f64) -> String {
    let padded = format!("{value} ");
    match size_token_re().captures(&padded) {
        Some(c) => {
            let m = c.get(2).unwrap();
            let unit = c.get(3).unwrap();
            let n: f64 = c[2].parse().unwrap_or(0.0);
            let mut out = String::new();
            out.push_str(&padded[..m.start()]);
            out.push_str(&fmt_num(n * factor, 4));
            out.push_str(unit.as_str());
            out.push_str(&padded[unit.end()..]);
            out.trim_end().to_string()
        }
        None => value.to_string(),
    }
}

/// `new` with the white space that surrounded `old`.
fn with_spacing_of(old: &str, new: &str) -> String {
    let lead = &old[..old.len() - old.trim_start().len()];
    let trail = &old[old.trim_end().len()..];
    format!("{lead}{new}{trail}")
}

/// What a transform does to one declaration.
enum Outcome {
    Keep,
    Remove,
    NewValue(String),
}

fn apply_one(t: &CssTransform, name: &str, value: &str) -> Outcome {
    let value_trim = value.trim().trim_end_matches("!important").trim();
    match t {
        CssTransform::RemoveProperty { name: n } => {
            if prop_matches(n, name) {
                Outcome::Remove
            } else {
                Outcome::Keep
            }
        }
        CssTransform::RemovePropertyWithValue { name: n, value: v } => {
            if prop_matches(n, name) && value_trim.eq_ignore_ascii_case(v.trim()) {
                Outcome::Remove
            } else {
                Outcome::Keep
            }
        }
        CssTransform::ReplaceColor { from, to } => {
            let Some(f) = parse_color(from) else {
                return Outcome::Keep;
            };
            if !COLOR_PROPS.contains(&name) {
                return Outcome::Keep;
            }
            let nv = replace_colors(value, f, to);
            if nv != value {
                Outcome::NewValue(nv)
            } else {
                Outcome::Keep
            }
        }
        CssTransform::ConvertUnits {
            from,
            to,
            base_px,
            properties,
        } => {
            if from == to
                || (!properties.is_empty() && !properties.iter().any(|p| prop_matches(p, name)))
            {
                return Outcome::Keep;
            }
            let nv = convert_units(value, *from, *to, *base_px);
            if nv != value {
                Outcome::NewValue(nv)
            } else {
                Outcome::Keep
            }
        }
        CssTransform::RenameFontFamily { from, to } => {
            let nv = match name {
                "font-family" => rename_family_list(value, from, to),
                "font" => rename_family_in_font_shorthand(value, from, to),
                _ => return Outcome::Keep,
            };
            if nv != value {
                Outcome::NewValue(nv)
            } else {
                Outcome::Keep
            }
        }
        CssTransform::ScaleFontSizes { factor } => {
            let nv = match name {
                "font-size" => scale_size(value, *factor),
                "font" => scale_font_shorthand(value, *factor),
                _ => return Outcome::Keep,
            };
            if nv != value {
                Outcome::NewValue(nv)
            } else {
                Outcome::Keep
            }
        }
        CssTransform::ChangeValue { name: n, from, to } => {
            if prop_matches(n, name) && value_trim.eq_ignore_ascii_case(from.trim()) {
                let important = if value.trim().ends_with("!important") {
                    " !important"
                } else {
                    ""
                };
                Outcome::NewValue(with_spacing_of(value, &format!("{}{important}", to.trim())))
            } else {
                Outcome::Keep
            }
        }
        CssTransform::SetProperty { name: n, value: v } => {
            if prop_matches(n, name) && v.trim() != value_trim {
                Outcome::NewValue(with_spacing_of(value, v.trim()))
            } else {
                Outcome::Keep
            }
        }
        CssTransform::AddProperty { .. } => Outcome::Keep,
        CssTransform::ReplaceValue { property, from, to } => {
            if from.is_empty() || (!property.is_empty() && !prop_matches(property, name)) {
                return Outcome::Keep;
            }
            let nv = value.replace(from.as_str(), to);
            if nv != value {
                Outcome::NewValue(nv)
            } else {
                Outcome::Keep
            }
        }
    }
}

/// Splits declaration text (the inside of `{ }` or a `style` attribute) into raw segments at top-level `;`.
pub(crate) fn split_declarations(inner: &str) -> Vec<&str> {
    let b = inner.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut quote = 0u8;
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if quote != 0 {
            if c == b'\\' {
                i += 1;
            } else if c == quote {
                quote = 0;
            }
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i = inner[i + 2..]
                .find("*/")
                .map(|p| i + 2 + p + 1)
                .unwrap_or(b.len());
        } else if c == b'(' {
            depth += 1;
        } else if c == b')' {
            depth -= 1;
        } else if c == b';' && depth <= 0 {
            out.push(&inner[start..i + 1]);
            start = i + 1;
        }
        i += 1;
    }
    if start < inner.len() {
        out.push(&inner[start..]);
    }
    out
}

/// Transforms declaration text and returns it with the number of declarations that changed or were removed.
pub fn transform_declarations(inner: &str, ts: &[CssTransform]) -> (String, usize) {
    let mut out = String::with_capacity(inner.len());
    let mut changes = 0;
    let segments = split_declarations(inner);
    let last_index = segments.len().saturating_sub(1);
    for (idx, seg) in segments.into_iter().enumerate() {
        let body = seg.strip_suffix(';').unwrap_or(seg);
        let semi = &seg[body.len()..];
        let Some(colon) = body.find(':') else {
            out.push_str(seg);
            continue;
        };
        let (lead_and_name, value) = (&body[..colon], &body[colon + 1..]);
        let name = lead_and_name.trim().to_ascii_lowercase();
        if name.is_empty() || name.starts_with("/*") || name.contains('{') {
            out.push_str(seg);
            continue;
        }
        let mut cur = value.to_string();
        let mut removed = false;
        let mut modified = false;
        for t in ts {
            match apply_one(t, &name, &cur) {
                Outcome::Keep => {}
                Outcome::Remove => {
                    removed = true;
                    break;
                }
                Outcome::NewValue(v) => {
                    cur = v;
                    modified = true;
                }
            }
        }
        if removed {
            changes += 1;
            if idx == last_index {
                // keep the whitespace before the closing brace
                out.push_str(&seg[seg.trim_end().len()..]);
            }
            continue;
        }
        if modified {
            changes += 1;
        }
        out.push_str(lead_and_name);
        out.push(':');
        out.push_str(&cur);
        out.push_str(semi);
    }
    for t in ts {
        if let CssTransform::AddProperty {
            name,
            value,
            when_property,
            when_contains,
        } = t
        {
            let (o, c) = add_property(&out, name, value, when_property, when_contains);
            out = o;
            changes += c;
        }
    }
    (out, changes)
}

/// Declaration names and values of a block, lower-cased names.
fn declared(inner: &str) -> Vec<(String, String)> {
    split_declarations(inner)
        .into_iter()
        .filter_map(|seg| {
            let body = seg.strip_suffix(';').unwrap_or(seg);
            let (n, v) = body.split_once(':')?;
            let n = n.trim().to_ascii_lowercase();
            (!n.is_empty() && !n.starts_with("/*")).then(|| (n, v.trim().to_string()))
        })
        .collect()
}

fn add_property(
    inner: &str,
    name: &str,
    value: &str,
    when_property: &str,
    when_contains: &str,
) -> (String, usize) {
    let decls = declared(inner);
    let needle = when_contains.trim().to_ascii_lowercase();
    let applies = decls
        .iter()
        .any(|(n, v)| prop_matches(when_property, n) && v.to_ascii_lowercase().contains(&needle));
    if !applies {
        return (inner.to_string(), 0);
    }
    let name_l = name.trim().to_ascii_lowercase();
    match decls.iter().find(|(n, _)| *n == name_l) {
        Some((_, v)) if v.trim_end_matches("!important").trim() == value.trim() => {
            (inner.to_string(), 0)
        }
        Some(_) => transform_declarations(
            inner,
            &[CssTransform::SetProperty {
                name: name.to_string(),
                value: value.to_string(),
            }],
        ),
        None => {
            let trimmed = inner.trim_end();
            let tail = &inner[trimmed.len()..];
            let sep = if trimmed.ends_with(';') || trimmed.is_empty() {
                ""
            } else {
                ";"
            };
            // keep the layout of a multi-line block
            let glue = if inner.contains('\n') { "\n  " } else { " " };
            let tail = if tail.is_empty() && !inner.contains('\n') {
                " "
            } else {
                tail
            };
            (
                format!(
                    "{trimmed}{sep}{}{}: {};{tail}",
                    if trimmed.is_empty() { "" } else { glue },
                    name.trim(),
                    value.trim()
                ),
                1,
            )
        }
    }
}

fn block_inner(block: &str) -> Option<(&str, &str, &str)> {
    let open = block.find('{')?;
    let close = block.rfind('}')?;
    (close > open).then(|| (&block[..=open], &block[open + 1..close], &block[close..]))
}

/// Transforms a whole stylesheet. Rules inside `@media`/`@supports` are included.
pub fn transform_stylesheet(css: &str, ts: &[CssTransform]) -> (String, usize) {
    let mut out = String::with_capacity(css.len());
    let mut changes = 0;
    for item in parse_css(css) {
        match item {
            CssItem::Rule { selectors, block } => {
                out.push_str(selectors);
                match block_inner(block) {
                    Some((open, inner, close)) => {
                        let (new, n) = transform_declarations(inner, ts);
                        changes += n;
                        out.push_str(open);
                        out.push_str(&new);
                        out.push_str(close);
                    }
                    None => out.push_str(block),
                }
            }
            CssItem::At(s) => {
                let lower = s.to_ascii_lowercase();
                match block_inner(s) {
                    Some((open, inner, close))
                        if lower.starts_with("@media")
                            || lower.starts_with("@supports")
                            || lower.starts_with("@document") =>
                    {
                        let (new, n) = transform_stylesheet(inner, ts);
                        changes += n;
                        out.push_str(open);
                        out.push_str(&new);
                        out.push_str(close);
                    }
                    Some((open, inner, close))
                        if lower.starts_with("@font-face") || lower.starts_with("@page") =>
                    {
                        let (new, n) = transform_declarations(inner, ts);
                        changes += n;
                        out.push_str(open);
                        out.push_str(&new);
                        out.push_str(close);
                    }
                    _ => out.push_str(s),
                }
            }
            CssItem::Other(s) => out.push_str(s),
        }
    }
    (out, changes)
}

/// Transforms the CSS of a markup document: `<style>` blocks and `style` attributes.
pub fn transform_markup_styles(src: &str, ts: &[CssTransform]) -> (String, usize) {
    let mut changes = 0;
    let after_blocks = map_style_contents(src, &mut |css| {
        let (n, c) = transform_stylesheet(css, ts);
        changes += c;
        n
    });
    let after_attrs = rewrite_all_tags(&after_blocks, &mut |_, attr, val| {
        if attr != "style" {
            return None;
        }
        let (n, c) = transform_declarations(val, ts);
        changes += c;
        let n = if val.starts_with(char::is_whitespace) {
            n
        } else {
            n.trim_start().to_string()
        };
        (c > 0).then_some(n)
    });
    (after_attrs, changes)
}

/// Applies the transforms to every stylesheet, `<style>` block and inline style of the book.
pub fn transform_book(book: &mut OpenBook, ts: &[CssTransform]) -> TransformReport {
    let mut rep = TransformReport::default();
    for name in book.archive.names().to_vec() {
        let Some(text) = book.archive.get_string(&name) else {
            continue;
        };
        let (new, n) = if is_css_path(&name) {
            transform_stylesheet(&text, ts)
        } else if is_markup_path(&name) {
            transform_markup_styles(&text, ts)
        } else {
            continue;
        };
        if n > 0 && new != text {
            book.archive.set_string(&name, &new);
            rep.files.insert(name, n);
            rep.total_changes += n;
        }
    }
    rep
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::make_book;

    #[test]
    fn color_parsing() {
        assert_eq!(parse_color("#f00"), Some(Rgba(255, 0, 0, 255)));
        assert_eq!(parse_color("#FF0000"), Some(Rgba(255, 0, 0, 255)));
        assert_eq!(parse_color("#ff000080"), Some(Rgba(255, 0, 0, 128)));
        assert_eq!(parse_color("#f008"), Some(Rgba(255, 0, 0, 136)));
        assert_eq!(parse_color("rgb(255, 0, 0)"), Some(Rgba(255, 0, 0, 255)));
        assert_eq!(
            parse_color("rgba(100%, 0%, 0%, 0.5)"),
            Some(Rgba(255, 0, 0, 128))
        );
        assert_eq!(
            parse_color("rgb(255 0 0 / 50%)"),
            Some(Rgba(255, 0, 0, 128))
        );
        assert_eq!(parse_color("RED"), Some(Rgba(255, 0, 0, 255)));
        assert_eq!(parse_color("#12"), None);
        assert_eq!(parse_color("#ggg"), None);
        assert_eq!(parse_color("rgb(1,2)"), None);
        assert_eq!(parse_color("notacolor"), None);
    }

    #[test]
    fn remove_property_with_wildcard_and_value() {
        let ts = [
            CssTransform::RemoveProperty {
                name: "margin*".into(),
            },
            CssTransform::RemovePropertyWithValue {
                name: "display".into(),
                value: "NONE".into(),
            },
        ];
        let (out, n) = transform_declarations(
            " margin: 0; margin-left: 1em; color: red; display: none !important; display: block; padding:0 ",
            &ts,
        );
        assert_eq!(n, 3);
        assert_eq!(out, " color: red; display: block; padding:0 ");
        let (same, n) = transform_declarations(&out, &ts);
        assert_eq!((same.as_str(), n), (out.as_str(), 0), "idempotent");
    }

    #[test]
    fn replace_colors_everywhere_but_urls_and_fonts() {
        let ts = [CssTransform::ReplaceColor {
            from: "red".into(),
            to: "#0000ff".into(),
        }];
        let (out, n) = transform_declarations(
            "color: #F00; background: url(red.png) rgb(255,0,0); border: 1px solid RED; font-family: red; fill: #00f",
            &ts,
        );
        assert_eq!(n, 3);
        assert_eq!(
            out,
            "color: #0000ff; background: url(red.png) #0000ff; border: 1px solid #0000ff; font-family: red; fill: #00f"
        );
        let bad = [CssTransform::ReplaceColor {
            from: "nonsense".into(),
            to: "x".into(),
        }];
        assert_eq!(
            transform_declarations("color:red", &bad),
            ("color:red".to_string(), 0)
        );
    }

    #[test]
    fn unit_conversion() {
        let ts = [CssTransform::ConvertUnits {
            from: Unit::Pt,
            to: Unit::Px,
            base_px: 16.0,
            properties: vec![],
        }];
        let (out, n) = transform_declarations(
            "font-size: 12pt; margin: 0 .75pt -3pt; width: 10em; background: url(a.5pt.png); line-height:1.5",
            &ts,
        );
        assert_eq!(n, 2);
        assert_eq!(
            out,
            "font-size: 16px; margin: 0 1px -4px; width: 10em; background: url(a.5pt.png); line-height:1.5"
        );
        let em = [CssTransform::ConvertUnits {
            from: Unit::Px,
            to: Unit::Em,
            base_px: 16.0,
            properties: vec!["margin*".into()],
        }];
        let (out, _) = transform_declarations("margin-left: 24px; font-size: 24px", &em);
        assert_eq!(out, "margin-left: 1.5em; font-size: 24px");
        let back = [CssTransform::ConvertUnits {
            from: Unit::Em,
            to: Unit::Pt,
            base_px: 16.0,
            properties: vec![],
        }];
        assert_eq!(
            transform_declarations("font-size: 1.5em", &back).0,
            "font-size: 18pt"
        );
        let same = [CssTransform::ConvertUnits {
            from: Unit::Pt,
            to: Unit::Pt,
            base_px: 16.0,
            properties: vec![],
        }];
        assert_eq!(transform_declarations("a: 3pt", &same).1, 0);
        assert_eq!(
            transform_declarations(
                "margin: 1in 2.54cm 25.4mm 1pc",
                &[CssTransform::ConvertUnits {
                    from: Unit::In,
                    to: Unit::Px,
                    base_px: 16.0,
                    properties: vec![]
                }]
            )
            .0,
            "margin: 96px 2.54cm 25.4mm 1pc"
        );
        assert_eq!(Unit::parse("PT"), Some(Unit::Pt));
        assert_eq!(Unit::parse("vh"), None);
    }

    #[test]
    fn unit_conversion_idempotent() {
        let ts = [CssTransform::ConvertUnits {
            from: Unit::Pt,
            to: Unit::Px,
            base_px: 16.0,
            properties: vec![],
        }];
        let (once, _) = transform_stylesheet(
            "p { margin: 10pt 7.5pt } @media screen { h1 { font-size: 20pt } }",
            &ts,
        );
        let (twice, n) = transform_stylesheet(&once, &ts);
        assert_eq!(once, twice);
        assert_eq!(n, 0);
        assert!(
            once.contains("margin: 13.3333px 10px") && once.contains("font-size: 26.6667px"),
            "{once}"
        );
    }

    #[test]
    fn font_family_rename() {
        let ts = [CssTransform::RenameFontFamily {
            from: "Times New Roman".into(),
            to: "Georgia".into(),
        }];
        let (out, n) = transform_declarations(
            "font-family: 'Times New Roman', Times, serif; font: italic 12px/1.5 \"times new roman\", serif; color: red",
            &ts,
        );
        assert_eq!(n, 2);
        assert_eq!(
            out,
            "font-family: 'Georgia', Times, serif; font: italic 12px/1.5 \"Georgia\", serif; color: red"
        );
        let to_space = [CssTransform::RenameFontFamily {
            from: "Arial".into(),
            to: "Open Sans".into(),
        }];
        assert_eq!(
            transform_declarations("font-family: Arial, sans-serif", &to_space).0,
            "font-family: \"Open Sans\", sans-serif"
        );
        assert_eq!(
            transform_declarations(
                &transform_declarations("font-family: Arial", &to_space).0,
                &to_space
            )
            .1,
            0
        );
        let (css, n) = transform_stylesheet(
            "@font-face { font-family: Arial; src: url(a.ttf) }",
            &to_space,
        );
        assert_eq!(n, 1);
        assert!(css.contains("font-family: \"Open Sans\""));
    }

    #[test]
    fn font_size_scaling() {
        let ts = [CssTransform::ScaleFontSizes { factor: 1.25 }];
        let (out, n) = transform_declarations(
            "font-size: 12px; font-size: 1.2em !important; font-size: larger; font: bold 10pt/14pt serif; margin: 10px",
            &ts,
        );
        assert_eq!(n, 3);
        assert_eq!(
            out,
            "font-size: 15px; font-size: 1.5em !important; font-size: larger; font: bold 12.5pt/14pt serif; margin: 10px"
        );
        assert_eq!(
            transform_declarations("font-size: 100%", &ts).0,
            "font-size: 125%"
        );
    }

    #[test]
    fn replace_value() {
        let ts = [
            CssTransform::ReplaceValue {
                property: "text-align".into(),
                from: "justify".into(),
                to: "left".into(),
            },
            CssTransform::ReplaceValue {
                property: "".into(),
                from: "".into(),
                to: "x".into(),
            },
        ];
        assert_eq!(
            transform_declarations("text-align: justify; width: justify", &ts),
            ("text-align: left; width: justify".to_string(), 1)
        );
    }

    #[test]
    fn transforms_compose_in_order() {
        let ts = [
            CssTransform::ReplaceColor {
                from: "red".into(),
                to: "blue".into(),
            },
            CssTransform::RemovePropertyWithValue {
                name: "color".into(),
                value: "blue".into(),
            },
        ];
        assert_eq!(
            transform_declarations("color: red; border-color: red", &ts),
            (" border-color: blue".to_string(), 2)
        );
    }

    #[test]
    fn declaration_splitting_is_robust() {
        let ts = [CssTransform::RemoveProperty {
            name: "color".into(),
        }];
        let (out, n) = transform_declarations(
            "background: url(\"a;b.png\"); /* color: red; */ content: \"a;color:red\"; color: red",
            &ts,
        );
        assert_eq!(n, 1);
        assert_eq!(
            out,
            "background: url(\"a;b.png\"); /* color: red; */ content: \"a;color:red\";"
        );
        assert_eq!(transform_declarations("", &ts), (String::new(), 0));
        assert_eq!(
            transform_declarations("junk without colon", &ts),
            ("junk without colon".to_string(), 0)
        );
    }

    #[test]
    fn stylesheet_structure_preserved() {
        let css = "/* c */\n@import url(x.css);\nbody {\n  color: red;\n  margin: 0;\n}\n@media print {\n  p { color: red }\n}\n@page { margin: 2cm }\n";
        let ts = [
            CssTransform::RemoveProperty {
                name: "margin".into(),
            },
            CssTransform::ReplaceColor {
                from: "red".into(),
                to: "green".into(),
            },
        ];
        let (out, n) = transform_stylesheet(css, &ts);
        assert_eq!(n, 4);
        assert_eq!(
            out,
            "/* c */\n@import url(x.css);\nbody {\n  color: green;\n}\n@media print {\n  p { color: green }\n}\n@page { }\n"
        );
        let (again, n2) = transform_stylesheet(&out, &ts);
        assert_eq!((again.as_str(), n2), (out.as_str(), 0));
        assert_eq!(transform_stylesheet("", &ts).1, 0);
        assert_eq!(
            transform_stylesheet("p { color: red", &ts).0,
            "p { color: red"
        );
    }

    #[test]
    fn book_wide_css_style_blocks_and_attributes() {
        let mut b = make_book(&[
            "<p style=\"color: red; margin: 0\">x</p><p style='font-family: Arial'>y</p>",
        ]);
        b.archive
            .set_string("OEBPS/style.css", "p { color: red }\n");
        let t = b
            .archive
            .get_string("OEBPS/chapter-1.xhtml")
            .unwrap()
            .replace("</head>", "<style>h1 { color: #f00 }</style></head>");
        b.archive.set_string("OEBPS/chapter-1.xhtml", &t);
        let ts = [
            CssTransform::ReplaceColor {
                from: "red".into(),
                to: "#00f".into(),
            },
            CssTransform::RenameFontFamily {
                from: "Arial".into(),
                to: "Helvetica".into(),
            },
        ];
        let rep = transform_book(&mut b, &ts);
        assert_eq!(rep.total_changes, 4);
        assert_eq!(rep.files.len(), 2);
        assert_eq!(
            b.archive.get_string("OEBPS/style.css").unwrap(),
            "p { color: #00f }\n"
        );
        let ch = b.archive.get_string("OEBPS/chapter-1.xhtml").unwrap();
        assert!(ch.contains("<style>h1 { color: #00f }</style>"));
        assert!(ch.contains("style=\"color: #00f; margin: 0\""));
        assert!(ch.contains("style='font-family: Helvetica'"), "{ch}");
        roxmltree::Document::parse_with_options(
            &ch,
            roxmltree::ParsingOptions {
                allow_dtd: true,
                ..Default::default()
            },
        )
        .unwrap();
        let again = transform_book(&mut b, &ts);
        assert_eq!(again.total_changes, 0);
        assert!(again.files.is_empty());
    }

    #[test]
    fn change_value_needs_the_whole_value() {
        let ts = [CssTransform::ChangeValue {
            name: "color".into(),
            from: "red".into(),
            to: "green".into(),
        }];
        let (o, n) = transform_stylesheet(
            "p { color: red } q { color: darkred } r { color: RED !important }",
            &ts,
        );
        assert_eq!(n, 2);
        assert_eq!(
            o,
            "p { color: green } q { color: darkred } r { color: green !important }"
        );
    }

    #[test]
    fn set_property_whatever_the_value() {
        let ts = [CssTransform::SetProperty {
            name: "text-align".into(),
            value: "left".into(),
        }];
        let (o, n) = transform_stylesheet(
            "p { text-align: justify; margin: 0 } q { text-align: left }",
            &ts,
        );
        assert_eq!(n, 1);
        assert_eq!(
            o,
            "p { text-align: left; margin: 0 } q { text-align: left }"
        );
    }

    #[test]
    fn family_italic_adds_replaces_and_is_idempotent() {
        let ts = [CssTransform::make_family_italic("Georgia")];
        let css = "p { font-family: Georgia, serif }
q { font-family: Arial }
r {
  font-family: \"Georgia\";
  font-style: normal;
}
s { font-style: italic; font-family: georgia }
";
        let (o, n) = transform_stylesheet(css, &ts);
        assert_eq!(n, 2, "{o}");
        assert!(
            o.contains("p { font-family: Georgia, serif; font-style: italic; }"),
            "{o}"
        );
        assert!(o.contains("q { font-family: Arial }"));
        assert!(o.contains("font-style: italic;\n}"), "{o}");
        assert!(!o.contains("normal"));
        let (again, n2) = transform_stylesheet(&o, &ts);
        assert_eq!(n2, 0);
        assert_eq!(again, o);
    }

    #[test]
    fn rule_sets_roundtrip_and_describe() {
        let set = RuleSet {
            name: "Mine".into(),
            rules: vec![
                CssTransform::ScaleFontSizes { factor: 1.25 },
                CssTransform::ChangeValue {
                    name: "color".into(),
                    from: "red".into(),
                    to: "green".into(),
                },
                CssTransform::make_family_italic("Georgia"),
            ],
        };
        assert_eq!(RuleSet::from_json(&set.to_json()), Some(set.clone()));
        assert_eq!(RuleSet::from_json("nonsense"), None);
        assert_eq!(set.rules[0].describe(), "Multiply every font-size by 1.25");
        assert_eq!(
            set.rules[1].describe(),
            "If color is red, change it to green"
        );
    }

    #[test]
    fn number_formatting() {
        assert_eq!(fmt_num(16.0, 4), "16");
        assert_eq!(fmt_num(13.33333, 4), "13.3333");
        assert_eq!(fmt_num(-0.00001, 4), "0");
        assert_eq!(fmt_num(1.5, 3), "1.5");
        assert_eq!(split_commas("a, 'b,c', d(1,2), e").len(), 4);
    }
}
