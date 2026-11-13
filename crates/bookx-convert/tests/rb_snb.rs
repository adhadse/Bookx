//! Rocket eBook (RB) and Shanda Bambook (SNB) writers: decoded with parsers written for the tests
//! (flate2 and the `bzip2` crate, not the formats crate), compared with Calibre's output and read
//! back by the formats crate.

mod common;

use std::io::Read;

use bookx_convert::writers::{RbWriter, SnbWriter};
use bookx_core::contracts::{FormatReader, FormatWriter};
use common::*;

fn le32(b: &[u8], o: usize) -> usize {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize
}
fn be32(b: &[u8], o: usize) -> usize {
    u32::from_be_bytes(b[o..o + 4].try_into().unwrap()) as usize
}

fn normalized(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---- RB ---------------------------------------------------------------------------------------

#[derive(Debug)]
struct RbPage {
    name: String,
    length: usize,
    offset: usize,
    flags: usize,
}

fn rb_toc(f: &[u8]) -> Vec<RbPage> {
    assert_eq!(&f[..4], &[0xb0, 0x0c, 0xb0, 0x0c]);
    let toc = le32(f, 0x18);
    let n = le32(f, toc);
    (0..n)
        .map(|i| {
            let e = &f[toc + 4 + i * 44..toc + 48 + i * 44];
            let end = e[..32].iter().position(|b| *b == 0).unwrap();
            RbPage {
                name: String::from_utf8(e[..end].to_vec()).unwrap(),
                length: le32(e, 32),
                offset: le32(e, 36),
                flags: le32(e, 40),
            }
        })
        .collect()
}

fn rb_page(f: &[u8], p: &RbPage) -> Vec<u8> {
    let raw = &f[p.offset..p.offset + p.length];
    if p.flags & 8 == 0 {
        return raw.to_vec();
    }
    let count = le32(raw, 0);
    let total = le32(raw, 4);
    let mut pos = 8 + count * 4;
    let mut out = Vec::new();
    for i in 0..count {
        let size = le32(raw, 8 + i * 4);
        let mut chunk = Vec::new();
        flate2::read::ZlibDecoder::new(&raw[pos..pos + size])
            .read_to_end(&mut chunk)
            .unwrap();
        assert!(chunk.len() <= 4096);
        if i + 1 < count {
            assert_eq!(chunk.len(), 4096, "all chunks but the last are full");
        }
        pos += size;
        out.extend(chunk);
    }
    assert_eq!(pos, raw.len(), "no trailing bytes");
    assert_eq!(out.len(), total);
    out
}

fn rb_html(f: &[u8]) -> Vec<u8> {
    let toc = rb_toc(f);
    let p = toc.iter().find(|p| p.name == "index.html").unwrap();
    rb_page(f, p)
}

fn rb(book: &bookx_core::book::OpenBook) -> Vec<u8> {
    RbWriter.write(book).unwrap()
}

#[test]
fn rb_header_and_toc() {
    let f = rb(&sample_book());
    assert_eq!(&f[4..6], &[2, 0]);
    assert_eq!(&f[6..10], b"NUVO");
    assert!(f[10..0x18].iter().all(|b| *b == 0));
    assert_eq!(le32(&f, 0x18), 0x128);
    assert_eq!(le32(&f, 0x1c), f.len());
    assert!(f[0x20..0x128].iter().all(|b| *b == 0));
    let toc = rb_toc(&f);
    let names: Vec<&str> = toc.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "info.info",
            "index.html",
            "index.hidx",
            "cover.png",
            "0.png"
        ]
    );
    let flags: Vec<usize> = toc.iter().map(|p| p.flags).collect();
    assert_eq!(flags, [2, 8, 0, 0, 0]);
    // pages are contiguous and fill the file
    let mut pos = 0x128 + 4 + 44 * toc.len();
    for p in &toc {
        assert_eq!(p.offset, pos, "{}", p.name);
        pos += p.length;
    }
    assert_eq!(pos, f.len());
}

