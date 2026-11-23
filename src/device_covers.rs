//! Covers of the books on a device, for the Device window.
//!
//! The cover comes out of the book file itself (EPUB and KEPUB; other formats show a placeholder).
//! It is cached in a folder under the user's cache directory, keyed by the book's path, size and
//! modification time, so a book is opened at most once and a changed file gets a new entry.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use bookx_serve::devfs::DeviceBook;

/// The cache entry of a book: changes when the file changes.
pub fn cache_key(book: &DeviceBook) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    book.path.hash(&mut h);
    book.size.hash(&mut h);
    book.modified.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Whether the cover of this kind of file can be read.
pub fn can_read(book: &DeviceBook) -> bool {
    book.format().ends_with("epub")
}

/// The raw cover image of the book file.
pub fn extract(path: &Path) -> Option<Vec<u8>> {
    let book = bookx_core::book::OpenBook::open(path).ok()?;
    book.cover_bytes().map(|b| b.to_vec())
}

/// The cached cover file of `book`, extracting it on the first call. `None` when the book has no
/// cover (also remembered, so the file is not opened again).
pub fn cached(dir: &Path, book: &DeviceBook) -> Option<PathBuf> {
    if !can_read(book) {
        return None;
    }
    let key = cache_key(book);
    let file = dir.join(&key);
    if file.exists() {
        return Some(file);
    }
    let none = dir.join(format!("{key}.none"));
    if none.exists() {
        return None;
    }
    let _ = std::fs::create_dir_all(dir);
    if let Some(bytes) = extract(&book.path) {
        if std::fs::write(&file, bytes).is_ok() {
            return Some(file);
        }
    }
    let _ = std::fs::write(none, b"");
    None
}

/// Where the covers are cached.
pub fn cache_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.cache_dir().join("device-covers"))
        .unwrap_or_else(|| std::env::temp_dir().join("bookx-device-covers"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_core::convert::{Chapter, create_epub};
    use bookx_core::opf::Metadata;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-dc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn device_book(dir: &Path, name: &str, cover: Option<Vec<u8>>) -> DeviceBook {
        let md = Metadata {
            title: "Covered".into(),
            authors: vec!["Ann".into()],
            language: "en".into(),
            ..Default::default()
        };
        let ch = vec![Chapter {
            title: "One".into(),
            body: "<p>Text.</p>".into(),
        }];
        let bytes = create_epub(&md, &ch, cover.map(|c| (c, "png")))
            .to_bytes()
            .unwrap();
        let path = dir.join(name);
        std::fs::write(&path, &bytes).unwrap();
        DeviceBook {
            path,
            rel: name.into(),
            size: bytes.len() as u64,
            modified: Some(1),
        }
    }

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nnot really an image but bytes";

    #[test]
    fn the_cover_is_taken_from_the_book_and_cached() {
        let d = tmp();
        let b = device_book(&d, "a.epub", Some(PNG.to_vec()));
        let cache = d.join("cache");
        let f = cached(&cache, &b).expect("cover");
        assert_eq!(std::fs::read(&f).unwrap(), PNG);
        // the second call does not need the book any more
        std::fs::remove_file(&b.path).unwrap();
        assert_eq!(cached(&cache, &b), Some(f));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_book_without_a_cover_is_remembered() {
        let d = tmp();
        let b = device_book(&d, "b.epub", None);
        let cache = d.join("cache");
        assert_eq!(cached(&cache, &b), None);
        assert!(cache.join(format!("{}.none", cache_key(&b))).exists());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_changed_file_gets_a_new_entry_and_other_formats_are_skipped() {
        let d = tmp();
        let mut b = device_book(&d, "c.epub", None);
        let k = cache_key(&b);
        b.modified = Some(2);
        assert_ne!(k, cache_key(&b));
        let mut mobi = b.clone();
        mobi.path = d.join("c.azw3");
        mobi.rel = "c.azw3".into();
        assert!(!can_read(&mobi));
        assert_eq!(cached(&d.join("cache"), &mobi), None);
        assert!(can_read(&DeviceBook {
            path: d.join("k.kepub.epub"),
            rel: "k.kepub.epub".into(),
            size: 0,
            modified: None
        }));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_file_that_is_not_a_book_has_no_cover() {
        let d = tmp();
        let p = d.join("junk.epub");
        std::fs::write(&p, b"PK-fake").unwrap();
        let b = DeviceBook {
            path: p,
            rel: "junk.epub".into(),
            size: 7,
            modified: None,
        };
        assert_eq!(cached(&d.join("cache"), &b), None);
        let _ = std::fs::remove_dir_all(d);
    }
}
