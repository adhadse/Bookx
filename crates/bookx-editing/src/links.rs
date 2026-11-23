//! Links (E2/E21): external link checking, downloading external resources, repairing broken internal links,
//! moving files and arranging them into folders by type.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Context, Result, bail};
use bookx_core::book::OpenBook;
use bookx_core::util::{
    extension, file_name, is_external, join_path, media_type_for, parent_dir, relative_href,
    resolve_href,
};
use serde::{Deserialize, Serialize};

use crate::markup::*;
use crate::opf_edit::edit_opf;

// ---------------------------------------------------------------------------
// external links
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkKind {
    Http,
    Https,
    Mailto,
    Ftp,
    Tel,
    Data,
    Other,
}

pub fn classify_link(href: &str) -> LinkKind {
    let h = href.trim().to_ascii_lowercase();
    if h.starts_with("https://") {
        LinkKind::Https
    } else if h.starts_with("http://") {
        LinkKind::Http
    } else if h.starts_with("mailto:") {
        LinkKind::Mailto
    } else if h.starts_with("ftp://") {
        LinkKind::Ftp
    } else if h.starts_with("tel:") {
        LinkKind::Tel
    } else if h.starts_with("data:") {
        LinkKind::Data
    } else {
        LinkKind::Other
    }
}

