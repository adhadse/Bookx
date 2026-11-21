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

/// The previous or next volume of the book `id` in the library: (id, title). The series decides,
/// else the files that share a folder (books kept where they were, the Linked layout).
pub fn neighbour_of(
    lib: &crate::core::library::Library,
    id: u64,
    forward: bool,
) -> Option<(u64, String)> {
    let me = lib.get(id)?;
    let folder_of = |b: &crate::core::library::Book| -> String {
        let p = std::path::Path::new(&b.file);
        if p.is_absolute() {
            p.parent().map(|d| d.to_string_lossy().into_owned())
        } else {
            None
        }
        .unwrap_or_default()
    };
    let mine = folder_of(me);
    let vols: Vec<bookx_comic::volumes::Volume> = lib
        .books
        .iter()
        .filter(|b| {
            b.id == id
                || (!me.meta.series.is_empty()
                    && b.meta.series.eq_ignore_ascii_case(&me.meta.series))
                || (!mine.is_empty() && folder_of(b) == mine)
        })
        .map(|b| bookx_comic::volumes::Volume {
            id: b.id,
            series: b.meta.series.clone(),
            index: b.meta.series_index,
            title: b.meta.title.clone(),
            folder: folder_of(b),
            file: std::path::Path::new(&b.file)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| b.meta.title.clone()),
        })
        .collect();
    let n = bookx_comic::volumes::neighbour(&vols, id, forward)?;
    lib.get(n).map(|b| (n, b.meta.title.clone()))
}

static LAST_PAGE: std::sync::Mutex<Vec<u64>> = std::sync::Mutex::new(Vec::new());

/// The next time book `id` opens as a comic it starts at its last page (coming back from the
/// next volume).
pub fn start_at_last_page(id: u64) {
    LAST_PAGE.lock().unwrap_or_else(|e| e.into_inner()).push(id);
}

/// Whether the book was asked to open at its last page; asking clears the request.
pub fn take_last_page(id: u64) -> bool {
    let mut v = LAST_PAGE.lock().unwrap_or_else(|e| e.into_inner());
    let had = v.contains(&id);
    v.retain(|i| *i != id);
    had
}

/// Fills in the series from the folder (`Berserk/Berserk v03.cbz`) when the book names none.
/// Returns whether anything changed.
pub fn apply_folder_series(md: &mut crate::core::opf::Metadata, path: &str) -> bool {
    if !md.series.is_empty() {
        return false;
    }
    let Some((series, number)) = bookx_comic::volumes::series_from_path(path) else {
        return false;
    };
    md.series = series;
    if let Some(n) = number {
        md.series_index = n;
    }
    true
}

/// Reads a comic archive for import: honours the "ignore ComicInfo" setting and takes the series
/// from the folder when the file does not name one.
pub fn read_for_import(
    path: &std::path::Path,
    ext: &str,
    bytes: &[u8],
    ignore_info: bool,
) -> anyhow::Result<Archive> {
    let archive = bookx_formats::comic::read_with(
        ext,
        bytes,
        bookx_formats::comic::ComicOptions { ignore_info },
    )?;
    let mut book = OpenBook::from_archive(archive)?;
    let mut md = book.metadata().clone();
    let mut changed = apply_folder_series(&mut md, &path.to_string_lossy());
    // a comic with no title of its own is called after its file
    if md.title == "Untitled" || md.title.trim().is_empty() {
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            md.title = stem.to_string();
            changed = true;
        }
    }
    if changed {
        book.set_metadata(md)?;
    }
    Ok(book.archive)
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
    fn a_request_for_the_last_page_is_handed_out_once() {
        start_at_last_page(987_654_321);
        assert!(take_last_page(987_654_321));
        assert!(!take_last_page(987_654_321));
        assert!(!take_last_page(5));
    }

    #[test]
    fn the_folder_names_the_series_when_the_file_does_not() {
        let dir = std::path::Path::new("/c/Berserk/Berserk v03.cbz");
        let bytes = small_comic(None);
        let a = read_for_import(dir, "cbz", &bytes, false).expect("read");
        let book = OpenBook::from_archive(a).expect("open");
        assert_eq!(book.metadata().series, "Berserk");
        assert_eq!(book.metadata().series_index, 3.0);
        assert_eq!(book.metadata().title, "Berserk v03", "named after the file");
        // the file's own series wins
        let ci = "<ComicInfo><Series>Own</Series></ComicInfo>";
        let a = read_for_import(dir, "cbz", &small_comic(Some(ci)), false).expect("read");
        assert_eq!(
            OpenBook::from_archive(a).expect("open").metadata().series,
            "Own"
        );
        // ... unless ComicInfo is ignored
        let a = read_for_import(dir, "cbz", &small_comic(Some(ci)), true).expect("read");
        assert_eq!(
            OpenBook::from_archive(a).expect("open").metadata().series,
            "Berserk"
        );
        // a generic folder gives nothing
        let a = read_for_import(
            std::path::Path::new("/home/me/Downloads/x.cbz"),
            "cbz",
            &bytes,
            false,
        )
        .expect("read");
        assert_eq!(
            OpenBook::from_archive(a).expect("open").metadata().series,
            ""
        );
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
