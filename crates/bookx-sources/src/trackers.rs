//! Reading-tracker exchange: import a Goodreads, StoryGraph or Hardcover CSV export (reading
//! status, rating, date read, shelves) and export the library in the layouts those services import.
//!
//! Pure functions: [`parse`] reads the CSV, [`match_rows`] pairs the rows with library books by
//! ISBN and then by title and author, [`plan`] says what would change, [`export`] writes CSV.
//! Nothing here touches the library or the network.
//!
//! Columns are found by name, so the three services (and slightly different export versions) work
//! with one reader. Rating scales: the services use 0 to 5 stars, the library 0 to 10 (2 per star).

use bookx_core::library::Book;
use chrono::NaiveDate;

use crate::authors::same_authors;
use crate::dedup::title_key;
use crate::isbn;

/// Reading state as the services know it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackerStatus {
    ToRead,
    Reading,
    Read,
    /// Paused or did not finish.
    OnHold,
}

impl TrackerStatus {
    fn parse(s: &str) -> Option<TrackerStatus> {
        let s = s.trim().to_lowercase().replace(['_', '-'], " ");
        Some(match s.as_str() {
            "read" | "finished" | "completed" => TrackerStatus::Read,
            "currently reading" | "reading" | "currently-reading" | "in progress" => {
                TrackerStatus::Reading
            }
            "to read" | "want to read" | "tbr" | "wishlist" => TrackerStatus::ToRead,
            "paused" | "on hold" | "did not finish" | "dnf" | "abandoned" | "stopped" => {
                TrackerStatus::OnHold
            }
            _ => return None,
        })
    }
}

/// One book of a tracker export.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackerRow {
    pub title: String,
    pub authors: Vec<String>,
    pub isbn: Option<String>,
    pub status: Option<TrackerStatus>,
    /// 0 to 10 (library scale); `None` when the row has no rating.
    pub rating: Option<u8>,
    pub date_read: Option<NaiveDate>,
    /// Shelves or tags, without the shelves that only say the status.
    pub shelves: Vec<String>,
}

/// Splits CSV text (RFC 4180: quotes, doubled quotes, newlines inside quotes; BOM and CRLF are fine).
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows.retain(|r| !(r.len() == 1 && r[0].trim().is_empty()));
    rows
}

fn find_col(header: &[String], names: &[&str]) -> Option<usize> {
    let h: Vec<String> = header
        .iter()
        .map(|c| c.trim().to_lowercase().replace(['_', '-'], " "))
        .collect();
    names.iter().find_map(|n| h.iter().position(|c| c == n))
}

/// Goodreads writes ISBNs as `="0441172717"` to keep spreadsheets from mangling them.
fn clean_isbn(s: &str) -> Option<String> {
    let s: String = s.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    isbn::canonical(&s)
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    // "Dates Read" can hold several ranges: take the last date in the text
    let last = s
        .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .rfind(|p| p.chars().any(|c| c.is_ascii_digit()))?;
    let last = last.split('T').next().unwrap_or(last);
    for f in ["%Y/%m/%d", "%Y-%m-%d", "%m/%d/%Y", "%d/%m/%Y"] {
        if let Ok(d) = NaiveDate::parse_from_str(last, f) {
            return Some(d);
        }
    }
    None
}

fn stars_to_rating(s: &str) -> Option<u8> {
    let v: f64 = s.trim().parse().ok()?;
    if v <= 0.0 {
        return None;
    }
    Some(((v.min(5.0) * 2.0).round()) as u8)
}

