//! XHTML / HTML chapter -> [`Doc`].

use bookx_text::css::{ComputedStyle, Display, Float, FontStyle, StyleSheetSet, TextAlign};
use scraper::{ElementRef, Node};

use super::doc::*;
use super::util::*;

#[derive(Clone)]
struct Ctx {
    kind: BlockKind,
    indent: u8,
    align: Align,
    pre: bool,
    css: BlockCss,
    float: FloatSide,
}

/// The author's style sheets, applied on top of the plain structure of the document.
///
/// Only what the stylesheets *change* is applied: every element is computed once with the
/// built-in defaults alone and once with the book's CSS, and the differences (bold, italic,
/// underline, strike-through, small caps, monospace, alignment, `display: none`) are used. That
/// keeps `<h1>`, `<b>` and friends exactly as they are, so exports stay clean.
pub struct Styling {
    base: StyleSheetSet,
    author: StyleSheetSet,
}

impl Styling {
    pub fn new(sheets: &[String]) -> Styling {
        Styling::layered(sheets, "", true)
    }

    /// The book's style sheets (`publisher`: obeyed or not) with the reader's own style sheet on
    /// top. The user's rules win over the book's, as in a browser.
    pub fn layered(sheets: &[String], user_css: &str, publisher: bool) -> Styling {
        let mut author = StyleSheetSet::new().ignore_publisher_styles(!publisher);
        for s in sheets {
            author = author.add_author(s);
        }
        if !user_css.trim().is_empty() {
            author = author.add_user(user_css);
        }
        Styling {
            base: StyleSheetSet::new(),
            author,
        }
    }
}

type CssPair = (ComputedStyle, ComputedStyle);

struct Builder<'a> {
    styling: Option<&'a Styling>,
    base_dir: &'a str,
    this_path: &'a str,
    doc: Doc,
    cur: Vec<Span>,
    pending_anchors: Vec<String>,
    pending_marker: Option<String>,
    lists: Vec<(bool, usize)>,
}

pub fn parse_html(source: &str, chapter_path: &str) -> Doc {
    parse_html_styled(source, chapter_path, None)
}

/// Like [`parse_html`], applying the author's CSS when `styling` is given.
pub fn parse_html_styled(source: &str, chapter_path: &str, styling: Option<&Styling>) -> Doc {
    let html = crate::util::parse_html_document(source);
    let mut b = Builder {
        styling,
        base_dir: parent_dir(chapter_path),
        this_path: chapter_path,
        doc: Doc::default(),
        cur: Vec::new(),
        pending_anchors: Vec::new(),
        pending_marker: None,
        lists: Vec::new(),
    };
    let ctx = Ctx {
        kind: BlockKind::Paragraph,
        indent: 0,
        align: Align::Start,
        pre: false,
        css: BlockCss::default(),
        float: FloatSide::None,
    };
    b.walk(html.root_element(), &Style::default(), &None, &ctx, None);
    b.flush(&ctx);
    if !b.doc.writing_mode.is_vertical() {
        if let Some(first) = b
            .doc
            .blocks
            .iter()
            .find(|bl| bl.css.writing_mode.is_vertical())
        {
            b.doc.writing_mode = first.css.writing_mode;
            b.doc.orientation = first.css.orientation;
        }
    }
    if !b.pending_anchors.is_empty() {
        let anchors = std::mem::take(&mut b.pending_anchors);
        if let Some(last) = b.doc.blocks.last_mut() {
            last.spans.extend(anchors.into_iter().map(Span::Anchor));
        }
    }
    b.doc
}

fn style_attr_has(el: &ElementRef, needle: &str) -> bool {
    el.value()
        .attr("style")
        .map(|s| s.to_ascii_lowercase().replace(' ', "").contains(needle))
        .unwrap_or(false)
}

/// `px` expressed in em of a font of `font_px`, clamped to `lo..=hi` (a stylesheet cannot push the
/// text off the page).
/// The font a book asks for: its first family, with the generic names mapped to the system's.
/// Names with anything but letters, digits, spaces, hyphens and dots are dropped (the name ends up in
/// markup).
fn book_font(families: &[String]) -> Option<String> {
    let f = families.first()?.trim();
    let name = match f.to_ascii_lowercase().as_str() {
        "serif" => "Serif",
        "sans-serif" | "system-ui" | "ui-sans-serif" => "Sans",
        "monospace" | "ui-monospace" => return None,
        "cursive" | "fantasy" | "inherit" | "initial" | "" => return None,
        _ => f,
    };
    (name.chars().count() <= 64
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '.' | '_')))
    .then(|| name.to_string())
}

fn em_of(px: f64, font_px: f64, lo: f32, hi: f32) -> f32 {
    if !px.is_finite() || font_px <= 0.0 {
        return 0.0;
    }
    ((px / font_px) as f32).clamp(lo, hi)
}

/// Spacing of a block element from its computed styles. Only what the book's CSS changed counts
/// (compared with the defaults); paragraphs that are indented by the book are packed tight, as
/// in print, unless the book gives them a margin.
fn block_css(
    author: &ComputedStyle,
    base: &ComputedStyle,
    is_paragraph: bool,
    inherited_indent: f32,
    inherited_left: f32,
) -> BlockCss {
    let changed = |a: f64, b: f64| (a - b).abs() > 0.01;
    // in a vertical line the block axis runs sideways: the margins at its ends are the right and
    // left ones, and the top margin is the indent of the line
    let (m_top, m_bottom, b_top, b_bottom) = match author.writing_mode {
        WritingMode::HorizontalTb => (
            author.margin.top,
            author.margin.bottom,
            base.margin.top,
            base.margin.bottom,
        ),
        WritingMode::VerticalRl => (
            author.margin.right,
            author.margin.left,
            base.margin.right,
            base.margin.left,
        ),
        WritingMode::VerticalLr => (
            author.margin.left,
            author.margin.right,
            base.margin.left,
            base.margin.right,
        ),
    };
    let top_changed = changed(m_top, b_top);
    let bottom_changed = changed(m_bottom, b_bottom);
    let tight_paragraph = is_paragraph && inherited_indent > 0.0;
    let font = author.font_size;
    let m = |px: f64| Some(em_of(px, font, 0.0, 3.0));
    let (left, base_left) = if author.writing_mode.is_vertical() {
        (
            author.margin.top + author.padding.top,
            base.margin.top + base.padding.top,
        )
    } else {
        (
            author.margin.left + author.padding.left,
            base.margin.left + base.padding.left,
        )
    };
    BlockCss {
        text_indent: inherited_indent,
        margin_top: if top_changed || tight_paragraph {
            m(m_top)
        } else {
            None
        },
        margin_bottom: if bottom_changed || tight_paragraph {
            m(m_bottom)
        } else {
            None
        },
        // nested blocks add up: a paragraph in an indented division is indented by both
        margin_left: (inherited_left
            + if changed(left, base_left) {
                em_of(left, font, 0.0, 8.0)
            } else {
                0.0
            })
        .min(8.0),
        drop_cap: 0,
        writing_mode: author.writing_mode,
        orientation: author.text_orientation,
    }
}

