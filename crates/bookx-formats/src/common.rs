//! Shared building blocks: the EPUB assembler, text decoding, zip helpers and an HTML to XHTML
//! serializer used by several readers.

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};

use anyhow::{bail, Context, Result};
use bookx_core::archive::Archive;
use bookx_core::convert::{create_epub, Chapter};
use bookx_core::opf::{Identifier, Metadata};
use bookx_core::util::{media_type_for, xml_escape};
use regex::Regex;
use zip::ZipArchive;

/// Largest single decompressed entry we accept (protects against zip bombs).
pub const MAX_ENTRY: u64 = 256 << 20;
/// Largest total of decompressed entries.
pub const MAX_TOTAL: u64 = 1 << 30;
/// Largest number of entries read from one archive.
pub const MAX_ENTRIES: usize = 100_000;

/// A file stored next to the chapters (path relative to `OEBPS/`, e.g. `images/a.png`).
#[derive(Debug, Clone)]
pub struct Media {
    pub path: String,
    pub data: Vec<u8>,
}

/// Everything a reader extracted from a foreign file.
#[derive(Default)]
pub struct Parts {
    pub md: Metadata,
    pub chapters: Vec<Chapter>,
    /// Cover image bytes and lower case extension.
    pub cover: Option<(Vec<u8>, String)>,
    pub media: Vec<Media>,
    /// Right to left page progression (manga).
    pub rtl: bool,
    /// Appended to the default style sheet.
    pub css: String,
    /// Extra `<meta>` / `<dc:*>` lines for the OPF metadata (already XML escaped).
    pub opf_meta: String,
}

fn chapter_file(index: usize) -> String {
    format!("chapter-{}.xhtml", index + 1)
}

/// Rewrites `href="#id"` links that point to an id living in another chapter.
pub fn fix_links(chapters: &mut [Chapter]) {
    let id_re = id_regex();
    let mut owner: HashMap<String, usize> = HashMap::new();
    let mut per_chapter: Vec<HashSet<String>> = Vec::new();
    for (i, ch) in chapters.iter().enumerate() {
        let mut set = HashSet::new();
        for c in id_re.captures_iter(&ch.body) {
            let id = c[1].to_string();
            owner.entry(id.clone()).or_insert(i);
            set.insert(id);
        }
        per_chapter.push(set);
    }
    let href_re = Regex::new(r##"href="#([^"]+)""##).expect("static regex");
    for (i, ch) in chapters.iter_mut().enumerate() {
        let own = &per_chapter[i];
        ch.body = href_re
            .replace_all(&ch.body, |c: &regex::Captures| {
                let frag = &c[1];
                if own.contains(frag) {
                    return c[0].to_string();
                }
                match owner.get(frag) {
                    Some(&j) => format!("href=\"{}#{}\"", chapter_file(j), frag),
                    None => c[0].to_string(),
                }
            })
            .into_owned();
    }
}

fn id_regex() -> Regex {
    Regex::new(r#"\bid="([^"]+)""#).expect("static regex")
}

/// Detects the image type from magic bytes.
pub fn image_ext(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        Some("gif")
    } else if data.len() > 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some("webp")
    } else if data.starts_with(b"BM") && data.len() > 14 {
        Some("bmp")
    } else {
        let head = String::from_utf8_lossy(&data[..data.len().min(512)]);
        if head.contains("<svg") {
            Some("svg")
        } else {
            None
        }
    }
}

/// Assembles the EPUB archive from the pieces a reader produced.
pub fn build(mut p: Parts) -> Result<Archive> {
    if p.chapters.is_empty() {
        bail!("the book has no readable text");
    }
    if p.md.title.trim().is_empty() {
        p.md.title = "Untitled".into();
    }
    // formats with one author field join several authors with " & " (Calibre's convention)
    p.md.authors =
        p.md.authors
            .iter()
            .flat_map(|a| {
                a.split(" & ")
                    .map(|x| x.trim().to_string())
                    .collect::<Vec<_>>()
            })
            .filter(|a| !a.is_empty())
            .collect();
    fix_links(&mut p.chapters);
    let toc = nested_toc(&mut p.chapters);
    let cover = p.cover.take();
    let cover_ref = cover.as_ref().map(|(d, e)| (d.clone(), e.as_str()));
    let mut arc = create_epub(&p.md, &p.chapters, cover_ref);

    let mut items = String::new();
    let mut seen = HashSet::new();
    for (n, m) in p.media.iter().enumerate() {
        if !seen.insert(m.path.clone()) || m.data.is_empty() {
            continue;
        }
        arc.set(&format!("OEBPS/{}", m.path), m.data.clone());
        items.push_str(&format!(
            "    <item id=\"media-{n}\" href=\"{}\" media-type=\"{}\"/>\n",
            xml_escape(&m.path),
            media_type_for(&m.path)
        ));
    }
    let mut opf = arc.get_string("OEBPS/content.opf").unwrap_or_default();
    if !items.is_empty() {
        opf = opf.replacen("  </manifest>", &format!("{items}  </manifest>"), 1);
    }
    if p.rtl {
        opf = opf.replacen(
            "<spine toc=\"ncx\">",
            "<spine toc=\"ncx\" page-progression-direction=\"rtl\">",
            1,
        );
    }
    let extra = format!("{}{}", extra_metadata_xml(&p.md), p.opf_meta);
    opf = opf.replacen("  </metadata>", &format!("{extra}  </metadata>"), 1);
    arc.set_string("OEBPS/content.opf", &opf);
    if !p.css.is_empty() {
        let css = arc.get_string("OEBPS/style.css").unwrap_or_default();
        arc.set_string("OEBPS/style.css", &format!("{css}{}\n", p.css));
    }
    if let Some(toc) = toc {
        if let Ok(book) = bookx_core::book::OpenBook::from_archive(arc.clone()) {
            let pkg = book.package.clone();
            let mut a = book.archive;
            if bookx_core::toc::write_toc(&mut a, &pkg, &toc).is_ok() {
                arc = a;
            }
        }
    }

    Ok(arc)
}

