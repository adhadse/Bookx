//! "Check book" — a lightweight EPUB linter, modelled after Calibre's editor checker.

use std::collections::{HashMap, HashSet};

use regex::Regex;

use super::archive::Archive;
use super::opf::Package;
use super::util::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone)]
pub struct Issue {
    pub severity: Severity,
    pub file: String,
    pub line: Option<u32>,
    pub message: String,
}

fn issue(sev: Severity, file: &str, line: Option<u32>, msg: impl Into<String>) -> Issue {
    Issue {
        severity: sev,
        file: file.to_string(),
        line,
        message: msg.into(),
    }
}

pub fn check_book(archive: &Archive) -> Vec<Issue> {
    let mut out = Vec::new();
    let pkg = match Package::parse(archive) {
        Ok(p) => p,
        Err(e) => {
            out.push(issue(
                Severity::Error,
                "META-INF/container.xml",
                None,
                format!("Cannot read package: {e}"),
            ));
            return out;
        }
    };

    if archive
        .get("mimetype")
        .map(|m| m != b"application/epub+zip")
        .unwrap_or(true)
    {
        out.push(issue(
            Severity::Warning,
            "mimetype",
            None,
            "Missing or incorrect mimetype file",
        ));
    }

    // Well-formedness
    for name in archive.names() {
        let ext = extension(name);
        if matches!(
            ext.as_str(),
            "xhtml" | "xht" | "opf" | "ncx" | "xml" | "svg"
        ) {
            if let Some(t) = archive.get_string(name) {
                if let Err(e) = roxmltree::Document::parse_with_options(
                    t.trim_start_matches('\u{feff}'),
                    roxmltree::ParsingOptions {
                        allow_dtd: true,
                        ..Default::default()
                    },
                ) {
                    let pos = e.pos();
                    out.push(issue(
                        Severity::Error,
                        name,
                        Some(pos.row),
                        format!("XML error: {e}"),
                    ));
                }
            }
        }
    }

    // Metadata sanity
    let md = &pkg.metadata;
    if md.title.trim().is_empty() {
        out.push(issue(
            Severity::Warning,
            &pkg.opf_path,
            None,
            "The book has no title",
        ));
    }
    if md.language.trim().is_empty() {
        out.push(issue(
            Severity::Warning,
            &pkg.opf_path,
            None,
            "The book has no language (dc:language)",
        ));
    }
    if md.authors.is_empty() {
        out.push(issue(
            Severity::Info,
            &pkg.opf_path,
            None,
            "The book has no author",
        ));
    }
    if md.identifiers.is_empty() {
        out.push(issue(
            Severity::Warning,
            &pkg.opf_path,
            None,
            "The book has no identifier",
        ));
    } else if !pkg.unique_id.is_empty()
        && !md
            .identifiers
            .iter()
            .any(|i| i.xml_id.as_deref() == Some(pkg.unique_id.as_str()))
    {
        out.push(issue(
            Severity::Error,
            &pkg.opf_path,
            None,
            format!(
                "unique-identifier \"{}\" does not match any dc:identifier",
                pkg.unique_id
            ),
        ));
    }
    if pkg.cover_item().is_none() {
        out.push(issue(
            Severity::Info,
            &pkg.opf_path,
            None,
            "The book has no cover image",
        ));
    }

    // Manifest
    let mut ids: HashMap<&str, usize> = HashMap::new();
    let mut manifest_paths: HashSet<String> = HashSet::new();
    for it in &pkg.manifest {
        *ids.entry(it.id.as_str()).or_default() += 1;
        let p = pkg.item_path(it);
        if !archive.contains(&p) && !is_external(&it.href) {
            out.push(issue(
                Severity::Error,
                &pkg.opf_path,
                None,
                format!(
                    "Manifest item \"{}\" points to a missing file: {}",
                    it.id, it.href
                ),
            ));
        }
        manifest_paths.insert(p);
    }
    for (id, n) in ids {
        if n > 1 {
            out.push(issue(
                Severity::Error,
                &pkg.opf_path,
                None,
                format!("Duplicate manifest id \"{id}\""),
            ));
        }
    }
    for sp in &pkg.spine {
        if pkg.item_by_id(&sp.idref).is_none() {
            out.push(issue(
                Severity::Error,
                &pkg.opf_path,
                None,
                format!("Spine references unknown manifest id \"{}\"", sp.idref),
            ));
        }
    }
    if pkg.spine.is_empty() {
        out.push(issue(
            Severity::Error,
            &pkg.opf_path,
            None,
            "The spine is empty — the book has no readable content",
        ));
    }
    for name in archive.names() {
        if name == "mimetype" || name.starts_with("META-INF/") || *name == pkg.opf_path {
            continue;
        }
        if !manifest_paths.contains(name) {
            out.push(issue(
                Severity::Warning,
                name,
                None,
                "File is not listed in the manifest (unmanifested)",
            ));
        }
    }

    // Links
    let link_re = Regex::new(r#"(?i)\b(href|src)\s*=\s*"([^"]*)""#).unwrap();
    let id_re = Regex::new(r#"\bid\s*=\s*"([^"]*)""#).unwrap();
    let mut ids_in: HashMap<String, HashSet<String>> = HashMap::new();
    let mut html_files = Vec::new();
    for it in &pkg.manifest {
        if it.media_type == "application/xhtml+xml" || it.media_type == "text/html" {
            html_files.push(pkg.item_path(it));
        }
    }
    for f in &html_files {
        if let Some(t) = archive.get_string(f) {
            ids_in.insert(
                f.clone(),
                id_re.captures_iter(&t).map(|c| c[1].to_string()).collect(),
            );
        }
    }
    for f in &html_files {
        let Some(text) = archive.get_string(f) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            for c in link_re.captures_iter(line) {
                let href = &c[2];
                if href.is_empty() || is_external(href) {
                    continue;
                }
                let (path, frag) = resolve_href(parent_dir(f), href);
                let target = if path.is_empty() { f.clone() } else { path };
                if !archive.contains(&target) {
                    out.push(issue(
                        Severity::Error,
                        f,
                        Some(i as u32 + 1),
                        format!("Broken link: \"{href}\" does not exist"),
                    ));
                } else if let Some(fr) = frag {
                    if let Some(set) = ids_in.get(&target) {
                        if !set.contains(&fr) {
                            out.push(issue(
                                Severity::Warning,
                                f,
                                Some(i as u32 + 1),
                                format!("Link target #{fr} not found in {}", file_name(&target)),
                            ));
                        }
                    }
                }
            }
        }
    }

    out.sort_by(|a, b| a.severity.cmp(&b.severity).then(a.file.cmp(&b.file)));
    out
}
