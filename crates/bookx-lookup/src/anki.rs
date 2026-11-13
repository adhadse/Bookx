//! Anki cards from highlights and from the vocabulary list.
//!
//! * [`notes_from_items`] / [`notes_from_vocab`] make format-neutral [`Note`]s: *basic* (front /
//!   back) or *cloze* (a sentence with a word hidden), each with a stable guid so importing the same
//!   export again updates the cards in Anki instead of adding copies, the source book as a tag and
//!   the context sentence on the back.
//! * [`to_delimited`] writes the text file Anki's importer reads (tab or comma separated, with the
//!   `#notetype`, `#deck`, `#guid column` and `#tags column` header lines of Anki 2.1.54+).
//! * [`to_apkg`] writes a package Anki opens with *File → Import*: a zip with a SQLite collection
//!   in the legacy schema 11 (`collection.anki2`) and an empty `media` map. The schema follows
//!   Anki's documented database layout; the tests re-open the package and check the tables and the
//!   model and deck JSON, but no real Anki was available to import it, see `docs/annotation-sync-notes.md`.

use std::io::Write as _;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use regex::RegexBuilder;
use serde_json::{json, Value};

use crate::export::{AnnotationItem, HighlightStyle};
use crate::ids;
use crate::vocab::VocabStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardKind {
    /// Front and back.
    Basic,
    /// A sentence with one word hidden (`{{c1::word}}`).
    Cloze,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub deck: String,
    pub kind: CardKind,
    /// Tags on every note, besides `bookx` and the book.
    pub tags: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            deck: "Bookx".into(),
            kind: CardKind::Basic,
            tags: Vec::new(),
        }
    }
}

/// One note: two fields (front/back or text/extra), tags and a guid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub guid: String,
    pub kind: CardKind,
    pub first: String,
    pub second: String,
    pub tags: Vec<String>,
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn html_raw(s: &str) -> String {
    esc(s).replace("\r\n", "<br>").replace('\n', "<br>")
}

fn html(s: &str) -> String {
    html_raw(s.trim())
}

fn tag_safe(s: &str) -> String {
    let t: String = s
        .chars()
        .filter(|c| !matches!(c, '"' | '\'' | ',' | ';' | '<' | '>' | '&'))
        .map(|c| if c.is_whitespace() { '_' } else { c })
        .collect();
    t.split('_')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn tags_for(opts: &Options, book: Option<&str>) -> Vec<String> {
    let mut v = vec!["bookx".to_string()];
    if let Some(b) = book {
        v.push(tag_safe(b));
    }
    v.extend(opts.tags.iter().map(|t| tag_safe(t)));
    v.retain(|t| !t.is_empty());
    v.dedup();
    v
}

/// `raw` with the first case-insensitive occurrence of `target` hidden as a cloze, the rest
/// escaped. Falls back to the whole text hidden if `target` is not in it.
pub fn cloze_text(raw: &str, target: &str) -> String {
    let t = target.trim();
    if !t.is_empty() {
        if let Ok(re) = RegexBuilder::new(&regex::escape(t))
            .case_insensitive(true)
            .build()
        {
            if let Some(m) = re.find(raw) {
                return format!(
                    "{}{{{{c1::{}}}}}{}",
                    html_raw(&raw[..m.start()]),
                    esc(m.as_str()),
                    html_raw(&raw[m.end()..])
                );
            }
        }
    }
    format!("{{{{c1::{}}}}}", html(raw))
}

/// The word to hide in a highlight: the longest word of at least five letters (the first of them
/// on a tie).
pub fn pick_cloze_word(text: &str) -> Option<String> {
    let mut best: Option<&str> = None;
    for w in text.split(|c: char| !c.is_alphanumeric() && c != '\'' && c != '-') {
        let w = w.trim_matches(|c| c == '\'' || c == '-');
        if w.chars().count() >= 5 && best.is_none_or(|b| w.chars().count() > b.chars().count()) {
            best = Some(w);
        }
    }
    best.map(str::to_string)
}

/// Notes for highlights (bookmarks and empty quotes are skipped, a quote twice makes one note).
/// The back of a basic card holds your note and the source; a cloze card hides one word of the
/// quote and shows the same on the extra field.
pub fn notes_from_items(items: &[AnnotationItem], opts: &Options) -> Vec<Note> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for i in items {
        if i.style == HighlightStyle::Bookmark || i.text.trim().is_empty() {
            continue;
        }
        let guid = ids::item_id(i);
        if !seen.insert(guid.clone()) {
            continue;
        }
        let mut source = html(&i.book_title);
        if let Some(a) = i.book_author.as_deref().filter(|a| !a.trim().is_empty()) {
            source.push_str(&format!(", {}", html(a)));
        }
        if let Some(c) = i.chapter.as_deref().filter(|c| !c.trim().is_empty()) {
            source.push_str(&format!(" ({})", html(c)));
        }
        let note = i.note.as_deref().map(str::trim).filter(|n| !n.is_empty());
        let mut back = String::new();
        if let Some(n) = note {
            back.push_str(&html(n));
            back.push_str("<br><br>");
        }
        back.push_str(&format!("<i>{source}</i>"));
        let first = match opts.kind {
            CardKind::Basic => html(&i.text),
            CardKind::Cloze => {
                let word = pick_cloze_word(&i.text).unwrap_or_default();
                cloze_text(i.text.trim(), &word)
            }
        };
        out.push(Note {
            guid,
            kind: opts.kind,
            first,
            second: back,
            tags: tags_for(opts, Some(&i.book_title)),
        });
    }
    out
}

