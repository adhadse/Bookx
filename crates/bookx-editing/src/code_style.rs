//! Text colour, background colour and block alignment for the code view (CE-350): pure functions over `&str` with
//! byte offsets that return a [`TagEdit`], like `code_format`. A colour is written as a `<span style="color:…">`
//! (or a `<span class="…">` when the book wants classes); an alignment as `style="text-align:…"` (or a class) on the
//! innermost block around the caret. Applying the same value again takes it off.

use std::ops::Range;

use crate::code_format::{FormatError, inside_markup, is_balanced};
use crate::markup::{remove_tag_attr, set_tag_attr, tag_attr};
use crate::tags::{TagEdit, TagPair, pairs};

/// Which colour property a span carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColourKind {
    Text,
    Background,
}

impl ColourKind {
    pub fn property(self) -> &'static str {
        match self {
            ColourKind::Text => "color",
            ColourKind::Background => "background-color",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            ColourKind::Text => "text",
            ColourKind::Background => "background",
        }
    }
}

/// Horizontal alignment of a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
    Justify,
}

impl Align {
    pub const ALL: [Align; 4] = [Align::Left, Align::Center, Align::Right, Align::Justify];

    pub fn css(self) -> &'static str {
        match self {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
            Align::Justify => "justify",
        }
    }

    pub fn from_id(id: &str) -> Option<Align> {
        Align::ALL.into_iter().find(|a| a.css() == id)
    }
}

/// How the style is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via<'a> {
    /// In the `style` attribute.
    Inline,
    /// As a class of this name (the style sheet defines it, see [`colour_rule`] and [`align_rule`]).
    Class(&'a str),
}

/// Block elements an alignment goes on.
const BLOCKS: [&str; 14] = [
    "p",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "blockquote",
    "li",
    "td",
    "th",
    "figure",
    "figcaption",
];

/// A colour value that is safe to write into an attribute and a rule: `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb()/rgba()/hsl()/
/// hsla()` with plain numbers, or a colour keyword.
pub fn valid_colour(c: &str) -> bool {
    let c = c.trim();
    if let Some(h) = c.strip_prefix('#') {
        return matches!(h.len(), 3 | 4 | 6 | 8) && h.chars().all(|x| x.is_ascii_hexdigit());
    }
    if let Some((f, rest)) = c.split_once('(') {
        return matches!(f, "rgb" | "rgba" | "hsl" | "hsla")
            && rest.ends_with(')')
            && rest[..rest.len() - 1]
                .chars()
                .all(|x| x.is_ascii_digit() || matches!(x, ',' | '.' | ' ' | '%' | '/'));
    }
    !c.is_empty() && c.len() <= 24 && c.chars().all(|x| x.is_ascii_alphabetic())
}

/// A class name that is safe to write: letters, digits, `-` and `_`, not starting with a digit.
pub fn valid_class(c: &str) -> bool {
    !c.is_empty()
        && !c.starts_with(|x: char| x.is_ascii_digit())
        && c.chars()
            .all(|x| x.is_ascii_alphanumeric() || x == '-' || x == '_')
}

