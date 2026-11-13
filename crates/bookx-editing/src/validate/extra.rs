//! The extended checks of Bookx (switched on by [`Options::extended`]): problems an editor wants to point out that
//! the epubcheck-style rules of the validator do not cover, in the spirit of Calibre's *Check book* (empty files,
//! encodings, entities, ids, bare text, CSS declarations, fonts, images, links, unreferenced files, package
//! sections) plus the optional accessibility checks ([`Options::accessibility`]).
//!
//! The ids are `BKX-xxx` (and `ACC-9xx` for accessibility). A rule is an `ERROR` only where epubveri also fails the
//! book for the same problem (declared encoding, undefined entity, CSS syntax, malformed or `file:` addresses, a link
//! to something that is not a content document, a package without a section); the oracle test in `tests_extra.rs`
//! compares the verdicts. Everything else is a warning or usage hint.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use roxmltree::{Document, Node};

use super::package::{node_pos, prepare_xml};
use super::{offset_pos, r, resolve, Cx, Item, Pos, Rule, Severity, Target};
use super::{Severity::Error as E, Severity::Usage as U, Severity::Warning as W};

/// What [`super::validate_with`] checks on top of the classic rules.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    /// The `BKX-` checks.
    pub extended: bool,
    /// The accessibility checks (`ACC-9xx`, and the usage-level `ACC-004`/`ACC-007` are always reported).
    pub accessibility: bool,
}

impl Options {
    /// Everything except the accessibility checks.
    pub fn extended() -> Options {
        Options {
            extended: true,
            accessibility: false,
        }
    }
}

/// HTML documents larger than this are slow to open on small devices.
pub const MAX_HTML_BYTES: usize = 260 * 1024;

pub const EXTRA_RULES: &[Rule] = &[
    r("BKX-001", W, "the file is empty"),
    r("BKX-002", W, "the file cannot be decoded as UTF-8 text"),
    r(
        "BKX-003",
        E,
        "the file declares an encoding other than UTF-8",
    ),
    r("BKX-004", E, "a named character entity is used"),
    r(
        "BKX-005",
        W,
        "a character reference is malformed or unknown",
    ),
    r("BKX-006", W, "a private entity is used"),
    r("BKX-007", W, "the document is very large"),
    r("BKX-008", W, "the id is not a valid XML name"),
    r("BKX-009", W, "text directly inside body"),
    r("BKX-010", E, "a CSS declaration is malformed"),
    r("BKX-011", W, "unknown CSS property"),
    r("BKX-012", W, "the font cannot be read"),
    r("BKX-013", W, "the font's license forbids embedding"),
    r(
        "BKX-014",
        U,
        "the font-face family differs from the font's own name",
    ),
    r("BKX-015", W, "the image cannot be read"),
    r("BKX-016", W, "the JPEG image uses the CMYK colour model"),
    r("BKX-017", W, "the link points to a folder"),
    r("BKX-018", E, "the link address is malformed"),
    r(
        "BKX-019",
        W,
        "the link contains a character that is not allowed",
    ),
    r("BKX-020", E, "the link points to a file on the computer"),
    r("BKX-021", W, "the link has an empty address"),
    r(
        "BKX-022",
        W,
        "the link differs from the file name in upper/lower case",
    ),
    r(
        "BKX-023",
        E,
        "the link points to a file that is not a text document",
    ),
    r("BKX-024", W, "the file is not used anywhere in the book"),
    r("BKX-025", W, "the document is not in the reading order"),
    r(
        "BKX-026",
        W,
        "a bookmarks file of another program is left in the book",
    ),
    r(
        "BKX-027",
        E,
        "the package lacks a metadata, manifest or spine section",
    ),
    r("BKX-028", W, "the book declares more than one cover"),
    r("ACC-901", U, "a heading level is skipped"),
    r("ACC-902", U, "an epub:type has no matching ARIA role"),
    r("ACC-903", U, "the book has no accessibility metadata"),
    r("ACC-904", U, "the book has no accessibility summary"),
    r("ACC-905", U, "an inline SVG image has no text alternative"),
];

