//! Add or replace the cover (CE-129): pick an image already in the book or bring a new one, crop and scale it,
//! make the cover page that shows it, and keep every place that names the cover consistent (OPF `cover-image`
//! property and `<meta name="cover">`, guide, navigation landmarks, spine).

use anyhow::{bail, Context, Result};
use bookx_core::book::OpenBook;
use bookx_core::util::{extension, relative_href, xml_escape};
use image::{DynamicImage, ImageFormat};

use crate::files::add_file;
use crate::images;
use crate::markup::is_markup_path;
use crate::opf_edit::edit_item_properties;
use crate::semantics::{read_semantics, set_semantic, Semantic};

/// Largest image the cover dialog opens, in bytes and in pixels (guards against decompression bombs).
pub const MAX_COVER_BYTES: usize = 40 * 1024 * 1024;
pub const MAX_COVER_PIXELS: u64 = 64_000_000;

/// Where the cover picture comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverSource {
    /// An image that is part of the book (archive path).
    Existing(String),
    /// Bytes read from a file or generated; `ext` is the extension to store it under.
    New { data: Vec<u8>, ext: String },
}

/// A crop in fractions of the image (0..=1): left, top, width, height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crop {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Crop {
    pub fn is_whole(&self) -> bool {
        self.x <= 0.0 && self.y <= 0.0 && self.w >= 1.0 && self.h >= 1.0
    }

    /// Pixel rectangle `(x, y, w, h)` for an image, or `None` when the crop is empty.
    pub fn pixels(&self, iw: u32, ih: u32) -> Option<(u32, u32, u32, u32)> {
        let x = (self.x.clamp(0.0, 1.0) * f64::from(iw)).round() as u32;
        let y = (self.y.clamp(0.0, 1.0) * f64::from(ih)).round() as u32;
        let w = ((self.w.clamp(0.0, 1.0) * f64::from(iw)).round() as u32).min(iw.saturating_sub(x));
        let h = ((self.h.clamp(0.0, 1.0) * f64::from(ih)).round() as u32).min(ih.saturating_sub(y));
        (w > 0 && h > 0).then_some((x, y, w, h))
    }
}

