//! Selector parsing and matching on `scraper` elements.

use scraper::ElementRef;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AttrOp {
    Exists,
    Equals,
    Includes,
    DashMatch,
    Prefix,
    Suffix,
    Substring,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AttrSel {
    name: String,
    op: AttrOp,
    value: String,
    case_insensitive: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Pseudo {
    FirstChild,
    LastChild,
    OnlyChild,
    FirstOfType,
    LastOfType,
    OnlyOfType,
    Root,
    Empty,
    Link,
    NthChild(i32, i32),
    NthLastChild(i32, i32),
    NthOfType(i32, i32),
    Not(Box<Compound>),
    /// Valid but never matches here (`:hover`, `::before`, ...).
    Never,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Compound {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attrs: Vec<AttrSel>,
    pseudos: Vec<Pseudo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Comb {
    Descendant,
    Child,
    Adjacent,
    Sibling,
}

/// A complex selector, e.g. `div > p.note:first-child`.
#[derive(Debug, Clone, PartialEq)]
pub struct Selector {
    compounds: Vec<Compound>,
    /// `combs[i]` joins `compounds[i]` and `compounds[i + 1]`.
    combs: Vec<Comb>,
    /// Targets the `::first-letter` pseudo-element of the matched element.
    pub first_letter: bool,
    specificity: u32,
}

impl Selector {
    /// Specificity packed as `ids << 16 | classes << 8 | types`.
    pub fn specificity(&self) -> u32 {
        self.specificity
    }

    /// Whether the selector (ignoring a trailing `::first-letter`) matches the element.
    pub fn matches(&self, el: ElementRef<'_>) -> bool {
        match_from(self, self.compounds.len() - 1, el)
    }
}

fn parent_el(el: ElementRef<'_>) -> Option<ElementRef<'_>> {
    el.parent().and_then(ElementRef::wrap)
}

fn prev_sibling_el(el: ElementRef<'_>) -> Option<ElementRef<'_>> {
    el.prev_siblings().find_map(ElementRef::wrap)
}

fn match_from(sel: &Selector, idx: usize, el: ElementRef<'_>) -> bool {
    if !match_compound(&sel.compounds[idx], el) {
        return false;
    }
    if idx == 0 {
        return true;
    }
    match sel.combs[idx - 1] {
        Comb::Child => parent_el(el).is_some_and(|p| match_from(sel, idx - 1, p)),
        Comb::Descendant => {
            let mut cur = parent_el(el);
            while let Some(p) = cur {
                if match_from(sel, idx - 1, p) {
                    return true;
                }
                cur = parent_el(p);
            }
            false
        }
        Comb::Adjacent => prev_sibling_el(el).is_some_and(|p| match_from(sel, idx - 1, p)),
        Comb::Sibling => {
            let mut cur = prev_sibling_el(el);
            while let Some(p) = cur {
                if match_from(sel, idx - 1, p) {
                    return true;
                }
                cur = prev_sibling_el(p);
            }
            false
        }
    }
}

fn element_siblings(el: ElementRef<'_>) -> Vec<ElementRef<'_>> {
    match el.parent() {
        Some(p) => p.children().filter_map(ElementRef::wrap).collect(),
        None => vec![el],
    }
}

fn nth_matches(a: i32, b: i32, pos: i32) -> bool {
    // pos = a*n + b for some n >= 0
    if a == 0 {
        return pos == b;
    }
    let d = pos - b;
    d % a == 0 && d / a >= 0
}

fn match_compound(c: &Compound, el: ElementRef<'_>) -> bool {
    let v = el.value();
    if let Some(t) = &c.tag {
        if !v.name().eq_ignore_ascii_case(t) {
            return false;
        }
    }
    if let Some(id) = &c.id {
        if v.id() != Some(id.as_str()) {
            return false;
        }
    }
    for cl in &c.classes {
        if !v.classes().any(|x| x == cl) {
            return false;
        }
    }
    for a in &c.attrs {
        let Some(actual) = v.attr(&a.name) else {
            return false;
        };
        let (actual, want) = if a.case_insensitive {
            (actual.to_lowercase(), a.value.to_lowercase())
        } else {
            (actual.to_string(), a.value.clone())
        };
        let ok = match a.op {
            AttrOp::Exists => true,
            AttrOp::Equals => actual == want,
            AttrOp::Includes => !want.is_empty() && actual.split_whitespace().any(|w| w == want),
            AttrOp::DashMatch => actual == want || actual.starts_with(&format!("{want}-")),
            AttrOp::Prefix => !want.is_empty() && actual.starts_with(&want),
            AttrOp::Suffix => !want.is_empty() && actual.ends_with(&want),
            AttrOp::Substring => !want.is_empty() && actual.contains(&want),
        };
        if !ok {
            return false;
        }
    }
    for p in &c.pseudos {
        let ok = match p {
            Pseudo::Never => false,
            Pseudo::Root => parent_el(el).is_none(),
            Pseudo::Empty => el.children().all(|n| match n.value() {
                scraper::Node::Element(_) => false,
                scraper::Node::Text(t) => t.is_empty(),
                _ => true,
            }),
            Pseudo::Link => v.name().eq_ignore_ascii_case("a") && v.attr("href").is_some(),
            Pseudo::Not(inner) => !match_compound(inner, el),
            Pseudo::FirstChild
            | Pseudo::LastChild
            | Pseudo::OnlyChild
            | Pseudo::NthChild(..)
            | Pseudo::NthLastChild(..) => {
                let sibs = element_siblings(el);
                let i = sibs.iter().position(|s| s.id() == el.id()).unwrap_or(0) as i32;
                let n = sibs.len() as i32;
                match p {
                    Pseudo::FirstChild => i == 0,
                    Pseudo::LastChild => i == n - 1,
                    Pseudo::OnlyChild => n == 1,
                    Pseudo::NthChild(a, b) => nth_matches(*a, *b, i + 1),
                    Pseudo::NthLastChild(a, b) => nth_matches(*a, *b, n - i),
                    _ => false,
                }
            }
            Pseudo::FirstOfType
            | Pseudo::LastOfType
            | Pseudo::OnlyOfType
            | Pseudo::NthOfType(..) => {
                let sibs: Vec<_> = element_siblings(el)
                    .into_iter()
                    .filter(|s| s.value().name() == v.name())
                    .collect();
                let i = sibs.iter().position(|s| s.id() == el.id()).unwrap_or(0) as i32;
                let n = sibs.len() as i32;
                match p {
                    Pseudo::FirstOfType => i == 0,
                    Pseudo::LastOfType => i == n - 1,
                    Pseudo::OnlyOfType => n == 1,
                    Pseudo::NthOfType(a, b) => nth_matches(*a, *b, i + 1),
                    _ => false,
                }
            }
        };
        if !ok {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------- parsing

struct P {
    s: Vec<char>,
    i: usize,
    /// How many `:not(` we are inside.
    depth: u8,
}

/// `:not(:not(:not(...)))` deeper than this is refused: the parser recurses once per level.
const MAX_NOT_DEPTH: u8 = 8;

impl P {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn skip_ws(&mut self) -> bool {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_whitespace()) {
            self.i += 1;
        }
        self.i > start
    }

    fn ident(&mut self) -> Option<String> {
        let mut out = String::new();
        let mut first = true;
        loop {
            match self.peek() {
                Some('\\') => {
                    self.i += 1;
                    let mut hex = String::new();
                    while hex.len() < 6 && self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                        hex.push(self.peek()?);
                        self.i += 1;
                    }
                    if hex.is_empty() {
                        out.push(self.peek()?);
                        self.i += 1;
                    } else {
                        out.push(
                            u32::from_str_radix(&hex, 16)
                                .ok()
                                .and_then(char::from_u32)
                                .unwrap_or('\u{fffd}'),
                        );
                        if self.peek().is_some_and(|c| c.is_whitespace()) {
                            self.i += 1;
                        }
                    }
                }
                Some(c) if c.is_alphanumeric() || c == '_' || c == '-' || !c.is_ascii() => {
                    if first && c.is_ascii_digit() {
                        return None;
                    }
                    out.push(c);
                    self.i += 1;
                }
                _ => break,
            }
            first = false;
        }
        (!out.is_empty()).then_some(out)
    }

    fn string_or_ident(&mut self) -> Option<String> {
        match self.peek()? {
            q @ ('"' | '\'') => {
                self.i += 1;
                let mut out = String::new();
                loop {
                    let c = self.peek()?;
                    self.i += 1;
                    if c == q {
                        return Some(out);
                    }
                    if c == '\\' {
                        out.push(self.peek()?);
                        self.i += 1;
                    } else {
                        out.push(c);
                    }
                }
            }
            _ => self.ident(),
        }
    }
}

pub(crate) fn strip_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        match rest[i + 2..].find("*/") {
            Some(j) => rest = &rest[i + 2 + j + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

fn parse_nth(arg: &str) -> Option<(i32, i32)> {
    let s: String = arg
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    match s.as_str() {
        "odd" => return Some((2, 1)),
        "even" => return Some((2, 0)),
        "" => return None,
        _ => {}
    }
    if let Some(npos) = s.find('n') {
        let a = match &s[..npos] {
            "" | "+" => 1,
            "-" => -1,
            t => t.parse().ok()?,
        };
        let rest = &s[npos + 1..];
        let b = if rest.is_empty() {
            0
        } else {
            rest.parse::<i32>().ok()?
        };
        if !rest.is_empty() && !rest.starts_with(['+', '-']) {
            return None;
        }
        Some((a, b))
    } else {
        Some((0, s.parse().ok()?))
    }
}

fn parse_compound(p: &mut P, spec: &mut [u32; 3], first_letter: &mut bool) -> Option<Compound> {
    let mut c = Compound::default();
    let mut any = false;
    if p.eat('*') {
        any = true;
    } else if let Some(name) = {
        let save = p.i;
        let n = p.ident();
        if n.is_none() {
            p.i = save;
        }
        n
    } {
        if p.peek() == Some('|') {
            return None;
        }
        c.tag = Some(name.to_ascii_lowercase());
        spec[2] += 1;
        any = true;
    }
    loop {
        match p.peek() {
            Some('#') => {
                p.i += 1;
                c.id = Some(p.ident()?);
                spec[0] += 1;
            }
            Some('.') => {
                p.i += 1;
                c.classes.push(p.ident()?);
                spec[1] += 1;
            }
            Some('[') => {
                p.i += 1;
                p.skip_ws();
                let name = p.ident()?.to_ascii_lowercase();
                p.skip_ws();
                let (op, value) = if p.eat(']') {
                    c.attrs.push(AttrSel {
                        name,
                        op: AttrOp::Exists,
                        value: String::new(),
                        case_insensitive: false,
                    });
                    spec[1] += 1;
                    any = true;
                    continue;
                } else {
                    let op = match p.peek()? {
                        '=' => {
                            p.i += 1;
                            AttrOp::Equals
                        }
                        '~' | '|' | '^' | '$' | '*' => {
                            let ch = p.peek()?;
                            p.i += 1;
                            if !p.eat('=') {
                                return None;
                            }
                            match ch {
                                '~' => AttrOp::Includes,
                                '|' => AttrOp::DashMatch,
                                '^' => AttrOp::Prefix,
                                '$' => AttrOp::Suffix,
                                _ => AttrOp::Substring,
                            }
                        }
                        _ => return None,
                    };
                    p.skip_ws();
                    (op, p.string_or_ident()?)
                };
                p.skip_ws();
                let mut ci = false;
                if matches!(p.peek(), Some('i' | 'I')) {
                    p.i += 1;
                    ci = true;
                    p.skip_ws();
                } else if matches!(p.peek(), Some('s' | 'S')) {
                    p.i += 1;
                    p.skip_ws();
                }
                if !p.eat(']') {
                    return None;
                }
                c.attrs.push(AttrSel {
                    name,
                    op,
                    value,
                    case_insensitive: ci,
                });
                spec[1] += 1;
            }
            Some(':') => {
                p.i += 1;
                let double = p.eat(':');
                let name = p.ident()?.to_ascii_lowercase();
                let arg = if p.eat('(') {
                    let mut depth = 1;
                    let start = p.i;
                    while let Some(ch) = p.peek() {
                        match ch {
                            '(' => depth += 1,
                            ')' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        p.i += 1;
                    }
                    if depth != 0 {
                        return None;
                    }
                    let a: String = p.s[start..p.i].iter().collect();
                    p.i += 1;
                    Some(a)
                } else {
                    None
                };
                if double
                    || matches!(
                        name.as_str(),
                        "first-line" | "first-letter" | "before" | "after"
                    ) && arg.is_none()
                {
                    // pseudo-element (the legacy single-colon forms are accepted)
                    match name.as_str() {
                        "first-letter" => {
                            *first_letter = true;
                            spec[2] += 1;
                        }
                        "before" | "after" | "first-line" | "marker" | "selection"
                        | "placeholder" | "backdrop" => {
                            c.pseudos.push(Pseudo::Never);
                            spec[2] += 1;
                        }
                        _ => return None,
                    }
                    any = true;
                    continue;
                }
                let pseudo = match (name.as_str(), arg.as_deref()) {
                    ("first-child", None) => Pseudo::FirstChild,
                    ("last-child", None) => Pseudo::LastChild,
                    ("only-child", None) => Pseudo::OnlyChild,
                    ("first-of-type", None) => Pseudo::FirstOfType,
                    ("last-of-type", None) => Pseudo::LastOfType,
                    ("only-of-type", None) => Pseudo::OnlyOfType,
                    ("root", None) => Pseudo::Root,
                    ("empty", None) => Pseudo::Empty,
                    ("link", None) | ("any-link", None) => Pseudo::Link,
                    (
                        "hover" | "active" | "focus" | "visited" | "target" | "focus-within"
                        | "focus-visible" | "checked" | "disabled" | "enabled",
                        None,
                    ) => Pseudo::Never,
                    ("nth-child", Some(a)) => {
                        let (x, y) = parse_nth(a)?;
                        Pseudo::NthChild(x, y)
                    }
                    ("nth-last-child", Some(a)) => {
                        let (x, y) = parse_nth(a)?;
                        Pseudo::NthLastChild(x, y)
                    }
                    ("nth-of-type", Some(a)) => {
                        let (x, y) = parse_nth(a)?;
                        Pseudo::NthOfType(x, y)
                    }
                    ("not", Some(a)) => {
                        if p.depth >= MAX_NOT_DEPTH {
                            return None;
                        }
                        let mut inner = P {
                            s: a.trim().chars().collect(),
                            i: 0,
                            depth: p.depth + 1,
                        };
                        let mut ispec = [0u32; 3];
                        let mut fl = false;
                        let comp = parse_compound(&mut inner, &mut ispec, &mut fl)?;
                        if inner.i != inner.s.len() || fl {
                            return None;
                        }
                        for k in 0..3 {
                            spec[k] += ispec[k];
                        }
                        c.pseudos.push(Pseudo::Not(Box::new(comp)));
                        any = true;
                        continue;
                    }
                    _ => return None,
                };
                spec[1] += 1;
                c.pseudos.push(pseudo);
            }
            _ => break,
        }
        any = true;
    }
    any.then_some(c)
}

fn parse_one(text: &str) -> Option<Selector> {
    let mut p = P {
        s: text.trim().chars().collect(),
        i: 0,
        depth: 0,
    };
    let mut spec = [0u32; 3];
    let mut first_letter = false;
    let mut compounds = Vec::new();
    let mut combs = Vec::new();
    loop {
        compounds.push(parse_compound(&mut p, &mut spec, &mut first_letter)?);
        if first_letter && p.peek().is_some() {
            return None;
        }
        let had_ws = p.skip_ws();
        match p.peek() {
            None => break,
            Some(ch @ ('>' | '+' | '~')) => {
                p.i += 1;
                p.skip_ws();
                combs.push(match ch {
                    '>' => Comb::Child,
                    '+' => Comb::Adjacent,
                    _ => Comb::Sibling,
                });
            }
            Some(_) if had_ws => combs.push(Comb::Descendant),
            Some(_) => return None,
        }
    }
    let specificity = spec[0].min(255) << 16 | spec[1].min(255) << 8 | spec[2].min(255);
    Some(Selector {
        compounds,
        combs,
        first_letter,
        specificity,
    })
}

/// Parse a comma separated selector list. Returns `None` if any selector is invalid (the whole rule is dropped).
pub fn parse_selector_list(text: &str) -> Option<Vec<Selector>> {
    let text = strip_comments(text);
    let mut parts = Vec::new();
    let (mut depth, mut quote, mut start) = (0i32, None::<char>, 0usize);
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            Some(q) => {
                if c == '\\' {
                    i += 1;
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '\\' => i += 1,
                '(' | '[' => depth += 1,
                ')' | ']' => depth -= 1,
                ',' if depth == 0 => {
                    parts.push(chars[start..i].iter().collect::<String>());
                    start = i + 1;
                }
                _ => {}
            },
        }
        i += 1;
    }
    parts.push(chars[start.min(chars.len())..].iter().collect::<String>());
    let mut out = Vec::new();
    for part in parts {
        out.push(parse_one(&part)?);
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scraper::{Html, Selector as ScraperSel};

    fn doc() -> Html {
        Html::parse_document(
            r#"<html lang="en"><body class="b">
            <div id="main" class="wrap big" data-k="a-b c">
              <p class="intro first">One</p>
              <p>Two <em>em</em></p>
              <span>s</span>
              <p id="last" lang="en-GB" title="Hello World">Three</p>
            </div>
            <ul><li>a</li><li>b</li><li>c</li><li>d</li></ul>
            <a href="x">link</a><a>anchor</a>
            <h1></h1>
            </body></html>"#,
        )
    }

    fn matched(css: &str, html: &Html) -> Vec<String> {
        let sels = parse_selector_list(css).unwrap_or_else(|| panic!("{css} did not parse"));
        let all = ScraperSel::parse("*").unwrap();
        html.select(&all)
            .filter(|el| sels.iter().any(|s| !s.first_letter && s.matches(*el)))
            .map(|el| {
                let v = el.value();
                match v.id() {
                    Some(id) => format!("{}#{}", v.name(), id),
                    None => format!(
                        "{}:{}",
                        v.name(),
                        el.text()
                            .collect::<String>()
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                            .chars()
                            .take(5)
                            .collect::<String>()
                    ),
                }
            })
            .collect()
    }

    #[test]
    fn type_class_id() {
        let d = doc();
        assert_eq!(matched("p", &d), vec!["p:One", "p:Two e", "p#last"]);
        assert_eq!(matched("P", &d), vec!["p:One", "p:Two e", "p#last"]);
        assert_eq!(matched(".intro", &d), vec!["p:One"]);
        assert_eq!(matched("p.intro.first", &d), vec!["p:One"]);
        assert_eq!(matched(".first.nope", &d), Vec::<String>::new());
        assert_eq!(matched("#last", &d), vec!["p#last"]);
        assert_eq!(matched("div#main.wrap", &d), vec!["div#main"]);
        assert_eq!(matched("span", &d), vec!["span:s"]);
        assert_eq!(matched("#main, h1", &d), vec!["div#main", "h1:"]);
    }

    #[test]
    fn universal_and_combinators() {
        let d = doc();
        assert_eq!(matched("body > div", &d), vec!["div#main"]);
        assert_eq!(
            matched("body div p", &d),
            vec!["p:One", "p:Two e", "p#last"]
        );
        assert_eq!(matched("div > em", &d), Vec::<String>::new());
        assert_eq!(matched("div em", &d), vec!["em:em"]);
        assert_eq!(matched("p + span", &d), vec!["span:s"]);
        assert_eq!(matched("span + p", &d), vec!["p#last"]);
        assert_eq!(matched("p.intro ~ p", &d), vec!["p:Two e", "p#last"]);
        assert_eq!(matched("p~span", &d), vec!["span:s"]);
        assert_eq!(matched("div>p+p", &d), vec!["p:Two e"]);
        assert_eq!(matched("ul *", &d).len(), 4);
        assert_eq!(matched("li li", &d), Vec::<String>::new());
        // backtracking: the nearest ancestor does not match, a farther one does
        assert_eq!(matched("body.b em", &d), vec!["em:em"]);
        assert_eq!(matched("div.wrap p em", &d), vec!["em:em"]);
    }

    #[test]
    fn attribute_selectors() {
        let d = doc();
        assert_eq!(matched("[title]", &d), vec!["p#last"]);
        assert_eq!(matched("[title=\"Hello World\"]", &d), vec!["p#last"]);
        assert_eq!(matched("[title='Hello World']", &d), vec!["p#last"]);
        assert_eq!(matched("[title=Hello]", &d), Vec::<String>::new());
        assert_eq!(matched("[title~=Hello]", &d), vec!["p#last"]);
        assert_eq!(matched("[title~=Hell]", &d), Vec::<String>::new());
        assert_eq!(matched("[lang|=en]", &d).len(), 2, "html and p#last");
        assert_eq!(matched("[title^=Hel]", &d), vec!["p#last"]);
        assert_eq!(matched("[title$=World]", &d), vec!["p#last"]);
        assert!(parse_selector_list("[title*=lo W]").is_none());
        assert_eq!(matched("[title*='lo W']", &d), vec!["p#last"]);
        assert_eq!(matched("[title=\"hello world\" i]", &d), vec!["p#last"]);
        assert_eq!(matched("[title=\"hello world\"]", &d), Vec::<String>::new());
        assert_eq!(matched("[DATA-K]", &d), vec!["div#main"]);
        assert_eq!(matched("[data-k~=c]", &d), vec!["div#main"]);
        assert_eq!(matched("a[href]", &d), vec!["a:link"]);
        assert_eq!(matched("[ title ]", &d), vec!["p#last"]);
        assert_eq!(
            matched("[title^='']", &d),
            Vec::<String>::new(),
            "empty value never matches prefix"
        );
    }

    #[test]
    fn structural_pseudos() {
        let d = doc();
        assert_eq!(matched("li:first-child", &d), vec!["li:a"]);
        assert_eq!(matched("li:last-child", &d), vec!["li:d"]);
        assert_eq!(matched("p:first-child", &d), vec!["p:One"]);
        assert_eq!(matched("p:first-of-type", &d), vec!["p:One"]);
        assert_eq!(matched("p:last-of-type", &d), vec!["p#last"]);
        assert_eq!(matched("span:only-child", &d), Vec::<String>::new());
        assert_eq!(matched("span:only-of-type", &d), vec!["span:s"]);
        assert_eq!(matched("em:only-child", &d), vec!["em:em"]);
        assert_eq!(matched("li:nth-child(2)", &d), vec!["li:b"]);
        assert_eq!(matched("li:nth-child(odd)", &d), vec!["li:a", "li:c"]);
        assert_eq!(matched("li:nth-child(even)", &d), vec!["li:b", "li:d"]);
        assert_eq!(matched("li:nth-child(2n+1)", &d), vec!["li:a", "li:c"]);
        assert_eq!(matched("li:nth-child(-n+2)", &d), vec!["li:a", "li:b"]);
        assert_eq!(matched("li:nth-child(n+3)", &d), vec!["li:c", "li:d"]);
        assert_eq!(matched("li:nth-child( 3n )", &d), vec!["li:c"]);
        assert_eq!(matched("li:nth-last-child(1)", &d), vec!["li:d"]);
        assert_eq!(matched("li:nth-of-type(2)", &d), vec!["li:b"]);
        assert_eq!(matched("html:root", &d), vec!["html:One T"]);
        assert_eq!(matched("h1:empty", &d), vec!["h1:"]);
        assert_eq!(matched("p:empty", &d), Vec::<String>::new());
        assert_eq!(matched("a:link", &d), vec!["a:link"]);
        assert_eq!(matched("p:not(.intro)", &d), vec!["p:Two e", "p#last"]);
        assert_eq!(matched("p:not(#last):not(.intro)", &d), vec!["p:Two e"]);
        assert_eq!(
            matched("li:not(:first-child)", &d),
            vec!["li:b", "li:c", "li:d"]
        );
    }

    #[test]
    fn never_matching_pseudos_are_valid_but_inert() {
        let d = doc();
        for css in [
            "a:hover",
            "p:active",
            "a:visited",
            "p::before",
            "p:after",
            "p::selection",
            "p::marker",
            "a:focus",
        ] {
            assert!(parse_selector_list(css).is_some(), "{css}");
            assert!(matched(css, &d).is_empty(), "{css}");
        }
        // inside a list the other selectors still work
        assert_eq!(matched("a:hover, #last", &d), vec!["p#last"]);
    }

    #[test]
    fn first_letter_flag() {
        let s = parse_selector_list("p::first-letter").unwrap();
        assert!(s[0].first_letter);
        let d = doc();
        let p = d.select(&ScraperSel::parse("p").unwrap()).next().unwrap();
        assert!(s[0].matches(p));
        let s = parse_selector_list("p:first-letter").unwrap();
        assert!(s[0].first_letter, "legacy single-colon syntax");
        assert!(!parse_selector_list("p").unwrap()[0].first_letter);
        assert!(
            parse_selector_list("p::first-letter span").is_none(),
            "pseudo-element must be last"
        );
    }

    #[test]
    fn specificity_values() {
        let sp = |s: &str| parse_selector_list(s).unwrap()[0].specificity();
        assert_eq!(sp("*"), 0);
        assert_eq!(sp("p"), 1);
        assert_eq!(sp("div p"), 2);
        assert_eq!(sp(".a"), 1 << 8);
        assert_eq!(sp("p.a"), 1 << 8 | 1);
        assert_eq!(sp("#a"), 1 << 16);
        assert_eq!(sp("#a .b p"), 1 << 16 | 1 << 8 | 1);
        assert_eq!(sp("[x]"), 1 << 8);
        assert_eq!(sp("a:first-child"), 1 << 8 | 1);
        assert_eq!(sp("p::first-letter"), 2);
        assert_eq!(sp("p:not(.a)"), 1 << 8 | 1);
        assert_eq!(sp("p:not(#a)"), 1 << 16 | 1);
        assert!(sp("#a") > sp(".a.b.c.d.e.f"));
        assert!(sp("p.a") > sp("p"));
        assert_eq!(sp(&".a".repeat(300)), 255 << 8, "capped");
    }

    #[test]
    fn selector_list_splitting() {
        assert_eq!(parse_selector_list("a, b ,c").unwrap().len(), 3);
        assert_eq!(parse_selector_list("a[x=\"1,2\"], b").unwrap().len(), 2);
        assert_eq!(
            parse_selector_list("a:not(.x, .y)"),
            None,
            "selector lists inside :not are unsupported"
        );
        assert_eq!(parse_selector_list("p /* c, d */ , q").unwrap().len(), 2);
        assert_eq!(parse_selector_list("p /* unterminated").unwrap().len(), 1);
    }

    #[test]
    fn deeply_nested_not_is_refused_instead_of_overflowing_the_stack() {
        let deep = format!("{}a{}", ":not(".repeat(100_000), ")".repeat(100_000));
        assert!(parse_selector_list(&format!("p{deep}")).is_none());
        let ok = ":not(.a)".to_string();
        assert!(parse_selector_list(&format!("p{ok}")).is_some());
        let eight = format!("{}.a{}", ":not(".repeat(8), ")".repeat(8));
        assert!(parse_selector_list(&format!("p{eight}")).is_some());
        let nine = format!("{}.a{}", ":not(".repeat(9), ")".repeat(9));
        assert!(parse_selector_list(&format!("p{nine}")).is_none());
    }

    #[test]
    fn invalid_selectors_rejected() {
        for bad in [
            "",
            " ",
            ",",
            "p,",
            ",p",
            "p > ",
            "> p",
            "p >> q",
            ".",
            "#",
            "[",
            "[a",
            "[a=]",
            "[a=b",
            "[a!=b]",
            ":",
            "p:nope",
            "p::nope",
            ":nth-child()",
            ":nth-child(x)",
            "p:not()",
            "ns|p",
            "1p",
            "p.1",
            "p:not(",
            "p)",
            "@media",
            "p{",
            "a:nth-child(2n+)",
            "a:nth-child(2n1)",
        ] {
            assert!(
                parse_selector_list(bad).is_none(),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn escapes_and_unicode_idents() {
        let h = Html::parse_document(
            r#"<html><body><p class="a:b é">x</p><p class="1x">y</p></body></html>"#,
        );
        assert_eq!(matched(r".a\:b", &h), vec!["p:x"]);
        assert_eq!(matched(".é", &h), vec!["p:x"]);
        assert_eq!(matched(r".\31 x", &h), vec!["p:y"]);
    }

    #[test]
    fn whitespace_variants() {
        let d = doc();
        assert_eq!(matched("  body   >   div  ", &d), vec!["div#main"]);
        assert_eq!(matched("body\n>\ndiv", &d), vec!["div#main"]);
        assert_eq!(matched("p\t.intro", &d), Vec::<String>::new());
    }
}