/// `#RRGGBB` (lower case) for an 8-bit colour.
pub fn hex(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The class a colour gets in class mode: `color-ff0000` / `bg-ff0000`. `None` when `colour` is not a hex colour.
pub fn colour_class(kind: ColourKind, colour: &str) -> Option<String> {
    let h = colour.trim().strip_prefix('#')?;
    if !valid_colour(colour) {
        return None;
    }
    let p = match kind {
        ColourKind::Text => "color",
        ColourKind::Background => "bg",
    };
    Some(format!("{p}-{}", h.to_ascii_lowercase()))
}

/// The style rule that defines a class made by [`colour_class`].
pub fn colour_rule(kind: ColourKind, class: &str, colour: &str) -> String {
    format!(".{class} {{ {}: {colour}; }}\n", kind.property())
}

/// The class an alignment gets in class mode: `align-center`.
pub fn align_class(a: Align) -> String {
    format!("align-{}", a.css())
}

/// The style rule that defines an alignment class.
pub fn align_rule(class: &str, a: Align) -> String {
    format!(".{class} {{ text-align: {}; }}\n", a.css())
}

/// The value of `prop` in a `style` attribute text.
pub fn style_get(style: &str, prop: &str) -> Option<String> {
    style.split(';').find_map(|d| {
        let (k, v) = d.split_once(':')?;
        k.trim()
            .eq_ignore_ascii_case(prop)
            .then(|| v.trim().to_string())
    })
}

/// `style` with `prop` set to `value` (replacing it) or removed (`None`); other declarations stay.
pub fn style_set(style: &str, prop: &str, value: Option<&str>) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut done = false;
    for d in style.split(';').map(str::trim).filter(|d| !d.is_empty()) {
        let same = d
            .split_once(':')
            .is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case(prop));
        if same {
            if let (false, Some(v)) = (done, value) {
                out.push(format!("{prop}:{v}"));
                done = true;
            }
        } else {
            out.push(d.to_string());
        }
    }
    if let (false, Some(v)) = (done, value) {
        out.push(format!("{prop}:{v}"));
    }
    out.join(";")
}

fn has_class(tag: &str, class: &str) -> bool {
    tag_attr(tag, "class").is_some_and(|c| c.split_whitespace().any(|x| x == class))
}

fn with_class(tag: &str, class: &str, on: bool) -> String {
    let cur = tag_attr(tag, "class").unwrap_or_default();
    let mut names: Vec<&str> = cur.split_whitespace().filter(|x| *x != class).collect();
    if on {
        names.push(class);
    }
    if names.is_empty() {
        remove_tag_attr(tag, "class")
    } else {
        set_tag_attr(tag, "class", &names.join(" "))
    }
}

/// The open tag with `prop` set or removed in its `style` attribute (the attribute goes when it ends up empty).
fn tag_with_style(tag: &str, prop: &str, value: Option<&str>) -> String {
    let style = style_set(&tag_attr(tag, "style").unwrap_or_default(), prop, value);
    if style.is_empty() {
        remove_tag_attr(tag, "style")
    } else {
        set_tag_attr(tag, "style", &style)
    }
}

fn open_has_nothing(tag: &str) -> bool {
    crate::markup::tag_attrs(tag).is_empty()
}

/// Whether the tag carries the colour `kind` (any value for `Inline`, the class for `Class`).
fn carries(tag: &str, kind: ColourKind, via: Via) -> bool {
    match via {
        Via::Inline => {
            style_get(&tag_attr(tag, "style").unwrap_or_default(), kind.property()).is_some()
        }
        Via::Class(c) => has_class(tag, c),
    }
}

fn same_colour(tag: &str, kind: ColourKind, colour: &str) -> bool {
    style_get(&tag_attr(tag, "style").unwrap_or_default(), kind.property())
        .is_some_and(|v| v.eq_ignore_ascii_case(colour.trim()))
}

fn edit_open(src: &str, p: &TagPair, new_open: String) -> TagEdit {
    let close = p.close.clone().expect("closed pair");
    let a = p.open.start + new_open.len();
    let inner = close.start - p.open.end;
    TagEdit {
        range: p.open.clone(),
        replacement: new_open,
        select: a..a + inner.min(src.len()),
    }
}

fn unwrap_span(src: &str, p: &TagPair) -> TagEdit {
    let close = p.close.clone().expect("closed pair");
    let inner = src[p.open.end..close.start].to_string();
    let at = p.open.start;
    TagEdit {
        range: p.whole(),
        select: at..at + inner.len(),
        replacement: inner,
    }
}

