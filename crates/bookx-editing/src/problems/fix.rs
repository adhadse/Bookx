//! Automatic fixes for single problems of the list. Which ids can be fixed is [`fix_label`]; [`apply_fix`] fixes
//! one problem's file or every problem of the kind. The caller takes a checkpoint first.

use std::collections::{BTreeSet, HashMap};

use anyhow::Result;
use bookx_core::book::OpenBook;
use bookx_core::util::{extension, file_name, parent_dir};

use super::{collect, CheckOptions, Problem};
use crate::fixes::{self, FixOptions};
use crate::markup;
use crate::validate::entities;
use crate::validate::scan_entities;

/// What a fix applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// The problems of this kind in one file.
    File(String),
    /// Every problem of this kind in the book.
    Kind,
}

/// What a fix did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FixOutcome {
    pub changed: bool,
    /// One line per change, for the toast and the log.
    pub notes: Vec<String>,
}

impl FixOutcome {
    fn note(&mut self, s: impl Into<String>) {
        self.changed = true;
        self.notes.push(s.into());
    }
}

/// The verb of the *Fix* button, `None` when the problem has to be fixed by hand.
pub fn fix_label(id: &str) -> Option<&'static str> {
    Some(match id {
        "PKG-006" | "PKG-007" => "Write the mimetype file",
        "OPF-003" => "Add to the manifest",
        "RSC-001" => "Remove the entry",
        "OPF-906" => "Rename the duplicate ids",
        "HTM-905" => "Rename the repeated ids",
        "HTM-907" => "Mark pictures as decorative",
        "RSC-016" => "Repair the markup",
        "HTM-901" => "Add the namespace",
        "OPF-901" | "OPF-902" | "OPF-903" => "Add the missing metadata",
        "OPF-048" | "OPF-030" => "Repair the identifier",
        "OPF-001" => "Set the version",
        "RSC-007" | "BKX-022" => "Repair the link",
        "PKG-009" | "PKG-010" => "Rename the file",
        "BKX-001" | "BKX-024" | "BKX-026" => "Delete the file",
        "BKX-002" => "Convert to UTF-8 (from Windows-1252)",
        "BKX-003" => "Convert to UTF-8",
        "BKX-004" => "Use numeric references",
        "BKX-005" => "Write the & as &amp;",
        "BKX-006" => "Replace the entity by its text",
        "BKX-008" => "Rename the ids",
        "BKX-009" => "Wrap the text in paragraphs",
        "BKX-016" => "Convert to RGB",
        "BKX-019" => "Encode the characters",
        "BKX-025" => "Add to the reading order",
        "BKX-028" => "Keep the first cover",
        "ACC-902" => "Add the roles",
        "ACC-903" | "ACC-904" => "Add accessibility metadata",
        _ => return None,
    })
}

