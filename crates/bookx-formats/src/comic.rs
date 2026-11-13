//! Comic book archives: CBZ (zip), CBT (tar) and CBR (RAR).
//!
//! Every image becomes one XHTML page (`images/page-NNNN.ext` shown full width) in natural file
//! name order; the front cover (or first image) is the cover. `ComicInfo.xml` supplies title,
//! series, writers, summary, genre, language and the manga reading direction.
//!
//! CBR: there is no pure-Rust RAR decoder, so real RAR archives are rejected with a clear
//! error. Many `.cbr` files are really zip archives; those are read as CBZ.

use std::cmp::Ordering;
use std::io::Read;

use anyhow::{bail, Context, Result};
use bookx_core::archive::Archive;
use bookx_core::contracts::FormatReader;
use bookx_core::convert::Chapter;
use bookx_core::opf::Metadata;
use bookx_core::util::{parse_xml, xml_escape};

use crate::common::*;

const CSS: &str = "body { margin: 0; padding: 0; }\n.page { margin: 0; padding: 0; text-align: center; }\n.page img { max-width: 100%; max-height: 100vh; object-fit: contain; }\n";

/// Reader for `.cbz`.
pub struct CbzReader;
/// Reader for `.cbt`.
pub struct CbtReader;
/// Reader for `.cbr` (zip-in-disguise only, see the module docs).
pub struct CbrReader;

const RAR_MAGIC: &[u8] = b"Rar!\x1a\x07";

/// Compares file names the way people expect: `page2` before `page10`, case-insensitive.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut xs = String::new();
                while let Some(c) = ai.next_if(|c| c.is_ascii_digit()) {
                    xs.push(c);
                }
                let mut ys = String::new();
                while let Some(c) = bi.next_if(|c| c.is_ascii_digit()) {
                    ys.push(c);
                }
                let (xt, yt) = (xs.trim_start_matches('0'), ys.trim_start_matches('0'));
                let ord = xt
                    .len()
                    .cmp(&yt.len())
                    .then_with(|| xt.cmp(yt))
                    .then_with(|| xs.len().cmp(&ys.len()));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                ai.next();
                bi.next();
            }
        }
    }
}

pub(crate) fn is_page_name(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    if l.starts_with("__macosx/") || l.contains("/__macosx/") {
        return false;
    }
    let file = l.rsplit('/').next().unwrap_or("");
    if file.starts_with('.') {
        return false;
    }
    matches!(
        file.rsplit_once('.').map(|(_, e)| e),
        Some("jpg" | "jpeg" | "png" | "gif" | "webp")
    )
}

fn zip_looks_like_comic(names: &[String]) -> bool {
    let pages = names.iter().filter(|n| is_page_name(n)).count();
    pages > 0 && pages * 2 >= names.iter().filter(|n| !n.ends_with('/')).count()
}

impl FormatReader for CbzReader {
    fn id(&self) -> &'static str {
        "cbz"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["cbz"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        zip_names(head).is_some_and(|n| zip_looks_like_comic(&n))
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        let zip = read_zip(bytes)?;
        convert_files(zip.files)
    }
}

/// Extracts a tar archive into memory with size guards.
fn read_tar(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    read_tar_limited(bytes, MAX_ENTRY, MAX_TOTAL)
}

fn read_tar_limited(
    bytes: &[u8],
    max_entry: u64,
    max_total: u64,
) -> Result<Vec<(String, Vec<u8>)>> {
    let mut ar = tar::Archive::new(bytes);
    let mut out = Vec::new();
    let mut total = 0u64;
    for entry in ar.entries().context("not a tar archive")? {
        let entry = entry.context("damaged tar entry")?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let name = entry
            .path()
            .context("bad tar path")?
            .to_string_lossy()
            .replace('\\', "/");
        if name.split('/').any(|s| s == "..") {
            continue;
        }
        let mut buf = Vec::new();
        entry.take(max_entry + 1).read_to_end(&mut buf)?;
        if buf.len() as u64 > max_entry {
            bail!("`{name}` is too large");
        }
        total += buf.len() as u64;
        if total > max_total || out.len() >= MAX_ENTRIES {
            bail!("tar content is too large");
        }
        out.push((name, buf));
    }
    Ok(out)
}

impl FormatReader for CbtReader {
    fn id(&self) -> &'static str {
        "cbt"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["cbt"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        head.len() > 262 && &head[257..262] == b"ustar"
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        convert_files(read_tar(bytes)?)
    }
}