/// Container paths in different forms, for link checks.
#[derive(Debug, Default)]
pub(crate) struct NameIndex {
    pub dirs: HashSet<String>,
    pub lower: HashMap<String, Vec<String>>,
}

impl NameIndex {
    pub fn new(names: &[String]) -> NameIndex {
        let mut ix = NameIndex::default();
        for n in names {
            let mut dir = bookx_core::util::parent_dir(n);
            while !dir.is_empty() && ix.dirs.insert(dir.to_string()) {
                dir = bookx_core::util::parent_dir(dir);
            }
            ix.lower
                .entry(n.to_lowercase())
                .or_default()
                .push(n.clone());
        }
        ix
    }
}

static XML_DECL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)^\s*<\?xml[^>]*?encoding\s*=\s*["']([^"']+)["']"#).expect("static regex")
});
static META_CHARSET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<meta[^>]*?charset\s*=\s*["']?([A-Za-z0-9_\-:.]+)"#).expect("static regex")
});
static MASK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)<!--.*?-->|<!\[CDATA\[.*?\]\]>|<script\b.*?</script>|<style\b.*?</style>")
        .expect("static regex")
});
static PRIVATE_ENTITY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<!ENTITY\s+([A-Za-z_][\w.\-]*)\s+("[^"]*"|'[^']*')"#).expect("static regex")
});
static VALID_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_.\-]*$").expect("static regex"));

const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";
const SVG_NS: &str = "http://www.w3.org/2000/svg";
const OPS_NS: &str = "http://www.idpf.org/2007/ops";
const MAX_PER_RULE: usize = 100;

fn is_xhtml(n: Node) -> bool {
    n.tag_name().namespace() == Some(XHTML_NS)
}

/// The declared encoding of a markup file (XML declaration or `<meta charset>`), lowercase.
pub fn declared_encoding(bytes: &[u8]) -> Option<String> {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).into_owned();
    XML_DECL
        .captures(&head)
        .or_else(|| META_CHARSET.captures(&head))
        .map(|c| c[1].trim().to_ascii_lowercase())
}

pub fn is_utf8_label(label: &str) -> bool {
    matches!(label, "utf-8" | "utf8")
}

fn is_text_file(path: &str) -> bool {
    matches!(
        bookx_core::util::extension(path).as_str(),
        "xhtml" | "html" | "htm" | "xht" | "opf" | "ncx" | "xml" | "svg"
    )
}

fn is_font_item(it: &Item) -> bool {
    let ext = bookx_core::util::extension(&it.path);
    matches!(
        it.media_type.as_str(),
        "font/ttf"
            | "font/otf"
            | "font/sfnt"
            | "application/x-font-ttf"
            | "application/x-font-otf"
            | "application/x-font-truetype"
            | "application/x-font-opentype"
            | "application/vnd.ms-opentype"
            | "application/font-sfnt"
    ) || matches!(ext.as_str(), "ttf" | "otf" | "ttc")
}

/// The number of colour components of a JPEG (3 = YCbCr/RGB, 4 = CMYK), read from the start-of-frame marker.
pub fn jpeg_components(data: &[u8]) -> Option<u8> {
    if !data.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut i = 2;
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let m = data[i + 1];
        if m == 0xFF {
            i += 1;
            continue;
        }
        if m == 0xD8 || m == 0x01 || (0xD0..=0xD7).contains(&m) {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        if matches!(m, 0xC0..=0xCF) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
            return data.get(i + 9).copied();
        }
        i += 2 + len.max(2);
    }
    None
}