/// Fixes the problems of kind `id` in `scope`. Returns what was changed; `changed` is false when the fix found
/// nothing to do.
pub fn apply_fix(
    book: &mut OpenBook,
    id: &str,
    scope: &Scope,
    opts: &CheckOptions,
) -> Result<FixOutcome> {
    let mut out = FixOutcome::default();
    let targets: Vec<Problem> = collect(&book.archive, opts)
        .into_iter()
        .filter(|p| {
            p.id == id
                && match scope {
                    Scope::File(f) => &p.file == f,
                    Scope::Kind => true,
                }
        })
        .collect();
    if targets.is_empty() {
        return Ok(out);
    }
    let files: Vec<String> = targets
        .iter()
        .map(|p| p.file.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let describe = |fs: Vec<fixes::Fix>, out: &mut FixOutcome| {
        for f in fs {
            out.note(format!("{}: {}", f.file, f.description));
        }
    };
    match id {
        "PKG-006" | "PKG-007" => {
            if let Some(f) = fixes::fix_mimetype(&mut book.archive) {
                describe(vec![f], &mut out);
            }
        }
        "OPF-003" => describe(fixes::add_missing_manifest_entries(book)?, &mut out),
        "RSC-001" => describe(fixes::remove_missing_manifest_items(book)?, &mut out),
        "OPF-906" => describe(fixes::fix_duplicate_manifest_ids(book)?, &mut out),
        "HTM-905" => describe(fixes::fix_duplicate_element_ids(book), &mut out),
        "HTM-907" => describe(fixes::fix_missing_alt(book), &mut out),
        "OPF-901" | "OPF-902" | "OPF-903" => {
            describe(fixes::fix_metadata(book, &FixOptions::default())?, &mut out)
        }
        "OPF-048" | "OPF-030" => describe(fixes::fix_unique_identifier(book)?, &mut out),
        "OPF-001" => describe(fixes::fix_versions(book)?, &mut out),
        "RSC-007" | "BKX-022" => {
            let (repairs, _) = crate::links::repair_broken_links(book);
            for r in repairs {
                out.note(format!("{}: {} → {} ({})", r.file, r.old, r.new, r.how));
            }
        }
        "RSC-016" => {
            for f in &files {
                if !matches!(extension(f).as_str(), "xhtml" | "xht" | "html" | "htm") {
                    continue;
                }
                let Some(src) = book.archive.get_string(f) else {
                    continue;
                };
                if let Some(new) = fixes::repair_xhtml(&src) {
                    book.archive.set_string(f, &new);
                    out.note(format!("{f}: repaired the markup"));
                }
            }
        }
        "HTM-901" => {
            for f in &files {
                let Some(src) = book.archive.get_string(f) else {
                    continue;
                };
                if let Some(new) = fixes::fix_missing_namespace(&src) {
                    book.archive.set_string(f, &new);
                    out.note(format!("{f}: added the XHTML namespace"));
                }
            }
        }
        "PKG-009" | "PKG-010" => rename_unsafe(book, &files, &mut out)?,
        "BKX-001" | "BKX-024" | "BKX-026" => {
            for f in &files {
                if delete_path(book, f)? {
                    out.note(format!("{f}: deleted"));
                }
            }
        }
        "BKX-002" | "BKX-003" => {
            for f in &files {
                if transcode(book, f) {
                    out.note(format!("{f}: converted to UTF-8"));
                }
            }
        }
        "BKX-004" | "BKX-005" | "BKX-006" => {
            for f in &files {
                if let Some(n) = fix_entities(book, f, id) {
                    out.note(format!("{f}: fixed {n} character reference(s)"));
                }
            }
        }
        "BKX-008" => {
            for f in &files {
                let n = rename_invalid_ids(book, f);
                if n > 0 {
                    out.note(format!("{f}: renamed {n} id(s)"));
                }
            }
        }
        "BKX-009" => {
            for f in &files {
                let Some(src) = book.archive.get_string(f) else {
                    continue;
                };
                if let Some(new) = wrap_bare_text(&src) {
                    book.archive.set_string(f, &new);
                    out.note(format!("{f}: wrapped loose text in <p>"));
                }
            }
        }
        "BKX-016" => {
            for f in &files {
                let Some(data) = book.archive.get(f).map(<[u8]>::to_vec) else {
                    continue;
                };
                if let Ok(rgb) = crate::images::convert_to_jpeg(&data, 92) {
                    book.archive.set(f, rgb);
                    out.note(format!("{f}: converted to RGB"));
                }
            }
        }
        "BKX-019" => {
            for f in &files {
                if let Some(n) = encode_links(book, f) {
                    out.note(format!("{f}: encoded {n} link(s)"));
                }
            }
        }
        "BKX-025" => {
            for f in &files {
                if add_to_spine(book, f)? {
                    out.note(format!("{f}: added to the reading order"));
                }
            }
        }
        "BKX-028" => {
            if keep_first_cover(book)? {
                out.note("kept the first cover, removed the others");
            }
        }
        "ACC-902" => {
            for f in &files {
                let Some(src) = book.archive.get_string(f) else {
                    continue;
                };
                if let Some(new) = add_roles(&src) {
                    book.archive.set_string(f, &new);
                    out.note(format!("{f}: added ARIA roles"));
                }
            }
        }
        "ACC-903" | "ACC-904" => {
            let r = crate::accessibility::make_accessible(
                book,
                &crate::accessibility::AccessibilityOptions::default(),
            )?;
            for c in r.changes {
                out.note(c);
            }
        }
        _ => {}
    }
    Ok(out)
}

fn delete_path(book: &mut OpenBook, path: &str) -> Result<bool> {
    if path == book.package.opf_path || path == "mimetype" || path == "META-INF/container.xml" {
        return Ok(false);
    }
    if book.package.item_by_path(path).is_some() {
        return Ok(!crate::files::delete_files(book, &[path.to_string()])?.is_empty());
    }
    Ok(book.archive.remove(path))
}

/// A file name without the characters PKG-009/PKG-010 complain about.
pub fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_whitespace() || c.is_control() || "\\:*?\"<>|#%".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    cleaned.trim_end_matches('.').to_string()
}

