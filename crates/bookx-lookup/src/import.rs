//! Annotation import: parsers for what other readers and tools write, all pure text in, a list of
//! [`Imported`] out. Nothing here knows about the library; matching books and storing highlights is
//! the application's business.
//!
//! * Kindle `My Clippings.txt` ([`parse_kindle_clippings`]), also what KOReader's "My Clippings"
//!   exporter and Bookx's own Kindle export write.
//! * KOReader's highlight export as JSON ([`parse_json`]) and Markdown ([`parse_koreader_markdown`]).
//! * Bookx's own JSON exports and a calibre annotation collection ([`parse_json`] recognises the
//!   shape; calibre's file is read leniently, see the function).
//!
//! KOReader sidecars (`metadata.*.lua`) and the Kobo database are read by `bookx-serve`, which
//! already knows those formats.

use anyhow::{bail, Result};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use serde_json::Value;

/// A clipping with its kind and location range while the Kindle file is being read.
type Entry = (Imported, Kind, Option<(u32, u32)>);

/// A KOReader Markdown entry being collected: page, time, lines.
type Pending = Option<(Option<String>, Option<DateTime<Utc>>, Vec<String>)>;

/// One highlight, note or bookmark found in a foreign file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    pub book_title: String,
    pub book_author: Option<String>,
    /// The highlighted text; empty for a bookmark or a note that came without its quote.
    pub text: String,
    /// What the reader wrote; empty when nothing.
    pub note: String,
    /// Page or location, as the source says it.
    pub location: Option<String>,
    pub chapter: Option<String>,
    pub created: Option<DateTime<Utc>>,
    pub bookmark: bool,
    /// The colour as the source wrote it (a name, `#rrggbb`); `None` when the source has none
    /// (Kindle clippings never have). Mapped by `bookx_core::devstyle`.
    pub color: Option<String>,
    /// The style as the source wrote it (`lighten`, `underscore`, `strikeout`, `underline`, ...).
    pub style: Option<String>,
}

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// The authors of a foreign file as a list: `Last, First` becomes `First Last`, several authors
/// are split at `;` and ` & `.
pub fn split_authors(author: &str) -> Vec<String> {
    author
        .split([';', '&'])
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(|a| match a.split_once(',') {
            Some((last, first)) if !first.contains(',') && !first.trim().is_empty() => {
                format!("{} {}", first.trim(), last.trim())
            }
            _ => a.to_string(),
        })
        .collect()
}

// -- Kindle -----------------------------------------------------------------------------------

/// Parses a `(` … `)` author suffix off a clippings title line.
fn title_and_author(line: &str) -> (String, Option<String>) {
    let line = line.trim().trim_start_matches('\u{feff}').trim();
    if let Some(body) = line.strip_suffix(')') {
        let mut depth = 0usize;
        for (i, c) in body.char_indices().rev() {
            match c {
                ')' => depth += 1,
                '(' if depth > 0 => depth -= 1,
                '(' => {
                    let title = body[..i].trim();
                    let author = body[i + 1..].trim();
                    if !title.is_empty() && !author.is_empty() {
                        return (title.to_string(), Some(author.to_string()));
                    }
                    break;
                }
                _ => {}
            }
        }
    }
    (line.to_string(), None)
}

fn parse_kindle_date(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    // drop a weekday: "Monday, 3 March 2014 10:42:11"
    let s = match s.split_once(", ") {
        Some((day, rest)) if !day.chars().any(|c| c.is_ascii_digit()) => rest,
        _ => s,
    };
    for f in [
        "%d %B %Y %H:%M:%S",
        "%B %d, %Y %I:%M:%S %p",
        "%B %d, %Y %H:%M:%S",
        "%d %B %Y %I:%M:%S %p",
        "%Y-%m-%d %H:%M:%S",
    ] {
        if let Ok(t) = NaiveDateTime::parse_from_str(s, f) {
            return Some(Utc.from_utc_datetime(&t));
        }
    }
    None
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Highlight,
    Note,
    Bookmark,
}

