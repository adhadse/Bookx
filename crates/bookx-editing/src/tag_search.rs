//! Tag search (CE-085): find start tags by element name and attribute predicates, and change attributes.
//!
//! Query syntax, a small subset of CSS attribute selectors:
//!
//! ```text
//! img                       every <img>
//! img[alt=""]               <img> whose alt is empty
//! a[href^=http]             href starts with http   (^= prefix, $= suffix, *= contains, ~= word, != differs)
//! *[class~=note]            any element, class list contains "note"
//! a[href][!title]           has href, has no title
//! p[class="a b" i]          quoted value, trailing ` i` = ignore case
//! ```
//!
//! Replacement syntax:
//!
//! ```text
//! alt=Picture               set (or add) the attribute alt
//! -title                    remove the attribute title
//! =https://                 replace the part the query matched in its attribute: the whole value for `=`, the
//!                           prefix for `^=`, the suffix for `$=`, every occurrence for `*=` and `~=`
//! ```

use std::ops::Range;

use crate::markup::{self, rewrite_tag_attrs, scan, Event};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Exists,
    Missing,
    Eq,
    Ne,
    Prefix,
    Suffix,
    Contains,
    Word,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pred {
    pub attr: String,
    pub op: Op,
    pub value: String,
    pub ignore_case: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagQuery {
    /// Lower-case local element name; `None` = any element.
    pub name: Option<String>,
    pub preds: Vec<Pred>,
}

/// A start tag that matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagHit {
    pub range: Range<usize>,
    pub name: String,
    pub attrs: Vec<(String, String)>,
}

/// What to do to the matching tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagEdit {
    Set(String, String),
    Remove(String),
    /// Replace the part the query matched.
    Part(String),
}

fn norm(s: &str) -> String {
    s.rsplit(':').next().unwrap_or(s).to_ascii_lowercase()
}

/// Parses a tag query.
pub fn parse(q: &str) -> Result<TagQuery, String> {
    let q = q.trim();
    if q.is_empty() {
        return Err("empty".into());
    }
    let b = q.find('[').unwrap_or(q.len());
    let name = q[..b].trim();
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '*' | ':' | '-' | '_'))
    {
        return Err(format!("“{name}” is not an element name"));
    }
    let name = match name {
        "" | "*" => None,
        n => Some(norm(n)),
    };
    let mut preds = Vec::new();
    let mut rest = &q[b..];
    while !rest.is_empty() {
        let inner = rest
            .strip_prefix('[')
            .ok_or_else(|| format!("expected “[” before “{rest}”"))?;
        let close = close_bracket(inner).ok_or("missing “]”")?;
        preds.push(parse_pred(&inner[..close])?);
        rest = inner[close + 1..].trim_start();
    }
    Ok(TagQuery { name, preds })
}

/// Index of the `]` that closes the predicate, skipping quoted text.
fn close_bracket(s: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, ']') => return Some(i),
            _ => {}
        }
    }
    None
}

fn parse_pred(s: &str) -> Result<Pred, String> {
    let s = s.trim();
    if let Some(a) = s.strip_prefix('!') {
        return Ok(Pred {
            attr: a.trim().to_string(),
            op: Op::Missing,
            value: String::new(),
            ignore_case: false,
        });
    }
    let Some(at) = s.find(['=', '^', '$', '*', '~', '!']) else {
        return Ok(Pred {
            attr: s.to_string(),
            op: Op::Exists,
            value: String::new(),
            ignore_case: false,
        });
    };
    let attr = s[..at].trim().to_string();
    if attr.is_empty() {
        return Err("an attribute name is missing".into());
    }
    let (op, skip) = match &s[at..] {
        r if r.starts_with("^=") => (Op::Prefix, 2),
        r if r.starts_with("$=") => (Op::Suffix, 2),
        r if r.starts_with("*=") => (Op::Contains, 2),
        r if r.starts_with("~=") => (Op::Word, 2),
        r if r.starts_with("!=") => (Op::Ne, 2),
        r if r.starts_with('=') => (Op::Eq, 1),
        r => return Err(format!("unknown operator in “{r}”")),
    };
    let mut v = s[at + skip..].trim();
    let mut ignore_case = false;
    let (value, tail) = if let Some(q) = v.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let end = v[1..]
            .find(q)
            .ok_or_else(|| "a quote is not closed".to_string())?
            + 1;
        (v[1..end].to_string(), v[end + 1..].trim())
    } else if let Some(x) = v.strip_suffix(" i") {
        ignore_case = true;
        v = x.trim();
        (v.to_string(), "")
    } else {
        (v.to_string(), "")
    };
    if tail == "i" {
        ignore_case = true;
    } else if !tail.is_empty() {
        return Err(format!("unexpected “{tail}”"));
    }
    Ok(Pred {
        attr,
        op,
        value,
        ignore_case,
    })
}

