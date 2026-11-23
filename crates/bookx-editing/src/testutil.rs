//! Fixtures shared by unit tests.

use bookx_core::book::OpenBook;
use bookx_core::convert::{Chapter, create_epub};
use bookx_core::opf::Metadata;

/// Parses XML that may carry a doctype.
pub fn xml(s: &str) -> roxmltree::Document<'_> {
    roxmltree::Document::parse_with_options(
        s,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .unwrap()
}

pub fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x * 7) as u8, (y * 3) as u8, 90]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

pub fn make_book(bodies: &[&str]) -> OpenBook {
    let md = Metadata {
        title: "Fixture".into(),
        authors: vec!["A".into()],
        language: "en".into(),
        ..Default::default()
    };
    let chapters: Vec<Chapter> = bodies
        .iter()
        .enumerate()
        .map(|(i, b)| Chapter {
            title: format!("Ch {}", i + 1),
            body: format!("{b}\n"),
        })
        .collect();
    OpenBook::from_archive(create_epub(&md, &chapters, None)).unwrap()
}
