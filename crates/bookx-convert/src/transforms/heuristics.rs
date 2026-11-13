//! Heuristic clean-ups from Calibre's "Heuristic processing" group: blank paragraphs, fake
//! margins and soft scene breaks.

use std::collections::HashMap;

use anyhow::Result;
use bookx_core::book::OpenBook;
use bookx_core::contracts::{Transform, TransformReport};
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::css::{css_paths, parse_css, parse_decls, split_selectors, CssItem};
use super::map_spine_files;
use crate::markup::{decode_entities, get_attr, tag_info, tokenize, Tok};

fn paragraph_re() -> Regex {
    Regex::new(r"(?s)(<p(?:\s[^>]*)?>)(.*?)</p>").expect("static regex")
}

// ---------------------------------------------------------------------------
// blank paragraphs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoveBlankParagraphs {
    pub jobs: usize,
}

fn is_blank_inner(inner: &str) -> bool {
    for t in tokenize(inner) {
        match t {
            Tok::Text(s) => {
                let d = decode_entities(s);
                if d.chars()
                    .any(|c| !(c.is_whitespace() || c == '\u{200b}' || c == '\u{feff}'))
                {
                    return false;
                }
            }
            Tok::Tag(tag) => {
                if let Some(info) = tag_info(tag) {
                    if matches!(
                        info.name.as_str(),
                        "img"
                            | "svg"
                            | "image"
                            | "audio"
                            | "video"
                            | "object"
                            | "iframe"
                            | "hr"
                            | "math"
                            | "embed"
                    ) {
                        return false;
                    }
                    if get_attr(tag, "id").is_some() || get_attr(tag, "name").is_some() {
                        return false;
                    }
                }
            }
        }
    }
    true
}

impl RemoveBlankParagraphs {
    pub fn clean(&self, html: &str) -> String {
        let re = paragraph_re();
        let empty_selfclosed = Regex::new(r"<p(?:\s[^>/]*)?/>\n?").expect("static regex");
        let out = re.replace_all(html, |c: &regex::Captures| {
            if get_attr(&c[1], "id").is_none() && is_blank_inner(&c[2]) {
                String::new()
            } else {
                c[0].to_string()
            }
        });
        let out = empty_selfclosed.replace_all(&out, "");
        // drop the blank line a removed paragraph leaves behind
        Regex::new(r"\n[ \t]*\n([ \t]*\n)+")
            .expect("static regex")
            .replace_all(&out, "\n\n")
            .into_owned()
    }
}

impl Transform for RemoveBlankParagraphs {
    fn id(&self) -> &'static str {
        "remove_blank_paragraphs"
    }
    fn apply(&self, book: &mut OpenBook) -> Result<TransformReport> {
        let n = map_spine_files(book, self.jobs, |_, t| Some(self.clean(t)))?;
        Ok(TransformReport {
            files_changed: n,
            notes: vec![],
        })
    }
}

// ---------------------------------------------------------------------------
// scene breaks
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReplaceSceneBreaks {
    /// XHTML that replaces each soft scene break.
    pub replacement: String,
    pub jobs: usize,
}

impl Default for ReplaceSceneBreaks {
    fn default() -> Self {
        ReplaceSceneBreaks {
            replacement: "<hr class=\"scene-break\"/>".into(),
            jobs: 0,
        }
    }
}

const BREAK_CHARS: &str = "*-\u{2013}\u{2014}_=~#\u{2022}\u{b7}\u{25cf}\u{25c6}\u{25ca}\u{2021}\u{2020}+\u{2217}\u{2042}\u{2766}";

pub fn is_scene_break_text(text: &str) -> bool {
    let t: String = decode_entities(text)
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{a0}')
        .collect();
    let n = t.chars().count();
    (3..=24).contains(&n) && t.chars().all(|c| BREAK_CHARS.contains(c))
}

