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

/// Snippets that come with Bookx.
pub fn builtins() -> Vec<Snippet> {
    vec![
        Snippet::new("Paragraph", "p", "<p>${1:text}</p>$0", "markup"),
        Snippet::new("Heading", "h", "<h${1:2}>${2:Heading}</h$1>$0", "markup"),
        Snippet::new(
            "Image",
            "img",
            "<img src=\"${1:../Images/image.jpg}\" alt=\"${2:description}\"/>$0",
            "markup",
        ),
        Snippet::new(
            "Link",
            "link",
            "<a href=\"${1:#}\">${2:text}</a>$0",
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
        Snippet::new(
            "CSS rule",
            "rule",
            "${1:selector} {\n\t${2:property}: ${3:value};\n}$0",
            "css",
        ),
    ]
}

/// Why a snippet cannot be saved, or `None` when it is fine.
pub fn problem(s: &Snippet) -> Option<String> {
    if s.trigger.trim().is_empty() {
        return Some(tr("The trigger word is empty"));
    }
    if s.trigger
        .chars()
        .any(|c| !(c.is_alphanumeric() || c == '_'))
    {
        return Some(tr(
            "The trigger may only contain letters, digits and underscores",
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
}