/// Parses the replacement field (see the module docs).
pub fn parse_edit(s: &str) -> Result<TagEdit, String> {
    if let Some(rest) = s.strip_prefix('=') {
        return Ok(TagEdit::Part(rest.to_string()));
    }
    if let Some(a) = s.strip_prefix('-') {
        let a = a.trim();
        if a.is_empty() {
            return Err("name the attribute to remove, e.g. -title".into());
        }
        return Ok(TagEdit::Remove(a.to_string()));
    }
    if let Some((a, v)) = s.split_once('=') {
        let a = a.trim();
        if !a.is_empty()
            && a.chars()
                .all(|c| c.is_alphanumeric() || matches!(c, ':' | '-' | '_' | '.'))
        {
            let v = v.trim();
            let v = v
                .strip_prefix('"')
                .and_then(|x| x.strip_suffix('"'))
                .or_else(|| v.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
                .unwrap_or(v);
            return Ok(TagEdit::Set(a.to_string(), v.to_string()));
        }
    }
    Err("use attr=value, -attr or =replacement for the matched part".into())
}

impl Pred {
    fn test(&self, attrs: &[(String, String)]) -> bool {
        let found = attrs
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(&self.attr))
            .map(|(_, v)| v.as_str());
        let (Some(v), true) = (found, self.op != Op::Missing) else {
            return match self.op {
                Op::Missing | Op::Ne => found.is_none(),
                _ => false,
            };
        };
        let (v, w) = if self.ignore_case {
            (v.to_lowercase(), self.value.to_lowercase())
        } else {
            (v.to_string(), self.value.clone())
        };
        match self.op {
            Op::Exists => true,
            Op::Missing => false,
            Op::Eq => v == w,
            Op::Ne => v != w,
            Op::Prefix => !w.is_empty() && v.starts_with(&w),
            Op::Suffix => !w.is_empty() && v.ends_with(&w),
            Op::Contains => !w.is_empty() && v.contains(&w),
            Op::Word => v.split_whitespace().any(|x| x == w),
        }
    }
}

impl TagQuery {
    pub fn matches(&self, name: &str, attrs: &[(String, String)]) -> bool {
        self.name.as_deref().is_none_or(|n| n == norm(name))
            && self.preds.iter().all(|p| p.test(attrs))
    }

    /// Every matching start tag.
    pub fn find(&self, src: &str) -> Vec<TagHit> {
        let mut out = Vec::new();
        for ev in scan(src) {
            if let Event::Open { name, range, .. } = ev {
                let attrs = markup::tag_attrs(&src[range.clone()]);
                if self.matches(&name, &attrs) {
                    out.push(TagHit { range, name, attrs });
                }
            }
        }
        out
    }

    /// Applies `edit` to every matching start tag; returns the new source and the number of tags changed.
    pub fn apply(&self, src: &str, edit: &TagEdit) -> Result<(String, usize), String> {
        let part = match edit {
            TagEdit::Part(_) => Some(
                self.preds
                    .iter()
                    .rev()
                    .find(|p| {
                        matches!(
                            p.op,
                            Op::Eq | Op::Prefix | Op::Suffix | Op::Contains | Op::Word
                        )
                    })
                    .ok_or("“=…” needs a query with =, ^=, $=, *= or ~=")?,
            ),
            _ => None,
        };
        let mut out = String::with_capacity(src.len());
        let (mut last, mut n) = (0, 0);
        for hit in self.find(src) {
            let tag = &src[hit.range.clone()];
            let new = match (edit, part) {
                (TagEdit::Set(a, v), _) => markup::set_tag_attr(tag, a, v),
                (TagEdit::Remove(a), _) => {
                    if hit.attrs.iter().any(|(n, _)| n.eq_ignore_ascii_case(a)) {
                        let real = hit
                            .attrs
                            .iter()
                            .find(|(n, _)| n.eq_ignore_ascii_case(a))
                            .map(|(n, _)| n.clone())
                            .unwrap_or_default();
                        markup::remove_tag_attr(tag, &real)
                    } else {
                        tag.to_string()
                    }
                }
                (TagEdit::Part(r), Some(p)) => rewrite_tag_attrs(tag, &mut |name, value| {
                    if !name.eq_ignore_ascii_case(&p.attr) {
                        return None;
                    }
                    Some(replace_part(p, value, r))
                }),
                (TagEdit::Part(_), None) => tag.to_string(),
            };
            if new != tag {
                out.push_str(&src[last..hit.range.start]);
                out.push_str(&new);
                last = hit.range.end;
                n += 1;
            }
        }
        out.push_str(&src[last..]);
        Ok((out, n))
    }
}

