//! Cascade, inheritance and computed values.

use super::sheet::{parse_declaration_text, parse_stylesheet, MediaEnv, Rule};
use super::values::*;
use scraper::ElementRef;

/// Built-in user agent stylesheet. Paragraphs have no margin or indent so that the paragraph spacing setting
/// decides; headings, lists and quotes get the usual browser look.
pub const UA_CSS: &str = r#"
html, body, div, section, article, aside, nav, header, footer, main, address, blockquote, center, dd, dl, dt, figure,
figcaption, fieldset, form, hgroup, p, pre, hr, h1, h2, h3, h4, h5, h6, ul, ol, menu, dir, details, summary, table, caption { display: block }
head, script, style, title, meta, link, template, noscript, base, area, datalist, param, source, track, rp { display: none }
li { display: list-item }
h1 { font-size: 2em; margin-top: 0.67em; margin-bottom: 0.67em }
h2 { font-size: 1.5em; margin-top: 0.83em; margin-bottom: 0.83em }
h3 { font-size: 1.17em; margin-top: 1em; margin-bottom: 1em }
h4 { margin-top: 1.33em; margin-bottom: 1.33em }
h5 { font-size: 0.83em; margin-top: 1.67em; margin-bottom: 1.67em }
h6 { font-size: 0.67em; margin-top: 2.33em; margin-bottom: 2.33em }
h1, h2, h3, h4, h5, h6, b, strong, th { font-weight: bold }
h1, h2, h3, h4, h5, h6 { page-break-after: avoid }
i, em, cite, var, dfn, address { font-style: italic }
u, ins { text-decoration: underline }
s, strike, del { text-decoration: line-through }
small { font-size: 0.83em }
big { font-size: 1.17em }
sub, sup { font-size: 0.83em }
code, kbd, samp, tt, pre { font-family: monospace }
blockquote { margin: 1em 40px }
ul, ol, menu, dir { margin-top: 1em; margin-bottom: 1em; padding-left: 40px }
dd { margin-left: 40px }
center { text-align: center }
"#;

/// Fully resolved style of an element (lengths in px).
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedStyle {
    /// Empty = use the reader's font.
    pub font_family: Vec<String>,
    pub font_size: f64,
    pub font_weight: u16,
    pub font_style: FontStyle,
    /// `None` = use the theme's text colour.
    pub color: Option<Color>,
    /// Not inherited; `None` = transparent.
    pub background_color: Option<Color>,
    pub text_align: TextAlign,
    pub text_indent: f64,
    pub line_height: LineHeight,
    pub text_decoration: TextDecoration,
    pub display: Display,
    pub margin: Edges,
    pub padding: Edges,
    pub margin_left_auto: bool,
    pub margin_right_auto: bool,
    pub page_break_before: PageBreak,
    pub page_break_after: PageBreak,
    pub hyphens: Hyphens,
    pub small_caps: bool,
    pub float: Float,
    /// Inherited.
    pub writing_mode: WritingMode,
    /// Inherited.
    pub text_orientation: TextOrientation,
    /// Inherited.
    pub text_combine: TextCombine,
    pub width: Option<Size>,
    pub max_width: Option<Size>,
    /// Font size of the root element, the base of `rem`.
    pub root_font_size: f64,
}

impl ComputedStyle {
    /// Initial values for a root font size.
    pub fn initial(base_font_size: f64) -> Self {
        ComputedStyle {
            font_family: Vec::new(),
            font_size: base_font_size,
            font_weight: 400,
            font_style: FontStyle::Normal,
            color: None,
            background_color: None,
            text_align: TextAlign::Start,
            text_indent: 0.0,
            line_height: LineHeight::Normal,
            text_decoration: TextDecoration::default(),
            display: Display::Inline,
            margin: Edges::default(),
            padding: Edges::default(),
            margin_left_auto: false,
            margin_right_auto: false,
            page_break_before: PageBreak::Auto,
            page_break_after: PageBreak::Auto,
            hyphens: Hyphens::Manual,
            small_caps: false,
            float: Float::None,
            writing_mode: WritingMode::HorizontalTb,
            text_orientation: TextOrientation::Mixed,
            text_combine: TextCombine::None,
            width: None,
            max_width: None,
            root_font_size: base_font_size,
        }
    }

    /// Line height in px for this element's font size (`Normal` = 1.2).
    pub fn line_height_px(&self) -> f64 {
        match self.line_height {
            LineHeight::Normal => self.font_size * 1.2,
            LineHeight::Number(n) => self.font_size * n,
            LineHeight::Px(p) => p,
        }
    }
}

#[derive(Default)]
struct Specified {
    font_family: Option<Val<Vec<String>>>,
    font_size: Option<Val<Len>>,
    font_weight: Option<Val<WeightSpec>>,
    font_style: Option<Val<FontStyle>>,
    color: Option<Val<Color>>,
    background_color: Option<Val<Color>>,
    text_align: Option<Val<TextAlign>>,
    text_indent: Option<Val<Len>>,
    margin: [Option<Val<Len>>; 4],
    padding: [Option<Val<Len>>; 4],
    line_height: Option<Val<LineHeightSpec>>,
    text_decoration: Option<Val<TextDecoration>>,
    display: Option<Val<Display>>,
    page_break_before: Option<Val<PageBreak>>,
    page_break_after: Option<Val<PageBreak>>,
    hyphens: Option<Val<Hyphens>>,
    small_caps: Option<Val<bool>>,
    float: Option<Val<Float>>,
    writing_mode: Option<Val<WritingMode>>,
    text_orientation: Option<Val<TextOrientation>>,
    text_combine: Option<Val<TextCombine>>,
    width: Option<Val<Option<Len>>>,
    max_width: Option<Val<Option<Len>>>,
}

fn side_idx(s: Side) -> usize {
    match s {
        Side::Top => 0,
        Side::Right => 1,
        Side::Bottom => 2,
        Side::Left => 3,
    }
}