/// Notes for the vocabulary list. `book_title` maps a context's book id to a title for the tag.
/// Basic: the word on the front, definition and the context sentence (word in bold) on the back.
/// Cloze: the context sentence with the word hidden and the definition as the extra.
pub fn notes_from_vocab(
    store: &VocabStore,
    opts: &Options,
    book_title: &dyn Fn(&str) -> Option<String>,
) -> Vec<Note> {
    store
        .entries()
        .iter()
        .filter(|e| !e.word.trim().is_empty())
        .map(|e| {
            let ctx = e.contexts.first();
            let def = e.definition.as_deref().map(html).unwrap_or_default();
            let book = ctx.and_then(|c| c.book_id.as_deref()).and_then(book_title);
            let (first, second) = match (opts.kind, ctx) {
                (CardKind::Cloze, Some(c)) => (cloze_text(&c.sentence, &e.word), def),
                (CardKind::Cloze, None) => (format!("{{{{c1::{}}}}}", html(&e.word)), def),
                (CardKind::Basic, c) => {
                    let mut back = def;
                    if let Some(c) = c {
                        if !back.is_empty() {
                            back.push_str("<br>");
                        }
                        let bold = cloze_text(&c.sentence, &e.word)
                            .replacen("{{c1::", "<b>", 1)
                            .replacen("}}", "</b>", 1);
                        back.push_str(&format!("<i>{bold}</i>"));
                    }
                    (html(&e.word), back)
                }
            };
            Note {
                guid: ids::content_id("vocabulary", None, &e.word),
                kind: opts.kind,
                first,
                second,
                tags: tags_for(opts, book.as_deref()),
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Separator {
    Tab,
    Comma,
}

fn field(s: &str, sep: Separator) -> String {
    let one = s.replace("\r\n", "<br>").replace(['\n', '\r'], "<br>");
    match sep {
        Separator::Tab => one.replace('\t', " "),
        Separator::Comma => {
            if one.contains([',', '"']) {
                format!("\"{}\"", one.replace('"', "\"\""))
            } else {
                one
            }
        }
    }
}

/// The text file for Anki's *Import File*: `guid, field 1, field 2, tags` with header lines that
/// tell Anki the separator, that fields are HTML, the note type, the deck and where guid and tags are.
pub fn to_delimited(notes: &[Note], opts: &Options, sep: Separator) -> String {
    let kind = notes.first().map_or(opts.kind, |n| n.kind);
    let mut out = String::new();
    out.push_str(match sep {
        Separator::Tab => "#separator:tab\n",
        Separator::Comma => "#separator:comma\n",
    });
    out.push_str("#html:true\n");
    out.push_str(match kind {
        CardKind::Basic => "#notetype:Basic\n",
        CardKind::Cloze => "#notetype:Cloze\n",
    });
    out.push_str(&format!("#deck:{}\n", opts.deck.replace('\n', " ")));
    out.push_str("#guid column:1\n#tags column:4\n");
    let d = match sep {
        Separator::Tab => "\t",
        Separator::Comma => ",",
    };
    for n in notes {
        out.push_str(
            &[
                field(&n.guid, sep),
                field(&n.first, sep),
                field(&n.second, sep),
                field(&n.tags.join(" "), sep),
            ]
            .join(d),
        );
        out.push('\n');
    }
    out
}

// ---- .apkg ---------------------------------------------------------------------------------

fn sha1_first_word(data: &[u8]) -> u32 {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for (i, b) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6u32),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    h[0]
}

/// Anki's note checksum: the first 8 hex digits of the SHA-1 of the first field without HTML.
pub fn field_checksum(first_field: &str) -> u32 {
    let mut plain = String::new();
    let mut in_tag = false;
    for c in first_field.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => plain.push(c),
            _ => {}
        }
    }
    sha1_first_word(plain.trim().as_bytes())
}

fn model_id(kind: CardKind, base: i64) -> i64 {
    base + match kind {
        CardKind::Basic => 1,
        CardKind::Cloze => 2,
    }
}

fn model_json(kind: CardKind, mid: i64, did: i64, now_s: i64) -> Value {
    let fields = |a: &str, b: &str| {
        Value::Array(
            [a, b]
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    json!({"name": n, "ord": i, "sticky": false, "rtl": false,
                        "font": "Arial", "size": 20, "media": []})
                })
                .collect(),
        )
    };
    let (name, ty, flds, tmpls, req, css_extra) = match kind {
        CardKind::Basic => (
            "Bookx Basic",
            0,
            fields("Front", "Back"),
            json!([{"name": "Card 1", "ord": 0,
                "qfmt": "{{Front}}", "afmt": "{{FrontSide}}\n\n<hr id=answer>\n\n{{Back}}",
                "bqfmt": "", "bafmt": "", "did": null, "bfont": "", "bsize": 0}]),
            json!([[0, "any", [0]]]),
            "",
        ),
        CardKind::Cloze => (
            "Bookx Cloze",
            1,
            fields("Text", "Extra"),
            json!([{"name": "Cloze", "ord": 0,
                "qfmt": "{{cloze:Text}}", "afmt": "{{cloze:Text}}<br>\n{{Extra}}",
                "bqfmt": "", "bafmt": "", "did": null, "bfont": "", "bsize": 0}]),
            json!([[0, "all", [0]]]),
            ".cloze { font-weight: bold; color: blue; }\n",
        ),
    };
    json!({
        "id": mid, "name": name, "type": ty, "mod": now_s, "usn": -1, "sortf": 0, "did": did,
        "tmpls": tmpls, "flds": flds,
        "css": format!(".card {{ font-family: arial; font-size: 20px; text-align: left; color: black; background-color: white; }}\n{css_extra}"),
        "latexPre": "\\documentclass[12pt]{article}\n\\special{papersize=3in,5in}\n\\usepackage[utf8]{inputenc}\n\\usepackage{amssymb,amsmath}\n\\pagestyle{empty}\n\\setlength{\\parindent}{0in}\n\\begin{document}\n",
        "latexPost": "\\end{document}",
        "latexsvg": false, "req": req, "tags": [], "vers": []
    })
}