impl FormatReader for CbrReader {
    fn id(&self) -> &'static str {
        "cbr"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["cbr"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        head.starts_with(RAR_MAGIC)
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        if bytes.starts_with(RAR_MAGIC) {
            bail!("RAR comic archives (CBR) are not supported: no pure-Rust RAR decoder is available. Convert the file to CBZ first");
        }
        if bytes.starts_with(b"PK\x03\x04") {
            return CbzReader.read(bytes);
        }
        if bytes.len() > 262 && &bytes[257..262] == b"ustar" {
            return CbtReader.read(bytes);
        }
        bail!("not a comic archive");
    }
}

#[derive(Default)]
struct ComicInfo {
    md: Metadata,
    rtl: bool,
    cover_index: Option<usize>,
}

fn parse_comic_info(text: &str) -> Option<ComicInfo> {
    let doc = parse_xml(text).ok()?;
    let root = doc.root_element();
    if !root.tag_name().name().eq_ignore_ascii_case("ComicInfo") {
        return None;
    }
    let mut ci = ComicInfo::default();
    let mut numbers = (None::<u32>, None::<u32>, None::<u32>);
    let mut series = String::new();
    let mut number = String::new();
    for n in root.children().filter(|c| c.is_element()) {
        let t = node_text(n).trim().to_string();
        match n.tag_name().name().to_ascii_lowercase().as_str() {
            "title" => ci.md.title = t,
            "series" => series = t,
            "number" => number = t,
            "writer" => ci.md.authors = split_keywords(&t),
            "summary" => ci.md.description = t,
            "genre" => ci.md.tags = split_keywords(&t),
            "languageiso" => ci.md.language = t,
            "publisher" => ci.md.publisher = t,
            "year" => numbers.0 = t.parse().ok(),
            "month" => numbers.1 = t.parse().ok(),
            "day" => numbers.2 = t.parse().ok(),
            "manga" => ci.rtl = t.eq_ignore_ascii_case("YesAndRightToLeft"),
            "pages" => {
                for p in n.children().filter(|p| p.is_element()) {
                    if attr(p, "Type").is_some_and(|t| t.eq_ignore_ascii_case("FrontCover")) {
                        ci.cover_index = attr(p, "Image").and_then(|i| i.parse().ok());
                    }
                }
            }
            _ => {}
        }
    }
    if !series.is_empty() {
        ci.md.series = series.clone();
        if let Ok(i) = number.parse::<f64>() {
            ci.md.series_index = i;
        }
        if ci.md.title.is_empty() {
            ci.md.title = if number.is_empty() {
                series
            } else {
                format!("{series} #{number}")
            };
        }
    }
    if let Some(y) = numbers.0 {
        ci.md.published = match (numbers.1, numbers.2) {
            (Some(m), Some(d)) => format!("{y:04}-{m:02}-{d:02}"),
            (Some(m), None) => format!("{y:04}-{m:02}"),
            _ => format!("{y:04}"),
        };
    }
    Some(ci)
}