impl Specified {
    fn apply(&mut self, d: &Decl) {
        match d.clone() {
            Decl::FontFamily(v) => self.font_family = Some(v),
            Decl::FontSize(v) => self.font_size = Some(v),
            Decl::FontWeight(v) => self.font_weight = Some(v),
            Decl::FontStyle(v) => self.font_style = Some(v),
            Decl::Color(v) => self.color = Some(v),
            Decl::BackgroundColor(v) => self.background_color = Some(v),
            Decl::TextAlign(v) => self.text_align = Some(v),
            Decl::TextIndent(v) => self.text_indent = Some(v),
            Decl::Margin(s, v) => self.margin[side_idx(s)] = Some(v),
            Decl::Padding(s, v) => self.padding[side_idx(s)] = Some(v),
            Decl::LineHeight(v) => self.line_height = Some(v),
            Decl::TextDecoration(v) => self.text_decoration = Some(v),
            Decl::Display(v) => self.display = Some(v),
            Decl::PageBreakBefore(v) => self.page_break_before = Some(v),
            Decl::PageBreakAfter(v) => self.page_break_after = Some(v),
            Decl::Hyphens(v) => self.hyphens = Some(v),
            Decl::SmallCaps(v) => self.small_caps = Some(v),
            Decl::Float(v) => self.float = Some(v),
            Decl::WritingMode(v) => self.writing_mode = Some(v),
            Decl::TextOrientation(v) => self.text_orientation = Some(v),
            Decl::TextCombine(v) => self.text_combine = Some(v),
            Decl::Width(v) => self.width = Some(v),
            Decl::MaxWidth(v) => self.max_width = Some(v),
        }
    }
}

/// Resolve a specified value against parent (for inherited / `inherit`) and initial values.
fn pick<T: Clone, U>(
    spec: &Option<Val<U>>,
    inherited: bool,
    parent: T,
    initial: T,
    set: impl Fn(&U) -> T,
) -> T {
    match spec {
        None => {
            if inherited {
                parent
            } else {
                initial
            }
        }
        Some(Val::Inherit) => parent,
        Some(Val::Initial) => initial,
        Some(Val::Unset) => {
            if inherited {
                parent
            } else {
                initial
            }
        }
        Some(Val::Set(v)) => set(v),
    }
}

/// Cascade layers, lowest priority first.
const LAYER_UA: u8 = 0;
const LAYER_AUTHOR: u8 = 1;
const LAYER_USER: u8 = 2;
const LAYER_AUTHOR_IMPORTANT: u8 = 3;
const LAYER_USER_IMPORTANT: u8 = 4;

/// Inline `style=""` attributes beat any selector of the same layer.
const INLINE_SPECIFICITY: u32 = u32::MAX;

/// A collection of stylesheets: built-in, author (the book), and user (tweaks). See the module docs for the cascade.
#[derive(Debug, Clone)]
pub struct StyleSheetSet {
    ua: Vec<Rule>,
    author: Vec<Rule>,
    user: Vec<Rule>,
    next_order: usize,
    ignore_publisher_styles: bool,
    ignore_publisher_fonts: bool,
    base_font_size: f64,
    container_width: f64,
}

impl Default for StyleSheetSet {
    fn default() -> Self {
        Self::new()
    }
}

impl StyleSheetSet {
    /// A set with only the built-in stylesheet ([`UA_CSS`]), 16px base font size and a 600px container.
    pub fn new() -> Self {
        let mut s = StyleSheetSet {
            ua: Vec::new(),
            author: Vec::new(),
            user: Vec::new(),
            next_order: 0,
            ignore_publisher_styles: false,
            ignore_publisher_fonts: false,
            base_font_size: 16.0,
            container_width: 600.0,
        };
        let (rules, next) = parse_stylesheet(UA_CSS, s.env(), 0);
        s.ua = rules;
        s.next_order = next;
        s
    }

    fn env(&self) -> MediaEnv {
        MediaEnv {
            viewport_width: self.container_width,
        }
    }

    /// Add a stylesheet from the book (several may be added, later ones win ties).
    pub fn add_author(mut self, css: &str) -> Self {
        let (rules, next) = parse_stylesheet(css, self.env(), self.next_order);
        self.author.extend(rules);
        self.next_order = next;
        self
    }

    /// Add a user stylesheet (R21 tweaks). User rules beat author rules, also over specificity, and user `!important`
    /// beats author `!important`.
    pub fn add_user(mut self, css: &str) -> Self {
        let (rules, next) = parse_stylesheet(css, self.env(), self.next_order);
        self.user.extend(rules);
        self.next_order = next;
        self
    }

    /// R22: ignore every publisher style (author sheets and inline styles).
    pub fn ignore_publisher_styles(mut self, on: bool) -> Self {
        self.ignore_publisher_styles = on;
        self
    }

    /// R22: ignore publisher `font-family` (and the `font` shorthand's family) but keep the other styles.
    pub fn ignore_publisher_fonts(mut self, on: bool) -> Self {
        self.ignore_publisher_fonts = on;
        self
    }

    /// Font size in px of the root element (default 16). Rules already added are unaffected.
    pub fn base_font_size(mut self, px: f64) -> Self {
        if px.is_finite() && px > 0.0 {
            self.base_font_size = px;
        }
        self
    }

    /// Width of the text container in px, used for percentages of margins/indent and `@media` width queries. Set it
    /// before adding stylesheets for `@media` to see it.
    pub fn container_width(mut self, px: f64) -> Self {
        if px.is_finite() && px > 0.0 {
            self.container_width = px;
        }
        self
    }

    pub fn author_rule_count(&self) -> usize {
        self.author.len()
    }

    pub fn user_rule_count(&self) -> usize {
        self.user.len()
    }

