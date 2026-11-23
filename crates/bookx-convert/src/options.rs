//! Conversion options and presets (C20, C21).

use std::path::Path;

use anyhow::{Context, Result};
use bookx_core::contracts::{Registry, Transform};
use serde::{Deserialize, Serialize};

use crate::transforms::{
    AutoToc, AutoTocMode, DeviceProfile, MetadataOverride, ProfileKind, RemoveBlankParagraphs,
    RemoveFakeMargins, RemoveUnusedCss, ReplaceSceneBreaks, SetMetadataOverride,
    SmartenPunctuation, SplitLargeHtml, UnsmartenPunctuation, UnwrapLines,
};
use crate::writers::{DocxOptions, EpubVersion, PdbCompression};

/// Everything the user can tune for one conversion. All fields have defaults, so presets saved by
/// older versions keep loading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConvertOptions {
    /// EPUB 2 or 3 for EPUB and KEPUB output (`None` = the writer's default).
    pub epub_version: Option<EpubVersion>,
    pub device: Option<ProfileKind>,
    pub smarten_punctuation: bool,
    pub unsmarten_punctuation: bool,
    /// Line-unwrap threshold (see [`UnwrapLines`]); `None` = off.
    pub unwrap_lines: Option<f64>,
    pub remove_unused_css: bool,
    pub remove_fake_margins: bool,
    pub remove_blank_paragraphs: bool,
    /// XHTML that replaces soft scene breaks; `None` = leave them.
    pub replace_scene_breaks: Option<String>,
    pub metadata: MetadataOverride,
    /// Split XHTML files larger than this many bytes.
    pub split_large_html: Option<usize>,
    pub auto_toc: Option<AutoTocMode>,
    pub toc_max_level: u8,
    /// MOBI: append an inline table of contents page.
    pub mobi_inline_toc: bool,
    /// DOCX output: page size, margins, font, contents page.
    pub docx: DocxOptions,
    /// PDB (eReader) output: text record compression.
    pub pdb_compression: PdbCompression,
    /// Worker threads for per-file transforms (`0` = all cores).
    pub jobs: usize,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        ConvertOptions {
            epub_version: None,
            device: None,
            smarten_punctuation: false,
            unsmarten_punctuation: false,
            unwrap_lines: None,
            remove_unused_css: false,
            remove_fake_margins: false,
            remove_blank_paragraphs: false,
            replace_scene_breaks: None,
            metadata: MetadataOverride::default(),
            split_large_html: None,
            auto_toc: None,
            toc_max_level: 3,
            mobi_inline_toc: true,
            docx: DocxOptions::default(),
            pdb_compression: PdbCompression::default(),
            jobs: 0,
        }
    }
}

impl ConvertOptions {
    /// The transforms these options select, in the order they must run.
    pub fn transforms(&self) -> Registry<dyn Transform> {
        let mut r: Registry<dyn Transform> = Registry::new();
        if !self.metadata.is_empty() {
            r.register(Box::new(SetMetadataOverride::new(self.metadata.clone())));
        }
        if let Some(t) = self.unwrap_lines {
            r.register(Box::new(UnwrapLines {
                threshold: t,
                jobs: self.jobs,
            }));
        }
        if self.remove_blank_paragraphs {
            r.register(Box::new(RemoveBlankParagraphs { jobs: self.jobs }));
        }
        if self.remove_fake_margins {
            r.register(Box::new(RemoveFakeMargins));
        }
        if let Some(rep) = &self.replace_scene_breaks {
            r.register(Box::new(ReplaceSceneBreaks {
                replacement: rep.clone(),
                jobs: self.jobs,
            }));
        }
        if self.unsmarten_punctuation {
            r.register(Box::new(UnsmartenPunctuation { jobs: self.jobs }));
        }
        if self.smarten_punctuation {
            r.register(Box::new(SmartenPunctuation { jobs: self.jobs }));
        }
        if self.remove_unused_css {
            r.register(Box::new(RemoveUnusedCss));
        }
        if let Some(kind) = self.device {
            r.register(Box::new(DeviceProfile::for_kind(kind)));
        }
        if let Some(max) = self.split_large_html {
            r.register(Box::new(SplitLargeHtml { max_bytes: max }));
        }
        if let Some(mode) = self.auto_toc {
            r.register(Box::new(AutoToc {
                mode,
                max_level: self.toc_max_level.clamp(1, 3),
            }));
        }
        r
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s).context("invalid conversion options")
    }
}