impl ReplaceSceneBreaks {
    pub fn replace(&self, html: &str) -> String {
        paragraph_re()
            .replace_all(html, |c: &regex::Captures| {
                let inner = &c[2];
                let has_image = tokenize(inner)
                    .iter()
                    .any(|t| matches!(t, Tok::Tag(g) if tag_info(g).is_some_and(|i| i.name == "img" || i.name == "svg")));
                let text = crate::markup::plain_text(inner);
                if !has_image && is_scene_break_text(&text) {
                    self.replacement.clone()
                } else {
                    c[0].to_string()
                }
            })
            .into_owned()
    }
}

impl Transform for ReplaceSceneBreaks {
    fn id(&self) -> &'static str {
        "replace_scene_breaks"
    }
    fn apply(&self, book: &mut OpenBook) -> Result<TransformReport> {
        let n = map_spine_files(book, self.jobs, |_, t| Some(self.replace(t)))?;
        Ok(TransformReport {
            files_changed: n,
            notes: vec![],
        })
    }
}

// ---------------------------------------------------------------------------
// fake margins
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct RemoveFakeMargins;

type Margins = (String, String);

fn norm_len(v: &str) -> String {
    let v = v.trim().to_ascii_lowercase().replace("!important", "");
    let v = v.trim().to_string();
    let digits: String = v
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();
    if digits.parse::<f64>().map(|n| n == 0.0).unwrap_or(false) {
        "0".into()
    } else {
        v
    }
}

fn margins_of_decls(decls: &[(String, String, String)]) -> (Option<String>, Option<String>) {
    let mut l = None;
    let mut r = None;
    for (n, v, _) in decls {
        match n.as_str() {
            "margin-left" => l = Some(norm_len(v)),
            "margin-right" => r = Some(norm_len(v)),
            _ => {}
        }
    }
    (l, r)
}

fn class_margins(book: &OpenBook) -> HashMap<String, (Option<String>, Option<String>)> {
    let mut map = HashMap::new();
    let class_re = Regex::new(r"^(?:p|div)?\.([A-Za-z0-9_-]+)$").expect("static regex");
    for path in css_paths(book) {
        let Some(css) = book.archive.get_string(&path) else {
            continue;
        };
        for item in parse_css(&css) {
            if let CssItem::Rule { selectors, block } = item {
                let (l, r) = margins_of_decls(&parse_decls(block));
                if l.is_none() && r.is_none() {
                    continue;
                }
                for s in split_selectors(selectors) {
                    if let Some(c) = class_re.captures(&s) {
                        map.insert(c[1].to_string(), (l.clone(), r.clone()));
                    }
                }
            }
        }
    }
    map
}

fn effective_margins(
    tag: &str,
    classes: &HashMap<String, (Option<String>, Option<String>)>,
) -> Margins {
    let mut l = None;
    let mut r = None;
    if let Some(cl) = get_attr(tag, "class") {
        for c in cl.split_whitespace() {
            if let Some((cl_l, cl_r)) = classes.get(c) {
                l = cl_l.clone().or(l);
                r = cl_r.clone().or(r);
            }
        }
    }
    if let Some(style) = get_attr(tag, "style") {
        let (il, ir) = margins_of_decls(&parse_decls(&format!("{{{style}}}")));
        l = il.or(l);
        r = ir.or(r);
    }
    (
        l.filter(|v| v != "0").unwrap_or_else(|| "0".into()),
        r.filter(|v| v != "0").unwrap_or_else(|| "0".into()),
    )
}