/// Reads a tracker CSV. Rows without a title are skipped. An export without a title column is an error.
pub fn parse(text: &str) -> Result<Vec<TrackerRow>, String> {
    let rows = parse_csv(text);
    let Some(header) = rows.first() else {
        return Err("the file is empty".into());
    };
    let title = find_col(header, &["title", "book title", "name"]).ok_or(
        "no Title column: this does not look like a Goodreads, StoryGraph or Hardcover export",
    )?;
    let author = find_col(header, &["author", "authors", "author name"]);
    let extra_authors = find_col(header, &["additional authors", "contributors"]);
    let isbn13 = find_col(header, &["isbn13", "isbn 13", "isbn/uid", "isbn"]);
    let isbn10 = find_col(header, &["isbn", "isbn10", "isbn 10"]);
    let status = find_col(
        header,
        &["exclusive shelf", "read status", "status", "reading status"],
    );
    let rating = find_col(header, &["my rating", "star rating", "rating"]);
    let date = find_col(
        header,
        &[
            "date read",
            "last date read",
            "date finished",
            "finished at",
            "read at",
            "dates read",
        ],
    );
    let shelves = find_col(header, &["bookshelves", "shelves", "tags", "lists"]);
    let get = |r: &[String], i: Option<usize>| -> String {
        i.and_then(|i| r.get(i)).cloned().unwrap_or_default()
    };
    let mut out = Vec::new();
    for r in &rows[1..] {
        let t = get(r, Some(title)).trim().to_string();
        if t.is_empty() {
            continue;
        }
        let mut authors: Vec<String> = Vec::new();
        for a in [get(r, author), get(r, extra_authors)] {
            for part in a.split([',', ';']) {
                let p = part.trim();
                if !p.is_empty() && !authors.iter().any(|x| x == p) {
                    authors.push(p.to_string());
                }
            }
        }
        // Goodreads authors are one name each ("Author", "Additional Authors"); a comma in the
        // main column is only a separator in StoryGraph, which lists names that way too.
        let isbn_value = [get(r, isbn13), get(r, isbn10)]
            .iter()
            .find_map(|s| clean_isbn(s));
        let status_text = get(r, status);
        let st = TrackerStatus::parse(&status_text);
        let mut shelf_list: Vec<String> = get(r, shelves)
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && TrackerStatus::parse(s).is_none())
            .collect();
        shelf_list.dedup();
        out.push(TrackerRow {
            title: t,
            authors,
            isbn: isbn_value,
            status: st,
            rating: stars_to_rating(&get(r, rating)),
            date_read: parse_date(&get(r, date)),
            shelves: shelf_list,
        });
    }
    Ok(out)
}

/// How a row was matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchBy {
    Isbn,
    TitleAuthor,
}

/// A row with the library book it belongs to, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct Matched {
    pub row: usize,
    pub book: Option<u64>,
    pub by: Option<MatchBy>,
}

/// Pairs rows with books: by ISBN first, then by title (without subtitle and articles) and authors.
pub fn match_rows(rows: &[TrackerRow], books: &[Book]) -> Vec<Matched> {
    let book_isbns: Vec<Vec<String>> = books
        .iter()
        .map(|b| {
            b.meta
                .identifiers
                .iter()
                .filter(|i| i.scheme.to_lowercase().starts_with("isbn") || i.scheme.is_empty())
                .filter_map(|i| isbn::canonical(&i.value))
                .collect()
        })
        .collect();
    let keys: Vec<String> = books
        .iter()
        .map(|b| title_key(&b.meta.title).main)
        .collect();
    rows.iter()
        .enumerate()
        .map(|(ri, r)| {
            if let Some(i) = &r.isbn {
                if let Some(bi) = book_isbns
                    .iter()
                    .position(|list| list.iter().any(|x| isbn::same_isbn(x, i)))
                {
                    return Matched {
                        row: ri,
                        book: Some(books[bi].id),
                        by: Some(MatchBy::Isbn),
                    };
                }
            }
            let tk = title_key(&r.title).main;
            let hit = books.iter().enumerate().find(|(bi, b)| {
                !tk.is_empty()
                    && keys[*bi] == tk
                    && (r.authors.is_empty()
                        || b.meta.authors.is_empty()
                        || same_authors(&r.authors, &b.meta.authors))
            });
            Matched {
                row: ri,
                book: hit.map(|(_, b)| b.id),
                by: hit.map(|_| MatchBy::TitleAuthor),
            }
        })
        .collect()
}

/// What an import would change in one book.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackerUpdate {
    pub book: u64,
    pub title: String,
    /// New rating (0 to 10), when the row has one and the book has none (or `overwrite`).
    pub rating: Option<u8>,
    pub status: Option<TrackerStatus>,
    pub date_read: Option<NaiveDate>,
    /// Tags to add: the shelves, and `to-read` for a to-read row.
    pub add_tags: Vec<String>,
}

