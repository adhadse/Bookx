//! The book map's thumbnails kept on disk (PNG files in the cache folder), so that the second time a
//! map is opened its pages are there at once. A file is named by the book (its file and date), the
//! layout (size, style, language, highlights, device scale) and the page; a different layout is a
//! different folder. The cache has a size cap: the oldest files go first. Without GTK.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// How much the thumbnails may take on disk.
pub const CAP_BYTES: u64 = 64 * 1024 * 1024;

/// The folder of one book in one layout: `<root>/<book hash>/<layout hash>`. `book` identifies the
/// book file (path and modification time), `layout` is a text with everything that changes how a
/// page looks.
pub fn folder(root: &Path, book: &str, layout: &str) -> PathBuf {
    let h = |s: &str| {
        let mut d = DefaultHasher::new();
        s.hash(&mut d);
        format!("{:016x}", d.finish())
    };
    root.join(h(book)).join(h(layout))
}

pub fn file(folder: &Path, page: usize) -> PathBuf {
    folder.join(format!("{page}.png"))
}

pub fn load(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path)
        .ok()
        .filter(|b| b.starts_with(b"\x89PNG"))
}

/// Writes a thumbnail (atomically) and, when the cache is over `cap`, removes the oldest files.
pub fn store(root: &Path, path: &Path, bytes: &[u8], cap: u64) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("png.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    trim(root, cap);
    Ok(())
}

fn files(root: &Path, out: &mut Vec<(SystemTime, u64, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        match e.metadata() {
            Ok(m) if m.is_dir() => files(&p, out),
            Ok(m) if p.extension().is_some_and(|x| x == "png") => {
                out.push((m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len(), p))
            }
            _ => {}
        }
    }
}

/// Removes the oldest thumbnails until the cache fits in `cap` bytes. Returns the bytes left.
pub fn trim(root: &Path, cap: u64) -> u64 {
    let mut all = Vec::new();
    files(root, &mut all);
    let mut total: u64 = all.iter().map(|f| f.1).sum();
    if total <= cap {
        return total;
    }
    all.sort_by_key(|f| f.0);
    for (_, len, p) in all {
        if total <= cap {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total -= len;
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-thumbs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn png(n: usize) -> Vec<u8> {
        let mut v = b"\x89PNG".to_vec();
        v.extend(std::iter::repeat_n(7u8, n));
        v
    }

    #[test]
    fn the_folder_depends_on_the_book_and_on_the_layout() {
        let r = Path::new("/c");
        let a = folder(r, "book.epub 100 5", "800x600 serif 14");
        assert_eq!(a, folder(r, "book.epub 100 5", "800x600 serif 14"));
        assert_ne!(a, folder(r, "book.epub 100 6", "800x600 serif 14")); // the book was changed
        assert_ne!(a, folder(r, "book.epub 100 5", "800x600 serif 15")); // another layout
        assert_ne!(
            a.parent(),
            folder(r, "other.epub 100 5", "800x600 serif 14").parent()
        );
        assert!(file(&a, 12).ends_with("12.png"));
    }

    #[test]
    fn a_stored_page_is_read_back_and_a_damaged_file_is_ignored() {
        let root = tmp("rw");
        let f = file(&folder(&root, "b", "l"), 3);
        assert!(load(&f).is_none());
        store(&root, &f, &png(100), CAP_BYTES).unwrap();
        assert_eq!(load(&f).unwrap().len(), 104);
        std::fs::write(&f, b"not a png").unwrap();
        assert!(load(&f).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_cache_stays_under_its_cap_and_drops_the_oldest() {
        let root = tmp("cap");
        let dir = folder(&root, "b", "l");
        for p in 0..10 {
            store(&root, &file(&dir, p), &png(1000), 4 * 1004).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        let mut left: Vec<usize> = (0..10).filter(|p| file(&dir, *p).exists()).collect();
        left.sort();
        assert_eq!(left, vec![6, 7, 8, 9]);
        assert!(trim(&root, 1) <= 1004 * 4);
        let _ = std::fs::remove_dir_all(&root);
    }
}
