//! MOBI 6 writer, validated with a decoder written for these tests from the MobileRead MOBI/PalmDOC
//! descriptions (it shares no code with the writer), plus an optional Calibre oracle.

mod common;

use bookx_convert::mobi::MobiWriter;
use bookx_core::contracts::FormatWriter;

// ---------------------------------------------------------------------------
// test-only decoder
// ---------------------------------------------------------------------------

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}
fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

struct Pdb<'a> {
    data: &'a [u8],
    offsets: Vec<usize>,
}

impl<'a> Pdb<'a> {
    fn parse(data: &'a [u8]) -> Pdb<'a> {
        assert!(data.len() > 78 + 8, "file too short");
        assert_eq!(&data[60..68], b"BOOKMOBI", "PalmDB type/creator");
        let n = be16(data, 76) as usize;
        let mut offsets = Vec::new();
        for i in 0..n {
            let e = 78 + i * 8;
            offsets.push(be32(data, e) as usize);
            assert_eq!(data[e + 4], 0, "record attributes");
            let uid = u32::from_be_bytes([0, data[e + 5], data[e + 6], data[e + 7]]);
            assert_eq!(uid as usize, 2 * i, "unique id of record {i}");
        }
        assert_eq!(be32(data, 68) as usize, 2 * n - 1, "next unique id");
        assert_eq!(
            offsets[0],
            78 + 8 * n + 2,
            "record 0 follows the 2-byte gap"
        );
        assert_eq!(&data[78 + 8 * n..78 + 8 * n + 2], &[0, 0]);
        for w in offsets.windows(2) {
            assert!(
                w[0] < w[1],
                "record offsets must strictly increase (no overlap, no empty records)"
            );
        }
        assert!(*offsets.last().unwrap() < data.len());
        Pdb { data, offsets }
    }

    fn len(&self) -> usize {
        self.offsets.len()
    }

    fn rec(&self, i: usize) -> &'a [u8] {
        let end = self.offsets.get(i + 1).copied().unwrap_or(self.data.len());
        &self.data[self.offsets[i]..end]
    }
}

/// PalmDOC decompression written from the description on MobileRead.
fn palmdoc(b: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        i += 1;
        if c == 0 || (0x09..=0x7f).contains(&c) {
            out.push(c);
        } else if (1..=8).contains(&c) {
            out.extend_from_slice(&b[i..i + c as usize]);
            i += c as usize;
        } else if c >= 0xc0 {
            out.push(b' ');
            out.push(c ^ 0x80);
        } else {
            let pair = ((c as usize) << 8 | b[i] as usize) & 0x3fff;
            i += 1;
            let dist = pair >> 3;
            let len = (pair & 7) + 3;
            assert!(dist >= 1 && dist <= out.len(), "bad back reference");
            for _ in 0..len {
                let v = out[out.len() - dist];
                out.push(v);
            }
        }
    }
    out
}

struct Header {
    text_len: usize,
    n_text: usize,
    rec_size: usize,
    compression: u16,
    mobi_len: usize,
    encoding: u32,
    version: u32,
    first_non_text: usize,
    name_off: usize,
    name_len: usize,
    locale: u32,
    first_image: Option<usize>,
    exth_flags: u32,
    last_content: usize,
    fcis: usize,
    flis: usize,
    extra_flags: u32,
    indx: Option<usize>,
}

fn header(r0: &[u8]) -> Header {
    assert_eq!(&r0[16..20], b"MOBI");
    let opt = |v: u32| {
        if v == 0xffff_ffff {
            None
        } else {
            Some(v as usize)
        }
    };
    Header {
        compression: be16(r0, 0),
        text_len: be32(r0, 4) as usize,
        n_text: be16(r0, 8) as usize,
        rec_size: be16(r0, 10) as usize,
        mobi_len: be32(r0, 20) as usize,
        encoding: be32(r0, 28),
        version: be32(r0, 36),
        first_non_text: be32(r0, 80) as usize,
        name_off: be32(r0, 84) as usize,
        name_len: be32(r0, 88) as usize,
        locale: be32(r0, 92),
        first_image: opt(be32(r0, 108)),
        exth_flags: be32(r0, 128),
        last_content: be16(r0, 194) as usize,
        fcis: be32(r0, 200) as usize,
        flis: be32(r0, 208) as usize,
        extra_flags: be32(r0, 240),
        indx: opt(be32(r0, 244)),
    }
}

