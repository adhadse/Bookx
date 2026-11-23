//! Fonts and images on a real EPUB, verified with independent parsers (ttf-parser, image).

mod common;

use std::collections::BTreeSet;

use bookx_editing::fonts::{
    analyze_fonts, embed_referenced_fonts, remove_unused_fonts, subset_embedded_fonts,
};
use bookx_editing::images::{ImageOptions, optimize_images};
use bookx_editing::testfont::tiny_ttf;

#[test]
fn embed_subset_and_clean_up_fonts() {
    let mut book = common::book_from_bodies(&[
        "<h1 style=\"font-family: 'Tiny Test'\">ABC</h1><p style=\"font-family: 'Tiny Test'\">BA</p><p>plain</p>".to_string(),
    ]);
    let font = tiny_ttf(400);
    let looked_up = std::cell::RefCell::new(Vec::new());
    let lookup = |family: &str, weight: u16, italic: bool| {
        looked_up
            .borrow_mut()
            .push((family.to_string(), weight, italic));
        (family == "tiny test").then(|| font.clone())
    };
    let embedded = embed_referenced_fonts(&mut book, &lookup, false).unwrap();
    assert_eq!(embedded.len(), 2, "regular (p) and bold (h1): {embedded:?}");
    common::validate_epub(&book.archive.to_bytes().unwrap());
    assert!(looked_up.borrow().iter().any(|l| l.1 == 700));

    let report = analyze_fonts(&book);
    assert_eq!(report.embedded.len(), 2);
    assert!(
        report
            .embedded
            .iter()
            .all(|f| !f.unused && f.info.as_ref().unwrap().family.as_deref() == Some("Tiny Test"))
    );
    assert!(report.undeclared_families.is_empty());

    let before: usize = report.embedded.iter().map(|f| f.size).sum();
    let results = subset_embedded_fonts(&mut book);
    assert_eq!(results.len(), 2);
    let after: usize = results.iter().map(|r| r.after).sum();
    assert!(after * 3 < before, "{before} -> {after}");
    common::validate_epub(&book.archive.to_bytes().unwrap());
    for r in &results {
        let data = book.archive.get(&r.path).unwrap();
        let face = ttf_parser::Face::parse(data, 0).unwrap();
        let have: BTreeSet<char> = ['A', 'B', 'C']
            .into_iter()
            .filter(|c| face.glyph_index(*c).is_some())
            .collect();
        assert!(have.contains(&'B') && have.contains(&'A'));
        assert!(face.glyph_index('\u{450}').is_none());
        assert!(face.number_of_glyphs() <= 4);
    }
    // the same book later stops using the family: the fonts are removed together with their rules
    let ch = book
        .archive
        .get_string("OEBPS/chapter-1.xhtml")
        .unwrap()
        .replace("font-family: 'Tiny Test'", "color: red");
    book.archive.set_string("OEBPS/chapter-1.xhtml", &ch);
    let removed = remove_unused_fonts(&mut book).unwrap();
    assert_eq!(removed.len(), 2);
    assert!(
        !book
            .archive
            .get_string("OEBPS/style.css")
            .unwrap()
            .contains("@font-face")
    );
    common::validate_epub(&book.archive.to_bytes().unwrap());
}

fn jpeg(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([
            ((x * 3 + y) % 256) as u8,
            ((x ^ y) % 256) as u8,
            ((x * y / 5) % 256) as u8,
        ])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 98)
        .encode_image(&image::DynamicImage::ImageRgb8(img))
        .unwrap();
    out.into_inner()
}

#[test]
fn image_optimisation_keeps_the_book_valid() {
    let mut book = common::book_from_bodies(&[
        "<p><img src=\"img/photo.jpg\" alt=\"photo\"/><img src=\"img/pic.bmp\" alt=\"bmp\"/><img src=\"img/big.png\" alt=\"big\"/></p>".to_string(),
    ]);
    let mut bmp = Vec::new();
    image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(20, 20, |x, y| {
        image::Rgb([x as u8 * 9, y as u8 * 9, 7])
    }))
    .write_to(&mut std::io::Cursor::new(&mut bmp), image::ImageFormat::Bmp)
    .unwrap();
    book.archive.set("OEBPS/img/photo.jpg", jpeg(400, 300));
    book.archive.set("OEBPS/img/pic.bmp", bmp);
    book.archive.set("OEBPS/img/big.png", common::png(600, 400));
    let pkg = book.package.clone();
    for (id, f, m) in [
        ("ph", "photo.jpg", "image/jpeg"),
        ("bm", "pic.bmp", "image/bmp"),
        ("bg", "big.png", "image/png"),
    ] {
        pkg.add_manifest_item(&mut book.archive, id, &format!("OEBPS/img/{f}"), m, "")
            .unwrap();
    }
    book.reload_package().unwrap();
    let total_before = book
        .archive
        .names()
        .iter()
        .filter(|n| n.starts_with("OEBPS/img/"))
        .map(|n| book.archive.size_of(n))
        .sum::<usize>();
    let rep = optimize_images(
        &mut book,
        &ImageOptions {
            jpeg_quality: Some(60),
            max_width: Some(300),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(rep.changes.len(), 3, "{rep:#?}");
    let files = common::validate_epub(&book.archive.to_bytes().unwrap());
    assert!(files.contains_key("OEBPS/img/pic.png") && !files.contains_key("OEBPS/img/pic.bmp"));
    let total_after: usize = files
        .iter()
        .filter(|(n, _)| n.starts_with("OEBPS/img/"))
        .map(|(_, d)| d.len())
        .sum();
    assert!(
        total_after < total_before,
        "{total_before} -> {total_after}"
    );
    assert!(rep.bytes_saved() > 0);
    for (name, w) in [("OEBPS/img/photo.jpg", 300u32), ("OEBPS/img/big.png", 300)] {
        let img = image::load_from_memory(&files[name]).unwrap();
        assert_eq!(img.width(), w, "{name}");
    }
    assert_eq!(
        image::guess_format(&files["OEBPS/img/pic.png"]).unwrap(),
        image::ImageFormat::Png
    );
    let ch = std::str::from_utf8(&files["OEBPS/chapter-1.xhtml"]).unwrap();
    assert!(ch.contains("img/pic.png") && !ch.contains("pic.bmp"));
}
