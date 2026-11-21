//! The reading fonts the Text appearance popover offers: only the families this system has.

/// The generic families, always offered (Pango maps them to something installed).
const GENERIC: [&str; 3] = ["Serif", "Sans", "Monospace"];

/// `list` without the families that are not in `available` (case-insensitive). The generic
/// families and `keep` (the family in use, so the dropdown can still show it) stay.
pub fn installed_fonts<'a>(list: &[&'a str], available: &[String], keep: &str) -> Vec<&'a str> {
    list.iter()
        .copied()
        .filter(|f| {
            GENERIC.contains(f)
                || f.eq_ignore_ascii_case(keep)
                || available.iter().any(|a| a.eq_ignore_ascii_case(f))
        })
        .collect()
}

/// The family names of the system font map.
pub fn system_families() -> Vec<String> {
    use relm4::gtk::pango::prelude::{FontFamilyExt, FontMapExt};
    pangocairo::FontMap::default()
        .list_families()
        .iter()
        .map(|f| f.name().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_families_are_not_offered() {
        let all = ["Serif", "Cantarell", "OpenDyslexic", "Monospace"];
        let have = vec!["cantarell".to_string()];
        assert_eq!(
            installed_fonts(&all, &have, "Serif"),
            ["Serif", "Cantarell", "Monospace"]
        );
        // the one in use stays even when it is gone
        assert_eq!(
            installed_fonts(&all, &have, "OpenDyslexic"),
            ["Serif", "Cantarell", "OpenDyslexic", "Monospace"]
        );
    }
}
