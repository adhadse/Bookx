//! Your own vocabulary as a glossary: reading a pasted glossary or a `term;definition` CSV, writing
//! the CSV again, and writing the terms as a StarDict dictionary ("My Vocabulary") for other apps.

use std::path::Path;

use anyhow::{Context, Result};

use crate::vocab::{VocabEntry, VocabStore};

/// One line of a glossary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlossaryItem {
    pub term: String,
    pub definition: String,
    pub note: String,
    pub tags: Vec<String>,
}

/// Rows of CSV text with the given delimiter: quoted fields may hold the delimiter, doubled quotes
/// and line breaks.
fn csv_rows(text: &str, delim: char) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        any = true;
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            c if c == delim => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                any = false;
            }
            c => field.push(c),
        }
    }
    if any || !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows.retain(|r| r.iter().any(|f| !f.trim().is_empty()));
    rows
}

/// The delimiter a glossary uses: the first of tab, `;` and `,` found on its first line.
fn delimiter(text: &str) -> Option<char> {
    let first = text.lines().find(|l| !l.trim().is_empty())?;
    ['\t', ';', ','].into_iter().find(|d| first.contains(*d))
}

/// Splits `term — definition` (also `–`, ` - ` and `:`) on its first separator.
fn split_dash(line: &str) -> Option<(&str, &str)> {
    for sep in [" — ", " – ", " - ", ": ", " = "] {
        if let Some((a, b)) = line.split_once(sep) {
            return Some((a, b));
        }
    }
    None
}

/// Reads a glossary: `term;definition` (or tab or comma separated, CSV quoting understood, a
/// header row `term;definition` skipped; further columns are the note and the tags), or lines of
/// `term — definition`. Lines without a definition are skipped.
pub fn parse_glossary(text: &str) -> Vec<GlossaryItem> {
    let mut out = Vec::new();
    let item = |t: &str, d: &str, n: &str, g: &str| -> Option<GlossaryItem> {
        let (t, d) = (t.trim(), d.trim());
        (!t.is_empty() && !d.is_empty()).then(|| GlossaryItem {
            term: t.to_string(),
            definition: d.to_string(),
            note: n.trim().to_string(),
            tags: g
                .split([',', ' '])
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .map(str::to_string)
                .collect(),
        })
    };
    match delimiter(text) {
        Some(d) => {
            for (i, r) in csv_rows(text, d).iter().enumerate() {
                let get = |n: usize| r.get(n).map(String::as_str).unwrap_or("");
                if i == 0
                    && matches!(
                        get(0).trim().to_lowercase().as_str(),
                        "term" | "word" | "headword"
                    )
                {
                    continue;
                }
                // a line of a mixed glossary that has no delimiter may still be `term — definition`
                if r.len() == 1 {
                    if let Some((t, d)) = split_dash(get(0).trim()) {
                        out.extend(item(t, d, "", ""));
                    }
                    continue;
                }
                if let Some(it) = item(get(0), get(1), get(2), get(3)) {
                    out.push(it);
                }
            }
        }
        None => {
            for line in text.lines() {
                if let Some((t, d)) = split_dash(line.trim()) {
                    if let Some(it) = item(t, d, "", "") {
                        out.push(it);
                    }
                }
            }
        }
    }
    // a term that comes twice keeps its last definition
    let mut seen: Vec<GlossaryItem> = Vec::new();
    for it in out {
        match seen
            .iter_mut()
            .find(|s| s.term.to_lowercase() == it.term.to_lowercase())
        {
            Some(s) => *s = it,
            None => seen.push(it),
        }
    }
    seen
}

