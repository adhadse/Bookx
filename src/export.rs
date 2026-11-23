//! Converting library books to other e-book formats (roadmap C5/C6/C9), on top of `bookx-convert`.
//! No GTK in here, so it is unit tested; the library view only adds dialogs around it.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, anyhow};
use bookx_convert::{ConvertOptions, builtin_presets, convert};
use bookx_formats::default_readers;

/// An output format offered in the *Convert to* menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    /// Name in the menu.
    pub label: &'static str,
    /// Value passed to the converter (`"kepub"`, `"mobi"`, …).
    pub id: &'static str,
    /// Extension of the file that is written, without the dot.
    pub extension: &'static str,
    /// Built-in preset whose options are used (device clean-up for Kindles).
    pub preset: Option<&'static str>,
}

pub const TARGETS: &[Target] = &[
    Target {
        label: "EPUB (cleaned up)",
        id: "epub",
        extension: "epub",
        preset: None,
    },
    Target {
        label: "KEPUB (Kobo)",
        id: "kepub",
        extension: "kepub.epub",
        preset: None,
    },
    Target {
        label: "AZW3 (Kindle)",
        id: "azw3",
        extension: "azw3",
        preset: Some("Kindle (AZW3)"),
    },
    Target {
        label: "MOBI (older Kindles)",
        id: "mobi",
        extension: "mobi",
        preset: Some("Kindle (MOBI)"),
    },
    Target {
        label: "FictionBook (FB2)",
        id: "fb2",
        extension: "fb2",
        preset: None,
    },
    Target {
        label: "HTML archive (HTMLZ)",
        id: "htmlz",
        extension: "htmlz",
        preset: None,
    },
    Target {
        label: "Text archive (TXTZ)",
        id: "txtz",
        extension: "txtz",
        preset: None,
    },
    Target {
        label: "Word document (DOCX)",
        id: "docx",
        extension: "docx",
        preset: None,
    },
    Target {
        label: "Rich Text (RTF)",
        id: "rtf",
        extension: "rtf",
        preset: None,
    },
    Target {
        label: "eReader (PDB)",
        id: "pdb",
        extension: "pdb",
        preset: None,
    },
    Target {
        label: "PML archive (PMLZ)",
        id: "pmlz",
        extension: "pmlz",
        preset: None,
    },
    Target {
        label: "Rocket eBook (RB)",
        id: "rb",
        extension: "rb",
        preset: None,
    },
    Target {
        label: "Shanda Bambook (SNB)",
        id: "snb",
        extension: "snb",
        preset: None,
    },
    Target {
        label: "OEB package",
        id: "oeb",
        extension: "oeb",
        preset: None,
    },
    Target {
        label: "HTML in a ZIP",
        id: "zip",
        extension: "zip",
        preset: None,
    },
];

pub fn target(id: &str) -> Option<&'static Target> {
    TARGETS
        .iter()
        .find(|t| t.id == id)
        .or_else(|| crate::plugins::target(id))
}

/// The built-in options of a target: its preset when it has one, otherwise the defaults.
pub fn builtin_options(t: &Target) -> Result<ConvertOptions> {
    match t.preset {
        None => Ok(ConvertOptions::default()),
        Some(name) => builtin_presets()
            .presets
            .into_iter()
            .find(|p| p.name == name)
            .map(|p| p.options)
            .ok_or_else(|| anyhow!("the built-in preset `{name}` is missing")),
    }
}

/// `user` (what the options dialog saved for the target) when there is something, else the
/// built-in options.
pub fn options_with(t: &Target, user: Option<ConvertOptions>) -> Result<ConvertOptions> {
    match user {
        Some(o) => Ok(o),
        None => builtin_options(t),
    }
}

/// Options for a target: the ones saved in *Conversion Options*, otherwise the built-in ones.
/// (The tests never read the user's file.)
pub fn options_for(t: &Target) -> Result<ConvertOptions> {
    #[cfg(not(test))]
    let user = crate::convert_prefs::user_options(t.id);
    #[cfg(test)]
    let user = None;
    options_with(t, user)
}

/// `"Title.ext"` with characters that are not safe in file names removed.
pub fn file_name(title: &str, t: &Target) -> String {
    let clean: String = title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0'..='\u{1f}' => '_',
            c => c,
        })
        .collect();
    let clean = clean.trim().trim_matches('.').trim();
    let clean = if clean.is_empty() { "book" } else { clean };
    let clean: String = clean.chars().take(100).collect();
    format!("{clean}.{}", t.extension)
}

