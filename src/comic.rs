//! Comics in the app: recognising a comic book, serving its pages to the decoder threads, and the
//! settings file. The reading logic is in the `bookx-comic` crate; the window is `ui/reader_comic.rs`.

use std::path::PathBuf;
use std::sync::Arc;

use bookx_comic::meta::{self, PageInfo};
use bookx_comic::settings::Store;
use bookx_comic::worker::Source;

use crate::core::archive::Archive;
use crate::core::book::OpenBook;

/// The pages of a comic book, shared with the decoder threads.
pub struct ArchiveSource {
    arc: Archive,
    paths: Vec<String>,
}

impl Source for ArchiveSource {
    fn count(&self) -> usize {
        self.paths.len()
    }
    fn bytes(&self, page: usize) -> Option<&[u8]> {
        self.paths.get(page).and_then(|p| self.arc.get(p))
    }
}

/// A comic ready to be read.
pub struct ComicBook {
    pub source: Arc<ArchiveSource>,
    pub info: PageInfo,
    /// The book reads right to left (ComicInfo `Manga`).
    pub rtl: bool,
}

/// Takes the pages out of an opened book when it is a comic.
pub fn open_comic(book: OpenBook) -> Result<ComicBook, Box<OpenBook>> {
    let opf = book.archive.get_string(&book.package.opf_path);
    let first = book
        .spine_paths()
        .first()
        .and_then(|p| book.archive.get_string(p));
    let is_comic = opf
        .as_deref()
        .is_some_and(|o| meta::is_comic(o, first.as_deref()));
    let paths = meta::page_paths(book.archive.names());
    if !is_comic || paths.is_empty() {
        return Err(Box::new(book));
    }
    let info = opf.as_deref().and_then(meta::read_opf).unwrap_or_default();
    let rtl = book.is_rtl();
    Ok(ComicBook {
        source: Arc::new(ArchiveSource {
            arc: book.archive,
            paths,
        }),
        info,
        rtl,
    })
}

fn path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.config_dir().join("comics.json"))
}

pub fn load() -> Store {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| Store::from_json(&t))
        .unwrap_or_default()
}

pub fn save(store: &Store) {
    if let Some(p) = path() {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(p, store.to_json());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::contracts::FormatReader;
    use bookx_comic::testgen::small_comic;
    use bookx_formats::comic::CbzReader;

    fn comic_archive(info: Option<&str>) -> OpenBook {
        let arc = CbzReader.read(&small_comic(info)).expect("convert");
        OpenBook::from_archive(arc).expect("open")
    }

    #[test]
    fn a_converted_cbz_is_a_comic_with_its_pages_in_order() {
        let c = open_comic(comic_archive(None)).ok().expect("a comic");
        assert_eq!(c.source.count(), 6);
        assert!(!c.rtl);
        for i in 0..6 {
            let b = c.source.bytes(i).expect("page bytes");
            assert!(bookx_comic::decode::dimensions(b).is_ok());
        }
        assert!(c.source.bytes(6).is_none());
        assert_eq!(
            bookx_comic::decode::dimensions(c.source.bytes(3).unwrap()).unwrap(),
            (800, 600)
        );
    }

    #[test]
    fn comic_info_reaches_the_reader() {
        let ci = r#"<ComicInfo><Series>S</Series><Manga>YesAndRightToLeft</Manga><Pages><Page Image="3" DoublePage="true"/></Pages></ComicInfo>"#;
        let c = open_comic(comic_archive(Some(ci))).ok().expect("a comic");
        assert!(c.rtl, "Manga=YesAndRightToLeft reads right to left");
        assert_eq!(c.info.double, vec![3]);
    }

    #[test]
    fn a_novel_is_not_a_comic() {
        let md = crate::core::opf::Metadata {
            title: "Novel".into(),
            ..Default::default()
        };
        let chapters = vec![crate::core::convert::Chapter {
            title: "One".into(),
            body: "<p>Once upon a time.</p>".into(),
        }];
        let arc = crate::core::convert::create_epub(&md, &chapters, None);
        let book = OpenBook::from_archive(arc).expect("open");
        assert!(open_comic(book).is_err());
    }
}