/// A named set of options, optionally tied to an input format (`"txt"`, `"epub"`, …).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    #[serde(default)]
    pub input_format: Option<String>,
    #[serde(default)]
    pub output_format: Option<String>,
    pub options: ConvertOptions,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PresetStore {
    pub presets: Vec<Preset>,
}

impl PresetStore {
    /// Inserts or replaces the preset with the same name and input format.
    pub fn put(&mut self, preset: Preset) {
        match self
            .presets
            .iter_mut()
            .find(|p| p.name == preset.name && p.input_format == preset.input_format)
        {
            Some(slot) => *slot = preset,
            None => self.presets.push(preset),
        }
    }

    pub fn remove(&mut self, name: &str, input_format: Option<&str>) -> bool {
        let before = self.presets.len();
        self.presets
            .retain(|p| !(p.name == name && p.input_format.as_deref() == input_format));
        self.presets.len() != before
    }

    /// The preset called `name` for `input_format`; falls back to the one without input format.
    pub fn find(&self, name: &str, input_format: &str) -> Option<&Preset> {
        let input = input_format.to_ascii_lowercase();
        self.presets
            .iter()
            .find(|p| p.name == name && p.input_format.as_deref() == Some(input.as_str()))
            .or_else(|| {
                self.presets
                    .iter()
                    .find(|p| p.name == name && p.input_format.is_none())
            })
    }

