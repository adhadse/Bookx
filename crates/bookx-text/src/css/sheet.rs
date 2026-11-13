//! Stylesheet parsing: rules, `@media`, error recovery.

use super::selector::{parse_selector_list, Selector};
use super::values::{parse_declaration, split_important, tokenize, Decl};
use cssparser::{Delimiter, ParseError, Parser, Token};
use std::rc::Rc;

/// One style rule for a single selector (a selector list is split into several rules sharing declarations).
#[derive(Debug, Clone)]
pub struct Rule {
    pub selector: Selector,
    /// Declarations with their `!important` flag, in source order.
    pub decls: Rc<Vec<(Decl, bool)>>,
    /// Source order across the whole stylesheet set.
    pub order: usize,
}

/// Evaluation environment for `@media`.
#[derive(Debug, Clone, Copy)]
pub struct MediaEnv {
    pub viewport_width: f64,
}

type Pe = ParseError<()>;

/// Parse a stylesheet. Invalid rules are skipped, `@import`, `@font-face`, `@page`, `@keyframes`, `@supports`
/// and `@media print` are ignored, `@media screen`/`all` blocks are included. `first_order` is the order number of
/// the first rule; the next free number is returned.
pub fn parse_stylesheet(css: &str, env: MediaEnv, first_order: usize) -> (Vec<Rule>, usize) {
    let mut p = Parser::new(css);
    let mut rules = Vec::new();
    let mut order = first_order;
    parse_rules(&mut p, env, &mut rules, &mut order, 0);
    (rules, order)
}

fn drain(p: &mut Parser<'_>) {
    while p.next().is_ok() {}
}

fn parse_rules(
    p: &mut Parser<'_>,
    env: MediaEnv,
    out: &mut Vec<Rule>,
    order: &mut usize,
    depth: usize,
) {
    loop {
        let start = p.position();
        let tok = match p.next() {
            Ok(t) => t.clone(),
            Err(_) => return,
        };
        match tok {
            Token::AtKeyword(name) => {
                let name = name.to_ascii_lowercase();
                // prelude up to `;` or `{`
                let prelude_start = p.position();
                let _ = p.parse_until_before(
                    Delimiter::CurlyBracketBlock | Delimiter::Semicolon,
                    |p| {
                        drain(p);
                        Ok::<_, Pe>(())
                    },
                );
                let prelude = p.slice_from(prelude_start).to_string();
                // a `;` or the end of input means a statement at-rule such as @import / @charset
                if let Ok(Token::CurlyBracketBlock) = p.next().cloned() {
                    if name == "media" && depth < 8 && media_applies(&prelude, env) {
                        let _ = p.parse_nested_block(|p| {
                            parse_rules(p, env, out, order, depth + 1);
                            Ok::<_, Pe>(())
                        });
                    } else {
                        let _ = p.parse_nested_block(|p| {
                            drain(p);
                            Ok::<_, Pe>(())
                        });
                    }
                }
            }
            Token::CurlyBracketBlock => {
                // block without prelude
                let _ = p.parse_nested_block(|p| {
                    drain(p);
                    Ok::<_, Pe>(())
                });
            }
            Token::Semicolon | Token::CloseCurlyBracket | Token::CDO | Token::CDC => {}
            _ => {
                // qualified rule: everything up to the `{` block is the prelude
                let _ = p.parse_until_before(Delimiter::CurlyBracketBlock, |p| {
                    drain(p);
                    Ok::<_, Pe>(())
                });
                let prelude = p.slice_from(start).to_string();
                match p.next().cloned() {
                    Ok(Token::CurlyBracketBlock) => {
                        let decls = p
                            .parse_nested_block(|p| Ok::<_, Pe>(parse_declarations(p)))
                            .unwrap_or_default();
                        if let Some(sels) = parse_selector_list(&prelude) {
                            let decls = Rc::new(decls);
                            for selector in sels {
                                out.push(Rule {
                                    selector,
                                    decls: decls.clone(),
                                    order: *order,
                                });
                                *order += 1;
                            }
                        }
                    }
                    _ => return, // prelude ran into the end of input
                }
            }
        }
    }
}

fn parse_declarations(p: &mut Parser<'_>) -> Vec<(Decl, bool)> {
    let mut out = Vec::new();
    loop {
        p.skip_whitespace();
        if p.is_exhausted() {
            return out;
        }
        let start = p.position();
        let _ = p.parse_until_after(Delimiter::Semicolon, |p| {
            drain(p);
            Ok::<_, Pe>(())
        });
        let text = p.slice_from(start);
        out.extend(parse_declaration_text(text));
    }
}