#[test]
fn rb_info_page() {
    let f = rb(&sample_book());
    let toc = rb_toc(&f);
    let info = rb_page(&f, &toc[0]);
    let text = String::from_utf8_lossy(&info).to_string();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "TYPE=2");
    for want in [
        "TITLE=The Test Book: A Story",
        "AUTHOR=Ada Lovelace & Grace Hopper",
        "PUB_NAME=Bookx Press",
        "TITLE_LANGUAGE=en",
        "GENRE=testing, fiction",
        "ISBN=9781234567897",
        "GENERATOR=Bookx",
        "PARSE=1",
        "OUTPUT=1",
        "BODY=index.html",
    ] {
        assert!(lines.contains(&want), "missing {want} in {lines:?}");
    }
    assert!(info.ends_with(b"\n"));
}

#[test]
fn rb_html_dialect() {
    let f = rb(&sample_book());
    let html = rb_html(&f);
    let s: String = html.iter().map(|b| *b as char).collect(); // bytes as Latin-1
    assert!(s.starts_with("<HTML><HEAD><TITLE>The Test Book: A Story</TITLE><META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=windows-1252\"></HEAD><BODY>"));
    assert!(s.ends_with("</BODY></HTML>"));
    assert!(s.contains("<H1>Chapter One</H1>"));
    assert!(s.contains("<H2>A section</H2>"));
    assert!(s.contains("<I>italic</I>") && s.contains("<B>bold</B>"));
    assert!(s.contains("&amp; the rain"));
    assert!(s.contains("\"dark\""), "quotes are not entities");
    assert!(
        s.contains("Na\u{ef}ve caf\u{e9} \u{97} &#26085;&#26412;&#35486;."),
        "cp1252 bytes and numeric entities: {s}"
    );
    assert!(s.contains("<PRE><TT>code  line</TT></PRE>"));
    assert!(s.contains("<IMG SRC=\"0.png\">") && s.contains("<IMG SRC=\"cover.png\">"));
    // internal links
    let names: Vec<_> = s
        .match_indices("<A NAME=\"")
        .map(|(p, _)| s[p + 9..].split('"').next().unwrap().to_string())
        .collect();
    let hrefs: Vec<_> = s
        .match_indices("<A HREF=\"#")
        .map(|(p, _)| s[p + 10..].split('"').next().unwrap().to_string())
        .collect();
    assert_eq!(hrefs.len(), 2);
    for h in hrefs {
        assert!(names.contains(&h), "{h}");
    }
    assert!(
        !s.contains("example.org"),
        "external links keep only their text"
    );
    // tags balance (every opening tag has its closing tag)
    for t in ["P", "B", "I", "H1", "A"] {
        assert_eq!(
            s.matches(&format!("<{t}>")).count() + s.matches(&format!("<{t} ")).count(),
            s.matches(&format!("</{t}>")).count(),
            "{t}"
        );
    }
}

#[test]
fn rb_images_are_png_pages() {
    let f = rb(&sample_book());
    let toc = rb_toc(&f);
    let cover = rb_page(&f, &toc[3]);
    let img = image::load_from_memory(&cover).unwrap();
    assert_eq!((img.width(), img.height()), (300, 450));
    assert_eq!(rb_page(&f, &toc[4]), png(40, 30), "PNG passes through");
}

#[test]
fn rb_lists_tables_quotes() {
    let book = book_with(
        &["<ul><li>a</li><li>b<p>more</p></li></ul><ol><li>x</li></ol><table><tr><td>1</td><td>2</td></tr></table><blockquote><p>q</p></blockquote><hr/><p style=\"text-align:center\">c</p><p style=\"text-align:right\">r</p><p>l<br/>m</p>"],
        false,
    );
    let s = String::from_utf8(rb_html(&rb(&book))).unwrap();
    assert!(
        s.contains("<UL><LI>a</LI><LI>b<P>more</P></LI></UL>"),
        "{s}"
    );
    assert!(s.contains("<OL><LI>x</LI></OL>"));
    assert!(s.contains("<P>1 | 2</P>"));
    assert!(s.contains("<BLOCKQUOTE><P>q</P></BLOCKQUOTE>"));
    assert!(s.contains("<HR>"));
    assert!(s.contains("<P ALIGN=\"CENTER\">c</P>") && s.contains("<P ALIGN=\"RIGHT\">r</P>"));
    assert!(s.contains("<P>l<BR>m</P>"));
}