fn rename_unsafe(book: &mut OpenBook, files: &[String], out: &mut FixOutcome) -> Result<()> {
    let mut map: HashMap<String, String> = HashMap::new();
    for f in files {
        let new_name = safe_name(file_name(f));
        if new_name.is_empty() || new_name == file_name(f) {
            continue;
        }
        let dir = parent_dir(f);
        let target = crate::files::unique_path(book, dir, &new_name);
        map.insert(f.clone(), target);
    }
    if map.is_empty() {
        return Ok(());
    }
    crate::files::rename_files(book, &map)?;
    let mut pairs: Vec<_> = map.into_iter().collect();
    pairs.sort();
    for (a, b) in pairs {
        out.note(format!("{a} → {b}"));
    }
    Ok(())
}

/// Decodes the file from its declared encoding (Windows-1252 when it declares none and is not UTF-8) to UTF-8
/// and rewrites the declaration.
fn transcode(book: &mut OpenBook, path: &str) -> bool {
    let Some(data) = book.archive.get(path).map(<[u8]>::to_vec) else {
        return false;
    };
    let declared = crate::validate::declared_encoding(&data);
    let enc = declared
        .as_deref()
        .and_then(|l| encoding_rs::Encoding::for_label(l.as_bytes()))
        .filter(|e| *e != encoding_rs::UTF_8)
        .unwrap_or(encoding_rs::WINDOWS_1252);
    let enc = if data.starts_with(&[0xFF, 0xFE]) {
        encoding_rs::UTF_16LE
    } else if data.starts_with(&[0xFE, 0xFF]) {
        encoding_rs::UTF_16BE
    } else if std::str::from_utf8(&data).is_ok() {
        encoding_rs::UTF_8
    } else {
        enc
    };
    let (text, _, _) = enc.decode(&data);
    let mut text = text.into_owned();
    if let Some(d) = declared.filter(|d| !crate::validate::is_utf8_label(d)) {
        let re = regex::Regex::new(&format!(r"(?i){}", regex::escape(&d))).expect("escaped");
        text = re.replace(&text, "UTF-8").into_owned();
    } else if data.starts_with(&[0xFF, 0xFE]) || data.starts_with(&[0xFE, 0xFF]) {
        text = regex::Regex::new(r"(?i)utf-16(le|be)?")
            .expect("static")
            .replace(&text, "UTF-8")
            .into_owned();
    }
    book.archive.set(path, text.into_bytes());
    true
}

fn fix_entities(book: &mut OpenBook, path: &str, id: &str) -> Option<usize> {
    let src = book.archive.get_string(path)?;
    let hits: Vec<_> = scan_entities(&src)
        .into_iter()
        .filter(|h| h.id == id)
        .collect();
    if hits.is_empty() {
        return None;
    }
    let declared: HashMap<String, String> = if id == "BKX-006" {
        regex::Regex::new(r#"<!ENTITY\s+([A-Za-z_][\w.\-]*)\s+(?:"([^"]*)"|'([^']*)')\s*>"#)
            .expect("static")
            .captures_iter(&src)
            .map(|c| {
                (
                    c[1].to_string(),
                    c.get(2)
                        .or(c.get(3))
                        .map(|m| m.as_str())
                        .unwrap_or("")
                        .to_string(),
                )
            })
            .collect()
    } else {
        HashMap::new()
    };
    let mut new = String::with_capacity(src.len());
    let mut last = 0;
    for h in &hits {
        new.push_str(&src[last..h.offset]);
        let span = &src[h.offset..h.offset + h.len];
        match id {
            "BKX-004" => match entities::lookup(&h.name) {
                Some(cp) => new.push_str(&format!("&#{cp};")),
                None => new.push_str(span),
            },
            "BKX-005" => {
                new.push_str("&amp;");
                new.push_str(&span[1..]);
            }
            _ => match declared.get(&h.name) {
                Some(v) => new.push_str(v),
                None => new.push_str(span),
            },
        }
        last = h.offset + h.len;
    }
    new.push_str(&src[last..]);
    if id == "BKX-006" {
        let re = regex::Regex::new(
            r#"[ \t]*<!ENTITY\s+[A-Za-z_][\w.\-]*\s+(?:"[^"]*"|'[^']*')\s*>\r?\n?"#,
        )
        .expect("static");
        new = re.replace_all(&new, "").into_owned();
    }
    let n = hits.len();
    book.archive.set_string(path, &new);
    Some(n)
}

/// Makes `id` a valid XML name: invalid characters become `_`, a leading digit gets `id` in front.
pub fn valid_id(id: &str) -> String {
    let mut s: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if !s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        s.insert_str(0, "id");
    }
    s
}