fn deck_json(id: i64, name: &str, now_s: i64) -> Value {
    json!({
        "id": id, "name": name, "mod": now_s, "usn": -1,
        "lrnToday": [0, 0], "revToday": [0, 0], "newToday": [0, 0], "timeToday": [0, 0],
        "collapsed": false, "browserCollapsed": false, "desc": "", "dyn": 0, "conf": 1,
        "extendNew": 0, "extendRev": 0
    })
}

const SCHEMA: &str = "
CREATE TABLE col (id integer primary key, crt integer not null, mod integer not null, scm integer not null,
  ver integer not null, dty integer not null, usn integer not null, ls integer not null, conf text not null,
  models text not null, decks text not null, dconf text not null, tags text not null);
CREATE TABLE notes (id integer primary key, guid text not null, mid integer not null, mod integer not null,
  usn integer not null, tags text not null, flds text not null, sfld integer not null, csum integer not null,
  flags integer not null, data text not null);
CREATE TABLE cards (id integer primary key, nid integer not null, did integer not null, ord integer not null,
  mod integer not null, usn integer not null, type integer not null, queue integer not null, due integer not null,
  ivl integer not null, factor integer not null, reps integer not null, lapses integer not null, left integer not null,
  odue integer not null, odid integer not null, flags integer not null, data text not null);