fn strip_margin_decls(style: &str, common: &Margins) -> String {
    parse_decls(&format!("{{{style}}}"))
        .into_iter()
        .filter(|(n, v, _)| {
            let v = norm_len(v);
            !((n == "margin-left" && v == common.0) || (n == "margin-right" && v == common.1))
        })
        .map(|(_, _, text)| text.trim().to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

impl Transform for RemoveFakeMargins {
    fn id(&self) -> &'static str {
        "remove_fake_margins"
    }

    fn apply(&self, book: &mut OpenBook) -> Result<TransformReport> {
        let classes = class_margins(book);
        let p_open = Regex::new(r"(?i)<p(?:\s[^>]*)?>").expect("static regex");

        // histogram of (left, right) margins over all paragraphs of the book
        let mut hist: HashMap<Margins, usize> = HashMap::new();
        let mut total = 0usize;
        for path in book.spine_paths() {
            let Some(text) = book.archive.get_string(&path) else {
                continue;
            };
            for m in p_open.find_iter(&text) {
                total += 1;
                *hist
                    .entry(effective_margins(m.as_str(), &classes))
                    .or_default() += 1;
            }
        }
        let common = hist
            .iter()
            .filter(|(k, _)| **k != ("0".to_string(), "0".to_string()))
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
            .map(|(k, v)| (k.clone(), *v));
        let Some((common, count)) = common.filter(|(_, c)| c * 2 > total) else {
            return Ok(TransformReport::default());
        };

        // paragraphs: strip inline declarations; remember the classes that carried the margin
        let mut carried: std::collections::HashSet<String> = std::collections::HashSet::new();
        for path in book.spine_paths() {
            let Some(text) = book.archive.get_string(&path) else {
                continue;
            };
            for m in p_open.find_iter(&text) {
                if effective_margins(m.as_str(), &classes) == common {
                    if let Some(cl) = get_attr(m.as_str(), "class") {
                        carried.extend(cl.split_whitespace().map(String::from));
                    }
                }
            }
        }
        let common_ref = &common;
        let classes_ref = &classes;
        let changed_html = map_spine_files(book, 0, |_, text| {
            Some(
                p_open
                    .replace_all(text, |c: &regex::Captures| {
                        let tag = &c[0];
                        if effective_margins(tag, classes_ref) != *common_ref {
                            return tag.to_string();
                        }
                        match get_attr(tag, "style") {
                            Some(style) => {
                                let new = strip_margin_decls(&style, common_ref);
                                if new == style {
                                    tag.to_string()
                                } else if new.trim().is_empty() {
                                    crate::markup::remove_attr(tag, "style")
                                } else {
                                    crate::markup::set_attr(tag, "style", &new)
                                }
                            }
                            None => tag.to_string(),
                        }
                    })
                    .into_owned(),
            )
        })?;

        // stylesheets: remove the margin declarations from the classes that carried it
        let mut changed_css = 0;
        let class_re = Regex::new(r"^(?:p|div)?\.([A-Za-z0-9_-]+)$").expect("static regex");
        for path in css_paths(book) {
            let Some(css) = book.archive.get_string(&path) else {
                continue;
            };
            let mut out = String::new();
            let mut touched = false;
            for item in parse_css(&css) {
                match item {
                    CssItem::Rule { selectors, block } => {
                        let hit = split_selectors(selectors).iter().any(|s| {
                            class_re
                                .captures(s)
                                .is_some_and(|c| carried.contains(&c[1]))
                        });
                        if hit {
                            let decls = parse_decls(block);
                            let kept: Vec<String> = decls
                                .iter()
                                .filter(|(n, v, _)| {
                                    let v = norm_len(v);
                                    !((n == "margin-left" && v == common.0)
                                        || (n == "margin-right" && v == common.1))
                                })
                                .map(|(_, _, t)| t.trim().to_string())
                                .collect();
                            if kept.len() != decls.len() {
                                touched = true;
                                if !kept.is_empty() {
                                    out.push_str(&format!(
                                        "{}{{ {} }}",
                                        selectors,
                                        kept.join("; ")
                                    ));
                                }
                                continue;
                            }
                        }
                        out.push_str(selectors);
                        out.push_str(block);
                    }
                    CssItem::At(s) | CssItem::Other(s) => out.push_str(s),
                }
            }
            if touched {
                book.archive.set_string(&path, &out);
                changed_css += 1;
            }
        }
        Ok(TransformReport {
            files_changed: changed_html + changed_css,
            notes: vec![format!(
                "removed margins {} / {} from {count} of {total} paragraphs",
                common.0, common.1
            )],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;

    #[test]
    fn blank_paragraphs_removed() {
        let t = RemoveBlankParagraphs::default();
        let html = "<p>a</p>\n<p></p>\n<p>&nbsp;</p>\n<p> <br/> </p>\n<p class=\"x\"> </p>\n<p>b</p>\n<p/>\n";
        let out = t.clean(html);
        assert_eq!(out.matches("<p").count(), 2, "{out}");
        assert!(out.contains("<p>a</p>") && out.contains("<p>b</p>"));
    }

    #[test]
    fn blank_paragraph_keeps_images_ids_and_text() {
        let t = RemoveBlankParagraphs::default();
        let html = "<p><img src=\"a.png\"/></p><p id=\"anchor\"></p><p><a id=\"x\"/></p><p>0</p><p><span>.</span></p><p><svg><rect/></svg></p>";
        assert_eq!(t.clean(html), html);
    }

    #[test]
    fn blank_paragraph_idempotent_and_collapses_gaps() {
        let t = RemoveBlankParagraphs::default();
        let out = t.clean("<p>a</p>\n\n<p></p>\n\n\n<p>b</p>");
        assert_eq!(t.clean(&out), out);
        assert!(!out.contains("\n\n\n"));
    }

    #[test]
    fn blank_transform_reports() {
        let mut b = book_with(&[("A", "<p>x</p><p>&nbsp;</p>"), ("B", "<p>y</p>")]);
        let t = RemoveBlankParagraphs::default();
        assert_eq!(t.apply(&mut b).unwrap().files_changed, 1);
        assert_eq!(t.apply(&mut b).unwrap().files_changed, 0);
        assert_eq!(t.id(), "remove_blank_paragraphs");
    }

    #[test]
    fn scene_break_detection() {
        for yes in [
            "* * *",
            "***",
            "- - -",
            "~~~",
            "###",
            "• • •",
            "&#8226; &#8226; &#8226;",
            "———",
            "_ _ _ _",
            "*\u{a0}*\u{a0}*",
        ] {
            assert!(is_scene_break_text(yes), "{yes}");
        }
        for no in [
            "",
            "*",
            "**",
            "...",
            "Chapter",
            "* * * and then",
            "-",
            "—",
            "1 2 3",
            "*****************************",
        ] {
            assert!(!is_scene_break_text(no), "{no}");
        }
    }

    #[test]
    fn scene_breaks_replaced_with_default_hr() {
        let t = ReplaceSceneBreaks::default();
        let out =
            t.replace("<p>text</p><p>* * *</p><p class=\"c\"><b>***</b></p><p>more *** text</p>");
        assert_eq!(
            out.matches("<hr class=\"scene-break\"/>").count(),
            2,
            "{out}"
        );
        assert!(out.contains("<p>more *** text</p>"));
    }

    #[test]
    fn scene_breaks_custom_replacement_and_idempotent() {
        let t = ReplaceSceneBreaks {
            replacement: "<p class=\"sb\">&#8258;</p>".into(),
            jobs: 1,
        };
        let out = t.replace("<p>#  #  #</p>");
        assert_eq!(out, "<p class=\"sb\">&#8258;</p>");
        assert_eq!(t.replace(&out), out);
        let d = ReplaceSceneBreaks::default();
        let o = d.replace("<p>***</p>");
        assert_eq!(d.replace(&o), o);
    }

    #[test]
    fn scene_break_with_image_is_kept() {
        let t = ReplaceSceneBreaks::default();
        let html = "<p>* * *<img src=\"x.png\"/></p>";
        assert_eq!(t.replace(html), html);
    }

    #[test]
    fn scene_break_transform() {
        let mut b = book_with(&[("A", "<p>a</p><p>* * *</p><p>b</p>")]);
        let t = ReplaceSceneBreaks::default();
        assert_eq!(t.apply(&mut b).unwrap().files_changed, 1);
        assert_eq!(t.apply(&mut b).unwrap().files_changed, 0);
        assert_eq!(t.id(), "replace_scene_breaks");
    }

    fn margin_book(n_margin: usize, n_plain: usize) -> OpenBook {
        let mut body = String::new();
        for i in 0..n_margin {
            body.push_str(&format!(
                "<p style=\"margin-left: 5%; margin-right:5%; color:red\">m{i}</p>\n"
            ));
        }
        for i in 0..n_plain {
            body.push_str(&format!("<p>p{i}</p>\n"));
        }
        book_with(&[("A", &body)])
    }

    #[test]
    fn fake_inline_margins_removed_when_majority() {
        let mut b = margin_book(6, 2);
        let r = RemoveFakeMargins.apply(&mut b).unwrap();
        assert!(r.files_changed >= 1);
        let ch = chapter(&b, 0);
        assert!(!ch.contains("margin-left"), "{ch}");
        assert!(ch.contains("color:red"));
        assert!(ch.contains("m0") && ch.contains("p1"));
    }

    #[test]
    fn minority_margins_are_kept() {
        let mut b = margin_book(2, 6);
        let before = snapshot(&b);
        let r = RemoveFakeMargins.apply(&mut b).unwrap();
        assert_eq!(r.files_changed, 0);
        assert_eq!(snapshot(&b), before);
    }

    #[test]
    fn exactly_half_is_not_a_majority() {
        let mut b = margin_book(4, 4);
        assert_eq!(RemoveFakeMargins.apply(&mut b).unwrap().files_changed, 0);
    }

    #[test]
    fn style_attribute_removed_when_it_becomes_empty() {
        let mut body = String::new();
        for _ in 0..5 {
            body.push_str("<p style=\"margin-left:10%\">x</p>");
        }
        let mut b = book_with(&[("A", &body)]);
        RemoveFakeMargins.apply(&mut b).unwrap();
        assert!(!chapter(&b, 0).contains("style="));
    }

    #[test]
    fn class_based_margins_removed_from_css_and_idempotent() {
        let mut body = String::new();
        for _ in 0..5 {
            body.push_str("<p class=\"ind\">x</p>");
        }
        body.push_str("<p class=\"other\">y</p>");
        let mut b = book_with(&[("A", &body)]);
        b.archive.set_string(
            "OEBPS/style.css",
            ".ind { margin-left: 4em; margin-right: 4em; font-style: italic }\n.other { margin-left: 1em }\n",
        );
        let r = RemoveFakeMargins.apply(&mut b).unwrap();
        assert!(r.files_changed >= 1);
        let css = b.archive.get_string("OEBPS/style.css").unwrap();
        assert!(!css.contains("4em"), "{css}");
        assert!(css.contains("font-style: italic"));
        assert!(css.contains(".other { margin-left: 1em }"));
        let snap = snapshot(&b);
        assert_eq!(RemoveFakeMargins.apply(&mut b).unwrap().files_changed, 0);
        assert_eq!(snapshot(&b), snap);
    }

    #[test]
    fn second_most_common_margin_survives_idempotently() {
        let mut body = String::new();
        for _ in 0..6 {
            body.push_str("<p style=\"margin-left:5%\">a</p>");
        }
        for _ in 0..4 {
            body.push_str("<p style=\"margin-left:10%\">b</p>");
        }
        let mut b = book_with(&[("A", &body)]);
        RemoveFakeMargins.apply(&mut b).unwrap();
        assert_eq!(chapter(&b, 0).matches("margin-left:10%").count(), 4);
        let snap = snapshot(&b);
        RemoveFakeMargins.apply(&mut b).unwrap();
        assert_eq!(snapshot(&b), snap);
    }

    #[test]
    fn zero_margins_do_not_count_and_no_paragraphs_is_noop() {
        let mut b = book_with(&[("A", "<p style=\"margin-left:0\">x</p><div>y</div>")]);
        assert_eq!(RemoveFakeMargins.apply(&mut b).unwrap().files_changed, 0);
        assert_eq!(RemoveFakeMargins.id(), "remove_fake_margins");
    }
}
