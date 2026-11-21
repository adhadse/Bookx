//! In-book text search with whole-word, case and regex options, context snippets,
//! a result limit and cooperative cancellation.

use anyhow::{anyhow, bail, Result};
use regex::{Regex, RegexBuilder};
use std::sync::atomic::{AtomicBool, Ordering};

pub const MAX_QUERY_CHARS: usize = 1000;
const REGEX_SIZE_LIMIT: usize = 1 << 20;
const MAX_SNIPPET_MATCH_CHARS: usize = 200;

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub whole_word: bool,
    pub case_sensitive: bool,
    /// Treat the query as a regular expression (`^`/`$` match at line boundaries).
    pub regex: bool,
    pub max_results: usize,
    /// Characters of context kept on each side of a match.
    pub context_chars: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            whole_word: false,
            case_sensitive: false,
            regex: false,
            max_results: 500,
            context_chars: 40,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub chapter: usize,
    /// Byte offset of the match inside the chapter text.
    pub offset: usize,
    /// Byte length of the match.
    pub len: usize,
    /// Context with `…` where it was cut and whitespace runs collapsed.
    pub snippet: String,
    /// Byte range of the match inside `snippet`.
    pub match_range: (usize, usize),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchOutcome {
    pub hits: Vec<SearchHit>,
    /// More matches exist than `max_results`.
    pub truncated: bool,
    pub cancelled: bool,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Compiles the query. `Ok(None)` for an empty query.
pub fn compile(query: &str, opts: &SearchOptions) -> Result<Option<Regex>> {
    if query.is_empty() {
        return Ok(None);
    }
    if query.chars().count() > MAX_QUERY_CHARS {
        bail!("search query is longer than {MAX_QUERY_CHARS} characters");
    }
    let pattern = if opts.regex {
        if opts.whole_word {
            format!(r"\b(?:{query})\b")
        } else {
            query.to_string()
        }
    } else {
        let mut p = regex::escape(query);
        if opts.whole_word {
            if query.chars().next().is_some_and(is_word_char) {
                p = format!(r"\b{p}");
            }
            if query.chars().last().is_some_and(is_word_char) {
                p = format!(r"{p}\b");
            }
        }
        p
    };
    let re = RegexBuilder::new(&pattern)
        .case_insensitive(!opts.case_sensitive)
        .multi_line(opts.regex)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT * 8)
        .build()
        .map_err(|e| anyhow!("invalid search pattern: {e}"))?;
    Ok(Some(re))
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out
}

/// Builds the snippet and the match range inside it.
pub fn make_snippet(
    text: &str,
    start: usize,
    end: usize,
    context_chars: usize,
) -> (String, (usize, usize)) {
    let before_all = &text[..start];
    let after_all = &text[end..];
    let (before, cut_before) = if context_chars == 0 {
        ("", !before_all.is_empty())
    } else {
        match before_all.char_indices().rev().nth(context_chars - 1) {
            Some((i, _)) => (&before_all[i..], i > 0),
            None => (before_all, false),
        }
    };
    let (after, cut_after) = match after_all.char_indices().nth(context_chars) {
        Some((i, _)) => (&after_all[..i], true),
        None => (after_all, false),
    };
    let matched_full = &text[start..end];
    let (matched, cut_match) = match matched_full.char_indices().nth(MAX_SNIPPET_MATCH_CHARS) {
        Some((i, _)) => (&matched_full[..i], true),
        None => (matched_full, false),
    };
    let mut snippet = String::new();
    if cut_before {
        snippet.push('…');
    }
    snippet.push_str(&collapse_ws(before));
    let ms = snippet.len();
    snippet.push_str(&collapse_ws(matched));
    let me = snippet.len();
    if cut_match {
        snippet.push('…');
    }
    snippet.push_str(&collapse_ws(after));
    if cut_after {
        snippet.push('…');
    }
    (snippet, (ms, me))
}