/// The largest crop of the given aspect ratio (width / height) that fits an image, centred.
pub fn centered_crop(iw: u32, ih: u32, aspect: f64) -> Crop {
    if iw == 0 || ih == 0 || aspect <= 0.0 {
        return Crop {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        };
    }
    let cur = f64::from(iw) / f64::from(ih);
    if cur > aspect {
        let w = aspect / cur;
        Crop {
            x: (1.0 - w) / 2.0,
            y: 0.0,
            w,
            h: 1.0,
        }
    } else {
        let h = cur / aspect;
        Crop {
            x: 0.0,
            y: (1.0 - h) / 2.0,
            w: 1.0,
            h,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CoverOptions {
    pub source: CoverSource,
    pub crop: Option<Crop>,
    /// Scale down to fit this box (the cover is never enlarged).
    pub max_size: Option<(u32, u32)>,
    /// Make (or update) the cover page that shows the image.
    pub page: bool,
    /// Delete the previous cover image when nothing uses it any more.
    pub remove_old: bool,
}

impl CoverOptions {
    pub fn new(source: CoverSource) -> Self {
        Self {
            source,
            crop: None,
            max_size: None,
            page: true,
            remove_old: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoverResult {
    pub image: String,
    pub page: Option<String>,
    pub removed: Vec<String>,
}

/// Reads an image with limits on its size and dimensions.
pub fn decode_limited(data: &[u8]) -> Result<DynamicImage> {
    if data.len() > MAX_COVER_BYTES {
        bail!("the image is too large ({} MB)", data.len() / (1024 * 1024));
    }
    let reader = image::ImageReader::new(std::io::Cursor::new(data))
        .with_guessed_format()
        .context("cannot read the image")?;
    let (w, h) = reader
        .into_dimensions()
        .context("this is not an image Bookx can read")?;
    if u64::from(w) * u64::from(h) > MAX_COVER_PIXELS {
        bail!("the image is too large ({w}x{h} pixels)");
    }
    images::decode(data)
}

/// Applies the crop and the size limit; returns the bytes and their extension. An image that needs no change
/// keeps its bytes.
pub fn prepare_image(
    data: &[u8],
    ext: &str,
    crop: Option<Crop>,
    max: Option<(u32, u32)>,
) -> Result<(Vec<u8>, String)> {
    let img = decode_limited(data)?;
    let fmt = images::guess_format(data);
    let ext_in = match (ext.to_ascii_lowercase().as_str(), fmt) {
        (_, Some(ImageFormat::Png)) => "png",
        (_, Some(ImageFormat::Jpeg)) => "jpg",
        (_, Some(ImageFormat::Gif)) => "gif",
        (_, Some(ImageFormat::WebP)) => "webp",
        ("jpeg" | "jpg", _) => "jpg",
        ("png", _) => "png",
        _ => "png",
    };
    let crop = crop.filter(|c| !c.is_whole());
    let target = max.and_then(|(mw, mh)| {
        let (cw, ch) = match crop.and_then(|c| c.pixels(img.width(), img.height())) {
            Some((_, _, w, h)) => (w, h),
            None => (img.width(), img.height()),
        };
        images::fit_within(cw, ch, Some(mw), Some(mh))
    });
    if crop.is_none() && target.is_none() {
        return Ok((data.to_vec(), ext_in.to_string()));
    }
    let mut img = img;
    if let Some(c) = crop {
        let (x, y, w, h) = c
            .pixels(img.width(), img.height())
            .context("the crop is empty")?;
        img = img.crop_imm(x, y, w, h);
    }
    if let Some((w, h)) = target {
        img = img.resize_exact(w, h, image::imageops::FilterType::Lanczos3);
    }
    if ext_in == "jpg" {
        Ok((
            images::convert_to_jpeg(&encode_rgb(&img)?, 90)?,
            "jpg".into(),
        ))
    } else {
        Ok((images::convert_to_png(&encode_rgb(&img)?)?, "png".into()))
    }
}

fn encode_rgb(img: &DynamicImage) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png)
        .context("cannot encode the image")?;
    Ok(out)
}

/// The XHTML of the cover page for an image at `src` (relative to the page).
pub fn cover_page_xhtml(src: &str, title: &str, lang: &str) -> String {
    let lang = if lang.is_empty() { "en" } else { lang };
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" xml:lang=\"{lang}\" lang=\"{lang}\">\n<head>\n  <title>{}</title>\n  <style type=\"text/css\">\n    body {{ margin: 0; padding: 0; text-align: center; }}\n    img {{ max-width: 100%; max-height: 100vh; height: auto; }}\n  </style>\n</head>\n<body>\n  <section epub:type=\"cover\">\n    <img src=\"{}\" alt=\"{}\"/>\n  </section>\n</body>\n</html>\n",
        xml_escape(title),
        xml_escape(src),
        xml_escape(title)
    )
}

/// The page that is the cover now: the guide/landmark entry, else the first page that is mostly one image.
pub fn find_cover_page(book: &OpenBook) -> Option<String> {
    if let Some(e) = read_semantics(book)
        .into_iter()
        .find(|e| e.semantic == Semantic::Cover)
    {
        let p = e.href.split('#').next().unwrap_or("").to_string();
        if is_markup_path(&p) && book.archive.contains(&p) {
            return Some(p);
        }
    }
    let first = book.spine_paths().into_iter().next()?;
    let src = book.archive.get_string(&first)?;
    let text = crate::markup::plain_text(&src);
    (text.trim().chars().count() < 40 && (src.contains("<img") || src.contains("<svg")))
        .then_some(first)
}

fn mark_cover_image(book: &mut OpenBook, path: &str) -> Result<()> {
    let id = book
        .package
        .item_by_path(path)
        .map(|i| i.id.clone())
        .with_context(|| format!("{path} is not in the manifest"))?;
    if !book.package.is_epub3() {
        let mut pkg = book.package.clone();
        pkg.cover_id = Some(id);
        pkg.write_metadata(&mut book.archive)?;
        return book.reload_package();
    }
    let others: Vec<String> = book
        .package
        .manifest
        .iter()
        .filter(|i| i.id != id && i.properties.split_whitespace().any(|p| p == "cover-image"))
        .map(|i| i.id.clone())
        .collect();
    for o in others {
        crate::opf_edit::edit_opf(book, |t| {
            Ok(edit_item_properties(t, &o, |p| {
                p.split_whitespace()
                    .filter(|x| *x != "cover-image")
                    .collect::<Vec<_>>()
                    .join(" ")
            }))
        })?;
    }
    crate::opf_edit::edit_opf(book, |t| {
        Ok(edit_item_properties(t, &id, |p| {
            crate::opf_edit::add_property(p, "cover-image")
        }))
    })?;
    let mut pkg = book.package.clone();
    pkg.cover_id = Some(id);
    pkg.write_metadata(&mut book.archive)?;
    book.reload_package()
}

/// Sets the cover. See [`CoverOptions`]; the caller takes the checkpoint first.
pub fn set_cover(book: &mut OpenBook, opts: &CoverOptions) -> Result<CoverResult> {
    let old = book.package.cover_path();
    let (data, ext) = match &opts.source {
        CoverSource::Existing(p) => (
            book.archive
                .get(p)
                .with_context(|| format!("{p} is not in the book"))?
                .to_vec(),
            extension(p),
        ),
        CoverSource::New { data, ext } => (data.clone(), ext.clone()),
    };
    if !matches!(
        ext.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp"
    ) {
        bail!("a cover must be a PNG, JPEG, GIF or WebP image");
    }
    let unchanged = opts.crop.is_none_or(|c| c.is_whole()) && opts.max_size.is_none();
    let image = match (&opts.source, unchanged) {
        (CoverSource::Existing(p), true) => {
            decode_limited(&data)?;
            mark_cover_image(book, p)?;
            p.clone()
        }
        _ => {
            let (bytes, ext) = prepare_image(&data, &ext, opts.crop, opts.max_size)?;
            let path = add_file(book, &format!("cover.{ext}"), bytes, None)?;
            mark_cover_image(book, &path)?;
            path
        }
    };
    let mut result = CoverResult {
        image: image.clone(),
        ..Default::default()
    };
    if opts.page {
        let title = book.metadata().title.clone();
        let lang = book.metadata().language.clone();
        let existing = find_cover_page(book);
        let page = match existing {
            Some(p) => {
                let rel = relative_href(&p, &image);
                book.archive
                    .set_string(&p, &cover_page_xhtml(&rel, &title, &lang));
                p
            }
            None => {
                let dir = crate::files::unique_path(book, book.package.base_dir(), "cover.xhtml");
                let rel = relative_href(&dir, &image);
                let name = bookx_core::util::file_name(&dir).to_string();
                let p = add_file(
                    book,
                    &name,
                    cover_page_xhtml(&rel, &title, &lang).into_bytes(),
                    None,
                )?;
                move_to_front(book, &p)?;
                p
            }
        };
        set_semantic(book, Semantic::Cover, &page, "Cover")?;
        result.page = Some(page);
    }
    if opts.remove_old {
        if let Some(o) = old.filter(|o| *o != image) {
            let unused = crate::book_tools::unused_media(book)
                .iter()
                .any(|u| u.path == o);
            if unused {
                result.removed = crate::files::delete_files(book, std::slice::from_ref(&o))?;
            }
        }
    }
    Ok(result)
}

fn move_to_front(book: &mut OpenBook, path: &str) -> Result<()> {
    crate::book_tools::spine_move_to(book, path, 0)?;
    Ok(())
}

/// Removes the cover: the image stays in the book, but it is no longer marked as the cover and the cover page
/// entry in the guide and landmarks goes.
pub fn remove_cover(book: &mut OpenBook) -> Result<()> {
    book.remove_cover()?;
    crate::semantics::clear_semantic(book, Semantic::Cover)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{make_book, png};

    fn book_with_image() -> OpenBook {
        let mut b = make_book(&["<p>one</p>", "<p>two</p>"]);
        add_file(&mut b, "pic.png", png(200, 300), None).unwrap();
        b
    }

    #[test]
    fn crop_geometry() {
        let c = centered_crop(400, 200, 0.5);
        assert!((c.w - 0.25).abs() < 1e-9 && c.h == 1.0 && (c.x - 0.375).abs() < 1e-9);
        let c = centered_crop(100, 400, 1.0);
        assert_eq!(c.pixels(100, 400), Some((0, 150, 100, 100)));
        assert!(Crop {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0
        }
        .is_whole());
        assert_eq!(
            Crop {
                x: 0.9,
                y: 0.0,
                w: 0.0,
                h: 1.0
            }
            .pixels(10, 10),
            None
        );
        assert_eq!(
            Crop {
                x: 0.5,
                y: 0.0,
                w: 1.0,
                h: 1.0
            }
            .pixels(10, 10),
            Some((5, 0, 5, 10))
        );
    }

    #[test]
    fn prepare_crops_scales_and_keeps_untouched_bytes() {
        let src = png(200, 300);
        let (same, ext) = prepare_image(&src, "png", None, None).unwrap();
        assert_eq!((same, ext.as_str()), (src.clone(), "png"));
        let (out, _) = prepare_image(
            &src,
            "png",
            Some(Crop {
                x: 0.0,
                y: 0.0,
                w: 0.5,
                h: 0.5,
            }),
            Some((50, 50)),
        )
        .unwrap();
        let (w, h) = images::image_dimensions(&out);
        assert!(w <= 50 && h <= 50, "{w}x{h}");
        assert!(prepare_image(b"not an image", "png", None, None).is_err());
    }

    #[test]
    fn limits_refuse_huge_images() {
        assert!(decode_limited(&vec![0u8; MAX_COVER_BYTES + 1]).is_err());
        // a PNG header that claims 100000 x 100000 pixels
        let mut png_head = png(8, 8);
        png_head[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        png_head[20..24].copy_from_slice(&100_000u32.to_be_bytes());
        let e = decode_limited(&png_head).unwrap_err().to_string();
        assert!(e.contains("too large") || e.contains("not an image"), "{e}");
    }

    #[test]
    fn existing_image_becomes_the_cover_with_page_guide_and_landmarks() {
        let mut b = book_with_image();
        let img = b
            .archive
            .names()
            .iter()
            .find(|n| n.ends_with("pic.png"))
            .unwrap()
            .clone();
        let r = set_cover(
            &mut b,
            &CoverOptions::new(CoverSource::Existing(img.clone())),
        )
        .unwrap();
        assert_eq!(r.image, img);
        assert_eq!(b.package.cover_path().as_deref(), Some(img.as_str()));
        let page = r.page.expect("a cover page");
        assert_eq!(b.spine_paths()[0], page, "the cover page comes first");
        let src = b.archive.get_string(&page).unwrap();
        assert!(
            src.contains("pic.png") && src.contains("epub:type=\"cover\""),
            "{src}"
        );
        let sem = read_semantics(&b);
        assert!(sem
            .iter()
            .any(|e| e.semantic == Semantic::Cover && e.href == page));
        let opf = b.archive.get_string(&b.package.opf_path).unwrap();
        assert!(opf.contains("cover-image"), "{opf}");
        // doing it again reuses the page and adds nothing
        let files = b.archive.names().len();
        let r2 = set_cover(&mut b, &CoverOptions::new(CoverSource::Existing(img))).unwrap();
        assert_eq!(r2.page.as_deref(), Some(page.as_str()));
        assert_eq!(b.archive.names().len(), files);
    }

    #[test]
    fn a_new_image_replaces_the_old_cover_consistently() {
        let mut b = book_with_image();
        let img = b
            .archive
            .names()
            .iter()
            .find(|n| n.ends_with("pic.png"))
            .unwrap()
            .clone();
        set_cover(
            &mut b,
            &CoverOptions::new(CoverSource::Existing(img.clone())),
        )
        .unwrap();
        let mut o = CoverOptions::new(CoverSource::New {
            data: png(120, 180),
            ext: "png".into(),
        });
        o.remove_old = true;
        let r = set_cover(&mut b, &o).unwrap();
        assert_ne!(r.image, img);
        assert_eq!(b.package.cover_path().as_deref(), Some(r.image.as_str()));
        let page = r.page.unwrap();
        let src = b.archive.get_string(&page).unwrap();
        assert!(
            src.contains(bookx_core::util::file_name(&r.image)),
            "the page shows the new image: {src}"
        );
        assert!(!src.contains("pic.png"));
        assert_eq!(r.removed, vec![img.clone()]);
        assert!(!b.archive.contains(&img));
        let covers = b
            .package
            .manifest
            .iter()
            .filter(|i| i.properties.contains("cover-image"))
            .count();
        assert_eq!(covers, 1, "exactly one cover-image item");
        assert_eq!(b.package.cover_path().as_deref(), Some(r.image.as_str()));
        let reopened = OpenBook::from_archive(b.archive.clone()).unwrap();
        assert_eq!(reopened.package.cover_path(), b.package.cover_path());
    }

    #[test]
    fn without_a_page_only_the_image_changes() {
        let mut b = book_with_image();
        let mut o = CoverOptions::new(CoverSource::New {
            data: png(120, 180),
            ext: "png".into(),
        });
        o.page = false;
        let r = set_cover(&mut b, &o).unwrap();
        assert!(r.page.is_none());
        assert!(b.package.cover_path().is_some());
        assert!(!read_semantics(&b)
            .iter()
            .any(|e| e.semantic == Semantic::Cover));
    }

    #[test]
    fn bad_sources_are_refused_and_nothing_changes() {
        let mut b = book_with_image();
        let before = b.archive.to_bytes().unwrap();
        assert!(set_cover(
            &mut b,
            &CoverOptions::new(CoverSource::Existing("OEBPS/none.png".into()))
        )
        .is_err());
        assert!(set_cover(
            &mut b,
            &CoverOptions::new(CoverSource::New {
                data: b"x".to_vec(),
                ext: "png".into()
            })
        )
        .is_err());
        assert!(set_cover(
            &mut b,
            &CoverOptions::new(CoverSource::New {
                data: png(10, 10),
                ext: "exe".into()
            })
        )
        .is_err());
        assert_eq!(b.archive.to_bytes().unwrap(), before);
    }

    #[test]
    fn remove_cover_clears_marks() {
        let mut b = book_with_image();
        set_cover(
            &mut b,
            &CoverOptions::new(CoverSource::New {
                data: png(120, 180),
                ext: "png".into(),
            }),
        )
        .unwrap();
        remove_cover(&mut b).unwrap();
        let opf = b.archive.get_string(&b.package.opf_path).unwrap();
        assert!(
            !opf.contains("cover-image") && !opf.contains("name=\"cover\""),
            "{opf}"
        );
        assert!(!read_semantics(&b)
            .iter()
            .any(|e| e.semantic == Semantic::Cover));
    }

    #[test]
    fn cover_page_escapes_the_title() {
        let x = cover_page_xhtml("../images/c.png", "A & B <i>", "de");
        assert!(bookx_core::util::parse_xml(&x).is_ok());
        assert!(x.contains("lang=\"de\"") && x.contains("A &amp; B"));
    }
}
