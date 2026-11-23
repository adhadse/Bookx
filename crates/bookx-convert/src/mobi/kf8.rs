//! AZW3 / KF8 writer.
//!
//! A KF8-only PalmDB file: version-8 MOBI header, EXTH, PalmDOC-compressed text made of the
//! *flows* described by an FDST record, and four index tables:
//!
//! * flow 0 is the markup. Every spine document is stored as a **skeleton** (the document with the
//!   inside of `<body>` removed) immediately followed by one **fragment** (that body content). The
//!   SKEL table says where each skeleton and its fragment count live; the FRAG table says where a
//!   fragment is inserted into its skeleton and names the skeleton's `<body aid="…">` through a CNCX
//!   selector string `P-//*[@aid='…']`.
//! * flows 1.. are the style sheets, referenced as `kindle:flow:NNNN?mime=text/css`.
//! * images live in records after the indexes and are referenced as
//!   `kindle:embed:NNNN?mime=image/…` (1-based, base 32).
//! * internal links are `kindle:pos:fid:FFFF:off:OOOOOOOOOO` (fragment number and byte offset inside
//!   the fragment, both base 32).
//! * the NCX table lists the table of contents in level order with parent/child links.
//!
//! The layout was derived from the format as read by Calibre's KF8 reader and from files produced
//! by Calibre; the tests compare against both.

use std::collections::HashMap;

use anyhow::{Result, bail};
use bookx_core::book::OpenBook;
use bookx_core::contracts::FormatWriter;
use bookx_core::toc::TocEntry;
use bookx_core::util::{extension, is_external, parent_dir, resolve_href};
use regex::Regex;

use super::html::{doc_title, thumbnail};
use super::indx::{Cncx, RawEntry, TagDef, build_index, vwi};
use super::palmdoc;
use super::{
    NO_VALUE, TEXT_RECORD, assemble_pdb, exth_record, locale_for, pad4, put_u32, split_text,
};
use crate::markup::{
    Tok, body_range, decode_entities, get_attr, remove_attr, set_attr, tag_info, tokenize,
};
use crate::transforms::css::{CssItem, parse_css};

const KF8_HEADER_LEN: usize = 264;
const B32: &[u8; 32] = b"0123456789ABCDEFGHIJKLMNOPQRSTUV";
const LINK_TEMPLATE: &str = "kindle:pos:fid:0000:off:0000000000";

/// Base-32 (digits `0-9A-V`), zero padded to `width`.
pub fn base32(mut n: usize, width: usize) -> String {
    let mut digits = Vec::new();
    while n > 0 {
        digits.push(B32[n & 31]);
        n >>= 5;
    }
    while digits.len() < width {
        digits.push(b'0');
    }
    digits.reverse();
    String::from_utf8(digits).expect("ASCII")
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Azw3Writer;

impl FormatWriter for Azw3Writer {
    fn id(&self) -> &'static str {
        "azw3"
    }
    fn extension(&self) -> &'static str {
        "azw3"
    }
    fn write(&self, book: &OpenBook) -> Result<Vec<u8>> {
        write_azw3(book)
    }
}

type Key = (String, String);

fn sniff_mime(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("image/png")
    } else if data.starts_with(b"GIF8") {
        Some("image/gif")
    } else {
        None
    }
}

struct Part {
    pre: String,
    post: String,
    chunk: String,
    aid: String,
    anchors: HashMap<String, usize>,
    links: Vec<(usize, Key)>,
}

struct Ctx<'a> {
    book: &'a OpenBook,
    resources: Vec<(Vec<u8>, &'static str)>,
    res_ix: HashMap<String, usize>,
    css_flow: HashMap<String, usize>,
    aid: usize,
}

