//! RTF writer.
//!
//! The document has a font table, a colour table (black, link blue), a stylesheet with
//! `heading 1`..`heading 6` carrying outline levels, an `\info` group and one `\pard` per block.
//! Text is `\'hh` for Windows-1252 characters and `\uN?` (with `\uc1`) for the rest, so the file is
//! pure ASCII. Images are `\pict` groups with `\pngblip` or `\jpegblip` and hex data (GIF is
//! converted to PNG), links are `HYPERLINK` fields (internal ones use `\l` with bookmarks), list
//! items are paragraphs with a bullet or number followed by a tab, and tables use `\trowd`/`\cell`.
//! Sections are separated by `\pagebb` on their first paragraph.

use std::collections::HashMap;
use std::fmt::Write as _;

use anyhow::Result;
use bookx_core::book::OpenBook;
use bookx_core::contracts::FormatWriter;

use crate::charset::cp1252_byte;
use crate::images::{Kind, image_size, kind_of, to_png};
use crate::model::{Align, Block, Fmt, Inline, Link, Model, split_lines};

#[derive(Debug, Clone, Copy, Default)]
pub struct RtfWriter;

/// Usable text width in twips (US Letter, one inch margins).
const TEXT_WIDTH: u32 = 9360;
const HEADING_SIZE: [u32; 6] = [36, 32, 28, 26, 24, 22];

/// Appends `s` to `out` as RTF text (all output is ASCII).
pub fn escape_into(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\n' | '\r' => out.push(' '),
            '\t' => out.push_str("\\tab "),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {}
            c if c.is_ascii() => out.push(c),
            c => match cp1252_byte(c) {
                Some(b) => {
                    let _ = write!(out, "\\'{b:02x}");
                }
                None => {
                    let mut units = [0u16; 2];
                    for u in c.encode_utf16(&mut units) {
                        let _ = write!(out, "\\u{}?", *u as i16);
                    }
                }
            },
        }
    }
}

fn esc(s: &str) -> String {
    let mut o = String::new();
    escape_into(s, &mut o);
    o
}

struct Rtf<'a> {
    book: &'a OpenBook,
    model: &'a Model,
    out: String,
    bookmarks: HashMap<String, String>,
    page_break: bool,
}

fn align_word(a: Align) -> &'static str {
    match a {
        Align::Left => "\\ql",
        Align::Center => "\\qc",
        Align::Right => "\\qr",
    }
}

fn char_props(f: &Fmt) -> String {
    let mut s = String::new();
    if f.bold {
        s.push_str("\\b");
    }
    if f.italic {
        s.push_str("\\i");
    }
    if f.underline {
        s.push_str("\\ul");
    }
    if f.strike {
        s.push_str("\\strike");
    }
    if f.sup {
        s.push_str("\\super");
    }
    if f.sub {
        s.push_str("\\sub");
    }
    if f.mono {
        s.push_str("\\f1");
    }
    if f.small {
        s.push_str("\\fs18");
    }
    s
}

impl<'a> Rtf<'a> {
    fn bookmark_name(&mut self, key: &str) -> String {
        let n = self.bookmarks.len() + 1;
        self.bookmarks
            .entry(key.to_string())
            .or_insert_with(|| format!("bx{n}"))
            .clone()
    }

    fn pict(&mut self, path: &str, max_width: u32) {
        let Some(data) = self.book.archive.get(path) else {
            return;
        };
        let (bytes, word): (Vec<u8>, &str) = match kind_of(data) {
            Some(Kind::Png) => (data.to_vec(), "\\pngblip"),
            Some(Kind::Jpeg) => (data.to_vec(), "\\jpegblip"),
            _ => match to_png(data) {
                Some((p, _, _)) => (p, "\\pngblip"),
                None => return,
            },
        };
        let Some((w, h)) = image_size(&bytes) else {
            return;
        };
        let mut gw = w * 15;
        let mut gh = h * 15;
        if gw > max_width {
            gh = (gh as u64 * max_width as u64 / gw as u64) as u32;
            gw = max_width;
        }
        let _ = writeln!(
            self.out,
            "{{\\pict{word}\\picw{w}\\pich{h}\\picwgoal{gw}\\pichgoal{gh}"
        );
        for line in bytes.chunks(48) {
            for b in line {
                let _ = write!(self.out, "{b:02x}");
            }
            self.out.push('\n');
        }
        self.out.push_str("}\n");
    }

