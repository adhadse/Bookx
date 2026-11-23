//! Find & replace for the book editor, without GTK: the pattern engine (plain text, or regular
//! expressions with look-around and back-references through `fancy-regex`), the replacement
//! dialect (`\1` and `$1` both work), search scopes and "go to line".

use fancy_regex::{Captures, Regex};
use serde::{Deserialize, Serialize};

use crate::core::util::extension;

/// How a search is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Opts {
    /// The query is a regular expression (otherwise it is literal text).
    pub regex: bool,
    pub case_sensitive: bool,
    /// `.` also matches a line break.
    pub dot_all: bool,
    /// Plain text and regular expressions only match whole words (`\b…\b`).
    pub whole_word: bool,
}

/// What a replacement function (function mode) is told about one match. Offsets are bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchInfo {
    pub start: usize,
    pub end: usize,
    /// Group 0 is the whole match; a group that did not take part is `None`.
    pub groups: Vec<Option<String>>,
    /// `(name, group index)` of the named groups.
    pub names: Vec<(String, usize)>,
}

/// A compiled search.
pub struct Pattern {
    re: Regex,
    regex_mode: bool,
}

/// Text files the editor can open.
pub fn is_text_path(path: &str) -> bool {
    matches!(
        extension(path).as_str(),
        "xhtml"
            | "html"
            | "htm"
            | "xht"
            | "css"
            | "xml"
            | "opf"
            | "ncx"
            | "txt"
            | "svg"
            | "js"
            | "smil"
            | "md"
    ) || path == "mimetype"
}

fn is_markup(path: &str) -> bool {
    matches!(extension(path).as_str(), "xhtml" | "html" | "htm" | "xht")
}

