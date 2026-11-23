//! A tiny PDF writer for tests (and the UI test fixtures): pages with positioned text, links,
//! an outline, page labels, document info and the RC4 40-bit security handler.
//!
//! Not a general purpose writer: fonts are the standard Helvetica/Times (not embedded), streams
//! are not compressed, and everything is built in memory.

/// Where a link or an outline entry goes.
#[derive(Debug, Clone)]
pub enum Target {
    /// A page (zero based).
    Page(usize),
    /// An external address.
    Uri(String),
    /// A named destination (`/Dests` dictionary of the catalog), pointing at a page.
    Named(String),
}

#[derive(Debug, Clone)]
pub enum Item {
    /// `size` points, baseline at `(x, y)` measured from the bottom left (PDF space).
    Text {
        x: f32,
        y: f32,
        size: f32,
        text: String,
        font: &'static str,
    },
    Link {
        rect: [f32; 4],
        to: Target,
    },
    /// A filled rectangle, for image-like content.
    Rect {
        rect: [f32; 4],
        gray: f32,
    },
}

#[derive(Debug, Clone)]
pub struct Page {
    pub width: f32,
    pub height: f32,
    pub items: Vec<Item>,
}

impl Page {
    pub fn new(width: f32, height: f32) -> Page {
        Page {
            width,
            height,
            items: Vec::new(),
        }
    }

    pub fn text(mut self, x: f32, y: f32, size: f32, text: &str) -> Page {
        self.items.push(Item::Text {
            x,
            y,
            size,
            text: text.into(),
            font: "Helvetica",
        });
        self
    }

    pub fn text_in(mut self, font: &'static str, x: f32, y: f32, size: f32, text: &str) -> Page {
        self.items.push(Item::Text {
            x,
            y,
            size,
            text: text.into(),
            font,
        });
        self
    }

    pub fn link(mut self, rect: [f32; 4], to: Target) -> Page {
        self.items.push(Item::Link { rect, to });
        self
    }

    pub fn rect(mut self, rect: [f32; 4], gray: f32) -> Page {
        self.items.push(Item::Rect { rect, gray });
        self
    }
}

#[derive(Debug, Clone)]
pub struct OutlineEntry {
    pub title: String,
    pub to: Target,
    pub children: Vec<OutlineEntry>,
}