/// Builds a nested table of contents from the headings below the chapter level, so a book
/// split at its `<h1>`s still lists its `<h2>` sections (like Calibre does). Headings without an
/// `id` get one. Returns `None` when no chapter has sub-headings.
fn nested_toc(chapters: &mut [Chapter]) -> Option<Vec<bookx_core::toc::TocEntry>> {
    use bookx_core::toc::TocEntry;
    let re = Regex::new(r"(?s)<h([1-6])\b([^>]*)>(.*?)</h[1-6]>").ok()?;
    let starts_with_heading = |b: &str| {
        b.trim_start().starts_with("<h")
            && re
                .find(b)
                .is_some_and(|m| m.start() == b.len() - b.trim_start().len())
    };
    let base = chapters
        .iter()
        .filter(|c| starts_with_heading(&c.body))
        .filter_map(|c| {
            re.captures(&c.body)
                .and_then(|m| m[1].parse::<usize>().ok())
        })
        .min()
        .unwrap_or(1);
    let mut any_children = false;
    let mut out: Vec<TocEntry> = Vec::new();
    let id_re = Regex::new(r#"\bid\s*=\s*"([^"]+)""#).ok()?;
    #[allow(clippy::needless_range_loop)]
    for i in 0..chapters.len() {
        let file = format!("OEBPS/{}", chapter_file(i));
        let mut entry = TocEntry {
            title: chapters[i].title.clone(),
            href: file.clone(),
            children: Vec::new(),
        };
        let body = chapters[i].body.clone();
        let mut rewritten = String::new();
        let mut last = 0;
        let mut stack: Vec<(usize, Vec<usize>)> = Vec::new(); // (level, path of child indexes)
        let first_is_title = starts_with_heading(&body);
        let mut title_taken = false;
        for (k, m) in re.captures_iter(&body).enumerate() {
            let whole = m.get(0).expect("match");
            let level: usize = m[1].parse().unwrap_or(1);
            rewritten.push_str(&body[last..whole.start()]);
            last = whole.end();
            let attrs = m[2].to_string();
            let title = strip_tags(&m[3]);
            let is_chapter_title = !title_taken && title == strip_tags(&chapters[i].title);
            if is_chapter_title {
                title_taken = true;
            }
            if is_chapter_title || (k == 0 && first_is_title) || level <= base || title.is_empty() {
                rewritten.push_str(whole.as_str());
                continue;
            }
            let id = id_re
                .captures(&attrs)
                .map(|c| c[1].to_string())
                .unwrap_or_else(|| format!("bx-h{}-{k}", i + 1));
            if attrs.contains("id=") {
                rewritten.push_str(whole.as_str());
            } else {
                rewritten.push_str(&format!(
                    "<h{level}{attrs} id=\"{id}\">{}</h{level}>",
                    &m[3]
                ));
            }
            while stack.last().is_some_and(|(l, _)| *l >= level) {
                stack.pop();
            }
            let node = TocEntry {
                title,
                href: format!("{file}#{id}"),
                children: Vec::new(),
            };
            let mut parent = &mut entry.children;
            if let Some((_, path)) = stack.last() {
                for ix in path {
                    parent = &mut parent[*ix].children;
                }
            }
            parent.push(node);
            let mut path = stack.last().map(|(_, p)| p.clone()).unwrap_or_default();
            path.push(parent.len() - 1);
            stack.push((level, path));
            any_children = true;
        }
        rewritten.push_str(&body[last..]);
        chapters[i].body = rewritten;
        out.push(entry);
    }
    any_children.then_some(out)
}

/// Same rule as the core's `title_sort` ("The Rust Book" -> "Rust Book, The") but safe for titles
/// whose first bytes are not ASCII.
pub fn safe_title_sort(title: &str) -> String {
    let t = title.trim();
    for art in ["The ", "A ", "An "] {
        let n = art.len();
        if t.is_char_boundary(n) && t.len() > n && t[..n].eq_ignore_ascii_case(art) {
            return format!("{}, {}", &t[n..], &t[..n - 1]);
        }
    }
    t.to_string()
}

fn extra_metadata_xml(md: &Metadata) -> String {
    let mut x = String::new();
    if !md.description.is_empty() {
        x.push_str(&format!(
            "    <dc:description>{}</dc:description>\n",
            xml_escape(&md.description)
        ));
    }
    for t in md.tags.iter().filter(|t| !t.trim().is_empty()) {
        x.push_str(&format!("    <dc:subject>{}</dc:subject>\n", xml_escape(t)));
    }
    if !md.published.is_empty() {
        x.push_str(&format!(
            "    <dc:date>{}</dc:date>\n",
            xml_escape(&md.published)
        ));
    }
    if !md.rights.is_empty() {
        x.push_str(&format!(
            "    <dc:rights>{}</dc:rights>\n",
            xml_escape(&md.rights)
        ));
    }
    if !md.series.is_empty() {
        x.push_str(&format!(
            "    <meta name=\"calibre:series\" content=\"{}\"/>\n",
            xml_escape(&md.series)
        ));
        let idx = if md.series_index > 0.0 {
            md.series_index
        } else {
            1.0
        };
        x.push_str(&format!(
            "    <meta name=\"calibre:series_index\" content=\"{}\"/>\n",
            bookx_core::opf::format_index(idx)
        ));
    }
    x.push_str(&format!(
        "    <meta name=\"calibre:title_sort\" content=\"{}\"/>\n",
        xml_escape(&safe_title_sort(&md.title))
    ));
    x
}

// ---------------------------------------------------------------------------
// Text decoding
// ---------------------------------------------------------------------------

/// Decodes text of unknown encoding: UTF-8/UTF-16 BOM, valid UTF-8, BOM-less UTF-16 (detected by
/// NUL bytes), then Windows-1252 (a superset of Latin-1).
pub fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return encoding_rs::UTF_16LE
            .decode_without_bom_handling(rest)
            .0
            .into_owned();
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return encoding_rs::UTF_16BE
            .decode_without_bom_handling(rest)
            .0
            .into_owned();
    }
    let sample = &bytes[..bytes.len().min(4096)];
    let even_nuls = sample.iter().step_by(2).filter(|b| **b == 0).count();
    let odd_nuls = sample
        .iter()
        .skip(1)
        .step_by(2)
        .filter(|b| **b == 0)
        .count();
    if sample.len() >= 4 {
        if odd_nuls * 4 > sample.len() / 2 && even_nuls == 0 {
            return encoding_rs::UTF_16LE
                .decode_without_bom_handling(bytes)
                .0
                .into_owned();
        }
        if even_nuls * 4 > sample.len() / 2 && odd_nuls == 0 {
            return encoding_rs::UTF_16BE
                .decode_without_bom_handling(bytes)
                .0
                .into_owned();
        }
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    legacy_encoding(bytes)
        .decode_without_bom_handling(bytes)
        .0
        .into_owned()
}

fn script_of(c: char) -> u8 {
    match c as u32 {
        0x0400..=0x04FF => 1,
        0x0370..=0x03FF => 2,
        0x0590..=0x05FF => 3,
        0x0600..=0x06FF => 4,
        0x3040..=0x30FF | 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xAC00..=0xD7AF => 5,
        _ => 0,
    }
}

/// The most frequent letters of a script; real text in the right encoding is dominated by them,
/// the same bytes read with a wrong table are not.
fn common_letters(script: u8) -> &'static str {
    match script {
        1 => "оеаинтсрвлОЕАИНТСРВЛ",
        2 => "αοειτνσρυκςΑΟΕΙΤΝΣΡΥΚ",
        3 => "יוהאלרבמתנ",
        4 => "اليمنورتبه",
        _ => "",
    }
}

const SIMPLIFIED: &str = "的一是不了人我在有他这中大来上国个到说们为子和你地出道也时年得就那要下以生会自着去之过家学对可她里后小么心多天而能好都然没日于起还发成事只作当想看文无开手十用主行方又如前所本见经头面公同三已老从动两长知民样现分将外但身些与高意进把法此实回二理美点月明其种声全工";
const TRADITIONAL: &str = "的一是不了人我在有他這中大來上國個到說們為子和你地出道也時年得就那要下以生會自著去之過家學對可她裡後小麼心多天而能好都然沒日於起還發成事只作當想看文無開手十用主行方又如前所本見經頭面公同三已老從動兩長知民樣現分將外但身些與高意進把法此實回二理美點月明其種聲全工";

