//! The Reports window as plain tables (CE-303..CE-311): every report kind becomes a [`Table`] of rows that know
//! where they point in the book, can be filtered and sorted, and exported as CSV. No GUI in here.

use std::collections::{BTreeMap, HashMap, HashSet};

use bookx_convert::transforms::css::{CssItem, parse_css, split_selectors};
use bookx_core::book::OpenBook;
use bookx_core::util::{is_external, parent_dir, resolve_href};

use crate::charinfo;
use crate::markup::*;
use crate::reports::{
    book_text_files, collect_classes, css_report, document_language, files_report, images_report,
    links_report,
};
use crate::spell::{extract_words, lang_key};

/// Most places kept per row (a letter that occurs a million times does not need a million jump targets).
pub const MAX_LOCS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TableKind {
    Files,
    Words,
    Images,
    StyleRules,
    StyleClasses,
    Characters,
    Links,
}

impl TableKind {
    pub const ALL: [TableKind; 7] = [
        TableKind::Files,
        TableKind::Words,
        TableKind::Images,
        TableKind::StyleRules,
        TableKind::StyleClasses,
        TableKind::Characters,
        TableKind::Links,
    ];

    pub fn id(self) -> &'static str {
        match self {
            TableKind::Files => "files",
            TableKind::Words => "words",
            TableKind::Images => "images",
            TableKind::StyleRules => "rules",
            TableKind::StyleClasses => "classes",
            TableKind::Characters => "chars",
            TableKind::Links => "links",
        }
    }

    pub fn from_id(id: &str) -> Option<TableKind> {
        Self::ALL.into_iter().find(|k| k.id() == id)
    }

    pub fn title(self) -> &'static str {
        match self {
            TableKind::Files => "Files",
            TableKind::Words => "Words",
            TableKind::Images => "Images",
            TableKind::StyleRules => "Style rules",
            TableKind::StyleClasses => "Style classes",
            TableKind::Characters => "Characters",
            TableKind::Links => "Links",
        }
    }
}

/// A place in the book: file, 1-based line and, where known, the byte range in the file's source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loc {
    pub file: String,
    pub line: u32,
    pub range: Option<(usize, usize)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub text: String,
    /// The value used when sorting; `None` sorts by text.
    pub num: Option<f64>,
}

