//! eReader PDB and PMLZ writers: decoded with a decoder written for the tests (Palm database,
//! zlib, PalmDOC, PML), compared with Calibre's own output and read back by the formats crate.

mod common;

use std::collections::HashSet;
use std::io::Read;

use bookx_convert::writers::{EReaderWriter, PdbCompression, PmlzWriter};
use bookx_core::contracts::{FormatReader, FormatWriter};
use common::*;

fn be16(b: &[u8], o: usize) -> usize {
    u16::from_be_bytes([b[o], b[o + 1]]) as usize
}
fn be32(b: &[u8], o: usize) -> usize {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as usize
}

struct Palm {
    name: Vec<u8>,
    db_type: [u8; 4],
    creator: [u8; 4],
    records: Vec<Vec<u8>>,
}

fn palm(data: &[u8]) -> Palm {
    let n = be16(data, 76);
    let offs: Vec<usize> = (0..n).map(|i| be32(data, 78 + i * 8)).collect();
    assert!(
        offs.windows(2).all(|w| w[0] <= w[1]),
        "record offsets ascend"
    );
    assert!(*offs.last().unwrap() <= data.len());
    assert_eq!(
        offs[0],
        78 + n * 8 + 2,
        "first record follows the list and the 2 byte gap"
    );
    let mut records = Vec::new();
    for i in 0..n {
        let end = offs.get(i + 1).copied().unwrap_or(data.len());
        records.push(data[offs[i]..end].to_vec());
    }
    let name_end = data[..32]
        .iter()
        .position(|b| *b == 0)
        .expect("name is NUL terminated");
    Palm {
        name: data[..name_end].to_vec(),
        db_type: data[60..64].try_into().unwrap(),
        creator: data[64..68].try_into().unwrap(),
        records,
    }
}

fn zlib_decode(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(b)
        .read_to_end(&mut out)
        .expect("valid zlib");
    out
}

/// PalmDOC decompression written from the format description.
fn palmdoc_decode(b: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        i += 1;
        match c {
            0 | 9..=0x7f => out.push(c),
            1..=8 => {
                out.extend_from_slice(&b[i..i + c as usize]);
                i += c as usize;
            }
            0x80..=0xbf => {
                let pair = (((c as usize) << 8) | b[i] as usize) & 0x3fff;
                i += 1;
                let (dist, len) = (pair >> 3, (pair & 7) + 3);
                for _ in 0..len {
                    out.push(out[out.len() - dist]);
                }
            }
            0xc0..=0xff => {
                out.push(b' ');
                out.push(c ^ 0x80);
            }
        }
    }
    out
}

struct Ereader {
    palm: Palm,
    header: Vec<u8>,
}

impl Ereader {
    fn h(&self, idx: usize) -> usize {
        be16(&self.header, idx * 2)
    }
    fn version(&self) -> usize {
        self.h(0)
    }
    fn non_text(&self) -> usize {
        self.h(6)
    }
    fn text(&self) -> String {
        let mut out = Vec::new();
        for r in &self.palm.records[1..self.non_text()] {
            let part = match self.version() {
                10 => zlib_decode(r),
                2 => palmdoc_decode(r),
                v => panic!("version {v}"),
            };
            out.extend(part);
        }
        String::from_utf8(out).expect("PML text is ASCII")
    }
    fn text_records(&self) -> Vec<Vec<u8>> {
        self.palm.records[1..self.non_text()]
            .iter()
            .map(|r| match self.version() {
                10 => zlib_decode(r),
                _ => palmdoc_decode(r),
            })
            .collect()
    }
}

fn ereader(bytes: &[u8]) -> Ereader {
    let p = palm(bytes);
    assert_eq!(&p.db_type, b"PNRd");
    assert_eq!(&p.creator, b"PPrs");
    let header = p.records[0].clone();
    assert_eq!(header.len(), 132);
    Ereader { palm: p, header }
}