    fn matching_decls<'a>(
        &'a self,
        el: ElementRef<'_>,
        first_letter: bool,
        inline: &'a [(Decl, bool)],
    ) -> Vec<(u8, u32, usize, &'a Decl)> {
        let mut out: Vec<(u8, u32, usize, &Decl)> = Vec::new();
        let sources: [(&Vec<Rule>, u8); 3] = [
            (&self.ua, LAYER_UA),
            (&self.author, LAYER_AUTHOR),
            (&self.user, LAYER_USER),
        ];
        for (rules, layer) in sources {
            if layer == LAYER_AUTHOR && self.ignore_publisher_styles {
                continue;
            }
            for r in rules.iter() {
                if r.selector.first_letter != first_letter || !r.selector.matches(el) {
                    continue;
                }
                for (d, important) in r.decls.iter() {
                    if layer == LAYER_AUTHOR
                        && self.ignore_publisher_fonts
                        && matches!(d, Decl::FontFamily(_))
                    {
                        continue;
                    }
                    let l = match (layer, important) {
                        (LAYER_AUTHOR, true) => LAYER_AUTHOR_IMPORTANT,
                        (LAYER_USER, true) => LAYER_USER_IMPORTANT,
                        (l, _) => l,
                    };
                    out.push((l, r.selector.specificity(), r.order, d));
                }
            }
        }
        if !first_letter && !self.ignore_publisher_styles {
            for (i, (d, important)) in inline.iter().enumerate() {
                if self.ignore_publisher_fonts && matches!(d, Decl::FontFamily(_)) {
                    continue;
                }
                let l = if *important {
                    LAYER_AUTHOR_IMPORTANT
                } else {
                    LAYER_AUTHOR
                };
                out.push((l, INLINE_SPECIFICITY, usize::MAX - inline.len() + i, d));
            }
        }
        out.sort_by_key(|(layer, spec, order, _)| (*layer, *spec, *order));
        out
    }

    fn compute_specified(&self, decls: &[(u8, u32, usize, &Decl)]) -> Specified {
        let mut s = Specified::default();
        for (_, _, _, d) in decls {
            s.apply(d);
        }
        s
    }

    /// Compute the style of an element. Ancestors are cascaded as needed (use
    /// [`StyleSheetSet::compute_with_parent`] when walking the tree to avoid the repeated work).
    pub fn compute(&self, el: ElementRef<'_>) -> ComputedStyle {
        let mut chain = vec![el];
        let mut cur = el;
        while let Some(p) = cur.parent().and_then(ElementRef::wrap) {
            chain.push(p);
            cur = p;
        }
        let mut style: Option<ComputedStyle> = None;
        for e in chain.into_iter().rev() {
            style = Some(self.compute_with_parent(e, style.as_ref()));
        }
        style.unwrap_or_else(|| ComputedStyle::initial(self.base_font_size))
    }

    /// Compute the style of `el` given the already computed style of its parent element (`None` for the root).
    pub fn compute_with_parent(
        &self,
        el: ElementRef<'_>,
        parent: Option<&ComputedStyle>,
    ) -> ComputedStyle {
        let inline = el
            .value()
            .attr("style")
            .map(parse_inline)
            .unwrap_or_default();
        let decls = self.matching_decls(el, false, &inline);
        let spec = self.compute_specified(&decls);
        match parent {
            Some(p) => self.resolve(&spec, p, false),
            None => {
                let mut style =
                    self.resolve(&spec, &ComputedStyle::initial(self.base_font_size), true);
                style.root_font_size = style.font_size;
                style
            }
        }
    }

    /// The style of the `::first-letter` pseudo-element of `el` (drop caps, initials), computed on top of `el_style`.
    /// `None` when no `::first-letter` rule matches the element.
    pub fn first_letter(
        &self,
        el: ElementRef<'_>,
        el_style: &ComputedStyle,
    ) -> Option<ComputedStyle> {
        let decls = self.matching_decls(el, true, &[]);
        if decls.is_empty() {
            return None;
        }
        let spec = self.compute_specified(&decls);
        Some(self.resolve(&spec, el_style, false))
    }

    #[allow(clippy::needless_range_loop)]
    fn resolve(&self, s: &Specified, parent: &ComputedStyle, is_root: bool) -> ComputedStyle {
        let root_fs = if is_root {
            self.base_font_size
        } else {
            parent.root_font_size
        };
        // font-size first: other lengths depend on it
        let font_size = pick(
            &s.font_size,
            true,
            parent.font_size,
            self.base_font_size,
            |l| match *l {
                Len::Px(v) => v,
                Len::Em(v) => v * parent.font_size,
                Len::Rem(v) => v * root_fs,
                Len::Pct(v) => v / 100.0 * parent.font_size,
                Len::Auto => parent.font_size,
            },
        );
        let font_size = if font_size.is_finite() && font_size >= 0.0 {
            font_size
        } else {
            parent.font_size
        };
        let len = |l: &Len, percent_base: f64| -> f64 {
            match *l {
                Len::Px(v) => v,
                Len::Em(v) => v * font_size,
                Len::Rem(v) => v * root_fs,
                Len::Pct(v) => v / 100.0 * percent_base,
                Len::Auto => 0.0,
            }
        };
        let cw = self.container_width;
        let parent_weight = parent.font_weight;
        let font_weight = pick(&s.font_weight, true, parent_weight, 400, |w| match *w {
            WeightSpec::Abs(v) => v,
            WeightSpec::Bolder => match parent_weight {
                0..=349 => 400,
                350..=549 => 700,
                _ => 900,
            },
            WeightSpec::Lighter => match parent_weight {
                0..=549 => 100,
                550..=749 => 400,
                _ => 700,
            },
        });
        let mut margin = Edges::default();
        let mut padding = Edges::default();
        let mut auto = [false; 4];
        for i in 0..4 {
            let m = pick(
                &s.margin[i],
                false,
                (parent_edge(&parent.margin, i), parent_auto(parent, i)),
                (0.0, false),
                |l| (len(l, cw), *l == Len::Auto),
            );
            set_edge(&mut margin, i, m.0);
            auto[i] = m.1;
            let p = pick(
                &s.padding[i],
                false,
                parent_edge(&parent.padding, i),
                0.0,
                |l| len(l, cw),
            );
            set_edge(&mut padding, i, p);
        }
        let line_height = pick(
            &s.line_height,
            true,
            parent.line_height,
            LineHeight::Normal,
            |l| match *l {
                LineHeightSpec::Normal => LineHeight::Normal,
                LineHeightSpec::Number(n) => LineHeight::Number(n),
                LineHeightSpec::Len(l) => LineHeight::Px(len(&l, font_size)),
            },
        );
        let size = |v: &Option<Len>| -> Option<Size> {
            v.as_ref().and_then(|l| match *l {
                Len::Px(v) => Some(Size::Px(v)),
                Len::Em(v) => Some(Size::Px(v * font_size)),
                Len::Rem(v) => Some(Size::Px(v * root_fs)),
                Len::Pct(v) => Some(Size::Percent(v)),
                Len::Auto => None,
            })
        };
        ComputedStyle {
            font_family: pick(
                &s.font_family,
                true,
                parent.font_family.clone(),
                Vec::new(),
                |v| v.clone(),
            ),
            font_size,
            font_weight,
            font_style: pick(
                &s.font_style,
                true,
                parent.font_style,
                FontStyle::Normal,
                |v| *v,
            ),
            color: pick(&s.color, true, parent.color, None, |c| Some(*c)),
            background_color: pick(
                &s.background_color,
                false,
                parent.background_color,
                None,
                |c| Some(*c),
            ),
            text_align: pick(
                &s.text_align,
                true,
                parent.text_align,
                TextAlign::Start,
                |v| *v,
            ),
            text_indent: pick(&s.text_indent, true, parent.text_indent, 0.0, |l| {
                len(l, cw)
            }),
            line_height,
            text_decoration: pick(
                &s.text_decoration,
                false,
                parent.text_decoration,
                TextDecoration::default(),
                |v| *v,
            ),
            display: pick(&s.display, false, parent.display, Display::Inline, |v| *v),
            margin,
            padding,
            margin_left_auto: auto[3],
            margin_right_auto: auto[1],
            page_break_before: pick(
                &s.page_break_before,
                false,
                parent.page_break_before,
                PageBreak::Auto,
                |v| *v,
            ),
            page_break_after: pick(
                &s.page_break_after,
                false,
                parent.page_break_after,
                PageBreak::Auto,
                |v| *v,
            ),
            hyphens: pick(&s.hyphens, true, parent.hyphens, Hyphens::Manual, |v| *v),
            small_caps: pick(&s.small_caps, true, parent.small_caps, false, |v| *v),
            float: pick(&s.float, false, parent.float, Float::None, |v| *v),
            writing_mode: pick(
                &s.writing_mode,
                true,
                parent.writing_mode,
                WritingMode::HorizontalTb,
                |v| *v,
            ),
            text_orientation: pick(
                &s.text_orientation,
                true,
                parent.text_orientation,
                TextOrientation::Mixed,
                |v| *v,
            ),
            text_combine: pick(
                &s.text_combine,
                true,
                parent.text_combine,
                TextCombine::None,
                |v| *v,
            ),
            width: pick(&s.width, false, parent.width, None, |v| size(v)),
            max_width: pick(&s.max_width, false, parent.max_width, None, |v| size(v)),
            root_font_size: root_fs,
        }
    }
}