/// What is wrong with an image file, if anything: `Err(true)` for CMYK, `Err(false)` for unreadable.
fn image_problem(data: &[u8]) -> Option<(&'static str, String)> {
    if jpeg_components(data) == Some(4) {
        return Some((
            "BKX-016",
            "the JPEG image uses CMYK colours, which many readers show with wrong colours or not at all".into(),
        ));
    }
    let png = data.starts_with(&[0x89, b'P', b'N', b'G']);
    let jpeg = data.starts_with(&[0xFF, 0xD8]);
    if png
        && !data[data.len().saturating_sub(64)..]
            .windows(4)
            .any(|w| w == b"IEND")
    {
        return Some(("BKX-015", "the PNG image is cut off".into()));
    }
    if jpeg {
        let trimmed = data
            .iter()
            .rposition(|b| *b != 0 && !b.is_ascii_whitespace())
            .map(|p| &data[..=p])
            .unwrap_or(data);
        if !trimmed.ends_with(&[0xFF, 0xD9]) {
            return Some(("BKX-015", "the JPEG image is cut off".into()));
        }
    }
    let reader = image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format();
    match reader.map(|r| r.into_dimensions()) {
        Ok(Ok(_)) => None,
        Ok(Err(e)) => Some(("BKX-015", format!("the image cannot be decoded: {e}"))),
        Err(e) => Some(("BKX-015", format!("the image cannot be read: {e}"))),
    }
}

// ---------------------------------------------------------------------------
// whole-container checks
// ---------------------------------------------------------------------------

pub(super) fn run(cx: &mut Cx) {
    let Some(archive) = cx.archive else { return };
    let names: Vec<String> = archive.names().to_vec();
    for n in &names {
        if n.ends_with('/') {
            continue;
        }
        let Some(data) = archive.get(n) else { continue };
        if data.is_empty() && n != "mimetype" {
            cx.emit("BKX-001", n, None, format!("\"{n}\" is empty"));
        }
        if bookx_core::util::file_name(n) == "calibre_bookmarks.txt" {
            cx.emit(
                "BKX-026",
                n,
                None,
                "calibre_bookmarks.txt is a bookmarks file of another program and does not belong in the book",
            );
        }
        if is_text_file(n) && !data.is_empty() {
            check_encoding(cx, n, data);
        }
    }
    let items = cx.items.clone();
    for it in &items {
        if it.path.is_empty() || !cx.contains(&it.path) || cx.encrypted.contains(&it.path) {
            continue;
        }
        let data = cx.get(&it.path).unwrap_or_default();
        let is_doc = it.media_type == "application/xhtml+xml" || it.media_type == "text/html";
        if is_doc && data.len() > MAX_HTML_BYTES {
            cx.emit(
                "BKX-007",
                &it.path,
                None,
                format!(
                    "\"{}\" is {} KB; many reading devices are slow or fail above {} KB",
                    it.path,
                    data.len() / 1024,
                    MAX_HTML_BYTES / 1024
                ),
            );
        }
        if is_doc {
            let text = String::from_utf8_lossy(data).into_owned();
            check_entities(cx, &it.path, &text);
        }
        if is_font_item(it) {
            check_font(cx, it, data);
        }
        if it.media_type.starts_with("image/") && it.media_type != "image/svg+xml" {
            if let Some((id, text)) = image_problem(data) {
                cx.emit(id_static(id), &it.path, None, text);
            }
        }
    }
    check_font_aliases(cx, &items);
    check_unreferenced(cx, &items);
    check_opf(cx);
}

fn check_entities(cx: &mut Cx, path: &str, text: &str) {
    let mut count: HashMap<&'static str, usize> = HashMap::new();
    // an XHTML 1.x document type defines the named entities; any other one (HTML5's) does not
    let dtd = text.contains("-//W3C//DTD XHTML");
    for hit in scan_entities(text) {
        let n = count.entry(hit.id).or_default();
        *n += 1;
        if *n > MAX_PER_RULE {
            continue;
        }
        let msg = match hit.id {
            "BKX-004" => format!(
                "the named entity &{}; is not understood by every reading system; use the character itself or a numeric reference",
                hit.name
            ),
            "BKX-006" => format!(
                "the private entity &{}; is defined in the document type",
                hit.name
            ),
            _ => format!("\"{}\" is not a valid character reference", hit.name),
        };
        let pos = offset_pos(text, hit.offset);
        if hit.id == "BKX-004" && dtd {
            cx.emit_as(hit.id, Severity::Warning, path, pos, msg);
        } else {
            cx.emit(hit.id, path, pos, msg);
        }
    }
}