#[test]
fn rb_large_pages_use_several_chunks() {
    let big: String = (0..600)
        .map(|i| format!("<p>Line {i} of a rather long page of text.</p>"))
        .collect();
    let book = book_with(&[&big], false);
    let f = rb(&book);
    let toc = rb_toc(&f);
    let page = &toc[1];
    let raw = &f[page.offset..page.offset + page.length];
    assert!(le32(raw, 0) >= 7, "several chunks: {}", le32(raw, 0));
    let html = String::from_utf8(rb_page(&f, page)).unwrap();
    assert!(html.contains("Line 599 of"));
}

#[test]
fn rb_empty_book_and_odd_titles() {
    let mut book = book_with(&["<div></div>"], false);
    let mut md = book.metadata().clone();
    md.title = "Multi\nline".into();
    md.authors.clear();
    book.set_metadata(md).unwrap();
    let f = rb(&book);
    let info = String::from_utf8(rb_page(&f, &rb_toc(&f)[0])).unwrap();
    assert!(info.contains("TITLE=Multi line\n"));
    assert!(!info.contains("AUTHOR="));
    assert!(rb_html(&f).ends_with(b"</BODY></HTML>"));
}

#[test]
fn rb_round_trips_through_the_rb_reader() {
    let f = rb(&sample_book());
    let archive = bookx_formats::rb::RbReader.read(&f).unwrap();
    let t = epub_text(archive.clone());
    assert_sentences(&t, "rb round trip");
    let book = bookx_core::book::OpenBook::from_archive(archive).unwrap();
    let md = book.metadata();
    assert_eq!(md.title, "The Test Book: A Story");
    assert_eq!(md.publisher, "Bookx Press");
    assert_eq!(md.identifier("isbn"), Some("9781234567897"));
    let titles: Vec<String> = bookx_core::toc::flatten(&book.toc())
        .into_iter()
        .map(|f| f.title)
        .collect();
    for want in ["Chapter One", "Chapter Two", "Chapter Three"] {
        assert!(titles.iter().any(|t| t == want), "{titles:?}");
    }
}

#[test]
fn rb_calibre_reads_ours() {
    let f = rb(&sample_book());
    let Some(t) = calibre_text(&f, "ours.rb") else {
        eprintln!("skipped: Calibre not available");
        return;
    };
    assert_sentences(&t, "Calibre read of our rb");
}

#[test]
fn rb_layout_matches_calibre() {
    let book = sample_book();
    let epub = book.archive.to_bytes().unwrap();
    let Some(theirs) = calibre(&epub, "b.epub", "b.rb", &[]) else {
        eprintln!("skipped: Calibre not available");
        return;
    };
    let ours = rb(&book);
    assert_eq!(theirs[..0x18], ours[..0x18], "magic, version, NUVO");
    assert_eq!(le32(&theirs, 0x18), le32(&ours, 0x18), "toc offset");
    let (t, o) = (rb_toc(&theirs), rb_toc(&ours));
    for i in 0..3 {
        assert_eq!(t[i].name, o[i].name);
        assert_eq!(t[i].flags, o[i].flags, "{}", t[i].name);
    }
    assert_eq!(t[2].length, o[2].length);
    assert_eq!(rb_page(&theirs, &t[2]), rb_page(&ours, &o[2]));
    // images are stored PNG pages in both
    assert!(t[3..]
        .iter()
        .all(|p| p.name.ends_with(".png") && p.flags == 0));
    assert!(o[3..]
        .iter()
        .all(|p| p.name.ends_with(".png") && p.flags == 0));
    let ti = rb_page(&theirs, &t[1]);
    let th = String::from_utf8_lossy(&ti);
    assert!(
        th.starts_with("<HTML>") && th.contains("<H1>"),
        "same dialect as Calibre: {th}"
    );
    // info pages share their keys
    let ik = |f: &[u8], p: &RbPage| -> Vec<String> {
        String::from_utf8_lossy(&rb_page(f, p))
            .lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, _)| k.to_string())
            .collect()
    };
    let theirs_keys = ik(&theirs, &t[0]);
    let our_keys = ik(&ours, &o[0]);
    for k in [
        "TYPE",
        "TITLE",
        "AUTHOR",
        "GENERATOR",
        "PARSE",
        "OUTPUT",
        "BODY",
    ] {
        assert!(theirs_keys.contains(&k.to_string()), "Calibre writes {k}");
        assert!(our_keys.contains(&k.to_string()), "we write {k}");
    }
}