fn parent_edge(e: &Edges, i: usize) -> f64 {
    match i {
        0 => e.top,
        1 => e.right,
        2 => e.bottom,
        _ => e.left,
    }
}

fn parent_auto(p: &ComputedStyle, i: usize) -> bool {
    match i {
        1 => p.margin_right_auto,
        3 => p.margin_left_auto,
        _ => false,
    }
}

fn set_edge(e: &mut Edges, i: usize, v: f64) {
    match i {
        0 => e.top = v,
        1 => e.right = v,
        2 => e.bottom = v,
        _ => e.left = v,
    }
}

fn parse_inline(style: &str) -> Vec<(Decl, bool)> {
    // split on `;` outside parentheses and quotes
    let mut out = Vec::new();
    let (mut depth, mut quote, mut start) = (0i32, None::<char>, 0usize);
    let bytes: Vec<(usize, char)> = style.char_indices().collect();
    for (k, &(i, c)) in bytes.iter().enumerate() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '(' => depth += 1,
                ')' => depth -= 1,
                ';' if depth <= 0 => {
                    out.extend(parse_declaration_text(&style[start..i]));
                    start = i + 1;
                }
                _ => {}
            },
        }
        if k + 1 == bytes.len() {
            out.extend(parse_declaration_text(&style[start.min(style.len())..]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use scraper::{Html, Selector};

    fn page(body: &str) -> Html {
        Html::parse_document(&format!(
            "<html><head><title>t</title></head><body>{body}</body></html>"
        ))
    }

    fn el<'a>(h: &'a Html, sel: &str) -> ElementRef<'a> {
        h.select(&Selector::parse(sel).unwrap())
            .next()
            .unwrap_or_else(|| panic!("no element {sel}"))
    }

    fn st(sheets: &StyleSheetSet, h: &Html, sel: &str) -> ComputedStyle {
        sheets.compute(el(h, sel))
    }

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-6, "{a} != {b}");
    }

    #[test]
    fn ua_defaults() {
        let h = page("<h1>a</h1><p>b</p><em>c</em><strong>d</strong><ul><li>e</li></ul><blockquote>q</blockquote><u>u</u><s>s</s><code>c</code><center>x</center>");
        let s = StyleSheetSet::new();
        let h1 = st(&s, &h, "h1");
        assert_eq!(h1.display, Display::Block);
        assert_eq!(h1.font_weight, 700);
        approx(h1.font_size, 32.0);
        approx(h1.margin.top, 0.67 * 32.0);
        assert_eq!(h1.page_break_after, PageBreak::Avoid);
        let p = st(&s, &h, "p");
        assert_eq!(
            (p.display, p.margin, p.text_indent),
            (Display::Block, Edges::default(), 0.0)
        );
        assert_eq!(st(&s, &h, "em").font_style, FontStyle::Italic);
        assert_eq!(st(&s, &h, "strong").font_weight, 700);
        assert_eq!(st(&s, &h, "li").display, Display::ListItem);
        assert_eq!(st(&s, &h, "ul").padding.left, 40.0);
        let bq = st(&s, &h, "blockquote");
        assert_eq!(
            (bq.margin.left, bq.margin.right, bq.margin.top),
            (40.0, 40.0, 16.0)
        );
        assert!(st(&s, &h, "u").text_decoration.underline);
        assert!(st(&s, &h, "s").text_decoration.line_through);
        assert_eq!(
            st(&s, &h, "code").font_family,
            vec!["monospace".to_string()]
        );
        assert_eq!(st(&s, &h, "center").text_align, TextAlign::Center);
        assert_eq!(st(&s, &h, "title").display, Display::None);
        assert_eq!(st(&s, &h, "head").display, Display::None);
        assert_eq!(st(&s, &h, "html").display, Display::Block);
    }

    #[test]
    fn units_resolve_against_the_right_base() {
        let h = page("<div id=a><p id=b>x</p></div><div id=c><p id=d>x</p></div>");
        let s = StyleSheetSet::new().add_author(
            "#a { font-size: 20px } #b { font-size: 1.5em; margin-left: 2em; padding-top: 1rem; text-indent: 50%; margin-top: 10% }
             #c { font-size: 12pt } #d { font-size: 150%; width: 50%; max-width: 10em }",
        );
        let b = st(&s, &h, "#b");
        approx(b.font_size, 30.0);
        approx(b.margin.left, 60.0);
        approx(b.padding.top, 16.0);
        approx(b.text_indent, 300.0);
        approx(b.margin.top, 60.0);
        let d = st(&s, &h, "#d");
        approx(d.font_size, 24.0);
        assert_eq!(d.width, Some(Size::Percent(50.0)));
        assert_eq!(d.max_width, Some(Size::Px(240.0)));
        let s2 = StyleSheetSet::new()
            .container_width(1000.0)
            .add_author("p { text-indent: 10% }");
        approx(st(&s2, &h, "#b").text_indent, 100.0);
    }

    #[test]
    fn base_font_size_and_rem() {
        let h = page("<p>x</p>");
        let s = StyleSheetSet::new()
            .base_font_size(20.0)
            .add_author("p { font-size: 2rem; margin-left: 1rem }");
        let p = st(&s, &h, "p");
        approx(p.font_size, 40.0);
        approx(p.margin.left, 20.0);
        approx(p.root_font_size, 20.0);
        // invalid base is ignored
        let s = StyleSheetSet::new()
            .base_font_size(-3.0)
            .base_font_size(f64::NAN);
        approx(st(&s, &h, "body").font_size, 16.0);
        // html { font-size } changes rem for everything below
        let s = StyleSheetSet::new().add_author("html { font-size: 10px } p { font-size: 2rem }");
        approx(st(&s, &h, "p").font_size, 20.0);
        approx(st(&s, &h, "p").root_font_size, 10.0);
    }

    #[test]
    fn font_size_keywords() {
        let h = page("<p>x</p><div style='font-size: 10px'><span style='font-size: larger'>y</span><i style='font-size: smaller'>z</i></div>");
        let s = StyleSheetSet::new().add_author("p { font-size: x-large }");
        approx(st(&s, &h, "p").font_size, 24.0);
        approx(st(&s, &h, "span").font_size, 12.0);
        approx(st(&s, &h, "i").font_size, 10.0 / 1.2);
    }

    #[test]
    fn inheritance_rules() {
        let h = page("<div id=a><p id=b><span id=c>x</span></p></div>");
        let s = StyleSheetSet::new().add_author(
            "#a { color: #123456; font-family: Georgia, serif; font-style: italic; font-weight: 600; text-align: justify; text-indent: 2em; line-height: 1.5;
                  hyphens: auto; font-variant: small-caps; text-decoration: underline; margin: 10px; padding: 5px; display: block; float: left;
                  page-break-before: always; width: 100px; font-size: 20px }",
        );
        let c = st(&s, &h, "#c");
        assert_eq!(c.color, Some(Color::rgb(0x12, 0x34, 0x56)));
        assert_eq!(
            c.font_family,
            vec!["Georgia".to_string(), "serif".to_string()]
        );
        assert_eq!(c.font_style, FontStyle::Italic);
        assert_eq!(c.font_weight, 600);
        assert_eq!(c.text_align, TextAlign::Justify);
        approx(c.text_indent, 40.0);
        assert_eq!(c.line_height, LineHeight::Number(1.5));
        assert_eq!(c.hyphens, Hyphens::Auto);
        assert!(c.small_caps);
        approx(c.font_size, 20.0);
        // not inherited
        assert!(c.text_decoration.is_none());
        assert_eq!(c.margin, Edges::default());
        assert_eq!(c.padding, Edges::default());
        assert_eq!(c.display, Display::Inline);
        assert_eq!(c.float, Float::None);
        assert_eq!(c.page_break_before, PageBreak::Auto);
        assert_eq!(c.width, None);
        // the element itself has them
        let a = st(&s, &h, "#a");
        assert!(a.text_decoration.underline);
        assert_eq!(a.margin.top, 10.0);
        assert_eq!(a.float, Float::Left);
        assert_eq!(a.width, Some(Size::Px(100.0)));
    }

    #[test]
    fn line_height_length_resolved_at_declaring_element() {
        let h = page("<div id=a><p id=b style='font-size: 40px'>x</p></div>");
        let s = StyleSheetSet::new().add_author("#a { font-size: 10px; line-height: 2em }");
        assert_eq!(st(&s, &h, "#a").line_height, LineHeight::Px(20.0));
        assert_eq!(
            st(&s, &h, "#b").line_height,
            LineHeight::Px(20.0),
            "em line-height inherits as computed px"
        );
        let s = StyleSheetSet::new().add_author("#a { font-size: 10px; line-height: 2 }");
        let b = st(&s, &h, "#b");
        assert_eq!(b.line_height, LineHeight::Number(2.0));
        approx(b.line_height_px(), 80.0);
        let s = StyleSheetSet::new().add_author("#a { line-height: 150% }");
        assert_eq!(st(&s, &h, "#a").line_height, LineHeight::Px(24.0));
        assert_eq!(ComputedStyle::initial(10.0).line_height_px(), 12.0);
    }

    #[test]
    fn global_keywords() {
        let h = page("<div id=a><p id=b>x</p></div>");
        let s = StyleSheetSet::new().add_author(
            "#a { color: red; margin-left: 30px; font-size: 20px; text-align: center }
             #b { color: initial; margin-left: inherit; text-align: unset; font-size: initial }",
        );
        let b = st(&s, &h, "#b");
        assert_eq!(b.color, None);
        assert_eq!(b.margin.left, 30.0, "inherit on a non-inherited property");
        assert_eq!(
            b.text_align,
            TextAlign::Center,
            "unset on an inherited property inherits"
        );
        approx(b.font_size, 16.0);
        let s = StyleSheetSet::new()
            .add_author("#a { margin-left: 30px } #b { margin-left: unset; padding: inherit }");
        assert_eq!(
            st(&s, &h, "#b").margin.left,
            0.0,
            "unset on a non-inherited property resets"
        );
        let s = StyleSheetSet::new().add_author("#a { padding: 3px } #b { padding: inherit }");
        assert_eq!(
            st(&s, &h, "#b").padding,
            Edges {
                top: 3.0,
                right: 3.0,
                bottom: 3.0,
                left: 3.0
            }
        );
        let s = StyleSheetSet::new().add_author("#a { color: red } #b { color: currentColor }");
        assert_eq!(st(&s, &h, "#b").color, Some(Color::rgb(255, 0, 0)));
    }

    #[test]
    fn weight_bolder_lighter() {
        let h = page("<div id=a><p id=b><span id=c>x</span></p></div>");
        let s = StyleSheetSet::new().add_author(
            "#a { font-weight: 300 } #b { font-weight: bolder } #c { font-weight: bolder }",
        );
        assert_eq!(st(&s, &h, "#b").font_weight, 400);
        assert_eq!(st(&s, &h, "#c").font_weight, 700);
        let s = StyleSheetSet::new().add_author(
            "#a { font-weight: 800 } #b { font-weight: lighter } #c { font-weight: lighter }",
        );
        assert_eq!(st(&s, &h, "#b").font_weight, 700);
        assert_eq!(st(&s, &h, "#c").font_weight, 400);
        let s = StyleSheetSet::new().add_author(
            "#a { font-weight: 500 } #b { font-weight: bolder } #c { font-weight: bolder }",
        );
        assert_eq!(st(&s, &h, "#b").font_weight, 700);
        assert_eq!(st(&s, &h, "#c").font_weight, 900);
        let s =
            StyleSheetSet::new().add_author("#a { font-weight: 200 } #b { font-weight: lighter }");
        assert_eq!(st(&s, &h, "#b").font_weight, 100);
    }

    #[test]
    fn specificity_and_source_order() {
        let h = page("<div id=main class=c><p class='a b' id=x>t</p></div>");
        let s = StyleSheetSet::new().add_author(
            "p { color: red } .a { color: green } #x { color: blue } p.a.b { color: black }",
        );
        assert_eq!(
            st(&s, &h, "#x").color,
            Some(Color::rgb(0, 0, 255)),
            "id wins"
        );
        let s = StyleSheetSet::new()
            .add_author("p { color: red } .a { color: green } p.a.b { color: black }");
        assert_eq!(
            st(&s, &h, "#x").color,
            Some(Color::rgb(0, 0, 0)),
            "more classes win"
        );
        let s = StyleSheetSet::new().add_author(".a { color: green } p { color: red }");
        assert_eq!(
            st(&s, &h, "#x").color,
            Some(Color::rgb(0, 128, 0)),
            "class beats type regardless of order"
        );
        let s = StyleSheetSet::new().add_author(".a { color: green } .b { color: red }");
        assert_eq!(
            st(&s, &h, "#x").color,
            Some(Color::rgb(255, 0, 0)),
            "later wins at equal specificity"
        );
        let s = StyleSheetSet::new()
            .add_author(".b { color: red }")
            .add_author(".a { color: green }");
        assert_eq!(
            st(&s, &h, "#x").color,
            Some(Color::rgb(0, 128, 0)),
            "later stylesheet wins"
        );
        assert_eq!(s.author_rule_count(), 2);
        // a more specific rule on the ancestor does not beat a type rule on the element for inherited props
        let s = StyleSheetSet::new().add_author("#main { color: blue } p { color: red }");
        assert_eq!(st(&s, &h, "#x").color, Some(Color::rgb(255, 0, 0)));
    }

    #[test]
    fn important_and_inline_style() {
        let h = page("<p id=x class=a style='color: green; margin-left: 5px'>t</p>");
        let s = StyleSheetSet::new().add_author("#x { color: red; margin-left: 1px !important }");
        let x = st(&s, &h, "#x");
        assert_eq!(
            x.color,
            Some(Color::rgb(0, 128, 0)),
            "inline beats id selector"
        );
        assert_eq!(
            x.margin.left, 1.0,
            "!important in a sheet beats normal inline"
        );
        let s = StyleSheetSet::new().add_author("#x { color: red !important }");
        assert_eq!(st(&s, &h, "#x").color, Some(Color::rgb(255, 0, 0)));
        let h2 = page("<p id=x style='color: green !important'>t</p>");
        let s = StyleSheetSet::new().add_author("#x { color: red !important }");
        assert_eq!(
            st(&s, &h2, "#x").color,
            Some(Color::rgb(0, 128, 0)),
            "important inline beats important sheet"
        );
        // semicolons inside url() / strings in inline styles
        let h3 = page("<p id=x style=\"font-family: 'a;b'; color: rgb(1,2,3);\">t</p>");
        let x = st(&StyleSheetSet::new(), &h3, "#x");
        assert_eq!(x.font_family, vec!["a;b".to_string()]);
        assert_eq!(x.color, Some(Color::rgb(1, 2, 3)));
        // broken inline style is harmless
        let h4 = page("<p id=x style='color: ; : ; ;;; font-size'>t</p>");
        assert_eq!(st(&StyleSheetSet::new(), &h4, "#x").color, None);
    }

    #[test]
    fn user_layer_beats_author_and_important_rules() {
        let h = page("<p id=x class=a>t</p>");
        let author = "#x.a { color: red; font-size: 10px !important; text-align: left }";
        let user = "p { color: blue; font-size: 20px; text-align: right !important }";
        let s = StyleSheetSet::new().add_author(author).add_user(user);
        let x = st(&s, &h, "#x");
        assert_eq!(
            x.color,
            Some(Color::rgb(0, 0, 255)),
            "user normal beats author normal despite specificity"
        );
        approx(x.font_size, 10.0);
        assert_eq!(
            x.text_align,
            TextAlign::Right,
            "user important beats author normal"
        );
        let s = StyleSheetSet::new()
            .add_author("p { color: red !important }")
            .add_user("p { color: blue !important }");
        assert_eq!(
            st(&s, &h, "#x").color,
            Some(Color::rgb(0, 0, 255)),
            "user important beats author important"
        );
        assert_eq!(s.user_rule_count(), 1);
        // user rules beat inline style
        let h = page("<p id=x style='color: green'>t</p>");
        let s = StyleSheetSet::new().add_user("p { color: blue }");
        assert_eq!(st(&s, &h, "#x").color, Some(Color::rgb(0, 0, 255)));
        // author overrides the built-in sheet
        let s = StyleSheetSet::new().add_author("p { display: inline }");
        assert_eq!(st(&s, &h, "#x").display, Display::Inline);
    }

    #[test]
    fn ignore_publisher_styles_and_fonts() {
        let h = page("<p id=x style='color: green; font-family: Fancy'>t</p>");
        let author = "p { color: red; font-family: Garamond; font-size: 30px; text-align: center }";
        let user = "p { font-family: Mono }";
        let base = StyleSheetSet::new().add_author(author).add_user(user);
        // with fonts ignored, publisher font-family is dropped but the rest stays; user font remains
        let s = base.clone().ignore_publisher_fonts(true);
        let x = st(&s, &h, "#x");
        assert_eq!(x.font_family, vec!["Mono".to_string()]);
        approx(x.font_size, 30.0);
        assert_eq!(
            x.color,
            Some(Color::rgb(0, 128, 0)),
            "inline colour still applies"
        );
        let s2 = StyleSheetSet::new()
            .add_author(author)
            .ignore_publisher_fonts(true);
        assert!(
            st(&s2, &h, "#x").font_family.is_empty(),
            "no user font: reader default"
        );
        // all publisher styles ignored
        let s = base.clone().ignore_publisher_styles(true);
        let x = st(&s, &h, "#x");
        assert_eq!(x.font_family, vec!["Mono".to_string()]);
        approx(x.font_size, 16.0);
        assert_eq!(x.color, None);
        assert_eq!(x.text_align, TextAlign::Start);
        assert_eq!(x.display, Display::Block, "built-in defaults remain");
        // toggling back restores
        let s = base
            .ignore_publisher_styles(true)
            .ignore_publisher_styles(false);
        approx(st(&s, &h, "#x").font_size, 30.0);
        // the font shorthand's family is also dropped
        let s = StyleSheetSet::new()
            .add_author("p { font: italic 20px Garamond }")
            .ignore_publisher_fonts(true);
        let x = st(&s, &h, "#x");
        assert!(x.font_family.is_empty());
    }

    #[test]
    fn selectors_in_cascade() {
        let h = page("<div class=box><p>first</p><p>second</p><p class=x>third</p></div><ul><li>a</li><li>b</li></ul><a href=u>l</a>");
        let s = StyleSheetSet::new().add_author(
            "p:first-child { text-indent: 0 } p + p { text-indent: 1em } .box > p.x { color: red } div p:last-child { font-weight: bold }
             li:nth-child(2) { margin-left: 7px } a[href] { text-decoration: underline } a[href='nope'] { color: red }",
        );
        let ps: Vec<_> = h.select(&Selector::parse("p").unwrap()).collect();
        approx(s.compute(ps[0]).text_indent, 0.0);
        approx(s.compute(ps[1]).text_indent, 16.0);
        assert_eq!(s.compute(ps[2]).color, Some(Color::rgb(255, 0, 0)));
        assert_eq!(s.compute(ps[2]).font_weight, 700);
        assert_eq!(s.compute(ps[1]).font_weight, 400);
        let lis: Vec<_> = h.select(&Selector::parse("li").unwrap()).collect();
        assert_eq!(s.compute(lis[0]).margin.left, 0.0);
        assert_eq!(s.compute(lis[1]).margin.left, 7.0);
        let a = st(&s, &h, "a");
        assert!(a.text_decoration.underline);
        assert_eq!(a.color, None);
    }

    #[test]
    fn margin_auto_flags_and_negative_margins() {
        let h = page("<img id=a><img id=b><img id=c>");
        let s = StyleSheetSet::new().add_author("#a { margin: 0 auto; display: block; max-width: 100% } #b { margin-left: auto } #c { margin: -4px 0 -4px 10px }");
        let a = st(&s, &h, "#a");
        assert!(a.margin_left_auto && a.margin_right_auto);
        assert_eq!((a.margin.left, a.margin.right), (0.0, 0.0));
        assert_eq!(a.max_width, Some(Size::Percent(100.0)));
        let b = st(&s, &h, "#b");
        assert!(b.margin_left_auto && !b.margin_right_auto);
        let c = st(&s, &h, "#c");
        assert_eq!(
            (c.margin.top, c.margin.bottom, c.margin.left),
            (-4.0, -4.0, 10.0)
        );
        assert!(!c.margin_left_auto);
    }

    #[test]
    fn font_shorthand_cascades() {
        let h = page("<p id=x>t</p>");
        let s = StyleSheetSet::new()
            .add_author("p { font: italic small-caps 700 20px/1.5 'EB Garamond', serif }");
        let x = st(&s, &h, "#x");
        assert_eq!(x.font_style, FontStyle::Italic);
        assert!(x.small_caps);
        assert_eq!(x.font_weight, 700);
        approx(x.font_size, 20.0);
        assert_eq!(x.line_height, LineHeight::Number(1.5));
        assert_eq!(
            x.font_family,
            vec!["EB Garamond".to_string(), "serif".to_string()]
        );
        // the shorthand resets what it does not mention
        let s = StyleSheetSet::new()
            .add_author("p { font-weight: bold; font-style: italic } p { font: 12px serif }");
        let x = st(&s, &h, "#x");
        assert_eq!((x.font_weight, x.font_style), (400, FontStyle::Normal));
    }

    #[test]
    fn first_letter_pseudo_element() {
        let h = page("<p id=x class=intro>Once upon</p><p id=y>plain</p>");
        let s = StyleSheetSet::new().add_author("p.intro::first-letter { font-size: 3em; float: left; color: red; font-weight: bold } p { color: blue; font-size: 10px }");
        let x = el(&h, "#x");
        let xs = s.compute(x);
        // the normal style is untouched
        approx(xs.font_size, 10.0);
        assert_eq!(xs.float, Float::None);
        let fl = s.first_letter(x, &xs).unwrap();
        approx(fl.font_size, 30.0);
        assert_eq!(fl.float, Float::Left);
        assert_eq!(fl.color, Some(Color::rgb(255, 0, 0)));
        assert_eq!(fl.font_weight, 700);
        assert_eq!(fl.display, Display::Inline);
        let y = el(&h, "#y");
        assert!(s.first_letter(y, &s.compute(y)).is_none());
        // inherited properties flow from the element
        let s = StyleSheetSet::new()
            .add_author("p { font-family: Serif; color: blue } p::first-letter { float: left }");
        let fl = s.first_letter(x, &s.compute(x)).unwrap();
        assert_eq!(fl.font_family, vec!["Serif".to_string()]);
        assert_eq!(fl.color, Some(Color::rgb(0, 0, 255)));
        // legacy syntax and the user layer
        let s = StyleSheetSet::new()
            .add_author("p:first-letter { color: red }")
            .add_user("p::first-letter { color: blue }");
        assert_eq!(
            s.first_letter(x, &s.compute(x)).unwrap().color,
            Some(Color::rgb(0, 0, 255))
        );
        // publisher styles ignored: no first-letter
        let s = StyleSheetSet::new()
            .add_author("p::first-letter { color: red }")
            .ignore_publisher_styles(true);
        assert!(s.first_letter(x, &s.compute(x)).is_none());
    }

    #[test]
    fn compute_equals_compute_with_parent_walk() {
        let h = page("<div id=a style='font-size: 14px'><section><p class=q>deep <em>text</em></p></section></div>");
        let s = StyleSheetSet::new().add_author(
            "div { color: #abc; line-height: 1.3 } .q { margin-top: 1em } em { font-size: 120% }",
        );
        let em = el(&h, "em");
        let direct = s.compute(em);
        let mut chain = vec![em];
        let mut cur = em;
        while let Some(p) = cur.parent().and_then(ElementRef::wrap) {
            chain.push(p);
            cur = p;
        }
        let mut prev: Option<ComputedStyle> = None;
        for e in chain.into_iter().rev() {
            prev = Some(s.compute_with_parent(e, prev.as_ref()));
        }
        assert_eq!(prev.unwrap(), direct);
        approx(direct.font_size, 14.0 * 1.2);
        assert_eq!(direct.color, Some(Color::rgb(0xaa, 0xbb, 0xcc)));
    }

    #[test]
    fn display_none_and_page_breaks() {
        let h = page("<div id=a class=hide><p>x</p></div><h2 id=b>t</h2>");
        let s = StyleSheetSet::new().add_author(
            ".hide { display: none } h2 { page-break-before: always; break-after: avoid }",
        );
        assert_eq!(st(&s, &h, "#a").display, Display::None);
        assert_eq!(
            st(&s, &h, "p").display,
            Display::Block,
            "display is not inherited"
        );
        let b = st(&s, &h, "#b");
        assert_eq!(
            (b.page_break_before, b.page_break_after),
            (PageBreak::Always, PageBreak::Avoid)
        );
    }

    #[test]
    fn malformed_css_recovery_in_cascade() {
        let h = page("<p id=x>t</p>");
        let css = "p { color: red; } }}} q { color: blue } p { font-size: ; margin: 1px 2px 3px 4px 5px; font-weight: bold } @import url(x.css); @media print { p { color: green } } p { text-align: ; text-align: center }";
        let s = StyleSheetSet::new().add_author(css);
        let x = st(&s, &h, "#x");
        assert_eq!(x.color, Some(Color::rgb(255, 0, 0)));
        assert_eq!(x.font_weight, 700);
        assert_eq!(x.margin, Edges::default());
        assert_eq!(x.text_align, TextAlign::Center);
        approx(x.font_size, 16.0);
    }

    #[test]
    fn import_and_print_media_ignored_screen_applied() {
        let h = page("<p id=x>t</p>");
        let s = StyleSheetSet::new().add_author("@import 'evil.css'; @media print { p { color: red } } @media screen { p { text-align: right } }");
        let x = st(&s, &h, "#x");
        assert_eq!(x.color, None);
        assert_eq!(x.text_align, TextAlign::Right);
        // width media queries use the container width
        let narrow = StyleSheetSet::new()
            .container_width(400.0)
            .add_author("@media (max-width: 480px) { p { text-align: center } }");
        assert_eq!(st(&narrow, &h, "#x").text_align, TextAlign::Center);
        let wide = StyleSheetSet::new()
            .container_width(900.0)
            .add_author("@media (max-width: 480px) { p { text-align: center } }");
        assert_eq!(st(&wide, &h, "#x").text_align, TextAlign::Start);
    }

    #[test]
    fn empty_sets_and_default() {
        let h = page("<p>x</p>");
        let a = StyleSheetSet::default().add_author("").add_user("   ");
        assert_eq!(a.author_rule_count(), 0);
        assert_eq!(a.user_rule_count(), 0);
        assert_eq!(st(&a, &h, "p"), st(&StyleSheetSet::new(), &h, "p"));
        let root = st(&a, &h, "html");
        assert_eq!(root.color, None);
        approx(root.font_size, 16.0);
        approx(root.root_font_size, 16.0);
    }

    #[test]
    fn deep_nesting_does_not_overflow_the_stack() {
        let depth = 2000;
        let body = format!("{}x{}", "<div>".repeat(depth), "</div>".repeat(depth));
        let h = page(&body);
        let s = StyleSheetSet::new().add_author("div { margin-left: 1px }");
        let last = h
            .select(&Selector::parse("div").unwrap())
            .next_back()
            .unwrap();
        let c = s.compute(last);
        assert_eq!(c.margin.left, 1.0);
    }

    #[test]
    fn background_colour_is_not_inherited() {
        let h = page("<div id=a><p id=b>t</p></div>");
        let s =
            StyleSheetSet::new().add_author("#a { background-color: #112233 } #b { color: red }");
        assert_eq!(
            st(&s, &h, "#a").background_color,
            Some(Color::rgb(0x11, 0x22, 0x33))
        );
        assert_eq!(st(&s, &h, "#b").background_color, None);
        let s = StyleSheetSet::new()
            .add_author("#a { background: #112233 } #b { background: inherit }");
        assert_eq!(
            st(&s, &h, "#b").background_color,
            Some(Color::rgb(0x11, 0x22, 0x33))
        );
    }
}
