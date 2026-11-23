//! Search and replace across the metadata of several books (roadmap L16): one field, a literal text
//! or a regular expression, a preview of every change before anything is written, and one undo
//! step for the whole replacement.
//!
//! Lists (authors, tags) are matched as the text the user sees (`A & B`, `x, y`) and split again
//! after the replacement. Regular expressions use the `regex` crate (no backtracking, so a hostile
//! pattern cannot hang the program) with a size limit; `$1` in the replacement is the first group.

use anyhow::{Context, Result, bail};
use regex::{Regex, RegexBuilder};

use super::library::{Book, Library};
use super::opf::Metadata;
use super::util::author_sort;

/// The longest pattern or replacement accepted.
pub const MAX_RULE_CHARS: usize = 1000;

/// The field a rule works on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Title,
    Authors,
    Series,
    Publisher,
    Language,
    Tags,
    Comments,
    /// A custom column by label.
    Custom(String),
}

impl Target {
    /// The built-in fields in the order a menu lists them.
    pub const BUILTIN: [Target; 7] = [
        Target::Title,
        Target::Authors,
        Target::Series,
        Target::Publisher,
        Target::Language,
        Target::Tags,
        Target::Comments,
    ];

    /// The name used on the command line and in files.
    pub fn key(&self) -> String {
        match self {
            Target::Title => "title".into(),
            Target::Authors => "authors".into(),
            Target::Series => "series".into(),
            Target::Publisher => "publisher".into(),
            Target::Language => "language".into(),
            Target::Tags => "tags".into(),
            Target::Comments => "comments".into(),
            Target::Custom(l) => format!("#{l}"),
        }
    }

    pub fn parse(s: &str) -> Option<Target> {
        let s = s.trim();
        if let Some(l) = s.strip_prefix('#') {
            return super::custom::is_label_shape(l).then(|| Target::Custom(l.to_lowercase()));
        }
        match s.to_ascii_lowercase().as_str() {
            "title" => Some(Target::Title),
            "authors" | "author" => Some(Target::Authors),
            "series" => Some(Target::Series),
            "publisher" => Some(Target::Publisher),
            "language" => Some(Target::Language),
            "tags" | "tag" => Some(Target::Tags),
            "comments" | "description" => Some(Target::Comments),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub target: Target,
    pub find: String,
    pub replace: String,
    pub regex: bool,
    pub case_sensitive: bool,
}

/// One book that the rule would change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub id: u64,
    pub title: String,
    pub before: String,
    pub after: String,
    /// Why the change cannot be applied (an empty title, a value the column refuses).
    pub problem: Option<String>,
}

/// What [`preview`] found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Preview {
    /// The first changes, at most the `limit` that was asked for.
    pub changes: Vec<Change>,
    /// How many books the rule changes in all.
    pub total: usize,
    /// How many of those cannot be applied.
    pub problems: usize,
}

fn matcher(rule: &Rule) -> Result<Regex> {
    if rule.find.is_empty() {
        bail!("write the text to search for");
    }
    if rule.find.chars().count() > MAX_RULE_CHARS || rule.replace.chars().count() > MAX_RULE_CHARS {
        bail!(
            "the text to search for and the replacement can have {MAX_RULE_CHARS} characters at most"
        );
    }
    let pattern = if rule.regex {
        rule.find.clone()
    } else {
        regex::escape(&rule.find)
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(!rule.case_sensitive)
        .size_limit(1 << 20)
        .build()
        .with_context(|| "the search pattern is not valid".to_string())
}

fn replacement(rule: &Rule) -> String {
    if rule.regex {
        rule.replace.clone()
    } else {
        // a literal replacement: `$` means itself
        rule.replace.replace('$', "$$")
    }
}

/// The text of a field as the user sees it.
pub fn field_text(b: &Book, target: &Target) -> String {
    let m = &b.meta;
    match target {
        Target::Title => m.title.clone(),
        Target::Authors => m.authors.join(" & "),
        Target::Series => m.series.clone(),
        Target::Publisher => m.publisher.clone(),
        Target::Language => m.language.clone(),
        Target::Tags => m.tags.join(", "),
        Target::Comments => m.description.clone(),
        Target::Custom(l) => b.custom_value(l).to_string(),
    }
}

fn split_list(s: &str, seps: &[char]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in s.split(|c| seps.contains(&c)) {
        let p = p.trim();
        if !p.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(p)) {
            out.push(p.to_string());
        }
    }
    out
}

/// `meta` with `text` stored in the field (lists are split again, the author sort follows).
pub fn with_field(meta: &Metadata, target: &Target, text: &str) -> Metadata {
    let mut m = meta.clone();
    match target {
        Target::Title => m.title = text.trim().to_string(),
        Target::Authors => {
            m.authors = split_list(text, &['&', ';']);
            m.author_sort = m
                .authors
                .iter()
                .map(|a| author_sort(a))
                .collect::<Vec<_>>()
                .join(" & ");
        }
        Target::Series => m.series = text.trim().to_string(),
        Target::Publisher => m.publisher = text.trim().to_string(),
        Target::Language => m.language = text.trim().to_string(),
        Target::Tags => m.tags = split_list(text, &[',']),
        Target::Comments => m.description = text.to_string(),
        Target::Custom(_) => {}
    }
    m
}

