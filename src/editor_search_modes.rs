//! The search modes of the editor behind one interface: plain text and regular expressions (optionally
//! ignoring HTML markup, CE-083), function mode (CE-100) and tag search (CE-085). No GTK in here.

use crate::i18n::tr;
use bookx_editing::search_ignore::{self, TextMatch};
use bookx_editing::tag_search::{self, TagEdit, TagQuery};
use serde::{Deserialize, Serialize};

use crate::editor_search::{Opts, Pattern};
use crate::editor_search_fn::Runner;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Normal,
    Regex,
    Function,
    Tag,
    /// Approximate matches that tolerate typing mistakes (CE-424).
    Fuzzy,
}

impl Mode {
    pub const ALL: [Mode; 5] = [
        Mode::Normal,
        Mode::Regex,
        Mode::Function,
        Mode::Tag,
        Mode::Fuzzy,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "Text",
            Mode::Regex => "Regex",
            Mode::Function => "Function",
            Mode::Tag => "Tag",
            Mode::Fuzzy => "Fuzzy",
        }
    }

    pub fn from_index(i: u32) -> Mode {
        Mode::ALL.get(i as usize).copied().unwrap_or_default()
    }

    pub fn index(self) -> u32 {
        Mode::ALL.iter().position(|m| *m == self).unwrap_or(0) as u32
    }

    /// The query is a regular expression.
    pub fn is_regex(self) -> bool {
        matches!(self, Mode::Regex | Mode::Function)
    }
}

/// Everything that defines what is searched for.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Settings {
    pub find: String,
    #[serde(default)]
    pub replace: String,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub dot_all: bool,
    #[serde(default)]
    pub whole_word: bool,
    /// Text and regex modes: match the text only, tags are skipped.
    #[serde(default)]
    pub ignore_markup: bool,
}

pub enum Engine {
    Text(Pattern),
    Markup(Pattern),
    Function(Pattern),
    Tag(TagQuery),
    Fuzzy {
        query: String,
        case_sensitive: bool,
        whole_word: bool,
    },
}

/// What Replace All needs besides the text: the function runner of function mode and the file name.
#[derive(Default)]
pub struct RunCtx<'a> {
    pub runner: Option<&'a mut Runner>,
    pub file: &'a str,
}

impl Engine {
    /// Compiles the settings; the error is a short message for the search bar.
    pub fn new(s: &Settings) -> Result<Engine, String> {
        if s.find.is_empty() {
            return Err("empty".into());
        }
        let opts = Opts {
            regex: s.mode.is_regex(),
            case_sensitive: s.case_sensitive,
            dot_all: s.dot_all,
            whole_word: s.whole_word,
        };
        match s.mode {
            Mode::Tag => tag_search::parse(&s.find)
                .map(Engine::Tag)
                .map_err(|e| format!("Tag query: {e}")),
            Mode::Fuzzy => Ok(Engine::Fuzzy {
                query: s.find.clone(),
                case_sensitive: s.case_sensitive,
                whole_word: s.whole_word,
            }),
            Mode::Function => Pattern::new(&s.find, opts)
                .map(Engine::Function)
                .map_err(|_| tr("Invalid regular expression")),
            _ => Pattern::new(&s.find, opts)
                .map(|p| {
                    if s.ignore_markup {
                        Engine::Markup(p)
                    } else {
                        Engine::Text(p)
                    }
                })
                .map_err(|_| tr("Invalid regular expression")),
        }
    }

    pub fn is_function(&self) -> bool {
        matches!(self, Engine::Function(_))
    }

    /// Every match as a source byte range, optionally only inside `ranges`.
    pub fn find_all(&self, text: &str, ranges: Option<&[(usize, usize)]>) -> Vec<(usize, usize)> {
        match self {
            Engine::Text(p) | Engine::Function(p) => p.find_all(text, ranges),
            Engine::Markup(p) => search_ignore::find_source_spans(text, |t| p.find_all(t, None))
                .into_iter()
                .filter(|m| ranges.is_none_or(|rs| rs.iter().any(|r| m.0 >= r.0 && m.1 <= r.1)))
                .collect(),
            Engine::Fuzzy {
                query,
                case_sensitive,
                whole_word,
            } => crate::editor_search_fuzzy::find_all(
                text,
                query,
                None,
                *case_sensitive,
                *whole_word,
            )
            .into_iter()
            .filter(|m| ranges.is_none_or(|rs| rs.iter().any(|r| m.0 >= r.0 && m.1 <= r.1)))
            .collect(),
            Engine::Tag(q) => q
                .find(text)
                .into_iter()
                .map(|h| (h.range.start, h.range.end))
                .filter(|m| ranges.is_none_or(|rs| rs.iter().any(|r| m.0 >= r.0 && m.1 <= r.1)))
                .collect(),
        }
    }

    pub fn find_at(&self, text: &str, from: usize) -> Option<(usize, usize)> {
        match self {
            Engine::Text(p) | Engine::Function(p) => p.find_at(text, from),
            _ => self.find_all(text, None).into_iter().find(|m| m.0 >= from),
        }
    }

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

