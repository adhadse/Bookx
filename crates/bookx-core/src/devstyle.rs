//! How a highlight *looks* on the devices and in Bookx: the one mapping table between
//! Kobo's `Bookmark.Color`, KOReader's `color` and `drawer`, Bookx's colour names and highlight
//! styles, and the words of the export formats. Pure functions, no I/O.
//!
//! A highlight stores `color` and `style` as strings (`Highlight::color`, `Highlight::style`).
//! A colour the device has and Bookx's palette lacks (KOReader's `olive` and `cyan`) is stored
//! **as the device wrote it**, so nothing is lost and a later export or sync restores it; painting
//! and the filters use [`display_color`], the nearest palette colour.
//!
//! | Bookx colour | Kobo `Color` | KOReader `color` |
//! |---|---|---|
//! | yellow | 0 | yellow |
//! | pink | 1 | red (nearest) |
//! | blue | 2 | blue |
//! | green | 3 | green |
//! | orange, red, purple, gray | (nearest: yellow, pink, blue, yellow) | same names |
//! | olive, cyan (kept as written) | — | olive, cyan |
//!
//! Words the tables do not know are **kept verbatim** (a device colour or style from a newer firmware, a
//! Kobo colour number above 3 as `kobo-<n>`), never turned into the default: a later sync, export or
//! repair can still hand them back unchanged. Only a value that is not a plausible word at all (empty,
//! control characters, longer than [`MAX_WORD`]) counts as "the device said nothing".
//!
//! | Bookx style | KOReader `drawer` |
//! |---|---|
//! | background | lighten |
//! | underline | underscore |
//! | strikethrough | strikeout |
//! | squiggly | underscore (nearest) |
//! | invert | invert |

pub use bookx_text::highlight::{HighlightColor, HighlightStyle};

/// The look of one highlight.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Look {
    pub color: String,
    pub style: String,
}

/// Colour names that are stored as written although Bookx's palette lacks them.
pub const EXTRA_COLORS: [(&str, (u8, u8, u8)); 2] =
    [("olive", (0x8a, 0xa6, 0x2b)), ("cyan", (0x2b, 0xc4, 0xd6))];

/// Longest colour or style word that is kept as written.
pub const MAX_WORD: usize = 32;

/// Every colour word KOReader writes in an annotation's `color` (its highlight colour menu).
pub const KOREADER_COLORS: [&str; 9] = [
    "red", "orange", "yellow", "green", "olive", "cyan", "blue", "purple", "gray",
];

/// Every `drawer` word KOReader writes, with the Bookx style each one is.
pub const KOREADER_DRAWERS: [(&str, &str); 4] = [
    ("lighten", "background"),
    ("underscore", "underline"),
    ("strikeout", "strikethrough"),
    ("invert", "invert"),
];

/// The four colours of a Kobo's `Bookmark.Color` (0 to 3).
pub const KOBO_COLORS: [&str; 4] = ["yellow", "pink", "blue", "green"];

/// A word worth keeping as written: not empty, no control characters, no `|` (the PDF style
/// separator), at most [`MAX_WORD`] characters.
fn plausible(word: &str) -> bool {
    !word.is_empty()
        && word.chars().count() <= MAX_WORD
        && !word.chars().any(|c| c.is_control() || c == '|')
}

/// What a Kobo's `Bookmark.Color` number means: the Bookx colour for 0 to 3, `kobo-<n>` (kept as
/// written) for another number a newer firmware may use, `None` for a number that cannot be one.
pub fn kobo_color(n: i64) -> Option<String> {
    match n {
        0..=3 => Some(KOBO_COLORS[n as usize].to_string()),
        4..=255 => Some(format!("kobo-{n}")),
        _ => None,
    }
}

/// The Kobo number nearest to a Bookx colour (a Kobo has four); `kobo-<n>` gives `n` back.
pub fn to_kobo_color(name: &str) -> i64 {
    if let Some(n) = name
        .trim()
        .strip_prefix("kobo-")
        .and_then(|n| n.parse::<i64>().ok())
        .filter(|n| (4..=255).contains(n))
    {
        return n;
    }
    match canonical_color(name).as_deref() {
        Some("pink") | Some("red") => 1,
        Some("blue") | Some("purple") | Some("cyan") => 2,
        Some("green") | Some("olive") => 3,
        _ => 0,
    }
}

