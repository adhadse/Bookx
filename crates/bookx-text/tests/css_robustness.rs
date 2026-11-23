//! The CSS parser and cascade must never panic, whatever the input.

use bookx_text::css::{
    MediaEnv, StyleSheetSet, parse_declaration_text, parse_selector_list, parse_stylesheet,
};
use scraper::{Html, Selector};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

const CORPUS: &[&str] = &[
    "body { margin: 0; font: 12pt/1.4 Georgia, 'Times New Roman', serif } p { text-indent: 1.5em; margin: 0 0 .5em } h1 { page-break-before: always }",
    "@charset 'utf-8'; @import url('a.css'); @media screen and (min-width: 400px) { .a > .b + .c ~ .d { color: #abc !important } } @page { margin: 1in }",
    "a[href^='http']:not(.x)::first-letter, li:nth-child(2n+1) em { font-variant: small-caps; text-decoration: underline line-through; float: left }",
    "@font-face { font-family: X; src: url(data:font/woff2;base64,AAAA) } p::before { content: '\\201C' } .e\\:f { width: calc(100% - 2px) }",
    "img { max-width: 100%; width: 50%; margin: 0 auto; display: block } blockquote { margin: 1em 40px; padding: 0 1em } hr { border: 0 }",
    "/* comment */ p:first-child { color: rgba(1,2,3,.5) } p:last-of-type { hyphens: auto; -epub-hyphens: manual } td { text-align: -webkit-center }",
];

const ALPHABET: &[&str] = &[
    "{",
    "}",
    "(",
    ")",
    "[",
    "]",
    ";",
    ":",
    ",",
    "\"",
    "'",
    "\\",
    "/",
    "*",
    "@",
    "!",
    "#",
    ".",
    ">",
    "+",
    "~",
    " ",
    "\n",
    "%",
    "-",
    "é",
    "日本",
    "\u{0}",
    "\u{fffd}",
    "0",
    "9",
    "e",
    "important",
    "url(",
    "/*",
    "*/",
    "<!--",
    "-->",
    "calc(",
    "inherit",
    "px",
    "em",
    "n",
    "::",
    ":not(",
    "auto",
];

fn mutate(rng: &mut Rng, src: &str) -> String {
    let mut bytes = src.as_bytes().to_vec();
    for _ in 0..1 + rng.below(12) {
        match rng.below(5) {
            0 if !bytes.is_empty() => {
                let i = rng.below(bytes.len());
                bytes.remove(i);
            }
            1 => {
                let i = rng.below(bytes.len() + 1);
                let tok = ALPHABET[rng.below(ALPHABET.len())].as_bytes();
                for (k, b) in tok.iter().enumerate() {
                    bytes.insert(i + k, *b);
                }
            }
            2 if !bytes.is_empty() => {
                let i = rng.below(bytes.len());
                bytes[i] = (rng.next() & 0xff) as u8;
            }
            3 if bytes.len() > 4 => {
                let a = rng.below(bytes.len() - 2);
                let b = (a + 1 + rng.below(20)).min(bytes.len());
                let chunk = bytes[a..b].to_vec();
                let at = rng.below(bytes.len() + 1);
                for (k, c) in chunk.iter().enumerate() {
                    bytes.insert(at + k, *c);
                }
            }
            _ if bytes.len() > 4 => {
                let a = rng.below(bytes.len());
                bytes.truncate(a.max(1));
            }
            _ => {}
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn doc() -> Html {
    Html::parse_document(
        "<html><body><div class='a b' id=c><p class=x style='margin: 1px; color: red'>Text <em>em</em><a href='http://x'>l</a></p><ul><li>1</li><li>2</li></ul><img src=i></div></body></html>",
    )
}

#[test]
fn thousand_mutated_stylesheets_never_panic() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let html = doc();
    let all = Selector::parse("*").unwrap();
    let env = MediaEnv {
        viewport_width: 600.0,
    };
    for i in 0..1500 {
        let src = CORPUS[i % CORPUS.len()];
        let css = mutate(&mut rng, src);
        let (rules, next) = parse_stylesheet(&css, env, 0);
        assert!(next >= rules.len());
        let sheets = StyleSheetSet::new().add_author(&css).add_user(&css);
        for el in html.select(&all) {
            let s = sheets.compute(el);
            assert!(
                s.font_size.is_finite() && s.font_size >= 0.0,
                "css: {css:?}"
            );
            assert!(s.margin.top.is_finite() && s.padding.left.is_finite());
            let _ = sheets.first_letter(el, &s);
        }
    }
}

#[test]
fn random_byte_soup_never_panics() {
    let mut rng = Rng(42);
    let env = MediaEnv {
        viewport_width: 320.0,
    };
    for _ in 0..1000 {
        let len = rng.below(300);
        let bytes: Vec<u8> = (0..len).map(|_| (rng.next() & 0xff) as u8).collect();
        let s = String::from_utf8_lossy(&bytes).into_owned();
        let _ = parse_stylesheet(&s, env, 0);
        let _ = parse_selector_list(&s);
        let _ = parse_declaration_text(&s);
    }
}

#[test]
fn token_soup_from_css_alphabet_never_panics() {
    let mut rng = Rng(7);
    let env = MediaEnv {
        viewport_width: 800.0,
    };
    let html = doc();
    let p = html.select(&Selector::parse("p").unwrap()).next().unwrap();
    for _ in 0..1000 {
        let n = rng.below(60);
        let css: String = (0..n)
            .map(|_| ALPHABET[rng.below(ALPHABET.len())])
            .collect::<Vec<_>>()
            .join("");
        let _ = parse_stylesheet(&css, env, 0);
        let sheets = StyleSheetSet::new().add_author(&css);
        let _ = sheets.compute(p);
    }
}

#[test]
fn pathological_inputs() {
    let env = MediaEnv {
        viewport_width: 600.0,
    };
    let deep = format!(
        "{}p{{color:red}}{}",
        "@media screen{".repeat(5000),
        "}".repeat(5000)
    );
    let _ = parse_stylesheet(&deep, env, 0);
    let long_sel = format!("{} {{ color: red }}", "a ".repeat(20000));
    let _ = parse_stylesheet(&long_sel, env, 0);
    let many_commas = format!("{} {{ color: red }}", "a,".repeat(20000));
    let _ = parse_stylesheet(&many_commas, env, 0);
    let unclosed = "(".repeat(100_000);
    let _ = parse_stylesheet(&unclosed, env, 0);
    let _ = parse_selector_list(&format!("p{}", ":not(".repeat(1000)));
    let _ = parse_selector_list(&"[".repeat(10_000));
    let _ = parse_declaration_text(&format!("margin: {}", "1px ".repeat(10_000)));
    let _ = parse_declaration_text(&format!("font-family: {}", "a,".repeat(10_000)));
}

#[test]
fn nested_parens_in_values() {
    let html = doc();
    let p = html.select(&Selector::parse("p").unwrap()).next().unwrap();
    let css = format!(
        "p {{ color: rgb({}1,2,3{}); margin: 3px }}",
        "(".repeat(50),
        ")".repeat(50)
    );
    let s = StyleSheetSet::new().add_author(&css).compute(p);
    assert_eq!(s.margin.top, 1.0, "inline style margin: 1px still wins");
}
