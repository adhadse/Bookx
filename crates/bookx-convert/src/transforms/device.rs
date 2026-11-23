//! Device profiles (C9): image limits, margins and fonts for Kindle, Kobo, e-ink and tablets.

use std::io::Cursor;

use anyhow::{Context, Result};
use bookx_core::book::OpenBook;
use bookx_core::contracts::{Transform, TransformReport};
use bookx_core::util::{join_path, relative_href};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{ImageFormat, ImageReader};
use serde::{Deserialize, Serialize};

use super::map_spine_files;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileKind {
    Kindle,
    Kobo,
    GenericEink,
    Tablet,
}

impl ProfileKind {
    pub const ALL: [ProfileKind; 4] = [Self::Kindle, Self::Kobo, Self::GenericEink, Self::Tablet];

    pub fn id(&self) -> &'static str {
        match self {
            Self::Kindle => "kindle",
            Self::Kobo => "kobo",
            Self::GenericEink => "generic_eink",
            Self::Tablet => "tablet",
        }
    }

    pub fn from_id(id: &str) -> Option<ProfileKind> {
        Self::ALL.into_iter().find(|k| k.id() == id)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceProfile {
    pub name: String,
    pub max_width: u32,
    pub max_height: u32,
    /// Quality used when a JPEG has to be re-encoded (1..=100).
    pub jpeg_quality: u8,
    /// CSS `margin` value for `body`, e.g. `0.4em 2%`. Empty = leave alone.
    pub margin: String,
    /// Font family list for `body`. Empty = leave alone.
    pub fonts: Vec<String>,
    /// Formats the device can show; images in other formats are left untouched.
    pub formats: Vec<String>,
}

impl Default for DeviceProfile {
    fn default() -> Self {
        DeviceProfile::generic_eink()
    }
}

pub const PROFILE_CSS_FILE: &str = "bookx-device.css";

impl DeviceProfile {
    pub fn for_kind(kind: ProfileKind) -> DeviceProfile {
        match kind {
            ProfileKind::Kindle => DeviceProfile::kindle(),
            ProfileKind::Kobo => DeviceProfile::kobo(),
            ProfileKind::GenericEink => DeviceProfile::generic_eink(),
            ProfileKind::Tablet => DeviceProfile::tablet(),
        }
    }

    pub fn kindle() -> DeviceProfile {
        DeviceProfile {
            name: "kindle".into(),
            max_width: 1072,
            max_height: 1448,
            jpeg_quality: 80,
            margin: "0 1%".into(),
            fonts: vec!["serif".into()],
            formats: vec!["jpeg".into(), "png".into()],
        }
    }

    pub fn kobo() -> DeviceProfile {
        DeviceProfile {
            name: "kobo".into(),
            max_width: 1264,
            max_height: 1680,
            jpeg_quality: 85,
            margin: "0 2%".into(),
            fonts: vec![],
            formats: vec!["jpeg".into(), "png".into()],
        }
    }

    pub fn generic_eink() -> DeviceProfile {
        DeviceProfile {
            name: "generic_eink".into(),
            max_width: 758,
            max_height: 1024,
            jpeg_quality: 75,
            margin: "0.3em 3%".into(),
            fonts: vec!["serif".into()],
            formats: vec!["jpeg".into(), "png".into()],
        }
    }

    pub fn tablet() -> DeviceProfile {
        DeviceProfile {
            name: "tablet".into(),
            max_width: 1600,
            max_height: 2560,
            jpeg_quality: 90,
            margin: "0 5%".into(),
            fonts: vec![],
            formats: vec!["jpeg".into(), "png".into()],
        }
    }

    pub fn css(&self) -> String {
        let mut css = format!("/* Bookx device profile: {} */\n", self.name);
        let mut body = String::new();
        if !self.margin.is_empty() {
            body.push_str(&format!("margin: {};", self.margin));
        }
        if !self.fonts.is_empty() {
            if !body.is_empty() {
                body.push(' ');
            }
            body.push_str(&format!("font-family: {};", self.fonts.join(", ")));
        }
        if !body.is_empty() {
            css.push_str(&format!("body {{ {body} }}\n"));
        }
        css.push_str("img { max-width: 100%; max-height: 100%; }\n");
        css
    }

    fn allows(&self, ext: &str) -> bool {
        let fmt = if ext == "jpg" { "jpeg" } else { ext };
        self.formats.iter().any(|f| f == fmt)
    }

    /// Returns resized bytes when the image exceeds the limits, `None` when it already fits (or
    /// cannot be decoded).
    pub fn fit_image(&self, data: &[u8], ext: &str) -> Option<Vec<u8>> {
        let ext = ext.to_ascii_lowercase();
        if !self.allows(&ext) || self.max_width == 0 || self.max_height == 0 {
            return None;
        }
        let reader = ImageReader::new(Cursor::new(data))
            .with_guessed_format()
            .ok()?;
        let fmt = reader.format()?;
        let (w, h) = reader.into_dimensions().ok()?;
        if w <= self.max_width && h <= self.max_height {
            return None;
        }
        let img = image::load_from_memory_with_format(data, fmt).ok()?;
        let small = img.resize(self.max_width, self.max_height, FilterType::Lanczos3);
        let mut out = Cursor::new(Vec::new());
        match fmt {
            ImageFormat::Jpeg => {
                let rgb = small.to_rgb8();
                JpegEncoder::new_with_quality(&mut out, self.jpeg_quality.clamp(1, 100))
                    .encode_image(&rgb)
                    .ok()?;
            }
            ImageFormat::Png => small.write_to(&mut out, ImageFormat::Png).ok()?,
            _ => return None,
        }
        Some(out.into_inner())
    }
}

impl Transform for DeviceProfile {
    fn id(&self) -> &'static str {
        "device_profile"
    }

    fn apply(&self, book: &mut OpenBook) -> Result<TransformReport> {
        let mut report = TransformReport::default();

        // 1. images
        let images: Vec<String> = book
            .package
            .manifest
            .iter()
            .filter(|i| i.media_type == "image/jpeg" || i.media_type == "image/png")
            .map(|i| book.package.item_path(i))
            .collect();
        for path in images {
            let Some(data) = book.archive.get(&path) else {
                continue;
            };
            let ext = bookx_core::util::extension(&path);
            if let Some(new) = self.fit_image(data, &ext) {
                report
                    .notes
                    .push(format!("{path}: {} -> {} bytes", data.len(), new.len()));
                book.archive.set(&path, new);
                report.files_changed += 1;
            }
        }

        // 2. stylesheet
        let css_path = join_path(book.package.base_dir(), PROFILE_CSS_FILE);
        let css = self.css();
        let exists = book.archive.contains(&css_path);
        if book.archive.get_string(&css_path).as_deref() != Some(css.as_str()) {
            book.archive.set_string(&css_path, &css);
            report.files_changed += 1;
        }
        if !exists || book.package.item_by_path(&css_path).is_none() {
            let pkg = book.package.clone();
            pkg.add_manifest_item(
                &mut book.archive,
                "bookx-device-css",
                &css_path,
                "text/css",
                "",
            )
            .context("registering the device stylesheet")?;
            book.reload_package()?;
            report.files_changed += 1;
        }

        // 3. link it from every document
        let n = map_spine_files(book, 0, |path, text| {
            if text.contains(PROFILE_CSS_FILE) {
                return None;
            }
            let href = relative_href(path, &css_path);
            let link = format!("<link rel=\"stylesheet\" type=\"text/css\" href=\"{href}\"/>\n");
            let lower = text.to_ascii_lowercase();
            lower.find("</head>").map(|pos| {
                let mut s = text.to_string();
                s.insert_str(pos, &link);
                s
            })
        })?;
        report.files_changed += n;
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;

    fn r#gen(w: u32, h: u32, fmt: ImageFormat) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([(x % 251) as u8, (y % 241) as u8, ((x + y) % 199) as u8])
        });
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut out, fmt)
            .unwrap();
        out.into_inner()
    }

    fn dims(data: &[u8]) -> (u32, u32) {
        image::load_from_memory(data).unwrap().dimensions_pair()
    }

    trait Dims {
        fn dimensions_pair(&self) -> (u32, u32);
    }
    impl Dims for image::DynamicImage {
        fn dimensions_pair(&self) -> (u32, u32) {
            (self.width(), self.height())
        }
    }

    fn small_profile() -> DeviceProfile {
        DeviceProfile {
            name: "test".into(),
            max_width: 100,
            max_height: 100,
            jpeg_quality: 70,
            margin: "0 1em".into(),
            fonts: vec!["Georgia".into(), "serif".into()],
            formats: vec!["jpeg".into(), "png".into()],
        }
    }

    #[test]
    fn large_png_is_downscaled_keeping_aspect_ratio() {
        let data = r#gen(400, 200, ImageFormat::Png);
        let out = small_profile().fit_image(&data, "png").unwrap();
        assert_eq!(dims(&out), (100, 50));
        assert!(out.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn large_jpeg_is_downscaled_and_stays_jpeg() {
        let data = r#gen(150, 300, ImageFormat::Jpeg);
        let out = small_profile().fit_image(&data, "jpg").unwrap();
        assert_eq!(dims(&out), (50, 100));
        assert!(out.starts_with(&[0xff, 0xd8]));
    }

    #[test]
    fn small_images_untouched() {
        let data = r#gen(100, 100, ImageFormat::Png);
        assert!(small_profile().fit_image(&data, "png").is_none());
        assert!(small_profile().fit_image(b"garbage", "png").is_none());
        let gif = b"GIF89a....".to_vec();
        assert!(small_profile().fit_image(&gif, "gif").is_none());
    }

    #[test]
    fn formats_list_limits_what_is_touched() {
        let mut p = small_profile();
        p.formats = vec!["jpeg".into()];
        let data = r#gen(400, 200, ImageFormat::Png);
        assert!(p.fit_image(&data, "png").is_none());
        p.max_width = 0;
        assert!(
            p.fit_image(&r#gen(400, 200, ImageFormat::Jpeg), "jpg")
                .is_none()
        );
    }

    #[test]
    fn builtin_profiles_are_distinct_and_sane() {
        let mut names = vec![];
        for k in ProfileKind::ALL {
            let p = DeviceProfile::for_kind(k);
            assert_eq!(p.name, k.id());
            assert!(p.max_width > 300 && p.max_height > p.max_width);
            assert!(p.jpeg_quality > 0 && p.jpeg_quality <= 100);
            assert_eq!(ProfileKind::from_id(k.id()), Some(k));
            names.push(p.name);
        }
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 4);
        assert_eq!(ProfileKind::from_id("nope"), None);
        assert_eq!(DeviceProfile::default().name, "generic_eink");
        assert!(DeviceProfile::tablet().max_width > DeviceProfile::kindle().max_width);
    }

    #[test]
    fn css_contains_margins_fonts_and_image_limit() {
        let css = small_profile().css();
        assert!(css.contains("margin: 0 1em;"));
        assert!(css.contains("font-family: Georgia, serif;"));
        assert!(css.contains("img { max-width: 100%"));
        let mut p = small_profile();
        p.margin.clear();
        p.fonts.clear();
        assert!(!p.css().contains("body {"));
    }

    fn book_with_images() -> OpenBook {
        let mut b = book_with(&[("A", "<p>x</p>"), ("B", "<p>y</p>")]);
        b.set_cover(r#gen(300, 500, ImageFormat::Jpeg), "jpg")
            .unwrap();
        let pkg = b.package.clone();
        b.archive
            .set("OEBPS/images/pic.png", r#gen(250, 120, ImageFormat::Png));
        pkg.add_manifest_item(
            &mut b.archive,
            "pic",
            "OEBPS/images/pic.png",
            "image/png",
            "",
        )
        .unwrap();
        b.reload_package().unwrap();
        b
    }

    #[test]
    fn transform_resizes_cover_and_inline_images_and_links_css() {
        let mut b = book_with_images();
        let p = small_profile();
        let r = p.apply(&mut b).unwrap();
        assert!(r.files_changed >= 4, "{r:?}");
        assert_eq!(dims(b.cover_bytes().unwrap()), (60, 100));
        assert_eq!(
            dims(b.archive.get("OEBPS/images/pic.png").unwrap()),
            (100, 48)
        );
        assert!(
            b.archive
                .get_string("OEBPS/bookx-device.css")
                .unwrap()
                .contains("Georgia")
        );
        assert!(b.package.item_by_path("OEBPS/bookx-device.css").is_some());
        assert!(
            chapter(&b, 0)
                .contains("<link rel=\"stylesheet\" type=\"text/css\" href=\"bookx-device.css\"/>")
        );
        assert!(chapter(&b, 1).contains("bookx-device.css"));
    }

    #[test]
    fn transform_is_idempotent() {
        let mut b = book_with_images();
        let p = small_profile();
        p.apply(&mut b).unwrap();
        let snap = snapshot(&b);
        let r = p.apply(&mut b).unwrap();
        assert_eq!(r.files_changed, 0, "{r:?}");
        assert_eq!(snapshot(&b), snap);
        assert_eq!(p.id(), "device_profile");
    }

    #[test]
    fn switching_profile_updates_the_same_stylesheet() {
        let mut b = book_with_images();
        small_profile().apply(&mut b).unwrap();
        DeviceProfile::tablet().apply(&mut b).unwrap();
        let css = b.archive.get_string("OEBPS/bookx-device.css").unwrap();
        assert!(css.contains("profile: tablet"));
        assert_eq!(chapter(&b, 0).matches("bookx-device.css").count(), 1);
        assert_eq!(
            b.package
                .manifest
                .iter()
                .filter(|i| i.href.contains("bookx-device"))
                .count(),
            1
        );
    }

    #[test]
    fn profile_json_roundtrip() {
        let p = DeviceProfile::kobo();
        let j = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<DeviceProfile>(&j).unwrap(), p);
        let k: ProfileKind = serde_json::from_str("\"generic_eink\"").unwrap();
        assert_eq!(k, ProfileKind::GenericEink);
    }
}
