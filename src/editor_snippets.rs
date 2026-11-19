//! Snippets of the book editor (Calibre's snippets, CE-343..): a trigger word that expands to text
//! with tab stops. The body uses GtkSourceView's snippet syntax: `$1`, `${2:default}`, `$0` for the
//! final cursor position, `\$` for a literal dollar. Built-ins ship with the app; the user's own
//! are stored as JSON in the config folder. No GTK in here.

use crate::i18n::tr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    pub name: String,
    pub trigger: String,
    pub body: String,
    /// Files it applies to: `markup` (XHTML, HTML, XML), `css` or `any`.
    #[serde(default = "any")]
    pub applies_to: String,
}

fn any() -> String {
    "any".into()
}

impl Snippet {
    pub fn new(name: &str, trigger: &str, body: &str, applies_to: &str) -> Snippet {
        Snippet {
            name: name.into(),
            trigger: trigger.into(),
            body: body.into(),
            applies_to: applies_to.into(),
        }
    }

    pub fn applies_to_file(&self, css: bool) -> bool {
        match self.applies_to.as_str() {
            "css" => css,
            "markup" => !css,
            _ => true,
        }
    }
}

/// Filler text for layout work (three paragraphs).
const LOREM: &str = "<p>Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat.</p>\n<p>Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.</p>\n<p>Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo.</p>$0";

/// Snippets that come with Bookx.
pub fn builtins() -> Vec<Snippet> {
    vec![
        Snippet::new("Paragraph", "p", "<p>${1*:text}</p>$0", "markup"),
        Snippet::new("Heading", "h", "<h${1:2}>${2*:Heading}</h$1>$0", "markup"),
        Snippet::new(
            "Image",
            "img",
            "<img src=\"${1:../Images/image.jpg}\" alt=\"${2:description}\"/>$0",
            "markup",
        ),
        Snippet::new(
            "Link",
            "link",
            "<a href=\"${1:#}\">${2*:text}</a>$0",
            "markup",
        ),
        Snippet::new(
            "Section (epub:type)",
            "sec",
            "<section epub:type=\"${1:chapter}\" id=\"${2:id}\">\n\t<h2>${3:Title}</h2>\n\t<p>${4:text}</p>\n</section>$0",
            "markup",
        ),
        Snippet::new(
            "Footnote reference",
            "fn",
            "<a epub:type=\"noteref\" href=\"#${1:note1}\">${2:1}</a>$0",
            "markup",
        ),
        Snippet::new(
            "Aside footnote",
            "aside",
            "<aside epub:type=\"footnote\" id=\"${1:note1}\">\n\t<p>${2:text}</p>\n</aside>$0",
            "markup",
        ),
        Snippet::new("Any tag (wraps the selection)", "<<", "<${1:p}>${2*}</$1>$0", "markup"),
        Snippet::new(
            "Tag with a class (wraps the selection)",
            "<c",
            "<${1:p} class=\"${2:class}\">${3*}</$1>$0",
            "markup",
        ),
        Snippet::new("Self-closing tag", "<>", "<${1:br}/>$0", "markup"),
        Snippet::new(
            "Link (wraps the selection)",
            "<a",
            "<a href=\"${1:#}\">${2*:text}</a>$0",
            "markup",
        ),
        Snippet::new(
            "Image",
            "<i",
            "<img src=\"${1:../Images/image.jpg}\" alt=\"${2:description}\"/>$0",
            "markup",
        ),
        Snippet::new("Filler paragraphs", "Lorem", LOREM, "markup"),
        Snippet::new(
            "CSS rule",
            "rule",
            "${1:selector} {\n\t${2:property}: ${3:value};\n}$0",
            "css",
        ),
    ]
}

/// Letters, digits and underscores, optionally after a `<` (like `<a`), or exactly `<>` / `<<`.
pub fn valid_trigger(t: &str) -> bool {
    let word = |w: &str| w.chars().all(|c| c.is_alphanumeric() || c == '_');
    t == "<>"
        || t == "<<"
        || word(t)
        || t.strip_prefix('<')
            .is_some_and(|r| !r.is_empty() && word(r))
}