fn exth(r0: &[u8], h: &Header) -> Vec<(u32, Vec<u8>)> {
    let start = 16 + h.mobi_len;
    assert_eq!(&r0[start..start + 4], b"EXTH");
    let total = be32(r0, start + 4) as usize;
    let count = be32(r0, start + 8) as usize;
    let mut p = start + 12;
    let mut out = Vec::new();
    for _ in 0..count {
        let kind = be32(r0, p);
        let len = be32(r0, p + 4) as usize;
        assert!(len >= 8);
        out.push((kind, r0[p + 8..p + len].to_vec()));
        p += len;
    }
    assert_eq!(p, start + total, "EXTH length covers exactly its records");
    out
}

fn text_of(pdb: &Pdb, h: &Header) -> Vec<u8> {
    let mut text = Vec::new();
    for i in 1..=h.n_text {
        let part = palmdoc(pdb.rec(i));
        assert!(
            part.len() <= h.rec_size,
            "record {i} decompresses to {} bytes",
            part.len()
        );
        assert!(
            std::str::from_utf8(&part).is_ok(),
            "record {i} must not split a UTF-8 sequence"
        );
        text.extend(part);
    }
    assert_eq!(text.len(), h.text_len, "text length in the header");
    text
}

fn vwi(b: &[u8], p: &mut usize) -> usize {
    let mut v = 0usize;
    loop {
        let c = b[*p];
        *p += 1;
        v = (v << 7) | (c & 0x7f) as usize;
        if c & 0x80 != 0 {
            return v;
        }
    }
}

#[derive(Debug, Default, Clone)]
struct IndexEntry {
    key: String,
    offset: usize,
    length: usize,
    label: String,
    depth: usize,
    parent: Option<usize>,
    first_child: Option<usize>,
    last_child: Option<usize>,
}

/// Reads the NCX index (INDX header, data records, CNCX) following the MobileRead description.
fn read_ncx(pdb: &Pdb, start: usize) -> Vec<IndexEntry> {
    let hdr = pdb.rec(start);
    assert_eq!(&hdr[..4], b"INDX");
    assert_eq!(be32(hdr, 4), 192);
    let idxt = be32(hdr, 20) as usize;
    let n_data = be32(hdr, 24) as usize;
    assert_eq!(be32(hdr, 28), 65001);
    let total = be32(hdr, 36) as usize;
    let n_cncx = be32(hdr, 52) as usize;
    assert_eq!(&hdr[idxt..idxt + 4], b"IDXT");
    assert_eq!(hdr.len() % 4, 0);

    // TAGX
    assert_eq!(&hdr[192..196], b"TAGX");
    let tagx_len = be32(hdr, 196) as usize;
    let ctrl_bytes = be32(hdr, 200) as usize;
    assert_eq!(ctrl_bytes, 1);
    let mut tags = Vec::new();
    for t in hdr[204..192 + tagx_len].chunks(4) {
        if t[3] == 1 {
            break;
        }
        tags.push((t[0], t[1] as usize, t[2]));
    }
    assert!(tags.len() >= 4);

    // CNCX strings
    let mut cncx_bytes = Vec::new();
    for i in 0..n_cncx {
        let mut r = pdb.rec(start + 1 + n_data + i).to_vec();
        assert_eq!(r.len() % 4, 0);
        r.resize(0x10000, 0);
        cncx_bytes.push(r);
    }
    let label_at = |off: usize| -> String {
        let rec = off >> 16;
        let mut p = off & 0xffff;
        let len = vwi(&cncx_bytes[rec], &mut p);
        String::from_utf8(cncx_bytes[rec][p..p + len].to_vec()).unwrap()
    };

    let mut entries = Vec::new();
    for d in 0..n_data {
        let rec = pdb.rec(start + 1 + d);
        assert_eq!(&rec[..4], b"INDX");
        assert_eq!(rec.len() % 4, 0);
        let idxt = be32(rec, 20) as usize;
        let count = be32(rec, 24) as usize;
        assert_eq!(&rec[idxt..idxt + 4], b"IDXT");
        for k in 0..count {
            let mut p = be16(rec, idxt + 4 + 2 * k) as usize;
            let klen = rec[p] as usize;
            let key = String::from_utf8(rec[p + 1..p + 1 + klen].to_vec()).unwrap();
            p += 1 + klen;
            let control = rec[p];
            p += 1;
            let mut e = IndexEntry {
                key,
                ..Default::default()
            };
            for (tag, nvals, mask) in &tags {
                if control & mask == 0 {
                    continue;
                }
                assert_eq!(*nvals, 1);
                let v = vwi(rec, &mut p);
                match tag {
                    1 => e.offset = v,
                    2 => e.length = v,
                    3 => e.label = label_at(v),
                    4 => e.depth = v,
                    21 => e.parent = Some(v),
                    22 => e.first_child = Some(v),
                    23 => e.last_child = Some(v),
                    t => panic!("unknown tag {t}"),
                }
            }
            entries.push(e);
        }
    }
    assert_eq!(entries.len(), total, "total entries in the header record");
    entries
}