fn id_static(id: &str) -> &'static str {
    EXTRA_RULES
        .iter()
        .find(|r| r.id == id)
        .map(|r| r.id)
        .unwrap_or("BKX-015")
}

fn check_encoding(cx: &mut Cx, path: &str, data: &[u8]) {
    if data.starts_with(&[0xFF, 0xFE]) || data.starts_with(&[0xFE, 0xFF]) {
        cx.emit("BKX-003", path, None, "the file is encoded as UTF-16");
        return;
    }
    let declared = declared_encoding(data);
    if let Some(d) = declared.as_deref().filter(|d| !is_utf8_label(d)) {
        cx.emit(
            "BKX-003",
            path,
            None,
            format!("the file declares the encoding \"{d}\" instead of UTF-8"),
        );
        return;
    }
    if let Err(e) = std::str::from_utf8(data) {
        let pos = offset_pos(&String::from_utf8_lossy(data), e.valid_up_to());
        cx.emit(
            "BKX-002",
            path,
            pos,
            "the file contains bytes that are not valid UTF-8 text",
        );
    }
}

fn check_font(cx: &mut Cx, it: &Item, data: &[u8]) {
    if data.starts_with(b"wOFF") || data.starts_with(b"wOF2") {
        return;
    }
    match ttf_parser::Face::parse(data, 0) {
        Err(e) => cx.emit(
            "BKX-012",
            &it.path,
            None,
            format!("the font \"{}\" cannot be read: {e}", it.path),
        ),
        Ok(face) => {
            if matches!(
                face.permissions(),
                Some(ttf_parser::os2::Permissions::Restricted)
            ) {
                cx.emit(
                    "BKX-013",
                    &it.path,
                    None,
                    format!(
                        "the license of the font \"{}\" does not allow embedding it in a book",
                        it.path
                    ),
                );
            }
        }
    }
}

static FONT_FACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)@font-face\s*\{([^}]*)\}").expect("static regex"));
static FACE_FAMILY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)font-family\s*:\s*("[^"]*"|'[^']*'|[^;}]+)"#).expect("static regex")
});
static FACE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)url\(\s*(?:"([^"]*)"|'([^']*)'|([^)\s]*))\s*\)"#).expect("static regex")
});

fn check_font_aliases(cx: &mut Cx, items: &[Item]) {
    for it in items.iter().filter(|i| i.media_type == "text/css") {
        let Some(css) = cx.text(&it.path) else {
            continue;
        };
        for m in FONT_FACE.captures_iter(&css) {
            let body = &m[1];
            let (Some(fam), Some(url)) = (FACE_FAMILY.captures(body), FACE_URL.captures(body))
            else {
                continue;
            };
            let family = fam[1]
                .trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .trim()
                .to_lowercase();
            let href = url
                .get(1)
                .or(url.get(2))
                .or(url.get(3))
                .map(|m| m.as_str())
                .unwrap_or("");
            let Target::Local(path, _) = resolve(&it.path, href) else {
                continue;
            };
            let Some(data) = cx.get(&path) else { continue };
            let Ok(face) = ttf_parser::Face::parse(data, 0) else {
                continue;
            };
            let mut internal: Vec<String> = Vec::new();
            for n in face.names() {
                if matches!(n.name_id, 1 | 4 | 16) {
                    if let Some(s) = n.to_string() {
                        internal.push(s.to_lowercase());
                    }
                }
            }
            if !internal.is_empty() && !internal.contains(&family) {
                let off = m.get(0).map(|x| x.start()).unwrap_or(0);
                cx.emit(
                    "BKX-014",
                    &it.path,
                    offset_pos(&css, off),
                    format!(
                        "the @font-face family \"{}\" is not the name inside the font file (\"{}\")",
                        fam[1].trim().trim_matches(|c| c == '"' || c == '\''),
                        internal.first().cloned().unwrap_or_default()
                    ),
                );
            }
        }
    }
}