/// Decodes PML to plain text the way a reader shows it.
fn pml_plain(pml: &str) -> String {
    let c: Vec<char> = pml.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        if c[i] != '\\' {
            out.push(c[i]);
            i += 1;
            continue;
        }
        i += 1;
        match c[i] {
            '\\' => {
                out.push('\\');
                i += 1;
            }
            'a' => {
                let d: String = c[i + 1..i + 4].iter().collect();
                let b = d.parse::<u8>().unwrap();
                out.push(
                    encoding_rs::WINDOWS_1252
                        .decode_without_bom_handling(&[b])
                        .0
                        .chars()
                        .next()
                        .unwrap(),
                );
                i += 4;
            }
            'U' => {
                let h: String = c[i + 1..i + 5].iter().collect();
                assert!(
                    h.chars()
                        .all(|x| x.is_ascii_hexdigit() && !x.is_ascii_uppercase()),
                    "lower case hex: {h}"
                );
                out.push(char::from_u32(u32::from_str_radix(&h, 16).unwrap()).unwrap());
                i += 5;
            }
            'X' => i += 2,
            'S' => i += 2,
            'p' | 'x' | 'B' | 'i' | 'u' | 'o' | 'k' | 'c' | 'r' | 't' => i += 1,
            'q' | 'Q' | 'm' | 'w' => {
                i += 1;
                if c.get(i) == Some(&'=') {
                    let end = c[i + 2..].iter().position(|x| *x == '"').unwrap();
                    i += 2 + end + 1;
                }
            }
            'v' => i += 1,
            other => panic!("unknown PML code \\{other}"),
        }
    }
    out
}