    fn text_run(&mut self, text: &str, fmt: &Fmt) {
        let props = char_props(fmt);
        if props.is_empty() {
            escape_into(text, &mut self.out);
        } else {
            let _ = write!(self.out, "{{{props} ");
            escape_into(text, &mut self.out);
            self.out.push('}');
        }
    }

    fn inlines(&mut self, inl: &[Inline], max_width: u32) {
        let mut i = 0;
        while i < inl.len() {
            match &inl[i] {
                Inline::Text { link, .. } if link.is_some() => {
                    let mut j = i;
                    while j < inl.len() {
                        match &inl[j] {
                            Inline::Text { link: l, .. } if l == link => j += 1,
                            _ => break,
                        }
                    }
                    let target = match link {
                        Some(Link::External(u)) => Some(format!("HYPERLINK \"{}\"", field_text(u))),
                        Some(Link::Internal(k)) if self.model.linked.contains(k) => {
                            Some(format!("HYPERLINK \\\\l \"{}\"", self.bookmark_name(k)))
                        }
                        _ => None,
                    };
                    if let Some(instr) = target {
                        let _ = write!(
                            self.out,
                            "{{\\field{{\\*\\fldinst{{{instr}}}}}{{\\fldrslt{{"
                        );
                        for r in &inl[i..j] {
                            if let Inline::Text { text, fmt, .. } = r {
                                let mut f = *fmt;
                                f.underline = true;
                                let _ = write!(self.out, "{{\\cf2{} ", char_props(&f));
                                escape_into(text, &mut self.out);
                                self.out.push('}');
                            }
                        }
                        self.out.push_str("}}}");
                    } else {
                        for r in &inl[i..j] {
                            if let Inline::Text { text, fmt, .. } = r {
                                self.text_run(text, fmt);
                            }
                        }
                    }
                    i = j;
                    continue;
                }
                Inline::Text { text, fmt, .. } => self.text_run(text, fmt),
                Inline::Image { path, .. } => self.pict(path, max_width),
                Inline::Break => self.out.push_str("\\line "),
                Inline::Anchor(k) => {
                    if self.model.linked.contains(k) {
                        let n = self.bookmark_name(k);
                        let _ = write!(self.out, "{{\\*\\bkmkstart {n}}}{{\\*\\bkmkend {n}}}");
                    }
                }
            }
            i += 1;
        }
    }

    fn pard(&mut self, extra: &str) {
        self.out.push_str("\\pard\\plain\\f0\\fs24");
        if self.page_break {
            self.out.push_str("\\pagebb");
            self.page_break = false;
        }
        self.out.push_str(extra);
        self.out.push(' ');
    }

    fn blocks(&mut self, blocks: &[Block], indent: u32, max_width: u32) {
        for b in blocks {
            match b {
                Block::Heading {
                    level,
                    align,
                    inlines,
                } => {
                    let l = (*level).clamp(1, 6) as usize;
                    let size = HEADING_SIZE[l - 1];
                    let extra = format!(
                        "\\s{l}{}\\sb240\\sa120\\keepn\\outlinelevel{}\\li{indent}\\b\\fs{size}",
                        align_word(*align),
                        l - 1
                    );
                    self.pard(&extra);
                    self.inlines(inlines, max_width);
                    self.out.push_str("\\par\n");
                }
                Block::Para { align, inlines } => {
                    let extra = format!("{}\\sa120\\li{indent}", align_word(*align));
                    self.pard(&extra);
                    self.inlines(inlines, max_width);
                    self.out.push_str("\\par\n");
                }
                Block::Pre(inlines) => {
                    let extra = format!("\\ql\\li{indent}\\f1\\fs20");
                    self.pard(&extra);
                    self.inlines(&split_lines(inlines), max_width);
                    self.out.push_str("\\par\n");
                }
                Block::Quote(inner) => self.blocks(inner, indent + 720, max_width),
                Block::Rule => {
                    self.pard(&format!("\\li{indent}\\brdrb\\brdrs\\brdrw10\\brsp20"));
                    self.out.push_str("\\par\n");
                }
                Block::List { ordered, items } => self.list(*ordered, items, indent, max_width),
                Block::Table { rows } => self.table(rows, indent, max_width),
            }
        }
    }