// ---- SNB --------------------------------------------------------------------------------------

struct Snb {
    files: Vec<(u32, String, Vec<u8>)>, // attr, name, data
    block_offsets: Vec<usize>,
    positions: Vec<(usize, usize)>,
    bin_blocks: usize,
    plain_blocks: usize,
}

fn zlib(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(b)
        .read_to_end(&mut out)
        .unwrap();
    out
}

fn snb_parse(f: &[u8]) -> Snb {
    assert_eq!(&f[..8], b"SNBP000B");
    assert_eq!(&f[f.len() - 8..], b"SNBP000B");
    assert_eq!(be32(f, 8), 0x8000);
    assert_eq!(be32(f, 12), 0x00a3_a3a3);
    assert_eq!(be32(f, 16), 0);
    assert_eq!(be32(f, 40), 0);
    let (fc, vs, vc, bs, ps) = (
        be32(f, 20),
        be32(f, 24),
        be32(f, 28),
        be32(f, 32),
        be32(f, 36),
    );
    let vfat = zlib(&f[44..44 + vc]);
    assert_eq!(vfat.len(), vs);
    let tail_size = be32(f, f.len() - 16);
    let tail_off = be32(f, f.len() - 12);
    assert_eq!(tail_off + tail_size + 16, f.len());
    let tail = zlib(&f[tail_off..tail_off + tail_size]);
    let (bb, pb) = (bs.div_ceil(0x8000), ps.div_ceil(0x8000));
    assert_eq!(tail.len(), (bb + pb) * 4 + fc * 8);
    let block_offsets: Vec<usize> = (0..bb + pb).map(|i| be32(&tail, i * 4)).collect();
    let positions: Vec<(usize, usize)> = (0..fc)
        .map(|i| {
            (
                be32(&tail, (bb + pb) * 4 + i * 8),
                be32(&tail, (bb + pb) * 4 + i * 8 + 4),
            )
        })
        .collect();
    // plain stream: independent bzip2 blocks
    let mut plain = Vec::new();
    for i in 0..pb {
        let s = block_offsets[bb + i];
        let e = if i + 1 < pb {
            block_offsets[bb + i + 1]
        } else {
            tail_off
        };
        let mut out = Vec::new();
        bzip2::read::BzDecoder::new(&f[s..e])
            .read_to_end(&mut out)
            .unwrap();
        assert!(out.len() <= 0x8000);
        if i + 1 < pb {
            assert_eq!(out.len(), 0x8000);
        }
        plain.extend(out);
    }
    assert_eq!(plain.len(), ps);
    let bin = &f[44 + vc..44 + vc + bs];
    for (i, off) in block_offsets.iter().take(bb).enumerate() {
        assert_eq!(*off, 44 + vc + i * 0x8000);
    }
    assert_eq!(
        block_offsets.get(bb).copied().unwrap_or(44 + vc + bs),
        44 + vc + bs,
        "plain blocks follow the binary stream"
    );
    let names_at = fc * 12;
    let (mut bp, mut pp) = (0, 0);
    let mut files = Vec::new();
    for i in 0..fc {
        let attr = be32(&vfat, i * 12) as u32;
        let no = be32(&vfat, i * 12 + 4);
        let size = be32(&vfat, i * 12 + 8);
        let rest = &vfat[names_at + no..];
        let name =
            String::from_utf8(rest[..rest.iter().position(|b| *b == 0).unwrap()].to_vec()).unwrap();
        let data = if attr == 0x4100_0000 {
            let d = plain[pp..pp + size].to_vec();
            pp += size;
            d
        } else {
            assert_eq!(attr, 0x0100_0000);
            let d = bin[bp..bp + size].to_vec();
            bp += size;
            d
        };
        files.push((attr, name, data));
    }
    assert_eq!(pp, ps);
    assert_eq!(bp, bs);
    Snb {
        files,
        block_offsets,
        positions,
        bin_blocks: bb,
        plain_blocks: pb,
    }
}

