//! Package document edits for EPUB 3 Media Overlays (CE-422): manifest items for SMIL and audio, the
//! `media-overlay` attribute of chapters, `media:duration` and `media:active-class`, and the removal
//! of all of it. Text-level, so the rest of the package stays byte-identical.

use anyhow::{Result, bail};
use regex::Regex;

use crate::markup::{Event, remove_tag_attr, scan, set_tag_attr, tag_attr};
use crate::media_overlay::{ACTIVE_CLASS, format_clock};
use crate::opf_edit::{append_to_section, esc};

pub const SMIL_TYPE: &str = "application/smil+xml";

/// What one narrated chapter adds to the package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterOverlay {
    /// Manifest id of the chapter.
    pub chapter_id: String,
    pub smil_id: String,
    /// Relative to the package document.
    pub smil_href: String,
    pub audio_id: String,
    pub audio_href: String,
    pub audio_type: String,
    pub duration_ms: u64,
}

fn has_id(opf: &str, id: &str) -> bool {
    scan(opf).into_iter().any(|e| match e {
        Event::Open { name, range, .. } if name == "item" => {
            tag_attr(&opf[range], "id").as_deref() == Some(id)
        }
        _ => false,
    })
}

/// Sets `media-overlay` on the manifest item `id`.
fn set_overlay_attr(opf: &str, id: &str, smil_id: &str) -> Result<String> {
    let mut out = String::with_capacity(opf.len() + 40);
    let mut last = 0;
    let mut done = false;
    for ev in scan(opf) {
        if let Event::Open { name, range, .. } = ev {
            if name == "item" && tag_attr(&opf[range.clone()], "id").as_deref() == Some(id) {
                out.push_str(&opf[last..range.start]);
                out.push_str(&set_tag_attr(&opf[range.clone()], "media-overlay", smil_id));
                last = range.end;
                done = true;
                break;
            }
        }
    }
    if !done {
        bail!("the manifest has no item \"{id}\"");
    }
    out.push_str(&opf[last..]);
    Ok(out)
}

/// Adds a chapter's SMIL and audio to the package and points the chapter at its overlay.
pub fn add_chapter_overlay(opf: &str, ov: &ChapterOverlay) -> Result<String> {
    if has_id(opf, &ov.smil_id) || has_id(opf, &ov.audio_id) {
        bail!("the manifest already has the id \"{}\"", ov.smil_id);
    }
    let mut t = set_overlay_attr(opf, &ov.chapter_id, &ov.smil_id)?;
    t = append_to_section(
        &t,
        "manifest",
        &format!(
            "<item id=\"{}\" href=\"{}\" media-type=\"{SMIL_TYPE}\"/>",
            esc(&ov.smil_id),
            esc(&ov.smil_href)
        ),
    )?;
    t = append_to_section(
        &t,
        "manifest",
        &format!(
            "<item id=\"{}\" href=\"{}\" media-type=\"{}\"/>",
            esc(&ov.audio_id),
            esc(&ov.audio_href),
            esc(&ov.audio_type)
        ),
    )?;
    append_to_section(
        &t,
        "metadata",
        &format!(
            "<meta property=\"media:duration\" refines=\"#{}\">{}</meta>",
            esc(&ov.smil_id),
            format_clock(ov.duration_ms)
        ),
    )
}

/// Writes the total `media:duration` (replacing the old one) and makes sure `media:active-class`
/// is there.
pub fn set_total_duration(opf: &str, total_ms: u64) -> Result<String> {
    let all = Regex::new(
        r#"(?s)[ \t]*<meta\b[^>]*property\s*=\s*["']media:duration["'][^>]*>[^<]*</meta\s*>[ \t]*\r?\n?"#,
    )?;
    // the per-chapter durations (they `refines` a SMIL item) stay
    let mut t = all
        .replace_all(opf, |c: &regex::Captures| {
            if c[0].contains("refines") {
                c[0].to_string()
            } else {
                String::new()
            }
        })
        .into_owned();
    t = append_to_section(
        &t,
        "metadata",
        &format!(
            "<meta property=\"media:duration\">{}</meta>",
            format_clock(total_ms)
        ),
    )?;
    if !t.contains("media:active-class") {
        t = append_to_section(
            &t,
            "metadata",
            &format!("<meta property=\"media:active-class\">{ACTIVE_CLASS}</meta>"),
        )?;
    }
    Ok(t)
}