/// A colour as a stored name: a palette name, an extra name (kept as written, lower case), a
/// `#rrggbb` value, `grey` as `gray`; any other plausible word is kept **as written** (a colour a
/// newer device has). `None` only when the value is no word at all (empty, control characters, too long).
pub fn canonical_color(raw: &str) -> Option<String> {
    let t = raw.trim();
    let s = t.to_ascii_lowercase();
    if s.is_empty() {
        return None;
    }
    if let Some(c) = HighlightColor::parse(&s) {
        return Some(match c.name() {
            Some(n) => n.to_string(),
            None => c.hex(),
        });
    }
    if let Some((n, _)) = EXTRA_COLORS.iter().find(|(n, _)| *n == s) {
        return Some(n.to_string());
    }
    plausible(t).then(|| t.to_string())
}

/// The colour a KOReader `color` word means.
pub fn koreader_color(raw: &str) -> Option<String> {
    canonical_color(raw)
}

/// The KOReader colour word for a stored colour. KOReader has no pink (red is the nearest) and no
/// custom RGB (the nearest palette name); a word Bookx does not know is handed back as written.
pub fn to_koreader_color(name: &str) -> String {
    match canonical_color(name).as_deref() {
        Some("pink") => "red".to_string(),
        Some(c) if c.starts_with('#') => nearest_koreader(display_color(c).rgb()).to_string(),
        Some(c) => c.to_string(),
        None => "yellow".to_string(),
    }
}

/// The KOReader colour word nearest to an RGB value (by the shades Bookx draws them in).
fn nearest_koreader((r, g, b): (u8, u8, u8)) -> &'static str {
    let rgb_of = |n: &str| -> (u8, u8, u8) {
        EXTRA_COLORS
            .iter()
            .find(|(e, _)| *e == n)
            .map(|(_, c)| *c)
            .or_else(|| HighlightColor::parse(n).map(HighlightColor::rgb))
            .unwrap_or((0xff, 0xd9, 0x1f))
    };
    let d = |a: u8, b: u8| (a as i32 - b as i32).pow(2);
    KOREADER_COLORS
        .iter()
        .copied()
        .min_by_key(|n| {
            let (x, y, z) = rgb_of(n);
            d(x, r) + d(y, g) + d(z, b)
        })
        .unwrap_or("yellow")
}

/// The palette colour to paint a stored colour with: the colour itself, else the nearest.
pub fn display_color(name: &str) -> HighlightColor {
    let Some(c) = canonical_color(name) else {
        return HighlightColor::Yellow;
    };
    if let Some(p) = HighlightColor::parse(&c) {
        return p;
    }
    match c.as_str() {
        "olive" => HighlightColor::Green,
        "cyan" => HighlightColor::Blue,
        _ => HighlightColor::Yellow,
    }
}

/// The colour as drawn, `0..=1` per channel. Extra colours are drawn in their own shade.
pub fn display_rgb(name: &str) -> (f64, f64, f64) {
    if let Some(c) = canonical_color(name) {
        if let Some((_, (r, g, b))) = EXTRA_COLORS.iter().find(|(n, _)| *n == c) {
            return (*r as f64 / 255.0, *g as f64 / 255.0, *b as f64 / 255.0);
        }
    }
    // a word that is no colour Bookx knows is painted in the reader's default yellow
    match canonical_color(name).as_deref().map(HighlightColor::parse) {
        Some(Some(c)) => c.rgb_f64(),
        _ => DEFAULT_RGB,
    }
}

/// The yellow of the reader's colour picker, used for a colour word nobody knows.
pub const DEFAULT_RGB: (f64, f64, f64) = (1.0, 0.85, 0.12);

/// The Bookx style a stored or foreign style word means: Bookx's own names, KOReader's `drawer`
/// words, and the words of the export formats. `None` for words that are no style.
pub fn style_from_any(raw: &str) -> Option<&'static str> {
    Some(match raw.trim().to_ascii_lowercase().as_str() {
        "background" | "lighten" | "highlight" | "highlighted" => "background",
        "underline" | "underscore" | "underlined" => "underline",
        "strikethrough" | "strikeout" | "strike" | "strikeout-through" => "strikethrough",
        "squiggly" => "squiggly",
        "invert" | "inverted" => "invert",
        _ => return None,
    })
}

