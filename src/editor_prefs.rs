//! Small view settings of the book editor (minimap, hard-to-see characters), kept in their own
//! file so the main configuration stays untouched. No GTK in here.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorPrefs {
    /// The overview of the whole file at the right edge of the code view.
    pub minimap: bool,
    /// Scrolling the preview also scrolls the code view (the other way, code to preview, is always on). On by default.
    pub scroll_both: bool,
    /// Draw spaces, tabs, line breaks and invisible characters (CE-338).
    pub show_whitespace: bool,
    /// Typing `;` after `&name` turns the entity into its character (CE-302). Off by default.
    pub convert_entities: bool,
    /// Kinds of problems hidden in the Problems list, per book (library id) (CE-162).
    pub ignored_checks: BTreeMap<String, BTreeSet<String>>,
    /// "Also check accessibility" in the Problems panel (CE-164).
    pub check_accessibility: bool,
}

impl Default for EditorPrefs {
    fn default() -> Self {
        EditorPrefs {
            minimap: true,
            scroll_both: true,
            show_whitespace: false,
            convert_entities: false,
            ignored_checks: BTreeMap::new(),
            check_accessibility: false,
        }
    }
}

/// How the code view and the preview look and behave (CE-357, CE-400..402). Kept in its own file
/// (`editor-look.json`) so the other editor settings, saved by their own windows, never overwrite it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorLook {
    /// Code font as a Pango description ("Source Code Pro 11"); empty = the system monospace font (CE-357).
    pub font: String,
    pub tab_width: u32,
    pub insert_spaces: bool,
    pub line_numbers: bool,
    pub highlight_line: bool,
    pub right_margin: bool,
    pub right_margin_column: u32,
    /// Id of a GtkSourceView style scheme; empty = follows light/dark and e-ink.
    pub scheme: String,
    /// Paint colour literals of style sheets in their colour (CE-356).
    pub css_colors: bool,
    /// Typing `>` of an opening tag inserts the closing tag.
    pub auto_close_tags: bool,
    /// The completion popup opens while typing (off: only on Ctrl+Space).
    pub completion_auto: bool,
    /// Underline misspelled words while editing.
    pub spell_underline: bool,
    /// Font of the preview as a Pango description; empty = the reader's font.
    pub preview_font: String,
    /// Smallest text size of the preview in points (0 = no minimum).
    pub preview_min_size: u32,
    /// The preview follows the cursor when the editor starts.
    pub preview_follow: bool,
}

impl Default for EditorLook {
    fn default() -> Self {
        EditorLook {
            font: String::new(),
            tab_width: 2,
            insert_spaces: true,
            line_numbers: true,
            highlight_line: true,
            right_margin: false,
            right_margin_column: 80,
            scheme: String::new(),
            css_colors: true,
            auto_close_tags: true,
            completion_auto: true,
            spell_underline: true,
            preview_font: String::new(),
            preview_min_size: 0,
            preview_follow: true,
        }
    }
}

pub fn look_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.config_dir().join("editor-look.json"))
}

impl EditorLook {
    /// Values brought into a usable range.
    pub fn sanitized(mut self) -> EditorLook {
        self.tab_width = self.tab_width.clamp(1, 16);
        self.right_margin_column = self.right_margin_column.clamp(20, 200);
        self.preview_min_size = self.preview_min_size.min(48);
        self
    }

    pub fn load_from(path: &Path) -> EditorLook {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<EditorLook>(&t).ok())
            .map(EditorLook::sanitized)
            .unwrap_or_default()
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(
            path,
            serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into()),
        )
    }

    /// Loads the file, changes it with `f`, saves it and tells every open editor window.
    pub fn update(f: impl FnOnce(&mut EditorLook)) {
        let mut p = look();
        f(&mut p);
        p = p.sanitized();
        if let Some(path) = look_path() {
            let _ = p.save_to(&path);
        }
        CURRENT.with(|c| *c.borrow_mut() = Some(p));
        crate::settings_bus::editor().notify();
    }
}

thread_local! {
    static CURRENT: std::cell::RefCell<Option<EditorLook>> = const { std::cell::RefCell::new(None) };
}

/// The settings the running app uses now (read from the file on first use, replaced by every change).
pub fn look() -> EditorLook {
    CURRENT.with(|c| {
        c.borrow_mut()
            .get_or_insert_with(|| {
                look_path()
                    .map(|p| EditorLook::load_from(&p))
                    .unwrap_or_default()
            })
            .clone()
    })
}

/// Folder for user style schemes (`*.xml`), next to the other settings.
pub fn schemes_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx").map(|d| d.config_dir().join("styles"))
}

