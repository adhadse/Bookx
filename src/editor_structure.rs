//! Glue of the structure tools of the editor (roadmap CE-028, CE-120, CE-129, CE-420, CE-421, CE-426, CE-427,
//! CE-429): operations that turn dialog values into [`Outcome`]s, saved HTML rule sets, and the open/save-back
//! logic for books that are not EPUB. GTK-free, unit tested here; the dialogs are `ui/editor/*`.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, bail};
use bookx_core::archive::Archive;
use bookx_core::book::OpenBook;
use bookx_editing::a11y_meta::{self, A11yForm};
use bookx_editing::cover::{self, CoverOptions};
use bookx_editing::fonts_manage;
use bookx_editing::jacket::{self, JacketOptions};
use bookx_editing::transform_html::{self, HtmlRule, HtmlRuleSet};

use crate::editor_tools::Outcome;
use crate::i18n::{ntr, trf};

fn outcome(changed: bool, summary: impl Into<String>, details: Vec<String>) -> Outcome {
    Outcome {
        changed,
        summary: summary.into(),
        details,
    }
}

// ---------------------------------------------------------------------------
// Transform HTML
// ---------------------------------------------------------------------------

/// Runs the rules; the details list the changed pages (and the pages that had to be skipped).
pub fn html_transform(
    book: &mut OpenBook,
    rules: &[HtmlRule],
    paths: &[String],
) -> Result<Outcome> {
    if rules.is_empty() {
        bail!("Add at least one rule");
    }
    let rep = transform_html::transform_book(book, rules, paths, false);
    let mut details: Vec<String> = rep
        .changed
        .iter()
        .map(|(f, n)| trf("{}: {} changes", &[&f, n]))
        .collect();
    details.extend(
        rep.skipped
            .iter()
            .map(|(f, why)| trf("{} was skipped: {}", &[&f, why])),
    );
    Ok(outcome(
        rep.total() > 0,
        if rep.total() == 0 {
            "No page matched the rules".to_string()
        } else {
            trf(
                "Changed {} in {}",
                &[
                    &ntr("{} element", "{} elements", rep.total()),
                    &ntr("{} page", "{} pages", rep.changed.len()),
                ],
            )
        },
        details,
    ))
}

pub fn html_rules_dir() -> Option<PathBuf> {
    crate::editor_state::config_dir().map(|d| d.join("editor-html-rules"))
}

fn rules_file_name(name: &str) -> String {
    let safe: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{}.json", safe.trim())
}

pub fn list_html_rule_sets_in(dir: &Path) -> Vec<HtmlRuleSet> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<HtmlRuleSet> = rd
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .filter(|e| e.metadata().map(|m| m.len() < 1_000_000).unwrap_or(false))
        .filter_map(|e| HtmlRuleSet::from_json(&std::fs::read_to_string(e.path()).ok()?))
        .filter(|s| s.rules.iter().all(|r| r.validate().is_ok()))
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn save_html_rule_set_in(dir: &Path, set: &HtmlRuleSet) -> std::io::Result<()> {
    if set.name.trim().is_empty() {
        return Err(std::io::Error::other("a rule set needs a name"));
    }
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(rules_file_name(&set.name)), set.to_json())
}

pub fn delete_html_rule_set_in(dir: &Path, name: &str) {
    let _ = std::fs::remove_file(dir.join(rules_file_name(name)));
}

// ---------------------------------------------------------------------------
// Cover, jacket, fonts, accessibility
// ---------------------------------------------------------------------------

pub fn set_cover(book: &mut OpenBook, opts: &CoverOptions) -> Result<Outcome> {
    let r = cover::set_cover(book, opts)?;
    let mut details = vec![trf("Cover image: {}", &[&r.image])];
    if let Some(p) = &r.page {
        details.push(trf("Cover page: {}", &[p]));
    }
    details.extend(
        r.removed
            .iter()
            .map(|f| trf("Removed the old cover {}", &[f])),
    );
    Ok(outcome(true, trf("Cover set to {}", &[&r.image]), details))
}