fn check_unreferenced(cx: &mut Cx, items: &[Item]) {
    let fallbacks: HashSet<&str> = items.iter().filter_map(|i| i.fallback.as_deref()).collect();
    let cover_id: Option<String> = cx.cover_meta.clone();
    for it in items {
        if it.path.is_empty() || !cx.contains(&it.path) || cx.encrypted.contains(&it.path) {
            continue;
        }
        if it.has_prop("nav")
            || it.has_prop("cover-image")
            || it.media_type == "application/x-dtbncx+xml"
            || it.path == cx.opf_path
            || cover_id.as_deref() == Some(it.id.as_str())
            || fallbacks.contains(it.id.as_str())
        {
            continue;
        }
        let is_doc = it.media_type == "application/xhtml+xml" || it.media_type == "text/html";
        let in_spine = cx.spine_paths.contains(&it.path);
        if is_doc {
            if !in_spine {
                cx.emit(
                    "BKX-025",
                    &it.path,
                    it.pos,
                    format!(
                        "\"{}\" is in the book but not in the reading order",
                        it.path
                    ),
                );
            }
        } else if !in_spine && !cx.referenced.contains(&it.path) {
            cx.emit(
                "BKX-024",
                &it.path,
                it.pos,
                format!("\"{}\" is not used by any page or style sheet", it.path),
            );
        }
    }
}

fn check_opf(cx: &mut Cx) {
    let opf = cx.opf_path.clone();
    if opf.is_empty() || !cx.contains(&opf) {
        return;
    }
    let text = cx.text(&opf).unwrap_or_default();
    let t = prepare_xml(&text).into_owned();
    let Ok(doc) = Document::parse_with_options(
        &t,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    ) else {
        return;
    };
    let root = doc.root_element();
    if root.tag_name().name() != "package" {
        return;
    }
    for section in ["metadata", "manifest", "spine"] {
        if !root
            .children()
            .any(|n| n.is_element() && n.tag_name().name() == section)
        {
            cx.emit(
                "BKX-027",
                &opf,
                node_pos(&doc, root),
                format!("the package document has no <{section}> section"),
            );
        }
    }
    let covers = doc
        .descendants()
        .filter(|n| {
            n.is_element() && n.tag_name().name() == "meta" && n.attribute("name") == Some("cover")
        })
        .count();
    let cover_items = cx
        .items
        .iter()
        .filter(|i| i.has_prop("cover-image"))
        .count();
    if covers > 1 || cover_items > 1 {
        cx.emit(
            "BKX-028",
            &opf,
            None,
            if covers > 1 {
                format!("the package has {covers} <meta name=\"cover\"> entries")
            } else {
                format!("{cover_items} manifest items have the cover-image property")
            },
        );
    }
    if cx.opts.accessibility && cx.major >= 3 {
        let props: Vec<&str> = doc
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "meta")
            .filter_map(|n| n.attribute("property"))
            .collect();
        let has = |name: &str| props.iter().any(|p| p.ends_with(name));
        let missing: Vec<&str> = ["accessMode", "accessibilityFeature", "accessibilityHazard"]
            .into_iter()
            .filter(|n| !has(n))
            .collect();
        if missing.len() == 3 {
            cx.emit(
                "ACC-903",
                &opf,
                None,
                "the package declares no accessibility metadata (accessMode, accessibilityFeature, accessibilityHazard)",
            );
        }
        if !has("accessibilitySummary") {
            cx.emit(
                "ACC-904",
                &opf,
                None,
                "the package declares no accessibility summary",
            );
        }
    }
}

// ---------------------------------------------------------------------------
// per document checks (called by the XHTML checker with the parsed document)
// ---------------------------------------------------------------------------

