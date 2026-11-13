//! CBC (Comic Book Collection): a zip with `comics.txt` listing CBZ files, one `file.cbz:Title`
//! per line. All listed comics are merged into one book with one chapter per comic whose body
//! is the comic's pages. An optional `metadata.opf` supplies the collection's metadata.

use anyhow::{bail, Context, Result};
use bookx_core::archive::Archive;
use bookx_core::contracts::FormatReader;
use bookx_core::convert::Chapter;
use bookx_core::opf::Metadata;
use bookx_core::util::xml_escape;

use crate::comic::{is_page_name, natural_cmp};
use crate::common::*;

const CSS: &str = "body { margin: 0; padding: 0; }\n.page { margin: 0; padding: 0; text-align: center; }\n.page img { max-width: 100%; max-height: 100vh; object-fit: contain; }\n";
const MAX_COMICS: usize = 2_000;

/// Reader for `.cbc` files.
pub struct CbcReader;

impl FormatReader for CbcReader {
    fn id(&self) -> &'static str {
        "cbc"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["cbc"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        zip_names(head).is_some_and(|n| n.iter().any(|f| f.eq_ignore_ascii_case("comics.txt")))
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        convert_cbc(bytes)
    }
}

/// One line of `comics.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComicEntry {
    pub file: String,
    pub title: String,
}

/// Parses `comics.txt`: `file.cbz:Title`, or just `file.cbz`. Blank lines and `#` comments are
/// skipped; the title defaults to the file name without extension.
pub fn parse_comics_txt(text: &str) -> Vec<ComicEntry> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // The file name ends at the first `.cb?` extension followed by `:` or the line end.
        let lower = line.to_ascii_lowercase();
        let split = [".cbz", ".cbr", ".cb7", ".cbt"]
            .iter()
            .filter_map(|e| lower.find(e).map(|p| p + e.len()))
            .filter(|end| {
                line[*end..].trim_start().is_empty() || line[*end..].trim_start().starts_with(':')
            })
            .min();
        let (file, title) = match split {
            Some(end) => (
                line[..end].trim().to_string(),
                line[end..]
                    .trim_start()
                    .trim_start_matches(':')
                    .trim()
                    .to_string(),
            ),
            None => match line.split_once(':') {
                Some((f, t)) => (f.trim().to_string(), t.trim().to_string()),
                None => (line.to_string(), String::new()),
            },
        };
        if file.is_empty() {
            continue;
        }
        let title = if title.is_empty() {
            let base = file.rsplit('/').next().unwrap_or(&file);
            base.rsplit_once('.')
                .map(|(s, _)| s)
                .unwrap_or(base)
                .to_string()
        } else {
            title
        };
        out.push(ComicEntry { file, title });
    }
    out
}