fn problem(lib: &Library, target: &Target, after: &str) -> Option<String> {
    match target {
        Target::Title if after.trim().is_empty() => Some("the title would be empty".into()),
        Target::Custom(l) => match lib.column(l) {
            None => Some(format!("there is no custom column `#{l}`")),
            Some(d) => d.normalize(after).err().map(|e| format!("{e:#}")),
        },
        _ => None,
    }
}

fn compute(lib: &Library, ids: &[u64], rule: &Rule) -> Result<Vec<Change>> {
    let re = matcher(rule)?;
    let rep = replacement(rule);
    let mut out = Vec::new();
    for b in lib.books_for(ids) {
        let before = field_text(b, &rule.target);
        let after = re.replace_all(&before, rep.as_str()).into_owned();
        if after == before {
            continue;
        }
        let problem = problem(lib, &rule.target, &after);
        out.push(Change {
            id: b.id,
            title: b.meta.title.clone(),
            before,
            after,
            problem,
        });
    }
    Ok(out)
}

/// What the rule would do to `ids`; the first `limit` changes are returned.
pub fn preview(lib: &Library, ids: &[u64], rule: &Rule, limit: usize) -> Result<Preview> {
    let all = compute(lib, ids, rule)?;
    Ok(Preview {
        total: all.len(),
        problems: all.iter().filter(|c| c.problem.is_some()).count(),
        changes: all.into_iter().take(limit).collect(),
    })
}

/// Applies the rule to `ids` as one undo step. Changes that cannot be applied are skipped (the
/// preview lists them). Returns `(changed, skipped)`.
pub fn apply(lib: &mut Library, ids: &[u64], rule: &Rule) -> Result<(usize, usize)> {
    let all = compute(lib, ids, rule)?;
    let skipped = all.iter().filter(|c| c.problem.is_some()).count();
    let good: Vec<&Change> = all.iter().filter(|c| c.problem.is_none()).collect();
    if good.is_empty() {
        return Ok((0, skipped));
    }
    let label = format!(
        "Replaced “{}” with “{}” in {}",
        rule.find,
        rule.replace,
        rule.target.key()
    );
    let n = match &rule.target {
        Target::Custom(l) => {
            let edits: Vec<(u64, Vec<(String, String)>)> = good
                .iter()
                .map(|c| (c.id, vec![(l.clone(), c.after.clone())]))
                .collect();
            lib.edit_custom(label, &edits)?
        }
        t => {
            let edits: Vec<(u64, Metadata)> = good
                .iter()
                .filter_map(|c| {
                    lib.get(c.id)
                        .map(|b| (c.id, with_field(&b.meta, t, &c.after)))
                })
                .collect();
            lib.update_metadata_many(label, edits)?
        }
    };
    Ok((n, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(target: Target, find: &str, replace: &str) -> Rule {
        Rule {
            target,
            find: find.into(),
            replace: replace.into(),
            regex: false,
            case_sensitive: false,
        }
    }

    #[test]
    fn targets_have_names() {
        for t in Target::BUILTIN {
            assert_eq!(Target::parse(&t.key()), Some(t));
        }
        assert_eq!(Target::parse("#Mood"), Some(Target::Custom("mood".into())));
        assert_eq!(Target::parse("#2bad"), None);
        assert_eq!(Target::parse("size"), None);
    }

    #[test]
    fn lists_are_split_again_and_the_author_sort_follows() {
        let mut m = Metadata {
            title: "T".into(),
            authors: vec!["A One".into()],
            tags: vec!["x".into()],
            ..Default::default()
        };
        m = with_field(&m, &Target::Authors, "Jane Doe & John Smith");
        assert_eq!(m.authors, vec!["Jane Doe", "John Smith"]);
        assert_eq!(m.author_sort, "Doe, Jane & Smith, John");
        m = with_field(&m, &Target::Tags, "b, a ,B,");
        assert_eq!(m.tags, vec!["b", "a"]);
    }

    #[test]
    fn the_pattern_is_checked() {
        assert!(matcher(&rule(Target::Title, "", "x")).is_err());
        let mut r = rule(Target::Title, "(", "x");
        r.regex = true;
        assert!(matcher(&r).is_err());
        r.find = "a".repeat(MAX_RULE_CHARS + 1);
        assert!(matcher(&r).is_err());
        // a literal search treats every character as itself
        let lit = rule(Target::Title, "(a+)", "x");
        assert!(matcher(&lit).unwrap().is_match("is (a+) here"));
        // and a literal replacement keeps `$`
        assert_eq!(replacement(&lit), "x");
        assert_eq!(replacement(&rule(Target::Title, "a", "$5")), "$$5");
    }
}