pub fn remove_cover(book: &mut OpenBook) -> Result<Outcome> {
    let had = book.package.cover_path().is_some();
    cover::remove_cover(book)?;
    Ok(outcome(
        had,
        if had {
            "Cover removed"
        } else {
            "The book has no cover"
        },
        Vec::new(),
    ))
}

pub fn jacket_add(book: &mut OpenBook, o: &JacketOptions) -> Result<Outcome> {
    let (p, new) = jacket::add_or_update(book, o)?;
    Ok(outcome(
        true,
        if new {
            trf("Added the book jacket {}", &[&p])
        } else {
            trf("Updated the book jacket {}", &[&p])
        },
        Vec::new(),
    ))
}

pub fn jacket_remove(book: &mut OpenBook) -> Result<Outcome> {
    Ok(match jacket::remove(book)? {
        Some(p) => outcome(true, trf("Removed the book jacket {}", &[&p]), Vec::new()),
        None => outcome(false, "The book has no jacket", Vec::new()),
    })
}

/// Refreshes the jacket with the options it was made with. `None` when there is no jacket or it is current.
pub fn jacket_refresh(book: &mut OpenBook) -> Result<Option<String>> {
    match jacket::options_of(book) {
        Some(o) => jacket::refresh(book, &o),
        None => Ok(None),
    }
}

pub fn fonts_rename(book: &mut OpenBook, from: &str, to: &str) -> Result<Outcome> {
    let n = fonts_manage::rename_family(book, from, to)?;
    Ok(outcome(
        n > 0,
        if n > 0 {
            trf("Renamed {} to {}", &[&from, &to.trim()])
        } else {
            trf("Nothing used the name {}", &[&from])
        },
        Vec::new(),
    ))
}

pub fn fonts_remove(book: &mut OpenBook, paths: &[String]) -> Result<Outcome> {
    let r = fonts_manage::remove_fonts(book, paths)?;
    Ok(outcome(
        !r.files.is_empty(),
        trf(
            "Removed {} and {}",
            &[
                &ntr("{} font", "{} fonts", r.files.len()),
                &ntr("{} rule", "{} rules", r.rules),
            ],
        ),
        r.files,
    ))
}

pub fn a11y_apply(book: &mut OpenBook, form: &A11yForm) -> Result<Outcome> {
    let changed = a11y_meta::write_form(book, form)?;
    Ok(outcome(
        changed,
        if changed {
            "Accessibility metadata saved"
        } else {
            "The accessibility metadata is already like that"
        },
        a11y_meta::check_claims(book, form),
    ))
}

// ---------------------------------------------------------------------------
// Books that are not EPUB (CE-429)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForeignFormat {
    pub id: &'static str,
    pub label: &'static str,
    pub extensions: &'static [&'static str],
    /// The converter id that writes the format back; `None` when Bookx cannot write it.
    pub writer: Option<&'static str>,
    /// What is likely lost in the round trip, in words for the user.
    pub warning: &'static str,
}