/// Blanks out comments, CDATA, scripts and styles, keeping every offset.
pub fn mask_markup(text: &str) -> String {
    let mut out = text.as_bytes().to_vec();
    for m in MASK.find_iter(text) {
        for b in &mut out[m.start()..m.end()] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One character reference problem found in markup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityHit {
    pub id: &'static str,
    pub offset: usize,
    pub len: usize,
    pub name: String,
}

/// Finds named, private and malformed character references in the markup `text`.
pub fn scan_entities(text: &str) -> Vec<EntityHit> {
    let masked = mask_markup(text);
    let private: HashSet<String> = PRIVATE_ENTITY
        .captures_iter(&masked)
        .map(|c| c[1].to_string())
        .collect();
    let b = masked.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'&' {
            i += 1;
            continue;
        }
        let rest = &masked[i + 1..];
        let (id, len, name) = if let Some(num) = rest.strip_prefix('#') {
            let hex = num.starts_with(['x', 'X']);
            let digits: String = num
                .chars()
                .skip(usize::from(hex))
                .take_while(|c| {
                    if hex {
                        c.is_ascii_hexdigit()
                    } else {
                        c.is_ascii_digit()
                    }
                })
                .collect();
            let used = 1 + usize::from(hex) + digits.len();
            if !digits.is_empty() && rest[used..].starts_with(';') {
                i += 1;
                continue;
            }
            ("BKX-005", used + 1, format!("&#{}", &num[..used - 1]))
        } else {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
                ("BKX-005", 1, "&".to_string())
            } else if !rest[name.len()..].starts_with(';') {
                ("BKX-005", 1 + name.len(), format!("&{name}"))
            } else if super::entities::is_xml_entity(&name) {
                i += 1;
                continue;
            } else if private.contains(&name) {
                ("BKX-006", name.len() + 2, name)
            } else if super::entities::lookup(&name).is_some() {
                ("BKX-004", name.len() + 2, name)
            } else {
                ("BKX-005", name.len() + 2, name)
            }
        };
        out.push(EntityHit {
            id,
            offset: i,
            len,
            name,
        });
        i += len.max(1);
    }
    out
}

pub(super) fn check_doc(cx: &mut Cx, path: &str, doc: &Document) {
    let mut per_rule: HashMap<&'static str, usize> = HashMap::new();
    let mut note = |id: &'static str| -> bool {
        let n = per_rule.entry(id).or_default();
        *n += 1;
        *n <= MAX_PER_RULE
    };

    let mut last_heading = 0u32;
    for n in doc.descendants().filter(|n| n.is_element()) {
        let pos = node_pos(doc, n);
        if let Some(id) = n.attribute("id") {
            if !id.is_empty() && !VALID_ID.is_match(id) && note("BKX-008") {
                cx.emit(
                    "BKX-008",
                    path,
                    pos,
                    format!("the id \"{id}\" is not a valid XML name"),
                );
            }
        }
        if let Some(style) = n.attribute("style") {
            if note("BKX-010") {
                super::css::check_declaration_list(cx, path, style, pos.map(|p| p.0).unwrap_or(1));
            }
        }
        if is_xhtml(n) {
            match n.tag_name().name() {
                "body" => {
                    for c in n.children().filter(|c| c.is_text()) {
                        if c.text().is_some_and(|t| !t.trim().is_empty()) && note("BKX-009") {
                            let p = doc.text_pos_at(c.range().start);
                            cx.emit(
                                "BKX-009",
                                path,
                                Some((p.row, p.col)),
                                "text directly inside <body> is not wrapped in a block element",
                            );
                        }
                    }
                }
                "a" | "area" => {
                    if let Some(href) = n.attribute("href") {
                        check_link(cx, path, href, pos, n.tag_name().name() == "a");
                    }
                }
                h @ ("h1" | "h2" | "h3" | "h4" | "h5" | "h6") if cx.opts.accessibility => {
                    let level = u32::from(h.as_bytes()[1] - b'0');
                    if last_heading != 0 && level > last_heading + 1 {
                        cx.emit(
                            "ACC-901",
                            path,
                            pos,
                            format!(
                                "<{h}> follows a level {last_heading} heading; level {} is skipped",
                                last_heading + 1
                            ),
                        );
                    }
                    last_heading = level;
                }
                _ => {}
            }
        }
        if cx.opts.accessibility {
            if let Some(ty) = n.attribute((OPS_NS, "type")) {
                if n.attribute("role").is_none() {
                    for tok in ty.split_whitespace() {
                        if let Some(role) = crate::accessibility::epub_type_to_role(tok) {
                            cx.emit(
                                "ACC-902",
                                path,
                                pos,
                                format!("epub:type=\"{tok}\" has no role=\"{role}\""),
                            );
                            break;
                        }
                    }
                }
            }
            if n.tag_name().namespace() == Some(SVG_NS)
                && n.tag_name().name() == "svg"
                && n.parent()
                    .is_none_or(|p| p.tag_name().namespace() != Some(SVG_NS))
            {
                let labelled = n.attribute("aria-label").is_some()
                    || n.attribute("aria-labelledby").is_some()
                    || n.attribute("aria-hidden") == Some("true")
                    || matches!(n.attribute("role"), Some("presentation" | "none"))
                    || n.children()
                        .any(|c| c.is_element() && c.tag_name().name() == "title");
                if !labelled {
                    cx.emit(
                        "ACC-905",
                        path,
                        pos,
                        "the inline SVG image has no <title> or aria-label",
                    );
                }
            }
        }
    }
}