/// Sets, replaces or removes (`colour == None`) the text or background colour of the selection (CE-350).
/// * a `span` that is the selection (its content or the whole element), or around an empty selection, is changed in
///   place: the same colour again takes it off, another colour replaces it, a span left without attributes goes;
/// * otherwise the selection is wrapped in a new `span` (an empty selection gets an empty one with the caret inside).
///
/// With `Via::Class(name)` the class takes the place of the `style` declaration.
pub fn apply_colour(
    src: &str,
    sel: Range<usize>,
    kind: ColourKind,
    colour: Option<&str>,
    via: Via,
) -> Result<TagEdit, FormatError> {
    if sel.start > sel.end
        || sel.end > src.len()
        || !src.is_char_boundary(sel.start)
        || !src.is_char_boundary(sel.end)
        || inside_markup(src, sel.start)
        || inside_markup(src, sel.end)
    {
        return Err(FormatError::NotText);
    }
    let spans: Vec<TagPair> = pairs(src)
        .into_iter()
        .filter(|p| p.name == "span" && p.close.is_some())
        .collect();
    let target = spans
        .iter()
        .filter(|p| {
            let c = p.close.clone().unwrap();
            let content = p.open.end..c.start;
            if sel.is_empty() {
                content.start <= sel.start && sel.start <= content.end
            } else {
                sel == content || sel == p.whole()
            }
        })
        .max_by_key(|p| p.open.start);
    if let Some(p) = target {
        let open = &src[p.open.clone()];
        let has = carries(open, kind, via);
        match (colour, via) {
            (None, _) => {
                let stripped = strip_colour(open, kind);
                if stripped == open {
                    return Err(FormatError::NotText);
                }
                return Ok(strip(src, p, stripped));
            }
            (Some(v), Via::Inline) => {
                if has && same_colour(open, kind, v) {
                    return Ok(strip(src, p, tag_with_style(open, kind.property(), None)));
                }
                return Ok(edit_open(
                    src,
                    p,
                    tag_with_style(open, kind.property(), Some(v.trim())),
                ));
            }
            (Some(_), Via::Class(c)) => {
                if has {
                    return Ok(strip(src, p, with_class(open, c, false)));
                }
                // another colour class of the same kind is replaced
                let prefix = c.split('-').next().unwrap_or("");
                let mut tag = open.to_string();
                if let Some(cur) = tag_attr(open, "class") {
                    for old in cur.split_whitespace() {
                        if old.starts_with(&format!("{prefix}-")) && old != c {
                            tag = with_class(&tag, old, false);
                        }
                    }
                }
                return Ok(edit_open(src, p, with_class(&tag, c, true)));
            }
        }
    }
    let Some(v) = colour else {
        return Err(FormatError::NotText);
    };
    if !is_balanced(&src[sel.clone()]) {
        return Err(FormatError::Unbalanced);
    }
    let open = match via {
        Via::Inline => format!("<span style=\"{}:{}\">", kind.property(), v.trim()),
        Via::Class(c) => format!("<span class=\"{c}\">"),
    };
    let inner = &src[sel.clone()];
    let a = sel.start + open.len();
    Ok(TagEdit {
        range: sel.clone(),
        replacement: format!("{open}{inner}</span>"),
        select: a..a + inner.len(),
    })
}

/// The open tag without the colour of `kind`: its `style` declaration and any class made by [`colour_class`].
fn strip_colour(tag: &str, kind: ColourKind) -> String {
    let mut out = tag_with_style(tag, kind.property(), None);
    let prefix = match kind {
        ColourKind::Text => "color-",
        ColourKind::Background => "bg-",
    };
    if let Some(cur) = tag_attr(&out, "class") {
        for c in cur.split_whitespace() {
            let hex = c.strip_prefix(prefix);
            if hex.is_some_and(|h| {
                matches!(h.len(), 3 | 4 | 6 | 8) && h.chars().all(|x| x.is_ascii_hexdigit())
            }) {
                out = with_class(&out, c, false);
            }
        }
    }
    out
}

/// The span with its new open tag; a span without any attribute left is dissolved.
fn strip(src: &str, p: &TagPair, new_open: String) -> TagEdit {
    if open_has_nothing(&new_open) {
        unwrap_span(src, p)
    } else {
        edit_open(src, p, new_open)
    }
}