pub const FOREIGN_FORMATS: &[ForeignFormat] = &[
    ForeignFormat {
        id: "kepub",
        label: "KEPUB",
        extensions: &["kepub"],
        writer: None,
        warning: "A KEPUB is an EPUB with Kobo's reading markup. It is edited as it is, so the markup stays; save it back to the same file.",
    },
    ForeignFormat {
        id: "azw3",
        label: "AZW3",
        extensions: &["azw3", "azw"],
        writer: Some("azw3"),
        warning: "Converting from and back to AZW3 can change the page layout, fonts and some styles. Kindle-only features such as X-Ray and notes are not kept.",
    },
    ForeignFormat {
        id: "mobi",
        label: "MOBI",
        extensions: &["mobi", "prc"],
        writer: Some("mobi"),
        warning: "MOBI keeps little of the styling: CSS is reduced, and tables and floats may change.",
    },
    ForeignFormat {
        id: "fb2",
        label: "FictionBook (FB2)",
        extensions: &["fb2"],
        writer: Some("fb2"),
        warning: "FB2 has a limited set of elements; styles, classes and unusual markup are lost.",
    },
    ForeignFormat {
        id: "fbz",
        label: "FictionBook archive (FBZ)",
        extensions: &["fbz"],
        writer: None,
        warning: "Bookx cannot write FBZ; save the result as an EPUB.",
    },
    ForeignFormat {
        id: "docx",
        label: "Word (DOCX)",
        extensions: &["docx"],
        writer: Some("docx"),
        warning: "Word documents become reflowable pages; page layout, headers and footers, and comments are lost, and so is much of the formatting when converted back.",
    },
    ForeignFormat {
        id: "odt",
        label: "OpenDocument text (ODT)",
        extensions: &["odt"],
        writer: None,
        warning: "Bookx cannot write ODT; save the result as an EPUB.",
    },
    ForeignFormat {
        id: "rtf",
        label: "Rich Text (RTF)",
        extensions: &["rtf"],
        writer: Some("rtf"),
        warning: "RTF keeps basic text formatting only; styles, tables and images may change.",
    },
    ForeignFormat {
        id: "htmlz",
        label: "HTML archive (HTMLZ)",
        extensions: &["htmlz"],
        writer: Some("htmlz"),
        warning: "HTMLZ is one page with its images; the chapter split and the package metadata are rebuilt.",
    },
    ForeignFormat {
        id: "txtz",
        label: "Text archive (TXTZ)",
        extensions: &["txtz"],
        writer: Some("txtz"),
        warning: "Plain text keeps no styling: bold, italics, images and links are lost when converted back.",
    },
];

/// The format of a file Bookx can edit through a temporary EPUB, or `None` for EPUB and unknown files.
pub fn foreign_format(path: &Path) -> Option<&'static ForeignFormat> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if name.ends_with(".kepub.epub") {
        return FOREIGN_FORMATS.iter().find(|f| f.id == "kepub");
    }
    let ext = name.rsplit_once('.')?.1;
    FOREIGN_FORMATS.iter().find(|f| f.extensions.contains(&ext))
}

/// Where a book that is being edited came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub path: PathBuf,
    pub format: &'static ForeignFormat,
}

/// Largest file opened for editing through a conversion.
pub const MAX_FOREIGN_BYTES: u64 = 400 * 1024 * 1024;

/// Opens the file for editing: KEPUB as it is, everything else converted to an EPUB archive in memory.
pub fn open_foreign(path: &Path) -> Result<(Archive, Origin)> {
    let format = foreign_format(path).context("Bookx cannot edit that kind of file")?;
    let meta =
        std::fs::metadata(path).with_context(|| format!("cannot read {}", path.display()))?;
    if !meta.is_file() {
        bail!("{} is not a file", path.display());
    }
    if meta.len() > MAX_FOREIGN_BYTES {
        bail!(
            "the file is too large to edit ({} MB)",
            meta.len() / (1024 * 1024)
        );
    }
    let bytes = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("book");
    let archive = if format.id == "kepub" {
        Archive::from_bytes(bytes)?
    } else {
        bookx_convert::read_input(&bookx_formats::default_readers(), &bytes, name)
            .with_context(|| format!("cannot convert {name} for editing"))?
            .archive
    };
    Ok((
        archive,
        Origin {
            path: path.to_path_buf(),
            format,
        },
    ))
}