impl Ctx<'_> {
    fn next_aid(&mut self) -> String {
        let a = base32(self.aid, 1);
        self.aid += 1;
        a
    }

    /// `(1-based resource number, mime)` for the image at `path`.
    fn resource(&mut self, path: &str) -> Option<(usize, &'static str)> {
        if let Some(i) = self.res_ix.get(path) {
            return Some((*i, self.resources[*i - 1].1));
        }
        let data = self.book.archive.get(path)?;
        let mime = sniff_mime(data)?;
        self.resources.push((data.to_vec(), mime));
        let idx = self.resources.len();
        self.res_ix.insert(path.to_string(), idx);
        Some((idx, mime))
    }

    fn embed_uri(&mut self, doc_dir: &str, src: &str) -> Option<String> {
        let (path, _) = resolve_href(doc_dir, &decode_entities(src));
        let (idx, mime) = self.resource(&path)?;
        Some(format!("kindle:embed:{}?mime={mime}", base32(idx, 4)))
    }

    fn rewrite_head(&mut self, path: &str, prefix: &str) -> (String, String) {
        let dir = parent_dir(path).to_string();
        let mut out = String::with_capacity(prefix.len());
        let mut skip = 0usize;
        let body_aid = self.next_aid();
        for t in tokenize(prefix) {
            match t {
                Tok::Tag(tag) => {
                    if tag.len() > 9 && tag[..9].eq_ignore_ascii_case("<!DOCTYPE")
                        || tag.starts_with("<!--")
                    {
                        continue;
                    }
                    let Some(info) = tag_info(tag) else {
                        out.push_str(tag);
                        continue;
                    };
                    match info.name.as_str() {
                        "script" if !info.self_closing => {
                            if info.closing {
                                skip = skip.saturating_sub(1)
                            } else {
                                skip += 1
                            }
                        }
                        "link" if !info.closing => {
                            let rel = get_attr(tag, "rel")
                                .unwrap_or_default()
                                .to_ascii_lowercase();
                            let href = get_attr(tag, "href").unwrap_or_default();
                            if rel.contains("stylesheet") {
                                let (target, _) = resolve_href(&dir, &decode_entities(&href));
                                if let Some(n) = self.css_flow.get(&target) {
                                    out.push_str(&format!(
                                        "<link rel=\"stylesheet\" type=\"text/css\" href=\"kindle:flow:{}?mime=text/css\"/>",
                                        base32(*n, 4)
                                    ));
                                }
                            }
                        }
                        "body" if !info.closing => {
                            out.push_str(&set_attr(tag, "aid", &body_aid));
                        }
                        _ if skip > 0 => {}
                        _ => out.push_str(tag),
                    }
                }
                Tok::Text(s) if skip == 0 => out.push_str(s),
                Tok::Text(_) => {}
            }
        }
        (out, body_aid)
    }

    fn rewrite_body(
        &mut self,
        path: &str,
        body: &str,
    ) -> (String, HashMap<String, usize>, Vec<(usize, Key)>) {
        let dir = parent_dir(path).to_string();
        let mut out = String::with_capacity(body.len() + body.len() / 4);
        let mut anchors = HashMap::new();
        let mut links = Vec::new();
        let (mut skip, mut svg) = (0usize, 0usize);
        for t in tokenize(body) {
            match t {
                Tok::Text(s) => {
                    if skip == 0 && svg == 0 {
                        out.push_str(s);
                    }
                }
                Tok::Tag(tag) => {
                    let Some(info) = tag_info(tag) else {
                        if !tag.starts_with("<!--") && svg == 0 && skip == 0 {
                            out.push_str(tag);
                        }
                        continue;
                    };
                    let name = info.name.as_str();
                    match name {
                        "script" | "title" | "head" if !info.self_closing => {
                            if info.closing {
                                skip = skip.saturating_sub(1)
                            } else {
                                skip += 1
                            }
                            continue;
                        }
                        "svg" | "math" if !info.self_closing => {
                            if info.closing {
                                svg = svg.saturating_sub(1)
                            } else {
                                svg += 1
                            }
                            continue;
                        }
                        _ => {}
                    }
                    if skip > 0 {
                        continue;
                    }
                    if svg > 0 {
                        if name == "image" && !info.closing {
                            if let Some(h) =
                                get_attr(tag, "xlink:href").or_else(|| get_attr(tag, "href"))
                            {
                                if let Some(uri) = self.embed_uri(&dir, &h) {
                                    out.push_str(&format!("<img src=\"{uri}\" alt=\"\"/>"));
                                }
                            }
                        }
                        continue;
                    }
                    if matches!(name, "link" | "meta" | "base") {
                        continue;
                    }
                    if info.closing {
                        out.push_str(tag);
                        continue;
                    }
                    if !info.closing {
                        for attr in ["id", "name"] {
                            if let Some(v) = get_attr(tag, attr) {
                                anchors.entry(decode_entities(&v)).or_insert(out.len());
                            }
                        }
                    }
                    if name == "img" {
                        let Some(src) = get_attr(tag, "src") else {
                            continue;
                        };
                        let Some(uri) = self.embed_uri(&dir, &src) else {
                            continue;
                        };
                        out.push_str(&set_attr(tag, "src", &uri));
                        continue;
                    }
                    let mut tag_out = tag.to_string();
                    if name == "a" {
                        if let Some(href) = get_attr(tag, "href") {
                            let href = decode_entities(&href);
                            if !is_external(&href) {
                                let (target, frag) = resolve_href(&dir, &href);
                                let target = if target.is_empty() {
                                    path.to_string()
                                } else {
                                    target
                                };
                                tag_out = set_attr(tag, "href", LINK_TEMPLATE);
                                links.push((
                                    out.len() + tag_out.find(LINK_TEMPLATE).unwrap_or(0),
                                    (target, frag.unwrap_or_default()),
                                ));
                            }
                        }
                    }
                    if !info.self_closing && !matches!(name, "style") {
                        let aid = self.next_aid();
                        tag_out = set_attr(&tag_out, "aid", &aid);
                    }
                    out.push_str(&tag_out);
                }
            }
        }
        (out, anchors, links)
    }

    fn build_part(&mut self, path: &str, html: &str) -> Part {
        let (pre_src, body, post_src) = match body_range(html) {
            Some((s, e)) => (html[..s].to_string(), &html[s..e], html[e..].to_string()),
            None => (
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\">\n<head><title></title></head>\n<body>".to_string(),
                html,
                "</body>\n</html>\n".to_string(),
            ),
        };
        let (pre, aid) = self.rewrite_head(path, &pre_src);
        // white space before the first element would make "start of file" links point at nothing
        let (mut chunk, anchors, links) = self.rewrite_body(path, body.trim_start());
        let mut post = String::new();
        for t in tokenize(&post_src) {
            if let Tok::Tag(g) = t {
                if g.starts_with("<!--") {
                    continue;
                }
            }
            post.push_str(t.as_str());
        }
        if chunk.trim().is_empty() {
            chunk = "\n".to_string();
        }
        Part {
            pre,
            post,
            chunk,
            aid,
            anchors,
            links,
        }
    }

    /// Rewrites `url()` references of a style sheet; font rules are dropped.
    fn rewrite_css(&mut self, path: &str, css: &str) -> String {
        let dir = parent_dir(path).to_string();
        let url_re = Regex::new(r#"url\(\s*(["']?)([^"')]+)(["']?)\s*\)"#).expect("static regex");
        let mut out = String::with_capacity(css.len());
        for item in parse_css(css) {
            let text = match item {
                CssItem::Rule { selectors, block } => format!("{selectors}{block}"),
                CssItem::At(s) => {
                    let head = s.trim_start().to_ascii_lowercase();
                    if head.starts_with("@font-face")
                        || head.starts_with("@import")
                        || head.starts_with("@charset")
                    {
                        continue;
                    }
                    s.to_string()
                }
                CssItem::Other(s) => s.to_string(),
            };
            let replaced = url_re.replace_all(&text, |c: &regex::Captures| {
                let target = &c[2];
                if target.starts_with("data:") || is_external(target) {
                    return c[0].to_string();
                }
                match self.embed_uri(&dir, target) {
                    Some(uri) => format!("url({uri})"),
                    None => "none".to_string(),
                }
            });
            out.push_str(&replaced);
        }
        out
    }
}

