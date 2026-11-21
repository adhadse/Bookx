//! A small intermediate representation of a chapter, used by the reader,
//! the editor preview and the exporters (TXT / Markdown).

pub use bookx_text::css::{TextCombine, TextOrientation, WritingMode};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub underline: bool,
    pub strike: bool,
    pub sup: bool,
    pub sub: bool,
    pub small: bool,
    /// `font-variant: small-caps`: lower case letters are set as small capitals.
    pub small_caps: bool,
    /// Font family the book's CSS asks for (`None` = the reader's font).
    pub font: Option<String>,
    /// Size relative to the text around it in percent, when the book's CSS changes it (50 to 300).
    pub size_pct: Option<u16>,
    /// Text colour the book's CSS asks for (`None` = the reader's colour).
    pub color: Option<Rgb8>,
    /// Ruby (furigana): the annotation that is set beside this text (`<ruby>` base and `<rt>`).
    pub ruby: Option<String>,
    /// `text-combine-upright`: characters of this text are set side by side in one em of a
    /// vertical line (tate-chu-yoko).
    pub combine: TextCombine,
}

/// 8-bit RGB.
pub type Rgb8 = (u8, u8, u8);

#[derive(Debug, Clone, PartialEq)]
pub enum Span {
    Text {
        text: String,
        style: Style,
        /// Internal links are `archive/path#fragment`, external ones are kept verbatim.
        link: Option<String>,
    },
    Image {
        path: String,
        alt: String,
        /// The book floats the image to a side and lets the text wrap around it.
        float: FloatSide,
    },
    Break,
    Anchor(String),
}

/// CSS `float` as far as the reader supports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FloatSide {
    #[default]
    None,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Paragraph,
    Heading(u8),
    Quote,
    Code,
    Rule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
}

/// What the book's own CSS says about a block (only filled in when the styling is applied).
/// All lengths are in `em` of the block's font; `None` means "the reader's default".
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BlockCss {
    /// Indent of the first line (0 = none).
    pub text_indent: f32,
    pub margin_top: Option<f32>,
    pub margin_bottom: Option<f32>,
    /// Extra left margin (0 = none).
    pub margin_left: f32,
    /// The book asks for a drop cap (`::first-letter { float: left }`) this many lines tall
    /// (0 = none).
    pub drop_cap: u8,
    /// `writing-mode` of the block (inherited).
    pub writing_mode: WritingMode,
    /// `text-orientation` of the block (inherited).
    pub orientation: TextOrientation,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub kind: BlockKind,
    pub spans: Vec<Span>,
    pub align: Align,
    pub indent: u8,
    pub marker: Option<String>,
    pub css: BlockCss,
}

#[derive(Debug, Clone, Default)]
pub struct Doc {
    pub blocks: Vec<Block>,
    /// Page colour the book's CSS puts on `body` or `html`.
    pub background: Option<Rgb8>,
    /// How the chapter's lines run: the `writing-mode` of its `html`/`body` (or of its first
    /// block). A chapter is laid out in one mode.
    pub writing_mode: WritingMode,
    /// `text-orientation` of the chapter, see [`Doc::writing_mode`].
    pub orientation: TextOrientation,
}

/// Whether the package document asks for fixed layout (`rendition:layout` = `pre-paginated`, or
/// the Kindle style `fixed-layout` = `true`): every spine item is one page whose picture or
/// text is placed by the book, not reflowed.
pub fn is_fixed_layout(opf_xml: &str) -> bool {
    let Ok(doc) = roxmltree::Document::parse(opf_xml) else {
        return false;
    };
    doc.descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "meta")
        .any(|m| {
            let text = m.text().unwrap_or("").trim();
            (m.attribute("property") == Some("rendition:layout") && text == "pre-paginated")
                || (m.attribute("name") == Some("fixed-layout")
                    && m.attribute("content") == Some("true"))
        })
}

impl Block {
    pub fn plain_text(&self) -> String {
        let mut s = String::new();
        for sp in &self.spans {
            match sp {
                Span::Text { text, .. } => s.push_str(text),
                Span::Break => s.push('\n'),
                Span::Image { alt, .. } => {
                    if !alt.is_empty() {
                        s.push_str(&format!("[{alt}]"))
                    }
                }
                Span::Anchor(_) => {}
            }
        }
        s
    }
}

impl Doc {
    pub fn word_count(&self) -> usize {
        self.blocks
            .iter()
            .map(|b| b.plain_text().split_whitespace().count())
            .sum()
    }

