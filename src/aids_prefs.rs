//! Settings of the small reading aids: the page-turn animation, the auto-turn timer, the night
//! schedule, copy with a citation, the keys of a page turner and the shelf for finished books.
//! The decisions themselves are in `bookx_text::aids`; this file holds what is stored.

use bookx_text::aids::{AUTO_TURN_RANGE, NightSchedule, PageAnimation};
use serde::{Deserialize, Serialize};

/// Name of the shelf that finished books go to when the setting is on.
pub const ARCHIVE_SHELF: &str = "Archive";

/// Finished means this much of the book was read (the statistics use the same value).
pub const FINISHED_FRACTION: f64 = 0.98;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReaderAids {
    pub night: NightSchedule,
    pub page_animation: PageAnimation,
    /// Seconds between two pages while auto-turn runs.
    pub auto_turn_secs: u32,
    /// Template of Copy with Citation (`{text}`, `{title}`, `{author}`, `{chapter}`, `{page}`,
    /// `{date}`, `{isbn}`); empty = the default.
    pub citation_template: String,
    /// Plain Copy adds the citation too.
    pub copy_with_citation: bool,
    /// `KeyName=action` lines laid over the default keys of page turners.
    pub remote_keys: Vec<String>,
    /// A finished book is added to the Archive shelf.
    pub archive_finished: bool,
}

impl Default for ReaderAids {
    fn default() -> Self {
        ReaderAids {
            night: NightSchedule::default(),
            page_animation: PageAnimation::None,
            auto_turn_secs: 30,
            citation_template: String::new(),
            copy_with_citation: false,
            remote_keys: Vec::new(),
            archive_finished: false,
        }
    }
}

impl ReaderAids {
    pub fn clamped(mut self) -> Self {
        self.night = self.night.clamped();
        self.auto_turn_secs = self
            .auto_turn_secs
            .clamp(AUTO_TURN_RANGE.0, AUTO_TURN_RANGE.1);
        self.citation_template = self.citation_template.chars().take(2000).collect();
        self.remote_keys.truncate(64);
        self
    }
}

/// Whether a book that was read to `frac` goes to the Archive shelf now.
pub fn should_archive(aids: &ReaderAids, frac: f64, already: bool) -> bool {
    aids.archive_finished && !already && frac >= FINISHED_FRACTION
}

/// `a=b,c=d` (the Preferences entry) as lines, and back.
pub fn remote_lines(text: &str) -> Vec<String> {
    text.split([',', '\n', ';'])
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .take(64)
        .collect()
}

pub fn remote_text(lines: &[String]) -> String {
    lines.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_do_nothing() {
        let a = ReaderAids::default();
        assert!(!a.night.enabled);
        assert_eq!(a.page_animation, PageAnimation::None);
        assert!(!a.archive_finished && !a.copy_with_citation);
        assert!(!should_archive(&a, 1.0, false), "off by default");
    }

    #[test]
    fn a_finished_book_is_archived_once_when_asked() {
        let a = ReaderAids {
            archive_finished: true,
            ..Default::default()
        };
        assert!(!should_archive(&a, 0.5, false));
        assert!(should_archive(&a, 0.98, false));
        assert!(!should_archive(&a, 1.0, true));
    }

    #[test]
    fn old_config_files_and_partial_ones_load() {
        let a: ReaderAids = serde_json::from_str("{}").unwrap();
        assert_eq!(a, ReaderAids::default());
        let a: ReaderAids =
            serde_json::from_str(r#"{"auto_turn_secs": 12, "page_animation": "Slide"}"#).unwrap();
        assert_eq!(a.auto_turn_secs, 12);
        assert_eq!(a.page_animation, PageAnimation::Slide);
        let s = serde_json::to_string(&a).unwrap();
        assert_eq!(serde_json::from_str::<ReaderAids>(&s).unwrap(), a);
    }

    #[test]
    fn values_are_clamped() {
        let a = ReaderAids {
            auto_turn_secs: 0,
            citation_template: "x".repeat(9000),
            remote_keys: vec!["a=next-page".into(); 500],
            ..Default::default()
        }
        .clamped();
        assert_eq!(a.auto_turn_secs, 2);
        assert_eq!(a.citation_template.len(), 2000);
        assert_eq!(a.remote_keys.len(), 64);
    }

    #[test]
    fn remote_keys_are_typed_as_one_line() {
        let l = remote_lines(" F7=prev-chapter, F8=next-page ;; ,Menu=toggle-chrome\nX= ");
        assert_eq!(
            l,
            vec![
                "F7=prev-chapter",
                "F8=next-page",
                "Menu=toggle-chrome",
                "X="
            ]
        );
        assert_eq!(remote_lines(&remote_text(&l)), l);
        assert!(remote_lines("  ").is_empty());
    }
}