impl OutlineEntry {
    pub fn new(title: &str, page: usize) -> OutlineEntry {
        OutlineEntry {
            title: title.into(),
            to: Target::Page(page),
            children: Vec::new(),
        }
    }
    pub fn with(mut self, child: OutlineEntry) -> OutlineEntry {
        self.children.push(child);
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct TestPdf {
    pub pages: Vec<Page>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub outline: Vec<OutlineEntry>,
    /// `(first page, style, prefix, start)`; style is `D`, `r`, `R`, `a`, `A` or empty.
    pub labels: Vec<(usize, String, String, u32)>,
    pub named: Vec<(String, usize)>,
    /// User password of the RC4 40-bit handler.
    pub password: Option<String>,
}

fn esc(s: &str) -> Vec<u8> {
    let mut v = vec![b'('];
    for b in s.bytes() {
        if matches!(b, b'(' | b')' | b'\\') {
            v.push(b'\\');
        }
        v.push(b);
    }
    v.push(b')');
    v
}

/// Latin-1 / UTF-16BE text string literal, as PDF stores titles.
fn text_string(s: &str) -> Vec<u8> {
    if s.is_ascii() {
        return esc(s);
    }
    let mut v = vec![b'<', b'F', b'E', b'F', b'F'];
    for u in s.encode_utf16() {
        v.extend(format!("{u:04X}").bytes());
    }
    v.push(b'>');
    v
}

impl TestPdf {
    pub fn new() -> TestPdf {
        TestPdf::default()
    }

    /// A document of `n` letter pages, page `i` saying "Page i+1" in 24 point and holding the lines.
    pub fn simple(lines_per_page: &[&[&str]]) -> TestPdf {
        let mut d = TestPdf::new();
        for (i, lines) in lines_per_page.iter().enumerate() {
            let mut p = Page::new(612.0, 792.0).text(72.0, 740.0, 24.0, &format!("Page {}", i + 1));
            for (j, l) in lines.iter().enumerate() {
                p = p.text(72.0, 700.0 - 16.0 * j as f32, 12.0, l);
            }
            d.pages.push(p);
        }
        d
    }

    pub fn page(mut self, p: Page) -> TestPdf {
        self.pages.push(p);
        self
    }

    pub fn title(mut self, t: &str) -> TestPdf {
        self.title = Some(t.into());
        self
    }

    pub fn author(mut self, t: &str) -> TestPdf {
        self.author = Some(t.into());
        self
    }

    pub fn password(mut self, p: &str) -> TestPdf {
        self.password = Some(p.into());
        self
    }

    pub fn outline(mut self, e: OutlineEntry) -> TestPdf {
        self.outline.push(e);
        self
    }

    pub fn label(mut self, first: usize, style: &str, prefix: &str, start: u32) -> TestPdf {
        self.labels
            .push((first, style.into(), prefix.into(), start));
        self
    }

    pub fn named(mut self, name: &str, page: usize) -> TestPdf {
        self.named.push((name.into(), page));
        self
    }

    /// The file.
    pub fn build(&self) -> Vec<u8> {
        let mut w = Writer::default();
        // object numbers: 1 catalog, 2 pages, 3 info, 4.. fonts, then pages and contents, outline
        let fonts = ["Helvetica", "Times-Roman", "Courier"];
        let font_id = |name: &str| 4 + fonts.iter().position(|f| *f == name).unwrap_or(0);
        let first_page = 4 + fonts.len();
        let page_id = |i: usize| first_page + i * 2;
        let content_id = |i: usize| first_page + i * 2 + 1;
        let mut next = first_page + self.pages.len() * 2;
        let outline_root = if self.outline.is_empty() {
            None
        } else {
            let id = next;
            next += 1;
            Some(id)
        };
        // flatten outline entries to object ids
        struct Flat<'a> {
            id: usize,
            parent: usize,
            prev: Option<usize>,
            next: Option<usize>,
            first: Option<usize>,
            last: Option<usize>,
            entry: &'a OutlineEntry,
            count: usize,
        }
        let mut flat: Vec<Flat> = Vec::new();
        fn add<'a>(
            list: &'a [OutlineEntry],
            parent: usize,
            next: &mut usize,
            flat: &mut Vec<Flat<'a>>,
        ) -> Vec<usize> {
            let ids: Vec<usize> = list
                .iter()
                .map(|_| {
                    let i = *next;
                    *next += 1;
                    i
                })
                .collect();
            for (k, e) in list.iter().enumerate() {
                let idx = flat.len();
                flat.push(Flat {
                    id: ids[k],
                    parent,
                    prev: if k > 0 { Some(ids[k - 1]) } else { None },
                    next: ids.get(k + 1).copied(),
                    first: None,
                    last: None,
                    entry: e,
                    count: e.children.len(),
                });
                let kids = add(&e.children, ids[k], next, flat);
                flat[idx].first = kids.first().copied();
                flat[idx].last = kids.last().copied();
            }
            ids
        }
        if let Some(root) = outline_root {
            add(&self.outline, root, &mut next, &mut flat);
        }
        let first_top = flat.first().map(|f| f.id);
        let last_top = flat.iter().rfind(|f| f.parent == outline_root.unwrap_or(0));
        let dest = |t: &Target| -> String {
            match t {
                Target::Page(p) => format!(
                    "/Dest [{} 0 R /Fit]",
                    page_id((*p).min(self.pages.len().saturating_sub(1)))
                ),
                Target::Named(n) => format!("/Dest ({n})"),
                Target::Uri(u) => {
                    format!("/A << /S /URI /URI {} >>", String::from_utf8_lossy(&esc(u)))
                }
            }
        };

        // catalog
        let mut cat = String::from("<< /Type /Catalog /Pages 2 0 R");
        if let Some(r) = outline_root {
            cat.push_str(&format!(" /Outlines {r} 0 R"));
        }
        if !self.labels.is_empty() {
            cat.push_str(" /PageLabels << /Nums [");
            for (first, style, prefix, start) in &self.labels {
                cat.push_str(&format!(" {first} <<"));
                if !style.is_empty() {
                    cat.push_str(&format!(" /S /{style}"));
                }
                if !prefix.is_empty() {
                    cat.push_str(&format!(" /P {}", String::from_utf8_lossy(&esc(prefix))));
                }
                if *start != 1 {
                    cat.push_str(&format!(" /St {start}"));
                }
                cat.push_str(" >>");
            }
            cat.push_str(" ] >>");
        }
        if !self.named.is_empty() {
            cat.push_str(" /Dests <<");
            for (n, p) in &self.named {
                cat.push_str(&format!(" /{n} [{} 0 R /Fit]", page_id(*p)));
            }
            cat.push_str(" >>");
        }
        cat.push_str(" >>");
        w.obj(1, cat.into_bytes(), false);
        let kids: Vec<String> = (0..self.pages.len())
            .map(|i| format!("{} 0 R", page_id(i)))
            .collect();
        w.obj(
            2,
            format!(
                "<< /Type /Pages /Kids [{}] /Count {} >>",
                kids.join(" "),
                self.pages.len()
            )
            .into_bytes(),
            false,
        );
        let mut info = String::from("<<");
        let mut info_bytes: Vec<u8> = Vec::new();
        if let Some(t) = &self.title {
            info.push_str(" /Title ");
            info_bytes.extend(info.bytes());
            info.clear();
            info_bytes.extend(text_string(t));
        }
        if let Some(a) = &self.author {
            info_bytes.extend(info.bytes());
            info.clear();
            info_bytes.extend(b" /Author ");
            info_bytes.extend(text_string(a));
        }
        info_bytes.extend(info.bytes());
        info_bytes.extend(b" >>");
        w.obj(3, info_bytes, true);
        for (k, f) in fonts.iter().enumerate() {
            w.obj(
                4 + k,
                format!(
                    "<< /Type /Font /Subtype /Type1 /BaseFont /{f} /Encoding /WinAnsiEncoding >>"
                )
                .into_bytes(),
                false,
            );
        }
        for (i, p) in self.pages.iter().enumerate() {
            let mut annots = Vec::new();
            let mut content = String::new();
            for it in &p.items {
                match it {
                    Item::Text {
                        x,
                        y,
                        size,
                        text,
                        font,
                    } => {
                        let fid = font_id(font);
                        content.push_str(&format!(
                            "BT /F{fid} {size} Tf {x} {y} Td {} Tj ET\n",
                            String::from_utf8_lossy(&esc(text))
                        ));
                    }
                    Item::Rect { rect, gray } => {
                        content.push_str(&format!(
                            "{gray} g {} {} {} {} re f 0 g\n",
                            rect[0],
                            rect[1],
                            rect[2] - rect[0],
                            rect[3] - rect[1]
                        ));
                    }
                    Item::Link { rect, to } => {
                        annots.push(format!(
                            "<< /Type /Annot /Subtype /Link /Rect [{} {} {} {}] /Border [0 0 0] {} >>",
                            rect[0], rect[1], rect[2], rect[3], dest(to)
                        ));
                    }
                }
            }
            let res: String = fonts
                .iter()
                .enumerate()
                .map(|(k, _)| format!("/F{} {} 0 R", 4 + k, 4 + k))
                .collect::<Vec<_>>()
                .join(" ");
            let mut dict = format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << {res} >> >> /Contents {} 0 R",
                p.width,
                p.height,
                content_id(i)
            );
            if !annots.is_empty() {
                dict.push_str(&format!(" /Annots [{}]", annots.join(" ")));
            }
            dict.push_str(" >>");
            w.obj(page_id(i), dict.into_bytes(), false);
            w.stream(content_id(i), content.into_bytes());
        }
        if let (Some(root), Some(first)) = (outline_root, first_top) {
            let last = last_top.map(|l| l.id).unwrap_or(first);
            let count = flat.len();
            w.obj(
                root,
                format!("<< /Type /Outlines /First {first} 0 R /Last {last} 0 R /Count {count} >>")
                    .into_bytes(),
                false,
            );
            for f in &flat {
                let mut d = Vec::new();
                d.extend(b"<< /Title ");
                d.extend(text_string(&f.entry.title));
                d.extend(format!(" /Parent {} 0 R", f.parent).bytes());
                if let Some(p) = f.prev {
                    d.extend(format!(" /Prev {p} 0 R").bytes());
                }
                if let Some(n) = f.next {
                    d.extend(format!(" /Next {n} 0 R").bytes());
                }
                if let (Some(a), Some(b)) = (f.first, f.last) {
                    d.extend(format!(" /First {a} 0 R /Last {b} 0 R /Count {}", f.count).bytes());
                }
                d.extend(format!(" {} >>", dest(&f.entry.to)).bytes());
                w.obj(f.id, d, true);
            }
        }
        let max_id = next - 1;
        w.finish(max_id, self.password.as_deref())
    }
}

