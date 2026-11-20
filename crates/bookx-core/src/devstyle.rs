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

/// What a Kobo's `Bookmark.Color` number means (`None` for a number it has no meaning for).
pub fn kobo_color(n: i64) -> Option<&'static str> {
    Some(match n {
        0 => "yellow",
        1 => "pink",
        2 => "blue",
        3 => "green",
        _ => return None,
    })
}

/// The Kobo number nearest to a Bookx colour (a Kobo has four).
pub fn to_kobo_color(name: &str) -> i64 {
    match canonical_color(name).as_deref() {
        Some("pink") | Some("red") => 1,
        Some("blue") | Some("purple") | Some("cyan") => 2,
        Some("green") | Some("olive") => 3,
        _ => 0,
    }
}

/// A colour as a stored name: a palette name, an extra name (kept as written, lower case), a
/// `#rrggbb` value; `None` for words that are no colour. `grey` is `gray`.
pub fn canonical_color(raw: &str) -> Option<String> {
    let s = raw.trim().to_ascii_lowercase();
    if s.is_empty() {
        return None;
    }
    if let Some(c) = HighlightColor::parse(&s) {
        return Some(match c.name() {
            Some(n) => n.to_string(),
            None => c.hex(),
        });
    }
    EXTRA_COLORS
        .iter()
        .find(|(n, _)| *n == s)
        .map(|(n, _)| n.to_string())
}

/// The colour a KOReader `color` word means.
pub fn koreader_color(raw: &str) -> Option<String> {
    canonical_color(raw)
}

/// The KOReader colour word for a stored colour (KOReader has no pink: red is the nearest).
pub fn to_koreader_color(name: &str) -> String {
    match canonical_color(name).as_deref() {
        Some("pink") => "red".to_string(),
        Some(c) if c.starts_with('#') => display_color(c)
            .name()
            .map_or("yellow", |n| if n == "pink" { "red" } else { n })
            .to_string(),
        Some(c) => c.to_string(),
        None => "yellow".to_string(),
    }
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
    display_color(name).rgb_f64()
}

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

/// The KOReader `drawer` word for a Bookx style.
pub fn to_koreader_drawer(style: &str) -> &'static str {
    match style_from_any(style).unwrap_or("background") {
        "underline" | "squiggly" => "underscore",
        "strikethrough" => "strikeout",
        "invert" => "invert",
        _ => "lighten",
    }
}

/// The look a device row asks for, as stored words: `None` for a part the device did not say.
pub fn look_of(color: Option<&str>, style: Option<&str>) -> (Option<String>, Option<String>) {
    (
        color.and_then(canonical_color),
        style.and_then(style_from_any).map(str::to_string),
    )
}

/// Whether a stored look is Bookx's default (what an import used to give to everything).
pub fn is_default_look(color: &str, style: &str) -> bool {
    (color.is_empty() || color == "yellow") && (style.is_empty() || style == "background")
}

/// A look with its missing words filled in with the defaults, for comparing.
pub fn full_look(color: &str, style: &str) -> Look {
    Look {
        color: canonical_color(color).unwrap_or_else(|| "yellow".into()),
        style: style_from_any(style).unwrap_or("background").to_string(),
    }
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
            assert_eq!(kobo_color(n), Some(name));
            assert_eq!(to_kobo_color(name), n);
        }
        assert_eq!(kobo_color(9), None);
        assert_eq!(to_kobo_color("red"), 1);
        assert_eq!(to_kobo_color("purple"), 2);
        assert_eq!(to_kobo_color("olive"), 3);
        assert_eq!(to_kobo_color("orange"), 0);
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
        assert_eq!(koreader_color("chartreuse"), None);
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