impl Snb {
    fn get(&self, name: &str) -> &[u8] {
        &self
            .files
            .iter()
            .find(|f| f.1 == name)
            .unwrap_or_else(|| {
                panic!(
                    "no {name}: {:?}",
                    self.files.iter().map(|f| &f.1).collect::<Vec<_>>()
                )
            })
            .2
    }
    fn text(&self, name: &str) -> String {
        String::from_utf8(self.get(name).to_vec()).unwrap()
    }
}

fn snb(book: &bookx_core::book::OpenBook) -> Vec<u8> {
    SnbWriter.write(book).unwrap()
}

#[test]
fn snb_container_structure() {
    let f = snb(&sample_book());
    let s = snb_parse(&f);
    let names: Vec<&str> = s.files.iter().map(|f| f.1.as_str()).collect();
    assert_eq!(
        names,
        [
            "snbc/chapter_1.snbc",
            "snbc/chapter_2.snbc",
            "snbc/chapter_3.snbc",
            "snbc/images/cover.jpg",
            "snbc/images/img1.jpg",
            "snbf/book.snbf",
            "snbf/toc.snbf"
        ]
    );
    assert_eq!(s.bin_blocks, 1);
    assert_eq!(s.plain_blocks, 1);
    // file positions: (block, offset within block); binary blocks come first
    let bin_sizes = [
        s.get("snbc/images/cover.jpg").len(),
        s.get("snbc/images/img1.jpg").len(),
    ];
    assert_eq!(s.positions[3], (0, 0));
    assert_eq!(s.positions[4], (0, bin_sizes[0]));
    assert_eq!(s.positions[0], (1, 0));
    assert_eq!(s.positions[1], (1, s.get("snbc/chapter_1.snbc").len()));
}