#[derive(Default)]
struct Writer {
    /// `(id, body, is_dict_with_strings, is_stream)`
    objs: Vec<(usize, Vec<u8>, bool, bool)>,
}

impl Writer {
    fn obj(&mut self, id: usize, body: Vec<u8>, strings: bool) {
        self.objs.push((id, body, strings, false));
    }
    fn stream(&mut self, id: usize, data: Vec<u8>) {
        self.objs.push((id, data, false, true));
    }

    fn finish(mut self, max_id: usize, password: Option<&str>) -> Vec<u8> {
        const PAD: [u8; 32] = [
            0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA,
            0x01, 0x08, 0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE,
            0x64, 0x53, 0x69, 0x7A,
        ];
        let file_id = *b"bookx-test-file!";
        let mut key: Option<Vec<u8>> = None;
        let mut encrypt_obj = String::new();
        if let Some(pw) = password {
            let o = [0x42u8; 32];
            let p: i32 = -4;
            let mut padded = pw.as_bytes().to_vec();
            padded.extend(PAD);
            padded.truncate(32);
            let mut input = padded;
            input.extend(o);
            input.extend(p.to_le_bytes());
            input.extend(file_id);
            let k = md5(&input)[..5].to_vec();
            let u = rc4(&k, &PAD);
            encrypt_obj = format!(
                "<< /Filter /Standard /V 1 /R 2 /O <{}> /U <{}> /P {p} >>",
                hex(&o),
                hex(&u)
            );
            key = Some(k);
        }
        let enc_id = max_id + 1;
        let mut out: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = vec![0usize; enc_id + 1];
        let mut all = std::mem::take(&mut self.objs);
        all.sort_by_key(|o| o.0);
        for (id, body, strings, is_stream) in all {
            offsets[id] = out.len();
            out.extend(format!("{id} 0 obj\n").bytes());
            if is_stream {
                let data = match &key {
                    Some(k) => rc4(&object_key(k, id), &body),
                    None => body,
                };
                out.extend(format!("<< /Length {} >>\nstream\n", data.len()).bytes());
                out.extend(data);
                out.extend(b"\nendstream");
            } else if strings && key.is_some() {
                out.extend(encrypt_strings(&body, key.as_ref().unwrap(), id));
            } else {
                out.extend(body);
            }
            out.extend(b"\nendobj\n");
        }
        if password.is_some() {
            offsets[enc_id] = out.len();
            out.extend(format!("{enc_id} 0 obj\n{encrypt_obj}\nendobj\n").bytes());
        }
        let xref = out.len();
        let count = enc_id + 1;
        out.extend(format!("xref\n0 {count}\n0000000000 65535 f \n").bytes());
        for off in offsets.iter().take(count).skip(1) {
            out.extend(format!("{off:010} 00000 n \n").bytes());
        }
        let encrypt = if password.is_some() {
            format!(" /Encrypt {enc_id} 0 R")
        } else {
            String::new()
        };
        out.extend(
            format!(
                "trailer\n<< /Size {count} /Root 1 0 R /Info 3 0 R{encrypt} /ID [<{h}> <{h}>] >>\nstartxref\n{xref}\n%%EOF\n",
                h = hex(&file_id)
            )
            .bytes(),
        );
        out
    }
}