/// Turns the replacement the user typed into the engine's template syntax (`${1}`, `${name}`, `$$`).
///
/// * plain-text mode: the text is literal, so every `$` is escaped;
/// * regex mode: `\1`..`\99` and `\g<name>` are groups, `$1`, `${1}`, `$name` and `${name}` too, `\n`, `\t`,
///   `\r` are control characters, `\\` is a backslash, `\$` a dollar sign, and a lone `$` stays a dollar sign.
pub fn translate_replacement(rep: &str, regex_mode: bool) -> String {
    if !regex_mode {
        return rep.replace('$', "$$");
    }
    let c: Vec<char> = rep.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '\\' if i + 1 < c.len() => {
                let n = c[i + 1];
                match n {
                    '0'..='9' => {
                        let mut j = i + 1;
                        let mut num = String::new();
                        while j < c.len() && c[j].is_ascii_digit() && num.len() < 2 {
                            num.push(c[j]);
                            j += 1;
                        }
                        out.push_str(&format!("${{{num}}}"));
                        i = j;
                        continue;
                    }
                    'g' if c.get(i + 2) == Some(&'<') => {
                        if let Some(close) = c[i + 3..].iter().position(|x| *x == '>') {
                            let name: String = c[i + 3..i + 3 + close].iter().collect();
                            out.push_str(&format!("${{{name}}}"));
                            i += 4 + close;
                            continue;
                        }
                        out.push_str("\\g");
                    }
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    '\\' => out.push('\\'),
                    '$' => out.push_str("$$"),
                    other => {
                        out.push('\\');
                        out.push(other);
                    }
                }
                i += 2;
            }
            '$' => {
                let rest = &c[i + 1..];
                if rest.first() == Some(&'{') {
                    if let Some(close) = rest.iter().position(|x| *x == '}') {
                        let name: String = rest[1..close].iter().collect();
                        out.push_str(&format!("${{{name}}}"));
                        i += close + 2;
                        continue;
                    }
                    out.push_str("$$");
                } else if rest.first() == Some(&'$') {
                    out.push_str("$$");
                    i += 2;
                    continue;
                } else if rest.first().is_some_and(|x| x.is_ascii_digit()) {
                    let n: String = rest.iter().take_while(|x| x.is_ascii_digit()).collect();
                    out.push_str(&format!("${{{n}}}"));
                    i += 1 + n.len();
                    continue;
                } else if rest.first().is_some_and(|x| x.is_alphabetic() || *x == '_') {
                    let n: String = rest
                        .iter()
                        .take_while(|x| x.is_alphanumeric() || **x == '_')
                        .collect();
                    out.push_str(&format!("${{{n}}}"));
                    i += 1 + n.chars().count();
                    continue;
                } else {
                    out.push_str("$$");
                }
                i += 1;
            }
            ch => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

fn expand(caps: &Captures, template: &str) -> String {
    let mut s = String::new();
    caps.expand(template, &mut s);
    s
}

impl Pattern {
    pub fn new(query: &str, opts: Opts) -> Result<Pattern, String> {
        if query.is_empty() {
            return Err("empty".into());
        }
        let body = if opts.regex {
            query.to_string()
        } else {
            fancy_regex::escape(query).into_owned()
        };
        let body = if opts.whole_word {
            format!(r"\b(?:{body})\b")
        } else {
            body
        };
        let mut flags = String::from("(?m");
        if !opts.case_sensitive {
            flags.push('i');
        }
        if opts.dot_all {
            flags.push('s');
        }
        flags.push(')');
        Regex::new(&format!("{flags}{body}"))
            .map(|re| Pattern {
                re,
                regex_mode: opts.regex,
            })
            .map_err(|e| e.to_string())
    }

    /// The first match starting at or after byte `from`.
    pub fn find_at(&self, text: &str, from: usize) -> Option<(usize, usize)> {
        if from > text.len() {
            return None;
        }
        self.re
            .find_from_pos(text, from)
            .ok()
            .flatten()
            .map(|m| (m.start(), m.end()))
    }

    /// Every match, optionally only those inside one of the byte `ranges`.
    pub fn find_all(&self, text: &str, ranges: Option<&[(usize, usize)]>) -> Vec<(usize, usize)> {
        self.re
            .find_iter(text)
            .filter_map(|m| m.ok())
            .map(|m| (m.start(), m.end()))
            .filter(|m| inside(*m, ranges))
            .collect()
    }

    /// The last match that ends at or before byte `before`.
    #[cfg(test)]
    pub fn find_before(
        &self,
        text: &str,
        before: usize,
        ranges: Option<&[(usize, usize)]>,
    ) -> Option<(usize, usize)> {
        self.find_all(text, ranges)
            .into_iter()
            .rev()
            .find(|m| m.1 <= before)
    }

    /// Replaces every match (inside `ranges`, when given) and returns the new text and the count.
    pub fn replace_all(
        &self,
        text: &str,
        replacement: &str,
        ranges: Option<&[(usize, usize)]>,
    ) -> (String, usize) {
        let template = translate_replacement(replacement, self.regex_mode);
        let mut out = String::with_capacity(text.len());
        let (mut last, mut n) = (0, 0);
        for caps in self.re.captures_iter(text).filter_map(|c| c.ok()) {
            let Some(m) = caps.get(0) else { continue };
            if !inside((m.start(), m.end()), ranges) {
                continue;
            }
            out.push_str(&text[last..m.start()]);
            out.push_str(&expand(&caps, &template));
            last = m.end();
            n += 1;
        }
        out.push_str(&text[last..]);
        (out, n)
    }

    /// Every match with the replacement the user typed expanded for it, as `(start, end, replacement)`.
    pub fn replacements(
        &self,
        text: &str,
        replacement: &str,
        ranges: Option<&[(usize, usize)]>,
    ) -> Vec<(usize, usize, String)> {
        let template = translate_replacement(replacement, self.regex_mode);
        self.re
            .captures_iter(text)
            .filter_map(|c| c.ok())
            .filter_map(|caps| {
                let m = caps.get(0)?;
                inside((m.start(), m.end()), ranges)
                    .then(|| (m.start(), m.end(), expand(&caps, &template)))
            })
            .collect()
    }

    /// Replaces every match (inside `ranges`) with what `f` returns; `None` leaves the match as it is. Stops at
    /// the first error. Returns the new text and the number of matches replaced.
    pub fn replace_with<E>(
        &self,
        text: &str,
        ranges: Option<&[(usize, usize)]>,
        f: &mut dyn FnMut(&MatchInfo) -> Result<Option<String>, E>,
    ) -> Result<(String, usize), E> {
        let names: Vec<(String, usize)> = self
            .re
            .capture_names()
            .enumerate()
            .filter_map(|(i, n)| n.map(|n| (n.to_string(), i)))
            .collect();
        let mut out = String::with_capacity(text.len());
        let (mut last, mut n) = (0, 0);
        for caps in self.re.captures_iter(text).filter_map(|c| c.ok()) {
            let Some(m) = caps.get(0) else { continue };
            if !inside((m.start(), m.end()), ranges) {
                continue;
            }
            let info = MatchInfo {
                start: m.start(),
                end: m.end(),
                groups: (0..caps.len())
                    .map(|i| caps.get(i).map(|g| g.as_str().to_string()))
                    .collect(),
                names: names.clone(),
            };
            if let Some(rep) = f(&info)? {
                out.push_str(&text[last..m.start()]);
                out.push_str(&rep);
                last = m.end();
                n += 1;
            }
        }
        out.push_str(&text[last..]);
        Ok((out, n))
    }

    /// The replacement for the match that is exactly `start..end`, or `None` when that range is not a match.
    pub fn replacement_for(
        &self,
        text: &str,
        start: usize,
        end: usize,
        replacement: &str,
    ) -> Option<String> {
        let caps = self.re.captures_from_pos(text, start).ok().flatten()?;
        let m = caps.get(0)?;
        if m.start() != start || m.end() != end {
            return None;
        }
        Some(expand(
            &caps,
            &translate_replacement(replacement, self.regex_mode),
        ))
    }
}

fn inside(m: (usize, usize), ranges: Option<&[(usize, usize)]>) -> bool {
    ranges.is_none_or(|rs| rs.iter().any(|r| m.0 >= r.0 && m.1 <= r.1))
}

/// Where a search looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    ThisFile,
    TextFiles,
    StyleFiles,
    AllFiles,
    SelectedFiles,
    /// The files that have a tab.
    OpenFiles,
    MarkedText,
}