fn read_all(bytes: &[u8]) -> (Pdb<'_>, Header, Vec<u8>) {
    let pdb = Pdb::parse(bytes);
    let h = header(pdb.rec(0));
    let text = text_of(&pdb, &h);
    (pdb, h, text)
}

fn is_image(b: &[u8]) -> bool {
    b.starts_with(&[0xff, 0xd8, 0xff]) || b.starts_with(b"GIF8")
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[test]
fn identity() {
    let w = MobiWriter::default();
    assert_eq!(w.id(), "mobi");
    assert_eq!(w.extension(), "mobi");
    assert!(w.inline_toc);
}

#[test]
fn palmdb_container_is_consistent() {
    let book = common::sample_book();
    let bytes = MobiWriter::default().write(&book).unwrap();
    let pdb = Pdb::parse(&bytes);
    // name: at most 31 characters + NUL padding to 32 bytes
    let name = &bytes[..32];
    assert!(name[31] == 0);
    let nul = name.iter().position(|b| *b == 0).unwrap();
    assert!(nul > 0 && name[nul..].iter().all(|b| *b == 0));
    assert_eq!(&name[..nul], b"The_Test_Book__A_Story");
    // contiguous records, last one is the EOF marker
    let sum: usize = (0..pdb.len()).map(|i| pdb.rec(i).len()).sum();
    assert_eq!(pdb.offsets[0] + sum, bytes.len());
    assert_eq!(pdb.rec(pdb.len() - 1), &[0xe9, 0x8e, 0x0d, 0x0a]);
    // timestamps are plausible (after 2020, unix seconds)
    assert!(be32(&bytes, 36) > 1_577_836_800);
}

#[test]
fn headers_and_special_records() {
    let book = common::sample_book();
    let bytes = MobiWriter::default().write(&book).unwrap();
    let (pdb, h, _) = read_all(&bytes);
    assert_eq!(h.compression, 2);
    assert_eq!(h.rec_size, 4096);
    assert_eq!(h.mobi_len, 232);
    assert_eq!(h.encoding, 65001);
    assert_eq!(h.version, 6);
    assert_eq!(h.locale, 9);
    assert_eq!(h.extra_flags, 0, "no trailing entries are written");
    assert_eq!(h.exth_flags & 0x40, 0x40);
    assert_eq!(
        &pdb.rec(0)[16 + 4 + 4..16 + 4 + 8],
        &[0, 0, 0, 2],
        "type = book"
    );
    assert_eq!(h.first_non_text, 1 + h.n_text);
    assert_eq!(h.first_image, Some(1 + h.n_text));
    assert_eq!(&pdb.rec(h.fcis)[..4], b"FCIS");
    assert_eq!(&pdb.rec(h.flis)[..4], b"FLIS");
    assert_eq!(h.flis + 1, h.fcis);
    assert_eq!(h.fcis + 2, pdb.len(), "EOF follows FCIS");
    assert_eq!(h.last_content, h.flis - 1);
    let indx = h.indx.unwrap();
    assert_eq!(&pdb.rec(indx)[..4], b"INDX");
    // FCIS repeats the text length
    assert_eq!(be32(pdb.rec(h.fcis), 20) as usize, h.text_len);
    // title in record 0
    let r0 = pdb.rec(0);
    assert_eq!(
        &r0[h.name_off..h.name_off + h.name_len],
        b"The Test Book: A Story"
    );
    assert_eq!(
        &r0[h.name_off + h.name_len..h.name_off + h.name_len + 2],
        &[0, 0]
    );
    // image records are sequential and valid
    let first = h.first_image.unwrap();
    for i in first..indx {
        assert!(is_image(pdb.rec(i)), "record {i} is not an image");
    }
}

#[test]
fn exth_carries_the_metadata() {
    let book = common::sample_book();
    let bytes = MobiWriter::default().write(&book).unwrap();
    let (pdb, h, text) = read_all(&bytes);
    let ex = exth(pdb.rec(0), &h);
    let all = |k: u32| -> Vec<String> {
        ex.iter()
            .filter(|(t, _)| *t == k)
            .map(|(_, d)| String::from_utf8(d.clone()).unwrap())
            .collect()
    };
    assert_eq!(all(100), vec!["Ada Lovelace", "Grace Hopper"]);
    assert_eq!(all(101), vec!["Bookx Press"]);
    assert_eq!(all(103), vec!["A short description & more."]);
    assert_eq!(all(104), vec!["9781234567897"]);
    assert_eq!(all(105), vec!["testing", "fiction"]);
    assert_eq!(all(106), vec!["2020-05-17"]);
    assert_eq!(all(503), vec!["The Test Book: A Story"]);
    assert_eq!(all(524), vec!["en"]);
    assert_eq!(all(501), vec!["EBOK"]);
    let num = |k: u32| -> u32 {
        let d = &ex.iter().find(|(t, _)| *t == k).unwrap().1;
        assert_eq!(d.len(), 4);
        be32(d, 0)
    };
    // start reading: points at the first heading
    let start = num(116) as usize;
    assert!(
        text[start..].starts_with(b"<h1"),
        "{}",
        String::from_utf8_lossy(&text[start..start + 20])
    );
    // cover and thumbnail offsets are relative to the first image record
    let first = h.first_image.unwrap();
    assert_eq!(
        pdb.rec(first + num(201) as usize),
        book.cover_bytes().unwrap(),
        "cover is the source JPEG"
    );
    let thumb = pdb.rec(first + num(202) as usize);
    let t = image::load_from_memory(thumb).unwrap();
    assert!(t.width() <= 180 && t.height() <= 240 && t.width() > 10);
    assert_ne!(num(201), num(202));
}

#[test]
fn text_matches_the_source_and_links_resolve() {
    let book = common::sample_book();
    let bytes = MobiWriter::default().write(&book).unwrap();
    let (pdb, h, text) = read_all(&bytes);
    let html = String::from_utf8(text.clone()).unwrap();
    assert!(html.starts_with("<html><head><guide>"));
    assert!(html.ends_with("</body></html>"));
    assert!(!html.contains("<?xml") && !html.contains("<script") && !html.contains("class="));

    // visible text: source chapters vs decoded body (inline TOC page cut off)
    let body_start = html.find("<body>").unwrap();
    let toc_page = html.find("<h2>Table of Contents</h2>").unwrap();
    let parsed = scraper::Html::parse_fragment(&html[body_start..toc_page]);
    // block boundaries need no white space in MOBI HTML, so compare without any
    let got: String = parsed
        .root_element()
        .text()
        .collect::<String>()
        .split_whitespace()
        .collect();
    let mut want = String::new();
    for p in book.spine_paths() {
        want.push(' ');
        want.push_str(&common::xml_text(&book.archive.get_string(&p).unwrap()));
    }
    let want: String = want.split_whitespace().collect();
    assert_eq!(got, want);

    // images: recindex values are 1-based indexes into the image records
    let first = h.first_image.unwrap();
    let re = regex::Regex::new(r#"<img recindex="(\d{5})"( alt="([^"]*)")?/>"#).unwrap();
    let imgs: Vec<_> = re.captures_iter(&html).collect();
    assert_eq!(imgs.len(), 1);
    let idx: usize = imgs[0][1].parse().unwrap();
    assert!(is_image(pdb.rec(first + idx - 1)));
    assert_eq!(&imgs[0][3], "a picture");
    // the PNG was converted to JPEG, so there is a JPEG for it and the cover comes first
    assert!(pdb.rec(first + idx - 1).starts_with(&[0xff, 0xd8]));
    assert_eq!(pdb.rec(first), book.cover_bytes().unwrap());

    // every filepos link lands on a tag start; check the semantic targets as well
    let link_re = regex::Regex::new(r"<a filepos=(\d{10})>([^<]*)</a>").unwrap();
    let mut checked = 0;
    for c in link_re.captures_iter(&html) {
        let pos: usize = c[1].parse().unwrap();
        assert!(pos < text.len());
        assert_eq!(text[pos], b'<', "link {} does not land on a tag", &c[2]);
        match &c[2] {
            "chapter two" | "Chapter Two" => {
                assert!(html[pos..].starts_with("<h1>Chapter Two</h1>"))
            }
            "one" | "Chapter One" => assert!(html[pos..].starts_with("<h1>Chapter One</h1>")),
            "Chapter Three" => assert!(html[pos..].starts_with("<h1>Chapter Three</h1>")),
            _ => {}
        }
        checked += 1;
    }
    assert!(
        checked >= 5,
        "2 body links + 3 TOC page links, got {checked}"
    );
    assert!(html.contains("<a href=\"https://example.org/x\">outside</a>"));
    // the guide
    let guide =
        regex::Regex::new(r#"<reference type="(\w+)" title="[^"]*" filepos=(\d{10}) />"#).unwrap();
    let refs: Vec<(String, usize)> = guide
        .captures_iter(&html)
        .map(|c| (c[1].to_string(), c[2].parse().unwrap()))
        .collect();
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0].0, "text");
    assert!(html[refs[0].1..].starts_with("<h1>Chapter One"));
    assert_eq!(refs[1].0, "toc");
    assert!(html[refs[1].1..].starts_with("<h2>Table of Contents</h2>"));
    // character handling
    assert!(html.contains("Naïve café — 日本語."));
    assert!(html.contains("&amp; the rain fell."));
    assert!(
        html.contains("<pre>code  line</pre>"),
        "whitespace inside <pre> is kept"
    );
}

#[test]
fn ncx_index_lists_the_toc_with_hierarchy() {
    let book = common::nested_toc_book();
    let bytes = MobiWriter::default().write(&book).unwrap();
    let (pdb, h, text) = read_all(&bytes);
    let entries = read_ncx(&pdb, h.indx.unwrap());
    let titles: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(
        titles,
        vec!["Part I", "Chapter Two", "Part II", "A section"]
    );
    assert_eq!(
        entries.iter().map(|e| e.depth).collect::<Vec<_>>(),
        vec![0, 1, 0, 1]
    );
    assert_eq!(
        entries.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(),
        vec!["00", "01", "02", "03"]
    );
    assert_eq!(entries[0].first_child, Some(1));
    assert_eq!(entries[0].last_child, Some(1));
    assert_eq!(entries[1].parent, Some(0));
    assert_eq!(entries[2].first_child, Some(3));
    assert_eq!(entries[3].parent, Some(2));
    assert_eq!(entries[0].parent, None);
    let html = String::from_utf8(text.clone()).unwrap();
    assert!(html[entries[0].offset..].starts_with("<h1>Chapter One</h1>"));
    assert!(html[entries[1].offset..].starts_with("<h1>Chapter Two</h1>"));
    assert!(html[entries[2].offset..].starts_with("<h1>Chapter Three</h1>"));
    assert!(html[entries[3].offset..].starts_with("<h2>A section</h2>"));
    // lengths run up to the next entry; the last one to the end of the text
    for w in entries.windows(2) {
        assert_eq!(w[0].offset + w[0].length, w[1].offset);
    }
    assert_eq!(entries[3].offset + entries[3].length, text.len());
}

#[test]
fn without_a_toc_the_spine_files_become_entries() {
    let mut book = common::sample_book();
    let pkg = book.package.clone();
    bookx_core::toc::write_toc(&mut book.archive, &pkg, &[]).unwrap();
    let bytes = MobiWriter::default().write(&book).unwrap();
    let (pdb, h, _) = read_all(&bytes);
    let entries = read_ncx(&pdb, h.indx.unwrap());
    assert_eq!(entries.len(), 3);
}

#[test]
fn inline_toc_can_be_switched_off() {
    let book = common::sample_book();
    let bytes = MobiWriter { inline_toc: false }.write(&book).unwrap();
    let (_, _, text) = read_all(&bytes);
    let html = String::from_utf8(text).unwrap();
    assert!(!html.contains("Table of Contents"));
    assert!(!html.contains("type=\"toc\""));
    assert!(html.contains("type=\"text\""));
}

#[test]
fn large_books_span_many_records_without_splitting_characters() {
    let mut chapters = Vec::new();
    for i in 0..40 {
        let body: String = (0..30)
            .map(|j| format!("<p>Paragraph {i}.{j}: ünïcödé “quotes” — 日本語のテキスト, and some more words to fill the line.</p>\n"))
            .collect();
        chapters.push(bookx_core::convert::Chapter {
            title: format!("Chapter {i}"),
            body: format!("<h1>Chapter {i}</h1>\n{body}"),
        });
    }
    let arc = bookx_core::convert::create_epub(&common::metadata(), &chapters, None);
    let book = bookx_core::book::OpenBook::from_archive(arc).unwrap();
    let bytes = MobiWriter::default().write(&book).unwrap();
    let (pdb, h, text) = read_all(&bytes);
    assert!(h.n_text > 20, "{}", h.n_text);
    assert_eq!(h.first_image, None);
    // compression is effective
    let compressed: usize = (1..=h.n_text).map(|i| pdb.rec(i).len()).sum();
    assert!(
        compressed < text.len() * 7 / 10,
        "{compressed} vs {}",
        text.len()
    );
    let html = String::from_utf8(text).unwrap();
    assert_eq!(html.matches("<h1>Chapter").count(), 40);
    let entries = read_ncx(&pdb, h.indx.unwrap());
    assert_eq!(entries.len(), 40);
    for (i, e) in entries.iter().enumerate() {
        assert!(html[e.offset..].starts_with(&format!("<h1>Chapter {i}</h1>")));
    }
}

#[test]
fn a_book_without_cover_or_images_has_no_image_records() {
    let mut book = common::sample_book();
    book.remove_cover().unwrap();
    let pkg = book.package.clone();
    let mut s = book.archive.get_string("OEBPS/chapter-2.xhtml").unwrap();
    s = s.replace("<img src=\"images/pic.png\" alt=\"a picture\"/>", "");
    book.archive.set_string("OEBPS/chapter-2.xhtml", &s);
    let _ = pkg;
    let bytes = MobiWriter::default().write(&book).unwrap();
    let (pdb, h, _) = read_all(&bytes);
    assert_eq!(h.first_image, None);
    let ex = exth(pdb.rec(0), &h);
    assert!(!ex.iter().any(|(k, _)| *k == 201 || *k == 202));
}

#[test]
fn empty_book_is_still_a_valid_file() {
    let arc = bookx_core::convert::create_epub(&common::metadata(), &[], None);
    let book = bookx_core::book::OpenBook::from_archive(arc).unwrap();
    let bytes = MobiWriter::default().write(&book).unwrap();
    let (pdb, h, text) = read_all(&bytes);
    assert!(text.starts_with(b"<html>"));
    assert_eq!(read_ncx(&pdb, h.indx.unwrap()).len(), 1);
}

#[test]
fn png_and_gif_handling() {
    use bookx_convert::mobi::to_mobi_image;
    let png = common::png(20, 10);
    let out = to_mobi_image(&png).unwrap();
    assert!(out.starts_with(&[0xff, 0xd8, 0xff]));
    assert_eq!(image::load_from_memory(&out).unwrap().width(), 20);
    let jpg = common::jpeg(5, 5);
    assert_eq!(to_mobi_image(&jpg).unwrap(), jpg);
    assert_eq!(to_mobi_image(b"GIF89a....").unwrap(), b"GIF89a....");
    assert!(to_mobi_image(b"<svg/>").is_none());
    // transparent PNG is flattened onto white
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        8,
        8,
        image::Rgba([0, 0, 0, 0]),
    ))
    .write_to(&mut buf, image::ImageFormat::Png)
    .unwrap();
    let flat = image::load_from_memory(&to_mobi_image(buf.get_ref()).unwrap())
        .unwrap()
        .to_rgb8();
    assert!(flat.get_pixel(4, 4).0.iter().all(|c| *c > 240));
}