/// Characters that must be percent-encoded in a relative link (or that show a Windows path).
pub fn link_bad_char(href: &str) -> Option<char> {
    let b = href.as_bytes();
    for (i, c) in href.char_indices() {
        if c == '%' {
            let ok = b.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                && b.get(i + 2).is_some_and(u8::is_ascii_hexdigit);
            if !ok {
                return Some('%');
            }
        } else if c.is_control()
            || matches!(
                c,
                '\\' | '<' | '>' | '"' | '|' | '^' | '`' | '{' | '}' | ' ' | ':'
            )
        {
            return Some(c);
        }
    }
    None
}

fn check_link(cx: &mut Cx, from: &str, raw: &str, pos: Pos, is_a: bool) {
    let href = raw.trim();
    if href.is_empty() {
        if is_a {
            cx.emit("BKX-021", from, pos, "the link has no address");
        }
        return;
    }
    let lower = href.to_ascii_lowercase();
    if lower.starts_with("file:") {
        cx.emit(
            "BKX-020",
            from,
            pos,
            format!("\"{href}\" points to a file on the author's computer"),
        );
        return;
    }
    match resolve(from, href) {
        Target::Remote | Target::Ignored => {
            use crate::links::{classify_link, validate_url, LinkKind};
            if classify_link(href) != LinkKind::Other {
                if let Err(why) = validate_url(href) {
                    cx.emit(
                        "BKX-018",
                        from,
                        pos,
                        format!("the address \"{href}\" is malformed: {why}"),
                    );
                }
            }
        }
        Target::Leaks => {}
        Target::Local(path, _) => {
            if href.starts_with('#') {
                return;
            }
            let path_part = href.split(['#', '?']).next().unwrap_or("");
            if let Some(c) = link_bad_char(path_part) {
                cx.emit(
                    "BKX-019",
                    from,
                    pos,
                    format!(
                        "the link \"{href}\" contains the character {c:?}, which is not allowed in a link"
                    ),
                );
                return;
            }
            if cx.contains(&path) {
                let ext = bookx_core::util::extension(&path);
                if is_a
                    && !matches!(
                        ext.as_str(),
                        "xhtml" | "html" | "htm" | "xht" | "svg" | "xml"
                    )
                {
                    cx.emit(
                        "BKX-023",
                        from,
                        pos,
                        format!("the link \"{href}\" points to \"{path}\", which is not a text document"),
                    );
                }
                return;
            }
            if path_part.ends_with('/') || cx.index.dirs.contains(&path) {
                cx.emit(
                    "BKX-017",
                    from,
                    pos,
                    format!("the link \"{href}\" points to a folder, not to a file"),
                );
                return;
            }
            if let Some(found) = cx.index.lower.get(&path.to_lowercase()) {
                if found.len() == 1 {
                    cx.emit(
                        "BKX-022",
                        from,
                        pos,
                        format!(
                            "the link \"{href}\" differs in upper/lower case from the file \"{}\"",
                            found[0]
                        ),
                    );
                }
            }
        }
    }
}