    /// The text that replaces the match `start..end` (`None` when that range is not a match, or when the
    /// replacement does not apply: function mode replaces only in Replace All).
    pub fn replacement_for(
        &self,
        text: &str,
        start: usize,
        end: usize,
        replacement: &str,
    ) -> Option<String> {
        match self {
            Engine::Text(p) => p.replacement_for(text, start, end, replacement),
            Engine::Function(_) => None,
            Engine::Markup(p) => {
                let proj = search_ignore::project(text);
                p.replacements(&proj.text, replacement, None)
                    .into_iter()
                    .find(|(a, b, _)| proj.source_span(*a, *b) == Some((start, end)))
                    .map(|(a, b, r)| {
                        let edits = proj.edits(a, b, &r);
                        search_ignore::apply_edits_range(text, &edits, start, end)
                    })
            }
            Engine::Fuzzy { .. } => self
                .find_all(text, None)
                .contains(&(start, end))
                .then(|| replacement.to_string()),
            Engine::Tag(q) => {
                let edit = tag_search::parse_edit(replacement).ok()?;
                let tag = text.get(start..end)?;
                if !q.find(text).iter().any(|h| h.range == (start..end)) {
                    return None;
                }
                let (new, n) = q.apply(tag, &edit).ok()?;
                (n > 0).then_some(new)
            }
        }
    }