/// Syntax check of a URL: `Err(reason)` for addresses that cannot work.
pub fn validate_url(url: &str) -> Result<(), String> {
    let u = url.trim();
    if u.is_empty() {
        return Err("empty address".into());
    }
    if u.contains(char::is_whitespace) {
        return Err("the address contains whitespace".into());
    }
    match classify_link(u) {
        LinkKind::Http | LinkKind::Https | LinkKind::Ftp => {
            let rest = u.split_once("://").map(|x| x.1).unwrap_or("");
            let host = rest.split(['/', '?', '#']).next().unwrap_or("");
            let host = host.rsplit('@').next().unwrap_or("");
            let host = host.split(':').next().unwrap_or("");
            if host.is_empty() {
                Err("the address has no host".into())
            } else if host.starts_with('.') || host.ends_with('.') || host.contains("..") {
                Err("the host name is malformed".into())
            } else {
                Ok(())
            }
        }
        LinkKind::Mailto => {
            let addr = u[7..].split('?').next().unwrap_or("");
            match addr.split_once('@') {
                Some((l, d)) if !l.is_empty() && d.contains('.') && !d.ends_with('.') => Ok(()),
                _ => Err("not a valid e-mail address".into()),
            }
        }
        LinkKind::Tel => {
            if u[4..].chars().any(|c| c.is_ascii_digit()) {
                Ok(())
            } else {
                Err("not a valid phone number".into())
            }
        }
        LinkKind::Data => Ok(()),
        LinkKind::Other => Err("unknown link scheme".into()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalLink {
    pub file: String,
    pub url: String,
    pub kind: LinkKind,
}

/// Every external `href` of an anchor, in document order.
pub fn external_links(book: &OpenBook) -> Vec<ExternalLink> {
    let mut out = Vec::new();
    for file in markup_files(&book.archive) {
        let Some(src) = book.archive.get_string(&file) else {
            continue;
        };
        for ev in scan(&src) {
            if let Event::Open { name, range, .. } = ev {
                if name == "a" || name == "area" {
                    if let Some(h) = tag_attr(&src[range], "href") {
                        if is_external(&h) || classify_link(&h) != LinkKind::Other {
                            out.push(ExternalLink {
                                file: file.clone(),
                                kind: classify_link(&h),
                                url: h,
                            });
                        }
                    }
                }
            }
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkStatus {
    Ok,
    Redirect(String),
    NotFound,
    Error(String),
    /// Not checked (e-mail, phone, data URLs).
    Skipped,
    /// Syntactically invalid.
    Invalid(String),
}

/// Checks one URL. The real implementation lives in the application (network); tests use [`MockChecker`].
pub trait LinkChecker {
    fn check(&self, url: &str) -> LinkStatus;
}

#[derive(Debug, Clone, Default)]
pub struct MockChecker {
    pub responses: HashMap<String, LinkStatus>,
}

impl MockChecker {
    pub fn with(mut self, url: &str, status: LinkStatus) -> Self {
        self.responses.insert(url.to_string(), status);
        self
    }
}

impl LinkChecker for MockChecker {
    fn check(&self, url: &str) -> LinkStatus {
        self.responses
            .get(url)
            .cloned()
            .unwrap_or_else(|| LinkStatus::Error("no response".into()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkCheckResult {
    pub link: ExternalLink,
    pub status: LinkStatus,
}

/// Validates and checks every external link. Each distinct URL is checked once; mail, phone and data links are
/// skipped after syntax validation.
pub fn check_external_links(book: &OpenBook, checker: &dyn LinkChecker) -> Vec<LinkCheckResult> {
    let mut cache: HashMap<String, LinkStatus> = HashMap::new();
    external_links(book)
        .into_iter()
        .map(|link| {
            let status = cache
                .entry(link.url.clone())
                .or_insert_with(|| match validate_url(&link.url) {
                    Err(e) => LinkStatus::Invalid(e),
                    Ok(()) => match link.kind {
                        LinkKind::Http | LinkKind::Https | LinkKind::Ftp => {
                            checker.check(&link.url)
                        }
                        _ => LinkStatus::Skipped,
                    },
                })
                .clone();
            LinkCheckResult { link, status }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// download external resources
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Fetched {
    pub data: Vec<u8>,
    pub content_type: Option<String>,
}

/// Downloads a URL. The application provides the network implementation.
pub trait Fetcher {
    fn fetch(&self, url: &str) -> Result<Fetched>;
}

#[derive(Debug, Clone, Default)]
pub struct MockFetcher {
    pub responses: HashMap<String, Result<Fetched, String>>,
}

impl MockFetcher {
    pub fn with(mut self, url: &str, data: &[u8], content_type: Option<&str>) -> Self {
        self.responses.insert(
            url.to_string(),
            Ok(Fetched {
                data: data.to_vec(),
                content_type: content_type.map(|s| s.to_string()),
            }),
        );
        self
    }

    pub fn failing(mut self, url: &str, reason: &str) -> Self {
        self.responses
            .insert(url.to_string(), Err(reason.to_string()));
        self
    }
}

impl Fetcher for MockFetcher {
    fn fetch(&self, url: &str) -> Result<Fetched> {
        match self.responses.get(url) {
            Some(Ok(f)) => Ok(f.clone()),
            Some(Err(e)) => bail!("{e}"),
            None => bail!("404 not found"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DownloadOptions {
    /// Folder (relative to the package document) for the downloaded files.
    pub folder: String,
    pub max_bytes: usize,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            folder: "external".into(),
            max_bytes: 20 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DownloadReport {
    /// `(url, archive path)`
    pub downloaded: Vec<(String, String)>,
    /// `(url, reason)`
    pub failed: Vec<(String, String)>,
}

fn ext_for(content_type: Option<&str>, url: &str) -> String {
    let ct = content_type
        .map(|c| {
            c.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })
        .unwrap_or_default();
    let by_ct = match ct.as_str() {
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/gif" => Some("gif"),
        "image/svg+xml" => Some("svg"),
        "image/webp" => Some("webp"),
        "text/css" => Some("css"),
        "audio/mpeg" => Some("mp3"),
        "audio/mp4" => Some("m4a"),
        "video/mp4" => Some("mp4"),
        "font/ttf" | "application/x-font-ttf" => Some("ttf"),
        "font/otf" => Some("otf"),
        "font/woff" => Some("woff"),
        "font/woff2" => Some("woff2"),
        _ => None,
    };
    if let Some(e) = by_ct {
        return e.to_string();
    }
    let path = url.split(['?', '#']).next().unwrap_or("");
    let e = extension(path);
    if !e.is_empty() && e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()) {
        e
    } else {
        "bin".into()
    }
}

fn local_name_for(url: &str, ext: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let last = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("resource");
    let stem = last.rsplit_once('.').map(|(s, _)| s).unwrap_or(last);
    let mut s: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() || s.chars().all(|c| c == '_') {
        s = "resource".into();
    }
    format!("{s}.{ext}")
}

fn is_remote_resource(v: &str) -> bool {
    let l = v.trim().to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://")
}

fn is_resource_attr(tag: &str, attr: &str, tag_text: &str) -> bool {
    match attr {
        "src" | "poster" | "data" => true,
        "href" | "xlink:href" => {
            tag == "link"
                && tag_attr(tag_text, "rel").is_some_and(|r| {
                    r.to_ascii_lowercase().contains("stylesheet") || r.contains("icon")
                })
                || (tag == "image" || tag == "use")
        }
        _ => false,
    }
}

fn css_remote_urls(css: &str) -> Vec<String> {
    static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = R.get_or_init(|| {
        regex::Regex::new(
            r#"(?i)url\(\s*['"]?(https?://[^'")\s]+)|@import\s+['"](https?://[^'"]+)"#,
        )
        .unwrap()
    });
    re.captures_iter(css)
        .filter_map(|c| {
            c.get(1)
                .or_else(|| c.get(2))
                .map(|m| m.as_str().to_string())
        })
        .collect()
}

/// The `http(s)` URLs that [`download_external_resources`] would ask for in the first round: images, media and
/// stylesheets referenced from pages, and `url()`/`@import` in stylesheets and `<style>` blocks. Distinct, in
/// document order. (Resources named inside a downloaded stylesheet are found after it is downloaded.)
pub fn remote_resources(book: &OpenBook) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |u: String| {
        if !out.contains(&u) {
            out.push(u);
        }
    };
    for file in book.archive.names().to_vec() {
        let Some(src) = book.archive.get_string(&file) else {
            continue;
        };
        if is_css_path(&file) {
            css_remote_urls(&src).into_iter().for_each(&mut push);
        } else if is_markup_path(&file) {
            for ev in scan(&src) {
                if let Event::Open { name, range, .. } = ev {
                    let tag_text = &src[range];
                    for (attr, val) in tag_attrs(tag_text) {
                        if is_resource_attr(&name, &attr, tag_text) && is_remote_resource(&val) {
                            push(decode_entities(&val));
                        }
                    }
                }
            }
            css_remote_urls(&src).into_iter().for_each(&mut push);
        }
    }
    out
}

/// The same for the text of one downloaded stylesheet.
pub fn remote_resources_in_css(css: &str) -> Vec<String> {
    css_remote_urls(css)
}

/// Where `from` ends up when moved into `folder` (relative to the package document's folder; empty = that folder).
pub fn destination_in_folder(book: &OpenBook, from: &str, folder: &str) -> String {
    let base = book.package.base_dir();
    let folder = folder.trim().trim_matches('/');
    let dir = if folder.is_empty() {
        base.to_string()
    } else {
        join_path(base, folder)
    };
    join_path(&dir, file_name(from))
}

/// Downloads `http(s)` resources used by the book (images, stylesheets, fonts and media referenced from markup
/// and CSS; never plain hyperlinks), adds them to the manifest and rewrites the references to point at the local
/// copies. Stylesheets are scanned for their own resources. Unreachable resources stay as they are and are reported.
pub fn download_external_resources(
    book: &mut OpenBook,
    fetcher: &dyn Fetcher,
    opts: &DownloadOptions,
) -> Result<DownloadReport> {
    let mut rep = DownloadReport::default();
    let mut local: HashMap<String, String> = HashMap::new();
    let mut failed: HashSet<String> = HashSet::new();
    let base = book.package.base_dir().to_string();
    let mut pending_manifest: Vec<(String, String)> = Vec::new();

    let mut ensure = |book: &mut OpenBook,
                      url: &str,
                      rep: &mut DownloadReport,
                      pm: &mut Vec<(String, String)>|
     -> Option<String> {
        if let Some(p) = local.get(url) {
            return Some(p.clone());
        }
        if failed.contains(url) {
            return None;
        }
        match fetcher.fetch(url) {
            Ok(f) if f.data.is_empty() => {
                failed.insert(url.to_string());
                rep.failed.push((url.to_string(), "empty response".into()));
                None
            }
            Ok(f) if f.data.len() > opts.max_bytes => {
                failed.insert(url.to_string());
                rep.failed.push((
                    url.to_string(),
                    format!("larger than {} bytes", opts.max_bytes),
                ));
                None
            }
            Ok(f) => {
                let ext = ext_for(f.content_type.as_deref(), url);
                let name = local_name_for(url, &ext);
                let mut path = join_path(&base, &format!("{}/{name}", opts.folder));
                let mut n = 1;
                while book.archive.contains(&path) {
                    n += 1;
                    let (stem, e) = name.rsplit_once('.').unwrap_or((&name, ""));
                    path = join_path(&base, &format!("{}/{stem}-{n}.{e}", opts.folder));
                }
                book.archive.set(&path, f.data);
                pm.push((path.clone(), media_type_for(&path).to_string()));
                local.insert(url.to_string(), path.clone());
                rep.downloaded.push((url.to_string(), path.clone()));
                Some(path)
            }
            Err(e) => {
                failed.insert(url.to_string());
                rep.failed.push((url.to_string(), e.to_string()));
                None
            }
        }
    };

    // markup
    for file in markup_files(&book.archive) {
        let Some(src) = book.archive.get_string(&file) else {
            continue;
        };
        let new = {
            let rewritten = {
                let mut out = String::with_capacity(src.len());
                let mut last = 0;
                for ev in scan(&src) {
                    if let Event::Open { name, range, .. } = ev {
                        let tag = &src[range.clone()];
                        let mut changed = tag.to_string();
                        for (a, v) in tag_attrs(tag) {
                            if is_resource_attr(&name, &a, tag) && is_remote_resource(&v) {
                                if let Some(p) = ensure(book, &v, &mut rep, &mut pending_manifest) {
                                    let rel = relative_href(&file, &p);
                                    changed = rewrite_tag_attrs(&changed, &mut |an, av| {
                                        (an == a && av == v).then(|| rel.clone())
                                    });
                                }
                            }
                        }
                        if changed.contains("url(") {
                            changed = map_css_refs(&changed, &mut |r| {
                                is_remote_resource(r)
                                    .then(|| ensure(book, r, &mut rep, &mut pending_manifest))
                                    .flatten()
                                    .map(|p| relative_href(&file, &p))
                            });
                        }
                        if changed != tag {
                            out.push_str(&src[last..range.start]);
                            out.push_str(&changed);
                            last = range.end;
                        }
                    }
                }
                out.push_str(&src[last..]);
                out
            };
            map_style_blocks(&rewritten, &mut |r| {
                is_remote_resource(r)
                    .then(|| ensure(book, r, &mut rep, &mut pending_manifest))
                    .flatten()
                    .map(|p| relative_href(&file, &p))
            })
        };
        if new != src {
            book.archive.set_string(&file, &new);
        }
    }
    // stylesheets (including ones downloaded above: two passes)
    for _ in 0..2 {
        for file in css_files(&book.archive) {
            let Some(css) = book.archive.get_string(&file) else {
                continue;
            };
            let new = map_css_refs(&css, &mut |r| {
                is_remote_resource(r)
                    .then(|| ensure(book, r, &mut rep, &mut pending_manifest))
                    .flatten()
                    .map(|p| relative_href(&file, &p))
            });
            if new != css {
                book.archive.set_string(&file, &new);
            }
        }
    }

    if !pending_manifest.is_empty() {
        let mut ids: HashSet<String> = book.package.manifest.iter().map(|i| i.id.clone()).collect();
        let opf = book.package.opf_path.clone();
        let mut lines = Vec::new();
        for (path, media) in &pending_manifest {
            let stem = file_name(path)
                .rsplit_once('.')
                .map(|x| x.0)
                .unwrap_or(file_name(path));
            let mut id = format!("ext-{stem}");
            let mut n = 1;
            while ids.contains(&id) {
                n += 1;
                id = format!("ext-{stem}-{n}");
            }
            ids.insert(id.clone());
            lines.push(format!(
                "<item id=\"{}\" href=\"{}\" media-type=\"{}\"/>",
                bookx_core::util::xml_escape(&id),
                bookx_core::util::xml_escape(&relative_href(&opf, path)),
                media
            ));
        }
        edit_opf(book, |t| {
            let mut t = t.to_string();
            for l in &lines {
                t = crate::opf_edit::append_to_section(&t, "manifest", l)?;
            }
            Ok(t)
        })?;
    }
    Ok(rep)
}

// ---------------------------------------------------------------------------
// broken internal links
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokenLink {
    pub file: String,
    pub href: String,
    pub reason: String,
}

const LINK_ATTRS: &[&str] = &["href", "src", "xlink:href", "poster", "data"];

fn id_index(book: &OpenBook) -> HashMap<String, HashSet<String>> {
    markup_files(&book.archive)
        .into_iter()
        .filter_map(|f| {
            let ids = defined_ids(&book.archive.get_string(&f)?)
                .into_iter()
                .collect();
            Some((f, ids))
        })
        .collect()
}

/// Internal links (anchors, images, stylesheets, ...) whose file or fragment does not exist.
pub fn find_broken_links(book: &OpenBook) -> Vec<BrokenLink> {
    let ids = id_index(book);
    let mut out = Vec::new();
    for file in markup_files(&book.archive) {
        let Some(src) = book.archive.get_string(&file) else {
            continue;
        };
        let dir = parent_dir(&file).to_string();
        rewrite_all_tags(&src, &mut |_, attr, val| {
            if !LINK_ATTRS.contains(&attr)
                || val.trim().is_empty()
                || is_external(val)
                || !is_local_or_fragment(val)
            {
                return None;
            }
            let (p, frag) = resolve_href(&dir, val);
            let target = if p.is_empty() { file.clone() } else { p };
            if !book.archive.contains(&target) {
                out.push(BrokenLink {
                    file: file.clone(),
                    href: val.to_string(),
                    reason: "file not found".into(),
                });
            } else if let (Some(f), Some(set)) = (frag, ids.get(&target)) {
                if !set.contains(&f) {
                    out.push(BrokenLink {
                        file: file.clone(),
                        href: val.to_string(),
                        reason: format!("no element with id \"{f}\""),
                    });
                }
            }
            None
        });
    }
    out
}

fn is_local_or_fragment(v: &str) -> bool {
    v.trim().starts_with('#') || is_local_ref(v)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkRepair {
    pub file: String,
    pub old: String,
    pub new: String,
    pub how: String,
}

/// Repairs what can be repaired safely: links whose target differs only by letter case, links to a file that
/// exists exactly once elsewhere under the same name, and fragments that differ only by case or percent-encoding.
/// Returns the repairs and the links that remain broken.
pub fn repair_broken_links(book: &mut OpenBook) -> (Vec<LinkRepair>, Vec<BrokenLink>) {
    let ids = id_index(book);
    let names: Vec<String> = book.archive.names().to_vec();
    let mut repairs = Vec::new();
    for file in markup_files(&book.archive) {
        let Some(src) = book.archive.get_string(&file) else {
            continue;
        };
        let dir = parent_dir(&file).to_string();
        let mut local_repairs = Vec::new();
        let new = rewrite_all_tags(&src, &mut |_, attr, val| {
            if !LINK_ATTRS.contains(&attr)
                || val.trim().is_empty()
                || is_external(val)
                || !is_local_or_fragment(val)
            {
                return None;
            }
            let (p, frag) = resolve_href(&dir, val);
            let self_ref = p.is_empty();
            let mut target = if self_ref { file.clone() } else { p.clone() };
            let mut how = Vec::new();
            if !book.archive.contains(&target) {
                let ci: Vec<&String> = names
                    .iter()
                    .filter(|n| n.eq_ignore_ascii_case(&target))
                    .collect();
                let same_name: Vec<&String> = names
                    .iter()
                    .filter(|n| file_name(n).eq_ignore_ascii_case(file_name(&target)))
                    .collect();
                if ci.len() == 1 {
                    target = ci[0].clone();
                    how.push("fixed the letter case of the file name");
                } else if same_name.len() == 1 {
                    target = same_name[0].clone();
                    how.push("pointed at the file of the same name");
                } else {
                    return None;
                }
            }
            let mut new_frag = frag.clone();
            if let (Some(f), Some(set)) = (&frag, ids.get(&target)) {
                if !set.contains(f) {
                    let ci: Vec<&String> =
                        set.iter().filter(|i| i.eq_ignore_ascii_case(f)).collect();
                    if ci.len() == 1 {
                        new_frag = Some(ci[0].clone());
                        how.push("fixed the letter case of the fragment");
                    } else {
                        return None;
                    }
                }
            }
            if how.is_empty() {
                return None;
            }
            let mut to = target;
            if let Some(f) = new_frag {
                to.push('#');
                to.push_str(&f);
            }
            let new_href = if to.split('#').next() == Some(file.as_str()) && self_ref {
                format!("#{}", to.split_once('#').map(|x| x.1).unwrap_or(""))
            } else {
                relative_href(&file, &to)
            };
            local_repairs.push(LinkRepair {
                file: file.clone(),
                old: val.to_string(),
                new: new_href.clone(),
                how: how.join(" and "),
            });
            Some(new_href)
        });
        if new != src {
            book.archive.set_string(&file, &new);
            repairs.extend(local_repairs);
        }
    }
    let remaining = find_broken_links(book);
    (repairs, remaining)
}

// ---------------------------------------------------------------------------
// moving and arranging
// ---------------------------------------------------------------------------

/// Moves or renames one file and repairs every reference to it (and from it).
pub fn move_file(book: &mut OpenBook, from: &str, to: &str) -> Result<usize> {
    let to = normalize_dest(to);
    if !book.archive.contains(from) {
        bail!("{from} does not exist");
    }
    if from == to {
        return Ok(0);
    }
    if book.archive.contains(&to) {
        bail!("{to} already exists");
    }
    if from == book.package.opf_path || from == "META-INF/container.xml" || from == "mimetype" {
        bail!("{from} cannot be moved");
    }
    let mut map = HashMap::new();
    map.insert(from.to_string(), to);
    let n = rename_paths(&mut book.archive, &map);
    book.reload_package()?;
    Ok(n)
}

fn normalize_dest(p: &str) -> String {
    bookx_core::util::normalize_path(p)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FileGroup {
    Text,
    Images,
    Styles,
    Fonts,
    Audio,
    Video,
    Misc,
}

#[derive(Debug, Clone)]
pub struct ArrangeOptions {
    pub text: String,
    pub images: String,
    pub styles: String,
    pub fonts: String,
    pub audio: String,
    pub video: String,
    pub misc: String,
}

impl Default for ArrangeOptions {
    fn default() -> Self {
        Self {
            text: "Text".into(),
            images: "Images".into(),
            styles: "Styles".into(),
            fonts: "Fonts".into(),
            audio: "Audio".into(),
            video: "Video".into(),
            misc: "Misc".into(),
        }
    }
}

impl ArrangeOptions {
    pub fn folder(&self, g: FileGroup) -> &str {
        match g {
            FileGroup::Text => &self.text,
            FileGroup::Images => &self.images,
            FileGroup::Styles => &self.styles,
            FileGroup::Fonts => &self.fonts,
            FileGroup::Audio => &self.audio,
            FileGroup::Video => &self.video,
            FileGroup::Misc => &self.misc,
        }
    }
}

pub fn group_for(media_type: &str, path: &str) -> FileGroup {
    let ext = extension(path);
    if media_type == "application/xhtml+xml"
        || media_type == "text/html"
        || matches!(ext.as_str(), "xhtml" | "html" | "htm" | "xht")
    {
        FileGroup::Text
    } else if media_type == "text/css" || ext == "css" {
        FileGroup::Styles
    } else if media_type.starts_with("image/")
        || matches!(
            ext.as_str(),
            "jpg" | "jpeg" | "png" | "gif" | "svg" | "webp" | "bmp"
        )
    {
        FileGroup::Images
    } else if media_type.starts_with("font/")
        || media_type.contains("font")
        || media_type.contains("opentype")
        || matches!(ext.as_str(), "ttf" | "otf" | "woff" | "woff2")
    {
        FileGroup::Fonts
    } else if media_type.starts_with("audio/")
        || matches!(ext.as_str(), "mp3" | "m4a" | "ogg" | "opus" | "wav")
    {
        FileGroup::Audio
    } else if media_type.starts_with("video/") || matches!(ext.as_str(), "mp4" | "webm" | "m4v") {
        FileGroup::Video
    } else {
        FileGroup::Misc
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArrangeReport {
    pub moved: Vec<(String, String)>,
}

/// Moves every manifest file into a folder (next to the package document) according to its type and repairs all
/// references: markup `href`/`src`/`xlink:href`, CSS `url()` and `@import`, OPF, NCX and the navigation
/// document. The package document, the NCX and the container are not moved. Name clashes get a numeric suffix.
pub fn arrange_files(book: &mut OpenBook, opts: &ArrangeOptions) -> Result<ArrangeReport> {
    let base = book.package.base_dir().to_string();
    let ncx = book.package.ncx_item().map(|i| book.package.item_path(i));
    let mut taken: HashSet<String> = book.archive.names().iter().cloned().collect();
    let mut map: HashMap<String, String> = HashMap::new();
    let mut report = ArrangeReport::default();
    let items: Vec<(String, String)> = book
        .package
        .manifest
        .iter()
        .map(|i| (book.package.item_path(i), i.media_type.clone()))
        .collect();
    let mut seen: HashSet<String> = HashSet::new();
    for (path, media) in items {
        if !book.archive.contains(&path)
            || path == book.package.opf_path
            || Some(&path) == ncx.as_ref()
            || !seen.insert(path.clone())
        {
            continue;
        }
        let group = group_for(&media, &path);
        let dir = join_path(&base, opts.folder(group));
        if parent_dir(&path) == dir {
            continue;
        }
        let name = file_name(&path).to_string();
        let mut dest = join_path(&dir, &name);
        let mut n = 1;
        while taken.contains(&dest) {
            n += 1;
            let (stem, e) = name.rsplit_once('.').unwrap_or((&name, ""));
            dest = join_path(
                &dir,
                &if e.is_empty() {
                    format!("{stem}-{n}")
                } else {
                    format!("{stem}-{n}.{e}")
                },
            );
        }
        taken.remove(&path);
        taken.insert(dest.clone());
        map.insert(path.clone(), dest.clone());
        report.moved.push((path, dest));
    }
    if !map.is_empty() {
        rename_paths(&mut book.archive, &map);
        book.reload_package()
            .context("package broke while moving files")?;
    }
    Ok(report)
}

/// Summary of the files by group, for display.
pub fn group_counts(book: &OpenBook) -> BTreeMap<FileGroup, usize> {
    let mut m = BTreeMap::new();
    for i in &book.package.manifest {
        *m.entry(group_for(&i.media_type, &i.href)).or_default() += 1;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{make_book, png};
    use bookx_core::check::{Severity, check_book};

    #[test]
    fn classification_and_validation() {
        assert_eq!(classify_link("HTTPS://a.b/c"), LinkKind::Https);
        assert_eq!(classify_link("http://a.b"), LinkKind::Http);
        assert_eq!(classify_link("mailto:a@b.c"), LinkKind::Mailto);
        assert_eq!(classify_link("ftp://h/f"), LinkKind::Ftp);
        assert_eq!(classify_link("tel:+1 555"), LinkKind::Tel);
        assert_eq!(classify_link("data:text/plain,hi"), LinkKind::Data);
        assert_eq!(classify_link("chapter.xhtml"), LinkKind::Other);
        assert!(validate_url("https://example.com/a?b=c#d").is_ok());
        assert!(validate_url("http://user:pw@example.com:8080/").is_ok());
        assert_eq!(validate_url("").unwrap_err(), "empty address");
        assert_eq!(
            validate_url("http://").unwrap_err(),
            "the address has no host"
        );
        assert_eq!(
            validate_url("http://a b.com").unwrap_err(),
            "the address contains whitespace"
        );
        assert_eq!(
            validate_url("http://.example.com").unwrap_err(),
            "the host name is malformed"
        );
        assert_eq!(
            validate_url("http://exa..mple.com").unwrap_err(),
            "the host name is malformed"
        );
        assert!(validate_url("mailto:a@b.co?subject=x").is_ok());
        assert_eq!(
            validate_url("mailto:nobody").unwrap_err(),
            "not a valid e-mail address"
        );
        assert_eq!(
            validate_url("mailto:a@b.").unwrap_err(),
            "not a valid e-mail address"
        );
        assert!(validate_url("tel:+1-555-0100").is_ok());
        assert_eq!(
            validate_url("tel:abc").unwrap_err(),
            "not a valid phone number"
        );
        assert!(validate_url("data:image/png;base64,AA").is_ok());
        assert_eq!(
            validate_url("gopher://x").unwrap_err(),
            "unknown link scheme"
        );
    }

    fn link_book() -> OpenBook {
        make_book(&[
            "<p><a href=\"http://ok.example/\">ok</a> <a href=\"https://gone.example/x\">gone</a> <a href=\"https://ok.example/\">dup</a> <a href=\"mailto:a@b.co\">mail</a> <a href=\"http://\">bad</a> <a href=\"chapter-2.xhtml\">internal</a> <a href=\"https://moved.example/\">moved</a></p>",
            "<p><a href=\"http://ok.example/\">again</a></p>",
        ])
    }

    #[test]
    fn external_link_check_uses_the_checker_once_per_url() {
        use std::cell::RefCell;
        struct Counting(MockChecker, RefCell<Vec<String>>);
        impl LinkChecker for Counting {
            fn check(&self, url: &str) -> LinkStatus {
                self.1.borrow_mut().push(url.to_string());
                self.0.check(url)
            }
        }
        let b = link_book();
        let c = Counting(
            MockChecker::default()
                .with("http://ok.example/", LinkStatus::Ok)
                .with("https://ok.example/", LinkStatus::Ok)
                .with("https://gone.example/x", LinkStatus::NotFound)
                .with(
                    "https://moved.example/",
                    LinkStatus::Redirect("https://new.example/".into()),
                ),
            RefCell::new(Vec::new()),
        );
        let r = check_external_links(&b, &c);
        let status = |u: &str| {
            r.iter()
                .filter(|x| x.link.url == u)
                .map(|x| x.status.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            r.len(),
            7,
            "6 external in chapter 1 (the internal one is excluded) + 1 in chapter 2"
        );
        assert_eq!(
            status("http://ok.example/"),
            vec![LinkStatus::Ok, LinkStatus::Ok]
        );
        assert_eq!(status("https://gone.example/x"), vec![LinkStatus::NotFound]);
        assert_eq!(status("mailto:a@b.co"), vec![LinkStatus::Skipped]);
        assert_eq!(
            status("http://"),
            vec![LinkStatus::Invalid("the address has no host".into())]
        );
        assert_eq!(
            status("https://moved.example/"),
            vec![LinkStatus::Redirect("https://new.example/".into())]
        );
        assert!(r.iter().all(|x| x.link.url != "chapter-2.xhtml"));
        let calls = c.1.borrow();
        assert_eq!(
            calls.iter().filter(|u| *u == "http://ok.example/").count(),
            1
        );
        assert_eq!(calls.len(), 4);
        // unknown URL -> mock error
        let unknown = check_external_links(&b, &MockChecker::default());
        assert!(matches!(&unknown[0].status, LinkStatus::Error(e) if e == "no response"));
    }

    #[test]
    fn download_rewrites_references_and_manifest() {
        let mut b = make_book(&[
            "<p><img src=\"http://img.example/pics/cat.PNG?size=2\" alt=\"\"/><a href=\"http://img.example/pics/cat.png\">link stays</a> <img src=\"https://img.example/bad.png\" alt=\"\"/><img src=\"http://img.example/pics/cat.PNG?size=2\" alt=\"\"/></p><div style=\"background: url(http://img.example/bg.jpg)\">x</div>",
        ]);
        b.archive.set_string("OEBPS/style.css", "@import url(http://css.example/more.css);\nbody { background: url('http://img.example/bg.jpg') }\n@font-face { font-family: F; src: url(https://f.example/f.woff2) }\n");
        let png = png(4, 4);
        let f = MockFetcher::default()
            .with(
                "http://img.example/pics/cat.PNG?size=2",
                &png,
                Some("image/png"),
            )
            .failing("https://img.example/bad.png", "HTTP 500")
            .with(
                "http://img.example/bg.jpg",
                b"\xff\xd8\xff",
                Some("image/jpeg; charset=binary"),
            )
            .with(
                "http://css.example/more.css",
                b"p { background: url(http://img.example/deep.gif) }",
                Some("text/css"),
            )
            .with("http://img.example/deep.gif", b"GIF89a", None)
            .with("https://f.example/f.woff2", b"wOF2", Some("font/woff2"));
        let rep = download_external_resources(&mut b, &f, &DownloadOptions::default()).unwrap();
        assert_eq!(
            rep.failed,
            vec![(
                "https://img.example/bad.png".to_string(),
                "HTTP 500".to_string()
            )]
        );
        assert_eq!(rep.downloaded.len(), 5, "{:?}", rep.downloaded);
        let ch = b.archive.get_string("OEBPS/chapter-1.xhtml").unwrap();
        assert_eq!(ch.matches("src=\"external/cat.png\"").count(), 2, "{ch}");
        assert!(
            ch.contains("href=\"http://img.example/pics/cat.png\""),
            "hyperlinks are never rewritten"
        );
        assert!(ch.contains("src=\"https://img.example/bad.png\""));
        assert!(ch.contains("url(external/bg.jpg)"));
        let css = b.archive.get_string("OEBPS/style.css").unwrap();
        assert!(css.contains("@import url(external/more.css)"), "{css}");
        assert!(css.contains("url('external/bg.jpg')"));
        assert!(css.contains("url(external/f.woff2)"));
        let more = b.archive.get_string("OEBPS/external/more.css").unwrap();
        assert!(more.contains("url(deep.gif)"), "{more}");
        assert_eq!(
            b.archive.get("OEBPS/external/cat.png").unwrap(),
            png.as_slice()
        );
        for (path, media) in [
            ("OEBPS/external/cat.png", "image/png"),
            ("OEBPS/external/bg.jpg", "image/jpeg"),
            ("OEBPS/external/more.css", "text/css"),
            ("OEBPS/external/deep.gif", "image/gif"),
            ("OEBPS/external/f.woff2", "font/woff2"),
        ] {
            assert_eq!(
                b.package.item_by_path(path).unwrap().media_type,
                media,
                "{path}"
            );
        }
        assert!(
            check_book(&b.archive)
                .iter()
                .all(|i| !i.message.contains("not listed in the manifest"))
        );
        // second run finds nothing new to download except the failing one
        let again = download_external_resources(&mut b, &f, &DownloadOptions::default()).unwrap();
        assert_eq!(again.downloaded.len(), 0);
        assert_eq!(again.failed.len(), 1);
    }

    #[test]
    fn download_limits_and_naming() {
        let mut b = make_book(&[
            "<p><img src=\"http://x.example/a.png\" alt=\"\"/><img src=\"http://y.example/a.png\" alt=\"\"/><img src=\"http://z.example/big.png\" alt=\"\"/><img src=\"http://z.example/empty.png\" alt=\"\"/><img src=\"http://z.example/\" alt=\"\"/></p>",
        ]);
        let f = MockFetcher::default()
            .with("http://x.example/a.png", &[1], Some("image/png"))
            .with("http://y.example/a.png", &[2], Some("image/png"))
            .with("http://z.example/big.png", &[0; 100], None)
            .with("http://z.example/empty.png", &[], None)
            .with("http://z.example/", &[3], Some("application/x-unknown"));
        let rep = download_external_resources(
            &mut b,
            &f,
            &DownloadOptions {
                max_bytes: 50,
                folder: "dl/files".into(),
            },
        )
        .unwrap();
        let paths: Vec<&str> = rep.downloaded.iter().map(|d| d.1.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "OEBPS/dl/files/a.png",
                "OEBPS/dl/files/a-2.png",
                "OEBPS/dl/files/z.bin"
            ]
        );
        assert_eq!(rep.failed.len(), 2);
        assert!(
            rep.failed
                .iter()
                .any(|(u, why)| u.ends_with("big.png") && why == "larger than 50 bytes")
        );
        assert!(
            rep.failed
                .iter()
                .any(|(u, why)| u.ends_with("empty.png") && why == "empty response")
        );
        assert_eq!(
            b.package
                .item_by_path("OEBPS/dl/files/z.bin")
                .unwrap()
                .media_type,
            "application/octet-stream"
        );
        assert_eq!(ext_for(Some("image/svg+xml"), "x"), "svg");
        assert_eq!(ext_for(None, "http://a/b.JPG?x"), "jpg");
        assert_eq!(ext_for(None, "http://a/b.toolongext"), "bin");
        assert_eq!(local_name_for("http://a/%%%.png", "png"), "resource.png");
    }

    #[test]
    fn broken_links_found_and_repaired() {
        let mut b = make_book(&[
            "<h1 id=\"Top\">T</h1><p><a href=\"Chapter-2.XHTML\">case</a> <a href=\"chapter-2.xhtml#SEC\">frag case</a> <a href=\"#top\">self frag case</a> <a href=\"nothere.xhtml\">gone</a> <a href=\"chapter-2.xhtml#zzz\">no frag</a> <a href=\"sub/chapter-2.xhtml\">other dir</a> <img src=\"IMAGES/p.PNG\" alt=\"\"/> <a href=\"chapter-2.xhtml\">fine</a></p>",
            "<h2 id=\"sec\">S</h2>",
        ]);
        b.archive.set("OEBPS/images/p.png", png(2, 2));
        let found = find_broken_links(&b);
        assert_eq!(found.len(), 7, "{found:#?}");
        assert!(
            found.iter().any(
                |f| f.href == "chapter-2.xhtml#zzz" && f.reason == "no element with id \"zzz\""
            )
        );
        let (repairs, remaining) = repair_broken_links(&mut b);
        assert_eq!(repairs.len(), 5, "{repairs:#?}");
        let ch = b.archive.get_string("OEBPS/chapter-1.xhtml").unwrap();
        assert!(ch.contains("href=\"chapter-2.xhtml\">case"));
        assert!(ch.contains("href=\"chapter-2.xhtml#sec\">frag case"));
        assert!(ch.contains("href=\"#Top\">self frag case"));
        assert!(ch.contains("src=\"images/p.png\""));
        assert!(ch.contains("href=\"chapter-2.xhtml\">other dir"));
        let rem: Vec<&str> = remaining.iter().map(|r| r.href.as_str()).collect();
        assert_eq!(rem, vec!["nothere.xhtml", "chapter-2.xhtml#zzz"]);
        // idempotent
        let (again, _) = repair_broken_links(&mut b);
        assert!(again.is_empty());
        assert_eq!(
            repairs
                .iter()
                .find(|r| r.old == "Chapter-2.XHTML")
                .unwrap()
                .how,
            "fixed the letter case of the file name"
        );
        assert_eq!(
            repairs
                .iter()
                .find(|r| r.old == "sub/chapter-2.xhtml")
                .unwrap()
                .how,
            "pointed at the file of the same name"
        );
    }

    #[test]
    fn ambiguous_same_name_is_not_guessed() {
        let mut b = make_book(&["<p><a href=\"x/dup.xhtml\">d</a></p>"]);
        b.archive.set_string("OEBPS/a/dup.xhtml", "<html/>");
        b.archive.set_string("OEBPS/b/dup.xhtml", "<html/>");
        let (repairs, remaining) = repair_broken_links(&mut b);
        assert!(repairs.is_empty());
        assert_eq!(remaining.len(), 1);
    }

    #[test]
    fn move_file_checks_and_repairs() {
        let mut b = make_book(&["<p><img src=\"p.png\" alt=\"\"/></p>"]);
        b.archive.set("OEBPS/p.png", png(2, 2));
        let pkg = b.package.clone();
        pkg.add_manifest_item(&mut b.archive, "p", "OEBPS/p.png", "image/png", "")
            .unwrap();
        b.reload_package().unwrap();
        assert!(
            move_file(&mut b, "OEBPS/nope.png", "OEBPS/x.png")
                .unwrap_err()
                .to_string()
                .contains("does not exist")
        );
        assert!(
            move_file(&mut b, "OEBPS/p.png", "OEBPS/style.css")
                .unwrap_err()
                .to_string()
                .contains("already exists")
        );
        let opf = b.package.opf_path.clone();
        assert!(
            move_file(&mut b, &opf, "OEBPS/x.opf")
                .unwrap_err()
                .to_string()
                .contains("cannot be moved")
        );
        assert_eq!(move_file(&mut b, "OEBPS/p.png", "OEBPS/p.png").unwrap(), 0);
        assert!(move_file(&mut b, "OEBPS/p.png", "OEBPS/img/../media/q.png").unwrap() >= 2);
        assert!(b.archive.contains("OEBPS/media/q.png"));
        assert!(
            b.archive
                .get_string("OEBPS/chapter-1.xhtml")
                .unwrap()
                .contains("src=\"media/q.png\"")
        );
        assert_eq!(b.package.item_by_id("p").unwrap().href, "media/q.png");
        assert!(
            check_book(&b.archive)
                .iter()
                .all(|i| i.severity != Severity::Error)
        );
    }

    #[test]
    fn groups() {
        assert_eq!(group_for("application/xhtml+xml", "a"), FileGroup::Text);
        assert_eq!(group_for("", "a.HTML"), FileGroup::Text);
        assert_eq!(group_for("text/css", "a"), FileGroup::Styles);
        assert_eq!(group_for("image/svg+xml", "a.svg"), FileGroup::Images);
        assert_eq!(
            group_for("application/vnd.ms-opentype", "a"),
            FileGroup::Fonts
        );
        assert_eq!(group_for("", "a.woff2"), FileGroup::Fonts);
        assert_eq!(group_for("audio/mpeg", "a"), FileGroup::Audio);
        assert_eq!(group_for("", "a.mp4"), FileGroup::Video);
        assert_eq!(
            group_for("application/octet-stream", "a.bin"),
            FileGroup::Misc
        );
        let o = ArrangeOptions::default();
        assert_eq!(o.folder(FileGroup::Fonts), "Fonts");
        assert_eq!(o.folder(FileGroup::Misc), "Misc");
    }

    pub(crate) fn messy_book() -> OpenBook {
        let mut b = make_book(&[
            "<h1 id=\"a\">One</h1><p><img src=\"pics/p.png\" alt=\"p\"/><a href=\"chapter-2.xhtml#b\">to two</a></p><svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"><image xlink:href=\"pics/p.png\" width=\"2\" height=\"2\"/></svg>",
            "<h2 id=\"b\">Two</h2><p style=\"background:url(pics/p.png)\">x <a href=\"chapter-1.xhtml#a\">back</a> <a href=\"https://e.example/p.png\">ext</a></p>",
        ]);
        b.archive.set("OEBPS/pics/p.png", png(3, 3));
        b.archive.set_string("OEBPS/style.css", "@import url(more.css);\nbody { background: url(\"pics/p.png\") }\n@font-face { font-family: F; src: url(f/x.ttf) }\n");
        b.archive
            .set_string("OEBPS/more.css", "p { background: url(pics/p.png) }");
        b.archive.set("OEBPS/f/x.ttf", vec![0, 1, 2]);
        b.archive.set("OEBPS/audio.mp3", vec![9]);
        let pkg = b.package.clone();
        for (id, f, m) in [
            ("p", "pics/p.png", "image/png"),
            ("more", "more.css", "text/css"),
            ("fx", "f/x.ttf", "font/ttf"),
            ("au", "audio.mp3", "audio/mpeg"),
        ] {
            pkg.add_manifest_item(&mut b.archive, id, &format!("OEBPS/{f}"), m, "")
                .unwrap();
        }
        b.reload_package().unwrap();
        b
    }

    #[test]
    fn arrange_files_moves_and_repairs_everything() {
        let mut b = messy_book();
        assert!(
            check_book(&b.archive)
                .iter()
                .all(|i| i.severity != Severity::Error),
            "{:?}",
            check_book(&b.archive)
        );
        let text_before: Vec<String> = b
            .spine_paths()
            .iter()
            .map(|p| normalized_text(&b.archive.get_string(p).unwrap()))
            .collect();
        let toc_before: Vec<String> = b.toc().iter().map(|t| t.title.clone()).collect();
        let rep = arrange_files(&mut b, &ArrangeOptions::default()).unwrap();
        assert!(!rep.moved.is_empty());
        for (from, to) in [
            ("OEBPS/chapter-1.xhtml", "OEBPS/Text/chapter-1.xhtml"),
            ("OEBPS/nav.xhtml", "OEBPS/Text/nav.xhtml"),
            ("OEBPS/pics/p.png", "OEBPS/Images/p.png"),
            ("OEBPS/style.css", "OEBPS/Styles/style.css"),
            ("OEBPS/more.css", "OEBPS/Styles/more.css"),
            ("OEBPS/f/x.ttf", "OEBPS/Fonts/x.ttf"),
            ("OEBPS/audio.mp3", "OEBPS/Audio/audio.mp3"),
        ] {
            assert!(
                rep.moved.contains(&(from.to_string(), to.to_string())),
                "{from}"
            );
            assert!(b.archive.contains(to) && !b.archive.contains(from));
        }
        assert!(b.archive.contains("OEBPS/toc.ncx"), "NCX stays");
        assert!(b.archive.contains("OEBPS/content.opf"));
        // references, each format
        let ch = b.archive.get_string("OEBPS/Text/chapter-1.xhtml").unwrap();
        assert!(ch.contains("src=\"../Images/p.png\""), "{ch}");
        assert!(ch.contains("xlink:href=\"../Images/p.png\""));
        assert!(ch.contains("href=\"chapter-2.xhtml#b\""));
        assert!(ch.contains("href=\"../Styles/style.css\""));
        let c2 = b.archive.get_string("OEBPS/Text/chapter-2.xhtml").unwrap();
        assert!(c2.contains("url(../Images/p.png)"));
        assert!(c2.contains("https://e.example/p.png"));
        let css = b.archive.get_string("OEBPS/Styles/style.css").unwrap();
        assert!(
            css.contains("@import url(more.css)"),
            "same folder keeps the short path: {css}"
        );
        assert!(css.contains("url(\"../Images/p.png\")"));
        assert!(css.contains("url(../Fonts/x.ttf)"));
        assert!(
            b.archive
                .get_string("OEBPS/Styles/more.css")
                .unwrap()
                .contains("url(../Images/p.png)")
        );
        let ncx = b.archive.get_string("OEBPS/toc.ncx").unwrap();
        assert!(ncx.contains("src=\"Text/chapter-1.xhtml\""), "{ncx}");
        let nav = b.archive.get_string("OEBPS/Text/nav.xhtml").unwrap();
        assert!(nav.contains("href=\"chapter-1.xhtml\""));
        let opf = b.archive.get_string("OEBPS/content.opf").unwrap();
        assert!(
            opf.contains("href=\"Text/chapter-1.xhtml\"")
                && opf.contains("href=\"Images/p.png\"")
                && opf.contains("href=\"Fonts/x.ttf\"")
        );
        // semantics preserved
        assert!(
            check_book(&b.archive)
                .iter()
                .all(|i| i.severity != Severity::Error),
            "{:?}",
            check_book(&b.archive)
        );
        let text_after: Vec<String> = b
            .spine_paths()
            .iter()
            .map(|p| normalized_text(&b.archive.get_string(p).unwrap()))
            .collect();
        assert_eq!(text_before, text_after);
        assert_eq!(
            toc_before,
            b.toc().iter().map(|t| t.title.clone()).collect::<Vec<_>>()
        );
        assert!(
            b.toc()
                .iter()
                .all(|t| b.archive.contains(t.href.split('#').next().unwrap()))
        );
        // idempotent
        assert!(
            arrange_files(&mut b, &ArrangeOptions::default())
                .unwrap()
                .moved
                .is_empty()
        );
        let reopened = OpenBook::from_archive(
            bookx_core::archive::Archive::from_bytes(b.archive.to_bytes().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(reopened.spine_paths().len(), 2);
    }

    #[test]
    fn arrange_handles_name_clashes_and_custom_folders() {
        let mut b =
            make_book(&["<p><img src=\"a/p.png\" alt=\"\"/><img src=\"b/p.png\" alt=\"\"/></p>"]);
        b.archive.set("OEBPS/a/p.png", png(2, 2));
        b.archive.set("OEBPS/b/p.png", png(3, 3));
        let pkg = b.package.clone();
        pkg.add_manifest_item(&mut b.archive, "pa", "OEBPS/a/p.png", "image/png", "")
            .unwrap();
        pkg.add_manifest_item(&mut b.archive, "pb", "OEBPS/b/p.png", "image/png", "")
            .unwrap();
        b.reload_package().unwrap();
        let opts = ArrangeOptions {
            images: "gfx/all".into(),
            text: "".into(),
            ..Default::default()
        };
        let rep = arrange_files(&mut b, &opts).unwrap();
        assert!(rep.moved.contains(&(
            "OEBPS/a/p.png".to_string(),
            "OEBPS/gfx/all/p.png".to_string()
        )));
        assert!(rep.moved.contains(&(
            "OEBPS/b/p.png".to_string(),
            "OEBPS/gfx/all/p-2.png".to_string()
        )));
        let ch = b.archive.get_string("OEBPS/chapter-1.xhtml").unwrap();
        assert!(ch.contains("src=\"gfx/all/p.png\"") && ch.contains("src=\"gfx/all/p-2.png\""));
        assert_eq!(
            b.archive.get("OEBPS/gfx/all/p-2.png").unwrap(),
            png(3, 3).as_slice()
        );
        assert_eq!(group_counts(&b)[&FileGroup::Images], 2);
    }

    #[test]
    fn remote_resources_and_destination() {
        let mut b = crate::testutil::make_book(&[
            "<p><img src=\"https://x.test/a.png\" alt=\"\"/><a href=\"https://x.test/page\">l</a><img src=\"https://x.test/a.png\" alt=\"\"/></p>",
        ]);
        let css = crate::markup::css_files(&b.archive).remove(0);
        b.archive.set_string(
            &css,
            "@import 'https://x.test/more.css';\np { background: url(\"https://x.test/bg.jpg\") }\nq { background: url(local.png) }",
        );
        let mut found = remote_resources(&b);
        found.sort();
        assert_eq!(
            found,
            vec![
                "https://x.test/a.png".to_string(),
                "https://x.test/bg.jpg".to_string(),
                "https://x.test/more.css".to_string()
            ]
        );
        assert_eq!(
            remote_resources_in_css("a{b:url(http://h/z.gif)}"),
            vec!["http://h/z.gif"]
        );
        let from = b.spine_paths()[0].clone();
        let base = b.package.base_dir().to_string();
        assert_eq!(
            destination_in_folder(&b, &from, "text/"),
            format!("{base}/text/{}", file_name(&from)).trim_start_matches('/')
        );
        assert_eq!(
            destination_in_folder(&b, &from, ""),
            format!("{base}/{}", file_name(&from)).trim_start_matches('/')
        );
    }
}