/// The innermost block element whose content holds `pos`.
fn block_at(src: &str, pos: usize) -> Option<TagPair> {
    pairs(src)
        .into_iter()
        .filter(|p| {
            BLOCKS.contains(&p.name.as_str())
                && p.close
                    .as_ref()
                    .is_some_and(|c| p.open.end <= pos && pos <= c.start)
        })
        .max_by_key(|p| p.open.start)
}

/// The alignment of the block around `pos` as written in its `style` attribute or one of the classes in `classes`
/// (`(class, alignment)`).
pub fn align_at(src: &str, pos: usize, classes: &[(&str, Align)]) -> Option<Align> {
    let p = block_at(src, pos)?;
    let open = &src[p.open];
    if let Some(v) = style_get(&tag_attr(open, "style").unwrap_or_default(), "text-align") {
        return Align::from_id(&v.to_ascii_lowercase());
    }
    classes
        .iter()
        .find(|(c, _)| has_class(open, c))
        .map(|x| x.1)
}

/// Aligns the block around `pos` (CE-350). The same alignment again takes it off (`Align::Left` is written like
/// the others: it overrides a style sheet that centres). `None` removes any alignment. With `Via::Class(name)` the
/// class is toggled instead of the `style` declaration; `other_classes` are the alignment classes it replaces.
pub fn set_align(
    src: &str,
    pos: usize,
    align: Option<Align>,
    via: Via,
    other_classes: &[&str],
) -> Result<TagEdit, FormatError> {
    if pos > src.len() || !src.is_char_boundary(pos) || inside_markup(src, pos) {
        return Err(FormatError::NotText);
    }
    let p = block_at(src, pos).ok_or(FormatError::NotText)?;
    let open = &src[p.open.clone()];
    let new_open = match via {
        Via::Inline => {
            let cur = style_get(&tag_attr(open, "style").unwrap_or_default(), "text-align");
            let value = align.map(Align::css);
            let off =
                value.is_none() || cur.as_deref().map(str::to_ascii_lowercase).as_deref() == value;
            tag_with_style(open, "text-align", if off { None } else { value })
        }
        Via::Class(c) => {
            let mut tag = open.to_string();
            for o in other_classes.iter().filter(|o| **o != c) {
                tag = with_class(&tag, o, false);
            }
            let on = align.is_some() && !has_class(open, c);
            with_class(&tag, c, on)
        }
    };
    let at = p.open.start + new_open.len();
    Ok(TagEdit {
        range: p.open,
        replacement: new_open,
        select: at..at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng(s: &str, needle: &str) -> Range<usize> {
        let a = s.find(needle).unwrap();
        a..a + needle.len()
    }

    fn col(s: &str, sel: Range<usize>, c: &str) -> String {
        apply_colour(s, sel, ColourKind::Text, Some(c), Via::Inline)
            .unwrap()
            .apply(s)
    }

    #[test]
    fn colours_are_checked() {
        for ok in [
            "#fff",
            "#ff0000",
            "#ff000080",
            "red",
            "rgb(1, 2, 3)",
            "hsla(10,20%,30%,0.5)",
        ] {
            assert!(valid_colour(ok), "{ok}");
        }
        for bad in [
            "",
            "#ff",
            "#gggggg",
            "red;x",
            "url(x)",
            "rgb(1;2)",
            "a\"b",
            "expression(1)",
        ] {
            assert!(!valid_colour(bad), "{bad}");
        }
        assert!(
            valid_class("color-ff0000")
                && !valid_class("1a")
                && !valid_class("a b")
                && !valid_class("")
        );
        assert_eq!(hex(255, 0, 16), "#ff0010");
        assert_eq!(
            colour_class(ColourKind::Text, "#FF0000").unwrap(),
            "color-ff0000"
        );
        assert_eq!(
            colour_class(ColourKind::Background, "#00f").unwrap(),
            "bg-00f"
        );
        assert!(colour_class(ColourKind::Text, "red").is_none());
    }

    #[test]
    fn style_declarations_are_edited_one_by_one() {
        assert_eq!(
            style_get("color: red; margin:0", "color").as_deref(),
            Some("red")
        );
        assert_eq!(style_get("color:red", "background-color"), None);
        assert_eq!(
            style_set("margin:0;color:red", "color", Some("blue")),
            "margin:0;color:blue"
        );
        assert_eq!(style_set("margin:0;color:red", "color", None), "margin:0");
        assert_eq!(style_set("", "color", Some("red")), "color:red");
    }

    #[test]
    fn wraps_the_selection_in_a_coloured_span() {
        let s = "<p>hello world</p>";
        assert_eq!(
            col(s, rng(s, "world"), "#ff0000"),
            "<p>hello <span style=\"color:#ff0000\">world</span></p>"
        );
        let e = apply_colour(s, 5..5, ColourKind::Background, Some("yellow"), Via::Inline).unwrap();
        assert_eq!(
            e.apply(s),
            "<p>he<span style=\"background-color:yellow\"></span>llo world</p>"
        );
        let at = 5 + "<span style=\"background-color:yellow\">".len();
        assert_eq!(e.select, at..at, "the caret sits inside the span");
    }

    #[test]
    fn the_same_colour_again_takes_it_off_another_replaces_it() {
        let s = "<p>a <span style=\"color:#f00\">red</span> b</p>";
        assert_eq!(col(s, rng(s, "red"), "#F00"), "<p>a red b</p>");
        assert_eq!(
            col(s, rng(s, "<span style=\"color:#f00\">red</span>"), "#00f"),
            "<p>a <span style=\"color:#00f\">red</span> b</p>"
        );
        // a caret inside the span counts as the span
        assert_eq!(
            col(
                s,
                5 + "<span style=\"color:#f00\">".len() + 1
                    ..5 + "<span style=\"color:#f00\">".len() + 1,
                "#00f"
            ),
            "<p>a <span style=\"color:#00f\">red</span> b</p>"
        );
    }

    #[test]
    fn a_span_keeps_its_other_style_and_attributes() {
        let s = "<p><span class=\"k\" style=\"font-weight:bold;color:red\">x</span></p>";
        let e = apply_colour(s, rng(s, "x"), ColourKind::Text, None, Via::Inline).unwrap();
        assert_eq!(
            e.apply(s),
            "<p><span class=\"k\" style=\"font-weight:bold\">x</span></p>"
        );
        let s2 = "<p><span style=\"font-weight:bold\">x</span></p>";
        assert_eq!(
            col(s2, rng(s2, "x"), "red"),
            "<p><span style=\"font-weight:bold;color:red\">x</span></p>"
        );
    }

    #[test]
    fn removing_where_there_is_no_colour_is_an_error_and_not_a_wrap() {
        let s = "<p>plain</p>";
        assert!(apply_colour(s, rng(s, "plain"), ColourKind::Text, None, Via::Inline).is_err());
    }

    #[test]
    fn refuses_selections_that_cut_tags_or_sit_in_markup() {
        let s = "<p>a <b>b</b> c</p>";
        let a = s.find("b</b>").unwrap();
        assert!(
            apply_colour(
                s,
                rng(s, "<b>b</b>"),
                ColourKind::Text,
                Some("red"),
                Via::Inline
            )
            .is_ok(),
            "a whole element is fine"
        );
        assert_eq!(
            apply_colour(s, 5..a + 1, ColourKind::Text, Some("red"), Via::Inline),
            Err(FormatError::Unbalanced)
        );
        assert_eq!(
            apply_colour(s, 1..2, ColourKind::Text, Some("red"), Via::Inline),
            Err(FormatError::NotText)
        );
    }

    #[test]
    fn class_mode_writes_and_toggles_a_class() {
        let s = "<p>hello world</p>";
        let e = apply_colour(
            s,
            rng(s, "world"),
            ColourKind::Text,
            Some("#f00"),
            Via::Class("color-f00"),
        )
        .unwrap();
        let t = e.apply(s);
        assert_eq!(t, "<p>hello <span class=\"color-f00\">world</span></p>");
        let r = rng(&t, "world");
        let off = apply_colour(
            &t,
            r.clone(),
            ColourKind::Text,
            Some("#f00"),
            Via::Class("color-f00"),
        )
        .unwrap();
        assert_eq!(off.apply(&t), s);
        let other = apply_colour(
            &t,
            r,
            ColourKind::Text,
            Some("#00f"),
            Via::Class("color-00f"),
        )
        .unwrap();
        assert_eq!(
            other.apply(&t),
            "<p>hello <span class=\"color-00f\">world</span></p>"
        );
        assert_eq!(
            colour_rule(ColourKind::Text, "color-f00", "#f00"),
            ".color-f00 { color: #f00; }\n"
        );
        assert_eq!(
            align_rule("center", Align::Center),
            ".center { text-align: center; }\n"
        );
    }

    #[test]
    fn aligns_the_innermost_block_and_toggles() {
        let s = "<div><p>one</p><p class=\"x\">two</p></div>";
        let pos = s.find("two").unwrap() + 1;
        let e = set_align(s, pos, Some(Align::Center), Via::Inline, &[]).unwrap();
        let t = e.apply(s);
        assert_eq!(
            t,
            "<div><p>one</p><p class=\"x\" style=\"text-align:center\">two</p></div>"
        );
        assert_eq!(
            align_at(&t, t.find("two").unwrap() + 1, &[]),
            Some(Align::Center)
        );
        // the same again: off, and the empty style attribute goes
        let pos2 = t.find("two").unwrap() + 1;
        assert_eq!(
            set_align(&t, pos2, Some(Align::Center), Via::Inline, &[])
                .unwrap()
                .apply(&t),
            s
        );
        // another one replaces
        assert!(
            set_align(&t, pos2, Some(Align::Right), Via::Inline, &[])
                .unwrap()
                .apply(&t)
                .contains("text-align:right")
        );
        // none removes
        assert_eq!(
            set_align(&t, pos2, None, Via::Inline, &[])
                .unwrap()
                .apply(&t),
            s
        );
    }

    #[test]
    fn keeps_other_style_declarations_when_aligning() {
        let s = "<p style=\"margin:0\">x</p>";
        let t = set_align(s, 20, Some(Align::Justify), Via::Inline, &[])
            .unwrap()
            .apply(s);
        assert_eq!(t, "<p style=\"margin:0;text-align:justify\">x</p>");
        assert_eq!(
            set_align(&t, 39, None, Via::Inline, &[]).unwrap().apply(&t),
            s
        );
    }

    #[test]
    fn class_alignment_replaces_the_other_alignment_classes() {
        let s = "<p class=\"a-left\">x</p>";
        let others = ["a-left", "a-center", "a-right"];
        let t = set_align(s, 18, Some(Align::Center), Via::Class("a-center"), &others)
            .unwrap()
            .apply(s);
        assert_eq!(t, "<p class=\"a-center\">x</p>");
        assert_eq!(
            align_at(&t, 20, &[("a-center", Align::Center)]),
            Some(Align::Center)
        );
        let off = set_align(&t, 20, Some(Align::Center), Via::Class("a-center"), &others)
            .unwrap()
            .apply(&t);
        assert_eq!(off, "<p>x</p>");
    }

    #[test]
    fn alignment_needs_a_block_and_text() {
        assert_eq!(
            set_align("plain", 2, Some(Align::Left), Via::Inline, &[]),
            Err(FormatError::NotText)
        );
        assert_eq!(
            set_align("<p>x</p>", 1, Some(Align::Left), Via::Inline, &[]),
            Err(FormatError::NotText),
            "inside the tag"
        );
    }
}
