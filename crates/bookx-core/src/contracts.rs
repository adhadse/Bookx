//! Trait contracts between Bookx crates.
//!
//! Every track of the roadmap (`docs/bookx-feature-roadmap.md`) plugs into one of these traits, so
//! parallel agents only meet at this file. Changing a signature here is a cross-track change.
//!
//! * [`FormatReader`] turns a foreign file into the internal EPUB archive (input formats).
//! * [`FormatWriter`] turns an [`OpenBook`] into bytes of a target format (output formats).
//! * [`Transform`] is one stage between them (smarten punctuation, unwrap lines, …).
//! * [`MetadataSource`] looks metadata up (Open Library, Google Books, …).
//! * [`DeviceDriver`] talks to an e-reader (USB mass storage, Kobo, MTP, …).
//! * [`Store`] persists the catalogue and reading state (JSON today, SQLite next).

use std::path::PathBuf;

use anyhow::{Result, bail};

use super::archive::Archive;
use super::book::OpenBook;
use super::library::Book;
use super::opf::Metadata;

/// Collection of implementations of one contract, looked up by id or file extension.
pub struct Registry<T: ?Sized> {
    items: Vec<Box<T>>,
}

impl<T: ?Sized> Default for Registry<T> {
    fn default() -> Self {
        Registry { items: Vec::new() }
    }
}

impl<T: ?Sized> Registry<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, item: Box<T>) {
        self.items.push(item);
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.items.iter().map(|b| &**b)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Reads one input format.
pub trait FormatReader: Send + Sync {
    /// Stable id, lower case (`"fb2"`).
    fn id(&self) -> &'static str;
    /// File extensions without the dot, lower case.
    fn extensions(&self) -> &'static [&'static str];
    /// Cheap content check on the first bytes, used when the extension is missing or wrong.
    fn sniff(&self, head: &[u8]) -> bool;
    /// Converts the file into the internal EPUB archive.
    fn read(&self, bytes: &[u8]) -> Result<Archive>;
}

/// Writes one output format.
pub trait FormatWriter: Send + Sync {
    fn id(&self) -> &'static str;
    fn extension(&self) -> &'static str;
    fn write(&self, book: &OpenBook) -> Result<Vec<u8>>;
}

/// What a [`Transform`] changed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransformReport {
    pub files_changed: usize,
    pub notes: Vec<String>,
}

/// One stage of the conversion/polish pipeline. Must be deterministic and idempotent where possible.
pub trait Transform: Send + Sync {
    fn id(&self) -> &'static str;
    fn apply(&self, book: &mut OpenBook) -> Result<TransformReport>;
}

/// What to look for.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct MetadataQuery {
    pub title: String,
    pub authors: Vec<String>,
    pub isbn: Option<String>,
}

/// A metadata provider (network or local).
pub trait MetadataSource: Send + Sync {
    fn id(&self) -> &'static str;
    fn lookup(&self, query: &MetadataQuery) -> Result<Vec<Metadata>>;
    /// Cover image bytes for a result, if the source has one.
    fn cover(&self, _meta: &Metadata) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }
}

/// A connected reading device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub mount: PathBuf,
    /// Set when this is additional storage (an SD card, a second disk) of another device: the
    /// `id` of that device. Such a storage is a destination for sends and is listed under its
    /// device, never an e-reader of its own.
    pub parent: Option<String>,
}

/// Talks to e-readers.
pub trait DeviceDriver: Send + Sync {
    fn id(&self) -> &'static str;
    /// Devices currently connected that this driver understands.
    fn detect(&self) -> Vec<DeviceInfo>;
    /// Formats the device opens, best first.
    fn preferred_formats(&self) -> &'static [&'static str];
    /// Copies a book to the device. Returns the path on the device.
    fn send(&self, device: &DeviceInfo, file_name: &str, bytes: &[u8]) -> Result<PathBuf>;
    fn list(&self, device: &DeviceInfo) -> Result<Vec<PathBuf>>;
}

/// Persistence of the catalogue. Implementations must be safe to call from several threads.
pub trait Store: Send + Sync {
    fn ids(&self) -> Vec<u64>;
    fn get(&self, id: u64) -> Option<Book>;
    fn put(&self, book: Book) -> Result<()>;
    fn remove(&self, id: u64) -> Result<bool>;
}