fn share_of(text: &str, set: &str) -> f64 {
    let mut n = 0usize;
    let mut total = 0usize;
    for c in text.chars().filter(|c| c.is_alphabetic()) {
        total += 1;
        if set.contains(c) {
            n += 1;
        }
    }
    if total == 0 {
        0.0
    } else {
        n as f64 / total as f64
    }
}

/// Guesses the legacy encoding of text that is not valid UTF-8. Western texts stay
/// Windows-1252 (accented letters are rare there); texts made mostly of high bytes are tried
/// as Cyrillic (Windows-1251 / KOI8-R / KOI8-U), Greek, Hebrew, Arabic and the East Asian
/// multi-byte encodings, and the candidate whose output reads like real text of its script wins.
pub fn legacy_encoding(bytes: &[u8]) -> &'static encoding_rs::Encoding {
    let sample = &bytes[..bytes.len().min(16 * 1024)];
    let default = encoding_rs::WINDOWS_1252;
    let high = sample.iter().filter(|b| **b >= 0x80).count();
    let letters = sample
        .iter()
        .filter(|b| b.is_ascii_alphabetic() || **b >= 0x80)
        .count();
    if letters == 0 || high * 100 < letters * 25 {
        return default;
    }
    // (encoding, script id; 5 = CJK)
    let candidates: [(&'static encoding_rs::Encoding, u8); 11] = [
        (encoding_rs::WINDOWS_1251, 1),
        (encoding_rs::KOI8_R, 1),
        (encoding_rs::KOI8_U, 1),
        (encoding_rs::WINDOWS_1253, 2),
        (encoding_rs::WINDOWS_1255, 3),
        (encoding_rs::WINDOWS_1256, 4),
        (encoding_rs::SHIFT_JIS, 5),
        (encoding_rs::EUC_JP, 5),
        (encoding_rs::GBK, 5),
        (encoding_rs::BIG5, 5),
        (encoding_rs::EUC_KR, 5),
    ];
    let mut best: Option<(&'static encoding_rs::Encoding, f64)> = None;
    for (enc, script) in candidates {
        let (text, errors) = enc.decode_without_bom_handling(sample);
        if errors {
            // a character cut at the end of the sample is not a real error
            let (_, e2) = enc.decode_without_bom_handling(&sample[..sample.len() - 1]);
            if e2 || script != 5 {
                continue;
            }
        }
        let alpha = text.chars().filter(|c| c.is_alphabetic()).count();
        let in_script = text.chars().filter(|c| script_of(*c) == script).count();
        if alpha == 0 || in_script * 100 < alpha * 85 {
            continue;
        }
        let score = if script == 5 {
            let kana = text
                .chars()
                .filter(|c| matches!(*c as u32, 0x3040..=0x30FF))
                .count();
            let hangul = text
                .chars()
                .filter(|c| matches!(*c as u32, 0xAC00..=0xD7AF))
                .count();
            let name = enc.name();
            let japanese = name == "Shift_JIS" || name == "EUC-JP";
            let bonus = match name {
                _ if japanese => kana as f64 / alpha as f64,
                "EUC-KR" => hangul as f64 / alpha as f64,
                "GBK" => share_of(&text, SIMPLIFIED) * 2.0,
                _ => share_of(&text, TRADITIONAL) * 2.0,
            };
            0.4 + bonus.min(0.6) + if japanese { 0.001 } else { 0.0 }
        } else {
            share_of(&text, common_letters(script))
        };
        if best.is_none_or(|(_, b)| score > b + 1e-9) {
            best = Some((enc, score));
        }
    }
    match best {
        Some((enc, score)) if score > 0.4 => enc,
        _ => default,
    }
}

/// Decodes an XML/HTML document honouring a BOM, the `encoding="..."` declaration or a
/// `<meta charset>`.
pub fn decode_xml(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF])
        || bytes.starts_with(&[0xFF, 0xFE])
        || bytes.starts_with(&[0xFE, 0xFF])
    {
        return decode_text(bytes);
    }
    let head = &bytes[..bytes.len().min(1024)];
    let head_s: String = head.iter().map(|&b| b as char).collect();
    let label = Regex::new(r#"(?i)(?:encoding|charset)\s*=\s*["']?([A-Za-z0-9_.:-]+)"#)
        .ok()
        .and_then(|re| re.captures(&head_s).map(|c| c[1].to_string()));
    let declared = label.and_then(|l| encoding_rs::Encoding::for_label(l.as_bytes()));
    if let Some(enc) = declared {
        if enc != encoding_rs::UTF_8 {
            return enc.decode_without_bom_handling(bytes).0.into_owned();
        }
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        // no declaration and not UTF-8: a legacy single or multi byte encoding
        Err(_) if declared.is_none() => legacy_encoding(bytes)
            .decode_without_bom_handling(bytes)
            .0
            .into_owned(),
        Err(_) => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Splits plain text into `<p>` paragraphs (blank-line separated, lines joined by spaces).
pub fn text_to_paragraphs(text: &str) -> String {
    let mut out = String::new();
    let mut para: Vec<&str> = Vec::new();
    for line in text.lines().chain(std::iter::once("")) {
        if line.trim().is_empty() {
            if !para.is_empty() {
                out.push_str(&format!("<p>{}</p>\n", xml_escape(&para.join(" "))));
                para.clear();
            }
        } else {
            para.push(line.trim());
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Zip helpers
// ---------------------------------------------------------------------------

/// All files of a zip, in archive order.
#[derive(Debug, Default)]
pub struct ZipFiles {
    pub files: Vec<(String, Vec<u8>)>,
}

impl ZipFiles {
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.files
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, d)| d.as_slice())
    }

    pub fn get_ci(&self, name: &str) -> Option<&[u8]> {
        self.files
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, d)| d.as_slice())
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.files.iter().map(|(n, _)| n.as_str())
    }
}

/// Reads a whole zip into memory with size guards.
pub fn read_zip(bytes: &[u8]) -> Result<ZipFiles> {
    read_zip_limited(bytes, MAX_ENTRY, MAX_TOTAL)
}

/// [`read_zip`] with explicit limits for one entry and for the sum of all entries.
pub fn read_zip_limited(bytes: &[u8], max_entry: u64, max_total: u64) -> Result<ZipFiles> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).context("not a valid zip file")?;
    if zip.len() > MAX_ENTRIES {
        bail!("zip has too many entries ({})", zip.len());
    }
    let mut out = ZipFiles::default();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let f = zip.by_index(i).context("damaged zip entry")?;
        if f.is_dir() {
            continue;
        }
        let name = f.name()?.to_string();
        if name.contains("..") && name.split('/').any(|s| s == "..") {
            continue;
        }
        let mut buf = Vec::new();
        f.take(max_entry + 1)
            .read_to_end(&mut buf)
            .with_context(|| format!("reading `{name}` from the zip"))?;
        if buf.len() as u64 > max_entry {
            bail!("`{name}` is too large");
        }
        total += buf.len() as u64;
        if total > max_total {
            bail!("zip content is too large");
        }
        out.files.push((name, buf));
    }
    Ok(out)
}