fn kind_of(meta: &str) -> Option<Kind> {
    let m = meta.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| m.contains(w));
    if has(&[
        "bookmark",
        "lesezeichen",
        "signet",
        "marcador",
        "segnalibro",
    ]) {
        Some(Kind::Bookmark)
    } else if has(&[
        "highlight",
        "markierung",
        "surlignement",
        "subrayado",
        "evidenziazione",
    ]) {
        Some(Kind::Highlight)
    } else if has(&["note", "notiz", "nota"]) {
        Some(Kind::Note)
    } else {
        None
    }
}

/// `(start, end)` of a location or page label such as `183-184` or `12`.
fn range(label: &str) -> Option<(u32, u32)> {
    let (a, b) = match label.split_once('-') {
        Some((a, b)) => (a.trim().parse().ok()?, b.trim().parse().ok()?),
        None => {
            let v = label.trim().parse().ok()?;
            (v, v)
        }
    };
    Some((a, b.max(a)))
}

fn word_after<'a>(meta: &'a str, word: &str) -> Option<&'a str> {
    let lower = meta.to_lowercase();
    let i = lower.find(word)? + word.len();
    let rest = meta[i..].trim_start();
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '|')
        .unwrap_or(rest.len());
    Some(rest[..end].trim_end_matches(','))
}

/// Parses a Kindle `My Clippings.txt`. A *Note* is attached to the highlight whose location range
/// contains it (the Kindle stores them as separate entries); a note without a highlight comes out
/// with an empty `text`. Entries are in file order. Titles, the "Added on" dates and the entry types
/// are recognised in English (and the type words in a few other languages); anything else is kept
/// without a date.
pub fn parse_kindle_clippings(text: &str) -> Vec<Imported> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut out: Vec<Entry> = Vec::new();
    for block in text.split("\n==========") {
        let mut lines = block.lines().skip_while(|l| l.trim().is_empty());
        let Some(head) = lines.next() else { continue };
        let Some(meta) = lines.next() else { continue };
        let Some(kind) = kind_of(meta) else { continue };
        let body: Vec<&str> = lines.collect();
        let body = body.join("\n").trim().to_string();
        let (title, author) = title_and_author(head);
        if title.is_empty() {
            continue;
        }
        let page = word_after(meta, "page").map(str::to_string);
        let loc = word_after(meta, "location")
            .or_else(|| word_after(meta, "position"))
            .map(str::to_string);
        let rng = loc
            .as_deref()
            .and_then(range)
            .or_else(|| page.as_deref().and_then(range));
        let created = meta
            .split_once("Added on ")
            .or_else(|| meta.split_once("Hinzugefügt am "))
            .and_then(|(_, d)| parse_kindle_date(d));
        let author = author.map(|a| split_authors(&a).join("; "));
        let (text, note) = match kind {
            Kind::Highlight => (body, String::new()),
            Kind::Note => (String::new(), body),
            Kind::Bookmark => (String::new(), String::new()),
        };
        out.push((
            Imported {
                book_title: title,
                book_author: author,
                text,
                note,
                location: loc.or(page),
                chapter: None,
                created,
                bookmark: kind == Kind::Bookmark,
                color: None,
                style: None,
            },
            kind,
            rng,
        ));
    }
    // attach each note to the highlight it was made on
    let mut gone = vec![false; out.len()];
    for i in 0..out.len() {
        if out[i].1 != Kind::Note {
            continue;
        }
        let Some((a, b)) = out[i].2 else {
            // no location to go by: a note straight after a highlight of the same book belongs to it
            if i > 0
                && out[i - 1].1 == Kind::Highlight
                && out[i - 1].0.note.is_empty()
                && out[i - 1].0.book_title == out[i].0.book_title
            {
                out[i - 1].0.note = out[i].0.note.clone();
                gone[i] = true;
            }
            continue;
        };
        let target = (0..out.len()).find(|&j| {
            out[j].1 == Kind::Highlight
                && out[j].0.note.is_empty()
                && out[j].0.book_title == out[i].0.book_title
                && out[j]
                    .2
                    .is_some_and(|(s, e)| (s <= a && a <= e) || (s <= b && b <= e))
        });
        if let Some(j) = target {
            let note = out[i].0.note.clone();
            out[j].0.note = note;
            gone[i] = true;
        }
    }
    out.into_iter()
        .zip(gone)
        .filter(|(_, g)| !*g)
        .map(|((i, _, _), _)| i)
        .collect()
}