/// What [`remove_overlays`] took out of the package.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Removed {
    /// Package-relative hrefs of the SMIL files and of the audio files only they used.
    pub files: Vec<String>,
    pub chapters: usize,
}

/// Removes every overlay from the package text: SMIL manifest items, the audio items they used,
/// `media-overlay` attributes and the `media:*` metadata.
pub fn remove_overlays(opf: &str) -> (String, Removed) {
    // pass 1: which items are SMIL, which audio files do they reference (via the caller: hrefs)
    let mut smil_ids = Vec::new();
    for ev in scan(opf) {
        if let Event::Open { name, range, .. } = ev {
            if name == "item" {
                let tag = &opf[range];
                if tag_attr(tag, "media-type").as_deref() == Some(SMIL_TYPE) {
                    if let Some(id) = tag_attr(tag, "id") {
                        smil_ids.push(id);
                    }
                }
            }
        }
    }
    let mut removed = Removed::default();
    let mut out = String::with_capacity(opf.len());
    let mut last = 0;
    for ev in scan(opf) {
        if let Event::Open {
            name,
            range,
            self_closing: _,
        } = ev
        {
            if name != "item" {
                continue;
            }
            let tag = &opf[range.clone()];
            let ty = tag_attr(tag, "media-type").unwrap_or_default();
            if ty == SMIL_TYPE {
                if let Some(h) = tag_attr(tag, "href") {
                    removed.files.push(h);
                }
                // drop the whole line when the item stands alone on it
                let start = line_start_if_blank(opf, range.start);
                let end = line_end_if_blank(opf, range.end);
                out.push_str(&opf[last..start]);
                last = end;
            } else if tag_attr(tag, "media-overlay").is_some() {
                removed.chapters += 1;
                out.push_str(&opf[last..range.start]);
                out.push_str(&remove_tag_attr(tag, "media-overlay"));
                last = range.end;
            }
        }
    }
    out.push_str(&opf[last..]);
    let re = Regex::new(
        r#"(?s)[ \t]*<meta\b[^>]*property\s*=\s*["']media:(?:duration|active-class|narrator|playback-active-class)["'][^>]*?(?:/>|>[^<]*</meta\s*>)[ \t]*\r?\n?"#,
    )
    .expect("static regex");
    let out = re.replace_all(&out, "").into_owned();
    (out, removed)
}

/// Removes the manifest items whose `href` is in `hrefs` (package-relative).
pub fn remove_items(opf: &str, hrefs: &[String]) -> String {
    let mut out = String::with_capacity(opf.len());
    let mut last = 0;
    for ev in scan(opf) {
        if let Event::Open { name, range, .. } = ev {
            if name == "item" {
                let tag = &opf[range.clone()];
                if tag_attr(tag, "href").is_some_and(|h| hrefs.contains(&h)) {
                    let start = line_start_if_blank(opf, range.start);
                    let end = line_end_if_blank(opf, range.end);
                    out.push_str(&opf[last..start]);
                    last = end;
                }
            }
        }
    }
    out.push_str(&opf[last..]);
    out
}