fn rename_invalid_ids(book: &mut OpenBook, path: &str) -> usize {
    let Some(src) = book.archive.get_string(path) else {
        return 0;
    };
    let existing: std::collections::HashSet<String> =
        markup::defined_ids(&src).into_iter().collect();
    let mut taken = existing.clone();
    let mut renames: Vec<(String, String)> = Vec::new();
    for id in &existing {
        if id.is_empty() || valid_id(id) == *id {
            continue;
        }
        let mut new = valid_id(id);
        let mut n = 2;
        while taken.contains(&new) {
            new = format!("{}_{n}", valid_id(id));
            n += 1;
        }
        taken.insert(new.clone());
        renames.push((id.clone(), new));
    }
    if renames.is_empty() {
        return 0;
    }
    let map: HashMap<&str, &str> = renames
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let text = markup::rewrite_all_tags(&src, &mut |_, attr, value| {
        if attr == "id" {
            map.get(value).map(|n| n.to_string())
        } else {
            None
        }
    });
    if text != src {
        book.archive.set_string(path, &text);
    }
    // links to the renamed ids, in this and in other files
    for f in markup::markup_files(&book.archive) {
        let Some(t) = book.archive.get_string(&f) else {
            continue;
        };
        if !t.contains('#') {
            continue;
        }
        let new = markup::map_refs(&t, markup::RefKind::Markup, &mut |href| {
            let (p, frag) = href.split_once('#')?;
            let new = map.get(bookx_core::util::percent_decode(frag).as_str())?;
            let target = if p.is_empty() {
                f.clone()
            } else {
                bookx_core::util::resolve_href(parent_dir(&f), p).0
            };
            (target == path).then(|| format!("{p}#{new}"))
        });
        if new != t {
            book.archive.set_string(&f, &new);
        }
    }
    renames.len()
}

const INLINE: &[&str] = &[
    "a", "span", "em", "strong", "i", "b", "u", "sup", "sub", "small", "br", "code", "abbr",
    "cite", "q", "ruby", "img", "mark", "s", "kbd", "var", "samp", "dfn", "bdi", "bdo", "time",
    "wbr",
];

/// Wraps text that sits directly in `<body>` (together with the inline elements around it) in `<p>`.
/// `None` when there is none or the document does not parse.
pub fn wrap_bare_text(src: &str) -> Option<String> {
    let doc = roxmltree::Document::parse_with_options(
        src,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .ok()?;
    let body = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "body")?;
    let mut spans: Vec<(usize, usize)> = Vec::new();
    // (start, end, has loose text) of the run being collected
    let mut run: Option<(usize, usize, bool)> = None;
    let close = |run: &mut Option<(usize, usize, bool)>, spans: &mut Vec<(usize, usize)>| {
        if let Some((s, e, loose)) = run.take() {
            if loose {
                spans.push((s, e));
            }
        }
    };
    for c in body.children() {
        let inline = c.is_text()
            || (c.is_element() && INLINE.contains(&c.tag_name().name()))
            || c.is_comment();
        if !inline {
            close(&mut run, &mut spans);
            continue;
        }
        let blank = c.is_text() && c.text().is_none_or(|t| t.trim().is_empty());
        if blank || c.is_comment() {
            if let Some(r) = run.as_mut() {
                r.1 = c.range().end;
            }
            continue;
        }
        let r = run.get_or_insert((c.range().start, c.range().end, false));
        r.1 = c.range().end;
        if c.is_text() {
            r.2 = true;
        }
    }
    close(&mut run, &mut spans);
    if spans.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(src.len() + spans.len() * 7);
    let mut last = 0;
    for (s, e) in spans {
        let raw = &src[s..e];
        let lead = raw.len() - raw.trim_start().len();
        let trail = raw.trim_end().len();
        out.push_str(&src[last..s + lead]);
        out.push_str("<p>");
        out.push_str(&raw[lead..trail]);
        out.push_str("</p>");
        last = s + trail;
    }
    out.push_str(&src[last..]);
    Some(out)
}