impl Cell {
    fn text(t: impl Into<String>) -> Cell {
        Cell {
            text: t.into(),
            num: None,
        }
    }
    fn num(n: usize) -> Cell {
        Cell {
            text: n.to_string(),
            num: Some(n as f64),
        }
    }
    fn sized(bytes: u64) -> Cell {
        Cell {
            text: format!("{:.1}", bytes as f64 / 1024.0),
            num: Some(bytes as f64),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Row {
    pub cells: Vec<Cell>,
    /// The thing the row is about: the path of a file or image, the word, the class… (what actions work on).
    pub key: String,
    /// Where to jump; more than one for words and characters (the first places they occur).
    pub locs: Vec<Loc>,
}

#[derive(Debug, Clone)]
pub struct Table {
    pub kind: TableKind,
    /// Column titles; the numeric ones are right aligned by the window.
    pub columns: Vec<(&'static str, bool)>,
    pub rows: Vec<Row>,
}

impl Table {
    /// Indices of the rows with `needle` (case-insensitive) in any cell, in table order.
    pub fn filter(&self, needle: &str) -> Vec<usize> {
        let n = needle.trim().to_lowercase();
        (0..self.rows.len())
            .filter(|i| self.matches(*i, &n))
            .collect()
    }

    /// Orders two rows by a column: numbers by value, text case-insensitively.
    pub fn compare(&self, col: usize, a: usize, b: usize) -> std::cmp::Ordering {
        let (x, y) = (&self.rows[a].cells[col], &self.rows[b].cells[col]);
        match (x.num, y.num) {
            (Some(p), Some(q)) => p.partial_cmp(&q).unwrap_or(std::cmp::Ordering::Equal),
            _ => x.text.to_lowercase().cmp(&y.text.to_lowercase()),
        }
    }

    /// Sorts row indices by a column (ties keep table order).
    pub fn sort(&self, idx: &mut [usize], col: usize, descending: bool) {
        idx.sort_by(|a, b| {
            let ord = self.compare(col, *a, *b);
            if descending { ord.reverse() } else { ord }
        });
    }

    /// Whether row `i` has the lower-case `needle` in any cell (an empty needle matches).
    pub fn matches(&self, i: usize, needle: &str) -> bool {
        needle.is_empty()
            || self.rows[i]
                .cells
                .iter()
                .any(|c| c.text.to_lowercase().contains(needle))
    }

    /// CSV (RFC 4180, CRLF) of the given rows, with a header line.
    pub fn to_csv(&self, idx: &[usize]) -> String {
        let mut out = String::new();
        let head: Vec<String> = self.columns.iter().map(|(t, _)| csv_field(t)).collect();
        out.push_str(&head.join(","));
        out.push_str("\r\n");
        for i in idx {
            let row: Vec<String> = self.rows[*i]
                .cells
                .iter()
                .map(|c| csv_field(&c.text))
                .collect();
            out.push_str(&row.join(","));
            out.push_str("\r\n");
        }
        out
    }
}

/// Quotes a CSV field. A text that a spreadsheet would run as a formula (`=`, `+`, `@`, or `-` before a letter)
/// gets a leading apostrophe.
pub fn csv_field(s: &str) -> String {
    let risky = match s.chars().next() {
        Some('=') | Some('+') | Some('@') => true,
        Some('-') => s
            .chars()
            .nth(1)
            .is_some_and(|c| !c.is_ascii_digit() && c != '.'),
        _ => false,
    };
    let s = if risky {
        format!("'{s}")
    } else {
        s.to_string()
    };
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

pub fn build_table(book: &OpenBook, kind: TableKind) -> Table {
    match kind {
        TableKind::Files => files_table(book),
        TableKind::Words => words_table(book),
        TableKind::Images => images_table(book),
        TableKind::StyleRules => style_rules_table(book),
        TableKind::StyleClasses => style_classes_table(book),
        TableKind::Characters => chars_table(book),
        TableKind::Links => links_table(book),
    }
}

/// Line numbers for byte offsets of one source.
struct Lines(Vec<usize>);

impl Lines {
    fn new(src: &str) -> Lines {
        Lines(
            src.bytes()
                .enumerate()
                .filter(|(_, b)| *b == b'\n')
                .map(|(i, _)| i)
                .collect(),
        )
    }
    fn line(&self, at: usize) -> u32 {
        self.0.partition_point(|n| *n < at) as u32 + 1
    }
}

fn loc_file(file: &str) -> Vec<Loc> {
    vec![Loc {
        file: file.to_string(),
        line: 1,
        range: None,
    }]
}

// ---------------------------------------------------------------------------

pub fn files_table(book: &OpenBook) -> Table {
    let rep = files_report(book);
    let mut files = rep.files;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let rows = files
        .into_iter()
        .map(|f| {
            let folder = parent_dir(&f.path).to_string();
            let name = f.path.rsplit('/').next().unwrap_or(&f.path).to_string();
            Row {
                cells: vec![
                    Cell::text(folder),
                    Cell::text(name),
                    Cell::sized(f.size),
                    Cell::text(f.kind.clone()),
                    if f.kind == "text" {
                        Cell::num(f.words)
                    } else {
                        Cell::text("")
                    },
                ],
                locs: loc_file(&f.path),
                key: f.path,
            }
        })
        .collect();
    Table {
        kind: TableKind::Files,
        columns: vec![
            ("Folder", false),
            ("Name", false),
            ("Size (KB)", true),
            ("Type", false),
            ("Words", true),
        ],
        rows,
    }
}

pub fn words_table(book: &OpenBook) -> Table {
    let book_lang = lang_key(&book.package.metadata.language);
    let mut map: HashMap<(String, String), (usize, Vec<Loc>)> = HashMap::new();
    for (path, src) in book_sources(book) {
        let doc = document_language(&src).map(|l| lang_key(&l));
        let lines = Lines::new(&src);
        for w in extract_words(&src) {
            let lang = w
                .lang
                .as_deref()
                .map(lang_key)
                .or_else(|| doc.clone())
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| {
                    if book_lang.is_empty() {
                        "und".into()
                    } else {
                        book_lang.clone()
                    }
                });
            let e = map.entry((w.word.to_lowercase(), lang)).or_default();
            e.0 += 1;
            if e.1.len() < MAX_LOCS {
                e.1.push(Loc {
                    file: path.clone(),
                    line: lines.line(w.start),
                    range: Some((w.start, w.end)),
                });
            }
        }
    }
    let mut rows: Vec<Row> = map
        .into_iter()
        .map(|((word, lang), (n, locs))| Row {
            cells: vec![Cell::text(word.clone()), Cell::text(lang), Cell::num(n)],
            key: word,
            locs,
        })
        .collect();
    rows.sort_by(|a, b| {
        b.cells[2]
            .num
            .partial_cmp(&a.cells[2].num)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.key.cmp(&b.key))
    });
    Table {
        kind: TableKind::Words,
        columns: vec![("Word", false), ("Language", false), ("Count", true)],
        rows,
    }
}

pub fn images_table(book: &OpenBook) -> Table {
    let rep = images_report(book);
    let mut images = rep.images;
    images.sort_by(|a, b| a.path.cmp(&b.path));
    let rows = images
        .into_iter()
        .map(|i| {
            let res = match (i.width, i.height) {
                (Some(w), Some(h)) => format!("{w}×{h}"),
                _ => String::new(),
            };
            Row {
                cells: vec![
                    Cell::text(i.path.clone()),
                    Cell::sized(i.bytes),
                    Cell::num(i.references),
                    Cell {
                        text: res,
                        num: i.width.zip(i.height).map(|(w, h)| w as f64 * h as f64),
                    },
                    Cell::text(i.format),
                ],
                locs: loc_file(&i.path),
                key: i.path,
            }
        })
        .collect();
    Table {
        kind: TableKind::Images,
        columns: vec![
            ("Image", false),
            ("Size (KB)", true),
            ("Times used", true),
            ("Resolution", true),
            ("Format", false),
        ],
        rows,
    }
}

/// Paths of the images nothing refers to.
pub fn unused_images(book: &OpenBook) -> Vec<String> {
    images_report(book)
        .images
        .into_iter()
        .filter(|i| i.references == 0)
        .map(|i| i.path)
        .collect()
}

pub fn style_rules_table(book: &OpenBook) -> Table {
    let rep = css_report(book);
    let mut rows = Vec::new();
    for f in rep.files {
        let src = book.archive.get_string(&f.path).unwrap_or_default();
        let lines = Lines::new(&src);
        let mut from = 0;
        for r in f.rules {
            let at = src[from.min(src.len())..]
                .find(r.selector.split_whitespace().next().unwrap_or(&r.selector))
                .map(|p| p + from)
                .unwrap_or(from);
            from = at;
            rows.push(Row {
                cells: vec![
                    Cell::text(r.selector.clone()),
                    Cell::text(f.path.clone()),
                    match r.matches {
                        Some(n) => Cell::num(n),
                        None => Cell::text("?"),
                    },
                    Cell::num(r.declarations),
                ],
                key: r.selector,
                locs: vec![Loc {
                    file: f.path.clone(),
                    line: lines.line(at),
                    range: None,
                }],
            });
        }
    }
    Table {
        kind: TableKind::StyleRules,
        columns: vec![
            ("Rule", false),
            ("Stylesheet", false),
            ("Matches", true),
            ("Declarations", true),
        ],
        rows,
    }
}

/// How often each class is used in the markup, and in which stylesheets it is defined.
#[derive(Debug, Default, Clone)]
pub struct ClassUse {
    pub used: usize,
    pub defined_in: Vec<String>,
    pub first_use: Option<Loc>,
    pub first_rule: Option<Loc>,
}

pub fn class_usage(book: &OpenBook) -> BTreeMap<String, ClassUse> {
    let mut out: BTreeMap<String, ClassUse> = BTreeMap::new();
    for (path, src) in book_sources(book) {
        let lines = Lines::new(&src);
        for ev in scan(&src) {
            if let Event::Open { range, .. } = ev {
                let tag = &src[range.clone()];
                if let Some(c) = tag_attr(tag, "class") {
                    for name in c.split_whitespace() {
                        let u = out.entry(name.to_string()).or_default();
                        u.used += 1;
                        if u.first_use.is_none() {
                            u.first_use = Some(Loc {
                                file: path.clone(),
                                line: lines.line(range.start),
                                range: Some((range.start, range.end)),
                            });
                        }
                    }
                }
            }
        }
    }
    for path in css_files(&book.archive) {
        let Some(css) = book.archive.get_string(&path) else {
            continue;
        };
        let lines = Lines::new(&css);
        for item in parse_css(&css) {
            if let CssItem::Rule { selectors, .. } = item {
                for s in split_selectors(selectors) {
                    let mut names = HashSet::new();
                    collect_classes(&s, &mut names);
                    for n in names {
                        let u = out.entry(n.clone()).or_default();
                        if !u.defined_in.contains(&path) {
                            u.defined_in.push(path.clone());
                        }
                        if u.first_rule.is_none() {
                            let at = css.find(&format!(".{n}")).unwrap_or(0);
                            u.first_rule = Some(Loc {
                                file: path.clone(),
                                line: lines.line(at),
                                range: None,
                            });
                        }
                    }
                }
            }
        }
    }
    out
}

pub fn style_classes_table(book: &OpenBook) -> Table {
    let rows = class_usage(book)
        .into_iter()
        .map(|(name, u)| {
            let status = match (u.used > 0, !u.defined_in.is_empty()) {
                (true, true) => "Used",
                (true, false) => "Not defined",
                (false, _) => "Unused",
            };
            let mut locs = Vec::new();
            locs.extend(u.first_use.clone());
            locs.extend(u.first_rule.clone());
            Row {
                cells: vec![
                    Cell::text(name.clone()),
                    Cell::num(u.used),
                    Cell::text(status),
                    Cell::text(u.defined_in.join(", ")),
                ],
                key: name,
                locs,
            }
        })
        .collect();
    Table {
        kind: TableKind::StyleClasses,
        columns: vec![
            ("Class", false),
            ("Times used", true),
            ("Status", false),
            ("Defined in", false),
        ],
        rows,
    }
}

pub fn chars_table(book: &OpenBook) -> Table {
    let mut map: HashMap<char, (usize, Vec<Loc>)> = HashMap::new();
    for (path, src) in book_sources(book) {
        let lines = Lines::new(&src);
        for run in text_runs(&src, DEFAULT_SKIP) {
            let (decoded, offs) = decode_with_offsets(&src[run.clone()]);
            for (i, c) in decoded.char_indices() {
                let e = map.entry(c).or_default();
                e.0 += 1;
                if e.1.len() < MAX_LOCS {
                    let start = run.start + offs[i];
                    let end = run.start + offs[(i + c.len_utf8()).min(offs.len() - 1)];
                    e.1.push(Loc {
                        file: path.clone(),
                        line: lines.line(start),
                        range: Some((start, end.max(start))),
                    });
                }
            }
        }
    }
    let mut rows: Vec<Row> = map
        .into_iter()
        .map(|(c, (n, locs))| {
            let shown = if c.is_control() || c.is_whitespace() {
                format!("{}", c.escape_default())
            } else {
                c.to_string()
            };
            Row {
                cells: vec![
                    Cell::text(shown),
                    Cell {
                        text: charinfo::code_label(c),
                        num: Some(c as u32 as f64),
                    },
                    Cell::text(charinfo::name(c).unwrap_or_default()),
                    Cell::num(n),
                ],
                key: c.to_string(),
                locs,
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        b.cells[3]
            .num
            .partial_cmp(&a.cells[3].num)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                a.cells[1]
                    .num
                    .partial_cmp(&b.cells[1].num)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    Table {
        kind: TableKind::Characters,
        columns: vec![
            ("Character", false),
            ("Code point", true),
            ("Name", false),
            ("Count", true),
        ],
        rows,
    }
}

pub fn links_table(book: &OpenBook) -> Table {
    let mut ids: HashMap<String, HashSet<String>> = HashMap::new();
    let sources: Vec<(String, String)> = markup_files(&book.archive)
        .into_iter()
        .filter_map(|f| book.archive.get_string(&f).map(|t| (f, t)))
        .collect();
    for (f, t) in &sources {
        ids.insert(f.clone(), defined_ids(t).into_iter().collect());
    }
    let mut rows = Vec::new();
    for (file, src) in &sources {
        let lines = Lines::new(src);
        let dir = parent_dir(file);
        let events = scan(src);
        for (k, ev) in events.iter().enumerate() {
            let Event::Open { name, range, .. } = ev else {
                continue;
            };
            if name != "a" {
                continue;
            }
            let tag = &src[range.clone()];
            let Some(href) = tag_attr(tag, "href") else {
                continue;
            };
            let text = events[k + 1..]
                .iter()
                .take_while(|e| !matches!(e, Event::Close{name, ..} if name == "a"))
                .filter_map(|e| match e {
                    Event::Text(r) => Some(decode_entities(&src[r.clone()])),
                    _ => None,
                })
                .collect::<String>();
            let h = href.trim();
            let (kind, target, anchor, ok) = if h.to_ascii_lowercase().starts_with("mailto:") {
                ("Mail", h.to_string(), String::new(), true)
            } else if is_external(h) {
                ("External", h.to_string(), String::new(), true)
            } else if h.is_empty() {
                ("Internal", String::new(), String::new(), false)
            } else {
                let (path, frag) = resolve_href(dir, h);
                let target = if path.is_empty() { file.clone() } else { path };
                let ok = if !book.archive.contains(&target) {
                    false
                } else if let (Some(f), Some(set)) = (frag.as_ref(), ids.get(&target)) {
                    set.contains(f)
                } else {
                    true
                };
                ("Internal", target, frag.unwrap_or_default(), ok)
            };
            rows.push(Row {
                cells: vec![
                    Cell::text(file.clone()),
                    Cell::text(text.trim().to_string()),
                    Cell::text(target.clone()),
                    Cell::text(anchor),
                    Cell::text(kind),
                    Cell::text(if ok { "OK" } else { "Broken" }),
                ],
                key: href.clone(),
                locs: {
                    let mut v = vec![Loc {
                        file: file.clone(),
                        line: lines.line(range.start),
                        range: Some((range.start, range.end)),
                    }];
                    if kind == "Internal" && ok && book.archive.contains(&target) {
                        v.push(Loc {
                            file: target,
                            line: 1,
                            range: None,
                        });
                    }
                    v
                },
            });
        }
    }
    for m in links_report(book).missing_resources {
        let src = book.archive.get_string(&m.file).unwrap_or_default();
        let at = src.find(&m.href).unwrap_or(0);
        rows.push(Row {
            cells: vec![
                Cell::text(m.file.clone()),
                Cell::text(""),
                Cell::text(m.href.clone()),
                Cell::text(""),
                Cell::text("Resource"),
                Cell::text("Broken"),
            ],
            key: m.href,
            locs: vec![Loc {
                file: m.file,
                line: Lines::new(&src).line(at),
                range: Some((at, at)),
            }],
        });
    }
    Table {
        kind: TableKind::Links,
        columns: vec![
            ("Source", false),
            ("Text", false),
            ("Target", false),
            ("Anchor", false),
            ("Kind", false),
            ("Status", false),
        ],
        rows,
    }
}

/// Counts for the window's subtitle line.
pub fn summary(t: &Table) -> String {
    match t.kind {
        TableKind::Links => {
            let broken = t
                .rows
                .iter()
                .filter(|r| r.cells[5].text == "Broken")
                .count();
            format!("{} links, {} broken", t.rows.len(), broken)
        }
        TableKind::Images => {
            let unused = t.rows.iter().filter(|r| r.cells[2].text == "0").count();
            format!("{} images, {} unused", t.rows.len(), unused)
        }
        TableKind::StyleClasses => {
            let unused = t
                .rows
                .iter()
                .filter(|r| r.cells[2].text == "Unused")
                .count();
            format!("{} classes, {} unused", t.rows.len(), unused)
        }
        _ => format!("{} rows", t.rows.len()),
    }
}

fn book_sources(book: &OpenBook) -> Vec<(String, String)> {
    book_text_files(book)
        .into_iter()
        .filter_map(|p| book.archive.get_string(&p).map(|t| (p, t)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{make_book, png};

    fn book() -> OpenBook {
        let mut b = make_book(&[
            "<h1>One</h1><p class=\"used\">The cat &amp; the dog. Café café.</p><p><a href=\"chapter-2.xhtml#x\">next</a> <a href=\"chapter-2.xhtml#nope\">bad</a> <a href=\"http://example.com/\">web</a> <img src=\"images/p.png\"/> <img src=\"images/gone.png\"/></p>",
            "<p id=\"x\" class=\"other\" lang=\"de\">Der Hund</p>",
        ]);
        b.archive.set("OEBPS/images/p.png", png(40, 30));
        b.archive.set("OEBPS/images/unused.png", png(8, 8));
        b.archive.set_string(
            "OEBPS/style.css",
            "body { margin: 0 }\n.used { color: red }\n.unused { color: blue }\n.other { x: y }\nh9 { a: b }\n",
        );
        b
    }

    fn col(t: &Table, name: &str) -> usize {
        t.columns.iter().position(|(c, _)| *c == name).unwrap()
    }

    fn cell<'a>(t: &'a Table, key: &str, col_name: &str) -> &'a str {
        let c = col(t, col_name);
        &t.rows.iter().find(|r| r.key == key).unwrap().cells[c].text
    }

    #[test]
    fn files_table_has_folder_name_size_type_and_words() {
        let b = book();
        let t = files_table(&b);
        let ch = b.spine_paths()[0].clone();
        let row = t.rows.iter().find(|r| r.key == ch).unwrap();
        assert_eq!(row.cells[0].text, "OEBPS");
        assert_eq!(row.cells[1].text, "chapter-1.xhtml");
        assert_eq!(row.cells[3].text, "text");
        assert!(row.cells[4].num.unwrap() >= 10.0, "the chapter has words");
        assert_eq!(
            cell(&t, "OEBPS/style.css", "Words"),
            "",
            "only text has a word count"
        );
        assert_eq!(row.locs[0].file, ch);
        let size: f64 = row.cells[2].text.parse().unwrap();
        assert!((size * 1024.0 - row.cells[2].num.unwrap()).abs() < 52.0);
    }

    #[test]
    fn words_are_listed_in_full_with_language_and_places() {
        let b = book();
        let t = words_table(&b);
        let the = t
            .rows
            .iter()
            .find(|r| r.key == "the" && r.cells[1].text == "en")
            .unwrap();
        assert_eq!(the.cells[2].text, "2", "The and the are one word");
        assert_eq!(the.locs.len(), 2);
        let src = b.archive.get_string(&the.locs[0].file).unwrap();
        let (s, e) = the.locs[0].range.unwrap();
        assert_eq!(src[s..e].to_lowercase(), "the");
        assert_eq!(src[..s].matches('\n').count() as u32 + 1, the.locs[0].line);
        let hund = t.rows.iter().find(|r| r.key == "hund").unwrap();
        assert_eq!(hund.cells[1].text, "de");
        assert!(
            t.rows
                .iter()
                .any(|r| r.key == "café" && r.cells[2].text == "2")
        );
        assert!(
            !t.rows.iter().any(|r| r.key == "amp"),
            "entities are decoded"
        );
        assert!(t.rows[0].cells[2].num >= t.rows[t.rows.len() - 1].cells[2].num);
    }

    #[test]
    fn images_show_use_and_resolution_and_unused_ones_are_listed() {
        let b = book();
        let t = images_table(&b);
        assert_eq!(cell(&t, "OEBPS/images/p.png", "Times used"), "1");
        assert_eq!(cell(&t, "OEBPS/images/p.png", "Resolution"), "40×30");
        assert_eq!(cell(&t, "OEBPS/images/unused.png", "Times used"), "0");
        assert_eq!(unused_images(&b), ["OEBPS/images/unused.png"]);
        assert!(summary(&t).contains("1 unused"));
    }

    #[test]
    fn style_rules_have_matches_and_lines() {
        let b = book();
        let t = style_rules_table(&b);
        assert_eq!(cell(&t, ".used", "Matches"), "1");
        assert_eq!(cell(&t, ".unused", "Matches"), "0");
        let r = t.rows.iter().find(|r| r.key == ".unused").unwrap();
        assert_eq!(
            (r.locs[0].file.as_str(), r.locs[0].line),
            ("OEBPS/style.css", 3)
        );
        let r = t.rows.iter().find(|r| r.key == "h9").unwrap();
        assert_eq!(r.locs[0].line, 5);
    }

    #[test]
    fn style_classes_are_used_unused_or_not_defined() {
        let mut b = book();
        let p = b.spine_paths()[0].clone();
        let s = b
            .archive
            .get_string(&p)
            .unwrap()
            .replace("<h1>", "<h1 class=\"ghost used\">");
        b.archive.set_string(&p, &s);
        let t = style_classes_table(&b);
        assert_eq!(cell(&t, "used", "Status"), "Used");
        assert_eq!(cell(&t, "used", "Times used"), "2");
        assert_eq!(cell(&t, "unused", "Status"), "Unused");
        assert_eq!(cell(&t, "ghost", "Status"), "Not defined");
        assert_eq!(cell(&t, "other", "Defined in"), "OEBPS/style.css");
        let u = t.rows.iter().find(|r| r.key == "unused").unwrap();
        assert_eq!(u.locs.len(), 1);
        assert_eq!(u.locs[0].line, 3);
        let used = t.rows.iter().find(|r| r.key == "used").unwrap();
        assert_eq!(used.locs.len(), 2, "first use and first rule");
        assert!(summary(&t).contains("1 unused"));
    }

    #[test]
    fn characters_have_names_codes_counts_and_places() {
        let b = book();
        let t = chars_table(&b);
        assert_eq!(cell(&t, "é", "Code point"), "U+00E9");
        assert_eq!(cell(&t, "é", "Name"), "LATIN SMALL LETTER E WITH ACUTE");
        assert_eq!(cell(&t, "é", "Count"), "2");
        let e = t.rows.iter().find(|r| r.key == "é").unwrap();
        let src = b.archive.get_string(&e.locs[0].file).unwrap();
        let (s, en) = e.locs[0].range.unwrap();
        assert_eq!(decode_entities(&src[s..en]), "é");
        let amp = t.rows.iter().find(|r| r.key == "&").unwrap();
        let src = b.archive.get_string(&amp.locs[0].file).unwrap();
        let (s, en) = amp.locs[0].range.unwrap();
        assert_eq!(&src[s..en], "&amp;", "the range covers the entity");
        assert!(
            t.rows.iter().any(|r| r.cells[0].text == "\\n"),
            "whitespace is shown escaped"
        );
        assert!(
            !t.rows.iter().any(|r| r.key == "{"),
            "style content is not text"
        );
    }

    #[test]
    fn links_table_marks_broken_internal_links_and_resources() {
        let b = book();
        let t = links_table(&b);
        let status = |text: &str| {
            t.rows
                .iter()
                .find(|r| r.cells[1].text == text)
                .unwrap()
                .cells[5]
                .text
                .clone()
        };
        assert_eq!(status("next"), "OK");
        assert_eq!(status("bad"), "Broken");
        assert_eq!(status("web"), "OK");
        let next = t.rows.iter().find(|r| r.cells[1].text == "next").unwrap();
        assert_eq!(next.cells[3].text, "x");
        assert_eq!(next.cells[2].text, "OEBPS/chapter-2.xhtml");
        assert_eq!(next.locs.len(), 2, "source and target");
        assert_eq!(next.locs[1].file, "OEBPS/chapter-2.xhtml");
        let web = t.rows.iter().find(|r| r.cells[1].text == "web").unwrap();
        assert_eq!(web.cells[4].text, "External");
        assert!(t.rows.iter().any(|r| r.cells[4].text == "Resource"
            && r.cells[2].text == "images/gone.png"
            && r.cells[5].text == "Broken"));
        assert!(summary(&t).contains("2 broken"));
        let src = b.archive.get_string(&next.locs[0].file).unwrap();
        let (s, e) = next.locs[0].range.unwrap();
        assert!(src[s..e].starts_with("<a "));
    }

    #[test]
    fn filter_sort_and_csv() {
        let b = book();
        let t = words_table(&b);
        let hits = t.filter("  CAF ");
        assert!(hits.iter().all(|i| t.rows[*i].key.contains("caf")) && !hits.is_empty());
        assert_eq!(t.filter("").len(), t.rows.len());
        let mut idx: Vec<usize> = (0..t.rows.len()).collect();
        t.sort(&mut idx, 0, false);
        assert!(t.rows[idx[0]].key <= t.rows[idx[1]].key);
        t.sort(&mut idx, 2, true);
        assert!(t.rows[idx[0]].cells[2].num >= t.rows[idx[1]].cells[2].num);
        let csv = t.to_csv(&hits);
        assert!(csv.starts_with("Word,Language,Count\r\n"));
        assert_eq!(csv.lines().count(), hits.len() + 1);
        let f = files_table(&b);
        let mut idx: Vec<usize> = (0..f.rows.len()).collect();
        f.sort(&mut idx, 2, true);
        assert!(
            f.rows[idx[0]].cells[2].num >= f.rows[idx[1]].cells[2].num,
            "sizes sort as numbers"
        );
    }

    #[test]
    fn csv_fields_are_quoted_and_formulas_defused() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("two\nlines"), "\"two\nlines\"");
        assert_eq!(csv_field("=SUM(A1)"), "'=SUM(A1)");
        assert_eq!(csv_field("@x"), "'@x");
        assert_eq!(csv_field("-cmd"), "'-cmd");
        assert_eq!(csv_field("-5"), "-5");
        assert_eq!(csv_field("-"), "-");
    }

    #[test]
    fn table_ids_round_trip_and_every_kind_builds() {
        let b = book();
        for k in TableKind::ALL {
            assert_eq!(TableKind::from_id(k.id()), Some(k));
            let t = build_table(&b, k);
            assert_eq!(t.kind, k);
            assert!(!t.rows.is_empty(), "{} is empty", k.title());
            for r in &t.rows {
                assert_eq!(r.cells.len(), t.columns.len(), "{}", k.title());
                assert!(!r.locs.is_empty() || k == TableKind::StyleClasses);
            }
        }
        assert_eq!(TableKind::from_id("nope"), None);
    }
}