impl Scope {
    pub const ALL: [Scope; 7] = [
        Scope::ThisFile,
        Scope::TextFiles,
        Scope::StyleFiles,
        Scope::AllFiles,
        Scope::SelectedFiles,
        Scope::OpenFiles,
        Scope::MarkedText,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Scope::ThisFile => "This file",
            Scope::TextFiles => "Text files",
            Scope::StyleFiles => "Style files",
            Scope::AllFiles => "All files",
            Scope::SelectedFiles => "Selected files",
            Scope::OpenFiles => "Open files",
            Scope::MarkedText => "Marked text",
        }
    }

    pub fn from_index(i: u32) -> Scope {
        Scope::ALL
            .get(i as usize)
            .copied()
            .unwrap_or(Scope::ThisFile)
    }
}

/// The files a scope covers, in the order of `all` (callers rotate the current file to the front).
pub fn scope_files(
    scope: Scope,
    current: Option<&str>,
    all: &[String],
    selected: &[String],
    open: &[String],
) -> Vec<String> {
    let keep = |f: &&String| f.as_str() != "mimetype";
    match scope {
        Scope::ThisFile | Scope::MarkedText => current.map(String::from).into_iter().collect(),
        Scope::TextFiles => all
            .iter()
            .filter(keep)
            .filter(|f| is_markup(f))
            .cloned()
            .collect(),
        Scope::StyleFiles => all
            .iter()
            .filter(keep)
            .filter(|f| extension(f) == "css")
            .cloned()
            .collect(),
        Scope::AllFiles => all
            .iter()
            .filter(keep)
            .filter(|f| is_text_path(f))
            .cloned()
            .collect(),
        Scope::SelectedFiles => all
            .iter()
            .filter(keep)
            .filter(|f| selected.contains(f) && is_text_path(f))
            .cloned()
            .collect(),
        Scope::OpenFiles => all
            .iter()
            .filter(keep)
            .filter(|f| open.contains(f) && is_text_path(f))
            .cloned()
            .collect(),
    }
}

