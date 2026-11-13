//! Fixtures for the unit tests of the writers.

use std::io::Cursor;

use bookx_core::archive::Archive;
use bookx_core::book::OpenBook;
use bookx_core::convert::{create_epub, Chapter};
use bookx_core::opf::Metadata;

pub fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x * 9) as u8, (y * 7) as u8, 99]));
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("png");
    out.into_inner()
}

/// A book with one chapter per body; `OEBPS/pic.png` (8x6) exists.
pub fn book_with(bodies: &[&str]) -> OpenBook {
    let chapters: Vec<_> = bodies
        .iter()
        .enumerate()
        .map(|(i, b)| Chapter {
            title: format!("T{i}"),
            body: (*b).to_string(),
        })
        .collect();
    let md = Metadata {
        title: "Unit Book".into(),
        authors: vec!["A. Writer".into()],
        ..Default::default()
    };
    let mut arc: Archive = create_epub(&md, &chapters, None);
    arc.set("OEBPS/pic.png", png(8, 6));
    OpenBook::from_archive(Archive::from_bytes(arc.to_bytes().expect("zip")).expect("zip"))
        .expect("epub")
}
