//! The EPUB accessibility metadata form (CE-427): schema.org `accessMode`, `accessModeSufficient`,
//! `accessibilityFeature`, `accessibilityHazard`, `accessibilitySummary`, the certifier and `conformsTo`.
//! The form checks the values against the vocabulary, offers presets, suggests values from what the book
//! really contains, and warns when a claim does not match the book.

use std::collections::BTreeSet;

use anyhow::Result;
use bookx_core::book::OpenBook;
use bookx_core::util::{parse_xml, xml_escape};

use crate::accessibility::{alt_text_report, AltKind};
use crate::book_tools::{
    read_access_meta, write_access_meta, AccessMeta, ACCESS_FEATURES, ACCESS_HAZARDS, ACCESS_MODES,
};
use crate::markup::{markup_files, plain_text};
use crate::opf_edit::{append_to_section, edit_opf, require_epub3};

/// Conformance statements the form offers (the first is what Bookx suggests for a checked book).
pub const CONFORMANCE: &[&str] = &[
    "EPUB Accessibility 1.1 - WCAG 2.1 Level A",
    "EPUB Accessibility 1.1 - WCAG 2.1 Level AA",
    "EPUB Accessibility 1.1 - WCAG 2.1 Level AAA",
    "EPUB Accessibility 1.1 - WCAG 2.2 Level AA",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct A11yForm {
    pub meta: AccessMeta,
    pub certified_by: String,
    pub certifier_credential: String,
    /// URL of the certifier's report (a `link rel="a11y:certifierReport"`).
    pub certifier_report: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub field: &'static str,
    pub severity: Severity,
    pub message: String,
}

fn err(field: &'static str, m: impl Into<String>) -> Problem {
    Problem {
        field,
        severity: Severity::Error,
        message: m.into(),
    }
}
fn warn(field: &'static str, m: impl Into<String>) -> Problem {
    Problem {
        field,
        severity: Severity::Warning,
        message: m.into(),
    }
}

/// The form as stored in the book.
pub fn read_form(book: &OpenBook) -> A11yForm {
    let mut f = A11yForm {
        meta: read_access_meta(book),
        ..Default::default()
    };
    let Some(text) = book.archive.get_string(&book.package.opf_path) else {
        return f;
    };
    let Ok(doc) = parse_xml(&text) else { return f };
    for n in doc.descendants().filter(|n| n.is_element()) {
        match (
            n.tag_name().name(),
            n.attribute("property"),
            n.attribute("rel"),
        ) {
            ("meta", Some("a11y:certifiedBy"), _) => {
                f.certified_by = n.text().unwrap_or("").trim().to_string()
            }
            ("meta", Some("a11y:certifierCredential"), _) => {
                f.certifier_credential = n.text().unwrap_or("").trim().to_string()
            }
            ("link", _, Some("a11y:certifierReport")) => {
                f.certifier_report = n.attribute("href").unwrap_or("").to_string()
            }
            _ => {}
        }
    }
    f
}

/// Checks the form. Errors mean the values would make an invalid package; warnings are things a checker
/// such as Ace or the EPUB accessibility specification would flag.
pub fn validate(f: &A11yForm) -> Vec<Problem> {
    let mut p = Vec::new();
    let m = &f.meta;
    for v in &m.access_modes {
        if !ACCESS_MODES.contains(&v.as_str()) {
            p.push(err("accessMode", format!("\"{v}\" is not an access mode")));
        }
    }
    for v in &m.features {
        if !ACCESS_FEATURES.contains(&v.as_str()) && !v.is_empty() {
            p.push(err(
                "accessibilityFeature",
                format!("\"{v}\" is not an accessibility feature"),
            ));
        }
    }
    if m.features.contains("none") && m.features.len() > 1 {
        p.push(err(
            "accessibilityFeature",
            "\"none\" cannot be combined with other features",
        ));
    }
    for v in &m.hazards {
        if !ACCESS_HAZARDS.contains(&v.as_str()) {
            p.push(err(
                "accessibilityHazard",
                format!("\"{v}\" is not a hazard value"),
            ));
        }
    }
    for exclusive in ["none", "unknown"] {
        if m.hazards.contains(exclusive) && m.hazards.len() > 1 {
            p.push(err(
                "accessibilityHazard",
                format!("\"{exclusive}\" cannot be combined with other hazard values"),
            ));
        }
    }
    for (a, b) in [
        ("flashing", "noFlashingHazard"),
        ("motionSimulation", "noMotionSimulationHazard"),
        ("sound", "noSoundHazard"),
    ] {
        if m.hazards.contains(a) && m.hazards.contains(b) {
            p.push(err(
                "accessibilityHazard",
                format!("\"{a}\" and \"{b}\" contradict each other"),
            ));
        }
    }
    for s in &m.sufficient_modes {
        let parts: Vec<&str> = s.split(',').map(str::trim).collect();
        if parts
            .iter()
            .any(|x| x.is_empty() || !ACCESS_MODES.contains(x))
        {
            p.push(err(
                "accessModeSufficient",
                format!("\"{s}\" must list access modes separated by commas"),
            ));
        }
        let uniq: BTreeSet<&&str> = parts.iter().collect();
        if uniq.len() != parts.len() {
            p.push(err(
                "accessModeSufficient",
                format!("\"{s}\" repeats a mode"),
            ));
        }
    }
    if m.access_modes.is_empty() {
        p.push(warn("accessMode", "No access mode is given"));
    }
    if m.sufficient_modes.is_empty() {
        p.push(warn(
            "accessModeSufficient",
            "No sufficient access mode is given (for example \"textual\")",
        ));
    }
    if m.features.is_empty() {
        p.push(warn(
            "accessibilityFeature",
            "No accessibility feature is given (use \"none\" if the book has none)",
        ));
    }
    if m.hazards.is_empty() {
        p.push(warn(
            "accessibilityHazard",
            "No hazard statement is given (use \"unknown\" if you do not know)",
        ));
    }
    if m.summary.trim().is_empty() {
        p.push(warn("accessibilitySummary", "The summary is empty"));
    } else if m.summary.chars().count() > 1000 {
        p.push(warn(
            "accessibilitySummary",
            "The summary is very long; keep it to a few sentences",
        ));
    }
    if !m.conformance.trim().is_empty() && !CONFORMANCE.contains(&m.conformance.trim()) {
        p.push(warn(
            "conformsTo",
            "This is not one of the standard conformance statements",
        ));
    }
    if !f.certifier_credential.trim().is_empty() && f.certified_by.trim().is_empty() {
        p.push(warn(
            "certifiedBy",
            "A credential needs the name of the certifier",
        ));
    }
    let r = f.certifier_report.trim();
    let web_or_path = r.starts_with("https://") || r.starts_with("http://") || !r.contains(':');
    if !r.is_empty() && !web_or_path {
        p.push(err(
            "certifierReport",
            "The report must be a web address or a path in the book",
        ));
    }
    if r.contains(['"', '<', '>', ' ']) {
        p.push(err(
            "certifierReport",
            "The report address contains characters that are not allowed",
        ));
    }
    p
}

/// A ready-made set of values.
#[derive(Debug, Clone)]
pub struct Preset {
    pub id: &'static str,
    pub label: &'static str,
    pub form: A11yForm,
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

pub fn presets() -> Vec<Preset> {
    let text_only = A11yForm {
        meta: AccessMeta {
            summary: "This publication is a text-only book with a table of contents and a logical reading order. It has no images, audio or video.".into(),
            access_modes: set(&["textual"]),
            sufficient_modes: vec!["textual".into()],
            features: set(&["tableOfContents", "readingOrder", "displayTransformability", "structuralNavigation"]),
            hazards: set(&["noFlashingHazard", "noMotionSimulationHazard", "noSoundHazard"]),
            conformance: CONFORMANCE[1].into(),
        },
        ..Default::default()
    };
    let illustrated = A11yForm {
        meta: AccessMeta {
            summary: "This publication has images that carry text descriptions, a table of contents and a logical reading order.".into(),
            access_modes: set(&["textual", "visual"]),
            sufficient_modes: vec!["textual".into(), "textual,visual".into()],
            features: set(&["alternativeText", "tableOfContents", "readingOrder", "displayTransformability", "structuralNavigation"]),
            hazards: set(&["noFlashingHazard", "noMotionSimulationHazard", "noSoundHazard"]),
            conformance: CONFORMANCE[1].into(),
        },
        ..Default::default()
    };
    let unknown = A11yForm {
        meta: AccessMeta {
            summary: "The accessibility of this publication has not been assessed.".into(),
            access_modes: set(&["textual"]),
            sufficient_modes: vec!["textual".into()],
            features: set(&["none"]),
            hazards: set(&["unknown"]),
            conformance: String::new(),
        },
        ..Default::default()
    };
    vec![
        Preset {
            id: "text",
            label: "Text only, structured",
            form: text_only,
        },
        Preset {
            id: "illustrated",
            label: "Illustrated, images described",
            form: illustrated,
        },
        Preset {
            id: "unassessed",
            label: "Not assessed",
            form: unknown,
        },
    ]
}

pub fn preset(id: &str) -> Option<Preset> {
    presets().into_iter().find(|p| p.id == id)
}

/// What the book contains that matters for the metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    pub text_chars: usize,
    pub images: usize,
    pub images_without_alt: usize,
    pub math: bool,
    pub audio: bool,
    pub video: bool,
    pub scripts: bool,
    pub aria: bool,
    pub page_breaks: bool,
    pub headings: bool,
    pub has_toc: bool,
    pub has_index: bool,
    pub fixed_layout: bool,
}

pub fn facts(book: &OpenBook) -> Facts {
    let mut f = Facts {
        fixed_layout: book.is_fixed_layout(),
        has_toc: !book.toc().is_empty(),
        ..Default::default()
    };
    for file in markup_files(&book.archive) {
        let Some(src) = book.archive.get_string(&file) else {
            continue;
        };
        f.text_chars += plain_text(&src)
            .chars()
            .filter(|c| !c.is_whitespace())
            .count();
        f.images += src.matches("<img").count() + src.matches("<svg").count();
        f.math |= src.contains("<math");
        f.audio |= src.contains("<audio");
        f.video |= src.contains("<video");
        f.scripts |= src.contains("<script");
        f.aria |= src.contains("role=\"doc-") || src.contains("aria-");
        f.page_breaks |= src.contains("doc-pagebreak") || src.contains("epub:type=\"pagebreak\"");
        f.headings |= ["<h1", "<h2", "<h3"].iter().any(|h| src.contains(h));
        f.has_index |= src.contains("epub:type=\"index\"") || src.contains("doc-index");
    }
    f.images_without_alt = alt_text_report(book)
        .iter()
        .filter(|i| matches!(i.kind, AltKind::Missing | AltKind::LooksLikeFilename))
        .count();
    f
}

/// Suggested values and the reasons, from the book itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Suggestion {
    pub form: A11yForm,
    pub notes: Vec<String>,
}

