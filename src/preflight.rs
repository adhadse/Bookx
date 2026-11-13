//! The check before converting or sending (roadmap PP-G-6, PP-C-5, PP-C-6): the EPUB validator
//! plus the rules of the place the book is going to (Kindle, Kobo, e-mail), as a list of blocking
//! problems and warnings per book, with a one-click fix wherever `bookx_editing::fixes` can repair
//! it. After a conversion a short report is written next to the output.
//!
//! No GTK in here, so it is unit tested; the window is `ui/preflight.rs`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use bookx_editing::fixes::FixOptions;
use bookx_editing::validate::{self, Severity};

use crate::core::archive::Archive;
use crate::core::book::OpenBook;

/// Where the book is going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dest {
    Email,
    Kindle,
    Kobo,
    Generic,
}

impl Dest {
    /// The destination implied by an output format id of [`crate::export::TARGETS`].
    pub fn for_target(id: &str) -> Dest {
        match id {
            "azw3" | "mobi" => Dest::Kindle,
            "kepub" => Dest::Kobo,
            _ => Dest::Generic,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// The device or the converter will refuse or break the book.
    Blocking,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub level: Level,
    /// The validator rule id, or a `PRE-…` id for the checks made here.
    pub id: String,
    pub text: String,
    /// `bookx_editing::fixes` repairs this.
    pub fixable: bool,
}

/// All findings for one book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookReport {
    pub book: u64,
    pub title: String,
    pub issues: Vec<Issue>,
}

impl BookReport {
    pub fn blocking(&self) -> Vec<&Issue> {
        self.issues
            .iter()
            .filter(|i| i.level == Level::Blocking)
            .collect()
    }

    pub fn warnings(&self) -> Vec<&Issue> {
        self.issues
            .iter()
            .filter(|i| i.level == Level::Warning)
            .collect()
    }

    pub fn fixable(&self) -> bool {
        self.issues.iter().any(|i| i.fixable)
    }

    pub fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }

    /// `2 blocking, 3 warnings` or `ready to send`.
    pub fn summary(&self) -> String {
        let (b, w) = (self.blocking().len(), self.warnings().len());
        match (b, w) {
            (0, 0) => "ready".into(),
            (0, w) => crate::library_health::plural(w, "warning"),
            (b, 0) => format!("{} blocking", b),
            (b, w) => format!(
                "{b} blocking, {}",
                crate::library_health::plural(w, "warning")
            ),
        }
    }
}

const MAIL_LIMIT: u64 = 25 * 1024 * 1024;
const FONT_OBFUSCATION: &[&str] = &[
    "http://www.idpf.org/2008/embedding",
    "http://ns.adobe.com/pdf/enc#RC",
];

fn has_drm(archive: &Archive) -> bool {
    let Some(x) = archive.get_string("META-INF/encryption.xml") else {
        return false;
    };
    x.matches("<enc:EncryptedData")
        .count()
        .max(x.matches("<EncryptedData").count())
        > 0
        && x.matches("Algorithm=\"").count()
            > FONT_OBFUSCATION
                .iter()
                .map(|a| x.matches(a).count())
                .sum::<usize>()
}

/// Checks the EPUB in `archive` (`size` is the file size) for `dest`.
pub fn check_archive(archive: &Archive, size: u64, dest: Dest) -> Vec<Issue> {
    let mut issues: Vec<Issue> = Vec::new();
    let report = validate::validate(archive);
    let resolved = fixable_ids(archive);
    for m in &report.messages {
        let level = match m.severity {
            Severity::Error => Level::Blocking,
            Severity::Warning => Level::Warning,
            _ => continue,
        };
        let text = if m.file.is_empty() {
            m.text.clone()
        } else {
            format!("{} ({})", m.text, m.file)
        };
        if issues.iter().any(|i| i.id == m.id && i.text == text) {
            continue;
        }
        issues.push(Issue {
            level,
            id: m.id.clone(),
            text,
            fixable: resolved.contains(&m.id),
        });
    }
    let mut pre = |level, id: &str, text: &str| {
        issues.push(Issue {
            level,
            id: id.into(),
            text: text.into(),
            fixable: false,
        });
    };
    if has_drm(archive) {
        pre(
            Level::Blocking,
            "PRE-001",
            "The book is protected by DRM; it cannot be converted or re-read here",
        );
    }
    if let Ok(book) = OpenBook::from_archive(archive.clone()) {
        let md = book.metadata();
        if md.language.trim().is_empty() && matches!(dest, Dest::Kindle | Dest::Email) {
            pre(
                Level::Blocking,
                "PRE-002",
                "No language is set; Amazon rejects books without one",
            );
        }
        if md.title.trim().is_empty() {
            pre(Level::Warning, "PRE-003", "The book has no title");
        }
        if md.authors.iter().all(|a| a.trim().is_empty()) {
            pre(Level::Warning, "PRE-004", "The book has no author");
        }
        if book.cover_bytes().is_none() {
            pre(
                Level::Warning,
                "PRE-005",
                "The book has no cover; the device shows a plain one",
            );
        }
    }
    if dest == Dest::Email && size > MAIL_LIMIT {
        pre(
            Level::Blocking,
            "PRE-006",
            "The file is over 25 MB; most mail servers refuse attachments that big",
        );
    }
    issues.sort_by_key(|i| i.level != Level::Blocking);
    issues
}

