//! Limits for everything that is read from an audio file or a zip. A file may lie about any size
//! or count; these caps keep one hostile file from using memory or time.

/// An ID3v2 tag larger than this is read up to this size only.
pub const MAX_ID3_BYTES: usize = 24 * 1024 * 1024;
/// The `moov` box (and with it the chapter atoms) of an MP4 file.
pub const MAX_MOOV_BYTES: u64 = 48 * 1024 * 1024;
/// A cover picture.
pub const MAX_COVER_BYTES: usize = 16 * 1024 * 1024;
/// Chapters of one file.
pub const MAX_CHAPTERS: usize = 5_000;
/// A text field (title, comment).
pub const MAX_TEXT_BYTES: usize = 64 * 1024;
/// Frames or atoms looked at in one tag.
pub const MAX_ITEMS: usize = 20_000;
/// Nesting of MP4 boxes.
pub const MAX_BOX_DEPTH: usize = 12;
/// Pages read when looking for the comment header of an Ogg file.
pub const MAX_OGG_PAGES: usize = 512;
/// Parts of one audiobook.
pub const MAX_PARTS: usize = 20_000;
/// Entries a zip may have before it is refused, and the total of what is unpacked.
pub const MAX_ZIP_ENTRIES: usize = 50_000;
pub const MAX_ZIP_TOTAL: u64 = 16 * 1024 * 1024 * 1024;
/// Folders are searched this deep.
pub const MAX_DEPTH: usize = 6;