/// How many lines tall the drop cap of `el` is (`::first-letter` floated left), 0 for none.
fn drop_cap_lines(st: &Styling, el: ElementRef, author: &ComputedStyle) -> u8 {
    let Some(fl) = st.author.first_letter(el, author) else {
        return 0;
    };
    if fl.float != Float::Left || author.font_size <= 0.0 {
        return 0;
    }
    ((fl.font_size / author.font_size * 0.8).ceil() as i32).clamp(2, 6) as u8
}

impl<'a> Builder<'a> {
    fn ends_with_space(&self) -> bool {
        match self
            .cur
            .iter()
            .rev()
            .find(|s| !matches!(s, Span::Anchor(_)))
        {
            None => true,
            Some(Span::Text { text, .. }) => text.ends_with(' ') || text.ends_with('\n'),
            Some(Span::Break) => true,
            Some(_) => false,
        }
    }

    fn push_text(&mut self, t: &str, style: &Style, link: &Option<String>, pre: bool) {
        let text = if pre {
            t.to_string()
        } else {
            let mut s = String::with_capacity(t.len());
            let mut last_space = self.ends_with_space();
            for ch in t.chars() {
                if ch.is_whitespace() {
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
        self.cur.push(Span::Text {
            text,
            style: style.clone(),
            link: link.clone(),
        });
    }

    fn flush(&mut self, ctx: &Ctx) {
        let mut spans = std::mem::take(&mut self.cur);
        if !ctx.pre {
            loop {
                let Some(i) = spans.iter().rposition(|s| !matches!(s, Span::Anchor(_))) else {
                    break;
                };
                match &mut spans[i] {
                    Span::Text { text, .. } => {
                        let t = text.trim_end().len();
                        text.truncate(t);
                        if text.is_empty() {
                            spans.remove(i);
                            continue;
                        }
                    }
                    Span::Break => {
                        spans.remove(i);
                        continue;
                    }
                    _ => {}
                }
                break;
            }
            loop {
                let Some(i) = spans.iter().position(|s| !matches!(s, Span::Anchor(_))) else {
                    break;
                };
                if let Span::Text { text, .. } = &mut spans[i] {
                    *text = text.trim_start().to_string();
                    if text.is_empty() {
                        spans.remove(i);
                        continue;
                    }
                }
                break;
            }
        }
        let has_content = spans.iter().any(|s| !matches!(s, Span::Anchor(_)));
        if !has_content {
            for s in spans {
                if let Span::Anchor(a) = s {
                    self.pending_anchors.push(a);
                }
            }
            return;
        }
        let mut full: Vec<Span> = self.pending_anchors.drain(..).map(Span::Anchor).collect();
        full.extend(spans);
        self.doc.blocks.push(Block {
            kind: ctx.kind,
            spans: full,
            align: ctx.align,
            indent: ctx.indent,
            marker: self.pending_marker.take(),
            css: ctx.css,
        });
    }

    fn resolve_link(&self, href: &str) -> String {
        if is_external(href) {
            return href.to_string();
        }
        let (path, frag) = resolve_href(self.base_dir, href);
        let path = if path.is_empty() {
            self.this_path.to_string()
        } else {
            path
        };
        match frag {
            Some(f) => format!("{path}#{f}"),
            None => path,
        }
    }

    /// Applies the CSS differences of `el` to `style` and `ctx`. Returns the computed pair for the
    /// children, or `None` when the element is hidden.
    fn apply_css(
        &self,
        el: ElementRef,
        parent: Option<&CssPair>,
        style: &mut Style,
        ctx: &mut Ctx,
    ) -> Option<Option<CssPair>> {
        let Some(st) = self.styling else {
            return Some(None);
        };
        let author = st.author.compute_with_parent(el, parent.map(|p| &p.0));
        let base = st.base.compute_with_parent(el, parent.map(|p| &p.1));
        if author.display == Display::None && base.display != Display::None {
            return None;
        }
        if author.font_weight >= 600 && base.font_weight < 600 {
            style.bold = true;
        }
        if matches!(author.font_style, FontStyle::Italic | FontStyle::Oblique)
            && base.font_style == FontStyle::Normal
        {
            style.italic = true;
        }
        if author.text_decoration.underline && !base.text_decoration.underline {
            style.underline = true;
        }
        if author.text_decoration.line_through && !base.text_decoration.line_through {
            style.strike = true;
        }
        if author.small_caps && !base.small_caps {
            style.small_caps = true;
        }
        if author.font_family != base.font_family {
            style.font = book_font(&author.font_family);
        }
        if base.font_size > 0.0 && author.font_size.is_finite() {
            let r = author.font_size / base.font_size;
            if (r - 1.0).abs() > 0.02 {
                style.size_pct = Some((r * 100.0).round().clamp(50.0, 300.0) as u16);
            }
        }
        let mono = |c: &ComputedStyle| {
            c.font_family
                .iter()
                .any(|f| f.eq_ignore_ascii_case("monospace") || f.eq_ignore_ascii_case("courier"))
        };
        if mono(&author) && !mono(&base) {
            style.code = true;
        }
        if author.text_align != base.text_align {
            match author.text_align {
                TextAlign::Center => ctx.align = Align::Center,
                TextAlign::Right => ctx.align = Align::End,
                TextAlign::Left | TextAlign::Justify | TextAlign::Start => {}
            }
        }
        if author.color != base.color {
            if let Some(c) = author.color.filter(|c| c.a > 0) {
                style.color = Some((c.r, c.g, c.b));
            }
        }
        match author.float {
            Float::Left => ctx.float = FloatSide::Left,
            Float::Right => ctx.float = FloatSide::Right,
            Float::None => {}
        }
        // `text-indent` inherits, so every element can simply carry the computed value
        ctx.css.text_indent = em_of(author.text_indent, author.font_size, 0.0, 4.0);
        ctx.css.writing_mode = author.writing_mode;
        ctx.css.orientation = author.text_orientation;
        style.combine = author.text_combine;
        Some(Some((author, base)))
    }

    /// `<ruby>`: every base gets the annotation of its `<rt>` (a base is text up to the next
    /// `<rt>`, or one `<rb>`; `<rb>`s followed by as many `<rt>`s are paired in order). The
    /// annotation goes on the style of the base text, `<rp>` (the fallback brackets) is dropped.
    fn walk_ruby(
        &mut self,
        el: ElementRef,
        style: &Style,
        link: &Option<String>,
        ctx: &Ctx,
        css_here: Option<&CssPair>,
    ) {
        enum Piece<'t> {
            Text(String),
            El(ElementRef<'t>),
        }
        struct Group<'t> {
            nodes: Vec<Piece<'t>>,
            rt: Option<String>,
            open: bool,
        }
        let mut groups: Vec<Group> = Vec::new();
        let mut stack: Vec<_> = el.children().collect();
        stack.reverse();
        while let Some(c) = stack.pop() {
            match c.value() {
                Node::Element(e) => match e.name().to_ascii_lowercase().as_str() {
                    "rp" => {}
                    "rt" => {
                        // `rt { display: none }` (the book's or the user's sheet) hides the furigana
                        let hidden = match (self.styling, ElementRef::wrap(c)) {
                            (Some(st), Some(r)) => {
                                st.author
                                    .compute_with_parent(r, css_here.map(|p| &p.0))
                                    .display
                                    == Display::None
                            }
                            _ => false,
                        };
                        let text: String = ElementRef::wrap(c)
                            .filter(|_| !hidden)
                            .map(|r| r.text().collect::<String>())
                            .unwrap_or_default();
                        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                        if let Some(g) = groups.iter_mut().find(|g| g.rt.is_none()) {
                            g.rt = Some(text);
                            g.open = false;
                        }
                    }
                    "rb" => groups.push(Group {
                        nodes: ElementRef::wrap(c).map(Piece::El).into_iter().collect(),
                        rt: None,
                        open: false,
                    }),
                    "rtc" => {
                        let mut kids: Vec<_> = c.children().collect();
                        kids.reverse();
                        stack.extend(kids);
                    }
                    _ => {
                        let Some(ce) = ElementRef::wrap(c) else {
                            continue;
                        };
                        match groups.last_mut() {
                            Some(g) if g.open => g.nodes.push(Piece::El(ce)),
                            _ => groups.push(Group {
                                nodes: vec![Piece::El(ce)],
                                rt: None,
                                open: true,
                            }),
                        }
                    }
                },
                Node::Text(t) => {
                    if t.trim().is_empty() && groups.last().is_none_or(|g| !g.open) {
                        continue;
                    }
                    let text: &str = t;
                    match groups.last_mut() {
                        Some(g) if g.open => g.nodes.push(Piece::Text(text.to_string())),
                        _ => groups.push(Group {
                            nodes: vec![Piece::Text(text.to_string())],
                            rt: None,
                            open: true,
                        }),
                    }
                }
                _ => {}
            }
        }
        for g in groups {
            let mut st = style.clone();
            st.ruby = g.rt.filter(|r| !r.is_empty());
            for n in g.nodes {
                match n {
                    Piece::Text(t) => self.push_text(&t, &st, link, ctx.pre),
                    Piece::El(ce) if ce.value().name().eq_ignore_ascii_case("rb") => {
                        let text: String = ce.text().collect();
                        self.push_text(&text, &st, link, ctx.pre);
                    }
                    Piece::El(ce) => self.walk(ce, &st, link, ctx, css_here),
                }
            }
        }
    }

    fn walk(
        &mut self,
        el: ElementRef,
        style: &Style,
        link: &Option<String>,
        ctx: &Ctx,
        css_parent: Option<&CssPair>,
    ) {
        let name = el.value().name().to_ascii_lowercase();
        if matches!(
            name.as_str(),
            "head" | "script" | "style" | "title" | "meta" | "link" | "noscript" | "template"
        ) {
            return;
        }
        if let Some(id) = el.value().attr("id") {
            self.cur.push(Span::Anchor(id.to_string()));
        }

        let mut style = style.clone();
        let mut link = link.clone();
        let mut ctx = ctx.clone();
        let css_here = match self.apply_css(el, css_parent, &mut style, &mut ctx) {
            Some(c) => c,
            None => return, // display: none
        };
        let css_here = css_here.as_ref();
        if let Some((author, base)) = css_here {
            if matches!(name.as_str(), "body" | "html") {
                if author.background_color != base.background_color {
                    if let Some(c) = author.background_color.filter(|c| c.a > 0) {
                        self.doc.background = Some((c.r, c.g, c.b));
                    }
                }
                if author.writing_mode.is_vertical() {
                    self.doc.writing_mode = author.writing_mode;
                    self.doc.orientation = author.text_orientation;
                }
            }
        }
        if style_attr_has(&el, "font-weight:bold") {
            style.bold = true;
        }
        if style_attr_has(&el, "font-style:italic") {
            style.italic = true;
        }
        if style_attr_has(&el, "text-align:center") || el.value().attr("align") == Some("center") {
            ctx.align = Align::Center;
        } else if style_attr_has(&el, "text-align:right") {
            ctx.align = Align::End;
        }
        if let Some(cls) = el.value().attr("class") {
            let c = cls.to_ascii_lowercase();
            if c.split_whitespace()
                .any(|w| w == "center" || w == "centered" || w == "text-center")
            {
                ctx.align = Align::Center;
            }
        }

        match name.as_str() {
            "br" => {
                if ctx.pre {
                    self.push_text("\n", &style, &link, true);
                } else {
                    self.cur.push(Span::Break);
                }
                return;
            }
            "img" | "image" => {
                // SVG `<image xlink:href>` carries a namespaced attribute; match on the local name.
                let src = el
                    .value()
                    .attr("src")
                    .or_else(|| {
                        el.value()
                            .attrs()
                            .find(|(k, _)| *k == "href" || *k == "src")
                            .map(|(_, v)| v)
                    })
                    .unwrap_or("");
                let alt = el.value().attr("alt").unwrap_or("").to_string();
                if !src.is_empty() {
                    let path = if is_external(src) {
                        src.to_string()
                    } else {
                        resolve_href(self.base_dir, src).0
                    };
                    self.cur.push(Span::Image {
                        path,
                        alt,
                        float: ctx.float,
                    });
                }
                return;
            }
            "ruby" => {
                self.walk_ruby(el, &style, &link, &ctx, css_here);
                return;
            }
            "hr" => {
                self.flush(&ctx);
                self.doc.blocks.push(Block {
                    kind: BlockKind::Rule,
                    spans: Vec::new(),
                    align: Align::Center,
                    indent: 0,
                    marker: None,
                    css: BlockCss::default(),
                });
                return;
            }
            _ => {}
        }

        let mut block = false;
        match name.as_str() {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                block = true;
                ctx.kind = BlockKind::Heading(name.as_bytes()[1] - b'0');
            }
            "blockquote" => {
                block = true;
                ctx.kind = BlockKind::Quote;
                ctx.indent += 1;
            }
            "pre" => {
                block = true;
                ctx.kind = BlockKind::Code;
                ctx.pre = true;
            }
            "center" => {
                block = true;
                ctx.align = Align::Center;
            }
            "p" | "div" | "section" | "article" | "header" | "footer" | "aside" | "nav"
            | "main" | "figure" | "figcaption" | "address" | "dd" | "dt" | "dl" | "body"
            | "html" | "tr" | "caption" | "details" | "summary" | "form" | "fieldset" | "table"
            | "thead" | "tbody" | "tfoot" => block = true,
            "ul" | "ol" => {
                self.flush(&ctx);
                self.lists.push((name == "ol", 0));
                ctx.indent += 1;
                for c in el.children() {
                    if let Some(ce) = ElementRef::wrap(c) {
                        self.walk(ce, &style, &link, &ctx, css_here);
                    } else if let Node::Text(t) = c.value() {
                        if !t.trim().is_empty() {
                            self.push_text(t, &style, &link, false);
                        }
                    }
                }
                self.flush(&ctx);
                self.lists.pop();
                return;
            }
            "li" => {
                block = true;
                let marker = match self.lists.last_mut() {
                    Some((true, n)) => {
                        *n += 1;
                        format!("{}.", n)
                    }
                    _ => "•".to_string(),
                };
                self.flush(&ctx);
                self.pending_marker = Some(marker);
            }
            "td" | "th" => {
                if !self.cur.is_empty() {
                    self.push_text(" │ ", &style, &link, true);
                }
                if name == "th" {
                    style.bold = true;
                }
            }
            "b" | "strong" => style.bold = true,
            "i" | "em" | "cite" | "var" | "dfn" => style.italic = true,
            "u" | "ins" => style.underline = true,
            "s" | "strike" | "del" => style.strike = true,
            "code" | "tt" | "kbd" | "samp" => style.code = true,
            "sup" => style.sup = true,
            "sub" => style.sub = true,
            "small" => style.small = true,
            "a" => {
                if let Some(h) = el.value().attr("href") {
                    link = Some(self.resolve_link(h));
                }
            }
            _ => {}
        }

        if block {
            self.flush(&ctx);
            if let Some((author, base)) = css_here {
                ctx.css = block_css(
                    author,
                    base,
                    name == "p",
                    ctx.css.text_indent,
                    ctx.css.margin_left,
                );
                if let Some(st) = self.styling {
                    ctx.css.drop_cap = drop_cap_lines(st, el, author);
                }
            }
        }
        for c in el.children() {
            match c.value() {
                Node::Element(_) => {
                    if let Some(ce) = ElementRef::wrap(c) {
                        self.walk(ce, &style, &link, &ctx, css_here);
                    }
                }
                Node::Text(t) => {
                    let text: &str = t;
                    self.push_text(text, &style, &link, ctx.pre);
                }
                _ => {}
            }
        }
        if block {
            self.flush(&ctx);
            if name == "li" {
                self.pending_marker = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_structure() {
        let d = parse_html(
            "<html><body><h1 id='t'>Title</h1><p>Hello <b>big</b>\n  <i>world</i> <a href='b.xhtml#x'>l</a></p><ul><li>one</li><li>two</li></ul></body></html>",
            "OEBPS/a.xhtml",
        );
        assert_eq!(d.blocks.len(), 4);
        assert_eq!(d.blocks[0].kind, BlockKind::Heading(1));
        assert!(matches!(&d.blocks[0].spans[0], Span::Anchor(a) if a == "t"));
        assert_eq!(d.blocks[1].plain_text(), "Hello big world l");
        assert_eq!(d.blocks[2].marker.as_deref(), Some("•"));
        let md = d.to_markdown();
        assert!(md.contains("# Title"));
        assert!(md.contains("**big**"));
        assert!(md.contains("[l](OEBPS/b.xhtml#x)"));
    }
}

#[cfg(test)]
mod svg_tests {
    use super::*;

    #[test]
    fn svg_cover_image() {
        let d = parse_html(
            "<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><div><svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" viewBox=\"0 0 10 10\"><image width=\"10\" height=\"10\" xlink:href=\"../Images/cover.jpg\"/></svg></div></body></html>",
            "OEBPS/Text/cover.xhtml",
        );
        assert!(
            matches!(&d.blocks[0].spans[0], Span::Image { path, .. } if path == "OEBPS/Images/cover.jpg"),
            "{:?}",
            d.blocks
        );
    }
}

#[cfg(test)]
mod css_tests {
    use super::*;

    fn styled(body: &str, css: &str) -> Doc {
        let html = format!("<html><head></head><body>{body}</body></html>");
        let st = Styling::new(&[css.to_string()]);
        parse_html_styled(&html, "OEBPS/a.xhtml", Some(&st))
    }

    fn styles(d: &Doc) -> Vec<(String, Style)> {
        d.blocks
            .iter()
            .flat_map(|b| b.spans.iter())
            .filter_map(|s| match s {
                Span::Text { text, style, .. } => Some((text.clone(), style.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_class_can_make_text_italic_or_bold() {
        let d = styled(
            r#"<p>plain <span class="i">slanted</span> <span class="b">heavy</span></p>"#,
            ".i { font-style: italic } .b { font-weight: bold }",
        );
        let s = styles(&d);
        let get = |t: &str| s.iter().find(|(x, _)| x.contains(t)).unwrap().1.clone();
        assert!(get("slanted").italic && !get("slanted").bold);
        assert!(get("heavy").bold && !get("heavy").italic);
        assert!(!get("plain").bold && !get("plain").italic);
    }

    #[test]
    fn without_a_stylesheet_nothing_changes() {
        let body = r#"<h1>T</h1><p>a <i>b</i> <span class="x">c</span></p>"#;
        let plain = parse_html(&format!("<html><body>{body}</body></html>"), "a.xhtml");
        let none = parse_html_styled(
            &format!("<html><body>{body}</body></html>"),
            "a.xhtml",
            Some(&Styling::new(&[])),
        );
        assert_eq!(plain.blocks, none.blocks);
    }

    #[test]
    fn tags_and_headings_are_not_marked_twice() {
        // the defaults already make <b>, <em> and <h1> bold/italic; only the structure carries that
        let d = styled(
            "<h1>Title</h1><p><b>b</b> <em>e</em></p>",
            "p { margin: 0 }",
        );
        let plain = parse_html(
            "<html><body><h1>Title</h1><p><b>b</b> <em>e</em></p></body></html>",
            "a.xhtml",
        );
        assert_eq!(d.blocks, plain.blocks);
        assert!(
            !d.to_markdown().contains("**Title**"),
            "{}",
            d.to_markdown()
        );
    }

    #[test]
    fn a_heading_made_bold_by_css_stays_clean_in_exports() {
        let d = styled("<h2>Chapter</h2>", "h2 { font-weight: bold }");
        assert_eq!(d.to_markdown().trim(), "## Chapter");
    }

    #[test]
    fn inheritance_reaches_the_children() {
        let d = styled(
            r#"<div class="q"><p>one <span>two</span></p></div>"#,
            ".q { font-style: italic }",
        );
        assert!(styles(&d).iter().all(|(_, s)| s.italic), "{:?}", styles(&d));
    }

    #[test]
    fn display_none_hides_an_element_and_its_children() {
        let d = styled(
            r#"<p>shown</p><div class="gone"><p>secret <b>words</b></p></div><p>also shown</p>"#,
            ".gone { display: none }",
        );
        let text = d.to_text();
        assert!(text.contains("shown") && text.contains("also shown"));
        assert!(
            !text.contains("secret") && !text.contains("words"),
            "{text}"
        );
    }

    #[test]
    fn text_alignment_from_css() {
        let d = styled(
            r#"<p class="c">centred</p><p class="r">right</p><p class="j">justified</p><p>none</p>"#,
            ".c { text-align: center } .r { text-align: right } .j { text-align: justify }",
        );
        let al: Vec<Align> = d.blocks.iter().map(|b| b.align).collect();
        assert_eq!(
            al,
            vec![Align::Center, Align::End, Align::Start, Align::Start]
        );
    }

    #[test]
    fn decorations_small_caps_and_monospace() {
        let d = styled(
            r#"<p><span class="u">under</span> <span class="s">strike</span> <span class="sc">caps</span> <span class="m">mono</span></p>"#,
            ".u { text-decoration: underline } .s { text-decoration: line-through } \
             .sc { font-variant: small-caps } .m { font-family: monospace }",
        );
        let s = styles(&d);
        let get = |t: &str| s.iter().find(|(x, _)| x.contains(t)).unwrap().1.clone();
        assert!(get("under").underline);
        assert!(get("strike").strike);
        assert!(get("caps").small_caps);
        assert!(get("mono").code);
    }

    #[test]
    fn book_fonts_and_sizes_reach_the_spans() {
        let d = styled(
            r#"<p>plain <span class="f">fancy</span> <span class="g">sans</span> <span class="big">big</span> <span class="x">weird</span></p>"#,
            ".f { font-family: 'EB Garamond', serif } .g { font-family: sans-serif } \
             .big { font-size: 1.5em } .x { font-family: \"a<b>\" }",
        );
        let s = styles(&d);
        let get = |t: &str| s.iter().find(|(x, _)| x.contains(t)).unwrap().1.clone();
        assert_eq!(get("fancy").font.as_deref(), Some("EB Garamond"));
        assert_eq!(get("sans").font.as_deref(), Some("Sans"));
        assert_eq!(get("big").size_pct, Some(150));
        assert_eq!(
            get("weird").font,
            None,
            "names that could break markup are dropped"
        );
        assert_eq!(get("plain").font, None);
        assert_eq!(get("plain").size_pct, None);
    }

    #[test]
    fn small_caps_is_its_own_flag() {
        let d = styled(
            r#"<p><span class="sc">caps</span> <small>small</small></p>"#,
            ".sc { font-variant: small-caps }",
        );
        let s = styles(&d);
        let get = |t: &str| s.iter().find(|(x, _)| x.contains(t)).unwrap().1.clone();
        assert!(get("caps").small_caps && !get("caps").small);
        assert!(get("small").small && !get("small").small_caps);
    }

    #[test]
    fn important_specificity_and_inline_styles_follow_css_rules() {
        let d = styled(
            r#"<p class="x" id="y">a</p><p style="font-weight: bold">b</p><p class="x" style="font-style: normal">c</p>"#,
            "p.x { font-style: italic } #y { font-style: normal !important }",
        );
        let s = styles(&d);
        let get = |t: &str| s.iter().find(|(x, _)| x.trim() == t).unwrap().1.clone();
        assert!(!get("a").italic, "id rule with !important wins");
        assert!(get("b").bold, "inline style");
        assert!(!get("c").italic, "inline style beats the class");
    }

    #[test]
    fn broken_css_is_survivable() {
        let d = styled(
            "<p>text</p>",
            "p { color: ; } } { font-style italic ::: @media",
        );
        assert_eq!(d.blocks.len(), 1);
        assert_eq!(d.to_text().trim(), "text");
    }

    #[test]
    fn a_big_chapter_with_many_rules_is_fast_enough() {
        let mut css = String::new();
        for i in 0..200 {
            css.push_str(&format!(
                ".c{i} {{ font-style: italic }} p.k{i} > span {{ font-weight: bold }}\n"
            ));
        }
        let mut body = String::new();
        for i in 0..1500 {
            body.push_str(&format!(
                r#"<p class="k{}">para <span class="c{}">x</span> text</p>"#,
                i % 200,
                i % 200
            ));
        }
        let t = std::time::Instant::now();
        let d = styled(&body, &css);
        let took = t.elapsed();
        assert_eq!(d.blocks.len(), 1500);
        let budget = if cfg!(debug_assertions) { 15_000 } else { 600 };
        assert!(took.as_millis() < budget, "took {took:?}");
    }
}

#[cfg(test)]
mod block_css_tests {
    use super::*;

    fn styled(body: &str, css: &str) -> Doc {
        let html = format!("<html><head></head><body>{body}</body></html>");
        let st = Styling::new(&[css.to_string()]);
        parse_html_styled(&html, "OEBPS/a.xhtml", Some(&st))
    }

    fn css_of(d: &Doc, i: usize) -> BlockCss {
        d.blocks[i].css
    }

    #[test]
    fn nothing_changes_without_styling() {
        let plain = parse_html("<html><body><p>a</p><p>b</p></body></html>", "a.xhtml");
        assert!(plain.blocks.iter().all(|b| b.css == BlockCss::default()));
    }

    #[test]
    fn first_line_indent_is_read_in_em_and_inherited() {
        let d = styled(
            "<p>one</p><div><p>two</p></div>",
            "p { text-indent: 1.5em }",
        );
        assert!(
            (css_of(&d, 0).text_indent - 1.5).abs() < 0.01,
            "{:?}",
            css_of(&d, 0)
        );
        assert!((css_of(&d, 1).text_indent - 1.5).abs() < 0.01);
        // text-indent on a container reaches the paragraphs inside it
        let d = styled("<div class=\"c\"><p>x</p></div>", ".c { text-indent: 2em }");
        assert!(
            (css_of(&d, 0).text_indent - 2.0).abs() < 0.01,
            "{:?}",
            css_of(&d, 0)
        );
    }

    #[test]
    fn pixels_become_em_of_the_block_font() {
        let d = styled("<p>x</p>", "p { text-indent: 24px }"); // 24px of a 16px font
        assert!(
            (css_of(&d, 0).text_indent - 1.5).abs() < 0.05,
            "{:?}",
            css_of(&d, 0)
        );
    }

    #[test]
    fn indented_paragraphs_are_packed_tight_unless_the_book_adds_a_margin() {
        let d = styled("<p>x</p>", "p { text-indent: 1em; margin: 0 }");
        assert_eq!(css_of(&d, 0).margin_top, Some(0.0));
        assert_eq!(css_of(&d, 0).margin_bottom, Some(0.0));
        let d = styled("<p>x</p>", "p { text-indent: 1em; margin: 0 0 0.8em }");
        assert_eq!(css_of(&d, 0).margin_top, Some(0.0));
        assert!((css_of(&d, 0).margin_bottom.unwrap() - 0.8).abs() < 0.01);
    }

    #[test]
    fn margins_without_indent_only_count_when_the_book_changed_them() {
        let d = styled("<p>x</p>", "span { color: red }");
        assert_eq!(css_of(&d, 0).margin_top, None);
        assert_eq!(css_of(&d, 0).margin_bottom, None);
        let d = styled("<p>x</p>", "p { margin-bottom: 2em }");
        assert_eq!(css_of(&d, 0).margin_top, None);
        assert!((css_of(&d, 0).margin_bottom.unwrap() - 2.0).abs() < 0.01);
    }

    #[test]
    fn left_margins_add_up_across_nested_blocks() {
        let d = styled(
            "<div class=\"a\"><p class=\"b\">x</p></div>",
            ".a { margin-left: 2em } .b { margin-left: 1em }",
        );
        assert!(
            (css_of(&d, 0).margin_left - 3.0).abs() < 0.05,
            "{:?}",
            css_of(&d, 0)
        );
    }

    #[test]
    fn absurd_values_are_clamped() {
        let d = styled(
            "<p>x</p>",
            "p { text-indent: 500em; margin: 900em 0 900em 900em }",
        );
        let c = css_of(&d, 0);
        assert!(c.text_indent <= 4.0, "{c:?}");
        assert!(
            c.margin_top.unwrap() <= 3.0 && c.margin_bottom.unwrap() <= 3.0,
            "{c:?}"
        );
        assert!(c.margin_left <= 8.0, "{c:?}");
        let d = styled("<p>x</p>", "p { text-indent: -3em; margin-left: -4em }");
        assert!(css_of(&d, 0).text_indent >= 0.0 && css_of(&d, 0).margin_left >= 0.0);
    }

    #[test]
    fn the_defaults_of_headings_and_quotes_are_not_mistaken_for_book_styling() {
        let d = styled(
            "<h1>T</h1><blockquote><p>q</p></blockquote>",
            "em { font-style: italic }",
        );
        assert!(
            d.blocks
                .iter()
                .all(|b| b.css.margin_top.is_none() && b.css.margin_left == 0.0),
            "{:?}",
            d.blocks.iter().map(|b| b.css).collect::<Vec<_>>()
        );
    }

    #[test]
    fn text_and_page_colours_of_the_book_are_read() {
        let d = styled(
            "<p>plain</p><p class=\"r\">red <span class=\"g\">green</span></p>",
            "body { background: #fdf6e3 } .r { color: #ff0000 } .g { color: rgb(0, 128, 0) }",
        );
        assert_eq!(d.background, Some((0xfd, 0xf6, 0xe3)));
        let color_of = |b: usize, i: usize| match &d.blocks[b].spans[i] {
            Span::Text { style, .. } => style.color,
            other => panic!("{other:?}"),
        };
        assert_eq!(color_of(0, 0), None);
        assert_eq!(color_of(1, 0), Some((255, 0, 0)));
        assert_eq!(color_of(1, 1), Some((0, 128, 0)));
    }

    #[test]
    fn no_colours_without_styling_and_transparent_is_ignored() {
        let plain = parse_html(
            "<html><body><p style=\"color:red\">a</p></body></html>",
            "a.xhtml",
        );
        assert_eq!(plain.background, None);
        let d = styled(
            "<p>x</p>",
            "body { background: transparent } p { color: rgba(1,2,3,0) }",
        );
        assert_eq!(d.background, None);
        match &d.blocks[0].spans[0] {
            Span::Text { style, .. } => assert_eq!(style.color, None),
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod float_tests {
    use super::*;

    fn styled(body: &str, css: &str) -> Doc {
        let html = format!("<html><head></head><body>{body}</body></html>");
        let st = Styling::new(&[css.to_string()]);
        parse_html_styled(&html, "OEBPS/a.xhtml", Some(&st))
    }

    fn floats(d: &Doc) -> Vec<FloatSide> {
        d.blocks
            .iter()
            .flat_map(|b| b.spans.iter())
            .filter_map(|s| match s {
                Span::Image { float, .. } => Some(*float),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_floated_image_keeps_its_side() {
        let d = styled(
            r#"<p><img class="l" src="a.png"/> text</p><p><img class="r" src="b.png"/></p><p><img src="c.png"/></p>"#,
            ".l { float: left } .r { float: right }",
        );
        assert_eq!(
            floats(&d),
            vec![FloatSide::Left, FloatSide::Right, FloatSide::None]
        );
    }

    #[test]
    fn an_image_in_a_floated_figure_floats_too() {
        let d = styled(
            r#"<figure class="f"><img src="a.png"/></figure>"#,
            ".f { float: right }",
        );
        assert_eq!(floats(&d), vec![FloatSide::Right]);
    }

    #[test]
    fn nothing_floats_without_styling() {
        let d = parse_html(r#"<p><img class="l" src="a.png"/></p>"#, "a.xhtml");
        assert_eq!(floats(&d), vec![FloatSide::None]);
    }

    #[test]
    fn first_letter_float_makes_a_drop_cap_of_matching_height() {
        let d = styled(
            "<p>Once upon a time</p><p>Second</p>",
            "p:first-of-type::first-letter { float: left; font-size: 3em }",
        );
        let cap = d.blocks[0].css.drop_cap;
        assert!((2..=6).contains(&cap), "{cap}");
        let d = styled(
            "<p class=\"a\">Once</p><p>Other</p>",
            "p.a::first-letter { float: left; font-size: 4em }",
        );
        assert_eq!(d.blocks[0].css.drop_cap, 4);
        assert_eq!(d.blocks[1].css.drop_cap, 0);
    }

    #[test]
    fn a_first_letter_without_float_is_no_drop_cap() {
        let d = styled("<p>Once</p>", "p::first-letter { font-size: 3em }");
        assert_eq!(d.blocks[0].css.drop_cap, 0);
        let d = parse_html("<p>Once</p>", "a.xhtml");
        assert_eq!(d.blocks[0].css.drop_cap, 0);
    }

    #[test]
    fn the_user_sheet_wins_over_the_book_and_can_switch_it_off() {
        let html = "<html><head></head><body><p class=\"x\">hello</p></body></html>";
        let book = vec![".x { text-align: center }".to_string()];
        let d = parse_html_styled(html, "a.xhtml", Some(&Styling::layered(&book, "", true)));
        assert_eq!(d.blocks[0].align, Align::Center);
        let d = parse_html_styled(html, "a.xhtml", Some(&Styling::layered(&book, "", false)));
        assert_eq!(d.blocks[0].align, Align::Start);
        let user = ".x { text-indent: 2em; font-style: italic !important }";
        let d = parse_html_styled(html, "a.xhtml", Some(&Styling::layered(&book, user, true)));
        assert!(d.blocks[0].css.text_indent > 1.9);
        assert!(matches!(&d.blocks[0].spans[0], Span::Text { style, .. } if style.italic));
        assert_eq!(
            d.blocks[0].align,
            Align::Center,
            "the book's rules still apply"
        );
    }
}

#[cfg(test)]
mod vertical_tests {
    use super::*;

    fn styled(body: &str, css: &str) -> Doc {
        let src = format!("<html><body>{body}</body></html>");
        parse_html_styled(&src, "c.xhtml", Some(&Styling::new(&[css.to_string()])))
    }

    fn spans(d: &Doc) -> Vec<(String, Option<String>, TextCombine)> {
        d.blocks
            .iter()
            .flat_map(|b| b.spans.iter())
            .filter_map(|s| match s {
                Span::Text { text, style, .. } => {
                    Some((text.clone(), style.ruby.clone(), style.combine))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_writing_mode_of_the_body_sets_the_chapter_and_every_block() {
        let d = styled(
            "<p>縦書き</p><p>二</p>",
            "body { writing-mode: vertical-rl; text-orientation: upright }",
        );
        assert_eq!(d.writing_mode, WritingMode::VerticalRl);
        assert_eq!(d.orientation, TextOrientation::Upright);
        assert!(d
            .blocks
            .iter()
            .all(|b| b.css.writing_mode == WritingMode::VerticalRl));
    }

    #[test]
    fn prefixed_and_legacy_spellings_work_and_horizontal_stays_the_default() {
        let d = styled("<p>x</p>", "html { -epub-writing-mode: vertical-lr }");
        assert_eq!(d.writing_mode, WritingMode::VerticalLr);
        let d = styled("<p>x</p>", "body { -webkit-writing-mode: vertical-rl }");
        assert_eq!(d.writing_mode, WritingMode::VerticalRl);
        let d = styled("<p>x</p>", "body { writing-mode: tb-rl }");
        assert_eq!(d.writing_mode, WritingMode::VerticalRl);
        let d = styled("<p>x</p>", "p { color: red }");
        assert_eq!(d.writing_mode, WritingMode::HorizontalTb);
        assert_eq!(
            parse_html("<p>x</p>", "c.xhtml").writing_mode,
            WritingMode::HorizontalTb
        );
    }

    #[test]
    fn an_inline_style_on_a_division_makes_the_chapter_vertical() {
        let d = styled(
            "<div style='writing-mode: vertical-rl'><p>縦</p></div><p>横</p>",
            "",
        );
        assert_eq!(d.writing_mode, WritingMode::VerticalRl);
    }

    #[test]
    fn horizontal_blocks_in_a_vertical_book_keep_their_own_flag() {
        let d = styled(
            "<p class='h'>x</p><p>縦</p>",
            "body { writing-mode: vertical-rl } .h { writing-mode: horizontal-tb }",
        );
        assert_eq!(d.blocks[0].css.writing_mode, WritingMode::HorizontalTb);
        assert_eq!(d.blocks[1].css.writing_mode, WritingMode::VerticalRl);
    }

    #[test]
    fn text_combine_upright_marks_the_text_of_the_element() {
        let d = styled(
            "<p>平成<span class='t'>12</span>年<span class='u'>3456</span></p>",
            ".t { text-combine-upright: all } .u { -epub-text-combine: horizontal }
             body { writing-mode: vertical-rl }",
        );
        let s = spans(&d);
        assert_eq!(s[0].2, TextCombine::None);
        assert_eq!(s[1], ("12".into(), None, TextCombine::All));
        assert_eq!(s[2].2, TextCombine::None);
        assert_eq!(s[3].2, TextCombine::All);
    }

    #[test]
    fn ruby_annotates_its_base_text() {
        let d = parse_html(
            "<p>これは<ruby>漢字<rp>(</rp><rt>かんじ</rt><rp>)</rp></ruby>です</p>",
            "c.xhtml",
        );
        let s = spans(&d);
        assert_eq!(
            s.iter().map(|x| x.0.as_str()).collect::<String>(),
            "これは漢字です",
            "annotation and fallback brackets are not part of the text"
        );
        assert_eq!(s[1].0, "漢字");
        assert_eq!(s[1].1.as_deref(), Some("かんじ"));
        assert_eq!(s[0].1, None);
        assert_eq!(s[2].1, None);
        assert_eq!(d.blocks[0].plain_text(), "これは漢字です");
    }

    #[test]
    fn ruby_with_several_bases_pairs_them_in_order() {
        let d = parse_html(
            "<p><ruby>東<rt>とう</rt>京<rt>きょう</rt></ruby>と<ruby><rb>大</rb><rb>阪</rb><rt>おお</rt><rt>さか</rt></ruby></p>",
            "c.xhtml",
        );
        let s: Vec<_> = spans(&d).into_iter().map(|x| (x.0, x.1)).collect();
        assert_eq!(
            s,
            vec![
                ("東".into(), Some("とう".into())),
                ("京".into(), Some("きょう".into())),
                ("と".into(), None),
                ("大".into(), Some("おお".into())),
                ("阪".into(), Some("さか".into())),
            ]
        );
    }

    #[test]
    fn margins_of_a_vertical_block_follow_the_block_axis() {
        let d = styled(
            "<p>縦</p>",
            "body { writing-mode: vertical-rl } p { margin-right: 2em; margin-left: 1em; margin-top: 3em }",
        );
        let c = d.blocks[0].css;
        assert_eq!(c.margin_top, Some(2.0), "the right margin starts the block");
        assert_eq!(c.margin_bottom, Some(1.0));
        assert_eq!(
            c.margin_left, 3.0,
            "the top margin is the indent of the line"
        );
    }

    #[test]
    fn a_style_sheet_can_hide_the_furigana() {
        let body = "<p>これは<ruby>漢字<rt>かんじ</rt></ruby>です</p>";
        let shown = styled(body, "p { color: red }");
        assert_eq!(spans(&shown)[1].1.as_deref(), Some("かんじ"));
        let hidden = styled(body, "rt { display: none }");
        assert_eq!(spans(&hidden)[1].1, None);
        assert_eq!(hidden.blocks[0].plain_text(), "これは漢字です");
    }

    #[test]
    fn ruby_without_a_base_group_or_with_an_empty_annotation_is_survivable() {
        let d = parse_html(
            "<p><ruby>字<rt></rt></ruby><ruby><rt>あ</rt></ruby>x</p>",
            "c.xhtml",
        );
        assert_eq!(d.blocks[0].plain_text(), "字x");
        assert_eq!(spans(&d)[0].1, None);
    }
}