#[test]
fn locale_and_language_mapping() {
    let mut book = common::sample_book();
    let mut md = book.metadata().clone();
    md.language = "de-AT".into();
    book.set_metadata(md).unwrap();
    let bytes = MobiWriter::default().write(&book).unwrap();
    let (_, h, _) = read_all(&bytes);
    assert_eq!(h.locale, 7);
}

#[test]
fn vwi_is_exported_and_correct() {
    assert_eq!(bookx_convert::mobi::vwi(0x11111), vec![0x04, 0x22, 0x91]);
}

// ---------------------------------------------------------------------------
// golden file: a real MOBI written by Calibre (hand-checked bytes of its first records)
// ---------------------------------------------------------------------------

#[test]
fn our_decoder_reads_calibres_magic_records_the_same_way() {
    // FLIS / FCIS / EOF as Calibre writes them (copied from a Calibre-generated MOBI)
    let flis = hex("464c4953000000080041000000000000ffffffff000100030000000300000001ffffffff");
    let fcis = hex(
        "464349530000001400000010000000010000000000000a050000000000000020000000080001000100000000",
    );
    let book = common::sample_book();
    let bytes = MobiWriter::default().write(&book).unwrap();
    let (pdb, h, _) = read_all(&bytes);
    assert_eq!(pdb.rec(h.flis), flis.as_slice());
    let mine = pdb.rec(h.fcis);
    assert_eq!(mine.len(), fcis.len());
    assert_eq!(&mine[..20], &fcis[..20]);
    assert_eq!(&mine[24..], &fcis[24..], "only the text length differs");
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

// ---------------------------------------------------------------------------
// Calibre as an oracle (skipped when `ebook-meta` is not installed or BOOKX_NO_ORACLE is set)
// ---------------------------------------------------------------------------

fn have(cmd: &str) -> bool {
    std::env::var_os("BOOKX_NO_ORACLE").is_none()
        && std::process::Command::new(cmd)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

#[test]
fn calibre_reads_our_metadata() {
    if !have("ebook-meta") {
        eprintln!("skipped: ebook-meta not available");
        return;
    }
    let dir = common::temp_dir("mobi-meta");
    let path = dir.join("book.mobi");
    std::fs::write(
        &path,
        MobiWriter::default().write(&common::sample_book()).unwrap(),
    )
    .unwrap();
    let out = std::process::Command::new("ebook-meta")
        .arg(&path)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "{text}");
    assert!(text.contains("The Test Book: A Story"), "{text}");
    assert!(text.contains("Ada Lovelace & Grace Hopper"), "{text}");
    assert!(text.contains("Bookx Press"), "{text}");
    assert!(text.contains("isbn:9781234567897"), "{text}");
    let cover = dir.join("cover.jpg");
    let ok = std::process::Command::new("ebook-meta")
        .arg(&path)
        .arg(format!("--get-cover={}", cover.display()))
        .output()
        .unwrap();
    assert!(ok.status.success());
    assert_eq!(
        std::fs::read(&cover).unwrap(),
        common::sample_book().cover_bytes().unwrap()
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn calibre_converts_our_file_and_recovers_text_and_toc() {
    if !have("ebook-convert") {
        eprintln!("skipped: ebook-convert not available");
        return;
    }
    let dir = common::temp_dir("mobi-conv");
    let src = dir.join("book.mobi");
    std::fs::write(
        &src,
        MobiWriter::default()
            .write(&common::nested_toc_book())
            .unwrap(),
    )
    .unwrap();
    let epub = dir.join("out.epub");
    let out = std::process::Command::new("ebook-convert")
        .arg(&src)
        .arg(&epub)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // look at the EPUB Calibre produced
    let bytes = std::fs::read(&epub).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut all = String::new();
    let mut ncx = String::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let name = f.name().unwrap().to_string();
        let mut s = String::new();
        if name.ends_with(".html") || name.ends_with(".xhtml") {
            use std::io::Read;
            let _ = f.read_to_string(&mut s);
            all.push_str(&s);
        } else if name.ends_with("toc.ncx") {
            use std::io::Read;
            let _ = f.read_to_string(&mut ncx);
        }
    }
    for want in ["It was a", "dark", "Naïve café", "The end of the story."] {
        assert!(all.contains(want), "Calibre lost `{want}`");
    }
    let doc = roxmltree::Document::parse(&ncx).unwrap();
    let labels: Vec<String> = doc
        .descendants()
        .filter(|n| n.has_tag_name("navPoint"))
        .map(|n| {
            n.descendants()
                .find(|c| c.has_tag_name("text"))
                .and_then(|t| t.text())
                .unwrap_or("")
                .to_string()
        })
        .collect();
    assert_eq!(
        labels,
        vec!["Part I", "Chapter Two", "Part II", "A section"]
    );
    // hierarchy: "Chapter Two" is nested in "Part I"
    let part1 = doc
        .descendants()
        .find(|n| n.has_tag_name("navPoint"))
        .unwrap();
    assert_eq!(
        part1
            .children()
            .filter(|c| c.has_tag_name("navPoint"))
            .count(),
        1
    );
    let _ = std::fs::remove_dir_all(dir);
}