/// Parse `name: value [!important]` (a trailing `;` is allowed) to declarations.
pub fn parse_declaration_text(text: &str) -> Vec<(Decl, bool)> {
    let text = super::selector::strip_comments(text);
    let text = text.trim().trim_end_matches(';');
    let Some((name, value)) = text.split_once(':') else {
        return Vec::new();
    };
    let name = name.trim();
    if name.is_empty() || name.contains(|c: char| c.is_whitespace() || c == '{' || c == '}') {
        return Vec::new();
    }
    let (toks, important) = split_important(tokenize(value));
    parse_declaration(name, &toks)
        .into_iter()
        .map(|d| (d, important))
        .collect()
}

/// Whether an `@media` prelude applies to screen rendering.
pub fn media_applies(prelude: &str, env: MediaEnv) -> bool {
    let prelude = prelude.trim();
    if prelude.is_empty() {
        return true;
    }
    prelude.split(',').any(|q| query_applies(q.trim(), env))
}

fn query_applies(q: &str, env: MediaEnv) -> bool {
    let lower = q.to_ascii_lowercase();
    let mut rest = lower.trim();
    let mut negate = false;
    if let Some(r) = rest.strip_prefix("not ") {
        negate = true;
        rest = r.trim();
    } else if let Some(r) = rest.strip_prefix("only ") {
        rest = r.trim();
    }
    let (ty, features) = if rest.starts_with('(') {
        ("all", rest)
    } else {
        match rest.find(" and ") {
            Some(i) => (rest[..i].trim(), rest[i + 5..].trim()),
            None => (rest, ""),
        }
    };
    let type_ok = matches!(ty, "all" | "screen");
    let features_ok = features.is_empty()
        || features
            .split(" and ")
            .all(|f| feature_applies(f.trim(), env));
    (type_ok && features_ok) != negate
}