/// Converts a CBC archive.
pub fn convert_cbc(bytes: &[u8]) -> Result<Archive> {
    let zip = read_zip(bytes)?;
    let Some(list) = zip.get_ci("comics.txt") else {
        bail!("the CBC archive has no comics.txt");
    };
    let entries = parse_comics_txt(&decode_text(list));
    if entries.is_empty() {
        bail!("comics.txt lists no comics");
    }
    if entries.len() > MAX_COMICS {
        bail!("comics.txt lists {} comics", entries.len());
    }
    let mut chapters = Vec::new();
    let mut media = Vec::new();
    let mut cover: Option<(Vec<u8>, String)> = None;
    let mut budget = MAX_TOTAL;
    let mut page_no = 0usize;
    for e in &entries {
        let Some(inner) = zip
            .get(&e.file)
            .or_else(|| zip.get_ci(&e.file))
            .or_else(|| {
                let base = e.file.rsplit('/').next().unwrap_or(&e.file);
                zip.files
                    .iter()
                    .find(|(n, _)| {
                        n.rsplit('/')
                            .next()
                            .is_some_and(|b| b.eq_ignore_ascii_case(base))
                    })
                    .map(|(_, d)| d.as_slice())
            })
        else {
            bail!(
                "comics.txt lists `{}` but the archive does not contain it",
                e.file
            );
        };
        let comic = read_zip_limited(inner, MAX_ENTRY, budget)
            .with_context(|| format!("reading `{}`", e.file))?;
        budget = budget.saturating_sub(comic.files.iter().map(|(_, d)| d.len() as u64).sum());
        let mut pages: Vec<&(String, Vec<u8>)> = comic
            .files
            .iter()
            .filter(|(n, d)| is_page_name(n) && image_ext(d).is_some())
            .collect();
        if pages.is_empty() {
            bail!("`{}` contains no images", e.file);
        }
        pages.sort_by(|a, b| natural_cmp(&a.0, &b.0));
        let mut body = String::new();
        for (_, data) in pages {
            page_no += 1;
            let ext = image_ext(data).unwrap_or("jpg");
            let path = format!("images/page-{page_no:05}.{ext}");
            body.push_str(&format!(
                "<div class=\"page\"><img src=\"{path}\" alt=\"{}\"/></div>\n",
                xml_escape(&format!("{} page {page_no}", e.title))
            ));
            if cover.is_none() {
                cover = Some((data.clone(), ext.to_string()));
            }
            media.push(Media {
                path,
                data: data.clone(),
            });
        }
        chapters.push(Chapter {
            title: e.title.clone(),
            body,
        });
    }
    let mut md = Metadata::default();
    if let Some(opf) = zip.get_ci("metadata.opf") {
        if let Some(info) = parse_opf_info(&decode_xml(opf)) {
            md = info.md;
        }
    }
    if md.title.is_empty() {
        md.title = "Comic Book Collection".into();
    }
    build(Parts {
        md,
        chapters,
        cover,
        media,
        css: CSS.into(),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    fn comic(tag: u8, pages: usize) -> Vec<u8> {
        let imgs: Vec<(String, Vec<u8>)> = (0..pages)
            .map(|i| {
                let mut d = PNG.to_vec();
                d.push(tag * 10 + i as u8);
                (format!("p{}.png", pages - i), d)
            })
            .collect();
        let refs: Vec<(&str, &[u8])> = imgs
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect();
        make_zip(&refs)
    }

    fn sample() -> Vec<u8> {
        make_zip(&[
            ("comics.txt", b"one.cbz:First: The Beginning\ntwo.cbz:Second\n"),
            ("one.cbz", &comic(1, 2)),
            ("two.cbz", &comic(2, 3)),
            ("metadata.opf", br#"<package><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Collection</dc:title><dc:creator>Cee</dc:creator></metadata></package>"#),
        ])
    }

    #[test]
    fn comics_txt_parsing() {
        let v = parse_comics_txt("a.cbz:Title: with colon\n\n# note\nB.CBZ\nsub/c.cbr : C comic \nweird:thing\n:x\n\u{feff}d.cbz");
        let t: Vec<(&str, &str)> = v
            .iter()
            .map(|e| (e.file.as_str(), e.title.as_str()))
            .collect();
        assert_eq!(
            t,
            [
                ("a.cbz", "Title: with colon"),
                ("B.CBZ", "B"),
                ("sub/c.cbr", "C comic"),
                ("weird", "thing"),
                ("d.cbz", "d"),
            ]
        );
        assert!(parse_comics_txt("").is_empty());
        assert!(parse_comics_txt("   \n#only comment").is_empty());
    }

    #[test]
    fn merges_comics_into_chapters_with_all_pages() {
        let arc = CbcReader.read(&sample()).unwrap();
        let book = validate(&arc);
        assert_eq!(book.metadata().title, "Collection");
        assert_eq!(toc_titles(&book), ["First: The Beginning", "Second"]);
        assert_eq!(chapter_xml(&book, 0).matches("<img ").count(), 2);
        assert_eq!(chapter_xml(&book, 1).matches("<img ").count(), 3);
        // pages are numbered continuously across comics
        assert!(chapter_xml(&book, 1).contains("page-00003."));
        // natural order inside a comic: p1 before p2 (stored as p2, p1)
        let bytes = arc.to_bytes().unwrap();
        let z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut names: Vec<_> = z
            .file_names()
            .filter_map(|n| n.ok())
            .filter(|n| n.contains("page-"))
            .map(String::from)
            .collect();
        names.sort();
        assert_eq!(names.len(), 5);
    }

    #[test]
    fn page_order_follows_natural_file_names() {
        let arc = CbcReader.read(&sample()).unwrap();
        let bytes = arc.to_bytes().unwrap();
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut first = Vec::new();
        std::io::Read::read_to_end(
            &mut z.by_name("OEBPS/images/page-00001.png").unwrap(),
            &mut first,
        )
        .unwrap();
        // comic(1, 2): p1 holds tag 10+1 (stored second), p2 holds tag 10+0
        assert_eq!(*first.last().unwrap(), 11);
    }

    #[test]
    fn missing_or_bad_members_are_errors() {
        let z = make_zip(&[("comics.txt", b"gone.cbz:Gone\n")]);
        assert!(format!("{}", CbcReader.read(&z).unwrap_err()).contains("does not contain it"));
        let z = make_zip(&[("comics.txt", b"a.cbz:A\n"), ("a.cbz", b"not a zip")]);
        assert!(format!("{:#}", CbcReader.read(&z).unwrap_err()).contains("reading `a.cbz`"));
        let z = make_zip(&[
            ("comics.txt", b"a.cbz\n"),
            ("a.cbz", &make_zip(&[("x.txt", b"hi")])),
        ]);
        assert!(format!("{}", CbcReader.read(&z).unwrap_err()).contains("contains no images"));
        let z = make_zip(&[("comics.txt", b"\n# nothing\n")]);
        assert!(format!("{}", CbcReader.read(&z).unwrap_err()).contains("lists no comics"));
        let z = make_zip(&[("other.txt", b"x")]);
        assert!(format!("{}", CbcReader.read(&z).unwrap_err()).contains("no comics.txt"));
    }

    #[test]
    fn default_title_without_opf_and_case_insensitive_lookup() {
        let z = make_zip(&[("COMICS.TXT", b"A.CBZ:Up\n"), ("a.cbz", &comic(3, 1))]);
        assert!(CbcReader.sniff(&z));
        let book = validate(&CbcReader.read(&z).unwrap());
        assert_eq!(book.metadata().title, "Comic Book Collection");
    }

    #[test]
    fn sniff_ids_and_fuzz() {
        assert!(CbcReader.sniff(&sample()));
        assert!(!CbcReader.sniff(&make_zip(&[("a.png", PNG)])));
        assert!(!CbcReader.sniff(b"nope"));
        assert_eq!(CbcReader.id(), "cbc");
        assert_eq!(CbcReader.extensions(), &["cbc"]);
        fuzz_no_panic(&CbcReader, &sample());
        assert_rejects_garbage(&CbcReader);
    }
}