/// A style as a stored word: the Bookx name for every word [`style_from_any`] knows, any other
/// plausible word as written (a drawer a newer KOReader has), `None` for no word at all.
pub fn canonical_style(raw: &str) -> Option<String> {
    let t = raw.trim();
    if let Some(s) = style_from_any(t) {
        return Some(s.to_string());
    }
    plausible(t).then(|| t.to_string())
}

/// The KOReader `drawer` word for a stored style; a word Bookx does not know is handed back as written.
pub fn to_koreader_drawer(style: &str) -> String {
    match style_from_any(style) {
        Some("underline") | Some("squiggly") => "underscore".to_string(),
        Some("strikethrough") => "strikeout".to_string(),
        Some("invert") => "invert".to_string(),
        Some(_) => "lighten".to_string(),
        None => canonical_style(style).unwrap_or_else(|| "lighten".to_string()),
    }
}

/// The look a device row asks for, as stored words: `None` for a part the device did not say.
pub fn look_of(color: Option<&str>, style: Option<&str>) -> (Option<String>, Option<String>) {
    (
        color.and_then(canonical_color),
        style.and_then(canonical_style),
    )
}

/// Whether a stored look is Bookx's default (what an import used to give to everything).
pub fn is_default_look(color: &str, style: &str) -> bool {
    let c = color.trim();
    let c_default = c.is_empty() || canonical_color(c).as_deref() == Some("yellow");
    let st = style.trim();
    c_default && (st.is_empty() || style_from_any(st) == Some("background"))
}

/// A look with its missing words filled in with the defaults, for comparing.
pub fn full_look(color: &str, style: &str) -> Look {
    Look {
        color: canonical_color(color).unwrap_or_else(|| "yellow".into()),
        style: canonical_style(style).unwrap_or_else(|| "background".to_string()),
    }
}

/// Gives `color`/`style` the look `other_*` has when they are still Bookx's default and the other is
/// not: the way two copies of one highlight are united (a duplicate book merge, two devices) so the
/// copy that kept the device's colour or style is never lost to the one that only has the default.
/// Returns whether anything changed.
pub fn take_look_if_default(
    color: &mut String,
    style: &mut String,
    other_color: &str,
    other_style: &str,
) -> bool {
    if !is_default_look(color, style) || is_default_look(other_color, other_style) {
        return false;
    }
    let full = full_look(other_color, other_style);
    if full_look(color, style) == full {
        return false;
    }
    *color = full.color;
    *style = full.style;
    true
}

/// What each device said the look of a highlight was the last time it was read, kept in
/// `<library>/.bookx/device-looks.json`. A later read that says the same changes nothing, so a colour
/// the reader changed in Bookx since is never reset by a re-import; a read that says something else
/// means the device changed it later, and the device's word is taken.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeviceLooks {
    /// `"<book id>/<highlight id>"` to the look the device had.
    pub looks: std::collections::BTreeMap<String, Look>,
}

impl DeviceLooks {
    fn path(root: &std::path::Path) -> std::path::PathBuf {
        root.join(crate::history::STATE_DIR)
            .join("device-looks.json")
    }

    /// A missing or damaged file is an empty ledger.
    pub fn load(root: &std::path::Path) -> DeviceLooks {
        std::fs::read_to_string(Self::path(root))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, root: &std::path::Path) -> std::io::Result<()> {
        let p = Self::path(root);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = p.with_extension("json.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?,
        )?;
        std::fs::rename(&tmp, &p)
    }

    pub fn key(book: u64, highlight: &str) -> String {
        format!("{book}/{highlight}")
    }

