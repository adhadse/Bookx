//! Metadata override applied during conversion (C15).

use anyhow::{Result, bail};
use base64::Engine;
use bookx_core::book::OpenBook;
use bookx_core::contracts::{Transform, TransformReport};
use serde::{Deserialize, Serialize};

mod b64 {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(b) => s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(b)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        let s: Option<String> = Option::deserialize(d)?;
        match s {
            Some(s) => base64::engine::general_purpose::STANDARD
                .decode(s.as_bytes())
                .map(Some)
                .map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}

/// Fields that replace the book's own metadata. `None` leaves a field alone.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MetadataOverride {
    pub title: Option<String>,
    pub authors: Option<Vec<String>>,
    pub series: Option<String>,
    pub series_index: Option<f64>,
    pub tags: Option<Vec<String>>,
    pub language: Option<String>,
    pub publisher: Option<String>,
    pub description: Option<String>,
    /// JPEG, PNG or GIF bytes (base64 in JSON presets).
    #[serde(with = "b64")]
    pub cover: Option<Vec<u8>>,
}

impl MetadataOverride {
    pub fn is_empty(&self) -> bool {
        *self == MetadataOverride::default()
    }
}

/// File extension for image bytes, from the magic number.
pub fn sniff_image_ext(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("png")
    } else if data.starts_with(b"GIF8") {
        Some("gif")
    } else {
        None
    }
}

#[derive(Debug, Clone, Default)]
pub struct SetMetadataOverride {
    pub over: MetadataOverride,
}

impl SetMetadataOverride {
    pub fn new(over: MetadataOverride) -> Self {
        SetMetadataOverride { over }
    }
}

impl Transform for SetMetadataOverride {
    fn id(&self) -> &'static str {
        "set_metadata_override"
    }

    fn apply(&self, book: &mut OpenBook) -> Result<TransformReport> {
        let o = &self.over;
        let mut report = TransformReport::default();
        let mut md = book.metadata().clone();
        if let Some(v) = &o.title {
            md.title = v.clone();
            md.title_sort = bookx_core::util::title_sort(v);
        }
        if let Some(v) = &o.authors {
            md.authors = v.clone();
            md.author_sort = v
                .iter()
                .map(|a| bookx_core::util::author_sort(a))
                .collect::<Vec<_>>()
                .join(" & ");
        }
        if let Some(v) = &o.series {
            md.series = v.clone();
        }
        if let Some(v) = o.series_index {
            md.series_index = v;
        }
        if let Some(v) = &o.tags {
            md.tags = v.clone();
        }
        if let Some(v) = &o.language {
            md.language = v.clone();
        }
        if let Some(v) = &o.publisher {
            md.publisher = v.clone();
        }
        if let Some(v) = &o.description {
            md.description = v.clone();
        }
        if md != *book.metadata() {
            book.set_metadata(md)?;
            report.files_changed += 1;
            report.notes.push("metadata updated".into());
        }
        if let Some(cover) = &o.cover {
            let Some(ext) = sniff_image_ext(cover) else {
                bail!("the replacement cover is not a JPEG, PNG or GIF image");
            };
            if book.cover_bytes() != Some(cover.as_slice()) {
                book.set_cover(cover.clone(), ext)?;
                report.files_changed += 1;
                report.notes.push("cover replaced".into());
            }
        }
        Ok(report)
    }
}

