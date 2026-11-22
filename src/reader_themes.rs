//! The reader's own colour themes (R47): named text and page colours made in the theme editor,
//! shared by every book, with import and export as JSON. Without GTK. A theme is applied as the
//! reader's "custom colours".

use serde::{Deserialize, Serialize};

use crate::config::{ColorMode, ReaderPrefs, parse_hex};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    /// Text colour, `#rrggbb`.
    pub fg: String,
    /// Page colour, `#rrggbb`.
    pub bg: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Themes {
    pub list: Vec<Theme>,
}

/// Relative luminance of a `#rrggbb` colour (0 black, 1 white).
pub fn luminance(hex: &str) -> Option<f64> {
    let (r, g, b) = parse_hex(hex)?;
    let lin = |c: f64| {
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    Some(0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b))
}

/// WCAG contrast ratio of two colours (1 to 21).
pub fn contrast(fg: &str, bg: &str) -> Option<f64> {
    let (a, b) = (luminance(fg)?, luminance(bg)?);
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    Some((hi + 0.05) / (lo + 0.05))
}

/// What to say about the readability of a pair: `None` when it is fine.
pub fn warning(fg: &str, bg: &str) -> Option<String> {
    let c = contrast(fg, bg)?;
    if c < 3.0 {
        Some(format!(
            "Contrast {c:.1}:1 is too low to read comfortably (aim for 4.5:1 or more)."
        ))
    } else if c < 4.5 {
        Some(format!(
            "Contrast {c:.1}:1 is a little low (4.5:1 or more reads best)."
        ))
    } else {
        None
    }
}

impl Themes {
    pub fn get(&self, name: &str) -> Option<&Theme> {
        self.list.iter().find(|t| t.name.eq_ignore_ascii_case(name))
    }

    fn clean(&self, name: &str, fg: &str, bg: &str) -> Result<Theme, String> {
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if name.is_empty() {
            return Err("Give the theme a name.".into());
        }
        if name.chars().count() > 40 {
            return Err("Use a shorter name (up to 40 characters).".into());
        }
        let norm = |h: &str| parse_hex(h).map(crate::config::to_hex);
        let (Some(fg), Some(bg)) = (norm(fg), norm(bg)) else {
            return Err("Colours are written like #1a1a1a.".into());
        };
        Ok(Theme { name, fg, bg })
    }

    /// Adds a theme, or replaces the one of the same name.
    pub fn save(&mut self, name: &str, fg: &str, bg: &str) -> Result<String, String> {
        let t = self.clean(name, fg, bg)?;
        let name = t.name.clone();
        match self
            .list
            .iter()
            .position(|x| x.name.eq_ignore_ascii_case(&name))
        {
            Some(i) => self.list[i] = t,
            None => self.list.push(t),
        }
        Ok(name)
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let n = self.list.len();
        self.list.retain(|t| !t.name.eq_ignore_ascii_case(name));
        n != self.list.len()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&self.list).unwrap_or_default()
    }

    /// Adds the themes of a file; the number read. Bad entries are skipped.
    pub fn import(&mut self, text: &str) -> Result<usize, String> {
        let list: Vec<Theme> = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let mut n = 0;
        for t in list {
            if self.save(&t.name, &t.fg, &t.bg).is_ok() {
                n += 1;
            }
        }
        Ok(n)
    }
}

impl Theme {
    /// The reader settings that show this theme.
    pub fn apply(&self, prefs: &ReaderPrefs) -> ReaderPrefs {
        ReaderPrefs {
            colors: ColorMode::Custom,
            custom_fg: self.fg.clone(),
            custom_bg: self.bg.clone(),
            ..prefs.clone()
        }
    }

    pub fn is_dark(&self) -> bool {
        luminance(&self.bg).is_some_and(|l| l < 0.2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_of_black_and_white_is_21() {
        assert!((contrast("#000000", "#ffffff").unwrap() - 21.0).abs() < 1e-6);
        assert!((contrast("#ffffff", "#ffffff").unwrap() - 1.0).abs() < 1e-9);
        assert!(contrast("nope", "#ffffff").is_none());
    }

    #[test]
    fn low_contrast_is_flagged() {
        assert!(warning("#777777", "#808080").unwrap().contains("too low"));
        assert!(warning("#767676", "#ffffff").is_none());
        assert!(
            warning("#8a8a8a", "#ffffff")
                .unwrap()
                .contains("a little low")
        );
        assert!(warning("#000000", "#ffffff").is_none());
    }

    #[test]
    fn themes_are_saved_cleaned_and_replaced() {
        let mut t = Themes::default();
        assert_eq!(
            t.save("  Solar   Night ", "#EEE8D5", "#002B36").unwrap(),
            "Solar Night"
        );
        assert_eq!(t.list[0].fg, "#eee8d5");
        t.save("solar night", "#ffffff", "#000000").unwrap();
        assert_eq!(t.list.len(), 1);
        assert_eq!(t.list[0].bg, "#000000");
        assert!(t.save("", "#fff", "#000").is_err());
        assert!(t.save("x", "red", "#000000").is_err());
        assert!(t.save(&"y".repeat(41), "#000000", "#ffffff").is_err());
    }

    #[test]
    fn a_theme_turns_into_custom_colours() {
        let mut t = Themes::default();
        t.save("Paper", "#222222", "#f5f0e6").unwrap();
        let out = t.get("paper").unwrap().apply(&ReaderPrefs::default());
        assert_eq!(out.colors, ColorMode::Custom);
        assert_eq!(out.custom_bg, "#f5f0e6");
        assert_eq!(out.font_size, ReaderPrefs::default().font_size);
        assert!(!t.get("Paper").unwrap().is_dark());
        t.save("Ink", "#dddddd", "#101010").unwrap();
        assert!(t.get("Ink").unwrap().is_dark());
    }

    #[test]
    fn delete_and_import_export() {
        let mut t = Themes::default();
        t.save("A", "#111111", "#eeeeee").unwrap();
        t.save("B", "#eeeeee", "#111111").unwrap();
        let text = t.to_json();
        assert!(t.remove("a"));
        assert!(!t.remove("a"));
        let mut u = Themes::default();
        assert_eq!(u.import(&text).unwrap(), 2);
        assert_eq!(u.list.len(), 2);
        assert!(u.import("[}").is_err());
        // bad entries are skipped
        assert_eq!(u.import("[{\"name\":\"C\",\"fg\":\"zzz\",\"bg\":\"#000000\"},{\"name\":\"D\",\"fg\":\"#000000\",\"bg\":\"#ffffff\"}]").unwrap(), 1);
    }
}