#[test]
fn snb_xml_parts_are_well_formed() {
    let s = snb_parse(&snb(&sample_book()));
    for (attr, name, data) in &s.files {
        if *attr == 0x4100_0000 {
            let t = std::str::from_utf8(data).unwrap();
            roxmltree::Document::parse(t).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }
    let book = s.text("snbf/book.snbf");
    let d = roxmltree::Document::parse(&book).unwrap();
    let field = |n: &str| {
        d.descendants()
            .find(|x| x.has_tag_name(n))
            .unwrap()
            .text()
            .unwrap_or("")
            .to_string()
    };
    assert_eq!(field("name"), "The Test Book: A Story");
    assert_eq!(field("author"), "Ada Lovelace & Grace Hopper");
    assert_eq!(field("language"), "EN");
    assert_eq!(field("publisher"), "Bookx Press");
    assert_eq!(field("generator"), "Bookx");
    assert_eq!(field("created"), "2020-05-17");
    assert_eq!(field("cover"), "cover.jpg");
    assert!(field("abstract").contains("short description"));
    let toc = s.text("snbf/toc.snbf");
    let d = roxmltree::Document::parse(&toc).unwrap();
    let chapters: Vec<_> = d
        .descendants()
        .filter(|n| n.has_tag_name("chapter"))
        .collect();
    assert_eq!(chapters.len(), 3);
    assert_eq!(field_of(&d, "chapters"), "3");
    for (i, c) in chapters.iter().enumerate() {
        let src = c.attribute("src").unwrap();
        assert_eq!(src, format!("chapter_{}.snbc", i + 1));
        s.get(&format!("snbc/{src}"));
    }
    assert_eq!(chapters[1].text(), Some("Chapter Two"));
}

fn field_of(d: &roxmltree::Document, n: &str) -> String {
    d.descendants()
        .find(|x| x.has_tag_name(n))
        .unwrap()
        .text()
        .unwrap_or("")
        .to_string()
}

#[test]
fn snb_chapter_text_and_images() {
    let s = snb_parse(&snb(&sample_book()));
    let ch2 = s.text("snbc/chapter_2.snbc");
    let d = roxmltree::Document::parse(&ch2).unwrap();
    assert_eq!(field_of(&d, "title"), "Chapter Two");
    let texts: Vec<String> = d
        .descendants()
        .filter(|n| n.has_tag_name("text"))
        .map(|n| n.text().unwrap().to_string())
        .collect();
    assert!(
        texts.iter().all(|t| t.starts_with("\u{3000}\u{3000}")),
        "ideographic indent like Calibre"
    );
    assert_eq!(texts[0], "\u{3000}\u{3000}Chapter Two");
    assert!(texts[1].contains("Naïve café — 日本語."));
    let imgs: Vec<_> = d.descendants().filter(|n| n.has_tag_name("img")).collect();
    assert_eq!(imgs.len(), 1);
    let name = imgs[0].text().unwrap();
    let jpg = s.get(&format!("snbc/images/{name}"));
    let img = image::load_from_memory(jpg).unwrap();
    assert_eq!((img.width(), img.height()), (40, 30));
    assert!(jpg.starts_with(&[0xff, 0xd8, 0xff]));
    let cover = s.get("snbc/images/cover.jpg");
    assert_eq!(cover, jpeg(300, 450), "JPEG covers pass through");
    // image position within the paragraph flow: text, then image, then text
    let order: Vec<_> = d
        .descendants()
        .filter(|n| n.has_tag_name("text") || n.has_tag_name("img"))
        .map(|n| n.tag_name().name().to_string())
        .collect();
    assert_eq!(order, ["text", "text", "img", "text"]);
}

#[test]
fn snb_lists_tables_and_line_breaks() {
    let book = book_with(
        &["<ul><li>a</li></ul><ol><li>x</li><li>y</li></ol><table><tr><td>1</td><td>2</td></tr></table><p>l<br/>m</p><pre>p\n q</pre>]]>"],
        false,
    );
    let s = snb_parse(&snb(&book));
    let ch = s.text("snbc/chapter_1.snbc");
    let d = roxmltree::Document::parse(&ch).unwrap();
    let texts: Vec<String> = d
        .descendants()
        .filter(|n| n.has_tag_name("text"))
        .map(|n| n.text().unwrap().trim_start_matches('\u{3000}').to_string())
        .collect();
    assert_eq!(
        texts,
        ["• a", "1. x", "2. y", "1 | 2", "l", "m", "p", " q", "]]>"]
    );
}

#[test]
fn snb_large_books_use_many_blocks() {
    let big: String = (0..3000)
        .map(|i| {
            format!("<p>Sentence number {i} for the plain stream, long enough to cross blocks.</p>")
        })
        .collect();
    let book = book_with(&[&big, "<p>tail</p>"], false);
    let f = snb(&book);
    let s = snb_parse(&f);
    assert!(s.plain_blocks >= 5, "{} plain blocks", s.plain_blocks);
    let ch1 = s.text("snbc/chapter_1.snbc");
    assert!(ch1.contains("Sentence number 2999 for"));
    // chapter 2 starts in some later block at the right offset
    let (block, off) = s.positions[1];
    let start = ch1.len();
    assert_eq!(
        (block, off),
        (s.bin_blocks + start / 0x8000, start % 0x8000)
    );
    // plain block offsets ascend
    assert!(s.block_offsets.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn snb_book_without_text_has_one_empty_chapter() {
    let book = book_with(&["<div></div>"], false);
    let s = snb_parse(&snb(&book));
    assert!(s.files.iter().any(|f| f.1 == "snbc/chapter_1.snbc"));
    let toc = s.text("snbf/toc.snbf");
    assert!(toc.contains("<chapters>1</chapters>"));
}

#[test]
fn snb_round_trips_through_the_snb_reader() {
    let f = snb(&sample_book());
    let archive = bookx_formats::snb::SnbReader.read(&f).unwrap();
    let t = epub_text(archive.clone());
    assert_sentences(&t, "snb round trip");
    let book = bookx_core::book::OpenBook::from_archive(archive).unwrap();
    let md = book.metadata();
    assert_eq!(md.title, "The Test Book: A Story");
    assert_eq!(md.publisher, "Bookx Press");
    let titles: Vec<String> = bookx_core::toc::flatten(&book.toc())
        .into_iter()
        .map(|f| f.title)
        .collect();
    for want in ["Chapter One", "Chapter Two", "Chapter Three"] {
        assert!(titles.iter().any(|t| t == want), "{titles:?}");
    }
    assert!(book.cover_bytes().is_some(), "cover survives");
}

#[test]
fn snb_calibre_reads_ours() {
    let f = snb(&sample_book());
    let Some(t) = calibre_text(&f, "ours.snb") else {
        eprintln!("skipped: Calibre not available");
        return;
    };
    assert_sentences(&t, "Calibre read of our snb");
    assert!(normalized(&t).contains("Chapter Three"));
}

#[test]
fn snb_layout_matches_calibre() {
    let book = sample_book();
    let epub = book.archive.to_bytes().unwrap();
    let Some(theirs) = calibre(&epub, "b.epub", "b.snb", &[]) else {
        eprintln!("skipped: Calibre not available");
        return;
    };
    let ours = snb(&book);
    let (t, o) = (snb_parse(&theirs), snb_parse(&ours));
    // same header constants (checked by the parser), same stream attributes and name areas
    let kinds = |s: &Snb| -> Vec<(u32, String)> {
        s.files
            .iter()
            .map(|f| (f.0, f.1.split('/').next().unwrap().to_string()))
            .collect()
    };
    let (tk, ok) = (kinds(&t), kinds(&o));
    for (attr, dir) in &ok {
        assert!(
            tk.contains(&(*attr, dir.clone())),
            "Calibre has no {dir} with attribute {attr:#x}"
        );
    }
    // book.snbf has the same fields in the same order
    let fields = |s: &Snb| -> Vec<String> {
        let t = s.text("snbf/book.snbf");
        let d = roxmltree::Document::parse(&t).unwrap();
        d.descendants()
            .find(|n| n.has_tag_name("head"))
            .unwrap()
            .children()
            .filter(|c| c.is_element())
            .map(|c| c.tag_name().name().to_string())
            .collect()
    };
    assert_eq!(fields(&t), fields(&o));
    // chapter bodies use the same element vocabulary
    let tc = t
        .files
        .iter()
        .find(|f| f.1.ends_with(".snbc") && f.2.windows(6).any(|w| w == b"<text>"))
        .unwrap();
    let td = String::from_utf8(tc.2.clone()).unwrap();
    assert!(td.contains("<snbc>") && td.contains("<head>") && td.contains("<body>"));
    // tail layout: position tuples use global block numbers in both files
    assert_eq!(
        t.positions[t.files.iter().position(|f| f.0 == 0x0100_0000).unwrap()].0,
        0
    );
    assert_eq!(
        o.positions[o.files.iter().position(|f| f.0 == 0x0100_0000).unwrap()].0,
        0
    );
    for x in [&t, &o] {
        let first_plain = x.files.iter().position(|f| f.0 == 0x4100_0000).unwrap();
        assert_eq!(x.positions[first_plain], (x.bin_blocks, 0));
    }
}