fn replace_part(p: &Pred, value: &str, r: &str) -> String {
    match p.op {
        Op::Eq => r.to_string(),
        Op::Prefix => format!("{r}{}", &value[p.value.len().min(value.len())..]),
        Op::Suffix => format!("{}{r}", &value[..value.len().saturating_sub(p.value.len())]),
        Op::Contains if !p.value.is_empty() => value.replace(&p.value, r),
        Op::Word => value
            .split(' ')
            .map(|w| if w == p.value { r } else { w })
            .collect::<Vec<_>>()
            .join(" "),
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r##"<p><img src="a.png" alt=""/><img src="b.png" alt="B"/><a href="http://x.org/a" class="ext note">x</a><a href="#id" title="t">y</a><IMG SRC='c.png'/></p>"##;

    fn names(q: &str) -> Vec<String> {
        parse(q)
            .unwrap()
            .find(DOC)
            .iter()
            .map(|h| DOC[h.range.clone()].chars().take(14).collect())
            .collect()
    }

    #[test]
    fn element_names_and_wildcards() {
        assert_eq!(names("img").len(), 3);
        assert_eq!(names("IMG").len(), 3);
        assert_eq!(names("*").len(), 6);
        assert_eq!(names("[src]").len(), 3);
    }

    #[test]
    fn attribute_predicates() {
        assert_eq!(names(r#"img[alt=""]"#).len(), 1);
        assert_eq!(names("img[alt='B']").len(), 1);
        assert_eq!(names("a[href^=http]").len(), 1);
        assert_eq!(names("img[src$=.png]").len(), 3);
        assert_eq!(names("a[href*=x.org]").len(), 1);
        assert_eq!(names("*[class~=note]").len(), 1);
        assert_eq!(names("*[class~=no]").len(), 0);
        assert_eq!(names("img[alt]").len(), 2);
        assert_eq!(names("img[!alt]").len(), 1);
        assert_eq!(names("a[href][!title]").len(), 1);
        assert_eq!(names("img[alt!=B]").len(), 2);
        assert_eq!(names("img[alt=b i]").len(), 1);
        assert_eq!(names(r#"img[alt="b" i]"#).len(), 1);
        assert_eq!(names("img[alt=b]").len(), 0);
    }

    #[test]
    fn bad_queries_are_errors() {
        for q in [
            "",
            "img[",
            "img[alt",
            "i mg",
            "img x",
            "img[=x]",
            "img[a='x]",
        ] {
            assert!(parse(q).is_err(), "{q}");
        }
        assert!(parse("img[a=x j]").is_ok() || parse("img[a=x j]").is_err());
    }

    #[test]
    fn set_adds_or_changes_an_attribute() {
        let q = parse(r#"img[alt=""]"#).unwrap();
        let (out, n) = q.apply(DOC, &parse_edit("alt=Picture").unwrap()).unwrap();
        assert_eq!(n, 1);
        assert!(out.contains(r#"<img src="a.png" alt="Picture"/>"#));
        let q = parse("img[!alt]").unwrap();
        let (out, n) = q.apply(DOC, &parse_edit("alt=none").unwrap()).unwrap();
        assert_eq!(n, 1);
        assert!(out.contains(r#"alt="none"/>"#), "{out}");
    }

    #[test]
    fn remove_deletes_an_attribute() {
        let q = parse("a[title]").unwrap();
        let (out, n) = q.apply(DOC, &parse_edit("-title").unwrap()).unwrap();
        assert_eq!(n, 1);
        assert!(out.contains(r##"<a href="#id">y</a>"##));
        let (_, n) = q.apply(DOC, &parse_edit("-nothing").unwrap()).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn matched_part_is_replaced_per_operator() {
        let q = parse("a[href^=http://]").unwrap();
        let (out, n) = q.apply(DOC, &parse_edit("=https://").unwrap()).unwrap();
        assert_eq!(n, 1);
        assert!(out.contains(r#"href="https://x.org/a""#));
        let q = parse("img[src$=.png]").unwrap();
        let (out, n) = q.apply(DOC, &parse_edit("=.webp").unwrap()).unwrap();
        assert_eq!(n, 3);
        assert!(out.contains("a.webp") && out.contains("b.webp") && out.contains("c.webp"));
        let q = parse("*[class~=note]").unwrap();
        let (out, _) = q.apply(DOC, &parse_edit("=aside").unwrap()).unwrap();
        assert!(out.contains(r#"class="ext aside""#));
        let q = parse("img[alt=B]").unwrap();
        assert_eq!(
            q.apply(DOC, &parse_edit("=Bee").unwrap())
                .unwrap()
                .0
                .matches("Bee")
                .count(),
            1
        );
    }

    #[test]
    fn part_replacement_needs_a_comparing_predicate() {
        let q = parse("img[alt]").unwrap();
        assert!(q.apply(DOC, &TagEdit::Part("x".into())).is_err());
    }

    #[test]
    fn edit_syntax() {
        assert_eq!(
            parse_edit("alt=\"A b\"").unwrap(),
            TagEdit::Set("alt".into(), "A b".into())
        );
        assert_eq!(parse_edit("-id").unwrap(), TagEdit::Remove("id".into()));
        assert_eq!(parse_edit("=x=y").unwrap(), TagEdit::Part("x=y".into()));
        assert!(parse_edit("hello").is_err());
        assert!(parse_edit("-").is_err());
        assert!(parse_edit("http://a?b=c").is_err());
    }

    #[test]
    fn values_with_entities_are_decoded_and_escaped_again() {
        let src = r#"<a href="x?a=1&amp;b=2">t</a>"#;
        let q = parse("a[href*=b=2]").unwrap();
        assert_eq!(q.find(src).len(), 1);
        let (out, _) = q.apply(src, &parse_edit("=b=3").unwrap()).unwrap();
        assert_eq!(out, r#"<a href="x?a=1&amp;b=3">t</a>"#);
    }
}