    /// The chapter's text **as the reader numbers its characters**: the coordinate system of every
    /// highlight offset (`Highlight::start`/`end`). Paragraphs (and the pieces of a paragraph
    /// that an image splits) are joined by one `\n`; a list marker and its tab belong to the item;
    /// headings keep their case; rules, pictures and empty blocks have no text; soft hyphens do
    /// not count. [`Doc::to_text`] is a *different* text (blank lines, upper-cased headings,
    /// indents, `* * *`) meant for export: never use its offsets for highlights.
    /// `crate::ui::canvas` builds the same text from the laid-out chapter (a scenario checks it).
    pub fn reader_text(&self) -> String {
        use bookx_text::paragraph::{canonical_chars, sanitize};
        let mut pieces: Vec<String> = Vec::new();
        for b in &self.blocks {
            if b.kind == BlockKind::Rule {
                continue;
            }
            let mut cur = String::new();
            if let Some(m) = &b.marker {
                cur.push_str(m);
                cur.push('\t');
            }
            for sp in &b.spans {
                match sp {
                    Span::Text { text, .. } => cur.push_str(&sanitize(text)),
                    Span::Break => cur.push('\n'),
                    Span::Image { .. } => {
                        if !cur.trim().is_empty() {
                            pieces.push(std::mem::take(&mut cur));
                        } else {
                            cur.clear();
                        }
                    }
                    Span::Anchor(_) => {}
                }
            }
            if !cur.trim().is_empty() {
                pieces.push(cur);
            }
        }
        let mut out = String::new();
        for (i, p) in pieces.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.extend(canonical_chars(p).map(|(_, c)| c));
        }
        out
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for b in &self.blocks {
            match b.kind {
                BlockKind::Rule => out.push_str("* * *\n\n"),
                BlockKind::Heading(_) => {
                    out.push_str(&b.plain_text().to_uppercase());
                    out.push_str("\n\n");
                }
                _ => {
                    out.push_str(&"  ".repeat(b.indent as usize));
                    if let Some(m) = &b.marker {
                        out.push_str(m);
                        out.push(' ');
                    }
                    out.push_str(&b.plain_text());
                    out.push_str(if b.marker.is_some() { "\n" } else { "\n\n" });
                }
            }
        }
        out
    }

    pub fn to_markdown(&self) -> String {
        /// Backslash-escapes what Markdown would read as markup, so the text reads back unchanged.
        fn esc(t: &str) -> String {
            let mut o = String::with_capacity(t.len() + 4);
            for c in t.chars() {
                if matches!(
                    c,
                    '\\' | '*' | '_' | '`' | '[' | ']' | '<' | '>' | '~' | '|'
                ) {
                    o.push('\\');
                }
                o.push(c);
            }
            o
        }
        /// The first characters of a line that would start a heading, list or rule.
        fn esc_line_start(line: &str) -> String {
            let t = line.trim_start();
            let numbered = t.split_once(['.', ')']).is_some_and(|(n, r)| {
                !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) && r.starts_with(' ')
            });
            if t.starts_with('#')
                || t.starts_with("+ ")
                || t.starts_with("- ")
                || t == "-"
                || numbered
            {
                format!("\\{line}")
            } else {
                line.to_string()
            }
        }
        let mut out = String::new();
        let mut in_list = false;
        for b in &self.blocks {
            let is_item = b.marker.is_some();
            if in_list && !is_item {
                out.push('\n');
            }
            in_list = is_item;
            match b.kind {
                BlockKind::Rule => out.push_str("---\n\n"),
                BlockKind::Heading(n) => {
                    out.push_str(&"#".repeat(n.clamp(1, 6) as usize));
                    out.push(' ');
                    out.push_str(&esc(b.plain_text().trim()));
                    out.push_str("\n\n");
                }
                BlockKind::Code => {
                    out.push_str("```\n");
                    out.push_str(b.plain_text().trim_end());
                    out.push_str("\n```\n\n");
                }
                _ => {
                    let mut line = String::new();
                    for sp in &b.spans {
                        match sp {
                            Span::Text { text, style, link } => {
                                let mut t = text.clone();
                                if t.trim().is_empty() {
                                    line.push_str(&t);
                                    continue;
                                }
                                let lead = t.len() - t.trim_start().len();
                                let trail = t.len() - t.trim_end().len();
                                let core = t.trim().to_string();
                                let mut core = if style.code {
                                    format!("`{core}`")
                                } else {
                                    esc(&core)
                                };
                                if style.bold {
                                    core = format!("**{core}**");
                                }
                                if style.italic {
                                    core = format!("*{core}*");
                                }
                                if style.strike {
                                    core = format!("~~{core}~~");
                                }
                                if let Some(l) = link {
                                    core = format!("[{core}]({l})");
                                }
                                t = format!(
                                    "{}{}{}",
                                    &text[..lead],
                                    core,
                                    &text[text.len() - trail..]
                                );
                                line.push_str(&t);
                            }
                            Span::Image { path, alt, .. } => {
                                line.push_str(&format!("![{alt}]({path})"))
                            }
                            Span::Break => line.push_str("  \n"),
                            Span::Anchor(_) => {}
                        }
                    }
                    let prefix = "  ".repeat(b.indent.saturating_sub(1) as usize);
                    if let Some(m) = &b.marker {
                        out.push_str(&format!(
                            "{prefix}{} {line}\n",
                            if m == "•" { "-" } else { m }
                        ));
                    } else if b.kind == BlockKind::Quote {
                        for l in line.lines() {
                            out.push_str(&format!("> {l}\n"));
                        }
                        out.push('\n');
                    } else {
                        out.push_str(&esc_line_start(&line));
                        out.push_str("\n\n");
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod fixed_layout_tests {
    use super::is_fixed_layout;

    fn opf(meta: &str) -> String {
        format!(
            r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>t</dc:title>{meta}</metadata></package>"#
        )
    }

    #[test]
    fn pre_paginated_is_fixed_layout() {
        assert!(is_fixed_layout(&opf(
            r#"<meta property="rendition:layout">pre-paginated</meta>"#
        )));
        assert!(is_fixed_layout(&opf(
            r#"<meta property="rendition:layout"> pre-paginated </meta>"#
        )));
        assert!(is_fixed_layout(&opf(
            r#"<meta name="fixed-layout" content="true"/>"#
        )));
    }

    #[test]
    fn reflowable_and_broken_packages_are_not() {
        assert!(!is_fixed_layout(&opf(
            r#"<meta property="rendition:layout">reflowable</meta>"#
        )));
        assert!(!is_fixed_layout(&opf("")));
        assert!(!is_fixed_layout(&opf(
            r#"<meta name="fixed-layout" content="false"/>"#
        )));
        assert!(!is_fixed_layout("not xml <<<"));
        assert!(!is_fixed_layout(""));
    }
}