/// Entry names of a zip. If the central directory is missing (truncated head) the name of the
/// first local file header is returned alone.
pub fn zip_names(head: &[u8]) -> Option<Vec<String>> {
    if !head.starts_with(b"PK\x03\x04") {
        return None;
    }
    if let Ok(z) = ZipArchive::new(Cursor::new(head)) {
        return Some(
            z.file_names()
                .filter_map(|s| s.ok())
                .map(|s| s.to_string())
                .collect(),
        );
    }
    if head.len() >= 30 {
        let n = u16::from_le_bytes([head[26], head[27]]) as usize;
        if head.len() >= 30 + n {
            return Some(vec![String::from_utf8_lossy(&head[30..30 + n]).into_owned()]);
        }
    }
    Some(Vec::new())
}

/// Reads one named entry (bounded) from a zip given as bytes.
pub fn zip_entry(bytes: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).ok()?;
    let f = zip.by_name(name).ok()?;
    let mut buf = Vec::new();
    f.take(MAX_ENTRY).read_to_end(&mut buf).ok()?;
    Some(buf)
}

// ---------------------------------------------------------------------------
// HTML -> XHTML
// ---------------------------------------------------------------------------

const PASS_TAGS: &[&str] = &[
    "p",
    "div",
    "span",
    "a",
    "b",
    "i",
    "u",
    "em",
    "strong",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "br",
    "hr",
    "img",
    "ul",
    "ol",
    "li",
    "blockquote",
    "pre",
    "code",
    "sub",
    "sup",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "td",
    "th",
    "caption",
    "s",
    "strike",
    "del",
    "ins",
    "small",
    "big",
    "cite",
    "dl",
    "dt",
    "dd",
    "figure",
    "figcaption",
    "q",
    "abbr",
    "ruby",
    "rt",
    "rp",
    "mark",
    "kbd",
    "samp",
    "var",
    "dfn",
    "time",
    "address",
    "bdi",
    "bdo",
];
const RENAME_TAGS: &[(&str, &str)] = &[
    ("center", "div"),
    ("font", "span"),
    ("tt", "code"),
    ("section", "div"),
    ("article", "div"),
    ("aside", "div"),
    ("nav", "div"),
    ("header", "div"),
    ("footer", "div"),
    ("main", "div"),
];
const DROP_TAGS: &[&str] = &[
    "script", "style", "head", "title", "link", "meta", "iframe", "object", "embed", "form",
    "noscript", "template", "button", "select", "textarea", "input",
];
const KEEP_ATTRS: &[&str] = &[
    "id", "href", "src", "alt", "title", "colspan", "rowspan", "lang", "class", "style", "dir",
];

/// Serializes the `<body>` of (possibly sloppy) HTML as XHTML. `rewrite` may map `src` values.
pub fn html_to_xhtml(html: &str, rewrite: &dyn Fn(&str) -> Option<String>) -> String {
    html_blocks(html, rewrite).concat()
}

