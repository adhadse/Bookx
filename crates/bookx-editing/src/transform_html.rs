//! Transform HTML (CE-421): rule-based element and attribute changes over all the pages of a book, the way
//! `transform_css` does it for styles. A rule names the elements it applies to with the same small selector
//! language as the split tool (`p`, `div.note`, `#id`, `//section/h1`, `//*[@epub:type='note']`).
//!
//! The engine edits the source text in place, so everything a rule does not touch stays byte for byte as it
//! was: entities, quotes, comments, namespace prefixes, whitespace.

use anyhow::{Result, bail};
use bookx_core::book::OpenBook;
use bookx_core::util::parse_xml;
use serde::{Deserialize, Serialize};

use crate::markup::{is_markup_path, set_tag_attr, start_tag_end, tag_attr, tag_qname};
use crate::split_merge::Selector;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HtmlRule {
    /// `<b>` becomes `<strong>`; a namespace prefix is kept.
    RenameTag {
        selector: String,
        to: String,
    },
    /// Removes the tags and keeps what is inside.
    Unwrap {
        selector: String,
    },
    /// Removes the element with everything inside.
    Remove {
        selector: String,
    },
    SetAttr {
        selector: String,
        name: String,
        value: String,
    },
    RemoveAttr {
        selector: String,
        name: String,
    },
    /// Replaces text inside the value of an attribute.
    ReplaceAttr {
        selector: String,
        name: String,
        find: String,
        replace: String,
    },
    /// Wraps each element in a new one (`<div>`...).
    Wrap {
        selector: String,
        wrapper: String,
    },
    AddClass {
        selector: String,
        class: String,
    },
    RemoveClass {
        selector: String,
        class: String,
    },
    ReplaceClass {
        selector: String,
        from: String,
        to: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtmlRuleSet {
    pub name: String,
    pub rules: Vec<HtmlRule>,
}

impl HtmlRuleSet {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
    pub fn from_json(text: &str) -> Option<HtmlRuleSet> {
        serde_json::from_str(text).ok()
    }
}

/// The kinds of rule, in the order of the rule builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HtmlRuleKind {
    RenameTag,
    Unwrap,
    Remove,
    SetAttr,
    RemoveAttr,
    ReplaceAttr,
    Wrap,
    AddClass,
    RemoveClass,
    ReplaceClass,
}

impl HtmlRuleKind {
    pub const ALL: [HtmlRuleKind; 10] = [
        HtmlRuleKind::RenameTag,
        HtmlRuleKind::Unwrap,
        HtmlRuleKind::Remove,
        HtmlRuleKind::SetAttr,
        HtmlRuleKind::RemoveAttr,
        HtmlRuleKind::ReplaceAttr,
        HtmlRuleKind::Wrap,
        HtmlRuleKind::AddClass,
        HtmlRuleKind::RemoveClass,
        HtmlRuleKind::ReplaceClass,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::RenameTag => "rename",
            Self::Unwrap => "unwrap",
            Self::Remove => "remove",
            Self::SetAttr => "set-attr",
            Self::RemoveAttr => "remove-attr",
            Self::ReplaceAttr => "replace-attr",
            Self::Wrap => "wrap",
            Self::AddClass => "add-class",
            Self::RemoveClass => "remove-class",
            Self::ReplaceClass => "replace-class",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.id() == id)
    }

    /// A short verb phrase for a combo row (at most 26 characters); [`HtmlRuleKind::label`] is the sentence.
    pub fn short_label(self) -> &'static str {
        match self {
            Self::RenameTag => "Rename tag",
            Self::Unwrap => "Unwrap tag",
            Self::Remove => "Remove element",
            Self::SetAttr => "Set attribute",
            Self::RemoveAttr => "Remove attribute",
            Self::ReplaceAttr => "Replace in attribute",
            Self::Wrap => "Wrap in element",
            Self::AddClass => "Add class",
            Self::RemoveClass => "Remove class",
            Self::ReplaceClass => "Change class",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::RenameTag => "Rename the tag",
            Self::Unwrap => "Remove the tag, keep its content",
            Self::Remove => "Remove the element and its content",
            Self::SetAttr => "Set an attribute",
            Self::RemoveAttr => "Remove an attribute",
            Self::ReplaceAttr => "Replace text in an attribute",
            Self::Wrap => "Wrap in a new element",
            Self::AddClass => "Add a class",
            Self::RemoveClass => "Remove a class",
            Self::ReplaceClass => "Change a class",
        }
    }

    /// Labels and example texts of the fields after the selector.
    pub fn fields(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::RenameTag => &[("New tag name", "strong")],
            Self::Unwrap | Self::Remove => &[],
            Self::SetAttr => &[("Attribute", "lang"), ("Value", "en")],
            Self::RemoveAttr => &[("Attribute", "style")],
            Self::ReplaceAttr => &[
                ("Attribute", "href"),
                ("Find", ".htm"),
                ("Replace with", ".xhtml"),
            ],
            Self::Wrap => &[("Wrapper tag", "div")],
            Self::AddClass | Self::RemoveClass => &[("Class", "note")],
            Self::ReplaceClass => &[("Old class", "calibre1"), ("New class", "body")],
        }
    }

    /// Builds a rule from the selector and the field texts.
    pub fn build(self, selector: &str, v: &[String]) -> Result<HtmlRule> {
        let get = |i: usize| v.get(i).map(|s| s.trim().to_string()).unwrap_or_default();
        let selector = selector.trim().to_string();
        let rule = match self {
            Self::RenameTag => HtmlRule::RenameTag {
                selector,
                to: get(0),
            },
            Self::Unwrap => HtmlRule::Unwrap { selector },
            Self::Remove => HtmlRule::Remove { selector },
            Self::SetAttr => HtmlRule::SetAttr {
                selector,
                name: get(0),
                value: v.get(1).cloned().unwrap_or_default(),
            },
            Self::RemoveAttr => HtmlRule::RemoveAttr {
                selector,
                name: get(0),
            },
            Self::ReplaceAttr => HtmlRule::ReplaceAttr {
                selector,
                name: get(0),
                find: v.get(1).cloned().unwrap_or_default(),
                replace: v.get(2).cloned().unwrap_or_default(),
            },
            Self::Wrap => HtmlRule::Wrap {
                selector,
                wrapper: get(0),
            },
            Self::AddClass => HtmlRule::AddClass {
                selector,
                class: get(0),
            },
            Self::RemoveClass => HtmlRule::RemoveClass {
                selector,
                class: get(0),
            },
            Self::ReplaceClass => HtmlRule::ReplaceClass {
                selector,
                from: get(0),
                to: get(1),
            },
        };
        rule.validate()?;
        Ok(rule)
    }
}