pub fn suggest(book: &OpenBook) -> Suggestion {
    let f = facts(book);
    let mut modes = BTreeSet::new();
    let mut features = BTreeSet::new();
    let mut hazards = BTreeSet::new();
    let mut notes = Vec::new();
    if f.text_chars > 0 {
        modes.insert("textual".to_string());
    }
    if f.images > 0 {
        modes.insert("visual".to_string());
    }
    if f.audio || f.video {
        modes.insert("auditory".to_string());
    }
    let mut sufficient = Vec::new();
    if f.text_chars > 0 && f.images_without_alt == 0 && !f.audio && !f.video {
        sufficient.push("textual".to_string());
    } else if f.text_chars > 0 {
        sufficient.push(modes.iter().cloned().collect::<Vec<_>>().join(","));
    }
    if f.images > 0 {
        if f.images_without_alt == 0 {
            features.insert("alternativeText".to_string());
        } else {
            notes.push(format!(
                "{} of {} images have no description, so \"alternativeText\" is not suggested. Describe them in Tools → Accessibility.",
                f.images_without_alt, f.images
            ));
        }
    }
    if f.has_toc {
        features.insert("tableOfContents".into());
    } else {
        notes.push(
            "The book has no table of contents, so \"tableOfContents\" is not suggested.".into(),
        );
    }
    if !book.spine_paths().is_empty() {
        features.insert("readingOrder".into());
    }
    if f.headings {
        features.insert("structuralNavigation".into());
    }
    if !f.fixed_layout {
        features.insert("displayTransformability".into());
    } else {
        notes.push("A fixed-layout book cannot claim \"displayTransformability\".".into());
    }
    if f.math {
        features.insert("MathML".into());
    }
    if f.aria {
        features.insert("ARIA".into());
    }
    if f.page_breaks {
        features.insert("pageBreakMarkers".into());
    }
    if f.has_index {
        features.insert("index".into());
    }
    if features.is_empty() {
        features.insert("none".into());
    }
    if f.audio || f.video {
        hazards.insert("sound".to_string());
    } else {
        hazards.insert("noSoundHazard".to_string());
    }
    if f.scripts {
        notes.push("The book contains scripts: check for flashing and motion yourself.".into());
    }
    notes.push("Flashing and motion hazards cannot be detected from the files; confirm them before keeping the suggested values.".into());
    let summary = {
        let mut s = String::from("This publication");
        if f.images > 0 {
            if f.images_without_alt == 0 {
                s.push_str(" has images that carry text descriptions");
            } else {
                s.push_str(" has images, some without text descriptions");
            }
        } else {
            s.push_str(" is text only");
        }
        if f.has_toc {
            s.push_str(" and a table of contents");
        }
        s.push('.');
        s
    };
    Suggestion {
        form: A11yForm {
            meta: AccessMeta {
                summary,
                access_modes: modes,
                sufficient_modes: sufficient,
                features,
                hazards,
                conformance: String::new(),
            },
            ..Default::default()
        },
        notes,
    }
}

