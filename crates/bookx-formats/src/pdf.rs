//! PDF as a reflowed book (experimental, roadmap C4 / "PDF reflow").
//!
//! The library keeps the PDF itself and shows it page by page in the fixed layout PDF view; this
//! reader makes the EPUB next to it: the text of the PDF in reading order as chapters (see
//! `bookx_pdf::reflow` for the heuristics), the title and author from the document information,
//! page 1 as the cover. That EPUB is what searching, the full-text index, the *Reflow* view of
//! the reader and *Convert to* use.
//!
//! A PDF that is scanned (no text layer), password protected or that has nothing to reflow still
//! imports: the EPUB then holds a note saying why, and the PDF view works.

use anyhow::{bail, Result};
use bookx_core::archive::Archive;
use bookx_core::contracts::FormatReader;
use bookx_core::convert::Chapter;
use bookx_core::opf::Metadata;
use bookx_pdf::{reflow, Document, OpenError};

use crate::common::*;

/// Reader for `.pdf` files.
pub struct PdfReader;

/// Width of the cover image in pixels.
const COVER_WIDTH: u32 = 520;

impl FormatReader for PdfReader {
    fn id(&self) -> &'static str {
        "pdf"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["pdf"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        head.starts_with(b"%PDF-")
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        convert_pdf(bytes, "")
    }
}

/// The note chapter of a PDF whose text cannot be reflowed.
fn note(title: &str, why: &str) -> Vec<Chapter> {
    vec![Chapter {
        title: title.to_string(),
        body: format!(
            "<h2>{}</h2>\n<p>{}</p>\n",
            xml_escape_text(title),
            xml_escape_text(why)
        ),
    }]
}

fn xml_escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Titles word processors and scanners leave behind are not worth keeping.
pub fn clean_title(t: &str) -> Option<String> {
    let t = t.trim().trim_matches('\0').trim();
    if t.chars().count() < 2 {
        return None;
    }
    let lower = t.to_lowercase();
    let junk = [
        "untitled", "unknown", "title", "document", "scan", "scanned",
    ];
    if junk.iter().any(|j| lower == *j) {
        return None;
    }
    if lower.starts_with("microsoft word - ")
        || lower.starts_with("microsoft powerpoint - ")
        || lower.starts_with("microsoft excel - ")
    {
        let rest = t.split_once(" - ").map(|x| x.1).unwrap_or("").trim();
        return clean_title(rest.trim_end_matches(".docx").trim_end_matches(".doc"));
    }
    // "Document1", "scan0001"
    let letters: String = lower.chars().filter(|c| c.is_alphabetic()).collect();
    if matches!(
        letters.as_str(),
        "document" | "scan" | "img" | "image" | "page"
    ) && lower.chars().any(|c| c.is_ascii_digit())
    {
        return None;
    }
    // a file name: not a title
    if lower.ends_with(".pdf")
        || lower.ends_with(".docx")
        || lower.ends_with(".doc")
        || lower.ends_with(".tex")
    {
        let stem = t.rsplit_once('.').map(|x| x.0).unwrap_or(t);
        return clean_title(stem);
    }
    Some(t.to_string())
}

/// `(title, author)` guessed from a file name: `Author - Title.pdf` or `Title.pdf`.
pub fn guess_from_file_name(file_name: &str) -> (String, Option<String>) {
    let stem = stem_of(file_name).replace('_', " ");
    let stem = stem.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some((a, t)) = stem.split_once(" - ") {
        let (a, t) = (a.trim(), t.trim());
        if !a.is_empty() && !t.is_empty() && !t.contains(" - ") && a.split_whitespace().count() <= 4
        {
            return (t.to_string(), Some(a.to_string()));
        }
    }
    (stem, None)
}

pub(crate) fn stem_of(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    base.rsplit_once('.')
        .filter(|(s, _)| !s.is_empty())
        .map(|x| x.0)
        .unwrap_or(base)
        .to_string()
}

