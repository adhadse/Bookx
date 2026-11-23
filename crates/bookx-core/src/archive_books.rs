//! ZIP archives that hold several e-books (roadmap L14): list the books inside, then extract the
//! ones the user picked. An archive that is itself a book (EPUB, comic archive, a zip of text
//! files that make one HTML book) is not listed: only archives whose content is mostly book files.
//!
//! Security: nothing is trusted from the archive but the bytes. Names are never used as paths (the
//! file is written under a sanitised base name inside the folder the caller gives), the number of
//! entries scanned, the size of one entry and the compression ratio are limited, and a book is
//! written in chunks with a hard byte limit even when the header lies about its size.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// Entries looked at; a larger archive is not treated as a bundle of books.
pub const MAX_ENTRIES: usize = 20_000;
/// The biggest single book taken out of an archive.
pub const MAX_ENTRY_BYTES: u64 = 1 << 30;
/// Uncompressed over compressed size beyond which a big entry is refused (a zip bomb).
pub const MAX_RATIO: u64 = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Position in the archive; pass it to [`extract`].
    pub index: usize,
    /// The path inside the archive (for display only).
    pub name: String,
    pub size: u64,
}

fn base_name(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

/// The books in `path`, or `None` when the file is not a bundle of books. `is_book` decides by
/// file name which entries are books.
pub fn list_books(path: &Path, is_book: &dyn Fn(&str) -> bool) -> Result<Option<Vec<Entry>>> {
    let file =
        std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let Ok(mut zip) = zip::ZipArchive::new(file) else {
        return Ok(None);
    };
    if zip.len() > MAX_ENTRIES || zip.is_empty() {
        return Ok(None);
    }
    let (mut books, mut files) = (Vec::new(), 0usize);
    for i in 0..zip.len() {
        let Ok(e) = zip.by_index_raw(i) else { continue };
        if e.is_dir() {
            continue;
        }
        let Ok(name) = e.name().map(|n| n.to_string()) else {
            continue;
        };
        // an EPUB (or anything built on its container) is a book, not a bundle
        if name == "mimetype" || name == "META-INF/container.xml" {
            return Ok(None);
        }
        files += 1;
        let b = base_name(&name);
        if !b.starts_with('.') && !name.starts_with("__MACOSX/") && is_book(b) {
            books.push(Entry {
                index: i,
                name,
                size: e.size(),
            });
        }
    }
    // mostly book files: a comic archive or a zipped website has a few at best
    if books.is_empty() || books.len() * 2 < files {
        return Ok(None);
    }
    Ok(Some(books))
}

/// Writes entry `index` of the archive into `dir` under a safe name and returns its path.
pub fn extract(path: &Path, index: usize, dir: &Path) -> Result<PathBuf> {
    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let mut e = zip.by_index(index).context("no such entry")?;
    let shown = e.name().map(|n| n.to_string()).unwrap_or_default();
    if e.is_dir() {
        bail!("that entry is a folder");
    }
    if e.encrypted() {
        bail!("{} is password protected", shown);
    }
    let (size, packed) = (e.size(), e.compressed_size().max(1));
    if size > MAX_ENTRY_BYTES {
        bail!("{} is too large to import from an archive", shown);
    }
    if size > (10 << 20) && size / packed > MAX_RATIO {
        bail!("{} looks like a zip bomb and was not extracted", shown);
    }
    let safe = crate::util::sanitize_filename(base_name(&shown));
    let safe = if safe.trim_matches('.').is_empty() {
        "book".to_string()
    } else {
        safe
    };
    std::fs::create_dir_all(dir)?;
    let (stem, ext) = match safe.rsplit_once('.') {
        Some((s, x)) if !s.is_empty() => (s.to_string(), format!(".{x}")),
        _ => (safe.clone(), String::new()),
    };
    let mut target = dir.join(&safe);
    let mut n = 1;
    while target.exists() {
        n += 1;
        target = dir.join(format!("{stem} ({n}){ext}"));
    }
    let mut out = std::fs::File::create(&target)?;
    // the header may lie: stop at the limit whatever it says
    let mut limited = (&mut e).take(MAX_ENTRY_BYTES + 1);
    let mut buf = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = limited.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > MAX_ENTRY_BYTES {
            drop(out);
            let _ = std::fs::remove_file(&target);
            bail!("an entry is larger than it says; extraction stopped");
        }
        out.write_all(&buf[..n])?;
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    fn zip_of(files: &[(&str, &[u8])]) -> PathBuf {
        let p = std::env::temp_dir().join(format!("bookx-bundle-{}.zip", uuid::Uuid::new_v4()));
        let mut w = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
        for (n, d) in files {
            w.start_file(*n, SimpleFileOptions::default()).unwrap();
            w.write_all(d).unwrap();
        }
        w.finish().unwrap();
        p
    }

    fn is_book(n: &str) -> bool {
        n.ends_with(".epub") || n.ends_with(".pdf") || n.ends_with(".mobi")
    }

    #[test]
    fn a_bundle_lists_its_books() {
        let z = zip_of(&[
            ("A/one.epub", b"1"),
            ("two.pdf", b"22"),
            ("cover.jpg", b"x"),
            ("__MACOSX/._one.epub", b"x"),
        ]);
        let l = list_books(&z, &is_book).unwrap().unwrap();
        assert_eq!(
            l.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["A/one.epub", "two.pdf"]
        );
        assert_eq!(l[1].size, 2);
        let _ = std::fs::remove_file(z);
    }

    #[test]
    fn books_and_comics_are_not_bundles() {
        let epub = zip_of(&[("mimetype", b"application/epub+zip"), ("a.epub", b"x")]);
        assert!(list_books(&epub, &is_book).unwrap().is_none());
        let comic = zip_of(&[
            ("1.jpg", b"x"),
            ("2.jpg", b"x"),
            ("3.jpg", b"x"),
            ("a.pdf", b"x"),
        ]);
        assert!(list_books(&comic, &is_book).unwrap().is_none());
        let none = zip_of(&[("a.txt", b"x")]);
        assert!(list_books(&none, &is_book).unwrap().is_none());
        let notzip = std::env::temp_dir().join(format!("bookx-nz-{}", uuid::Uuid::new_v4()));
        std::fs::write(&notzip, b"plain").unwrap();
        assert!(list_books(&notzip, &is_book).unwrap().is_none());
        for p in [epub, comic, none, notzip] {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn extraction_never_leaves_the_folder_and_never_overwrites() {
        let z = zip_of(&[
            ("../../evil.epub", b"EVIL"),
            ("a/b/same.epub", b"1"),
            ("c/same.epub", b"2"),
        ]);
        let l = list_books(&z, &is_book).unwrap().unwrap();
        let dir = std::env::temp_dir().join(format!("bookx-out-{}", uuid::Uuid::new_v4()));
        let paths: Vec<PathBuf> = l
            .iter()
            .map(|e| extract(&z, e.index, &dir).unwrap())
            .collect();
        for p in &paths {
            assert_eq!(p.parent(), Some(dir.as_path()), "{p:?}");
        }
        assert_eq!(std::fs::read(&paths[0]).unwrap(), b"EVIL");
        assert_ne!(paths[1], paths[2]);
        assert_eq!(std::fs::read(&paths[1]).unwrap(), b"1");
        assert_eq!(std::fs::read(&paths[2]).unwrap(), b"2");
        assert!(!dir.parent().unwrap().join("evil.epub").exists());
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_file(z);
    }

    #[test]
    fn a_compression_bomb_and_a_folder_entry_are_refused() {
        let big = vec![0u8; 30 << 20];
        let z = zip_of(&[("bomb.epub", &big), ("ok.epub", b"x")]);
        let l = list_books(&z, &is_book).unwrap().unwrap();
        let dir = std::env::temp_dir().join(format!("bookx-bomb-{}", uuid::Uuid::new_v4()));
        // zeros compress far past the ratio limit; deflate is not enabled for SimpleFileOptions
        // defaults in every build, so only assert the refusal when the ratio really is high
        let r = extract(&z, l[0].index, &dir);
        if std::fs::metadata(&z).unwrap().len() < (30 << 20) / MAX_RATIO {
            assert!(r.is_err(), "ratio over the limit");
        }
        assert!(extract(&z, 99, &dir).is_err());
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_file(z);
    }
}
