//! A small structural model of a book (sections, blocks, inline runs) shared by the writers that
//! render to a non-HTML target (DOCX, RTF, PML, RB, SNB).
//!
//! [`Model::build`] reads every spine document with a forgiving HTML parser. Links are resolved
//! against the archive: an internal link survives only when its target exists, and images only
//! when the file exists, so a writer never has to deal with dangling references.

use std::collections::HashSet;

use bookx_core::book::OpenBook;
use bookx_core::toc::flatten;
use bookx_core::util::{extension, is_external, parent_dir, resolve_href};
use scraper::{ElementRef, Node};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Fmt {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub sup: bool,
    pub sub: bool,
    pub mono: bool,
    pub small: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    External(String),
    /// Anchor key, see [`anchor_key`].
    Internal(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Text {
        text: String,
        fmt: Fmt,
        link: Option<Link>,
    },
    /// An image that exists in the archive.
    Image {
        path: String,
        alt: String,
    },
    Break,
    /// A link target, as an anchor key.
    Anchor(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub header: bool,
    pub colspan: usize,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Heading {
        level: u8,
        align: Align,
        inlines: Vec<Inline>,
    },
    Para {
        align: Align,
        inlines: Vec<Inline>,
    },
    /// Preformatted text: runs may contain `\n`.
    Pre(Vec<Inline>),
    Quote(Vec<Block>),
    Rule,
    List {
        ordered: bool,
        items: Vec<Vec<Block>>,
    },
    Table {
        rows: Vec<Vec<Cell>>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub path: String,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocItem {
    /// 0 = top level.
    pub depth: usize,
    pub title: String,
    pub key: String,
}

#[derive(Debug, Clone)]
pub struct Cover {
    pub path: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct Model {
    pub sections: Vec<Section>,
    /// Table of contents entries whose targets exist in the model.
    pub toc: Vec<TocItem>,
    /// The cover image, unless the first section already shows it.
    pub cover: Option<Cover>,
    /// Keys of every anchor that some internal link or toc entry points to.
    pub linked: HashSet<String>,
}

/// The key of a link target: `path` for a whole file, `path#fragment` for an element.
pub fn anchor_key(path: &str, fragment: Option<&str>) -> String {
    match fragment {
        Some(f) if !f.is_empty() => format!("{path}#{f}"),
        _ => path.to_string(),
    }
}

/// Concatenated text of inline runs (images contribute nothing, breaks a space).
pub fn inline_text(inlines: &[Inline]) -> String {
    let mut s = String::new();
    for i in inlines {
        match i {
            Inline::Text { text, .. } => s.push_str(text),
            Inline::Break => s.push(' '),
            _ => {}
        }
    }
    s
}

/// Plain text of blocks, one line per paragraph.
pub fn blocks_text(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        match b {
            Block::Heading { inlines, .. } | Block::Para { inlines, .. } | Block::Pre(inlines) => {
                out.push_str(&inline_text(inlines));
                out.push('\n');
            }
            Block::Quote(inner) => out.push_str(&blocks_text(inner)),
            Block::Rule => {}
            Block::List { items, .. } => {
                for it in items {
                    out.push_str(&blocks_text(it));
                }
            }
            Block::Table { rows } => {
                for r in rows {
                    for c in r {
                        out.push_str(&blocks_text(&c.blocks));
                    }
                }
            }
        }
    }
    out
}

/// All inline runs of `blocks` in one line: paragraphs are separated by a space. Used for table
/// cells in formats that only have text lines.
pub fn flat_inlines(blocks: &[Block]) -> Vec<Inline> {
    fn push_space(out: &mut Vec<Inline>) {
        let ends_with_space =
            matches!(out.last(), Some(Inline::Text { text, .. }) if text.ends_with(' '));
        if !out.is_empty() && !ends_with_space {
            out.push(Inline::Text {
                text: " ".into(),
                fmt: Fmt::default(),
                link: None,
            });
        }
    }
    let mut out: Vec<Inline> = Vec::new();
    for b in blocks {
        match b {
            Block::Heading { inlines, .. } | Block::Para { inlines, .. } | Block::Pre(inlines) => {
                push_space(&mut out);
                out.extend(inlines.iter().map(|i| match i {
                    Inline::Text { text, fmt, link } => Inline::Text {
                        text: text.replace('\n', " "),
                        fmt: *fmt,
                        link: link.clone(),
                    },
                    other => other.clone(),
                }))
            }
            Block::Quote(inner) => {
                push_space(&mut out);
                out.extend(flat_inlines(inner))
            }
            Block::Rule => {}
            Block::List { items, .. } => {
                for it in items {
                    push_space(&mut out);
                    out.extend(flat_inlines(it));
                }
            }
            Block::Table { rows } => {
                for c in rows.iter().flatten() {
                    push_space(&mut out);
                    out.extend(flat_inlines(&c.blocks));
                }
            }
        }
    }
    while matches!(out.last(), Some(Inline::Text { text, .. }) if text.trim().is_empty()) {
        out.pop();
    }
    out
}

/// Replaces every `\n` inside text runs by [`Inline::Break`] (preformatted text for formats
/// where a newline in text means nothing).
pub fn split_lines(inlines: &[Inline]) -> Vec<Inline> {
    let mut out = Vec::new();
    for i in inlines {
        match i {
            Inline::Text { text, fmt, link } if text.contains('\n') => {
                for (n, part) in text.split('\n').enumerate() {
                    if n > 0 {
                        out.push(Inline::Break);
                    }
                    if !part.is_empty() {
                        out.push(Inline::Text {
                            text: part.to_string(),
                            fmt: *fmt,
                            link: link.clone(),
                        });
                    }
                }
            }
            other => out.push(other.clone()),
        }
    }
    out
}

fn has_content(inlines: &[Inline]) -> bool {
    inlines.iter().any(|i| match i {
        Inline::Text { text, .. } => !text.is_empty(),
        Inline::Image { .. } => true,
        Inline::Break | Inline::Anchor(_) => false,
    })
}

#[derive(Clone)]
struct Ctx {
    fmt: Fmt,
    link: Option<Link>,
    pre: bool,
    align: Align,
    heading: Option<u8>,
}

#[derive(Default)]
struct Acc {
    blocks: Vec<Block>,
    cur: Vec<Inline>,
    pending: Vec<Inline>,
}

struct Parser<'a> {
    book: &'a OpenBook,
    path: &'a str,
    base: &'a str,
    anchors: Vec<String>,
}

const SKIPPED: &[&str] = &[
    "head", "script", "style", "title", "meta", "link", "noscript", "template", "object", "iframe",
    "audio", "video", "canvas",
];

fn style_has(el: &ElementRef, needle: &str) -> bool {
    el.value()
        .attr("style")
        .map(|s| s.to_ascii_lowercase().replace(' ', "").contains(needle))
        .unwrap_or(false)
}

fn is_space(c: char) -> bool {
    c.is_whitespace() && c != '\u{a0}'
}

impl<'a> Parser<'a> {
    fn ends_with_space(acc: &Acc) -> bool {
        match acc
            .cur
            .iter()
            .rev()
            .find(|s| !matches!(s, Inline::Anchor(_)))
        {
            None => true,
            Some(Inline::Text { text, .. }) => text.ends_with(is_space),
            Some(Inline::Break) => true,
            Some(_) => false,
        }
    }

    fn push_text(acc: &mut Acc, t: &str, ctx: &Ctx) {
        let text = if ctx.pre {
            t.replace("\r\n", "\n")
        } else {
            let mut s = String::with_capacity(t.len());
            let mut last_space = Self::ends_with_space(acc);
            for ch in t.chars() {
                if is_space(ch) {
                    if !last_space {
                        s.push(' ');
                        last_space = true;
                    }
                } else {
                    s.push(ch);
                    last_space = false;
                }
            }
            s
        };
        if text.is_empty() {
            return;
        }
        if let Some(Inline::Text {
            text: prev,
            fmt,
            link,
        }) = acc.cur.last_mut()
        {
            if *fmt == ctx.fmt && *link == ctx.link {
                prev.push_str(&text);
                return;
            }
        }
        acc.cur.push(Inline::Text {
            text,
            fmt: ctx.fmt,
            link: ctx.link.clone(),
        });
    }

    fn flush(&mut self, acc: &mut Acc, ctx: &Ctx) {
        let mut inl = std::mem::take(&mut acc.cur);
        if !ctx.pre {
            // trim the edges
            while let Some(i) = inl.iter().position(|s| !matches!(s, Inline::Anchor(_))) {
                match &mut inl[i] {
                    Inline::Text { text, .. } => {
                        let t = text.trim_start_matches(is_space).to_string();
                        if t.is_empty() {
                            inl.remove(i);
                            continue;
                        }
                        *text = t;
                    }
                    Inline::Break => {
                        inl.remove(i);
                        continue;
                    }
                    _ => {}
                }
                break;
            }
            while let Some(i) = inl.iter().rposition(|s| !matches!(s, Inline::Anchor(_))) {
                match &mut inl[i] {
                    Inline::Text { text, .. } => {
                        let t = text.trim_end_matches(is_space).to_string();
                        if t.is_empty() {
                            inl.remove(i);
                            continue;
                        }
                        *text = t;
                    }
                    Inline::Break => {
                        inl.remove(i);
                        continue;
                    }
                    _ => {}
                }
                break;
            }
        }
        if !has_content(&inl) {
            acc.pending.extend(inl);
            return;
        }
        let mut full = std::mem::take(&mut acc.pending);
        full.extend(inl);
        let block = if ctx.pre {
            Block::Pre(full)
        } else if let Some(level) = ctx.heading {
            Block::Heading {
                level,
                align: ctx.align,
                inlines: full,
            }
        } else {
            Block::Para {
                align: ctx.align,
                inlines: full,
            }
        };
        acc.blocks.push(block);
    }

    /// Anchors that never found a block go to the end of the previous block.
    fn finish(acc: &mut Acc) -> Vec<Block> {
        let pend = std::mem::take(&mut acc.pending);
        if !pend.is_empty() {
            match acc.blocks.last_mut() {
                Some(
                    Block::Para { inlines, .. }
                    | Block::Heading { inlines, .. }
                    | Block::Pre(inlines),
                ) => inlines.extend(pend),
                _ => {
                    acc.blocks.push(Block::Para {
                        align: Align::Left,
                        inlines: pend,
                    });
                }
            }
        }
        std::mem::take(&mut acc.blocks)
    }

    fn resolve_link(&self, href: &str) -> Option<Link> {
        let href = href.trim();
        if href.is_empty() {
            return None;
        }
        if is_external(href) {
            return Some(Link::External(href.to_string()));
        }
        if href.contains(':') && !href.starts_with('#') && !href.contains('/') {
            return None;
        }
        let (path, frag) = resolve_href(self.base, href);
        let path = if path.is_empty() {
            self.path.to_string()
        } else {
            path
        };
        Some(Link::Internal(anchor_key(&path, frag.as_deref())))
    }

    fn note_anchor(&mut self, acc: &mut Acc, el: &ElementRef) {
        for attr in ["id", "name"] {
            if let Some(id) = el.value().attr(attr) {
                if !id.is_empty() {
                    let key = anchor_key(self.path, Some(id));
                    self.anchors.push(key.clone());
                    acc.cur.push(Inline::Anchor(key));
                }
            }
        }
    }

    fn walk_children(&mut self, el: ElementRef, ctx: &Ctx, acc: &mut Acc) {
        for c in el.children() {
            match c.value() {
                Node::Element(_) => {
                    if let Some(ce) = ElementRef::wrap(c) {
                        self.walk(ce, ctx, acc);
                    }
                }
                Node::Text(t) => {
                    let text: &str = t;
                    Self::push_text(acc, text, ctx);
                }
                _ => {}
            }
        }
    }

    fn walk_blocks(&mut self, el: ElementRef, ctx: &Ctx) -> Vec<Block> {
        let mut acc = Acc::default();
        self.walk_children(el, ctx, &mut acc);
        self.flush(&mut acc, ctx);
        Self::finish(&mut acc)
    }

    fn walk(&mut self, el: ElementRef, ctx: &Ctx, acc: &mut Acc) {
        let name = el.value().name().to_ascii_lowercase();
        if SKIPPED.contains(&name.as_str()) {
            return;
        }
        let mut ctx = ctx.clone();
        if style_has(&el, "font-weight:bold") || style_has(&el, "font-weight:700") {
            ctx.fmt.bold = true;
        }
        if style_has(&el, "font-style:italic") {
            ctx.fmt.italic = true;
        }
        if style_has(&el, "text-decoration:underline") {
            ctx.fmt.underline = true;
        }
        if style_has(&el, "text-decoration:line-through") {
            ctx.fmt.strike = true;
        }
        if style_has(&el, "monospace") {
            ctx.fmt.mono = true;
        }
        if style_has(&el, "text-align:center") || el.value().attr("align") == Some("center") {
            ctx.align = Align::Center;
        } else if style_has(&el, "text-align:right") || el.value().attr("align") == Some("right") {
            ctx.align = Align::Right;
        } else if let Some(cls) = el.value().attr("class") {
            let c = cls.to_ascii_lowercase();
            if c.split_whitespace()
                .any(|w| matches!(w, "center" | "centered" | "text-center"))
            {
                ctx.align = Align::Center;
            }
        }

        match name.as_str() {
            "br" => {
                if ctx.pre {
                    Self::push_text(acc, "\n", &ctx);
                } else {
                    acc.cur.push(Inline::Break);
                }
                return;
            }
            "img" | "image" => {
                self.note_anchor(acc, &el);
                let src = el
                    .value()
                    .attr("src")
                    .or_else(|| {
                        el.value()
                            .attrs()
                            .find(|(k, _)| k.ends_with("href") || *k == "src")
                            .map(|(_, v)| v)
                    })
                    .unwrap_or("");
                let alt = el.value().attr("alt").unwrap_or("").to_string();
                if !src.is_empty() && !is_external(src) {
                    let (path, _) = resolve_href(self.base, src);
                    if self.book.archive.contains(&path) {
                        acc.cur.push(Inline::Image { path, alt });
                        return;
                    }
                }
                if !alt.trim().is_empty() {
                    Self::push_text(acc, &alt, &ctx);
                }
                return;
            }
            "hr" => {
                self.flush(acc, &ctx);
                acc.blocks.push(Block::Rule);
                return;
            }
            _ => {}
        }

        let heading = match name.as_str() {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => Some(name.as_bytes()[1] - b'0'),
            _ => None,
        };
        let block_like = heading.is_some()
            || matches!(
                name.as_str(),
                "p" | "div"
                    | "section"
                    | "article"
                    | "header"
                    | "footer"
                    | "aside"
                    | "nav"
                    | "main"
                    | "figure"
                    | "figcaption"
                    | "address"
                    | "dd"
                    | "dt"
                    | "dl"
                    | "body"
                    | "html"
                    | "center"
                    | "details"
                    | "summary"
                    | "form"
                    | "fieldset"
                    | "caption"
            );

        match name.as_str() {
            "blockquote" => {
                self.flush(acc, &ctx);
                self.note_anchor(acc, &el);
                let mut inner_acc = Acc::default();
                inner_acc.cur.append(&mut acc.cur);
                self.walk_children(el, &ctx, &mut inner_acc);
                self.flush(&mut inner_acc, &ctx);
                let inner = Self::finish(&mut inner_acc);
                if !inner.is_empty() {
                    acc.blocks.push(Block::Quote(inner));
                }
                return;
            }
            "pre" => {
                self.flush(acc, &ctx);
                self.note_anchor(acc, &el);
                let mut c = ctx.clone();
                c.pre = true;
                c.fmt.mono = true;
                self.walk_children(el, &c, acc);
                // a trailing newline inside <pre> is not a blank line
                if let Some(Inline::Text { text, .. }) = acc.cur.last_mut() {
                    while text.ends_with('\n') {
                        text.pop();
                    }
                }
                self.flush(acc, &c);
                return;
            }
            "ul" | "ol" => {
                self.flush(acc, &ctx);
                self.note_anchor(acc, &el);
                let mut items: Vec<Vec<Block>> = Vec::new();
                for c in el.children() {
                    match c.value() {
                        Node::Element(_) => {
                            let Some(ce) = ElementRef::wrap(c) else {
                                continue;
                            };
                            if ce.value().name().eq_ignore_ascii_case("li") {
                                let mut item = Acc::default();
                                self.note_anchor(&mut item, &ce);
                                self.walk_children(ce, &ctx, &mut item);
                                self.flush(&mut item, &ctx);
                                items.push(Self::finish(&mut item));
                            } else if !SKIPPED.contains(&ce.value().name()) {
                                let mut item = Acc::default();
                                self.walk(ce, &ctx, &mut item);
                                self.flush(&mut item, &ctx);
                                let blocks = Self::finish(&mut item);
                                match items.last_mut() {
                                    Some(last)
                                        if matches!(blocks.first(), Some(Block::List { .. })) =>
                                    {
                                        last.extend(blocks)
                                    }
                                    _ => items.push(blocks),
                                }
                            }
                        }
                        Node::Text(t) => {
                            let text: &str = t;
                            if !text.trim().is_empty() {
                                let mut item = Acc::default();
                                Self::push_text(&mut item, text, &ctx);
                                self.flush(&mut item, &ctx);
                                items.push(Self::finish(&mut item));
                            }
                        }
                        _ => {}
                    }
                }
                items.retain(|i| !i.is_empty());
                if !items.is_empty() {
                    acc.blocks.push(Block::List {
                        ordered: name == "ol",
                        items,
                    });
                }
                return;
            }
            "table" => {
                self.flush(acc, &ctx);
                self.note_anchor(acc, &el);
                let mut rows = Vec::new();
                self.collect_rows(el, &ctx, &mut rows, acc);
                if !rows.is_empty() {
                    let pend = std::mem::take(&mut acc.cur);
                    acc.pending.extend(pend);
                    acc.blocks.push(Block::Table { rows });
                }
                return;
            }
            _ => {}
        }

        let mut c = ctx.clone();
        match name.as_str() {
            "b" | "strong" => c.fmt.bold = true,
            "i" | "em" | "cite" | "var" | "dfn" => c.fmt.italic = true,
            "u" | "ins" => c.fmt.underline = true,
            "s" | "strike" | "del" => c.fmt.strike = true,
            "code" | "tt" | "kbd" | "samp" => c.fmt.mono = true,
            "sup" => c.fmt.sup = true,
            "sub" => c.fmt.sub = true,
            "small" => c.fmt.small = true,
            "a" => {
                if let Some(h) = el.value().attr("href") {
                    c.link = self.resolve_link(h);
                }
            }
            _ => {}
        }
        if let Some(l) = heading {
            c.heading = Some(l);
        }
        if name == "dd" {
            // definitions are shown as indented quotes
            self.flush(acc, &ctx);
            self.note_anchor(acc, &el);
            let mut inner = Acc::default();
            inner.cur.append(&mut acc.cur);
            self.walk_children(el, &c, &mut inner);
            self.flush(&mut inner, &c);
            let blocks = Self::finish(&mut inner);
            if !blocks.is_empty() {
                acc.blocks.push(Block::Quote(blocks));
            }
            return;
        }
        if block_like {
            self.flush(acc, &ctx);
            if name == "dt" {
                c.fmt.bold = true;
            }
        }
        self.note_anchor(acc, &el);
        self.walk_children(el, &c, acc);
        if block_like {
            self.flush(acc, &c);
        }
    }

    fn collect_rows(
        &mut self,
        el: ElementRef,
        ctx: &Ctx,
        rows: &mut Vec<Vec<Cell>>,
        acc: &mut Acc,
    ) {
        for c in el.children() {
            let Some(ce) = ElementRef::wrap(c) else {
                continue;
            };
            match ce.value().name().to_ascii_lowercase().as_str() {
                "thead" | "tbody" | "tfoot" => self.collect_rows(ce, ctx, rows, acc),
                "caption" => self.walk(ce, ctx, acc),
                "tr" => {
                    let mut cells = Vec::new();
                    for td in ce.children().filter_map(ElementRef::wrap) {
                        let n = td.value().name().to_ascii_lowercase();
                        if n != "td" && n != "th" {
                            continue;
                        }
                        let header = n == "th";
                        let mut cc = ctx.clone();
                        if header {
                            cc.fmt.bold = true;
                        }
                        let mut cell_acc = Acc::default();
                        self.note_anchor(&mut cell_acc, &td);
                        self.walk_children(td, &cc, &mut cell_acc);
                        self.flush(&mut cell_acc, &cc);
                        let colspan = td
                            .value()
                            .attr("colspan")
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(1)
                            .clamp(1, 64);
                        cells.push(Cell {
                            header,
                            colspan,
                            blocks: Self::finish(&mut cell_acc),
                        });
                    }
                    if !cells.is_empty() {
                        rows.push(cells);
                    }
                }
                _ => {}
            }
        }
    }
}

fn is_markup(path: &str) -> bool {
    matches!(
        extension(path).as_str(),
        "xhtml" | "html" | "htm" | "xml" | "html5"
    )
}

fn collect_blocks<'a>(blocks: &'a [Block], out: &mut Vec<&'a Inline>) {
    for b in blocks {
        match b {
            Block::Heading { inlines, .. } | Block::Para { inlines, .. } | Block::Pre(inlines) => {
                out.extend(inlines.iter())
            }
            Block::Quote(inner) => collect_blocks(inner, out),
            Block::Rule => {}
            Block::List { items, .. } => items.iter().for_each(|i| collect_blocks(i, out)),
            Block::Table { rows } => rows
                .iter()
                .flatten()
                .for_each(|c| collect_blocks(&c.blocks, out)),
        }
    }
}

fn strip_dead_links(blocks: &mut [Block], anchors: &HashSet<String>, linked: &mut HashSet<String>) {
    fn fix(inlines: &mut [Inline], anchors: &HashSet<String>, linked: &mut HashSet<String>) {
        for i in inlines {
            if let Inline::Text { link, .. } = i {
                if let Some(Link::Internal(k)) = link {
                    if anchors.contains(k) {
                        linked.insert(k.clone());
                    } else {
                        *link = None;
                    }
                }
            }
        }
    }
    for b in blocks {
        match b {
            Block::Heading { inlines, .. } | Block::Para { inlines, .. } | Block::Pre(inlines) => {
                fix(inlines, anchors, linked)
            }
            Block::Quote(inner) => strip_dead_links(inner, anchors, linked),
            Block::Rule => {}
            Block::List { items, .. } => items
                .iter_mut()
                .for_each(|i| strip_dead_links(i, anchors, linked)),
            Block::Table { rows } => rows
                .iter_mut()
                .flatten()
                .for_each(|c| strip_dead_links(&mut c.blocks, anchors, linked)),
        }
    }
}

impl Model {
    /// Builds the model from the spine, table of contents and cover of `book`.
    pub fn build(book: &OpenBook) -> Model {
        let mut sections = Vec::new();
        let mut anchors: HashSet<String> = HashSet::new();
        for path in book.spine_paths() {
            if !is_markup(&path) {
                continue;
            }
            let Some(src) = book.archive.get_string(&path) else {
                continue;
            };
            let html = bookx_core::util::parse_html_document(&src);
            let mut p = Parser {
                book,
                path: &path,
                base: parent_dir(&path),
                anchors: Vec::new(),
            };
            let ctx = Ctx {
                fmt: Fmt::default(),
                link: None,
                pre: false,
                align: Align::Left,
                heading: None,
            };
            let blocks = p.walk_blocks(html.root_element(), &ctx);
            if blocks.is_empty() {
                continue;
            }
            anchors.insert(path.clone());
            anchors.extend(p.anchors);
            sections.push(Section { path, blocks });
        }

        let mut linked = HashSet::new();
        for s in &mut sections {
            strip_dead_links(&mut s.blocks, &anchors, &mut linked);
        }

        let mut toc = Vec::new();
        for f in flatten(&book.toc()) {
            let (path, frag) = match f.href.split_once('#') {
                Some((p, fr)) => (p.to_string(), Some(fr.to_string())),
                None => (f.href.clone(), None),
            };
            let key = anchor_key(&path, frag.as_deref());
            let key = if anchors.contains(&key) {
                key
            } else if anchors.contains(&path) {
                path
            } else {
                continue;
            };
            linked.insert(key.clone());
            toc.push(TocItem {
                depth: f.depth,
                title: f.title.trim().to_string(),
                key,
            });
        }

        let cover = book
            .package
            .cover_path()
            .zip(book.cover_bytes())
            .map(|(path, data)| Cover {
                path,
                data: data.to_vec(),
            });
        let mut model = Model {
            sections,
            toc,
            cover,
            linked,
        };
        if let Some(c) = &model.cover {
            let shown = model.sections.first().is_some_and(|s| {
                let mut inl = Vec::new();
                collect_blocks(&s.blocks[..s.blocks.len().min(2)], &mut inl);
                inl.iter()
                    .any(|i| matches!(i, Inline::Image { path, .. } if *path == c.path))
            });
            if shown {
                model.cover = None;
            }
        }
        model
    }

    /// Archive paths of every image used by the sections, in order of first use.
    pub fn image_paths(&self) -> Vec<String> {
        let mut seen = Vec::new();
        for s in &self.sections {
            let mut inl = Vec::new();
            collect_blocks(&s.blocks, &mut inl);
            for i in inl {
                if let Inline::Image { path, .. } = i {
                    if !seen.contains(path) {
                        seen.push(path.clone());
                    }
                }
            }
        }
        seen
    }

    /// Title of the first heading of a section, used when the toc has no entry for it.
    pub fn section_title(&self, index: usize) -> Option<String> {
        let s = self.sections.get(index)?;
        if let Some(t) = self.toc.iter().find(|t| t.key == s.path) {
            if !t.title.is_empty() {
                return Some(t.title.clone());
            }
        }
        s.blocks.iter().find_map(|b| match b {
            Block::Heading { inlines, .. } => {
                let t = inline_text(inlines).trim().to_string();
                (!t.is_empty()).then_some(t)
            }
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::testutil::book_with;

    fn first_blocks(book: &OpenBook) -> Vec<Block> {
        Model::build(book).sections[0].blocks.clone()
    }

    #[test]
    fn paragraph_styles_and_whitespace() {
        let b = book_with(&[
            "<p>  Hello \n  <b>bold <i>both</i></b>  <u>u</u><s>s</s><sup>1</sup><sub>2</sub><code>c</code>  </p>",
        ]);
        let blocks = first_blocks(&b);
        let Block::Para { inlines, .. } = &blocks[0] else {
            panic!("{blocks:?}");
        };
        assert_eq!(inline_text(inlines), "Hello bold both us12c");
        let fmts: Vec<Fmt> = inlines
            .iter()
            .filter_map(|i| match i {
                Inline::Text { fmt, .. } => Some(*fmt),
                _ => None,
            })
            .collect();
        assert!(fmts.iter().any(|f| f.bold && f.italic));
        assert!(fmts.iter().any(|f| f.underline));
        assert!(fmts.iter().any(|f| f.strike));
        assert!(fmts.iter().any(|f| f.sup));
        assert!(fmts.iter().any(|f| f.sub));
        assert!(fmts.iter().any(|f| f.mono));
    }

    #[test]
    fn headings_levels_and_alignment() {
        let b = book_with(&[
            "<h2 id=\"x\">Two</h2><p style=\"text-align: center\">c</p><p class=\"right\" align=\"right\">r</p>",
        ]);
        let blocks = first_blocks(&b);
        assert!(matches!(&blocks[0], Block::Heading { level: 2, .. }));
        assert!(matches!(
            &blocks[1],
            Block::Para {
                align: Align::Center,
                ..
            }
        ));
        assert!(matches!(
            &blocks[2],
            Block::Para {
                align: Align::Right,
                ..
            }
        ));
        let Block::Heading { inlines, .. } = &blocks[0] else {
            unreachable!()
        };
        assert!(matches!(&inlines[0], Inline::Anchor(k) if k.ends_with("#x")));
    }

    #[test]
    fn lists_nest_and_tables_have_cells() {
        let b = book_with(&[
            "<ul><li>a<ul><li>a1</li></ul></li><li>b</li></ul><ol><li>x</li></ol><table><thead><tr><th colspan=\"2\">H</th></tr></thead><tr><td>1</td><td><p>2</p></td></tr></table>",
        ]);
        let blocks = first_blocks(&b);
        let Block::List { ordered, items } = &blocks[0] else {
            panic!("{blocks:?}");
        };
        assert!(!ordered);
        assert_eq!(items.len(), 2);
        assert!(items[0].iter().any(|x| matches!(x, Block::List { .. })));
        assert!(matches!(&blocks[1], Block::List { ordered: true, .. }));
        let Block::Table { rows } = &blocks[2] else {
            panic!("{blocks:?}");
        };
        assert_eq!(rows.len(), 2);
        assert!(rows[0][0].header);
        assert_eq!(rows[0][0].colspan, 2);
        assert_eq!(blocks_text(&rows[1][1].blocks).trim(), "2");
    }

    #[test]
    fn pre_quote_rule_and_breaks() {
        let b = book_with(&[
            "<pre>a  b\nc\n</pre><blockquote><p>q</p></blockquote><hr/><p>x<br/>y</p>",
        ]);
        let blocks = first_blocks(&b);
        let Block::Pre(inl) = &blocks[0] else {
            panic!("{blocks:?}");
        };
        assert_eq!(inline_text(inl), "a  b\nc");
        assert!(matches!(&blocks[1], Block::Quote(q) if q.len() == 1));
        assert_eq!(blocks[2], Block::Rule);
        let Block::Para { inlines, .. } = &blocks[3] else {
            unreachable!()
        };
        assert!(inlines.contains(&Inline::Break));
    }

    #[test]
    fn links_resolve_and_dead_ones_are_dropped() {
        let b = book_with(&[
            "<p id=\"p1\"><a href=\"chapter-2.xhtml#t\">live</a> <a href=\"chapter-2.xhtml#nope\">dead</a> <a href=\"https://x.org\">ext</a> <a href=\"#p1\">self</a></p>",
            "<h1 id=\"t\">T</h1>",
        ]);
        let m = Model::build(&b);
        let Block::Para { inlines, .. } = &m.sections[0].blocks[0] else {
            panic!()
        };
        let links: Vec<_> = inlines
            .iter()
            .filter_map(|i| match i {
                Inline::Text { text, link, .. } => Some((text.as_str(), link.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            links[0].1,
            Some(Link::Internal("OEBPS/chapter-2.xhtml#t".into()))
        );
        assert!(
            links
                .iter()
                .any(|(t, l)| *t == "ext" && *l == Some(Link::External("https://x.org".into())))
        );
        assert!(links.iter().any(|(t, l)| t.contains("dead") && l.is_none()));
        assert!(m.linked.contains("OEBPS/chapter-2.xhtml#t"));
        assert!(m.linked.contains("OEBPS/chapter-1.xhtml#p1"));
        assert!(!m.linked.contains("OEBPS/chapter-2.xhtml#nope"));
    }

    #[test]
    fn images_exist_or_fall_back_to_alt_text() {
        let b = book_with(&[
            "<p><img src=\"pic.png\" alt=\"A\"/><img src=\"gone.png\" alt=\"fallback\"/></p>",
        ]);
        let m = Model::build(&b);
        let Block::Para { inlines, .. } = &m.sections[0].blocks[0] else {
            panic!()
        };
        assert!(matches!(&inlines[0], Inline::Image { path, .. } if path == "OEBPS/pic.png"));
        assert_eq!(inline_text(inlines), "fallback");
        assert_eq!(m.image_paths(), vec!["OEBPS/pic.png".to_string()]);
    }

    #[test]
    fn empty_sections_are_skipped_and_titles_found() {
        let b = book_with(&["<div></div>", "<h1>Second</h1><p>x</p>"]);
        let m = Model::build(&b);
        assert_eq!(m.sections.len(), 1);
        assert_eq!(m.section_title(0).as_deref(), Some("T1"));
        assert_eq!(m.section_title(5), None);
    }

    #[test]
    fn anchors_without_content_attach_to_the_previous_block() {
        let b = book_with(&["<p>text</p><div id=\"end\"></div>"]);
        let m = Model::build(&b);
        let Block::Para { inlines, .. } = &m.sections[0].blocks[0] else {
            panic!()
        };
        assert!(
            inlines
                .iter()
                .any(|i| matches!(i, Inline::Anchor(k) if k.ends_with("#end")))
        );
    }

    #[test]
    fn definition_lists_and_unknown_markup() {
        let b = book_with(&[
            "<dl><dt>Term</dt><dd>Meaning</dd></dl><script>x()</script><div>loose <span>text</span></div>",
        ]);
        let blocks = first_blocks(&b);
        assert_eq!(blocks_text(&blocks), "Term\nMeaning\nloose text\n");
        assert!(matches!(&blocks[1], Block::Quote(_)));
    }

    #[test]
    fn nbsp_is_kept() {
        let b = book_with(&["<p>a&#160;&#160;b</p>"]);
        let blocks = first_blocks(&b);
        assert_eq!(blocks_text(&blocks), "a\u{a0}\u{a0}b\n");
    }

    #[test]
    fn anchor_key_and_text_helpers() {
        assert_eq!(anchor_key("a.xhtml", None), "a.xhtml");
        assert_eq!(anchor_key("a.xhtml", Some("")), "a.xhtml");
        assert_eq!(anchor_key("a.xhtml", Some("x")), "a.xhtml#x");
        assert_eq!(
            inline_text(&[
                Inline::Break,
                Inline::Anchor("x".into()),
                Inline::Image {
                    path: "p".into(),
                    alt: "".into()
                }
            ]),
            " "
        );
    }

    #[test]
    fn flat_inlines_joins_paragraphs_with_spaces() {
        let b = book_with(&[
            "<p>one</p><p><b>two</b></p><ul><li>three</li></ul><table><tr><td>four</td></tr></table>",
        ]);
        let blocks = first_blocks(&b);
        let flat = flat_inlines(&blocks);
        assert_eq!(inline_text(&flat), "one two three four");
        assert!(
            flat.iter()
                .any(|i| matches!(i, Inline::Text { fmt, text, .. } if fmt.bold && text == "two"))
        );
        assert!(flat_inlines(&[]).is_empty());
        let pre = [Block::Pre(vec![Inline::Text {
            text: "a\nb".into(),
            fmt: Fmt::default(),
            link: None,
        }])];
        assert_eq!(inline_text(&flat_inlines(&pre)), "a b");
    }

    #[test]
    fn split_lines_turns_newlines_into_breaks() {
        let t = |s: &str| Inline::Text {
            text: s.into(),
            fmt: Fmt::default(),
            link: None,
        };
        let out = split_lines(&[t("a\nb\n"), Inline::Anchor("x".into()), t("plain")]);
        assert_eq!(
            out,
            vec![
                t("a"),
                Inline::Break,
                t("b"),
                Inline::Break,
                Inline::Anchor("x".into()),
                t("plain")
            ]
        );
        assert_eq!(split_lines(&[]), vec![]);
        assert_eq!(split_lines(&[t("\n")]), vec![Inline::Break]);
    }

    #[test]
    fn blocks_text_covers_every_block_kind() {
        let b = book_with(&[
            "<h1>H</h1><blockquote><p>Q</p></blockquote><hr/><ul><li>L</li></ul><table><tr><td>C</td></tr></table><pre>P</pre>",
        ]);
        assert_eq!(blocks_text(&first_blocks(&b)), "H\nQ\nL\nC\nP\n");
    }

    #[test]
    fn cover_shown_in_the_first_section_is_not_repeated() {
        let mut b = book_with(&["<p><img src=\"pic.png\"/></p><p>text</p>"]);
        b.set_cover(crate::testutil::png(8, 6), "png").unwrap();
        let m = Model::build(&b);
        assert!(m.cover.is_some(), "a cover that is not in the text is kept");
        let cover_path = b.package.cover_path().unwrap();
        let mut b2 = book_with(&["<p>text</p>"]);
        b2.archive.set(&cover_path, crate::testutil::png(8, 6));
        let body = format!(
            "<p><img src=\"{}\"/></p>",
            bookx_core::util::relative_href("OEBPS/chapter-1.xhtml", &cover_path)
        );
        let mut b3 = book_with(&[&body]);
        b3.set_cover(crate::testutil::png(8, 6), "png").unwrap();
        let cp = b3.package.cover_path().unwrap();
        let body = format!(
            "<p><img src=\"{}\"/></p>",
            bookx_core::util::relative_href("OEBPS/chapter-1.xhtml", &cp)
        );
        b3.archive.set_string(
            "OEBPS/chapter-1.xhtml",
            &format!("<html><body>{body}<p>text</p></body></html>"),
        );
        let m3 = Model::build(&b3);
        assert!(m3.cover.is_none(), "cover already shown on the first page");
    }
}