/// The backup name for `path`: `book.azw3.bak`, then `book.azw3.bak2`...; never an existing file.
pub fn backup_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("book")
        .to_string();
    let mut n = 1;
    loop {
        let suffix = if n == 1 {
            ".bak".to_string()
        } else {
            format!(".bak{n}")
        };
        let p = path.with_file_name(format!("{name}{suffix}"));
        if std::fs::symlink_metadata(&p).is_err() {
            return p;
        }
        n += 1;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedBack {
    pub path: PathBuf,
    pub backup: Option<PathBuf>,
    pub bytes: usize,
}

/// The edited book in the original format.
pub fn to_original(origin: &Origin, archive: &Archive) -> Result<Vec<u8>> {
    let epub = archive.to_bytes()?;
    let Some(writer) = origin.format.writer else {
        if origin.format.id == "kepub" {
            return Ok(epub);
        }
        bail!(
            "Bookx cannot write {}; save as EPUB instead",
            origin.format.label
        );
    };
    let cancel = AtomicBool::new(false);
    let out = bookx_convert::convert(
        &bookx_formats::default_readers(),
        &epub,
        "book.epub",
        writer,
        &bookx_convert::ConvertOptions::default(),
        &|_| {},
        &cancel,
    )
    .with_context(|| format!("converting back to {}", origin.format.label))?;
    if out.bytes.is_empty() {
        bail!("the converter returned nothing");
    }
    Ok(out.bytes)
}

/// Replaces the original file with the edited book in the original format. The old file is copied to a backup
/// first; nothing is replaced when the conversion fails.
pub fn save_back(origin: &Origin, archive: &Archive) -> Result<SavedBack> {
    let bytes = to_original(origin, archive)?;
    let target = std::fs::canonicalize(&origin.path).unwrap_or_else(|_| origin.path.clone());
    let backup = if target.exists() {
        let b = backup_path(&target);
        std::fs::copy(&target, &b)
            .with_context(|| format!("cannot make the backup {}", b.display()))?;
        Some(b)
    } else {
        None
    };
    let tmp = target.with_file_name(format!(
        ".{}.bookx-tmp",
        target.file_name().and_then(|n| n.to_str()).unwrap_or("out")
    ));
    let res = std::fs::write(&tmp, &bytes).and_then(|_| std::fs::rename(&tmp, &target));
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("cannot write {}", target.display()));
    }
    Ok(SavedBack {
        path: target,
        backup,
        bytes: bytes.len(),
    })
}