/// Encrypts the literal strings `( ... )` and `<FEFF ...>` of a dictionary body.
fn encrypt_strings(body: &[u8], key: &[u8], id: usize) -> Vec<u8> {
    let k = object_key(key, id);
    let mut out = Vec::new();
    let mut i = 0;
    while i < body.len() {
        match body[i] {
            b'(' => {
                let mut s = Vec::new();
                i += 1;
                while i < body.len() && body[i] != b')' {
                    if body[i] == b'\\' {
                        i += 1;
                    }
                    s.push(body[i]);
                    i += 1;
                }
                i += 1;
                out.push(b'<');
                out.extend(hex(&rc4(&k, &s)).bytes());
                out.push(b'>');
            }
            b'<' if body.get(i + 1) == Some(&b'F') => {
                let end = body[i..].iter().position(|b| *b == b'>').unwrap() + i;
                let h = std::str::from_utf8(&body[i + 1..end]).unwrap();
                let raw: Vec<u8> = (0..h.len() / 2)
                    .map(|j| u8::from_str_radix(&h[2 * j..2 * j + 2], 16).unwrap())
                    .collect();
                out.push(b'<');
                out.extend(hex(&rc4(&k, &raw)).bytes());
                out.push(b'>');
                i = end + 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

fn object_key(key: &[u8], id: usize) -> Vec<u8> {
    let mut v = key.to_vec();
    v.extend(&(id as u32).to_le_bytes()[..3]);
    v.extend([0, 0]);
    let n = (key.len() + 5).min(16);
    md5(&v)[..n].to_vec()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect()
}

fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s: Vec<u8> = (0..=255).collect();
    let mut j = 0u8;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, j as usize);
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|b| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[i as usize]);
            s.swap(i as usize, j as usize);
            b ^ s[s[i as usize].wrapping_add(s[j as usize]) as usize]
        })
        .collect()
}

fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64)
        .map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32)
        .collect();
    let (mut a0, mut b0, mut c0, mut d0) =
        (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend(((data.len() as u64) * 8).to_le_bytes());
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk
            .chunks(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (mut f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    out[..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..].copy_from_slice(&d0.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_matches_the_known_digest() {
        assert_eq!(hex(&md5(b"abc")), "900150983CD24FB0D6963F7D28E17F72");
    }

    #[test]
    fn rc4_matches_the_known_vector() {
        assert_eq!(hex(&rc4(b"Key", b"Plaintext")), "BBF316E8D940AF0AD3");
    }

    #[test]
    fn the_file_has_the_pdf_framing() {
        let b = TestPdf::simple(&[&["a"]]).build();
        assert!(b.starts_with(b"%PDF-1.4"));
        assert!(b.ends_with(b"%%EOF\n"));
    }
}