/// Like [`html_to_xhtml`] but returns one string per top level node of the `<body>`.
pub fn html_blocks(html: &str, rewrite: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    use scraper::{Html, Node};
    let doc = Html::parse_document(html);
    let mut blocks = Vec::new();
    let Some(body) = scraper::Selector::parse("body")
        .ok()
        .and_then(|s| doc.select(&s).next())
    else {
        return blocks;
    };
    fn push(
        node: ego_tree::NodeRef<scraper::Node>,
        blocks: &mut Vec<String>,
        rewrite: &dyn Fn(&str) -> Option<String>,
        depth: usize,
    ) {
        // wrapper elements (one `<div>` per source file, `<section>`s) that hold headings are
        // opened so the headings still split chapters
        if depth < 4 {
            if let Node::Element(e) = node.value() {
                let name = e.name().to_ascii_lowercase();
                let has_heading = node.descendants().any(|d| {
                    matches!(d.value(), Node::Element(x)
                        if matches!(x.name(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6"))
                });
                if matches!(name.as_str(), "div" | "section" | "article" | "main") && has_heading {
                    for c in node.children() {
                        push(c, blocks, rewrite, depth + 1);
                    }
                    return;
                }
            }
        }
        let mut out = String::new();
        emit_node(node, &mut out, rewrite, 0);
        if matches!(node.value(), Node::Text(_)) && out.trim().is_empty() {
            return;
        }
        if !out.is_empty() {
            blocks.push(out);
        }
    }
    for child in body.children() {
        push(child, &mut blocks, rewrite, 0);
    }
    blocks
}

fn emit_node(
    node: ego_tree::NodeRef<scraper::Node>,
    out: &mut String,
    rewrite: &dyn Fn(&str) -> Option<String>,
    depth: usize,
) {
    use scraper::Node;
    if depth > 200 {
        return;
    }
    match node.value() {
        Node::Text(t) => out.push_str(&xml_escape(t)),
        Node::Element(e) => {
            let name = e.name().to_ascii_lowercase();
            if DROP_TAGS.contains(&name.as_str()) {
                return;
            }
            let tag = if PASS_TAGS.contains(&name.as_str()) {
                Some(name.as_str())
            } else {
                RENAME_TAGS
                    .iter()
                    .find(|(from, _)| *from == name)
                    .map(|(_, to)| *to)
            };
            if name == "svg" {
                // an SVG wrapper around one picture (cover pages) becomes a plain image
                let image = node.descendants().find_map(|d| match d.value() {
                    Node::Element(x) if x.name() == "image" => x.attr("href").map(str::to_string),
                    _ => None,
                });
                if let Some(src) = image {
                    let src = rewrite(&src).unwrap_or(src);
                    out.push_str(&format!("<img src=\"{}\" alt=\"\"/>", xml_escape(&src)));
                    return;
                }
            }
            let Some(tag) = tag else {
                for c in node.children() {
                    emit_node(c, out, rewrite, depth + 1);
                }
                return;
            };
            // a block inside a phrasing element (Calibre's RB writer nests <div> in <i>) is
            // invalid XHTML: keep its content, drop the box
            if matches!(
                tag,
                "p" | "div" | "blockquote" | "ul" | "ol" | "table" | "pre" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            ) && node.ancestors().any(|a| {
                matches!(a.value(), Node::Element(x)
                    if matches!(x.name(), "span" | "b" | "i" | "u" | "em" | "strong" | "sub" | "sup" | "code" | "small" | "big" | "s" | "strike" | "del" | "ins" | "cite" | "q" | "abbr" | "font" | "tt" | "mark" | "kbd"))
            }) {
                for c in node.children() {
                    emit_node(c, out, rewrite, depth + 1);
                }
                return;
            }
            out.push('<');
            out.push_str(tag);
            if tag == "a" && e.attr("id").is_none() {
                if let Some(n) = e.attr("name").filter(|n| !n.is_empty()) {
                    out.push_str(&format!(" id=\"{}\"", xml_escape(n)));
                }
            }
            for (k, v) in e.attrs() {
                let extra = (tag == "img" && matches!(k, "width" | "height"))
                    || (tag == "ol" && k == "start");
                if !(KEEP_ATTRS.contains(&k) || extra) || (k == "href" && v.starts_with("kindle:"))
                {
                    continue;
                }
                let v = if k == "src" {
                    rewrite(v).unwrap_or_else(|| v.to_string())
                } else {
                    v.to_string()
                };
                out.push_str(&format!(" {k}=\"{}\"", xml_escape(&v)));
            }
            if tag == "img" && e.attr("alt").is_none() {
                out.push_str(" alt=\"\"");
            }
            if matches!(tag, "br" | "hr" | "img") {
                out.push_str("/>");
            } else {
                out.push('>');
                for c in node.children() {
                    emit_node(c, out, rewrite, depth + 1);
                }
                out.push_str(&format!("</{tag}>"));
            }
        }
        _ => {}
    }
}

/// Text of the HTML `<title>`, if any.
pub fn html_title(html: &str) -> Option<String> {
    let doc = scraper::Html::parse_document(html);
    let sel = scraper::Selector::parse("title").ok()?;
    let t = doc
        .select(&sel)
        .next()?
        .text()
        .collect::<String>()
        .trim()
        .to_string();
    (!t.is_empty()).then_some(t)
}

/// Number of visible characters (tags removed) in an XHTML fragment.
pub fn visible_text_len(xhtml: &str) -> usize {
    let mut n = 0;
    let mut in_tag = false;
    for c in xhtml.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            c if !in_tag && !c.is_whitespace() => n += 1,
            _ => {}
        }
    }
    n
}

/// Text content of an XHTML fragment with entities decoded and whitespace collapsed.
pub fn strip_tags(xhtml: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in xhtml.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    xml_unescape(&out)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Decodes the five XML entities and numeric character references.
pub fn xml_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ => ent
                .strip_prefix("#x")
                .or_else(|| ent.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Groups XHTML blocks into chapters, starting a new chapter at every `<h1>` (or at `<h2>` when
/// the document has no `<h1>`). Content before the first heading goes to a chapter called
/// `default_title`. Without headings the result is a single chapter.
pub fn chapters_from_blocks(blocks: Vec<String>, default_title: &str) -> Vec<Chapter> {
    let level = [1, 2].into_iter().find(|l| {
        let open = format!("<h{l}");
        blocks.iter().any(|b| is_heading_block(b, &open))
    });
    let mut chapters: Vec<Chapter> = Vec::new();
    let mut cur_title = default_title.to_string();
    let mut cur = String::new();
    for b in blocks {
        let starts = level.is_some_and(|l| is_heading_block(&b, &format!("<h{l}")));
        if starts {
            push_chapter(&mut chapters, &cur_title, &mut cur);
            cur_title = strip_tags(&b);
            if cur_title.is_empty() {
                cur_title = default_title.to_string();
            }
        }
        cur.push_str(&b);
        cur.push('\n');
    }
    push_chapter(&mut chapters, &cur_title, &mut cur);
    chapters
}

fn is_heading_block(b: &str, open: &str) -> bool {
    b.starts_with(open)
        && b[open.len()..]
            .chars()
            .next()
            .is_some_and(|c| c == '>' || c == ' ')
}

fn push_chapter(chapters: &mut Vec<Chapter>, title: &str, body: &mut String) {
    if visible_text_len(body) > 0 || body.contains("<img") {
        chapters.push(Chapter {
            title: title.to_string(),
            body: std::mem::take(body),
        });
    } else {
        body.clear();
    }
}

/// Registers media files under unique, URL-safe names.
#[derive(Default)]
pub struct MediaSet {
    pub items: Vec<Media>,
    by_key: HashMap<String, String>,
}

impl MediaSet {
    /// Adds `data` (identified by `key`, e.g. its original path) and returns the new path
    /// (`images/<name>.<ext>`). Non-image data returns `None`.
    pub fn add(&mut self, key: &str, data: Vec<u8>) -> Option<String> {
        if let Some(p) = self.by_key.get(key) {
            return Some(p.clone());
        }
        let ext = image_ext(&data)?;
        let stem = key
            .rsplit('/')
            .next()
            .unwrap_or(key)
            .rsplit_once('.')
            .map(|(s, _)| s)
            .unwrap_or(key);
        let stem = sanitize_stem(stem);
        let mut path = format!("images/{stem}.{ext}");
        let mut k = 1;
        while self.items.iter().any(|m| m.path == path) {
            k += 1;
            path = format!("images/{stem}-{k}.{ext}");
        }
        self.items.push(Media {
            path: path.clone(),
            data,
        });
        self.by_key.insert(key.to_string(), path.clone());
        Some(path)
    }

    pub fn path_of(&self, key: &str) -> Option<&str> {
        self.by_key.get(key).map(|s| s.as_str())
    }
}

/// Metadata and cover reference read from an OPF document (Calibre's `metadata.opf` in HTMLZ).
#[derive(Default)]
pub struct OpfInfo {
    pub md: Metadata,
    pub cover_href: Option<String>,
}

/// Parses the metadata of an OPF package. Independent of the core parser.
pub fn parse_opf_info(text: &str) -> Option<OpfInfo> {
    let text = text.trim_start_matches('\u{feff}');
    let doc = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .ok()?;
    let mut info = OpfInfo::default();
    let md = &mut info.md;
    let mut cover_id = None;
    let mut items: Vec<(String, String)> = Vec::new();
    for n in doc.descendants().filter(|n| n.is_element()) {
        let t = n
            .descendants()
            .filter(|d| d.is_text())
            .filter_map(|d| d.text())
            .collect::<String>()
            .trim()
            .to_string();
        match n.tag_name().name() {
            "title" if md.title.is_empty() => md.title = t,
            "creator" if !t.is_empty() => md.authors.push(t),
            "publisher" if md.publisher.is_empty() => md.publisher = t,
            "language" if md.language.is_empty() => md.language = t,
            "description" if md.description.is_empty() => md.description = t,
            "subject" if !t.is_empty() => md.tags.push(t),
            "date" if md.published.is_empty() => md.published = t,
            "rights" if md.rights.is_empty() => md.rights = t,
            "identifier" if !t.is_empty() => {
                let scheme = n
                    .attributes()
                    .find(|a| a.name() == "scheme")
                    .map(|a| a.value().to_ascii_lowercase())
                    .unwrap_or_default();
                if scheme == "isbn" {
                    md.identifiers.push(Identifier {
                        scheme,
                        value: t,
                        xml_id: None,
                    });
                }
            }
            "meta" => {
                let content = n.attribute("content").unwrap_or("");
                match n.attribute("name") {
                    Some("calibre:series") => md.series = content.to_string(),
                    Some("calibre:series_index") => {
                        md.series_index = content.parse().unwrap_or(1.0)
                    }
                    Some("cover") => cover_id = Some(content.to_string()),
                    _ => {}
                }
            }
            "item" => {
                if let (Some(id), Some(h)) = (n.attribute("id"), n.attribute("href")) {
                    items.push((id.to_string(), h.to_string()));
                }
            }
            "reference" if n.attribute("type") == Some("cover") => {
                info.cover_href = n.attribute("href").map(|s| s.to_string());
            }
            _ => {}
        }
    }
    if let Some(id) = cover_id {
        if let Some((_, h)) = items.iter().find(|(i, _)| *i == id) {
            info.cover_href = Some(h.clone());
        } else if info.cover_href.is_none() {
            info.cover_href = Some(id);
        }
    }
    Some(info)
}

/// Reduces `s` to ASCII letters, digits, `-` and `_` (other characters become `_`).
pub fn sanitize_stem(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "x".into()
    } else {
        cleaned
    }
}

/// Attribute by local name, ignoring its namespace (`w:val`, `xlink:href`, ...).
pub fn attr<'a>(n: roxmltree::Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.attributes().find(|a| a.name() == name).map(|a| a.value())
}

/// Concatenated text of all descendants.
pub fn node_text(n: roxmltree::Node) -> String {
    n.descendants()
        .filter(|d| d.is_text())
        .filter_map(|d| d.text())
        .collect()
}

/// First child element with the given local name.
pub fn child_el<'a, 'i>(n: roxmltree::Node<'a, 'i>, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
    n.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
}

/// Splits a keyword list on commas and semicolons.
pub fn split_keywords(s: &str) -> Vec<String> {
    s.split([',', ';'])
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

pub const EPUB_NS_DECL: &str = "xmlns:epub=\"http://www.idpf.org/2007/ops\"";

/// A footnote reference link and the footnote itself, as XHTML.
pub fn noteref(id: &str, label: &str) -> String {
    format!(
        "<a {EPUB_NS_DECL} epub:type=\"noteref\" id=\"{0}-ref\" href=\"#{0}\">{1}</a>",
        xml_escape(id),
        xml_escape(label)
    )
}

pub fn footnote_aside(id: &str, inner: &str) -> String {
    format!(
        "<aside {EPUB_NS_DECL} id=\"{}\" epub:type=\"footnote\">\n{inner}</aside>\n",
        xml_escape(id)
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn legacy_encodings_are_detected() {
        let cases: &[(&str, &'static encoding_rs::Encoding)] = &[
            ("Все смешалось в доме Облонских. Жена узнала, что муж был в связи с бывшею в их доме француженкою-гувернанткой, и объявила мужу, что не может жить с ним в одном доме.", encoding_rs::WINDOWS_1251),
            ("Все смешалось в доме Облонских. Жена узнала, что муж был в связи с бывшею в их доме француженкою-гувернанткой, и объявила мужу, что не может жить с ним в одном доме.", encoding_rs::KOI8_R),
            ("吾輩は猫である。名前はまだ無い。どこで生れたかとんと見当がつかぬ。何でも薄暗いじめじめした所でニャーニャー泣いていた事だけは記憶している。", encoding_rs::SHIFT_JIS),
            ("吾輩は猫である。名前はまだ無い。どこで生れたかとんと見当がつかぬ。何でも薄暗いじめじめした所でニャーニャー泣いていた事だけは記憶している。", encoding_rs::EUC_JP),
            ("天下大势，分久必合，合久必分。周末七国分争，并入于秦。及秦灭之后，楚、汉分争，又并入于汉。", encoding_rs::GBK),
            ("天下大勢，分久必合，合久必分。周末七國分爭，並入於秦。及秦滅之後，楚、漢分爭，又並入於漢。", encoding_rs::BIG5),
            ("한국어 텍스트는 이렇게 인코딩되어 있습니다. 모든 사람은 태어날 때부터 자유로우며 그 존엄과 권리에 있어 동등하다.", encoding_rs::EUC_KR),
            ("Η γλώσσα μου έδωσαν ελληνική, το σπίτι φτωχικό στις αμμουδιές του Ομήρου.", encoding_rs::WINDOWS_1253),
            ("בראשית ברא אלהים את השמים ואת הארץ. והארץ היתה תהו ובהו וחשך על פני תהום.", encoding_rs::WINDOWS_1255),
            ("كان يا ما كان في قديم الزمان ملك عادل يحكم مدينة جميلة على ضفة النهر", encoding_rs::WINDOWS_1256),
        ];
        for (text, enc) in cases {
            let (bytes, _, bad) = enc.encode(text);
            assert!(!bad, "{} cannot encode the sample", enc.name());
            assert_eq!(super::decode_text(&bytes), *text, "decoding {}", enc.name());
        }
    }

    #[test]
    fn western_text_stays_windows_1252() {
        let text =
            "Café crème brûlée: voilà l'été, déjà vu. Naïve façade, garçon à la mode — « oui ».";
        let (bytes, _, bad) = encoding_rs::WINDOWS_1252.encode(text);
        assert!(!bad);
        assert_eq!(super::decode_text(&bytes), text);
        let (bytes, _, _) = encoding_rs::WINDOWS_1250.encode("Zażółć gęślą jaźń");
        assert!(!super::decode_text(&bytes).is_empty());
    }

    use super::*;
    use crate::testutil::*;

    fn ch(title: &str, body: &str) -> Chapter {
        Chapter {
            title: title.into(),
            body: body.into(),
        }
    }

    #[test]
    fn image_ext_detects_magic_numbers() {
        assert_eq!(image_ext(JPG), Some("jpg"));
        assert_eq!(image_ext(PNG), Some("png"));
        assert_eq!(image_ext(b"GIF89a\x01\0"), Some("gif"));
        assert_eq!(image_ext(b"GIF87a\x01\0"), Some("gif"));
        assert_eq!(image_ext(b"RIFF\x10\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(image_ext(b"BM\0\0\0\0\0\0\0\0\0\0\0\0\0\0"), Some("bmp"));
        assert_eq!(
            image_ext(b"<?xml version='1.0'?><svg xmlns=''/>"),
            Some("svg")
        );
        assert_eq!(image_ext(b"plain text"), None);
        assert_eq!(image_ext(b""), None);
    }

    #[test]
    fn fix_links_rewrites_only_cross_chapter_anchors() {
        let mut chapters = vec![
            ch("a", "<a id=\"x\"></a><a href=\"#x\">own</a><a href=\"#y\">other</a><a href=\"#nope\">none</a>"),
            ch("b", "<p id=\"y\">t</p><a href=\"#x\">back</a>"),
        ];
        fix_links(&mut chapters);
        assert!(chapters[0].body.contains("href=\"#x\">own"));
        assert!(chapters[0].body.contains("href=\"chapter-2.xhtml#y\""));
        assert!(chapters[0].body.contains("href=\"#nope\""));
        assert!(chapters[1].body.contains("href=\"chapter-1.xhtml#x\""));
    }

    #[test]
    fn build_adds_media_css_rtl_and_extra_metadata() {
        let md = Metadata {
            title: "Война и мир".into(),
            authors: vec!["Л. Толстой".into()],
            description: "Desc & more".into(),
            tags: vec!["war".into(), "peace".into()],
            series: "Cycle".into(),
            series_index: 2.5,
            published: "1869".into(),
            rights: "PD".into(),
            language: "ru".into(),
            ..Default::default()
        };
        let arc = build(Parts {
            md,
            chapters: vec![ch("One", "<p>x</p>")],
            cover: Some((PNG.to_vec(), "png".into())),
            media: vec![
                Media {
                    path: "images/a.png".into(),
                    data: PNG.to_vec(),
                },
                Media {
                    path: "images/a.png".into(),
                    data: JPG.to_vec(),
                },
                Media {
                    path: "images/empty.png".into(),
                    data: vec![],
                },
            ],
            rtl: true,
            css: ".extra{color:red}".into(),
            opf_meta: "    <meta property=\"x:test\">1</meta>\n".into(),
        })
        .unwrap();
        let book = validate(&arc);
        let m = book.metadata();
        assert_eq!(m.title, "Война и мир");
        assert_eq!(m.description, "Desc & more");
        assert_eq!(m.tags, vec!["war", "peace"]);
        assert_eq!(m.series, "Cycle");
        assert_eq!(m.series_index, 2.5);
        assert_eq!(m.published, "1869");
        assert_eq!(m.rights, "PD");
        assert_eq!(m.title_sort, "Война и мир");
        assert_eq!(
            arc.get("OEBPS/images/a.png"),
            Some(PNG),
            "first duplicate wins"
        );
        assert!(!arc.contains("OEBPS/images/empty.png"));
        let opf = arc.get_string("OEBPS/content.opf").unwrap();
        assert!(opf.contains("page-progression-direction=\"rtl\""));
        assert!(opf.contains("<meta property=\"x:test\">1</meta>"));
        assert!(arc
            .get_string("OEBPS/style.css")
            .unwrap()
            .contains(".extra{color:red}"));
        assert_eq!(book.cover_bytes(), Some(PNG));
    }

    #[test]
    fn build_rejects_empty_books_and_defaults_the_title() {
        assert!(build(Parts::default())
            .unwrap_err()
            .to_string()
            .contains("no readable text"));
        let arc = build(Parts {
            chapters: vec![ch("t", "<p>x</p>")],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(validate(&arc).metadata().title, "Untitled");
    }

    #[test]
    fn title_sort_is_safe_for_multibyte_titles() {
        assert_eq!(safe_title_sort("The Rust Book"), "Rust Book, The");
        assert_eq!(safe_title_sort("An Apple"), "Apple, An");
        assert_eq!(safe_title_sort("a tale"), "tale, a");
        assert_eq!(safe_title_sort("Война и мир"), "Война и мир");
        assert_eq!(safe_title_sort("日本語の本"), "日本語の本");
        assert_eq!(safe_title_sort("The "), "The");
        assert_eq!(safe_title_sort(""), "");
    }

    #[test]
    fn decode_text_handles_boms_utf16_and_latin1() {
        assert_eq!(decode_text(b"\xEF\xBB\xBFhi \xC3\xA9"), "hi é");
        assert_eq!(decode_text("héllo".as_bytes()), "héllo");
        let le: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain("héllo".encode_utf16().flat_map(|u| u.to_le_bytes()))
            .collect();
        assert_eq!(decode_text(&le), "héllo");
        let be: Vec<u8> = [0xFE, 0xFF]
            .into_iter()
            .chain("héllo".encode_utf16().flat_map(|u| u.to_be_bytes()))
            .collect();
        assert_eq!(decode_text(&be), "héllo");
        let le_nobom: Vec<u8> = "hello world"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        assert_eq!(decode_text(&le_nobom), "hello world");
        let be_nobom: Vec<u8> = "hello world"
            .encode_utf16()
            .flat_map(|u| u.to_be_bytes())
            .collect();
        assert_eq!(decode_text(&be_nobom), "hello world");
        // invalid UTF-8 -> Windows-1252 (caf\xe9 = café, \x93quoted\x94 = curly quotes)
        assert_eq!(decode_text(b"caf\xe9 \x93q\x94"), "café “q”");
        assert_eq!(decode_text(b""), "");
    }

    #[test]
    fn decode_xml_honours_declared_encoding() {
        let (b, _, _) = encoding_rs::WINDOWS_1251
            .encode("<?xml version='1.0' encoding='windows-1251'?><a>Привет</a>");
        assert!(decode_xml(&b).contains("Привет"));
        let (b, _, _) = encoding_rs::SHIFT_JIS
            .encode("<?xml version=\"1.0\" encoding=\"Shift_JIS\"?><a>日本語</a>");
        assert!(decode_xml(&b).contains("日本語"));
        assert_eq!(
            decode_xml("<?xml version='1.0' encoding='utf-8'?><a>é</a>".as_bytes()),
            "<?xml version='1.0' encoding='utf-8'?><a>é</a>"
        );
        assert_eq!(decode_xml(b"<a>x</a>"), "<a>x</a>");
        assert_eq!(decode_xml(b"\xEF\xBB\xBF<a/>"), "<a/>");
        // an unknown label counts as undeclared: a legacy single byte encoding, not mojibake
        assert!(decode_xml(b"<?xml encoding='bogus-enc'?>\xff").contains('\u{ff}'));
        assert!(decode_xml(b"<?xml encoding='utf-8'?>\xff").contains('\u{fffd}'));
    }

    #[test]
    fn paragraphs_split_on_blank_lines() {
        assert_eq!(
            text_to_paragraphs("one\ntwo & three\n\n\nfour\n"),
            "<p>one two &amp; three</p>\n<p>four</p>\n"
        );
        assert_eq!(text_to_paragraphs(""), "");
        assert_eq!(text_to_paragraphs("  \n \n"), "");
    }

    #[test]
    fn zip_helpers_read_names_and_entries() {
        let z = make_zip(&[("a/b.txt", b"hello"), ("c.txt", b"world")]);
        let f = read_zip(&z).unwrap();
        assert_eq!(f.get("a/b.txt"), Some(&b"hello"[..]));
        assert_eq!(f.get_ci("C.TXT"), Some(&b"world"[..]));
        assert_eq!(f.get("zzz"), None);
        assert_eq!(f.names().collect::<Vec<_>>(), vec!["a/b.txt", "c.txt"]);
        assert_eq!(zip_names(&z).unwrap(), vec!["a/b.txt", "c.txt"]);
        assert_eq!(zip_names(&z[..40]).unwrap(), vec!["a/b.txt"]);
        assert_eq!(zip_names(&z[..10]).unwrap(), Vec::<String>::new());
        assert!(zip_names(b"not a zip").is_none());
        assert_eq!(zip_entry(&z, "c.txt"), Some(b"world".to_vec()));
        assert_eq!(zip_entry(&z, "nope"), None);
        assert!(read_zip(b"garbage").is_err());
        assert!(read_zip(&z[..z.len() / 2]).is_err());
    }

    #[test]
    fn zip_bombs_are_refused_by_the_size_limits() {
        let z = make_zip(&[("big.bin", &vec![0u8; 100_000]), ("b.bin", &[1u8; 10])]);
        let e = read_zip_limited(&z, 50_000, 1 << 30)
            .unwrap_err()
            .to_string();
        assert!(e.contains("`big.bin` is too large"), "{e}");
        let z = make_zip(&[("a.bin", &vec![0u8; 600]), ("b.bin", &vec![1u8; 600])]);
        let e = read_zip_limited(&z, 1000, 1000).unwrap_err().to_string();
        assert!(e.contains("content is too large"), "{e}");
        assert!(read_zip_limited(&z, 1000, 1200).is_ok());
    }

    #[test]
    fn kindle_links_are_dropped_from_html() {
        let x = html_to_xhtml(
            "<body><a href=\"kindle:pos:fid:0001:off:0\">t</a><a href=\"http://x\">u</a></body>",
            &|_| None,
        );
        assert_eq!(x, "<a>t</a><a href=\"http://x\">u</a>");
    }

    #[test]
    fn read_zip_skips_path_traversal_entries() {
        let z = make_zip(&[("../evil.txt", b"x"), ("ok.txt", b"y")]);
        let f = read_zip(&z).unwrap();
        assert_eq!(f.names().collect::<Vec<_>>(), vec!["ok.txt"]);
    }

    #[test]
    fn html_is_serialised_as_xhtml() {
        let html = r#"<html><head><title>T</title><style>p{}</style><script>alert(1)</script></head>
            <body onload="x()"><h1 id=a class=c>Title</h1><p>one<br>two <img src="pic.png"> &amp; <b>bold</b><font>f</font></p>
            <center>c</center><span unknown="1" style="x">kept</span><hr><custom>inner</custom></body></html>"#;
        let x = html_to_xhtml(html, &|s| {
            (s == "pic.png").then(|| "images/p.png".to_string())
        });
        assert!(
            x.contains(">Title</h1>") && x.contains("id=\"a\"") && x.contains("class=\"c\""),
            "{x}"
        );
        assert!(x.contains("one<br/>two"));
        assert!(x.contains("<img src=\"images/p.png\" alt=\"\"/>"));
        assert!(x.contains("&amp; <b>bold</b><span>f</span>"));
        assert!(x.contains("<div>c</div>"));
        assert!(x.contains("<span style=\"x\">kept</span>"));
        assert!(x.contains("<hr/>"));
        assert!(x.contains("inner"));
        assert!(!x.contains("alert") && !x.contains("onload") && !x.contains("unknown"));
        roxmltree::Document::parse(&format!("<body>{x}</body>")).unwrap();
        assert_eq!(
            html_blocks("<body>text<p>a</p>  <p>b</p></body>", &|_| None),
            vec!["text", "<p>a</p>", "<p>b</p>"]
        );
        assert!(html_blocks("", &|_| None).is_empty());
        assert_eq!(
            html_title("<title> My Book </title>"),
            Some("My Book".into())
        );
        assert_eq!(html_title("<p>x</p>"), None);
        assert_eq!(html_title("<title> </title>"), None);
    }

    #[test]
    fn entity_and_tag_helpers() {
        assert_eq!(
            xml_unescape("a &amp; b &lt;c&gt; &quot;q&quot; &apos;s&apos; &#65;&#x42; &bogus; &"),
            "a & b <c> \"q\" 's' AB &bogus; &"
        );
        assert_eq!(xml_unescape("&#xFFFFFFFF;"), "&#xFFFFFFFF;");
        assert_eq!(
            strip_tags("<h1>A <b>bold</b>\n&amp; co</h1>"),
            "A bold & co"
        );
        assert_eq!(visible_text_len("<p> a b </p><img/>"), 2);
        assert_eq!(visible_text_len(""), 0);
        assert_eq!(sanitize_stem("a b/ü.png"), "a_b___png");
        assert_eq!(sanitize_stem(""), "x");
    }

    #[test]
    fn chapters_split_at_headings() {
        let blocks = vec![
            "<p>preface</p>".to_string(),
            "<h1>One</h1>".to_string(),
            "<p>a</p>".to_string(),
            "<h2>sub</h2>".to_string(),
            "<h1 id=\"x\">Two</h1>".to_string(),
            "<p>b</p>".to_string(),
        ];
        let c = chapters_from_blocks(blocks, "Start");
        let titles: Vec<_> = c.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, vec!["Start", "One", "Two"]);
        assert!(c[1].body.contains("<h2>sub</h2>"));
        // no h1: falls back to h2
        let c = chapters_from_blocks(
            vec![
                "<h2>A</h2>".into(),
                "<p>t</p>".into(),
                "<h2>B</h2>".into(),
                "<p>u</p>".into(),
            ],
            "S",
        );
        assert_eq!(c.len(), 2);
        // no headings: single chapter; h10 is not a heading
        let c = chapters_from_blocks(vec!["<p>t</p>".into(), "<h10>x</h10>".into()], "S");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].title, "S");
        // empty heading chapters are dropped, empty input gives nothing
        assert!(chapters_from_blocks(vec!["<h1>Only</h1>".into()], "S").len() == 1);
        assert!(chapters_from_blocks(vec![], "S").is_empty());
        assert_eq!(
            chapters_from_blocks(vec!["<h1></h1>".into(), "<p>x</p>".into()], "D")[0].title,
            "D"
        );
    }

    #[test]
    fn media_set_names_are_unique_and_url_safe() {
        let mut m = MediaSet::default();
        assert_eq!(
            m.add("dir/My Pic.PNG", PNG.to_vec()).unwrap(),
            "images/My_Pic.png"
        );
        assert_eq!(
            m.add("other/My Pic.png", PNG.to_vec()).unwrap(),
            "images/My_Pic-2.png"
        );
        assert_eq!(
            m.add("dir/My Pic.PNG", PNG.to_vec()).unwrap(),
            "images/My_Pic.png"
        );
        assert_eq!(m.items.len(), 2);
        assert_eq!(m.path_of("dir/My Pic.PNG"), Some("images/My_Pic.png"));
        assert_eq!(m.path_of("zzz"), None);
        assert!(m.add("notimage.txt", b"hello".to_vec()).is_none());
    }

    #[test]
    fn opf_info_is_parsed_without_the_core_parser() {
        let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
<dc:title>Война и мир</dc:title><dc:creator>A</dc:creator><dc:creator>B</dc:creator><dc:publisher>P</dc:publisher>
<dc:language>ru</dc:language><dc:description>D</dc:description><dc:subject>s1</dc:subject><dc:date>2000</dc:date><dc:rights>R</dc:rights>
<dc:identifier opf:scheme="ISBN">123</dc:identifier><dc:identifier opf:scheme="uuid">u</dc:identifier>
<meta name="calibre:series" content="S"/><meta name="calibre:series_index" content="3"/><meta name="cover" content="cvr"/></metadata>
<manifest><item id="cvr" href="cover.jpg" media-type="image/jpeg"/></manifest></package>"#;
        let i = parse_opf_info(opf).unwrap();
        assert_eq!(i.md.title, "Война и мир");
        assert_eq!(i.md.authors, vec!["A", "B"]);
        assert_eq!(
            (
                i.md.publisher.as_str(),
                i.md.language.as_str(),
                i.md.description.as_str()
            ),
            ("P", "ru", "D")
        );
        assert_eq!(i.md.tags, vec!["s1"]);
        assert_eq!(i.md.published, "2000");
        assert_eq!(i.md.rights, "R");
        assert_eq!(i.md.identifiers.len(), 1);
        assert_eq!(i.md.identifiers[0].value, "123");
        assert_eq!((i.md.series.as_str(), i.md.series_index), ("S", 3.0));
        assert_eq!(i.cover_href.as_deref(), Some("cover.jpg"));
        assert!(parse_opf_info("<not xml").is_none());
        let g = parse_opf_info(
            r#"<package><guide><reference type="cover" href="c.png"/></guide></package>"#,
        )
        .unwrap();
        assert_eq!(g.cover_href.as_deref(), Some("c.png"));
    }
}