/// Why a snippet cannot be saved, or `None` when it is fine.
pub fn problem(s: &Snippet) -> Option<String> {
    if s.trigger.trim().is_empty() {
        return Some(tr("The trigger word is empty"));
    }
    if !valid_trigger(&s.trigger) {
        return Some(tr(
            "The trigger may only contain letters, digits and underscores (a leading < is allowed, and <> or <<)",
        ));
    }
    if s.body.trim().is_empty() {
        return Some(tr("The snippet text is empty"));
    }
    if s.name.trim().is_empty() {
        return Some(tr("The snippet needs a name"));
    }
    // `${` must be closed
    let mut depth = 0i32;
    let mut chars = s.body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '$' if chars.peek() == Some(&'{') => {
                chars.next();
                depth += 1;
            }
            '}' if depth > 0 => depth -= 1,
            _ => {}
        }
    }
    if depth != 0 {
        return Some("A ${…} tab stop is not closed".into());
    }
    None
}

/// Built-ins first, then the user's; a user snippet replaces a built-in with the same trigger
/// (and applicability).
pub fn merged(user: &[Snippet]) -> Vec<Snippet> {
    let mut v: Vec<Snippet> = builtins()
        .into_iter()
        .filter(|b| {
            !user
                .iter()
                .any(|u| u.trigger == b.trigger && u.applies_to == b.applies_to)
        })
        .collect();
    v.extend(user.iter().cloned());
    v
}

/// The snippet whose trigger is `word`, for the file kind.
pub fn find<'a>(all: &'a [Snippet], word: &str, css: bool) -> Option<&'a Snippet> {
    all.iter()
        .rev()
        .find(|s| s.trigger == word && s.applies_to_file(css))
}

/// The trigger-like word at the end of `before`.
pub fn trailing_trigger(before: &str) -> &str {
    let start = before
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
        .last()
        .map(|(i, _)| i)
        .unwrap_or(before.len());
    &before[start..]
}

/// The tab stops of a body, in the order Tab visits them (`$0` last).
pub fn tab_stops(body: &str) -> Vec<u32> {
    let mut v = Vec::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '$' => {
                if chars.peek() == Some(&'{') {
                    chars.next();
                }
                let mut n = String::new();
                while let Some(d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                    n.push(*d);
                    chars.next();
                }
                if let Ok(n) = n.parse::<u32>() {
                    if !v.contains(&n) {
                        v.push(n);
                    }
                }
            }
            _ => {}
        }
    }
    v.sort_by_key(|n| if *n == 0 { u32::MAX } else { *n });
    v
}

// ---- selected text, plain expansion, import and export ---------------------------------------

fn escape_default(text: &str) -> String {
    let mut o = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '$' | '}') {
            o.push('\\');
        }
        o.push(c);
    }
    o
}

/// Reads `${N*}` / `${N*:default}` at `chars[i..]` (which starts with `${`): the number, the
/// default text and the index after the closing brace.
fn star_placeholder(chars: &[char], i: usize) -> Option<(u32, Option<String>, usize)> {
    let mut j = i + 2;
    let mut n = String::new();
    while chars.get(j).is_some_and(|c| c.is_ascii_digit()) {
        n.push(chars[j]);
        j += 1;
    }
    if n.is_empty() || chars.get(j) != Some(&'*') {
        return None;
    }
    j += 1;
    let mut def = None;
    if chars.get(j) == Some(&':') {
        j += 1;
        let mut d = String::new();
        let mut depth = 0;
        while let Some(&c) = chars.get(j) {
            match c {
                '\\' => {
                    d.push(c);
                    if let Some(&x) = chars.get(j + 1) {
                        d.push(x);
                    }
                    j += 2;
                    continue;
                }
                '{' => depth += 1,
                '}' if depth == 0 => break,
                '}' => depth -= 1,
                _ => {}
            }
            d.push(c);
            j += 1;
        }
        def = Some(d);
    }
    if chars.get(j) != Some(&'}') {
        return None;
    }
    Some((n.parse().ok()?, def, j + 1))
}

/// Whether the body has a `${N*}` placeholder that takes the selected text (CE-383).
pub fn has_selection_placeholder(body: &str) -> bool {
    let chars: Vec<char> = body.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '$' if chars.get(i + 1) == Some(&'{') && star_placeholder(&chars, i).is_some() => {
                return true;
            }
            _ => i += 1,
        }
    }
    false
}

