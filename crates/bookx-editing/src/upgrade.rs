//! Upgrade the internals of an EPUB 2 book to EPUB 3 (Calibre "Upgrade book internals").

use std::collections::HashSet;

use anyhow::Result;
use bookx_core::book::OpenBook;
use bookx_core::toc::TocEntry;
use bookx_core::util::{join_path, parse_xml, xml_escape};
use regex::Regex;

use crate::markup::*;
use crate::opf_edit::*;
use crate::semantics::{SemanticsTarget, read_semantics, write_semantics};

#[derive(Debug, Clone)]
pub struct UpgradeOptions {
    /// Value for `dcterms:modified` (`2026-11-12T10:00:00Z`); the current time when `None`.
    pub modified: Option<String>,
    /// Delete the NCX (and `<spine toc>`) after the navigation document exists.
    pub remove_ncx: bool,
    /// Delete `<guide>` after converting it to landmarks.
    pub remove_guide: bool,
}

impl Default for UpgradeOptions {
    fn default() -> Self {
        Self {
            modified: None,
            remove_ncx: false,
            remove_guide: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpgradeReport {
    pub changes: Vec<String>,
}

impl UpgradeReport {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

fn nav_shell(title: &str, lang: &str) -> String {
    let lang_attr = if lang.is_empty() {
        String::new()
    } else {
        format!(" lang=\"{0}\" xml:lang=\"{0}\"", xml_escape(lang))
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"{lang_attr}>\n<head>\n  <meta charset=\"utf-8\"/>\n  <title>{}</title>\n</head>\n<body>\n  <nav epub:type=\"toc\" id=\"toc\">\n    <h1>Contents</h1>\n    <ol>\n    </ol>\n  </nav>\n</body>\n</html>\n",
        xml_escape(title)
    )
}

fn unique_item_id(book: &OpenBook, base: &str) -> String {
    let mut id = base.to_string();
    let mut n = 1;
    while book.package.item_by_id(&id).is_some() {
        n += 1;
        id = format!("{base}-{n}");
    }
    id
}

/// Converts attributes that only exist in EPUB 2 (`opf:role`, `opf:file-as`, `opf:scheme`, `opf:event`) into their
/// EPUB 3 equivalents. Returns the new package text and a description per change.
pub fn modernize_metadata(text: &str) -> Result<(String, Vec<String>)> {
    let doc = parse_xml(text)?;
    let md = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "metadata");
    let Some(md) = md else {
        return Ok((text.to_string(), Vec::new()));
    };
    let used_ids: HashSet<String> = doc
        .descendants()
        .filter_map(|n| n.attribute("id"))
        .map(|s| s.to_string())
        .collect();
    let mut taken = used_ids;
    let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    let mut metas: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for el in md.children().filter(|n| n.is_element()) {
        let local = el.tag_name().name();
        if el.tag_name().namespace() != Some(bookx_core::opf::NS_DC) {
            continue;
        }
        let start = el.range().start;
        let tag_end = start_tag_end(text, start);
        let tag = &text[start..tag_end];
        let attrs = tag_attrs(tag);
        let opf_attr = |suffix: &str| {
            attrs
                .iter()
                .find(|(n, _)| n.ends_with(&format!(":{suffix}")) && !n.starts_with("xmlns"))
                .cloned()
        };
        let role = opf_attr("role");
        let file_as = opf_attr("file-as");
        let scheme = opf_attr("scheme");
        let event = opf_attr("event");
        if role.is_none() && file_as.is_none() && scheme.is_none() && event.is_none() {
            continue;
        }
        let mut new_tag = tag.to_string();
        let mut id = el.attribute("id").map(|s| s.to_string());
        let needs_refine = role.is_some() || file_as.is_some();
        if needs_refine && id.is_none() {
            let mut n = 1;
            let mut cand = format!("{local}{n}");
            while taken.contains(&cand) {
                n += 1;
                cand = format!("{local}{n}");
            }
            taken.insert(cand.clone());
            new_tag = set_tag_attr(&new_tag, "id", &cand);
            id = Some(cand);
        }
        for a in [&role, &file_as, &scheme, &event].into_iter().flatten() {
            new_tag = remove_tag_attr(&new_tag, &a.0);
        }
        if let (Some((_, v)), Some(id)) = (&role, &id) {
            metas.push(format!(
                "<meta refines=\"#{id}\" property=\"role\" scheme=\"marc:relators\">{}</meta>",
                xml_escape(v)
            ));
            notes.push(format!(
                "Converted opf:role=\"{v}\" on dc:{local} to a refining meta"
            ));
        }
        if let (Some((_, v)), Some(id)) = (&file_as, &id) {
            metas.push(format!(
                "<meta refines=\"#{id}\" property=\"file-as\">{}</meta>",
                xml_escape(v)
            ));
            notes.push(format!(
                "Converted opf:file-as on dc:{local} to a refining meta"
            ));
        }
        let mut replacement_tail: Option<(std::ops::Range<usize>, String)> = None;
        if let Some((_, s)) = &scheme {
            notes.push(format!("Removed opf:scheme=\"{s}\" from dc:{local}"));
            if local == "identifier" {
                let value = el.text().unwrap_or("").trim().to_string();
                if !value.is_empty() && !value.to_ascii_lowercase().starts_with("urn:") {
                    let urn = format!("urn:{}:{}", s.to_ascii_lowercase(), value);
                    let content_start = tag_end;
                    let content_end = content_start + el.text().map(|t| t.len()).unwrap_or(0);
                    replacement_tail = Some((content_start..content_end, xml_escape(&urn)));
                    notes.push(format!("Rewrote identifier {value} as {urn}"));
                }
            }
        }
        if event.is_some() {
            notes.push(format!("Removed opf:event from dc:{local}"));
        }
        edits.push((start..tag_end, new_tag));
        if let Some(t) = replacement_tail {
            edits.push(t);
        }
    }
    if edits.is_empty() {
        return Ok((text.to_string(), notes));
    }
    edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    let mut out = text.to_string();
    for (r, s) in edits {
        out.replace_range(r, &s);
    }
    for m in metas {
        out = append_to_section(&out, "metadata", &m)?;
    }
    Ok((out, notes))
}

fn replace_doctype(src: &str) -> Option<String> {
    let re = Regex::new(r#"(?is)<!DOCTYPE\s+html\s+PUBLIC[^>\[]*>"#).unwrap();
    let new = re.replace(src, "<!DOCTYPE html>").into_owned();
    (new != src).then_some(new)
}

/// Upgrades EPUB 2 internals to EPUB 3. Does nothing (and returns an empty report) for EPUB 3 books.
pub fn upgrade_to_epub3(book: &mut OpenBook, opts: &UpgradeOptions) -> Result<UpgradeReport> {
    let mut rep = UpgradeReport::default();
    if book.package.is_epub3() {
        return Ok(rep);
    }
    let toc_entries = book.toc();
    let guide = read_semantics(book);
    let ncx_before = book
        .package
        .ncx_item()
        .map(|i| book.package.item_path(i))
        .and_then(|p| book.archive.get_string(&p).map(|t| (p, t)));

    // navigation document
    if book.package.nav_item().is_none() {
        let base = book.package.base_dir().to_string();
        let mut path = join_path(&base, "nav.xhtml");
        let mut n = 1;
        while book.archive.contains(&path) {
            n += 1;
            path = join_path(&base, &format!("nav-{n}.xhtml"));
        }
        let md = book.package.metadata.clone();
        book.archive
            .set_string(&path, &nav_shell(&md.title, &md.language));
        let id = unique_item_id(book, "nav");
        let pkg = book.package.clone();
        pkg.add_manifest_item(
            &mut book.archive,
            &id,
            &path,
            "application/xhtml+xml",
            "nav",
        )?;
        book.reload_package()?;
        let entries: Vec<TocEntry> = if toc_entries.is_empty() {
            book.spine_paths()
                .first()
                .map(|p| {
                    vec![TocEntry {
                        title: if md.title.is_empty() {
                            "Start".into()
                        } else {
                            md.title.clone()
                        },
                        href: p.clone(),
                        children: vec![],
                    }]
                })
                .unwrap_or_default()
        } else {
            toc_entries.clone()
        };
        let pkg = book.package.clone();
        bookx_core::toc::write_toc(&mut book.archive, &pkg, &entries)?;
        if let Some((p, t)) = &ncx_before {
            book.archive.set_string(p, t);
        }
        rep.changes.push(format!(
            "Created the navigation document {path} from the table of contents"
        ));
    }

    // package attributes and metadata
    let modified = opts
        .modified
        .clone()
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());
    let mut notes: Vec<String> = Vec::new();
    edit_opf(book, |t| {
        let (t, n) = modernize_metadata(t)?;
        notes = n;
        let mut t = set_element_attr(&t, "package", "version", "3.0");
        if !t.contains("dcterms:modified") {
            t = append_to_section(
                &t,
                "metadata",
                &format!(
                    "<meta property=\"dcterms:modified\">{}</meta>",
                    xml_escape(&modified)
                ),
            )?;
        }
        Ok(t)
    })?;
    rep.changes.push("Set package version to 3.0".into());
    rep.changes.extend(notes);
    if book
        .archive
        .get_string(&book.package.opf_path)
        .is_some_and(|t| t.contains(&format!(">{modified}<")))
    {
        rep.changes
            .push(format!("Added dcterms:modified {modified}"));
    }

    // cover image property
    if let Some(cover) = book.package.cover_item().cloned() {
        if !cover
            .properties
            .split_whitespace()
            .any(|p| p == "cover-image")
        {
            edit_opf(book, |t| {
                Ok(edit_item_properties(t, &cover.id, |o| {
                    add_property(o, "cover-image")
                }))
            })?;
            rep.changes.push(format!(
                "Marked \"{}\" with the cover-image property",
                cover.id
            ));
        }
    }

    // XHTML doctypes
    let xhtml: Vec<String> = book
        .package
        .manifest
        .iter()
        .filter(|i| i.media_type == "application/xhtml+xml")
        .map(|i| book.package.item_path(i))
        .collect();
    for p in xhtml {
        if let Some(src) = book.archive.get_string(&p) {
            if let Some(new) = replace_doctype(&src) {
                book.archive.set_string(&p, &new);
                rep.changes.push(format!(
                    "{p}: replaced the XHTML 1.x doctype by <!DOCTYPE html>"
                ));
            }
        }
    }

    // guide -> landmarks
    if !guide.is_empty() {
        let can_landmarks = book.package.nav_item().is_some();
        if can_landmarks {
            write_semantics(
                book,
                &guide,
                SemanticsTarget {
                    guide: false,
                    landmarks: true,
                },
            )?;
            rep.changes.push(format!(
                "Converted {} guide reference(s) to landmarks",
                guide.len()
            ));
        }
        if opts.remove_guide && can_landmarks {
            edit_opf(book, |t| Ok(remove_section(t, "guide")))?;
            rep.changes.push("Removed the <guide>".into());
        }
    }

    // NCX removal
    if opts.remove_ncx {
        if let Some(ncx) = book.package.ncx_item().cloned() {
            let path = book.package.item_path(&ncx);
            let pkg = book.package.clone();
            pkg.remove_manifest_item(&mut book.archive, &ncx.id)?;
            book.archive.remove(&path);
            book.reload_package()?;
            edit_opf(book, |t| {
                let mut out = String::new();
                let mut last = 0;
                for ev in scan(t) {
                    if let Event::Open { name, range, .. } = ev {
                        if name == "spine" {
                            out.push_str(&t[last..range.start]);
                            out.push_str(&remove_tag_attr(&t[range.clone()], "toc"));
                            last = range.end;
                            break;
                        }
                    }
                }
                out.push_str(&t[last..]);
                Ok(out)
            })?;
            rep.changes.push(format!("Removed the NCX {path}"));
        }
    }
    book.reload_package()?;
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantics::{Semantic, SemanticEntry};
    use bookx_core::archive::Archive;
    use bookx_core::check::{Severity, check_book};

    const OPF2: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:title opf:file-as="Tale, A">A Tale</dc:title>
    <dc:creator opf:role="aut" opf:file-as="Doe, Jane">Jane Doe</dc:creator>
    <dc:creator id="c9" opf:role="ill">Ill Ustrator</dc:creator>
    <dc:identifier id="uid" opf:scheme="UUID">0f0e0d0c-0b0a-0908-0706-050403020100</dc:identifier>
    <dc:identifier opf:scheme="ISBN">9781234567897</dc:identifier>
    <dc:date opf:event="publication">2001</dc:date>
    <dc:language>en</dc:language>
    <meta name="cover" content="img"/>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="c2.xhtml" media-type="application/xhtml+xml"/>
    <item id="img" href="cover.jpg" media-type="image/jpeg"/>
  </manifest>
  <spine toc="ncx"><itemref idref="c1"/><itemref idref="c2"/></spine>
  <guide>
    <reference type="cover" title="Cover" href="c1.xhtml"/>
    <reference type="text" title="Start" href="c2.xhtml#s"/>
  </guide>
</package>
"#;

    const NCX: &str = r#"<?xml version="1.0"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1"><head><meta name="dtb:uid" content="x"/></head><docTitle><text>A Tale</text></docTitle>
<navMap><navPoint id="n1" playOrder="1"><navLabel><text>One</text></navLabel><content src="c1.xhtml"/></navPoint>
<navPoint id="n2" playOrder="2"><navLabel><text>Two &amp; more</text></navLabel><content src="c2.xhtml#s"/>
</navPoint></navMap></ncx>"#;

    const XHTML: &str = "<?xml version=\"1.0\"?>\n<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.1//EN\" \"http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd\">\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>t</title></head><body><h1 id=\"s\">Hi</h1></body></html>";

    fn epub2() -> OpenBook {
        let mut a = Archive::default();
        a.set("mimetype", b"application/epub+zip".to_vec());
        a.set_string("META-INF/container.xml", "<?xml version=\"1.0\"?><container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>");
        a.set_string("OEBPS/content.opf", OPF2);
        a.set_string("OEBPS/toc.ncx", NCX);
        a.set_string("OEBPS/c1.xhtml", XHTML);
        a.set_string("OEBPS/c2.xhtml", XHTML);
        a.set("OEBPS/cover.jpg", vec![0xff, 0xd8, 0xff]);
        OpenBook::from_archive(a).unwrap()
    }

    #[test]
    fn modernize_attributes() {
        let (t, notes) = modernize_metadata(OPF2).unwrap();
        let doc = roxmltree::Document::parse(&t).unwrap();
        assert!(
            !t.contains("opf:role")
                && !t.contains("opf:file-as")
                && !t.contains("opf:scheme")
                && !t.contains("opf:event"),
            "{t}"
        );
        let metas: Vec<(String, String, String)> = doc
            .descendants()
            .filter(|n| n.has_tag_name("meta") && n.attribute("refines").is_some())
            .map(|n| {
                (
                    n.attribute("refines").unwrap().to_string(),
                    n.attribute("property").unwrap().to_string(),
                    n.text().unwrap().to_string(),
                )
            })
            .collect();
        assert!(
            metas.contains(&("#creator1".into(), "role".into(), "aut".into())),
            "{metas:?}"
        );
        assert!(metas.contains(&("#creator1".into(), "file-as".into(), "Doe, Jane".into())));
        assert!(metas.contains(&("#c9".into(), "role".into(), "ill".into())));
        assert!(metas.contains(&("#title1".into(), "file-as".into(), "Tale, A".into())));
        let ids: Vec<&str> = doc
            .descendants()
            .filter(|n| n.has_tag_name("identifier"))
            .map(|n| n.text().unwrap())
            .collect();
        assert_eq!(
            ids,
            vec![
                "urn:uuid:0f0e0d0c-0b0a-0908-0706-050403020100",
                "urn:isbn:9781234567897"
            ]
        );
        assert!(notes.len() >= 8);
        let (again, notes2) = modernize_metadata(&t).unwrap();
        assert_eq!(again, t);
        assert!(notes2.is_empty());
    }

    #[test]
    fn full_upgrade() {
        let mut b = epub2();
        let r = upgrade_to_epub3(
            &mut b,
            &UpgradeOptions {
                modified: Some("2026-11-12T10:00:00Z".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(b.package.is_epub3());
        assert!(r.changes.iter().any(|c| c.contains("navigation document")));
        assert!(
            r.changes
                .iter()
                .any(|c| c.contains("dcterms:modified 2026-11-12T10:00:00Z"))
        );
        let nav = b.package.nav_item().unwrap();
        assert_eq!(nav.href, "nav.xhtml");
        // nav document: independent check
        let src = b.archive.get_string("OEBPS/nav.xhtml").unwrap();
        let doc = roxmltree::Document::parse_with_options(
            &src,
            roxmltree::ParsingOptions {
                allow_dtd: true,
                ..Default::default()
            },
        )
        .unwrap();
        let toc_nav = doc
            .descendants()
            .find(|n| {
                n.has_tag_name("nav")
                    && n.attributes()
                        .any(|a| a.name() == "type" && a.value() == "toc")
            })
            .unwrap();
        let links: Vec<(&str, String)> = toc_nav
            .descendants()
            .filter(|n| n.has_tag_name("a"))
            .map(|n| {
                (
                    n.attribute("href").unwrap(),
                    n.descendants()
                        .filter(|t| t.is_text())
                        .filter_map(|t| t.text())
                        .collect::<String>(),
                )
            })
            .collect();
        assert_eq!(
            links,
            vec![
                ("c1.xhtml", "One".to_string()),
                ("c2.xhtml#s", "Two & more".to_string())
            ]
        );
        let lm: Vec<&str> = doc
            .descendants()
            .filter(|n| n.has_tag_name("a"))
            .filter_map(|n| {
                n.attributes()
                    .find(|a| a.name() == "type")
                    .map(|a| a.value())
            })
            .collect();
        assert_eq!(lm, vec!["cover", "bodymatter"]);
        // package
        let opf = b.archive.get_string("OEBPS/content.opf").unwrap();
        assert!(opf.contains("version=\"3.0\""));
        assert!(!opf.contains("<guide"));
        assert!(opf.contains("<meta property=\"dcterms:modified\">2026-11-12T10:00:00Z</meta>"));
        assert!(
            b.package
                .item_by_id("img")
                .unwrap()
                .properties
                .contains("cover-image")
        );
        // doctype
        assert!(
            b.archive
                .get_string("OEBPS/c1.xhtml")
                .unwrap()
                .contains("<!DOCTYPE html>\n<html")
        );
        assert!(
            !b.archive
                .get_string("OEBPS/c1.xhtml")
                .unwrap()
                .contains("PUBLIC")
        );
        // NCX kept by default and still valid
        assert!(b.package.ncx_item().is_some());
        assert_eq!(b.toc().len(), 2);
        assert!(
            check_book(&b.archive)
                .iter()
                .all(|i| i.severity != Severity::Error),
            "{:?}",
            check_book(&b.archive)
        );
        // idempotent
        let snapshot = b.archive.clone();
        let again = upgrade_to_epub3(&mut b, &UpgradeOptions::default()).unwrap();
        assert!(again.is_empty());
        for n in snapshot.names() {
            assert_eq!(snapshot.get(n), b.archive.get(n), "{n}");
        }
    }

    #[test]
    fn remove_ncx_and_keep_guide() {
        let mut b = epub2();
        let r = upgrade_to_epub3(
            &mut b,
            &UpgradeOptions {
                remove_ncx: true,
                remove_guide: false,
                modified: Some("2026-01-01T00:00:00Z".into()),
            },
        )
        .unwrap();
        assert!(b.package.ncx_item().is_none());
        assert!(!b.archive.contains("OEBPS/toc.ncx"));
        assert!(
            !b.archive
                .get_string("OEBPS/content.opf")
                .unwrap()
                .contains("toc=\"ncx\"")
        );
        assert!(
            b.archive
                .get_string("OEBPS/content.opf")
                .unwrap()
                .contains("<guide")
        );
        assert!(r.changes.iter().any(|c| c.starts_with("Removed the NCX")));
        assert_eq!(b.toc().len(), 2, "TOC now comes from the nav document");
        assert!(
            check_book(&b.archive)
                .iter()
                .all(|i| i.severity != Severity::Error)
        );
        let sem = read_semantics(&b);
        assert!(sem.contains(&SemanticEntry {
            semantic: Semantic::Cover,
            href: "OEBPS/c1.xhtml".into(),
            title: "Cover".into()
        }));
    }

    #[test]
    fn empty_toc_gets_fallback_entry_and_nav_name_collision() {
        let mut b = epub2();
        b.archive.set_string(
            "OEBPS/toc.ncx",
            "<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\"><navMap/></ncx>",
        );
        b.archive.set_string("OEBPS/nav.xhtml", "<html/>");
        upgrade_to_epub3(&mut b, &UpgradeOptions::default()).unwrap();
        let nav = b.package.nav_item().unwrap();
        assert_eq!(nav.href, "nav-2.xhtml");
        assert_eq!(b.toc().len(), 1);
        assert_eq!(b.toc()[0].href, "OEBPS/c1.xhtml");
    }

    #[test]
    fn existing_nav_is_kept() {
        let mut b = crate::testutil::make_book(&["<p>x</p>"]);
        let opf = b.package.opf_path.clone();
        let t = b
            .archive
            .get_string(&opf)
            .unwrap()
            .replace("version=\"3.0\"", "version=\"2.0\"");
        b.archive.set_string(&opf, &t);
        b.reload_package().unwrap();
        let nav_before = b.archive.get_string("OEBPS/nav.xhtml").unwrap();
        let r = upgrade_to_epub3(&mut b, &UpgradeOptions::default()).unwrap();
        assert!(!r.changes.iter().any(|c| c.contains("navigation document")));
        assert_eq!(b.archive.get_string("OEBPS/nav.xhtml").unwrap(), nav_before);
        assert!(b.package.is_epub3());
    }

    #[test]
    fn doctype_replacement_variants() {
        assert_eq!(replace_doctype("<!DOCTYPE html>"), None);
        assert_eq!(
            replace_doctype(
                "<!doctype html public \"-//W3C//DTD XHTML 1.0 Strict//EN\" \"x\"><html/>"
            )
            .unwrap(),
            "<!DOCTYPE html><html/>"
        );
        assert_eq!(
            replace_doctype("<!DOCTYPE html PUBLIC \"x\" [<!ENTITY a \"b\">]><html/>"),
            None,
            "internal subsets are left alone"
        );
    }
}