/// Converts a PDF to the internal EPUB. `file_name` supplies the title when the PDF has none.
pub fn convert_pdf(bytes: &[u8], file_name: &str) -> Result<Archive> {
    let (guess_title, guess_author) = guess_from_file_name(file_name);
    let guess_title = if guess_title.trim().is_empty() {
        "Untitled".to_string()
    } else {
        guess_title
    };
    let opened = std::panic::catch_unwind(|| Document::open(bytes.to_vec(), None));
    let doc = match opened {
        Ok(Ok(d)) => d,
        Ok(Err(OpenError::NeedsPassword)) => {
            return stub(
                &guess_title,
                guess_author,
                "This PDF is password protected. Open it from the library and enter the password to read it.",
            );
        }
        Ok(Err(e)) => bail!("{e}"),
        Err(_) => bail!("the PDF could not be read"),
    };
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let info = doc.info();
        let pages = doc.all_text();
        let outline = doc.outline();
        let cover = doc.renderer().render_png(0, COVER_WIDTH);
        (info, pages, outline, cover)
    }));
    let Ok((info, pages, outline, cover)) = built else {
        bail!("the PDF could not be read");
    };
    let title = clean_title(&info.title).unwrap_or(guess_title);
    let author = if info.author.trim().is_empty() {
        guess_author
    } else {
        Some(info.author.trim().to_string())
    };
    let mut md = Metadata {
        title: title.clone(),
        authors: author.clone().into_iter().collect(),
        language: "en".into(),
        description: info.subject.trim().to_string(),
        tags: split_keywords(&info.keywords),
        ..Default::default()
    };
    if let Some(a) = &author {
        md.author_sort = a.clone();
    }
    md.title_sort = safe_title_sort(&title);

    let chapters = if bookx_pdf::reflow::is_scanned(&pages) {
        note(
            &title,
            "This PDF has no text layer (it is probably a scan), so there is nothing to reflow. Open it in the PDF view to read it page by page.",
        )
    } else {
        let ch = reflow::reflow(&pages, &outline, &title);
        if ch.is_empty() {
            note(&title, "No text could be read from this PDF.")
        } else {
            ch.into_iter()
                .map(|c| Chapter {
                    title: c.title,
                    body: c.html,
                })
                .collect()
        }
    };
    build(Parts {
        md,
        chapters,
        cover: cover.map(|c| (c, "png".to_string())),
        opf_meta: "    <meta name=\"bookx:reflowed-from\" content=\"pdf\"/>\n".to_string(),
        ..Default::default()
    })
}