    /// The ledger of one book: highlight id to look.
    pub fn of_book(&self, book: u64) -> std::collections::BTreeMap<String, Look> {
        let prefix = format!("{book}/");
        self.looks
            .iter()
            .filter_map(|(k, v)| {
                k.strip_prefix(&prefix)
                    .map(|id| (id.to_string(), v.clone()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kobo_numbers_map_both_ways() {
        for (n, name) in [(0, "yellow"), (1, "pink"), (2, "blue"), (3, "green")] {
            assert_eq!(kobo_color(n).as_deref(), Some(name));
            assert_eq!(to_kobo_color(name), n);
        }
        // a number a newer firmware may use is kept, and handed back
        assert_eq!(kobo_color(9).as_deref(), Some("kobo-9"));
        assert_eq!(to_kobo_color("kobo-9"), 9);
        assert_eq!(kobo_color(-1), None);
        assert_eq!(kobo_color(100_000), None);
        assert_eq!(to_kobo_color("red"), 1);
        assert_eq!(to_kobo_color("purple"), 2);
        assert_eq!(to_kobo_color("olive"), 3);
        assert_eq!(to_kobo_color("orange"), 0);
    }

    /// Read from the owner's Kobo (KOReader v2026.07.2, 2026-11-21): the sidecar of a book with 400 highlights
    /// holds exactly these words, and KOReader's own menu (`readerhighlight.lua`) offers the nine colours and
    /// four drawers listed in this module. Nothing the device wrote is outside the tables.
    #[test]
    fn every_look_word_seen_on_a_real_kobo_is_known() {
        for c in [
            "red", "orange", "yellow", "green", "olive", "cyan", "purple",
        ] {
            assert!(KOREADER_COLORS.contains(&c), "{c}");
            assert_eq!(koreader_color(c).as_deref(), Some(c));
            assert_eq!(to_koreader_color(&canonical_color(c).unwrap()), c);
        }
        for d in ["lighten", "underscore"] {
            assert!(KOREADER_DRAWERS.iter().any(|(k, _)| *k == d), "{d}");
            assert_eq!(to_koreader_drawer(style_from_any(d).unwrap()), d);
        }
        // KOReader's menu also has blue, gray, strikeout and invert; they must stay in the tables
        for c in ["blue", "gray"] {
            assert!(KOREADER_COLORS.contains(&c));
        }
    }

    #[test]
    fn koreader_colours_and_drawers_map_both_ways() {
        for c in ["yellow", "red", "green", "blue", "orange", "purple", "gray"] {
            assert_eq!(koreader_color(c).as_deref(), Some(c));
            assert_eq!(to_koreader_color(c), c);
        }
        assert_eq!(koreader_color("Grey").as_deref(), Some("gray"));
        // colours Bookx has no palette entry for are kept as written
        assert_eq!(koreader_color("olive").as_deref(), Some("olive"));
        assert_eq!(to_koreader_color("olive"), "olive");
        assert_eq!(to_koreader_color("cyan"), "cyan");
        assert_eq!(to_koreader_color("pink"), "red");
        // a word nobody knows is kept as written, never defaulted
        assert_eq!(koreader_color("chartreuse").as_deref(), Some("chartreuse"));
        assert_eq!(to_koreader_color("chartreuse"), "chartreuse");
        assert_eq!(koreader_color("  "), None);
        assert_eq!(koreader_color("bad\u{7}word"), None);
        for (d, s) in [
            ("lighten", "background"),
            ("underscore", "underline"),
            ("strikeout", "strikethrough"),
            ("invert", "invert"),
        ] {
            assert_eq!(style_from_any(d), Some(s));
            assert_eq!(to_koreader_drawer(s), d);
        }
        assert_eq!(to_koreader_drawer("squiggly"), "underscore");
        assert_eq!(style_from_any("highlight"), Some("background"));
        assert_eq!(style_from_any("wat"), None);
        assert_eq!(canonical_style("wat").as_deref(), Some("wat"));
        assert_eq!(to_koreader_drawer("wat"), "wat");
        assert_eq!(canonical_style("Lighten").as_deref(), Some("background"));
        assert_eq!(canonical_style(""), None);
    }

    /// The table: every colour and every drawer KOReader writes comes in, goes out as the same
    /// word, and a Bookx colour/style goes out and comes back unchanged (pink and squiggly are the
    /// two Bookx words KOReader has no word for: red and underscore are their nearest).
    #[test]
    fn every_koreader_colour_and_drawer_round_trips() {
        for c in KOREADER_COLORS {
            let stored = koreader_color(c).unwrap();
            assert_eq!(stored, c, "{c} is stored as the device wrote it");
            assert_eq!(to_koreader_color(&stored), c, "{c} goes back unchanged");
            // case and padding never change the answer
            assert_eq!(
                koreader_color(&format!(" {} ", c.to_uppercase())),
                Some(stored)
            );
        }
        for (d, s) in KOREADER_DRAWERS {
            let stored = canonical_style(d).unwrap();
            assert_eq!(stored, s);
            assert_eq!(to_koreader_drawer(&stored), d);
        }
        for name in ["yellow", "orange", "red", "purple", "blue", "green", "gray"] {
            assert_eq!(to_koreader_color(name), name);
        }
        for style in ["background", "underline", "strikethrough", "invert"] {
            assert_eq!(style_from_any(&to_koreader_drawer(style)), Some(style));
        }
        assert_eq!(to_koreader_color("pink"), "red");
        assert_eq!(to_koreader_drawer("squiggly"), "underscore");
    }

    #[test]
    fn every_kobo_colour_round_trips() {
        for (n, name) in KOBO_COLORS.iter().enumerate() {
            let stored = kobo_color(n as i64).unwrap();
            assert_eq!(&stored, name);
            assert_eq!(to_kobo_color(&stored), n as i64);
        }
        // the colours Kobo lacks go to the nearest of its four, and stay what they are in Bookx
        for (name, kobo) in [
            ("red", 1),
            ("orange", 0),
            ("purple", 2),
            ("cyan", 2),
            ("olive", 3),
            ("gray", 0),
        ] {
            assert_eq!(to_kobo_color(name), kobo, "{name}");
        }
    }

    #[test]
    fn a_custom_colour_goes_to_the_nearest_koreader_name_not_to_yellow() {
        assert_eq!(to_koreader_color("#ff0000"), "red");
        assert_eq!(to_koreader_color("#0000ff"), "blue");
        assert_eq!(to_koreader_color("#10c0d0"), "cyan");
        assert_eq!(to_koreader_color("#808080"), "gray");
        assert_eq!(to_koreader_color(""), "yellow");
    }

    #[test]
    fn taking_the_look_of_the_copy_that_has_one() {
        let (mut c, mut s) = ("yellow".to_string(), String::new());
        assert!(take_look_if_default(&mut c, &mut s, "purple", "underscore"));
        assert_eq!((c.as_str(), s.as_str()), ("purple", "underline"));
        // a look somebody chose is never replaced, and the default never replaces anything
        assert!(!take_look_if_default(&mut c, &mut s, "green", "invert"));
        let (mut c2, mut s2) = ("red".to_string(), "underline".to_string());
        assert!(!take_look_if_default(&mut c2, &mut s2, "yellow", ""));
        let (mut c3, mut s3) = ("yellow".to_string(), String::new());
        assert!(!take_look_if_default(&mut c3, &mut s3, "", ""));
        assert!(!take_look_if_default(&mut c3, &mut s3, "Yellow", "lighten"));
    }

    #[test]
    fn display_uses_the_nearest_palette_colour_and_hex_survives() {
        assert_eq!(display_color("olive"), HighlightColor::Green);
        assert_eq!(display_color("cyan"), HighlightColor::Blue);
        assert_eq!(display_color("nonsense"), HighlightColor::Yellow);
        assert_eq!(display_color("pink"), HighlightColor::Pink);
        assert_eq!(canonical_color("#FFEE00").as_deref(), Some("#ffee00"));
        assert_eq!(canonical_color("#ffd91f").as_deref(), Some("yellow"));
        let (r, g, b) = display_rgb("olive");
        assert!(g > r && g > b);
        // a word nobody knows is kept but painted in the default yellow
        assert_eq!(display_rgb("teal"), DEFAULT_RGB);
        assert_eq!(display_rgb(""), DEFAULT_RGB);
        assert_eq!(display_rgb("#ff0000"), (1.0, 0.0, 0.0));
    }

    #[test]
    fn looks_compare_with_defaults_filled_in() {
        assert!(is_default_look("", ""));
        assert!(is_default_look("yellow", "background"));
        assert!(!is_default_look("red", ""));
        assert_eq!(full_look("", ""), full_look("yellow", "background"));
        assert_ne!(full_look("red", ""), full_look("yellow", ""));
        assert_eq!(
            look_of(Some("Grey"), Some("lighten")),
            (Some("gray".into()), Some("background".into()))
        );
        assert_eq!(look_of(None, None), (None, None));
    }
}