    fn list(&mut self, ordered: bool, items: &[Vec<Block>], indent: u32, max_width: u32) {
        let li = indent + 720;
        for (n, item) in items.iter().enumerate() {
            let marker = if ordered {
                format!("{}.", n + 1)
            } else {
                "\\u8226?".to_string()
            };
            let mut first = true;
            for b in item {
                match b {
                    Block::Para { align, inlines } if first => {
                        self.pard(&format!("{}\\fi-360\\li{li}\\sa60", align_word(*align)));
                        let _ = write!(self.out, "{marker}\\tab ");
                        self.inlines(inlines, max_width.saturating_sub(li));
                        self.out.push_str("\\par\n");
                    }
                    Block::List { ordered, items } => self.list(*ordered, items, li, max_width),
                    other => {
                        if first {
                            self.pard(&format!("\\fi-360\\li{li}\\sa60"));
                            let _ = writeln!(self.out, "{marker}\\tab \\par");
                        }
                        self.blocks(std::slice::from_ref(other), li, max_width);
                    }
                }
                first = false;
            }
        }
    }

    fn table(&mut self, rows: &[Vec<crate::model::Cell>], indent: u32, max_width: u32) {
        let cols = rows
            .iter()
            .map(|r| r.iter().map(|c| c.colspan).sum::<usize>())
            .max()
            .unwrap_or(1)
            .max(1);
        let width = max_width.saturating_sub(indent).max(1440);
        let col_w = (width / cols as u32).max(360);
        if self.page_break {
            self.pard("");
            self.out.push_str("\\par\n");
        }
        for row in rows {
            self.out.push_str("\\trowd\\trgaph108");
            let _ = write!(self.out, "\\trleft{indent}");
            let mut cell_defs = Vec::new();
            let mut used = 0;
            for c in row {
                for k in 0..c.colspan {
                    let merge = match (c.colspan > 1, k) {
                        (false, _) => "",
                        (true, 0) => "\\clmgf",
                        (true, _) => "\\clmrg",
                    };
                    used += 1;
                    cell_defs.push(format!(
                        "{merge}\\clbrdrt\\brdrs\\brdrw10\\clbrdrl\\brdrs\\brdrw10\\clbrdrb\\brdrs\\brdrw10\\clbrdrr\\brdrs\\brdrw10\\cellx{}",
                        indent + col_w * used as u32
                    ));
                }
            }
            while used < cols {
                used += 1;
                cell_defs.push(format!(
                    "\\clbrdrt\\brdrs\\brdrw10\\clbrdrl\\brdrs\\brdrw10\\clbrdrb\\brdrs\\brdrw10\\clbrdrr\\brdrs\\brdrw10\\cellx{}",
                    indent + col_w * used as u32
                ));
            }
            self.out.push_str(&cell_defs.join(""));
            self.out.push('\n');
            let mut written = 0;
            for c in row {
                self.out.push_str("\\pard\\plain\\f0\\fs24\\intbl ");
                let inl = crate::model::flat_inlines(&c.blocks);
                self.inlines(&inl, col_w.saturating_sub(216));
                self.out.push_str("\\cell\n");
                written += 1;
                for _ in 1..c.colspan {
                    self.out.push_str("\\pard\\plain\\intbl \\cell\n");
                    written += 1;
                }
            }
            while written < cols {
                self.out.push_str("\\pard\\plain\\intbl \\cell\n");
                written += 1;
            }
            self.out.push_str("\\row\n");
        }
        self.out.push_str("\\pard\\plain\\par\n");
    }
}

/// Text inside a field instruction: quotes and backslashes escaped.
fn field_text(s: &str) -> String {
    let mut o = String::new();
    for ch in s.chars() {
        match ch {
            '"' => o.push_str("%22"),
            '\\' => o.push_str("\\\\"),
            '{' => o.push_str("\\{"),
            '}' => o.push_str("\\}"),
            c if c.is_ascii() && !c.is_control() => o.push(c),
            c => {
                let mut b = [0u8; 4];
                for x in c.encode_utf8(&mut b).bytes() {
                    let _ = write!(o, "%{x:02X}");
                }
            }
        }
    }
    o
}