impl TrackerUpdate {
    pub fn is_empty(&self) -> bool {
        self.rating.is_none()
            && self.status.is_none()
            && self.date_read.is_none()
            && self.add_tags.is_empty()
    }
}

/// The changes an import would make. A rating the book already has is kept unless `overwrite`;
/// tags already on the book are not repeated.
pub fn plan(
    rows: &[TrackerRow],
    matches: &[Matched],
    books: &[Book],
    overwrite: bool,
) -> Vec<TrackerUpdate> {
    let mut out = Vec::new();
    for m in matches {
        let (Some(id), Some(row)) = (m.book, rows.get(m.row)) else {
            continue;
        };
        let Some(book) = books.iter().find(|b| b.id == id) else {
            continue;
        };
        let rating = row
            .rating
            .filter(|_| overwrite || book.meta.rating == 0)
            .filter(|r| *r != book.meta.rating);
        let mut tags: Vec<String> = row.shelves.clone();
        if row.status == Some(TrackerStatus::ToRead) {
            tags.push("to-read".into());
        }
        tags.retain(|t| !book.meta.tags.iter().any(|x| x.eq_ignore_ascii_case(t)));
        tags.dedup();
        let date = row
            .date_read
            .filter(|d| book.last_read.is_none_or(|l| l.date_naive() != *d));
        let u = TrackerUpdate {
            book: id,
            title: book.meta.title.clone(),
            rating,
            status: row.status.filter(|s| *s != TrackerStatus::ToRead),
            date_read: date,
            add_tags: tags,
        };
        if !u.is_empty() {
            out.push(u);
        }
    }
    out
}

/// Which service's CSV layout to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    /// The Goodreads export layout; Hardcover and StoryGraph also import it.
    Goodreads,
    /// StoryGraph's own layout.
    StoryGraph,
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn stars(rating: u8) -> String {
    if rating == 0 {
        "0".into()
    } else {
        rating.div_ceil(2).to_string()
    }
}