/// Claims in `form` that the book contradicts.
pub fn check_claims(book: &OpenBook, form: &A11yForm) -> Vec<String> {
    let f = facts(book);
    let m = &form.meta;
    let mut out = Vec::new();
    if m.features.contains("alternativeText") && f.images_without_alt > 0 {
        out.push(format!(
            "Claims \"alternativeText\" but {} images have no description",
            f.images_without_alt
        ));
    }
    if m.features.contains("alternativeText") && f.images == 0 {
        out.push("Claims \"alternativeText\" but the book has no images".into());
    }
    if m.features.contains("MathML") && !f.math {
        out.push("Claims \"MathML\" but the book has no MathML".into());
    }
    if m.features.contains("tableOfContents") && !f.has_toc {
        out.push("Claims \"tableOfContents\" but the book has no table of contents".into());
    }
    if m.features.contains("pageBreakMarkers") && !f.page_breaks {
        out.push("Claims \"pageBreakMarkers\" but the book has none".into());
    }
    if m.features.contains("displayTransformability") && f.fixed_layout {
        out.push("Claims \"displayTransformability\" but the book is fixed layout".into());
    }
    if m.hazards.contains("noSoundHazard") && (f.audio || f.video) {
        out.push("Claims \"noSoundHazard\" but the book has audio or video".into());
    }
    if m.access_modes.contains("auditory") && !(f.audio || f.video) {
        out.push("Claims the \"auditory\" mode but the book has no audio or video".into());
    }
    if m.access_modes.contains("visual") && f.images == 0 {
        out.push("Claims the \"visual\" mode but the book has no images".into());
    }
    out
}