/// Converts the EPUB at `path` to `t`. Returns the bytes of the new file.
pub fn convert_epub_file(path: &Path, t: &Target) -> Result<Vec<u8>> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("book.epub");
    convert_epub_bytes(&bytes, name, t)
}

/// Converts EPUB `bytes` (called `name`, for messages and format detection) to `t`.
pub fn convert_epub_bytes(bytes: &[u8], name: &str, t: &Target) -> Result<Vec<u8>> {
    crate::trace::traced(crate::trace::Area::Convert, "epub_convert", |o| {
        o.fmt("epub").word("to", t.id).size_class("size", bytes.len() as u64);
    if let Some(done) = crate::plugins::convert_hooked(bytes, name, t) {
        return done;
    }
    let options = options_for(t)?;
    let readers = default_readers();
    let cancel = AtomicBool::new(false);
    let out = convert(&readers, bytes, name, t.id, &options, &|_| {}, &cancel)
        .with_context(|| format!("converting `{name}` to {}", t.label))?;
    Ok(out.bytes)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::convert::{Chapter, create_epub};
    use crate::core::opf::Metadata;
    use std::io::Read;

    fn sample_epub(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "bookx-export-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&d).unwrap();
        let md = Metadata {
            title: "Export Me".into(),
            authors: vec!["Ann Author".into()],
            language: "en".into(),
            ..Default::default()
        };
        let chapters = vec![
            Chapter {
                title: "One".into(),
                body: "<h1>One</h1><p>First chapter has words.</p>".into(),
            },
            Chapter {
                title: "Two".into(),
                body: "<h1>Two</h1><p>Second chapter too.</p>".into(),
            },
        ];
        let p = d.join("book.epub");
        std::fs::write(&p, create_epub(&md, &chapters, None).to_bytes().unwrap()).unwrap();
        p
    }

    fn zip_text(bytes: &[u8], name_part: &str) -> String {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
        for i in 0..z.len() {
            let mut f = z.by_index(i).unwrap();
            if f.name().unwrap_or_default().contains(name_part) {
                let mut s = String::new();
                f.read_to_string(&mut s).unwrap();
                return s;
            }
        }
        panic!("no entry containing {name_part}");
    }

    #[test]
    fn every_target_has_a_unique_id_and_a_known_converter() {
        let mut ids: Vec<_> = TARGETS.iter().map(|t| t.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), TARGETS.len());
        for t in TARGETS {
            assert!(options_for(t).is_ok(), "{}", t.id);
            assert!(!t.extension.is_empty() && !t.label.is_empty());
        }
        assert!(target("kepub").is_some());
        assert!(target("pdf").is_none());
    }

    #[test]
    fn file_names_are_safe() {
        let k = target("kepub").unwrap();
        assert_eq!(file_name("A Title", k), "A Title.kepub.epub");
        assert_eq!(file_name("a/b:c*d?", k), "a_b_c_d_.kepub.epub");
        assert_eq!(file_name("  ", k), "book.kepub.epub");
        assert_eq!(file_name("..", k), "book.kepub.epub");
        assert_eq!(
            file_name("日本語の本", target("fb2").unwrap()),
            "日本語の本.fb2"
        );
        assert!(file_name(&"x".repeat(500), k).chars().count() < 120);
        assert!(!file_name("tab\there", k).contains('\t'));
    }

    #[test]
    fn kindle_targets_use_their_presets() {
        let o = options_for(target("mobi").unwrap()).unwrap();
        assert!(o.device.is_some());
        let d = options_for(target("fb2").unwrap()).unwrap();
        assert!(d.device.is_none());
    }

    #[test]
    fn saved_options_replace_the_built_in_ones_of_their_format_only() {
        let kindle = target("azw3").unwrap();
        let builtin = builtin_options(kindle).unwrap();
        assert_eq!(options_with(kindle, None).unwrap(), builtin);
        let mut mine = builtin.clone();
        mine.smarten_punctuation = !builtin.smarten_punctuation;
        assert_eq!(options_with(kindle, Some(mine.clone())).unwrap(), mine);
        assert_eq!(
            options_for(kindle).unwrap(),
            builtin,
            "tests ignore the user's file"
        );
    }

    #[test]
    fn converts_to_kepub_with_kobo_spans() {
        let p = sample_epub("kepub");
        let out = convert_epub_file(&p, target("kepub").unwrap()).unwrap();
        assert!(out.starts_with(b"PK"));
        assert!(zip_text(&out, "chapter-1").contains("koboSpan"));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn converts_to_mobi_and_azw3() {
        let p = sample_epub("kindle");
        for id in ["mobi", "azw3"] {
            let out = convert_epub_file(&p, target(id).unwrap()).unwrap();
            assert_eq!(&out[60..68], b"BOOKMOBI", "{id}");
            // the title is stored in the file
            assert!(
                out.windows(9).any(|w| w == b"Export Me"),
                "{id}: title missing"
            );
        }
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn converts_to_fb2_htmlz_txtz_and_epub() {
        let p = sample_epub("flat");
        let fb2 = convert_epub_file(&p, target("fb2").unwrap()).unwrap();
        let text = String::from_utf8(fb2).unwrap();
        assert!(text.contains("<FictionBook") && text.contains("Export Me"));
        let htmlz = convert_epub_file(&p, target("htmlz").unwrap()).unwrap();
        assert!(zip_text(&htmlz, "index.html").contains("First chapter has words."));
        let txtz = convert_epub_file(&p, target("txtz").unwrap()).unwrap();
        assert!(zip_text(&txtz, "index.txt").contains("Second chapter too."));
        let epub = convert_epub_file(&p, target("epub").unwrap()).unwrap();
        assert!(
            crate::core::book::OpenBook::from_archive(
                crate::core::archive::Archive::from_bytes(epub).unwrap()
            )
            .is_ok()
        );
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn converts_to_the_remaining_calibre_formats() {
        let p = sample_epub("more");
        let magic: &[(&str, &[u8], usize)] = &[
            ("docx", b"PK", 0),
            ("rtf", b"{\\rtf1", 0),
            ("zip", b"PK", 0),
            ("oeb", b"PK", 0),
            ("pmlz", b"PK", 0),
            ("snb", b"SNBP000B", 0),
            ("pdb", b"PNRdPPrs", 60),
        ];
        for (id, head, at) in magic {
            let out = convert_epub_file(&p, target(id).unwrap()).unwrap();
            assert!(out[*at..].starts_with(head), "{id}: {:?}", &out[..16]);
        }
        let rb = convert_epub_file(&p, target("rb").unwrap()).unwrap();
        assert_eq!(&rb[..4], &[0xb0, 0x0c, 0xb0, 0x0c]);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn the_new_formats_round_trip_their_text() {
        let p = sample_epub("trip");
        for id in ["docx", "rtf", "pdb", "pmlz", "rb", "snb"] {
            let t = target(id).unwrap();
            let out = convert_epub_file(&p, t).unwrap();
            let a = read_back(&file_name("Export Me", t), &out);
            assert!(a.contains("First chapter has words."), "{id}: {a}");
            assert!(a.contains("Second chapter too."), "{id}");
        }
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn zip_holds_the_epub_layout() {
        let p = sample_epub("ziplayout");
        let out = convert_epub_file(&p, target("zip").unwrap()).unwrap();
        let book = crate::core::book::OpenBook::from_archive(
            crate::core::archive::Archive::from_bytes(out).unwrap(),
        )
        .unwrap();
        let text: String = book
            .spine_paths()
            .iter()
            .filter_map(|p| book.chapter_source(p))
            .collect();
        assert!(text.contains("First chapter has words."));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    fn read_back(name: &str, bytes: &[u8]) -> String {
        let a = bookx_formats::read_any(name, bytes).unwrap();
        let book = crate::core::book::OpenBook::from_archive(a).unwrap();
        book.spine_paths()
            .iter()
            .filter_map(|p| book.chapter_source(p))
            .collect()
    }

    #[test]
    fn conversion_errors_name_the_book_and_the_target() {
        let d = std::env::temp_dir().join(format!("bookx-export-bad-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        let bad = d.join("broken.epub");
        std::fs::write(&bad, b"not a zip").unwrap();
        let err = convert_epub_file(&bad, target("mobi").unwrap()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("broken.epub") && msg.contains("MOBI"), "{msg}");
        let missing = convert_epub_file(&d.join("none.epub"), target("fb2").unwrap()).unwrap_err();
        assert!(format!("{missing:#}").contains("none.epub"));
        let _ = std::fs::remove_dir_all(d);
    }
}