fn normalized(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn pdb(book: &bookx_core::book::OpenBook) -> Vec<u8> {
    EReaderWriter::default().write(book).unwrap()
}

// ---- container ------------------------------------------------------------------------------

#[test]
fn database_header_and_record_layout() {
    let bytes = pdb(&sample_book());
    // attributes byte zero, unique ids ascend
    let n = be16(&bytes, 76);
    let uids: Vec<usize> = (0..n)
        .map(|i| {
            assert_eq!(bytes[78 + i * 8 + 4], 0);
            be32(&bytes, 78 + i * 8 + 4) & 0xff_ffff
        })
        .collect();
    assert!(uids.windows(2).all(|w| w[0] < w[1]), "unique ids ascend");
    assert_eq!(&bytes[32..36], &[0, 0, 0, 0], "attributes and version");
    let created = be32(&bytes, 36);
    assert!(created > 3_000_000_000, "seconds since 1904: {created}");
    assert_eq!(created, be32(&bytes, 40));
    let e = ereader(&bytes);
    assert_eq!(e.palm.name, b"The Test Book: A Story");
    assert_eq!(e.version(), 10, "zlib compression like Calibre");
    let n = e.palm.records.len();
    let non_text = e.non_text();
    assert!(non_text >= 2);
    let chapters = e.h(7);
    assert_eq!(chapters, 4, "h1 x3 and the h2 of chapter three");
    let chapter_start = e.h(16);
    assert_eq!(chapter_start, non_text, "chapter index follows the text");
    let images = e.h(10);
    assert_eq!(images, 2, "cover plus the inline picture");
    let links = e.h(11);
    assert!(
        links >= 3,
        "an anchor per linked chapter and fragment: {links}"
    );
    let link_start = e.h(21);
    assert_eq!(link_start, chapter_start + chapters);
    let image_start = e.h(20);
    assert_eq!(image_start, link_start + links);
    assert_eq!(e.h(12), 1, "has a metadata record");
    let meta = e.h(22);
    assert_eq!(meta, image_start + images);
    let dummy = meta + 1;
    assert_eq!(n, dummy + 2);
    for idx in [18, 19, 24, 25, 26] {
        assert_eq!(
            e.h(idx),
            dummy,
            "unused index field {idx} points at the empty record"
        );
    }
    let d = &e.palm.records[dummy];
    assert_eq!(d.len(), 4);
    assert_eq!(d[..2], d[2..], "the visible length, twice");
    assert_eq!(
        be16(d, 0),
        bookx_convert::writers::pml::visible_len(&e.text())
    );
    assert_eq!(e.palm.records[n - 1], b"MeTaInFo\0");
    assert_eq!(e.h(3), 0x6240);
    assert_eq!(e.h(17), 0x0a00);
    // unused sections are zero
    for idx in [1, 2, 4, 5, 8, 9, 13, 14, 15] {
        assert_eq!(e.h(idx), 0, "field {idx}");
    }
}

#[test]
fn metadata_record_has_five_nul_terminated_fields() {
    let bytes = pdb(&sample_book());
    let e = ereader(&bytes);
    let rec = &e.palm.records[e.h(22)];
    let fields: Vec<&[u8]> = rec.split(|b| *b == 0).collect();
    assert_eq!(
        fields.len(),
        6,
        "five fields and the empty tail after the last NUL"
    );
    assert_eq!(fields[0], b"The Test Book: A Story");
    assert_eq!(fields[1], b"Ada Lovelace & Grace Hopper");
    assert_eq!(fields[2], b"");
    assert_eq!(fields[3], b"Bookx Press");
    assert_eq!(fields[4], b"9781234567897");
}

#[test]
fn text_records_are_8192_bytes_except_the_last() {
    let big: String = (0..1500)
        .map(|i| format!("<p>Paragraph {i} with some words to fill the record up nicely.</p>"))
        .collect();
    let book = book_with(&[&big], false);
    let e = ereader(&pdb(&book));
    let parts = e.text_records();
    assert!(parts.len() >= 5, "{} text records", parts.len());
    for p in &parts[..parts.len() - 1] {
        assert_eq!(p.len(), 8192);
    }
    assert!(parts.last().unwrap().len() <= 8192);
    assert_eq!(e.non_text(), parts.len() + 1);
    let plain = normalized(&pml_plain(&e.text()));
    assert!(plain.contains("Paragraph 0 with") && plain.contains("Paragraph 1499 with"));
}

#[test]
fn pml_text_markup() {
    let e = ereader(&pdb(&sample_book()));
    let t = e.text();
    assert!(t.is_ascii(), "pure ASCII PML");
    assert!(t.contains("\\x\\BChapter One\\B\\x\n\n"), "{t}");
    assert!(t.contains("\\X0\\BA section\\B\\X0"), "h2 is level \\X0");
    assert!(t.contains("\\iitalic\\i"));
    assert!(t.contains("\\Bbold\\B"));
    assert!(!t.contains("\\a147"), "straight quotes stay ASCII");
    assert!(
        t.contains("Na\\a239ve caf\\a233 \\a151 \\U65e5\\U672c\\U8a9e."),
        "{t}"
    );
    assert!(t.contains("\\m=\"img1.png\""));
    assert!(t.starts_with("\\c\\m=\"cover.png\"\\c\n\n"));
    assert_eq!(
        t.matches("\\p\n").count(),
        3,
        "page break before every chapter after the cover"
    );
    // links: every target is an anchor
    let targets: HashSet<String> = t
        .match_indices("\\q=\"#")
        .map(|(p, _)| t[p + 5..].split('"').next().unwrap().to_string())
        .collect();
    assert_eq!(targets.len(), 2);
    for tg in &targets {
        assert!(t.contains(&format!("\\Q=\"{tg}\"")), "anchor {tg}");
    }
    // external link: text only
    assert!(t.contains("outside"));
    assert!(!t.contains("example.org"));
    // plain text equals the sentences
    assert_sentences(&pml_plain(&t), "pdb text");
}

#[test]
fn chapter_index_points_at_the_chapter_codes() {
    let e = ereader(&pdb(&sample_book()));
    let text = e.text();
    let start = e.h(16);
    let mut found = Vec::new();
    for i in 0..e.h(7) {
        let r = &e.palm.records[start + i];
        let off = be32(r, 0);
        let title = std::str::from_utf8(&r[4..r.len() - 1]).unwrap();
        assert_eq!(r[r.len() - 1], 0);
        let at = &text[off..];
        assert!(
            at.starts_with("\\x") || at.starts_with("\\X"),
            "offset {off} -> {:?}",
            &at[..8]
        );
        assert!(at.contains(&format!("\\B{title}\\B")) || title.is_empty());
        found.push(title.to_string());
    }
    assert_eq!(
        found,
        ["Chapter One", "Chapter Two", "Chapter Three", "A section"]
    );
}

#[test]
fn link_index_points_at_the_anchor_codes() {
    let e = ereader(&pdb(&sample_book()));
    let text = e.text();
    let start = e.h(21);
    let mut names = HashSet::new();
    for i in 0..e.h(11) {
        let r = &e.palm.records[start + i];
        let off = be32(r, 0);
        let name = std::str::from_utf8(&r[4..r.len() - 1]).unwrap();
        assert_eq!(r[r.len() - 1], 0);
        assert!(
            text[off..].starts_with(&format!("\\Q=\"{name}\"")),
            "{name} at {off}: {:?}",
            &text[off..off + 12]
        );
        assert!(names.insert(name.to_string()), "unique names");
    }
    assert_eq!(names.len(), text.matches("\\Q=\"").count());
    for (p, _) in text.match_indices("\\q=\"#") {
        let tg = text[p + 5..].split('"').next().unwrap();
        assert!(names.contains(tg), "link target {tg} is indexed");
    }
}

#[test]
fn image_records_have_62_byte_headers() {
    let e = ereader(&pdb(&sample_book()));
    let first = e.h(20);
    let cover = &e.palm.records[first];
    assert_eq!(&cover[..4], b"PNG ");
    let name_end = 4 + cover[4..36].iter().position(|b| *b == 0).unwrap();
    assert_eq!(&cover[4..name_end], b"cover.png");
    assert!(cover[name_end..58].iter().all(|b| *b == 0));
    assert_eq!((be16(cover, 58), be16(cover, 60)), (300, 450));
    let img = image::load_from_memory(&cover[62..]).unwrap();
    assert_eq!((img.width(), img.height()), (300, 450));
    let pic = &e.palm.records[first + 1];
    assert_eq!(&pic[4..12], b"img1.png");
    assert_eq!((be16(pic, 58), be16(pic, 60)), (40, 30));
    assert_eq!(
        &pic[62..],
        png(40, 30).as_slice(),
        "PNG sources are embedded unchanged"
    );
}

#[test]
fn palmdoc_compression_option() {
    let book = sample_book();
    let bytes = EReaderWriter {
        compression: PdbCompression::PalmDoc,
    }
    .write(&book)
    .unwrap();
    let e = ereader(&bytes);
    assert_eq!(e.version(), 2);
    assert_sentences(&pml_plain(&e.text()), "palmdoc pdb");
    let zlib = ereader(&pdb(&book));
    assert_eq!(e.text(), zlib.text(), "same PML either way");
    assert_ne!(e.palm.records[1], zlib.palm.records[1]);
    // and the formats reader understands it
    let t = epub_text(bookx_formats::pdb::PdbReader.read(&bytes).unwrap());
    assert_sentences(&t, "palmdoc round trip");
}

#[test]
fn long_and_odd_titles_make_valid_names() {
    let mut book = sample_book();
    let mut md = book.metadata().clone();
    md.title = "Ünïcödé 日本語 title that is far longer than thirty one bytes".into();
    book.set_metadata(md).unwrap();
    let bytes = pdb(&book);
    let e = ereader(&bytes);
    assert_eq!(e.palm.name.len(), 31);
    assert!(
        e.palm.name.starts_with(b"\xdcn\xefc\xf6d\xe9 ???"),
        "cp1252 with ? for CJK"
    );
}

#[test]
fn empty_book_has_one_empty_text_record() {
    let book = book_with(&["<div></div>"], false);
    let e = ereader(&pdb(&book));
    assert_eq!(e.non_text(), 2);
    assert_eq!(e.text(), "");
    assert_eq!(e.h(7), 0);
    assert_eq!(e.h(10), 0);
    assert_eq!(
        e.h(20),
        e.h(22) + 1,
        "no images: points at the empty record"
    );
}

#[test]
fn unreadable_images_are_dropped() {
    let mut book = book_with(&["<p>t <img src=\"bad.png\" alt=\"x\"/></p>"], false);
    book.archive.set("OEBPS/bad.png", b"not an image".to_vec());
    let e = ereader(&pdb(&book));
    assert_eq!(e.h(10), 0);
    assert!(!e.text().contains("\\m="));
}

// ---- round trip & oracles --------------------------------------------------------------------

#[test]
fn round_trips_through_the_pdb_reader() {
    let bytes = pdb(&sample_book());
    let archive = bookx_formats::pdb::PdbReader.read(&bytes).unwrap();
    let t = epub_text(archive.clone());
    assert_sentences(&t, "pdb round trip");
    let book = bookx_core::book::OpenBook::from_archive(archive).unwrap();
    let md = book.metadata();
    assert_eq!(md.title, "The Test Book: A Story");
    assert_eq!(md.authors, ["Ada Lovelace", "Grace Hopper"]);
    assert_eq!(md.publisher, "Bookx Press");
    assert_eq!(md.identifier("isbn"), Some("9781234567897"));
    let titles: Vec<String> = bookx_core::toc::flatten(&book.toc())
        .into_iter()
        .map(|f| f.title)
        .collect();
    for want in ["Chapter One", "Chapter Two", "Chapter Three"] {
        assert!(titles.iter().any(|t| t == want), "{titles:?}");
    }
    assert!(
        book.archive
            .names()
            .iter()
            .filter(|n| n.ends_with(".png"))
            .count()
            >= 2
    );
}

#[test]
fn internal_links_survive_the_round_trip() {
    let archive = bookx_formats::pdb::PdbReader
        .read(&pdb(&sample_book()))
        .unwrap();
    let all: String = archive
        .names()
        .iter()
        .filter(|n| n.ends_with("html"))
        .filter_map(|n| archive.get_string(n))
        .collect();
    assert!(all.matches("<a href=\"").count() >= 2, "{all}");
}

#[test]
fn calibre_reads_our_pdb() {
    let bytes = pdb(&sample_book());
    let Some(t) = calibre_text(&bytes, "ours.pdb") else {
        eprintln!("skipped: Calibre not available");
        return;
    };
    assert_sentences(&t, "Calibre read of our pdb");
    assert!(t.contains("日本語"), "\\U escapes understood by Calibre");
}

#[test]
fn calibre_reads_palmdoc_compressed_pdb() {
    let bytes = EReaderWriter {
        compression: PdbCompression::PalmDoc,
    }
    .write(&sample_book())
    .unwrap();
    let Some(t) = calibre_text(&bytes, "ours.pdb") else {
        eprintln!("skipped: Calibre not available");
        return;
    };
    assert_sentences(&t, "Calibre read of palmdoc pdb");
}

#[test]
fn ebook_meta_reads_our_metadata() {
    if !oracle_available() {
        eprintln!("skipped: Calibre not available");
        return;
    }
    let dir = temp_dir("meta");
    let path = dir.join("m.pdb");
    std::fs::write(&path, pdb(&sample_book())).unwrap();
    let out = std::process::Command::new("ebook-meta").arg(&path).output();
    let Ok(out) = out else {
        eprintln!("skipped: ebook-meta not available");
        return;
    };
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("Test Book"), "{s}");
    assert!(s.contains("Ada Lovelace"), "{s}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Record kinds of an eReader file as a compact string (runs collapsed): H(eader) T(ext)
/// C(hapter index) L(ink index) I(mage) M(etadata) D(ummy) E(nd marker).
fn record_kinds(bytes: &[u8]) -> String {
    let e = ereader(bytes);
    let n = e.palm.records.len();
    let (cs, ce) = (e.h(16), e.h(16) + e.h(7));
    let (ls, le) = (e.h(21), e.h(21) + e.h(11));
    let (is, ie) = (e.h(20), e.h(20) + e.h(10));
    let mut s = String::from("H");
    for i in 1..n {
        let c = if i < e.non_text() {
            'T'
        } else if e.h(7) > 0 && (cs..ce).contains(&i) {
            'C'
        } else if e.h(11) > 0 && (ls..le).contains(&i) {
            'L'
        } else if e.h(10) > 0 && (is..ie).contains(&i) {
            'I'
        } else if i == e.h(22) {
            'M'
        } else if e.palm.records[i] == b"MeTaInFo\0" {
            'E'
        } else {
            'D'
        };
        if !s.ends_with(c) {
            s.push(c);
        }
    }
    s
}

#[test]
fn layout_matches_calibres_ereader_output() {
    let book = sample_book();
    let epub = book.archive.to_bytes().unwrap();
    let Some(theirs) = calibre(&epub, "b.epub", "b.pdb", &["--format", "ereader"]) else {
        eprintln!("skipped: Calibre not available");
        return;
    };
    let ours = pdb(&book);
    let (t, o) = (ereader(&theirs), ereader(&ours));
    assert_eq!(t.palm.db_type, o.palm.db_type);
    assert_eq!(t.palm.creator, o.palm.creator);
    assert_eq!(t.version(), o.version());
    for idx in [3, 17] {
        assert_eq!(t.h(idx), o.h(idx), "constant header field {idx}");
    }
    assert_eq!(t.h(12), o.h(12));
    assert_eq!(record_kinds(&theirs), "HTCLIMDE");
    assert_eq!(record_kinds(&theirs), record_kinds(&ours), "record order");
    assert_eq!(t.h(10), 2, "Calibre also embeds cover and picture");
    assert_eq!(o.h(10), 2);
    // the link index of both files names every anchor of the text
    for e in [&t, &o] {
        let text = e.text();
        assert_eq!(e.h(11), text.matches("\\Q=\"").count());
    }
    // an image record header is laid out identically
    let ti = &t.palm.records[t.h(20)];
    let oi = &o.palm.records[o.h(20)];
    assert_eq!(&ti[..4], &oi[..4]);
    assert_eq!(ti[36..58], oi[36..58]);
    assert!(t.palm.records[t.palm.records.len() - 1] == o.palm.records[o.palm.records.len() - 1]);
}

// ---- PMLZ -------------------------------------------------------------------------------------

#[test]
fn pmlz_package_layout() {
    let files = unzip(&PmlzWriter.write(&sample_book()).unwrap());
    let names: Vec<&str> = files.iter().map(|f| f.0.as_str()).collect();
    assert_eq!(
        names,
        [
            "index.pml",
            "index_img/cover.png",
            "index_img/img1.png",
            "metadata.opf"
        ]
    );
    let pml = part_str(&files, "index.pml");
    assert!(pml.starts_with("\\vTITLE=\"The Test Book: A Story\" AUTHOR=\"Ada Lovelace & Grace Hopper\" PUBLISHER=\"Bookx Press\" EISBN=\"9781234567897\"\\v\n\n"), "{pml}");
    assert!(pml.contains("\\m=\"cover.png\"") && pml.contains("\\m=\"img1.png\""));
    assert!(part(&files, "index_img/img1.png").starts_with(&[0x89, b'P', b'N', b'G']));
    // each referenced image exists
    for (p, _) in pml.match_indices("\\m=\"") {
        let name = pml[p + 4..].split('"').next().unwrap();
        assert!(
            files.iter().any(|(n, _)| *n == format!("index_img/{name}")),
            "{name}"
        );
    }
    let opf = part_str(&files, "metadata.opf");
    let doc = roxmltree::Document::parse(&opf).unwrap();
    let title = doc
        .descendants()
        .find(|n| n.tag_name().name() == "title")
        .unwrap();
    assert_eq!(title.text(), Some("The Test Book: A Story"));
    assert_sentences(
        &pml_plain(pml.split_once("\\v\n\n").map(|x| x.1).unwrap_or_default()),
        "pmlz text",
    );
}

#[test]
fn pmlz_without_images_or_metadata() {
    let mut book = book_with(&["<p>bare</p>"], false);
    let md = bookx_core::opf::Metadata::default();
    book.set_metadata(md).unwrap();
    let files = unzip(&PmlzWriter.write(&book).unwrap());
    let names: Vec<&str> = files.iter().map(|f| f.0.as_str()).collect();
    assert_eq!(names, ["index.pml", "metadata.opf"]);
    assert!(!part_str(&files, "index.pml").contains("\\v"));
}

#[test]
fn pmlz_metadata_values_cannot_break_the_markup() {
    let mut book = book_with(&["<p>x</p>"], false);
    let mut md = book.metadata().clone();
    md.title = "Say \"hi\" \\ é".into();
    book.set_metadata(md).unwrap();
    let pml = part_str(&unzip(&PmlzWriter.write(&book).unwrap()), "index.pml");
    assert!(pml.contains("TITLE=\"Say 'hi' \\\\ \\a233\""), "{pml}");
}

#[test]
fn pmlz_round_trips_through_the_reader() {
    let bytes = PmlzWriter.write(&sample_book()).unwrap();
    let archive = bookx_formats::pml::PmlzReader.read(&bytes).unwrap();
    let t = epub_text(archive.clone());
    assert_sentences(&t, "pmlz round trip");
    let book = bookx_core::book::OpenBook::from_archive(archive).unwrap();
    assert_eq!(book.metadata().title, "The Test Book: A Story");
    assert!(PmlzReader_sniffs(&bytes));
}

#[allow(non_snake_case)]
fn PmlzReader_sniffs(bytes: &[u8]) -> bool {
    bookx_formats::pml::PmlzReader.sniff(bytes)
}

#[test]
fn calibre_reads_our_pmlz() {
    let bytes = PmlzWriter.write(&sample_book()).unwrap();
    let Some(t) = calibre_text(&bytes, "ours.pmlz") else {
        eprintln!("skipped: Calibre not available");
        return;
    };
    assert_sentences(&t, "Calibre read of our pmlz");
}

#[test]
fn layout_matches_calibres_pmlz_output() {
    let epub = sample_book().archive.to_bytes().unwrap();
    let Some(theirs) = calibre(&epub, "b.epub", "b.pmlz", &[]) else {
        eprintln!("skipped: Calibre not available");
        return;
    };
    let their_names: Vec<String> = unzip(&theirs).into_iter().map(|f| f.0).collect();
    assert!(their_names.contains(&"index.pml".to_string()));
    assert!(
        their_names
            .iter()
            .any(|n| n.starts_with("index_img/") && n.ends_with(".png"))
    );
    let ours = unzip(&PmlzWriter.write(&sample_book()).unwrap());
    assert!(ours.iter().any(|(n, _)| n == "index.pml"));
    assert!(
        ours.iter()
            .all(|(n, _)| n == "index.pml" || n == "metadata.opf" || n.starts_with("index_img/"))
    );
}