fn valid_tag(n: &str) -> bool {
    let mut c = n.chars();
    c.next().is_some_and(|f| f.is_ascii_alphabetic())
        && c.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn valid_attr(n: &str) -> bool {
    let mut c = n.chars();
    c.next()
        .is_some_and(|f| f.is_ascii_alphabetic() || f == '_' || f == ':')
        && c.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
}

fn valid_class(n: &str) -> bool {
    !n.is_empty() && !n.contains(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>'))
}

impl HtmlRule {
    pub fn selector(&self) -> &str {
        match self {
            Self::RenameTag { selector, .. }
            | Self::Unwrap { selector }
            | Self::Remove { selector }
            | Self::SetAttr { selector, .. }
            | Self::RemoveAttr { selector, .. }
            | Self::ReplaceAttr { selector, .. }
            | Self::Wrap { selector, .. }
            | Self::AddClass { selector, .. }
            | Self::RemoveClass { selector, .. }
            | Self::ReplaceClass { selector, .. } => selector,
        }
    }

    pub fn validate(&self) -> Result<()> {
        Selector::parse(self.selector())?;
        match self {
            Self::RenameTag { to, .. } if !valid_tag(to) => bail!("\"{to}\" is not a tag name"),
            Self::Wrap { wrapper, .. } if !valid_tag(wrapper) => {
                bail!("\"{wrapper}\" is not a tag name")
            }
            Self::SetAttr { name, .. }
            | Self::RemoveAttr { name, .. }
            | Self::ReplaceAttr { name, .. }
                if !valid_attr(name) =>
            {
                bail!("\"{name}\" is not an attribute name")
            }
            Self::ReplaceAttr { find, .. } if find.is_empty() => bail!("Find is empty"),
            Self::AddClass { class, .. } | Self::RemoveClass { class, .. }
                if !valid_class(class) =>
            {
                bail!("\"{class}\" is not a class name")
            }
            Self::ReplaceClass { from, to, .. } if !valid_class(from) || !valid_class(to) => {
                bail!("the class names must be single words")
            }
            _ => Ok(()),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::RenameTag { selector, to } => format!("Rename {selector} to <{to}>"),
            Self::Unwrap { selector } => format!("Remove the tag of {selector}, keep the content"),
            Self::Remove { selector } => format!("Remove {selector} with its content"),
            Self::SetAttr {
                selector,
                name,
                value,
            } => format!("Set {name}=\"{value}\" on {selector}"),
            Self::RemoveAttr { selector, name } => format!("Remove {name} from {selector}"),
            Self::ReplaceAttr {
                selector,
                name,
                find,
                replace,
            } => format!("In {name} of {selector}: \"{find}\" becomes \"{replace}\""),
            Self::Wrap { selector, wrapper } => format!("Wrap {selector} in <{wrapper}>"),
            Self::AddClass { selector, class } => format!("Add class {class} to {selector}"),
            Self::RemoveClass { selector, class } => {
                format!("Remove class {class} from {selector}")
            }
            Self::ReplaceClass { selector, from, to } => {
                format!("On {selector}: class {from} becomes {to}")
            }
        }
    }
}

struct Edit {
    start: usize,
    end: usize,
    text: String,
    seq: usize,
}

/// Elements that structure the page; rules that remove, unwrap, wrap or rename leave them alone.
fn is_structural(name: &str) -> bool {
    matches!(name, "html" | "head" | "body")
}

fn class_list(tag: &str) -> Vec<String> {
    tag_attr(tag, "class")
        .map(|c| c.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default()
}

fn with_classes(tag: &str, classes: &[String]) -> String {
    if classes.is_empty() {
        crate::markup::remove_tag_attr(tag, "class")
    } else {
        set_tag_attr(tag, "class", &classes.join(" "))
    }
}

/// The new text of a start tag for the attribute and class rules; `None` when the rule does not edit tags.
fn edit_start_tag(rule: &HtmlRule, tag: &str) -> Option<String> {
    Some(match rule {
        HtmlRule::SetAttr { name, value, .. } => {
            if tag_attr(tag, name).as_deref() == Some(value) {
                tag.to_string()
            } else {
                set_tag_attr(tag, name, value)
            }
        }
        HtmlRule::RemoveAttr { name, .. } => crate::markup::remove_tag_attr(tag, name),
        HtmlRule::ReplaceAttr {
            name,
            find,
            replace,
            ..
        } => match tag_attr(tag, name) {
            Some(v) if v.contains(find.as_str()) => {
                set_tag_attr(tag, name, &v.replace(find.as_str(), replace))
            }
            _ => tag.to_string(),
        },
        HtmlRule::AddClass { class, .. } => {
            let mut c = class_list(tag);
            if c.contains(class) {
                tag.to_string()
            } else {
                c.push(class.clone());
                with_classes(tag, &c)
            }
        }
        HtmlRule::RemoveClass { class, .. } => {
            let c = class_list(tag);
            if c.contains(class) {
                let rest: Vec<String> = c.into_iter().filter(|x| x != class).collect();
                with_classes(tag, &rest)
            } else {
                tag.to_string()
            }
        }
        HtmlRule::ReplaceClass { from, to, .. } => {
            let c = class_list(tag);
            if c.contains(from) {
                let mut out: Vec<String> = Vec::new();
                for x in c {
                    let x = if &x == from { to.clone() } else { x };
                    if !out.contains(&x) {
                        out.push(x);
                    }
                }
                with_classes(tag, &out)
            } else {
                tag.to_string()
            }
        }
        _ => return None,
    })
}

/// Applies one rule to a page. Returns the new source and the number of elements changed.
pub fn apply_rule(src: &str, rule: &HtmlRule) -> Result<(String, usize)> {
    rule.validate()?;
    let sel = Selector::parse(rule.selector())?;
    let doc = parse_xml(src)?;
    let mut edits: Vec<Edit> = Vec::new();
    let mut count = 0;
    let mut removed: Vec<std::ops::Range<usize>> = Vec::new();
    for n in doc.descendants().filter(|n| n.is_element()) {
        if !sel.matches(n) {
            continue;
        }
        let name = n.tag_name().name();
        let structural = is_structural(name);
        let r = n.range();
        if removed.iter().any(|x| x.start <= r.start && r.end <= x.end) {
            continue;
        }
        let se = start_tag_end(src, r.start).min(r.end);
        let self_closing = src[r.start..se].ends_with("/>");
        let end_tag = (!self_closing)
            .then(|| src[..r.end].rfind("</").filter(|p| *p >= se))
            .flatten();
        let seq = edits.len();
        let mut push = |start: usize, end: usize, text: String| {
            let seq = seq + edits.len();
            edits.push(Edit {
                start,
                end,
                text,
                seq,
            });
        };
        match rule {
            HtmlRule::RenameTag { to, .. } if !structural => {
                let q = tag_qname(&src[r.start..se]);
                let new = match q.split_once(':') {
                    Some((p, _)) => format!("{p}:{to}"),
                    None => to.clone(),
                };
                if new == q {
                    continue;
                }
                push(r.start + 1, r.start + 1 + q.len(), new.clone());
                if let Some(e) = end_tag {
                    push(e + 2, e + 2 + q.len(), new);
                }
                count += 1;
            }
            HtmlRule::Unwrap { .. } if !structural => {
                push(r.start, se, String::new());
                if let Some(e) = end_tag {
                    push(e, r.end, String::new());
                }
                count += 1;
            }
            HtmlRule::Remove { .. } if !structural => {
                push(r.start, r.end, String::new());
                removed.push(r.clone());
                count += 1;
            }
            HtmlRule::Wrap { wrapper, .. } if !structural => {
                push(r.start, r.start, format!("<{wrapper}>"));
                push(r.end, r.end, format!("</{wrapper}>"));
                count += 1;
            }
            HtmlRule::RenameTag { .. }
            | HtmlRule::Unwrap { .. }
            | HtmlRule::Remove { .. }
            | HtmlRule::Wrap { .. } => {}
            _ => {
                let tag = &src[r.start..se];
                if let Some(new) = edit_start_tag(rule, tag) {
                    if new != tag {
                        push(r.start, se, new);
                        count += 1;
                    }
                }
            }
        }
    }
    if count == 0 {
        return Ok((src.to_string(), 0));
    }
    edits.sort_by_key(|e| std::cmp::Reverse((e.start, e.seq)));
    let mut out = src.to_string();
    for e in edits {
        out.replace_range(e.start..e.end, &e.text);
    }
    Ok((out, count))
}

/// Applies the rules in order. A rule that fails to apply stops the whole run for this page (the error says
/// which rule). Returns the new source and the total number of element changes.
pub fn transform_source(src: &str, rules: &[HtmlRule]) -> Result<(String, usize)> {
    let mut cur = src.to_string();
    let mut total = 0;
    for r in rules {
        let (next, n) = apply_rule(&cur, r)?;
        cur = next;
        total += n;
    }
    if parse_xml(&cur).is_err() {
        bail!("the rules would leave the page broken");
    }
    Ok((cur, total))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HtmlTransformReport {
    /// `(file, element changes)`.
    pub changed: Vec<(String, usize)>,
    /// `(file, why)` for pages that were left alone.
    pub skipped: Vec<(String, String)>,
}

impl HtmlTransformReport {
    pub fn total(&self) -> usize {
        self.changed.iter().map(|(_, n)| n).sum()
    }
}

/// Applies the rules to `paths` (every page of the book when empty). Pages that are not well-formed XHTML are
/// skipped and listed; the rest are written. `dry_run` only reports.
pub fn transform_book(
    book: &mut OpenBook,
    rules: &[HtmlRule],
    paths: &[String],
    dry_run: bool,
) -> HtmlTransformReport {
    let mut report = HtmlTransformReport::default();
    let files: Vec<String> = if paths.is_empty() {
        book.archive
            .names()
            .iter()
            .filter(|n| is_markup_path(n))
            .cloned()
            .collect()
    } else {
        paths.to_vec()
    };
    for f in files {
        let Some(src) = book.archive.get_string(&f) else {
            continue;
        };
        match transform_source(&src, rules) {
            Ok((new, n)) => {
                if n > 0 && new != src {
                    if !dry_run {
                        book.archive.set_string(&f, &new);
                    }
                    report.changed.push((f, n));
                }
            }
            Err(e) => report.skipped.push((f, format!("{e:#}"))),
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    const NS: &str = "xmlns=\"http://www.w3.org/1999/xhtml\"";

    fn page(body: &str) -> String {
        format!("<html {NS}><head><title>T</title></head><body>{body}</body></html>")
    }
    fn run(body: &str, rule: HtmlRule) -> (String, usize) {
        let (s, n) = apply_rule(&page(body), &rule).unwrap();
        let inner = s
            .split_once("<body>")
            .unwrap()
            .1
            .trim_end_matches("</body></html>")
            .to_string();
        (inner, n)
    }
    fn rename(sel: &str, to: &str) -> HtmlRule {
        HtmlRule::RenameTag {
            selector: sel.into(),
            to: to.into(),
        }
    }

    #[test]
    fn rename_changes_both_tags_and_nothing_else() {
        let (s, n) = run(
            "<p>a <b class=\"x\">b &amp; c</b> <b>d</b><br/></p>",
            rename("b", "strong"),
        );
        assert_eq!(n, 2);
        assert_eq!(
            s,
            "<p>a <strong class=\"x\">b &amp; c</strong> <strong>d</strong><br/></p>"
        );
    }

    #[test]
    fn rename_keeps_prefix_and_ignores_structural() {
        let src = "<h:html xmlns:h=\"http://www.w3.org/1999/xhtml\"><h:body><h:p>x</h:p></h:body></h:html>".to_string();
        let (s, n) = apply_rule(&src, &rename("p", "div")).unwrap();
        assert_eq!(n, 1);
        assert!(s.contains("<h:div>x</h:div>"), "{s}");
        let (_, n) = apply_rule(&page("<p>x</p>"), &rename("body", "div")).unwrap();
        assert_eq!(n, 0, "html, head and body are never renamed");
    }

    #[test]
    fn unwrap_keeps_children_and_entities() {
        let (s, n) = run(
            "<p><span class=\"a\">x &lt; <i>y</i></span>z<span/></p>",
            HtmlRule::Unwrap {
                selector: "span".into(),
            },
        );
        assert_eq!(n, 2);
        assert_eq!(s, "<p>x &lt; <i>y</i>z</p>");
    }

    #[test]
    fn unwrap_nested_same_tag() {
        let (s, n) = run(
            "<div><div>a</div>b</div>",
            HtmlRule::Unwrap {
                selector: "div".into(),
            },
        );
        assert_eq!(n, 2);
        assert_eq!(s, "ab");
    }

    #[test]
    fn remove_takes_the_content_and_nested_matches_once() {
        let (s, n) = run(
            "<p>a</p><div class=\"ad\">x<div class=\"ad\">y</div></div><p>b</p>",
            HtmlRule::Remove {
                selector: "div.ad".into(),
            },
        );
        assert_eq!(n, 1);
        assert_eq!(s, "<p>a</p><p>b</p>");
    }

    #[test]
    fn attributes_set_remove_replace() {
        let (s, n) = run(
            "<p class=\"c\">a</p><p lang=\"fr\">b</p>",
            HtmlRule::SetAttr {
                selector: "p".into(),
                name: "lang".into(),
                value: "en".into(),
            },
        );
        assert_eq!(n, 2);
        assert_eq!(s, "<p class=\"c\" lang=\"en\">a</p><p lang=\"en\">b</p>");
        let (s, n) = run(
            "<p style=\"x\" id=\"i\">a</p><p>b</p>",
            HtmlRule::RemoveAttr {
                selector: "p".into(),
                name: "style".into(),
            },
        );
        assert_eq!((s.as_str(), n), ("<p id=\"i\">a</p><p>b</p>", 1));
        let (s, n) = run(
            "<a href=\"a.htm#x\">1</a><a href='b.htm'>2</a><a>3</a>",
            HtmlRule::ReplaceAttr {
                selector: "a".into(),
                name: "href".into(),
                find: ".htm".into(),
                replace: ".xhtml".into(),
            },
        );
        assert_eq!(n, 2);
        assert_eq!(
            s,
            "<a href=\"a.xhtml#x\">1</a><a href=\"b.xhtml\">2</a><a>3</a>"
        );
    }

    #[test]
    fn namespaced_attributes_are_matched_as_written() {
        let src = format!(
            "<html {NS} xmlns:epub=\"http://www.idpf.org/2007/ops\"><body><section epub:type=\"chapter\"><p>x</p></section></body></html>"
        );
        let rule = HtmlRule::SetAttr {
            selector: "//section".into(),
            name: "epub:type".into(),
            value: "bodymatter chapter".into(),
        };
        let (s, n) = apply_rule(&src, &rule).unwrap();
        assert_eq!(n, 1);
        assert!(
            s.contains("<section epub:type=\"bodymatter chapter\">"),
            "{s}"
        );
        let rule = HtmlRule::RemoveAttr {
            selector: "section".into(),
            name: "epub:type".into(),
        };
        assert!(apply_rule(&src, &rule).unwrap().0.contains("<section>"));
    }

    #[test]
    fn attribute_values_with_markup_characters_are_escaped() {
        let (s, _) = run(
            "<p>a</p>",
            HtmlRule::SetAttr {
                selector: "p".into(),
                name: "title".into(),
                value: "a \"q\" & <b>".into(),
            },
        );
        assert!(s.contains("&quot;q&quot;") && s.contains("&amp;"), "{s}");
        assert!(parse_xml(&page(&s)).is_ok());
    }

    #[test]
    fn wrap_surrounds_each_match() {
        let (s, n) = run(
            "<img src=\"a.png\"/><p>x</p><img src=\"b.png\"/>",
            HtmlRule::Wrap {
                selector: "img".into(),
                wrapper: "div".into(),
            },
        );
        assert_eq!(n, 2);
        assert_eq!(
            s,
            "<div><img src=\"a.png\"/></div><p>x</p><div><img src=\"b.png\"/></div>"
        );
        let (s, _) = run(
            "<div><div>a</div></div>",
            HtmlRule::Wrap {
                selector: "div".into(),
                wrapper: "section".into(),
            },
        );
        assert_eq!(
            s,
            "<section><div><section><div>a</div></section></div></section>"
        );
    }

    #[test]
    fn class_rules() {
        let add = |c: &str| HtmlRule::AddClass {
            selector: "p".into(),
            class: c.into(),
        };
        let (s, n) = run(
            "<p>a</p><p class=\"x\">b</p><p class=\"x n\">c</p>",
            add("n"),
        );
        assert_eq!(
            n, 2,
            "a paragraph that has the class already is not counted"
        );
        assert_eq!(
            s,
            "<p class=\"n\">a</p><p class=\"x n\">b</p><p class=\"x n\">c</p>"
        );
        let (s, n) = run(
            "<p class=\"x n\">a</p><p class=\"n\">b</p><p>c</p>",
            HtmlRule::RemoveClass {
                selector: "p".into(),
                class: "n".into(),
            },
        );
        assert_eq!((s.as_str(), n), ("<p class=\"x\">a</p><p>b</p><p>c</p>", 2));
        let (s, n) = run(
            "<p class=\"a b\">a</p><p class=\"b c\">b</p>",
            HtmlRule::ReplaceClass {
                selector: "p".into(),
                from: "a".into(),
                to: "c".into(),
            },
        );
        assert_eq!(
            (s.as_str(), n),
            ("<p class=\"c b\">a</p><p class=\"b c\">b</p>", 1)
        );
    }

    #[test]
    fn selectors_with_ancestors_and_predicates() {
        let (s, n) = run("<div><p>a</p></div><p>b</p>", rename("//div/p", "h3"));
        assert_eq!((s.as_str(), n), ("<div><h3>a</h3></div><p>b</p>", 1));
        let (s, n) = run("<p id=\"k\">a</p><p>b</p>", rename("#k", "h3"));
        assert_eq!((s.as_str(), n), ("<h3 id=\"k\">a</h3><p>b</p>", 1));
    }

    #[test]
    fn rules_run_in_order() {
        let rules = vec![
            rename("b", "strong"),
            HtmlRule::AddClass {
                selector: "strong".into(),
                class: "k".into(),
            },
        ];
        let (s, n) = transform_source(&page("<b>x</b>"), &rules).unwrap();
        assert_eq!(n, 2);
        assert!(s.contains("<strong class=\"k\">x</strong>"));
    }

    #[test]
    fn invalid_rules_are_refused() {
        assert!(HtmlRuleKind::RenameTag.build("b", &["1x".into()]).is_err());
        assert!(HtmlRuleKind::RenameTag.build("", &["i".into()]).is_err());
        assert!(
            HtmlRuleKind::SetAttr
                .build("p", &["a b".into(), "v".into()])
                .is_err()
        );
        assert!(
            HtmlRuleKind::ReplaceAttr
                .build("p", &["href".into(), "".into(), "x".into()])
                .is_err()
        );
        assert!(HtmlRuleKind::AddClass.build("p", &["a b".into()]).is_err());
        assert!(HtmlRuleKind::Wrap.build("p", &["<div".into()]).is_err());
        assert!(HtmlRuleKind::RenameTag.build("b", &["i".into()]).is_ok());
    }

    #[test]
    fn rule_set_round_trips_and_describes() {
        let set = HtmlRuleSet {
            name: "Mine".into(),
            rules: vec![
                rename("b", "strong"),
                HtmlRule::Remove {
                    selector: "div.ad".into(),
                },
            ],
        };
        assert_eq!(HtmlRuleSet::from_json(&set.to_json()), Some(set.clone()));
        assert!(set.rules[0].describe().contains("strong"));
        assert!(HtmlRuleSet::from_json("nonsense").is_none());
        for k in HtmlRuleKind::ALL {
            assert_eq!(HtmlRuleKind::from_id(k.id()), Some(k));
        }
    }

    #[test]
    fn book_run_reports_changed_and_skipped_pages_and_dry_run_writes_nothing() {
        let mut b = crate::testutil::make_book(&["<p><b>x</b></p>", "<p>none</p>"]);
        let broken = "OEBPS/broken.xhtml";
        b.archive
            .set_string(broken, "<html><body><p>unclosed<b></body>");
        let rules = vec![rename("b", "strong")];
        let before = b.archive.clone().to_bytes().unwrap();
        let r = transform_book(&mut b, &rules, &[], true);
        assert_eq!(r.total(), 1);
        assert_eq!(r.changed.len(), 1);
        assert_eq!(r.skipped.len(), 1);
        assert_eq!(r.skipped[0].0, broken);
        assert_eq!(
            b.archive.to_bytes().unwrap(),
            before,
            "dry run changes nothing"
        );
        let r = transform_book(&mut b, &rules, &[], false);
        assert_eq!(r.total(), 1);
        let f = &r.changed[0].0;
        assert!(
            b.archive
                .get_string(f)
                .unwrap()
                .contains("<strong>x</strong>")
        );
        let r = transform_book(&mut b, &rules, &[], false);
        assert_eq!(r.total(), 0, "a second run finds nothing");
    }

    #[test]
    fn short_labels_are_short_and_different() {
        let mut seen = Vec::new();
        for k in HtmlRuleKind::ALL {
            assert!(k.short_label().chars().count() <= 26, "{}", k.short_label());
            assert!(k.label().len() >= k.short_label().len());
            assert!(!seen.contains(&k.short_label()));
            seen.push(k.short_label());
        }
    }
}