// -- KOReader's exporter ----------------------------------------------------------------------

fn json_time(v: &Value) -> Option<DateTime<Utc>> {
    match v {
        Value::Number(n) => n.as_i64().and_then(|t| DateTime::from_timestamp(t, 0)),
        Value::String(s) => DateTime::parse_from_rfc3339(s)
            .map(|d| d.with_timezone(&Utc))
            .ok()
            .or_else(|| parse_kindle_date(s))
            .or_else(|| {
                s.parse::<i64>()
                    .ok()
                    .and_then(|t| DateTime::from_timestamp(t, 0))
            }),
        _ => None,
    }
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(text_of).collect::<Vec<_>>().join("; "),
        _ => String::new(),
    }
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .map(text_of)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn page_label(v: &Value) -> Option<String> {
    match v {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    }
}

/// KOReader exports one document as `{title, author, entries: [{text, note, page, time, chapter}]}`
/// and several as `{documents: [..]}`.
fn koreader_doc(doc: &Value, out: &mut Vec<Imported>) {
    let Some(title) = str_field(doc, "title") else {
        return;
    };
    let author = str_field(doc, "author").map(|a| a.replace('\n', "; "));
    for e in doc
        .get("entries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = str_field(e, "text").unwrap_or_default();
        let note = str_field(e, "note").unwrap_or_default();
        if text.is_empty() && note.is_empty() {
            continue;
        }
        out.push(Imported {
            book_title: title.clone(),
            book_author: author.clone(),
            text,
            note,
            location: e.get("page").and_then(page_label),
            chapter: str_field(e, "chapter"),
            created: e.get("time").and_then(json_time),
            bookmark: false,
            color: str_field(e, "color"),
            style: str_field(e, "drawer").or_else(|| str_field(e, "style")),
        });
    }
}

/// Bookx's own JSON from *Export Annotations* in the book menu.
fn bookx_core_doc(doc: &Value, out: &mut Vec<Imported>) {
    let Some(title) = str_field(doc, "title") else {
        return;
    };
    let author = str_field(doc, "authors");
    for h in doc
        .get("highlights")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = str_field(h, "text").unwrap_or_default();
        if text.is_empty() {
            continue;
        }
        out.push(Imported {
            book_title: title.clone(),
            book_author: author.clone(),
            text,
            note: str_field(h, "note").unwrap_or_default(),
            location: None,
            chapter: str_field(h, "chapter"),
            created: h.get("created").and_then(json_time),
            bookmark: false,
            color: str_field(h, "color"),
            style: str_field(h, "style"),
        });
    }
}

/// The JSON written by the export of `bookx-lookup` (`{version, books: [{title, author, annotations}]}`).
fn bookx_lookup_doc(doc: &Value, out: &mut Vec<Imported>) {
    for b in doc
        .get("books")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(title) = str_field(b, "title") else {
            continue;
        };
        let author = str_field(b, "author");
        for a in b
            .get("annotations")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let bookmark = a.get("style").and_then(Value::as_str) == Some("bookmark");
            let text = str_field(a, "text").unwrap_or_default();
            let note = str_field(a, "note").unwrap_or_default();
            if text.is_empty() && note.is_empty() && !bookmark {
                continue;
            }
            out.push(Imported {
                book_title: title.clone(),
                book_author: author.clone(),
                text,
                note,
                location: a.get("location").and_then(page_label),
                chapter: str_field(a, "chapter"),
                created: a.get("created").and_then(json_time),
                bookmark,
                color: str_field(a, "color"),
                style: str_field(a, "style").filter(|_| !bookmark),
            });
        }
    }
}