/// Rule ids that the automatic fixes repair (tried on a copy, nothing is changed).
fn fixable_ids(archive: &Archive) -> Vec<String> {
    match bookx_editing::fixes::fix_archive(archive.clone(), &FixOptions::default()) {
        Ok((_, r)) => r.resolved_validator_ids,
        Err(_) => Vec::new(),
    }
}

/// Reads the file and checks it.
pub fn check_file(path: &Path, dest: Dest) -> Result<Vec<Issue>> {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let archive = Archive::open(path).with_context(|| format!("opening {}", path.display()))?;
    Ok(check_archive(&archive, size, dest))
}

/// Repairs what can be repaired and returns the fixed archive with a line per fix.
pub fn fix_archive(archive: Archive) -> Result<(Archive, Vec<String>)> {
    let (book, report) = bookx_editing::fixes::fix_archive(archive, &FixOptions::default())?;
    let lines = report.fixes.iter().map(|f| f.description.clone()).collect();
    Ok((book.archive, lines))
}

/// The conversion report next to `output`: `Title.azw3` → `Title.azw3.report.txt`.
pub fn report_path(output: &Path) -> PathBuf {
    let mut name = output
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".report.txt");
    output.with_file_name(name)
}

/// What a finished conversion looked like.
#[derive(Debug, Clone)]
pub struct Conversion<'a> {
    pub title: &'a str,
    pub target_label: &'a str,
    pub target_id: &'a str,
    pub input_size: u64,
    pub output_size: u64,
    pub before: Vec<Issue>,
    /// Findings of the output when it is an EPUB family file, `None` otherwise.
    pub after: Option<Vec<Issue>>,
    pub fonts: usize,
    pub svg: usize,
}

/// Counts of the embedded fonts and SVG images of a book.
pub fn fonts_and_svg(archive: &Archive) -> (usize, usize) {
    let mut fonts = 0;
    let mut svg = 0;
    for n in archive.names() {
        let l = n.to_lowercase();
        if l.ends_with(".ttf")
            || l.ends_with(".otf")
            || l.ends_with(".woff")
            || l.ends_with(".woff2")
        {
            fonts += 1;
        } else if l.ends_with(".svg") {
            svg += 1;
        }
    }
    (fonts, svg)
}

/// The text of the report written next to the converted file.
pub fn render_report(c: &Conversion) -> String {
    let mut s = format!(
        "Conversion report: {}\nTarget: {} ({})\nSize: {} → {}\n",
        c.title,
        c.target_label,
        c.target_id,
        crate::library_health::size_text(c.input_size),
        crate::library_health::size_text(c.output_size),
    );
    let list = |s: &mut String, head: &str, issues: &[Issue]| {
        if issues.is_empty() {
            *s += &format!("{head}: nothing found\n");
        } else {
            *s += &format!("{head}:\n");
            for i in issues {
                let tag = if i.level == Level::Blocking {
                    "BLOCKING"
                } else {
                    "warning"
                };
                *s += &format!("  - [{tag}] {} {}\n", i.id, i.text);
            }
        }
    };
    list(&mut s, "Source book", &c.before);
    match &c.after {
        Some(a) => list(&mut s, "Result", a),
        None => s += "Result: not checked (the validator reads EPUB files only)\n",
    }
    let epub_family = matches!(c.target_id, "epub" | "kepub");
    if !epub_family && (c.fonts > 0 || c.svg > 0) {
        s += &format!(
            "Note: the book has {} embedded font{} and {} SVG image{}; {} may not keep them.\n",
            c.fonts,
            if c.fonts == 1 { "" } else { "s" },
            c.svg,
            if c.svg == 1 { "" } else { "s" },
            c.target_label
        );
    }
    s
}