CREATE TABLE revlog (id integer primary key, cid integer not null, usn integer not null, ease integer not null,
  ivl integer not null, lastIvl integer not null, factor integer not null, time integer not null, type integer not null);
CREATE TABLE graves (usn integer not null, oid integer not null, type integer not null);
CREATE INDEX ix_notes_usn on notes (usn);
CREATE INDEX ix_cards_usn on cards (usn);
CREATE INDEX ix_revlog_usn on revlog (usn);
CREATE INDEX ix_cards_nid on cards (nid);
CREATE INDEX ix_cards_sched on cards (did, queue, due);
CREATE INDEX ix_revlog_cid on revlog (cid);
CREATE INDEX ix_notes_csum on notes (csum);
";

/// A package (`.apkg`) with `notes` in the deck `deck`. Notes of both kinds can be mixed. The
/// ids are derived from `now`.
pub fn to_apkg(notes: &[Note], deck: &str, now: DateTime<Utc>) -> Result<Vec<u8>> {
    let now_ms = now.timestamp_millis();
    let now_s = now.timestamp();
    let did = now_ms;
    let base = now_ms + 10;
    let mut conn = rusqlite::Connection::open_in_memory()?;
    conn.execute_batch(SCHEMA)?;

    let mut models = serde_json::Map::new();
    for k in [CardKind::Basic, CardKind::Cloze] {
        let mid = model_id(k, base);
        models.insert(mid.to_string(), model_json(k, mid, did, now_s));
    }
    let mut decks = serde_json::Map::new();
    decks.insert("1".into(), deck_json(1, "Default", now_s));
    decks.insert(did.to_string(), deck_json(did, deck, now_s));
    let dconf = json!({"1": {
        "id": 1, "name": "Default", "replayq": true, "timer": 0, "maxTaken": 60, "usn": 0, "mod": 0, "autoplay": true,
        "lapse": {"delays": [10], "mult": 0, "minInt": 1, "leechFails": 8, "leechAction": 0},
        "rev": {"perDay": 200, "ease4": 1.3, "ivlFct": 1, "maxIvl": 36500, "bury": true, "fuzz": 0.05},
        "new": {"delays": [1, 10], "ints": [1, 4, 7], "initialFactor": 2500, "separate": true, "order": 1, "perDay": 20, "bury": true}
    }});
    let conf = json!({
        "nextPos": notes.len() + 1, "estTimes": true, "activeDecks": [did], "sortType": "noteFld",
        "timeLim": 0, "sortBackwards": false, "addToCur": true, "curDeck": did,
        "newBury": true, "newSpread": 0, "dueCounts": true,
        "curModel": model_id(CardKind::Basic, base).to_string(), "collapseTime": 1200
    });
    conn.execute(
        "INSERT INTO col VALUES (1, ?1, ?2, ?3, 11, 0, 0, 0, ?4, ?5, ?6, ?7, '{}')",
        rusqlite::params![
            now_s,
            now_ms,
            now_ms,
            conf.to_string(),
            Value::Object(models).to_string(),
            Value::Object(decks).to_string(),
            dconf.to_string(),
        ],
    )?;

    {
        let tx = conn.transaction()?;
        for (n, note) in notes.iter().enumerate() {
            let nid = now_ms + 1000 + n as i64;
            let flds = format!("{}\x1f{}", note.first, note.second);
            let tags = if note.tags.is_empty() {
                String::new()
            } else {
                format!(" {} ", note.tags.join(" "))
            };
            tx.execute(
                "INSERT INTO notes VALUES (?1, ?2, ?3, ?4, -1, ?5, ?6, ?7, ?8, 0, '')",
                rusqlite::params![
                    nid,
                    note.guid,
                    model_id(note.kind, base),
                    now_s,
                    tags,
                    flds,
                    note.first,
                    field_checksum(&note.first),
                ],
            )?;
            tx.execute(
                "INSERT INTO cards VALUES (?1, ?2, ?3, 0, ?4, -1, 0, 0, ?5, 0, 0, 0, 0, 0, 0, 0, 0, '')",
                rusqlite::params![nid, nid, did, now_s, n as i64 + 1],
            )?;
        }
        tx.commit()?;
    }
    let db = conn.serialize(rusqlite::MAIN_DB)?.to_vec();

    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    zip.start_file("collection.anki2", opts)
        .context("writing the package")?;
    zip.write_all(&db)?;
    zip.start_file("media", opts)?;
    zip.write_all(b"{}")?;
    Ok(zip.finish()?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::io::Read as _;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 13, 9, 0, 0).unwrap()
    }

    fn item(text: &str, note: Option<&str>) -> AnnotationItem {
        AnnotationItem {
            book_title: "Dune & Co".into(),
            book_author: Some("Frank Herbert".into()),
            chapter: Some("Part 1".into()),
            text: text.into(),
            note: note.map(str::to_string),
            style: HighlightStyle::Highlight,
            color: None,
            created: now(),
            location: None,
        }
    }

    fn vocab() -> VocabStore {
        let mut s = VocabStore::new();
        s.add(
            "ephemeral",
            Some("It was an <ephemeral> joy."),
            Some("b1"),
            now(),
        )
        .unwrap();
        s.set_definition("ephemeral", "lasting a very short time")
            .unwrap();
        s
    }

    #[test]
    fn sha1_matches_the_known_vectors() {
        assert_eq!(sha1_first_word(b"abc"), 0xa9993e36);
        assert_eq!(sha1_first_word(b""), 0xda39a3ee);
        assert_eq!(
            sha1_first_word(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            0x84983e44
        );
    }

    #[test]
    fn the_checksum_ignores_markup() {
        assert_eq!(field_checksum("<b>abc</b>"), 0xa9993e36);
    }

    #[test]
    fn a_basic_card_has_the_quote_in_front_and_note_and_source_behind() {
        let n = notes_from_items(
            &[item("Fear is <the> mind-killer.", Some("Litany"))],
            &Options::default(),
        );
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].first, "Fear is &lt;the&gt; mind-killer.");
        assert!(n[0]
            .second
            .starts_with("Litany<br><br><i>Dune &amp; Co, Frank Herbert (Part 1)"));
        assert_eq!(n[0].tags, vec!["bookx", "Dune_Co"]);
    }

    #[test]
    fn a_cloze_card_hides_the_longest_word() {
        let o = Options {
            kind: CardKind::Cloze,
            ..Default::default()
        };
        let n = notes_from_items(&[item("Fear is the mind-killer & more", None)], &o);
        assert_eq!(n[0].first, "Fear is the {{c1::mind-killer}} &amp; more");
        assert_eq!(pick_cloze_word("a b cc"), None);
    }

    #[test]
    fn the_same_quote_twice_and_bookmarks_make_one_note_with_a_stable_guid() {
        let mut bm = item("page 3", None);
        bm.style = HighlightStyle::Bookmark;
        let a = notes_from_items(
            &[
                item("Same quote here.", None),
                item("same  quote here.", Some("x")),
                bm,
            ],
            &Options::default(),
        );
        assert_eq!(a.len(), 1);
        let b = notes_from_items(&[item("Same quote here.", None)], &Options::default());
        assert_eq!(a[0].guid, b[0].guid, "re-exporting never makes a new guid");
    }

    #[test]
    fn vocabulary_cards_carry_the_context_sentence() {
        let title = |id: &str| (id == "b1").then(|| "Some Book".to_string());
        let basic = notes_from_vocab(&vocab(), &Options::default(), &title);
        assert_eq!(basic[0].first, "ephemeral");
        assert!(basic[0].second.contains(
            "lasting a very short time<br><i>It was an &lt;<b>ephemeral</b>&gt; joy.</i>"
        ));
        assert_eq!(basic[0].tags, vec!["bookx", "Some_Book"]);
        let o = Options {
            kind: CardKind::Cloze,
            ..Default::default()
        };
        let cloze = notes_from_vocab(&vocab(), &o, &title);
        assert_eq!(cloze[0].first, "It was an &lt;{{c1::ephemeral}}&gt; joy.");
        assert_eq!(cloze[0].second, "lasting a very short time");
    }

    #[test]
    fn the_text_file_has_the_headers_anki_reads_and_one_line_per_note() {
        let o = Options {
            deck: "Books::Dune".into(),
            ..Default::default()
        };
        let n = notes_from_items(&[item("A, \"quoted\"\nline", None)], &o);
        let tsv = to_delimited(&n, &o, Separator::Tab);
        assert!(tsv.starts_with(
            "#separator:tab\n#html:true\n#notetype:Basic\n#deck:Books::Dune\n#guid column:1\n#tags column:4\n"
        ));
        assert_eq!(tsv.lines().count(), 7);
        let row = tsv.lines().last().unwrap();
        assert_eq!(row.split('\t').count(), 4);
        assert!(row.contains("A, \"quoted\"<br>line"));
        let csv = to_delimited(&n, &o, Separator::Comma);
        assert!(csv.contains("\"A, \"\"quoted\"\"<br>line\""));
    }

    #[test]
    fn the_package_reopens_with_the_collection_the_notes_and_the_models() {
        let mut notes = notes_from_items(
            &[item("Fear is the mind-killer.", Some("n"))],
            &Options::default(),
        );
        notes.extend(notes_from_vocab(
            &vocab(),
            &Options {
                kind: CardKind::Cloze,
                ..Default::default()
            },
            &|_| None,
        ));
        let bytes = to_apkg(&notes, "Bookx::Test", now()).unwrap();

        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut media = String::new();
        z.by_name("media")
            .unwrap()
            .read_to_string(&mut media)
            .unwrap();
        assert_eq!(media, "{}");
        let mut db = Vec::new();
        z.by_name("collection.anki2")
            .unwrap()
            .read_to_end(&mut db)
            .unwrap();

        let mut c = rusqlite::Connection::open_in_memory().unwrap();
        c.deserialize_read_exact(rusqlite::MAIN_DB, std::io::Cursor::new(&db), db.len(), true)
            .unwrap();
        let (ver, models, decks): (i64, String, String) = c
            .query_row("SELECT ver, models, decks FROM col", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert_eq!(ver, 11);
        let models: Value = serde_json::from_str(&models).unwrap();
        let types: Vec<i64> = models
            .as_object()
            .unwrap()
            .values()
            .map(|m| m["type"].as_i64().unwrap())
            .collect();
        assert_eq!(types.len(), 2);
        assert!(types.contains(&0) && types.contains(&1));
        let decks: Value = serde_json::from_str(&decks).unwrap();
        assert!(decks
            .as_object()
            .unwrap()
            .values()
            .any(|d| d["name"] == "Bookx::Test"));

        let n: i64 = c
            .query_row("SELECT count(*) FROM notes", [], |r| r.get(0))
            .unwrap();
        let k: i64 = c
            .query_row("SELECT count(*) FROM cards", [], |r| r.get(0))
            .unwrap();
        assert_eq!((n, k), (2, 2));
        let (flds, tags, guid, csum): (String, String, String, i64) = c
            .query_row(
                "SELECT flds, tags, guid, csum FROM notes ORDER BY id LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(flds.split('\x1f').count(), 2);
        assert!(flds.starts_with("Fear is the mind-killer."));
        assert_eq!(tags, " bookx Dune_Co ");
        assert_eq!(guid, notes[0].guid);
        assert_eq!(csum as u32, field_checksum("Fear is the mind-killer."));
        let dids: i64 = c
            .query_row("SELECT count(DISTINCT did) FROM cards", [], |r| r.get(0))
            .unwrap();
        assert_eq!(dids, 1);
        let integrity: String = c
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
    }
}