fn feature_applies(f: &str, env: MediaEnv) -> bool {
    let inner = f.trim().trim_start_matches('(').trim_end_matches(')');
    let Some((name, value)) = inner.split_once(':') else {
        return false;
    };
    let toks = tokenize(value);
    let px = match toks.first() {
        Some(t) => match super::values::parse_len(t) {
            Some(super::values::Len::Px(v)) => v,
            Some(super::values::Len::Em(v)) | Some(super::values::Len::Rem(v)) => v * 16.0,
            _ => return false,
        },
        None => return false,
    };
    match name.trim() {
        "min-width" => env.viewport_width >= px,
        "max-width" => env.viewport_width <= px,
        "width" => (env.viewport_width - px).abs() < 0.5,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::values::{Color, Val};

    const ENV: MediaEnv = MediaEnv {
        viewport_width: 600.0,
    };

    fn parse(css: &str) -> Vec<Rule> {
        parse_stylesheet(css, ENV, 0).0
    }

    #[test]
    fn simple_rules_and_selector_lists() {
        let r = parse("p { color: red } h1, h2 { color: blue; font-weight: bold }");
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].decls.len(), 1);
        assert_eq!(r[1].decls.len(), 2);
        assert!(Rc::ptr_eq(&r[1].decls, &r[2].decls));
        assert_eq!(r.iter().map(|x| x.order).collect::<Vec<_>>(), vec![0, 1, 2]);
        assert_eq!(
            r[0].decls[0],
            (Decl::Color(Val::Set(Color::rgb(255, 0, 0))), false)
        );
    }

    #[test]
    fn order_continues_across_calls() {
        let (a, next) = parse_stylesheet("p{color:red} q{color:red}", ENV, 10);
        assert_eq!(a.iter().map(|x| x.order).collect::<Vec<_>>(), vec![10, 11]);
        assert_eq!(next, 12);
        let (b, next) = parse_stylesheet("", ENV, next);
        assert!(b.is_empty());
        assert_eq!(next, 12);
    }

    #[test]
    fn important_flag() {
        let r = parse("p { color: red !important; margin: 0 ! IMPORTANT; font-size: 1em }");
        assert!(r[0].decls[0].1);
        assert!(r[0].decls[1].1);
        assert!(r[0].decls.iter().filter(|d| d.1).count() >= 2);
        assert!(!r[0].decls.last().unwrap().1);
    }

    #[test]
    fn comments_and_whitespace() {
        let r =
            parse("/* c */ p /* x */ { /* y */ color /* z */ : /* w */ red ; /* v */ } /* end */");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].decls.len(), 1);
    }

    #[test]
    fn bad_declarations_skipped_good_ones_kept() {
        let r = parse("p { color: ; font-size: huge; margin: 1px; : red; nothing; color: blue; glorp-prop: left; ;; }");
        assert_eq!(r.len(), 1);
        let d = &r[0].decls;
        assert_eq!(d.len(), 5, "margin expands to 4 + color: {d:?}");
        assert!(matches!(
            d.last().unwrap().0,
            Decl::Color(Val::Set(Color {
                r: 0,
                g: 0,
                b: 255,
                ..
            }))
        ));
    }

    #[test]
    fn bad_selector_drops_rule_but_not_following() {
        let r = parse(
            "p >> q { color: red } !!! { color: red } r { color: blue } ::: {} s { color: green }",
        );
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn unterminated_input_does_not_panic() {
        for css in [
            "p { color: red",
            "p { color: red;",
            "p",
            "p {",
            "{",
            "}",
            "p { color: red } }",
            "@media screen { p { color: red",
            "@import url(x",
            "p { color: rgb(1,2,3 }",
            "p { content: \"abc }",
        ] {
            let _ = parse(css);
        }
        assert_eq!(
            parse("p { color: red").len(),
            1,
            "an open block is closed at EOF"
        );
        assert_eq!(parse("p { color: red } }  q { color: red }").len(), 2);
    }

    #[test]
    fn at_rules_ignored() {
        let css = r#"
            @charset "utf-8";
            @import url("other.css");
            @import 'x.css' screen;
            @namespace epub "http://www.idpf.org/2007/ops";
            @font-face { font-family: X; src: url(x.ttf) }
            @page { margin: 1in }
            @keyframes spin { from { color: red } to { color: blue } }
            @supports (display: grid) { p { color: green } }
            p { color: red }
        "#;
        let r = parse(css);
        assert_eq!(r.len(), 1);
        assert_eq!(
            r[0].decls[0].0,
            Decl::Color(Val::Set(Color::rgb(255, 0, 0)))
        );
    }

    #[test]
    fn media_print_ignored_screen_applied() {
        let css = r#"
            @media print { p { color: red } }
            @media screen { p { color: blue } }
            @media all { q { color: blue } }
            @media only screen and (min-width: 400px) { a { color: blue } }
            @media screen and (max-width: 400px) { b { color: red } }
            @media (min-width: 50em) { i { color: red } }
            @media print, screen { u { color: blue } }
            @media not print { s { color: blue } }
            @media speech { em { color: red } }
            @media { strong { color: blue } }
            @media screen { @media print { x { color: red } } y { color: blue } }
        "#;
        let r = parse(css);
        let colors: Vec<_> = r
            .iter()
            .map(|x| matches!(x.decls[0].0, Decl::Color(Val::Set(Color { b: 255, .. }))))
            .collect();
        assert!(
            colors.iter().all(|c| *c),
            "only blue (applicable) rules remain: {colors:?}"
        );
        assert_eq!(r.len(), 7, "p, q, a, u, s, strong, y");
    }

    #[test]
    fn media_queries_direct() {
        let a = |q| media_applies(q, ENV);
        assert!(a(""));
        assert!(a("screen"));
        assert!(a("ALL"));
        assert!(a("only screen"));
        assert!(!a("print"));
        assert!(!a("only print"));
        assert!(a("not print"));
        assert!(!a("not screen"));
        assert!(a("print, screen"));
        assert!(!a("print, speech"));
        assert!(a("screen and (min-width: 600px)"));
        assert!(!a("screen and (min-width: 601px)"));
        assert!(a("screen and (max-width: 600px)"));
        assert!(!a("(max-width: 599px)"));
        assert!(a("(min-width: 20em) and (max-width: 40em)"));
        assert!(!a("(orientation: portrait)"));
        assert!(!a("(min-width)"));
        assert!(!a("screen and (min-width: foo)"));
        assert!(a("(width: 600px)"));
        assert!(!a("(width: 500px)"));
    }

    #[test]
    fn nested_blocks_in_declarations_do_not_break_parsing() {
        let r = parse("p { color: red; background: url(data:image/png;base64,AAAA); font-size: 2em } q { color: blue }");
        assert_eq!(r.len(), 2);
        assert_eq!(
            r[0].decls.len(),
            2,
            "unknown background skipped, semicolon inside url() respected"
        );
    }

    #[test]
    fn declaration_text_parser() {
        assert_eq!(parse_declaration_text("color: red").len(), 1);
        assert_eq!(parse_declaration_text(" color : red ; ").len(), 1);
        assert_eq!(
            parse_declaration_text("color: red !important"),
            vec![(Decl::Color(Val::Set(Color::rgb(255, 0, 0))), true)]
        );
        assert!(parse_declaration_text("color red").is_empty());
        assert!(parse_declaration_text("").is_empty());
        assert!(parse_declaration_text(": red").is_empty());
        assert!(parse_declaration_text("a b: red").is_empty());
        assert!(parse_declaration_text("margin: 1 2 3 4 5").is_empty());
    }

    #[test]
    fn unicode_and_escapes_survive() {
        let r = parse(".é { font-family: 'Pâté' } .a\\:b { color: red }");
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn huge_input_is_fast_enough() {
        let mut css = String::new();
        for i in 0..5000 {
            css.push_str(&format!(".c{i} {{ color: red; margin: {i}px }}\n"));
        }
        let r = parse(&css);
        assert_eq!(r.len(), 5000);
    }
}