fn stub(title: &str, author: Option<String>, why: &str) -> Result<Archive> {
    let md = Metadata {
        title: title.to_string(),
        title_sort: safe_title_sort(title),
        author_sort: author.clone().unwrap_or_default(),
        authors: author.into_iter().collect(),
        language: "en".into(),
        ..Default::default()
    };
    build(Parts {
        md,
        chapters: note(title, why),
        opf_meta: "    <meta name=\"bookx:reflowed-from\" content=\"pdf\"/>\n".to_string(),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use bookx_pdf::testpdf::{OutlineEntry, Page, TestPdf};

    fn story() -> Vec<u8> {
        let mut d = TestPdf::new().title("The Story").author("A. Writer");
        for i in 0..3 {
            d = d.page(
                Page::new(612.0, 792.0)
                    .text(
                        72.0,
                        700.0,
                        12.0,
                        &format!("Chapter text on page {} that runs on", i + 1),
                    )
                    .text(72.0, 686.0, 12.0, "and ends here with a full stop."),
            );
        }
        d.outline(OutlineEntry::new("Beginning", 0))
            .outline(OutlineEntry::new("Ending", 2))
            .build()
    }

    #[test]
    fn a_pdf_becomes_chapters_with_metadata_and_cover() {
        let arc = convert_pdf(&story(), "story.pdf").unwrap();
        let book = validate(&arc);
        assert_eq!(book.metadata().title, "The Story");
        assert_eq!(book.metadata().authors, vec!["A. Writer".to_string()]);
        let toc: Vec<String> = book
            .archive
            .names()
            .iter()
            .filter(|n| n.ends_with(".xhtml") && n.contains("chapter-"))
            .cloned()
            .collect();
        assert_eq!(toc.len(), 2, "{toc:?}");
        let ch1 = arc.get_string("OEBPS/chapter-1.xhtml").unwrap();
        assert!(ch1.contains("Chapter text on page 1 that runs on and ends here with a full stop."));
        assert!(book.package.cover_path().is_some(), "page 1 is the cover");
        let cover = book
            .archive
            .get(&book.package.cover_path().unwrap())
            .unwrap();
        assert_eq!(&cover[..4], b"\x89PNG");
    }

    #[test]
    fn the_file_name_stands_in_for_missing_metadata() {
        let pdf = TestPdf::simple(&[&["Some text here that is long enough to be text."]]).build();
        let arc = convert_pdf(&pdf, "Jane Austen - Emma.pdf").unwrap();
        let book = validate(&arc);
        assert_eq!(book.metadata().title, "Emma");
        assert_eq!(book.metadata().authors, vec!["Jane Austen".to_string()]);
    }

    #[test]
    fn a_scan_imports_with_a_note() {
        let pdf = TestPdf::new()
            .page(Page::new(300.0, 400.0).rect([10.0, 10.0, 290.0, 390.0], 0.5))
            .build();
        let arc = convert_pdf(&pdf, "scan_01.pdf").unwrap();
        let ch = arc.get_string("OEBPS/chapter-1.xhtml").unwrap();
        assert!(ch.contains("no text layer"), "{ch}");
        assert_eq!(validate(&arc).metadata().title, "scan 01");
    }

    #[test]
    fn a_protected_pdf_imports_with_a_note() {
        let pdf = TestPdf::simple(&[&["secret"]]).password("pw").build();
        let arc = convert_pdf(&pdf, "locked.pdf").unwrap();
        let ch = arc.get_string("OEBPS/chapter-1.xhtml").unwrap();
        assert!(ch.contains("password protected"));
        assert!(!ch.contains("secret"));
    }

    #[test]
    fn garbage_is_rejected() {
        for f in [&b"%PDF-1.7\n1 0 obj"[..], b"%PDF-", b""] {
            assert!(convert_pdf(f, "x.pdf").is_err());
        }
    }

    #[test]
    fn titles_from_word_processors_are_cleaned() {
        assert_eq!(
            clean_title("Microsoft Word - Report.docx"),
            Some("Report".into())
        );
        assert_eq!(clean_title("Untitled"), None);
        assert_eq!(clean_title("Document1"), None);
        assert_eq!(clean_title("paper.pdf"), Some("paper".into()));
        assert_eq!(clean_title("  Real Title "), Some("Real Title".into()));
        assert_eq!(clean_title(""), None);
    }

    #[test]
    fn file_names_are_guessed() {
        assert_eq!(
            guess_from_file_name("a/b/Plain_title.pdf"),
            ("Plain title".into(), None)
        );
        assert_eq!(
            guess_from_file_name("Frank Herbert - Dune.pdf"),
            ("Dune".into(), Some("Frank Herbert".into()))
        );
        assert_eq!(guess_from_file_name("A - B - C.pdf").1, None);
    }

    #[test]
    fn sniff_and_extension() {
        assert_eq!(PdfReader.extensions(), &["pdf"]);
        assert!(PdfReader.sniff(b"%PDF-1.4"));
        for other in [&b"PK\x03\x04"[..], b"PDF-1.4", b"", b"%PDF"] {
            assert!(!PdfReader.sniff(other));
        }
    }

    #[test]
    fn read_any_converts_by_extension_and_by_content() {
        for name in ["a.pdf", "noextension"] {
            let arc = crate::read_any(name, &story()).unwrap();
            assert_eq!(validate(&arc).metadata().title, "The Story");
        }
    }

    #[test]
    fn fuzz_does_not_panic() {
        fuzz_no_panic(&PdfReader, &story());
        assert_rejects_garbage(&PdfReader);
    }
}