fn flatten(entries: &[TocEntry], depth: usize, out: &mut Vec<(String, String, usize)>) {
    for e in entries {
        out.push((e.title.clone(), e.href.clone(), depth));
        flatten(&e.children, depth + 1, out);
    }
}

fn write_azw3(book: &OpenBook) -> Result<Vec<u8>> {
    let md = book.metadata();
    let mut cx = Ctx {
        book,
        resources: Vec::new(),
        res_ix: HashMap::new(),
        css_flow: HashMap::new(),
        aid: 0,
    };

    // resources 1 and 2: cover and its thumbnail
    let mut cover = None;
    if let Some(p) = book.package.cover_path() {
        if let Some((idx, _)) = cx.resource(&p) {
            cover = Some(idx);
        }
    }
    let mut thumb = None;
    if cover.is_some() {
        if let Some(t) = book.cover_bytes().and_then(thumbnail) {
            cx.resources.push((t, "image/jpeg"));
            thumb = Some(cx.resources.len());
        }
    }

    // style sheets become flows 1..
    let css_paths: Vec<String> = book
        .package
        .manifest
        .iter()
        .filter(|i| i.media_type == "text/css")
        .map(|i| book.package.item_path(i))
        .filter(|p| book.archive.contains(p))
        .collect();
    for (i, p) in css_paths.iter().enumerate() {
        cx.css_flow.insert(p.clone(), i + 1);
    }

    // documents
    let spine = book.spine_paths();
    let mut parts: Vec<(String, Part)> = Vec::new();
    for path in &spine {
        let Some(html) = book.archive.get_string(path) else {
            continue;
        };
        let part = cx.build_part(path, &html);
        parts.push((path.clone(), part));
    }
    if parts.is_empty() {
        let part = cx.build_part(
            "empty.xhtml",
            "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title></title></head><body><p></p></body></html>",
        );
        parts.push(("empty.xhtml".to_string(), part));
    }

    let css_flows: Vec<String> = css_paths
        .iter()
        .map(|p| {
            let css = book.archive.get_string(p).unwrap_or_default();
            cx.rewrite_css(p, &css)
        })
        .collect();

    // positions of every skeleton, fragment and anchor
    struct Place {
        skel_pos: usize,
        skel_len: usize,
        insert_pos: usize,
        chunk_len: usize,
    }
    let mut places = Vec::new();
    let mut cursor = 0usize;
    for (_, p) in &parts {
        let skel_len = p.pre.len() + p.post.len();
        places.push(Place {
            skel_pos: cursor,
            skel_len,
            insert_pos: cursor + p.pre.len(),
            chunk_len: p.chunk.len(),
        });
        cursor += skel_len + p.chunk.len();
    }
    let flow0_len = cursor;

    let mut pos: HashMap<Key, (usize, usize)> = HashMap::new();
    for (fid, (path, p)) in parts.iter().enumerate() {
        pos.insert((path.clone(), String::new()), (fid, 0));
        for (id, off) in &p.anchors {
            pos.entry((path.clone(), id.clone())).or_insert((fid, *off));
        }
    }
    // patch links
    for (path, p) in parts.iter_mut() {
        let links = std::mem::take(&mut p.links);
        for (at, key) in links {
            let (fid, off) = pos
                .get(&key)
                .or_else(|| pos.get(&(key.0.clone(), String::new())))
                .copied()
                .unwrap_or_else(|| {
                    pos.get(&(path.clone(), String::new()))
                        .copied()
                        .unwrap_or((0, 0))
                });
            let fixed = format!("kindle:pos:fid:{}:off:{}", base32(fid, 4), base32(off, 10));
            p.chunk.replace_range(at..at + LINK_TEMPLATE.len(), &fixed);
        }
    }

    // the text stream: flow 0 (skeletons + fragments), then one flow per style sheet
    let mut stream: Vec<u8> = Vec::with_capacity(flow0_len);
    for (_, p) in &parts {
        stream.extend_from_slice(p.pre.as_bytes());
        stream.extend_from_slice(p.post.as_bytes());
        stream.extend_from_slice(p.chunk.as_bytes());
    }
    debug_assert_eq!(stream.len(), flow0_len);
    let mut flow_bounds = vec![(0u32, flow0_len as u32)];
    for f in &css_flows {
        let start = stream.len();
        stream.extend_from_slice(f.as_bytes());
        flow_bounds.push((start as u32, stream.len() as u32));
    }
    if stream.len() > u32::MAX as usize / 2 {
        bail!("the book is too large for the AZW3 format");
    }

    // --- index tables -------------------------------------------------------------------------
    // FRAG
    let mut frag_cncx = Cncx::default();
    let frag_entries: Vec<RawEntry> = parts
        .iter()
        .zip(&places)
        .enumerate()
        .map(|(i, ((_, p), pl))| {
            let off = frag_cncx.add(&format!("P-//*[@aid='{}']", p.aid));
            let mut body = vec![0x0f];
            for v in [off, i, i, 0, pl.chunk_len] {
                body.extend(vwi(v));
            }
            RawEntry {
                key: format!("{:010}", pl.insert_pos).into_bytes(),
                body,
            }
        })
        .collect();
    let frag_tags = [
        TagDef(2, 1, 1),
        TagDef(3, 1, 2),
        TagDef(4, 1, 4),
        TagDef(6, 2, 8),
    ];
    let frag_records = build_index(&frag_tags, &frag_entries, frag_cncx.finish());

    // SKEL
    let skel_entries: Vec<RawEntry> = places
        .iter()
        .enumerate()
        .map(|(i, pl)| {
            let mut body = vec![0x0a];
            for v in [1, 1, pl.skel_pos, pl.skel_len, pl.skel_pos, pl.skel_len] {
                body.extend(vwi(v));
            }
            RawEntry {
                key: format!("SKEL{i:010}").into_bytes(),
                body,
            }
        })
        .collect();
    let skel_tags = [TagDef(1, 1, 3), TagDef(6, 2, 0x0c)];
    let skel_records = build_index(&skel_tags, &skel_entries, Vec::new());

    // NCX
    let mut flat = Vec::new();
    flatten(&book.toc(), 0, &mut flat);
    let mut toc: Vec<(String, usize, usize, usize)> = flat // (title, depth, fid, off)
        .into_iter()
        .filter_map(|(title, href, depth)| {
            let (path, frag) = match href.split_once('#') {
                Some((p, f)) => (p.to_string(), f.to_string()),
                None => (href.clone(), String::new()),
            };
            let (fid, off) = pos
                .get(&(path.clone(), frag))
                .or_else(|| pos.get(&(path, String::new())))
                .copied()?;
            let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
            (!title.is_empty())
                .then(|| (title.chars().take(200).collect::<String>(), depth, fid, off))
        })
        .collect();
    if toc.is_empty() {
        for (fid, (path, _)) in parts.iter().enumerate() {
            let title = doc_title(&book.archive.get_string(path).unwrap_or_default(), fid + 1);
            toc.push((title, 0, fid, 0));
        }
    }
    let ncx_records = ncx_index(
        &toc,
        &places.iter().map(|p| p.insert_pos).collect::<Vec<_>>(),
        flow0_len,
    );

    // --- records ----------------------------------------------------------------------------------
    let text_records: Vec<Vec<u8>> = split_text(&stream)
        .into_iter()
        .map(palmdoc::compress)
        .collect();
    if text_records.len() > 0xfff0 {
        bail!("the book has too many text records for the AZW3 format");
    }
    let n_text = text_records.len();
    let pad_rec = 1 + n_text;
    let frag_start = pad_rec + 1;
    let skel_start = frag_start + frag_records.len();
    let ncx_start = skel_start + skel_records.len();
    let res_start = ncx_start + ncx_records.len();
    let n_res = cx.resources.len();
    let fdst_rec = res_start + n_res;
    let flis_rec = fdst_rec + 1;
    let fcis_rec = fdst_rec + 2;

    let mut fdst = b"FDST".to_vec();
    fdst.extend_from_slice(&12u32.to_be_bytes());
    fdst.extend_from_slice(&(flow_bounds.len() as u32).to_be_bytes());
    for (s, e) in &flow_bounds {
        fdst.extend_from_slice(&s.to_be_bytes());
        fdst.extend_from_slice(&e.to_be_bytes());
    }

    let mut exth = Vec::new();
    let mut n_exth = 0u32;
    for a in &md.authors {
        exth_record(&mut exth, &mut n_exth, 100, a.as_bytes());
    }
    if !md.publisher.is_empty() {
        exth_record(&mut exth, &mut n_exth, 101, md.publisher.as_bytes());
    }
    let descr = crate::markup::plain_text(&md.description);
    if !descr.trim().is_empty() {
        exth_record(&mut exth, &mut n_exth, 103, descr.trim().as_bytes());
    }
    if let Some(isbn) = md.identifier("isbn") {
        exth_record(&mut exth, &mut n_exth, 104, isbn.as_bytes());
    }
    for t in &md.tags {
        exth_record(&mut exth, &mut n_exth, 105, t.as_bytes());
    }
    if !md.published.is_empty() {
        exth_record(&mut exth, &mut n_exth, 106, md.published.as_bytes());
    }
    exth_record(&mut exth, &mut n_exth, 108, b"Bookx");
    exth_record(
        &mut exth,
        &mut n_exth,
        113,
        uuid::Uuid::new_v4().to_string().as_bytes(),
    );
    exth_record(&mut exth, &mut n_exth, 501, b"EBOK");
    exth_record(&mut exth, &mut n_exth, 503, md.title.as_bytes());
    if !md.language.is_empty() {
        exth_record(&mut exth, &mut n_exth, 524, md.language.as_bytes());
    }
    if let Some(c) = cover {
        exth_record(&mut exth, &mut n_exth, 201, &((c - 1) as u32).to_be_bytes());
        exth_record(&mut exth, &mut n_exth, 203, &0u32.to_be_bytes());
    }
    if let Some(t) = thumb {
        exth_record(&mut exth, &mut n_exth, 202, &((t - 1) as u32).to_be_bytes());
    }
    if cover.is_some() {
        exth_record(
            &mut exth,
            &mut n_exth,
            129,
            format!("kindle:embed:{}", base32(1, 4)).as_bytes(),
        );
    }
    exth_record(&mut exth, &mut n_exth, 125, &(n_res as u32).to_be_bytes());

    let mut r0 = vec![0u8; 16 + KF8_HEADER_LEN];
    r0[0..2].copy_from_slice(&2u16.to_be_bytes());
    put_u32(&mut r0, 4, stream.len() as u32);
    r0[8..10].copy_from_slice(&(n_text as u16).to_be_bytes());
    r0[10..12].copy_from_slice(&(TEXT_RECORD as u16).to_be_bytes());
    r0[16..20].copy_from_slice(b"MOBI");
    put_u32(&mut r0, 20, KF8_HEADER_LEN as u32);
    put_u32(&mut r0, 24, 2);
    put_u32(&mut r0, 28, 65001);
    put_u32(&mut r0, 32, uuid::Uuid::new_v4().as_u128() as u32);
    put_u32(&mut r0, 36, 8);
    for off in (40..=76).step_by(4) {
        put_u32(&mut r0, off, NO_VALUE);
    }
    put_u32(&mut r0, 80, (pad_rec + 1) as u32);
    put_u32(&mut r0, 92, locale_for(&md.language));
    put_u32(&mut r0, 104, 8);
    put_u32(
        &mut r0,
        108,
        if n_res > 0 {
            res_start as u32
        } else {
            NO_VALUE
        },
    );
    put_u32(&mut r0, 128, 0x50);
    put_u32(&mut r0, 164, NO_VALUE);
    put_u32(&mut r0, 168, NO_VALUE);
    put_u32(&mut r0, 192, fdst_rec as u32);
    put_u32(&mut r0, 196, flow_bounds.len() as u32);
    put_u32(&mut r0, 200, fcis_rec as u32);
    put_u32(&mut r0, 204, 1);
    put_u32(&mut r0, 208, flis_rec as u32);
    put_u32(&mut r0, 212, 1);
    put_u32(&mut r0, 224, NO_VALUE);
    put_u32(&mut r0, 232, NO_VALUE);
    put_u32(&mut r0, 236, NO_VALUE);
    put_u32(&mut r0, 240, 0);
    put_u32(&mut r0, 244, ncx_start as u32);
    put_u32(&mut r0, 248, frag_start as u32);
    put_u32(&mut r0, 252, skel_start as u32);
    put_u32(&mut r0, 256, NO_VALUE); // DATP
    put_u32(&mut r0, 260, NO_VALUE); // guide
    put_u32(&mut r0, 264, NO_VALUE);
    put_u32(&mut r0, 272, NO_VALUE);

    let mut exth_block = b"EXTH".to_vec();
    exth_block.extend_from_slice(&((exth.len() + 12) as u32).to_be_bytes());
    exth_block.extend_from_slice(&n_exth.to_be_bytes());
    exth_block.extend_from_slice(&exth);
    pad4(&mut exth_block);
    r0.extend_from_slice(&exth_block);
    let name_offset = r0.len();
    let title = if md.title.is_empty() {
        "Untitled"
    } else {
        &md.title
    };
    r0.extend_from_slice(title.as_bytes());
    r0.extend_from_slice(&[0, 0]);
    pad4(&mut r0);
    r0.extend_from_slice(&[0u8; 8]);
    put_u32(&mut r0, 84, name_offset as u32);
    put_u32(&mut r0, 88, title.len() as u32);

    let flis = {
        let mut f = b"FLIS".to_vec();
        f.extend_from_slice(&8u32.to_be_bytes());
        f.extend_from_slice(&0x41u16.to_be_bytes());
        f.extend_from_slice(&[0; 2]);
        f.extend_from_slice(&0u32.to_be_bytes());
        f.extend_from_slice(&NO_VALUE.to_be_bytes());
        f.extend_from_slice(&1u16.to_be_bytes());
        f.extend_from_slice(&3u16.to_be_bytes());
        f.extend_from_slice(&3u32.to_be_bytes());
        f.extend_from_slice(&1u32.to_be_bytes());
        f.extend_from_slice(&NO_VALUE.to_be_bytes());
        f
    };
    let fcis = {
        let mut f = b"FCIS".to_vec();
        for v in [
            0x14u32,
            0x10,
            2,
            0,
            stream.len() as u32,
            0,
            0x28,
            0,
            0x28,
            8,
        ] {
            f.extend_from_slice(&v.to_be_bytes());
        }
        f.extend_from_slice(&1u16.to_be_bytes());
        f.extend_from_slice(&1u16.to_be_bytes());
        f.extend_from_slice(&0u32.to_be_bytes());
        f
    };

    let mut records: Vec<Vec<u8>> = vec![r0];
    records.extend(text_records);
    records.push(vec![0, 0]);
    records.extend(frag_records);
    records.extend(skel_records);
    records.extend(ncx_records);
    records.extend(cx.resources.into_iter().map(|(d, _)| d));
    records.push(fdst);
    records.push(flis);
    records.push(fcis);
    records.push(vec![0xe9, 0x8e, 0x0d, 0x0a]);
    if records.len() > 0xffff {
        bail!("the book has too many records for the AZW3 format");
    }
    let _ = extension;
    let _ = remove_attr;
    Ok(assemble_pdb(title, &records))
}