    /// Replaces every match; returns the new text and the number of replacements. Function mode calls the
    /// runner in `ctx`.
    pub fn replace_all(
        &self,
        text: &str,
        replacement: &str,
        ranges: Option<&[(usize, usize)]>,
        ctx: &mut RunCtx,
    ) -> Result<(String, usize), String> {
        match self {
            Engine::Text(p) => Ok(p.replace_all(text, replacement, ranges)),
            Engine::Markup(p) => {
                let (out, n) = search_ignore::replace_matches(text, |t| {
                    p.replacements(t, replacement, None)
                        .into_iter()
                        .map(|(start, end, replacement)| TextMatch {
                            start,
                            end,
                            replacement,
                        })
                        .collect()
                });
                if ranges.is_some() {
                    return Err(tr("Marked text can't be combined with “ignore markup”"));
                }
                Ok((out, n))
            }
            Engine::Function(p) => {
                let runner = ctx
                    .runner
                    .as_deref_mut()
                    .ok_or("Choose a function (Function mode)")?;
                runner
                    .run_text(p, text, ranges, ctx.file)
                    .map_err(|e| e.to_string())
            }
            Engine::Fuzzy { .. } => {
                let hits = self.find_all(text, ranges);
                let mut out = String::with_capacity(text.len());
                let mut at = 0;
                for (a, b) in &hits {
                    out.push_str(&text[at..*a]);
                    out.push_str(replacement);
                    at = *b;
                }
                out.push_str(&text[at..]);
                Ok((out, hits.len()))
            }
            Engine::Tag(q) => {
                if ranges.is_some() {
                    return Err(tr("Marked text can't be combined with tag search"));
                }
                let edit: TagEdit = tag_search::parse_edit(replacement)?;
                q.apply(text, &edit)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_search_fn::{Env, Function};

    fn s(find: &str, mode: Mode) -> Settings {
        Settings {
            find: find.into(),
            mode,
            case_sensitive: true,
            ..Default::default()
        }
    }

    #[test]
    fn text_and_regex_modes() {
        let e = Engine::new(&s("a.b", Mode::Normal)).unwrap();
        assert_eq!(e.find_all("a.b axb", None), vec![(0, 3)]);
        let e = Engine::new(&s("a.b", Mode::Regex)).unwrap();
        assert_eq!(e.find_all("a.b axb", None), vec![(0, 3), (4, 7)]);
        assert!(Engine::new(&s("(", Mode::Regex)).is_err());
        assert_eq!(
            Engine::new(&s("", Mode::Normal)).err().as_deref(),
            Some("empty")
        );
    }

    #[test]
    fn whole_word_and_dot_all_flags() {
        let mut st = s("cat", Mode::Normal);
        st.whole_word = true;
        let e = Engine::new(&st).unwrap();
        assert_eq!(e.find_all("cat concat", None), vec![(0, 3)]);
        let mut st = s("a.b", Mode::Regex);
        st.dot_all = true;
        assert_eq!(Engine::new(&st).unwrap().find_all("a\nb", None).len(), 1);
    }

    #[test]
    fn ignore_markup_finds_across_tags_and_replaces() {
        let mut st = s("on a word", Mode::Normal);
        st.ignore_markup = true;
        let e = Engine::new(&st).unwrap();
        let src = "<p>so on <i>a</i> word. on a word</p>";
        let hits = e.find_all(src, None);
        assert_eq!(hits.len(), 2);
        assert_eq!(&src[hits[0].0..hits[0].1], "on <i>a</i> word");
        let (out, n) = e
            .replace_all(src, "X", None, &mut RunCtx::default())
            .unwrap();
        assert_eq!((out.as_str(), n), ("<p>so X<i></i>. X</p>", 2));
        // replace one
        assert_eq!(
            e.replacement_for(src, hits[0].0, hits[0].1, "Y").as_deref(),
            Some("Y<i></i>")
        );
        assert_eq!(e.replacement_for(src, 0, 3, "Y"), None);
    }

    #[test]
    fn ignore_markup_with_regex_groups() {
        let mut st = s(r"(\w+) (\w+)", Mode::Regex);
        st.ignore_markup = true;
        let e = Engine::new(&st).unwrap();
        let (out, _) = e
            .replace_all(
                "<p>ab <b>cd</b></p>",
                r"\2 \1",
                None,
                &mut RunCtx::default(),
            )
            .unwrap();
        assert_eq!(out, "<p>cd ab<b></b></p>");
    }

    #[test]
    fn plain_matches_do_not_cross_into_tags_without_the_option() {
        let e = Engine::new(&s("on a word", Mode::Normal)).unwrap();
        assert!(e.find_all("<p>on <i>a</i> word</p>", None).is_empty());
    }

    #[test]
    fn tag_mode_finds_and_edits() {
        let e = Engine::new(&s(r#"img[alt=""]"#, Mode::Tag)).unwrap();
        let src = r#"<p><img src="a.png" alt=""/><img src="b.png" alt="x"/></p>"#;
        let hits = e.find_all(src, None);
        assert_eq!(hits.len(), 1);
        assert_eq!(&src[hits[0].0..hits[0].1], r#"<img src="a.png" alt=""/>"#);
        let (out, n) = e
            .replace_all(src, "alt=Picture", None, &mut RunCtx::default())
            .unwrap();
        assert_eq!(n, 1);
        assert!(out.contains(r#"alt="Picture""#));
        assert_eq!(
            e.replacement_for(src, hits[0].0, hits[0].1, "alt=Pic")
                .as_deref(),
            Some(r#"<img src="a.png" alt="Pic"/>"#)
        );
        assert!(e
            .replace_all(src, "bad", None, &mut RunCtx::default())
            .is_err());
        assert!(Engine::new(&s("img[", Mode::Tag)).is_err());
    }

    #[test]
    fn function_mode_needs_a_runner_and_replaces_through_it() {
        let e = Engine::new(&s("x", Mode::Function)).unwrap();
        assert!(e.is_function());
        assert!(e
            .replace_all("x", "", None, &mut RunCtx::default())
            .is_err());
        assert_eq!(e.replacement_for("x", 0, 1, ""), None);
        let f = Function::new(
            "t",
            "",
            "fn replace(m, number, file, meta, data) { number }",
        );
        let mut r = Runner::new(&f, &Env::default()).unwrap();
        let (out, n) = e
            .replace_all(
                "x x",
                "",
                None,
                &mut RunCtx {
                    runner: Some(&mut r),
                    file: "f",
                },
            )
            .unwrap();
        assert_eq!((out.as_str(), n), ("1 2", 2));
    }

    #[test]
    fn find_helpers_work_for_every_engine() {
        let e = Engine::new(&s("a", Mode::Normal)).unwrap();
        assert_eq!(e.find_at("bab", 0), Some((1, 2)));
        assert_eq!(e.find_before("aba", 3, None), Some((2, 3)));
        assert_eq!(e.find_before("aba", 2, None), Some((0, 1)));
        let mut st = s("b", Mode::Normal);
        st.ignore_markup = true;
        let m = Engine::new(&st).unwrap();
        assert_eq!(m.find_at("<p>ab</p>", 0), Some((4, 5)));
    }

    #[test]
    fn fuzzy_mode_finds_and_replaces_near_matches() {
        let e = Engine::new(&s("receive", Mode::Fuzzy)).unwrap();
        let src = "<p>We recieve and receive.</p>";
        let hits = e.find_all(src, None);
        assert_eq!(hits.len(), 2);
        assert_eq!(&src[hits[0].0..hits[0].1], "recieve");
        let (out, n) = e
            .replace_all(src, "get", None, &mut RunCtx::default())
            .unwrap();
        assert_eq!((out.as_str(), n), ("<p>We get and get.</p>", 2));
        assert_eq!(
            e.replacement_for(src, hits[0].0, hits[0].1, "get")
                .as_deref(),
            Some("get")
        );
        assert_eq!(e.replacement_for(src, 0, 3, "get"), None);
        // marked ranges limit the matches
        let inside = [(0, 14)];
        assert_eq!(e.find_all(src, Some(&inside)).len(), 1);
        assert!(!Mode::Fuzzy.is_regex());
        assert_eq!(Mode::from_index(4), Mode::Fuzzy);
    }

    #[test]
    fn mode_indexes_round_trip() {
        for (i, m) in Mode::ALL.iter().enumerate() {
            assert_eq!(Mode::from_index(i as u32), *m);
            assert_eq!(m.index(), i as u32);
            assert!(!m.label().is_empty());
        }
        assert_eq!(Mode::from_index(99), Mode::Normal);
    }
}