/// "Go to line": parses what the user typed (1-based) and returns the 0-based line, clamped to the file.
pub fn parse_line(input: &str, total_lines: usize) -> Option<usize> {
    let n: usize = input.trim().parse().ok()?;
    if n == 0 || total_lines == 0 {
        return None;
    }
    Some(n.min(total_lines) - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pat(q: &str, regex: bool, case: bool) -> Pattern {
        Pattern::new(
            q,
            Opts {
                regex,
                case_sensitive: case,
                dot_all: false,
                whole_word: false,
            },
        )
        .unwrap()
    }

    #[test]
    fn plain_text_is_literal_and_case_insensitive_by_default() {
        let p = pat("a.b", false, false);
        assert_eq!(p.find_all("A.B axb a.b", None), vec![(0, 3), (8, 11)]);
        let p = pat("a.b", false, true);
        assert_eq!(p.find_all("A.B a.b", None), vec![(4, 7)]);
    }

    #[test]
    fn regex_supports_lookaround_and_backreferences() {
        let p = pat(r"(?<=<p>)\w+(?=</p>)", true, true);
        assert_eq!(p.find_all("<p>one</p><b>two</b>", None), vec![(3, 6)]);
        let p = pat(r"(\w)\1", true, true);
        assert_eq!(p.find_all("book cat", None), vec![(1, 3)]);
        let p = pat(r"foo(?!bar)", true, true);
        assert_eq!(p.find_all("foobar foobaz", None), vec![(7, 10)]);
    }

    #[test]
    fn invalid_patterns_and_empty_queries_are_errors() {
        assert!(
            Pattern::new(
                "(",
                Opts {
                    regex: true,
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert_eq!(Pattern::new("", Opts::default()).err().unwrap(), "empty");
        assert!(Pattern::new("(", Opts::default()).is_ok());
    }

    #[test]
    fn dot_all_lets_dot_cross_lines() {
        let o = Opts {
            regex: true,
            case_sensitive: true,
            dot_all: true,
            whole_word: false,
        };
        assert_eq!(
            Pattern::new("a.b", o).unwrap().find_all("a\nb", None).len(),
            1
        );
        let o = Opts {
            dot_all: false,
            ..o
        };
        assert!(
            Pattern::new("a.b", o)
                .unwrap()
                .find_all("a\nb", None)
                .is_empty()
        );
    }

    #[test]
    fn whole_word_wraps_plain_text_and_regex() {
        let o = Opts {
            whole_word: true,
            case_sensitive: true,
            ..Default::default()
        };
        let p = Pattern::new("cat", o).unwrap();
        assert_eq!(p.find_all("cat concat cat.", None), vec![(0, 3), (11, 14)]);
        let r = Pattern::new("ca|do", Opts { regex: true, ..o }).unwrap();
        assert_eq!(r.find_all("ca cat do", None), vec![(0, 2), (7, 9)]);
    }

    #[test]
    fn replace_with_hands_groups_and_names_to_the_callback() {
        let p = pat(r"(?P<k>\w)(\d)?", true, true);
        let mut seen = Vec::new();
        let (out, n) = p
            .replace_with::<()>("a1 b", None, &mut |m| {
                seen.push(m.clone());
                Ok(Some(format!("<{}>", m.groups[1].clone().unwrap())))
            })
            .unwrap();
        assert_eq!((out.as_str(), n), ("<a> <b>", 2));
        assert_eq!(
            seen[0].groups,
            vec![Some("a1".into()), Some("a".into()), Some("1".into())]
        );
        assert_eq!(seen[1].groups[2], None);
        assert_eq!(seen[0].names, vec![("k".to_string(), 1)]);
        // None keeps the match, an error stops the run
        let (out, n) = p
            .replace_with::<()>("a b", None, &mut |_| Ok(None))
            .unwrap();
        assert_eq!((out.as_str(), n), ("a b", 0));
        assert!(p.replace_with("a b", None, &mut |_| Err("boom")).is_err());
        // ranges restrict the calls
        let (out, _) = p
            .replace_with::<()>("a|b", Some(&[(2, 3)]), &mut |_| Ok(Some("X".into())))
            .unwrap();
        assert_eq!(out, "a|X");
    }

    #[test]
    fn caret_and_dollar_match_at_line_boundaries() {
        let p = pat("^x$", true, true);
        assert_eq!(p.find_all("x\ny\nx", None), vec![(0, 1), (4, 5)]);
    }

    #[test]
    fn replacement_accepts_backslash_and_dollar_groups() {
        let p = pat(r"(\w+)-(\w+)", true, true);
        assert_eq!(p.replace_all("ab-cd", r"\2-\1", None), ("cd-ab".into(), 1));
        assert_eq!(p.replace_all("ab-cd", "$2-$1", None), ("cd-ab".into(), 1));
        assert_eq!(
            p.replace_all("ab-cd", "${2}x${1}", None),
            ("cdxab".into(), 1)
        );
        let n = pat(r"(?P<a>\w)(?P<b>\w)", true, true);
        assert_eq!(n.replace_all("xy", r"\g<b>\g<a>", None).0, "yx");
        assert_eq!(n.replace_all("xy", "$b$a", None).0, "yx");
    }

    #[test]
    fn translate_replacement_cases() {
        assert_eq!(translate_replacement(r"\1", true), "${1}");
        assert_eq!(translate_replacement(r"\12", true), "${12}");
        assert_eq!(translate_replacement("$1a", true), "${1}a");
        assert_eq!(translate_replacement("$name!", true), "${name}!");
        assert_eq!(translate_replacement("${n}", true), "${n}");
        assert_eq!(translate_replacement(r"\g<n>", true), "${n}");
        assert_eq!(translate_replacement(r"a\nb\t", true), "a\nb\t");
        assert_eq!(translate_replacement(r"\\1", true), "\\1");
        assert_eq!(translate_replacement(r"\$1", true), "$$1");
        assert_eq!(translate_replacement("cost $ 5", true), "cost $$ 5");
        assert_eq!(translate_replacement("$$", true), "$$");
        assert_eq!(translate_replacement("end$", true), "end$$");
        assert_eq!(translate_replacement(r"\q", true), r"\q");
        assert_eq!(translate_replacement("a\\", true), "a\\");
        assert_eq!(translate_replacement("$1 and \\1", false), "$$1 and \\1");
    }

    #[test]
    fn plain_text_replacement_is_literal() {
        let p = pat("price", false, false);
        assert_eq!(p.replace_all("the price", r"$5 \1", None).0, r"the $5 \1");
    }

    #[test]
    fn replace_all_in_ranges_only_touches_marked_text() {
        let p = pat("a", false, true);
        let (t, n) = p.replace_all("aa|aa|aa", "b", Some(&[(3, 5)]));
        assert_eq!((t.as_str(), n), ("aa|bb|aa", 2));
        assert_eq!(p.find_all("aa|aa", Some(&[(0, 1)])), vec![(0, 1)]);
        // a match must lie completely inside the range
        let w = pat("aa", false, true);
        assert!(w.find_all("aa", Some(&[(0, 1)])).is_empty());
    }

    #[test]
    fn find_at_and_find_before_walk_both_directions() {
        let p = pat("o", false, true);
        let t = "foo boo";
        assert_eq!(p.find_at(t, 0), Some((1, 2)));
        assert_eq!(p.find_at(t, 3), Some((5, 6)));
        assert_eq!(p.find_at(t, 7), None);
        assert_eq!(p.find_at(t, 99), None);
        assert_eq!(p.find_before(t, 7, None), Some((6, 7)));
        assert_eq!(p.find_before(t, 2, None), Some((1, 2)));
        assert_eq!(p.find_before(t, 1, None), None);
    }

    #[test]
    fn replacement_for_requires_an_exact_match_and_keeps_context() {
        let p = pat(r"(?<=<p>)(\w+)", true, true);
        let t = "<p>word</p>";
        assert_eq!(p.replacement_for(t, 3, 7, "[$1]"), Some("[word]".into()));
        assert_eq!(p.replacement_for(t, 3, 6, "x"), None);
        assert_eq!(p.replacement_for(t, 4, 7, "x"), None);
    }

    #[test]
    fn unicode_offsets_are_bytes() {
        let p = pat("é", false, true);
        assert_eq!(p.find_all("aéé", None), vec![(1, 3), (3, 5)]);
        assert_eq!(p.replace_all("aéb", "e", None).0, "aeb");
    }

    #[test]
    fn scopes_pick_the_right_files() {
        let all: Vec<String> = [
            "OEBPS/c1.xhtml",
            "OEBPS/c2.html",
            "OEBPS/s.css",
            "OEBPS/content.opf",
            "OEBPS/pic.png",
            "mimetype",
        ]
        .map(String::from)
        .to_vec();
        let sel = vec!["OEBPS/s.css".to_string(), "OEBPS/pic.png".to_string()];
        let open = vec!["OEBPS/c2.html".to_string(), "OEBPS/pic.png".to_string()];
        let f = |s| scope_files(s, Some("OEBPS/c2.html"), &all, &sel, &open);
        assert_eq!(f(Scope::ThisFile), vec!["OEBPS/c2.html"]);
        assert_eq!(f(Scope::MarkedText), vec!["OEBPS/c2.html"]);
        assert_eq!(f(Scope::TextFiles), vec!["OEBPS/c1.xhtml", "OEBPS/c2.html"]);
        assert_eq!(f(Scope::StyleFiles), vec!["OEBPS/s.css"]);
        assert_eq!(
            f(Scope::AllFiles),
            vec![
                "OEBPS/c1.xhtml",
                "OEBPS/c2.html",
                "OEBPS/s.css",
                "OEBPS/content.opf"
            ]
        );
        assert_eq!(f(Scope::SelectedFiles), vec!["OEBPS/s.css"]);
        assert_eq!(f(Scope::OpenFiles), vec!["OEBPS/c2.html"]);
        assert!(scope_files(Scope::ThisFile, None, &all, &sel, &open).is_empty());
    }

    #[test]
    fn scope_labels_and_indexes_round_trip() {
        for (i, s) in Scope::ALL.iter().enumerate() {
            assert_eq!(Scope::from_index(i as u32), *s);
            assert!(!s.label().is_empty());
        }
        assert_eq!(Scope::from_index(99), Scope::ThisFile);
    }

    #[test]
    fn go_to_line_parses_and_clamps() {
        assert_eq!(parse_line("1", 10), Some(0));
        assert_eq!(parse_line(" 7 ", 10), Some(6));
        assert_eq!(parse_line("99", 10), Some(9));
        assert_eq!(parse_line("0", 10), None);
        assert_eq!(parse_line("x", 10), None);
        assert_eq!(parse_line("", 10), None);
        assert_eq!(parse_line("3", 0), None);
    }

    #[test]
    fn text_paths() {
        assert!(is_text_path("a/b.XHTML"));
        assert!(is_text_path("mimetype"));
        assert!(!is_text_path("a.png"));
    }
}

#[cfg(test)]
mod perf {
    use super::*;
    use std::time::{Duration, Instant};

    fn budget(release_ms: u64) -> Duration {
        let debug = if cfg!(debug_assertions) { 30.0 } else { 1.0 };
        let scale: f64 = std::env::var("BOOKX_PERF_SCALE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0);
        Duration::from_secs_f64(release_ms as f64 / 1000.0 * debug * scale)
    }

    fn timed<T>(what: &str, ms: u64, f: impl FnOnce() -> T) -> T {
        let t = Instant::now();
        let out = f();
        let took = t.elapsed();
        assert!(
            took <= budget(ms),
            "{what} took {took:?}, budget {:?}",
            budget(ms)
        );
        out
    }

    /// About 2 MB of XHTML, 200 files of 10 KB, like a large book.
    fn files() -> Vec<String> {
        (0..200)
            .map(|f| {
                let mut s = String::from("<html><body>\n");
                for p in 0..60 {
                    s.push_str(&format!(
                        "<p class=\"c{}\">Paragraph {f}-{p}: the quick brown fox jumps over the lazy dog, again and again.</p>\n",
                        p % 5
                    ));
                }
                s.push_str("</body></html>");
                s
            })
            .collect()
    }

    #[test]
    fn searching_and_replacing_across_a_large_book() {
        let files = files();
        let total: usize = files.iter().map(|f| f.len()).sum();
        assert!(total > 1_000_000);

        let literal = Pattern::new("lazy dog", Opts::default()).unwrap();
        let n = timed("literal find_all over all files", 500, || {
            files
                .iter()
                .map(|f| literal.find_all(f, None).len())
                .sum::<usize>()
        });
        assert_eq!(n, 200 * 60);

        let re = Pattern::new(
            r#"class="c(\d)">Paragraph (\d+)-"#,
            Opts {
                regex: true,
                ..Opts::default()
            },
        )
        .unwrap();
        let n = timed("regex find_all over all files", 1000, || {
            files
                .iter()
                .map(|f| re.find_all(f, None).len())
                .sum::<usize>()
        });
        assert_eq!(n, 200 * 60);

        let (out, n) = timed("regex replace_all over all files", 1500, || {
            let mut n = 0;
            let out: Vec<String> = files
                .iter()
                .map(|f| {
                    let (t, k) = re.replace_all(f, r#"class="x$1">P$2-"#, None);
                    n += k;
                    t
                })
                .collect();
            (out, n)
        });
        assert_eq!(n, 200 * 60);
        assert!(out[0].contains(r#"class="x0">P0-0"#));

        timed("find_at stepping through one file", 500, || {
            let f = &files[0];
            let (mut from, mut hits) = (0, 0);
            while let Some((_, e)) = literal.find_at(f, from) {
                from = e;
                hits += 1;
            }
            assert_eq!(hits, 60);
        });
    }

    #[test]
    fn scoping_thousands_of_files_is_instant() {
        let all: Vec<String> = (0..5000)
            .map(|i| format!("OEBPS/text/ch{i}.xhtml"))
            .chain((0..2000).map(|i| format!("OEBPS/img/{i}.png")))
            .collect();
        let n = timed("scope_files", 20, || {
            scope_files(Scope::TextFiles, None, &all, &[], &[]).len()
                + scope_files(Scope::AllFiles, None, &all, &[], &[]).len()
        });
        assert_eq!(n, 10_000);
    }
}