/// Runs transforms in order, stopping at the first error.
pub fn run_pipeline(
    book: &mut OpenBook,
    stages: &Registry<dyn Transform>,
) -> Result<Vec<(&'static str, TransformReport)>> {
    let mut out = Vec::new();
    for t in stages.iter() {
        let r = t
            .apply(book)
            .map_err(|e| e.context(format!("transform `{}` failed", t.id())))?;
        out.push((t.id(), r));
    }
    Ok(out)
}

/// Finds the reader for a file: by extension first, then by sniffing the content.
pub fn pick_reader<'a>(
    readers: &'a Registry<dyn FormatReader>,
    file_name: &str,
    head: &[u8],
) -> Result<&'a dyn FormatReader> {
    let ext = file_name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    if let Some(r) = readers
        .iter()
        .find(|r| r.extensions().contains(&ext.as_str()))
    {
        return Ok(r);
    }
    match readers.iter().find(|r| r.sniff(head)) {
        Some(r) => Ok(r),
        None => bail!("no reader for `{file_name}`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::{Chapter, create_epub};

    struct Upper;
    impl Transform for Upper {
        fn id(&self) -> &'static str {
            "upper"
        }
        fn apply(&self, book: &mut OpenBook) -> Result<TransformReport> {
            let mut md = book.metadata().clone();
            md.title = md.title.to_uppercase();
            book.set_metadata(md)?;
            Ok(TransformReport {
                files_changed: 1,
                notes: vec![],
            })
        }
    }

    struct Fails;
    impl Transform for Fails {
        fn id(&self) -> &'static str {
            "fails"
        }
        fn apply(&self, _: &mut OpenBook) -> Result<TransformReport> {
            bail!("boom")
        }
    }

    struct Txt;
    impl FormatReader for Txt {
        fn id(&self) -> &'static str {
            "txt"
        }
        fn extensions(&self) -> &'static [&'static str] {
            &["txt"]
        }
        fn sniff(&self, _: &[u8]) -> bool {
            false
        }
        fn read(&self, _: &[u8]) -> Result<Archive> {
            bail!("unused")
        }
    }

    struct Fb2;
    impl FormatReader for Fb2 {
        fn id(&self) -> &'static str {
            "fb2"
        }
        fn extensions(&self) -> &'static [&'static str] {
            &["fb2"]
        }
        fn sniff(&self, head: &[u8]) -> bool {
            head.starts_with(b"<FictionBook")
        }
        fn read(&self, _: &[u8]) -> Result<Archive> {
            bail!("unused")
        }
    }

    fn sample() -> OpenBook {
        let md = Metadata {
            title: "Hello".into(),
            authors: vec!["A".into()],
            ..Default::default()
        };
        let ch = Chapter {
            title: "One".into(),
            body: "<p>hi</p>".into(),
        };
        OpenBook::from_archive(create_epub(&md, &[ch], None)).unwrap()
    }

    #[test]
    fn pipeline_runs_stages_in_order() {
        let mut stages: Registry<dyn Transform> = Registry::new();
        stages.register(Box::new(Upper));
        let mut book = sample();
        let report = run_pipeline(&mut book, &stages).unwrap();
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].0, "upper");
        assert_eq!(book.metadata().title, "HELLO");
    }

    #[test]
    fn pipeline_names_the_failing_stage() {
        let mut stages: Registry<dyn Transform> = Registry::new();
        stages.register(Box::new(Upper));
        stages.register(Box::new(Fails));
        let err = run_pipeline(&mut sample(), &stages).unwrap_err();
        assert!(format!("{err:#}").contains("`fails`"));
    }

    #[test]
    fn reader_is_picked_by_extension_then_by_content() {
        let mut readers: Registry<dyn FormatReader> = Registry::new();
        readers.register(Box::new(Txt));
        readers.register(Box::new(Fb2));
        assert_eq!(pick_reader(&readers, "a.TXT", b"").unwrap().id(), "txt");
        // wrong extension, right content
        assert_eq!(
            pick_reader(&readers, "book.xml", b"<FictionBook xmlns=")
                .unwrap()
                .id(),
            "fb2"
        );
        assert!(pick_reader(&readers, "book.xyz", b"nothing").is_err());
    }
}