/// The body with every `${N*}` / `${N*:default}` turned into a normal tab stop: the selected text
/// when there is one (it becomes the stop's text), else the default (CE-383).
pub fn with_selection(body: &str, selected: &str) -> String {
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' {
            out.push('\\');
            if let Some(&c) = chars.get(i + 1) {
                out.push(c);
            }
            i += 2;
        } else if chars[i] == '$' && chars.get(i + 1) == Some(&'{') {
            if let Some((n, def, next)) = star_placeholder(&chars, i) {
                if !selected.is_empty() {
                    out.push_str(&format!("${{{n}:{}}}", escape_default(selected)));
                } else if let Some(d) = def {
                    out.push_str(&format!("${{{n}:{d}}}"));
                } else {
                    out.push_str(&format!("${n}"));
                }
                i = next;
            } else {
                out.push('$');
                i += 1;
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// The text of a snippet with its tab stops resolved to their default text and no caret marker, for
/// the Search & replace fields where nothing may jump around (CE-387). Mirrors repeat the text of
/// the first stop with a default; `selected` fills `${N*}`.
pub fn flatten(body: &str, selected: &str) -> String {
    let body = with_selection(body, selected);
    let chars: Vec<char> = body.chars().collect();
    // defaults by number (first one wins)
    let mut defaults: std::collections::BTreeMap<u32, String> = Default::default();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '$' if chars.get(i + 1) == Some(&'{') => {
                let (mut j, mut n) = (i + 2, String::new());
                while chars.get(j).is_some_and(|c| c.is_ascii_digit()) {
                    n.push(chars[j]);
                    j += 1;
                }
                if let (Ok(n), Some(':')) = (n.parse::<u32>(), chars.get(j).copied()) {
                    let (text, _) = read_default(&chars, j + 1);
                    defaults.entry(n).or_insert(text);
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                if let Some(&c) = chars.get(i + 1) {
                    out.push(c);
                }
                i += 2;
            }
            '$' if chars.get(i + 1) == Some(&'{') => {
                let (mut j, mut n) = (i + 2, String::new());
                while chars.get(j).is_some_and(|c| c.is_ascii_digit()) {
                    n.push(chars[j]);
                    j += 1;
                }
                let num = n.parse::<u32>().ok();
                if chars.get(j) == Some(&':') {
                    let (_, end) = read_default(&chars, j + 1);
                    if let Some(d) = num.and_then(|n| defaults.get(&n)) {
                        out.push_str(&flatten_text(d));
                    }
                    i = end;
                } else if chars.get(j) == Some(&'}') {
                    if let Some(d) = num.and_then(|n| defaults.get(&n)) {
                        out.push_str(&flatten_text(d));
                    }
                    i = j + 1;
                } else {
                    out.push('$');
                    i += 1;
                }
            }
            '$' => {
                let (mut j, mut n) = (i + 1, String::new());
                while chars.get(j).is_some_and(|c| c.is_ascii_digit()) {
                    n.push(chars[j]);
                    j += 1;
                }
                if n.is_empty() {
                    out.push('$');
                    i += 1;
                } else {
                    if let Some(d) = n.parse::<u32>().ok().and_then(|n| defaults.get(&n)) {
                        out.push_str(&flatten_text(d));
                    }
                    i = j;
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// Text after `${N:` up to the closing brace (escapes kept) and the index after that brace.
fn read_default(chars: &[char], mut j: usize) -> (String, usize) {
    let mut d = String::new();
    let mut depth = 0;
    while let Some(&c) = chars.get(j) {
        match c {
            '\\' => {
                d.push(c);
                if let Some(&x) = chars.get(j + 1) {
                    d.push(x);
                }
                j += 2;
                continue;
            }
            '{' => depth += 1,
            '}' if depth == 0 => return (d, j + 1),
            '}' => depth -= 1,
            _ => {}
        }
        d.push(c);
        j += 1;
    }
    (d, j)
}

/// A default text may itself hold escapes or nested stops.
fn flatten_text(d: &str) -> String {
    if d.contains('$') || d.contains('\\') {
        flatten(d, "")
    } else {
        d.to_string()
    }
}

/// Candidate triggers at the end of `before`, best first: `<>` / `<<`, a word with its `<` (`<a`),
/// the bare word.
pub fn trigger_candidates(before: &str) -> Vec<&str> {
    let mut v = Vec::new();
    for sym in ["<>", "<<"] {
        if before.ends_with(sym) {
            v.push(&before[before.len() - 2..]);
        }
    }
    let w = trailing_trigger(before);
    if !w.is_empty() {
        let head = &before[..before.len() - w.len()];
        if head.ends_with('<') {
            v.push(&before[before.len() - w.len() - 1..]);
        }
        v.push(w);
    }
    v
}

/// The first snippet for a candidate of [`trigger_candidates`], with the candidate.
pub fn find_for<'a>(all: &'a [Snippet], before: &str, css: bool) -> Option<(&'a Snippet, String)> {
    trigger_candidates(before).into_iter().find_map(|c| {
        find(all, c, css).map(|s| {
            // `<p` + Tab on a snippet that starts with `<` takes the typed `<` along
            let typed_lt = !c.starts_with('<') && before.ends_with(&format!("<{c}"));
            let c = if typed_lt && s.body.starts_with('<') {
                format!("<{c}")
            } else {
                c.to_string()
            };
            (s, c)
        })
    })
}

/// The user's snippets as a JSON file for sharing.
pub fn export_json(user: &[Snippet]) -> String {
    serde_json::to_string_pretty(user).unwrap_or_else(|_| "[]".into())
}

/// Reads a shared snippet file: the valid snippets and a note for each one that was skipped.
pub fn import_json(text: &str) -> Result<(Vec<Snippet>, Vec<String>), String> {
    let list: Vec<Snippet> =
        serde_json::from_str(text).map_err(|_| tr("This file is not a list of snippets"))?;
    let (mut ok, mut skipped) = (Vec::new(), Vec::new());
    for s in list {
        match problem(&s) {
            None => ok.push(s),
            Some(p) => skipped.push(format!("{}: {p}", s.name)),
        }
    }
    Ok((ok, skipped))
}

/// `existing` with `new` added; a snippet with the same trigger and file kind is replaced.
pub fn merge_import(existing: &[Snippet], new: &[Snippet]) -> Vec<Snippet> {
    let mut out: Vec<Snippet> = existing
        .iter()
        .filter(|e| {
            !new.iter()
                .any(|n| n.trigger == e.trigger && n.applies_to == e.applies_to)
        })
        .cloned()
        .collect();
    out.extend(new.iter().cloned());
    out
}

/// Writes a user template for new files (`{title}` is replaced when it is used). The label becomes
/// the file name, so it may not hold a path.
pub fn save_template(dir: &Path, label: &str, extension: &str, body: &str) -> Result<(), String> {
    let label = label.trim();
    if label.is_empty() || label.contains(['/', '\\', '.']) {
        return Err(tr(
            "The template name may not be empty or contain / \\ or a dot",
        ));
    }
    if !matches!(extension, "xhtml" | "html" | "css") {
        return Err(tr("Templates are XHTML, HTML or CSS files"));
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    // one template per label: drop the same label with another extension
    for ext in ["xhtml", "html", "css"] {
        if ext != extension {
            let _ = std::fs::remove_file(dir.join(format!("{label}.{ext}")));
        }
    }
    std::fs::write(dir.join(format!("{label}.{extension}")), body).map_err(|e| e.to_string())
}

pub fn delete_template(dir: &Path, label: &str, extension: &str) -> bool {
    if label.contains(['/', '\\']) {
        return false;
    }
    let ext = if extension == "xhtml" {
        "xhtml"
    } else {
        extension
    };
    let a = std::fs::remove_file(dir.join(format!("{label}.{ext}"))).is_ok();
    let b = ext == "xhtml" && std::fs::remove_file(dir.join(format!("{label}.html"))).is_ok();
    a || b
}

/// `<config>/Bookx/snippets.json`.
pub fn path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.config_dir().join("snippets.json"))
}

pub fn load_from(path: &Path) -> Vec<Snippet> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_to(path: &Path, snippets: &[Snippet]) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(snippets).unwrap_or_else(|_| "[]".into()),
    )
}

/// The user's snippets.
pub fn load() -> Vec<Snippet> {
    path().map(|p| load_from(&p)).unwrap_or_default()
}

pub fn save(snippets: &[Snippet]) -> std::io::Result<()> {
    match path() {
        Some(p) => save_to(&p, snippets),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_ins_are_valid_and_cover_the_basics() {
        let b = builtins();
        for s in &b {
            assert_eq!(problem(s), None, "{}", s.name);
        }
        for t in ["p", "h", "img", "link", "sec"] {
            assert!(b.iter().any(|s| s.trigger == t), "{t}");
        }
        assert!(find(&b, "sec", false)
            .unwrap()
            .body
            .contains("epub:type=\"${1:chapter}\""));
        assert!(
            find(&b, "rule", false).is_none(),
            "CSS snippets are not offered in markup"
        );
        assert!(find(&b, "rule", true).is_some());
    }

    #[test]
    fn problems_are_explained() {
        let ok = Snippet::new("x", "ab", "<b>$0</b>", "any");
        assert_eq!(problem(&ok), None);
        let mut s = ok.clone();
        s.trigger = "a b".into();
        assert!(problem(&s).unwrap().contains("trigger"));
        s.trigger = String::new();
        assert!(problem(&s).unwrap().contains("empty"));
        let mut s = ok.clone();
        s.body = "<b>${1:open</b>".into();
        assert!(problem(&s).unwrap().contains("not closed"));
        let mut s = ok.clone();
        s.body = "cost \\${5}".into();
        assert_eq!(problem(&s), None, "an escaped dollar is not a tab stop");
        let mut s = ok;
        s.name = " ".into();
        assert!(problem(&s).unwrap().contains("name"));
    }

    #[test]
    fn user_snippets_override_built_ins_and_round_trip() {
        let user = vec![
            Snippet::new("My paragraph", "p", "<p class=\"x\">$0</p>", "markup"),
            Snippet::new("Mine", "zz", "zzz", "any"),
        ];
        let all = merged(&user);
        assert_eq!(find(&all, "p", false).unwrap().name, "My paragraph");
        assert_eq!(all.iter().filter(|s| s.trigger == "p").count(), 1);
        assert!(find(&all, "zz", true).is_some(), "'any' applies to CSS too");
        let dir = std::env::temp_dir().join(format!("bookx-snip-{}", uuid::Uuid::new_v4()));
        let f = dir.join("snippets.json");
        assert!(load_from(&f).is_empty(), "no file means no snippets");
        save_to(&f, &user).unwrap();
        assert_eq!(load_from(&f), user);
        std::fs::write(&f, "not json").unwrap();
        assert!(load_from(&f).is_empty(), "a broken file is ignored");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn trigger_before_the_cursor_and_tab_stops() {
        assert_eq!(trailing_trigger("<p>x</p>\n\timg"), "img");
        assert_eq!(trailing_trigger("<p>"), "");
        assert_eq!(trailing_trigger("ab cd"), "cd");
        assert_eq!(tab_stops("<h${1:2}>${2:T}</h$1>$0 \\$9"), vec![1, 2, 0]);
        assert_eq!(tab_stops("none"), Vec::<u32>::new());
    }

    #[test]
    fn selected_text_goes_into_the_starred_stop() {
        assert!(has_selection_placeholder("<a>${2*}</a>"));
        assert!(has_selection_placeholder("<a>${2*:text}</a>"));
        assert!(!has_selection_placeholder("<a>${2:text}</a> \\${1*}"));
        assert_eq!(
            with_selection("<b>${1*:word}</b>$0", "Hello"),
            "<b>${1:Hello}</b>$0"
        );
        assert_eq!(with_selection("<b>${1*:word}</b>", ""), "<b>${1:word}</b>");
        assert_eq!(with_selection("<b>${1*}</b>", ""), "<b>$1</b>");
        assert_eq!(
            with_selection("[${1*}]", "a$b}c\\d"),
            "[${1:a\\$b\\}c\\\\d}]",
            "special characters of the selection are escaped"
        );
        assert_eq!(with_selection("${2:plain}", "x"), "${2:plain}");
        assert_eq!(
            with_selection("<${1:p}>${2*}</$1>", "text"),
            "<${1:p}>${2:text}</$1>"
        );
    }

    #[test]
    fn plain_expansion_for_search_fields() {
        assert_eq!(flatten("<p>${1:text}</p>$0", ""), "<p>text</p>");
        assert_eq!(flatten("<h${1:2}>${2:Title}</h$1>$0", ""), "<h2>Title</h2>");
        assert_eq!(flatten("<${1:p}>${2*}</$1>", "x"), "<p>x</p>");
        assert_eq!(
            flatten("<${1*:p}>${2*}</$1>", "x"),
            "<x>x</x>",
            "every starred stop takes the selection"
        );
        assert_eq!(flatten("cost \\$5 \\\\", ""), "cost $5 \\");
        assert_eq!(flatten("a$0b", ""), "ab");
        assert_eq!(flatten("${1}${2:x}$2", ""), "xx");
    }

    #[test]
    fn built_in_symbol_snippets_and_candidates() {
        let b = builtins();
        for t in ["<>", "<a", "<i", "<<", "<c", "Lorem"] {
            assert!(b.iter().any(|s| s.trigger == t), "{t}");
            assert!(valid_trigger(t));
        }
        assert!(
            find(&b, "Lorem", false)
                .unwrap()
                .body
                .matches("<p>")
                .count()
                >= 3
        );
        assert_eq!(trigger_candidates("text <a"), vec!["<a", "a"]);
        assert_eq!(trigger_candidates("x<>"), vec!["<>"]);
        assert_eq!(trigger_candidates("x<<"), vec!["<<"]);
        assert_eq!(trigger_candidates("Lorem"), vec!["Lorem"]);
        assert!(trigger_candidates("a ").is_empty());
        let (s, c) = find_for(&b, "<p>x</p>\n<i", false).unwrap();
        assert_eq!((s.trigger.as_str(), c.as_str()), ("<i", "<i"));
        let (s, c) = find_for(&b, "<p", false).unwrap();
        assert_eq!(s.trigger, "p", "no `<p` snippet: the bare word is used");
        assert_eq!(c, "<p", "and the typed < is replaced too");
        for s in &b {
            assert_eq!(problem(s), None, "{}", s.name);
        }
        assert!(!valid_trigger("a b") && !valid_trigger("<") && !valid_trigger("a<"));
    }

    #[test]
    fn snippets_travel_as_json_and_bad_ones_are_reported() {
        let user = vec![
            Snippet::new("Mine", "zz", "zzz", "any"),
            Snippet::new("Mine 2", "<q", "<q>$0</q>", "markup"),
        ];
        let (back, skipped) = import_json(&export_json(&user)).unwrap();
        assert_eq!(back, user);
        assert!(skipped.is_empty());
        let (ok, skipped) = import_json(
            r#"[{"name":"A","trigger":"ok","body":"x"},{"name":"B","trigger":"no way","body":"y"}]"#,
        )
        .unwrap();
        assert_eq!(ok.len(), 1);
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].starts_with("B:"));
        assert_eq!(ok[0].applies_to, "any", "file kind defaults to all files");
        assert!(import_json("nonsense").is_err());
        let merged = merge_import(&user, &[Snippet::new("New zz", "zz", "new", "any")]);
        assert_eq!(merged.len(), 2);
        assert_eq!(
            merged.iter().find(|s| s.trigger == "zz").unwrap().body,
            "new"
        );
    }

    #[test]
    fn templates_are_saved_and_deleted_as_files() {
        let dir = std::env::temp_dir().join(format!("bookx-tpl-{}", uuid::Uuid::new_v4()));
        save_template(&dir, "Chapter", "xhtml", "<h1>{title}</h1>").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("Chapter.xhtml")).unwrap(),
            "<h1>{title}</h1>"
        );
        save_template(&dir, "Chapter", "css", "p {}").unwrap();
        assert!(!dir.join("Chapter.xhtml").exists(), "one template per name");
        assert!(save_template(&dir, "../x", "css", "").is_err());
        assert!(save_template(&dir, "", "css", "").is_err());
        assert!(save_template(&dir, "a.b", "css", "").is_err());
        assert!(save_template(&dir, "x", "exe", "").is_err());
        assert!(delete_template(&dir, "Chapter", "css"));
        assert!(!delete_template(&dir, "Chapter", "css"));
        assert!(!delete_template(&dir, "../etc", "css"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