/// Writes the report next to `output`. A failure is returned but never undoes the conversion.
pub fn write_report(output: &Path, c: &Conversion) -> Result<PathBuf> {
    let p = report_path(output);
    std::fs::write(&p, render_report(c)).with_context(|| format!("writing {}", p.display()))?;
    Ok(p)
}

/// Writes a converted book to `dest` and, when `report` is set, the report next to it. A report
/// that cannot be written never fails the conversion.
pub fn write_converted(
    dest: &Path,
    title: &str,
    t: &crate::export::Target,
    input: &Path,
    bytes: &[u8],
    report: bool,
) -> Result<()> {
    std::fs::write(dest, bytes).with_context(|| format!("writing {}", dest.display()))?;
    if report {
        let c = conversion_for(title, t, input, bytes);
        let _ = write_report(dest, &c);
    }
    Ok(())
}

/// Builds the report data for `input` (the library EPUB) converted into `output_bytes`.
pub fn conversion_for<'a>(
    title: &'a str,
    t: &'a crate::export::Target,
    input: &Path,
    output_bytes: &[u8],
) -> Conversion<'a> {
    let archive = Archive::open(input).ok();
    let input_size = std::fs::metadata(input).map(|m| m.len()).unwrap_or(0);
    let dest = Dest::for_target(t.id);
    let before = archive
        .as_ref()
        .map(|a| check_archive(a, input_size, dest))
        .unwrap_or_default();
    let (fonts, svg) = archive.as_ref().map_or((0, 0), fonts_and_svg);
    let after = matches!(t.id, "epub" | "kepub").then(|| {
        Archive::from_bytes(output_bytes.to_vec())
            .map(|a| check_archive(&a, output_bytes.len() as u64, dest))
            .unwrap_or_default()
    });
    Conversion {
        title,
        target_label: t.label,
        target_id: t.id,
        input_size,
        output_size: output_bytes.len() as u64,
        before,
        after,
        fonts,
        svg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::convert::{create_epub, Chapter};
    use crate::core::opf::Metadata;

    fn epub(md: Metadata) -> Vec<u8> {
        let chapters = [Chapter {
            title: "One".into(),
            body: "<h1>One</h1><p>Words.</p>".into(),
        }];
        let a = create_epub(&md, &chapters, None);
        if md.language.is_empty() || md.authors.is_empty() {
            let mut b = OpenBook::from_archive(a).unwrap();
            b.set_metadata(md).unwrap();
            return b.archive.to_bytes().unwrap();
        }
        a.to_bytes().unwrap()
    }

    fn good() -> Metadata {
        Metadata {
            title: "Good".into(),
            authors: vec!["A B".into()],
            language: "en".into(),
            ..Default::default()
        }
    }

    #[test]
    fn destinations_follow_the_target_formats() {
        assert_eq!(Dest::for_target("azw3"), Dest::Kindle);
        assert_eq!(Dest::for_target("mobi"), Dest::Kindle);
        assert_eq!(Dest::for_target("kepub"), Dest::Kobo);
        assert_eq!(Dest::for_target("fb2"), Dest::Generic);
    }

    #[test]
    fn a_book_without_language_blocks_kindle_and_mail_but_not_kobo() {
        let mut md = good();
        md.language.clear();
        let a = Archive::from_bytes(epub(md)).unwrap();
        for d in [Dest::Kindle, Dest::Email] {
            let i = check_archive(&a, 1000, d);
            assert!(
                i.iter()
                    .any(|x| x.id == "PRE-002" && x.level == Level::Blocking),
                "{d:?}: {i:?}"
            );
        }
        let i = check_archive(&a, 1000, Dest::Kobo);
        assert!(!i.iter().any(|x| x.id == "PRE-002"));
    }

    #[test]
    fn missing_cover_and_author_are_only_warnings() {
        let mut md = good();
        md.authors.clear();
        let a = Archive::from_bytes(epub(md)).unwrap();
        let i = check_archive(&a, 1000, Dest::Generic);
        let ids: Vec<&str> = i.iter().map(|x| x.id.as_str()).collect();
        assert!(
            ids.contains(&"PRE-004") && ids.contains(&"PRE-005"),
            "{ids:?}"
        );
        assert!(i
            .iter()
            .filter(|x| x.id.starts_with("PRE-00") && (x.id == "PRE-004" || x.id == "PRE-005"))
            .all(|x| x.level == Level::Warning));
    }

    #[test]
    fn big_files_cannot_be_mailed() {
        let a = Archive::from_bytes(epub(good())).unwrap();
        assert!(!check_archive(&a, 1000, Dest::Email)
            .iter()
            .any(|x| x.id == "PRE-006"));
        let i = check_archive(&a, 30 * 1024 * 1024, Dest::Email);
        assert!(i
            .iter()
            .any(|x| x.id == "PRE-006" && x.level == Level::Blocking));
        assert_eq!(i[0].level, Level::Blocking, "blocking problems come first");
    }

    #[test]
    fn a_broken_mimetype_is_found_and_fixed_in_one_step() {
        let mut a = Archive::from_bytes(epub(good())).unwrap();
        a.set("mimetype", b"application/zip".to_vec());
        let issues = check_archive(&a, 1000, Dest::Generic);
        let bad: Vec<_> = issues
            .iter()
            .filter(|i| i.level == Level::Blocking)
            .collect();
        assert!(!bad.is_empty(), "{issues:?}");
        assert!(bad.iter().any(|i| i.fixable), "{bad:?}");
        let (fixed, lines) = fix_archive(a).unwrap();
        assert!(!lines.is_empty());
        let after = check_archive(&fixed, 1000, Dest::Generic);
        assert!(
            !after
                .iter()
                .any(|i| i.level == Level::Blocking && i.id == bad[0].id),
            "{after:?}"
        );
    }

    #[test]
    fn the_summary_counts() {
        let r = BookReport {
            book: 1,
            title: "T".into(),
            issues: vec![],
        };
        assert_eq!(r.summary(), "ready");
        assert!(r.is_clean() && !r.fixable());
        let mk = |level| Issue {
            level,
            id: "X".into(),
            text: "t".into(),
            fixable: false,
        };
        let r = BookReport {
            book: 1,
            title: "T".into(),
            issues: vec![mk(Level::Blocking), mk(Level::Warning), mk(Level::Warning)],
        };
        assert_eq!(r.summary(), "1 blocking, 2 warnings");
    }

    #[test]
    fn the_conversion_report_lists_problems_and_what_was_not_checked() {
        let dir = std::env::temp_dir().join(format!("bookx-preflight-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("in.epub");
        std::fs::write(&input, epub(good())).unwrap();
        let t = crate::export::target("azw3").unwrap();
        let out = vec![0u8; 2048];
        let c = conversion_for("Good", t, &input, &out);
        assert!(c.after.is_none());
        let text = render_report(&c);
        assert!(text.contains("Conversion report: Good"), "{text}");
        assert!(text.contains("Target: AZW3 (Kindle) (azw3)"), "{text}");
        assert!(text.contains("Result: not checked"), "{text}");
        let p = write_report(&dir.join("Good.azw3"), &c).unwrap();
        assert_eq!(p.file_name().unwrap(), "Good.azw3.report.txt");
        assert!(std::fs::read_to_string(p).unwrap().contains("Source book"));

        let t = crate::export::target("epub").unwrap();
        let bytes = std::fs::read(&input).unwrap();
        let c = conversion_for("Good", t, &input, &bytes);
        assert!(c.after.is_some());
        assert!(render_report(&c).contains("Result:"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn converted_files_get_a_report_unless_it_is_switched_off() {
        let dir = std::env::temp_dir().join(format!("bookx-preflight-w-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("in.epub");
        std::fs::write(&input, epub(good())).unwrap();
        let t = crate::export::target("kepub").unwrap();
        write_converted(&dir.join("a.kepub.epub"), "A", t, &input, b"x", true).unwrap();
        assert!(dir.join("a.kepub.epub.report.txt").is_file());
        write_converted(&dir.join("b.kepub.epub"), "B", t, &input, b"x", false).unwrap();
        assert!(dir.join("b.kepub.epub").is_file());
        assert!(!dir.join("b.kepub.epub.report.txt").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fonts_and_svg_are_counted() {
        let mut a = Archive::from_bytes(epub(good())).unwrap();
        a.set("OEBPS/f.ttf", vec![1]);
        a.set("OEBPS/g.WOFF", vec![1]);
        a.set("OEBPS/p.svg", vec![1]);
        assert_eq!(fonts_and_svg(&a), (2, 1));
    }
}
