//! Which audio format a file is: the first bytes decide, the extension only tells `.m4a` from `.m4b`.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Mp3,
    M4a,
    M4b,
    Ogg,
    Opus,
    Flac,
    Wav,
}

/// The extensions an audiobook part may have.
pub const EXTENSIONS: &[&str] = &[
    "mp3", "m4a", "m4b", "aac", "ogg", "oga", "opus", "flac", "wav", "mp4",
];

impl Format {
    /// Lower-case name used in the catalogue and in `format:` searches.
    pub fn name(self) -> &'static str {
        match self {
            Format::Mp3 => "mp3",
            Format::M4a => "m4a",
            Format::M4b => "m4b",
            Format::Ogg => "ogg",
            Format::Opus => "opus",
            Format::Flac => "flac",
            Format::Wav => "wav",
        }
    }

    /// Can Bookx decode it? (Symphonia has no Opus decoder.)
    pub fn playable(self) -> bool {
        self != Format::Opus
    }
}

/// Is `path` named like an audio file?
pub fn has_audio_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// The format of `head` (the first 4 KiB of a file); `ext` is the lower-case extension, if any.
pub fn detect(head: &[u8], ext: &str) -> Option<Format> {
    if head.starts_with(b"fLaC") {
        return Some(Format::Flac);
    }
    if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WAVE" {
        return Some(Format::Wav);
    }
    if head.starts_with(b"OggS") {
        let n = head.len().min(512);
        if head[..n].windows(8).any(|w| w == b"OpusHead") {
            return Some(Format::Opus);
        }
        return Some(Format::Ogg);
    }
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        return Some(match (&head[8..12], ext) {
            (b"M4B ", _) | (_, "m4b") => Format::M4b,
            _ => Format::M4a,
        });
    }
    if head.starts_with(b"ID3") {
        return Some(Format::Mp3);
    }
    // a bare MPEG audio frame: 11 sync bits, layer III or II or I, not "reserved"
    if head.len() >= 4 && head[0] == 0xFF && head[1] & 0xE0 == 0xE0 && head[1] & 0x18 != 0x08 {
        return Some(Format::Mp3);
    }
    // ADTS AAC is an MPEG-like sync with layer 0; leave it to the extension
    if head.len() >= 2 && head[0] == 0xFF && head[1] & 0xF6 == 0xF0 && ext == "aac" {
        return Some(Format::M4a);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_are_found_by_their_magic() {
        assert_eq!(detect(b"fLaC\0\0\0\"", "x"), Some(Format::Flac));
        assert_eq!(detect(b"RIFF\0\0\0\0WAVEfmt ", "x"), Some(Format::Wav));
        assert_eq!(detect(b"RIFF\0\0\0\0AVI LIST", "wav"), None);
        assert_eq!(detect(b"OggS\0\x02rest-of-page", ""), Some(Format::Ogg));
        let mut opus = b"OggS\0\x02".to_vec();
        opus.extend_from_slice(&[0; 20]);
        opus.extend_from_slice(b"OpusHead");
        assert_eq!(detect(&opus, "ogg"), Some(Format::Opus));
        assert_eq!(detect(b"\0\0\0\x20ftypM4B \0\0", "m4a"), Some(Format::M4b));
        assert_eq!(detect(b"\0\0\0\x20ftypM4A \0\0", "m4a"), Some(Format::M4a));
        assert_eq!(detect(b"\0\0\0\x20ftypisom\0\0", "m4b"), Some(Format::M4b));
        assert_eq!(detect(b"ID3\x03\0\0\0\0\0\0", "bin"), Some(Format::Mp3));
        assert_eq!(detect(&[0xFF, 0xFB, 0x90, 0x00], ""), Some(Format::Mp3));
        assert_eq!(
            detect(b"plain text file", "mp3"),
            None,
            "the name is not enough"
        );
        assert_eq!(detect(b"", "mp3"), None);
    }

    #[test]
    fn extensions_are_recognised_in_any_case() {
        assert!(has_audio_extension(Path::new("a/B.M4B")));
        assert!(!has_audio_extension(Path::new("a/B.epub")));
        assert!(!has_audio_extension(Path::new("noext")));
        assert!(Format::Mp3.playable() && !Format::Opus.playable());
        assert_eq!(Format::M4b.name(), "m4b");
    }
}