/// Walks a calibre annotation collection (`calibredb annotations export --format json`, or the
/// *Export* button of the Browse annotations tool): every object with `"type": "highlight"` and a
/// `highlighted_text` is a highlight, `notes` its note, `timestamp` its time, the last of
/// `toc_family_titles` its chapter; the title and authors come from the nearest enclosing object
/// that has a `title`. The exact layout of the collection is calibre's and was not checked against
/// a file written by calibre itself, hence the lenient walk.
fn calibre_walk(v: &Value, book: &(Option<String>, Option<String>), out: &mut Vec<Imported>) {
    match v {
        Value::Object(m) => {
            let mut book = book.clone();
            if let Some(t) = str_field(v, "title") {
                book = (
                    Some(t),
                    str_field(v, "authors").or_else(|| str_field(v, "author")),
                );
            }
            if m.get("type").and_then(Value::as_str) == Some("highlight") {
                if let (Some(title), Some(text)) = (&book.0, str_field(v, "highlighted_text")) {
                    out.push(Imported {
                        book_title: title.clone(),
                        book_author: book.1.clone(),
                        text,
                        note: str_field(v, "notes").unwrap_or_default(),
                        location: None,
                        chapter: v
                            .get("toc_family_titles")
                            .and_then(Value::as_array)
                            .and_then(|a| a.last())
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        created: v.get("timestamp").and_then(json_time),
                        bookmark: false,
                        color: None,
                        style: None,
                    });
                }
                return;
            }
            for (_, child) in m {
                calibre_walk(child, &book, out);
            }
        }
        Value::Array(a) => {
            for child in a {
                calibre_walk(child, book, out);
            }
        }
        _ => {}
    }
}

/// Reads a JSON annotation file and works out whose it is: KOReader's export, Bookx's own export
/// (either of the two), or a calibre annotation collection. An error says the shape was not
/// recognised.
pub fn parse_json(text: &str) -> Result<Vec<Imported>> {
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))?;
    let mut out = Vec::new();
    if v.get("entries").is_some() {
        koreader_doc(&v, &mut out);
    } else if let Some(docs) = v.get("documents").and_then(Value::as_array) {
        for d in docs {
            koreader_doc(d, &mut out);
        }
    } else if v.get("highlights").is_some() && v.get("title").is_some() {
        bookx_core_doc(&v, &mut out);
    } else {
        if v.get("books").is_some() {
            bookx_lookup_doc(&v, &mut out);
        }
        if out.is_empty() {
            calibre_walk(&v, &(None, None), &mut out);
        }
        if out.is_empty() {
            bail!("this JSON is not a KOReader, Bookx or calibre annotation export");
        }
    }
    Ok(out)
}

fn unwrap_format(s: &str) -> String {
    let s = s.trim();
    for (a, b) in [
        ("**_", "_**"),
        ("**", "**"),
        ("==", "=="),
        ("~~", "~~"),
        ("++", "++"),
        ("<u>", "</u>"),
    ] {
        if let Some(inner) = s.strip_prefix(a).and_then(|r| r.strip_suffix(b)) {
            return inner.to_string();
        }
    }
    s.to_string()
}