/// NCX index records for entries `(title, depth, fragment, offset in fragment)` given in document
/// order. Entries are written level by level, with parent/child links in the new numbering.
fn ncx_index(
    toc: &[(String, usize, usize, usize)],
    insert_pos: &[usize],
    flow0_len: usize,
) -> Vec<Vec<u8>> {
    let n = toc.len();
    let virt = |i: usize| insert_pos[toc[i].2.min(insert_pos.len() - 1)] + toc[i].3;
    // relationships in document order
    let mut parent: Vec<Option<usize>> = vec![None; n];
    let mut stack: Vec<usize> = Vec::new();
    for i in 0..n {
        while stack.last().is_some_and(|p| toc[*p].1 >= toc[i].1) {
            stack.pop();
        }
        parent[i] = stack.last().copied();
        stack.push(i);
    }
    let length = |i: usize| -> usize {
        let end = (i + 1..n)
            .find(|j| toc[*j].1 <= toc[i].1)
            .map(virt)
            .unwrap_or(flow0_len);
        end.saturating_sub(virt(i))
    };
    // level order (stable)
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|i| toc[*i].1);
    let mut new_ix = vec![0usize; n];
    for (new, old) in order.iter().enumerate() {
        new_ix[*old] = new;
    }
    let mut cncx = Cncx::default();
    let labels: Vec<usize> = order.iter().map(|o| cncx.add(&toc[*o].0)).collect();
    let digits = format!("{:X}", n.saturating_sub(1)).len().max(2);
    let tags = [
        TagDef(1, 1, 1),
        TagDef(2, 1, 2),
        TagDef(3, 1, 4),
        TagDef(4, 1, 8),
        TagDef(21, 1, 0x10),
        TagDef(22, 1, 0x20),
        TagDef(23, 1, 0x40),
        TagDef(6, 2, 0x80),
    ];
    let entries: Vec<RawEntry> = order
        .iter()
        .enumerate()
        .map(|(new, &old)| {
            let children: Vec<usize> = (0..n)
                .filter(|c| parent[*c] == Some(old))
                .map(|c| new_ix[c])
                .collect();
            let mut control = 0x8f_u8;
            let mut values: Vec<u8> = Vec::new();
            values.extend(vwi(virt(old)));
            values.extend(vwi(length(old)));
            values.extend(vwi(labels[new]));
            values.extend(vwi(toc[old].1));
            if let Some(p) = parent[old] {
                control |= 0x10;
                values.extend(vwi(new_ix[p]));
            }
            if let (Some(first), Some(last)) = (children.iter().min(), children.iter().max()) {
                control |= 0x60;
                values.extend(vwi(*first));
                values.extend(vwi(*last));
            }
            values.extend(vwi(toc[old].2));
            values.extend(vwi(toc[old].3));
            let mut body = vec![control];
            body.extend(values);
            RawEntry {
                key: format!("{new:0digits$X}").into_bytes(),
                body,
            }
        })
        .collect();
    build_index(&tags, &entries, cncx.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_digits_and_padding() {
        assert_eq!(base32(0, 1), "0");
        assert_eq!(base32(0, 4), "0000");
        assert_eq!(base32(9, 1), "9");
        assert_eq!(base32(10, 1), "A");
        assert_eq!(base32(31, 1), "V");
        assert_eq!(base32(32, 1), "10");
        assert_eq!(base32(1023, 2), "VV");
        assert_eq!(base32(52, 10), "000000001K");
    }

    #[test]
    fn image_sniffing() {
        assert_eq!(sniff_mime(&[0xff, 0xd8, 0xff, 0]), Some("image/jpeg"));
        assert_eq!(sniff_mime(&[0x89, b'P', b'N', b'G']), Some("image/png"));
        assert_eq!(sniff_mime(b"GIF89a"), Some("image/gif"));
        assert_eq!(sniff_mime(b"<svg"), None);
        assert_eq!(sniff_mime(b""), None);
    }

    #[test]
    fn link_template_has_the_fixed_width_of_a_real_link() {
        let real = format!(
            "kindle:pos:fid:{}:off:{}",
            base32(31, 4),
            base32(99_999, 10)
        );
        assert_eq!(real.len(), LINK_TEMPLATE.len());
    }

    #[test]
    fn ncx_index_orders_by_level_and_links_families() {
        let toc = vec![
            ("A".to_string(), 0, 0, 0),
            ("A1".to_string(), 1, 0, 10),
            ("A2".to_string(), 1, 1, 0),
            ("B".to_string(), 0, 2, 0),
        ];
        let recs = ncx_index(&toc, &[100, 200, 300], 400);
        assert_eq!(recs.len(), 3, "header, data, cncx");
        assert!(recs[0].starts_with(b"INDX") && recs[1].starts_with(b"INDX"));
        // CNCX holds the four labels in the new (level) order
        let cncx = &recs[2];
        assert_eq!(&cncx[1..2], b"A");
        assert!(cncx.windows(2).any(|w| w == b"A2"));
    }
}