/// The library as CSV. `status_of` gives the reading status of a book (from the catalogue).
pub fn export(
    books: &[Book],
    status_of: &dyn Fn(&Book) -> Option<TrackerStatus>,
    service: Service,
) -> String {
    let mut out = String::new();
    match service {
        Service::Goodreads => out.push_str(
            "Title,Author,Additional Authors,ISBN,ISBN13,My Rating,Date Read,Bookshelves,Exclusive Shelf\n",
        ),
        Service::StoryGraph => {
            out.push_str("Title,Authors,ISBN/UID,Read Status,Star Rating,Last Date Read,Tags\n")
        }
    }
    for b in books {
        let st = status_of(b).unwrap_or(TrackerStatus::ToRead);
        let date = b
            .last_read
            .filter(|_| st == TrackerStatus::Read)
            .map(|d| d.format("%Y/%m/%d").to_string())
            .unwrap_or_default();
        let isbn13 = b
            .meta
            .identifiers
            .iter()
            .filter(|i| i.scheme.eq_ignore_ascii_case("isbn"))
            .find_map(|i| isbn::canonical(&i.value))
            .map(|i| {
                if i.len() == 10 {
                    isbn::to_isbn13(&i).unwrap_or(i)
                } else {
                    i
                }
            })
            .unwrap_or_default();
        let isbn10 = isbn::to_isbn10(&isbn13).unwrap_or_default();
        let tags = b.meta.tags.join(", ");
        let row: Vec<String> = match service {
            Service::Goodreads => {
                let shelf = match st {
                    TrackerStatus::Read => "read",
                    TrackerStatus::Reading => "currently-reading",
                    TrackerStatus::ToRead => "to-read",
                    TrackerStatus::OnHold => "to-read",
                };
                vec![
                    b.meta.title.clone(),
                    b.meta.authors.first().cloned().unwrap_or_default(),
                    b.meta
                        .authors
                        .iter()
                        .skip(1)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", "),
                    if isbn10.is_empty() {
                        String::new()
                    } else {
                        format!("=\"{isbn10}\"")
                    },
                    if isbn13.is_empty() {
                        String::new()
                    } else {
                        format!("=\"{isbn13}\"")
                    },
                    stars(b.meta.rating),
                    date,
                    tags,
                    shelf.into(),
                ]
            }
            Service::StoryGraph => {
                let s = match st {
                    TrackerStatus::Read => "read",
                    TrackerStatus::Reading => "currently-reading",
                    TrackerStatus::ToRead => "to-read",
                    TrackerStatus::OnHold => "paused",
                };
                vec![
                    b.meta.title.clone(),
                    b.meta.authors.join(", "),
                    isbn13,
                    s.into(),
                    if b.meta.rating == 0 {
                        String::new()
                    } else {
                        stars(b.meta.rating)
                    },
                    date,
                    tags,
                ]
            }
        };
        out.push_str(
            &row.iter()
                .map(|f| csv_field(f))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOODREADS: &str = "Book Id,Title,Author,Author l-f,Additional Authors,ISBN,ISBN13,My Rating,Average Rating,Publisher,Date Read,Date Added,Bookshelves,Exclusive Shelf,My Review\n\
        1,\"Dune (Dune, #1)\",Frank Herbert,\"Herbert, Frank\",,\"=\"\"0441172717\"\"\",\"=\"\"9780441172719\"\"\",5,4.2,Ace,2021/03/14,2020/01/01,\"favorites, sci-fi\",read,\"Great,\nreally\"\n\
        2,Emma,Jane Austen,\"Austen, Jane\",,\"=\"\"\"\"\",\"=\"\"\"\"\",0,3.9,,,2020/02/02,classics,to-read,\n\
        3,The Hobbit,J.R.R. Tolkien,,,,,3,4.3,,,,currently-reading,currently-reading,\n";

    const STORYGRAPH: &str = "Title,Authors,Contributors,ISBN/UID,Format,Read Status,Date Added,Last Date Read,Dates Read,Read Count,Star Rating,Tags\n\
        Dune,Frank Herbert,,9780441172719,paperback,read,2020-01-01,2021-03-14,2021/03/01-2021/03/14,1,4.5,\"space, desert\"\n\
        Neuromancer,William Gibson,,,ebook,did-not-finish,2020-01-01,,,0,,\n";

    fn book(id: u64, title: &str, authors: &[&str], isbn13: Option<&str>) -> Book {
        let mut b: Book = serde_json::from_str(
            r#"{"id":1,"file":"a.epub","meta":{"title":"T","title_sort":"T","authors":[],"author_sort":"","publisher":"","language":"en","description":"","tags":[],"series":"","series_index":0.0,"identifiers":[],"published":"","rights":"","rating":0},"added":"2026-01-01T00:00:00Z","modified":"2026-01-01T00:00:00Z","size":1}"#,
        )
        .unwrap();
        b.id = id;
        b.meta.title = title.into();
        b.meta.authors = authors.iter().map(|s| s.to_string()).collect();
        if let Some(i) = isbn13 {
            b.meta.identifiers.push(bookx_core::opf::Identifier {
                scheme: "isbn".into(),
                value: i.into(),
                xml_id: None,
            });
        }
        b
    }

    #[test]
    fn csv_quotes_newlines_and_bom_are_handled() {
        let rows = parse_csv("\u{feff}a,\"b,\"\"c\"\"\",d\r\n1,\"x\ny\",3\r\n\r\n");
        assert_eq!(
            rows,
            vec![vec!["a", "b,\"c\"", "d"], vec!["1", "x\ny", "3"]]
        );
    }

    #[test]
    fn a_goodreads_export_is_read_with_status_rating_date_and_shelves() {
        let rows = parse(GOODREADS).unwrap();
        assert_eq!(rows.len(), 3);
        let dune = &rows[0];
        assert_eq!(dune.title, "Dune (Dune, #1)");
        assert_eq!(dune.isbn.as_deref(), Some("9780441172719"));
        assert_eq!(dune.status, Some(TrackerStatus::Read));
        assert_eq!(dune.rating, Some(10));
        assert_eq!(dune.date_read, NaiveDate::from_ymd_opt(2021, 3, 14));
        assert_eq!(dune.shelves, vec!["favorites", "sci-fi"]);
        assert_eq!(rows[1].status, Some(TrackerStatus::ToRead));
        assert_eq!(rows[1].rating, None);
        assert_eq!(rows[1].isbn, None);
        assert_eq!(rows[2].status, Some(TrackerStatus::Reading));
        assert!(rows[2].shelves.is_empty(), "a status shelf is not a tag");
        assert_eq!(rows[2].rating, Some(6));
    }

    #[test]
    fn a_storygraph_export_is_read_too() {
        let rows = parse(STORYGRAPH).unwrap();
        assert_eq!(rows[0].rating, Some(9));
        assert_eq!(rows[0].date_read, NaiveDate::from_ymd_opt(2021, 3, 14));
        assert_eq!(rows[0].isbn.as_deref(), Some("9780441172719"));
        assert_eq!(rows[1].status, Some(TrackerStatus::OnHold));
        assert!(
            parse("Name,Foo\nx,y\n").is_ok(),
            "a name column counts as a title"
        );
        assert!(parse("Foo,Bar\n1,2\n").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn rows_match_by_isbn_then_by_title_and_author() {
        let books = vec![
            book(1, "Dune", &["Frank Herbert"], Some("978-0-441-17271-9")),
            book(2, "Emma", &["Austen, Jane"], None),
            book(3, "The Hobbit", &["Someone Else"], None),
        ];
        let rows = parse(GOODREADS).unwrap();
        let m = match_rows(&rows, &books);
        assert_eq!((m[0].book, m[0].by), (Some(1), Some(MatchBy::Isbn)));
        assert_eq!((m[1].book, m[1].by), (Some(2), Some(MatchBy::TitleAuthor)));
        assert_eq!(m[2].book, None, "same title, different author");
    }

    #[test]
    fn the_plan_keeps_existing_ratings_unless_told_otherwise() {
        let mut dune = book(1, "Dune", &["Frank Herbert"], Some("9780441172719"));
        dune.meta.rating = 4;
        dune.meta.tags = vec!["Favorites".into()];
        let books = vec![dune, book(2, "Emma", &["Jane Austen"], None)];
        let rows = parse(GOODREADS).unwrap();
        let m = match_rows(&rows, &books);
        let p = plan(&rows, &m, &books, false);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].rating, None, "the book has a rating");
        assert_eq!(p[0].status, Some(TrackerStatus::Read));
        assert_eq!(p[0].add_tags, vec!["sci-fi"], "favorites is there already");
        assert_eq!(p[1].add_tags, vec!["classics", "to-read"]);
        assert_eq!(p[1].status, None);
        let p = plan(&rows, &m, &books, true);
        assert_eq!(p[0].rating, Some(10));
    }

    #[test]
    fn exports_use_the_layout_of_the_service_and_read_back() {
        let mut dune = book(
            1,
            "Dune, Messiah",
            &["Frank Herbert", "Brian"],
            Some("9780441172719"),
        );
        dune.meta.rating = 9;
        dune.meta.tags = vec!["sci-fi".into(), "desert".into()];
        dune.last_read = Some("2021-03-14T10:00:00Z".parse().unwrap());
        let books = vec![dune, book(2, "Emma", &["Jane Austen"], None)];
        let st = |b: &Book| (b.id == 1).then_some(TrackerStatus::Read);
        let gr = export(&books, &st, Service::Goodreads);
        assert!(gr.starts_with("Title,Author,Additional Authors,ISBN,ISBN13,My Rating"));
        assert!(gr.contains("\"Dune, Messiah\",Frank Herbert,Brian,\"=\"\"0441172717\"\"\",\"=\"\"9780441172719\"\"\",5,2021/03/14,\"sci-fi, desert\",read"));
        assert!(gr.contains("Emma,Jane Austen,,,,0,,,to-read"));
        let back = parse(&gr).unwrap();
        assert_eq!(back[0].rating, Some(10));
        assert_eq!(back[0].status, Some(TrackerStatus::Read));
        assert_eq!(back[0].isbn.as_deref(), Some("9780441172719"));
        assert_eq!(back[0].date_read, NaiveDate::from_ymd_opt(2021, 3, 14));
        assert_eq!(back[1].status, Some(TrackerStatus::ToRead));
        let sg = export(&books, &st, Service::StoryGraph);
        assert!(sg.contains("9780441172719,read,5,2021/03/14"));
        let back = parse(&sg).unwrap();
        assert_eq!(back[0].authors, vec!["Frank Herbert", "Brian"]);
        assert_eq!(back[0].shelves, vec!["sci-fi", "desert"]);
    }
}