/// KOReader's Markdown export:
///
/// ```text
/// # Title
/// ##### Author
/// ## Chapter
/// ### Page 12 @ 03 March 2024 10:42:11 AM
/// the highlighted text
///
/// ---
/// the note
/// ```
///
/// The text may be wrapped in the formatting the user chose (`**`, `==`, …); that is removed.
pub fn parse_koreader_markdown(text: &str) -> Vec<Imported> {
    let mut out: Vec<Imported> = Vec::new();
    let (mut title, mut author, mut chapter) = (String::new(), None::<String>, None::<String>);
    let mut cur: Pending = None;
    let flush = |cur: &mut Pending,
                 out: &mut Vec<Imported>,
                 title: &str,
                 author: &Option<String>,
                 chapter: &Option<String>| {
        let Some((page, created, lines)) = cur.take() else {
            return;
        };
        let body = lines.join("\n");
        let (text, note) = match body.split_once("\n---\n") {
            Some((t, n)) => (t.to_string(), n.trim().to_string()),
            None => (body, String::new()),
        };
        let text = unwrap_format(&text);
        if !title.is_empty() && (!text.is_empty() || !note.is_empty()) {
            out.push(Imported {
                book_title: title.to_string(),
                book_author: author.clone(),
                text,
                note,
                location: page,
                chapter: chapter.clone(),
                created,
                bookmark: false,
                color: None,
                style: None,
            });
        }
    };
    for line in text.trim_start_matches('\u{feff}').lines() {
        if let Some(t) = line.strip_prefix("# ") {
            flush(&mut cur, &mut out, &title, &author, &chapter);
            title = t.trim().to_string();
            author = None;
            chapter = None;
        } else if let Some(a) = line.strip_prefix("##### ") {
            author = Some(a.trim().to_string()).filter(|a| !a.is_empty() && a != "N/A");
        } else if let Some(c) = line.strip_prefix("### ") {
            flush(&mut cur, &mut out, &title, &author, &chapter);
            let (page, date) = match c.trim().strip_prefix("Page ") {
                Some(rest) => match rest.split_once(" @ ") {
                    Some((p, d)) => (Some(p.trim().to_string()), Some(d)),
                    None => (Some(rest.trim().to_string()), None),
                },
                None => (None, None),
            };
            let created = date.and_then(|d| {
                NaiveDateTime::parse_from_str(d.trim(), "%d %B %Y %I:%M:%S %p")
                    .ok()
                    .map(|t| Utc.from_utc_datetime(&t))
            });
            cur = Some((page, created, Vec::new()));
        } else if let Some(c) = line.strip_prefix("## ") {
            flush(&mut cur, &mut out, &title, &author, &chapter);
            chapter = Some(c.trim().to_string()).filter(|c| !c.is_empty());
        } else if let Some((_, _, lines)) = cur.as_mut() {
            lines.push(line.to_string());
        }
    }
    flush(&mut cur, &mut out, &title, &author, &chapter);
    for i in &mut out {
        i.text = i.text.trim().to_string();
    }
    out
}

/// A colour or style word with the aliases of the devices folded together (the same table as
/// `bookx_core::devstyle`, which this crate does not depend on).
fn look_word(w: &str) -> String {
    let w = w.trim().to_ascii_lowercase();
    match w.as_str() {
        "lighten" | "highlight" | "highlighted" => "background".into(),
        "underscore" | "underlined" => "underline".into(),
        "strikeout" | "strike" | "strikeout-through" => "strikethrough".into(),
        "inverted" => "invert".into(),
        "grey" => "gray".into(),
        _ => w,
    }
}