fn encode_links(book: &mut OpenBook, path: &str) -> Option<usize> {
    let src = book.archive.get_string(path)?;
    let mut n = 0;
    let new = markup::map_refs(&src, markup::RefKind::Markup, &mut |href| {
        if !markup::is_local_ref(href) {
            return None;
        }
        let (p, rest) = match href.find(['#', '?']) {
            Some(i) => (&href[..i], &href[i..]),
            None => (href, ""),
        };
        let bad = crate::validate::link_bad_char(p)?;
        if bad == ':' {
            return None;
        }
        let mut s = String::new();
        let b = p.as_bytes();
        for (i, c) in p.char_indices() {
            let broken_pct = c == '%'
                && !(b.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                    && b.get(i + 2).is_some_and(u8::is_ascii_hexdigit));
            if c == '\\' {
                s.push('/');
            } else if broken_pct || c.is_control() || " <>\"|^`{}".contains(c) {
                let mut buf = [0u8; 4];
                for byte in c.encode_utf8(&mut buf).bytes() {
                    s.push_str(&format!("%{byte:02X}"));
                }
            } else {
                s.push(c);
            }
        }
        n += 1;
        Some(format!("{s}{rest}"))
    });
    if n == 0 || new == src {
        return None;
    }
    book.archive.set_string(path, &new);
    Some(n)
}

fn add_to_spine(book: &mut OpenBook, path: &str) -> Result<bool> {
    let Some(id) = book.package.item_by_path(path).map(|i| i.id.clone()) else {
        return Ok(false);
    };
    crate::opf_edit::edit_opf(book, |t| {
        let at = t
            .rfind("</spine>")
            .ok_or_else(|| anyhow::anyhow!("no spine"))?;
        Ok(format!(
            "{}<itemref idref=\"{}\"/>\n{}",
            &t[..at],
            crate::opf_edit::esc(&id),
            &t[at..]
        ))
    })
}

fn keep_first_cover(book: &mut OpenBook) -> Result<bool> {
    crate::opf_edit::edit_opf(book, |t| {
        let meta =
            regex::Regex::new(r#"[ \t]*<meta\b[^>]*\bname\s*=\s*["']cover["'][^>]*/>[ \t]*\r?\n?"#)
                .expect("static");
        let mut seen = false;
        let t = meta
            .replace_all(t, |c: &regex::Captures| {
                if seen {
                    String::new()
                } else {
                    seen = true;
                    c[0].to_string()
                }
            })
            .into_owned();
        let item = regex::Regex::new(r"<item\b[^>]*>").expect("static");
        let mut seen_item = false;
        Ok(item
            .replace_all(&t, |c: &regex::Captures| {
                let tag = &c[0];
                let has = markup::tag_attr(tag, "properties")
                    .is_some_and(|p| p.split_whitespace().any(|x| x == "cover-image"));
                if !has {
                    return tag.to_string();
                }
                if !seen_item {
                    seen_item = true;
                    return tag.to_string();
                }
                let props: Vec<String> = markup::tag_attr(tag, "properties")
                    .unwrap_or_default()
                    .split_whitespace()
                    .filter(|x| *x != "cover-image")
                    .map(str::to_string)
                    .collect();
                if props.is_empty() {
                    markup::remove_tag_attr(tag, "properties")
                } else {
                    markup::set_tag_attr(tag, "properties", &props.join(" "))
                }
            })
            .into_owned())
    })
}

/// Adds `role` to elements that have an `epub:type` with a matching DPUB-ARIA role. `None` when nothing changes.
pub fn add_roles(src: &str) -> Option<String> {
    let mut changed = false;
    let mut pending: HashMap<usize, String> = HashMap::new();
    let _ = &mut pending;
    let re =
        regex::Regex::new(r#"<[A-Za-z][^>]*\bepub:type\s*=\s*"([^"]*)"[^>]*>"#).expect("static");
    let out = re.replace_all(src, |c: &regex::Captures| {
        let tag = &c[0];
        if markup::tag_attr(tag, "role").is_some() {
            return tag.to_string();
        }
        match c[1]
            .split_whitespace()
            .find_map(crate::accessibility::epub_type_to_role)
        {
            Some(role) => {
                changed = true;
                markup::set_tag_attr(tag, "role", role)
            }
            None => tag.to_string(),
        }
    });
    changed.then(|| out.into_owned())
}