/// Writes the form into the package. Errors in the form stop it; warnings do not.
pub fn write_form(book: &mut OpenBook, form: &A11yForm) -> Result<bool> {
    require_epub3(book)?;
    if let Some(e) = validate(form)
        .into_iter()
        .find(|p| p.severity == Severity::Error)
    {
        anyhow::bail!("{}: {}", e.field, e.message);
    }
    let before = book.archive.get_string(&book.package.opf_path);
    write_access_meta(book, &form.meta)?;
    let mut lines: Vec<String> = Vec::new();
    if !form.certified_by.trim().is_empty() {
        lines.push(format!(
            "<meta property=\"a11y:certifiedBy\">{}</meta>",
            xml_escape(form.certified_by.trim())
        ));
        if !form.certifier_credential.trim().is_empty() {
            lines.push(format!(
                "<meta property=\"a11y:certifierCredential\">{}</meta>",
                xml_escape(form.certifier_credential.trim())
            ));
        }
    }
    if !form.certifier_report.trim().is_empty() {
        lines.push(format!(
            "<link rel=\"a11y:certifierReport\" href=\"{}\"/>",
            xml_escape(form.certifier_report.trim())
        ));
    }
    edit_opf(book, |text| {
        let mut t = text.to_string();
        for pat in [
            r#"[ \t]*<meta\s[^>]*property\s*=\s*["']a11y:certifiedBy["'][^>]*?(?:/>|>[^<]*</meta>)[ \t]*\r?\n?"#,
            r#"[ \t]*<meta\s[^>]*property\s*=\s*["']a11y:certifierCredential["'][^>]*?(?:/>|>[^<]*</meta>)[ \t]*\r?\n?"#,
            r#"[ \t]*<link\s[^>]*rel\s*=\s*["']a11y:certifierReport["'][^>]*?/>[ \t]*\r?\n?"#,
        ] {
            t = regex::Regex::new(pat)?.replace_all(&t, "").into_owned();
        }
        for l in &lines {
            t = append_to_section(&t, "metadata", l)?;
        }
        Ok(t)
    })?;
    Ok(before != book.archive.get_string(&book.package.opf_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{make_book, png};

    fn epub3_book(bodies: &[&str]) -> OpenBook {
        let b = make_book(bodies);
        assert!(b.package.is_epub3(), "fixture must be EPUB 3");
        b
    }

    #[test]
    fn presets_are_valid_apart_from_warnings() {
        for p in presets() {
            let bad: Vec<_> = validate(&p.form)
                .into_iter()
                .filter(|x| x.severity == Severity::Error)
                .collect();
            assert!(bad.is_empty(), "{}: {bad:?}", p.id);
        }
        assert!(preset("text").is_some() && preset("zzz").is_none());
    }

    #[test]
    fn validation_catches_vocabulary_and_contradictions() {
        let mut f = A11yForm::default();
        f.meta.access_modes.insert("smell".into());
        f.meta.features.insert("none".into());
        f.meta.features.insert("index".into());
        f.meta.hazards.insert("none".into());
        f.meta.hazards.insert("sound".into());
        f.meta.sufficient_modes.push("textual,textual".into());
        f.meta.sufficient_modes.push("textual, nothing".into());
        f.certifier_report = "javascript:alert(1)".into();
        let p = validate(&f);
        let fields: Vec<&str> = p
            .iter()
            .filter(|x| x.severity == Severity::Error)
            .map(|x| x.field)
            .collect();
        for want in [
            "accessMode",
            "accessibilityFeature",
            "accessibilityHazard",
            "accessModeSufficient",
            "certifierReport",
        ] {
            assert!(fields.contains(&want), "{want} in {fields:?}");
        }
        let mut c = A11yForm::default();
        c.meta.hazards.insert("sound".into());
        c.meta.hazards.insert("noSoundHazard".into());
        assert!(validate(&c)
            .iter()
            .any(|x| x.message.contains("contradict")));
        let mut g = preset("text").unwrap().form;
        g.certifier_report = "https://example.org/report.html".into();
        assert!(validate(&g).iter().all(|x| x.severity == Severity::Warning));
        g.certifier_credential = "Ace".into();
        assert!(validate(&g).iter().any(|x| x.field == "certifiedBy"));
    }

    #[test]
    fn warnings_for_missing_statements() {
        let p = validate(&A11yForm::default());
        for f in [
            "accessMode",
            "accessModeSufficient",
            "accessibilityFeature",
            "accessibilityHazard",
            "accessibilitySummary",
        ] {
            assert!(
                p.iter()
                    .any(|x| x.field == f && x.severity == Severity::Warning),
                "{f}"
            );
        }
    }

    #[test]
    fn form_round_trips_with_certifier_and_replaces_old_values() {
        let mut b = epub3_book(&["<p>x</p>"]);
        let mut f = preset("text").unwrap().form;
        f.certified_by = "A & B Labs".into();
        f.certifier_credential = "Certified <A>".into();
        f.certifier_report = "https://example.org/r.html?a=1&b=2".into();
        assert!(write_form(&mut b, &f).unwrap());
        assert_eq!(read_form(&b), f);
        assert!(
            !write_form(&mut b, &f).unwrap(),
            "nothing changes the second time"
        );
        f.certified_by.clear();
        f.certifier_report.clear();
        f.meta.summary = "Changed".into();
        write_form(&mut b, &f).unwrap();
        let r = read_form(&b);
        assert_eq!(r.meta.summary, "Changed");
        assert!(r.certified_by.is_empty() && r.certifier_report.is_empty());
        let opf = b.archive.get_string(&b.package.opf_path).unwrap();
        assert_eq!(opf.matches("a11y:certifierReport").count(), 0);
        assert_eq!(opf.matches("schema:accessibilitySummary").count(), 1);
        assert!(bookx_core::util::parse_xml(&opf).is_ok());
    }

    #[test]
    fn invalid_forms_are_not_written() {
        let mut b = epub3_book(&["<p>x</p>"]);
        let before = b.archive.to_bytes().unwrap();
        let mut f = A11yForm::default();
        f.meta.features.insert("bogus".into());
        assert!(write_form(&mut b, &f).is_err());
        assert_eq!(b.archive.to_bytes().unwrap(), before);
    }

    #[test]
    fn suggestions_follow_the_book() {
        let mut b = epub3_book(&["<h2>Head</h2><p>Some text here.</p>"]);
        let s = suggest(&b);
        assert!(
            s.form.meta.access_modes.contains("textual")
                && !s.form.meta.access_modes.contains("visual")
        );
        assert!(s.form.meta.features.contains("readingOrder"));
        assert!(s.form.meta.features.contains("structuralNavigation"));
        assert!(!s.form.meta.features.contains("alternativeText"));
        assert_eq!(s.form.meta.sufficient_modes, vec!["textual".to_string()]);
        assert!(s.form.meta.hazards.contains("noSoundHazard"));
        // images without descriptions: no alternativeText, a note, textual alone is not sufficient any more
        let p = b.spine_paths()[0].clone();
        let src = b
            .archive
            .get_string(&p)
            .unwrap()
            .replace("<p>Some", "<p><img src=\"x.png\"/></p><p>Some");
        b.archive.set_string(&p, &src);
        crate::files::add_file(&mut b, "x.png", png(10, 10), None).unwrap();
        let s = suggest(&b);
        assert!(s.form.meta.access_modes.contains("visual"));
        assert!(!s.form.meta.features.contains("alternativeText"));
        assert!(
            s.notes.iter().any(|n| n.contains("no description")),
            "{:?}",
            s.notes
        );
        assert_eq!(
            s.form.meta.sufficient_modes,
            vec!["textual,visual".to_string()]
        );
        // describe them
        let src = b.archive.get_string(&p).unwrap().replace(
            "<img src=\"x.png\"/>",
            "<img src=\"x.png\" alt=\"A drawing of a house\"/>",
        );
        b.archive.set_string(&p, &src);
        let s = suggest(&b);
        assert!(s.form.meta.features.contains("alternativeText"));
        assert_eq!(s.form.meta.sufficient_modes, vec!["textual".to_string()]);
    }

    #[test]
    fn suggestions_see_math_audio_and_aria() {
        let mut b = epub3_book(&["<p>t</p>"]);
        let p = b.spine_paths()[0].clone();
        let src = b.archive.get_string(&p).unwrap().replace(
            "<p>t</p>",
            "<p>t</p><math xmlns=\"http://www.w3.org/1998/Math/MathML\"><mi>x</mi></math><audio src=\"a.mp3\"></audio><a role=\"doc-noteref\" href=\"#a\">1</a>",
        );
        b.archive.set_string(&p, &src);
        let s = suggest(&b);
        assert!(s.form.meta.features.contains("MathML") && s.form.meta.features.contains("ARIA"));
        assert!(s.form.meta.access_modes.contains("auditory"));
        assert!(
            s.form.meta.hazards.contains("sound") && !s.form.meta.hazards.contains("noSoundHazard")
        );
    }

    #[test]
    fn claims_are_checked_against_the_book() {
        let b = epub3_book(&["<p>plain</p>"]);
        let mut f = A11yForm::default();
        for x in ["alternativeText", "MathML", "pageBreakMarkers"] {
            f.meta.features.insert(x.into());
        }
        f.meta.access_modes.insert("auditory".into());
        f.meta.access_modes.insert("visual".into());
        let c = check_claims(&b, &f);
        assert_eq!(c.len(), 5, "{c:?}");
        assert!(check_claims(&b, &suggest(&b).form).is_empty());
    }

    #[test]
    fn epub2_books_are_refused() {
        let mut b = epub3_book(&["<p>x</p>"]);
        let opf = b.package.opf_path.clone();
        let t = b
            .archive
            .get_string(&opf)
            .unwrap()
            .replace("version=\"3.0\"", "version=\"2.0\"");
        b.archive.set_string(&opf, &t);
        b.reload_package().unwrap();
        if !b.package.is_epub3() {
            assert!(write_form(&mut b, &preset("text").unwrap().form).is_err());
        }
    }
}
