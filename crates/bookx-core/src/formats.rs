//! Several format files for one book, and choosing the best one for a purpose.
//!
//! A [`Book`](crate::library::Book) has a primary file and, after *Merge as formats of one book* or
//! *Add as another format*, further [`FormatFile`]s. Pure logic only; the file handling is in
//! `files.rs`.

use serde::{Deserialize, Serialize};

/// One format file of a book. `file` is relative to the library folder, or absolute when linked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatFile {
    pub file: String,
    pub format: String,
    #[serde(default)]
    pub size: u64,
}

/// A linked item that is not a book file (an audiobook folder, for instance).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resource {
    /// `audiobook`.
    pub kind: String,
    /// Absolute path of a file or folder.
    pub path: String,
    #[serde(default)]
    pub label: String,
}

pub const AUDIOBOOK: &str = "audiobook";

/// What a format is wanted for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Bookx's own reader (reads EPUB).
    Reader,
    /// A Kobo (KEPUB first).
    Kobo,
    /// A Kindle (AZW3, then MOBI).
    Kindle,
    /// Any other device or sharing: EPUB first.
    Generic,
    /// The source of a conversion: the format that keeps the most structure.
    Convert,
}

impl Target {
    fn preference(self) -> &'static [&'static str] {
        match self {
            Target::Reader => &["epub", "kepub"],
            Target::Kobo => &["kepub", "epub"],
            Target::Kindle => &["azw3", "kfx", "mobi", "epub"],
            Target::Generic => &["epub", "kepub", "azw3", "mobi", "fb2", "pdf"],
            Target::Convert => &[
                "epub", "kepub", "azw3", "mobi", "fb2", "docx", "odt", "rtf", "htmlz", "txtz",
                "cbz", "pdf",
            ],
        }
    }
}

/// The file format of a library path: `Dune.kepub.epub` is `kepub`, `a.AZW3` is `azw3`.
pub fn format_of(file: &str) -> String {
    let name = file.rsplit('/').next().unwrap_or(file).to_lowercase();
    if name.ends_with(".kepub.epub") {
        return "kepub".into();
    }
    name.rsplit_once('.')
        .map(|(_, e)| e.to_string())
        .unwrap_or_default()
}

/// The best of `formats` for `target`: the first in the target's preference order, else (for
/// anything but the reader) the first one. The reader never gets a format it cannot open.
pub fn choose(formats: &[FormatFile], target: Target) -> Option<&FormatFile> {
    for want in target.preference() {
        if let Some(f) = formats.iter().find(|f| f.format.eq_ignore_ascii_case(want)) {
            return Some(f);
        }
    }
    match target {
        Target::Reader => None,
        _ => formats.first(),
    }
}

/// The index that [`choose`] picks for the reader, or the first one.
pub fn reader_index(formats: &[FormatFile]) -> usize {
    choose(formats, Target::Reader)
        .and_then(|c| formats.iter().position(|f| f.file == c.file))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(file: &str) -> FormatFile {
        FormatFile {
            file: file.into(),
            format: format_of(file),
            size: 1,
        }
    }

    #[test]
    fn formats_are_read_from_names() {
        assert_eq!(format_of("a/Dune.kepub.epub"), "kepub");
        assert_eq!(format_of("a/Dune.AZW3"), "azw3");
        assert_eq!(format_of("noext"), "");
    }

    #[test]
    fn each_target_gets_its_own_best_format() {
        let all = [f("a.mobi"), f("a.epub"), f("a.kepub.epub"), f("a.azw3")];
        assert_eq!(choose(&all, Target::Reader).unwrap().file, "a.epub");
        assert_eq!(choose(&all, Target::Kobo).unwrap().file, "a.kepub.epub");
        assert_eq!(choose(&all, Target::Kindle).unwrap().file, "a.azw3");
        assert_eq!(choose(&all, Target::Convert).unwrap().file, "a.epub");
    }

    #[test]
    fn missing_preferred_formats_fall_back() {
        let kindle_only = [f("a.mobi")];
        assert_eq!(choose(&kindle_only, Target::Kindle).unwrap().file, "a.mobi");
        assert_eq!(choose(&kindle_only, Target::Kobo).unwrap().file, "a.mobi");
        assert!(choose(&kindle_only, Target::Reader).is_none());
        assert_eq!(reader_index(&[f("a.pdf"), f("a.epub")]), 1);
        assert!(choose(&[], Target::Generic).is_none());
    }
}