/// An RGB colour literal found in style sheet text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorSpan {
    pub start: usize,
    pub end: usize,
    pub rgb: (u8, u8, u8),
}

/// `#rgb`, `#rrggbb` and `rgb(r, g, b)` literals in `text`, as char offsets (CE-356).
pub fn css_colors(text: &str) -> Vec<ColorSpan> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '#' {
            let n = chars[i + 1..]
                .iter()
                .take_while(|c| c.is_ascii_hexdigit())
                .count();
            let after_ok = chars
                .get(i + 1 + n)
                .is_none_or(|c| !c.is_alphanumeric() && *c != '_' && *c != '-');
            if (n == 3 || n == 6) && after_ok {
                let h: String = chars[i + 1..i + 1 + n].iter().collect();
                let v = |a: usize, w: usize| u8::from_str_radix(&h[a..a + w], 16).unwrap_or(0);
                let rgb = if n == 3 {
                    (v(0, 1) * 17, v(1, 1) * 17, v(2, 1) * 17)
                } else {
                    (v(0, 2), v(2, 2), v(4, 2))
                };
                out.push(ColorSpan {
                    start: i,
                    end: i + 1 + n,
                    rgb,
                });
                i += 1 + n;
                continue;
            }
        } else if chars[i] == 'r'
            && chars[i..].starts_with(&['r', 'g', 'b', '('])
            && (i == 0 || !chars[i - 1].is_alphanumeric())
        {
            if let Some(close) = chars[i..].iter().position(|c| *c == ')') {
                let inner: String = chars[i + 4..i + close].iter().collect();
                let nums: Vec<u32> = inner
                    .split([',', ' ', '/'])
                    .filter(|p| !p.is_empty())
                    .filter_map(|p| p.parse().ok())
                    .collect();
                if nums.len() >= 3 && nums.iter().take(3).all(|n| *n <= 255) {
                    out.push(ColorSpan {
                        start: i,
                        end: i + close + 1,
                        rgb: (nums[0] as u8, nums[1] as u8, nums[2] as u8),
                    });
                    i += close + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

/// Black or white, whichever reads better on `rgb`.
pub fn contrast_text(rgb: (u8, u8, u8)) -> (u8, u8, u8) {
    let l = 0.299 * rgb.0 as f64 + 0.587 * rgb.1 as f64 + 0.114 * rgb.2 as f64;
    if l > 140.0 {
        (0, 0, 0)
    } else {
        (255, 255, 255)
    }
}

/// The closing tag to insert when `>` is typed after `before` (the text before the caret): the
/// name of the open tag, `None` for closing tags, comments, declarations, self-closing and void
/// elements (CE-401).
pub fn closing_tag_for(before: &str) -> Option<String> {
    let lt = before.rfind('<')?;
    let tag = &before[lt + 1..];
    if tag.contains('>') || tag.contains('<') {
        return None;
    }
    let first = tag.chars().next()?;
    if !first.is_ascii_alphabetic() || tag.ends_with('/') {
        return None;
    }
    // inside an attribute value that contains `<` the quote count is odd
    if tag.matches('"').count() % 2 == 1 || tag.matches('\'').count() % 2 == 1 {
        return None;
    }
    let name: String = tag
        .chars()
        .take_while(|c| c.is_alphanumeric() || matches!(c, ':' | '-' | '_' | '.'))
        .collect();
    const VOID: [&str; 14] = [
        "br", "hr", "img", "meta", "link", "input", "area", "base", "col", "embed", "source",
        "track", "wbr", "param",
    ];
    if VOID.contains(&name.to_ascii_lowercase().as_str()) {
        return None;
    }
    Some(format!("</{name}>"))
}

pub fn path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.config_dir().join("editor.json"))
}

impl EditorPrefs {
    pub fn load_from(path: &Path) -> EditorPrefs {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(
            path,
            serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into()),
        )
    }

    pub fn load() -> EditorPrefs {
        path().map(|p| Self::load_from(&p)).unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(p) = path() {
            let _ = self.save_to(&p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_both_is_on_by_default_but_a_saved_choice_wins() {
        assert!(EditorPrefs::default().scroll_both);
        let dir = std::env::temp_dir().join(format!("bookx-eprefs-sb-{}", uuid::Uuid::new_v4()));
        let f = dir.join("editor.json");
        assert!(EditorPrefs::load_from(&f).scroll_both, "no file: on");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&f, "{\"minimap\": false}").unwrap();
        assert!(EditorPrefs::load_from(&f).scroll_both, "key missing: on");
        std::fs::write(&f, "{\"scroll_both\": false}").unwrap();
        assert!(
            !EditorPrefs::load_from(&f).scroll_both,
            "explicit false is kept"
        );
        let off = EditorPrefs {
            scroll_both: false,
            ..EditorPrefs::default()
        };
        off.save_to(&f).unwrap();
        assert!(
            !EditorPrefs::load_from(&f).scroll_both,
            "saved false round trips"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn defaults_round_trip_and_tolerate_damage() {
        let dir = std::env::temp_dir().join(format!("bookx-eprefs-{}", uuid::Uuid::new_v4()));
        let f = dir.join("editor.json");
        assert_eq!(EditorPrefs::load_from(&f), EditorPrefs::default());
        let p = EditorPrefs {
            minimap: false,
            scroll_both: true,
            show_whitespace: true,
            convert_entities: true,
            ignored_checks: BTreeMap::from([(
                "7".to_string(),
                BTreeSet::from(["OPF-003".to_string()]),
            )]),
            check_accessibility: true,
        };
        p.save_to(&f).unwrap();
        assert_eq!(EditorPrefs::load_from(&f), p);
        std::fs::write(&f, "{\"minimap\": false}").unwrap();
        let q = EditorPrefs::load_from(&f);
        assert!(
            !q.minimap && !q.show_whitespace && !q.convert_entities,
            "missing keys use defaults (entity conversion is off unless asked for)"
        );
        std::fs::write(&f, "garbage").unwrap();
        assert_eq!(EditorPrefs::load_from(&f), EditorPrefs::default());
        assert!(q.ignored_checks.is_empty() && !q.check_accessibility);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn new_settings_round_trip_and_are_clamped() {
        let dir = std::env::temp_dir().join(format!("bookx-eprefs-{}", uuid::Uuid::new_v4()));
        let f = dir.join("editor.json");
        let p = EditorLook {
            font: "Fira Code 12".into(),
            tab_width: 4,
            insert_spaces: false,
            scheme: "kate".into(),
            preview_font: "Serif 15".into(),
            preview_min_size: 12,
            ..Default::default()
        };
        p.save_to(&f).unwrap();
        assert_eq!(EditorLook::load_from(&f), p);
        std::fs::write(&f, "{\"tab_width\": 99, \"right_margin_column\": 1}").unwrap();
        let q = EditorLook::load_from(&f);
        assert_eq!((q.tab_width, q.right_margin_column), (16, 20));
        assert!(q.line_numbers && q.auto_close_tags && q.completion_auto);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn colour_literals_are_found_with_their_offsets() {
        let t = "a { color: #fff; background: #1a2B3c; border: 1px solid rgb(10, 20, 30); }";
        let c = css_colors(t);
        assert_eq!(c.len(), 3);
        assert_eq!(c[0].rgb, (255, 255, 255));
        assert_eq!(c[1].rgb, (0x1a, 0x2b, 0x3c));
        assert_eq!(c[2].rgb, (10, 20, 30));
        let s: String = t
            .chars()
            .skip(c[0].start)
            .take(c[0].end - c[0].start)
            .collect();
        assert_eq!(s, "#fff");
        assert!(css_colors("a#header { } #abcd { } #12345g").is_empty());
        assert!(css_colors("rgb(300, 0, 0)").is_empty());
        assert_eq!(
            css_colors("é #000;")[0].start,
            2,
            "offsets count characters"
        );
        assert_eq!(contrast_text((255, 255, 0)), (0, 0, 0));
        assert_eq!(contrast_text((10, 10, 80)), (255, 255, 255));
    }

    #[test]
    fn closing_tags_are_proposed_for_open_tags_only() {
        assert_eq!(closing_tag_for("<p"), Some("</p>".into()));
        assert_eq!(
            closing_tag_for("text <div class=\"a\""),
            Some("</div>".into())
        );
        assert_eq!(closing_tag_for("<svg:g id=\"x\""), Some("</svg:g>".into()));
        assert_eq!(closing_tag_for("<br"), None, "void element");
        assert_eq!(closing_tag_for("<br /"), None, "self-closing");
        assert_eq!(closing_tag_for("</p"), None);
        assert_eq!(closing_tag_for("<!-- a"), None);
        assert_eq!(closing_tag_for("<?xml version=\"1.0\"?"), None);
        assert_eq!(closing_tag_for("<p>text"), None, "tag already closed");
        assert_eq!(closing_tag_for("a < b"), None, "a comparison is not a tag");
        assert_eq!(
            closing_tag_for("<a title=\"x"),
            None,
            "inside an attribute value"
        );
        assert_eq!(closing_tag_for("<IMG src=\"a\""), None);
    }
}