/// Searches all chapters in order. `cancel` is polled between matches and chapters.
pub fn search<S: AsRef<str>>(
    chapters: &[S],
    query: &str,
    opts: &SearchOptions,
    cancel: Option<&AtomicBool>,
) -> Result<SearchOutcome> {
    if opts.max_results == 0 {
        bail!("max_results must be at least 1");
    }
    let mut out = SearchOutcome::default();
    let Some(re) = compile(query, opts)? else {
        return Ok(out);
    };
    let cancelled = || cancel.is_some_and(|c| c.load(Ordering::Relaxed));
    'chapters: for (ci, chapter) in chapters.iter().enumerate() {
        let text = chapter.as_ref();
        let mut pos = 0;
        while pos <= text.len() {
            if cancelled() {
                out.cancelled = true;
                break 'chapters;
            }
            let Some(m) = re.find_at(text, pos) else {
                break;
            };
            let next_char = |from: usize| {
                text[from..]
                    .chars()
                    .next()
                    .map_or(text.len() + 1, |c| from + c.len_utf8())
            };
            if m.is_empty() {
                pos = next_char(m.end());
                continue;
            }
            if out.hits.len() == opts.max_results {
                out.truncated = true;
                break 'chapters;
            }
            let (snippet, match_range) = make_snippet(text, m.start(), m.end(), opts.context_chars);
            out.hits.push(SearchHit {
                chapter: ci,
                offset: m.start(),
                len: m.len(),
                snippet,
                match_range,
            });
            // literal searches report overlapping occurrences, regexes continue after the match
            pos = if opts.regex {
                m.end()
            } else {
                next_char(m.start())
            };
        }
    }
    Ok(out)
}