/// The document as an RTF string.
pub fn render_rtf(book: &OpenBook, model: &Model) -> String {
    let md = book.metadata();
    let mut head = String::from(
        "{\\rtf1\\ansi\\ansicpg1252\\uc1\\deff0\\deflang1033\n{\\fonttbl{\\f0\\froman\\fcharset0 Times New Roman;}{\\f1\\fmodern\\fcharset0 Courier New;}{\\f2\\fswiss\\fcharset0 Arial;}}\n{\\colortbl;\\red0\\green0\\blue0;\\red0\\green0\\blue255;}\n{\\stylesheet{\\s0\\f0\\fs24 Normal;}",
    );
    for (i, size) in HEADING_SIZE.iter().enumerate() {
        let _ = write!(
            head,
            "{{\\s{}\\sbasedon0\\snext0\\sb240\\sa120\\keepn\\outlinelevel{}\\b\\f0\\fs{size} heading {};}}",
            i + 1,
            i,
            i + 1
        );
    }
    head.push_str("}\n{\\info");
    let _ = write!(head, "{{\\title {}}}", esc(&md.title));
    if !md.authors.is_empty() {
        let _ = write!(head, "{{\\author {}}}", esc(&md.authors.join(" & ")));
    }
    if !md.description.is_empty() {
        let _ = write!(
            head,
            "{{\\subject {}}}",
            esc(&crate::markup::plain_text(&md.description))
        );
    }
    if !md.tags.is_empty() {
        let _ = write!(head, "{{\\keywords {}}}", esc(&md.tags.join(", ")));
    }
    if !md.publisher.is_empty() {
        let _ = write!(head, "{{\\company {}}}", esc(&md.publisher));
    }
    head.push_str("{\\operator Bookx}}\n\\paperw12240\\paperh15840\\margl1440\\margr1440\\margt1440\\margb1440\n");

    let mut r = Rtf {
        book,
        model,
        out: head,
        bookmarks: HashMap::new(),
        page_break: false,
    };
    if let Some(c) = &model.cover {
        if image_size(&c.data).is_some() {
            r.pard("\\qc");
            r.pict(&c.path, TEXT_WIDTH);
            r.out.push_str("\\par\n");
            r.page_break = true;
        }
    }
    for s in &model.sections {
        if !r.out.ends_with('\n') {
            r.out.push('\n');
        }
        if model.linked.contains(&s.path) {
            let n = r.bookmark_name(&s.path);
            let _ = write!(r.out, "{{\\*\\bkmkstart {n}}}{{\\*\\bkmkend {n}}}");
        }
        r.blocks(&s.blocks, 0, TEXT_WIDTH);
        r.page_break = true;
    }
    r.out.push_str("}\n");
    r.out
}

impl FormatWriter for RtfWriter {
    fn id(&self) -> &'static str {
        "rtf"
    }
    fn extension(&self) -> &'static str {
        "rtf"
    }
    fn write(&self, book: &OpenBook) -> Result<Vec<u8>> {
        let model = Model::build(book);
        Ok(render_rtf(book, &model).into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::book_with;

    fn esc_str(s: &str) -> String {
        esc(s)
    }

    #[test]
    fn escape_specials_and_encodings() {
        assert_eq!(esc_str("a\\{b}"), "a\\\\\\{b\\}");
        assert_eq!(esc_str("é“"), "\\'e9\\'93");
        assert_eq!(esc_str("日"), "\\u26085?");
        assert_eq!(esc_str("😀"), "\\u-10179?\\u-8704?");
        assert_eq!(esc_str("a\nb\rc"), "a b c");
        assert_eq!(esc_str("x\ty"), "x\\tab y");
        assert_eq!(esc_str("\u{1}\u{7f}"), "");
        assert_eq!(esc_str(""), "");
        assert!(esc_str("anything é日😀").is_ascii());
    }

    #[test]
    fn field_text_escapes_quotes_and_unicode() {
        assert_eq!(field_text("https://a/b"), "https://a/b");
        assert_eq!(field_text("a\"b"), "a%22b");
        assert_eq!(field_text("a\\b{c}"), "a\\\\b\\{c\\}");
        assert_eq!(field_text("é"), "%C3%A9");
    }

    #[test]
    fn character_properties() {
        let all = Fmt {
            bold: true,
            italic: true,
            underline: true,
            strike: true,
            sup: true,
            sub: true,
            mono: true,
            small: true,
        };
        assert_eq!(char_props(&all), "\\b\\i\\ul\\strike\\super\\sub\\f1\\fs18");
        assert_eq!(char_props(&Fmt::default()), "");
        assert_eq!(align_word(Align::Left), "\\ql");
        assert_eq!(align_word(Align::Center), "\\qc");
        assert_eq!(align_word(Align::Right), "\\qr");
    }

    #[test]
    fn writer_identity_and_render() {
        assert_eq!(RtfWriter.id(), "rtf");
        assert_eq!(RtfWriter.extension(), "rtf");
        let book = book_with(&["<h1>Hi</h1><p>text <a href=\"https://x.org\">l</a></p>"]);
        let model = Model::build(&book);
        let s = render_rtf(&book, &model);
        assert!(s.starts_with("{\\rtf1"));
        assert!(s.contains("\\s1") && s.contains("HYPERLINK \"https://x.org\""));
        assert!(s.contains("{\\title Unit Book}"));
        assert_eq!(RtfWriter.write(&book).unwrap(), s.into_bytes());
    }
}