fn csv_field(s: &str) -> String {
    if s.contains([';', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// `term;definition;note;tags` with a header row, for the entries that have your own definition
/// (`only_mine`) or all of them (the shown definition is yours, else the dictionary's).
pub fn export_csv(store: &VocabStore, only_mine: bool) -> String {
    let mut out = String::from("term;definition;note;tags\n");
    for e in store.entries() {
        if only_mine && e.own.is_none() {
            continue;
        }
        let Some(d) = e.shown_definition() else {
            continue;
        };
        out.push_str(&format!(
            "{};{};{};{}\n",
            csv_field(&e.word),
            csv_field(d),
            csv_field(&e.note),
            csv_field(&e.tags.join(" "))
        ));
    }
    out
}

/// Adds a parsed glossary to the store, scope for all books. Returns `(added, updated)`.
pub fn import_glossary(
    store: &mut VocabStore,
    items: &[GlossaryItem],
    scope: &crate::vocab::Scope,
    language: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> (usize, usize) {
    let (mut added, mut updated) = (0, 0);
    for it in items {
        let t = crate::vocab::OwnTerm {
            term: it.term.clone(),
            definition: it.definition.clone(),
            note: it.note.clone(),
            tags: it.tags.clone(),
            language: language.to_string(),
            scope: scope.clone(),
            ..Default::default()
        };
        match store.set_own(&t, now) {
            Ok(true) => added += 1,
            Ok(false) => updated += 1,
            Err(_) => {}
        }
    }
    (added, updated)
}

/// StarDict's order for the index: case-insensitive on ASCII, then by bytes.
fn stardict_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (x, y) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    x.as_bytes()
        .cmp(y.as_bytes())
        .then(a.as_bytes().cmp(b.as_bytes()))
}

fn dict_text(e: &VocabEntry) -> String {
    let mut t = e.own.clone().unwrap_or_default();
    if !e.note.is_empty() {
        t.push_str("\n\n");
        t.push_str(&e.note);
    }
    t
}

/// Writes the entries with your own definition as a StarDict dictionary `name.ifo/.idx/.dict` in
/// `dir`. Returns the number of terms written.
pub fn export_stardict(store: &VocabStore, dir: &Path, name: &str) -> Result<usize> {
    let mut entries: Vec<&VocabEntry> = store.own_entries();
    entries.sort_by(|a, b| stardict_cmp(&a.word, &b.word));
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let (mut idx, mut dict) = (Vec::new(), Vec::new());
    for e in &entries {
        let body = dict_text(e);
        idx.extend_from_slice(e.word.as_bytes());
        idx.push(0);
        idx.extend_from_slice(&(dict.len() as u32).to_be_bytes());
        idx.extend_from_slice(&(body.len() as u32).to_be_bytes());
        dict.extend_from_slice(body.as_bytes());
    }
    let base = dir.join("my-vocabulary");
    std::fs::write(base.with_extension("idx"), &idx)?;
    std::fs::write(base.with_extension("dict"), &dict)?;
    std::fs::write(
        base.with_extension("ifo"),
        format!(
            "StarDict's dict ifo file\nversion=2.4.2\nbookname={name}\nwordcount={}\nidxfilesize={}\nsametypesequence=m\ndescription=Terms with your own definitions, exported from Bookx\n",
            entries.len(),
            idx.len()
        ),
    )?;
    Ok(entries.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stardict::StarDict;
    use crate::vocab::{OwnTerm, Scope};
    use chrono::{TimeZone, Utc};

    fn now() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 20, 9, 0, 0).unwrap()
    }

    fn store() -> VocabStore {
        let mut s = VocabStore::new();
        s.set_own(
            &OwnTerm {
                term: "Zorb".into(),
                definition: "a rolling\nball; \"fun\"".into(),
                note: "ch. 3".into(),
                tags: vec!["sci-fi".into()],
                ..Default::default()
            },
            now(),
        )
        .unwrap();
        s.set_own(
            &OwnTerm {
                term: "alpha".into(),
                definition: "first".into(),
                ..Default::default()
            },
            now(),
        )
        .unwrap();
        s.add("plain", None, None, now()).unwrap();
        s
    }

    #[test]
    fn csv_with_header_delimiters_and_quotes() {
        let t =
            "term;definition\nZorb;\"a rolling\nball; \"\"fun\"\"\"\nalpha;first;a note;t1 t2\n";
        let g = parse_glossary(t);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].term, "Zorb");
        assert_eq!(g[0].definition, "a rolling\nball; \"fun\"");
        assert_eq!(g[1].note, "a note");
        assert_eq!(g[1].tags, ["t1", "t2"]);
        let tab = parse_glossary("a\tone\nb\ttwo\n");
        assert_eq!(tab.len(), 2);
        let comma = parse_glossary("term,definition\nx,y");
        assert_eq!(comma.len(), 1);
        assert_eq!(comma[0].definition, "y");
    }

    #[test]
    fn a_pasted_glossary_of_dashed_lines() {
        let g =
            parse_glossary("Zorb — a rolling ball\n\nflux - a flow\nno definition here\nA: b\n");
        let terms: Vec<_> = g.iter().map(|i| i.term.as_str()).collect();
        assert_eq!(terms, ["Zorb", "flux", "A"]);
        assert_eq!(g[1].definition, "a flow");
    }

    #[test]
    fn a_glossary_may_mix_delimiters_and_dashes() {
        let g = parse_glossary("alpha; first letter\nbeta — second letter\nnodef\n");
        let terms: Vec<_> = g.iter().map(|i| i.term.as_str()).collect();
        assert_eq!(terms, ["alpha", "beta"]);
    }

    #[test]
    fn duplicates_keep_the_last_definition_and_empty_rows_are_skipped() {
        let g = parse_glossary("a;1\nA;2\n;nothing\nb;\n");
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].definition, "2");
        assert!(parse_glossary("").is_empty());
        assert!(parse_glossary("just words").is_empty());
    }

    #[test]
    fn export_then_import_round_trips() {
        let s = store();
        let csv = export_csv(&s, true);
        assert_eq!(csv.lines().next(), Some("term;definition;note;tags"));
        let back = parse_glossary(&csv);
        assert_eq!(back.len(), 2);
        let z = back.iter().find(|i| i.term == "Zorb").unwrap();
        assert_eq!(z.definition, "a rolling\nball; \"fun\"");
        assert_eq!(z.note, "ch. 3");
        assert_eq!(z.tags, ["sci-fi"]);
        assert!(!export_csv(&s, true).contains("plain"));
        // without a definition a word has nothing to export
        assert!(!export_csv(&s, false).contains("plain"));
    }

    #[test]
    fn import_adds_and_updates() {
        let mut s = store();
        let items = parse_glossary("alpha;new meaning\nbeta;second");
        let (a, u) = import_glossary(&mut s, &items, &Scope::Book("7".into()), "en", now());
        assert_eq!((a, u), (1, 1));
        let e = s.find("alpha").unwrap();
        assert_eq!(e.own.as_deref(), Some("new meaning"));
        assert_eq!(e.scope, Scope::Book("7".into()));
        assert_eq!(e.language, "en");
    }

    #[test]
    fn stardict_export_can_be_read_back_in_order() {
        let tmp = crate::testutil::TmpDir::new("own");
        let dir = tmp.path().to_path_buf();
        let n = export_stardict(&store(), &dir, "My Vocabulary").unwrap();
        assert_eq!(n, 2);
        let d = StarDict::open(&dir).unwrap();
        assert_eq!(d.name(), "My Vocabulary");
        assert_eq!(d.len(), 2);
        let r = d.lookup("zorb").unwrap();
        assert_eq!(r.len(), 1);
        assert!(r[0].plain_text().starts_with("a rolling\nball; \"fun\""));
        assert!(r[0].plain_text().contains("ch. 3"));
    }
}