    pub fn names(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.presets.iter().map(|p| p.name.as_str()).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s).context("invalid preset file")
    }

    /// A missing file is an empty store; a corrupt one is an error.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::from_json(&s).with_context(|| format!("reading {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Atomic save (temporary file + rename).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.bookx-tmp");
        std::fs::write(&tmp, self.to_json()?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// Presets shipped with Bookx.
pub fn builtin_presets() -> PresetStore {
    let mut s = PresetStore::default();
    s.put(Preset {
        name: "Kindle (MOBI)".into(),
        input_format: None,
        output_format: Some("mobi".into()),
        options: ConvertOptions {
            device: Some(ProfileKind::Kindle),
            smarten_punctuation: true,
            remove_unused_css: true,
            split_large_html: Some(260_000),
            auto_toc: Some(AutoTocMode::IfMissing),
            ..Default::default()
        },
    });
    s.put(Preset {
        name: "Kindle (AZW3)".into(),
        input_format: None,
        output_format: Some("azw3".into()),
        options: ConvertOptions {
            device: Some(ProfileKind::Kindle),
            remove_unused_css: true,
            split_large_html: Some(260_000),
            auto_toc: Some(AutoTocMode::IfMissing),
            ..Default::default()
        },
    });
    s.put(Preset {
        name: "Kobo (KEPUB)".into(),
        input_format: None,
        output_format: Some("kepub".into()),
        options: ConvertOptions {
            device: Some(ProfileKind::Kobo),
            remove_unused_css: true,
            auto_toc: Some(AutoTocMode::IfMissing),
            ..Default::default()
        },
    });
    s.put(Preset {
        name: "Generic e-ink (EPUB 2)".into(),
        input_format: None,
        output_format: Some("epub".into()),
        options: ConvertOptions {
            epub_version: Some(EpubVersion::V2),
            device: Some(ProfileKind::GenericEink),
            remove_unused_css: true,
            split_large_html: Some(200_000),
            auto_toc: Some(AutoTocMode::IfMissing),
            ..Default::default()
        },
    });
    s.put(Preset {
        name: "Plain text to book".into(),
        input_format: Some("txt".into()),
        output_format: Some("epub".into()),
        options: ConvertOptions {
            unwrap_lines: Some(0.7),
            remove_blank_paragraphs: true,
            replace_scene_breaks: Some("<hr class=\"scene-break\"/>".into()),
            smarten_punctuation: true,
            auto_toc: Some(AutoTocMode::IfMissing),
            ..Default::default()
        },
    });
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "bookx-opts-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn defaults_select_no_transforms() {
        assert!(ConvertOptions::default().transforms().is_empty());
    }

    #[test]
    fn every_flag_adds_its_transform_in_order() {
        let o = ConvertOptions {
            metadata: MetadataOverride {
                title: Some("T".into()),
                ..Default::default()
            },
            unwrap_lines: Some(0.5),
            remove_blank_paragraphs: true,
            remove_fake_margins: true,
            replace_scene_breaks: Some("<hr/>".into()),
            unsmarten_punctuation: true,
            smarten_punctuation: true,
            remove_unused_css: true,
            device: Some(ProfileKind::Kobo),
            split_large_html: Some(1000),
            auto_toc: Some(AutoTocMode::Force),
            ..Default::default()
        };
        let ids: Vec<&str> = o.transforms().iter().map(|t| t.id()).collect();
        assert_eq!(
            ids,
            vec![
                "set_metadata_override",
                "unwrap_lines",
                "remove_blank_paragraphs",
                "remove_fake_margins",
                "replace_scene_breaks",
                "unsmarten_punctuation",
                "smarten_punctuation",
                "remove_unused_css",
                "device_profile",
                "split_large_html",
                "auto_toc"
            ]
        );
    }

    #[test]
    fn options_json_roundtrip_and_partial_files() {
        let o = ConvertOptions {
            epub_version: Some(EpubVersion::V2),
            device: Some(ProfileKind::Kindle),
            unwrap_lines: Some(0.65),
            metadata: MetadataOverride {
                cover: Some(vec![9, 8, 7]),
                tags: Some(vec!["x".into()]),
                ..Default::default()
            },
            ..Default::default()
        };
        let j = o.to_json().unwrap();
        assert_eq!(ConvertOptions::from_json(&j).unwrap(), o);
        // an old preset with only one key still loads
        let partial = ConvertOptions::from_json("{\"smarten_punctuation\": true}").unwrap();
        assert!(partial.smarten_punctuation);
        assert!(partial.mobi_inline_toc);
        assert!(ConvertOptions::from_json("{\"jobs\": \"many\"}").is_err());
        assert!(
            ConvertOptions::from_json("nope")
                .unwrap_err()
                .to_string()
                .contains("invalid conversion options")
        );
    }

    #[test]
    fn store_put_find_remove() {
        let mut s = PresetStore::default();
        let base = Preset {
            name: "mine".into(),
            input_format: None,
            output_format: None,
            options: ConvertOptions::default(),
        };
        s.put(base.clone());
        let mut txt = base.clone();
        txt.input_format = Some("txt".into());
        txt.options.smarten_punctuation = true;
        s.put(txt.clone());
        assert_eq!(s.presets.len(), 2);
        assert!(s.find("mine", "TXT").unwrap().options.smarten_punctuation);
        assert!(!s.find("mine", "epub").unwrap().options.smarten_punctuation);
        assert!(s.find("other", "txt").is_none());
        // replace instead of duplicate
        txt.options.smarten_punctuation = false;
        s.put(txt);
        assert_eq!(s.presets.len(), 2);
        assert_eq!(s.names(), vec!["mine"]);
        assert!(s.remove("mine", Some("txt")));
        assert!(!s.remove("mine", Some("txt")));
        assert_eq!(s.presets.len(), 1);
    }

    #[test]
    fn store_saves_and_loads_atomically() {
        let dir = tmp("store");
        let path = dir.join("nested/presets.json");
        assert_eq!(
            PresetStore::load(&path).unwrap(),
            PresetStore::default(),
            "missing file = empty"
        );
        let s = builtin_presets();
        s.save(&path).unwrap();
        assert_eq!(PresetStore::load(&path).unwrap(), s);
        assert!(!path.with_extension("json.bookx-tmp").exists());
        std::fs::write(&path, "{ not json").unwrap();
        let e = PresetStore::load(&path).unwrap_err();
        assert!(format!("{e:#}").contains("invalid preset file"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn builtin_presets_are_usable() {
        let s = builtin_presets();
        assert!(s.names().len() >= 4);
        let kindle = s.find("Kindle (MOBI)", "epub").unwrap();
        assert_eq!(kindle.output_format.as_deref(), Some("mobi"));
        assert!(!kindle.options.transforms().is_empty());
        let txt = s.find("Plain text to book", "txt").unwrap();
        assert!(txt.options.unwrap_lines.is_some());
        assert!(
            s.find("Plain text to book", "epub").is_none(),
            "tied to txt input"
        );
    }
}
