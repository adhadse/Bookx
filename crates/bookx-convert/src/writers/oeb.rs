//! Writers that keep the EPUB's own files: OEB (a zip of the package rooted at `content.opf`, as
//! Calibre's OEB directory output) and ZIP (every file of the EPUB, `mimetype` and `META-INF`
//! included, without any ordering or compression rules).

use anyhow::Result;
use bookx_core::book::OpenBook;
use bookx_core::contracts::FormatWriter;

use super::flat::zip_files;

#[derive(Debug, Clone, Copy, Default)]
pub struct OebWriter;

#[derive(Debug, Clone, Copy, Default)]
pub struct ZipWriter;

impl FormatWriter for ZipWriter {
    fn id(&self) -> &'static str {
        "zip"
    }
    fn extension(&self) -> &'static str {
        "zip"
    }
    fn write(&self, book: &OpenBook) -> Result<Vec<u8>> {
        let files: Vec<(String, Vec<u8>)> = book
            .archive
            .names()
            .iter()
            .filter(|n| !n.ends_with('/'))
            .filter_map(|n| book.archive.get(n).map(|d| (n.clone(), d.to_vec())))
            .collect();
        zip_files(&files)
    }
}

impl FormatWriter for OebWriter {
    fn id(&self) -> &'static str {
        "oeb"
    }
    fn extension(&self) -> &'static str {
        "oeb"
    }
    fn write(&self, book: &OpenBook) -> Result<Vec<u8>> {
        let opf_path = &book.package.opf_path;
        let base = book.package.base_dir();
        let prefix = if base.is_empty() {
            String::new()
        } else {
            format!("{}/", base.trim_end_matches('/'))
        };
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        for n in book.archive.names() {
            if n.ends_with('/') || n == "mimetype" || n.starts_with("META-INF/") {
                continue;
            }
            let Some(data) = book.archive.get(n) else {
                continue;
            };
            let Some(rel) = n.strip_prefix(&prefix) else {
                continue;
            };
            let name = if n == opf_path {
                "content.opf".to_string()
            } else {
                rel.to_string()
            };
            files.push((name, data.to_vec()));
        }
        zip_files(&files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::book_with;

    #[test]
    fn identities() {
        assert_eq!((OebWriter.id(), OebWriter.extension()), ("oeb", "oeb"));
        assert_eq!((ZipWriter.id(), ZipWriter.extension()), ("zip", "zip"));
    }

    fn names(bytes: Vec<u8>) -> Vec<String> {
        let z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut n: Vec<String> = z
            .file_names()
            .map(|n| n.expect("name").into_owned())
            .collect();
        n.sort();
        n
    }

    #[test]
    fn zip_keeps_paths_and_oeb_roots_the_package() {
        let book = book_with(&["<p>x</p>"]);
        let zip_names = names(ZipWriter.write(&book).unwrap());
        assert!(zip_names.contains(&"mimetype".to_string()));
        assert!(zip_names.contains(&"OEBPS/chapter-1.xhtml".to_string()));
        let oeb_names = names(OebWriter.write(&book).unwrap());
        assert!(oeb_names.contains(&"content.opf".to_string()));
        assert!(oeb_names.contains(&"chapter-1.xhtml".to_string()));
        assert!(
            !oeb_names
                .iter()
                .any(|n| n == "mimetype" || n.starts_with("META-INF"))
        );
    }
}