/// Builds the EPUB from the extracted files of a comic archive.
pub fn convert_files(files: Vec<(String, Vec<u8>)>) -> Result<Archive> {
    let info_data = files
        .iter()
        .find(|(n, _)| {
            n.rsplit('/')
                .next()
                .is_some_and(|f| f.eq_ignore_ascii_case("ComicInfo.xml"))
        })
        .map(|(_, d)| decode_xml(d));
    let info = info_data
        .as_deref()
        .and_then(parse_comic_info)
        .unwrap_or_default();

    let mut pages: Vec<(String, Vec<u8>)> = files
        .into_iter()
        .filter(|(n, d)| is_page_name(n) && image_ext(d).is_some())
        .collect();
    if pages.is_empty() {
        bail!("the archive contains no images");
    }
    pages.sort_by(|a, b| natural_cmp(&a.0, &b.0));

    let mut media = Vec::with_capacity(pages.len());
    let mut chapters = Vec::with_capacity(pages.len());
    for (i, (_, data)) in pages.iter().enumerate() {
        let ext = image_ext(data).unwrap_or("jpg");
        let path = format!("images/page-{:04}.{ext}", i + 1);
        chapters.push(Chapter {
            title: format!("Page {}", i + 1),
            body: format!(
                "<div class=\"page\"><img src=\"{path}\" alt=\"{}\"/></div>\n",
                xml_escape(&format!("Page {}", i + 1))
            ),
        });
        media.push(Media {
            path,
            data: data.clone(),
        });
    }
    let cover_idx = info.cover_index.filter(|i| *i < pages.len()).unwrap_or(0);
    let cover_data = pages[cover_idx].1.clone();
    let cover_ext = image_ext(&cover_data).unwrap_or("jpg").to_string();

    let mut md = info.md;
    if md.title.is_empty() {
        md.title = "Untitled".into();
    }
    build(Parts {
        md,
        chapters,
        cover: Some((cover_data, cover_ext)),
        media,
        rtl: info.rtl,
        css: CSS.into(),
        opf_meta: String::new(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::testutil::*;

    pub fn img(tag: u8, kind: &str) -> Vec<u8> {
        let mut v = if kind == "png" {
            PNG.to_vec()
        } else {
            JPG.to_vec()
        };
        v.push(tag);
        v
    }

    pub const INFO: &str = r#"<?xml version="1.0"?><ComicInfo><Title>Issue Title</Title><Series>Space Saga</Series><Number>3.5</Number><Writer>Alan, Bea</Writer>
<Summary>A summary.</Summary><Genre>Action, Sci-Fi</Genre><LanguageISO>ja</LanguageISO><Publisher>Comics Inc</Publisher><Year>2020</Year><Month>3</Month><Day>9</Day><Manga>YesAndRightToLeft</Manga>
<Pages><Page Image="0"/><Page Image="1" Type="FrontCover"/></Pages></ComicInfo>"#;

    pub fn sample() -> Vec<u8> {
        make_zip(&[
            ("ComicInfo.xml", INFO.as_bytes()),
            ("pages/10.jpg", &img(10, "jpg")),
            ("pages/2.png", &img(2, "png")),
            ("pages/1.jpg", &img(1, "jpg")),
            ("__MACOSX/pages/._1.jpg", &img(99, "jpg")),
            ("pages/.hidden.jpg", &img(98, "jpg")),
            ("pages/Thumbs.db", b"junk"),
            ("pages/fake.jpg", b"not an image"),
        ])
    }

    #[test]
    fn natural_sort() {
        let mut v = vec![
            "p10.jpg",
            "p2.jpg",
            "P1.jpg",
            "p02a.jpg",
            "a/p1.jpg",
            "p2.jpg.bak",
        ];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            v,
            vec![
                "a/p1.jpg",
                "P1.jpg",
                "p2.jpg",
                "p2.jpg.bak",
                "p02a.jpg",
                "p10.jpg"
            ]
        );
        assert_eq!(natural_cmp("a", "a"), Ordering::Equal);
        assert_eq!(
            natural_cmp("a1", "a01"),
            Ordering::Less,
            "equal numbers: fewer zeros first"
        );
        assert_eq!(natural_cmp("a", "ab"), Ordering::Less);
        assert_eq!(natural_cmp("ab", "a"), Ordering::Greater);
        assert_eq!(natural_cmp("img9", "img10"), Ordering::Less);
        assert_eq!(
            natural_cmp("img99999999999999999999", "img100000000000000000000"),
            Ordering::Less
        );
    }

    #[test]
    fn pages_are_ordered_and_junk_is_skipped() {
        let arc = CbzReader.read(&sample()).unwrap();
        let book = validate(&arc);
        assert_eq!(book.spine_paths().len(), 3);
        assert_eq!(
            arc.get("OEBPS/images/page-0001.jpg"),
            Some(&img(1, "jpg")[..])
        );
        assert_eq!(
            arc.get("OEBPS/images/page-0002.png"),
            Some(&img(2, "png")[..])
        );
        assert_eq!(
            arc.get("OEBPS/images/page-0003.jpg"),
            Some(&img(10, "jpg")[..])
        );
        assert_eq!(toc_titles(&book), vec!["Page 1", "Page 2", "Page 3"]);
        let x = chapter_xml(&book, 1);
        assert!(x.contains(
            "<div class=\"page\"><img src=\"images/page-0002.png\" alt=\"Page 2\"/></div>"
        ));
        assert!(arc
            .get_string("OEBPS/style.css")
            .unwrap()
            .contains("max-width: 100%"));
    }

    #[test]
    fn comic_info_metadata_and_direction() {
        let arc = CbzReader.read(&sample()).unwrap();
        let book = validate(&arc);
        let md = book.metadata();
        assert_eq!(md.title, "Issue Title");
        assert_eq!(md.series, "Space Saga");
        assert_eq!(md.series_index, 3.5);
        assert_eq!(md.authors, vec!["Alan", "Bea"]);
        assert_eq!(md.description, "A summary.");
        assert_eq!(md.tags, vec!["Action", "Sci-Fi"]);
        assert_eq!(md.language, "ja");
        assert_eq!(md.publisher, "Comics Inc");
        assert_eq!(md.published, "2020-03-09");
        let opf = arc.get_string("OEBPS/content.opf").unwrap();
        assert!(opf.contains("<spine toc=\"ncx\" page-progression-direction=\"rtl\">"));
        assert_eq!(book.package.spine.len(), 3);
    }

    #[test]
    fn front_cover_page_type_selects_the_cover() {
        let book = validate(&CbzReader.read(&sample()).unwrap());
        assert_eq!(book.cover_bytes(), Some(&img(2, "png")[..]));
    }

    #[test]
    fn without_comic_info_defaults_apply() {
        let z = make_zip(&[("b.jpg", &img(2, "jpg")), ("a.jpg", &img(1, "jpg"))]);
        let arc = CbzReader.read(&z).unwrap();
        let book = validate(&arc);
        assert_eq!(book.metadata().title, "Untitled");
        assert_eq!(book.cover_bytes(), Some(&img(1, "jpg")[..]));
        assert!(!arc
            .get_string("OEBPS/content.opf")
            .unwrap()
            .contains("page-progression-direction"));
    }

    #[test]
    fn comic_info_variants() {
        let ci = parse_comic_info("<ComicInfo><Series>S</Series><Number>7</Number><Year>1999</Year><Manga>Yes</Manga></ComicInfo>").unwrap();
        assert_eq!(ci.md.title, "S #7");
        assert_eq!(ci.md.series_index, 7.0);
        assert_eq!(ci.md.published, "1999");
        assert!(!ci.rtl, "plain Yes is manga but left to right");
        let ci = parse_comic_info(
            "<ComicInfo><Series>S</Series><Year>1999</Year><Month>4</Month></ComicInfo>",
        )
        .unwrap();
        assert_eq!(
            (ci.md.title.as_str(), ci.md.published.as_str()),
            ("S", "1999-04")
        );
        let ci = parse_comic_info("<comicinfo><TITLE>Lower</TITLE></comicinfo>").unwrap();
        assert_eq!(ci.md.title, "Lower");
        assert!(parse_comic_info("<Other/>").is_none());
        assert!(parse_comic_info("<broken").is_none());
        // invalid cover index falls back to the first page
        let z = make_zip(&[
            ("a.jpg", &img(1, "jpg")),
            (
                "ComicInfo.xml",
                b"<ComicInfo><Pages><Page Image=\"9\" Type=\"FrontCover\"/></Pages></ComicInfo>",
            ),
        ]);
        let book = validate(&CbzReader.read(&z).unwrap());
        assert_eq!(book.cover_bytes(), Some(&img(1, "jpg")[..]));
    }

    #[test]
    fn webp_and_gif_pages_are_accepted() {
        let webp = b"RIFF\x10\0\0\0WEBPVP8 \0\0".to_vec();
        let gif = b"GIF89a\x01\0\x01\0".to_vec();
        let z = make_zip(&[("1.webp", &webp), ("2.gif", &gif)]);
        let arc = CbzReader.read(&z).unwrap();
        validate(&arc);
        assert!(
            arc.contains("OEBPS/images/page-0001.webp")
                && arc.contains("OEBPS/images/page-0002.gif")
        );
    }

    #[test]
    fn errors() {
        assert!(CbzReader.read(b"").is_err());
        assert!(CbzReader
            .read(&make_zip(&[("a.txt", b"x")]))
            .unwrap_err()
            .to_string()
            .contains("no images"));
        assert!(CbzReader
            .read(&make_zip(&[("a.jpg", b"not an image")]))
            .is_err());
        assert!(CbtReader.read(b"").is_err());
    }

    fn make_tar(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for (n, d) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(d.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, n, *d).unwrap();
        }
        b.into_inner().unwrap()
    }

    #[test]
    fn cbt_reads_tar_archives() {
        let t = make_tar(&[
            ("ComicInfo.xml", INFO.as_bytes()),
            ("10.jpg", &img(10, "jpg")),
            ("2.jpg", &img(2, "jpg")),
            ("1.jpg", &img(1, "jpg")),
        ]);
        let arc = CbtReader.read(&t).unwrap();
        let book = validate(&arc);
        assert_eq!(book.spine_paths().len(), 3);
        assert_eq!(
            arc.get("OEBPS/images/page-0003.jpg"),
            Some(&img(10, "jpg")[..])
        );
        assert_eq!(book.metadata().series, "Space Saga");
        assert!(CbtReader.sniff(&t));
        assert!(!CbtReader.sniff(b"short"));
        assert!(!CbtReader.sniff(&sample()));
        assert!(CbtReader
            .read(b"definitely not a tar file, just text that is long enough")
            .is_err());
        assert!(CbtReader
            .read(&make_tar(&[("a.txt", b"x")]))
            .unwrap_err()
            .to_string()
            .contains("no images"));
        let evil = make_tar(&[("a.jpg", &img(1, "jpg"))]);
        assert!(CbtReader.read(&evil).is_ok());
    }

    #[test]
    fn oversized_tar_entries_are_refused() {
        let t = make_tar(&[
            ("1.jpg", &vec![0xFFu8; 5000]),
            ("2.jpg", &vec![0xFFu8; 5000]),
        ]);
        assert!(read_tar_limited(&t, 4000, 1 << 30)
            .unwrap_err()
            .to_string()
            .contains("too large"));
        assert!(read_tar_limited(&t, 6000, 8000)
            .unwrap_err()
            .to_string()
            .contains("too large"));
        assert_eq!(read_tar_limited(&t, 6000, 20000).unwrap().len(), 2);
    }

    #[test]
    fn cbr_rar_is_rejected_clearly_and_disguised_zip_is_read() {
        let rar = b"Rar!\x1a\x07\x01\x00rest of the archive".to_vec();
        let err = CbrReader.read(&rar).unwrap_err().to_string();
        assert!(
            err.contains("RAR") && err.contains("not supported"),
            "{err}"
        );
        assert!(CbrReader.sniff(&rar));
        assert!(!CbrReader.sniff(&sample()));
        let book = validate(&CbrReader.read(&sample()).unwrap());
        assert_eq!(book.spine_paths().len(), 3);
        let t = make_tar(&[("1.jpg", &img(1, "jpg"))]);
        assert!(CbrReader.read(&t).is_ok());
        assert!(CbrReader
            .read(b"nothing useful here")
            .unwrap_err()
            .to_string()
            .contains("not a comic"));
    }

    #[test]
    fn sniffing_zip_content() {
        assert!(CbzReader.sniff(&sample()));
        assert!(!CbzReader.sniff(&make_zip(&[
            ("a.txt", b"x"),
            ("b.txt", b"y"),
            ("c.jpg", JPG)
        ])));
        assert!(!CbzReader.sniff(b"hello"));
        assert!(CbzReader.sniff(&make_zip(&[("01.jpg", JPG), ("02.jpg", JPG)])[..60]));
        assert_eq!(CbzReader.id(), "cbz");
        assert_eq!((CbtReader.id(), CbrReader.id()), ("cbt", "cbr"));
        assert_eq!(
            (
                CbzReader.extensions(),
                CbtReader.extensions(),
                CbrReader.extensions()
            ),
            (&["cbz"][..], &["cbt"][..], &["cbr"][..])
        );
    }

    #[test]
    fn robust_against_garbage_and_mutations() {
        assert_rejects_garbage(&CbzReader);
        assert_rejects_garbage(&CbtReader);
        assert_rejects_garbage(&CbrReader);
        fuzz_no_panic(&CbzReader, &sample());
        fuzz_no_panic(
            &CbtReader,
            &make_tar(&[
                ("1.jpg", &img(1, "jpg")),
                ("ComicInfo.xml", INFO.as_bytes()),
            ]),
        );
        fuzz_no_panic(&CbrReader, &sample());
    }

    #[test]
    fn many_pages_are_fast_enough() {
        let imgs: Vec<(String, Vec<u8>)> = (0..400)
            .map(|i| (format!("{i:04}.jpg"), img((i % 250) as u8, "jpg")))
            .collect();
        let refs: Vec<(&str, &[u8])> = imgs
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect();
        let t = std::time::Instant::now();
        let arc = CbzReader.read(&make_zip(&refs)).unwrap();
        assert_eq!(validate(&arc).spine_paths().len(), 400);
        assert!(t.elapsed().as_secs() < 30);
    }
}
