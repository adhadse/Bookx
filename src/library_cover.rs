//! Cover tools of the library (roadmap L13): take the cover from the first page of a PDF and trim
//! a cover to the 2:3 shape of a book. No GTK.

use anyhow::{anyhow, Result};

/// The width of a cover made from a PDF page, in pixels.
pub const PDF_COVER_WIDTH: u32 = 800;

/// The first page of a PDF as a PNG cover.
pub fn pdf_cover(pdf: Vec<u8>) -> Result<Vec<u8>> {
    let doc = bookx_pdf::doc::Document::open(pdf, None)
        .map_err(|e| anyhow!("the PDF cannot be read: {e:?}"))?;
    doc.renderer()
        .render_png(0, PDF_COVER_WIDTH)
        .ok_or_else(|| anyhow!("the first page of the PDF could not be drawn"))
}

/// A cover cut in the middle to 2:3 (width to height). The same bytes when it is 2:3 already.
pub fn trim_cover(bytes: &[u8]) -> Result<Vec<u8>> {
    bookx_sources::cover::crop_to_aspect(bytes, 2, 3)
}

/// The extension of an image (`png`, `jpg`), by its first bytes.
pub fn image_ext(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG") {
        "png"
    } else {
        "jpg"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_pdf::testpdf::TestPdf;

    #[test]
    fn the_first_page_of_a_pdf_becomes_a_png_cover() {
        let png = pdf_cover(TestPdf::simple(&[&["Title page"], &["Two"]]).build()).unwrap();
        assert_eq!(image_ext(&png), "png");
        let (w, h, _) = bookx_sources::cover::info(&png).unwrap();
        assert!(w <= PDF_COVER_WIDTH + 1 && h > 0);
        assert!(pdf_cover(b"not a pdf".to_vec()).is_err());
    }

    #[test]
    fn a_page_shaped_cover_is_trimmed_to_two_by_three_and_a_good_one_is_kept() {
        let png = pdf_cover(TestPdf::simple(&[&["Title page"]]).build()).unwrap();
        let (w0, h0, _) = bookx_sources::cover::info(&png).unwrap();
        assert!(
            w0 * 3 > h0 * 2,
            "a letter page is wider than 2:3: {w0}x{h0}"
        );
        let out = trim_cover(&png).unwrap();
        let (w, h, _) = bookx_sources::cover::info(&out).unwrap();
        assert_eq!(h, h0, "the height stays");
        assert!((w as i64 * 3 - h as i64 * 2).abs() <= 3, "{w}x{h} is 2:3");
        let again = trim_cover(&out).unwrap();
        let (w2, h2, _) = bookx_sources::cover::info(&again).unwrap();
        assert_eq!((w2, h2), (w, h), "a trimmed cover stays as it is");
        assert!(trim_cover(b"junk").is_err());
    }
}