/// Encodes cover bytes the way presets store them.
pub fn cover_to_base64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;

    fn png() -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(4, 4)
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        buf.into_inner()
    }

    #[test]
    fn overrides_all_fields() {
        let mut b = book_with(&[("A", "<p>x</p>")]);
        let o = MetadataOverride {
            title: Some("New Title".into()),
            authors: Some(vec!["Grace Hopper".into(), "Ada Lovelace".into()]),
            series: Some("Saga".into()),
            series_index: Some(2.5),
            tags: Some(vec!["a".into(), "b".into()]),
            language: Some("fr".into()),
            publisher: Some("Pub & Co".into()),
            description: Some("<b>desc</b> & more".into()),
            cover: Some(png()),
        };
        let r = SetMetadataOverride::new(o.clone()).apply(&mut b).unwrap();
        assert_eq!(r.files_changed, 2);
        // read back from a fresh parse of the bytes
        let again = OpenBook::from_archive(
            bookx_core::archive::Archive::from_bytes(b.archive.to_bytes().unwrap()).unwrap(),
        )
        .unwrap();
        let m = again.metadata();
        assert_eq!(m.title, "New Title");
        assert_eq!(m.authors, o.authors.clone().unwrap());
        assert_eq!(m.series, "Saga");
        assert_eq!(m.series_index, 2.5);
        assert_eq!(m.tags, o.tags.clone().unwrap());
        assert_eq!(m.language, "fr");
        assert_eq!(m.publisher, "Pub & Co");
        assert_eq!(m.description, "<b>desc</b> & more");
        assert_eq!(again.cover_bytes(), o.cover.as_deref());
    }

    #[test]
    fn unset_fields_are_left_alone() {
        let mut b = book_with(&[("A", "<p>x</p>")]);
        let before = b.metadata().clone();
        let o = MetadataOverride {
            publisher: Some("P".into()),
            ..Default::default()
        };
        SetMetadataOverride::new(o).apply(&mut b).unwrap();
        assert_eq!(b.metadata().title, before.title);
        assert_eq!(b.metadata().authors, before.authors);
        assert_eq!(b.metadata().publisher, "P");
    }

    #[test]
    fn idempotent() {
        let mut b = book_with(&[("A", "<p>x</p>")]);
        let t = SetMetadataOverride::new(MetadataOverride {
            title: Some("T".into()),
            tags: Some(vec!["x".into()]),
            cover: Some(png()),
            ..Default::default()
        });
        assert_eq!(t.apply(&mut b).unwrap().files_changed, 2);
        let snap = snapshot(&b);
        assert_eq!(t.apply(&mut b).unwrap().files_changed, 0);
        assert_eq!(snapshot(&b), snap);
        assert_eq!(t.id(), "set_metadata_override");
    }

    #[test]
    fn empty_override_is_noop() {
        let mut b = book_with(&[("A", "<p>x</p>")]);
        let snap = snapshot(&b);
        let o = MetadataOverride::default();
        assert!(o.is_empty());
        assert_eq!(
            SetMetadataOverride::new(o)
                .apply(&mut b)
                .unwrap()
                .files_changed,
            0
        );
        assert_eq!(snapshot(&b), snap);
    }

    #[test]
    fn bad_cover_is_an_error_with_message() {
        let mut b = book_with(&[("A", "<p>x</p>")]);
        let t = SetMetadataOverride::new(MetadataOverride {
            cover: Some(b"not an image".to_vec()),
            ..Default::default()
        });
        let e = t.apply(&mut b).unwrap_err();
        assert!(e.to_string().contains("not a JPEG, PNG or GIF"));
    }

    #[test]
    fn jpeg_cover_extension() {
        let mut b = book_with(&[("A", "<p>x</p>")]);
        let mut jpg = vec![0xff, 0xd8, 0xff, 0xe0];
        jpg.extend_from_slice(&[0; 16]);
        SetMetadataOverride::new(MetadataOverride {
            cover: Some(jpg.clone()),
            ..Default::default()
        })
        .apply(&mut b)
        .unwrap();
        assert_eq!(b.cover_bytes(), Some(jpg.as_slice()));
        assert!(b.package.cover_path().unwrap().ends_with(".jpg"));
    }

    #[test]
    fn sniffing() {
        assert_eq!(sniff_image_ext(&[0xff, 0xd8, 0xff, 0]), Some("jpg"));
        assert_eq!(sniff_image_ext(&png()), Some("png"));
        assert_eq!(sniff_image_ext(b"GIF89a"), Some("gif"));
        assert_eq!(sniff_image_ext(b"xx"), None);
    }

    #[test]
    fn json_roundtrip_with_cover() {
        let o = MetadataOverride {
            title: Some("T".into()),
            cover: Some(vec![1, 2, 3, 255]),
            ..Default::default()
        };
        let j = serde_json::to_string(&o).unwrap();
        assert!(j.contains(&cover_to_base64(&[1, 2, 3, 255])));
        let back: MetadataOverride = serde_json::from_str(&j).unwrap();
        assert_eq!(back, o);
        let none: MetadataOverride = serde_json::from_str("{}").unwrap();
        assert!(none.is_empty());
        assert!(serde_json::from_str::<MetadataOverride>("{\"cover\":\"!!\"}").is_err());
    }
}