fn line_start_if_blank(s: &str, at: usize) -> usize {
    let ls = s[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
    if s[ls..at].chars().all(|c| c == ' ' || c == '\t') {
        ls
    } else {
        at
    }
}

fn line_end_if_blank(s: &str, at: usize) -> usize {
    let rest = &s[at..];
    let nl = rest.find('\n');
    match nl {
        Some(n)
            if rest[..n]
                .chars()
                .all(|c| c == ' ' || c == '\t' || c == '\r') =>
        {
            at + n + 1
        }
        _ => at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPF: &str = "<?xml version=\"1.0\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"id\">\n  <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n    <dc:title>T</dc:title>\n  </metadata>\n  <manifest>\n    <item id=\"c1\" href=\"c1.xhtml\" media-type=\"application/xhtml+xml\"/>\n    <item id=\"c2\" href=\"c2.xhtml\" media-type=\"application/xhtml+xml\"/>\n  </manifest>\n  <spine><itemref idref=\"c1\"/><itemref idref=\"c2\"/></spine>\n</package>\n";

    fn ov(n: u32, ms: u64) -> ChapterOverlay {
        ChapterOverlay {
            chapter_id: format!("c{n}"),
            smil_id: format!("mo-c{n}"),
            smil_href: format!("smil/c{n}.smil"),
            audio_id: format!("au-c{n}"),
            audio_href: format!("audio/c{n}.wav"),
            audio_type: "audio/wav".into(),
            duration_ms: ms,
        }
    }

    #[test]
    fn adds_items_attributes_and_durations() {
        let t = add_chapter_overlay(OPF, &ov(1, 61_500)).unwrap();
        let t = add_chapter_overlay(&t, &ov(2, 1_000)).unwrap();
        let t = set_total_duration(&t, 62_500).unwrap();
        assert!(t.contains("<item id=\"c1\" href=\"c1.xhtml\" media-type=\"application/xhtml+xml\" media-overlay=\"mo-c1\"/>"));
        assert!(t.contains(
            "<item id=\"mo-c2\" href=\"smil/c2.smil\" media-type=\"application/smil+xml\"/>"
        ));
        assert!(t.contains("<item id=\"au-c1\" href=\"audio/c1.wav\" media-type=\"audio/wav\"/>"));
        assert!(
            t.contains("<meta property=\"media:duration\" refines=\"#mo-c1\">0:01:01.500</meta>")
        );
        assert!(t.contains("<meta property=\"media:duration\">0:01:02.500</meta>"));
        assert!(
            t.contains("<meta property=\"media:active-class\">-epub-media-overlay-active</meta>")
        );
        assert!(bookx_core::util::parse_xml(&t).is_ok());
        // the total is replaced, not duplicated
        let t = set_total_duration(&t, 70_000).unwrap();
        assert_eq!(t.matches("<meta property=\"media:duration\">").count(), 1);
        assert!(t.contains(">0:01:10.000<"));
        assert_eq!(t.matches("media:active-class").count(), 1);
        assert_eq!(t.matches("refines=\"#mo-c1\"").count(), 1);
    }

    #[test]
    fn refuses_unknown_chapters_and_duplicate_ids() {
        let mut bad = ov(1, 1);
        bad.chapter_id = "zz".into();
        assert!(add_chapter_overlay(OPF, &bad).is_err());
        let t = add_chapter_overlay(OPF, &ov(1, 1)).unwrap();
        assert!(add_chapter_overlay(&t, &ov(1, 1)).is_err());
    }

    #[test]
    fn removal_restores_the_original_package() {
        let mut t = add_chapter_overlay(OPF, &ov(1, 5_000)).unwrap();
        t = add_chapter_overlay(&t, &ov(2, 6_000)).unwrap();
        t = set_total_duration(&t, 11_000).unwrap();
        let (back, removed) = remove_overlays(&t);
        assert_eq!(removed.chapters, 2);
        assert_eq!(removed.files, vec!["smil/c1.smil", "smil/c2.smil"]);
        assert!(!back.contains("media:"));
        assert!(!back.contains("media-overlay"));
        assert!(!back.contains("smil"));
        assert!(bookx_core::util::parse_xml(&back).is_ok());
        // the audio items are left for the caller (it knows which files only the SMIL used)
        assert!(back.contains("au-c1"));
        let back = remove_items(&back, &["audio/c1.wav".into(), "audio/c2.wav".into()]);
        assert_eq!(back, OPF);
        let (again, r2) = remove_overlays(OPF);
        assert_eq!(again, OPF);
        assert_eq!(r2, Removed::default());
    }
}