/// Saves the edited book as an EPUB at `dest` (the `.epub` extension is added when missing).
pub fn save_as_epub(archive: &Archive, dest: &Path) -> Result<PathBuf> {
    let dest = if dest
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("epub"))
    {
        dest.to_path_buf()
    } else {
        let mut s = dest.as_os_str().to_os_string();
        s.push(".epub");
        PathBuf::from(s)
    };
    archive.save(&dest)?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_core::convert::{Chapter, create_epub};
    use bookx_core::opf::Metadata;
    use bookx_editing::transform_html::{HtmlRule, HtmlRuleKind};

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-struct-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn sample() -> Archive {
        let md = Metadata {
            title: "Sample Book".into(),
            authors: vec!["A. Writer".into()],
            language: "en".into(),
            ..Default::default()
        };
        create_epub(
            &md,
            &[
                Chapter {
                    title: "One".into(),
                    body: "<h1>One</h1><p>The first words of the book.</p>".into(),
                },
                Chapter {
                    title: "Two".into(),
                    body: "<h1>Two</h1><p>More words follow.</p>".into(),
                },
            ],
            None,
        )
    }

    fn write_as(dir: &Path, name: &str, fmt: &str) -> PathBuf {
        let epub = sample().to_bytes().unwrap();
        let out = bookx_convert::convert(
            &bookx_formats::default_readers(),
            &epub,
            "book.epub",
            fmt,
            &bookx_convert::ConvertOptions::default(),
            &|_| {},
            &AtomicBool::new(false),
        )
        .unwrap();
        let p = dir.join(name);
        std::fs::write(&p, out.bytes).unwrap();
        p
    }

    fn all_text(a: &Archive) -> String {
        let b = OpenBook::from_archive(a.clone()).unwrap();
        b.spine_paths()
            .iter()
            .filter_map(|p| b.archive.get_string(p))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn formats_are_recognised_by_name() {
        let f = |n: &str| foreign_format(Path::new(n)).map(|f| f.id);
        assert_eq!(f("a.azw3"), Some("azw3"));
        assert_eq!(f("a.AZW"), Some("azw3"));
        assert_eq!(f("a.kepub.epub"), Some("kepub"));
        assert_eq!(f("a.mobi"), Some("mobi"));
        assert_eq!(f("a.epub"), None);
        assert_eq!(f("a.pdf"), None);
        assert_eq!(f("noext"), None);
        assert!(FOREIGN_FORMATS.iter().all(|f| !f.warning.is_empty()));
    }

    #[test]
    fn fb2_round_trip_edits_and_keeps_a_backup() {
        let d = tmp("fb2");
        let p = write_as(&d, "book.fb2", "fb2");
        let (mut a, o) = open_foreign(&p).unwrap();
        assert_eq!(o.format.id, "fb2");
        assert!(all_text(&a).contains("first words"));
        let b = OpenBook::from_archive(a.clone()).unwrap();
        let ch = b.spine_paths()[0].clone();
        let t = a
            .get_string(&ch)
            .unwrap()
            .replace("first words", "EDITED words");
        a.set_string(&ch, &t);
        let original = std::fs::read(&p).unwrap();
        let s = save_back(&o, &a).unwrap();
        assert_eq!(s.path, std::fs::canonicalize(&p).unwrap());
        let backup = s.backup.clone().unwrap();
        assert_eq!(
            std::fs::read(&backup).unwrap(),
            original,
            "the backup is the old file"
        );
        assert!(
            backup
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .ends_with(".fb2.bak")
        );
        let (again, _) = open_foreign(&p).unwrap();
        assert!(all_text(&again).contains("EDITED words"));
        let s2 = save_back(&o, &a).unwrap();
        assert!(
            s2.backup.unwrap().to_str().unwrap().ends_with(".bak2"),
            "backups never overwrite each other"
        );
    }

    #[test]
    fn mobi_azw3_htmlz_open_and_convert_back() {
        let d = tmp("many");
        for (name, fmt) in [("b.mobi", "mobi"), ("b.azw3", "azw3"), ("b.htmlz", "htmlz")] {
            let p = write_as(&d, name, fmt);
            let (a, o) = open_foreign(&p).unwrap();
            assert!(all_text(&a).contains("first words"), "{name}");
            let s = save_back(&o, &a).unwrap();
            assert!(s.bytes > 100 && s.backup.is_some(), "{name}");
            let (b, _) = open_foreign(&p).unwrap();
            assert!(
                all_text(&b).contains("first words"),
                "{name} after the round trip"
            );
        }
    }

    #[test]
    fn kepub_is_edited_directly_and_keeps_its_markup() {
        let d = tmp("kepub");
        let p = write_as(&d, "book.kepub.epub", "kepub");
        let (mut a, o) = open_foreign(&p).unwrap();
        assert_eq!(o.format.id, "kepub");
        let ch = OpenBook::from_archive(a.clone()).unwrap().spine_paths()[0].clone();
        let src = a.get_string(&ch).unwrap();
        assert!(
            src.contains("koboSpan"),
            "the spans are there to keep: {src}"
        );
        a.set_string(&ch, &src.replace("first", "FIRST"));
        save_back(&o, &a).unwrap();
        let (b, _) = open_foreign(&p).unwrap();
        let text = b.get_string(&ch).unwrap();
        assert!(text.contains("koboSpan") && text.contains("FIRST"));
    }

    #[test]
    fn formats_without_a_writer_only_save_as_epub() {
        let d = tmp("odt");
        let p = d.join("x.odt");
        std::fs::write(&p, b"PK").unwrap();
        assert!(
            open_foreign(&p).is_err(),
            "a broken file is refused, nothing panics"
        );
        let o = Origin {
            path: p.clone(),
            format: foreign_format(&p).unwrap(),
        };
        assert!(to_original(&o, &sample()).is_err());
        let saved = save_as_epub(&sample(), &d.join("out")).unwrap();
        assert_eq!(saved.file_name().unwrap(), "out.epub");
        assert!(Archive::open(&saved).is_ok());
        let kept = save_as_epub(&sample(), &d.join("keep.EPUB")).unwrap();
        assert_eq!(kept.file_name().unwrap(), "keep.EPUB");
    }

    #[test]
    fn a_failed_conversion_leaves_the_original_alone() {
        let d = tmp("fail");
        let p = write_as(&d, "book.fb2", "fb2");
        let before = std::fs::read(&p).unwrap();
        let (_, o) = open_foreign(&p).unwrap();
        let broken = Archive::from_bytes(sample().to_bytes().unwrap()).unwrap();
        let mut broken = broken;
        broken.remove("META-INF/container.xml");
        assert!(save_back(&o, &broken).is_err());
        assert_eq!(std::fs::read(&p).unwrap(), before);
        assert!(
            !d.join("book.fb2.bak").exists(),
            "no backup for a failed save"
        );
    }

    #[test]
    fn missing_and_oversized_files_are_refused() {
        let d = tmp("bad");
        assert!(open_foreign(&d.join("none.fb2")).is_err());
        assert!(open_foreign(&d).is_err());
        assert!(open_foreign(&d.join("x.pdf")).is_err());
    }

    #[test]
    fn rule_sets_are_saved_listed_and_deleted_with_safe_names() {
        let d = tmp("rules");
        let set = HtmlRuleSet {
            name: "../evil/Name".into(),
            rules: vec![
                HtmlRuleKind::RenameTag
                    .build("b", &["strong".into()])
                    .unwrap(),
            ],
        };
        save_html_rule_set_in(&d, &set).unwrap();
        let names: Vec<String> = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 1);
        assert!(!names[0].contains('/') && names[0].ends_with(".json"));
        assert!(!d.parent().unwrap().join("evil").exists());
        assert_eq!(list_html_rule_sets_in(&d), vec![set.clone()]);
        std::fs::write(d.join("bad.json"), "{nope").unwrap();
        std::fs::write(
            d.join("sneaky.json"),
            r#"{"name":"s","rules":[{"RenameTag":{"selector":"b","to":"<script>"}}]}"#,
        )
        .unwrap();
        assert_eq!(
            list_html_rule_sets_in(&d).len(),
            1,
            "damaged and invalid sets are ignored"
        );
        delete_html_rule_set_in(&d, &set.name);
        assert!(list_html_rule_sets_in(&d).is_empty());
        assert!(save_html_rule_set_in(&d, &HtmlRuleSet::default()).is_err());
    }

    #[test]
    fn ops_report_what_changed() {
        let mut b = OpenBook::from_archive(sample()).unwrap();
        let rules = vec![HtmlRule::RenameTag {
            selector: "p".into(),
            to: "div".into(),
        }];
        let o = html_transform(&mut b, &rules, &[]).unwrap();
        assert!(o.changed && o.details.len() >= 2);
        let o = html_transform(&mut b, &rules, &[]).unwrap();
        assert!(!o.changed);
        assert!(html_transform(&mut b, &[], &[]).is_err());
        let o = jacket_add(&mut b, &JacketOptions::default()).unwrap();
        assert!(o.changed);
        b.set_metadata(Metadata {
            title: "Changed".into(),
            ..b.metadata().clone()
        })
        .unwrap();
        assert!(jacket_refresh(&mut b).unwrap().is_some());
        assert_eq!(jacket_refresh(&mut b).unwrap(), None);
        assert!(jacket_remove(&mut b).unwrap().changed);
        assert!(!jacket_remove(&mut b).unwrap().changed);
        assert!(jacket_refresh(&mut b).unwrap().is_none());
        let o = a11y_apply(&mut b, &a11y_meta::preset("text").unwrap().form).unwrap();
        assert!(o.changed);
        assert!(
            !a11y_apply(&mut b, &a11y_meta::preset("text").unwrap().form)
                .unwrap()
                .changed
        );
    }
}