/// The non-overlapping matches of `re` in `text` as `(first char, end char)` pairs, at most `limit`;
/// empty matches are skipped. This is the search of one page or paragraph: positions are in
/// characters, like the reader's own text offsets.
pub fn char_ranges(re: &Regex, text: &str, limit: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    // byte offset of every char start, to turn the regex's byte offsets into char offsets
    let mut chars_before = 0usize;
    let mut last_byte = 0usize;
    for m in re.find_iter(text) {
        if m.is_empty() {
            continue;
        }
        if out.len() >= limit {
            break;
        }
        chars_before += text[last_byte..m.start()].chars().count();
        let len = text[m.start()..m.end()].chars().count();
        out.push((chars_before, chars_before + len));
        chars_before += len;
        last_byte = m.end();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_ranges_count_characters_not_bytes() {
        let o = SearchOptions::default();
        let re = compile("é", &o).unwrap().unwrap();
        assert_eq!(
            char_ranges(&re, "aé éé b", 10),
            vec![(1, 2), (3, 4), (4, 5)]
        );
        let re = compile(
            "b+",
            &SearchOptions {
                regex: true,
                ..o.clone()
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(char_ranges(&re, "日本 bbb x b", 10), vec![(3, 6), (9, 10)]);
        assert_eq!(char_ranges(&re, "日本 bbb x b", 1), vec![(3, 6)]);
        let empty = compile("x*", &SearchOptions { regex: true, ..o })
            .unwrap()
            .unwrap();
        assert_eq!(
            char_ranges(&empty, "ax", 10),
            vec![(1, 2)],
            "empty matches are skipped"
        );
    }

    #[test]
    fn whole_word_matches_words_only_in_the_page_search() {
        let o = SearchOptions {
            whole_word: true,
            ..Default::default()
        };
        let re = compile("cat", &o).unwrap().unwrap();
        assert_eq!(
            char_ranges(&re, "cat concat cat. cats", 10),
            vec![(0, 3), (11, 14)]
        );
    }

    fn run(chapters: &[&str], q: &str, o: &SearchOptions) -> Vec<(usize, usize)> {
        search(chapters, q, o, None)
            .unwrap()
            .hits
            .iter()
            .map(|h| (h.chapter, h.offset))
            .collect()
    }

    #[test]
    fn plain_case_insensitive_default() {
        let o = SearchOptions::default();
        assert_eq!(
            run(&["Hello hello HELLO"], "hello", &o),
            vec![(0, 0), (0, 6), (0, 12)]
        );
    }

    #[test]
    fn case_sensitive() {
        let o = SearchOptions {
            case_sensitive: true,
            ..Default::default()
        };
        assert_eq!(run(&["Hello hello HELLO"], "hello", &o), vec![(0, 6)]);
    }

    #[test]
    fn whole_word_literal() {
        let o = SearchOptions {
            whole_word: true,
            ..Default::default()
        };
        assert_eq!(
            run(&["cat concat cat. cats _cat cat_ cat"], "cat", &o),
            vec![(0, 0), (0, 11), (0, 31)]
        );
        // query edges that are not word characters need no boundary
        assert_eq!(run(&["a c++ b c++x"], "c++", &o), vec![(0, 2), (0, 8)]);
        assert_eq!(run(&["x (go) y"], "(go)", &o), vec![(0, 2)]);
    }

    #[test]
    fn whole_word_unicode() {
        let o = SearchOptions {
            whole_word: true,
            ..Default::default()
        };
        assert_eq!(run(&["café cafés café"], "café", &o), vec![(0, 0), (0, 13)]);
    }

    #[test]
    fn regex_mode_and_whole_word_regex() {
        let o = SearchOptions {
            regex: true,
            ..Default::default()
        };
        assert_eq!(
            run(&["a1 b22 c333"], r"\d+", &o),
            vec![(0, 1), (0, 4), (0, 8)]
        );
        let ow = SearchOptions {
            regex: true,
            whole_word: true,
            ..Default::default()
        };
        assert_eq!(
            run(&["foo foobar bar|foo"], "foo|bar", &ow),
            vec![(0, 0), (0, 11), (0, 15)]
        );
    }

    #[test]
    fn regex_multiline_anchors() {
        let o = SearchOptions {
            regex: true,
            ..Default::default()
        };
        assert_eq!(run(&["one\ntwo\nthree"], "^t", &o), vec![(0, 4), (0, 8)]);
        assert_eq!(run(&["one\ntwo\nthree"], "e$", &o), vec![(0, 2), (0, 12)]);
    }

    #[test]
    fn regex_is_not_used_in_literal_mode() {
        let o = SearchOptions::default();
        assert_eq!(run(&["a.b axb a.b"], "a.b", &o), vec![(0, 0), (0, 8)]);
        assert_eq!(run(&["(x) [y] $5"], "$5", &o), vec![(0, 8)]);
    }

    #[test]
    fn invalid_regex_errors_with_message() {
        let o = SearchOptions {
            regex: true,
            ..Default::default()
        };
        let e = search(&["x"], "(unclosed", &o, None)
            .unwrap_err()
            .to_string();
        assert!(e.starts_with("invalid search pattern"), "{e}");
        assert!(search(&["x"], "[z-a]", &o, None).is_err());
        assert!(search(&["x"], r"\p{Nope}", &o, None).is_err());
    }

    #[test]
    fn regex_size_limit_and_query_length() {
        let o = SearchOptions {
            regex: true,
            ..Default::default()
        };
        assert!(search(&["x"], "((a{100}){100}){100}", &o, None).is_err());
        let long = "a".repeat(MAX_QUERY_CHARS + 1);
        assert!(search(&["x"], &long, &SearchOptions::default(), None)
            .unwrap_err()
            .to_string()
            .contains("longer"));
        let ok = "a".repeat(MAX_QUERY_CHARS);
        assert!(search(&["x"], &ok, &SearchOptions::default(), None).is_ok());
    }

    #[test]
    fn empty_query_and_empty_inputs() {
        let o = SearchOptions::default();
        let r = search(&["abc"], "", &o, None).unwrap();
        assert_eq!(r, SearchOutcome::default());
        let none: [&str; 0] = [];
        assert!(search(&none, "a", &o, None).unwrap().hits.is_empty());
        assert!(search(&[""], "a", &o, None).unwrap().hits.is_empty());
        // regex that only matches the empty string yields no hits and terminates
        let ro = SearchOptions {
            regex: true,
            ..Default::default()
        };
        assert!(search(&["abc"], "x*", &ro, None).unwrap().hits.is_empty());
        assert!(search(&["日本"], "(?:)", &ro, None)
            .unwrap()
            .hits
            .is_empty());
    }

    #[test]
    fn regex_with_empty_matches_still_finds_real_ones() {
        let ro = SearchOptions {
            regex: true,
            ..Default::default()
        };
        assert_eq!(run(&["xaax"], "a*", &ro), vec![(0, 1)]);
    }

    #[test]
    fn overlapping_literal_matches() {
        let o = SearchOptions::default();
        assert_eq!(run(&["aaaa"], "aa", &o), vec![(0, 0), (0, 1), (0, 2)]);
        let ro = SearchOptions {
            regex: true,
            ..Default::default()
        };
        assert_eq!(run(&["aaaa"], "aa", &ro), vec![(0, 0), (0, 2)]);
        let w = SearchOptions {
            whole_word: true,
            ..Default::default()
        };
        assert_eq!(run(&["aaaa aa"], "aa", &w), vec![(0, 5)]);
    }

    #[test]
    fn multibyte_offsets_and_lengths() {
        let o = SearchOptions::default();
        let r = search(&["日本語日本"], "日本", &o, None).unwrap();
        assert_eq!(
            r.hits.iter().map(|h| (h.offset, h.len)).collect::<Vec<_>>(),
            vec![(0, 6), (9, 6)]
        );
        let text = "naïve 🦀 crab 🦀";
        let r = search(&[text], "🦀", &o, None).unwrap();
        for h in &r.hits {
            assert_eq!(&text[h.offset..h.offset + h.len], "🦀");
        }
        assert_eq!(r.hits.len(), 2);
        // overlapping stepping never splits a char
        let r = search(&["ééé"], "éé", &o, None).unwrap();
        assert_eq!(
            r.hits.iter().map(|h| h.offset).collect::<Vec<_>>(),
            vec![0, 2]
        );
    }

    #[test]
    fn unicode_case_folding() {
        let o = SearchOptions::default();
        assert_eq!(
            run(&["École ÉCOLE école"], "école", &o),
            vec![(0, 0), (0, 7), (0, 14)]
        );
        assert_eq!(run(&["ΣΊΣΥΦΟΣ"], "σίσυφος", &o).len(), 1);
        let cs = SearchOptions {
            case_sensitive: true,
            ..Default::default()
        };
        assert_eq!(run(&["École école"], "école", &cs), vec![(0, 7)]);
    }

    #[test]
    fn chapters_are_indexed_and_ordered() {
        let o = SearchOptions::default();
        assert_eq!(
            run(&["a", "ba", "", "aaa"], "a", &o),
            vec![(0, 0), (1, 1), (3, 0), (3, 1), (3, 2)]
        );
    }

    #[test]
    fn limit_and_truncation() {
        let o = SearchOptions {
            max_results: 2,
            ..Default::default()
        };
        let r = search(&["a a a"], "a", &o, None).unwrap();
        assert_eq!(r.hits.len(), 2);
        assert!(r.truncated);
        let r = search(&["a a"], "a", &o, None).unwrap();
        assert_eq!(r.hits.len(), 2);
        assert!(!r.truncated, "exactly at the limit is not truncated");
        let bad = SearchOptions {
            max_results: 0,
            ..Default::default()
        };
        assert!(search(&["a"], "a", &bad, None).is_err());
    }

    #[test]
    fn cancellation() {
        let flag = AtomicBool::new(true);
        let o = SearchOptions::default();
        let r = search(&["a a a"], "a", &o, Some(&flag)).unwrap();
        assert!(r.cancelled && r.hits.is_empty());
        let flag = AtomicBool::new(false);
        let r = search(&["a a a"], "a", &o, Some(&flag)).unwrap();
        assert!(!r.cancelled && r.hits.len() == 3);
    }

    #[test]
    fn snippets_with_ellipses_and_ranges() {
        let o = SearchOptions {
            context_chars: 5,
            ..Default::default()
        };
        let text = "0123456789 needle 0123456789";
        let h = &search(&[text], "needle", &o, None).unwrap().hits[0];
        assert_eq!(h.snippet, "…6789 needle 0123…");
        assert_eq!(&h.snippet[h.match_range.0..h.match_range.1], "needle");
        // no ellipsis when the text is short
        let h = &search(&["a needle b"], "needle", &o, None).unwrap().hits[0];
        assert_eq!(h.snippet, "a needle b");
        assert_eq!(&h.snippet[h.match_range.0..h.match_range.1], "needle");
        // exactly context_chars on a side: no ellipsis
        let h = &search(&["12345needle12345"], "needle", &o, None)
            .unwrap()
            .hits[0];
        assert_eq!(h.snippet, "12345needle12345");
    }

    #[test]
    fn snippets_collapse_whitespace_and_handle_unicode() {
        let o = SearchOptions {
            context_chars: 8,
            ..Default::default()
        };
        let text = "line one\n\n  line  two ✨ naïve found ☕ café\nlast";
        let h = &search(&[text], "found", &o, None).unwrap().hits[0];
        assert!(!h.snippet.contains('\n'));
        assert!(!h.snippet.contains("  "));
        assert_eq!(&h.snippet[h.match_range.0..h.match_range.1], "found");
        assert!(h.snippet.starts_with('…') && h.snippet.ends_with('…'));
    }

    #[test]
    fn snippet_zero_context_and_long_match() {
        let (s, r) = make_snippet("abc def ghi", 4, 7, 0);
        assert_eq!(&s[r.0..r.1], "def");
        assert_eq!(s, "…def…");
        let (s, r) = make_snippet("def", 0, 3, 0);
        assert_eq!((s.as_str(), r), ("def", (0, 3)));
        let long = "x".repeat(1000);
        let (s, r) = make_snippet(&long, 0, 1000, 3);
        assert_eq!(&s[r.0..r.1].chars().count(), &200);
        assert!(s.ends_with('…'));
        assert!(s.len() < 300);
    }

    #[test]
    fn snippet_at_text_edges_never_panics() {
        let text = "αβγδε ζηθ ικλ";
        for ctx in 0..12 {
            for (s, e) in [
                (0, 4),
                (4, 8),
                (text.len() - 6, text.len()),
                (0, text.len()),
            ] {
                let (snip, (a, b)) = make_snippet(text, s, e, ctx);
                assert!(a <= b && b <= snip.len());
                assert!(snip.is_char_boundary(a) && snip.is_char_boundary(b));
            }
        }
    }

    #[test]
    fn huge_text_with_limit_and_no_match() {
        let text = "lorem ipsum dolor ".repeat(60_000);
        let o = SearchOptions {
            max_results: 50,
            ..Default::default()
        };
        let r = search(&[text.as_str()], "ipsum", &o, None).unwrap();
        assert_eq!(r.hits.len(), 50);
        assert!(r.truncated);
        assert!(
            search(&[text.as_str()], "zzz", &SearchOptions::default(), None)
                .unwrap()
                .hits
                .is_empty()
        );
        let many = SearchOptions {
            max_results: 100_000,
            ..Default::default()
        };
        assert_eq!(
            search(&[text.as_str()], "dolor", &many, None)
                .unwrap()
                .hits
                .len(),
            60_000
        );
    }

    #[test]
    fn works_with_owned_strings() {
        let chapters = vec![String::from("one two"), String::from("two three")];
        let r = search(&chapters, "two", &SearchOptions::default(), None).unwrap();
        assert_eq!(r.hits.len(), 2);
    }

    #[test]
    fn compile_returns_none_for_empty() {
        assert!(compile("", &SearchOptions::default()).unwrap().is_none());
        assert!(compile("x", &SearchOptions::default()).unwrap().is_some());
    }
}