/// Removes repeats from a list of imported notes: the same quote (ignoring case, spacing and
/// punctuation) of the same book is one entry; when one quote contains the other (12 characters or
/// more), the longer wins, as when a Kindle highlight is extended and the old one stays in the file.
/// A note is kept from whichever copy has one. Order of first appearance is kept.
pub fn dedupe(items: Vec<Imported>) -> Vec<Imported> {
    let mut out: Vec<Imported> = Vec::new();
    for it in items {
        let (key, text) = (norm(&it.book_title), norm(&it.text));
        let same = |o: &Imported| {
            if norm(&o.book_title) != key {
                return false;
            }
            // the same words in another colour or style are another highlight
            let differ = |a: &Option<String>, b: &Option<String>| matches!((a, b), (Some(x), Some(y)) if !x.eq_ignore_ascii_case(y));
            // the words of two sources for one look are the same look: KOReader's `lighten` and
            // Bookx's `background`, `underscore` and `underline`, `grey` and `gray`
            let differ_look = |a: &Option<String>, b: &Option<String>| matches!((a, b), (Some(x), Some(y)) if look_word(x) != look_word(y));
            if differ_look(&o.color, &it.color) || differ_look(&o.style, &it.style) {
                return false;
            }
            // the same words in two different chapters are two highlights (a word or term that is
            // marked every time it is explained); a source that does not name the chapter says nothing
            if differ(&o.chapter, &it.chapter) {
                return false;
            }
            let ot = norm(&o.text);
            if text.is_empty() || ot.is_empty() {
                return text == ot && !it.note.is_empty() && norm(&o.note) == norm(&it.note);
            }
            let (short, long) = if text.len() <= ot.len() {
                (&text, &ot)
            } else {
                (&ot, &text)
            };
            short == long || (short.chars().count() >= 12 && long.contains(short.as_str()))
        };
        match out.iter().position(same) {
            None => out.push(it),
            Some(i) => {
                let o = &mut out[i];
                if norm(&it.text).len() > norm(&o.text).len() {
                    o.text = it.text.clone();
                    o.location = it.location.clone().or(o.location.take());
                }
                if o.note.trim().is_empty() {
                    o.note = it.note;
                }
                if o.created.is_none() {
                    o.created = it.created;
                }
                if o.color.is_none() {
                    o.color = it.color;
                }
                if o.style.is_none() {
                    o.style = it.style;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::{export, AnnotationItem, ExportFormat, ExportOptions, HighlightStyle};
    use chrono::TimeZone;

    const CLIPPINGS: &str = "\u{feff}Dune (Frank Herbert)\r\n- Your Highlight on page 12 | Location 183-184 | Added on Monday, 3 March 2014 10:42:11\r\n\r\nFear is the mind-killer.\r\n==========\r\nDune (Frank Herbert)\r\n- Your Note on page 12 | Location 184 | Added on Monday, 3 March 2014 10:43:00\r\n\r\nthe litany\r\n==========\r\nThe Road (McCarthy, Cormac)\r\n- Your Bookmark on Location 55 | Added on Tuesday, 4 March 2014 08:00:00\r\n\r\n\r\n==========\r\nThe Road (McCarthy, Cormac)\r\n- Your Highlight on Location 300-301 | Added on Tuesday, 4 March 2014 09:00:00\r\n\r\nCarrying the fire.\r\n==========\r\nDune (Frank Herbert)\r\n- Your Note on Location 900 | Added on Tuesday, 4 March 2014 09:00:00\r\n\r\nalone note\r\n==========\r\n";

    #[test]
    fn kindle_clippings_are_parsed_and_notes_join_their_highlight() {
        let v = parse_kindle_clippings(CLIPPINGS);
        assert_eq!(v.len(), 4, "{v:#?}");
        assert_eq!(v[0].book_title, "Dune");
        assert_eq!(v[0].book_author.as_deref(), Some("Frank Herbert"));
        assert_eq!(v[0].text, "Fear is the mind-killer.");
        assert_eq!(v[0].note, "the litany");
        assert_eq!(v[0].location.as_deref(), Some("183-184"));
        assert_eq!(
            v[0].created,
            Some(Utc.with_ymd_and_hms(2014, 3, 3, 10, 42, 11).unwrap())
        );
        assert!(v[1].bookmark && v[1].text.is_empty());
        assert_eq!(v[1].book_author.as_deref(), Some("Cormac McCarthy"));
        assert_eq!(v[2].text, "Carrying the fire.");
        // a note whose highlight is not in the file stays a note without a quote
        assert_eq!((v[3].text.as_str(), v[3].note.as_str()), ("", "alone note"));
    }

    #[test]
    fn titles_with_parentheses_keep_them() {
        let (t, a) = title_and_author("Dune (Deluxe Edition) (Frank Herbert)");
        assert_eq!(
            (t.as_str(), a.as_deref()),
            ("Dune (Deluxe Edition)", Some("Frank Herbert"))
        );
        let (t, a) = title_and_author("No Author Here");
        assert_eq!((t.as_str(), a), ("No Author Here", None));
    }

    /// Root cause R7: the same highlight from two sources (KOReader's `lighten`, Bookx's
    /// `background`) was told apart by the spelling of its style and imported twice.
    #[test]
    fn the_same_look_in_two_spellings_is_one_highlight() {
        let mk = |color: &str, style: &str| Imported {
            book_title: "B".into(),
            book_author: None,
            text: "Some highlighted words that are long enough".into(),
            note: String::new(),
            location: None,
            chapter: None,
            created: None,
            bookmark: false,
            color: Some(color.into()),
            style: Some(style.into()),
        };
        let out = dedupe(vec![
            mk("grey", "lighten"),
            mk("gray", "background"),
            mk("gray", "underscore"),
        ]);
        assert_eq!(
            out.len(),
            2,
            "lighten == background, underscore is another look"
        );
    }

    #[test]
    fn dates_in_both_kindle_styles_are_read() {
        let want = Some(Utc.with_ymd_and_hms(2024, 1, 2, 15, 4, 5).unwrap());
        assert_eq!(
            parse_kindle_date("Tuesday, January 2, 2024 3:04:05 PM"),
            want
        );
        assert_eq!(parse_kindle_date("Tuesday, 2 January 2024 15:04:05"), want);
        assert_eq!(parse_kindle_date("Dienstag, 2. Januar 2024"), None);
    }

    #[test]
    fn our_own_kindle_export_reads_back() {
        let item = |text: &str, note: Option<&str>, loc: &str| AnnotationItem {
            book_title: "Dune".into(),
            book_author: Some("Frank Herbert".into()),
            chapter: None,
            text: text.into(),
            note: note.map(str::to_string),
            style: HighlightStyle::Highlight,
            color: None,
            created: Utc.with_ymd_and_hms(2024, 1, 2, 15, 4, 5).unwrap(),
            location: Some(loc.into()),
        };
        let items = vec![
            item("Fear is the mind-killer.", Some("litany"), "12"),
            item("I will face my fear.", None, "20"),
        ];
        let txt = export(
            &items,
            ExportFormat::KindleClippings,
            &ExportOptions::default(),
            Utc::now(),
        );
        let v = parse_kindle_clippings(&txt);
        assert_eq!(v.len(), 2, "{txt}");
        assert_eq!(v[0].text, "Fear is the mind-killer.");
        assert_eq!(v[0].note, "litany");
        assert_eq!(v[1].location.as_deref(), Some("20"));
        assert_eq!(v[1].created, Some(items[1].created));
    }

    #[test]
    fn koreader_json_single_and_many_documents() {
        let one = r#"{"title":"Dune","author":"Frank Herbert\nSecond Author","created_on":1,"version":"1.0",
          "entries":[{"sort":"highlight","page":12,"time":1700000000,"text":"Fear is the mind-killer.","note":"mine","chapter":"One","drawer":"lighten"},
                     {"page":13,"time":1700000100,"text":"","note":""}]}"#;
        let v = parse_json(one).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(
            v[0].book_author.as_deref(),
            Some("Frank Herbert; Second Author")
        );
        assert_eq!(v[0].location.as_deref(), Some("12"));
        assert_eq!(v[0].note, "mine");
        assert_eq!(v[0].created, DateTime::from_timestamp(1_700_000_000, 0));
        let many = format!(r#"{{"documents":[{one},{one}],"version":"1.0"}}"#);
        assert_eq!(parse_json(&many).unwrap().len(), 2);
    }

    #[test]
    fn bookx_exports_read_back() {
        let core = r#"{"title":"T","authors":["A","B"],"highlights":[{"chapter":"c","text":"quote one","note":"","color":"yellow","created":"2026-01-02T03:04:05Z"}],"bookmarks":[]}"#;
        let v = parse_json(core).unwrap();
        assert_eq!(
            (v[0].book_title.as_str(), v[0].text.as_str()),
            ("T", "quote one")
        );
        let items = vec![AnnotationItem {
            book_title: "T".into(),
            book_author: Some("A".into()),
            chapter: Some("c".into()),
            text: "quote two".into(),
            note: Some("n".into()),
            style: HighlightStyle::Highlight,
            color: None,
            created: Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap(),
            location: None,
        }];
        let j = export(
            &items,
            ExportFormat::Json,
            &ExportOptions::default(),
            Utc::now(),
        );
        let v = parse_json(&j).unwrap();
        assert_eq!((v[0].text.as_str(), v[0].note.as_str()), ("quote two", "n"));
        assert_eq!(v[0].created, Some(items[0].created));
    }

    #[test]
    fn calibre_collections_are_read_leniently() {
        let j = r#"{"version":1,"books":[{"title":"Dune","authors":["Frank Herbert"],"annotations":{"EPUB":{"viewer":[
          {"type":"highlight","uuid":"u","timestamp":"2023-05-06T07:08:09.123Z","highlighted_text":"Fear is the mind-killer.","notes":"n","toc_family_titles":["Book","Chapter 1"],"spine_name":"c1.xhtml"},
          {"type":"bookmark","title":"Bookmark 1","pos":"x"}]}}}]}"#;
        let v = parse_json(j).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].book_title, "Dune");
        assert_eq!(v[0].chapter.as_deref(), Some("Chapter 1"));
        assert_eq!(v[0].note, "n");
        assert!(parse_json(r#"{"x":1}"#).is_err());
        assert!(parse_json("not json").is_err());
    }

    #[test]
    fn koreader_markdown_is_read() {
        let md = "# Dune\n##### Frank Herbert\n\n## Chapter One\n### Page 12 @ 03 March 2024 10:42:11 AM\n**Fear is the mind-killer.**\n\n---\nthe litany\n### Page 14 @ 03 March 2024 10:50:00 AM\nSecond quote\n# Other\n##### N/A\n### Page 1 @ 04 March 2024 01:00:00 PM\n==Third==\n";
        let v = parse_koreader_markdown(md);
        assert_eq!(v.len(), 3, "{v:#?}");
        assert_eq!(v[0].text, "Fear is the mind-killer.");
        assert_eq!(v[0].note, "the litany");
        assert_eq!(v[0].chapter.as_deref(), Some("Chapter One"));
        assert_eq!(v[0].book_author.as_deref(), Some("Frank Herbert"));
        assert_eq!(
            v[0].created,
            Some(Utc.with_ymd_and_hms(2024, 3, 3, 10, 42, 11).unwrap())
        );
        assert_eq!(v[1].text, "Second quote");
        assert_eq!(
            (v[2].book_title.as_str(), v[2].book_author.clone()),
            ("Other", None)
        );
        assert_eq!(v[2].text, "Third");
    }

    #[test]
    fn dedupe_keeps_the_longest_and_the_note() {
        let mk = |text: &str, note: &str| Imported {
            book_title: "Dune".into(),
            book_author: None,
            text: text.into(),
            note: note.into(),
            location: None,
            chapter: None,
            created: None,
            bookmark: false,
            color: None,
            style: None,
        };
        let v = dedupe(vec![
            mk("Fear is the mind-killer", ""),
            mk("Fear is the mind-killer. Fear is the little-death.", "n"),
            mk("fear is THE mind killer", ""),
            mk("Something else entirely", ""),
        ]);
        assert_eq!(v.len(), 2);
        assert_eq!(
            v[0].text,
            "Fear is the mind-killer. Fear is the little-death."
        );
        assert_eq!(v[0].note, "n");
    }

    /// Found on the owner's Kobo: the term "Request-response" is highlighted in two chapters (pages 150 and
    /// 157); the second was thrown away as a repeat.
    #[test]
    fn the_same_words_in_two_chapters_are_two_highlights() {
        let mk = |chapter: Option<&str>| Imported {
            book_title: "Building Microservices".into(),
            book_author: None,
            text: "Request-response".into(),
            note: String::new(),
            location: None,
            chapter: chapter.map(str::to_string),
            created: None,
            bookmark: false,
            color: Some("cyan".into()),
            style: None,
        };
        let v = dedupe(vec![
            mk(Some("Styles of Microservice Communication")),
            mk(Some("Pattern: Asynchronous Nonblocking")),
            mk(Some("Pattern: Asynchronous Nonblocking")),
            mk(None),
        ]);
        assert_eq!(
            v.len(),
            2,
            "two chapters; a copy and one that names no chapter fold in"
        );
    }

    #[test]
    fn authors_are_split_and_reordered() {
        assert_eq!(split_authors("McCarthy, Cormac"), ["Cormac McCarthy"]);
        assert_eq!(split_authors("A B; C D"), ["A B", "C D"]);
        assert_eq!(split_authors("Frank Herbert"), ["Frank Herbert"]);
    }
}
