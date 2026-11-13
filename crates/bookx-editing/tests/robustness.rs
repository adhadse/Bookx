//! Garbage in, no panic out: every text-processing entry point is fed thousands of random, malformed inputs.

use bookx_editing::fixes::{fix_missing_namespace, repair_xhtml};
use bookx_editing::markup::*;
use bookx_editing::spell::extract_words;
use bookx_editing::split_merge::{split_points, PageBreakRules, SplitSpec};
use bookx_editing::transform_css::{
    transform_declarations, transform_stylesheet, CssTransform, Unit,
};

struct Lcg(u64);
impl Lcg {
    fn next(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n
    }
}

const PIECES: &[&str] = &[
    "<",
    ">",
    "</",
    "/>",
    "<p>",
    "</p>",
    "<div class=\"a\">",
    "</div>",
    "<br>",
    "<img src=\"x.png\">",
    "<a href=\"#x\">",
    "</a>",
    "<!--",
    "-->",
    "<![CDATA[",
    "]]>",
    "<?xml ?>",
    "<style>",
    "</style>",
    "<script>",
    "</script>",
    "&",
    "&amp;",
    "&#",
    "&#x41;",
    "&bogus;",
    "\"",
    "'",
    "=",
    " ",
    "\n",
    "é",
    "日本",
    "😀",
    "\u{feff}",
    "id=\"x\"",
    "href=",
    "src=",
    "url(",
    ")",
    "{",
    "}",
    ";",
    ":",
    "color: red",
    "@media",
    "@import",
    "/*",
    "*/",
    "<html>",
    "</html>",
    "<body>",
    "</body>",
    "<h1 id=\"a\">",
    "</h1>",
    "lang=\"en\"",
    "text",
    "word",
    "42",
    "\\",
    "%",
    "#",
    "?",
    "xlink:href=",
];

fn garbage(r: &mut Lcg) -> String {
    (0..r.next(40))
        .map(|_| PIECES[r.next(PIECES.len())])
        .collect()
}

#[test]
fn text_processing_never_panics() {
    for seed in [0xB00Cu64, 7, 0xDEAD_BEEF] {
        let mut r = Lcg(seed);
        let ts = [
            CssTransform::RemoveProperty {
                name: "color".into(),
            },
            CssTransform::ReplaceColor {
                from: "red".into(),
                to: "blue".into(),
            },
            CssTransform::ConvertUnits {
                from: Unit::Pt,
                to: Unit::Px,
                base_px: 16.0,
                properties: vec![],
            },
            CssTransform::RenameFontFamily {
                from: "a".into(),
                to: "b".into(),
            },
            CssTransform::ScaleFontSizes { factor: 2.0 },
        ];
        let mut repaired = 0;
        for _ in 0..4000 {
            let s = garbage(&mut r);
            let _ = scan(&s);
            let _ = plain_text(&s);
            let _ = normalized_text(&s);
            let _ = decode_entities(&s);
            let _ = decode_with_offsets(&s);
            let _ = collect_refs(&s, RefKind::Markup);
            let _ = collect_refs(&s, RefKind::Css);
            let _ = map_refs(&s, RefKind::Markup, &mut |x| Some(format!("{x}!")));
            let _ = map_refs(&s, RefKind::Css, &mut |x| Some(format!("{x}!")));
            let _ = defined_ids(&s);
            let _ = body_bounds(&s);
            let _ = tag_attrs(&s);
            let _ = rewrite_all_tags(&s, &mut |_, _, v| Some(format!("{v}x")));
            let _ = map_style_contents(&s, &mut |c| c.to_string());
            let _ = extract_words(&s);
            let _ = fix_missing_namespace(&s);
            if let Some(fixed) = repair_xhtml(&s) {
                repaired += 1;
                roxmltree::Document::parse_with_options(
                    &fixed,
                    roxmltree::ParsingOptions {
                        allow_dtd: true,
                        ..Default::default()
                    },
                )
                .expect("repaired output must parse");
            }
            let _ = split_points(&s, &SplitSpec::Headings(2), &PageBreakRules::default());
            let _ = split_points(&s, &SplitSpec::PageBreaks, &PageBreakRules::default());
            let _ = transform_stylesheet(&s, &ts);
            let _ = transform_declarations(&s, &ts);
            let _ = bookx_editing::links::validate_url(&s);
            let _ = bookx_editing::reports::tokenize_words(&s);
        }
        assert!(
            repaired > 100,
            "seed {seed}: only {repaired} inputs could be repaired"
        );
    }
}

#[test]
fn well_formed_inputs_survive_repair_untouched_in_meaning() {
    let src = "<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p id=\"a\">x &amp; y &lt; z</p><br/></body></html>";
    let fixed = repair_xhtml(src).unwrap();
    assert_eq!(normalized_text(&fixed), normalized_text(src));
    assert_eq!(fixed, src);
}
